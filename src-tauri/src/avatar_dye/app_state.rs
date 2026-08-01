use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine;
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder};
use serde::{Deserialize, Serialize};

use crate::dnflib_safe::NpkArchive;
use crate::game_resolver::{
    img_path_lookup_candidates, resolve_game_npk_path, resolve_img_resource,
    same_named_mask_number, ImgResourceMetadata,
};
use crate::models::{
    enable_dye_mask, sort_avatar_layers_for_draw, AvatarLayer, PreviewImage, WardrobeSnapshotInput,
    WardrobeSnapshotView,
};
use crate::preview_frame::preview_frame_index_for_profession;

use super::{
    AvatarDyeSession, DyeBatchMutation, DyeNpkExportSummary, DyeRecipeV1, DyeRelativeSelection,
    DyeRelativeTransform, DyeSessionIdentity, DyeTargetId, DyeTargetKindV1,
    DyeUnavailableTargetInput, DyeWorkbenchLayerInput, PersistedDyeStateV2,
    PositionedDyeUnavailableTargetInput, PositionedDyeWorkbenchLayerInput,
};

const MAX_PROCESS_SESSIONS: usize = 16;
const MAX_EXPORT_NAME_ATTEMPTS: usize = 10_000;
const MAX_WORKBENCH_TARGETS: usize = 2_048;
const MAX_WORKBENCH_SOURCE_BYTES: usize = 256 * 1024 * 1024;
const MAX_OPEN_NPK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_PREVIEW_HIDDEN_TARGETS: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyeRelativeScope {
    Current,
    Zone,
    Family,
    Part,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DyeColorOperation {
    SetRgb {
        #[serde(rename = "colorIndices")]
        color_indices: Vec<u16>,
        rgb: [u8; 3],
    },
    RelativeHsl {
        #[serde(rename = "colorIndices")]
        color_indices: Vec<u16>,
        #[serde(rename = "hueDelta")]
        hue_delta: i16,
        #[serde(rename = "saturationDelta")]
        saturation_delta: i16,
        #[serde(rename = "lightnessDelta")]
        lightness_delta: i16,
        #[serde(default, rename = "contrastDelta")]
        contrast_delta: i16,
        scope: DyeRelativeScope,
    },
    CopyReferencePalette {
        #[serde(rename = "referencePaletteIndex")]
        reference_palette_index: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyeCompareMode {
    Dyed,
    Original,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyePreviewScope {
    Outfit,
    Part,
    Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DyeFrameResponse {
    pub width: u32,
    pub height: u32,
    pub rgba_base64: String,
    pub missing_imgs: Vec<String>,
    pub request_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DyeComparisonFrameResponse {
    pub original: PreviewImage,
    pub dyed: PreviewImage,
    pub request_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DyeComparisonImagePair {
    pub original: PreviewImage,
    pub dyed: PreviewImage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DyeAnimationResponse {
    pub frame_count: usize,
    pub missing_imgs: Vec<String>,
    pub frames: Vec<PreviewImage>,
    pub request_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DyeComparisonAnimationResponse {
    pub frame_count: usize,
    pub missing_imgs: Vec<String>,
    pub frames: Vec<DyeComparisonImagePair>,
    pub request_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DyeWorkspaceSummary {
    pub label: String,
    pub occupied_slot_count: usize,
    pub has_dye: bool,
    pub dye_status: String,
    pub dyed_slot_count: usize,
    pub dyed_base_count: usize,
    pub dyed_mask_count: usize,
    pub recipe_revision: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedDyePersistence {
    pub snapshot: WardrobeSnapshotInput,
    #[allow(dead_code)] // Baked projection kept for legacy in-crate callers.
    pub recipe: Option<DyeRecipeV1>,
    pub dye_state: Option<PersistedDyeStateV2>,
    pub revision: u64,
}

#[derive(Debug)]
pub(crate) struct AvatarDyeSessionRecord {
    session: AvatarDyeSession,
    snapshot: WardrobeSnapshotInput,
    game_dir: PathBuf,
    source_npk_paths: Vec<PathBuf>,
    resource_changed: bool,
}

#[derive(Default)]
pub struct AvatarDyeAppState {
    sessions: Mutex<BTreeMap<String, Arc<Mutex<AvatarDyeSessionRecord>>>>,
}

impl AvatarDyeAppState {
    pub(crate) fn open_from_snapshot(
        &self,
        game_dir: &Path,
        snapshot: WardrobeSnapshotInput,
        recipe: Option<DyeRecipeV1>,
    ) -> Result<String, String> {
        self.open_from_snapshot_with_dye_state(
            game_dir,
            snapshot,
            recipe.map(PersistedDyeStateV2::from_legacy_recipe),
        )
    }

    pub(crate) fn open_from_snapshot_with_dye_state(
        &self,
        game_dir: &Path,
        snapshot: WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
    ) -> Result<String, String> {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&snapshot)?;
        let game_dir = normalize_game_dir(game_dir)?;
        let layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&snapshot)?;
        let (inputs, unavailable, source_npk_paths) = load_workbench_inputs(&game_dir, layers)?;
        let mut session = AvatarDyeSession::open_positioned_workbench(inputs, unavailable)?;
        if let Some(dye_state) = dye_state.as_ref() {
            session.restore_persisted_dye_state_best_effort(dye_state)?;
        }
        let session_id = session.view().session_id.as_str().to_string();
        let record = Arc::new(Mutex::new(AvatarDyeSessionRecord {
            session,
            snapshot,
            game_dir,
            source_npk_paths,
            resource_changed: false,
        }));
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "染色会话状态不可用".to_string())?;
        if sessions.len() >= MAX_PROCESS_SESSIONS {
            return Err("打开的染色会话过多，请先关闭不用的工作台".to_string());
        }
        if sessions.insert(session_id.clone(), record).is_some() {
            return Err("染色会话标识冲突".to_string());
        }
        Ok(session_id)
    }

    pub(crate) fn record(
        &self,
        session_id: &str,
    ) -> Result<Arc<Mutex<AvatarDyeSessionRecord>>, String> {
        validate_opaque_id(session_id)?;
        self.sessions
            .lock()
            .map_err(|_| "染色会话状态不可用".to_string())?
            .get(session_id)
            .cloned()
            .ok_or_else(|| "染色会话不存在或已经关闭".to_string())
    }

    pub(crate) fn view(&self, session_id: &str) -> Result<serde_json::Value, String> {
        let record = self.record(session_id)?;
        let record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        public_session_view(&record)
    }

    pub(crate) fn update_colors(
        &self,
        session_id: &str,
        target_id: &str,
        operation: DyeColorOperation,
    ) -> Result<serde_json::Value, String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        ensure_live_source_editable(&record)?;
        if record.session.has_unresolved_persisted_targets() {
            return Err("染色资源存在待修复目标，请先修复目标或恢复全部染色".to_string());
        }
        let target_id = resolve_target_id(&record.session, target_id)?;
        apply_color_operation(&mut record.session, &target_id, operation)?;
        public_session_view(&record)
    }

    pub(crate) fn update_batch(
        &self,
        session_id: &str,
        mutation: &DyeBatchMutation,
    ) -> Result<serde_json::Value, String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        ensure_live_source_editable(&record)?;
        if record.session.has_unresolved_persisted_targets() {
            return Err("染色资源存在待修复目标，请先修复目标或恢复全部染色".to_string());
        }
        record.session.replace_color_tables_atomic(mutation)?;
        public_session_view(&record)
    }

    pub(crate) fn render_transient_colors_frame_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        frame_index: usize,
        compare_mode: DyeCompareMode,
        preview_scope: DyePreviewScope,
        target_id: &str,
        operation: DyeColorOperation,
        highlight: bool,
        hide_selected_mask: bool,
        hidden_target_ids: &[String],
        request_id: u64,
    ) -> Result<DyeFrameResponse, String> {
        let (session, target_id, hidden_target_ids) = {
            let record = record
                .lock()
                .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
            ensure_live_source_editable(&record)?;
            if record.session.has_unresolved_persisted_targets() {
                return Err("染色资源存在待修复目标，请先修复目标或恢复全部染色".to_string());
            }
            let target_id = resolve_target_id(&record.session, target_id)?;
            let hidden_target_ids = resolve_hidden_target_ids(&record.session, hidden_target_ids)?;
            let mut session = record.session.clone_for_transient_preview();
            apply_color_operation(&mut session, &target_id, operation)?;
            session.adopt_preview_cache_from(&record.session);
            (session, target_id, hidden_target_ids)
        };
        render_frame_response(
            &session,
            frame_index,
            compare_mode,
            preview_scope,
            &target_id,
            highlight,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_transient_comparison_frame_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        frame_index: usize,
        preview_scope: DyePreviewScope,
        target_id: &str,
        operation: DyeColorOperation,
        hide_selected_mask: bool,
        hidden_target_ids: &[String],
        request_id: u64,
    ) -> Result<DyeComparisonFrameResponse, String> {
        let (session, target_id, hidden_target_ids) = {
            let record = record
                .lock()
                .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
            ensure_live_source_editable(&record)?;
            if record.session.has_unresolved_persisted_targets() {
                return Err("染色资源存在待修复目标，请先修复目标或恢复全部染色".to_string());
            }
            let target_id = resolve_target_id(&record.session, target_id)?;
            let hidden_target_ids = resolve_hidden_target_ids(&record.session, hidden_target_ids)?;
            let mut session = record.session.clone_for_transient_preview();
            apply_color_operation(&mut session, &target_id, operation)?;
            session.adopt_preview_cache_from(&record.session);
            (session, target_id, hidden_target_ids)
        };
        render_comparison_frame_response(
            &session,
            frame_index,
            preview_scope,
            &target_id,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_transient_batch_comparison_frame_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        frame_index: usize,
        preview_scope: DyePreviewScope,
        target_id: &str,
        mutation: &DyeBatchMutation,
        hide_selected_mask: bool,
        hidden_target_ids: &[String],
        request_id: u64,
    ) -> Result<DyeComparisonFrameResponse, String> {
        let (session, target_id, hidden_target_ids) = {
            let record = record
                .lock()
                .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
            ensure_live_source_editable(&record)?;
            if record.session.has_unresolved_persisted_targets() {
                return Err("染色资源存在待修复目标，请先修复目标或恢复全部染色".to_string());
            }
            let target_id = resolve_target_id(&record.session, target_id)?;
            let hidden_target_ids = resolve_hidden_target_ids(&record.session, hidden_target_ids)?;
            let mut session = record.session.clone_for_transient_preview();
            session.replace_color_tables_atomic(mutation)?;
            session.adopt_preview_cache_from(&record.session);
            (session, target_id, hidden_target_ids)
        };
        render_comparison_frame_response(
            &session,
            frame_index,
            preview_scope,
            &target_id,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    }

    pub(crate) fn reset_scope(
        &self,
        session_id: &str,
        scope: &str,
        target_id: Option<&str>,
        color_indices: &[u16],
    ) -> Result<serde_json::Value, String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        let normalized_scope = scope.trim().to_ascii_lowercase();
        if normalized_scope != "all" {
            ensure_live_source_editable(&record)?;
        }
        if normalized_scope != "all" && record.session.has_unresolved_persisted_targets() {
            return Err("染色资源存在待修复目标，只能先修复目标或恢复全部染色".to_string());
        }
        match normalized_scope.as_str() {
            "all" => {
                record.session.reset_all_atomic()?;
            }
            "target" => {
                let target = require_target(&record.session, target_id)?;
                record.session.reset_target_atomic(&target)?;
            }
            "part" => {
                let target = require_target(&record.session, target_id)?;
                record.session.reset_part_atomic(&target)?;
            }
            "colors" => {
                let target = require_target(&record.session, target_id)?;
                record.session.reset_colors_atomic(&target, color_indices)?;
            }
            _ => return Err("未知的染色恢复范围".to_string()),
        }
        public_session_view(&record)
    }

    pub(crate) fn undo(&self, session_id: &str) -> Result<serde_json::Value, String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        ensure_live_source_editable(&record)?;
        if record.session.has_unresolved_persisted_targets() {
            return Err("染色资源存在待修复目标，当前不能撤销".to_string());
        }
        record.session.undo();
        public_session_view(&record)
    }

    pub(crate) fn redo(&self, session_id: &str) -> Result<serde_json::Value, String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        ensure_live_source_editable(&record)?;
        record.session.redo();
        public_session_view(&record)
    }

    pub(crate) fn repair_target(
        &self,
        session_id: &str,
        target_id: &str,
    ) -> Result<serde_json::Value, String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        ensure_live_source_editable(&record)?;
        let target_id = resolve_target_id(&record.session, target_id)?;
        record.session.repair_persisted_target(&target_id)?;
        public_session_view(&record)
    }

    pub(crate) fn render_frame_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        frame_index: usize,
        compare_mode: DyeCompareMode,
        preview_scope: DyePreviewScope,
        target_id: &str,
        highlight: bool,
        hide_selected_mask: bool,
        hidden_target_ids: &[String],
        request_id: u64,
    ) -> Result<DyeFrameResponse, String> {
        let frame = {
            let record = record
                .lock()
                .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
            let target_id = resolve_target_id(&record.session, target_id)?;
            let hidden_target_ids = resolve_hidden_target_ids(&record.session, hidden_target_ids)?;
            super::preview::render_session_frame_with_visibility(
                &record.session,
                frame_index,
                compare_mode,
                preview_scope,
                &target_id,
                highlight,
                hide_selected_mask,
                &hidden_target_ids,
            )
        }?;
        Ok(encode_frame_response(frame, request_id))
    }

    pub(crate) fn render_comparison_frame_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        frame_index: usize,
        preview_scope: DyePreviewScope,
        target_id: &str,
        hide_selected_mask: bool,
        hidden_target_ids: &[String],
        request_id: u64,
    ) -> Result<DyeComparisonFrameResponse, String> {
        let (session, target_id, hidden_target_ids) =
            comparison_preview_snapshot(record, target_id, hidden_target_ids)?;
        render_comparison_frame_response(
            &session,
            frame_index,
            preview_scope,
            &target_id,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    }

    pub(crate) fn render_animation_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        compare_mode: DyeCompareMode,
        preview_scope: DyePreviewScope,
        target_id: &str,
        highlight: bool,
        hide_selected_mask: bool,
        hidden_target_ids: &[String],
        request_id: u64,
    ) -> Result<DyeAnimationResponse, String> {
        let animation = {
            let record = record
                .lock()
                .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
            let target_id = resolve_target_id(&record.session, target_id)?;
            let hidden_target_ids = resolve_hidden_target_ids(&record.session, hidden_target_ids)?;
            super::preview::render_session_animation_with_visibility(
                &record.session,
                compare_mode,
                preview_scope,
                &target_id,
                highlight,
                hide_selected_mask,
                &hidden_target_ids,
            )
        }?;
        let frames = animation
            .frames
            .into_iter()
            .map(|frame| PreviewImage {
                width: frame.width,
                height: frame.height,
                rgba_base64: base64::engine::general_purpose::STANDARD.encode(frame.rgba),
                missing_imgs: Vec::new(),
            })
            .collect::<Vec<_>>();
        Ok(DyeAnimationResponse {
            frame_count: animation.frame_count,
            missing_imgs: Vec::new(),
            frames,
            request_id,
        })
    }

    pub(crate) fn render_comparison_animation_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        preview_scope: DyePreviewScope,
        target_id: &str,
        hide_selected_mask: bool,
        hidden_target_ids: &[String],
        request_id: u64,
    ) -> Result<DyeComparisonAnimationResponse, String> {
        let (session, target_id, hidden_target_ids) =
            comparison_preview_snapshot(record, target_id, hidden_target_ids)?;
        let animation = super::preview::render_session_comparison_animation_with_visibility(
            &session,
            preview_scope,
            &target_id,
            hide_selected_mask,
            &hidden_target_ids,
        )?;
        let frame_count = animation.frame_count;
        let frames = animation
            .frames
            .into_iter()
            .map(encode_comparison_image_pair)
            .collect::<Vec<_>>();
        if frames.len() != frame_count {
            return Err("染色对比动画帧对齐失败".to_string());
        }
        Ok(DyeComparisonAnimationResponse {
            frame_count,
            missing_imgs: Vec::new(),
            frames,
            request_id,
        })
    }

    pub(crate) fn prepare_persistence(
        &self,
        session_id: &str,
    ) -> Result<PreparedDyePersistence, String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        if record.resource_changed || record.session.has_unresolved_persisted_targets() {
            return Err("染色资源已经变化，必须先修复配方".to_string());
        }
        if record.session.has_incomplete_workbench() {
            return Err("染色工作台存在缺失资源，禁止保存不完整结果".to_string());
        }
        if let Err(error) = validate_live_sources(&record) {
            record.resource_changed = true;
            return Err(error);
        }
        let view = record.session.view();
        if !view.has_dye && !view.is_dirty {
            return Err("当前染色会话没有需要保存的变化".to_string());
        }
        let persisted = record.session.persisted_dye_state()?;
        let recipe = (!persisted.baked_recipe.is_empty()).then(|| persisted.baked_recipe.clone());
        let dye_state = (!persisted.baked_recipe.is_empty()
            || persisted.baseline_recipe.is_some()
            || persisted.semantic_state_json.is_some())
        .then_some(persisted);
        let mut snapshot = record.snapshot.clone();
        let preview_frame_index = preview_frame_index_for_profession(
            &record.snapshot.playable_role_key,
            record.session.preview_frame_count(),
        );
        let frame = record.session.preview_frame(preview_frame_index)?;
        snapshot.preview_data_url = encode_preview_data_url(&frame)?;
        Ok(PreparedDyePersistence {
            snapshot,
            recipe,
            dye_state,
            revision: view.revision,
        })
    }

    pub(crate) fn mark_persisted_if_unchanged(
        &self,
        session_id: &str,
        revision: u64,
    ) -> Result<(), String> {
        let record = self.record(session_id)?;
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        if record.session.view().revision == revision {
            record.session.mark_clean();
        }
        Ok(())
    }

    pub(crate) fn export_npk_from_record(
        record: Arc<Mutex<AvatarDyeSessionRecord>>,
        output_directory: Option<PathBuf>,
    ) -> Result<DyeNpkExportSummary, String> {
        let mut record = record
            .lock()
            .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
        if record.resource_changed || record.session.has_unresolved_persisted_targets() {
            return Err("染色资源已经变化，禁止导出补丁".to_string());
        }
        if record.session.has_incomplete_workbench() {
            return Err("染色工作台存在缺失资源，禁止导出不完整补丁".to_string());
        }
        if let Err(error) = validate_live_sources(&record) {
            record.resource_changed = true;
            return Err(error);
        }
        if !record.session.view().has_dye {
            return Err("当前会话没有可导出的染色".to_string());
        }
        let output_directory = match output_directory.as_deref() {
            Some(path) => canonical_output_directory(path)?,
            None => record.game_dir.clone(),
        };
        for index in 0..MAX_EXPORT_NAME_ATTEMPTS {
            let name = if index == 0 {
                "avatar_dye_patch.npk".to_string()
            } else {
                format!("avatar_dye_patch_{index:03}.npk")
            };
            let output_path = output_directory.join(name);
            if output_path.exists() {
                continue;
            }
            return record
                .session
                .export_dye_npk(&output_path, &record.source_npk_paths);
        }
        Err("输出目录中的染色补丁名称已经用尽".to_string())
    }

    pub(crate) fn close(&self, session_id: &str) -> Result<(), String> {
        validate_opaque_id(session_id)?;
        self.sessions
            .lock()
            .map_err(|_| "染色会话状态不可用".to_string())?
            .remove(session_id)
            .ok_or_else(|| "染色会话不存在或已经关闭".to_string())?;
        Ok(())
    }
}

