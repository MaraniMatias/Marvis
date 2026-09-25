use tauri::{
    ipc::{Channel, Response},
    State,
};

use crate::{
    domain::workspace::{Session, TerminalSessionStatus, WorkspaceState},
    persistence::Database,
    services::terminal,
    terminal::{OutputSink, TerminalBackend},
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedTerminal {
    session: Session,
    workspace: WorkspaceState,
}

#[tauri::command]
pub async fn terminal_create(
    checkout_id: String,
    cols: u16,
    rows: u16,
    on_output: Channel<Response>,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<CreatedTerminal, String> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output: OutputSink = Box::new(move |bytes| {
            on_output
                .send(Response::new(bytes.to_vec()))
                .map_err(|error| error.to_string())
        });
        terminal::create(&database, &backend, &checkout_id, cols, rows, output).map(|created| {
            CreatedTerminal {
                session: created.session,
                workspace: created.workspace,
            }
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_write(
    session_id: String,
    bytes: Vec<u8>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<(), String> {
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || backend.write(&session_id, &bytes))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_resize(
    session_id: String,
    cols: u16,
    rows: u16,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<(), String> {
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        backend.resize(&session_id, cols, rows).map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_status(
    session_id: String,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<TerminalSessionStatus, String> {
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || backend.status(&session_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_close(
    session_id: String,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<WorkspaceState, String> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || terminal::close(&database, &backend, &session_id))
        .await
        .map_err(|error| error.to_string())?
}
