use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{self, BufRead, BufReader},
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
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
const FILE_ACTIVITY_EVENT: &str = "checkout-file-activity";
const SMALL_DIFF_LINES: usize = 1000;
const SMALL_DIFF_BYTES: usize = 512 * 1024;
const MAX_DIFF_LINES: usize = 100_000;
const MAX_DIFF_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIFF_LINE_BYTES: usize = 64 * 1024;
const MAX_DIFF_HUNKS: usize = 10_000;
pub const MAX_DIFF_PAGE_LINES: usize = 32;

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
    pub large: bool,
    pub too_large: bool,
    pub total_lines: usize,
    pub hunks: Vec<GitDiffHunk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffHunk {
    pub start_line: usize,
    pub end_line: usize,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffPageLine {
    pub index: usize,
    pub kind: GitDiffLineKind,
    pub text: String,
    pub old_line_number: Option<u64>,
    pub new_line_number: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitDiffLineKind {
    Hunk,
    Added,
    Removed,
    Context,
    Meta,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffPage {
    pub path: String,
    pub start_line: usize,
    pub total_lines: usize,
    pub lines: Vec<GitDiffPageLine>,
}

#[derive(Default)]
pub struct GitWatcherManager {
    watchers: Mutex<BTreeMap<String, (RecommendedWatcher, mpsc::SyncSender<WatchMessage>)>>,
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

        let (sender, receiver) = mpsc::sync_channel(1);
        let event_sender = sender.clone();
        let checkout_root = paths.first().cloned().unwrap_or_default();
        let file_activity_pending = Arc::new(AtomicBool::new(false));
        let callback_activity_pending = file_activity_pending.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if let Ok(event) = result {
                    if event
                        .paths
                        .iter()
                        .any(|path| is_checkout_file_activity(path, &checkout_root))
                    {
                        callback_activity_pending.store(true, Ordering::Relaxed);
                    }
                    if should_refresh_for_event(&event) {
                        let _ = event_sender.try_send(WatchMessage::Changed);
                    }
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
        let activity_checkout_id = checkout_id.clone();
        let worker_activity_pending = file_activity_pending;
        thread::Builder::new()
            .name("marvis-git-watch".into())
            .spawn(move || {
                while let WatchWakeup::Changed = receive_debounced_change(&receiver, WATCH_DEBOUNCE)
                {
                    let _ = app.emit(STATUS_CHANGED_EVENT, &event_checkout_id);
                    if worker_activity_pending.swap(false, Ordering::Relaxed) {
                        let _ = app.emit(FILE_ACTIVITY_EVENT, &activity_checkout_id);
                    }
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

fn should_refresh_for_event(event: &notify::Event) -> bool {
    event.paths.iter().any(|path| should_refresh_path(path))
}

fn should_refresh_path(path: &Path) -> bool {
    let components: Vec<_> = path.components().collect();
    let Some(git_directory) = components
        .iter()
        .position(|component| matches!(component, Component::Normal(name) if *name == ".git"))
    else {
        return true;
    };
    let metadata = &components[git_directory + 1..];
    let ends_with =
        |name: &str| matches!(metadata.last(), Some(Component::Normal(value)) if *value == name);
    let head_changed = (metadata.len() == 1 && ends_with("HEAD"))
        || (metadata.len() >= 3
            && matches!(metadata[metadata.len() - 3], Component::Normal(name) if name == "worktrees")
            && ends_with("HEAD"));
    let index_changed = ends_with("index") || ends_with("index.lock");
    let packed_refs_changed = metadata.len() == 1 && ends_with("packed-refs");
    head_changed
        || index_changed
        || packed_refs_changed
        || metadata.windows(2).any(|pair| {
        matches!(pair[0], Component::Normal(name) if name == "refs")
            && matches!(pair[1], Component::Normal(name) if matches!(name.to_str(), Some("heads" | "remotes")))
        })
}

fn is_checkout_file_activity(path: &Path, checkout_root: &Path) -> bool {
    path.strip_prefix(checkout_root)
        .ok()
        .and_then(|relative| relative.components().next())
        .is_some_and(|component| !matches!(component, Component::Normal(name) if name == ".git"))
}

pub fn status(database: &Database, checkout_id: &str) -> Result<GitStatus, IpcError> {
    Ok(snapshot(&registered_git_context(database, checkout_id)?)?.status)
}

pub fn diff(database: &Database, checkout_id: &str, path: &str) -> Result<GitFileDiff, IpcError> {
    let context = registered_git_context(database, checkout_id)?;
    let snapshot = snapshot(&context)?;
    let changed_file = changed_file(&snapshot.status, path)?;
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
            large: false,
            too_large: false,
            total_lines: 0,
            hunks: Vec::new(),
            symlink_target: Some(target.to_string_lossy().into_owned()),
        });
    }

    let scan = scan_git_diff(
        &context.root,
        diff_args(&context, &snapshot, &changed_file, path)?,
        None,
    )?;
    ensure_diff_succeeded(&scan.output, changed_file.status == "??", scan.too_large)?;
    Ok(GitFileDiff {
        path: path.to_owned(),
        patch: if scan.large || scan.too_large || scan.is_binary {
            String::new()
        } else {
            String::from_utf8_lossy(&scan.patch).into_owned()
        },
        is_binary: scan.is_binary,
        large: scan.large,
        too_large: scan.too_large,
        total_lines: scan.total_lines,
        hunks: scan.hunks,
        symlink_target: None,
    })
}

pub fn diff_page(
    database: &Database,
    checkout_id: &str,
    path: &str,
    offset: usize,
    limit: usize,
) -> Result<GitDiffPage, IpcError> {
    if limit == 0 || limit > MAX_DIFF_PAGE_LINES || offset > MAX_DIFF_LINES {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "diff page range is outside the allowed limit",
        ));
    }
    let context = registered_git_context(database, checkout_id)?;
    let snapshot = snapshot(&context)?;
    let changed_file = changed_file(&snapshot.status, path)?;
    let relative_path = validate_relative_path(path)?;
    validate_diff_target(&context, &relative_path)?;
    if fs::symlink_metadata(context.root.join(&relative_path))
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "symlink diffs do not have text pages",
        ));
    }
    let scan = scan_git_diff(
        &context.root,
        diff_args(&context, &snapshot, &changed_file, path)?,
        Some((offset, limit)),
    )?;
    ensure_diff_succeeded(&scan.output, changed_file.status == "??", scan.too_large)?;
    if scan.is_binary || scan.too_large {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "this diff does not have accessible text pages",
        ));
    }
    Ok(GitDiffPage {
        path: path.to_owned(),
        start_line: offset,
        total_lines: scan.total_lines,
        lines: scan.page,
    })
}

