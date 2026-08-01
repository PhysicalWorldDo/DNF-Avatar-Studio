use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use encoding_rs::EUC_KR;
use serde::{Deserialize, Serialize};

use crate::dnflib_safe::{ImgArchive, NpkArchive};

use super::build_patched_img_bytes_many;
use super::models::{DyeTargetKindV1, DyeTargetRecipeV1};
use super::palette_patch::sha256;
use super::session::AvatarDyeSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DyeChangedEntry {
    pub(crate) entry_ref: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) source_bytes: Vec<u8>,
    pub(crate) target_kind: DyeTargetKindV1,
    pub(crate) palette_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeNpkExportSummary {
    pub output_path: String,
    pub entry_count: usize,
    pub dyed_base_count: usize,
    pub dyed_mask_count: usize,
    pub recipe_revision: String,
}

struct VerifiedSourceNpk {
    canonical_path: PathBuf,
    original_sha256: [u8; 32],
    archive: NpkArchive,
}

struct CreatedOutputGuard {
    path: PathBuf,
    file: Option<File>,
    armed: bool,
}

impl CreatedOutputGuard {
    fn new(path: PathBuf, file: File) -> Self {
        Self {
            path,
            file: Some(file),
            armed: true,
        }
    }

    fn close_file(&mut self) {
        drop(self.file.take());
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        self.close_file();
        if !self.armed {
            return Ok(());
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn error_after_cleanup(mut self, primary_error: String) -> String {
        match self.cleanup() {
            Ok(()) => primary_error,
            Err(cleanup_error) => {
                format!("{primary_error}；清理本次新建的染色补丁 NPK 失败：{cleanup_error}")
            }
        }
    }

    fn disarm(mut self) {
        self.close_file();
        self.armed = false;
    }
}

impl Drop for CreatedOutputGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

trait ExportHooks {
    fn after_header_write(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn before_output_validation(&mut self, _output_path: &Path) -> Result<(), String> {
        Ok(())
    }

    fn before_source_verification(&mut self) -> Result<(), String> {
        Ok(())
    }
}

struct NoExportHooks;

impl ExportHooks for NoExportHooks {}

impl AvatarDyeSession {
    pub(crate) fn changed_entries(&self) -> Result<Vec<DyeChangedEntry>, String> {
        let recipe = self.recipe()?;
        if recipe.is_empty() {
            return Ok(Vec::new());
        }
        let mut source_entries = BTreeMap::<String, (String, Vec<u8>, DyeTargetKindV1)>::new();
        for target in self.targets.iter().filter(|target| target.has_edits()) {
            let key = normalized_entry(target.private_entry_ref());
            if let Some(existing) = source_entries.get(&key) {
                if existing.1 != target.source_bytes() {
                    return Err("同一 IMG 入口存在冲突的染色结果".to_string());
                }
                continue;
            }
            source_entries.insert(
                key,
                (
                    target.private_entry_ref().to_string(),
                    target.source_bytes().to_vec(),
                    target.target_kind().clone(),
                ),
            );
        }
        let mut recipe_targets = BTreeMap::<String, Vec<&DyeTargetRecipeV1>>::new();
        for target in &recipe.targets {
            recipe_targets
                .entry(normalized_entry(&target.target.private_entry_ref))
                .or_default()
                .push(target);
        }
        let mut entries = Vec::with_capacity(recipe_targets.len());
        for (key, targets) in recipe_targets {
            let (entry_ref, source_bytes, target_kind) = source_entries
                .remove(&key)
                .ok_or_else(|| "染色配方与当前会话资源不匹配".to_string())?;
            let bytes = build_patched_img_bytes_many(&source_bytes, &targets)?;
            let mut palette_indices = targets
                .iter()
                .map(|target| usize::from(target.palette_index))
                .collect::<Vec<_>>();
            palette_indices.sort_unstable();
            palette_indices.dedup();
            entries.push(DyeChangedEntry {
                entry_ref,
                bytes,
                source_bytes,
                target_kind,
                palette_indices,
            });
        }
        if !source_entries.is_empty() {
            return Err("染色配方与当前会话资源不匹配".to_string());
        }
        Ok(entries)
    }

    pub(crate) fn export_dye_npk(
        &self,
        output_path: &Path,
        source_npk_paths: &[PathBuf],
    ) -> Result<DyeNpkExportSummary, String> {
        self.export_dye_npk_with_hooks(output_path, source_npk_paths, NoExportHooks)
    }

    fn export_dye_npk_with_hooks<H: ExportHooks>(
        &self,
        output_path: &Path,
        source_npk_paths: &[PathBuf],
        mut hooks: H,
    ) -> Result<DyeNpkExportSummary, String> {
        let entries = self.changed_entries()?;
        if entries.is_empty() {
            return Err("当前染色会话没有需要导出的改动".to_string());
        }
        let sources = open_verified_sources(source_npk_paths)?;
        validate_output_path(output_path, &sources)?;
        validate_source_entries(&entries, &sources)?;

        let created_output = write_npk_create_new(output_path, &entries, &mut hooks)?;
        let result = (|| {
            hooks.before_output_validation(output_path)?;
            validate_exported_npk(output_path, &entries)?;
            hooks.before_source_verification()?;
            verify_sources_unchanged(&sources)?;

            let recipe = self.recipe()?;
            let dyed_base_count = entries
                .iter()
                .filter(|entry| entry.target_kind == DyeTargetKindV1::Base)
                .count();
            let dyed_mask_count = entries.len() - dyed_base_count;
            Ok(DyeNpkExportSummary {
                output_path: output_path.to_string_lossy().to_string(),
                entry_count: entries.len(),
                dyed_base_count,
                dyed_mask_count,
                recipe_revision: hex_bytes(&recipe.recipe_hash),
            })
        })();

        match result {
            Ok(summary) => {
                created_output.disarm();
                Ok(summary)
            }
            Err(error) => Err(created_output.error_after_cleanup(error)),
        }
    }
}

fn write_npk_create_new(
    output_path: &Path,
    entries: &[DyeChangedEntry],
    hooks: &mut impl ExportHooks,
) -> Result<CreatedOutputGuard, String> {
    const MAGIC: &[u8; 16] = b"NeoplePack_Bill\0";
    const HEADER_BASE_SIZE: usize = 20;
    const INDEX_ENTRY_SIZE: usize = 264;
    const HASH_SIZE: usize = 32;
    const MAX_ENTRY_COUNT: usize = 65_535;

    if entries.is_empty() || entries.len() > MAX_ENTRY_COUNT {
        return Err("染色补丁 NPK 入口数量无效".to_string());
    }
    let index_size = entries
        .len()
        .checked_mul(INDEX_ENTRY_SIZE)
        .ok_or_else(|| "染色补丁 NPK 索引大小溢出".to_string())?;
    let header_size = HEADER_BASE_SIZE
        .checked_add(index_size)
        .ok_or_else(|| "染色补丁 NPK 头部大小溢出".to_string())?;
    let mut next_offset = header_size
        .checked_add(HASH_SIZE)
        .ok_or_else(|| "染色补丁 NPK 数据偏移溢出".to_string())?;
    let entry_count =
        u32::try_from(entries.len()).map_err(|_| "染色补丁 NPK 入口数量无效".to_string())?;

    let mut header = vec![0_u8; header_size];
    header[..MAGIC.len()].copy_from_slice(MAGIC);
    header[16..20].copy_from_slice(&entry_count.to_le_bytes());
    for (index, entry) in entries.iter().enumerate() {
        if entry.bytes.is_empty() {
            return Err("染色补丁 NPK 不能包含空 IMG".to_string());
        }
        let offset = i32::try_from(next_offset)
            .map_err(|_| "染色补丁 NPK 数据偏移超过格式上限".to_string())?;
        let size = i32::try_from(entry.bytes.len())
            .map_err(|_| "染色补丁 IMG 大小超过格式上限".to_string())?;
        next_offset = next_offset
            .checked_add(entry.bytes.len())
            .ok_or_else(|| "染色补丁 NPK 数据大小溢出".to_string())?;
        if next_offset > i32::MAX as usize {
            return Err("染色补丁 NPK 数据大小超过格式上限".to_string());
        }

        let (encoded_name, _, had_errors) = EUC_KR.encode(&entry.entry_ref);
        if had_errors {
            return Err("染色补丁 IMG 入口名无法编码为 EUC-KR".to_string());
        }
        if encoded_name.len() > 255 {
            return Err("染色补丁 IMG 入口名超过格式上限".to_string());
        }
        if encoded_name.contains(&0) {
            return Err("染色补丁 IMG 入口名包含 NUL 字节".to_string());
        }
        let mut encrypted_name = [0_u8; 256];
        encrypted_name[..encoded_name.len()].copy_from_slice(encoded_name.as_ref());
        xor_npk_entry_name(&mut encrypted_name);

        let entry_start = HEADER_BASE_SIZE + index * INDEX_ENTRY_SIZE;
        header[entry_start..entry_start + 4].copy_from_slice(&offset.to_le_bytes());
        header[entry_start + 4..entry_start + 8].copy_from_slice(&size.to_le_bytes());
        header[entry_start + 8..entry_start + INDEX_ENTRY_SIZE].copy_from_slice(&encrypted_name);
    }

    let hash_input_len = header.len() / 17 * 17;
    let header_hash = sha256(&header[..hash_input_len]);
    let output_file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Err("输出 NPK 已存在，禁止覆盖".to_string());
        }
        Err(_) => return Err("无法创建染色补丁 NPK".to_string()),
    };
    let mut output = CreatedOutputGuard::new(output_path.to_path_buf(), output_file);
    let write_result = (|| {
        let file = output
            .file
            .as_mut()
            .expect("new dye output remains open while writing");
        file.write_all(&header)
            .and_then(|_| file.write_all(&header_hash))
            .map_err(|_| "写入染色补丁 NPK 头部失败".to_string())?;
        hooks.after_header_write()?;
        for entry in entries {
            file.write_all(&entry.bytes)
                .map_err(|_| "写入染色补丁 IMG 数据失败".to_string())?;
        }
        file.sync_all()
            .map_err(|_| "同步染色补丁 NPK 失败".to_string())?;
        Ok(())
    })();
    if let Err(error) = write_result {
        return Err(output.error_after_cleanup(error));
    }
    output.close_file();
    Ok(output)
}

fn xor_npk_entry_name(name: &mut [u8; 256]) {
    const KEY_HEADER: &[u8] = b"puchikon@neople dungeon and fighter ";
    const KEY_SUFFIX: &[u8; 3] = b"DNF";
    for (index, byte) in name.iter_mut().enumerate() {
        let key = if index < KEY_HEADER.len() {
            KEY_HEADER[index]
        } else if index < 255 {
            KEY_SUFFIX[index % KEY_SUFFIX.len()]
        } else {
            0
        };
        *byte ^= key;
    }
}

fn open_verified_sources(paths: &[PathBuf]) -> Result<Vec<VerifiedSourceNpk>, String> {
    if paths.is_empty() {
        return Err("没有提供只读源 NPK".to_string());
    }
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical_path = path
            .canonicalize()
            .map_err(|_| "源 NPK 不存在或无法读取".to_string())?;
        if sources
            .iter()
            .any(|source: &VerifiedSourceNpk| source.canonical_path == canonical_path)
        {
            continue;
        }
        let bytes =
            std::fs::read(&canonical_path).map_err(|_| "源 NPK 不存在或无法读取".to_string())?;
        let original_sha256 = sha256(&bytes);
        let archive = NpkArchive::open_bytes(bytes)?;
        sources.push(VerifiedSourceNpk {
            canonical_path,
            original_sha256,
            archive,
        });
    }
    Ok(sources)
}

