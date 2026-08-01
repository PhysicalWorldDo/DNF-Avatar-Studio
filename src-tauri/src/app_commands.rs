use std::path::Path;

use crate::app_config::AppConfigState;
use crate::catalog_gallery::{
    CatalogGalleryPage, CatalogGalleryPageRequest, CatalogGallerySnapshotRequest,
    CatalogGallerySnapshotResponse, CatalogGalleryState, CatalogThumbnailBatchAction,
    CatalogThumbnailBatchPreflight, CatalogThumbnailBatchRequest, CatalogThumbnailBatchStatus,
    CatalogThumbnailRequest, CatalogThumbnailResponse,
};
use crate::catalog_thumbnail::{CacheRetention, ThumbnailCacheSummary};
use crate::dye_color_presets::{DyeColorPresetAppState, DyeColorPresetCatalog};
use crate::feature_gate::HiddenFeature;
use crate::game_resolver::DEFAULT_GAME_DIR;
use crate::models::{
    AppConfigUpdate, AppConfigView, HiddenFeatureUnlockResult, OutfitSwapAnalyzeRequest,
    OutfitSwapExportRequest, OutfitSwapExportResult, OutfitSwapPageResult, OutfitSwapPlan,
    OutfitSwapProgress, ProfessionWideCarrierCandidate, ProfessionWideSwapPlan, WardrobeEntryDraft,
    WardrobeEntryView, WardrobeGroupView, WardrobeRole, WardrobeShareImportResult,
    WardrobeSnapshotInput, WardrobeStateView,
};
use crate::models::{
    AvatarItem, AvatarLayer, AvatarSet, AvatarSetDetail, CatalogMetadataResponse, CatalogPage,
    ExportNpkRequest, ExportNpkResult, ExportWebpRequest, ExportWebpResult, PreviewAnimation,
    PreviewImage, PreviewLayerRequest,
};
use crate::preview_frame::preview_start_frame_for_profession;
use crate::wardrobe::WardrobeAppState;
use tauri::Manager;

#[tauri::command]
pub fn default_game_dir() -> String {
    DEFAULT_GAME_DIR.to_string()
}

#[tauri::command]
pub fn load_dye_color_presets(
    state: tauri::State<'_, DyeColorPresetAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<DyeColorPresetCatalog, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.load_catalog()
}

#[tauri::command]
pub fn create_dye_color_preset(
    expected_revision: u64,
    name: String,
    rgb: [u8; 3],
    state: tauri::State<'_, DyeColorPresetAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<DyeColorPresetCatalog, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.create_preset(expected_revision, &name, rgb)
}

#[tauri::command]
pub fn update_dye_color_preset(
    expected_revision: u64,
    id: String,
    name: String,
    rgb: [u8; 3],
    state: tauri::State<'_, DyeColorPresetAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<DyeColorPresetCatalog, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.update_preset(expected_revision, &id, &name, rgb)
}

#[tauri::command]
pub fn delete_dye_color_preset(
    expected_revision: u64,
    id: String,
    state: tauri::State<'_, DyeColorPresetAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<DyeColorPresetCatalog, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.delete_preset(expected_revision, &id)
}

fn configured_game_dir(
    config_state: &tauri::State<'_, AppConfigState>,
) -> Result<std::path::PathBuf, String> {
    let configured = config_state.load_view()?.game_dir;
    let configured = configured.trim();
    Ok(std::path::PathBuf::from(if configured.is_empty() {
        DEFAULT_GAME_DIR
    } else {
        configured
    }))
}

#[tauri::command]
pub fn load_app_config(
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<AppConfigView, String> {
    config_state.load_view()
}

#[tauri::command]
pub fn save_app_config(
    config: AppConfigUpdate,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<AppConfigView, String> {
    config_state.save_update(config)
}

#[tauri::command]
pub fn unlock_hidden_feature(
    phrase: String,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<HiddenFeatureUnlockResult, String> {
    config_state.unlock_hidden_feature(&phrase)
}

#[tauri::command]
pub async fn load_wardrobe_state(
    app_handle: tauri::AppHandle,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    drop(config_state);
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<WardrobeAppState>();
        state.with_repository(|repository| Ok(repository.view()))
    })
    .await
    .map_err(|_| "执行时装橱窗读取失败".to_string())?
}

#[tauri::command]
pub async fn validate_wardrobe_state(
    app_handle: tauri::AppHandle,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let game_dir = configured_game_dir(&config_state)?;
    drop(config_state);
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<WardrobeAppState>();
        state.with_repository(|repository| {
            let item_ids = repository.referenced_item_ids();
            let catalog = crate::avatar_data::catalog_context_for_item_ids(&item_ids)?;
            let mut validator = crate::avatar_data::WardrobeDyeValidator::new(&game_dir);
            Ok(
                repository.validated_view_with_dye(&catalog, |snapshot, recipe| {
                    validator.validate_from_data_library(snapshot, recipe)
                }),
            )
        })
    })
    .await
    .map_err(|_| "执行时装橱窗资源校验失败".to_string())?
}

#[tauri::command]
pub fn save_wardrobe_active(
    role: WardrobeRole,
    snapshot: WardrobeSnapshotInput,
    clear_incompatible: bool,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&snapshot)?;
    state.with_repository_mut(|repository| {
        let mut item_ids = repository.referenced_item_ids();
        item_ids.extend(crate::avatar_data::wardrobe_snapshot_item_ids(&snapshot));
        let catalog = crate::avatar_data::catalog_context_for_item_ids(&item_ids)?;
        if clear_incompatible {
            repository.save_active_replacing_incompatible(role, snapshot)?;
        } else {
            repository.save_active(role, snapshot)?;
        }
        Ok(repository.validated_view(&catalog))
    })
}

