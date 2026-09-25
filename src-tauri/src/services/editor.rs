use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use serde::Serialize;

use crate::{
    domain::ipc::{IpcError, IpcErrorCode},
    persistence::Database,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorAvailability {
    pub zed: bool,
    pub neovim: bool,
}

#[derive(Debug, Clone)]
pub struct EditorTarget {
    pub path: PathBuf,
    pub is_file: bool,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

pub trait EditorAdapter {
    fn executable(&self) -> &'static str;
    fn arguments(&self, target: &EditorTarget) -> Vec<String>;
}

pub struct ZedAdapter;
pub struct NeovimAdapter;

impl EditorAdapter for ZedAdapter {
    fn executable(&self) -> &'static str {
        "zed"
    }

    fn arguments(&self, target: &EditorTarget) -> Vec<String> {
        let location = if target.is_file {
            target.line.map_or_else(
                || target.path.display().to_string(),
                |line| {
                    format!(
                        "{}:{line}:{}",
                        target.path.display(),
                        target.column.unwrap_or(1)
                    )
                },
            )
        } else {
            target.path.display().to_string()
        };
        vec![location]
    }
}

impl EditorAdapter for NeovimAdapter {
    fn executable(&self) -> &'static str {
        "nvim"
    }

    fn arguments(&self, target: &EditorTarget) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(line) = target.line {
            args.push(format!("+{line}"));
        }
        if target.is_file {
            args.push("--".into());
            args.push(target.path.to_string_lossy().into_owned());
        }
        args
    }
}

pub fn availability() -> EditorAvailability {
    EditorAvailability {
        zed: find_executable("zed").is_some(),
        neovim: find_executable("nvim").is_some(),
    }
}

pub fn resolve_target(
    database: &Database,
    checkout_id: &str,
    file_path: Option<&str>,
    line: Option<u32>,
    column: Option<u32>,
) -> Result<EditorTarget, IpcError> {
    if line.is_some_and(|line| line == 0) || column.is_some_and(|column| column == 0) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "editor line and column numbers must start at 1",
        ));
    }
    if column.is_some() && line.is_none() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "an editor column requires a line number",
        ));
    }
    if file_path.is_none() && line.is_some() {
        return Err(IpcError::new(
            IpcErrorCode::InvalidPath,
            "an editor location requires a file",
        ));
    }

    let workspace = database
        .load_workspace()
        .map_err(|error| IpcError::new(IpcErrorCode::OperationFailed, error))?;
    let repo = workspace
        .repos
        .iter()
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
    let checkout = repo
        .checkouts
        .iter()
        .find(|checkout| checkout.id == checkout_id)
        .expect("checkout was found in repo");
    if checkout.is_missing {
        return Err(IpcError::new(
            IpcErrorCode::FolderMissing,
            "checkout is no longer available",
        ));
    }

    let (path, is_file) = if let Some(file_path) = file_path {
        let relative_path = Path::new(file_path);
        if file_path.is_empty()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                ) || matches!(component, Component::Normal(name) if name == ".git")
            })
        {
            return Err(IpcError::new(
                IpcErrorCode::InvalidPath,
                "editor file path must be a relative checkout file and cannot access .git",
            ));
        }
        let path =
            crate::services::checkout::resolve_checkout_path(repo, checkout_id, relative_path)?;
        if !path.is_file() {
            return Err(IpcError::new(
                IpcErrorCode::InvalidPath,
                "editor target is not a file",
            ));
        }
        (path, true)
    } else {
        (
            crate::services::checkout::resolve_checkout_path(repo, checkout_id, Path::new("."))?,
            false,
        )
    };

    Ok(EditorTarget {
        path,
        is_file,
        line,
        column,
    })
}

pub fn open_in_zed(
    database: &Database,
    checkout_id: &str,
    file_path: Option<&str>,
    line: Option<u32>,
    column: Option<u32>,
) -> Result<(), IpcError> {
    let target = resolve_target(database, checkout_id, file_path, line, column)?;
    let adapter = ZedAdapter;
    launch(&adapter, &target)
}

pub fn nvim_launch_spec(target: Option<&EditorTarget>) -> Result<(PathBuf, Vec<String>), IpcError> {
    let adapter = NeovimAdapter;
    let executable = find_executable(adapter.executable()).ok_or_else(|| unavailable("Neovim"))?;
    Ok((
        executable,
        target.map_or_else(Vec::new, |target| adapter.arguments(target)),
    ))
}

fn launch(adapter: &dyn EditorAdapter, target: &EditorTarget) -> Result<(), IpcError> {
    let executable = find_executable(adapter.executable())
        .ok_or_else(|| unavailable(editor_name(adapter.executable())))?;
    Command::new(executable)
        .args(adapter.arguments(target))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::OperationFailed,
                format!("could not launch {}: {error}", adapter.executable()),
            )
        })
}

fn editor_name(executable: &str) -> &str {
    match executable {
        "zed" => "Zed",
        "nvim" => "Neovim",
        name => name,
    }
}

