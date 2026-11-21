mod autovacuum;
mod index_health;
mod io_wal;
mod management;
mod pgss;
mod scheduler_store;
mod settings;
mod what_changed;
mod what_changed_maintenance;
mod what_changed_storage;
mod xid;

pub use autovacuum::{
    AutovacuumComparison, AutovacuumRelationDelta, AutovacuumRelationHistory,
    AutovacuumRelationHistoryPoint, AutovacuumRelationHistorySummary, AutovacuumSnapshotSummary,
    AutovacuumWorkloadHistory, AutovacuumWorkloadInterval, AutovacuumWorkloadRelation,
    CollectedAutovacuumSnapshot,
};
pub use index_health::{
    CollectedIndexHealthSnapshot, IndexHealthComparison, IndexHealthDelta,
    IndexHealthSnapshotSummary,
};
pub use io_wal::{
    CollectedIoWalSnapshot, IoWalComparison, IoWalHistorySizePoint, IoWalRelationSizeChange,
    IoWalSnapshotSummary, IoWalWorkloadHistory, IoWalWorkloadInterval,
};
pub use management::{StorageHealth, StorageMaintenance, StoragePolicy, StorageSection};
pub use pgss::{
    CollectedPgssSnapshot, PgssCallTargetDelta, PgssCommandDelta, PgssComparison,
    PgssRoleDatabaseDelta, PgssSnapshotSummary, PgssStatementHistory, PgssStatementHistoryPoint,
    PgssStatementHistorySummary, PgssWorkloadHistory, PgssWorkloadInterval, PgssWorkloadMetrics,
    PgssWorkloadStatement,
};
pub use scheduler_store::ScheduleRule;
pub use settings::{
    CollectedPgSettingsSnapshot, PgSettingChange, PgSettingsChangeEvent, PgSettingsComparison,
    PgSettingsHistoryAnalysis, PgSettingsHistoryInterval, PgSettingsSnapshotSummary,
};
pub use what_changed::{
    ChangeMarker, PgssPeriodQuality, PgssStatementPeriodMetrics, WhatChangedComparison,
    WhatChangedMetricChange, WhatChangedPeriod, WhatChangedPgssComparison,
    WhatChangedSettingChange, WhatChangedSettingsComparison, WhatChangedStatementChange,
    WhatChangedTarget, WhatChangedTimelineEvent,
};
pub use what_changed_maintenance::{
    AutovacuumRelationPeriodMetrics, MaintenancePeriodQuality, WhatChangedAutovacuumComparison,
    WhatChangedAutovacuumRelationChange, WhatChangedXidComparison, WhatChangedXidRelationChange,
    XidPeriodSummary, XidRelationPeriodMetrics,
};

