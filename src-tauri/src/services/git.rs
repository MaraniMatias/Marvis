use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Output},
    sync::{mpsc, Mutex},
    thread,
    time::Duration,
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        workspace::{Checkout, Repo, RepoKind},
    },
    persistence::Database,
    services::checkout::resolve_checkout_path,
};

const WATCH_DEBOUNCE: Duration = Duration::from_millis(220);
const STATUS_CHANGED_EVENT: &str = "git-status-changed";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitChangedFile {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub default_branch: String,
    pub ahead_count: u32,
    pub files: Vec<GitChangedFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileDiff {
    pub path: String,
    pub patch: String,
    pub is_binary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
}

#[derive(Default)]
pub struct GitWatcherManager {
    watchers: Mutex<BTreeMap<String, (RecommendedWatcher, mpsc::Sender<WatchMessage>)>>,
}

#[derive(Debug, PartialEq, Eq)]
enum WatchMessage {
    Changed,
    Stop,
}

#[derive(Debug, PartialEq, Eq)]
enum WatchWakeup {
    Changed,
    Stop,
}

#[derive(Clone)]
struct GitContext {
    repo: Repo,
    checkout: Checkout,
    root: PathBuf,
}

struct Snapshot {
    status: GitStatus,
    merge_base: String,
}

impl GitWatcherManager {
    pub fn watch(
        &self,
        app: AppHandle,
        checkout_id: String,
        paths: Vec<PathBuf>,
    ) -> Result<(), IpcError> {
        let mut watchers = self
            .watchers
            .lock()
            .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error.to_string()))?;
        if watchers.contains_key(&checkout_id) {
            return Ok(());
        }

        let (sender, receiver) = mpsc::channel();
        let event_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if result.is_ok() {
                    let _ = event_sender.send(WatchMessage::Changed);
                }
            })
            .map_err(|error| {
                IpcError::new(
                    IpcErrorCode::OperationFailed,
                    format!("could not watch checkout: {error}"),
                )
            })?;
        for path in paths {
            watcher
                .watch(&path, RecursiveMode::Recursive)
                .map_err(|error| {
                    IpcError::new(
                        IpcErrorCode::OperationFailed,
                        format!("could not watch checkout: {error}"),
                    )
                })?;
        }

        let event_checkout_id = checkout_id.clone();
        thread::Builder::new()
            .name("marvis-git-watch".into())
            .spawn(move || {
                while let WatchWakeup::Changed = receive_debounced_change(&receiver, WATCH_DEBOUNCE)
                {
                    let _ = app.emit(STATUS_CHANGED_EVENT, &event_checkout_id);
                }
            })
            .map_err(|error| {
                IpcError::new(
                    IpcErrorCode::OperationFailed,
                    format!("could not start checkout watcher: {error}"),
                )
            })?;

        watchers.insert(checkout_id, (watcher, sender));
        Ok(())
    }

    pub fn unwatch(&self, checkout_id: &str) {
        if let Ok(mut watchers) = self.watchers.lock() {
            if let Some((watcher, sender)) = watchers.remove(checkout_id) {
                let _ = sender.send(WatchMessage::Stop);
                drop(watcher);
            }
        }
    }
}

