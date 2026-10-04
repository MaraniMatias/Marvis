use tauri::{
    ipc::{Channel, Response},
    State,
};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        terminal_layout::CheckoutTerminalLayout,
        workspace::{Session, TerminalSessionStatus, WorkspaceState},
    },
    persistence::Database,
    services::terminal,
    terminal::{OutputSink, TerminalBackend},
};

#[tauri::command]
pub async fn terminal_layout_load(
    checkout_id: String,
    database: State<'_, Database>,
) -> Result<Option<CheckoutTerminalLayout>, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        database
            .load_terminal_layout(&checkout_id)
            .map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn terminal_layout_save(
    checkout_id: String,
    layout: CheckoutTerminalLayout,
    database: State<'_, Database>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        database
            .save_terminal_layout(&checkout_id, &layout)
            .map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedTerminal {
    session: Session,
    workspace: WorkspaceState,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateRequest {
    checkout_id: String,
    cols: u16,
    rows: u16,
    prompt: Option<String>,
}

#[tauri::command]
pub async fn terminal_create(
    request: TerminalCreateRequest,
    on_output: Channel<Response>,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<CreatedTerminal, IpcError> {
    validate_dimensions(request.cols, request.rows)?;
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output: OutputSink = Box::new(move |bytes| {
            on_output
                .send(Response::new(bytes.to_vec()))
                .map_err(|error| error.to_string())
        });
        terminal::create_with_options(
            &database,
            &backend,
            &request.checkout_id,
            terminal::TerminalOptions {
                cols: request.cols,
                rows: request.rows,
                prompt: request.prompt,
            },
            output,
        )
        .map(|created| CreatedTerminal {
            session: created.session,
            workspace: created.workspace,
        })
        .map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn terminal_write(
    checkout_id: String,
    session_id: String,
    bytes: Vec<u8>,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        backend.write(&session_id, &bytes).map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn terminal_resize(
    checkout_id: String,
    session_id: String,
    cols: u16,
    rows: u16,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        validate_dimensions(cols, rows)?;
        backend
            .resize(&session_id, cols, rows)
            .map(|_| ())
            .map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn terminal_status(
    checkout_id: String,
    session_id: String,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<TerminalSessionStatus, IpcError> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        backend.status(&session_id).map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn terminal_close(
    checkout_id: String,
    session_id: String,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        terminal::close(&database, &backend, &session_id).map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn terminal_rename(
    checkout_id: String,
    session_id: String,
    name: String,
    database: State<'_, Database>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        terminal::rename(&database, &session_id, &name).map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn terminal_move(
    checkout_id: String,
    session_id: String,
    target_checkout_id: String,
    database: State<'_, Database>,
) -> Result<WorkspaceState, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        database
            .move_terminal_session(&session_id, &target_checkout_id)
            .map_err(operation_error)
    })
    .await
    .map_err(operation_error)?
}

fn validate_session_owner(
    database: &Database,
    session_id: &str,
    checkout_id: &str,
) -> Result<(), IpcError> {
    let owner = database
        .terminal_session_checkout(session_id)
        .map_err(operation_error)?
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::TerminalSessionMissing,
                "terminal session ID is not registered",
            )
        })?;
    if owner != checkout_id {
        return Err(IpcError::new(
            IpcErrorCode::TerminalOwnershipMismatch,
            "terminal session does not belong to the requested checkout",
        ));
    }
    Ok(())
}

fn validate_dimensions(cols: u16, rows: u16) -> Result<(), IpcError> {
    if cols == 0 || rows == 0 {
        return Err(IpcError::new(
            IpcErrorCode::InvalidTerminalDimensions,
            "PTY dimensions must be non-zero",
        ));
    }
    Ok(())
}

fn operation_error(error: impl std::fmt::Display) -> IpcError {
    IpcError::new(IpcErrorCode::OperationFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{domain::workspace::Repo, persistence::Database};

    use super::{validate_dimensions, validate_session_owner};

    #[test]
    fn invalid_dimensions_have_a_typed_ipc_error() {
        let error = validate_dimensions(0, 24).unwrap_err();
        assert_eq!(
            error.code,
            crate::domain::ipc::IpcErrorCode::InvalidTerminalDimensions
        );
        assert_eq!(error.message, "PTY dimensions must be non-zero");
    }

    #[test]
    fn sec_06_session_ids_are_validated_against_the_requested_checkout() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let first = Repo::plain(&first, "now").unwrap();
        let second = Repo::plain(&second, "now").unwrap();
        database.register_plain_repo(first.clone()).unwrap();
        database.register_plain_repo(second.clone()).unwrap();
        database
            .add_terminal_session(&crate::domain::workspace::Session {
                id: "session:first".into(),
                session_type: crate::domain::workspace::SessionType::Shell,
                checkout_id: first.checkouts[0].id.clone(),
                name: "shell".into(),
                created_at: "now".into(),
                status: crate::domain::workspace::SessionStatus::Active,
            })
            .unwrap();

        assert!(validate_session_owner(&database, "session:first", &first.checkouts[0].id).is_ok());
        let missing = validate_session_owner(&database, "session:unknown", &first.checkouts[0].id)
            .unwrap_err();
        assert_eq!(
            missing.code,
            crate::domain::ipc::IpcErrorCode::TerminalSessionMissing
        );
        let foreign = validate_session_owner(&database, "session:first", &second.checkouts[0].id)
            .unwrap_err();
        assert_eq!(
            foreign.code,
            crate::domain::ipc::IpcErrorCode::TerminalOwnershipMismatch
        );
        assert_eq!(
            foreign.message,
            "terminal session does not belong to the requested checkout"
        );
        assert_eq!(
            serde_json::to_value(foreign).unwrap(),
            serde_json::json!({
                "code": "terminal_ownership_mismatch",
                "message": "terminal session does not belong to the requested checkout"
            })
        );
    }
}
