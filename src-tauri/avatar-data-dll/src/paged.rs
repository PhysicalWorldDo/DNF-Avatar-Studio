use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{key_bytes, DATA_SCHEMA_VERSION};

const PAGED_MAGIC: &[u8; 16] = b"DNFAVATARDBv0002";
const PAGED_FOOTER_LEN: u64 = 68;
const AEAD_TAG_LEN: u64 = 16;
const MAX_ROOT_CIPHERTEXT_LEN: u64 = 64 * 1024 * 1024;
const MAX_PAGE_PLAINTEXT_LEN: usize = 64 * 1024 * 1024;
const ROOT_AAD_LABEL: &[u8] = b"dnf-avatar-paged-root-v2";
const PAGE_AAD_LABEL: &[u8] = b"dnf-avatar-paged-page-v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionKind {
    None,
    Lz4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageType {
    Metadata,
    ItemRecords,
    SetRecords,
    SetMembers,
    ItemIdIndex,
    SetIdIndex,
    ItemFilterIndex,
    SetFilterIndex,
    ItemSearchIndex,
    SetSearchIndex,
    LayerRecords,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageDescriptor {
    pub page_id: u64,
    pub page_type: PageType,
    pub logical_key: String,
    pub offset: u64,
    pub ciphertext_len: u64,
    pub stored_len: u64,
    pub plaintext_len: u64,
    pub compression: CompressionKind,
    pub nonce: [u8; 12],
    pub range_start: u32,
    pub range_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RootIndex {
    schema_version: u32,
    build_id: [u8; 16],
    pages: Vec<PageDescriptor>,
    collections: BTreeMap<String, Vec<u64>>,
}

#[derive(Debug, Clone)]
struct PendingPage {
    page_type: PageType,
    logical_key: String,
    compression: CompressionKind,
    range_start: u32,
    range_len: u32,
    plaintext: Vec<u8>,
}

#[derive(Debug)]
pub struct PagedDataBuilder {
    build_id: [u8; 16],
    pages: Vec<PendingPage>,
}

#[derive(Debug, Clone)]
pub struct PagedBuildReport {
    pub build_id: [u8; 16],
    pub pages: Vec<PageDescriptor>,
    pub root_offset: u64,
    pub root_len: u64,
    pub root_nonce: [u8; 12],
    pub page_nonces: Vec<[u8; 12]>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub resident_bytes: usize,
    pub resident_pages: usize,
}

#[derive(Debug)]
struct CacheEntry {
    bytes: Arc<[u8]>,
    last_used: u64,
}

#[derive(Debug)]
struct PageCache {
    budget_bytes: usize,
    resident_bytes: usize,
    clock: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    entries: HashMap<u64, CacheEntry>,
}

impl PageCache {
    fn new(budget_bytes: usize) -> Self {
        Self {
            budget_bytes,
            resident_bytes: 0,
            clock: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: HashMap::new(),
        }
    }

    fn get(&mut self, page_id: u64) -> Option<Arc<[u8]>> {
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&page_id) {
            entry.last_used = self.clock;
            self.hits = self.hits.saturating_add(1);
            return Some(Arc::clone(&entry.bytes));
        }
        self.misses = self.misses.saturating_add(1);
        None
    }

    fn insert(&mut self, page_id: u64, bytes: Arc<[u8]>) {
        let len = bytes.len();
        if self.budget_bytes == 0 || len > self.budget_bytes {
            return;
        }
        if let Some(previous) = self.entries.remove(&page_id) {
            self.resident_bytes = self.resident_bytes.saturating_sub(previous.bytes.len());
        }
        while self.resident_bytes.saturating_add(len) > self.budget_bytes {
            let Some((&oldest_id, _)) =
                self.entries.iter().min_by_key(|(_, entry)| entry.last_used)
            else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest_id) {
                self.resident_bytes = self.resident_bytes.saturating_sub(evicted.bytes.len());
                self.evictions = self.evictions.saturating_add(1);
            }
        }
        self.clock = self.clock.wrapping_add(1);
        self.resident_bytes = self.resident_bytes.saturating_add(len);
        self.entries.insert(
            page_id,
            CacheEntry {
                bytes,
                last_used: self.clock,
            },
        );
    }

    fn stats(&self) -> PageCacheStats {
        PageCacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            resident_bytes: self.resident_bytes,
            resident_pages: self.entries.len(),
        }
    }
}