#[tauri::command]
pub fn clear_wardrobe_active(
    role: WardrobeRole,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.clear_active(role))
}

#[tauri::command]
pub fn create_wardrobe_group(
    name: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeGroupView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.create_group(&name))
}

#[tauri::command]
pub fn rename_wardrobe_group(
    group_id: String,
    name: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.rename_group(&group_id, &name))
}

#[tauri::command]
pub fn reorder_wardrobe_groups(
    group_ids: Vec<String>,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.reorder_groups(&group_ids))
}

#[tauri::command]
pub fn delete_wardrobe_group(
    group_id: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.delete_group(&group_id))
}

#[tauri::command]
pub fn create_wardrobe_entry(
    snapshot: WardrobeSnapshotInput,
    draft: WardrobeEntryDraft,
    allow_duplicate: bool,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeEntryView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&snapshot)?;
    state.with_repository_mut(|repository| {
        repository.create_entry_with_duplicate_policy(snapshot, draft, allow_duplicate)
    })
}

#[tauri::command]
pub fn find_matching_wardrobe_entry(
    snapshot: WardrobeSnapshotInput,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<Option<WardrobeEntryView>, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository(|repository| repository.find_matching_entry(&snapshot))
}

#[tauri::command]
pub fn update_wardrobe_entry(
    entry_id: String,
    draft: WardrobeEntryDraft,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.update_entry(&entry_id, draft))
}

#[tauri::command]
pub fn replace_wardrobe_entry_snapshot(
    entry_id: String,
    snapshot: WardrobeSnapshotInput,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&snapshot)?;
    state.with_repository_mut(|repository| repository.replace_entry_snapshot(&entry_id, snapshot))
}

#[tauri::command]
pub fn set_wardrobe_favorite(
    entry_id: String,
    favorite: bool,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.set_favorite(&entry_id, favorite))
}

#[tauri::command]
pub fn activate_wardrobe_entry(
    entry_id: String,
    role: WardrobeRole,
    clear_incompatible: bool,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let game_dir = configured_game_dir(&config_state)?;
    state.with_repository_mut(|repository| {
        let private = repository.entry_private_snapshot(&entry_id)?;
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(
            &private.snapshot,
        )?;
        let mut validator = crate::avatar_data::WardrobeDyeValidator::new(&game_dir);
        validator.ensure_from_data_library(&private.snapshot, private.dye_recipe.as_ref())?;
        let item_ids = repository.referenced_item_ids();
        let catalog = crate::avatar_data::catalog_context_for_item_ids(&item_ids)?;
        if clear_incompatible {
            repository.activate_entry_replacing_incompatible(&entry_id, role)?;
        } else {
            repository.activate_entry(&entry_id, role)?;
        }
        Ok(
            repository.validated_view_with_dye(&catalog, |snapshot, recipe| {
                validator.validate_from_data_library(snapshot, recipe)
            }),
        )
    })
}

#[tauri::command]
pub fn delete_wardrobe_entry(
    entry_id: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository_mut(|repository| repository.delete_entry(&entry_id))
}

#[tauri::command]
pub fn export_wardrobe_entry_share(
    entry_id: String,
    output_path: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<usize, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository(|repository| {
        repository.export_entry_share_file(&entry_id, Path::new(&output_path))
    })
}

#[tauri::command]
pub fn export_wardrobe_group_share(
    group_id: String,
    output_path: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<usize, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.with_repository(|repository| {
        repository.export_group_share_file(&group_id, Path::new(&output_path))
    })
}

#[tauri::command]
pub fn import_wardrobe_share(
    input_path: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeShareImportResult, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let game_dir = configured_game_dir(&config_state)?;
    crate::avatar_data::open_catalog_index_from_data_library()?;
    let (imported, wardrobe) = state.with_repository_mut(|repository| {
        let mut validator = crate::avatar_data::WardrobeDyeValidator::new(&game_dir);
        repository.import_share_file_validated(Path::new(&input_path), |candidate| {
            let item_ids = candidate.referenced_item_ids();
            let catalog = crate::avatar_data::catalog_context_for_item_ids(&item_ids)?;
            Ok(
                candidate.validated_view_with_dye(&catalog, |snapshot, recipe| {
                    validator.validate_from_data_library(snapshot, recipe)
                }),
            )
        })
    })?;
    Ok(WardrobeShareImportResult {
        wardrobe,
        entry_count: imported.entry_count,
        group_name: imported.group_name,
    })
}

#[tauri::command]
pub fn reset_wardrobe_after_confirmation(
    confirmed: bool,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    state.reset_after_confirmation(confirmed)
}

#[tauri::command]
pub fn open_avatar_catalog_index() -> Result<CatalogMetadataResponse, String> {
    crate::avatar_data::open_catalog_index_from_data_library()
}

#[tauri::command]
pub fn query_avatar_items(
    slot: String,
    playable_role: String,
    query: String,
) -> Result<Vec<AvatarItem>, String> {
    crate::avatar_data::query_avatar_items_from_data_library(&slot, &playable_role, &query)
}

