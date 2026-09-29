use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use crate::{
    domain::review::MAX_ROUND_PROMPT_BYTES,
    domain::{
        files::{CheckoutImage, FileContent, FileEntry, FileEntryKind, FileSearchResult, FileTree},
        ipc::{IpcError, IpcErrorCode},
        workspace::{Checkout, Repo, RepoKind},
    },
    persistence::Database,
};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 2000;
/// Git's own bookkeeping, never shown in the tree. `check-ignore` reports it ignored on every
/// repository, which would otherwise put it in the list the moment ignores became visible.
const GIT_DIRECTORY: &str = ".git";
const MAX_SEARCH_ENTRIES: usize = 50_000;
/// The one product choice behind `~/.marvis/tmp/code-reviews/`.
const REVIEW_ROOT_FROM_HOME: &str = ".marvis/tmp/code-reviews";

#[derive(Clone)]
pub struct ReviewRoot(pub PathBuf);

pub fn review_root(home: &Path) -> ReviewRoot {
    ReviewRoot(home.join(REVIEW_ROOT_FROM_HOME))
}

pub fn review_root_path(review_root: &Path) -> Result<String, IpcError> {
    fs::canonicalize(review_root)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| filesystem_error("could not resolve review export folder", error))
}

pub fn list(
    database: &Database,
    checkout_id: &str,
    relative_path: &str,
) -> Result<FileTree, IpcError> {
    let (repo, checkout) = registered_checkout(database, checkout_id)?;
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
            ignored: false,
        });
        if collected.len() > MAX_DIRECTORY_ENTRIES {
            truncated = true;
            collected.pop();
            break;
        }
    }

    if repo.kind == RepoKind::Git {
        // Gitignored files are listed and marked, not dropped: the tree shows them a step
        // quieter, so a build directory reads as present but uninteresting. `.git` is the
        // exception — `check-ignore` always reports it ignored because it is Git's own
        // bookkeeping, not something the project ignores, so it stays out of the tree
        // entirely, matched by name.
        let ignored = ignored_paths(root, &collected)?;
        for entry in &mut collected {
            entry.ignored = ignored.contains(entry.path.as_str());
        }
        collected.retain(|entry| entry.name != GIT_DIRECTORY);
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

pub fn search(database: &Database, checkout_id: &str) -> Result<FileSearchResult, IpcError> {
    let (repo, checkout) = registered_checkout(database, checkout_id)?;
    ensure_checkout_available(checkout.is_missing, &checkout.canonical_path)?;
    let root = Path::new(&checkout.canonical_path);
    let (mut entries, truncated) = if repo.kind == RepoKind::Git {
        search_git_files(root)?
    } else {
        search_plain_files(root)?
    };
    entries.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(FileSearchResult { entries, truncated })
}

fn search_git_files(root: &Path) -> Result<(Vec<FileEntry>, bool), IpcError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not search checkout files: {error}"),
            )
        })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);
    let mut entries = Vec::new();
    let mut truncated = false;
    loop {
        let mut record = Vec::new();
        let length = reader.read_until(0, &mut record).map_err(|error| {
            IpcError::new(
                IpcErrorCode::GitFailed,
                format!("could not read checkout file search results: {error}"),
            )
        })?;
        if length == 0 {
            break;
        }
        if entries.len() == MAX_SEARCH_ENTRIES {
            truncated = true;
            break;
        }
        let Some(path) = record
            .strip_suffix(&[0])
            .and_then(|path| std::str::from_utf8(path).ok())
        else {
            continue;
        };
        let relative = Path::new(path);
        if relative.components().any(
            |component| matches!(component, std::path::Component::Normal(name) if name == ".git"),
        ) {
            continue;
        }
        let full_path = root.join(relative);
        let Ok(metadata) = fs::symlink_metadata(&full_path) else {
            continue;
        };
        let kind = if metadata.file_type().is_symlink() {
            FileEntryKind::Symlink
        } else if metadata.is_file() {
            FileEntryKind::File
        } else {
            continue;
        };
        entries.push(FileEntry {
            name: relative
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned(),
            path: path.replace(std::path::MAIN_SEPARATOR, "/"),
            kind,
            ignored: false,
        });
    }
    if truncated {
        let _ = child.kill();
    }
    let output = child.wait_with_output().map_err(|error| {
        IpcError::new(
            IpcErrorCode::GitFailed,
            format!("could not finish checkout file search: {error}"),
        )
    })?;
    if !truncated && !output.status.success() {
        return Err(IpcError::new(
            IpcErrorCode::GitFailed,
            format!(
                "could not search checkout files: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok((entries, truncated))
}

fn search_plain_files(root: &Path) -> Result<(Vec<FileEntry>, bool), IpcError> {
    let mut pending = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(&directory)
            .map_err(|error| filesystem_error("could not search folder", error))?
        {
            let item =
                item.map_err(|error| filesystem_error("could not read folder entry", error))?;
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
            if file_type.is_dir() {
                pending.push(item.path());
                continue;
            }
            let kind = if file_type.is_symlink() {
                FileEntryKind::Symlink
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
            entries.push(FileEntry {
                name,
                path: path.replace(std::path::MAIN_SEPARATOR, "/"),
                kind,
                ignored: false,
            });
            if entries.len() > MAX_SEARCH_ENTRIES {
                entries.pop();
                return Ok((entries, true));
            }
        }
    }
    Ok((entries, false))
}

pub fn read(
    database: &Database,
    checkout_id: &str,
    origin: &str,
    relative_path: &Path,
    review_root: &Path,
) -> Result<FileContent, IpcError> {
    let (repo, checkout) = registered_checkout(database, checkout_id)?;
    let relative_path = relative_path
        .to_str()
        .ok_or_else(|| IpcError::new(IpcErrorCode::InvalidPath, "file path is not valid UTF-8"))?;
    let (relative_path, path) = match origin {
        "checkout" => {
            ensure_checkout_available(checkout.is_missing, &checkout.canonical_path)?;
            let relative_path = parse_relative_path(relative_path)?;
            let path = crate::services::checkout::resolve_checkout_path(
                &repo,
                checkout_id,
                &relative_path,
            )?;
            (relative_path, path)
        }
        "review" => {
            let relative_path = parse_review_relative_path(relative_path)?;
            let path = resolve_review_path(review_root, &relative_path)?;
            (relative_path, path)
        }
        _ => return Err(invalid_origin()),
    };
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

pub fn write(
    database: &Database,
    checkout_id: &str,
    origin: &str,
    relative_path: &Path,
    content: &str,
    expected_content: &str,
    review_root: &Path,
) -> Result<(), IpcError> {
    let (repo, checkout) = registered_checkout(database, checkout_id)?;
    if content.len() as u64 > MAX_FILE_BYTES {
        return Err(too_large());
    }
    if content.contains('\0') {
        return Err(binary_file());
    }
    let relative_path = relative_path
        .to_str()
        .ok_or_else(|| IpcError::new(IpcErrorCode::InvalidPath, "file path is not valid UTF-8"))?;
    let path = match origin {
        "checkout" => {
            ensure_checkout_available(checkout.is_missing, &checkout.canonical_path)?;
            let relative_path = parse_relative_path(relative_path)?;
            crate::services::checkout::resolve_checkout_path(&repo, checkout_id, &relative_path)?
        }
        "review" => {
            let relative_path = parse_review_relative_path(relative_path)?;
            resolve_review_path(review_root, &relative_path)?
        }
        _ => return Err(invalid_origin()),
    };
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
    // A read-only file is a decision someone made outside this app, and the temporary-file
    // replacement would quietly undo it: `rename` needs write access to the *directory*, not to
    // the file, so the save would succeed and the bit would be gone. Refuse instead.
    let permissions = metadata.permissions();
    if permissions.readonly() {
        return Err(IpcError::new(
            IpcErrorCode::PermissionDenied,
            "file is read-only",
        ));
    }

    let current =
        fs::read(&path).map_err(|error| filesystem_error("could not read file", error))?;
    if current.len() as u64 > MAX_FILE_BYTES {
        return Err(too_large());
    }
    if current.contains(&0) {
        return Err(binary_file());
    }
    let current = String::from_utf8(current).map_err(|_| binary_file())?;
    if current != expected_content {
        return Err(IpcError::new(
            IpcErrorCode::FileChanged,
            "file changed on disk; reload it before saving",
        ));
    }

    atomic_write(&path, content.as_bytes(), permissions)
}

pub fn export_review_markdown(
    review_root: &Path,
    date: &str,
    timestamp: &str,
    markdown: &str,
) -> Result<String, IpcError> {
    validate_review_timestamp(date, timestamp)?;
    if markdown.len() > MAX_ROUND_PROMPT_BYTES {
        return Err(too_large());
    }
    if markdown.contains('\0') {
        return Err(binary_file());
    }

    fs::create_dir_all(review_root)
        .map_err(|error| filesystem_error("could not create review export folder", error))?;
    let root = fs::canonicalize(review_root)
        .map_err(|error| filesystem_error("could not resolve review export folder", error))?;
    let stem = timestamp.to_string();
    for suffix in 1usize.. {
        let filename = if suffix == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem}-{suffix}.md")
        };
        let path = root.join(&filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(markdown.as_bytes()) {
                    let _ = fs::remove_file(&path);
                    return Err(filesystem_error("could not write review export", error));
                }
                return Ok(filename);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(filesystem_error("could not create review export", error)),
        }
    }
    unreachable!("the filename suffix range is unbounded")
}

fn validate_review_timestamp(date: &str, timestamp: &str) -> Result<(), IpcError> {
    let valid_date = date.len() == 10
        && date.as_bytes()[4] == b'-'
        && date.as_bytes()[7] == b'-'
        && date
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    let valid_timestamp = timestamp.len() == 15
        && timestamp.starts_with(date)
        && timestamp.as_bytes()[10] == b'-'
        && timestamp
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7 | 10) || byte.is_ascii_digit());
    if !valid_date || !valid_timestamp {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "review date and timestamp must use YYYY-MM-DD and YYYY-MM-DD-HHMM",
        ));
    }
    let year = date[0..4].parse::<u32>().unwrap();
    let month = date[5..7].parse::<u32>().unwrap();
    let day = date[8..10].parse::<u32>().unwrap();
    let hour = timestamp[11..13].parse::<u32>().unwrap();
    let minute = timestamp[13..15].parse::<u32>().unwrap();
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if day == 0 || day > days_in_month || hour > 23 || minute > 59 {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "review date or local time is out of range",
        ));
    }
    Ok(())
}