fn apply_color_operation(
    session: &mut AvatarDyeSession,
    target_id: &DyeTargetId,
    operation: DyeColorOperation,
) -> Result<(), String> {
    match operation {
        DyeColorOperation::SetRgb { color_indices, rgb } => {
            session.set_rgb_many(target_id, &color_indices, rgb)?;
        }
        DyeColorOperation::RelativeHsl {
            color_indices,
            hue_delta,
            saturation_delta,
            lightness_delta,
            contrast_delta,
            scope,
        } => {
            let transform = DyeRelativeTransform {
                hue_degrees: hue_delta,
                saturation_percent: saturation_delta,
                lightness_percent: lightness_delta,
                contrast_percent: contrast_delta,
            };
            match scope {
                DyeRelativeScope::Current => {
                    session.apply_target_relative_hsl(target_id, transform)?;
                }
                DyeRelativeScope::Zone => {
                    session.apply_relative_sync(
                        &[DyeRelativeSelection {
                            opaque_target_id: target_id.clone(),
                            color_indices,
                        }],
                        transform,
                    )?;
                }
                DyeRelativeScope::Family => {
                    session.sync_resource_family_relative_hsl(
                        target_id,
                        &color_indices,
                        transform,
                    )?;
                }
                DyeRelativeScope::Part => {
                    session.sync_part_relative_hsl(target_id, &color_indices, transform)?;
                }
            }
        }
        DyeColorOperation::CopyReferencePalette {
            reference_palette_index,
        } => {
            session.copy_reference_palette(target_id, reference_palette_index)?;
        }
    }
    Ok(())
}

