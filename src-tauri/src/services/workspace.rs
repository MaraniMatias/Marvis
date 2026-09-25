use std::{ffi::OsString, path::Path, process::Command};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        workspace::{Repo, RepoKind},
    },
    git,
    persistence::{timestamp, Database},
    services::folder,
};

pub fn register_folder(
    database: &Database,
    path: &Path,
) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let opened = folder::open_folder(path)?;
    let now = timestamp();
    if let Some((repo, focus_checkout_id)) = git::resolve_repository(Path::new(&opened.path), &now)?
    {
        database
            .register_git_repo(repo, &focus_checkout_id)
            .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
    } else {
        let repo = Repo::plain(Path::new(&opened.path), now)
            .map_err(|error| IpcError::new(IpcErrorCode::InvalidPath, error))?;
        database
            .register_plain_repo(repo)
            .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
    }
}

pub fn restore(database: &Database) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let state = database
        .load_workspace()
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?;
    for repo in state.repos.iter().filter(|repo| repo.kind == RepoKind::Git) {
        let Some(path) = database
            .existing_git_checkout(&repo.id)
            .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?
        else {
            continue;
        };
        if let Ok(Some((resolved, _))) = git::resolve_repository(&path, &timestamp()) {
            if resolved.id == repo.id {
                database
                    .reconcile_git_repo(&resolved)
                    .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?;
            }
        }
    }
    database
        .load_workspace()
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
}

