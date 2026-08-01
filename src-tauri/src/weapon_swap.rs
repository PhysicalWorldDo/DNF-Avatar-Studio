use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::avatar_dye::equivalent_animation_job_layer;
use crate::dnflib_safe::{ImgArchive, NpkArchive, NpkBuilder};
use crate::game_resolver::{
    img_path_lookup_candidates, normalize_img_path_for_npk, resolve_game_npk_path,
};
use crate::models::{
    AvatarLayer, AvatarLayerDrawOrder, OutfitSwapExportResult, OutfitSwapProgress,
};
use crate::outfit_swap::{
    compose_weapon_part_frames_with_target_effect, encode_part_img_bytes_v4_v6, PartFrame,
    TargetVisualEffect,
};

const WEAPON_AVATAR_EQUIPMENT_TYPE: u16 = 10;
const ORDINARY_WEAPON_EQUIPMENT_TYPE: u16 = 12;
const MAX_CURRENT_WEAPON_DECODE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ValidatedWeaponLayers<'a> {
    current: Vec<&'a AvatarLayer>,
    target: Vec<&'a AvatarLayer>,
}

#[derive(Debug)]
struct RawImgEntry {
    entry_path: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct CurrentWeaponEntry {
    entry_path: String,
    source_hash: [u8; 32],
    source_version: i32,
    palette_board_count: usize,
    draw_order: AvatarLayerDrawOrder,
    visible_frames: Vec<bool>,
    transparent_frames: Vec<PartFrame>,
}

pub(crate) fn validate_weapon_swap_layers<'a>(
    current_visual_family: &str,
    target_visual_family: &str,
    current_layers: &'a [AvatarLayer],
    target_layers: &'a [AvatarLayer],
) -> Result<ValidatedWeaponLayers<'a>, String> {
    let current_visual_family = current_visual_family.trim();
    let target_visual_family = target_visual_family.trim();
    if current_visual_family.is_empty()
        || target_visual_family.is_empty()
        || !current_visual_family.eq_ignore_ascii_case(target_visual_family)
    {
        return Err("武器换装只能在同一视觉族之间进行".to_string());
    }

    let mut current = normalized_weapon_layers(current_layers, "当前")?;
    let mut target =
        deduplicate_equivalent_weapon_layers(normalized_weapon_layers(target_layers, "目标")?);
    if current.first().map(|layer| layer.item_id.trim())
        == target.first().map(|layer| layer.item_id.trim())
    {
        return Err("当前武器与目标武器相同，无需生成补丁".to_string());
    }

    current.sort_by_key(|layer| layer.draw_order());
    target.sort_by_key(|layer| layer.draw_order());
    Ok(ValidatedWeaponLayers { current, target })
}

pub(crate) fn export_weapon_swap_from_layers(
    game_dir: &Path,
    output_path: &Path,
    current_visual_family: &str,
    target_visual_family: &str,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
) -> Result<OutfitSwapExportResult, String> {
    export_weapon_swap_from_layers_with_progress(
        game_dir,
        output_path,
        current_visual_family,
        target_visual_family,
        current_layers,
        target_layers,
        TargetVisualEffect::None,
        |_| {},
    )
}

