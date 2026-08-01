use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::{c_char, CStr, CString};
use std::path::{Path, PathBuf};
use std::slice;
use std::sync::{Mutex, OnceLock};

use libloading::Library;
use serde::{Deserialize, Serialize};

use crate::avatar_dye::{
    build_patched_img_bytes, canonicalize_recipe, inspect_img_for_dye, select_equivalent_dye_layer,
    DyeCapabilityStatus, DyeRecipeV1, DyeTargetKindV1, DyeTargetRecipeV1,
};
use crate::dnflib_safe::{ImgArchive, NpkArchive};
use crate::game_resolver::{
    legacy_same_named_mask_number_for_ownership, normalize_img_path_for_npk, resolve_game_npk_path,
    resolve_img_resource, same_named_mask_number, ImgResourceMetadata,
};
use crate::models::{
    sort_avatar_layers_for_draw, AvatarItem, AvatarLayer, AvatarRole, AvatarSet, AvatarSetDetail,
    AvatarVisualFamily, CatalogMetadataResponse, CatalogPage, CatalogResponse, CatalogStats,
    OutfitSnapshot, WardrobeSnapshotInput,
};
pub(crate) use crate::wardrobe_limits::MAX_WARDROBE_CATALOG_CONTEXT_ITEM_IDS;

pub const DATA_DLL_NAME: &str = "avatar_data.dll";
const DATA_SCHEMA_VERSION: u32 = 7;
const ITEM_RESULT_LIMIT: usize = 800;
const SET_RESULT_LIMIT: usize = 500;
pub(crate) const CATALOG_PAGE_LIMIT: usize = 256;
const MAX_QUERY_CHARS: usize = 256;
const MAX_ITEM_IDS_PER_REQUEST: usize = 256;
const MAX_ITEM_ID_CHARS: usize = 128;
const MAX_SET_ID_CHARS: usize = 128;
const MAX_ROLE_CHARS: usize = 64;
const MAX_SLOT_CHARS: usize = 64;
const MAX_DATA_LIBRARY_RESPONSE_BYTES: usize = 256 * 1024 * 1024;
const MAX_WARDROBE_VALIDATION_NPK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
static DATA_LIBRARY: OnceLock<Library> = OnceLock::new();
static DATA_LIBRARY_CALL_LOCK: Mutex<()> = Mutex::new(());
static CATALOG_METADATA_CACHE: OnceLock<CatalogMetadataResponse> = OnceLock::new();
static CATALOG_METADATA_INIT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
struct SummaryJson {
    stats: CatalogStats,
    roles: Vec<AvatarRole>,
    visual_families: Vec<AvatarVisualFamily>,
    sets: Vec<AvatarSet>,
    items: Vec<SummaryItem>,
}

#[derive(Debug, Deserialize)]
struct SummaryItem {
    item_id: String,
    item_name: String,
    slot: String,
    #[serde(default)]
    profession_dir: String,
    playable_roles: Vec<String>,
    #[serde(default)]
    path_family: String,
    resource_kind: String,
    icon_img: String,
    #[serde(default)]
    layer_count: usize,
    layer_counts_by_visual_family: HashMap<String, usize>,
    #[serde(default)]
    hide_equipment_types: Vec<String>,
    #[serde(default)]
    hide_equipment_type_enums: Vec<u16>,
    enable_dye_mask1: bool,
    enable_dye_mask2: bool,
}

#[derive(Debug, Deserialize)]
struct LayerBucketJson {
    items: HashMap<String, Vec<AvatarLayer>>,
}

#[derive(Debug, Serialize)]
struct ItemQueryRequest<'a> {
    slot: &'a str,
    playable_role: &'a str,
    query: &'a str,
    limit: usize,
}

#[derive(Debug, Serialize)]
struct SetQueryRequest<'a> {
    playable_role: &'a str,
    query: &'a str,
    limit: usize,
}

#[derive(Debug, Serialize)]
struct ItemPageQueryRequest<'a> {
    slot: &'a str,
    playable_role: &'a str,
    query: &'a str,
    offset: usize,
    limit: usize,
}

#[derive(Debug, Serialize)]
struct SetPageQueryRequest<'a> {
    playable_role: &'a str,
    query: &'a str,
    offset: usize,
    limit: usize,
}

#[derive(Debug, Serialize)]
struct ItemIdsRequest<'a> {
    item_ids: &'a [String],
}

#[derive(Debug, Serialize)]
struct SetDetailRequest<'a> {
    set_id: &'a str,
    playable_role: &'a str,
}

fn validate_catalog_text(
    value: &str,
    label: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if !allow_empty && value.trim().is_empty() {
        return Err(format!("内置数据{label}不能为空"));
    }
    if value.chars().count() > max_chars {
        return Err(format!("内置数据{label}超过 {max_chars} 字符安全上限"));
    }
    Ok(())
}

pub(crate) fn validate_catalog_query_fields(
    slot: &str,
    playable_role: &str,
    query: &str,
) -> Result<(), String> {
    validate_catalog_text(slot, "部位", MAX_SLOT_CHARS, true)?;
    validate_catalog_text(playable_role, "职业", MAX_ROLE_CHARS, true)?;
    validate_catalog_text(query, "搜索词", MAX_QUERY_CHARS, true)
}

pub(crate) fn validate_catalog_page_fields(
    slot: &str,
    playable_role: &str,
    query: &str,
    _offset: usize,
    limit: usize,
) -> Result<(), String> {
    validate_catalog_query_fields(slot, playable_role, query)?;
    if !(1..=CATALOG_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "内置数据分页条数必须介于 1 到 {CATALOG_PAGE_LIMIT} 之间"
        ));
    }
    Ok(())
}

pub(crate) fn validate_catalog_set_detail_fields(
    set_id: &str,
    playable_role: &str,
) -> Result<(), String> {
    validate_catalog_text(set_id, "套装 ID", MAX_SET_ID_CHARS, false)?;
    validate_catalog_text(playable_role, "职业", MAX_ROLE_CHARS, true)
}

fn normalized_catalog_item_ids_with_limit(
    item_ids: &[String],
    max_items: usize,
) -> Result<Vec<String>, String> {
    if item_ids.len() > max_items {
        return Err(format!("内置数据物品 ID 超过 {max_items} 条安全上限"));
    }
    normalized_catalog_context_item_ids(item_ids)
}

pub(crate) fn normalized_catalog_context_item_ids(
    item_ids: &[String],
) -> Result<Vec<String>, String> {
    if item_ids.len() > MAX_WARDROBE_CATALOG_CONTEXT_ITEM_IDS {
        return Err(format!(
            "橱窗内置数据物品 ID 超过 {MAX_WARDROBE_CATALOG_CONTEXT_ITEM_IDS} 条安全上限"
        ));
    }
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(item_ids.len());
    for item_id in item_ids {
        let item_id = item_id.trim();
        validate_catalog_text(item_id, "物品 ID", MAX_ITEM_ID_CHARS, false)?;
        if seen.insert(item_id.to_string()) {
            normalized.push(item_id.to_string());
        }
    }
    Ok(normalized)
}