#[tauri::command]
pub fn query_avatar_items_page(
    slot: String,
    playable_role: String,
    query: String,
    offset: usize,
    limit: usize,
) -> Result<CatalogPage<AvatarItem>, String> {
    crate::avatar_data::query_avatar_items_page_from_data_library(
        &slot,
        &playable_role,
        &query,
        offset,
        limit,
    )
}

#[tauri::command]
pub fn query_avatar_sets(playable_role: String, query: String) -> Result<Vec<AvatarSet>, String> {
    crate::avatar_data::query_avatar_sets_from_data_library(&playable_role, &query)
}

#[tauri::command]
pub fn query_avatar_sets_page(
    playable_role: String,
    query: String,
    offset: usize,
    limit: usize,
) -> Result<CatalogPage<AvatarSet>, String> {
    crate::avatar_data::query_avatar_sets_page_from_data_library(
        &playable_role,
        &query,
        offset,
        limit,
    )
}

#[tauri::command]
pub fn query_catalog_gallery_page(
    request: CatalogGalleryPageRequest,
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<CatalogGalleryPage, String> {
    state.query_page(request)
}

#[tauri::command]
pub async fn load_catalog_gallery_snapshot(
    request: CatalogGallerySnapshotRequest,
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<CatalogGallerySnapshotResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.load_snapshot(request))
        .await
        .map_err(|_| "执行图库快照校验失败".to_string())?
}

#[tauri::command]
pub fn load_catalog_thumbnail(
    request: CatalogThumbnailRequest,
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<CatalogThumbnailResponse, String> {
    state.load_thumbnail(request, CacheRetention::Automatic)
}

#[tauri::command]
pub fn get_catalog_thumbnail_cache_summary(
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<ThumbnailCacheSummary, String> {
    state.cache_summary()
}

#[tauri::command]
pub fn prepare_catalog_thumbnail_batch(
    request: CatalogThumbnailBatchRequest,
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<CatalogThumbnailBatchPreflight, String> {
    state.prepare_batch(&request)
}

#[tauri::command]
pub fn start_catalog_thumbnail_batch(
    request: CatalogThumbnailBatchRequest,
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<CatalogThumbnailBatchStatus, String> {
    state.start_batch(request)
}

#[tauri::command]
pub fn control_catalog_thumbnail_batch(
    job_id: String,
    action: CatalogThumbnailBatchAction,
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<CatalogThumbnailBatchStatus, String> {
    state.control_batch(&job_id, action)
}

#[tauri::command]
pub fn get_catalog_thumbnail_batch_status(
    job_id: String,
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<CatalogThumbnailBatchStatus, String> {
    state.batch_status(&job_id)
}

#[tauri::command]
pub fn list_catalog_thumbnail_batch_jobs(
    state: tauri::State<'_, CatalogGalleryState>,
) -> Result<Vec<CatalogThumbnailBatchStatus>, String> {
    state.batch_jobs()
}

#[tauri::command]
pub fn get_avatar_items_by_ids(item_ids: Vec<String>) -> Result<Vec<AvatarItem>, String> {
    crate::avatar_data::get_avatar_items_by_ids_from_data_library(&item_ids)
}

#[tauri::command]
pub fn get_avatar_set_with_members(
    set_id: String,
    playable_role: String,
) -> Result<AvatarSetDetail, String> {
    crate::avatar_data::get_avatar_set_with_members_from_data_library(&set_id, &playable_role)
}

#[tauri::command]
pub fn load_avatar_layers(
    item_ids: Vec<String>,
    playable_role_key: String,
) -> Result<Vec<AvatarLayer>, String> {
    crate::avatar_data::load_layers_for_playable_role_from_data_library(
        &item_ids,
        &playable_role_key,
    )
}

#[tauri::command]
pub fn render_avatar_layers(
    game_dir: String,
    layers: Vec<PreviewLayerRequest>,
) -> Result<PreviewImage, String> {
    crate::renderer::render_layers_from_game_dir(Path::new(&game_dir), &layers)
}

#[tauri::command]
pub fn render_avatar_animation(
    game_dir: String,
    layers: Vec<PreviewLayerRequest>,
) -> Result<PreviewAnimation, String> {
    crate::renderer::render_animation_from_game_dir(Path::new(&game_dir), &layers)
}

#[tauri::command]
pub fn export_avatar_npk(request: ExportNpkRequest) -> Result<ExportNpkResult, String> {
    crate::exporter::export_avatar_npk(request)
}

#[tauri::command]
pub fn export_avatar_webp(request: ExportWebpRequest) -> Result<ExportWebpResult, String> {
    crate::exporter::export_avatar_webp(request)
}

#[tauri::command]
pub fn analyze_outfit_swap(
    request: OutfitSwapAnalyzeRequest,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<OutfitSwapPlan, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    crate::avatar_data::ensure_outfit_snapshot_role_binding_from_data_library(&request.current)?;
    crate::avatar_data::ensure_outfit_snapshot_role_binding_from_data_library(&request.target)?;
    let current_layers = crate::avatar_data::load_layers_for_playable_role_from_data_library(
        &request.current.item_ids,
        &request.current.playable_role_key,
    )?;
    let target_layers = crate::avatar_data::load_layers_for_playable_role_from_data_library(
        &request.target.item_ids,
        &request.target.playable_role_key,
    )?;
    crate::outfit_swap::analyze_outfit_swap_from_layers(
        Path::new(&request.game_dir),
        &request.current,
        &request.target,
        &current_layers,
        &target_layers,
    )
}

#[tauri::command]
pub fn export_outfit_swap_npk(
    request: OutfitSwapExportRequest,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<OutfitSwapExportResult, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    crate::avatar_data::ensure_outfit_snapshot_role_binding_from_data_library(&request.current)?;
    crate::avatar_data::ensure_outfit_snapshot_role_binding_from_data_library(&request.target)?;
    let current_layers = crate::avatar_data::load_layers_for_playable_role_from_data_library(
        &request.current.item_ids,
        &request.current.playable_role_key,
    )?;
    let target_layers = crate::avatar_data::load_layers_for_playable_role_from_data_library(
        &request.target.item_ids,
        &request.target.playable_role_key,
    )?;
    crate::outfit_swap::export_outfit_swap_from_layers(
        Path::new(&request.game_dir),
        Path::new(&request.output_path),
        &request.current,
        &request.target,
        &current_layers,
        &target_layers,
    )
}

#[tauri::command]
pub fn analyze_active_outfit_swap(
    game_dir: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<OutfitSwapPlan, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let result = (|| {
        let ((current_input, current, current_recipe), (target_input, target, target_recipe)) =
            state.with_repository(|repository| repository.active_dye_aware_pair())?;
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&current_input)?;
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&target_input)?;
        let current_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&current_input)?;
        let target_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&target_input)?;
        crate::outfit_swap::analyze_outfit_swap_from_layers_with_dye(
            Path::new(&game_dir),
            &current,
            &target,
            &current_layers,
            &target_layers,
            current_recipe.as_ref(),
            target_recipe.as_ref(),
        )
    })();
    result.map_err(|_| "一键换装分析失败：时装资源无法读取或不完整".to_string())
}

pub(crate) fn outfit_swap_analysis_error(_error: String) -> String {
    "一键换装分析失败：时装资源无法读取或不完整".to_string()
}

#[tauri::command]
pub async fn render_active_outfit(
    role: WardrobeRole,
    game_dir: String,
    linear_dodge: bool,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<PreviewImage, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let (input, snapshot, recipe) = state
        .with_repository(|repository| repository.active_dye_aware_snapshot(role))
        .map_err(|_| "读取已保存装扮失败".to_string())?;
    let target_effect = if role == WardrobeRole::Target {
        target_visual_effect(linear_dodge)
    } else {
        crate::outfit_swap::TargetVisualEffect::None
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&input)?;
        let mut layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&input)?;
        if input.weapon.is_some() {
            let weapon_layers =
                crate::avatar_data::load_wardrobe_weapon_layers_from_data_library(&input)?;
            layers.extend(
                crate::weapon_swap::deduplicate_equivalent_weapon_layers(
                    weapon_layers.iter().collect(),
                )
                .into_iter()
                .cloned(),
            );
        }
        let preview_start_frame = preview_start_frame_for_profession(&input.playable_role_key);
        let preview =
            crate::outfit_swap::render_outfit_preview_frame_from_layers_with_dye_and_target_effect(
                Path::new(&game_dir),
                &snapshot,
                &layers,
                recipe.as_ref(),
                preview_start_frame,
                target_effect,
            )?;
        if !preview.missing_imgs.is_empty() {
            return Err("一键换装预览资源不完整".to_string());
        }
        Ok(preview)
    })
    .await
    .map_err(|_| "装扮预览后台任务失败".to_string())?;
    result.map_err(|_| "装扮预览失败：时装资源无法读取或不完整".to_string())
}

