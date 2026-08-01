use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::ops::Range;

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::dnflib_safe::{FrameInfo, ImgArchive};

use super::models::{DyeSourceFingerprintV1, DyeTargetRecipeV1};

pub(crate) const DNF_FMT_ARGB_1555: i32 = 0x0e;
pub(crate) const DNF_FMT_ARGB_4444: i32 = 0x0f;
pub(crate) const DNF_FMT_ARGB_8888: i32 = 0x10;
pub(crate) const DNF_FMT_LINK: i32 = 0x11;
pub(crate) const DNF_COMPRESS_NONE: i32 = 0x05;
pub(crate) const DNF_COMPRESS_ZLIB: i32 = 0x06;

const MAX_DIRECT_COLORS: usize = 256;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const MAX_DIRECT_FRAME_DECODED_RGBA_BYTES: usize = 64 * 1024 * 1024;
const MAX_DIRECT_TOTAL_DECODED_RGBA_BYTES: usize = 256 * 1024 * 1024;
const MAX_COLOR_FRAME_BITMAP_BYTES: usize = 2 * 1024 * 1024;
const VIRTUAL_PALETTE_HASH_DOMAIN: &[u8] = b"dnf-avatar-studio:v2-direct-colors:v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectColorFormat {
    Argb1555,
    Argb4444,
    Argb8888,
}

impl DirectColorFormat {
    fn from_raw(value: i32) -> Option<Self> {
        match value {
            DNF_FMT_ARGB_1555 => Some(Self::Argb1555),
            DNF_FMT_ARGB_4444 => Some(Self::Argb4444),
            DNF_FMT_ARGB_8888 => Some(Self::Argb8888),
            _ => None,
        }
    }

    pub(crate) fn quantize_rgb(self, rgb: [u8; 3]) -> [u8; 3] {
        match self {
            Self::Argb1555 => rgb.map(|channel| {
                let value = channel >> 3;
                (value << 3) | (value >> 2)
            }),
            Self::Argb4444 => rgb.map(|channel| channel & 0xf0),
            Self::Argb8888 => rgb,
        }
    }