fn receive_debounced_change(
    receiver: &mpsc::Receiver<WatchMessage>,
    debounce: Duration,
) -> WatchWakeup {
    match receiver.recv() {
        Ok(WatchMessage::Stop) | Err(_) => return WatchWakeup::Stop,
        Ok(WatchMessage::Changed) => {}
    }
    loop {
        match receiver.recv_timeout(debounce) {
            Ok(WatchMessage::Changed) => {}
            Ok(WatchMessage::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return WatchWakeup::Stop;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => return WatchWakeup::Changed,
        }
    }
}

pub fn status(database: &Database, checkout_id: &str) -> Result<GitStatus, IpcError> {
    Ok(snapshot(&registered_git_context(database, checkout_id)?)?.status)
}

pub fn diff(database: &Database, checkout_id: &str, path: &str) -> Result<GitFileDiff, IpcError> {
    let context = registered_git_context(database, checkout_id)?;
    let snapshot = snapshot(&context)?;
    let changed_file = snapshot
        .status
        .files
        .iter()
        .find(|file| file.path == path)
        .cloned()
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidPath,
                "requested path is not a changed file in this checkout",
            )
        })?;
    let relative_path = validate_relative_path(path)?;
    validate_diff_target(&context, &relative_path)?;

    let working_path = context.root.join(&relative_path);
    if fs::symlink_metadata(&working_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        let target = fs::read_link(&working_path).map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not read symlink target: {error}"),
            )
        })?;
        return Ok(GitFileDiff {
            path: path.to_owned(),
            patch: String::new(),
            is_binary: false,
            symlink_target: Some(target.to_string_lossy().into_owned()),
        });
    }

    let output = if changed_file.status == "??" {
        let path = context.root.join(&relative_path);
        let output = run_git(
            &context.root,
            vec![
                "diff".into(),
                "--no-index".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                "--no-color".into(),
                "--unified=3".into(),
                "--".into(),
                "/dev/null".into(),
                path.into_os_string(),
            ],
        )?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(git_error("could not diff untracked file", &output));
        }
        output
    } else {
        let mut args = vec![
            "diff".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--no-color".into(),
            "--find-renames".into(),
            "--unified=3".into(),
            snapshot.merge_base.into(),
            "--".into(),
        ];
        if let Some(old_path) = &changed_file.old_path {
            args.push(literal_pathspec(old_path));
        }
        args.push(literal_pathspec(path));
        let output = run_git(&context.root, args)?;
        if !output.status.success() {
            return Err(git_error("could not diff changed file", &output));
        }
        output
    };

    let raw_patch = String::from_utf8_lossy(&output.stdout).into_owned();
    let is_binary = raw_patch.contains("Binary files ") || raw_patch.contains("GIT binary patch");
    Ok(GitFileDiff {
        path: path.to_owned(),
        patch: if is_binary { String::new() } else { raw_patch },
        is_binary,
        symlink_target: None,
    })
}

pub fn watch_paths(database: &Database, checkout_id: &str) -> Result<Vec<PathBuf>, IpcError> {
    let context = registered_git_context(database, checkout_id)?;
    let mut paths = vec![context.root.clone()];
    let common_dir = output_text(&checked_git(
        &context.root,
        ["rev-parse", "--git-common-dir"],
        "could not locate Git metadata for watcher",
    )?);
    let common_dir = Path::new(&common_dir);
    let common_dir = if common_dir.is_absolute() {
        common_dir.to_path_buf()
    } else {
        context.root.join(common_dir)
    }
    .canonicalize()
    .map_err(|error| {
        IpcError::new(
            IpcErrorCode::GitFailed,
            format!("could not resolve Git metadata for watcher: {error}"),
        )
    })?;
    if !common_dir.starts_with(&context.root) {
        paths.push(common_dir);
    }
    Ok(paths)
}

fn registered_git_context(database: &Database, checkout_id: &str) -> Result<GitContext, IpcError> {
    let workspace = database
        .load_workspace()
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?;
    let repo = workspace
        .repos
        .into_iter()
        .find(|repo| {
            repo.checkouts
                .iter()
                .any(|checkout| checkout.id == checkout_id)
        })
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "checkout ID is not registered",
            )
        })?;
    if repo.kind != RepoKind::Git {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "Git changes are only available for Git checkouts",
        ));
    }
    let checkout = repo
        .checkouts
        .iter()
        .find(|checkout| checkout.id == checkout_id)
        .cloned()
        .ok_or_else(|| {
            IpcError::new(IpcErrorCode::InvalidCheckout, "checkout is not registered")
        })?;
    if checkout.is_missing {
        return Err(IpcError::new(
            IpcErrorCode::FolderMissing,
            "checkout is no longer available",
        ));
    }
    let root = resolve_checkout_path(&repo, checkout_id, Path::new("."))?;
    Ok(GitContext {
        repo,
        checkout,
        root,
    })
}

