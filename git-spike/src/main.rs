use serde::Serialize;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

#[derive(Debug)]
struct GitError {
    args: Vec<String>,
    cwd: PathBuf,
    status: Option<i32>,
    stderr: String,
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "git {} failed (exit code {:?}) in {}: {}",
            self.args.join(" "),
            self.status,
            self.cwd.display(),
            self.stderr.trim()
        )
    }
}

impl Error for GitError {}

#[derive(Debug)]
struct UnsupportedRepository(String);

impl fmt::Display for UnsupportedRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for UnsupportedRepository {}

#[derive(Debug, Serialize)]
struct Checkout {
    path: String,
    head: Option<String>,
    branch: Option<String>,
    detached: bool,
    bare: bool,
    prunable: bool,
    exists: bool,
}

#[derive(Debug, Serialize)]
struct DefaultBranch {
    branch: Option<String>,
    source: String,
    requires_confirmation: bool,
}

#[derive(Debug, Serialize)]
struct Resolution {
    requested_path: String,
    primary_root: String,
    checkout_root: String,
    is_primary: bool,
    branch: Option<String>,
    head: Option<String>,
    default_branch: DefaultBranch,
    superproject_root: Option<String>,
    checkouts: Vec<Checkout>,
    warning: Option<String>,
}

fn os_args(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn git<I, S>(args: I, cwd: &Path, check: bool) -> Result<Output, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args
        .into_iter()
        .map(|argument| argument.as_ref().to_os_string())
        .collect();
    let result = Command::new("git")
        .args(&args)
        .current_dir(cwd)
        .output()
        .map_err(|error| GitError {
            args: args
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
            cwd: cwd.to_path_buf(),
            status: None,
            stderr: error.to_string(),
        })?;
    if check && !result.status.success() {
        return Err(GitError {
            args: args
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
            cwd: cwd.to_path_buf(),
            status: result.status.code(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        });
    }
    Ok(result)
}

fn output_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn trim_git_output(output: &Output) -> String {
    output_text(output)
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

#[derive(Default)]
struct WorktreeEntry {
    path: Option<PathBuf>,
    head: Option<String>,
    branch: Option<String>,
    detached: bool,
    bare: bool,
    prunable: bool,
}

fn finish_worktree(entry: WorktreeEntry, entries: &mut Vec<Checkout>) {
    let Some(path) = entry.path else {
        return;
    };
    let exists = path.is_dir();
    let path = if exists {
        path.canonicalize().unwrap_or(path)
    } else {
        path
    };
    entries.push(Checkout {
        path: path.to_string_lossy().into_owned(),
        head: entry.head,
        branch: entry.branch,
        detached: entry.detached,
        bare: entry.bare,
        prunable: entry.prunable,
        exists,
    });
}

fn worktrees(repo_path: &Path) -> Result<Vec<Checkout>, GitError> {
    let result = git(
        os_args(&["worktree", "list", "--porcelain"]),
        repo_path,
        true,
    )?;
    let mut entries = Vec::new();
    let mut current = WorktreeEntry::default();
    for line in output_text(&result).lines() {
        if line.is_empty() {
            finish_worktree(current, &mut entries);
            current = WorktreeEntry::default();
            continue;
        }
        let Some((key, value)) = line.split_once(' ') else {
            match line {
                "detached" => current.detached = true,
                "bare" => current.bare = true,
                _ => {}
            }
            continue;
        };
        match key {
            "worktree" => current.path = Some(PathBuf::from(value)),
            "HEAD" => current.head = Some(value.to_owned()),
            "branch" => {
                current.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_owned(),
                )
            }
            "prunable" => current.prunable = true,
            _ => {}
        }
    }
    finish_worktree(current, &mut entries);
    Ok(entries)
}

fn symbolic_branch(path: &Path) -> Option<String> {
    let result = git(
        os_args(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
        path,
        false,
    )
    .ok()?;
    result.status.success().then(|| trim_git_output(&result))
}

fn head(path: &Path) -> Option<String> {
    let result = git(os_args(&["rev-parse", "--verify", "HEAD"]), path, false).ok()?;
    result.status.success().then(|| trim_git_output(&result))
}

fn default_branch(path: &Path, branch: Option<&str>) -> DefaultBranch {
    let result = git(
        os_args(&[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ]),
        path,
        false,
    );
    if let Ok(result) = result {
        if result.status.success() {
            let name = trim_git_output(&result);
            if let Some(name) = name.strip_prefix("origin/").filter(|name| !name.is_empty()) {
                return DefaultBranch {
                    branch: Some(name.to_owned()),
                    source: "origin/HEAD".to_owned(),
                    requires_confirmation: false,
                };
            }
        }
    }
    DefaultBranch {
        branch: branch.map(str::to_owned),
        source: "current-HEAD-candidate".to_owned(),
        requires_confirmation: true,
    }
}

fn resolve(path: impl AsRef<Path>) -> Result<Resolution, Box<dyn Error>> {
    let requested = path.as_ref().canonicalize()?;
    if !requested.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a directory", requested.display()),
        )
        .into());
    }

    let bare = git(
        os_args(&["rev-parse", "--is-bare-repository"]),
        &requested,
        false,
    )?;
    if bare.status.success() && trim_git_output(&bare) == "true" {
        return Err(UnsupportedRepository(format!(
            "Bare repository is not a checkout: {}",
            requested.display()
        ))
        .into());
    }

    let top = git(
        os_args(&["rev-parse", "--show-toplevel"]),
        &requested,
        false,
    )?;
    if !top.status.success() {
        return Err(GitError {
            args: vec!["rev-parse".to_owned(), "--show-toplevel".to_owned()],
            cwd: requested.clone(),
            status: top.status.code(),
            stderr: String::from_utf8_lossy(&top.stderr).into_owned(),
        }
        .into());
    }
    let checkout_root = PathBuf::from(trim_git_output(&top)).canonicalize()?;
    let superproject_output = git(
        os_args(&["rev-parse", "--show-superproject-working-tree"]),
        &checkout_root,
        false,
    )?;
    let superproject = if superproject_output.status.success() {
        let value = trim_git_output(&superproject_output);
        (!value.is_empty()).then(|| PathBuf::from(value))
    } else {
        None
    };

    let mut checkouts = worktrees(&checkout_root)?;
    let live: Vec<&Checkout> = checkouts.iter().filter(|entry| !entry.bare).collect();
    if live.is_empty() {
        return Err(UnsupportedRepository(format!(
            "No working checkout found for repository: {}",
            requested.display()
        ))
        .into());
    }

    let matching = live
        .iter()
        .find(|entry| Path::new(&entry.path) == checkout_root);
    let mut warning = None;
    let primary_root = if let Some(_) = matching {
        PathBuf::from(&live[0].path)
    } else if superproject.is_some() {
        let branch = symbolic_branch(&checkout_root);
        let current_head = head(&checkout_root);
        checkouts = vec![Checkout {
            path: checkout_root.to_string_lossy().into_owned(),
            head: current_head,
            detached: branch.is_none(),
            branch,
            bare: false,
            prunable: false,
            exists: true,
        }];
        warning = Some("Git worktree list reports the submodule gitdir; only the opened submodule checkout is represented.".to_owned());
        checkout_root.clone()
    } else {
        return Err(io::Error::other(format!(
            "Git did not list the containing checkout: {}",
            checkout_root.display()
        ))
        .into());
    };
    let branch = symbolic_branch(&checkout_root);
    Ok(Resolution {
        requested_path: requested.to_string_lossy().into_owned(),
        primary_root: primary_root.to_string_lossy().into_owned(),
        checkout_root: checkout_root.to_string_lossy().into_owned(),
        is_primary: primary_root == checkout_root,
        branch: branch.clone(),
        head: head(&checkout_root),
        default_branch: default_branch(&checkout_root, branch.as_deref()),
        superproject_root: superproject.map(|path| {
            path.canonicalize()
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        }),
        checkouts,
        warning,
    })
}