#[tauri::command]
pub async fn render_wardrobe_entry_preview(
    entry_id: String,
    game_dir: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<PreviewImage, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let (input, snapshot, recipe) = state
        .with_repository(|repository| repository.entry_dye_aware_snapshot(&entry_id))
        .map_err(|_| "读取橱窗装扮失败".to_string())?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&input)?;
        let mut layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&input)?;
        if input.weapon.is_some() {
            let weapon_layers =
                crate::avatar_data::load_wardrobe_weapon_layers_from_data_library(&input)?;
            layers.extend(
                crate::weapon_swap::deduplicate_equivalent_weapon_layers(
                    weapon_layers.iter().collect(),
                )
                .into_iter()
                .cloned(),
            );
        }
        let preview_start_frame = preview_start_frame_for_profession(&input.playable_role_key);
        let preview = crate::outfit_swap::render_outfit_preview_frame_from_layers_with_dye(
            Path::new(&game_dir),
            &snapshot,
            &layers,
            recipe.as_ref(),
            preview_start_frame,
        )?;
        if !preview.missing_imgs.is_empty() {
            return Err("橱窗预览资源不完整".to_string());
        }
        Ok(preview)
    })
    .await
    .map_err(|_| "橱窗预览后台任务失败".to_string())?;
    result.map_err(|_| "橱窗预览失败：时装资源无法读取或不完整".to_string())
}

#[tauri::command]
pub async fn load_active_outfit_swap_page(
    game_dir: String,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<OutfitSwapPageResult, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let ((current_input, current, current_recipe), (target_input, target, target_recipe)) = state
        .with_repository(|repository| repository.active_dye_aware_pair())
        .map_err(|_| "读取一键换装工作区失败".to_string())?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&current_input)?;
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&target_input)?;
        let current_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&current_input)?;
        let target_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&target_input)?;
        let mut plan = crate::outfit_swap::analyze_outfit_swap_from_layers_with_dye(
            Path::new(&game_dir),
            &current,
            &target,
            &current_layers,
            &target_layers,
            current_recipe.as_ref(),
            target_recipe.as_ref(),
        )?;
        sanitize_outfit_swap_plan(&mut plan);
        Ok(OutfitSwapPageResult {
            plan,
            current_preview: unavailable_outfit_preview(),
            target_preview: unavailable_outfit_preview(),
            current_preview_available: false,
            target_preview_available: false,
        })
    })
    .await
    .map_err(|_| "一键换装后台任务失败".to_string())?;
    result.map_err(outfit_swap_analysis_error)
}

