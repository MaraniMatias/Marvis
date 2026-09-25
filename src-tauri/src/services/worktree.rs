use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, Output},
};

use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        workspace::{Checkout, Repo, Session, SessionStatus, SessionType, WorkspaceState},
    },
    persistence::Database,
    services::{checkout::resolve_checkout_path, git, workspace},
    terminal::TerminalBackend,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDefaults {
    pub location: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedWorktree {
    pub workspace: WorkspaceState,
    pub checkout_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveWorktreeSession {
    pub id: String,
    #[serde(rename = "type")]
    pub session_type: SessionType,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRemovalInfo {
    pub checkout_id: String,
    pub is_primary: bool,
    pub is_missing: bool,
    pub branch: Option<String>,
    pub dirty_files: Vec<String>,
    pub unmerged_commits: u32,
    pub active_sessions: Vec<ActiveWorktreeSession>,
    pub active_agent_sessions: Vec<ActiveWorktreeSession>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRemovalConfirmation {
    pub confirm_dirty: bool,
    pub confirmed_dirty_files: Vec<String>,
    pub confirmed_session_ids: Vec<String>,
    pub expected_branch: Option<String>,
    pub expected_unmerged_commits: u32,
    pub delete_branch: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedWorktree {
    pub workspace: WorkspaceState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Clone)]
struct Context {
    repo: Repo,
    checkout: Checkout,
    root: PathBuf,
}

pub fn defaults(database: &Database, checkout_id: &str) -> Result<WorktreeDefaults, IpcError> {
    let context = context(database, checkout_id)?;
    ensure_available_source(&context)?;
    require_main_branch(&context.root)?;
    Ok(WorktreeDefaults {
        location: Path::new(&context.repo.root)
            .join(".worktrees")
            .display()
            .to_string(),
        default_branch: "main".into(),
    })
}

pub fn create(
    database: &Database,
    checkout_id: &str,
    task_name: &str,
    branch: &str,
    location: &Path,
) -> Result<CreatedWorktree, IpcError> {
    let context = context(database, checkout_id)?;
    ensure_available_source(&context)?;
    let task_name = sanitize_component(task_name)?;
    let branch = branch.trim();
    if branch.is_empty() || !valid_branch(&context.root, branch) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "enter a valid Git branch name",
        ));
    }

    let default = require_main_branch(&context.root)?;

    let entries = worktrees(&context.root)?;
    if let Some(holder) = entries
        .iter()
        .find(|entry| entry.branch.as_deref() == Some(branch))
    {
        return Err(branch_in_use(branch, holder, &context.repo));
    }

    let destination_parent = prepare_location(location, Path::new(&context.repo.root))?;
    let repo_worktrees = Path::new(&context.repo.root).join(".worktrees");
    if location == repo_worktrees && destination_parent != repo_worktrees {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "the repository's .worktrees location cannot be a symbolic link",
        ));
    }
    if destination_parent != repo_worktrees {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "worktrees must be created in the repository's .worktrees directory",
        ));
    }
    ensure_worktrees_ignored(Path::new(&context.repo.root))?;
    fs::create_dir_all(&destination_parent).map_err(|error| {
        IpcError::new(
            IpcErrorCode::PermissionDenied,
            format!("could not create the worktree location: {error}"),
        )
    })?;
    let destination_parent = destination_parent.canonicalize().map_err(|error| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            format!("could not resolve the worktree location: {error}"),
        )
    })?;
    if destination_parent != repo_worktrees {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "worktrees must be created in the repository's .worktrees directory",
        ));
    }
    let destination = destination_parent.join(task_name);
    if destination.exists() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            format!(
                "worktree destination already exists: {}",
                destination.display()
            ),
        ));
    }

    let result = git_output(
        &context.root,
        vec![
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            destination.clone().into_os_string(),
            default.reference.clone().into(),
        ],
    )?;
    if !result.status.success() {
        if let Some(holder) = worktrees(&context.root)?
            .iter()
            .find(|entry| entry.branch.as_deref() == Some(branch))
        {
            return Err(branch_in_use(branch, holder, &context.repo));
        }
        return Err(git_error("could not create worktree", &result));
    }

    let registered = match workspace::register_folder(database, &destination) {
        Ok(workspace) => workspace,
        Err(error) => {
            let _ = git_output(
                &context.root,
                vec![
                    "worktree".into(),
                    "remove".into(),
                    "--force".into(),
                    destination.clone().into_os_string(),
                ],
            );
            let _ = git_output(
                &context.root,
                vec!["branch".into(), "-D".into(), "--".into(), branch.into()],
            );
            return Err(error);
        }
    };
    let created = registered
        .repos
        .iter()
        .flat_map(|repo| repo.checkouts.iter())
        .find(|checkout| checkout.canonical_path == destination.display().to_string())
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::OperationFailed,
                "Git created the worktree, but Marvis could not register it",
            )
        })?;
    Ok(CreatedWorktree {
        checkout_id: created.id.clone(),
        workspace: registered,
    })
}

