use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::dnflib_safe::{DecodedFrame, ImgArchive};
use crate::renderer::alpha_blend_rgba;

use super::app_state::{DyeCompareMode, DyePreviewScope};
use super::session::AvatarDyeSession;
use super::DyeTargetId;

const MAX_FRAME_PIXELS: usize = 16 * 1024 * 1024;
const MAX_CANVAS_PIXELS: usize = 32 * 1024 * 1024;
const MAX_ANIMATION_FRAMES: usize = 4_096;
const MAX_DECODED_ANIMATION_BYTES: usize = 512 * 1024 * 1024;
const MAX_SERIALIZED_ANIMATION_BYTES: usize = 128 * 1024 * 1024;

type Bounds = (i64, i64, i64, i64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyePreviewFrame {
    pub frame_index: usize,
    pub width: u32,
    pub height: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyePreviewAnimation {
    pub frame_count: usize,
    pub frames: Vec<DyePreviewFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyePreviewComparisonFrame {
    pub original: DyePreviewFrame,
    pub dyed: DyePreviewFrame,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyePreviewComparisonAnimation {
    pub frame_count: usize,
    pub frames: Vec<DyePreviewComparisonFrame>,
}

#[derive(Debug, Clone)]
struct PreviewLayer {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    decoded: Arc<DecodedFrame>,
}

#[derive(Debug)]
struct AnimationTargetFrames {
    is_mask: bool,
    frames: Vec<PreviewLayer>,
}

#[derive(Debug)]
struct ScopedPreviewLayer {
    target_index: usize,
    layer: PreviewLayer,
}

pub(super) fn render_session_frame(
    session: &AvatarDyeSession,
    frame_index: usize,
) -> Result<DyePreviewFrame, String> {
    let mut layers = Vec::with_capacity(session.targets.len());
    let mut retained_decoded_bytes = 0_usize;
    for target in &session.targets {
        if !target_is_visible_in_composite(target) {
            continue;
        }
        if target.frame_count() == 0 {
            return Err("染色预览目标没有帧".to_string());
        }
        if target.is_mask() && frame_index >= target.frame_count() {
            continue;
        }
        let actual_index = frame_index % target.frame_count();
        let mut archive = None;
        let layer = decode_target_frame(session, target, actual_index, false, &mut archive)?;
        retained_decoded_bytes = retained_decoded_bytes
            .checked_add(layer.decoded.rgba.len())
            .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
        if retained_decoded_bytes > MAX_DECODED_ANIMATION_BYTES {
            return Err("染色帧预览数据超过安全上限".to_string());
        }
        layers.push(layer);
    }
    let bounds = combined_bounds(layers.iter().filter_map(visible_bounds));
    compose_layers(&layers, bounds, frame_index)
}

pub(super) fn render_session_animation(
    session: &AvatarDyeSession,
) -> Result<DyePreviewAnimation, String> {
    let frame_count = session_composite_frame_count(session);
    if frame_count == 0 {
        return Err("染色预览没有动画帧".to_string());
    }
    if frame_count > MAX_ANIMATION_FRAMES {
        return Err("染色动画帧数超过安全上限".to_string());
    }

    let mut decoded_bytes = 0_usize;
    let mut target_frames = Vec::<AnimationTargetFrames>::with_capacity(session.targets.len());
    for target in &session.targets {
        if !target_is_visible_in_composite(target) {
            continue;
        }
        if target.frame_count() == 0 {
            return Err("染色预览目标没有帧".to_string());
        }
        let mut archive = None;
        let mut frames = Vec::with_capacity(target.frame_count());
        for frame_index in 0..target.frame_count() {
            let layer = decode_target_frame(session, target, frame_index, false, &mut archive)?;
            decoded_bytes = decoded_bytes
                .checked_add(layer.decoded.rgba.len())
                .ok_or_else(|| "染色动画解码大小溢出".to_string())?;
            if decoded_bytes > MAX_DECODED_ANIMATION_BYTES {
                return Err("染色动画解码数据超过安全上限".to_string());
            }
            frames.push(layer);
        }
        target_frames.push(AnimationTargetFrames {
            is_mask: target.is_mask(),
            frames,
        });
    }

    let stable_bounds = combined_bounds(
        target_frames
            .iter()
            .flat_map(|target| target.frames.iter().filter_map(visible_bounds)),
    );
    let (canvas_width, canvas_height) = canvas_dimensions(stable_bounds)?;
    validate_animation_output_budget(canvas_width, canvas_height, frame_count, decoded_bytes)?;
    let mut frames = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        frames.push(compose_animation_frame(
            &target_frames,
            stable_bounds,
            frame_index,
        )?);
    }
    Ok(DyePreviewAnimation {
        frame_count,
        frames,
    })
}

pub(super) fn session_composite_frame_count(session: &AvatarDyeSession) -> usize {
    session
        .targets
        .iter()
        .filter(|target| target_is_visible_in_composite(target))
        .map(|target| target.frame_count())
        .max()
        .unwrap_or(0)
}

pub(super) fn render_session_frame_with_options(
    session: &AvatarDyeSession,
    frame_index: usize,
    compare_mode: DyeCompareMode,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    highlight: bool,
    hide_selected_mask: bool,
) -> Result<DyePreviewFrame, String> {
    render_session_frame_with_visibility(
        session,
        frame_index,
        compare_mode,
        preview_scope,
        target_id,
        highlight,
        hide_selected_mask,
        &[],
    )
}

pub(super) fn render_session_frame_with_visibility(
    session: &AvatarDyeSession,
    frame_index: usize,
    compare_mode: DyeCompareMode,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    highlight: bool,
    hide_selected_mask: bool,
    hidden_target_ids: &[DyeTargetId],
) -> Result<DyePreviewFrame, String> {
    let indices = scoped_target_indices(
        session,
        preview_scope,
        target_id,
        hide_selected_mask,
        hidden_target_ids,
    )?;
    let need_original = compare_mode != DyeCompareMode::Dyed || highlight;
    let need_dyed = compare_mode != DyeCompareMode::Original || highlight;
    let original_layers = if need_original {
        decode_scoped_frame(session, &indices, frame_index, true)?
    } else {
        Vec::new()
    };
    let dyed_layers = if need_dyed {
        decode_scoped_frame(session, &indices, frame_index, false)?
    } else {
        Vec::new()
    };
    let retained_bytes = original_layers
        .iter()
        .chain(&dyed_layers)
        .try_fold(0_usize, |total, layer| {
            total.checked_add(layer.layer.decoded.rgba.len())
        })
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    if retained_bytes > MAX_DECODED_ANIMATION_BYTES {
        return Err("染色帧预览数据超过安全上限".to_string());
    }
    let bounds = combined_bounds(
        original_layers
            .iter()
            .chain(&dyed_layers)
            .filter_map(|layer| visible_bounds(&layer.layer)),
    );
    let (width, height) = canvas_dimensions(bounds)?;
    validate_frame_output_budget(width, height, retained_bytes, compare_mode, highlight)?;
    let original = if need_original {
        Some(compose_layers(
            original_layers.iter().map(|layer| &layer.layer),
            bounds,
            frame_index,
        )?)
    } else {
        None
    };
    let dyed = if need_dyed {
        Some(compose_layers(
            dyed_layers.iter().map(|layer| &layer.layer),
            bounds,
            frame_index,
        )?)
    } else {
        None
    };
    let highlight_original = if highlight {
        Some(compose_selected_original_frame(
            &original_layers,
            &dyed_layers,
            target_id,
            session,
            bounds,
            frame_index,
        )?)
    } else {
        None
    };
    merge_comparison_with_highlight(
        original,
        dyed.clone(),
        compare_mode,
        highlight_original.zip(dyed),
    )
}

pub(super) fn render_session_comparison_frame_with_visibility(
    session: &AvatarDyeSession,
    frame_index: usize,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    hide_selected_mask: bool,
    hidden_target_ids: &[DyeTargetId],
) -> Result<DyePreviewComparisonFrame, String> {
    let indices = scoped_target_indices(
        session,
        preview_scope,
        target_id,
        hide_selected_mask,
        hidden_target_ids,
    )?;
    let (original_layers, dyed_layers) =
        decode_scoped_comparison_frame(session, &indices, frame_index)?;
    let retained_bytes = original_layers
        .iter()
        .chain(&dyed_layers)
        .try_fold(0_usize, |total, layer| {
            total.checked_add(layer.layer.decoded.rgba.len())
        })
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    if retained_bytes > MAX_DECODED_ANIMATION_BYTES {
        return Err("染色帧预览数据超过安全上限".to_string());
    }
    let bounds = combined_bounds(
        original_layers
            .iter()
            .chain(&dyed_layers)
            .filter_map(|layer| visible_bounds(&layer.layer)),
    );
    let (width, height) = canvas_dimensions(bounds)?;
    validate_comparison_frame_output_budget(width, height, retained_bytes)?;
    let original = compose_layers(
        original_layers.iter().map(|layer| &layer.layer),
        bounds,
        frame_index,
    )?;
    let dyed = compose_layers(
        dyed_layers.iter().map(|layer| &layer.layer),
        bounds,
        frame_index,
    )?;
    ensure_same_canvas(&original, &dyed)?;
    Ok(DyePreviewComparisonFrame { original, dyed })
}

pub(super) fn render_session_animation_with_options(
    session: &AvatarDyeSession,
    compare_mode: DyeCompareMode,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    highlight: bool,
    hide_selected_mask: bool,
) -> Result<DyePreviewAnimation, String> {
    render_session_animation_with_visibility(
        session,
        compare_mode,
        preview_scope,
        target_id,
        highlight,
        hide_selected_mask,
        &[],
    )
}

pub(super) fn render_session_animation_with_visibility(
    session: &AvatarDyeSession,
    compare_mode: DyeCompareMode,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    highlight: bool,
    hide_selected_mask: bool,
    hidden_target_ids: &[DyeTargetId],
) -> Result<DyePreviewAnimation, String> {
    let indices = scoped_target_indices(
        session,
        preview_scope,
        target_id,
        hide_selected_mask,
        hidden_target_ids,
    )?;
    let selected_global = session
        .targets
        .iter()
        .position(|target| target.id() == target_id)
        .ok_or_else(|| "染色高亮目标不存在或已经过期".to_string())?;
    let selected_position = indices.iter().position(|index| *index == selected_global);
    let frame_count = indices
        .iter()
        .map(|index| session.targets[*index].frame_count())
        .max()
        .unwrap_or(1);
    if frame_count > MAX_ANIMATION_FRAMES {
        return Err("染色动画帧数超过安全上限".to_string());
    }

    let need_original = compare_mode != DyeCompareMode::Dyed || highlight;
    let need_dyed = compare_mode != DyeCompareMode::Original || highlight;
    let mut retained_decoded_bytes = 0_usize;
    let original = if need_original {
        decode_scoped_animation(session, &indices, true, &mut retained_decoded_bytes)?
    } else {
        Vec::new()
    };
    let dyed = if need_dyed {
        decode_scoped_animation(session, &indices, false, &mut retained_decoded_bytes)?
    } else {
        Vec::new()
    };
    let stable_bounds = combined_bounds(
        original
            .iter()
            .chain(&dyed)
            .flat_map(|target| target.frames.iter().filter_map(visible_bounds)),
    );
    let (width, height) = canvas_dimensions(stable_bounds)?;
    validate_animation_output_budget_with_options(
        width,
        height,
        frame_count,
        retained_decoded_bytes,
        compare_mode,
        highlight,
    )?;

    let mut frames = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let original_frame = if need_original {
            Some(compose_animation_frame(
                &original,
                stable_bounds,
                frame_index,
            )?)
        } else {
            None
        };
        let dyed_frame = if need_dyed {
            Some(compose_animation_frame(&dyed, stable_bounds, frame_index)?)
        } else {
            None
        };
        let highlight_original = if highlight {
            Some(match selected_position {
                Some(position) => compose_animation_frame_with_selected_original(
                    &dyed,
                    &original,
                    position,
                    stable_bounds,
                    frame_index,
                )?,
                None => dyed_frame
                    .clone()
                    .ok_or_else(|| "染色高亮预览数据缺失".to_string())?,
            })
        } else {
            None
        };
        frames.push(merge_comparison_with_highlight(
            original_frame,
            dyed_frame.clone(),
            compare_mode,
            highlight_original.zip(dyed_frame),
        )?);
    }
    Ok(DyePreviewAnimation {
        frame_count,
        frames,
    })
}

pub(super) fn render_session_comparison_animation_with_visibility(
    session: &AvatarDyeSession,
    preview_scope: DyePreviewScope,
    target_id: &DyeTargetId,
    hide_selected_mask: bool,
    hidden_target_ids: &[DyeTargetId],
) -> Result<DyePreviewComparisonAnimation, String> {
    let indices = scoped_target_indices(
        session,
        preview_scope,
        target_id,
        hide_selected_mask,
        hidden_target_ids,
    )?;
    let frame_count = indices
        .iter()
        .map(|index| session.targets[*index].frame_count())
        .max()
        .unwrap_or(1);
    if frame_count > MAX_ANIMATION_FRAMES {
        return Err("染色动画帧数超过安全上限".to_string());
    }

    let mut retained_decoded_bytes = 0_usize;
    let (original, dyed) =
        decode_scoped_comparison_animation(session, &indices, &mut retained_decoded_bytes)?;
    let stable_bounds = combined_bounds(
        original
            .iter()
            .chain(&dyed)
            .flat_map(|target| target.frames.iter().filter_map(visible_bounds)),
    );
    let (width, height) = canvas_dimensions(stable_bounds)?;
    validate_comparison_animation_output_budget(
        width,
        height,
        frame_count,
        retained_decoded_bytes,
    )?;

    let mut frames = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let original = compose_animation_frame(&original, stable_bounds, frame_index)?;
        let dyed = compose_animation_frame(&dyed, stable_bounds, frame_index)?;
        ensure_same_canvas(&original, &dyed)?;
        frames.push(DyePreviewComparisonFrame { original, dyed });
    }
    Ok(DyePreviewComparisonAnimation {
        frame_count,
        frames,
    })
}

fn scoped_target_indices(
    session: &AvatarDyeSession,
    scope: DyePreviewScope,
    target_id: &DyeTargetId,
    hide_selected_mask: bool,
    hidden_target_ids: &[DyeTargetId],
) -> Result<Vec<usize>, String> {
    let selected = session
        .targets
        .iter()
        .position(|target| target.id() == target_id)
        .ok_or_else(|| "染色预览目标不存在或已经过期".to_string())?;
    let mut indices = match scope {
        DyePreviewScope::Outfit => (0..session.targets.len()).collect(),
        DyePreviewScope::Part => {
            let slot = session.targets[selected].slot();
            session
                .targets
                .iter()
                .enumerate()
                .filter(|(_, target)| target.slot().eq_ignore_ascii_case(slot))
                .map(|(index, _)| index)
                .collect()
        }
        DyePreviewScope::Target => vec![selected],
    };
    if hide_selected_mask && scope != DyePreviewScope::Target && session.targets[selected].is_mask()
    {
        indices.retain(|index| *index != selected);
    }
    indices.retain(|index| {
        (scope == DyePreviewScope::Target && *index == selected)
            || !hidden_target_ids.contains(session.targets[*index].id())
    });
    if scope != DyePreviewScope::Target {
        indices.retain(|index| target_is_visible_in_composite(&session.targets[*index]));
    }
    Ok(indices)
}

fn target_is_visible_in_composite(target: &super::session::TargetState) -> bool {
    !target.is_mask()
}

fn decode_target_frame(
    session: &AvatarDyeSession,
    target: &super::session::TargetState,
    frame_index: usize,
    original: bool,
    archive: &mut Option<ImgArchive>,
) -> Result<PreviewLayer, String> {
    let img = session.preview_img_bytes(target, original)?;
    let decoded = session.preview_decoded_frame(target, &img, frame_index, || {
        if archive.is_none() {
            *archive = Some(ImgArchive::open_owned_bytes(img.bytes.to_vec())?);
        }
        archive
            .as_ref()
            .ok_or_else(|| "染色预览 IMG 无法打开".to_string())?
            .decode_frame_with_palette(frame_index, target.active_palette())
    })?;
    decoded_layer(decoded)
}

fn decode_scoped_frame(
    session: &AvatarDyeSession,
    indices: &[usize],
    frame_index: usize,
    original: bool,
) -> Result<Vec<ScopedPreviewLayer>, String> {
    let mut layers = Vec::with_capacity(indices.len());
    for index in indices {
        let target = session
            .targets
            .get(*index)
            .ok_or_else(|| "染色预览目标索引无效".to_string())?;
        if target.frame_count() == 0 {
            return Err("染色预览目标没有帧".to_string());
        }
        if target.is_mask() && frame_index >= target.frame_count() {
            continue;
        }
        let actual_index = frame_index % target.frame_count();
        let mut archive = None;
        layers.push(ScopedPreviewLayer {
            target_index: *index,
            layer: decode_target_frame(session, target, actual_index, original, &mut archive)?,
        });
    }
    Ok(layers)
}

fn decode_scoped_comparison_frame(
    session: &AvatarDyeSession,
    indices: &[usize],
    frame_index: usize,
) -> Result<(Vec<ScopedPreviewLayer>, Vec<ScopedPreviewLayer>), String> {
    let mut original = Vec::with_capacity(indices.len());
    let mut dyed = Vec::with_capacity(indices.len());
    for index in indices {
        let target = session
            .targets
            .get(*index)
            .ok_or_else(|| "染色预览目标索引无效".to_string())?;
        if target.frame_count() == 0 {
            return Err("染色预览目标没有帧".to_string());
        }
        if target.is_mask() && frame_index >= target.frame_count() {
            continue;
        }
        let actual_index = frame_index % target.frame_count();
        let mut original_archive = None;
        let mut dyed_archive = None;
        original.push(ScopedPreviewLayer {
            target_index: *index,
            layer: decode_target_frame(session, target, actual_index, true, &mut original_archive)?,
        });
        dyed.push(ScopedPreviewLayer {
            target_index: *index,
            layer: decode_target_frame(session, target, actual_index, false, &mut dyed_archive)?,
        });
    }
    Ok((original, dyed))
}

fn compose_selected_original_frame(
    original: &[ScopedPreviewLayer],
    dyed: &[ScopedPreviewLayer],
    target_id: &DyeTargetId,
    session: &AvatarDyeSession,
    bounds: Option<Bounds>,
    frame_index: usize,
) -> Result<DyePreviewFrame, String> {
    let selected = session
        .targets
        .iter()
        .position(|target| target.id() == target_id)
        .ok_or_else(|| "染色高亮目标不存在或已经过期".to_string())?;
    let layers = dyed.iter().map(|dyed_layer| {
        if dyed_layer.target_index == selected {
            original
                .iter()
                .find(|layer| layer.target_index == selected)
                .map(|layer| &layer.layer)
                .unwrap_or(&dyed_layer.layer)
        } else {
            &dyed_layer.layer
        }
    });
    compose_layers(layers, bounds, frame_index)
}

fn decode_scoped_animation(
    session: &AvatarDyeSession,
    indices: &[usize],
    original: bool,
    retained_decoded_bytes: &mut usize,
) -> Result<Vec<AnimationTargetFrames>, String> {
    let mut targets = Vec::with_capacity(indices.len());
    for index in indices {
        let target = session
            .targets
            .get(*index)
            .ok_or_else(|| "染色动画目标索引无效".to_string())?;
        if target.frame_count() == 0 {
            return Err("染色动画目标没有帧".to_string());
        }
        let mut archive = None;
        let mut frames = Vec::with_capacity(target.frame_count());
        for frame_index in 0..target.frame_count() {
            let layer = decode_target_frame(session, target, frame_index, original, &mut archive)?;
            *retained_decoded_bytes = retained_decoded_bytes
                .checked_add(layer.decoded.rgba.len())
                .ok_or_else(|| "染色动画解码大小溢出".to_string())?;
            if *retained_decoded_bytes > MAX_DECODED_ANIMATION_BYTES {
                return Err("染色动画解码数据超过安全上限".to_string());
            }
            frames.push(layer);
        }
        targets.push(AnimationTargetFrames {
            is_mask: target.is_mask(),
            frames,
        });
    }
    Ok(targets)
}

fn decode_scoped_comparison_animation(
    session: &AvatarDyeSession,
    indices: &[usize],
    retained_decoded_bytes: &mut usize,
) -> Result<(Vec<AnimationTargetFrames>, Vec<AnimationTargetFrames>), String> {
    let mut original_targets = Vec::with_capacity(indices.len());
    let mut dyed_targets = Vec::with_capacity(indices.len());
    for index in indices {
        let target = session
            .targets
            .get(*index)
            .ok_or_else(|| "染色动画目标索引无效".to_string())?;
        if target.frame_count() == 0 {
            return Err("染色动画目标没有帧".to_string());
        }
        let mut original_archive = None;
        let mut dyed_archive = None;
        let mut original_frames = Vec::with_capacity(target.frame_count());
        let mut dyed_frames = Vec::with_capacity(target.frame_count());
        for frame_index in 0..target.frame_count() {
            let original =
                decode_target_frame(session, target, frame_index, true, &mut original_archive)?;
            add_retained_decoded_bytes(retained_decoded_bytes, original.decoded.rgba.len())?;
            original_frames.push(original);

            let dyed = decode_target_frame(session, target, frame_index, false, &mut dyed_archive)?;
            add_retained_decoded_bytes(retained_decoded_bytes, dyed.decoded.rgba.len())?;
            dyed_frames.push(dyed);
        }
        original_targets.push(AnimationTargetFrames {
            is_mask: target.is_mask(),
            frames: original_frames,
        });
        dyed_targets.push(AnimationTargetFrames {
            is_mask: target.is_mask(),
            frames: dyed_frames,
        });
    }
    Ok((original_targets, dyed_targets))
}

fn add_retained_decoded_bytes(total: &mut usize, bytes: usize) -> Result<(), String> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| "染色动画解码大小溢出".to_string())?;
    if *total > MAX_DECODED_ANIMATION_BYTES {
        return Err("染色动画解码数据超过安全上限".to_string());
    }
    Ok(())
}