fn unavailable_outfit_preview() -> PreviewImage {
    PreviewImage {
        width: 1,
        height: 1,
        rgba_base64: "AAAAAA==".to_string(),
        missing_imgs: Vec::new(),
    }
}

fn sanitize_outfit_swap_plan(plan: &mut OutfitSwapPlan) {
    for part in &mut plan.parts {
        part.carrier_img.clear();
        part.transparent_imgs.clear();
    }
}

#[tauri::command]
pub async fn export_active_outfit_swap_npk(
    game_dir: String,
    output_path: String,
    linear_dodge: bool,
    on_progress: tauri::ipc::Channel<OutfitSwapProgress>,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<OutfitSwapExportResult, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let ((current_input, current, current_recipe), (target_input, target, target_recipe)) = state
        .with_repository(|repository| repository.active_dye_aware_pair())
        .map_err(|_| "读取一键换装工作区失败".to_string())?;
    let target_effect = target_visual_effect(linear_dodge);
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&current_input)?;
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&target_input)?;
        let current_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&current_input)?;
        let target_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&target_input)?;
        crate::outfit_swap::export_outfit_swap_from_layers_with_dye_and_target_effect_and_progress(
            Path::new(&game_dir),
            Path::new(&output_path),
            &current,
            &target,
            &current_layers,
            &target_layers,
            current_recipe.as_ref(),
            target_recipe.as_ref(),
            target_effect,
            |event| {
                let _ = on_progress.send(event);
            },
        )
    })
    .await
    .map_err(|_| "换装补丁后台任务失败".to_string())?;
    result.map_err(|_| "换装补丁生成失败：请检查装扮资源和输出位置".to_string())
}

#[tauri::command]
pub async fn export_active_weapon_swap_npk(
    game_dir: String,
    output_path: String,
    linear_dodge: bool,
    on_progress: tauri::ipc::Channel<OutfitSwapProgress>,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<OutfitSwapExportResult, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let (current, target) = state
        .with_repository(|repository| repository.active_input_pair())
        .map_err(|_| "读取武器换装工作区失败".to_string())?;
    let target_effect = target_visual_effect(linear_dodge);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let current_layers =
            crate::avatar_data::load_wardrobe_weapon_layers_from_data_library(&current)?;
        let target_layers =
            crate::avatar_data::load_wardrobe_weapon_layers_from_data_library(&target)?;
        crate::weapon_swap::export_weapon_swap_from_layers_with_progress(
            Path::new(&game_dir),
            Path::new(&output_path),
            &current.visual_family_key,
            &target.visual_family_key,
            &current_layers,
            &target_layers,
            target_effect,
            |event| {
                let _ = on_progress.send(event);
            },
        )
    })
    .await
    .map_err(|_| "武器换装后台任务失败".to_string())?;
    result.map_err(|_| "武器换装补丁生成失败：请检查武器资源和输出位置".to_string())
}

#[tauri::command]
pub async fn list_active_profession_wide_carriers(
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<Vec<ProfessionWideCarrierCandidate>, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let (current_input, current, _) = state
        .with_repository(|repository| repository.active_dye_aware_snapshot(WardrobeRole::Current))
        .map_err(|_| "读取当前穿戴失败".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&current_input)?;
        let current_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&current_input)?;
        crate::profession_wide_swap::list_profession_wide_carrier_candidates(
            &current,
            &current_layers,
        )
    })
    .await
    .map_err(|_| "载体候选加载后台任务失败".to_string())?
}

#[tauri::command]
pub async fn preview_active_profession_wide_single_carrier(
    game_dir: String,
    carrier_slot: String,
    linear_dodge: bool,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<ProfessionWideSwapPlan, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let ((current_input, current, _), (target_input, target, target_recipe)) = state
        .with_repository(|repository| repository.active_dye_aware_pair())
        .map_err(|_| "读取一键换装工作区失败".to_string())?;
    let issued_at_ms = command_now_ms();
    let target_effect = target_visual_effect(linear_dodge);
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&current_input)?;
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&target_input)?;
        let current_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&current_input)?;
        let target_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&target_input)?;
        crate::profession_wide_swap::preview_profession_wide_single_carrier_from_layers_with_target_effect(
            Path::new(&game_dir),
            &current,
            &target,
            &current_layers,
            &target_layers,
            target_recipe.as_ref(),
            target_effect,
            &carrier_slot,
            issued_at_ms,
        )
    })
    .await
    .map_err(|_| "单时装特殊分析后台任务失败".to_string())?;
    result.map_err(outfit_swap_analysis_error)
}

