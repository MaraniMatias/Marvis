//! `~/.marvis/config.yml`: the preferences the Settings dialog owns.
//!
//! One file, in the user's home, written by the dialog and read at launch. It is not the
//! database: the database holds state that belongs to a workspace (which checkouts exist, how wide
//! a pane is), and this holds the choices a person makes once and forgets they made.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    domain::ipc::{IpcError, IpcErrorCode},
    services::files::atomic_write,
};

const CONFIG_DIR_FROM_HOME: &str = ".marvis";
const CONFIG_FILE_NAME: &str = "config.yml";

const UI_FONT_SIZE_MIN: f64 = 11.0;
const UI_FONT_SIZE_MAX: f64 = 20.0;
const TERMINAL_FONT_SIZE_MIN: f64 = 9.0;
const TERMINAL_FONT_SIZE_MAX: f64 = 32.0;
const EDITOR_FONT_SIZE_MIN: f64 = 9.0;
const EDITOR_FONT_SIZE_MAX: f64 = 32.0;
const INDENTATION_SIZE_MIN: u32 = 1;
const INDENTATION_SIZE_MAX: u32 = 8;
/// The ends of the zoom steps the renderer walks. Kept here so a factor typed into the file
/// cannot come back as a scale no step in `src/domain/zoom.ts` names.
const ZOOM_MIN: f64 = 0.8;
const ZOOM_MAX: f64 = 1.5;

#[derive(Clone)]
pub struct ConfigFile(pub PathBuf);

pub fn config_file(home: &Path) -> ConfigFile {
    ConfigFile(home.join(CONFIG_DIR_FROM_HOME).join(CONFIG_FILE_NAME))
}

