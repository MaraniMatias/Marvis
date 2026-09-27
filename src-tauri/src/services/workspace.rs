use std::{ffi::OsString, fs, path::Path, process::Command};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        workspace::{Repo, RepoKind},
    },
    git,
    persistence::{timestamp, Database},
    services::{folder, worktree},
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

pub fn locate_missing_checkout(
    database: &Database,
    checkout_id: &str,
    selected_path: &Path,
) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let state = database.load_workspace().map_err(operation_error)?;
    let (repo, checkout) = crate::services::checkout::registered_checkout(
        &state.repos,
        checkout_id,
        "checkout ID is not registered",
    )?;
    let selected_path = fs::canonicalize(selected_path).map_err(|error| {
        IpcError::new(
            IpcErrorCode::FolderMissing,
            format!("located directory is unavailable: {error}"),
        )
    })?;
    if !selected_path.is_dir() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "located path is not a directory",
        ));
    }
    if !checkout.is_missing {
        if selected_path != Path::new(&checkout.canonical_path) {
            return Err(IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "only a missing checkout can be located",
            ));
        }
        return register_folder(database, &selected_path);
    }
    if checkout
        .sessions
        .iter()
        .any(|session| session.status == crate::domain::workspace::SessionStatus::Active)
    {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "close active terminal sessions before relocating this checkout",
        ));
    }

    if repo.kind == RepoKind::Plain {
        if git::resolve_repository(&selected_path, &timestamp())?.is_some() {
            return Err(IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "located directory is a Git checkout, not the missing plain directory",
            ));
        }
        let relocated = Repo::plain(&selected_path, timestamp())
            .map_err(|error| IpcError::new(IpcErrorCode::InvalidPath, error))?;
        return database
            .relocate_plain_checkout(&repo.id, checkout_id, &relocated)
            .map_err(operation_error);
    }

    if selected_path == Path::new(&checkout.canonical_path) {
        let (resolved, focus_id) = git::resolve_repository(&selected_path, &timestamp())?
            .ok_or_else(|| {
                IpcError::new(
                    IpcErrorCode::NotRepository,
                    "located directory is not a Git checkout",
                )
            })?;
        if resolved.id != repo.id || focus_id != checkout_id {
            return Err(IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "located directory does not match the missing checkout",
            ));
        }
        return database
            .register_git_repo(resolved, &focus_id)
            .map_err(operation_error);
    }

    if checkout.is_primary {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "plain directories and primary checkouts can only be located at their original path; open a moved directory as a new location",
        ));
    }
    let management_root = database
        .existing_git_checkout(&repo.id)
        .map_err(operation_error)?
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::FolderMissing,
                "no available checkout exists to verify this worktree",
            )
        })?;
    if git_common_dir(&management_root)? != git_common_dir(&selected_path)? {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "located directory belongs to a different Git repository",
        ));
    }
    let repair = Command::new("git")
        .args(["worktree", "repair"])
        .arg(&selected_path)
        .current_dir(&management_root)
        .output()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not repair the located worktree: {error}"),
            )
        })?;
    if !repair.status.success() {
        return Err(IpcError::new(
            IpcErrorCode::GitFailed,
            format!(
                "Git could not repair the located worktree: {}",
                String::from_utf8_lossy(&repair.stderr).trim()
            ),
        ));
    }
    let (resolved, focus_id) =
        git::resolve_repository(&selected_path, &timestamp())?.ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::NotRepository,
                "located directory is not a Git checkout",
            )
        })?;
    if resolved.id != repo.id || !resolved.checkouts.iter().any(|item| item.id == focus_id) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "located directory does not resolve to the repository containing this missing worktree",
        ));
    }
    database
        .locate_git_checkout(&resolved, checkout_id, &focus_id)
        .map_err(operation_error)
}

fn git_common_dir(path: &Path) -> Result<std::path::PathBuf, IpcError> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(path)
        .output()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not verify Git worktree ownership: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "located directory does not contain Git worktree metadata",
        ));
    }
    let common_dir_text = String::from_utf8_lossy(&output.stdout);
    let common_dir = Path::new(common_dir_text.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir.to_path_buf()
    } else {
        path.join(common_dir)
    };
    common_dir.canonicalize().map_err(|error| {
        IpcError::new(
            IpcErrorCode::InvalidCheckout,
            format!("could not resolve Git worktree ownership: {error}"),
        )
    })
}