/// Writes through a sibling temporary file so readers never observe a partial UTF-8 document.
///
/// The replacement arrives carrying the mode of the file it replaces, because a temporary file is
/// created with the default one and `rename` does not reconcile it: without this a saved script
/// loses its executable bit and a private file gains a group-read it never had.
fn atomic_write(path: &Path, content: &[u8], permissions: fs::Permissions) -> Result<(), IpcError> {
    let parent = path.parent().ok_or_else(|| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            "selected file has no parent directory",
        )
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidPath,
                "selected file name is not valid UTF-8",
            )
        })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temporary = parent.join(format!(".{name}.marvis-{nonce}-{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| filesystem_error("could not create temporary file", error))?;
        file.write_all(content)
            .and_then(|_| file.sync_all())
            .map_err(|error| filesystem_error("could not write file", error))?;
        drop(file);
        fs::set_permissions(&temporary, permissions)
            .map_err(|error| filesystem_error("could not apply file permissions", error))?;
        fs::rename(&temporary, path)
            .map_err(|error| filesystem_error("could not replace file", error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn read_markdown_image(
    database: &Database,
    checkout_id: &str,
    markdown_path: &str,
    image_path: &str,
) -> Result<CheckoutImage, IpcError> {
    let (repo, checkout) = registered_checkout(database, checkout_id)?;
    ensure_checkout_available(checkout.is_missing, &checkout.canonical_path)?;
    let markdown_path = parse_relative_path(markdown_path)?;
    let markdown_file =
        crate::services::checkout::resolve_checkout_path(&repo, checkout_id, &markdown_path)?;
    if !fs::metadata(&markdown_file)
        .map_err(|error| filesystem_error("could not inspect Markdown file", error))?
        .is_file()
    {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "Markdown path is not a file",
        ));
    }

    let root = Path::new(&checkout.canonical_path);
    let markdown_directory = markdown_file.parent().ok_or_else(|| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            "Markdown file has no parent folder",
        )
    })?;
    let relative_image = normalize_markdown_image_path(root, markdown_directory, image_path)?;
    let image =
        crate::services::checkout::resolve_checkout_path(&repo, checkout_id, &relative_image)?;
    let metadata =
        fs::metadata(&image).map_err(|error| filesystem_error("could not inspect image", error))?;
    if !metadata.is_file() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "Markdown image path is not a file",
        ));
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(image_too_large());
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&image)
        .and_then(|file| file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| filesystem_error("could not read image", error))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(image_too_large());
    }
    let mime_type = image_mime_type(&bytes).ok_or_else(|| {
        IpcError::new(
            IpcErrorCode::BinaryFile,
            "Markdown image must be PNG, JPEG, GIF, or WebP",
        )
    })?;

    Ok(CheckoutImage {
        mime_type: mime_type.to_owned(),
        data_base64: BASE64.encode(&bytes),
        size_bytes: bytes.len(),
    })
}