pub fn viewed_files(database: &Database, checkout_id: &str) -> Result<Vec<String>, IpcError> {
    registered_git_context(database, checkout_id)?;
    database
        .viewed_files(checkout_id)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
}

pub fn mark_viewed(database: &Database, checkout_id: &str, path: &str) -> Result<(), IpcError> {
    let context = registered_git_context(database, checkout_id)?;
    let snapshot = snapshot(&context)?;
    changed_file(&snapshot.status, path)?;
    let relative_path = validate_relative_path(path)?;
    validate_diff_target(&context, &relative_path)?;
    database
        .mark_file_viewed(checkout_id, path)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
}

fn changed_file(status: &GitStatus, path: &str) -> Result<GitChangedFile, IpcError> {
    status
        .files
        .iter()
        .find(|file| file.path == path)
        .cloned()
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidPath,
                "requested path is not a changed file in this checkout",
            )
        })
}

fn diff_args(
    context: &GitContext,
    snapshot: &Snapshot,
    changed_file: &GitChangedFile,
    path: &str,
) -> Result<Vec<OsString>, IpcError> {
    if changed_file.status == "??" {
        let relative_path = validate_relative_path(path)?;
        Ok(vec![
            "diff".into(),
            "--no-index".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--no-color".into(),
            "--unified=3".into(),
            "--".into(),
            "/dev/null".into(),
            context.root.join(relative_path).into_os_string(),
        ])
    } else {
        let mut args = vec![
            "diff".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--no-color".into(),
            "--find-renames".into(),
            "--unified=3".into(),
            snapshot.merge_base.clone().into(),
            "--".into(),
        ];
        if let Some(old_path) = &changed_file.old_path {
            args.push(literal_pathspec(old_path));
        }
        args.push(literal_pathspec(path));
        Ok(args)
    }
}

struct ScannedDiff {
    output: Output,
    patch: Vec<u8>,
    page: Vec<GitDiffPageLine>,
    hunks: Vec<GitDiffHunk>,
    total_lines: usize,
    is_binary: bool,
    large: bool,
    too_large: bool,
}