pub(crate) fn normalized_catalog_item_ids(item_ids: &[String]) -> Result<Vec<String>, String> {
    normalized_catalog_item_ids_with_limit(item_ids, MAX_ITEM_IDS_PER_REQUEST)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WardrobeDyeValidationStatus {
    NoDye,
    Unchecked,
    Valid,
    Changed,
    Partial,
    Unsupported,
}

impl WardrobeDyeValidationStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoDye => "none",
            Self::Unchecked => "unchecked",
            Self::Valid => "valid",
            Self::Changed => "changed",
            Self::Partial => "partial",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WardrobeDyeValidation {
    pub(crate) status: WardrobeDyeValidationStatus,
    pub(crate) invalid_slots: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WardrobeDyeTargetValidation {
    Valid,
    Changed,
    Unsupported,
}

const DNF_DYE_INDEXED_FORMAT: i32 = 0x0e;
const DNF_DYE_LINK_FORMAT: i32 = 0x11;
const DNF_DYE_UNCOMPRESSED: i32 = 0x05;
const DNF_DYE_ZLIB: i32 = 0x06;

impl WardrobeDyeValidation {
    fn no_dye() -> Self {
        Self {
            status: WardrobeDyeValidationStatus::NoDye,
            invalid_slots: Vec::new(),
        }
    }

    fn unchecked() -> Self {
        Self {
            status: WardrobeDyeValidationStatus::Unchecked,
            invalid_slots: Vec::new(),
        }
    }
}

pub(crate) struct WardrobeDyeValidator {
    game_dir: Option<PathBuf>,
    archives: HashMap<PathBuf, NpkArchive>,
    unreadable_archives: HashSet<PathBuf>,
    opened_npk_bytes: u64,
    layer_cache: HashMap<Vec<String>, Option<Vec<AvatarLayer>>>,
}

impl WardrobeDyeValidator {
    pub(crate) fn new(configured_game_dir: &Path) -> Self {
        let configured = if configured_game_dir.as_os_str().is_empty() {
            None
        } else if configured_game_dir.join("ImagePacks2").is_dir() {
            Some(configured_game_dir.join("ImagePacks2"))
        } else if configured_game_dir.is_dir() {
            Some(configured_game_dir.to_path_buf())
        } else {
            None
        };
        Self {
            game_dir: configured,
            archives: HashMap::new(),
            unreadable_archives: HashSet::new(),
            opened_npk_bytes: 0,
            layer_cache: HashMap::new(),
        }
    }

    pub(crate) fn validate_from_data_library(
        &mut self,
        snapshot: &WardrobeSnapshotInput,
        recipe: Option<&DyeRecipeV1>,
    ) -> WardrobeDyeValidation {
        if recipe.is_none_or(DyeRecipeV1::is_empty) {
            return WardrobeDyeValidation::no_dye();
        }
        if self.game_dir.is_none() {
            return WardrobeDyeValidation::unchecked();
        }
        let mut item_ids = snapshot
            .items
            .iter()
            .filter_map(|item| item.item_ref.clone())
            .collect::<Vec<_>>();
        item_ids.sort();
        item_ids.dedup();
        if !self.layer_cache.contains_key(&item_ids) {
            let loaded = load_raw_layers_from_data_library(&item_ids)
                .ok()
                .map(deduplicate_avatar_layers);
            self.layer_cache.insert(item_ids.clone(), loaded);
        }
        let Some(raw_layers) = self.layer_cache.get(&item_ids).cloned().flatten() else {
            return WardrobeDyeValidation::unchecked();
        };
        let Ok(catalog) = catalog_context_for_wardrobe_snapshot(snapshot) else {
            return WardrobeDyeValidation::unchecked();
        };
        let Ok(family) = ensure_snapshot_role_binding(snapshot, &catalog) else {
            return WardrobeDyeValidation::unchecked();
        };
        let raw_layers = filter_layers_for_visual_family(raw_layers, &family.visual_family_key);
        let Ok(layers) = with_default_avatar_skin_for_family(family, &raw_layers) else {
            return WardrobeDyeValidation::unchecked();
        };
        self.validate_with_layers(snapshot, recipe, &layers)
    }

    pub(crate) fn ensure_from_data_library(
        &mut self,
        snapshot: &WardrobeSnapshotInput,
        recipe: Option<&DyeRecipeV1>,
    ) -> Result<(), String> {
        ensure_live_dye_validation(self.validate_from_data_library(snapshot, recipe))
    }

    pub(crate) fn validate_with_layers(
        &mut self,
        snapshot: &WardrobeSnapshotInput,
        recipe: Option<&DyeRecipeV1>,
        layers: &[AvatarLayer],
    ) -> WardrobeDyeValidation {
        let Some(recipe) = recipe.filter(|recipe| !recipe.is_empty()) else {
            return WardrobeDyeValidation::no_dye();
        };
        if self.game_dir.is_none() {
            return WardrobeDyeValidation::unchecked();
        }
        let canonical = match canonicalize_recipe(recipe.clone()) {
            Ok(canonical) if canonical == *recipe && !canonical.is_empty() => canonical,
            _ => return changed_recipe_validation(snapshot, recipe),
        };
        let mut valid_target_count = 0usize;
        let mut has_unsupported_target = false;
        let mut invalid_slots = BTreeSet::new();
        for target in &canonical.targets {
            match self.validate_target(snapshot, layers, target) {
                WardrobeDyeTargetValidation::Valid => valid_target_count += 1,
                WardrobeDyeTargetValidation::Changed => {
                    if let Some(slot) = public_snapshot_slot(snapshot, &target.target.slot) {
                        invalid_slots.insert(slot);
                    }
                }
                WardrobeDyeTargetValidation::Unsupported => {
                    has_unsupported_target = true;
                    if let Some(slot) = public_snapshot_slot(snapshot, &target.target.slot) {
                        invalid_slots.insert(slot);
                    }
                }
            }
        }
        let status = if has_unsupported_target {
            WardrobeDyeValidationStatus::Unsupported
        } else if valid_target_count == canonical.targets.len() {
            WardrobeDyeValidationStatus::Valid
        } else if valid_target_count == 0 {
            WardrobeDyeValidationStatus::Changed
        } else {
            WardrobeDyeValidationStatus::Partial
        };
        WardrobeDyeValidation {
            status,
            invalid_slots: invalid_slots.into_iter().collect(),
        }
    }

    fn validate_target(
        &mut self,
        snapshot: &WardrobeSnapshotInput,
        layers: &[AvatarLayer],
        target: &DyeTargetRecipeV1,
    ) -> WardrobeDyeTargetValidation {
        let matching_layers = layers
            .iter()
            .filter(|layer| {
                target
                    .target
                    .slot
                    .trim()
                    .eq_ignore_ascii_case(layer.slot.trim())
                    && target.target.item_ref.trim() == layer.item_id.trim()
                    && target.target.layer_tag.trim() == layer.layer_tag.trim()
                    && wardrobe_snapshot_owns_layer(snapshot, layer)
            })
            .collect::<Vec<_>>();
        let Some(layer) = select_equivalent_dye_layer(&matching_layers) else {
            return WardrobeDyeTargetValidation::Changed;
        };
        if let DyeTargetKindV1::Mask { number } = target.target.target_kind {
            if !layer.enable_dye_mask(number) {
                return WardrobeDyeTargetValidation::Changed;
            }
        }
        let Some((bytes, resolved_palette_index)) = self.target_entry_bytes(layer, target) else {
            return WardrobeDyeTargetValidation::Changed;
        };
        let version = match ImgArchive::open_owned_bytes(bytes.clone()) {
            Ok(archive) => archive.version(),
            Err(_) => return WardrobeDyeTargetValidation::Changed,
        };
        let expected_palette = match (&target.target.target_kind, version) {
            (DyeTargetKindV1::Mask { .. }, _) => 0,
            (DyeTargetKindV1::Base, _) => resolved_palette_index,
        };
        if usize::from(target.palette_index) != expected_palette {
            return WardrobeDyeTargetValidation::Changed;
        }
        let carrier_supported = match dye_carrier_supported(&bytes) {
            Ok(supported) => supported,
            Err(_) => return WardrobeDyeTargetValidation::Changed,
        };
        if !carrier_supported {
            return WardrobeDyeTargetValidation::Unsupported;
        }
        if build_patched_img_bytes(&bytes, target).is_ok() {
            WardrobeDyeTargetValidation::Valid
        } else {
            WardrobeDyeTargetValidation::Changed
        }
    }

    fn target_entry_bytes(
        &mut self,
        layer: &AvatarLayer,
        target: &DyeTargetRecipeV1,
    ) -> Option<(Vec<u8>, usize)> {
        let game_dir = self.game_dir.clone()?;
        let resolved = resolve_img_resource(
            &layer.visual_img,
            layer.palette_index,
            |base_entry| -> Result<_, ()> {
                let Some(npk_path) = resolve_game_npk_path(&game_dir, base_entry) else {
                    return Ok(None);
                };
                if !npk_path.is_file() || self.unreadable_archives.contains(&npk_path) {
                    return Ok(None);
                }
                if !self.archives.contains_key(&npk_path) {
                    let Some(size) = std::fs::metadata(&npk_path)
                        .ok()
                        .map(|metadata| metadata.len())
                    else {
                        self.unreadable_archives.insert(npk_path);
                        return Ok(None);
                    };
                    let Some(total) = self.opened_npk_bytes.checked_add(size) else {
                        self.unreadable_archives.insert(npk_path);
                        return Ok(None);
                    };
                    if total > MAX_WARDROBE_VALIDATION_NPK_BYTES {
                        self.unreadable_archives.insert(npk_path);
                        return Ok(None);
                    }
                    match NpkArchive::open_path(&npk_path) {
                        Ok(archive) => {
                            self.opened_npk_bytes = total;
                            self.archives.insert(npk_path.clone(), archive);
                        }
                        Err(_) => {
                            self.unreadable_archives.insert(npk_path);
                            return Ok(None);
                        }
                    }
                }
                let Some(archive) = self.archives.get(&npk_path) else {
                    return Ok(None);
                };
                let Some(base_index) = archive.find_entry_index(base_entry) else {
                    return Ok(None);
                };
                let metadata = archive
                    .entry_bytes(base_index)
                    .ok()
                    .and_then(|bytes| ImgArchive::open_owned_bytes(bytes.to_vec()).ok())
                    .map(|img| ImgResourceMetadata {
                        version: img.version(),
                        palette_count: img.palette_count(),
                    })
                    .unwrap_or(ImgResourceMetadata {
                        version: 0,
                        palette_count: 0,
                    });
                Ok(Some((npk_path, metadata)))
            },
        )
        .ok()??;
        let base_entry = resolved.entry_path;
        let archive = self.archives.get(&resolved.value)?;
        let matching_entries = archive
            .entries()
            .into_iter()
            .filter(|entry| match target.target.target_kind {
                DyeTargetKindV1::Base => entry.name.eq_ignore_ascii_case(&base_entry),
                DyeTargetKindV1::Mask { number } => {
                    same_named_mask_number(&base_entry, &entry.name) == Some(number)
                }
            })
            .collect::<Vec<_>>();
        let [entry] = matching_entries.as_slice() else {
            return None;
        };
        if normalize_img_path_for_npk(&entry.name)
            != normalize_img_path_for_npk(&target.target.private_entry_ref)
        {
            return None;
        }
        archive
            .entry_bytes(entry.index)
            .ok()
            .map(|bytes| (bytes.to_vec(), resolved.palette_index))
    }
}

fn dye_carrier_supported(bytes: &[u8]) -> Result<bool, String> {
    let archive = ImgArchive::open_owned_bytes(bytes.to_vec())?;
    if archive.version() == 2 {
        let inspection = inspect_img_for_dye(bytes, 0)?;
        return Ok(
            inspection.palette_count == 1 && inspection.capability != DyeCapabilityStatus::ReadOnly
        );
    }
    if !matches!(archive.version(), 4 | 6) {
        return Ok(false);
    }
    let frames = archive.frames();
    if frames.is_empty() {
        return Err("染色资源 IMG 没有帧".to_string());
    }
    let mut has_indexed_frame = false;
    for index in 0..frames.len() {
        let mut current = index;
        let mut effective = None;
        for _ in 0..=frames.len() {
            let frame = frames
                .get(current)
                .ok_or_else(|| "染色资源 LINK 帧目标无效".to_string())?;
            if frame.format != DNF_DYE_LINK_FORMAT {
                effective = Some(frame);
                break;
            }
            current = usize::try_from(frame.link_index)
                .map_err(|_| "染色资源 LINK 帧目标无效".to_string())?;
        }
        let effective = effective.ok_or_else(|| "染色资源 LINK 帧存在循环引用".to_string())?;
        if effective.format != DNF_DYE_INDEXED_FORMAT {
            continue;
        }
        if !matches!(effective.compress, DNF_DYE_UNCOMPRESSED | DNF_DYE_ZLIB) {
            return Err("染色资源索引帧压缩格式不受支持".to_string());
        }
        has_indexed_frame = true;
    }
    Ok(has_indexed_frame)
}

fn ensure_live_dye_validation(validation: WardrobeDyeValidation) -> Result<(), String> {
    match validation.status {
        WardrobeDyeValidationStatus::NoDye | WardrobeDyeValidationStatus::Valid => Ok(()),
        WardrobeDyeValidationStatus::Unchecked => {
            Err("当前无法校验染色配方，禁止指定为当前穿戴或目标外观".to_string())
        }
        WardrobeDyeValidationStatus::Unsupported => {
            if validation.invalid_slots.is_empty() {
                Err("染色配方的资源或承载条件不支持，禁止指定或生成".to_string())
            } else {
                let labels = validation
                    .invalid_slots
                    .iter()
                    .map(|slot| public_slot_label(slot))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join("、");
                Err(format!(
                    "染色配方的资源或承载条件不支持，禁止指定或生成；失效部位：{}",
                    labels
                ))
            }
        }
        WardrobeDyeValidationStatus::Changed | WardrobeDyeValidationStatus::Partial => {
            if validation.invalid_slots.is_empty() {
                Err("染色配方源资源已变化，禁止指定为当前穿戴或目标外观".to_string())
            } else {
                let labels = validation
                    .invalid_slots
                    .iter()
                    .map(|slot| public_slot_label(slot))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join("、");
                Err(format!(
                    "染色配方源资源已变化，禁止指定；失效部位：{}",
                    labels
                ))
            }
        }
    }
}

fn changed_recipe_validation(
    snapshot: &WardrobeSnapshotInput,
    recipe: &DyeRecipeV1,
) -> WardrobeDyeValidation {
    let invalid_slots = recipe
        .targets
        .iter()
        .filter_map(|target| public_snapshot_slot(snapshot, &target.target.slot))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    WardrobeDyeValidation {
        status: WardrobeDyeValidationStatus::Changed,
        invalid_slots,
    }
}

fn public_snapshot_slot(snapshot: &WardrobeSnapshotInput, slot: &str) -> Option<String> {
    snapshot
        .items
        .iter()
        .find(|item| item.slot.trim().eq_ignore_ascii_case(slot.trim()))
        .map(|item| item.slot.trim().to_ascii_lowercase())
}

fn public_slot_label(slot: &str) -> &'static str {
    match slot.trim().to_ascii_lowercase().as_str() {
        "skin" => "皮肤",
        "cap" => "帽子",
        "hair" => "头部",
        "face" => "脸部",
        "neck" => "胸部",
        "coat" => "上衣",
        "belt" => "腰带",
        "pants" => "裤子",
        "shoes" => "鞋子",
        _ => "时装部位",
    }
}

pub fn catalog_from_summary_bytes(bytes: &[u8]) -> Result<CatalogResponse, String> {
    let summary: SummaryJson =
        serde_json::from_slice(bytes).map_err(|err| format!("解析内置时装摘要失败：{err}"))?;
    validate_catalog_role_family_model(&summary)?;
    Ok(CatalogResponse {
        stats: summary.stats,
        roles: summary.roles,
        visual_families: summary.visual_families,
        sets: summary.sets,
        items: summary
            .items
            .into_iter()
            .map(|item| AvatarItem {
                item_id: item.item_id,
                item_name: item.item_name,
                slot: item.slot,
                profession_dir: item.profession_dir,
                playable_roles: item.playable_roles,
                path_family: item.path_family,
                resource_kind: item.resource_kind,
                icon_img: item.icon_img,
                layer_count: item.layer_count,
                layer_counts_by_visual_family: item.layer_counts_by_visual_family,
                hide_equipment_types: item.hide_equipment_types,
                hide_equipment_type_enums: item.hide_equipment_type_enums,
                enable_dye_mask1: item.enable_dye_mask1,
                enable_dye_mask2: item.enable_dye_mask2,
                layers: Vec::new(),
            })
            .collect(),
    })
}

fn validate_catalog_role_family_model(summary: &SummaryJson) -> Result<(), String> {
    if summary.roles.is_empty() || summary.visual_families.is_empty() {
        return Err("内置数据缺少玩法职业或视觉族映射".to_string());
    }
    let mut family_keys = HashSet::new();
    let mut default_skin_owners = HashMap::<String, String>::new();
    for family in &summary.visual_families {
        let key = family.visual_family_key.trim().to_ascii_lowercase();
        let generation = family.resource_generation.trim().to_ascii_lowercase();
        if key.is_empty()
            || !family_keys.insert(key.clone())
            || family.resource_owner_key.trim().is_empty()
            || !matches!(generation.as_str(), "base" | "at")
            || family.default_skin_img.trim().is_empty()
        {
            return Err("内置数据包含空、重复或无效的视觉族定义".to_string());
        }
        let default_skin = normalize_img_path_for_npk(&family.default_skin_img);
        if let Some(previous) = default_skin_owners.insert(default_skin.clone(), key.clone()) {
            if previous != key {
                return Err(format!(
                    "default skin IMG {default_skin} is shared across visual families {previous} and {key}"
                ));
            }
        }
    }
    let mut role_keys = HashSet::new();
    let mut role_families = HashMap::new();
    for role in &summary.roles {
        let role_key = role.playable_role_key.trim().to_ascii_lowercase();
        let family_key = role.visual_family_key.trim().to_ascii_lowercase();
        if role_key.is_empty()
            || role_key == "all"
            || !role_keys.insert(role_key.clone())
            || !family_keys.contains(&family_key)
        {
            return Err("内置数据包含空、重复或悬空的玩法职业映射".to_string());
        }
        role_families.insert(role_key, family_key);
    }
    for item in &summary.items {
        let item_id = item.item_id.trim();
        let resource_kind = item.resource_kind.trim().to_ascii_lowercase();
        if !matches!(
            resource_kind.as_str(),
            "avatar" | "weapon_avatar" | "weapon"
        ) {
            return Err(format!(
                "catalog item {item_id} has invalid resource kind {:?}",
                item.resource_kind
            ));
        }
        if item.playable_roles.is_empty() {
            return Err(format!(
                "catalog item {item_id} has no playable role binding"
            ));
        }
        let mut item_family_keys = HashSet::new();
        for role in &item.playable_roles {
            let normalized = role.trim().to_ascii_lowercase();
            let Some(family_key) = role_families.get(&normalized) else {
                return Err(format!(
                    "catalog item {item_id} references unbound playable role {role}"
                ));
            };
            item_family_keys.insert(family_key.as_str());
        }
        if item.layer_count > 0 && item.layer_counts_by_visual_family.is_empty() {
            return Err(format!(
                "catalog item {item_id} has layers but no visual-family layer counts"
            ));
        }
        for (family_key, count) in &item.layer_counts_by_visual_family {
            let normalized = family_key.trim().to_ascii_lowercase();
            if !family_keys.contains(&normalized) {
                return Err(format!(
                    "catalog item {item_id} references visual family {family_key}"
                ));
            }
            if !item_family_keys.contains(normalized.as_str()) {
                return Err(format!(
                    "catalog item {item_id} has a layer count for unrelated visual family {family_key}"
                ));
            }
            if *count == 0 {
                return Err(format!(
                    "catalog item {item_id} has a zero layer count for visual family {family_key}"
                ));
            }
        }
    }
    for set in &summary.sets {
        let set_id = set.set_id.trim();
        if set.playable_roles.is_empty() {
            return Err(format!("catalog set {set_id} has no playable role binding"));
        }
        for role in &set.playable_roles {
            if !role_keys.contains(&role.trim().to_ascii_lowercase()) {
                return Err(format!(
                    "catalog set {set_id} references unbound playable role {role}"
                ));
            }
        }
    }
    Ok(())
}

pub fn layers_from_bucket_bytes(
    bytes: &[u8],
    item_ids: &[String],
) -> Result<Vec<AvatarLayer>, String> {
    let mut layers = deduplicate_avatar_layers(raw_layers_from_bucket_bytes(bytes, item_ids)?);
    sort_avatar_layers_for_draw(&mut layers);
    Ok(layers)
}

pub fn layers_from_bucket_bytes_for_visual_family(
    bytes: &[u8],
    item_ids: &[String],
    visual_family_key: &str,
) -> Result<Vec<AvatarLayer>, String> {
    let raw_layers = raw_layers_from_bucket_bytes(bytes, item_ids)?;
    let mut layers = deduplicate_avatar_layers(filter_layers_for_visual_family(
        raw_layers,
        visual_family_key,
    ));
    sort_avatar_layers_for_draw(&mut layers);
    Ok(layers)
}

pub fn raw_layers_from_bucket_bytes(
    bytes: &[u8],
    item_ids: &[String],
) -> Result<Vec<AvatarLayer>, String> {
    let wanted = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
    let bucket: LayerBucketJson =
        serde_json::from_slice(bytes).map_err(|err| format!("解析内置时装图层失败：{err}"))?;
    let mut layers = Vec::new();
    let mut selected = HashSet::new();
    for item_id in item_ids {
        if !wanted.contains(item_id.as_str()) || !selected.insert(item_id.as_str()) {
            continue;
        }
        if let Some(item_layers) = bucket.items.get(item_id) {
            layers.extend(item_layers.iter().cloned());
        }
    }
    Ok(layers)
}

pub(crate) fn layers_in_item_id_order(
    layers: Vec<AvatarLayer>,
    item_ids: &[String],
) -> Vec<AvatarLayer> {
    let mut by_item = HashMap::<String, Vec<AvatarLayer>>::new();
    for layer in layers {
        by_item
            .entry(layer.item_id.clone())
            .or_default()
            .push(layer);
    }
    let mut ordered = Vec::new();
    let mut selected = HashSet::new();
    for item_id in item_ids {
        if selected.insert(item_id.as_str()) {
            if let Some(item_layers) = by_item.remove(item_id) {
                ordered.extend(item_layers);
            }
        }
    }
    ordered
}

fn deduplicate_avatar_layers(mut layers: Vec<AvatarLayer>) -> Vec<AvatarLayer> {
    let mut deduplicated = Vec::<AvatarLayer>::new();
    let mut indices =
        HashMap::<(String, String, String, i32, u16, u32, String, String, usize), Vec<usize>>::new(
        );
    for layer in layers.drain(..) {
        let key = (
            layer.item_id.clone(),
            layer.slot.to_ascii_lowercase(),
            layer.visual_family_key.to_ascii_lowercase(),
            layer.layer_order,
            layer.equipment_type_enum,
            layer.layer_sequence,
            layer.layer_tag.clone(),
            layer.variation.clone(),
            layer.palette_index,
        );
        let normalized_visual = layer
            .visual_img
            .trim()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let candidates = indices.entry(key).or_default();
        if normalized_visual.is_empty() {
            if candidates.is_empty() {
                candidates.push(deduplicated.len());
                deduplicated.push(layer);
            }
            continue;
        }
        if candidates.iter().any(|&index| {
            deduplicated[index]
                .visual_img
                .trim()
                .replace('\\', "/")
                .eq_ignore_ascii_case(&normalized_visual)
        }) {
            continue;
        }
        if let Some(index) = candidates
            .iter()
            .copied()
            .find(|&index| deduplicated[index].visual_img.trim().is_empty())
        {
            deduplicated[index] = layer;
            continue;
        }
        candidates.push(deduplicated.len());
        deduplicated.push(layer);
    }
    deduplicated
}

fn filter_layers_for_visual_family(
    layers: Vec<AvatarLayer>,
    visual_family_key: &str,
) -> Vec<AvatarLayer> {
    let wanted = visual_family_key.trim().to_ascii_lowercase();
    if wanted.is_empty() || wanted == "all" {
        return Vec::new();
    }
    layers
        .into_iter()
        .filter(|layer| layer.visual_family_key.trim().eq_ignore_ascii_case(&wanted))
        .collect()
}

pub(crate) fn snapshot_playable_role_key(snapshot: &WardrobeSnapshotInput) -> String {
    snapshot.playable_role_key.trim().to_ascii_lowercase()
}

pub(crate) fn visual_family_for_playable_role<'a>(
    catalog: &'a CatalogResponse,
    playable_role_key: &str,
) -> Result<&'a AvatarVisualFamily, String> {
    let wanted = playable_role_key.trim();
    if wanted.is_empty() || wanted.eq_ignore_ascii_case("all") {
        return Err("必须选择明确的玩法职业".to_string());
    }
    let binding = catalog
        .roles
        .iter()
        .find(|role| role.playable_role_key.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| format!("玩法职业 {wanted} 没有显式 visual_family 映射"))?;
    catalog
        .visual_families
        .iter()
        .find(|family| {
            family
                .visual_family_key
                .eq_ignore_ascii_case(&binding.visual_family_key)
        })
        .ok_or_else(|| {
            format!(
                "玩法职业 {wanted} 引用了不存在的视觉族 {}",
                binding.visual_family_key
            )
        })
}