pub fn close_missing_checkout(
    database: &Database,
    backend: &crate::terminal::TerminalBackend,
    checkout_id: &str,
) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let state = database.load_workspace().map_err(operation_error)?;
    let (repo, checkout) = crate::services::checkout::registered_checkout(
        &state.repos,
        checkout_id,
        "checkout ID is not registered",
    )?;
    if !checkout.is_missing {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "only a missing checkout can be closed",
        ));
    }
    if repo.kind == RepoKind::Git && !checkout.is_primary {
        if checkout
            .sessions
            .iter()
            .any(|session| session.status == crate::domain::workspace::SessionStatus::Active)
        {
            return Err(IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "close active terminal sessions before closing this missing worktree",
            ));
        }
        let info = worktree::removal_info(database, checkout_id)?;
        let confirmation = worktree::WorktreeRemovalConfirmation {
            confirm_dirty: false,
            confirmed_dirty_files: info.dirty_files,
            confirmed_session_ids: Vec::new(),
            expected_branch: info.branch,
            expected_unmerged_commits: info.unmerged_commits,
            delete_branch: false,
        };
        return worktree::remove(database, backend, checkout_id, &confirmation)
            .map(|removed| removed.workspace);
    }
    database
        .close_missing_checkout(checkout_id)
        .map_err(operation_error)
}