fn validate_output_path(output_path: &Path, sources: &[VerifiedSourceNpk]) -> Result<(), String> {
    if !output_path.is_absolute() {
        return Err("输出 NPK 必须使用绝对路径".to_string());
    }
    if !output_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("npk"))
    {
        return Err("输出文件扩展名必须是 .npk".to_string());
    }
    let comparable_output = comparable_output_path(output_path)?;
    if sources
        .iter()
        .any(|source| source.canonical_path == comparable_output)
    {
        return Err("输出路径不能等于任何源 NPK".to_string());
    }
    if output_path.exists() {
        return Err("输出 NPK 已存在，禁止覆盖".to_string());
    }
    Ok(())
}

fn comparable_output_path(output_path: &Path) -> Result<PathBuf, String> {
    if output_path.exists() {
        return output_path
            .canonicalize()
            .map_err(|_| "输出 NPK 路径无效".to_string());
    }
    let parent = output_path
        .parent()
        .ok_or_else(|| "输出 NPK 路径无效".to_string())?
        .canonicalize()
        .map_err(|_| "输出 NPK 目录不存在或无法读取".to_string())?;
    let file_name = output_path
        .file_name()
        .ok_or_else(|| "输出 NPK 路径无效".to_string())?;
    Ok(parent.join(file_name))
}