pub(crate) fn ensure_role_visual_family_binding<'a>(
    catalog: &'a CatalogResponse,
    playable_role_key: &str,
    visual_family_key: &str,
) -> Result<&'a AvatarVisualFamily, String> {
    let expected = visual_family_for_playable_role(catalog, playable_role_key)?;
    let declared = visual_family_key.trim();
    if declared.is_empty() || !declared.eq_ignore_ascii_case(&expected.visual_family_key) {
        return Err(format!(
            "玩法职业 {} 的视觉族声明无效：期望 {}，实际 {}",
            playable_role_key, expected.visual_family_key, declared
        ));
    }
    Ok(expected)
}

pub(crate) fn ensure_snapshot_role_binding<'a>(
    snapshot: &WardrobeSnapshotInput,
    catalog: &'a CatalogResponse,
) -> Result<&'a AvatarVisualFamily, String> {
    let family = ensure_role_visual_family_binding(
        catalog,
        &snapshot.playable_role_key,
        &snapshot.visual_family_key,
    )?;
    if !snapshot
        .resource_owner_key
        .eq_ignore_ascii_case(&family.resource_owner_key)
        || !snapshot
            .resource_generation
            .eq_ignore_ascii_case(&family.resource_generation)
    {
        return Err("快照资源所有者或 base/at 代际与视觉族不一致".to_string());
    }
    Ok(family)
}

