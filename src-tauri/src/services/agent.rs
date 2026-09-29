//! A minimal OpenCode bridge: one `opencode serve` child per checkout.
//!
//! Everything Marvis needs from the agent is scoped to the checkout that owns it, so the
//! server is started with that directory as its working directory and every response is
//! checked against the expected path before it is trusted. A `ses_…` id from one checkout
//! is meaningless in another checkout's server, and `agent_session` is what enforces that.

use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    domain::{
        agent::{agent_event_kind, AgentAgent, AgentSession},
        ipc::{IpcError, IpcErrorCode},
    },
    services::executable,
};

pub use crate::domain::agent::AgentEvent;

static NEXT_PORT_ATTEMPT: AtomicU64 = AtomicU64::new(0);

const SERVER_BOOT_TIMEOUT: Duration = Duration::from_secs(30);
const JSON_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CHILD_REAP_POLL: Duration = Duration::from_millis(10);
// `stopAgent` is awaited by the UI.
const STARTUP_STOP_WAIT: Duration = Duration::from_secs(3);
// A waiter has to cover the launcher's wall time: its 30s shared budget plus the 3s child reap
// and 3s reader cleanup, which is the one path that spends both. It is only an inert caller-wait
// bound: expiry does not cancel a healthy startup, and bounded retries share that same 30s launch
// budget.
const STARTUP_WAIT_TIMEOUT: Duration = Duration::from_secs(36);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(30);
const AGENT_PROGRAM: &str = "opencode";
const EARLY_EOF_MESSAGE: &str = "the agent server reached EOF before reporting credentials";
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
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    model: Option<ApiModel>,
}

#[derive(Debug, Deserialize)]
struct ApiModel {
    #[serde(default)]
    provider_id: String,
    id: String,
    #[serde(default)]
    variant: Option<String>,
}

impl ApiModel {
    /// The model as one string, in the `provider/id#variant` form OpenCode writes in config.
    fn label(&self) -> String {
        let base = if self.provider_id.is_empty() {
            self.id.clone()
        } else {
            format!("{}/{}", self.provider_id, self.id)
        };
        match &self.variant {
            Some(variant) => format!("{base}#{variant}"),
            None => base,
        }
    }
}

/// One entry of `GET /api/agent`, which is where the color an agent is painted with lives.
#[derive(Debug, Deserialize)]
struct ApiAgent {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    hidden: bool,
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

#[derive(Debug, Clone)]
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
    /// Drains stdout after credentials arrive, so later child output cannot fill the pipe.
    output_reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// A clone of the event socket, used to interrupt a blocking read during stop.
    event_socket: Mutex<Option<TcpStream>>,
    /// Set once the server is up, so the event reader can be told to stop.
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The reader loop reconnects forever, so it needs its own exit condition.
    stopped: std::sync::atomic::AtomicBool,
}

/// Where normalized events go. Injected so the service does not depend on Tauri.
pub type EventSink = Arc<dyn Fn(AgentEvent) + Send + Sync>;

#[derive(Clone)]
struct StartupChild(Arc<Mutex<Option<Child>>>);

impl StartupChild {
    fn new(child: Child) -> Self {
        Self(Arc::new(Mutex::new(Some(child))))
    }

    fn with_mut<R>(&self, operation: impl FnOnce(&mut Child) -> R) -> R {
        let mut child = self
            .0
            .lock()
            .expect("the startup child lock should not be poisoned");
        operation(
            child
                .as_mut()
                .expect("the startup child should still be owned"),
        )
    }

    fn take(&self) -> Option<Child> {
        self.0
            .lock()
            .expect("the startup child lock should not be poisoned")
            .take()
    }
}

struct ChildGuard(Option<StartupChild>);

/// Startup port hooks: the retry predicate is only consulted after the failed child is
/// terminated, and a retry still requires the exact `EARLY_EOF_MESSAGE` result. `track_child`
/// is only a test seam for retaining a cleanup handle to the same startup child.
struct PortHooks<FindPort, IsAvailable, TrackChild> {
    find_port: FindPort,
    is_available: IsAvailable,
    track_child: TrackChild,
}

impl ChildGuard {
    fn new(child: impl Into<StartupChild>) -> Self {
        Self(Some(child.into()))
    }

    fn with_mut<R>(&self, operation: impl FnOnce(&mut Child) -> R) -> R {
        self.0
            .as_ref()
            .expect("the startup child should still be owned")
            .with_mut(operation)
    }

    fn take(&mut self) -> Option<StartupChild> {
        self.0.take()
    }
}

impl From<Child> for StartupChild {
    fn from(child: Child) -> Self {
        Self::new(child)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.take().and_then(|child| child.take()) {
            terminate_child(child, Instant::now() + STARTUP_STOP_WAIT);
        }
    }
}

impl AgentBridge {
    /// Starts a server for `directory` and waits until it prints its credentials.
    fn start(checkout_id: &str, directory: &Path) -> Result<Self, BridgeError> {
        Self::start_with_timeout(checkout_id, directory, SERVER_BOOT_TIMEOUT)
    }

    fn start_with_timeout(
        checkout_id: &str,
        directory: &Path,
        startup_timeout: Duration,
    ) -> Result<Self, BridgeError> {
        let deadline = Instant::now() + startup_timeout;
        let program = agent_program()
            .ok_or_else(|| BridgeError::Unavailable(OPENCODE_UNAVAILABLE.to_string()))?;
        Self::start_with_deadline(
            checkout_id,
            directory,
            deadline,
            PortHooks {
                find_port: free_port,
                is_available: is_port_available,
                track_child: no_startup_child_tracking,
            },
            |port| {
                Command::new(&program)
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
                    })
            },
            read_credentials,
            finish_credentials,
        )
    }

    /// Runs startup through injected seams so collision handling can be tested without real
    /// ports. Only an exact early-EOF read error plus a post-termination unavailable port retries.
    fn start_with_deadline<FindPort, IsAvailable, TrackChild, Launch, Read, Finish>(
        checkout_id: &str,
        directory: &Path,
        deadline: Instant,
        mut ports: PortHooks<FindPort, IsAvailable, TrackChild>,
        mut launch: Launch,
        mut read: Read,
        mut finish: Finish,
    ) -> Result<Self, BridgeError>
    where
        FindPort: FnMut() -> Result<u16, BridgeError>,
        IsAvailable: FnMut(u16) -> bool,
        TrackChild: FnMut(&StartupChild),
        Launch: FnMut(u16) -> Result<Child, BridgeError>,
        Read: FnMut(
            &mut Child,
            &Path,
            u16,
            Instant,
        )
            -> Result<(ServerCredentials, std::thread::JoinHandle<()>), BridgeError>,
        Finish: FnMut(ServerCredentials, Instant) -> Result<ServerCredentials, BridgeError>,
    {
        for attempt in 0..MAX_START_ATTEMPTS {
            // A retry shares the original deadline; do not spend a fresh startup budget on it.
            if attempt > 0 && Instant::now() >= deadline {
                return Err(startup_timeout_error());
            }
            let port = (ports.find_port)()?;
            let child = StartupChild::new(launch(port)?);
            (ports.track_child)(&child);
            let mut child = ChildGuard::new(child);

            let read_result = child.with_mut(|child| read(child, directory, port, deadline));
            let (credentials, output_reader) = match read_result {
                Ok(credentials) => credentials,
                Err(error) => {
                    let early_eof = is_early_eof(&error);
                    let failed_child = child
                        .take()
                        .expect("the startup child should still be owned");
                    terminate_child(
                        failed_child
                            .take()
                            .expect("the startup child should still be owned"),
                        Instant::now() + STARTUP_STOP_WAIT,
                    );
                    // The availability probe is intentionally repeated after EOF: the initial
                    // bind is only a check, so another process may win the bind-to-spawn race.
                    if early_eof && !(ports.is_available)(port) {
                        if Instant::now() >= deadline {
                            return Err(startup_timeout_error());
                        }
                        if attempt + 1 == MAX_START_ATTEMPTS {
                            return Err(BridgeError::Unavailable(format!(
                                "another process took the selected agent port; exhausted {MAX_START_ATTEMPTS} startup attempts"
                            )));
                        }
                        continue;
                    }
                    return Err(error);
                }
            };
            if Instant::now() >= deadline {
                terminate_child(
                    child
                        .take()
                        .expect("the startup child should still be owned")
                        .take()
                        .expect("the startup child should still be owned"),
                    Instant::now() + STARTUP_STOP_WAIT,
                );
                join_reader(output_reader, Instant::now() + STARTUP_STOP_WAIT);
                return Err(startup_timeout_error());
            }
            if let Err(error) = finish(credentials.clone(), deadline) {
                terminate_child(
                    child
                        .take()
                        .expect("the startup child should still be owned")
                        .take()
                        .expect("the startup child should still be owned"),
                    Instant::now() + STARTUP_STOP_WAIT,
                );
                join_reader(output_reader, Instant::now() + STARTUP_STOP_WAIT);
                return Err(error);
            }
            let child = child
                .take()
                .expect("the startup child should still be owned")
                .take()
                .expect("the startup child should still be owned");
            return Ok(Self {
                checkout_id: checkout_id.to_string(),
                child: Mutex::new(Some(child)),
                credentials,
                output_reader: Mutex::new(Some(output_reader)),
                event_socket: Mutex::new(None),
                reader: Mutex::new(None),
                stopped: std::sync::atomic::AtomicBool::new(false),
            });
        }
        Err(BridgeError::Unavailable(
            "could not start the agent server after exhausting startup attempts".into(),
        ))
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

    fn install_event_socket(&self, socket: TcpStream) -> bool {
        let Ok(mut slot) = self.event_socket.lock() else {
            let _ = socket.shutdown(Shutdown::Both);
            return false;
        };
        *slot = Some(socket);
        if self.is_stopped() {
            if let Some(socket) = slot.take() {
                let _ = socket.shutdown(Shutdown::Both);
            }
            false
        } else {
            true
        }
    }

    fn clear_event_socket(&self) {
        if let Ok(mut slot) = self.event_socket.lock() {
            if let Some(socket) = slot.take() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }

    /// Fetches `path` and returns the `data` payload, refusing a foreign directory.
    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, BridgeError> {
        self.get_json_with_timeout(path, JSON_REQUEST_TIMEOUT)
    }

    fn get_json_with_timeout<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        timeout: Duration,
    ) -> Result<T, BridgeError> {
        let envelope: ApiEnvelope<T> = send_json(
            json_client(timeout)
                .get(&format!("{}{path}", self.base_url()))
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
            json_client(JSON_REQUEST_TIMEOUT)
                .post(&format!("{}{path}", self.base_url()))
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
            agent: raw.agent.clone(),
            model: raw.model.as_ref().map(ApiModel::label),
            parent_id: raw.parent_id.clone(),
            outcome: raw.outcome.clone(),
            created_at: raw.time.created,
            updated_at: raw.time.updated,
        }
    }

    fn stop(&self) {
        self.stop_with_timeout(STARTUP_STOP_WAIT);
    }

    fn stop_with_timeout(&self, timeout: Duration) {
        // The reader reconnects forever unless told to stop, so set the flag before
        // killing the server: otherwise joining it would never return.
        self.signal_stop();
        let deadline = Instant::now() + timeout;
        if let Some(socket) = self
            .event_socket
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
        {
            let _ = socket.shutdown(Shutdown::Both);
        }
        if let Some(child) = self.child.lock().ok().and_then(|mut slot| slot.take()) {
            terminate_child(child, deadline);
        }
        if let Some(handle) = self
            .output_reader
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
        {
            join_reader(handle, deadline);
        }
        if let Some(handle) = self.reader.lock().ok().and_then(|mut slot| slot.take()) {
            join_reader(handle, deadline);
        }
    }

    /// Sends the child termination signal without waiting for process or reader cleanup.
    fn signal_stop(&self) {
        if let Ok(mut slot) = self.child.lock() {
            if !self.stopped.swap(true, Ordering::SeqCst) {
                if let Some(child) = slot.as_mut() {
                    let _ = child.kill();
                }
            }
        } else {
            self.stopped.store(true, Ordering::SeqCst);
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

/// A small HTTP body reader for `/api/event`. ureq's body timeout is a total-body timeout, which
/// would periodically tear down a healthy event stream. The bridge keeps a clone of this socket
/// so stop can interrupt a silent read without changing the stream's lifetime.
struct SseReader {
    reader: BufReader<TcpStream>,
    chunked: bool,
    remaining_chunk: usize,
    finished: bool,
}

impl Read for SseReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.finished || buffer.is_empty() {
            return Ok(0);
        }
        if self.chunked && self.remaining_chunk == 0 {
            let mut size = String::new();
            self.reader.read_line(&mut size)?;
            let size = size
                .trim_end()
                .split(';')
                .next()
                .and_then(|value| usize::from_str_radix(value.trim(), 16).ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE chunk"))?;
            if size == 0 {
                let mut trailer = String::new();
                loop {
                    trailer.clear();
                    if self.reader.read_line(&mut trailer)? == 0 {
                        self.finished = true;
                        return Ok(0);
                    }
                    if trailer == "\r\n" || trailer == "\n" {
                        break;
                    }
                }
                self.finished = true;
                return Ok(0);
            }
            self.remaining_chunk = size;
        }

        let limit = if self.chunked {
            buffer.len().min(self.remaining_chunk)
        } else {
            buffer.len()
        };
        let read = self.reader.read(&mut buffer[..limit])?;
        if read == 0 {
            self.finished = true;
            return Ok(0);
        }
        if self.chunked {
            self.remaining_chunk -= read;
            if self.remaining_chunk == 0 {
                let mut crlf = [0; 2];
                self.reader.read_exact(&mut crlf)?;
                if crlf != *b"\r\n" {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid SSE chunk terminator",
                    ));
                }
            }
        }
        Ok(read)
    }
}

