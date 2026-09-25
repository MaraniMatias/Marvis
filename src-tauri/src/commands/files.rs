use std::path::PathBuf;

use tauri::State;

use crate::{
    domain::{files::FileContent, files::FileTree, ipc::IpcError},
    persistence::Database,
    services,
};

#[tauri::command]
pub async fn files_list(
    checkout_id: String,
    path: String,
    database: State<'_, Database>,
) -> Result<FileTree, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::list(&database, &checkout_id, &path)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn file_read(
    checkout_id: String,
    path: PathBuf,
    database: State<'_, Database>,
) -> Result<FileContent, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::read(&database, &checkout_id, &path)
    })
    .await
    .map_err(operation_error)?
}

fn operation_error(error: tauri::Error) -> IpcError {
    crate::domain::ipc::IpcError::new(
        crate::domain::ipc::IpcErrorCode::OperationFailed,
        format!("file operation could not complete: {error}"),
    )
}
