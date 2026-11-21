use crate::postgres::ConnectionInput;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize)]
pub struct SavedConnection {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub sslmode: String,
    pub has_password: bool,
    pub project_id: Option<i64>,
    /// Optional global numeric shortcut (0 represents the 0 key).
    pub shortcut_slot: Option<u8>,
    /// Last observed result; never a live connection indicator.
    pub last_probe: Option<SavedConnectionProbe>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SavedConnectionProbe {
    pub kind: String,
    pub checked_at_ms: i64,
    pub server_version: Option<String>,
}

/// The existing probe command returns database|user|server_version. Store only
/// a bounded, validated PostgreSQL version, never the raw response.
pub fn probe_server_version(reply: &str) -> Option<String> {
    if reply.contains('\n') || reply.contains('\r') {
        return None;
    }
    let columns: Vec<&str> = reply.split('|').collect();
    if columns.len() != 3 {
        return None;
    }
    let version = columns[2].split_whitespace().next()?;
    if version.len() > 12
        || version.is_empty()
        || !version.split('.').all(|part| {
            !part.is_empty() && part.len() <= 3 && part.bytes().all(|c| c.is_ascii_digit())
        })
        || version.split('.').count() > 3
    {
        return None;
    }
    Some(version.to_owned())
}

fn saved_connection_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SavedConnection> {
    let kind: Option<String> = row.get(8)?;
    let checked_at: Option<i64> = row.get(9)?;
    let server_version: Option<String> = row.get(10)?;
    Ok(SavedConnection {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: row.get(3)?,
        database: row.get(4)?,
        username: row.get(5)?,
        sslmode: row.get(6)?,
        has_password: row.get::<_, i64>(7)? != 0,
        project_id: row.get(11)?,
        shortcut_slot: row.get(12)?,
        last_probe: kind
            .zip(checked_at)
            .map(|(kind, checked_at_ms)| SavedConnectionProbe {
                kind,
                checked_at_ms,
                server_version,
            }),
    })
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ConnectionProject {
    pub id: i64,
    pub name: String,
}

fn validate_project_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(crate::i18n::text(
            "Название проекта: от 1 до 80 печатных символов",
            "Project name must contain 1–80 printable characters",
        )
        .into());
    }
    Ok(name)
}

pub struct ConnectionStore {
    db: Connection,
    credentials_dir: PathBuf,
}

fn validate_name(name: &str) -> Result<&str, String> {
    let name = name.trim();

    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(crate::i18n::text(
            "Название подключения: от 1 до 80 печатных символов",
            "Connection name must contain 1–80 printable characters",
        )
        .into());
    }

    Ok(name)
}

fn credential_path(directory: &Path, id: i64) -> PathBuf {
    directory.join(format!("{id}.secret"))
}

fn write_password(directory: &Path, id: i64, password: &str) -> Result<(), String> {
    let path = credential_path(directory, id);
    let temporary = directory.join(format!(".{id}.secret.tmp"));

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| {
            crate::i18n::text(
                "Не удалось создать приватный файл пароля PostgreSQL",
                "Failed to create a private PostgreSQL password file",
            )
        })?;

    file.write_all(password.as_bytes()).map_err(|_| {
        crate::i18n::text(
            "Не удалось сохранить пароль PostgreSQL",
            "Failed to save PostgreSQL password",
        )
    })?;

    file.sync_all().map_err(|_| {
        crate::i18n::text(
            "Не удалось синхронизировать файл пароля PostgreSQL",
            "Failed to sync PostgreSQL password file",
        )
    })?;

    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).map_err(|_| {
        crate::i18n::text(
            "Не удалось выставить права 0600 на файл пароля PostgreSQL",
            "Failed to set mode 0600 on PostgreSQL password file",
        )
    })?;

    fs::rename(&temporary, &path).map_err(|_| {
        crate::i18n::text(
            "Не удалось завершить сохранение пароля PostgreSQL",
            "Failed to finalize PostgreSQL password storage",
        )
    })?;

    Ok(())
}

fn read_password(directory: &Path, id: i64) -> Result<String, String> {
    fs::read_to_string(credential_path(directory, id)).map_err(|_| {
        crate::i18n::text(
            "Сохранённый пароль PostgreSQL недоступен",
            "Stored PostgreSQL password is unavailable",
        )
        .into()
    })
}

