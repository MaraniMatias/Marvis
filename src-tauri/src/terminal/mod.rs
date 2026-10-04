use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use crate::domain::workspace::{TerminalProcessState, TerminalSessionStatus};

mod process;

pub type OutputSink = Box<dyn FnMut(&[u8]) -> Result<(), String> + Send + 'static>;

pub struct SpawnOptions {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
}

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<ChildState>,
    process_id: Option<u32>,
    #[cfg(unix)]
    terminal_session_id: Option<u32>,
    closing: AtomicBool,
}

struct ChildState {
    child: Option<Box<dyn Child + Send + Sync>>,
    exit_code: Option<u32>,
    child_released: bool,
}

enum SessionEntry {
    Active(Arc<Session>),
    Closing(Arc<Session>),
    CleanupFailed(Arc<Session>),
}

impl SessionEntry {
    fn session(&self) -> &Arc<Session> {
        match self {
            Self::Active(session) | Self::Closing(session) | Self::CleanupFailed(session) => {
                session
            }
        }
    }
}

#[derive(Default)]
pub struct TerminalBackend {
    sessions: Mutex<HashMap<String, SessionEntry>>,
}

impl TerminalBackend {
    pub fn spawn(
        &self,
        id: String,
        options: SpawnOptions,
        mut output: OutputSink,
    ) -> Result<(), String> {
        if options.cols == 0 || options.rows == 0 {
            return Err("PTY dimensions must be non-zero".into());
        }
        if !options.cwd.is_dir() {
            return Err(format!(
                "PTY working directory is not a directory: {}",
                options.cwd.display()
            ));
        }

        let system = native_pty_system();
        let pair = system
            .openpty(PtySize {
                rows: options.rows,
                cols: options.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let mut command = CommandBuilder::new(options.program);
        command.args(options.args);
        command.cwd(options.cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "Marvis");

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        let process_id = child.process_id();
        #[cfg(unix)]
        let terminal_session_id = process_session_id(process_id);
        drop(pair.slave);

        let session = Arc::new(Session {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(ChildState {
                child: Some(child),
                exit_code: None,
                child_released: false,
            }),
            process_id,
            #[cfg(unix)]
            terminal_session_id,
            closing: AtomicBool::new(false),
        });
        if let Err(error) = thread::Builder::new()
            .name("marvis-pty-reader".into())
            .spawn(move || read_output(reader, &mut output))
        {
            let _ = terminate_session(&session, &id);
            return Err(error.to_string());
        }

        let inserted = match self.sessions.lock() {
            Ok(mut sessions) if !sessions.contains_key(&id) => {
                sessions.insert(id.clone(), SessionEntry::Active(Arc::clone(&session)));
                Ok(())
            }
            Ok(_) => Err("terminal session already exists".to_string()),
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = inserted {
            let _ = terminate_session(&session, &id);
            return Err(error);
        }
        Ok(())
    }

    fn session(&self, id: &str) -> Result<Arc<Session>, String> {
        let sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        match sessions.get(id) {
            Some(SessionEntry::Active(session)) => Ok(Arc::clone(session)),
            Some(SessionEntry::Closing(_) | SessionEntry::CleanupFailed(_)) => {
                Err(format!("terminal session {id} is closing"))
            }
            None => Err(format!("unknown terminal session {id}")),
        }
    }

    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        let session = self.session(id)?;
        let mut writer = session.writer.lock().map_err(|error| error.to_string())?;
        if session.closing.load(Ordering::Acquire) {
            return Err(format!("terminal session {id} is closing"));
        }
        writer.write_all(bytes).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(u16, u16), String> {
        if cols == 0 || rows == 0 {
            return Err("PTY dimensions must be non-zero".into());
        }
        let session = self.session(id)?;
        let master = session.master.lock().map_err(|error| error.to_string())?;
        if session.closing.load(Ordering::Acquire) {
            return Err(format!("terminal session {id} is closing"));
        }
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let size = master.get_size().map_err(|error| error.to_string())?;
        Ok((size.cols, size.rows))
    }

    pub fn status(&self, id: &str) -> Result<TerminalSessionStatus, String> {
        let session = self.session(id)?;
        let mut process = session.child.lock().map_err(|error| error.to_string())?;
        if session.closing.load(Ordering::Acquire) {
            return Err(format!("terminal session {id} is closing"));
        }
        if process.exit_code.is_none() {
            if let Some(status) = process
                .child
                .as_mut()
                .ok_or_else(|| format!("terminal session {id} is closing"))?
                .try_wait()
                .map_err(|error| error.to_string())?
            {
                process.exit_code = Some(status.exit_code());
            }
        }
        let exit_code = process.exit_code;
        drop(process);
        Ok(match exit_code {
            Some(exit_code) => TerminalSessionStatus {
                state: TerminalProcessState::Exited,
                exit_code: Some(exit_code),
                foreground_process: false,
                foreground_app: None,
            },
            None => TerminalSessionStatus {
                state: TerminalProcessState::Running,
                exit_code: None,
                foreground_process: session.foreground_process(),
                foreground_app: session.foreground_app(),
            },
        })
    }

    pub fn close(&self, id: &str) -> Result<bool, String> {
        self.close_with(id, || {})
    }

    fn close_with(&self, id: &str, on_master_contention: impl FnMut()) -> Result<bool, String> {
        let session = {
            let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
            let Some(entry) = sessions.get(id) else {
                return Ok(false);
            };
            let session = match entry {
                SessionEntry::Active(session) | SessionEntry::CleanupFailed(session) => {
                    Arc::clone(session)
                }
                SessionEntry::Closing(_) => {
                    return Err(format!("terminal session {id} is already closing"));
                }
            };
            sessions.insert(id.to_string(), SessionEntry::Closing(Arc::clone(&session)));
            session
        };
        if let Err(error) = terminate_session_with(&session, id, on_master_contention) {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(sessions.get(id), Some(SessionEntry::Closing(current)) if Arc::ptr_eq(current, &session))
            {
                sessions.insert(
                    id.to_string(),
                    SessionEntry::CleanupFailed(Arc::clone(&session)),
                );
            }
            return Err(error);
        }
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(sessions.get(id), Some(SessionEntry::Closing(current)) if Arc::ptr_eq(current, &session))
        {
            sessions.remove(id);
        }
        Ok(true)
    }
}

impl Session {
    fn foreground_process(&self) -> bool {
        self.foreground_group()
            .zip(self.process_id)
            .is_some_and(|(group, shell)| group != shell)
    }

    /// The program the foreground group is running, when that group is not the shell itself.
    ///
    /// A job-control shell gives every job its own group led by the job's first process, so
    /// the group's id *is* that process's pid and the OS can name it. Resolving the group
    /// rather than a fixed pid is what keeps the answer right after a job is replaced.
    fn foreground_app(&self) -> Option<String> {
        let (group, shell) = self.foreground_group().zip(self.process_id)?;
        (group != shell)
            .then(|| process::executable_name(group))
            .flatten()
    }

    fn foreground_group(&self) -> Option<u32> {
        self.master
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .process_group_leader()
            .and_then(|group| u32::try_from(group).ok())
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        let sessions = self
            .sessions
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (id, session) in sessions {
            if let Err(error) = terminate_session(session.session(), id) {
                log::warn!("terminal {id} did not stop with the app: {error}");
            }
        }
    }
}

const TERMINATE_GRACE: Duration = Duration::from_millis(200);
const TERMINATE_TIMEOUT: Duration = Duration::from_secs(1);
const REAP_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn terminate_session(session: &Session, id: &str) -> Result<(), String> {
    terminate_session_with(session, id, || {})
}

fn terminate_session_with(
    session: &Session,
    id: &str,
    mut on_master_contention: impl FnMut(),
) -> Result<(), String> {
    session.closing.store(true, Ordering::Release);
    let mut process = session
        .child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if process.exit_code.is_none() {
        if let Some(child) = process.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => process.exit_code = Some(status.exit_code()),
                Ok(None) => {}
                Err(error) => log::warn!("could not inspect terminal {id} before closing: {error}"),
            }
        }
    }

    #[cfg(unix)]
    {
        if let Err(error) = signal_foreground_group(
            session,
            libc::SIGTERM,
            id,
            TERMINATE_GRACE,
            &mut on_master_contention,
        ) {
            log::warn!("terminal {id} foreground TERM deferred: {error}");
        }
        if process.exit_code.is_none() {
            if let Err(error) = signal_shell_group(session, libc::SIGTERM, id) {
                log::warn!("terminal {id} shell TERM failed; child kill remains armed: {error}");
            }
        }
    }
    #[cfg(not(unix))]
    if process.exit_code.is_none() {
        if let Some(child) = process.child.as_mut() {
            if let Err(error) = child.kill() {
                log::warn!("could not stop terminal {id} before escalation: {error}");
            }
        }
    }

    if process.exit_code.is_none() {
        if let Err(error) = wait_for_exit(&mut process, TERMINATE_GRACE) {
            log::warn!("could not inspect terminal {id} during graceful close: {error}");
        }
    }
    #[cfg(unix)]
    let foreground_cleanup_error = signal_foreground_group(
        session,
        libc::SIGKILL,
        id,
        TERMINATE_TIMEOUT,
        &mut on_master_contention,
    )
    .err();
    #[cfg(unix)]
    if process.exit_code.is_none() {
        if let Err(error) = signal_shell_group(session, libc::SIGKILL, id) {
            log::warn!(
                "terminal {id} shell SIGKILL failed; portable child kill remains armed: {error}"
            );
        }
    }

    let child_result = finish_child(&mut process, id);
    #[cfg(unix)]
    if let Some(error) = foreground_cleanup_error {
        return Err(format!(
            "terminal {id} child cleanup: {}; foreground cleanup failed: {error}",
            child_result.err().unwrap_or_else(|| "reaped".into())
        ));
    }
    child_result
}

fn finish_child(process: &mut ChildState, id: &str) -> Result<(), String> {
    if process.child_released {
        return Ok(());
    }
    if process.exit_code.is_none() {
        if let Some(child) = process.child.as_mut() {
            if let Err(error) = child.kill() {
                log::warn!("could not kill terminal {id}: {error}");
            }
        }
        if process.child.is_some() {
            if let Err(error) = wait_for_exit(process, TERMINATE_TIMEOUT) {
                log::warn!("could not inspect terminal {id} after kill: {error}");
            }
        }
    }
    if process.exit_code.is_some() {
        return Ok(());
    }
    match start_child_reaper(process, |reaper| {
        thread::Builder::new()
            .name("marvis-pty-reaper".into())
            .spawn(reaper)
            .map(drop)
    }) {
        Ok(()) => Err(format!(
            "terminal {id} did not exit after kill; reaping continues in background"
        )),
        Err(error) => Err(format!(
            "terminal {id} did not exit; reaper could not start: {error}"
        )),
    }
}

fn start_child_reaper(
    process: &mut ChildState,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if process.child_released {
        return Ok(());
    }
    let Some(child) = process.child.take() else {
        return Err(std::io::Error::other("terminal child handle is missing"));
    };
    let child = Arc::new(Mutex::new(Some(child)));
    let reaper_child = Arc::clone(&child);
    let reaper = Box::new(move || {
        let mut child = reaper_child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(child) = child.as_mut() {
            if let Err(error) = child.wait() {
                log::warn!("terminal child reaper failed: {error}");
            }
        }
    });
    match spawn(reaper) {
        Ok(()) => {
            process.child_released = true;
            Ok(())
        }
        Err(error) => {
            process.child = child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            Err(error)
        }
    }
}

fn wait_for_exit(process: &mut ChildState, timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if process.exit_code.is_some() {
            return Ok(true);
        }
        let child = process
            .child
            .as_mut()
            .ok_or_else(|| "terminal child handle was transferred to its reaper".to_string())?;
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            process.exit_code = Some(status.exit_code());
            return Ok(true);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        thread::sleep(remaining.min(REAP_POLL_INTERVAL));
    }
}

