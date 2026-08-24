mod auto_match;
mod bangumi;
mod cache;
mod commands;
mod db;
pub mod models;
mod player;
mod scanner;
mod title_extractor;
mod window_state;

use std::{io, path::PathBuf, sync::Mutex};

use db::Database;
use scanner::ScanControl;
use tauri::{AppHandle, Manager, PhysicalSize, RunEvent, WindowEvent};

pub const DATABASE_URL: &str = "sqlite:morimediashelf.db";

pub struct AppState {
    database: Database,
    default_cover_cache_dir: PathBuf,
    active_scan: Mutex<Option<ScanControl>>,
    window_size: window_state::WindowSizeMemory,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| io::Error::other(format!("无法确定应用数据目录：{error}")))?;
            std::fs::create_dir_all(&app_data_dir)?;
            let default_cover_cache_dir = app_data_dir.join("cache").join("covers");
            let database = Database::new(app_data_dir.join("morimediashelf.db"));
            database.migrate().map_err(io::Error::other)?;
            let saved_window_size = database.get_window_size().map_err(io::Error::other)?;
            let initial_window_size = if let Some(window) = app.get_webview_window("main") {
                saved_window_size
                    .and_then(|size| window_state::restore_window_size(&window, size))
                    .or_else(|| {
                        window.inner_size().ok().and_then(|physical_size| {
                            window_state::capture_normal_window_size(&window, physical_size, None)
                        })
                    })
            } else {
                None
            };
            let configured_cache = database
                .get_settings(&default_cover_cache_dir)
                .map(|settings| PathBuf::from(settings.cover_cache_directory))
                .map_err(io::Error::other)?;
            let library_roots = database
                .list_roots()
                .map_err(io::Error::other)?
                .into_iter()
                .map(|root| PathBuf::from(root.path))
                .collect::<Vec<_>>();
            if let Ok(validated) = cache::validate_cache_location(&configured_cache, library_roots)
            {
                let is_default = cache::is_equal_or_within(&validated, &default_cover_cache_dir)
                    && cache::is_equal_or_within(&default_cover_cache_dir, &validated);
                if is_default {
                    cache::ensure_directories(&validated).map_err(io::Error::other)?;
                }
            }
            app.manage(AppState {
                database,
                default_cover_cache_dir,
                active_scan: Mutex::new(None),
                window_size: window_state::WindowSizeMemory::new(initial_window_size),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_bootstrap,
            commands::show_main_window,
            commands::list_library_roots,
            commands::add_library_root,
            commands::remove_library_root,
            commands::update_library_root_name,
            commands::open_library_root_in_explorer,
            commands::get_all_resources,
            commands::list_recently_watched,
            commands::list_favorite_folders,
            commands::create_favorite_folder,
            commands::rename_favorite_folder,
            commands::delete_favorite_folder,
            commands::list_favorite_folder_nodes,
            commands::batch_add_nodes_to_favorite,
            commands::batch_remove_nodes_from_favorite,
            commands::browse_library,
            commands::get_node_detail,
            commands::search_library,
            commands::start_scan,
            commands::match_existing_content,
            commands::cancel_scan,
            commands::get_scan_status,
            commands::set_node_type,
            commands::reset_node_type,
            commands::batch_set_node_type,
            commands::batch_reset_node_type,
            commands::set_node_display_name,
            commands::list_user_tags,
            commands::create_or_assign_user_tag,
            commands::assign_user_tag,
            commands::batch_assign_tag,
            commands::batch_create_and_assign_tag,
            commands::rename_user_tag,
            commands::unassign_user_tag,
            commands::delete_user_tag,
            commands::get_bangumi_search_prefill,
            commands::search_bangumi,
            commands::bind_bangumi,
            commands::retry_bangumi_cover,
            commands::clear_bangumi_binding,
            commands::set_container_cover,
            commands::clear_node_cover,
            commands::get_cover_data_url,
            commands::get_settings,
            commands::update_settings,
            commands::get_collection_sort_preferences,
            commands::update_collection_sort_preference,
            commands::test_mpv,
            commands::play_media,
            commands::open_node_in_explorer,
            commands::open_media_in_explorer,
            commands::open_resource_file,
            commands::open_resource_in_explorer,
            commands::open_cover_cache_directory,
            commands::open_external_url,
            commands::get_cache_stats,
            commands::clear_cover_cache,
            commands::rebuild_index,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build M²Shelf");

    app.run(|app, event| match event {
        RunEvent::WindowEvent { label, event, .. } if label == "main" => match event {
            WindowEvent::Resized(physical_size) => {
                remember_main_window_size(app, physical_size, None);
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor,
                new_inner_size,
                ..
            } => {
                remember_main_window_size(app, new_inner_size, Some(scale_factor));
            }
            // Capture once more while the native window is still available. SQLite is not
            // touched here; the single persistence write remains in ExitRequested below.
            WindowEvent::CloseRequested { .. } => remember_current_main_window_size(app),
            _ => {}
        },
        RunEvent::ExitRequested { .. } => {
            remember_current_main_window_size(app);
            // Preserve the existing shutdown contract: request scan cancellation before the one
            // small application-settings write, and never wait for or resume background work.
            commands::cancel_scan_on_exit(app);
            let state = app.state::<AppState>();
            if let Some(size) = state.window_size.get() {
                if let Err(error) = state.database.save_window_size(size) {
                    eprintln!("failed to persist main window size: {error}");
                }
            }
        }
        _ => {}
    });
}

fn remember_current_main_window_size(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(physical_size) = window.inner_size() else {
        return;
    };
    if let Some(size) = window_state::capture_normal_window_size(&window, physical_size, None) {
        app.state::<AppState>().window_size.remember(size);
    }
}

fn remember_main_window_size(
    app: &AppHandle,
    physical_size: PhysicalSize<u32>,
    scale_factor: Option<f64>,
) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Some(size) =
        window_state::capture_normal_window_size(&window, physical_size, scale_factor)
    {
        app.state::<AppState>().window_size.remember(size);
    }
}
