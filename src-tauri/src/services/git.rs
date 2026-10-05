use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::{self, BufRead, BufReader, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
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
const WATCH_MAX_BATCH: Duration = Duration::from_secs(1);
const STATUS_CHANGED_EVENT: &str = "git-status-changed";
const FILE_ACTIVITY_EVENT: &str = "checkout-file-activity";
const WORKTREES_CHANGED_EVENT: &str = "git-worktrees-changed";
const WATCH_FAILED_EVENT: &str = "git-watch-failed";
const SMALL_DIFF_LINES: usize = 1000;
const SMALL_DIFF_BYTES: usize = 512 * 1024;
const MAX_DIFF_LINES: usize = 100_000;
const MAX_DIFF_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIFF_LINE_BYTES: usize = 64 * 1024;
const MAX_DIFF_HUNKS: usize = 10_000;
/// How much of a file is read looking for the NUL byte that makes Git call it binary.
const BINARY_SNIFF_BYTES: usize = 8_000;
/// How much of a file's own text a diff carries so its syntax can be read in context.
///
/// A grammar reads a file, not a hunk: the lines inside `<script setup lang="ts">` are markup to a
/// grammar that never saw the opening tag, so a patch on its own highlights the wrong language or
/// none at all. The text is what the grammar reads and not what the diff draws (the patch still
/// draws every line). So the cap is a cap on the reading of one file, and past it the file is read
/// the way this app read every file before any of this. `SMALL_DIFF_BYTES` is the same size a patch
/// is allowed to be, so a file that small in this repository is carried whole.
const MAX_SYNTAX_CONTEXT_BYTES: usize = SMALL_DIFF_BYTES;
/// How many checkouts are read at once when the sidebar names all of them. Each reading is a
/// handful of `git` processes, so a workspace with many worktrees would otherwise fork all of
/// them into the machine in the same moment. The point is to overlap the work, not to let the
/// work starve itself.
const PARALLEL_READS: usize = 8;
pub const MAX_DIFF_PAGE_LINES: usize = 32;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitChangedFile {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
    /// Absent rather than zero when Git has no count for the file: `--numstat` prints `-` for
    /// both columns of one it cannot measure, a binary file among them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u64>,
}

/// The lines a change set adds and removes.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffStats {
    pub additions: u64,
    pub deletions: u64,
}

/// The totals of every registered Git checkout, keyed by checkout id.
pub type GitCheckoutDiffStats = BTreeMap<String, GitDiffStats>;

/// The changed files of one checkout, each with the lines it adds and removes.
pub type GitFileDiffStats = Vec<GitChangedFile>;

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
    /// The text of each side of the diff, whole, which is what a grammar reads rather than the
    /// patch's hunks. Absent rather than empty when there is no text to read: a binary or symlink
    /// diff, a diff too large to hold, a side that does not exist (an added file's old side, a
    /// removed file's new one), and a file past `MAX_SYNTAX_CONTEXT_BYTES`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,
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
    /// One watcher per repository, not per checkout. The worktrees of a repo share a Git
    /// directory, so watching them one at a time means every write in it is reported once per
    /// worktree, the kernel walks the same tree the same number of times, and a commit still
    /// leaves the siblings holding numbers taken against a merge base that has moved.
    watchers: Arc<Mutex<BTreeMap<String, RepoWatcher>>>,
    /// What each checkout was when it was last read, kept here because the same debounced
    /// change that refreshes the file list is what makes it stale. `Arc` so a watch thread can
    /// mark its own entry without borrowing the manager.
    snapshots: Arc<GitSnapshotCache>,
}

struct RepoWatcher {
    _watcher: RecommendedWatcher,
    sender: mpsc::Sender<WatchMessage>,
    failed: Arc<AtomicBool>,
    token: Arc<()>,
    /// The renderer's request generation, echoed by runtime failure events.
    registration_id: String,
    /// The plan this watcher was built for, so a worktree joining or leaving the repository is
    /// answered by a watcher that knows about it rather than by the one that predates it.
    plan: RepoWatchPlan,
}

/// What one repository's watcher has to know to say which checkouts a change speaks for.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct RepoWatchPlan {
    /// The repository this plan watches, named in the one signal no checkout can speak for.
    pub repo_id: String,
    /// Each live worktree's directory, mapped to the checkout that lives in it, so a file
    /// written there is attributed to one row rather than to the whole repo.
    pub roots: BTreeMap<PathBuf, String>,
    /// Each worktree's own Git directory, which holds its index and its HEAD. Staging in one
    /// worktree is that worktree's business; only a shared ref is the repository's.
    pub git_dirs: BTreeMap<PathBuf, String>,
    /// The Git directory the worktrees share. A ref or index write there moves the merge base
    /// every sibling diffs against, so it speaks for all of them and not for one.
    pub common_dir: Option<PathBuf>,
    /// Every checkout whose Git context this watcher can actually monitor.
    pub all: Vec<String>,
    /// Live checkout IDs captured before resolving their paths. Runtime failure signals use these
    /// as their request identity, including checkouts the watcher could not observe.
    pub requested: Vec<String>,
}

/// The checkouts a batch of filesystem changes speaks for: the ones whose own files moved, the
/// whole repository when what moved was a ref every one of them reads, and the repository whose
/// own list of worktrees moved.
#[derive(Debug, Default, PartialEq, Eq)]
struct WatchUpdate {
    status: Vec<String>,
    activity: Vec<String>,
    /// Repositories a worktree joined or left. The rows a repository is made of are not a change
    /// any checkout can speak for, so the repository is named and its registration is read again
    /// rather than refreshed.
    worktrees: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum WatchMessage {
    Changed(WatchUpdate),
    Failed(String),
    Stop,
}

#[derive(Debug, PartialEq, Eq)]
enum WatchWakeup {
    Changed(WatchUpdate),
    Failed(String),
    Stop,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchFailure {
    repo_id: String,
    checkout_ids: Vec<String>,
    registration_id: String,
}

#[derive(Clone)]
struct GitContext {
    repo: Repo,
    checkout: Checkout,
    root: PathBuf,
}

#[derive(Clone)]
struct Snapshot {
    status: GitStatus,
    merge_base: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitCounts {
    files: GitFileDiffStats,
    totals: GitDiffStats,
}

/// One checkout snapshot, kept next to the watchers because reading one costs several `git`
/// processes and the sidebar asks about every checkout at once.
///
/// The file list, the row totals and the merge base they were taken against are all views of
/// the same reading, so one entry answers all three and a change drops it whole. `git status`
/// and the counts of one checkout are therefore the same snapshot rather than two readings of
/// the same working tree.
///
/// Every Git checkout in the workspace is watched, so an entry is invalidated by a real change
/// signal. That is what makes holding one honest, and it is why a checkout the user is not
/// looking at carries numbers as fresh as the one they are.
#[derive(Clone)]
struct CachedGitSnapshot {
    /// Choosing another default branch moves the base ref, which no watcher can see, so the
    /// entry is matched against it instead of invalidated.
    default_branch: Option<String>,
    revision: u64,
    stale: bool,
    snapshot: Snapshot,
    /// Filled in the first time something asks for the lines rather than the file list, and
    /// read back by the next ask. The sidebar and the Changes tab both want them on every
    /// change and neither should pay for the other: a checkout nobody touched is a lookup.
    counts: Option<GitCounts>,
}

#[derive(Default)]
struct GitSnapshotCache {
    entries: Mutex<BTreeMap<String, CachedGitSnapshot>>,
    revisions: Mutex<BTreeMap<String, u64>>,
}

impl GitWatcherManager {
    pub fn watch<R: tauri::Runtime>(
        &self,
        app: AppHandle<R>,
        repo_id: String,
        registration_id: String,
        plan: RepoWatchPlan,
    ) -> Result<(), IpcError> {
        let mut watchers = self
            .watchers
            .lock()
            .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error.to_string()))?;
        // A repository that is already watched for the plan it was given has nothing to ask for.
        // A plan that grew is another matter: the worktree that just joined has a directory and a
        // Git directory nothing is watching yet, so its row would describe changes nobody reports.
        // Replacing the watcher is what covers it, and it costs a new watch rather than a second
        // watcher per repository.
        if watchers.get(&repo_id).is_some_and(|watched| {
            watched.plan == plan
                && watched.registration_id == registration_id
                && !watched.failed.load(Ordering::Acquire)
        }) {
            return Ok(());
        }

        let (sender, receiver) = mpsc::channel();
        let event_sender = sender.clone();
        let callback_plan = plan.clone();
        let failed = Arc::new(AtomicBool::new(false));
        let callback_failed = failed.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                forward_watch_result(result, &callback_plan, &event_sender, &callback_failed);
            })
            .map_err(|error| {
                IpcError::new(
                    IpcErrorCode::OperationFailed,
                    format!("could not watch repository: {error}"),
                )
            })?;
        for path in plan
            .roots
            .keys()
            .chain(plan.git_dirs.keys())
            .chain(plan.common_dir.iter())
        {
            watcher
                .watch(path, RecursiveMode::Recursive)
                .map_err(|error| {
                    IpcError::new(
                        IpcErrorCode::OperationFailed,
                        format!("could not watch repository: {error}"),
                    )
                })?;
        }

        let worker_snapshots = self.snapshots.clone();
        let worker_watchers = self.watchers.clone();
        let worker_token = Arc::new(());
        let token = worker_token.clone();
        let worker_failed = failed.clone();
        let worker_plan = plan.clone();
        let worker_registration_id = registration_id.clone();
        thread::Builder::new()
            .name("marvis-git-watch".into())
            .spawn(move || {
                loop {
                    match receive_debounced_change(&receiver, WATCH_DEBOUNCE, WATCH_MAX_BATCH) {
                        WatchWakeup::Changed(update) => {
                            // Before the event, so a refresh it triggers never reads what the same
                            // change just invalidated.
                            worker_snapshots.mark_stale(&update.status);
                            if !update.status.is_empty() {
                                let _ = app.emit(STATUS_CHANGED_EVENT, &update.status);
                            }
                            if !update.activity.is_empty() {
                                let _ = app.emit(FILE_ACTIVITY_EVENT, &update.activity);
                            }
                            if !update.worktrees.is_empty() {
                                // The rows themselves are not a status refresh, so the repository is named
                                // and its registration read again: a worktree created by an agent belongs
                                // in the sidebar the same way one created here does.
                                let _ = app.emit(WORKTREES_CHANGED_EVENT, &update.worktrees);
                            }
                        }
                        WatchWakeup::Failed(error) => {
                            let update = invalidate_failed_watch(&worker_snapshots, &worker_plan);
                            log::error!(
                                "Git watcher for repository {} failed: {error}",
                                worker_plan.repo_id
                            );
                            let _ = app.emit(STATUS_CHANGED_EVENT, &update.status);
                            let _ = app.emit(
                                WATCH_FAILED_EVENT,
                                watch_failure(&worker_plan, &worker_registration_id),
                            );
                            if let Ok(mut watchers) = worker_watchers.lock() {
                                if watchers
                                    .get(&worker_plan.repo_id)
                                    .is_some_and(|current| Arc::ptr_eq(&current.token, &token))
                                {
                                    watchers.remove(&worker_plan.repo_id);
                                }
                            }
                            break;
                        }
                        WatchWakeup::Stop => break,
                    }
                }
            })
            .map_err(|error| {
                IpcError::new(
                    IpcErrorCode::OperationFailed,
                    format!("could not start repository watcher: {error}"),
                )
            })?;

        // The replaced watcher is dropped with its sender, which is what ends the worker reading
        // the plan this one supersedes.
        if let Some(previous) = watchers.get(&repo_id) {
            invalidate_replaced_plan(&self.snapshots, &previous.plan, &plan);
        }
        watchers.insert(
            repo_id,
            RepoWatcher {
                _watcher: watcher,
                sender,
                failed: worker_failed,
                token: worker_token,
                registration_id,
                plan: plan.clone(),
            },
        );
        Ok(())
    }

    pub fn unwatch(&self, repo_id: &str, checkout_ids: &[String]) {
        let mut checkout_ids: BTreeSet<_> = checkout_ids.iter().cloned().collect();
        if let Ok(mut watchers) = self.watchers.lock() {
            if let Some(RepoWatcher { sender, plan, .. }) = watchers.remove(repo_id) {
                let _ = sender.send(WatchMessage::Stop);
                checkout_ids.extend(plan.requested);
            }
        }
        self.snapshots
            .forget_many(&checkout_ids.into_iter().collect::<Vec<_>>());
    }

    /// What is already held for a checkout, or `None` when it must be read again. Every live
    /// Git checkout is watched, so a held entry is invalidated by whatever changed it.
    fn cached_state(
        &self,
        checkout_id: &str,
        default_branch: Option<&str>,
    ) -> Option<CachedGitSnapshot> {
        self.snapshots.fresh(checkout_id, default_branch)
    }

    fn store_state(&self, checkout_id: String, state: CachedGitSnapshot, revision: Option<u64>) {
        self.snapshots.insert_at(checkout_id, state, revision);
    }
}