fn normalize_markdown_image_path(
    root: &Path,
    markdown_directory: &Path,
    image_path: &str,
) -> Result<PathBuf, IpcError> {
    let image_path = image_path.split(['?', '#']).next().unwrap_or_default();
    let image_path = decode_markdown_image_url_path(image_path)?;
    if image_path.is_empty()
        || image_path.starts_with(['/', '\\'])
        || image_path.contains('\\')
        || image_path
            .split('/')
            .next()
            .is_some_and(|first_component| first_component.contains(':'))
    {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "Markdown image URL must be a relative checkout path",
        ));
    }

    let mut normalized = markdown_directory
        .strip_prefix(root)
        .map_err(|_| {
            IpcError::new(
                IpcErrorCode::PathOutsideCheckout,
                "Markdown file is outside the checkout",
            )
        })?
        .to_path_buf();
    for component in Path::new(&image_path).components() {
        match component {
            std::path::Component::Normal(name) if name == ".git" => {
                return Err(IpcError::new(
                    IpcErrorCode::InvalidPath,
                    "Markdown images cannot access .git",
                ));
            }
            std::path::Component::Normal(name) => normalized.push(name),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if normalized.pop() => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(IpcError::new(
                    IpcErrorCode::InvalidPath,
                    "Markdown image path escapes the checkout",
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "Markdown image path is empty",
        ));
    }
    Ok(normalized)
}