pub use what_changed_storage::{
    StoragePeriodMetrics, StoragePeriodQuality, WhatChangedIndexHealthChange,
    WhatChangedIndexHealthComparison, WhatChangedRelationSizeChange, WhatChangedStorageComparison,
};
pub use xid::{
    CollectedXidSnapshot, XidBlockerComparison, XidComparison, XidHistoryOverviewPoint,
    XidOldestRelationComparison, XidRate, XidRelationDelta, XidRelationHistoryPoint,
    XidRelationSummary, XidSnapshotSummary, XidVacuumComparison,
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_HISTORY_LIMIT: u32 = 100;
const MAX_HISTORY_LIMIT: u32 = 500;

pub struct DiagnosticStore {
    connection: Connection,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryRetentionSettings {
    pub retention_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryMaintenanceResult {
    pub xid_deleted: usize,
    pub pgss_deleted: usize,
    pub autovacuum_deleted: usize,
    pub pg_settings_deleted: usize,
    pub io_wal_deleted: usize,
    pub index_health_deleted: usize,
    pub total_deleted: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistorySectionCounts {
    pub xid: i64,
    pub pgss: i64,
    pub autovacuum: i64,
    pub pg_settings: i64,
    pub io_wal: i64,
    pub index_health: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryConnectionSummary {
    pub connection_id: i64,
    pub connection_name: String,
    pub snapshot_count: i64,
    pub oldest_at_ms: i64,
    pub newest_at_ms: i64,
    pub section_counts: HistorySectionCounts,
}

impl HistoryMaintenanceResult {
    fn new(
        xid_deleted: usize,
        pgss_deleted: usize,
        autovacuum_deleted: usize,
        pg_settings_deleted: usize,
        io_wal_deleted: usize,
        index_health_deleted: usize,
    ) -> Self {
        Self {
            xid_deleted,
            pgss_deleted,
            autovacuum_deleted,
            pg_settings_deleted,
            io_wal_deleted,
            index_health_deleted,
            total_deleted: xid_deleted
                + pgss_deleted
                + autovacuum_deleted
                + pg_settings_deleted
                + io_wal_deleted
                + index_health_deleted,
        }
    }
}

impl DiagnosticStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path)
            .map_err(|error| format!("Не удалось открыть историю диагностики: {error}"))?;

        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| format!("Не удалось настроить busy timeout истории: {error}"))?;

        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                "#,
            )
            .map_err(|error| {
                format!("Не удалось настроить SQLite для истории диагностики: {error}")
            })?;

        xid::init_schema(&connection)?;

        pgss::init_schema(&connection)?;

        autovacuum::init_schema(&connection)?;

        settings::init_schema(&connection)?;

        io_wal::init_schema(&connection)?;

        index_health::init_schema(&connection)?;

        what_changed::init_schema(&connection)?;

        connection
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS diagnostic_settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_xid_snapshots_stored_at
                    ON xid_snapshots (stored_at_ms);

                CREATE INDEX IF NOT EXISTS idx_pgss_snapshots_stored_at
                    ON pgss_snapshots (stored_at_ms);

                CREATE INDEX IF NOT EXISTS idx_autovacuum_snapshots_stored_at
                    ON autovacuum_snapshots (stored_at_ms);

                CREATE INDEX IF NOT EXISTS idx_pg_settings_snapshots_stored_at
                    ON pg_settings_snapshots (stored_at_ms);
                "#,
            )
            .map_err(|error| {
                format!("Не удалось подготовить настройки хранения истории: {error}")
            })?;

        management::migrate_diagnostics_schema(&connection)?;
        let store = Self { connection };

        // Deletion is deferred until a workspace protection check can succeed.
        // Never prune on startup before workspace.sqlite3 has been initialized.
        // Maintenance is invoked explicitly or by the periodic maintenance tick.

        Ok(store)
    }

    pub fn integrity_check(&self) -> Result<(), String> {
        let result: String = self
            .connection
            .query_row(crate::SQLITE_QUICK_CHECK_SQL, [], |row| row.get(0))
            .map_err(|error| format!("Не удалось проверить diagnostics SQLite: {error}"))?;

        if result == "ok" {
            Ok(())
        } else {
            Err(format!("diagnostics SQLite quick_check: {result}"))
        }
    }

    pub fn history_retention_settings(&self) -> Result<HistoryRetentionSettings, String> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM diagnostic_settings WHERE key = 'history_retention_days'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать retention истории: {error}"))?;

        let retention_days = match value {
            Some(value) => {
                let days = value.parse::<u32>().map_err(|_| {
                    crate::i18n::text(
                        "Некорректное значение retention в diagnostics.sqlite3",
                        "Invalid retention value in diagnostics.sqlite3",
                    )
                })?;

                Some(validate_retention_days(days)?)
            }
            None => None,
        };

        Ok(HistoryRetentionSettings { retention_days })
    }

    pub fn set_history_retention(
        &mut self,
        retention_days: Option<u32>,
    ) -> Result<HistoryMaintenanceResult, String> {
        match retention_days {
            Some(days) => {
                let days = validate_retention_days(days)?;

                self.connection
                    .execute(
                        r#"
                        INSERT INTO diagnostic_settings (key, value)
                        VALUES ('history_retention_days', ?1)
                        ON CONFLICT(key) DO UPDATE SET value = excluded.value
                        "#,
                        [days.to_string()],
                    )
                    .map_err(|error| format!("Не удалось сохранить retention истории: {error}"))?;
            }
            None => {
                self.connection
                    .execute(
                        "DELETE FROM diagnostic_settings WHERE key = 'history_retention_days'",
                        [],
                    )
                    .map_err(|error| format!("Не удалось отключить retention истории: {error}"))?;
            }
        }

        self.apply_history_retention()
    }

    pub fn apply_history_retention(&mut self) -> Result<HistoryMaintenanceResult, String> {
        let settings = self.history_retention_settings()?;

        let Some(days) = settings.retention_days else {
            return Ok(HistoryMaintenanceResult::new(0, 0, 0, 0, 0, 0));
        };

        let now_ms = unix_time_millis()?;

        let age_ms = i64::from(days)
            .checked_mul(86_400_000)
            .ok_or(crate::i18n::text(
                "Retention истории слишком велик",
                "History retention is too large",
            ))?;

        let cutoff_ms = now_ms.checked_sub(age_ms).ok_or(crate::i18n::text(
            "Не удалось вычислить retention cutoff",
            "Failed to calculate the retention cutoff",
        ))?;

        self.delete_history_before(cutoff_ms)
    }

    pub fn delete_connection_history(
        &mut self,
        connection_id: i64,
    ) -> Result<HistoryMaintenanceResult, String> {
        if connection_id <= 0 {
            return Err(
                crate::i18n::text("Некорректный connection id", "Invalid connection ID").into(),
            );
        }

        let _workspace_guard = self.refresh_protected_connections()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("Не удалось начать удаление истории: {error}"))?;
        if tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM protected_connections WHERE connection_id = ?1)",
                [connection_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|e| e.to_string())?
        {
            return Err(crate::i18n::text(
                "У подключения есть Investigation Cases. Удаление истории запрещено.",
                "This connection has Investigation Cases. History deletion is prohibited.",
            )
            .into());
        }
        const PINNED: &[(&str, &str)] = &[
            ("xid", "xid_snapshots"),
            ("pgss", "pgss_snapshots"),
            ("autovacuum", "autovacuum_snapshots"),
            ("pg_settings", "pg_settings_snapshots"),
            ("io_wal", "io_wal_snapshots"),
            ("index_health", "index_health_snapshots"),
        ];
        for (kind, table) in PINNED {
            let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} s JOIN snapshot_protections p ON p.section = ?1 AND p.snapshot_id = s.id WHERE s.connection_id = ?2)");
            let pinned: bool = tx
                .query_row(&sql, params![kind, connection_id], |row| row.get(0))
                .map_err(|error| error.to_string())?;
            if pinned {
                return Err(crate::i18n::text("Подключение содержит закреплённые снимки. Сними закрепление перед удалением истории.", "This connection contains pinned snapshots. Unpin them before deleting history.").into());
            }
        }

        let xid_deleted = tx
            .execute(
                "DELETE FROM xid_snapshots WHERE connection_id = ?1",
                params![connection_id],
            )
            .map_err(|error| format!("Не удалось удалить XID history: {error}"))?;

        let pgss_deleted = tx
            .execute(
                "DELETE FROM pgss_snapshots WHERE connection_id = ?1",
                params![connection_id],
            )
            .map_err(|error| format!("Не удалось удалить PGSS history: {error}"))?;

        let autovacuum_deleted = tx
            .execute(
                "DELETE FROM autovacuum_snapshots WHERE connection_id = ?1",
                params![connection_id],
            )
            .map_err(|error| format!("Не удалось удалить Autovacuum history: {error}"))?;

        let pg_settings_deleted = tx
            .execute(
                "DELETE FROM pg_settings_snapshots WHERE connection_id = ?1",
                params![connection_id],
            )
            .map_err(|error| format!("Не удалось удалить pg_settings history: {error}"))?;

        let io_wal_deleted = tx
            .execute(
                "DELETE FROM io_wal_snapshots WHERE connection_id = ?1",
                params![connection_id],
            )
            .map_err(|error| format!("Не удалось удалить I/O & WAL history: {error}"))?;

        let index_health_deleted = tx
            .execute(
                "DELETE FROM index_health_snapshots WHERE connection_id = ?1",
                params![connection_id],
            )
            .map_err(|error| format!("Не удалось удалить Index Health history: {error}"))?;

        tx.execute(
            "DELETE FROM change_markers WHERE connection_id = ?1",
            params![connection_id],
        )
        .map_err(|error| format!("Не удалось удалить manual change markers: {error}"))?;

        tx.commit()
            .map_err(|error| format!("Не удалось завершить удаление истории: {error}"))?;

        Ok(HistoryMaintenanceResult::new(
            xid_deleted,
            pgss_deleted,
            autovacuum_deleted,
            pg_settings_deleted,
            io_wal_deleted,
            index_health_deleted,
        ))
    }

    pub fn list_history_connections(&self) -> Result<Vec<HistoryConnectionSummary>, String> {
        let mut statement = self
            .connection
            .prepare(
                r#"
                WITH all_snapshots AS (
                    SELECT connection_id, connection_name, stored_at_ms, 'xid' AS section
                    FROM xid_snapshots
                    UNION ALL
                    SELECT connection_id, connection_name, stored_at_ms, 'pgss' AS section
                    FROM pgss_snapshots
                    UNION ALL
                    SELECT connection_id, connection_name, stored_at_ms, 'autovacuum' AS section
                    FROM autovacuum_snapshots
                    UNION ALL
                    SELECT connection_id, connection_name, stored_at_ms, 'pg_settings' AS section
                    FROM pg_settings_snapshots
                    UNION ALL
                    SELECT connection_id, connection_name, stored_at_ms, 'io_wal' AS section
                    FROM io_wal_snapshots
                    UNION ALL
                    SELECT connection_id, connection_name, stored_at_ms, 'index_health' AS section
                    FROM index_health_snapshots
                )
                SELECT
                    a.connection_id,
                    (
                        SELECT b.connection_name
                        FROM all_snapshots AS b
                        WHERE b.connection_id = a.connection_id
                        ORDER BY b.stored_at_ms DESC
                        LIMIT 1
                    ) AS connection_name,
                    COUNT(*) AS snapshot_count,
                    MIN(a.stored_at_ms) AS oldest_at_ms,
                    MAX(a.stored_at_ms) AS newest_at_ms,
                    SUM(CASE WHEN a.section = 'xid' THEN 1 ELSE 0 END) AS xid_count,
                    SUM(CASE WHEN a.section = 'pgss' THEN 1 ELSE 0 END) AS pgss_count,
                    SUM(CASE WHEN a.section = 'autovacuum' THEN 1 ELSE 0 END) AS autovacuum_count,
                    SUM(CASE WHEN a.section = 'pg_settings' THEN 1 ELSE 0 END) AS pg_settings_count,
                    SUM(CASE WHEN a.section = 'io_wal' THEN 1 ELSE 0 END) AS io_wal_count,
                    SUM(CASE WHEN a.section = 'index_health' THEN 1 ELSE 0 END) AS index_health_count
                FROM all_snapshots AS a
                GROUP BY a.connection_id
                ORDER BY newest_at_ms DESC, a.connection_id DESC
                "#,
            )
            .map_err(|error| {
                format!("Не удалось подготовить список history connections: {error}")
            })?;

        let rows = statement
            .query_map([], |row| {
                Ok(HistoryConnectionSummary {
                    connection_id: row.get(0)?,
                    connection_name: row.get(1)?,
                    snapshot_count: row.get(2)?,
                    oldest_at_ms: row.get(3)?,
                    newest_at_ms: row.get(4)?,
                    section_counts: HistorySectionCounts {
                        xid: row.get(5)?,
                        pgss: row.get(6)?,
                        autovacuum: row.get(7)?,
                        pg_settings: row.get(8)?,
                        io_wal: row.get(9)?,
                        index_health: row.get(10)?,
                    },
                })
            })
            .map_err(|error| format!("Не удалось прочитать history connections: {error}"))?;

        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    fn delete_history_before(
        &mut self,
        cutoff_ms: i64,
    ) -> Result<HistoryMaintenanceResult, String> {
        let _workspace_guard = self.refresh_protected_connections()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("Не удалось начать retention cleanup: {error}"))?;

        let xid_deleted = tx
            .execute(
                "DELETE FROM xid_snapshots WHERE stored_at_ms < ?1 AND connection_id NOT IN (SELECT connection_id FROM protected_connections) AND id NOT IN (SELECT snapshot_id FROM snapshot_protections WHERE section='xid')",
                params![cutoff_ms],
            )
            .map_err(|error| format!("Не удалось очистить старую XID history: {error}"))?;

        let pgss_deleted = tx
            .execute(
                "DELETE FROM pgss_snapshots WHERE stored_at_ms < ?1 AND connection_id NOT IN (SELECT connection_id FROM protected_connections) AND id NOT IN (SELECT snapshot_id FROM snapshot_protections WHERE section='pgss')",
                params![cutoff_ms],
            )
            .map_err(|error| format!("Не удалось очистить старую PGSS history: {error}"))?;

        let autovacuum_deleted = tx
            .execute(
                "DELETE FROM autovacuum_snapshots WHERE stored_at_ms < ?1 AND connection_id NOT IN (SELECT connection_id FROM protected_connections) AND id NOT IN (SELECT snapshot_id FROM snapshot_protections WHERE section='autovacuum')",
                params![cutoff_ms],
            )
            .map_err(|error| format!("Не удалось очистить старую Autovacuum history: {error}"))?;

        let pg_settings_deleted = tx
            .execute(
                "DELETE FROM pg_settings_snapshots WHERE stored_at_ms < ?1 AND connection_id NOT IN (SELECT connection_id FROM protected_connections) AND id NOT IN (SELECT snapshot_id FROM snapshot_protections WHERE section='pg_settings')",
                params![cutoff_ms],
            )
            .map_err(|error| format!("Не удалось очистить старую pg_settings history: {error}"))?;

        let io_wal_deleted = tx
            .execute(
                "DELETE FROM io_wal_snapshots WHERE stored_at_ms < ?1 AND connection_id NOT IN (SELECT connection_id FROM protected_connections) AND id NOT IN (SELECT snapshot_id FROM snapshot_protections WHERE section='io_wal')",
                params![cutoff_ms],
            )
            .map_err(|error| format!("Не удалось очистить старую I/O & WAL history: {error}"))?;

        let index_health_deleted = tx
            .execute(
                "DELETE FROM index_health_snapshots WHERE stored_at_ms < ?1 AND connection_id NOT IN (SELECT connection_id FROM protected_connections) AND id NOT IN (SELECT snapshot_id FROM snapshot_protections WHERE section='index_health')",
                params![cutoff_ms],
            )
            .map_err(|error| format!("Не удалось очистить старую Index Health history: {error}"))?;

        tx.commit()
            .map_err(|error| format!("Не удалось завершить retention cleanup: {error}"))?;

        Ok(HistoryMaintenanceResult::new(
            xid_deleted,
            pgss_deleted,
            autovacuum_deleted,
            pg_settings_deleted,
            io_wal_deleted,
            index_health_deleted,
        ))
    }
}

