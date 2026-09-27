//! A minimal OpenCode bridge: one `opencode serve` child per checkout.
//!
//! Everything Marvis needs from the agent is scoped to the checkout that owns it, so the
//! server is started with that directory as its working directory and every response is
//! checked against the expected path before it is trusted. A `ses_…` id from one checkout
//! is meaningless in another checkout's server, and `agent_session` is what enforces that.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    domain::{
        agent::{agent_event_kind, AgentSession},
        ipc::{IpcError, IpcErrorCode},
    },
    services::executable,
};

pub use crate::domain::agent::AgentEvent;

static NEXT_PORT_ATTEMPT: AtomicU64 = AtomicU64::new(0);

const SERVER_BOOT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(30);
const AGENT_PROGRAM: &str = "opencode";
/// The server is a loopback child, so basic auth with a per-child password is the
/// whole trust boundary. See `sec_09`.
const MAX_PROMPT_BYTES: usize = 512 * 1024;

/// `opencode` prints two lines on startup; the second is the only place the password exists.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerCredentials {
    port: u16,
    password: String,
    directory: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    data: T,
    #[serde(default)]
    location: Option<ApiLocation>,
}

#[derive(Debug, Deserialize)]
struct ApiLocation {
    directory: String,
}

#[derive(Debug, Deserialize)]
struct ApiSession {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    time: ApiSessionTime,
    /// Present inside the session object, unlike the envelope-level `location`.
    #[serde(default)]
    location: Option<ApiLocation>,
}

#[derive(Debug, Default, Deserialize)]
struct ApiSessionTime {
    #[serde(default)]
    created: i64,
    #[serde(default)]
    updated: i64,
    #[serde(default)]
    idle: Option<i64>,
}

impl ApiSession {
    /// Refuses a session that belongs to another directory than the checkout we asked about.
    fn check_scope(&self, expected: &Path) -> Result<(), BridgeError> {
        match &self.location {
            Some(location) if !same_directory(Path::new(&location.directory), expected) => Err(
                BridgeError::Foreign("the agent server answered for a different directory".into()),
            ),
            _ => Ok(()),
        }
    }
}