fn comparison_preview_snapshot(
    record: Arc<Mutex<AvatarDyeSessionRecord>>,
    target_id: &str,
    hidden_target_ids: &[String],
) -> Result<(AvatarDyeSession, DyeTargetId, Vec<DyeTargetId>), String> {
    let record = record
        .lock()
        .map_err(|_| "染色会话正在被其他操作使用".to_string())?;
    let target_id = resolve_target_id(&record.session, target_id)?;
    let hidden_target_ids = resolve_hidden_target_ids(&record.session, hidden_target_ids)?;
    let mut session = record.session.clone_for_transient_preview();
    session.adopt_preview_cache_from(&record.session);
    Ok((session, target_id, hidden_target_ids))
}

#[allow(clippy::too_many_arguments)]
fn render_comparison_frame_response(
    session: &AvatarDyeSession,
    frame_index: usize,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    hide_selected_mask: bool,
    hidden_target_ids: &[DyeTargetId],
    request_id: u64,
) -> Result<DyeComparisonFrameResponse, String> {
    let pair = super::preview::render_session_comparison_frame_with_visibility(
        session,
        frame_index,
        preview_scope,
        target_id,
        hide_selected_mask,
        hidden_target_ids,
    )?;
    let pair = encode_comparison_image_pair(pair);
    Ok(DyeComparisonFrameResponse {
        original: pair.original,
        dyed: pair.dyed,
        request_id,
    })
}

