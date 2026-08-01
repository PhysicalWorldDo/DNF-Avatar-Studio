use std::collections::BTreeMap;
use std::ops::Range;

use sha2::{Digest, Sha256};

use crate::dnflib_safe::ImgArchive;

use super::direct_color::{
    build_patched_v2_direct_bytes, direct_source_fingerprint, DNF_COMPRESS_NONE, DNF_FMT_ARGB_1555,
    DNF_FMT_ARGB_4444, DNF_FMT_ARGB_8888, DNF_FMT_LINK,
};
use super::models::{DyeSourceFingerprintV1, DyeTargetRecipeV1};

const IMG_HEADER_BYTES: usize = 32;
const MAX_PALETTE_BOARDS: usize = 64;
const MAX_PALETTE_COLORS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteBoardLayout {
    pub count_offset: usize,
    pub rgba_offset: usize,
    pub color_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteLayout {
    pub img_version: u16,
    pub frame_count: u32,
    pub boards: Vec<PaletteBoardLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FramePayloadLayout {
    pub(crate) format: i32,
    pub(crate) compression: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) declared_size: i32,
    pub(crate) data_range: Range<usize>,
}

impl PaletteLayout {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        validate_img_magic(bytes)?;
        let version = read_i32(bytes, 24, "IMG 版本")?;
        let frame_count = read_i32(bytes, 28, "IMG 帧数")?;
        if !(0..=65_535).contains(&frame_count) {
            return Err("IMG 帧数无效".to_string());
        }

        let (board_count, mut cursor) = match version {
            4 => (1_usize, IMG_HEADER_BYTES),
            6 => {
                let count = read_i32(bytes, IMG_HEADER_BYTES, "Palette 数量")?;
                if count < 0 || count as usize > MAX_PALETTE_BOARDS {
                    return Err("Palette 数量无效".to_string());
                }
                (count as usize, IMG_HEADER_BYTES + 4)
            }
            5 => return Err("V5 IMG 暂不支持调色板染色".to_string()),
            _ => return Err("只有 V4/V6 IMG 支持调色板染色".to_string()),
        };

        let mut boards = Vec::with_capacity(board_count);
        for _ in 0..board_count {
            let count_offset = cursor;
            let count = read_i32(bytes, count_offset, "Palette 颜色数量")?;
            if count < 0 || count as usize > MAX_PALETTE_COLORS {
                return Err("Palette 颜色数量无效".to_string());
            }
            let color_count = count as usize;
            let rgba_offset = count_offset
                .checked_add(4)
                .ok_or_else(|| "Palette 偏移溢出".to_string())?;
            let color_bytes = color_count
                .checked_mul(4)
                .ok_or_else(|| "Palette 长度溢出".to_string())?;
            cursor = rgba_offset
                .checked_add(color_bytes)
                .ok_or_else(|| "Palette 长度溢出".to_string())?;
            if cursor > bytes.len() {
                return Err("IMG Palette 数据被截断".to_string());
            }
            boards.push(PaletteBoardLayout {
                count_offset,
                rgba_offset,
                color_count,
            });
        }

        Ok(Self {
            img_version: version as u16,
            frame_count: frame_count as u32,
            boards,
        })
    }

    pub(crate) fn frame_payloads(
        &self,
        bytes: &[u8],
    ) -> Result<Vec<Option<FramePayloadLayout>>, String> {
        validate_img_magic(bytes)?;
        if read_i32(bytes, 24, "IMG 版本")? != i32::from(self.img_version)
            || read_i32(bytes, 28, "IMG 帧数")? != self.frame_count as i32
        {
            return Err("IMG 帧头与 Palette 布局不一致".to_string());
        }

        let header_start = self
            .boards
            .last()
            .map(|board| board.rgba_offset + board.color_count * 4)
            .unwrap_or(IMG_HEADER_BYTES + 4);
        let mut cursor = header_start;
        let mut descriptors = Vec::with_capacity(self.frame_count as usize);
        for _ in 0..self.frame_count {
            let format = read_i32(bytes, cursor, "帧格式")?;
            cursor = checked_add(cursor, 4, "帧头偏移")?;
            if format == DNF_FMT_LINK {
                let _link_index = read_i32(bytes, cursor, "LINK 目标")?;
                cursor = checked_add(cursor, 4, "LINK 帧头偏移")?;
                descriptors.push(None);
                continue;
            }
            let compression = read_i32(bytes, cursor, "帧压缩格式")?;
            let width = read_i32(bytes, cursor + 4, "帧宽度")?;
            let height = read_i32(bytes, cursor + 8, "帧高度")?;
            let declared_size = read_i32(bytes, cursor + 12, "帧数据长度")?;
            if width <= 0 || height <= 0 || declared_size < 0 {
                return Err("IMG 帧元数据无效".to_string());
            }
            cursor = checked_add(cursor, 32, "帧头偏移")?;
            descriptors.push(Some(FramePayloadLayout {
                format,
                compression,
                width,
                height,
                declared_size,
                data_range: 0..0,
            }));
        }

        let images_size = if bytes.starts_with(b"Neople Img File\0") {
            let value = read_i32(bytes, 16, "IMG 头区长度")?;
            if value < 0 {
                return Err("IMG 头区长度无效".to_string());
            }
            value as usize
        } else {
            0
        };
        let mut data_cursor = if images_size == 0 {
            cursor
        } else {
            checked_add(header_start, images_size, "IMG 数据区偏移")?
        };
        if data_cursor < cursor || data_cursor > bytes.len() {
            return Err("IMG 数据区偏移无效".to_string());
        }

        for descriptor in descriptors.iter_mut().flatten() {
            let stored_size = stored_frame_size(descriptor)?;
            let data_end = checked_add(data_cursor, stored_size, "帧数据区偏移")?;
            if data_end > bytes.len() {
                return Err("IMG 帧数据被截断".to_string());
            }
            descriptor.data_range = data_cursor..data_end;
            data_cursor = data_end;
        }
        Ok(descriptors)
    }

    pub fn color_rgba(
        &self,
        bytes: &[u8],
        palette_index: usize,
        color_index: usize,
    ) -> Result<[u8; 4], String> {
        let board = self
            .boards
            .get(palette_index)
            .ok_or_else(|| "Palette 索引无效".to_string())?;
        if color_index >= board.color_count {
            return Err("颜色索引无效".to_string());
        }
        let offset = board
            .rgba_offset
            .checked_add(
                color_index
                    .checked_mul(4)
                    .ok_or_else(|| "颜色索引溢出".to_string())?,
            )
            .ok_or_else(|| "颜色索引溢出".to_string())?;
        let color = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| "颜色数据被截断".to_string())?;
        Ok([color[0], color[1], color[2], color[3]])
    }
}