#[tauri::command]
pub async fn export_active_profession_wide_single_carrier_npk(
    game_dir: String,
    output_path: String,
    carrier_slot: String,
    confirmation_token: String,
    linear_dodge: bool,
    on_progress: tauri::ipc::Channel<OutfitSwapProgress>,
    state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<OutfitSwapExportResult, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let ((current_input, current, _), (target_input, target, target_recipe)) = state
        .with_repository(|repository| repository.active_dye_aware_pair())
        .map_err(|_| "读取一键换装工作区失败".to_string())?;
    let now_ms = command_now_ms();
    let target_effect = target_visual_effect(linear_dodge);
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&current_input)?;
        crate::avatar_data::ensure_wardrobe_snapshot_resources_from_data_library(&target_input)?;
        let current_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&current_input)?;
        let target_layers =
            crate::avatar_data::load_complete_wardrobe_layers_from_data_library(&target_input)?;
        crate::profession_wide_swap::export_profession_wide_single_carrier_from_layers_with_target_effect(
            Path::new(&game_dir),
            Path::new(&output_path),
            &current,
            &target,
            &current_layers,
            &target_layers,
            target_recipe.as_ref(),
            target_effect,
            &carrier_slot,
            &confirmation_token,
            now_ms,
            |event| {
                let _ = on_progress.send(event);
            },
        )
    })
    .await
    .map_err(|_| "单时装特殊补丁后台任务失败".to_string())?;
    result.map_err(|_| "单时装特殊补丁生成失败：请重新分析资源范围和输出位置".to_string())
}

fn target_visual_effect(linear_dodge: bool) -> crate::outfit_swap::TargetVisualEffect {
    if linear_dodge {
        crate::outfit_swap::TargetVisualEffect::LinearDodge
    } else {
        crate::outfit_swap::TargetVisualEffect::None
    }
}

fn command_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn avatar_dye_view_with_workspace(
    session_id: &str,
    dye_state: &crate::avatar_dye::AvatarDyeAppState,
    wardrobe_state: &WardrobeAppState,
) -> Result<serde_json::Value, String> {
    let view = dye_state.view(session_id)?;
    wardrobe_state.with_repository(|repository| {
        let wardrobe = repository.view();
        crate::avatar_dye::attach_workspace_summaries(
            view,
            wardrobe.current.as_ref(),
            wardrobe.target.as_ref(),
        )
    })
}

fn avatar_dye_public_error(context: &str) -> String {
    format!("{context}失败：请检查时装资源、会话状态和输入参数")
}

fn avatar_dye_wardrobe_save_error(error: String) -> String {
    if error.contains("已存在内容完全相同") {
        return "橱窗中已存在相同时装与配色，不能重复保存；请取消后更新已有记录，或返回修改配色"
            .to_string();
    }
    avatar_dye_public_error("保存染色装扮到橱窗")
}

#[tauri::command]
pub async fn open_avatar_dye_session_from_studio(
    game_dir: String,
    snapshot: WardrobeSnapshotInput,
    app_handle: tauri::AppHandle,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    drop(config_state);
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dye_state = app_handle.state::<crate::avatar_dye::AvatarDyeAppState>();
        let wardrobe_state = app_handle.state::<WardrobeAppState>();
        let session_id = dye_state
            .open_from_snapshot(Path::new(&game_dir), snapshot, None)
            .map_err(|_| avatar_dye_public_error("打开染色工作台"))?;
        avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state).map_err(|_| {
            let _ = dye_state.close(&session_id);
            avatar_dye_public_error("读取染色工作台")
        })
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行打开染色工作台"))?
}

#[tauri::command]
pub async fn open_avatar_dye_session_from_wardrobe(
    game_dir: String,
    entry_id: String,
    app_handle: tauri::AppHandle,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    drop(config_state);
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dye_state = app_handle.state::<crate::avatar_dye::AvatarDyeAppState>();
        let wardrobe_state = app_handle.state::<WardrobeAppState>();
        let (snapshot, dye_payload) = wardrobe_state
            .with_repository(|repository| {
                let (snapshot, _, dye_payload) = repository.entry_dye_state_snapshot(&entry_id)?;
                Ok((snapshot, dye_payload))
            })
            .map_err(|_| avatar_dye_public_error("读取橱窗染色记录"))?;
        let session_id = dye_state
            .open_from_snapshot_with_dye_state(Path::new(&game_dir), snapshot, dye_payload)
            .map_err(|_| avatar_dye_public_error("打开橱窗染色工作台"))?;
        avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state).map_err(|_| {
            let _ = dye_state.close(&session_id);
            avatar_dye_public_error("读取染色工作台")
        })
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行打开橱窗染色工作台"))?
}

#[tauri::command]
pub fn get_avatar_dye_session_view(
    session_id: String,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    wardrobe_state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state)
        .map_err(|_| avatar_dye_public_error("读取染色会话"))
}

#[tauri::command]
pub fn update_avatar_dye_colors(
    session_id: String,
    target_id: String,
    operation: crate::avatar_dye::DyeColorOperation,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    wardrobe_state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    dye_state
        .update_colors(&session_id, &target_id, operation)
        .map_err(|_| avatar_dye_public_error("更新染色颜色"))?;
    avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state)
        .map_err(|_| avatar_dye_public_error("刷新染色会话"))
}

#[tauri::command]
pub fn update_avatar_dye_batch(
    session_id: String,
    mutation: crate::avatar_dye::DyeBatchMutation,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    wardrobe_state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    dye_state
        .update_batch(&session_id, &mutation)
        .map_err(|_| avatar_dye_public_error("批量更新染色颜色"))?;
    avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state)
        .map_err(|_| avatar_dye_public_error("刷新染色会话"))
}