fn event_stream(bridge: &AgentBridge) -> io::Result<SseReader> {
    let address = SocketAddr::from(([127, 0, 0, 1], bridge.credentials.port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))?;
    let interrupt_socket = stream.try_clone()?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET /api/event HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
        bridge.credentials.port,
        bridge.auth_header()
    )?;
    if !bridge.install_event_socket(interrupt_socket) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "the event stream was stopped",
        ));
    }

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE response"))?;
    if status != 200 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the agent event stream was rejected",
        ));
    }
    let mut chunked = false;
    let mut header = String::new();
    loop {
        header.clear();
        reader.read_line(&mut header)?;
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|value| value.trim().eq_ignore_ascii_case("chunked"))
            {
                chunked = true;
            }
        }
    }
    Ok(SseReader {
        reader,
        chunked,
        remaining_chunk: 0,
        finished: false,
    })
}

/// Consumes `/api/event` until the server dies, normalizing and forwarding each frame.
///
/// Reconnects with a bounded backoff: a dropped stream is normal (laptop sleep, server
/// restart) and a fresh stream replays no history, so the caller must re-read state after
/// a reconnect rather than assume nothing was missed.
fn read_events(bridge: Arc<AgentBridge>, sink: EventSink) {
    let mut backoff = Duration::from_millis(250);
    while !bridge.is_stopped() {
        let mut reader = BufReader::new(match event_stream(&bridge) {
            Ok(reader) => reader,
            Err(_) => {
                bridge.clear_event_socket();
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
        bridge.clear_event_socket();
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

/// Kills only the child owned by this bridge and reaps it. If the bounded poll cannot observe
/// exit, a detached reaper owns the child and performs the final wait without extending the
/// caller's deadline.
fn terminate_child(mut child: Child, deadline: Instant) {
    let _ = child.kill();
    reap_child(child, deadline);
}

fn reap_child(mut child: Child, deadline: Instant) {
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Err(_) => return reap_child_in_background(child),
            Ok(None) if Instant::now() >= deadline => break,
            Ok(None) => {
                sleep(CHILD_REAP_POLL.min(deadline.saturating_duration_since(Instant::now())))
            }
        }
    }
    reap_child_in_background(child);
}

fn reap_child_in_background(child: Child) {
    let _ = std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
}

/// Reader I/O is interruptible, so the normal path finishes before the deadline. If an
/// unexpected reader does not finish, dropping its handle detaches it rather than blocking the
/// caller. Joining the current event thread would panic; it is already on the return path.
fn join_reader(handle: std::thread::JoinHandle<()>, deadline: Instant) {
    if handle.thread().id() == std::thread::current().id() {
        return;
    }
    while !handle.is_finished() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        sleep(CHILD_REAP_POLL.min(remaining));
    }
    if handle.is_finished() {
        let _ = handle.join();
    }
}

fn json_client(timeout: Duration) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .timeout_connect(Some(timeout))
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

/// The window of ports a bridge may take, starting at `PORT_RANGE_START`.
///
/// It is wide on purpose. A bridge that is killed with its parent cannot run its own cleanup,
/// so the port it was given stays taken by a process nobody manages, and a narrow window fills
/// up after a few crashes and leaves the next bridge with nowhere to start. A few hundred
/// ports costs nothing and makes that recoverable without touching the leftovers.
const PORT_RANGE_START: u16 = 46000;
const PORT_RANGE_LEN: u16 = 512;
const MAX_START_ATTEMPTS: usize = 3;

/// Checks candidates by briefly binding each port and releasing it before the child starts. The
/// port is not reserved until the child binds it; the startup loop may rescan the window after a
/// bind-to-spawn race.
fn free_port() -> Result<u16, BridgeError> {
    free_port_with(is_port_available)
}

/// Applies the injected availability predicate to candidates from `port_candidates`; the
/// corresponding post-failure hook must be checked only after child termination.
fn free_port_with(is_available: impl FnMut(u16) -> bool) -> Result<u16, BridgeError> {
    first_available_port(
        port_candidates(NEXT_PORT_ATTEMPT.fetch_add(1, Ordering::Relaxed)),
        is_available,
    )
    .ok_or_else(|| {
        BridgeError::Unavailable("could not find a free port for the agent server".into())
    })
}

fn is_port_available(candidate: u16) -> bool {
    TcpListener::bind(("127.0.0.1", candidate)).is_ok()
}

fn no_startup_child_tracking(_: &StartupChild) {}

fn port_candidates(start_offset: u64) -> impl Iterator<Item = u16> {
    let range_len = u64::from(PORT_RANGE_LEN);
    let start_offset = start_offset % range_len;
    (0..PORT_RANGE_LEN).map(move |attempt| {
        PORT_RANGE_START + ((start_offset + u64::from(attempt)) % range_len) as u16
    })
}

fn first_available_port(
    candidates: impl IntoIterator<Item = u16>,
    mut is_available: impl FnMut(u16) -> bool,
) -> Option<u16> {
    candidates
        .into_iter()
        .find(|candidate| is_available(*candidate))
}

fn startup_timeout_error() -> BridgeError {
    BridgeError::Unavailable("the agent server timed out while starting".into())
}

fn is_early_eof(error: &BridgeError) -> bool {
    matches!(
        error,
        BridgeError::Unavailable(message)
            if message == EARLY_EOF_MESSAGE
    )
}

/// Reads the password the child prints. The reader remains alive after the password so stdout
/// written later cannot block the server. The port is the one we asked for, and
/// `finish_credentials` proves the child actually bound it.
fn read_credentials(
    child: &mut Child,
    directory: &Path,
    port: u16,
    deadline: Instant,
) -> Result<(ServerCredentials, std::thread::JoinHandle<()>), BridgeError> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Unavailable("the agent server produced no output".into()))?;

    let (credentials_tx, credentials_rx) = mpsc::channel();
    let output_reader = std::thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        let mut reported = false;
        loop {
            match lines.next() {
                Some(Ok(line)) if !reported => {
                    if let Some(password) = line.strip_prefix("server password ") {
                        reported = true;
                        let _ = credentials_tx.send(Ok(Some(password.trim().to_string())));
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    if !reported {
                        let _ = credentials_tx.send(Err(error));
                    }
                    break;
                }
                None => {
                    if !reported {
                        let _ = credentials_tx.send(Ok(None));
                    }
                    break;
                }
            }
        }
    });

    let result = credentials_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    let password = match result {
        Ok(Ok(Some(password))) => password,
        Ok(Ok(None)) => {
            let _ = child.kill();
            join_reader(output_reader, Instant::now() + STARTUP_STOP_WAIT);
            return Err(BridgeError::Unavailable(EARLY_EOF_MESSAGE.into()));
        }
        Ok(Err(error)) => {
            let _ = child.kill();
            join_reader(output_reader, Instant::now() + STARTUP_STOP_WAIT);
            return Err(BridgeError::Unavailable(format!(
                "could not read the agent server output: {error}"
            )));
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            join_reader(output_reader, Instant::now() + STARTUP_STOP_WAIT);
            return Err(BridgeError::Unavailable(
                "timed out waiting for the agent server credentials".into(),
            ));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            join_reader(output_reader, Instant::now() + STARTUP_STOP_WAIT);
            return Err(BridgeError::Unavailable(EARLY_EOF_MESSAGE.into()));
        }
    };

    Ok((
        ServerCredentials {
            port,
            password,
            directory: directory.to_path_buf(),
        },
        output_reader,
    ))
}