#[derive(Debug)]
pub struct PagedDataStore {
    path: PathBuf,
    file: File,
    build_id: [u8; 16],
    root_offset: u64,
    pages: HashMap<u64, PageDescriptor>,
    collections: HashMap<String, Vec<u64>>,
    cache: PageCache,
}

#[derive(Debug, Clone)]
struct PagedFooter {
    schema_version: u32,
    root_offset: u64,
    root_len: u64,
    root_nonce: [u8; 12],
    build_id: [u8; 16],
}

impl PagedDataBuilder {
    pub fn new() -> Result<Self, String> {
        let mut build_id = [0u8; 16];
        OsRng.fill_bytes(&mut build_id);
        if build_id.iter().all(|byte| *byte == 0) {
            return Err("operating system returned an invalid empty build id".to_string());
        }
        Ok(Self {
            build_id,
            pages: Vec::new(),
        })
    }

    pub fn build_id(&self) -> [u8; 16] {
        self.build_id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_page(
        &mut self,
        logical_key: impl Into<String>,
        page_type: PageType,
        compression: CompressionKind,
        range_start: u32,
        range_len: u32,
        plaintext: Vec<u8>,
    ) -> Result<u64, String> {
        let logical_key = logical_key.into();
        if logical_key.trim().is_empty() || logical_key.as_bytes().contains(&0) {
            return Err("paged data logical key is empty or contains NUL".to_string());
        }
        if plaintext.len() > MAX_PAGE_PLAINTEXT_LEN {
            return Err(format!(
                "paged data page {logical_key} exceeds the {} byte plaintext limit",
                MAX_PAGE_PLAINTEXT_LEN
            ));
        }
        range_start
            .checked_add(range_len)
            .ok_or_else(|| format!("paged data range overflows for {logical_key}"))?;
        let page_id = u64::try_from(self.pages.len())
            .map_err(|_| "paged data page id overflow".to_string())?;
        self.pages.push(PendingPage {
            page_type,
            logical_key,
            compression,
            range_start,
            range_len,
            plaintext,
        });
        Ok(page_id)
    }

    pub fn write(
        self,
        base_dll: impl AsRef<Path>,
        output_dll: impl AsRef<Path>,
    ) -> Result<PagedBuildReport, String> {
        let base_dll = base_dll.as_ref();
        let output_dll = output_dll.as_ref();
        if base_dll == output_dll {
            return Err("base and output avatar data DLL paths must differ".to_string());
        }
        if let Some(parent) = output_dll.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        fs::copy(base_dll, output_dll).map_err(|err| {
            format!(
                "failed to copy {} to {}: {err}",
                base_dll.display(),
                output_dll.display()
            )
        })?;
        let result = self.append_to(output_dll);
        if result.is_err() {
            let _ = fs::remove_file(output_dll);
        }
        result
    }

    fn append_to(self, output_dll: &Path) -> Result<PagedBuildReport, String> {
        let mut out = fs::OpenOptions::new()
            .append(true)
            .read(true)
            .open(output_dll)
            .map_err(|err| format!("failed to open {}: {err}", output_dll.display()))?;
        let mut used_nonces = HashSet::with_capacity(self.pages.len() + 1);
        let mut descriptors = Vec::with_capacity(self.pages.len());
        let mut collections = BTreeMap::<String, Vec<u64>>::new();

        for (page_id, pending) in self.pages.into_iter().enumerate() {
            let page_id =
                u64::try_from(page_id).map_err(|_| "paged data page id overflow".to_string())?;
            let stored = match pending.compression {
                CompressionKind::None => pending.plaintext.clone(),
                CompressionKind::Lz4 => compress_prepend_size(&pending.plaintext),
            };
            let offset = out
                .seek(SeekFrom::End(0))
                .map_err(|err| format!("failed to seek output DLL: {err}"))?;
            let nonce = fresh_nonce(&mut used_nonces)?;
            let stored_len = u64::try_from(stored.len())
                .map_err(|_| format!("stored page length overflow for {}", pending.logical_key))?;
            let plaintext_len = u64::try_from(pending.plaintext.len()).map_err(|_| {
                format!("plaintext page length overflow for {}", pending.logical_key)
            })?;
            let descriptor = PageDescriptor {
                page_id,
                page_type: pending.page_type,
                logical_key: pending.logical_key.clone(),
                offset,
                ciphertext_len: stored_len
                    .checked_add(AEAD_TAG_LEN)
                    .ok_or_else(|| "paged data ciphertext length overflow".to_string())?,
                stored_len,
                plaintext_len,
                compression: pending.compression,
                nonce,
                range_start: pending.range_start,
                range_len: pending.range_len,
            };
            let ciphertext = encrypt_authenticated(
                &nonce,
                &page_aad(self.build_id, &descriptor),
                &stored,
                "page",
            )?;
            if u64::try_from(ciphertext.len()).ok() != Some(descriptor.ciphertext_len) {
                return Err(format!(
                    "encrypted page length mismatch for {}",
                    descriptor.logical_key
                ));
            }
            out.write_all(&ciphertext)
                .map_err(|err| format!("failed to write encrypted page: {err}"))?;
            collections
                .entry(descriptor.logical_key.clone())
                .or_default()
                .push(page_id);
            descriptors.push(descriptor);
        }

        let root = RootIndex {
            schema_version: DATA_SCHEMA_VERSION,
            build_id: self.build_id,
            pages: descriptors.clone(),
            collections,
        };
        let root_plaintext = postcard::to_allocvec(&root)
            .map_err(|err| format!("failed to serialize paged root index: {err}"))?;
        let root_nonce = fresh_nonce(&mut used_nonces)?;
        let root_ciphertext = encrypt_authenticated(
            &root_nonce,
            &root_aad(DATA_SCHEMA_VERSION, self.build_id),
            &root_plaintext,
            "root index",
        )?;
        let root_len = u64::try_from(root_ciphertext.len())
            .map_err(|_| "paged root index length overflow".to_string())?;
        if root_len > MAX_ROOT_CIPHERTEXT_LEN {
            return Err(format!(
                "paged root index exceeds the {} byte limit",
                MAX_ROOT_CIPHERTEXT_LEN
            ));
        }
        let root_offset = out
            .seek(SeekFrom::End(0))
            .map_err(|err| format!("failed to seek paged root index: {err}"))?;
        out.write_all(&root_ciphertext)
            .map_err(|err| format!("failed to write encrypted root index: {err}"))?;
        let footer = PagedFooter {
            schema_version: DATA_SCHEMA_VERSION,
            root_offset,
            root_len,
            root_nonce,
            build_id: self.build_id,
        };
        out.write_all(&paged_footer_bytes(&footer))
            .map_err(|err| format!("failed to write paged footer: {err}"))?;
        out.flush()
            .map_err(|err| format!("failed to flush paged data DLL: {err}"))?;

        let mut page_nonces = descriptors
            .iter()
            .map(|descriptor| descriptor.nonce)
            .collect::<Vec<_>>();
        page_nonces.push(root_nonce);
        Ok(PagedBuildReport {
            build_id: self.build_id,
            pages: descriptors,
            root_offset,
            root_len,
            root_nonce,
            page_nonces,
        })
    }
}

impl PagedDataStore {
    pub fn open(path: impl AsRef<Path>, cache_budget_bytes: usize) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)
            .map_err(|err| format!("failed to open paged data DLL {}: {err}", path.display()))?;
        let file_len = file
            .metadata()
            .map_err(|err| format!("failed to inspect paged data DLL {}: {err}", path.display()))?
            .len();
        let footer = read_paged_footer(&mut file, file_len)?;
        validate_root_bounds(&footer, file_len)?;
        let root_ciphertext =
            read_exact_at(&mut file, footer.root_offset, footer.root_len, "root index")?;
        let root_plaintext = decrypt_authenticated(
            &footer.root_nonce,
            &root_aad(footer.schema_version, footer.build_id),
            &root_ciphertext,
            "root index",
        )?;
        let root: RootIndex = postcard::from_bytes(&root_plaintext)
            .map_err(|err| format!("failed to parse authenticated paged root index: {err}"))?;
        validate_root(&root, &footer)?;