#[allow(clippy::too_many_arguments)]
fn render_frame_response(
    session: &AvatarDyeSession,
    frame_index: usize,
    compare_mode: DyeCompareMode,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    highlight: bool,
    hide_selected_mask: bool,
    hidden_target_ids: &[DyeTargetId],
    request_id: u64,
) -> Result<DyeFrameResponse, String> {
    let frame = super::preview::render_session_frame_with_visibility(
        session,
        frame_index,
        compare_mode,
        preview_scope,
        target_id,
        highlight,
        hide_selected_mask,
        hidden_target_ids,
    )?;
    Ok(encode_frame_response(frame, request_id))
}

fn encode_frame_response(
    frame: super::preview::DyePreviewFrame,
    request_id: u64,
) -> DyeFrameResponse {
    DyeFrameResponse {
        width: frame.width,
        height: frame.height,
        rgba_base64: base64::engine::general_purpose::STANDARD.encode(frame.rgba),
        missing_imgs: Vec::new(),
        request_id,
    }
}

fn encode_preview_image(frame: super::preview::DyePreviewFrame) -> PreviewImage {
    PreviewImage {
        width: frame.width,
        height: frame.height,
        rgba_base64: base64::engine::general_purpose::STANDARD.encode(frame.rgba),
        missing_imgs: Vec::new(),
    }
}

