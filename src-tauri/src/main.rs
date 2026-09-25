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
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(Some(geometry)) = database.window_geometry() {
                    let geometry_is_visible = window.available_monitors().is_ok_and(|monitors| {
                        monitors.iter().any(|monitor| {
                            let monitor_position = monitor.position();
                            let monitor_size = monitor.size();
                            let left = i64::from(geometry.x).max(i64::from(monitor_position.x));
                            let top = i64::from(geometry.y).max(i64::from(monitor_position.y));
                            let right = (i64::from(geometry.x) + i64::from(geometry.width))
                                .min(i64::from(monitor_position.x) + i64::from(monitor_size.width));
                            let bottom = (i64::from(geometry.y) + i64::from(geometry.height)).min(
                                i64::from(monitor_position.y) + i64::from(monitor_size.height),
                            );
                            right - left >= 100 && bottom - top >= 100
                        })
                    });
                    if geometry_is_visible {
                        let _ = window
                            .set_size(tauri::PhysicalSize::new(geometry.width, geometry.height));
                        let _ = window
                            .set_position(tauri::PhysicalPosition::new(geometry.x, geometry.y));
                    }
                }
                if database.window_maximized().unwrap_or(false) {
                    let _ = window.maximize();
                }
                let tracked_window = window.clone();
                let window_database = database.clone();
                window.on_window_event(move |event| {
                    if !matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                        return;
                    }
                    let maximized = tracked_window.is_maximized().unwrap_or(false);
                    if !maximized {
                        if let (Ok(position), Ok(size)) =
                            (tracked_window.outer_position(), tracked_window.outer_size())
                        {
                            let _ =
                                window_database.set_window_geometry(persistence::WindowGeometry {
                                    x: position.x,
                                    y: position.y,
                                    width: size.width,
                                    height: size.height,
                                });
                        }
                    }
                    let _ = window_database.set_window_maximized(maximized);
                });
            }
            app.manage(database);
            app.manage(std::sync::Arc::new(terminal::TerminalBackend::default()));
            app.manage(std::sync::Arc::new(
                services::git::GitWatcherManager::default(),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::editor::editor_availability,
            commands::editor::editor_open_zed,
            commands::folder::open_folder,
            commands::workspace::restore_workspace,
            commands::workspace::register_folder,
            commands::workspace::locate_missing_checkout,
            commands::workspace::close_missing_checkout,
            commands::workspace::set_default_branch,
            commands::workspace::select_checkout,
            commands::workspace::select_session,
            commands::files::files_list,
            commands::files::files_search,
            commands::files::file_read,
            commands::files::file_read_markdown_image,
            commands::git::git_status,
            commands::git::git_diff,
            commands::git::git_diff_page,
            commands::git::git_viewed_files,
            commands::git::git_mark_viewed,
            commands::git::git_watch_checkout,
            commands::git::git_unwatch_checkout,
            commands::worktree::worktree_defaults,
            commands::worktree::worktree_create,
            commands::worktree::worktree_removal_info,
            commands::worktree::worktree_remove,
            commands::terminal::terminal_create,
            commands::terminal::terminal_write,
            commands::terminal::terminal_resize,
            commands::terminal::terminal_status,
            commands::terminal::terminal_close,
            commands::terminal::terminal_layout_load,
            commands::terminal::terminal_layout_save,
            commands::ui_state::ui_layout_load,
            commands::ui_state::ui_layout_save,
            commands::ui_state::checkout_ui_state_load,
            commands::ui_state::checkout_ui_state_save
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Marvis");
}

#[cfg(test)]
mod security_tests {
    use serde_json::Value;

    #[test]
    fn sec_05_webview_capabilities_are_domain_scoped_without_process_or_filesystem_privileges() {
        let capabilities: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let permissions = capabilities["permissions"].as_array().unwrap();
        let permissions = permissions
            .iter()
            .map(|permission| permission.as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            permissions,
            [
                "core:default",
                "core:window:allow-close",
                "core:window:allow-start-dragging",
                "dialog:allow-open"
            ]
        );

        let cargo_manifest = include_str!("../Cargo.toml");
        assert!(!cargo_manifest.contains("tauri-plugin-shell"));
        assert!(!cargo_manifest.contains("tauri-plugin-fs"));
        let registered_commands = include_str!("main.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!registered_commands.contains("shell::exec"));
        assert!(!registered_commands.contains("fs::read"));

        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let csp = config["app"]["security"]["csp"].as_str().unwrap();
        assert!(!csp.contains("unsafe-eval"));
        assert!(!csp.contains("*"));
    }
}
