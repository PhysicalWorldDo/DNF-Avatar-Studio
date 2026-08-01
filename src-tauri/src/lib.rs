pub mod app_commands;
pub mod app_config;
pub mod avatar_data;
pub mod avatar_dye;
pub mod catalog_gallery;
pub mod catalog_gallery_snapshot;
pub mod catalog_thumbnail;
pub mod dnflib_safe;
mod dnflib_sys;
pub mod dye_color_presets;
pub mod exporter;
pub mod feature_gate;
pub mod game_resolver;
pub mod local_protection;
pub mod models;
pub mod outfit_swap;
mod preview_frame;
pub mod profession_wide_swap;
pub mod renderer;
pub mod resource_probe;
pub mod wardrobe;
mod wardrobe_limits;
pub mod weapon_swap;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(app_config::AppConfigState::open_default())
        .manage(wardrobe::WardrobeAppState::open_default())
        .manage(avatar_dye::AvatarDyeAppState::default())
        .manage(dye_color_presets::DyeColorPresetAppState::open_default())
        .manage(catalog_gallery::CatalogGalleryState::default())
        .invoke_handler(tauri::generate_handler![
            app_commands::activate_wardrobe_entry,
            app_commands::close_avatar_dye_session,
            app_commands::control_catalog_thumbnail_batch,
            app_commands::analyze_active_outfit_swap,
            app_commands::create_dye_color_preset,
            app_commands::default_game_dir,
            app_commands::analyze_outfit_swap,
            app_commands::clear_wardrobe_active,
            app_commands::create_wardrobe_entry,
            app_commands::create_wardrobe_group,
            app_commands::delete_wardrobe_entry,
            app_commands::delete_wardrobe_group,
            app_commands::delete_dye_color_preset,
            app_commands::export_avatar_npk,
            app_commands::export_avatar_dye_npk,
            app_commands::export_avatar_webp,
            app_commands::export_active_outfit_swap_npk,
            app_commands::export_active_weapon_swap_npk,
            app_commands::export_active_profession_wide_single_carrier_npk,
            app_commands::export_outfit_swap_npk,
            app_commands::export_wardrobe_entry_share,
            app_commands::export_wardrobe_group_share,
            app_commands::find_matching_wardrobe_entry,
            app_commands::get_avatar_items_by_ids,
            app_commands::get_avatar_set_with_members,
            app_commands::get_catalog_thumbnail_cache_summary,
            app_commands::get_catalog_thumbnail_batch_status,
            app_commands::import_wardrobe_share,
            app_commands::load_app_config,
            app_commands::load_dye_color_presets,
            app_commands::get_avatar_dye_session_view,
            app_commands::load_active_outfit_swap_page,
            app_commands::list_active_profession_wide_carriers,
            app_commands::list_catalog_thumbnail_batch_jobs,
            app_commands::load_wardrobe_state,
            app_commands::validate_wardrobe_state,
            app_commands::load_avatar_layers,
            app_commands::load_catalog_gallery_snapshot,
            app_commands::load_catalog_thumbnail,
            app_commands::open_avatar_catalog_index,
            app_commands::open_avatar_dye_session_from_studio,
            app_commands::open_avatar_dye_session_from_wardrobe,
            app_commands::preview_active_profession_wide_single_carrier,
            app_commands::preview_avatar_dye_colors,
            app_commands::preview_avatar_dye_comparison_colors,
            app_commands::preview_avatar_dye_batch_comparison,
            app_commands::prepare_catalog_thumbnail_batch,
            app_commands::query_avatar_items,
            app_commands::query_avatar_items_page,
            app_commands::query_avatar_sets,
            app_commands::query_avatar_sets_page,
            app_commands::query_catalog_gallery_page,
            app_commands::render_avatar_animation,
            app_commands::render_avatar_dye_animation,
            app_commands::render_avatar_dye_comparison_animation,
            app_commands::render_avatar_dye_comparison_frame,
            app_commands::render_avatar_dye_frame,
            app_commands::repair_avatar_dye_target,
            app_commands::render_avatar_layers,
            app_commands::render_active_outfit,
            app_commands::render_wardrobe_entry_preview,
            app_commands::reset_wardrobe_after_confirmation,
            app_commands::rename_wardrobe_group,
            app_commands::reorder_wardrobe_groups,
            app_commands::replace_wardrobe_entry_snapshot,
            app_commands::save_app_config,
            app_commands::save_avatar_dye_as_active,
            app_commands::save_avatar_dye_as_target_and_open_swap,
            app_commands::save_avatar_dye_to_wardrobe,
            app_commands::save_wardrobe_active,
            app_commands::start_catalog_thumbnail_batch,
            app_commands::set_wardrobe_favorite,
            app_commands::redo_avatar_dye,
            app_commands::reset_avatar_dye_scope,
            app_commands::undo_avatar_dye,
            app_commands::unlock_hidden_feature,
            app_commands::update_avatar_dye_colors,
            app_commands::update_avatar_dye_batch,
            app_commands::update_dye_color_preset,
            app_commands::update_wardrobe_entry
        ])
        .run(tauri::generate_context!())
        .expect("failed to run 薛定谔的化裁间");
}