pub fn ensure_outfit_snapshot_role_binding_from_data_library(
    snapshot: &OutfitSnapshot,
) -> Result<(), String> {
    let catalog = catalog_context_from_data_library(Vec::new())?;
    let family = ensure_role_visual_family_binding(
        &catalog,
        &snapshot.playable_role_key,
        &snapshot.visual_family_key,
    )?;
    if !snapshot
        .resource_owner_key
        .eq_ignore_ascii_case(&family.resource_owner_key)
        || !snapshot
            .resource_generation
            .eq_ignore_ascii_case(&family.resource_generation)
    {
        return Err("换装快照资源所有者或 base/at 代际与视觉族不一致".to_string());
    }
    Ok(())
}

fn catalog_item_matches_playable_role(item: &AvatarItem, playable_role_key: &str) -> bool {
    let wanted = playable_role_key.trim().to_ascii_lowercase();
    if wanted.is_empty() || wanted == "all" {
        return false;
    }
    item.playable_roles
        .iter()
        .any(|key| key.eq_ignore_ascii_case(&wanted))
}

fn catalog_item_layer_count_for_playable_role(
    item: &AvatarItem,
    playable_role_key: &str,
    catalog: &CatalogResponse,
) -> usize {
    let Ok(family) = visual_family_for_playable_role(catalog, playable_role_key) else {
        return 0;
    };
    if item.layer_counts_by_visual_family.is_empty() {
        return item.layer_count;
    }
    item.layer_counts_by_visual_family
        .iter()
        .find_map(|(key, count)| {
            key.eq_ignore_ascii_case(&family.visual_family_key)
                .then_some(*count)
        })
        .unwrap_or(0)
}

