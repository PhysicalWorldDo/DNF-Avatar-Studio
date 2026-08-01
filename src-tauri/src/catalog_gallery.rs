use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::avatar_data;
use crate::catalog_gallery_snapshot::{
    GallerySnapshot, GallerySnapshotContext, GallerySnapshotDependency, GallerySnapshotEntry,
    GallerySnapshotStore,
};
use crate::catalog_thumbnail::{
    build_thumbnail_recipe, normalize_thumbnail_webp, CacheRetention, CatalogThumbnailCache,
    CatalogThumbnailRecipe, ThumbnailCacheLookup, ThumbnailCacheSummary, ThumbnailEntityKind,
    ThumbnailExpectedMember, ThumbnailRecipeInput,
};
use crate::game_resolver::{normalize_img_path_for_npk, resolve_game_npk_path};
use crate::models::{AvatarItem, AvatarSet, CatalogMetadataResponse, CatalogPage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogGalleryKind {
    Set,
    Item,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogThumbnailBatchPhase {
    Queued,
    Running,
    Paused,
    Finalizing,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogThumbnailBatchAction {
    Pause,
    Resume,
    Cancel,
}

pub(crate) fn transition_batch_phase(
    phase: CatalogThumbnailBatchPhase,
    action: CatalogThumbnailBatchAction,
) -> Result<CatalogThumbnailBatchPhase, String> {
    match (phase, action) {
        (CatalogThumbnailBatchPhase::Running, CatalogThumbnailBatchAction::Pause) => {
            Ok(CatalogThumbnailBatchPhase::Paused)
        }
        (CatalogThumbnailBatchPhase::Paused, CatalogThumbnailBatchAction::Resume) => {
            Ok(CatalogThumbnailBatchPhase::Running)
        }
        (
            CatalogThumbnailBatchPhase::Queued
            | CatalogThumbnailBatchPhase::Running
            | CatalogThumbnailBatchPhase::Paused
            | CatalogThumbnailBatchPhase::Finalizing,
            CatalogThumbnailBatchAction::Cancel,
        ) => Ok(CatalogThumbnailBatchPhase::Cancelled),
        _ => Err("当前批量任务状态不允许此操作".to_string()),
    }
}

pub(crate) fn complete_finalized_batch_phase(phase: &mut CatalogThumbnailBatchPhase) -> bool {
    if *phase != CatalogThumbnailBatchPhase::Finalizing {
        return false;
    }
    *phase = CatalogThumbnailBatchPhase::Completed;
    true
}

impl CatalogGalleryKind {
    fn thumbnail_kind(self) -> ThumbnailEntityKind {
        match self {
            Self::Set => ThumbnailEntityKind::Set,
            Self::Item => ThumbnailEntityKind::Item,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogGalleryPageRequest {
    pub game_dir: String,
    pub kind: CatalogGalleryKind,
    pub playable_role: String,
    pub slot: String,
    pub query: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogGalleryCard {
    pub entity_kind: CatalogGalleryKind,
    pub entity_id: String,
    pub name: String,
    pub slot: String,
    pub source_order: usize,
    pub entity_recipe_key: String,
    pub visual_signature: String,
    pub thumbnail_status: String,
    pub thumbnail_webp_base64: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogGalleryPage {
    pub cards: Vec<CatalogGalleryCard>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogGallerySnapshotRequest {
    pub game_dir: String,
    pub kind: CatalogGalleryKind,
    pub playable_role: String,
    pub slot: String,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogGallerySnapshotResponse {
    pub available: bool,
    pub snapshot_id: String,
    pub cards: Vec<CatalogGalleryCard>,
    pub catalog_total: usize,
    pub named_entries: usize,
    pub unique_visuals: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogThumbnailRequest {
    pub game_dir: String,
    pub kind: CatalogGalleryKind,
    pub entity_id: String,
    pub playable_role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogThumbnailResponse {
    pub entity_recipe_key: String,
    pub visual_signature: String,
    pub webp_base64: String,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogThumbnailBatchRequest {
    pub game_dir: String,
    pub kind: CatalogGalleryKind,
    pub playable_role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogThumbnailBatchPreflight {
    pub scope: String,
    pub total_entities: usize,
    pub cache_root: String,
    pub cache: ThumbnailCacheSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogThumbnailBatchStatus {
    pub job_id: String,
    pub scope: String,
    pub kind: CatalogGalleryKind,
    pub playable_role: String,
    pub phase: CatalogThumbnailBatchPhase,
    pub total_entities: usize,
    pub resolved_entities: usize,
    pub unique_visuals: usize,
    pub cache_hits: usize,
    pub generated_visuals: usize,
    pub failed_entities: usize,
    pub last_error: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct GalleryEntitySeed {
    kind: CatalogGalleryKind,
    id: String,
    name: String,
    slot: String,
    item: Option<AvatarItem>,
}

#[derive(Debug)]
struct CompletedBatchSnapshot {
    context: GallerySnapshotContext,
    catalog_total: usize,
    entries: Vec<GallerySnapshotEntry>,
    dependencies: Vec<GallerySnapshotDependency>,
}

#[derive(Debug)]
struct GalleryResourceEpoch {
    game_dir: PathBuf,
    dependencies: BTreeMap<String, GallerySnapshotDependency>,
    shared: Arc<Mutex<BTreeMap<String, GallerySnapshotDependency>>>,
}

impl GalleryResourceEpoch {
    fn new(
        game_dir: &Path,
        shared: Arc<Mutex<BTreeMap<String, GallerySnapshotDependency>>>,
    ) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            dependencies: BTreeMap::new(),
            shared,
        }
    }

    fn fingerprint(&mut self, img_path: &str) -> Result<String, String> {
        let normalized_img = normalize_img_path_for_npk(img_path);
        let npk_path = resolve_game_npk_path(&self.game_dir, &normalized_img)
            .ok_or_else(|| format!("无法定位 IMG 所属 NPK：{normalized_img}"))?;
        let path_key = GallerySnapshotDependency::normalized_path(&npk_path);
        if !self.dependencies.contains_key(&path_key) {
            let cached = self
                .shared
                .lock()
                .map_err(|_| "图库资源指纹缓存不可用".to_string())?
                .get(&path_key)
                .filter(|dependency| dependency.metadata_is_current())
                .cloned();
            let dependency = match cached {
                Some(dependency) => dependency,
                None => GallerySnapshotDependency::from_file(&npk_path)?,
            };
            self.shared
                .lock()
                .map_err(|_| "图库资源指纹缓存不可用".to_string())?
                .insert(path_key.clone(), dependency.clone());
            self.dependencies.insert(path_key.clone(), dependency);
        }
        self.dependencies
            .get(&path_key)
            .map(GallerySnapshotDependency::fingerprint)
            .ok_or_else(|| "图库资源指纹未能进入当前批次".to_string())
    }

    fn finish(self) -> Result<Vec<GallerySnapshotDependency>, String> {
        let dependencies = self.dependencies.into_values().collect::<Vec<_>>();
        if dependencies
            .iter()
            .any(|dependency| !dependency.is_current())
        {
            return Err("NPK 在缩略图批量生成期间发生变化，请重新生成".to_string());
        }
        Ok(dependencies)
    }
}

struct CatalogThumbnailBatchJob {
    status: Mutex<CatalogThumbnailBatchStatus>,
    wake: Condvar,
}

#[derive(Clone)]
pub struct CatalogGalleryState {
    cache_root: PathBuf,
    cache: Arc<Mutex<Option<CatalogThumbnailCache>>>,
    snapshot_store: GallerySnapshotStore,
    render_lock: Arc<Mutex<()>>,
    resource_fingerprints: Arc<Mutex<BTreeMap<String, GallerySnapshotDependency>>>,
    jobs: Arc<Mutex<HashMap<String, Arc<CatalogThumbnailBatchJob>>>>,
}

impl Default for CatalogGalleryState {
    fn default() -> Self {
        Self::new(
            crate::app_config::runtime_root()
                .join("data")
                .join("preview-cache"),
        )
    }
}

impl CatalogGalleryState {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            snapshot_store: GallerySnapshotStore::new(cache_root.join("gallery-snapshots")),
            cache_root,
            cache: Arc::new(Mutex::new(None)),
            render_lock: Arc::new(Mutex::new(())),
            resource_fingerprints: Arc::new(Mutex::new(BTreeMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn query_page(
        &self,
        request: CatalogGalleryPageRequest,
    ) -> Result<CatalogGalleryPage, String> {
        validate_gallery_selection(request.kind, &request.playable_role, &request.slot)?;
        let game_dir = concrete_game_dir(&request.game_dir)?;
        if request.offset == 0 {
            self.reset_resource_fingerprints()?;
        }
        let metadata = avatar_data::open_catalog_index_from_data_library()?;
        let (seeds, total, next_offset) = query_entity_seeds(&request)?;
        let mut cards = Vec::with_capacity(seeds.len());
        let mut resource_epoch = self.resource_epoch(&game_dir);
        for (index, seed) in seeds.into_iter().enumerate() {
            let source_order = request.offset.saturating_add(index);
            cards.push(
                match resolve_recipe_in_epoch(
                    &seed,
                    &request.playable_role,
                    &metadata,
                    &mut resource_epoch,
                ) {
                    Ok(recipe) => self.card_for_recipe(seed, source_order, recipe)?,
                    Err(error) => failed_card(seed, source_order, &metadata.build_id, error),
                },
            );
        }
        Ok(CatalogGalleryPage {
            cards,
            total,
            next_offset,
        })
    }

    pub fn load_snapshot(
        &self,
        request: CatalogGallerySnapshotRequest,
    ) -> Result<CatalogGallerySnapshotResponse, String> {
        validate_gallery_selection(request.kind, &request.playable_role, &request.slot)?;
        let game_dir = concrete_game_dir(&request.game_dir)?;
        self.reset_resource_fingerprints()?;
        let metadata = avatar_data::open_catalog_index_from_data_library()?;
        let context = GallerySnapshotContext::from_game_dir(
            &game_dir,
            &metadata.build_id,
            request.kind,
            &request.playable_role,
        );
        let Some(snapshot) = self.snapshot_store.load(&context)? else {
            return Ok(unavailable_snapshot_response());
        };

        let matching_keys =
            if request.kind == CatalogGalleryKind::Item || !request.query.trim().is_empty() {
                Some(query_all_entity_seed_keys(&request)?)
            } else {
                None
            };
        let entries =
            snapshot_entries_matching_catalog_selection(&snapshot.entries, matching_keys.as_ref());
        let ready_payloads = self.with_cache(|cache| {
            let mut payloads = HashMap::<String, Option<String>>::new();
            for entry in &entries {
                if payloads.contains_key(&entry.visual_signature) {
                    continue;
                }
                let payload =
                    match cache.lookup(&entry.entity_recipe_key, &entry.visual_signature)? {
                        ThumbnailCacheLookup::Ready(webp) => {
                            Some(base64::engine::general_purpose::STANDARD.encode(webp))
                        }
                        ThumbnailCacheLookup::Missing | ThumbnailCacheLookup::Stale => None,
                    };
                payloads.insert(entry.visual_signature.clone(), payload);
            }
            Ok(payloads)
        })?;
        if !snapshot_payloads_complete(&entries, &ready_payloads) {
            return Ok(unavailable_snapshot_response());
        }

        let mut emitted_payloads = HashSet::new();
        let mut cards = Vec::with_capacity(entries.len());
        for entry in entries {
            let payload = ready_payloads
                .get(&entry.visual_signature)
                .and_then(Option::as_ref)
                .ok_or_else(|| "图库快照缓存完整性复核失败".to_string())?;
            let first_visual = emitted_payloads.insert(entry.visual_signature.clone());
            cards.push(CatalogGalleryCard {
                entity_kind: request.kind,
                entity_id: entry.entity_id,
                name: entry.name,
                slot: entry.slot,
                source_order: entry.source_order,
                entity_recipe_key: entry.entity_recipe_key,
                visual_signature: entry.visual_signature,
                thumbnail_status: if first_visual {
                    "ready".to_string()
                } else {
                    "missing".to_string()
                },
                thumbnail_webp_base64: first_visual.then(|| payload.clone()),
                error: None,
            });
        }
        Ok(CatalogGallerySnapshotResponse {
            available: true,
            snapshot_id: snapshot.snapshot_id,
            catalog_total: snapshot.catalog_total,
            named_entries: cards.len(),
            unique_visuals: emitted_payloads.len(),
            cards,
        })
    }

    pub fn load_thumbnail(
        &self,
        request: CatalogThumbnailRequest,
        retention: CacheRetention,
    ) -> Result<CatalogThumbnailResponse, String> {
        validate_concrete_role(&request.playable_role)?;
        if request.entity_id.trim().is_empty() {
            return Err("缩略图实体 ID 不能为空".to_string());
        }
        let game_dir = concrete_game_dir(&request.game_dir)?;
        let metadata = avatar_data::open_catalog_index_from_data_library()?;
        let seed = load_entity_seed(
            request.kind,
            request.entity_id.trim(),
            &request.playable_role,
        )?;
        let mut resource_epoch = self.resource_epoch(&game_dir);
        let recipe = resolve_recipe_in_epoch(
            &seed,
            &request.playable_role,
            &metadata,
            &mut resource_epoch,
        )?;
        self.load_or_render_recipe(&game_dir, &recipe, retention)
    }

    pub fn cache_summary(&self) -> Result<ThumbnailCacheSummary, String> {
        self.with_cache(|cache| Ok(cache.summary()))
    }

    pub fn prepare_batch(
        &self,
        request: &CatalogThumbnailBatchRequest,
    ) -> Result<CatalogThumbnailBatchPreflight, String> {
        validate_concrete_role(&request.playable_role)?;
        concrete_game_dir(&request.game_dir)?;
        let scope = batch_scope(request.kind, &request.playable_role)?;
        let total_entities = batch_total(request.kind, &request.playable_role)?;
        Ok(CatalogThumbnailBatchPreflight {
            scope,
            total_entities,
            cache_root: self.cache_root.to_string_lossy().to_string(),
            cache: self.cache_summary()?,
        })
    }

    pub fn start_batch(
        &self,
        request: CatalogThumbnailBatchRequest,
    ) -> Result<CatalogThumbnailBatchStatus, String> {
        let preflight = self.prepare_batch(&request)?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "缩略图批量任务队列不可用".to_string())?;
        if jobs.values().any(|job| {
            job.status
                .lock()
                .map(|status| {
                    status.scope == preflight.scope
                        && matches!(
                            status.phase,
                            CatalogThumbnailBatchPhase::Queued
                                | CatalogThumbnailBatchPhase::Running
                                | CatalogThumbnailBatchPhase::Paused
                                | CatalogThumbnailBatchPhase::Finalizing
                        )
                })
                .unwrap_or(false)
        }) {
            return Err("该职业预览库已有正在进行的任务".to_string());
        }
        let status = CatalogThumbnailBatchStatus {
            job_id: Uuid::new_v4().to_string(),
            scope: preflight.scope,
            kind: request.kind,
            playable_role: request.playable_role.trim().to_ascii_lowercase(),
            phase: CatalogThumbnailBatchPhase::Queued,
            total_entities: preflight.total_entities,
            resolved_entities: 0,
            unique_visuals: 0,
            cache_hits: 0,
            generated_visuals: 0,
            failed_entities: 0,
            last_error: None,
            started_at_ms: now_ms(),
            finished_at_ms: None,
        };
        let job = Arc::new(CatalogThumbnailBatchJob {
            status: Mutex::new(status.clone()),
            wake: Condvar::new(),
        });
        jobs.insert(status.job_id.clone(), job.clone());
        drop(jobs);

        let worker_state = self.clone();
        if let Err(error) = std::thread::Builder::new()
            .name(format!("thumbnail-batch-{}", status.job_id))
            .spawn(move || worker_state.run_batch(request, job))
        {
            let mut jobs = self
                .jobs
                .lock()
                .map_err(|_| "缩略图批量任务队列不可用".to_string())?;
            jobs.remove(&status.job_id);
            return Err(format!("启动缩略图批量任务失败：{error}"));
        }
        Ok(status)
    }

    pub fn control_batch(
        &self,
        job_id: &str,
        action: CatalogThumbnailBatchAction,
    ) -> Result<CatalogThumbnailBatchStatus, String> {
        let job = self.batch_job(job_id)?;
        let mut status = job
            .status
            .lock()
            .map_err(|_| "缩略图批量任务状态不可用".to_string())?;
        status.phase = transition_batch_phase(status.phase, action)?;
        if status.phase == CatalogThumbnailBatchPhase::Cancelled {
            status.finished_at_ms = Some(now_ms());
        }
        let snapshot = status.clone();
        drop(status);
        job.wake.notify_all();
        Ok(snapshot)
    }

    pub fn batch_status(&self, job_id: &str) -> Result<CatalogThumbnailBatchStatus, String> {
        let job = self.batch_job(job_id)?;
        let status = job
            .status
            .lock()
            .map_err(|_| "缩略图批量任务状态不可用".to_string())?
            .clone();
        Ok(status)
    }

    pub fn batch_jobs(&self) -> Result<Vec<CatalogThumbnailBatchStatus>, String> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| "缩略图批量任务队列不可用".to_string())?;
        let mut statuses = jobs
            .values()
            .filter_map(|job| job.status.lock().ok().map(|status| status.clone()))
            .collect::<Vec<_>>();
        statuses.sort_by_key(|status| std::cmp::Reverse(status.started_at_ms));
        Ok(statuses)
    }

    fn batch_job(&self, job_id: &str) -> Result<Arc<CatalogThumbnailBatchJob>, String> {
        self.jobs
            .lock()
            .map_err(|_| "缩略图批量任务队列不可用".to_string())?
            .get(job_id)
            .cloned()
            .ok_or_else(|| "缩略图批量任务不存在".to_string())
    }

    fn run_batch(&self, request: CatalogThumbnailBatchRequest, job: Arc<CatalogThumbnailBatchJob>) {
        let batch_result = self.run_batch_inner(&request, &job);
        let flush_result = self.flush_cache();
        let result = match (batch_result, flush_result) {
            (Err(batch_error), Err(flush_error)) => {
                Err(format!("{batch_error}；提交缓存检查点失败：{flush_error}"))
            }
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Ok(snapshot), Ok(())) => Ok(snapshot),
        };
        let snapshot = match result {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return,
            Err(error) => {
                if let Ok(mut status) = job.status.lock() {
                    if status.phase != CatalogThumbnailBatchPhase::Cancelled {
                        status.phase = CatalogThumbnailBatchPhase::Failed;
                        status.last_error = Some(error);
                        status.finished_at_ms = Some(now_ms());
                    }
                }
                job.wake.notify_all();
                return;
            }
        };
        let Ok(should_finalize) = enter_batch_finalizing(&job) else {
            if let Ok(mut status) = job.status.lock() {
                if status.phase != CatalogThumbnailBatchPhase::Cancelled {
                    status.phase = CatalogThumbnailBatchPhase::Failed;
                    status.last_error = Some("缩略图批量任务终态不可用".to_string());
                    status.finished_at_ms = Some(now_ms());
                }
            }
            job.wake.notify_all();
            return;
        };
        if !should_finalize {
            return;
        }
        if let Err(error) = self.persist_completed_snapshot(&snapshot) {
            if let Ok(mut status) = job.status.lock() {
                if status.phase != CatalogThumbnailBatchPhase::Cancelled {
                    status.phase = CatalogThumbnailBatchPhase::Failed;
                    status.last_error = Some(error);
                    status.finished_at_ms = Some(now_ms());
                }
            }
            job.wake.notify_all();
            return;
        }
        if let Ok(mut status) = job.status.lock() {
            if complete_finalized_batch_phase(&mut status.phase) {
                status.finished_at_ms = Some(now_ms());
            }
        }
        job.wake.notify_all();
    }

    fn run_batch_inner(
        &self,
        request: &CatalogThumbnailBatchRequest,
        job: &CatalogThumbnailBatchJob,
    ) -> Result<Option<CompletedBatchSnapshot>, String> {
        if !wait_for_batch(job)? {
            return Ok(None);
        }
        let game_dir = concrete_game_dir(&request.game_dir)?;
        let metadata = avatar_data::open_catalog_index_from_data_library()?;
        let context = GallerySnapshotContext::from_game_dir(
            &game_dir,
            &metadata.build_id,
            request.kind,
            &request.playable_role,
        );
        let scope = batch_scope(request.kind, &request.playable_role)?;
        let catalog_total = job
            .status
            .lock()
            .map_err(|_| "缩略图批量任务状态不可用".to_string())?
            .total_entities;
        let mut offset = 0_usize;
        let mut seen_signatures = HashSet::new();
        let mut completed_signatures = HashSet::new();
        let mut pending_checkpoint_entities = 0usize;
        self.reset_resource_fingerprints()?;
        let mut resource_epoch = self.resource_epoch(&game_dir);
        let mut snapshot_entries = Vec::new();

        loop {
            if !wait_for_batch(job)? {
                return Ok(None);
            }
            let (seeds, next_offset) =
                query_batch_seeds(request.kind, &request.playable_role, offset)?;
            if seeds.is_empty() {
                let Some(next) = next_offset else {
                    break;
                };
                if next <= offset {
                    return Err("图库批量目录分页未向前推进".to_string());
                }
                offset = next;
                continue;
            }
            let page_entity_count = seeds.len();
            let retention = CacheRetention::Scope(scope.clone());
            let mut pending_visuals = Vec::<Vec<CatalogThumbnailRecipe>>::new();
            let mut pending_by_signature = HashMap::<String, usize>::new();
            let mut page_manifest = Vec::new();
            for (index, seed) in seeds.into_iter().enumerate() {
                if !wait_for_batch(job)? {
                    return Ok(None);
                }
                increment_batch(job, |status| status.resolved_entities += 1)?;
                let recipe = match resolve_recipe_in_epoch(
                    &seed,
                    &request.playable_role,
                    &metadata,
                    &mut resource_epoch,
                ) {
                    Ok(recipe) => recipe,
                    Err(error) => {
                        record_batch_failure(job, error)?;
                        continue;
                    }
                };
                page_manifest.push((seed.clone(), offset.saturating_add(index), recipe.clone()));
                let first_visual = seen_signatures.insert(recipe.visual_signature.clone());
                if first_visual {
                    increment_batch(job, |status| status.unique_visuals += 1)?;
                }
                if completed_signatures.contains(&recipe.visual_signature) {
                    if !self.link_existing_recipe_deferred(&recipe, retention.clone())? {
                        return Err(format!(
                            "已完成视觉 {} 的缓存记录意外缺失",
                            recipe.visual_signature
                        ));
                    }
                    continue;
                }
                if let Some(index) = pending_by_signature.get(&recipe.visual_signature).copied() {
                    pending_visuals[index].push(recipe);
                    continue;
                }
                match self.lookup_or_link_deferred(&recipe, retention.clone())? {
                    Some(_) => {
                        completed_signatures.insert(recipe.visual_signature.clone());
                        increment_batch(job, |status| status.cache_hits += 1)?;
                    }
                    None => {
                        pending_by_signature
                            .insert(recipe.visual_signature.clone(), pending_visuals.len());
                        pending_visuals.push(vec![recipe]);
                    }
                }
            }

            if !pending_visuals.is_empty() {
                if !wait_for_batch(job)? {
                    return Ok(None);
                }
                let _render_guard = self
                    .render_lock
                    .lock()
                    .map_err(|_| "缩略图渲染队列不可用".to_string())?;
                let mut groups_to_render = Vec::<Vec<CatalogThumbnailRecipe>>::new();
                for recipes in pending_visuals {
                    let primary = recipes
                        .first()
                        .ok_or_else(|| "缩略图批量渲染分组为空".to_string())?;
                    match self.lookup_or_link_deferred(primary, retention.clone())? {
                        Some(_) => {
                            completed_signatures.insert(primary.visual_signature.clone());
                            increment_batch(job, |status| status.cache_hits += 1)?;
                            self.link_batch_aliases(&recipes[1..], retention.clone())?;
                        }
                        None => groups_to_render.push(recipes),
                    }
                }

                let layer_groups = groups_to_render
                    .iter()
                    .map(|recipes| recipes[0].layers.clone())
                    .collect::<Vec<_>>();
                let previews = crate::renderer::render_layer_groups_results_from_game_dir(
                    &game_dir,
                    &layer_groups,
                );
                for (recipes, preview) in groups_to_render.into_iter().zip(previews) {
                    let primary = &recipes[0];
                    let rendered = preview.and_then(|preview| normalize_thumbnail_webp(&preview));
                    let webp = match rendered {
                        Ok(webp) => webp,
                        Err(error) => {
                            record_batch_failures(job, recipes.len(), error)?;
                            continue;
                        }
                    };
                    self.with_cache(|cache| {
                        cache.put_deferred(
                            &primary.entity_recipe_key,
                            &primary.visual_signature,
                            &webp,
                            retention.clone(),
                        )
                    })?;
                    completed_signatures.insert(primary.visual_signature.clone());
                    increment_batch(job, |status| status.generated_visuals += 1)?;
                    self.link_batch_aliases(&recipes[1..], retention.clone())?;
                }
            }
            snapshot_entries.extend(page_manifest.into_iter().filter_map(
                |(seed, source_order, recipe)| {
                    completed_signatures
                        .contains(&recipe.visual_signature)
                        .then_some(GallerySnapshotEntry {
                            entity_id: seed.id,
                            name: seed.name,
                            slot: seed.slot,
                            source_order,
                            entity_recipe_key: recipe.entity_recipe_key,
                            visual_signature: recipe.visual_signature,
                        })
                },
            ));
            pending_checkpoint_entities =
                pending_checkpoint_entities.saturating_add(page_entity_count);
            if should_flush_batch_checkpoint(pending_checkpoint_entities, next_offset.is_none()) {
                self.flush_cache()?;
                pending_checkpoint_entities = 0;
            }
            let Some(next) = next_offset else {
                break;
            };
            if next <= offset {
                return Err("图库批量目录分页未向前推进".to_string());
            }
            offset = next;
        }

        snapshot_entries.sort_by_key(|entry| entry.source_order);
        Ok(Some(CompletedBatchSnapshot {
            context,
            catalog_total,
            entries: snapshot_entries,
            dependencies: resource_epoch.finish()?,
        }))
    }

    fn persist_completed_snapshot(&self, completed: &CompletedBatchSnapshot) -> Result<(), String> {
        self.with_cache(|cache| {
            for entry in &completed.entries {
                if !matches!(
                    cache.lookup(&entry.entity_recipe_key, &entry.visual_signature)?,
                    ThumbnailCacheLookup::Ready(_)
                ) {
                    return Err(format!(
                        "图库快照引用的缩略图缓存缺失：{}",
                        entry.visual_signature
                    ));
                }
            }
            Ok(())
        })?;
        self.snapshot_store.save(&GallerySnapshot::new(
            completed.context.clone(),
            completed.catalog_total,
            completed.entries.clone(),
            completed.dependencies.clone(),
        ))
    }

    fn card_for_recipe(
        &self,
        seed: GalleryEntitySeed,
        source_order: usize,
        recipe: CatalogThumbnailRecipe,
    ) -> Result<CatalogGalleryCard, String> {
        let cached = self.lookup_or_link(&recipe, CacheRetention::Automatic)?;
        let (thumbnail_status, thumbnail_webp_base64) = match cached {
            Some(webp) => (
                "ready".to_string(),
                Some(base64::engine::general_purpose::STANDARD.encode(webp)),
            ),
            None => ("missing".to_string(), None),
        };
        Ok(CatalogGalleryCard {
            entity_kind: seed.kind,
            entity_id: seed.id,
            name: seed.name,
            slot: seed.slot,
            source_order,
            entity_recipe_key: recipe.entity_recipe_key,
            visual_signature: recipe.visual_signature,
            thumbnail_status,
            thumbnail_webp_base64,
            error: None,
        })
    }

    fn load_or_render_recipe(
        &self,
        game_dir: &Path,
        recipe: &CatalogThumbnailRecipe,
        retention: CacheRetention,
    ) -> Result<CatalogThumbnailResponse, String> {
        self.load_or_render_recipe_inner(game_dir, recipe, retention, false)
    }

    fn load_or_render_recipe_inner(
        &self,
        game_dir: &Path,
        recipe: &CatalogThumbnailRecipe,
        retention: CacheRetention,
        defer_index: bool,
    ) -> Result<CatalogThumbnailResponse, String> {
        if let Some(webp) = self.lookup_or_link_inner(recipe, retention.clone(), defer_index)? {
            return Ok(thumbnail_response(recipe, webp, true));
        }

        let _render_guard = self
            .render_lock
            .lock()
            .map_err(|_| "缩略图渲染队列不可用".to_string())?;
        if let Some(webp) = self.lookup_or_link_inner(recipe, retention.clone(), defer_index)? {
            return Ok(thumbnail_response(recipe, webp, true));
        }

        let preview = crate::renderer::render_layers_from_game_dir(game_dir, &recipe.layers)?;
        let webp = normalize_thumbnail_webp(&preview)?;
        self.with_cache(|cache| {
            if defer_index {
                cache.put_deferred(
                    &recipe.entity_recipe_key,
                    &recipe.visual_signature,
                    &webp,
                    retention,
                )
            } else {
                cache.put(
                    &recipe.entity_recipe_key,
                    &recipe.visual_signature,
                    &webp,
                    retention,
                )
            }
        })?;
        Ok(thumbnail_response(recipe, webp, false))
    }

    fn lookup_or_link(
        &self,
        recipe: &CatalogThumbnailRecipe,
        retention: CacheRetention,
    ) -> Result<Option<Vec<u8>>, String> {
        self.lookup_or_link_inner(recipe, retention, false)
    }

    fn lookup_or_link_deferred(
        &self,
        recipe: &CatalogThumbnailRecipe,
        retention: CacheRetention,
    ) -> Result<Option<Vec<u8>>, String> {
        self.lookup_or_link_inner(recipe, retention, true)
    }

    fn link_existing_recipe_deferred(
        &self,
        recipe: &CatalogThumbnailRecipe,
        retention: CacheRetention,
    ) -> Result<bool, String> {
        self.with_cache(|cache| {
            cache.link_existing_deferred(
                &recipe.entity_recipe_key,
                &recipe.visual_signature,
                retention,
            )
        })
    }

    fn link_batch_aliases(
        &self,
        recipes: &[CatalogThumbnailRecipe],
        retention: CacheRetention,
    ) -> Result<(), String> {
        for recipe in recipes {
            if !self.link_existing_recipe_deferred(recipe, retention.clone())? {
                return Err(format!(
                    "视觉 {} 的批量缓存别名无法关联",
                    recipe.visual_signature
                ));
            }
        }
        Ok(())
    }

    fn lookup_or_link_inner(
        &self,
        recipe: &CatalogThumbnailRecipe,
        retention: CacheRetention,
        defer_index: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        self.with_cache(|cache| {
            let lookup = if defer_index {
                cache.lookup_retained_deferred(
                    &recipe.entity_recipe_key,
                    &recipe.visual_signature,
                    retention,
                )?
            } else {
                cache.lookup_retained(
                    &recipe.entity_recipe_key,
                    &recipe.visual_signature,
                    retention,
                )?
            };
            match lookup {
                ThumbnailCacheLookup::Ready(webp) => Ok(Some(webp)),
                ThumbnailCacheLookup::Missing | ThumbnailCacheLookup::Stale => Ok(None),
            }
        })
    }

    fn flush_cache(&self) -> Result<(), String> {
        self.with_cache(|cache| cache.flush())
    }

    fn resource_epoch(&self, game_dir: &Path) -> GalleryResourceEpoch {
        GalleryResourceEpoch::new(game_dir, self.resource_fingerprints.clone())
    }

    fn reset_resource_fingerprints(&self) -> Result<(), String> {
        self.resource_fingerprints
            .lock()
            .map_err(|_| "图库资源指纹缓存不可用".to_string())?
            .clear();
        Ok(())
    }

    fn with_cache<T>(
        &self,
        operation: impl FnOnce(&mut CatalogThumbnailCache) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| "缩略图缓存队列不可用".to_string())?;
        if cache.is_none() {
            *cache = Some(CatalogThumbnailCache::open_user_bound(
                self.cache_root.clone(),
            )?);
        }
        operation(cache.as_mut().expect("thumbnail cache initialized"))
    }
}

pub(crate) fn validate_gallery_selection(
    kind: CatalogGalleryKind,
    playable_role: &str,
    slot: &str,
) -> Result<(), String> {
    let role = playable_role.trim().to_ascii_lowercase();
    if role.is_empty() || role == "all" {
        return Err("图库模式需要先选择具体职业".to_string());
    }
    if kind == CatalogGalleryKind::Item {
        let slot = slot.trim().to_ascii_lowercase();
        if slot.is_empty() || slot == "all" {
            return Err("单件图库需要先选择具体部位".to_string());
        }
    }
    Ok(())
}

pub(crate) fn batch_scope(kind: CatalogGalleryKind, playable_role: &str) -> Result<String, String> {
    let role = playable_role.trim().to_ascii_lowercase();
    if role.is_empty() || role == "all" {
        return Err("批量生成需要先选择具体职业".to_string());
    }
    Ok(match kind {
        CatalogGalleryKind::Set => format!("{role}:sets"),
        CatalogGalleryKind::Item => format!("{role}:items:all"),
    })
}

pub(crate) fn should_flush_batch_checkpoint(pending_entities: usize, reached_end: bool) -> bool {
    const BATCH_INDEX_CHECKPOINT_ENTITIES: usize = 4_096;
    reached_end || pending_entities >= BATCH_INDEX_CHECKPOINT_ENTITIES
}

pub(crate) type GallerySnapshotEntryKey = (String, String, String);

fn snapshot_entry_key(entry: &GallerySnapshotEntry) -> GallerySnapshotEntryKey {
    (
        entry.entity_id.clone(),
        entry.name.clone(),
        entry.slot.to_ascii_lowercase(),
    )
}

pub(crate) fn snapshot_entries_matching_catalog_selection(
    entries: &[GallerySnapshotEntry],
    matching_keys: Option<&HashSet<GallerySnapshotEntryKey>>,
) -> Vec<GallerySnapshotEntry> {
    entries
        .iter()
        .filter(|entry| matching_keys.is_none_or(|keys| keys.contains(&snapshot_entry_key(entry))))
        .cloned()
        .collect()
}

fn entity_seed_key(seed: GalleryEntitySeed) -> GallerySnapshotEntryKey {
    (seed.id, seed.name, seed.slot.to_ascii_lowercase())
}

fn query_all_entity_seed_keys(
    request: &CatalogGallerySnapshotRequest,
) -> Result<HashSet<GallerySnapshotEntryKey>, String> {
    let mut keys = HashSet::new();
    let mut offset = 0usize;
    loop {
        let page_request = CatalogGalleryPageRequest {
            game_dir: request.game_dir.clone(),
            kind: request.kind,
            playable_role: request.playable_role.clone(),
            slot: request.slot.clone(),
            query: request.query.clone(),
            offset,
            limit: 256,
        };
        let (seeds, _, next_offset) = query_entity_seeds(&page_request)?;
        keys.extend(seeds.into_iter().map(entity_seed_key));
        let Some(next) = next_offset else {
            break;
        };
        if next <= offset {
            return Err("图库快照搜索分页未向前推进".to_string());
        }
        offset = next;
    }
    Ok(keys)
}

fn snapshot_payloads_complete(
    entries: &[GallerySnapshotEntry],
    payloads: &HashMap<String, Option<String>>,
) -> bool {
    entries.iter().all(|entry| {
        payloads
            .get(&entry.visual_signature)
            .is_some_and(Option::is_some)
    })
}

fn unavailable_snapshot_response() -> CatalogGallerySnapshotResponse {
    CatalogGallerySnapshotResponse {
        available: false,
        snapshot_id: String::new(),
        cards: Vec::new(),
        catalog_total: 0,
        named_entries: 0,
        unique_visuals: 0,
    }
}

fn validate_concrete_role(playable_role: &str) -> Result<(), String> {
    let role = playable_role.trim();
    if role.is_empty() || role.eq_ignore_ascii_case("all") {
        Err("图库模式需要先选择具体职业".to_string())
    } else {
        Ok(())
    }
}

fn concrete_game_dir(game_dir: &str) -> Result<PathBuf, String> {
    let game_dir = PathBuf::from(game_dir.trim());
    if game_dir.as_os_str().is_empty() || !game_dir.is_dir() {
        return Err("游戏 ImagePacks2 目录不可用".to_string());
    }
    Ok(game_dir)
}

fn query_entity_seeds(
    request: &CatalogGalleryPageRequest,
) -> Result<(Vec<GalleryEntitySeed>, usize, Option<usize>), String> {
    match request.kind {
        CatalogGalleryKind::Set => {
            let page = avatar_data::query_avatar_sets_page_from_data_library(
                &request.playable_role,
                &request.query,
                request.offset,
                request.limit,
            )?;
            Ok(set_page_seeds(page))
        }
        CatalogGalleryKind::Item => {
            let page = avatar_data::query_avatar_items_page_from_data_library(
                &request.slot,
                &request.playable_role,
                &request.query,
                request.offset,
                request.limit,
            )?;
            Ok(item_page_seeds(page))
        }
    }
}

fn set_page_seeds(page: CatalogPage<AvatarSet>) -> (Vec<GalleryEntitySeed>, usize, Option<usize>) {
    let seeds = page
        .records
        .into_iter()
        .filter_map(|set| {
            let name = clear_gallery_display_name(&set.set_name)?;
            Some(GalleryEntitySeed {
                kind: CatalogGalleryKind::Set,
                id: set.set_id,
                name,
                slot: "set".to_string(),
                item: None,
            })
        })
        .collect();
    (seeds, page.total, page.next_offset)
}

pub(crate) fn clear_gallery_display_name(raw_name: &str) -> Option<String> {
    const PLACEHOLDER_PREFIXES: &[&str] = &[
        "未知",
        "未命名",
        "待确认",
        "后续确认",
        "待补充",
        "暂未命名",
        "名称待定",
        "待定",
    ];
    let name = raw_name.trim();
    if name.is_empty()
        || (name != "未知套装"
            && PLACEHOLDER_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix)))
        || name.starts_with("((")
        || name.starts_with("（（")
        || name.starts_with('<')
        || name.to_ascii_lowercase().starts_with("name_")
        || name.chars().all(|character| character.is_ascii_digit())
        || !name.chars().any(is_cjk_ideograph)
    {
        None
    } else {
        Some(name.to_string())
    }
}

fn is_cjk_ideograph(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'
    )
}

fn item_page_seeds(
    page: CatalogPage<AvatarItem>,
) -> (Vec<GalleryEntitySeed>, usize, Option<usize>) {
    let seeds = page
        .records
        .into_iter()
        .filter_map(|item| {
            let name = clear_gallery_display_name(&item.item_name)?;
            Some(GalleryEntitySeed {
                kind: CatalogGalleryKind::Item,
                id: item.item_id.clone(),
                name,
                slot: item.slot.clone(),
                item: Some(item),
            })
        })
        .collect();
    (seeds, page.total, page.next_offset)
}

fn load_entity_seed(
    kind: CatalogGalleryKind,
    entity_id: &str,
    playable_role: &str,
) -> Result<GalleryEntitySeed, String> {
    match kind {
        CatalogGalleryKind::Set => {
            let detail = avatar_data::get_avatar_set_with_members_from_data_library(
                entity_id,
                playable_role,
            )?;
            let name = clear_gallery_display_name(&detail.set.set_name)
                .ok_or_else(|| "所选套装没有明确中文名，不进入图库".to_string())?;
            Ok(GalleryEntitySeed {
                kind,
                id: detail.set.set_id,
                name,
                slot: "set".to_string(),
                item: None,
            })
        }
        CatalogGalleryKind::Item => {
            let mut items =
                avatar_data::get_avatar_items_by_ids_from_data_library(&[entity_id.to_string()])?;
            let item = items
                .pop()
                .ok_or_else(|| format!("内置目录中不存在单件 {entity_id}"))?;
            if !item
                .playable_roles
                .iter()
                .any(|role| role.eq_ignore_ascii_case(playable_role))
            {
                return Err("所选单件不属于当前职业".to_string());
            }
            let name = clear_gallery_display_name(&item.item_name)
                .ok_or_else(|| "所选单件没有明确中文名，不进入图库".to_string())?;
            Ok(GalleryEntitySeed {
                kind,
                id: item.item_id.clone(),
                name,
                slot: item.slot.clone(),
                item: Some(item),
            })
        }
    }
}

fn resolve_recipe_in_epoch(
    seed: &GalleryEntitySeed,
    playable_role: &str,
    metadata: &CatalogMetadataResponse,
    resource_epoch: &mut GalleryResourceEpoch,
) -> Result<CatalogThumbnailRecipe, String> {
    let role = metadata
        .roles
        .iter()
        .find(|role| {
            role.playable_role_key
                .eq_ignore_ascii_case(playable_role.trim())
        })
        .ok_or_else(|| format!("内置目录中不存在职业 {playable_role}"))?;
    let family = metadata
        .visual_families
        .iter()
        .find(|family| {
            family
                .visual_family_key
                .eq_ignore_ascii_case(&role.visual_family_key)
        })
        .ok_or_else(|| format!("职业 {} 缺少视觉族", role.playable_role_key))?;
    let items = match seed.kind {
        CatalogGalleryKind::Set => {
            avatar_data::get_avatar_set_with_members_from_data_library(&seed.id, playable_role)?
                .items
        }
        CatalogGalleryKind::Item => {
            if let Some(item) = &seed.item {
                vec![item.clone()]
            } else {
                avatar_data::get_avatar_items_by_ids_from_data_library(&[seed.id.clone()])?
            }
        }
    };
    let expected_members = items
        .iter()
        .map(|item| expected_member_for_visual_family(item, &family.visual_family_key))
        .collect::<Vec<_>>();
    let item_ids = expected_members
        .iter()
        .map(|member| member.item_id.clone())
        .collect::<Vec<_>>();
    let layers =
        avatar_data::load_layers_for_playable_role_from_data_library(&item_ids, playable_role)?;
    build_thumbnail_recipe(
        ThumbnailRecipeInput {
            entity_kind: seed.kind.thumbnail_kind(),
            entity_id: seed.id.clone(),
            name: seed.name.clone(),
            playable_role_key: role.playable_role_key.clone(),
            visual_family_key: family.visual_family_key.clone(),
            catalog_build_id: metadata.build_id.clone(),
            default_skin_img: family.default_skin_img.clone(),
            expected_members,
            layers,
        },
        |img_path| resource_epoch.fingerprint(img_path),
    )
}

fn expected_member_for_visual_family(
    item: &AvatarItem,
    visual_family_key: &str,
) -> ThumbnailExpectedMember {
    let expected_layer_count = if item.layer_counts_by_visual_family.is_empty() {
        item.layer_count
    } else {
        item.layer_counts_by_visual_family
            .iter()
            .find_map(|(family, count)| {
                family
                    .eq_ignore_ascii_case(visual_family_key)
                    .then_some(*count)
            })
            .unwrap_or(0)
    };
    ThumbnailExpectedMember {
        item_id: item.item_id.clone(),
        slot: item.slot.clone(),
        expected_layer_count,
    }
}

fn failed_card(
    seed: GalleryEntitySeed,
    source_order: usize,
    catalog_build_id: &str,
    error: String,
) -> CatalogGalleryCard {
    let mut digest = Sha256::new();
    digest.update(b"gallery-failed-v1|");
    digest.update(match seed.kind {
        CatalogGalleryKind::Set => b"set".as_slice(),
        CatalogGalleryKind::Item => b"item".as_slice(),
    });
    digest.update(b"|");
    digest.update(seed.id.as_bytes());
    digest.update(b"|");
    digest.update(catalog_build_id.as_bytes());
    let signature = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    CatalogGalleryCard {
        entity_kind: seed.kind,
        entity_id: seed.id,
        name: seed.name,
        slot: seed.slot,
        source_order,
        entity_recipe_key: signature.clone(),
        visual_signature: signature,
        thumbnail_status: "failed".to_string(),
        thumbnail_webp_base64: None,
        error: Some(error),
    }
}

fn thumbnail_response(
    recipe: &CatalogThumbnailRecipe,
    webp: Vec<u8>,
    cache_hit: bool,
) -> CatalogThumbnailResponse {
    CatalogThumbnailResponse {
        entity_recipe_key: recipe.entity_recipe_key.clone(),
        visual_signature: recipe.visual_signature.clone(),
        webp_base64: base64::engine::general_purpose::STANDARD.encode(webp),
        cache_hit,
    }
}

fn batch_total(kind: CatalogGalleryKind, playable_role: &str) -> Result<usize, String> {
    let mut total = 0usize;
    let mut offset = 0usize;
    loop {
        let (seeds, next_offset) = query_batch_seeds(kind, playable_role, offset)?;
        total = total.saturating_add(seeds.len());
        let Some(next) = next_offset else {
            break;
        };
        if next <= offset {
            return Err("图库批量目录分页未向前推进".to_string());
        }
        offset = next;
    }
    Ok(total)
}

fn query_batch_seeds(
    kind: CatalogGalleryKind,
    playable_role: &str,
    offset: usize,
) -> Result<(Vec<GalleryEntitySeed>, Option<usize>), String> {
    const BATCH_PAGE_SIZE: usize = 128;
    Ok(match kind {
        CatalogGalleryKind::Set => {
            let (seeds, _, next_offset) =
                set_page_seeds(avatar_data::query_avatar_sets_page_from_data_library(
                    playable_role,
                    "",
                    offset,
                    BATCH_PAGE_SIZE,
                )?);
            (seeds, next_offset)
        }
        CatalogGalleryKind::Item => {
            let (seeds, _, next_offset) =
                item_page_seeds(avatar_data::query_avatar_items_page_from_data_library(
                    "all",
                    playable_role,
                    "",
                    offset,
                    BATCH_PAGE_SIZE,
                )?);
            (seeds, next_offset)
        }
    })
}

fn enter_batch_finalizing(job: &CatalogThumbnailBatchJob) -> Result<bool, String> {
    let mut status = job
        .status
        .lock()
        .map_err(|_| "缩略图批量任务状态不可用".to_string())?;
    loop {
        match status.phase {
            CatalogThumbnailBatchPhase::Queued | CatalogThumbnailBatchPhase::Running => {
                status.phase = CatalogThumbnailBatchPhase::Finalizing;
                return Ok(true);
            }
            CatalogThumbnailBatchPhase::Paused => {
                status = job
                    .wake
                    .wait(status)
                    .map_err(|_| "缩略图批量任务暂停状态不可用".to_string())?;
            }
            CatalogThumbnailBatchPhase::Finalizing
            | CatalogThumbnailBatchPhase::Completed
            | CatalogThumbnailBatchPhase::Cancelled
            | CatalogThumbnailBatchPhase::Failed => return Ok(false),
        }
    }
}

fn wait_for_batch(job: &CatalogThumbnailBatchJob) -> Result<bool, String> {
    let mut status = job
        .status
        .lock()
        .map_err(|_| "缩略图批量任务状态不可用".to_string())?;
    loop {
        match status.phase {
            CatalogThumbnailBatchPhase::Queued => {
                status.phase = CatalogThumbnailBatchPhase::Running;
                return Ok(true);
            }
            CatalogThumbnailBatchPhase::Running => return Ok(true),
            CatalogThumbnailBatchPhase::Paused => {
                status = job
                    .wake
                    .wait(status)
                    .map_err(|_| "缩略图批量任务暂停状态不可用".to_string())?;
            }
            CatalogThumbnailBatchPhase::Finalizing
            | CatalogThumbnailBatchPhase::Cancelled
            | CatalogThumbnailBatchPhase::Completed
            | CatalogThumbnailBatchPhase::Failed => return Ok(false),
        }
    }
}

fn increment_batch(
    job: &CatalogThumbnailBatchJob,
    update: impl FnOnce(&mut CatalogThumbnailBatchStatus),
) -> Result<(), String> {
    let mut status = job
        .status
        .lock()
        .map_err(|_| "缩略图批量任务状态不可用".to_string())?;
    update(&mut status);
    Ok(())
}

fn record_batch_failure(job: &CatalogThumbnailBatchJob, error: String) -> Result<(), String> {
    record_batch_failures(job, 1, error)
}

fn record_batch_failures(
    job: &CatalogThumbnailBatchJob,
    count: usize,
    error: String,
) -> Result<(), String> {
    increment_batch(job, |status| {
        status.failed_entities = status.failed_entities.saturating_add(count);
        status.last_error = Some(error);
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
