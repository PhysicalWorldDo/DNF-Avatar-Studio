use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use color_quant::NeuQuant;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::avatar_dye::{
    build_patched_img_bytes_many, canonicalize_recipe, select_equivalent_dye_layer, DyeRecipeV1,
    DyeTargetKindV1, DyeTargetRecipeV1,
};
use crate::dnflib_safe::{ImgArchive, NpkArchive, NpkBuilder};
use crate::dnflib_sys;
use crate::game_resolver::{
    normalize_img_path_for_npk, resolve_game_npk_path, resolve_img_resource,
    same_named_mask_number, ImgResourceMetadata,
};
use crate::models::{
    sort_avatar_layers_for_draw, visible_avatar_layers, AvatarLayer, AvatarLayerDrawOrder,
    OutfitSnapshot, OutfitSwapCarrierGroupPlan, OutfitSwapExportResult, OutfitSwapPartPlan,
    OutfitSwapPlan, OutfitSwapProgress, PreviewAnimation, PreviewImage,
};
use crate::renderer::{compose_animation_frames, compose_layer_frames, LayerFrame};

const IMG_FORMAT_ARGB_8888: i32 = 0x10;
const IMG_COMPRESS_NONE: i32 = 0x05;
const IMG_FORMAT_INDEXED: i32 = 0x0e;
const IMG_COMPRESS_ZLIB: i32 = 0x06;
const MAX_WEAPON_QUANTIZER_SAMPLE_PIXELS: usize = 65_536;
const MAX_DECODED_ENTRY_BYTES: usize = 128 * 1024 * 1024;
const MAX_DECODED_PART_BYTES: usize = 384 * 1024 * 1024;
const MAX_FLATTENED_SEQUENCE_BYTES: usize = 256 * 1024 * 1024;
const MAX_FLATTEN_CANVAS_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTFIT_DECODE_CACHE_BYTES: usize = MAX_DECODED_PART_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartFrame {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TargetVisualEffect {
    #[default]
    None,
    LinearDodge,
}

pub(crate) fn apply_linear_dodge_to_part_frames(frames: &mut [PartFrame]) {
    for frame in frames {
        for pixel in frame.rgba.chunks_exact_mut(4) {
            let Some([r, g, b, a]) = linear_dodge_emissive_pixel(
                u32::from(pixel[0]),
                u32::from(pixel[1]),
                u32::from(pixel[2]),
                u32::from(pixel[3]),
            ) else {
                pixel.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            };
            pixel.copy_from_slice(&[r, g, b, a]);
        }
    }
}

fn linear_dodge_emissive_pixel(r: u32, g: u32, b: u32, a: u32) -> Option<[u8; 4]> {
    let intensity = r.max(g).max(b).min(255);
    if intensity == 0 || a == 0 {
        return None;
    }

    let out_a = ((a.min(255) * intensity + 127) / 255).min(255);
    if out_a == 0 {
        return None;
    }

    Some([
        unmatte_black_channel(r, intensity),
        unmatte_black_channel(g, intensity),
        unmatte_black_channel(b, intensity),
        out_a as u8,
    ])
}

fn unmatte_black_channel(channel: u32, intensity: u32) -> u8 {
    ((channel.min(255) * 255 + intensity / 2) / intensity).min(255) as u8
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartLayerFrames {
    pub layer_order: i32,
    pub equipment_type_enum: u16,
    pub layer_sequence: u32,
    pub frames: Vec<PartFrame>,
}

impl PartLayerFrames {
    fn draw_order(&self) -> AvatarLayerDrawOrder {
        AvatarLayerDrawOrder::new(
            self.layer_order,
            self.equipment_type_enum,
            self.layer_sequence,
        )
    }
}

#[derive(Clone)]
struct GlobalCarrierNode {
    slot: String,
    item_id: String,
    item_name: String,
    draw_order: AvatarLayerDrawOrder,
    stable_ordinal: usize,
    entry_path: String,
}

#[derive(Clone)]
struct GlobalTargetNode {
    layer: AvatarLayer,
    stable_ordinal: usize,
}

#[derive(Clone)]
struct GlobalCarrierGroup {
    carrier: GlobalCarrierNode,
    targets: Vec<GlobalTargetNode>,
}

struct GlobalLayerMapping {
    strategy: &'static str,
    groups: Vec<GlobalCarrierGroup>,
    warnings: Vec<String>,
}

struct GenerationOutfitLayers {
    current: Vec<AvatarLayer>,
    target: Vec<AvatarLayer>,
    reserved_target_skin_item_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OutfitDecodedEntryKey {
    npk_path: PathBuf,
    entry_path: String,
    palette_index: usize,
    recipe_hash: [u8; 32],
    source_bytes_hash: [u8; 32],
}

#[derive(Default)]
struct OutfitResourceCache {
    archives: HashMap<PathBuf, Rc<NpkArchive>>,
    decoded_entries: HashMap<OutfitDecodedEntryKey, Rc<Vec<PartFrame>>>,
    decoded_entry_bytes: usize,
}

impl OutfitResourceCache {
    fn cached_decoded_entry(&self, key: &OutfitDecodedEntryKey) -> Option<Rc<Vec<PartFrame>>> {
        let frames = self.decoded_entries.get(key).cloned();
        frames
    }

    fn cache_decoded_entry(
        &mut self,
        key: OutfitDecodedEntryKey,
        frames: Rc<Vec<PartFrame>>,
        decoded_bytes: usize,
    ) {
        if self.decoded_entries.contains_key(&key) || decoded_bytes > MAX_OUTFIT_DECODE_CACHE_BYTES
        {
            return;
        }
        let Some(next_bytes) = self.decoded_entry_bytes.checked_add(decoded_bytes) else {
            return;
        };
        if next_bytes > MAX_OUTFIT_DECODE_CACHE_BYTES {
            return;
        }
        self.decoded_entries.insert(key, frames);
        self.decoded_entry_bytes = next_bytes;
    }
}

#[derive(Default)]
struct ResolvedDyeRecipe {
    recipe: DyeRecipeV1,
    patched_entries: BTreeMap<String, Vec<u8>>,
    dyed_base_count: usize,
    dyed_mask_count: usize,
    dyed_color_count: usize,
}

pub fn select_carrier_layer(layers: &[AvatarLayer]) -> Option<&AvatarLayer> {
    layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
        .min_by(|left, right| {
            let left_is_a = layer_tag_is_a(&left.layer_tag);
            let right_is_a = layer_tag_is_a(&right.layer_tag);
            right_is_a
                .cmp(&left_is_a)
                .then_with(|| left.draw_order().cmp(&right.draw_order()))
        })
}

pub fn validate_snapshot_pair(
    current: &OutfitSnapshot,
    target: &OutfitSnapshot,
) -> Result<(), String> {
    if current.item_ids.is_empty() || target.item_ids.is_empty() {
        return Err("current and target outfit snapshots must not be empty".to_string());
    }
    if current.playable_role_key.trim().is_empty()
        || target.playable_role_key.trim().is_empty()
        || current.visual_family_key.trim().is_empty()
        || target.visual_family_key.trim().is_empty()
        || current.resource_owner_key.trim().is_empty()
        || target.resource_owner_key.trim().is_empty()
        || !matches!(
            current
                .resource_generation
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "base" | "at"
        )
        || !matches!(
            target
                .resource_generation
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "base" | "at"
        )
    {
        return Err(
            "current and target snapshots must declare playable role and visual family".to_string(),
        );
    }
    if !current
        .resource_owner_key
        .eq_ignore_ascii_case(&target.resource_owner_key)
        || !current
            .resource_generation
            .eq_ignore_ascii_case(&target.resource_generation)
    {
        return Err("visual family resource binding mismatch".to_string());
    }
    if !current
        .visual_family_key
        .eq_ignore_ascii_case(&target.visual_family_key)
    {
        return Err(format!(
            "visual family mismatch: {} != {}",
            current.visual_family_key, target.visual_family_key
        ));
    }
    Ok(())
}

pub fn render_outfit_swap_previews_from_layers(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
) -> Result<(PreviewAnimation, PreviewAnimation), String> {
    render_outfit_swap_previews_from_layers_with_dye(
        game_dir,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        None,
        None,
    )
}

pub fn render_outfit_swap_previews_from_layers_with_dye(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_recipe: Option<&DyeRecipeV1>,
) -> Result<(PreviewAnimation, PreviewAnimation), String> {
    validate_snapshot_pair(current_snapshot, target_snapshot)?;
    let mut resources = OutfitResourceCache::default();
    let effective_current_layers =
        effective_outfit_layers(&mut resources, game_dir, current_snapshot, current_layers)?;
    let effective_target_layers =
        effective_outfit_layers(&mut resources, game_dir, target_snapshot, target_layers)?;
    let current_dye = resolve_dye_recipe(
        &mut resources,
        game_dir,
        current_recipe,
        &effective_current_layers,
    )
    .map_err(|_| "当前染色配方与当前资源不匹配，无法生成预览。".to_string())?;
    let target_dye = resolve_dye_recipe(
        &mut resources,
        game_dir,
        target_recipe,
        &effective_target_layers,
    )
    .map_err(|_| "目标染色配方与当前资源不匹配，无法生成预览。".to_string())?;
    let current = render_outfit_preview(
        &mut resources,
        game_dir,
        &effective_current_layers,
        &current_dye.patched_entries,
        current_dye.recipe.recipe_hash,
    )
    .map_err(|_| "当前换装预览生成失败。".to_string())?;
    let target = render_outfit_preview(
        &mut resources,
        game_dir,
        &effective_target_layers,
        &target_dye.patched_entries,
        target_dye.recipe.recipe_hash,
    )
    .map_err(|_| "目标换装预览生成失败。".to_string())?;
    Ok((current, target))
}

pub fn load_outfit_swap_page_from_layers_with_dye(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_recipe: Option<&DyeRecipeV1>,
) -> Result<(OutfitSwapPlan, PreviewAnimation, PreviewAnimation), String> {
    let mut resources = OutfitResourceCache::default();
    let plan = analyze_outfit_swap_with_resources(
        &mut resources,
        game_dir,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        current_recipe,
        target_recipe,
    )?;
    if plan.parts.iter().any(|part| part.action == "blocked") {
        return Err("目标染色配方与当前资源不匹配，无法生成页面预览。".to_string());
    }
    let effective_current_layers =
        effective_outfit_layers(&mut resources, game_dir, current_snapshot, current_layers)?;
    let effective_target_layers =
        effective_outfit_layers(&mut resources, game_dir, target_snapshot, target_layers)?;
    let current_dye = resolve_dye_recipe(
        &mut resources,
        game_dir,
        current_recipe,
        &effective_current_layers,
    )
    .map_err(|_| "当前染色配方与当前资源不匹配，无法生成页面预览。".to_string())?;
    let target_dye = resolve_dye_recipe(
        &mut resources,
        game_dir,
        target_recipe,
        &effective_target_layers,
    )
    .map_err(|_| "目标染色配方与当前资源不匹配，无法生成页面预览。".to_string())?;
    let current = render_outfit_preview(
        &mut resources,
        game_dir,
        &effective_current_layers,
        &current_dye.patched_entries,
        current_dye.recipe.recipe_hash,
    )
    .map_err(|_| "当前换装页面预览生成失败。".to_string())?;
    let target = render_outfit_preview(
        &mut resources,
        game_dir,
        &effective_target_layers,
        &target_dye.patched_entries,
        target_dye.recipe.recipe_hash,
    )
    .map_err(|_| "目标换装页面预览生成失败。".to_string())?;
    Ok((plan, current, target))
}

pub fn render_outfit_preview_from_layers_with_dye(
    game_dir: &Path,
    snapshot: &OutfitSnapshot,
    layers: &[AvatarLayer],
    recipe: Option<&DyeRecipeV1>,
) -> Result<PreviewAnimation, String> {
    render_outfit_preview_from_layers_with_dye_and_target_effect(
        game_dir,
        snapshot,
        layers,
        recipe,
        TargetVisualEffect::None,
    )
}

pub fn render_outfit_preview_from_layers_with_dye_and_target_effect(
    game_dir: &Path,
    snapshot: &OutfitSnapshot,
    layers: &[AvatarLayer],
    recipe: Option<&DyeRecipeV1>,
    target_effect: TargetVisualEffect,
) -> Result<PreviewAnimation, String> {
    let mut resources = OutfitResourceCache::default();
    let effective_layers = effective_outfit_layers(&mut resources, game_dir, snapshot, layers)?;
    let dye = resolve_dye_recipe(&mut resources, game_dir, recipe, &effective_layers)
        .map_err(|_| "染色配方与当前资源不匹配，无法生成预览。".to_string())?;
    render_outfit_preview_with_target_effect(
        &mut resources,
        game_dir,
        &effective_layers,
        &dye.patched_entries,
        dye.recipe.recipe_hash,
        target_effect,
    )
    .map_err(|_| "换装预览生成失败。".to_string())
}

pub fn render_outfit_preview_frame_from_layers_with_dye(
    game_dir: &Path,
    snapshot: &OutfitSnapshot,
    layers: &[AvatarLayer],
    recipe: Option<&DyeRecipeV1>,
    requested_frame_index: usize,
) -> Result<PreviewImage, String> {
    render_outfit_preview_frame_from_layers_with_dye_and_target_effect(
        game_dir,
        snapshot,
        layers,
        recipe,
        requested_frame_index,
        TargetVisualEffect::None,
    )
}

pub fn render_outfit_preview_frame_from_layers_with_dye_and_target_effect(
    game_dir: &Path,
    snapshot: &OutfitSnapshot,
    layers: &[AvatarLayer],
    recipe: Option<&DyeRecipeV1>,
    requested_frame_index: usize,
    target_effect: TargetVisualEffect,
) -> Result<PreviewImage, String> {
    let mut resources = OutfitResourceCache::default();
    let effective_layers = effective_outfit_layers(&mut resources, game_dir, snapshot, layers)?;
    let dye = resolve_dye_recipe(&mut resources, game_dir, recipe, &effective_layers)
        .map_err(|_| "染色配方与当前资源不匹配，无法生成预览。".to_string())?;
    render_outfit_preview_frame_with_target_effect(
        &mut resources,
        game_dir,
        &effective_layers,
        &dye.patched_entries,
        requested_frame_index,
        target_effect,
    )
    .map_err(|_| "换装预览生成失败。".to_string())
}

pub(crate) fn compose_effective_outfit_frames_with_dye_and_target_effect(
    game_dir: &Path,
    snapshot: &OutfitSnapshot,
    layers: &[AvatarLayer],
    recipe: Option<&DyeRecipeV1>,
    target_effect: TargetVisualEffect,
) -> Result<Vec<PartFrame>, String> {
    let mut resources = OutfitResourceCache::default();
    let effective_layers = effective_outfit_layers(&mut resources, game_dir, snapshot, layers)?;
    let dye = resolve_dye_recipe(&mut resources, game_dir, recipe, &effective_layers)
        .map_err(|_| "目标染色配方与当前资源不匹配，无法合成特殊载体。".to_string())?;
    let mut decoded = load_visible_part_frames(
        &mut resources,
        game_dir,
        &effective_layers,
        &dye.patched_entries,
        dye.recipe.recipe_hash,
    )?;
    apply_target_visual_effect_to_layers(&mut decoded, target_effect);
    flatten_part_frames(&decoded)
}

pub(crate) fn compose_weapon_part_frames_with_target_effect(
    game_dir: &Path,
    layers: &[AvatarLayer],
    target_effect: TargetVisualEffect,
) -> Result<Vec<PartFrame>, String> {
    let mut resources = OutfitResourceCache::default();
    let mut decoded =
        load_visible_part_frames(&mut resources, game_dir, layers, &BTreeMap::new(), [0; 32])?;
    apply_target_visual_effect_to_layers(&mut decoded, target_effect);
    flatten_part_frames(&decoded)
}

fn apply_target_visual_effect_to_layers(
    layers: &mut [PartLayerFrames],
    target_effect: TargetVisualEffect,
) {
    if target_effect == TargetVisualEffect::LinearDodge {
        for layer in layers {
            apply_linear_dodge_to_part_frames(&mut layer.frames);
        }
    }
}

fn render_outfit_preview(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    layers: &[AvatarLayer],
    patches: &BTreeMap<String, Vec<u8>>,
    recipe_hash: [u8; 32],
) -> Result<PreviewAnimation, String> {
    render_outfit_preview_with_target_effect(
        resources,
        game_dir,
        layers,
        patches,
        recipe_hash,
        TargetVisualEffect::None,
    )
}

fn render_outfit_preview_with_target_effect(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    layers: &[AvatarLayer],
    patches: &BTreeMap<String, Vec<u8>>,
    recipe_hash: [u8; 32],
    target_effect: TargetVisualEffect,
) -> Result<PreviewAnimation, String> {
    let mut ordered_layers = layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    sort_avatar_layers_for_draw(&mut ordered_layers);

    let mut available_layers = Vec::with_capacity(ordered_layers.len());
    let mut missing_imgs = Vec::new();
    for layer in ordered_layers {
        match resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index) {
            Ok(_) => available_layers.push(layer),
            Err(error) if error.starts_with("missing IMG") || error.starts_with("missing NPK") => {
                missing_imgs.push(layer.visual_img);
            }
            Err(error) => return Err(error),
        }
    }

    let mut decoded =
        load_visible_part_frames(resources, game_dir, &available_layers, patches, recipe_hash)?;
    apply_target_visual_effect_to_layers(&mut decoded, target_effect);
    let frame_count = decoded
        .iter()
        .map(|layer| layer.frames.len())
        .max()
        .unwrap_or(0)
        .max(1);
    let transparent = LayerFrame {
        width: 1,
        height: 1,
        x: 0,
        y: 0,
        rgba: vec![0, 0, 0, 0],
    };
    let layer_frames = decoded
        .into_iter()
        .map(|layer| {
            let mut frames = layer
                .frames
                .into_iter()
                .map(|frame| LayerFrame {
                    width: frame.width,
                    height: frame.height,
                    x: frame.x,
                    y: frame.y,
                    rgba: frame.rgba,
                })
                .collect::<Vec<_>>();
            frames.resize(frame_count, transparent.clone());
            frames
        })
        .collect::<Vec<_>>();
    let frames = compose_animation_frames(&layer_frames, Vec::new());
    Ok(PreviewAnimation {
        frame_count: frames.len(),
        missing_imgs,
        frames,
    })
}

fn render_outfit_preview_frame_with_target_effect(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    layers: &[AvatarLayer],
    patches: &BTreeMap<String, Vec<u8>>,
    requested_frame_index: usize,
    target_effect: TargetVisualEffect,
) -> Result<PreviewImage, String> {
    let mut ordered_layers = layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    sort_avatar_layers_for_draw(&mut ordered_layers);

    let mut available_layers = Vec::with_capacity(ordered_layers.len());
    let mut missing_imgs = Vec::new();
    for layer in ordered_layers {
        match resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index) {
            Ok(_) => available_layers.push(layer),
            Err(error) if error.starts_with("missing IMG") || error.starts_with("missing NPK") => {
                missing_imgs.push(layer.visual_img);
            }
            Err(error) => return Err(error),
        }
    }

    let mut decoded = load_visible_part_frame_at(
        resources,
        game_dir,
        &available_layers,
        patches,
        requested_frame_index,
    )?;
    apply_target_visual_effect_to_layers(&mut decoded, target_effect);
    decoded.sort_by_key(PartLayerFrames::draw_order);
    let frames = decoded
        .into_iter()
        .filter_map(|mut layer| layer.frames.pop())
        .map(|frame| LayerFrame {
            width: frame.width,
            height: frame.height,
            x: frame.x,
            y: frame.y,
            rgba: frame.rgba,
        })
        .collect::<Vec<_>>();
    Ok(compose_layer_frames(&frames, missing_imgs))
}

pub fn flatten_part_frames(layers: &[PartLayerFrames]) -> Result<Vec<PartFrame>, String> {
    flatten_part_frame_refs(layers.iter().collect())
}

fn flatten_part_frame_refs(mut ordered: Vec<&PartLayerFrames>) -> Result<Vec<PartFrame>, String> {
    let frame_count = ordered
        .iter()
        .map(|layer| layer.frames.len())
        .max()
        .unwrap_or(0);
    if frame_count == 0 {
        return Err("target part has no IMG frames".to_string());
    }

    ordered.sort_by_key(|layer| layer.draw_order());
    let mut flattened = Vec::with_capacity(frame_count);
    let mut retained_bytes = 0usize;
    for frame_index in 0..frame_count {
        let frame = flatten_frame_at(&ordered, frame_index)?;
        retained_bytes = retained_bytes
            .checked_add(frame.rgba.len())
            .ok_or_else(|| "flattened IMG sequence is too large".to_string())?;
        if retained_bytes > MAX_FLATTENED_SEQUENCE_BYTES {
            return Err("flattened IMG sequence exceeds the memory safety limit".to_string());
        }
        flattened.push(frame);
    }
    Ok(flattened)
}

fn is_default_skin_layer(layer: &AvatarLayer) -> bool {
    layer.item_id.eq_ignore_ascii_case("default_skin")
        && layer.status.eq_ignore_ascii_case("default_skin")
}

fn selected_skin_item_ids(snapshot: &OutfitSnapshot, layers: &[AvatarLayer]) -> BTreeSet<String> {
    let selected_item_ids = snapshot
        .item_ids
        .iter()
        .map(|item_id| item_id.trim())
        .filter(|item_id| !item_id.is_empty())
        .collect::<BTreeSet<_>>();
    layers
        .iter()
        .filter(|layer| {
            layer.slot.trim().eq_ignore_ascii_case("skin")
                && !is_default_skin_layer(layer)
                && selected_item_ids.contains(layer.item_id.trim())
        })
        .map(|layer| layer.item_id.trim().to_string())
        .collect()
}

fn generation_outfit_layers(
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_source_layers: &[AvatarLayer],
    target_source_layers: &[AvatarLayer],
    effective_current_layers: &[AvatarLayer],
    effective_target_layers: &[AvatarLayer],
) -> GenerationOutfitLayers {
    let current_skin_item_ids = selected_skin_item_ids(current_snapshot, current_source_layers);
    let target_skin_item_ids = selected_skin_item_ids(target_snapshot, target_source_layers);
    let reserve_default_skin = current_skin_item_ids.is_empty() && !target_skin_item_ids.is_empty();

    GenerationOutfitLayers {
        current: effective_current_layers
            .iter()
            .filter(|layer| !is_default_skin_layer(layer) || reserve_default_skin)
            .cloned()
            .collect(),
        target: effective_target_layers
            .iter()
            .filter(|layer| !is_default_skin_layer(layer))
            .cloned()
            .collect(),
        reserved_target_skin_item_ids: if reserve_default_skin {
            target_skin_item_ids
        } else {
            BTreeSet::new()
        },
    }
}

fn plan_global_layer_mapping(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    reserved_target_skin_item_ids: &BTreeSet<String>,
) -> Result<GlobalLayerMapping, String> {
    let default_skin_carrier = current_layers
        .iter()
        .find(|layer| is_default_skin_layer(layer) && !layer.visual_img.trim().is_empty())
        .map(|layer| {
            load_highest_current_part_carriers(resources, game_dir, std::slice::from_ref(layer))
        })
        .transpose()?
        .and_then(|mut carriers| carriers.pop());
    let reserve_target_skin = default_skin_carrier.is_some();
    let ordinary_current_layers = current_layers
        .iter()
        .filter(|layer| !is_default_skin_layer(layer))
        .cloned()
        .collect::<Vec<_>>();
    let carriers =
        load_highest_current_part_carriers(resources, game_dir, &ordinary_current_layers)?;
    let mut targets = Vec::new();
    let mut reserved_targets = Vec::new();
    for (stable_ordinal, layer) in target_layers.iter().enumerate() {
        if layer.visual_img.trim().is_empty() {
            continue;
        }
        let target = GlobalTargetNode {
            layer: layer.clone(),
            stable_ordinal,
        };
        if reserve_target_skin
            && layer.slot.trim().eq_ignore_ascii_case("skin")
            && reserved_target_skin_item_ids.contains(layer.item_id.trim())
        {
            reserved_targets.push(target);
        } else {
            targets.push(target);
        }
    }
    targets.sort_by(|left, right| {
        left.layer
            .draw_order()
            .cmp(&right.layer.draw_order())
            .then_with(|| left.stable_ordinal.cmp(&right.stable_ordinal))
    });
    let reserved_group = default_skin_carrier
        .filter(|_| !reserved_targets.is_empty())
        .map(|carrier| GlobalCarrierGroup {
            carrier,
            targets: reserved_targets,
        });
    if targets.is_empty() {
        return Ok(GlobalLayerMapping {
            strategy: if carriers.is_empty() && reserved_group.is_none() {
                "noop"
            } else {
                "global_layer_mapping"
            },
            groups: reserved_group.into_iter().collect(),
            warnings: Vec::new(),
        });
    }
    if carriers.is_empty() {
        return Ok(requires_single_carrier_mapping(
            "当前穿戴没有可用于标准全局映射的可见物理 IMG，请使用“单时装完整替代”。",
        ));
    }

    let target_item_count = targets
        .iter()
        .map(|target| target.layer.item_id.trim().to_string())
        .collect::<BTreeSet<_>>()
        .len();
    let require_multiple_current_items = target_item_count > 1;
    let mut selected_path = select_ordered_carrier_groups(&targets, &carriers);
    if require_multiple_current_items
        && !path_uses_multiple_current_carriers(&selected_path, &carriers)
    {
        let Some(multi_carrier_path) = select_ordered_multi_carrier_groups(&targets, &carriers)
        else {
            return Ok(requires_single_carrier_mapping(
                "多个目标物品只能落到一个物理 IMG，或全部载体属于同一件当前时装；标准模式已阻止，请使用“单时装完整替代”。",
            ));
        };
        selected_path = multi_carrier_path;
    }

    let mut targets_by_carrier = BTreeMap::<usize, Vec<GlobalTargetNode>>::new();
    for (target, carrier_index) in targets.into_iter().zip(selected_path) {
        targets_by_carrier
            .entry(carrier_index)
            .or_default()
            .push(target);
    }
    let mut groups = targets_by_carrier
        .into_iter()
        .map(|(carrier_index, targets)| GlobalCarrierGroup {
            carrier: carriers[carrier_index].clone(),
            targets,
        })
        .collect::<Vec<_>>();
    if let Some(group) = reserved_group {
        groups.push(group);
        groups.sort_by(|left, right| {
            left.carrier
                .draw_order
                .cmp(&right.carrier.draw_order)
                .then_with(|| {
                    left.carrier
                        .stable_ordinal
                        .cmp(&right.carrier.stable_ordinal)
                })
                .then_with(|| left.carrier.entry_path.cmp(&right.carrier.entry_path))
        });
    }
    Ok(GlobalLayerMapping {
        strategy: "global_layer_mapping",
        groups,
        warnings: Vec::new(),
    })
}

fn requires_single_carrier_mapping(warning: &str) -> GlobalLayerMapping {
    GlobalLayerMapping {
        strategy: "requires_single_carrier_mode",
        groups: Vec::new(),
        warnings: vec![warning.to_string()],
    }
}

fn load_highest_current_part_carriers(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    current_layers: &[AvatarLayer],
) -> Result<Vec<GlobalCarrierNode>, String> {
    let mut highest_by_slot = BTreeMap::<String, (usize, AvatarLayer)>::new();
    for (stable_ordinal, layer) in current_layers.iter().enumerate() {
        if layer.visual_img.trim().is_empty() {
            continue;
        }
        let slot = layer.slot.trim().to_ascii_lowercase();
        let replace = highest_by_slot.get(&slot).is_none_or(|(_, selected)| {
            layer.draw_order() > selected.draw_order()
                || (layer.draw_order() == selected.draw_order()
                    && layer_tag_is_a(&layer.layer_tag)
                    && !layer_tag_is_a(&selected.layer_tag))
        });
        if replace {
            highest_by_slot.insert(slot, (stable_ordinal, layer.clone()));
        }
    }

    let mut carriers = Vec::with_capacity(highest_by_slot.len());
    for (slot, (stable_ordinal, layer)) in highest_by_slot {
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        carriers.push(GlobalCarrierNode {
            slot,
            item_id: layer.item_id.trim().to_string(),
            item_name: layer.item_name.clone(),
            draw_order: layer.draw_order(),
            stable_ordinal,
            entry_path: family.base_entry,
        });
    }
    carriers.sort_by(|left, right| {
        left.draw_order
            .cmp(&right.draw_order)
            .then_with(|| left.stable_ordinal.cmp(&right.stable_ordinal))
            .then_with(|| left.entry_path.cmp(&right.entry_path))
    });
    Ok(carriers)
}

fn ordered_carrier_distance(
    target: &GlobalTargetNode,
    carrier: &GlobalCarrierNode,
) -> (u32, u64, u64, u64) {
    let target_order = target.layer.draw_order();
    (
        u32::from(target_order != carrier.draw_order),
        i64::from(target_order.layer_order).abs_diff(i64::from(carrier.draw_order.layer_order)),
        u64::from(
            target_order
                .equipment_type_enum
                .abs_diff(carrier.draw_order.equipment_type_enum),
        ),
        u64::from(
            target_order
                .layer_sequence
                .abs_diff(carrier.draw_order.layer_sequence),
        ),
    )
}

fn select_ordered_carrier_groups(
    targets: &[GlobalTargetNode],
    carriers: &[GlobalCarrierNode],
) -> Vec<usize> {
    let mut minimum_carrier = 0usize;
    targets
        .iter()
        .map(|target| {
            let selected = (minimum_carrier..carriers.len())
                .min_by_key(|index| (ordered_carrier_distance(target, &carriers[*index]), *index))
                .unwrap_or(minimum_carrier);
            minimum_carrier = selected;
            selected
        })
        .collect()
}

fn path_uses_multiple_current_carriers(
    selected_path: &[usize],
    carriers: &[GlobalCarrierNode],
) -> bool {
    let item_count = selected_path
        .iter()
        .map(|index| carriers[*index].item_id.trim())
        .collect::<BTreeSet<_>>()
        .len();
    let entry_count = selected_path
        .iter()
        .map(|index| normalized_entry_key(&carriers[*index].entry_path))
        .collect::<BTreeSet<_>>()
        .len();
    item_count >= 2 && entry_count >= 2
}

fn select_ordered_multi_carrier_groups(
    targets: &[GlobalTargetNode],
    carriers: &[GlobalCarrierNode],
) -> Option<Vec<usize>> {
    if targets.len() < 2 || carriers.len() < 2 {
        return None;
    }
    let last_target = targets.last()?;
    let (lower_carrier, upper_carrier) = (0..carriers.len())
        .flat_map(|lower| ((lower + 1)..carriers.len()).map(move |upper| (lower, upper)))
        .filter(|(lower, upper)| {
            carriers[*lower].item_id.trim() != carriers[*upper].item_id.trim()
                && normalized_entry_key(&carriers[*lower].entry_path)
                    != normalized_entry_key(&carriers[*upper].entry_path)
        })
        .min_by_key(|(lower, upper)| {
            let lower_distance = ordered_carrier_distance(&targets[0], &carriers[*lower]);
            let upper_distance = ordered_carrier_distance(last_target, &carriers[*upper]);
            (
                lower_distance.0 + upper_distance.0,
                lower_distance.1 + upper_distance.1,
                lower_distance.2 + upper_distance.2,
                lower_distance.3 + upper_distance.3,
                *lower,
                *upper,
            )
        })?;

    let final_target_index = targets.len() - 1;
    let mut minimum_carrier = lower_carrier;
    Some(
        targets
            .iter()
            .enumerate()
            .map(|(target_index, target)| {
                let selected = if target_index == 0 {
                    lower_carrier
                } else if target_index == final_target_index {
                    upper_carrier
                } else {
                    (minimum_carrier..=upper_carrier)
                        .min_by_key(|index| {
                            (ordered_carrier_distance(target, &carriers[*index]), *index)
                        })
                        .unwrap_or(minimum_carrier)
                };
                minimum_carrier = selected;
                selected
            })
            .collect(),
    )
}

fn effective_outfit_identity_equal(current: &[AvatarLayer], target: &[AvatarLayer]) -> bool {
    current.len() == target.len()
        && current.iter().zip(target).all(|(current, target)| {
            current.item_id.trim() == target.item_id.trim()
                && current.slot.eq_ignore_ascii_case(&target.slot)
                && current.draw_order() == target.draw_order()
                && normalize_img_path_for_npk(&current.visual_img)
                    == normalize_img_path_for_npk(&target.visual_img)
                && current.layer_tag == target.layer_tag
                && current.palette_index == target.palette_index
        })
}

pub fn encode_part_img_bytes(frames: &[PartFrame]) -> Result<Vec<u8>, String> {
    if frames.is_empty() {
        return Err("cannot encode an IMG without frames".to_string());
    }
    let encoded = frames
        .iter()
        .map(encode_frame_pixels)
        .collect::<Result<Vec<_>, _>>()?;
    let frame_count =
        i32::try_from(frames.len()).map_err(|_| "IMG has too many frames to encode".to_string())?;
    const FRAME_INDEX_SIZE: i32 = 9 * 4;
    let index_size = frame_count
        .checked_mul(FRAME_INDEX_SIZE)
        .ok_or_else(|| "IMG frame index is too large".to_string())?;
    let payload_size = encoded.iter().try_fold(0usize, |total, pixels| {
        let size = usize::try_from(pixels.size)
            .map_err(|_| "IMG frame encoded to an invalid size".to_string())?;
        total
            .checked_add(size)
            .ok_or_else(|| "IMG pixel payload is too large".to_string())
    })?;
    let capacity = 32usize
        .checked_add(index_size as usize)
        .and_then(|size| size.checked_add(payload_size))
        .ok_or_else(|| "IMG output is too large".to_string())?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(b"Neople Img File\0");
    push_i32(&mut bytes, index_size);
    push_i32(&mut bytes, 0);
    push_i32(&mut bytes, 2);
    push_i32(&mut bytes, frame_count);
    for (frame, pixels) in frames.iter().zip(&encoded) {
        let width = i32::try_from(frame.width)
            .map_err(|_| format!("IMG frame width {} is too large", frame.width))?;
        let height = i32::try_from(frame.height)
            .map_err(|_| format!("IMG frame height {} is too large", frame.height))?;
        push_i32(&mut bytes, IMG_FORMAT_ARGB_8888);
        push_i32(&mut bytes, IMG_COMPRESS_NONE);
        push_i32(&mut bytes, width);
        push_i32(&mut bytes, height);
        push_i32(&mut bytes, pixels.size);
        push_i32(&mut bytes, frame.x);
        push_i32(&mut bytes, frame.y);
        push_i32(&mut bytes, frame.canvas_width);
        push_i32(&mut bytes, frame.canvas_height);
    }
    for pixels in &encoded {
        // encode_frame_pixels guarantees a non-null pointer and a strictly positive size.
        let payload = unsafe { std::slice::from_raw_parts(pixels.data, pixels.size as usize) };
        bytes.extend_from_slice(payload);
    }
    Ok(bytes)
}

pub(crate) fn encode_part_img_bytes_v4_v6(
    frames: &[PartFrame],
    version: i32,
    palette_board_count: usize,
) -> Result<Vec<u8>, String> {
    if frames.is_empty() {
        return Err("cannot encode an IMG without frames".to_string());
    }
    if !matches!(version, 4 | 6) {
        return Err(format!(
            "weapon IMG output version must be V4 or V6, got V{version}"
        ));
    }
    let palette_board_count = palette_board_count.max(1);
    if palette_board_count > 256 {
        return Err("weapon IMG palette board count exceeds the safety limit".to_string());
    }

    let (palette, encoded) = encode_indexed_weapon_frames(frames)?;
    let frame_count =
        i32::try_from(frames.len()).map_err(|_| "IMG has too many frames to encode".to_string())?;
    const FRAME_INDEX_SIZE: i32 = 9 * 4;
    let index_size = frame_count
        .checked_mul(FRAME_INDEX_SIZE)
        .ok_or_else(|| "IMG frame index is too large".to_string())?;
    let payload_size = encoded.iter().try_fold(0usize, |total, pixels| {
        total
            .checked_add(pixels.len())
            .ok_or_else(|| "IMG pixel payload is too large".to_string())
    })?;
    let palette_board_bytes = 4usize
        .checked_add(
            palette
                .len()
                .checked_mul(4)
                .ok_or_else(|| "weapon IMG palette is too large".to_string())?,
        )
        .ok_or_else(|| "weapon IMG palette is too large".to_string())?;
    let palette_bytes = if version == 4 {
        palette_board_bytes
    } else {
        4usize
            .checked_add(
                palette_board_count
                    .checked_mul(palette_board_bytes)
                    .ok_or_else(|| "weapon IMG palette metadata is too large".to_string())?,
            )
            .ok_or_else(|| "weapon IMG palette metadata is too large".to_string())?
    };
    let capacity = 32usize
        .checked_add(palette_bytes)
        .and_then(|size| size.checked_add(index_size as usize))
        .and_then(|size| size.checked_add(payload_size))
        .ok_or_else(|| "IMG output is too large".to_string())?;

    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(b"Neople Img File\0");
    push_i32(&mut bytes, index_size);
    push_i32(&mut bytes, 0);
    push_i32(&mut bytes, version);
    push_i32(&mut bytes, frame_count);
    if version == 6 {
        push_i32(
            &mut bytes,
            i32::try_from(palette_board_count)
                .map_err(|_| "weapon IMG has too many palette boards".to_string())?,
        );
        for _ in 0..palette_board_count {
            push_weapon_palette_board(&mut bytes, &palette)?;
        }
    } else {
        push_weapon_palette_board(&mut bytes, &palette)?;
    }

    for (frame, pixels) in frames.iter().zip(&encoded) {
        let width = i32::try_from(frame.width)
            .map_err(|_| format!("IMG frame width {} is too large", frame.width))?;
        let height = i32::try_from(frame.height)
            .map_err(|_| format!("IMG frame height {} is too large", frame.height))?;
        push_i32(&mut bytes, IMG_FORMAT_INDEXED);
        push_i32(&mut bytes, IMG_COMPRESS_ZLIB);
        push_i32(&mut bytes, width);
        push_i32(&mut bytes, height);
        push_i32(
            &mut bytes,
            i32::try_from(pixels.len())
                .map_err(|_| "weapon IMG compressed frame is too large".to_string())?,
        );
        push_i32(&mut bytes, frame.x);
        push_i32(&mut bytes, frame.y);
        push_i32(&mut bytes, frame.canvas_width);
        push_i32(&mut bytes, frame.canvas_height);
    }
    for pixels in &encoded {
        bytes.extend_from_slice(pixels);
    }
    Ok(bytes)
}

fn encode_indexed_weapon_frames(
    frames: &[PartFrame],
) -> Result<(Vec<[u8; 4]>, Vec<Vec<u8>>), String> {
    let transparent = [0, 0, 0, 0];
    let mut has_transparency = false;
    for frame in frames {
        let expected = usize::try_from(frame.width)
            .ok()
            .and_then(|width| {
                usize::try_from(frame.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "weapon IMG frame dimensions are too large".to_string())?;
        if frame.width == 0 || frame.height == 0 || frame.rgba.len() != expected {
            return Err(format!(
                "weapon IMG frame has {} RGBA bytes, expected {expected}",
                frame.rgba.len()
            ));
        }
        has_transparency |= frame.rgba.chunks_exact(4).any(|pixel| pixel[3] == 0);
    }

    let mut palette = Vec::new();
    let mut color_indices = BTreeMap::new();
    if has_transparency {
        palette.push(transparent);
        color_indices.insert(transparent, 0_u8);
    }
    let mut indexed_frames = Vec::with_capacity(frames.len());
    let mut exact = true;
    'frames: for frame in frames {
        let expected = frame.rgba.len();
        let mut indices = Vec::with_capacity(expected / 4);
        for pixel in frame.rgba.chunks_exact(4) {
            let color = normalized_weapon_color(pixel);
            let index = if let Some(index) = color_indices.get(&color) {
                *index
            } else {
                if palette.len() >= 256 {
                    exact = false;
                    break 'frames;
                }
                let index = palette.len() as u8;
                palette.push(color);
                color_indices.insert(color, index);
                index
            };
            indices.push(index);
        }
        indexed_frames.push(indices);
    }

    if !exact {
        (palette, indexed_frames) = quantize_indexed_weapon_frames(frames, has_transparency)?;
    }
    let encoded = compress_weapon_index_frames(indexed_frames)?;
    Ok((palette, encoded))
}

fn quantize_indexed_weapon_frames(
    frames: &[PartFrame],
    has_transparency: bool,
) -> Result<(Vec<[u8; 4]>, Vec<Vec<u8>>), String> {
    let eligible_pixels = frames
        .iter()
        .flat_map(|frame| frame.rgba.chunks_exact(4))
        .filter(|pixel| !has_transparency || pixel[3] != 0)
        .count();
    if eligible_pixels == 0 {
        return Ok((
            vec![[0, 0, 0, 0]],
            frames
                .iter()
                .map(|frame| vec![0; frame.rgba.len() / 4])
                .collect(),
        ));
    }
    let stride = eligible_pixels.saturating_add(MAX_WEAPON_QUANTIZER_SAMPLE_PIXELS - 1)
        / MAX_WEAPON_QUANTIZER_SAMPLE_PIXELS;
    let mut sample = Vec::with_capacity(
        eligible_pixels
            .min(MAX_WEAPON_QUANTIZER_SAMPLE_PIXELS)
            .saturating_mul(4),
    );
    let mut ordinal = 0usize;
    for pixel in frames
        .iter()
        .flat_map(|frame| frame.rgba.chunks_exact(4))
        .filter(|pixel| !has_transparency || pixel[3] != 0)
    {
        if ordinal % stride == 0 && sample.len() / 4 < MAX_WEAPON_QUANTIZER_SAMPLE_PIXELS {
            sample.extend_from_slice(pixel);
        }
        ordinal += 1;
    }

    let palette_offset = usize::from(has_transparency);
    let quantizer = NeuQuant::new(1, 256 - palette_offset, &sample);
    let mut palette = Vec::with_capacity(256);
    if has_transparency {
        palette.push([0, 0, 0, 0]);
    }
    palette.extend(
        quantizer
            .color_map_rgba()
            .chunks_exact(4)
            .map(|color| [color[0], color[1], color[2], color[3]]),
    );

    let indexed_frames = frames
        .iter()
        .map(|frame| {
            frame
                .rgba
                .chunks_exact(4)
                .map(|pixel| {
                    if has_transparency && pixel[3] == 0 {
                        0
                    } else {
                        (quantizer.index_of(pixel) + palette_offset) as u8
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Ok((palette, indexed_frames))
}

fn normalized_weapon_color(pixel: &[u8]) -> [u8; 4] {
    if pixel[3] == 0 {
        [0, 0, 0, 0]
    } else {
        [pixel[0], pixel[1], pixel[2], pixel[3]]
    }
}

fn compress_weapon_index_frames(indexed_frames: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>, String> {
    let encoded = indexed_frames
        .into_iter()
        .map(|indices| {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(&indices)
                .map_err(|error| format!("compress weapon IMG indices failed: {error}"))?;
            encoder
                .finish()
                .map_err(|error| format!("finish weapon IMG index compression failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(encoded)
}

fn push_weapon_palette_board(bytes: &mut Vec<u8>, palette: &[[u8; 4]]) -> Result<(), String> {
    push_i32(
        bytes,
        i32::try_from(palette.len())
            .map_err(|_| "weapon IMG palette has too many colors".to_string())?,
    );
    for color in palette {
        bytes.extend_from_slice(color);
    }
    Ok(())
}

pub fn export_outfit_swap_from_layers(
    game_dir: &Path,
    output_path: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
) -> Result<OutfitSwapExportResult, String> {
    export_outfit_swap_from_layers_with_dye(
        game_dir,
        output_path,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        None,
        None,
    )
}

pub fn export_outfit_swap_from_layers_with_dye(
    game_dir: &Path,
    output_path: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_recipe: Option<&DyeRecipeV1>,
) -> Result<OutfitSwapExportResult, String> {
    export_outfit_swap_from_layers_with_dye_and_progress(
        game_dir,
        output_path,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        current_recipe,
        target_recipe,
        |_| {},
    )
}

pub fn export_outfit_swap_from_layers_with_progress<F>(
    game_dir: &Path,
    output_path: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    report: F,
) -> Result<OutfitSwapExportResult, String>
where
    F: FnMut(OutfitSwapProgress),
{
    export_outfit_swap_from_layers_with_dye_and_progress(
        game_dir,
        output_path,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        None,
        None,
        report,
    )
}

pub fn export_outfit_swap_from_layers_with_dye_and_progress<F>(
    game_dir: &Path,
    output_path: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_recipe: Option<&DyeRecipeV1>,
    report: F,
) -> Result<OutfitSwapExportResult, String>
where
    F: FnMut(OutfitSwapProgress),
{
    export_outfit_swap_from_layers_with_dye_and_target_effect_and_progress(
        game_dir,
        output_path,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        current_recipe,
        target_recipe,
        TargetVisualEffect::None,
        report,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn export_outfit_swap_from_layers_with_dye_and_target_effect_and_progress<F>(
    game_dir: &Path,
    output_path: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_recipe: Option<&DyeRecipeV1>,
    target_effect: TargetVisualEffect,
    mut report: F,
) -> Result<OutfitSwapExportResult, String>
where
    F: FnMut(OutfitSwapProgress),
{
    report(export_progress(0, "准备生成换装补丁"));
    validate_snapshot_pair(current_snapshot, target_snapshot)?;
    report(export_progress(3, "校验时装资源"));
    let mut resources = OutfitResourceCache::default();
    let effective_current_layers =
        effective_outfit_layers(&mut resources, game_dir, current_snapshot, current_layers)?;
    let effective_target_layers =
        effective_outfit_layers(&mut resources, game_dir, target_snapshot, target_layers)?;
    let generation_layers = generation_outfit_layers(
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        &effective_current_layers,
        &effective_target_layers,
    );
    let current_recipe = canonical_optional_recipe(current_recipe)?;
    resolve_dye_recipe(
        &mut resources,
        game_dir,
        current_recipe.as_ref(),
        &effective_current_layers,
    )
    .map_err(|_| "当前染色配方与当前资源不匹配，禁止导出。".to_string())?;
    let target_dye = resolve_dye_recipe(
        &mut resources,
        game_dir,
        target_recipe,
        &effective_target_layers,
    )
    .map_err(|_| "目标染色配方与当前资源不匹配，禁止导出。".to_string())?;
    report(export_progress(5, "分析全局图层映射"));
    let mapping = plan_global_layer_mapping(
        &mut resources,
        game_dir,
        &generation_layers.current,
        &generation_layers.target,
        &generation_layers.reserved_target_skin_item_ids,
    )?;
    let plan = build_global_outfit_swap_plan(
        &mut resources,
        game_dir,
        current_snapshot,
        &generation_layers.current,
        &generation_layers.target,
        current_recipe.as_ref(),
        &target_dye,
        &mapping,
    )?;
    if plan.strategy == "requires_single_carrier_mode" {
        return Err("global layer mapping requires single-carrier special generation".to_string());
    }
    if plan.strategy == "noop" {
        return Err("current and target outfits already have the same visual parts".to_string());
    }
    protect_source_npks(
        &mut resources,
        game_dir,
        output_path,
        &generation_layers.current,
        &generation_layers.target,
    )?;
    let clear_entries =
        load_clear_entry_frames(&mut resources, game_dir, &generation_layers.current)?;
    let current_entry_keys = clear_entries
        .iter()
        .map(|entry| normalized_entry_key(&entry.entry_path))
        .collect::<HashSet<_>>();
    let used_carrier_keys = mapping
        .groups
        .iter()
        .map(|group| normalized_entry_key(&group.carrier.entry_path))
        .collect::<HashSet<_>>();
    if let Some(group) = mapping.groups.iter().find(|group| {
        !current_entry_keys.contains(&normalized_entry_key(&group.carrier.entry_path))
    }) {
        return Err(format!(
            "global carrier {} is not an authorized current IMG entry",
            group.carrier.entry_path
        ));
    }

    let mut builder = NpkBuilder::create()?;
    let mut written = BTreeMap::<String, Vec<u8>>::new();
    let mut entry_count = 0usize;
    let total_steps = mapping.groups.len()
        + clear_entries
            .iter()
            .filter(|entry| !used_carrier_keys.contains(&normalized_entry_key(&entry.entry_path)))
            .count();
    let mut completed_steps = 0usize;
    report(export_progress(15, "写入全局载体组"));

    for group in &mapping.groups {
        let target_layers = group
            .targets
            .iter()
            .map(|target| target.layer.clone())
            .collect::<Vec<_>>();
        let mut target_frames = load_visible_part_frames(
            &mut resources,
            game_dir,
            &target_layers,
            &target_dye.patched_entries,
            target_dye.recipe.recipe_hash,
        )?;
        apply_target_visual_effect_to_layers(&mut target_frames, target_effect);
        let flattened = flatten_part_frames(&target_frames)?;
        let flattened_bytes = encode_part_img_bytes(&flattened)?;
        add_unique_entry(
            &mut builder,
            &mut written,
            &group.carrier.entry_path,
            &flattened_bytes,
            &mut entry_count,
        )?;
        completed_steps += 1;
        report(global_mapping_progress(completed_steps, total_steps));
    }

    for entry in &clear_entries {
        if used_carrier_keys.contains(&normalized_entry_key(&entry.entry_path)) {
            continue;
        }
        let transparent_bytes = encode_part_img_bytes(&entry.frames)?;
        add_unique_entry(
            &mut builder,
            &mut written,
            &entry.entry_path,
            &transparent_bytes,
            &mut entry_count,
        )?;
        completed_steps += 1;
        report(global_mapping_progress(completed_steps, total_steps));
    }

    if entry_count == 0 {
        return Err("current and target outfits already have the same visual parts".to_string());
    }
    let part_count = plan.parts.len();
    report(export_progress(95, "写入换装补丁文件"));
    builder.save_path_create_new(output_path, true)?;
    if let Err(error) = validate_outfit_swap_output(output_path, &written) {
        let _ = std::fs::remove_file(output_path);
        return Err(error);
    }
    report(export_progress(100, "换装补丁生成完成"));
    Ok(OutfitSwapExportResult {
        output_path: output_path.to_string_lossy().into_owned(),
        part_count,
        entry_count,
    })
}

fn export_progress(percent: u8, stage: &str) -> OutfitSwapProgress {
    OutfitSwapProgress {
        percent,
        stage: stage.to_string(),
    }
}

fn global_mapping_progress(completed: usize, total: usize) -> OutfitSwapProgress {
    let percent = if total == 0 {
        90
    } else {
        15 + ((completed.saturating_mul(75) / total).min(75) as u8)
    };
    export_progress(percent, &format!("处理全局图层入口 {completed}/{total}"))
}

pub fn analyze_outfit_swap_from_layers(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
) -> Result<OutfitSwapPlan, String> {
    analyze_outfit_swap_from_layers_with_dye(
        game_dir,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        None,
        None,
    )
}

pub fn analyze_outfit_swap_from_layers_with_dye(
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_recipe: Option<&DyeRecipeV1>,
) -> Result<OutfitSwapPlan, String> {
    let mut resources = OutfitResourceCache::default();
    analyze_outfit_swap_with_resources(
        &mut resources,
        game_dir,
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        current_recipe,
        target_recipe,
    )
}

fn analyze_outfit_swap_with_resources(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    target_snapshot: &OutfitSnapshot,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_recipe: Option<&DyeRecipeV1>,
) -> Result<OutfitSwapPlan, String> {
    validate_snapshot_pair(current_snapshot, target_snapshot)?;
    let effective_current_layers =
        effective_outfit_layers(resources, game_dir, current_snapshot, current_layers)?;
    let effective_target_layers =
        effective_outfit_layers(resources, game_dir, target_snapshot, target_layers)?;
    let generation_layers = generation_outfit_layers(
        current_snapshot,
        target_snapshot,
        current_layers,
        target_layers,
        &effective_current_layers,
        &effective_target_layers,
    );
    let current_recipe = canonical_optional_recipe(current_recipe)?;
    if resolve_dye_recipe(
        resources,
        game_dir,
        current_recipe.as_ref(),
        &effective_current_layers,
    )
    .is_err()
    {
        let blocked = analyze_outfit_swap_with_resources(
            resources,
            game_dir,
            current_snapshot,
            target_snapshot,
            current_layers,
            target_layers,
            None,
            target_recipe,
        )?;
        return Ok(block_outfit_swap_plan(
            blocked,
            "当前染色配方与当前资源不匹配，整套换装已阻止。",
        ));
    }
    let target_dye =
        match resolve_dye_recipe(resources, game_dir, target_recipe, &effective_target_layers) {
            Ok(target_dye) => target_dye,
            Err(_) => {
                let blocked = analyze_outfit_swap_with_resources(
                    resources,
                    game_dir,
                    current_snapshot,
                    target_snapshot,
                    current_layers,
                    target_layers,
                    current_recipe.as_ref(),
                    None,
                )?;
                return Ok(block_outfit_swap_plan(
                    blocked,
                    "目标染色配方与当前资源不匹配，整套换装已阻止。",
                ));
            }
        };
    let mapping = plan_global_layer_mapping(
        resources,
        game_dir,
        &generation_layers.current,
        &generation_layers.target,
        &generation_layers.reserved_target_skin_item_ids,
    )?;
    build_global_outfit_swap_plan(
        resources,
        game_dir,
        current_snapshot,
        &generation_layers.current,
        &generation_layers.target,
        current_recipe.as_ref(),
        &target_dye,
        &mapping,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_global_outfit_swap_plan(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    current_snapshot: &OutfitSnapshot,
    effective_current_layers: &[AvatarLayer],
    effective_target_layers: &[AvatarLayer],
    current_recipe: Option<&DyeRecipeV1>,
    target_dye: &ResolvedDyeRecipe,
    mapping: &GlobalLayerMapping,
) -> Result<OutfitSwapPlan, String> {
    let current_by_slot = layers_by_slot(effective_current_layers);
    let target_by_slot = layers_by_slot(effective_target_layers);

    if mapping.strategy == "requires_single_carrier_mode" {
        let mut parts = Vec::new();
        for (slot, target_layers) in &target_by_slot {
            let target_identity = target_layers
                .first()
                .ok_or_else(|| format!("target {slot} has no visual layer"))?;
            let metadata = inspect_target_part_metadata(resources, game_dir, target_layers)?;
            parts.push(OutfitSwapPartPlan {
                slot: slot.clone(),
                action: "requires_single_carrier".to_string(),
                current_item_name: String::new(),
                target_item_name: target_identity.item_name.clone(),
                carrier_img: String::new(),
                target_layer_count: target_layers.len(),
                output_frame_count: metadata.frame_count,
                canvas_width: metadata.canvas_width,
                canvas_height: metadata.canvas_height,
                transparent_imgs: Vec::new(),
                status: "blocked".to_string(),
                has_dye: false,
                dyed_base_count: 0,
                dyed_mask_count: 0,
                dyed_color_count: 0,
                recipe_revision: String::new(),
            });
        }
        return Ok(OutfitSwapPlan {
            profession_dir: current_snapshot.profession_dir.clone(),
            path_family: current_snapshot.path_family.clone(),
            strategy: mapping.strategy.to_string(),
            carrier_groups: Vec::new(),
            parts,
            warnings: mapping.warnings.clone(),
            dyed_base_count: 0,
            dyed_mask_count: 0,
            dyed_color_count: 0,
        });
    }

    let used_carriers = mapping
        .groups
        .iter()
        .map(|group| normalized_entry_key(&group.carrier.entry_path))
        .collect::<HashSet<_>>();
    let no_op = effective_outfit_identity_equal(effective_current_layers, effective_target_layers)
        && !recipe_affects_layers(current_recipe, effective_current_layers)
        && !recipe_affects_layers(Some(&target_dye.recipe), effective_target_layers);

    let mut carrier_groups = Vec::with_capacity(mapping.groups.len());
    for group in &mapping.groups {
        let target_layers = group
            .targets
            .iter()
            .map(|target| target.layer.clone())
            .collect::<Vec<_>>();
        let metadata = inspect_target_part_metadata(resources, game_dir, &target_layers)?;
        let mut target_slots = Vec::new();
        let mut target_item_names = Vec::new();
        for target in &group.targets {
            let slot = target.layer.slot.trim().to_ascii_lowercase();
            if !target_slots.contains(&slot) {
                target_slots.push(slot);
            }
            if !target_item_names.contains(&target.layer.item_name) {
                target_item_names.push(target.layer.item_name.clone());
            }
        }
        carrier_groups.push(OutfitSwapCarrierGroupPlan {
            carrier_slot: group.carrier.slot.clone(),
            current_item_name: group.carrier.item_name.clone(),
            carrier_layer_order: group.carrier.draw_order.layer_order,
            carrier_equipment_type_enum: group.carrier.draw_order.equipment_type_enum,
            carrier_layer_sequence: group.carrier.draw_order.layer_sequence,
            target_slots,
            target_item_names,
            target_layer_count: group.targets.len(),
            output_frame_count: metadata.frame_count,
            canvas_width: metadata.canvas_width,
            canvas_height: metadata.canvas_height,
            exact_order_match_count: group
                .targets
                .iter()
                .filter(|target| target.layer.draw_order() == group.carrier.draw_order)
                .count(),
            cross_slot_layer_count: group
                .targets
                .iter()
                .filter(|target| !target.layer.slot.eq_ignore_ascii_case(&group.carrier.slot))
                .count(),
        });
    }

    let mut parts = Vec::new();
    for (slot, target_layers) in &target_by_slot {
        let target_identity = target_layers
            .first()
            .ok_or_else(|| format!("target {slot} has no visual layer"))?;
        let primary_group = mapping
            .groups
            .iter()
            .filter(|group| {
                group
                    .targets
                    .iter()
                    .any(|target| target.layer.slot.eq_ignore_ascii_case(slot))
            })
            .min_by_key(|group| {
                group
                    .targets
                    .iter()
                    .filter(|target| target.layer.slot.eq_ignore_ascii_case(slot))
                    .map(|target| target.stable_ordinal)
                    .min()
                    .unwrap_or(usize::MAX)
            })
            .ok_or_else(|| format!("target {slot} has no global carrier group"))?;
        let current_part_layers = current_by_slot.get(slot);
        let has_current_dye =
            current_part_layers.is_some_and(|layers| recipe_affects_layers(current_recipe, layers));
        let has_target_dye = recipe_affects_layers(Some(&target_dye.recipe), target_layers);
        let action = if no_op {
            "keep"
        } else if let Some(current_part_layers) = current_part_layers {
            let current_identity = select_carrier_layer(current_part_layers)
                .ok_or_else(|| format!("current {slot} has no visual identity"))?;
            changed_part_action(
                current_part_layers,
                target_layers,
                current_identity,
                target_identity,
                has_current_dye,
                has_target_dye,
            )
        } else {
            "insert"
        };
        let metadata = inspect_target_part_metadata(resources, game_dir, target_layers)?;
        let transparent_imgs = if no_op {
            Vec::new()
        } else if let Some(current_part_layers) = current_part_layers {
            collect_current_entries(resources, game_dir, current_part_layers)?
                .into_iter()
                .filter(|entry| !used_carriers.contains(&normalized_entry_key(entry)))
                .collect()
        } else {
            Vec::new()
        };
        parts.push(OutfitSwapPartPlan {
            slot: slot.clone(),
            action: action.to_string(),
            current_item_name: primary_group.carrier.item_name.clone(),
            target_item_name: target_identity.item_name.clone(),
            carrier_img: primary_group.carrier.entry_path.clone(),
            target_layer_count: target_layers.len(),
            output_frame_count: if no_op { 0 } else { metadata.frame_count },
            canvas_width: if no_op { 0 } else { metadata.canvas_width },
            canvas_height: if no_op { 0 } else { metadata.canvas_height },
            transparent_imgs,
            status: "ready".to_string(),
            has_dye: false,
            dyed_base_count: 0,
            dyed_mask_count: 0,
            dyed_color_count: 0,
            recipe_revision: String::new(),
        });
    }

    for (slot, current_layers) in current_by_slot
        .iter()
        .filter(|(slot, _)| !target_by_slot.contains_key(*slot))
    {
        let (mut part, _) = analyze_clear_part(resources, game_dir, slot, current_layers)?;
        let borrowed_groups = mapping
            .groups
            .iter()
            .filter(|group| group.carrier.slot.eq_ignore_ascii_case(slot))
            .collect::<Vec<_>>();
        if !borrowed_groups.is_empty() {
            part.action = "lend".to_string();
            part.carrier_img = borrowed_groups[0].carrier.entry_path.clone();
            part.target_item_name = borrowed_groups
                .iter()
                .flat_map(|group| group.targets.iter())
                .map(|target| target.layer.item_name.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join("、");
        }
        part.transparent_imgs = collect_current_entries(resources, game_dir, current_layers)?
            .into_iter()
            .filter(|entry| !used_carriers.contains(&normalized_entry_key(entry)))
            .collect();
        parts.push(part);
    }

    if parts.is_empty() && !no_op {
        return Err("current and target snapshots have no replaceable parts".to_string());
    }
    apply_part_dye_stats(&mut parts, &target_dye.recipe);
    Ok(OutfitSwapPlan {
        profession_dir: current_snapshot.profession_dir.clone(),
        path_family: current_snapshot.path_family.clone(),
        strategy: if no_op {
            "noop".to_string()
        } else {
            mapping.strategy.to_string()
        },
        carrier_groups,
        parts,
        warnings: mapping.warnings.clone(),
        dyed_base_count: target_dye.dyed_base_count,
        dyed_mask_count: target_dye.dyed_mask_count,
        dyed_color_count: target_dye.dyed_color_count,
    })
}

fn block_outfit_swap_plan(mut plan: OutfitSwapPlan, warning: &str) -> OutfitSwapPlan {
    plan.strategy = "blocked".to_string();
    plan.carrier_groups.clear();
    for part in &mut plan.parts {
        part.action = "blocked".to_string();
        part.status = "blocked".to_string();
        part.has_dye = false;
        part.dyed_base_count = 0;
        part.dyed_mask_count = 0;
        part.dyed_color_count = 0;
        part.recipe_revision.clear();
    }
    plan.dyed_base_count = 0;
    plan.dyed_mask_count = 0;
    plan.dyed_color_count = 0;
    plan.warnings.clear();
    plan.warnings.push(warning.to_string());
    plan
}

fn recipe_affects_layers(recipe: Option<&DyeRecipeV1>, layers: &[AvatarLayer]) -> bool {
    recipe.is_some_and(|recipe| {
        recipe.targets.iter().any(|target| {
            !target.colors.is_empty()
                && layers.iter().any(|layer| {
                    target
                        .target
                        .slot
                        .trim()
                        .eq_ignore_ascii_case(layer.slot.trim())
                        && target.target.item_ref.trim() == layer.item_id.trim()
                        && target.target.layer_tag.trim() == layer.layer_tag.trim()
                })
        })
    })
}

fn same_item_id(current: &AvatarLayer, target: &AvatarLayer) -> bool {
    current.item_id.trim() == target.item_id.trim()
}

fn changed_part_action(
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
    current_identity: &AvatarLayer,
    target_identity: &AvatarLayer,
    has_valid_current_dye: bool,
    has_valid_target_dye: bool,
) -> &'static str {
    if same_item_id(current_identity, target_identity)
        && (part_layers_equal(current_layers, target_layers)
            || has_valid_current_dye
            || has_valid_target_dye)
    {
        "recolor"
    } else {
        "replace"
    }
}

fn canonical_optional_recipe(recipe: Option<&DyeRecipeV1>) -> Result<Option<DyeRecipeV1>, String> {
    recipe
        .map(|recipe| {
            canonicalize_recipe(recipe.clone())
                .map_err(|error| format!("invalid dye recipe: {error}"))
        })
        .transpose()
}

fn dye_layer_match_error(
    matching_layer_count: usize,
    slot: &str,
    item_ref: &str,
    layer_tag: &str,
) -> String {
    let reason = if matching_layer_count == 0 {
        "does not match any outfit layer"
    } else {
        "matches multiple non-equivalent outfit layers"
    };
    format!("dye recipe target {reason}: {slot}/{item_ref}/{layer_tag}")
}

fn resolve_dye_recipe(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    recipe: Option<&DyeRecipeV1>,
    layers: &[AvatarLayer],
) -> Result<ResolvedDyeRecipe, String> {
    let Some(recipe) = canonical_optional_recipe(recipe)? else {
        return Ok(ResolvedDyeRecipe::default());
    };
    let mut pending_entries = BTreeMap::<String, (String, Vec<u8>, Vec<&DyeTargetRecipeV1>)>::new();
    let mut dyed_base_count = 0usize;
    let mut dyed_mask_count = 0usize;
    let mut dyed_color_count = 0usize;
    for target in &recipe.targets {
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
            })
            .collect::<Vec<_>>();
        let Some(layer) = select_equivalent_dye_layer(&matching_layers) else {
            return Err(dye_layer_match_error(
                matching_layers.len(),
                &target.target.slot,
                &target.target.item_ref,
                &target.target.layer_tag,
            ));
        };
        if let DyeTargetKindV1::Mask { number } = target.target.target_kind {
            if !layer.enable_dye_mask(number) {
                return Err(format!(
                    "dye recipe targets Mask {number} disabled by the item's [enable dye] rule"
                ));
            }
        }
        let expected_palette = match target.target.target_kind {
            DyeTargetKindV1::Base => layer.palette_index,
            DyeTargetKindV1::Mask { .. } => 0,
        };
        if usize::from(target.palette_index) != expected_palette {
            return Err(format!(
                "dye recipe palette does not match the effective outfit layer: {}",
                target.target.private_entry_ref
            ));
        }
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        let matching_entries = family
            .entries
            .iter()
            .filter(|entry| match target.target.target_kind {
                DyeTargetKindV1::Base => entry.eq_ignore_ascii_case(&family.base_entry),
                DyeTargetKindV1::Mask { number } => {
                    same_named_mask_number(&family.base_entry, entry) == Some(number)
                }
            })
            .collect::<Vec<_>>();
        let [entry_path] = matching_entries.as_slice() else {
            return Err(format!(
                "dye recipe target does not match exactly one IMG family entry: {}",
                target.target.private_entry_ref
            ));
        };
        if normalized_entry_key(entry_path)
            != normalized_entry_key(&target.target.private_entry_ref)
        {
            return Err(format!(
                "dye recipe target IMG entry changed: {}",
                target.target.private_entry_ref
            ));
        }
        let entry_index = family
            .archive
            .find_entry_index(entry_path)
            .ok_or_else(|| format!("missing dye target IMG entry {entry_path}"))?;
        let source_bytes = family.archive.entry_bytes(entry_index)?;
        let entry_key = normalized_entry_key(entry_path);
        let pending = pending_entries
            .entry(entry_key)
            .or_insert_with(|| ((*entry_path).clone(), source_bytes.to_vec(), Vec::new()));
        if pending.1 != source_bytes {
            return Err(format!(
                "conflicting source bytes for duplicate IMG entry {entry_path}"
            ));
        }
        pending.2.push(target);
        match target.target.target_kind {
            DyeTargetKindV1::Base => dyed_base_count += 1,
            DyeTargetKindV1::Mask { .. } => {
                dyed_mask_count += 1;
            }
        }
        dyed_color_count += target.colors.len();
    }
    let mut patched_entries = BTreeMap::new();
    for (entry_key, (entry_path, source_bytes, targets)) in pending_entries {
        let patched = build_patched_img_bytes_many(&source_bytes, &targets)
            .map_err(|error| format!("stale dye recipe target {entry_path}: {error}"))?;
        patched_entries.insert(entry_key, patched);
    }
    Ok(ResolvedDyeRecipe {
        recipe,
        patched_entries,
        dyed_base_count,
        dyed_mask_count,
        dyed_color_count,
    })
}

fn normalized_entry_key(value: &str) -> String {
    normalize_img_path_for_npk(value)
}

fn apply_part_dye_stats(parts: &mut [OutfitSwapPartPlan], recipe: &DyeRecipeV1) {
    let revision = recipe
        .recipe_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    for part in parts {
        let mut dyed_base_count = 0;
        let mut dyed_mask_count = 0;
        let mut dyed_color_count = 0;
        for target in recipe.targets.iter().filter(|target| {
            !target.colors.is_empty()
                && target
                    .target
                    .slot
                    .trim()
                    .eq_ignore_ascii_case(part.slot.trim())
        }) {
            match target.target.target_kind {
                DyeTargetKindV1::Base => dyed_base_count += 1,
                DyeTargetKindV1::Mask { .. } => dyed_mask_count += 1,
            }
            dyed_color_count += target.colors.len();
        }
        part.has_dye = dyed_color_count > 0;
        part.dyed_base_count = dyed_base_count;
        part.dyed_mask_count = dyed_mask_count;
        part.dyed_color_count = dyed_color_count;
        if part.has_dye {
            part.recipe_revision.clone_from(&revision);
        }
    }
}

fn part_frame_bytes(frames: &[PartFrame]) -> Result<usize, String> {
    frames.iter().try_fold(0usize, |total, frame| {
        total
            .checked_add(frame.rgba.len())
            .ok_or_else(|| "decoded IMG frames are too large".to_string())
    })
}

struct EncodedPixels {
    data: *mut u8,
    size: i32,
}

impl Drop for EncodedPixels {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe { dnflib_sys::dnf_free(self.data.cast()) };
        }
    }
}

fn encode_frame_pixels(frame: &PartFrame) -> Result<EncodedPixels, String> {
    if frame.width == 0 || frame.height == 0 {
        return Err("cannot encode an empty frame dimension".to_string());
    }
    let expected = frame.width as usize * frame.height as usize * 4;
    if frame.rgba.len() != expected {
        return Err(format!(
            "IMG frame has {} RGBA bytes, expected {expected}",
            frame.rgba.len()
        ));
    }
    let mut data = std::ptr::null_mut();
    let mut size = 0;
    let rc = unsafe {
        dnflib_sys::dnf_image_encode(
            frame.rgba.as_ptr(),
            frame.width as i32,
            frame.height as i32,
            IMG_FORMAT_ARGB_8888,
            IMG_COMPRESS_NONE,
            &mut data,
            &mut size,
        )
    };
    if rc != dnflib_sys::DNF_OK || data.is_null() || size <= 0 {
        return Err(format!("dnflib encode outfit IMG frame failed: {rc}"));
    }
    Ok(EncodedPixels { data, size })
}

struct ResolvedImgFamily {
    base_entry: String,
    palette_index: usize,
    npk_path: PathBuf,
    archive: Rc<NpkArchive>,
    entries: Vec<String>,
}

fn img_family_entry_authorized(layer: &AvatarLayer, base_entry: &str, entry_path: &str) -> bool {
    same_named_mask_number(base_entry, entry_path)
        .is_none_or(|number| layer.enable_dye_mask(number))
}

fn img_family_entry_is_visible(base_entry: &str, entry_path: &str) -> bool {
    same_named_mask_number(base_entry, entry_path).is_none()
}

fn resolve_img_family(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    img_path: &str,
    requested_palette_index: usize,
) -> Result<ResolvedImgFamily, String> {
    let mut saw_npk = false;
    let resolved = resolve_img_resource(
        img_path,
        requested_palette_index,
        |entry_path| -> Result<_, String> {
            let Some(npk_path) = resolve_game_npk_path(game_dir, &entry_path) else {
                return Ok(None);
            };
            if !npk_path.is_file() {
                return Ok(None);
            }
            saw_npk = true;
            let archive = if let Some(archive) = resources.archives.get(&npk_path) {
                Rc::clone(archive)
            } else {
                let archive = Rc::new(NpkArchive::open_path(&npk_path)?);
                resources
                    .archives
                    .insert(npk_path.clone(), Rc::clone(&archive));
                archive
            };
            let Some(entry_index) = archive.find_entry_index(entry_path) else {
                return Ok(None);
            };
            let img = archive.read_img(entry_index)?;
            Ok(Some((
                (npk_path, archive),
                ImgResourceMetadata {
                    version: img.version(),
                    palette_count: img.palette_count(),
                },
            )))
        },
    )?;
    if let Some(resolved) = resolved {
        let (npk_path, archive) = resolved.value;
        let mut related = archive
            .entries()
            .into_iter()
            .filter_map(|entry| {
                if entry.name.eq_ignore_ascii_case(&resolved.entry_path) {
                    Some((0_u32, entry.name))
                } else {
                    same_named_mask_number(&resolved.entry_path, &entry.name)
                        .map(|number| (number, entry.name))
                }
            })
            .collect::<Vec<_>>();
        related.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        return Ok(ResolvedImgFamily {
            base_entry: resolved.entry_path,
            palette_index: resolved.palette_index,
            npk_path,
            archive,
            entries: related.into_iter().map(|(_, path)| path).collect(),
        });
    }
    if saw_npk {
        Err(format!("missing IMG entry for {img_path}"))
    } else {
        Err(format!("missing NPK for {img_path}"))
    }
}

fn load_visible_part_frames(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    layers: &[AvatarLayer],
    patches: &BTreeMap<String, Vec<u8>>,
    recipe_hash: [u8; 32],
) -> Result<Vec<PartLayerFrames>, String> {
    let mut result = Vec::new();
    let mut decoded_part_bytes = 0usize;
    for layer in layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
    {
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        for entry_path in &family.entries {
            if !img_family_entry_is_visible(&family.base_entry, entry_path) {
                continue;
            }
            let entry_index = family
                .archive
                .find_entry_index(entry_path)
                .ok_or_else(|| format!("missing related IMG entry {entry_path}"))?;
            let source_bytes = if let Some(patched) = patches.get(&normalized_entry_key(entry_path))
            {
                patched.as_slice()
            } else {
                family.archive.entry_bytes(entry_index)?
            };
            let palette_index = family.palette_index;
            let source_bytes_hash = Sha256::digest(source_bytes).into();
            let cache_key = OutfitDecodedEntryKey {
                npk_path: family.npk_path.clone(),
                entry_path: normalized_entry_key(entry_path),
                palette_index,
                recipe_hash,
                source_bytes_hash,
            };
            let frames = if let Some(frames) = resources.cached_decoded_entry(&cache_key) {
                frames
            } else {
                let img = ImgArchive::open_owned_bytes(source_bytes.to_vec())?;
                let metadata = img.frames();
                let expected_bytes = metadata.iter().try_fold(0usize, |total, frame| {
                    let frame_bytes = usize::try_from(frame.width.max(1))
                        .ok()
                        .and_then(|width| {
                            usize::try_from(frame.height.max(1))
                                .ok()
                                .and_then(|height| width.checked_mul(height))
                        })
                        .and_then(|pixels| pixels.checked_mul(4))
                        .ok_or_else(|| "decoded IMG frame dimensions are too large".to_string())?;
                    total
                        .checked_add(frame_bytes)
                        .ok_or_else(|| "decoded IMG entry is too large".to_string())
                })?;
                if expected_bytes > MAX_DECODED_ENTRY_BYTES {
                    return Err("decoded IMG entry exceeds the memory safety limit".to_string());
                }
                let frames = Rc::new(
                    metadata
                        .into_iter()
                        .map(|metadata| {
                            let decoded = img
                                .decode_frame_with_palette(metadata.index, palette_index)
                                .map_err(|err| {
                                    format!("{entry_path} frame {}: {err}", metadata.index)
                                })?;
                            Ok(PartFrame {
                                width: decoded.width.max(1) as u32,
                                height: decoded.height.max(1) as u32,
                                x: decoded.x,
                                y: decoded.y,
                                canvas_width: decoded.canvas_width,
                                canvas_height: decoded.canvas_height,
                                rgba: decoded.rgba,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                );
                let decoded_bytes = part_frame_bytes(&frames)?;
                resources.cache_decoded_entry(cache_key, Rc::clone(&frames), decoded_bytes);
                frames
            };
            let decoded_bytes = part_frame_bytes(&frames)?;
            decoded_part_bytes = decoded_part_bytes
                .checked_add(decoded_bytes)
                .ok_or_else(|| "decoded IMG part is too large".to_string())?;
            if decoded_part_bytes > MAX_DECODED_PART_BYTES {
                return Err("decoded IMG part exceeds the memory safety limit".to_string());
            }
            result.push(PartLayerFrames {
                layer_order: layer.layer_order,
                equipment_type_enum: layer.equipment_type_enum,
                layer_sequence: layer.layer_sequence,
                frames: frames.as_ref().clone(),
            });
        }
    }
    let animation_frame_count = result
        .iter()
        .map(|layer| layer.frames.len())
        .max()
        .unwrap_or(0);
    for layer in &mut result {
        if layer.frames.is_empty() || layer.frames.len() >= animation_frame_count {
            continue;
        }
        let original = layer.frames.clone();
        layer.frames.extend(
            (layer.frames.len()..animation_frame_count)
                .map(|index| original[index % original.len()].clone()),
        );
    }
    let retained_bytes = result.iter().try_fold(0usize, |total, layer| {
        total
            .checked_add(part_frame_bytes(&layer.frames)?)
            .ok_or_else(|| "decoded IMG part is too large".to_string())
    })?;
    if retained_bytes > MAX_DECODED_PART_BYTES {
        return Err("decoded IMG part exceeds the memory safety limit".to_string());
    }
    Ok(result)
}

struct SingleFrameEntry {
    layer_order: i32,
    equipment_type_enum: u16,
    layer_sequence: u32,
    entry_path: String,
    palette_index: usize,
    img: ImgArchive,
    frame_count: usize,
}

fn load_visible_part_frame_at(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    layers: &[AvatarLayer],
    patches: &BTreeMap<String, Vec<u8>>,
    requested_frame_index: usize,
) -> Result<Vec<PartLayerFrames>, String> {
    let mut entries = Vec::new();
    let mut animation_frame_count = 0usize;
    for layer in layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
    {
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        for entry_path in &family.entries {
            if !img_family_entry_is_visible(&family.base_entry, entry_path) {
                continue;
            }
            let entry_index = family
                .archive
                .find_entry_index(entry_path)
                .ok_or_else(|| format!("missing related IMG entry {entry_path}"))?;
            let source_bytes = patches
                .get(&normalized_entry_key(entry_path))
                .map(Vec::as_slice)
                .unwrap_or(family.archive.entry_bytes(entry_index)?);
            let img = ImgArchive::open_owned_bytes(source_bytes.to_vec())?;
            let frame_count = img.frames().len();
            animation_frame_count = animation_frame_count.max(frame_count);
            entries.push(SingleFrameEntry {
                layer_order: layer.layer_order,
                equipment_type_enum: layer.equipment_type_enum,
                layer_sequence: layer.layer_sequence,
                entry_path: entry_path.clone(),
                palette_index: family.palette_index,
                img,
                frame_count,
            });
        }
    }

    let selected_frame_index = if animation_frame_count == 0 {
        0
    } else {
        requested_frame_index % animation_frame_count
    };
    let mut decoded_part_bytes = 0usize;
    let mut result = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.frame_count == 0 {
            continue;
        }
        let metadata = entry.img.frames();
        let actual_frame_index = selected_frame_index % entry.frame_count;
        let frame_metadata = metadata
            .get(actual_frame_index)
            .ok_or_else(|| format!("{} frame metadata is missing", entry.entry_path))?;
        let expected_bytes = usize::try_from(frame_metadata.width.max(1))
            .ok()
            .and_then(|width| {
                usize::try_from(frame_metadata.height.max(1))
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "decoded IMG frame dimensions are too large".to_string())?;
        if expected_bytes > MAX_DECODED_ENTRY_BYTES {
            return Err("decoded IMG entry exceeds the memory safety limit".to_string());
        }
        decoded_part_bytes = decoded_part_bytes
            .checked_add(expected_bytes)
            .ok_or_else(|| "decoded IMG part is too large".to_string())?;
        if decoded_part_bytes > MAX_DECODED_PART_BYTES {
            return Err("decoded IMG part exceeds the memory safety limit".to_string());
        }
        let decoded = entry
            .img
            .decode_frame_with_palette(frame_metadata.index, entry.palette_index)
            .map_err(|error| {
                format!(
                    "{} frame {}: {error}",
                    entry.entry_path, frame_metadata.index
                )
            })?;
        result.push(PartLayerFrames {
            layer_order: entry.layer_order,
            equipment_type_enum: entry.equipment_type_enum,
            layer_sequence: entry.layer_sequence,
            frames: vec![PartFrame {
                width: decoded.width.max(1) as u32,
                height: decoded.height.max(1) as u32,
                x: decoded.x,
                y: decoded.y,
                canvas_width: decoded.canvas_width,
                canvas_height: decoded.canvas_height,
                rgba: decoded.rgba,
            }],
        });
    }
    Ok(result)
}

struct PartMetadata {
    frame_count: usize,
    canvas_width: i32,
    canvas_height: i32,
}

fn inspect_target_part_metadata(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    layers: &[AvatarLayer],
) -> Result<PartMetadata, String> {
    let mut frame_count = 0usize;
    let mut canvas_width = 0i32;
    let mut canvas_height = 0i32;
    for layer in layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
    {
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        for entry_path in &family.entries {
            if !img_family_entry_is_visible(&family.base_entry, entry_path) {
                continue;
            }
            let entry_index = family
                .archive
                .find_entry_index(entry_path)
                .ok_or_else(|| format!("missing related IMG entry {entry_path}"))?;
            let frames = family.archive.read_img(entry_index)?.frames();
            let first = frames
                .first()
                .ok_or_else(|| format!("target IMG {entry_path} has no frames"))?;
            frame_count = frame_count.max(frames.len());
            canvas_width = canvas_width.max(first.canvas_width);
            canvas_height = canvas_height.max(first.canvas_height);
        }
    }
    if frame_count == 0 {
        return Err("target part has no IMG frames".to_string());
    }
    Ok(PartMetadata {
        frame_count,
        canvas_width,
        canvas_height,
    })
}

fn layers_by_slot(layers: &[AvatarLayer]) -> BTreeMap<String, Vec<AvatarLayer>> {
    let mut grouped = BTreeMap::<String, Vec<AvatarLayer>>::new();
    for layer in layers
        .iter()
        .filter(|layer| !layer.visual_img.trim().is_empty())
    {
        grouped
            .entry(layer.slot.to_ascii_lowercase())
            .or_default()
            .push(layer.clone());
    }
    grouped
}

fn effective_outfit_layers(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    snapshot: &OutfitSnapshot,
    layers: &[AvatarLayer],
) -> Result<Vec<AvatarLayer>, String> {
    if layers.iter().any(|layer| {
        !layer
            .visual_family_key
            .eq_ignore_ascii_case(&snapshot.visual_family_key)
    }) {
        return Err("outfit layers do not match the declared visual family".to_string());
    }
    let effective = visible_avatar_layers(layers);
    let mut resolved_effective = Vec::with_capacity(effective.len());
    for mut layer in effective {
        if layer.visual_img.trim().is_empty() {
            resolved_effective.push(layer);
            continue;
        }
        let family =
            match resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index) {
                Ok(family) => family,
                Err(error)
                    if (error.starts_with("missing IMG") || error.starts_with("missing NPK"))
                        && layer.item_id.eq_ignore_ascii_case("default_skin")
                        && layer.status.eq_ignore_ascii_case("default_skin") =>
                {
                    continue;
                }
                Err(error)
                    if error.starts_with("missing IMG") || error.starts_with("missing NPK") =>
                {
                    resolved_effective.push(layer);
                    continue;
                }
                Err(error) => return Err(error),
            };
        layer.visual_img = family.base_entry;
        layer.palette_index = family.palette_index;
        resolved_effective.push(layer);
    }
    sort_avatar_layers_for_draw(&mut resolved_effective);
    Ok(resolved_effective)
}

pub(crate) fn part_layers_equal(current: &[AvatarLayer], target: &[AvatarLayer]) -> bool {
    fn signatures(layers: &[AvatarLayer]) -> Vec<(String, String, i32, u16, u32, usize)> {
        let mut ordered = layers
            .iter()
            .filter(|layer| !layer.visual_img.trim().is_empty())
            .collect::<Vec<_>>();
        ordered.sort_by_key(|layer| layer.draw_order());
        ordered
            .into_iter()
            .map(|layer| {
                (
                    normalize_family(&layer.visual_img),
                    normalize_family(&layer.layer_tag),
                    layer.layer_order,
                    layer.equipment_type_enum,
                    layer.layer_sequence,
                    layer.palette_index,
                )
            })
            .collect()
    }
    signatures(current) == signatures(target)
}

fn collect_current_entries(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    current_layers: &[AvatarLayer],
) -> Result<Vec<String>, String> {
    let mut seen = HashSet::<String>::new();
    let mut entries = Vec::new();
    for layer in current_layers {
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        for entry in &family.entries {
            if !img_family_entry_authorized(layer, &family.base_entry, entry) {
                continue;
            }
            if seen.insert(entry.to_ascii_lowercase()) {
                entries.push(entry.clone());
            }
        }
    }
    entries.sort_by_key(|entry| entry.to_ascii_lowercase());
    Ok(entries)
}

fn add_unique_entry(
    builder: &mut NpkBuilder,
    written: &mut BTreeMap<String, Vec<u8>>,
    entry_path: &str,
    bytes: &[u8],
    entry_count: &mut usize,
) -> Result<(), String> {
    let key = normalized_entry_key(entry_path);
    if let Some(existing) = written.get(&key) {
        if existing != bytes {
            return Err(format!(
                "conflicting generated bytes for duplicate output IMG entry {entry_path}"
            ));
        }
        return Ok(());
    }
    builder.add_entry(entry_path, bytes)?;
    written.insert(key, bytes.to_vec());
    *entry_count += 1;
    Ok(())
}

fn validate_outfit_swap_output(
    output_path: &Path,
    expected: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let archive =
        NpkArchive::open_path(output_path).map_err(|_| "换装补丁写入后无法重新打开".to_string())?;
    if archive.entries().len() != expected.len() {
        return Err("换装补丁重读后的 IMG 数量不一致".to_string());
    }
    for (entry_path, expected_bytes) in expected {
        let index = archive
            .find_entry_index(entry_path)
            .ok_or_else(|| "换装补丁重读后缺少 IMG".to_string())?;
        let bytes = archive
            .entry_bytes(index)
            .map_err(|_| "换装补丁重读 IMG 失败".to_string())?;
        if bytes != expected_bytes {
            return Err("换装补丁重读后的 IMG 字节不一致".to_string());
        }
        let img = ImgArchive::open_owned_bytes(bytes.to_vec())
            .map_err(|_| "换装补丁 IMG 无法重新打开".to_string())?;
        let frames = img.frames();
        if frames.is_empty() {
            return Err("换装补丁 IMG 没有帧".to_string());
        }
        for frame in frames {
            img.decode_frame(frame.index)
                .map_err(|_| "换装补丁 IMG 帧无法解码".to_string())?;
        }
    }
    Ok(())
}

fn protect_source_npks(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    output_path: &Path,
    current_layers: &[AvatarLayer],
    target_layers: &[AvatarLayer],
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
    for layer in current_layers.iter().chain(target_layers) {
        if layer.visual_img.trim().is_empty() {
            continue;
        }
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        if comparable_path(&family.npk_path)? == output_key {
            return Err(format!(
                "output path must not overwrite source NPK {}",
                family.npk_path.display()
            ));
        }
    }
    Ok(())
}

struct ClearEntryFrames {
    entry_path: String,
    frames: Vec<PartFrame>,
}

fn load_clear_entry_frames(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    current_part_layers: &[AvatarLayer],
) -> Result<Vec<ClearEntryFrames>, String> {
    let mut seen = HashSet::<String>::new();
    let mut entries = Vec::new();
    for layer in current_part_layers {
        let family =
            resolve_img_family(resources, game_dir, &layer.visual_img, layer.palette_index)?;
        for entry_path in &family.entries {
            if !img_family_entry_authorized(layer, &family.base_entry, entry_path) {
                continue;
            }
            if !seen.insert(entry_path.to_ascii_lowercase()) {
                continue;
            }
            let entry_index = family
                .archive
                .find_entry_index(&entry_path)
                .ok_or_else(|| format!("missing current IMG entry {entry_path}"))?;
            let img = family.archive.read_img(entry_index)?;
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
                return Err(format!("current IMG {entry_path} has no frames"));
            }
            entries.push(ClearEntryFrames {
                entry_path: entry_path.clone(),
                frames,
            });
        }
    }
    entries.sort_by_key(|entry| entry.entry_path.to_ascii_lowercase());
    Ok(entries)
}

fn analyze_clear_part(
    resources: &mut OutfitResourceCache,
    game_dir: &Path,
    slot: &str,
    current_part_layers: &[AvatarLayer],
) -> Result<(OutfitSwapPartPlan, Vec<ClearEntryFrames>), String> {
    let carrier_layer = select_carrier_layer(current_part_layers)
        .ok_or_else(|| format!("current {slot} part has no carrier IMG"))?;
    let carrier_family = resolve_img_family(
        resources,
        game_dir,
        &carrier_layer.visual_img,
        carrier_layer.palette_index,
    )?;
    let transparent_entries = load_clear_entry_frames(resources, game_dir, current_part_layers)?;
    let carrier_frames = transparent_entries
        .iter()
        .find(|entry| {
            entry
                .entry_path
                .eq_ignore_ascii_case(&carrier_family.base_entry)
        })
        .map(|entry| entry.frames.as_slice())
        .ok_or_else(|| format!("current {slot} carrier frames are missing"))?;
    let first_frame = carrier_frames
        .first()
        .ok_or_else(|| format!("current {slot} carrier has no frames"))?;
    let output_frame_count = transparent_entries
        .iter()
        .map(|entry| entry.frames.len())
        .max()
        .unwrap_or(0);
    let transparent_imgs = transparent_entries
        .iter()
        .map(|entry| entry.entry_path.clone())
        .collect();
    Ok((
        OutfitSwapPartPlan {
            slot: slot.to_string(),
            action: "clear".to_string(),
            current_item_name: carrier_layer.item_name.clone(),
            target_item_name: String::new(),
            carrier_img: carrier_family.base_entry,
            target_layer_count: 0,
            output_frame_count,
            canvas_width: first_frame.canvas_width,
            canvas_height: first_frame.canvas_height,
            transparent_imgs,
            status: "ready".to_string(),
            has_dye: false,
            dyed_base_count: 0,
            dyed_mask_count: 0,
            dyed_color_count: 0,
            recipe_revision: String::new(),
        },
        transparent_entries,
    ))
}

fn comparable_path(path: &Path) -> Result<String, String> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| format!("resolve current directory: {err}"))?
            .join(path)
    };
    let absolute = if candidate.exists() {
        std::fs::canonicalize(&candidate)
            .map_err(|err| format!("resolve path {}: {err}", path.display()))?
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| format!("output path has no parent: {}", path.display()))?;
        let file_name = candidate
            .file_name()
            .ok_or_else(|| format!("output path has no file name: {}", path.display()))?;
        std::fs::canonicalize(parent)
            .map_err(|err| format!("resolve parent directory {}: {err}", parent.display()))?
            .join(file_name)
    };
    Ok(absolute
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase())
}

fn flatten_frame_at(layers: &[&PartLayerFrames], frame_index: usize) -> Result<PartFrame, String> {
    let available = layers
        .iter()
        .filter_map(|layer| layer.frames.get(frame_index))
        .collect::<Vec<_>>();
    let first = available
        .first()
        .ok_or_else(|| format!("frame {frame_index} has no source layer"))?;
    if first.canvas_width <= 0 || first.canvas_height <= 0 {
        return Err(format!(
            "frame {frame_index} has invalid canvas {}x{}",
            first.canvas_width, first.canvas_height
        ));
    }
    let canvas_width = first.canvas_width;
    let canvas_height = first.canvas_height;
    let canvas_bytes = usize::try_from(canvas_width)
        .ok()
        .and_then(|width| {
            usize::try_from(canvas_height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("frame {frame_index} canvas is too large"))?;
    if canvas_bytes > MAX_FLATTEN_CANVAS_BYTES {
        return Err(format!(
            "frame {frame_index} canvas exceeds the memory safety limit"
        ));
    }
    let mut canvas = vec![0_u8; canvas_bytes];

    for frame in available {
        validate_source_frame(frame, frame_index, canvas_width, canvas_height)?;
        if frame_is_transparent(frame) || frame_is_dnf_placeholder(frame) {
            continue;
        }
        for src_y in 0..frame.height {
            for src_x in 0..frame.width {
                let src_index = ((src_y * frame.width + src_x) * 4) as usize;
                let dst_x = frame.x + src_x as i32;
                let dst_y = frame.y + src_y as i32;
                let dst_index =
                    ((dst_y as usize * canvas_width as usize + dst_x as usize) * 4) as usize;
                alpha_blend_straight(
                    &mut canvas[dst_index..dst_index + 4],
                    &frame.rgba[src_index..src_index + 4],
                );
            }
        }
    }

    let Some((left, top, right, bottom)) = visible_bounds(&canvas, canvas_width, canvas_height)
    else {
        return Ok(PartFrame {
            width: 1,
            height: 1,
            x: canvas_width,
            y: 0,
            canvas_width,
            canvas_height,
            rgba: vec![0, 0, 0, 0],
        });
    };

    let width = (right - left) as u32;
    let height = (bottom - top) as u32;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in top..bottom {
        let row_start = ((y * canvas_width + left) * 4) as usize;
        let row_end = row_start + width as usize * 4;
        rgba.extend_from_slice(&canvas[row_start..row_end]);
    }

    Ok(PartFrame {
        width,
        height,
        x: left,
        y: top,
        canvas_width,
        canvas_height,
        rgba,
    })
}

fn validate_source_frame(
    frame: &PartFrame,
    frame_index: usize,
    canvas_width: i32,
    canvas_height: i32,
) -> Result<(), String> {
    if frame.canvas_width != canvas_width || frame.canvas_height != canvas_height {
        return Err(format!(
            "frame {frame_index} mixes canvas {}x{} with {}x{}",
            canvas_width, canvas_height, frame.canvas_width, frame.canvas_height
        ));
    }
    let expected = frame.width as usize * frame.height as usize * 4;
    if frame.rgba.len() != expected {
        return Err(format!(
            "frame {frame_index} has {} RGBA bytes, expected {expected}",
            frame.rgba.len()
        ));
    }
    if !frame_is_transparent(frame)
        && !frame_is_dnf_placeholder(frame)
        && (frame.x < 0
            || frame.y < 0
            || frame.x + frame.width as i32 > canvas_width
            || frame.y + frame.height as i32 > canvas_height)
    {
        return Err(format!(
            "frame {frame_index} bounds {},{} {}x{} exceed canvas {}x{}",
            frame.x, frame.y, frame.width, frame.height, canvas_width, canvas_height
        ));
    }
    Ok(())
}

fn frame_is_transparent(frame: &PartFrame) -> bool {
    frame.rgba.chunks_exact(4).all(|pixel| pixel[3] == 0)
}

fn frame_is_dnf_placeholder(frame: &PartFrame) -> bool {
    frame.width == 1 && frame.height == 1 && frame.x == frame.canvas_width && frame.y == 0
}

fn visible_bounds(
    canvas: &[u8],
    canvas_width: i32,
    canvas_height: i32,
) -> Option<(i32, i32, i32, i32)> {
    let mut left = canvas_width;
    let mut top = canvas_height;
    let mut right = 0;
    let mut bottom = 0;
    let mut found = false;
    for y in 0..canvas_height {
        for x in 0..canvas_width {
            let index = ((y * canvas_width + x) * 4) as usize;
            if canvas.get(index + 3).copied().unwrap_or(0) == 0 {
                continue;
            }
            found = true;
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    found.then_some((left, top, right, bottom))
}

fn alpha_blend_straight(dst: &mut [u8], src: &[u8]) {
    let src_alpha = u32::from(src[3]);
    let dst_alpha = u32::from(dst[3]);
    let inverse_src = 255 - src_alpha;
    let output_alpha_numerator = src_alpha * 255 + dst_alpha * inverse_src;
    if output_alpha_numerator == 0 {
        dst.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for channel in 0..3 {
        let numerator = u32::from(src[channel]) * src_alpha * 255
            + u32::from(dst[channel]) * dst_alpha * inverse_src;
        dst[channel] = ((numerator + output_alpha_numerator / 2) / output_alpha_numerator) as u8;
    }
    dst[3] = ((output_alpha_numerator + 127) / 255) as u8;
}

fn layer_tag_is_a(tag: &str) -> bool {
    tag.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
        .ends_with("_a")
}

fn normalize_family(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