fn compose_animation_frame(
    targets: &[AnimationTargetFrames],
    bounds: Option<Bounds>,
    frame_index: usize,
) -> Result<DyePreviewFrame, String> {
    let layers = targets
        .iter()
        .filter(|target| {
            !target.frames.is_empty() && (!target.is_mask || frame_index < target.frames.len())
        })
        .map(|target| &target.frames[frame_index % target.frames.len()]);
    compose_layers(layers, bounds, frame_index)
}

fn compose_animation_frame_with_selected_original(
    dyed: &[AnimationTargetFrames],
    original: &[AnimationTargetFrames],
    selected_position: usize,
    bounds: Option<Bounds>,
    frame_index: usize,
) -> Result<DyePreviewFrame, String> {
    let layers = dyed.iter().enumerate().filter_map(|(position, target)| {
        let target = if position == selected_position {
            original.get(position).unwrap_or(target)
        } else {
            target
        };
        (!target.frames.is_empty() && (!target.is_mask || frame_index < target.frames.len()))
            .then(|| &target.frames[frame_index % target.frames.len()])
    });
    compose_layers(layers, bounds, frame_index)
}

fn merge_comparison_with_highlight(
    original: Option<DyePreviewFrame>,
    dyed: Option<DyePreviewFrame>,
    compare_mode: DyeCompareMode,
    highlight_pair: Option<(DyePreviewFrame, DyePreviewFrame)>,
) -> Result<DyePreviewFrame, String> {
    let mut output = match compare_mode {
        DyeCompareMode::Original => original
            .clone()
            .ok_or_else(|| "原色预览数据缺失".to_string())?,
        DyeCompareMode::Dyed => dyed.clone().ok_or_else(|| "染色预览数据缺失".to_string())?,
        DyeCompareMode::Split => {
            let original = original
                .as_ref()
                .ok_or_else(|| "分屏原色预览数据缺失".to_string())?;
            let dyed = dyed
                .as_ref()
                .ok_or_else(|| "分屏染色预览数据缺失".to_string())?;
            ensure_same_canvas(original, dyed)?;
            let mut split = dyed.clone();
            let split_x = usize::try_from(split.width).unwrap_or_default() / 2;
            let width = usize::try_from(split.width).unwrap_or_default();
            let height = usize::try_from(split.height).unwrap_or_default();
            for y in 0..height {
                let start = y * width * 4;
                let end = start + split_x * 4;
                split.rgba[start..end].copy_from_slice(&original.rgba[start..end]);
            }
            split
        }
    };
    if let Some((highlight_original, highlight_dyed)) = highlight_pair {
        let original = &highlight_original;
        let dyed = &highlight_dyed;
        ensure_same_canvas(original, dyed)?;
        for pixel in 0..output.rgba.len() / 4 {
            let index = pixel * 4;
            if original.rgba[index..index + 4] == dyed.rgba[index..index + 4] {
                continue;
            }
            let alpha = output.rgba[index + 3];
            if alpha == 0 {
                continue;
            }
            output.rgba[index] = blend_highlight(output.rgba[index], 255);
            output.rgba[index + 1] = blend_highlight(output.rgba[index + 1], 205);
            output.rgba[index + 2] = blend_highlight(output.rgba[index + 2], 32);
        }
    }
    Ok(output)
}

