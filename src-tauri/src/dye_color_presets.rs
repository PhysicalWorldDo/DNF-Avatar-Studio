use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

const STORE_SCHEMA_VERSION: u32 = 1;
const MAX_CUSTOM_PRESETS: usize = 128;
const MAX_PRESET_NAME_CHARS: usize = 40;
const BUILTIN_PRESET_ROWS: [(u8, &str, [u8; 3]); 48] = [
    (1, "珊瑚", [252, 159, 172]),
    (2, "杏黄", [236, 106, 44]),
    (3, "土黄", [228, 162, 100]),
    (4, "绿", [76, 142, 100]),
    (5, "绿宝石", [132, 214, 220]),
    (6, "苍紫", [172, 130, 172]),
    (7, "深紫", [132, 70, 116]),
    (8, "深灰", [116, 119, 108]),
    (9, "鲜红", [224, 96, 124]),
    (10, "褐", [149, 90, 34]),
    (11, "黄绿", [196, 230, 69]),
    (12, "海绿", [116, 194, 156]),
    (13, "天蓝", [84, 154, 236]),
    (14, "蓝紫", [124, 79, 164]),
    (15, "淡紫", [188, 99, 172]),
    (16, "深红", [192, 34, 29]),
    (17, "金", [252, 206, 60]),
    (18, "草绿", [108, 158, 28]),
    (19, "钢青", [69, 102, 156]),
    (20, "藏青", [60, 74, 116]),
    (21, "粉红", [220, 154, 196]),
    (22, "银灰", [156, 162, 156]),
    (23, "栗子", [156, 74, 76]),
    (24, "红宝石", [249, 36, 24]),
    (25, "浅红", [255, 117, 116]),
    (26, "桃红", [255, 85, 249]),
    (27, "紫", [180, 51, 176]),
    (28, "翠绿", [0, 238, 255]),
    (29, "浅蓝", [102, 196, 255]),
    (30, "灰绿", [139, 164, 185]),
    (31, "群青", [61, 132, 237]),
    (32, "浅绿", [71, 216, 91]),
    (33, "柠檬", [255, 241, 105]),
    (34, "浅灰", [206, 204, 203]),
    (35, "夜空", [44, 102, 192]),
    (36, "棕红", [160, 69, 98]),
    (37, "浅黄", [188, 156, 141]),
    (38, "紫红", [155, 21, 19]),
    (39, "黄玉", [249, 160, 24]),
    (40, "孔雀石", [14, 150, 153]),
    (41, "深墨", [60, 111, 116]),
    (42, "冰蓝", [158, 224, 255]),
    (43, "深青", [102, 125, 255]),
    (44, "黑紫", [74, 55, 100]),
    (45, "青紫", [91, 78, 164]),
    (46, "灰紫", [141, 138, 212]),
    (47, "橄榄", [174, 176, 74]),
    (48, "黑", [10, 10, 10]),
];