fn snapshot(context: &GitContext) -> Result<Snapshot, IpcError> {
    let default = resolve_default_ref(&context.root, context.repo.default_branch.as_deref())?;
    let merge_base_output = run_git(
        &context.root,
        vec![
            "merge-base".into(),
            default.reference.clone().into(),
            "HEAD".into(),
        ],
    )?;
    if !merge_base_output.status.success() {
        return Err(IpcError::new(
            IpcErrorCode::GitFailed,
            format!(
                "Git could not find a merge base between HEAD and {}",
                default.branch
            ),
        ));
    }
    let merge_base = output_text(&merge_base_output);

    let porcelain = checked_git(
        &context.root,
        ["status", "--porcelain=v2", "-z", "--untracked-files=all"],
        "could not read Git status",
    )?;
    let mut files = parse_porcelain_v2(&porcelain.stdout)?;

    let committed = checked_git(
        &context.root,
        [
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            &merge_base,
            "HEAD",
        ],
        "could not list committed changes",
    )?;
    for file in parse_name_status(&committed.stdout)? {
        files
            .entry(file.path.clone())
            .and_modify(|current| {
                if current.old_path.is_none() {
                    current.old_path = file.old_path.clone();
                }
            })
            .or_insert(file);
    }

    let files: Vec<_> = files.into_values().collect();
    let branch = optional_git_text(
        &context.root,
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
    );
    let head = optional_git_text(&context.root, ["rev-parse", "--short", "HEAD"]);
    let ahead_count = checked_git(
        &context.root,
        ["rev-list", "--count", &format!("{merge_base}..HEAD")],
        "could not count commits ahead of the default branch",
    )?
    .stdout;
    let ahead_count = String::from_utf8_lossy(&ahead_count)
        .trim()
        .parse()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("Git returned an invalid ahead count: {error}"),
            )
        })?;

    Ok(Snapshot {
        status: GitStatus {
            branch,
            head,
            default_branch: default.branch,
            ahead_count,
            files,
        },
        merge_base,
    })
}

#[derive(Debug)]
struct DefaultRef {
    reference: String,
    branch: String,
}

fn resolve_default_ref(root: &Path, persisted: Option<&str>) -> Result<DefaultRef, IpcError> {
    if let Some(branch) = persisted.map(|branch| branch.strip_prefix("origin/").unwrap_or(branch)) {
        if let Some(default) = branch_ref(root, branch) {
            return Ok(default);
        }
    }

    if let Ok(output) = run_git(
        root,
        vec![
            "symbolic-ref".into(),
            "--quiet".into(),
            "refs/remotes/origin/HEAD".into(),
        ],
    ) {
        if output.status.success() {
            let reference = output_text(&output);
            if let Some(branch) = reference.strip_prefix("refs/remotes/origin/") {
                if let Some(default) = branch_ref(root, branch) {
                    return Ok(default);
                }
            }
        }
    }

    Err(IpcError::new(
        IpcErrorCode::DefaultBranchUnknown,
        "Git could not determine a valid default branch; choose one before viewing changes",
    ))
}

fn branch_ref(root: &Path, branch: &str) -> Option<DefaultRef> {
    if branch.is_empty()
        || !run_git(
            root,
            vec![
                "check-ref-format".into(),
                format!("refs/heads/{branch}").into(),
            ],
        )
        .is_ok_and(|output| output.status.success())
    {
        return None;
    }
    for prefix in ["refs/remotes/origin/", "refs/heads/"] {
        let reference = format!("{prefix}{branch}");
        if run_git(
            root,
            vec![
                "rev-parse".into(),
                "--verify".into(),
                "--quiet".into(),
                format!("{reference}^{{commit}}").into(),
            ],
        )
        .is_ok_and(|output| output.status.success())
        {
            return Some(DefaultRef {
                reference,
                branch: branch.to_owned(),
            });
        }
    }
    None
}