fn encode_comparison_image_pair(
    pair: super::preview::DyePreviewComparisonFrame,
) -> DyeComparisonImagePair {
    DyeComparisonImagePair {
        original: encode_preview_image(pair.original),
        dyed: encode_preview_image(pair.dyed),
    }
}

fn ensure_live_source_editable(record: &AvatarDyeSessionRecord) -> Result<(), String> {
    if record.resource_changed {
        return Err("源资源已变化，当前染色会话已冻结；请关闭后重新打开".to_string());
    }
    Ok(())
}

fn public_session_view(record: &AvatarDyeSessionRecord) -> Result<serde_json::Value, String> {
    let view = record.session.view();
    let mut value = serde_json::to_value(&view).map_err(|_| "无法生成染色会话视图".to_string())?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| "染色会话视图格式无效".to_string())?;
    root.insert(
        "playable_role_key".to_string(),
        serde_json::Value::String(record.snapshot.playable_role_key.clone()),
    );
    if record.resource_changed {
        root.insert(
            "resource_status".to_string(),
            serde_json::Value::String("changed".to_string()),
        );
        root.insert("editing_blocked".to_string(), serde_json::Value::Bool(true));
        root.insert(
            "persistence_blocked".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    if let Some(targets) = root
        .get_mut("targets")
        .and_then(serde_json::Value::as_array_mut)
    {
        for target in targets {
            let Some(target) = target.as_object_mut() else {
                continue;
            };
            let label = target
                .get("slot")
                .and_then(serde_json::Value::as_str)
                .map(slot_label)
                .unwrap_or("时装部位");
            target.insert(
                "slot_label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
    }
    Ok(value)
}

pub(crate) fn attach_workspace_summaries(
    mut view: serde_json::Value,
    current: Option<&WardrobeSnapshotView>,
    target: Option<&WardrobeSnapshotView>,
) -> Result<serde_json::Value, String> {
    let root = view
        .as_object_mut()
        .ok_or_else(|| "染色会话视图格式无效".to_string())?;
    root.insert(
        "current".to_string(),
        serde_json::to_value(current.map(|snapshot| workspace_summary("当前穿戴", snapshot)))
            .map_err(|_| "无法生成当前穿戴摘要".to_string())?,
    );
    root.insert(
        "target".to_string(),
        serde_json::to_value(target.map(|snapshot| workspace_summary("目标外观", snapshot)))
            .map_err(|_| "无法生成目标外观摘要".to_string())?,
    );
    Ok(view)
}

fn workspace_summary(label: &str, snapshot: &WardrobeSnapshotView) -> DyeWorkspaceSummary {
    DyeWorkspaceSummary {
        label: label.to_string(),
        occupied_slot_count: snapshot
            .items
            .iter()
            .filter(|item| item.item_name.is_some())
            .count(),
        has_dye: snapshot.has_dye,
        dye_status: snapshot.dye_status.clone(),
        dyed_slot_count: snapshot.dyed_slot_count,
        dyed_base_count: snapshot.dyed_base_count,
        dyed_mask_count: snapshot.dyed_mask_count,
        recipe_revision: snapshot.recipe_revision.clone(),
    }
}

fn load_workbench_inputs(
    game_dir: &Path,
    mut layers: Vec<AvatarLayer>,
) -> Result<
    (
        Vec<PositionedDyeWorkbenchLayerInput>,
        Vec<PositionedDyeUnavailableTargetInput>,
        Vec<PathBuf>,
    ),
    String,
> {
    layers.retain(|layer| !layer.visual_img.trim().is_empty());
    sort_avatar_layers_for_draw(&mut layers);
    if layers.is_empty() {
        return Err("搭配没有可读取的时装 IMG".to_string());
    }
    let mut archives = HashMap::<PathBuf, NpkArchive>::new();
    let mut unavailable_archives = BTreeSet::<PathBuf>::new();
    let mut source_paths = BTreeSet::<PathBuf>::new();
    let mut inputs = Vec::with_capacity(layers.len());
    let mut unavailable = Vec::new();
    let mut opened_npk_bytes = 0_u64;
    let mut retained_source_bytes = 0_usize;
    let mut target_count = 0_usize;
    for (draw_ordinal, layer) in layers.into_iter().enumerate() {
        let enable_dye_mask1 = layer.enable_dye_mask1;
        let enable_dye_mask2 = layer.enable_dye_mask2;
        let identity = DyeSessionIdentity {
            slot: layer.slot,
            item_ref: layer.item_id,
            layer_tag: layer.layer_tag,
        };
        let (base_entry, npk_path, palette_index) = match resolve_layer_entry(
            game_dir,
            &layer.visual_img,
            layer.palette_index,
            enable_dye_mask1,
            enable_dye_mask2,
            &mut archives,
            &mut unavailable_archives,
            &mut opened_npk_bytes,
        ) {
            Ok(resolved) => resolved,
            Err(error) if error == "时装 IMG 在游戏资源中不存在" => {
                target_count = target_count
                    .checked_add(1)
                    .ok_or_else(|| "染色工作台目标数量溢出".to_string())?;
                if target_count > MAX_WORKBENCH_TARGETS {
                    return Err("染色工作台目标数量超过安全上限".to_string());
                }
                unavailable.push(PositionedDyeUnavailableTargetInput {
                    draw_ordinal,
                    input: DyeUnavailableTargetInput {
                        identity,
                        layer_order: layer.layer_order,
                        kind: DyeTargetKindV1::Base,
                    },
                });
                continue;
            }
            Err(error) => return Err(error),
        };
        let active_palette = u16::try_from(palette_index)
            .map_err(|_| "时装 Palette 索引超出安全范围".to_string())?;
        let archive = archives
            .get(&npk_path)
            .ok_or_else(|| "时装资源缓存不可用".to_string())?;
        let mut family = vec![(0_u32, DyeTargetKindV1::Base, base_entry.clone())];
        family.extend(archive.entries().into_iter().filter_map(|entry| {
            same_named_mask_number(&base_entry, &entry.name)
                .filter(|number| enable_dye_mask(*number, enable_dye_mask1, enable_dye_mask2))
                .map(|number| {
                    (
                        number.saturating_add(1),
                        DyeTargetKindV1::Mask { number },
                        entry.name,
                    )
                })
        }));
        family.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.2.cmp(&right.2)));
        target_count = target_count
            .checked_add(family.len())
            .ok_or_else(|| "染色工作台目标数量溢出".to_string())?;
        if target_count > MAX_WORKBENCH_TARGETS {
            return Err("染色工作台目标数量超过安全上限".to_string());
        }
        let mut entries = Vec::with_capacity(family.len());
        for (_, kind, entry) in family {
            let Some(index) = archive.find_entry_index(&entry) else {
                if matches!(kind, DyeTargetKindV1::Base) {
                    entries.push((entry, Vec::new()));
                } else {
                    unavailable.push(PositionedDyeUnavailableTargetInput {
                        draw_ordinal,
                        input: DyeUnavailableTargetInput {
                            identity: identity.clone(),
                            layer_order: layer.layer_order,
                            kind,
                        },
                    });
                }
                continue;
            };
            let bytes = match archive.entry_bytes(index) {
                Ok(bytes) => bytes,
                Err(_) => {
                    if matches!(kind, DyeTargetKindV1::Base) {
                        entries.push((entry, Vec::new()));
                        continue;
                    }
                    unavailable.push(PositionedDyeUnavailableTargetInput {
                        draw_ordinal,
                        input: DyeUnavailableTargetInput {
                            identity: identity.clone(),
                            layer_order: layer.layer_order,
                            kind,
                        },
                    });
                    continue;
                }
            };
            retained_source_bytes = retained_source_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| "染色工作台 IMG 数据大小溢出".to_string())?;
            if retained_source_bytes > MAX_WORKBENCH_SOURCE_BYTES {
                return Err("染色工作台 IMG 数据超过安全上限".to_string());
            }
            entries.push((entry, bytes.to_vec()));
        }
        if !entries
            .iter()
            .any(|(entry, _)| entry.eq_ignore_ascii_case(&base_entry))
        {
            continue;
        }
        source_paths.insert(
            npk_path
                .canonicalize()
                .map_err(|_| "源时装 NPK 无法读取".to_string())?,
        );
        inputs.push(PositionedDyeWorkbenchLayerInput {
            draw_ordinal,
            input: DyeWorkbenchLayerInput {
                identity,
                layer_order: layer.layer_order,
                requested_base_entry: base_entry,
                active_palette,
                entries,
            },
        });
    }
    Ok((inputs, unavailable, source_paths.into_iter().collect()))
}

