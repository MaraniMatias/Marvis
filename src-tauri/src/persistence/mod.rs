use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::domain::terminal_layout::CheckoutTerminalLayout;
use crate::domain::workspace::{
    Checkout, Repo, RepoKind, Session, SessionStatus, SessionType, WorkspaceState,
};

const SCHEMA_VERSION: i64 = 5;
const ACTIVE_CHECKOUT: &str = "active_checkout_id";
const ACTIVE_SESSION: &str = "active_session_id";
const WORKTREE_LOCATION: &str = "worktree_location";
const WINDOW_GEOMETRY: &str = "window_geometry";
const WINDOW_MAXIMIZED: &str = "window_maximized";

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
        migrate(&connection)?;
        connection
            .execute("UPDATE sessions SET status = 'inactive'", [])
            .map_err(db_error)?;

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

    pub fn close_missing_checkout(&self, checkout_id: &str) -> Result<WorkspaceState, String> {
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
        if !is_missing {
            return Err("only a missing checkout can be closed".into());
        }
        if kind == "git" && !is_primary {
            return Err("missing Git worktrees must be closed through Git reconciliation".into());
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
            return Err(
                "close active terminal sessions before closing this missing location".into(),
            );
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

    pub fn load_workspace(&self) -> Result<WorkspaceState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        load_workspace(&connection)
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
        validate_terminal_layout(&transaction, checkout_id, layout)?;
        let serialized = serde_json::to_string(layout).map_err(|error| error.to_string())?;
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

fn migrate(connection: &Connection) -> Result<(), String> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_error)?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "database schema version {version} is newer than supported version {SCHEMA_VERSION}"
        ));
    }
    if version < 1 {
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
                    position INTEGER NOT NULL,
                    UNIQUE (repo_id, position)
                );
                CREATE TABLE sessions (
                    id TEXT PRIMARY KEY NOT NULL,
                    checkout_id TEXT NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                    session_type TEXT NOT NULL CHECK (session_type IN ('shell', 'nvim', 'server', 'custom', 'agent')),
                    name TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'inactive' CHECK (status = 'inactive')
                );
                CREATE TABLE recent_paths (
                    canonical_path TEXT PRIMARY KEY NOT NULL,
                    last_opened_at TEXT NOT NULL
                );
                CREATE TABLE preferences (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL
                );
                PRAGMA user_version = 1;",
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
    }
    if version < 2 {
        connection
            .execute_batch(
                "ALTER TABLE checkouts ADD COLUMN is_missing INTEGER NOT NULL DEFAULT 0;
                 PRAGMA user_version = 2;",
            )
            .map_err(db_error)?;
    }
    if version < 3 {
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        transaction
            .execute_batch(
                "ALTER TABLE sessions RENAME TO sessions_v2;
                 CREATE TABLE sessions (
                     id TEXT PRIMARY KEY NOT NULL,
                     checkout_id TEXT NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                     session_type TEXT NOT NULL CHECK (session_type IN ('shell', 'nvim', 'server', 'custom', 'agent')),
                     name TEXT NOT NULL,
                     created_at TEXT NOT NULL,
                     status TEXT NOT NULL DEFAULT 'inactive' CHECK (status IN ('active', 'inactive'))
                 );
                 INSERT INTO sessions (id, checkout_id, session_type, name, created_at, status)
                     SELECT id, checkout_id, session_type, name, created_at, 'inactive' FROM sessions_v2;
                 DROP TABLE sessions_v2;
                 PRAGMA user_version = 3;",
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
    }
    if version < 4 {
        connection
            .execute_batch(
                "CREATE TABLE checkout_terminal_layouts (
                    checkout_id TEXT PRIMARY KEY NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                    layout_json TEXT NOT NULL
                );
                PRAGMA user_version = 4;",
            )
            .map_err(db_error)?;
    }
    if version < 5 {
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        transaction
            .execute_batch(
                "CREATE TABLE viewed_files (
                    checkout_id TEXT NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
                    path TEXT NOT NULL,
                    viewed_at TEXT NOT NULL,
                    PRIMARY KEY (checkout_id, path)
                );
                PRAGMA user_version = 5;",
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
    }
    Ok(())
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
    for (id, kind, name, root, default_branch, created_at, last_opened_at) in repo_rows {
        let mut checkouts_statement = connection
            .prepare(
                "SELECT id, path, canonical_path, is_primary, branch, head, ahead_of_default, changed_files, is_missing
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
        ) in checkout_rows
        {
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

    use rusqlite::params;
    use tempfile::tempdir;

    use crate::domain::{
        terminal_layout::{CheckoutTerminalLayout, TerminalLayoutNode, TerminalLayoutTab},
        workspace::{Repo, Session, SessionStatus, SessionType},
    };

    use super::{Database, SCHEMA_VERSION};

    fn plain_repo(path: &Path, now: &str) -> Repo {
        Repo::plain(path, now).expect("plain repo")
    }

    #[test]
    fn versioned_migration_is_idempotent_and_has_no_diff_reference_column() {
        let database = Database::open_in_memory().expect("database");
        let version: i64 = database
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        super::migrate(&database.connection.lock().unwrap()).expect("migration reruns safely");
        let has_diff_reference: bool = database
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('checkouts') WHERE name = 'diff_ref')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!has_diff_reference);

        let has_checkout_layouts: bool = database
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'checkout_terminal_layouts')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(has_checkout_layouts);
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
    fn historical_session_metadata_is_inactive_after_restart() {
        let temp = tempdir().expect("temporary directory");
        let db_path = temp.path().join("workspace.sqlite3");
        let folder = temp.path().join("plain");
        fs::create_dir(&folder).unwrap();
        let repo = plain_repo(&folder, "1");
        {
            let database = Database::open(&db_path).expect("database");
            database
                .register_plain_repo(repo.clone())
                .expect("register plain repo");
            database
                .connection
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO sessions (id, checkout_id, session_type, name, created_at) VALUES ('old-shell', ?1, 'shell', 'zsh', '1')",
                    [&repo.checkouts[0].id],
                )
                .unwrap();
            database
                .select_session(Some("old-shell".into()))
                .expect("select saved session");
            let state = database
                .register_plain_repo(plain_repo(&folder, "2"))
                .expect("reopen same checkout");
            assert_eq!(state.active_session_id.as_deref(), Some("old-shell"));
        }

        let state = Database::open(&db_path)
            .expect("restart database")
            .load_workspace()
            .expect("restore workspace");
        assert_eq!(state.active_session_id.as_deref(), Some("old-shell"));
        let session = &state.repos[0].checkouts[0].sessions[0];
        assert_eq!(
            session.status,
            crate::domain::workspace::SessionStatus::Inactive
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
    fn checkout_layouts_persist_and_reject_sessions_owned_by_another_checkout() {
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
        assert!(database
            .save_terminal_layout(&first_repo.checkouts[0].id, &foreign_layout)
            .is_err());
        assert!(database.load_terminal_layout("checkout:unknown").is_err());
        assert_eq!(
            database
                .load_terminal_layout(&first_repo.checkouts[0].id)
                .expect("failed save leaves old layout intact"),
            Some(layout.clone())
        );
        drop(database);

        let reopened = Database::open(&database_path).expect("reopen database");
        assert_eq!(
            reopened
                .load_terminal_layout(&first_repo.checkouts[0].id)
                .expect("restore layout from SQLite"),
            Some(layout)
        );
        reopened
            .connection
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM sessions WHERE id = ?1",
                [&first_split_session.id],
            )
            .unwrap();
        let pruned = reopened
            .load_terminal_layout(&first_repo.checkouts[0].id)
            .expect("prune a removed historical session");
        assert_eq!(
            pruned.unwrap().tabs[0].root,
            TerminalLayoutNode::Session {
                session_id: first_session.id
            }
        );
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

        let state = Database::open(&db_path)
            .expect("restart database")
            .load_workspace()
            .expect("restore workspace");
        assert_eq!(state.repos.len(), 1);
        assert_eq!(state.repos[0].id, repo.id);
        assert!(state.repos[0].checkouts[0].is_missing);
    }
}
