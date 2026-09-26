use serde::Serialize;

/// One OpenCode session Marvis knows about, scoped to the checkout that owns it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    /// OpenCode's own `ses_…` id, only ever valid inside its checkout's server.
    pub id: String,
    pub checkout_id: String,
    pub title: String,
    /// When the server last saw this session go idle, if ever.
    ///
    /// Absent means "no turn has finished yet", which covers both a session that never ran
    /// and one that is working right now. Only the client, which has seen a turn start, can
    /// tell those apart, so this is the fact rather than a derived verdict.
    pub idle_at: Option<i64>,
    /// Set when a permission request arrived and Marvis could not answer it.
    pub blocked_on_permission: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A normalized event from the checkout's server, with the server's own payload
/// kept intact so the UI can render tool names and permission resources.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub checkout_id: String,
    pub session_id: Option<String>,
    /// Normalized kind the UI branches on; see `agent_event_kind`.
    pub kind: AgentEventKind,
    /// The original SSE `type`, kept for diagnostics and unknown-event passthrough.
    pub raw_type: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentEventKind {
    /// Anything Marvis does not model yet. The UI ignores it safely.
    Unknown,
    /// A turn started: `session.execution.started` or a new `session.step.started`.
    TurnStarted,
    /// A turn ended successfully.
    TurnFinished,
    /// A turn ended in an error. `data` carries the server's `error` object.
    TurnFailed,
    /// A tool was invoked. `data` carries `name` and `input`.
    ToolCalled,
    /// A permission was requested and cannot be answered on this server version.
    PermissionAsked,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub checkout_id: String,
    pub running: bool,
    pub sessions: Vec<AgentSession>,
    /// Why the bridge is not running, when it is not.
    pub error: String,
}

impl AgentStatus {
    pub fn stopped(checkout_id: &str, error: &str) -> Self {
        Self {
            checkout_id: checkout_id.to_string(),
            running: false,
            sessions: Vec::new(),
            error: error.to_string(),
        }
    }
}

/// Maps a raw SSE type to the kind the UI branches on.
///
/// v2.0.18 has no `session.execution.completed`, only the `*.failed` variants, and no
/// `session.idle` event at all: idleness is `time.idle` on the session object. So a turn
/// is "finished" only when the server says so, and Marvis must not infer it from silence.
pub fn agent_event_kind(raw_type: &str) -> AgentEventKind {
    match raw_type {
        "session.execution.started" | "session.step.started" => AgentEventKind::TurnStarted,
        "session.execution.completed" | "session.step.completed" | "session.idle" => {
            AgentEventKind::TurnFinished
        }
        "session.execution.failed" | "session.step.failed" => AgentEventKind::TurnFailed,
        "session.tool.called" => AgentEventKind::ToolCalled,
        "permission.asked" => AgentEventKind::PermissionAsked,
        _ => AgentEventKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{agent_event_kind, AgentEventKind};

    #[test]
    fn maps_the_observed_turn_lifecycle() {
        assert_eq!(
            agent_event_kind("session.execution.started"),
            AgentEventKind::TurnStarted
        );
        assert_eq!(
            agent_event_kind("session.step.started"),
            AgentEventKind::TurnStarted
        );
        assert_eq!(
            agent_event_kind("session.tool.called"),
            AgentEventKind::ToolCalled
        );
        assert_eq!(
            agent_event_kind("permission.asked"),
            AgentEventKind::PermissionAsked
        );
        assert_eq!(
            agent_event_kind("session.execution.failed"),
            AgentEventKind::TurnFailed
        );
        assert_eq!(
            agent_event_kind("session.step.failed"),
            AgentEventKind::TurnFailed
        );
    }

    #[test]
    fn ignores_types_marvis_does_not_model() {
        for raw in [
            "server.connected",
            "mcp.status.changed",
            "session.inbox.enqueued",
            "session.text.delta",
            "session.instructions.updated",
            "project.updated",
        ] {
            assert_eq!(agent_event_kind(raw), AgentEventKind::Unknown, "{raw}");
        }
    }
}
