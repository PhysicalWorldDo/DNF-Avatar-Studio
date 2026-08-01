use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;

use crate::dnflib_safe::NpkArchive;
use crate::game_resolver::{resolve_game_npk_path, resolve_img_resource, ImgResourceMetadata};
use crate::models::{PreviewAnimation, PreviewImage, PreviewLayerRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerFrame {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub rgba: Vec<u8>,
}

type Bounds = (i32, i32, i32, i32);

pub fn alpha_blend_rgba(dst: &mut [u8], src: &[u8]) {
    for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let src_alpha = u32::from(src_px[3]);
        let dst_alpha = u32::from(dst_px[3]);
        let inverse_src = 255 - src_alpha;
        let output_alpha_numerator = src_alpha * 255 + dst_alpha * inverse_src;
        if output_alpha_numerator == 0 {
            dst_px.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let rounding_bias = if dst_alpha == 255 {
            0
        } else {
            output_alpha_numerator / 2
        };
        for channel in 0..3 {
            let numerator = u32::from(src_px[channel]) * src_alpha * 255
                + u32::from(dst_px[channel]) * dst_alpha * inverse_src;
            dst_px[channel] = ((numerator + rounding_bias) / output_alpha_numerator) as u8;
        }
        dst_px[3] = ((output_alpha_numerator + 127) / 255) as u8;
    }
}

pub fn compose_layer_frames(frames: &[LayerFrame], missing_imgs: Vec<String>) -> PreviewImage {
    let bounds = combined_bounds(frames.iter().filter_map(visible_frame_bounds));
    compose_layer_frames_with_bounds(frames, missing_imgs, bounds)
}

fn compose_layer_frames_with_bounds(
    frames: &[LayerFrame],
    missing_imgs: Vec<String>,
    bounds: Option<Bounds>,
) -> PreviewImage {
    let Some((min_x, min_y, max_x, max_y)) = bounds else {
        return PreviewImage {
            width: 1,
            height: 1,
            rgba_base64: base64::engine::general_purpose::STANDARD.encode([0_u8, 0, 0, 0]),
            missing_imgs,
        };
    };
    let width = (max_x - min_x).max(1) as u32;
    let height = (max_y - min_y).max(1) as u32;
    let mut canvas = vec![0_u8; width as usize * height as usize * 4];

    for frame in frames {
        for src_y in 0..frame.height {
            for src_x in 0..frame.width {
                let dst_x = frame.x + src_x as i32 - min_x;
                let dst_y = frame.y + src_y as i32 - min_y;
                if dst_x < 0 || dst_y < 0 || dst_x >= width as i32 || dst_y >= height as i32 {
                    continue;
                }
                let src_idx = ((src_y * frame.width + src_x) * 4) as usize;
                if frame.rgba.get(src_idx + 3).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let dst_idx = ((dst_y as u32 * width + dst_x as u32) * 4) as usize;
                alpha_blend_rgba(
                    &mut canvas[dst_idx..dst_idx + 4],
                    &frame.rgba[src_idx..src_idx + 4],
                );
            }
        }
    }

    PreviewImage {
        width,
        height,
        rgba_base64: base64::engine::general_purpose::STANDARD.encode(canvas),
        missing_imgs,
    }
}

pub fn compose_animation_frames(
    layer_frames: &[Vec<LayerFrame>],
    missing_imgs: Vec<String>,
) -> Vec<PreviewImage> {
    let animation_bounds = combined_bounds(
        layer_frames
            .iter()
            .flat_map(|frames| frames.iter().filter_map(visible_frame_bounds)),
    );
    let frame_count = layer_frames.iter().map(Vec::len).max().unwrap_or(0).max(1);
    (0..frame_count)
        .map(|frame_index| {
            let frames = layer_frames
                .iter()
                .filter(|frames| !frames.is_empty())
                .map(|frames| frames[frame_index % frames.len()].clone())
                .collect::<Vec<_>>();
            compose_layer_frames_with_bounds(&frames, missing_imgs.clone(), animation_bounds)
        })
        .collect()
}

fn combined_bounds<I>(bounds: I) -> Option<Bounds>
where
    I: IntoIterator<Item = Bounds>,
{
    let mut iter = bounds.into_iter();
    let first = iter.next()?;
    Some(iter.fold(first, |acc, bounds| {
        (
            acc.0.min(bounds.0),
            acc.1.min(bounds.1),
            acc.2.max(bounds.2),
            acc.3.max(bounds.3),
        )
    }))
}

fn visible_frame_bounds(frame: &LayerFrame) -> Option<(i32, i32, i32, i32)> {
    let mut min_x = frame.width;
    let mut min_y = frame.height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    let mut found = false;

    for src_y in 0..frame.height {
        for src_x in 0..frame.width {
            let src_idx = ((src_y * frame.width + src_x) * 4) as usize;
            if frame.rgba.get(src_idx + 3).copied().unwrap_or(0) == 0 {
                continue;
            }
            found = true;
            min_x = min_x.min(src_x);
            min_y = min_y.min(src_y);
            max_x = max_x.max(src_x + 1);
            max_y = max_y.max(src_y + 1);
        }
    }

    found.then_some((
        frame.x + min_x as i32,
        frame.y + min_y as i32,
        frame.x + max_x as i32,
        frame.y + max_y as i32,
    ))
}

pub fn render_layers_from_game_dir(
    game_dir: &Path,
    layers: &[PreviewLayerRequest],
) -> Result<PreviewImage, String> {
    let mut results = render_layer_groups_from_game_dir(game_dir, &[layers.to_vec()])?;
    results
        .pop()
        .ok_or_else(|| "preview renderer returned no image".to_string())
}

pub fn render_layer_groups_from_game_dir(
    game_dir: &Path,
    groups: &[Vec<PreviewLayerRequest>],
) -> Result<Vec<PreviewImage>, String> {
    render_layer_groups_results_from_game_dir(game_dir, groups)
        .into_iter()
        .collect()
}

pub fn render_layer_groups_results_from_game_dir(
    game_dir: &Path,
    groups: &[Vec<PreviewLayerRequest>],
) -> Vec<Result<PreviewImage, String>> {
    let mut npk_cache = HashMap::<PathBuf, NpkArchive>::new();
    let mut decoded_cache = HashMap::<(String, usize, usize), LayerFrame>::new();
    groups
        .iter()
        .map(|layers| {
            render_layers_with_cache(game_dir, layers, &mut npk_cache, &mut decoded_cache)
        })
        .collect()
}

fn render_layers_with_cache(
    game_dir: &Path,
    layers: &[PreviewLayerRequest],
    npk_cache: &mut HashMap<PathBuf, NpkArchive>,
    decoded_cache: &mut HashMap<(String, usize, usize), LayerFrame>,
) -> Result<PreviewImage, String> {
    let mut frames = Vec::new();
    let mut missing_imgs = Vec::new();

    for layer in layers
        .iter()
        .filter(|layer| !layer.img_path.trim().is_empty())
    {
        let Some((entry_path, npk_path, resolved_palette_index)) =
            resolve_existing_entry_with_cache(
                game_dir,
                &layer.img_path,
                layer.palette_index,
                npk_cache,
            )?
        else {
            missing_imgs.push(layer.img_path.clone());
            continue;
        };
        let archive = npk_cache
            .get(&npk_path)
            .ok_or_else(|| format!("failed to cache {}", npk_path.display()))?;
        let entry_index = archive
            .find_entry_index(&entry_path)
            .ok_or_else(|| format!("missing IMG entry {entry_path}"))?;
        let img = archive.read_img(entry_index)?;
        let palette_index = palette_index_for_base_entry(resolved_palette_index, img.version());
        let frame_infos = img.frames();
        if frame_infos.is_empty() {
            missing_imgs.push(entry_path);
            continue;
        }
        let frame_index = layer.frame_index % frame_infos.len();
        let cache_key = (entry_path.to_ascii_lowercase(), palette_index, frame_index);
        if let Some(frame) = decoded_cache.get(&cache_key) {
            frames.push(frame.clone());
            continue;
        }
        let decoded = img
            .decode_frame_with_palette(frame_index, palette_index)
            .map_err(|err| format!("{}: {}", entry_path, err))?;
        let frame = LayerFrame {
            width: decoded.width.max(1) as u32,
            height: decoded.height.max(1) as u32,
            x: decoded.x,
            y: decoded.y,
            rgba: decoded.rgba,
        };
        decoded_cache.insert(cache_key, frame.clone());
        frames.push(frame);
    }

    Ok(compose_layer_frames(&frames, missing_imgs))
}

pub fn render_animation_from_game_dir(
    game_dir: &Path,
    layers: &[PreviewLayerRequest],
) -> Result<PreviewAnimation, String> {
    let mut npk_cache = HashMap::<PathBuf, NpkArchive>::new();
    let mut decoded_cache = HashMap::<(String, usize), Vec<LayerFrame>>::new();
    let mut layer_frames = Vec::<Vec<LayerFrame>>::new();
    let mut missing_imgs = Vec::new();

    for layer in layers
        .iter()
        .filter(|layer| !layer.img_path.trim().is_empty())
    {
        let Some((entry_path, npk_path, resolved_palette_index)) =
            resolve_existing_entry_with_cache(
                game_dir,
                &layer.img_path,
                layer.palette_index,
                &mut npk_cache,
            )?
        else {
            missing_imgs.push(layer.img_path.clone());
            continue;
        };
        let archive = npk_cache
            .get(&npk_path)
            .ok_or_else(|| format!("failed to cache {}", npk_path.display()))?;

        let entry_index = archive
            .find_entry_index(&entry_path)
            .ok_or_else(|| format!("missing IMG entry {entry_path}"))?;
        let img = archive.read_img(entry_index)?;
        let palette_index = palette_index_for_base_entry(resolved_palette_index, img.version());
        let cache_key = (entry_path.to_ascii_lowercase(), palette_index);
        if let Some(frames) = decoded_cache.get(&cache_key) {
            layer_frames.push(frames.clone());
            continue;
        }
        let frame_infos = img.frames();
        if frame_infos.is_empty() {
            missing_imgs.push(entry_path.clone());
            continue;
        }

        let mut frames = Vec::with_capacity(frame_infos.len());
        for frame_info in frame_infos {
            let decoded = img
                .decode_frame_with_palette(frame_info.index, palette_index)
                .map_err(|err| format!("{} frame {}: {}", entry_path, frame_info.index, err))?;
            frames.push(LayerFrame {
                width: decoded.width.max(1) as u32,
                height: decoded.height.max(1) as u32,
                x: decoded.x,
                y: decoded.y,
                rgba: decoded.rgba,
            });
        }
        decoded_cache.insert(cache_key, frames.clone());
        layer_frames.push(frames);
    }

    let frames = compose_animation_frames(&layer_frames, Vec::new());
    Ok(PreviewAnimation {
        frame_count: frames.len(),
        missing_imgs,
        frames,
    })
}

fn resolve_existing_entry_with_cache(
    game_dir: &Path,
    img_path: &str,
    requested_palette_index: usize,
    npk_cache: &mut HashMap<PathBuf, NpkArchive>,
) -> Result<Option<(String, PathBuf, usize)>, String> {
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
            if !npk_cache.contains_key(&npk_path) {
                npk_cache.insert(npk_path.clone(), NpkArchive::open_path(&npk_path)?);
            }
            let Some(entry_index) = npk_cache
                .get(&npk_path)
                .and_then(|archive| archive.find_entry_index(entry_path))
            else {
                return Ok(None);
            };
            let img = npk_cache
                .get(&npk_path)
                .ok_or_else(|| format!("failed to cache {}", npk_path.display()))?
                .read_img(entry_index)?;
            Ok(Some((
                npk_path,
                ImgResourceMetadata {
                    version: img.version(),
                    palette_count: img.palette_count(),
                },
            )))
        },
    )?;
    Ok(resolved.map(|resolved| (resolved.entry_path, resolved.value, resolved.palette_index)))
}

pub(crate) fn palette_index_for_base_entry(
    requested_palette_index: usize,
    img_version: i32,
) -> usize {
    if img_version == 4 {
        0
    } else {
        requested_palette_index
    }
}
