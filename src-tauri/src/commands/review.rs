use std::sync::Arc;

use tauri::State;

use crate::{
    domain::{ipc::IpcError, review::ReviewRound},
    persistence::Database,
    services::{
        agent::AgentService,
        review::{self, NewReviewNote, NoteAnchorCheck},
        review_round,
    },
};

fn operation_error(error: impl std::fmt::Display) -> IpcError {
    IpcError::new(
        crate::domain::ipc::IpcErrorCode::OperationFailed,
        error.to_string(),
    )
}

#[tauri::command]
pub async fn review_notes(
    checkout_id: String,
    database: State<'_, Database>,
) -> Result<Vec<crate::domain::review::ReviewNote>, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || review::notes(&database, &checkout_id))
        .await
        .map_err(operation_error)?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewNoteCreateRequest {
    checkout_id: String,
    path: String,
    side: String,
    line_start: i64,
    line_end: Option<i64>,
    content: String,
    code: String,
}

#[tauri::command]
pub async fn review_note_create(
    request: ReviewNoteCreateRequest,
    database: State<'_, Database>,
) -> Result<crate::domain::review::ReviewNote, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        review::add_note(
            &database,
            &request.checkout_id,
            NewReviewNote {
                path: request.path,
                side: request.side,
                line_start: request.line_start,
                line_end: request.line_end,
                content: request.content,
                code: request.code,
            },
        )
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_note_update(
    checkout_id: String,
    id: String,
    content: String,
    database: State<'_, Database>,
) -> Result<crate::domain::review::ReviewNote, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        review::update_note(&database, &checkout_id, &id, &content)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_note_delete(
    checkout_id: String,
    id: String,
    database: State<'_, Database>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || review::delete_note(&database, &checkout_id, &id))
        .await
        .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_notes_mark_sent(
    checkout_id: String,
    ids: Vec<String>,
    database: State<'_, Database>,
) -> Result<(), IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || review::mark_sent(&database, &checkout_id, &ids))
        .await
        .map_err(operation_error)?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAnchorCheckRequest {
    pub id: String,
    pub current_code: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAnchorsVerifyRequest {
    pub checkout_id: String,
    pub path: String,
    pub checks: Vec<ReviewAnchorCheckRequest>,
}

#[tauri::command]
pub async fn review_note_anchors_verify(
    request: ReviewAnchorsVerifyRequest,
    database: State<'_, Database>,
) -> Result<Vec<crate::domain::review::ReviewNote>, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let checks: Vec<NoteAnchorCheck> = request
            .checks
            .into_iter()
            .map(|check| NoteAnchorCheck {
                id: check.id,
                current_code: check.current_code,
            })
            .collect();
        review::verify_note_anchors(&database, &request.checkout_id, &request.path, &checks)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_note_outdated_clear(
    checkout_id: String,
    id: String,
    database: State<'_, Database>,
) -> Result<crate::domain::review::ReviewNote, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        review::clear_outdated(&database, &checkout_id, &id)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_note_resolve(
    checkout_id: String,
    id: String,
    database: State<'_, Database>,
) -> Result<crate::domain::review::ReviewNote, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        review_round::resolve_note(&database, &checkout_id, &id)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_rounds(
    checkout_id: String,
    database: State<'_, Database>,
) -> Result<Vec<ReviewRound>, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || review_round::rounds(&database, &checkout_id))
        .await
        .map_err(operation_error)?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRoundDispatchRequest {
    pub checkout_id: String,
    /// The session chosen for this round; must belong to this checkout's server.
    pub session_id: String,
    pub ids: Vec<String>,
    /// The exported review. Built by the caller, which owns the Markdown format.
    pub markdown: String,
}

/// Delivers one review round to one agent session as a single message.
///
/// The order is deliberate: the round and its marker are recorded *before* the agent is
/// called, so an interrupted send is recognizable afterwards instead of being repeated.
#[tauri::command]
pub async fn review_round_dispatch(
    database: State<'_, Database>,
    agents: State<'_, Arc<AgentService>>,
    request: ReviewRoundDispatchRequest,
) -> Result<ReviewRound, IpcError> {
    let database = database.inner().clone();
    let agents: Arc<AgentService> = Arc::clone(agents.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let directory = super::agent::checkout_directory(&database, &request.checkout_id)?;
        // Checked against the marker that will be prepended, so the limit is the real one.
        review_round::check_prompt_size(
            &(request.markdown.clone() + &"x".repeat(review_round::marker_budget())),
        )?;
        // `begin_round` links the notes, which is also what rejects a foreign note id.
        let round = review_round::begin_round(
            &database,
            &request.checkout_id,
            &request.session_id,
            &request.ids,
        )?;
        let prompt = review_round::build_round_prompt(&request.markdown, &round.marker);
        match agents.prompt(
            &request.checkout_id,
            &directory,
            &request.session_id,
            &prompt,
        ) {
            Ok(_) => review_round::confirm_round(&database, &request.checkout_id, &round.id),
            // Left as `dispatching` on purpose: the marker is what lets reconciliation learn
            // whether this send landed, and requeueing blindly could duplicate it.
            Err(error) => Err(review_round::agent_error(error)),
        }
    })
    .await
    .map_err(operation_error)?
}

/// Marks every unconfirmed round of the checkout as retryable. Called on reconnect.
#[tauri::command]
pub async fn review_rounds_requeue(
    checkout_id: String,
    database: State<'_, Database>,
) -> Result<usize, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || review_round::requeue_all(&database, &checkout_id))
        .await
        .map_err(operation_error)?
}

/// Asks the session whether a round's message actually arrived, and acts on the answer.
#[tauri::command]
pub async fn review_round_reconcile(
    database: State<'_, Database>,
    agents: State<'_, Arc<AgentService>>,
    checkout_id: String,
    round_id: String,
) -> Result<ReviewRound, IpcError> {
    let database = database.inner().clone();
    let agents: Arc<AgentService> = Arc::clone(agents.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let directory = super::agent::checkout_directory(&database, &checkout_id)?;
        review_round::reconcile_round(&database, &agents, &checkout_id, &directory, &round_id)
    })
    .await
    .map_err(operation_error)?
}

#[tauri::command]
pub async fn review_round_ack(
    checkout_id: String,
    round_id: String,
    database: State<'_, Database>,
) -> Result<ReviewRound, IpcError> {
    let database = database.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        review_round::ack_round(&database, &checkout_id, &round_id)
    })
    .await
    .map_err(operation_error)?
}