impl<T> ApiEnvelope<T> {
    /// Refuses a response that names a different directory than the checkout we asked about.
    fn into_scoped(self, expected: &Path) -> Result<T, BridgeError> {
        if let Some(location) = self.location {
            if !same_directory(Path::new(&location.directory), expected) {
                return Err(BridgeError::Foreign(
                    "the agent server answered for a different directory".into(),
                ));
            }
        }
        Ok(self.data)
    }
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[derive(Debug)]
pub enum BridgeError {
    Unavailable(String),
    Foreign(String),
    Failed(String),
}

impl BridgeError {
    fn into_ipc(self, fallback: IpcErrorCode) -> IpcError {
        let (code, message) = match self {
            BridgeError::Unavailable(message) => (IpcErrorCode::AgentUnavailable, message),
            BridgeError::Foreign(message) => (IpcErrorCode::AgentOwnershipMismatch, message),
            BridgeError::Failed(message) => (fallback, message),
        };
        IpcError::new(code, message)
    }
}

pub struct AgentBridge {
    checkout_id: String,
    child: Mutex<Option<Child>>,
    credentials: ServerCredentials,
    /// Set once the server is up, so the event reader can be told to stop.
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The reader loop reconnects forever, so it needs its own exit condition.
    stopped: std::sync::atomic::AtomicBool,
}

/// Where normalized events go. Injected so the service does not depend on Tauri.
pub type EventSink = Arc<dyn Fn(AgentEvent) + Send + Sync>;

impl AgentBridge {
    /// Starts a server for `directory` and waits until it prints its credentials.
    fn start(checkout_id: &str, directory: &Path) -> Result<Self, BridgeError> {
        let program = agent_program()
            .ok_or_else(|| BridgeError::Unavailable(OPENCODE_UNAVAILABLE.to_string()))?;
        let port = free_port()?;
        let mut child = Command::new(&program)
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--hostname",
                "127.0.0.1",
            ])
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                BridgeError::Unavailable(format!("could not start OpenCode: {error}"))
            })?;

        let credentials = match read_credentials(&mut child, directory, port) {
            Ok(credentials) => credentials,
            Err(error) => {
                let _ = child.kill();
                return Err(error);
            }
        };
        Ok(Self {
            checkout_id: checkout_id.to_string(),
            child: Mutex::new(Some(child)),
            credentials,
            reader: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.credentials.port)
    }

    fn auth_header(&self) -> String {
        format!("Basic {}", basic_credentials(&self.credentials.password))
    }

    /// Starts the event reader for this checkout's server, once.
    fn start_reader(self: &Arc<Self>, sink: &EventSink) {
        let mut slot = match self.reader.lock() {
            Ok(slot) => slot,
            Err(_) => return,
        };
        if slot.is_some() {
            return;
        }
        let bridge = Arc::clone(self);
        let sink = Arc::clone(sink);
        *slot = Some(std::thread::spawn(move || {
            read_events(bridge, sink);
        }));
    }

    fn directory(&self) -> &Path {
        &self.credentials.directory
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Fetches `path` and returns the `data` payload, refusing a foreign directory.
    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, BridgeError> {
        let envelope: ApiEnvelope<T> = send_json(
            ureq::get(&format!("{}{path}", self.base_url()))
                .header("authorization", self.auth_header())
                .header("accept", "application/json")
                .call(),
        )?;
        envelope.into_scoped(self.directory())
    }

    /// Posts to `path` and returns the `data` payload, refusing a foreign directory.
    fn post_json<B: serde::Serialize + ?Sized, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, BridgeError> {
        let envelope: ApiEnvelope<T> = send_json(
            ureq::post(&format!("{}{path}", self.base_url()))
                .header("authorization", self.auth_header())
                .send_json(body),
        )?;
        envelope.into_scoped(self.directory())
    }

    /// Resolves a session id inside this bridge only, then hands back the raw session.
    fn session(&self, session_id: &str) -> Result<ApiSession, BridgeError> {
        validate_session_id(session_id)?;
        let session: ApiSession = self.get_json(&format!("/api/session/{session_id}"))?;
        session.check_scope(self.directory())?;
        Ok(session)
    }

    /// Everything the session has been told, as one searchable blob.
    ///
    /// The route answers JSON, and the message shape differs between OpenCode versions, so
    /// the payload is re-serialized and searched rather than modelled: the only question
    /// asked is whether a given marker is present.
    fn session_transcript(&self, session_id: &str) -> Result<String, BridgeError> {
        validate_session_id(session_id)?;
        // Resolve first, so a foreign id is refused before anything is read.
        self.session(session_id)?;
        let messages: serde_json::Value =
            self.get_json(&format!("/api/session/{session_id}/message"))?;
        serde_json::to_string(&messages)
            .map_err(|error| BridgeError::Failed(format!("could not read the transcript: {error}")))
    }

    fn to_agent_session(&self, raw: &ApiSession) -> AgentSession {
        AgentSession {
            id: raw.id.clone(),
            checkout_id: self.checkout_id.clone(),
            title: if raw.title.is_empty() {
                "OpenCode session".to_string()
            } else {
                raw.title.clone()
            },
            idle_at: raw.time.idle,
            blocked_on_permission: false,
            created_at: raw.time.created,
            updated_at: raw.time.updated,
        }
    }

    fn stop(&self) {
        // The reader reconnects forever unless told to stop, so set the flag before
        // killing the server: otherwise joining it would never return.
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(mut child) = self.child.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.reader.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = handle.join();
        }
    }
}

/// A single event frame from the stream.
#[derive(Debug, Deserialize)]
struct SseFrame {
    #[serde(rename = "type")]
    raw_type: String,
    #[serde(default)]
    location: Option<ApiLocation>,
    #[serde(default)]
    data: serde_json::Value,
}

