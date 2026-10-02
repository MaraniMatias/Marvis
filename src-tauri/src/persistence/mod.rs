use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::domain::review::{ReviewNote, ReviewRound};
use crate::domain::terminal_layout::CheckoutTerminalLayout;
use crate::domain::workspace::{
    ArchivedCheckout, Checkout, RecentPath, Repo, RepoKind, Session, SessionStatus, SessionType,
    WorkspaceState,
};

const SCHEMA_VERSION: i64 = 11;
const REVIEW_NOTE_COLUMNS: &str = "id, checkout_id, path, side, line_start, line_end, content, code, status, code_hash, outdated, round_id, created_at, updated_at";
const ACTIVE_CHECKOUT: &str = "active_checkout_id";
const ACTIVE_SESSION: &str = "active_session_id";
const WORKTREE_LOCATION: &str = "worktree_location";
const WINDOW_GEOMETRY: &str = "window_geometry";
const WINDOW_MAXIMIZED: &str = "window_maximized";
const UI_LAYOUT: &str = "ui_layout_v1";
const SIDEBAR_WIDTH_MIN: u32 = 240;
const SIDEBAR_WIDTH_MAX: u32 = 500;
const INSPECTOR_WIDTH_MIN: u32 = 200;
const INSPECTOR_WIDTH_MAX: u32 = 480;
const PREVIEW_WIDTH_MIN: u32 = 260;
const PREVIEW_WIDTH_MAX: u32 = 900;
/// The layout row is the window's own shape and nothing else. The preferences a person sets once
/// — sizes, ligatures, the scale — live in `~/.marvis/config.yml`, which this shape was widened
/// and narrowed once to make room for, before that file existed.
const APP_LAYOUT_VERSION: u8 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppLayoutState {
    pub version: u8,
    pub mode: String,
    pub sidebar_width: u32,
    pub inspector_width: u32,
    pub preview_width: u32,
}

impl Default for AppLayoutState {
    fn default() -> Self {
        Self {
            version: APP_LAYOUT_VERSION,
            mode: "focus".into(),
            sidebar_width: 240,
            inspector_width: 280,
            preview_width: 360,
        }
    }
}

