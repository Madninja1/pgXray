use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::OsRng;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
pub struct Session {
    pub username: String,
}

#[derive(Serialize)]
pub struct AuthState {
    pub needs_registration: bool,
    pub session: Option<Session>,
}

#[derive(Serialize)]
pub struct UserSummary {
    pub username: String,
    pub current: bool,
}

pub struct Auth {
    db: Connection,
    session: Option<Session>,
    next_login: Instant,
}

fn hash(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|value| value.to_string())
        .map_err(|_| {
            crate::i18n::text(
                "Не удалось вычислить хеш пароля",
                "Failed to calculate password hash",
            )
            .into()
        })
}

fn validate_username(username: &str) -> Result<(), String> {
    if username.is_empty()
        || username.len() > 64
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(crate::i18n::text(
            "Логин: 1–64 латинских букв, цифр, символов _ или -",
            "Username: 1–64 Latin letters, digits, underscores, or hyphens",
        )
        .into());
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < 12 || password.len() > 1024 {
        return Err(crate::i18n::text(
            "Пароль: минимум 12 символов, максимум 1024 байта",
            "Password: at least 12 characters, at most 1024 bytes",
        )
        .into());
    }
    Ok(())
}

fn is_legacy_default_admin(encoded: &str) -> bool {
    PasswordHash::new(encoded)
        .ok()
        .is_some_and(|parsed| Argon2::default().verify_password(b"admin", &parsed).is_ok())
}

fn sqlite_integrity_check(db: &Connection, label: &str) -> Result<(), String> {
    let result: String = db
        .query_row(crate::SQLITE_QUICK_CHECK_SQL, [], |row| row.get(0))
        .map_err(|error| format!("Не удалось проверить {label} SQLite: {error}"))?;

    if result == "ok" {
        Ok(())
    } else {
        Err(format!("{label} SQLite quick_check: {result}"))
    }
}

impl Auth {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut db = Connection::open(path).map_err(|error| error.to_string())?;
        db.busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;

        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let version: u32 = tx
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;

        match version {
            0 => {
                tx.execute_batch(
                    "CREATE TABLE users (username TEXT PRIMARY KEY, password_hash TEXT NOT NULL);\
                     PRAGMA user_version = 2;",
                )
                .map_err(|error| error.to_string())?;
            }
            1 => {
                let default_admin_hash: Option<String> = tx
                    .query_row(
                        "SELECT password_hash FROM users WHERE username = 'admin'",
                        [],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;

                tx.execute_batch(
                    "ALTER TABLE users RENAME TO users_v1;\
                     CREATE TABLE users (username TEXT PRIMARY KEY, password_hash TEXT NOT NULL);\
                     INSERT INTO users (username, password_hash)\
                         SELECT username, password_hash FROM users_v1;\
                     DROP TABLE users_v1;\
                     PRAGMA user_version = 2;",
                )
                .map_err(|error| error.to_string())?;

                if default_admin_hash
                    .as_deref()
                    .is_some_and(is_legacy_default_admin)
                {
                    tx.execute("DELETE FROM users WHERE username = 'admin'", [])
                        .map_err(|error| error.to_string())?;
                }
            }
            2 => {}
            _ => {
                return Err(crate::i18n::text(
                    "Неподдерживаемая версия локального хранилища",
                    "Unsupported local storage version",
                )
                .into())
            }
        }

        tx.commit().map_err(|error| error.to_string())?;
        Ok(Self {
            db,
            session: None,
            next_login: Instant::now(),
        })
    }

    pub fn integrity_check(&self) -> Result<(), String> {
        sqlite_integrity_check(&self.db, "auth")
    }

    fn user_count(&self) -> Result<i64, String> {
        self.db
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .map_err(|error| error.to_string())
    }

    pub fn state(&self) -> Result<AuthState, String> {
        Ok(AuthState {
            needs_registration: self.user_count()? == 0,
            session: self.session.clone(),
        })
    }

