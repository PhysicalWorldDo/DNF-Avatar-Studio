use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::avatar_dye::DyeRecipeV1;
use crate::dnflib_safe::{ImgArchive, NpkArchive, NpkBuilder};
use crate::game_resolver::{
    normalize_img_path_for_npk, resolve_game_npk_path, resolve_img_resource, ImgResourceMetadata,
};
use crate::models::{
    visible_avatar_layers, AvatarLayer, OutfitSnapshot, OutfitSwapExportResult, OutfitSwapProgress,
    ProfessionWideCarrierCandidate, ProfessionWideSwapPlan,
};
use crate::outfit_swap::{
    compose_effective_outfit_frames_with_dye_and_target_effect, encode_part_img_bytes,
    validate_snapshot_pair, PartFrame, TargetVisualEffect,
};

const REQUIRED_AVATAR_GROUPS: [&str; 9] = [
    "belt", "cap", "coat", "face", "hair", "neck", "pants", "shoes", "skin",
];
const CONFIRMATION_TOKEN_VERSION: &str = "pw2";
const CONFIRMATION_TOKEN_TTL_MS: u64 = 10 * 60 * 1_000;
const MAX_SOURCE_NPK_COUNT: usize = 64;
const MAX_SUPPRESSED_IMG_COUNT: usize = 100_000;
// The production swordman/equipment Avatar family currently contains 3,286,469 frames.
const MAX_TOTAL_FRAME_COUNT: u64 = 4_000_000;
const MAX_ESTIMATED_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TransparentFrameMetadata {
    canvas_width: i32,
    canvas_height: i32,
}

#[derive(Debug, Clone)]
struct SuppressionEntry {
    entry_path: String,
    frames: Vec<TransparentFrameMetadata>,
}

#[derive(Debug, Clone)]
struct SourceNpkIdentity {
    path: PathBuf,
    byte_len: u64,
    sha256: [u8; 32],
}

struct DiscoveredAvatarFamily {
    avatar_root: String,
    entries: Vec<SuppressionEntry>,
    sources: Vec<SourceNpkIdentity>,
    total_frame_count: u64,
}

#[derive(Clone)]
struct CarrierSelection {
    view: ProfessionWideCarrierCandidate,
    layer: AvatarLayer,
}

struct PreparedProfessionWideSwap {
    public: ProfessionWideSwapPlan,
    carrier_entry: String,
    target_img_bytes: Vec<u8>,
    entries: Vec<SuppressionEntry>,
    sources: Vec<SourceNpkIdentity>,
}

pub fn list_profession_wide_carrier_candidates(
    current_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
) -> Result<Vec<ProfessionWideCarrierCandidate>, String> {
    Ok(carrier_selections(current_snapshot, current_layers)?
        .into_iter()
        .map(|candidate| candidate.view)
        .collect())
}

pub fn preview_profession_wide_single_carrier_from_layers(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    target_recipe: Option<&DyeRecipeV1>,
    carrier_slot: &str,
    issued_at_ms: u64,
) -> Result<ProfessionWideSwapPlan, String> {
    preview_profession_wide_single_carrier_from_layers_with_target_effect(
        game_dir,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        target_recipe,
        TargetVisualEffect::None,
        carrier_slot,
        issued_at_ms,
    )
}

pub(crate) fn preview_profession_wide_single_carrier_from_layers_with_target_effect(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    target_recipe: Option<&DyeRecipeV1>,
    target_effect: TargetVisualEffect,
    carrier_slot: &str,
    issued_at_ms: u64,
) -> Result<ProfessionWideSwapPlan, String> {
    Ok(prepare_profession_wide_swap(
        game_dir,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        target_recipe,
        target_effect,
        carrier_slot,
        issued_at_ms,
    )?
    .public)
}