impl DyeSourceFingerprintV1 {
    pub fn from_img_bytes(bytes: &[u8], palette_index: usize) -> Result<Self, String> {
        validate_img_magic(bytes)?;
        if read_i32(bytes, 24, "IMG 版本")? == 2 {
            return direct_source_fingerprint(bytes, palette_index);
        }
        let layout = PaletteLayout::parse(bytes)?;
        let entry_sha256 = validate_img_against_layout(bytes, &layout)?;
        Self::from_validated_layout(bytes, &layout, palette_index, entry_sha256)
    }

    pub(crate) fn from_validated_layout(
        bytes: &[u8],
        layout: &PaletteLayout,
        palette_index: usize,
        entry_sha256: [u8; 32],
    ) -> Result<Self, String> {
        let board = layout
            .boards
            .get(palette_index)
            .ok_or_else(|| "Palette 索引无效".to_string())?;

        let palette_end = board
            .rgba_offset
            .checked_add(board.color_count * 4)
            .ok_or_else(|| "Palette 长度溢出".to_string())?;
        let palette = bytes
            .get(board.count_offset..palette_end)
            .ok_or_else(|| "Palette 数据被截断".to_string())?;

        Ok(Self {
            img_version: layout.img_version,
            frame_count: layout.frame_count,
            palette_count: u16::try_from(layout.boards.len())
                .map_err(|_| "Palette 数量过多".to_string())?,
            color_count: u16::try_from(board.color_count)
                .map_err(|_| "Palette 颜色数量过多".to_string())?,
            palette_sha256: sha256(palette),
            entry_sha256,
        })
    }
}

