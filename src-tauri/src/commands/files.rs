use std::path::PathBuf;

use tauri::State;

use crate::{
    config::{ConfigFile, REVIEW_STORAGE_WORKDIR},
    domain::{
        files::{
            CheckoutImage, FileContent, FileProbe, FileTree, PrettierConfig, ReviewFolder,
            ReviewFolderCleared,
        },
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
    config_file: State<'_, ConfigFile>,
) -> Result<FileContent, IpcError> {
    let database = database.inner().clone();
    let config_file = config_file.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let places = services::files::ReviewPlaces::load(&config_file)?;
        services::files::read(&database, &checkout_id, &origin, &path, &places)
    })
    .await
    .map_err(operation_error)?
}

/// The Prettier options that govern a file, read from the nearest config above it. `None` is
/// the answer for a checkout that configures nothing, which is not an error: Prettier has defaults
/// and this is how the caller learns it may use them.
#[tauri::command]
pub async fn file_read_prettier_config(
    checkout_id: String,
    path: String,
    origin: String,
    database: State<'_, Database>,
    config_file: State<'_, ConfigFile>,
) -> Result<Option<PrettierConfig>, IpcError> {
    let database = database.inner().clone();
    let config_file = config_file.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let places = services::files::ReviewPlaces::load(&config_file)?;
        services::files::read_prettier_config(&database, &checkout_id, &origin, &path, &places)
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

/// Opens a file or a folder of a checkout in the application this machine has for it, which is the
/// only place one can be opened: this window is a preview, not the program that draws a `.psd`.
///
/// The name is checkout-relative and is resolved on this side, where the checkout's own boundary
/// is enforced, rather than handed to the opener as it arrived. The opener is a process and its
/// status is read, so the call waits on a blocking thread for the same reason `open_url` does.
#[tauri::command]
pub async fn file_open_externally(
    checkout_id: String,
    path: String,
    database: State<'_, Database>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        services::files::open_externally(&database, &checkout_id, &path)
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
    config_file: State<'_, ConfigFile>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    let config_file = config_file.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let places = services::files::ReviewPlaces::load(&config_file)?;
        services::files::write(
            &database,
            &checkout_id,
            &origin,
            &path,
            &content,
            &expected_content,
            &places,
        )
    })
    .await
    .map_err(operation_error)?
}

/// Writes one exported review round and answers the file it wrote.
///
/// The folder is the one the preference names for this checkout, so an export is a decision made
/// once rather than a question asked per round.
#[tauri::command]
pub async fn review_export_markdown(
    checkout_id: String,
    date: String,
    timestamp: String,
    markdown: String,
    database: State<'_, Database>,
    config_file: State<'_, ConfigFile>,
) -> Result<String, IpcError> {
    let database = database.inner().clone();
    let config_file = config_file.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let places = services::files::ReviewPlaces::load(&config_file)?;
        let root = services::files::review_root_for(&database, &checkout_id, &places)?;
        services::files::export_review_markdown(&root, &date, &timestamp, &markdown)
    })
    .await
    .map_err(operation_error)?
}

/// Where exported reviews are kept and what is in there, for the settings dialog.
///
/// `storage` is the mode the row is showing rather than the one on file, so a person changing the
/// selector sees the folder they are choosing before they Apply. It is read as a mode and not as a
/// path: an answer that is neither of the two is the saved one.
#[tauri::command]
pub async fn review_folder(
    checkout_id: String,
    storage: Option<String>,
    database: State<'_, Database>,
    config_file: State<'_, ConfigFile>,
) -> Result<ReviewFolder, IpcError> {
    let database = database.inner().clone();
    let config_file = config_file.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut places = services::files::ReviewPlaces::load(&config_file)?;
        if storage.as_deref() == Some(REVIEW_STORAGE_WORKDIR) {
            places.set_storage(REVIEW_STORAGE_WORKDIR);
        }
        services::files::review_folder(&database, &checkout_id, &places)
    })
    .await
    .map_err(operation_error)?
}

/// Deletes the exported reviews in the app's own folder. Refused for a working directory: those
/// notes belong to the repository, and the button that asks for this is not drawn there.
#[tauri::command]
pub async fn review_folder_clear(
    checkout_id: String,
    database: State<'_, Database>,
    config_file: State<'_, ConfigFile>,
) -> Result<ReviewFolderCleared, IpcError> {
    let database = database.inner().clone();
    let config_file = config_file.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let places = services::files::ReviewPlaces::load(&config_file)?;
        services::files::clear_review_folder(&database, &checkout_id, &places)
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
