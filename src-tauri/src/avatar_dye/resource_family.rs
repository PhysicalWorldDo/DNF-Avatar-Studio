use std::collections::BTreeMap;

use crate::game_resolver::{normalize_img_path_for_npk, same_named_mask_number};

use super::DyeTargetKindV1;

const MAX_MASK_NUMBER: u32 = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DyeResourceFamilyMember {
    pub(crate) entry_index: usize,
    pub(crate) entry_ref: String,
    pub(crate) kind: DyeTargetKindV1,
}

pub(crate) fn discover_resource_family_entries(
    entries: &[String],
    requested_base_entry: &str,
) -> Result<Vec<DyeResourceFamilyMember>, String> {
    let normalized_base = normalize_img_path_for_npk(requested_base_entry);
    if !normalized_base.ends_with(".img") {
        return Err("染色资源族的主入口必须是 IMG".to_string());
    }
    if is_numeric_mask_entry(&normalized_base) {
        return Err("染色资源族必须从主 IMG 打开，不能从 Mask 打开".to_string());
    }

    let matching_bases = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| normalize_img_path_for_npk(entry) == normalized_base)
        .collect::<Vec<_>>();
    let [(base_index, base_entry)] = matching_bases.as_slice() else {
        return match matching_bases.len() {
            0 => Err("找不到染色资源族的主 IMG".to_string()),
            _ => Err("NPK 中存在重复的主 IMG 入口".to_string()),
        };
    };

    let mut masks = BTreeMap::<u32, (usize, String)>::new();
    for (entry_index, entry_ref) in entries.iter().enumerate() {
        let Some(number) = same_named_mask_number(&normalized_base, entry_ref) else {
            continue;
        };
        if number == 0 || number > MAX_MASK_NUMBER {
            return Err(format!("Mask 编号超出安全范围: {number}"));
        }
        if masks
            .insert(number, (entry_index, entry_ref.clone()))
            .is_some()
        {
            return Err(format!("NPK 中存在重复的 Mask {number} 入口"));
        }
    }

    let mut family = Vec::with_capacity(1 + masks.len());
    family.push(DyeResourceFamilyMember {
        entry_index: *base_index,
        entry_ref: (*base_entry).clone(),
        kind: DyeTargetKindV1::Base,
    });
    family.extend(masks.into_iter().map(|(number, (entry_index, entry_ref))| {
        DyeResourceFamilyMember {
            entry_index,
            entry_ref,
            kind: DyeTargetKindV1::Mask { number },
        }
    }));
    Ok(family)
}

fn is_numeric_mask_entry(entry: &str) -> bool {
    let Some(stem) = entry.strip_suffix(".img") else {
        return false;
    };
    let Some((_, number)) = stem.rsplit_once("_mask") else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}
