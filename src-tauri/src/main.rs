#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Emitter, Manager};

mod commands;
pub mod domain;
pub mod git;
#[cfg(target_os = "macos")]
mod menus;
mod persistence;
pub mod services;
mod terminal;

/// Registers the local MCP automation bridge. It only exists in debug builds
/// compiled with `--features dev-bridge`, so release builds never include it.
fn with_dev_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    #[cfg(all(debug_assertions, feature = "dev-bridge"))]
    {
        builder.plugin(tauri_plugin_mcp_bridge::init())
    }
    #[cfg(not(all(debug_assertions, feature = "dev-bridge")))]
    {
        builder
    }
}

/// Replaces the menu Tauri builds when it is given none, which on macOS is a File, a View, a
/// Window and a Help with nothing in it. It is replaced only on macOS: a menu set on Linux or
/// Windows draws a bar inside the window, and neither has one today.
fn with_menus(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    #[cfg(target_os = "macos")]
    {
        builder.menu(menus::build)
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder
    }
}

fn main() {
    // The one plugin that has to come before every other, and before `setup`, because plugins run
    // in the order they are added. A second launch has to stop before it opens the database:
    // opening the database is what clears the stored terminal sessions, so two copies of the app
    // would each wipe the other's on the way in. What the second launch does instead is bring the
    // window that is already open back to the front.
    let builder =
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    with_menus(with_dev_plugins(builder))
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
                            // The app is on its way out, so this is the only chance to keep the
                            // window where the user put it. Losing it silently costs them the
                            // placement on every launch after this one.
                            if let Err(error) =
                                window_database.set_window_geometry(persistence::WindowGeometry {
                                    x: position.x,
                                    y: position.y,
                                    width: size.width,
                                    height: size.height,
                                })
                            {
                                log::warn!("the window's size and place were not saved: {error}");
                            }
                        }
                    }
                    if let Err(error) = window_database.set_window_maximized(maximized) {
                        log::warn!("the window's maximized state was not saved: {error}");
                    }
                });
            }
            app.manage(database);
            app.manage(std::sync::Arc::new(terminal::TerminalBackend::default()));
            app.manage(std::sync::Arc::new(
                services::git::GitWatcherManager::default(),
            ));
            // One OpenCode server per checkout, owned for the app's lifetime so the
            // child process is stopped on exit rather than leaked.
            let agents = std::sync::Arc::new(services::agent::AgentService::new());
            let emitter = app.handle().clone();
            agents.set_event_sink(std::sync::Arc::new(
                move |event: services::agent::AgentEvent| {
                    // A dead window must not take the reader thread down with it.
                    let _ = emitter.emit("marvis://agent-event", event);
                },
            ));
            app.manage(agents);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agent::agent_sessions,
            commands::agent::agent_agents,
            commands::agent::agent_session_create,
            commands::agent::agent_prompt,
            commands::agent::agent_stop,
            commands::folder::open_folder,
            commands::workspace::restore_workspace,
            commands::workspace::register_folder,
            commands::workspace::list_recent_paths,
            commands::workspace::locate_missing_checkout,
            commands::workspace::close_missing_checkout,
            commands::workspace::close_checkout,
            commands::workspace::archive_checkout,
            commands::workspace::restore_archived_worktrees,
            commands::workspace::set_default_branch,
            commands::workspace::select_checkout,
            commands::workspace::select_session,
            commands::files::files_list,
            commands::files::files_search,
            commands::files::file_read,
            commands::files::file_write,
            commands::files::file_read_markdown_image,
            commands::git::git_status,
            commands::git::git_diff_stats,
            commands::git::git_checkout_diff_stats,
            commands::git::git_diff,
            commands::git::git_diff_page,
            commands::git::git_viewed_files,
            commands::git::git_mark_viewed,
            commands::git::git_watch_checkout,
            commands::git::git_unwatch_checkout,
            commands::review::review_notes,
            commands::review::review_note_create,
            commands::review::review_note_update,
            commands::review::review_note_delete,
            commands::review::review_notes_mark_sent,
            commands::review::review_note_anchors_verify,
            commands::review::review_note_outdated_clear,
            commands::review::review_note_resolve,
            commands::review::review_rounds,
            commands::review::review_round_dispatch,
            commands::review::review_rounds_requeue,
            commands::review::review_round_flush,
            commands::review::review_round_reconcile,
            commands::review::review_round_ack,
            commands::worktree::worktree_defaults,
            commands::worktree::worktree_create,
            commands::worktree::worktree_removal_info,
            commands::worktree::worktree_remove,
            commands::terminal::terminal_create,
            commands::terminal::terminal_write,
            commands::terminal::terminal_resize,
            commands::terminal::terminal_status,
            commands::terminal::terminal_close,
            commands::terminal::terminal_rename,
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
                // Double-clicking the title-bar drag region zooms the window.
                "core:window:allow-toggle-maximize",
                "dialog:allow-open",
                "clipboard-manager:allow-write-text"
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

        // The whole Tauri API is reached through the `@tauri-apps/api` imports, so injecting it a
        // second way as a global on the window would be reachability nothing needs and anything
        // running in the document would have.
        assert!(
            !config["app"]
                .get("withGlobalTauri")
                .is_some_and(|enabled| enabled.as_bool() == Some(true)),
            "`withGlobalTauri` hands the webview a global copy of the API it does not use"
        );

        // The dev bridge is the one build that does need it, and it needs it completely: it drives
        // the webview through `window.__TAURI__`, so without the global the automation can still
        // read the DOM but can no longer reach the app. Dropping the global from the base config
        // without putting it back here is what broke it, and nothing in the app's own source says
        // so. It is merged in by the one script that cannot produce an installable build, which is
        // what keeps it out of every build a user runs.
        let dev_bridge: Value =
            serde_json::from_str(include_str!("../tauri.dev-bridge.conf.json")).unwrap();
        assert_eq!(
            dev_bridge["app"]["withGlobalTauri"],
            Value::Bool(true),
            "the dev bridge overlay is the only place the global comes back"
        );
        let scripts: Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
        assert!(
            scripts["scripts"]["dev:app"]
                .as_str()
                .unwrap()
                .contains("tauri.dev-bridge.conf.json"),
            "the dev-bridge build is the one that gets the global"
        );
        assert!(
            !scripts["scripts"]["build:app"]
                .as_str()
                .unwrap()
                .contains("tauri.dev-bridge.conf.json"),
            "the build that ships must not get the global back"
        );
    }

    #[test]
    fn sec_05_2_the_only_write_the_webview_can_reach_is_contained_by_the_checkout() {
        // `sec_05` says what the webview may ask for. This says what it may change, which is the
        // question that only became one when the app started writing into the user's
        // repositories. It is numbered beside `sec_05` rather than on its own: `sec_06` is
        // already taken three times over by the ownership checks, and `sec_01_02` is the
        // precedent for a companion.
        //
        // Everything the webview can invoke is a domain command: the setup hook runs before any
        // of it and is not part of this surface.
        let registered_commands = include_str!("main.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let handler = registered_commands
            .split("generate_handler![")
            .nth(1)
            .and_then(|block| block.split("])").next())
            .expect("the invoke handler is not a list of commands any more");
        for entry in handler
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            assert!(
                entry.starts_with("commands::"),
                "`{entry}` is reachable from the webview outside the domain commands"
            );
        }

        // And no command module reaches for the filesystem itself, outside its own tests: each one
        // names a `services::` entry instead, which is where containment lives. Read from disk
        // rather than listed here, so a module added later is covered without editing this test.
        let commands_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
        let mut modules: Vec<_> = std::fs::read_dir(&commands_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
            .collect();
        modules.sort();
        assert!(!modules.is_empty(), "no command module was found to check");
        for module in modules {
            let source = std::fs::read_to_string(&module).unwrap();
            let production = source.split("#[cfg(test)]").next().unwrap();
            for primitive in ["fs::", "File::", "OpenOptions", "Command::new"] {
                assert!(
                    !production.contains(primitive),
                    "{} reached for `{primitive}` instead of a service",
                    module.display()
                );
            }
        }

        // The one command that mutates a file names the checkout it is allowed to write inside
        // and hands the work to the service that resolves the path against it. A `file_write`
        // that grew a bare path, or that stopped going through the service, is the escape.
        let write_command = include_str!("commands/files.rs")
            .split("#[tauri::command]")
            .find(|block| block.contains("pub async fn file_write("))
            .expect("file_write is not a command any more");
        assert!(write_command.contains("checkout_id: String"));
        assert!(write_command.contains("services::files::write"));

        // And that service is the one holding the guards: the path is resolved against the
        // checkout, a traversal is refused, and a read-only file is not replaced.
        let file_service = include_str!("services/files.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for guard in [
            "resolve_checkout_path",
            "parse_relative_path",
            "permissions.readonly()",
            "fs::set_permissions",
        ] {
            assert!(
                file_service.contains(guard),
                "the write path stopped checking for `{guard}`"
            );
        }
    }
}

