use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{
    domain::{
        files::{FileContent, FileEntry, FileEntryKind, FileTree},
        ipc::{IpcError, IpcErrorCode},
        workspace::{Repo, RepoKind},
    },
    persistence::Database,
};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 2000;

pub fn list(
    database: &Database,
    checkout_id: &str,
    relative_path: &str,
) -> Result<FileTree, IpcError> {
    let repo = registered_repo(database, checkout_id)?;
    let checkout = repo
        .checkouts
        .iter()
        .find(|checkout| checkout.id == checkout_id)
        .unwrap();
    ensure_checkout_available(checkout.is_missing, &checkout.canonical_path)?;
    let root = Path::new(&checkout.canonical_path);
    let requested = parse_relative_path(relative_path)?;
    let directory =
        crate::services::checkout::resolve_checkout_path(&repo, checkout_id, &requested)?;
    let metadata = fs::metadata(&directory)
        .map_err(|error| filesystem_error("could not inspect folder", error))?;
    if !metadata.is_dir() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "requested path is not a directory",
        ));
    }

    let mut collected = Vec::new();
    let mut truncated = false;
    for item in fs::read_dir(&directory)
        .map_err(|error| filesystem_error("could not list folder", error))?
    {
        let item = item.map_err(|error| filesystem_error("could not read folder entry", error))?;
        let name = match item.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };
        if name == ".git" {
            continue;
        }
        let file_type = item
            .file_type()
            .map_err(|error| filesystem_error("could not inspect folder entry", error))?;
        let kind = if file_type.is_symlink() {
            FileEntryKind::Symlink
        } else if file_type.is_dir() {
            FileEntryKind::Directory
        } else if file_type.is_file() {
            FileEntryKind::File
        } else {
            continue;
        };
        let entry_path = item.path();
        let relative = entry_path.strip_prefix(root).map_err(|_| {
            IpcError::new(
                IpcErrorCode::PathOutsideCheckout,
                "entry is outside checkout",
            )
        })?;
        let Some(path) = relative.to_str() else {
            continue;
        };
        collected.push(FileEntry {
            name,
            path: path.replace(std::path::MAIN_SEPARATOR, "/"),
            kind,
        });
        if collected.len() > MAX_DIRECTORY_ENTRIES {
            truncated = true;
            collected.pop();
            break;
        }
    }

    if repo.kind == RepoKind::Git {
        let ignored = ignored_paths(root, &collected)?;
        collected.retain(|entry| !ignored.contains(entry.path.as_str()));
    }
    collected.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(FileTree {
        entries: collected,
        truncated,
    })
}

pub fn read(
    database: &Database,
    checkout_id: &str,
    relative_path: &Path,
) -> Result<FileContent, IpcError> {
    let repo = registered_repo(database, checkout_id)?;
    let checkout = repo
        .checkouts
        .iter()
        .find(|checkout| checkout.id == checkout_id)
        .unwrap();
    ensure_checkout_available(checkout.is_missing, &checkout.canonical_path)?;
    let relative_path = relative_path
        .to_str()
        .ok_or_else(|| IpcError::new(IpcErrorCode::InvalidPath, "file path is not valid UTF-8"))?;
    let relative_path = parse_relative_path(relative_path)?;
    let path =
        crate::services::checkout::resolve_checkout_path(&repo, checkout_id, &relative_path)?;
    let metadata =
        fs::metadata(&path).map_err(|error| filesystem_error("could not inspect file", error))?;
    if !metadata.is_file() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "selected path is not a file",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(too_large());
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| filesystem_error("could not read file", error))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(too_large());
    }
    if bytes.contains(&0) {
        return Err(binary_file());
    }
    let content = String::from_utf8(bytes).map_err(|_| binary_file())?;
    let path = relative_path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    Ok(FileContent { path, content })
}

fn registered_repo(database: &Database, checkout_id: &str) -> Result<Repo, IpcError> {
    database
        .load_workspace()
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?
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
        })
}

fn parse_relative_path(path: &str) -> Result<PathBuf, IpcError> {
    if path.is_empty() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "path must be relative to the checkout",
        ));
    }
    let path = Path::new(path);
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        ) || matches!(component, std::path::Component::Normal(name) if name == ".git")
    }) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "path must be relative, cannot contain '..', and cannot access .git",
        ));
    }
    Ok(path.to_path_buf())
}

fn ignored_paths(root: &Path, entries: &[FileEntry]) -> Result<HashSet<String>, IpcError> {
    if entries.is_empty() {
        return Ok(HashSet::new());
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "--no-index", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not apply Git ignore rules: {error}"),
            )
        })?;
    {
        let stdin = child.stdin.as_mut().expect("piped stdin");
        for entry in entries {
            stdin
                .write_all(entry.path.as_bytes())
                .and_then(|_| stdin.write_all(&[0]))
                .map_err(|error| {
                    IpcError::new(
                        IpcErrorCode::GitFailed,
                        format!("could not apply Git ignore rules: {error}"),
                    )
                })?;
        }
    }
    let output = child.wait_with_output().map_err(|error| {
        IpcError::new(
            IpcErrorCode::GitFailed,
            format!("could not apply Git ignore rules: {error}"),
        )
    })?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(IpcError::new(
            IpcErrorCode::GitFailed,
            "Git could not apply ignore rules",
        ));
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .filter_map(|path| std::str::from_utf8(path).ok().map(str::to_owned))
        .collect())
}