fn ensure_same_canvas(left: &DyePreviewFrame, right: &DyePreviewFrame) -> Result<(), String> {
    if left.width != right.width
        || left.height != right.height
        || left.origin_x != right.origin_x
        || left.origin_y != right.origin_y
        || left.rgba.len() != right.rgba.len()
    {
        return Err("染色对比预览画布不一致".to_string());
    }
    Ok(())
}

fn blend_highlight(value: u8, marker: u8) -> u8 {
    ((u16::from(value) * 2 + u16::from(marker) * 3) / 5) as u8
}

fn decoded_layer(decoded: Arc<DecodedFrame>) -> Result<PreviewLayer, String> {
    validate_frame_dimensions(decoded.width, decoded.height)?;
    let width = u32::try_from(decoded.width).map_err(|_| "预览帧宽度无效".to_string())?;
    let height = u32::try_from(decoded.height).map_err(|_| "预览帧高度无效".to_string())?;
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "预览帧像素长度溢出".to_string())?;
    if decoded.rgba.len() != expected {
        return Err("预览帧 RGBA 长度无效".to_string());
    }
    Ok(PreviewLayer {
        width,
        height,
        x: decoded.x,
        y: decoded.y,
        decoded,
    })
}

fn validate_frame_dimensions(width: i32, height: i32) -> Result<(), String> {
    let width = usize::try_from(width).map_err(|_| "预览帧宽度无效".to_string())?;
    let height = usize::try_from(height).map_err(|_| "预览帧高度无效".to_string())?;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| "预览帧像素数量溢出".to_string())?;
    if width == 0 || height == 0 || pixels > MAX_FRAME_PIXELS {
        return Err("预览帧尺寸超过安全上限".to_string());
    }
    Ok(())
}

