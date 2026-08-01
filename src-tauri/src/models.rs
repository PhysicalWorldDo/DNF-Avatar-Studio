use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::feature_gate::HiddenFeature;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarLayer {
    pub item_id: String,
    pub item_name: String,
    pub slot: String,
    pub visual_family_key: String,
    pub layer_order: i32,
    pub equipment_type_enum: u16,
    #[serde(default)]
    pub hide_equipment_type_enums: Vec<u16>,
    #[serde(default)]
    pub enable_dye_mask1: bool,
    #[serde(default)]
    pub enable_dye_mask2: bool,
    pub layer_sequence: u32,
    pub layer_tag: String,
    pub variation: String,
    pub palette_index: usize,
    pub visual_img: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct AvatarLayerDrawOrder {
    pub layer_order: i32,
    pub equipment_type_enum: u16,
    pub layer_sequence: u32,
}

impl AvatarLayerDrawOrder {
    pub(crate) const fn new(
        layer_order: i32,
        equipment_type_enum: u16,
        layer_sequence: u32,
    ) -> Self {
        Self {
            layer_order,
            equipment_type_enum,
            layer_sequence,
        }
    }
}

impl AvatarLayer {
    pub(crate) const fn draw_order(&self) -> AvatarLayerDrawOrder {
        AvatarLayerDrawOrder::new(
            self.layer_order,
            self.equipment_type_enum,
            self.layer_sequence,
        )
    }

    pub(crate) const fn enable_dye_mask(&self, number: u32) -> bool {
        enable_dye_mask(number, self.enable_dye_mask1, self.enable_dye_mask2)
    }
}

pub(crate) const fn enable_dye_mask(number: u32, mask1: bool, mask2: bool) -> bool {
    match number {
        1 => mask1,
        2 => mask2,
        _ => false,
    }
}

pub(crate) fn sort_avatar_layers_for_draw(layers: &mut [AvatarLayer]) {
    layers.sort_by_key(AvatarLayer::draw_order);
}

pub(crate) fn visible_avatar_layers(layers: &[AvatarLayer]) -> Vec<AvatarLayer> {
    let mut rules_by_item = HashMap::<&str, HashSet<u16>>::new();
    for layer in layers {
        rules_by_item
            .entry(layer.item_id.as_str())
            .or_default()
            .extend(layer.hide_equipment_type_enums.iter().copied());
    }
    layers
        .iter()
        .filter(|target| {
            !rules_by_item.iter().any(|(source_item_id, hidden_types)| {
                *source_item_id != target.item_id.as_str()
                    && hidden_types.contains(&target.equipment_type_enum)
            })
        })
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarRole {
    pub playable_role_key: String,
    pub visual_family_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarVisualFamily {
    pub visual_family_key: String,
    pub resource_owner_key: String,
    pub resource_generation: String,
    pub default_skin_img: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarItem {
    pub item_id: String,
    pub item_name: String,
    pub slot: String,
    #[serde(default)]
    pub profession_dir: String,
    pub playable_roles: Vec<String>,
    #[serde(default)]
    pub path_family: String,
    pub resource_kind: String,
    pub icon_img: String,
    #[serde(default)]
    pub layer_count: usize,
    pub layer_counts_by_visual_family: HashMap<String, usize>,
    #[serde(default)]
    pub hide_equipment_types: Vec<String>,
    #[serde(default)]
    pub hide_equipment_type_enums: Vec<u16>,
    #[serde(default)]
    pub enable_dye_mask1: bool,
    #[serde(default)]
    pub enable_dye_mask2: bool,
    pub layers: Vec<AvatarLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarSet {
    pub set_id: String,
    pub set_name: String,
    pub source: String,
    pub playable_roles: Vec<String>,
    pub member_ids: Vec<String>,
    pub member_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPage<T> {
    pub records: Vec<T>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogStats {
    pub item_count: usize,
    pub set_count: usize,
    pub layer_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogResponse {
    pub stats: CatalogStats,
    pub roles: Vec<AvatarRole>,
    pub visual_families: Vec<AvatarVisualFamily>,
    pub sets: Vec<AvatarSet>,
    pub items: Vec<AvatarItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMetadataResponse {
    pub stats: CatalogStats,
    pub roles: Vec<AvatarRole>,
    pub visual_families: Vec<AvatarVisualFamily>,
    pub item_result_limit: usize,
    pub set_result_limit: usize,
    pub schema_version: u32,
    pub build_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarSetDetail {
    pub set: AvatarSet,
    pub items: Vec<AvatarItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredAppConfig {
    #[serde(default)]
    pub game_dir: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub wardrobe_swap_unlock_proof: serde_json::Value,
}

impl Default for StoredAppConfig {
    fn default() -> Self {
        Self {
            game_dir: String::new(),
            wardrobe_swap_unlock_proof: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppConfigView {
    pub game_dir: String,
    pub wardrobe_swap_unlocked: bool,
    pub wardrobe_swap_credential_invalid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppConfigUpdate {
    #[serde(default)]
    pub game_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenFeatureUnlockResult {
    pub matched: bool,
    pub feature: Option<HiddenFeature>,
    pub already_unlocked: bool,
    pub config: AppConfigView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewLayerRequest {
    pub img_path: String,
    pub frame_index: usize,
    pub palette_index: usize,
    #[serde(default)]
    pub enable_dye_mask1: bool,
    #[serde(default)]
    pub enable_dye_mask2: bool,
}

impl PreviewLayerRequest {
    pub(crate) const fn enable_dye_mask(&self, number: u32) -> bool {
        enable_dye_mask(number, self.enable_dye_mask1, self.enable_dye_mask2)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSnapshot {
    #[serde(default)]
    pub playable_role_key: String,
    #[serde(default)]
    pub visual_family_key: String,
    #[serde(default)]
    pub resource_owner_key: String,
    #[serde(default)]
    pub resource_generation: String,
    pub profession_dir: String,
    pub path_family: String,
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WardrobeRole {
    Current,
    Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeSnapshotItemInput {
    pub slot: String,
    pub item_ref: Option<String>,
    pub item_name: Option<String>,
    pub layer_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeSnapshotInput {
    #[serde(default)]
    pub playable_role_key: String,
    #[serde(default)]
    pub visual_family_key: String,
    #[serde(default)]
    pub resource_owner_key: String,
    #[serde(default)]
    pub resource_generation: String,
    pub profession_dir: String,
    pub path_family: String,
    pub items: Vec<WardrobeSnapshotItemInput>,
    #[serde(default)]
    pub weapon: Option<WardrobeSnapshotItemInput>,
    pub preview_data_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeSnapshotItemView {
    pub slot: String,
    pub item_name: Option<String>,
    pub layer_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeSnapshotView {
    pub playable_role_key: String,
    pub visual_family_key: String,
    pub resource_owner_key: String,
    pub resource_generation: String,
    pub profession_dir: String,
    pub path_family: String,
    pub items: Vec<WardrobeSnapshotItemView>,
    pub weapon: Option<WardrobeSnapshotItemView>,
    pub preview_data_url: String,
    pub saved_at_ms: u64,
    pub revision: String,
    pub recipe_revision: Option<String>,
    pub has_dye: bool,
    pub dyed_slot_count: usize,
    pub dyed_base_count: usize,
    pub dyed_mask_count: usize,
    pub dyed_color_count: usize,
    pub representative_colors: Vec<String>,
    pub dye_status: String,
    pub resource_status: String,
    pub invalid_slots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeStateView {
    pub current: Option<WardrobeSnapshotView>,
    pub target: Option<WardrobeSnapshotView>,
    pub groups: Vec<WardrobeGroupView>,
    pub entries: Vec<WardrobeEntryView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeEntryDraft {
    pub name: String,
    pub note: String,
    pub group_id: Option<String>,
    pub favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeGroupView {
    pub id: String,
    pub name: String,
    pub sort_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeEntryView {
    pub id: String,
    pub name: String,
    pub note: String,
    pub group_id: Option<String>,
    pub favorite: bool,
    pub snapshot: WardrobeSnapshotView,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_used_at_ms: Option<u64>,
    pub is_current: bool,
    pub is_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardrobeShareImportResult {
    pub wardrobe: WardrobeStateView,
    pub entry_count: usize,
    pub group_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapExportResult {
    pub output_path: String,
    pub part_count: usize,
    pub entry_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapPartPlan {
    pub slot: String,
    pub action: String,
    pub current_item_name: String,
    pub target_item_name: String,
    pub carrier_img: String,
    pub target_layer_count: usize,
    pub output_frame_count: usize,
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub transparent_imgs: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub has_dye: bool,
    #[serde(default)]
    pub dyed_base_count: usize,
    #[serde(default)]
    pub dyed_mask_count: usize,
    #[serde(default)]
    pub dyed_color_count: usize,
    #[serde(default)]
    pub recipe_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapCarrierGroupPlan {
    pub carrier_slot: String,
    pub current_item_name: String,
    pub carrier_layer_order: i32,
    pub carrier_equipment_type_enum: u16,
    pub carrier_layer_sequence: u32,
    pub target_slots: Vec<String>,
    pub target_item_names: Vec<String>,
    pub target_layer_count: usize,
    pub output_frame_count: usize,
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub exact_order_match_count: usize,
    pub cross_slot_layer_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapPlan {
    pub profession_dir: String,
    pub path_family: String,
    pub strategy: String,
    pub carrier_groups: Vec<OutfitSwapCarrierGroupPlan>,
    pub parts: Vec<OutfitSwapPartPlan>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub dyed_base_count: usize,
    #[serde(default)]
    pub dyed_mask_count: usize,
    #[serde(default)]
    pub dyed_color_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapPageResult {
    pub plan: OutfitSwapPlan,
    pub current_preview: PreviewImage,
    pub target_preview: PreviewImage,
    #[serde(default)]
    pub current_preview_available: bool,
    #[serde(default)]
    pub target_preview_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapProgress {
    pub percent: u8,
    pub stage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfessionWideCarrierCandidate {
    pub slot: String,
    pub item_name: String,
    pub layer_order: i32,
    pub equipment_type_enum: u16,
    pub layer_sequence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfessionWideSwapPlan {
    pub status: String,
    pub strategy: String,
    pub profession_dir: String,
    pub path_family: String,
    pub avatar_root: String,
    pub carrier_slot: String,
    pub carrier_item_name: String,
    pub carrier_layer_order: i32,
    pub carrier_equipment_type_enum: u16,
    pub carrier_layer_sequence: u32,
    pub source_npk_count: usize,
    pub total_img_count: usize,
    pub suppressed_img_count: usize,
    pub total_frame_count: u64,
    pub estimated_output_bytes: u64,
    pub affected_scope: String,
    pub confirmation_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapAnalyzeRequest {
    pub game_dir: String,
    pub current: OutfitSnapshot,
    pub target: OutfitSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfitSwapExportRequest {
    pub game_dir: String,
    pub output_path: String,
    pub current: OutfitSnapshot,
    pub target: OutfitSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewImage {
    pub width: u32,
    pub height: u32,
    pub rgba_base64: String,
    pub missing_imgs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewAnimation {
    pub frame_count: usize,
    pub missing_imgs: Vec<String>,
    pub frames: Vec<PreviewImage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportNpkRequest {
    pub game_dir: String,
    pub output_path: String,
    pub entry_name: String,
    pub mode: String,
    pub layers: Vec<PreviewLayerRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportNpkResult {
    pub output_path: String,
    pub entry_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportWebpRequest {
    pub game_dir: String,
    pub output_path: String,
    #[serde(default = "default_webp_frame_duration_ms")]
    pub frame_duration_ms: u32,
    pub layers: Vec<PreviewLayerRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportWebpResult {
    pub output_path: String,
    pub width: u32,
    pub height: u32,
    pub frame_count: usize,
}

fn default_webp_frame_duration_ms() -> u32 {
    80
}
