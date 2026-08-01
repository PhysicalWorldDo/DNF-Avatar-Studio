use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::feature_gate::{FeatureGate, HiddenFeature};
use crate::models::{AppConfigUpdate, AppConfigView, HiddenFeatureUnlockResult, StoredAppConfig};

const CONFIG_FILE_NAME: &str = "dnf_avatar_studio_config.json";
static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(1);

pub struct AppConfigState {
    path: PathBuf,
    gate: FeatureGate,
    runtime: Mutex<ConfigRuntime>,
}

#[derive(Debug)]
struct ConfigRuntime {
    stored: StoredAppConfig,
    wardrobe_swap_unlocked: bool,
    wardrobe_swap_credential_invalid: bool,
    load_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialStatus {
    Absent,
    Valid,
    Invalid,
}

impl ConfigRuntime {
    fn from_stored(stored: StoredAppConfig, gate: &FeatureGate) -> Self {
        let wardrobe_swap_status = credential_status(
            &stored.wardrobe_swap_unlock_proof,
            HiddenFeature::WardrobeSwap,
            gate,
        );
        Self {
            stored,
            wardrobe_swap_unlocked: wardrobe_swap_status == CredentialStatus::Valid,
            wardrobe_swap_credential_invalid: wardrobe_swap_status == CredentialStatus::Invalid,
            load_error: None,
        }
    }

    fn view(&self) -> AppConfigView {
        AppConfigView {
            game_dir: self.stored.game_dir.clone(),
            wardrobe_swap_unlocked: self.wardrobe_swap_unlocked,
            wardrobe_swap_credential_invalid: self.wardrobe_swap_credential_invalid,
        }
    }

    fn is_unlocked(&self, feature: HiddenFeature) -> bool {
        match feature {
            HiddenFeature::WardrobeSwap => self.wardrobe_swap_unlocked,
        }
    }
}

impl AppConfigState {
    pub fn open_default() -> Self {
        let path = default_config_path();
        let gate = FeatureGate::production();
        Self::open_resilient_with_gate(path, gate)
    }

    pub(crate) fn open_resilient_with_gate(path: PathBuf, gate: FeatureGate) -> Self {
        match Self::open_with_gate(path.clone(), gate.clone()) {
            Ok(state) => state,
            Err(error) => Self {
                path,
                gate,
                runtime: Mutex::new(ConfigRuntime {
                    stored: StoredAppConfig::default(),
                    wardrobe_swap_unlocked: false,
                    wardrobe_swap_credential_invalid: false,
                    load_error: Some(error),
                }),
            },
        }
    }

    pub(crate) fn open_with_gate(path: PathBuf, gate: FeatureGate) -> Result<Self, String> {
        let stored = read_stored_config_from_path(&path)?;
        let runtime = ConfigRuntime::from_stored(stored, &gate);
        Ok(Self {
            path,
            gate,
            runtime: Mutex::new(runtime),
        })
    }

    pub fn load_view(&self) -> Result<AppConfigView, String> {
        let runtime = self.lock_runtime()?;
        if let Some(error) = &runtime.load_error {
            return Err(error.clone());
        }
        Ok(runtime.view())
    }

    pub fn save_update(&self, update: AppConfigUpdate) -> Result<AppConfigView, String> {
        let mut runtime = self.lock_runtime()?;
        let _file_lock = ConfigFileLock::acquire(&self.path)?;
        let mut next = self.read_latest_into_runtime(&mut runtime)?;
        next.game_dir = update.game_dir;
        write_stored_config_to_path(&self.path, &next)?;
        *runtime = ConfigRuntime::from_stored(next, &self.gate);
        Ok(runtime.view())
    }

    pub fn unlock_hidden_feature(&self, phrase: &str) -> Result<HiddenFeatureUnlockResult, String> {
        let feature = self.gate.feature_for_phrase(phrase);
        let mut runtime = self.lock_runtime()?;
        let Some(feature) = feature else {
            return Ok(HiddenFeatureUnlockResult {
                matched: false,
                feature: None,
                already_unlocked: false,
                config: runtime.view(),
            });
        };
        let _file_lock = ConfigFileLock::acquire(&self.path)?;
        let latest = self.read_latest_into_runtime(&mut runtime)?;
        if runtime.is_unlocked(feature) {
            return Ok(HiddenFeatureUnlockResult {
                matched: true,
                feature: Some(feature),
                already_unlocked: true,
                config: runtime.view(),
            });
        }

        let proof = self.gate.issue_proof(feature, phrase)?;
        let mut next = latest;
        *proof_value_mut(&mut next, feature) = serde_json::Value::String(proof);
        write_stored_config_to_path(&self.path, &next)?;
        *runtime = ConfigRuntime::from_stored(next, &self.gate);
        Ok(HiddenFeatureUnlockResult {
            matched: true,
            feature: Some(feature),
            already_unlocked: false,
            config: runtime.view(),
        })
    }

    pub fn ensure_unlocked(&self, feature: HiddenFeature) -> Result<(), String> {
        let runtime = self.lock_runtime()?;
        if runtime.is_unlocked(feature) {
            Ok(())
        } else {
            Err("该功能尚未激活".to_string())
        }
    }

