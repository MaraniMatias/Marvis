//! Review rounds: one batch of notes delivered to an agent as a single message.
//!
//! The round is recorded *before* the agent is called, and its marker is embedded in the
//! prompt. That ordering is the whole point: if Marvis dies mid-send, the round is left in
//! `dispatching` and reconciliation can ask the session whether the message actually
//! landed, instead of guessing and sending the review twice.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        review::{
            review_round_marker, ReviewNote, ReviewRound, AGENT_PROMPT_PREFIX, MAX_ROUND_NOTES,
            MAX_ROUND_PROMPT_BYTES, ROUND_MARKER_PREFIX,
        },
    },
    persistence::{timestamp, Database},
    services::agent::AgentService,
};

static NEXT_ROUND_ID: AtomicU64 = AtomicU64::new(1);

fn invalid(message: &str) -> IpcError {
    IpcError::new(IpcErrorCode::InvalidPath, message)
}

fn foreign(message: &str) -> IpcError {
    IpcError::new(IpcErrorCode::CheckoutOwnershipMismatch, message)
}

fn failed(message: impl Into<String>) -> IpcError {
    IpcError::new(IpcErrorCode::OperationFailed, message)
}

/// Builds the prompt for a round: the fixed instructions, the round marker, and the notes.
///
/// `marker` already carries the `marvis-review:` prefix (see `review_round_marker`), so it
/// is embedded verbatim: the deduplication key must be exactly what the session will hold.
pub fn build_round_prompt(review_markdown: &str, marker: &str) -> String {
    format!("{AGENT_PROMPT_PREFIX}\n[{marker}]\n{review_markdown}")
}

pub fn rounds(database: &Database, checkout_id: &str) -> Result<Vec<ReviewRound>, IpcError> {
    database
        .review_rounds(checkout_id)
        .map_err(|error| IpcError::new(IpcErrorCode::InvalidCheckout, error))
}

/// Records a round as `dispatching` with its marker, and links its notes to it.
///
/// The notes are marked `sent` here too, before the agent is called, so a crash cannot leave
/// notes looking delivered when they were not, nor undelivered when they were.
pub fn begin_round(
    database: &Database,
    checkout_id: &str,
    session_id: &str,
    note_ids: &[String],
) -> Result<ReviewRound, IpcError> {
    if note_ids.is_empty() || note_ids.len() > MAX_ROUND_NOTES {
        return Err(failed(format!(
            "a review round needs between 1 and {MAX_ROUND_NOTES} notes"
        )));
    }
    if session_id.is_empty() {
        return Err(invalid("a review round needs a target session"));
    }
    let now = timestamp();
    let id = format!(
        "round:{}:{}:{}",
        now,
        std::process::id(),
        NEXT_ROUND_ID.fetch_add(1, Ordering::Relaxed)
    );
    let round = ReviewRound {
        id: id.clone(),
        checkout_id: checkout_id.to_string(),
        session_id: Some(session_id.to_string()),
        status: "dispatching".into(),
        marker: review_round_marker(&id),
        note_ids: note_ids.to_vec(),
        created_at: now.clone(),
        updated_at: now,
    };
    database
        .add_review_round(&round, note_ids)
        .map_err(|error| foreign(&error))
}

/// Records that the message reached the agent.
pub fn confirm_round(
    database: &Database,
    checkout_id: &str,
    round_id: &str,
) -> Result<ReviewRound, IpcError> {
    database
        .set_review_round_status(round_id, checkout_id, "dispatching", "dispatched")
        .map_err(|error| foreign(&error))
}

/// Closes a round once its turn finished and the notes were re-checked.
pub fn ack_round(
    database: &Database,
    checkout_id: &str,
    round_id: &str,
) -> Result<ReviewRound, IpcError> {
    database
        .set_review_round_status(round_id, checkout_id, "dispatched", "acked")
        .map_err(|error| foreign(&error))
}

/// Resolves whether a `dispatching` round actually landed, and requeues it if not.
///
/// Called on reconnect. A round whose marker is in the session's transcript is confirmed;
/// one whose marker is absent never reached the agent, so putting it back to `queued` is
/// safe: the notes are not duplicated because the message was never delivered.
pub fn reconcile_round(
    database: &Database,
    agents: &AgentService,
    checkout_id: &str,
    directory: &std::path::Path,
    round_id: &str,
) -> Result<ReviewRound, IpcError> {
    let round = database
        .review_rounds(checkout_id)
        .map_err(failed)?
        .into_iter()
        .find(|item| item.id == round_id)
        .ok_or_else(|| foreign("review round does not belong to the requested checkout"))?;
    if !round.is_pending() {
        return Ok(round);
    }
    let Some(session_id) = round.session_id.clone() else {
        // Never had a target: nothing to check, so it waits for one.
        return requeue(database, checkout_id, &round);
    };
    let landed = agents
        .session_mentions(checkout_id, directory, &session_id, &round.marker)
        .map_err(agent_error)?;
    if landed {
        database
            .set_review_round_status(round_id, checkout_id, "dispatching", "dispatched")
            .map_err(|error| foreign(&error))
    } else {
        requeue(database, checkout_id, &round)
    }
}