fn scan_git_diff(
    root: &Path,
    args: Vec<OsString>,
    page_range: Option<(usize, usize)>,
) -> Result<ScannedDiff, IpcError> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not start Git: {error}"),
            )
        })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);
    let mut patch = Vec::with_capacity(SMALL_DIFF_BYTES.min(64 * 1024));
    let mut page = Vec::new();
    let mut hunks: Vec<GitDiffHunk> = Vec::new();
    let mut total_lines = 0;
    let mut total_bytes = 0;
    let mut old_line = None;
    let mut new_line = None;
    let mut is_binary = false;
    let mut large = false;
    let mut too_large = false;
    loop {
        let raw_line = match read_diff_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                too_large = true;
                break;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(IpcError::new(
                    IpcErrorCode::GitFailed,
                    format!("could not read Git diff: {error}"),
                ));
            }
        };
        total_bytes += raw_line.len();
        if total_bytes > MAX_DIFF_BYTES {
            too_large = true;
            break;
        }
        if raw_line.starts_with(b"@@") && raw_line.len() > 4096 {
            too_large = true;
            break;
        }
        is_binary |=
            raw_line.starts_with(b"Binary files ") || raw_line.starts_with(b"GIT binary patch");
        if !large && patch.len() + raw_line.len() <= SMALL_DIFF_BYTES {
            patch.extend_from_slice(&raw_line);
        } else if !large {
            large = true;
            patch.clear();
        }
        if let Some(mut line) = parse_diff_display_line(&raw_line, &mut old_line, &mut new_line) {
            if matches!(line.kind, GitDiffLineKind::Hunk) {
                if let Some(previous) = hunks.last_mut() {
                    previous.end_line = total_lines;
                }
                if hunks.len() == MAX_DIFF_HUNKS {
                    too_large = true;
                    break;
                }
                hunks.push(GitDiffHunk {
                    start_line: total_lines,
                    end_line: 0,
                    title: line.text.clone(),
                });
            }
            line.index = total_lines;
            if page_range.is_some_and(|(start, count)| {
                total_lines >= start && total_lines < start.saturating_add(count)
            }) {
                page.push(line);
            }
            total_lines += 1;
            if total_lines > MAX_DIFF_LINES {
                too_large = true;
                break;
            }
            if total_lines > SMALL_DIFF_LINES && !large {
                large = true;
                patch.clear();
            }
        }
    }
    if too_large {
        let _ = child.kill();
    }
    let output = child.wait_with_output().map_err(|error| {
        IpcError::new(
            IpcErrorCode::GitFailed,
            format!("could not finish Git diff: {error}"),
        )
    })?;
    if let Some(hunk) = hunks.last_mut() {
        hunk.end_line = total_lines;
    }
    Ok(ScannedDiff {
        output,
        patch,
        page,
        hunks,
        total_lines,
        is_binary,
        large: large || too_large,
        too_large,
    })
}

fn read_diff_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let has_newline = available.contains(&b'\n');
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if line.len() + count > MAX_DIFF_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "diff line exceeds the size limit",
            ));
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if has_newline {
            return Ok(Some(line));
        }
    }
}

fn parse_diff_display_line(
    raw_line: &[u8],
    old_line: &mut Option<u64>,
    new_line: &mut Option<u64>,
) -> Option<GitDiffPageLine> {
    let text = String::from_utf8_lossy(raw_line)
        .trim_end_matches(['\n', '\r'])
        .to_owned();
    let (kind, old_number, new_number) = if text.starts_with("@@ ") {
        let Some((old_start, new_start)) = hunk_start_lines(&text) else {
            *old_line = None;
            *new_line = None;
            return None;
        };
        *old_line = Some(old_start);
        *new_line = Some(new_start);
        (GitDiffLineKind::Hunk, None, None)
    } else if old_line.is_some() && text.starts_with('-') {
        let line = *old_line;
        *old_line = old_line.map(|value| value.saturating_add(1));
        (GitDiffLineKind::Removed, line, None)
    } else if old_line.is_some() && text.starts_with('+') {
        let line = *new_line;
        *new_line = new_line.map(|value| value.saturating_add(1));
        (GitDiffLineKind::Added, None, line)
    } else if old_line.is_some() && text.starts_with(' ') {
        let old = *old_line;
        let new = *new_line;
        *old_line = old_line.map(|value| value.saturating_add(1));
        *new_line = new_line.map(|value| value.saturating_add(1));
        (GitDiffLineKind::Context, old, new)
    } else if text.starts_with('\\') && old_line.is_some() {
        (GitDiffLineKind::Meta, None, None)
    } else {
        return None;
    };
    Some(GitDiffPageLine {
        index: 0,
        kind,
        text,
        old_line_number: old_number,
        new_line_number: new_number,
    })
}

