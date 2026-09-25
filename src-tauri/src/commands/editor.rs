use tauri::State;

use crate::{
    domain::ipc::IpcError,
    persistence::Database,
    services::editor::{self, EditorAvailability},
};

#[tauri::command]
pub async fn editor_availability() -> EditorAvailability {
    tauri::async_runtime::spawn_blocking(editor::availability)
        .await
        .unwrap_or(EditorAvailability {
            zed: false,
            neovim: false,
        })
}

#[tauri::command]
pub async fn editor_open_zed(
    checkout_id: String,
    file_path: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    database: State<'_, Database>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        editor::open_in_zed(&database, &checkout_id, file_path.as_deref(), line, column)
    })
    .await
    .map_err(|error| {
        IpcError::new(
            crate::domain::ipc::IpcErrorCode::OperationFailed,
            format!("could not open Zed: {error}"),
        )
    })?
}
