use std::{env, path::Path, path::PathBuf};
use tauri::{
    ipc::{Channel, Response},
    State,
};
use terminal_backend::{SpawnOptions, TerminalBackend, TerminalId, TerminalMetrics};

#[tauri::command]
fn default_cwd() -> String {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .display()
        .to_string()
}

#[tauri::command]
fn terminal_start(
    backend: State<'_, TerminalBackend>,
    cwd: String,
    cols: u16,
    rows: u16,
    on_output: Channel<Response>,
) -> Result<TerminalId, String> {
    let cwd = Path::new(&cwd)
        .canonicalize()
        .map_err(|error| format!("invalid working directory: {error}"))?;
    if !cwd.is_dir() {
        return Err("working directory is not a directory".into());
    }
    let shell = env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
    backend.spawn(
        SpawnOptions {
            program: shell,
            args: Vec::new(),
            cwd,
            cols,
            rows,
        },
        Box::new(move |bytes| {
            on_output
                .send(Response::new(bytes.to_vec()))
                .map_err(|error| error.to_string())
        }),
    )
}

#[tauri::command]
fn terminal_write(
    backend: State<'_, TerminalBackend>,
    id: TerminalId,
    bytes: Vec<u8>,
) -> Result<(), String> {
    backend.write(id, &bytes)
}

#[tauri::command]
fn terminal_resize(
    backend: State<'_, TerminalBackend>,
    id: TerminalId,
    cols: u16,
    rows: u16,
) -> Result<(u16, u16), String> {
    backend.resize(id, cols, rows)
}

#[tauri::command]
fn terminal_metrics(
    backend: State<'_, TerminalBackend>,
    id: TerminalId,
) -> Result<TerminalMetrics, String> {
    backend.metrics(id)
}

#[tauri::command]
fn terminal_close(backend: State<'_, TerminalBackend>, id: TerminalId) -> Result<(), String> {
    backend.terminate(id)
}

fn main() {
    tauri::Builder::default()
        .manage(TerminalBackend::default())
        .invoke_handler(tauri::generate_handler![
            default_cwd,
            terminal_start,
            terminal_write,
            terminal_resize,
            terminal_metrics,
            terminal_close
        ])
        .run(tauri::generate_context!())
        .expect("failed to run terminal spike");
}
