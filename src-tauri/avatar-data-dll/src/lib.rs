use std::collections::HashSet;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::ptr;
use std::sync::{Mutex, OnceLock};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod catalog;
mod paged;

pub use catalog::{
    write_catalog_index, AvatarItemRecord, AvatarLayerRecord, AvatarRoleRecord, AvatarSetDetail,
    AvatarSetRecord, AvatarVisualFamilyRecord, CatalogMetadata, CatalogPage, CatalogQueryEngine,
    CatalogSource, CatalogStatsRecord, ItemPageQuery, ItemQuery, SetPageQuery, SetQuery,
    CATALOG_PAGE_LIMIT, DEFAULT_PAGE_CACHE_BYTES, ITEM_RESULT_LIMIT, SET_RESULT_LIMIT,
    TARGET_PAGE_PLAINTEXT_BYTES,
};
pub use paged::{
    CompressionKind, PageCacheStats, PageDescriptor, PageType, PagedBuildReport, PagedDataBuilder,
    PagedDataStore,
};

pub const DATA_DLL_NAME: &str = "avatar_data.dll";
pub const DATA_SCHEMA_VERSION: u32 = 7;
pub const BUCKET_COUNT: usize = 512;

const KEY_SEED: &[u8] = b"DNF Avatar Studio encrypted avatar summary data key 2026-06-27";
const MAX_FFI_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_QUERY_CHARS: usize = 256;
pub const MAX_ITEM_IDS_PER_REQUEST: usize = 256;
pub const MAX_ITEM_ID_CHARS: usize = 128;
pub const MAX_SET_ID_CHARS: usize = 128;
pub const MAX_ROLE_CHARS: usize = 64;
pub const MAX_SLOT_CHARS: usize = 64;

static QUERY_ENGINE: OnceLock<Mutex<CatalogQueryEngine>> = OnceLock::new();
static QUERY_ENGINE_INIT_LOCK: Mutex<()> = Mutex::new(());
static LAST_ERROR: OnceLock<Mutex<CString>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct ItemIdsRequest {
    item_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SetDetailRequest {
    set_id: String,
    playable_role: String,
}

fn validate_text(
    value: &str,
    label: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), String> {
    let length = value.chars().count();
    if !allow_empty && value.trim().is_empty() {
        return Err(format!("avatar catalog {label} must not be empty"));
    }
    if length > max_chars {
        return Err(format!(
            "avatar catalog {label} exceeds the {max_chars} character limit"
        ));
    }
    Ok(())
}

fn validate_item_query(request: ItemQuery) -> Result<ItemQuery, String> {
    validate_text(&request.slot, "slot", MAX_SLOT_CHARS, true)?;
    validate_text(
        &request.playable_role,
        "playable role",
        MAX_ROLE_CHARS,
        true,
    )?;
    validate_text(&request.query, "query", MAX_QUERY_CHARS, true)?;
    Ok(request)
}

fn validate_set_query(request: SetQuery) -> Result<SetQuery, String> {
    validate_text(
        &request.playable_role,
        "playable role",
        MAX_ROLE_CHARS,
        true,
    )?;
    validate_text(&request.query, "query", MAX_QUERY_CHARS, true)?;
    Ok(request)
}

fn validate_page_limit(limit: usize) -> Result<(), String> {
    if !(1..=CATALOG_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "avatar catalog page limit must be between 1 and {CATALOG_PAGE_LIMIT}"
        ));
    }
    Ok(())
}

fn validate_item_page_query(request: ItemPageQuery) -> Result<ItemPageQuery, String> {
    validate_text(&request.slot, "slot", MAX_SLOT_CHARS, true)?;
    validate_text(
        &request.playable_role,
        "playable role",
        MAX_ROLE_CHARS,
        true,
    )?;
    validate_text(&request.query, "query", MAX_QUERY_CHARS, true)?;
    validate_page_limit(request.limit)?;
    Ok(request)
}

fn validate_set_page_query(request: SetPageQuery) -> Result<SetPageQuery, String> {
    validate_text(
        &request.playable_role,
        "playable role",
        MAX_ROLE_CHARS,
        true,
    )?;
    validate_text(&request.query, "query", MAX_QUERY_CHARS, true)?;
    validate_page_limit(request.limit)?;
    Ok(request)
}

fn validate_item_ids(mut request: ItemIdsRequest) -> Result<ItemIdsRequest, String> {
    if request.item_ids.len() > MAX_ITEM_IDS_PER_REQUEST {
        return Err(format!(
            "avatar catalog item ID request exceeds the {MAX_ITEM_IDS_PER_REQUEST} item limit"
        ));
    }
    let mut seen = HashSet::new();
    for item_id in &mut request.item_ids {
        *item_id = item_id.trim().to_string();
        validate_text(item_id, "item ID", MAX_ITEM_ID_CHARS, false)?;
    }
    request
        .item_ids
        .retain(|item_id| seen.insert(item_id.clone()));
    Ok(request)
}

