use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use avatar_data::{
    bucket_for_item_id, write_catalog_index, AvatarItemRecord, AvatarLayerRecord, AvatarRoleRecord,
    AvatarSetRecord, AvatarVisualFamilyRecord, CatalogSource, PagedDataStore, BUCKET_COUNT,
};
use csv::{ReaderBuilder, StringRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize)]
struct PlayableRoleBinding {
    playable_role_key: String,
    visual_family_key: String,
}

#[derive(Debug, Clone, Serialize)]
struct VisualFamily {
    visual_family_key: String,
    resource_owner_key: String,
    resource_generation: String,
    default_skin_img: String,
}

#[derive(Debug, Clone, Serialize)]
struct CatalogItem {
    item_id: String,
    item_name: String,
    slot: String,
    profession_dir: String,
    playable_roles: Vec<String>,
    path_family: String,
    resource_kind: String,
    icon_img: String,
    layer_count: usize,
    layer_counts_by_visual_family: BTreeMap<String, usize>,
    hide_equipment_types: Vec<String>,
    hide_equipment_type_enums: Vec<u16>,
    enable_dye_mask1: bool,
    enable_dye_mask2: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CatalogSet {
    set_id: String,
    set_name: String,
    source: String,
    playable_roles: Vec<String>,
    member_ids: Vec<String>,
    member_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LayerRow {
    item_id: String,
    item_name: String,
    slot: String,
    visual_family_key: String,
    layer_order: i32,
    equipment_type_enum: u16,
    layer_sequence: u32,
    layer_tag: String,
    variation: String,
    palette_index: usize,
    visual_img: String,
    status: String,
}

type LayerBuckets = [BTreeMap<String, Vec<LayerRow>>; BUCKET_COUNT];
type LayerReadResult = (LayerBuckets, HashMap<String, usize>, usize);

#[derive(Debug, Clone)]
struct ItemReadResult {
    items: Vec<CatalogItem>,
    allowed_item_ids: HashSet<String>,
    item_names: HashMap<String, String>,
    skin_seeds: Vec<SkinLayerSeed>,
}

#[derive(Debug, Clone)]
struct SkinLayerSeed {
    item_id: String,
    item_name: String,
    slot: String,
    visual_family_key: String,
    variations: String,
    equipment_type_enum: u16,
}

#[derive(Debug, Deserialize)]
struct SummaryManifest {
    schema_version: u32,
    avatar_data_contract: SummaryDataContract,
}

#[derive(Debug, Deserialize)]
struct SummaryDataContract {
    schema_version: u32,
    files: HashMap<String, SummaryFileContract>,
}

#[derive(Debug, Deserialize)]
struct SummaryFileContract {
    sha256: String,
    rows: u64,
    columns: Vec<String>,
    required_columns: Vec<String>,
}

const REQUIRED_SUMMARY_FILES: &[&str] = &[
    "avatar_weapon_items.csv",
    "avatar_weapon_layers.csv",
    "avatar_layer_visual_imgs.csv",
    "avatar_box_sets.csv",
];

const SUMMARY_SCHEMA_VERSION: u32 = 5;

const PLAYABLE_ROLE_MAPPINGS: &[(&str, &str)] = &[
    ("archer", "archer_base"),
    ("at_fighter", "fighter_at"),
    ("at_gunner", "gunner_at"),
    ("at_mage", "mage_at"),
    ("at_priest", "priest_at"),
    ("at_swordman", "swordman_at"),
    ("creator_mage", "mage_base"),
    ("demonic_swordman", "swordman_base"),
    ("demoniclancer", "demoniclancer_base"),
    ("fighter", "fighter_base"),
    ("gunblader", "gunblader_base"),
    ("gunner", "gunner_base"),
    ("imperialknight", "imperialknight_base"),
    ("knight", "knight_base"),
    ("mage", "mage_base"),
    ("priest", "priest_base"),
    ("swordman", "swordman_base"),
    ("thief", "thief_base"),
];

const VISUAL_FAMILY_MAPPINGS: &[(&str, &str, &str, &str)] = &[
    (
        "archer_base",
        "archer",
        "base",
        "Character/Archer/Equipment/Avatar/skin/ac_body0000.img",
    ),
    (
        "demoniclancer_base",
        "demoniclancer",
        "base",
        "Character/DemonicLancer/Equipment/Avatar/skin/dl_body0000.img",
    ),
    (
        "fighter_at",
        "fighter",
        "at",
        "Character/fighter/ATEquipment/Avatar/skin/fm_body0000.img",
    ),
    (
        "fighter_base",
        "fighter",
        "base",
        "Character/fighter/Equipment/Avatar/skin/ft_body0000.img",
    ),
    (
        "gunblader_base",
        "gunblader",
        "base",
        "Character/GunBlader/Equipment/Avatar/skin/gb_body0000.img",
    ),
    (
        "gunner_at",
        "gunner",
        "at",
        "Character/gunner/ATEquipment/Avatar/skin/gg_body0000.img",
    ),
    (
        "gunner_base",
        "gunner",
        "base",
        "Character/gunner/Equipment/Avatar/skin/gn_body0000.img",
    ),
    (
        "imperialknight_base",
        "imperialknight",
        "base",
        "Character/ImperialKnight/Equipment/Avatar/skin/IK_body0000.img",
    ),
    (
        "knight_base",
        "knight",
        "base",
        "Character/Knight/Equipment/Avatar/skin/kn_body0000.img",
    ),
    (
        "mage_at",
        "mage",
        "at",
        "Character/mage/ATEquipment/Avatar/skin/mm_body0000.img",
    ),
    (
        "mage_base",
        "mage",
        "base",
        "Character/mage/Equipment/Avatar/skin/mg_body0000.img",
    ),
    (
        "priest_at",
        "priest",
        "at",
        "Character/priest/ATEquipment/Avatar/skin/pg_body0000.img",
    ),
    (
        "priest_base",
        "priest",
        "base",
        "Character/priest/Equipment/Avatar/skin/pr_body0000.img",
    ),
    (
        "swordman_at",
        "swordman",
        "at",
        "Character/swordman/ATEquipment/Avatar/skin/sg_body0000.img",
    ),
    (
        "swordman_base",
        "swordman",
        "base",
        "Character/swordman/Equipment/Avatar/skin/sm_body0000.img",
    ),
    (
        "thief_base",
        "thief",
        "base",
        "Character/thief/Equipment/Avatar/skin/th_body0000.img",
    ),
];

const REQUIRED_ITEM_COLUMNS: &[&str] = &[
    "item_id",
    "item_name_cn",
    "pvf_path",
    "profession_dir",
    "profession_keys",
    "path_family",
    "slot_dir",
    "equipment_type",
    "equipment_type_enum",
    "hide_equipment_types",
    "hide_equipment_type_enums",
    "enable_dye_mask1",
    "enable_dye_mask2",
    "variations",
    "implicit_variation_visual",
    "icon_img",
    "layer_count",
];

const REQUIRED_PACKED_ITEM_COLUMNS: &[&str] = &[
    "item_id",
    "item_name_cn",
    "pvf_path",
    "profession_dir",
    "profession_keys",
    "path_family",
    "slot_dir",
    "equipment_type",
    "equipment_type_enum",
    "hide_equipment_types",
    "hide_equipment_type_enums",
    "enable_dye_mask1",
    "enable_dye_mask2",
    "variations",
    "icon_img",
    "layer_count",
];
const REQUIRED_ITEM_KIND_COLUMNS: &[&str] = &["item_kind"];
const REQUIRED_RAW_LAYER_COLUMNS: &[&str] = &[
    "item_id",
    "pvf_path",
    "equipment_type",
    "equipment_type_enum",
    "animation_job",
    "visual_profession_key",
    "layer_sequence",
    "layer_variation",
    "layer_tag",
];
const REQUIRED_LAYER_COLUMNS: &[&str] = &[
    "item_id",
    "item_name_cn",
    "pvf_path",
    "visual_profession_key",
    "slot_dir",
    "equipment_type",
    "equipment_type_enum",
    "layer_sequence",
    "layer_variation",
    "layer_tag",
    "variation",
    "palette_index",
    "visual_img",
    "visual_img_status",
];
const REQUIRED_VISUAL_CONTRACT_COLUMNS: &[&str] = &[
    "item_id",
    "item_name_cn",
    "pvf_path",
    "profession_dir",
    "visual_profession_key",
    "slot_dir",
    "equipment_type",
    "equipment_type_enum",
    "layer_sequence",
    "layer_variation",
    "layer_tag",
    "variation",
    "palette_index",
    "visual_img",
    "visual_img_status",
];
const REQUIRED_SET_COLUMNS: &[&str] = &[
    "box_item_id",
    "box_name_cn",
    "selection_name_cn",
    "profession_keys",
    "member_ids",
    "member_names",
];

fn playable_role_bindings() -> Vec<PlayableRoleBinding> {
    PLAYABLE_ROLE_MAPPINGS
        .iter()
        .map(
            |(playable_role_key, visual_family_key)| PlayableRoleBinding {
                playable_role_key: (*playable_role_key).to_string(),
                visual_family_key: (*visual_family_key).to_string(),
            },
        )
        .collect()
}

fn visual_families() -> Vec<VisualFamily> {
    VISUAL_FAMILY_MAPPINGS
        .iter()
        .map(
            |(visual_family_key, resource_owner_key, resource_generation, default_skin_img)| {
                VisualFamily {
                    visual_family_key: (*visual_family_key).to_string(),
                    resource_owner_key: (*resource_owner_key).to_string(),
                    resource_generation: (*resource_generation).to_string(),
                    default_skin_img: (*default_skin_img).to_string(),
                }
            },
        )
        .collect()
}

fn visual_family_for_playable_role(playable_role_key: &str) -> Option<&'static str> {
    PLAYABLE_ROLE_MAPPINGS.iter().find_map(|(role, family)| {
        role.eq_ignore_ascii_case(playable_role_key)
            .then_some(*family)
    })
}

fn visual_family_definition(visual_family_key: &str) -> Option<VisualFamily> {
    VISUAL_FAMILY_MAPPINGS
        .iter()
        .find_map(|(family, owner, generation, default_skin_img)| {
            family
                .eq_ignore_ascii_case(visual_family_key)
                .then(|| VisualFamily {
                    visual_family_key: (*family).to_string(),
                    resource_owner_key: (*owner).to_string(),
                    resource_generation: (*generation).to_string(),
                    default_skin_img: (*default_skin_img).to_string(),
                })
        })
}

fn require_known_playable_roles(
    roles: Vec<String>,
    field_name: &str,
    path: &Path,
    row: usize,
) -> Result<Vec<String>, String> {
    for role in &roles {
        if visual_family_for_playable_role(role).is_none() {
            return Err(format!(
                "unknown {field_name} value {role:?} in {} row {row}: no explicit visual_family mapping",
                path.display()
            ));
        }
    }
    Ok(roles)
}

fn client_equipment_type_enum(tag: &str) -> Option<u16> {
    match tag {
        "[hat avatar]" => Some(0),
        "[hair avatar]" => Some(1),
        "[face avatar]" => Some(2),
        "[coat avatar]" => Some(3),
        "[pants avatar]" => Some(4),
        "[shoes avatar]" => Some(5),
        "[breast avatar]" => Some(6),
        "[waist avatar]" => Some(7),
        "[skin avatar]" => Some(8),
        "[aurora avatar]" => Some(9),
        "[weapon avatar]" => Some(10),
        "[aura skin avatar]" => Some(11),
        "[weapon]" => Some(12),
        "[title name]" => Some(13),
        "[coat]" => Some(14),
        "[shoulder]" => Some(15),
        "[pants]" => Some(16),
        "[shoes]" => Some(17),
        "[waist]" => Some(18),
        "[amulet]" => Some(19),
        "[wrist]" => Some(20),
        "[ring]" => Some(21),
        "[support]" => Some(22),
        "[magic stone]" => Some(23),
        "[support weapon]" => Some(24),
        "[earring]" => Some(25),
        "[creature]" => Some(26),
        "[artifact red]" => Some(27),
        "[artifact blue]" => Some(28),
        "[artifact green]" => Some(29),
        "[name tag]" => Some(30),
        "[charm]" => Some(31),
        "[flag]" => Some(32),
        "[creature skin]" => Some(33),
        "[talisman]" => Some(34),
        "[talisman addition]" => Some(35),
        "[talisman third]" => Some(36),
        "[odd]" => Some(37),
        "[primer]" => Some(38),
        "[oath]" => Some(49),
        "[skill coat]" => Some(51),
        "[amalgamation stone]" => Some(52),
        _ => None,
    }
}

const APPROVED_MISSING_EQUIPMENT_TYPE_ITEM_ID: &str = "417600061";
const APPROVED_MISSING_EQUIPMENT_TYPE_PVF_PATH: &str =
    "equipment/character/archer/avatar/weapon/chn_417600061.equ";
const APPROVED_MISSING_EQUIPMENT_TYPE_LAYERS: [(i32, &str); 9] = [
    (2050, "[ac_lbowa]"),
    (650, "[ac_lbowb]"),
    (2790, "[ac_lbowc]"),
    (200, "[ac_lbowd]"),
    (-200, "[ac_lbowg]"),
    (2790, "[ac_cbowc]"),
    (200, "[ac_cbowd]"),
    (-200, "[ac_cbowg]"),
    (1350, "[ac_cbowx]"),
];

fn is_approved_missing_equipment_type_layer(
    item_id: &str,
    slot: &str,
    pvf_path: &str,
    sequence: u32,
    layer_variation: &str,
    layer_tag: &str,
) -> bool {
    if item_id != APPROVED_MISSING_EQUIPMENT_TYPE_ITEM_ID
        || slot != "weapon"
        || pvf_path != APPROVED_MISSING_EQUIPMENT_TYPE_PVF_PATH
    {
        return false;
    }
    let Some(&(expected_variation, expected_tag)) = sequence
        .checked_sub(1)
        .and_then(|index| APPROVED_MISSING_EQUIPMENT_TYPE_LAYERS.get(index as usize))
    else {
        return false;
    };
    layer_variation.trim().parse::<i32>() == Ok(expected_variation)
        && layer_tag.trim() == expected_tag
}

fn validate_summary_contract(summary_dir: &Path) -> Result<SummaryManifest, String> {
    let manifest_path = summary_dir.join("summary.json");
    let bytes = fs::read(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let manifest = serde_json::from_slice::<SummaryManifest>(&bytes)
        .map_err(|error| format!("invalid {}: {error}", manifest_path.display()))?;
    if manifest.schema_version != SUMMARY_SCHEMA_VERSION
        || manifest.avatar_data_contract.schema_version != SUMMARY_SCHEMA_VERSION
    {
        return Err(format!(
            "{} must declare schema_version {} at both contract levels",
            manifest_path.display(),
            SUMMARY_SCHEMA_VERSION
        ));
    }
    for &file_name in REQUIRED_SUMMARY_FILES {
        let contract = manifest
            .avatar_data_contract
            .files
            .get(file_name)
            .ok_or_else(|| {
                format!(
                    "{} is missing the v5 file contract for {file_name}",
                    manifest_path.display()
                )
            })?;
        let expected_required = required_summary_columns(file_name);
        if contract.required_columns.len() != expected_required.len()
            || !contract
                .required_columns
                .iter()
                .zip(expected_required)
                .all(|(actual, expected)| actual == expected)
        {
            return Err(format!(
                "{} has invalid required_columns for {file_name}",
                manifest_path.display()
            ));
        }
        let mut unique_columns = HashSet::new();
        if contract.columns.is_empty()
            || contract
                .columns
                .iter()
                .any(|column| column.is_empty() || !unique_columns.insert(column.as_str()))
        {
            return Err(format!(
                "{} has invalid columns for {file_name}",
                manifest_path.display()
            ));
        }
        let missing_required = expected_required
            .iter()
            .copied()
            .filter(|column| !unique_columns.contains(column))
            .collect::<Vec<_>>();
        if !missing_required.is_empty() {
            return Err(format!(
                "{} columns for {file_name} omit required fields: {}",
                manifest_path.display(),
                missing_required.join(", ")
            ));
        }
    }
    Ok(manifest)
}

fn required_summary_columns(file_name: &str) -> &'static [&'static str] {
    match file_name {
        "avatar_weapon_items.csv" => REQUIRED_ITEM_COLUMNS,
        "avatar_weapon_layers.csv" => REQUIRED_RAW_LAYER_COLUMNS,
        "avatar_layer_visual_imgs.csv" => REQUIRED_VISUAL_CONTRACT_COLUMNS,
        "avatar_box_sets.csv" => REQUIRED_SET_COLUMNS,
        _ => &[],
    }
}

fn validate_summary_file_hashes(
    summary_dir: &Path,
    manifest: &SummaryManifest,
) -> Result<(), String> {
    for &file_name in REQUIRED_SUMMARY_FILES {
        let expected = manifest
            .avatar_data_contract
            .files
            .get(file_name)
            .ok_or_else(|| format!("missing summary contract for {file_name}"))?
            .sha256
            .trim()
            .to_ascii_lowercase();
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("invalid SHA-256 contract for {file_name}"));
        }
        let path = summary_dir.join(file_name);
        let mut file = File::open(&path)
            .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 1024 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            return Err(format!(
                "summary contract hash mismatch for {}: expected {expected}, found {actual}",
                path.display()
            ));
        }

        let mut reader = ReaderBuilder::new()
            .flexible(false)
            .from_path(&path)
            .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
        let actual_columns = reader
            .headers()
            .map_err(|error| format!("failed to read columns from {}: {error}", path.display()))?
            .iter()
            .map(|column| column.trim_start_matches('\u{feff}').to_string())
            .collect::<Vec<_>>();
        let contract = manifest
            .avatar_data_contract
            .files
            .get(file_name)
            .ok_or_else(|| format!("missing summary contract for {file_name}"))?;
        if actual_columns != contract.columns {
            return Err(format!(
                "summary contract columns mismatch for {}",
                path.display()
            ));
        }
        let mut actual_rows = 0u64;
        for record in reader.records() {
            record.map_err(|error| format!("invalid CSV row in {}: {error}", path.display()))?;
            actual_rows = actual_rows
                .checked_add(1)
                .ok_or_else(|| format!("row count overflow in {}", path.display()))?;
        }
        if actual_rows != contract.rows {
            return Err(format!(
                "summary contract rows mismatch for {}: expected {}, found {actual_rows}",
                path.display(),
                contract.rows
            ));
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 4 {
        return Err(
            "usage: avatar_data_pack <summary_dir> <base_avatar_data.dll> <output_avatar_data.dll>"
                .to_string(),
        );
    }
    let summary_dir = PathBuf::from(&args[1]);
    let base_dll = PathBuf::from(&args[2]);
    let output_dll = PathBuf::from(&args[3]);

    let summary_manifest = validate_summary_contract(&summary_dir)?;
    validate_summary_file_hashes(&summary_dir, &summary_manifest)?;

    let item_read = read_items(&summary_dir.join("avatar_weapon_items.csv"))?;
    let mut items = item_read.items;
    let (mut layers, mut layer_counts, mut layer_count) = read_layers(
        &summary_dir.join("avatar_layer_visual_imgs.csv"),
        &item_read.allowed_item_ids,
    )?;
    layer_count += add_skin_layers(
        &mut layers,
        &mut layer_counts,
        &item_read.skin_seeds,
        &item_read.allowed_item_ids,
    );
    validate_visual_img_ownership(&layers)?;
    let layer_counts_by_visual_family = count_layers_by_visual_family(&layers);
    items.retain(|item| {
        let Some(counts) = layer_counts_by_visual_family.get(&item.item_id) else {
            return false;
        };
        item.playable_roles
            .iter()
            .filter_map(|role| visual_family_for_playable_role(role))
            .any(|family| counts.get(family).copied().unwrap_or(0) > 0)
    });
    for item in &mut items {
        item.layer_count = layer_counts
            .get(&item.item_id)
            .copied()
            .unwrap_or(item.layer_count);
        item.layer_counts_by_visual_family = layer_counts_by_visual_family
            .get(&item.item_id)
            .cloned()
            .unwrap_or_default();
    }
    let visible_item_ids = items
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<HashSet<_>>();
    let visible_item_names = item_read
        .item_names
        .into_iter()
        .filter(|(item_id, _)| visible_item_ids.contains(item_id))
        .collect::<HashMap<_, _>>();
    let sets = read_sets(
        &summary_dir.join("avatar_box_sets.csv"),
        &visible_item_ids,
        &visible_item_names,
    )?;
    let source = catalog_source_from_packed(items, sets, layers)?;
    let packed_layer_count = source.layers_by_item.values().map(Vec::len).sum::<usize>();
    if packed_layer_count > layer_count {
        return Err(format!(
            "paged layer count {packed_layer_count} exceeds parsed layer count {layer_count}"
        ));
    }
    let report = write_catalog_index(source, &base_dll, &output_dll)?;
    let verified_pages = PagedDataStore::open(&output_dll, 0)?.verify_all_pages()?;
    if verified_pages != report.pages.len() {
        return Err(format!(
            "authenticated page verification count mismatch: built {}, verified {verified_pages}",
            report.pages.len()
        ));
    }
    println!("{}", output_dll.display());
    println!(
        "schema=7 pages={} verified_pages={verified_pages} build_id={}",
        report.pages.len(),
        report
            .build_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    Ok(())
}

fn catalog_source_from_packed(
    items: Vec<CatalogItem>,
    sets: Vec<CatalogSet>,
    layer_buckets: impl IntoIterator<Item = BTreeMap<String, Vec<LayerRow>>>,
) -> Result<CatalogSource, String> {
    let visible_item_ids = items
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<HashSet<_>>();
    let mut rules_by_item = HashMap::<String, (BTreeSet<u16>, bool, bool)>::new();
    for item in &items {
        let rules = rules_by_item
            .entry(item.item_id.clone())
            .or_insert_with(|| (BTreeSet::new(), false, false));
        rules
            .0
            .extend(item.hide_equipment_type_enums.iter().copied());
        rules.1 |= item.enable_dye_mask1;
        rules.2 |= item.enable_dye_mask2;
    }
    let items = items
        .into_iter()
        .enumerate()
        .map(|(source_order, item)| {
            Ok(AvatarItemRecord {
                source_order: u32::try_from(source_order)
                    .map_err(|_| "catalog item source_order overflow".to_string())?,
                item_id: item.item_id,
                item_name: item.item_name,
                slot: item.slot,
                profession_dir: item.profession_dir,
                playable_roles: item.playable_roles,
                path_family: item.path_family,
                resource_kind: item.resource_kind,
                icon_img: item.icon_img,
                layer_count: item.layer_count,
                layer_counts_by_visual_family: item
                    .layer_counts_by_visual_family
                    .into_iter()
                    .collect(),
                hide_equipment_types: item.hide_equipment_types,
                hide_equipment_type_enums: item.hide_equipment_type_enums,
                enable_dye_mask1: item.enable_dye_mask1,
                enable_dye_mask2: item.enable_dye_mask2,
                layers: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sets = sets
        .into_iter()
        .enumerate()
        .map(|(source_order, set)| {
            Ok(AvatarSetRecord {
                source_order: u32::try_from(source_order)
                    .map_err(|_| "catalog set source_order overflow".to_string())?,
                set_id: set.set_id,
                set_name: set.set_name,
                source: set.source,
                playable_roles: set.playable_roles,
                member_ids: set.member_ids,
                member_names: set.member_names,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut layers_by_item = BTreeMap::<String, Vec<AvatarLayerRecord>>::new();
    for bucket in layer_buckets {
        for (item_id, layers) in bucket {
            if !visible_item_ids.contains(&item_id) {
                continue;
            }
            let (hide_rules, enable_dye_mask1, enable_dye_mask2) = rules_by_item
                .get(&item_id)
                .ok_or_else(|| format!("missing catalog rules for layer item {item_id}"))?;
            let output = layers_by_item.entry(item_id.clone()).or_default();
            for layer in layers {
                if layer.item_id != item_id {
                    return Err(format!(
                        "layer item {} is stored under foreign key {item_id}",
                        layer.item_id
                    ));
                }
                output.push(AvatarLayerRecord {
                    item_id: layer.item_id,
                    item_name: layer.item_name,
                    slot: layer.slot,
                    visual_family_key: layer.visual_family_key,
                    layer_order: layer.layer_order,
                    equipment_type_enum: layer.equipment_type_enum,
                    hide_equipment_type_enums: hide_rules.iter().copied().collect(),
                    enable_dye_mask1: *enable_dye_mask1,
                    enable_dye_mask2: *enable_dye_mask2,
                    layer_sequence: layer.layer_sequence,
                    layer_tag: layer.layer_tag,
                    variation: layer.variation,
                    palette_index: layer.palette_index,
                    visual_img: layer.visual_img,
                    status: layer.status,
                });
            }
        }
    }
    Ok(CatalogSource {
        roles: playable_role_bindings()
            .into_iter()
            .map(|role| AvatarRoleRecord {
                playable_role_key: role.playable_role_key,
                visual_family_key: role.visual_family_key,
            })
            .collect(),
        visual_families: visual_families()
            .into_iter()
            .map(|family| AvatarVisualFamilyRecord {
                visual_family_key: family.visual_family_key,
                resource_owner_key: family.resource_owner_key,
                resource_generation: family.resource_generation,
                default_skin_img: family.default_skin_img,
            })
            .collect(),
        items,
        sets,
        layers_by_item,
    })
}

fn read_items(path: &Path) -> Result<ItemReadResult, String> {
    let mut reader = csv_reader(path)?;
    let headers = header_map(reader.headers().map_err(|err| format!("{err}"))?);
    require_headers(path, &headers, REQUIRED_PACKED_ITEM_COLUMNS)?;
    require_headers(path, &headers, REQUIRED_ITEM_KIND_COLUMNS)?;
    let mut items = Vec::new();
    let mut allowed_item_ids = HashSet::new();
    let mut item_names = HashMap::new();
    let mut skin_seeds = Vec::new();
    let mut enable_dye_by_item_id = HashMap::<String, (bool, bool)>::new();
    let mut playable_roles_by_item_id = HashMap::<String, BTreeSet<String>>::new();
    let mut resource_kind_by_item_id = HashMap::<String, String>::new();
    for (row_index, record) in reader.records().enumerate() {
        let record = record.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let item_id = field(&record, &headers, "item_id").trim().to_string();
        if item_id.is_empty() {
            return Err(format!(
                "missing item_id for {} in {} row {}",
                field(&record, &headers, "pvf_path").trim(),
                path.display(),
                row_index + 2
            ));
        }
        let playable_roles = require_known_playable_roles(
            required_profession_keys(&record, &headers, "profession_keys", path, row_index + 2)?,
            "profession_keys",
            path,
            row_index + 2,
        )?;
        playable_roles_by_item_id
            .entry(item_id.clone())
            .or_default()
            .extend(playable_roles.iter().cloned());
        let resource_kind = required_resource_kind(&record, &headers, path, row_index + 2)?;
        if let Some(previous) =
            resource_kind_by_item_id.insert(item_id.clone(), resource_kind.clone())
        {
            if previous != resource_kind {
                return Err(format!(
                    "conflicting item_kind values {previous:?} and {resource_kind:?} for item {item_id} in {} row {}",
                    path.display(),
                    row_index + 2
                ));
            }
        }
        let enable_dye_mask1 =
            required_bool01(&record, &headers, "enable_dye_mask1", path, row_index + 2)?;
        let enable_dye_mask2 =
            required_bool01(&record, &headers, "enable_dye_mask2", path, row_index + 2)?;
        let union = enable_dye_by_item_id
            .entry(item_id.clone())
            .or_insert((false, false));
        union.0 |= enable_dye_mask1;
        union.1 |= enable_dye_mask2;
        let slot = field(&record, &headers, "slot_dir").trim().to_string();
        let equipment_type = field(&record, &headers, "equipment_type").trim();
        if is_aura_item(&slot, equipment_type) {
            continue;
        }
        let Some(item_name) = display_item_name(field(&record, &headers, "item_name_cn"), &slot)
        else {
            continue;
        };
        let hide_equipment_types =
            split_nonempty_pipe(field(&record, &headers, "hide_equipment_types"));
        let hide_equipment_type_enums =
            split_nonempty_pipe(field(&record, &headers, "hide_equipment_type_enums"))
                .into_iter()
                .map(|value| {
                    value.parse::<u16>().map_err(|error| {
                        format!(
                    "invalid hide_equipment_type_enums value {value:?} in {} row {}: {error}",
                    path.display(),
                    row_index + 2
                )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        if hide_equipment_types.len() != hide_equipment_type_enums.len() {
            return Err(format!(
                "hide equipment type/enum length mismatch in {} row {}",
                path.display(),
                row_index + 2
            ));
        }
        let mut seen_hide_types = HashSet::new();
        let mut seen_hide_enums = HashSet::new();
        for (hide_type, hide_enum) in hide_equipment_types.iter().zip(&hide_equipment_type_enums) {
            if !seen_hide_types.insert(hide_type.as_str()) || !seen_hide_enums.insert(*hide_enum) {
                return Err(format!(
                    "duplicate hide equipment target {hide_type:?}/{hide_enum} in {} row {}",
                    path.display(),
                    row_index + 2
                ));
            }
            let expected = client_equipment_type_enum(hide_type).ok_or_else(|| {
                format!(
                    "unknown hide equipment type {hide_type:?} in {} row {}",
                    path.display(),
                    row_index + 2
                )
            })?;
            if expected != *hide_enum {
                return Err(format!(
                    "hide equipment type {hide_type:?} does not map to enum {hide_enum} (expected {expected}) in {} row {}",
                    path.display(),
                    row_index + 2
                ));
            }
        }
        allowed_item_ids.insert(item_id.clone());
        item_names.insert(item_id.clone(), item_name.clone());
        if slot.eq_ignore_ascii_case("skin") {
            if playable_roles.len() != 1 {
                return Err(format!(
                    "skin item {} must declare exactly one profession key in {} row {}",
                    item_id,
                    path.display(),
                    row_index + 2
                ));
            }
            let equipment_type_enum = required_csv_value::<u16>(
                &record,
                &headers,
                "equipment_type_enum",
                path,
                row_index + 2,
            )?;
            skin_seeds.push(SkinLayerSeed {
                item_id: item_id.clone(),
                item_name: item_name.clone(),
                slot: slot.clone(),
                visual_family_key: visual_family_for_playable_role(&playable_roles[0])
                    .expect("validated playable role")
                    .to_string(),
                variations: field(&record, &headers, "variations").trim().to_string(),
                equipment_type_enum,
            });
        }
        items.push(CatalogItem {
            item_id,
            item_name,
            slot,
            profession_dir: field(&record, &headers, "profession_dir")
                .trim()
                .to_string(),
            playable_roles,
            path_family: field(&record, &headers, "path_family").trim().to_string(),
            resource_kind,
            icon_img: field(&record, &headers, "icon_img").to_string(),
            layer_count: field(&record, &headers, "layer_count")
                .parse::<usize>()
                .unwrap_or(0),
            layer_counts_by_visual_family: BTreeMap::new(),
            hide_equipment_types,
            hide_equipment_type_enums,
            enable_dye_mask1,
            enable_dye_mask2,
        });
    }
    for item in &mut items {
        if let Some((enable_mask1, enable_mask2)) = enable_dye_by_item_id.get(&item.item_id) {
            item.enable_dye_mask1 = *enable_mask1;
            item.enable_dye_mask2 = *enable_mask2;
        }
        if let Some(playable_roles) = playable_roles_by_item_id.get(&item.item_id) {
            item.playable_roles = playable_roles.iter().cloned().collect();
        }
    }
    Ok(ItemReadResult {
        items,
        allowed_item_ids,
        item_names,
        skin_seeds,
    })
}

fn read_layers(path: &Path, allowed_item_ids: &HashSet<String>) -> Result<LayerReadResult, String> {
    let mut reader = csv_reader(path)?;
    let headers = header_map(reader.headers().map_err(|err| format!("{err}"))?);
    require_headers(path, &headers, REQUIRED_LAYER_COLUMNS)?;
    let mut buckets = std::array::from_fn(|_| BTreeMap::<String, Vec<LayerRow>>::new());
    let mut layer_counts = HashMap::<String, usize>::new();
    let mut total = 0usize;
    let mut approved_exception_sequences = HashSet::<u32>::new();
    let mut seen_layers = HashSet::<(
        String,
        String,
        String,
        i32,
        u16,
        String,
        String,
        usize,
        String,
    )>::new();

    for (row_index, record) in reader.records().enumerate() {
        let record = record.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let item_id = field(&record, &headers, "item_id").trim().to_string();
        if item_id.is_empty() {
            return Err(format!(
                "missing item_id for {} in {} row {}",
                field(&record, &headers, "pvf_path").trim(),
                path.display(),
                row_index + 2
            ));
        }
        if !allowed_item_ids.contains(&item_id) {
            continue;
        }
        let slot = field(&record, &headers, "slot_dir").trim().to_string();
        let equipment_type = field(&record, &headers, "equipment_type").trim();
        if is_aura_item(&slot, equipment_type) {
            continue;
        }
        let visual_img = field(&record, &headers, "visual_img").trim();
        let status = field(&record, &headers, "visual_img_status").trim();
        if field(&record, &headers, "equipment_type_enum")
            .trim()
            .is_empty()
            && equipment_type.is_empty()
            && visual_img.is_empty()
            && status == "unsupported_layer_tag"
        {
            let source_row = row_index + 2;
            let sequence =
                required_csv_value::<u32>(&record, &headers, "layer_sequence", path, source_row)?;
            if is_approved_missing_equipment_type_layer(
                &item_id,
                &slot,
                field(&record, &headers, "pvf_path").trim(),
                sequence,
                field(&record, &headers, "layer_variation"),
                field(&record, &headers, "layer_tag"),
            ) {
                if !approved_exception_sequences.insert(sequence) {
                    return Err(format!(
                        "{} row {source_row} duplicates approved exception item {item_id} layer_sequence {sequence}",
                        path.display()
                    ));
                }
                continue;
            }
        }
        let source_row = row_index + 2;
        let playable_role_key =
            required_profession_key(&record, &headers, "visual_profession_key", path, source_row)?;
        let visual_family_key = visual_family_for_playable_role(&playable_role_key)
            .ok_or_else(|| {
                format!(
                    "unknown visual_profession_key value {playable_role_key:?} in {} row {source_row}: no explicit visual_family mapping",
                    path.display()
                )
            })?
            .to_string();
        let bucket = bucket_for_item_id(&item_id);
        let layer = LayerRow {
            item_id: item_id.clone(),
            item_name: display_item_name(field(&record, &headers, "item_name_cn"), &slot)
                .unwrap_or_else(|| "未知单件".to_string()),
            slot,
            visual_family_key,
            layer_order: required_csv_value::<i32>(
                &record,
                &headers,
                "layer_variation",
                path,
                source_row,
            )?,
            equipment_type_enum: required_csv_value::<u16>(
                &record,
                &headers,
                "equipment_type_enum",
                path,
                source_row,
            )?,
            layer_sequence: required_csv_value::<u32>(
                &record,
                &headers,
                "layer_sequence",
                path,
                source_row,
            )?,
            layer_tag: field(&record, &headers, "layer_tag").to_string(),
            variation: field(&record, &headers, "variation").to_string(),
            palette_index: field(&record, &headers, "palette_index")
                .parse::<usize>()
                .unwrap_or(0),
            visual_img: visual_img.to_string(),
            status: status.to_string(),
        };
        let identity = (
            layer.item_id.clone(),
            layer.visual_family_key.to_ascii_lowercase(),
            layer.slot.to_ascii_lowercase(),
            layer.layer_order,
            layer.equipment_type_enum,
            layer.layer_tag.clone(),
            layer.variation.clone(),
            layer.palette_index,
            normalize_visual_img(&layer.visual_img),
        );
        if !seen_layers.insert(identity) {
            continue;
        }
        buckets[bucket]
            .entry(item_id.clone())
            .or_default()
            .push(layer);
        *layer_counts.entry(item_id).or_insert(0) += 1;
        total += 1;
    }
    if !approved_exception_sequences.is_empty()
        && approved_exception_sequences.len() != APPROVED_MISSING_EQUIPMENT_TYPE_LAYERS.len()
    {
        return Err(format!(
            "{} contains an incomplete approved exception for item {}: expected sequences 1..={}, found {:?}",
            path.display(),
            APPROVED_MISSING_EQUIPMENT_TYPE_ITEM_ID,
            APPROVED_MISSING_EQUIPMENT_TYPE_LAYERS.len(),
            approved_exception_sequences
        ));
    }
    validate_visual_img_ownership(&buckets)?;
    Ok((buckets, layer_counts, total))
}

fn add_skin_layers(
    buckets: &mut [BTreeMap<String, Vec<LayerRow>>; BUCKET_COUNT],
    layer_counts: &mut HashMap<String, usize>,
    skin_seeds: &[SkinLayerSeed],
    allowed_item_ids: &HashSet<String>,
) -> usize {
    let mut added = 0usize;
    for seed in skin_seeds {
        if !allowed_item_ids.contains(&seed.item_id) {
            continue;
        }
        let Some(visual_img) = skin_visual_img(seed) else {
            continue;
        };
        let bucket = bucket_for_item_id(&seed.item_id);
        let layer_sequence = buckets[bucket]
            .get(&seed.item_id)
            .and_then(|layers| layers.iter().map(|layer| layer.layer_sequence).max())
            .unwrap_or(0)
            .saturating_add(1);
        let layer = LayerRow {
            item_id: seed.item_id.clone(),
            item_name: seed.item_name.clone(),
            slot: seed.slot.clone(),
            visual_family_key: seed.visual_family_key.clone(),
            layer_order: 0,
            equipment_type_enum: seed.equipment_type_enum,
            layer_sequence,
            layer_tag: "[skin]".to_string(),
            variation: seed.variations.clone(),
            palette_index: 0,
            visual_img,
            status: "derived_skin".to_string(),
        };
        buckets[bucket]
            .entry(seed.item_id.clone())
            .or_default()
            .push(layer);
        *layer_counts.entry(seed.item_id.clone()).or_insert(0) += 1;
        added += 1;
    }
    added
}

fn count_layers_by_visual_family(
    buckets: &[BTreeMap<String, Vec<LayerRow>>; BUCKET_COUNT],
) -> HashMap<String, BTreeMap<String, usize>> {
    let mut counts = HashMap::<String, BTreeMap<String, usize>>::new();
    for bucket in buckets {
        for (item_id, layers) in bucket {
            let item_counts = counts.entry(item_id.clone()).or_default();
            for layer in layers {
                *item_counts
                    .entry(layer.visual_family_key.clone())
                    .or_insert(0) += 1;
            }
        }
    }
    counts
}

fn normalize_visual_img(visual_img: &str) -> String {
    visual_img.trim().replace('\\', "/").to_ascii_lowercase()
}

fn validate_visual_img_ownership(
    buckets: &[BTreeMap<String, Vec<LayerRow>>; BUCKET_COUNT],
) -> Result<(), String> {
    let mut owners = HashMap::<String, String>::new();
    for bucket in buckets {
        for layers in bucket.values() {
            for layer in layers {
                let visual_img = normalize_visual_img(&layer.visual_img);
                if visual_img.is_empty() {
                    continue;
                }
                if let Some(owner) = owners.get(&visual_img) {
                    if !owner.eq_ignore_ascii_case(&layer.visual_family_key) {
                        return Err(format!(
                            "IMG {visual_img} is referenced by different visual families: {owner} and {}",
                            layer.visual_family_key
                        ));
                    }
                } else {
                    owners.insert(visual_img, layer.visual_family_key.clone());
                }
            }
        }
    }
    Ok(())
}

fn read_sets(
    path: &Path,
    visible_item_ids: &HashSet<String>,
    visible_item_names: &HashMap<String, String>,
) -> Result<Vec<CatalogSet>, String> {
    let mut reader = csv_reader(path)?;
    let headers = header_map(reader.headers().map_err(|err| format!("{err}"))?);
    require_headers(path, &headers, REQUIRED_SET_COLUMNS)?;
    let mut sets = Vec::new();
    let mut seen_by_box = HashMap::<String, usize>::new();

    for (row_index, record) in reader.records().enumerate() {
        let record = record.map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let box_id = field(&record, &headers, "box_item_id").trim().to_string();
        if box_id.is_empty() {
            continue;
        }
        let ordinal = seen_by_box.entry(box_id.clone()).or_insert(0);
        let set_id = format!("{box_id}:{ordinal}");
        *ordinal += 1;
        let box_name = field(&record, &headers, "box_name_cn").trim();
        let selection_name = field(&record, &headers, "selection_name_cn").trim();
        let set_name = if selection_name.is_empty() {
            box_name.to_string()
        } else if box_name.is_empty() {
            selection_name.to_string()
        } else {
            format!("{box_name} - {selection_name}")
        };
        let Some(set_name) = display_set_name(&set_name) else {
            continue;
        };
        let raw_member_ids = split_pipe(field(&record, &headers, "member_ids"));
        let raw_member_names = split_pipe(field(&record, &headers, "member_names"));
        let mut member_ids = Vec::new();
        let mut member_names = Vec::new();
        for (index, member_id) in raw_member_ids.into_iter().enumerate() {
            if !visible_item_ids.contains(&member_id) {
                continue;
            }
            member_ids.push(member_id.clone());
            let raw_name = raw_member_names
                .get(index)
                .map(String::as_str)
                .unwrap_or("");
            member_names.push(
                visible_item_names
                    .get(&member_id)
                    .cloned()
                    .or_else(|| display_item_name(raw_name, ""))
                    .unwrap_or_else(|| "未知单件".to_string()),
            );
        }
        if member_ids.len() < 2 {
            continue;
        }
        sets.push(CatalogSet {
            set_id,
            set_name,
            source: "box".to_string(),
            playable_roles: require_known_playable_roles(
                required_profession_keys(
                    &record,
                    &headers,
                    "profession_keys",
                    path,
                    row_index + 2,
                )?,
                "profession_keys",
                path,
                row_index + 2,
            )?,
            member_ids,
            member_names,
        });
    }
    Ok(sets)
}

fn csv_reader(path: &Path) -> Result<csv::Reader<File>, String> {
    ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))
}

fn header_map(headers: &StringRecord) -> HashMap<String, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(index, header)| (header.trim_start_matches('\u{feff}').to_string(), index))
        .collect()
}

fn require_headers(
    path: &Path,
    headers: &HashMap<String, usize>,
    required: &[&str],
) -> Result<(), String> {
    let missing = required
        .iter()
        .copied()
        .filter(|name| !headers.contains_key(*name))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} is missing required v5 columns: {}",
            path.display(),
            missing.join(", ")
        ))
    }
}

fn field<'a>(record: &'a StringRecord, headers: &HashMap<String, usize>, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|index| record.get(*index))
        .unwrap_or("")
}

fn required_csv_value<T>(
    record: &StringRecord,
    headers: &HashMap<String, usize>,
    name: &str,
    path: &Path,
    row: usize,
) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let raw = field(record, headers, name).trim();
    if raw.is_empty() {
        return Err(format!(
            "missing required {name} in {} row {row}",
            path.display()
        ));
    }
    raw.parse::<T>().map_err(|error| {
        format!(
            "invalid {name} {raw:?} in {} row {row}: {error}",
            path.display()
        )
    })
}

fn required_bool01(
    record: &StringRecord,
    headers: &HashMap<String, usize>,
    name: &str,
    path: &Path,
    row: usize,
) -> Result<bool, String> {
    match field(record, headers, name).trim() {
        "0" => Ok(false),
        "1" => Ok(true),
        raw => Err(format!(
            "invalid {name} {raw:?} in {} row {row}: expected 0 or 1",
            path.display()
        )),
    }
}

fn required_resource_kind(
    record: &StringRecord,
    headers: &HashMap<String, usize>,
    path: &Path,
    row: usize,
) -> Result<String, String> {
    let value = field(record, headers, "item_kind")
        .trim()
        .to_ascii_lowercase();
    if matches!(value.as_str(), "avatar" | "weapon_avatar" | "weapon") {
        Ok(value)
    } else {
        Err(format!(
            "invalid item_kind {value:?} in {} row {row}: expected avatar, weapon_avatar, or weapon",
            path.display()
        ))
    }
}

fn required_profession_keys(
    record: &StringRecord,
    headers: &HashMap<String, usize>,
    name: &str,
    path: &Path,
    row: usize,
) -> Result<Vec<String>, String> {
    let keys = split_nonempty_pipe(field(record, headers, name));
    if keys.is_empty() {
        return Err(format!("missing {name} in {} row {row}", path.display()));
    }
    let mut seen = HashSet::new();
    for key in &keys {
        if key == "all" {
            return Err(format!(
                "forbidden shared {name} value {key:?} in {} row {row}",
                path.display()
            ));
        }
        if !is_canonical_profession_key(key) {
            return Err(format!(
                "invalid {name} value {key:?} in {} row {row}",
                path.display()
            ));
        }
        if !seen.insert(key.as_str()) {
            return Err(format!(
                "duplicate {name} value {key:?} in {} row {row}",
                path.display()
            ));
        }
    }
    Ok(keys)
}

fn required_profession_key(
    record: &StringRecord,
    headers: &HashMap<String, usize>,
    name: &str,
    path: &Path,
    row: usize,
) -> Result<String, String> {
    let keys = required_profession_keys(record, headers, name, path, row)?;
    if keys.len() != 1 {
        return Err(format!(
            "{name} must contain exactly one key in {} row {row}",
            path.display()
        ));
    }
    Ok(keys.into_iter().next().expect("one profession key"))
}

fn is_canonical_profession_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        && !value.contains("__")
}

fn split_pipe(value: &str) -> Vec<String> {
    value.split('|').map(ToString::to_string).collect()
}

fn split_nonempty_pipe(value: &str) -> Vec<String> {
    value
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn display_item_name(raw_name: &str, slot: &str) -> Option<String> {
    let name = raw_name.trim();
    if name.is_empty() || is_pure_numeric_name(name) {
        return None;
    }
    if slot.eq_ignore_ascii_case("skin") && !contains_cjk(name) {
        return None;
    }
    if is_placeholder_name(name) {
        return Some("未知单件".to_string());
    }
    sanitize_indexed_display_name(name)
}

fn display_set_name(raw_name: &str) -> Option<String> {
    let name = raw_name.trim();
    if name.is_empty() || is_pure_numeric_name(name) {
        return None;
    }
    if is_lucky_bag_set_name(name) {
        return None;
    }
    if is_placeholder_name(name) {
        return Some("未知套装".to_string());
    }
    sanitize_indexed_display_name(name)
}

fn is_pure_numeric_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit())
}