/// Which checkouts a filesystem change speaks for.
///
/// A path inside a worktree, or inside the Git directory that belongs to that one worktree, is
/// that worktree's business: a file written or a file staged there moves nothing for anyone
/// else. A path in the Git directory the worktrees share is the repository's, because a commit
/// there moves the merge base every sibling counts its lines against and their numbers are now
/// describing a change set that no longer exists.
///
/// Anything outside all of them belongs to none of them, and a change that touches no checkout
/// is not a change at all: Git writes an object for every commit, and re-reading the sidebar
/// because a blob landed is the cost this whole path exists to avoid.
///
/// The one exception is a change in the list of worktrees themselves, which is not a change any
/// checkout can speak for: it is answered by naming the repository, and the workspace is read
/// again to find the worktree that joined or the one that left.
fn affected_checkouts(plan: &RepoWatchPlan, event: &notify::Event) -> Option<WatchUpdate> {
    // Reading a path is not a change to it, and on Linux it is reported as an event of its own
    // for every open of every watched file. Git commands read .git/HEAD before acting, so treating
    // those reads as ref writes would refresh the whole repository on every command. Only Linux
    // reports these read events.
    if matches!(event.kind, notify::EventKind::Access(_)) {
        return None;
    }
    let mut status: BTreeSet<String> = BTreeSet::new();
    let mut activity: BTreeSet<String> = BTreeSet::new();
    let mut worktrees: BTreeSet<String> = BTreeSet::new();
    let mut repo_wide = false;
    for path in &event.paths {
        // Git writes a ref, the index and the configuration by creating `<name>.lock`, filling it
        // and renaming it over `<name>`. The lock is where the write begins, never what it leaves
        // behind: the rename that follows is the change, and a lock cleaned up afterwards
        // describes nothing. Skipping the path rather than the event is what keeps the rename,
        // which arrives as a second path in the same event or the next one, answering for the
        // change. Only Git's own metadata is filtered this way: a file in a checkout that happens
        // to be called `notes.lock` is a file the user edited.
        if is_git_lock_file(path) && is_git_metadata_path(plan, path) {
            continue;
        }
        // Check registration before a known worktree's Git directory: its `gitdir` file is
        // inside that directory, but removing it means the row's membership changed, not just
        // that checkout's status did.
        if let Some(common) = plan.common_dir.as_deref() {
            if path.starts_with(common) && is_worktree_registration_path(event, path, common) {
                worktrees.insert(plan.repo_id.clone());
                continue;
            }
        }
        if let Some(checkout_id) = longest_match(&plan.git_dirs, path) {
            status.insert(checkout_id.clone());
            continue;
        }
        if let Some(common) = plan.common_dir.as_deref() {
            if path.starts_with(common) {
                // Git checks the new worktree's branch out into `<common>/worktrees/<name>/`, and
                // the index it writes there is that checkout's own bookkeeping. The plan cannot
                // attribute it to anyone until the registration names it, and reading the whole
                // repository over it is the cost this signal must not pay.
                if is_pending_worktree_git_dir(path, common) {
                    continue;
                }
                repo_wide |= should_refresh_path(path);
                continue;
            }
        }
        if let Some((root, checkout_id)) = longest_path_match(&plan.roots, path) {
            // Marvis's default location is inside the repository root. Until the registration
            // event has brought a new worktree into the plan, its files would look like primary
            // checkout edits; ignore that reserved folder and let the shared Git metadata event
            // add its real root to the next watch plan.
            if is_pending_worktree_path(path, root) {
                continue;
            }
            status.insert(checkout_id.clone());
            if is_checkout_file_activity(path, root) {
                activity.insert(checkout_id.clone());
            }
        }
    }
    if repo_wide {
        status.extend(plan.all.iter().cloned());
    }
    let nothing_moved = status.is_empty() && activity.is_empty() && worktrees.is_empty();
    (!nothing_moved).then(|| WatchUpdate {
        status: status.into_iter().collect(),
        activity: activity.into_iter().collect(),
        worktrees: worktrees.into_iter().collect(),
    })
}

/// Whether a change in the shared Git directory is the list of worktrees moving.
///
/// Git registers a linked worktree as `<common>/worktrees/<name>`, and it does so whichever hand
/// added it: a worktree an agent created with `git worktree add` in a terminal is registered the
/// same way one Marvis created itself is, so the same write is what the sidebar has to notice.
///
/// The registration is also where a checkout does its own bookkeeping, and that is the part to
/// refuse. A `git add` creates `index.lock` and rewrites `index`, a commit moves `HEAD`, and a
/// checkout a user is typing in does it several times a minute: none of that adds or removes a
/// worktree, and answering it with a read of the whole repository is the cost this signal must not
/// pay. Registration directories only count as they appear or go; within them only Git's own
/// pointers to a checkout count, not the files a worktree uses.
fn is_worktree_registration_path(event: &notify::Event, path: &Path, common_dir: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(common_dir) else {
        return false;
    };
    let mut components = relative.components();
    if !matches!(components.next(), Some(Component::Normal(name)) if name.to_str() == Some("worktrees"))
    {
        return false;
    }
    match (components.next(), components.next()) {
        // The directory holding the registrations itself.
        (None, _) => is_worktree_lifecycle_event(event),
        // The worktree's own Git directory, appearing or going.
        (Some(Component::Normal(_)), None) => is_worktree_lifecycle_event(event),
        // A file directly in it, one Git writes to register a worktree rather than to use one.
        (Some(Component::Normal(_)), Some(Component::Normal(file)))
            if matches!(file.to_str(), Some("gitdir" | "commondir")) =>
        {
            matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Remove(_)
            )
        }
        _ => false,
    }
}

fn is_worktree_lifecycle_event(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Remove(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
}

/// The checkout owning a directory the path sits in, when it sits in one of them. The longest
/// match wins, so a worktree nested under another is its own and not its parent.
fn longest_match(dirs: &BTreeMap<PathBuf, String>, path: &Path) -> Option<String> {
    longest_path_match(dirs, path).map(|(_, checkout_id)| checkout_id.clone())
}

fn longest_path_match<'a>(
    dirs: &'a BTreeMap<PathBuf, String>,
    path: &Path,
) -> Option<(&'a PathBuf, &'a String)> {
    dirs.iter()
        .filter(|(dir, _)| path.starts_with(dir))
        .max_by_key(|(dir, _)| dir.components().count())
}

fn is_pending_worktree_path(path: &Path, checkout_root: &Path) -> bool {
    path.strip_prefix(checkout_root)
        .ok()
        .and_then(|relative| relative.components().next())
        .is_some_and(|component| {
            matches!(component, Component::Normal(name) if name.to_str() == Some(".worktrees"))
        })
}

/// Whether a change in the shared Git directory belongs to a worktree the plan has not learned
/// about yet. Git writes that worktree's `HEAD`, `index` and `ORIG_HEAD` into
/// `<common>/worktrees/<name>/` while it checks the branch out, and none of it moves anything for
/// anyone else. A worktree the plan already knows is matched before this is asked, so what is left
/// is the window between the worktree landing on disk and the membership signal that adds it.
fn is_pending_worktree_git_dir(path: &Path, common_dir: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(common_dir) else {
        return false;
    };
    let mut components = relative.components();
    matches!(
        components.next(),
        Some(Component::Normal(name)) if name.to_str() == Some("worktrees")
    ) && components.next().is_some()
}

/// Whether a path is one of Git's lock files: the scratch name it writes a ref, the index or the
/// configuration under before renaming it into place.
fn is_git_lock_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".lock"))
}

/// Whether a path belongs to Git's own bookkeeping rather than to a checkout's files: the Git
/// directory of one linked worktree, or the directory they all share. A lock in either is a write
/// in flight, and that includes a lock under a ref in the shared directory, which is why this is
/// asked separately from the lock itself.
fn is_git_metadata_path(plan: &RepoWatchPlan, path: &Path) -> bool {
    plan.git_dirs.keys().any(|dir| path.starts_with(dir))
        || plan
            .common_dir
            .as_deref()
            .is_some_and(|common| path.starts_with(common))
}

impl GitSnapshotCache {
    fn revision(&self, checkout_id: &str) -> Option<u64> {
        let revisions = self.revisions.lock().ok()?;
        Some(*revisions.get(checkout_id).unwrap_or(&0))
    }

    fn fresh(&self, checkout_id: &str, default_branch: Option<&str>) -> Option<CachedGitSnapshot> {
        let revisions = self.revisions.lock().ok()?;
        let revision = revisions.get(checkout_id).copied().unwrap_or(0);
        self.entries
            .lock()
            .ok()?
            .get(checkout_id)
            .filter(|entry| {
                entry.revision == revision
                    && !entry.stale
                    && entry.default_branch.as_deref() == default_branch
            })
            .cloned()
    }

    #[cfg(test)]
    fn insert(&self, checkout_id: String, state: CachedGitSnapshot) {
        let revision = self.revision(&checkout_id);
        self.insert_at(checkout_id, state, revision);
    }

