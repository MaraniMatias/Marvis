//! A minimal OpenCode bridge: a client of the service the person running OpenCode started.
//!
//! Everything Marvis needs from the agent is scoped to the checkout that owns it, so every
//! request names that checkout's directory and every response is checked against the expected
//! path before it is trusted. A `ses_…` id from one checkout is meaningless in another's
//! directory, and `agent_session` is what enforces that.
//!
//! The service is the user's, not this app's. Nothing here starts one and nothing here ends one:
//! `services::opencode` finds the service that is already running, and the absence of one leaves
//! the app disconnected rather than leaving a server behind.

use std::{
    collections::{HashMap, HashSet},
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc, Condvar, Mutex},
    thread::sleep,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    domain::{
        agent::{agent_event_kind, AgentAgent, AgentSession},
        ipc::{IpcError, IpcErrorCode},
    },
    services::opencode::{self, ServiceEndpoint},
};

pub use crate::domain::agent::AgentEvent;

/// The HTTP requests a bridge makes, which a service that is busy answering a turn can delay.
const JSON_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a freshly found service is given to answer for this checkout before the app says it
/// is not there yet.
const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(5);
// `stopAgent` is awaited by the UI.
const STARTUP_STOP_WAIT: Duration = Duration::from_secs(3);
// A waiter has to cover the launcher's wall time: its discovery budget plus the 3s reader cleanup.
// It is only an inert caller-wait bound: expiry does not cancel a healthy connect, which does not
// hold the slot open.
const STARTUP_WAIT_TIMEOUT: Duration = Duration::from_secs(9);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(30);
/// The service is a loopback process guarded by its own password, so basic auth is the whole
/// trust boundary. See `sec_09`.
const MAX_PROMPT_BYTES: usize = 512 * 1024;
// Bound both each response header line and the aggregate (status + headers).
const MAX_EVENT_HEADERS_BYTES: usize = 32 * 1024;
const MAX_EVENT_HEADER_LINE_BYTES: usize = 8 * 1024;
// SSE event payloads can include long transcript text; bound all wire bytes per event.
const MAX_SSE_FRAME_BYTES: usize = 1024 * 1024;
const MAX_SSE_FRAME_LINES: usize = 16 * 1024;

/// The service a bridge talks to, and the directory it is scoped to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerCredentials {
    endpoint: ServiceEndpoint,
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
    credentials: ServerCredentials,
    /// A clone of the event socket, used to interrupt a blocking read during stop.
    event_socket: Mutex<Option<TcpStream>>,
    /// Set once the service is reached, so the event reader can be told to stop.
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The reader loop reconnects forever, so it needs its own exit condition.
    stopped: std::sync::atomic::AtomicBool,
}

/// Where normalized events go. Injected so the service does not depend on Tauri.
pub type EventSink = Arc<dyn Fn(AgentEvent) + Send + Sync>;

impl AgentBridge {
    /// Reaches the service the user started, if there is one.
    ///
    /// Discovery is the whole of startup: there is no port to pick and no process to wait for, so
    /// this either has a service to talk to or reports that there is not one yet.
    fn connect(home: &Path, checkout_id: &str, directory: &Path) -> Result<Self, BridgeError> {
        let endpoint = opencode::discover(home).map_err(BridgeError::Unavailable)?;
        let credentials = ServerCredentials {
            endpoint,
            directory: directory.to_path_buf(),
        };
        let credentials = ready(credentials, Instant::now() + SERVER_READY_TIMEOUT)?;
        Ok(Self {
            checkout_id: checkout_id.to_string(),
            credentials,
            event_socket: Mutex::new(None),
            reader: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn base_url(&self) -> String {
        self.credentials.endpoint.url.clone()
    }

    fn auth_header(&self) -> String {
        opencode::authorization(&self.credentials.endpoint.password)
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

    pub(crate) fn directory(&self) -> &Path {
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
        self.clear_event_socket();
        if let Some(handle) = self.reader.lock().ok().and_then(|mut slot| slot.take()) {
            join_reader(handle, deadline);
        }
    }

    /// Stops this bridge without waiting: the reader is told to end and the socket it is blocked
    /// on is closed, which is all that ending this app's half of the connection takes. The service
    /// on the other side is the user's and keeps running.
    fn signal_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.clear_event_socket();
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

/// Reads at most limit wire bytes; LF/CRLF terminators count, while an EOF-terminated line
/// may use the full limit because EOF adds no byte.
fn read_bounded_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    limit: usize,
) -> io::Result<usize> {
    line.clear();
    loop {
        if line.len() == limit {
            if reader.fill_buf()?.is_empty() {
                return Ok(line.len());
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "line exceeds configured limit",
            ));
        }
        let (read, complete, over_limit) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(line.len());
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let read = newline.map_or(available.len(), |index| index + 1);
            let remaining = limit - line.len();
            if read > remaining {
                line.extend_from_slice(&available[..remaining]);
                (remaining, false, true)
            } else {
                line.extend_from_slice(&available[..read]);
                (read, newline.is_some(), false)
            }
        };
        reader.consume(read);
        if over_limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "line exceeds configured limit",
            ));
        }
        if complete {
            return Ok(line.len());
        }
    }
}

fn trim_line_ending(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn read_http_header_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    remaining: &mut usize,
) -> io::Result<()> {
    let length = read_bounded_line(reader, line, (*remaining).min(MAX_EVENT_HEADER_LINE_BYTES))?;
    if length == 0 || line.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete SSE response headers",
        ));
    }
    *remaining -= length;
    Ok(())
}

