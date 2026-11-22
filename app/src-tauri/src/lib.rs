//! Davilize desktop app: a Tauri shell over `dv-engine` (queries) and
//! `dv-render` (charts and exports). The same crates back the `davilize` CLI.

pub mod commands;
pub mod error;
pub mod service;
pub mod state;

use std::sync::Arc;

/// Starts the desktop window.
pub fn run() {
    let state = match state::AppState::new() {
        Ok(state) => Arc::new(state),
        Err(err) => {
            eprintln!("cannot start the query engine: {err}");
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::add_data_files,
            commands::open_report,
            commands::save_report,
            commands::preview_dataset,
            commands::profile_dataset,
            commands::render_visual,
            commands::render_dashboard_visual,
            commands::export_report,
            commands::validate_calculated_field,
            commands::register_calculated_fields,
            commands::cancel_query,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Davilize app");
}