pub fn removal_info(
    database: &Database,
    checkout_id: &str,
) -> Result<WorktreeRemovalInfo, IpcError> {
    let context = context(database, checkout_id)?;
    let (branch, is_missing) = if context.checkout.is_missing {
        (context.checkout.branch.clone(), true)
    } else {
        let entry = worktrees(&context.root)?
            .into_iter()
            .find(|entry| paths_equal(entry.path.as_deref(), &context.checkout.canonical_path))
            .ok_or_else(|| {
                IpcError::new(
                    IpcErrorCode::InvalidCheckout,
                    "Git no longer lists this checkout; refresh the workspace before removing it",
                )
            })?;
        (entry.branch.or(context.checkout.branch.clone()), false)
    };
    let branch = branch.filter(|branch| local_branch_exists(&context.root, branch));

    let dirty_files = if is_missing {
        Vec::new()
    } else {
        let status = checked_git(
            &context.root,
            ["status", "--porcelain=v2", "-z", "--untracked-files=all"],
            "could not inspect worktree changes",
        )?;
        git::parse_porcelain_v2(&status.stdout)?
            .into_values()
            .map(|file| file.path)
            .collect()
    };

    let unmerged_commits = if let Some(branch) = branch.as_deref() {
        let default = git::resolve_default_ref(
            &context.root,
            context.repo.default_branch.as_deref(),
            Some(Path::new(&context.repo.root)),
        )?;
        let ref_name = format!("refs/heads/{branch}");
        let output = checked_git(
            &context.root,
            [
                "rev-list",
                "--count",
                &format!("{}..{ref_name}", default.reference),
            ],
            "could not count commits not merged into the default branch",
        )?;
        output_text(&output).parse::<u32>().map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("Git returned an invalid unmerged commit count: {error}"),
            )
        })?
    } else {
        0
    };

    let mut active_sessions = Vec::new();
    let mut active_agent_sessions = Vec::new();
    for session in &context.checkout.sessions {
        if session.status != SessionStatus::Active {
            continue;
        }
        let active = ActiveWorktreeSession::from(session);
        if session.session_type == SessionType::Agent {
            active_agent_sessions.push(active);
        } else {
            active_sessions.push(active);
        }
    }
    Ok(WorktreeRemovalInfo {
        checkout_id: context.checkout.id,
        is_primary: context.checkout.is_primary,
        is_missing,
        branch,
        dirty_files,
        unmerged_commits,
        active_sessions,
        active_agent_sessions,
    })
}