fn resolve_layer_entry(
    game_dir: &Path,
    visual_img: &str,
    requested_palette_index: usize,
    enable_dye_mask1: bool,
    enable_dye_mask2: bool,
    archives: &mut HashMap<PathBuf, NpkArchive>,
    unavailable_archives: &mut BTreeSet<PathBuf>,
    opened_npk_bytes: &mut u64,
) -> Result<(String, PathBuf, usize), String> {
    let exact_candidates = img_path_lookup_candidates(visual_img);
    let mut mask_only_candidate = None;
    let resolved = resolve_img_resource(
        visual_img,
        requested_palette_index,
        |entry| -> Result<_, String> {
            let Some(path) = resolve_game_npk_path(game_dir, &entry) else {
                return Ok(None);
            };
            if !path.is_file() {
                return Ok(None);
            }
            if unavailable_archives.contains(&path) {
                return Ok(None);
            }
            if !archives.contains_key(&path) {
                let size = match std::fs::metadata(&path) {
                    Ok(metadata) => metadata.len(),
                    Err(_) => {
                        unavailable_archives.insert(path.clone());
                        return Ok(None);
                    }
                };
                *opened_npk_bytes = opened_npk_bytes
                    .checked_add(size)
                    .ok_or_else(|| "源时装 NPK 总大小溢出".to_string())?;
                if *opened_npk_bytes > MAX_OPEN_NPK_BYTES {
                    return Err("本次染色会话需要读取的 NPK 超过安全上限".to_string());
                }
                match NpkArchive::open_path(&path) {
                    Ok(archive) => {
                        archives.insert(path.clone(), archive);
                    }
                    Err(_) => {
                        unavailable_archives.insert(path.clone());
                        return Ok(None);
                    }
                }
            }
            let Some(archive) = archives.get(&path) else {
                return Ok(None);
            };
            let base_index = archive.find_entry_index(entry);
            if base_index.is_none() {
                let mask_exists = archive.entries().into_iter().any(|candidate| {
                    same_named_mask_number(&entry, &candidate.name).is_some_and(|number| {
                        enable_dye_mask(number, enable_dye_mask1, enable_dye_mask2)
                    })
                });
                if mask_exists
                    && mask_only_candidate.is_none()
                    && exact_candidates
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(entry))
                {
                    mask_only_candidate = Some((entry.to_string(), path));
                }
                return Ok(None);
            }
            let metadata = base_index
                .and_then(|index| archive.read_img(index).ok())
                .map(|img| ImgResourceMetadata {
                    version: img.version(),
                    palette_count: img.palette_count(),
                })
                .unwrap_or(ImgResourceMetadata {
                    version: 0,
                    palette_count: 0,
                });
            Ok(Some((path, metadata)))
        },
    )?;
    resolved
        .map(|resolved| (resolved.entry_path, resolved.value, resolved.palette_index))
        .or_else(|| mask_only_candidate.map(|(entry, path)| (entry, path, requested_palette_index)))
        .ok_or_else(|| "时装 IMG 在游戏资源中不存在".to_string())
}