    fn insert_at(&self, checkout_id: String, state: CachedGitSnapshot, revision: Option<u64>) {
        let Some(revision) = revision else {
            return;
        };
        let Ok(revisions) = self.revisions.lock() else {
            return;
        };
        if revisions.get(&checkout_id).copied().unwrap_or(0) != revision {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        entries.insert(
            checkout_id,
            CachedGitSnapshot {
                revision,
                stale: false,
                ..state
            },
        );
    }

    /// Marks every checkout a change speaks for, so the refresh it triggers reads the tree
    /// rather than what it looked like a moment ago.
    fn mark_stale(&self, checkout_ids: &[String]) {
        let Ok(mut revisions) = self.revisions.lock() else {
            return;
        };
        for checkout_id in checkout_ids {
            let revision = revisions.entry(checkout_id.clone()).or_default();
            *revision = revision.wrapping_add(1);
        }
        if let Ok(mut entries) = self.entries.lock() {
            for checkout_id in checkout_ids {
                if let Some(entry) = entries.get_mut(checkout_id) {
                    entry.stale = true;
                }
            }
        }
    }

    fn forget_many(&self, checkout_ids: &[String]) {
        let Ok(mut revisions) = self.revisions.lock() else {
            return;
        };
        for checkout_id in checkout_ids {
            let revision = revisions.entry(checkout_id.clone()).or_default();
            *revision = revision.wrapping_add(1);
        }
        if let Ok(mut entries) = self.entries.lock() {
            for checkout_id in checkout_ids {
                entries.remove(checkout_id);
            }
        }
    }
}

fn invalidate_replaced_plan(
    snapshots: &GitSnapshotCache,
    previous: &RepoWatchPlan,
    next: &RepoWatchPlan,
) {
    if previous == next {
        return;
    }
    let affected: BTreeSet<_> = previous
        .requested
        .iter()
        .chain(&next.requested)
        .cloned()
        .collect();
    snapshots.mark_stale(&affected.into_iter().collect::<Vec<_>>());
}

/// Waits for the directory to go quiet, then names every checkout the quiet period touched.
///
/// A burst is one answer, so the checkouts are merged as they arrive: a save that rewrites a
/// file and its lock, or a commit that moves a ref and an index, are two filesystem events
/// describing one change, and answering them apart would refresh the same rows twice.
fn receive_debounced_change(
    receiver: &mpsc::Receiver<WatchMessage>,
    debounce: Duration,
    max_batch: Duration,
) -> WatchWakeup {
    let merged = match receiver.recv() {
        Ok(WatchMessage::Changed(update)) => update,
        Ok(WatchMessage::Failed(error)) => return WatchWakeup::Failed(error),
        Ok(WatchMessage::Stop) | Err(_) => return WatchWakeup::Stop,
    };
    let started = Instant::now();
    receive_debounced_updates(
        merged,
        debounce,
        max_batch,
        || started.elapsed(),
        |timeout| receiver.recv_timeout(timeout),
    )
}

fn receive_debounced_updates(
    mut merged: WatchUpdate,
    debounce: Duration,
    max_batch: Duration,
    mut elapsed: impl FnMut() -> Duration,
    mut receive: impl FnMut(Duration) -> Result<WatchMessage, mpsc::RecvTimeoutError>,
) -> WatchWakeup {
    loop {
        let remaining = max_batch.saturating_sub(elapsed());
        if remaining.is_zero() {
            return WatchWakeup::Changed(merged);
        }
        match receive(debounce.min(remaining)) {
            Ok(WatchMessage::Changed(update)) => merge_update(&mut merged, update),
            Ok(WatchMessage::Failed(error)) => return WatchWakeup::Failed(error),
            Ok(WatchMessage::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return WatchWakeup::Stop;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => return WatchWakeup::Changed(merged),
        }
    }
}

fn merge_update(merged: &mut WatchUpdate, update: WatchUpdate) {
    for checkout_id in update.status {
        if !merged.status.contains(&checkout_id) {
            merged.status.push(checkout_id);
        }
    }
    for checkout_id in update.activity {
        if !merged.activity.contains(&checkout_id) {
            merged.activity.push(checkout_id);
        }
    }
    for repo_id in update.worktrees {
        if !merged.worktrees.contains(&repo_id) {
            merged.worktrees.push(repo_id);
        }
    }
}

fn failed_watch_update(plan: &RepoWatchPlan) -> WatchUpdate {
    WatchUpdate {
        status: plan.requested.clone(),
        ..WatchUpdate::default()
    }
}

fn requested_watch_ids(plan: &RepoWatchPlan) -> Vec<String> {
    let mut requested = plan.requested.clone();
    requested.sort();
    requested
}

fn watch_failure(plan: &RepoWatchPlan, registration_id: &str) -> WatchFailure {
    WatchFailure {
        repo_id: plan.repo_id.clone(),
        checkout_ids: requested_watch_ids(plan),
        registration_id: registration_id.to_owned(),
    }
}

fn invalidate_failed_watch(snapshots: &GitSnapshotCache, plan: &RepoWatchPlan) -> WatchUpdate {
    let update = failed_watch_update(plan);
    snapshots.mark_stale(&update.status);
    update
}

fn forward_watch_result(
    result: notify::Result<notify::Event>,
    plan: &RepoWatchPlan,
    sender: &mpsc::Sender<WatchMessage>,
    failed: &AtomicBool,
) {
    match result {
        Ok(event) if !failed.load(Ordering::Acquire) => {
            if let Some(update) = affected_checkouts(plan, &event) {
                // The channel only has to wake the worker; which checkouts moved rides along with
                // it, and the worker merges a burst into one answer.
                let _ = sender.send(WatchMessage::Changed(update));
            }
        }
        Err(error) if !failed.swap(true, Ordering::AcqRel) => {
            let message = error.to_string();
            if sender.send(WatchMessage::Failed(message.clone())).is_err() {
                log::error!(
                    "Git watcher for repository {} failed: {message}",
                    plan.repo_id
                );
            }
        }
        _ => {}
    }
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
    // A linked worktree's HEAD is private to that checkout, not a repo-wide ref. Known worktree
    // Git directories are matched before this helper; an unregistered one is not a reason to
    // refresh every existing checkout while the membership signal adds its row. The scratch name
    // a lock is written under is answered by the rename that follows it, which arrives separately
    // and does not match anything here: `HEAD.lock` is not a HEAD that moved.
    let head_changed = metadata.len() == 1 && ends_with("HEAD");
    let index_changed = ends_with("index");
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

pub fn status(
    database: &Database,
    watchers: &GitWatcherManager,
    checkout_id: &str,
) -> Result<GitStatus, IpcError> {
    let context = registered_git_context(database, checkout_id)?;
    Ok(snapshot_of(&context, watchers)?.status)
}

/// The counts of one checkout's changed files, from the same snapshot the file list is built
/// from, so the numbers on a row and the row's own base ref cannot disagree.
pub fn diff_stats(
    database: &Database,
    watchers: &GitWatcherManager,
    checkout_id: &str,
) -> Result<GitFileDiffStats, IpcError> {
    let context = registered_git_context(database, checkout_id)?;
    Ok(counts_of(&context, watchers)?.files)
}

/// The same counts folded into one number per checkout, for the rows that name every checkout
/// at once. A checkout Git cannot answer for is left out rather than reported as zero, so a
/// missing, plain or base-less checkout shows no counts instead of a false clean bill.
///
/// Every checkout the sidebar names is watched, so all but the ones a change just spoke for are
/// answered from what was already read. The rest are read side by side rather than one after
/// another: on a commit, a new worktree or the first paint, that is the whole sidebar at once,
/// and each reading is several `git` processes of its own.
pub fn checkout_diff_stats(
    database: &Database,
    watchers: &GitWatcherManager,
) -> Result<GitCheckoutDiffStats, IpcError> {
    let repos = workspace_repos(database)?;
    let mut totals = GitCheckoutDiffStats::new();
    let mut unread: Vec<GitContext> = Vec::new();
    for repo in &repos {
        if repo.kind != RepoKind::Git {
            continue;
        }
        for checkout in &repo.checkouts {
            if checkout.is_missing {
                continue;
            }
            let Ok(context) = git_context(&repos, &checkout.id) else {
                continue;
            };
            match held_state(&context, watchers).and_then(|held| held.counts) {
                Some(counts) => {
                    totals.insert(checkout.id.clone(), counts.totals);
                }
                None => unread.push(context),
            }
        }
    }
    for batch in unread.chunks(PARALLEL_READS) {
        for (checkout_id, counts) in thread::scope(|scope| {
            let reads: Vec<_> = batch
                .iter()
                .map(|context| {
                    let watchers = &*watchers;
                    scope.spawn(move || {
                        counts_of(context, watchers)
                            .ok()
                            .map(|counts| (context.checkout.id.clone(), counts.totals))
                    })
                })
                .collect();
            reads
                .into_iter()
                .map(|read| read.join().unwrap_or(None))
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        {
            totals.insert(checkout_id, counts);
        }
    }
    Ok(totals)
}

/// What the checkout was when it was last read, if a watcher has not said anything moved since.
///
/// The base ref is part of the entry rather than of the key alone, because choosing another
/// default branch moves it and no filesystem event describes that.
fn held_state(context: &GitContext, watchers: &GitWatcherManager) -> Option<CachedGitSnapshot> {
    watchers.cached_state(&context.checkout.id, context.repo.default_branch.as_deref())
}

/// Reads the checkout and remembers it, so the next question about it is a lookup rather than
/// another walk of the same working tree.
fn snapshot_of(context: &GitContext, watchers: &GitWatcherManager) -> Result<Snapshot, IpcError> {
    let revision = watchers.snapshots.revision(&context.checkout.id);
    let default_branch = context.repo.default_branch.clone();
    if let Some(held) = held_state(context, watchers) {
        return Ok(held.snapshot);
    }
    let read = snapshot(context)?;
    watchers.store_state(
        context.checkout.id.clone(),
        CachedGitSnapshot {
            default_branch,
            revision: 0,
            stale: false,
            snapshot: read.clone(),
            counts: None,
        },
        revision,
    );
    Ok(read)
}

/// The line counts, from the same snapshot the file list is served from, so a number and the
/// row it decorates can never describe two change sets.
fn counts_of(context: &GitContext, watchers: &GitWatcherManager) -> Result<GitCounts, IpcError> {
    let revision = watchers.snapshots.revision(&context.checkout.id);
    let default_branch = context.repo.default_branch.clone();
    let held = held_state(context, watchers);
    if let Some(counts) = held.as_ref().and_then(|held| held.counts.clone()) {
        return Ok(counts);
    }
    // The file list is the expensive part and it is already held; only the lines are missing.
    let read = match &held {
        Some(held) => held.snapshot.clone(),
        None => snapshot(context)?,
    };
    let counts = counted_files(context, &read)?;
    watchers.store_state(
        context.checkout.id.clone(),
        CachedGitSnapshot {
            default_branch,
            revision: 0,
            stale: false,
            snapshot: read,
            counts: Some(counts.clone()),
        },
        revision,
    );
    Ok(counts)
}

fn workspace_repos(database: &Database) -> Result<Vec<Repo>, IpcError> {
    database
        .load_workspace()
        .map(|workspace| workspace.repos)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))
}

/// The snapshot's own file list with the lines each file adds and removes attached, and their
/// sum. Both come from the same snapshot the file list is served from, so a number and the row
/// it decorates are always about the same base ref.
fn counted_files(context: &GitContext, snapshot: &Snapshot) -> Result<GitCounts, IpcError> {
    let counted = numstat_counts(context, snapshot)?;
    let mut totals = GitDiffStats::default();
    let files = snapshot
        .status
        .files
        .iter()
        .map(|file| {
            let counts = counted.get(&file.path).copied();
            if let Some(counts) = counts {
                totals.additions += counts.additions;
                totals.deletions += counts.deletions;
            }
            GitChangedFile {
                additions: counts.map(|counts| counts.additions),
                deletions: counts.map(|counts| counts.deletions),
                ..file.clone()
            }
        })
        .collect();
    Ok(GitCounts { files, totals })
}

/// Counts the lines every changed file adds and removes, against exactly the base the file
/// list uses: `snapshot.merge_base` for the tracked files, which is the ref `diff_args` diffs a
/// tracked file against, and `/dev/null` for the untracked ones, which is how it diffs those.
fn numstat_counts(
    context: &GitContext,
    snapshot: &Snapshot,
) -> Result<BTreeMap<String, GitDiffStats>, IpcError> {
    let tracked = checked_git(
        &context.root,
        [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--numstat",
            "-z",
            "--find-renames",
            &snapshot.merge_base,
        ],
        "could not count changed lines",
    )?;
    let mut counts = parse_numstat(&tracked.stdout)?;
    for file in snapshot
        .status
        .files
        .iter()
        .filter(|file| file.status == "??")
    {
        // A path that leaves the checkout is not one of its files, whatever is at the other
        // end of the symlink, so it is left out of the counts the same way it is left out of
        // the diff.
        let Ok(path) = contained_untracked_path(context, &file.path) else {
            continue;
        };
        if let Some(untracked) = untracked_line_count(&path) {
            counts.insert(file.path.clone(), untracked);
        }
    }
    Ok(counts)
}

/// The lines an untracked file adds, counted the way `git diff --numstat` would count them.
///
/// An untracked file is a whole file Git has never seen, so every one of its lines is an
/// addition and there is nothing to remove. That makes the count the number of lines in the
/// file, which is what Git reports, and it is why this reads the file rather than asking Git to
/// diff it against nothing: the answer needs one read, and `git diff --no-index` needs one
/// process per untracked file. A worktree an agent has been working in holds a handful of
/// them, and every sidebar refresh pays for each.
///
/// A file Git declines to count has no number at all rather than a zero: a binary file is one
/// of those, and a row showing `+0` for it would be a claim about a file Git never measured.
/// The test below is the parity that keeps this honest.
fn untracked_line_count(path: &Path) -> Option<GitDiffStats> {
    let bytes = fs::read(path).ok()?;
    if is_binary(&bytes) {
        return None;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    // A file whose last line has no newline of its own is still a line, which is why Git
    // reports one more for `a` and one for `a\nb` alike.
    let trailing = !bytes.is_empty() && !bytes.ends_with(b"\n");
    Some(GitDiffStats {
        additions: newlines + u64::from(trailing),
        deletions: 0,
    })
}

/// Whether Git would call this file binary, by the same rule it uses: a NUL byte anywhere in
/// the first block it looks at. A file that reads as binary after that block is counted as
/// text, which is the same answer Git gives.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0)
}

/// Every `--numstat` record is one NUL-terminated field, so a path that holds a newline, a
/// tab or a NUL of its own cannot be split apart: a plain change is `adds<tab>deletes<tab>path`
/// and a rename or copy is `adds<tab>deletes<tab>` followed by the old and the new path as two
/// more fields. The counts are keyed by the new path, the one the file list shows, and a
/// record Git declined to count is left out instead of being reported as zero lines.
fn parse_numstat(output: &[u8]) -> Result<BTreeMap<String, GitDiffStats>, IpcError> {
    let mut fields = output.split(|byte| *byte == 0);
    let mut stats = BTreeMap::new();
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let (additions, deletions, path) = numstat_columns(record).ok_or_else(malformed_numstat)?;
        let path = if path.is_empty() {
            // A rename or copy: the counts came first, then the two paths.
            fields.next().ok_or_else(malformed_numstat)?;
            fields.next().ok_or_else(malformed_numstat)?
        } else {
            path
        };
        if let (Some(additions), Some(deletions)) = (additions, deletions) {
            stats.insert(
                path_text(path)?,
                GitDiffStats {
                    additions,
                    deletions,
                },
            );
        }
    }
    Ok(stats)
}

/// Splits one `--numstat` record into its two counts and its path, or `None` when the bytes
/// are not a record at all. A `-` column is Git declining to count the file, which is not a
/// zero, and is carried out as a `None` of its own.
fn numstat_columns(record: &[u8]) -> Option<(Option<u64>, Option<u64>, &[u8])> {
    let mut columns = record.splitn(3, |byte| *byte == b'\t');
    let additions = numstat_count(columns.next()?)?;
    let deletions = numstat_count(columns.next()?)?;
    Some((additions, deletions, columns.next()?))
}

fn numstat_count(column: &[u8]) -> Option<Option<u64>> {
    let text = std::str::from_utf8(column).ok()?;
    if text == "-" {
        return Some(None);
    }
    Some(Some(text.parse().ok()?))
}

fn malformed_numstat() -> IpcError {
    IpcError::new(
        IpcErrorCode::GitFailed,
        "Git returned an invalid line count record",
    )
}

/// An untracked entry is the one path the counts read straight from the working tree, so it
/// goes through the same containment as every other path a command touches: relative to the
/// checkout, resolving to a file inside it. A link that leaves the checkout, or that points at
/// a folder, is refused rather than followed.
fn contained_untracked_path(context: &GitContext, path: &str) -> Result<PathBuf, IpcError> {
    let target = context.root.join(validate_relative_path(path)?);
    let resolved = fs::canonicalize(&target).map_err(|error| {
        IpcError::new(
            IpcErrorCode::FolderMissing,
            format!("untracked file is no longer available: {error}"),
        )
    })?;
    if !resolved.starts_with(&context.root) || !resolved.is_file() {
        return Err(IpcError::new(
            IpcErrorCode::PathOutsideCheckout,
            "untracked path does not resolve to a file inside the checkout",
        ));
    }
    Ok(target)
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
            old_content: None,
            new_content: None,
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
    let patch = if scan.large || scan.too_large || scan.is_binary {
        String::new()
    } else {
        String::from_utf8_lossy(&scan.patch).into_owned()
    };
    // Only for a diff that is drawn: a patch nothing shows is not worth two more reads of the file.
    // Each side is the text `diff_args` diffed, read the way it was diffed (the merge base's blob
    // and the file as it is on disk) rather than the head's or the index's, which the patch of this
    // diff never saw.
    let (old_content, new_content) = if patch.is_empty() {
        (None, None)
    } else {
        (
            merge_base_content(
                &context.root,
                &snapshot.merge_base,
                changed_file.old_path.as_deref().unwrap_or(path),
            ),
            worktree_content(&working_path),
        )
    };
    Ok(GitFileDiff {
        path: path.to_owned(),
        patch,
        old_content,
        new_content,
        is_binary: scan.is_binary,
        large: scan.large,
        too_large: scan.too_large,
        total_lines: scan.total_lines,
        hunks: scan.hunks,
        symlink_target: None,
    })
}

/// The text of the file as it is on disk, or nothing when there is none to read within the cap.
///
/// The path is one `validate_diff_target` has resolved inside the checkout and that has already been
/// turned away if it is a symlink, so this reads a file the checkout owns. `O_NOFOLLOW` is what keeps
/// that true by the time the file is actually opened: a checkout is writable by whatever is editing
/// it, and between the check above and the open below the path could have become a link out of the
/// checkout, which is the one thing `validate_diff_target` exists to prevent. `is_file` on the open
/// handle is then a statement about the file that was read rather than about the path it was reached
/// by, which is what keeps this from being a read of a directory, or of a pipe that would never end.
/// Bytes that are not text are not carried at all, where the patch carries them lossy: text the two
/// cannot be compared line for line is text no grammar could be handed.
fn worktree_content(path: &Path) -> Option<String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .ok()?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_SYNTAX_CONTEXT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_SYNTAX_CONTEXT_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The text of the blob `spec` names, or nothing when there is none to read within the cap.
///
/// Read through a pipe under a cap rather than through `run_git`, which buffers all of whatever Git
/// prints: a blob is the size of a file, and this one is read to color a diff rather than to show
/// it. A merge base with no such path (an added file, an untracked one) fails here, which is the
/// same answer as an old side with no lines to draw; the exit status is what says so, because a
/// failure prints nothing on the pipe it would have been read from.
fn merge_base_content(root: &Path, merge_base: &str, path: &str) -> Option<String> {
    let mut child = Command::new("git")
        .args(["cat-file", "blob", &format!("{merge_base}:{path}")])
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Nothing is read of Git's own account of a failure, so its pipe is never the reason a read
        // stops: a blob past the cap ends the read and Git's complaint has nowhere to go.
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut bytes = Vec::new();
    BufReader::new(child.stdout.take()?)
        .take(MAX_SYNTAX_CONTEXT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    // A blob past the cap ends the read early and Git never finishes writing, which is one of the
    // two ways this is nothing rather than the text of a file.
    if !child.wait().ok()?.success() || bytes.len() > MAX_SYNTAX_CONTEXT_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
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
        .env("GIT_OPTIONAL_LOCKS", "0")
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

/// What the repository watcher needs to know: every worktree still on disk, and the Git
/// directory they share.
///
/// A missing checkout is left out because there is nothing to watch for it, and nothing to read
/// either, so its row shows no counts rather than a false clean bill. A checkout whose
/// directory cannot be resolved is left out for the same reason rather than failing the whole
/// repository, which would leave every other worktree of it unwatched.
pub fn watch_plan(database: &Database, repo_id: &str) -> Result<RepoWatchPlan, IpcError> {
    let repos = workspace_repos(database)?;
    let repo = repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "repository is not registered",
            )
        })?;
    if repo.kind != RepoKind::Git {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "only a Git repository has changes to watch",
        ));
    }
    let mut plan = RepoWatchPlan {
        repo_id: repo_id.to_owned(),
        ..RepoWatchPlan::default()
    };
    for checkout in &repo.checkouts {
        if checkout.is_missing {
            continue;
        }
        plan.requested.push(checkout.id.clone());
        let Ok(context) = git_context(&repos, &checkout.id) else {
            continue;
        };
        if plan.common_dir.is_none() {
            plan.common_dir = common_git_dir(&context);
        }
        plan.all.push(checkout.id.clone());
        plan.roots.insert(context.root.clone(), checkout.id.clone());
        // The primary repository's own Git directory *is* the shared one, so it is left out:
        // a ref written under it moves every sibling's merge base, and attributing it to the
        // checkout that happens to live there would leave the other rows describing a change
        // set that no longer exists.
        if let Some(git_dir) =
            own_git_dir(&context).filter(|dir| Some(dir) != plan.common_dir.as_ref())
        {
            plan.git_dirs.insert(git_dir, checkout.id.clone());
        }
    }
    if plan.roots.is_empty() {
        return Err(IpcError::new(
            IpcErrorCode::FolderMissing,
            "no worktree of this repository is on disk",
        ));
    }
    Ok(plan)
}

