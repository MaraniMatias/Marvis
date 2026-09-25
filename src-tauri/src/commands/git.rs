use tauri::{AppHandle, State};

use crate::{
    domain::ipc::{IpcError, IpcErrorCode},
    persistence::Database,
    services::git::{self, GitFileDiff, GitStatus, GitWatcherManager},
};

#[tauri::command]
pub async fn git_status(
    checkout_id: String,
    database: State<'_, Database>,
) -> Result<GitStatus, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || git::status(&database, &checkout_id))
        .await
        .map_err(operation_error)?
}

#[tauri::command]
pub async fn git_diff(
    checkout_id: String,
    path: String,
    database: State<'_, Database>,
) -> Result<GitFileDiff, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || git::diff(&database, &checkout_id, &path))
        .await
        .map_err(operation_error)?
}

#[tauri::command]
pub async fn git_watch_checkout(
    checkout_id: String,
    app: AppHandle,
    database: State<'_, Database>,
    watchers: State<'_, std::sync::Arc<GitWatcherManager>>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    let watchers = watchers.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let paths = git::watch_paths(&database, &checkout_id)?;
        watchers.watch(app, checkout_id, paths)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn git_unwatch_checkout(
    checkout_id: String,
    watchers: State<'_, std::sync::Arc<GitWatcherManager>>,
) -> Result<(), IpcError> {
    let watchers = watchers.inner().clone();
    tauri::async_runtime::spawn_blocking(move || watchers.unwatch(&checkout_id))
        .await
        .map_err(operation_error)
}

fn operation_error(error: tauri::Error) -> IpcError {
    IpcError::new(
        IpcErrorCode::OperationFailed,
        format!("Git operation could not complete: {error}"),
    )
}
