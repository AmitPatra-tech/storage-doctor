// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The Explorer right-click "Force delete" verb is handled with native
    // dialogs and exits, without ever starting the app window.
    if storage_doctor_lib::maybe_run_force_delete_cli() {
        return;
    }
    storage_doctor_lib::run()
}