pub fn build_patched_img_bytes(
    source: &[u8],
    target: &DyeTargetRecipeV1,
) -> Result<Vec<u8>, String> {
    build_patched_img_bytes_many(source, &[target])
}

pub(crate) fn build_patched_img_bytes_many(
    source: &[u8],
    targets: &[&DyeTargetRecipeV1],
) -> Result<Vec<u8>, String> {
    validate_img_magic(source)?;
    if read_i32(source, 24, "IMG 版本")? == 2 {
        return build_patched_v2_direct_bytes(source, targets);
    }
    let layout = PaletteLayout::parse(source)?;
    let entry_sha256 = validate_img_against_layout(source, &layout)?;
    let mut fingerprints = BTreeMap::<usize, DyeSourceFingerprintV1>::new();
    let mut unique = BTreeMap::<(usize, u16), ([u8; 4], [u8; 3])>::new();
    for target in targets {
        let palette_index = usize::from(target.palette_index);
        let current = if let Some(current) = fingerprints.get(&palette_index) {
            current
        } else {
            let current = DyeSourceFingerprintV1::from_validated_layout(
                source,
                &layout,
                palette_index,
                entry_sha256,
            )?;
            fingerprints.entry(palette_index).or_insert(current)
        };
        if current != &target.source {
            return Err("源资源已变化，染色配方不能应用".to_string());
        }

        let board = layout
            .boards
            .get(palette_index)
            .ok_or_else(|| "Palette 索引无效".to_string())?;
        for edit in &target.colors {
            if usize::from(edit.color_index) >= board.color_count {
                return Err("颜色索引无效".to_string());
            }
            let key = (palette_index, edit.color_index);
            if let Some(existing) = unique.insert(key, (edit.expected_rgba, edit.replacement_rgb)) {
                if existing != (edit.expected_rgba, edit.replacement_rgb) {
                    return Err("同一 IMG 和 Palette 存在冲突配方".to_string());
                }
            }
        }
    }

    let direct_replacements = v4_direct_argb1555_replacements(source, &layout, &unique)?;
    let mut patched = source.to_vec();
    for ((palette_index, color_index), (expected_rgba, replacement_rgb)) in &unique {
        let board = layout
            .boards
            .get(*palette_index)
            .ok_or_else(|| "Palette 索引无效".to_string())?;
        let actual = layout.color_rgba(source, *palette_index, usize::from(*color_index))?;
        if actual != *expected_rgba {
            return Err("Palette 原色不一致，源资源已变化".to_string());
        }
        if actual[3] == 0 && *replacement_rgb != actual[..3] {
            return Err("完全透明颜色不可修改".to_string());
        }
        if *replacement_rgb == actual[..3] {
            continue;
        }
        let offset = board.rgba_offset + usize::from(*color_index) * 4;
        patched[offset..offset + 3].copy_from_slice(replacement_rgb);
    }
    patch_v4_direct_argb1555_frames(source, &layout, &mut patched, &direct_replacements)?;

    if patched.len() != source.len() {
        return Err("补丁后 IMG 长度发生变化".to_string());
    }
    let reopened = ImgArchive::open_owned_bytes(patched.clone())?;
    if reopened.version() != i32::from(layout.img_version)
        || reopened.frames().len() != layout.frame_count as usize
        || reopened.palette_count() != layout.boards.len()
    {
        return Err("补丁后 IMG 结构校验失败".to_string());
    }
    Ok(patched)
}

fn validate_img_against_layout(bytes: &[u8], layout: &PaletteLayout) -> Result<[u8; 32], String> {
    let img = ImgArchive::open_owned_bytes(bytes.to_vec())?;
    if img.version() != i32::from(layout.img_version)
        || img.frames().len() != layout.frame_count as usize
        || img.palette_count() != layout.boards.len()
    {
        return Err("IMG 结构校验失败".to_string());
    }
    Ok(sha256(bytes))
}