/// Consumes `/api/event` until the server dies, normalizing and forwarding each frame.
///
/// Reconnects with a bounded backoff: a dropped stream is normal (laptop sleep, server
/// restart) and a fresh stream replays no history, so the caller must re-read state after
/// a reconnect rather than assume nothing was missed.
fn read_events(bridge: Arc<AgentBridge>, sink: EventSink) {
    let mut backoff = Duration::from_millis(250);
    while !bridge.is_stopped() {
        let stream = stream_client();
        let response = stream
            .get(&format!("{}/api/event", bridge.base_url()))
            .header("authorization", bridge.auth_header())
            .header("accept", "text/event-stream")
            .call();
        let mut reader = BufReader::new(match response {
            Ok(response) => response.into_body().into_reader(),
            Err(_) => {
                if !sleep_unless_stopped(&bridge, backoff) {
                    return;
                }
                backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
                continue;
            }
        });
        backoff = Duration::from_millis(250);
        let mut line = String::new();
        while !bridge.is_stopped() {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let Some(payload) = line.trim().strip_prefix("data:") else {
                // `: heartbeat` comments and event/id/field lines carry no payload here.
                continue;
            };
            let Ok(frame) = serde_json::from_str::<SseFrame>(payload.trim()) else {
                continue;
            };
            // A frame for another directory is not ours, even from our own server.
            if let Some(location) = &frame.location {
                if !same_directory(Path::new(&location.directory), bridge.directory()) {
                    continue;
                }
            }
            let data = frame.data;
            sink(AgentEvent {
                checkout_id: bridge.checkout_id.clone(),
                session_id: data
                    .get("sessionID")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                kind: agent_event_kind(&frame.raw_type),
                raw_type: frame.raw_type.clone(),
                data,
            });
        }
        if !sleep_unless_stopped(&bridge, backoff) {
            return;
        }
        backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
    }
}

/// Sleeps in small slices so a stop request is honoured promptly. Returns false if the
/// bridge was stopped while waiting.
fn sleep_unless_stopped(bridge: &AgentBridge, total: Duration) -> bool {
    let slice = Duration::from_millis(50);
    let mut waited = Duration::ZERO;
    while waited < total {
        if bridge.is_stopped() {
            return false;
        }
        sleep(slice.min(total - waited));
        waited += slice;
    }
    !bridge.is_stopped()
}

/// A client with no global timeout, because an event stream is expected to stay idle.
fn stream_client() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(None)
            .timeout_connect(Some(Duration::from_secs(10)))
            .build(),
    )
}

impl Drop for AgentBridge {
    fn drop(&mut self) {
        self.stop();
    }
}

const OPENCODE_UNAVAILABLE: &str =
    "OpenCode is unavailable. Install it and make its command available on PATH.";

/// An OpenCode session id is `ses_` plus an opaque suffix. Rejecting anything else keeps a
/// WebView-supplied value from reaching a path we would otherwise interpolate.
fn validate_session_id(session_id: &str) -> Result<(), BridgeError> {
    let valid = session_id.len() > 4
        && session_id.starts_with("ses_")
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(BridgeError::Foreign(
            "that is not a valid OpenCode session id".into(),
        ))
    }
}

fn basic_credentials(password: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(format!("opencode:{password}"))
}

fn agent_program() -> Option<PathBuf> {
    executable::find_executable(AGENT_PROGRAM)
}

/// Reserves a port by binding it, then releases it for the child. A collision is retried a
/// bounded number of times; giving up is better than serving on someone else's port.
fn free_port() -> Result<u16, BridgeError> {
    for attempt in 0..8u32 {
        let offset = NEXT_PORT_ATTEMPT.fetch_add(1, Ordering::Relaxed) as u16;
        let candidate = 46000u16.wrapping_add(offset).wrapping_add(attempt as u16);
        if TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return Ok(candidate);
        }
    }
    Err(BridgeError::Unavailable(
        "could not find a free port for the agent server".into(),
    ))
}

/// Reads the password the child prints. The port is the one we asked for, and
/// `finish_credentials` proves the child actually bound it.
fn read_credentials(
    child: &mut Child,
    directory: &Path,
    port: u16,
) -> Result<ServerCredentials, BridgeError> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Unavailable("the agent server produced no output".into()))?;
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + SERVER_BOOT_TIMEOUT;
    while Instant::now() < deadline {
        let line = match lines.next() {
            Some(Ok(line)) => line,
            Some(Err(error)) => {
                return Err(BridgeError::Unavailable(format!(
                    "could not read the agent server output: {error}"
                )))
            }
            None => break,
        };
        if let Some(password) = line.strip_prefix("server password ") {
            return finish_credentials(ServerCredentials {
                port,
                password: password.trim().to_string(),
                directory: directory.to_path_buf(),
            });
        }
    }
    Err(BridgeError::Unavailable(
        "the agent server did not report a password in time".into(),
    ))
}

