//! Local diagnostics SQLite observability and controlled maintenance.
//! Auto-prune never touches connections referenced by Investigation Cases,
//! nor individually pinned snapshots. No SQL is sent to PostgreSQL here.
use super::{unix_time_millis, DiagnosticStore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{fs, path::PathBuf, time::Duration};

const SECTIONS: &[(&str, &str)] = &[
    ("xid", "xid_snapshots"),
    ("pgss", "pgss_snapshots"),
    ("autovacuum", "autovacuum_snapshots"),
    ("pg_settings", "pg_settings_snapshots"),
    ("io_wal", "io_wal_snapshots"),
    ("index_health", "index_health_snapshots"),
];
const MIN_BUDGET_BYTES: u64 = 100 * 1024 * 1024;
const MAX_BUDGET_BYTES: u64 = 50 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct StorageSection {
    pub kind: String,
    pub count: i64,
    pub json_bytes: i64,
    pub last_week_bytes: i64,
    pub oldest_at_ms: Option<i64>,
    pub newest_at_ms: Option<i64>,
    pub pinned_count: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct StoragePolicy {
    pub max_bytes: Option<u64>,
    pub auto_prune: bool,
    pub retention_days: Option<u32>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StorageHealth {
    pub db_bytes: u64,
    pub wal_bytes: u64,
    pub shm_bytes: u64,
    pub live_bytes_estimate: u64,
    pub reusable_bytes: u64,
    pub last_maintenance_ms: Option<i64>,
    pub policy: StoragePolicy,
    pub sections: Vec<StorageSection>,
    pub case_protected_connections: i64,
    pub projected_days_to_budget: Option<f64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct StorageMaintenance {
    pub action: String,
    pub deleted: u64,
    pub detail: String,
}

/// The diagnostics DB predates versioned migrations. v0 is its original
/// six-collector schema; v1 adds durable rules, claims and protected snapshots.
/// The migration and version advance are in one SQLite transaction.
pub(super) fn migrate_diagnostics_schema(db: &Connection) -> Result<(), String> {
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version > 1 {
        return Err(format!(
            "diagnostics SQLite версии {version} новее приложения"
        ));
    }
    if version == 1 {
        return Ok(());
    }
    db.execute_batch(
        r#"
        BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS snapshot_schedules (
            owner TEXT NOT NULL,
            rule_key TEXT NOT NULL,
            rule_json TEXT NOT NULL,
            revision INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY(owner, rule_key)
        );
        CREATE TABLE IF NOT EXISTS snapshot_schedule_claims (
            connection_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            slot_at_ms INTEGER NOT NULL,
            claimed_at_ms INTEGER NOT NULL,
            PRIMARY KEY(connection_id, kind, slot_at_ms)
        );
        CREATE TABLE IF NOT EXISTS snapshot_protections (
            section TEXT NOT NULL,
            snapshot_id INTEGER NOT NULL,
            PRIMARY KEY(section, snapshot_id)
        );
        PRAGMA user_version = 1;
        COMMIT;
    "#,
    )
    .map_err(|e| format!("Не удалось мигрировать diagnostics SQLite v0 → v1: {e}"))?;
    Ok(())
}

impl DiagnosticStore {
    fn setting(&self, name: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM diagnostic_settings WHERE key=?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    fn store_setting(&self, name: &str, value: Option<String>) -> Result<(), String> {
        if let Some(value) = value {
            self.connection.execute(
                "INSERT INTO diagnostic_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![name, value],
            ).map_err(|e| e.to_string())?;
        } else {
            self.connection
                .execute("DELETE FROM diagnostic_settings WHERE key=?1", [name])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn storage_policy(&self) -> Result<StoragePolicy, String> {
        let max_bytes = self
            .setting("history_max_bytes")?
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|_| {
                crate::i18n::text("Повреждён лимит истории", "History limit is corrupted")
            })?;
        Ok(StoragePolicy {
            max_bytes,
            auto_prune: self.setting("history_auto_prune")?.as_deref() == Some("1"),
            retention_days: self.history_retention_settings()?.retention_days,
        })
    }
    pub fn update_storage_policy(
        &self,
        max_bytes: Option<u64>,
        auto_prune: bool,
    ) -> Result<StoragePolicy, String> {
        if max_bytes.is_some_and(|value| !(MIN_BUDGET_BYTES..=MAX_BUDGET_BYTES).contains(&value)) {
            return Err(crate::i18n::text(
                "Лимит SQLite должен быть от 100 МиБ до 50 ГиБ",
                "SQLite storage budget must be between 100 MiB and 50 GiB",
            )
            .into());
        }
        if auto_prune
            && max_bytes.is_none()
            && self.history_retention_settings()?.retention_days.is_none()
        {
            return Err(crate::i18n::text(
                "Для автоматической очистки задай срок хранения или бюджет",
                "Set a retention period or storage budget to enable automatic cleanup",
            )
            .into());
        }
        self.store_setting("history_max_bytes", max_bytes.map(|n| n.to_string()))?;
        self.store_setting(
            "history_auto_prune",
            Some(if auto_prune { "1" } else { "0" }.into()),
        )?;
        self.storage_policy()
    }
    fn main_db_path(&self) -> Result<PathBuf, String> {
        let path: String = self
            .connection
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if path.is_empty() {
            return Err(crate::i18n::text(
                "Обслуживание файлового SQLite недоступно для in-memory database",
                "File-based SQLite maintenance is unavailable for in-memory databases",
            )
            .into());
        }
        Ok(PathBuf::from(path))
    }
    fn file_bytes(path: &std::path::Path) -> u64 {
        fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }
    fn space(&self) -> Result<(u64, u64), String> {
        let page_count: u64 = self
            .connection
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let freelist: u64 = self
            .connection
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let size: u64 = self
            .connection
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(((page_count - freelist) * size, freelist * size))
    }
    fn case_count(&self) -> Result<i64, String> {
        let path = self.main_db_path()?.with_file_name("workspace.sqlite3");
        if !path.exists() {
            return Ok(0);
        }
        let ws = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Недоступно хранилище Investigation Cases: {e}"))?;
        let exists: bool = ws.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='investigation_cases')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if !exists {
            return Ok(0);
        }
        ws.query_row(
            "SELECT COUNT(DISTINCT connection_id) FROM investigation_cases",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
    }
    pub fn storage_health(&self) -> Result<StorageHealth, String> {
        let path = self.main_db_path()?;
        let now = unix_time_millis()?;
        let since = now - 7 * 86_400_000;
        let mut sections = Vec::with_capacity(SECTIONS.len());
        let mut week_bytes: u64 = 0;
        for (kind, table) in SECTIONS {
            // Table names are hard-coded constants, never user input.
            let sql = format!("SELECT COUNT(*), COALESCE(SUM(LENGTH(snapshot_json)),0), COALESCE(SUM(CASE WHEN stored_at_ms >= ?1 THEN LENGTH(snapshot_json) ELSE 0 END),0), MIN(stored_at_ms), MAX(stored_at_ms) FROM {table}");
            let (count, json_bytes, last_week_bytes, oldest_at_ms, newest_at_ms): (
                i64,
                i64,
                i64,
                Option<i64>,
                Option<i64>,
            ) = self
                .connection
                .query_row(&sql, [since], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .map_err(|e| e.to_string())?;
            let pinned_count: i64 = self
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM snapshot_protections WHERE section=?1",
                    [kind],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            week_bytes += last_week_bytes as u64;
            sections.push(StorageSection {
                kind: kind.to_string(),
                count,
                json_bytes,
                last_week_bytes,
                oldest_at_ms,
                newest_at_ms,
                pinned_count,
            });
        }
        let policy = self.storage_policy()?;
        let (live_bytes_estimate, reusable_bytes) = self.space()?;
        let projected_days_to_budget = policy.max_bytes.and_then(|target| {
            if week_bytes == 0 || target <= live_bytes_estimate {
                None
            } else {
                Some((target - live_bytes_estimate) as f64 * 7. / week_bytes as f64)
            }
        });
        let last_maintenance_ms = self
            .setting("history_last_maintenance_ms")?
            .and_then(|s| s.parse().ok());
        Ok(StorageHealth {
            db_bytes: Self::file_bytes(&path),
            wal_bytes: Self::file_bytes(&PathBuf::from(format!("{}-wal", path.display()))),
            shm_bytes: Self::file_bytes(&PathBuf::from(format!("{}-shm", path.display()))),
            live_bytes_estimate,
            reusable_bytes,
            last_maintenance_ms,
            policy,
            sections,
            case_protected_connections: self.case_count()?,
            projected_days_to_budget,
        })
    }
    /// Lock workspace writer before checking Case references; hold through pruning.
    /// Fail closed if workspace database does not exist/is unreadable/uninitialized.
    pub(super) fn refresh_protected_connections(&mut self) -> Result<Connection, String> {
        let workspace = self.main_db_path()?.with_file_name("workspace.sqlite3");
        if !workspace.exists() {
            return Err(crate::i18n::text(
                "Workspace SQLite ещё не создан: очистка запрещена",
                "Workspace SQLite does not exist yet; cleanup is prohibited",
            )
            .into());
        }
        let ws = Connection::open(&workspace)
            .map_err(|e| format!("Workspace недоступен, очистка запрещена: {e}"))?;
        ws.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        ws.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("Не удалось заблокировать Case metadata для очистки: {e}"))?;
        let rows: Vec<i64> = {
            let mut stmt = ws
                .prepare("SELECT DISTINCT connection_id FROM investigation_cases")
                .map_err(|e| format!("Нет проверенной схемы Cases: {e}"))?;
            // Finish consuming the mapped rows before `stmt` is dropped.
            let mapped = stmt
                .query_map([], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            mapped
                .collect::<rusqlite::Result<Vec<i64>>>()
                .map_err(|e| e.to_string())?
        };
        self.connection.execute_batch("CREATE TEMP TABLE IF NOT EXISTS protected_connections (connection_id INTEGER PRIMARY KEY); DELETE FROM protected_connections;")
            .map_err(|e| e.to_string())?;
        for id in rows {
            self.connection
                .execute(
                    "INSERT OR IGNORE INTO protected_connections VALUES(?1)",
                    [id],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(ws)
    }
    pub fn protect_snapshot(
        &mut self,
        section: &str,
        snapshot_id: i64,
        protect: bool,
    ) -> Result<(), String> {
        let table = SECTIONS
            .iter()
            .find(|(kind, _)| *kind == section)
            .map(|(_, table)| *table)
            .ok_or("Неизвестный вид снимка")?;
        if snapshot_id <= 0 {
            return Err("Некорректный snapshot ID".into());
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if protect {
            let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1)");
            let exists: bool = tx
                .query_row(&sql, [snapshot_id], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if !exists {
                return Err("Снимок не найден".into());
            }
            tx.execute(
                "INSERT OR IGNORE INTO snapshot_protections(section,snapshot_id) VALUES(?1,?2)",
                params![section, snapshot_id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            tx.execute(
                "DELETE FROM snapshot_protections WHERE section=?1 AND snapshot_id=?2",
                params![section, snapshot_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Budget is for occupied SQLite pages, not the un-compacted physical file.
    /// Deletes at most 300 snapshots per run and never touches protected rows.
    pub fn prune_budget(&mut self) -> Result<StorageMaintenance, String> {
        let Some(budget) = self.storage_policy()?.max_bytes else {
            return Ok(StorageMaintenance {
                action: "prune".into(),
                deleted: 0,
                detail: "Лимит не задан".into(),
            });
        };
        let _workspace_guard = self.refresh_protected_connections()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut deleted = 0_u64;
        for _ in 0..300 {
            let page_count: u64 = tx
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let freelist: u64 = tx
                .query_row("PRAGMA freelist_count", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let size: u64 = tx
                .query_row("PRAGMA page_size", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if (page_count - freelist) * size <= budget {
                break;
            }
            let mut union = Vec::new();
            for (section, table) in SECTIONS {
                union.push(format!("SELECT '{section}' AS section, id, stored_at_ms FROM {table} WHERE connection_id NOT IN (SELECT connection_id FROM protected_connections) AND id NOT IN (SELECT snapshot_id FROM snapshot_protections WHERE section='{section}')"));
            }
            let query = format!(
                "SELECT section,id FROM ({}) ORDER BY stored_at_ms LIMIT 1",
                union.join(" UNION ALL ")
            );
            let oldest: Option<(String, i64)> = tx
                .query_row(&query, [], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .map_err(|e| e.to_string())?;
            let Some((section, id)) = oldest else {
                break;
            };
            let table = SECTIONS
                .iter()
                .find(|(name, _)| *name == section)
                .expect("known section")
                .1;
            deleted += tx
                .execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])
                .map_err(|e| e.to_string())? as u64;
        }
        tx.commit().map_err(|e| e.to_string())?;
        let blocked = self.space()?.0 > budget;
        Ok(StorageMaintenance {action:"prune".into(),deleted,detail: (if blocked {
            crate::i18n::text("Лимит ещё превышен: нужны дополнительные проходы или все оставшиеся снимки защищены. Физический файл сжимается отдельно через VACUUM.", "Storage is still over budget: additional passes are needed, or all remaining snapshots are protected. VACUUM compacts the physical file separately.")
        } else {crate::i18n::text("Лимит занятого пространства достигнут", "Storage is within the configured budget")}).into()})
    }
    pub fn run_storage_maintenance(&mut self, action: &str) -> Result<StorageMaintenance, String> {
        match action {
            "analyze" => {
                self.connection
                    .execute_batch("ANALYZE; PRAGMA optimize;")
                    .map_err(|e| e.to_string())?;
                Ok(StorageMaintenance {
                    action: action.into(),
                    deleted: 0,
                    detail: crate::i18n::text(
                        "ANALYZE и PRAGMA optimize завершены",
                        "ANALYZE and PRAGMA optimize completed",
                    )
                    .into(),
                })
            }
            "check" => {
                self.integrity_check()?;
                Ok(StorageMaintenance {
                    action: action.into(),
                    deleted: 0,
                    detail: "SQLite quick_check: ok".into(),
                })
            }
            "checkpoint" => {
                let (busy, _, _): (i64, i64, i64) = self
                    .connection
                    .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .map_err(|e| e.to_string())?;
                Ok(StorageMaintenance {
                    action: action.into(),
                    deleted: 0,
                    detail: if busy > 0 {
                        crate::i18n::text(
                            "WAL занят другим соединением, повтори позже",
                            "WAL is busy with another connection; try again later",
                        )
                    } else {
                        crate::i18n::text("WAL checkpoint выполнен", "WAL checkpoint completed")
                    }
                    .into(),
                })
            }
            "vacuum" => {
                self.connection.execute_batch("VACUUM").map_err(|e| {
                    if crate::i18n::is_ru() {
                        format!("VACUUM не выполнен (возможно, хранилище занято): {e}")
                    } else {
                        format!("VACUUM failed (the storage may be busy): {e}")
                    }
                })?;
                Ok(StorageMaintenance {
                    action: action.into(),
                    deleted: 0,
                    detail: crate::i18n::text(
                        "VACUUM завершён; обнови показатели размера",
                        "VACUUM completed; refresh the size metrics",
                    )
                    .into(),
                })
            }
            "retention" => {
                let result = self.apply_history_retention()?;
                Ok(StorageMaintenance {
                    action: action.into(),
                    deleted: result.total_deleted as u64,
                    detail: crate::i18n::text("Удалены только незакреплённые снимки; Case-подключения защищены", "Only unpinned snapshots were deleted; connections with Cases are protected")
                        .into(),
                })
            }
            "prune" => self.prune_budget(),
            _ => Err(crate::i18n::text(
                "Неизвестная операция обслуживания SQLite",
                "Unknown SQLite maintenance operation",
            )
            .into()),
        }
    }
    pub fn maintenance_tick(&mut self) -> Result<StorageMaintenance, String> {
        let policy = self.storage_policy()?;
        if !policy.auto_prune {
            return Ok(StorageMaintenance {
                action: "scheduled".into(),
                deleted: 0,
                detail: crate::i18n::text("Автоочистка отключена", "Automatic cleanup is disabled")
                    .into(),
            });
        }
        let now = unix_time_millis()?;
        let last = self
            .setting("history_last_maintenance_ms")?
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        if now.saturating_sub(last) < 86_400_000 {
            return Ok(StorageMaintenance {
                action: "scheduled".into(),
                deleted: 0,
                detail: crate::i18n::text(
                    "Следующий проход позже",
                    "Next cleanup pass is scheduled for later",
                )
                .into(),
            });
        }
        // Do not advance the timestamp on failure: retry safely on the next tick.
        let retention = self.apply_history_retention()?;
        let budget = self.prune_budget()?;
        self.store_setting("history_last_maintenance_ms", Some(now.to_string()))?;
        Ok(StorageMaintenance {
            action: "scheduled".into(),
            deleted: (retention.total_deleted as u64) + budget.deleted,
            detail: budget.detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn sqlite_migration_is_idempotent_and_versioned() {
        let connection = Connection::open_in_memory().unwrap();
        migrate_diagnostics_schema(&connection).unwrap();
        migrate_diagnostics_schema(&connection).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let count:i64=connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('snapshot_schedules','snapshot_schedule_claims','snapshot_protections')",[],|r|r.get(0)).unwrap();
        assert_eq!(count, 3);
    }
    #[test]
    fn retention_preserves_case_connections_and_pinned_snapshots() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::env::temp_dir().join(format!(
            "pgxray-history-protection-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&folder).unwrap();
        let db_path = folder.join("diagnostics.sqlite3");
        let workspace = folder.join("workspace.sqlite3");
        let ws = Connection::open(&workspace).unwrap();
        ws.execute_batch("CREATE TABLE investigation_cases(connection_id INTEGER NOT NULL); INSERT INTO investigation_cases VALUES(1);").unwrap();
        drop(ws);
        let connection = Connection::open(&db_path).unwrap();
        connection.execute_batch(r#"
            CREATE TABLE diagnostic_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE snapshot_protections(section TEXT NOT NULL,snapshot_id INTEGER NOT NULL,PRIMARY KEY(section,snapshot_id));
            CREATE TABLE change_markers(connection_id INTEGER NOT NULL);
            CREATE TABLE xid_snapshots(id INTEGER PRIMARY KEY,connection_id INTEGER NOT NULL,stored_at_ms INTEGER NOT NULL,snapshot_json TEXT NOT NULL);
            CREATE TABLE pgss_snapshots(id INTEGER PRIMARY KEY,connection_id INTEGER NOT NULL,stored_at_ms INTEGER NOT NULL,snapshot_json TEXT NOT NULL);
            CREATE TABLE autovacuum_snapshots(id INTEGER PRIMARY KEY,connection_id INTEGER NOT NULL,stored_at_ms INTEGER NOT NULL,snapshot_json TEXT NOT NULL);
            CREATE TABLE pg_settings_snapshots(id INTEGER PRIMARY KEY,connection_id INTEGER NOT NULL,stored_at_ms INTEGER NOT NULL,snapshot_json TEXT NOT NULL);
            CREATE TABLE io_wal_snapshots(id INTEGER PRIMARY KEY,connection_id INTEGER NOT NULL,stored_at_ms INTEGER NOT NULL,snapshot_json TEXT NOT NULL);
            CREATE TABLE index_health_snapshots(id INTEGER PRIMARY KEY,connection_id INTEGER NOT NULL,stored_at_ms INTEGER NOT NULL,snapshot_json TEXT NOT NULL);
            INSERT INTO xid_snapshots VALUES(1,1,1,'{}'),(2,2,1,'{}'),(3,3,1,'{}');
            INSERT INTO snapshot_protections VALUES('xid',3);
        "#).unwrap();
        let mut store = DiagnosticStore { connection };
        let removed = store.set_history_retention(Some(1)).unwrap();
        assert_eq!(removed.total_deleted, 1);
        let survivors: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM xid_snapshots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(survivors, 2);
        assert!(store.delete_connection_history(1).is_err());
        assert!(store.delete_connection_history(3).is_err());
        drop(store);
        let _ = fs::remove_dir_all(folder);
    }
}