fn visible_bounds(layer: &PreviewLayer) -> Option<Bounds> {
    let mut min_x = layer.width;
    let mut min_y = layer.height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    let mut found = false;
    for y in 0..layer.height {
        for x in 0..layer.width {
            let index = ((y * layer.width + x) * 4) as usize;
            if layer.decoded.rgba.get(index + 3).copied().unwrap_or(0) == 0 {
                continue;
            }
            found = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + 1);
            max_y = max_y.max(y + 1);
        }
    }
    found.then_some((
        i64::from(layer.x) + i64::from(min_x),
        i64::from(layer.y) + i64::from(min_y),
        i64::from(layer.x) + i64::from(max_x),
        i64::from(layer.y) + i64::from(max_y),
    ))
}

fn combined_bounds<I>(bounds: I) -> Option<Bounds>
where
    I: IntoIterator<Item = Bounds>,
{
    let mut bounds = bounds.into_iter();
    let first = bounds.next()?;
    Some(bounds.fold(first, |current, next| {
        (
            current.0.min(next.0),
            current.1.min(next.1),
            current.2.max(next.2),
            current.3.max(next.3),
        )
    }))
}

pub(super) fn validate_animation_output_budget(
    width: usize,
    height: usize,
    frame_count: usize,
    retained_decoded_bytes: usize,
) -> Result<(), String> {
    validate_animation_output_budget_with_counts(
        width,
        height,
        frame_count,
        retained_decoded_bytes,
        1,
        0,
    )
}