pub fn filter_catalog_for_display(
    mut catalog: CatalogResponse,
    visual_item_ids: &HashSet<String>,
) -> CatalogResponse {
    catalog.items = catalog
        .items
        .into_iter()
        .filter_map(|mut item| {
            item.item_name = sanitize_display_name(&item.item_name)?;
            Some(item)
        })
        .collect();
    catalog.sets = catalog
        .sets
        .into_iter()
        .filter_map(|mut set| {
            set.set_name = sanitize_display_name(&set.set_name)?;
            if is_lucky_bag_set_name(&set.set_name)
                || !set.member_ids.iter().any(|id| visual_item_ids.contains(id))
            {
                return None;
            }
            set.member_names = set
                .member_names
                .into_iter()
                .filter_map(|name| sanitize_display_name(&name))
                .collect();
            Some(set)
        })
        .collect();
    catalog.stats.item_count = catalog.items.len();
    catalog.stats.set_count = catalog.sets.len();
    catalog
}

pub fn open_catalog_index_from_data_library() -> Result<CatalogMetadataResponse, String> {
    Ok(cached_catalog_metadata_from_data_library()?.clone())
}

fn cached_catalog_metadata_from_data_library() -> Result<&'static CatalogMetadataResponse, String> {
    if let Some(metadata) = CATALOG_METADATA_CACHE.get() {
        return Ok(metadata);
    }
    let _guard = CATALOG_METADATA_INIT_LOCK
        .lock()
        .map_err(|_| "内置时装元数据初始化锁已损坏".to_string())?;
    if let Some(metadata) = CATALOG_METADATA_CACHE.get() {
        return Ok(metadata);
    }
    let metadata =
        call_data_library_without_request::<CatalogMetadataResponse>(b"dnf_avatar_data_open\0")?;
    if metadata.schema_version != DATA_SCHEMA_VERSION
        || metadata.item_result_limit != ITEM_RESULT_LIMIT
        || metadata.set_result_limit != SET_RESULT_LIMIT
    {
        return Err("内置时装元数据契约与程序不兼容".to_string());
    }
    let _ = CATALOG_METADATA_CACHE.set(metadata);
    CATALOG_METADATA_CACHE
        .get()
        .ok_or_else(|| "缓存内置时装元数据失败".to_string())
}

fn catalog_context_from_data_library(items: Vec<AvatarItem>) -> Result<CatalogResponse, String> {
    let metadata = cached_catalog_metadata_from_data_library()?;
    Ok(CatalogResponse {
        stats: metadata.stats.clone(),
        roles: metadata.roles.clone(),
        visual_families: metadata.visual_families.clone(),
        sets: Vec::new(),
        items,
    })
}

pub(crate) fn collect_catalog_context_items<T>(
    item_ids: &[String],
    mut fetch: impl FnMut(&[String]) -> Result<Vec<T>, String>,
) -> Result<Vec<T>, String> {
    let item_ids = normalized_catalog_context_item_ids(item_ids)?;
    let mut items = Vec::new();
    for chunk in item_ids.chunks(MAX_ITEM_IDS_PER_REQUEST) {
        items.extend(fetch(chunk)?);
    }
    Ok(items)
}

pub(crate) fn catalog_context_for_item_ids(item_ids: &[String]) -> Result<CatalogResponse, String> {
    // The aggregate wardrobe context follows the wardrobe's own capacity contract. Each native
    // request remains independently bounded so a large valid local wardrobe cannot amplify one
    // FFI call or conflict with MAX_ITEM_IDS_PER_REQUEST.
    let items =
        collect_catalog_context_items(item_ids, find_avatar_items_by_ids_from_data_library)?;
    catalog_context_from_data_library(items)
}

pub(crate) fn wardrobe_snapshot_item_ids(snapshot: &WardrobeSnapshotInput) -> Vec<String> {
    let mut item_ids = snapshot
        .items
        .iter()
        .filter_map(|saved| saved.item_ref.clone())
        .collect::<Vec<_>>();
    if let Some(item_ref) = snapshot
        .weapon
        .as_ref()
        .and_then(|saved| saved.item_ref.clone())
    {
        item_ids.push(item_ref);
    }
    item_ids.retain(|item_id| !item_id.trim().is_empty());
    item_ids.sort();
    item_ids.dedup();
    item_ids
}

fn catalog_context_for_wardrobe_snapshot(
    snapshot: &WardrobeSnapshotInput,
) -> Result<CatalogResponse, String> {
    let item_ids = wardrobe_snapshot_item_ids(snapshot);
    catalog_context_for_item_ids(&item_ids)
}

pub fn query_avatar_items_from_data_library(
    slot: &str,
    playable_role: &str,
    query: &str,
) -> Result<Vec<AvatarItem>, String> {
    validate_catalog_query_fields(slot, playable_role, query)?;
    call_data_library_with_request(
        b"dnf_avatar_data_query_items\0",
        &ItemQueryRequest {
            slot,
            playable_role,
            query,
            limit: ITEM_RESULT_LIMIT,
        },
    )
}

