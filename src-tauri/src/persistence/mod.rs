use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::domain::workspace::{
    Checkout, Repo, RepoKind, Session, SessionStatus, SessionType, WorkspaceState,
};

const SCHEMA_VERSION: i64 = 2;
const ACTIVE_CHECKOUT: &str = "active_checkout_id";
const ACTIVE_SESSION: &str = "active_session_id";

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
                 VALUES (?1, ?2, 'shell', ?3, ?4, 'inactive')",
                params![
                    session.id,
                    session.checkout_id,
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
                    "SELECT id, session_type, name, created_at FROM sessions WHERE checkout_id = ?1 ORDER BY rowid",
                )
                .map_err(db_error)?;
            let sessions = sessions_statement
                .query_map([&checkout_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(db_error)?
                .map(|row| {
                    let (id, session_type, name, created_at) = row?;
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
                        status: SessionStatus::Inactive,
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

    use crate::domain::workspace::Repo;

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