pub(super) fn validate_animation_output_budget_with_options(
    width: usize,
    height: usize,
    frame_count: usize,
    retained_decoded_bytes: usize,
    compare_mode: DyeCompareMode,
    highlight: bool,
) -> Result<(), String> {
    validate_animation_output_budget_with_counts(
        width,
        height,
        frame_count,
        retained_decoded_bytes,
        1,
        comparison_canvas_count(compare_mode, highlight).saturating_sub(1),
    )
}

pub(super) fn validate_comparison_animation_output_budget(
    width: usize,
    height: usize,
    frame_count: usize,
    retained_decoded_bytes: usize,
) -> Result<(), String> {
    validate_animation_output_budget_with_counts(
        width,
        height,
        frame_count,
        retained_decoded_bytes,
        2,
        0,
    )
}

fn validate_animation_output_budget_with_counts(
    width: usize,
    height: usize,
    frame_count: usize,
    retained_decoded_bytes: usize,
    output_canvases_per_frame: usize,
    additional_transient_canvases: usize,
) -> Result<(), String> {
    let canvas_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "染色动画输出数据大小溢出".to_string())?;
    let output_count = frame_count
        .checked_mul(output_canvases_per_frame)
        .ok_or_else(|| "染色动画输出数据大小溢出".to_string())?;
    let output_bytes = canvas_bytes
        .checked_mul(output_count)
        .ok_or_else(|| "染色动画输出数据大小溢出".to_string())?;
    let transient_bytes = canvas_bytes
        .checked_mul(additional_transient_canvases)
        .ok_or_else(|| "染色动画输出数据大小溢出".to_string())?;
    let retained_bytes = retained_decoded_bytes
        .checked_add(output_bytes)
        .and_then(|bytes| bytes.checked_add(transient_bytes))
        .ok_or_else(|| "染色动画输出数据大小溢出".to_string())?;
    if retained_bytes > MAX_DECODED_ANIMATION_BYTES {
        return Err("染色动画输出数据超过安全上限".to_string());
    }
    validate_serialized_output_budget(canvas_bytes, output_count, "染色动画")?;
    Ok(())
}