/// Confirms the server is really up on the port we asked for, and really scoped to this
/// checkout, before the bridge is handed out.
///
/// Ready means "the agent catalog has something in it", not merely "a route answered":
/// `/api/agent` is empty for the first moments after the password is printed, while the
/// server is still loading its configuration. A bridge that returned from that window would
/// tell every later reader that the project has no agents at all, and the colors the sidebar
/// paints come from that list. Waiting here costs about a second of boot, once per checkout,
/// and no later call has to wonder.
fn finish_credentials(
    credentials: ServerCredentials,
    deadline: Instant,
) -> Result<ServerCredentials, BridgeError> {
    let probe = AgentBridge {
        checkout_id: String::new(),
        child: Mutex::new(None),
        credentials: credentials.clone(),
        output_reader: Mutex::new(None),
        event_socket: Mutex::new(None),
        reader: Mutex::new(None),
        stopped: std::sync::atomic::AtomicBool::new(false),
    };
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(BridgeError::Unavailable(
                "the agent server timed out while waiting for readiness".into(),
            ));
        }
        // /api/agent is cheap and confirms reachability, directory scoping and readiness at
        // once, which is why this route and not another.
        let catalog: Vec<serde_json::Value> =
            probe.get_json_with_timeout("/api/agent", remaining)?;
        if !catalog.is_empty() || Instant::now() >= deadline {
            // A catalog that never fills is still a usable server: the bridge works, and a row
            // with no color of its own falls back rather than failing.
            return Ok(credentials);
        }
        sleep(Duration::from_millis(50).min(remaining));
    }
}

/// Parses the `{data: …}` envelope every route replies with.
fn send_json<T: serde::de::DeserializeOwned>(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<ApiEnvelope<T>, BridgeError> {
    match response {
        Ok(mut response) => {
            let raw = response
                .body_mut()
                .read_to_vec()
                .map_err(|error| match error {
                    ureq::Error::Timeout(_) => {
                        BridgeError::Unavailable("the agent server request timed out".into())
                    }
                    error => BridgeError::Failed(format!("could not read the reply: {error}")),
                })?;
            // The body is deliberately not echoed: it can carry session content.
            serde_json::from_slice(&raw).map_err(|error| {
                BridgeError::Failed(format!("unexpected agent server reply: {error}"))
            })
        }
        // Status errors are reduced to safe, actionable details; their bodies are not echoed.
        Err(ureq::Error::StatusCode(status)) => Err(BridgeError::Failed(status_detail(status))),
        Err(ureq::Error::Timeout(_)) => Err(BridgeError::Unavailable(
            "the agent server request timed out".into(),
        )),
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

type BridgeLauncher = dyn Fn(&str, &Path) -> Result<Arc<AgentBridge>, BridgeError> + Send + Sync;
type ReaderStarter = dyn Fn(&Arc<AgentBridge>, &EventSink) + Send + Sync;
type BridgeStopper = dyn Fn(&Arc<AgentBridge>, Duration) + Send + Sync;
type SlotStopper = dyn Fn(Arc<BridgeSlot>, Duration) + Send + Sync;

enum BridgeState {
    Starting { stop_requested: bool },
    Ready(Arc<AgentBridge>),
    Failed(BridgeError),
    Stopping,
    Stopped,
}

struct BridgeSlot {
    state: Mutex<BridgeState>,
    changed: Condvar,
}

fn remove_slot_if_current(
    bridges: &mut HashMap<String, Arc<BridgeSlot>>,
    checkout_id: &str,
    slot: &Arc<BridgeSlot>,
) {
    if bridges
        .get(checkout_id)
        .is_some_and(|current| Arc::ptr_eq(current, slot))
    {
        bridges.remove(checkout_id);
    }
}

impl BridgeSlot {
    fn starting() -> Self {
        Self {
            state: Mutex::new(BridgeState::Starting {
                stop_requested: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn wait_with_timeout(&self, timeout: Duration) -> Result<Arc<AgentBridge>, BridgeError> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .map_err(|_| BridgeError::Failed("the agent bridge state is poisoned".into()))?;
        loop {
            match &*state {
                BridgeState::Ready(bridge) => return Ok(Arc::clone(bridge)),
                BridgeState::Failed(error) => return Err(error.clone()),
                BridgeState::Stopped => {
                    return Err(BridgeError::Failed(
                        "the agent bridge was stopped while starting".into(),
                    ))
                }
                BridgeState::Starting { .. } | BridgeState::Stopping => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(BridgeError::Unavailable(
                            "timed out waiting for the agent bridge to start".into(),
                        ));
                    }
                    let (next, timed_out) =
                        self.changed.wait_timeout(state, remaining).map_err(|_| {
                            BridgeError::Failed("the agent bridge state is poisoned".into())
                        })?;
                    state = next;
                    if timed_out.timed_out()
                        && matches!(
                            &*state,
                            BridgeState::Starting { .. } | BridgeState::Stopping
                        )
                    {
                        return Err(BridgeError::Unavailable(
                            "timed out waiting for the agent bridge to start".into(),
                        ));
                    }
                }
            }
        }
    }

    /// Signals a published child immediately, or records cancellation for an in-flight start.
    fn signal(&self) {
        let bridge = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            match &mut *state {
                BridgeState::Starting { stop_requested } => {
                    *stop_requested = true;
                    self.changed.notify_all();
                    return;
                }
                BridgeState::Ready(bridge) => Some(Arc::clone(bridge)),
                BridgeState::Failed(_) | BridgeState::Stopping | BridgeState::Stopped => None,
            }
        };
        if let Some(bridge) = bridge {
            bridge.signal_stop();
        }
    }

    fn stop(&self, stopper: &BridgeStopper, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        self.signal();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        loop {
            match &mut *state {
                BridgeState::Starting { stop_requested } => {
                    *stop_requested = true;
                    self.changed.notify_all();
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        *state = BridgeState::Stopped;
                        self.changed.notify_all();
                        return;
                    }
                    let (next, timed_out) = match self.changed.wait_timeout(state, remaining) {
                        Ok(wait) => wait,
                        Err(_) => return,
                    };
                    state = next;
                    if timed_out.timed_out() && matches!(*state, BridgeState::Starting { .. }) {
                        *state = BridgeState::Stopped;
                        self.changed.notify_all();
                        return;
                    }
                }
                BridgeState::Stopping => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        *state = BridgeState::Stopped;
                        self.changed.notify_all();
                        return;
                    }
                    let (next, timed_out) = match self.changed.wait_timeout(state, remaining) {
                        Ok(wait) => wait,
                        Err(_) => return,
                    };
                    state = next;
                    if timed_out.timed_out() && matches!(*state, BridgeState::Stopping) {
                        *state = BridgeState::Stopped;
                        self.changed.notify_all();
                        return;
                    }
                }
                BridgeState::Ready(bridge) => {
                    let bridge = Arc::clone(bridge);
                    *state = BridgeState::Stopping;
                    drop(state);
                    stopper(&bridge, deadline.saturating_duration_since(Instant::now()));
                    if let Ok(mut state) = self.state.lock() {
                        *state = BridgeState::Stopped;
                        self.changed.notify_all();
                    }
                    return;
                }
                BridgeState::Failed(_) | BridgeState::Stopped => return,
            }
        }
    }
}

/// Owns one server per checkout for the app's lifetime.
pub struct AgentService {
    bridges: Mutex<HashMap<String, Arc<BridgeSlot>>>,
    sink: Mutex<Option<EventSink>>,
    launcher: Arc<BridgeLauncher>,
    reader_starter: Arc<ReaderStarter>,
    stopper: Arc<BridgeStopper>,
    slot_stopper: Arc<SlotStopper>,
    startup_stop_wait: Duration,
    startup_wait: Duration,
}

impl AgentService {
    pub fn new() -> Self {
        let mut service = Self::with_lifecycle(
            Arc::new(|checkout_id, directory| {
                AgentBridge::start(checkout_id, directory).map(Arc::new)
            }),
            Arc::new(|bridge, sink| bridge.start_reader(sink)),
            Arc::new(|bridge, timeout| bridge.stop_with_timeout(timeout)),
            STARTUP_STOP_WAIT,
        );
        service.startup_wait = STARTUP_WAIT_TIMEOUT;
        service
    }

    fn with_lifecycle(
        launcher: Arc<BridgeLauncher>,
        reader_starter: Arc<ReaderStarter>,
        stopper: Arc<BridgeStopper>,
        startup_stop_wait: Duration,
    ) -> Self {
        let slot_stopper_callback = Arc::clone(&stopper);
        Self::with_lifecycle_and_slot_stopper(
            launcher,
            reader_starter,
            stopper,
            startup_stop_wait,
            Arc::new(move |slot, timeout| {
                slot.stop(slot_stopper_callback.as_ref(), timeout);
            }),
        )
    }