fn hunk_start_lines(header: &str) -> Option<(u64, u64)> {
    fn range_start(field: &str, marker: char) -> Option<u64> {
        let range = field.strip_prefix(marker)?;
        let (start, count) = range.split_once(',').unwrap_or((range, "1"));
        let start = start.parse::<u64>().ok()?;
        let count = count.parse::<u64>().ok()?;
        start.checked_add(count)?;
        Some(start)
    }

    let mut fields = header.split_whitespace();
    if fields.next()? != "@@" {
        return None;
    }
    let old_start = range_start(fields.next()?, '-')?;
    let new_start = range_start(fields.next()?, '+')?;
    (fields.next()? == "@@").then_some((old_start, new_start))
}

fn ensure_diff_succeeded(
    output: &Output,
    untracked: bool,
    too_large: bool,
) -> Result<(), IpcError> {
    if !too_large && !output.status.success() && !(untracked && output.status.code() == Some(1)) {
        return Err(git_error("could not diff changed file", output));
    }
    Ok(())
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
    let default = resolve_default_ref(
        &context.root,
        context.repo.default_branch.as_deref(),
        Some(Path::new(&context.repo.root)),
    )?;
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
pub(crate) struct DefaultRef {
    pub(crate) reference: String,
    pub(crate) branch: String,
}

pub(crate) fn resolve_default_ref(
    root: &Path,
    persisted: Option<&str>,
    primary_root: Option<&Path>,
) -> Result<DefaultRef, IpcError> {
    if let Some(branch) = persisted.map(|branch| branch.strip_prefix("origin/").unwrap_or(branch)) {
        if let Some(default) = resolve_branch_ref(root, branch) {
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
                if let Some(default) = resolve_branch_ref(root, branch) {
                    return Ok(default);
                }
            }
        }
    }

    if let Some(primary_root) = primary_root
        .filter(|path| path.is_dir())
        .filter(|path| share_git_common_dir(root, path))
    {
        if let Some(branch) =
            optional_git_text(primary_root, ["symbolic-ref", "--quiet", "--short", "HEAD"])
        {
            if let Some(default) = resolve_branch_ref(root, &branch) {
                return Ok(default);
            }
        }
    }

    Err(IpcError::new(
        IpcErrorCode::DefaultBranchUnknown,
        "Git could not determine a valid default branch; choose one before viewing changes",
    ))
}

fn share_git_common_dir(first: &Path, second: &Path) -> bool {
    fn common_dir(path: &Path) -> Option<PathBuf> {
        let text = optional_git_text(path, ["rev-parse", "--git-common-dir"])?;
        let common = Path::new(&text);
        if common.is_absolute() {
            common.canonicalize().ok()
        } else {
            path.join(common).canonicalize().ok()
        }
    }
    common_dir(first)
        .zip(common_dir(second))
        .is_some_and(|(first, second)| first == second)
}