pub fn remove(
    database: &Database,
    backend: &TerminalBackend,
    checkout_id: &str,
    confirmation: &WorktreeRemovalConfirmation,
) -> Result<RemovedWorktree, IpcError> {
    let info = removal_info(database, checkout_id)?;
    if info.is_primary {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "the primary checkout cannot be removed",
        ));
    }
    if !info.active_agent_sessions.is_empty() {
        let agents = info
            .active_agent_sessions
            .iter()
            .map(|session| session.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            format!("stop the active agent session(s) before removing this worktree: {agents}"),
        ));
    }
    if info.branch != confirmation.expected_branch
        || info.unmerged_commits != confirmation.expected_unmerged_commits
    {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "the worktree branch or unmerged commit count changed; review the removal safeguards and retry",
        ));
    }
    if info.dirty_files != confirmation.confirmed_dirty_files {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "the worktree changes changed; review the dirty files and confirm again",
        ));
    }
    if !info.dirty_files.is_empty() && !confirmation.confirm_dirty {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "worktree has uncommitted changes; commit or stash them in a shell, or explicitly confirm removal",
        ));
    }
    let mut actual_ids: Vec<_> = info
        .active_sessions
        .iter()
        .map(|session| session.id.clone())
        .collect();
    let mut confirmed_ids = confirmation.confirmed_session_ids.clone();
    actual_ids.sort();
    confirmed_ids.sort();
    if actual_ids != confirmed_ids {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "the active session list changed; review and confirm the current sessions before removing this worktree",
        ));
    }

    let context = context(database, checkout_id)?;
    let management_root = database
        .existing_git_checkout(&context.repo.id)
        .map_err(operation_error)?
        .and_then(|path| path.canonicalize().ok())
        .filter(|path| path != Path::new(&context.checkout.canonical_path))
        .unwrap_or_else(|| context.root.clone());
    for session_id in &actual_ids {
        if !backend.close(session_id).map_err(operation_error)? {
            return Err(IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "an active terminal session is no longer available; refresh and retry",
            ));
        }
        database
            .remove_terminal_session(session_id)
            .map_err(operation_error)?;
    }

    if info.is_missing {
        let pruned = git_output(
            &management_root,
            vec![
                "worktree".into(),
                "prune".into(),
                "--expire".into(),
                "now".into(),
            ],
        )?;
        if !pruned.status.success() {
            return Err(git_error("could not prune the missing worktree", &pruned));
        }
    } else {
        let mut args: Vec<OsString> = vec!["worktree".into(), "remove".into()];
        if !info.dirty_files.is_empty() {
            args.push("--force".into());
        }
        args.push(PathBuf::from(&context.checkout.canonical_path).into_os_string());
        let output = git_output(&management_root, args)?;
        if !output.status.success() {
            return Err(git_error("could not remove worktree", &output));
        }
    }

    let mut branch_error = None;
    if confirmation.delete_branch {
        if let Some(branch) = info.branch.as_deref() {
            let output = git_output(
                &management_root,
                vec!["branch".into(), "-D".into(), "--".into(), branch.into()],
            )?;
            if !output.status.success() {
                branch_error = Some(git_error(
                    "worktree was removed, but branch deletion failed",
                    &output,
                ));
            }
        }
    }
    let workspace = database
        .remove_checkout(&context.repo.id, checkout_id)
        .map_err(operation_error)?;
    Ok(RemovedWorktree {
        workspace,
        warning: branch_error.map(|error| error.message),
    })
}

impl From<&Session> for ActiveWorktreeSession {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            session_type: session.session_type.clone(),
            name: session.name.clone(),
        }
    }
}

fn context(database: &Database, checkout_id: &str) -> Result<Context, IpcError> {
    let workspace = database.load_workspace().map_err(operation_error)?;
    let (repo, checkout) = crate::services::checkout::registered_checkout(
        &workspace.repos,
        checkout_id,
        "checkout ID is not registered",
    )?;
    if repo.kind != crate::domain::workspace::RepoKind::Git {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "worktrees are only available for Git repositories",
        ));
    }
    let root = if checkout.is_missing {
        let path = database
            .existing_git_checkout(&repo.id)
            .map_err(operation_error)?
            .ok_or_else(|| {
                IpcError::new(
                    IpcErrorCode::FolderMissing,
                    "no available checkout exists to manage this repository's worktrees",
                )
            })?;
        path.canonicalize().map_err(|error| {
            IpcError::new(
                IpcErrorCode::FolderMissing,
                format!("repository checkout is unavailable: {error}"),
            )
        })?
    } else {
        resolve_checkout_path(repo, checkout_id, Path::new("."))?
    };
    Ok(Context {
        repo: repo.clone(),
        checkout: checkout.clone(),
        root,
    })
}

fn ensure_available_source(context: &Context) -> Result<(), IpcError> {
    if context.checkout.is_missing {
        return Err(IpcError::new(
            IpcErrorCode::FolderMissing,
            "cannot create a worktree from a missing checkout",
        ));
    }
    Ok(())
}

#[derive(Default)]
struct WorktreeEntry {
    path: Option<PathBuf>,
    branch: Option<String>,
}

fn worktrees(root: &Path) -> Result<Vec<WorktreeEntry>, IpcError> {
    let output = checked_git(
        root,
        ["worktree", "list", "--porcelain", "-z"],
        "could not list Git worktrees",
    )?;
    let mut result = Vec::new();
    let mut current = WorktreeEntry::default();
    for line in output.stdout.split(|byte| *byte == 0) {
        if line.is_empty() {
            if current.path.is_some() {
                result.push(current);
            }
            current = WorktreeEntry::default();
            continue;
        }
        let line = String::from_utf8_lossy(line);
        if let Some(value) = line.strip_prefix("worktree ") {
            current.path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
            current.branch = Some(value.to_owned());
        }
    }
    if current.path.is_some() {
        result.push(current);
    }
    Ok(result)
}