    pub fn register_initial_user(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<Session, String> {
        validate_username(username)?;
        validate_password(password)?;

        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if count != 0 {
            return Err(crate::i18n::text(
                "Первичная регистрация уже выполнена",
                "Initial registration has already been completed",
            )
            .into());
        }

        let encoded = hash(password)?;
        tx.execute(
            "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
            params![username, encoded],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;

        let session = Session {
            username: username.into(),
        };
        self.session = Some(session.clone());
        Ok(session)
    }

    pub fn login(&mut self, username: &str, password: &str) -> Result<Session, String> {
        self.session = None;
        if Instant::now() < self.next_login {
            return Err(crate::i18n::text(
                "Повторите вход через секунду",
                "Try signing in again in one second",
            )
            .into());
        }
        self.next_login = Instant::now() + Duration::from_secs(1);
        if username.len() > 64 || password.len() > 1024 {
            return Err(crate::i18n::text(
                "Неверный логин или пароль",
                "Invalid username or password",
            )
            .into());
        }

        let encoded: Option<String> = self
            .db
            .query_row(
                "SELECT password_hash FROM users WHERE username = ?1",
                [username],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let encoded = encoded.ok_or(crate::i18n::text(
            "Неверный логин или пароль",
            "Invalid username or password",
        ))?;
        let parsed = PasswordHash::new(&encoded).map_err(|_| "Повреждён хеш пароля")?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| {
                crate::i18n::text("Неверный логин или пароль", "Invalid username or password")
            })?;

        let session = Session {
            username: username.into(),
        };
        self.session = Some(session.clone());
        Ok(session)
    }

    pub fn require_authenticated(&self) -> Result<(), String> {
        let session = self
            .session
            .as_ref()
            .ok_or("Сначала войдите в приложение")?;
        let exists: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM users WHERE username = ?1)",
                [&session.username],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !exists {
            return Err("Пользователь недоступен".into());
        }
        Ok(())
    }

    pub fn list_users(&self) -> Result<Vec<UserSummary>, String> {
        self.require_authenticated()?;
        let current = self
            .session
            .as_ref()
            .map(|session| session.username.as_str())
            .ok_or("Сначала войдите в приложение")?;
        let mut statement = self
            .db
            .prepare("SELECT username FROM users ORDER BY username COLLATE NOCASE")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;

        rows.map(|row| {
            let username = row.map_err(|error| error.to_string())?;
            Ok(UserSummary {
                current: username == current,
                username,
            })
        })
        .collect()
    }

    pub fn create_user(&self, username: &str, password: &str) -> Result<(), String> {
        self.require_authenticated()?;
        validate_username(username)?;
        validate_password(password)?;
        let encoded = hash(password)?;
        self.db
            .execute(
                "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
                params![username, encoded],
            )
            .map_err(|_| "Не удалось создать пользователя: проверьте уникальность логина")?;
        Ok(())
    }

    pub fn rename_user(&mut self, username: &str, new_username: &str) -> Result<(), String> {
        self.require_authenticated()?;
        validate_username(new_username)?;
        if username == new_username {
            return Ok(());
        }

        let changed = self
            .db
            .execute(
                "UPDATE users SET username = ?1 WHERE username = ?2",
                params![new_username, username],
            )
            .map_err(|_| "Не удалось изменить логин: проверьте уникальность нового логина")?;
        if changed == 0 {
            return Err("Пользователь не найден".into());
        }

        if self
            .session
            .as_ref()
            .is_some_and(|session| session.username == username)
        {
            self.session = Some(Session {
                username: new_username.into(),
            });
        }
        Ok(())
    }

    pub fn change_user_password(&self, username: &str, password: &str) -> Result<(), String> {
        self.require_authenticated()?;
        validate_password(password)?;
        let encoded = hash(password)?;
        let changed = self
            .db
            .execute(
                "UPDATE users SET password_hash = ?1 WHERE username = ?2",
                params![encoded, username],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("Пользователь не найден".into());
        }
        Ok(())
    }

    pub fn delete_user(&mut self, username: &str) -> Result<(), String> {
        self.require_authenticated()?;
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.username == username)
        {
            return Err("Нельзя удалить пользователя, под которым выполнен вход".into());
        }
        if self.user_count()? <= 1 {
            return Err("Нельзя удалить последнего пользователя".into());
        }

        let changed = self
            .db
            .execute("DELETE FROM users WHERE username = ?1", [username])
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("Пользователь не найден".into());
        }
        Ok(())
    }

    pub fn logout(&mut self) {
        self.session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_registration_and_user_management_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.sqlite3");
        let mut auth = Auth::open(&path).unwrap();

        assert!(auth.state().unwrap().needs_registration);
        assert!(auth.login("owner", "owner-password-123").is_err());
        auth.next_login = Instant::now();

        let session = auth
            .register_initial_user("owner", "owner-password-123")
            .unwrap();
        assert_eq!(session.username, "owner");
        assert!(!auth.state().unwrap().needs_registration);
        assert!(auth
            .register_initial_user("second", "second-password-123")
            .is_err());

        auth.create_user("alice", "alice-password-123").unwrap();
        auth.rename_user("alice", "bob").unwrap();
        auth.change_user_password("bob", "bob-password-new-123")
            .unwrap();
        assert!(auth.delete_user("owner").is_err());
        auth.delete_user("bob").unwrap();
        assert_eq!(auth.list_users().unwrap().len(), 1);

        auth.logout();
        drop(auth);

        let mut auth = Auth::open(&path).unwrap();
        assert!(!auth.state().unwrap().needs_registration);
        assert_eq!(
            auth.login("owner", "owner-password-123").unwrap().username,
            "owner"
        );
    }

    #[test]
    fn migration_removes_known_default_admin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.sqlite3");
        let legacy_hash = hash("admin").unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE users (username TEXT PRIMARY KEY, password_hash TEXT NOT NULL, must_change INTEGER NOT NULL CHECK(must_change IN (0,1)));\
             PRAGMA user_version = 1;",
        )
        .unwrap();
        db.execute(
            "INSERT INTO users VALUES (?1, ?2, 1)",
            params!["admin", legacy_hash],
        )
        .unwrap();
        drop(db);

        let auth = Auth::open(&path).unwrap();
        assert!(auth.state().unwrap().needs_registration);
    }
}