        let mut pages = HashMap::with_capacity(root.pages.len());
        for descriptor in root.pages {
            validate_page_descriptor(&descriptor, footer.root_offset)?;
            if pages.insert(descriptor.page_id, descriptor).is_some() {
                return Err("authenticated paged root contains a duplicate page id".to_string());
            }
        }
        let mut collections = HashMap::with_capacity(root.collections.len());
        for (name, page_ids) in root.collections {
            if name.trim().is_empty() || page_ids.iter().any(|page_id| !pages.contains_key(page_id))
            {
                return Err(format!(
                    "authenticated paged root contains an invalid collection {name:?}"
                ));
            }
            if collections.insert(name.clone(), page_ids).is_some() {
                return Err(format!(
                    "authenticated paged root contains duplicate collection {name}"
                ));
            }
        }

        Ok(Self {
            path,
            file,
            build_id: footer.build_id,
            root_offset: footer.root_offset,
            pages,
            collections,
            cache: PageCache::new(cache_budget_bytes),
        })
    }

    pub fn build_id(&self) -> [u8; 16] {
        self.build_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn collection_page_ids(&self, logical_key: &str) -> Result<&[u64], String> {
        self.collections
            .get(logical_key)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("authenticated paged collection not found: {logical_key}"))
    }

    pub fn page_descriptor(&self, page_id: u64) -> Result<&PageDescriptor, String> {
        self.pages
            .get(&page_id)
            .ok_or_else(|| format!("authenticated paged descriptor not found: {page_id}"))
    }

    pub fn read_first<T: DeserializeOwned>(
        &mut self,
        logical_key: &str,
        expected_type: PageType,
    ) -> Result<T, String> {
        let page_id = self
            .collection_page_ids(logical_key)?
            .first()
            .copied()
            .ok_or_else(|| format!("authenticated paged collection is empty: {logical_key}"))?;
        self.read_page(page_id, expected_type)
    }

    pub fn read_page<T: DeserializeOwned>(
        &mut self,
        page_id: u64,
        expected_type: PageType,
    ) -> Result<T, String> {
        let bytes = self.read_page_bytes(page_id, expected_type)?;
        postcard::from_bytes(&bytes)
            .map_err(|err| format!("failed to decode authenticated page {page_id}: {err}"))
    }

    pub fn read_page_bytes(
        &mut self,
        page_id: u64,
        expected_type: PageType,
    ) -> Result<Arc<[u8]>, String> {
        let descriptor = self.page_descriptor(page_id)?.clone();
        if descriptor.page_type != expected_type {
            return Err(format!(
                "authenticated page {page_id} type mismatch: expected {:?}, found {:?}",
                expected_type, descriptor.page_type
            ));
        }
        if let Some(bytes) = self.cache.get(page_id) {
            return Ok(bytes);
        }
        let ciphertext = read_exact_at(
            &mut self.file,
            descriptor.offset,
            descriptor.ciphertext_len,
            "data page",
        )?;
        let stored = decrypt_authenticated(
            &descriptor.nonce,
            &page_aad(self.build_id, &descriptor),
            &ciphertext,
            &format!("page {page_id}"),
        )?;
        if u64::try_from(stored.len()).ok() != Some(descriptor.stored_len) {
            return Err(format!(
                "authenticated page {page_id} stored length does not match its descriptor"
            ));
        }
        let plaintext = match descriptor.compression {
            CompressionKind::None => stored,
            CompressionKind::Lz4 => decompress_size_prepended(&stored).map_err(|err| {
                format!("failed to decompress authenticated page {page_id}: {err}")
            })?,
        };
        if u64::try_from(plaintext.len()).ok() != Some(descriptor.plaintext_len) {
            return Err(format!(
                "authenticated page {page_id} plaintext length does not match its descriptor"
            ));
        }
        let plaintext: Arc<[u8]> = plaintext.into();
        self.cache.insert(page_id, Arc::clone(&plaintext));
        Ok(plaintext)
    }

    pub fn verify_all_pages(&mut self) -> Result<usize, String> {
        let mut pages = self
            .pages
            .values()
            .map(|descriptor| (descriptor.page_id, descriptor.page_type))
            .collect::<Vec<_>>();
        pages.sort_unstable_by_key(|(page_id, _)| *page_id);
        for (page_id, page_type) in &pages {
            self.read_page_bytes(*page_id, *page_type)?;
        }
        Ok(pages.len())
    }

    pub fn cache_stats(&self) -> PageCacheStats {
        self.cache.stats()
    }

    pub fn root_offset(&self) -> u64 {
        self.root_offset
    }
}

