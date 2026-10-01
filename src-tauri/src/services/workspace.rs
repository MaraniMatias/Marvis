use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        workspace::{RecentPath, Repo, RepoKind},
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

pub fn list_recent_paths(database: &Database) -> Result<Vec<RecentPath>, IpcError> {
    database.list_recent_paths().map_err(operation_error)
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

/// Takes a workdir off the panel, and leaves the disk exactly as it was.
///
/// A workdir that is still there loses its registration and nothing else: the branch, the
/// commits and the files are untouched, so opening the folder again brings it back. A workdir
/// whose directory is gone has nothing left to keep, so it goes through
/// [`close_missing_checkout`], which also prunes the Git worktree that pointed at the
/// directory that is no longer there.
pub fn close_checkout(
    database: &Database,
    backend: &crate::terminal::TerminalBackend,
    checkout_id: &str,
) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let state = database.load_workspace().map_err(operation_error)?;
    let (_, checkout) = crate::services::checkout::registered_checkout(
        &state.repos,
        checkout_id,
        "checkout ID is not registered",
    )?;
    if checkout.is_missing {
        return close_missing_checkout(database, backend, checkout_id);
    }
    database
        .close_checkout(checkout_id)
        .map_err(operation_error)
}

/// Takes a worktree off the panel and keeps it, so the repo root can offer it back.
///
/// Archiving is the reversible half of leaving a workdir: the row stays registered and the
/// disk is untouched, and [`restore_archived_worktrees`] is the way back. Closing is the
/// other half and forgets the checkout entirely.
pub fn archive_checkout(
    database: &Database,
    checkout_id: &str,
) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let state = database.load_workspace().map_err(operation_error)?;
    crate::services::checkout::registered_checkout(
        &state.repos,
        checkout_id,
        "checkout ID is not registered",
    )?;
    database
        .archive_checkout(checkout_id)
        .map_err(operation_error)
}

/// Puts back every worktree this repository archived, so one action brings the whole set
/// to the panel rather than making the user walk them one at a time.
pub fn restore_archived_worktrees(
    database: &Database,
    repo_id: &str,
) -> Result<crate::domain::workspace::WorkspaceState, IpcError> {
    let state = database.load_workspace().map_err(operation_error)?;
    if !state.repos.iter().any(|repo| repo.id == repo_id) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "repository ID is not registered",
        ));
    }
    database
        .restore_archived_worktrees(repo_id)
        .map_err(operation_error)
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

/// Reads one repository's worktrees again and registers the ones Git now lists.
///
/// More than one hand adds a worktree: Marvis's own dialog, an agent running `git worktree add`
/// in a terminal, a script. The disk is what they all wrote to and what the panel has to agree
/// with, so this asks Git for one repository's list and reconciles the answer — the same reading
/// [`restore`] takes for every repository, asked for one because a change said this one moved.
///
/// The archived stay archived: reconciliation writes what Git lists and leaves every other
/// column of a checkout it already knows alone, so a worktree the user put on the shelf is not
/// brought back by another hand adding a sibling.
pub fn sync_repo(
    database: &Database,
    repo_id: &str,
) -> Result<Option<crate::domain::workspace::WorkspaceState>, IpcError> {
    let state = database
        .load_workspace()
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?;
    let repo = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id && repo.kind == RepoKind::Git)
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "Git repository is not registered",
            )
        })?;
    let Some(path) = repo
        .checkouts
        .iter()
        .map(|checkout| PathBuf::from(&checkout.canonical_path))
        .chain(
            state
                .archived_worktrees
                .iter()
                .filter(|checkout| checkout.repo_id == repo_id)
                .map(|checkout| PathBuf::from(&checkout.path)),
        )
        .find(|path| path.is_dir())
    else {
        return Ok(None);
    };

    // Git lists all live and missing worktrees; the workspace splits those same rows between
    // the panel and the archive shelf. Compare just their paths first: most filesystem events
    // near `.git/worktrees` are not a membership change, and one `worktree list` is cheaper than
    // resolving every checkout, updating the database and returning the workspace again.
    let registered: BTreeSet<PathBuf> = repo
        .checkouts
        .iter()
        .map(|checkout| PathBuf::from(&checkout.canonical_path))
        .chain(
            state
                .archived_worktrees
                .iter()
                .filter(|checkout| checkout.repo_id == repo_id)
                .map(|checkout| PathBuf::from(&checkout.path)),
        )
        .collect();
    if worktree::registered_paths(&path)? == registered {
        return Ok(None);
    }

    // A membership change is a reason to reconcile, not a reason to ignore Git errors. Unlike
    // startup's best-effort scan, this is a live request with a panel waiting for its answer.
    let Some(resolved) = resolve_repo(database, repo_id)? else {
        return Ok(None);
    };
    database
        .reconcile_git_repo(&resolved)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?;
    database
        .load_workspace()
        .map(Some)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
}

