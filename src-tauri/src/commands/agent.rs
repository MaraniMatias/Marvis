use std::path::PathBuf;

use tauri::State;

use crate::{
    domain::{
        agent::AgentSession,
        ipc::{IpcError, IpcErrorCode},
    },
    persistence::Database,
    services::agent::{self, AgentService},
};
use std::sync::Arc;

fn operation_error(error: impl std::fmt::Display) -> IpcError {
    IpcError::new(IpcErrorCode::OperationFailed, error.to_string())
}

/// The directory a checkout lives in, resolved once and reused by every agent call.
pub fn checkout_directory(database: &Database, checkout_id: &str) -> Result<PathBuf, IpcError> {
    database
        .terminal_checkout_path(checkout_id)
        .map_err(|error| IpcError::new(IpcErrorCode::InvalidCheckout, error))
}

#[tauri::command]
pub async fn agent_sessions(
    checkout_id: String,
    database: State<'_, Database>,
    agents: State<'_, Arc<AgentService>>,
) -> Result<Vec<AgentSession>, IpcError> {
    let database = database.inner().clone();
    let agents: Arc<AgentService> = Arc::clone(agents.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let directory = checkout_directory(&database, &checkout_id)?;
        agents
            .sessions(&checkout_id, &directory)
            .map_err(agent::map_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn agent_session_create(
    checkout_id: String,
    title: String,
    database: State<'_, Database>,
    agents: State<'_, Arc<AgentService>>,
) -> Result<AgentSession, IpcError> {
    let database = database.inner().clone();
    let agents: Arc<AgentService> = Arc::clone(agents.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let directory = checkout_directory(&database, &checkout_id)?;
        agents
            .create_session(&checkout_id, &directory, &title)
            .map_err(agent::map_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn agent_prompt(
    checkout_id: String,
    session_id: String,
    text: String,
    database: State<'_, Database>,
    agents: State<'_, Arc<AgentService>>,
) -> Result<AgentSession, IpcError> {
    let database = database.inner().clone();
    let agents: Arc<AgentService> = Arc::clone(agents.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let directory = checkout_directory(&database, &checkout_id)?;
        agents
            .prompt(&checkout_id, &directory, &session_id, &text)
            .map_err(agent::map_error)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn agent_stop(
    checkout_id: String,
    agents: State<'_, Arc<AgentService>>,
) -> Result<(), IpcError> {
    let agents: Arc<AgentService> = Arc::clone(agents.inner());
    tauri::async_runtime::spawn_blocking(move || agents.stop(&checkout_id))
        .await
        .map_err(operation_error)
}