fn validate_set_detail(mut request: SetDetailRequest) -> Result<SetDetailRequest, String> {
    request.set_id = request.set_id.trim().to_string();
    validate_text(&request.set_id, "set ID", MAX_SET_ID_CHARS, false)?;
    validate_text(
        &request.playable_role,
        "playable role",
        MAX_ROLE_CHARS,
        true,
    )?;
    Ok(request)
}

pub fn bucket_for_item_id(item_id: &str) -> usize {
    stable_bucket(item_id, BUCKET_COUNT)
}

pub fn key_bytes() -> [u8; 32] {
    Sha256::digest(KEY_SEED).into()
}

fn stable_bucket(value: &str, bucket_count: usize) -> usize {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash as usize) % bucket_count
}

fn data_dll_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("DNF_AVATAR_DATA_DLL") {
        return Ok(PathBuf::from(path));
    }
    let exe =
        std::env::current_exe().map_err(|err| format!("failed to locate current exe: {err}"))?;
    let parent = exe
        .parent()
        .ok_or_else(|| "failed to locate current exe parent".to_string())?;
    Ok(parent.join(DATA_DLL_NAME))
}

fn query_engine() -> Result<&'static Mutex<CatalogQueryEngine>, String> {
    if let Some(engine) = QUERY_ENGINE.get() {
        return Ok(engine);
    }
    let _guard = QUERY_ENGINE_INIT_LOCK
        .lock()
        .map_err(|_| "avatar catalog query initialization lock is poisoned".to_string())?;
    if let Some(engine) = QUERY_ENGINE.get() {
        return Ok(engine);
    }
    let path = data_dll_path()?;
    let engine = CatalogQueryEngine::open(&path, DEFAULT_PAGE_CACHE_BYTES)
        .map_err(|err| format!("failed to open avatar catalog {}: {err}", path.display()))?;
    QUERY_ENGINE
        .set(Mutex::new(engine))
        .map_err(|_| "avatar catalog query engine was initialized concurrently".to_string())?;
    QUERY_ENGINE
        .get()
        .ok_or_else(|| "avatar catalog query engine initialization failed".to_string())
}

fn with_query_engine<T>(
    operation: impl FnOnce(&mut CatalogQueryEngine) -> Result<T, String>,
) -> Result<T, String> {
    let engine = query_engine()?;
    let mut engine = engine
        .lock()
        .map_err(|_| "avatar catalog query lock is poisoned".to_string())?;
    operation(&mut engine)
}

fn set_last_error(message: String) {
    let error = LAST_ERROR.get_or_init(|| Mutex::new(CString::new("").expect("empty CString")));
    if let Ok(mut guard) = error.lock() {
        *guard = CString::new(message.replace('\0', " ")).unwrap_or_else(|_| {
            CString::new("failed to store last error").expect("static CString")
        });
    }
}

unsafe fn parse_request<T: DeserializeOwned>(request: *const c_char) -> Result<T, String> {
    if request.is_null() {
        return Err("invalid null request".to_string());
    }
    let request = unsafe { CStr::from_ptr(request) };
    let bytes = request.to_bytes();
    if bytes.len() > MAX_FFI_REQUEST_BYTES {
        return Err(format!(
            "avatar catalog request exceeds the {} byte limit",
            MAX_FFI_REQUEST_BYTES
        ));
    }
    serde_json::from_slice(bytes).map_err(|err| format!("invalid avatar catalog request: {err}"))
}

unsafe fn return_json<T: Serialize>(result: Result<T, String>, out_len: *mut usize) -> *mut u8 {
    if out_len.is_null() {
        set_last_error("invalid null output length".to_string());
        return ptr::null_mut();
    }
    unsafe {
        *out_len = 0;
    }
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            set_last_error(error);
            return ptr::null_mut();
        }
    };
    let bytes = match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(error) => {
            set_last_error(format!(
                "failed to serialize avatar catalog response: {error}"
            ));
            return ptr::null_mut();
        }
    };
    unsafe {
        *out_len = bytes.len();
    }
    let mut bytes = bytes.into_boxed_slice();
    let pointer = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    pointer
}

#[no_mangle]
pub extern "C" fn dnf_avatar_data_version() -> u32 {
    DATA_SCHEMA_VERSION
}

#[no_mangle]
/// Opens the authenticated catalog and returns its metadata as allocated UTF-8 JSON.
///
/// # Safety
///
/// `out_len` must be a valid writable pointer. The returned allocation must be released with
/// [`dnf_avatar_data_free`] using the exact returned length.
pub unsafe extern "C" fn dnf_avatar_data_open(out_len: *mut usize) -> *mut u8 {
    unsafe { return_json(with_query_engine(CatalogQueryEngine::metadata), out_len) }
}