#[cfg(unix)]
fn signal_foreground_group(
    session: &Session,
    signal: libc::c_int,
    id: &str,
    timeout: Duration,
    on_contention: &mut impl FnMut(),
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let master = loop {
        match session.master.try_lock() {
            Ok(master) => break master,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => on_contention(),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("could not reacquire the PTY master".into());
        }
        thread::sleep(remaining.min(REAP_POLL_INTERVAL));
    };
    let Some(group) = master
        .process_group_leader()
        .and_then(|group| u32::try_from(group).ok())
        .filter(|group| *group > 0)
    else {
        return Ok(());
    };
    let expected_session = session
        .terminal_session_id
        .and_then(|sid| libc::pid_t::try_from(sid).ok())
        .ok_or_else(|| format!("terminal {id} session could not be verified"))?;
    let fd = master
        .as_raw_fd()
        .ok_or_else(|| format!("terminal {id} PTY descriptor is unavailable"))?;
    // tcgetpgrp's result belongs to the PTY's controlling session; tcgetsid verifies that
    // session without requiring the foreground process-group leader to still exist.
    let actual_session = unsafe { libc::tcgetsid(fd) };
    if actual_session < 0 {
        return Err(format!(
            "could not verify terminal {id} PTY session: {}",
            std::io::Error::last_os_error()
        ));
    }
    let sent = signal_group_with(
        Some(group),
        u32::try_from(actual_session).ok(),
        signal,
        |_, sid| sid == expected_session as u32,
        |group, signal| {
            if unsafe { libc::kill(group, signal) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        },
    )
    .map_err(|error| format!("could not signal terminal {id} foreground group: {error}"))?;
    if !sent && process_group_exists(group) {
        return Err(format!(
            "terminal {id} foreground group is not verified in its original PTY session"
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn signal_shell_group(session: &Session, signal: libc::c_int, id: &str) -> Result<(), String> {
    signal_process_group(session.process_id, session.terminal_session_id, signal, id)?;
    signal_process(session.process_id, session.terminal_session_id, signal, id)
}

#[cfg(unix)]
fn process_session_id(pid: Option<u32>) -> Option<u32> {
    let pid = libc::pid_t::try_from(pid?).ok()?;
    let session_id = unsafe { libc::getsid(pid) };
    (session_id > 0)
        .then(|| u32::try_from(session_id).ok())
        .flatten()
}

#[cfg(unix)]
fn process_group_in_session(group: u32, session_id: u32) -> bool {
    let (Ok(group), Ok(session_id)) = (
        libc::pid_t::try_from(group),
        libc::pid_t::try_from(session_id),
    ) else {
        return false;
    };
    group > 0
        && session_id > 0
        && unsafe { libc::getpgid(group) == group && libc::getsid(group) == session_id }
}

#[cfg(unix)]
fn process_group_exists(group: u32) -> bool {
    let Ok(group) = libc::pid_t::try_from(group) else {
        return false;
    };
    if group <= 0 || unsafe { libc::kill(-group, 0) } == 0 {
        return group > 0;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
fn signal_group_with(
    group: Option<u32>,
    session_id: Option<u32>,
    signal: libc::c_int,
    belongs_to_session: impl FnOnce(u32, u32) -> bool,
    send: impl FnOnce(libc::pid_t, libc::c_int) -> Result<(), std::io::Error>,
) -> Result<bool, std::io::Error> {
    let (Some(group), Some(session_id)) = (group, session_id) else {
        return Ok(false);
    };
    if group == 0 || !belongs_to_session(group, session_id) {
        return Ok(false);
    }
    let group = libc::pid_t::try_from(group).map_err(std::io::Error::other)?;
    match send(-group, signal) {
        Ok(()) => Ok(true),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn signal_process_group(
    group: Option<u32>,
    session_id: Option<u32>,
    signal: libc::c_int,
    id: &str,
) -> Result<(), String> {
    let Some(group) = group else {
        return Ok(());
    };
    let sent = signal_group_with(
        Some(group),
        session_id,
        signal,
        process_group_in_session,
        |group, signal| {
            if unsafe { libc::kill(group, signal) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        },
    )
    .map_err(|error| format!("could not signal terminal {id} process group: {error}"))?;
    if !sent && process_group_exists(group) {
        return Err(format!(
            "terminal {id} process group could not be verified in its original session"
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn signal_process(
    pid: Option<u32>,
    session_id: Option<u32>,
    signal: libc::c_int,
    id: &str,
) -> Result<(), String> {
    let Some(pid) = pid.and_then(|pid| libc::pid_t::try_from(pid).ok()) else {
        return Ok(());
    };
    let Some(session_id) = session_id.and_then(|id| libc::pid_t::try_from(id).ok()) else {
        return Err(format!("terminal {id} shell session could not be verified"));
    };
    let actual_session = unsafe { libc::getsid(pid) };
    if actual_session < 0 {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(format!(
                "could not verify terminal {id} shell session: {error}"
            ))
        };
    }
    if actual_session != session_id {
        return Err(format!(
            "terminal {id} shell PID no longer belongs to its session"
        ));
    }
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!("could not signal terminal {id} shell: {error}"))
    }
}

fn read_output(mut reader: Box<dyn Read + Send>, output: &mut OutputSink) {
    let mut buffer = [0_u8; 64 * 1024];
    // Said once. A window that is gone refuses every chunk from here to the end of the session,
    // and a line per chunk would bury everything else the log is for.
    let mut reported = false;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                // A disconnected renderer must not stop draining the PTY and deadlock its child.
                if let Err(error) = output(&buffer[..count]) {
                    if !reported {
                        reported = true;
                        log::warn!(
                            "terminal output stopped being delivered, dropping the rest: {error}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    use portable_pty::{Child, ChildKiller, ExitStatus};
    use tempfile::tempdir;

    #[cfg(unix)]
    use super::signal_group_with;
    use super::{start_child_reaper, ChildState, OutputSink, SpawnOptions, TerminalBackend};

    fn spawn(
        backend: &TerminalBackend,
        id: &str,
        program: &str,
        args: &[&str],
        output: OutputSink,
    ) {
        backend
            .spawn(
                id.to_string(),
                SpawnOptions {
                    program: PathBuf::from(program),
                    args: args.iter().map(|arg| (*arg).to_string()).collect(),
                    cwd: std::env::current_dir().unwrap(),
                    cols: 80,
                    rows: 24,
                },
                output,
            )
            .unwrap();
    }

    fn sink() -> (OutputSink, mpsc::Receiver<Vec<u8>>) {
        let (sender, receiver) = mpsc::channel();
        (
            Box::new(move |bytes| sender.send(bytes.to_vec()).map_err(|e| e.to_string())),
            receiver,
        )
    }

    struct PausingWriter {
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
    }

    impl Write for PausingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.entered.take().unwrap().send(()).map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "test writer receiver dropped")
            })?;
            self.release.recv().map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "test writer release dropped")
            })?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn synchronize_shell(backend: &TerminalBackend, receiver: &mpsc::Receiver<Vec<u8>>, id: &str) {
        backend
            .write(id, b"printf '\\036MARVIS_READY\\037\\n'\n")
            .unwrap();
        wait_for_output(receiver, b"\x1eMARVIS_READY\x1f", Duration::from_secs(5));
    }

    fn wait_for_output(receiver: &mpsc::Receiver<Vec<u8>>, marker: &[u8], timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut actual = Vec::new();
        while !actual.windows(marker.len()).any(|window| window == marker) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "terminal output marker timed out");
            match receiver.recv_timeout(remaining) {
                Ok(bytes) => actual.extend(bytes),
                Err(error) => panic!("terminal output marker timed out: {error}"),
            }
        }
    }

    #[test]
    fn preserves_binary_output_and_reports_exit_code() {
        let backend = TerminalBackend::default();
        let (output, receiver) = sink();
        spawn(
            &backend,
            "binary",
            "/bin/sh",
            &["-c", r"printf '\303\251\033[31mred\033[0m'; exit 7"],
            output,
        );
        let mut actual = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !actual.windows(3).any(|window| window == b"red") {
            actual.extend(
                receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap(),
            );
        }
        assert!(actual.starts_with("é\x1b[31m".as_bytes()));
        assert!(actual.ends_with(b"\x1b[0m"));

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(status) = backend.status("binary") {
                if status.state == TerminalProcessState::Exited {
                    assert_eq!(status.exit_code, Some(7));
                    break;
                }
            }
            assert!(Instant::now() < deadline, "shell exit status timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// A shell that ends is the ordinary way a terminal session is used up — `exit`, `:q`, a
    /// logout — and the app that hosted it has to still be there afterwards. That is the invariant
    /// this pins: the process that spawned the shell outlives it, the ending is *reported* rather
    /// than the session quietly vanishing, and the next session in the same backend works. A
    /// backend that mistook its shell's exit for its own would take this test process down with
    /// it, so the assertions after the `exit` are the ones that carry the weight.
    #[cfg(unix)]
    #[test]
    fn a_shell_that_exits_leaves_the_backend_running_and_the_next_session_usable() {
        let backend = TerminalBackend::default();
        let (output, _receiver) = sink();
        spawn(&backend, "exiting", "/bin/sh", &["-i"], output);

        backend.write("exiting", b"exit\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = backend.status("exiting").unwrap();
            if status.state == TerminalProcessState::Exited {
                assert_eq!(status.exit_code, Some(0));
                break;
            }
            assert!(Instant::now() < deadline, "shell exit status timed out");
            thread::sleep(Duration::from_millis(10));
        }
        // Still a session to close, which is what the panel does when it reads that exit.
        assert!(backend.close("exiting").unwrap());

        let (output, echoed) = sink();
        spawn(&backend, "next", "/bin/cat", &[], output);
        backend.write("next", b"still here\n").unwrap();
        let written = echoed.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(written.windows(10).any(|window| window == b"still here"));
        assert_eq!(
            backend.status("next").unwrap().state,
            TerminalProcessState::Running
        );
        backend.close("next").unwrap();
    }

    #[test]
    fn shell_receives_the_requested_working_directory_and_terminal_environment() {
        let backend = TerminalBackend::default();
        let directory = tempdir().unwrap();
        let (output, receiver) = sink();
        backend
            .spawn(
                "environment".into(),
                SpawnOptions {
                    program: PathBuf::from("/bin/sh"),
                    args: vec![
                        "-c".into(),
                        r#"pwd; printf '%s\n' "$TERM" "$COLORTERM""#.into(),
                    ],
                    cwd: directory.path().to_path_buf(),
                    cols: 80,
                    rows: 24,
                },
                output,
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut actual = Vec::new();
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(bytes) => actual.extend(bytes),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(error) => panic!("shell output timed out: {error}"),
            }
        }
        let output = String::from_utf8_lossy(&actual);
        assert!(output.contains(
            &directory
                .path()
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
        ));
        assert!(output.contains("xterm-256color"));
        assert!(output.contains("truecolor"));
        backend.close("environment").unwrap();
    }

    #[test]
    fn sessions_have_independent_input_and_dimensions_and_close_running_processes() {
        let backend = TerminalBackend::default();
        let (first_sink, first_output) = sink();
        let (second_sink, second_output) = sink();
        spawn(&backend, "first", "/bin/cat", &[], first_sink);
        spawn(&backend, "second", "/bin/cat", &[], second_sink);

        let initial_size = backend.sessions.lock().unwrap()["first"]
            .session()
            .master
            .lock()
            .unwrap()
            .get_size()
            .unwrap();
        assert_eq!((initial_size.cols, initial_size.rows), (80, 24));
        assert_eq!(backend.resize("first", 91, 31).unwrap(), (91, 31));
        backend.write("first", b"one\n").unwrap();
        backend.write("second", b"two\n").unwrap();
        let first = first_output.recv_timeout(Duration::from_secs(3)).unwrap();
        let second = second_output.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(first.windows(3).any(|window| window == b"one"));
        assert!(second.windows(3).any(|window| window == b"two"));
        assert_eq!(
            backend.status("first").unwrap().state,
            TerminalProcessState::Running
        );
        assert!(backend.close("first").unwrap());
        assert!(backend.close("second").unwrap());
        assert!(!backend.close("second").unwrap());
    }

    #[test]
    fn blocked_writer_does_not_block_other_sessions_or_close() {
        let backend = Arc::new(TerminalBackend::default());
        let (blocked_output, _) = sink();
        let (other_output, _) = sink();
        spawn(&backend, "blocked", "/bin/cat", &[], blocked_output);
        spawn(&backend, "other", "/bin/cat", &[], other_output);

        let (entered, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let session = backend.sessions.lock().unwrap()["blocked"]
            .session()
            .clone();
        *session.writer.lock().unwrap() = Box::new(PausingWriter {
            entered: Some(entered),
            release: release_rx,
        });
        drop(session);

        let (write_done, write_result) = mpsc::channel();
        let write_backend = Arc::clone(&backend);
        let write_thread = thread::spawn(move || {
            let _ = write_done.send(write_backend.write("blocked", b"blocked"));
        });
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();

        let (other_done, other_result) = mpsc::channel();
        let other_backend = Arc::clone(&backend);
        let other_thread = thread::spawn(move || {
            let _ = other_done.send(other_backend.write("other", b"unblocked\n"));
        });
        let other_result = other_result.recv_timeout(Duration::from_secs(2));

        let (close_done, close_result) = mpsc::channel();
        let close_backend = Arc::clone(&backend);
        let close_thread = thread::spawn(move || {
            let _ = close_done.send(close_backend.close("blocked"));
        });
        let close_result = close_result.recv_timeout(Duration::from_secs(2));

        release.send(()).unwrap();
        let write_result = write_result.recv_timeout(Duration::from_secs(2));
        let _ = write_thread.join();
        let _ = other_thread.join();
        let _ = close_thread.join();

        assert!(other_result
            .expect("other session write was blocked")
            .is_ok());
        assert!(close_result
            .expect("close waited for the blocked writer")
            .unwrap());
        assert!(write_result.expect("blocked writer did not resume").is_ok());
        assert!(backend.close("other").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn close_retries_foreground_lookup_after_transient_master_contention() {
        let backend = Arc::new(TerminalBackend::default());
        let (output, receiver) = sink();
        spawn(&backend, "contended", "/bin/sh", &["-i"], output);
        synchronize_shell(&backend, &receiver, "contended");
        backend.write("contended", b"sleep 30\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        let (session, group) = loop {
            let status = backend.status("contended").unwrap();
            if status.foreground_app.as_deref() == Some("sleep") {
                let session = backend.sessions.lock().unwrap()["contended"]
                    .session()
                    .clone();
                let group = session
                    .master
                    .lock()
                    .unwrap()
                    .process_group_leader()
                    .unwrap();
                break (session, group);
            }
            assert!(
                Instant::now() < deadline,
                "foreground sleep was not observed"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let master_guard = session.master.lock().unwrap();
        let (contention_tx, contention_rx) = mpsc::channel();
        let (close_tx, close_rx) = mpsc::channel();
        let close_backend = Arc::clone(&backend);
        let close_thread = thread::spawn(move || {
            let mut sent_contention = Some(contention_tx);
            let _ = close_tx.send(close_backend.close_with("contended", || {
                if let Some(sender) = sent_contention.take() {
                    let _ = sender.send(());
                }
            }));
        });
        contention_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("close did not observe the held master lock");
        drop(master_guard);
        assert!(close_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap());
        close_thread.join().unwrap();

        assert_ne!(unsafe { libc::kill(-group, 0) }, 0);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_group_signals_reject_gone_or_reused_foreign_groups() {
        let mut signaled = Vec::new();
        let first = signal_group_with(
            Some(123),
            Some(77),
            libc::SIGTERM,
            |group, session_id| group == 123 && session_id == 77,
            |target, _| {
                signaled.push(target);
                Ok(())
            },
        )
        .unwrap();
        let escalation = signal_group_with(
            Some(456),
            Some(77),
            libc::SIGKILL,
            |_, _| false,
            |target, _| {
                signaled.push(target);
                Ok(())
            },
        )
        .unwrap();
        let foreign_session = signal_group_with(
            Some(123),
            Some(78),
            libc::SIGKILL,
            |_, session_id| session_id == 77,
            |target, _| {
                signaled.push(target);
                Ok(())
            },
        )
        .unwrap();
        let gone = signal_group_with(
            Some(123),
            Some(77),
            libc::SIGKILL,
            |_, _| true,
            |_, _| Err(std::io::Error::from_raw_os_error(libc::ESRCH)),
        )
        .unwrap();

        assert!(first);
        assert!(!escalation);
        assert!(!foreign_session);
        assert!(!gone);
        assert_eq!(signaled, vec![-123]);
    }

    #[test]
    fn failed_reaper_spawn_retains_child_for_retry() {
        #[derive(Clone, Debug)]
        struct ReaperChild(Arc<AtomicUsize>);

        impl ChildKiller for ReaperChild {
            fn kill(&mut self) -> io::Result<()> {
                Ok(())
            }

            fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
                Box::new(self.clone())
            }
        }

        impl Child for ReaperChild {
            fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
                Ok(None)
            }

            fn wait(&mut self) -> io::Result<ExitStatus> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(ExitStatus::with_exit_code(0))
            }

            fn process_id(&self) -> Option<u32> {
                None
            }
        }

        let waited = Arc::new(AtomicUsize::new(0));
        let mut process = ChildState {
            child: Some(Box::new(ReaperChild(Arc::clone(&waited)))),
            exit_code: None,
            child_released: false,
        };
        let error = start_child_reaper(&mut process, |_| Err(io::Error::other("injected")))
            .expect_err("injected reaper spawn should fail");
        assert_eq!(error.to_string(), "injected");
        assert!(
            process.child.is_some(),
            "failed spawn discarded the child handle"
        );
        assert!(!process.child_released);

        start_child_reaper(&mut process, |reaper| {
            thread::Builder::new().spawn(reaper).map(drop)
        })
        .unwrap();
        assert!(process.child.is_none());
        assert!(process.child_released);
        let deadline = Instant::now() + Duration::from_secs(2);
        while waited.load(Ordering::SeqCst) == 0 {
            assert!(
                Instant::now() < deadline,
                "retry reaper did not wait for child"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(waited.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[test]
    fn close_kills_foreground_group_when_its_leader_has_exited() {
        let backend = TerminalBackend::default();
        let (output, receiver) = sink();
        spawn(&backend, "leaderless", "/bin/sh", &["-i"], output);
        synchronize_shell(&backend, &receiver, "leaderless");
        backend.write("leaderless", b"true | sleep 30\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        let group = loop {
            let session = backend.sessions.lock().unwrap()["leaderless"]
                .session()
                .clone();
            let group = session
                .master
                .lock()
                .unwrap()
                .process_group_leader()
                .map(|group| group as libc::pid_t);
            if let Some(group) = group {
                let group_exists = unsafe { libc::kill(-group, 0) } == 0;
                let leader_reaped = unsafe { libc::getpgid(group) } < 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
                if group_exists && leader_reaped {
                    break group;
                }
            }
            assert!(
                Instant::now() < deadline,
                "pipeline leader did not exit while its process group remained alive"
            );
            thread::sleep(Duration::from_millis(10));
        };

        assert!(backend.close("leaderless").unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(-group, 0) } == 0 {
            assert!(
                Instant::now() < deadline,
                "leaderless foreground group survived close"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(unix)]
    #[test]
    fn close_kills_the_foreground_process_group() {
        let backend = TerminalBackend::default();
        let (output, receiver) = sink();
        spawn(&backend, "closing-foreground", "/bin/sh", &["-i"], output);
        synchronize_shell(&backend, &receiver, "closing-foreground");
        backend.write("closing-foreground", b"sleep 30\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        let group = loop {
            let status = backend.status("closing-foreground").unwrap();
            if status.foreground_app.as_deref() == Some("sleep") {
                break backend.sessions.lock().unwrap()["closing-foreground"]
                    .session()
                    .master
                    .lock()
                    .unwrap()
                    .process_group_leader()
                    .unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "foreground sleep was not observed"
            );
            thread::sleep(Duration::from_millis(10));
        };

        assert!(backend.close("closing-foreground").unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(-group, 0) } == 0 {
            assert!(
                Instant::now() < deadline,
                "foreground process group was orphaned"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(unix)]
    #[test]
    fn foreground_activity_excludes_an_idle_shell_and_tracks_a_foreground_command() {
        let backend = TerminalBackend::default();
        let (output, receiver) = sink();
        spawn(&backend, "foreground", "/bin/sh", &["-i"], output);
        synchronize_shell(&backend, &receiver, "foreground");

        let resting = backend.status("foreground").unwrap();
        assert!(!resting.foreground_process);
        // A shell in front of itself is the resting state and has nothing to name.
        assert_eq!(resting.foreground_app, None);

        backend.write("foreground", b"sleep 10\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let status = backend.status("foreground").unwrap();
            if status.foreground_process && status.foreground_app.as_deref() == Some("sleep") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "foreground sleep was not observed"
            );
            thread::sleep(Duration::from_millis(10));
        }
        // The row in the sidebar shows this, so it has to be the job's own name.
        assert_eq!(
            backend
                .status("foreground")
                .unwrap()
                .foreground_app
                .as_deref(),
            Some("sleep")
        );

        // Keep the foreground-name polling: the child name can settle after the foreground group
        // is first observed.
        let deadline = Instant::now() + Duration::from_secs(3);
        while backend
            .status("foreground")
            .unwrap()
            .foreground_app
            .as_deref()
            != Some("sleep")
        {
            assert!(
                Instant::now() < deadline,
                "the foreground process was not named after the job"
            );
            thread::sleep(Duration::from_millis(10));
        }

        backend.write("foreground", b"\x03").unwrap();
        synchronize_shell(&backend, &receiver, "foreground");
        let resting = backend.status("foreground").unwrap();
        assert!(!resting.foreground_process);
        assert_eq!(resting.foreground_app, None);
        backend.close("foreground").unwrap();
    }

    use crate::domain::workspace::TerminalProcessState;
}
