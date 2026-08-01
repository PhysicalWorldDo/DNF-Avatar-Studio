use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{
    bucket_for_item_id, CompressionKind, PageDescriptor, PageType, PagedBuildReport,
    PagedDataBuilder, PagedDataStore, DATA_SCHEMA_VERSION,
};

pub const ITEM_RESULT_LIMIT: usize = 800;
pub const SET_RESULT_LIMIT: usize = 500;
pub const CATALOG_PAGE_LIMIT: usize = 256;
pub const DEFAULT_PAGE_CACHE_BYTES: usize = 96 * 1024 * 1024;
pub const TARGET_PAGE_PLAINTEXT_BYTES: usize = 128 * 1024;

const ITEM_LOOKUP_SHARDS: usize = 256;
const SET_LOOKUP_SHARDS: usize = 128;
const SEARCH_SHARDS: usize = 32;
const MAX_SEARCH_TOKEN_CHARS: usize = 3;
const MAX_POSTING_ORDINALS_PER_ENTRY: usize = 24 * 1024;

const STUDIO_SLOTS: &[&str] = &[
    "skin", "cap", "hair", "face", "neck", "coat", "belt", "pants", "shoes", "weapon",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarLayerRecord {
    pub item_id: String,
    pub item_name: String,
    pub slot: String,
    pub visual_family_key: String,
    pub layer_order: i32,
    pub equipment_type_enum: u16,
    #[serde(default)]
    pub hide_equipment_type_enums: Vec<u16>,
    #[serde(default)]
    pub enable_dye_mask1: bool,
    #[serde(default)]
    pub enable_dye_mask2: bool,
    pub layer_sequence: u32,
    pub layer_tag: String,
    pub variation: String,
    pub palette_index: usize,
    pub visual_img: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarRoleRecord {
    pub playable_role_key: String,
    pub visual_family_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarVisualFamilyRecord {
    pub visual_family_key: String,
    pub resource_owner_key: String,
    pub resource_generation: String,
    pub default_skin_img: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarItemRecord {
    pub source_order: u32,
    pub item_id: String,
    pub item_name: String,
    pub slot: String,
    #[serde(default)]
    pub profession_dir: String,
    pub playable_roles: Vec<String>,
    #[serde(default)]
    pub path_family: String,
    pub resource_kind: String,
    pub icon_img: String,
    #[serde(default)]
    pub layer_count: usize,
    pub layer_counts_by_visual_family: HashMap<String, usize>,
    #[serde(default)]
    pub hide_equipment_types: Vec<String>,
    #[serde(default)]
    pub hide_equipment_type_enums: Vec<u16>,
    #[serde(default)]
    pub enable_dye_mask1: bool,
    #[serde(default)]
    pub enable_dye_mask2: bool,
    #[serde(default)]
    pub layers: Vec<AvatarLayerRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarSetRecord {
    pub source_order: u32,
    pub set_id: String,
    pub set_name: String,
    pub source: String,
    pub playable_roles: Vec<String>,
    pub member_ids: Vec<String>,
    pub member_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogStatsRecord {
    pub item_count: usize,
    pub set_count: usize,
    pub layer_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMetadata {
    pub stats: CatalogStatsRecord,
    pub roles: Vec<AvatarRoleRecord>,
    pub visual_families: Vec<AvatarVisualFamilyRecord>,
    pub item_result_limit: usize,
    pub set_result_limit: usize,
    pub schema_version: u32,
    pub build_id: String,
}

#[derive(Debug, Clone)]
pub struct CatalogSource {
    pub roles: Vec<AvatarRoleRecord>,
    pub visual_families: Vec<AvatarVisualFamilyRecord>,
    pub items: Vec<AvatarItemRecord>,
    pub sets: Vec<AvatarSetRecord>,
    pub layers_by_item: BTreeMap<String, Vec<AvatarLayerRecord>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemQuery {
    pub slot: String,
    pub playable_role: String,
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetQuery {
    pub playable_role: String,
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemPageQuery {
    pub slot: String,
    pub playable_role: String,
    pub query: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPageQuery {
    pub playable_role: String,
    pub query: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPage<T> {
    pub records: Vec<T>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarSetDetail {
    pub set: AvatarSetRecord,
    pub items: Vec<AvatarItemRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordPage<T> {
    records: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchPage {
    entries: Vec<SearchPosting>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchPosting {
    token: String,
    ordinals: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LayerItemRecord {
    item_id: String,
    layers: Vec<AvatarLayerRecord>,
}

#[derive(Debug)]
pub struct CatalogQueryEngine {
    store: PagedDataStore,
    metadata: CatalogMetadata,
    known_roles: HashSet<String>,
}

pub fn write_catalog_index(
    mut source: CatalogSource,
    base_dll: impl AsRef<Path>,
    output_dll: impl AsRef<Path>,
) -> Result<PagedBuildReport, String> {
    validate_catalog_source(&source)?;
    source.items.sort_by_key(|item| item.source_order);
    source.sets.sort_by_key(|set| set.source_order);

    let mut builder = PagedDataBuilder::new()?;
    let build_id = builder.build_id();
    let metadata = CatalogMetadata {
        stats: CatalogStatsRecord {
            item_count: source.items.len(),
            set_count: source.sets.len(),
            layer_count: source.layers_by_item.values().map(Vec::len).sum(),
        },
        roles: source.roles.clone(),
        visual_families: source.visual_families.clone(),
        item_result_limit: ITEM_RESULT_LIMIT,
        set_result_limit: SET_RESULT_LIMIT,
        schema_version: DATA_SCHEMA_VERSION,
        build_id: hex_build_id(build_id),
    };
    builder.add_page(
        "metadata",
        PageType::Metadata,
        CompressionKind::None,
        0,
        1,
        serialize_page(&metadata, "metadata")?,
    )?;

    add_item_lookup_pages(&mut builder, &source.items)?;
    add_set_lookup_pages(&mut builder, &source.sets)?;
    add_layer_pages(&mut builder, source.layers_by_item)?;
    add_item_filter_and_search_pages(&mut builder, &source.items)?;
    add_set_filter_and_search_pages(&mut builder, &source.sets)?;

    builder.write(base_dll, output_dll)
}

impl CatalogQueryEngine {
    pub fn open(path: impl AsRef<Path>, cache_budget_bytes: usize) -> Result<Self, String> {
        let mut store = PagedDataStore::open(path, cache_budget_bytes)?;
        let metadata: CatalogMetadata = store.read_first("metadata", PageType::Metadata)?;
        if metadata.schema_version != DATA_SCHEMA_VERSION {
            return Err(format!(
                "catalog metadata schema {} does not match required version {}",
                metadata.schema_version, DATA_SCHEMA_VERSION
            ));
        }
        if metadata.build_id != hex_build_id(store.build_id()) {
            return Err("catalog metadata build id does not match authenticated root".to_string());
        }
        if metadata.item_result_limit != ITEM_RESULT_LIMIT
            || metadata.set_result_limit != SET_RESULT_LIMIT
        {
            return Err(
                "catalog metadata result limits do not match the application contract".to_string(),
            );
        }
        let known_roles = metadata
            .roles
            .iter()
            .map(|role| normalize_key(&role.playable_role_key))
            .collect();
        Ok(Self {
            store,
            metadata,
            known_roles,
        })
    }

    pub fn metadata(&mut self) -> Result<CatalogMetadata, String> {
        Ok(self.metadata.clone())
    }

    pub fn query_items(&mut self, request: &ItemQuery) -> Result<Vec<AvatarItemRecord>, String> {
        let slot = normalize_slot(&request.slot)?;
        let role = self.normalize_role(&request.playable_role)?;
        let limit = request.limit.min(ITEM_RESULT_LIMIT);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let collection = item_filter_key(&role, &slot);
        let query = normalize_search_text(&request.query);
        if query.is_empty() {
            return self.read_partition_prefix(&collection, PageType::ItemFilterIndex, limit);
        }
        let ordinals = self.search_ordinals("item", &role, Some(&slot), &query)?;
        self.read_matching_partition(
            &collection,
            PageType::ItemFilterIndex,
            &ordinals,
            limit,
            |item: &AvatarItemRecord| item_matches_query(item, &query),
        )
    }

    pub fn query_sets(&mut self, request: &SetQuery) -> Result<Vec<AvatarSetRecord>, String> {
        let role = self.normalize_role(&request.playable_role)?;
        let limit = request.limit.min(SET_RESULT_LIMIT);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let collection = set_filter_key(&role);
        let query = normalize_search_text(&request.query);
        if query.is_empty() {
            return self.read_partition_prefix(&collection, PageType::SetFilterIndex, limit);
        }
        let ordinals = self.search_ordinals("set", &role, None, &query)?;
        self.read_matching_partition(
            &collection,
            PageType::SetFilterIndex,
            &ordinals,
            limit,
            |set: &AvatarSetRecord| set_matches_query(set, &query),
        )
    }

    pub fn query_items_page(
        &mut self,
        request: &ItemPageQuery,
    ) -> Result<CatalogPage<AvatarItemRecord>, String> {
        let slot = normalize_slot(&request.slot)?;
        let role = self.normalize_role(&request.playable_role)?;
        let collection = item_filter_key(&role, &slot);
        let query = normalize_search_text(&request.query);
        let limit = request.limit.min(CATALOG_PAGE_LIMIT);
        if query.is_empty() {
            return self.read_partition_page(
                &collection,
                PageType::ItemFilterIndex,
                request.offset,
                limit,
            );
        }
        let ordinals = self.search_ordinals("item", &role, Some(&slot), &query)?;
        self.read_matching_partition_page(
            &collection,
            PageType::ItemFilterIndex,
            &ordinals,
            request.offset,
            limit,
            |item: &AvatarItemRecord| item_matches_query(item, &query),
        )
    }

    pub fn query_sets_page(
        &mut self,
        request: &SetPageQuery,
    ) -> Result<CatalogPage<AvatarSetRecord>, String> {
        let role = self.normalize_role(&request.playable_role)?;
        let collection = set_filter_key(&role);
        let query = normalize_search_text(&request.query);
        let limit = request.limit.min(CATALOG_PAGE_LIMIT);
        if query.is_empty() {
            return self.read_partition_page(
                &collection,
                PageType::SetFilterIndex,
                request.offset,
                limit,
            );
        }
        let ordinals = self.search_ordinals("set", &role, None, &query)?;
        self.read_matching_partition_page(
            &collection,
            PageType::SetFilterIndex,
            &ordinals,
            request.offset,
            limit,
            |set: &AvatarSetRecord| set_matches_query(set, &query),
        )
    }

    pub fn get_items_by_ids(
        &mut self,
        item_ids: &[String],
    ) -> Result<Vec<AvatarItemRecord>, String> {
        let variants = self.get_item_variants_by_ids(item_ids)?;
        Ok(Self::first_items_by_ids(item_ids, variants))
    }

    pub fn find_items_by_ids(
        &mut self,
        item_ids: &[String],
    ) -> Result<Vec<AvatarItemRecord>, String> {
        let variants = self.find_item_variants_by_ids(item_ids)?;
        Ok(Self::first_items_by_ids(item_ids, variants))
    }

    fn first_items_by_ids(
        item_ids: &[String],
        variants: Vec<AvatarItemRecord>,
    ) -> Vec<AvatarItemRecord> {
        let mut first_by_id = HashMap::<String, AvatarItemRecord>::new();
        for record in variants {
            first_by_id.entry(record.item_id.clone()).or_insert(record);
        }
        item_ids
            .iter()
            .filter_map(|item_id| first_by_id.get(item_id).cloned())
            .collect()
    }

    pub fn get_set_with_members(
        &mut self,
        set_id: &str,
        playable_role: &str,
    ) -> Result<AvatarSetDetail, String> {
        let set = self.get_set_by_id(set_id)?;
        let effective_role = if normalize_key(playable_role) == "all" {
            let roles = normalized_roles(&set.playable_roles);
            if roles.len() != 1 {
                return Ok(AvatarSetDetail {
                    set,
                    items: Vec::new(),
                });
            }
            roles[0].clone()
        } else {
            let role = self.normalize_role(playable_role)?;
            if !normalized_roles(&set.playable_roles).contains(&role) {
                return Err(format!(
                    "catalog set {set_id} does not belong to role {role}"
                ));
            }
            role
        };
        let member_items = self.get_item_variants_by_ids(&set.member_ids)?;
        let items = member_items
            .into_iter()
            .filter(|item| normalized_roles(&item.playable_roles).contains(&effective_role))
            .collect();
        Ok(AvatarSetDetail { set, items })
    }

    pub fn load_layers(&mut self, item_ids: &[String]) -> Result<Vec<AvatarLayerRecord>, String> {
        if item_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let mut by_item = HashMap::<String, Vec<AvatarLayerRecord>>::new();
        let mut buckets = item_ids
            .iter()
            .map(|item_id| bucket_for_item_id(item_id))
            .collect::<Vec<_>>();
        buckets.sort_unstable();
        buckets.dedup();
        for bucket in buckets {
            let collection = layer_lookup_key(bucket);
            let records = self.read_all_records::<LayerItemRecord>(
                &collection,
                PageType::LayerRecords,
                true,
            )?;
            for record in records {
                if wanted.contains(record.item_id.as_str()) {
                    by_item.insert(record.item_id, record.layers);
                }
            }
        }
        let mut layers = Vec::new();
        let mut seen = HashSet::new();
        for item_id in item_ids {
            if !seen.insert(item_id.as_str()) {
                continue;
            }
            if let Some(item_layers) = by_item.remove(item_id) {
                layers.extend(item_layers);
            }
        }
        Ok(layers)
    }

    pub fn cache_stats(&self) -> crate::PageCacheStats {
        self.store.cache_stats()
    }

    fn get_set_by_id(&mut self, set_id: &str) -> Result<AvatarSetRecord, String> {
        let collection = set_lookup_key(stable_bucket(set_id, SET_LOOKUP_SHARDS));
        self.read_all_records::<AvatarSetRecord>(&collection, PageType::SetRecords, false)?
            .into_iter()
            .find(|set| set.set_id == set_id)
            .ok_or_else(|| format!("catalog set id is missing: {set_id}"))
    }

    fn get_item_variants_by_ids(
        &mut self,
        item_ids: &[String],
    ) -> Result<Vec<AvatarItemRecord>, String> {
        let records = self.find_item_variants_by_ids(item_ids)?;
        let found = records
            .iter()
            .map(|record| record.item_id.as_str())
            .collect::<HashSet<_>>();
        if let Some(missing) = item_ids
            .iter()
            .find(|item_id| !found.contains(item_id.as_str()))
        {
            return Err(format!("catalog item id is missing: {missing}"));
        }
        Ok(records)
    }

    fn find_item_variants_by_ids(
        &mut self,
        item_ids: &[String],
    ) -> Result<Vec<AvatarItemRecord>, String> {
        if item_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let mut shards = item_ids
            .iter()
            .map(|item_id| stable_bucket(item_id, ITEM_LOOKUP_SHARDS))
            .collect::<Vec<_>>();
        shards.sort_unstable();
        shards.dedup();

        let mut records = Vec::new();
        for shard in shards {
            let collection = item_lookup_key(shard);
            records.extend(
                self.read_all_records::<AvatarItemRecord>(
                    &collection,
                    PageType::ItemRecords,
                    true,
                )?
                .into_iter()
                .filter(|record| wanted.contains(record.item_id.as_str())),
            );
        }
        records.sort_by_key(|record| record.source_order);
        Ok(records)
    }

    fn normalize_role(&self, playable_role: &str) -> Result<String, String> {
        let role = normalize_key(playable_role);
        if role.is_empty() || role == "all" {
            return Ok("all".to_string());
        }
        if self.known_roles.contains(&role) {
            Ok(role)
        } else {
            Err(format!("unknown playable role: {playable_role}"))
        }
    }

    fn search_ordinals(
        &mut self,
        kind: &str,
        role: &str,
        slot: Option<&str>,
        query: &str,
    ) -> Result<Vec<u32>, String> {
        let tokens = query_tokens(query);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let mut postings = Vec::with_capacity(tokens.len());
        for token in tokens {
            let shard = stable_bucket(&token, SEARCH_SHARDS);
            let collection = search_key(kind, role, slot, shard);
            let Some(page_ids) = self
                .store
                .collection_page_ids(&collection)
                .ok()
                .map(<[_]>::to_vec)
            else {
                return Ok(Vec::new());
            };
            let mut ordinals = Vec::new();
            for page_id in page_ids {
                let page: SearchPage = self.store.read_page(page_id, search_page_type(kind)?)?;
                for entry in page.entries {
                    if entry.token == token {
                        ordinals.extend(entry.ordinals);
                    }
                }
            }
            ordinals.sort_unstable();
            ordinals.dedup();
            if ordinals.is_empty() {
                return Ok(Vec::new());
            }
            postings.push(ordinals);
        }
        postings.sort_by_key(Vec::len);
        let mut intersection = postings.remove(0);
        for posting in postings {
            intersection = intersect_sorted(&intersection, &posting);
            if intersection.is_empty() {
                break;
            }
        }
        Ok(intersection)
    }

    fn read_partition_prefix<T: DeserializeOwned>(
        &mut self,
        collection: &str,
        page_type: PageType,
        limit: usize,
    ) -> Result<Vec<T>, String> {
        let Some(page_ids) = self
            .store
            .collection_page_ids(collection)
            .ok()
            .map(<[_]>::to_vec)
        else {
            return Ok(Vec::new());
        };
        let mut results = Vec::with_capacity(limit);
        for page_id in page_ids {
            let descriptor = self.store.page_descriptor(page_id)?.clone();
            let page: RecordPage<T> = self.store.read_page(page_id, page_type)?;
            validate_record_page(&descriptor, page.records.len())?;
            let remaining = limit.saturating_sub(results.len());
            results.extend(page.records.into_iter().take(remaining));
            if results.len() == limit {
                break;
            }
        }
        Ok(results)
    }

    fn read_partition_page<T: DeserializeOwned>(
        &mut self,
        collection: &str,
        page_type: PageType,
        offset: usize,
        limit: usize,
    ) -> Result<CatalogPage<T>, String> {
        let Some(page_ids) = self
            .store
            .collection_page_ids(collection)
            .ok()
            .map(<[_]>::to_vec)
        else {
            return Ok(CatalogPage {
                records: Vec::new(),
                total: 0,
                next_offset: None,
            });
        };
        let total = page_ids.iter().try_fold(0usize, |total, page_id| {
            let descriptor = self.store.page_descriptor(*page_id)?;
            let page_len = usize::try_from(descriptor.range_len)
                .map_err(|_| "catalog page length does not fit in memory".to_string())?;
            total
                .checked_add(page_len)
                .ok_or_else(|| "catalog result count overflows".to_string())
        })?;
        let requested_end = offset.saturating_add(limit).min(total);
        let mut records = Vec::with_capacity(requested_end.saturating_sub(offset));
        if limit > 0 && offset < total {
            for page_id in page_ids {
                let descriptor = self.store.page_descriptor(page_id)?.clone();
                let page_start = usize::try_from(descriptor.range_start)
                    .map_err(|_| "catalog page offset does not fit in memory".to_string())?;
                let page_len = usize::try_from(descriptor.range_len)
                    .map_err(|_| "catalog page length does not fit in memory".to_string())?;
                let page_end = page_start
                    .checked_add(page_len)
                    .ok_or_else(|| "catalog page range overflows".to_string())?;
                if page_end <= offset || page_start >= requested_end {
                    continue;
                }
                let page: RecordPage<T> = self.store.read_page(page_id, page_type)?;
                validate_record_page(&descriptor, page.records.len())?;
                let local_start = offset.saturating_sub(page_start);
                let local_end = requested_end.min(page_end) - page_start;
                records.extend(
                    page.records
                        .into_iter()
                        .skip(local_start)
                        .take(local_end - local_start),
                );
            }
        }
        Ok(CatalogPage {
            next_offset: next_catalog_page_offset(offset, records.len(), total),
            records,
            total,
        })
    }

    fn read_matching_partition_page<T, F>(
        &mut self,
        collection: &str,
        page_type: PageType,
        ordinals: &[u32],
        offset: usize,
        limit: usize,
        matches: F,
    ) -> Result<CatalogPage<T>, String>
    where
        T: Clone + DeserializeOwned,
        F: Fn(&T) -> bool,
    {
        if ordinals.is_empty() {
            return Ok(CatalogPage {
                records: Vec::new(),
                total: 0,
                next_offset: None,
            });
        }
        let Some(page_ids) = self
            .store
            .collection_page_ids(collection)
            .ok()
            .map(<[_]>::to_vec)
        else {
            return Ok(CatalogPage {
                records: Vec::new(),
                total: 0,
                next_offset: None,
            });
        };
        let requested_end = offset.saturating_add(limit);
        let mut total = 0usize;
        let mut records = Vec::with_capacity(limit);
        for page_id in page_ids {
            let descriptor = self.store.page_descriptor(page_id)?.clone();
            let range_end = descriptor
                .range_start
                .checked_add(descriptor.range_len)
                .ok_or_else(|| format!("page {} range overflows", descriptor.page_id))?;
            let start_index = ordinals.partition_point(|ordinal| *ordinal < descriptor.range_start);
            let end_index = ordinals.partition_point(|ordinal| *ordinal < range_end);
            if start_index == end_index {
                continue;
            }
            let page: RecordPage<T> = self.store.read_page(page_id, page_type)?;
            validate_record_page(&descriptor, page.records.len())?;
            for ordinal in &ordinals[start_index..end_index] {
                let local = usize::try_from(*ordinal - descriptor.range_start)
                    .map_err(|_| "catalog ordinal does not fit in memory".to_string())?;
                let record = page.records.get(local).ok_or_else(|| {
                    format!(
                        "catalog ordinal {ordinal} is outside authenticated page {}",
                        descriptor.page_id
                    )
                })?;
                if !matches(record) {
                    continue;
                }
                if total >= offset && total < requested_end {
                    records.push(record.clone());
                }
                total = total
                    .checked_add(1)
                    .ok_or_else(|| "catalog result count overflows".to_string())?;
            }
        }
        Ok(CatalogPage {
            next_offset: next_catalog_page_offset(offset, records.len(), total),
            records,
            total,
        })
    }

    fn read_matching_partition<T, F>(
        &mut self,
        collection: &str,
        page_type: PageType,
        ordinals: &[u32],
        limit: usize,
        matches: F,
    ) -> Result<Vec<T>, String>
    where
        T: Clone + DeserializeOwned,
        F: Fn(&T) -> bool,
    {
        if ordinals.is_empty() {
            return Ok(Vec::new());
        }
        let Some(page_ids) = self
            .store
            .collection_page_ids(collection)
            .ok()
            .map(<[_]>::to_vec)
        else {
            return Ok(Vec::new());
        };
        let mut results = Vec::with_capacity(limit);
        for page_id in page_ids {
            let descriptor = self.store.page_descriptor(page_id)?.clone();
            let range_end = descriptor
                .range_start
                .checked_add(descriptor.range_len)
                .ok_or_else(|| format!("page {} range overflows", descriptor.page_id))?;
            let start_index = ordinals.partition_point(|ordinal| *ordinal < descriptor.range_start);
            let end_index = ordinals.partition_point(|ordinal| *ordinal < range_end);
            if start_index == end_index {
                continue;
            }
            let page: RecordPage<T> = self.store.read_page(page_id, page_type)?;
            validate_record_page(&descriptor, page.records.len())?;
            for ordinal in &ordinals[start_index..end_index] {
                let local = usize::try_from(*ordinal - descriptor.range_start)
                    .map_err(|_| "catalog ordinal does not fit in memory".to_string())?;
                let record = page.records.get(local).ok_or_else(|| {
                    format!(
                        "catalog ordinal {ordinal} is outside authenticated page {}",
                        descriptor.page_id
                    )
                })?;
                if matches(record) {
                    results.push(record.clone());
                    if results.len() == limit {
                        break;
                    }
                }
            }
            if results.len() == limit {
                break;
            }
        }
        Ok(results)
    }

    fn read_all_records<T: DeserializeOwned>(
        &mut self,
        collection: &str,
        page_type: PageType,
        missing_is_empty: bool,
    ) -> Result<Vec<T>, String> {
        let page_ids = match self.store.collection_page_ids(collection) {
            Ok(page_ids) => page_ids.to_vec(),
            Err(_) if missing_is_empty => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut records = Vec::new();
        for page_id in page_ids {
            let descriptor = self.store.page_descriptor(page_id)?.clone();
            let page: RecordPage<T> = self.store.read_page(page_id, page_type)?;
            validate_record_page(&descriptor, page.records.len())?;
            records.extend(page.records);
        }
        Ok(records)
    }
}

fn validate_catalog_source(source: &CatalogSource) -> Result<(), String> {
    if source.roles.is_empty() || source.visual_families.is_empty() {
        return Err("catalog source must contain roles and visual families".to_string());
    }
    let families = source
        .visual_families
        .iter()
        .map(|family| normalize_key(&family.visual_family_key))
        .collect::<HashSet<_>>();
    if families.len() != source.visual_families.len() || families.contains("") {
        return Err("catalog source contains duplicate or empty visual families".to_string());
    }
    let mut roles = HashSet::new();
    for role in &source.roles {
        let role_key = normalize_key(&role.playable_role_key);
        if role_key.is_empty()
            || role_key == "all"
            || !roles.insert(role_key)
            || !families.contains(&normalize_key(&role.visual_family_key))
        {
            return Err("catalog source contains duplicate, empty, or unbound roles".to_string());
        }
    }
    validate_item_records(
        source
            .items
            .iter()
            .map(|item| (item.item_id.as_str(), item.source_order)),
    )?;
    validate_unique_records(
        source
            .sets
            .iter()
            .map(|set| (set.set_id.as_str(), set.source_order)),
        "set",
    )?;
    let item_ids = source
        .items
        .iter()
        .map(|item| item.item_id.as_str())
        .collect::<HashSet<_>>();
    for item in &source.items {
        if item.item_name.trim().is_empty()
            || item
                .playable_roles
                .iter()
                .map(|role| normalize_key(role))
                .any(|role| !roles.contains(&role))
        {
            return Err(format!(
                "catalog item {} has an empty name or unbound role",
                item.item_id
            ));
        }
    }
    for set in &source.sets {
        if set.set_name.trim().is_empty()
            || set
                .playable_roles
                .iter()
                .map(|role| normalize_key(role))
                .any(|role| !roles.contains(&role))
            || set
                .member_ids
                .iter()
                .any(|member_id| !item_ids.contains(member_id.as_str()))
        {
            return Err(format!(
                "catalog set {} has an empty name, unbound role, or missing member",
                set.set_id
            ));
        }
    }
    for (item_id, layers) in &source.layers_by_item {
        if !item_ids.contains(item_id.as_str())
            || layers.iter().any(|layer| layer.item_id != *item_id)
        {
            return Err(format!(
                "catalog layer collection {item_id} is missing its item or contains a foreign layer"
            ));
        }
    }
    Ok(())
}

fn validate_item_records<'a>(records: impl Iterator<Item = (&'a str, u32)>) -> Result<(), String> {
    let mut orders = HashSet::new();
    for (id, order) in records {
        if id.trim().is_empty() || !orders.insert(order) {
            return Err(
                "catalog source contains an empty item id or duplicate item source_order"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn validate_unique_records<'a>(
    records: impl Iterator<Item = (&'a str, u32)>,
    kind: &str,
) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut orders = HashSet::new();
    for (id, order) in records {
        if id.trim().is_empty() || !ids.insert(id) || !orders.insert(order) {
            return Err(format!(
                "catalog source contains an empty or duplicate {kind} id/source_order"
            ));
        }
    }
    Ok(())
}

fn add_item_lookup_pages(
    builder: &mut PagedDataBuilder,
    items: &[AvatarItemRecord],
) -> Result<(), String> {
    let mut shards = vec![Vec::new(); ITEM_LOOKUP_SHARDS];
    for item in items {
        shards[stable_bucket(&item.item_id, ITEM_LOOKUP_SHARDS)].push(item.clone());
    }
    for (shard, records) in shards.into_iter().enumerate() {
        if !records.is_empty() {
            add_record_pages(
                builder,
                item_lookup_key(shard),
                PageType::ItemRecords,
                CompressionKind::None,
                records,
            )?;
        }
    }
    Ok(())
}

fn add_set_lookup_pages(
    builder: &mut PagedDataBuilder,
    sets: &[AvatarSetRecord],
) -> Result<(), String> {
    let mut shards = vec![Vec::new(); SET_LOOKUP_SHARDS];
    for set in sets {
        shards[stable_bucket(&set.set_id, SET_LOOKUP_SHARDS)].push(set.clone());
    }
    for (shard, records) in shards.into_iter().enumerate() {
        if !records.is_empty() {
            add_record_pages(
                builder,
                set_lookup_key(shard),
                PageType::SetRecords,
                CompressionKind::None,
                records,
            )?;
        }
    }
    Ok(())
}

fn add_layer_pages(
    builder: &mut PagedDataBuilder,
    layers_by_item: BTreeMap<String, Vec<AvatarLayerRecord>>,
) -> Result<(), String> {
    let mut buckets = vec![Vec::new(); crate::BUCKET_COUNT];
    for (item_id, layers) in layers_by_item {
        buckets[bucket_for_item_id(&item_id)].push(LayerItemRecord { item_id, layers });
    }
    for (bucket, records) in buckets.into_iter().enumerate() {
        if !records.is_empty() {
            add_record_pages(
                builder,
                layer_lookup_key(bucket),
                PageType::LayerRecords,
                CompressionKind::Lz4,
                records,
            )?;
        }
    }
    Ok(())
}

fn add_item_filter_and_search_pages(
    builder: &mut PagedDataBuilder,
    items: &[AvatarItemRecord],
) -> Result<(), String> {
    let mut roles = BTreeSet::from(["all".to_string()]);
    for item in items {
        roles.extend(normalized_roles(&item.playable_roles));
    }
    let slots = std::iter::once("all")
        .chain(STUDIO_SLOTS.iter().copied())
        .collect::<Vec<_>>();
    for role in roles {
        for slot in &slots {
            let records = items
                .iter()
                .filter(|item| {
                    (role == "all" || normalized_roles(&item.playable_roles).contains(&role))
                        && (*slot == "all" || studio_slot_for_item(item).as_deref() == Some(*slot))
                })
                .cloned()
                .collect::<Vec<_>>();
            if records.is_empty() {
                continue;
            }
            let fields = records
                .iter()
                .map(|item| {
                    vec![
                        item.item_name.clone(),
                        slot_label(studio_slot_for_item(item).as_deref().unwrap_or(&item.slot))
                            .to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            add_record_pages(
                builder,
                item_filter_key(&role, slot),
                PageType::ItemFilterIndex,
                CompressionKind::None,
                records,
            )?;
            add_search_pages(builder, "item", &role, Some(slot), &fields)?;
        }
    }
    Ok(())
}

fn add_set_filter_and_search_pages(
    builder: &mut PagedDataBuilder,
    sets: &[AvatarSetRecord],
) -> Result<(), String> {
    let mut roles = BTreeSet::from(["all".to_string()]);
    for set in sets {
        roles.extend(normalized_roles(&set.playable_roles));
    }
    for role in roles {
        let records = sets
            .iter()
            .filter(|set| role == "all" || normalized_roles(&set.playable_roles).contains(&role))
            .cloned()
            .collect::<Vec<_>>();
        if records.is_empty() {
            continue;
        }
        let fields = records
            .iter()
            .map(|set| {
                let mut fields = Vec::with_capacity(set.member_names.len() + 1);
                fields.push(set.set_name.clone());
                fields.extend(set.member_names.iter().cloned());
                fields
            })
            .collect::<Vec<_>>();
        add_record_pages(
            builder,
            set_filter_key(&role),
            PageType::SetFilterIndex,
            CompressionKind::None,
            records,
        )?;
        add_search_pages(builder, "set", &role, None, &fields)?;
    }
    Ok(())
}

fn add_record_pages<T: Clone + Serialize>(
    builder: &mut PagedDataBuilder,
    logical_key: String,
    page_type: PageType,
    compression: CompressionKind,
    records: Vec<T>,
) -> Result<(), String> {
    let mut page_records = Vec::new();
    let mut estimated_bytes = 16usize;
    let mut range_start = 0u32;
    for record in records {
        let record_size = postcard::to_allocvec(&record)
            .map_err(|err| format!("failed to size record for {logical_key}: {err}"))?
            .len()
            .saturating_add(8);
        if !page_records.is_empty()
            && estimated_bytes.saturating_add(record_size) > TARGET_PAGE_PLAINTEXT_BYTES
        {
            let page_record_count = page_records.len();
            write_record_page(
                builder,
                &logical_key,
                page_type,
                compression,
                range_start,
                std::mem::take(&mut page_records),
            )?;
            range_start = range_start
                .checked_add(
                    u32::try_from(page_record_count)
                        .map_err(|_| "catalog record page range overflow".to_string())?,
                )
                .ok_or_else(|| "catalog record page range overflow".to_string())?;
            estimated_bytes = 16;
        }
        estimated_bytes = estimated_bytes.saturating_add(record_size);
        page_records.push(record);
    }
    if !page_records.is_empty() {
        write_record_page(
            builder,
            &logical_key,
            page_type,
            compression,
            range_start,
            page_records,
        )?;
    }
    Ok(())
}

fn write_record_page<T: Serialize>(
    builder: &mut PagedDataBuilder,
    logical_key: &str,
    page_type: PageType,
    compression: CompressionKind,
    range_start: u32,
    records: Vec<T>,
) -> Result<(), String> {
    let range_len = u32::try_from(records.len())
        .map_err(|_| format!("record count overflow for {logical_key}"))?;
    let page = RecordPage { records };
    builder.add_page(
        logical_key,
        page_type,
        compression,
        range_start,
        range_len,
        serialize_page(&page, logical_key)?,
    )?;
    Ok(())
}

fn add_search_pages(
    builder: &mut PagedDataBuilder,
    kind: &str,
    role: &str,
    slot: Option<&str>,
    fields_by_ordinal: &[Vec<String>],
) -> Result<(), String> {
    let mut shards = vec![BTreeMap::<String, Vec<u32>>::new(); SEARCH_SHARDS];
    for (ordinal, fields) in fields_by_ordinal.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| "search ordinal overflow".to_string())?;
        let mut tokens = BTreeSet::new();
        for field in fields {
            tokens.extend(index_tokens(&normalize_search_text(field)));
        }
        for token in tokens {
            shards[stable_bucket(&token, SEARCH_SHARDS)]
                .entry(token)
                .or_default()
                .push(ordinal);
        }
    }
    for (shard, postings) in shards.into_iter().enumerate() {
        if postings.is_empty() {
            continue;
        }
        let logical_key = search_key(kind, role, slot, shard);
        let mut entries = Vec::new();
        for (token, ordinals) in postings {
            for segment in ordinals.chunks(MAX_POSTING_ORDINALS_PER_ENTRY) {
                entries.push(SearchPosting {
                    token: token.clone(),
                    ordinals: segment.to_vec(),
                });
            }
        }
        add_search_entry_pages(builder, logical_key, search_page_type(kind)?, entries)?;
    }
    Ok(())
}

fn add_search_entry_pages(
    builder: &mut PagedDataBuilder,
    logical_key: String,
    page_type: PageType,
    entries: Vec<SearchPosting>,
) -> Result<(), String> {
    let mut page_entries = Vec::new();
    let mut estimated_bytes = 16usize;
    for entry in entries {
        let entry_size = entry
            .token
            .len()
            .saturating_add(entry.ordinals.len().saturating_mul(5))
            .saturating_add(16);
        if !page_entries.is_empty()
            && estimated_bytes.saturating_add(entry_size) > TARGET_PAGE_PLAINTEXT_BYTES
        {
            let page = SearchPage {
                entries: std::mem::take(&mut page_entries),
            };
            builder.add_page(
                &logical_key,
                page_type,
                CompressionKind::None,
                0,
                0,
                serialize_page(&page, &logical_key)?,
            )?;
            estimated_bytes = 16;
        }
        estimated_bytes = estimated_bytes.saturating_add(entry_size);
        page_entries.push(entry);
    }
    if !page_entries.is_empty() {
        let page = SearchPage {
            entries: page_entries,
        };
        builder.add_page(
            &logical_key,
            page_type,
            CompressionKind::None,
            0,
            0,
            serialize_page(&page, &logical_key)?,
        )?;
    }
    Ok(())
}

fn validate_record_page(
    descriptor: &PageDescriptor,
    actual_record_count: usize,
) -> Result<(), String> {
    if usize::try_from(descriptor.range_len).ok() != Some(actual_record_count) {
        return Err(format!(
            "authenticated record page {} count does not match its range",
            descriptor.page_id
        ));
    }
    Ok(())
}

fn next_catalog_page_offset(offset: usize, record_count: usize, total: usize) -> Option<usize> {
    let next = offset.saturating_add(record_count);
    (next < total).then_some(next)
}

fn serialize_page<T: Serialize>(value: &T, label: &str) -> Result<Vec<u8>, String> {
    postcard::to_allocvec(value).map_err(|err| format!("failed to serialize {label}: {err}"))
}

fn item_matches_query(item: &AvatarItemRecord, query: &str) -> bool {
    normalize_search_text(&item.item_name).contains(query)
        || normalize_search_text(slot_label(
            studio_slot_for_item(item).as_deref().unwrap_or(&item.slot),
        ))
        .contains(query)
}

fn set_matches_query(set: &AvatarSetRecord, query: &str) -> bool {
    normalize_search_text(&set.set_name).contains(query)
        || set
            .member_names
            .iter()
            .any(|name| normalize_search_text(name).contains(query))
}

fn normalized_roles(roles: &[String]) -> Vec<String> {
    let mut normalized = roles
        .iter()
        .map(|role| normalize_key(role))
        .filter(|role| !role.is_empty() && role != "all")
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_slot(slot: &str) -> Result<String, String> {
    let slot = normalize_key(slot);
    if slot.is_empty() || slot == "all" {
        return Ok("all".to_string());
    }
    if STUDIO_SLOTS.contains(&slot.as_str()) {
        Ok(slot)
    } else {
        Err(format!("unknown avatar slot: {slot}"))
    }
}

fn studio_slot_for_item(item: &AvatarItemRecord) -> Option<String> {
    let kind = normalize_key(&item.resource_kind);
    if kind == "weapon" || kind == "weapon_avatar" {
        return Some("weapon".to_string());
    }
    let slot = normalize_key(&item.slot);
    STUDIO_SLOTS.contains(&slot.as_str()).then_some(slot)
}

fn slot_label(slot: &str) -> &str {
    match normalize_key(slot).as_str() {
        "all" => "全部",
        "skin" => "皮肤",
        "cap" => "帽子",
        "hair" => "头部",
        "face" => "脸部",
        "neck" => "胸部",
        "coat" => "上衣",
        "belt" => "腰带",
        "pants" => "裤子",
        "shoes" => "鞋子",
        "weapon" => "武器",
        _ => slot,
    }
}

fn normalize_search_text(value: &str) -> String {
    value.trim().to_lowercase()
}

fn index_tokens(value: &str) -> BTreeSet<String> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut tokens = BTreeSet::new();
    for width in 1..=MAX_SEARCH_TOKEN_CHARS {
        if characters.len() < width {
            continue;
        }
        for start in 0..=characters.len() - width {
            tokens.insert(characters[start..start + width].iter().collect());
        }
    }
    tokens
}

fn query_tokens(value: &str) -> Vec<String> {
    let characters = value.chars().collect::<Vec<_>>();
    let width = characters.len().min(MAX_SEARCH_TOKEN_CHARS);
    if width == 0 {
        return Vec::new();
    }
    if characters.len() <= MAX_SEARCH_TOKEN_CHARS {
        return vec![characters.iter().collect()];
    }
    (0..=characters.len() - width)
        .map(|start| characters[start..start + width].iter().collect())
        .collect()
}

fn intersect_sorted(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut intersection = Vec::with_capacity(left.len().min(right.len()));
    let (mut left_index, mut right_index) = (0usize, 0usize);
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                intersection.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }
    intersection
}

fn stable_bucket(value: &str, bucket_count: usize) -> usize {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash as usize) % bucket_count
}

fn item_lookup_key(shard: usize) -> String {
    format!("items/by-id/{shard:03}")
}

fn set_lookup_key(shard: usize) -> String {
    format!("sets/by-id/{shard:03}")
}

fn layer_lookup_key(bucket: usize) -> String {
    format!("layers/{bucket:03}")
}

fn item_filter_key(role: &str, slot: &str) -> String {
    format!("items/filter/{role}/{slot}")
}

fn set_filter_key(role: &str) -> String {
    format!("sets/filter/{role}")
}

fn search_key(kind: &str, role: &str, slot: Option<&str>, shard: usize) -> String {
    match slot {
        Some(slot) => format!("{kind}/search/{role}/{slot}/{shard:02}"),
        None => format!("{kind}/search/{role}/{shard:02}"),
    }
}

fn search_page_type(kind: &str) -> Result<PageType, String> {
    match kind {
        "item" => Ok(PageType::ItemSearchIndex),
        "set" => Ok(PageType::SetSearchIndex),
        _ => Err(format!("unknown catalog search kind: {kind}")),
    }
}

fn hex_build_id(build_id: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(32);
    for byte in build_id {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