fn unavailable(name: &str) -> IpcError {
    IpcError::new(
        IpcErrorCode::OperationFailed,
        format!("{name} is unavailable. Install it and make its command available on PATH."),
    )
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let mut candidates = env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths)
                .map(|path| path.join(name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    candidates.extend(
        ["/opt/homebrew/bin", "/usr/local/bin"]
            .into_iter()
            .map(|directory| Path::new(directory).join(name)),
    );
    if name == "zed" {
        candidates.push(PathBuf::from("/Applications/Zed.app/Contents/MacOS/zed"));
    }
    candidates.into_iter().find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use crate::{domain::ipc::IpcErrorCode, persistence::Database, services::workspace};

    use super::{
        launch, resolve_target, unavailable, EditorAdapter, EditorTarget, NeovimAdapter, ZedAdapter,
    };

    fn register_checkout(path: &Path) -> (Database, String) {
        let database = Database::open(path.join("workspace.sqlite3")).unwrap();
        let state = workspace::register_folder(&database, path).unwrap();
        (database, state.repos[0].checkouts[0].id.clone())
    }

    #[test]
    fn editor_arguments_keep_metacharacters_inside_a_single_filename_argument() {
        let target = EditorTarget {
            path: Path::new("/checkout/$(touch nope); `echo bad` 'quoted'.txt").to_path_buf(),
            is_file: true,
            line: Some(7),
            column: Some(3),
        };
        let args = ZedAdapter.arguments(&target);

        assert_eq!(args.len(), 1);
        assert_eq!(
            args[0],
            "/checkout/$(touch nope); `echo bad` 'quoted'.txt:7:3"
        );
        assert_eq!(
            NeovimAdapter.arguments(&target),
            ["+7", "--", target.path.to_str().unwrap()]
        );
    }

    #[test]
    fn sec_03_newlines_and_shell_metacharacters_remain_inside_one_editor_argument() {
        let filename = "space 'quote' \"double\" ; $(touch no)
line.rs";
        let target = EditorTarget {
            path: Path::new("/checkout").join(filename),
            is_file: true,
            line: Some(17),
            column: None,
        };

        assert_eq!(
            ZedAdapter.arguments(&target),
            [format!("{}:17:1", target.path.display())]
        );
        assert_eq!(
            NeovimAdapter.arguments(&target),
            ["+17", "--", target.path.to_str().unwrap()]
        );
    }

    #[test]
    fn neovim_location_uses_the_requested_exact_line() {
        let target = EditorTarget {
            path: Path::new("/checkout/src/main.rs").to_path_buf(),
            is_file: true,
            line: Some(123),
            column: Some(1),
        };
        let args = NeovimAdapter.arguments(&target);

        assert_eq!(args, ["+123", "--", "/checkout/src/main.rs"]);
    }

    #[test]
    fn editor_availability_errors_are_actionable() {
        struct MissingEditor;
        impl EditorAdapter for MissingEditor {
            fn executable(&self) -> &'static str {
                "marvis-test-missing-editor-7f02"
            }

            fn arguments(&self, _target: &EditorTarget) -> Vec<String> {
                Vec::new()
            }
        }

        let target = EditorTarget {
            path: Path::new("/checkout").to_path_buf(),
            is_file: false,
            line: None,
            column: None,
        };
        let error = launch(&MissingEditor, &target).unwrap_err();
        assert!(error.message.contains("marvis-test-missing-editor"));
        assert!(error.message.contains("available on PATH"));
        assert!(unavailable("Neovim")
            .message
            .contains("Neovim is unavailable"));
    }

    #[test]
    fn target_resolution_validates_checkout_ids_paths_and_file_containment() {
        #[cfg(unix)]
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("$(touch nope); file.rs"), "hello").unwrap();
        fs::write(outside.join("secret.rs"), "secret").unwrap();
        #[cfg(unix)]
        symlink(outside.join("secret.rs"), root.join("escape.rs")).unwrap();
        let (database, checkout_id) = register_checkout(&root);

        let target = resolve_target(
            &database,
            &checkout_id,
            Some("$(touch nope); file.rs"),
            Some(9),
            Some(4),
        )
        .unwrap();
        assert_eq!(target.path.file_name().unwrap(), "$(touch nope); file.rs");
        assert!(!temp.path().join("nope").exists());
        assert!(matches!(
            resolve_target(&database, "checkout:unknown", None, None, None)
                .unwrap_err()
                .code,
            IpcErrorCode::InvalidCheckout
        ));
        assert!(matches!(
            resolve_target(
                &database,
                &checkout_id,
                Some("../outside/secret.rs"),
                None,
                None
            )
            .unwrap_err()
            .code,
            IpcErrorCode::InvalidPath
        ));
        #[cfg(unix)]
        assert!(matches!(
            resolve_target(&database, &checkout_id, Some("escape.rs"), None, None)
                .unwrap_err()
                .code,
            IpcErrorCode::PathOutsideCheckout
        ));
    }
}