fn parse_porcelain_v2(output: &[u8]) -> Result<BTreeMap<String, GitChangedFile>, IpcError> {
    let mut records = output.split(|byte| *byte == 0).peekable();
    let mut files = BTreeMap::new();
    while let Some(record) = records.next() {
        if record.is_empty() || record.first() == Some(&b'#') {
            continue;
        }
        let (path, old_path, status) = match record.first() {
            Some(b'?') if record.starts_with(b"? ") => {
                (record.get(2..).unwrap_or_default(), None, "??".to_owned())
            }
            Some(b'1') => (
                path_after_fields(record, 8).ok_or_else(malformed_status)?,
                None,
                status_from_xy(record.get(2..4).unwrap_or_default()),
            ),
            Some(b'2') => {
                let path = path_after_fields(record, 9).ok_or_else(malformed_status)?;
                let old_path = records.next().ok_or_else(malformed_status)?;
                (
                    path,
                    Some(path_text(old_path)?),
                    status_from_xy(record.get(2..4).unwrap_or_default()),
                )
            }
            Some(b'u') => (
                path_after_fields(record, 10).ok_or_else(malformed_status)?,
                None,
                status_from_xy(record.get(2..4).unwrap_or_default()),
            ),
            _ => return Err(malformed_status()),
        };
        let path = path_text(path)?;
        files.insert(
            path.clone(),
            GitChangedFile {
                path,
                old_path,
                status,
            },
        );
    }
    Ok(files)
}

fn parse_name_status(output: &[u8]) -> Result<Vec<GitChangedFile>, IpcError> {
    let records: Vec<_> = output.split(|byte| *byte == 0).collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let status_record = records[index];
        index += 1;
        if status_record.is_empty() {
            continue;
        }
        let status = status_record
            .first()
            .copied()
            .ok_or_else(malformed_status)?;
        if matches!(status, b'R' | b'C') {
            let old_path = records.get(index).ok_or_else(malformed_status)?;
            let path = records.get(index + 1).ok_or_else(malformed_status)?;
            index += 2;
            let path = path_text(path)?;
            files.push(GitChangedFile {
                path,
                old_path: Some(path_text(old_path)?),
                status: String::from_utf8_lossy(&status_record[..1]).into_owned(),
            });
        } else {
            let path = records.get(index).ok_or_else(malformed_status)?;
            index += 1;
            let path = path_text(path)?;
            files.push(GitChangedFile {
                path,
                old_path: None,
                status: String::from_utf8_lossy(&status_record[..1]).into_owned(),
            });
        }
    }
    Ok(files)
}

fn path_after_fields(record: &[u8], field_count: usize) -> Option<&[u8]> {
    let mut remaining = record;
    for _ in 0..field_count {
        let separator = remaining.iter().position(|byte| *byte == b' ')?;
        remaining = &remaining[separator + 1..];
    }
    Some(remaining)
}

fn status_from_xy(xy: &[u8]) -> String {
    let status = String::from_utf8_lossy(xy).trim().to_owned();
    if status.is_empty() {
        "M".to_owned()
    } else {
        status
    }
}

fn path_text(path: &[u8]) -> Result<String, IpcError> {
    String::from_utf8(path.to_vec())
        .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
        .map_err(|_| {
            IpcError::new(
                IpcErrorCode::InvalidPath,
                "Git paths that are not valid UTF-8 cannot be displayed in Changes",
            )
        })
}

fn malformed_status() -> IpcError {
    IpcError::new(
        IpcErrorCode::GitFailed,
        "Git returned an invalid porcelain status record",
    )
}

fn validate_relative_path(path: &str) -> Result<PathBuf, IpcError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            ) || matches!(component, Component::Normal(name) if name == ".git")
        })
    {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "diff paths must be relative to the checkout and cannot access .git",
        ));
    }
    Ok(path.to_path_buf())
}

fn validate_diff_target(context: &GitContext, path: &Path) -> Result<(), IpcError> {
    let mut parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    loop {
        match resolve_checkout_path(&context.repo, &context.checkout.id, parent) {
            Ok(_) => break,
            Err(error) if matches!(error.code, IpcErrorCode::FolderMissing) => {
                parent = parent
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                if parent == Path::new(".") {
                    resolve_checkout_path(&context.repo, &context.checkout.id, parent)?;
                    break;
                }
            }
            Err(error) => return Err(error),
        }
    }
    let target = context.root.join(path);
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => {
            resolve_checkout_path(&context.repo, &context.checkout.id, path)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(IpcError::new(
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                IpcErrorCode::PermissionDenied
            } else {
                IpcErrorCode::OperationFailed
            },
            format!("could not inspect diff path: {error}"),
        )),
    }
}