pub(crate) fn export_weapon_swap_from_layers_with_progress<F>(
    game_dir: &Path,
    output_path: &Path,
    current_visual_family: &str,
    target_visual_family: &str,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    target_effect: TargetVisualEffect,
    mut report: F,
) -> Result<OutfitSwapExportResult, String>
where
    F: FnMut(OutfitSwapProgress),
{
    report(progress(0, "校验同视觉族武器资源"));
    let validated = validate_weapon_swap_layers(
        current_visual_family,
        target_visual_family,
        current_layers,
        target_layers,
    )?;
    let mut archives = HashMap::<PathBuf, NpkArchive>::new();
    let mut current_entries = BTreeMap::<String, CurrentWeaponEntry>::new();
    let mut preferred_generated_version = 4;

    report(progress(8, "读取当前武器载体"));
    for layer in &validated.current {
        let raw = load_raw_img_entry(game_dir, &layer.visual_img, &mut archives)?;
        let key = normalize_img_path_for_npk(&raw.entry_path);
        let source_hash = Sha256::digest(&raw.bytes).into();
        let img = ImgArchive::open_owned_bytes(raw.bytes)?;
        let source_version = img.version();
        let visible_frames = visible_frame_mask(&img, layer.palette_index, &raw.entry_path)?;
        let transparent_frames = transparent_frames_for_img(&img, &raw.entry_path)?;
        let palette_board_count = img
            .palette_count()
            .max(layer.palette_index.saturating_add(1));
        match current_entries.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(CurrentWeaponEntry {
                    entry_path: raw.entry_path,
                    source_hash,
                    source_version,
                    palette_board_count,
                    draw_order: layer.draw_order(),
                    visible_frames,
                    transparent_frames,
                });
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                if existing.source_hash != source_hash
                    || existing.source_version != source_version
                    || existing.transparent_frames != transparent_frames
                    || existing.visible_frames.len() != visible_frames.len()
                {
                    return Err(format!(
                        "当前武器 IMG 入口存在冲突：{}",
                        existing.entry_path
                    ));
                }
                existing.palette_board_count =
                    existing.palette_board_count.max(palette_board_count);
                for (existing_visible, visible) in
                    existing.visible_frames.iter_mut().zip(visible_frames)
                {
                    *existing_visible |= visible;
                }
                existing.draw_order = existing.draw_order.max(layer.draw_order());
            }
        }
    }
    if current_entries.is_empty() {
        return Err("当前武器没有可写入的载体 IMG".to_string());
    }

    report(progress(22, "读取并合成目标武器全部图层"));
    for layer in &validated.target {
        let raw = load_raw_img_entry(game_dir, &layer.visual_img, &mut archives)?;
        let version = ImgArchive::open_owned_bytes(raw.bytes)?.version();
        if version == 6 {
            preferred_generated_version = 6;
        }
    }
    if archives.keys().any(|source| same_path(source, output_path)) {
        return Err("输出补丁不能覆盖武器源 NPK".to_string());
    }
    let target_layers = validated
        .target
        .iter()
        .map(|layer| (*layer).clone())
        .collect::<Vec<_>>();
    let composed_frames =
        compose_weapon_part_frames_with_target_effect(game_dir, &target_layers, target_effect)?;
    let distributed_frames =
        distribute_composed_frames_by_current_visibility(&current_entries, &composed_frames)?;

    report(progress(58, "编码 V4/V6 武器 IMG"));
    let mut entries = BTreeMap::<String, Vec<u8>>::new();
    for (key, entry) in &current_entries {
        let output_version = match entry.source_version {
            4 | 6 => entry.source_version,
            _ => preferred_generated_version,
        };
        let frames = distributed_frames
            .get(key)
            .ok_or_else(|| format!("武器载体帧分配缺少入口：{}", entry.entry_path))?;
        let bytes = encode_part_img_bytes_v4_v6(frames, output_version, entry.palette_board_count)?;
        ensure_supported_weapon_version(&bytes)?;
        entries.insert(entry.entry_path.clone(), bytes);
    }

    report(progress(84, "写入独立武器换装补丁"));
    let mut builder = NpkBuilder::create()?;
    for (entry, bytes) in &entries {
        builder.add_entry(entry, bytes)?;
    }
    builder.save_path_create_new(output_path, false)?;

    report(progress(94, "校验武器 IMG 版本与像素"));
    if let Err(error) = validate_weapon_swap_output(output_path, &entries) {
        let _ = std::fs::remove_file(output_path);
        return Err(error);
    }

    report(progress(100, "武器换装补丁生成完成"));
    Ok(OutfitSwapExportResult {
        output_path: output_path.to_string_lossy().into_owned(),
        part_count: 1,
        entry_count: entries.len(),
    })
}