pub fn query_avatar_sets_from_data_library(
    playable_role: &str,
    query: &str,
) -> Result<Vec<AvatarSet>, String> {
    validate_catalog_query_fields("", playable_role, query)?;
    call_data_library_with_request(
        b"dnf_avatar_data_query_sets\0",
        &SetQueryRequest {
            playable_role,
            query,
            limit: SET_RESULT_LIMIT,
        },
    )
}

pub fn query_avatar_items_page_from_data_library(
    slot: &str,
    playable_role: &str,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<CatalogPage<AvatarItem>, String> {
    validate_catalog_page_fields(slot, playable_role, query, offset, limit)?;
    call_data_library_with_request(
        b"dnf_avatar_data_query_items_page\0",
        &ItemPageQueryRequest {
            slot,
            playable_role,
            query,
            offset,
            limit,
        },
    )
}

pub fn query_avatar_sets_page_from_data_library(
    playable_role: &str,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<CatalogPage<AvatarSet>, String> {
    validate_catalog_page_fields("", playable_role, query, offset, limit)?;
    call_data_library_with_request(
        b"dnf_avatar_data_query_sets_page\0",
        &SetPageQueryRequest {
            playable_role,
            query,
            offset,
            limit,
        },
    )
}

pub fn get_avatar_items_by_ids_from_data_library(
    item_ids: &[String],
) -> Result<Vec<AvatarItem>, String> {
    let item_ids = normalized_catalog_item_ids(item_ids)?;
    call_data_library_with_request(
        b"dnf_avatar_data_get_items_by_ids\0",
        &ItemIdsRequest {
            item_ids: &item_ids,
        },
    )
}

pub(crate) fn find_avatar_items_by_ids_from_data_library(
    item_ids: &[String],
) -> Result<Vec<AvatarItem>, String> {
    let item_ids = normalized_catalog_item_ids(item_ids)?;
    call_data_library_with_request(
        b"dnf_avatar_data_find_items_by_ids\0",
        &ItemIdsRequest {
            item_ids: &item_ids,
        },
    )
}

pub fn get_avatar_set_with_members_from_data_library(
    set_id: &str,
    playable_role: &str,
) -> Result<AvatarSetDetail, String> {
    validate_catalog_set_detail_fields(set_id, playable_role)?;
    call_data_library_with_request(
        b"dnf_avatar_data_get_set_with_members\0",
        &SetDetailRequest {
            set_id,
            playable_role,
        },
    )
}

pub fn ensure_wardrobe_snapshot_resources_from_data_library(
    snapshot: &WardrobeSnapshotInput,
) -> Result<(), String> {
    let catalog = catalog_context_for_wardrobe_snapshot(snapshot)?;
    ensure_wardrobe_snapshot_resources(snapshot, &catalog)
}

pub fn load_layers_from_data_library(item_ids: &[String]) -> Result<Vec<AvatarLayer>, String> {
    let mut layers = deduplicate_avatar_layers(load_raw_layers_from_data_library(item_ids)?);
    sort_avatar_layers_for_draw(&mut layers);
    Ok(layers)
}

pub fn load_layers_for_playable_role_from_data_library(
    item_ids: &[String],
    playable_role_key: &str,
) -> Result<Vec<AvatarLayer>, String> {
    let catalog = catalog_context_from_data_library(Vec::new())?;
    let family = visual_family_for_playable_role(&catalog, playable_role_key)?;
    load_layers_for_visual_family_from_data_library(item_ids, &family.visual_family_key)
}

fn load_layers_for_visual_family_from_data_library(
    item_ids: &[String],
    visual_family_key: &str,
) -> Result<Vec<AvatarLayer>, String> {
    let raw_layers = load_raw_layers_from_data_library(item_ids)?;
    let mut layers = deduplicate_avatar_layers(filter_layers_for_visual_family(
        raw_layers,
        visual_family_key,
    ));
    sort_avatar_layers_for_draw(&mut layers);
    Ok(layers)
}

fn load_raw_layers_from_data_library(item_ids: &[String]) -> Result<Vec<AvatarLayer>, String> {
    let item_ids = normalized_catalog_item_ids(item_ids)?;
    let mut layers: Vec<AvatarLayer> = call_data_library_with_request(
        b"dnf_avatar_data_load_layers\0",
        &ItemIdsRequest {
            item_ids: &item_ids,
        },
    )?;
    sort_avatar_layers_for_draw(&mut layers);
    Ok(layers)
}

pub(crate) fn attach_hide_equipment_rules(layers: &mut [AvatarLayer], catalog: &CatalogResponse) {
    let mut rules_by_item = HashMap::<&str, (BTreeSet<u16>, bool, bool)>::new();
    for item in &catalog.items {
        let rules = rules_by_item
            .entry(item.item_id.as_str())
            .or_insert_with(|| (BTreeSet::new(), false, false));
        rules
            .0
            .extend(item.hide_equipment_type_enums.iter().copied());
        rules.1 |= item.enable_dye_mask1;
        rules.2 |= item.enable_dye_mask2;
    }
    for layer in layers {
        if let Some((hide_rules, mask1, mask2)) = rules_by_item.get(layer.item_id.as_str()) {
            layer.hide_equipment_type_enums = hide_rules.iter().copied().collect();
            layer.enable_dye_mask1 = *mask1;
            layer.enable_dye_mask2 = *mask2;
        } else {
            layer.hide_equipment_type_enums.clear();
            layer.enable_dye_mask1 = false;
            layer.enable_dye_mask2 = false;
        }
    }
}

pub fn load_complete_wardrobe_layers_from_data_library(
    snapshot: &WardrobeSnapshotInput,
) -> Result<Vec<AvatarLayer>, String> {
    let item_ids = snapshot
        .items
        .iter()
        .filter_map(|saved| saved.item_ref.clone())
        .collect::<Vec<_>>();
    let catalog = catalog_context_for_wardrobe_snapshot(snapshot)?;
    let family = ensure_snapshot_role_binding(snapshot, &catalog)?;
    let raw_layers = filter_layers_for_visual_family(
        load_raw_layers_from_data_library(&item_ids)?,
        &family.visual_family_key,
    );
    complete_wardrobe_layers_for_family(snapshot, raw_layers, family)
}

pub fn load_wardrobe_weapon_layers_from_data_library(
    snapshot: &WardrobeSnapshotInput,
) -> Result<Vec<AvatarLayer>, String> {
    let saved = snapshot
        .weapon
        .as_ref()
        .ok_or_else(|| "已保存装扮没有武器".to_string())?;
    if !saved.slot.trim().eq_ignore_ascii_case("weapon") {
        return Err("已保存武器槽位无效".to_string());
    }
    let item_ref = saved
        .item_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "已保存武器缺少私有资源引用".to_string())?;
    let catalog = catalog_context_for_wardrobe_snapshot(snapshot)?;
    let family = ensure_snapshot_role_binding(snapshot, &catalog)?;
    let item = catalog
        .items
        .iter()
        .find(|item| item.item_id.trim() == item_ref)
        .ok_or_else(|| "已保存武器已不在当前数据版本中".to_string())?;
    let playable_role_key = snapshot_playable_role_key(snapshot);
    if !catalog_item_is_weapon(item)
        || !catalog_item_matches_playable_role(item, &playable_role_key)
    {
        return Err("武器只能与同职业的普通武器或武器时装换装".to_string());
    }

    let loaded_layers = load_layers_for_visual_family_from_data_library(
        &[item_ref.to_string()],
        &family.visual_family_key,
    )?;
    if loaded_layers.len() != saved.layer_count
        || loaded_layers
            .iter()
            .any(|layer| layer.item_id.trim() != item_ref || !loaded_layer_is_weapon(layer))
    {
        return Err("已保存武器 IMG 资源已变化或不完整".to_string());
    }
    let layers = loaded_layers
        .into_iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
        .collect::<Vec<_>>();
    if layers.is_empty() {
        return Err("已保存武器没有可读取的 IMG".to_string());
    }
    Ok(layers)
}

pub(crate) fn catalog_item_is_weapon(item: &AvatarItem) -> bool {
    matches!(
        item.resource_kind.trim().to_ascii_lowercase().as_str(),
        "weapon" | "weapon_avatar"
    )
}

pub(crate) fn loaded_layer_is_weapon(layer: &AvatarLayer) -> bool {
    layer.equipment_type_enum == 12
        || (layer.equipment_type_enum == 10 && layer.slot.trim().eq_ignore_ascii_case("weapon"))
}

pub(crate) fn complete_wardrobe_layers_for_family(
    snapshot: &WardrobeSnapshotInput,
    raw_layers: Vec<AvatarLayer>,
    family: &AvatarVisualFamily,
) -> Result<Vec<AvatarLayer>, String> {
    if snapshot.visual_family_key.trim().is_empty()
        || !family
            .visual_family_key
            .eq_ignore_ascii_case(&snapshot.visual_family_key)
        || raw_layers.iter().any(|layer| {
            !layer
                .visual_family_key
                .eq_ignore_ascii_case(&snapshot.visual_family_key)
        })
    {
        return Err("装扮图层不属于快照声明的视觉族".to_string());
    }
    ensure_loaded_wardrobe_layers(snapshot, &raw_layers)?;
    with_default_avatar_skin_for_family(family, &deduplicate_avatar_layers(raw_layers))
}

pub(crate) fn with_default_avatar_skin_for_family(
    family: &AvatarVisualFamily,
    layers: &[AvatarLayer],
) -> Result<Vec<AvatarLayer>, String> {
    let mut effective = layers.to_vec();
    if effective
        .iter()
        .any(|layer| layer.slot.eq_ignore_ascii_case("skin"))
    {
        sort_avatar_layers_for_draw(&mut effective);
        return Ok(effective);
    }
    let visual_img = family.default_skin_img.trim();
    if visual_img.is_empty() {
        return Err(format!(
            "no default skin is registered for visual family {}",
            family.visual_family_key
        ));
    }
    effective.push(AvatarLayer {
        item_id: "default_skin".to_string(),
        item_name: "默认皮肤".to_string(),
        slot: "skin".to_string(),
        visual_family_key: family.visual_family_key.clone(),
        layer_order: -100,
        equipment_type_enum: 8,
        hide_equipment_type_enums: Vec::new(),
        enable_dye_mask1: false,
        enable_dye_mask2: false,
        layer_sequence: 0,
        layer_tag: "[skin]".to_string(),
        variation: "0".to_string(),
        palette_index: 0,
        visual_img: visual_img.to_string(),
        status: "default_skin".to_string(),
    });
    sort_avatar_layers_for_draw(&mut effective);
    Ok(effective)
}

pub(crate) fn wardrobe_snapshot_owns_layer(
    snapshot: &WardrobeSnapshotInput,
    layer: &AvatarLayer,
) -> bool {
    wardrobe_snapshot_owns_layer_with_visual_family_record(snapshot, layer, None)
}

fn wardrobe_snapshot_owns_layer_with_visual_family_record(
    snapshot: &WardrobeSnapshotInput,
    layer: &AvatarLayer,
    family: Option<&AvatarVisualFamily>,
) -> bool {
    snapshot.items.iter().any(|item| {
        if !item.slot.trim().eq_ignore_ascii_case(layer.slot.trim()) {
            return false;
        }
        if let Some(item_ref) = item.item_ref.as_deref() {
            return item_ref.trim() == layer.item_id.trim();
        }
        if !item.slot.trim().eq_ignore_ascii_case("skin")
            || layer.item_id.trim() != "default_skin"
            || layer.status.trim() != "default_skin"
        {
            return false;
        }
        layer
            .visual_family_key
            .eq_ignore_ascii_case(&snapshot.visual_family_key)
            && family.is_none_or(|family| {
                family
                    .visual_family_key
                    .eq_ignore_ascii_case(&snapshot.visual_family_key)
                    && normalize_img_path_for_npk(&family.default_skin_img)
                        == normalize_img_path_for_npk(layer.visual_img.trim())
            })
    })
}

pub(crate) fn wardrobe_snapshot_owns_dye_target(
    snapshot: &WardrobeSnapshotInput,
    target: &DyeTargetRecipeV1,
) -> bool {
    wardrobe_snapshot_owns_dye_target_with_visual_family_record(snapshot, target, None)
}

fn wardrobe_snapshot_owns_dye_target_with_visual_family_record(
    snapshot: &WardrobeSnapshotInput,
    target: &DyeTargetRecipeV1,
    family: Option<&AvatarVisualFamily>,
) -> bool {
    snapshot.items.iter().any(|item| {
        if !item
            .slot
            .trim()
            .eq_ignore_ascii_case(target.target.slot.trim())
        {
            return false;
        }
        if let Some(item_ref) = item.item_ref.as_deref() {
            return item_ref.trim() == target.target.item_ref.trim();
        }
        if !item.slot.trim().eq_ignore_ascii_case("skin")
            || !target
                .target
                .item_ref
                .trim()
                .eq_ignore_ascii_case("default_skin")
            || target.target.layer_tag.trim() != "[skin]"
        {
            return false;
        }
        let Some(family) = family else {
            return default_skin_target_matches_snapshot_scope(
                snapshot,
                &target.target.private_entry_ref,
            );
        };
        if !family
            .visual_family_key
            .eq_ignore_ascii_case(&snapshot.visual_family_key)
        {
            return false;
        }
        let expected_base = family.default_skin_img.as_str();
        match target.target.target_kind {
            DyeTargetKindV1::Base => {
                normalize_img_path_for_npk(expected_base)
                    == normalize_img_path_for_npk(&target.target.private_entry_ref)
            }
            DyeTargetKindV1::Mask { number } => {
                legacy_same_named_mask_number_for_ownership(
                    expected_base,
                    &target.target.private_entry_ref,
                ) == Some(number)
            }
        }
    })
}

fn default_skin_target_matches_snapshot_scope(
    snapshot: &WardrobeSnapshotInput,
    private_entry_ref: &str,
) -> bool {
    let owner = snapshot.resource_owner_key.trim().to_ascii_lowercase();
    let generation = snapshot.resource_generation.trim().to_ascii_lowercase();
    if owner.is_empty() || !matches!(generation.as_str(), "base" | "at") {
        return false;
    }
    let equipment_root = if generation == "at" {
        "atequipment"
    } else {
        "equipment"
    };
    let normalized = normalize_img_path_for_npk(private_entry_ref).to_ascii_lowercase();
    normalized.contains(&format!("character/{owner}/{equipment_root}/avatar/skin/"))
}

pub fn invalid_wardrobe_snapshot_slots(
    snapshot: &WardrobeSnapshotInput,
    catalog: &CatalogResponse,
) -> Vec<String> {
    if ensure_snapshot_role_binding(snapshot, catalog).is_err() {
        let mut invalid = snapshot
            .items
            .iter()
            .map(|saved| saved.slot.trim())
            .filter(|slot| !slot.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if snapshot.weapon.is_some() {
            invalid.push("weapon".to_string());
        }
        if invalid.is_empty() {
            invalid.push("skin".to_string());
        }
        return invalid;
    }
    let playable_role_key = snapshot_playable_role_key(snapshot);
    let mut invalid = snapshot
        .items
        .iter()
        .filter_map(|saved| {
            let item_ref = saved.item_ref.as_deref()?;
            let valid = catalog.items.iter().any(|item| {
                item.item_id == item_ref
                    && item.slot.eq_ignore_ascii_case(saved.slot.trim())
                    && item.resource_kind.trim().eq_ignore_ascii_case("avatar")
                    && saved.layer_count > 0
                    && catalog_item_layer_count_for_playable_role(item, &playable_role_key, catalog)
                        == saved.layer_count
                    && catalog_item_matches_playable_role(item, &playable_role_key)
            });
            (!valid).then(|| saved.slot.clone())
        })
        .collect::<Vec<_>>();
    if let Some(saved) = snapshot.weapon.as_ref() {
        let valid = saved
            .item_ref
            .as_deref()
            .filter(|item_ref| !item_ref.trim().is_empty())
            .is_some_and(|item_ref| {
                catalog.items.iter().any(|item| {
                    item.item_id == item_ref
                        && catalog_item_is_weapon(item)
                        && saved.layer_count > 0
                        && catalog_item_layer_count_for_playable_role(
                            item,
                            &playable_role_key,
                            catalog,
                        ) == saved.layer_count
                        && catalog_item_matches_playable_role(item, &playable_role_key)
                })
            });
        if !valid {
            invalid.push("weapon".to_string());
        }
    }
    invalid
}

pub fn ensure_loaded_wardrobe_layers(
    snapshot: &WardrobeSnapshotInput,
    layers: &[AvatarLayer],
) -> Result<(), String> {
    let incomplete_slots = snapshot
        .items
        .iter()
        .filter_map(|saved| {
            let item_ref = saved.item_ref.as_deref()?;
            let loaded_layers = layers
                .iter()
                .filter(|layer| {
                    layer.item_id == item_ref && layer.slot.eq_ignore_ascii_case(saved.slot.trim())
                })
                .collect::<Vec<_>>();
            let logical_layers = deduplicate_avatar_layers(
                loaded_layers.iter().map(|layer| (*layer).clone()).collect(),
            );
            let complete = loaded_layers.len() == saved.layer_count
                && !logical_layers.is_empty()
                && logical_layers.iter().all(|layer| {
                    !layer.visual_img.trim().is_empty()
                        || !layer.hide_equipment_type_enums.is_empty()
                });
            (!complete).then(|| saved.slot.clone())
        })
        .collect::<Vec<_>>();
    if incomplete_slots.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "装扮 IMG 图层不完整，禁止按空部位处理：{}",
            incomplete_slots.join("、")
        ))
    }
}