fn validate_live_sources(record: &AvatarDyeSessionRecord) -> Result<(), String> {
    let mut archives = Vec::with_capacity(record.source_npk_paths.len());
    for path in &record.source_npk_paths {
        if !path.is_file() {
            return Err("源时装 NPK 已经不存在".to_string());
        }
        archives.push(NpkArchive::open_path(path)?);
    }
    for target in &record.session.targets {
        let entry = target.private_entry_ref();
        let mut name_exists = false;
        let mut exact_source_exists = false;
        for archive in &archives {
            let Some(index) = archive.find_entry_index(entry) else {
                continue;
            };
            name_exists = true;
            if archive.entry_bytes(index)? == target.source_bytes() {
                exact_source_exists = true;
                break;
            }
        }
        if !name_exists || !exact_source_exists {
            return Err("源时装 IMG 已经变化".to_string());
        }
    }
    Ok(())
}

fn normalize_game_dir(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("游戏资源目录必须使用绝对路径".to_string());
    }
    let candidate = if path.join("ImagePacks2").is_dir() {
        path.join("ImagePacks2")
    } else {
        path.to_path_buf()
    };
    if !candidate.is_dir() {
        return Err("游戏 ImagePacks2 目录不存在".to_string());
    }
    candidate
        .canonicalize()
        .map_err(|_| "游戏 ImagePacks2 目录无法读取".to_string())
}