#[no_mangle]
/// Runs a bounded item query and returns allocated UTF-8 JSON.
///
/// # Safety
///
/// `request` must point to a NUL-terminated UTF-8 JSON string and `out_len` must be writable. The
/// returned allocation must be released with [`dnf_avatar_data_free`].
pub unsafe extern "C" fn dnf_avatar_data_query_items(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<ItemQuery>(request) }
        .and_then(validate_item_query)
        .and_then(|request| with_query_engine(|engine| engine.query_items(&request)));
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
pub unsafe extern "C" fn dnf_avatar_data_query_items_page(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<ItemPageQuery>(request) }
        .and_then(validate_item_page_query)
        .and_then(|request| with_query_engine(|engine| engine.query_items_page(&request)));
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
/// Runs a bounded set query and returns allocated UTF-8 JSON.
///
/// # Safety
///
/// `request` must point to a NUL-terminated UTF-8 JSON string and `out_len` must be writable. The
/// returned allocation must be released with [`dnf_avatar_data_free`].
pub unsafe extern "C" fn dnf_avatar_data_query_sets(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<SetQuery>(request) }
        .and_then(validate_set_query)
        .and_then(|request| with_query_engine(|engine| engine.query_sets(&request)));
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
pub unsafe extern "C" fn dnf_avatar_data_query_sets_page(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<SetPageQuery>(request) }
        .and_then(validate_set_page_query)
        .and_then(|request| with_query_engine(|engine| engine.query_sets_page(&request)));
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
/// Reads authoritative item records for a bounded list of IDs.
///
/// # Safety
///
/// `request` must point to a NUL-terminated UTF-8 JSON string and `out_len` must be writable. The
/// returned allocation must be released with [`dnf_avatar_data_free`].
pub unsafe extern "C" fn dnf_avatar_data_get_items_by_ids(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<ItemIdsRequest>(request) }
        .and_then(validate_item_ids)
        .and_then(|request| with_query_engine(|engine| engine.get_items_by_ids(&request.item_ids)));
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
/// Reads the existing authoritative item records for a bounded list of IDs, skipping unknown IDs.
///
/// Authentication, schema, and index errors are still returned to the caller.
///
/// # Safety
///
/// `request` must point to a NUL-terminated UTF-8 JSON string and `out_len` must be writable. The
/// returned allocation must be released with [`dnf_avatar_data_free`].
pub unsafe extern "C" fn dnf_avatar_data_find_items_by_ids(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<ItemIdsRequest>(request) }
        .and_then(validate_item_ids)
        .and_then(|request| {
            with_query_engine(|engine| engine.find_items_by_ids(&request.item_ids))
        });
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
/// Reads one authoritative set and its role-projected member records.
///
/// # Safety
///
/// `request` must point to a NUL-terminated UTF-8 JSON string and `out_len` must be writable. The
/// returned allocation must be released with [`dnf_avatar_data_free`].
pub unsafe extern "C" fn dnf_avatar_data_get_set_with_members(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<SetDetailRequest>(request) }
        .and_then(validate_set_detail)
        .and_then(|request| {
            with_query_engine(|engine| {
                engine.get_set_with_members(&request.set_id, &request.playable_role)
            })
        });
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
/// Reads layer records for a bounded list of item IDs.
///
/// # Safety
///
/// `request` must point to a NUL-terminated UTF-8 JSON string and `out_len` must be writable. The
/// returned allocation must be released with [`dnf_avatar_data_free`].
pub unsafe extern "C" fn dnf_avatar_data_load_layers(
    request: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let result = unsafe { parse_request::<ItemIdsRequest>(request) }
        .and_then(validate_item_ids)
        .and_then(|request| with_query_engine(|engine| engine.load_layers(&request.item_ids)));
    unsafe { return_json(result, out_len) }
}

#[no_mangle]
/// Releases a response allocation returned by this library.
///
/// # Safety
///
/// `pointer` and `len` must be the exact pair returned by one of this library's query functions
/// and must not have been freed previously.
pub unsafe extern "C" fn dnf_avatar_data_free(pointer: *mut u8, len: usize) {
    if pointer.is_null() {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(pointer, len, len));
    }
}

#[no_mangle]
pub extern "C" fn dnf_avatar_data_last_error() -> *const c_char {
    let error = LAST_ERROR.get_or_init(|| Mutex::new(CString::new("").expect("empty CString")));
    match error.lock() {
        Ok(guard) => guard.as_ptr(),
        Err(_) => c"failed to lock last error".as_ptr(),
    }
}