pub fn ensure_wardrobe_snapshot_resources(
    snapshot: &WardrobeSnapshotInput,
    catalog: &CatalogResponse,
) -> Result<(), String> {
    ensure_snapshot_role_binding(snapshot, catalog)?;
    let invalid_slots = invalid_wardrobe_snapshot_slots(snapshot, catalog);
    if invalid_slots.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "装扮资源已变化，以下部位无法从当前数据源精确解析：{}",
            invalid_slots.join("、")
        ))
    }
}

fn data_library() -> Result<&'static Library, String> {
    if let Some(library) = DATA_LIBRARY.get() {
        return Ok(library);
    }
    let path = data_library_path()?;
    let library = unsafe { Library::new(&path) }
        .map_err(|err| format!("载入内置数据失败 {}：{err}", path.display()))?;
    let version = unsafe {
        let version = library
            .get::<unsafe extern "C" fn() -> u32>(b"dnf_avatar_data_version\0")
            .map_err(|err| format!("读取内置数据版本接口失败：{err}"))?;
        version()
    };
    ensure_data_schema_version(version)?;
    let _ = DATA_LIBRARY.set(library);
    DATA_LIBRARY
        .get()
        .ok_or_else(|| "内置数据 DLL 初始化失败".to_string())
}

pub(crate) fn ensure_data_schema_version(actual: u32) -> Result<(), String> {
    if actual == DATA_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(format!(
            "内置数据版本不兼容：需要 {DATA_SCHEMA_VERSION}，实际为 {actual}"
        ))
    }
}