fn validate_source_entries(
    entries: &[DyeChangedEntry],
    sources: &[VerifiedSourceNpk],
) -> Result<(), String> {
    for entry in entries {
        let mut matches = Vec::new();
        for source in sources {
            if let Some(index) = source.archive.find_entry_index(&entry.entry_ref) {
                matches.push(source.archive.entry_bytes(index)?);
            }
        }
        if matches.is_empty() {
            return Err("染色资源已变化：源 NPK 中缺少 IMG 入口".to_string());
        }
        if matches.len() != 1 {
            return Err("染色资源来源不唯一，拒绝导出".to_string());
        }
        if matches[0] != entry.source_bytes {
            return Err("染色资源已变化：源 IMG 指纹不一致".to_string());
        }
    }
    Ok(())
}

fn validate_exported_npk(path: &Path, expected: &[DyeChangedEntry]) -> Result<(), String> {
    let archive = NpkArchive::open_path(path)?;
    if archive.entries().len() != expected.len() {
        return Err("染色补丁 NPK 重读后的入口数量不一致".to_string());
    }
    for entry in expected {
        let index = archive
            .find_entry_index(&entry.entry_ref)
            .ok_or_else(|| "染色补丁 NPK 重读后缺少入口".to_string())?;
        let bytes = archive.entry_bytes(index)?;
        if bytes != entry.bytes {
            return Err("染色补丁 NPK 重读后的 IMG 字节不一致".to_string());
        }
        let img = ImgArchive::open_owned_bytes(bytes.to_vec())?;
        let frames = img.frames();
        if frames.is_empty() {
            return Err("染色补丁 IMG 没有帧".to_string());
        }
        for frame in frames {
            for palette_index in &entry.palette_indices {
                img.decode_frame_with_palette(frame.index, *palette_index)?;
            }
        }
    }
    Ok(())
}

fn verify_sources_unchanged(sources: &[VerifiedSourceNpk]) -> Result<(), String> {
    for source in sources {
        let current = std::fs::read(&source.canonical_path)
            .map_err(|_| "导出期间源 NPK 变得不可读取".to_string())?;
        if sha256(&current) != source.original_sha256 {
            return Err("导出期间源 NPK 发生变化".to_string());
        }
    }
    Ok(())
}

fn normalized_entry(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