fn read_sse_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut data = Vec::new();
    let mut line = Vec::new();
    let mut frame_bytes = 0;
    let mut frame_lines = 0;
    let mut data_seen = false;
    loop {
        let length = read_bounded_line(
            reader,
            &mut line,
            MAX_SSE_FRAME_BYTES.saturating_sub(frame_bytes),
        )?;
        if length == 0 {
            if data_seen {
                data.pop();
                return Ok(Some(data));
            }
            return Ok(None);
        }
        frame_bytes += length;
        frame_lines += 1;
        if frame_lines > MAX_SSE_FRAME_LINES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SSE frame has too many lines",
            ));
        }
        let text = std::str::from_utf8(trim_line_ending(&line)).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 in SSE stream")
        })?;
        if text.is_empty() {
            if data_seen {
                data.pop();
                return Ok(Some(data));
            }
            frame_bytes = 0;
            frame_lines = 0;
            continue;
        }
        if text.starts_with(':') {
            continue;
        }
        let (field, value) = text.split_once(':').unwrap_or((text, ""));
        if field == "data" {
            let value = value.strip_prefix(' ').unwrap_or(value);
            data.extend_from_slice(value.as_bytes());
            data.push(b'\n');
            data_seen = true;
        }
    }
}

fn event_from_payload(payload: &[u8], bridge: &AgentBridge) -> io::Result<Option<AgentEvent>> {
    let frame = serde_json::from_slice::<SseFrame>(payload)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE event"))?;
    if let Some(location) = &frame.location {
        if !same_directory(Path::new(&location.directory), bridge.directory()) {
            return Ok(None);
        }
    }
    let data = frame.data;
    let session_id = data
        .get("sessionID")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let raw_type = frame.raw_type;
    Ok(Some(AgentEvent {
        checkout_id: bridge.checkout_id.clone(),
        session_id,
        kind: agent_event_kind(&raw_type),
        raw_type,
        data,
    }))
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
            let mut size_line = Vec::new();
            let length = read_bounded_line(
                &mut self.reader,
                &mut size_line,
                MAX_EVENT_HEADER_LINE_BYTES,
            )?;
            if length == 0 || size_line.last() != Some(&b'\n') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "incomplete SSE chunk header",
                ));
            }
            let size_text = std::str::from_utf8(trim_line_ending(&size_line))
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE chunk"))?;
            let size = size_text
                .split(';')
                .next()
                .and_then(|value| usize::from_str_radix(value.trim(), 16).ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE chunk"))?;
            if size == 0 {
                let mut trailer_budget = MAX_EVENT_HEADERS_BYTES;
                let mut trailer = Vec::new();
                loop {
                    let length = read_bounded_line(
                        &mut self.reader,
                        &mut trailer,
                        trailer_budget.min(MAX_EVENT_HEADER_LINE_BYTES),
                    )?;
                    if length == 0 {
                        self.finished = true;
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "incomplete SSE chunk trailers",
                        ));
                    }
                    trailer_budget -= length;
                    if trailer == b"\r\n" || trailer == b"\n" {
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
            return if self.chunked {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated SSE chunk",
                ))
            } else {
                Ok(0)
            };
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
    let port = bridge.credentials.endpoint.port;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))?;
    let interrupt_socket = stream.try_clone()?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET /api/event HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
        bridge.auth_header()
    )?;
    if !bridge.install_event_socket(interrupt_socket) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "the event stream was stopped",
        ));
    }

    let mut reader = BufReader::new(stream);
    let mut header_budget = MAX_EVENT_HEADERS_BYTES;
    let mut line = Vec::new();
    read_http_header_line(&mut reader, &mut line, &mut header_budget)?;
    let status_line = std::str::from_utf8(trim_line_ending(&line))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE response"))?;
    let status = status_line
        .split_ascii_whitespace()
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
    loop {
        read_http_header_line(&mut reader, &mut line, &mut header_budget)?;
        let header = std::str::from_utf8(trim_line_ending(&line))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE response"))?;
        if header.is_empty() {
            break;
        }
        let (name, value) = header
            .split_once(':')
            .filter(|(name, _)| !name.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid SSE response"))?;
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|value| value.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
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
        while !bridge.is_stopped() {
            let payload = match read_sse_frame(&mut reader) {
                Ok(Some(payload)) => payload,
                Ok(None) | Err(_) => break,
            };
            match event_from_payload(&payload, &bridge) {
                Ok(Some(event)) => sink(event),
                Ok(None) => continue,
                Err(_) => break,
            }
        }
        drop(reader);
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
        sleep(Duration::from_millis(10).min(remaining));
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

/// Waits until the discovered service answers for this checkout, before the bridge is handed out.
///
/// Ready means "the agent catalog has something in it", not merely "a route answered":
/// `/api/agent` is empty for the first moments after the service starts, while it is still loading
/// its configuration. A bridge that returned from that window would tell every later reader that
/// the project has no agents at all, and the colors the sidebar paints come from that list.
fn ready(
    credentials: ServerCredentials,
    deadline: Instant,
) -> Result<ServerCredentials, BridgeError> {
    let probe = AgentBridge {
        checkout_id: String::new(),
        credentials: credentials.clone(),
        event_socket: Mutex::new(None),
        reader: Mutex::new(None),
        stopped: std::sync::atomic::AtomicBool::new(false),
    };
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(BridgeError::Unavailable(
                "the OpenCode service did not become ready in time".into(),
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackedTurn {
    Pending,
    Started,
}

#[derive(Default)]
struct RemovalState {
    active: HashSet<String>,
    generations: HashMap<String, u64>,
}

/// Keep delivery serialized with epoch changes so an old event cannot cross a reopen.
fn generation_scoped_sink(
    state: Arc<Mutex<RemovalState>>,
    checkout_id: String,
    generation: u64,
    sink: EventSink,
) -> EventSink {
    Arc::new(move |event| {
        if let Ok(state) = state.lock() {
            let current = !state.active.contains(&checkout_id)
                && state
                    .generations
                    .get(&checkout_id)
                    .copied()
                    .unwrap_or_default()
                    == generation;
            if current {
                sink(event);
            }
        }
    })
}

pub(crate) struct WorktreeRemovalGuard {
    state: Arc<Mutex<RemovalState>>,
    checkout_id: String,
    committed: bool,
}

impl WorktreeRemovalGuard {
    /// Invalidate queued requests atomically before releasing the removal reservation.
    pub(crate) fn commit(mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.active.remove(&self.checkout_id);
            let generation = state
                .generations
                .entry(self.checkout_id.clone())
                .or_default();
            *generation = generation.wrapping_add(1);
        }
        self.committed = true;
    }
}

impl Drop for WorktreeRemovalGuard {
    fn drop(&mut self) {
        if !self.committed {
            if let Ok(mut state) = self.state.lock() {
                state.active.remove(&self.checkout_id);
            }
        }
    }
}

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
    checkout_operations: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    removal_state: Arc<Mutex<RemovalState>>,
    busy_turns: Arc<Mutex<HashMap<String, HashMap<String, TrackedTurn>>>>,
    sink: Mutex<Option<EventSink>>,
    launcher: Arc<BridgeLauncher>,
    reader_starter: Arc<ReaderStarter>,
    stopper: Arc<BridgeStopper>,
    slot_stopper: Arc<SlotStopper>,
    startup_stop_wait: Duration,
    startup_wait: Duration,
}

impl AgentService {
    /// Finds the service the user runs. `home` is where that service registers itself, and it is
    /// kept for the life of the app so every later connect looks in the same place.
    pub fn new(home: PathBuf) -> Self {
        let mut service = Self::with_lifecycle(
            Arc::new(move |checkout_id, directory| {
                AgentBridge::connect(&home, checkout_id, directory).map(Arc::new)
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

    #[cfg(test)]
    pub(crate) fn with_test_server(port: u16) -> Self {
        Self::with_lifecycle(
            Arc::new(move |checkout_id, directory| {
                Ok(Arc::new(AgentBridge {
                    checkout_id: checkout_id.to_string(),
                    credentials: ServerCredentials {
                        endpoint: ServiceEndpoint {
                            url: format!("http://127.0.0.1:{port}"),
                            port,
                            password: "test".into(),
                        },
                        directory: directory.to_path_buf(),
                    },
                    event_socket: Mutex::new(None),
                    reader: Mutex::new(None),
                    stopped: std::sync::atomic::AtomicBool::new(false),
                }))
            }),
            Arc::new(|_, _| {}),
            Arc::new(|bridge, timeout| bridge.stop_with_timeout(timeout)),
            Duration::from_millis(50),
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
            checkout_operations: Mutex::new(HashMap::new()),
            removal_state: Arc::new(Mutex::new(RemovalState::default())),
            busy_turns: Arc::new(Mutex::new(HashMap::new())),
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
        let forward = sink;
        let busy_turns = Arc::clone(&self.busy_turns);
        let sink: EventSink = Arc::new(move |event| {
            if let Some(session_id) = event.session_id.as_deref() {
                if let Ok(mut turns) = busy_turns.lock() {
                    let checkout = turns.entry(event.checkout_id.clone()).or_default();
                    match event.kind {
                        crate::domain::agent::AgentEventKind::TurnStarted
                        | crate::domain::agent::AgentEventKind::TurnFinished => {
                            checkout.insert(session_id.to_string(), TrackedTurn::Started);
                        }
                        crate::domain::agent::AgentEventKind::TurnFailed
                        | crate::domain::agent::AgentEventKind::PermissionAsked => {
                            checkout.remove(session_id);
                        }
                        _ => {}
                    }
                    if checkout.is_empty() {
                        turns.remove(&event.checkout_id);
                    }
                }
            }
            forward(event);
        });
        if let Ok(mut slot) = self.sink.lock() {
            *slot = Some(sink);
        }
    }

    /// A service that finds no OpenCode, for the tests that need a holder for generations and
    /// removal state without talking to anything.
    #[cfg(test)]
    pub(crate) fn without_service() -> Self {
        Self::new(PathBuf::from("/marvis-no-opencode-service"))
    }

    fn checkout_operation_lock(&self, checkout_id: &str) -> Arc<Mutex<()>> {
        let mut operations = self
            .checkout_operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Arc::clone(
            operations
                .entry(checkout_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    pub(crate) fn with_checkout_operation<T>(
        &self,
        checkout_id: &str,
        operation: impl FnOnce() -> T,
    ) -> T {
        let lock = self.checkout_operation_lock(checkout_id);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        operation()
    }

    fn ensure_checkout_not_removing(&self, checkout_id: &str) -> Result<(), BridgeError> {
        let state = self
            .removal_state
            .lock()
            .map_err(|_| BridgeError::Failed("the agent removal state is poisoned".into()))?;
        if state.active.contains(checkout_id) {
            return Err(BridgeError::Unavailable(
                "this checkout is being removed".into(),
            ));
        }
        Ok(())
    }

    /// Snapshot before command path lookup / async dispatch; revalidate after taking its lock.
    pub(crate) fn checkout_generation(&self, checkout_id: &str) -> Result<u64, BridgeError> {
        let state = self
            .removal_state
            .lock()
            .map_err(|_| BridgeError::Failed("the agent removal state is poisoned".into()))?;
        if state.active.contains(checkout_id) {
            return Err(BridgeError::Unavailable(
                "this checkout is being removed".into(),
            ));
        }
        Ok(state
            .generations
            .get(checkout_id)
            .copied()
            .unwrap_or_default())
    }

    /// Read the current epoch even while a removal reservation is held by its owner.
    pub(crate) fn checkout_epoch(&self, checkout_id: &str) -> Result<u64, BridgeError> {
        let state = self
            .removal_state
            .lock()
            .map_err(|_| BridgeError::Failed("the agent removal state is poisoned".into()))?;
        Ok(state
            .generations
            .get(checkout_id)
            .copied()
            .unwrap_or_default())
    }

    /// Revalidate the request epoch and current DB registration under the checkout operation lock.
    pub(crate) fn with_checkout_generation<T>(
        &self,
        checkout_id: &str,
        generation: u64,
        is_registered: impl Fn() -> bool,
        operation: impl FnOnce() -> Result<T, BridgeError>,
    ) -> Result<T, BridgeError> {
        self.with_checkout_operation(checkout_id, || {
            if self.checkout_generation(checkout_id)? != generation {
                return Err(BridgeError::Unavailable(
                    "the checkout changed while the agent request was queued; retry it".into(),
                ));
            }
            if !is_registered() {
                return Err(BridgeError::Unavailable(
                    "this checkout is no longer registered".into(),
                ));
            }
            operation()
        })
    }

    pub(crate) fn reserve_worktree_removal(
        &self,
        checkout_id: &str,
    ) -> Result<WorktreeRemovalGuard, BridgeError> {
        {
            let mut state = self
                .removal_state
                .lock()
                .map_err(|_| BridgeError::Failed("the agent removal state is poisoned".into()))?;
            if !state.active.insert(checkout_id.to_string()) {
                return Err(BridgeError::Unavailable(
                    "this checkout is already being removed".into(),
                ));
            }
        }
        let removal = WorktreeRemovalGuard {
            state: Arc::clone(&self.removal_state),
            checkout_id: checkout_id.to_string(),
            committed: false,
        };
        let slot = self
            .bridges
            .lock()
            .map_err(|_| BridgeError::Failed("the agent service is poisoned".into()))?
            .get(checkout_id)
            .cloned();
        if let Some(slot) = slot {
            let state = slot
                .state
                .lock()
                .map_err(|_| BridgeError::Failed("the agent bridge state is poisoned".into()))?;
            if matches!(
                &*state,
                BridgeState::Starting { .. } | BridgeState::Stopping | BridgeState::Stopped
            ) {
                return Err(BridgeError::Unavailable(
                    "the agent bridge is starting or stopping; stop it and retry worktree removal"
                        .into(),
                ));
            }
        }
        Ok(removal)
    }

    /// Busy sessions observed by the real checkout bridge; does not start a bridge.
    pub(crate) fn active_worktree_agent_sessions(
        &self,
        checkout_id: &str,
    ) -> Result<Vec<AgentSession>, BridgeError> {
        let slot = self
            .bridges
            .lock()
            .map_err(|_| BridgeError::Failed("the agent service is poisoned".into()))?
            .get(checkout_id)
            .cloned();
        let Some(slot) = slot else {
            return Ok(Vec::new());
        };
        let bridge = {
            let state = slot
                .state
                .lock()
                .map_err(|_| BridgeError::Failed("the agent bridge state is poisoned".into()))?;
            match &*state {
                BridgeState::Ready(bridge) => Arc::clone(bridge),
                BridgeState::Failed(_) => return Ok(Vec::new()),
                BridgeState::Starting { .. } | BridgeState::Stopping | BridgeState::Stopped => {
                    return Err(BridgeError::Unavailable(
                        "the agent bridge is starting or stopping; retry worktree removal".into(),
                    ));
                }
            }
        };
        let listed: Vec<ApiSession> =
            bridge.get_json_with_timeout("/api/session", Duration::from_secs(5))?;
        let turns = self
            .busy_turns
            .lock()
            .map_err(|_| BridgeError::Failed("the agent activity state is poisoned".into()))?;
        let mut tracked = turns.get(checkout_id).cloned().unwrap_or_default();
        drop(turns);
        let mut seen = HashSet::new();
        let mut idle = HashSet::new();
        let mut active = Vec::new();
        for raw in listed {
            if raw.check_scope(bridge.directory()).is_err() {
                continue;
            }
            seen.insert(raw.id.clone());
            match tracked.get(&raw.id) {
                Some(TrackedTurn::Pending) => active.push(bridge.to_agent_session(&raw)),
                Some(TrackedTurn::Started) if raw.time.idle.is_none() => {
                    active.push(bridge.to_agent_session(&raw));
                }
                Some(TrackedTurn::Started) => {
                    idle.insert(raw.id.clone());
                }
                None => {}
            }
        }
        for (session_id, state) in &tracked {
            if *state == TrackedTurn::Pending && !seen.contains(session_id) {
                active.push(AgentSession {
                    id: session_id.clone(),
                    checkout_id: checkout_id.to_string(),
                    title: "OpenCode prompt status is unknown".into(),
                    idle_at: None,
                    blocked_on_permission: false,
                    agent: None,
                    model: None,
                    parent_id: None,
                    outcome: None,
                    created_at: 0,
                    updated_at: 0,
                });
            }
        }
        tracked.retain(|session_id, state| match state {
            TrackedTurn::Pending => true,
            TrackedTurn::Started => seen.contains(session_id) && !idle.contains(session_id),
        });
        if let Ok(mut turns) = self.busy_turns.lock() {
            if tracked.is_empty() {
                turns.remove(checkout_id);
            } else {
                turns.insert(checkout_id.to_string(), tracked);
            }
        }
        Ok(active)
    }

    pub(crate) fn stop_for_worktree_removal(&self, checkout_id: &str) {
        self.stop_locked(checkout_id);
    }

    /// Returns the checkout's bridge, starting its server on first use.
    pub fn bridge(
        &self,
        checkout_id: &str,
        directory: &Path,
    ) -> Result<Arc<AgentBridge>, BridgeError> {
        // Reserve this checkout under the global lock, then boot and create its reader only
        // after releasing it. Stops likewise remove slots before killing or joining anything.
        let removal_state = self
            .removal_state
            .lock()
            .map_err(|_| BridgeError::Failed("the agent removal state is poisoned".into()))?;
        if removal_state.active.contains(checkout_id) {
            return Err(BridgeError::Unavailable(
                "this checkout is being removed".into(),
            ));
        }
        let generation = removal_state
            .generations
            .get(checkout_id)
            .copied()
            .unwrap_or_default();
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
        drop(removal_state);

        if !is_starter {
            return slot.wait_with_timeout(self.startup_wait);
        }

        let sink = self
            .sink
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(Arc::clone))
            .map(|sink| {
                generation_scoped_sink(
                    Arc::clone(&self.removal_state),
                    checkout_id.to_string(),
                    generation,
                    sink,
                )
            });
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
        self.with_checkout_operation(checkout_id, || {
            self.create_session_locked(checkout_id, directory, title)
        })
    }

    pub(crate) fn create_session_at_generation(
        &self,
        checkout_id: &str,
        directory: &Path,
        title: &str,
        generation: u64,
        is_registered: impl Fn() -> bool,
    ) -> Result<AgentSession, BridgeError> {
        self.with_checkout_generation(checkout_id, generation, is_registered, || {
            self.create_session_locked(checkout_id, directory, title)
        })
    }

    fn create_session_locked(
        &self,
        checkout_id: &str,
        directory: &Path,
        title: &str,
    ) -> Result<AgentSession, BridgeError> {
        self.ensure_checkout_not_removing(checkout_id)?;
        let bridge = self.bridge(checkout_id, directory)?;
        if bridge.is_stopped() {
            return Err(BridgeError::Unavailable(
                "the agent bridge was stopped".into(),
            ));
        }
        let created: ApiSession =
            bridge.post_json("/api/session", &serde_json::json!({ "title": title }))?;
        created.check_scope(directory)?;
        Ok(bridge.to_agent_session(&created))
    }

    /// Sends the review as one message. v2.0.18 wants {"text": …} on this route.
    pub fn prompt(
        &self,
        checkout_id: &str,
        directory: &Path,
        session_id: &str,
        text: &str,
    ) -> Result<AgentSession, BridgeError> {
        let text = Self::validate_prompt(text)?;
        self.with_checkout_operation(checkout_id, || {
            self.prompt_locked(checkout_id, directory, session_id, text)
        })
    }

    pub(crate) fn prompt_at_generation(
        &self,
        checkout_id: &str,
        directory: &Path,
        session_id: &str,
        text: &str,
        generation: u64,
        is_registered: impl Fn() -> bool,
    ) -> Result<AgentSession, BridgeError> {
        let text = Self::validate_prompt(text)?;
        self.with_checkout_generation(checkout_id, generation, is_registered, || {
            self.prompt_locked(checkout_id, directory, session_id, text)
        })
    }

    fn validate_prompt(text: &str) -> Result<&str, BridgeError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(BridgeError::Failed("the review is empty".into()));
        }
        if text.len() > MAX_PROMPT_BYTES {
            return Err(BridgeError::Failed(
                "the review is too large to send to the agent in one message".into(),
            ));
        }
        Ok(text)
    }

    fn prompt_locked(
        &self,
        checkout_id: &str,
        directory: &Path,
        session_id: &str,
        text: &str,
    ) -> Result<AgentSession, BridgeError> {
        self.ensure_checkout_not_removing(checkout_id)?;
        let bridge = self.bridge(checkout_id, directory)?;
        if bridge.is_stopped() {
            return Err(BridgeError::Unavailable(
                "the agent bridge was stopped".into(),
            ));
        }
        // Resolve first so a foreign id fails before anything is sent.
        bridge.session(session_id)?;
        self.track_prompt_pending(checkout_id, session_id);
        // A transport error can arrive after OpenCode accepted the prompt, so retain the
        // pending activity marker until a turn ends or the bridge is explicitly stopped.
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

    pub(crate) fn session_mentions_at_generation(
        &self,
        checkout_id: &str,
        directory: &Path,
        session_id: &str,
        marker: &str,
        generation: u64,
        is_registered: impl Fn() -> bool,
    ) -> Result<bool, BridgeError> {
        self.with_checkout_generation(checkout_id, generation, is_registered, || {
            self.session_mentions(checkout_id, directory, session_id, marker)
        })
    }

    /// Stops the checkout's server. Called when the app shuts down.
    pub fn stop(&self, checkout_id: &str) {
        self.with_checkout_operation(checkout_id, || self.stop_locked(checkout_id));
    }

    fn stop_locked(&self, checkout_id: &str) {
        if let Ok(mut state) = self.removal_state.lock() {
            let generation = state
                .generations
                .entry(checkout_id.to_string())
                .or_default();
            *generation = generation.wrapping_add(1);
        }
        let slot = self
            .bridges
            .lock()
            .ok()
            .and_then(|mut bridges| bridges.remove(checkout_id));
        if let Some(slot) = slot {
            slot.stop(self.stopper.as_ref(), self.startup_stop_wait);
        }
        if let Ok(mut turns) = self.busy_turns.lock() {
            turns.remove(checkout_id);
        }
    }

    pub(crate) fn stop_at_generation(
        &self,
        checkout_id: &str,
        generation: u64,
        is_registered: impl Fn() -> bool,
    ) -> Result<(), BridgeError> {
        self.with_checkout_generation(checkout_id, generation, is_registered, || {
            self.stop_locked(checkout_id);
            Ok(())
        })
    }

    fn track_prompt_pending(&self, checkout_id: &str, session_id: &str) {
        if let Ok(mut turns) = self.busy_turns.lock() {
            turns
                .entry(checkout_id.to_string())
                .or_default()
                .insert(session_id.to_string(), TrackedTurn::Pending);
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
        io::{self, BufRead, BufReader, Cursor, Read, Write},
        net::{Shutdown, TcpListener},
        path::{Path, PathBuf},
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
        event_from_payload, event_stream, generation_scoped_sink, join_reader, read_bounded_line,
        read_sse_frame, ready, remove_slot_if_current, same_directory, status_detail,
        validate_session_id, AgentBridge, AgentEvent, AgentService, BridgeError, BridgeState,
        BridgeStopper, EventSink, RemovalState, ServerCredentials, MAX_EVENT_HEADERS_BYTES,
        MAX_EVENT_HEADER_LINE_BYTES, MAX_PROMPT_BYTES, MAX_SSE_FRAME_BYTES, MAX_SSE_FRAME_LINES,
    };
    use crate::services::opencode::ServiceEndpoint;

    fn test_event_bridge(directory: &Path, port: u16) -> AgentBridge {
        AgentBridge {
            checkout_id: "test-checkout".into(),
            credentials: ServerCredentials {
                endpoint: ServiceEndpoint {
                    url: format!("http://127.0.0.1:{port}"),
                    port,
                    password: "test-secret".into(),
                },
                directory: directory.to_path_buf(),
            },
            event_socket: Mutex::new(None),
            reader: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn mock_event_response(
        directory: &Path,
        response: Vec<u8>,
    ) -> (AgentBridge, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock server should bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = Vec::new();
                let mut buffer = [0; 512];
                while request.len() < 2048 && !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let Ok(read) = stream.read(&mut buffer) else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let _ = stream.write_all(&response);
            }
        });
        (test_event_bridge(directory, port), server)
    }

    /// A service that answers every request with an empty agent catalog.
    ///
    /// The thread is left running rather than joined: it ends when the test binary does, and a
    /// listener waiting for a request nobody makes is not something to wait on. Its port is
    /// returned because that is all a caller needs to point a bridge at it.
    fn serving_empty_catalog(directory: &Path) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock service should bind");
        let port = listener.local_addr().unwrap().port();
        let body = serde_json::to_vec(&serde_json::json!({
            "data": [],
            "location": { "directory": directory.to_string_lossy() },
        }))
        .expect("the catalog envelope should encode");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                if write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .is_err()
                {
                    return;
                }
                if stream.write_all(&body).is_err() {
                    return;
                }
            }
        });
        port
    }

    type CapturedJsonRequest = (String, String, Vec<u8>);

    fn mock_json_responses(
        responses: Vec<serde_json::Value>,
    ) -> (u16, std::thread::JoinHandle<Vec<CapturedJsonRequest>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock server should bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            responses
                .into_iter()
                .map(|response| {
                    let (mut stream, _) = listener.accept().expect("request should connect");
                    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                    let mut request_line = String::new();
                    reader
                        .read_line(&mut request_line)
                        .expect("request line should be readable");
                    let mut authorization = String::new();
                    let mut content_length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).expect("headers should be readable");
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        if let Some((name, value)) = line.trim_end().split_once(':') {
                            if name.eq_ignore_ascii_case("authorization") {
                                authorization = value.trim().to_string();
                            } else if name.eq_ignore_ascii_case("content-length") {
                                content_length = value.trim().parse().expect("valid body length");
                            }
                        }
                    }
                    let mut body = vec![0; content_length];
                    reader.read_exact(&mut body).expect("request body should be readable");
                    let response = serde_json::to_vec(&response).expect("response should encode");
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        response.len()
                    )
                    .expect("response headers should be writable");
                    stream
                        .write_all(&response)
                        .expect("response body should be writable");
                    (request_line.trim().to_string(), authorization, body)
                })
                .collect()
        });
        (port, server)
    }

    fn assert_event_response_rejected(directory: &Path, response: Vec<u8>) {
        let (bridge, server) = mock_event_response(directory, response);
        let error = match event_stream(&bridge) {
            Err(error) => error,
            Ok(reader) => {
                drop(reader);
                panic!("malformed event response should be rejected");
            }
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        bridge.clear_event_socket();
        server.join().expect("mock server should finish");
    }

    #[test]
    fn bounded_line_accepts_exact_limit_at_eof_and_rejects_the_next_byte() {
        let limit = 16;
        let mut exact = std::io::BufReader::with_capacity(8, Cursor::new(vec![b'x'; limit]));
        let mut line = Vec::new();
        assert_eq!(
            read_bounded_line(&mut exact, &mut line, limit).unwrap(),
            limit
        );
        assert_eq!(line.len(), limit);
        assert_eq!(read_bounded_line(&mut exact, &mut line, limit).unwrap(), 0);

        let mut over = std::io::BufReader::with_capacity(8, Cursor::new(vec![b'x'; limit + 1]));
        let error = read_bounded_line(&mut over, &mut line, limit)
            .expect_err("an additional byte must exceed the limit");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(line.len(), limit);
        assert_eq!(read_bounded_line(&mut over, &mut line, limit).unwrap(), 1);
        assert_eq!(line.len(), 1);
    }

    #[test]
    fn an_oversized_line_is_bounded() {
        let input = vec![b'x'; MAX_EVENT_HEADER_LINE_BYTES * 2 + 7];
        let mut reader = std::io::BufReader::with_capacity(1024, Cursor::new(input));
        let mut line = Vec::new();

        let error = read_bounded_line(&mut reader, &mut line, MAX_EVENT_HEADER_LINE_BYTES)
            .expect_err("an oversized line must be rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(line.len(), MAX_EVENT_HEADER_LINE_BYTES);

        // The rejected bytes stay unconsumed, so the same line is rejected again rather than
        // being handed to the caller a second time as if it had been read.
        assert_eq!(
            read_bounded_line(&mut reader, &mut line, MAX_EVENT_HEADER_LINE_BYTES)
                .expect_err("an oversized line must stay rejected")
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(line.len(), MAX_EVENT_HEADER_LINE_BYTES);
    }

    #[test]
    fn event_stream_bounds_header_lines_and_aggregate_header_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let mut long_line = b"HTTP/1.1 200 OK\r\nX: ".to_vec();
        long_line.extend(vec![b'a'; MAX_EVENT_HEADER_LINE_BYTES]);
        long_line.extend_from_slice(b"\r\n\r\n");
        assert_event_response_rejected(directory.path(), long_line);

        let mut many_lines = b"HTTP/1.1 200 OK\r\n".to_vec();
        for _ in 0..=(MAX_EVENT_HEADERS_BYTES / 6) {
            many_lines.extend_from_slice(b"X: a\r\n");
        }
        many_lines.extend_from_slice(b"\r\n");
        assert_event_response_rejected(directory.path(), many_lines);

        assert_event_response_rejected(
            directory.path(),
            b"HTTP/1.1 200 OK\r\nMalformed\r\n\r\n".to_vec(),
        );
    }

    #[test]
    fn sse_frames_bound_single_lines_multiline_data_and_line_count() {
        let mut oversized_line = b"data: ".to_vec();
        oversized_line.extend(vec![b'x'; MAX_SSE_FRAME_BYTES]);
        oversized_line.extend_from_slice(b"\n\n");
        let error = read_sse_frame(&mut Cursor::new(oversized_line))
            .expect_err("an oversized data line must close the stream");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let mut oversized_frame = Vec::new();
        for _ in 0..2 {
            oversized_frame.extend_from_slice(b"data: ");
            oversized_frame.extend(vec![b'x'; MAX_SSE_FRAME_BYTES / 2]);
            oversized_frame.push(b'\n');
        }
        let error = read_sse_frame(&mut Cursor::new(oversized_frame))
            .expect_err("the aggregate multiline frame must stay bounded");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let mut exact_bytes = b"data: {}\n".to_vec();
        exact_bytes.resize(MAX_SSE_FRAME_BYTES, b':');
        assert_eq!(
            read_sse_frame(&mut Cursor::new(exact_bytes.clone()))
                .unwrap()
                .as_deref(),
            Some(&b"{}"[..])
        );
        exact_bytes.push(b'x');
        assert_eq!(
            read_sse_frame(&mut Cursor::new(exact_bytes))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let exact_lines = [
            b"data: {}\n".as_slice(),
            &b":\n".repeat(MAX_SSE_FRAME_LINES - 1),
        ]
        .concat();
        assert_eq!(
            read_sse_frame(&mut Cursor::new(exact_lines.clone()))
                .unwrap()
                .as_deref(),
            Some(&b"{}"[..])
        );
        let mut extra_line = exact_lines;
        extra_line.extend_from_slice(b":\n");
        assert_eq!(
            read_sse_frame(&mut Cursor::new(extra_line))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn sse_frames_accumulate_crlf_multiline_data_and_dispatch_at_eof() {
        let mut reader = Cursor::new(
            ": heartbeat\r\ndata: {\"type\":\"session.idle\",\r\ndata: \"data\":{\"text\":\"café\"}}\r\n\r\n"
                .as_bytes()
                .to_vec(),
        );
        let payload = read_sse_frame(&mut reader)
            .expect("the CRLF multiline frame should parse")
            .expect("the data lines form one frame");
        let parsed: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(parsed["data"]["text"], "café");
        assert!(read_sse_frame(&mut reader).unwrap().is_none());

        let mut eof = Cursor::new(b"data: {\"type\":\"done\"}\r\n".to_vec());
        assert_eq!(
            read_sse_frame(&mut eof).unwrap().as_deref(),
            Some(&b"{\"type\":\"done\"}"[..])
        );
        assert!(read_sse_frame(&mut eof).unwrap().is_none());

        let mut invalid_utf8 = Cursor::new(b"data: \xff\n\n".to_vec());
        assert_eq!(
            read_sse_frame(&mut invalid_utf8).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn sse_payloads_reject_malformed_json_and_foreign_scopes() {
        let local = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let bridge = test_event_bridge(local.path(), 0);
        let location = foreign.path().to_string_lossy();
        let payload = serde_json::json!({
            "type": "session.idle",
            "location": {"directory": location},
            "data": {"sessionID": "ses_foreign"}
        })
        .to_string();
        assert!(event_from_payload(payload.as_bytes(), &bridge)
            .unwrap()
            .is_none());

        let location = local.path().to_string_lossy();
        let payload = serde_json::json!({
            "type": "session.idle",
            "location": {"directory": location},
            "data": {"sessionID": "ses_local"}
        })
        .to_string();
        let event = event_from_payload(payload.as_bytes(), &bridge)
            .unwrap()
            .expect("a local event should pass the scope check");
        assert_eq!(event.checkout_id, "test-checkout");
        assert_eq!(event.session_id.as_deref(), Some("ses_local"));
        assert_eq!(
            event_from_payload(b"not-json", &bridge).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn owned_loopback_api_auth_scopes_catalog_and_prompt_contract_to_checkout() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let first_directory = first.path().to_string_lossy().into_owned();
        let second_directory = second.path().to_string_lossy().into_owned();
        let owned_session = serde_json::json!({
            "id": "ses_owned",
            "title": "Owned session",
            "location": {"directory": first_directory},
            "time": {"created": 1, "updated": 2, "idle": 3}
        });
        let foreign_session = serde_json::json!({
            "id": "ses_foreign",
            "title": "Foreign session",
            "location": {"directory": second_directory}
        });
        let (first_port, first_server) = mock_json_responses(vec![
            serde_json::json!({
                "data": [{"id": "build", "name": "Build", "mode": "primary", "color": "#123456"}],
                "location": {"directory": first_directory}
            }),
            serde_json::json!({"data": [owned_session.clone(), foreign_session]}),
            serde_json::json!({"data": owned_session.clone()}),
            serde_json::json!({"data": {"accepted": true}}),
            serde_json::json!({"data": owned_session.clone()}),
        ]);
        let (second_port, second_server) = mock_json_responses(vec![serde_json::json!({
            "data": {
                "id": "ses_owned",
                "title": "Wrong checkout",
                "location": {"directory": first_directory}
            }
        })]);
        let agents = AgentService::with_test_server(first_port);
        let other_agents = AgentService::with_test_server(second_port);

        let catalog = agents.agents("first", first.path()).unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id, "build");
        let sessions = agents.sessions("first", first.path()).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "ses_owned");
        let prompted = agents
            .prompt("first", first.path(), "ses_owned", "  send one prompt  ")
            .unwrap();
        assert_eq!(prompted.id, "ses_owned");
        assert_eq!(prompted.idle_at, Some(3));
        assert!(matches!(
            other_agents.owned_session("second", second.path(), "ses_owned"),
            Err(BridgeError::Foreign(_))
        ));

        let first_requests = first_server
            .join()
            .expect("first mock server should finish");
        assert_eq!(first_requests.len(), 5);
        assert_eq!(
            first_requests
                .iter()
                .map(|request| request.0.as_str())
                .collect::<Vec<_>>(),
            [
                "GET /api/agent HTTP/1.1",
                "GET /api/session HTTP/1.1",
                "GET /api/session/ses_owned HTTP/1.1",
                "POST /api/session/ses_owned/prompt HTTP/1.1",
                "GET /api/session/ses_owned HTTP/1.1",
            ]
        );
        let first_authorization = crate::services::opencode::authorization("test");
        assert!(first_requests
            .iter()
            .all(|request| request.1 == first_authorization));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&first_requests[3].2).unwrap(),
            serde_json::json!({"text": "send one prompt"})
        );

        let second_requests = second_server
            .join()
            .expect("second mock server should finish");
        assert_eq!(second_requests.len(), 1);
        assert_eq!(second_requests[0].0, "GET /api/session/ses_owned HTTP/1.1");
        assert_eq!(second_requests[0].1, first_authorization);
    }

    #[test]
    fn old_bridge_events_are_dropped_after_checkout_reopens() {
        let state = Arc::new(Mutex::new(RemovalState::default()));
        let delivered = Arc::new(AtomicUsize::new(0));
        let forward = Arc::clone(&delivered);
        let sink: EventSink = Arc::new(move |_| {
            forward.fetch_add(1, Ordering::SeqCst);
        });
        let old =
            generation_scoped_sink(Arc::clone(&state), "checkout".into(), 0, Arc::clone(&sink));
        let event = || AgentEvent {
            checkout_id: "checkout".into(),
            session_id: None,
            kind: crate::domain::agent::AgentEventKind::Unknown,
            raw_type: "test".into(),
            data: serde_json::json!({}),
        };

        old(event());
        assert_eq!(delivered.load(Ordering::SeqCst), 1);
        state
            .lock()
            .unwrap()
            .generations
            .insert("checkout".into(), 1);
        old(event());
        assert_eq!(delivered.load(Ordering::SeqCst), 1);
        generation_scoped_sink(state, "checkout".into(), 1, sink)(event());
        assert_eq!(delivered.load(Ordering::SeqCst), 2);
    }

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
        // These tests cover the service lifecycle, not any connection to a service.
        Arc::new(AgentBridge {
            checkout_id: checkout_id.to_string(),
            credentials: ServerCredentials {
                endpoint: ServiceEndpoint {
                    url: "http://127.0.0.1:1".into(),
                    port: 1,
                    password: "test".into(),
                },
                directory: PathBuf::new(),
            },
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

    fn error_message(error: &BridgeError) -> &str {
        match error {
            BridgeError::Unavailable(message)
            | BridgeError::Foreign(message)
            | BridgeError::Failed(message) => message,
        }
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
        let bridge = Arc::new(bridge_at("idle-sse", directory.path(), port));
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

    /// A bridge pointed at a loopback port that is serving an event stream, for the tests that cover
    /// the reader's own behaviour rather than how a service was found.
    fn bridge_at(checkout_id: &str, directory: &Path, port: u16) -> AgentBridge {
        AgentBridge {
            checkout_id: checkout_id.to_string(),
            credentials: ServerCredentials {
                endpoint: ServiceEndpoint {
                    url: format!("http://127.0.0.1:{port}"),
                    port,
                    password: "secret".into(),
                },
                directory: directory.to_path_buf(),
            },
            event_socket: Mutex::new(None),
            reader: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }
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
        let bridge = bridge_at("truncated-sse", directory.path(), port);
        let mut reader = event_stream(&bridge).expect("the truncated stream should connect");
        let started = Instant::now();
        let mut body = Vec::new();
        let result = reader.read_to_end(&mut body);

        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert!(body.is_empty());
        bridge.clear_event_socket();
        server.join().unwrap();
    }

    #[test]
    fn truncated_chunk_cannot_dispatch_a_complete_json_line() {
        let directory = tempfile::tempdir().unwrap();
        let body =
            b"data: {\"type\":\"session.idle\",\"data\":{\"sessionID\":\"ses_truncated\"}}\n";
        let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        response.extend_from_slice(format!("{:x}\r\n", body.len() + 8).as_bytes());
        response.extend_from_slice(body);
        let (bridge, server) = mock_event_response(directory.path(), response);
        let mut reader =
            std::io::BufReader::new(event_stream(&bridge).expect("mock stream should connect"));

        let error = read_sse_frame(&mut reader)
            .expect_err("EOF inside a declared chunk must not dispatch the pending frame");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        bridge.clear_event_socket();
        server.join().expect("mock server should finish");
    }

    #[test]
    fn truncated_chunk_at_exact_frame_byte_limit_is_not_accepted_as_eof() {
        let directory = tempfile::tempdir().unwrap();
        let mut body = b"data: {}\n".to_vec();
        body.resize(MAX_SSE_FRAME_BYTES, b':');
        let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        response.extend_from_slice(format!("{:x}\r\n", body.len() + 1).as_bytes());
        response.extend_from_slice(&body);
        let (bridge, server) = mock_event_response(directory.path(), response);
        let mut reader =
            std::io::BufReader::new(event_stream(&bridge).expect("mock stream should connect"));

        let error = read_sse_frame(&mut reader)
            .expect_err("truncated chunk EOF must not finalize an exact-cap frame");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        bridge.clear_event_socket();
        server.join().expect("mock server should finish");
    }

    #[test]
    fn truncated_chunk_terminator_cannot_dispatch_a_complete_frame() {
        let directory = tempfile::tempdir().unwrap();
        let body = b"data: {\"type\":\"session.idle\",\"data\":{}}\n\n";
        let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        response.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
        response.extend_from_slice(body);
        let (bridge, server) = mock_event_response(directory.path(), response);
        let mut reader =
            std::io::BufReader::new(event_stream(&bridge).expect("mock stream should connect"));

        let error = read_sse_frame(&mut reader)
            .expect_err("EOF before the chunk CRLF must not dispatch its event");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        bridge.clear_event_socket();
        server.join().expect("mock server should finish");
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

    /// The service the person running the tests already has open.
    ///
    /// The live tests talk to that one rather than to a service of their own, because this app does
    /// not start one; a person running them is expected to have OpenCode running for them to pass.
    fn live_service() -> AgentService {
        let home = std::env::var_os("HOME").expect("a live test needs a home to find OpenCode in");
        AgentService::new(PathBuf::from(home))
    }

    #[test]
    fn a_service_with_no_agents_yet_is_given_until_the_deadline_and_no_longer() {
        let directory = tempfile::tempdir().unwrap();
        // A service that answers for this checkout with an empty catalog, which is what the first
        // moments after someone starts OpenCode look like. It keeps answering, because how many
        // times a readiness loop asks is a function of how fast it answers, not something a test
        // should have to predict; the loop is bounded by the deadline instead.
        let port = serving_empty_catalog(directory.path());
        let credentials = ServerCredentials {
            endpoint: ServiceEndpoint {
                url: format!("http://127.0.0.1:{port}"),
                port,
                password: "secret".into(),
            },
            directory: directory.path().to_path_buf(),
        };

        let error = ready(credentials, Instant::now() + Duration::from_millis(100))
            .expect_err("an empty catalog must not pass as a ready service");

        assert!(
            error_message(&error).contains("did not become ready"),
            "{error:?}"
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
        let agents = live_service();
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
        let agents = live_service();
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
        let agents = live_service();
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
        let agents = live_service();

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
    /// different checkouts: the shape the acceptance runbook asks for.
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

        let agents = live_service();
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
