use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, RgbaImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{
    sort_avatar_layers_for_draw, visible_avatar_layers, AvatarLayer, PreviewImage,
    PreviewLayerRequest,
};
use crate::preview_frame::preview_start_frame_for_profession;

pub const THUMBNAIL_WIDTH: u32 = 128;
pub const THUMBNAIL_HEIGHT: u32 = 160;

const THUMBNAIL_MARGIN: u32 = 6;
const THUMBNAIL_SPEC_VERSION: u16 = 1;
const CACHE_SCHEMA_VERSION: u16 = 1;
const INDEX_MAGIC: &[u8; 8] = b"DNFTIDX1";
const RECORD_MAGIC: &[u8; 8] = b"DNFTHMB1";
const INDEX_AAD: &[u8] = b"dnf-avatar-thumbnail-index-v1";
const RECORD_AAD_PREFIX: &[u8] = b"dnf-avatar-thumbnail-record-v1:";
const MASTER_KEY_PURPOSE: &[u8] = b"dnf-avatar-studio-thumbnail-cache-v1";
const INDEX_NONCE_LEN: usize = 12;
const MAX_PROTECTED_KEY_BYTES: u64 = 64 * 1024;
const MAX_INDEX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_THUMBNAIL_BYTES: usize = 16 * 1024 * 1024;
const MAX_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThumbnailEntityKind {
    Set,
    Item,
}

