use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DyeBatchColorReplacement {
    pub color_index: u16,
    pub rgb: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DyeBatchTargetReplacement {
    pub target_id: super::session::DyeTargetId,
    pub palette_index: u16,
    pub colors: Vec<DyeBatchColorReplacement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DyeBatchMutation {
    pub targets: Vec<DyeBatchTargetReplacement>,
    #[serde(default)]
    pub semantic_state: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeRecipeV1 {
    pub schema_version: u16,
    pub targets: Vec<DyeTargetRecipeV1>,
    pub recipe_hash: [u8; 32],
}

impl Default for DyeRecipeV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            targets: Vec::new(),
            recipe_hash: [0; 32],
        }
    }
}

impl DyeRecipeV1 {
    pub fn is_empty(&self) -> bool {
        self.targets.iter().all(|target| target.colors.is_empty())
    }
}

/// Versioned dye payload stored by wardrobe snapshots.
///
/// `semantic_state_json` intentionally uses canonical JSON text instead of
/// `serde_json::Value`: postcard is not required to implement
/// `deserialize_any`, while the text form stays forward-compatible and can be
/// validated before it is exposed to the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDyeStateV2 {
    pub schema_version: u16,
    pub baked_recipe: DyeRecipeV1,
    pub baseline_recipe: Option<DyeRecipeV1>,
    pub semantic_state_json: Option<String>,
}

impl PersistedDyeStateV2 {
    pub fn from_legacy_recipe(baked_recipe: DyeRecipeV1) -> Self {
        Self {
            schema_version: 2,
            baseline_recipe: Some(baked_recipe.clone()),
            baked_recipe,
            semantic_state_json: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeTargetRecipeV1 {
    pub target: PersistedDyeTargetRefV1,
    pub palette_index: u16,
    pub source: DyeSourceFingerprintV1,
    pub colors: Vec<DyeColorEditV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyeTargetKindV1 {
    Base,
    Mask { number: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedDyeTargetRefV1 {
    pub slot: String,
    pub item_ref: String,
    pub layer_tag: String,
    pub target_kind: DyeTargetKindV1,
    pub private_entry_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeSourceFingerprintV1 {
    pub img_version: u16,
    pub frame_count: u32,
    pub palette_count: u16,
    pub color_count: u16,
    pub palette_sha256: [u8; 32],
    pub entry_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeColorEditV1 {
    pub color_index: u16,
    pub expected_rgba: [u8; 4],
    pub replacement_rgb: [u8; 3],
}