#[tauri::command]
pub fn repair_avatar_dye_target(
    session_id: String,
    target_id: String,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    wardrobe_state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    dye_state
        .repair_target(&session_id, &target_id)
        .map_err(|_| avatar_dye_public_error("修复染色目标"))?;
    avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state)
        .map_err(|_| avatar_dye_public_error("刷新染色会话"))
}

#[tauri::command]
pub fn reset_avatar_dye_scope(
    session_id: String,
    scope: String,
    target_id: Option<String>,
    color_indices: Option<Vec<u16>>,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    wardrobe_state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    dye_state
        .reset_scope(
            &session_id,
            &scope,
            target_id.as_deref(),
            color_indices.as_deref().unwrap_or(&[]),
        )
        .map_err(|_| avatar_dye_public_error("恢复染色范围"))?;
    avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state)
        .map_err(|_| avatar_dye_public_error("刷新染色会话"))
}

#[tauri::command]
pub fn undo_avatar_dye(
    session_id: String,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    wardrobe_state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    dye_state
        .undo(&session_id)
        .map_err(|_| avatar_dye_public_error("撤销染色"))?;
    avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state)
        .map_err(|_| avatar_dye_public_error("刷新染色会话"))
}

#[tauri::command]
pub fn redo_avatar_dye(
    session_id: String,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    wardrobe_state: tauri::State<'_, WardrobeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<serde_json::Value, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    dye_state
        .redo(&session_id)
        .map_err(|_| avatar_dye_public_error("重做染色"))?;
    avatar_dye_view_with_workspace(&session_id, &dye_state, &wardrobe_state)
        .map_err(|_| avatar_dye_public_error("刷新染色会话"))
}