/// Confirms the server is really up on the port we asked for, and really scoped to this
/// checkout, before the bridge is handed out.
fn finish_credentials(credentials: ServerCredentials) -> Result<ServerCredentials, BridgeError> {
    let probe = AgentBridge {
        checkout_id: String::new(),
        child: Mutex::new(None),
        credentials: credentials.clone(),
        reader: Mutex::new(None),
        stopped: std::sync::atomic::AtomicBool::new(false),
    };
    // /api/agent is cheap and confirms both reachability and directory scoping.
    let _: Vec<serde_json::Value> = probe.get_json("/api/agent")?;
    Ok(credentials)
}

/// Parses the `{data: …}` envelope every route replies with.
fn send_json<T: serde::de::DeserializeOwned>(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<ApiEnvelope<T>, BridgeError> {
    match response {
        Ok(mut response) => {
            let raw = response.body_mut().read_to_vec().map_err(|error| {
                BridgeError::Failed(format!("could not read the reply: {error}"))
            })?;
            // The body is deliberately not echoed: it can carry session content.
            serde_json::from_slice(&raw).map_err(|error| {
                BridgeError::Failed(format!("unexpected agent server reply: {error}"))
            })
        }
        // 4xx bodies carry the server's own message; keep it, it is actionable.
        Err(ureq::Error::StatusCode(status)) => Err(BridgeError::Failed(status_detail(status))),
        Err(error) => Err(BridgeError::Unavailable(format!(
            "the agent server is not reachable: {error}"
        ))),
    }
}

fn status_detail(status: u16) -> String {
    match status {
        404 => "the agent server does not support that request in this version".to_string(),
        401 | 403 => "the agent server rejected Marvis's credentials".to_string(),
        other => format!("the agent server answered {other}"),
    }
}

/// Owns one server per checkout for the app's lifetime.
#[derive(Default)]
pub struct AgentService {
    bridges: Mutex<HashMap<String, Arc<AgentBridge>>>,
    sink: Mutex<Option<EventSink>>,
}

impl AgentService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets where normalized events are delivered. Without one the bridges still work,
    /// they just do not report live turn state.
    pub fn set_event_sink(&self, sink: EventSink) {
        if let Ok(mut slot) = self.sink.lock() {
            *slot = Some(sink);
        }
    }

    /// Returns the checkout's bridge, starting its server on first use.
    pub fn bridge(
        &self,
        checkout_id: &str,
        directory: &Path,
    ) -> Result<Arc<AgentBridge>, BridgeError> {
        let bridges = self
            .bridges
            .lock()
            .map_err(|_| BridgeError::Failed("the agent service is poisoned".into()))?;
        if let Some(existing) = bridges.get(checkout_id) {
            return Ok(Arc::clone(existing));
        }
        let sink = self
            .sink
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(Arc::clone));
        // The lock is released while the child boots: startup can take seconds, and
        // holding it would serialize every other checkout's bridge behind it.
        drop(bridges);
        let started = Arc::new(AgentBridge::start(checkout_id, directory)?);
        if let Some(sink) = sink {
            started.start_reader(&sink);
        }
        let mut bridges = self
            .bridges
            .lock()
            .map_err(|_| BridgeError::Failed("the agent service is poisoned".into()))?;
        // A concurrent call may have won the race; keep the winner and drop ours.
        if let Some(existing) = bridges.get(checkout_id) {
            return Ok(Arc::clone(existing));
        }
        bridges.insert(checkout_id.to_string(), Arc::clone(&started));
        Ok(started)
    }

    /// Every session the checkout's server knows about.
    pub fn sessions(
        &self,
        checkout_id: &str,
        directory: &Path,
    ) -> Result<Vec<AgentSession>, BridgeError> {
        let bridge = self.bridge(checkout_id, directory)?;
        let listed: Vec<ApiSession> = bridge.get_json("/api/session")?;
        let mut sessions = Vec::new();
        for raw in listed {
            // The list spans every directory the server knows, so foreign sessions are
            // filtered out here. Addressing one by id is what rejects them, below.
            if raw.check_scope(directory).is_ok() {
                sessions.push(bridge.to_agent_session(&raw));
            }
        }
        Ok(sessions)
    }

    /// Resolves a session inside `checkout_id` only. A session belonging to another
    /// checkout cannot be addressed here: it does not exist in this server.
    pub fn owned_session(
        &self,
        checkout_id: &str,
        directory: &Path,
        session_id: &str,
    ) -> Result<AgentSession, BridgeError> {
        let bridge = self.bridge(checkout_id, directory)?;
        Ok(bridge.to_agent_session(&bridge.session(session_id)?))
    }

    pub fn create_session(
        &self,
        checkout_id: &str,
        directory: &Path,
        title: &str,
    ) -> Result<AgentSession, BridgeError> {
        let bridge = self.bridge(checkout_id, directory)?;
        let created: ApiSession =
            bridge.post_json("/api/session", &serde_json::json!({ "title": title }))?;
        created.check_scope(directory)?;
        Ok(bridge.to_agent_session(&created))
    }

    /// Sends the review as one message. v2.0.18 wants `{"text": …}` on this route.
    pub fn prompt(
        &self,
        checkout_id: &str,
        directory: &Path,
        session_id: &str,
        text: &str,
    ) -> Result<AgentSession, BridgeError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(BridgeError::Failed("the review is empty".into()));
        }
        if text.len() > MAX_PROMPT_BYTES {
            return Err(BridgeError::Failed(
                "the review is too large to send to the agent in one message".into(),
            ));
        }
        let bridge = self.bridge(checkout_id, directory)?;
        // Resolve first so a foreign id fails before anything is sent.
        bridge.session(session_id)?;
        let _: serde_json::Value = bridge.post_json(
            &format!("/api/session/{session_id}/prompt"),
            &serde_json::json!({ "text": text }),
        )?;
        Ok(bridge.to_agent_session(&bridge.session(session_id)?))
    }

    /// Whether the session's transcript mentions `marker`.
    ///
    /// This is what makes a reconnect safe: a round that was recorded as `dispatching` but
    /// never confirmed is only retried when its marker is genuinely absent.
    pub fn session_mentions(
        &self,
        checkout_id: &str,
        directory: &Path,
        session_id: &str,
        marker: &str,
    ) -> Result<bool, BridgeError> {
        if marker.is_empty() {
            return Ok(false);
        }
        let bridge = self.bridge(checkout_id, directory)?;
        Ok(bridge.session_transcript(session_id)?.contains(marker))
    }

    /// Stops the checkout's server. Called when the app shuts down.
    pub fn stop(&self, checkout_id: &str) {
        if let Ok(mut bridges) = self.bridges.lock() {
            if let Some(bridge) = bridges.remove(checkout_id) {
                bridge.stop();
            }
        }
    }

    pub fn stop_all(&self) {
        if let Ok(mut bridges) = self.bridges.lock() {
            for (_, bridge) in bridges.drain() {
                bridge.stop();
            }
        }
    }
}