fn distribute_composed_frames_by_current_visibility(
    current_entries: &BTreeMap<String, CurrentWeaponEntry>,
    composed_frames: &[PartFrame],
) -> Result<BTreeMap<String, Vec<PartFrame>>, String> {
    if composed_frames.is_empty() {
        return Err("目标武器没有可分配的合成帧".to_string());
    }
    // Match the studio renderer: the longest IMG defines the animation cycle and
    // shorter IMGs loop inside that cycle. Using an LCM would lengthen the action.
    let frame_count = current_entries
        .values()
        .map(|entry| entry.visible_frames.len())
        .max()
        .unwrap_or(0);
    if frame_count == 0 {
        return Err("当前武器 IMG 没有帧".to_string());
    }

    let selected_carriers = (0..frame_count)
        .map(|frame_index| {
            current_entries
                .iter()
                .filter(|(_, entry)| entry.visible_frames[frame_index % entry.visible_frames.len()])
                .max_by(|(left_key, left), (right_key, right)| {
                    left.draw_order
                        .cmp(&right.draw_order)
                        .then_with(|| left_key.cmp(right_key))
                })
                .map(|(key, _)| key.as_str())
        })
        .collect::<Vec<_>>();
    if selected_carriers.iter().all(Option::is_none) {
        return Err("当前武器所有 IMG 帧均为透明，无法承载目标武器".to_string());
    }

    current_entries
        .iter()
        .map(|(key, entry)| {
            let frames = (0..frame_count)
                .map(|frame_index| {
                    if selected_carriers.get(frame_index).copied().flatten() == Some(key.as_str()) {
                        composed_frames[frame_index % composed_frames.len()].clone()
                    } else {
                        entry.transparent_frames[frame_index % entry.transparent_frames.len()]
                            .clone()
                    }
                })
                .collect::<Vec<_>>();
            Ok((key.clone(), frames))
        })
        .collect()
}

fn validate_weapon_swap_output(
    output_path: &Path,
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let patch = NpkArchive::open_path(output_path)?;
    if patch.entry_count() != entries.len() {
        return Err("武器换装补丁入口数量校验失败".to_string());
    }
    for (entry, expected) in entries {
        let index = patch
            .find_entry_index(entry)
            .ok_or_else(|| format!("武器换装补丁缺少入口：{entry}"))?;
        let actual = patch.entry_bytes(index)?;
        if actual != expected {
            return Err(format!("武器换装补丁 IMG 字节校验失败：{entry}"));
        }
        let img = ensure_supported_weapon_version(actual)?;
        for frame_index in 0..img.frame_count() {
            img.decode_frame(frame_index)
                .map_err(|error| format!("武器换装补丁 IMG 解码失败：{entry}：{error}"))?;
        }
    }
    Ok(())
}

fn normalized_weapon_layers<'a>(
    layers: &'a [AvatarLayer],
    label: &str,
) -> Result<Vec<&'a AvatarLayer>, String> {
    if layers
        .iter()
        .any(|layer| !layer.visual_img.trim().is_empty() && !is_weapon_layer(layer))
    {
        return Err(format!("{label}资源包含非武器图层，武器只能和武器换装"));
    }
    let weapon_layers = layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty() && is_weapon_layer(layer))
        .collect::<Vec<_>>();
    if weapon_layers.is_empty() {
        return Err(format!("{label}快照没有可读取的武器 IMG"));
    }
    let item_id = weapon_layers[0].item_id.trim();
    if item_id.is_empty()
        || weapon_layers
            .iter()
            .any(|layer| layer.item_id.trim() != item_id)
    {
        return Err(format!("{label}快照必须只包含一件武器"));
    }
    Ok(weapon_layers)
}

pub(crate) fn deduplicate_equivalent_weapon_layers(layers: Vec<&AvatarLayer>) -> Vec<&AvatarLayer> {
    let mut unique = Vec::<&AvatarLayer>::with_capacity(layers.len());
    for layer in layers {
        if unique
            .iter()
            .copied()
            .any(|existing| equivalent_animation_job_layer(existing, layer))
        {
            continue;
        }
        unique.push(layer);
    }
    unique
}

fn is_weapon_layer(layer: &AvatarLayer) -> bool {
    layer.equipment_type_enum == ORDINARY_WEAPON_EQUIPMENT_TYPE
        || (layer.equipment_type_enum == WEAPON_AVATAR_EQUIPMENT_TYPE
            && layer.slot.trim().eq_ignore_ascii_case("weapon"))
}

