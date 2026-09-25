mod apps;
mod classify;
pub mod cleaner;
mod commands;
mod db;
mod dupes;
mod recommendations;
pub mod scanner;
mod shell;
mod thumbs;
mod uninstall;

use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    /// A path passed on the command line via `--force-delete "<path>"` (the
    /// Explorer right-click verb), waiting for the UI to pick it up on start.
    /// Taken exactly once by `take_launch_delete_path`.
    pub pending_force_delete: Mutex<Option<String>>,
}

/// A `--force-delete` target from an argv, kept only if it is a real file or
/// folder — a junk `%1` from the shell should launch nothing.
fn force_delete_arg(args: &[String]) -> Option<String> {
    shell::force_delete_target(args).filter(|p| shell::is_deletable_target(p))
}

pub fn run() {
    let conn = db::open().expect("failed to open database");
    let initial_force_delete = force_delete_arg(&std::env::args().collect::<Vec<_>>());

    tauri::Builder::default()
        // Must be registered first: a second launch (e.g. another right-click
        // "Force delete") forwards its arguments to this running instance
        // instead of opening a duplicate window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(path) = force_delete_arg(&argv) {
                if let Ok(mut pending) = app.state::<AppState>().pending_force_delete.lock() {
                    *pending = Some(path.clone());
                }
                let _ = app.emit("force-delete-request", path);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            db: Mutex::new(conn),
            pending_force_delete: Mutex::new(initial_force_delete),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_drives,
            commands::get_last_scan,
            commands::start_scan,
            commands::get_recommendations,
            commands::regenerate_recommendations,
            commands::set_recommendation_ignored,
            commands::browse_folder,
            commands::measure_recoverable,
            commands::get_scan_comparison,
            commands::get_app_usage,
            commands::get_installed_apps,
            commands::launch_uninstaller,
            commands::force_uninstall,
            commands::force_uninstall_elevated,
            commands::find_app_leftovers,
            commands::get_thumbnails,
            commands::get_app_icons,
            commands::delete_paths,
            commands::delete_paths_elevated,
            commands::force_delete_paths,
            commands::find_safe_cleanup,
            commands::find_duplicates,
            commands::search,
            commands::search_files,
            commands::cancel_search,
            commands::get_operations,
            commands::get_license,
            commands::set_license,
            commands::clear_license,
            commands::write_bytes,
            commands::open_external,
            commands::reveal_in_explorer,
            commands::delete_file,
            commands::take_launch_delete_path,
            commands::context_menu_enabled,
            commands::set_context_menu_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