pub fn watch_plan_matches_request(plan: &RepoWatchPlan, expected_checkout_ids: &[String]) -> bool {
    let mut expected = expected_checkout_ids.to_vec();
    expected.sort();
    requested_watch_ids(plan) == expected
}

/// The checkout ids a repository is registered with, so unwatching can drop exactly the
/// snapshots that repository held.
pub fn repo_checkout_ids(database: &Database, repo_id: &str) -> Vec<String> {
    workspace_repos(database)
        .map(|repos| {
            repos
                .iter()
                .filter(|repo| repo.id == repo_id)
                .flat_map(|repo| repo.checkouts.iter())
                .map(|checkout| checkout.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The Git directory that belongs to this checkout alone, as an absolute path: the primary
/// repository's own `.git`, or the `worktrees/<name>` directory a linked worktree gets. It is
/// where that worktree's index and HEAD live, and staging there moves nothing for its siblings.
fn own_git_dir(context: &GitContext) -> Option<PathBuf> {
    let text = optional_git_text(&context.root, ["rev-parse", "--absolute-git-dir"])?;
    absolute_git_path(&context.root, &text)
}

/// The Git directory the checkout shares with its siblings, as an absolute path.
///
/// A linked worktree resolves this to the primary repository's `.git`, which is what makes it
/// worth watching: it is where a commit in one worktree is published to the others.
fn common_git_dir(context: &GitContext) -> Option<PathBuf> {
    let text = optional_git_text(&context.root, ["rev-parse", "--git-common-dir"])?;
    absolute_git_path(&context.root, &text)
}

fn absolute_git_path(root: &Path, text: &str) -> Option<PathBuf> {
    let path = Path::new(text);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    absolute.canonicalize().ok()
}

fn registered_git_context(database: &Database, checkout_id: &str) -> Result<GitContext, IpcError> {
    let (repo, checkout) = database
        .load_registered_checkout(checkout_id)
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidCheckout,
                "checkout ID is not registered",
            )
        })?;
    git_context_for_checkout(&repo, &checkout)
}

fn git_context(repos: &[Repo], checkout_id: &str) -> Result<GitContext, IpcError> {
    let (repo, checkout) = crate::services::checkout::registered_checkout(
        repos,
        checkout_id,
        "checkout ID is not registered",
    )?;
    git_context_for_checkout(repo, checkout)
}

fn git_context_for_checkout(repo: &Repo, checkout: &Checkout) -> Result<GitContext, IpcError> {
    if repo.kind != RepoKind::Git {
        return Err(IpcError::new(
            IpcErrorCode::InvalidCheckout,
            "Git changes are only available for Git checkouts",
        ));
    }
    if checkout.is_missing {
        return Err(IpcError::new(
            IpcErrorCode::FolderMissing,
            "checkout is no longer available",
        ));
    }
    let root = resolve_checkout_path(repo, &checkout.id, Path::new("."))?;
    Ok(GitContext {
        repo: repo.clone(),
        checkout: checkout.clone(),
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
                additions: None,
                deletions: None,
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
                additions: None,
                deletions: None,
            });
        } else {
            let path = records.get(index).ok_or_else(malformed_status)?;
            index += 1;
            let path = path_text(path)?;
            files.push(GitChangedFile {
                path,
                old_path: None,
                status: String::from_utf8_lossy(&status_record[..1]).into_owned(),
                additions: None,
                deletions: None,
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

/// Runs Git to read, never to write, and with the optional locks off so the read leaves nothing
/// behind.
///
/// `git status` takes the index lock to refresh stat data, and the index of a linked worktree
/// lives inside the very directory the watcher is watching. A refresh would then announce itself,
/// the announcement would ask for the refresh that wrote it, and the repository would spend the
/// rest of its life re-reading itself. Git answers the same question without the lock, and the
/// index stays exactly where the last command that meant to change it left it.
fn run_git(root: &Path, args: Vec<OsString>) -> Result<Output, IpcError> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
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
    use std::{
        cell::Cell,
        collections::{BTreeMap, VecDeque},
        fs,
        path::Path,
        process::Command,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use notify::Watcher;
    use tempfile::tempdir;

    use crate::{
        domain::{ipc::IpcErrorCode, workspace::RepoKind},
        persistence::Database,
        services::workspace,
    };

    use super::{
        affected_checkouts, checked_git, diff, diff_page, failed_watch_update,
        forward_watch_result, invalidate_failed_watch, parse_diff_display_line, parse_name_status,
        parse_numstat, parse_porcelain_v2, receive_debounced_change, receive_debounced_updates,
        requested_watch_ids, resolve_default_ref, should_refresh_path, status,
        untracked_line_count, watch_failure, watch_plan_matches_request, CachedGitSnapshot,
        GitCounts, GitDiffStats, GitSnapshotCache, GitStatus, GitWatcherManager, PathBuf,
        RepoWatchPlan, WatchMessage, WatchUpdate, WatchWakeup, MAX_SYNTAX_CONTEXT_BYTES,
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
        let watchers = GitWatcherManager::default();

        let snapshot = status(&database, &watchers, &checkout_id).unwrap();

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
        let watchers = GitWatcherManager::default();

        let snapshot = status(&database, &watchers, &checkout_id).unwrap();
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
        let watchers = GitWatcherManager::default();

        let status = status(&database, &watchers, &checkout_id).unwrap();
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
    fn a_diff_carries_the_whole_text_of_each_of_its_sides() {
        // A grammar reads a file, not the hunks of a diff: the lines inside the `<script>` of a Vue
        // file are markup to a grammar that was never shown the tag that opened them. So each side
        // travels whole, and each is read the way `diff_args` diffed it — the merge base's blob and
        // the file as it is on disk.
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let before = "<template>\n  <p>one</p>\n</template>\n\n<script setup lang=\"ts\">\nconst n = 1;\n</script>\n";
        let after = "<template>\n  <p>two</p>\n</template>\n\n<script setup lang=\"ts\">\nconst n = 2;\n</script>\n";
        fs::write(root.join("Widget.vue"), before).unwrap();
        git(&root, &["add", "Widget.vue"]);
        git(&root, &["commit", "-m", "widget"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::write(root.join("Widget.vue"), after).unwrap();

        let result = diff(&database, &checkout_id, "Widget.vue").unwrap();

        assert_eq!(result.old_content.as_deref(), Some(before));
        assert_eq!(result.new_content.as_deref(), Some(after));
    }

    #[test]
    fn an_added_file_carries_no_old_text_and_a_removed_one_no_new() {
        // Each side the diff does not have is absent rather than empty: an added file has no old side
        // and a removed one no new side, and text that is not there is not something to read.
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let gone = "const removed = 1;\n";
        fs::write(root.join("gone.ts"), gone).unwrap();
        git(&root, &["add", "gone.ts"]);
        git(&root, &["commit", "-m", "gone"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::write(root.join("added.ts"), "const added = 1;\n").unwrap();
        fs::remove_file(root.join("gone.ts")).unwrap();

        let added = diff(&database, &checkout_id, "added.ts").unwrap();
        let removed = diff(&database, &checkout_id, "gone.ts").unwrap();

        assert_eq!(added.old_content, None);
        assert_eq!(added.new_content.as_deref(), Some("const added = 1;\n"));
        assert_eq!(removed.old_content.as_deref(), Some(gone));
        assert_eq!(removed.new_content, None);
    }

    #[test]
    fn a_renamed_files_old_text_is_the_path_the_rename_came_from() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        fs::create_dir_all(root.join("before")).unwrap();
        let original = (0..40)
            .map(|line| format!("const value{line} = {line};\n"))
            .collect::<String>();
        fs::write(root.join("before/View.ts"), &original).unwrap();
        git(&root, &["add", "before/View.ts"]);
        git(&root, &["commit", "-m", "view"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::create_dir_all(root.join("after")).unwrap();
        git(&root, &["mv", "before/View.ts", "after/View.ts"]);
        let renamed = original.replace("const value20 = 20;\n", "const value20 = 21;\n");
        fs::write(root.join("after/View.ts"), &renamed).unwrap();

        let result = diff(&database, &checkout_id, "after/View.ts").unwrap();

        assert_eq!(result.old_content.as_deref(), Some(original.as_str()));
        assert_eq!(result.new_content.as_deref(), Some(renamed.as_str()));
    }

    #[test]
    fn no_text_travels_with_a_binary_diff_or_with_a_file_past_the_cap() {
        // There is nothing to read in the first case, and in the second there is more of it than a
        // diff is worth carrying: both are absent, which is what leaves the diff to draw itself.
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        fs::write(root.join("bytes.dat"), [0, 1, 2]).unwrap();
        git(&root, &["add", "bytes.dat"]);
        git(&root, &["commit", "-m", "bytes"]);
        let big = "line\n".repeat(MAX_SYNTAX_CONTEXT_BYTES / 5 + 1);
        fs::write(root.join("big.txt"), &big).unwrap();
        git(&root, &["add", "big.txt"]);
        git(&root, &["commit", "-m", "big"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        fs::write(root.join("bytes.dat"), [0, 2, 3]).unwrap();
        fs::write(root.join("big.txt"), big.replacen("line\n", "LINE\n", 1)).unwrap();

        let binary = diff(&database, &checkout_id, "bytes.dat").unwrap();
        let large = diff(&database, &checkout_id, "big.txt").unwrap();

        assert_eq!((binary.old_content, binary.new_content), (None, None));
        assert!(!binary.is_binary || binary.patch.is_empty());
        // The change is small, so the patch is still drawn: it is the file's text that does not fit.
        assert!(!large.patch.is_empty());
        assert_eq!((large.old_content, large.new_content), (None, None));
    }

    #[test]
    fn plain_folders_do_not_run_git_and_changes_are_rejected() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("plain");
        fs::create_dir_all(&root).unwrap();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let state = workspace::register_folder(&database, &root).unwrap();
        assert_eq!(state.repos[0].kind, RepoKind::Plain);

        let watchers = GitWatcherManager::default();
        let error = status(&database, &watchers, &state.repos[0].checkouts[0].id).unwrap_err();

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

    fn touched(ids: &[&str]) -> WatchUpdate {
        watch_update(ids, &[], &[])
    }

    fn watch_update(status: &[&str], activity: &[&str], worktrees: &[&str]) -> WatchUpdate {
        WatchUpdate {
            status: status.iter().map(|id| (*id).to_owned()).collect(),
            activity: activity.iter().map(|id| (*id).to_owned()).collect(),
            worktrees: worktrees.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    fn watcher_debounces_a_burst_into_one_signal_naming_every_checkout_it_touched() {
        let (sender, receiver) = mpsc::channel();
        sender.send(WatchMessage::Changed(touched(&["a"]))).unwrap();
        sender.send(WatchMessage::Changed(touched(&["b"]))).unwrap();
        sender.send(WatchMessage::Changed(touched(&["a"]))).unwrap();

        // One save rewrites a file and its lock, and a commit moves a ref and an index. Those
        // are separate filesystem events describing one change, and answering them apart would
        // refresh the same rows twice. A checkout named twice is still one row.
        assert_eq!(
            receive_debounced_change(&receiver, Duration::from_millis(1), Duration::from_secs(1),),
            WatchWakeup::Changed(touched(&["a", "b"]))
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn watcher_caps_a_continuous_batch_and_preserves_changes_for_the_next_batch() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut pending = VecDeque::from([
            (
                Duration::from_millis(50),
                WatchMessage::Changed(watch_update(
                    &["early"],
                    &["activity-early"],
                    &["repo-early"],
                )),
            ),
            (
                Duration::from_millis(120),
                WatchMessage::Changed(watch_update(
                    &["middle"],
                    &["activity-middle"],
                    &["repo-middle"],
                )),
            ),
            (
                Duration::from_millis(190),
                WatchMessage::Changed(watch_update(
                    &["initial"],
                    &["activity-early"],
                    &["repo-early"],
                )),
            ),
            (
                Duration::from_millis(240),
                WatchMessage::Changed(watch_update(
                    &["near-limit"],
                    &["activity-near-limit"],
                    &["repo-near-limit"],
                )),
            ),
            (
                Duration::from_millis(260),
                WatchMessage::Changed(touched(&["next-batch"])),
            ),
        ]);
        let mut waits = Vec::new();
        let mut receives = 0;

        let wakeup = receive_debounced_updates(
            watch_update(&["initial"], &["activity-initial"], &["repo-initial"]),
            Duration::from_millis(100),
            Duration::from_millis(250),
            || elapsed.get(),
            |timeout| {
                receives += 1;
                waits.push(timeout);
                let deadline = elapsed.get() + timeout;
                if pending.front().is_some_and(|(at, _)| *at < deadline) {
                    let (at, message) = pending.pop_front().unwrap();
                    elapsed.set(at);
                    Ok(message)
                } else {
                    elapsed.set(deadline);
                    Err(mpsc::RecvTimeoutError::Timeout)
                }
            },
        );

        assert_eq!(
            wakeup,
            WatchWakeup::Changed(watch_update(
                &["initial", "early", "middle", "near-limit"],
                &[
                    "activity-initial",
                    "activity-early",
                    "activity-middle",
                    "activity-near-limit",
                ],
                &[
                    "repo-initial",
                    "repo-early",
                    "repo-middle",
                    "repo-near-limit"
                ],
            ))
        );
        assert_eq!(
            waits,
            [
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_millis(60),
                Duration::from_millis(10),
            ]
        );
        assert_eq!(elapsed.get(), Duration::from_millis(250));
        assert_eq!(receives, 5);
        assert!(
            matches!(pending.front(), Some((at, WatchMessage::Changed(_))) if *at == Duration::from_millis(260))
        );
    }

    #[test]
    fn watcher_stops_if_the_channel_disconnects_during_a_batch() {
        let (sender, receiver) = mpsc::channel();
        sender.send(WatchMessage::Changed(touched(&["a"]))).unwrap();
        drop(sender);

        assert_eq!(
            receive_debounced_change(&receiver, Duration::from_millis(1), Duration::from_secs(1),),
            WatchWakeup::Stop
        );
    }

    #[test]
    fn repository_watcher_observes_file_edits_and_says_which_worktree_they_landed_in() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join("main")).unwrap();
        fs::create_dir_all(temp.path().join("task")).unwrap();
        // Resolved, because the filesystem reports resolved paths: a checkout reached
        // through a symlink would otherwise match no event and quietly go stale forever.
        let root = temp.path().join("main").canonicalize().unwrap();
        let worktree = temp.path().join("task").canonicalize().unwrap();
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [
                (root.clone(), "main".to_owned()),
                (worktree.clone(), "task".to_owned()),
            ]
            .into_iter()
            .collect(),
            git_dirs: BTreeMap::new(),
            common_dir: None,
            all: vec!["main".to_owned(), "task".to_owned()],
            requested: vec!["main".to_owned(), "task".to_owned()],
        };
        let (sender, receiver) = mpsc::channel();
        let event_sender = sender.clone();
        let event_plan = plan.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if let Some(update) = result
                    .ok()
                    .and_then(|event| affected_checkouts(&event_plan, &event))
                {
                    let _ = event_sender.send(WatchMessage::Changed(update));
                }
            })
            .unwrap();
        for path in [root, worktree.clone()] {
            watcher
                .watch(&path, notify::RecursiveMode::Recursive)
                .unwrap();
        }

        fs::write(worktree.join("terminal-edit.txt"), "changed\n").unwrap();

        // Only the worktree that was written to is named. The row beside it is untouched, and
        // saying otherwise would make a save in one worktree cost a re-read of every other.
        //
        // FSEvents, the backend `recommended_watcher` uses on macOS, can name a watched directory
        // once when its stream starts, and on a loaded runner that event arrives beside the write
        // instead of before it. Reading only the first message would then assert on the startup
        // event. So this waits for the update that names the written worktree, the same way
        // `assert_membership_signal` waits for the worktree it asked for: the claim under test
        // is still that one update, and it still has to name that worktree and nothing else.
        let deadline = Instant::now() + Duration::from_secs(5);
        let update = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(remaining) {
                Ok(WatchMessage::Changed(update)) if update.status.contains(&"task".to_owned()) => {
                    break update;
                }
                Ok(WatchMessage::Changed(_)) => continue,
                Ok(WatchMessage::Failed(error)) => panic!("watch failed: {error}"),
                Ok(WatchMessage::Stop) => panic!("watcher stopped"),
                Err(_) => panic!("the write into the worktree was never reported"),
            }
        };
        assert_eq!(update.status, vec!["task".to_owned()]);
        assert_eq!(update.activity, vec!["task".to_owned()]);
    }

    #[test]
    fn a_ref_write_speaks_for_every_worktree_because_a_commit_moves_the_merge_base() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [
                (PathBuf::from("/repo"), "main".to_owned()),
                (PathBuf::from("/repo-task"), "task".to_owned()),
            ]
            .into_iter()
            .collect(),
            // The primary's own Git directory is the shared one, so it is not here: a ref
            // written under it is the repository's change, not the primary checkout's.
            git_dirs: BTreeMap::from([(
                PathBuf::from("/repo/.git/worktrees/task"),
                "task".to_owned(),
            )]),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned(), "task".to_owned()],
            requested: vec!["main".to_owned(), "task".to_owned()],
        };
        let write = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/repo/.git/refs/heads/main")],
            attrs: Default::default(),
        };

        // The sibling on another branch counted its lines against a merge base that just moved,
        // so its row is now describing a change set that does not exist.
        assert_eq!(
            affected_checkouts(&plan, &write).map(|update| update.status),
            Some(vec!["main".to_owned(), "task".to_owned()])
        );
    }

    #[test]
    fn staging_in_one_worktree_does_not_re_read_the_siblings() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [
                (PathBuf::from("/repo"), "main".to_owned()),
                (PathBuf::from("/repo-task"), "task".to_owned()),
            ]
            .into_iter()
            .collect(),
            // The primary's own Git directory is the shared one, so it is not here: a ref
            // written under it is the repository's change, not the primary checkout's.
            git_dirs: BTreeMap::from([(
                PathBuf::from("/repo/.git/worktrees/task"),
                "task".to_owned(),
            )]),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned(), "task".to_owned()],
            requested: vec!["main".to_owned(), "task".to_owned()],
        };
        let staged = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/repo/.git/worktrees/task/index")],
            attrs: Default::default(),
        };

        // The sibling is on another branch with another index: nothing it counts moved. An
        // agent turn stages in the worktree it is working in, several times, and each of those
        // is a save in every other row if the whole repository is re-read.
        assert_eq!(
            affected_checkouts(&plan, &staged).map(|update| update.status),
            Some(vec!["task".to_owned()])
        );
    }

    #[test]
    fn a_worktree_joining_the_repository_asks_for_its_own_list_to_be_read_again() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            git_dirs: BTreeMap::new(),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned()],
            requested: vec!["main".to_owned()],
        };
        // Git registers the worktree, writes its metadata and only then the checkout, wherever
        // the new directory landed: the registration is the one write every way of adding a
        // worktree goes through, and an agent running `git worktree add` is the same event.
        for (path, kind) in [
            (
                "/repo/.git/worktrees/task",
                notify::EventKind::Create(notify::event::CreateKind::Folder),
            ),
            (
                "/repo/.git/worktrees/task/gitdir",
                notify::EventKind::Create(notify::event::CreateKind::File),
            ),
        ] {
            let event = notify::Event {
                kind,
                paths: vec![PathBuf::from(path)],
                attrs: Default::default(),
            };
            // No row of the repository is stale — there is no row for the new worktree yet — so
            // the answer is the repository itself, and nothing is refreshed for the worktree it
            // just joined.
            assert_eq!(
                affected_checkouts(&plan, &event),
                Some(WatchUpdate {
                    status: Vec::new(),
                    activity: Vec::new(),
                    worktrees: vec!["repo".to_owned()],
                }),
                "{path}"
            );
        }
    }

    #[test]
    fn a_real_git_worktree_add_and_remove_emit_membership_changes() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        fs::create_dir_all(root.join(".worktrees")).unwrap();
        git(&root, &["branch", "external"]);
        let (database, _) = git_database(temp.path(), &root);
        let repo_id = database.load_workspace().unwrap().repos[0].id.clone();
        let plan = super::watch_plan(&database, &repo_id).unwrap();
        let common_dir = plan.common_dir.clone().unwrap();
        let (sender, receiver) = mpsc::channel();
        let event_sender = sender.clone();
        let event_plan = plan.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if let Some(update) = result
                    .ok()
                    .and_then(|event| affected_checkouts(&event_plan, &event))
                {
                    let _ = event_sender.send(update);
                }
            })
            .unwrap();
        watcher
            .watch(&common_dir, notify::RecursiveMode::Recursive)
            .unwrap();
        for root in plan.roots.keys() {
            watcher
                .watch(root, notify::RecursiveMode::Recursive)
                .unwrap();
        }

        let added = root.join(".worktrees/external");
        git(
            &root,
            &["worktree", "add", added.to_str().unwrap(), "external"],
        );
        assert_membership_signal(&receiver, &repo_id);

        git(&root, &["worktree", "remove", added.to_str().unwrap()]);
        assert_membership_signal(&receiver, &repo_id);
    }

    fn assert_membership_signal(receiver: &mpsc::Receiver<WatchUpdate>, repo_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let update = receiver
                .recv_timeout(remaining)
                .expect("Git worktree add/remove should notify the shared Git directory");
            if update.worktrees.iter().any(|id| id == repo_id) {
                return;
            }
            assert!(
                update.status.is_empty() && update.activity.is_empty(),
                "the new worktree's files must not be reported as primary-checkout changes"
            );
        }
    }

    #[test]
    fn removing_a_known_worktree_registration_is_not_mistaken_for_status() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            git_dirs: BTreeMap::from([(
                PathBuf::from("/repo/.git/worktrees/task"),
                "task".to_owned(),
            )]),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned(), "task".to_owned()],
            requested: vec!["main".to_owned(), "task".to_owned()],
        };
        let removed = notify::Event {
            kind: notify::EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![PathBuf::from("/repo/.git/worktrees/task/gitdir")],
            attrs: Default::default(),
        };

        let update = affected_checkouts(&plan, &removed).unwrap();

        assert!(update.status.is_empty());
        assert_eq!(update.worktrees, ["repo"]);
    }

    #[test]
    fn opening_primary_files_while_creating_a_worktree_is_not_a_checkout_change() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned()],
            ..RepoWatchPlan::default()
        };
        let path = PathBuf::from("/repo/base.txt");

        // Linux inotify surfaces opens and read-only closes as Access events.
        for kind in [
            notify::EventKind::Access(notify::event::AccessKind::Open(
                notify::event::AccessMode::Any,
            )),
            notify::EventKind::Access(notify::event::AccessKind::Close(
                notify::event::AccessMode::Read,
            )),
        ] {
            let event = notify::Event {
                kind,
                paths: vec![path.clone()],
                attrs: Default::default(),
            };
            assert_eq!(affected_checkouts(&plan, &event), None);
        }

        let write = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![path],
            attrs: Default::default(),
        };
        assert_eq!(
            affected_checkouts(&plan, &write),
            Some(WatchUpdate {
                status: vec!["main".to_owned()],
                activity: vec!["main".to_owned()],
                worktrees: Vec::new(),
            })
        );
    }

    #[test]
    fn reading_a_ref_or_an_index_is_not_a_change_in_one() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned()],
            ..RepoWatchPlan::default()
        };
        // A path alone cannot tell a read from a write, and Linux reports every open of a watched
        // path as an event of its own. Every Git command opens the HEAD and the refs it reads
        // before it changes anything, so answering those as the writes they resemble refreshed the
        // whole repository on every command run inside it.
        for path in [
            "/repo/.git/HEAD",
            "/repo/.git/index",
            "/repo/.git/refs/heads/trunk",
            "/repo/base.txt",
        ] {
            let read = notify::Event {
                kind: notify::EventKind::Access(notify::event::AccessKind::Open(
                    notify::event::AccessMode::Any,
                )),
                paths: vec![PathBuf::from(path)],
                attrs: Default::default(),
            };

            assert_eq!(affected_checkouts(&plan, &read), None, "{path}");
        }
    }

    #[test]
    fn a_worktree_under_the_reserved_container_is_not_counted_as_primary_checkout_activity() {
        let mut plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned()],
            ..RepoWatchPlan::default()
        };
        let added_file = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/repo/.worktrees/task/new-file.txt")],
            attrs: Default::default(),
        };

        // The default worktree lives inside the root, but it is not part of the primary checkout.
        // Before the shared Git event registers it, treating these writes as primary edits would
        // repeatedly refresh and announce files from the new checkout.
        assert_eq!(affected_checkouts(&plan, &added_file), None);

        plan.roots
            .insert(PathBuf::from("/repo/.worktrees/task"), "task".to_owned());
        plan.all.push("task".to_owned());

        let update = affected_checkouts(&plan, &added_file).unwrap();
        assert_eq!(update.status, ["task"]);
        assert_eq!(update.activity, ["task"]);
    }

    #[test]
    fn a_worktree_staging_does_not_ask_the_whole_repository_to_be_read_again() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            git_dirs: BTreeMap::from([(
                PathBuf::from("/repo/.git/worktrees/task"),
                "task".to_owned(),
            )]),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned(), "task".to_owned()],
            requested: vec!["main".to_owned(), "task".to_owned()],
        };
        // A checkout's own index, lock and HEAD move on every command, and the lock appears and
        // disappears, so kind alone cannot tell them from a registration. Reading the whole
        // repository because an agent ran `git add` is the cost this signal has to refuse.
        let events = [
            notify::EventKind::Modify(notify::event::ModifyKind::Any),
            notify::EventKind::Create(notify::event::CreateKind::File),
            notify::EventKind::Remove(notify::event::RemoveKind::File),
        ];
        for path in [
            "/repo/.git/worktrees/task/HEAD",
            "/repo/.git/worktrees/task/index",
            "/repo/.git/worktrees/task/index.lock",
            "/repo/.git/worktrees/task/ORIG_HEAD",
        ] {
            for kind in events {
                let event = notify::Event {
                    kind,
                    paths: vec![PathBuf::from(path)],
                    attrs: Default::default(),
                };
                // The lock is filtered before anything is attributed, so it produces no update at
                // all. The other three still speak for the checkout that owns them, and say
                // nothing about the repository's worktrees.
                let expected = if path.ends_with(".lock") {
                    None
                } else {
                    Some(Vec::new())
                };
                assert_eq!(
                    affected_checkouts(&plan, &event).map(|update| update.worktrees),
                    expected,
                    "{path}"
                );
            }
        }
    }

    /// The self-inflicted loop: reading a checkout refreshed the index of a linked worktree, and
    /// the index is inside the directory the watcher watches, so every read announced itself and
    /// the next read was already on its way. A lock is a write in flight, and reading must not
    /// be one, so no lock in Git's own metadata speaks for anything.
    #[test]
    fn a_lock_in_git_metadata_is_not_a_change_while_the_rename_that_finishes_it_is() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [
                (PathBuf::from("/repo"), "main".to_owned()),
                (PathBuf::from("/repo-task"), "task".to_owned()),
            ]
            .into_iter()
            .collect(),
            // The primary's own Git directory is the shared one, so it is not here: what is written
            // under it is the repository's change, not the primary checkout's.
            git_dirs: BTreeMap::from([(
                PathBuf::from("/repo/.git/worktrees/task"),
                "task".to_owned(),
            )]),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned(), "task".to_owned()],
            requested: vec!["main".to_owned(), "task".to_owned()],
        };
        let kinds = [
            notify::EventKind::Modify(notify::event::ModifyKind::Any),
            notify::EventKind::Create(notify::event::CreateKind::File),
            notify::EventKind::Remove(notify::event::RemoveKind::File),
        ];
        for path in [
            // A linked worktree's own index, which is the one a `git status` there would lock.
            "/repo/.git/worktrees/task/index.lock",
            "/repo/.git/worktrees/task/HEAD.lock",
            // The primary checkout's index, which reaches the shared directory instead and is the
            // one that would re-read every worktree in the repository.
            "/repo/.git/index.lock",
            "/repo/.git/HEAD.lock",
            // A ref is written the same way, and a lock under one would move the merge base
            // every sibling counts its lines against.
            "/repo/.git/refs/heads/trunk.lock",
            "/repo/.git/packed-refs.lock",
        ] {
            for kind in kinds {
                let event = notify::Event {
                    kind,
                    paths: vec![PathBuf::from(path)],
                    attrs: Default::default(),
                };
                assert_eq!(affected_checkouts(&plan, &event), None, "{path}");
            }
        }

        // The rename that lands the write is the change, and it is the path that has to speak.
        // Both the checkout's own index and the repository's are asked here, because the first
        // names one worktree and the second names all of them.
        let staged = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/repo/.git/worktrees/task/index")],
            attrs: Default::default(),
        };
        assert_eq!(
            affected_checkouts(&plan, &staged).map(|update| update.status),
            Some(vec!["task".to_owned()])
        );
        let shared = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/repo/.git/index")],
            attrs: Default::default(),
        };
        assert_eq!(
            affected_checkouts(&plan, &shared).map(|update| update.status),
            Some(vec!["main".to_owned(), "task".to_owned()])
        );
    }

    /// A backend that does not batch reports the rename of a lock over the file it replaces as
    /// one event carrying both paths. Dropping the event because one of its paths is a lock would
    /// lose the change, so the lock is skipped and the destination still answers.
    #[test]
    fn a_rename_onto_a_git_file_answers_with_the_file_it_landed_on() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            git_dirs: BTreeMap::from([(
                PathBuf::from("/repo/.git/worktrees/task"),
                "task".to_owned(),
            )]),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned(), "task".to_owned()],
            requested: vec!["main".to_owned(), "task".to_owned()],
        };
        let rename = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![
                PathBuf::from("/repo/.git/worktrees/task/index.lock"),
                PathBuf::from("/repo/.git/worktrees/task/index"),
            ],
            attrs: Default::default(),
        };

        assert_eq!(
            affected_checkouts(&plan, &rename).map(|update| update.status),
            Some(vec!["task".to_owned()])
        );
    }

    /// The filter is about where Git writes, not about the shape of a name. A file in a checkout
    /// that ends in `.lock` is a file the user saved, and treating it as bookkeeping would leave
    /// the row describing an older version of the worktree than the one on disk.
    #[test]
    fn a_file_the_user_named_like_a_lock_is_still_a_change_to_their_checkout() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            git_dirs: BTreeMap::new(),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned()],
            requested: vec!["main".to_owned()],
        };
        let saved = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/repo/notes.lock")],
            attrs: Default::default(),
        };

        let update = affected_checkouts(&plan, &saved).unwrap();
        assert_eq!(update.status, ["main"]);
        assert_eq!(update.activity, ["main"]);
    }

    /// The half of the loop that does not depend on the watcher: a read that takes the optional
    /// index lock writes into the directory being watched, which is enough to keep the repository
    /// re-reading itself forever. Reading has to answer the same question from what is already on
    /// disk and leave the index alone.
    #[test]
    fn reading_the_status_leaves_the_index_exactly_as_git_wrote_it() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        init_repo(root);
        fs::write(root.join("edited.txt"), "first\n").unwrap();
        git(root, &["add", "edited.txt"]);
        git(root, &["commit", "-m", "second"]);

        // A file saved with the content it already had, and a timestamp Git has not seen: the
        // answer is that nothing changed, and getting there means comparing the file, because the
        // index on disk still describes the old timestamp. Recording the new one is the write the
        // optional lock exists for, and it is a write into the directory being watched.
        fs::write(root.join("base.txt"), "base\n").unwrap();
        Command::new("touch")
            .args(["-t", "202001010000", "base.txt"])
            .current_dir(root)
            .output()
            .expect("touch is installed");
        // Something genuinely edited, so the read has to be live and not merely quiet.
        fs::write(root.join("edited.txt"), "second\n").unwrap();

        let index = root.join(".git").join("index");
        let before = fs::read(&index).unwrap();
        let output = checked_git(
            root,
            ["status", "--porcelain=v2"],
            "could not read Git status",
        )
        .expect("Git status");
        let reported = String::from_utf8_lossy(&output.stdout);

        assert!(
            reported.contains("edited.txt"),
            "the edit was not reported: {reported}"
        );
        assert!(
            !reported.contains("base.txt"),
            "a file saved unchanged is still unchanged: {reported}"
        );
        assert_eq!(
            before,
            fs::read(&index).unwrap(),
            "reading the status rewrote the index it was reading"
        );
    }

    #[test]
    fn an_unregistered_worktree_writing_its_own_index_is_not_a_repository_wide_change() {
        let mut plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            git_dirs: BTreeMap::new(),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned()],
            requested: vec!["main".to_owned()],
        };
        let event = notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/repo/.git/worktrees/task/index")],
            attrs: Default::default(),
        };

        // Git writes the new worktree's index while it is still checking the branch out, before
        // the membership signal has named it. Every checkout holds an index of its own, so
        // answering that write with a read of all of them is the cost this signal refuses, and on
        // a backend that does not batch events it is the update that arrives instead.
        assert_eq!(affected_checkouts(&plan, &event), None);

        plan.git_dirs.insert(
            PathBuf::from("/repo/.git/worktrees/task"),
            "task".to_owned(),
        );
        plan.all.push("task".to_owned());

        let update = affected_checkouts(&plan, &event).unwrap();
        assert_eq!(update.status, ["task"]);
    }

    /// The whole chain a worktree an agent created travels, as far as the backend is concerned.
    /// A plan names the worktrees that are registered, so it is built from the database rather
    /// than from disk, and it grows only once something has read the disk. That something is the
    /// sync the worktree signal asks for, and the plan the watcher is then rebuilt for is the one
    /// that has the new worktree's directory and Git directory in it — a plan that did not grow
    /// is a watcher whose reach stops before the worktree the user is about to look at.
    #[test]
    fn a_worktree_added_outside_marvis_reaches_the_plan_and_is_therefore_watched() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let (database, _) = git_database(temp.path(), &root);
        let repo_id = database.load_workspace().unwrap().repos[0].id.clone();
        git(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "task",
                temp.path().join("task").to_str().unwrap(),
            ],
        );
        // Resolved, because the plan resolves: the filesystem reports resolved paths, and a
        // temporary directory reached through the `/var` symlink names a checkout the plan would
        // not match an event against.
        let added = temp.path().join("task").canonicalize().unwrap();

        // On disk and in Git, but nothing has registered it: the plan still watches the one
        // worktree, which is the whole reason the signal exists.
        let before = super::watch_plan(&database, &repo_id).unwrap();
        assert!(!before.roots.keys().any(|path| path == &added));

        workspace::sync_repo(
            &database,
            &crate::services::agent::AgentService::default(),
            &repo_id,
        )
        .unwrap();

        let after = super::watch_plan(&database, &repo_id).unwrap();
        assert!(after.roots.keys().any(|path| path == &added));
        assert_ne!(before, after);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_watch_keeps_requested_ids_for_a_symlink_invalid_checkout() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let linked = temp.path().join("task");
        let outside = temp.path().join("outside");
        init_repo(&root);
        git(
            &root,
            &["worktree", "add", "-b", "task", linked.to_str().unwrap()],
        );
        let (database, _) = git_database(temp.path(), &root);
        let state = workspace::register_folder(&database, &linked).unwrap();
        let repo = &state.repos[0];
        let mut requested: Vec<_> = repo
            .checkouts
            .iter()
            .filter(|checkout| !checkout.is_missing)
            .map(|checkout| checkout.id.clone())
            .collect();
        requested.sort();
        let primary = repo
            .checkouts
            .iter()
            .find(|checkout| checkout.is_primary)
            .unwrap()
            .id
            .clone();

        fs::remove_dir_all(&linked).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, &linked).unwrap();

        let plan = super::watch_plan(&database, &repo.id).unwrap();

        assert_eq!(plan.all, vec![primary]);
        assert_eq!(requested_watch_ids(&plan), requested);
        assert!(watch_plan_matches_request(&plan, &requested));
        assert!(!watch_plan_matches_request(&plan, &plan.all));
        assert_eq!(failed_watch_update(&plan).status, plan.requested);
        let failure = watch_failure(&plan, "registration-7");
        assert_eq!(failure.checkout_ids, requested);
        assert_eq!(failure.registration_id, "registration-7");
        let payload = serde_json::to_value(failure).unwrap();
        assert_eq!(payload["registrationId"], "registration-7");
        assert_eq!(payload["checkoutIds"], serde_json::json!(requested));
    }

    #[test]
    fn a_write_outside_every_worktree_and_the_git_directory_is_not_a_change() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            roots: [(PathBuf::from("/repo"), "main".to_owned())]
                .into_iter()
                .collect(),
            git_dirs: BTreeMap::new(),
            common_dir: Some(PathBuf::from("/repo/.git")),
            all: vec!["main".to_owned()],
            requested: vec!["main".to_owned()],
        };
        // Objects and logs churn on every Git command. Re-reading every checkout because a
        // blob was written is the cost this whole path exists to avoid.
        for path in ["/repo/.git/objects/aa/object", "/repo/.git/logs/HEAD"] {
            let event = notify::Event {
                kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
                paths: vec![PathBuf::from(path)],
                attrs: Default::default(),
            };
            assert_eq!(affected_checkouts(&plan, &event), None, "{path}");
        }
    }

    #[test]
    fn watcher_ignores_git_object_churn_but_tracks_status_metadata_and_checkout_files() {
        assert!(!should_refresh_path(Path::new(
            "/repo/.git/objects/aa/object"
        )));
        assert!(!should_refresh_path(Path::new("/repo/.git/logs/HEAD")));
        assert!(should_refresh_path(Path::new("/repo/.git/index")));
        assert!(!should_refresh_path(Path::new(
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

    #[test]
    fn numstat_records_are_keyed_by_the_new_path_of_a_rename_and_skip_uncounted_files() {
        // The two literals are concatenated so no `\0` sits next to a digit, which would
        // read as an octal escape rather than a NUL.
        let parsed = parse_numstat(
            b"2\t0\tbase.txt\0-\t-\tbinary.dat\0\
              1\t0\t\0old name.txt\0new name.txt\0\
              2\t3\tweird\tname.txt\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 3);
        assert_eq!(
            parsed["base.txt"],
            GitDiffStats {
                additions: 2,
                deletions: 0
            }
        );
        // A rename is keyed by the path the file list shows, not the one it came from.
        assert_eq!(
            parsed["new name.txt"],
            GitDiffStats {
                additions: 1,
                deletions: 0
            }
        );
        // A tab inside a path stays inside that path, because only NUL ends a record.
        assert_eq!(
            parsed["weird\tname.txt"],
            GitDiffStats {
                additions: 2,
                deletions: 3
            }
        );
        // A file Git could not count has no numbers, rather than zero lines.
        assert!(!parsed.contains_key("binary.dat"));
        assert!(parse_numstat(b"not a record\0").is_err());
    }

    /// What `git diff --no-index --numstat /dev/null <file>` prints, which is the answer the
    /// in-process count has to agree with.
    fn git_says_for_an_untracked_file(root: &Path, name: &str) -> Option<GitDiffStats> {
        let output = Command::new("git")
            .args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--numstat",
                "--no-index",
                "--",
                "/dev/null",
                name,
            ])
            .current_dir(root)
            .output()
            .expect("Git is installed");
        assert!(
            matches!(output.status.code(), Some(0) | Some(1)),
            "git diff --no-index failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let record = output
            .stdout
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or(b"");
        let mut columns = record.split(|byte| *byte == b'\t');
        let additions = columns.next()?;
        let deletions = columns.next()?;
        // Git prints `-` for both columns of a file it declined to count, and so does a row
        // here: no numbers at all, rather than a zero nobody measured.
        if additions == b"-" || deletions == b"-" {
            return None;
        }
        Some(GitDiffStats {
            additions: String::from_utf8_lossy(additions).parse().ok()?,
            deletions: String::from_utf8_lossy(deletions).parse().ok()?,
        })
    }

    /// The in-process count is a reimplementation of what Git does with `git diff --no-index`
    /// against an empty file, so the only honest way to hold it to that is to ask Git.
    #[test]
    fn counting_an_untracked_file_agrees_with_asking_git_to_diff_it_against_nothing() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let cases: &[(&str, &[u8])] = &[
            ("empty.txt", b""),
            ("one-line.txt", b"a\n"),
            // The last line has no newline of its own and is still a line.
            ("no-trailing-newline.txt", b"a\nb\nc"),
            ("three-lines.txt", b"a\nb\nc\n"),
            ("blank-lines.txt", b"\n\n\n"),
            ("crlf.txt", b"a\r\nb\r\n"),
            // A NUL byte is what makes Git call a file binary, and it looks only at the head.
            ("binary.dat", &[0, 1, 2, 3]),
            ("binary-late.dat", b"aaaa\nbbbb\n\0\n"),
        ];
        for (name, bytes) in cases {
            fs::write(root.join(name), bytes).unwrap();
        }

        for (name, _) in cases {
            assert_eq!(
                untracked_line_count(&root.join(name)),
                git_says_for_an_untracked_file(&root, name),
                "{name} counted differently than Git counts it"
            );
        }
    }

    #[test]
    fn an_untracked_file_git_cannot_read_is_left_out_rather_than_counted_as_zero() {
        let temp = tempdir().unwrap();
        assert_eq!(untracked_line_count(&temp.path().join("gone.txt")), None);
    }

    #[test]
    fn counts_lines_against_the_same_base_as_the_file_list() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        fs::write(root.join("gone.txt"), "gone\n").unwrap();
        fs::write(root.join("old name.txt"), "one\ntwo\n").unwrap();
        git(&root, &["add", "gone.txt", "old name.txt"]);
        git(&root, &["commit", "-m", "more files"]);
        let (database, checkout_id) = git_database(temp.path(), &root);
        let watchers = GitWatcherManager::default();

        git(&root, &["mv", "old name.txt", "new name.txt"]);
        fs::write(root.join("new name.txt"), "one\ntwo\nthree\n").unwrap();
        fs::write(root.join("base.txt"), "base\none\ntwo\n").unwrap();
        fs::write(root.join("added.txt"), "one\ntwo\n").unwrap();
        fs::write(root.join("binary.dat"), [0, 1, 2]).unwrap();
        fs::remove_file(root.join("gone.txt")).unwrap();

        let listed = status(&database, &watchers, &checkout_id).unwrap();
        let counted = super::diff_stats(&database, &watchers, &checkout_id).unwrap();
        let totals = super::checkout_diff_stats(&database, &watchers).unwrap();

        // The counted list is the same list, with the same paths in the same order, so a row
        // and the numbers on it can never describe different things.
        let paths: Vec<_> = counted.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(
            paths,
            listed
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>()
        );
        let counts = |path: &str| {
            let file = counted.iter().find(|file| file.path == path).unwrap();
            (file.additions, file.deletions)
        };
        assert_eq!(counts("base.txt"), (Some(2), Some(0)));
        assert_eq!(counts("added.txt"), (Some(2), Some(0)));
        assert_eq!(counts("gone.txt"), (Some(0), Some(1)));
        // A rename is counted under the path the file list shows, not the one it came from.
        assert_eq!(counts("new name.txt"), (Some(1), Some(0)));
        // Git has no count for a binary file, and the row says so instead of showing zero.
        assert_eq!(counts("binary.dat"), (None, None));
        // The per-checkout total is exactly the sum of its files.
        assert_eq!(
            totals[&checkout_id],
            GitDiffStats {
                additions: 5,
                deletions: 1
            }
        );
    }

    #[test]
    fn counts_default_to_the_whole_change_set_rather_than_the_default_branch_tip() {
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
        let (database, checkout_id) = git_database(temp.path(), &root);
        let watchers = GitWatcherManager::default();

        let listed = status(&database, &watchers, &checkout_id).unwrap();
        let counted = super::diff_stats(&database, &watchers, &checkout_id).unwrap();
        let counts: BTreeMap<_, _> = counted
            .iter()
            .map(|file| (file.path.as_str(), (file.additions, file.deletions)))
            .collect();

        assert_eq!(counts["feature.txt"], (Some(1), Some(0)));
        assert!(!counts.contains_key("default-only.txt"));
        assert!(!listed
            .files
            .iter()
            .any(|file| file.path == "default-only.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn sec_07_diff_stats_refuses_an_untracked_path_that_leaves_the_checkout() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret\n").unwrap();
        let (database, checkout_id) = git_database(temp.path(), &root);
        symlink(&outside, root.join("link")).unwrap();
        fs::write(root.join("added.txt"), "one\n").unwrap();

        let counted =
            super::diff_stats(&database, &GitWatcherManager::default(), &checkout_id).unwrap();
        let counts: BTreeMap<_, _> = counted
            .iter()
            .map(|file| (file.path.as_str(), file.additions))
            .collect();

        // A path that stays inside the checkout is still counted: the refusal is containment.
        assert_eq!(counts["added.txt"], Some(1));
        assert_eq!(counts["link"], None);
        assert!(!counts.contains_key("secret.txt"));
    }

    #[test]
    fn sec_08_diff_stats_answers_only_for_registered_git_checkouts() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("plain");
        fs::create_dir_all(&root).unwrap();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let state = workspace::register_folder(&database, &root).unwrap();
        let watchers = GitWatcherManager::default();

        let plain = state.repos[0].checkouts[0].id.clone();
        let error = super::diff_stats(&database, &watchers, &plain).unwrap_err();
        assert!(matches!(error.code, IpcErrorCode::InvalidCheckout));
        let error = super::diff_stats(&database, &watchers, "checkout:unknown").unwrap_err();
        assert!(matches!(error.code, IpcErrorCode::InvalidCheckout));
        // A plain folder is left out of the totals instead of being reported as a clean bill.
        assert_eq!(state.repos[0].kind, RepoKind::Plain);
        assert!(super::checkout_diff_stats(&database, &watchers)
            .unwrap()
            .is_empty());
    }

    fn cached_state(counts: Option<GitCounts>) -> CachedGitSnapshot {
        CachedGitSnapshot {
            default_branch: Some("trunk".to_owned()),
            revision: 0,
            stale: false,
            snapshot: super::Snapshot {
                status: GitStatus {
                    branch: None,
                    head: None,
                    default_branch: "trunk".to_owned(),
                    ahead_count: 0,
                    files: Vec::new(),
                },
                merge_base: "abc123".to_owned(),
            },
            counts,
        }
    }

    #[test]
    fn runtime_notify_error_is_reported_and_invalidates_the_repository_snapshot() {
        let plan = RepoWatchPlan {
            repo_id: "repo".to_owned(),
            all: vec!["checkout:a".to_owned(), "checkout:b".to_owned()],
            requested: vec!["checkout:a".to_owned(), "checkout:b".to_owned()],
            ..RepoWatchPlan::default()
        };
        let cache = GitSnapshotCache::default();
        for checkout_id in &plan.all {
            cache.insert(checkout_id.clone(), cached_state(None));
            assert!(cache.fresh(checkout_id, Some("trunk")).is_some());
        }

        let (sender, receiver) = mpsc::channel();
        let failed = AtomicBool::new(false);
        forward_watch_result(
            Err(notify::Error::generic("watch stream failed")),
            &plan,
            &sender,
            &failed,
        );
        assert_eq!(
            receive_debounced_change(&receiver, Duration::ZERO, Duration::ZERO),
            WatchWakeup::Failed("watch stream failed".to_owned())
        );

        let update = invalidate_failed_watch(&cache, &plan);
        assert_eq!(update.status, plan.all);
        assert!(cache.fresh("checkout:a", Some("trunk")).is_none());
        assert!(cache.fresh("checkout:b", Some("trunk")).is_none());

        // One terminal error fails the watcher; repeated callback errors do not queue retries.
        forward_watch_result(
            Err(notify::Error::generic("second error")),
            &plan,
            &sender,
            &failed,
        );
        assert!(receiver.try_recv().is_err());
        assert!(failed.load(Ordering::Acquire));
    }

    #[test]
    fn a_checkout_is_read_once_until_its_watcher_says_a_change_moved_it() {
        let cache = GitSnapshotCache::default();

        assert!(cache.fresh("checkout:a", Some("trunk")).is_none());
        cache.insert("checkout:a".to_owned(), cached_state(None));
        // The file list is held, which is what lets a `git status` and the counts of the same
        // checkout be one reading rather than two.
        assert_eq!(
            cache
                .fresh("checkout:a", Some("trunk"))
                .map(|held| held.snapshot.merge_base),
            Some("abc123".to_owned())
        );
        // Another default branch is another base ref, which no watcher can see.
        assert!(cache.fresh("checkout:a", Some("main")).is_none());
        cache.mark_stale(&["checkout:a".to_owned()]);
        assert!(cache.fresh("checkout:a", Some("trunk")).is_none());
        cache.insert("checkout:a".to_owned(), cached_state(None));
        assert_eq!(
            cache
                .fresh("checkout:a", Some("trunk"))
                .map(|held| held.snapshot.merge_base),
            Some("abc123".to_owned())
        );
        cache.forget_many(&["checkout:a".to_owned()]);
        assert!(cache.fresh("checkout:a", Some("trunk")).is_none());
    }

    #[test]
    fn a_change_drops_the_checkouts_it_speaks_for_and_leaves_the_others_held() {
        let cache = GitSnapshotCache::default();
        cache.insert("checkout:a".to_owned(), cached_state(None));
        cache.insert("checkout:b".to_owned(), cached_state(None));

        // A commit in one worktree moves the merge base every sibling counts against, so the
        // signal names all of them. A save in one names only that one, and the rest of the
        // sidebar is a lookup rather than another walk of the same working trees.
        cache.mark_stale(&["checkout:a".to_owned()]);
        assert!(cache.fresh("checkout:a", Some("trunk")).is_none());
        assert!(cache.fresh("checkout:b", Some("trunk")).is_some());
    }

    #[test]
    fn an_entry_holds_the_counts_once_something_has_asked_for_them() {
        let cache = GitSnapshotCache::default();
        let counts = GitCounts {
            files: Vec::new(),
            totals: GitDiffStats {
                additions: 7,
                deletions: 1,
            },
        };
        cache.insert("checkout:a".to_owned(), cached_state(Some(counts.clone())));

        // The sidebar and the Changes tab both want the lines on the same change, and neither
        // should pay for the other.
        assert_eq!(
            cache
                .fresh("checkout:a", Some("trunk"))
                .and_then(|held| held.counts),
            Some(counts)
        );
    }

    #[test]
    fn a_refresh_started_before_invalidation_cannot_repopulate_the_cache() {
        let cache = GitSnapshotCache::default();
        let checkout_id = "checkout:a".to_owned();
        cache.insert(checkout_id.clone(), cached_state(None));
        let refresh_revision = cache.revision(&checkout_id);

        cache.mark_stale(std::slice::from_ref(&checkout_id));
        cache.insert_at(checkout_id.clone(), cached_state(None), refresh_revision);

        assert!(cache.fresh(&checkout_id, Some("trunk")).is_none());
    }

    #[test]
    fn archiving_and_restoring_a_worktree_invalidates_its_cached_status_and_counts() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let worktree = temp.path().join("task");
        init_repo(&root);
        git(
            &root,
            &["worktree", "add", "-b", "task", worktree.to_str().unwrap()],
        );
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let state = workspace::register_folder(&database, &root).unwrap();
        let repo_id = state.repos[0].id.clone();
        let worktree_path = worktree.canonicalize().unwrap();
        let checkout_id = state.repos[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.canonical_path == worktree_path.to_string_lossy().as_ref())
            .unwrap()
            .id
            .clone();
        workspace::set_default_branch(&database, &repo_id, "trunk").unwrap();

        let app = tauri::test::mock_app();
        let watchers = GitWatcherManager::default();
        watchers
            .watch(
                app.handle().clone(),
                repo_id.clone(),
                "initial".to_owned(),
                super::watch_plan(&database, &repo_id).unwrap(),
            )
            .unwrap();
        assert!(status(&database, &watchers, &checkout_id)
            .unwrap()
            .files
            .is_empty());
        assert!(super::diff_stats(&database, &watchers, &checkout_id)
            .unwrap()
            .is_empty());

        workspace::archive_checkout(
            &database,
            &crate::services::agent::AgentService::default(),
            &checkout_id,
        )
        .unwrap();
        watchers
            .watch(
                app.handle().clone(),
                repo_id.clone(),
                "archived".to_owned(),
                super::watch_plan(&database, &repo_id).unwrap(),
            )
            .unwrap();
        assert!(watchers.cached_state(&checkout_id, Some("trunk")).is_none());

        fs::write(worktree.join("after-archive.txt"), "changed\n").unwrap();
        workspace::restore_archived_worktrees(
            &database,
            &crate::services::agent::AgentService::default(),
            &repo_id,
        )
        .unwrap();
        watchers
            .watch(
                app.handle().clone(),
                repo_id.clone(),
                "restored".to_owned(),
                super::watch_plan(&database, &repo_id).unwrap(),
            )
            .unwrap();

        assert!(status(&database, &watchers, &checkout_id)
            .unwrap()
            .files
            .iter()
            .any(|file| file.path == "after-archive.txt"));
        let counts = super::diff_stats(&database, &watchers, &checkout_id).unwrap();
        assert_eq!(
            counts
                .iter()
                .find(|file| file.path == "after-archive.txt")
                .unwrap()
                .additions,
            Some(1)
        );
        watchers.unwatch(&repo_id, &[]);
    }

    #[test]
    fn a_status_and_the_counts_of_one_checkout_are_the_same_reading() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        init_repo(&root);
        fs::write(root.join("added.txt"), "one\ntwo\n").unwrap();
        let (database, checkout_id) = git_database(temp.path(), &root);
        let watchers = GitWatcherManager::default();

        let listed = status(&database, &watchers, &checkout_id).unwrap();
        let counted = super::diff_stats(&database, &watchers, &checkout_id).unwrap();
        let totals = super::checkout_diff_stats(&database, &watchers).unwrap();

        // The file list the status reports and the file list the counts decorate are the same
        // list, which is only true if both came from one snapshot.
        assert_eq!(
            counted
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            listed
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            totals[&checkout_id],
            GitDiffStats {
                additions: 2,
                deletions: 0
            }
        );
    }
}