impl Drop for AgentService {
    fn drop(&mut self) {
        self.stop_all();
    }
}

pub fn map_error(error: BridgeError) -> IpcError {
    error.into_ipc(IpcErrorCode::OperationFailed)
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        thread::sleep,
        time::{Duration, Instant},
    };

    use crate::{
        domain::review::{review_anchor_hash, review_round_marker},
        services::review_round::build_round_prompt,
    };

    use super::{
        agent_program, basic_credentials, same_directory, validate_session_id, AgentEvent,
        AgentService, BridgeError, MAX_PROMPT_BYTES,
    };

    #[test]
    fn resolves_the_opencode_binary_without_a_shell() {
        // None is acceptable on a machine without OpenCode; a shell string is not.
        if let Some(program) = agent_program() {
            assert!(!program.to_string_lossy().contains(" -"), "{program:?}");
        }
    }

    #[test]
    fn builds_basic_credentials_for_the_server_password() {
        assert_eq!(basic_credentials("abc"), "b3BlbmNvZGU6YWJj");
    }

    #[test]
    fn sec_09_session_ids_from_another_checkout_cannot_be_addressed() {
        // A well-formed id from checkout A is still only meaningful in A's server; the
        // value itself must never be usable as a path fragment.
        for hostile in [
            "ses_../../etc/passwd",
            "ses_a/b",
            "ses_ ",
            "ses_",
            "not-a-session",
            "",
            "ses_a?directory=/other",
            "ses_a#",
        ] {
            assert!(
                validate_session_id(hostile).is_err(),
                "accepted a hostile session id: {hostile:?}"
            );
        }
        assert!(validate_session_id("ses_f21a52baaffeAS3HejslJJxVor").is_ok());
    }

    #[test]
    fn compares_directories_after_resolving_symlinks() {
        let directory = std::env::temp_dir();
        assert!(same_directory(&directory, &directory));
        assert!(!same_directory(
            &directory,
            Path::new("/definitely/not/here")
        ));
    }

    #[test]
    fn keeps_prompt_limits_and_reports_unavailable_agents_distinctly() {
        // A review larger than this is rejected instead of being split: the contract is
        // that one round is one message.
        assert_eq!(MAX_PROMPT_BYTES, 512 * 1024);
        let unavailable = BridgeError::Unavailable("no binary".into())
            .into_ipc(crate::domain::ipc::IpcErrorCode::OperationFailed);
        assert_eq!(
            unavailable.code,
            crate::domain::ipc::IpcErrorCode::AgentUnavailable
        );
        let foreign = BridgeError::Foreign("other checkout".into())
            .into_ipc(crate::domain::ipc::IpcErrorCode::OperationFailed);
        assert_eq!(
            foreign.code,
            crate::domain::ipc::IpcErrorCode::AgentOwnershipMismatch
        );
    }

    /// Proves the round marker really reaches the session, which is what makes the
    /// reconnect path safe. Gated so it never runs in CI.
    ///
    /// `MARVIS_AGENT_BRIDGE=1 cargo test a_round_marker_reaches_the_real_session -- --nocapture`
    #[test]
    fn a_round_marker_reaches_the_real_session() {
        if std::env::var("MARVIS_AGENT_BRIDGE").as_deref() != Ok("1") {
            eprintln!("skipped: set MARVIS_AGENT_BRIDGE=1 to exercise a real agent server");
            return;
        }
        let directory = PathBuf::from(
            std::env::var("MARVIS_AGENT_BRIDGE_DIR").expect("MARVIS_AGENT_BRIDGE_DIR"),
        );
        let agents = AgentService::new();
        let created = agents
            .create_session("checkout:marker", &directory, "marker probe")
            .expect("create a session on the real server");
        let marker = review_round_marker("round:marker-probe");

        // Before the send, the marker is genuinely absent: an absent marker after a crash
        // is the signal that the message never arrived, so this must not be a false match.
        assert!(
            !agents
                .session_mentions("checkout:marker", &directory, &created.id, &marker)
                .expect("read the transcript"),
            "the marker was found before anything was sent"
        );

        agents
            .prompt(
                "checkout:marker",
                &directory,
                &created.id,
                &build_round_prompt("# Code Review\n", &marker),
            )
            .expect("send the round prompt");

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if agents
                .session_mentions("checkout:marker", &directory, &created.id, &marker)
                .expect("read the transcript")
            {
                break;
            }
            assert!(
                Instant::now() <= deadline,
                "the marker never reached the session transcript"
            );
            sleep(Duration::from_millis(500));
        }
        agents.stop("checkout:marker");
    }

    /// Proves what the server can and cannot report about a turn.
    ///
    /// v2.0.18 has no turn-completed event, and `idle` is absent both for a session that
    /// never ran and for one that is working. Only the client, which saw the turn start, can
    /// tell those apart, so the wire contract is `idle_at` and this test pins the ambiguity.
    ///
    /// `MARVIS_AGENT_BRIDGE=1 cargo test a_turn_is_observable_through_the_idle_time -- --nocapture`
    #[test]
    fn a_turn_is_observable_through_the_idle_time() {
        if std::env::var("MARVIS_AGENT_BRIDGE").as_deref() != Ok("1") {
            eprintln!("skipped: set MARVIS_AGENT_BRIDGE=1 to exercise a real agent server");
            return;
        }
        let directory = PathBuf::from(
            std::env::var("MARVIS_AGENT_BRIDGE_DIR").expect("MARVIS_AGENT_BRIDGE_DIR"),
        );
        let agents = AgentService::new();
        let created = agents
            .create_session("checkout:turn", &directory, "turn probe")
            .expect("create a session on the real server");

        let idle_at = |id: &str| {
            agents
                .owned_session("checkout:turn", &directory, id)
                .map(|session| session.idle_at)
        };
        // A session that never ran reports no idle time. This is why `idle_at` cannot be
        // turned into a `busy` flag on the server's side.
        assert_eq!(
            idle_at(&created.id).ok().flatten(),
            None,
            "a fresh session has no idle time"
        );

        agents
            .prompt(
                "checkout:turn",
                &directory,
                &created.id,
                "Reply with the single word OK and do nothing else.",
            )
            .expect("send a prompt");

        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if idle_at(&created.id).ok().flatten().is_some() {
                break;
            }
            assert!(
                Instant::now() <= deadline,
                "the session never reported an idle time"
            );
            sleep(Duration::from_millis(400));
        }
        agents.stop("checkout:turn");
    }

    /// Talks to a real `opencode serve`. Gated so it never runs in CI, because it needs
    /// the OpenCode binary, a working provider and the user's machine.
    ///
    /// `MARVIS_AGENT_BRIDGE=1 cargo test bridge_talks_to_a_real_server -- --nocapture`
    #[test]
    fn bridge_talks_to_a_real_server() {
        if std::env::var("MARVIS_AGENT_BRIDGE").as_deref() != Ok("1") {
            eprintln!("skipped: set MARVIS_AGENT_BRIDGE=1 to exercise a real agent server");
            return;
        }
        let first = std::env::var("MARVIS_AGENT_BRIDGE_DIR").expect("MARVIS_AGENT_BRIDGE_DIR");
        let second =
            std::env::var("MARVIS_AGENT_BRIDGE_OTHER_DIR").expect("MARVIS_AGENT_BRIDGE_OTHER_DIR");
        let agents = AgentService::new();
        let first = PathBuf::from(first);
        let second = PathBuf::from(second);

        // Collect normalized events so the SSE reader is exercised, not just the requests.
        let seen: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        agents.set_event_sink(Arc::new(move |event: AgentEvent| {
            if let Ok(mut seen) = sink.lock() {
                seen.push(event);
            }
        }));

        let created = agents
            .create_session("checkout:first", &first, "bridge probe")
            .expect("create a session on the real server");
        assert!(created.id.starts_with("ses_"), "{}", created.id);

        // The same id in another checkout's server must not resolve.
        let leaked = agents.owned_session("checkout:second", &second, &created.id);
        assert!(
            leaked.is_err(),
            "a session leaked across checkouts: {leaked:?}"
        );

        let sent = agents
            .prompt(
                "checkout:first",
                &first,
                &created.id,
                "Reply with the single word OK and do nothing else.",
            )
            .expect("send a prompt to the real server");
        assert_eq!(sent.id, created.id);

        let sessions = agents
            .sessions("checkout:first", &first)
            .expect("list sessions");
        assert!(
            sessions.iter().any(|session| session.id == created.id),
            "the session vanished: {sessions:?}"
        );

        // The sink is app-wide, so both checkouts' events arrive tagged. What must hold is
        // that this session's turn is reported as ours, and never as the other checkout's.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let snapshot = seen.lock().map(|events| events.clone()).unwrap_or_default();
            assert!(
                snapshot
                    .iter()
                    .filter(|event| event.session_id.as_deref() == Some(created.id.as_str()))
                    .all(|event| event.checkout_id == "checkout:first"),
                "an event for this session was attributed to another checkout: {snapshot:?}"
            );
            if snapshot.iter().any(|event| {
                event.kind == crate::domain::agent::AgentEventKind::TurnStarted
                    && event.session_id.as_deref() == Some(created.id.as_str())
                    && event.checkout_id == "checkout:first"
            }) {
                break;
            }
            assert!(
                Instant::now() <= deadline,
                "no turnStarted event arrived: {snapshot:?}"
            );
            sleep(Duration::from_millis(250));
        }

        agents.stop("checkout:first");
        agents.stop("checkout:second");
    }

    /// D2-05, D2-09, D2-10 and D2-15 in one pass, against two real servers running in two
    /// different checkouts — the shape the acceptance runbook asks for.
    ///
    /// What is proven here is the loop itself: one message carrying the whole review, a turn
    /// that ends, a line the agent actually rewrote, and a second checkout that stays out of
    /// the way. What is not proven here is the presentation layer, which is judged from the
    /// diff by the frontend and cannot be seen from a test.
    ///
    /// `MARVIS_AGENT_BRIDGE=1 cargo test two_checkouts_run_the_review_loop -- --nocapture`
    #[test]
    fn two_checkouts_run_the_review_loop() {
        if std::env::var("MARVIS_AGENT_BRIDGE").as_deref() != Ok("1") {
            eprintln!("skipped: set MARVIS_AGENT_BRIDGE=1 to exercise a real agent server");
            return;
        }
        let first = PathBuf::from(
            std::env::var("MARVIS_AGENT_BRIDGE_DIR").expect("MARVIS_AGENT_BRIDGE_DIR"),
        );
        let second = PathBuf::from(
            std::env::var("MARVIS_AGENT_BRIDGE_OTHER_DIR").expect("MARVIS_AGENT_BRIDGE_OTHER_DIR"),
        );
        let target = first.join("marvis_loop_target.txt");
        let second_file = first.join("marvis_second_target.txt");
        let original = "alpha\nbravo\n";
        std::fs::write(&target, original).expect("write the file under review");
        std::fs::write(&second_file, "one\ntwo\n").expect("write the second file under review");

        let agents = AgentService::new();
        let mine = agents
            .create_session("checkout:loop", &first, "loop")
            .expect("session in the first checkout");
        let other = agents
            .create_session("checkout:loop-other", &second, "other loop")
            .expect("session in the second checkout");

        // Two agents means two servers: an id from one checkout is meaningless in the other.
        assert!(
            agents
                .owned_session("checkout:loop-other", &second, &mine.id)
                .is_err(),
            "a session id resolved in another checkout's server"
        );

        // D2-05: both files' drafts leave as one message, marker included. Each instruction
        // is unambiguous on purpose: a review the model has to guess at makes it stop and
        // ask, and an unanswered question is not something this server version can settle.
        let marker = review_round_marker("round:loop");
        let review = [
            "### `marvis_loop_target.txt`".to_string(),
            String::new(),
            "- line 1: replace the whole file with exactly: `done by agent`.".to_string(),
            String::new(),
            "### `marvis_second_target.txt`".to_string(),
            String::new(),
            "- line 1: replace the whole file with exactly: `also done by agent`.".to_string(),
        ]
        .join("\n");
        let prompt = build_round_prompt(&review, &marker);
        assert!(prompt.contains(&marker), "the marker is not in the prompt");
        assert!(
            prompt.contains("marvis_loop_target.txt")
                && prompt.contains("marvis_second_target.txt"),
            "the two files did not leave in the same message"
        );
        agents
            .prompt("checkout:loop", &first, &mine.id, &prompt)
            .expect("send the round");

        // D2-09 needs the turn to be over before anything can be judged, and v2.0.18 reports
        // that only as an idle time on the session.
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let session = agents.owned_session("checkout:loop", &first, &mine.id);
            let finished = matches!(&session, Ok(session) if session.idle_at.is_some());
            let seen = match &session {
                Ok(session) => format!("still running, idle_at={:?}", session.idle_at),
                Err(error) => format!("session read failed: {error:?}"),
            };
            if finished {
                break;
            }
            assert!(
                Instant::now() <= deadline,
                "the review turn never came back to idle ({seen})"
            );
            sleep(Duration::from_millis(500));
        }

        // D2-05, second half: the message that carried the review is really in the
        // transcript, so a reconnect would recognize this exact send.
        assert!(
            agents
                .session_mentions("checkout:loop", &first, &mine.id, &marker)
                .expect("read the transcript"),
            "the round marker never landed in the session"
        );

        // D2-15: the agent kept working, so the working tree moved — on both files, which is
        // what makes this a batch rather than a single comment that got through.
        let changed = std::fs::read_to_string(&target).expect("read the file again");
        assert_ne!(changed, original, "the agent did not touch the first file");
        assert!(
            std::fs::read_to_string(&second_file)
                .expect("read the second file")
                .contains("also done by agent"),
            "the second file's comment did not reach the agent's work"
        );

        // D2-10: a note anchored on the old first line has drifted, and that is exactly what
        // marks it outdated — the hash is what decides, not who edited the line.
        assert_ne!(
            review_anchor_hash(original),
            review_anchor_hash(&changed),
            "the rewritten line still hashes the same"
        );

        // The second agent was never dragged into this: its session exists, has no turn, and
        // does not know about the first checkout's session.
        let untouched = agents
            .sessions("checkout:loop-other", &second)
            .expect("list the other checkout's sessions");
        assert!(
            untouched.iter().any(|session| session.id == other.id),
            "the second agent's session vanished"
        );
        assert!(
            untouched.iter().all(|session| session.id != mine.id),
            "the first checkout's session leaked into the second"
        );
        assert!(
            untouched
                .iter()
                .find(|session| session.id == other.id)
                .and_then(|session| session.idle_at)
                .is_none(),
            "the second agent reports a turn it never ran"
        );

        agents.stop("checkout:loop");
        agents.stop("checkout:loop-other");
    }
}