fn canonical_output_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || !path.is_dir() {
        return Err("染色补丁输出目录不存在或不是绝对路径".to_string());
    }
    path.canonicalize()
        .map_err(|_| "染色补丁输出目录无法读取".to_string())
}

fn resolve_target_id(session: &AvatarDyeSession, value: &str) -> Result<DyeTargetId, String> {
    validate_opaque_id(value)?;
    session
        .view()
        .targets
        .into_iter()
        .find(|target| target.opaque_target_id.as_str() == value)
        .map(|target| target.opaque_target_id)
        .ok_or_else(|| "染色目标不存在或已经过期".to_string())
}

fn resolve_hidden_target_ids(
    session: &AvatarDyeSession,
    values: &[String],
) -> Result<Vec<DyeTargetId>, String> {
    if values.len() > MAX_PREVIEW_HIDDEN_TARGETS {
        return Err("染色预览隐藏目标数量超过安全上限".to_string());
    }
    let mut hidden = BTreeSet::new();
    for value in values {
        let target_id = resolve_target_id(session, value)?;
        if session
            .targets
            .iter()
            .any(|target| target.id() == &target_id)
        {
            hidden.insert(target_id);
        }
    }
    Ok(hidden.into_iter().collect())
}

fn require_target(session: &AvatarDyeSession, value: Option<&str>) -> Result<DyeTargetId, String> {
    resolve_target_id(
        session,
        value.ok_or_else(|| "当前操作需要染色目标".to_string())?,
    )
}

fn validate_opaque_id(value: &str) -> Result<(), String> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("染色会话或目标标识无效".to_string());
    }
    Ok(())
}

fn encode_preview_data_url(frame: &super::DyePreviewFrame) -> Result<String, String> {
    let mut bytes = Vec::new();
    WebPEncoder::new_lossless(&mut bytes)
        .write_image(
            &frame.rgba,
            frame.width,
            frame.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|_| "无法生成染色预览缩略图".to_string())?;
    Ok(format!(
        "data:image/webp;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn slot_label(slot: &str) -> &'static str {
    match slot.trim().to_ascii_lowercase().as_str() {
        "skin" => "皮肤",
        "cap" => "帽子",
        "hair" => "头发",
        "face" => "脸部",
        "neck" => "胸部",
        "coat" => "上衣",
        "belt" => "腰部",
        "pants" => "下装",
        "shoes" => "鞋子",
        _ => "时装部位",
    }
}
