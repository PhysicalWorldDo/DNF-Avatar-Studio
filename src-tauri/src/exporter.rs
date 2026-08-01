use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::path::Path;

use base64::Engine;
use image::codecs::webp::WebPEncoder;
use image::ExtendedColorType;

use crate::dnflib_safe::{NpkArchive, NpkBuilder};
use crate::dnflib_sys::{self, ImageFrame, ImgFile};
use crate::game_resolver::{
    img_path_lookup_candidates, normalize_img_path_for_npk, resolve_game_npk_path,
    resolve_img_resource, same_named_mask_number, ImgResourceMetadata,
};
use crate::models::{
    ExportNpkRequest, ExportNpkResult, ExportWebpRequest, ExportWebpResult, PreviewImage,
};

const IMG_FORMAT_ARGB_8888: i32 = 0x10;
const IMG_COMPRESS_NONE: i32 = 0x05;
const DEFAULT_WEBP_FRAME_DURATION_MS: u32 = 80;
const WEBP_CHUNK_HEADER_SIZE: usize = 8;
const WEBP_RIFF_HEADER_SIZE: usize = 12;

pub fn export_avatar_npk(request: ExportNpkRequest) -> Result<ExportNpkResult, String> {
    let output_path = Path::new(&request.output_path);
    let entry_count = match request.mode.as_str() {
        "composed" => export_composed(output_path, &request)?,
        "original" => export_original(output_path, &request)?,
        other => return Err(format!("unknown export mode: {other}")),
    };
    Ok(ExportNpkResult {
        output_path: request.output_path,
        entry_count,
    })
}

pub fn export_avatar_webp(request: ExportWebpRequest) -> Result<ExportWebpResult, String> {
    let animation = crate::renderer::render_animation_from_game_dir(
        Path::new(&request.game_dir),
        &request.layers,
    )?;
    if !animation.missing_imgs.is_empty() {
        return Err(format!(
            "cannot export WebP, missing {} IMG",
            animation.missing_imgs.len()
        ));
    }
    let frame_duration_ms = if request.frame_duration_ms == 0 {
        DEFAULT_WEBP_FRAME_DURATION_MS
    } else {
        request.frame_duration_ms
    };
    let bytes = preview_animation_webp_bytes(&animation.frames, frame_duration_ms)?;
    let first = animation
        .frames
        .first()
        .ok_or_else(|| "composed animation is empty".to_string())?;
    fs::write(&request.output_path, bytes)
        .map_err(|err| format!("failed to write WebP {}: {err}", request.output_path))?;
    Ok(ExportWebpResult {
        output_path: request.output_path,
        width: first.width,
        height: first.height,
        frame_count: animation.frames.len(),
    })
}

fn export_composed(output_path: &Path, request: &ExportNpkRequest) -> Result<usize, String> {
    let animation = crate::renderer::render_animation_from_game_dir(
        Path::new(&request.game_dir),
        &request.layers,
    )?;
    if !animation.missing_imgs.is_empty() {
        return Err(format!(
            "cannot export composed NPK, missing {} IMG",
            animation.missing_imgs.len()
        ));
    }
    let frames = animation
        .frames
        .iter()
        .map(preview_frame_pixels)
        .collect::<Result<Vec<_>, _>>()?;
    let img_bytes = multi_frame_img_bytes(&frames)?;
    let entry_name = if request.entry_name.trim().is_empty() {
        "sprite/avatar_studio/composed.img"
    } else {
        request.entry_name.trim()
    };
    let mut builder = NpkBuilder::create()?;
    let composed_entry_path = normalize_img_path_for_npk(entry_name);
    builder.add_entry(&composed_entry_path, &img_bytes)?;
    let mut seen = HashSet::<String>::new();
    seen.insert(composed_entry_path.to_ascii_lowercase());
    let mut added = 1usize;
    for layer in request
        .layers
        .iter()
        .filter(|layer| !layer.img_path.trim().is_empty())
    {
        let entry_path = normalize_img_path_for_npk(&layer.img_path);
        let original_entries =
            original_entry_and_mask_owned_bytes(Path::new(&request.game_dir), &entry_path, layer)?;
        let base_entry_path = original_entries
            .first()
            .map(|(path, _)| path.clone())
            .unwrap_or_else(|| entry_path.clone());
        for (mask_or_base_path, bytes) in original_entries {
            if same_named_mask_number(&base_entry_path, &mask_or_base_path).is_none() {
                continue;
            }
            let key = mask_or_base_path.to_ascii_lowercase();
            if !seen.insert(key) {
                continue;
            }
            builder.add_entry(&mask_or_base_path, &bytes)?;
            added += 1;
        }
    }
    builder.save_path(output_path, false)?;
    Ok(added)
}

