use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::catalog_gallery::CatalogGalleryKind;
use crate::catalog_thumbnail::thumbnail_render_profile_key;

const GALLERY_SNAPSHOT_SCHEMA_VERSION: u16 = 2;
const GALLERY_SNAPSHOT_PURPOSE: &[u8] = b"dnf-avatar-studio-gallery-snapshot-v2";
const MAX_GALLERY_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GallerySnapshotContext {
    pub game_dir: String,
    pub catalog_build_id: String,
    pub kind: CatalogGalleryKind,
    pub playable_role: String,
}

impl GallerySnapshotContext {
    pub fn from_game_dir(
        game_dir: &Path,
        catalog_build_id: &str,
        kind: CatalogGalleryKind,
        playable_role: &str,
    ) -> Self {
        let normalized_game_dir = fs::canonicalize(game_dir)
            .unwrap_or_else(|_| game_dir.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        Self {
            game_dir: normalized_game_dir,
            catalog_build_id: catalog_build_id.trim().to_string(),
            kind,
            playable_role: playable_role.trim().to_ascii_lowercase(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GallerySnapshotEntry {
    pub entity_id: String,
    pub name: String,
    pub slot: String,
    pub source_order: usize,
    pub entity_recipe_key: String,
    pub visual_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GallerySnapshotDependency {
    pub path: String,
    pub file_len: u64,
    pub modified_ns: u64,
    pub content_sha256: String,
}

impl GallerySnapshotDependency {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let file = File::open(path)
            .map_err(|error| format!("打开图库快照资源失败 {}：{error}", path.display()))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("读取图库快照资源失败 {}：{error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("图库快照依赖不是文件：{}", path.display()));
        }
        let file_len = metadata.len();
        let modified_at_ns = modified_ns(&metadata);
        let mut digest = Sha256::new();
        let mut reader = BufReader::new(file);
        let mut buffer = [0_u8; 1024 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("读取图库快照资源内容失败 {}：{error}", path.display()))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        let current = fs::metadata(path)
            .map_err(|error| format!("复核图库快照资源失败 {}：{error}", path.display()))?;
        if !current.is_file()
            || current.len() != file_len
            || modified_ns(&current) != modified_at_ns
        {
            return Err(format!(
                "图库快照资源在读取期间发生变化：{}",
                path.display()
            ));
        }
        Ok(Self {
            path: Self::normalized_path(path),
            file_len,
            modified_ns: modified_at_ns,
            content_sha256: hex_digest(digest.finalize()),
        })
    }

    pub(crate) fn normalized_path(path: &Path) -> String {
        fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase()
    }

    pub(crate) fn fingerprint(&self) -> String {
        format!(
            "npk-v2|{}|{}|{}|{}",
            self.path, self.file_len, self.modified_ns, self.content_sha256
        )
    }

    pub(crate) fn metadata_is_current(&self) -> bool {
        let Ok(metadata) = fs::metadata(&self.path) else {
            return false;
        };
        metadata.is_file()
            && metadata.len() == self.file_len
            && modified_ns(&metadata) == self.modified_ns
    }

    pub(crate) fn is_current(&self) -> bool {
        if !self.metadata_is_current() {
            return false;
        }
        Self::from_file(Path::new(&self.path))
            .map(|current| current == *self)
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GallerySnapshot {
    pub schema_version: u16,
    pub snapshot_id: String,
    pub context: GallerySnapshotContext,
    pub catalog_total: usize,
    pub entries: Vec<GallerySnapshotEntry>,
    pub dependencies: Vec<GallerySnapshotDependency>,
}

impl GallerySnapshot {
    pub fn new(
        context: GallerySnapshotContext,
        catalog_total: usize,
        entries: Vec<GallerySnapshotEntry>,
        dependencies: Vec<GallerySnapshotDependency>,
    ) -> Self {
        Self {
            schema_version: GALLERY_SNAPSHOT_SCHEMA_VERSION,
            snapshot_id: Uuid::new_v4().to_string(),
            context,
            catalog_total,
            entries,
            dependencies,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GallerySnapshotStore {
    root: PathBuf,
}

impl GallerySnapshotStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn save(&self, snapshot: &GallerySnapshot) -> Result<(), String> {
        validate_snapshot(snapshot)?;
        if !snapshot_dependencies_are_current(&snapshot.dependencies) {
            return Err("图库快照依赖已变化，拒绝保存混合版本快照".to_string());
        }
        fs::create_dir_all(&self.root)
            .map_err(|error| format!("创建图库快照目录失败 {}：{error}", self.root.display()))?;
        let plaintext = postcard::to_allocvec(snapshot)
            .map_err(|error| format!("序列化图库快照失败：{error}"))?;
        if plaintext.len() as u64 > MAX_GALLERY_SNAPSHOT_BYTES {
            return Err("图库快照超过安全上限".to_string());
        }
        let encrypted = crate::local_protection::protect_for_current_user(
            &plaintext,
            GALLERY_SNAPSHOT_PURPOSE,
        )?;
        if encrypted.len() as u64 > MAX_GALLERY_SNAPSHOT_BYTES {
            return Err("加密图库快照超过安全上限".to_string());
        }

        let path = self.snapshot_path(&snapshot.context);
        let temporary = self
            .root
            .join(format!(".gallery-snapshot-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = File::create(&temporary)
                .map_err(|error| format!("创建图库快照临时文件失败：{error}"))?;
            file.write_all(&encrypted)
                .and_then(|_| file.flush())
                .map_err(|error| format!("写入图库快照失败：{error}"))?;
            file.sync_all()
                .map_err(|error| format!("同步图库快照失败：{error}"))?;
            drop(file);
            if !snapshot_dependencies_are_current(&snapshot.dependencies) {
                return Err("图库快照依赖在提交前发生变化".to_string());
            }
            replace_snapshot_file(&path, &temporary)?;
            if !snapshot_dependencies_are_current(&snapshot.dependencies) {
                let _ = fs::remove_file(&path);
                return Err("图库快照依赖在提交期间发生变化".to_string());
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn load(
        &self,
        context: &GallerySnapshotContext,
    ) -> Result<Option<GallerySnapshot>, String> {
        let path = self.snapshot_path(context);
        if !path.is_file() {
            return Ok(None);
        }
        let metadata =
            fs::metadata(&path).map_err(|error| format!("读取图库快照信息失败：{error}"))?;
        if metadata.len() == 0 || metadata.len() > MAX_GALLERY_SNAPSHOT_BYTES {
            return Err("图库快照文件大小无效".to_string());
        }
        let encrypted = fs::read(&path).map_err(|error| format!("读取图库快照失败：{error}"))?;
        let plaintext = crate::local_protection::unprotect_for_current_user(
            &encrypted,
            GALLERY_SNAPSHOT_PURPOSE,
        )?;
        if plaintext.len() as u64 > MAX_GALLERY_SNAPSHOT_BYTES {
            return Err("解密图库快照超过安全上限".to_string());
        }
        let snapshot: GallerySnapshot = postcard::from_bytes(&plaintext)
            .map_err(|error| format!("解析图库快照失败：{error}"))?;
        validate_snapshot(&snapshot)?;
        if snapshot.context != *context {
            return Ok(None);
        }
        if snapshot
            .dependencies
            .iter()
            .any(|dependency| !dependency.is_current())
        {
            return Ok(None);
        }
        Ok(Some(snapshot))
    }

    pub(crate) fn snapshot_path(&self, context: &GallerySnapshotContext) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(b"gallery-snapshot-context-v3|");
        digest.update(thumbnail_render_profile_key(&context.playable_role).as_bytes());
        digest.update(b"|");
        digest.update(context.game_dir.as_bytes());
        digest.update(b"|");
        digest.update(context.catalog_build_id.as_bytes());
        digest.update(b"|");
        digest.update(match context.kind {
            CatalogGalleryKind::Set => b"set".as_slice(),
            CatalogGalleryKind::Item => b"item".as_slice(),
        });
        digest.update(b"|");
        digest.update(context.playable_role.as_bytes());
        let key = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.root.join(format!("{key}.bin"))
    }
}

fn validate_snapshot(snapshot: &GallerySnapshot) -> Result<(), String> {
    if snapshot.schema_version != GALLERY_SNAPSHOT_SCHEMA_VERSION {
        return Err(format!("图库快照版本 {} 不受支持", snapshot.schema_version));
    }
    if snapshot.snapshot_id.trim().is_empty()
        || snapshot.context.game_dir.trim().is_empty()
        || snapshot.context.catalog_build_id.trim().is_empty()
        || snapshot.context.playable_role.trim().is_empty()
    {
        return Err("图库快照上下文无效".to_string());
    }
    if snapshot.entries.iter().any(|entry| {
        entry.entity_id.trim().is_empty()
            || entry.name.trim().is_empty()
            || entry.entity_recipe_key.trim().is_empty()
            || entry.visual_signature.trim().is_empty()
    }) {
        return Err("图库快照包含无效目录条目".to_string());
    }
    if snapshot.dependencies.iter().any(|dependency| {
        dependency.path.trim().is_empty()
            || dependency.content_sha256.len() != 64
            || !dependency
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err("图库快照包含无效资源依赖".to_string());
    }
    Ok(())
}

fn snapshot_dependencies_are_current(dependencies: &[GallerySnapshotDependency]) -> bool {
    dependencies
        .iter()
        .all(GallerySnapshotDependency::is_current)
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn modified_ns(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn replace_snapshot_file(path: &Path, temporary: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(temporary.as_ptr(), path.as_ptr(), 0x1 | 0x8) } == 0 {
        return Err("原子提交图库快照失败".to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_snapshot_file(path: &Path, temporary: &Path) -> Result<(), String> {
    fs::rename(temporary, path).map_err(|error| format!("提交图库快照失败：{error}"))
}