fn transparent_frames_for_img(
    img: &ImgArchive,
    entry_path: &str,
) -> Result<Vec<PartFrame>, String> {
    let frames = img
        .frames()
        .into_iter()
        .map(|metadata| PartFrame {
            width: 1,
            height: 1,
            x: metadata.canvas_width,
            y: 0,
            canvas_width: metadata.canvas_width,
            canvas_height: metadata.canvas_height,
            rgba: vec![0, 0, 0, 0],
        })
        .collect::<Vec<_>>();
    if frames.is_empty() {
        return Err(format!("当前武器 IMG 没有帧：{entry_path}"));
    }
    Ok(frames)
}

fn visible_frame_mask(
    img: &ImgArchive,
    palette_index: usize,
    entry_path: &str,
) -> Result<Vec<bool>, String> {
    let palette_index = if img.version() == 4 { 0 } else { palette_index };
    let expected_bytes = img.frames().iter().try_fold(0usize, |total, frame| {
        let frame_bytes = usize::try_from(frame.width.max(1))
            .ok()
            .and_then(|width| {
                usize::try_from(frame.height.max(1))
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| format!("当前武器 IMG 帧尺寸过大：{entry_path}"))?;
        total
            .checked_add(frame_bytes)
            .ok_or_else(|| format!("当前武器 IMG 解码尺寸溢出：{entry_path}"))
    })?;
    if expected_bytes > MAX_CURRENT_WEAPON_DECODE_BYTES {
        return Err(format!("当前武器 IMG 超过解码安全限制：{entry_path}"));
    }
    (0..img.frame_count())
        .map(|frame_index| {
            let decoded = img
                .decode_frame_with_palette(frame_index, palette_index)
                .map_err(|error| {
                    format!("读取当前武器有效帧失败：{entry_path} 第 {frame_index} 帧：{error}")
                })?;
            Ok(decoded.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0))
        })
        .collect()
}

fn load_raw_img_entry(
    game_dir: &Path,
    img_path: &str,
    archives: &mut HashMap<PathBuf, NpkArchive>,
) -> Result<RawImgEntry, String> {
    let npk_path = resolve_game_npk_path(game_dir, img_path)
        .ok_or_else(|| format!("无法定位武器 IMG 所属 NPK：{img_path}"))?;
    if !archives.contains_key(&npk_path) {
        archives.insert(npk_path.clone(), NpkArchive::open_path(&npk_path)?);
    }
    let archive = archives
        .get(&npk_path)
        .ok_or_else(|| "武器 NPK 缓存读取失败".to_string())?;
    for candidate in img_path_lookup_candidates(img_path) {
        if let Some(index) = archive.find_entry_index(&candidate) {
            return Ok(RawImgEntry {
                entry_path: archive
                    .entries()
                    .into_iter()
                    .find(|entry| entry.index == index)
                    .map(|entry| entry.name)
                    .unwrap_or_else(|| normalize_img_path_for_npk(&candidate)),
                bytes: archive.entry_bytes(index)?.to_vec(),
            });
        }
    }
    Err(format!("武器 NPK 中缺少 IMG：{img_path}"))
}

fn ensure_supported_weapon_version(bytes: &[u8]) -> Result<ImgArchive, String> {
    let img = ImgArchive::open_owned_bytes(bytes.to_vec())?;
    let version = img.version();
    if !matches!(version, 4 | 6) {
        return Err(format!(
            "武器换装输出 IMG V{version} 不受支持，只允许 V4/V6"
        ));
    }
    if img.palette_count() == 0
        || img
            .frames()
            .iter()
            .any(|frame| frame.format != 0x0e || frame.compress != 0x06)
    {
        return Err("武器换装输出必须全部使用索引色 ZLIB 帧".to_string());
    }
    Ok(img)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .replace('\\', "/")
        .eq_ignore_ascii_case(&right.to_string_lossy().replace('\\', "/"))
}

fn progress(percent: u8, stage: &str) -> OutfitSwapProgress {
    OutfitSwapProgress {
        percent,
        stage: stage.to_string(),
    }
}