fn remove_password(directory: &Path, id: i64) -> Result<(), String> {
    match fs::remove_file(credential_path(directory, id)) {
        Ok(()) => Ok(()),

        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),

        Err(_) => Err(crate::i18n::text(
            "Не удалось удалить сохранённый пароль PostgreSQL",
            "Failed to remove stored PostgreSQL password",
        )
        .into()),
    }
}

impl ConnectionStore {
    pub fn open(database_path: &Path, credentials_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(credentials_dir).map_err(|error| error.to_string())?;

        fs::set_permissions(credentials_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;

        let db = Connection::open(database_path).map_err(|error| error.to_string())?;
        db.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|error| error.to_string())?;

        db.busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;

        let version: u32 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;

        match version {
            0 => {
                db.execute_batch(
                    "CREATE TABLE saved_connections (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                        host TEXT NOT NULL,
                        port INTEGER NOT NULL
                            CHECK(port BETWEEN 1 AND 65535),
                        database_name TEXT NOT NULL,
                        username TEXT NOT NULL,
                        sslmode TEXT NOT NULL
                            CHECK(
                                sslmode IN (
                                    'verify-full',
                                    'require',
                                    'disable'
                                )
                            ),
                        has_password INTEGER NOT NULL
                            CHECK(has_password IN (0, 1))
                    );

                    PRAGMA user_version = 1;",
                )
                .map_err(|error| error.to_string())?;
            }

            1..=4 => {}

            _ => {
                return Err("Неподдерживаемая версия хранилища подключений".into());
            }
        }

        // v1 -> v2 is transactional: a failed migration must not advance user_version.
        if version <= 1 {
            db.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE connection_probe_results (
                    connection_id INTEGER PRIMARY KEY REFERENCES saved_connections(id) ON DELETE CASCADE,
                    kind TEXT NOT NULL CHECK(kind IN ('success', 'error')),
                    checked_at_ms INTEGER NOT NULL CHECK(checked_at_ms > 0),
                    server_version TEXT
                );
                PRAGMA user_version = 2;
                COMMIT;")
                .map_err(|error| error.to_string())?;
        }