fn fresh_nonce(used: &mut HashSet<[u8; 12]>) -> Result<[u8; 12], String> {
    for _ in 0..64 {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        if used.insert(nonce) {
            return Ok(nonce);
        }
    }
    Err("failed to obtain a unique random page nonce".to_string())
}

fn encrypt_authenticated(
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
    label: &str,
) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new((&key_bytes()).into());
    cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| format!("failed to encrypt authenticated {label}"))
}

fn decrypt_authenticated(
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
    label: &str,
) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new((&key_bytes()).into());
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| format!("failed to authenticate encrypted {label}"))
}

fn root_aad(schema_version: u32, build_id: [u8; 16]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(ROOT_AAD_LABEL.len() + 4 + build_id.len());
    aad.extend_from_slice(ROOT_AAD_LABEL);
    aad.extend_from_slice(&schema_version.to_le_bytes());
    aad.extend_from_slice(&build_id);
    aad
}

fn page_aad(build_id: [u8; 16], descriptor: &PageDescriptor) -> Vec<u8> {
    let mut aad = Vec::with_capacity(PAGE_AAD_LABEL.len() + descriptor.logical_key.len() + 96);
    aad.extend_from_slice(PAGE_AAD_LABEL);
    aad.extend_from_slice(&DATA_SCHEMA_VERSION.to_le_bytes());
    aad.extend_from_slice(&build_id);
    aad.extend_from_slice(&descriptor.page_id.to_le_bytes());
    aad.push(descriptor.page_type as u8);
    aad.push(descriptor.compression as u8);
    aad.extend_from_slice(&descriptor.offset.to_le_bytes());
    aad.extend_from_slice(&descriptor.ciphertext_len.to_le_bytes());
    aad.extend_from_slice(&descriptor.stored_len.to_le_bytes());
    aad.extend_from_slice(&descriptor.plaintext_len.to_le_bytes());
    aad.extend_from_slice(&descriptor.range_start.to_le_bytes());
    aad.extend_from_slice(&descriptor.range_len.to_le_bytes());
    aad.extend_from_slice(descriptor.logical_key.as_bytes());
    aad
}

