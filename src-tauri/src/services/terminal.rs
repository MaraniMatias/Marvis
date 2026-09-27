use std::{
    env,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    domain::workspace::{Session, SessionStatus, SessionType, WorkspaceState},
    persistence::{timestamp, Database},
    terminal::{OutputSink, SpawnOptions, TerminalBackend},
};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Long enough for a branch-like name, short enough that a sidebar row never has to scroll
/// to show what a terminal is.
const MAX_NAME_CHARS: usize = 60;

pub struct CreatedTerminal {
    pub session: Session,
    pub workspace: WorkspaceState,
}

pub struct TerminalOptions {
    pub cols: u16,
    pub rows: u16,
    pub prompt: Option<String>,
}

pub fn create(
    database: &Database,
    backend: &TerminalBackend,
    checkout_id: &str,
    cols: u16,
    rows: u16,
    output: OutputSink,
) -> Result<CreatedTerminal, String> {
    create_with_options(
        database,
        backend,
        checkout_id,
        TerminalOptions {
            cols,
            rows,
            prompt: None,
        },
        output,
    )
}

pub fn create_with_options(
    database: &Database,
    backend: &TerminalBackend,
    checkout_id: &str,
    options: TerminalOptions,
    output: OutputSink,
) -> Result<CreatedTerminal, String> {
    let TerminalOptions { cols, rows, prompt } = options;
    let cwd = database.terminal_checkout_path(checkout_id)?;
    reject_prompt(prompt.as_deref())?;
    let program = inherited_shell();
    let name = program
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "shell".into());
    let session = Session {
        id: format!(
            "session:terminal:{}-{}-{}",
            timestamp(),
            std::process::id(),
            NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
        ),
        session_type: SessionType::Shell,
        checkout_id: checkout_id.to_string(),
        name,
        created_at: timestamp(),
        status: SessionStatus::Active,
    };

    backend.spawn(
        session.id.clone(),
        SpawnOptions {
            program,
            args: vec!["-l".into()],
            cwd,
            cols,
            rows,
        },
        output,
    )?;

    match database.add_terminal_session(&session) {
        Ok(workspace) => Ok(CreatedTerminal { session, workspace }),
        Err(error) => {
            let _ = backend.close(&session.id);
            Err(error)
        }
    }
}

/// Renames a session, so a row the user cannot tell apart from its neighbours gets a name.
///
/// The name is a label and nothing more: it is never a path, an argument or a command, so the
/// only rules are that it is something to read and short enough for a sidebar row.
pub fn rename(database: &Database, session_id: &str, name: &str) -> Result<WorkspaceState, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a session needs a name".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "a session name is at most {MAX_NAME_CHARS} characters"
        ));
    }
    // Control characters would break the row they are drawn in, and a newline would draw a
    // second one. Neither is a name anyone means to type.
    if name.chars().any(char::is_control) {
        return Err("a session name cannot contain control characters".into());
    }
    database.rename_terminal_session(session_id, name)
}

pub fn close(
    database: &Database,
    backend: &TerminalBackend,
    session_id: &str,
) -> Result<WorkspaceState, String> {
    backend.close(session_id)?;
    database.remove_terminal_session(session_id)
}

