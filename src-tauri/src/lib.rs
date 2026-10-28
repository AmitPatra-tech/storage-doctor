mod apps;
mod classify;
pub mod cleaner;
mod commands;
mod db;
mod dupes;
mod elevation;
mod recommendations;
pub mod scanner;
mod shell;
mod orphans;
mod store_apps;
mod thumbs;
mod uninstall;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
}

/// The Explorer right-click "Force delete" verb launches this exe with
/// `--force-delete "<path>"`. When that flag is present we handle the whole
/// thing with native dialogs and exit — never starting the webview UI, so it
/// is instant. Returns whether it handled the launch (and the app should not
/// start normally).
pub fn maybe_run_force_delete_cli() -> bool {
    let args: Vec<String> = std::env::args().collect();
    match shell::force_delete_target(&args) {
        Some(path) => {
            commands::force_delete_cli(&path);
            true
        }
        None => false,
    }
}

/// Relaunches elevated at startup when "Run as administrator" is on. Returns
/// true when this process should exit (an elevated copy replaced it).
pub fn maybe_relaunch_elevated() -> bool {
    elevation::startup()
}

pub fn run() {
    elevation::mark_instance();
    let conn = db::open().expect("failed to open database");

    tauri::Builder::default()
        // Registered first: a normal second launch focuses the existing
        // window instead of opening a duplicate. (The right-click force-delete
        // verb never reaches here — it is handled headlessly in `main`.)
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
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
            commands::force_delete_preview,
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
            commands::context_menu_enabled,
            commands::set_context_menu_enabled,
            commands::admin_status,
            commands::set_run_as_admin,
            commands::restart_as_admin,
            commands::find_orphaned_leftovers,
            commands::list_store_apps,
            commands::remove_store_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