fn paged_footer_bytes(footer: &PagedFooter) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(PAGED_FOOTER_LEN as usize);
    bytes.extend_from_slice(PAGED_MAGIC);
    bytes.extend_from_slice(&footer.schema_version.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&footer.root_offset.to_le_bytes());
    bytes.extend_from_slice(&footer.root_len.to_le_bytes());
    bytes.extend_from_slice(&footer.root_nonce);
    bytes.extend_from_slice(&footer.build_id);
    bytes
}

fn read_paged_footer(file: &mut File, file_len: u64) -> Result<PagedFooter, String> {
    if file_len < PAGED_FOOTER_LEN {
        return Err("paged data DLL footer is missing".to_string());
    }
    file.seek(SeekFrom::End(-(PAGED_FOOTER_LEN as i64)))
        .map_err(|err| format!("failed to seek paged data DLL footer: {err}"))?;
    let mut bytes = vec![0u8; PAGED_FOOTER_LEN as usize];
    file.read_exact(&mut bytes)
        .map_err(|err| format!("failed to read paged data DLL footer: {err}"))?;
    if &bytes[..16] != PAGED_MAGIC {
        return Err("paged data DLL magic does not match".to_string());
    }
    let schema_version = u32::from_le_bytes(bytes[16..20].try_into().expect("footer slice"));
    let flags = u32::from_le_bytes(bytes[20..24].try_into().expect("footer slice"));
    if flags != 0 {
        return Err(format!(
            "paged data DLL footer uses unsupported flags {flags}"
        ));
    }
    if schema_version != DATA_SCHEMA_VERSION {
        return Err(format!(
            "paged data DLL schema version {schema_version} does not match required version {DATA_SCHEMA_VERSION}"
        ));
    }
    let root_offset = u64::from_le_bytes(bytes[24..32].try_into().expect("footer slice"));
    let root_len = u64::from_le_bytes(bytes[32..40].try_into().expect("footer slice"));
    let mut root_nonce = [0u8; 12];
    root_nonce.copy_from_slice(&bytes[40..52]);
    let mut build_id = [0u8; 16];
    build_id.copy_from_slice(&bytes[52..68]);
    if build_id.iter().all(|byte| *byte == 0) {
        return Err("paged data DLL footer contains an empty build id".to_string());
    }
    Ok(PagedFooter {
        schema_version,
        root_offset,
        root_len,
        root_nonce,
        build_id,
    })
}