fn operation_error(error: String) -> IpcError {
    IpcError::new(IpcErrorCode::OperationFailed, error)
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
    // Reconciliation re-inserts the checkouts Git still lists, including the ones whose
    // directory is gone, so the prune runs last: what comes back is what is on disk.
    database
        .prune_missing_checkouts()
        .map_err(operation_error)?;
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
        domain::{
            terminal_layout::{CheckoutTerminalLayout, TerminalLayoutNode, TerminalLayoutTab},
            workspace::{RepoKind, Session, SessionStatus, SessionType},
        },
        persistence::Database,
    };

    use super::{
        close_missing_checkout, locate_missing_checkout, register_folder, restore,
        set_default_branch,
    };

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
    fn restore_prunes_a_worktree_removed_while_the_app_was_closed() {
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
        let focused = register_folder(&db, &linked).unwrap();
        let removed_id = focused
            .active_checkout_id
            .clone()
            .expect("opening the worktree selects it");
        fs::remove_dir_all(&linked).unwrap();

        let state = restore(&db).unwrap();

        // Git still lists the deleted worktree, so reconciliation re-inserts it first. The
        // prune is what runs last, and the worktree is gone from the reopened list.
        assert!(state.repos[0]
            .checkouts
            .iter()
            .all(|checkout| checkout.canonical_path != linked_canonical));
        assert_eq!(state.repos[0].checkouts.len(), 1);
        // The selection that pointed at the removed worktree falls back to the repo's
        // primary instead of naming a row that no longer exists.
        assert_eq!(
            state.active_checkout_id.as_deref(),
            Some(state.repos[0].checkouts[0].id.as_str())
        );
        assert!(state.repos[0].checkouts[0].is_primary);
        assert_ne!(
            state.active_checkout_id.as_deref(),
            Some(removed_id.as_str())
        );
    }

    #[test]
    fn restore_removes_a_repository_whose_root_is_gone() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        init_repo(&primary);
        let db = database(temp.path());
        register_folder(&db, &primary).unwrap();
        fs::remove_dir_all(&primary).unwrap();

        let state = restore(&db).unwrap();

        assert!(state.repos.is_empty());
        assert_eq!(state.active_checkout_id, None);
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

    #[test]
    fn locating_a_moved_worktree_repairs_git_and_migrates_its_checkout_metadata() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let moved_from = temp.path().join("worktree-old");
        let moved_to = temp.path().join("worktree-new");
        init_repo(&primary);
        git(
            &primary,
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                moved_from.to_str().unwrap(),
            ],
        );
        let database = database(temp.path());
        let registered = register_folder(&database, &primary).unwrap();
        let old_id = register_folder(&database, &moved_from)
            .unwrap()
            .active_checkout_id
            .unwrap();
        let session = Session {
            id: "session:feature-shell".into(),
            session_type: SessionType::Shell,
            checkout_id: old_id.clone(),
            name: "zsh".into(),
            created_at: "now".into(),
            status: SessionStatus::Active,
        };
        database.add_terminal_session(&session).unwrap();
        let layout = CheckoutTerminalLayout {
            active_tab_id: Some("tab:feature".into()),
            tabs: vec![TerminalLayoutTab {
                id: "tab:feature".into(),
                root: TerminalLayoutNode::Session {
                    session_id: session.id.clone(),
                },
            }],
            session_order: vec![session.id.clone()],
        };
        database.save_terminal_layout(&old_id, &layout).unwrap();
        database.mark_file_viewed(&old_id, "README.md").unwrap();
        drop(database);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        fs::rename(&moved_from, &moved_to).unwrap();

        let located = locate_missing_checkout(&database, &old_id, &moved_to).unwrap();
        let repo = &located.repos[0];
        let new_id = crate::domain::workspace::checkout_id_for_path(
            &moved_to.canonicalize().unwrap().display().to_string(),
        );
        assert_eq!(repo.checkouts.len(), 2);
        assert!(repo.checkouts.iter().all(|checkout| checkout.id != old_id));
        let checkout = repo
            .checkouts
            .iter()
            .find(|checkout| checkout.id == new_id)
            .unwrap();
        assert_eq!(checkout.branch.as_deref(), Some("feature"));
        assert!(!checkout.is_missing);
        assert_eq!(
            checkout.sessions,
            [Session {
                checkout_id: new_id.clone(),
                status: SessionStatus::Inactive,
                ..session
            }]
        );
        assert_eq!(located.active_checkout_id.as_deref(), Some(new_id.as_str()));
        assert_eq!(
            located.active_session_id.as_deref(),
            Some("session:feature-shell")
        );
        assert_eq!(
            database.load_terminal_layout(&new_id).unwrap(),
            Some(layout)
        );
        assert_eq!(database.viewed_files(&new_id).unwrap(), ["README.md"]);
        assert_eq!(repo.id, registered.repos[0].id);
    }

    #[test]
    fn missing_plain_checkout_can_only_be_located_at_its_saved_path() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("plain");
        fs::create_dir(&path).unwrap();
        let database = database(temp.path());
        let state = register_folder(&database, &path).unwrap();
        let checkout_id = state.repos[0].checkouts[0].id.clone();
        fs::remove_dir(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let other = temp.path().join("other");
        fs::create_dir(&other).unwrap();

        assert!(locate_missing_checkout(&database, &checkout_id, &other).is_err());
        let restored = locate_missing_checkout(&database, &checkout_id, &path).unwrap();
        assert!(!restored.repos[0].checkouts[0].is_missing);
    }

    #[test]
    fn locating_a_moved_plain_directory_rekeys_the_workspace_and_preserves_history() {
        let temp = tempdir().unwrap();
        let old_path = temp.path().join("plain-old");
        let new_path = temp.path().join("plain-new");
        fs::create_dir(&old_path).unwrap();
        let database = database(temp.path());
        let state = register_folder(&database, &old_path).unwrap();
        let old_id = state.repos[0].checkouts[0].id.clone();
        let session = Session {
            id: "session:plain-shell".into(),
            session_type: SessionType::Shell,
            checkout_id: old_id.clone(),
            name: "zsh".into(),
            created_at: "now".into(),
            status: SessionStatus::Active,
        };
        database.add_terminal_session(&session).unwrap();
        database.mark_file_viewed(&old_id, "notes.md").unwrap();
        drop(database);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        fs::rename(&old_path, &new_path).unwrap();

        let relocated = locate_missing_checkout(&database, &old_id, &new_path).unwrap();
        let repo = &relocated.repos[0];
        let new_id = crate::domain::workspace::checkout_id_for_path(
            &new_path.canonicalize().unwrap().display().to_string(),
        );
        assert_eq!(
            repo.id,
            crate::domain::workspace::repo_id_for_path(&repo.root)
        );
        assert_eq!(
            repo.root,
            new_path.canonicalize().unwrap().display().to_string()
        );
        assert!(repo.checkouts.iter().all(|checkout| checkout.id != old_id));
        assert_eq!(repo.checkouts[0].id, new_id);
        assert_eq!(repo.checkouts[0].sessions[0].checkout_id, new_id);
        assert_eq!(
            repo.checkouts[0].sessions[0].status,
            SessionStatus::Inactive
        );
        assert_eq!(database.viewed_files(&new_id).unwrap(), ["notes.md"]);
        assert_eq!(
            relocated.active_checkout_id.as_deref(),
            Some(new_id.as_str())
        );
        assert_eq!(
            relocated.active_session_id.as_deref(),
            Some("session:plain-shell")
        );
    }

    #[test]
    fn closing_a_missing_location_removes_only_its_workspace_registration() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("plain");
        fs::create_dir(&path).unwrap();
        let database = database(temp.path());
        let state = register_folder(&database, &path).unwrap();
        let checkout_id = state.repos[0].checkouts[0].id.clone();
        fs::remove_dir(&path).unwrap();

        let closed = close_missing_checkout(
            &database,
            &crate::terminal::TerminalBackend::default(),
            &checkout_id,
        )
        .unwrap();
        assert!(closed.repos.is_empty());
    }

    #[test]
    fn closing_a_missing_git_worktree_drops_its_entry_and_leaves_the_disk_alone() {
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
        let database = database(temp.path());
        let registered = register_folder(&database, &primary).unwrap();
        let primary_id = registered.repos[0].checkouts[0].id.clone();
        let linked_id = register_folder(&database, &linked)
            .unwrap()
            .active_checkout_id
            .expect("opening the worktree selects it");
        database
            .add_active_session(&Session {
                id: "session:gone".into(),
                session_type: SessionType::Shell,
                checkout_id: linked_id.clone(),
                name: "shell".into(),
                created_at: "now".into(),
                status: SessionStatus::Active,
            })
            .unwrap();
        fs::remove_dir_all(&linked).unwrap();
        let backend = crate::terminal::TerminalBackend::default();

        // A live session still blocks the close, exactly as it does for a worktree on disk.
        let refused = close_missing_checkout(&database, &backend, &linked_id).unwrap_err();
        assert!(refused.message.contains("active terminal sessions"));
        assert_eq!(
            database.load_workspace().unwrap().repos[0].checkouts.len(),
            2
        );

        database.remove_terminal_session("session:gone").unwrap();
        let closed = close_missing_checkout(&database, &backend, &linked_id).unwrap();

        // Only the entry leaves the list: the repository, its file and its branch stay.
        assert_eq!(closed.repos[0].checkouts.len(), 1);
        assert_eq!(closed.repos[0].checkouts[0].id, primary_id);
        assert_eq!(
            closed.active_checkout_id.as_deref(),
            Some(closed.repos[0].checkouts[0].id.as_str())
        );
        assert!(primary.join("tracked.txt").exists());
        git(
            &primary,
            &["show-ref", "--verify", "--quiet", "refs/heads/temporary"],
        );
    }

    #[test]
    fn closing_a_location_that_still_exists_is_refused() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("plain");
        fs::create_dir(&path).unwrap();
        let database = database(temp.path());
        let state = register_folder(&database, &path).unwrap();
        let checkout_id = state.repos[0].checkouts[0].id.clone();

        let error = close_missing_checkout(
            &database,
            &crate::terminal::TerminalBackend::default(),
            &checkout_id,
        )
        .unwrap_err();

        assert!(error
            .message
            .contains("only a missing checkout can be closed"));
        assert_eq!(database.load_workspace().unwrap().repos.len(), 1);
    }

    #[test]
    fn closing_a_missing_checkout_is_refused_while_its_session_is_active() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("plain");
        fs::create_dir(&path).unwrap();
        let database = database(temp.path());
        let state = register_folder(&database, &path).unwrap();
        let checkout_id = state.repos[0].checkouts[0].id.clone();
        database
            .add_terminal_session(&Session {
                id: "session:missing".into(),
                session_type: SessionType::Shell,
                checkout_id: checkout_id.clone(),
                name: "shell".into(),
                created_at: "now".into(),
                status: SessionStatus::Active,
            })
            .unwrap();
        fs::remove_dir(&path).unwrap();

        let error = close_missing_checkout(
            &database,
            &crate::terminal::TerminalBackend::default(),
            &checkout_id,
        )
        .unwrap_err();
        assert!(error.message.contains("active terminal sessions"));
        assert_eq!(database.load_workspace().unwrap().repos.len(), 1);
    }
}