fn paths_equal(path: Option<&Path>, stored: &str) -> bool {
    path.is_some_and(|path| {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        path == Path::new(stored)
    })
}

fn branch_in_use(branch: &str, holder: &WorktreeEntry, repo: &Repo) -> IpcError {
    let path = holder
        .path
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "an unknown checkout".into());
    let label = repo
        .checkouts
        .iter()
        .find(|checkout| paths_equal(holder.path.as_deref(), &checkout.canonical_path))
        .map(|checkout| {
            format!(
                "{} checkout ({})",
                if checkout.is_primary {
                    "primary"
                } else {
                    "worktree"
                },
                checkout.canonical_path
            )
        })
        .unwrap_or_else(|| format!("checkout at {path}"));
    IpcError::new(
        IpcErrorCode::GitFailed,
        format!("branch '{branch}' is already checked out in {label}"),
    )
}

fn valid_branch(root: &Path, branch: &str) -> bool {
    let reference = format!("refs/heads/{branch}");
    git_output(root, vec!["check-ref-format".into(), reference.into()])
        .is_ok_and(|output| output.status.success())
}

fn require_main_branch(root: &Path) -> Result<git::DefaultRef, IpcError> {
    if local_branch_exists(root, "main") {
        return Ok(git::DefaultRef {
            reference: "refs/heads/main".into(),
            branch: "main".into(),
        });
    }
    git::resolve_branch_ref(root, "main").ok_or_else(|| {
        IpcError::new(
            IpcErrorCode::DefaultBranchUnknown,
            "Git branch 'main' does not exist; create or fetch it before creating a worktree",
        )
    })
}

fn local_branch_exists(root: &Path, branch: &str) -> bool {
    if !valid_branch(root, branch) {
        return false;
    }
    git_output(
        root,
        vec![
            "rev-parse".into(),
            "--verify".into(),
            "--quiet".into(),
            format!("refs/heads/{branch}^{{commit}}").into(),
        ],
    )
    .is_ok_and(|output| output.status.success())
}

fn sanitize_component(value: &str) -> Result<String, IpcError> {
    let mut sanitized = String::new();
    for character in value.trim().chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | ' ') {
            sanitized.push(character);
        } else if !sanitized.ends_with('-') {
            sanitized.push('-');
        }
    }
    let sanitized = sanitized.trim_matches([' ', '.']).trim_start_matches('.');
    let mut sanitized = sanitized.chars().take(80).collect::<String>();
    while sanitized.len() > 180 {
        sanitized.pop();
    }
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "worktree name must contain a letter or number",
        ));
    }
    Ok(sanitized)
}

fn prepare_location(location: &Path, repo_root: &Path) -> Result<PathBuf, IpcError> {
    if !location.is_absolute()
        || location
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "worktree location must be an absolute path without '..'",
        ));
    }
    let mut existing = location;
    let mut missing = Vec::new();
    while !existing.exists() {
        if fs::symlink_metadata(existing).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(IpcError::new(
                IpcErrorCode::InvalidPath,
                "worktree location contains a broken symbolic link",
            ));
        }
        let name = existing.file_name().ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidPath,
                "worktree location is unavailable",
            )
        })?;
        missing.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidPath,
                "worktree location is unavailable",
            )
        })?;
    }
    let mut resolved = existing.canonicalize().map_err(|error| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            format!("could not resolve worktree location: {error}"),
        )
    })?;
    if !resolved.is_dir() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "worktree location must be a directory",
        ));
    }
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    if resolved.starts_with(repo_root) && resolved != repo_root.join(".worktrees") {
        return Err(IpcError::new(
            IpcErrorCode::PathOutsideCheckout,
            "worktrees inside the repository must use its .worktrees directory",
        ));
    }
    Ok(resolved)
}