/// Every field defaults, unlike the rows in the database: this file is written by a person as
/// well as by the dialog, and a line they deleted is a preference they did not set, not a shape
/// this build cannot read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub ui: UiSettings,
    pub terminal: TerminalSettings,
    pub editor: EditorSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            ui: UiSettings::default(),
            terminal: TerminalSettings::default(),
            editor: EditorSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct UiSettings {
    pub font_size: f64,
    pub zoom: f64,
    pub theme: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            zoom: 1.0,
            theme: "system".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TerminalSettings {
    pub font_size: f64,
    pub ligatures: bool,
    pub cursor_blink: bool,
    pub scrollbar: String,
    /// Whether moving a terminal to another worktree also moves the shell's working directory.
    pub change_directory_on_move: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            ligatures: true,
            cursor_blink: true,
            scrollbar: "hidden".into(),
            change_directory_on_move: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditorSettings {
    pub font_size: f64,
    pub ligatures: bool,
    pub cursor_blink: bool,
    pub indentation: IndentationSettings,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            ligatures: true,
            cursor_blink: true,
            indentation: IndentationSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct IndentationSettings {
    pub use_spaces: bool,
    pub size: u32,
}

impl Default for IndentationSettings {
    fn default() -> Self {
        Self {
            use_spaces: true,
            size: 2,
        }
    }
}

impl AppSettings {
    /// Clamp what a hand-edited file asked for. Sizes are read-only for the shell, so a value
    /// outside the range is not drawn at that size, and a scrollbar mode outside the three the
    /// terminal knows is not a mode at all.
    fn normalized(mut self) -> Self {
        self.ui.font_size = bounded(self.ui.font_size, UI_FONT_SIZE_MIN, UI_FONT_SIZE_MAX, 14.0);
        self.ui.zoom = bounded(self.ui.zoom, ZOOM_MIN, ZOOM_MAX, 1.0);
        if !matches!(self.ui.theme.as_str(), "system" | "light" | "dark") {
            self.ui.theme = "system".into();
        }
        self.terminal.font_size = bounded(
            self.terminal.font_size,
            TERMINAL_FONT_SIZE_MIN,
            TERMINAL_FONT_SIZE_MAX,
            16.0,
        );
        if !matches!(
            self.terminal.scrollbar.as_str(),
            "hidden" | "auto" | "always"
        ) {
            self.terminal.scrollbar = "hidden".into();
        }
        self.editor.font_size = bounded(
            self.editor.font_size,
            EDITOR_FONT_SIZE_MIN,
            EDITOR_FONT_SIZE_MAX,
            13.0,
        );
        self.editor.indentation.size = self
            .editor
            .indentation
            .size
            .clamp(INDENTATION_SIZE_MIN, INDENTATION_SIZE_MAX);
        self
    }
}

/// A file the user typed into also carries `.nan` and `1e999`, which no font size can be, so
/// anything outside the range comes back as the default rather than reaching the renderer.
fn bounded(value: f64, min: f64, max: f64, default: f64) -> f64 {
    if !value.is_finite() {
        return default;
    }
    value.clamp(min, max)
}

/// Reads the file, or the defaults when there is not one yet.
///
/// A file that is there and does not parse is an error and not a fallback: the dialog says so and
/// the file stays exactly as it is, because the alternative is a Apply that overwrites a typo with
/// a page of defaults.
pub fn load(path: &Path) -> Result<AppSettings, IpcError> {
    let Some(document) = read_document(path)? else {
        return Ok(AppSettings::default());
    };
    let settings: AppSettings =
        serde_yaml::from_str(&document).map_err(|error| parse_error(&error.to_string()))?;
    Ok(settings.normalized())
}

/// The file's text, or `None` when there is nothing to read: no file yet, or one that is empty
/// because a person opened it, deleted everything and saved. Both are a person who has not set
/// anything, not a file that is broken.
fn read_document(path: &Path) -> Result<Option<String>, IpcError> {
    match fs::read_to_string(path) {
        Ok(document) if document.trim().is_empty() => Ok(None),
        Ok(document) => Ok(Some(document)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("could not read the settings file", error)),
    }
}

/// Writes the file, and refuses to write over one that cannot be read.
///
/// The refusal is the point of this function. A file somebody edited by hand and got wrong is the
/// only copy of the preferences they had, and an Apply — or a zoom step on the keyboard, which
/// writes the same file — would replace a typo they could have found with a page of defaults they
/// never asked for. So the file is parsed first: it either says what it says, or it is left alone
/// and the error names the line.
pub fn save(path: &Path, settings: &AppSettings) -> Result<(), IpcError> {
    if let Some(document) = read_document(path)? {
        serde_yaml::from_str::<AppSettings>(&document).map_err(|error| {
            parse_error(&format!(
                "{} was left as it is because it could not be read: {error}",
                path.display()
            ))
        })?;
    }
    let settings = settings.clone().normalized();
    let document = serde_yaml::to_string(&settings)
        .map_err(|error| parse_error(&format!("the settings could not be written: {error}")))?;
    let parent = path.parent().ok_or_else(|| {
        IpcError::new(
            IpcErrorCode::InvalidPath,
            "the settings path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| io_error("could not create the settings folder", error))?;
    atomic_write(path, document.as_bytes(), permissions_for(path))
}

/// The mode the file is written with. One the user never chose is private to them; one they did
/// is left alone, because a save is not a reason to undo a decision made outside this app.
fn permissions_for(path: &Path) -> fs::Permissions {
    if let Ok(metadata) = fs::metadata(path) {
        return metadata.permissions();
    }
    new_file_permissions()
}

#[cfg(unix)]
fn new_file_permissions() -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    fs::Permissions::from_mode(0o600)
}

#[cfg(not(unix))]
fn new_file_permissions() -> fs::Permissions {
    fs::Permissions::default()
}

fn io_error(action: &str, error: std::io::Error) -> IpcError {
    IpcError::new(IpcErrorCode::OperationFailed, format!("{action}: {error}"))
}

fn parse_error(message: &str) -> IpcError {
    IpcError::new(IpcErrorCode::InvalidPath, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Where `~/.marvis/config.yml` lands inside a temporary home, folder created: a test that
    /// writes the file by hand is standing in for a person who already has the folder.
    fn config_path_in(home: &tempfile::TempDir) -> PathBuf {
        let path = home
            .path()
            .join(CONFIG_DIR_FROM_HOME)
            .join(CONFIG_FILE_NAME);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }

    #[test]
    fn a_missing_file_reads_as_the_defaults_and_a_save_creates_the_folder() {
        let home = settings_dir();
        let path = home
            .path()
            .join(CONFIG_DIR_FROM_HOME)
            .join(CONFIG_FILE_NAME);
        assert_eq!(load(&path).unwrap(), AppSettings::default());
        save(&path, &AppSettings::default()).unwrap();
        assert_eq!(load(&path).unwrap(), AppSettings::default());
        assert!(path.is_file());
    }

    #[test]
    fn a_save_that_writes_over_a_missing_file_creates_the_folder_first() {
        let home = settings_dir();
        let path = home
            .path()
            .join(CONFIG_DIR_FROM_HOME)
            .join(CONFIG_FILE_NAME);
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn what_is_saved_is_what_is_read_back() {
        let home = settings_dir();
        let path = home
            .path()
            .join(CONFIG_DIR_FROM_HOME)
            .join(CONFIG_FILE_NAME);
        let written = AppSettings {
            ui: UiSettings {
                font_size: 16.0,
                zoom: 1.2,
                theme: "light".into(),
            },
            terminal: TerminalSettings {
                font_size: 18.0,
                ligatures: false,
                cursor_blink: false,
                scrollbar: "always".into(),
                change_directory_on_move: true,
            },
            editor: EditorSettings {
                font_size: 15.0,
                ligatures: false,
                cursor_blink: false,
                indentation: IndentationSettings {
                    use_spaces: false,
                    size: 4,
                },
            },
        };
        save(&path, &written).unwrap();
        assert_eq!(load(&path).unwrap(), written);
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_the_lines_it_leaves_out() {
        let home = settings_dir();
        let path = config_path_in(&home);
        std::fs::write(&path, "ui:\n  fontSize: 18\n").unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.ui.font_size, 18.0);
        assert_eq!(loaded.terminal, TerminalSettings::default());
        assert_eq!(loaded.editor, EditorSettings::default());
    }

    #[test]
    fn a_file_that_does_not_parse_is_an_error_and_is_left_where_it_is() {
        let home = settings_dir();
        let path = config_path_in(&home);
        let broken = "ui: [this is not\n";
        std::fs::write(&path, broken).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn a_save_refuses_to_write_over_a_file_it_cannot_read() {
        // The keyboard zoom writes this same file, so "save over a typo" is not a dialog-only
        // mistake: a scale step would destroy a hand-edited config nobody was looking at.
        let home = settings_dir();
        let path = config_path_in(&home);
        let broken = "ui: [this is not\n";
        std::fs::write(&path, broken).unwrap();

        let error = save(&path, &AppSettings::default()).unwrap_err().message;

        assert!(error.contains("left as it is"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn a_fractional_indentation_size_is_rejected_and_not_overwritten() {
        let home = settings_dir();
        let path = config_path_in(&home);
        let invalid = "editor:\n  indentation:\n    size: 2.5\n";
        std::fs::write(&path, invalid).unwrap();

        assert!(load(&path).is_err());
        assert!(save(&path, &AppSettings::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
    }

    #[test]
    fn a_save_over_a_file_with_lines_missing_from_it_is_allowed() {
        // A partial file is readable and is a person who set two preferences, not a broken one.
        let home = settings_dir();
        let path = config_path_in(&home);
        std::fs::write(&path, "editor:\n  indentation:\n    size: 4\n").unwrap();

        save(&path, &AppSettings::default()).unwrap();

        assert_eq!(load(&path).unwrap(), AppSettings::default());
    }

    #[test]
    fn a_save_over_an_empty_file_is_allowed() {
        let home = settings_dir();
        let path = config_path_in(&home);
        std::fs::write(&path, "  \n").unwrap();

        save(&path, &AppSettings::default()).unwrap();

        assert_eq!(load(&path).unwrap(), AppSettings::default());
    }

    #[test]
    fn sizes_outside_the_range_and_scrollbar_modes_that_are_not_modes_come_back_bounded() {
        let home = settings_dir();
        let path = config_path_in(&home);
        std::fs::write(
            &path,
            concat!(
                "ui:\n  fontSize: .nan\n  zoom: 99\n  theme: solarized\n",
                "terminal:\n  fontSize: 2\n  scrollbar: sometimes\n",
                "editor:\n  fontSize: 400\n  indentation:\n    size: 0\n",
            ),
        )
        .unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.ui.font_size, 14.0);
        assert_eq!(loaded.ui.zoom, ZOOM_MAX);
        assert_eq!(loaded.ui.theme, "system");
        assert_eq!(loaded.terminal.font_size, TERMINAL_FONT_SIZE_MIN);
        assert_eq!(loaded.terminal.scrollbar, "hidden");
        assert_eq!(loaded.editor.font_size, EDITOR_FONT_SIZE_MAX);
        assert_eq!(loaded.editor.indentation.size, INDENTATION_SIZE_MIN);
    }

    #[cfg(unix)]
    #[test]
    fn a_save_keeps_the_mode_the_file_already_had() {
        use std::os::unix::fs::PermissionsExt;
        let home = settings_dir();
        let path = home
            .path()
            .join(CONFIG_DIR_FROM_HOME)
            .join(CONFIG_FILE_NAME);
        save(&path, &AppSettings::default()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        save(
            &path,
            &AppSettings {
                ui: UiSettings {
                    font_size: 15.0,
                    zoom: 1.0,
                    theme: "dark".into(),
                },
                ..AppSettings::default()
            },
        )
        .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }
}
