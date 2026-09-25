//! Trusted Rust services invoke Git with argument arrays; no Tauri command exposes this runner.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use crate::{
    domain::{
        ipc::{IpcError, IpcErrorCode},
        workspace::{checkout_id_for_path, repo_id_for_path, Checkout, Repo, RepoKind},
    },
    services,
};

/// Runs a specific Git operation without shell parsing, off the Tauri/UI thread.
/// The checkout is resolved from the supplied repo contract before Git is invoked.
pub async fn run_git(
    repo: Repo,
    checkout_id: String,
    args: Vec<OsString>,
) -> Result<Output, IpcError> {
    tauri::async_runtime::spawn_blocking(move || {
        let cwd = services::checkout::resolve_checkout_path(&repo, &checkout_id, Path::new("."))?;
        command_for(&cwd, &args).output().map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not start Git: {error}"),
            )
        })
    })
    .await
    .map_err(|error| {
        IpcError::new(
            IpcErrorCode::OperationFailed,
            format!("Git operation could not complete: {error}"),
        )
    })?
}

fn command_for(cwd: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new("git");
    command.current_dir(cwd).args(args);
    command
}

#[derive(Default)]
struct Worktree {
    path: Option<PathBuf>,
    head: Option<String>,
    branch: Option<String>,
    bare: bool,
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<Output, IpcError> {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not start Git: {error}"),
            )
        })
}

fn output_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

fn parse_worktrees(output: &Output) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current = Worktree::default();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.is_empty() {
            if current.path.is_some() {
                worktrees.push(current);
            }
            current = Worktree::default();
            continue;
        }
        if line == "bare" {
            current.bare = true;
            continue;
        }
        if let Some((key, value)) = line.split_once(' ') {
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
                _ => {}
            }
        }
    }
    if current.path.is_some() {
        worktrees.push(current);
    }
    worktrees
}

fn canonical_or_original(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

/// Detects a Git checkout at an existing canonical directory. `None` means it is a plain folder.
/// For submodules, Git's worktree listing can expose the module gitdir instead of the checkout;
/// an explicitly opened submodule is therefore represented as its own standalone repo.
pub fn resolve_repository(
    requested_path: &Path,
    now: &str,
) -> Result<Option<(Repo, String)>, IpcError> {
    let bare = git_output(requested_path, &["rev-parse", "--is-bare-repository"])?;
    if bare.status.success() && output_text(&bare) == "true" {
        return Err(IpcError::new(
            IpcErrorCode::NotRepository,
            "Bare repositories are not supported; open a working checkout instead.",
        ));
    }

    let top = git_output(requested_path, &["rev-parse", "--show-toplevel"])?;
    if !top.status.success() {
        return Ok(None);
    }
    let checkout_root = PathBuf::from(output_text(&top))
        .canonicalize()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not resolve Git checkout root: {error}"),
            )
        })?;
    let superproject = git_output(
        &checkout_root,
        &["rev-parse", "--show-superproject-working-tree"],
    )?;
    let is_submodule = superproject.status.success() && !output_text(&superproject).is_empty();

    let listed = git_output(&checkout_root, &["worktree", "list", "--porcelain"])?;
    if !listed.status.success() {
        return Err(IpcError::new(
            IpcErrorCode::GitFailed,
            format!(
                "could not list Git worktrees: {}",
                String::from_utf8_lossy(&listed.stderr).trim()
            ),
        ));
    }

    let mut worktrees: Vec<_> = parse_worktrees(&listed)
        .into_iter()
        .filter(|entry| !entry.bare)
        .collect();
    let opened_is_listed = worktrees.iter().any(|entry| {
        entry
            .path
            .as_deref()
            .map(|path| canonical_or_original(path.to_path_buf()) == checkout_root)
            .unwrap_or(false)
    });
    if !opened_is_listed && is_submodule {
        let branch = git_output(
            &checkout_root,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output_text(&output));
        let head = git_output(&checkout_root, &["rev-parse", "--verify", "HEAD"])
            .ok()
            .filter(|output| output.status.success())
            .map(|output| output_text(&output));
        worktrees = vec![Worktree {
            path: Some(checkout_root.clone()),
            head,
            branch,
            bare: false,
        }];
    } else if !opened_is_listed || worktrees.is_empty() {
        return Err(IpcError::new(
            IpcErrorCode::GitFailed,
            format!(
                "Git did not list the containing checkout: {}",
                checkout_root.display()
            ),
        ));
    }

    let primary_root = worktrees
        .first()
        .and_then(|entry| entry.path.clone())
        .map(canonical_or_original)
        .ok_or_else(|| {
            IpcError::new(IpcErrorCode::GitFailed, "Git returned no primary checkout")
        })?;
    let repo_id = repo_id_for_path(&primary_root.to_string_lossy());
    let opened_checkout_id = checkout_id_for_path(&checkout_root.to_string_lossy());

    let checkouts = worktrees
        .into_iter()
        .filter_map(|entry| {
            let raw_path = entry.path?;
            let path = canonical_or_original(raw_path);
            let canonical_path = path.to_string_lossy().into_owned();
            Some(Checkout {
                id: checkout_id_for_path(&canonical_path),
                repo_id: repo_id.clone(),
                path: canonical_path.clone(),
                canonical_path: canonical_path.clone(),
                is_primary: path == primary_root,
                branch: entry.branch,
                head: entry.head,
                ahead_of_default: None,
                changed_files: 0,
                is_missing: !path.is_dir(),
                sessions: Vec::new(),
            })
        })
        .collect();

    let origin_head = git_output(
        &checkout_root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .ok()
    .filter(|output| output.status.success())
    .map(|output| output_text(&output))
    .and_then(|branch| branch.strip_prefix("origin/").map(str::to_owned))
    .filter(|branch| {
        git_output(
            &checkout_root,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{branch}"),
            ],
        )
        .is_ok_and(|output| output.status.success())
    });
    let path = primary_root.to_string_lossy().into_owned();
    let name = primary_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.clone());
    let repo = Repo {
        id: repo_id,
        kind: RepoKind::Git,
        name,
        root: path,
        default_branch: origin_head,
        checkouts,
        created_at: now.to_owned(),
        last_opened_at: now.to_owned(),
    };
    Ok(Some((repo, opened_checkout_id)))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path};

    use super::command_for;

    #[test]
    fn metacharacter_filename_stays_one_literal_git_argument() {
        let filename = "$(touch must-not-run); *.txt";
        let args = [
            OsString::from("diff"),
            OsString::from("--"),
            OsString::from(filename),
        ];
        let command = command_for(Path::new("."), &args);
        let actual: Vec<_> = command.get_args().map(OsString::from).collect();

        assert_eq!(actual, args);
    }
}
