use std::path::PathBuf;

use tauri::State;

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        workspace::WorkspaceState,
    },
    persistence::Database,
    services,
};

#[tauri::command]
pub async fn restore_workspace(database: State<'_, Database>) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || services::workspace::restore(&database))
        .await
        .map_err(operation_error)?
}

#[tauri::command]
pub async fn register_folder(
    path: PathBuf,
    database: State<'_, Database>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::workspace::register_folder(&database, &path)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn locate_missing_checkout(
    checkout_id: String,
    path: PathBuf,
    database: State<'_, Database>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::workspace::locate_missing_checkout(&database, &checkout_id, &path)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn close_missing_checkout(
    checkout_id: String,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<crate::terminal::TerminalBackend>>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::workspace::close_missing_checkout(&database, &backend, &checkout_id)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn set_default_branch(
    repo_id: String,
    branch: String,
    database: State<'_, Database>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::workspace::set_default_branch(&database, &repo_id, &branch)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn select_checkout(
    checkout_id: Option<String>,
    database: State<'_, Database>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        database
            .select_checkout(checkout_id)
            .map_err(|error| IpcError::new(IpcErrorCode::InvalidCheckout, error))
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn select_session(
    session_id: Option<String>,
    database: State<'_, Database>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        database
            .select_session(session_id)
            .map_err(|error| IpcError::new(IpcErrorCode::InvalidCheckout, error))
    })
    .await
    .map_err(operation_error)?
}

fn operation_error(error: tauri::Error) -> IpcError {
    IpcError::new(
        IpcErrorCode::OperationFailed,
        format!("workspace operation could not complete: {error}"),
    )
}