fn is_placeholder_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.starts_with('<') {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("name_")
}

fn sanitize_indexed_display_name(name: &str) -> Option<String> {
    let mut cleaned = name.trim().to_string();
    while let Some((prefix, suffix)) = cleaned.rsplit_once(" - ") {
        if !is_internal_token(suffix) {
            break;
        }
        cleaned = prefix.trim().to_string();
    }
    if cleaned.is_empty()
        || is_runtime_placeholder_name(&cleaned)
        || is_internal_token(&cleaned)
        || cleaned.chars().all(|ch| ch.is_ascii_digit())
    {
        None
    } else {
        Some(cleaned)
    }
}

fn is_runtime_placeholder_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.starts_with('<')
        || runtime_placeholder_suffix(trimmed).is_some()
        || trimmed
            .split_once("::")
            .and_then(|(_, suffix)| runtime_placeholder_suffix(suffix.trim_end_matches('>')))
            .is_some()
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

fn runtime_placeholder_suffix(value: &str) -> Option<&str> {
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

fn contains_cjk(name: &str) -> bool {
    name.chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
}

fn is_aura_item(slot: &str, equipment_type: &str) -> bool {
    let slot = slot.trim().to_ascii_lowercase();
    if slot == "aura" || slot == "aurora" {
        return true;
    }
    let equipment_type = equipment_type.trim().to_ascii_lowercase();
    equipment_type.contains("aurora avatar") || equipment_type.contains("aura avatar")
}

fn skin_visual_img(seed: &SkinLayerSeed) -> Option<String> {
    let template = skin_body_template(&seed.visual_family_key)?;
    let variation = skin_variation_index(&seed.variations)?;
    Some(template.replace("%04d", &format!("{variation:04}")))
}

fn skin_variation_index(variations: &str) -> Option<i32> {
    let first = variations.split('|').next().unwrap_or(variations);
    let mut numbers = Vec::new();
    let mut current = String::new();
    for ch in first.chars() {
        if ch.is_ascii_digit() || (ch == '-' && current.is_empty()) {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(value) = current.parse::<i32>() {
                numbers.push(value);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(value) = current.parse::<i32>() {
            numbers.push(value);
        }
    }
    numbers.get(1).copied().or_else(|| numbers.first().copied())
}

fn skin_body_template(visual_family_key: &str) -> Option<String> {
    let default_skin_img = visual_family_definition(visual_family_key)?.default_skin_img;
    default_skin_img
        .strip_suffix("0000.img")
        .map(|prefix| format!("{prefix}%04d.img"))
}
