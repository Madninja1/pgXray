pub mod activity;
pub mod app_log;
pub mod auth;
pub mod autovacuum;
pub mod connections;
pub mod evidence;
pub mod history;
pub mod i18n;
pub mod index_health;
pub mod internal_postgres;
pub mod io_wal;
pub mod locks;
pub mod pgss;
pub mod plan;
pub mod postgres;
pub mod postgres_settings;
pub mod privacy;
pub mod readiness;
pub mod sessions;
pub mod workspace;
pub mod xid;

/// SQL executed by the SQLite integrity checks exposed through Tauri readiness.
pub const SQLITE_QUICK_CHECK_SQL: &str = "PRAGMA quick_check(1)";
