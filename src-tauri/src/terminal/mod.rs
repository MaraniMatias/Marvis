use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
    thread,
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
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    process_id: Option<u32>,
    exit_code: Option<u32>,
}

#[derive(Default)]
pub struct TerminalBackend {
    sessions: Mutex<HashMap<String, Session>>,
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
        drop(pair.slave);

        if let Err(error) = thread::Builder::new()
            .name("marvis-pty-reader".into())
            .spawn(move || read_output(reader, &mut output))
        {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }

        let mut sessions = match self.sessions.lock() {
            Ok(sessions) => sessions,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        };
        if sessions.contains_key(&id) {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            return Err("terminal session already exists".into());
        }
        sessions.insert(
            id,
            Session {
                master: pair.master,
                writer,
                child,
                process_id,
                exit_code: None,
            },
        );
        Ok(())
    }

    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| format!("unknown terminal session {id}"))?;
        session
            .writer
            .write_all(bytes)
            .map_err(|error| error.to_string())?;
        session.writer.flush().map_err(|error| error.to_string())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(u16, u16), String> {
        if cols == 0 || rows == 0 {
            return Err("PTY dimensions must be non-zero".into());
        }
        let sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        let master = &sessions
            .get(id)
            .ok_or_else(|| format!("unknown terminal session {id}"))?
            .master;
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
        let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| format!("unknown terminal session {id}"))?;
        if session.exit_code.is_none() {
            if let Some(status) = session
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
            {
                session.exit_code = Some(status.exit_code());
            }
        }
        Ok(match session.exit_code {
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
        let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        let Some(session) = sessions.get_mut(id) else {
            return Ok(false);
        };
        if session.exit_code.is_none() {
            match session
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
            {
                Some(status) => session.exit_code = Some(status.exit_code()),
                None => {
                    if let Err(kill_error) = session.child.kill() {
                        match session
                            .child
                            .try_wait()
                            .map_err(|error| error.to_string())?
                        {
                            Some(status) => session.exit_code = Some(status.exit_code()),
                            None => return Err(kill_error.to_string()),
                        }
                    }
                    if session.exit_code.is_none() {
                        let status = session.child.wait().map_err(|error| error.to_string())?;
                        session.exit_code = Some(status.exit_code());
                    }
                }
            }
        }
        sessions.remove(id);
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
            .process_group_leader()
            .and_then(|group| u32::try_from(group).ok())
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        if let Ok(sessions) = self.sessions.get_mut() {
            for (id, session) in sessions.iter_mut() {
                if session.exit_code.is_none() {
                    // The last chance to notice a child that would otherwise keep running with
                    // nothing holding it, now that the app that spawned it is going away.
                    if let Err(error) = session.child.kill() {
                        log::warn!("terminal {id} did not stop with the app: {error}");
                    }
                    let _ = session.child.wait();
                }
            }
        }
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
        path::PathBuf,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use super::{OutputSink, SpawnOptions, TerminalBackend};

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
            .master
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

    #[cfg(unix)]
    #[test]
    fn foreground_activity_excludes_an_idle_shell_and_tracks_a_foreground_command() {
        let backend = TerminalBackend::default();
        let (output, _receiver) = sink();
        spawn(&backend, "foreground", "/bin/sh", &["-i"], output);
        let resting = backend.status("foreground").unwrap();
        assert!(!resting.foreground_process);
        // A shell in front of itself is the resting state and has nothing to name.
        assert_eq!(resting.foreground_app, None);

        backend.write("foreground", b"sleep 10\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !backend.status("foreground").unwrap().foreground_process {
            assert!(
                Instant::now() < deadline,
                "foreground process was not observed"
            );
            thread::sleep(Duration::from_millis(10));
        }
        // The row in the sidebar shows this, so it has to be the job's own name. A shell puts the
        // group in the foreground before the child in it has become the program that was asked
        // for, so the group already in front is for a moment the shell's own copy of the job: the
        // foreground is observed first and the name settles on it a moment later.
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
        let deadline = Instant::now() + Duration::from_secs(3);
        while backend.status("foreground").unwrap().foreground_process {
            assert!(
                Instant::now() < deadline,
                "foreground process did not return to the shell"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(backend.status("foreground").unwrap().foreground_app, None);
        backend.close("foreground").unwrap();
    }

    use crate::domain::workspace::TerminalProcessState;
}