impl ThumbnailEntityKind {
    fn key(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Item => "item",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThumbnailSkinSource {
    Explicit,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailExpectedMember {
    pub item_id: String,
    pub slot: String,
    pub expected_layer_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedExpectedMember {
    item_id: String,
    slots: BTreeSet<String>,
    expected_layer_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailRecipeInput {
    pub entity_kind: ThumbnailEntityKind,
    pub entity_id: String,
    pub name: String,
    pub playable_role_key: String,
    pub visual_family_key: String,
    pub catalog_build_id: String,
    pub default_skin_img: String,
    pub expected_members: Vec<ThumbnailExpectedMember>,
    pub layers: Vec<AvatarLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogThumbnailRecipe {
    pub entity_recipe_key: String,
    pub visual_signature: String,
    pub skin_source: ThumbnailSkinSource,
    pub layers: Vec<PreviewLayerRequest>,
}

#[derive(Serialize)]
struct VisualSignatureV1<'a> {
    spec_version: u16,
    visual_family_key: &'a str,
    layers: Vec<VisualLayerSignatureV1>,
}

#[derive(Serialize)]
struct VisualLayerSignatureV1 {
    img_path: String,
    resource_fingerprint: String,
    frame_index: usize,
    palette_index: usize,
}

pub(crate) fn thumbnail_render_profile_key(playable_role_key: &str) -> String {
    format!(
        "thumbnail-v{THUMBNAIL_SPEC_VERSION}-frame-{}",
        preview_start_frame_for_profession(playable_role_key),
    )
}

pub fn build_thumbnail_recipe(
    input: ThumbnailRecipeInput,
    mut resource_fingerprint: impl FnMut(&str) -> Result<String, String>,
) -> Result<CatalogThumbnailRecipe, String> {
    let entity_id = input.entity_id.trim();
    let role = input.playable_role_key.trim().to_ascii_lowercase();
    let visual_family = input.visual_family_key.trim().to_ascii_lowercase();
    let build_id = input.catalog_build_id.trim();
    if entity_id.is_empty() || role.is_empty() || role == "all" || visual_family.is_empty() {
        return Err("缩略图配方缺少具体实体、职业或视觉族".to_string());
    }
    let frame_index = preview_start_frame_for_profession(&role);

    let mut expected_members = BTreeMap::<String, NormalizedExpectedMember>::new();
    for mut member in input.expected_members {
        member.item_id = member.item_id.trim().to_string();
        member.slot = member.slot.trim().to_ascii_lowercase();
        if member.item_id.is_empty() || member.slot.is_empty() {
            return Err("缩略图配方包含缺少 ID 或部位的目录成员".to_string());
        }
        if let Some(existing) = expected_members.get_mut(&member.item_id) {
            if existing.expected_layer_count != member.expected_layer_count {
                return Err(format!(
                    "缩略图配方成员 {} 的视觉族图层数量声明不一致",
                    member.item_id
                ));
            }
            existing.slots.insert(member.slot);
            continue;
        }
        expected_members.insert(
            member.item_id.clone(),
            NormalizedExpectedMember {
                item_id: member.item_id,
                slots: BTreeSet::from([member.slot]),
                expected_layer_count: member.expected_layer_count,
            },
        );
    }
    if expected_members.is_empty() {
        return Err("缩略图配方没有目录声明的成员".to_string());
    }

    let mut layers = input.layers;
    let mut actual_layer_counts = BTreeMap::<String, usize>::new();
    for layer in &layers {
        let item_id = layer.item_id.trim();
        if !expected_members.contains_key(item_id) {
            return Err(format!("缩略图配方包含未声明的图层成员 {item_id}"));
        }
        *actual_layer_counts.entry(item_id.to_string()).or_default() += 1;
    }

    let explicit_skin_members = expected_members
        .values()
        .filter(|member| member.slots.contains("skin"))
        .collect::<Vec<_>>();
    if explicit_skin_members.len() > 1 {
        return Err("套装包含多个显式皮肤成员，无法确定唯一渲染配方".to_string());
    }

    let mut renderable_member_count = 0usize;
    for member in expected_members.values() {
        let actual_count = actual_layer_counts
            .get(&member.item_id)
            .copied()
            .unwrap_or(0);
        if member.expected_layer_count > 0 {
            renderable_member_count += 1;
        }
        if member.slots.contains("skin") && member.expected_layer_count == 0 {
            return Err(format!(
                "显式皮肤成员 {} 缺少当前视觉族图层，禁止回退默认皮肤",
                member.item_id
            ));
        }
        if input.entity_kind == ThumbnailEntityKind::Item && member.expected_layer_count == 0 {
            return Err(format!("单件 {} 缺少当前视觉族的必需图层", member.item_id));
        }
        if actual_count != member.expected_layer_count {
            return Err(format!(
                "成员 {} 的当前视觉族图层不完整：需要 {} 层，实际 {} 层",
                member.item_id, member.expected_layer_count, actual_count
            ));
        }
    }
    if renderable_member_count == 0 {
        return Err("缩略图配方没有可渲染的目录成员".to_string());
    }

    let skin_source = if explicit_skin_members.is_empty() {
        let default_skin = input.default_skin_img.trim();
        if default_skin.is_empty() {
            return Err("当前职业缺少默认皮肤资源".to_string());
        }
        layers.push(default_skin_layer(&visual_family, default_skin));
        ThumbnailSkinSource::Default
    } else {
        ThumbnailSkinSource::Explicit
    };

    let mut visible = visible_avatar_layers(&layers);
    sort_avatar_layers_for_draw(&mut visible);
    let mut requests = Vec::new();
    let mut signatures = Vec::new();
    for layer in visible {
        if layer.visual_img.trim().is_empty() {
            return Err(format!(
                "成员 {} 的可见图层缺少 IMG，禁止缓存残缺缩略图",
                layer.item_id
            ));
        }
        let img_path = normalize_img_path(&layer.visual_img);
        let fingerprint = resource_fingerprint(&img_path)?;
        requests.push(PreviewLayerRequest {
            img_path: img_path.clone(),
            frame_index,
            palette_index: layer.palette_index,
            // Normal catalog thumbnails never activate dye masks. Capability metadata must not
            // split otherwise identical final previews into separate visual signatures.
            enable_dye_mask1: false,
            enable_dye_mask2: false,
        });
        signatures.push(VisualLayerSignatureV1 {
            img_path,
            resource_fingerprint: fingerprint,
            frame_index,
            palette_index: layer.palette_index,
        });
    }
    if requests.is_empty() {
        return Err("缩略图配方没有可渲染的 IMG".to_string());
    }

    let signature_payload = serde_json::to_vec(&VisualSignatureV1 {
        spec_version: THUMBNAIL_SPEC_VERSION,
        visual_family_key: &visual_family,
        layers: signatures,
    })
    .map_err(|error| format!("序列化缩略图视觉签名失败：{error}"))?;
    let visual_signature = sha256_hex(&signature_payload);
    let expected_members_payload = expected_members
        .values()
        .map(|member| {
            format!(
                "{}:{}:{}",
                member.item_id,
                member.slots.iter().cloned().collect::<Vec<_>>().join(","),
                member.expected_layer_count
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    let entity_recipe_key = sha256_hex(
        format!(
            "thumbnail-entity-v1|{}|{}|{}|{}|{}|{}|{:?}",
            input.entity_kind.key(),
            entity_id,
            role,
            visual_family,
            build_id,
            expected_members_payload,
            skin_source,
        )
        .as_bytes(),
    );

    Ok(CatalogThumbnailRecipe {
        entity_recipe_key,
        visual_signature,
        skin_source,
        layers: requests,
    })
}

fn default_skin_layer(visual_family_key: &str, img_path: &str) -> AvatarLayer {
    AvatarLayer {
        item_id: "__thumbnail_default_skin__".to_string(),
        item_name: "默认皮肤".to_string(),
        slot: "skin".to_string(),
        visual_family_key: visual_family_key.to_string(),
        layer_order: -100,
        equipment_type_enum: 8,
        hide_equipment_type_enums: Vec::new(),
        enable_dye_mask1: false,
        enable_dye_mask2: false,
        layer_sequence: 0,
        layer_tag: "[skin]".to_string(),
        variation: "0".to_string(),
        palette_index: 0,
        visual_img: normalize_img_path(img_path),
        status: "default_skin".to_string(),
    }
}

fn normalize_img_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn normalize_thumbnail_webp(preview: &PreviewImage) -> Result<Vec<u8>, String> {
    if !preview.missing_imgs.is_empty() {
        return Err(format!(
            "缩略图资源不完整，缺少 {} 个 IMG",
            preview.missing_imgs.len()
        ));
    }
    if preview.width == 0 || preview.height == 0 || preview.width > 8192 || preview.height > 8192 {
        return Err("缩略图源画布尺寸无效".to_string());
    }
    let rgba = base64::engine::general_purpose::STANDARD
        .decode(&preview.rgba_base64)
        .map_err(|_| "缩略图源像素不是有效 Base64".to_string())?;
    let expected = usize::try_from(preview.width)
        .ok()
        .and_then(|width| {
            usize::try_from(preview.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "缩略图源像素尺寸溢出".to_string())?;
    if rgba.len() != expected {
        return Err("缩略图源像素长度与尺寸不一致".to_string());
    }
    let source = RgbaImage::from_raw(preview.width, preview.height, rgba)
        .ok_or_else(|| "无法创建缩略图源画布".to_string())?;
    let available_width = THUMBNAIL_WIDTH - THUMBNAIL_MARGIN * 2;
    let available_height = THUMBNAIL_HEIGHT - THUMBNAIL_MARGIN * 2;
    let scale = (available_width as f64 / preview.width as f64)
        .min(available_height as f64 / preview.height as f64);
    let target_width = ((preview.width as f64 * scale).round() as u32).clamp(1, available_width);
    let target_height = ((preview.height as f64 * scale).round() as u32).clamp(1, available_height);
    let resized = image::imageops::resize(
        &source,
        target_width,
        target_height,
        image::imageops::FilterType::Nearest,
    );
    let mut canvas = RgbaImage::new(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT);
    let x = i64::from((THUMBNAIL_WIDTH - target_width) / 2);
    let y = i64::from((THUMBNAIL_HEIGHT - target_height) / 2);
    image::imageops::overlay(&mut canvas, &resized, x, y);

    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .encode(
            canvas.as_raw(),
            THUMBNAIL_WIDTH,
            THUMBNAIL_HEIGHT,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("编码无损 WebP 缩略图失败：{error}"))?;
    Ok(webp)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheRetention {
    Automatic,
    Scope(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThumbnailCacheLookup {
    Missing,
    Stale,
    Ready(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThumbnailCacheSummary {
    pub entity_count: usize,
    pub blob_count: usize,
    pub encrypted_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntityRecord {
    visual_signature: String,
    automatic: bool,
    scopes: BTreeSet<String>,
    last_access_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheBlobRecord {
    segment_name: String,
    offset: u64,
    ciphertext_len: u64,
    plaintext_len: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheIndexV1 {
    schema_version: u16,
    next_segment_id: u64,
    entities: BTreeMap<String, CacheEntityRecord>,
    blobs: BTreeMap<String, CacheBlobRecord>,
}

impl Default for CacheIndexV1 {
    fn default() -> Self {
        Self {
            schema_version: CACHE_SCHEMA_VERSION,
            next_segment_id: 1,
            entities: BTreeMap::new(),
            blobs: BTreeMap::new(),
        }
    }
}

pub struct CatalogThumbnailCache {
    root: PathBuf,
    key: [u8; 32],
    index: CacheIndexV1,
    dirty: bool,
}

impl CatalogThumbnailCache {
    pub fn open_user_bound(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root)
            .map_err(|error| format!("创建缩略图缓存目录失败 {}：{error}", root.display()))?;
        let key_path = root.join("master.key");
        let key = if key_path.is_file() {
            let metadata = fs::metadata(&key_path)
                .map_err(|error| format!("读取缩略图缓存主密钥失败：{error}"))?;
            if metadata.len() == 0 || metadata.len() > MAX_PROTECTED_KEY_BYTES {
                return Err("缩略图缓存主密钥文件大小无效".to_string());
            }
            let protected = fs::read(&key_path)
                .map_err(|error| format!("读取缩略图缓存主密钥失败：{error}"))?;
            let plaintext = crate::local_protection::unprotect_for_current_user(
                &protected,
                MASTER_KEY_PURPOSE,
            )?;
            plaintext
                .try_into()
                .map_err(|_| "缩略图缓存主密钥长度无效".to_string())?
        } else {
            let generated = ChaCha20Poly1305::generate_key(&mut OsRng);
            let mut key = [0_u8; 32];
            key.copy_from_slice(&generated);
            let protected =
                crate::local_protection::protect_for_current_user(&key, MASTER_KEY_PURPOSE)?;
            let temporary = root.join("master.tmp");
            let mut file = File::create(&temporary)
                .map_err(|error| format!("创建缩略图缓存主密钥失败：{error}"))?;
            file.write_all(&protected)
                .and_then(|_| file.flush())
                .map_err(|error| format!("写入缩略图缓存主密钥失败：{error}"))?;
            file.sync_all()
                .map_err(|error| format!("同步缩略图缓存主密钥失败：{error}"))?;
            replace_cache_file(&key_path, &temporary)?;
            key
        };
        Self::open_with_key(root, key)
    }

    pub fn open_with_key(root: PathBuf, key: [u8; 32]) -> Result<Self, String> {
        fs::create_dir_all(root.join("segments"))
            .map_err(|error| format!("创建缩略图缓存目录失败 {}：{error}", root.display()))?;
        let index_path = root.join("index.bin");
        let (index, recovered_index) = if index_path.is_file() {
            match read_encrypted_index(&index_path, &key) {
                Ok(index) => (index, false),
                Err(error) if recoverable_index_error(&error) => {
                    quarantine_corrupted_index(&index_path)?;
                    (CacheIndexV1::default(), true)
                }
                Err(error) => return Err(error),
            }
        } else {
            (CacheIndexV1::default(), false)
        };
        if index.schema_version != CACHE_SCHEMA_VERSION {
            return Err(format!("缩略图缓存版本 {} 不受支持", index.schema_version));
        }
        let mut cache = Self {
            root,
            key,
            index,
            dirty: recovered_index,
        };
        cache.flush()?;
        Ok(cache)
    }

    pub fn put(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        webp: &[u8],
        retention: CacheRetention,
    ) -> Result<(), String> {
        self.put_inner(entity_recipe_key, visual_signature, webp, retention, true)
    }

    pub fn put_deferred(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        webp: &[u8],
        retention: CacheRetention,
    ) -> Result<(), String> {
        self.put_inner(entity_recipe_key, visual_signature, webp, retention, false)
    }

    fn put_inner(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        webp: &[u8],
        retention: CacheRetention,
        persist_immediately: bool,
    ) -> Result<(), String> {
        validate_cache_key(entity_recipe_key, "实体配方键")?;
        validate_cache_key(visual_signature, "视觉签名")?;
        validate_retention(&retention)?;
        if webp.is_empty() || webp.len() > MAX_THUMBNAIL_BYTES {
            return Err("缩略图缓存记录大小无效".to_string());
        }

        if !self.index.blobs.contains_key(visual_signature) {
            let blob = self.append_encrypted_blob(visual_signature, webp)?;
            self.index.blobs.insert(visual_signature.to_string(), blob);
            self.dirty = true;
        }
        let now = now_ms();
        let entity = self
            .index
            .entities
            .entry(entity_recipe_key.to_string())
            .or_insert_with(|| CacheEntityRecord {
                visual_signature: visual_signature.to_string(),
                automatic: false,
                scopes: BTreeSet::new(),
                last_access_ms: now,
            });
        if entity.visual_signature != visual_signature {
            entity.visual_signature = visual_signature.to_string();
            self.dirty = true;
        }
        entity.last_access_ms = now;
        if apply_retention(entity, retention)? {
            self.dirty = true;
        }
        if persist_immediately {
            self.flush()?;
        }
        Ok(())
    }

    pub fn link_existing(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        retention: CacheRetention,
    ) -> Result<bool, String> {
        self.link_existing_inner(entity_recipe_key, visual_signature, retention, true)
    }

    pub fn link_existing_deferred(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        retention: CacheRetention,
    ) -> Result<bool, String> {
        self.link_existing_inner(entity_recipe_key, visual_signature, retention, false)
    }

    fn link_existing_inner(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        retention: CacheRetention,
        persist_immediately: bool,
    ) -> Result<bool, String> {
        validate_cache_key(entity_recipe_key, "实体配方键")?;
        validate_cache_key(visual_signature, "视觉签名")?;
        validate_retention(&retention)?;
        if !self.index.blobs.contains_key(visual_signature) {
            return Ok(false);
        }
        let now = now_ms();
        let entity = self
            .index
            .entities
            .entry(entity_recipe_key.to_string())
            .or_insert_with(|| CacheEntityRecord {
                visual_signature: visual_signature.to_string(),
                automatic: false,
                scopes: BTreeSet::new(),
                last_access_ms: now,
            });
        if entity.visual_signature != visual_signature {
            entity.visual_signature = visual_signature.to_string();
            self.dirty = true;
        }
        entity.last_access_ms = now;
        if apply_retention(entity, retention)? {
            self.dirty = true;
        }
        if persist_immediately {
            self.flush()?;
        }
        Ok(true)
    }

    pub fn lookup(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
    ) -> Result<ThumbnailCacheLookup, String> {
        let Some(entity) = self.index.entities.get(entity_recipe_key) else {
            return Ok(ThumbnailCacheLookup::Missing);
        };
        if entity.visual_signature != visual_signature {
            return Ok(ThumbnailCacheLookup::Stale);
        }
        let Some(blob) = self.index.blobs.get(visual_signature).cloned() else {
            return Ok(ThumbnailCacheLookup::Missing);
        };
        let plaintext = match self.read_encrypted_blob(visual_signature, &blob) {
            Ok(plaintext) => plaintext,
            Err(error) if recoverable_blob_error(&error) => {
                self.invalidate_visual_signature(visual_signature)?;
                return Ok(ThumbnailCacheLookup::Missing);
            }
            Err(error) => return Err(error),
        };
        if plaintext.len() != usize::try_from(blob.plaintext_len).unwrap_or(usize::MAX) {
            self.invalidate_visual_signature(visual_signature)?;
            return Ok(ThumbnailCacheLookup::Missing);
        }
        if let Some(entity) = self.index.entities.get_mut(entity_recipe_key) {
            entity.last_access_ms = now_ms();
        }
        Ok(ThumbnailCacheLookup::Ready(plaintext))
    }

    pub fn lookup_retained(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        retention: CacheRetention,
    ) -> Result<ThumbnailCacheLookup, String> {
        self.lookup_retained_inner(entity_recipe_key, visual_signature, retention, true)
    }

    pub fn lookup_retained_deferred(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        retention: CacheRetention,
    ) -> Result<ThumbnailCacheLookup, String> {
        self.lookup_retained_inner(entity_recipe_key, visual_signature, retention, false)
    }

    fn lookup_retained_inner(
        &mut self,
        entity_recipe_key: &str,
        visual_signature: &str,
        retention: CacheRetention,
        persist_immediately: bool,
    ) -> Result<ThumbnailCacheLookup, String> {
        validate_cache_key(entity_recipe_key, "实体配方键")?;
        validate_cache_key(visual_signature, "视觉签名")?;
        validate_retention(&retention)?;
        let lookup = self.lookup(entity_recipe_key, visual_signature)?;
        if let ThumbnailCacheLookup::Ready(_) = lookup {
            let entity = self
                .index
                .entities
                .get_mut(entity_recipe_key)
                .ok_or_else(|| "缩略图缓存实体索引意外缺失".to_string())?;
            if apply_retention(entity, retention)? {
                self.dirty = true;
            }
            if persist_immediately {
                self.flush()?;
            }
            return Ok(lookup);
        }
        if !self.link_existing_inner(
            entity_recipe_key,
            visual_signature,
            retention,
            persist_immediately,
        )? {
            return Ok(lookup);
        }
        self.lookup(entity_recipe_key, visual_signature)
    }

    pub fn flush(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        self.persist_index()?;
        self.dirty = false;
        Ok(())
    }

    pub fn summary(&self) -> ThumbnailCacheSummary {
        let encrypted_bytes = directory_file_bytes(&self.root);
        ThumbnailCacheSummary {
            entity_count: self.index.entities.len(),
            blob_count: self.index.blobs.len(),
            encrypted_bytes,
        }
    }

    fn append_encrypted_blob(
        &mut self,
        visual_signature: &str,
        plaintext: &[u8],
    ) -> Result<CacheBlobRecord, String> {
        let cipher = cipher(&self.key)?;
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let aad = record_aad(visual_signature);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| "加密缩略图缓存记录失败".to_string())?;
        let segment_name = self.active_segment_name(ciphertext.len() as u64)?;
        let segment_path = self.root.join("segments").join(&segment_name);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&segment_path)
            .map_err(|error| {
                format!("打开缩略图缓存分段失败 {}：{error}", segment_path.display())
            })?;
        let offset = file
            .seek(SeekFrom::End(0))
            .map_err(|error| format!("定位缩略图缓存分段失败：{error}"))?;
        file.write_all(RECORD_MAGIC)
            .and_then(|_| file.write_all(nonce.as_slice()))
            .and_then(|_| file.write_all(&(ciphertext.len() as u64).to_le_bytes()))
            .and_then(|_| file.write_all(&ciphertext))
            .and_then(|_| file.flush())
            .map_err(|error| format!("写入缩略图缓存分段失败：{error}"))?;
        file.sync_data()
            .map_err(|error| format!("同步缩略图缓存分段失败：{error}"))?;
        Ok(CacheBlobRecord {
            segment_name,
            offset,
            ciphertext_len: ciphertext.len() as u64,
            plaintext_len: plaintext.len() as u64,
        })
    }

    fn active_segment_name(&mut self, ciphertext_len: u64) -> Result<String, String> {
        loop {
            let name = format!("{:06}.bin", self.index.next_segment_id);
            let path = self.root.join("segments").join(&name);
            let current_len = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            let record_len = u64::try_from(RECORD_MAGIC.len() + INDEX_NONCE_LEN + 8)
                .unwrap_or(28)
                .saturating_add(ciphertext_len);
            if current_len == 0 || current_len.saturating_add(record_len) <= MAX_SEGMENT_BYTES {
                return Ok(name);
            }
            self.index.next_segment_id = self.index.next_segment_id.saturating_add(1);
        }
    }

    fn read_encrypted_blob(
        &self,
        visual_signature: &str,
        blob: &CacheBlobRecord,
    ) -> Result<Vec<u8>, String> {
        if blob.ciphertext_len == 0 || blob.ciphertext_len > (MAX_THUMBNAIL_BYTES + 32) as u64 {
            return Err("缩略图缓存完整性校验失败：密文长度无效".to_string());
        }
        let path = self.root.join("segments").join(&blob.segment_name);
        let mut file = File::open(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "缩略图缓存完整性校验失败：缓存分段不存在".to_string()
            } else {
                format!("读取缩略图缓存分段失败 {}：{error}", path.display())
            }
        })?;
        file.seek(SeekFrom::Start(blob.offset))
            .map_err(|error| format!("定位缩略图缓存记录失败：{error}"))?;
        let mut magic = [0_u8; 8];
        let mut nonce_bytes = [0_u8; INDEX_NONCE_LEN];
        let mut length_bytes = [0_u8; 8];
        file.read_exact(&mut magic)
            .and_then(|_| file.read_exact(&mut nonce_bytes))
            .and_then(|_| file.read_exact(&mut length_bytes))
            .map_err(|_| "缩略图缓存完整性校验失败：记录被截断".to_string())?;
        if &magic != RECORD_MAGIC || u64::from_le_bytes(length_bytes) != blob.ciphertext_len {
            return Err("缩略图缓存完整性校验失败：记录头无效".to_string());
        }
        let mut ciphertext = vec![
            0_u8;
            usize::try_from(blob.ciphertext_len)
                .map_err(|_| "缩略图缓存密文过大".to_string())?
        ];
        file.read_exact(&mut ciphertext)
            .map_err(|_| "缩略图缓存完整性校验失败：密文被截断".to_string())?;
        cipher(&self.key)?
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &ciphertext,
                    aad: &record_aad(visual_signature),
                },
            )
            .map_err(|_| "缩略图缓存完整性校验失败：认证失败".to_string())
    }

    fn invalidate_visual_signature(&mut self, visual_signature: &str) -> Result<(), String> {
        let removed_blob = self.index.blobs.remove(visual_signature).is_some();
        let before = self.index.entities.len();
        self.index
            .entities
            .retain(|_, entity| entity.visual_signature != visual_signature);
        if removed_blob || self.index.entities.len() != before {
            self.dirty = true;
            self.flush()?;
        }
        Ok(())
    }

    fn persist_index(&self) -> Result<(), String> {
        let plaintext = postcard::to_allocvec(&self.index)
            .map_err(|error| format!("序列化缩略图缓存索引失败：{error}"))?;
        if plaintext.len() as u64 > MAX_INDEX_BYTES {
            return Err("缩略图缓存索引超过安全上限".to_string());
        }
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher(&self.key)?
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: INDEX_AAD,
                },
            )
            .map_err(|_| "加密缩略图缓存索引失败".to_string())?;
        let path = self.root.join("index.bin");
        let temporary = self.root.join("index.tmp");
        let mut file = File::create(&temporary)
            .map_err(|error| format!("创建缩略图缓存临时索引失败：{error}"))?;
        file.write_all(INDEX_MAGIC)
            .and_then(|_| file.write_all(nonce.as_slice()))
            .and_then(|_| file.write_all(&(ciphertext.len() as u64).to_le_bytes()))
            .and_then(|_| file.write_all(&ciphertext))
            .and_then(|_| file.flush())
            .map_err(|error| format!("写入缩略图缓存索引失败：{error}"))?;
        file.sync_all()
            .map_err(|error| format!("同步缩略图缓存索引失败：{error}"))?;
        replace_cache_file(&path, &temporary)
    }
}

fn apply_retention(
    entity: &mut CacheEntityRecord,
    retention: CacheRetention,
) -> Result<bool, String> {
    match retention {
        CacheRetention::Automatic => {
            let changed = !entity.automatic;
            entity.automatic = true;
            Ok(changed)
        }
        CacheRetention::Scope(scope) => {
            let scope = scope.trim();
            if scope.is_empty() || scope.len() > 256 {
                return Err("缩略图缓存范围无效".to_string());
            }
            Ok(entity.scopes.insert(scope.to_string()))
        }
    }
}

fn validate_retention(retention: &CacheRetention) -> Result<(), String> {
    if let CacheRetention::Scope(scope) = retention {
        let scope = scope.trim();
        if scope.is_empty() || scope.len() > 256 {
            return Err("缩略图缓存范围无效".to_string());
        }
    }
    Ok(())
}

fn read_encrypted_index(path: &Path, key: &[u8; 32]) -> Result<CacheIndexV1, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("读取缩略图缓存索引失败：{error}"))?;
    if metadata.len() < 28 || metadata.len() > MAX_INDEX_BYTES {
        return Err("缩略图缓存索引大小无效".to_string());
    }
    let mut file = File::open(path).map_err(|error| format!("打开缩略图缓存索引失败：{error}"))?;
    let mut magic = [0_u8; 8];
    let mut nonce_bytes = [0_u8; INDEX_NONCE_LEN];
    let mut length_bytes = [0_u8; 8];
    file.read_exact(&mut magic)
        .and_then(|_| file.read_exact(&mut nonce_bytes))
        .and_then(|_| file.read_exact(&mut length_bytes))
        .map_err(|_| "缩略图缓存索引完整性校验失败：索引被截断".to_string())?;
    if &magic != INDEX_MAGIC {
        return Err("缩略图缓存索引完整性校验失败：文件头无效".to_string());
    }
    let ciphertext_len = u64::from_le_bytes(length_bytes);
    if ciphertext_len == 0 || ciphertext_len > MAX_INDEX_BYTES {
        return Err("缩略图缓存索引完整性校验失败：密文长度无效".to_string());
    }
    let mut ciphertext = vec![
        0_u8;
        usize::try_from(ciphertext_len)
            .map_err(|_| "缩略图缓存索引密文过大".to_string())?
    ];
    file.read_exact(&mut ciphertext)
        .map_err(|_| "缩略图缓存索引完整性校验失败：密文被截断".to_string())?;
    let plaintext = cipher(key)?
        .decrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: &ciphertext,
                aad: INDEX_AAD,
            },
        )
        .map_err(|_| "缩略图缓存索引完整性校验失败：认证失败".to_string())?;
    postcard::from_bytes(&plaintext)
        .map_err(|_| "缩略图缓存索引完整性校验失败：内容无效".to_string())
}

fn recoverable_index_error(error: &str) -> bool {
    error.starts_with("缩略图缓存索引完整性校验失败") || error == "缩略图缓存索引大小无效"
}

fn recoverable_blob_error(error: &str) -> bool {
    error.starts_with("缩略图缓存完整性校验失败")
}

fn quarantine_corrupted_index(index_path: &Path) -> Result<(), String> {
    let root = index_path
        .parent()
        .ok_or_else(|| "缩略图缓存索引缺少父目录".to_string())?;
    for suffix in 0_u16..=u16::MAX {
        let candidate = root.join(format!("index.corrupt-{}-{suffix}.bin", now_ms()));
        if candidate.exists() {
            continue;
        }
        return fs::rename(index_path, &candidate).map_err(|error| {
            format!(
                "隔离损坏的缩略图缓存索引失败 {}：{error}",
                index_path.display()
            )
        });
    }
    Err("无法为损坏的缩略图缓存索引分配隔离文件名".to_string())
}

fn cipher(key: &[u8; 32]) -> Result<ChaCha20Poly1305, String> {
    ChaCha20Poly1305::new_from_slice(key).map_err(|_| "缩略图缓存主密钥长度无效".to_string())
}

fn record_aad(visual_signature: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(RECORD_AAD_PREFIX.len() + visual_signature.len());
    aad.extend_from_slice(RECORD_AAD_PREFIX);
    aad.extend_from_slice(visual_signature.as_bytes());
    aad
}

fn validate_cache_key(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b':' | b'_' | b'-')
        })
    {
        return Err(format!("{label}无效"));
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn directory_file_bytes(root: &Path) -> u64 {
    let mut total = fs::metadata(root.join("index.bin"))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if let Ok(entries) = fs::read_dir(root.join("segments")) {
        for entry in entries.flatten() {
            total =
                total.saturating_add(entry.metadata().map(|metadata| metadata.len()).unwrap_or(0));
        }
    }
    total
}

#[cfg(windows)]
fn replace_cache_file(path: &Path, temporary: &Path) -> Result<(), String> {
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
    let ok = unsafe { MoveFileExW(temporary.as_ptr(), path.as_ptr(), 0x1 | 0x8) };
    if ok == 0 {
        return Err("原子提交缩略图缓存索引失败".to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_cache_file(path: &Path, temporary: &Path) -> Result<(), String> {
    fs::rename(temporary, path).map_err(|error| format!("提交缩略图缓存索引失败：{error}"))
}