        if version <= 2 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE connection_projects (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL COLLATE NOCASE UNIQUE
                );
                ALTER TABLE saved_connections ADD COLUMN
                    project_id INTEGER REFERENCES connection_projects(id) ON DELETE SET NULL;
                CREATE INDEX idx_saved_connections_project_id ON saved_connections(project_id);
                PRAGMA user_version = 3;
                COMMIT;",
            )
            .map_err(|error| error.to_string())?;
        }

        if version <= 3 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE saved_connections ADD COLUMN shortcut_slot INTEGER
                     CHECK(shortcut_slot BETWEEN 0 AND 9);
                 CREATE UNIQUE INDEX idx_saved_connections_shortcut
                     ON saved_connections(shortcut_slot)
                     WHERE shortcut_slot IS NOT NULL;
                 PRAGMA user_version = 4;
                 COMMIT;",
            )
            .map_err(|error| error.to_string())?;
        }

        Ok(Self {
            db,
            credentials_dir: credentials_dir.to_path_buf(),
        })
    }

    pub fn integrity_check(&self) -> Result<(), String> {
        let result: String = self
            .db
            .query_row(crate::SQLITE_QUICK_CHECK_SQL, [], |row| row.get(0))
            .map_err(|error| format!("Не удалось проверить connections SQLite: {error}"))?;

        if result == "ok" {
            Ok(())
        } else {
            Err(format!("connections SQLite quick_check: {result}"))
        }
    }

    pub fn list(&self) -> Result<Vec<SavedConnection>, String> {
        let mut statement = self
            .db
            .prepare(
                "SELECT
                    c.id,
                    c.name,
                    c.host,
                    c.port,
                    c.database_name,
                    c.username,
                    c.sslmode,
                    c.has_password,
                    p.kind, p.checked_at_ms, p.server_version, c.project_id, c.shortcut_slot
                 FROM saved_connections c
                 LEFT JOIN connection_probe_results p ON p.connection_id = c.id
                 ORDER BY c.name COLLATE NOCASE",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], saved_connection_row)
            .map_err(|error| error.to_string())?;

        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Set or clear a numeric slot. A slot belongs to at most one profile,
    /// and reassignment is atomic across independently running pgXray processes.
    pub fn assign_shortcut(
        &mut self,
        id: i64,
        slot: Option<u8>,
    ) -> Result<Vec<SavedConnection>, String> {
        if slot.is_some_and(|value| value > 9) {
            return Err("Клавиша должна быть от 0 до 9".into());
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM saved_connections WHERE id = ?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !exists {
            return Err("Сохранённое подключение не найдено".into());
        }
        if let Some(slot) = slot {
            tx.execute("UPDATE saved_connections SET shortcut_slot = NULL WHERE shortcut_slot = ?1 AND id != ?2",
                params![slot, id]).map_err(|error| error.to_string())?;
        }
        tx.execute(
            "UPDATE saved_connections SET shortcut_slot = ?1 WHERE id = ?2",
            params![slot, id],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        self.list()
    }

    pub fn get(&self, id: i64) -> Result<SavedConnection, String> {
        self.db
            .query_row(
                "SELECT c.id, c.name, c.host, c.port, c.database_name,
                        c.username, c.sslmode, c.has_password,
                        p.kind, p.checked_at_ms, p.server_version, c.project_id, c.shortcut_slot
                 FROM saved_connections c
                 LEFT JOIN connection_probe_results p ON p.connection_id = c.id
                 WHERE c.id = ?1",
                [id],
                saved_connection_row,
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Сохранённое подключение не найдено".to_owned())
    }

    pub fn save(&mut self, name: &str, input: ConnectionInput) -> Result<SavedConnection, String> {
        let name = validate_name(name)?.to_owned();

        input.validate()?;

        let has_password = !input.password.is_empty();
        let credentials_dir = self.credentials_dir.clone();

        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;

        tx.execute(
            "INSERT INTO saved_connections (
                name,
                host,
                port,
                database_name,
                username,
                sslmode,
                has_password
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &name,
                &input.host,
                input.port,
                &input.database,
                &input.username,
                &input.sslmode,
                has_password,
            ],
        )
        .map_err(|_| {
            "Не удалось сохранить подключение: название уже занято или данные некорректны"
        })?;

        let id = tx.last_insert_rowid();

        if has_password {
            write_password(&credentials_dir, id, &input.password)?;
        }

        if let Err(error) = tx.commit() {
            if has_password {
                let _ = remove_password(&credentials_dir, id);
            }

            return Err(error.to_string());
        }

        Ok(SavedConnection {
            id,
            name,
            host: input.host,
            port: input.port,
            database: input.database,
            username: input.username,
            sslmode: input.sslmode,
            has_password,
            project_id: None,
            shortcut_slot: None,
            last_probe: None,
        })
    }

    pub fn update(
        &mut self,
        id: i64,
        name: &str,
        input: ConnectionInput,
        keep_existing_password: bool,
    ) -> Result<SavedConnection, String> {
        let name = validate_name(name)?.to_owned();

        input.validate()?;

        let existing = self.get(id)?;

        let password_is_replaced = !input.password.is_empty();
        let target_changed = existing.host != input.host
            || existing.port != input.port
            || existing.database != input.database
            || existing.username != input.username
            || existing.sslmode != input.sslmode;

        let password_is_removed =
            input.password.is_empty() && !keep_existing_password && existing.has_password;

        let previous_password = if password_is_replaced || password_is_removed {
            if existing.has_password {
                Some(read_password(&self.credentials_dir, id)?)
            } else {
                None
            }
        } else {
            None
        };

        let has_password = if password_is_replaced {
            true
        } else if keep_existing_password {
            existing.has_password
        } else {
            false
        };

        let credentials_dir = self.credentials_dir.clone();

        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;

        let changed = tx
            .execute(
                "UPDATE saved_connections
             SET
                name = ?1,
                host = ?2,
                port = ?3,
                database_name = ?4,
                username = ?5,
                sslmode = ?6,
                has_password = ?7
             WHERE id = ?8",
                params![
                    &name,
                    &input.host,
                    i64::from(input.port),
                    &input.database,
                    &input.username,
                    &input.sslmode,
                    if has_password { 1_i64 } else { 0_i64 },
                    id,
                ],
            )
            .map_err(|_| {
                "Не удалось обновить подключение: название уже занято или данные некорректны"
            })?;

        if changed == 0 {
            return Err("Сохранённое подключение не найдено".into());
        }

        // A replacement secret invalidates a previous observation even when
        // has_password remains true. Renaming alone does not invalidate it.
        if target_changed || password_is_replaced || password_is_removed {
            tx.execute(
                "DELETE FROM connection_probe_results WHERE connection_id = ?1",
                [id],
            )
            .map_err(|error| error.to_string())?;
        }

        if password_is_replaced {
            write_password(&credentials_dir, id, &input.password)?;
        } else if password_is_removed {
            remove_password(&credentials_dir, id)?;
        }

        if let Err(error) = tx.commit() {
            match previous_password {
                Some(password) => {
                    let _ = write_password(&credentials_dir, id, &password);
                }

                None if password_is_replaced => {
                    let _ = remove_password(&credentials_dir, id);
                }

                None => {}
            }

            return Err(error.to_string());
        }

        self.get(id)
    }

    pub fn list_projects(&self) -> Result<Vec<ConnectionProject>, String> {
        let mut statement = self
            .db
            .prepare("SELECT id, name FROM connection_projects ORDER BY name COLLATE NOCASE, id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(ConnectionProject {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn create_project(&mut self, name: &str) -> Result<ConnectionProject, String> {
        let name = validate_project_name(name)?.to_owned();
        self.db
            .execute(
                "INSERT INTO connection_projects (name) VALUES (?1)",
                [&name],
            )
            .map_err(|_| "Проект с таким названием уже существует".to_owned())?;
        Ok(ConnectionProject {
            id: self.db.last_insert_rowid(),
            name,
        })
    }

    pub fn rename_project(&mut self, id: i64, name: &str) -> Result<ConnectionProject, String> {
        let name = validate_project_name(name)?.to_owned();
        let affected = self
            .db
            .execute(
                "UPDATE connection_projects SET name = ?1 WHERE id = ?2",
                params![name, id],
            )
            .map_err(|_| "Проект с таким названием уже существует".to_owned())?;
        if affected == 0 {
            return Err("Проект не найден".into());
        }
        Ok(ConnectionProject { id, name })
    }

    /// Deleting a grouping never deletes its connections, secrets or history.
    pub fn delete_project(&mut self, id: i64) -> Result<(), String> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        tx.execute(
            "UPDATE saved_connections SET project_id = NULL WHERE project_id = ?1",
            [id],
        )
        .map_err(|error| error.to_string())?;
        let affected = tx
            .execute("DELETE FROM connection_projects WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
        if affected == 0 {
            return Err("Проект не найден".into());
        }
        tx.commit().map_err(|error| error.to_string())
    }

    /// Moving a profile between groups modifies only project_id.
    pub fn assign_project(
        &mut self,
        connection_id: i64,
        project_id: Option<i64>,
    ) -> Result<SavedConnection, String> {
        if let Some(project_id) = project_id {
            let exists: bool = self
                .db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM connection_projects WHERE id = ?1)",
                    [project_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if !exists {
                return Err("Проект не найден".into());
            }
        }
        let affected = self
            .db
            .execute(
                "UPDATE saved_connections SET project_id = ?1 WHERE id = ?2",
                params![project_id, connection_id],
            )
            .map_err(|error| error.to_string())?;
        if affected == 0 {
            return Err("Сохранённое подключение не найдено".into());
        }
        self.get(connection_id)
    }

    /// Called by the saved-profile probe while holding its diagnostic permit.
    /// The raw PostgreSQL response and errors are deliberately never persisted.
    pub fn record_probe_result(
        &mut self,
        id: i64,
        successful: bool,
        server_version: Option<&str>,
    ) -> Result<(), String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis();
        let checked_at_ms =
            i64::try_from(timestamp).map_err(|_| "Системное время вне допустимого диапазона")?;
        let exists: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM saved_connections WHERE id = ?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !exists {
            return Err("Сохранённое подключение не найдено".into());
        }
        self.db.execute(
            "INSERT INTO connection_probe_results (connection_id, kind, checked_at_ms, server_version)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(connection_id) DO UPDATE SET
               kind = excluded.kind,
               checked_at_ms = excluded.checked_at_ms,
               server_version = excluded.server_version",
            params![id, if successful { "success" } else { "error" }, checked_at_ms,
                if successful { server_version } else { None }],
        ).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn load_input(&self, id: i64) -> Result<ConnectionInput, String> {
        let row = self
            .db
            .query_row(
                "SELECT
                    host,
                    port,
                    database_name,
                    username,
                    sslmode,
                    has_password
                 FROM saved_connections
                 WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u16>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, bool>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or("Сохранённое подключение не найдено")?;

        let password = if row.5 {
            read_password(&self.credentials_dir, id)?
        } else {
            String::new()
        };

        Ok(ConnectionInput {
            host: row.0,
            port: row.1,
            database: row.2,
            username: row.3,
            password,
            sslmode: row.4,
        })
    }

    pub fn delete(&mut self, id: i64) -> Result<(), String> {
        let has_password: Option<bool> = self
            .db
            .query_row(
                "SELECT has_password
                 FROM saved_connections
                 WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;

        let has_password = has_password.ok_or("Сохранённое подключение не найдено")?;

        if has_password {
            remove_password(&self.credentials_dir, id)?;
        }

        self.db
            .execute(
                "DELETE FROM saved_connections
                 WHERE id = ?1",
                [id],
            )
            .map_err(|error| error.to_string())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(password: &str) -> ConnectionInput {
        ConnectionInput {
            host: "127.0.0.1".into(),
            port: 5432,
            database: "postgres".into(),
            username: "postgres".into(),
            password: password.into(),
            sslmode: "disable".into(),
        }
    }

    #[test]
    fn shortcut_slots_are_unique_and_persist_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("connections.sqlite3");
        let secrets = dir.path().join("secrets");
        let mut store = ConnectionStore::open(&db, &secrets).unwrap();
        let one = store.save("sample-a", input("")).unwrap();
        let two = store.save("sample-b", input("")).unwrap();
        store.assign_shortcut(one.id, Some(1)).unwrap();
        assert_eq!(store.get(one.id).unwrap().shortcut_slot, Some(1));
        store.assign_shortcut(two.id, Some(1)).unwrap();
        assert_eq!(store.get(one.id).unwrap().shortcut_slot, None);
        assert_eq!(store.get(two.id).unwrap().shortcut_slot, Some(1));
        assert!(store.assign_shortcut(-1, Some(2)).is_err());
        assert!(store.assign_shortcut(two.id, Some(10)).is_err());
        drop(store);
        let store = ConnectionStore::open(&db, &secrets).unwrap();
        assert_eq!(store.get(two.id).unwrap().shortcut_slot, Some(1));
    }

    #[test]
    fn persists_probe_and_invalidates_on_target_or_password_change() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("connections.sqlite3");
        let secrets = directory.path().join("credentials");
        let mut store = ConnectionStore::open(&db, &secrets).unwrap();
        let saved = store.save("local", input("first-password")).unwrap();
        store
            .record_probe_result(saved.id, true, Some("16.15"))
            .unwrap();
        assert_eq!(
            store
                .get(saved.id)
                .unwrap()
                .last_probe
                .unwrap()
                .server_version
                .as_deref(),
            Some("16.15")
        );
        drop(store);
        let mut store = ConnectionStore::open(&db, &secrets).unwrap();
        assert_eq!(
            store.list().unwrap()[0].last_probe.as_ref().unwrap().kind,
            "success"
        );
        store.update(saved.id, "renamed", input(""), true).unwrap();
        assert!(store.get(saved.id).unwrap().last_probe.is_some());
        // Replacing the password MUST invalidate, even though has_password is unchanged.
        store
            .update(saved.id, "renamed", input("second-password"), false)
            .unwrap();
        assert!(store.get(saved.id).unwrap().last_probe.is_none());
        store
            .record_probe_result(saved.id, true, Some("16.15"))
            .unwrap();
        let mut changed = input("");
        changed.host = "localhost".into();
        store.update(saved.id, "renamed", changed, true).unwrap();
        assert!(store.get(saved.id).unwrap().last_probe.is_none());
        store.record_probe_result(saved.id, false, None).unwrap();
        assert_eq!(
            store.get(saved.id).unwrap().last_probe.unwrap().kind,
            "error"
        );
        store.delete(saved.id).unwrap();
        let count: i64 = store
            .db
            .query_row("SELECT COUNT(*) FROM connection_probe_results", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn migrates_v1_connection_without_changing_its_id_or_secret() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("connections.sqlite3");
        let credentials = directory.path().join("credentials");
        fs::create_dir(&credentials).unwrap();
        let original = Connection::open(&db).unwrap();
        original.execute_batch("CREATE TABLE saved_connections (
            id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE,
            host TEXT NOT NULL, port INTEGER NOT NULL, database_name TEXT NOT NULL,
            username TEXT NOT NULL, sslmode TEXT NOT NULL, has_password INTEGER NOT NULL
        ); INSERT INTO saved_connections (id,name,host,port,database_name,username,sslmode,has_password)
        VALUES (42,'old','127.0.0.1',5432,'postgres','postgres','disable',1);
        PRAGMA user_version = 1;").unwrap();
        drop(original);
        write_password(&credentials, 42, "legacy-secret").unwrap();
        let store = ConnectionStore::open(&db, &credentials).unwrap();
        assert_eq!(store.get(42).unwrap().id, 42);
        assert_eq!(store.load_input(42).unwrap().password, "legacy-secret");
        let version: i64 = store
            .db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    #[test]
    fn migrates_v2_to_v3_without_losing_prior_probe() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("connections.sqlite3");
        let secrets = directory.path().join("credentials");
        let legacy = Connection::open(&db).unwrap();
        legacy.execute_batch("CREATE TABLE saved_connections (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, host TEXT NOT NULL,
            port INTEGER NOT NULL, database_name TEXT NOT NULL, username TEXT NOT NULL,
            sslmode TEXT NOT NULL, has_password INTEGER NOT NULL);
            CREATE TABLE connection_probe_results (
            connection_id INTEGER PRIMARY KEY REFERENCES saved_connections(id) ON DELETE CASCADE,
            kind TEXT NOT NULL, checked_at_ms INTEGER NOT NULL, server_version TEXT);
            INSERT INTO saved_connections VALUES(42,'existing','127.0.0.1',5432,'postgres','postgres','disable',0);
            INSERT INTO connection_probe_results VALUES(42,'success',123456789,'16.15');
            PRAGMA user_version = 2;").unwrap();
        drop(legacy);
        let store = ConnectionStore::open(&db, &secrets).unwrap();
        let saved = store.get(42).unwrap();
        assert_eq!(saved.project_id, None);
        assert_eq!(
            saved.last_probe.unwrap().server_version.as_deref(),
            Some("16.15")
        );
        let version: i64 = store
            .db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    #[test]
    fn grouping_is_independent_of_credentials_probe_and_profile_history() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("connections.sqlite3");
        let secrets = directory.path().join("credentials");
        let mut store = ConnectionStore::open(&db, &secrets).unwrap();
        let profile = store.save("local", input("secret")).unwrap();
        store
            .record_probe_result(profile.id, true, Some("16.15"))
            .unwrap();
        let first = store.create_project("dba-trainer").unwrap();
        let second = store.create_project("pgXray").unwrap();
        assert!(store.create_project("DBA-TRAINER").is_err());
        store.assign_project(profile.id, Some(first.id)).unwrap();
        assert_eq!(store.get(profile.id).unwrap().project_id, Some(first.id));
        store.rename_project(first.id, "dba-trainer-2").unwrap();
        store.assign_project(profile.id, Some(second.id)).unwrap();
        assert_eq!(
            store
                .get(profile.id)
                .unwrap()
                .last_probe
                .as_ref()
                .unwrap()
                .kind,
            "success"
        );
        store.delete_project(second.id).unwrap();
        assert!(store.get(profile.id).unwrap().project_id.is_none());
        assert_eq!(store.load_input(profile.id).unwrap().password, "secret");
        assert!(store.get(profile.id).unwrap().last_probe.is_some());
        assert!(store.assign_project(profile.id, Some(999)).is_err());
        drop(store);
        let store = ConnectionStore::open(&db, &secrets).unwrap();
        assert_eq!(store.list_projects().unwrap()[0].name, "dba-trainer-2");
        let version: i64 = store
            .db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    #[test]
    fn parses_only_bounded_server_versions() {
        assert_eq!(
            probe_server_version("postgres|reader|16.15 (Debian build)"),
            Some("16.15".into())
        );
        assert_eq!(probe_server_version("raw error text"), None);
        assert_eq!(probe_server_version("postgres|reader|16.15\nsecret"), None);
    }

    #[test]
    fn saves_loads_and_deletes_profile() {
        let directory = tempfile::tempdir().unwrap();

        let database_path = directory.path().join("connections.sqlite3");

        let credentials_dir = directory.path().join("credentials");

        let mut store = ConnectionStore::open(&database_path, &credentials_dir).unwrap();

        let saved = store.save("local", input("postgres-password")).unwrap();

        assert_eq!(store.list().unwrap().len(), 1,);

        assert_eq!(
            store.load_input(saved.id).unwrap().password,
            "postgres-password",
        );

        let mode = fs::metadata(credential_path(&credentials_dir, saved.id))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(mode, 0o600);

        store.delete(saved.id).unwrap();

        assert!(store.list().unwrap().is_empty());

        assert!(!credential_path(&credentials_dir, saved.id,).exists());
    }
}