fn v4_direct_argb1555_replacements(
    source: &[u8],
    layout: &PaletteLayout,
    edits: &BTreeMap<(usize, u16), ([u8; 4], [u8; 3])>,
) -> Result<BTreeMap<[u8; 2], [u8; 2]>, String> {
    if layout.img_version != 4 || edits.is_empty() {
        return Ok(BTreeMap::new());
    }
    if !layout
        .frame_payloads(source)?
        .iter()
        .flatten()
        .any(|frame| frame.format == DNF_FMT_ARGB_1555 && frame.compression == DNF_COMPRESS_NONE)
    {
        return Ok(BTreeMap::new());
    }
    let board = layout
        .boards
        .first()
        .ok_or_else(|| "V4 Palette 缺失".to_string())?;
    let mut replacements = BTreeMap::new();
    for color_index in 0..board.color_count {
        let actual = layout.color_rgba(source, 0, color_index)?;
        let replacement_rgb = edits
            .get(&(0, color_index as u16))
            .map(|(_, replacement)| *replacement)
            .unwrap_or([actual[0], actual[1], actual[2]]);
        let source_pixel = encode_argb1555(actual);
        let replacement_pixel = encode_argb1555([
            replacement_rgb[0],
            replacement_rgb[1],
            replacement_rgb[2],
            actual[3],
        ]);
        if let Some(prior) = replacements.insert(source_pixel, replacement_pixel) {
            if prior != replacement_pixel {
                return Err("V4 直色帧颜色量化后存在冲突，无法安全染色".to_string());
            }
        }
    }
    replacements.retain(|source_pixel, replacement_pixel| source_pixel != replacement_pixel);
    Ok(replacements)
}

fn patch_v4_direct_argb1555_frames(
    source: &[u8],
    layout: &PaletteLayout,
    patched: &mut [u8],
    replacements: &BTreeMap<[u8; 2], [u8; 2]>,
) -> Result<(), String> {
    if layout.img_version != 4 || replacements.is_empty() {
        return Ok(());
    }
    for frame in layout.frame_payloads(source)?.into_iter().flatten() {
        if frame.format != DNF_FMT_ARGB_1555 || frame.compression != DNF_COMPRESS_NONE {
            continue;
        }
        let pixels = patched
            .get_mut(frame.data_range)
            .ok_or_else(|| "V4 直色帧数据被截断".to_string())?;
        if pixels.len() % 2 != 0 {
            return Err("V4 ARGB1555 直色帧长度无效".to_string());
        }
        for pixel in pixels.chunks_exact_mut(2) {
            if let Some(replacement) = replacements.get(&[pixel[0], pixel[1]]) {
                pixel.copy_from_slice(replacement);
            }
        }
    }
    Ok(())
}

fn encode_argb1555(rgba: [u8; 4]) -> [u8; 2] {
    let [r, g, b, a] = rgba;
    let (rv, gv, bv, av) = (r >> 3, g >> 3, b >> 3, a >> 7);
    [((gv & 0x07) << 5) | bv, (av << 7) | (rv << 2) | (gv >> 3)]
}

fn validate_img_magic(bytes: &[u8]) -> Result<(), String> {
    let new_magic = b"Neople Img File\0";
    let old_magic = b"Neople Image File\0";
    if bytes.len() < IMG_HEADER_BYTES
        || !(bytes.starts_with(new_magic) || bytes.starts_with(old_magic))
    {
        return Err("IMG magic 无效或文件被截断".to_string());
    }
    Ok(())
}

fn read_i32(bytes: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{label}数据被截断"))?;
    Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn stored_frame_size(frame: &FramePayloadLayout) -> Result<usize, String> {
    if frame.compression != DNF_COMPRESS_NONE {
        return usize::try_from(frame.declared_size).map_err(|_| "帧数据长度无效".to_string());
    }
    let width = usize::try_from(frame.width).map_err(|_| "帧宽度无效".to_string())?;
    let height = usize::try_from(frame.height).map_err(|_| "帧高度无效".to_string())?;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| "帧像素数量溢出".to_string())?;
    let bytes_per_pixel = match frame.format {
        DNF_FMT_ARGB_1555 | DNF_FMT_ARGB_4444 => 2,
        DNF_FMT_ARGB_8888 => 4,
        _ => 2,
    };
    pixels
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| "帧数据长度溢出".to_string())
}

fn checked_add(left: usize, right: usize, label: &str) -> Result<usize, String> {
    left.checked_add(right)
        .ok_or_else(|| format!("{label}溢出"))
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