    fn with_lifecycle_and_slot_stopper(
        launcher: Arc<BridgeLauncher>,
        reader_starter: Arc<ReaderStarter>,
        stopper: Arc<BridgeStopper>,
        startup_stop_wait: Duration,
        slot_stopper: Arc<SlotStopper>,
    ) -> Self {
        Self {
            bridges: Mutex::new(HashMap::new()),
            sink: Mutex::new(None),
            launcher,
            reader_starter,
            stopper,
            slot_stopper,
            startup_stop_wait,
            startup_wait: startup_stop_wait,
        }
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
        // Reserve this checkout under the global lock, then boot and create its reader only
        // after releasing it. Stops likewise remove slots before killing or joining anything.
        let (slot, is_starter) = {
            let mut bridges = self
                .bridges
                .lock()
                .map_err(|_| BridgeError::Failed("the agent service is poisoned".into()))?;
            match bridges.get(checkout_id) {
                Some(slot) => (Arc::clone(slot), false),
                None => {
                    let slot = Arc::new(BridgeSlot::starting());
                    bridges.insert(checkout_id.to_string(), Arc::clone(&slot));
                    (slot, true)
                }
            }
        };

        if !is_starter {
            return slot.wait_with_timeout(self.startup_wait);
        }

        let sink = self
            .sink
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(Arc::clone));
        let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.launcher)(checkout_id, directory)
        }))
        .unwrap_or_else(|_| {
            Err(BridgeError::Failed(
                "the agent bridge launcher panicked".into(),
            ))
        });
        match started {
            Ok(bridge) => {
                let mut state = slot.state.lock().map_err(|_| {
                    BridgeError::Failed("the agent bridge state is poisoned".into())
                })?;
                if !matches!(
                    &*state,
                    BridgeState::Starting {
                        stop_requested: false
                    }
                ) {
                    // Stop already removed the slot, so a late successful start must only clean
                    // up its child and must never publish over a later checkout generation.
                    *state = BridgeState::Stopping;
                    drop(state);
                    (self.stopper)(&bridge, self.startup_stop_wait);
                    if let Ok(mut state) = slot.state.lock() {
                        *state = BridgeState::Stopped;
                        slot.changed.notify_all();
                    }
                    return Err(BridgeError::Failed(
                        "the agent bridge was stopped while starting".into(),
                    ));
                }
                if let Some(sink) = sink {
                    // Start the reader before publishing Ready, so a concurrent stop cannot
                    // finish and then have a reader started for the stopped bridge.
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        (self.reader_starter)(&bridge, &sink)
                    }))
                    .is_err()
                    {
                        let error =
                            BridgeError::Failed("the agent event reader starter panicked".into());
                        *state = BridgeState::Failed(error.clone());
                        slot.changed.notify_all();
                        drop(state);
                        if let Ok(mut bridges) = self.bridges.lock() {
                            remove_slot_if_current(&mut bridges, checkout_id, &slot);
                        }
                        (self.stopper)(&bridge, self.startup_stop_wait);
                        return Err(error);
                    }
                }
                *state = BridgeState::Ready(Arc::clone(&bridge));
                slot.changed.notify_all();
                Ok(bridge)
            }
            Err(error) => {
                let mut state = slot.state.lock().map_err(|_| {
                    BridgeError::Failed("the agent bridge state is poisoned".into())
                })?;
                if matches!(&*state, BridgeState::Stopped)
                    || matches!(
                        &*state,
                        BridgeState::Starting {
                            stop_requested: true
                        }
                    )
                {
                    *state = BridgeState::Stopped;
                } else {
                    *state = BridgeState::Failed(error.clone());
                }
                slot.changed.notify_all();
                drop(state);
                if let Ok(mut bridges) = self.bridges.lock() {
                    remove_slot_if_current(&mut bridges, checkout_id, &slot);
                }
                Err(error)
            }
        }
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

    /// Every agent the checkout's server offers, with the color OpenCode paints it with.
    ///
    /// The list is the same for every session in the checkout, so this is one call behind a
    /// cache rather than one per session.
    pub fn agents(
        &self,
        checkout_id: &str,
        directory: &Path,
    ) -> Result<Vec<AgentAgent>, BridgeError> {
        let bridge = self.bridge(checkout_id, directory)?;
        let listed: Vec<ApiAgent> = bridge.get_json("/api/agent")?;
        Ok(listed
            .into_iter()
            .map(|raw| AgentAgent {
                id: raw.id,
                name: raw.name,
                mode: raw.mode,
                color: raw.color,
                hidden: raw.hidden,
            })
            .collect())
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
        let slot = self
            .bridges
            .lock()
            .ok()
            .and_then(|mut bridges| bridges.remove(checkout_id));
        if let Some(slot) = slot {
            slot.stop(self.stopper.as_ref(), self.startup_stop_wait);
        }
    }

    pub fn stop_all(&self) {
        let slots = self
            .bridges
            .lock()
            .map(|mut bridges| bridges.drain().map(|(_, slot)| slot).collect::<Vec<_>>())
            .unwrap_or_default();
        // Signal every currently published child before any stopper waits for wait()/join().
        for slot in &slots {
            slot.signal();
        }
        let deadline = Instant::now() + self.startup_stop_wait;
        for slot in slots {
            (self.slot_stopper)(slot, deadline.saturating_duration_since(Instant::now()));
        }
    }
}

impl Default for AgentService {
    fn default() -> Self {
        Self::new()
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
        collections::HashMap,
        io::{self, Read, Write},
        net::{Shutdown, TcpListener},
        panic::{catch_unwind, AssertUnwindSafe},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
        thread::sleep,
        time::{Duration, Instant},
    };

    use crate::{
        domain::review::{review_anchor_hash, review_round_marker},
        services::review_round::build_round_prompt,
    };

    use super::{
        agent_program, basic_credentials, event_stream, finish_credentials, first_available_port,
        free_port_with, join_reader, no_startup_child_tracking, port_candidates, read_credentials,
        remove_slot_if_current, same_directory, status_detail, terminate_child,
        validate_session_id, AgentBridge, AgentEvent, AgentService, BridgeError, BridgeState,
        BridgeStopper, ChildGuard, EventSink, PortHooks, ServerCredentials, StartupChild,
        EARLY_EOF_MESSAGE, MAX_PROMPT_BYTES, MAX_START_ATTEMPTS, PORT_RANGE_LEN, PORT_RANGE_START,
    };

    fn required_live_directory(name: &str) -> PathBuf {
        let directory = std::env::var(name)
            .ok()
            .filter(|directory| !directory.is_empty())
            .unwrap_or_else(|| {
                panic!(
                    "{name} is required for ignored live agent tests; set it to a dedicated temporary directory"
                )
            });
        PathBuf::from(directory)
    }

    fn fake_bridge(checkout_id: &str) -> Arc<AgentBridge> {
        // These tests cover the service lifecycle, not OS child-process behavior.
        Arc::new(AgentBridge {
            checkout_id: checkout_id.to_string(),
            child: Mutex::new(None),
            credentials: ServerCredentials {
                port: 1,
                password: "test".into(),
                directory: PathBuf::new(),
            },
            output_reader: Mutex::new(None),
            event_socket: Mutex::new(None),
            reader: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        })
    }

    struct ReaderExitSignal(Option<mpsc::Sender<()>>);

    impl Drop for ReaderExitSignal {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    fn test_service(
        launcher: impl Fn(&str, &Path) -> Result<Arc<AgentBridge>, BridgeError> + Send + Sync + 'static,
        readers: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
    ) -> AgentService {
        AgentService::with_lifecycle(
            Arc::new(launcher),
            Arc::new(move |_, _| {
                readers.fetch_add(1, Ordering::SeqCst);
            }),
            Arc::new(move |_, _| {
                stops.fetch_add(1, Ordering::SeqCst);
            }),
            Duration::from_millis(400),
        )
    }

    #[test]
    fn concurrent_requests_share_one_start_and_reader() {
        const CALLERS: usize = 20;
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let (results_tx, results_rx) = mpsc::channel();
        let launch_starts = Arc::clone(&starts);
        let service = Arc::new(test_service(
            move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        ));
        service.set_event_sink(Arc::new(|_| {}));
        let barrier = Arc::new(std::sync::Barrier::new(CALLERS + 1));
        let callers = (0..CALLERS)
            .map(|_| {
                let service = Arc::clone(&service);
                let barrier = Arc::clone(&barrier);
                let results_tx = results_tx.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let _ = results_tx.send(service.bridge("checkout", Path::new(".")));
                })
            })
            .collect::<Vec<_>>();
        drop(results_tx);

        barrier.wait();
        let launch_started = started_rx.recv_timeout(Duration::from_secs(2));
        // Release every possible launcher so a duplicate-start regression fails assertions
        // instead of stranding a worker on the injected gate.
        for _ in 0..CALLERS {
            release_tx.send(()).unwrap();
        }
        let bridges = (0..CALLERS)
            .map(|_| {
                results_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("all callers should complete")
                    .expect("bridge start should succeed")
            })
            .collect::<Vec<_>>();
        drop(callers);