fn literal_pathspec(path: &str) -> OsString {
    OsString::from(format!(":(literal){path}"))
}

fn optional_git_text<const N: usize>(root: &Path, args: [&str; N]) -> Option<String> {
    checked_git(root, args, "Git query failed")
        .ok()
        .map(|output| output_text(&output))
        .filter(|text| !text.is_empty())
}

fn checked_git<const N: usize>(
    root: &Path,
    args: [&str; N],
    action: &str,
) -> Result<Output, IpcError> {
    let output = run_git(root, args.into_iter().map(OsString::from).collect())?;
    if !output.status.success() {
        return Err(git_error(action, &output));
    }
    Ok(output)
}

fn run_git(root: &Path, args: Vec<OsString>) -> Result<Output, IpcError> {
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

fn git_error(action: &str, output: &Output) -> IpcError {
    IpcError::new(
        IpcErrorCode::GitFailed,
        format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    )
}

fn output_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command, sync::mpsc, time::Duration};

    use notify::Watcher;
    use tempfile::tempdir;

    use crate::{
        domain::{ipc::IpcErrorCode, workspace::RepoKind},
        persistence::Database,
        services::workspace,
    };

    use super::{
        diff, parse_name_status, parse_porcelain_v2, receive_debounced_change, resolve_default_ref,
        status, WatchMessage, WatchWakeup,
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

    fn init_repo(root: &Path) {
        fs::create_dir_all(root).unwrap();
        git(root, &["init", "-b", "trunk"]);
        git(root, &["config", "user.name", "Marvis test"]);
        git(root, &["config", "user.email", "marvis@example.invalid"]);
        fs::write(root.join("base.txt"), "base\n").unwrap();
        git(root, &["add", "base.txt"]);
        git(root, &["commit", "-m", "base"]);
    }

    fn git_database(temp: &Path, root: &Path) -> (Database, String) {
        let database = Database::open(temp.join("workspace.sqlite3")).unwrap();
        let state = workspace::register_folder(&database, root).unwrap();
        let checkout_id = state.repos[0].checkouts[0].id.clone();
        workspace::set_default_branch(&database, &state.repos[0].id, "trunk").unwrap();
        (database, checkout_id)
    }

    #[test]
    fn lists_feature_commits_from_merge_base_without_default_branch_commits() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        git(&root, &["checkout", "-b", "feature"]);
        fs::write(root.join("feature.txt"), "feature\n").unwrap();
        git(&root, &["add", "feature.txt"]);
        git(&root, &["commit", "-m", "feature"]);
        git(&root, &["checkout", "trunk"]);
        fs::write(root.join("default-only.txt"), "default\n").unwrap();
        git(&root, &["add", "default-only.txt"]);
        git(&root, &["commit", "-m", "new default commit"]);
        git(&root, &["checkout", "feature"]);
        fs::write(root.join("feature.txt"), "feature edited\n").unwrap();
        let (database, checkout_id) = git_database(temp.path(), &root);

        let snapshot = status(&database, &checkout_id).unwrap();

        assert!(snapshot.files.iter().any(|file| file.path == "feature.txt"));
        assert_eq!(
            snapshot
                .files
                .iter()
                .filter(|file| file.path == "feature.txt")
                .count(),
            1
        );
        assert!(!snapshot
            .files
            .iter()
            .any(|file| file.path == "default-only.txt"));
        assert_eq!(snapshot.ahead_count, 1);
        assert_eq!(snapshot.default_branch, "trunk");
    }

    #[test]
    fn staged_and_unstaged_edits_and_untracked_files_are_listed_once_and_diffed_as_one_change() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::write(root.join("base.txt"), "staged\n").unwrap();
        git(&root, &["add", "base.txt"]);
        fs::write(root.join("base.txt"), "unstaged final\n").unwrap();
        fs::write(root.join("new file.txt"), "untracked\n").unwrap();
        fs::write(root.join("new binary.dat"), [0, 1, 2]).unwrap();

        let snapshot = status(&database, &checkout_id).unwrap();
        let paths: Vec<_> = snapshot
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(paths, ["base.txt", "new binary.dat", "new file.txt"]);

        let tracked = diff(&database, &checkout_id, "base.txt").unwrap();
        assert!(tracked.patch.contains("+unstaged final"));
        assert!(!tracked.patch.contains("+staged\n"));
        assert_eq!(tracked.patch.matches("diff --git").count(), 1);
        let untracked = diff(&database, &checkout_id, "new file.txt").unwrap();
        assert!(untracked.patch.contains("+untracked"));
        let binary = diff(&database, &checkout_id, "new binary.dat").unwrap();
        assert!(binary.is_binary);
        assert!(binary.patch.is_empty());
    }

    #[test]
    fn reports_binary_changes_without_returning_binary_patch_text() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        fs::write(root.join("binary.dat"), [0, 1, 2]).unwrap();
        git(&root, &["add", "binary.dat"]);
        git(&root, &["commit", "-m", "add binary"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::write(root.join("binary.dat"), [0, 2, 3]).unwrap();

        let result = diff(&database, &checkout_id, "binary.dat").unwrap();

        assert!(result.is_binary);
        assert!(result.patch.is_empty());
    }

    #[test]
    fn deleted_nested_files_still_have_diffs() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/removed.txt"), "remove me\n").unwrap();
        git(&root, &["add", "nested/removed.txt"]);
        git(&root, &["commit", "-m", "nested file"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::remove_dir_all(root.join("nested")).unwrap();

        let result = diff(&database, &checkout_id, "nested/removed.txt").unwrap();

        assert!(result.patch.contains("-remove me"));
    }

    #[test]
    fn plain_folders_do_not_run_git_and_changes_are_rejected() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("plain");
        fs::create_dir_all(&root).unwrap();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let state = workspace::register_folder(&database, &root).unwrap();
        assert_eq!(state.repos[0].kind, RepoKind::Plain);

        let error = status(&database, &state.repos[0].checkouts[0].id).unwrap_err();

        assert!(matches!(error.code, IpcErrorCode::InvalidCheckout));
    }

    #[test]
    fn resolves_only_a_valid_persisted_branch_or_origin_head() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);

        let unknown = resolve_default_ref(&root, None).unwrap_err();
        assert!(matches!(unknown.code, IpcErrorCode::DefaultBranchUnknown));

        git(&root, &["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
        git(
            &root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/trunk",
            ],
        );
        let resolved = resolve_default_ref(&root, None).unwrap();

        assert_eq!(resolved.branch, "trunk");
        assert_eq!(resolved.reference, "refs/remotes/origin/trunk");
    }

    #[test]
    fn porcelain_v2_parses_unusual_names_and_renames_without_splitting_paths() {
        let parsed = parse_porcelain_v2(
            b"1 .M N... 100644 100644 100644 abc def file with spaces.txt\0? new\nline.txt\0",
        )
        .unwrap();
        assert!(parsed.contains_key("file with spaces.txt"));
        assert!(parsed.contains_key("new\nline.txt"));

        let rename = parse_name_status(b"R100\0old name.txt\0new name.txt\0").unwrap();
        assert_eq!(rename[0].old_path.as_deref(), Some("old name.txt"));
        assert_eq!(rename[0].path, "new name.txt");
    }

    #[test]
    fn watcher_debounces_a_burst_into_one_refresh_signal() {
        let (sender, receiver) = mpsc::channel();
        sender.send(WatchMessage::Changed).unwrap();
        sender.send(WatchMessage::Changed).unwrap();
        sender.send(WatchMessage::Changed).unwrap();

        assert_eq!(
            receive_debounced_change(&receiver, Duration::from_millis(1)),
            WatchWakeup::Changed
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn checkout_watcher_observes_file_edits() {
        let temp = tempdir().unwrap();
        let (sender, receiver) = mpsc::channel();
        let event_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if result.is_ok() {
                    let _ = event_sender.send(WatchMessage::Changed);
                }
            })
            .unwrap();
        watcher
            .watch(temp.path(), notify::RecursiveMode::Recursive)
            .unwrap();

        fs::write(temp.path().join("terminal-edit.txt"), "changed\n").unwrap();

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            WatchMessage::Changed
        );
    }
}
