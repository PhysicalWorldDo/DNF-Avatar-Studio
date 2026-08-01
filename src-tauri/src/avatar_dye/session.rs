use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dnflib_safe::DecodedFrame;

use super::direct_color::{virtual_palette_sha256, DirectColorFormat};
use super::inspector::{
    img_header_metadata, inspect_img_catalog, DyeCapabilityStatus, ImgDyeInspection,
};
use super::models::{
    DyeBatchMutation, DyeColorEditV1, DyeRecipeV1, DyeSourceFingerprintV1, DyeTargetKindV1,
    DyeTargetRecipeV1, PersistedDyeStateV2, PersistedDyeTargetRefV1,
};
use super::palette_patch::{build_patched_img_bytes, sha256};
use super::preview::{
    render_session_animation, render_session_frame, session_composite_frame_count,
    DyePreviewAnimation, DyePreviewFrame,
};
use super::recipe::canonicalize_recipe;
use super::resource_family::discover_resource_family_entries;

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
const MAX_HISTORY_ENTRIES: usize = 100;
const OPAQUE_ID_HEX_LENGTH: usize = 32;
const MAX_PREVIEW_CACHE_ENTRIES: usize = 512;
const MAX_PREVIEW_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLOR_SELECTION_INPUTS: usize = 256;
const MAX_BATCH_TARGETS: usize = 2_048;
const MAX_SEMANTIC_STATE_JSON_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PatchedImgCacheKey {
    entry_identity: [u8; 32],
    active_palette: u16,
    recipe_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DecodedFrameCacheKey {
    entry_identity: [u8; 32],
    frame_index: usize,
    active_palette: u16,
    recipe_hash: [u8; 32],
    patched_bytes_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PreviewCacheKey {
    Patched(PatchedImgCacheKey),
    Decoded(DecodedFrameCacheKey),
}

#[derive(Debug, Clone)]
enum PreviewCacheValue {
    Patched(PreviewImgBytes),
    Decoded(Arc<DecodedFrame>),
}

#[derive(Debug, Clone)]
struct PreviewCacheEntry {
    value: PreviewCacheValue,
    weight: usize,
}

#[derive(Debug, Clone)]
pub(super) struct PreviewImgBytes {
    pub(super) bytes: Arc<[u8]>,
    recipe_hash: [u8; 32],
    bytes_hash: [u8; 32],
}

#[derive(Debug, Clone, Default)]
struct PreviewCacheCounters {
    patched_hits: u64,
    patched_builds: u64,
    decoded_hits: u64,
    decoded_misses: u64,
    evictions: u64,
}

#[derive(Debug, Default)]
struct SessionPreviewCache {
    entries: HashMap<PreviewCacheKey, PreviewCacheEntry>,
    lru: VecDeque<PreviewCacheKey>,
    total_bytes: usize,
    counters: PreviewCacheCounters,
}

impl SessionPreviewCache {
    fn get_patched(&mut self, key: &PatchedImgCacheKey) -> Option<PreviewImgBytes> {
        let cache_key = PreviewCacheKey::Patched(key.clone());
        let value = self.entries.get(&cache_key).and_then(|entry| {
            if let PreviewCacheValue::Patched(value) = &entry.value {
                Some(value.clone())
            } else {
                None
            }
        });
        if value.is_some() {
            self.counters.patched_hits = self.counters.patched_hits.saturating_add(1);
            self.touch(&cache_key);
        } else {
            self.counters.patched_builds = self.counters.patched_builds.saturating_add(1);
        }
        value
    }

    fn get_decoded(&mut self, key: &DecodedFrameCacheKey) -> Option<Arc<DecodedFrame>> {
        let cache_key = PreviewCacheKey::Decoded(key.clone());
        let value = self.entries.get(&cache_key).and_then(|entry| {
            if let PreviewCacheValue::Decoded(value) = &entry.value {
                Some(Arc::clone(value))
            } else {
                None
            }
        });
        if value.is_some() {
            self.counters.decoded_hits = self.counters.decoded_hits.saturating_add(1);
            self.touch(&cache_key);
        } else {
            self.counters.decoded_misses = self.counters.decoded_misses.saturating_add(1);
        }
        value
    }

    fn insert_patched(&mut self, key: PatchedImgCacheKey, value: PreviewImgBytes) {
        let weight = value.bytes.len();
        self.insert(
            PreviewCacheKey::Patched(key),
            PreviewCacheValue::Patched(value),
            weight,
        );
    }

    fn insert_decoded(&mut self, key: DecodedFrameCacheKey, value: Arc<DecodedFrame>) {
        let weight = value.rgba.len();
        self.insert(
            PreviewCacheKey::Decoded(key),
            PreviewCacheValue::Decoded(value),
            weight,
        );
    }

    fn insert(&mut self, key: PreviewCacheKey, value: PreviewCacheValue, weight: usize) {
        if weight > MAX_PREVIEW_CACHE_BYTES {
            return;
        }
        self.remove(&key);
        while self.entries.len() >= MAX_PREVIEW_CACHE_ENTRIES
            || self
                .total_bytes
                .checked_add(weight)
                .is_none_or(|bytes| bytes > MAX_PREVIEW_CACHE_BYTES)
        {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.weight);
                self.counters.evictions = self.counters.evictions.saturating_add(1);
            }
        }
        self.total_bytes = self.total_bytes.saturating_add(weight);
        self.entries
            .insert(key.clone(), PreviewCacheEntry { value, weight });
        self.lru.push_back(key);
    }

    fn touch(&mut self, key: &PreviewCacheKey) {
        if let Some(index) = self.lru.iter().position(|candidate| candidate == key) {
            self.lru.remove(index);
        }
        self.lru.push_back(key.clone());
    }

    fn remove(&mut self, key: &PreviewCacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.weight);
        }
        if let Some(index) = self.lru.iter().position(|candidate| candidate == key) {
            self.lru.remove(index);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.lru.clear();
        self.total_bytes = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DyeSessionIdentity {
    pub slot: String,
    pub item_ref: String,
    pub layer_tag: String,
}

pub(crate) struct DyeWorkbenchLayerInput {
    pub identity: DyeSessionIdentity,
    pub layer_order: i32,
    pub requested_base_entry: String,
    pub active_palette: u16,
    pub entries: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone)]
pub(crate) struct DyeUnavailableTargetInput {
    pub identity: DyeSessionIdentity,
    pub layer_order: i32,
    pub kind: DyeTargetKindV1,
}

pub(crate) struct PositionedDyeWorkbenchLayerInput {
    pub draw_ordinal: usize,
    pub input: DyeWorkbenchLayerInput,
}

pub(crate) struct PositionedDyeUnavailableTargetInput {
    pub draw_ordinal: usize,
    pub input: DyeUnavailableTargetInput,
}

