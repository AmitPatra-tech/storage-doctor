mod apps;
mod classify;
mod cleaner;
mod commands;
mod db;
mod dupes;
mod recommendations;
mod scanner;
mod uninstall;

use std::sync::Mutex;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
}

pub fn run() {
    let conn = db::open().expect("failed to open database");

    tauri::Builder::default()
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
            commands::get_scan_comparison,
            commands::get_app_usage,
            commands::get_installed_apps,
            commands::launch_uninstaller,
            commands::find_app_leftovers,
            commands::delete_paths,
            commands::delete_paths_elevated,
            commands::find_safe_cleanup,
            commands::find_duplicates,
            commands::search,
            commands::search_files,
            commands::get_operations,
            commands::get_license,
            commands::set_license,
            commands::clear_license,
            commands::write_bytes,
            commands::open_external,
            commands::reveal_in_explorer,
            commands::delete_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
