#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
pub mod domain;
pub mod git;
mod persistence;
pub mod services;
mod terminal;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            use tauri::Manager;

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let database = persistence::Database::open(data_dir.join("marvis.sqlite3"))
                .map_err(std::io::Error::other)?;
            app.manage(database);
            app.manage(std::sync::Arc::new(terminal::TerminalBackend::default()));
            app.manage(std::sync::Arc::new(
                services::git::GitWatcherManager::default(),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::folder::open_folder,
            commands::workspace::restore_workspace,
            commands::workspace::register_folder,
            commands::workspace::set_default_branch,
            commands::workspace::select_checkout,
            commands::workspace::select_session,
            commands::files::files_list,
            commands::files::file_read,
            commands::git::git_status,
            commands::git::git_diff,
            commands::git::git_watch_checkout,
            commands::git::git_unwatch_checkout,
            commands::terminal::terminal_create,
            commands::terminal::terminal_write,
            commands::terminal::terminal_resize,
            commands::terminal::terminal_status,
            commands::terminal::terminal_close
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Marvis");
}