fn call_data_library_without_request<T>(symbol_name: &[u8]) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let _call_guard = DATA_LIBRARY_CALL_LOCK
        .lock()
        .map_err(|_| "内置数据 DLL 调用锁已损坏".to_string())?;
    let library = data_library()?;
    let bytes = unsafe {
        let call = library
            .get::<unsafe extern "C" fn(*mut usize) -> *mut u8>(symbol_name)
            .map_err(|err| format!("读取内置数据查询接口失败：{err}"))?;
        let mut len = 0usize;
        let pointer = call(&mut len);
        copy_data_library_response(library, pointer, len)?
    };
    serde_json::from_slice(&bytes).map_err(|err| format!("解析内置数据查询响应失败：{err}"))
}

fn call_data_library_with_request<Request, Response>(
    symbol_name: &[u8],
    request: &Request,
) -> Result<Response, String>
where
    Request: Serialize,
    Response: for<'de> Deserialize<'de>,
{
    let request = serde_json::to_string(request)
        .map_err(|err| format!("序列化内置数据查询请求失败：{err}"))?;
    let request =
        CString::new(request).map_err(|_| "内置数据查询请求包含非法 NUL 字符".to_string())?;
    let _call_guard = DATA_LIBRARY_CALL_LOCK
        .lock()
        .map_err(|_| "内置数据 DLL 调用锁已损坏".to_string())?;
    let library = data_library()?;
    let bytes = unsafe {
        let call = library
            .get::<unsafe extern "C" fn(*const c_char, *mut usize) -> *mut u8>(symbol_name)
            .map_err(|err| format!("读取内置数据查询接口失败：{err}"))?;
        let mut len = 0usize;
        let pointer = call(request.as_ptr(), &mut len);
        copy_data_library_response(library, pointer, len)?
    };
    serde_json::from_slice(&bytes).map_err(|err| format!("解析内置数据查询响应失败：{err}"))
}

unsafe fn copy_data_library_response(
    library: &Library,
    pointer: *mut u8,
    len: usize,
) -> Result<Vec<u8>, String> {
    let free = unsafe {
        library
            .get::<unsafe extern "C" fn(*mut u8, usize)>(b"dnf_avatar_data_free\0")
            .map_err(|err| format!("读取内置数据释放接口失败：{err}"))?
    };
    let last_error = unsafe {
        library
            .get::<unsafe extern "C" fn() -> *const c_char>(b"dnf_avatar_data_last_error\0")
            .map_err(|err| format!("读取内置数据错误接口失败：{err}"))?
    };
    if pointer.is_null() {
        let error_pointer = unsafe { last_error() };
        let message = if error_pointer.is_null() {
            "未知内置数据查询错误".to_string()
        } else {
            unsafe { CStr::from_ptr(error_pointer) }
                .to_string_lossy()
                .into_owned()
        };
        return Err(message);
    }
    if len > MAX_DATA_LIBRARY_RESPONSE_BYTES {
        unsafe {
            free(pointer, len);
        }
        return Err(format!(
            "内置数据查询响应超过 {} 字节安全上限",
            MAX_DATA_LIBRARY_RESPONSE_BYTES
        ));
    }
    let bytes = unsafe { slice::from_raw_parts(pointer, len) }.to_vec();
    unsafe {
        free(pointer, len);
    }
    Ok(bytes)
}

fn is_placeholder_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.starts_with('<')
        || placeholder_suffix(trimmed).is_some()
        || trimmed
            .split_once("::")
            .and_then(|(_, suffix)| placeholder_suffix(suffix.trim_end_matches('>')))
            .is_some()
}

fn sanitize_display_name(name: &str) -> Option<String> {
    let mut cleaned = name.trim().to_string();
    loop {
        let Some((prefix, suffix)) = cleaned.rsplit_once(" - ") else {
            break;
        };
        if !is_internal_token(suffix) {
            break;
        }
        cleaned = prefix.trim().to_string();
    }
    if cleaned.is_empty()
        || is_placeholder_name(&cleaned)
        || is_internal_token(&cleaned)
        || cleaned.chars().all(|ch| ch.is_ascii_digit())
    {
        None
    } else {
        Some(cleaned)
    }
}

fn is_internal_token(value: &str) -> bool {
    let trimmed = value.trim().trim_start_matches('<').trim_end_matches('>');
    let Some((left, right)) = trimmed.split_once("::") else {
        return false;
    };
    !left.is_empty()
        && left.chars().all(|ch| ch.is_ascii_digit())
        && !right.is_empty()
        && right
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn placeholder_suffix(value: &str) -> Option<&str> {
    let rest = value.strip_prefix("name_")?;
    let digit_count = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    let suffix = &rest[digit_count..];
    (suffix.is_empty() || suffix.starts_with(char::is_whitespace) || suffix.starts_with('-'))
        .then_some(suffix)
}

fn is_lucky_bag_set_name(name: &str) -> bool {
    name.contains("福包") || name.contains("福宝")
}

fn data_library_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("DNF_AVATAR_DATA_DLL") {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe().map_err(|err| format!("定位程序失败：{err}"))?;
    let parent = exe.parent().ok_or_else(|| "定位程序目录失败".to_string())?;
    Ok(parent.join(DATA_DLL_NAME))
}