static BUILTIN_PRESETS: OnceLock<Result<Vec<BuiltinDyeColorPreset>, String>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(1);
static PROCESS_FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinDyeColorPreset {
    pub id: String,
    pub number: u8,
    pub name: String,
    pub rgb: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomDyeColorPreset {
    pub id: String,
    pub name: String,
    pub rgb: [u8; 3],
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DyeColorPresetCatalog {
    pub schema_version: u32,
    pub revision: u64,
    pub builtin_presets: Vec<BuiltinDyeColorPreset>,
    pub custom_presets: Vec<CustomDyeColorPreset>,
    pub storage_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DyeColorPresetStore {
    schema_version: u32,
    revision: u64,
    presets: Vec<CustomDyeColorPreset>,
}

impl DyeColorPresetStore {
    fn empty() -> Self {
        Self {
            schema_version: STORE_SCHEMA_VERSION,
            revision: 0,
            presets: Vec::new(),
        }
    }
}

pub struct DyeColorPresetAppState {
    path: PathBuf,
}

impl DyeColorPresetAppState {
    pub fn open_default() -> Self {
        Self::open_at(
            crate::app_config::runtime_root()
                .join("config")
                .join("dye_color_presets.json"),
        )
    }

    pub(crate) fn open_at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load_catalog(&self) -> Result<DyeColorPresetCatalog, String> {
        let _lock = PresetFileLock::acquire(&self.path)?;
        let (store, storage_error) = read_store_resilient(&self.path);
        catalog_from_store(store, storage_error)
    }

    pub fn create_preset(
        &self,
        expected_revision: u64,
        name: &str,
        rgb: [u8; 3],
    ) -> Result<DyeColorPresetCatalog, String> {
        let _lock = PresetFileLock::acquire(&self.path)?;
        let mut store = read_store_for_mutation(&self.path)?;
        ensure_revision(&store, expected_revision)?;
        if store.presets.len() >= MAX_CUSTOM_PRESETS {
            return Err(format!("自定义染色剂预设最多保存 {MAX_CUSTOM_PRESETS} 个"));
        }
        let name = normalize_name(name)?;
        ensure_unique(&store, None, &name, rgb)?;
        let timestamp = now_ms().max(1);
        store.presets.push(CustomDyeColorPreset {
            id: new_custom_id(),
            name,
            rgb,
            created_at_ms: timestamp,
            updated_at_ms: timestamp,
        });
        bump_revision(&mut store)?;
        write_store(&self.path, &store)?;
        catalog_from_store(store, None)
    }

    pub fn update_preset(
        &self,
        expected_revision: u64,
        id: &str,
        name: &str,
        rgb: [u8; 3],
    ) -> Result<DyeColorPresetCatalog, String> {
        let _lock = PresetFileLock::acquire(&self.path)?;
        let mut store = read_store_for_mutation(&self.path)?;
        ensure_revision(&store, expected_revision)?;
        let index = store
            .presets
            .iter()
            .position(|preset| preset.id == id)
            .ok_or_else(|| "没有找到自定义染色剂预设".to_string())?;
        let name = normalize_name(name)?;
        ensure_unique(&store, Some(id), &name, rgb)?;
        if store.presets[index].name == name && store.presets[index].rgb == rgb {
            return catalog_from_store(store, None);
        }
        let next_timestamp = now_ms()
            .max(store.presets[index].updated_at_ms.saturating_add(1))
            .max(1);
        store.presets[index].name = name;
        store.presets[index].rgb = rgb;
        store.presets[index].updated_at_ms = next_timestamp;
        bump_revision(&mut store)?;
        write_store(&self.path, &store)?;
        catalog_from_store(store, None)
    }

    pub fn delete_preset(
        &self,
        expected_revision: u64,
        id: &str,
    ) -> Result<DyeColorPresetCatalog, String> {
        let _lock = PresetFileLock::acquire(&self.path)?;
        let mut store = read_store_for_mutation(&self.path)?;
        ensure_revision(&store, expected_revision)?;
        let index = store
            .presets
            .iter()
            .position(|preset| preset.id == id)
            .ok_or_else(|| "没有找到自定义染色剂预设".to_string())?;
        store.presets.remove(index);
        bump_revision(&mut store)?;
        write_store(&self.path, &store)?;
        catalog_from_store(store, None)
    }
}

fn catalog_from_store(
    store: DyeColorPresetStore,
    storage_error: Option<String>,
) -> Result<DyeColorPresetCatalog, String> {
    Ok(DyeColorPresetCatalog {
        schema_version: STORE_SCHEMA_VERSION,
        revision: store.revision,
        builtin_presets: builtin_presets()?.to_vec(),
        custom_presets: store.presets,
        storage_error,
    })
}

fn builtin_presets() -> Result<&'static [BuiltinDyeColorPreset], String> {
    BUILTIN_PRESETS
        .get_or_init(|| {
            let presets = BUILTIN_PRESET_ROWS
                .iter()
                .map(|(number, name, rgb)| BuiltinDyeColorPreset {
                    id: format!("basic-{number:03}"),
                    number: *number,
                    name: (*name).to_string(),
                    rgb: *rgb,
                })
                .collect::<Vec<_>>();
            validate_builtin_presets(&presets)?;
            Ok(presets)
        })
        .as_ref()
        .map(Vec::as_slice)
        .map_err(Clone::clone)
}

fn validate_builtin_presets(presets: &[BuiltinDyeColorPreset]) -> Result<(), String> {
    if presets.len() != 48 {
        return Err("内置染色剂色表必须包含 48 个颜色".to_string());
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut colors = HashSet::new();
    for (index, preset) in presets.iter().enumerate() {
        let number = u8::try_from(index + 1).map_err(|_| "内置染色剂编号无效".to_string())?;
        if preset.number != number || preset.id != format!("basic-{number:03}") {
            return Err("内置染色剂编号或 ID 不连续".to_string());
        }
        if !ids.insert(preset.id.clone())
            || !names.insert(name_key(&normalize_name(&preset.name)?))
            || !colors.insert(preset.rgb)
        {
            return Err("内置染色剂色表存在重复项".to_string());
        }
    }
    Ok(())
}

fn read_store_resilient(path: &Path) -> (DyeColorPresetStore, Option<String>) {
    if path.exists() {
        return match read_store(path) {
            Ok(store) => (store, None),
            Err(primary_error) => {
                let backup = backup_path(path);
                let fallback = read_store(&backup).unwrap_or_else(|_| DyeColorPresetStore::empty());
                (fallback, Some(primary_error))
            }
        };
    }
    let backup = backup_path(path);
    if backup.exists() {
        return match read_store(&backup) {
            Ok(store) => (
                store,
                Some("染色剂预设主文件缺失，当前显示备份内容".to_string()),
            ),
            Err(error) => (DyeColorPresetStore::empty(), Some(error)),
        };
    }
    (DyeColorPresetStore::empty(), None)
}

fn read_store_for_mutation(path: &Path) -> Result<DyeColorPresetStore, String> {
    if path.exists() {
        return read_store(path);
    }
    let backup = backup_path(path);
    if backup.exists() {
        return read_store(&backup);
    }
    Ok(DyeColorPresetStore::empty())
}

fn read_store(path: &Path) -> Result<DyeColorPresetStore, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("读取染色剂预设失败 {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("解析染色剂预设失败 {}: {error}", path.display()))?;
    let schema_version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("染色剂预设缺少版本信息 {}", path.display()))?;
    if schema_version != u64::from(STORE_SCHEMA_VERSION) {
        return Err(format!(
            "染色剂预设版本 {schema_version} 不受支持，已保留原文件 {}",
            path.display()
        ));
    }
    let store: DyeColorPresetStore = serde_json::from_value(value)
        .map_err(|error| format!("解析染色剂预设失败 {}: {error}", path.display()))?;
    validate_store(&store)?;
    Ok(store)
}

fn validate_store(store: &DyeColorPresetStore) -> Result<(), String> {
    if store.schema_version != STORE_SCHEMA_VERSION {
        return Err("染色剂预设版本不受支持".to_string());
    }
    if store.presets.len() > MAX_CUSTOM_PRESETS {
        return Err(format!("自定义染色剂预设超过 {MAX_CUSTOM_PRESETS} 个"));
    }
    let mut ids = HashSet::new();
    for preset in &store.presets {
        if !preset.id.starts_with("custom-") || !ids.insert(preset.id.clone()) {
            return Err("自定义染色剂预设 ID 无效或重复".to_string());
        }
        if normalize_name(&preset.name)? != preset.name {
            return Err("自定义染色剂预设名称未规范化".to_string());
        }
        if preset.created_at_ms == 0 || preset.updated_at_ms < preset.created_at_ms {
            return Err("自定义染色剂预设时间信息无效".to_string());
        }
        ensure_unique_against(&store.presets, Some(&preset.id), &preset.name, preset.rgb)?;
    }
    Ok(())
}

fn ensure_revision(store: &DyeColorPresetStore, expected_revision: u64) -> Result<(), String> {
    if store.revision != expected_revision {
        return Err("染色剂预设版本已变化，请刷新后重试".to_string());
    }
    Ok(())
}

fn ensure_unique(
    store: &DyeColorPresetStore,
    excluded_id: Option<&str>,
    name: &str,
    rgb: [u8; 3],
) -> Result<(), String> {
    ensure_unique_against(&store.presets, excluded_id, name, rgb)
}

fn ensure_unique_against(
    customs: &[CustomDyeColorPreset],
    excluded_id: Option<&str>,
    name: &str,
    rgb: [u8; 3],
) -> Result<(), String> {
    let key = name_key(name);
    if builtin_presets()?
        .iter()
        .any(|preset| name_key(&preset.name) == key)
    {
        return Err("已有同名染色剂预设".to_string());
    }
    if builtin_presets()?.iter().any(|preset| preset.rgb == rgb) {
        return Err("已有相同颜色的染色剂预设".to_string());
    }
    for preset in customs
        .iter()
        .filter(|preset| Some(preset.id.as_str()) != excluded_id)
    {
        if name_key(&preset.name) == key {
            return Err("已有同名染色剂预设".to_string());
        }
        if preset.rgb == rgb {
            return Err("已有相同颜色的染色剂预设".to_string());
        }
    }
    Ok(())
}

fn normalize_name(name: &str) -> Result<String, String> {
    if name.chars().any(char::is_control) {
        return Err("染色剂预设名称不能包含控制字符".to_string());
    }
    let normalized = name.nfkc().collect::<String>();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = normalized.chars().count();
    if length == 0 {
        return Err("染色剂预设名称不能为空".to_string());
    }
    if length > MAX_PRESET_NAME_CHARS {
        return Err(format!(
            "染色剂预设名称不能超过 {MAX_PRESET_NAME_CHARS} 个字符"
        ));
    }
    Ok(normalized)
}

fn name_key(name: &str) -> String {
    name.to_lowercase()
}

fn bump_revision(store: &mut DyeColorPresetStore) -> Result<(), String> {
    store.revision = store
        .revision
        .checked_add(1)
        .ok_or_else(|| "染色剂预设版本已达到上限".to_string())?;
    Ok(())
}

fn new_custom_id() -> String {
    format!(
        "custom-{:x}-{:x}-{:x}",
        now_nanos(),
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn write_store(path: &Path, store: &DyeColorPresetStore) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(store).map_err(|error| format!("编码染色剂预设失败: {error}"))?;
    if path.exists() {
        let previous = fs::read(path)
            .map_err(|error| format!("读取染色剂预设备份源失败 {}: {error}", path.display()))?;
        write_bytes_atomically(&backup_path(path), &previous)?;
    } else {
        write_bytes_atomically(&backup_path(path), &bytes)?;
    }
    write_bytes_atomically(path, &bytes)
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("染色剂预设路径无效 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建染色剂预设目录失败 {}: {error}", parent.display()))?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("创建染色剂预设临时文件失败: {error}"))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("写入染色剂预设临时文件失败: {error}"))?;
        drop(file);
        replace_file(path, &temporary)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("bak")
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "dye_color_presets.json".into());
    path.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(windows)]