fn ensure_worktrees_ignored(root: &Path) -> Result<(), IpcError> {
    let output = checked_git(
        root,
        ["rev-parse", "--git-path", "info/exclude"],
        "could not locate Git exclude file",
    )?;
    let raw_path = PathBuf::from(output_text(&output));
    let path = if raw_path.is_absolute() {
        raw_path
    } else {
        root.join(raw_path)
    };
    let git_dir = checked_git(
        root,
        ["rev-parse", "--absolute-git-dir"],
        "could not locate Git metadata",
    )?;
    let git_dir = PathBuf::from(output_text(&git_dir))
        .canonicalize()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::InvalidPath,
                format!("could not resolve Git metadata directory: {error}"),
            )
        })?;
    let git_common_dir = checked_git(
        root,
        ["rev-parse", "--git-common-dir"],
        "could not locate shared Git metadata",
    )?;
    let git_common_dir = PathBuf::from(output_text(&git_common_dir));
    let git_common_dir = if git_common_dir.is_absolute() {
        git_common_dir
    } else {
        root.join(git_common_dir)
    }
    .canonicalize()
    .map_err(|error| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            format!("could not resolve shared Git metadata directory: {error}"),
        )
    })?;
    let exclude_parent = path.parent().ok_or_else(|| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            "Git exclude file has no parent directory",
        )
    })?;
    let exclude_parent = exclude_parent.canonicalize().map_err(|error| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            format!("could not resolve Git info directory: {error}"),
        )
    })?;
    if !exclude_parent.starts_with(&git_dir) && !exclude_parent.starts_with(&git_common_dir) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "Git's local exclude file is outside the repository metadata directory",
        ));
    }
    let contents = match fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(IpcError::new(
                IpcErrorCode::PermissionDenied,
                format!("could not read Git's local exclude file: {error}"),
            ));
        }
    };
    let already_ignored = contents
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .any(|line| {
            matches!(
                line,
                b"/.worktrees/" | b"/.worktrees" | b".worktrees/" | b".worktrees"
            )
        });
    if already_ignored {
        return Ok(());
    }
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "Git's local exclude file is a symbolic link; refusing to modify a file outside the repository metadata",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            IpcError::new(
                IpcErrorCode::PermissionDenied,
                format!("could not create Git's local info directory: {error}"),
            )
        })?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::PermissionDenied,
                format!("could not update Git's local exclude file: {error}"),
            )
        })?;
    if !contents.is_empty() && !contents.ends_with(b"\n") {
        file.write_all(b"\n").map_err(|error| {
            IpcError::new(
                IpcErrorCode::PermissionDenied,
                format!("could not update Git's local exclude file: {error}"),
            )
        })?;
    }
    file.write_all(b"/.worktrees/\n").map_err(|error| {
        IpcError::new(
            IpcErrorCode::PermissionDenied,
            format!("could not update Git's local exclude file: {error}"),
        )
    })
}

fn git_output(root: &Path, args: Vec<OsString>) -> Result<Output, IpcError> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not start Git: {error}"),
            )
        })
}

fn checked_git<const N: usize>(
    root: &Path,
    args: [&str; N],
    action: &str,
) -> Result<Output, IpcError> {
    let output = git_output(root, args.into_iter().map(OsString::from).collect())?;
    if !output.status.success() {
        return Err(git_error(action, &output));
    }
    Ok(output)
}

fn git_error(action: &str, output: &Output) -> IpcError {
    IpcError::new(
        IpcErrorCode::GitFailed,
        format!("{action}: {}", output_text(output)),
    )
}

fn output_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