fn requeue(
    database: &Database,
    checkout_id: &str,
    round: &ReviewRound,
) -> Result<ReviewRound, IpcError> {
    if round.status == "queued" {
        return Ok(round.clone());
    }
    database
        .set_review_round_status(&round.id, checkout_id, "dispatching", "queued")
        .map_err(|error| foreign(&error))
}

/// Puts every unconfirmed round of the checkout back to `queued` so they can be retried.
pub fn requeue_all(database: &Database, checkout_id: &str) -> Result<usize, IpcError> {
    database.requeue_review_rounds(checkout_id).map_err(failed)
}

/// Marks an outstanding note as resolved by the user.
pub fn resolve_note(
    database: &Database,
    checkout_id: &str,
    id: &str,
) -> Result<ReviewNote, IpcError> {
    database
        .resolve_review_note(id, checkout_id)
        .map_err(|error| foreign(&error))
}

/// Rejects a prompt that cannot be delivered as one message, before a round is recorded.
pub fn check_prompt_size(prompt: &str) -> Result<(), IpcError> {
    if prompt.trim().is_empty() {
        return Err(failed("a review round cannot be empty"));
    }
    if prompt.len() > MAX_ROUND_PROMPT_BYTES {
        return Err(failed(
            "the review is too large to send to the agent in one message",
        ));
    }
    Ok(())
}

/// The instructions and marker that `build_round_prompt` prepends to a round's review.
///
/// The caller knows the review body but not the marker yet, so this is how much room the
/// prefix needs, letting the size be checked before anything is recorded.
pub fn marker_budget() -> usize {
    AGENT_PROMPT_PREFIX.len() + ROUND_MARKER_PREFIX.len() + 128
}

