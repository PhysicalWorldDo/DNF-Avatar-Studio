use std::collections::HashMap;
use std::io::Read;

use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};

use crate::dnflib_safe::{FrameInfo, ImgArchive};

use super::direct_color::inspect_v2_direct_colors;
use super::palette_patch::{FramePayloadLayout, PaletteLayout};

const DNF_FMT_ARGB_1555: i32 = 0x0e;
const DNF_FMT_LINK: i32 = 0x11;
const DNF_COMPRESS_NONE: i32 = 0x05;
const DNF_COMPRESS_ZLIB: i32 = 0x06;
const DNF_COMPRESS_DDS_ZLIB: i32 = 0x07;
const MAX_INDEX_PIXELS_PER_FRAME: usize = 64 * 1024 * 1024;
const MAX_COLOR_FRAME_BITMAP_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyeCapabilityStatus {
    Dyeable,
    PartiallyDyeable,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImgDyeInspection {
    pub img_version: u16,
    pub frame_count: u32,
    pub palette_count: u16,
    pub color_count: u16,
    pub capability: DyeCapabilityStatus,
    pub indexed_frame_count: u32,
    pub direct_frame_count: u32,
    pub link_frame_count: u32,
    pub used_color_pixel_counts: Vec<u64>,
    pub used_color_frame_counts: Vec<u32>,
    #[serde(skip)]
    pub used_color_frame_bitmaps: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct ImgDyeCatalog {
    pub img_version: u16,
    pub frame_count: u32,
    pub palette_layout: Option<PaletteLayout>,
    pub direct_palette: Option<super::direct_color::DirectColorPalette>,
    pub inspections: Vec<ImgDyeInspection>,
}

pub fn inspect_img_for_dye(bytes: &[u8], palette_index: usize) -> Result<ImgDyeInspection, String> {
    inspect_img_catalog(bytes)?
        .inspections
        .get(palette_index)
        .cloned()
        .ok_or_else(|| "Palette 索引无效".to_string())
}

pub(crate) fn inspect_img_catalog(bytes: &[u8]) -> Result<ImgDyeCatalog, String> {
    let (version, declared_frame_count) = common_header(bytes)?;
    let archive = ImgArchive::open_owned_bytes(bytes.to_vec())?;
    let frames = archive.frames();
    if archive.version() != version || frames.len() != declared_frame_count as usize {
        return Err("IMG 结构校验失败".to_string());
    }

    if version == 2 {
        let direct_palette = inspect_v2_direct_colors(bytes, &archive, &frames)?;
        let img_version = 2;
        let link_frame_count = u32::try_from(
            frames
                .iter()
                .filter(|frame| frame.format == DNF_FMT_LINK)
                .count(),
        )
        .unwrap_or(u32::MAX);
        let inspection = if let Some(palette) = direct_palette.as_ref() {
            ImgDyeInspection {
                img_version,
                frame_count: declared_frame_count,
                palette_count: 1,
                color_count: u16::try_from(palette.colors.len())
                    .map_err(|_| "V2 虚拟 Palette 颜色数量过多".to_string())?,
                capability: DyeCapabilityStatus::Dyeable,
                indexed_frame_count: 0,
                direct_frame_count: u32::try_from(frames.len()).unwrap_or(u32::MAX),
                link_frame_count,
                used_color_pixel_counts: palette.used_color_pixel_counts.clone(),
                used_color_frame_counts: palette.used_color_frame_counts.clone(),
                used_color_frame_bitmaps: palette.used_color_frame_bitmaps.clone(),
            }
        } else {
            ImgDyeInspection {
                img_version,
                frame_count: declared_frame_count,
                palette_count: 0,
                color_count: 0,
                capability: DyeCapabilityStatus::ReadOnly,
                indexed_frame_count: 0,
                direct_frame_count: u32::try_from(frames.len()).unwrap_or(u32::MAX),
                link_frame_count,
                used_color_pixel_counts: Vec::new(),
                used_color_frame_counts: Vec::new(),
                used_color_frame_bitmaps: Vec::new(),
            }
        };
        return Ok(ImgDyeCatalog {
            img_version,
            frame_count: declared_frame_count,
            palette_layout: None,
            direct_palette,
            inspections: vec![inspection],
        });
    }

    if !matches!(version, 4 | 6) {
        let img_version = u16::try_from(version).unwrap_or_default();
        return Ok(ImgDyeCatalog {
            img_version,
            frame_count: declared_frame_count,
            palette_layout: None,
            direct_palette: None,
            inspections: vec![ImgDyeInspection {
                img_version,
                frame_count: declared_frame_count,
                palette_count: u16::try_from(archive.palette_count()).unwrap_or(u16::MAX),
                color_count: 0,
                capability: DyeCapabilityStatus::ReadOnly,
                indexed_frame_count: 0,
                direct_frame_count: u32::try_from(frames.len()).unwrap_or(u32::MAX),
                link_frame_count: u32::try_from(
                    frames
                        .iter()
                        .filter(|frame| frame.format == DNF_FMT_LINK)
                        .count(),
                )
                .unwrap_or(u32::MAX),
                used_color_pixel_counts: Vec::new(),
                used_color_frame_counts: Vec::new(),
                used_color_frame_bitmaps: Vec::new(),
            }],
        });
    }

    let layout = PaletteLayout::parse(bytes)?;
    if archive.palette_count() != layout.boards.len() {
        return Err("IMG 结构校验失败".to_string());
    }
    let mut indexed_frame_count = 0_u32;
    let mut direct_frame_count = 0_u32;
    let mut link_frame_count = 0_u32;
    if frames.iter().any(|frame| {
        frame.format == DNF_FMT_ARGB_1555
            && !matches!(
                frame.compress,
                DNF_COMPRESS_NONE | DNF_COMPRESS_ZLIB | DNF_COMPRESS_DDS_ZLIB
            )
    }) {
        return Err("ARGB1555 帧使用了不支持的压缩格式".to_string());
    }
    for index in 0..frames.len() {
        if frames[index].format == DNF_FMT_LINK {
            link_frame_count = link_frame_count.saturating_add(1);
        }
        let effective = &frames[effective_frame_index(&frames, index)?];
        if is_palette_indexed_frame(effective) {
            indexed_frame_count = indexed_frame_count.saturating_add(1);
        } else {
            direct_frame_count = direct_frame_count.saturating_add(1);
        }
    }

    let max_color_count = layout
        .boards
        .iter()
        .map(|board| board.color_count)
        .max()
        .unwrap_or(0);
    let empty_palette_placeholder = indexed_frame_count > 0
        && max_color_count == 0
        && !layout.boards.is_empty()
        && indexed_frames_are_canvas_placeholders(&frames)?;
    if indexed_frame_count > 0 && max_color_count == 0 && !empty_palette_placeholder {
        return Err("索引帧的调色板为空".to_string());
    }
    let capability = if indexed_frame_count == 0 || empty_palette_placeholder {
        DyeCapabilityStatus::ReadOnly
    } else if direct_frame_count == 0 {
        DyeCapabilityStatus::Dyeable
    } else {
        DyeCapabilityStatus::PartiallyDyeable
    };
    let usage = if indexed_frame_count == 0 {
        IndexUsage {
            pixel_counts: vec![0; max_color_count],
            frame_counts: vec![0; max_color_count],
            frame_bitmaps: allocate_frame_bitmaps(max_color_count, frames.len())?,
        }
    } else if empty_palette_placeholder {
        layout.frame_payloads(bytes)?;
        IndexUsage {
            pixel_counts: Vec::new(),
            frame_counts: Vec::new(),
            frame_bitmaps: Vec::new(),
        }
    } else {
        scan_index_usage(bytes, &layout, max_color_count, &frames, &archive)?
    };
    let palette_count =
        u16::try_from(layout.boards.len()).map_err(|_| "Palette 数量过多".to_string())?;
    let mut inspections = Vec::with_capacity(layout.boards.len());
    for board in &layout.boards {
        if usage.pixel_counts[board.color_count..]
            .iter()
            .any(|count| *count > 0)
        {
            return Err("帧颜色索引超出调色板".to_string());
        }
        inspections.push(ImgDyeInspection {
            img_version: layout.img_version,
            frame_count: layout.frame_count,
            palette_count,
            color_count: u16::try_from(board.color_count)
                .map_err(|_| "Palette 颜色数量过多".to_string())?,
            capability,
            indexed_frame_count,
            direct_frame_count,
            link_frame_count,
            used_color_pixel_counts: usage.pixel_counts[..board.color_count].to_vec(),
            used_color_frame_counts: usage.frame_counts[..board.color_count].to_vec(),
            used_color_frame_bitmaps: usage.frame_bitmaps[..board.color_count].to_vec(),
        });
    }

    Ok(ImgDyeCatalog {
        img_version: layout.img_version,
        frame_count: layout.frame_count,
        palette_layout: Some(layout),
        direct_palette: None,
        inspections,
    })
}

fn is_palette_indexed_frame(frame: &FrameInfo) -> bool {
    frame.format == DNF_FMT_ARGB_1555 && frame.compress == DNF_COMPRESS_ZLIB
}

fn effective_frame_index(frames: &[FrameInfo], index: usize) -> Result<usize, String> {
    let mut current = index;
    for _ in 0..=frames.len() {
        let frame = frames
            .get(current)
            .ok_or_else(|| "LINK 帧目标无效".to_string())?;
        if frame.format != DNF_FMT_LINK {
            return Ok(current);
        }
        current = usize::try_from(frame.link_index).map_err(|_| "LINK 帧目标无效".to_string())?;
    }
    Err("LINK 帧存在循环引用".to_string())
}

fn indexed_frames_are_canvas_placeholders(frames: &[FrameInfo]) -> Result<bool, String> {
    for index in 0..frames.len() {
        let frame = &frames[effective_frame_index(frames, index)?];
        if is_palette_indexed_frame(frame) && !is_canvas_placeholder(frame) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn is_canvas_placeholder(frame: &FrameInfo) -> bool {
    frame.width == 1
        && frame.height == 1
        && matches!(frame.compress, DNF_COMPRESS_NONE | DNF_COMPRESS_ZLIB)
        && frame.canvas_width > 0
        && frame.canvas_height > 0
        && (frame.x >= frame.canvas_width
            || frame.y >= frame.canvas_height
            || frame.x.saturating_add(frame.width) <= 0
            || frame.y.saturating_add(frame.height) <= 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NativePlaceholderProbeKey<'a> {
    format: i32,
    compression: i32,
    width: i32,
    height: i32,
    declared_size: i32,
    x: i32,
    y: i32,
    canvas_width: i32,
    canvas_height: i32,
    data: &'a [u8],
}

struct IndexUsage {
    pixel_counts: Vec<u64>,
    frame_counts: Vec<u32>,
    frame_bitmaps: Vec<Vec<u8>>,
}

fn scan_index_usage(
    bytes: &[u8],
    layout: &PaletteLayout,
    color_count: usize,
    frames: &[FrameInfo],
    archive: &ImgArchive,
) -> Result<IndexUsage, String> {
    if color_count == 0 {
        return Err("索引帧的调色板为空".to_string());
    }
    let payloads = layout.frame_payloads(bytes)?;
    let mut pixel_counts = vec![0_u64; color_count];
    let mut used_colors_by_frame = vec![Vec::<u16>::new(); payloads.len()];
    let mut native_placeholder_cache = HashMap::new();
    for (frame_index, payload) in payloads.iter().enumerate() {
        let Some(payload) = payload else {
            continue;
        };
        let descriptor = payload;
        if descriptor.format != DNF_FMT_ARGB_1555 || descriptor.compression != DNF_COMPRESS_ZLIB {
            continue;
        }
        let data = bytes
            .get(descriptor.data_range.clone())
            .ok_or_else(|| "IMG 帧数据被截断".to_string())?;
        let frame = frames
            .get(frame_index)
            .ok_or_else(|| "IMG 帧元数据缺失".to_string())?;
        let canvas_placeholder = is_canvas_placeholder(frame);
        let pixel_count = frame_pixel_count(descriptor)?;
        if data.is_empty() {
            return Err("索引帧压缩数据为空".to_string());
        }
        let raw = match decompress_compatible_zlib(data, pixel_count) {
            Ok(decoded) => decoded,
            Err(ZlibPayloadError::DecodeFailed)
                if canvas_placeholder
                    && native_preview_decodes_transparent(
                        archive,
                        frame_index,
                        frame,
                        descriptor,
                        data,
                        &mut native_placeholder_cache,
                    ) =>
            {
                continue;
            }
            Err(error) => return Err(zlib_payload_error(error)),
        };
        if raw.len() < pixel_count {
            if canvas_placeholder
                && native_preview_decodes_transparent(
                    archive,
                    frame_index,
                    frame,
                    descriptor,
                    data,
                    &mut native_placeholder_cache,
                )
            {
                continue;
            }
            return Err(format!(
                "索引帧数据长度不足：frame={frame_index} format={} compression={} dimensions={}x{} declared_size={} stored_size={} decoded_size={} stored_prefix={:02x?} position=({}, {}) canvas={}x{}",
                descriptor.format,
                descriptor.compression,
                descriptor.width,
                descriptor.height,
                descriptor.declared_size,
                data.len(),
                raw.len(),
                &data[..data.len().min(16)],
                frame.x,
                frame.y,
                frame.canvas_width,
                frame.canvas_height,
            ));
        }
        let mut seen_in_frame = vec![false; color_count];
        for index in &raw[..pixel_count] {
            let color_index = usize::from(*index);
            let slot = pixel_counts
                .get_mut(color_index)
                .ok_or_else(|| "帧颜色索引超出调色板".to_string())?;
            *slot = slot
                .checked_add(1)
                .ok_or_else(|| "颜色使用量溢出".to_string())?;
            seen_in_frame[color_index] = true;
        }
        used_colors_by_frame[frame_index] = seen_in_frame
            .into_iter()
            .enumerate()
            .filter_map(|(color_index, used)| used.then_some(color_index as u16))
            .collect();
    }

    let mut frame_counts = vec![0_u32; color_count];
    let mut frame_bitmaps = allocate_frame_bitmaps(color_count, frames.len())?;
    for frame_index in 0..frames.len() {
        let effective_index = effective_frame_index(frames, frame_index)?;
        let used_colors = used_colors_by_frame
            .get(effective_index)
            .ok_or_else(|| "LINK 帧目标无效".to_string())?;
        for color_index in used_colors {
            let count = frame_counts
                .get_mut(usize::from(*color_index))
                .ok_or_else(|| "帧颜色索引超出调色板".to_string())?;
            *count = count
                .checked_add(1)
                .ok_or_else(|| "颜色影响帧数溢出".to_string())?;
            let bitmap = frame_bitmaps
                .get_mut(usize::from(*color_index))
                .ok_or_else(|| "帧颜色索引超出调色板".to_string())?;
            bitmap[frame_index / 8] |= 1_u8 << (frame_index % 8);
        }
    }

    Ok(IndexUsage {
        pixel_counts,
        frame_counts,
        frame_bitmaps,
    })
}

fn native_preview_decodes_transparent<'a>(
    archive: &ImgArchive,
    frame_index: usize,
    frame: &FrameInfo,
    descriptor: &FramePayloadLayout,
    data: &'a [u8],
    cache: &mut HashMap<NativePlaceholderProbeKey<'a>, bool>,
) -> bool {
    let key = NativePlaceholderProbeKey {
        format: descriptor.format,
        compression: descriptor.compression,
        width: descriptor.width,
        height: descriptor.height,
        declared_size: descriptor.declared_size,
        x: frame.x,
        y: frame.y,
        canvas_width: frame.canvas_width,
        canvas_height: frame.canvas_height,
        data,
    };
    if let Some(result) = cache.get(&key) {
        return *result;
    }
    let result = archive
        .decode_frame_with_palette(frame_index, 0)
        .is_ok_and(|decoded| decoded.rgba.iter().all(|channel| *channel == 0));
    cache.insert(key, result);
    result
}

fn allocate_frame_bitmaps(color_count: usize, frame_count: usize) -> Result<Vec<Vec<u8>>, String> {
    let bytes_per_color = frame_count
        .checked_add(7)
        .ok_or_else(|| "颜色影响帧位图大小溢出".to_string())?
        / 8;
    let total_bytes = color_count
        .checked_mul(bytes_per_color)
        .ok_or_else(|| "颜色影响帧位图大小溢出".to_string())?;
    if total_bytes > MAX_COLOR_FRAME_BITMAP_BYTES {
        return Err("颜色影响帧位图超过安全上限".to_string());
    }
    Ok(vec![vec![0_u8; bytes_per_color]; color_count])
}

fn frame_pixel_count(frame: &FramePayloadLayout) -> Result<usize, String> {
    let width = usize::try_from(frame.width).map_err(|_| "帧宽度无效".to_string())?;
    let height = usize::try_from(frame.height).map_err(|_| "帧高度无效".to_string())?;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| "帧像素数量溢出".to_string())?;
    if pixels > MAX_INDEX_PIXELS_PER_FRAME {
        return Err("帧像素数量超过染色安全上限".to_string());
    }
    Ok(pixels)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZlibPayloadError {
    DecodeFailed,
    SafetyLimit,
    LengthOverflow,
}

fn decompress_compatible_zlib(
    data: &[u8],
    expected_len: usize,
) -> Result<Vec<u8>, ZlibPayloadError> {
    if data.is_empty() {
        return Err(ZlibPayloadError::DecodeFailed);
    }
    if data[0] == 0x78 {
        return decompress_zlib_stream(data, expected_len);
    }
    let Some(found) = data.iter().rposition(|byte| *byte == 0x78) else {
        return Err(ZlibPayloadError::DecodeFailed);
    };
    let rotated_len = data[found..]
        .len()
        .checked_add(data.len())
        .ok_or(ZlibPayloadError::LengthOverflow)?;
    let mut rotated = Vec::with_capacity(rotated_len);
    rotated.extend_from_slice(&data[found..]);
    rotated.extend_from_slice(data);
    match decompress_zlib_stream(&rotated, expected_len) {
        Ok(decoded) => return Ok(decoded),
        Err(ZlibPayloadError::DecodeFailed) => {}
        Err(error) => return Err(error),
    }
    if found + 2 <= data.len() {
        let prefixed_len = data
            .len()
            .checked_add(2)
            .ok_or(ZlibPayloadError::LengthOverflow)?;
        let mut prefixed = Vec::with_capacity(prefixed_len);
        prefixed.extend_from_slice(&data[found..found + 2]);
        prefixed.extend_from_slice(data);
        return decompress_zlib_stream(&prefixed, expected_len);
    }
    Err(ZlibPayloadError::DecodeFailed)
}

fn decompress_zlib_stream(data: &[u8], expected_len: usize) -> Result<Vec<u8>, ZlibPayloadError> {
    let limit = MAX_INDEX_PIXELS_PER_FRAME
        .checked_add(1)
        .ok_or(ZlibPayloadError::LengthOverflow)?;
    let mut decoder = ZlibDecoder::new(data).take(limit as u64);
    let mut decoded = Vec::with_capacity(expected_len.min(1024 * 1024));
    decoder
        .read_to_end(&mut decoded)
        .map_err(|_| ZlibPayloadError::DecodeFailed)?;
    if decoded.len() > MAX_INDEX_PIXELS_PER_FRAME {
        return Err(ZlibPayloadError::SafetyLimit);
    }
    Ok(decoded)
}

fn zlib_payload_error(error: ZlibPayloadError) -> String {
    match error {
        ZlibPayloadError::DecodeFailed => "索引帧解压失败",
        ZlibPayloadError::SafetyLimit => "索引帧解压数据超过安全上限",
        ZlibPayloadError::LengthOverflow => "索引帧长度溢出",
    }
    .to_string()
}

pub(crate) fn img_header_metadata(bytes: &[u8]) -> Option<(u16, u32)> {
    let (version, frame_count) = common_header(bytes).ok()?;
    Some((u16::try_from(version).ok()?, frame_count))
}

fn common_header(bytes: &[u8]) -> Result<(i32, u32), String> {
    if bytes.len() < 32
        || !(bytes.starts_with(b"Neople Img File\0") || bytes.starts_with(b"Neople Image File\0"))
    {
        return Err("IMG magic 无效或文件被截断".to_string());
    }
    let version = read_i32(bytes, 24, "IMG 版本")?;
    let frame_count = read_i32(bytes, 28, "IMG 帧数")?;
    if !(0..=65_535).contains(&frame_count) {
        return Err("IMG 帧数无效".to_string());
    }
    Ok((version, frame_count as u32))
}

fn read_i32(bytes: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{label}数据被截断"))?;
    Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}