impl AppLayoutState {
    /// Clamp persisted dimensions and discard unknown layout modes.
    fn normalized(mut self) -> Self {
        if self.mode != "focus" && self.mode != "split" {
            self.mode = "focus".into();
        }
        self.sidebar_width = self
            .sidebar_width
            .clamp(SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX);
        self.inspector_width = self
            .inspector_width
            .clamp(INSPECTOR_WIDTH_MIN, INSPECTOR_WIDTH_MAX);
        self.preview_width = self
            .preview_width
            .clamp(PREVIEW_WIDTH_MIN, PREVIEW_WIDTH_MAX);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedDocument {
    pub checkout_id: String,
    pub path: String,
    pub origin: String,
    pub source: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CheckoutUiState {
    pub version: u8,
    pub document: Option<PersistedDocument>,
    pub main_view: String,
    /// The whole-change-set diff has no path, so it cannot be a `document`. Added after
    /// version 1 shipped: absent in state saved before it, which means `false`.
    pub diff_all_files: bool,
    pub inspector_tab: String,
    pub selected_file_path: Option<String>,
    pub selected_change_path: Option<String>,
    pub expanded_directories: Vec<String>,
    pub files_scroll_top: u32,
    pub changes_scroll_top: u32,
    pub document_scroll_top: u32,
    pub document_scroll_left: u32,
    pub diff_scroll_top: u32,
}

impl Default for CheckoutUiState {
    fn default() -> Self {
        Self {
            version: 1,
            document: None,
            main_view: "terminal".into(),
            diff_all_files: false,
            inspector_tab: "files".into(),
            selected_file_path: None,
            selected_change_path: None,
            expanded_directories: Vec::new(),
            files_scroll_top: 0,
            changes_scroll_top: 0,
            document_scroll_top: 0,
            document_scroll_left: 0,
            diff_scroll_top: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(db_error)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, String> {
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(db_error)?;
        create_schema(&connection)?;
        // No PTY outlives the process that spawned it, so every stored session is dead the
        // moment the app reopens. Keeping the rows piled one per launch per checkout up
        // forever, and every one of them drags a dead tab into that checkout's saved layout.
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        transaction
            .execute("DELETE FROM sessions", [])
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM checkout_terminal_layouts", [])
            .map_err(db_error)?;
        set_preference(&transaction, ACTIVE_SESSION, None)?;
        transaction.commit().map_err(db_error)?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    #[cfg(test)]
    fn open_in_memory() -> Result<Self, String> {
        Self::from_connection(Connection::open_in_memory().map_err(db_error)?)
    }

    pub fn register_plain_repo(&self, repo: Repo) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let current_checkout_id = get_preference(&transaction, ACTIVE_CHECKOUT)?;
        let existing_repo_id = transaction
            .query_row(
                "SELECT repo_id FROM checkouts WHERE canonical_path = ?1",
                [&repo.root],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?;
        let checkout_id = if let Some(repo_id) = existing_repo_id {
            transaction
                .execute(
                    "UPDATE repos SET last_opened_at = ?1 WHERE id = ?2",
                    params![repo.last_opened_at, repo_id],
                )
                .map_err(db_error)?;
            repo.checkouts[0].id.clone()
        } else {
            let repo_position: i64 = transaction
                .query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM repos",
                    [],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            transaction
                .execute(
                    "INSERT INTO repos (id, kind, name, root, default_branch, position, created_at, last_opened_at)
                     VALUES (?1, 'plain', ?2, ?3, NULL, ?4, ?5, ?6)",
                    params![
                        repo.id,
                        repo.name,
                        repo.root,
                        repo_position,
                        repo.created_at,
                        repo.last_opened_at
                    ],
                )
                .map_err(db_error)?;
            let checkout = &repo.checkouts[0];
            transaction
                .execute(
                    "INSERT INTO checkouts
                     (id, repo_id, path, canonical_path, is_primary, branch, head, ahead_of_default, changed_files, position)
                     VALUES (?1, ?2, ?3, ?4, 1, NULL, NULL, NULL, 0, 0)",
                    params![checkout.id, repo.id, checkout.path, checkout.canonical_path],
                )
                .map_err(db_error)?;
            checkout.id.clone()
        };

        set_preference(&transaction, ACTIVE_CHECKOUT, Some(&checkout_id))?;
        if current_checkout_id.as_deref() != Some(checkout_id.as_str()) {
            set_preference(&transaction, ACTIVE_SESSION, None)?;
        }
        transaction
            .execute(
                "INSERT INTO recent_paths (canonical_path, last_opened_at)
                 VALUES (?1, ?2)
                 ON CONFLICT(canonical_path) DO UPDATE SET last_opened_at = excluded.last_opened_at",
                params![repo.root, repo.last_opened_at],
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "DELETE FROM recent_paths WHERE canonical_path NOT IN
                 (SELECT canonical_path FROM recent_paths ORDER BY last_opened_at DESC LIMIT 10)",
                [],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    pub fn register_git_repo(
        &self,
        repo: Repo,
        focus_checkout_id: &str,
    ) -> Result<WorkspaceState, String> {
        self.store_git_repo(&repo, Some(focus_checkout_id))?;
        self.load_workspace()
    }

    pub fn reconcile_git_repo(&self, repo: &Repo) -> Result<(), String> {
        self.store_git_repo(repo, None)
    }

    fn store_git_repo(&self, repo: &Repo, focus_checkout_id: Option<&str>) -> Result<(), String> {
        self.store_git_repo_replacing(repo, focus_checkout_id, None)
    }

    pub fn locate_git_checkout(
        &self,
        repo: &Repo,
        missing_checkout_id: &str,
        focus_checkout_id: &str,
    ) -> Result<WorkspaceState, String> {
        self.store_git_repo_replacing(repo, Some(focus_checkout_id), Some(missing_checkout_id))?;
        self.load_workspace()
    }

    pub fn relocate_plain_checkout(
        &self,
        old_repo_id: &str,
        old_checkout_id: &str,
        new_repo: &Repo,
    ) -> Result<WorkspaceState, String> {
        if new_repo.kind != RepoKind::Plain
            || new_repo.id != crate::domain::workspace::repo_id_for_path(&new_repo.root)
            || new_repo.checkouts.len() != 1
            || !new_repo.checkouts[0].is_primary
            || new_repo.checkouts[0].repo_id != new_repo.id
            || new_repo.checkouts[0].id
                != crate::domain::workspace::checkout_id_for_path(&new_repo.root)
            || new_repo.checkouts[0].canonical_path != new_repo.root
            || Path::new(&new_repo.root).canonicalize().ok().as_deref()
                != Some(Path::new(&new_repo.root))
        {
            return Err("located plain checkout failed backend identity validation".into());
        }
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let (kind, is_primary, stored_path, old_root, created_at, repo_position, is_missing) =
            transaction
                .query_row(
                    "SELECT r.kind, c.is_primary, c.canonical_path, r.root, r.created_at,
                            r.position, c.is_missing
                     FROM checkouts c JOIN repos r ON r.id = c.repo_id
                     WHERE r.id = ?1 AND c.id = ?2",
                    params![old_repo_id, old_checkout_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, bool>(6)?,
                        ))
                    },
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "missing plain checkout is no longer registered".to_string())?;
        if kind != "plain"
            || !is_primary
            || is_missing
            || Path::new(&stored_path).is_dir()
            || new_repo.id == old_repo_id
            || new_repo.checkouts[0].canonical_path != new_repo.root
            || old_repo_id != crate::domain::workspace::repo_id_for_path(&old_root)
            || old_checkout_id != crate::domain::workspace::checkout_id_for_path(&stored_path)
        {
            return Err("located directory does not match a missing plain checkout".into());
        }
        let checkout_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM checkouts WHERE repo_id = ?1",
                [old_repo_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let active_sessions: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE checkout_id = ?1 AND status = 'active')",
                [old_checkout_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if checkout_count != 1 || active_sessions {
            return Err("close active sessions before relocating this plain checkout".into());
        }
        let path_in_use: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkouts WHERE canonical_path = ?1)",
                [&new_repo.root],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let repo_id_in_use: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM repos WHERE id = ?1)",
                [&new_repo.id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if path_in_use || repo_id_in_use {
            return Err("located directory is already registered in Marvis".into());
        }
        let next_position: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM repos",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "UPDATE repos SET position = ?1 WHERE id = ?2",
                params![next_position, old_repo_id],
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO repos (id, kind, name, root, default_branch, position, created_at, last_opened_at)
                 VALUES (?1, 'plain', ?2, ?3, NULL, ?4, ?5, ?6)",
                params![
                    new_repo.id,
                    new_repo.name,
                    new_repo.root,
                    repo_position,
                    created_at,
                    new_repo.last_opened_at,
                ],
            )
            .map_err(db_error)?;
        let checkout = &new_repo.checkouts[0];
        transaction
            .execute(
                "INSERT INTO checkouts
                 (id, repo_id, path, canonical_path, is_primary, branch, head, ahead_of_default, changed_files, position, is_missing)
                 VALUES (?1, ?2, ?3, ?4, 1, NULL, NULL, NULL, 0, 0, 0)",
                params![checkout.id, new_repo.id, checkout.path, checkout.canonical_path],
            )
            .map_err(db_error)?;
        transfer_checkout_metadata(&transaction, old_checkout_id, &checkout.id)?;
        transaction
            .execute("DELETE FROM repos WHERE id = ?1", [old_repo_id])
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO recent_paths (canonical_path, last_opened_at) VALUES (?1, ?2)
                 ON CONFLICT(canonical_path) DO UPDATE SET last_opened_at = excluded.last_opened_at",
                params![new_repo.root, new_repo.last_opened_at],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    fn store_git_repo_replacing(
        &self,
        repo: &Repo,
        focus_checkout_id: Option<&str>,
        replace_missing_checkout_id: Option<&str>,
    ) -> Result<(), String> {
        if repo.kind != RepoKind::Git
            || repo.id != crate::domain::workspace::repo_id_for_path(&repo.root)
            || repo.checkouts.is_empty()
            || repo.checkouts.iter().any(|checkout| {
                checkout.repo_id != repo.id
                    || checkout.id
                        != crate::domain::workspace::checkout_id_for_path(&checkout.canonical_path)
            })
            || !repo
                .checkouts
                .iter()
                .any(|checkout| checkout.is_primary && checkout.canonical_path == repo.root)
        {
            return Err("Git repository membership failed backend validation".into());
        }
        if let Some(checkout_id) = focus_checkout_id {
            if !repo
                .checkouts
                .iter()
                .any(|checkout| checkout.id == checkout_id)
            {
                return Err("requested checkout is not a member of the resolved repository".into());
            }
        }

        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let now = &repo.last_opened_at;
        let position: i64 = transaction
            .query_row(
                "SELECT COALESCE((SELECT position FROM repos WHERE id = ?1), MAX(position) + 1, 0) FROM repos",
                [&repo.id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO repos (id, kind, name, root, default_branch, position, created_at, last_opened_at)
                 VALUES (?1, 'git', ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                   kind = 'git', name = excluded.name, root = excluded.root,
                   default_branch = COALESCE(repos.default_branch, excluded.default_branch),
                   last_opened_at = CASE WHEN ?8 THEN excluded.last_opened_at ELSE repos.last_opened_at END",
                params![
                    repo.id,
                    repo.name,
                    repo.root,
                    repo.default_branch,
                    position,
                    repo.created_at,
                    now,
                    focus_checkout_id.is_some(),
                ],
            )
            .map_err(db_error)?;
        let position_offset: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + ?2 FROM checkouts WHERE repo_id = ?1",
                params![repo.id, repo.checkouts.len() as i64 + 1],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "UPDATE checkouts SET is_missing = 1, position = position + ?2 WHERE repo_id = ?1",
                params![repo.id, position_offset],
            )
            .map_err(db_error)?;
        for (checkout_position, checkout) in repo.checkouts.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO checkouts
                     (id, repo_id, path, canonical_path, is_primary, branch, head, ahead_of_default, changed_files, position, is_missing)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                     ON CONFLICT(id) DO UPDATE SET
                       repo_id = excluded.repo_id, path = excluded.path,
                       canonical_path = excluded.canonical_path, is_primary = excluded.is_primary,
                       branch = excluded.branch, head = excluded.head,
                       ahead_of_default = excluded.ahead_of_default, position = excluded.position,
                       is_missing = excluded.is_missing",
                    params![
                        checkout.id,
                        repo.id,
                        checkout.path,
                        checkout.canonical_path,
                        checkout.is_primary,
                        checkout.branch,
                        checkout.head,
                        checkout.ahead_of_default,
                        checkout.changed_files,
                        checkout_position as i64,
                        checkout.is_missing,
                    ],
                )
                .map_err(db_error)?;
        }
        if let Some(missing_checkout_id) = replace_missing_checkout_id {
            let (old_repo_id, is_primary, is_missing, branch, head) = transaction
                .query_row(
                    "SELECT repo_id, is_primary, is_missing, branch, head FROM checkouts WHERE id = ?1",
                    [missing_checkout_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, bool>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "missing checkout is no longer registered".to_string())?;
            let replacement = repo
                .checkouts
                .iter()
                .find(|checkout| Some(checkout.id.as_str()) == focus_checkout_id)
                .ok_or_else(|| {
                    "located checkout is not part of the resolved repository".to_string()
                })?;
            let identity_matches = match (branch.as_deref(), replacement.branch.as_deref()) {
                (Some(previous), Some(located)) => previous == located,
                (None, None) => head
                    .as_deref()
                    .is_some_and(|previous| replacement.head.as_deref() == Some(previous)),
                _ => false,
            };
            if old_repo_id != repo.id || is_primary || !is_missing || !identity_matches {
                return Err(
                    "located directory does not match the missing worktree identity".into(),
                );
            }
            let has_active_sessions: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE checkout_id = ?1 AND status = 'active')",
                    [missing_checkout_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if has_active_sessions {
                return Err(
                    "close active terminal sessions before relocating this worktree".into(),
                );
            }
            transfer_checkout_metadata(&transaction, missing_checkout_id, &replacement.id)?;
            transaction
                .execute("DELETE FROM checkouts WHERE id = ?1", [missing_checkout_id])
                .map_err(db_error)?;
        }
        if let Some(checkout_id) = focus_checkout_id {
            let current_checkout_id = get_preference(&transaction, ACTIVE_CHECKOUT)?;
            set_preference(&transaction, ACTIVE_CHECKOUT, Some(checkout_id))?;
            if current_checkout_id.as_deref() != Some(checkout_id) {
                set_preference(&transaction, ACTIVE_SESSION, None)?;
            }
            transaction
                .execute(
                    "INSERT INTO recent_paths (canonical_path, last_opened_at)
                     VALUES (?1, ?2)
                     ON CONFLICT(canonical_path) DO UPDATE SET last_opened_at = excluded.last_opened_at",
                    params![repo.root, now],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)
    }

    pub fn existing_git_checkout(&self, repo_id: &str) -> Result<Option<PathBuf>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT canonical_path FROM checkouts
                 WHERE repo_id = ?1 ORDER BY is_primary DESC, position",
            )
            .map_err(db_error)?;
        let paths = statement
            .query_map([repo_id], |row| row.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(paths
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_dir()))
    }

    pub fn set_default_branch(
        &self,
        repo_id: &str,
        branch: &str,
    ) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let updated = connection
            .execute(
                "UPDATE repos SET default_branch = ?1 WHERE id = ?2 AND kind = 'git'",
                params![branch, repo_id],
            )
            .map_err(db_error)?;
        if updated == 0 {
            return Err("Git repository does not exist".into());
        }
        drop(connection);
        self.load_workspace()
    }

    pub fn worktree_location(&self) -> Result<Option<String>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        get_preference(&connection, WORKTREE_LOCATION)
    }

    pub fn set_worktree_location(&self, path: &str) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO preferences (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![WORKTREE_LOCATION, path],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn window_geometry(&self) -> Result<Option<WindowGeometry>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        get_preference(&connection, WINDOW_GEOMETRY)?
            .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .transpose()
    }

    pub fn set_window_geometry(&self, geometry: WindowGeometry) -> Result<(), String> {
        if geometry.width < 900
            || geometry.height < 600
            || geometry.width > 16_384
            || geometry.height > 16_384
        {
            return Err("window geometry is outside the supported range".into());
        }
        let serialized = serde_json::to_string(&geometry).map_err(|error| error.to_string())?;
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO preferences (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![WINDOW_GEOMETRY, serialized],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn window_maximized(&self) -> Result<bool, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        Ok(get_preference(&connection, WINDOW_MAXIMIZED)?.as_deref() == Some("true"))
    }

    pub fn set_window_maximized(&self, maximized: bool) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO preferences (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![WINDOW_MAXIMIZED, maximized.to_string()],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn load_app_layout(&self) -> Result<AppLayoutState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let Some(serialized) = get_preference(&connection, UI_LAYOUT)? else {
            return Ok(AppLayoutState::default());
        };
        let layout = serde_json::from_str::<AppLayoutState>(&serialized)
            .ok()
            .map(AppLayoutState::normalized)
            .filter(|layout| layout.version == APP_LAYOUT_VERSION)
            .unwrap_or_default();
        if serde_json::to_string(&layout).map_err(|error| error.to_string())? != serialized {
            connection
                .execute("DELETE FROM preferences WHERE key = ?1", [UI_LAYOUT])
                .map_err(db_error)?;
        }
        Ok(layout)
    }

    pub fn save_app_layout(&self, layout: &AppLayoutState) -> Result<(), String> {
        let layout = layout.clone().normalized();
        if layout.version != APP_LAYOUT_VERSION {
            return Err("saved UI layout has an unsupported version".into());
        }
        let serialized = serde_json::to_string(&layout).map_err(|error| error.to_string())?;
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO preferences (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![UI_LAYOUT, serialized],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn load_checkout_ui_state(&self, checkout_id: &str) -> Result<CheckoutUiState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkouts WHERE id = ?1)",
                [checkout_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !exists {
            return Err("checkout does not exist".into());
        }
        let serialized = connection
            .query_row(
                "SELECT state_json FROM checkout_ui_states WHERE checkout_id = ?1",
                [checkout_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?;
        let Some(serialized) = serialized else {
            return Ok(CheckoutUiState::default());
        };
        let state = serde_json::from_str::<CheckoutUiState>(&serialized)
            .ok()
            .filter(|state| validate_checkout_ui_state(checkout_id, state))
            .unwrap_or_default();
        if serde_json::to_string(&state).map_err(|error| error.to_string())? != serialized {
            connection
                .execute(
                    "DELETE FROM checkout_ui_states WHERE checkout_id = ?1",
                    [checkout_id],
                )
                .map_err(db_error)?;
        }
        Ok(state)
    }

    pub fn review_target(&self, checkout_id: &str) -> Result<String, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let key = format!("review_target:{checkout_id}");
        Ok(match get_preference(&connection, &key)?.as_deref() {
            Some("opencode") => "opencode".into(),
            _ => "markdown".into(),
        })
    }

    pub fn set_review_target(&self, checkout_id: &str, target: &str) -> Result<(), String> {
        if !matches!(target, "markdown" | "opencode") {
            return Err("review target is not supported".into());
        }
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let key = format!("review_target:{checkout_id}");
        set_preference(&transaction, &key, Some(target))?;
        transaction.commit().map_err(db_error)
    }

    pub fn save_checkout_ui_state(
        &self,
        checkout_id: &str,
        state: &CheckoutUiState,
    ) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let checkout_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkouts WHERE id = ?1)",
                [checkout_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !checkout_exists || !validate_checkout_ui_state(checkout_id, state) {
            return Err("saved checkout UI state failed validation".into());
        }
        let serialized = serde_json::to_string(state).map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO checkout_ui_states (checkout_id, state_json) VALUES (?1, ?2)
                 ON CONFLICT(checkout_id) DO UPDATE SET state_json = excluded.state_json",
                params![checkout_id, serialized],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Drops the registration of a checkout whose directory is still on disk, and nothing else.
    ///
    /// This is what takes a workdir off the panel while its files stay where they are: the row
    /// and everything that hangs off it go, the directory does not, so opening the folder again
    /// brings the workdir back.
    pub fn close_checkout(&self, checkout_id: &str) -> Result<WorkspaceState, String> {
        self.forget_checkout(checkout_id, false)
    }

    /// Drops a checkout that cannot be opened, and nothing else. A live one is refused, so that
    /// the only way to forget it is [`Database::close_checkout`], which asks for it on purpose.
    pub fn close_missing_checkout(&self, checkout_id: &str) -> Result<WorkspaceState, String> {
        self.forget_checkout(checkout_id, true)
    }

    /// The one routine behind both closes. `require_missing` is the whole difference: a checkout
    /// whose directory is gone has nothing left on disk to speak for, so it can be dropped on
    /// sight, while a live one is only dropped when the caller meant it.
    fn forget_checkout(
        &self,
        checkout_id: &str,
        require_missing: bool,
    ) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let (repo_id, is_primary, stored_missing, kind, path) = transaction
            .query_row(
                "SELECT c.repo_id, c.is_primary, c.is_missing, r.kind, c.canonical_path FROM checkouts c
                 JOIN repos r ON r.id = c.repo_id WHERE c.id = ?1",
                [checkout_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "checkout is no longer registered".to_string())?;
        let is_missing = stored_missing || !Path::new(&path).is_dir();
        if require_missing && !is_missing {
            return Err("only a missing checkout can be closed".into());
        }
        let affected_id = if is_primary || kind == "plain" {
            &repo_id
        } else {
            checkout_id
        };
        let has_active_sessions: bool = transaction
            .query_row(
                if is_primary || kind == "plain" {
                    "SELECT EXISTS(SELECT 1 FROM sessions s JOIN checkouts c ON c.id = s.checkout_id WHERE c.repo_id = ?1 AND s.status = 'active')"
                } else {
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE checkout_id = ?1 AND status = 'active')"
                },
                [affected_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if has_active_sessions {
            // A terminal still attached to the directory would outlive the row that names it.
            return Err(if is_missing {
                "close active terminal sessions before closing this missing location".into()
            } else {
                "close active terminal sessions before closing this checkout".into()
            });
        }
        let current_checkout_id = get_preference(&transaction, ACTIVE_CHECKOUT)?;
        if is_primary || kind == "plain" {
            transaction
                .execute("DELETE FROM repos WHERE id = ?1", [&repo_id])
                .map_err(db_error)?;
            if current_checkout_id.as_deref().is_some_and(|active_id| {
                transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM checkouts WHERE id = ?1)",
                        [active_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .unwrap_or(false)
            }) {
                // An active checkout in a different repo remains selected.
            } else {
                set_preference(&transaction, ACTIVE_CHECKOUT, None)?;
                set_preference(&transaction, ACTIVE_SESSION, None)?;
            }
        } else {
            transaction
                .execute(
                    "DELETE FROM checkouts WHERE id = ?1 AND repo_id = ?2",
                    params![checkout_id, repo_id],
                )
                .map_err(db_error)?;
            if current_checkout_id.as_deref() == Some(checkout_id) {
                let primary_id: String = transaction
                    .query_row(
                        "SELECT id FROM checkouts WHERE repo_id = ?1 AND is_primary = 1",
                        [&repo_id],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                set_preference(&transaction, ACTIVE_CHECKOUT, Some(&primary_id))?;
                set_preference(&transaction, ACTIVE_SESSION, None)?;
            }
        }
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    /// Takes a worktree off the panel without forgetting it, and leaves the disk exactly
    /// as it was: the branch, the commits and the files stay, and
    /// [`Database::restore_archived_worktrees`] puts the row back.
    ///
    /// Only a live worktree of a Git repository can be archived. A repo root is the head
    /// of the list its worktrees hang off, so archiving it would archive the list; a
    /// plain folder is not a worktree at all; and a worktree whose directory is gone has
    /// nothing to bring back, which is what closing it is for. A terminal still running
    /// in it is refused for the same reason a close is: the process would outlive the
    /// row that names it.
    pub fn archive_checkout(&self, checkout_id: &str) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let (repo_id, is_primary, stored_missing, kind, path) = transaction
            .query_row(
                "SELECT c.repo_id, c.is_primary, c.is_missing, r.kind, c.canonical_path FROM checkouts c
                 JOIN repos r ON r.id = c.repo_id WHERE c.id = ?1",
                [checkout_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "checkout is no longer registered".to_string())?;
        if is_primary || kind == "plain" {
            return Err("only a worktree can be archived".into());
        }
        if stored_missing || !Path::new(&path).is_dir() {
            return Err(
                "a worktree whose directory is gone cannot be archived; close it instead".into(),
            );
        }
        let has_active_sessions: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE checkout_id = ?1 AND status = 'active')",
                [checkout_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if has_active_sessions {
            return Err("close active terminal sessions before archiving this worktree".into());
        }
        transaction
            .execute(
                "UPDATE checkouts SET is_archived = 1 WHERE id = ?1",
                [checkout_id],
            )
            .map_err(db_error)?;
        // The row leaves the panel, so the selection cannot stay on it: the workdir the
        // user is looking at after archiving is the repo root, the same place a close
        // hands the selection over to.
        if get_preference(&transaction, ACTIVE_CHECKOUT)?.as_deref() == Some(checkout_id) {
            let primary_id: String = transaction
                .query_row(
                    "SELECT id FROM checkouts WHERE repo_id = ?1 AND is_primary = 1",
                    [&repo_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            set_preference(&transaction, ACTIVE_CHECKOUT, Some(&primary_id))?;
            set_preference(&transaction, ACTIVE_SESSION, None)?;
        }
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    /// Puts every archived worktree of one repository back on the panel, and leaves the
    /// disk alone: nothing here creates, removes or checks out anything.
    pub fn restore_archived_worktrees(&self, repo_id: &str) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM repos WHERE id = ?1)",
                [repo_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !exists {
            return Err("repository is no longer registered".into());
        }
        transaction
            .execute(
                "UPDATE checkouts SET is_archived = 0 WHERE repo_id = ?1 AND is_archived = 1",
                [repo_id],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    /// Drops every checkout whose directory no longer exists, and then every repository left
    /// without checkouts, so a reopened app lists only locations that are really on disk.
    ///
    /// `!Path::new(path).is_dir()` is the same test `close_missing_checkout` applies, so both
    /// agree on what "missing" means. A path on a volume that is not mounted fails it too:
    /// reopening Marvis with an external volume unplugged therefore discards the worktrees
    /// registered on it. To keep those registrations, drop the `prune_missing_checkouts` call
    /// from `services::workspace::restore`; a checkout whose directory is still gone then stays
    /// listed as `is_missing` and can be closed from the sidebar.
    pub fn prune_missing_checkouts(&self) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let mut statement = transaction
            .prepare("SELECT id, repo_id, canonical_path FROM checkouts")
            .map_err(db_error)?;
        let checkouts: Vec<(String, String, String)> = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        drop(statement);
        let active_id = get_preference(&transaction, ACTIVE_CHECKOUT)?;
        let mut active_repo_id = None;
        for (checkout_id, repo_id, path) in &checkouts {
            if Path::new(path).is_dir() {
                continue;
            }
            if active_id.as_deref() == Some(checkout_id.as_str()) {
                active_repo_id = Some(repo_id.as_str());
            }
            transaction
                .execute("DELETE FROM checkouts WHERE id = ?1", [checkout_id])
                .map_err(db_error)?;
        }
        // A repository whose checkouts are all gone has nothing left to represent.
        transaction
            .execute(
                "DELETE FROM repos WHERE NOT EXISTS (SELECT 1 FROM checkouts WHERE repo_id = repos.id)",
                [],
            )
            .map_err(db_error)?;
        if let Some(repo_id) = active_repo_id {
            // The selection that was dropped hands over to the repository's surviving primary
            // checkout, the same choice `close_missing_checkout` makes; nothing left means
            // nothing stays selected.
            let primary_id: Option<String> = transaction
                .query_row(
                    "SELECT id FROM checkouts WHERE repo_id = ?1 AND is_primary = 1",
                    [repo_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            set_preference(&transaction, ACTIVE_CHECKOUT, primary_id.as_deref())?;
            set_preference(&transaction, ACTIVE_SESSION, None)?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    pub fn terminal_session_checkout(&self, session_id: &str) -> Result<Option<String>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                "SELECT checkout_id FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)
    }

    pub fn remove_checkout(
        &self,
        repo_id: &str,
        checkout_id: &str,
    ) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let checkout = transaction
            .query_row(
                "SELECT is_primary FROM checkouts WHERE id = ?1 AND repo_id = ?2",
                params![checkout_id, repo_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "checkout is not registered under this repository".to_string())?;
        if checkout {
            return Err("the primary checkout cannot be removed".into());
        }
        transaction
            .execute(
                "DELETE FROM checkouts WHERE id = ?1 AND repo_id = ?2",
                params![checkout_id, repo_id],
            )
            .map_err(db_error)?;
        if get_preference(&transaction, ACTIVE_CHECKOUT)?.as_deref() == Some(checkout_id) {
            let primary_id: String = transaction
                .query_row(
                    "SELECT id FROM checkouts WHERE repo_id = ?1 AND is_primary = 1",
                    [repo_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            set_preference(&transaction, ACTIVE_CHECKOUT, Some(&primary_id))?;
            set_preference(&transaction, ACTIVE_SESSION, None)?;
        } else if let Some(active_session) = get_preference(&transaction, ACTIVE_SESSION)? {
            let still_exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
                    [&active_session],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if !still_exists {
                set_preference(&transaction, ACTIVE_SESSION, None)?;
            }
        }
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    pub fn select_checkout(&self, checkout_id: Option<String>) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        if let Some(checkout_id) = checkout_id.as_deref() {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM checkouts WHERE id = ?1)",
                    [checkout_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if !exists {
                return Err("checkout does not exist".into());
            }
            let current = get_preference(&transaction, ACTIVE_CHECKOUT)?;
            set_preference(&transaction, ACTIVE_CHECKOUT, Some(checkout_id))?;
            if current.as_deref() != Some(checkout_id) {
                set_preference(&transaction, ACTIVE_SESSION, None)?;
            }
        } else {
            set_preference(&transaction, ACTIVE_CHECKOUT, None)?;
            set_preference(&transaction, ACTIVE_SESSION, None)?;
        }
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    pub fn select_session(&self, session_id: Option<String>) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        if let Some(session_id) = session_id.as_deref() {
            let checkout_id = transaction
                .query_row(
                    "SELECT checkout_id FROM sessions WHERE id = ?1",
                    [session_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| "session does not exist".to_string())?;
            set_preference(&transaction, ACTIVE_CHECKOUT, Some(&checkout_id))?;
            set_preference(&transaction, ACTIVE_SESSION, Some(session_id))?;
        } else {
            set_preference(&transaction, ACTIVE_SESSION, None)?;
        }
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    pub fn terminal_checkout_path(&self, checkout_id: &str) -> Result<PathBuf, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let stored_path = connection
            .query_row(
                "SELECT canonical_path FROM checkouts WHERE id = ?1 AND is_missing = 0",
                [checkout_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "checkout does not exist or is missing".to_string())?;
        let path = PathBuf::from(&stored_path)
            .canonicalize()
            .map_err(|error| format!("checkout directory is unavailable: {error}"))?;
        if !path.is_dir() || path != Path::new(&stored_path) {
            return Err("checkout path is unavailable or no longer canonical".into());
        }
        Ok(path)
    }

    pub fn add_terminal_session(&self, session: &Session) -> Result<WorkspaceState, String> {
        self.add_active_session(session)
    }

    pub fn add_active_session(&self, session: &Session) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let checkout_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkouts WHERE id = ?1 AND is_missing = 0)",
                [&session.checkout_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !checkout_exists {
            return Err("checkout does not exist or is missing".into());
        }
        transaction
            .execute(
                "INSERT INTO sessions (id, checkout_id, session_type, name, created_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
                params![
                    session.id,
                    session.checkout_id,
                    session_type_name(&session.session_type),
                    session.name,
                    session.created_at
                ],
            )
            .map_err(db_error)?;
        set_preference(&transaction, ACTIVE_CHECKOUT, Some(&session.checkout_id))?;
        set_preference(&transaction, ACTIVE_SESSION, Some(&session.id))?;
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    pub fn remove_terminal_session(&self, session_id: &str) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let removed = transaction
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])
            .map_err(db_error)?;
        if removed == 0 {
            return Err("session does not exist".into());
        }
        if get_preference(&transaction, ACTIVE_SESSION)?.as_deref() == Some(session_id) {
            set_preference(&transaction, ACTIVE_SESSION, None)?;
        }
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    /// Renames a session in place.
    ///
    /// The row is already there and the name is a plain label, so this is one `UPDATE`: no new
    /// column and no new table, which matters because the schema is written once and never
    /// migrated. The workspace comes back so the caller paints the name the database now holds
    /// rather than the one it asked for.
    pub fn rename_terminal_session(
        &self,
        session_id: &str,
        name: &str,
    ) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let renamed = connection
            .execute(
                "UPDATE sessions SET name = ?2 WHERE id = ?1",
                rusqlite::params![session_id, name],
            )
            .map_err(db_error)?;
        if renamed == 0 {
            return Err("session does not exist".into());
        }
        drop(connection);
        self.load_workspace()
    }

    /// Moves a terminal session to another worktree of the same repository.
    ///
    /// The process is not touched: a session is a row, and moving it hands the row to another
    /// checkout so the terminal belongs to the worktree it is listed under. Both layouts are
    /// reconciled in the same transaction as the row, because a layout still naming a session its
    /// checkout no longer holds is a layout the next read prunes and the next save refuses.
    pub fn move_terminal_session(
        &self,
        session_id: &str,
        target_checkout_id: &str,
    ) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let source_checkout_id: String = transaction
            .query_row(
                "SELECT checkout_id FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "session does not exist".to_string())?;
        if source_checkout_id == target_checkout_id {
            return Err("the terminal session is already in that worktree".into());
        }
        let target_repo_id: String = transaction
            .query_row(
                "SELECT repo_id FROM checkouts WHERE id = ?1 AND is_missing = 0 AND is_archived = 0",
                [target_checkout_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "checkout does not exist or is missing".to_string())?;
        let source_repo_id: String = transaction
            .query_row(
                "SELECT repo_id FROM checkouts WHERE id = ?1",
                [&source_checkout_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        // One repository is one Git directory, so its worktrees are the only checkouts a session
        // may be handed to; anything else would be a session labelled with a tree it is not in.
        if source_repo_id != target_repo_id {
            return Err("a terminal session cannot leave the repository it was opened in".into());
        }
        transaction
            .execute(
                "UPDATE sessions SET checkout_id = ?1 WHERE id = ?2",
                params![target_checkout_id, session_id],
            )
            .map_err(db_error)?;
        reconcile_stored_layout(&transaction, &source_checkout_id)?;
        reconcile_stored_layout(&transaction, target_checkout_id)?;
        // The worktree the session now belongs to is the one whose files, changes and agent the
        // window shows, so the move selects it and the session inside it.
        set_preference(&transaction, ACTIVE_CHECKOUT, Some(target_checkout_id))?;
        set_preference(&transaction, ACTIVE_SESSION, Some(session_id))?;
        transaction.commit().map_err(db_error)?;
        drop(connection);
        self.load_workspace()
    }

    pub fn load_workspace(&self) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        load_workspace(&connection)
    }

    /// The folders opened before, newest first, inside the ten the writes keep. The stamps are
    /// ISO-8601, so ordering them as text is ordering them in time.
    pub fn list_recent_paths(&self) -> Result<Vec<RecentPath>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT canonical_path, last_opened_at FROM recent_paths
                 ORDER BY last_opened_at DESC LIMIT 10",
            )
            .map_err(db_error)?;
        let paths = statement
            .query_map([], |row| {
                Ok(RecentPath {
                    canonical_path: row.get(0)?,
                    last_opened_at: row.get(1)?,
                })
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(paths)
    }

    pub fn load_terminal_layout(
        &self,
        checkout_id: &str,
    ) -> Result<Option<CheckoutTerminalLayout>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let session_ids = terminal_layout_session_ids(&transaction, checkout_id)?;
        let serialized = transaction
            .query_row(
                "SELECT layout_json FROM checkout_terminal_layouts WHERE checkout_id = ?1",
                [checkout_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?;
        let Some(serialized) = serialized else {
            transaction.commit().map_err(db_error)?;
            return Ok(None);
        };
        let mut layout: CheckoutTerminalLayout = serde_json::from_str(&serialized)
            .map_err(|error| format!("saved terminal layout is invalid: {error}"))?;
        layout.reconcile_sessions(&session_ids);
        validate_terminal_layout(&transaction, checkout_id, &layout)?;
        let normalized = serde_json::to_string(&layout).map_err(|error| error.to_string())?;
        if normalized != serialized {
            transaction
                .execute(
                    "UPDATE checkout_terminal_layouts SET layout_json = ?1 WHERE checkout_id = ?2",
                    params![normalized, checkout_id],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(Some(layout))
    }

    pub fn save_terminal_layout(
        &self,
        checkout_id: &str,
        layout: &CheckoutTerminalLayout,
    ) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let session_ids = terminal_layout_session_ids(&transaction, checkout_id)?;
        let mut normalized = layout.clone();
        normalized.reconcile_sessions(&session_ids);
        validate_terminal_layout(&transaction, checkout_id, &normalized)?;
        let serialized = serde_json::to_string(&normalized).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO checkout_terminal_layouts (checkout_id, layout_json)
                 VALUES (?1, ?2)
                 ON CONFLICT(checkout_id) DO UPDATE SET layout_json = excluded.layout_json",
                params![checkout_id, serialized],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }

    pub fn viewed_files(&self, checkout_id: &str) -> Result<Vec<String>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare("SELECT path FROM viewed_files WHERE checkout_id = ?1 ORDER BY path")
            .map_err(db_error)?;
        let files = statement
            .query_map([checkout_id], |row| row.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(files)
    }

    pub fn mark_file_viewed(&self, checkout_id: &str, path: &str) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO viewed_files (checkout_id, path, viewed_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(checkout_id, path) DO UPDATE SET viewed_at = excluded.viewed_at",
                params![checkout_id, path, timestamp()],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn review_notes(&self, checkout_id: &str) -> Result<Vec<ReviewNote>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {REVIEW_NOTE_COLUMNS} FROM review_notes
                 WHERE checkout_id = ?1 ORDER BY path, line_start, created_at"
            ))
            .map_err(db_error)?;
        let notes = statement
            .query_map([checkout_id], read_review_note)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(notes)
    }

    pub fn add_review_note(&self, note: &ReviewNote) -> Result<ReviewNote, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO review_notes (id, checkout_id, path, side, line_start, line_end, content, code, status, code_hash, outdated, round_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    note.id,
                    note.checkout_id,
                    note.path,
                    note.side,
                    note.line_start,
                    note.line_end,
                    note.content,
                    note.code,
                    note.status,
                    note.code_hash,
                    note.outdated,
                    note.round_id,
                    note.created_at,
                    note.updated_at,
                ],
            )
            .map_err(db_error)?;
        Ok(note.clone())
    }

    /// Updates a note only when it belongs to `checkout_id`; a foreign ID is rejected.
    pub fn update_review_note(
        &self,
        id: &str,
        checkout_id: &str,
        content: &str,
    ) -> Result<ReviewNote, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let updated_at = timestamp();
        let changed = connection
            .execute(
                "UPDATE review_notes SET content = ?1, status = 'draft', updated_at = ?2
                 WHERE id = ?3 AND checkout_id = ?4",
                params![content, updated_at, id, checkout_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("review note does not belong to the requested checkout".into());
        }
        self.read_review_note_by_id(&connection, id)
    }

    pub fn delete_review_note(&self, id: &str, checkout_id: &str) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let deleted = connection
            .execute(
                "DELETE FROM review_notes WHERE id = ?1 AND checkout_id = ?2",
                params![id, checkout_id],
            )
            .map_err(db_error)?;
        if deleted == 0 {
            return Err("review note does not belong to the requested checkout".into());
        }
        Ok(())
    }

    pub fn mark_review_notes_sent(
        &self,
        checkout_id: &str,
        ids: &[String],
    ) -> Result<usize, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let updated_at = timestamp();
        let mut marked = 0;
        for id in ids {
            marked += transaction
                .execute(
                    "UPDATE review_notes SET status = 'sent', updated_at = ?1
                     WHERE id = ?2 AND checkout_id = ?3",
                    params![updated_at, id, checkout_id],
                )
                .map_err(db_error)?;
        }
        if marked != ids.len() {
            let _ = transaction.rollback();
            return Err("review note does not belong to the requested checkout".into());
        }
        transaction.commit().map_err(db_error)?;
        Ok(marked)
    }

    /// Stamps a single note as outdated. Never clears the mark: only the user may do that.
    pub fn mark_review_note_outdated(
        &self,
        id: &str,
        checkout_id: &str,
    ) -> Result<ReviewNote, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let changed = connection
            .execute(
                "UPDATE review_notes SET outdated = 1, updated_at = ?1
                 WHERE id = ?2 AND checkout_id = ?3",
                params![timestamp(), id, checkout_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("review note does not belong to the requested checkout".into());
        }
        self.read_review_note_by_id(&connection, id)
    }

    /// Clears the outdated mark, but only for a note that is actually marked.
    pub fn clear_review_note_outdated(
        &self,
        id: &str,
        checkout_id: &str,
    ) -> Result<ReviewNote, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let changed = connection
            .execute(
                "UPDATE review_notes SET outdated = 0, updated_at = ?1
                 WHERE id = ?2 AND checkout_id = ?3 AND outdated = 1",
                params![timestamp(), id, checkout_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("review note does not belong to the requested checkout".into());
        }
        self.read_review_note_by_id(&connection, id)
    }

    /// Marks a note resolved, and only for a note still in flight.
    pub fn resolve_review_note(&self, id: &str, checkout_id: &str) -> Result<ReviewNote, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let changed = connection
            .execute(
                "UPDATE review_notes SET status = 'resolved', updated_at = ?1
                 WHERE id = ?2 AND checkout_id = ?3 AND status = 'sent'",
                params![timestamp(), id, checkout_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("review note is not an outstanding note of this checkout".into());
        }
        self.read_review_note_by_id(&connection, id)
    }

    /// Records a round, and links its notes to it, in one transaction.
    ///
    /// `round.status` is written before the agent is called, so a crash between here and
    /// the send leaves a `dispatching` round that reconciliation can resolve.
    /// Stores a round with the exact message it will be sent with, if it has one yet.
    pub fn add_review_round(
        &self,
        round: &ReviewRound,
        note_ids: &[String],
        prompt: Option<&str>,
    ) -> Result<ReviewRound, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let encoded = serde_json::to_string(note_ids)
            .map_err(|error| format!("could not encode notes: {error}"))?;
        transaction
            .execute(
                "INSERT INTO review_rounds (id, checkout_id, session_id, status, marker, note_ids, prompt, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    round.id,
                    round.checkout_id,
                    round.session_id,
                    round.status,
                    round.marker,
                    encoded,
                    prompt,
                    round.created_at,
                    round.updated_at,
                ],
            )
            .map_err(db_error)?;
        for id in note_ids {
            let linked = transaction
                .execute(
                    "UPDATE review_notes
                     SET status = 'sent', round_id = ?1, updated_at = ?2
                     WHERE id = ?3 AND checkout_id = ?4",
                    params![round.id, round.updated_at, id, round.checkout_id],
                )
                .map_err(db_error)?;
            if linked == 0 {
                let _ = transaction.rollback();
                return Err("review note does not belong to the requested checkout".into());
            }
        }
        transaction.commit().map_err(db_error)?;
        Ok(round.clone())
    }

    /// The message a queued round is waiting to be sent with.
    ///
    /// Kept off [`ReviewRound`] on purpose: the prompt can be half a megabyte and the UI
    /// never asks for it, only the flush that is about to send it does.
    pub fn review_round_prompt(
        &self,
        id: &str,
        checkout_id: &str,
    ) -> Result<Option<String>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        // A round of another checkout must read as "no message", not as an error: the caller
        // is asking what to send, and "not ours" is a plain answer to that.
        connection
            .query_row(
                "SELECT prompt FROM review_rounds WHERE id = ?1 AND checkout_id = ?2",
                params![id, checkout_id],
                |row| row.get::<_, Option<String>>(0),
            )
            // `.optional()` turns "no such round" into `None`; the closure's own `Option`
            // is there for a round whose message column is NULL. The two must not stack.
            .optional()
            .map_err(db_error)
            .map(|stored| stored.flatten())
    }

    pub fn review_rounds(&self, checkout_id: &str) -> Result<Vec<ReviewRound>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let rounds = {
            let mut statement = connection
                .prepare(
                    "SELECT id, checkout_id, session_id, status, marker, note_ids, created_at, updated_at
                     FROM review_rounds WHERE checkout_id = ?1 ORDER BY created_at DESC, rowid DESC",
                )
                .map_err(db_error)?;
            let collected = statement
                .query_map([checkout_id], read_review_round)
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?;
            collected
        };
        Ok(rounds)
    }

    /// Advances a round's status, rejecting a move out of the state it is actually in.
    pub fn set_review_round_status(
        &self,
        id: &str,
        checkout_id: &str,
        from: &str,
        to: &str,
    ) -> Result<ReviewRound, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let changed = connection
            .execute(
                "UPDATE review_rounds SET status = ?1, updated_at = ?2
                 WHERE id = ?3 AND checkout_id = ?4 AND status = ?5",
                params![to, timestamp(), id, checkout_id, from],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("review round is not in the expected state".into());
        }
        connection
            .query_row(
                "SELECT id, checkout_id, session_id, status, marker, note_ids, created_at, updated_at
                 FROM review_rounds WHERE id = ?1",
                [id],
                read_review_round,
            )
            .map_err(db_error)
    }

    pub fn set_review_round_session(
        &self,
        id: &str,
        checkout_id: &str,
        session_id: &str,
    ) -> Result<ReviewRound, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let changed = connection
            .execute(
                "UPDATE review_rounds SET session_id = ?1, updated_at = ?2
                 WHERE id = ?3 AND checkout_id = ?4",
                params![session_id, timestamp(), id, checkout_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("review round does not belong to the requested checkout".into());
        }
        connection
            .query_row(
                "SELECT id, checkout_id, session_id, status, marker, note_ids, created_at, updated_at
                 FROM review_rounds WHERE id = ?1",
                [id],
                read_review_round,
            )
            .map_err(db_error)
    }

    /// Puts every still-pending round of the checkout back to `queued`.
    pub fn requeue_review_rounds(&self, checkout_id: &str) -> Result<usize, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE review_rounds SET status = 'queued', updated_at = ?1
                 WHERE checkout_id = ?2 AND status = 'dispatching'",
                params![timestamp(), checkout_id],
            )
            .map_err(db_error)
    }

    fn read_review_note_by_id(
        &self,
        connection: &Connection,
        id: &str,
    ) -> Result<ReviewNote, String> {
        connection
            .query_row(
                &format!("SELECT {REVIEW_NOTE_COLUMNS} FROM review_notes WHERE id = ?1"),
                [id],
                read_review_note,
            )
            .map_err(db_error)
    }

    /// Anchor hashes of the requested notes, so the caller can compare and stamp drift.
    pub fn review_note_hashes(
        &self,
        checkout_id: &str,
        path: &str,
        ids: &[String],
    ) -> Result<Vec<(String, String)>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT id, code_hash FROM review_notes
                 WHERE checkout_id = ?1 AND path = ?2 AND id = ?3",
            )
            .map_err(db_error)?;
        let mut found = Vec::with_capacity(ids.len());
        for id in ids {
            let row = statement
                .query_row(params![checkout_id, path, id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .optional()
                .map_err(db_error)?;
            match row {
                Some((id, code_hash)) => found.push((id, code_hash)),
                None => return Err("review note does not belong to the requested checkout".into()),
            }
        }
        Ok(found)
    }
}

fn read_review_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewNote> {
    Ok(ReviewNote {
        id: row.get(0)?,
        checkout_id: row.get(1)?,
        path: row.get(2)?,
        side: row.get(3)?,
        line_start: row.get(4)?,
        line_end: row.get(5)?,
        content: row.get(6)?,
        code: row.get(7)?,
        status: row.get(8)?,
        code_hash: row.get(9)?,
        outdated: row.get(10)?,
        round_id: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn read_review_round(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewRound> {
    let note_ids: String = row.get(5)?;
    Ok(ReviewRound {
        id: row.get(0)?,
        checkout_id: row.get(1)?,
        session_id: row.get(2)?,
        status: row.get(3)?,
        marker: row.get(4)?,
        note_ids: serde_json::from_str(&note_ids).unwrap_or_default(),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn terminal_layout_session_ids(
    connection: &Connection,
    checkout_id: &str,
) -> Result<Vec<String>, String> {
    let checkout_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM checkouts WHERE id = ?1)",
            [checkout_id],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if !checkout_exists {
        return Err("checkout does not exist".into());
    }
    let mut statement = connection
        .prepare("SELECT id FROM sessions WHERE checkout_id = ?1 ORDER BY rowid")
        .map_err(db_error)?;
    let session_ids = statement
        .query_map([checkout_id], |row| row.get::<_, String>(0))
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(session_ids)
}

fn transfer_checkout_metadata(
    transaction: &Transaction<'_>,
    previous_checkout_id: &str,
    next_checkout_id: &str,
) -> Result<(), String> {
    transaction
        .execute(
            "UPDATE sessions SET checkout_id = ?1 WHERE checkout_id = ?2",
            params![next_checkout_id, previous_checkout_id],
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "UPDATE checkout_terminal_layouts SET checkout_id = ?1 WHERE checkout_id = ?2",
            params![next_checkout_id, previous_checkout_id],
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "UPDATE viewed_files SET checkout_id = ?1 WHERE checkout_id = ?2",
            params![next_checkout_id, previous_checkout_id],
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "UPDATE review_notes SET checkout_id = ?1 WHERE checkout_id = ?2",
            params![next_checkout_id, previous_checkout_id],
        )
        .map_err(db_error)?;
    let checkout_ui_state = transaction
        .query_row(
            "SELECT state_json FROM checkout_ui_states WHERE checkout_id = ?1",
            [previous_checkout_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?;
    if let Some(serialized) = checkout_ui_state {
        let mut state = serde_json::from_str::<CheckoutUiState>(&serialized).ok();
        if let Some(state) = state
            .as_mut()
            .filter(|state| validate_checkout_ui_state(previous_checkout_id, state))
        {
            if let Some(document) = state.document.as_mut() {
                document.checkout_id = next_checkout_id.to_string();
            }
            let serialized = serde_json::to_string(state).map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "UPDATE checkout_ui_states SET checkout_id = ?1, state_json = ?2 WHERE checkout_id = ?3",
                    params![next_checkout_id, serialized, previous_checkout_id],
                )
                .map_err(db_error)?;
        } else {
            transaction
                .execute(
                    "DELETE FROM checkout_ui_states WHERE checkout_id = ?1",
                    [previous_checkout_id],
                )
                .map_err(db_error)?;
        }
    }
    if get_preference(transaction, ACTIVE_CHECKOUT)?.as_deref() == Some(previous_checkout_id) {
        set_preference(transaction, ACTIVE_CHECKOUT, Some(next_checkout_id))?;
    }
    Ok(())
}

fn validate_terminal_layout(
    connection: &Connection,
    checkout_id: &str,
    layout: &CheckoutTerminalLayout,
) -> Result<(), String> {
    let session_ids = terminal_layout_session_ids(connection, checkout_id)?;
    let session_ids = session_ids.into_iter().collect();
    layout.validate(&session_ids)
}

/// Brings one checkout's stored layout back in line with the sessions it holds now.
///
/// A checkout with no stored layout needs nothing here: the next read builds one from its
/// sessions, which is the same shape this would have written.
fn reconcile_stored_layout(transaction: &Transaction<'_>, checkout_id: &str) -> Result<(), String> {
    let Some(serialized) = transaction
        .query_row(
            "SELECT layout_json FROM checkout_terminal_layouts WHERE checkout_id = ?1",
            [checkout_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?
    else {
        return Ok(());
    };
    let mut layout: CheckoutTerminalLayout = serde_json::from_str(&serialized)
        .map_err(|error| format!("saved terminal layout is invalid: {error}"))?;
    layout.reconcile_sessions(&terminal_layout_session_ids(transaction, checkout_id)?);
    let reconciled = serde_json::to_string(&layout).map_err(|error| error.to_string())?;
    if reconciled == serialized {
        return Ok(());
    }
    transaction
        .execute(
            "UPDATE checkout_terminal_layouts SET layout_json = ?1 WHERE checkout_id = ?2",
            params![reconciled, checkout_id],
        )
        .map_err(db_error)?;
    Ok(())
}

fn safe_checkout_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn safe_review_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && path.ends_with(".md")
        && !path.contains('\\')
        && !path.contains('\0')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn validate_checkout_ui_state(checkout_id: &str, state: &CheckoutUiState) -> bool {
    state.version == 1
        && matches!(state.main_view.as_str(), "terminal" | "document")
        && matches!(state.inspector_tab.as_str(), "files" | "changes")
        && state.document.as_ref().is_none_or(|document| {
            document.checkout_id == checkout_id
                && matches!(document.origin.as_str(), "checkout" | "review")
                && if document.origin == "review" {
                    safe_review_relative_path(&document.path)
                } else {
                    safe_checkout_relative_path(&document.path)
                }
                && matches!(document.source.as_str(), "file" | "change")
                && matches!(document.mode.as_str(), "diff" | "view" | "code")
                && (document.source != "file" || document.mode != "diff")
                && (document.origin != "review"
                    || (document.source == "file" && document.mode != "diff"))
        })
        && (state.main_view != "document" || state.document.is_some())
        // The whole-change-set diff names no file, so a document alongside it is contradictory.
        && (!state.diff_all_files || (state.main_view == "terminal" && state.document.is_none()))
        && state
            .selected_file_path
            .as_ref()
            .is_none_or(|path| safe_checkout_relative_path(path))
        && state
            .selected_change_path
            .as_ref()
            .is_none_or(|path| safe_checkout_relative_path(path))
        && state.expanded_directories.len() <= 2000
        && state
            .expanded_directories
            .iter()
            .all(|path| safe_checkout_relative_path(path))
        && state.files_scroll_top <= 10_000_000
        && state.changes_scroll_top <= 10_000_000
        && state.document_scroll_top <= 10_000_000
        && state.document_scroll_left <= 10_000_000
        && state.diff_scroll_top <= 10_000_000
}

/// Writes the whole schema into a file that has none and stamps it with `SCHEMA_VERSION`.
///
/// There is no ladder: a file stamped with this build's version is left untouched, a file
/// with no stamp is written from scratch, and a file stamped with anything else is refused
/// rather than upgraded. Refusing is the point. The alternative was to reshape a database
/// this build did not write, and losing the repos someone registered is worse than asking
/// them to delete a file they were told to delete.
fn create_schema(connection: &Connection) -> Result<(), String> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_error)?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    if version != 0 {
        return Err(format!(
            "the workspace database is schema version {version} and this build speaks version \
             {SCHEMA_VERSION}: update by deleting {} and opening the app again, which starts an \
             empty workspace",
            connection.path().unwrap_or("the workspace database"),
        ));
    }

    let transaction = connection.unchecked_transaction().map_err(db_error)?;
    transaction
        .execute_batch(
            "CREATE TABLE repos (
                id TEXT PRIMARY KEY NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('git', 'plain')),
                name TEXT NOT NULL,
                root TEXT NOT NULL,
                default_branch TEXT,
                position INTEGER NOT NULL UNIQUE,
                created_at TEXT NOT NULL,
                last_opened_at TEXT NOT NULL
            );
            CREATE TABLE checkouts (
                id TEXT PRIMARY KEY NOT NULL,
                repo_id TEXT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                canonical_path TEXT NOT NULL UNIQUE,
                is_primary INTEGER NOT NULL CHECK (is_primary IN (0, 1)),
                branch TEXT,
                head TEXT,
                ahead_of_default INTEGER,
                changed_files INTEGER NOT NULL DEFAULT 0,
                is_missing INTEGER NOT NULL DEFAULT 0,
                is_archived INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
                position INTEGER NOT NULL,
                UNIQUE (repo_id, position)
            );
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY NOT NULL,
                checkout_id TEXT NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                session_type TEXT NOT NULL CHECK (session_type IN ('shell', 'nvim', 'server', 'custom', 'agent')),
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'inactive' CHECK (status IN ('active', 'inactive'))
            );
            CREATE TABLE recent_paths (
                canonical_path TEXT PRIMARY KEY NOT NULL,
                last_opened_at TEXT NOT NULL
            );
            CREATE TABLE preferences (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE TABLE checkout_terminal_layouts (
                checkout_id TEXT PRIMARY KEY NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                layout_json TEXT NOT NULL
            );
            CREATE TABLE viewed_files (
                checkout_id TEXT NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                viewed_at TEXT NOT NULL,
                PRIMARY KEY (checkout_id, path)
            );
            CREATE TABLE checkout_ui_states (
                checkout_id TEXT PRIMARY KEY NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                state_json TEXT NOT NULL
            );
            CREATE TABLE review_notes (
                id TEXT PRIMARY KEY NOT NULL,
                checkout_id TEXT NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                side TEXT NOT NULL CHECK (side IN ('old', 'new')),
                line_start INTEGER NOT NULL,
                line_end INTEGER,
                content TEXT NOT NULL,
                code TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('draft', 'sent', 'resolved')),
                code_hash TEXT NOT NULL,
                outdated INTEGER NOT NULL DEFAULT 0 CHECK (outdated IN (0, 1)),
                round_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX review_notes_checkout_path ON review_notes (checkout_id, path);
            CREATE INDEX review_notes_round ON review_notes (round_id);
            CREATE TABLE review_rounds (
                id TEXT PRIMARY KEY NOT NULL,
                checkout_id TEXT NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                session_id TEXT,
                status TEXT NOT NULL CHECK (
                    status IN ('queued', 'dispatching', 'dispatched', 'acked')
                ),
                marker TEXT NOT NULL,
                note_ids TEXT NOT NULL,
                prompt TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX review_rounds_checkout ON review_rounds (checkout_id, created_at);",
        )
        .map_err(db_error)?;
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(db_error)?;
    transaction.commit().map_err(db_error)
}

fn load_workspace(connection: &Connection) -> Result<WorkspaceState, String> {
    let mut repos_statement = connection
        .prepare(
            "SELECT id, kind, name, root, default_branch, created_at, last_opened_at
             FROM repos ORDER BY position",
        )
        .map_err(db_error)?;
    let repo_rows = repos_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(db_error)?;
    let repo_rows = repo_rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?;
    drop(repos_statement);

    let mut repos = Vec::with_capacity(repo_rows.len());
    let mut archived_worktrees = Vec::new();
    for (id, kind, name, root, default_branch, created_at, last_opened_at) in repo_rows {
        let mut checkouts_statement = connection
            .prepare(
                "SELECT id, path, canonical_path, is_primary, branch, head, ahead_of_default, changed_files, is_missing, is_archived
                 FROM checkouts WHERE repo_id = ?1 ORDER BY position",
            )
            .map_err(db_error)?;
        let checkout_rows = checkouts_statement
            .query_map([&id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<u32>>(6)?,
                    row.get::<_, u32>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, bool>(9)?,
                ))
            })
            .map_err(db_error)?;
        let checkout_rows = checkout_rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        drop(checkouts_statement);

        let mut checkouts = Vec::with_capacity(checkout_rows.len());
        for (
            checkout_id,
            path,
            canonical_path,
            is_primary,
            branch,
            head,
            ahead_of_default,
            changed_files,
            is_missing,
            is_archived,
        ) in checkout_rows
        {
            // An archived worktree is registered and alive; it is off the panel, not gone.
            // The two lists are one pass over one set of rows, so the panel and the list
            // of what could come back to it can never disagree.
            if is_archived {
                archived_worktrees.push(ArchivedCheckout {
                    id: checkout_id,
                    repo_id: id.clone(),
                    path,
                    branch,
                });
                continue;
            }
            let mut sessions_statement = connection
                .prepare(
                    "SELECT id, session_type, name, created_at, status FROM sessions WHERE checkout_id = ?1 ORDER BY rowid",
                )
                .map_err(db_error)?;
            let sessions = sessions_statement
                .query_map([&checkout_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(db_error)?
                .map(|row| {
                    let (id, session_type, name, created_at, status) = row?;
                    Ok(Session {
                        id,
                        session_type: parse_session_type(&session_type).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(std::io::Error::other(error)),
                            )
                        })?,
                        checkout_id: checkout_id.clone(),
                        name,
                        created_at,
                        status: match status.as_str() {
                            "active" => SessionStatus::Active,
                            "inactive" => SessionStatus::Inactive,
                            _ => return Err(rusqlite::Error::InvalidQuery),
                        },
                    })
                })
                .collect::<Result<Vec<_>, rusqlite::Error>>()
                .map_err(db_error)?;
            checkouts.push(Checkout {
                id: checkout_id,
                repo_id: id.clone(),
                is_missing: is_missing || !PathBuf::from(&canonical_path).is_dir(),
                path,
                canonical_path,
                is_primary,
                branch,
                head,
                ahead_of_default,
                changed_files,
                sessions,
            });
        }

        repos.push(Repo {
            id,
            kind: parse_repo_kind(&kind)?,
            name,
            root,
            default_branch,
            checkouts,
            created_at,
            last_opened_at,
        });
    }

    Ok(WorkspaceState {
        repos,
        archived_worktrees,
        active_checkout_id: get_preference(connection, ACTIVE_CHECKOUT)?,
        active_session_id: get_preference(connection, ACTIVE_SESSION)?,
    })
}

fn parse_repo_kind(kind: &str) -> Result<RepoKind, String> {
    match kind {
        "git" => Ok(RepoKind::Git),
        "plain" => Ok(RepoKind::Plain),
        _ => Err(format!("unknown repository kind: {kind}")),
    }
}

fn parse_session_type(session_type: &str) -> Result<SessionType, String> {
    match session_type {
        "shell" => Ok(SessionType::Shell),
        "nvim" => Ok(SessionType::Nvim),
        "server" => Ok(SessionType::Server),
        "custom" => Ok(SessionType::Custom),
        "agent" => Ok(SessionType::Agent),
        _ => Err(format!("unknown session type: {session_type}")),
    }
}

fn session_type_name(session_type: &SessionType) -> &'static str {
    match session_type {
        SessionType::Shell => "shell",
        SessionType::Nvim => "nvim",
        SessionType::Server => "server",
        SessionType::Custom => "custom",
        SessionType::Agent => "agent",
    }
}

fn get_preference(connection: &Connection, key: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT value FROM preferences WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)
}

fn set_preference(
    transaction: &Transaction<'_>,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    if let Some(value) = value {
        transaction
            .execute(
                "INSERT INTO preferences (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(db_error)?;
    } else {
        transaction
            .execute("DELETE FROM preferences WHERE key = ?1", [key])
            .map_err(db_error)?;
    }
    Ok(())
}

pub fn timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.to_string()
}

fn db_error(error: rusqlite::Error) -> String {
    format!("SQLite operation failed: {error}")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use rusqlite::{params, Connection};
    use tempfile::tempdir;

    use crate::domain::{
        review::review_anchor_hash,
        terminal_layout::{CheckoutTerminalLayout, TerminalLayoutNode, TerminalLayoutTab},
        workspace::{
            checkout_id_for_path, repo_id_for_path, Checkout, Repo, RepoKind, Session,
            SessionStatus, SessionType,
        },
    };

    use super::{
        AppLayoutState, CheckoutUiState, Database, PersistedDocument, ReviewNote,
        APP_LAYOUT_VERSION, SCHEMA_VERSION,
    };

    fn plain_repo(path: &Path, now: &str) -> Repo {
        Repo::plain(path, now).expect("plain repo")
    }

    /// A file with no stamp gets the whole schema, a file already stamped with this build's
    /// version is left alone, and a file from any other version is refused by name.
    #[test]
    fn a_fresh_file_gets_the_whole_schema_and_a_foreign_one_is_refused() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("workspace.sqlite3");
        let database = Database::open(&path).expect("fresh database");
        {
            let connection = database.connection.lock().unwrap();
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
            let tables: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
                     ('repos', 'checkouts', 'sessions', 'recent_paths', 'preferences',
                      'checkout_terminal_layouts', 'viewed_files', 'checkout_ui_states',
                      'review_notes', 'review_rounds')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(tables, 10);
            // Reopening what this build already wrote has nothing left to do.
            super::create_schema(&connection).expect("a stamped file is left alone");
        }

        let foreign_path = temp.path().join("foreign.sqlite3");
        Connection::open(&foreign_path)
            .unwrap()
            .pragma_update(None, "user_version", 3)
            .unwrap();
        let error = Database::open(&foreign_path)
            .err()
            .expect("a file from another schema version is refused");
        assert!(error.contains("schema version 3"), "{error}");
        assert!(error.contains(&SCHEMA_VERSION.to_string()), "{error}");
        assert!(error.contains("deleting"), "{error}");
    }

    /// A session outlives nothing: its PTY died with the process, so reopening drops the rows
    /// instead of letting one dead session per launch pile up per checkout.
    #[test]
    fn reopening_drops_the_terminal_sessions_the_previous_process_owned() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("workspace.sqlite3");
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "now");
        let checkout_id = repo.checkouts[0].id.clone();
        let session = Session {
            id: "session:dead".into(),
            session_type: SessionType::Shell,
            checkout_id: checkout_id.clone(),
            name: "zsh".into(),
            created_at: "now".into(),
            status: SessionStatus::Active,
        };
        {
            let database = Database::open(&path).unwrap();
            database.register_plain_repo(repo).unwrap();
            database.add_terminal_session(&session).unwrap();
            database
                .save_terminal_layout(
                    &checkout_id,
                    &CheckoutTerminalLayout {
                        active_tab_id: Some("tab:dead".into()),
                        tabs: vec![TerminalLayoutTab {
                            id: "tab:dead".into(),
                            root: TerminalLayoutNode::Session {
                                session_id: session.id.clone(),
                            },
                        }],
                        session_order: vec![session.id.clone()],
                    },
                )
                .unwrap();
            assert_eq!(
                database.load_workspace().unwrap().repos[0].checkouts[0]
                    .sessions
                    .len(),
                1
            );
        }

        let database = Database::open(&path).unwrap();
        let restored = database.load_workspace().unwrap();
        assert!(restored.repos[0].checkouts[0].sessions.is_empty());
        assert_eq!(restored.active_session_id, None);
        assert_eq!(database.load_terminal_layout(&checkout_id).unwrap(), None);
    }

    /// D2-04: three line comments across two files, still drafts after a restart.
    ///
    /// A restart reopens the file, so anything held only in memory would be lost here; the
    /// drafts are asserted from the reopened handle rather than the one that wrote them.
    #[test]
    fn review_drafts_across_two_files_survive_a_database_restart() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("workspace.sqlite3");
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();

        let checkout_id = {
            let database = Database::open(&db_path).unwrap();
            let repo = plain_repo(&folder, "now");
            let checkout_id = repo.checkouts[0].id.clone();
            database.register_plain_repo(repo).unwrap();
            let now = "2026-09-26T00:00:00Z";
            // Two notes on one file and one on another: the batch spans files, not just lines.
            let notes = [("src/main.rs", 4), ("src/main.rs", 12), ("src/lib.rs", 7)];
            for (index, (path, line)) in notes.into_iter().enumerate() {
                let code = format!("let value{index} = {index};");
                database
                    .add_review_note(&ReviewNote {
                        id: format!("note:{index}"),
                        checkout_id: checkout_id.clone(),
                        path: path.into(),
                        side: "new".into(),
                        line_start: line,
                        line_end: None,
                        content: format!("draft {index}"),
                        code: code.clone(),
                        status: "draft".into(),
                        code_hash: review_anchor_hash(&code),
                        outdated: false,
                        round_id: None,
                        created_at: now.into(),
                        updated_at: now.into(),
                    })
                    .unwrap();
            }
            checkout_id
        };

        let reopened = Database::open(&db_path).expect("reopened database");
        let drafts = reopened
            .review_notes(&checkout_id)
            .expect("drafts after restart");
        assert_eq!(drafts.len(), 3, "{drafts:?}");
        assert!(drafts.iter().all(|note| note.status == "draft"));
        assert!(drafts.iter().all(|note| !note.outdated));
        assert_eq!(
            drafts
                .iter()
                .filter(|note| note.path == "src/main.rs")
                .count(),
            2
        );
        assert_eq!(
            drafts
                .iter()
                .filter(|note| note.path == "src/lib.rs")
                .count(),
            1
        );
        // The unbounded line is what still has to point at the right code after the restart.
        assert!(drafts
            .iter()
            .any(|note| note.line_start == 12 && note.line_end.is_none()));
        assert!(drafts
            .iter()
            .all(|note| !note.code_hash.is_empty() && !note.code.is_empty()));
    }

    #[test]
    fn ui_state_round_trips_and_discards_invalid_saved_data() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("ui-state.sqlite3");
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "now");
        let checkout_id = repo.checkouts[0].id.clone();
        let database = Database::open(&db_path).unwrap();
        database.register_plain_repo(repo).unwrap();

        let layout = AppLayoutState {
            mode: "split".into(),
            sidebar_width: 340,
            inspector_width: 420,
            preview_width: 720,
            ..AppLayoutState::default()
        };
        database.save_app_layout(&layout).unwrap();
        let state = CheckoutUiState {
            document: Some(PersistedDocument {
                checkout_id: checkout_id.clone(),
                path: "docs/guide.md".into(),
                origin: "checkout".into(),
                source: "file".into(),
                mode: "view".into(),
            }),
            main_view: "document".into(),
            inspector_tab: "changes".into(),
            selected_file_path: Some("docs/guide.md".into()),
            selected_change_path: Some("src/main.rs".into()),
            expanded_directories: vec!["docs".into(), "src".into()],
            files_scroll_top: 640,
            changes_scroll_top: 480,
            document_scroll_top: 320,
            document_scroll_left: 4,
            diff_scroll_top: 900,
            ..CheckoutUiState::default()
        };
        database
            .save_checkout_ui_state(&checkout_id, &state)
            .unwrap();
        assert_eq!(database.load_app_layout().unwrap(), layout);
        assert_eq!(
            database.load_checkout_ui_state(&checkout_id).unwrap(),
            state
        );

        {
            let connection = database.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE checkout_ui_states SET state_json = ?1 WHERE checkout_id = ?2",
                    params![r#"{"version":1,"document":{"checkoutId":"checkout:wrong","path":"../secret","source":"file","mode":"view"},"mainView":"document","inspectorTab":"files","selectedFilePath":null,"selectedChangePath":null,"expandedDirectories":[],"filesScrollTop":0,"documentScrollTop":0,"documentScrollLeft":0}"#, checkout_id],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE preferences SET value = ?1 WHERE key = 'ui_layout_v1'",
                    [r#"{"version":2,"sidebarWidth":300,"inspectorWidth":320,"previewWidth":500}"#],
                )
                .unwrap();
        }
        assert_eq!(
            database.load_checkout_ui_state(&checkout_id).unwrap(),
            CheckoutUiState::default()
        );
        assert_eq!(
            database.load_app_layout().unwrap(),
            AppLayoutState::default()
        );
        let connection = database.connection.lock().unwrap();
        let invalid_checkout_state_remains: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkout_ui_states WHERE checkout_id = ?1)",
                [&checkout_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!invalid_checkout_state_remains);
        let invalid_global_state_remains: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM preferences WHERE key = 'ui_layout_v1')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!invalid_global_state_remains);
    }

    #[test]
    fn a_review_document_survives_a_restart_with_its_origin() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();
        let checkout_id = plain_repo(&folder, "now").checkouts[0].id.clone();
        let db_path = temp.path().join("ui-state.sqlite3");
        {
            let database = Database::open(&db_path).unwrap();
            database
                .register_plain_repo(plain_repo(&folder, "now"))
                .unwrap();
            database
                .save_checkout_ui_state(
                    &checkout_id,
                    &CheckoutUiState {
                        document: Some(PersistedDocument {
                            checkout_id: checkout_id.clone(),
                            path: "2026-03-14-1532.md".into(),
                            origin: "review".into(),
                            source: "file".into(),
                            mode: "view".into(),
                        }),
                        main_view: "document".into(),
                        ..CheckoutUiState::default()
                    },
                )
                .unwrap();
        }

        let reopened = Database::open(&db_path).unwrap();
        assert_eq!(
            reopened
                .load_checkout_ui_state(&checkout_id)
                .unwrap()
                .document
                .unwrap()
                .origin,
            "review"
        );
    }

    #[test]
    fn a_document_saved_without_an_origin_is_dropped_not_guessed() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "now");
        let checkout_id = repo.checkouts[0].id.clone();
        let database = Database::open(temp.path().join("ui-state.sqlite3")).unwrap();
        database.register_plain_repo(repo).unwrap();
        let old_shape = serde_json::to_string(&serde_json::json!({
            "version": 1,
            "document": {
                "checkoutId": checkout_id.clone(),
                "path": "README.md",
                "source": "file",
                "mode": "view"
            },
            "mainView": "document",
            "diffAllFiles": false,
            "inspectorTab": "files",
            "selectedFilePath": null,
            "selectedChangePath": null,
            "expandedDirectories": [],
            "filesScrollTop": 0,
            "changesScrollTop": 0,
            "documentScrollTop": 0,
            "documentScrollLeft": 0,
            "diffScrollTop": 0
        }))
        .unwrap();
        database
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO checkout_ui_states (checkout_id, state_json) VALUES (?1, ?2)",
                params![checkout_id, old_shape],
            )
            .unwrap();

        assert_eq!(
            database.load_checkout_ui_state(&checkout_id).unwrap(),
            CheckoutUiState::default()
        );
    }

    #[test]
    fn review_target_defaults_to_markdown_and_is_stored_per_checkout() {
        let database = Database::open_in_memory().unwrap();
        assert_eq!(database.review_target("checkout:one").unwrap(), "markdown");
        database
            .set_review_target("checkout:one", "opencode")
            .unwrap();
        assert_eq!(database.review_target("checkout:one").unwrap(), "opencode");
        assert_eq!(database.review_target("checkout:two").unwrap(), "markdown");
        assert!(database.set_review_target("checkout:one", "codex").is_err());
        database
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('review_target:checkout:three', 'codex')",
                [],
            )
            .unwrap();
        assert_eq!(
            database.review_target("checkout:three").unwrap(),
            "markdown"
        );
    }

    #[test]
    fn app_layout_normalizes_dimensions_and_unknown_modes() {
        let layout = serde_json::from_value::<AppLayoutState>(serde_json::json!({
            "version": 5,
            "mode": "unknown",
            "sidebarWidth": 260,
            "inspectorWidth": 320,
            "previewWidth": 1200
        }))
        .unwrap()
        .normalized();
        assert_eq!(
            layout,
            AppLayoutState {
                mode: "focus".into(),
                sidebar_width: 260,
                inspector_width: 320,
                preview_width: 900,
                ..Default::default()
            }
        );

        let out_of_range = serde_json::from_value::<AppLayoutState>(serde_json::json!({
            "version": 5,
            "sidebarWidth": 220,
            "inspectorWidth": 560,
            "previewWidth": 200
        }))
        .unwrap()
        .normalized();
        assert_eq!(
            out_of_range,
            AppLayoutState {
                sidebar_width: 240,
                inspector_width: 480,
                preview_width: 260,
                ..Default::default()
            }
        );
    }

    #[test]
    fn a_layout_row_from_before_the_settings_file_moved_out_is_refused_by_name() {
        // Widths are the only thing this row holds now, so a row written when it also held the
        // preferences is a shape this build cannot read. It is discarded whole rather than half
        // honoured: a pane width nobody asked for is a smaller cost than one that silently means
        // something else than it did.
        let layout = serde_json::from_value::<AppLayoutState>(serde_json::json!({
            "version": 4,
            "mode": "split",
            "sidebarWidth": 340,
            "inspectorWidth": 420,
            "previewWidth": 720,
            "terminalScrollbar": "auto",
            "zoom": 1.2
        }))
        .unwrap()
        .normalized();
        assert_ne!(layout.version, APP_LAYOUT_VERSION);
    }

    #[test]
    fn checkout_ui_state_defaults_new_scroll_fields_for_existing_saved_state() {
        let old_state = serde_json::json!({
            "version": 1,
            "document": null,
            "mainView": "terminal",
            "inspectorTab": "files",
            "selectedFilePath": null,
            "selectedChangePath": null,
            "expandedDirectories": [],
            "filesScrollTop": 64,
            "documentScrollTop": 32,
            "documentScrollLeft": 4
        });
        let state: CheckoutUiState = serde_json::from_value(old_state).unwrap();
        assert_eq!(state.files_scroll_top, 64);
        assert_eq!(state.changes_scroll_top, 0);
        assert_eq!(state.diff_scroll_top, 0);
        // The whole-change-set diff postdates this saved shape, so it reads as not shown.
        assert!(!state.diff_all_files);

        let invalid = CheckoutUiState {
            changes_scroll_top: 10_000_001,
            ..CheckoutUiState::default()
        };
        assert!(!super::validate_checkout_ui_state("checkout:one", &invalid));
    }

    #[test]
    fn the_whole_change_set_view_survives_a_save_and_load() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "now");
        let checkout_id = repo.checkouts[0].id.clone();
        let database = Database::open(temp.path().join("ui-state.sqlite3")).unwrap();
        database.register_plain_repo(repo).unwrap();

        let all_changes = CheckoutUiState {
            diff_all_files: true,
            ..CheckoutUiState::default()
        };
        assert!(super::validate_checkout_ui_state(
            &checkout_id,
            &all_changes
        ));
        database
            .save_checkout_ui_state(&checkout_id, &all_changes)
            .unwrap();
        assert_eq!(
            database.load_checkout_ui_state(&checkout_id).unwrap(),
            all_changes
        );

        // It names no file, so a state that also carries one is contradictory, not a guess.
        let contradictory = CheckoutUiState {
            diff_all_files: true,
            main_view: "document".into(),
            document: Some(PersistedDocument {
                checkout_id: checkout_id.clone(),
                path: "src/main.rs".into(),
                origin: "checkout".into(),
                source: "change".into(),
                mode: "diff".into(),
            }),
            ..CheckoutUiState::default()
        };
        assert!(!super::validate_checkout_ui_state(
            &checkout_id,
            &contradictory
        ));
    }

    #[test]
    fn checkout_ui_state_moves_with_relocated_plain_checkout_and_cascades_on_delete() {
        let temp = tempdir().unwrap();
        let old_path = temp.path().join("old");
        let new_path = temp.path().join("new");
        fs::create_dir(&old_path).unwrap();
        fs::create_dir(&new_path).unwrap();
        let old_repo = plain_repo(&old_path, "1");
        let new_repo = plain_repo(&new_path, "2");
        let old_checkout_id = old_repo.checkouts[0].id.clone();
        let new_checkout_id = new_repo.checkouts[0].id.clone();
        let database = Database::open_in_memory().unwrap();
        database.register_plain_repo(old_repo.clone()).unwrap();
        let ui_state = CheckoutUiState {
            document: Some(PersistedDocument {
                checkout_id: old_checkout_id.clone(),
                path: "README.md".into(),
                origin: "checkout".into(),
                source: "file".into(),
                mode: "view".into(),
            }),
            main_view: "document".into(),
            ..CheckoutUiState::default()
        };
        database
            .save_checkout_ui_state(&old_checkout_id, &ui_state)
            .unwrap();
        fs::remove_dir(&old_path).unwrap();

        database
            .relocate_plain_checkout(&old_repo.id, &old_checkout_id, &new_repo)
            .unwrap();
        let mut moved_state = ui_state;
        moved_state.document.as_mut().unwrap().checkout_id = new_checkout_id.clone();
        assert_eq!(
            database.load_checkout_ui_state(&new_checkout_id).unwrap(),
            moved_state
        );
        assert_eq!(
            database
                .load_checkout_ui_state(&old_checkout_id)
                .unwrap_err(),
            "checkout does not exist"
        );

        database
            .connection
            .lock()
            .unwrap()
            .execute("DELETE FROM repos WHERE id = ?1", [&new_repo.id])
            .unwrap();
        let connection = database.connection.lock().unwrap();
        let state_remains: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkout_ui_states WHERE checkout_id = ?1)",
                [&new_checkout_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!state_remains);
    }

    #[test]
    fn viewed_file_progress_is_checkout_scoped_and_survives_database_restart() {
        let temp = tempdir().expect("temporary directory");
        let db_path = temp.path().join("workspace.sqlite3");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();

        let (first_id, second_id) = {
            let database = Database::open(&db_path).expect("database");
            let first_state = database
                .register_plain_repo(plain_repo(&first, "1"))
                .expect("first checkout");
            let second_state = database
                .register_plain_repo(plain_repo(&second, "2"))
                .expect("second checkout");
            let first_id = first_state.repos[0].checkouts[0].id.clone();
            let second_id = second_state.repos[1].checkouts[0].id.clone();
            database
                .mark_file_viewed(&first_id, "src/main.rs")
                .expect("mark viewed");
            (first_id, second_id)
        };

        let reopened = Database::open(&db_path).expect("reopened database");
        assert_eq!(reopened.viewed_files(&first_id).unwrap(), ["src/main.rs"]);
        assert!(reopened.viewed_files(&second_id).unwrap().is_empty());
    }

    /// The workdir menu lists what this machine has had open, so the read has to come back in
    /// the order the menu shows — newest first — and inside the ten rows the writes keep.
    #[test]
    fn recent_paths_come_back_newest_first_and_capped_at_ten() {
        let database = Database::open_in_memory().expect("database");
        let mut newest_first = Vec::new();
        for index in 1..=12 {
            let temp = tempdir().expect("temporary directory");
            let folder = temp.path().join(format!("folder {index}"));
            fs::create_dir(&folder).unwrap();
            // The stamps are ISO-8601, so their text order is their time order.
            database
                .register_plain_repo(plain_repo(
                    &folder,
                    &format!("2026-01-{index:02}T00:00:00Z"),
                ))
                .expect("register folder");
        }
        for (index, recent) in database
            .list_recent_paths()
            .expect("recent paths")
            .iter()
            .enumerate()
        {
            let expected = 12 - index;
            assert!(
                recent
                    .canonical_path
                    .ends_with(&format!("folder {expected}")),
                "{recent:?}"
            );
            newest_first.push(recent.canonical_path.clone());
        }
        assert_eq!(newest_first.len(), 10);
    }

    #[test]
    fn plain_repo_order_selection_and_identity_survive_restart_without_duplicates() {
        let temp = tempdir().expect("temporary directory");
        let db_path = temp.path().join("workspace.sqlite3");
        let first = temp.path().join("first plain");
        let second = temp.path().join("second plain");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();

        let first_repo = plain_repo(&first, "1");
        let second_repo = plain_repo(&second, "2");
        {
            let database = Database::open(&db_path).expect("database");
            database
                .register_plain_repo(first_repo.clone())
                .expect("register first");
            database
                .register_plain_repo(second_repo)
                .expect("register second");
            let state = database
                .register_plain_repo(plain_repo(&first, "3"))
                .expect("reopen first");
            assert_eq!(state.repos.len(), 2);
            assert_eq!(state.repos[0].id, first_repo.id);
            assert_eq!(state.repos[1].name, "second plain");
            assert_eq!(
                state.active_checkout_id,
                Some(first_repo.checkouts[0].id.clone())
            );
        }

        let state = Database::open(&db_path)
            .expect("restart database")
            .load_workspace()
            .expect("restore workspace");
        assert_eq!(state.repos.len(), 2);
        assert_eq!(state.repos[0].id, first_repo.id);
        assert_eq!(
            state.active_checkout_id,
            Some(first_repo.checkouts[0].id.clone())
        );
        assert_eq!(
            state.repos[0].kind,
            crate::domain::workspace::RepoKind::Plain
        );
        assert_eq!(state.repos[0].checkouts.len(), 1);
    }

    #[test]
    fn window_geometry_and_maximized_preference_restore_from_sqlite() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("workspace.sqlite3");
        let geometry = super::WindowGeometry {
            x: -1200,
            y: 80,
            width: 1440,
            height: 920,
        };
        {
            let database = Database::open(&path).unwrap();
            database.set_window_geometry(geometry).unwrap();
            database.set_window_maximized(true).unwrap();
            assert!(database
                .set_window_geometry(super::WindowGeometry {
                    width: 800,
                    ..geometry
                })
                .is_err());
        }

        let restored = Database::open(path).unwrap();
        assert_eq!(restored.window_geometry().unwrap(), Some(geometry));
        assert!(restored.window_maximized().unwrap());
    }

    #[test]
    fn sec_06_session_lookup_only_returns_ids_registered_in_sqlite() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "now");
        let database = Database::open_in_memory().unwrap();
        database.register_plain_repo(repo.clone()).unwrap();
        let session = Session {
            id: "session:known".into(),
            session_type: SessionType::Shell,
            checkout_id: repo.checkouts[0].id.clone(),
            name: "shell".into(),
            created_at: "now".into(),
            status: SessionStatus::Active,
        };
        database.add_terminal_session(&session).unwrap();

        assert_eq!(
            database.terminal_session_checkout(&session.id).unwrap(),
            Some(repo.checkouts[0].id.clone())
        );
        assert_eq!(
            database
                .terminal_session_checkout("session:unknown")
                .unwrap(),
            None
        );
    }

    #[test]
    fn checkout_order_is_restored_from_its_persisted_position() {
        let temp = tempdir().expect("temporary directory");
        let db_path = temp.path().join("workspace.sqlite3");
        let root = temp.path().join("repo");
        let second_checkout = temp.path().join("feature");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&second_checkout).unwrap();
        let repo = plain_repo(&root, "1");
        let database = Database::open(&db_path).expect("database");
        database
            .register_plain_repo(repo.clone())
            .expect("register repo");
        {
            let connection = database.connection.lock().unwrap();
            connection
                .execute("UPDATE repos SET kind = 'git' WHERE id = ?1", [&repo.id])
                .unwrap();
            connection
                .execute(
                    "UPDATE checkouts SET position = 1 WHERE id = ?1",
                    [&repo.checkouts[0].id],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO checkouts
                     (id, repo_id, path, canonical_path, is_primary, branch, head, ahead_of_default, changed_files, position)
                     VALUES ('checkout:feature', ?1, ?2, ?2, 0, 'feature', NULL, NULL, 0, 0)",
                    params![repo.id, second_checkout.canonicalize().unwrap().display().to_string()],
                )
                .unwrap();
        }

        let state = database.load_workspace().expect("restore workspace");
        assert_eq!(state.repos[0].kind, crate::domain::workspace::RepoKind::Git);
        assert_eq!(
            state.repos[0]
                .checkouts
                .iter()
                .map(|checkout| checkout.id.as_str())
                .collect::<Vec<_>>(),
            ["checkout:feature", repo.checkouts[0].id.as_str()]
        );
    }

    #[test]
    fn terminal_checkout_path_comes_from_persisted_non_missing_checkouts() {
        let temp = tempdir().expect("temporary directory");
        let folder = temp.path().join("checkout");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "now");
        let database = Database::open_in_memory().expect("database");
        database
            .register_plain_repo(repo.clone())
            .expect("register checkout");

        assert_eq!(
            database
                .terminal_checkout_path(&repo.checkouts[0].id)
                .unwrap(),
            folder.canonicalize().unwrap()
        );
        assert!(database
            .terminal_checkout_path("checkout:unregistered")
            .is_err());

        database
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE checkouts SET is_missing = 1 WHERE id = ?1",
                [&repo.checkouts[0].id],
            )
            .unwrap();
        assert!(database
            .terminal_checkout_path(&repo.checkouts[0].id)
            .is_err());
        database
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE checkouts SET is_missing = 0 WHERE id = ?1",
                [&repo.checkouts[0].id],
            )
            .unwrap();
        fs::remove_dir(&folder).unwrap();
        assert!(database
            .terminal_checkout_path(&repo.checkouts[0].id)
            .is_err());
    }

    #[test]
    fn checkout_layouts_persist_and_heal_sessions_owned_by_another_checkout() {
        let temp = tempdir().expect("temporary directory");
        let first_path = temp.path().join("first");
        let second_path = temp.path().join("second");
        fs::create_dir(&first_path).unwrap();
        fs::create_dir(&second_path).unwrap();
        let first_repo = plain_repo(&first_path, "1");
        let second_repo = plain_repo(&second_path, "2");
        let database_path = temp.path().join("workspace.sqlite3");
        let database = Database::open(&database_path).expect("database");
        database
            .register_plain_repo(first_repo.clone())
            .expect("register first checkout");
        database
            .register_plain_repo(second_repo.clone())
            .expect("register second checkout");
        let first_session = Session {
            id: "session:first".into(),
            session_type: SessionType::Shell,
            checkout_id: first_repo.checkouts[0].id.clone(),
            name: "zsh".into(),
            created_at: "now".into(),
            status: SessionStatus::Active,
        };
        let second_session = Session {
            id: "session:second".into(),
            checkout_id: second_repo.checkouts[0].id.clone(),
            ..first_session.clone()
        };
        let first_split_session = Session {
            id: "session:first-split".into(),
            ..first_session.clone()
        };
        database
            .add_terminal_session(&first_session)
            .expect("add first session");
        database
            .add_terminal_session(&second_session)
            .expect("add second session");
        database
            .add_terminal_session(&first_split_session)
            .expect("add split session");
        let layout = CheckoutTerminalLayout {
            active_tab_id: Some("tab:first".into()),
            tabs: vec![TerminalLayoutTab {
                id: "tab:first".into(),
                root: TerminalLayoutNode::Split {
                    id: "split:first".into(),
                    direction: crate::domain::terminal_layout::SplitDirection::Horizontal,
                    ratio: 0.5,
                    first: Box::new(TerminalLayoutNode::Session {
                        session_id: first_session.id.clone(),
                    }),
                    second: Box::new(TerminalLayoutNode::Session {
                        session_id: first_split_session.id.clone(),
                    }),
                },
            }],
            session_order: vec![first_session.id.clone(), first_split_session.id.clone()],
        };

        database
            .save_terminal_layout(&first_repo.checkouts[0].id, &layout)
            .expect("save checkout layout");
        assert_eq!(
            database
                .load_terminal_layout(&first_repo.checkouts[0].id)
                .expect("load layout"),
            Some(layout.clone())
        );
        let foreign_layout = CheckoutTerminalLayout {
            active_tab_id: Some("tab:second".into()),
            tabs: vec![TerminalLayoutTab {
                id: "tab:second".into(),
                root: TerminalLayoutNode::Session {
                    session_id: second_session.id.clone(),
                },
            }],
            session_order: vec![second_session.id],
        };
        database
            .save_terminal_layout(&first_repo.checkouts[0].id, &foreign_layout)
            .expect("save checkout layout heals stale session membership");
        let healed = database
            .load_terminal_layout(&first_repo.checkouts[0].id)
            .expect("load healed layout")
            .expect("healed layout");
        assert_eq!(
            healed.session_order,
            vec![first_session.id.clone(), first_split_session.id.clone()]
        );
        database
            .save_terminal_layout(&first_repo.checkouts[0].id, &layout)
            .expect("restore layout for persistence checks");
        assert!(database.load_terminal_layout("checkout:unknown").is_err());
        assert_eq!(
            database
                .load_terminal_layout(&first_repo.checkouts[0].id)
                .expect("failed save leaves old layout intact"),
            Some(layout.clone())
        );
        database
            .connection
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM sessions WHERE id = ?1",
                [&first_split_session.id],
            )
            .unwrap();
        let pruned = database
            .load_terminal_layout(&first_repo.checkouts[0].id)
            .expect("prune a removed historical session");
        assert_eq!(
            pruned.unwrap().tabs[0].root,
            TerminalLayoutNode::Session {
                session_id: first_session.id
            }
        );
        drop(database);

        // A restart is not a session: what the saved layout pointed at is gone, so the layout
        // goes with it instead of healing into a row of dead tabs.
        let reopened = Database::open(&database_path).expect("reopen database");
        assert_eq!(
            reopened
                .load_terminal_layout(&first_repo.checkouts[0].id)
                .expect("load layout from SQLite"),
            None
        );
    }

    #[test]
    fn moving_a_session_hands_the_row_to_a_sibling_worktree_and_takes_it_off_the_panels() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        let other = temp.path().join("other");
        let other_worktree = temp.path().join("other-worktree");
        for folder in [&root, &worktree, &other, &other_worktree] {
            fs::create_dir_all(folder).unwrap();
        }
        let (repo, worktree_id) = git_repo_with_worktree(&root, &worktree);
        let (other_repo, other_worktree_id) = git_repo_with_worktree(&other, &other_worktree);
        let root_id = repo.checkouts[0].id.clone();
        let other_root_id = other_repo.checkouts[0].id.clone();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_git_repo(repo, &worktree_id).unwrap();
        database
            .register_git_repo(other_repo, &other_worktree_id)
            .unwrap();
        let session = Session {
            id: "session:moved".into(),
            session_type: SessionType::Shell,
            checkout_id: root_id.clone(),
            name: "zsh".into(),
            created_at: "now".into(),
            status: SessionStatus::Active,
        };
        database.add_terminal_session(&session).unwrap();
        database
            .save_terminal_layout(
                &root_id,
                &CheckoutTerminalLayout {
                    active_tab_id: Some("tab:moved".into()),
                    tabs: vec![TerminalLayoutTab {
                        id: "tab:moved".into(),
                        root: TerminalLayoutNode::Session {
                            session_id: session.id.clone(),
                        },
                    }],
                    session_order: vec![session.id.clone()],
                },
            )
            .unwrap();

        // Another repository is a different Git directory, so it is never a destination.
        assert!(database
            .move_terminal_session(&session.id, &other_root_id)
            .is_err());
        assert_eq!(
            database.terminal_session_checkout(&session.id).unwrap(),
            Some(root_id.clone())
        );
        assert!(database
            .move_terminal_session(&session.id, &root_id)
            .is_err());
        assert!(database
            .move_terminal_session("session:unknown", &worktree_id)
            .is_err());

        let moved = database
            .move_terminal_session(&session.id, &worktree_id)
            .unwrap();

        let moved_repo = moved
            .repos
            .iter()
            .find(|item| item.checkouts.iter().any(|checkout| checkout.id == root_id))
            .unwrap();
        let source = moved_repo
            .checkouts
            .iter()
            .find(|item| item.id == root_id)
            .unwrap();
        let target = moved_repo
            .checkouts
            .iter()
            .find(|item| item.id == worktree_id)
            .unwrap();
        assert!(source.sessions.is_empty());
        assert_eq!(target.sessions.len(), 1);
        assert_eq!(target.sessions[0].id, session.id);
        // The worktree the terminal now belongs to is the one the window shows, so the files and
        // the changes on screen are that worktree's.
        assert_eq!(
            moved.active_checkout_id.as_deref(),
            Some(worktree_id.as_str())
        );
        assert_eq!(
            moved.active_session_id.as_deref(),
            Some(session.id.as_str())
        );
        // Both layouts follow the row: the source loses the pane, the destination gains one.
        assert_eq!(
            database.load_terminal_layout(&root_id).unwrap(),
            Some(CheckoutTerminalLayout {
                active_tab_id: None,
                tabs: vec![],
                session_order: vec![],
            })
        );
        // The destination had no stored layout, so it has none to repair: the pane its sessions
        // name is built on the next read, and the session it gained is in it.
        assert_eq!(database.load_terminal_layout(&worktree_id).unwrap(), None);
    }

    #[test]
    fn missing_directory_is_reported_but_kept_in_persisted_organization() {
        let temp = tempdir().expect("temporary directory");
        let db_path = temp.path().join("workspace.sqlite3");
        let folder = temp.path().join("removed later");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "1");
        Database::open(&db_path)
            .expect("database")
            .register_plain_repo(repo.clone())
            .expect("register repo");
        fs::remove_dir(&folder).unwrap();

        // Reading the organization never prunes: the row stays, and reports itself missing.
        // Only the startup prune, in `services::workspace::restore`, drops it.
        let state = Database::open(&db_path)
            .expect("restart database")
            .load_workspace()
            .expect("restore workspace");
        assert_eq!(state.repos.len(), 1);
        assert_eq!(state.repos[0].id, repo.id);
        assert!(state.repos[0].checkouts[0].is_missing);
    }

    /// A Git repository with its primary and every given worktree on disk, without running
    /// Git: the persistence layer only stores paths. The returned ids are the worktrees, in
    /// the order they were given.
    fn git_repo_with_worktrees(root: &Path, worktrees: &[&Path]) -> (Repo, Vec<String>) {
        let root = fs::canonicalize(root).expect("root");
        let repo_id = repo_id_for_path(&root.display().to_string());
        let mut checkouts = vec![Checkout::new(repo_id.clone(), &root, true).expect("primary")];
        let ids = worktrees
            .iter()
            .map(|worktree| {
                let worktree = fs::canonicalize(worktree).expect("worktree");
                let id = checkout_id_for_path(&worktree.display().to_string());
                checkouts.push(Checkout::new(repo_id.clone(), &worktree, false).expect("worktree"));
                id
            })
            .collect();
        let repo = Repo {
            id: repo_id,
            kind: RepoKind::Git,
            name: "repo".into(),
            root: root.display().to_string(),
            default_branch: None,
            checkouts,
            created_at: "1".into(),
            last_opened_at: "1".into(),
        };
        (repo, ids)
    }

    /// A Git repository whose primary is on disk and whose worktree is not, without running
    /// Git: the persistence layer only stores paths.
    fn git_repo_with_worktree(root: &Path, worktree: &Path) -> (Repo, String) {
        let root = fs::canonicalize(root).expect("root");
        let root = fs::canonicalize(root).expect("root");
        let root_path = root.display().to_string();
        let repo_id = repo_id_for_path(&root_path);
        let worktree_id = checkout_id_for_path(
            &fs::canonicalize(worktree)
                .expect("worktree")
                .display()
                .to_string(),
        );
        let repo = Repo {
            id: repo_id.clone(),
            kind: RepoKind::Git,
            name: "repo".into(),
            root: root_path,
            default_branch: None,
            checkouts: vec![
                Checkout::new(repo_id.clone(), &root, true).expect("primary"),
                Checkout::new(repo_id, worktree, false).expect("worktree"),
            ],
            created_at: "1".into(),
            last_opened_at: "1".into(),
        };
        (repo, worktree_id)
    }

    /// Archiving is the reversible half of leaving a workdir: the row leaves the panel and
    /// the worktree stays registered, so a repo root can put it back and the disk is
    /// untouched either way. Closing is the half that forgets.
    #[test]
    fn archiving_a_worktree_takes_the_row_off_the_panel_and_keeps_it_restorable() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(root.join("tracked.txt"), "kept\n").unwrap();
        let (repo, worktree_id) = git_repo_with_worktree(&root, &worktree);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_git_repo(repo, &worktree_id).unwrap();

        let archived = database.archive_checkout(&worktree_id).unwrap();

        // Off the panel, and named on the shelf the repo root reads, with the branch it is
        // known by rather than a bare path.
        assert_eq!(archived.repos[0].checkouts.len(), 1);
        assert!(archived.repos[0].checkouts[0].is_primary);
        assert_eq!(archived.archived_worktrees.len(), 1);
        assert_eq!(archived.archived_worktrees[0].id, worktree_id);
        assert_eq!(
            archived.archived_worktrees[0].repo_id,
            repo_id_for_path(&root.canonicalize().unwrap().display().to_string())
        );
        // The worktree was the selected one, so the selection follows it off the panel.
        assert_ne!(
            archived.active_checkout_id.as_deref(),
            Some(worktree_id.as_str())
        );
        assert_eq!(archived.active_session_id, None);
        // Nothing on disk moved: the directory and its file are exactly where they were.
        assert!(worktree.is_dir());
        assert!(root.join("tracked.txt").exists());
    }

    #[test]
    fn restoring_puts_every_archived_worktree_of_a_repository_back_on_the_panel() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let first = temp.path().join("one");
        let second = temp.path().join("two");
        for folder in [&root, &first, &second] {
            fs::create_dir_all(folder).unwrap();
        }
        let (repo, ids) = git_repo_with_worktrees(&root, &[&first, &second]);
        let repo_id = repo.id.clone();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_git_repo(repo, &ids[0]).unwrap();
        for id in &ids {
            database.archive_checkout(id).unwrap();
        }
        let shelved = database.load_workspace().unwrap();
        assert_eq!(shelved.repos[0].checkouts.len(), 1);
        assert_eq!(shelved.archived_worktrees.len(), 2);

        let restored = database.restore_archived_worktrees(&repo_id).unwrap();

        // One action brings the whole set back, which is the point of counting them.
        assert_eq!(restored.repos[0].checkouts.len(), 3);
        assert!(restored.archived_worktrees.is_empty());
        assert!(first.is_dir() && second.is_dir());
    }

    #[test]
    fn restoring_leaves_the_archived_worktrees_of_another_repository_alone() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let other = temp.path().join("other");
        let worktree = temp.path().join("worktree");
        let other_worktree = temp.path().join("other-worktree");
        for folder in [&root, &other, &worktree, &other_worktree] {
            fs::create_dir_all(folder).unwrap();
        }
        let (first_repo, first_worktree_id) = git_repo_with_worktree(&root, &worktree);
        let (second_repo, second_worktree_id) = git_repo_with_worktree(&other, &other_worktree);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database
            .register_git_repo(first_repo, &first_worktree_id)
            .unwrap();
        database
            .register_git_repo(second_repo, &second_worktree_id)
            .unwrap();
        database.archive_checkout(&first_worktree_id).unwrap();
        database.archive_checkout(&second_worktree_id).unwrap();

        let restored = database
            .restore_archived_worktrees(&repo_id_for_path(
                &root.canonicalize().unwrap().display().to_string(),
            ))
            .unwrap();

        // The other repository's worktree is still on its own shelf: archiving is per repo,
        // so restoring one repo cannot quietly put back another repo's work.
        assert_eq!(restored.archived_worktrees.len(), 1);
        assert_eq!(restored.archived_worktrees[0].id, second_worktree_id);
    }

    #[test]
    fn archiving_is_refused_where_it_would_mean_more_than_taking_a_row_off_the_panel() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        let plain = temp.path().join("plain");
        for folder in [&root, &worktree, &plain] {
            fs::create_dir_all(folder).unwrap();
        }
        let (repo, worktree_id) = git_repo_with_worktree(&root, &worktree);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_git_repo(repo, &worktree_id).unwrap();
        let primary_id = checkout_id_for_path(&root.canonicalize().unwrap().display().to_string());
        let plain_repo = plain_repo(&plain, "1");
        let plain_id = plain_repo.checkouts[0].id.clone();
        database.register_plain_repo(plain_repo).unwrap();

        // A repo root is the head of the list its worktrees hang off: archiving it would
        // archive the list, and closing is what the row's cross is for.
        let root_error = database.archive_checkout(&primary_id).unwrap_err();
        assert!(
            root_error.contains("only a worktree can be archived"),
            "{root_error}"
        );

        // A plain folder is not a worktree at all.
        let plain_error = database.archive_checkout(&plain_id).unwrap_err();
        assert!(
            plain_error.contains("only a worktree can be archived"),
            "{plain_error}"
        );

        // A worktree whose directory is gone has nothing to bring back.
        fs::remove_dir(&worktree).unwrap();
        let missing_error = database.archive_checkout(&worktree_id).unwrap_err();
        assert!(
            missing_error.contains("cannot be archived"),
            "{missing_error}"
        );

        // Nothing was archived by any of the refusals.
        assert!(database
            .load_workspace()
            .unwrap()
            .archived_worktrees
            .is_empty());
    }

    #[test]
    fn archiving_a_worktree_is_refused_while_one_of_its_sessions_runs() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        let (repo, worktree_id) = git_repo_with_worktree(&root, &worktree);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_git_repo(repo, &worktree_id).unwrap();
        database
            .add_terminal_session(&Session {
                id: "session:live".into(),
                session_type: SessionType::Shell,
                checkout_id: worktree_id.clone(),
                name: "shell".into(),
                created_at: "now".into(),
                status: SessionStatus::Active,
            })
            .unwrap();

        let error = database.archive_checkout(&worktree_id).unwrap_err();

        // The process would outlive the row that names it, which is the same reason a close
        // refuses: a terminal attached to a directory the panel no longer lists is unowned.
        assert!(error.contains("close active terminal sessions"), "{error}");
        assert_eq!(
            database.load_workspace().unwrap().repos[0].checkouts.len(),
            2
        );
        assert!(worktree.is_dir());
    }

    #[test]
    fn an_archived_worktree_survives_a_restart_and_is_still_there_to_restore() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("workspace.sqlite3");
        let root = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        let (repo, worktree_id) = git_repo_with_worktree(&root, &worktree);
        {
            let database = Database::open(&db_path).unwrap();
            database.register_git_repo(repo, &worktree_id).unwrap();
            database.archive_checkout(&worktree_id).unwrap();
        }

        // Reopened from the file, not from the handle that wrote it: archiving is a stored
        // state, so it has to outlive the process that set it.
        let restored = Database::open(&db_path).unwrap().load_workspace().unwrap();
        assert_eq!(restored.repos[0].checkouts.len(), 1);
        assert_eq!(restored.archived_worktrees.len(), 1);
        assert_eq!(restored.archived_worktrees[0].id, worktree_id);
    }

    #[test]
    fn closing_a_missing_git_worktree_removes_only_its_registration() {
        let temp = tempdir().expect("temporary directory");
        let root = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(root.join("tracked.txt"), "kept\n").unwrap();
        let (repo, worktree_id) = git_repo_with_worktree(&root, &worktree);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let registered = database.register_git_repo(repo, &worktree_id).unwrap();
        assert_eq!(
            registered.active_checkout_id.as_deref(),
            Some(worktree_id.as_str())
        );
        // The worktree is deleted outside Marvis, so the stored row is only marked by its path.
        fs::remove_dir(&worktree).unwrap();

        let closed = database.close_missing_checkout(&worktree_id).unwrap();

        // The entry goes and the selection falls back to the surviving primary; the
        // repository and its files stay exactly where they were.
        assert_eq!(closed.repos.len(), 1);
        assert_eq!(closed.repos[0].checkouts.len(), 1);
        let primary = &closed.repos[0].checkouts[0];
        assert!(primary.is_primary);
        assert_eq!(
            closed.active_checkout_id.as_deref(),
            Some(primary.id.as_str())
        );
        assert_eq!(closed.active_session_id, None);
        assert!(root.join("tracked.txt").exists());
    }

    #[test]
    fn closing_a_checkout_that_still_exists_is_refused() {
        let temp = tempdir().expect("temporary directory");
        let folder = temp.path().join("plain");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "1");
        let checkout_id = repo.checkouts[0].id.clone();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_plain_repo(repo).unwrap();

        let error = database.close_missing_checkout(&checkout_id).unwrap_err();

        assert!(error.contains("only a missing checkout can be closed"));
        assert_eq!(database.load_workspace().unwrap().repos.len(), 1);
    }

    #[test]
    fn closing_a_live_worktree_drops_its_registration_and_leaves_its_directory() {
        let temp = tempdir().expect("temporary directory");
        let root = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(root.join("tracked.txt"), "kept\n").unwrap();
        let (repo, worktree_id) = git_repo_with_worktree(&root, &worktree);
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        let registered = database.register_git_repo(repo, &worktree_id).unwrap();

        let closed = database.close_checkout(&worktree_id).unwrap();

        // Only the row goes: the directory and everything Git wrote in it stay, which is what
        // makes this the reverse of opening the folder again.
        assert_eq!(closed.repos[0].checkouts.len(), 1);
        assert_eq!(
            closed.active_checkout_id.as_deref(),
            Some(registered.repos[0].checkouts[0].id.as_str())
        );
        assert_eq!(closed.active_session_id, None);
        assert!(worktree.is_dir());
        assert!(root.join("tracked.txt").exists());
    }

    #[test]
    fn closing_a_live_checkout_is_refused_while_one_of_its_sessions_runs() {
        let temp = tempdir().expect("temporary directory");
        let folder = temp.path().join("plain");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "1");
        let checkout_id = repo.checkouts[0].id.clone();
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_plain_repo(repo).unwrap();
        database
            .add_terminal_session(&Session {
                id: "session:live".into(),
                session_type: SessionType::Shell,
                checkout_id: checkout_id.clone(),
                name: "shell".into(),
                created_at: "now".into(),
                status: SessionStatus::Active,
            })
            .unwrap();

        let error = database.close_checkout(&checkout_id).unwrap_err();

        assert!(error.contains("close active terminal sessions"));
        assert_eq!(database.load_workspace().unwrap().repos.len(), 1);
        assert!(folder.is_dir());
    }

    #[test]
    fn pruning_drops_a_checkout_whose_path_cannot_be_resolved() {
        let temp = tempdir().expect("temporary directory");
        let volume = temp.path().join("volume");
        let folder = volume.join("work");
        let live = temp.path().join("live");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir(&live).unwrap();
        let on_volume = plain_repo(&folder, "1");
        let database = Database::open(temp.path().join("workspace.sqlite3")).unwrap();
        database.register_plain_repo(on_volume).unwrap();
        let kept = plain_repo(&live, "1");
        database.register_plain_repo(kept.clone()).unwrap();
        // The volume is not mounted: the path is still registered and still resolves to
        // nothing, which is exactly what the prune reads as missing. Its registrations are
        // the accepted cost of the startup prune; `services::workspace::restore` is where
        // dropping that call brings them back.
        fs::remove_dir_all(&volume).unwrap();

        database.prune_missing_checkouts().unwrap();

        let state = database.load_workspace().unwrap();
        assert_eq!(state.repos.len(), 1);
        assert_eq!(state.repos[0].id, kept.id);
        // A selection in another checkout survives the prune of an unrelated one.
        assert_eq!(
            state.active_checkout_id.as_deref(),
            Some(kept.checkouts[0].id.as_str())
        );
    }
}