fn status(path: impl AsRef<Path>) -> Result<String, Box<dyn Error>> {
    let root = path.as_ref().canonicalize()?;
    let result = git(os_args(&["status", "--porcelain=v2"]), &root, true)?;
    Ok(output_text(&result))
}

fn initialize_repo(path: &Path, branch: &str, commit: bool) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(path)?;
    let branch_args = vec![OsString::from("init"), OsString::from("-b"), branch.into()];
    git(branch_args, path, true)?;
    git(os_args(&["config", "user.name", "Git Spike"]), path, true)?;
    git(
        os_args(&["config", "user.email", "spike@example.invalid"]),
        path,
        true,
    )?;
    if commit {
        fs::write(path.join("initial.txt"), "initial\n")?;
        git(os_args(&["add", "initial.txt"]), path, true)?;
        git(os_args(&["commit", "-m", "initial"]), path, true)?;
    }
    Ok(())
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> io::Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..100 {
            let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("{prefix}{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a temporary directory",
        ))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Serialize)]
struct BenchmarkResult {
    tracked_files: usize,
    repeats: usize,
    median_ms: f64,
    samples_ms: Vec<f64>,
}

fn benchmark_status(
    counts: &[usize],
    repeats: usize,
) -> Result<Vec<BenchmarkResult>, Box<dyn Error>> {
    if repeats == 0 {
        return Err("repeats must be greater than zero".into());
    }
    let temporary = TempDir::new("marvis-git-status-")?;
    let mut results = Vec::new();
    for &count in counts {
        let repo = temporary.0.join(format!("repo-{count}"));
        initialize_repo(&repo, "main", false)?;
        for index in 0..count {
            fs::write(
                repo.join(format!("file-{index:06}.txt")),
                "tracked fixture\n",
            )?;
        }
        git(os_args(&["add", "--all"]), &repo, true)?;
        let commit_message = format!("seed {count} tracked files");
        git(
            [
                OsString::from("commit"),
                OsString::from("-m"),
                commit_message.into(),
            ],
            &repo,
            true,
        )?;

        let mut samples = Vec::with_capacity(repeats);
        for _ in 0..repeats {
            let started = Instant::now();
            git(os_args(&["status", "--porcelain=v2"]), &repo, true)?;
            samples.push(started.elapsed());
        }
        let mut sorted = samples.clone();
        sorted.sort_unstable();
        let median = if repeats % 2 == 0 {
            (sorted[repeats / 2 - 1].as_secs_f64() + sorted[repeats / 2].as_secs_f64()) / 2.0
        } else {
            sorted[repeats / 2].as_secs_f64()
        };
        results.push(BenchmarkResult {
            tracked_files: count,
            repeats,
            median_ms: (median * 1000.0 * 100.0).round() / 100.0,
            samples_ms: samples
                .iter()
                .map(|sample| (sample.as_secs_f64() * 1000.0 * 100.0).round() / 100.0)
                .collect(),
        });
    }
    Ok(results)
}