fn decode_markdown_image_url_path(path: &str) -> Result<String, IpcError> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = bytes.get(index + 1..index + 3).ok_or_else(|| {
                IpcError::new(
                    IpcErrorCode::InvalidPath,
                    "Markdown image URL has invalid encoding",
                )
            })?;
            let hex = std::str::from_utf8(pair).map_err(|_| {
                IpcError::new(
                    IpcErrorCode::InvalidPath,
                    "Markdown image URL has invalid encoding",
                )
            })?;
            let value = u8::from_str_radix(hex, 16).map_err(|_| {
                IpcError::new(
                    IpcErrorCode::InvalidPath,
                    "Markdown image URL has invalid encoding",
                )
            })?;
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            "Markdown image URL is not valid UTF-8",
        )
    })?;
    if decoded.contains('\0') {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "Markdown image URL contains an invalid character",
        ));
    }
    Ok(decoded)
}

fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn registered_checkout(
    database: &Database,
    checkout_id: &str,
) -> Result<(Repo, Checkout), IpcError> {
    let workspace = database
        .load_workspace()
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?;
    let (repo, checkout) = crate::services::checkout::registered_checkout(
        &workspace.repos,
        checkout_id,
        "checkout ID is not registered",
    )?;
    Ok((repo.clone(), checkout.clone()))
}