#[tauri::command]
pub async fn render_avatar_dye_frame(
    session_id: String,
    frame_index: usize,
    compare_mode: crate::avatar_dye::DyeCompareMode,
    preview_scope: crate::avatar_dye::DyePreviewScope,
    target_id: String,
    highlight: bool,
    hide_selected_mask: bool,
    hidden_target_ids: Vec<String>,
    request_id: u64,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeFrameResponse, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取染色帧会话"))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::render_frame_from_record(
            record,
            frame_index,
            compare_mode,
            preview_scope,
            &target_id,
            highlight,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行染色帧预览"))?
    .map_err(|_| avatar_dye_public_error("生成染色帧预览"))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn render_avatar_dye_comparison_frame(
    session_id: String,
    frame_index: usize,
    preview_scope: crate::avatar_dye::DyePreviewScope,
    target_id: String,
    hide_selected_mask: bool,
    hidden_target_ids: Vec<String>,
    request_id: u64,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeComparisonFrameResponse, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取染色对比帧会话"))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::render_comparison_frame_from_record(
            record,
            frame_index,
            preview_scope,
            &target_id,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行染色对比帧预览"))?
    .map_err(|_| avatar_dye_public_error("生成染色对比帧预览"))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn preview_avatar_dye_colors(
    session_id: String,
    frame_index: usize,
    compare_mode: crate::avatar_dye::DyeCompareMode,
    preview_scope: crate::avatar_dye::DyePreviewScope,
    target_id: String,
    operation: crate::avatar_dye::DyeColorOperation,
    highlight: bool,
    hide_selected_mask: bool,
    hidden_target_ids: Vec<String>,
    request_id: u64,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeFrameResponse, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取染色临时预览会话"))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::render_transient_colors_frame_from_record(
            record,
            frame_index,
            compare_mode,
            preview_scope,
            &target_id,
            operation,
            highlight,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行染色临时预览"))?
    .map_err(|_| avatar_dye_public_error("生成染色临时预览"))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn preview_avatar_dye_comparison_colors(
    session_id: String,
    frame_index: usize,
    preview_scope: crate::avatar_dye::DyePreviewScope,
    target_id: String,
    operation: crate::avatar_dye::DyeColorOperation,
    hide_selected_mask: bool,
    hidden_target_ids: Vec<String>,
    request_id: u64,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeComparisonFrameResponse, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取染色临时对比会话"))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::render_transient_comparison_frame_from_record(
            record,
            frame_index,
            preview_scope,
            &target_id,
            operation,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行染色临时对比预览"))?
    .map_err(|_| avatar_dye_public_error("生成染色临时对比预览"))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn preview_avatar_dye_batch_comparison(
    session_id: String,
    frame_index: usize,
    preview_scope: crate::avatar_dye::DyePreviewScope,
    target_id: String,
    mutation: crate::avatar_dye::DyeBatchMutation,
    hide_selected_mask: bool,
    hidden_target_ids: Vec<String>,
    request_id: u64,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeComparisonFrameResponse, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取批量染色临时预览会话"))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::render_transient_batch_comparison_frame_from_record(
            record,
            frame_index,
            preview_scope,
            &target_id,
            &mutation,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行批量染色临时对比预览"))?
    .map_err(|_| avatar_dye_public_error("生成批量染色临时对比预览"))
}

#[tauri::command]
pub async fn render_avatar_dye_animation(
    session_id: String,
    compare_mode: crate::avatar_dye::DyeCompareMode,
    preview_scope: crate::avatar_dye::DyePreviewScope,
    target_id: String,
    highlight: bool,
    hide_selected_mask: bool,
    hidden_target_ids: Vec<String>,
    request_id: u64,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeAnimationResponse, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取染色动画会话"))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::render_animation_from_record(
            record,
            compare_mode,
            preview_scope,
            &target_id,
            highlight,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行染色动画预览"))?
    .map_err(|_| avatar_dye_public_error("生成染色动画预览"))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn render_avatar_dye_comparison_animation(
    session_id: String,
    preview_scope: crate::avatar_dye::DyePreviewScope,
    target_id: String,
    hide_selected_mask: bool,
    hidden_target_ids: Vec<String>,
    request_id: u64,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeComparisonAnimationResponse, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取染色对比动画会话"))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::render_comparison_animation_from_record(
            record,
            preview_scope,
            &target_id,
            hide_selected_mask,
            &hidden_target_ids,
            request_id,
        )
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行染色对比动画预览"))?
    .map_err(|_| avatar_dye_public_error("生成染色对比动画预览"))
}

#[tauri::command]
pub async fn save_avatar_dye_to_wardrobe(
    session_id: String,
    draft: WardrobeEntryDraft,
    allow_duplicate: bool,
    app_handle: tauri::AppHandle,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    drop(config_state);
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dye_state = app_handle.state::<crate::avatar_dye::AvatarDyeAppState>();
        let wardrobe_state = app_handle.state::<WardrobeAppState>();
        let prepared = dye_state
            .prepare_persistence(&session_id)
            .map_err(|_| avatar_dye_public_error("准备保存染色装扮"))?;
        let revision = prepared.revision;
        let view = wardrobe_state
            .with_repository_mut(|repository| {
                repository.create_entry_with_duplicate_policy_and_dye_state(
                    prepared.snapshot,
                    prepared.dye_state,
                    draft,
                    allow_duplicate,
                )?;
                Ok(repository.view())
            })
            .map_err(avatar_dye_wardrobe_save_error)?;
        let _ = dye_state.mark_persisted_if_unchanged(&session_id, revision);
        Ok(view)
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行保存染色装扮到橱窗"))?
}

#[tauri::command]
pub async fn save_avatar_dye_as_active(
    session_id: String,
    role: WardrobeRole,
    clear_incompatible: bool,
    app_handle: tauri::AppHandle,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    drop(config_state);
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dye_state = app_handle.state::<crate::avatar_dye::AvatarDyeAppState>();
        let wardrobe_state = app_handle.state::<WardrobeAppState>();
        let prepared = dye_state
            .prepare_persistence(&session_id)
            .map_err(|_| avatar_dye_public_error("准备写入染色工作区"))?;
        let revision = prepared.revision;
        let view = wardrobe_state
            .with_repository_mut(|repository| {
                if clear_incompatible {
                    repository.save_active_replacing_incompatible_with_dye_state(
                        role,
                        prepared.snapshot,
                        prepared.dye_state,
                    )
                } else {
                    repository.save_active_with_dye_state(
                        role,
                        prepared.snapshot,
                        prepared.dye_state,
                    )
                }
            })
            .map_err(|_| avatar_dye_public_error("写入染色工作区"))?;
        let _ = dye_state.mark_persisted_if_unchanged(&session_id, revision);
        Ok(view)
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行写入染色工作区"))?
}

#[tauri::command]
pub async fn save_avatar_dye_as_target_and_open_swap(
    session_id: String,
    clear_incompatible: bool,
    app_handle: tauri::AppHandle,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<WardrobeStateView, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    drop(config_state);
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dye_state = app_handle.state::<crate::avatar_dye::AvatarDyeAppState>();
        let wardrobe_state = app_handle.state::<WardrobeAppState>();
        let prepared = dye_state
            .prepare_persistence(&session_id)
            .map_err(|_| avatar_dye_public_error("准备保存目标染色外观"))?;
        let revision = prepared.revision;
        let view = wardrobe_state
            .with_repository_mut(|repository| {
                if clear_incompatible {
                    repository.save_active_replacing_incompatible_with_dye_state(
                        WardrobeRole::Target,
                        prepared.snapshot,
                        prepared.dye_state,
                    )
                } else {
                    repository.save_active_with_dye_state(
                        WardrobeRole::Target,
                        prepared.snapshot,
                        prepared.dye_state,
                    )
                }
            })
            .map_err(|_| avatar_dye_public_error("保存目标染色外观"))?;
        let _ = dye_state.mark_persisted_if_unchanged(&session_id, revision);
        Ok(view)
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行保存目标染色外观"))?
}

#[tauri::command]
pub async fn export_avatar_dye_npk(
    session_id: String,
    output_directory: Option<String>,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<crate::avatar_dye::DyeNpkExportSummary, String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    let record = dye_state
        .record(&session_id)
        .map_err(|_| avatar_dye_public_error("读取染色导出会话"))?;
    let output_directory = output_directory.map(std::path::PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || {
        crate::avatar_dye::AvatarDyeAppState::export_npk_from_record(record, output_directory)
    })
    .await
    .map_err(|_| avatar_dye_public_error("执行染色补丁导出"))?
    .map_err(|_| avatar_dye_public_error("导出染色补丁"))
}

#[tauri::command]
pub fn close_avatar_dye_session(
    session_id: String,
    dye_state: tauri::State<'_, crate::avatar_dye::AvatarDyeAppState>,
    config_state: tauri::State<'_, AppConfigState>,
) -> Result<(), String> {
    config_state.ensure_unlocked(HiddenFeature::WardrobeSwap)?;
    dye_state
        .close(&session_id)
        .map_err(|_| avatar_dye_public_error("关闭染色会话"))
}