    fn lock_runtime(&self) -> Result<std::sync::MutexGuard<'_, ConfigRuntime>, String> {
        self.runtime
            .lock()
            .map_err(|_| "读取应用配置状态失败".to_string())
    }
    fn read_latest_into_runtime(
        &self,
        runtime: &mut ConfigRuntime,
    ) -> Result<StoredAppConfig, String> {
        match read_stored_config_from_path(&self.path) {
            Ok(stored) => {
                *runtime = ConfigRuntime::from_stored(stored.clone(), &self.gate);
                Ok(stored)
            }
            Err(error) => {
                runtime.load_error = Some(error.clone());
                Err(error)
            }
        }
    }
}

struct ConfigFileLock {
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
}

impl ConfigFileLock {
    #[cfg(windows)]
    fn acquire(path: &Path) -> Result<Self, String> {
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

        const WAIT_OBJECT_0: u32 = 0;
        const WAIT_ABANDONED: u32 = 0x80;
        const LOCK_TIMEOUT_MS: u32 = 10_000;

        let name = config_mutex_name(path);
        let name_wide = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name_wide.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "创建配置文件互斥锁失败 {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        let wait_result = unsafe { WaitForSingleObject(handle, LOCK_TIMEOUT_MS) };
        if wait_result != WAIT_OBJECT_0 && wait_result != WAIT_ABANDONED {
            unsafe {
                CloseHandle(handle);
            }
            return Err(format!("等待配置文件互斥锁失败 {}", path.display()));
        }
        Ok(Self { handle })
    }

    #[cfg(not(windows))]
    fn acquire(_path: &Path) -> Result<Self, String> {
        Ok(Self {})
    }
}

#[cfg(windows)]
impl Drop for ConfigFileLock {
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

fn config_mutex_name(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let normalized = absolute.to_string_lossy().to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    let mut suffix = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(suffix, "{byte:02x}");
    }
    format!("Local\\DNFAvatarStudioConfig-{suffix}")
}

fn credential_status(
    value: &serde_json::Value,
    feature: HiddenFeature,
    gate: &FeatureGate,
) -> CredentialStatus {
    match value {
        serde_json::Value::Null => CredentialStatus::Absent,
        serde_json::Value::String(proof) if proof.is_empty() => CredentialStatus::Absent,
        serde_json::Value::String(proof) if gate.proof_is_valid(feature, proof) => {
            CredentialStatus::Valid
        }
        _ => CredentialStatus::Invalid,
    }
}

fn proof_value_mut(stored: &mut StoredAppConfig, feature: HiddenFeature) -> &mut serde_json::Value {
    match feature {
        HiddenFeature::WardrobeSwap => &mut stored.wardrobe_swap_unlock_proof,
    }
}

fn read_stored_config_from_path(path: &Path) -> Result<StoredAppConfig, String> {
    if !path.exists() {
        return Ok(StoredAppConfig::default());
    }
    let text = fs::read_to_string(path)
        .map_err(|err| format!("读取配置文件失败 {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("解析配置文件失败 {}: {err}", path.display()))
}

fn write_stored_config_to_path(path: &Path, config: &StoredAppConfig) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("配置文件路径无效 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|err| format!("创建配置目录失败 {}: {err}", parent.display()))?;
    let text = serde_json::to_vec_pretty(config).map_err(|err| format!("序列化配置失败: {err}"))?;
    let temporary = temporary_config_path(path);
    let write_result = (|| {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|err| format!("创建临时配置文件失败 {}: {err}", temporary.display()))?;
        output
            .write_all(&text)
            .and_then(|_| output.sync_all())
            .map_err(|err| format!("写入临时配置文件失败 {}: {err}", temporary.display()))?;
        drop(output);
        replace_config_file(path, &temporary)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn temporary_config_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "config.json".into());
    path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(windows)]
fn replace_config_file(path: &Path, temporary: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, target: *const u16, flags: u32) -> i32;
    }

    if !path.exists() {
        return fs::rename(temporary, path)
            .map_err(|err| format!("提交配置文件失败 {}: {err}", path.display()));
    }
    let wide = |value: &Path| {
        value
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let temporary = wide(temporary);
    let path_wide = wide(path);
    let ok = unsafe { MoveFileExW(temporary.as_ptr(), path_wide.as_ptr(), 0x1 | 0x8) };
    if ok == 0 {
        return Err(format!("原子提交配置文件失败 {}", path.display()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_config_file(path: &Path, temporary: &Path) -> Result<(), String> {
    fs::rename(temporary, path).map_err(|err| format!("提交配置文件失败 {}: {err}", path.display()))
}

fn default_config_path() -> PathBuf {
    runtime_root().join("config").join(CONFIG_FILE_NAME)
}

pub(crate) fn runtime_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = packaged_root_from_app_dir(exe.parent()) {
            return root;
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    packaged_root_from_app_dir(Some(cwd.as_path())).unwrap_or(cwd)
}

fn packaged_root_from_app_dir(dir: Option<&Path>) -> Option<PathBuf> {
    let app_dir = dir?;
    if app_dir
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("app")
    {
        let bin_dir = app_dir.parent()?;
        if bin_dir
            .file_name()?
            .to_string_lossy()
            .eq_ignore_ascii_case("bin")
        {
            return bin_dir.parent().map(Path::to_path_buf);
        }
    }
    None
}