pub(crate) fn resolve_branch_ref(root: &Path, branch: &str) -> Option<DefaultRef> {
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

pub(crate) fn parse_porcelain_v2(
    output: &[u8],
) -> Result<BTreeMap<String, GitChangedFile>, IpcError> {
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
        diff, diff_page, parse_diff_display_line, parse_name_status, parse_porcelain_v2,
        receive_debounced_change, resolve_default_ref, should_refresh_path, status, WatchMessage,
        WatchWakeup,
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

    #[test]
    fn malformed_hunk_headers_do_not_attach_following_lines_to_partial_hunks() {
        let mut old_line = Some(1);
        let mut new_line = Some(1);
        assert!(parse_diff_display_line(
            b"@@ -broken +2 @@ partial\n",
            &mut old_line,
            &mut new_line,
        )
        .is_none());
        assert_eq!(old_line, None);
        assert_eq!(new_line, None);
        assert!(parse_diff_display_line(
            b"+must not be assigned a line number\n",
            &mut old_line,
            &mut new_line,
        )
        .is_none());
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
    fn sec_03_git_keeps_shell_metacharacters_and_newlines_in_a_single_path_argument() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let filename = "space 'quote' \"double\" ; $(touch pwned)
line.txt";
        fs::write(root.join(filename), "literal file content\n").unwrap();
        let (database, checkout_id) = git_database(temp.path(), &root);

        let status = status(&database, &checkout_id).unwrap();
        assert!(status.files.iter().any(|file| file.path == filename));
        let diff = diff(&database, &checkout_id, filename).unwrap();

        assert!(diff.patch.contains("+literal file content"));
        assert!(!temp.path().join("pwned").exists());
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
    fn large_diff_pages_are_bounded_and_reach_the_last_changed_line() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let original = (0..5000)
            .map(|line| format!("old-{line}\n"))
            .collect::<String>();
        fs::write(root.join("large.txt"), original).unwrap();
        git(&root, &["add", "large.txt"]);
        git(&root, &["commit", "-m", "large file"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        let changed = (0..5000)
            .map(|line| format!("new-{line}\n"))
            .collect::<String>();
        fs::write(root.join("large.txt"), changed).unwrap();

        let result = diff(&database, &checkout_id, "large.txt").unwrap();

        assert!(result.large);
        assert!(!result.too_large);
        assert!(result.total_lines > 4000);
        assert!(result.patch.is_empty());

        let offset = result.total_lines - 8;
        let page = diff_page(&database, &checkout_id, "large.txt", offset, 8).unwrap();
        assert_eq!(page.start_line, offset);
        assert_eq!(page.lines.len(), 8);
        assert!(page.lines.iter().all(|line| line.index >= offset));
        assert!(page.lines.iter().any(|line| line.text == "+new-4999"));
    }

    #[test]
    fn viewed_state_is_scoped_to_registered_changed_files() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::write(root.join("base.txt"), "changed\n").unwrap();

        super::mark_viewed(&database, &checkout_id, "base.txt").unwrap();
        assert_eq!(
            super::viewed_files(&database, &checkout_id).unwrap(),
            ["base.txt"]
        );
        assert!(super::mark_viewed(&database, &checkout_id, "not-changed.txt").is_err());
        assert!(super::mark_viewed(&database, &checkout_id, "../outside.txt").is_err());
        assert!(super::viewed_files(&database, "checkout:unknown").is_err());
        assert!(diff_page(&database, &checkout_id, "base.txt", 0, 33).is_err());
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

        let unknown = resolve_default_ref(&root, None, None).unwrap_err();
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
        let resolved = resolve_default_ref(&root, None, None).unwrap();

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
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.try_send(WatchMessage::Changed).unwrap();
        assert!(sender.try_send(WatchMessage::Changed).is_err());
        assert!(sender.try_send(WatchMessage::Changed).is_err());

        assert_eq!(
            receive_debounced_change(&receiver, Duration::from_millis(1)),
            WatchWakeup::Changed
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn checkout_watcher_observes_file_edits() {
        let temp = tempdir().unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let event_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if result.is_ok_and(|event| super::should_refresh_for_event(&event)) {
                    let _ = event_sender.try_send(WatchMessage::Changed);
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

    #[test]
    fn watcher_ignores_git_object_churn_but_tracks_status_metadata_and_checkout_files() {
        assert!(!should_refresh_path(Path::new(
            "/repo/.git/objects/aa/object"
        )));
        assert!(!should_refresh_path(Path::new("/repo/.git/logs/HEAD")));
        assert!(should_refresh_path(Path::new("/repo/.git/index")));
        assert!(should_refresh_path(Path::new(
            "/repo/.git/worktrees/task/HEAD"
        )));
        assert!(should_refresh_path(Path::new(
            "/repo/.git/refs/heads/feature"
        )));
        assert!(should_refresh_path(Path::new("/repo/src/file.rs")));
    }

    #[test]
    fn file_activity_only_signals_paths_inside_checkout_outside_git_metadata() {
        let root = Path::new("/repo");
        assert!(super::is_checkout_file_activity(
            Path::new("/repo/src/file.rs"),
            root
        ));
        assert!(!super::is_checkout_file_activity(Path::new("/repo"), root));
        assert!(!super::is_checkout_file_activity(
            Path::new("/repo/.git/index"),
            root
        ));
        assert!(!super::is_checkout_file_activity(
            Path::new("/other/file.rs"),
            root
        ));
    }
}