fn target_kind_ordinal(kind: &DyeTargetKindV1) -> u32 {
    match kind {
        DyeTargetKindV1::Base => 0,
        DyeTargetKindV1::Mask { number } => number.saturating_add(1),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DyeSessionId(String);

impl DyeSessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn parse_opaque(value: &str) -> Result<Self, String> {
        parse_opaque_id(value, "染色会话 ID").map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeRelativeSelection {
    pub opaque_target_id: DyeTargetId,
    pub color_indices: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelativeSyncScope {
    Family,
    Part,
}

impl RelativeSyncScope {
    fn includes(self, target: &PersistedDyeTargetRefV1, primary: &PersistedDyeTargetRefV1) -> bool {
        match self {
            Self::Family => target_refs_same_layer(target, primary),
            Self::Part => target.slot.trim().eq_ignore_ascii_case(primary.slot.trim()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeRelativeTransform {
    pub hue_degrees: i16,
    pub saturation_percent: i16,
    pub lightness_percent: i16,
    #[serde(default)]
    pub contrast_percent: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum DyeResetScope {
    Color {
        opaque_target_id: DyeTargetId,
        color_index: u16,
    },
    Target {
        opaque_target_id: DyeTargetId,
    },
    Family,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyeResourceStatus {
    Ready,
    PartialReadOnly,
    ReadOnly,
    Incomplete,
    Changed,
    Missing,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DyeTargetId(String);

impl DyeTargetId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn parse_opaque(value: &str) -> Result<Self, String> {
        parse_opaque_id(value, "染色目标 ID").map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyePaletteColorView {
    pub color_index: u16,
    pub original_rgba: [u8; 4],
    pub baseline_rgba: [u8; 4],
    pub current_rgba: [u8; 4],
    pub used_pixel_count: u64,
    pub used_frame_count: u32,
    pub used_frame_bitmap_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyePaletteView {
    pub palette_index: u16,
    pub rgb_encoding: Option<String>,
    pub colors: Vec<DyePaletteColorView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeTargetView {
    pub opaque_target_id: DyeTargetId,
    pub canonical_target_id: DyeTargetId,
    pub reference_count: usize,
    pub is_shared: bool,
    pub slot: String,
    pub layer_label: String,
    pub layer_order: i32,
    pub kind: DyeTargetKindV1,
    pub resource_status: String,
    pub preview_available: bool,
    pub active_palette: u16,
    pub img_version: u16,
    pub frame_count: u32,
    pub capability: DyeCapabilityStatus,
    pub palettes: Vec<DyePaletteView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeSessionView {
    pub session_id: DyeSessionId,
    pub revision: u64,
    pub recipe_revision: String,
    pub slot: String,
    pub layer_label: String,
    pub resource_status: DyeResourceStatus,
    pub editing_blocked: bool,
    pub persistence_blocked: bool,
    pub targets: Vec<DyeTargetView>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub has_dye: bool,
    pub is_dirty: bool,
    pub has_changes: bool,
    pub semantic_state: Option<serde_json::Value>,
    pub baked_legacy: bool,
}

#[derive(Debug, Clone)]
struct PaletteState {
    fingerprint: DyeSourceFingerprintV1,
    colors: Vec<[u8; 4]>,
    inspection: ImgDyeInspection,
    direct_format: Option<DirectColorFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersistedRecipeIssueStatus {
    Changed,
    Missing,
    Conflict,
}

impl PersistedRecipeIssueStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Changed => "changed",
            Self::Missing => "missing",
            Self::Conflict => "conflict",
        }
    }

    fn severity(self) -> u8 {
        match self {
            Self::Changed => 1,
            Self::Missing => 2,
            Self::Conflict => 3,
        }
    }
}

#[derive(Debug, Clone)]
struct PersistedRecipeIssue {
    opaque_target_id: DyeTargetId,
    matched_target_id: Option<DyeTargetId>,
    status: PersistedRecipeIssueStatus,
    target_ref: PersistedDyeTargetRefV1,
    recipes: Vec<DyeTargetRecipeV1>,
    scope: PersistedRecipeIssueScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersistedRecipeIssueScope {
    BakedProjection,
    MaterializationBaseline,
}

#[derive(Debug, Clone)]
struct UnavailableTargetState {
    id: DyeTargetId,
    identity: DyeSessionIdentity,
    layer_order: i32,
    draw_ordinal: usize,
    kind: DyeTargetKindV1,
    resource_status: UnavailableResourceStatus,
    img_version: u16,
    frame_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnavailableResourceStatus {
    Missing,
    Invalid,
}

impl UnavailableResourceStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DyeRestoreReport {
    pub restored_target_count: usize,
    pub changed_target_count: usize,
    pub missing_target_count: usize,
    pub conflict_target_count: usize,
}

#[derive(Debug, Clone)]
pub(super) struct TargetState {
    id: DyeTargetId,
    target_ref: PersistedDyeTargetRefV1,
    source_bytes: Arc<[u8]>,
    source_bytes_hash: [u8; 32],
    entry_identity: [u8; 32],
    layer_order: i32,
    draw_ordinal: usize,
    active_palette: u16,
    img_version: u16,
    frame_count: u32,
    capability: DyeCapabilityStatus,
    palettes: Arc<Vec<PaletteState>>,
    edits: BTreeMap<u16, [u8; 3]>,
}

#[derive(Debug, Clone)]
struct ParsedImgResource {
    source_bytes: Arc<[u8]>,
    source_bytes_hash: [u8; 32],
    img_version: u16,
    frame_count: u32,
    capability: DyeCapabilityStatus,
    palettes: Arc<Vec<PaletteState>>,
}

type ParsedImgCache = HashMap<[u8; 32], Result<Arc<ParsedImgResource>, String>>;

type EditSnapshot = BTreeMap<DyeTargetId, BTreeMap<u16, [u8; 3]>>;
type PhysicalPaletteKey = (String, u16);

#[derive(Debug, Clone)]
struct SessionHistorySnapshot {
    edits: EditSnapshot,
    persisted_recipe_issues: Vec<PersistedRecipeIssue>,
    metadata_dirty: bool,
    semantic_state: Option<serde_json::Value>,
    materialization_baseline: EditSnapshot,
    baked_legacy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicDyeSemanticStateV1 {
    version: u16,
    #[serde(default, alias = "baked_legacy")]
    baked_legacy: bool,
    #[serde(default)]
    all: Option<serde_json::Value>,
    #[serde(default)]
    parts: Vec<PublicSemanticPartV1>,
    #[serde(default)]
    images: Vec<PublicSemanticImageV1>,
    #[serde(default)]
    exact_overrides: Vec<PublicSemanticExactOverrideV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicSemanticPartV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot: Option<String>,
    adjustment: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicSemanticImageV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_id: Option<String>,
    palette_index: u16,
    adjustment: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicSemanticExactOverrideV1 {
    canonical_target_id: String,
    color_index: u16,
    rgb: [u8; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSemanticEnvelopeV1 {
    version: u16,
    state: PersistedSemanticStateV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSemanticStateV1 {
    version: u16,
    #[serde(default)]
    all: Option<serde_json::Value>,
    #[serde(default)]
    parts: Vec<PersistedSemanticPartV1>,
    #[serde(default)]
    images: Vec<PersistedSemanticImageV1>,
    #[serde(default)]
    exact_overrides: Vec<PersistedSemanticExactOverrideV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSemanticImageV1 {
    target: PersistedDyeTargetRefV1,
    palette_index: u16,
    adjustment: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSemanticPartV1 {
    #[serde(default)]
    target: Option<PersistedDyeTargetRefV1>,
    #[serde(default)]
    palette_index: u16,
    #[serde(default)]
    slot: Option<String>,
    adjustment: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSemanticExactOverrideV1 {
    target: PersistedDyeTargetRefV1,
    palette_index: u16,
    color_index: u16,
    rgb: [u8; 3],
}

#[derive(Debug)]
pub struct AvatarDyeSession {
    session_id: DyeSessionId,
    public_slot: String,
    public_layer_label: String,
    covers_full_workbench: bool,
    pub(super) targets: Vec<TargetState>,
    unavailable_targets: Vec<UnavailableTargetState>,
    undo_stack: Vec<SessionHistorySnapshot>,
    redo_stack: Vec<SessionHistorySnapshot>,
    saved_baseline: EditSnapshot,
    materialization_baseline: EditSnapshot,
    persisted_recipe_issues: Vec<PersistedRecipeIssue>,
    metadata_dirty: bool,
    semantic_state: Option<serde_json::Value>,
    baked_legacy: bool,
    revision: u64,
    preview_cache: Arc<Mutex<SessionPreviewCache>>,
}

#[derive(Default)]
pub struct DyeSessionStore {
    sessions: BTreeMap<DyeSessionId, AvatarDyeSession>,
}

impl DyeSessionStore {
    pub fn insert(&mut self, session: AvatarDyeSession) -> Result<DyeSessionId, String> {
        let session_id = session.session_id.clone();
        if self.sessions.insert(session_id.clone(), session).is_some() {
            return Err("染色会话 ID 冲突".to_string());
        }
        Ok(session_id)
    }

    pub fn view(&self, session_id: &DyeSessionId) -> Result<DyeSessionView, String> {
        self.sessions
            .get(session_id)
            .map(AvatarDyeSession::view)
            .ok_or_else(|| "染色会话不存在或已关闭".to_string())
    }

    pub fn session(&self, session_id: &DyeSessionId) -> Result<&AvatarDyeSession, String> {
        self.sessions
            .get(session_id)
            .ok_or_else(|| "染色会话不存在或已关闭".to_string())
    }

    pub fn session_mut(
        &mut self,
        session_id: &DyeSessionId,
    ) -> Result<&mut AvatarDyeSession, String> {
        self.sessions
            .get_mut(session_id)
            .ok_or_else(|| "染色会话不存在或已关闭".to_string())
    }

    pub fn close(&mut self, session_id: &DyeSessionId) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}

impl AvatarDyeSession {
    pub(crate) fn clone_for_transient_preview(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            public_slot: self.public_slot.clone(),
            public_layer_label: self.public_layer_label.clone(),
            covers_full_workbench: self.covers_full_workbench,
            targets: self.targets.clone(),
            unavailable_targets: self.unavailable_targets.clone(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            saved_baseline: self.capture_edits(),
            materialization_baseline: self.materialization_baseline.clone(),
            persisted_recipe_issues: self.persisted_recipe_issues.clone(),
            metadata_dirty: self.metadata_dirty,
            semantic_state: self.semantic_state.clone(),
            baked_legacy: self.baked_legacy,
            revision: self.revision,
            preview_cache: Arc::new(Mutex::new(SessionPreviewCache::default())),
        }
    }

    pub(crate) fn adopt_preview_cache_from(&mut self, source: &Self) {
        self.preview_cache = Arc::clone(&source.preview_cache);
    }

    pub(crate) fn open_workbench(layers: Vec<DyeWorkbenchLayerInput>) -> Result<Self, String> {
        Self::open_workbench_with_unavailable(layers, Vec::new())
    }

    pub(crate) fn open_workbench_with_unavailable(
        layers: Vec<DyeWorkbenchLayerInput>,
        unavailable: Vec<DyeUnavailableTargetInput>,
    ) -> Result<Self, String> {
        let mut combined_order = layers
            .iter()
            .enumerate()
            .map(|(index, layer)| (layer.layer_order, false, index))
            .chain(
                unavailable
                    .iter()
                    .enumerate()
                    .map(|(index, target)| (target.layer_order, true, index)),
            )
            .collect::<Vec<_>>();
        combined_order.sort_by_key(|(layer_order, _, _)| *layer_order);
        let mut layer_ordinals = vec![0usize; layers.len()];
        let mut unavailable_ordinals = vec![0usize; unavailable.len()];
        for (draw_ordinal, (_, is_unavailable, index)) in combined_order.into_iter().enumerate() {
            if is_unavailable {
                unavailable_ordinals[index] = draw_ordinal;
            } else {
                layer_ordinals[index] = draw_ordinal;
            }
        }
        Self::open_positioned_workbench(
            layers
                .into_iter()
                .enumerate()
                .map(|(index, input)| PositionedDyeWorkbenchLayerInput {
                    draw_ordinal: layer_ordinals[index],
                    input,
                })
                .collect(),
            unavailable
                .into_iter()
                .enumerate()
                .map(|(index, input)| PositionedDyeUnavailableTargetInput {
                    draw_ordinal: unavailable_ordinals[index],
                    input,
                })
                .collect(),
        )
    }

    pub(crate) fn open_positioned_workbench(
        layers: Vec<PositionedDyeWorkbenchLayerInput>,
        mut unavailable: Vec<PositionedDyeUnavailableTargetInput>,
    ) -> Result<Self, String> {
        if layers.is_empty() && unavailable.is_empty() {
            return Err("染色工作台没有 AvatarLayer".to_string());
        }
        if layers
            .len()
            .checked_add(unavailable.len())
            .is_none_or(|count| count > 512)
        {
            return Err("染色工作台图层数量超过安全上限".to_string());
        }
        for target in &mut unavailable {
            target.input.identity = normalize_identity(target.input.identity.clone())?;
        }
        let identities = layers
            .iter()
            .map(|layer| &layer.input.identity)
            .chain(unavailable.iter().map(|target| &target.input.identity))
            .collect::<Vec<_>>();
        let aggregate_nonce = unique_session_nonce();
        let aggregate_slot = if identities.iter().all(|identity| {
            identity
                .slot
                .trim()
                .eq_ignore_ascii_case(identities[0].slot.trim())
        }) {
            identities[0].slot.trim().to_ascii_lowercase()
        } else {
            "multiple".to_string()
        };
        let aggregate_label = if identities.len() == 1 {
            identities[0].layer_tag.trim().to_string()
        } else {
            "完整搭配".to_string()
        };
        let mut targets = Vec::new();
        let mut unavailable_targets = Vec::new();
        let mut parsed_img_cache = ParsedImgCache::new();
        for positioned in layers {
            let draw_ordinal = positioned.draw_ordinal;
            let layer = positioned.input;
            let identity = layer.identity.clone();
            let requested_source = layer.entries.iter().find(|(entry_ref, _)| {
                entry_ref.eq_ignore_ascii_case(layer.requested_base_entry.trim())
            });
            let source_present = requested_source.is_some();
            let source_metadata =
                requested_source.and_then(|(_, source_bytes)| img_header_metadata(source_bytes));
            let family = Self::open_from_entries_with_cache(
                layer.identity,
                &layer.requested_base_entry,
                layer.active_palette,
                layer.entries,
                &mut parsed_img_cache,
            );
            match family {
                Ok(mut family) => {
                    for target in &mut family.targets {
                        target.layer_order = layer.layer_order;
                        target.draw_ordinal = draw_ordinal;
                    }
                    for target in &mut family.unavailable_targets {
                        target.layer_order = layer.layer_order;
                        target.draw_ordinal = draw_ordinal;
                    }
                    targets.extend(family.targets);
                    unavailable_targets.extend(family.unavailable_targets);
                }
                Err(_) => {
                    let (img_version, frame_count) = source_metadata.unwrap_or_default();
                    unavailable_targets.push(UnavailableTargetState {
                        id: make_target_id(aggregate_nonce, 0, &DyeTargetKindV1::Base),
                        identity: normalize_identity(identity)?,
                        layer_order: layer.layer_order,
                        draw_ordinal,
                        kind: DyeTargetKindV1::Base,
                        resource_status: if source_present {
                            UnavailableResourceStatus::Invalid
                        } else {
                            UnavailableResourceStatus::Missing
                        },
                        img_version,
                        frame_count,
                    });
                }
            }
        }
        unavailable_targets.extend(unavailable.into_iter().map(|positioned| {
            UnavailableTargetState {
                id: make_target_id(aggregate_nonce, 0, &positioned.input.kind),
                identity: positioned.input.identity,
                layer_order: positioned.input.layer_order,
                draw_ordinal: positioned.draw_ordinal,
                kind: positioned.input.kind,
                resource_status: UnavailableResourceStatus::Missing,
                img_version: 0,
                frame_count: 0,
            }
        }));
        if targets
            .len()
            .checked_add(unavailable_targets.len())
            .is_none_or(|count| count > 2_048)
        {
            return Err("染色工作台目标数量超过安全上限".to_string());
        }
        targets.sort_by_key(|target| {
            (
                target.draw_ordinal,
                target_kind_ordinal(&target.target_ref.target_kind),
            )
        });
        for (index, target) in targets.iter_mut().enumerate() {
            target.id = make_target_id(aggregate_nonce, index, &target.target_ref.target_kind);
        }
        unavailable_targets
            .sort_by_key(|target| (target.draw_ordinal, target_kind_ordinal(&target.kind)));
        for (offset, target) in unavailable_targets.iter_mut().enumerate() {
            target.id = make_target_id(
                aggregate_nonce,
                targets.len().saturating_add(offset),
                &target.kind,
            );
        }
        let saved_baseline: EditSnapshot = targets
            .iter()
            .map(|target| (target.id.clone(), BTreeMap::new()))
            .collect();
        Ok(Self {
            session_id: make_session_id(aggregate_nonce),
            public_slot: aggregate_slot,
            public_layer_label: aggregate_label,
            covers_full_workbench: true,
            targets,
            unavailable_targets,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            materialization_baseline: saved_baseline.clone(),
            saved_baseline,
            persisted_recipe_issues: Vec::new(),
            metadata_dirty: false,
            semantic_state: None,
            baked_legacy: false,
            revision: 1,
            preview_cache: Arc::new(Mutex::new(SessionPreviewCache::default())),
        })
    }

    pub(crate) fn open_from_entries(
        identity: DyeSessionIdentity,
        requested_base_entry: &str,
        base_active_palette: u16,
        entries: Vec<(String, Vec<u8>)>,
    ) -> Result<Self, String> {
        Self::open_from_entries_with_cache(
            identity,
            requested_base_entry,
            base_active_palette,
            entries,
            &mut ParsedImgCache::new(),
        )
    }

    fn open_from_entries_with_cache(
        identity: DyeSessionIdentity,
        requested_base_entry: &str,
        base_active_palette: u16,
        entries: Vec<(String, Vec<u8>)>,
        parsed_img_cache: &mut ParsedImgCache,
    ) -> Result<Self, String> {
        let identity = normalize_identity(identity)?;
        let entry_names = entries
            .iter()
            .map(|(entry_ref, _)| entry_ref.clone())
            .collect::<Vec<_>>();
        let family = discover_resource_family_entries(&entry_names, requested_base_entry)?;
        let session_nonce = unique_session_nonce();
        let mut targets = Vec::with_capacity(family.len());
        let mut unavailable_targets = Vec::new();

        for (family_index, member) in family.into_iter().enumerate() {
            let (_, source_bytes) = entries
                .get(member.entry_index)
                .ok_or_else(|| "染色资源族入口索引无效".to_string())?;
            let kind = member.kind;
            let (img_version, frame_count) = img_header_metadata(source_bytes).unwrap_or_default();
            match build_target_state(
                &identity,
                session_nonce,
                family_index,
                kind.clone(),
                member.entry_ref,
                source_bytes,
                base_active_palette,
                parsed_img_cache,
            ) {
                Ok(target) => targets.push(target),
                Err(_) => unavailable_targets.push(UnavailableTargetState {
                    id: make_target_id(session_nonce, family_index, &kind),
                    identity: identity.clone(),
                    layer_order: 0,
                    draw_ordinal: 0,
                    kind,
                    resource_status: UnavailableResourceStatus::Invalid,
                    img_version,
                    frame_count,
                }),
            }
        }

        let saved_baseline: EditSnapshot = targets
            .iter()
            .map(|target| (target.id.clone(), BTreeMap::new()))
            .collect();
        Ok(Self {
            session_id: make_session_id(session_nonce),
            public_slot: identity.slot,
            public_layer_label: identity.layer_tag,
            covers_full_workbench: false,
            targets,
            unavailable_targets,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            materialization_baseline: saved_baseline.clone(),
            saved_baseline,
            persisted_recipe_issues: Vec::new(),
            metadata_dirty: false,
            semantic_state: None,
            baked_legacy: false,
            revision: 1,
            preview_cache: Arc::new(Mutex::new(SessionPreviewCache::default())),
        })
    }

    pub fn view(&self) -> DyeSessionView {
        let has_dye = self.targets.iter().any(|target| !target.edits.is_empty());
        let current = self.capture_edits();
        let current_conflicts = self.current_conflicting_target_ids();
        let mut public_physical_identities = BTreeMap::<DyeTargetId, (DyeTargetId, usize)>::new();
        for indices in physical_alias_groups(&self.targets).into_values() {
            let canonical_target_id = self.targets[indices[0]].id.clone();
            let reference_count = indices.len();
            for index in indices {
                public_physical_identities.insert(
                    self.targets[index].id.clone(),
                    (canonical_target_id.clone(), reference_count),
                );
            }
        }
        let mut positioned_targets = self
            .targets
            .iter()
            .map(|target| {
                let (canonical_target_id, reference_count) = public_physical_identities
                    .get(&target.id)
                    .cloned()
                    .unwrap_or_else(|| (target.id.clone(), 1));
                (
                    (
                        target.draw_ordinal,
                        target_kind_ordinal(&target.target_ref.target_kind),
                    ),
                    target.view(
                        self.target_resource_status(&target.id, &current_conflicts),
                        canonical_target_id,
                        reference_count,
                        self.materialization_baseline.get(&target.id),
                    ),
                )
            })
            .collect::<Vec<_>>();
        positioned_targets.extend(
            self.persisted_recipe_issues
                .iter()
                .filter(|issue| issue.matched_target_id.is_none())
                .map(|issue| ((usize::MAX, 0), issue.placeholder_view())),
        );
        positioned_targets.extend(self.unavailable_targets.iter().map(|target| {
            (
                (target.draw_ordinal, target_kind_ordinal(&target.kind)),
                target.placeholder_view(),
            )
        }));
        positioned_targets.sort_by_key(|(position, _)| *position);
        let targets = positioned_targets
            .into_iter()
            .map(|(_, target)| target)
            .collect();
        let editing_blocked =
            self.has_unresolved_persisted_targets() || !current_conflicts.is_empty();
        DyeSessionView {
            session_id: self.session_id.clone(),
            revision: self.revision,
            recipe_revision: self.recipe_revision(),
            slot: self.public_slot.clone(),
            layer_label: self.public_layer_label.clone(),
            resource_status: self.resource_status(&current_conflicts),
            editing_blocked,
            persistence_blocked: editing_blocked || self.has_incomplete_workbench(),
            targets,
            can_undo: !self.undo_stack.is_empty(),
            can_redo: !self.redo_stack.is_empty(),
            has_dye,
            is_dirty: current != self.saved_baseline || self.metadata_dirty,
            has_changes: has_dye,
            semantic_state: self.semantic_state.clone(),
            baked_legacy: self.baked_legacy,
        }
    }

    pub fn set_rgb(
        &mut self,
        target_id: &DyeTargetId,
        color_index: u16,
        replacement_rgb: [u8; 3],
    ) -> Result<bool, String> {
        self.edit_color(target_id, color_index, replacement_rgb)
    }

    pub fn set_rgb_many(
        &mut self,
        target_id: &DyeTargetId,
        color_indices: &[u16],
        replacement_rgb: [u8; 3],
    ) -> Result<bool, String> {
        let color_indices = unique_color_indices(color_indices, "批量染色")?;
        let target_id = target_id.clone();
        self.apply_mutation(move |targets| {
            let target = find_target_mut(targets, &target_id)?;
            for color_index in color_indices {
                target.set_color(color_index, replacement_rgb)?;
            }
            Ok(())
        })
    }

    /// Replaces the complete sparse edit map for every selected physical IMG
    /// palette. Omitted colors come from the session's fixed materialization
    /// baseline, so repeated previews or commits never become a new baseline.
    pub fn replace_color_tables_atomic(
        &mut self,
        mutation: &DyeBatchMutation,
    ) -> Result<bool, String> {
        if mutation.targets.len() > MAX_BATCH_TARGETS {
            return Err("批量染色目标数量超过安全上限".to_string());
        }
        if !self.current_conflicting_target_ids().is_empty() {
            return Err(
                "同一 IMG 与 Palette 的当前编辑存在冲突；请先撤销或恢复全部染色".to_string(),
            );
        }

        let semantic_state =
            validate_public_semantic_state(self, mutation.semantic_state.as_ref())?;
        let groups = physical_alias_groups(&self.targets);
        let mut desired_by_physical = BTreeMap::<PhysicalPaletteKey, BTreeMap<u16, [u8; 3]>>::new();

        for replacement in &mutation.targets {
            if replacement.colors.len() > MAX_COLOR_SELECTION_INPUTS {
                return Err("单个色表的批量颜色数量超过安全上限".to_string());
            }
            let target = self
                .targets
                .iter()
                .find(|target| target.id == replacement.target_id)
                .ok_or_else(|| "染色目标句柄无效或已过期".to_string())?;
            target.ensure_editable()?;
            if replacement.palette_index != target.active_palette {
                return Err("批量染色 Palette 与目标当前生效 Palette 不一致".to_string());
            }
            let palette = target.active_palette_state()?;
            // The IPC payload is the complete sparse map relative to the
            // immutable source palette. Baseline colors are exposed to the UI
            // for calculation only; they must never be implicitly carried
            // into the next materialization.
            let mut desired = BTreeMap::new();
            let mut seen = BTreeSet::new();
            for color in &replacement.colors {
                if !seen.insert(color.color_index) {
                    return Err("批量染色色表包含重复颜色索引".to_string());
                }
                let original = palette
                    .colors
                    .get(usize::from(color.color_index))
                    .copied()
                    .ok_or_else(|| "批量染色颜色索引无效".to_string())?;
                if original[3] == 0 {
                    return Err("完全透明颜色不可修改".to_string());
                }
                let replacement_rgb = palette
                    .direct_format
                    .map_or(color.rgb, |format| format.quantize_rgb(color.rgb));
                if replacement_rgb == original[..3] {
                    desired.remove(&color.color_index);
                } else {
                    desired.insert(color.color_index, replacement_rgb);
                }
            }

            let key = (
                normalized_private_entry(&target.target_ref.private_entry_ref),
                target.active_palette,
            );
            if let Some(existing) = desired_by_physical.insert(key, desired.clone()) {
                if existing != desired {
                    return Err("同一 IMG 与 Palette 的批量替换内容存在冲突".to_string());
                }
            }
        }

        let before = self.capture_history_snapshot();
        let mut next = before.edits.clone();
        for (key, desired) in desired_by_physical {
            let indices = groups
                .get(&key)
                .ok_or_else(|| "批量染色物理目标已失效".to_string())?;
            ensure_physical_alias_sources_match(&self.targets, indices)?;
            for index in indices {
                next.insert(self.targets[*index].id.clone(), desired.clone());
            }
        }
        canonicalize_physical_alias_snapshot(&self.targets, &mut next)?;

        if before.edits == next && before.semantic_state == semantic_state {
            return Ok(false);
        }
        self.restore_edits(&next);
        self.semantic_state = semantic_state;
        self.metadata_dirty = true;
        push_history(&mut self.undo_stack, before);
        self.redo_stack.clear();
        self.bump_revision();
        self.invalidate_preview_cache();
        Ok(true)
    }

    pub fn edit_color(
        &mut self,
        target_id: &DyeTargetId,
        color_index: u16,
        replacement_rgb: [u8; 3],
    ) -> Result<bool, String> {
        let target_id = target_id.clone();
        self.apply_mutation(move |targets| {
            let target = find_target_mut(targets, &target_id)?;
            target.set_color(color_index, replacement_rgb)
        })
    }

    pub fn copy_reference_palette(
        &mut self,
        target_id: &DyeTargetId,
        reference_palette: u16,
    ) -> Result<bool, String> {
        let target_id = target_id.clone();
        self.apply_mutation(move |targets| {
            let target = find_target_mut(targets, &target_id)?;
            target.ensure_editable()?;
            if reference_palette == target.active_palette {
                return Ok(());
            }
            let reference_colors = target
                .palettes
                .get(usize::from(reference_palette))
                .ok_or_else(|| "参考 Palette 索引无效".to_string())?
                .colors
                .clone();
            let active_colors = target.active_palette_state()?.colors.clone();
            if reference_colors.len() != active_colors.len() {
                return Err("参考 Palette 与当前 Palette 的颜色数量不同".to_string());
            }
            for (color_index, (reference, active)) in
                reference_colors.iter().zip(active_colors).enumerate()
            {
                if active[3] == 0 {
                    continue;
                }
                target.set_color(
                    u16::try_from(color_index)
                        .map_err(|_| "Palette 颜色索引超过安全上限".to_string())?,
                    [reference[0], reference[1], reference[2]],
                )?;
            }
            Ok(())
        })
    }

    pub fn apply_relative_sync(
        &mut self,
        selections: &[DyeRelativeSelection],
        transform: DyeRelativeTransform,
    ) -> Result<bool, String> {
        validate_transform(transform)?;
        if selections.is_empty() {
            return Err("相对同步没有选择染色目标".to_string());
        }
        let mut selected = BTreeMap::<DyeTargetId, BTreeSet<u16>>::new();
        for selection in selections {
            let color_indices = unique_color_indices(&selection.color_indices, "相对同步")?;
            let target_colors = selected
                .entry(selection.opaque_target_id.clone())
                .or_default();
            target_colors.extend(color_indices);
            if target_colors.len() > MAX_COLOR_SELECTION_INPUTS {
                return Err("相对同步颜色项数量超过安全上限".to_string());
            }
        }

        self.apply_mutation(move |targets| {
            for (target_id, color_indices) in selected {
                let target = find_target_mut(targets, &target_id)?;
                target.ensure_editable()?;
                for color_index in color_indices {
                    let current = target.current_rgb(color_index)?;
                    target.set_color(color_index, apply_hsl_transform(current, transform))?;
                }
            }
            Ok(())
        })
    }

    pub fn apply_target_relative_hsl(
        &mut self,
        target_id: &DyeTargetId,
        transform: DyeRelativeTransform,
    ) -> Result<bool, String> {
        validate_transform(transform)?;
        let target_id = target_id.clone();
        self.apply_mutation(move |targets| {
            let target = find_target_mut(targets, &target_id)?;
            let color_indices = target.used_opaque_color_indices()?;
            for color_index in color_indices {
                let current = target.current_rgb(color_index)?;
                target.set_color(color_index, apply_hsl_transform(current, transform))?;
            }
            Ok(())
        })
    }

    pub fn sync_family_relative_hsl(
        &mut self,
        primary_target_id: &DyeTargetId,
        primary_color_indices: &[u16],
        transform: DyeRelativeTransform,
    ) -> Result<bool, String> {
        self.sync_scoped_relative_hsl(
            primary_target_id,
            primary_color_indices,
            transform,
            RelativeSyncScope::Family,
        )
    }

    pub fn sync_part_relative_hsl(
        &mut self,
        primary_target_id: &DyeTargetId,
        primary_color_indices: &[u16],
        transform: DyeRelativeTransform,
    ) -> Result<bool, String> {
        self.sync_scoped_relative_hsl(
            primary_target_id,
            primary_color_indices,
            transform,
            RelativeSyncScope::Part,
        )
    }

    fn sync_scoped_relative_hsl(
        &mut self,
        primary_target_id: &DyeTargetId,
        primary_color_indices: &[u16],
        transform: DyeRelativeTransform,
        scope: RelativeSyncScope,
    ) -> Result<bool, String> {
        validate_transform(transform)?;
        let primary_color_indices = unique_color_indices(primary_color_indices, "相对同步染色")?;
        let primary_target_id = primary_target_id.clone();

        self.apply_mutation(move |targets| {
            let primary_index = targets
                .iter()
                .position(|target| target.id == primary_target_id)
                .ok_or_else(|| "染色目标句柄无效或已过期".to_string())?;
            let family_ref = targets[primary_index].target_ref.clone();
            let primary_colors = primary_color_indices
                .iter()
                .map(|color_index| targets[primary_index].current_rgb(*color_index))
                .collect::<Result<Vec<_>, String>>()?;

            {
                let primary = &mut targets[primary_index];
                for color_index in primary_color_indices {
                    let current = primary.current_rgb(color_index)?;
                    primary.set_color(color_index, apply_hsl_transform(current, transform))?;
                }
            }

            for (target_index, target) in targets.iter_mut().enumerate() {
                if target_index == primary_index
                    || !scope.includes(&target.target_ref, &family_ref)
                    || target.capability == DyeCapabilityStatus::ReadOnly
                {
                    continue;
                }
                let candidates = target.used_opaque_colors()?;
                let color_indices = matching_family_zone_indices(&primary_colors, &candidates);
                for color_index in color_indices {
                    let current = target.current_rgb(color_index)?;
                    target.set_color(color_index, apply_hsl_transform(current, transform))?;
                }
            }
            Ok(())
        })
    }

    pub fn sync_resource_family_relative_hsl(
        &mut self,
        primary_target_id: &DyeTargetId,
        primary_color_indices: &[u16],
        transform: DyeRelativeTransform,
    ) -> Result<bool, String> {
        self.sync_family_relative_hsl(primary_target_id, primary_color_indices, transform)
    }

    pub fn reset_active_palette(&mut self, target_id: &DyeTargetId) -> Result<bool, String> {
        self.reset_target(target_id)
    }

    pub fn reset_colors(
        &mut self,
        target_id: &DyeTargetId,
        color_indices: &[u16],
    ) -> Result<bool, String> {
        let color_indices = unique_color_indices(color_indices, "重置颜色")?;
        let target_id = target_id.clone();
        self.apply_mutation(move |targets| {
            let target = find_target_mut(targets, &target_id)?;
            for color_index in &color_indices {
                target.current_rgb(*color_index)?;
            }
            for color_index in color_indices {
                target.edits.remove(&color_index);
            }
            Ok(())
        })
    }

    pub fn reset_colors_atomic(
        &mut self,
        target_id: &DyeTargetId,
        color_indices: &[u16],
    ) -> Result<bool, String> {
        self.reset_colors(target_id, color_indices)
    }

    pub fn reset_target(&mut self, target_id: &DyeTargetId) -> Result<bool, String> {
        self.reset(DyeResetScope::Target {
            opaque_target_id: target_id.clone(),
        })
    }

    pub fn reset_target_atomic(&mut self, target_id: &DyeTargetId) -> Result<bool, String> {
        self.reset_target(target_id)
    }

    pub fn reset_part(&mut self, target_id: &DyeTargetId) -> Result<bool, String> {
        let target_id = target_id.clone();
        self.apply_mutation(move |targets| {
            let slot = targets
                .iter()
                .find(|target| target.id == target_id)
                .map(|target| target.target_ref.slot.clone())
                .ok_or_else(|| "染色目标句柄无效或已过期".to_string())?;
            for target in targets.iter_mut().filter(|target| {
                target
                    .target_ref
                    .slot
                    .trim()
                    .eq_ignore_ascii_case(slot.trim())
            }) {
                target.edits.clear();
            }
            Ok(())
        })
    }

    pub fn reset_part_atomic(&mut self, target_id: &DyeTargetId) -> Result<bool, String> {
        self.reset_part(target_id)
    }

    pub fn reset_all(&mut self) -> bool {
        self.reset_all_atomic().unwrap_or(false)
    }

    pub fn reset_all_atomic(&mut self) -> Result<bool, String> {
        let before = self.capture_history_snapshot();
        let had_state = before.edits.values().any(|edits| !edits.is_empty())
            || !before.persisted_recipe_issues.is_empty()
            || before.semantic_state.is_some()
            || before
                .materialization_baseline
                .values()
                .any(|edits| !edits.is_empty())
            || before.baked_legacy;
        if !had_state {
            return Ok(false);
        }
        for target in &mut self.targets {
            target.edits.clear();
        }
        self.materialization_baseline = self
            .targets
            .iter()
            .map(|target| (target.id.clone(), BTreeMap::new()))
            .collect();
        self.persisted_recipe_issues.clear();
        self.semantic_state = None;
        self.baked_legacy = false;
        self.metadata_dirty = true;
        push_history(&mut self.undo_stack, before);
        self.redo_stack.clear();
        self.bump_revision();
        self.invalidate_preview_cache();
        Ok(true)
    }

    pub fn reset(&mut self, scope: DyeResetScope) -> Result<bool, String> {
        if matches!(scope, DyeResetScope::Family) {
            return self.reset_all_atomic();
        }
        self.apply_mutation(move |targets| {
            match scope {
                DyeResetScope::Color {
                    opaque_target_id,
                    color_index,
                } => {
                    let target = find_target_mut(targets, &opaque_target_id)?;
                    target.current_rgb(color_index)?;
                    target.edits.remove(&color_index);
                }
                DyeResetScope::Target { opaque_target_id } => {
                    find_target_mut(targets, &opaque_target_id)?.edits.clear();
                }
                DyeResetScope::Family => unreachable!("family reset is handled before mutation"),
            }
            Ok(())
        })
    }

    pub fn mark_clean(&mut self) {
        let current = self.capture_edits();
        if current != self.saved_baseline || self.metadata_dirty {
            self.saved_baseline = current;
            self.metadata_dirty = false;
            self.bump_revision();
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.pop() else {
            return false;
        };
        let current = self.capture_history_snapshot();
        push_history(&mut self.redo_stack, current);
        self.restore_history_snapshot(&previous);
        self.bump_revision();
        self.invalidate_preview_cache();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = self.capture_history_snapshot();
        push_history(&mut self.undo_stack, current);
        self.restore_history_snapshot(&next);
        self.bump_revision();
        self.invalidate_preview_cache();
        true
    }

    pub(crate) fn recipe(&self) -> Result<DyeRecipeV1, String> {
        self.recipe_from_snapshot(&self.capture_edits())
    }

    fn recipe_from_snapshot(&self, snapshot: &EditSnapshot) -> Result<DyeRecipeV1, String> {
        let mut targets = Vec::new();
        for target in &self.targets {
            let edits = snapshot.get(&target.id).cloned().unwrap_or_default();
            if edits.is_empty() {
                continue;
            }
            let palette = target.active_palette_state()?;
            let colors = edits
                .iter()
                .map(|(color_index, replacement_rgb)| {
                    let expected_rgba = palette
                        .colors
                        .get(usize::from(*color_index))
                        .copied()
                        .ok_or_else(|| "染色颜色索引无效".to_string())?;
                    Ok(DyeColorEditV1 {
                        color_index: *color_index,
                        expected_rgba,
                        replacement_rgb: *replacement_rgb,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            targets.push(DyeTargetRecipeV1 {
                target: target.target_ref.clone(),
                palette_index: target.active_palette,
                source: palette.fingerprint.clone(),
                colors,
            });
        }
        canonicalize_recipe(DyeRecipeV1 {
            schema_version: 1,
            targets,
            recipe_hash: [0; 32],
        })
    }

    pub(crate) fn persisted_dye_state(&self) -> Result<PersistedDyeStateV2, String> {
        let baked_recipe = self.recipe()?;
        let baseline_recipe = if self.semantic_state.is_none() {
            (!baked_recipe.is_empty()).then(|| baked_recipe.clone())
        } else {
            let baseline = self.recipe_from_snapshot(&self.materialization_baseline)?;
            (!baseline.is_empty()).then_some(baseline)
        };
        let semantic_state_json = self
            .semantic_state
            .as_ref()
            .map(|semantic| project_public_semantic_for_persistence(self, semantic))
            .transpose()?;
        Ok(PersistedDyeStateV2 {
            schema_version: 2,
            baked_recipe,
            baseline_recipe,
            semantic_state_json,
        })
    }

    pub(crate) fn restore_persisted_dye_state_best_effort(
        &mut self,
        state: &PersistedDyeStateV2,
    ) -> Result<DyeRestoreReport, String> {
        if state.schema_version != 2 {
            return Err("不支持的染色状态版本".to_string());
        }
        let semantic_state = state.semantic_state_json.as_deref().and_then(|json| {
            restore_public_semantic_from_persistence(
                self,
                json,
                state
                    .baseline_recipe
                    .as_ref()
                    .is_some_and(|recipe| !recipe.is_empty()),
            )
            .ok()
        });
        let has_editable_semantics = semantic_state.is_some();
        let (mut materialization_baseline, mut baseline_issues) = if has_editable_semantics {
            if let Some(recipe) = state.baseline_recipe.as_ref() {
                let mut baseline_session = self.clone_for_transient_preview();
                baseline_session.restore_persisted_recipe_best_effort(recipe)?;
                (
                    baseline_session.capture_edits(),
                    baseline_session.persisted_recipe_issues,
                )
            } else {
                (
                    self.targets
                        .iter()
                        .map(|target| (target.id.clone(), BTreeMap::new()))
                        .collect(),
                    Vec::new(),
                )
            }
        } else {
            (
                self.targets
                    .iter()
                    .map(|target| (target.id.clone(), BTreeMap::new()))
                    .collect(),
                Vec::new(),
            )
        };
        for issue in &mut baseline_issues {
            issue.scope = PersistedRecipeIssueScope::MaterializationBaseline;
        }

        let mut report = self.restore_persisted_recipe_best_effort(&state.baked_recipe)?;
        if has_editable_semantics {
            merge_persisted_recipe_issues(&mut self.persisted_recipe_issues, baseline_issues);
            report.changed_target_count = unique_persisted_issue_count(
                &self.persisted_recipe_issues,
                PersistedRecipeIssueStatus::Changed,
            );
            report.missing_target_count = unique_persisted_issue_count(
                &self.persisted_recipe_issues,
                PersistedRecipeIssueStatus::Missing,
            );
            report.conflict_target_count = unique_persisted_issue_count(
                &self.persisted_recipe_issues,
                PersistedRecipeIssueStatus::Conflict,
            );
        } else {
            materialization_baseline = self.capture_edits();
        }
        self.materialization_baseline = materialization_baseline;
        self.semantic_state = semantic_state;
        self.baked_legacy = if has_editable_semantics {
            state
                .baseline_recipe
                .as_ref()
                .is_some_and(|recipe| !recipe.is_empty())
        } else {
            !state.baked_recipe.is_empty()
        };
        self.saved_baseline = self.capture_edits();
        self.metadata_dirty = false;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.bump_revision();
        self.invalidate_preview_cache();
        Ok(report)
    }

    pub(crate) fn apply_persisted_recipe(&mut self, recipe: &DyeRecipeV1) -> Result<(), String> {
        let recipe = canonicalize_recipe(recipe.clone())
            .map_err(|error| format!("染色配方无效: {error}"))?;
        let mut next = self
            .targets
            .iter()
            .map(|target| (target.id.clone(), BTreeMap::new()))
            .collect::<EditSnapshot>();

        for persisted in &recipe.targets {
            let same_layer = self
                .targets
                .iter()
                .any(|target| target_refs_same_layer(&target.target_ref, &persisted.target));
            if !same_layer {
                if self.covers_full_workbench {
                    return Err("染色资源已变化：配方目标已不在当前工作台".to_string());
                }
                continue;
            }
            let target = self
                .targets
                .iter()
                .find(|target| target_refs_match(&target.target_ref, &persisted.target))
                .ok_or_else(|| "染色资源已变化：资源族入口不再匹配".to_string())?;
            if persisted.palette_index != target.active_palette {
                return Err("染色资源已变化：实际生效 Palette 不再匹配".to_string());
            }
            let palette = target
                .active_palette_state()
                .map_err(|_| "染色资源已变化：Palette 不再可用".to_string())?;
            if persisted.source != palette.fingerprint {
                return Err("染色资源已变化：IMG 或 Palette 指纹不一致".to_string());
            }
            let direct_format = palette.direct_format;
            build_patched_img_bytes(&target.source_bytes, persisted)
                .map_err(|_| "染色资源已变化：颜色原值不一致".to_string())?;
            let edits = next
                .get_mut(&target.id)
                .ok_or_else(|| "染色会话内部目标缺失".to_string())?;
            for edit in &persisted.colors {
                let replacement_rgb = direct_format.map_or(edit.replacement_rgb, |format| {
                    format.quantize_rgb(edit.replacement_rgb)
                });
                if let Some(existing) = edits.insert(edit.color_index, replacement_rgb) {
                    if existing != replacement_rgb {
                        return Err("染色配方包含冲突颜色".to_string());
                    }
                }
            }
        }

        canonicalize_physical_alias_snapshot(&self.targets, &mut next)?;
        self.restore_edits(&next);
        self.saved_baseline = next.clone();
        self.materialization_baseline = next;
        self.persisted_recipe_issues.clear();
        self.metadata_dirty = false;
        self.semantic_state = None;
        self.baked_legacy = !recipe.is_empty();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.bump_revision();
        self.invalidate_preview_cache();
        Ok(())
    }

    pub(crate) fn restore_persisted_recipe_best_effort(
        &mut self,
        recipe: &DyeRecipeV1,
    ) -> Result<DyeRestoreReport, String> {
        let candidates = normalize_restore_candidates(recipe)?;
        let mut next = self
            .targets
            .iter()
            .map(|target| (target.id.clone(), BTreeMap::new()))
            .collect::<EditSnapshot>();
        let mut issues = Vec::new();
        let mut report = DyeRestoreReport::default();
        let mut handled_conflicts = BTreeSet::new();

        for (candidate_index, candidate) in candidates.iter().enumerate() {
            if !candidate.conflicted || handled_conflicts.contains(&candidate_index) {
                continue;
            }
            let conflict_indices = candidates
                .iter()
                .enumerate()
                .filter(|(_, other)| {
                    other.conflicted
                        && target_refs_match(&candidate.recipe.target, &other.recipe.target)
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            handled_conflicts.extend(conflict_indices.iter().copied());
            let recipes = conflict_indices
                .iter()
                .map(|index| candidates[*index].recipe.clone())
                .collect::<Vec<_>>();
            let matched_target_id = self
                .targets
                .iter()
                .find(|target| target_refs_match(&target.target_ref, &candidate.recipe.target))
                .map(|target| target.id.clone());
            let opaque_target_id = matched_target_id.clone().unwrap_or_else(|| {
                make_persisted_issue_id(
                    &self.session_id,
                    issues.len(),
                    &candidate.recipe.target.target_kind,
                )
            });
            issues.push(PersistedRecipeIssue {
                opaque_target_id,
                matched_target_id,
                status: PersistedRecipeIssueStatus::Conflict,
                target_ref: candidate.recipe.target.clone(),
                recipes,
                scope: PersistedRecipeIssueScope::BakedProjection,
            });
            report.conflict_target_count += 1;
        }

        for candidate in candidates.iter().filter(|candidate| !candidate.conflicted) {
            let persisted = &candidate.recipe;
            let matched_target = self
                .targets
                .iter()
                .find(|target| target_refs_match(&target.target_ref, &persisted.target))
                .or_else(|| {
                    self.targets.iter().find(|target| {
                        target_matches_persisted_physical(
                            target,
                            &persisted.target,
                            persisted.palette_index,
                        )
                    })
                });
            let Some(target) = matched_target else {
                issues.push(PersistedRecipeIssue {
                    opaque_target_id: make_persisted_issue_id(
                        &self.session_id,
                        issues.len(),
                        &persisted.target.target_kind,
                    ),
                    matched_target_id: None,
                    status: PersistedRecipeIssueStatus::Missing,
                    target_ref: persisted.target.clone(),
                    recipes: vec![persisted.clone()],
                    scope: PersistedRecipeIssueScope::BakedProjection,
                });
                report.missing_target_count += 1;
                continue;
            };

            let is_valid = persisted.palette_index == target.active_palette
                && target
                    .active_palette_state()
                    .is_ok_and(|palette| persisted.source == palette.fingerprint)
                && build_patched_img_bytes(&target.source_bytes, persisted).is_ok();
            if !is_valid {
                issues.push(PersistedRecipeIssue {
                    opaque_target_id: target.id.clone(),
                    matched_target_id: Some(target.id.clone()),
                    status: PersistedRecipeIssueStatus::Changed,
                    target_ref: persisted.target.clone(),
                    recipes: vec![persisted.clone()],
                    scope: PersistedRecipeIssueScope::BakedProjection,
                });
                report.changed_target_count += 1;
                continue;
            }

            let direct_format = target
                .active_palette_state()
                .map_err(|_| "染色资源已变化：Palette 不再可用".to_string())?
                .direct_format;
            let edits = next
                .get_mut(&target.id)
                .ok_or_else(|| "染色会话内部目标缺失".to_string())?;
            for edit in &persisted.colors {
                let replacement_rgb = direct_format.map_or(edit.replacement_rgb, |format| {
                    format.quantize_rgb(edit.replacement_rgb)
                });
                if let Some(existing) = edits.insert(edit.color_index, replacement_rgb) {
                    if existing != replacement_rgb {
                        return Err("染色配方包含冲突颜色".to_string());
                    }
                }
            }
            report.restored_target_count += 1;
        }

        canonicalize_physical_alias_snapshot(&self.targets, &mut next)?;
        self.restore_edits(&next);
        self.saved_baseline = next.clone();
        self.materialization_baseline = next;
        self.persisted_recipe_issues = issues;
        self.metadata_dirty = false;
        self.semantic_state = None;
        self.baked_legacy = !recipe.is_empty();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.bump_revision();
        self.invalidate_preview_cache();
        Ok(report)
    }

    pub(crate) fn has_unresolved_persisted_targets(&self) -> bool {
        !self.persisted_recipe_issues.is_empty()
    }

    pub(crate) fn has_incomplete_workbench(&self) -> bool {
        !self.unavailable_targets.is_empty()
    }

    pub(crate) fn repair_persisted_target(
        &mut self,
        target_id: &DyeTargetId,
    ) -> Result<bool, String> {
        let issues = self
            .persisted_recipe_issues
            .iter()
            .filter(|issue| &issue.opaque_target_id == target_id)
            .cloned()
            .collect::<Vec<_>>();
        if issues.is_empty() {
            return Err("染色目标没有待修复配方".to_string());
        }
        if issues
            .iter()
            .any(|issue| issue.status != PersistedRecipeIssueStatus::Changed)
        {
            return Err("该染色目标无法自动修复，只能明确重置整套旧配方".to_string());
        }
        let mut next_baked_projection = self.capture_edits();
        let mut next_materialization_baseline = self.materialization_baseline.clone();
        let mut repairs_baked_projection = false;
        let mut repairs_materialization_baseline = false;
        for issue in &issues {
            let matched_target_id = issue
                .matched_target_id
                .as_ref()
                .ok_or_else(|| "该染色目标无法自动修复，只能明确重置整套旧配方".to_string())?;
            let target = self
                .targets
                .iter()
                .find(|target| &target.id == matched_target_id)
                .ok_or_else(|| "该染色目标已不存在，无法自动修复".to_string())?;
            let active_palette = target.active_palette;
            let palette = target
                .active_palette_state()
                .map_err(|_| "当前目标 Palette 不可用，无法自动修复".to_string())?;
            let current_colors = palette.colors.clone();
            let mut rebased_edits = BTreeMap::new();
            for recipe in &issue.recipes {
                if recipe.palette_index != active_palette {
                    return Err("当前目标 Palette 已变化，无法自动修复".to_string());
                }
                for edit in &recipe.colors {
                    let replacement_rgb = palette
                        .direct_format
                        .map_or(edit.replacement_rgb, |format| {
                            format.quantize_rgb(edit.replacement_rgb)
                        });
                    let current = current_colors
                        .get(usize::from(edit.color_index))
                        .copied()
                        .ok_or_else(|| "旧配方颜色索引已不存在，无法自动修复".to_string())?;
                    if current[3] == 0 {
                        return Err("旧配方颜色现在完全透明，无法自动修复".to_string());
                    }
                    if replacement_rgb == current[..3] {
                        continue;
                    }
                    if let Some(existing) = rebased_edits.insert(edit.color_index, replacement_rgb)
                    {
                        if existing != replacement_rgb {
                            return Err("旧配方颜色存在冲突，无法自动修复".to_string());
                        }
                    }
                }
            }
            let next = match issue.scope {
                PersistedRecipeIssueScope::BakedProjection => {
                    repairs_baked_projection = true;
                    &mut next_baked_projection
                }
                PersistedRecipeIssueScope::MaterializationBaseline => {
                    repairs_materialization_baseline = true;
                    &mut next_materialization_baseline
                }
            };
            *next
                .get_mut(matched_target_id)
                .ok_or_else(|| "该染色目标已不存在，无法自动修复".to_string())? = rebased_edits;
        }

        if repairs_baked_projection {
            canonicalize_physical_alias_snapshot(&self.targets, &mut next_baked_projection)?;
        }
        if repairs_materialization_baseline {
            canonicalize_physical_alias_snapshot(
                &self.targets,
                &mut next_materialization_baseline,
            )?;
        }
        if repairs_baked_projection {
            self.restore_edits(&next_baked_projection);
        }
        if repairs_materialization_baseline {
            self.materialization_baseline = next_materialization_baseline;
        } else if repairs_baked_projection && self.baked_legacy && self.semantic_state.is_none() {
            self.materialization_baseline = next_baked_projection;
        }
        self.persisted_recipe_issues
            .retain(|issue| &issue.opaque_target_id != target_id);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.metadata_dirty = true;
        self.bump_revision();
        self.invalidate_preview_cache();
        Ok(true)
    }

    pub fn preview_frame(&self, frame_index: usize) -> Result<DyePreviewFrame, String> {
        render_session_frame(self, frame_index)
    }

    pub(crate) fn preview_frame_count(&self) -> usize {
        session_composite_frame_count(self)
    }

    pub fn preview_animation(&self) -> Result<DyePreviewAnimation, String> {
        render_session_animation(self)
    }

    pub(super) fn preview_img_bytes(
        &self,
        target: &TargetState,
        original: bool,
    ) -> Result<PreviewImgBytes, String> {
        if original || !target.has_edits() {
            return Ok(PreviewImgBytes {
                bytes: Arc::clone(&target.source_bytes),
                recipe_hash: [0; 32],
                bytes_hash: target.source_bytes_hash,
            });
        }

        let recipe_hash = target.preview_recipe_hash()?;
        let key = PatchedImgCacheKey {
            entry_identity: target.entry_identity,
            active_palette: target.active_palette,
            recipe_hash,
        };
        if let Some(cached) = self
            .preview_cache
            .lock()
            .map_err(|_| "染色预览缓存不可用".to_string())?
            .get_patched(&key)
        {
            return Ok(cached);
        }

        let bytes: Arc<[u8]> = Arc::from(target.patched_bytes()?);
        let cached = PreviewImgBytes {
            bytes_hash: sha256(&bytes),
            bytes,
            recipe_hash,
        };
        self.preview_cache
            .lock()
            .map_err(|_| "染色预览缓存不可用".to_string())?
            .insert_patched(key, cached.clone());
        Ok(cached)
    }

    pub(super) fn preview_decoded_frame<F>(
        &self,
        target: &TargetState,
        img: &PreviewImgBytes,
        frame_index: usize,
        decode: F,
    ) -> Result<Arc<DecodedFrame>, String>
    where
        F: FnOnce() -> Result<DecodedFrame, String>,
    {
        let key = DecodedFrameCacheKey {
            entry_identity: target.entry_identity,
            frame_index,
            active_palette: target.active_palette,
            recipe_hash: img.recipe_hash,
            patched_bytes_hash: img.bytes_hash,
        };
        if let Some(cached) = self
            .preview_cache
            .lock()
            .map_err(|_| "染色预览缓存不可用".to_string())?
            .get_decoded(&key)
        {
            return Ok(cached);
        }

        let decoded = Arc::new(decode()?);
        self.preview_cache
            .lock()
            .map_err(|_| "染色预览缓存不可用".to_string())?
            .insert_decoded(key, Arc::clone(&decoded));
        Ok(decoded)
    }

    fn invalidate_preview_cache(&self) {
        if let Ok(mut cache) = self.preview_cache.lock() {
            cache.clear();
        }
    }

    pub(crate) fn patched_img_bytes(&self, target_id: &DyeTargetId) -> Result<Vec<u8>, String> {
        let target = self
            .targets
            .iter()
            .find(|target| &target.id == target_id)
            .ok_or_else(|| "染色目标句柄无效或已过期".to_string())?;
        Ok(self.preview_img_bytes(target, false)?.bytes.to_vec())
    }

    fn apply_mutation<F>(&mut self, operation: F) -> Result<bool, String>
    where
        F: FnOnce(&mut [TargetState]) -> Result<(), String>,
    {
        if !self.current_conflicting_target_ids().is_empty() {
            return Err(
                "同一 IMG 与 Palette 的当前编辑存在冲突；请先撤销或恢复全部染色".to_string(),
            );
        }
        let before = self.capture_history_snapshot();
        if let Err(error) = operation(&mut self.targets) {
            self.restore_history_snapshot(&before);
            return Err(error);
        }
        if let Err(error) = synchronize_physical_alias_edits(&mut self.targets, &before.edits) {
            self.restore_history_snapshot(&before);
            return Err(error);
        }
        let after = self.capture_edits();
        if before.edits == after {
            return Ok(false);
        }
        // Legacy single-target commands cannot update the layered semantic
        // model, so keeping it would make the sliders disagree with RGB.
        self.semantic_state = None;
        self.metadata_dirty = true;
        push_history(&mut self.undo_stack, before);
        self.redo_stack.clear();
        self.bump_revision();
        self.invalidate_preview_cache();
        Ok(true)
    }

    fn capture_edits(&self) -> EditSnapshot {
        self.targets
            .iter()
            .map(|target| (target.id.clone(), target.edits.clone()))
            .collect()
    }

    fn capture_history_snapshot(&self) -> SessionHistorySnapshot {
        SessionHistorySnapshot {
            edits: self.capture_edits(),
            persisted_recipe_issues: self.persisted_recipe_issues.clone(),
            metadata_dirty: self.metadata_dirty,
            semantic_state: self.semantic_state.clone(),
            materialization_baseline: self.materialization_baseline.clone(),
            baked_legacy: self.baked_legacy,
        }
    }

    fn restore_edits(&mut self, snapshot: &EditSnapshot) {
        for target in &mut self.targets {
            target.edits = snapshot.get(&target.id).cloned().unwrap_or_default();
        }
    }

    fn restore_history_snapshot(&mut self, snapshot: &SessionHistorySnapshot) {
        self.restore_edits(&snapshot.edits);
        self.persisted_recipe_issues = snapshot.persisted_recipe_issues.clone();
        self.metadata_dirty = snapshot.metadata_dirty;
        self.semantic_state = snapshot.semantic_state.clone();
        self.materialization_baseline = snapshot.materialization_baseline.clone();
        self.baked_legacy = snapshot.baked_legacy;
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    fn recipe_revision(&self) -> String {
        if self.has_unresolved_persisted_targets()
            || !self.current_conflicting_target_ids().is_empty()
        {
            return "blocked".to_string();
        }
        if self.has_incomplete_workbench() {
            return "incomplete".to_string();
        }
        self.recipe()
            .map(|recipe| hex_prefix(&recipe.recipe_hash, 16))
            .unwrap_or_else(|_| "invalid".to_string())
    }

    fn resource_status(&self, current_conflicts: &BTreeSet<DyeTargetId>) -> DyeResourceStatus {
        if !current_conflicts.is_empty() {
            return DyeResourceStatus::Conflict;
        }
        if self
            .persisted_recipe_issues
            .iter()
            .any(|issue| issue.status == PersistedRecipeIssueStatus::Conflict)
        {
            return DyeResourceStatus::Conflict;
        }
        if self
            .persisted_recipe_issues
            .iter()
            .any(|issue| issue.status == PersistedRecipeIssueStatus::Missing)
        {
            return DyeResourceStatus::Missing;
        }
        if self
            .persisted_recipe_issues
            .iter()
            .any(|issue| issue.status == PersistedRecipeIssueStatus::Changed)
        {
            return DyeResourceStatus::Changed;
        }
        if self.has_incomplete_workbench() {
            return DyeResourceStatus::Incomplete;
        }
        let editable = self
            .targets
            .iter()
            .filter(|target| target.capability != DyeCapabilityStatus::ReadOnly)
            .count();
        let partial = self.targets.iter().any(|target| {
            target.capability == DyeCapabilityStatus::ReadOnly
                || target.capability == DyeCapabilityStatus::PartiallyDyeable
        });
        if editable == 0 {
            DyeResourceStatus::ReadOnly
        } else if partial {
            DyeResourceStatus::PartialReadOnly
        } else {
            DyeResourceStatus::Ready
        }
    }

    fn target_resource_status(
        &self,
        target_id: &DyeTargetId,
        current_conflicts: &BTreeSet<DyeTargetId>,
    ) -> &'static str {
        if current_conflicts.contains(target_id) {
            return "conflict";
        }
        self.persisted_recipe_issues
            .iter()
            .filter(|issue| issue.matched_target_id.as_ref() == Some(target_id))
            .max_by_key(|issue| issue.status.severity())
            .map(|issue| issue.status.as_str())
            .unwrap_or("valid")
    }

    fn current_conflicting_target_ids(&self) -> BTreeSet<DyeTargetId> {
        type PhysicalColorKey = (String, u16, u16);
        type PhysicalColorValue = ([u8; 4], [u8; 3]);
        let mut physical =
            BTreeMap::<PhysicalColorKey, Vec<(DyeTargetId, PhysicalColorValue)>>::new();
        for target in &self.targets {
            let Ok(palette) = target.active_palette_state() else {
                continue;
            };
            let entry = normalized_private_entry(&target.target_ref.private_entry_ref);
            for (color_index, replacement_rgb) in &target.edits {
                let Some(expected_rgba) = palette.colors.get(usize::from(*color_index)).copied()
                else {
                    continue;
                };
                physical
                    .entry((entry.clone(), target.active_palette, *color_index))
                    .or_default()
                    .push((target.id.clone(), (expected_rgba, *replacement_rgb)));
            }
        }

        let mut conflicts = BTreeSet::new();
        for edits in physical.into_values() {
            let Some((_, first)) = edits.first() else {
                continue;
            };
            if edits.iter().any(|(_, value)| value != first) {
                conflicts.extend(edits.into_iter().map(|(target_id, _)| target_id));
            }
        }
        conflicts
    }
}

#[derive(Debug, Clone)]
struct RestoreCandidate {
    recipe: DyeTargetRecipeV1,
    conflicted: bool,
}

fn normalize_restore_candidates(recipe: &DyeRecipeV1) -> Result<Vec<RestoreCandidate>, String> {
    const MAX_TARGETS: usize = 4_096;
    const MAX_COLOR_EDITS: usize = 65_536;

    if recipe.schema_version != 1 {
        return Err("染色配方无效: 不支持的染色配方版本".to_string());
    }
    if recipe.targets.len() > MAX_TARGETS {
        return Err("染色配方无效: 染色目标数量超过安全上限".to_string());
    }
    let total_color_edits = recipe.targets.iter().try_fold(0_usize, |total, target| {
        total
            .checked_add(target.colors.len())
            .ok_or_else(|| "染色配方无效: 染色颜色数量溢出".to_string())
    })?;
    if total_color_edits > MAX_COLOR_EDITS {
        return Err("染色配方无效: 染色颜色数量超过安全上限".to_string());
    }

    let mut candidates = Vec::<RestoreCandidate>::new();
    for raw_target in &recipe.targets {
        if raw_target.colors.is_empty() {
            canonicalize_recipe(DyeRecipeV1 {
                schema_version: recipe.schema_version,
                targets: vec![raw_target.clone()],
                recipe_hash: [0; 32],
            })
            .map_err(|error| format!("染色配方无效: {error}"))?;
            continue;
        }
        let mut normalized_target = None::<DyeTargetRecipeV1>;
        let mut colors = BTreeMap::<u16, DyeColorEditV1>::new();
        let mut conflicting_colors = Vec::new();
        let mut conflicted = false;
        for raw_edit in &raw_target.colors {
            let single = canonicalize_recipe(DyeRecipeV1 {
                schema_version: recipe.schema_version,
                targets: vec![DyeTargetRecipeV1 {
                    target: raw_target.target.clone(),
                    palette_index: raw_target.palette_index,
                    source: raw_target.source.clone(),
                    colors: vec![raw_edit.clone()],
                }],
                recipe_hash: [0; 32],
            })
            .map_err(|error| format!("染色配方无效: {error}"))?;
            let Some(mut normalized) = single.targets.into_iter().next() else {
                continue;
            };
            let edit = normalized
                .colors
                .pop()
                .ok_or_else(|| "染色配方无效: 颜色项缺失".to_string())?;
            normalized.colors.clear();
            if normalized_target.is_none() {
                normalized_target = Some(normalized);
            }
            if let Some(existing) = colors.get(&edit.color_index) {
                if existing != &edit {
                    conflicted = true;
                    conflicting_colors.push(edit);
                }
            } else {
                colors.insert(edit.color_index, edit);
            }
        }
        let Some(mut normalized_target) = normalized_target else {
            continue;
        };
        normalized_target.colors = colors.into_values().collect();
        normalized_target.colors.extend(conflicting_colors);

        if let Some(existing) = candidates.iter_mut().find(|candidate| {
            target_refs_match(&candidate.recipe.target, &normalized_target.target)
                && candidate.recipe.palette_index == normalized_target.palette_index
                && candidate.recipe.source == normalized_target.source
        }) {
            for edit in normalized_target.colors {
                if let Some(previous) = existing
                    .recipe
                    .colors
                    .iter()
                    .find(|previous| previous.color_index == edit.color_index)
                {
                    if previous != &edit {
                        existing.conflicted = true;
                        existing.recipe.colors.push(edit);
                    }
                } else {
                    existing.recipe.colors.push(edit);
                }
            }
            existing.conflicted |= conflicted;
        } else {
            candidates.push(RestoreCandidate {
                recipe: normalized_target,
                conflicted,
            });
        }
    }

    for left in 0..candidates.len() {
        for right in (left + 1)..candidates.len() {
            if target_refs_match(
                &candidates[left].recipe.target,
                &candidates[right].recipe.target,
            ) && (candidates[left].recipe.palette_index
                != candidates[right].recipe.palette_index
                || candidates[left].recipe.source != candidates[right].recipe.source)
            {
                candidates[left].conflicted = true;
                candidates[right].conflicted = true;
            }
        }
    }

    type PhysicalColorKey = (String, u16, u16);
    type PhysicalColorValue = ([u8; 4], [u8; 3]);
    let mut physical = BTreeMap::<PhysicalColorKey, Vec<(PhysicalColorValue, usize)>>::new();
    for candidate_index in 0..candidates.len() {
        let entry =
            normalized_private_entry(&candidates[candidate_index].recipe.target.private_entry_ref);
        let palette_index = candidates[candidate_index].recipe.palette_index;
        let colors = candidates[candidate_index].recipe.colors.clone();
        for edit in colors {
            let key = (entry.clone(), palette_index, edit.color_index);
            let value = (edit.expected_rgba, edit.replacement_rgb);
            let prior = physical.entry(key).or_default();
            let conflicting_indices = prior
                .iter()
                .filter(|(prior_value, _)| *prior_value != value)
                .map(|(_, index)| *index)
                .collect::<Vec<_>>();
            if !conflicting_indices.is_empty() {
                candidates[candidate_index].conflicted = true;
                for index in conflicting_indices {
                    candidates[index].conflicted = true;
                }
            }
            prior.push((value, candidate_index));
        }
    }
    Ok(candidates)
}

impl PersistedRecipeIssue {
    fn placeholder_view(&self) -> DyeTargetView {
        let source = self.recipes.first().map(|recipe| &recipe.source);
        DyeTargetView {
            opaque_target_id: self.opaque_target_id.clone(),
            canonical_target_id: self.opaque_target_id.clone(),
            reference_count: 1,
            is_shared: false,
            slot: public_persisted_slot(&self.target_ref.slot).to_string(),
            layer_label: match &self.target_ref.target_kind {
                DyeTargetKindV1::Base => "主 IMG".to_string(),
                DyeTargetKindV1::Mask { number } => format!("Mask {number}"),
            },
            layer_order: i32::MAX,
            kind: self.target_ref.target_kind.clone(),
            resource_status: self.status.as_str().to_string(),
            preview_available: false,
            active_palette: self
                .recipes
                .first()
                .map(|recipe| recipe.palette_index)
                .unwrap_or_default(),
            img_version: source.map(|source| source.img_version).unwrap_or_default(),
            frame_count: source.map(|source| source.frame_count).unwrap_or_default(),
            capability: DyeCapabilityStatus::ReadOnly,
            palettes: Vec::new(),
        }
    }
}

impl UnavailableTargetState {
    fn placeholder_view(&self) -> DyeTargetView {
        DyeTargetView {
            opaque_target_id: self.id.clone(),
            canonical_target_id: self.id.clone(),
            reference_count: 1,
            is_shared: false,
            slot: self.identity.slot.clone(),
            layer_label: self.identity.layer_tag.clone(),
            layer_order: self.layer_order,
            kind: self.kind.clone(),
            resource_status: self.resource_status.as_str().to_string(),
            preview_available: false,
            active_palette: 0,
            img_version: self.img_version,
            frame_count: self.frame_count,
            capability: DyeCapabilityStatus::ReadOnly,
            palettes: Vec::new(),
        }
    }
}

fn public_persisted_slot(slot: &str) -> &'static str {
    match slot.trim().to_ascii_lowercase().as_str() {
        "skin" => "skin",
        "cap" => "cap",
        "hair" => "hair",
        "face" => "face",
        "neck" => "neck",
        "coat" => "coat",
        "belt" => "belt",
        "pants" => "pants",
        "shoes" => "shoes",
        _ => "unknown",
    }
}

impl TargetState {
    pub(super) fn id(&self) -> &DyeTargetId {
        &self.id
    }

    fn view(
        &self,
        resource_status: &str,
        canonical_target_id: DyeTargetId,
        reference_count: usize,
        materialization_baseline: Option<&BTreeMap<u16, [u8; 3]>>,
    ) -> DyeTargetView {
        DyeTargetView {
            opaque_target_id: self.id.clone(),
            canonical_target_id,
            reference_count,
            is_shared: reference_count > 1,
            slot: self.target_ref.slot.clone(),
            layer_label: self.target_ref.layer_tag.clone(),
            layer_order: self.layer_order,
            kind: self.target_ref.target_kind.clone(),
            resource_status: resource_status.to_string(),
            preview_available: true,
            active_palette: self.active_palette,
            img_version: self.img_version,
            frame_count: self.frame_count,
            capability: self.capability,
            palettes: self
                .palettes
                .iter()
                .enumerate()
                .map(|(palette_index, palette)| {
                    let is_active = palette_index == usize::from(self.active_palette);
                    DyePaletteView {
                        palette_index: palette_index as u16,
                        rgb_encoding: palette
                            .direct_format
                            .map(|format| format.rgb_encoding().to_string()),
                        colors: palette
                            .colors
                            .iter()
                            .enumerate()
                            .map(|(color_index, original)| {
                                let mut current = *original;
                                let mut baseline = *original;
                                if is_active {
                                    if let Some(rgb) = materialization_baseline
                                        .and_then(|edits| edits.get(&(color_index as u16)))
                                    {
                                        baseline[..3].copy_from_slice(rgb);
                                    }
                                    if let Some(rgb) = self.edits.get(&(color_index as u16)) {
                                        current[..3].copy_from_slice(rgb);
                                    }
                                }
                                DyePaletteColorView {
                                    color_index: color_index as u16,
                                    original_rgba: *original,
                                    baseline_rgba: baseline,
                                    current_rgba: current,
                                    used_pixel_count: palette
                                        .inspection
                                        .used_color_pixel_counts
                                        .get(color_index)
                                        .copied()
                                        .unwrap_or(0),
                                    used_frame_count: palette
                                        .inspection
                                        .used_color_frame_counts
                                        .get(color_index)
                                        .copied()
                                        .unwrap_or(0),
                                    used_frame_bitmap_base64: palette
                                        .inspection
                                        .used_color_frame_bitmaps
                                        .get(color_index)
                                        .map(|bitmap| {
                                            base64::engine::general_purpose::STANDARD.encode(bitmap)
                                        })
                                        .unwrap_or_default(),
                                }
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }

    fn active_palette_state(&self) -> Result<&PaletteState, String> {
        self.palettes
            .get(usize::from(self.active_palette))
            .ok_or_else(|| "染色目标没有可编辑的 Palette".to_string())
    }

    fn ensure_editable(&self) -> Result<(), String> {
        if self.capability == DyeCapabilityStatus::ReadOnly {
            return Err("当前染色目标为只读".to_string());
        }
        self.active_palette_state().map(|_| ())
    }

    fn current_rgb(&self, color_index: u16) -> Result<[u8; 3], String> {
        self.ensure_editable()?;
        if let Some(rgb) = self.edits.get(&color_index) {
            return Ok(*rgb);
        }
        let original = self
            .active_palette_state()?
            .colors
            .get(usize::from(color_index))
            .ok_or_else(|| "染色颜色索引无效".to_string())?;
        Ok([original[0], original[1], original[2]])
    }

    fn set_color(&mut self, color_index: u16, replacement_rgb: [u8; 3]) -> Result<(), String> {
        self.ensure_editable()?;
        let palette = self.active_palette_state()?;
        let original = palette
            .colors
            .get(usize::from(color_index))
            .copied()
            .ok_or_else(|| "染色颜色索引无效".to_string())?;
        let replacement_rgb = palette.direct_format.map_or(replacement_rgb, |format| {
            format.quantize_rgb(replacement_rgb)
        });
        if original[3] == 0 && replacement_rgb != original[..3] {
            return Err("完全透明颜色不可修改".to_string());
        }
        if replacement_rgb == original[..3] {
            self.edits.remove(&color_index);
        } else {
            self.edits.insert(color_index, replacement_rgb);
        }
        Ok(())
    }

    fn used_opaque_color_indices(&self) -> Result<Vec<u16>, String> {
        self.ensure_editable()?;
        let palette = self.active_palette_state()?;
        palette
            .colors
            .iter()
            .zip(&palette.inspection.used_color_pixel_counts)
            .enumerate()
            .filter(|(_, (rgba, used_pixel_count))| rgba[3] > 0 && **used_pixel_count > 0)
            .map(|(color_index, _)| {
                u16::try_from(color_index).map_err(|_| "Palette 颜色索引超过安全上限".to_string())
            })
            .collect()
    }

    fn used_opaque_colors(&self) -> Result<Vec<(u16, [u8; 3], u64)>, String> {
        self.used_opaque_color_indices()?
            .into_iter()
            .map(|color_index| {
                let used_pixel_count = self
                    .active_palette_state()?
                    .inspection
                    .used_color_pixel_counts
                    .get(usize::from(color_index))
                    .copied()
                    .unwrap_or(0);
                Ok((
                    color_index,
                    self.current_rgb(color_index)?,
                    used_pixel_count,
                ))
            })
            .collect()
    }

    pub(super) fn frame_count(&self) -> usize {
        self.frame_count as usize
    }

    pub(super) fn active_palette(&self) -> usize {
        usize::from(self.active_palette)
    }

    pub(super) fn is_mask(&self) -> bool {
        matches!(self.target_ref.target_kind, DyeTargetKindV1::Mask { .. })
    }

    pub(super) fn slot(&self) -> &str {
        &self.target_ref.slot
    }

    pub(super) fn has_edits(&self) -> bool {
        !self.edits.is_empty()
    }

    pub(super) fn private_entry_ref(&self) -> &str {
        &self.target_ref.private_entry_ref
    }

    pub(super) fn target_kind(&self) -> &DyeTargetKindV1 {
        &self.target_ref.target_kind
    }

    pub(super) fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    pub(super) fn patched_bytes(&self) -> Result<Vec<u8>, String> {
        if self.edits.is_empty() {
            return Ok(self.source_bytes.to_vec());
        }
        let recipe = self.preview_target_recipe()?;
        build_patched_img_bytes(&self.source_bytes, &recipe)
    }

    fn preview_recipe_hash(&self) -> Result<[u8; 32], String> {
        let recipe = canonicalize_recipe(DyeRecipeV1 {
            schema_version: 1,
            targets: vec![self.preview_target_recipe()?],
            recipe_hash: [0; 32],
        })?;
        Ok(recipe.recipe_hash)
    }

    fn preview_target_recipe(&self) -> Result<DyeTargetRecipeV1, String> {
        let palette = self.active_palette_state()?;
        let colors = self
            .edits
            .iter()
            .map(|(color_index, replacement_rgb)| {
                let expected_rgba = palette
                    .colors
                    .get(usize::from(*color_index))
                    .copied()
                    .ok_or_else(|| "染色颜色索引无效".to_string())?;
                Ok(DyeColorEditV1 {
                    color_index: *color_index,
                    expected_rgba,
                    replacement_rgb: *replacement_rgb,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(DyeTargetRecipeV1 {
            target: self.target_ref.clone(),
            palette_index: self.active_palette,
            source: palette.fingerprint.clone(),
            colors,
        })
    }
}

fn build_target_state(
    identity: &DyeSessionIdentity,
    session_nonce: u128,
    family_index: usize,
    kind: DyeTargetKindV1,
    entry_ref: String,
    source_bytes: &[u8],
    base_active_palette: u16,
    parsed_img_cache: &mut ParsedImgCache,
) -> Result<TargetState, String> {
    let source_bytes_hash = sha256(source_bytes);
    let resource = if let Some(cached) = parsed_img_cache.get(&source_bytes_hash) {
        cached.clone()?
    } else {
        let parsed = parse_img_resource(source_bytes, source_bytes_hash).map(Arc::new);
        parsed_img_cache.insert(source_bytes_hash, parsed.clone());
        parsed?
    };
    if resource.source_bytes.as_ref() != source_bytes {
        return Err("IMG 内容指纹冲突，已拒绝复用解析缓存".to_string());
    }
    let archive_version = i32::from(resource.img_version);
    let active_palette = match (&kind, archive_version) {
        (DyeTargetKindV1::Mask { .. }, _) | (DyeTargetKindV1::Base, 2 | 4) => 0,
        (DyeTargetKindV1::Base, _) => base_active_palette,
    };
    let capability = if matches!(archive_version, 2 | 4 | 6) && !resource.palettes.is_empty() {
        resource
            .palettes
            .get(usize::from(active_palette))
            .ok_or_else(|| "实际生效 Palette 索引无效".to_string())?
            .inspection
            .capability
    } else {
        resource.capability
    };
    let entry_identity = preview_entry_identity(&entry_ref, source_bytes_hash);
    Ok(TargetState {
        id: make_target_id(session_nonce, family_index, &kind),
        target_ref: PersistedDyeTargetRefV1 {
            slot: identity.slot.clone(),
            item_ref: identity.item_ref.clone(),
            layer_tag: identity.layer_tag.clone(),
            target_kind: kind,
            private_entry_ref: entry_ref,
        },
        source_bytes: Arc::clone(&resource.source_bytes),
        source_bytes_hash: resource.source_bytes_hash,
        entry_identity,
        layer_order: 0,
        draw_ordinal: 0,
        active_palette,
        img_version: resource.img_version,
        frame_count: resource.frame_count,
        capability,
        palettes: Arc::clone(&resource.palettes),
        edits: BTreeMap::new(),
    })
}

fn parse_img_resource(
    source_bytes: &[u8],
    source_bytes_hash: [u8; 32],
) -> Result<ParsedImgResource, String> {
    let catalog = inspect_img_catalog(source_bytes)?;
    let frame_count = catalog.frame_count;
    if frame_count == 0 {
        return Err("染色资源 IMG 没有可预览帧".to_string());
    }
    let img_version = catalog.img_version;
    let archive_version = i32::from(img_version);

    let (palettes, capability) = if matches!(archive_version, 4 | 6) {
        let layout = catalog
            .palette_layout
            .as_ref()
            .ok_or_else(|| "IMG Palette 结构不可用".to_string())?;
        let mut palettes = Vec::with_capacity(layout.boards.len());
        for (palette_index, (board, inspection)) in layout
            .boards
            .iter()
            .zip(catalog.inspections.into_iter())
            .enumerate()
        {
            let colors = (0..board.color_count)
                .map(|color_index| layout.color_rgba(source_bytes, palette_index, color_index))
                .collect::<Result<Vec<_>, _>>()?;
            palettes.push(PaletteState {
                fingerprint: DyeSourceFingerprintV1::from_validated_layout(
                    source_bytes,
                    layout,
                    palette_index,
                    source_bytes_hash,
                )?,
                colors,
                inspection,
                direct_format: None,
            });
        }
        let capability = palettes
            .first()
            .map(|palette| palette.inspection.capability)
            .unwrap_or(DyeCapabilityStatus::ReadOnly);
        (palettes, capability)
    } else if archive_version == 2 {
        let inspection = catalog
            .inspections
            .into_iter()
            .next()
            .ok_or_else(|| "IMG 染色检查结果不可用".to_string())?;
        let capability = inspection.capability;
        let palettes = if let Some(palette) = catalog.direct_palette {
            vec![PaletteState {
                fingerprint: DyeSourceFingerprintV1 {
                    img_version,
                    frame_count,
                    palette_count: 1,
                    color_count: u16::try_from(palette.colors.len())
                        .map_err(|_| "V2 虚拟 Palette 颜色数量过多".to_string())?,
                    palette_sha256: virtual_palette_sha256(&palette.colors),
                    entry_sha256: source_bytes_hash,
                },
                colors: palette.colors,
                inspection,
                direct_format: Some(palette.format),
            }]
        } else {
            Vec::new()
        };
        (palettes, capability)
    } else {
        let inspection = catalog
            .inspections
            .into_iter()
            .next()
            .ok_or_else(|| "IMG 染色检查结果不可用".to_string())?;
        (Vec::new(), inspection.capability)
    };

    Ok(ParsedImgResource {
        source_bytes: Arc::from(source_bytes.to_vec()),
        source_bytes_hash,
        img_version,
        frame_count,
        capability,
        palettes: Arc::new(palettes),
    })
}

fn find_target_mut<'a>(
    targets: &'a mut [TargetState],
    target_id: &DyeTargetId,
) -> Result<&'a mut TargetState, String> {
    targets
        .iter_mut()
        .find(|target| &target.id == target_id)
        .ok_or_else(|| "染色目标句柄无效或已过期".to_string())
}

fn synchronize_physical_alias_edits(
    targets: &mut [TargetState],
    before: &EditSnapshot,
) -> Result<(), String> {
    for indices in physical_alias_groups(targets)
        .into_values()
        .filter(|indices| indices.len() > 1)
    {
        let mut transitions = BTreeMap::<u16, Option<[u8; 3]>>::new();
        for index in &indices {
            let target = &targets[*index];
            let prior = before.get(&target.id).cloned().unwrap_or_default();
            let color_indices = prior
                .keys()
                .chain(target.edits.keys())
                .copied()
                .collect::<BTreeSet<_>>();
            for color_index in color_indices {
                let previous = prior.get(&color_index).copied();
                let current = target.edits.get(&color_index).copied();
                if previous == current {
                    continue;
                }
                if let Some(existing) = transitions.insert(color_index, current) {
                    if existing != current {
                        return Err("同一 IMG 与 Palette 的单次编辑存在冲突".to_string());
                    }
                }
            }
        }
        if transitions.is_empty() {
            continue;
        }

        ensure_physical_alias_sources_match(targets, &indices)?;
        let mut complete_edits = merge_physical_alias_edit_maps(targets, &indices, before)?;
        for (color_index, replacement_rgb) in transitions {
            if let Some(replacement_rgb) = replacement_rgb {
                complete_edits.insert(color_index, replacement_rgb);
            } else {
                complete_edits.remove(&color_index);
            }
        }
        for index in indices {
            targets[index].edits = complete_edits.clone();
        }
    }
    Ok(())
}

fn canonicalize_physical_alias_snapshot(
    targets: &[TargetState],
    snapshot: &mut EditSnapshot,
) -> Result<(), String> {
    for indices in physical_alias_groups(targets)
        .into_values()
        .filter(|indices| indices.len() > 1)
    {
        let has_edits = indices.iter().any(|index| {
            snapshot
                .get(&targets[*index].id)
                .is_some_and(|edits| !edits.is_empty())
        });
        if has_edits {
            ensure_physical_alias_sources_match(targets, &indices)?;
        }
        let complete_edits = merge_physical_alias_edit_maps(targets, &indices, snapshot)?;
        for index in indices {
            snapshot.insert(targets[index].id.clone(), complete_edits.clone());
        }
    }
    Ok(())
}

fn merge_persisted_recipe_issues(
    existing: &mut Vec<PersistedRecipeIssue>,
    incoming: Vec<PersistedRecipeIssue>,
) {
    for issue in incoming {
        if existing.iter().any(|candidate| {
            candidate.scope == issue.scope && persisted_issues_target_same(candidate, &issue)
        }) {
            continue;
        }
        existing.push(issue);
    }
}

fn unique_persisted_issue_count(
    issues: &[PersistedRecipeIssue],
    status: PersistedRecipeIssueStatus,
) -> usize {
    let mut unique = Vec::<&PersistedRecipeIssue>::new();
    for issue in issues.iter().filter(|issue| issue.status == status) {
        if unique
            .iter()
            .any(|candidate| persisted_issues_target_same(candidate, issue))
        {
            continue;
        }
        unique.push(issue);
    }
    unique.len()
}

fn persisted_issues_target_same(left: &PersistedRecipeIssue, right: &PersistedRecipeIssue) -> bool {
    match (&left.matched_target_id, &right.matched_target_id) {
        (Some(left), Some(right)) => left == right,
        _ => target_refs_match(&left.target_ref, &right.target_ref),
    }
}

fn validate_public_semantic_state(
    session: &AvatarDyeSession,
    semantic_state: Option<&serde_json::Value>,
) -> Result<Option<serde_json::Value>, String> {
    let Some(semantic_state) = semantic_state else {
        return Ok(None);
    };
    let encoded =
        serde_json::to_vec(semantic_state).map_err(|_| "染色交互状态无法序列化".to_string())?;
    if encoded.len() > MAX_SEMANTIC_STATE_JSON_BYTES {
        return Err("染色交互状态超过安全上限".to_string());
    }
    let mut parsed: PublicDyeSemanticStateV1 =
        serde_json::from_slice(&encoded).map_err(|_| "染色交互状态格式无效".to_string())?;
    if parsed.version != 1 {
        return Err("不支持的染色交互状态版本".to_string());
    }
    parsed.baked_legacy = session.baked_legacy;

    for part in &mut parsed.parts {
        if let Some(id) = part.canonical_target_id.as_deref() {
            let (_, canonical) = resolve_public_semantic_target(session, id)?;
            part.canonical_target_id = Some(canonical.as_str().to_string());
            part.slot = None;
        } else if let Some(slot) = part.slot.as_mut() {
            *slot = slot.trim().to_ascii_lowercase();
            if slot.is_empty()
                || !session
                    .targets
                    .iter()
                    .any(|target| target.target_ref.slot.trim().eq_ignore_ascii_case(slot))
            {
                return Err("染色部位语义目标无效".to_string());
            }
        } else {
            return Err("染色部位语义缺少目标".to_string());
        }
    }
    for image in &mut parsed.images {
        let id = image
            .canonical_target_id
            .as_deref()
            .or(image.target_id.as_deref())
            .ok_or_else(|| "染色 IMG 语义缺少目标".to_string())?;
        let (target, canonical) = resolve_public_semantic_target(session, id)?;
        if image.palette_index != target.active_palette {
            return Err("染色 IMG 语义 Palette 无效".to_string());
        }
        image.canonical_target_id = Some(canonical.as_str().to_string());
        image.target_id = None;
    }
    for exact in &mut parsed.exact_overrides {
        let (target, canonical) =
            resolve_public_semantic_target(session, &exact.canonical_target_id)?;
        let palette = target.active_palette_state()?;
        let original = palette
            .colors
            .get(usize::from(exact.color_index))
            .ok_or_else(|| "染色色块语义索引无效".to_string())?;
        if original[3] == 0 {
            return Err("完全透明颜色不可修改".to_string());
        }
        if let Some(format) = palette.direct_format {
            exact.rgb = format.quantize_rgb(exact.rgb);
        }
        exact.canonical_target_id = canonical.as_str().to_string();
    }

    serde_json::to_value(parsed)
        .map(Some)
        .map_err(|_| "染色交互状态无法规范化".to_string())
}

fn resolve_public_semantic_target<'a>(
    session: &'a AvatarDyeSession,
    id: &str,
) -> Result<(&'a TargetState, DyeTargetId), String> {
    let id = DyeTargetId::parse_opaque(id)?;
    let target = session
        .targets
        .iter()
        .find(|target| target.id == id)
        .ok_or_else(|| "染色交互状态目标无效或已过期".to_string())?;
    let key = (
        normalized_private_entry(&target.target_ref.private_entry_ref),
        target.active_palette,
    );
    let canonical = physical_alias_groups(&session.targets)
        .get(&key)
        .and_then(|indices| indices.first())
        .map(|index| session.targets[*index].id.clone())
        .ok_or_else(|| "染色交互状态物理目标无效".to_string())?;
    Ok((target, canonical))
}

fn project_public_semantic_for_persistence(
    session: &AvatarDyeSession,
    semantic_state: &serde_json::Value,
) -> Result<String, String> {
    let normalized = validate_public_semantic_state(session, Some(semantic_state))?
        .ok_or_else(|| "染色交互状态缺失".to_string())?;
    let public: PublicDyeSemanticStateV1 =
        serde_json::from_value(normalized).map_err(|_| "染色交互状态格式无效".to_string())?;
    let mut parts = Vec::with_capacity(public.parts.len());
    for part in public.parts {
        let target = part
            .canonical_target_id
            .as_deref()
            .map(|id| resolve_public_semantic_target(session, id).map(|(target, _)| target))
            .transpose()?;
        let (target, palette_index) = target
            .map(|target| (Some(target.target_ref.clone()), target.active_palette))
            .unwrap_or((None, 0));
        parts.push(PersistedSemanticPartV1 {
            target,
            palette_index,
            slot: part.slot,
            adjustment: part.adjustment,
        });
    }
    let mut images = Vec::with_capacity(public.images.len());
    for image in public.images {
        let id = image
            .canonical_target_id
            .as_deref()
            .ok_or_else(|| "染色 IMG 语义缺少规范目标".to_string())?;
        let (target, _) = resolve_public_semantic_target(session, id)?;
        images.push(PersistedSemanticImageV1 {
            target: target.target_ref.clone(),
            palette_index: image.palette_index,
            adjustment: image.adjustment,
        });
    }
    let mut exact_overrides = Vec::with_capacity(public.exact_overrides.len());
    for exact in public.exact_overrides {
        let (target, _) = resolve_public_semantic_target(session, &exact.canonical_target_id)?;
        exact_overrides.push(PersistedSemanticExactOverrideV1 {
            target: target.target_ref.clone(),
            palette_index: target.active_palette,
            color_index: exact.color_index,
            rgb: exact.rgb,
        });
    }
    let envelope = PersistedSemanticEnvelopeV1 {
        version: 1,
        state: PersistedSemanticStateV1 {
            version: 1,
            all: public.all,
            parts,
            images,
            exact_overrides,
        },
    };
    let json =
        serde_json::to_string(&envelope).map_err(|_| "无法持久化染色交互状态".to_string())?;
    if json.len() > MAX_SEMANTIC_STATE_JSON_BYTES {
        return Err("持久化染色交互状态超过安全上限".to_string());
    }
    Ok(json)
}

fn restore_public_semantic_from_persistence(
    session: &AvatarDyeSession,
    json: &str,
    baked_legacy: bool,
) -> Result<serde_json::Value, String> {
    if json.len() > MAX_SEMANTIC_STATE_JSON_BYTES {
        return Err("持久化染色交互状态超过安全上限".to_string());
    }
    let envelope: PersistedSemanticEnvelopeV1 =
        serde_json::from_str(json).map_err(|_| "持久化染色交互状态格式无效".to_string())?;
    if envelope.version != 1 || envelope.state.version != 1 {
        return Err("不支持的持久化染色交互状态版本".to_string());
    }

    let mut parts = Vec::new();
    for part in envelope.state.parts {
        if let Some(target_ref) = part.target {
            let Some(target) = session.targets.iter().find(|target| {
                target_matches_persisted_physical(target, &target_ref, part.palette_index)
            }) else {
                continue;
            };
            let (_, canonical) = resolve_public_semantic_target(session, target.id.as_str())?;
            parts.push(PublicSemanticPartV1 {
                canonical_target_id: Some(canonical.as_str().to_string()),
                slot: None,
                adjustment: part.adjustment,
            });
        } else if part.slot.is_some() {
            parts.push(PublicSemanticPartV1 {
                canonical_target_id: None,
                slot: part.slot,
                adjustment: part.adjustment,
            });
        }
    }
    let mut images = Vec::new();
    for image in envelope.state.images {
        let Some(target) = session.targets.iter().find(|target| {
            target_matches_persisted_physical(target, &image.target, image.palette_index)
        }) else {
            continue;
        };
        let (_, canonical) = resolve_public_semantic_target(session, target.id.as_str())?;
        images.push(PublicSemanticImageV1 {
            canonical_target_id: Some(canonical.as_str().to_string()),
            target_id: None,
            palette_index: image.palette_index,
            adjustment: image.adjustment,
        });
    }
    let mut exact_overrides = Vec::new();
    for exact in envelope.state.exact_overrides {
        let Some(target) = session.targets.iter().find(|target| {
            target_matches_persisted_physical(target, &exact.target, exact.palette_index)
        }) else {
            continue;
        };
        let (_, canonical) = resolve_public_semantic_target(session, target.id.as_str())?;
        let palette = target.active_palette_state()?;
        let original = palette
            .colors
            .get(usize::from(exact.color_index))
            .ok_or_else(|| "持久化染色色块语义索引无效".to_string())?;
        if original[3] == 0 {
            return Err("完全透明颜色不可修改".to_string());
        }
        let rgb = palette
            .direct_format
            .map_or(exact.rgb, |format| format.quantize_rgb(exact.rgb));
        exact_overrides.push(PublicSemanticExactOverrideV1 {
            canonical_target_id: canonical.as_str().to_string(),
            color_index: exact.color_index,
            rgb,
        });
    }
    serde_json::to_value(PublicDyeSemanticStateV1 {
        version: 1,
        baked_legacy,
        all: envelope.state.all,
        parts,
        images,
        exact_overrides,
    })
    .map_err(|_| "无法恢复染色交互状态".to_string())
}

fn physical_alias_groups(targets: &[TargetState]) -> BTreeMap<PhysicalPaletteKey, Vec<usize>> {
    let mut groups = BTreeMap::<PhysicalPaletteKey, Vec<usize>>::new();
    for (index, target) in targets.iter().enumerate() {
        groups
            .entry((
                normalized_private_entry(&target.target_ref.private_entry_ref),
                target.active_palette,
            ))
            .or_default()
            .push(index);
    }
    groups
}

fn merge_physical_alias_edit_maps(
    targets: &[TargetState],
    indices: &[usize],
    snapshot: &EditSnapshot,
) -> Result<BTreeMap<u16, [u8; 3]>, String> {
    let mut complete_edits = BTreeMap::<u16, [u8; 3]>::new();
    for index in indices {
        if let Some(edits) = snapshot.get(&targets[*index].id) {
            for (color_index, replacement_rgb) in edits {
                if let Some(existing) = complete_edits.insert(*color_index, *replacement_rgb) {
                    if existing != *replacement_rgb {
                        return Err("同一 IMG 与 Palette 的当前编辑存在冲突".to_string());
                    }
                }
            }
        }
    }
    Ok(complete_edits)
}

fn ensure_physical_alias_sources_match(
    targets: &[TargetState],
    indices: &[usize],
) -> Result<(), String> {
    let first = &targets[indices[0]];
    let first_palette = first.active_palette_state()?;
    for index in indices.iter().skip(1) {
        let target = &targets[*index];
        let palette = target.active_palette_state()?;
        if target.source_bytes_hash != first.source_bytes_hash
            || (!Arc::ptr_eq(&target.source_bytes, &first.source_bytes)
                && target.source_bytes.as_ref() != first.source_bytes.as_ref())
            || palette.fingerprint != first_palette.fingerprint
            || palette.colors != first_palette.colors
        {
            return Err("同一 IMG 与 Palette 的源资源不一致，无法同步染色".to_string());
        }
    }
    Ok(())
}

fn unique_color_indices(
    color_indices: &[u16],
    operation_label: &str,
) -> Result<BTreeSet<u16>, String> {
    if color_indices.is_empty() {
        return Err(format!("{operation_label}没有选择颜色项"));
    }
    if color_indices.len() > MAX_COLOR_SELECTION_INPUTS {
        return Err(format!("{operation_label}颜色项数量超过安全上限"));
    }
    Ok(color_indices.iter().copied().collect())
}

fn target_refs_same_layer(left: &PersistedDyeTargetRefV1, right: &PersistedDyeTargetRefV1) -> bool {
    left.slot.trim().eq_ignore_ascii_case(right.slot.trim())
        && left.item_ref.trim() == right.item_ref.trim()
        && left.layer_tag.trim() == right.layer_tag.trim()
}

fn target_refs_match(left: &PersistedDyeTargetRefV1, right: &PersistedDyeTargetRefV1) -> bool {
    target_refs_same_layer(left, right)
        && left.target_kind == right.target_kind
        && normalized_private_entry(&left.private_entry_ref)
            == normalized_private_entry(&right.private_entry_ref)
}

fn target_matches_persisted_physical(
    target: &TargetState,
    persisted: &PersistedDyeTargetRefV1,
    palette_index: u16,
) -> bool {
    target.active_palette == palette_index
        && normalized_private_entry(&target.target_ref.private_entry_ref)
            == normalized_private_entry(&persisted.private_entry_ref)
}

fn normalized_private_entry(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn preview_entry_identity(entry_ref: &str, source_bytes_hash: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(normalized_private_entry(entry_ref).as_bytes());
    hasher.update([0]);
    hasher.update(source_bytes_hash);
    hasher.finalize().into()
}

fn push_history(stack: &mut Vec<SessionHistorySnapshot>, snapshot: SessionHistorySnapshot) {
    if stack.len() == MAX_HISTORY_ENTRIES {
        stack.remove(0);
    }
    stack.push(snapshot);
}

fn validate_transform(transform: DyeRelativeTransform) -> Result<(), String> {
    if !(-360..=360).contains(&transform.hue_degrees)
        || !(-100..=100).contains(&transform.saturation_percent)
        || !(-100..=100).contains(&transform.lightness_percent)
        || !(-100..=100).contains(&transform.contrast_percent)
    {
        return Err("相对染色参数超出安全范围".to_string());
    }
    Ok(())
}

fn apply_hsl_transform(rgb: [u8; 3], transform: DyeRelativeTransform) -> [u8; 3] {
    let (mut hue, mut saturation, mut lightness) = rgb_to_hsl(rgb);
    hue = (hue + f64::from(transform.hue_degrees)).rem_euclid(360.0);
    saturation =
        (saturation * (1.0 + f64::from(transform.saturation_percent) / 100.0)).clamp(0.0, 1.0);
    lightness = (lightness + f64::from(transform.lightness_percent) / 100.0).clamp(0.0, 1.0);
    let hsl_rgb = if transform.hue_degrees == 0
        && transform.saturation_percent == 0
        && transform.lightness_percent == 0
    {
        rgb
    } else {
        hsl_to_rgb(hue, saturation, lightness)
    };
    let contrast_factor = 1.0 + f64::from(transform.contrast_percent) / 100.0;
    hsl_rgb.map(|component| {
        (((f64::from(component) / 255.0 - 0.5) * contrast_factor + 0.5) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

fn matching_family_zone_indices(
    primary_colors: &[[u8; 3]],
    candidates: &[(u16, [u8; 3], u64)],
) -> Vec<u16> {
    if primary_colors.is_empty() || candidates.is_empty() {
        return Vec::new();
    }
    let best = candidates.iter().min_by(|left, right| {
        let left_distance = primary_colors
            .iter()
            .map(|primary| perceptual_hsl_distance(*primary, left.1))
            .fold(f64::INFINITY, f64::min);
        let right_distance = primary_colors
            .iter()
            .map(|primary| perceptual_hsl_distance(*primary, right.1))
            .fold(f64::INFINITY, f64::min);
        left_distance
            .total_cmp(&right_distance)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    let Some((_, best_rgb, _)) = best else {
        return Vec::new();
    };
    candidates
        .iter()
        .filter(|(_, rgb, _)| colors_share_zone(*best_rgb, *rgb))
        .map(|(color_index, _, _)| *color_index)
        .collect()
}

fn perceptual_hsl_distance(left: [u8; 3], right: [u8; 3]) -> f64 {
    let (left_hue, left_saturation, left_lightness) = rgb_to_hsl(left);
    let (right_hue, right_saturation, right_lightness) = rgb_to_hsl(right);
    let hue_distance = if left_saturation < 0.08 && right_saturation < 0.08 {
        0.0
    } else if left_saturation < 0.08 || right_saturation < 0.08 {
        1.0
    } else {
        hue_distance_degrees(left_hue, right_hue) / 180.0
    };
    0.55 * hue_distance
        + 0.20 * (left_saturation - right_saturation).abs()
        + 0.25 * (left_lightness - right_lightness).abs()
}

fn colors_share_zone(left: [u8; 3], right: [u8; 3]) -> bool {
    let (left_hue, left_saturation, left_lightness) = rgb_to_hsl(left);
    let (right_hue, right_saturation, right_lightness) = rgb_to_hsl(right);
    if left_saturation < 0.08 && right_saturation < 0.08 {
        return (left_lightness - right_lightness).abs() <= 0.32;
    }
    left_saturation >= 0.08
        && right_saturation >= 0.08
        && hue_distance_degrees(left_hue, right_hue) <= 38.0
        && (left_saturation - right_saturation).abs() <= 0.50
        && (left_lightness - right_lightness).abs() <= 0.40
}

fn hue_distance_degrees(left: f64, right: f64) -> f64 {
    let direct = (left - right).abs().rem_euclid(360.0);
    direct.min(360.0 - direct)
}

fn rgb_to_hsl(rgb: [u8; 3]) -> (f64, f64, f64) {
    let red = f64::from(rgb[0]) / 255.0;
    let green = f64::from(rgb[1]) / 255.0;
    let blue = f64::from(rgb[2]) / 255.0;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;
    let lightness = (max + min) / 2.0;
    if delta <= f64::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue_sector = if (max - red).abs() <= f64::EPSILON {
        ((green - blue) / delta).rem_euclid(6.0)
    } else if (max - green).abs() <= f64::EPSILON {
        (blue - red) / delta + 2.0
    } else {
        (red - green) / delta + 4.0
    };
    (hue_sector * 60.0, saturation, lightness)
}

fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> [u8; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let secondary = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match sector.floor() as i32 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = lightness - chroma / 2.0;
    [
        ((red + offset) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((green + offset) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((blue + offset) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn normalize_identity(identity: DyeSessionIdentity) -> Result<DyeSessionIdentity, String> {
    let normalized = DyeSessionIdentity {
        slot: identity.slot.trim().to_ascii_lowercase(),
        item_ref: identity.item_ref.trim().to_string(),
        layer_tag: identity.layer_tag.trim().to_string(),
    };
    if normalized.slot.is_empty()
        || normalized.item_ref.is_empty()
        || normalized.layer_tag.is_empty()
    {
        return Err("染色会话身份不完整".to_string());
    }
    Ok(normalized)
}

fn unique_session_nonce() -> u128 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = u128::from(SESSION_COUNTER.fetch_add(1, Ordering::Relaxed));
    nanos ^ (u128::from(std::process::id()) << 64) ^ counter
}

fn parse_opaque_id(value: &str, label: &str) -> Result<String, String> {
    if value.len() != OPAQUE_ID_HEX_LENGTH
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label}格式无效"));
    }
    Ok(value.to_string())
}

fn make_target_id(nonce: u128, index: usize, kind: &DyeTargetKindV1) -> DyeTargetId {
    let mut hasher = Sha256::new();
    hasher.update(b"dnf-avatar-dye-target-v1");
    hasher.update(nonce.to_le_bytes());
    hasher.update(index.to_le_bytes());
    match kind {
        DyeTargetKindV1::Base => hasher.update([0]),
        DyeTargetKindV1::Mask { number } => {
            hasher.update([1]);
            hasher.update(number.to_le_bytes());
        }
    }
    let digest = hasher.finalize();
    DyeTargetId(
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn make_persisted_issue_id(
    session_id: &DyeSessionId,
    index: usize,
    kind: &DyeTargetKindV1,
) -> DyeTargetId {
    let mut hasher = Sha256::new();
    hasher.update(b"dnf-avatar-dye-persisted-issue-v1");
    hasher.update(session_id.as_str().as_bytes());
    hasher.update(index.to_le_bytes());
    match kind {
        DyeTargetKindV1::Base => hasher.update([0]),
        DyeTargetKindV1::Mask { number } => {
            hasher.update([1]);
            hasher.update(number.to_le_bytes());
        }
    }
    DyeTargetId(hex_prefix(&hasher.finalize(), 16))
}

fn make_session_id(nonce: u128) -> DyeSessionId {
    let mut hasher = Sha256::new();
    hasher.update(b"dnf-avatar-dye-session-v1");
    hasher.update(nonce.to_le_bytes());
    DyeSessionId(hex_prefix(&hasher.finalize(), 16))
}

fn hex_prefix(bytes: &[u8], length: usize) -> String {
    bytes
        .iter()
        .take(length)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