fn replace_file(path: &Path, temporary: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, target: *const u16, flags: u32) -> i32;
    }

    if !path.exists() {
        return fs::rename(temporary, path)
            .map_err(|error| format!("提交染色剂预设失败 {}: {error}", path.display()));
    }
    let wide = |value: &Path| {
        value
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let from = wide(temporary);
    let to = wide(path);
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        return Err(format!("原子提交染色剂预设失败 {}", path.display()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(path: &Path, temporary: &Path) -> Result<(), String> {
    fs::rename(temporary, path)
        .map_err(|error| format!("提交染色剂预设失败 {}: {error}", path.display()))
}

struct PresetFileLock {
    _process_guard: MutexGuard<'static, ()>,
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
}

impl PresetFileLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        let process_guard = PROCESS_FILE_LOCK
            .lock()
            .map_err(|_| "染色剂预设文件锁不可用".to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;

            #[link(name = "Kernel32")]
            unsafe extern "system" {
                fn CreateMutexW(
                    mutex_attributes: *const std::ffi::c_void,
                    initial_owner: i32,
                    name: *const u16,
                ) -> *mut std::ffi::c_void;
                fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
                fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
            }

            let name = mutex_name(path);
            let wide = std::ffi::OsStr::new(&name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
            if handle.is_null() {
                return Err("创建染色剂预设文件锁失败".to_string());
            }
            let result = unsafe { WaitForSingleObject(handle, 10_000) };
            if result != 0 && result != 0x80 {
                unsafe {
                    CloseHandle(handle);
                }
                return Err("等待染色剂预设文件锁超时".to_string());
            }
            return Ok(Self {
                _process_guard: process_guard,
                handle,
            });
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Ok(Self {
                _process_guard: process_guard,
            })
        }
    }
}

#[cfg(windows)]
impl Drop for PresetFileLock {
    fn drop(&mut self) {
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

fn mutex_name(path: &Path) -> String {
    let identity = canonical_lock_path(path);
    let digest = Sha256::digest(identity.to_string_lossy().to_lowercase().as_bytes());
    let mut suffix = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(suffix, "{byte:02x}");
    }
    format!("Global\\DNFAvatarStudioDyePresets-{suffix}")
}

fn canonical_lock_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if let Ok(canonical) = fs::canonicalize(&absolute) {
        return canonical;
    }

    let mut missing = Vec::new();
    let mut cursor = absolute.as_path();
    while !cursor.exists() {
        let Some(name) = cursor.file_name() else {
            return absolute;
        };
        missing.push(name.to_os_string());
        let Some(parent) = cursor.parent() else {
            return absolute;
        };
        cursor = parent;
    }
    let mut canonical = fs::canonicalize(cursor).unwrap_or_else(|_| cursor.to_path_buf());
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    canonical
}