#[allow(clippy::too_many_arguments)]
pub fn export_profession_wide_single_carrier_from_layers<F>(
    game_dir: &Path,
    output_path: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    target_recipe: Option<&DyeRecipeV1>,
    carrier_slot: &str,
    confirmation_token: &str,
    now_ms: u64,
    report: F,
) -> Result<OutfitSwapExportResult, String>
where
    F: FnMut(OutfitSwapProgress),
{
    export_profession_wide_single_carrier_from_layers_with_target_effect(
        game_dir,
        output_path,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        target_recipe,
        TargetVisualEffect::None,
        carrier_slot,
        confirmation_token,
        now_ms,
        report,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn export_profession_wide_single_carrier_from_layers_with_target_effect<F>(
    game_dir: &Path,
    output_path: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    target_recipe: Option<&DyeRecipeV1>,
    target_effect: TargetVisualEffect,
    carrier_slot: &str,
    confirmation_token: &str,
    now_ms: u64,
    mut report: F,
) -> Result<OutfitSwapExportResult, String>
where
    F: FnMut(OutfitSwapProgress),
{
    let issued_at_ms = confirmation_token_issued_at(confirmation_token)?;
    if now_ms < issued_at_ms || now_ms.saturating_sub(issued_at_ms) > CONFIRMATION_TOKEN_TTL_MS {
        return Err("profession-wide confirmation token has expired".to_string());
    }
    report(progress(0, "重新校验职业时装资源"));
    let prepared = prepare_profession_wide_swap(
        game_dir,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        target_recipe,
        target_effect,
        carrier_slot,
        issued_at_ms,
    )?;
    if prepared.public.confirmation_token != confirmation_token {
        return Err(
            "profession-wide confirmation token no longer matches the resources".to_string(),
        );
    }
    validate_output_path(output_path, &prepared.sources)?;

    report(progress(10, "生成唯一载体与透明 IMG"));
    let mut builder = NpkBuilder::create()?;
    let mut transparent_templates = BTreeMap::<Vec<TransparentFrameMetadata>, Vec<u8>>::new();
    let mut expected = BTreeMap::<String, ExpectedOutputEntry>::new();
    let total = prepared.entries.len().max(1);
    for (index, entry) in prepared.entries.iter().enumerate() {
        let is_carrier = entry
            .entry_path
            .eq_ignore_ascii_case(&prepared.carrier_entry);
        let bytes = if is_carrier {
            prepared.target_img_bytes.as_slice()
        } else {
            if !transparent_templates.contains_key(&entry.frames) {
                let encoded = encode_part_img_bytes(&transparent_frames(&entry.frames))?;
                transparent_templates.insert(entry.frames.clone(), encoded);
            }
            transparent_templates
                .get(&entry.frames)
                .ok_or_else(|| "transparent IMG template disappeared".to_string())?
                .as_slice()
        };
        builder.add_entry(&entry.entry_path, bytes)?;
        expected.insert(
            normalize_img_path_for_npk(&entry.entry_path),
            ExpectedOutputEntry {
                entry_path: entry.entry_path.clone(),
                sha256: Sha256::digest(bytes).into(),
                frames: entry.frames.clone(),
                is_carrier,
            },
        );
        let percent = 10 + ((index.saturating_add(1) * 78 / total).min(78) as u8);
        report(progress(
            percent,
            &format!("写入职业时装入口 {}/{}", index + 1, total),
        ));
    }

    report(progress(90, "保存职业全量替代补丁"));
    builder.save_path_create_new(output_path, true)?;
    if let Err(error) = validate_profession_wide_output(output_path, &expected) {
        let _ = std::fs::remove_file(output_path);
        return Err(error);
    }
    report(progress(100, "单时装特殊补丁生成完成"));
    Ok(OutfitSwapExportResult {
        output_path: output_path.to_string_lossy().into_owned(),
        part_count: 1,
        entry_count: expected.len(),
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare_profession_wide_swap(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    target_recipe: Option<&DyeRecipeV1>,
    target_effect: TargetVisualEffect,
    carrier_slot: &str,
    issued_at_ms: u64,
) -> Result<PreparedProfessionWideSwap, String> {
    validate_snapshot_pair(current_snapshot, target_snapshot)?;
    if !game_dir.is_dir() {
        return Err("game NPK directory does not exist".to_string());
    }
    let selections = carrier_selections(current_snapshot, current_layers)?;
    let selection = selections
        .into_iter()
        .find(|candidate| {
            candidate
                .view
                .slot
                .eq_ignore_ascii_case(carrier_slot.trim())
        })
        .ok_or_else(|| "selected carrier is not visible in the current outfit".to_string())?;
    let scope = profession_avatar_scope(
        &current_snapshot.resource_owner_key,
        &current_snapshot.resource_generation,
    )?;
    let discovered = discover_profession_avatar_family(game_dir, &scope)?;
    let entries_by_key = discovered
        .entries
        .iter()
        .map(|entry| {
            (
                normalize_img_path_for_npk(&entry.entry_path),
                entry.entry_path.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let carrier_entry = resolve_img_resource(
        &selection.layer.visual_img,
        selection.layer.palette_index,
        |candidate| -> Result<_, String> {
            let Some(actual_entry) = entries_by_key
                .get(&normalize_img_path_for_npk(candidate))
                .cloned()
            else {
                return Ok(None);
            };
            let npk_path = resolve_game_npk_path(game_dir, &actual_entry)
                .ok_or_else(|| format!("cannot resolve carrier NPK for {actual_entry}"))?;
            let archive = NpkArchive::open_path(&npk_path)?;
            let entry_index = archive
                .find_entry_index(&actual_entry)
                .ok_or_else(|| format!("carrier IMG disappeared: {actual_entry}"))?;
            let img = archive.read_img(entry_index)?;
            Ok(Some((
                actual_entry,
                ImgResourceMetadata {
                    version: img.version(),
                    palette_count: img.palette_count(),
                },
            )))
        },
    )?
    .map(|resolved| resolved.value)
    .ok_or_else(|| "selected carrier is outside the profession Avatar NPK scope".to_string())?;

    let target_frames = compose_effective_outfit_frames_with_dye_and_target_effect(
        game_dir,
        target_snapshot,
        target_layers,
        target_recipe,
        target_effect,
    )?;
    if !target_frames.iter().any(|frame| {
        frame
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel.get(3).copied().unwrap_or_default() != 0)
    }) {
        return Err("special carrier target has no visible pixels".to_string());
    }
    let target_img_bytes = encode_part_img_bytes(&target_frames)?;
    let estimated_output_bytes =
        estimate_output_bytes(&discovered.entries, &carrier_entry, target_img_bytes.len())?;
    if estimated_output_bytes > MAX_ESTIMATED_OUTPUT_BYTES {
        return Err(
            "profession Avatar suppression output exceeds the size safety limit".to_string(),
        );
    }

    let fingerprint = profession_wide_fingerprint(
        current_snapshot,
        target_snapshot,
        target_recipe,
        target_effect,
        &selection,
        &carrier_entry,
        &target_img_bytes,
        &discovered,
    );
    let confirmation_token = confirmation_token(issued_at_ms, fingerprint);
    let total_img_count = discovered.entries.len();
    let public = ProfessionWideSwapPlan {
        status: "ready".to_string(),
        strategy: "profession_wide_single_carrier".to_string(),
        profession_dir: current_snapshot.profession_dir.clone(),
        path_family: current_snapshot.path_family.clone(),
        avatar_root: discovered.avatar_root.clone(),
        carrier_slot: selection.view.slot.clone(),
        carrier_item_name: selection.view.item_name.clone(),
        carrier_layer_order: selection.view.layer_order,
        carrier_equipment_type_enum: selection.view.equipment_type_enum,
        carrier_layer_sequence: selection.view.layer_sequence,
        source_npk_count: discovered.sources.len(),
        total_img_count,
        suppressed_img_count: total_img_count.saturating_sub(1),
        total_frame_count: discovered.total_frame_count,
        estimated_output_bytes,
        affected_scope: affected_scope_label(&scope),
        confirmation_token,
    };
    Ok(PreparedProfessionWideSwap {
        public,
        carrier_entry,
        target_img_bytes,
        entries: discovered.entries,
        sources: discovered.sources,
    })
}

fn carrier_selections(
    current_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
) -> Result<Vec<CarrierSelection>, String> {
    if current_snapshot.item_ids.is_empty() {
        return Err("current outfit snapshot has no selectable item".to_string());
    }
    let current_ids = current_snapshot
        .item_ids
        .iter()
        .map(|item| item.trim())
        .collect::<HashSet<_>>();
    let visible = visible_avatar_layers(current_layers);
    let mut grouped = BTreeMap::<(String, String), Vec<AvatarLayer>>::new();
    for layer in visible.into_iter().filter(|layer| {
        current_ids.contains(layer.item_id.trim()) && !layer.visual_img.trim().is_empty()
    }) {
        grouped
            .entry((
                layer.slot.trim().to_ascii_lowercase(),
                layer.item_id.trim().to_string(),
            ))
            .or_default()
            .push(layer);
    }

    let mut selections = Vec::new();
    for ((slot, _), layers) in grouped {
        if physical_carrier_has_conflicting_draw_orders(&layers) {
            continue;
        }
        let Some(layer) = layers.into_iter().max_by(|left, right| {
            left.draw_order().cmp(&right.draw_order()).then_with(|| {
                normalize_img_path_for_npk(&left.visual_img)
                    .cmp(&normalize_img_path_for_npk(&right.visual_img))
            })
        }) else {
            continue;
        };
        selections.push(CarrierSelection {
            view: ProfessionWideCarrierCandidate {
                slot,
                item_name: layer.item_name.clone(),
                layer_order: layer.layer_order,
                equipment_type_enum: layer.equipment_type_enum,
                layer_sequence: layer.layer_sequence,
            },
            layer,
        });
    }
    selections.sort_by(|left, right| {
        left.view
            .slot
            .cmp(&right.view.slot)
            .then_with(|| left.view.item_name.cmp(&right.view.item_name))
    });
    if selections.is_empty() {
        return Err("current outfit has no visible single-carrier candidate".to_string());
    }
    Ok(selections)
}

fn physical_carrier_has_conflicting_draw_orders(layers: &[AvatarLayer]) -> bool {
    let mut seen = HashMap::<String, &AvatarLayer>::new();
    for layer in layers {
        let key = normalize_img_path_for_npk(&layer.visual_img);
        if let Some(previous) = seen.get(&key) {
            let previous_order = previous.draw_order();
            let order = layer.draw_order();
            let equivalent_job_branch = previous.item_id == layer.item_id
                && previous.slot.eq_ignore_ascii_case(&layer.slot)
                && previous_order.layer_order == order.layer_order
                && previous_order.equipment_type_enum == order.equipment_type_enum
                && previous.layer_tag == layer.layer_tag
                && previous.variation == layer.variation
                && previous.palette_index == layer.palette_index;
            if !equivalent_job_branch {
                return true;
            }
        } else {
            seen.insert(key, layer);
        }
    }
    false
}

struct ProfessionAvatarScope {
    profession_dir: String,
    equipment_group: &'static str,
    avatar_root: String,
    npk_prefix: String,
}

fn profession_avatar_scope(
    resource_owner_key: &str,
    resource_generation: &str,
) -> Result<ProfessionAvatarScope, String> {
    let profession_dir = resource_owner_key.trim().to_ascii_lowercase();
    if profession_dir.is_empty() {
        return Err("visual family resource owner is empty".to_string());
    }
    let equipment_group = match resource_generation.trim().to_ascii_lowercase().as_str() {
        "base" => "equipment",
        "at" => "atequipment",
        _ => {
            return Err(format!(
                "unsupported visual family resource generation: {resource_generation}"
            ));
        }
    };
    let avatar_root = format!("sprite/character/{profession_dir}/{equipment_group}/avatar/");
    let npk_prefix = avatar_root.trim_end_matches('/').replace('/', "_") + "_";
    Ok(ProfessionAvatarScope {
        profession_dir,
        equipment_group,
        avatar_root,
        npk_prefix,
    })
}

fn discover_profession_avatar_family(
    game_dir: &Path,
    scope: &ProfessionAvatarScope,
) -> Result<DiscoveredAvatarFamily, String> {
    let directory = std::fs::read_dir(game_dir)
        .map_err(|error| format!("failed to list game NPK directory: {error}"))?;
    let mut candidates = Vec::new();
    for entry in directory {
        let entry =
            entry.map_err(|error| format!("failed to enumerate game NPK directory: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect game NPK entry: {error}"))?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if file_name.starts_with(&scope.npk_prefix) && file_name.ends_with(".npk") {
            candidates.push((file_name, entry.path()));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    if candidates.len() > MAX_SOURCE_NPK_COUNT {
        return Err("profession Avatar NPK count exceeds the safety limit".to_string());
    }
    let discovered_groups = candidates
        .iter()
        .filter_map(|(file_name, _)| {
            file_name
                .strip_prefix(&scope.npk_prefix)
                .and_then(|suffix| suffix.strip_suffix(".npk"))
                .map(str::to_string)
        })
        .collect::<BTreeSet<_>>();
    let missing = REQUIRED_AVATAR_GROUPS
        .iter()
        .filter(|group| !discovered_groups.contains(**group))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "profession Avatar NPK set is incomplete; missing {}",
            missing.join(", ")
        ));
    }

    let mut entries_by_key = BTreeMap::<String, SuppressionEntry>::new();
    let mut sources = Vec::with_capacity(candidates.len());
    let mut total_frame_count = 0_u64;
    for (_, npk_path) in candidates {
        let bytes = std::fs::read(&npk_path)
            .map_err(|error| format!("failed to read source NPK: {error}"))?;
        let byte_len = bytes.len() as u64;
        let source_sha256 = Sha256::digest(&bytes).into();
        let archive = NpkArchive::open_bytes(bytes)?;
        let mut archive_img_count = 0usize;
        for entry in archive.entries() {
            let normalized = normalize_img_path_for_npk(&entry.name);
            if !normalized.ends_with(".img") {
                continue;
            }
            archive_img_count += 1;
            if normalized
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                return Err("Avatar NPK contains an unsafe logical path".to_string());
            }
            if !normalized.starts_with(&scope.avatar_root) {
                return Err(
                    "Avatar NPK contains an IMG outside the expected profession root".to_string(),
                );
            }
            if entries_by_key.contains_key(&normalized) {
                return Err("duplicate logical IMG path across profession Avatar NPKs".to_string());
            }
            let img = archive
                .read_img(entry.index)
                .map_err(|error| format!("unsupported suppression IMG: {error}"))?;
            let frames = img
                .frames()
                .into_iter()
                .map(|frame| {
                    if frame.canvas_width <= 0 || frame.canvas_height <= 0 {
                        return Err("suppression IMG has an invalid frame canvas".to_string());
                    }
                    Ok(TransparentFrameMetadata {
                        canvas_width: frame.canvas_width,
                        canvas_height: frame.canvas_height,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if frames.is_empty() {
                return Err("suppression IMG has no frames".to_string());
            }
            total_frame_count = total_frame_count
                .checked_add(frames.len() as u64)
                .ok_or_else(|| "profession Avatar frame count overflowed".to_string())?;
            if total_frame_count > MAX_TOTAL_FRAME_COUNT {
                return Err("profession Avatar frame count exceeds the safety limit".to_string());
            }
            entries_by_key.insert(
                normalized,
                SuppressionEntry {
                    entry_path: entry.name,
                    frames,
                },
            );
            if entries_by_key.len() > MAX_SUPPRESSED_IMG_COUNT {
                return Err("profession Avatar IMG count exceeds the safety limit".to_string());
            }
        }
        if archive_img_count == 0 {
            return Err("profession Avatar NPK contains no IMG entries".to_string());
        }
        sources.push(SourceNpkIdentity {
            byte_len,
            path: npk_path,
            sha256: source_sha256,
        });
    }
    Ok(DiscoveredAvatarFamily {
        avatar_root: scope.avatar_root.clone(),
        entries: entries_by_key.into_values().collect(),
        sources,
        total_frame_count,
    })
}

fn estimate_output_bytes(
    entries: &[SuppressionEntry],
    carrier_entry: &str,
    carrier_bytes: usize,
) -> Result<u64, String> {
    let mut templates = BTreeMap::<Vec<TransparentFrameMetadata>, usize>::new();
    let mut total = u64::try_from(carrier_bytes)
        .map_err(|_| "carrier IMG is too large to estimate".to_string())?;
    for entry in entries {
        if entry.entry_path.eq_ignore_ascii_case(carrier_entry) {
            continue;
        }
        let size = if let Some(size) = templates.get(&entry.frames) {
            *size
        } else {
            let size = encode_part_img_bytes(&transparent_frames(&entry.frames))?.len();
            templates.insert(entry.frames.clone(), size);
            size
        };
        total = total
            .checked_add(size as u64)
            .ok_or_else(|| "profession Avatar output estimate overflowed".to_string())?;
    }
    let directory_overhead = (entries.len() as u64)
        .checked_mul(320)
        .ok_or_else(|| "profession Avatar directory estimate overflowed".to_string())?;
    total
        .checked_add(directory_overhead)
        .and_then(|value| value.checked_add(64))
        .ok_or_else(|| "profession Avatar output estimate overflowed".to_string())
}

fn transparent_frames(metadata: &[TransparentFrameMetadata]) -> Vec<PartFrame> {
    metadata
        .iter()
        .map(|frame| PartFrame {
            width: 1,
            height: 1,
            x: frame.canvas_width,
            y: 0,
            canvas_width: frame.canvas_width,
            canvas_height: frame.canvas_height,
            rgba: vec![0, 0, 0, 0],
        })
        .collect()
}

fn profession_wide_fingerprint(
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    target_recipe: Option<&DyeRecipeV1>,
    target_effect: TargetVisualEffect,
    selection: &CarrierSelection,
    carrier_entry: &str,
    target_img_bytes: &[u8],
    discovered: &DiscoveredAvatarFamily,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash_field(&mut hash, b"profession-wide-single-carrier-v1");
    hash_snapshot(&mut hash, current_snapshot);
    hash_snapshot(&mut hash, target_snapshot);
    hash_field(
        &mut hash,
        match target_effect {
            TargetVisualEffect::None => b"target-effect:none",
            TargetVisualEffect::LinearDodge => b"target-effect:linear-dodge",
        },
    );
    hash_field(&mut hash, selection.view.slot.as_bytes());
    hash_field(&mut hash, carrier_entry.as_bytes());
    hash_field(&mut hash, &Sha256::digest(target_img_bytes));
    if let Some(recipe) = target_recipe {
        hash_field(&mut hash, &recipe.recipe_hash);
    } else {
        hash_field(&mut hash, b"no-target-dye");
    }
    for source in &discovered.sources {
        hash_field(
            &mut hash,
            source
                .path
                .to_string_lossy()
                .to_ascii_lowercase()
                .as_bytes(),
        );
        hash_field(&mut hash, &source.byte_len.to_le_bytes());
        hash_field(&mut hash, &source.sha256);
    }
    for entry in &discovered.entries {
        hash_field(
            &mut hash,
            normalize_img_path_for_npk(&entry.entry_path).as_bytes(),
        );
        for frame in &entry.frames {
            hash_field(&mut hash, &frame.canvas_width.to_le_bytes());
            hash_field(&mut hash, &frame.canvas_height.to_le_bytes());
        }
    }
    hash.finalize().into()
}

fn hash_snapshot(hash: &mut Sha256, snapshot: &OutfitSnapshot) {
    hash_field(hash, snapshot.profession_dir.as_bytes());
    hash_field(hash, snapshot.path_family.as_bytes());
    hash_field(hash, snapshot.playable_role_key.as_bytes());
    hash_field(hash, snapshot.visual_family_key.as_bytes());
    hash_field(hash, snapshot.resource_owner_key.as_bytes());
    hash_field(hash, snapshot.resource_generation.as_bytes());
    for item in &snapshot.item_ids {
        hash_field(hash, item.as_bytes());
    }
}

fn hash_field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn confirmation_token(issued_at_ms: u64, fingerprint: [u8; 32]) -> String {
    format!(
        "{CONFIRMATION_TOKEN_VERSION}:{issued_at_ms}:{}",
        fingerprint
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn confirmation_token_issued_at(token: &str) -> Result<u64, String> {
    let mut parts = token.split(':');
    let version = parts.next().unwrap_or_default();
    let issued = parts.next().unwrap_or_default();
    let digest = parts.next().unwrap_or_default();
    if version != CONFIRMATION_TOKEN_VERSION
        || parts.next().is_some()
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("profession-wide confirmation token is invalid".to_string());
    }
    issued
        .parse::<u64>()
        .map_err(|_| "profession-wide confirmation token is invalid".to_string())
}

fn affected_scope_label(scope: &ProfessionAvatarScope) -> String {
    if scope.profession_dir == "swordman" && scope.equipment_group == "equipment" {
        "男鬼剑士、黑暗武士（共享 swordman/equipment）".to_string()
    } else {
        format!(
            "共享 {}/{} 的全部职业",
            scope.profession_dir, scope.equipment_group
        )
    }
}

fn validate_output_path(
    output_path: &Path,
    source_npks: &[SourceNpkIdentity],
) -> Result<(), String> {
    if !output_path.is_absolute() {
        return Err("output NPK path must be absolute".to_string());
    }
    if !output_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("npk"))
    {
        return Err("output file extension must be .npk".to_string());
    }
    let output_key = comparable_path(output_path)?;
    for source in source_npks {
        if comparable_path(&source.path)? == output_key {
            return Err("output path must not overwrite a source NPK".to_string());
        }
    }
    Ok(())
}

fn comparable_path(path: &Path) -> Result<String, String> {
    let absolute = if path.exists() {
        std::fs::canonicalize(path).map_err(|error| format!("failed to resolve path: {error}"))?
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| "path has no parent directory".to_string())?;
        let parent = std::fs::canonicalize(parent)
            .map_err(|error| format!("failed to resolve output directory: {error}"))?;
        parent.join(
            path.file_name()
                .ok_or_else(|| "path has no file name".to_string())?,
        )
    };
    Ok(absolute
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase())
}

struct ExpectedOutputEntry {
    entry_path: String,
    sha256: [u8; 32],
    frames: Vec<TransparentFrameMetadata>,
    is_carrier: bool,
}

fn validate_profession_wide_output(
    output_path: &Path,
    expected: &BTreeMap<String, ExpectedOutputEntry>,
) -> Result<(), String> {
    let archive = NpkArchive::open_path(output_path)
        .map_err(|_| "职业全量替代补丁写入后无法重新打开".to_string())?;
    if archive.entry_count() != expected.len() {
        return Err("职业全量替代补丁的 IMG 数量不一致".to_string());
    }
    let mut nontransparent_entries = 0usize;
    for expected_entry in expected.values() {
        let index = archive
            .find_entry_index(&expected_entry.entry_path)
            .ok_or_else(|| "职业全量替代补丁缺少预期 IMG".to_string())?;
        let bytes = archive
            .entry_bytes(index)
            .map_err(|_| "职业全量替代补丁 IMG 无法重读".to_string())?;
        if <[u8; 32]>::from(Sha256::digest(bytes)) != expected_entry.sha256 {
            return Err("职业全量替代补丁 IMG 字节校验失败".to_string());
        }
        let img = ImgArchive::open_owned_bytes(bytes.to_vec())
            .map_err(|_| "职业全量替代补丁 IMG 无法重新打开".to_string())?;
        let frames = img.frames();
        if !expected_entry.is_carrier {
            if frames.len() != expected_entry.frames.len() {
                return Err("透明 IMG 的帧数与源资源不一致".to_string());
            }
            for (frame, expected_frame) in frames.iter().zip(&expected_entry.frames) {
                if frame.canvas_width != expected_frame.canvas_width
                    || frame.canvas_height != expected_frame.canvas_height
                {
                    return Err("透明 IMG 的逐帧画布与源资源不一致".to_string());
                }
            }
        }
        let mut entry_has_alpha = false;
        for frame in frames {
            let decoded = img
                .decode_frame(frame.index)
                .map_err(|_| "职业全量替代补丁 IMG 帧无法解码".to_string())?;
            entry_has_alpha |= decoded.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0);
            if !expected_entry.is_carrier && entry_has_alpha {
                return Err("职业全量替代补丁包含非载体可见像素".to_string());
            }
        }
        if entry_has_alpha {
            nontransparent_entries += 1;
        }
    }
    if nontransparent_entries != 1 {
        return Err("职业全量替代补丁没有且仅有一个非透明载体".to_string());
    }
    Ok(())
}

fn progress(percent: u8, stage: &str) -> OutfitSwapProgress {
    OutfitSwapProgress {
        percent,
        stage: stage.to_string(),
    }
}
