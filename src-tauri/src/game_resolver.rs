use std::path::{Path, PathBuf};

pub const DEFAULT_GAME_DIR: &str = r"E:\WeGameApps\地下城与勇士：创新世纪\ImagePacks2";

pub fn normalize_img_path_for_npk(img_path: &str) -> String {
    let normalized = img_path
        .trim()
        .trim_matches('`')
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    if normalized.starts_with("character/") {
        format!("sprite/{normalized}")
    } else {
        normalized
    }
}

pub fn npk_name_for_img_path(img_path: &str) -> Option<String> {
    let normalized = normalize_img_path_for_npk(img_path);
    if !normalized.to_ascii_lowercase().ends_with(".img") {
        return None;
    }

    let mut parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.trim().is_empty())
        .collect();
    if parts.len() < 2 {
        return None;
    }

    parts.pop();
    Some(format!("{}.NPK", parts.join("_")))
}

pub fn resolve_game_npk_path(game_dir: &Path, img_path: &str) -> Option<PathBuf> {
    npk_name_for_img_path(img_path).map(|name| game_dir.join(name))
}

pub fn img_path_lookup_candidates(img_path: &str) -> Vec<String> {
    let original = normalize_img_path_for_npk(img_path);
    let mut candidates = vec![original.clone()];
    let Some((directory, file_name)) = original.rsplit_once('/') else {
        return candidates;
    };
    let Some(stem) = file_name.strip_suffix(".img") else {
        return candidates;
    };
    let Some((source_prefix, unprefixed_stem)) = stem.split_once('_') else {
        if let Some(visual_prefix) = character_visual_prefix_for_path(&original) {
            push_unique_candidate(
                &mut candidates,
                format!("{directory}/{visual_prefix}_{stem}.img"),
            );
        }
        return candidates;
    };
    if source_prefix.is_empty() || unprefixed_stem.is_empty() {
        return candidates;
    }

    if let Some(visual_prefix) = character_visual_prefix_for_path(&original) {
        if visual_prefix != source_prefix {
            push_unique_candidate(
                &mut candidates,
                format!("{directory}/{visual_prefix}_{unprefixed_stem}.img"),
            );
        }
    }
    push_unique_candidate(
        &mut candidates,
        format!("{directory}/{unprefixed_stem}.img"),
    );
    candidates
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImgResourceMetadata {
    pub version: i32,
    pub palette_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImgResource<T> {
    pub entry_path: String,
    pub palette_index: usize,
    pub palette_fallback: bool,
    pub metadata: ImgResourceMetadata,
    pub value: T,
}

pub fn resolve_img_resource<T, E, F>(
    img_path: &str,
    requested_palette_index: usize,
    mut lookup: F,
) -> Result<Option<ResolvedImgResource<T>>, E>
where
    F: FnMut(&str) -> Result<Option<(T, ImgResourceMetadata)>, E>,
{
    for entry_path in img_path_lookup_candidates(img_path) {
        let Some((value, metadata)) = lookup(&entry_path)? else {
            continue;
        };
        return Ok(Some(ResolvedImgResource {
            entry_path,
            palette_index: effective_palette_index(requested_palette_index, metadata.version),
            palette_fallback: false,
            metadata,
            value,
        }));
    }

    let Some((carrier_path, palette_index)) = skin_palette_carrier(img_path) else {
        return Ok(None);
    };
    for entry_path in img_path_lookup_candidates(&carrier_path) {
        let Some((value, metadata)) = lookup(&entry_path)? else {
            continue;
        };
        if metadata.version != 6 || metadata.palette_count <= palette_index {
            continue;
        }
        return Ok(Some(ResolvedImgResource {
            entry_path,
            palette_index,
            palette_fallback: true,
            metadata,
            value,
        }));
    }
    Ok(None)
}

fn effective_palette_index(requested_palette_index: usize, version: i32) -> usize {
    if matches!(version, 2 | 4) {
        0
    } else {
        requested_palette_index
    }
}

fn skin_palette_carrier(img_path: &str) -> Option<(String, usize)> {
    let normalized = normalize_img_path_for_npk(img_path);
    if !normalized.contains("/avatar/skin/") {
        return None;
    }
    let (directory, file_name) = normalized.rsplit_once('/')?;
    let stem = file_name.strip_suffix(".img")?;
    let (prefix, digits) = if let Some(digits) = stem.strip_prefix("body") {
        (None, digits)
    } else {
        let (prefix, digits) = stem.rsplit_once("_body")?;
        if prefix.is_empty() {
            return None;
        }
        (Some(prefix), digits)
    };
    if digits.len() < 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let variation = digits.parse::<usize>().ok()?;
    let palette_index = variation % 100;
    if palette_index == 0 {
        return None;
    }
    let carrier_variation = variation - palette_index;
    let carrier_digits = format!("{carrier_variation:0width$}", width = digits.len());
    let carrier_file = prefix.map_or_else(
        || format!("body{carrier_digits}.img"),
        |prefix| format!("{prefix}_body{carrier_digits}.img"),
    );
    Some((format!("{directory}/{carrier_file}"), palette_index))
}

fn push_unique_candidate(candidates: &mut Vec<String>, candidate: String) {
    if !candidates
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        candidates.push(candidate);
    }
}

fn character_visual_prefix_for_path(normalized_img_path: &str) -> Option<&'static str> {
    let mut parts = normalized_img_path.split('/');
    if parts.next()? != "sprite" || parts.next()? != "character" {
        return None;
    }
    let profession = parts.next()?;
    let equipment_group = parts.next()?;
    match (profession, equipment_group) {
        ("archer", "equipment") => Some("ac"),
        ("demoniclancer", "equipment") => Some("dl"),
        ("fighter", "atequipment") => Some("fm"),
        ("fighter", "equipment") => Some("ft"),
        ("gunblader", "equipment") => Some("gb"),
        ("gunner", "atequipment") => Some("gg"),
        ("gunner", "equipment") => Some("gn"),
        ("imperialknight", "equipment") => Some("ik"),
        ("knight", "equipment") => Some("kn"),
        ("mage", "atequipment") => Some("mm"),
        ("mage", "equipment") => Some("mg"),
        ("priest", "atequipment") => Some("pg"),
        ("priest", "equipment") => Some("pr"),
        ("swordman", "atequipment") => Some("sg"),
        ("swordman", "equipment") => Some("sm"),
        ("thief", "equipment") => Some("th"),
        _ => None,
    }
}