pub(super) fn validate_frame_output_budget(
    width: usize,
    height: usize,
    retained_decoded_bytes: usize,
    compare_mode: DyeCompareMode,
    highlight: bool,
) -> Result<(), String> {
    let canvas_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    let composed_bytes = canvas_bytes
        .checked_mul(comparison_canvas_count(compare_mode, highlight))
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    let retained_bytes = retained_decoded_bytes
        .checked_add(composed_bytes)
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    if retained_bytes > MAX_DECODED_ANIMATION_BYTES {
        return Err("染色帧预览数据超过安全上限".to_string());
    }
    validate_serialized_output_budget(canvas_bytes, 1, "染色帧预览")?;
    Ok(())
}

pub(super) fn validate_comparison_frame_output_budget(
    width: usize,
    height: usize,
    retained_decoded_bytes: usize,
) -> Result<(), String> {
    let canvas_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    let paired_canvas_bytes = canvas_bytes
        .checked_mul(2)
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    let retained_bytes = retained_decoded_bytes
        .checked_add(paired_canvas_bytes)
        .ok_or_else(|| "染色帧预览数据大小溢出".to_string())?;
    if retained_bytes > MAX_DECODED_ANIMATION_BYTES {
        return Err("染色帧预览数据超过安全上限".to_string());
    }
    validate_serialized_output_budget(canvas_bytes, 2, "染色帧预览")?;
    Ok(())
}