fn export_original(output_path: &Path, request: &ExportNpkRequest) -> Result<usize, String> {
    let mut builder = NpkBuilder::create()?;
    let mut seen = HashSet::<String>::new();
    let mut added = 0usize;
    for layer in request
        .layers
        .iter()
        .filter(|layer| !layer.img_path.trim().is_empty())
    {
        let entry_path = normalize_img_path_for_npk(&layer.img_path);
        for (entry_path, bytes) in
            original_entry_and_mask_owned_bytes(Path::new(&request.game_dir), &entry_path, layer)?
        {
            let key = entry_path.to_ascii_lowercase();
            if !seen.insert(key) {
                continue;
            }
            builder.add_entry(&entry_path, &bytes)?;
            added += 1;
        }
    }
    if added == 0 {
        return Err("no IMG entries to export".to_string());
    }
    builder.save_path(output_path, true)?;
    Ok(added)
}

fn original_entry_and_mask_owned_bytes(
    game_dir: &Path,
    img_path: &str,
    layer: &crate::models::PreviewLayerRequest,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let requested_entry_path = normalize_img_path_for_npk(img_path);
    let exact_candidates = img_path_lookup_candidates(&requested_entry_path);
    let mut saw_npk = false;

    let resolved = resolve_img_resource(
        &requested_entry_path,
        layer.palette_index,
        |entry_path| -> Result<_, String> {
            let Some(npk_path) = resolve_game_npk_path(game_dir, &entry_path) else {
                return Ok(None);
            };
            if !npk_path.is_file() {
                return Ok(None);
            }
            saw_npk = true;
            let archive = NpkArchive::open_path(&npk_path)?;
            let Some(entry_index) = archive.find_entry_index(entry_path) else {
                return Ok(None);
            };
            let metadata = match archive.read_img(entry_index) {
                Ok(img) => ImgResourceMetadata {
                    version: img.version(),
                    palette_count: img.palette_count(),
                },
                Err(_)
                    if exact_candidates
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(entry_path)) =>
                {
                    ImgResourceMetadata {
                        version: 0,
                        palette_count: 0,
                    }
                }
                Err(error) => {
                    return Err(format!("解析 IMG 调色板载体失败 {entry_path}：{error}"));
                }
            };
            Ok(Some(((archive, entry_index), metadata)))
        },
    )?;
    if let Some(resolved) = resolved {
        let entry_path = resolved.entry_path;
        let (archive, entry_index) = resolved.value;
        let mut entries = vec![(0_u32, entry_path.clone(), entry_index)];
        entries.extend(archive.entries().into_iter().filter_map(|entry| {
            same_named_mask_number(&entry_path, &entry.name)
                .filter(|number| layer.enable_dye_mask(*number))
                .map(|number| (number, entry.name.clone(), entry.index))
        }));
        entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        return entries
            .into_iter()
            .map(|(_, path, index)| {
                archive
                    .entry_bytes(index)
                    .map(|bytes| (path, bytes.to_vec()))
            })
            .collect();
    }

    if saw_npk {
        Err(format!("missing IMG entry {requested_entry_path}"))
    } else {
        Err(format!("missing NPK for {requested_entry_path}"))
    }
}

pub fn single_frame_img_bytes(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    multi_frame_img_bytes(&[(width, height, rgba.to_vec())])
}