fn parse_relative_path(path: &str) -> Result<PathBuf, IpcError> {
    if path.is_empty() || path.contains('\0') {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "path must be relative to the checkout and cannot contain NUL",
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

fn parse_review_relative_path(path: &str) -> Result<PathBuf, IpcError> {
    let parsed = Path::new(path);
    if path.contains('\0')
        || path.contains('\\')
        || parsed.is_absolute()
        || parsed
            .components()
            .any(|component| matches!(component, std::path::Component::Prefix(_)))
    {
        return Err(IpcError::new(
            IpcErrorCode::PathOutsideCheckout,
            "review path must stay inside the review folder",
        ));
    }
    if path
        .split('/')
        .any(|part| part == ".." || part == "." || part.is_empty())
    {
        return Err(IpcError::new(
            IpcErrorCode::PathOutsideCheckout,
            "review path must stay inside the review folder",
        ));
    }
    if path.is_empty() || path.len() > 4096 || !path.ends_with(".md") {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "review path must be a relative Markdown file",
        ));
    }
    Ok(parsed.to_path_buf())
}

fn resolve_review_path(root: &Path, relative_path: &Path) -> Result<PathBuf, IpcError> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| filesystem_error("could not resolve review export folder", error))?;
    let canonical_path = fs::canonicalize(root.join(relative_path))
        .map_err(|error| filesystem_error("could not resolve review file", error))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(IpcError::new(
            IpcErrorCode::PathOutsideCheckout,
            "selected path resolves outside the review folder",
        ));
    }
    Ok(canonical_path)
}

fn invalid_origin() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidPath, "file origin is not supported")
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