fn validate_root_bounds(footer: &PagedFooter, file_len: u64) -> Result<(), String> {
    if footer.root_len < AEAD_TAG_LEN || footer.root_len > MAX_ROOT_CIPHERTEXT_LEN {
        return Err("paged data DLL root length is invalid".to_string());
    }
    let footer_offset = file_len
        .checked_sub(PAGED_FOOTER_LEN)
        .ok_or_else(|| "paged data DLL footer offset underflow".to_string())?;
    let root_end = footer
        .root_offset
        .checked_add(footer.root_len)
        .ok_or_else(|| "paged data DLL root range overflows".to_string())?;
    if root_end != footer_offset {
        return Err("paged data DLL root range does not end at the footer".to_string());
    }
    Ok(())
}

fn validate_root(root: &RootIndex, footer: &PagedFooter) -> Result<(), String> {
    if root.schema_version != DATA_SCHEMA_VERSION {
        return Err(format!(
            "authenticated paged root schema {} does not match required version {}",
            root.schema_version, DATA_SCHEMA_VERSION
        ));
    }
    if root.build_id != footer.build_id {
        return Err("authenticated paged root build id does not match its footer".to_string());
    }
    Ok(())
}

fn validate_page_descriptor(descriptor: &PageDescriptor, root_offset: u64) -> Result<(), String> {
    if descriptor.logical_key.trim().is_empty()
        || descriptor.logical_key.as_bytes().contains(&0)
        || descriptor.ciphertext_len < AEAD_TAG_LEN
        || descriptor.ciphertext_len != descriptor.stored_len.saturating_add(AEAD_TAG_LEN)
        || descriptor.plaintext_len > MAX_PAGE_PLAINTEXT_LEN as u64
        || descriptor
            .range_start
            .checked_add(descriptor.range_len)
            .is_none()
    {
        return Err(format!(
            "authenticated paged descriptor {} is invalid",
            descriptor.page_id
        ));
    }
    let page_end = descriptor
        .offset
        .checked_add(descriptor.ciphertext_len)
        .ok_or_else(|| format!("authenticated page {} range overflows", descriptor.page_id))?;
    if page_end > root_offset {
        return Err(format!(
            "authenticated page {} overlaps the root index",
            descriptor.page_id
        ));
    }
    Ok(())
}

fn read_exact_at(file: &mut File, offset: u64, len: u64, label: &str) -> Result<Vec<u8>, String> {
    let len = usize::try_from(len).map_err(|_| format!("{label} length does not fit in memory"))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| format!("failed to seek {label}: {err}"))?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes)
        .map_err(|err| format!("failed to read {label}: {err}"))?;
    Ok(bytes)
}