fn inherited_shell() -> PathBuf {
    env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            ["/bin/zsh", "/bin/bash", "/bin/sh"]
                .into_iter()
                .map(PathBuf::from)
                .find(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// Terminals take no prompt any more: the review goes to the agent bridge, not a PTY.
fn reject_prompt(prompt: Option<&str>) -> Result<(), String> {
    match prompt {
        None => Ok(()),
        Some(_) => Err("a terminal session cannot carry a prompt".into()),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use crate::{
        domain::workspace::Repo,
        persistence::Database,
        terminal::{OutputSink, TerminalBackend},
    };

    use super::{create, create_with_options, rename, TerminalOptions};

    fn plain_repo(path: &Path) -> Repo {
        Repo::plain(path, "now").unwrap()
    }

    #[test]
    fn creates_multiple_sessions_only_for_persisted_checkouts_and_forgets_them_on_restart() {
        let directory = tempdir().unwrap();
        let checkout = directory.path().join("checkout");
        let other_checkout = directory.path().join("other-checkout");
        fs::create_dir(&checkout).unwrap();
        fs::create_dir(&other_checkout).unwrap();
        let database_path = directory.path().join("workspace.sqlite3");
        let repo = plain_repo(&checkout);
        let other_repo = plain_repo(&other_checkout);
        let database = Database::open(&database_path).unwrap();
        database.register_plain_repo(repo.clone()).unwrap();
        database.register_plain_repo(other_repo.clone()).unwrap();
        let backend = TerminalBackend::default();
        let output: OutputSink = Box::new(|_| Ok(()));

        assert!(create(
            &database,
            &backend,
            "checkout:unknown",
            80,
            24,
            Box::new(|_| Ok(()))
        )
        .is_err());
        let first = create(&database, &backend, &repo.checkouts[0].id, 80, 24, output).unwrap();
        let second = create(
            &database,
            &backend,
            &repo.checkouts[0].id,
            80,
            24,
            Box::new(|_| Ok(())),
        )
        .unwrap();
        assert_ne!(first.session.id, second.session.id);
        assert_eq!(
            first.session.status,
            crate::domain::workspace::SessionStatus::Active
        );
        assert_eq!(second.workspace.repos[0].checkouts[0].sessions.len(), 2);
        assert_eq!(
            second.workspace.active_session_id.as_deref(),
            Some(second.session.id.as_str())
        );
        let other = create(
            &database,
            &backend,
            &other_repo.checkouts[0].id,
            80,
            24,
            Box::new(|_| Ok(())),
        )
        .unwrap();
        assert_eq!(other.session.checkout_id, other_repo.checkouts[0].id);
        assert_eq!(
            other.workspace.active_checkout_id.as_deref(),
            Some(other_repo.checkouts[0].id.as_str())
        );

        backend.close(&first.session.id).unwrap();
        backend.close(&second.session.id).unwrap();
        backend.close(&other.session.id).unwrap();
        drop(database);
        // The PTYs died with the processes above, so the reopened database keeps the checkouts
        // and none of the sessions.
        let restored = Database::open(database_path)
            .unwrap()
            .load_workspace()
            .unwrap();
        let restored_first = restored
            .repos
            .iter()
            .find(|item| item.id == repo.id)
            .unwrap();
        let restored_checkout = restored_first
            .checkouts
            .iter()
            .find(|checkout| checkout.id == repo.checkouts[0].id)
            .unwrap();
        assert!(restored_checkout.sessions.is_empty());
        let restored_other_repo = restored
            .repos
            .iter()
            .find(|item| item.id == other_repo.id)
            .unwrap();
        let restored_other = restored_other_repo.checkouts.first().unwrap();
        assert_eq!(restored_other.id, other_repo.checkouts[0].id);
        assert!(restored_other.sessions.is_empty());
    }

    #[test]
    fn a_rename_is_a_label_and_nothing_else() {
        let directory = tempdir().unwrap();
        let checkout = directory.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let database = Database::open(directory.path().join("workspace.sqlite3")).unwrap();
        let repo = plain_repo(&checkout);
        database.register_plain_repo(repo.clone()).unwrap();
        let session = crate::domain::workspace::Session {
            id: "session:rename".into(),
            session_type: crate::domain::workspace::SessionType::Shell,
            checkout_id: repo.checkouts[0].id.clone(),
            name: "zsh".into(),
            created_at: "now".into(),
            status: crate::domain::workspace::SessionStatus::Active,
        };
        database.add_terminal_session(&session).unwrap();

        // What the user typed around a name is not part of it.
        let renamed = rename(&database, &session.id, "  build logs \n").unwrap();
        assert_eq!(renamed.repos[0].checkouts[0].sessions[0].name, "build logs");

        // An empty name would leave a row with nothing on it to click.
        assert!(rename(&database, &session.id, "   ").is_err());
        // A newline would draw a second row inside the first.
        assert!(rename(&database, &session.id, "logs\nrm -rf /").is_err());
        // A name long enough to be a payload is refused, not truncated.
        assert!(rename(&database, &session.id, &"x".repeat(61)).is_err());
        assert!(rename(&database, &session.id, &"x".repeat(60)).is_ok());
        // A session the database does not hold is not renamed, whatever it is called.
        assert!(rename(&database, "session:unknown", "anything").is_err());
    }

    #[test]
    fn terminals_reject_a_prompt_because_the_review_goes_to_the_agent_bridge() {
        let directory = tempdir().unwrap();
        let checkout = directory.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let database = Database::open(directory.path().join("workspace.sqlite3")).unwrap();
        let repo = plain_repo(&checkout);
        database.register_plain_repo(repo.clone()).unwrap();
        let backend = TerminalBackend::default();

        assert!(create_with_options(
            &database,
            &backend,
            &repo.checkouts[0].id,
            TerminalOptions {
                cols: 80,
                rows: 24,
                prompt: Some("injected".into()),
            },
            Box::new(|_| Ok(())),
        )
        .is_err());
    }
}