    pub(crate) fn rgb_encoding(self) -> &'static str {
        match self {
            Self::Argb1555 => "argb1555",
            Self::Argb4444 => "argb4444",
            Self::Argb8888 => "argb8888",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DirectColorPalette {
    pub(crate) format: DirectColorFormat,
    pub(crate) colors: Vec<[u8; 4]>,
    pub(crate) used_color_pixel_counts: Vec<u64>,
    pub(crate) used_color_frame_counts: Vec<u32>,
    pub(crate) used_color_frame_bitmaps: Vec<Vec<u8>>,
}

pub(crate) fn inspect_v2_direct_colors(
    bytes: &[u8],
    archive: &ImgArchive,
    frames: &[FrameInfo],
) -> Result<Option<DirectColorPalette>, String> {
    let mut format = None;
    for frame in frames.iter().filter(|frame| frame.format != DNF_FMT_LINK) {
        let Some(frame_format) = DirectColorFormat::from_raw(frame.format) else {
            return Ok(None);
        };
        if !matches!(frame.compress, DNF_COMPRESS_NONE | DNF_COMPRESS_ZLIB) {
            return Ok(None);
        }
        if format
            .replace(frame_format)
            .is_some_and(|prior| prior != frame_format)
        {
            return Ok(None);
        }
    }
    let Some(format) = format else {
        return Ok(None);
    };
    let effective_frame_indices = match resolve_effective_frame_indices(frames) {
        Ok(indices) => indices,
        Err(_) => return Ok(None),
    };
    if checked_total_decoded_rgba_bytes(frames).is_err() {
        return Ok(None);
    }
    let storage = match V2StorageLayout::parse(bytes, frames) {
        Ok(storage) => storage,
        Err(_) => return Ok(None),
    };
    for frame in &storage.frames {
        let Some(range) = frame.data_range.as_ref() else {
            continue;
        };
        if frame.compression == DNF_COMPRESS_ZLIB
            && decompress_exact(&bytes[range.clone()], frame.raw_size).is_err()
        {
            return Ok(None);
        }
    }

    let mut pixel_counts = BTreeMap::<[u8; 4], u64>::new();
    let mut physical_frame_colors = vec![BTreeSet::<[u8; 4]>::new(); frames.len()];
    for (frame_index, frame) in frames.iter().enumerate() {
        if frame.format == DNF_FMT_LINK {
            continue;
        }
        let pixel_count = checked_pixel_count(frame)?;
        let decoded = match archive.decode_frame(frame_index) {
            Ok(decoded) => decoded,
            Err(_) => return Ok(None),
        };
        if decoded.width != frame.width
            || decoded.height != frame.height
            || decoded.rgba.len() != pixel_count.saturating_mul(4)
        {
            return Err("V2 直色帧解码结果与帧头不一致".to_string());
        }
        for pixel in decoded.rgba.chunks_exact(4) {
            let rgba = [pixel[0], pixel[1], pixel[2], pixel[3]];
            if !pixel_counts.contains_key(&rgba) && pixel_counts.len() >= MAX_DIRECT_COLORS {
                return Ok(None);
            }
            let count = pixel_counts.entry(rgba).or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| "V2 直色像素使用量溢出".to_string())?;
            physical_frame_colors[frame_index].insert(rgba);
        }
    }

    let colors = pixel_counts.keys().copied().collect::<Vec<_>>();
    let color_indices = colors
        .iter()
        .enumerate()
        .map(|(index, color)| (*color, index))
        .collect::<BTreeMap<_, _>>();
    let used_color_pixel_counts = colors
        .iter()
        .map(|color| pixel_counts.get(color).copied().unwrap_or_default())
        .collect::<Vec<_>>();
    let bytes_per_color = frames
        .len()
        .checked_add(7)
        .ok_or_else(|| "V2 颜色影响帧位图大小溢出".to_string())?
        / 8;
    let bitmap_bytes = colors
        .len()
        .checked_mul(bytes_per_color)
        .ok_or_else(|| "V2 颜色影响帧位图大小溢出".to_string())?;
    if bitmap_bytes > MAX_COLOR_FRAME_BITMAP_BYTES {
        return Err("V2 颜色影响帧位图超过安全上限".to_string());
    }
    let mut used_color_frame_counts = vec![0_u32; colors.len()];
    let mut used_color_frame_bitmaps = vec![vec![0_u8; bytes_per_color]; colors.len()];
    for frame_index in 0..frames.len() {
        let effective_index = effective_frame_indices[frame_index];
        for color in &physical_frame_colors[effective_index] {
            let color_index = *color_indices
                .get(color)
                .ok_or_else(|| "V2 虚拟 Palette 颜色索引缺失".to_string())?;
            used_color_frame_counts[color_index] = used_color_frame_counts[color_index]
                .checked_add(1)
                .ok_or_else(|| "V2 颜色影响帧数溢出".to_string())?;
            used_color_frame_bitmaps[color_index][frame_index / 8] |= 1_u8 << (frame_index % 8);
        }
    }

    Ok(Some(DirectColorPalette {
        format,
        colors,
        used_color_pixel_counts,
        used_color_frame_counts,
        used_color_frame_bitmaps,
    }))
}

pub(crate) fn virtual_palette_sha256(colors: &[[u8; 4]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(VIRTUAL_PALETTE_HASH_DOMAIN);
    hasher.update((colors.len() as u64).to_le_bytes());
    for color in colors {
        hasher.update(color);
    }
    hasher.finalize().into()
}

pub(crate) fn direct_source_fingerprint(
    bytes: &[u8],
    palette_index: usize,
) -> Result<DyeSourceFingerprintV1, String> {
    if palette_index != 0 {
        return Err("V2 直色 IMG 只有虚拟 Palette P0".to_string());
    }
    let archive = ImgArchive::open_owned_bytes(bytes.to_vec())?;
    if archive.version() != 2 {
        return Err("当前 IMG 不是 V2 直色资源".to_string());
    }
    let frames = archive.frames();
    let palette = inspect_v2_direct_colors(bytes, &archive, &frames)?
        .ok_or_else(|| "V2 IMG 不满足直色染色条件".to_string())?;
    Ok(DyeSourceFingerprintV1 {
        img_version: 2,
        frame_count: u32::try_from(frames.len()).map_err(|_| "IMG 帧数过多".to_string())?,
        palette_count: 1,
        color_count: u16::try_from(palette.colors.len())
            .map_err(|_| "V2 虚拟 Palette 颜色数量过多".to_string())?,
        palette_sha256: virtual_palette_sha256(&palette.colors),
        entry_sha256: Sha256::digest(bytes).into(),
    })
}

pub(crate) fn build_patched_v2_direct_bytes(
    source: &[u8],
    targets: &[&DyeTargetRecipeV1],
) -> Result<Vec<u8>, String> {
    let archive = ImgArchive::open_owned_bytes(source.to_vec())?;
    if archive.version() != 2 {
        return Err("当前 IMG 不是 V2 直色资源".to_string());
    }
    let source_frames = archive.frames();
    let palette = inspect_v2_direct_colors(source, &archive, &source_frames)?
        .ok_or_else(|| "V2 IMG 不满足直色染色条件".to_string())?;
    let current_fingerprint = DyeSourceFingerprintV1 {
        img_version: 2,
        frame_count: u32::try_from(source_frames.len()).map_err(|_| "IMG 帧数过多".to_string())?,
        palette_count: 1,
        color_count: u16::try_from(palette.colors.len())
            .map_err(|_| "V2 虚拟 Palette 颜色数量过多".to_string())?,
        palette_sha256: virtual_palette_sha256(&palette.colors),
        entry_sha256: Sha256::digest(source).into(),
    };
    let mut edits = BTreeMap::<[u8; 4], [u8; 3]>::new();
    for target in targets {
        if target.palette_index != 0 {
            return Err("V2 直色 IMG 只有虚拟 Palette P0".to_string());
        }
        if target.source != current_fingerprint {
            return Err("源资源已变化，染色配方不能应用".to_string());
        }
        for edit in &target.colors {
            let actual = palette
                .colors
                .get(usize::from(edit.color_index))
                .copied()
                .ok_or_else(|| "颜色索引无效".to_string())?;
            if actual != edit.expected_rgba {
                return Err("V2 虚拟 Palette 原色不一致，源资源已变化".to_string());
            }
            let replacement = palette.format.quantize_rgb(edit.replacement_rgb);
            if actual[3] == 0 && replacement != actual[..3] {
                return Err("完全透明颜色不可修改".to_string());
            }
            if let Some(prior) = edits.insert(actual, replacement) {
                if prior != replacement {
                    return Err("同一 V2 直色存在冲突配方".to_string());
                }
            }
        }
    }
    if edits.is_empty() {
        return Ok(source.to_vec());
    }

    let layout = V2StorageLayout::parse(source, &source_frames)?;
    let mut header = source[..layout.data_start].to_vec();
    let mut rebuilt_data = Vec::new();
    for frame in &layout.frames {
        let Some(data_range) = frame.data_range.as_ref() else {
            continue;
        };
        let stored = &source[data_range.clone()];
        let mut raw = match frame.compression {
            DNF_COMPRESS_NONE => stored.to_vec(),
            DNF_COMPRESS_ZLIB => decompress_exact(stored, frame.raw_size)?,
            _ => return Err("V2 直色帧使用了不支持的压缩格式".to_string()),
        };
        if raw.len() != frame.raw_size {
            return Err("V2 直色帧解压长度不一致".to_string());
        }
        let mut changed = false;
        for pixel in raw.chunks_exact_mut(frame.bytes_per_pixel) {
            let original = decode_direct_pixel(frame.format, pixel)?;
            let Some(replacement_rgb) = edits.get(&original) else {
                continue;
            };
            if replacement_rgb == &original[..3] {
                continue;
            }
            encode_direct_pixel(
                frame.format,
                [
                    replacement_rgb[0],
                    replacement_rgb[1],
                    replacement_rgb[2],
                    original[3],
                ],
                pixel,
            )?;
            changed = true;
        }
        let rebuilt = if !changed {
            stored.to_vec()
        } else if frame.compression == DNF_COMPRESS_ZLIB {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(&raw)
                .map_err(|error| format!("V2 直色帧压缩失败：{error}"))?;
            encoder
                .finish()
                .map_err(|error| format!("V2 直色帧压缩失败：{error}"))?
        } else {
            raw
        };
        if frame.compression == DNF_COMPRESS_ZLIB {
            let stored_size =
                i32::try_from(rebuilt.len()).map_err(|_| "V2 直色帧压缩数据过大".to_string())?;
            header[frame.size_offset..frame.size_offset + 4]
                .copy_from_slice(&stored_size.to_le_bytes());
        }
        rebuilt_data.extend_from_slice(&rebuilt);
    }
    let mut patched = header;
    patched.extend_from_slice(&rebuilt_data);
    patched.extend_from_slice(&source[layout.data_end..]);

    let reopened = ImgArchive::open_owned_bytes(patched.clone())?;
    if reopened.version() != 2 || reopened.frames() != source_frames {
        return Err("补丁后 V2 IMG 结构校验失败".to_string());
    }
    Ok(patched)
}

#[derive(Debug)]
struct V2FrameStorage {
    format: i32,
    compression: i32,
    size_offset: usize,
    raw_size: usize,
    bytes_per_pixel: usize,
    data_range: Option<Range<usize>>,
}

#[derive(Debug)]
struct V2StorageLayout {
    data_start: usize,
    data_end: usize,
    frames: Vec<V2FrameStorage>,
}

impl V2StorageLayout {
    fn parse(bytes: &[u8], frames: &[FrameInfo]) -> Result<Self, String> {
        let mut cursor = 32_usize;
        let mut parsed = Vec::with_capacity(frames.len());
        for expected in frames {
            let format = read_i32(bytes, cursor, "V2 帧格式")?;
            if format != expected.format {
                return Err("V2 帧头与解析元数据不一致".to_string());
            }
            if format == DNF_FMT_LINK {
                let target = read_i32(bytes, cursor + 4, "V2 LINK 目标")?;
                if target != expected.link_index {
                    return Err("V2 LINK 帧头与解析元数据不一致".to_string());
                }
                parsed.push(V2FrameStorage {
                    format,
                    compression: 0,
                    size_offset: 0,
                    raw_size: 0,
                    bytes_per_pixel: 0,
                    data_range: None,
                });
                cursor = cursor
                    .checked_add(8)
                    .ok_or_else(|| "V2 LINK 帧头偏移溢出".to_string())?;
                continue;
            }
            let compression = read_i32(bytes, cursor + 4, "V2 帧压缩格式")?;
            let width = read_i32(bytes, cursor + 8, "V2 帧宽度")?;
            let height = read_i32(bytes, cursor + 12, "V2 帧高度")?;
            let declared_size = read_i32(bytes, cursor + 16, "V2 帧数据长度")?;
            if compression != expected.compress
                || width != expected.width
                || height != expected.height
                || declared_size < 0
            {
                return Err("V2 帧头与解析元数据不一致".to_string());
            }
            let bytes_per_pixel = bytes_per_pixel(format)?;
            let pixel_count = checked_pixel_count(expected)?;
            let raw_size = pixel_count
                .checked_mul(bytes_per_pixel)
                .ok_or_else(|| "V2 直色帧长度溢出".to_string())?;
            parsed.push(V2FrameStorage {
                format,
                compression,
                size_offset: cursor + 16,
                raw_size,
                bytes_per_pixel,
                data_range: None,
            });
            cursor = cursor
                .checked_add(36)
                .ok_or_else(|| "V2 帧头偏移溢出".to_string())?;
        }
        let data_start = if bytes.starts_with(b"Neople Img File\0") {
            let images_size = read_i32(bytes, 16, "V2 IMG 头区长度")?;
            if images_size < 0 {
                return Err("V2 IMG 头区长度无效".to_string());
            }
            32_usize
                .checked_add(images_size as usize)
                .ok_or_else(|| "V2 IMG 数据区偏移溢出".to_string())?
        } else {
            cursor
        };
        if data_start < cursor || data_start > bytes.len() {
            return Err("V2 IMG 数据区偏移无效".to_string());
        }
        let mut data_cursor = data_start;
        for frame in &mut parsed {
            if frame.format == DNF_FMT_LINK {
                continue;
            }
            let stored_size = if frame.compression == DNF_COMPRESS_NONE {
                frame.raw_size
            } else {
                usize::try_from(read_i32(bytes, frame.size_offset, "V2 帧数据长度")?)
                    .map_err(|_| "V2 帧数据长度无效".to_string())?
            };
            let data_end = data_cursor
                .checked_add(stored_size)
                .ok_or_else(|| "V2 帧数据区偏移溢出".to_string())?;
            if data_end > bytes.len() {
                return Err("V2 IMG 帧数据被截断".to_string());
            }
            frame.data_range = Some(data_cursor..data_end);
            data_cursor = data_end;
        }
        Ok(Self {
            data_start,
            data_end: data_cursor,
            frames: parsed,
        })
    }
}

fn bytes_per_pixel(format: i32) -> Result<usize, String> {
    match format {
        DNF_FMT_ARGB_1555 | DNF_FMT_ARGB_4444 => Ok(2),
        DNF_FMT_ARGB_8888 => Ok(4),
        _ => Err("V2 直色帧格式不受支持".to_string()),
    }
}

fn decompress_exact(stored: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    if stored.is_empty() {
        return Err("V2 直色帧解压数据为空".to_string());
    }
    if stored[0] == 0x78 {
        return decompress_standard_exact(stored, expected);
    }
    let Some(found) = stored.iter().rposition(|byte| *byte == 0x78) else {
        return Err("V2 直色帧解压失败：未找到 zlib 头".to_string());
    };
    if let Ok(raw) = decompress_reader_exact(
        Cursor::new(&stored[found..]).chain(Cursor::new(stored)),
        expected,
    ) {
        return Ok(raw);
    }
    let header_end = found
        .checked_add(2)
        .ok_or_else(|| "V2 直色帧解压输入长度溢出".to_string())?;
    let header = stored
        .get(found..header_end)
        .ok_or_else(|| "V2 直色帧 zlib 头被截断".to_string())?;
    decompress_reader_exact(Cursor::new(header).chain(Cursor::new(stored)), expected)
}

fn decompress_standard_exact(stored: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    decompress_reader_exact(Cursor::new(stored), expected)
}

fn decompress_reader_exact(reader: impl Read, expected: usize) -> Result<Vec<u8>, String> {
    let limit = expected
        .checked_add(1)
        .ok_or_else(|| "V2 直色帧解压长度溢出".to_string())?;
    let mut decoder = ZlibDecoder::new(reader).take(limit as u64);
    let mut raw = Vec::with_capacity(expected);
    decoder
        .read_to_end(&mut raw)
        .map_err(|error| format!("V2 直色帧解压失败：{error}"))?;
    if raw.len() != expected {
        return Err("V2 直色帧解压长度不一致".to_string());
    }
    Ok(raw)
}

fn decode_direct_pixel(format: i32, raw: &[u8]) -> Result<[u8; 4], String> {
    match format {
        DNF_FMT_ARGB_1555 => {
            let blue = raw[0] & 0x1f;
            let green = (raw[0] >> 5) | ((raw[1] & 0x03) << 3);
            let red = (raw[1] >> 2) & 0x1f;
            Ok([
                (red << 3) | (red >> 2),
                (green << 3) | (green >> 2),
                (blue << 3) | (blue >> 2),
                (raw[1] >> 7) * 0xff,
            ])
        }
        DNF_FMT_ARGB_4444 => Ok([
            (raw[1] & 0x0f) << 4,
            raw[0] & 0xf0,
            (raw[0] & 0x0f) << 4,
            raw[1] & 0xf0,
        ]),
        DNF_FMT_ARGB_8888 => Ok([raw[2], raw[1], raw[0], raw[3]]),
        _ => Err("V2 直色帧格式不受支持".to_string()),
    }
}

fn encode_direct_pixel(format: i32, rgba: [u8; 4], raw: &mut [u8]) -> Result<(), String> {
    match format {
        DNF_FMT_ARGB_1555 => {
            let blue = rgba[2] >> 3;
            let green = rgba[1] >> 3;
            let red = rgba[0] >> 3;
            let alpha = rgba[3] >> 7;
            raw[0] = ((green & 0x07) << 5) | blue;
            raw[1] = (alpha << 7) | (red << 2) | (green >> 3);
        }
        DNF_FMT_ARGB_4444 => {
            raw[0] = (rgba[1] & 0xf0) | (rgba[2] >> 4);
            raw[1] = (rgba[3] & 0xf0) | (rgba[0] >> 4);
        }
        DNF_FMT_ARGB_8888 => {
            raw.copy_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        _ => return Err("V2 直色帧格式不受支持".to_string()),
    }
    Ok(())
}

fn read_i32(bytes: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{label}数据被截断"))?;
    Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn resolve_effective_frame_indices(frames: &[FrameInfo]) -> Result<Vec<usize>, String> {
    let unresolved = usize::MAX;
    let mut effective = vec![unresolved; frames.len()];
    let mut visiting = vec![false; frames.len()];
    for start in 0..frames.len() {
        if effective[start] != unresolved {
            continue;
        }
        let mut path = Vec::new();
        let mut current = start;
        let resolved = loop {
            let frame = frames
                .get(current)
                .ok_or_else(|| "LINK 帧目标无效".to_string())?;
            if effective[current] != unresolved {
                break effective[current];
            }
            if visiting[current] {
                return Err("LINK 帧存在循环引用".to_string());
            }
            visiting[current] = true;
            path.push(current);
            if frame.format != DNF_FMT_LINK {
                break current;
            }
            current =
                usize::try_from(frame.link_index).map_err(|_| "LINK 帧目标无效".to_string())?;
        };
        for index in path.into_iter().rev() {
            effective[index] = resolved;
            visiting[index] = false;
        }
    }
    Ok(effective)
}

fn checked_pixel_count(frame: &FrameInfo) -> Result<usize, String> {
    let width = usize::try_from(frame.width).map_err(|_| "V2 帧宽度无效".to_string())?;
    let height = usize::try_from(frame.height).map_err(|_| "V2 帧高度无效".to_string())?;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| "V2 帧像素数量溢出".to_string())?;
    let decoded_rgba_bytes = pixels
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or_else(|| "V2 帧解码 RGBA 长度溢出".to_string())?;
    if pixels == 0 || decoded_rgba_bytes > MAX_DIRECT_FRAME_DECODED_RGBA_BYTES {
        return Err("V2 帧像素数量超过染色安全上限".to_string());
    }
    Ok(pixels)
}

fn checked_total_decoded_rgba_bytes(frames: &[FrameInfo]) -> Result<usize, String> {
    let mut total = 0_usize;
    for frame in frames.iter().filter(|frame| frame.format != DNF_FMT_LINK) {
        let decoded_rgba_bytes = checked_pixel_count(frame)?
            .checked_mul(RGBA_BYTES_PER_PIXEL)
            .ok_or_else(|| "V2 总解码 RGBA 长度溢出".to_string())?;
        total = total
            .checked_add(decoded_rgba_bytes)
            .ok_or_else(|| "V2 总解码 RGBA 长度溢出".to_string())?;
        if total > MAX_DIRECT_TOTAL_DECODED_RGBA_BYTES {
            return Err("V2 总解码 RGBA 长度超过染色安全上限".to_string());
        }
    }
    Ok(total)
}