fn validate_retention_days(days: u32) -> Result<u32, String> {
    if !(1..=36_500).contains(&days) {
        return Err(crate::i18n::text(
            "Retention истории должен быть от 1 до 36500 дней",
            "History retention must be between 1 and 36500 days",
        )
        .into());
    }

    Ok(days)
}

pub fn unix_time_millis() -> Result<i64, String> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        crate::i18n::text(
            "Системное время раньше Unix epoch",
            "System time is before the Unix epoch",
        )
        .to_owned()
    })?;

    i64::try_from(duration.as_millis()).map_err(|_| {
        crate::i18n::text(
            "Системное время не помещается в SQLite INTEGER",
            "System time does not fit in a SQLite INTEGER",
        )
        .to_owned()
    })
}

#[cfg(test)]
mod history_summary_tests {
    use super::DiagnosticStore;
    use rusqlite::Connection;

    #[test]
    fn lists_snapshot_counts_for_every_section_and_connection() {
        // Minimal schemas exercise the real aggregate query without production data.
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
            CREATE TABLE xid_snapshots (connection_id INTEGER, connection_name TEXT, stored_at_ms INTEGER);
            CREATE TABLE pgss_snapshots (connection_id INTEGER, connection_name TEXT, stored_at_ms INTEGER);
            CREATE TABLE autovacuum_snapshots (connection_id INTEGER, connection_name TEXT, stored_at_ms INTEGER);
            CREATE TABLE pg_settings_snapshots (connection_id INTEGER, connection_name TEXT, stored_at_ms INTEGER);
            CREATE TABLE io_wal_snapshots (connection_id INTEGER, connection_name TEXT, stored_at_ms INTEGER);
            CREATE TABLE index_health_snapshots (connection_id INTEGER, connection_name TEXT, stored_at_ms INTEGER);

            INSERT INTO xid_snapshots VALUES (1, 'old name', 100), (1, 'old name', 110);
            INSERT INTO pgss_snapshots VALUES (1, 'new name', 200), (2, 'other', 300);
            INSERT INTO autovacuum_snapshots VALUES (1, 'new name', 210);
            INSERT INTO pg_settings_snapshots VALUES (1, 'new name', 220);
            INSERT INTO io_wal_snapshots VALUES (1, 'new name', 230);
            INSERT INTO index_health_snapshots VALUES (1, 'new name', 240);
            "#,
            )
            .unwrap();
        let store = DiagnosticStore { connection };
        let rows = store.list_history_connections().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].connection_id, 2);
        assert_eq!(rows[0].snapshot_count, 1);
        assert_eq!(rows[0].section_counts.pgss, 1);

        let first = &rows[1];
        assert_eq!(first.connection_id, 1);
        assert_eq!(first.connection_name, "new name");
        assert_eq!(first.snapshot_count, 7);
        assert_eq!(first.oldest_at_ms, 100);
        assert_eq!(first.newest_at_ms, 240);
        assert_eq!(first.section_counts.xid, 2);
        assert_eq!(first.section_counts.pgss, 1);
        assert_eq!(first.section_counts.autovacuum, 1);
        assert_eq!(first.section_counts.pg_settings, 1);
        assert_eq!(first.section_counts.io_wal, 1);
        assert_eq!(first.section_counts.index_health, 1);
    }
}
