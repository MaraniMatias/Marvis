use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
};

pub type TerminalId = u64;
pub type OutputSink = Box<dyn FnMut(&[u8]) -> Result<(), String> + Send + 'static>;

pub struct SpawnOptions {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalMetrics {
    pub bytes_read: u64,
    pub bytes_sent: u64,
    pub dropped_bytes: u64,
    pub reader_closed: bool,
}

#[derive(Default)]
struct Counters {
    bytes_read: AtomicU64,
    bytes_sent: AtomicU64,
    dropped_bytes: AtomicU64,
    reader_closed: AtomicBool,
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    counters: Arc<Counters>,
}

#[derive(Default)]
pub struct TerminalBackend {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<TerminalId, Session>>,
}

impl TerminalBackend {
    pub fn spawn(
        &self,
        options: SpawnOptions,
        mut output: OutputSink,
    ) -> Result<TerminalId, String> {
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
        command.env("TERM_PROGRAM", "MarvisTerminalSpike");
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
        drop(pair.slave);
        let counters = Arc::new(Counters::default());
        let reader_counters = Arc::clone(&counters);
        if let Err(error) = thread::Builder::new()
            .name("terminal-pty-reader".into())
            .spawn(move || read_output(reader, &mut output, reader_counters))
        {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let replaced = self
            .sessions
            .lock()
            .map_err(|_| "terminal session lock poisoned".to_string())?
            .insert(
                id,
                Session {
                    master: pair.master,
                    writer,
                    child,
                    counters,
                },
            );
        debug_assert!(replaced.is_none());
        Ok(id)
    }

    pub fn write(&self, id: TerminalId, bytes: &[u8]) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "terminal session lock poisoned".to_string())?;
        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| format!("unknown terminal {id}"))?;
        session
            .writer
            .write_all(bytes)
            .map_err(|error| error.to_string())?;
        session.writer.flush().map_err(|error| error.to_string())
    }

    pub fn resize(&self, id: TerminalId, cols: u16, rows: u16) -> Result<(u16, u16), String> {
        if cols == 0 || rows == 0 {
            return Err("PTY dimensions must be non-zero".into());
        }
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "terminal session lock poisoned".to_string())?;
        let master = &sessions
            .get(&id)
            .ok_or_else(|| format!("unknown terminal {id}"))?
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

    pub fn metrics(&self, id: TerminalId) -> Result<TerminalMetrics, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "terminal session lock poisoned".to_string())?;
        let counters = &sessions
            .get(&id)
            .ok_or_else(|| format!("unknown terminal {id}"))?
            .counters;
        Ok(TerminalMetrics {
            bytes_read: counters.bytes_read.load(Ordering::Relaxed),
            bytes_sent: counters.bytes_sent.load(Ordering::Relaxed),
            dropped_bytes: counters.dropped_bytes.load(Ordering::Relaxed),
            reader_closed: counters.reader_closed.load(Ordering::Relaxed),
        })
    }

    pub fn process_id(&self, id: TerminalId) -> Result<Option<u32>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "terminal session lock poisoned".to_string())?;
        Ok(sessions
            .get(&id)
            .ok_or_else(|| format!("unknown terminal {id}"))?
            .child
            .process_id())
    }

    pub fn terminate(&self, id: TerminalId) -> Result<(), String> {
        let mut session = self
            .sessions
            .lock()
            .map_err(|_| "terminal session lock poisoned".to_string())?
            .remove(&id)
            .ok_or_else(|| format!("unknown terminal {id}"))?;
        if !matches!(session.child.try_wait(), Ok(Some(_))) {
            let _ = session.child.kill();
        }
        session.child.wait().map_err(|error| error.to_string())?;
        Ok(())
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        if let Ok(sessions) = self.sessions.get_mut() {
            for session in sessions.values_mut() {
                let _ = session.child.kill();
                let _ = session.child.wait();
            }
        }
    }
}

fn read_output(mut reader: Box<dyn Read + Send>, output: &mut OutputSink, counters: Arc<Counters>) {
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                counters
                    .bytes_read
                    .fetch_add(count as u64, Ordering::Relaxed);
                match output(&buffer[..count]) {
                    Ok(()) => {
                        counters
                            .bytes_sent
                            .fetch_add(count as u64, Ordering::Relaxed);
                    }
                    Err(_) => {
                        counters
                            .dropped_bytes
                            .fetch_add(count as u64, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    counters.reader_closed.store(true, Ordering::Relaxed);
}