        assert!(launch_started.is_ok(), "no caller launched the checkout");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(readers.load(Ordering::SeqCst), 1);
        assert!(bridges
            .iter()
            .all(|bridge| Arc::ptr_eq(bridge, &bridges[0])));
        service.stop("checkout");
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_waiter_timeout_does_not_cancel_a_slower_shared_start() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let starts_for_launcher = Arc::clone(&starts);
        let release_for_launcher = Arc::clone(&release_rx);
        let service = test_service(
            move |checkout_id, _| {
                starts_for_launcher.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                release_for_launcher.lock().unwrap().recv().unwrap();
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        );
        let mut service = service;
        service.startup_wait = Duration::from_millis(50);
        let service = Arc::new(service);
        service.set_event_sink(Arc::new(|_| {}));

        let starter_service = Arc::clone(&service);
        let (starter_result_tx, starter_result_rx) = mpsc::channel();
        let starter = std::thread::spawn(move || {
            let _ = starter_result_tx.send(starter_service.bridge("checkout", Path::new(".")));
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the slow launcher should start");

        let waiter_service = Arc::clone(&service);
        let (waiter_result_tx, waiter_result_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _ = waiter_result_tx.send(waiter_service.bridge("checkout", Path::new(".")));
        });
        assert!(waiter_result_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the waiter should finish at its deadline")
            .is_err());

        for _ in 0..2 {
            release_tx.send(()).unwrap();
        }
        assert!(starter_result_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the original starter should finish")
            .is_ok());
        starter.join().unwrap();
        waiter.join().unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(readers.load(Ordering::SeqCst), 1);
        service.stop("checkout");
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn different_checkouts_start_in_parallel() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_a_tx, release_a_rx) = mpsc::channel();
        let (release_b_tx, release_b_rx) = mpsc::channel();
        let release_a_rx = Arc::new(Mutex::new(release_a_rx));
        let release_b_rx = Arc::new(Mutex::new(release_b_rx));
        let (results_tx, results_rx) = mpsc::channel();
        let launch_starts = Arc::clone(&starts);
        let service = Arc::new(test_service(
            move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                started_tx.send(checkout_id.to_string()).unwrap();
                match checkout_id {
                    "a" => release_a_rx.lock().unwrap().recv().unwrap(),
                    "b" => release_b_rx.lock().unwrap().recv().unwrap(),
                    _ => unreachable!(),
                };
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        ));
        service.set_event_sink(Arc::new(|_| {}));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let a_service = Arc::clone(&service);
        let a_barrier = Arc::clone(&barrier);
        let a_results = results_tx.clone();
        let a = std::thread::spawn(move || {
            a_barrier.wait();
            let _ = a_results.send(a_service.bridge("a", Path::new(".")));
        });
        let b_service = Arc::clone(&service);
        let b_barrier = Arc::clone(&barrier);
        let b_results = results_tx.clone();
        let b = std::thread::spawn(move || {
            b_barrier.wait();
            let _ = b_results.send(b_service.bridge("b", Path::new(".")));
        });
        drop(results_tx);
        barrier.wait();

        let first = started_rx.recv_timeout(Duration::from_secs(5));
        let second = started_rx.recv_timeout(Duration::from_secs(5));
        release_a_tx.send(()).unwrap();
        release_b_tx.send(()).unwrap();
        let bridge_one = results_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first checkout should complete")
            .expect("first bridge should start");
        let bridge_two = results_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second checkout should complete")
            .expect("second bridge should start");
        drop((a, b));

        assert!(
            first.is_ok() && second.is_ok(),
            "checkouts did not boot in parallel"
        );
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(readers.load(Ordering::SeqCst), 2);
        let mut started = [
            first.expect("first checkout should reach the launcher"),
            second.expect("second checkout should reach the launcher"),
        ];
        started.sort();
        assert_eq!(started, ["a", "b"]);
        let mut bridge_ids = [
            bridge_one.checkout_id.as_str(),
            bridge_two.checkout_id.as_str(),
        ];
        bridge_ids.sort();
        assert_eq!(bridge_ids, ["a", "b"]);
        service.stop_all();
        assert_eq!(stops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stopping_during_start_prevents_publication_and_stops_the_child() {
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let (start_result_tx, start_result_rx) = mpsc::channel();
        let (stop_result_tx, stop_result_rx) = mpsc::channel();
        let service = Arc::new(test_service(
            move |checkout_id, _| {
                started_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        ));
        service.set_event_sink(Arc::new(|_| {}));
        let starter_service = Arc::clone(&service);
        let starter = std::thread::spawn(move || {
            let _ = start_result_tx.send(starter_service.bridge("checkout", Path::new(".")));
        });
        let launch_started = started_rx.recv_timeout(Duration::from_secs(2));
        let slot = Arc::clone(
            service
                .bridges
                .lock()
                .unwrap()
                .get("checkout")
                .expect("starting slot is published"),
        );
        let stop_service = Arc::clone(&service);
        let stopper = std::thread::spawn(move || {
            stop_service.stop("checkout");
            let _ = stop_result_tx.send(());
        });

        let mut state = slot.state.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(
            &*state,
            BridgeState::Starting {
                stop_requested: true
            }
        ) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (next, timed_out) = slot.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            if timed_out.timed_out() {
                break;
            }
        }
        let stop_requested = matches!(
            &*state,
            BridgeState::Starting {
                stop_requested: true
            }
        );
        drop(state);
        release_tx.send(()).unwrap();
        assert!(launch_started.is_ok(), "startup did not reach the launcher");
        stop_result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stop should finish after startup is released");
        assert!(start_result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("startup should complete")
            .is_err());
        drop((starter, stopper));
        assert!(stop_requested, "stop did not cancel the starting slot");
        assert!(service.bridges.lock().unwrap().get("checkout").is_none());
        assert_eq!(readers.load(Ordering::SeqCst), 0);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stop_all_cancels_an_in_flight_start_before_returning() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let (launch_tx, launch_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let (start_result_tx, start_result_rx) = mpsc::channel();
        let (stop_result_tx, stop_result_rx) = mpsc::channel();
        let launch_starts = Arc::clone(&starts);
        let service = Arc::new(test_service(
            move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                launch_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        ));
        service.set_event_sink(Arc::new(|_| {}));
        let starter_service = Arc::clone(&service);
        let starter = std::thread::spawn(move || {
            let _ = start_result_tx.send(starter_service.bridge("checkout", Path::new(".")));
        });
        let launch_started = launch_rx.recv_timeout(Duration::from_secs(2));
        let slot = Arc::clone(
            service
                .bridges
                .lock()
                .unwrap()
                .get("checkout")
                .expect("starting slot is published"),
        );
        let stop_service = Arc::clone(&service);
        let stopper = std::thread::spawn(move || {
            stop_service.stop_all();
            let _ = stop_result_tx.send(());
        });

        let mut state = slot.state.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(
            &*state,
            BridgeState::Starting {
                stop_requested: true
            }
        ) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (next, timed_out) = slot.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            if timed_out.timed_out() {
                break;
            }
        }
        let stop_requested = matches!(
            &*state,
            BridgeState::Starting {
                stop_requested: true
            }
        );
        drop(state);
        release_tx.send(()).unwrap();
        let stop_finished = stop_result_rx.recv_timeout(Duration::from_secs(2));
        let start_result = start_result_rx.recv_timeout(Duration::from_secs(2));
        drop((starter, stopper));

        assert!(launch_started.is_ok(), "startup did not reach the launcher");
        assert!(stop_requested, "stop_all did not cancel the starting slot");
        assert!(stop_finished.is_ok(), "stop_all did not finish");
        assert!(start_result.expect("startup should complete").is_err());
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(readers.load(Ordering::SeqCst), 0);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_stops_stop_a_checkout_once() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let launch_starts = Arc::clone(&starts);
        let service = Arc::new(test_service(
            move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        ));
        service.bridge("checkout", Path::new(".")).unwrap();

        let barrier = Arc::new(std::sync::Barrier::new(3));
        let (results_tx, results_rx) = mpsc::channel();
        let callers = (0..2)
            .map(|_| {
                let service = Arc::clone(&service);
                let barrier = Arc::clone(&barrier);
                let results_tx = results_tx.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    service.stop("checkout");
                    let _ = results_tx.send(());
                })
            })
            .collect::<Vec<_>>();
        drop(results_tx);
        barrier.wait();
        let first = results_rx.recv_timeout(Duration::from_secs(2));
        let second = results_rx.recv_timeout(Duration::from_secs(2));
        drop(callers);

        assert!(
            first.is_ok() && second.is_ok(),
            "both stop calls should return"
        );
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_startup_stop_wait_is_bounded() {
        let slot = Arc::new(super::BridgeSlot::starting());
        let stopper: Arc<BridgeStopper> = Arc::new(|_, _| {});
        let (done_tx, done_rx) = mpsc::channel();
        let stopping_slot = Arc::clone(&slot);
        let stopping_callback = Arc::clone(&stopper);
        let stop = std::thread::spawn(move || {
            stopping_slot.stop(stopping_callback.as_ref(), Duration::from_millis(20));
            let _ = done_tx.send(());
        });

        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("stop should time out instead of waiting forever");
        drop(stop);
        assert!(matches!(*slot.state.lock().unwrap(), BridgeState::Stopped));
    }

    #[test]
    fn waiting_for_a_start_is_bounded() {
        let slot = super::BridgeSlot::starting();
        let started = Instant::now();
        let error = match slot.wait_with_timeout(Duration::from_millis(20)) {
            Ok(_) => panic!("a starting slot without a publisher must time out"),
            Err(error) => error,
        };

        assert!(error_message(&error).contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(matches!(
            *slot.state.lock().unwrap(),
            BridgeState::Starting {
                stop_requested: false
            }
        ));
    }

    #[test]
    fn a_reader_join_does_not_outlive_its_budget() {
        let (release_tx, release_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = finished_tx.send(());
        });
        let started = Instant::now();
        join_reader(reader, Instant::now() + Duration::from_millis(20));

        assert!(started.elapsed() < Duration::from_millis(200));
        release_tx.send(()).unwrap();
        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the detached test reader should eventually exit");
    }

    #[test]
    fn failure_cleanup_preserves_a_replacement_slot() {
        let failed = Arc::new(super::BridgeSlot::starting());
        *failed.state.lock().unwrap() = BridgeState::Failed(BridgeError::Failed("failed".into()));
        let mut bridges = HashMap::new();
        bridges.insert("checkout".to_string(), Arc::clone(&failed));

        // stop removes failed S1; a later caller inserts S2 before S1's cleanup reaches map.
        let removed = bridges.remove("checkout").unwrap();
        let stopper: Arc<BridgeStopper> = Arc::new(|_, _| {});
        removed.stop(stopper.as_ref(), Duration::from_millis(20));
        let replacement = Arc::new(super::BridgeSlot::starting());
        bridges.insert("checkout".to_string(), Arc::clone(&replacement));
        remove_slot_if_current(&mut bridges, "checkout", &failed);

        assert!(Arc::ptr_eq(bridges.get("checkout").unwrap(), &replacement));
    }

    #[test]
    fn failed_start_can_be_retried() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let launch_starts = Arc::clone(&starts);
        let service = test_service(
            move |checkout_id, _| {
                if launch_starts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(BridgeError::Unavailable("injected failure".into()))
                } else {
                    Ok(fake_bridge(checkout_id))
                }
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        );
        service.set_event_sink(Arc::new(|_| {}));

        assert!(service.bridge("checkout", Path::new(".")).is_err());
        assert!(service.bridge("checkout", Path::new(".")).is_ok());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(readers.load(Ordering::SeqCst), 1);
        service.stop("checkout");
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panicking_launcher_releases_the_starting_slot() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let launch_starts = Arc::clone(&starts);
        let service = test_service(
            move |checkout_id, _| {
                if launch_starts.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("injected launcher panic");
                }
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        );

        assert!(service.bridge("checkout", Path::new(".")).is_err());
        assert!(service.bridge("checkout", Path::new(".")).is_ok());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        service.stop("checkout");
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stop_all_stops_each_child_once() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let launch_starts = Arc::clone(&starts);
        let service = test_service(
            move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        );
        service.set_event_sink(Arc::new(|_| {}));
        service.bridge("a", Path::new(".")).unwrap();
        service.bridge("b", Path::new(".")).unwrap();

        service.stop_all();
        service.stop_all();
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(readers.load(Ordering::SeqCst), 2);
        assert_eq!(stops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stop_all_signals_every_child_before_waiting_for_cleanup() {
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let bridges = Arc::new(Mutex::new(Vec::<Arc<AgentBridge>>::new()));
        let (cleanup_started_tx, cleanup_started_rx) = mpsc::channel();
        let (release_cleanup_tx, release_cleanup_rx) = mpsc::channel();
        let release_cleanup_rx = Arc::new(Mutex::new(release_cleanup_rx));
        let launch_starts = Arc::clone(&starts);
        let launch_bridges = Arc::clone(&bridges);
        let cleanup_bridges = Arc::clone(&bridges);
        let cleanup_stops = Arc::clone(&stops);
        let service = Arc::new(AgentService::with_lifecycle(
            Arc::new(move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                let bridge = fake_bridge(checkout_id);
                launch_bridges.lock().unwrap().push(Arc::clone(&bridge));
                Ok(bridge)
            }),
            Arc::new(|_, _| {}),
            Arc::new(move |_, _| {
                if cleanup_stops.fetch_add(1, Ordering::SeqCst) == 0 {
                    let signaled = cleanup_bridges
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|bridge| bridge.stopped.load(Ordering::SeqCst))
                        .count();
                    cleanup_started_tx.send(signaled).unwrap();
                    release_cleanup_rx.lock().unwrap().recv().unwrap();
                }
            }),
            Duration::from_millis(400),
        ));
        service.bridge("a", Path::new(".")).unwrap();
        service.bridge("b", Path::new(".")).unwrap();

        let stopping_service = Arc::clone(&service);
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let stop_all = std::thread::spawn(move || {
            stopping_service.stop_all();
            let _ = stopped_tx.send(());
        });
        let signaled = cleanup_started_rx.recv_timeout(Duration::from_secs(2));
        release_cleanup_tx.send(()).unwrap();
        let stopped = stopped_rx.recv_timeout(Duration::from_secs(2));
        drop(stop_all);

        assert_eq!(signaled.expect("cleanup should start"), 2);
        assert!(
            stopped.is_ok(),
            "stop_all should finish after cleanup is released"
        );
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(stops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stop_all_passes_the_remaining_shared_budget_to_each_slot() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&received);
        let service = AgentService::with_lifecycle_and_slot_stopper(
            Arc::new(|_, _| panic!("stop_all should not launch bridges")),
            Arc::new(|_, _| {}),
            Arc::new(|_, _| {}),
            Duration::from_millis(400),
            Arc::new(move |slot, timeout| {
                let call = {
                    let mut received = observed.lock().unwrap();
                    received.push(timeout);
                    received.len()
                };
                *slot.state.lock().unwrap() = BridgeState::Stopped;
                slot.changed.notify_all();
                if call == 1 {
                    // Exhaust the shared budget deterministically before the second slot.
                    std::thread::sleep(Duration::from_millis(500));
                }
            }),
        );
        let first = Arc::new(super::BridgeSlot::starting());
        let second = Arc::new(super::BridgeSlot::starting());
        {
            let mut slots = service.bridges.lock().unwrap();
            slots.insert("a".into(), Arc::clone(&first));
            slots.insert("b".into(), Arc::clone(&second));
        }

        service.stop_all();
        let received = received.lock().unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[1], Duration::ZERO);
    }

    #[test]
    fn stop_all_cancels_multiple_starts_within_smoke_limit() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_a_tx, release_a_rx) = mpsc::channel();
        let (release_b_tx, release_b_rx) = mpsc::channel();
        let release_a_rx = Arc::new(Mutex::new(release_a_rx));
        let release_b_rx = Arc::new(Mutex::new(release_b_rx));
        let launch_starts = Arc::clone(&starts);
        let service = Arc::new(test_service(
            move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                started_tx.send(checkout_id.to_string()).unwrap();
                match checkout_id {
                    "a" => release_a_rx.lock().unwrap().recv().unwrap(),
                    "b" => release_b_rx.lock().unwrap().recv().unwrap(),
                    _ => unreachable!(),
                };
                Ok(fake_bridge(checkout_id))
            },
            Arc::clone(&readers),
            Arc::clone(&stops),
        ));
        let (result_tx, result_rx) = mpsc::channel();
        let a_service = Arc::clone(&service);
        let a_result = result_tx.clone();
        let a = std::thread::spawn(move || {
            let _ = a_result.send(a_service.bridge("a", Path::new(".")));
        });
        let b_service = Arc::clone(&service);
        let b_result = result_tx.clone();
        let b = std::thread::spawn(move || {
            let _ = b_result.send(b_service.bridge("b", Path::new(".")));
        });
        drop(result_tx);
        let first_started = started_rx.recv_timeout(Duration::from_secs(2));
        let second_started = started_rx.recv_timeout(Duration::from_secs(2));

        let (stopped_tx, stopped_rx) = mpsc::channel();
        let stopping_service = Arc::clone(&service);
        let stop_all = std::thread::spawn(move || {
            stopping_service.stop_all();
            let _ = stopped_tx.send(());
        });
        // Deadline sharing is pinned structurally above; this is only a generous hang check.
        let stopped = stopped_rx.recv_timeout(Duration::from_secs(10));
        release_a_tx.send(()).unwrap();
        release_b_tx.send(()).unwrap();
        let first_result = result_rx.recv_timeout(Duration::from_secs(2));
        let second_result = result_rx.recv_timeout(Duration::from_secs(2));
        drop((a, b, stop_all));

        assert!(first_started.is_ok() && second_started.is_ok());
        assert!(stopped.is_ok(), "stop_all exceeded the smoke limit");
        assert!(first_result.expect("first start should complete").is_err());
        assert!(second_result
            .expect("second start should complete")
            .is_err());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(readers.load(Ordering::SeqCst), 0);
        assert_eq!(stops.load(Ordering::SeqCst), 2);
    }

    fn fixture_child(directory: &Path, script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the POSIX test fixture should start")
    }

    fn fixture_pid_is_gone(pid: u32) -> bool {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    struct FixtureChildCleanup(Arc<Mutex<Vec<StartupChild>>>);

    impl FixtureChildCleanup {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Vec::new())))
        }

        fn tracker(&self) -> Arc<Mutex<Vec<StartupChild>>> {
            Arc::clone(&self.0)
        }
    }

    impl Drop for FixtureChildCleanup {
        fn drop(&mut self) {
            let children = self
                .0
                .lock()
                .expect("fixture cleanup lock should not be poisoned")
                .drain(..)
                .collect::<Vec<_>>();
            for child in children {
                if let Some(child) = child.take() {
                    terminate_child(child, Instant::now() + Duration::from_secs(1));
                }
            }
        }
    }

    #[test]
    fn child_guard_reaps_the_child_when_startup_unwinds() {
        let directory = tempfile::tempdir().unwrap();
        let child = fixture_child(directory.path(), "exec tail -f /dev/null");
        let pid = child.id() as libc::pid_t;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = ChildGuard::new(child);
            panic!("injected startup panic");
        }));

        assert!(result.is_err());
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn credential_wait_times_out_when_stdout_stays_open_without_output() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = fixture_child(directory.path(), "exec tail -f /dev/null");
        let error = read_credentials(
            &mut child,
            directory.path(),
            1,
            Instant::now() + Duration::from_millis(50),
        )
        .expect_err("a silent child must not satisfy startup");

        assert!(error_message(&error).contains("timed out"));
        assert!(
            child.try_wait().unwrap().is_some(),
            "fixture child was not reaped"
        );
    }

    #[test]
    fn credential_wait_times_out_on_a_partial_line() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = fixture_child(directory.path(), "printf partial; exec tail -f /dev/null");
        let error = read_credentials(
            &mut child,
            directory.path(),
            1,
            Instant::now() + Duration::from_millis(50),
        )
        .expect_err("a partial line without EOF must not satisfy startup");

        assert!(error_message(&error).contains("timed out"));
        assert!(
            child.try_wait().unwrap().is_some(),
            "fixture child was not reaped"
        );
    }

    #[test]
    fn credential_wait_distinguishes_early_eof() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = fixture_child(directory.path(), "printf 'server is starting\\n'");
        let error = read_credentials(
            &mut child,
            directory.path(),
            1,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("a child that exits before credentials must fail");

        assert!(error_message(&error).contains("EOF"));
        assert!(
            child.try_wait().unwrap().is_some(),
            "fixture child was not reaped"
        );
    }

    #[test]
    fn credential_reader_drains_stdout_after_startup() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = fixture_child(
            directory.path(),
            "printf 'server password secret\\nstdout after boot\\n'; exec tail -f /dev/null",
        );
        let (credentials, output_reader) = read_credentials(
            &mut child,
            directory.path(),
            1,
            Instant::now() + Duration::from_secs(1),
        )
        .expect("the fixture credentials should be read");

        assert_eq!(credentials.password, "secret");
        terminate_child(child, Instant::now() + Duration::from_secs(1));
        join_reader(output_reader, Instant::now() + Duration::from_secs(1));
    }

    #[test]
    fn readiness_request_times_out_when_the_child_never_answers_http() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener.local_addr().unwrap().port();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test request should connect");
            accepted_tx.send(()).unwrap();
            let _ = release_rx.recv();
            let _ = stream.write_all(b"");
        });
        let error = finish_credentials(
            ServerCredentials {
                port,
                password: "secret".into(),
                directory: directory.path().to_path_buf(),
            },
            Instant::now() + Duration::from_millis(100),
        )
        .expect_err("an HTTP server that never answers must time out");

        accepted_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the readiness request should reach the fixture");
        release_tx.send(()).unwrap();
        server.join().unwrap();
        assert!(error_message(&error).contains("timed out"));
    }

    #[test]
    fn stopping_interrupts_an_idle_sse_reader() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener.local_addr().unwrap().port();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("SSE request should connect");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
                )
                .unwrap();
            accepted_tx.send(()).unwrap();
            let _ = release_rx.recv();
        });
        let child = fixture_child(directory.path(), "exec tail -f /dev/null");
        let bridge = Arc::new(AgentBridge {
            checkout_id: "idle-sse".into(),
            child: Mutex::new(Some(child)),
            credentials: ServerCredentials {
                port,
                password: "secret".into(),
                directory: directory.path().to_path_buf(),
            },
            output_reader: Mutex::new(None),
            event_socket: Mutex::new(None),
            reader: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        });
        let (reader_done_tx, reader_done_rx) = mpsc::channel();
        let reader_exit = ReaderExitSignal(Some(reader_done_tx));
        let sink: EventSink = Arc::new(move |_: AgentEvent| {
            let _ = &reader_exit;
        });
        bridge.start_reader(&sink);
        accepted_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the reader should connect to the fixture");

        let budget = Duration::from_millis(200);
        let started = Instant::now();
        bridge.stop_with_timeout(budget);
        assert!(started.elapsed() <= budget + Duration::from_millis(100));
        drop(sink);
        reader_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the SSE reader should terminate after stop");
        release_tx.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn truncated_chunked_sse_eof_returns_without_spinning() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("SSE request should connect");
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream
                    .read(&mut buffer)
                    .expect("the request should be readable");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n")
                .unwrap();
            let _ = stream.shutdown(Shutdown::Write);
        });
        let bridge = AgentBridge {
            checkout_id: "truncated-sse".into(),
            child: Mutex::new(None),
            credentials: ServerCredentials {
                port,
                password: "secret".into(),
                directory: directory.path().to_path_buf(),
            },
            output_reader: Mutex::new(None),
            event_socket: Mutex::new(None),
            reader: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        };
        let mut reader = event_stream(&bridge).expect("the truncated stream should connect");
        let started = Instant::now();
        let mut body = Vec::new();
        let result = reader.read_to_end(&mut body);

        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(result.is_ok() || result.unwrap_err().kind() == io::ErrorKind::ConnectionReset);
        assert!(body.is_empty());
        bridge.clear_event_socket();
        server.join().unwrap();
    }

    fn error_message(error: &BridgeError) -> &str {
        match error {
            BridgeError::Unavailable(message)
            | BridgeError::Foreign(message)
            | BridgeError::Failed(message) => message,
        }
    }

    #[test]
    fn a_panicking_reader_starter_does_not_poison_the_slot() {
        let starts = Arc::new(AtomicUsize::new(0));
        let readers = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let launch_starts = Arc::clone(&starts);
        let reader_starts = Arc::clone(&readers);
        let stop_calls = Arc::clone(&stops);
        let service = AgentService::with_lifecycle(
            Arc::new(move |checkout_id, _| {
                launch_starts.fetch_add(1, Ordering::SeqCst);
                Ok(fake_bridge(checkout_id))
            }),
            Arc::new(move |_, _| {
                if reader_starts.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("injected reader starter panic");
                }
            }),
            Arc::new(move |_, _| {
                stop_calls.fetch_add(1, Ordering::SeqCst);
            }),
            Duration::from_millis(400),
        );
        service.set_event_sink(Arc::new(|_| {}));

        assert!(service.bridge("checkout", Path::new(".")).is_err());
        assert!(service.bridge("checkout", Path::new(".")).is_ok());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(readers.load(Ordering::SeqCst), 2);
        service.stop("checkout");
        assert_eq!(stops.load(Ordering::SeqCst), 2);
    }

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
    fn a_port_range_full_of_leftovers_still_leaves_room_to_start() {
        const HELD: u16 = 24;
        let end = PORT_RANGE_START + PORT_RANGE_LEN;
        // u64::MAX normalizes to the last slot, so this sequence wraps immediately and
        // also verifies that offset handling cannot overflow.
        let candidates = port_candidates(u64::MAX).collect::<Vec<_>>();
        assert_eq!(candidates.len(), PORT_RANGE_LEN as usize);
        assert_eq!(candidates[0], end - 1);
        assert_eq!(candidates[1], PORT_RANGE_START);
        let mut sorted = candidates.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (PORT_RANGE_START..end).collect::<Vec<_>>());

        let occupied = &candidates[..HELD as usize];
        let mut tried = Vec::new();
        let found = first_available_port(candidates.iter().copied(), |candidate| {
            tried.push(candidate);
            !occupied.contains(&candidate)
        })
        .expect("a partly occupied window is enough to start");

        assert_eq!(found, candidates[HELD as usize]);
        assert_eq!(tried, candidates[..=HELD as usize]);

        let mut tried = Vec::new();
        let last_port = *candidates.last().expect("the port window is non-empty");
        assert_eq!(
            first_available_port(candidates.iter().copied(), |candidate| {
                tried.push(candidate);
                candidate == last_port
            }),
            Some(last_port),
            "the final candidate must still be reachable"
        );
        assert_eq!(tried, candidates);

        let mut tried = Vec::new();
        assert_eq!(
            first_available_port(candidates.iter().copied(), |candidate| {
                tried.push(candidate);
                false
            }),
            None,
            "a fully occupied window must be exhausted"
        );
        assert_eq!(tried, candidates);
        assert!(tried
            .iter()
            .all(|candidate| { *candidate >= PORT_RANGE_START && *candidate < end }));
    }

    #[test]
    fn retries_a_collision_and_leaves_only_the_final_child_running() {
        let directory = tempfile::tempdir().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let failed_pid = Arc::new(AtomicUsize::new(0));
        let fixture_cleanup = FixtureChildCleanup::new();
        let tracked_fixtures = fixture_cleanup.tracker();
        let occupied = Arc::new(Mutex::new(Vec::<u16>::new()));
        let launch_directory = directory.path().to_path_buf();
        let selected = Arc::new(Mutex::new(Vec::<u16>::new()));

        let finder_occupied = Arc::clone(&occupied);
        let mut finder_probe =
            move |candidate| !finder_occupied.lock().unwrap().contains(&candidate);
        let find_port = move || free_port_with(&mut finder_probe);

        let post_collision_occupied = Arc::clone(&occupied);
        let is_available =
            move |candidate| !post_collision_occupied.lock().unwrap().contains(&candidate);

        let launch_attempts = Arc::clone(&attempts);
        let launch_failed_pid = Arc::clone(&failed_pid);
        let launch_occupied = Arc::clone(&occupied);
        let launch_selected = Arc::clone(&selected);
        let launch = move |port| {
            let attempt = launch_attempts.fetch_add(1, Ordering::SeqCst);
            launch_selected.lock().unwrap().push(port);
            if attempt == 0 {
                // The fake availability probe passed; model a third party winning the gap
                // before this child could bind, without binding any real port.
                launch_occupied.lock().unwrap().push(port);
                let child = fixture_child(&launch_directory, "exec 1>&-; exec sleep 300");
                launch_failed_pid.store(child.id() as usize, Ordering::SeqCst);
                Ok(child)
            } else {
                Ok(fixture_child(
                    &launch_directory,
                    "printf 'server password secret\\n'; exec tail -f /dev/null",
                ))
            }
        };

        let read_attempts = Arc::new(AtomicUsize::new(0));
        let read = move |_child: &mut Child, path: &Path, port, _deadline| {
            if read_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                // stdout is closed, but the fixture remains alive so this assertion exercises
                // production termination/reaping rather than the test waiting for an exited child.
                Err(BridgeError::Unavailable(EARLY_EOF_MESSAGE.into()))
            } else {
                Ok((
                    ServerCredentials {
                        port,
                        password: "secret".into(),
                        directory: path.to_path_buf(),
                    },
                    std::thread::spawn(|| {}),
                ))
            }
        };

        let bridge = AgentBridge::start_with_deadline(
            "collision",
            directory.path(),
            Instant::now() + Duration::from_secs(2),
            PortHooks {
                find_port,
                is_available,
                track_child: move |child: &StartupChild| {
                    tracked_fixtures.lock().unwrap().push(child.clone())
                },
            },
            launch,
            read,
            |credentials, _| Ok(credentials),
        )
        .expect("the second candidate should start");

        let selected = selected.lock().unwrap().clone();
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let failed_pid = failed_pid.load(Ordering::SeqCst) as u32;
        assert_ne!(failed_pid, 0);
        assert!(
            fixture_pid_is_gone(failed_pid),
            "the failed startup fixture must be terminated and reaped"
        );
        assert_eq!(selected.len(), 2);
        assert_ne!(selected[0], selected[1]);
        assert!(selected
            .iter()
            .all(|port| *port >= PORT_RANGE_START && *port < PORT_RANGE_START + PORT_RANGE_LEN));
        assert_eq!(bridge.credentials.port, selected[1]);
        assert!(bridge
            .child
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none());
        drop(bridge);
    }

    #[test]
    fn bind_to_spawn_collisions_have_a_bounded_retry_limit() {
        let directory = tempfile::tempdir().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let failed_children_observed_exit = Arc::new(AtomicUsize::new(0));
        let occupied = Arc::new(Mutex::new(Vec::<u16>::new()));
        let launch_directory = directory.path().to_path_buf();

        let finder_occupied = Arc::clone(&occupied);
        let mut finder_probe =
            move |candidate| !finder_occupied.lock().unwrap().contains(&candidate);
        let find_port = move || free_port_with(&mut finder_probe);
        let post_collision_occupied = Arc::clone(&occupied);
        let is_available =
            move |candidate| !post_collision_occupied.lock().unwrap().contains(&candidate);

        let launch_attempts = Arc::clone(&attempts);
        let launch_occupied = Arc::clone(&occupied);
        let launch = move |port| {
            launch_attempts.fetch_add(1, Ordering::SeqCst);
            launch_occupied.lock().unwrap().push(port);
            Ok(fixture_child(
                &launch_directory,
                "printf 'server is starting\\n'",
            ))
        };

        // This counter observes fixture exit with the test's handle; the collision regression
        // above checks production cleanup independently by pid.
        let exited = Arc::clone(&failed_children_observed_exit);
        let read = move |child: &mut Child, path: &Path, port, deadline| {
            let result = read_credentials(child, path, port, deadline);
            if result.is_err() && child.try_wait().unwrap().is_some() {
                exited.fetch_add(1, Ordering::SeqCst);
            }
            result
        };

        let error = AgentBridge::start_with_deadline(
            "collision-limit",
            directory.path(),
            Instant::now() + Duration::from_secs(2),
            PortHooks {
                find_port,
                is_available,
                track_child: no_startup_child_tracking,
            },
            launch,
            read,
            |credentials, _| Ok(credentials),
        )
        .err()
        .expect("the collision retry limit must be enforced");

        assert!(error_message(&error).contains("another process took the selected agent port"));
        assert!(error_message(&error).contains("exhausted 3 startup attempts"));
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_START_ATTEMPTS);
        assert_eq!(
            failed_children_observed_exit.load(Ordering::SeqCst),
            MAX_START_ATTEMPTS
        );
    }

    #[test]
    fn an_eof_with_a_free_port_is_not_retried() {
        let directory = tempfile::tempdir().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let fixture_cleanup = FixtureChildCleanup::new();
        let tracked_fixtures = fixture_cleanup.tracker();
        let launch_directory = directory.path().to_path_buf();
        let mut finder_probe = |_| true;
        let find_port = move || free_port_with(&mut finder_probe);

        let launch_attempts = Arc::clone(&attempts);
        let launch = move |_| {
            launch_attempts.fetch_add(1, Ordering::SeqCst);
            Ok(fixture_child(&launch_directory, "exec tail -f /dev/null"))
        };
        let read = |_child: &mut Child, _path: &Path, _port, _deadline| {
            Err(BridgeError::Unavailable(EARLY_EOF_MESSAGE.into()))
        };

        let error = AgentBridge::start_with_deadline(
            "no-collision",
            directory.path(),
            Instant::now() + Duration::from_secs(1),
            PortHooks {
                find_port,
                is_available: |_| true,
                track_child: move |child: &StartupChild| {
                    tracked_fixtures.lock().unwrap().push(child.clone())
                },
            },
            launch,
            read,
            |credentials, _| Ok(credentials),
        )
        .err()
        .expect("an EOF with an available port must fail immediately");

        assert!(error_message(&error).contains("EOF"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_non_eof_read_error_with_an_occupied_port_is_not_retried() {
        let directory = tempfile::tempdir().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let fixture_cleanup = FixtureChildCleanup::new();
        let tracked_fixtures = fixture_cleanup.tracker();
        let launch_directory = directory.path().to_path_buf();
        let mut finder_probe = |_| true;
        let find_port = move || free_port_with(&mut finder_probe);

        let launch_attempts = Arc::clone(&attempts);
        let launch = move |_| {
            launch_attempts.fetch_add(1, Ordering::SeqCst);
            Ok(fixture_child(&launch_directory, "exec tail -f /dev/null"))
        };
        let read = |_child: &mut Child, _path: &Path, _port, _deadline| {
            Err(BridgeError::Unavailable(
                "could not read the agent server output: injected error".into(),
            ))
        };

        let error = AgentBridge::start_with_deadline(
            "non-eof-error",
            directory.path(),
            Instant::now() + Duration::from_secs(1),
            PortHooks {
                find_port,
                is_available: |_| false,
                track_child: move |child: &StartupChild| {
                    tracked_fixtures.lock().unwrap().push(child.clone())
                },
            },
            launch,
            read,
            |credentials, _| Ok(credentials),
        )
        .err()
        .expect("a non-EOF read error must fail immediately");

        assert!(error_message(&error).contains("could not read the agent server output"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rejected_credentials_are_not_retried() {
        let directory = tempfile::tempdir().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let fixture_cleanup = FixtureChildCleanup::new();
        let tracked_fixtures = fixture_cleanup.tracker();
        let launch_directory = directory.path().to_path_buf();
        let mut finder_probe = |_| true;
        let find_port = move || free_port_with(&mut finder_probe);

        let launch_attempts = Arc::clone(&attempts);
        let launch = move |_| {
            launch_attempts.fetch_add(1, Ordering::SeqCst);
            Ok(fixture_child(&launch_directory, "exec tail -f /dev/null"))
        };
        let read = move |_child: &mut Child, path: &Path, port, _deadline| {
            Ok((
                ServerCredentials {
                    port,
                    password: "secret".into(),
                    directory: path.to_path_buf(),
                },
                std::thread::spawn(|| {}),
            ))
        };

        let error = AgentBridge::start_with_deadline(
            "rejected",
            directory.path(),
            Instant::now() + Duration::from_secs(1),
            PortHooks {
                find_port,
                is_available: |_| false,
                track_child: move |child: &StartupChild| {
                    tracked_fixtures.lock().unwrap().push(child.clone())
                },
            },
            launch,
            read,
            |_, _| Err(BridgeError::Failed(status_detail(401))),
        )
        .err()
        .expect("rejected credentials must fail without a port retry");

        assert!(error_message(&error).contains("rejected Marvis's credentials"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn collision_retries_do_not_reset_the_startup_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let fixture_cleanup = FixtureChildCleanup::new();
        let tracked_fixtures = fixture_cleanup.tracker();
        let launch_directory = directory.path().to_path_buf();
        let mut finder_probe = |_| true;
        let find_port = move || free_port_with(&mut finder_probe);

        let launch_attempts = Arc::clone(&attempts);
        let launch = move |_| {
            launch_attempts.fetch_add(1, Ordering::SeqCst);
            Ok(fixture_child(&launch_directory, "exec tail -f /dev/null"))
        };
        let read = |_child: &mut Child, _path: &Path, _port, _deadline| {
            Err(BridgeError::Unavailable(EARLY_EOF_MESSAGE.into()))
        };

        let error = AgentBridge::start_with_deadline(
            "expired",
            directory.path(),
            Instant::now(),
            PortHooks {
                find_port,
                is_available: |_| false,
                track_child: move |child: &StartupChild| {
                    tracked_fixtures.lock().unwrap().push(child.clone())
                },
            },
            launch,
            read,
            |credentials, _| Ok(credentials),
        )
        .err()
        .expect("an exhausted shared deadline must report timeout");

        assert!(error_message(&error).contains("timed out while starting"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
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

    #[test]
    fn reports_rejected_credentials_distinctly() {
        assert!(status_detail(401).contains("rejected Marvis's credentials"));
    }

    /// Proves the round marker really reaches the session, which is what makes the
    /// reconnect path safe.
    ///
    /// `cargo test a_round_marker_reaches_the_real_session -- --ignored --nocapture`
    #[ignore = "live OpenCode integration; requires a configured provider and sends a prompt"]
    #[test]
    fn a_round_marker_reaches_the_real_session() {
        let directory = required_live_directory("MARVIS_AGENT_BRIDGE_DIR");
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
    /// `cargo test a_turn_is_observable_through_the_idle_time -- --ignored --nocapture`
    #[ignore = "live OpenCode integration; requires a configured provider and sends a prompt"]
    #[test]
    fn a_turn_is_observable_through_the_idle_time() {
        let directory = required_live_directory("MARVIS_AGENT_BRIDGE_DIR");
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

    /// Proves that the fields Marvis renders a row from are really on the wire.
    ///
    /// The agent name, the model, the parent of a subagent and the color OpenCode paints an
    /// agent with are all read off routes that exist in 2.0.18, and this is the only thing that
    /// says they still are.
    ///
    /// `cargo test the_agent_catalog_is_readable_from_a_real_server -- --ignored --nocapture`
    #[ignore = "live OpenCode integration; requires the local OpenCode server and creates a session"]
    #[test]
    fn the_agent_catalog_is_readable_from_a_real_server() {
        let directory = required_live_directory("MARVIS_AGENT_BRIDGE_DIR");
        let agents = AgentService::new();
        let agents_for_catalog = agents
            .agents("checkout:catalog", &directory)
            .expect("read the agent catalog");

        // A server with nothing configured still offers its built-in agents, which is what
        // makes a fallback for the missing color reachable at all.
        assert!(
            !agents_for_catalog.is_empty(),
            "the catalog came back empty"
        );
        for agent in &agents_for_catalog {
            assert!(!agent.id.is_empty(), "an agent with no id: {agent:?}");
            assert!(
                ["primary", "subagent", "all", ""].contains(&agent.mode.as_str()),
                "an agent with an unknown mode: {agent:?}"
            );
            // A color, where there is one, is something a stylesheet can take.
            if let Some(color) = &agent.color {
                assert!(
                    color.starts_with('#') && color.len() > 3,
                    "a color that is not a hex: {agent:?}"
                );
            }
        }

        // The session carries the agent and the model the row paints.
        let created = agents
            .create_session("checkout:catalog", &directory, "catalog probe")
            .expect("create a session on the real server");
        let resolved = agents
            .owned_session("checkout:catalog", &directory, &created.id)
            .expect("read the session back");
        assert_eq!(resolved.id, created.id);
        // A session nobody has prompted has run no turn, so both are absent: that absence is
        // what the row has to render as "nothing to say" rather than as a value.
        assert_eq!(resolved.idle_at, None);
        assert!(
            resolved.outcome.is_none() || resolved.outcome.as_deref() == Some("succeeded"),
            "a session that never ran reported an outcome: {resolved:?}"
        );
        agents.stop("checkout:catalog");
    }

    /// Talks to a real `opencode serve`; it is ignored by default because it needs the
    /// OpenCode binary and a working provider.
    ///
    /// `cargo test bridge_talks_to_a_real_server -- --ignored --nocapture`
    #[ignore = "live OpenCode integration; requires a configured provider and sends a prompt"]
    #[test]
    fn bridge_talks_to_a_real_server() {
        let first = required_live_directory("MARVIS_AGENT_BRIDGE_DIR");
        let second = required_live_directory("MARVIS_AGENT_BRIDGE_OTHER_DIR");
        let agents = AgentService::new();

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
    /// `cargo test two_checkouts_run_the_review_loop -- --ignored --nocapture`
    #[ignore = "live OpenCode integration; requires a configured provider, sends prompts, and edits files"]
    #[test]
    fn two_checkouts_run_the_review_loop() {
        let first = required_live_directory("MARVIS_AGENT_BRIDGE_DIR");
        let second = required_live_directory("MARVIS_AGENT_BRIDGE_OTHER_DIR");
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
