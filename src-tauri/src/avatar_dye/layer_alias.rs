use crate::game_resolver::normalize_img_path_for_npk;
use crate::models::AvatarLayer;

pub(crate) fn select_equivalent_dye_layer<'a>(
    candidates: &[&'a AvatarLayer],
) -> Option<&'a AvatarLayer> {
    let first = *candidates.first()?;
    candidates
        .iter()
        .skip(1)
        .all(|candidate| equivalent_animation_job_layer(first, candidate))
        .then_some(first)
}

pub(crate) fn equivalent_animation_job_layer(left: &AvatarLayer, right: &AvatarLayer) -> bool {
    left.item_id.trim() == right.item_id.trim()
        && left.slot.trim().eq_ignore_ascii_case(right.slot.trim())
        && left.layer_order == right.layer_order
        && left.equipment_type_enum == right.equipment_type_enum
        && left.layer_tag.trim() == right.layer_tag.trim()
        && left.variation.trim() == right.variation.trim()
        && left.palette_index == right.palette_index
        && left.enable_dye_mask1 == right.enable_dye_mask1
        && left.enable_dye_mask2 == right.enable_dye_mask2
        && normalize_img_path_for_npk(&left.visual_img)
            == normalize_img_path_for_npk(&right.visual_img)
}