pub fn same_named_mask_number(base_img_path: &str, candidate_img_path: &str) -> Option<u32> {
    same_named_mask_number_with_policy(base_img_path, candidate_img_path, true)
}

pub(crate) fn legacy_same_named_mask_number_for_ownership(
    base_img_path: &str,
    candidate_img_path: &str,
) -> Option<u32> {
    same_named_mask_number_with_policy(base_img_path, candidate_img_path, false)
}

fn same_named_mask_number_with_policy(
    base_img_path: &str,
    candidate_img_path: &str,
    require_canonical_decimal: bool,
) -> Option<u32> {
    let base = normalize_img_path_for_npk(base_img_path);
    let candidate = normalize_img_path_for_npk(candidate_img_path);
    if is_mask_img_path(&base) {
        return None;
    }

    let base_stem = base.strip_suffix(".img")?;
    let candidate_tail = candidate.strip_prefix(base_stem)?.strip_suffix(".img")?;
    let number = candidate_tail.strip_prefix("_mask")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let parsed = number.parse::<u32>().ok()?;
    (!require_canonical_decimal || parsed.to_string() == number).then_some(parsed)
}

fn is_mask_img_path(img_path: &str) -> bool {
    let Some(stem) = img_path.strip_suffix(".img") else {
        return false;
    };
    let Some((_, suffix)) = stem.rsplit_once("_mask") else {
        return false;
    };
    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
}