fn image_too_large() -> IpcError {
    IpcError::new(
        IpcErrorCode::FileTooLarge,
        "image exceeds the 2 MiB Markdown preview limit",
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

    use crate::{
        domain::{files::FileEntry, ipc::IpcErrorCode},
        persistence::Database,
        services::workspace,
    };

    use super::{
        export_review_markdown, list, read, read_markdown_image, search, write, FileContent,
        IpcError, MAX_ROUND_PROMPT_BYTES,
    };

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

    fn read_checkout(
        database: &Database,
        checkout_id: &str,
        path: &str,
    ) -> Result<FileContent, IpcError> {
        read(
            database,
            checkout_id,
            "checkout",
            Path::new(path),
            Path::new(""),
        )
    }

    fn write_checkout(
        database: &Database,
        checkout_id: &str,
        path: &str,
        content: &str,
        expected_content: &str,
    ) -> Result<(), IpcError> {
        write(
            database,
            checkout_id,
            "checkout",
            Path::new(path),
            content,
            expected_content,
            Path::new(""),
        )
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
    fn sec_01_02_rejects_path_traversal_and_symlinks_that_escape_the_checkout() {
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

        let traversal = read_checkout(&database, checkout_id, "../outside/secret.txt").unwrap_err();
        assert!(matches!(traversal.code, IpcErrorCode::InvalidPath));
        let git_metadata = read_checkout(&database, checkout_id, ".git/config").unwrap_err();
        assert!(matches!(git_metadata.code, IpcErrorCode::InvalidPath));
        #[cfg(unix)]
        {
            let escape = read_checkout(&database, checkout_id, "escape/secret.txt").unwrap_err();
            assert!(matches!(escape.code, IpcErrorCode::PathOutsideCheckout));
        }
    }

    #[test]
    fn sec_11_a_review_origin_reads_only_the_review_root() {
        #[cfg(unix)]
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let checkout = temp.path().join("checkout");
        let review_root = temp.path().join("reviews");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&review_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(checkout.join("checkout-only.md"), "checkout").unwrap();
        fs::write(outside.join("secret.md"), "secret").unwrap();
        #[cfg(unix)]
        symlink(outside.join("secret.md"), review_root.join("escape.md")).unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &checkout).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        let checkout_file = read(
            &database,
            checkout_id,
            "review",
            Path::new("checkout-only.md"),
            &review_root,
        )
        .unwrap_err();
        assert!(matches!(checkout_file.code, IpcErrorCode::FolderMissing));
        let traversal = read(
            &database,
            checkout_id,
            "review",
            Path::new("../checkout/checkout-only.md"),
            &review_root,
        )
        .unwrap_err();
        assert!(matches!(traversal.code, IpcErrorCode::PathOutsideCheckout));
        #[cfg(unix)]
        {
            let escape = read(
                &database,
                checkout_id,
                "review",
                Path::new("escape.md"),
                &review_root,
            )
            .unwrap_err();
            assert!(matches!(escape.code, IpcErrorCode::PathOutsideCheckout));
        }
    }

    #[test]
    fn sec_12_a_review_origin_refuses_to_write_anything_but_markdown() {
        let temp = tempdir().unwrap();
        let checkout = temp.path().join("checkout");
        let review_root = temp.path().join("reviews");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&review_root).unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &checkout).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        for path in ["notes.txt", "notes"] {
            let error = write(
                &database,
                checkout_id,
                "review",
                Path::new(path),
                "changed",
                "before",
                &review_root,
            )
            .unwrap_err();
            assert!(matches!(error.code, IpcErrorCode::InvalidPath));
        }
    }

    #[test]
    fn a_review_origin_never_reaches_outside_a_registered_checkout() {
        let temp = tempdir().unwrap();
        let checkout = temp.path().join("checkout");
        fs::create_dir_all(&checkout).unwrap();
        fs::write(checkout.join("notes.txt"), "safe").unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &checkout).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        let error = write_checkout(
            &database,
            checkout_id,
            "../outside/secret.txt",
            "changed",
            "secret",
        )
        .unwrap_err();
        assert!(matches!(error.code, IpcErrorCode::InvalidPath));
        assert_eq!(
            fs::read_to_string(checkout.join("notes.txt")).unwrap(),
            "safe"
        );
    }

    #[test]
    fn two_exports_in_the_same_minute_do_not_overwrite_each_other() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("reviews");
        let first =
            export_review_markdown(&root, "2026-03-14", "2026-03-14-1532", "first").unwrap();
        let first_bytes = fs::read(root.join(&first)).unwrap();
        let second =
            export_review_markdown(&root, "2026-03-14", "2026-03-14-1532", "second").unwrap();

        assert_eq!(first, "2026-03-14-1532.md");
        assert_eq!(second, "2026-03-14-1532-2.md");
        assert_eq!(fs::read(root.join(first)).unwrap(), first_bytes);
        assert_eq!(fs::read_to_string(root.join(second)).unwrap(), "second");
    }

    #[test]
    fn a_review_export_with_a_malformed_date_or_timestamp_is_refused() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("reviews");
        let too_long = "x".repeat(4097);
        let invalid = [
            ("2026-3-14", "2026-03-14-1532"),
            ("2026-03-14", "2026-03-14-999"),
            ("../etc/passwd", "2026-03-14-1532"),
            (too_long.as_str(), "2026-03-14-1532"),
        ];
        for (date, timestamp) in invalid {
            let error = export_review_markdown(&root, date, timestamp, "review").unwrap_err();
            assert!(matches!(error.code, IpcErrorCode::InvalidPath));
            assert!(!root.exists());
        }
        assert!(
            export_review_markdown(
                &root,
                "2026-03-14",
                "2026-03-14-1532",
                &"x".repeat(MAX_ROUND_PROMPT_BYTES + 1),
            )
            .unwrap_err()
            .code
                == IpcErrorCode::FileTooLarge
        );
        assert!(!root.exists());
    }

    #[test]
    fn gitignored_files_are_listed_and_marked_in_git_checkouts_only() {
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
        let ignored = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.ignored)
        };

        // Ignored files stay in the tree so it can show them a step quieter, and the flag says
        // which ones those are. `.git` is the one thing never listed.
        assert!(names(&entries).contains(&"visible.txt"));
        assert!(!names(&entries).contains(&".git"));
        assert_eq!(ignored("visible.txt"), Some(false));
        assert_eq!(ignored("ignored.txt"), Some(true));
        assert_eq!(ignored("ignored-dir"), Some(true));
    }

    #[test]
    fn a_plain_folder_marks_nothing_as_ignored() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("plain");
        fs::create_dir_all(&root).unwrap();
        // A .gitignore in a folder that is not a checkout is just a file: nothing honors it.
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("keep.log"), "kept").unwrap();
        let database = db(temp.path());

        let state = workspace::register_folder(&database, &root).unwrap();
        let entries = list(&database, &state.repos[0].checkouts[0].id, ".")
            .unwrap()
            .entries;

        assert!(entries.iter().all(|entry| !entry.ignored));
        assert!(names(&entries).contains(&"keep.log"));
        // And `.git` is a plain directory here, with nothing to hide it.
        assert!(!names(&entries).contains(&".git"));
    }

    fn names(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
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

        let large = read_checkout(&database, checkout_id, "large.txt").unwrap_err();
        let binary = read_checkout(&database, checkout_id, "binary.bin").unwrap_err();

        assert!(matches!(large.code, IpcErrorCode::FileTooLarge));
        assert!(matches!(binary.code, IpcErrorCode::BinaryFile));
    }

    #[test]
    fn writes_text_only_when_the_expected_content_still_matches() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("plain");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.txt"), "before").unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &root).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        write_checkout(&database, checkout_id, "notes.txt", "after", "before").unwrap();
        assert_eq!(fs::read_to_string(root.join("notes.txt")).unwrap(), "after");

        fs::write(root.join("notes.txt"), "external change").unwrap();
        let changed = write_checkout(&database, checkout_id, "notes.txt", "another edit", "after")
            .unwrap_err();
        assert!(matches!(changed.code, IpcErrorCode::FileChanged));
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "external change"
        );
    }

    #[test]
    fn rejects_unsafe_file_writes() {
        #[cfg(unix)]
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("notes.txt"), "safe").unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        #[cfg(unix)]
        symlink(&outside, root.join("escape")).unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &root).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        assert!(matches!(
            write_checkout(
                &database,
                checkout_id,
                "../outside/secret.txt",
                "x",
                "secret"
            )
            .unwrap_err()
            .code,
            IpcErrorCode::InvalidPath
        ));
        assert!(matches!(
            write_checkout(&database, checkout_id, "notes.txt", "bad\0text", "safe")
                .unwrap_err()
                .code,
            IpcErrorCode::BinaryFile
        ));
        #[cfg(unix)]
        assert!(matches!(
            write_checkout(
                &database,
                checkout_id,
                "escape/secret.txt",
                "changed",
                "secret"
            )
            .unwrap_err()
            .code,
            IpcErrorCode::PathOutsideCheckout
        ));
    }

    #[test]
    fn a_save_keeps_the_files_permissions_and_refuses_a_read_only_one() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("script.sh"), "echo one\n").unwrap();
        fs::write(root.join("locked.txt"), "locked\n").unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &root).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;

        // A read-only file is refused rather than quietly made writable by the replacement, and
        // refusing it leaves the bytes on disk untouched.
        let mut locked = fs::metadata(root.join("locked.txt")).unwrap().permissions();
        locked.set_readonly(true);
        fs::set_permissions(root.join("locked.txt"), locked).unwrap();
        assert!(matches!(
            write_checkout(
                &database,
                checkout_id,
                "locked.txt",
                "changed\n",
                "locked\n"
            )
            .unwrap_err()
            .code,
            IpcErrorCode::PermissionDenied
        ));
        assert_eq!(
            fs::read_to_string(root.join("locked.txt")).unwrap(),
            "locked\n"
        );

        // And a file that is writable keeps the mode it arrived with, so saving a script does not
        // quietly make it unrunnable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(root.join("script.sh"), fs::Permissions::from_mode(0o755)).unwrap();
            write_checkout(
                &database,
                checkout_id,
                "script.sh",
                "echo two\n",
                "echo one\n",
            )
            .unwrap();
            let mode = fs::metadata(root.join("script.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
            assert_eq!(
                fs::read_to_string(root.join("script.sh")).unwrap(),
                "echo two\n"
            );
        }
    }

    #[test]
    fn markdown_images_are_contained_raster_files_with_strict_limits() {
        #[cfg(unix)]
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let outside = temp.path().join("outside");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("images")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("docs/readme.md"), "![image](../images/pixel.png)").unwrap();
        fs::write(
            root.join("images/pixel.png"),
            b"\x89PNG\r\n\x1a\nimage-data",
        )
        .unwrap();
        fs::write(
            root.join("images/pixel photo.png"),
            b"\x89PNG\r\n\x1a\nimage-data",
        )
        .unwrap();
        fs::write(
            root.join("images/vector.svg"),
            "<svg onload='alert(1)'></svg>",
        )
        .unwrap();
        fs::write(
            root.join("images/large.png"),
            [
                b"\x89PNG\r\n\x1a\n".as_slice(),
                &vec![0; 2 * 1024 * 1024][..],
            ]
            .concat(),
        )
        .unwrap();
        fs::write(outside.join("secret.png"), b"\x89PNG\r\n\x1a\nsecret").unwrap();
        #[cfg(unix)]
        symlink(outside.join("secret.png"), root.join("images/escape.png")).unwrap();

        let database = db(temp.path());
        let state = workspace::register_folder(&database, &root).unwrap();
        let checkout_id = &state.repos[0].checkouts[0].id;
        let image = read_markdown_image(
            &database,
            checkout_id,
            "docs/readme.md",
            "../images/pixel.png",
        )
        .unwrap();
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.size_bytes, 18);
        assert!(!image.data_base64.is_empty());
        assert!(read_markdown_image(
            &database,
            checkout_id,
            "docs/readme.md",
            "../images/pixel%20photo.png",
        )
        .is_ok());

        let script_url = read_markdown_image(
            &database,
            checkout_id,
            "docs/readme.md",
            "javascript:alert(1)",
        )
        .unwrap_err();
        assert!(matches!(script_url.code, IpcErrorCode::InvalidPath));

        let unsupported = read_markdown_image(
            &database,
            checkout_id,
            "docs/readme.md",
            "../images/vector.svg",
        )
        .unwrap_err();
        assert!(matches!(unsupported.code, IpcErrorCode::BinaryFile));

        let too_large = read_markdown_image(
            &database,
            checkout_id,
            "docs/readme.md",
            "../images/large.png",
        )
        .unwrap_err();
        assert!(matches!(too_large.code, IpcErrorCode::FileTooLarge));

        #[cfg(unix)]
        {
            let escaped = read_markdown_image(
                &database,
                checkout_id,
                "docs/readme.md",
                "../images/escape.png",
            )
            .unwrap_err();
            assert!(matches!(escaped.code, IpcErrorCode::PathOutsideCheckout));
        }
    }

    #[test]
    fn file_search_includes_git_files_and_untracked_binary_files_but_honors_ignores() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-b", "trunk"]);
        fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
        fs::write(root.join(".gitignore"), "ignored.log\n").unwrap();
        git(&root, &["add", "tracked.txt", ".gitignore"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "base",
            ],
        );
        fs::write(root.join("new binary.dat"), [0, 1, 2]).unwrap();
        fs::write(root.join("ignored.log"), "ignore me").unwrap();
        let database = db(temp.path());
        let state = workspace::register_folder(&database, &root).unwrap();
        let result = search(&database, &state.repos[0].checkouts[0].id).unwrap();
        let paths: Vec<_> = result
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        assert!(paths.contains(&"tracked.txt"));
        assert!(paths.contains(&"new binary.dat"));
        assert!(!paths.contains(&"ignored.log"));
        assert!(!paths.iter().any(|path| path.contains(".git/")));
        assert!(!result.truncated);
    }
}
