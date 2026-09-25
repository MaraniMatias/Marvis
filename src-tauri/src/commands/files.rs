use std::path::PathBuf;

use tauri::State;

use crate::{
    domain::{
        files::{CheckoutImage, FileContent, FileSearchResult, FileTree},
        ipc::IpcError,
    },
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
pub async fn files_search(
    checkout_id: String,
    database: State<'_, Database>,
) -> Result<FileSearchResult, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || services::files::search(&database, &checkout_id))
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

#[tauri::command]
pub async fn file_read_markdown_image(
    checkout_id: String,
    markdown_path: String,
    image_path: String,
    database: State<'_, Database>,
) -> Result<CheckoutImage, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::read_markdown_image(&database, &checkout_id, &markdown_path, &image_path)
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