pub fn set_default_branch(
    database: &Database,
    repo_id: &str,
    branch: &str,
) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let branch = branch
        .trim()
        .strip_prefix("origin/")
        .unwrap_or(branch.trim());
    if branch.is_empty() {
        return Err(IpcError::new(
            IpcErrorCode::DefaultBranchUnknown,
            "default branch cannot be empty",
        ));
    }
    let root = database
        .existing_git_checkout(repo_id)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "no available checkout exists for this repository",
            )
        })?;
    let ref_name = format!("refs/heads/{branch}");
    let valid = Command::new("git")
        .args([OsString::from("check-ref-format"), ref_name.clone().into()])
        .current_dir(&root)
        .status()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not validate Git branch: {error}"),
            )
        })?
        .success();
    let exists = [ref_name, format!("refs/remotes/origin/{branch}")]
        .iter()
        .any(|reference| {
            Command::new("git")
                .args(["show-ref", "--verify", "--quiet", reference])
                .current_dir(&root)
                .status()
                .is_ok_and(|status| status.success())
        });
    if !valid || !exists {
        return Err(IpcError::new(
            IpcErrorCode::DefaultBranchUnknown,
            format!("Git branch '{branch}' does not exist or is not a valid branch name"),
        ));
    }
    database
        .set_default_branch(repo_id, branch)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use tempfile::tempdir;

    use crate::{
        domain::workspace::{RepoKind, Session, SessionStatus, SessionType},
        persistence::Database,
    };

    use super::{register_folder, restore, set_default_branch};

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("Git is installed");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "-b", "trunk"]);
        git(path, &["config", "user.name", "Marvis test"]);
        git(path, &["config", "user.email", "marvis@example.invalid"]);
        fs::write(path.join("tracked.txt"), "initial\n").unwrap();
        git(path, &["add", "tracked.txt"]);
        git(path, &["commit", "-m", "initial"]);
    }

    fn database(path: &Path) -> Database {
        Database::open(path.join("workspace.sqlite3")).expect("database")
    }

    #[test]
    fn opening_a_worktree_registers_primary_first_and_focuses_that_worktree() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo with spaces");
        let linked = temp.path().join("feature checkout");
        init_repo(&primary);
        git(
            &primary,
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        );

        let db = database(temp.path());
        let state = register_folder(&db, &linked).unwrap();

        assert_eq!(state.repos.len(), 1);
        let repo = &state.repos[0];
        assert_eq!(repo.kind, RepoKind::Git);
        assert_eq!(repo.checkouts.len(), 2);
        assert!(repo.checkouts[0].is_primary);
        assert_eq!(
            repo.checkouts[0].canonical_path,
            primary.canonicalize().unwrap().display().to_string()
        );
        assert_eq!(repo.checkouts[1].branch.as_deref(), Some("feature"));
        assert_eq!(
            state.active_checkout_id.as_deref(),
            Some(repo.checkouts[1].id.as_str())
        );
        assert_eq!(
            repo.default_branch, None,
            "no origin/HEAD must not assume trunk"
        );
        let chosen = set_default_branch(&db, &repo.id, "trunk").unwrap();
        assert_eq!(chosen.repos[0].default_branch.as_deref(), Some("trunk"));
        let reopened = register_folder(&db, &linked).unwrap();
        assert_eq!(reopened.repos[0].default_branch.as_deref(), Some("trunk"));
    }

    #[test]
    fn origin_head_is_used_and_a_manual_default_branch_is_persisted() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let remote = temp.path().join("origin.git");
        init_repo(&primary);
        fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare"]);
        git(
            &primary,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&primary, &["push", "-u", "origin", "trunk"]);
        git(
            &primary,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/trunk",
            ],
        );
        let db = database(temp.path());

        let detected = register_folder(&db, &primary).unwrap();

        assert_eq!(detected.repos[0].default_branch.as_deref(), Some("trunk"));
        let picked = set_default_branch(&db, &detected.repos[0].id, "trunk").unwrap();
        assert_eq!(picked.repos[0].default_branch.as_deref(), Some("trunk"));
        assert!(set_default_branch(&db, &detected.repos[0].id, "not-a-branch").is_err());
    }

    #[test]
    fn opening_subdirectories_and_symlinks_reuses_repo_and_focuses_containing_checkout() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("linked");
        init_repo(&primary);
        git(
            &primary,
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        );
        let nested = linked.join("nested");
        fs::create_dir(&nested).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&nested, temp.path().join("nested alias")).unwrap();

        let db = database(temp.path());
        let first = register_folder(&db, &primary).unwrap();
        let second = register_folder(&db, &nested).unwrap();
        #[cfg(unix)]
        let third = register_folder(&db, &temp.path().join("nested alias")).unwrap();

        assert_eq!(second.repos.len(), 1);
        assert_eq!(second.repos[0].checkouts.len(), 2);
        assert_eq!(
            second.active_checkout_id.as_deref(),
            Some(first.repos[0].checkouts[1].id.as_str())
        );
        #[cfg(unix)]
        assert_eq!(third.repos.len(), 1);
        #[cfg(unix)]
        assert_eq!(third.active_checkout_id, second.active_checkout_id);
    }

    #[test]
    fn reopening_a_repo_does_not_duplicate_repo_or_checkout_rows() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let db = database(temp.path());

        register_folder(&db, &root).unwrap();
        let reopened = register_folder(&db, &root).unwrap();

        assert_eq!(reopened.repos.len(), 1);
        assert_eq!(reopened.repos[0].checkouts.len(), 1);
    }

    #[test]
    fn restart_restores_checkout_order_focus_and_historical_session_selection() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("feature checkout");
        let database_path = temp.path().join("workspace.sqlite3");
        init_repo(&primary);
        git(
            &primary,
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        );

        let session = Session {
            id: "session:feature-shell".into(),
            session_type: SessionType::Shell,
            checkout_id: crate::domain::workspace::checkout_id_for_path(
                &linked.canonicalize().unwrap().display().to_string(),
            ),
            name: "zsh · feature".into(),
            created_at: "saved-at".into(),
            status: SessionStatus::Inactive,
        };
        {
            let database = Database::open(&database_path).unwrap();
            let opened = register_folder(&database, &primary).unwrap();
            let focused = register_folder(&database, &linked).unwrap();
            assert_eq!(
                focused.repos[0].checkouts[0].id,
                opened.repos[0].checkouts[0].id
            );
            assert_eq!(
                focused.active_checkout_id.as_deref(),
                Some(session.checkout_id.as_str())
            );
            database.add_terminal_session(&session).unwrap();

            let primary_id = focused.repos[0].checkouts[0].id.clone();
            let switched = database.select_checkout(Some(primary_id)).unwrap();
            assert_eq!(switched.active_session_id, None);
            let selected = database.select_session(Some(session.id.clone())).unwrap();
            assert_eq!(
                selected.active_checkout_id.as_deref(),
                Some(session.checkout_id.as_str())
            );
            assert_eq!(
                selected.active_session_id.as_deref(),
                Some(session.id.as_str())
            );
        }

        let database = Database::open(&database_path).unwrap();
        let restored = restore(&database).unwrap();
        let repo = &restored.repos[0];
        assert_eq!(repo.checkouts.len(), 2);
        assert!(repo.checkouts[0].is_primary);
        assert_eq!(repo.checkouts[1].branch.as_deref(), Some("feature"));
        assert_eq!(
            restored.active_checkout_id.as_deref(),
            Some(session.checkout_id.as_str())
        );
        assert_eq!(
            restored.active_session_id.as_deref(),
            Some(session.id.as_str())
        );
        let restored_session = &repo.checkouts[1].sessions[0];
        assert_eq!(restored_session.name, session.name);
        assert_eq!(restored_session.created_at, session.created_at);
        assert_eq!(restored_session.status, SessionStatus::Inactive);
    }

    #[test]
    fn restore_reconciles_worktrees_and_keeps_removed_checkout_visible_as_missing() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("gone");
        init_repo(&primary);
        git(
            &primary,
            &[
                "worktree",
                "add",
                "-b",
                "temporary",
                linked.to_str().unwrap(),
            ],
        );
        let linked_canonical = linked.canonicalize().unwrap().display().to_string();
        let db = database(temp.path());
        register_folder(&db, &primary).unwrap();
        fs::remove_dir_all(&linked).unwrap();

        let state = restore(&db).unwrap();

        let missing = state.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.canonical_path == linked_canonical)
            .unwrap_or_else(|| panic!("missing worktree stays visible: {:#?}", state.repos[0]));
        assert!(missing.is_missing);
    }

    #[test]
    fn restore_discovers_worktrees_created_outside_the_app() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("external worktree");
        init_repo(&primary);
        let db = database(temp.path());
        register_folder(&db, &primary).unwrap();
        git(
            &primary,
            &[
                "worktree",
                "add",
                "-b",
                "external",
                linked.to_str().unwrap(),
            ],
        );

        let restored = restore(&db).unwrap();

        assert_eq!(restored.repos.len(), 1);
        assert_eq!(restored.repos[0].checkouts.len(), 2);
        assert_eq!(
            restored.repos[0].checkouts[1].branch.as_deref(),
            Some("external")
        );
    }

    #[test]
    fn plain_directory_stays_a_single_plain_checkout() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("plain");
        fs::create_dir(&path).unwrap();

        let state = register_folder(&database(temp.path()), &path).unwrap();

        assert_eq!(state.repos.len(), 1);
        assert_eq!(state.repos[0].kind, RepoKind::Plain);
        assert_eq!(state.repos[0].checkouts.len(), 1);
        assert!(state.repos[0].checkouts[0].is_primary);
    }

    #[test]
    fn explicitly_opened_submodule_is_standalone_and_bare_repository_is_rejected() {
        let temp = tempdir().unwrap();
        let child = temp.path().join("child");
        let parent = temp.path().join("parent");
        let bare = temp.path().join("bare.git");
        init_repo(&child);
        init_repo(&parent);
        git(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                child.to_str().unwrap(),
                "vendor/child",
            ],
        );
        git(&parent, &["commit", "-am", "add submodule"]);
        fs::create_dir(&bare).unwrap();
        git(&bare, &["init", "--bare"]);

        let state = register_folder(&database(temp.path()), &parent.join("vendor/child")).unwrap();

        assert_eq!(state.repos.len(), 1);
        assert_eq!(
            state.repos[0].root,
            parent
                .join("vendor/child")
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(state.repos[0].checkouts.len(), 1);
        let error = register_folder(&database(temp.path()), &bare).unwrap_err();
        assert!(error
            .message
            .contains("Bare repositories are not supported"));
    }
}