pub fn agent_error(error: crate::services::agent::BridgeError) -> IpcError {
    crate::services::agent::map_error(error)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::{tempdir, TempDir};

    use crate::{
        domain::review::{review_anchor_hash, ReviewNote},
        persistence::Database,
        services::agent::AgentService,
    };

    use super::{
        ack_round, begin_round, build_round_prompt, check_prompt_size, confirm_round, requeue_all,
        resolve_note, rounds,
    };

    fn note(overrides: &ReviewNote) -> ReviewNote {
        ReviewNote {
            id: format!("note:{}", overrides.id),
            checkout_id: overrides.checkout_id.clone(),
            path: "src/foo.js".into(),
            side: "new".into(),
            line_start: 10,
            line_end: None,
            content: "check this".into(),
            code: "const result = a + b;".into(),
            status: "draft".into(),
            code_hash: review_anchor_hash("const result = a + b;"),
            outdated: false,
            round_id: None,
            created_at: "1".into(),
            updated_at: "1".into(),
        }
    }

    fn database_with_two_checkouts(temp: &TempDir) -> (Database, String, String) {
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let first = crate::domain::workspace::Repo::plain(&first, "now").unwrap();
        let second = crate::domain::workspace::Repo::plain(&second, "now").unwrap();
        database.register_plain_repo(first.clone()).unwrap();
        database.register_plain_repo(second.clone()).unwrap();
        (
            database,
            first.checkouts[0].id.clone(),
            second.checkouts[0].id.clone(),
        )
    }

    fn add_note(database: &Database, checkout_id: &str, id: &str) {
        database
            .add_review_note(&note(&ReviewNote {
                id: id.into(),
                checkout_id: checkout_id.into(),
                path: "src/foo.js".into(),
                side: "new".into(),
                line_start: 10,
                line_end: None,
                content: "check this".into(),
                code: "const result = a + b;".into(),
                status: "draft".into(),
                code_hash: String::new(),
                outdated: false,
                round_id: None,
                created_at: "1".into(),
                updated_at: "1".into(),
            }))
            .unwrap();
    }

    #[test]
    fn a_round_is_recorded_before_it_is_sent_and_carries_its_marker() {
        let temp = tempdir().unwrap();
        let (database, checkout_id, _) = database_with_two_checkouts(&temp);
        add_note(&database, &checkout_id, "a");

        let round = begin_round(
            &database,
            &checkout_id,
            "ses_target",
            &["note:a".to_string()],
        )
        .unwrap();
        assert_eq!(round.status, "dispatching");
        assert_eq!(round.session_id.as_deref(), Some("ses_target"));
        assert!(round.marker.starts_with("marvis-review:"));

        // The note is already linked and marked sent before any agent call happens.
        let stored = &database.review_notes(&checkout_id).unwrap()[0];
        assert_eq!(stored.status, "sent");
        assert_eq!(stored.round_id.as_deref(), Some(round.id.as_str()));

        let prompt = build_round_prompt("# Code Review", &round.marker);
        assert!(prompt.contains(&round.marker));

        let confirmed = confirm_round(&database, &checkout_id, &round.id).unwrap();
        assert_eq!(confirmed.status, "dispatched");
        let acked = ack_round(&database, &checkout_id, &round.id).unwrap();
        assert_eq!(acked.status, "acked");
        // A closed round cannot be moved again.
        assert!(ack_round(&database, &checkout_id, &round.id).is_err());
        assert_eq!(rounds(&database, &checkout_id).unwrap().len(), 1);
    }

    #[test]
    fn an_unconfirmed_round_is_requeued_for_retry() {
        let temp = tempdir().unwrap();
        let (database, checkout_id, _) = database_with_two_checkouts(&temp);
        add_note(&database, &checkout_id, "a");
        let round = begin_round(
            &database,
            &checkout_id,
            "ses_target",
            &["note:a".to_string()],
        )
        .unwrap();

        assert_eq!(requeue_all(&database, &checkout_id).unwrap(), 1);
        let requeued = &rounds(&database, &checkout_id).unwrap()[0];
        assert_eq!(requeued.status, "queued");
        // Requeuing twice must not double count or fail.
        assert_eq!(requeue_all(&database, &checkout_id).unwrap(), 0);
        assert_eq!(requeued.id, round.id);
    }

    #[test]
    fn only_outstanding_notes_of_this_checkout_can_be_resolved() {
        let temp = tempdir().unwrap();
        let (database, checkout_id, other_checkout_id) = database_with_two_checkouts(&temp);
        add_note(&database, &checkout_id, "a");
        add_note(&database, &checkout_id, "b");
        let ids = vec!["note:a".to_string()];
        begin_round(&database, &checkout_id, "ses_target", &ids).unwrap();

        let resolved = resolve_note(&database, &checkout_id, "note:a").unwrap();
        assert_eq!(resolved.status, "resolved");
        // A draft was never outstanding, and another checkout's note is not ours.
        assert!(resolve_note(&database, &checkout_id, "note:b").is_err());
        assert!(resolve_note(&database, &other_checkout_id, "note:a").is_err());
        // Resolving twice is not a transition.
        assert!(resolve_note(&database, &checkout_id, "note:a").is_err());
    }

    #[test]
    fn sec_10_a_round_cannot_carry_notes_from_another_checkout() {
        let temp = tempdir().unwrap();
        let (database, checkout_id, other_checkout_id) = database_with_two_checkouts(&temp);
        add_note(&database, &checkout_id, "a");
        add_note(&database, &other_checkout_id, "foreign");
        let agents = AgentService::new();

        assert!(begin_round(
            &database,
            &checkout_id,
            "ses_target",
            &["note:foreign".to_string()]
        )
        .is_err());
        // Nothing was written, so a foreign note is not even linked.
        assert!(database.review_rounds(&checkout_id).unwrap().is_empty());
        assert_eq!(
            database.review_notes(&checkout_id).unwrap()[0].status,
            "draft"
        );

        // And a round of this checkout cannot be confirmed through another one.
        let round = begin_round(
            &database,
            &checkout_id,
            "ses_target",
            &["note:a".to_string()],
        )
        .unwrap();
        assert!(super::reconcile_round(
            &database,
            &agents,
            &other_checkout_id,
            Path::new("/nonexistent"),
            &round.id
        )
        .is_err());
    }

    #[test]
    fn the_prompt_is_the_instructions_then_the_marker_then_the_review() {
        // The marker is the deduplication key, so it must be inside the message the agent
        // receives and not a side channel Marvis keeps to itself.
        let prompt = build_round_prompt("# Code Review 2026-09-26", "marvis-review:round:1");
        assert!(prompt.starts_with(crate::domain::review::AGENT_PROMPT_PREFIX));
        let marker_at = prompt
            .find("[marvis-review:round:1]")
            .expect("marker in the prompt");
        let review_at = prompt.find("# Code Review").expect("review in the prompt");
        assert!(
            marker_at < review_at,
            "the marker must come before the review"
        );
    }

    #[test]
    fn a_round_needs_a_target_and_at_least_one_note() {
        let temp = tempdir().unwrap();
        let (database, checkout_id, _) = database_with_two_checkouts(&temp);
        add_note(&database, &checkout_id, "a");

        assert!(begin_round(&database, &checkout_id, "ses_target", &[]).is_err());
        assert!(begin_round(&database, &checkout_id, "", &["note:a".into()]).is_err());
        assert!(check_prompt_size("   ").is_err());
        assert!(check_prompt_size("fix the review").is_ok());
        assert!(check_prompt_size(&"x".repeat(600 * 1024)).is_err());
    }
}