pub fn multi_frame_img_bytes(frames: &[(u32, u32, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if frames.is_empty() {
        return Err("composed animation is empty".to_string());
    }
    let temp_path = std::env::temp_dir().join(format!(
        "dnf_avatar_img_{}_{}.img",
        std::process::id(),
        unique_nanos()
    ));
    let c_path = CString::new(temp_path.to_string_lossy().as_bytes())
        .map_err(|_| format!("temp IMG path contains NUL byte: {}", temp_path.display()))?;

    let mut encoded = Vec::with_capacity(frames.len());
    for (width, height, rgba) in frames {
        encoded.push(encode_img_frame(*width, *height, rgba)?);
    }
    let mut image_frames = encoded
        .iter_mut()
        .map(|frame| ImageFrame {
            format: IMG_FORMAT_ARGB_8888,
            compress: IMG_COMPRESS_NONE,
            w: frame.width as i32,
            h: frame.height as i32,
            x: 0,
            y: 0,
            mw: frame.width as i32,
            mh: frame.height as i32,
            data: frame.data,
            data_size: frame.data_size,
            link_index: 0,
        })
        .collect::<Vec<_>>();

    let mut img = ImgFile {
        version: 2,
        frames: image_frames.as_mut_ptr(),
        frame_count: image_frames.len() as i32,
        private_data: std::ptr::null_mut(),
    };
    let save_rc = unsafe { dnflib_sys::dnf_img_save_file(c_path.as_ptr(), &mut img) };
    for frame in encoded {
        unsafe { dnflib_sys::dnf_free(frame.data.cast()) };
    }
    if save_rc != dnflib_sys::DNF_OK {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("dnflib save IMG failed: {save_rc}"));
    }
    let bytes = std::fs::read(&temp_path)
        .map_err(|err| format!("failed to read temp IMG {}: {err}", temp_path.display()))?;
    let _ = std::fs::remove_file(temp_path);
    Ok(bytes)
}

struct EncodedImgFrame {
    width: u32,
    height: u32,
    data: *mut u8,
    data_size: i32,
}

fn encode_img_frame(width: u32, height: u32, rgba: &[u8]) -> Result<EncodedImgFrame, String> {
    if width == 0 || height == 0 {
        return Err("composed image is empty".to_string());
    }
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "invalid RGBA length {}, expected {}",
            rgba.len(),
            expected
        ));
    }

    let mut data = std::ptr::null_mut();
    let mut data_size = 0;
    let rc = unsafe {
        dnflib_sys::dnf_image_encode(
            rgba.as_ptr(),
            width as i32,
            height as i32,
            IMG_FORMAT_ARGB_8888,
            IMG_COMPRESS_NONE,
            &mut data,
            &mut data_size,
        )
    };
    if rc != dnflib_sys::DNF_OK {
        return Err(format!("dnflib encode IMG frame failed: {rc}"));
    }
    if data.is_null() || data_size <= 0 {
        return Err("dnflib returned empty encoded IMG frame".to_string());
    }

    Ok(EncodedImgFrame {
        width,
        height,
        data,
        data_size,
    })
}

fn preview_frame_pixels(image: &PreviewImage) -> Result<(u32, u32, Vec<u8>), String> {
    let rgba = base64::engine::general_purpose::STANDARD
        .decode(image.rgba_base64.as_bytes())
        .map_err(|err| format!("failed to decode preview pixels: {err}"))?;
    let expected = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "preview dimensions overflow".to_string())? as usize;
    if rgba.len() != expected {
        return Err(format!(
            "invalid preview RGBA length {}, expected {}",
            rgba.len(),
            expected
        ));
    }
    Ok((image.width, image.height, rgba))
}

fn preview_image_webp_bytes(image: &PreviewImage) -> Result<Vec<u8>, String> {
    let (width, height, rgba) = preview_frame_pixels(image)?;
    let mut bytes = Vec::new();
    WebPEncoder::new_lossless(&mut bytes)
        .encode(&rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|err| format!("failed to encode WebP: {err}"))?;
    Ok(bytes)
}