#[cfg(test)]
mod startup_tests {
    #[test]
    fn a_second_launch_is_stopped_before_the_database_is_opened() {
        // The single-instance plugin works by being registered first, and "first" is the whole of
        // it: the plugins run in the order they are added, and `setup` is where the database is
        // opened. Opening it is what clears the stored terminal sessions, so a second copy that got
        // that far would take the first copy's sessions with it and leave a set of terminals whose
        // PTYs belong to a process that is gone.
        let main = include_str!("main.rs");
        let registered = main.split("#[cfg(test)]").next().unwrap();
        let guard = registered
            .find("tauri_plugin_single_instance::init")
            .expect("the single-instance plugin is not registered at all");
        for later in [
            ".setup(",
            "tauri_plugin_log::Builder",
            "tauri_plugin_dialog::init",
            "tauri_plugin_clipboard_manager::init",
        ] {
            assert!(
                !registered[..guard].contains(later),
                "`{later}` is registered before the single-instance plugin, so a second launch \
                 reaches it before anything can turn it away"
            );
        }
    }

    #[test]
    fn the_menu_bar_is_the_app_menu_and_the_one_macos_requires() {
        // Tauri builds a menu of its own when the builder is given none, so the way to have a
        // small menu bar is not to have no menu code. It is to have menu code that says what
        // stays, and these are the two submenus that stay.
        let menus = include_str!("menus.rs");
        assert_eq!(
            menus.matches("Submenu::with_items(").count(),
            2,
            "the menu bar is not the app menu and Edit any more"
        );
        for item in [
            // The app menu, named after the product. macOS reads the first submenu as the
            // application menu, and an app that cannot be hidden, put away or quit from it is
            // not an app.
            "handle.package_info().name.clone()",
            "PredefinedMenuItem::about",
            "PredefinedMenuItem::services",
            "PredefinedMenuItem::hide(",
            "PredefinedMenuItem::hide_others",
            "PredefinedMenuItem::quit",
            // Edit, and every one of these is load-bearing rather than conventional. macOS never
            // tells the webview about `⌘C`, `⌘V`, `⌘X`, `⌘A` or `⌘Z`; the key equivalents on
            // these items send `copy:` and the rest down the responder chain into it. Drop `copy`
            // and the search field, the editor and the terminal silently stop copying, and the
            // right-click copy menu does not stand in for it.
            r#""Edit""#,
            "PredefinedMenuItem::undo",
            "PredefinedMenuItem::redo",
            "PredefinedMenuItem::cut",
            "PredefinedMenuItem::copy",
            "PredefinedMenuItem::paste",
            "PredefinedMenuItem::select_all",
        ] {
            assert!(
                menus.contains(item),
                "the menu bar stopped carrying `{item}`"
            );
        }

        // And the About panel names the repository. macOS draws the name, the version, the short
        // version, the copyright, an icon and the credits, and no `website`, so the credits are
        // the only line of text in there that can say where the source is. Dropped, the panel
        // says what the app is called and nothing about where to get it or read it.
        assert!(
            menus.contains("credits: handle.config().bundle.homepage.clone()"),
            "the About panel stopped naming the repository"
        );

        // And it is set, on macOS only. A menu set on Linux or Windows would draw a bar inside
        // the window, which is a second place for the app's menus to live.
        let main = include_str!("main.rs");
        let registered = main.split("#[cfg(test)]").next().unwrap();
        assert!(
            registered.contains("with_menus(with_dev_plugins(builder))"),
            "the menu is built and then never handed to the app"
        );
        let helper = registered
            .split("fn with_menus(")
            .nth(1)
            .and_then(|body| body.split("\n}").next())
            .expect("`with_menus` is not a function any more");
        assert!(
            helper.contains(r#"#[cfg(target_os = "macos")]"#),
            "the menu is set on every platform, and only macOS has a menu bar"
        );
    }

    #[test]
    fn the_window_opens_on_the_page_background_and_not_on_a_white_frame() {
        // A webview with no background color of its own paints white until the page covers it, and
        // wry only stops it doing so when one is set. So the window opens on white and the app
        // opens on `--marvis-bg-0`, and the difference is a frame the user sees on every launch.
        // Setting it is not enough on its own: the value lives in a config file and the color it
        // has to match lives in a stylesheet, and nothing makes the two follow each other when
        // the palette changes. This is where that is said out loud.
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let page_background = config["app"]["windows"][0]["backgroundColor"]
            .as_str()
            .expect("the window has no `backgroundColor`, so it opens on a white frame")
            .to_lowercase();
        let stylesheet = include_str!("../../src/marvis.css");
        assert!(
            stylesheet.contains(&format!("--marvis-bg-0: {page_background};")),
            "the window opens on {page_background} but the page is painted on something else, so \
             the frame before the first paint is a different color than the app"
        );
    }
}
