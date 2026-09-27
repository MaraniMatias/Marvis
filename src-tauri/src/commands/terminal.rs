use tauri::{
    ipc::{Channel, Response},
    State,
};

use crate::{
    domain::terminal_layout::CheckoutTerminalLayout,
    domain::workspace::{Session, TerminalSessionStatus, WorkspaceState},
    persistence::Database,
    services::terminal,
    terminal::{OutputSink, TerminalBackend},
};

#[tauri::command]
pub async fn terminal_layout_load(
    checkout_id: String,
    database: State<'_, Database>,
) -> Result<Option<CheckoutTerminalLayout>, String> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || database.load_terminal_layout(&checkout_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_layout_save(
    checkout_id: String,
    layout: CheckoutTerminalLayout,
    database: State<'_, Database>,
) -> Result<(), String> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.save_terminal_layout(&checkout_id, &layout)
    })
    .await
    .map_err(|error| error.to_string())?
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
) -> Result<CreatedTerminal, String> {
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
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_write(
    checkout_id: String,
    session_id: String,
    bytes: Vec<u8>,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<(), String> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        backend.write(&session_id, &bytes)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_resize(
    checkout_id: String,
    session_id: String,
    cols: u16,
    rows: u16,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<(), String> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        backend.resize(&session_id, cols, rows).map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_status(
    checkout_id: String,
    session_id: String,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<TerminalSessionStatus, String> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        backend.status(&session_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_close(
    checkout_id: String,
    session_id: String,
    database: State<'_, Database>,
    backend: State<'_, std::sync::Arc<TerminalBackend>>,
) -> Result<WorkspaceState, String> {
    let database = database.inner().clone();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        terminal::close(&database, &backend, &session_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn terminal_rename(
    checkout_id: String,
    session_id: String,
    name: String,
    database: State<'_, Database>,
) -> Result<WorkspaceState, String> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_session_owner(&database, &session_id, &checkout_id)?;
        terminal::rename(&database, &session_id, &name)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn validate_session_owner(
    database: &Database,
    session_id: &str,
    checkout_id: &str,
) -> Result<(), String> {
    let owner = database
        .terminal_session_checkout(session_id)?
        .ok_or_else(|| "terminal session ID is not registered".to_string())?;
    if owner != checkout_id {
        return Err("terminal session does not belong to the requested checkout".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{domain::workspace::Repo, persistence::Database};

    use super::validate_session_owner;

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
        assert!(
            validate_session_owner(&database, "session:unknown", &first.checkouts[0].id).is_err()
        );
        assert!(
            validate_session_owner(&database, "session:first", &second.checkouts[0].id).is_err()
        );
    }
}