fn preview_animation_webp_bytes(
    frames: &[PreviewImage],
    frame_duration_ms: u32,
) -> Result<Vec<u8>, String> {
    let first = frames
        .first()
        .ok_or_else(|| "composed animation is empty".to_string())?;
    validate_webp_dimension(first.width, "width")?;
    validate_webp_dimension(first.height, "height")?;

    let duration = frame_duration_ms.clamp(1, 0x00ff_ffff);
    let mut chunks = Vec::<Vec<u8>>::new();
    chunks.push(webp_vp8x_chunk(first.width, first.height)?);
    chunks.push(webp_anim_chunk());

    for frame in frames {
        if frame.width != first.width || frame.height != first.height {
            return Err(format!(
                "inconsistent WebP frame size {}x{}, expected {}x{}",
                frame.width, frame.height, first.width, first.height
            ));
        }
        let frame_webp = preview_image_webp_bytes(frame)?;
        let frame_chunks = extract_webp_image_chunks(&frame_webp)?;
        chunks.push(webp_anmf_chunk(
            frame.width,
            frame.height,
            duration,
            &frame_chunks,
        )?);
    }

    let riff_payload_len = 4usize
        .checked_add(chunks.iter().map(Vec::len).sum::<usize>())
        .ok_or_else(|| "animated WebP is too large".to_string())?;
    let riff_size =
        u32::try_from(riff_payload_len).map_err(|_| "animated WebP is too large".to_string())?;

    let mut bytes = Vec::with_capacity(WEBP_CHUNK_HEADER_SIZE + riff_payload_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WEBP");
    for chunk in chunks {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn webp_vp8x_chunk(width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut payload = Vec::with_capacity(10);
    payload.push(0x12); // alpha + animation
    payload.extend_from_slice(&[0, 0, 0]);
    push_u24_le(&mut payload, width - 1)?;
    push_u24_le(&mut payload, height - 1)?;
    webp_chunk(*b"VP8X", &payload)
}

fn webp_anim_chunk() -> Vec<u8> {
    let mut payload = Vec::with_capacity(6);
    payload.extend_from_slice(&[0, 0, 0, 0]); // transparent background hint
    payload.extend_from_slice(&0_u16.to_le_bytes()); // loop forever
    webp_chunk(*b"ANIM", &payload).expect("fixed ANIM chunk is valid")
}

fn webp_anmf_chunk(
    width: u32,
    height: u32,
    duration_ms: u32,
    image_chunks: &[u8],
) -> Result<Vec<u8>, String> {
    let mut payload = Vec::with_capacity(16 + image_chunks.len());
    push_u24_le(&mut payload, 0)?;
    push_u24_le(&mut payload, 0)?;
    push_u24_le(&mut payload, width - 1)?;
    push_u24_le(&mut payload, height - 1)?;
    push_u24_le(&mut payload, duration_ms)?;
    payload.push(0b0000_0010); // full-canvas frame replaces the previous one
    payload.extend_from_slice(image_chunks);
    webp_chunk(*b"ANMF", &payload)
}

fn extract_webp_image_chunks(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < WEBP_RIFF_HEADER_SIZE
        || &bytes[0..4] != b"RIFF"
        || &bytes[8..WEBP_RIFF_HEADER_SIZE] != b"WEBP"
    {
        return Err("invalid WebP frame bytes".to_string());
    }
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let declared_len = riff_size
        .checked_add(WEBP_CHUNK_HEADER_SIZE)
        .ok_or_else(|| "WebP frame size overflow".to_string())?;
    if declared_len > bytes.len() {
        return Err("truncated WebP frame bytes".to_string());
    }
    Ok(bytes[WEBP_RIFF_HEADER_SIZE..declared_len].to_vec())
}

fn webp_chunk(fourcc: [u8; 4], payload: &[u8]) -> Result<Vec<u8>, String> {
    let size = u32::try_from(payload.len()).map_err(|_| "WebP chunk is too large".to_string())?;
    let padded_len = payload
        .len()
        .checked_add(payload.len() & 1)
        .ok_or_else(|| "WebP chunk is too large".to_string())?;
    let mut bytes = Vec::with_capacity(WEBP_CHUNK_HEADER_SIZE + padded_len);
    bytes.extend_from_slice(&fourcc);
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        bytes.push(0);
    }
    Ok(bytes)
}

fn push_u24_le(bytes: &mut Vec<u8>, value: u32) -> Result<(), String> {
    if value > 0x00ff_ffff {
        return Err("animated WebP field exceeds 24-bit limit".to_string());
    }
    bytes.write_all(&value.to_le_bytes()[..3]).unwrap();
    Ok(())
}

fn validate_webp_dimension(value: u32, name: &str) -> Result<(), String> {
    if value == 0 || value > 0x0100_0000 {
        return Err(format!("invalid WebP {name}: {value}"));
    }
    Ok(())
}

fn unique_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}
