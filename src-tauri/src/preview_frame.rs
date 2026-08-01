pub(crate) fn preview_start_frame_for_profession(playable_role_key: &str) -> usize {
    match playable_role_key.trim().to_ascii_lowercase().as_str() {
        "thief" => 10,
        "gunner" => 18,
        "at_gunner" => 6,
        "swordman" => 95,
        _ => 0,
    }
}

pub(crate) fn preview_frame_index_for_profession(
    playable_role_key: &str,
    frame_count: usize,
) -> usize {
    if frame_count == 0 {
        0
    } else {
        preview_start_frame_for_profession(playable_role_key) % frame_count
    }
}