fn comparison_canvas_count(compare_mode: DyeCompareMode, highlight: bool) -> usize {
    if highlight {
        5
    } else {
        match compare_mode {
            DyeCompareMode::Original => 2,
            DyeCompareMode::Dyed => 3,
            DyeCompareMode::Split => 4,
        }
    }
}

fn validate_serialized_output_budget(
    rgba_bytes_per_output: usize,
    output_count: usize,
    label: &str,
) -> Result<(), String> {
    let encoded_bytes_per_output = rgba_bytes_per_output
        .checked_add(2)
        .and_then(|bytes| (bytes / 3).checked_mul(4))
        .ok_or_else(|| format!("{label}跨进程数据大小溢出"))?;
    let encoded_bytes = encoded_bytes_per_output
        .checked_mul(output_count)
        .ok_or_else(|| format!("{label}跨进程数据大小溢出"))?;
    if encoded_bytes > MAX_SERIALIZED_ANIMATION_BYTES {
        return Err(format!("{label}跨进程数据超过安全上限"));
    }
    Ok(())
}

fn canvas_dimensions(bounds: Option<Bounds>) -> Result<(usize, usize), String> {
    let Some((min_x, min_y, max_x, max_y)) = bounds else {
        return Ok((1, 1));
    };
    let width = usize::try_from(max_x - min_x).map_err(|_| "预览画布宽度无效".to_string())?;
    let height = usize::try_from(max_y - min_y).map_err(|_| "预览画布高度无效".to_string())?;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| "预览画布像素数量溢出".to_string())?;
    if width == 0 || height == 0 || pixels > MAX_CANVAS_PIXELS {
        return Err("预览画布尺寸超过安全上限".to_string());
    }
    Ok((width, height))
}