fn usage() -> &'static str {
    "Usage:\n  git-spike resolve <path>\n  git-spike status <path>\n  git-spike bench [--counts <n> ...] [--repeats <n>]"
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let Some(command) = arguments.next() else {
        return Err(usage().into());
    };
    match command.as_str() {
        "resolve" => {
            let path = arguments.next().ok_or(usage())?;
            let result = resolve(path)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        "status" => {
            let path = arguments.next().ok_or(usage())?;
            print!("{}", status(path)?);
        }
        "bench" => {
            let mut counts = vec![100, 5_000, 50_000];
            let mut repeats = 5;
            let options: Vec<String> = arguments.collect();
            let mut index = 0;
            while index < options.len() {
                match options[index].as_str() {
                    "--counts" => {
                        index += 1;
                        counts.clear();
                        while index < options.len() && !options[index].starts_with("--") {
                            counts.push(options[index].parse()?);
                            index += 1;
                        }
                        if counts.is_empty() {
                            return Err("--counts requires at least one count".into());
                        }
                    }
                    "--repeats" => {
                        index += 1;
                        repeats = options
                            .get(index)
                            .ok_or("--repeats requires a value")?
                            .parse()?;
                        index += 1;
                    }
                    option => {
                        return Err(
                            format!("unknown benchmark option: {option}\n{}", usage()).into()
                        )
                    }
                }
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&benchmark_status(&counts, repeats)?)?
            );
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture(TempDir);

    impl Fixture {
        fn new() -> Self {
            Self(TempDir::new("marvis-git-test-").unwrap())
        }

        fn path(&self) -> &Path {
            &self.0 .0
        }

        fn init_repo(&self, name: &str, branch: &str, commit: bool) -> PathBuf {
            let repo = self.path().join(name);
            initialize_repo(&repo, branch, commit).unwrap();
            repo
        }
    }

    fn fixture_git<I, S>(cwd: &Path, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        git(args, cwd, true).unwrap()
    }

    #[test]
    fn primary_worktree_subdirectory_and_symlink_resolve_canonical_identity() {
        let fixture = Fixture::new();
        let repo = fixture.init_repo("repo", "main", true);
        let linked = repo.join(".worktrees/task");
        fs::create_dir_all(linked.parent().unwrap()).unwrap();
        fixture_git(
            &repo,
            [
                OsString::from("worktree"),
                "add".into(),
                "-b".into(),
                "task".into(),
                linked.as_os_str().into(),
            ],
        );
        let primary_subdir = repo.join("nested");
        let linked_subdir = linked.join("nested");
        fs::create_dir(&primary_subdir).unwrap();
        fs::create_dir(&linked_subdir).unwrap();
        let alias = fixture.path().join("primary-alias");
        symlink(&repo, &alias).unwrap();

        let primary = resolve(&repo).unwrap();
        let from_linked = resolve(&linked).unwrap();
        let from_primary_subdir = resolve(&primary_subdir).unwrap();
        let from_linked_subdir = resolve(&linked_subdir).unwrap();
        let from_symlink = resolve(alias.join("nested")).unwrap();
        let repo = repo.canonicalize().unwrap().to_string_lossy().into_owned();
        let linked = linked
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        assert_eq!(primary.primary_root, repo);
        assert_eq!(from_linked.primary_root, primary.primary_root);
        assert_eq!(from_linked.checkout_root, linked);
        assert!(!from_linked.is_primary);
        assert_eq!(from_primary_subdir.checkout_root, primary.checkout_root);
        assert_eq!(from_linked_subdir.checkout_root, from_linked.checkout_root);
        assert_eq!(from_symlink.checkout_root, primary.checkout_root);
        assert_eq!(
            from_linked
                .checkouts
                .iter()
                .filter_map(|checkout| checkout.branch.as_deref())
                .collect::<std::collections::HashSet<_>>(),
            ["main", "task"].into_iter().collect()
        );
    }

    #[test]
    fn origin_head_is_authoritative_and_detached_head_is_retained() {
        let fixture = Fixture::new();
        let repo = fixture.init_repo("repo", "main", true);
        let remote = fixture.path().join("remote.git");
        fs::create_dir(&remote).unwrap();
        fixture_git(&remote, os_args(&["init", "--bare"]));
        fixture_git(&repo, ["remote", "add", "origin", remote.to_str().unwrap()]);
        fixture_git(&repo, ["push", "-u", "origin", "main"]);
        fixture_git(
            &repo,
            [
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        fixture_git(&repo, ["switch", "-c", "feature"]);
        let result = resolve(&repo).unwrap();
        assert_eq!(result.default_branch.branch.as_deref(), Some("main"));
        assert_eq!(result.default_branch.source, "origin/HEAD");
        assert!(!result.default_branch.requires_confirmation);

        fixture_git(&repo, ["checkout", "--detach"]);
        let detached = resolve(&repo).unwrap();
        assert!(detached.branch.is_none());
        assert!(detached.head.is_some());
        assert!(
            detached
                .checkouts
                .iter()
                .find(|checkout| checkout.path == detached.checkout_root)
                .unwrap()
                .detached
        );
    }

    #[test]
    fn unborn_and_no_remote_branch_are_candidates_requiring_confirmation() {
        let fixture = Fixture::new();
        let repo = fixture.init_repo("repo", "trunk", false);
        let unborn = resolve(&repo).unwrap();
        assert_eq!(unborn.branch.as_deref(), Some("trunk"));
        assert!(unborn.head.is_none());
        assert_eq!(unborn.default_branch.branch.as_deref(), Some("trunk"));
        assert!(unborn.default_branch.requires_confirmation);

        fs::write(repo.join("initial.txt"), "initial\n").unwrap();
        fixture_git(&repo, ["add", "initial.txt"]);
        fixture_git(&repo, ["commit", "-m", "initial"]);
        fixture_git(&repo, ["switch", "-c", "feature"]);
        let result = resolve(&repo).unwrap();
        assert_eq!(result.default_branch.branch.as_deref(), Some("feature"));
        assert!(result.default_branch.requires_confirmation);
    }

    #[test]
    fn missing_worktree_is_retained_as_prunable_and_cannot_be_opened() {
        let fixture = Fixture::new();
        let repo = fixture.init_repo("repo", "main", true);
        let linked = fixture.path().join("deleted-checkout");
        fixture_git(
            &repo,
            [
                OsString::from("worktree"),
                "add".into(),
                "-b".into(),
                "temporary".into(),
                linked.as_os_str().into(),
            ],
        );
        let linked = linked.canonicalize().unwrap();
        fs::remove_dir_all(&linked).unwrap();
        let entries = resolve(&repo).unwrap().checkouts;
        let missing = entries
            .iter()
            .find(|entry| entry.path == linked.to_string_lossy())
            .unwrap();
        assert!(!missing.exists);
        assert!(missing.prunable);
        assert!(resolve(&linked).is_err());
    }

    #[test]
    fn git_rejects_a_branch_in_use_and_listing_identifies_its_checkout() {
        let fixture = Fixture::new();
        let repo = fixture.init_repo("repo", "main", true);
        let linked = fixture.path().join("task-checkout");
        fixture_git(
            &repo,
            [
                OsString::from("worktree"),
                "add".into(),
                "-b".into(),
                "occupied".into(),
                linked.as_os_str().into(),
            ],
        );
        let second = fixture.path().join("second-checkout");
        let failed = git(
            [
                OsString::from("worktree"),
                "add".into(),
                second.as_os_str().into(),
                "occupied".into(),
            ],
            &repo,
            false,
        )
        .unwrap();
        assert!(!failed.status.success());
        let holder = resolve(&repo)
            .unwrap()
            .checkouts
            .into_iter()
            .find(|entry| entry.branch.as_deref() == Some("occupied"))
            .unwrap();
        assert_eq!(
            holder.path,
            linked.canonicalize().unwrap().to_string_lossy()
        );
        assert!(String::from_utf8_lossy(&failed.stderr)
            .to_lowercase()
            .contains("already used by worktree"));
    }

    #[test]
    fn worktree_inside_dot_worktrees_is_resolved_and_seen_by_parent_status() {
        let fixture = Fixture::new();
        let repo = fixture.init_repo("repo", "main", true);
        let linked = repo.join(".worktrees/task");
        fs::create_dir_all(linked.parent().unwrap()).unwrap();
        fixture_git(
            &repo,
            [
                OsString::from("worktree"),
                "add".into(),
                "-b".into(),
                "task".into(),
                linked.as_os_str().into(),
            ],
        );
        assert_eq!(
            resolve(&linked).unwrap().checkout_root,
            linked.canonicalize().unwrap().to_string_lossy()
        );
        assert!(status(&repo).unwrap().contains(".worktrees/"));
    }

    #[test]
    fn submodule_is_its_own_repo_and_bare_repo_is_rejected() {
        let fixture = Fixture::new();
        let child = fixture.init_repo("child", "main", true);
        let parent = fixture.init_repo("parent", "main", true);
        let mut submodule_args = os_args(&["-c", "protocol.file.allow=always", "submodule", "add"]);
        submodule_args.push(child.as_os_str().to_os_string());
        submodule_args.extend(os_args(&["vendor/child"]));
        fixture_git(&parent, submodule_args);
        fixture_git(&parent, ["commit", "-am", "add child submodule"]);
        let nested_path = parent.join("vendor/child");
        let nested = resolve(&nested_path).unwrap();
        assert_eq!(
            nested.checkout_root,
            nested_path.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(nested.primary_root, nested.checkout_root);
        assert_eq!(
            nested.superproject_root.as_deref(),
            Some(parent.canonicalize().unwrap().to_string_lossy().as_ref())
        );
        assert!(nested
            .warning
            .as_deref()
            .unwrap()
            .contains("Git worktree list reports the submodule gitdir"));

        let bare = fixture.path().join("bare.git");
        fs::create_dir(&bare).unwrap();
        fixture_git(&bare, os_args(&["init", "--bare"]));
        let error = resolve(&bare).unwrap_err();
        assert!(error
            .to_string()
            .contains("Bare repository is not a checkout"));
    }

    #[test]
    fn status_returns_porcelain_v2_and_benchmark_reports_all_samples() {
        let fixture = Fixture::new();
        let repo = fixture.init_repo("repo", "main", true);
        fs::write(repo.join("changed.txt"), "change\n").unwrap();
        assert!(status(&repo).unwrap().contains("? changed.txt"));

        let results = benchmark_status(&[2], 2).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tracked_files, 2);
        assert_eq!(results[0].samples_ms.len(), 2);
        assert!(results[0].median_ms >= 0.0);
    }
}
