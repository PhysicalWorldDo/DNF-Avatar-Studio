use std::collections::{BTreeMap, HashMap};

use sha2::{Digest, Sha256};

use super::models::{
    DyeColorEditV1, DyeRecipeV1, DyeTargetKindV1, DyeTargetRecipeV1, PersistedDyeTargetRefV1,
};

const RECIPE_SCHEMA_V1: u16 = 1;
const MAX_DYE_TARGETS: usize = 4_096;
const MAX_TOTAL_COLOR_EDITS: usize = 65_536;

pub fn canonicalize_recipe(mut recipe: DyeRecipeV1) -> Result<DyeRecipeV1, String> {
    if recipe.schema_version != RECIPE_SCHEMA_V1 {
        return Err("不支持的染色配方版本".to_string());
    }
    if recipe.targets.len() > MAX_DYE_TARGETS {
        return Err("染色目标数量超过安全上限".to_string());
    }

    let mut total_edits = 0_usize;
    let mut physical_sources = HashMap::new();
    let mut physical_edits = HashMap::<(String, u16, u16), ([u8; 4], [u8; 3])>::new();
    for target in &mut recipe.targets {
        normalize_target_ref(&mut target.target)?;
        if usize::from(target.palette_index) >= usize::from(target.source.palette_count) {
            return Err("Palette 索引无效".to_string());
        }
        let mut colors = BTreeMap::<u16, DyeColorEditV1>::new();
        for edit in target.colors.drain(..) {
            if edit.color_index >= target.source.color_count {
                return Err("颜色索引无效".to_string());
            }
            if edit.expected_rgba[3] == 0 && edit.replacement_rgb != edit.expected_rgba[..3] {
                return Err("完全透明颜色不可修改".to_string());
            }
            if edit.replacement_rgb == edit.expected_rgba[..3] {
                continue;
            }
            if let Some(existing) = colors.get(&edit.color_index) {
                if existing != &edit {
                    return Err("同一颜色索引存在冲突配方".to_string());
                }
                continue;
            }
            colors.insert(edit.color_index, edit);
        }
        target.colors = colors.into_values().collect();
        total_edits = total_edits
            .checked_add(target.colors.len())
            .ok_or_else(|| "染色颜色数量溢出".to_string())?;
        if total_edits > MAX_TOTAL_COLOR_EDITS {
            return Err("染色颜色数量超过安全上限".to_string());
        }

        let entry_key = normalized_entry(&target.target.private_entry_ref);
        if !target.colors.is_empty() {
            let source_key = (entry_key.clone(), target.palette_index);
            if let Some(existing) = physical_sources.insert(source_key, target.source.clone()) {
                if existing != target.source {
                    return Err("同一 IMG 和 Palette 的源资源指纹不一致".to_string());
                }
            }
        }
        for edit in &target.colors {
            let key = (entry_key.clone(), target.palette_index, edit.color_index);
            let value = (edit.expected_rgba, edit.replacement_rgb);
            if let Some(existing) = physical_edits.insert(key, value) {
                if existing != value {
                    return Err("同一 IMG 和 Palette 存在冲突配方".to_string());
                }
            }
        }
    }

    recipe.targets.retain(|target| !target.colors.is_empty());
    recipe
        .targets
        .sort_by(|left, right| target_sort_key(left).cmp(&target_sort_key(right)));
    let mut merged = Vec::<DyeTargetRecipeV1>::with_capacity(recipe.targets.len());
    for target in recipe.targets.drain(..) {
        if let Some(previous) = merged.last_mut() {
            if target_sort_key(previous) == target_sort_key(&target) {
                if previous.source != target.source {
                    return Err("同一染色目标的源资源指纹不一致".to_string());
                }
                previous.colors.extend(target.colors);
                previous.colors.sort_by_key(|edit| edit.color_index);
                previous.colors.dedup();
                continue;
            }
        }
        merged.push(target);
    }
    recipe.targets = merged;
    recipe.recipe_hash = hash_recipe(&recipe);
    Ok(recipe)
}

fn normalize_target_ref(target: &mut PersistedDyeTargetRefV1) -> Result<(), String> {
    target.slot = target.slot.trim().to_ascii_lowercase();
    target.item_ref = target.item_ref.trim().to_string();
    target.layer_tag = target.layer_tag.trim().to_string();
    target.private_entry_ref = target.private_entry_ref.trim().replace('\\', "/");
    if target.slot.is_empty()
        || target.item_ref.is_empty()
        || target.layer_tag.is_empty()
        || target.private_entry_ref.is_empty()
    {
        return Err("染色目标引用不完整".to_string());
    }
    if let DyeTargetKindV1::Mask { number } = target.target_kind {
        if number == 0 || number > 4_096 {
            return Err("Mask 编号无效".to_string());
        }
    }
    Ok(())
}

fn target_sort_key(
    target: &DyeTargetRecipeV1,
) -> (String, String, String, DyeTargetKindV1, String, u16) {
    (
        target.target.slot.clone(),
        target.target.item_ref.clone(),
        target.target.layer_tag.clone(),
        target.target.target_kind.clone(),
        normalized_entry(&target.target.private_entry_ref),
        target.palette_index,
    )
}

fn normalized_entry(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn hash_recipe(recipe: &DyeRecipeV1) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(recipe.schema_version.to_le_bytes());
    hasher.update((recipe.targets.len() as u64).to_le_bytes());
    for target in &recipe.targets {
        hash_string(&mut hasher, &target.target.slot);
        hash_string(&mut hasher, &target.target.item_ref);
        hash_string(&mut hasher, &target.target.layer_tag);
        match target.target.target_kind {
            DyeTargetKindV1::Base => hasher.update([0]),
            DyeTargetKindV1::Mask { number } => {
                hasher.update([1]);
                hasher.update(number.to_le_bytes());
            }
        }
        hash_string(
            &mut hasher,
            &normalized_entry(&target.target.private_entry_ref),
        );
        hasher.update(target.palette_index.to_le_bytes());
        hasher.update(target.source.img_version.to_le_bytes());
        hasher.update(target.source.frame_count.to_le_bytes());
        hasher.update(target.source.palette_count.to_le_bytes());
        hasher.update(target.source.color_count.to_le_bytes());
        hasher.update(target.source.palette_sha256);
        hasher.update(target.source.entry_sha256);
        hasher.update((target.colors.len() as u64).to_le_bytes());
        for edit in &target.colors {
            hasher.update(edit.color_index.to_le_bytes());
            hasher.update(edit.expected_rgba);
            hasher.update(edit.replacement_rgb);
        }
    }
    hasher.finalize().into()
}

fn hash_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}