fn filesystem_error(action: &str, error: std::io::Error) -> IpcError {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => IpcErrorCode::FolderMissing,
        std::io::ErrorKind::PermissionDenied => IpcErrorCode::PermissionDenied,
        _ => IpcErrorCode::OperationFailed,
    };
    IpcError::new(code, format!("{action}: {error}"))
}

fn missing(message: &str) -> IpcError {
    IpcError::new(IpcErrorCode::FolderMissing, message)
}

fn ensure_checkout_available(is_missing: bool, path: &str) -> Result<(), IpcError> {
    if is_missing {
        return Err(missing("checkout is no longer available"));
    }
    let metadata =
        fs::metadata(path).map_err(|error| filesystem_error("checkout is unavailable", error))?;
    if !metadata.is_dir() {
        return Err(missing("checkout is no longer available"));
    }
    Ok(())
}

fn too_large() -> IpcError {
    IpcError::new(
        IpcErrorCode::FileTooLarge,
        "file exceeds the 1 MiB text preview limit",
    )
}

fn binary_file() -> IpcError {
    IpcError::new(
        IpcErrorCode::BinaryFile,
        "file is binary or is not valid UTF-8",
    )
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use tempfile::tempdir;

    use crate::{domain::ipc::IpcErrorCode, persistence::Database, services::workspace};

    use super::{list, read};

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn db(temp: &Path) -> Database {
        Database::open(temp.join("workspace.sqlite3")).unwrap()
    }

    #[test]
    fn checkout_id_selects_only_its_registered_tree_and_rejects_unknown_ids() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("first.txt"), "one").unwrap();
        fs::write(second.join("second.txt"), "two").unwrap();
        let database = db(temp.path());
        let first_state = workspace::register_folder(&database, &first).unwrap();
        let second_state = workspace::register_folder(&database, &second).unwrap();

        let first_files = list(&database, &first_state.repos[0].checkouts[0].id, ".").unwrap();
        let second_files = list(&database, &second_state.repos[1].checkouts[0].id, ".").unwrap();

        assert_eq!(
            first_files
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["first.txt"]
        );
        assert_eq!(
            second_files
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["second.txt"]
        );
        assert!(list(&database, "checkout:unregistered", ".").is_err());
    }

    #[test]
    fn rejects_traversal_and_symlinks_that_escape_the_checkout() {
        #[cfg(unix)]
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        #[cfg(unix)]
        symlink(&outside, root.join("escape")).unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &root).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        let traversal =
            read(&database, checkout_id, Path::new("../outside/secret.txt")).unwrap_err();
        assert!(matches!(traversal.code, IpcErrorCode::InvalidPath));
        let git_metadata = read(&database, checkout_id, Path::new(".git/config")).unwrap_err();
        assert!(matches!(git_metadata.code, IpcErrorCode::InvalidPath));
        #[cfg(unix)]
        {
            let escape = read(&database, checkout_id, Path::new("escape/secret.txt")).unwrap_err();
            assert!(matches!(escape.code, IpcErrorCode::PathOutsideCheckout));
        }
    }

    #[test]
    fn gitignore_rules_are_applied_only_for_git_checkouts() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init"]);
        fs::write(root.join(".gitignore"), "ignored.txt\nignored-dir/\n").unwrap();
        fs::write(root.join("visible.txt"), "visible").unwrap();
        fs::write(root.join("ignored.txt"), "ignored").unwrap();
        fs::create_dir(root.join("ignored-dir")).unwrap();
        fs::write(root.join("ignored-dir/hidden.txt"), "ignored").unwrap();
        let database = db(temp.path());

        let state = workspace::register_folder(&database, &root).unwrap();
        let entries = list(&database, &state.repos[0].checkouts[0].id, ".")
            .unwrap()
            .entries;
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"visible.txt"));
        assert!(!names.contains(&"ignored.txt"));
        assert!(!names.contains(&"ignored-dir"));
        assert!(!names.contains(&".git"));
    }

    #[test]
    fn limits_text_reads_and_rejects_binary_content() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("plain");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("large.txt"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
        fs::write(root.join("binary.bin"), [0, 1, 2]).unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &root).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        let large = read(&database, checkout_id, Path::new("large.txt")).unwrap_err();
        let binary = read(&database, checkout_id, Path::new("binary.bin")).unwrap_err();

        assert!(matches!(large.code, IpcErrorCode::FileTooLarge));
        assert!(matches!(binary.code, IpcErrorCode::BinaryFile));
    }
}