fn compose_layers<'a, I>(
    layers: I,
    bounds: Option<Bounds>,
    frame_index: usize,
) -> Result<DyePreviewFrame, String>
where
    I: IntoIterator<Item = &'a PreviewLayer>,
{
    let Some((min_x, min_y, _, _)) = bounds else {
        return Ok(DyePreviewFrame {
            frame_index,
            width: 1,
            height: 1,
            origin_x: 0,
            origin_y: 0,
            rgba: vec![0, 0, 0, 0],
        });
    };
    let (width, height) = canvas_dimensions(bounds)?;
    let pixels = width * height;
    let mut canvas = vec![
        0_u8;
        pixels
            .checked_mul(4)
            .ok_or_else(|| "预览画布长度溢出".to_string())?
    ];
    for layer in layers {
        for source_y in 0..layer.height {
            for source_x in 0..layer.width {
                let source_index = ((source_y * layer.width + source_x) * 4) as usize;
                if layer.decoded.rgba[source_index + 3] == 0 {
                    continue;
                }
                let target_x = i64::from(layer.x) + i64::from(source_x) - min_x;
                let target_y = i64::from(layer.y) + i64::from(source_y) - min_y;
                if target_x < 0
                    || target_y < 0
                    || target_x >= width as i64
                    || target_y >= height as i64
                {
                    continue;
                }
                let target_index = (target_y as usize * width + target_x as usize) * 4;
                alpha_blend_rgba(
                    &mut canvas[target_index..target_index + 4],
                    &layer.decoded.rgba[source_index..source_index + 4],
                );
            }
        }
    }
    Ok(DyePreviewFrame {
        frame_index,
        width: u32::try_from(width).map_err(|_| "预览画布宽度无效".to_string())?,
        height: u32::try_from(height).map_err(|_| "预览画布高度无效".to_string())?,
        origin_x: i32::try_from(min_x).map_err(|_| "预览画布横坐标无效".to_string())?,
        origin_y: i32::try_from(min_y).map_err(|_| "预览画布纵坐标无效".to_string())?,
        rgba: canvas,
    })
}
