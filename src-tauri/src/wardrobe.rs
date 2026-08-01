use std::collections::HashSet;
use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::avatar_data::{
    wardrobe_snapshot_owns_dye_target, WardrobeDyeValidation, WardrobeDyeValidationStatus,
};
use crate::avatar_dye::{canonicalize_recipe, DyeRecipeV1, DyeTargetKindV1, PersistedDyeStateV2};
use crate::models::{
    CatalogResponse, OutfitSnapshot, WardrobeEntryDraft, WardrobeEntryView, WardrobeGroupView,
    WardrobeRole, WardrobeSnapshotInput, WardrobeSnapshotItemView, WardrobeSnapshotView,
    WardrobeStateView,
};
use crate::wardrobe_limits::{
    MAX_SHARE_ENTRIES, MAX_WARDROBE_CATALOG_CONTEXT_ITEM_IDS, MAX_WARDROBE_ENTRIES,
    MAX_WARDROBE_READ_BYTES, MAX_WARDROBE_STORE_BYTES,
};

const FILE_MAGIC_V1: &[u8; 8] = b"DNFWRDB1";
const FILE_MAGIC_V2: &[u8; 8] = b"DNFWRDB2";
const FILE_MAGIC_V3: &[u8; 8] = b"DNFWRDB3";
const FILE_MAGIC_V4: &[u8; 8] = b"DNFWRDB4";
const FILE_MAGIC_V5: &[u8; 8] = b"DNFWRDB5";
const FILE_SCHEMA_V2: u16 = 2;
const FILE_SCHEMA_V3: u16 = 3;
const FILE_SCHEMA_V4: u16 = 4;
const FILE_SCHEMA_V5: u16 = 5;
const SHARE_MAGIC_V1: &[u8; 8] = b"DNFWDSH1";
const SHARE_MAGIC_V2: &[u8; 8] = b"DNFWDSH2";
const SHARE_MAGIC_V3: &[u8; 8] = b"DNFWDSH3";
const SHARE_MAGIC_V4: &[u8; 8] = b"DNFWDSH4";
const SHARE_MAGIC_V5: &[u8; 8] = b"DNFWDSH5";
const SHARE_VERSION_V1: u8 = 1;
const SHARE_VERSION_V2: u8 = 2;
const SHARE_VERSION_V3: u8 = 3;
const SHARE_VERSION_V4: u8 = 4;
const SHARE_VERSION_V5: u8 = 5;
const DYE_STATE_SCHEMA_V2: u16 = 2;
const SHARE_NONCE_LEN: usize = 12;
const SHARE_TAG_LEN: usize = 16;
const MAX_SHARE_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SEMANTIC_STATE_JSON_BYTES: usize = 512 * 1024;
const IMPORTED_ENTRY_STORAGE_OVERHEAD_BYTES: usize = 256;
const SHARE_KEY_SEED: &[u8] = b"DNF Avatar Studio portable encrypted wardrobe share key 2026-07-13";
const OUTFIT_SLOTS: [&str; 9] = [
    "skin", "cap", "hair", "face", "neck", "coat", "belt", "pants", "shoes",
];
const SYSTEM_GROUP_NAMES: [&str; 4] = ["全部", "收藏", "最近使用", "未分栏"];
const DUPLICATE_ENTRY_ERROR: &str =
    "橱窗中已存在内容完全相同的时装与配色，不能重复保存；可更新已有记录或取消";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WardrobeStore {
    current: Option<StoredSnapshot>,
    target: Option<StoredSnapshot>,
    groups: Vec<StoredGroup>,
    entries: Vec<StoredEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSnapshot {
    snapshot: WardrobeSnapshotInput,
    dye_state: Option<PersistedDyeStateV2>,
    saved_at_ms: u64,
    revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeFileV5 {
    schema_version: u16,
    store: WardrobeStore,
}

#[derive(Serialize)]
struct WardrobeFileV5Ref<'a> {
    schema_version: u16,
    store: &'a WardrobeStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeFileV4 {
    schema_version: u16,
    store: LegacyWardrobeStoreV4,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeFileV3 {
    schema_version: u16,
    store: LegacyWardrobeStoreV3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeFileV2 {
    schema_version: u16,
    store: LegacyWardrobeStoreV2,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyWardrobeStoreV2 {
    current: Option<LegacyStoredSnapshotV2>,
    target: Option<LegacyStoredSnapshotV2>,
    groups: Vec<LegacyStoredGroupV2>,
    entries: Vec<LegacyStoredEntryV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredSnapshotV2 {
    snapshot: LegacyWardrobeSnapshotInputV1,
    dye_recipe: Option<DyeRecipeV1>,
    saved_at_ms: u64,
    revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredGroupV2 {
    id: String,
    name: String,
    sort_order: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredEntryV2 {
    id: String,
    name: String,
    note: String,
    group_id: Option<String>,
    favorite: bool,
    snapshot: LegacyStoredSnapshotV2,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyWardrobeStoreV3 {
    current: Option<LegacyStoredSnapshotV3>,
    target: Option<LegacyStoredSnapshotV3>,
    groups: Vec<LegacyStoredGroupV2>,
    entries: Vec<LegacyStoredEntryV3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredSnapshotV3 {
    snapshot: LegacyWardrobeSnapshotInputV1,
    dye_state: Option<PersistedDyeStateV2>,
    saved_at_ms: u64,
    revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredEntryV3 {
    id: String,
    name: String,
    note: String,
    group_id: Option<String>,
    favorite: bool,
    snapshot: LegacyStoredSnapshotV3,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug)]
enum WardrobeLoadError {
    RecoverableCorruption(String),
    PreservePrimary(String),
}

impl WardrobeLoadError {
    fn into_message(self) -> String {
        match self {
            Self::RecoverableCorruption(message) | Self::PreservePrimary(message) => message,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyWardrobeStoreV1 {
    current: Option<LegacyStoredSnapshotV1>,
    target: Option<LegacyStoredSnapshotV1>,
    groups: Vec<LegacyStoredGroupV1>,
    entries: Vec<LegacyStoredEntryV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredSnapshotV1 {
    snapshot: LegacyWardrobeSnapshotInputV1,
    saved_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyWardrobeSnapshotInputV1 {
    profession_dir: String,
    path_family: String,
    items: Vec<LegacyWardrobeSnapshotItemInputV1>,
    preview_data_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyWardrobeSnapshotItemInputV1 {
    slot: String,
    item_ref: Option<String>,
    item_name: Option<String>,
    layer_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyWardrobeSnapshotInputV4 {
    profession_dir: String,
    path_family: String,
    items: Vec<LegacyWardrobeSnapshotItemInputV1>,
    weapon: Option<LegacyWardrobeSnapshotItemInputV1>,
    preview_data_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyWardrobeStoreV4 {
    current: Option<LegacyStoredSnapshotV4>,
    target: Option<LegacyStoredSnapshotV4>,
    groups: Vec<LegacyStoredGroupV2>,
    entries: Vec<LegacyStoredEntryV4>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredSnapshotV4 {
    snapshot: LegacyWardrobeSnapshotInputV4,
    dye_state: Option<PersistedDyeStateV2>,
    saved_at_ms: u64,
    revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredEntryV4 {
    id: String,
    name: String,
    note: String,
    group_id: Option<String>,
    favorite: bool,
    snapshot: LegacyStoredSnapshotV4,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredGroupV1 {
    id: String,
    name: String,
    sort_order: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyStoredEntryV1 {
    id: String,
    name: String,
    note: String,
    group_id: Option<String>,
    favorite: bool,
    snapshot: LegacyStoredSnapshotV1,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredGroup {
    id: String,
    name: String,
    sort_order: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    id: String,
    name: String,
    note: String,
    group_id: Option<String>,
    favorite: bool,
    snapshot: StoredSnapshot,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeSharePackageV2 {
    version: u8,
    group_name: Option<String>,
    entries: Vec<WardrobeShareEntryV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeSharePackageV5 {
    version: u8,
    group_name: Option<String>,
    entries: Vec<WardrobeShareEntryV5>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeShareEntryV5 {
    name: String,
    note: String,
    favorite: bool,
    snapshot: WardrobeSnapshotInput,
    dye_state: Option<PersistedDyeStateV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeSharePackageV3 {
    version: u8,
    group_name: Option<String>,
    entries: Vec<WardrobeShareEntryV3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeShareEntryV3 {
    name: String,
    note: String,
    favorite: bool,
    snapshot: LegacyWardrobeSnapshotInputV1,
    dye_state: Option<PersistedDyeStateV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeSharePackageV4 {
    version: u8,
    group_name: Option<String>,
    entries: Vec<WardrobeShareEntryV4>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeShareEntryV4 {
    name: String,
    note: String,
    favorite: bool,
    snapshot: LegacyWardrobeSnapshotInputV4,
    dye_state: Option<PersistedDyeStateV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeShareEntryV2 {
    name: String,
    note: String,
    favorite: bool,
    snapshot: LegacyWardrobeSnapshotInputV1,
    dye_recipe: Option<DyeRecipeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeSharePackageV1 {
    version: u8,
    group_name: Option<String>,
    entries: Vec<WardrobeShareEntryV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WardrobeShareEntryV1 {
    name: String,
    note: String,
    favorite: bool,
    snapshot: LegacyWardrobeSnapshotInputV1,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PrivateWardrobeSnapshot {
    pub snapshot: WardrobeSnapshotInput,
    pub dye_recipe: Option<DyeRecipeV1>,
    pub dye_state: Option<PersistedDyeStateV2>,
    pub saved_at_ms: u64,
    pub revision: String,
}

impl From<LegacyWardrobeSnapshotItemInputV1> for crate::models::WardrobeSnapshotItemInput {
    fn from(value: LegacyWardrobeSnapshotItemInputV1) -> Self {
        Self {
            slot: value.slot,
            item_ref: value.item_ref,
            item_name: value.item_name,
            layer_count: value.layer_count,
        }
    }
}

impl From<LegacyWardrobeSnapshotInputV1> for WardrobeSnapshotInput {
    fn from(value: LegacyWardrobeSnapshotInputV1) -> Self {
        let binding = legacy_role_binding(&value.profession_dir, &value.path_family);
        Self {
            playable_role_key: binding
                .as_ref()
                .map(|binding| binding.0.clone())
                .unwrap_or_default(),
            visual_family_key: binding
                .as_ref()
                .map(|binding| binding.1.clone())
                .unwrap_or_default(),
            resource_owner_key: binding
                .as_ref()
                .map(|binding| binding.2.clone())
                .unwrap_or_default(),
            resource_generation: binding
                .as_ref()
                .map(|binding| binding.3.clone())
                .unwrap_or_default(),
            profession_dir: value.profession_dir,
            path_family: value.path_family,
            items: value.items.into_iter().map(Into::into).collect(),
            weapon: None,
            preview_data_url: value.preview_data_url,
        }
    }
}

impl From<LegacyWardrobeSnapshotInputV4> for WardrobeSnapshotInput {
    fn from(value: LegacyWardrobeSnapshotInputV4) -> Self {
        let binding = legacy_role_binding(&value.profession_dir, &value.path_family);
        Self {
            playable_role_key: binding
                .as_ref()
                .map(|binding| binding.0.clone())
                .unwrap_or_default(),
            visual_family_key: binding
                .as_ref()
                .map(|binding| binding.1.clone())
                .unwrap_or_default(),
            resource_owner_key: binding
                .as_ref()
                .map(|binding| binding.2.clone())
                .unwrap_or_default(),
            resource_generation: binding
                .as_ref()
                .map(|binding| binding.3.clone())
                .unwrap_or_default(),
            profession_dir: value.profession_dir,
            path_family: value.path_family,
            items: value.items.into_iter().map(Into::into).collect(),
            weapon: value.weapon.map(Into::into),
            preview_data_url: value.preview_data_url,
        }
    }
}

fn legacy_role_binding(
    profession_dir: &str,
    path_family: &str,
) -> Option<(String, String, String, String)> {
    let raw_role = profession_dir.trim().to_ascii_lowercase();
    let legacy_at_path = {
        let family = path_family.trim().replace('\\', "/").to_ascii_lowercase();
        family == "at_avatar" || family.contains("/atequipment/")
    };
    let playable_role_key = if raw_role.starts_with("at_")
        || matches!(
            raw_role.as_str(),
            "archer"
                | "creator_mage"
                | "demonic_swordman"
                | "demoniclancer"
                | "gunblader"
                | "imperialknight"
                | "knight"
                | "thief"
        ) {
        raw_role
    } else if legacy_at_path
        && matches!(
            raw_role.as_str(),
            "fighter" | "gunner" | "mage" | "priest" | "swordman"
        )
    {
        format!("at_{raw_role}")
    } else {
        raw_role
    };
    let (visual_family_key, resource_owner_key, resource_generation) =
        match playable_role_key.as_str() {
            "archer" => ("archer_base", "archer", "base"),
            "at_fighter" => ("fighter_at", "fighter", "at"),
            "at_gunner" => ("gunner_at", "gunner", "at"),
            "at_mage" => ("mage_at", "mage", "at"),
            "at_priest" => ("priest_at", "priest", "at"),
            "at_swordman" => ("swordman_at", "swordman", "at"),
            "creator_mage" | "mage" => ("mage_base", "mage", "base"),
            "demonic_swordman" | "swordman" => ("swordman_base", "swordman", "base"),
            "demoniclancer" => ("demoniclancer_base", "demoniclancer", "base"),
            "fighter" => ("fighter_base", "fighter", "base"),
            "gunblader" => ("gunblader_base", "gunblader", "base"),
            "gunner" => ("gunner_base", "gunner", "base"),
            "imperialknight" => ("imperialknight_base", "imperialknight", "base"),
            "knight" => ("knight_base", "knight", "base"),
            "priest" => ("priest_base", "priest", "base"),
            "thief" => ("thief_base", "thief", "base"),
            _ => return None,
        };
    Some((
        playable_role_key,
        visual_family_key.to_string(),
        resource_owner_key.to_string(),
        resource_generation.to_string(),
    ))
}

impl From<LegacyStoredSnapshotV1> for StoredSnapshot {
    fn from(value: LegacyStoredSnapshotV1) -> Self {
        let snapshot = value.snapshot.into();
        Self {
            revision: new_snapshot_revision(),
            snapshot,
            dye_state: None,
            saved_at_ms: value.saved_at_ms,
        }
    }
}

impl From<LegacyStoredSnapshotV2> for StoredSnapshot {
    fn from(value: LegacyStoredSnapshotV2) -> Self {
        Self {
            snapshot: value.snapshot.into(),
            dye_state: persisted_state_from_recipe(value.dye_recipe),
            saved_at_ms: value.saved_at_ms,
            revision: value.revision,
        }
    }
}

impl From<LegacyStoredSnapshotV3> for StoredSnapshot {
    fn from(value: LegacyStoredSnapshotV3) -> Self {
        Self {
            snapshot: value.snapshot.into(),
            dye_state: value.dye_state,
            saved_at_ms: value.saved_at_ms,
            revision: value.revision,
        }
    }
}

impl From<LegacyStoredSnapshotV4> for StoredSnapshot {
    fn from(value: LegacyStoredSnapshotV4) -> Self {
        Self {
            snapshot: value.snapshot.into(),
            dye_state: value.dye_state,
            saved_at_ms: value.saved_at_ms,
            revision: value.revision,
        }
    }
}

impl From<LegacyWardrobeStoreV2> for WardrobeStore {
    fn from(value: LegacyWardrobeStoreV2) -> Self {
        Self {
            current: value.current.map(Into::into),
            target: value.target.map(Into::into),
            groups: value
                .groups
                .into_iter()
                .map(|group| StoredGroup {
                    id: group.id,
                    name: group.name,
                    sort_order: group.sort_order,
                })
                .collect(),
            entries: value
                .entries
                .into_iter()
                .map(|entry| StoredEntry {
                    id: entry.id,
                    name: entry.name,
                    note: entry.note,
                    group_id: entry.group_id,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    created_at_ms: entry.created_at_ms,
                    updated_at_ms: entry.updated_at_ms,
                    last_used_at_ms: entry.last_used_at_ms,
                })
                .collect(),
        }
    }
}

impl From<LegacyWardrobeStoreV3> for WardrobeStore {
    fn from(value: LegacyWardrobeStoreV3) -> Self {
        Self {
            current: value.current.map(Into::into),
            target: value.target.map(Into::into),
            groups: value
                .groups
                .into_iter()
                .map(|group| StoredGroup {
                    id: group.id,
                    name: group.name,
                    sort_order: group.sort_order,
                })
                .collect(),
            entries: value
                .entries
                .into_iter()
                .map(|entry| StoredEntry {
                    id: entry.id,
                    name: entry.name,
                    note: entry.note,
                    group_id: entry.group_id,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    created_at_ms: entry.created_at_ms,
                    updated_at_ms: entry.updated_at_ms,
                    last_used_at_ms: entry.last_used_at_ms,
                })
                .collect(),
        }
    }
}

impl From<LegacyWardrobeStoreV4> for WardrobeStore {
    fn from(value: LegacyWardrobeStoreV4) -> Self {
        Self {
            current: value.current.map(Into::into),
            target: value.target.map(Into::into),
            groups: value
                .groups
                .into_iter()
                .map(|group| StoredGroup {
                    id: group.id,
                    name: group.name,
                    sort_order: group.sort_order,
                })
                .collect(),
            entries: value
                .entries
                .into_iter()
                .map(|entry| StoredEntry {
                    id: entry.id,
                    name: entry.name,
                    note: entry.note,
                    group_id: entry.group_id,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    created_at_ms: entry.created_at_ms,
                    updated_at_ms: entry.updated_at_ms,
                    last_used_at_ms: entry.last_used_at_ms,
                })
                .collect(),
        }
    }
}

impl From<LegacyWardrobeStoreV1> for WardrobeStore {
    fn from(value: LegacyWardrobeStoreV1) -> Self {
        Self {
            current: value.current.map(Into::into),
            target: value.target.map(Into::into),
            groups: value
                .groups
                .into_iter()
                .map(|group| StoredGroup {
                    id: group.id,
                    name: group.name,
                    sort_order: group.sort_order,
                })
                .collect(),
            entries: value
                .entries
                .into_iter()
                .map(|entry| StoredEntry {
                    id: entry.id,
                    name: entry.name,
                    note: entry.note,
                    group_id: entry.group_id,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    created_at_ms: entry.created_at_ms,
                    updated_at_ms: entry.updated_at_ms,
                    last_used_at_ms: entry.last_used_at_ms,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WardrobeShareImportSummary {
    pub entry_count: usize,
    pub group_name: Option<String>,
}

pub struct WardrobeRepository {
    path: PathBuf,
    store: WardrobeStore,
}

pub struct WardrobeAppState {
    path: PathBuf,
    repository: Mutex<Option<Result<WardrobeRepository, String>>>,
}

impl WardrobeAppState {
    pub fn open_default() -> Self {
        let path = crate::app_config::runtime_root()
            .join("config")
            .join("outfit_wardrobe.bin");
        Self::open_at(path)
    }

    pub(crate) fn open_at(path: PathBuf) -> Self {
        Self {
            repository: Mutex::new(None),
            path,
        }
    }

    pub fn with_repository<T>(
        &self,
        action: impl FnOnce(&WardrobeRepository) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self
            .repository
            .lock()
            .map_err(|_| "时装橱窗状态不可用".to_string())?;
        let repository =
            guard.get_or_insert_with(|| WardrobeRepository::open_at(self.path.clone()));
        action(repository.as_ref().map_err(Clone::clone)?)
    }

    pub fn with_repository_mut<T>(
        &self,
        action: impl FnOnce(&mut WardrobeRepository) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self
            .repository
            .lock()
            .map_err(|_| "时装橱窗状态不可用".to_string())?;
        let repository =
            guard.get_or_insert_with(|| WardrobeRepository::open_at(self.path.clone()));
        action(repository.as_mut().map_err(|error| error.clone())?)
    }

    pub fn reset_after_confirmation(&self, confirmed: bool) -> Result<WardrobeStateView, String> {
        if !confirmed {
            return Err("重置时装橱窗需要用户确认".to_string());
        }
        let mut guard = self
            .repository
            .lock()
            .map_err(|_| "时装橱窗状态不可用".to_string())?;
        let repository = WardrobeRepository::reset_at(self.path.clone())?;
        let view = repository.view();
        *guard = Some(Ok(repository));
        Ok(view)
    }
}

impl WardrobeRepository {
    pub fn open_at(path: PathBuf) -> Result<Self, String> {
        let store = if path.exists() {
            match read_store(&path) {
                Ok(store) => store,
                Err(WardrobeLoadError::RecoverableCorruption(_)) => {
                    let backup = backup_path(&path);
                    let store = read_store(&backup).map_err(WardrobeLoadError::into_message)?;
                    restore_primary_from_backup(&path, &backup)?;
                    store
                }
                Err(WardrobeLoadError::PreservePrimary(error)) => return Err(error),
            }
        } else if backup_path(&path).exists() {
            let backup = backup_path(&path);
            let store = read_store(&backup).map_err(WardrobeLoadError::into_message)?;
            restore_primary_from_backup(&path, &backup)?;
            store
        } else {
            WardrobeStore::default()
        };
        Ok(Self { path, store })
    }

    pub fn reset_at(path: PathBuf) -> Result<Self, String> {
        for candidate in [&path, &backup_path(&path), &path.with_extension("tmp")] {
            if candidate.exists() {
                fs::remove_file(candidate).map_err(|_| "清理损坏的时装橱窗文件失败".to_string())?;
            }
        }
        let store = WardrobeStore::default();
        write_store(&path, &store)?;
        Ok(Self { path, store })
    }

    pub fn save_active(
        &mut self,
        role: WardrobeRole,
        snapshot: WardrobeSnapshotInput,
    ) -> Result<WardrobeStateView, String> {
        self.save_active_with_policy(role, snapshot, None, false)
    }

    pub(crate) fn save_active_with_dye_recipe(
        &mut self,
        role: WardrobeRole,
        snapshot: WardrobeSnapshotInput,
        dye_recipe: Option<DyeRecipeV1>,
    ) -> Result<WardrobeStateView, String> {
        self.save_active_with_dye_state(role, snapshot, persisted_state_from_recipe(dye_recipe))
    }

    pub(crate) fn save_active_with_dye_state(
        &mut self,
        role: WardrobeRole,
        snapshot: WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
    ) -> Result<WardrobeStateView, String> {
        self.save_active_with_policy(role, snapshot, dye_state, false)
    }

    pub fn save_active_replacing_incompatible(
        &mut self,
        role: WardrobeRole,
        snapshot: WardrobeSnapshotInput,
    ) -> Result<WardrobeStateView, String> {
        self.save_active_with_policy(role, snapshot, None, true)
    }

    pub(crate) fn save_active_replacing_incompatible_with_dye_recipe(
        &mut self,
        role: WardrobeRole,
        snapshot: WardrobeSnapshotInput,
        dye_recipe: Option<DyeRecipeV1>,
    ) -> Result<WardrobeStateView, String> {
        self.save_active_replacing_incompatible_with_dye_state(
            role,
            snapshot,
            persisted_state_from_recipe(dye_recipe),
        )
    }

    pub(crate) fn save_active_replacing_incompatible_with_dye_state(
        &mut self,
        role: WardrobeRole,
        snapshot: WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
    ) -> Result<WardrobeStateView, String> {
        self.save_active_with_policy(role, snapshot, dye_state, true)
    }

    fn save_active_with_policy(
        &mut self,
        role: WardrobeRole,
        snapshot: WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
        clear_incompatible: bool,
    ) -> Result<WardrobeStateView, String> {
        validate_snapshot(&snapshot)?;
        let dye_state = canonicalize_optional_dye_state(&snapshot, dye_state)?;
        let mut next = self.store.clone();
        if let Err(error) = validate_active_compatibility(&next, role, &snapshot) {
            if clear_incompatible {
                match role {
                    WardrobeRole::Current => next.target = None,
                    WardrobeRole::Target => next.current = None,
                }
            } else {
                return Err(error);
            }
        }
        let stored = new_stored_snapshot(snapshot, dye_state, now_ms());
        match role {
            WardrobeRole::Current => next.current = Some(stored),
            WardrobeRole::Target => next.target = Some(stored),
        }
        write_store(&self.path, &next)?;
        self.store = next;
        Ok(self.view())
    }

    pub fn clear_active(&mut self, role: WardrobeRole) -> Result<WardrobeStateView, String> {
        let mut next = self.store.clone();
        match role {
            WardrobeRole::Current => next.current = None,
            WardrobeRole::Target => next.target = None,
        }
        self.commit(next)?;
        Ok(self.view())
    }

    pub fn create_group(&mut self, name: &str) -> Result<WardrobeGroupView, String> {
        let name = normalized_label(name, "栏目名称不能为空")?;
        if SYSTEM_GROUP_NAMES.contains(&name.as_str())
            || self.store.groups.iter().any(|group| group.name == name)
        {
            return Err("栏目名称已存在".to_string());
        }
        let group = StoredGroup {
            id: new_id("group"),
            name,
            sort_order: self.store.groups.len(),
        };
        let mut next = self.store.clone();
        next.groups.push(group.clone());
        self.commit(next)?;
        Ok(group_view(&group))
    }

    pub fn rename_group(&mut self, group_id: &str, name: &str) -> Result<(), String> {
        let name = normalized_label(name, "栏目名称不能为空")?;
        if self
            .store
            .groups
            .iter()
            .any(|group| group.id != group_id && group.name == name)
            || SYSTEM_GROUP_NAMES.contains(&name.as_str())
        {
            return Err("栏目名称已存在".to_string());
        }
        let mut next = self.store.clone();
        let group = next
            .groups
            .iter_mut()
            .find(|group| group.id == group_id)
            .ok_or_else(|| "没有找到目标栏目".to_string())?;
        group.name = name;
        self.commit(next)
    }

    pub fn reorder_groups(&mut self, group_ids: &[String]) -> Result<(), String> {
        if group_ids.len() != self.store.groups.len()
            || self
                .store
                .groups
                .iter()
                .any(|group| !group_ids.iter().any(|id| id == &group.id))
        {
            return Err("栏目排序数据无效".to_string());
        }
        let mut next = self.store.clone();
        next.groups.sort_by_key(|group| {
            group_ids
                .iter()
                .position(|id| id == &group.id)
                .unwrap_or(usize::MAX)
        });
        for (index, group) in next.groups.iter_mut().enumerate() {
            group.sort_order = index;
        }
        self.commit(next)
    }

    pub fn delete_group(&mut self, group_id: &str) -> Result<(), String> {
        let mut next = self.store.clone();
        let original_len = next.groups.len();
        next.groups.retain(|group| group.id != group_id);
        if next.groups.len() == original_len {
            return Err("没有找到目标栏目".to_string());
        }
        for (index, group) in next.groups.iter_mut().enumerate() {
            group.sort_order = index;
        }
        for entry in &mut next.entries {
            if entry.group_id.as_deref() == Some(group_id) {
                entry.group_id = None;
                entry.updated_at_ms = now_ms();
            }
        }
        self.commit(next)
    }

    pub fn create_entry(
        &mut self,
        snapshot: WardrobeSnapshotInput,
        draft: WardrobeEntryDraft,
    ) -> Result<WardrobeEntryView, String> {
        self.create_entry_with_dye_state(snapshot, None, draft)
    }

    pub(crate) fn create_entry_with_dye_recipe(
        &mut self,
        snapshot: WardrobeSnapshotInput,
        dye_recipe: Option<DyeRecipeV1>,
        draft: WardrobeEntryDraft,
    ) -> Result<WardrobeEntryView, String> {
        self.create_entry_with_dye_state(snapshot, persisted_state_from_recipe(dye_recipe), draft)
    }

    pub(crate) fn create_entry_with_dye_state(
        &mut self,
        snapshot: WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
        draft: WardrobeEntryDraft,
    ) -> Result<WardrobeEntryView, String> {
        ensure_wardrobe_entry_growth(self.store.entries.len(), 1)?;
        validate_snapshot(&snapshot)?;
        let dye_state = canonicalize_optional_dye_state(&snapshot, dye_state)?;
        ensure_unique_entry_content(&self.store, None, &snapshot, dye_state.as_ref())?;
        let name = normalized_label(&draft.name, "装扮名称不能为空")?;
        validate_group(&self.store, draft.group_id.as_deref())?;
        let timestamp = now_ms();
        let entry = StoredEntry {
            id: new_id("outfit"),
            name,
            note: draft.note.trim().to_string(),
            group_id: draft.group_id,
            favorite: draft.favorite,
            snapshot: new_stored_snapshot(snapshot, dye_state, timestamp),
            created_at_ms: timestamp,
            updated_at_ms: timestamp,
            last_used_at_ms: None,
        };
        let mut next = self.store.clone();
        next.entries.push(entry.clone());
        self.commit(next)?;
        Ok(entry_view(
            &entry,
            None,
            None,
            self.store.current.as_ref(),
            self.store.target.as_ref(),
        ))
    }

    pub fn create_entry_with_duplicate_policy(
        &mut self,
        snapshot: WardrobeSnapshotInput,
        draft: WardrobeEntryDraft,
        allow_duplicate: bool,
    ) -> Result<WardrobeEntryView, String> {
        self.create_entry_with_duplicate_policy_and_dye_state(
            snapshot,
            None,
            draft,
            allow_duplicate,
        )
    }

    pub(crate) fn create_entry_with_duplicate_policy_and_dye_recipe(
        &mut self,
        snapshot: WardrobeSnapshotInput,
        dye_recipe: Option<DyeRecipeV1>,
        draft: WardrobeEntryDraft,
        allow_duplicate: bool,
    ) -> Result<WardrobeEntryView, String> {
        self.create_entry_with_duplicate_policy_and_dye_state(
            snapshot,
            persisted_state_from_recipe(dye_recipe),
            draft,
            allow_duplicate,
        )
    }

    pub(crate) fn create_entry_with_duplicate_policy_and_dye_state(
        &mut self,
        snapshot: WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
        draft: WardrobeEntryDraft,
        _allow_duplicate: bool,
    ) -> Result<WardrobeEntryView, String> {
        self.create_entry_with_dye_state(snapshot, dye_state, draft)
    }

    pub fn set_favorite(&mut self, entry_id: &str, favorite: bool) -> Result<(), String> {
        let mut next = self.store.clone();
        let entry = find_entry_mut(&mut next, entry_id)?;
        entry.favorite = favorite;
        entry.updated_at_ms = now_ms();
        self.commit(next)
    }

    pub fn update_entry(
        &mut self,
        entry_id: &str,
        draft: WardrobeEntryDraft,
    ) -> Result<(), String> {
        let name = normalized_label(&draft.name, "装扮名称不能为空")?;
        if draft.note.chars().count() > 500 {
            return Err("备注过长".to_string());
        }
        validate_group(&self.store, draft.group_id.as_deref())?;
        let mut next = self.store.clone();
        let entry = find_entry_mut(&mut next, entry_id)?;
        entry.name = name;
        entry.note = draft.note.trim().to_string();
        entry.group_id = draft.group_id;
        entry.favorite = draft.favorite;
        entry.updated_at_ms = now_ms();
        self.commit(next)
    }

    pub fn replace_entry_snapshot(
        &mut self,
        entry_id: &str,
        snapshot: WardrobeSnapshotInput,
    ) -> Result<(), String> {
        self.replace_entry_snapshot_with_dye_state(entry_id, snapshot, None)
    }

    pub(crate) fn replace_entry_snapshot_with_dye_recipe(
        &mut self,
        entry_id: &str,
        snapshot: WardrobeSnapshotInput,
        dye_recipe: Option<DyeRecipeV1>,
    ) -> Result<(), String> {
        self.replace_entry_snapshot_with_dye_state(
            entry_id,
            snapshot,
            persisted_state_from_recipe(dye_recipe),
        )
    }

    pub(crate) fn replace_entry_snapshot_with_dye_state(
        &mut self,
        entry_id: &str,
        snapshot: WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
    ) -> Result<(), String> {
        validate_snapshot(&snapshot)?;
        let dye_state = canonicalize_optional_dye_state(&snapshot, dye_state)?;
        if !self.store.entries.iter().any(|entry| entry.id == entry_id) {
            return Err("没有找到橱窗装扮".to_string());
        }
        ensure_unique_entry_content(&self.store, Some(entry_id), &snapshot, dye_state.as_ref())?;
        let mut next = self.store.clone();
        let entry = find_entry_mut(&mut next, entry_id)?;
        let timestamp = now_ms();
        entry.snapshot = new_stored_snapshot(snapshot, dye_state, timestamp);
        entry.updated_at_ms = timestamp;
        self.commit(next)
    }

    pub fn activate_entry(
        &mut self,
        entry_id: &str,
        role: WardrobeRole,
    ) -> Result<WardrobeStateView, String> {
        self.activate_entry_with_policy(entry_id, role, false)
    }

    pub fn activate_entry_replacing_incompatible(
        &mut self,
        entry_id: &str,
        role: WardrobeRole,
    ) -> Result<WardrobeStateView, String> {
        self.activate_entry_with_policy(entry_id, role, true)
    }

    fn activate_entry_with_policy(
        &mut self,
        entry_id: &str,
        role: WardrobeRole,
        clear_incompatible: bool,
    ) -> Result<WardrobeStateView, String> {
        let mut next = self.store.clone();
        let timestamp = now_ms();
        let snapshot = {
            let entry = find_entry_mut(&mut next, entry_id)?;
            entry.last_used_at_ms = Some(timestamp);
            let mut snapshot = entry.snapshot.clone();
            snapshot.saved_at_ms = timestamp;
            snapshot.revision = new_snapshot_revision();
            snapshot
        };
        if let Err(error) = validate_active_compatibility(&next, role, &snapshot.snapshot) {
            if clear_incompatible && role == WardrobeRole::Current {
                next.target = None;
            } else {
                return Err(error);
            }
        }
        match role {
            WardrobeRole::Current => next.current = Some(snapshot),
            WardrobeRole::Target => next.target = Some(snapshot),
        }
        self.commit(next)?;
        Ok(self.view())
    }

    pub fn delete_entry(&mut self, entry_id: &str) -> Result<(), String> {
        let mut next = self.store.clone();
        let original_len = next.entries.len();
        next.entries.retain(|entry| entry.id != entry_id);
        if next.entries.len() == original_len {
            return Err("没有找到橱窗装扮".to_string());
        }
        self.commit(next)
    }

    pub fn export_entry_share_file(
        &self,
        entry_id: &str,
        output_path: &Path,
    ) -> Result<usize, String> {
        let entry = self
            .store
            .entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| "没有找到橱窗装扮".to_string())?;
        let package = WardrobeSharePackageV5 {
            version: SHARE_VERSION_V5,
            group_name: None,
            entries: vec![share_entry(entry)],
        };
        write_share_file(output_path, &package)?;
        Ok(1)
    }

    pub fn export_group_share_file(
        &self,
        group_id: &str,
        output_path: &Path,
    ) -> Result<usize, String> {
        let group = self
            .store
            .groups
            .iter()
            .find(|group| group.id == group_id)
            .ok_or_else(|| "没有找到目标栏目".to_string())?;
        let entries = self
            .store
            .entries
            .iter()
            .filter(|entry| entry.group_id.as_deref() == Some(group_id))
            .map(share_entry)
            .collect::<Vec<_>>();
        let entry_count = entries.len();
        write_share_file(
            output_path,
            &WardrobeSharePackageV5 {
                version: SHARE_VERSION_V5,
                group_name: Some(group.name.clone()),
                entries,
            },
        )?;
        Ok(entry_count)
    }

    pub fn import_share_file(
        &mut self,
        input_path: &Path,
    ) -> Result<WardrobeShareImportSummary, String> {
        self.import_share_file_validated(input_path, |_| Ok(()))
            .map(|(summary, ())| summary)
    }

    pub(crate) fn import_share_file_validated<T>(
        &mut self,
        input_path: &Path,
        validate: impl FnOnce(&WardrobeRepository) -> Result<T, String>,
    ) -> Result<(WardrobeShareImportSummary, T), String> {
        let package = read_share_file(input_path)?;
        validate_share_package(&package)?;
        ensure_wardrobe_entry_growth(self.store.entries.len(), package.entries.len())?;
        ensure_import_serialized_growth(&self.store, &package)?;
        let mut next = self.store.clone();
        let imported_group = package.group_name.as_deref().map(|name| {
            let name = unique_imported_group_name(&next, name);
            let group = StoredGroup {
                id: new_id("group"),
                name: name.clone(),
                sort_order: next.groups.len(),
            };
            next.groups.push(group.clone());
            (group.id, name)
        });
        let timestamp = now_ms();
        let group_id = imported_group.as_ref().map(|(id, _)| id.clone());
        let entry_count = package.entries.len();
        // Import preserves every authenticated share entry as authored. Local save APIs enforce
        // exact-content deduplication before reaching this separate bulk-import path.
        next.entries
            .extend(package.entries.into_iter().map(|entry| StoredEntry {
                id: new_id("outfit"),
                name: entry.name.trim().to_string(),
                note: entry.note.trim().to_string(),
                group_id: group_id.clone(),
                favorite: entry.favorite,
                snapshot: new_stored_snapshot(entry.snapshot, entry.dye_state, timestamp),
                created_at_ms: timestamp,
                updated_at_ms: timestamp,
                last_used_at_ms: None,
            }));
        let summary = WardrobeShareImportSummary {
            entry_count,
            group_name: imported_group.map(|(_, name)| name),
        };
        let candidate = WardrobeRepository {
            path: self.path.clone(),
            store: next.clone(),
        };
        let validated = validate(&candidate)?;
        self.commit(next)?;
        Ok((summary, validated))
    }

    pub fn view(&self) -> WardrobeStateView {
        self.build_view(|_| None, |_, _| None)
    }

    pub(crate) fn referenced_item_ids(&self) -> Vec<String> {
        let mut item_ids = Vec::new();
        let mut collect = |snapshot: &WardrobeSnapshotInput| {
            item_ids.extend(
                snapshot
                    .items
                    .iter()
                    .filter_map(|item| item.item_ref.as_ref())
                    .chain(
                        snapshot
                            .weapon
                            .iter()
                            .filter_map(|item| item.item_ref.as_ref()),
                    )
                    .map(|item_id| item_id.trim())
                    .filter(|item_id| !item_id.is_empty())
                    .map(str::to_owned),
            );
        };
        if let Some(snapshot) = &self.store.current {
            collect(&snapshot.snapshot);
        }
        if let Some(snapshot) = &self.store.target {
            collect(&snapshot.snapshot);
        }
        for entry in &self.store.entries {
            collect(&entry.snapshot.snapshot);
        }
        item_ids.sort();
        item_ids.dedup();
        item_ids
    }

    pub fn validated_view(&self, catalog: &CatalogResponse) -> WardrobeStateView {
        self.build_view(
            |snapshot| {
                Some(crate::avatar_data::invalid_wardrobe_snapshot_slots(
                    snapshot, catalog,
                ))
            },
            |_, _| None,
        )
    }

    pub(crate) fn validated_view_with_dye(
        &self,
        catalog: &CatalogResponse,
        mut validation_for: impl FnMut(
            &WardrobeSnapshotInput,
            Option<&DyeRecipeV1>,
        ) -> WardrobeDyeValidation,
    ) -> WardrobeStateView {
        self.build_view(
            |snapshot| {
                Some(crate::avatar_data::invalid_wardrobe_snapshot_slots(
                    snapshot, catalog,
                ))
            },
            |snapshot, recipe| Some(validation_for(snapshot, recipe)),
        )
    }

    fn build_view(
        &self,
        mut invalid_slots_for: impl FnMut(&WardrobeSnapshotInput) -> Option<Vec<String>>,
        mut dye_validation_for: impl FnMut(
            &WardrobeSnapshotInput,
            Option<&DyeRecipeV1>,
        ) -> Option<WardrobeDyeValidation>,
    ) -> WardrobeStateView {
        WardrobeStateView {
            current: self.store.current.as_ref().map(|snapshot| {
                let invalid_slots = invalid_slots_for(&snapshot.snapshot);
                let dye_validation = dye_validation_for(
                    &snapshot.snapshot,
                    baked_recipe(snapshot.dye_state.as_ref()),
                );
                snapshot_view(snapshot, invalid_slots, dye_validation)
            }),
            target: self.store.target.as_ref().map(|snapshot| {
                let invalid_slots = invalid_slots_for(&snapshot.snapshot);
                let dye_validation = dye_validation_for(
                    &snapshot.snapshot,
                    baked_recipe(snapshot.dye_state.as_ref()),
                );
                snapshot_view(snapshot, invalid_slots, dye_validation)
            }),
            groups: self.store.groups.iter().map(group_view).collect(),
            entries: self
                .store
                .entries
                .iter()
                .map(|entry| {
                    let invalid_slots = invalid_slots_for(&entry.snapshot.snapshot);
                    let dye_validation = dye_validation_for(
                        &entry.snapshot.snapshot,
                        baked_recipe(entry.snapshot.dye_state.as_ref()),
                    );
                    entry_view(
                        entry,
                        invalid_slots,
                        dye_validation,
                        self.store.current.as_ref(),
                        self.store.target.as_ref(),
                    )
                })
                .collect(),
        }
    }

    pub fn find_matching_entry(
        &self,
        snapshot: &WardrobeSnapshotInput,
    ) -> Result<Option<WardrobeEntryView>, String> {
        self.find_matching_entry_with_dye_state(snapshot, None)
    }

    pub(crate) fn find_matching_entry_with_dye_recipe(
        &self,
        snapshot: &WardrobeSnapshotInput,
        dye_recipe: Option<DyeRecipeV1>,
    ) -> Result<Option<WardrobeEntryView>, String> {
        self.find_matching_entry_with_dye_state(snapshot, persisted_state_from_recipe(dye_recipe))
    }

    pub(crate) fn find_matching_entry_with_dye_state(
        &self,
        snapshot: &WardrobeSnapshotInput,
        dye_state: Option<PersistedDyeStateV2>,
    ) -> Result<Option<WardrobeEntryView>, String> {
        validate_snapshot(snapshot)?;
        let dye_state = canonicalize_optional_dye_state(snapshot, dye_state)?;
        Ok(self
            .store
            .entries
            .iter()
            .find(|entry| {
                snapshots_have_same_content(
                    &entry.snapshot.snapshot,
                    entry.snapshot.dye_state.as_ref(),
                    snapshot,
                    dye_state.as_ref(),
                )
            })
            .map(|entry| {
                entry_view(
                    entry,
                    None,
                    None,
                    self.store.current.as_ref(),
                    self.store.target.as_ref(),
                )
            }))
    }

    pub fn entry_snapshot(&self, entry_id: &str) -> Result<WardrobeSnapshotInput, String> {
        self.store
            .entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .map(|entry| entry.snapshot.snapshot.clone())
            .ok_or_else(|| "没有找到橱窗装扮".to_string())
    }

    pub(crate) fn entry_private_snapshot(
        &self,
        entry_id: &str,
    ) -> Result<PrivateWardrobeSnapshot, String> {
        self.store
            .entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .map(|entry| private_wardrobe_snapshot(&entry.snapshot))
            .ok_or_else(|| "没有找到橱窗装扮".to_string())
    }

    pub(crate) fn entry_dye_aware_snapshot(
        &self,
        entry_id: &str,
    ) -> Result<(WardrobeSnapshotInput, OutfitSnapshot, Option<DyeRecipeV1>), String> {
        let (snapshot, outfit, dye_state) = self.entry_dye_state_snapshot(entry_id)?;
        Ok((snapshot, outfit, dye_state.map(|state| state.baked_recipe)))
    }

    pub(crate) fn entry_dye_state_snapshot(
        &self,
        entry_id: &str,
    ) -> Result<
        (
            WardrobeSnapshotInput,
            OutfitSnapshot,
            Option<PersistedDyeStateV2>,
        ),
        String,
    > {
        let stored = self
            .store
            .entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .map(|entry| &entry.snapshot)
            .ok_or_else(|| "没有找到橱窗装扮".to_string())?;
        Ok((
            stored.snapshot.clone(),
            private_snapshot(stored),
            stored.dye_state.clone(),
        ))
    }

    pub fn active_input_snapshot(
        &self,
        role: WardrobeRole,
    ) -> Result<WardrobeSnapshotInput, String> {
        let stored = match role {
            WardrobeRole::Current => self.store.current.as_ref(),
            WardrobeRole::Target => self.store.target.as_ref(),
        }
        .ok_or_else(|| match role {
            WardrobeRole::Current => "尚未保存当前穿戴".to_string(),
            WardrobeRole::Target => "尚未保存目标外观".to_string(),
        })?;
        Ok(stored.snapshot.clone())
    }

    pub fn active_input_pair(
        &self,
    ) -> Result<(WardrobeSnapshotInput, WardrobeSnapshotInput), String> {
        Ok((
            self.active_input_snapshot(WardrobeRole::Current)?,
            self.active_input_snapshot(WardrobeRole::Target)?,
        ))
    }

    pub fn active_snapshot(&self, role: WardrobeRole) -> Result<OutfitSnapshot, String> {
        let stored = self.active_stored_snapshot(role)?;
        Ok(private_snapshot(stored))
    }

    pub(crate) fn active_private_snapshot(
        &self,
        role: WardrobeRole,
    ) -> Result<PrivateWardrobeSnapshot, String> {
        Ok(private_wardrobe_snapshot(
            self.active_stored_snapshot(role)?,
        ))
    }

    pub(crate) fn active_dye_aware_snapshot(
        &self,
        role: WardrobeRole,
    ) -> Result<(WardrobeSnapshotInput, OutfitSnapshot, Option<DyeRecipeV1>), String> {
        let (snapshot, outfit, dye_state) = self.active_dye_state_snapshot(role)?;
        Ok((snapshot, outfit, dye_state.map(|state| state.baked_recipe)))
    }

    pub(crate) fn active_dye_state_snapshot(
        &self,
        role: WardrobeRole,
    ) -> Result<
        (
            WardrobeSnapshotInput,
            OutfitSnapshot,
            Option<PersistedDyeStateV2>,
        ),
        String,
    > {
        let stored = self.active_stored_snapshot(role)?;
        Ok((
            stored.snapshot.clone(),
            private_snapshot(stored),
            stored.dye_state.clone(),
        ))
    }

    pub(crate) fn active_dye_aware_pair(
        &self,
    ) -> Result<
        (
            (WardrobeSnapshotInput, OutfitSnapshot, Option<DyeRecipeV1>),
            (WardrobeSnapshotInput, OutfitSnapshot, Option<DyeRecipeV1>),
        ),
        String,
    > {
        Ok((
            self.active_dye_aware_snapshot(WardrobeRole::Current)?,
            self.active_dye_aware_snapshot(WardrobeRole::Target)?,
        ))
    }

    pub(crate) fn active_dye_state_pair(
        &self,
    ) -> Result<
        (
            (
                WardrobeSnapshotInput,
                OutfitSnapshot,
                Option<PersistedDyeStateV2>,
            ),
            (
                WardrobeSnapshotInput,
                OutfitSnapshot,
                Option<PersistedDyeStateV2>,
            ),
        ),
        String,
    > {
        Ok((
            self.active_dye_state_snapshot(WardrobeRole::Current)?,
            self.active_dye_state_snapshot(WardrobeRole::Target)?,
        ))
    }

    fn active_stored_snapshot(&self, role: WardrobeRole) -> Result<&StoredSnapshot, String> {
        match role {
            WardrobeRole::Current => self.store.current.as_ref(),
            WardrobeRole::Target => self.store.target.as_ref(),
        }
        .ok_or_else(|| match role {
            WardrobeRole::Current => "尚未保存当前穿戴".to_string(),
            WardrobeRole::Target => "尚未保存目标外观".to_string(),
        })
    }

    pub fn active_pair(&self) -> Result<(OutfitSnapshot, OutfitSnapshot), String> {
        Ok((
            self.active_snapshot(WardrobeRole::Current)?,
            self.active_snapshot(WardrobeRole::Target)?,
        ))
    }

    fn commit(&mut self, next: WardrobeStore) -> Result<(), String> {
        validate_store_capacity(&next)?;
        write_store(&self.path, &next)?;
        self.store = next;
        Ok(())
    }
}

fn share_entry(entry: &StoredEntry) -> WardrobeShareEntryV5 {
    WardrobeShareEntryV5 {
        name: entry.name.clone(),
        note: entry.note.clone(),
        favorite: entry.favorite,
        snapshot: entry.snapshot.snapshot.clone(),
        dye_state: entry.snapshot.dye_state.clone(),
    }
}

fn write_share_file(path: &Path, package: &WardrobeSharePackageV5) -> Result<(), String> {
    validate_share_package(package)?;
    let plain = postcard::to_allocvec(package).map_err(|_| "编码橱窗分享失败".to_string())?;
    let maximum_plain_size =
        MAX_SHARE_FILE_BYTES.saturating_sub(SHARE_MAGIC_V5.len() + SHARE_NONCE_LEN + SHARE_TAG_LEN);
    if plain.len() > maximum_plain_size {
        return Err("橱窗分享内容过大".to_string());
    }
    let cipher = share_cipher();
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plain,
                aad: SHARE_MAGIC_V5,
            },
        )
        .map_err(|_| "加密橱窗分享失败".to_string())?;
    let mut file = Vec::with_capacity(SHARE_MAGIC_V5.len() + SHARE_NONCE_LEN + ciphertext.len());
    file.extend_from_slice(SHARE_MAGIC_V5);
    file.extend_from_slice(&nonce);
    file.extend_from_slice(&ciphertext);
    if file.len() > MAX_SHARE_FILE_BYTES {
        return Err("橱窗分享内容过大".to_string());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "创建橱窗分享目录失败".to_string())?;
    }
    let mut output = fs::File::create(path).map_err(|_| "创建橱窗分享文件失败".to_string())?;
    output
        .write_all(&file)
        .and_then(|_| output.sync_all())
        .map_err(|_| "写入橱窗分享文件失败".to_string())
}

fn read_share_file(path: &Path) -> Result<WardrobeSharePackageV5, String> {
    let metadata = fs::metadata(path).map_err(|_| "读取橱窗分享文件失败".to_string())?;
    if metadata.len() > MAX_SHARE_FILE_BYTES as u64 {
        return Err("橱窗分享文件过大".to_string());
    }
    let file = fs::read(path).map_err(|_| "读取橱窗分享文件失败".to_string())?;
    if file.len() > MAX_SHARE_FILE_BYTES {
        return Err("橱窗分享文件过大".to_string());
    }
    let magic = if file.starts_with(SHARE_MAGIC_V5) {
        SHARE_MAGIC_V5.as_slice()
    } else if file.starts_with(SHARE_MAGIC_V4) {
        SHARE_MAGIC_V4.as_slice()
    } else if file.starts_with(SHARE_MAGIC_V3) {
        SHARE_MAGIC_V3.as_slice()
    } else if file.starts_with(SHARE_MAGIC_V2) {
        SHARE_MAGIC_V2.as_slice()
    } else if file.starts_with(SHARE_MAGIC_V1) {
        SHARE_MAGIC_V1.as_slice()
    } else {
        return Err("橱窗分享文件格式无效".to_string());
    };
    let header_len = magic.len() + SHARE_NONCE_LEN;
    if file.len() <= header_len {
        return Err("橱窗分享文件格式无效".to_string());
    }
    let nonce = Nonce::from_slice(&file[magic.len()..header_len]);
    let plain = share_cipher()
        .decrypt(
            nonce,
            Payload {
                msg: &file[header_len..],
                aad: magic,
            },
        )
        .map_err(|_| "橱窗分享文件校验失败或已被篡改".to_string())?;
    let mut package = if magic == SHARE_MAGIC_V5 {
        let package = postcard::from_bytes::<WardrobeSharePackageV5>(&plain)
            .map_err(|_| "解析橱窗分享失败".to_string())?;
        if package.version != SHARE_VERSION_V5 {
            return Err("橱窗分享版本不受支持".to_string());
        }
        package
    } else if magic == SHARE_MAGIC_V4 {
        let legacy = postcard::from_bytes::<WardrobeSharePackageV4>(&plain)
            .map_err(|_| "解析橱窗分享失败".to_string())?;
        if legacy.version != SHARE_VERSION_V4 {
            return Err("橱窗分享版本不受支持".to_string());
        }
        WardrobeSharePackageV5 {
            version: SHARE_VERSION_V5,
            group_name: legacy.group_name,
            entries: legacy
                .entries
                .into_iter()
                .map(|entry| WardrobeShareEntryV5 {
                    name: entry.name,
                    note: entry.note,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    dye_state: entry.dye_state,
                })
                .collect(),
        }
    } else if magic == SHARE_MAGIC_V3 {
        let legacy = postcard::from_bytes::<WardrobeSharePackageV3>(&plain)
            .map_err(|_| "解析橱窗分享失败".to_string())?;
        if legacy.version != SHARE_VERSION_V3 {
            return Err("橱窗分享版本不受支持".to_string());
        }
        WardrobeSharePackageV5 {
            version: SHARE_VERSION_V5,
            group_name: legacy.group_name,
            entries: legacy
                .entries
                .into_iter()
                .map(|entry| WardrobeShareEntryV5 {
                    name: entry.name,
                    note: entry.note,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    dye_state: entry.dye_state,
                })
                .collect(),
        }
    } else if magic == SHARE_MAGIC_V2 {
        let legacy = postcard::from_bytes::<WardrobeSharePackageV2>(&plain)
            .map_err(|_| "解析橱窗分享失败".to_string())?;
        if legacy.version != SHARE_VERSION_V2 {
            return Err("橱窗分享版本不受支持".to_string());
        }
        WardrobeSharePackageV5 {
            version: SHARE_VERSION_V5,
            group_name: legacy.group_name,
            entries: legacy
                .entries
                .into_iter()
                .map(|entry| WardrobeShareEntryV5 {
                    name: entry.name,
                    note: entry.note,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    dye_state: persisted_state_from_recipe(entry.dye_recipe),
                })
                .collect(),
        }
    } else {
        let legacy = postcard::from_bytes::<WardrobeSharePackageV1>(&plain)
            .map_err(|_| "解析橱窗分享失败".to_string())?;
        if legacy.version != SHARE_VERSION_V1 {
            return Err("橱窗分享版本不受支持".to_string());
        }
        WardrobeSharePackageV5 {
            version: SHARE_VERSION_V5,
            group_name: legacy.group_name,
            entries: legacy
                .entries
                .into_iter()
                .map(|entry| WardrobeShareEntryV5 {
                    name: entry.name,
                    note: entry.note,
                    favorite: entry.favorite,
                    snapshot: entry.snapshot.into(),
                    dye_state: None,
                })
                .collect(),
        }
    };
    normalize_share_package(&mut package)?;
    Ok(package)
}

fn share_cipher() -> ChaCha20Poly1305 {
    let key: [u8; 32] = Sha256::digest(SHARE_KEY_SEED).into();
    ChaCha20Poly1305::new((&key).into())
}

fn validate_share_package(package: &WardrobeSharePackageV5) -> Result<(), String> {
    if package.version != SHARE_VERSION_V5 {
        return Err("橱窗分享版本不受支持".to_string());
    }
    if package.entries.len() > MAX_SHARE_ENTRIES
        || (package.group_name.is_none() && package.entries.len() != 1)
    {
        return Err("橱窗分享内容数量无效".to_string());
    }
    if let Some(group_name) = package.group_name.as_deref() {
        normalized_label(group_name, "分享栏目名称为空")?;
    }
    for entry in &package.entries {
        normalized_label(&entry.name, "分享装扮名称为空")?;
        if entry.note.chars().count() > 500 {
            return Err("分享装扮备注过长".to_string());
        }
        validate_snapshot(&entry.snapshot)?;
        if canonicalize_optional_dye_state(&entry.snapshot, entry.dye_state.clone())?
            != entry.dye_state
        {
            return Err("橱窗分享包含未规范化的染色状态".to_string());
        }
    }
    Ok(())
}

fn unique_imported_group_name(store: &WardrobeStore, requested: &str) -> String {
    let requested = requested.trim();
    let exists = |candidate: &str| {
        SYSTEM_GROUP_NAMES.contains(&candidate)
            || store.groups.iter().any(|group| group.name == candidate)
    };
    if !exists(requested) {
        return requested.to_string();
    }
    for sequence in 2usize.. {
        let suffix = format!("（导入 {sequence}）");
        let base_len = 80usize.saturating_sub(suffix.chars().count()).max(1);
        let base = requested.chars().take(base_len).collect::<String>();
        let candidate = format!("{base}{suffix}");
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn normalized_label(value: &str, empty_error: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(empty_error.to_string());
    }
    if value.chars().count() > 80 {
        return Err("名称过长".to_string());
    }
    Ok(value.to_string())
}

fn validate_group(store: &WardrobeStore, group_id: Option<&str>) -> Result<(), String> {
    if let Some(group_id) = group_id {
        if !store.groups.iter().any(|group| group.id == group_id) {
            return Err("没有找到目标栏目".to_string());
        }
    }
    Ok(())
}

fn find_entry_mut<'a>(
    store: &'a mut WardrobeStore,
    entry_id: &str,
) -> Result<&'a mut StoredEntry, String> {
    store
        .entries
        .iter_mut()
        .find(|entry| entry.id == entry_id)
        .ok_or_else(|| "没有找到橱窗装扮".to_string())
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}-{:x}-{:x}",
        now_ms(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn group_view(group: &StoredGroup) -> WardrobeGroupView {
    WardrobeGroupView {
        id: group.id.clone(),
        name: group.name.clone(),
        sort_order: group.sort_order,
    }
}

fn entry_view(
    entry: &StoredEntry,
    invalid_slots: Option<Vec<String>>,
    dye_validation: Option<WardrobeDyeValidation>,
    current: Option<&StoredSnapshot>,
    target: Option<&StoredSnapshot>,
) -> WardrobeEntryView {
    WardrobeEntryView {
        id: entry.id.clone(),
        name: entry.name.clone(),
        note: entry.note.clone(),
        group_id: entry.group_id.clone(),
        favorite: entry.favorite,
        snapshot: snapshot_view(&entry.snapshot, invalid_slots, dye_validation),
        created_at_ms: entry.created_at_ms,
        updated_at_ms: entry.updated_at_ms,
        last_used_at_ms: entry.last_used_at_ms,
        is_current: current.is_some_and(|snapshot| {
            snapshots_have_same_content(
                &snapshot.snapshot,
                snapshot.dye_state.as_ref(),
                &entry.snapshot.snapshot,
                entry.snapshot.dye_state.as_ref(),
            )
        }),
        is_target: target.is_some_and(|snapshot| {
            snapshots_have_same_content(
                &snapshot.snapshot,
                snapshot.dye_state.as_ref(),
                &entry.snapshot.snapshot,
                entry.snapshot.dye_state.as_ref(),
            )
        }),
    }
}

fn validate_snapshot(snapshot: &WardrobeSnapshotInput) -> Result<(), String> {
    if snapshot.playable_role_key.trim().is_empty()
        || snapshot.visual_family_key.trim().is_empty()
        || snapshot.resource_owner_key.trim().is_empty()
        || !matches!(
            snapshot
                .resource_generation
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "base" | "at"
        )
        || snapshot.profession_dir.trim().is_empty()
        || snapshot.path_family.trim().is_empty()
    {
        return Err("装扮快照为空或职业信息不完整".to_string());
    }
    if snapshot.items.iter().any(|item| {
        !OUTFIT_SLOTS.contains(&item.slot.trim())
            || match (&item.item_ref, &item.item_name) {
                (Some(item_ref), Some(item_name)) => {
                    item_ref.trim().is_empty()
                        || item_name.trim().is_empty()
                        || item.layer_count == 0
                }
                (None, None) => item.layer_count != 0,
                _ => true,
            }
    }) {
        return Err("装扮快照包含无效部位".to_string());
    }
    let mut slots = HashSet::with_capacity(snapshot.items.len());
    if snapshot
        .items
        .iter()
        .any(|item| !slots.insert(item.slot.trim()))
    {
        return Err("装扮快照包含重复部位".to_string());
    }
    if snapshot.items.len() != OUTFIT_SLOTS.len() {
        return Err("装扮快照必须显式保存九个部位".to_string());
    }
    if snapshot.weapon.as_ref().is_some_and(|weapon| {
        !weapon.slot.trim().eq_ignore_ascii_case("weapon")
            || match (&weapon.item_ref, &weapon.item_name) {
                (Some(item_ref), Some(item_name)) => {
                    item_ref.trim().is_empty()
                        || item_name.trim().is_empty()
                        || weapon.layer_count == 0
                }
                _ => true,
            }
    }) {
        return Err("装扮快照包含无效武器".to_string());
    }
    Ok(())
}

fn validate_active_compatibility(
    store: &WardrobeStore,
    role: WardrobeRole,
    snapshot: &WardrobeSnapshotInput,
) -> Result<(), String> {
    let other = match role {
        WardrobeRole::Current => store.target.as_ref(),
        WardrobeRole::Target => store.current.as_ref(),
    };
    let Some(other) = other else {
        return Ok(());
    };
    if !other
        .snapshot
        .visual_family_key
        .eq_ignore_ascii_case(&snapshot.visual_family_key)
        || !other
            .snapshot
            .resource_owner_key
            .eq_ignore_ascii_case(&snapshot.resource_owner_key)
        || !other
            .snapshot
            .resource_generation
            .eq_ignore_ascii_case(&snapshot.resource_generation)
    {
        return Err("当前穿戴与目标外观必须属于同一视觉族和资源代际".to_string());
    }
    Ok(())
}

fn snapshot_view(
    stored: &StoredSnapshot,
    invalid_slots: Option<Vec<String>>,
    dye_validation: Option<WardrobeDyeValidation>,
) -> WardrobeSnapshotView {
    let catalog_resource_status = match invalid_slots.as_ref() {
        None => "unchecked",
        Some(slots) if slots.is_empty() => "valid",
        Some(_) => "changed",
    };
    let mut dyed_slots = HashSet::new();
    let mut dyed_base_count = 0usize;
    let mut dyed_mask_count = 0usize;
    let mut dyed_color_count = 0usize;
    let mut representative_colors = Vec::new();
    if let Some(recipe) = baked_recipe(stored.dye_state.as_ref()) {
        for target in &recipe.targets {
            dyed_slots.insert(target.target.slot.as_str());
            match target.target.target_kind {
                DyeTargetKindV1::Base => dyed_base_count += 1,
                DyeTargetKindV1::Mask { .. } => dyed_mask_count += 1,
            }
            dyed_color_count = dyed_color_count.saturating_add(target.colors.len());
            for edit in &target.colors {
                let color = format!(
                    "#{:02x}{:02x}{:02x}",
                    edit.replacement_rgb[0], edit.replacement_rgb[1], edit.replacement_rgb[2]
                );
                if representative_colors.len() < 6 && !representative_colors.contains(&color) {
                    representative_colors.push(color);
                }
            }
        }
    }
    let has_dye = dyed_base_count + dyed_mask_count > 0;
    let recipe_revision =
        optional_dye_state_hash(stored.dye_state.as_ref()).map(|state_hash| hex_bytes(&state_hash));
    let dye_status = match (has_dye, dye_validation.as_ref()) {
        (false, _) => "none",
        (true, Some(validation)) => validation.status.as_str(),
        (true, None)
            if invalid_slots
                .as_ref()
                .is_some_and(|slots| !slots.is_empty()) =>
        {
            "changed"
        }
        (true, None) => "unchecked",
    };
    let resource_status = match dye_validation.as_ref().map(|validation| validation.status) {
        Some(
            WardrobeDyeValidationStatus::Changed
            | WardrobeDyeValidationStatus::Partial
            | WardrobeDyeValidationStatus::Unsupported,
        ) => "changed",
        Some(WardrobeDyeValidationStatus::Unchecked) if catalog_resource_status != "changed" => {
            "unchecked"
        }
        None if has_dye && catalog_resource_status != "changed" => "unchecked",
        _ => catalog_resource_status,
    };
    let invalid_slots = merge_invalid_slots(
        invalid_slots.unwrap_or_default(),
        dye_validation
            .map(|validation| validation.invalid_slots)
            .unwrap_or_default(),
    );
    WardrobeSnapshotView {
        playable_role_key: stored.snapshot.playable_role_key.clone(),
        visual_family_key: stored.snapshot.visual_family_key.clone(),
        resource_owner_key: stored.snapshot.resource_owner_key.clone(),
        resource_generation: stored.snapshot.resource_generation.clone(),
        profession_dir: stored.snapshot.profession_dir.clone(),
        path_family: stored.snapshot.path_family.clone(),
        items: stored
            .snapshot
            .items
            .iter()
            .map(|item| WardrobeSnapshotItemView {
                slot: item.slot.clone(),
                item_name: item.item_name.clone(),
                layer_count: item.layer_count,
            })
            .collect(),
        weapon: stored
            .snapshot
            .weapon
            .as_ref()
            .map(|weapon| WardrobeSnapshotItemView {
                slot: weapon.slot.clone(),
                item_name: weapon.item_name.clone(),
                layer_count: weapon.layer_count,
            }),
        preview_data_url: stored.snapshot.preview_data_url.clone(),
        saved_at_ms: stored.saved_at_ms,
        revision: stored.revision.clone(),
        recipe_revision,
        has_dye,
        dyed_slot_count: dyed_slots.len(),
        dyed_base_count,
        dyed_mask_count,
        dyed_color_count,
        representative_colors,
        dye_status: dye_status.to_string(),
        resource_status: resource_status.to_string(),
        invalid_slots,
    }
}

fn merge_invalid_slots(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    let mut slots = left
        .into_iter()
        .chain(right)
        .map(|slot| slot.trim().to_ascii_lowercase())
        .filter(|slot| OUTFIT_SLOTS.contains(&slot.as_str()))
        .collect::<HashSet<_>>();
    let mut ordered = OUTFIT_SLOTS
        .iter()
        .filter_map(|slot| slots.remove(*slot).then(|| (*slot).to_string()))
        .collect::<Vec<_>>();
    let mut unexpected = slots.into_iter().collect::<Vec<_>>();
    unexpected.sort();
    ordered.extend(unexpected);
    ordered
}

fn private_snapshot(stored: &StoredSnapshot) -> OutfitSnapshot {
    OutfitSnapshot {
        playable_role_key: stored.snapshot.playable_role_key.clone(),
        visual_family_key: stored.snapshot.visual_family_key.clone(),
        resource_owner_key: stored.snapshot.resource_owner_key.clone(),
        resource_generation: stored.snapshot.resource_generation.clone(),
        profession_dir: stored.snapshot.profession_dir.clone(),
        path_family: stored.snapshot.path_family.clone(),
        item_ids: stored
            .snapshot
            .items
            .iter()
            .filter_map(|item| item.item_ref.clone())
            .collect(),
    }
}

fn private_wardrobe_snapshot(stored: &StoredSnapshot) -> PrivateWardrobeSnapshot {
    PrivateWardrobeSnapshot {
        snapshot: stored.snapshot.clone(),
        dye_recipe: baked_recipe(stored.dye_state.as_ref()).cloned(),
        dye_state: stored.dye_state.clone(),
        saved_at_ms: stored.saved_at_ms,
        revision: stored.revision.clone(),
    }
}

fn new_stored_snapshot(
    snapshot: WardrobeSnapshotInput,
    dye_state: Option<PersistedDyeStateV2>,
    saved_at_ms: u64,
) -> StoredSnapshot {
    StoredSnapshot {
        snapshot,
        dye_state,
        saved_at_ms,
        revision: new_snapshot_revision(),
    }
}

fn new_snapshot_revision() -> String {
    new_id("snapshot")
}

fn persisted_state_from_recipe(dye_recipe: Option<DyeRecipeV1>) -> Option<PersistedDyeStateV2> {
    dye_recipe.map(|baked_recipe| PersistedDyeStateV2 {
        schema_version: DYE_STATE_SCHEMA_V2,
        baseline_recipe: Some(baked_recipe.clone()),
        baked_recipe,
        semantic_state_json: None,
    })
}

fn baked_recipe(dye_state: Option<&PersistedDyeStateV2>) -> Option<&DyeRecipeV1> {
    dye_state.map(|state| &state.baked_recipe)
}

fn canonicalize_recipe_for_snapshot(
    snapshot: &WardrobeSnapshotInput,
    recipe: DyeRecipeV1,
) -> Result<DyeRecipeV1, String> {
    let recipe = canonicalize_recipe(recipe)?;
    for target in &recipe.targets {
        if !wardrobe_snapshot_owns_dye_target(snapshot, target) {
            return Err("染色配方目标不属于当前装扮快照".to_string());
        }
    }
    Ok(recipe)
}

fn canonicalize_optional_dye_state(
    snapshot: &WardrobeSnapshotInput,
    dye_state: Option<PersistedDyeStateV2>,
) -> Result<Option<PersistedDyeStateV2>, String> {
    let Some(mut dye_state) = dye_state else {
        return Ok(None);
    };
    if dye_state.schema_version != DYE_STATE_SCHEMA_V2 {
        return Err("染色状态版本不受支持".to_string());
    }
    dye_state.baked_recipe = canonicalize_recipe_for_snapshot(snapshot, dye_state.baked_recipe)?;
    dye_state.baseline_recipe = dye_state
        .baseline_recipe
        .map(|recipe| canonicalize_recipe_for_snapshot(snapshot, recipe))
        .transpose()?
        .filter(|recipe| !recipe.is_empty());
    if let Some(semantic_state_json) = dye_state.semantic_state_json.as_deref() {
        if semantic_state_json.len() > MAX_SEMANTIC_STATE_JSON_BYTES {
            return Err("染色语义状态过大".to_string());
        }
        serde_json::from_str::<serde_json::Value>(semantic_state_json)
            .map_err(|_| "染色语义状态 JSON 无效".to_string())?;
    }
    if dye_state.baked_recipe.is_empty()
        && dye_state.baseline_recipe.is_none()
        && dye_state.semantic_state_json.is_none()
    {
        return Ok(None);
    }
    Ok(Some(dye_state))
}

fn optional_dye_state_hash(dye_state: Option<&PersistedDyeStateV2>) -> Option<[u8; 32]> {
    dye_state.map(|state| {
        let mut hasher = Sha256::new();
        hasher.update(state.schema_version.to_le_bytes());
        hasher.update(state.baked_recipe.recipe_hash);
        match state.baseline_recipe.as_ref() {
            Some(baseline_recipe) => {
                hasher.update([1]);
                hasher.update(baseline_recipe.recipe_hash);
            }
            None => hasher.update([0]),
        }
        match state.semantic_state_json.as_deref() {
            Some(semantic_state_json) => {
                hasher.update([1]);
                hasher.update((semantic_state_json.len() as u64).to_le_bytes());
                hasher.update(semantic_state_json.as_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.finalize().into()
    })
}

fn optional_baked_recipe_hash(dye_state: Option<&PersistedDyeStateV2>) -> Option<[u8; 32]> {
    dye_state.and_then(|state| {
        (!state.baked_recipe.is_empty()).then_some(state.baked_recipe.recipe_hash)
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn normalize_share_package(package: &mut WardrobeSharePackageV5) -> Result<(), String> {
    for entry in &mut package.entries {
        entry.dye_state = canonicalize_optional_dye_state(&entry.snapshot, entry.dye_state.take())?;
    }
    Ok(())
}

fn normalize_store(store: &mut WardrobeStore) -> Result<(), String> {
    validate_store_capacity(store)?;
    if let Some(snapshot) = store.current.as_mut() {
        normalize_stored_snapshot(snapshot)?;
    }
    if let Some(snapshot) = store.target.as_mut() {
        normalize_stored_snapshot(snapshot)?;
    }
    for entry in &mut store.entries {
        normalize_stored_snapshot(&mut entry.snapshot)?;
    }
    Ok(())
}

fn ensure_wardrobe_entry_growth(existing: usize, additional: usize) -> Result<(), String> {
    if existing
        .checked_add(additional)
        .is_none_or(|total| total > MAX_WARDROBE_ENTRIES)
    {
        return Err(format!("时装橱窗容量已达 {MAX_WARDROBE_ENTRIES} 套上限"));
    }
    Ok(())
}

fn snapshot_item_reference_count(snapshot: &WardrobeSnapshotInput) -> usize {
    snapshot
        .items
        .iter()
        .filter(|item| {
            item.item_ref
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        })
        .count()
        + usize::from(
            snapshot
                .weapon
                .as_ref()
                .and_then(|item| item.item_ref.as_deref())
                .is_some_and(|value| !value.trim().is_empty()),
        )
}

fn validate_store_capacity(store: &WardrobeStore) -> Result<(), String> {
    ensure_wardrobe_entry_growth(0, store.entries.len())?;
    let referenced_item_count = store
        .current
        .iter()
        .chain(store.target.iter())
        .map(|stored| snapshot_item_reference_count(&stored.snapshot))
        .chain(
            store
                .entries
                .iter()
                .map(|entry| snapshot_item_reference_count(&entry.snapshot.snapshot)),
        )
        .try_fold(0usize, |total, count| total.checked_add(count))
        .ok_or_else(|| "时装橱窗引用数量溢出".to_string())?;
    if referenced_item_count > MAX_WARDROBE_CATALOG_CONTEXT_ITEM_IDS {
        return Err(format!(
            "时装橱窗物品引用超过 {MAX_WARDROBE_CATALOG_CONTEXT_ITEM_IDS} 条安全上限"
        ));
    }
    serialized_store_size_with_limit(store, MAX_WARDROBE_STORE_BYTES)?;
    Ok(())
}

#[derive(Debug)]
struct BoundedByteCounter {
    count: usize,
    limit: usize,
    exceeded: bool,
}

impl BoundedByteCounter {
    fn new(limit: usize) -> Self {
        Self {
            count: 0,
            limit,
            exceeded: false,
        }
    }
}

impl Extend<u8> for BoundedByteCounter {
    fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
        for _ in iter {
            if self.count >= self.limit {
                self.exceeded = true;
                return;
            }
            self.count += 1;
        }
    }
}

fn serialized_size_with_limit<T: Serialize + ?Sized>(
    value: &T,
    limit: usize,
    too_large_error: &str,
) -> Result<usize, String> {
    let counter = postcard::to_extend(value, BoundedByteCounter::new(limit))
        .map_err(|_| "计算时装橱窗编码大小失败".to_string())?;
    if counter.exceeded {
        return Err(too_large_error.to_string());
    }
    Ok(counter.count)
}

fn serialized_store_size_with_limit(store: &WardrobeStore, limit: usize) -> Result<usize, String> {
    serialized_size_with_limit(
        &WardrobeFileV5Ref {
            schema_version: FILE_SCHEMA_V5,
            store,
        },
        limit,
        "时装橱窗内容过大",
    )
}

fn ensure_import_serialized_growth(
    store: &WardrobeStore,
    package: &WardrobeSharePackageV5,
) -> Result<(), String> {
    let current_size = serialized_store_size_with_limit(store, MAX_WARDROBE_STORE_BYTES)?;
    let incoming_size =
        serialized_size_with_limit(package, MAX_WARDROBE_STORE_BYTES, "橱窗分享内容过大")?;
    let imported_overhead = package
        .entries
        .len()
        .checked_mul(IMPORTED_ENTRY_STORAGE_OVERHEAD_BYTES)
        .ok_or_else(|| "时装橱窗容量计算溢出".to_string())?;
    let projected_size = current_size
        .checked_add(incoming_size)
        .and_then(|size| size.checked_add(imported_overhead))
        .ok_or_else(|| "时装橱窗容量计算溢出".to_string())?;
    if projected_size > MAX_WARDROBE_STORE_BYTES {
        return Err(format!(
            "时装橱窗内容将超过 {} MiB 安全上限",
            MAX_WARDROBE_STORE_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}

fn normalize_stored_snapshot(stored: &mut StoredSnapshot) -> Result<(), String> {
    if stored.snapshot.playable_role_key.trim().is_empty()
        || stored.snapshot.visual_family_key.trim().is_empty()
        || stored.snapshot.resource_owner_key.trim().is_empty()
        || stored.snapshot.resource_generation.trim().is_empty()
    {
        let (playable_role_key, visual_family_key, resource_owner_key, resource_generation) =
            legacy_role_binding(
                &stored.snapshot.profession_dir,
                &stored.snapshot.path_family,
            )
            .ok_or_else(|| "旧版装扮快照无法迁移到显式视觉族模型".to_string())?;
        stored.snapshot.playable_role_key = playable_role_key;
        stored.snapshot.visual_family_key = visual_family_key;
        stored.snapshot.resource_owner_key = resource_owner_key;
        stored.snapshot.resource_generation = resource_generation;
    }
    stored.dye_state = canonicalize_optional_dye_state(&stored.snapshot, stored.dye_state.take())?;
    if stored.revision.trim().is_empty() {
        return Err("时装橱窗快照版本无效".to_string());
    }
    Ok(())
}

fn snapshots_have_same_content(
    left: &WardrobeSnapshotInput,
    left_dye_state: Option<&PersistedDyeStateV2>,
    right: &WardrobeSnapshotInput,
    right_dye_state: Option<&PersistedDyeStateV2>,
) -> bool {
    if !left
        .playable_role_key
        .eq_ignore_ascii_case(&right.playable_role_key)
        || !left
            .visual_family_key
            .eq_ignore_ascii_case(&right.visual_family_key)
    {
        return false;
    }
    OUTFIT_SLOTS
        .iter()
        .all(|slot| snapshot_item_ref(left, slot) == snapshot_item_ref(right, slot))
        && snapshot_weapon_item_ref(left) == snapshot_weapon_item_ref(right)
        && optional_baked_recipe_hash(left_dye_state) == optional_baked_recipe_hash(right_dye_state)
}

fn ensure_unique_entry_content(
    store: &WardrobeStore,
    excluded_entry_id: Option<&str>,
    snapshot: &WardrobeSnapshotInput,
    dye_state: Option<&PersistedDyeStateV2>,
) -> Result<(), String> {
    if store.entries.iter().any(|entry| {
        excluded_entry_id != Some(entry.id.as_str())
            && snapshots_have_same_content(
                &entry.snapshot.snapshot,
                entry.snapshot.dye_state.as_ref(),
                snapshot,
                dye_state,
            )
    }) {
        return Err(DUPLICATE_ENTRY_ERROR.to_string());
    }
    Ok(())
}

fn snapshot_item_ref<'a>(snapshot: &'a WardrobeSnapshotInput, slot: &str) -> Option<&'a str> {
    snapshot
        .items
        .iter()
        .find(|item| item.slot.trim() == slot)
        .and_then(|item| item.item_ref.as_deref())
        .map(str::trim)
}

fn snapshot_weapon_item_ref(snapshot: &WardrobeSnapshotInput) -> Option<&str> {
    snapshot
        .weapon
        .as_ref()
        .and_then(|weapon| weapon.item_ref.as_deref())
        .map(str::trim)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn write_store(path: &PathBuf, store: &WardrobeStore) -> Result<(), String> {
    validate_store_capacity(store)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "创建时装橱窗目录失败".to_string())?;
    }
    let file_v5 = WardrobeFileV5Ref {
        schema_version: FILE_SCHEMA_V5,
        store,
    };
    let plain = postcard::to_allocvec(&file_v5).map_err(|_| "编码时装橱窗失败".to_string())?;
    let encrypted = protect_data(&plain)?;
    let mut file = Vec::with_capacity(FILE_MAGIC_V5.len() + encrypted.len());
    file.extend_from_slice(FILE_MAGIC_V5);
    file.extend_from_slice(&encrypted);
    if file.len() > MAX_WARDROBE_STORE_BYTES {
        return Err("加密后的时装橱窗内容过大".to_string());
    }
    replace_with_backup(path, &file)
}

fn read_store(path: &PathBuf) -> Result<WardrobeStore, WardrobeLoadError> {
    let file = read_file_with_limit(path, MAX_WARDROBE_READ_BYTES, "时装橱窗文件过大")
        .map_err(WardrobeLoadError::PreservePrimary)?;
    decode_store(&file)
}

fn read_file_with_limit(
    path: &Path,
    limit: usize,
    too_large_error: &str,
) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|_| "读取文件失败".to_string())?;
    if metadata.len() > limit as u64 {
        return Err(too_large_error.to_string());
    }
    let file = fs::read(path).map_err(|_| "读取文件失败".to_string())?;
    if file.len() > limit {
        return Err(too_large_error.to_string());
    }
    Ok(file)
}

fn backup_path(path: &PathBuf) -> PathBuf {
    path.with_extension("bak")
}

fn restore_primary_from_backup(path: &PathBuf, backup: &PathBuf) -> Result<(), String> {
    let encrypted_file =
        read_file_with_limit(backup, MAX_WARDROBE_READ_BYTES, "时装橱窗备份文件过大")
            .map_err(|_| "读取时装橱窗备份失败".to_string())?;
    decode_store(&encrypted_file).map_err(WardrobeLoadError::into_message)?;
    let temporary = path.with_extension("tmp");
    let mut output = fs::File::create(&temporary).map_err(|_| "修复时装橱窗失败".to_string())?;
    output
        .write_all(&encrypted_file)
        .and_then(|_| output.sync_all())
        .map_err(|_| "修复时装橱窗失败".to_string())?;
    drop(output);
    if path.exists() {
        replace_file_without_backup(path, &temporary).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    } else {
        fs::rename(&temporary, path).map_err(|_| "修复时装橱窗失败".to_string())
    }
}

fn replace_with_backup(path: &PathBuf, encrypted_file: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    let mut output = fs::File::create(&temporary).map_err(|_| "保存时装橱窗失败".to_string())?;
    output
        .write_all(encrypted_file)
        .and_then(|_| output.sync_all())
        .map_err(|_| "保存时装橱窗失败".to_string())?;
    drop(output);

    if !path.exists() {
        return fs::rename(&temporary, path).map_err(|_| "提交时装橱窗失败".to_string());
    }

    let backup = backup_path(path);
    if backup.exists() {
        fs::remove_file(&backup).map_err(|_| "更新时装橱窗备份失败".to_string())?;
    }
    replace_file(path, &temporary, &backup).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

fn decode_store(file: &[u8]) -> Result<WardrobeStore, WardrobeLoadError> {
    if file.len() > MAX_WARDROBE_READ_BYTES {
        return Err(WardrobeLoadError::PreservePrimary(
            "时装橱窗文件过大".to_string(),
        ));
    }
    let magic = if file.starts_with(FILE_MAGIC_V5) {
        FILE_MAGIC_V5.as_slice()
    } else if file.starts_with(FILE_MAGIC_V4) {
        FILE_MAGIC_V4.as_slice()
    } else if file.starts_with(FILE_MAGIC_V3) {
        FILE_MAGIC_V3.as_slice()
    } else if file.starts_with(FILE_MAGIC_V2) {
        FILE_MAGIC_V2.as_slice()
    } else if file.starts_with(FILE_MAGIC_V1) {
        FILE_MAGIC_V1.as_slice()
    } else {
        return Err(WardrobeLoadError::RecoverableCorruption(
            "时装橱窗文件格式无效".to_string(),
        ));
    };
    if file.len() <= magic.len() {
        return Err(WardrobeLoadError::RecoverableCorruption(
            "时装橱窗文件格式无效".to_string(),
        ));
    }
    let plain =
        unprotect_data(&file[magic.len()..]).map_err(WardrobeLoadError::RecoverableCorruption)?;
    if plain.len() > MAX_WARDROBE_READ_BYTES {
        return Err(WardrobeLoadError::PreservePrimary(
            "时装橱窗解密内容过大".to_string(),
        ));
    }
    let mut store = if magic == FILE_MAGIC_V5 {
        let file_v5 = postcard::from_bytes::<WardrobeFileV5>(&plain)
            .map_err(|_| WardrobeLoadError::PreservePrimary("解析时装橱窗失败".to_string()))?;
        if file_v5.schema_version != FILE_SCHEMA_V5 {
            return Err(WardrobeLoadError::PreservePrimary(
                "时装橱窗版本不受支持".to_string(),
            ));
        }
        file_v5.store
    } else if magic == FILE_MAGIC_V4 {
        let file_v4 = postcard::from_bytes::<WardrobeFileV4>(&plain)
            .map_err(|_| WardrobeLoadError::PreservePrimary("解析时装橱窗失败".to_string()))?;
        if file_v4.schema_version != FILE_SCHEMA_V4 {
            return Err(WardrobeLoadError::PreservePrimary(
                "时装橱窗版本不受支持".to_string(),
            ));
        }
        file_v4.store.into()
    } else if magic == FILE_MAGIC_V3 {
        let file_v3 = postcard::from_bytes::<WardrobeFileV3>(&plain)
            .map_err(|_| WardrobeLoadError::PreservePrimary("解析时装橱窗失败".to_string()))?;
        if file_v3.schema_version != FILE_SCHEMA_V3 {
            return Err(WardrobeLoadError::PreservePrimary(
                "时装橱窗版本不受支持".to_string(),
            ));
        }
        file_v3.store.into()
    } else if magic == FILE_MAGIC_V2 {
        let file_v2 = postcard::from_bytes::<WardrobeFileV2>(&plain)
            .map_err(|_| WardrobeLoadError::PreservePrimary("解析时装橱窗失败".to_string()))?;
        if file_v2.schema_version != FILE_SCHEMA_V2 {
            return Err(WardrobeLoadError::PreservePrimary(
                "时装橱窗版本不受支持".to_string(),
            ));
        }
        file_v2.store.into()
    } else {
        postcard::from_bytes::<LegacyWardrobeStoreV1>(&plain)
            .map(WardrobeStore::from)
            .map_err(|_| WardrobeLoadError::PreservePrimary("解析时装橱窗失败".to_string()))?
    };
    normalize_store(&mut store).map_err(WardrobeLoadError::PreservePrimary)?;
    Ok(store)
}

#[cfg(windows)]
fn replace_file(path: &PathBuf, temporary: &PathBuf, backup: &PathBuf) -> Result<(), String> {
    replace_file_impl(path, temporary, Some(backup))
}

#[cfg(windows)]
fn replace_file_without_backup(path: &PathBuf, temporary: &PathBuf) -> Result<(), String> {
    replace_file_impl(path, temporary, None)
}

#[cfg(windows)]
fn replace_file_impl(
    path: &PathBuf,
    temporary: &PathBuf,
    backup: Option<&PathBuf>,
) -> Result<(), String> {
    use libloading::{Library, Symbol};
    use std::os::windows::ffi::OsStrExt;

    type ReplaceFile = unsafe extern "system" fn(
        *const u16,
        *const u16,
        *const u16,
        u32,
        *const c_void,
        *const c_void,
    ) -> i32;

    let wide = |value: &PathBuf| {
        value
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let path = wide(path);
    let temporary = wide(temporary);
    let backup = backup.map(wide);
    let library =
        unsafe { Library::new("kernel32.dll") }.map_err(|_| "提交时装橱窗失败".to_string())?;
    let replace: Symbol<ReplaceFile> =
        unsafe { library.get(b"ReplaceFileW") }.map_err(|_| "提交时装橱窗失败".to_string())?;
    let ok = unsafe {
        replace(
            path.as_ptr(),
            temporary.as_ptr(),
            backup
                .as_ref()
                .map_or(std::ptr::null(), |value| value.as_ptr()),
            0x00000002,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        return Err("提交时装橱窗失败".to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(path: &PathBuf, temporary: &PathBuf, backup: &PathBuf) -> Result<(), String> {
    fs::copy(path, backup).map_err(|_| "更新时装橱窗备份失败".to_string())?;
    fs::remove_file(path).map_err(|_| "提交时装橱窗失败".to_string())?;
    fs::rename(temporary, path).map_err(|_| "提交时装橱窗失败".to_string())
}

#[cfg(not(windows))]
fn replace_file_without_backup(path: &PathBuf, temporary: &PathBuf) -> Result<(), String> {
    fs::remove_file(path).map_err(|_| "修复时装橱窗失败".to_string())?;
    fs::rename(temporary, path).map_err(|_| "修复时装橱窗失败".to_string())
}

#[cfg(windows)]
fn protect_data(input: &[u8]) -> Result<Vec<u8>, String> {
    crate::local_protection::protect_for_current_user(input, &[])
        .map_err(|_| "加密时装橱窗失败".to_string())
}

#[cfg(windows)]
fn unprotect_data(input: &[u8]) -> Result<Vec<u8>, String> {
    crate::local_protection::unprotect_for_current_user(input, &[])
        .map_err(|_| "解密时装橱窗失败".to_string())
}

#[cfg(not(windows))]
fn protect_data(_input: &[u8]) -> Result<Vec<u8>, String> {
    Err("时装橱窗加密仅支持 Windows".to_string())
}

#[cfg(not(windows))]
fn unprotect_data(_input: &[u8]) -> Result<Vec<u8>, String> {
    Err("时装橱窗解密仅支持 Windows".to_string())
}
