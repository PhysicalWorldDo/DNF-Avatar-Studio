use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::avatar_data::{
    find_avatar_items_by_ids_from_data_library, load_layers_from_data_library,
    open_catalog_index_from_data_library, query_avatar_items_from_data_library,
};
use crate::avatar_dye::PaletteLayout;
use crate::dnflib_safe::{ImgArchive, NpkArchive};
use crate::game_resolver::{
    resolve_game_npk_path, resolve_img_resource, same_named_mask_number, ImgResourceMetadata,
};
use crate::models::{AvatarItem, AvatarLayer, CatalogResponse, CatalogStats};

pub const RESOURCE_PROBE_SCHEMA_VERSION: u32 = 1;

const DNF_FMT_LINK: i32 = 0x11;
const DNF_FMT_DXT_1: i32 = 0x12;
const DNF_FMT_DXT_3: i32 = 0x13;
const DNF_FMT_DXT_5: i32 = 0x14;
const DNF_COMPRESS_NONE: i32 = 0x05;
const DNF_COMPRESS_ZLIB: i32 = 0x06;
const DNF_COMPRESS_DDS_ZLIB: i32 = 0x07;
const MAX_PROBE_NPK_CACHE_BYTES: usize = 512 * 1024 * 1024;
const MAX_PROBE_NPK_ENTRY_COUNT: usize = 100_000;
const MAX_PROBE_IMG_ENTRY_BYTES: usize = 32 * 1024 * 1024;
const MAX_PROBE_COMPRESSED_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROBE_FRAME_COUNT: usize = 4_096;
const MAX_PROBE_PALETTE_COUNT: usize = 256;
const MAX_PROBE_FRAME_RGBA_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROBE_TOTAL_DECODED_RGBA_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProbeResourceLimits {
    pub(crate) max_single_npk_bytes: usize,
    pub(crate) max_cached_npk_bytes: usize,
    pub(crate) max_npk_entry_count: usize,
    pub(crate) max_img_entry_bytes: usize,
    pub(crate) max_compressed_frame_bytes: usize,
    pub(crate) max_frame_count: usize,
    pub(crate) max_palette_count: usize,
    pub(crate) max_frame_rgba_bytes: usize,
    pub(crate) max_total_decoded_rgba_bytes: usize,
}