/// Re-resolves one registered repository from any checkout that still exists.
///
/// A repository whose checkouts are all gone has nothing to reconcile, and one Git no longer
/// recognizes is left as it is rather than deleted: the directory may be coming back, and the row
/// that says so is the one that offers to look for it.
fn resolve_repo(
    database: &Database,
    repo_id: &str,
) -> Result<Option<crate::domain::workspace::Repo>, IpcError> {
    let Some(path) = database
        .existing_git_checkout(repo_id)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?
    else {
        return Ok(None);
    };
    let Some((resolved, _)) = git::resolve_repository(&path, &timestamp())? else {
        return Ok(None);
    };
    if resolved.id != repo_id {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "checkout no longer belongs to the registered Git repository",
        ));
    }
    Ok(Some(resolved))
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
        archive_checkout, close_checkout, close_missing_checkout, locate_missing_checkout,
        register_folder, restore, set_default_branch, sync_repo,
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
    fn restart_restores_checkout_order_and_focus_but_no_terminal_sessions() {
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
        // The session the previous process selected is gone with its PTY, so nothing points at it.
        assert_eq!(restored.active_session_id, None);
        assert!(repo.checkouts[1].sessions.is_empty());
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

    /// A worktree is created by more than one hand: an agent running `git worktree add` in a
    /// terminal, a script, this app's own dialog. Whichever one it was, the panel has to show it
    /// while the app is open rather than at the next launch, and the registration Marvis holds is
    /// the only place that reconciliation can happen.
    #[test]
    fn sync_repo_lists_a_worktree_created_outside_the_app_while_it_is_open() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("external worktree");
        init_repo(&primary);
        let db = database(temp.path());
        let opened = register_folder(&db, &primary).unwrap();
        let repo_id = opened.repos[0].id.clone();
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

        let synced = sync_repo(&db, &repo_id)
            .unwrap()
            .expect("a worktree joined");

        assert_eq!(synced.repos.len(), 1);
        assert_eq!(synced.repos[0].checkouts.len(), 2);
        assert_eq!(
            synced.repos[0].checkouts[1].branch.as_deref(),
            Some("external")
        );
        // The panel grows rather than the selection moving: an agent adding a worktree is not
        // something the user asked the app to open.
        assert_eq!(synced.active_checkout_id, opened.active_checkout_id);
    }

    #[test]
    fn sync_repo_does_not_reload_the_workspace_when_the_worktree_list_is_unchanged() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        init_repo(&primary);
        let db = database(temp.path());
        let opened = register_folder(&db, &primary).unwrap();

        let synced = sync_repo(&db, &opened.repos[0].id).unwrap();

        assert!(synced.is_none());
    }

    #[test]
    fn sync_repo_reports_a_failed_git_worktree_list() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        init_repo(&primary);
        let db = database(temp.path());
        let opened = register_folder(&db, &primary).unwrap();
        fs::rename(primary.join(".git"), primary.join(".git unavailable")).unwrap();

        let result = sync_repo(&db, &opened.repos[0].id);

        assert!(result.is_err());
    }

    /// A worktree that is gone from disk stops being one of the rows, the same way a restart
    /// leaves it: as a row that says its directory is missing and can be closed, rather than a row
    /// describing changes nobody can read any more.
    #[test]
    fn sync_repo_marks_a_worktree_removed_outside_the_app_as_missing() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("external worktree");
        init_repo(&primary);
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
        let db = database(temp.path());
        let opened = register_folder(&db, &primary).unwrap();
        let repo_id = opened.repos[0].id.clone();
        let worktree_id = opened.repos[0].checkouts[1].id.clone();
        git(&primary, &["worktree", "remove", linked.to_str().unwrap()]);

        let synced = sync_repo(&db, &repo_id).unwrap().expect("a worktree left");

        let checkout = synced.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.id == worktree_id)
            .expect("the row is still there to close");
        assert!(checkout.is_missing);
    }

    /// An archived worktree is registered and alive but off the panel, so reading the disk again
    /// has to leave it there. Otherwise the shelf emptied itself the moment an agent added a
    /// sibling, and the row the user had put away came back on its own.
    #[test]
    fn sync_repo_keeps_an_archived_worktree_archived() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let archived = temp.path().join("archived worktree");
        let added = temp.path().join("added worktree");
        init_repo(&primary);
        git(
            &primary,
            &[
                "worktree",
                "add",
                "-b",
                "archived",
                archived.to_str().unwrap(),
            ],
        );
        let db = database(temp.path());
        let opened = register_folder(&db, &archived).unwrap();
        let repo_id = opened.repos[0].id.clone();
        let worktree_id = opened.repos[0].checkouts[1].id.clone();
        archive_checkout(&db, &worktree_id).unwrap();
        git(
            &primary,
            &["worktree", "add", "-b", "added", added.to_str().unwrap()],
        );

        let synced = sync_repo(&db, &repo_id)
            .unwrap()
            .expect("a worktree joined");

        assert_eq!(synced.repos[0].checkouts.len(), 2);
        assert!(synced.repos[0]
            .checkouts
            .iter()
            .all(|checkout| checkout.branch.as_deref() != Some("archived")));
        assert!(synced.repos[0]
            .checkouts
            .iter()
            .any(|checkout| checkout.branch.as_deref() == Some("added")));
        assert_eq!(synced.archived_worktrees.len(), 1);
        assert_eq!(synced.archived_worktrees[0].id, worktree_id);
    }

    /// Launching the app re-reads Git's own worktree list and writes it back over what is
    /// registered, so an archived worktree is re-inserted on every start. It has to come
    /// back archived: otherwise archiving would last until the next launch, and the user
    /// would find their panel rearranged by a restart they did nothing to cause.
    #[test]
    fn a_restart_keeps_an_archived_worktree_archived() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("feature worktree");
        init_repo(&primary);
        git(
            &primary,
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        );
        let db = database(temp.path());
        let opened = register_folder(&db, &linked).unwrap();
        let worktree_id = opened
            .repos
            .iter()
            .flat_map(|repo| repo.checkouts.iter())
            .find(|checkout| checkout.branch.as_deref() == Some("feature"))
            .expect("the feature worktree")
            .id
            .clone();

        archive_checkout(&db, &worktree_id).unwrap();
        let restored = restore(&db).unwrap();

        assert_eq!(restored.repos[0].checkouts.len(), 1);
        assert!(restored.repos[0].checkouts[0].is_primary);
        assert_eq!(restored.archived_worktrees.len(), 1);
        assert_eq!(restored.archived_worktrees[0].id, worktree_id);
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
        // The restart that precedes the relocation already took the session and its layout with
        // it, so what still has to move is the history that outlives a terminal.
        assert!(checkout.sessions.is_empty());
        assert_eq!(located.active_checkout_id.as_deref(), Some(new_id.as_str()));
        assert_eq!(located.active_session_id, None);
        assert_eq!(database.load_terminal_layout(&new_id).unwrap(), None);
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
        assert!(repo.checkouts[0].sessions.is_empty());
        assert_eq!(database.viewed_files(&new_id).unwrap(), ["notes.md"]);
        assert_eq!(
            relocated.active_checkout_id.as_deref(),
            Some(new_id.as_str())
        );
        assert_eq!(relocated.active_session_id, None);
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
    fn closing_a_live_worktree_takes_the_entry_off_the_list_and_nothing_else() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("feature");
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
        fs::write(linked.join("work-in-progress.txt"), "uncommitted\n").unwrap();
        let database = database(temp.path());
        let registered = register_folder(&database, &primary).unwrap();
        let primary_id = registered.repos[0].checkouts[0].id.clone();
        let linked_id = register_folder(&database, &linked)
            .unwrap()
            .active_checkout_id
            .expect("opening the worktree selects it");
        let backend = crate::terminal::TerminalBackend::default();

        let closed = close_checkout(&database, &backend, &linked_id).unwrap();

        // The row is gone and everything it pointed at is still there, uncommitted work
        // included: opening the folder again is what brings the workdir back.
        assert_eq!(closed.repos[0].checkouts.len(), 1);
        assert_eq!(closed.repos[0].checkouts[0].id, primary_id);
        assert_eq!(
            closed.active_checkout_id.as_deref(),
            Some(closed.repos[0].checkouts[0].id.as_str())
        );
        assert!(linked.join("work-in-progress.txt").exists());
        // Git still knows the worktree, because Marvis only forgot it: `git worktree list` goes
        // on naming the directory, and so does the branch.
        let listed = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&primary)
            .output()
            .expect("Git is installed");
        assert!(String::from_utf8_lossy(&listed.stdout).contains(linked.to_str().unwrap()));
        git(
            &primary,
            &["show-ref", "--verify", "--quiet", "refs/heads/temporary"],
        );
    }

    #[test]
    fn closing_a_live_worktree_is_refused_while_its_session_is_active() {
        let temp = tempdir().unwrap();
        let primary = temp.path().join("repo");
        let linked = temp.path().join("feature");
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
        register_folder(&database, &primary).unwrap();
        let linked_id = register_folder(&database, &linked)
            .unwrap()
            .active_checkout_id
            .expect("opening the worktree selects it");
        database
            .add_active_session(&Session {
                id: "session:live".into(),
                session_type: SessionType::Shell,
                checkout_id: linked_id.clone(),
                name: "shell".into(),
                created_at: "now".into(),
                status: SessionStatus::Active,
            })
            .unwrap();

        let refused = close_checkout(
            &database,
            &crate::terminal::TerminalBackend::default(),
            &linked_id,
        )
        .unwrap_err();

        // A terminal still attached to the directory would outlive the row that names it.
        assert!(refused.message.contains("active terminal sessions"));
        assert_eq!(
            database.load_workspace().unwrap().repos[0].checkouts.len(),
            2
        );
    }

    #[test]
    fn closing_a_location_that_is_gone_still_prunes_the_worktree_pointing_at_it() {
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
        register_folder(&database, &primary).unwrap();
        let linked_id = register_folder(&database, &linked)
            .unwrap()
            .active_checkout_id
            .expect("opening the worktree selects it");
        fs::remove_dir_all(&linked).unwrap();

        // The same row closes both ways, so the one command cannot skip the prune a directory
        // that is gone would otherwise leave behind in the repository's worktree list.
        let closed = close_checkout(
            &database,
            &crate::terminal::TerminalBackend::default(),
            &linked_id,
        )
        .unwrap();

        assert_eq!(closed.repos[0].checkouts.len(), 1);
        let listed = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&primary)
            .output()
            .expect("Git is installed");
        let listed = String::from_utf8_lossy(&listed.stdout);
        assert!(!listed.contains(linked.to_str().unwrap()));
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