fn operation_error(error: String) -> IpcError {
    IpcError::new(IpcErrorCode::OperationFailed, error)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use tempfile::tempdir;

    use crate::{
        domain::workspace::{Session, SessionStatus, SessionType},
        persistence::Database,
        services::workspace,
        terminal::{SpawnOptions, TerminalBackend},
    };

    use super::{
        create, defaults, removal_info, remove, WorktreeRemovalConfirmation, WorktreeRemovalInfo,
    };

    fn git(cwd: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn fixture(base: &Path) -> (Database, PathBuf, String) {
        let root = base.join("repo with spaces");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Marvis test"]);
        git(&root, &["config", "user.email", "marvis@example.invalid"]);
        fs::write(root.join("base.txt"), "base\n").unwrap();
        git(&root, &["add", "base.txt"]);
        git(&root, &["commit", "-m", "base"]);
        let database = Database::open(base.join("workspace.sqlite3")).unwrap();
        let state = workspace::register_folder(&database, &root).unwrap();
        let checkout_id = state.repos[0].checkouts[0].id.clone();
        (database, root, checkout_id)
    }

    fn add_worktree(database: &Database, root: &Path, base: &Path, branch: &str) -> String {
        let path = base.join(format!("checkout-{branch}"));
        git(
            root,
            &["worktree", "add", "-b", branch, path.to_str().unwrap()],
        );
        let state = workspace::register_folder(database, &path).unwrap();
        state.repos[0]
            .checkouts
            .iter()
            .find(|checkout| {
                checkout.canonical_path == path.canonicalize().unwrap().display().to_string()
            })
            .unwrap()
            .id
            .clone()
    }

    fn branch_exists(root: &Path, branch: &str) -> bool {
        Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    }

    fn confirmation(
        info: &WorktreeRemovalInfo,
        confirm_dirty: bool,
        confirmed_dirty_files: Vec<String>,
        confirmed_session_ids: Vec<String>,
        delete_branch: bool,
    ) -> WorktreeRemovalConfirmation {
        WorktreeRemovalConfirmation {
            confirm_dirty,
            confirmed_dirty_files,
            confirmed_session_ids,
            expected_branch: info.branch.clone(),
            expected_unmerged_commits: info.unmerged_commits,
            delete_branch,
        }
    }

    #[test]
    fn sec_07_creates_from_any_checkout_and_sanitizes_worktree_task_names() {
        let temp = tempdir().unwrap();
        let (database, root, primary_id) = fixture(temp.path());
        let source_id = add_worktree(&database, &root, temp.path(), "current-feature");
        assert_ne!(source_id, primary_id);
        fs::write(root.join("main-only.txt"), "main head\n").unwrap();
        git(&root, &["add", "main-only.txt"]);
        git(&root, &["commit", "-m", "main-only"]);
        let location = root.canonicalize().unwrap().join(".worktrees");
        let excludes_path = root.join(".git/info/exclude");
        fs::write(&excludes_path, b"# existing user rule\n*.local\n").unwrap();
        let excludes_before = fs::read(&excludes_path).unwrap();

        let settings = defaults(&database, &source_id).unwrap();
        assert_eq!(settings.default_branch, "main");
        assert_eq!(settings.location, location.display().to_string());
        let created = create(
            &database,
            &source_id,
            "../../$(touch pwned); *",
            "feature/safe-name",
            &location,
        )
        .unwrap();

        let checkout = created.workspace.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.id == created.checkout_id)
            .unwrap();
        assert_eq!(checkout.branch.as_deref(), Some("feature/safe-name"));
        assert_eq!(
            Path::new(&checkout.canonical_path).parent(),
            Some(location.canonicalize().unwrap().as_path())
        );
        assert_eq!(
            git(
                Path::new(&checkout.canonical_path),
                &["log", "-1", "--format=%s"]
            ),
            "main-only"
        );
        assert_eq!(
            created.workspace.active_checkout_id.as_deref(),
            Some(created.checkout_id.as_str())
        );
        assert!(!temp.path().join("pwned").exists());
        let excludes_after = fs::read(excludes_path).unwrap();
        assert!(excludes_after.starts_with(&excludes_before));
        assert!(String::from_utf8_lossy(&excludes_after).contains("/.worktrees/"));
        assert_eq!(
            git(&root, &["status", "--porcelain", "--untracked-files=all"]),
            ""
        );
        assert_eq!(
            defaults(&database, &primary_id).unwrap().location,
            location.display().to_string()
        );
    }

    #[test]
    fn worktree_creation_requires_main_without_falling_back_to_the_current_branch() {
        let temp = tempdir().unwrap();
        let (database, root, checkout_id) = fixture(temp.path());
        git(&root, &["branch", "-m", "trunk"]);

        let error = defaults(&database, &checkout_id).unwrap_err();

        assert!(error.message.contains("Git branch 'main' does not exist"));
        assert!(create(
            &database,
            &checkout_id,
            "work",
            "feature/work",
            &root.join(".worktrees"),
        )
        .unwrap_err()
        .message
        .contains("Git branch 'main' does not exist"));
    }

    #[test]
    fn worktree_creation_is_fixed_to_the_repository_worktrees_directory() {
        let temp = tempdir().unwrap();
        let (database, root, checkout_id) = fixture(temp.path());
        let external = temp.path().join("external-worktrees");

        let error = create(&database, &checkout_id, "work", "feature/work", &external).unwrap_err();

        assert!(error.message.contains("repository's .worktrees directory"));
        assert!(!external.exists());
        assert!(!root.join(".worktrees").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_append_to_an_exclude_symlink_outside_git_metadata() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let (database, root, checkout_id) = fixture(temp.path());
        let exclude = root.join(".git/info/exclude");
        let outside = temp.path().join("user-exclude");
        fs::write(&outside, b"user data\n").unwrap();
        fs::remove_file(&exclude).unwrap();
        symlink(&outside, &exclude).unwrap();

        let error = create(
            &database,
            &checkout_id,
            "work",
            "feature/work",
            &root.join(".worktrees"),
        )
        .unwrap_err();

        assert!(error.message.contains("symbolic link"));
        assert_eq!(fs::read(outside).unwrap(), b"user data\n");
    }

    #[test]
    fn sec_07_rejects_branch_metacharacters_and_names_the_checkout_holding_an_existing_branch() {
        let temp = tempdir().unwrap();
        let (database, root, primary_id) = fixture(temp.path());
        let holder_id = add_worktree(&database, &root, temp.path(), "occupied");
        let location = temp.path().join("outside");

        let in_use = create(&database, &primary_id, "other", "occupied", &location).unwrap_err();
        assert!(in_use.message.contains("already checked out"));
        assert!(in_use.message.contains(&holder_id.replace("checkout:", "")));

        let invalid = create(
            &database,
            &primary_id,
            "other",
            "$(touch pwned);bad",
            &location,
        )
        .unwrap_err();
        assert!(invalid.message.contains("valid Git branch"));
        assert!(!temp.path().join("pwned").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_configured_location_symlink_that_traverses_into_the_repository() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let (database, root, checkout_id) = fixture(temp.path());
        let alias = temp.path().join("external-alias");
        symlink(&root, &alias).unwrap();

        let error = create(&database, &checkout_id, "work", "feature/symlink", &alias).unwrap_err();

        assert!(error.message.contains("inside the repository"));
        assert!(!root.join("repo with spaces/work").exists());
    }

    #[test]
    fn dirty_removal_requires_confirmation_and_keeps_the_branch_by_default() {
        let temp = tempdir().unwrap();
        let (database, root, _) = fixture(temp.path());
        let checkout_id = add_worktree(&database, &root, temp.path(), "dirty-feature");
        let state = database.load_workspace().unwrap();
        let checkout = state.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)
            .unwrap();
        fs::write(
            Path::new(&checkout.canonical_path).join("untracked.txt"),
            "dirty",
        )
        .unwrap();
        let info = removal_info(&database, &checkout_id).unwrap();
        assert_eq!(info.dirty_files, ["untracked.txt"]);

        let backend = TerminalBackend::default();
        let unconfirmed = confirmation(&info, false, info.dirty_files.clone(), vec![], false);
        assert!(remove(&database, &backend, &checkout_id, &unconfirmed).is_err());
        fs::write(
            Path::new(&checkout.canonical_path).join("another-change.txt"),
            "changed after confirmation",
        )
        .unwrap();
        let stale_confirmation = confirmation(&info, true, info.dirty_files.clone(), vec![], false);
        let stale = remove(&database, &backend, &checkout_id, &stale_confirmation).unwrap_err();
        assert!(stale.message.contains("changes changed"));
        let info = removal_info(&database, &checkout_id).unwrap();
        let confirmed = confirmation(&info, true, info.dirty_files.clone(), vec![], false);
        let result = remove(&database, &backend, &checkout_id, &confirmed).unwrap();

        assert!(branch_exists(&root, "dirty-feature"));
        assert!(!result.workspace.repos[0]
            .checkouts
            .iter()
            .any(|item| item.id == checkout_id));
        assert!(!Path::new(&checkout.canonical_path).exists());
    }

    #[test]
    fn reports_unmerged_commit_count_and_only_deletes_branch_when_chosen() {
        let temp = tempdir().unwrap();
        let (database, root, _) = fixture(temp.path());
        let checkout_id = add_worktree(&database, &root, temp.path(), "ahead-feature");
        let state = database.load_workspace().unwrap();
        let checkout = state.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)
            .unwrap();
        let path = Path::new(&checkout.canonical_path);
        fs::write(path.join("ahead.txt"), "ahead\n").unwrap();
        git(path, &["add", "ahead.txt"]);
        git(path, &["commit", "-m", "ahead"]);
        let info = removal_info(&database, &checkout_id).unwrap();
        assert_eq!(info.unmerged_commits, 1);

        let confirmed = confirmation(&info, false, vec![], vec![], true);
        remove(
            &database,
            &TerminalBackend::default(),
            &checkout_id,
            &confirmed,
        )
        .unwrap();

        assert!(!branch_exists(&root, "ahead-feature"));
    }

    #[test]
    fn primary_is_denied_and_active_agent_is_a_hard_block() {
        let temp = tempdir().unwrap();
        let (database, root, primary_id) = fixture(temp.path());
        let backend = TerminalBackend::default();
        let primary_info = removal_info(&database, &primary_id).unwrap();
        let primary_confirmation = confirmation(&primary_info, true, vec![], vec![], false);
        assert!(
            remove(&database, &backend, &primary_id, &primary_confirmation)
                .unwrap_err()
                .message
                .contains("primary")
        );

        let checkout_id = add_worktree(&database, &root, temp.path(), "agent-feature");
        database
            .add_active_session(&Session {
                id: "session:agent-active".into(),
                session_type: SessionType::Agent,
                checkout_id: checkout_id.clone(),
                name: "coding agent".into(),
                created_at: "now".into(),
                status: SessionStatus::Active,
            })
            .unwrap();
        let info = removal_info(&database, &checkout_id).unwrap();
        assert_eq!(info.active_agent_sessions[0].name, "coding agent");
        let agent_confirmation = confirmation(&info, false, vec![], vec![], false);
        assert!(
            remove(&database, &backend, &checkout_id, &agent_confirmation)
                .unwrap_err()
                .message
                .contains("active agent")
        );
    }

    #[test]
    fn active_terminal_sessions_are_listed_require_exact_confirmation_and_are_stopped() {
        let temp = tempdir().unwrap();
        let (database, root, _) = fixture(temp.path());
        let checkout_id = add_worktree(&database, &root, temp.path(), "session-feature");
        let state = database.load_workspace().unwrap();
        let checkout = state.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)
            .unwrap();
        let session_id = "session:active-worktree";
        let backend = TerminalBackend::default();
        backend
            .spawn(
                session_id.into(),
                SpawnOptions {
                    program: PathBuf::from("/bin/sh"),
                    args: vec!["-c".into(), "exec sleep 30".into()],
                    cwd: PathBuf::from(&checkout.canonical_path),
                    cols: 80,
                    rows: 24,
                },
                Box::new(|_| Ok(())),
            )
            .unwrap();
        database
            .add_active_session(&Session {
                id: session_id.into(),
                session_type: SessionType::Shell,
                checkout_id: checkout_id.clone(),
                name: "shell · session-feature".into(),
                created_at: "now".into(),
                status: SessionStatus::Active,
            })
            .unwrap();

        let info = removal_info(&database, &checkout_id).unwrap();
        assert_eq!(info.active_sessions[0].id, session_id);
        let no_session_confirmation = confirmation(&info, false, vec![], vec![], false);
        assert!(remove(&database, &backend, &checkout_id, &no_session_confirmation).is_err());
        let confirmed = confirmation(&info, false, vec![], vec![session_id.into()], false);
        let result = remove(&database, &backend, &checkout_id, &confirmed).unwrap();

        assert!(!result.workspace.repos[0]
            .checkouts
            .iter()
            .any(|checkout| checkout.id == checkout_id));
        assert!(backend.status(session_id).is_err());
    }

    #[test]
    fn missing_worktree_removal_prunes_git_metadata_and_clears_registration() {
        let temp = tempdir().unwrap();
        let (database, root, _) = fixture(temp.path());
        let checkout_id = add_worktree(&database, &root, temp.path(), "missing-feature");
        let state = database.load_workspace().unwrap();
        let path = state.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)
            .unwrap()
            .canonical_path
            .clone();
        fs::remove_dir_all(&path).unwrap();
        git(&root, &["worktree", "prune", "--expire", "now"]);
        git(&root, &["branch", "-D", "--", "missing-feature"]);

        let info = removal_info(&database, &checkout_id).unwrap();
        assert!(info.is_missing);
        assert_eq!(info.branch, None);
        let confirmed = confirmation(&info, false, vec![], vec![], false);
        let result = remove(
            &database,
            &TerminalBackend::default(),
            &checkout_id,
            &confirmed,
        )
        .unwrap();

        assert!(!result.workspace.repos[0]
            .checkouts
            .iter()
            .any(|checkout| checkout.id == checkout_id));
        let listed = git(&root, &["worktree", "list", "--porcelain"]);
        assert!(!listed.contains("missing-feature"));
    }
}
