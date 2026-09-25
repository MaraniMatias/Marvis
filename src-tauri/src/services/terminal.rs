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

pub struct CreatedTerminal {
    pub session: Session,
    pub workspace: WorkspaceState,
}

pub fn create(
    database: &Database,
    backend: &TerminalBackend,
    checkout_id: &str,
    cols: u16,
    rows: u16,
    output: OutputSink,
) -> Result<CreatedTerminal, String> {
    let cwd = database.terminal_checkout_path(checkout_id)?;
    let program = inherited_shell();
    let session = Session {
        id: format!(
            "session:terminal:{}-{}-{}",
            timestamp(),
            std::process::id(),
            NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
        ),
        session_type: SessionType::Shell,
        checkout_id: checkout_id.to_string(),
        name: program
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "shell".into()),
        created_at: timestamp(),
        status: SessionStatus::Inactive,
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

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use crate::{
        domain::workspace::Repo,
        persistence::Database,
        terminal::{OutputSink, TerminalBackend},
    };

    use super::create;

    fn plain_repo(path: &Path) -> Repo {
        Repo::plain(path, "now").unwrap()
    }

    #[test]
    fn creates_multiple_sessions_only_for_persisted_checkouts_and_marks_them_inactive_after_restart(
    ) {
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
        assert_eq!(restored_checkout.sessions.len(), 2);
        assert!(restored_checkout
            .sessions
            .iter()
            .all(|session| session.status == crate::domain::workspace::SessionStatus::Inactive));
        let restored_other_repo = restored
            .repos
            .iter()
            .find(|item| item.id == other_repo.id)
            .unwrap();
        let restored_other = restored_other_repo.checkouts.first().unwrap();
        assert_eq!(restored_other.id, other_repo.checkouts[0].id);
        assert_eq!(restored_other.sessions.len(), 1);
        assert_eq!(
            restored_other.sessions[0].status,
            crate::domain::workspace::SessionStatus::Inactive
        );
    }
}
