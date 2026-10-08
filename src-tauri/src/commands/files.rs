use std::path::PathBuf;

use tauri::State;

use crate::{
    domain::{
        files::{CheckoutImage, FileContent, FileProbe, FileTree},
        ipc::IpcError,
    },
    persistence::Database,
    services::{self, files::ReviewRoot},
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
pub async fn file_read_media(
    checkout_id: String,
    path: String,
    database: State<'_, Database>,
) -> Result<tauri::ipc::Response, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::read_media(&database, &checkout_id, &path).map(tauri::ipc::Response::new)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn file_read(
    checkout_id: String,
    path: PathBuf,
    origin: String,
    database: State<'_, Database>,
    review_root: State<'_, ReviewRoot>,
) -> Result<FileContent, IpcError> {
    let database = database.inner().clone();
    let review_root = review_root.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::read(&database, &checkout_id, &origin, &path, &review_root)
    })
    .await
    .map_err(operation_error)?
}

/// Whether a path a terminal printed opens in the preview, with the checkout-relative path to
/// open. `None` is the common answer and not an error: the path may name nothing, or name a file
/// the preview cannot draw, and a hover has to be able to hear that without being told it failed.
#[tauri::command]
pub async fn file_probe(
    checkout_id: String,
    path: String,
    database: State<'_, Database>,
) -> Result<Option<FileProbe>, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::probe(&database, &checkout_id, &path)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn file_write(
    checkout_id: String,
    path: PathBuf,
    content: String,
    expected_content: String,
    origin: String,
    database: State<'_, Database>,
    review_root: State<'_, ReviewRoot>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    let review_root = review_root.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::write(
            &database,
            &checkout_id,
            &origin,
            &path,
            &content,
            &expected_content,
            &review_root,
        )
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_export_markdown(
    date: String,
    timestamp: String,
    markdown: String,
    review_root: State<'_, ReviewRoot>,
) -> Result<String, IpcError> {
    let review_root = review_root.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::export_review_markdown(&review_root, &date, &timestamp, &markdown)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_root_path(review_root: State<'_, ReviewRoot>) -> Result<String, IpcError> {
    let review_root = review_root.0.clone();
    tauri::async_runtime::spawn_blocking(move || services::files::review_root_path(&review_root))
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