impl Default for ProbeResourceLimits {
    fn default() -> Self {
        Self {
            max_single_npk_bytes: MAX_PROBE_NPK_CACHE_BYTES,
            max_cached_npk_bytes: MAX_PROBE_NPK_CACHE_BYTES,
            max_npk_entry_count: MAX_PROBE_NPK_ENTRY_COUNT,
            max_img_entry_bytes: MAX_PROBE_IMG_ENTRY_BYTES,
            max_compressed_frame_bytes: MAX_PROBE_COMPRESSED_FRAME_BYTES,
            max_frame_count: MAX_PROBE_FRAME_COUNT,
            max_palette_count: MAX_PROBE_PALETTE_COUNT,
            max_frame_rgba_bytes: MAX_PROBE_FRAME_RGBA_BYTES,
            max_total_decoded_rgba_bytes: MAX_PROBE_TOTAL_DECODED_RGBA_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceProbeCliOptions {
    pub queries: Vec<String>,
    pub data_dll: Option<PathBuf>,
    pub game_dir: Option<PathBuf>,
    pub frame_index: usize,
    pub limit: usize,
    pub inspect_npk: bool,
    pub help: bool,
}

pub fn parse_resource_probe_args<I, S>(args: I) -> Result<ResourceProbeCliOptions, String>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let mut args = args.into_iter().map(|arg| arg.into()).skip(1);
    let mut options = ResourceProbeCliOptions {
        queries: Vec::new(),
        data_dll: None,
        game_dir: None,
        frame_index: 0,
        limit: 20,
        inspect_npk: true,
        help: false,
    };
    while let Some(raw) = args.next() {
        let arg = raw
            .into_string()
            .map_err(|_| "命令行参数不是有效 Unicode".to_string())?;
        match arg.as_str() {
            "-h" | "--help" => options.help = true,
            "--no-npk" => options.inspect_npk = false,
            "--data-dll" => {
                options.data_dll = Some(PathBuf::from(required_cli_value(&mut args, &arg)?));
            }
            "--game-dir" => {
                options.game_dir = Some(PathBuf::from(required_cli_value(&mut args, &arg)?));
            }
            "--frame" => {
                options.frame_index = required_cli_value(&mut args, &arg)?
                    .parse::<usize>()
                    .map_err(|_| "--frame 必须是非负整数".to_string())?;
            }
            "--limit" => {
                options.limit = required_cli_value(&mut args, &arg)?
                    .parse::<usize>()
                    .map_err(|_| "--limit 必须是 1 到 100 之间的整数".to_string())?;
                if !(1..=100).contains(&options.limit) {
                    return Err("--limit 必须是 1 到 100 之间的整数".to_string());
                }
            }
            _ if arg.starts_with('-') => return Err(format!("未知参数：{arg}")),
            _ => options.queries.push(arg),
        }
    }
    if !options.help && options.queries.is_empty() {
        return Err("至少需要一个物品 ID 或名称；使用 --help 查看用法".to_string());
    }
    Ok(options)
}

fn required_cli_value<I>(args: &mut I, option: &str) -> Result<String, String>
where
    I: Iterator<Item = std::ffi::OsString>,
{
    args.next()
        .ok_or_else(|| format!("{option} 缺少参数"))?
        .into_string()
        .map_err(|_| format!("{option} 参数不是有效 Unicode"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImgEntryProbe {
    pub entry_path: String,
    pub version: i32,
    pub frame_count: usize,
    pub palette_count: usize,
    pub requested_palette: usize,
    pub effective_palette: usize,
    pub effective_frame: Option<usize>,
    pub effective_frame_sha256: Option<String>,
    pub palette_frame_sha256: Vec<String>,
    pub decode_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaskEntryProbe {
    pub number: u32,
    pub authorized: bool,
    pub exists: bool,
    pub default_active: bool,
    pub entry: Option<ImgEntryProbe>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerResourceProbe {
    pub requested_img: String,
    pub resolved_base_entry: Option<String>,
    pub npk_path: Option<String>,
    pub base: Option<ImgEntryProbe>,
    pub masks: Vec<MaskEntryProbe>,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AvatarLayerProbe {
    pub layer_order: i32,
    pub equipment_type_enum: u16,
    pub layer_sequence: u32,
    pub layer_tag: String,
    pub variation: String,
    pub palette_index: usize,
    pub visual_img: String,
    pub status: String,
    pub resources: Option<LayerResourceProbe>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AvatarItemProbe {
    pub item_id: String,
    pub item_name: String,
    pub slot: String,
    pub profession_dir: String,
    pub path_family: String,
    pub resource_kind: String,
    pub enable_dye_mask1: bool,
    pub enable_dye_mask2: bool,
    pub layers: Vec<AvatarLayerProbe>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AvatarResourceProbeReport {
    pub schema_version: u32,
    pub queries: Vec<String>,
    pub game_dir: Option<String>,
    pub catalog_stats: CatalogStats,
    pub items: Vec<AvatarItemProbe>,
}

pub fn find_catalog_items<'a>(
    catalog: &'a CatalogResponse,
    queries: &[String],
    limit: usize,
) -> Vec<&'a AvatarItem> {
    let mut matches = Vec::new();
    let mut selected = HashSet::<&str>::new();
    let limit = limit.max(1);
    for raw_query in queries {
        let query = raw_query.trim();
        if query.is_empty() {
            continue;
        }
        let mut candidates = catalog
            .items
            .iter()
            .filter(|item| item.item_id == query)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            candidates = catalog
                .items
                .iter()
                .filter(|item| item.item_name.eq_ignore_ascii_case(query))
                .collect();
        }
        if candidates.is_empty() {
            let folded = query.to_lowercase();
            candidates = catalog
                .items
                .iter()
                .filter(|item| item.item_name.to_lowercase().contains(&folded))
                .collect();
        }
        for item in candidates {
            if selected.insert(item.item_id.as_str()) {
                matches.push(item);
                if matches.len() >= limit {
                    return matches;
                }
            }
        }
    }
    matches
}

fn find_catalog_items_from_data_library(
    queries: &[String],
    limit: usize,
) -> Result<Vec<AvatarItem>, String> {
    let mut matches = Vec::new();
    let mut selected = HashSet::<String>::new();
    let limit = limit.max(1);
    for raw_query in queries {
        let query = raw_query.trim();
        if query.is_empty() {
            continue;
        }
        let exact_id = find_avatar_items_by_ids_from_data_library(&[query.to_string()])?;
        if !exact_id.is_empty() {
            for item in exact_id {
                if selected.insert(item.item_id.clone()) {
                    matches.push(item);
                    if matches.len() >= limit {
                        return Ok(matches);
                    }
                }
            }
            continue;
        }

        let candidates = query_avatar_items_from_data_library("all", "all", query)?;
        let exact_name = candidates
            .iter()
            .filter(|item| item.item_name.eq_ignore_ascii_case(query))
            .cloned()
            .collect::<Vec<_>>();
        let candidates = if exact_name.is_empty() {
            candidates
        } else {
            exact_name
        };
        for item in candidates {
            if selected.insert(item.item_id.clone()) {
                matches.push(item);
                if matches.len() >= limit {
                    return Ok(matches);
                }
            }
        }
    }
    Ok(matches)
}

pub fn probe_avatar_resources(
    queries: &[String],
    game_dir: Option<&Path>,
    frame_index: usize,
    limit: usize,
) -> Result<AvatarResourceProbeReport, String> {
    if !queries.iter().any(|query| !query.trim().is_empty()) {
        return Err("至少需要一个物品 ID 或名称".to_string());
    }
    if let Some(game_dir) = game_dir {
        validate_game_dir(game_dir)?;
    }
    let metadata = open_catalog_index_from_data_library()?;
    let matches = find_catalog_items_from_data_library(queries, limit)?;
    let item_ids = matches
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<Vec<_>>();
    let layers = load_layers_from_data_library(&item_ids)?;
    let mut layers_by_item = HashMap::<&str, Vec<&AvatarLayer>>::new();
    for layer in &layers {
        layers_by_item
            .entry(layer.item_id.as_str())
            .or_default()
            .push(layer);
    }
    let mut inspector =
        game_dir.map(|game_dir| ResourceInspector::new(game_dir, ProbeResourceLimits::default()));
    let items = matches
        .into_iter()
        .map(|item| {
            let item_layers = layers_by_item
                .remove(item.item_id.as_str())
                .unwrap_or_default()
                .into_iter()
                .map(|layer| {
                    let resources = inspector
                        .as_mut()
                        .map(|inspector| inspector.inspect_layer(layer, frame_index));
                    AvatarLayerProbe {
                        layer_order: layer.layer_order,
                        equipment_type_enum: layer.equipment_type_enum,
                        layer_sequence: layer.layer_sequence,
                        layer_tag: layer.layer_tag.clone(),
                        variation: layer.variation.clone(),
                        palette_index: layer.palette_index,
                        visual_img: layer.visual_img.clone(),
                        status: layer.status.clone(),
                        resources,
                    }
                })
                .collect::<Vec<_>>();
            AvatarItemProbe {
                item_id: item.item_id.clone(),
                item_name: item.item_name.clone(),
                slot: item.slot.clone(),
                profession_dir: item.profession_dir.clone(),
                path_family: item.path_family.clone(),
                resource_kind: item.resource_kind.clone(),
                enable_dye_mask1: item.enable_dye_mask1,
                enable_dye_mask2: item.enable_dye_mask2,
                layers: item_layers,
            }
        })
        .collect::<Vec<_>>();

    Ok(AvatarResourceProbeReport {
        schema_version: RESOURCE_PROBE_SCHEMA_VERSION,
        queries: queries.to_vec(),
        game_dir: game_dir.map(|path| path.to_string_lossy().into_owned()),
        catalog_stats: metadata.stats,
        items,
    })
}

pub fn inspect_layer_resources(
    game_dir: &Path,
    layer: &AvatarLayer,
    frame_index: usize,
) -> Result<LayerResourceProbe, String> {
    validate_game_dir(game_dir)?;
    Ok(
        ResourceInspector::new(game_dir, ProbeResourceLimits::default())
            .inspect_layer(layer, frame_index),
    )
}

fn validate_game_dir(game_dir: &Path) -> Result<(), String> {
    if game_dir.is_dir() {
        Ok(())
    } else {
        Err(format!("NPK 目录不存在或不是目录：{}", game_dir.display()))
    }
}

struct CachedNpk {
    archive: Rc<NpkArchive>,
    file_bytes: usize,
}

struct ResourceInspector<'a> {
    game_dir: &'a Path,
    archives: HashMap<PathBuf, CachedNpk>,
    archive_order: VecDeque<PathBuf>,
    cached_npk_bytes: usize,
    decoded_rgba_bytes: usize,
    limits: ProbeResourceLimits,
}

impl<'a> ResourceInspector<'a> {
    fn new(game_dir: &'a Path, limits: ProbeResourceLimits) -> Self {
        Self {
            game_dir,
            archives: HashMap::new(),
            archive_order: VecDeque::new(),
            cached_npk_bytes: 0,
            decoded_rgba_bytes: 0,
            limits,
        }
    }

    fn inspect_layer(&mut self, layer: &AvatarLayer, frame_index: usize) -> LayerResourceProbe {
        let mut report = LayerResourceProbe {
            requested_img: layer.visual_img.clone(),
            resolved_base_entry: None,
            npk_path: None,
            base: None,
            masks: (1..=2)
                .map(|number| MaskEntryProbe {
                    number,
                    authorized: layer.enable_dye_mask(number),
                    exists: false,
                    default_active: false,
                    entry: None,
                })
                .collect(),
            notes: Vec::new(),
            errors: Vec::new(),
        };
        if report.masks.iter().any(|mask| mask.authorized) {
            report.notes.push(
                "[enable dye] 只表示通道已授权；没有非默认实际染色值时，客户端不会默认激活 mask"
                    .to_string(),
            );
        }
        if layer.visual_img.trim().is_empty() {
            report.notes.push("图层没有 visual IMG".to_string());
            return report;
        }

        let resolved = resolve_img_resource(
            &layer.visual_img,
            layer.palette_index,
            |entry_path| -> Result<_, String> {
                let Some(npk_path) = resolve_game_npk_path(self.game_dir, &entry_path) else {
                    return Ok(None);
                };
                if !npk_path.is_file() {
                    return Ok(None);
                }
                let archive = match self.archive_for(&npk_path) {
                    Ok(archive) => archive,
                    Err(error) => {
                        report.errors.push(error);
                        return Ok(None);
                    }
                };
                let Some(entry_index) = archive.find_entry_index(entry_path) else {
                    return Ok(None);
                };
                let bytes = archive
                    .entry_bytes(entry_index)
                    .map_err(|error| format!("读取 IMG 入口失败 {entry_path}：{error}"))?;
                if bytes.len() > self.limits.max_img_entry_bytes {
                    return Err(format!(
                        "IMG 入口字节数超过排查上限（{} > {} bytes）：{entry_path}",
                        bytes.len(),
                        self.limits.max_img_entry_bytes
                    ));
                }
                let preflight = preflight_img_entry(bytes, self.limits)
                    .map_err(|error| format!("IMG 安全预检失败 {entry_path}：{error}"))?;
                let metadata = ImgResourceMetadata {
                    version: preflight.version,
                    palette_count: preflight.palette_count,
                };
                Ok(Some(((npk_path, archive), metadata)))
            },
        );
        let resolved = match resolved {
            Ok(Some(resolved)) => resolved,
            Ok(None) => {
                report
                    .notes
                    .push("未在给定 NPK 目录解析到主 IMG".to_string());
                return report;
            }
            Err(error) => {
                report.errors.push(error);
                return report;
            }
        };
        if resolved.palette_fallback {
            report.notes.push(format!(
                "逻辑皮肤 IMG 不存在，已使用 V6 调色板载体 Palette {}",
                resolved.palette_index
            ));
        }
        let base_entry = resolved.entry_path;
        let (npk_path, archive) = resolved.value;
        report.resolved_base_entry = Some(base_entry.clone());
        report.npk_path = Some(npk_path.to_string_lossy().into_owned());
        match inspect_entry(
            &archive,
            &base_entry,
            resolved.palette_index,
            frame_index,
            &mut self.decoded_rgba_bytes,
            self.limits,
        ) {
            Ok(base) => report.base = Some(base),
            Err(error) => report.errors.push(format!("主 IMG {base_entry}：{error}")),
        }

        let entries = archive.entries();
        for mask in &mut report.masks {
            let matching = entries
                .iter()
                .filter(|entry| {
                    same_named_mask_number(&base_entry, &entry.name) == Some(mask.number)
                })
                .collect::<Vec<_>>();
            if matching.len() > 1 {
                report.errors.push(format!(
                    "主 IMG 存在重复 mask{} 入口：{}",
                    mask.number, base_entry
                ));
                continue;
            }
            let Some(entry) = matching.first() else {
                continue;
            };
            mask.exists = true;
            match inspect_entry(
                &archive,
                &entry.name,
                0,
                frame_index,
                &mut self.decoded_rgba_bytes,
                self.limits,
            ) {
                Ok(entry_probe) => mask.entry = Some(entry_probe),
                Err(error) => report
                    .errors
                    .push(format!("mask{} {}：{error}", mask.number, entry.name)),
            }
        }
        if report
            .base
            .as_ref()
            .is_some_and(|base| base.palette_count > 1)
            && report.masks.iter().any(|mask| {
                mask.authorized
                    && mask
                        .entry
                        .as_ref()
                        .is_some_and(|entry| entry.palette_count <= 1)
            })
        {
            report.notes.push(
                "主 IMG 有多个调色板而授权 mask 只有一个；若把授权误当默认激活并直接叠加，会覆盖颜色变体"
                    .to_string(),
            );
        }
        report
    }

    fn archive_for(&mut self, npk_path: &Path) -> Result<Rc<NpkArchive>, String> {
        if self.archives.contains_key(npk_path) {
            self.touch_archive(npk_path);
            return self
                .archives
                .get(npk_path)
                .map(|cached| Rc::clone(&cached.archive))
                .ok_or_else(|| format!("NPK 缓存丢失：{}", npk_path.display()));
        }

        let file_bytes = std::fs::metadata(npk_path)
            .map_err(|error| format!("读取 NPK 元数据失败 {}：{error}", npk_path.display()))?
            .len();
        let file_bytes = usize::try_from(file_bytes)
            .map_err(|_| format!("NPK 文件大小超出平台范围：{}", npk_path.display()))?;
        if file_bytes > self.limits.max_single_npk_bytes {
            return Err(format!(
                "NPK 文件超过单文件缓存上限（{} > {} bytes）：{}",
                file_bytes,
                self.limits.max_single_npk_bytes,
                npk_path.display()
            ));
        }
        if file_bytes > self.limits.max_cached_npk_bytes {
            return Err(format!(
                "NPK 文件超过总缓存上限（{} > {} bytes）：{}",
                file_bytes,
                self.limits.max_cached_npk_bytes,
                npk_path.display()
            ));
        }
        while self
            .cached_npk_bytes
            .checked_add(file_bytes)
            .is_none_or(|total| total > self.limits.max_cached_npk_bytes)
        {
            let Some(oldest) = self.archive_order.pop_front() else {
                return Err("NPK 缓存预算无法容纳当前文件".to_string());
            };
            if let Some(evicted) = self.archives.remove(&oldest) {
                self.cached_npk_bytes = self.cached_npk_bytes.saturating_sub(evicted.file_bytes);
            }
        }

        let archive = Rc::new(
            NpkArchive::open_path(npk_path)
                .map_err(|error| format!("打开 NPK 失败 {}：{error}", npk_path.display()))?,
        );
        if archive.entry_count() > self.limits.max_npk_entry_count {
            return Err(format!(
                "NPK 入口数超过排查上限（{} > {}）：{}",
                archive.entry_count(),
                self.limits.max_npk_entry_count,
                npk_path.display()
            ));
        }
        self.archives.insert(
            npk_path.to_path_buf(),
            CachedNpk {
                archive: Rc::clone(&archive),
                file_bytes,
            },
        );
        self.archive_order.push_back(npk_path.to_path_buf());
        self.cached_npk_bytes += file_bytes;
        Ok(archive)
    }

    fn touch_archive(&mut self, npk_path: &Path) {
        if let Some(index) = self
            .archive_order
            .iter()
            .position(|cached_path| cached_path == npk_path)
        {
            self.archive_order.remove(index);
        }
        self.archive_order.push_back(npk_path.to_path_buf());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreflightImgLayout {
    version: i32,
    palette_count: usize,
    frames: Vec<PreflightFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreflightFrame {
    Link(usize),
    Payload(PreflightFramePayload),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreflightFramePayload {
    format: i32,
    compression: i32,
    width: i32,
    height: i32,
    stored_bytes: usize,
}

impl PreflightImgLayout {
    fn effective_frame_index(&self, index: usize) -> Result<usize, String> {
        let mut current = index;
        for _ in 0..=self.frames.len() {
            match self
                .frames
                .get(current)
                .ok_or_else(|| "IMG LINK 帧目标无效".to_string())?
            {
                PreflightFrame::Link(target) => current = *target,
                PreflightFrame::Payload(_) => return Ok(current),
            }
        }
        Err("IMG LINK 帧存在循环引用".to_string())
    }
}

fn inspect_entry(
    archive: &NpkArchive,
    entry_path: &str,
    requested_palette: usize,
    frame_index: usize,
    decoded_rgba_bytes: &mut usize,
    limits: ProbeResourceLimits,
) -> Result<ImgEntryProbe, String> {
    let entry_index = archive
        .find_entry_index(entry_path)
        .ok_or_else(|| format!("NPK 缺少 IMG 入口：{entry_path}"))?;
    let source_bytes = archive.entry_bytes(entry_index)?;
    if source_bytes.len() > limits.max_img_entry_bytes {
        return Err(format!(
            "IMG 入口字节数超过排查上限（{} > {} bytes）",
            source_bytes.len(),
            limits.max_img_entry_bytes
        ));
    }
    let preflight = preflight_img_entry(source_bytes, limits)
        .map_err(|error| format!("IMG 安全预检失败：{error}"))?;
    let img = ImgArchive::open_owned_bytes(source_bytes.to_vec())?;
    if img.version() != preflight.version || img.frame_count() != preflight.frames.len() {
        return Err("IMG native 元数据与安全预检结果不一致".to_string());
    }
    let native_palette_count = img.palette_count();
    if preflight.version != 2 && native_palette_count != preflight.palette_count {
        return Err("IMG native Palette 数与安全预检结果不一致".to_string());
    }
    let frame_count = preflight.frames.len();
    let palette_count = preflight.palette_count;
    let effective_palette = if matches!(preflight.version, 2 | 4) {
        0
    } else {
        requested_palette
    };
    let effective_frame = (frame_count > 0).then_some(frame_index % frame_count);
    let Some(frame) = effective_frame else {
        return Ok(ImgEntryProbe {
            entry_path: entry_path.to_string(),
            version: preflight.version,
            frame_count,
            palette_count,
            requested_palette,
            effective_palette,
            effective_frame,
            effective_frame_sha256: None,
            palette_frame_sha256: Vec::new(),
            decode_error: Some("IMG 没有帧".to_string()),
        });
    };
    let decode_frame = preflight.effective_frame_index(frame)?;
    let payload = match preflight.frames.get(decode_frame) {
        Some(PreflightFrame::Payload(payload)) => payload,
        _ => return Err("IMG 安全预检没有解析到有效物理帧".to_string()),
    };
    let frames = img.frames();
    let native_frame = frames
        .get(decode_frame)
        .ok_or_else(|| "IMG native 物理帧丢失".to_string())?;
    let native_stored_bytes = usize::try_from(
        img.frame_data_size(decode_frame)
            .ok_or_else(|| "IMG native 物理帧数据长度丢失".to_string())?,
    )
    .map_err(|_| "IMG native 帧数据长度无效".to_string())?;
    if native_frame.format != payload.format
        || native_frame.compress != payload.compression
        || native_frame.width != payload.width
        || native_frame.height != payload.height
        || native_stored_bytes != payload.stored_bytes
    {
        return Err("IMG native 帧元数据与安全预检结果不一致".to_string());
    }
    if matches!(
        payload.compression,
        DNF_COMPRESS_ZLIB | DNF_COMPRESS_DDS_ZLIB
    ) && native_stored_bytes > limits.max_compressed_frame_bytes
    {
        return Err(format!(
            "压缩帧数据超过排查上限（{native_stored_bytes} > {} bytes）",
            limits.max_compressed_frame_bytes
        ));
    }
    let frame_rgba_bytes = checked_frame_rgba_dimensions(payload.width, payload.height)?;
    let palette_variants = palette_count.max(1);
    let effective_palette_in_range = effective_palette < palette_variants;
    if matches!(
        payload.format,
        DNF_FMT_DXT_1 | DNF_FMT_DXT_3 | DNF_FMT_DXT_5
    ) {
        let reason = "probe 为避免未经界限验证的 DDS 内部尺寸，不执行 DXT native 解码";
        return Ok(ImgEntryProbe {
            entry_path: entry_path.to_string(),
            version: preflight.version,
            frame_count,
            palette_count,
            requested_palette,
            effective_palette,
            effective_frame,
            effective_frame_sha256: None,
            palette_frame_sha256: vec![format!("ERROR:{reason}"); palette_variants],
            decode_error: Some(reason.to_string()),
        });
    }
    if frame_rgba_bytes > limits.max_frame_rgba_bytes {
        return Err(format!(
            "单帧解码大小超过排查上限（{frame_rgba_bytes} > {} bytes）",
            limits.max_frame_rgba_bytes
        ));
    }
    let mut decode_palettes = (0..palette_variants).collect::<BTreeSet<_>>();
    if effective_palette_in_range {
        decode_palettes.insert(effective_palette);
    }
    let planned_decode_bytes = frame_rgba_bytes
        .checked_mul(decode_palettes.len())
        .ok_or_else(|| "IMG 解码预算计算溢出".to_string())?;
    let next_decoded_bytes = decoded_rgba_bytes
        .checked_add(planned_decode_bytes)
        .ok_or_else(|| "IMG 累计解码预算计算溢出".to_string())?;
    if next_decoded_bytes > limits.max_total_decoded_rgba_bytes {
        return Err(format!(
            "累计解码量超过排查上限（{next_decoded_bytes} > {} bytes）",
            limits.max_total_decoded_rgba_bytes
        ));
    }
    *decoded_rgba_bytes = next_decoded_bytes;

    let mut decoded_hashes = BTreeMap::<usize, Result<String, String>>::new();
    for palette in decode_palettes {
        let decoded = img
            .decode_frame_with_palette(decode_frame, palette)
            .and_then(|decoded| {
                if decoded.rgba.len() > limits.max_frame_rgba_bytes {
                    return Err(format!(
                        "实际单帧解码大小超过排查上限（{} > {} bytes）",
                        decoded.rgba.len(),
                        limits.max_frame_rgba_bytes
                    ));
                }
                Ok(sha256_hex(&decoded.rgba))
            });
        decoded_hashes.insert(palette, decoded);
    }
    let palette_frame_sha256 = (0..palette_variants)
        .map(|palette| match decoded_hashes.get(&palette) {
            Some(Ok(hash)) => hash.clone(),
            Some(Err(error)) => format!("ERROR:{error}"),
            None => "ERROR:未执行解码".to_string(),
        })
        .collect();
    let (effective_frame_sha256, decode_error) = if effective_palette_in_range {
        match decoded_hashes.get(&effective_palette) {
            Some(Ok(hash)) => (Some(hash.clone()), None),
            Some(Err(error)) => (None, Some(error.clone())),
            None => (None, Some("未执行有效 Palette 解码".to_string())),
        }
    } else {
        (
            None,
            Some(format!(
                "请求的 Palette {effective_palette} 超出有效范围 0..{palette_variants}"
            )),
        )
    };
    Ok(ImgEntryProbe {
        entry_path: entry_path.to_string(),
        version: preflight.version,
        frame_count,
        palette_count,
        requested_palette,
        effective_palette,
        effective_frame,
        effective_frame_sha256,
        palette_frame_sha256,
        decode_error,
    })
}

fn checked_frame_rgba_dimensions(width: i32, height: i32) -> Result<usize, String> {
    let width = usize::try_from(width)
        .ok()
        .filter(|width| *width > 0)
        .ok_or_else(|| "IMG 帧宽度无效".to_string())?;
    let height = usize::try_from(height)
        .ok()
        .filter(|height| *height > 0)
        .ok_or_else(|| "IMG 帧高度无效".to_string())?;
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "IMG 帧 RGBA 大小溢出".to_string())
}

fn preflight_img_entry(
    bytes: &[u8],
    limits: ProbeResourceLimits,
) -> Result<PreflightImgLayout, String> {
    if bytes.len() < 32 {
        return Err("IMG 头被截断".to_string());
    }
    let new_magic = bytes.starts_with(b"Neople Img File\0");
    let old_magic = bytes.starts_with(b"Neople Image File\0");
    if !new_magic && !old_magic {
        return Err("IMG magic 无效".to_string());
    }
    let version = probe_read_i32(bytes, 24, "IMG 版本")?;
    let frame_count = probe_read_i32(bytes, 28, "IMG 帧数")?;
    let frame_count = usize::try_from(frame_count).map_err(|_| "IMG 帧数无效".to_string())?;
    if frame_count > limits.max_frame_count {
        return Err(format!(
            "IMG 帧数超过排查上限（{frame_count} > {}）",
            limits.max_frame_count
        ));
    }
    let images_size = if new_magic {
        let value = probe_read_i32(bytes, 16, "IMG 头区长度")?;
        usize::try_from(value).map_err(|_| "IMG 头区长度无效".to_string())?
    } else {
        0
    };
    let (palette_count, frame_header_start) = match version {
        2 => (1, 32),
        4 | 6 => {
            let layout = PaletteLayout::parse(bytes)?;
            if usize::try_from(layout.frame_count).ok() != Some(frame_count) {
                return Err("IMG Palette 布局帧数不一致".to_string());
            }
            if layout.boards.len() > limits.max_palette_count {
                return Err(format!(
                    "IMG Palette 数超过排查上限（{} > {}）",
                    layout.boards.len(),
                    limits.max_palette_count
                ));
            }
            let frame_header_start = layout
                .boards
                .last()
                .map(|board| board.rgba_offset + board.color_count * 4)
                .unwrap_or(36);
            (layout.boards.len(), frame_header_start)
        }
        1 | 5 => {
            return Err(format!(
                "V{version} IMG 的 native 结构无法在解码前完整验证，probe 拒绝打开"
            ));
        }
        _ => return Err(format!("probe 不支持 V{version} IMG")),
    };

    let mut cursor = frame_header_start;
    let mut frames = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let format = probe_read_i32(bytes, cursor, "IMG 帧格式")?;
        cursor = probe_checked_add(cursor, 4, "IMG 帧头偏移")?;
        if format == DNF_FMT_LINK {
            let target = probe_read_i32(bytes, cursor, "IMG LINK 目标")?;
            cursor = probe_checked_add(cursor, 4, "IMG LINK 帧头偏移")?;
            let target = usize::try_from(target).map_err(|_| "IMG LINK 帧目标无效".to_string())?;
            if target >= frame_count {
                return Err("IMG LINK 帧目标无效".to_string());
            }
            frames.push(PreflightFrame::Link(target));
            continue;
        }

        let compression = probe_read_i32(bytes, cursor, "IMG 帧压缩格式")?;
        let width = probe_read_i32(bytes, cursor + 4, "IMG 帧宽度")?;
        let height = probe_read_i32(bytes, cursor + 8, "IMG 帧高度")?;
        let declared_size = probe_read_i32(bytes, cursor + 12, "IMG 帧数据长度")?;
        cursor = probe_checked_add(cursor, 32, "IMG 帧头偏移")?;
        let frame_rgba_bytes = checked_frame_rgba_dimensions(width, height)?;
        if frame_rgba_bytes > limits.max_frame_rgba_bytes {
            return Err(format!(
                "单帧解码大小超过排查上限（{frame_rgba_bytes} > {} bytes）",
                limits.max_frame_rgba_bytes
            ));
        }
        let declared_size =
            usize::try_from(declared_size).map_err(|_| "IMG 帧数据长度无效".to_string())?;
        if compression == DNF_COMPRESS_DDS_ZLIB {
            return Err("V2/V4/V6 DDS_ZLIB 帧无法在 native 打开前安全验证".to_string());
        }
        let stored_bytes = if compression == DNF_COMPRESS_NONE {
            let pixel_bytes = match format {
                0x0e | 0x0f => 2,
                0x10 => 4,
                _ => 2,
            };
            usize::try_from(width)
                .ok()
                .and_then(|width| {
                    usize::try_from(height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(pixel_bytes))
                .ok_or_else(|| "IMG 未压缩帧大小溢出".to_string())?
        } else {
            declared_size
        };
        if compression == DNF_COMPRESS_ZLIB && stored_bytes > limits.max_compressed_frame_bytes {
            return Err(format!(
                "压缩帧数据超过排查上限（{stored_bytes} > {} bytes）",
                limits.max_compressed_frame_bytes
            ));
        }
        frames.push(PreflightFrame::Payload(PreflightFramePayload {
            format,
            compression,
            width,
            height,
            stored_bytes,
        }));
    }

    let mut data_cursor = if images_size == 0 {
        cursor
    } else {
        probe_checked_add(frame_header_start, images_size, "IMG 数据区偏移")?
    };
    if data_cursor < cursor || data_cursor > bytes.len() {
        return Err("IMG 数据区偏移无效".to_string());
    }
    for frame in &frames {
        let PreflightFrame::Payload(payload) = frame else {
            continue;
        };
        data_cursor = probe_checked_add(data_cursor, payload.stored_bytes, "IMG 帧数据区偏移")?;
        if data_cursor > bytes.len() {
            return Err("IMG 帧数据被截断".to_string());
        }
    }
    let layout = PreflightImgLayout {
        version,
        palette_count,
        frames,
    };
    for index in 0..layout.frames.len() {
        layout.effective_frame_index(index)?;
    }
    Ok(layout)
}

fn probe_read_i32(bytes: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    let end = probe_checked_add(offset, 4, label)?;
    let raw = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label}被截断"))?;
    Ok(i32::from_le_bytes(
        raw.try_into().map_err(|_| format!("{label}无效"))?,
    ))
}

fn probe_checked_add(left: usize, right: usize, label: &str) -> Result<usize, String> {
    left.checked_add(right)
        .ok_or_else(|| format!("{label}溢出"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
