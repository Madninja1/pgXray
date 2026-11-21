use super::{unix_time_millis, DiagnosticStore, DEFAULT_HISTORY_LIMIT, MAX_HISTORY_LIMIT};
use crate::{
    connections::SavedConnection,
    postgres_settings::{PgSetting, PgSettingsSnapshot},
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

const PG_SETTINGS_SNAPSHOT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
pub struct CollectedPgSettingsSnapshot {
    pub history: PgSettingsSnapshotSummary,
    pub snapshot: PgSettingsSnapshot,
    pub comparison: Option<PgSettingsComparison>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgSettingsSnapshotSummary {
    pub id: i64,
    pub schema_version: i64,
    pub app_version: String,
    pub connection_id: i64,
    pub connection_name: String,
    pub host: String,
    pub port: i64,
    pub database: String,
    pub database_oid: i64,
    pub username: String,
    pub sslmode: String,
    pub server_version: String,
    pub server_version_num: i32,
    pub postmaster_start_ms: i64,
    pub collection_started_at_ms: i64,
    pub collection_completed_at_ms: i64,
    pub measured_at: String,
    pub setting_count: i64,
    pub non_default_count: i64,
    pub pending_restart_count: i64,
    pub changed_from_previous: i64,
    pub stored_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgSettingChange {
    pub name: String,
    pub category: String,
    pub state: String,
    pub older_value: Option<String>,
    pub newer_value: Option<String>,
    pub older_unit: Option<String>,
    pub newer_unit: Option<String>,
    pub older_reset_value: Option<String>,
    pub newer_reset_value: Option<String>,
    pub older_source: Option<String>,
    pub newer_source: Option<String>,
    pub older_sourcefile: Option<String>,
    pub newer_sourcefile: Option<String>,
    pub older_sourceline: Option<i64>,
    pub newer_sourceline: Option<i64>,
    pub older_pending_restart: Option<bool>,
    pub newer_pending_restart: Option<bool>,
    pub value_changed: bool,
    pub reset_value_changed: bool,
    pub source_changed: bool,
    pub pending_restart_changed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgSettingsChangeEvent {
    pub detected_at_ms: i64,
    pub changed_after_ms: i64,
    pub changed_before_ms: i64,
    pub older_snapshot_id: i64,
    pub newer_snapshot_id: i64,
    pub database: String,
    pub username: String,
    pub change: PgSettingChange,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgSettingsComparison {
    pub older: PgSettingsSnapshotSummary,
    pub newer: PgSettingsSnapshotSummary,
    pub elapsed_seconds: f64,
    pub value_change_count: i64,
    pub source_change_count: i64,
    pub pending_restart_change_count: i64,
    pub added_count: i64,
    pub removed_count: i64,
    pub changes: Vec<PgSettingChange>,
}

/// Consecutive observed configuration states. A restart is kept visible as a
/// separate state: settings are still comparable, but collection was interrupted.
#[derive(Clone, Debug, Serialize)]
pub struct PgSettingsHistoryInterval {
    pub from_snapshot_id: i64,
    pub to_snapshot_id: i64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub state: String,
    pub reason: Option<String>,
    pub value_change_count: i64,
    pub source_change_count: i64,
    pub pending_restart_change_count: i64,
    pub changes: Vec<PgSettingChange>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgSettingsHistoryAnalysis {
    pub snapshot_count: usize,
    pub intervals: Vec<PgSettingsHistoryInterval>,
    pub limitations: Vec<String>,
}

#[derive(Clone)]
struct StoredPgSettingsSnapshot {
    summary: PgSettingsSnapshotSummary,
    snapshot: PgSettingsSnapshot,
}

pub(super) fn init_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS pg_settings_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                schema_version INTEGER NOT NULL,
                app_version TEXT NOT NULL,
                connection_id INTEGER NOT NULL,
                connection_name TEXT NOT NULL,
                host TEXT NOT NULL,
                port INTEGER NOT NULL,
                database_name TEXT NOT NULL,
                database_oid INTEGER NOT NULL,
                username TEXT NOT NULL,
                sslmode TEXT NOT NULL,
                server_version TEXT NOT NULL,
                server_version_num INTEGER NOT NULL,
                postmaster_start_ms INTEGER NOT NULL,
                collection_started_at_ms INTEGER NOT NULL,
                collection_completed_at_ms INTEGER NOT NULL,
                measured_at TEXT NOT NULL,
                setting_count INTEGER NOT NULL,
                non_default_count INTEGER NOT NULL,
                pending_restart_count INTEGER NOT NULL,
                changed_from_previous INTEGER NOT NULL,
                stored_at_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_pg_settings_snapshots_connection_time
                ON pg_settings_snapshots (
                    connection_id,
                    collection_started_at_ms DESC,
                    id DESC
                );

            CREATE INDEX IF NOT EXISTS idx_pg_settings_snapshots_target_time
                ON pg_settings_snapshots (
                    host,
                    port,
                    database_oid,
                    collection_started_at_ms DESC
                );
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить pg_settings history schema: {error}"))
}

impl DiagnosticStore {
    pub fn save_pg_settings_snapshot(
        &mut self,
        saved_connection: &SavedConnection,
        snapshot: &PgSettingsSnapshot,
        collection_started_at_ms: i64,
        collection_completed_at_ms: i64,
    ) -> Result<CollectedPgSettingsSnapshot, String> {
        if collection_completed_at_ms < collection_started_at_ms {
            return Err(crate::i18n::text(
                "Время завершения pg_settings snapshot раньше времени начала",
                "pg_settings snapshot finished before it started",
            )
            .into());
        }

        let previous = self.load_latest_pg_settings_snapshot(
            saved_connection.id,
            &saved_connection.host,
            i64::from(saved_connection.port),
            snapshot.database_oid,
            &snapshot.username,
        )?;
        let changes = previous
            .as_ref()
            .map(|stored| setting_changes(&stored.snapshot, snapshot));
        let changed_from_previous = changes
            .as_ref()
            .map(|items| items.len() as i64)
            .unwrap_or(0);

        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|error| format!("Не удалось сериализовать pg_settings snapshot: {error}"))?;
        let setting_count = snapshot.settings.len() as i64;
        let non_default_count = snapshot
            .settings
            .iter()
            .filter(|setting| !is_default_source(&setting.source))
            .count() as i64;
        let pending_restart_count = snapshot
            .settings
            .iter()
            .filter(|setting| setting.pending_restart)
            .count() as i64;
        let stored_at_ms = unix_time_millis()?;

        // Сохранение и формирование comparison должны быть атомарными:
        // вызывающий код не получит Err после необратимого INSERT.
        self.connection
            .execute_batch("SAVEPOINT pg_settings_snapshot_save")
            .map_err(|error| {
                format!("Не удалось начать сохранение pg_settings snapshot: {error}")
            })?;
        let result = (|| -> Result<CollectedPgSettingsSnapshot, String> {
            self.connection
                .execute(
                    r#"
                INSERT INTO pg_settings_snapshots (
                    schema_version,
                    app_version,
                    connection_id,
                    connection_name,
                    host,
                    port,
                    database_name,
                    database_oid,
                    username,
                    sslmode,
                    server_version,
                    server_version_num,
                    postmaster_start_ms,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                    measured_at,
                    setting_count,
                    non_default_count,
                    pending_restart_count,
                    changed_from_previous,
                    stored_at_ms,
                    snapshot_json
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22
                )
                "#,
                    params![
                        PG_SETTINGS_SNAPSHOT_SCHEMA_VERSION,
                        env!("CARGO_PKG_VERSION"),
                        saved_connection.id,
                        &saved_connection.name,
                        &saved_connection.host,
                        i64::from(saved_connection.port),
                        &snapshot.database,
                        snapshot.database_oid,
                        &snapshot.username,
                        &saved_connection.sslmode,
                        &snapshot.server_version,
                        snapshot.server_version_num,
                        snapshot.postmaster_start_ms,
                        collection_started_at_ms,
                        collection_completed_at_ms,
                        &snapshot.measured_at,
                        setting_count,
                        non_default_count,
                        pending_restart_count,
                        changed_from_previous,
                        stored_at_ms,
                        snapshot_json,
                    ],
                )
                .map_err(|error| format!("Не удалось сохранить pg_settings snapshot: {error}"))?;

            let id = self.connection.last_insert_rowid();
            let current = self.load_pg_settings_snapshot(id)?;

            let comparison = previous
                .map(|older| compare_stored_snapshots(older, current.clone()))
                .transpose()?;

            Ok(CollectedPgSettingsSnapshot {
                history: current.summary,
                snapshot: current.snapshot,
                comparison,
            })
        })();

        match result {
            Ok(collected) => {
                self.connection
                    .execute_batch("RELEASE SAVEPOINT pg_settings_snapshot_save")
                    .map_err(|error| {
                        format!("Не удалось завершить сохранение pg_settings snapshot: {error}")
                    })?;
                Ok(collected)
            }
            Err(error) => {
                self.connection
                    .execute_batch("ROLLBACK TO SAVEPOINT pg_settings_snapshot_save; RELEASE SAVEPOINT pg_settings_snapshot_save")
                    .map_err(|rollback| format!("{error}; откат pg_settings snapshot также завершился ошибкой: {rollback}"))?;
                Err(error)
            }
        }
    }

    pub fn list_pg_settings_snapshots(
        &mut self,
        connection_id: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<PgSettingsSnapshotSummary>, String> {
        let limit = limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);
        let mut result = Vec::new();

        if let Some(connection_id) = connection_id {
            let mut statement = self
                .connection
                .prepare(&format!(
                    "{} WHERE connection_id = ?1 \
                     ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?2",
                    summary_select_sql(),
                ))
                .map_err(|error| {
                    format!("Не удалось подготовить чтение pg_settings history: {error}")
                })?;
            let mut rows = statement
                .query(params![connection_id, i64::from(limit)])
                .map_err(|error| format!("Не удалось прочитать pg_settings history: {error}"))?;

            while let Some(row) = rows
                .next()
                .map_err(|error| format!("Не удалось прочитать pg_settings history row: {error}"))?
            {
                result.push(
                    summary_from_row(row).map_err(|error| {
                        format!("Повреждена строка pg_settings history: {error}")
                    })?,
                );
            }
        } else {
            let mut statement = self
                .connection
                .prepare(&format!(
                    "{} ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?1",
                    summary_select_sql(),
                ))
                .map_err(|error| {
                    format!("Не удалось подготовить чтение pg_settings history: {error}")
                })?;
            let mut rows = statement
                .query(params![i64::from(limit)])
                .map_err(|error| format!("Не удалось прочитать pg_settings history: {error}"))?;

            while let Some(row) = rows
                .next()
                .map_err(|error| format!("Не удалось прочитать pg_settings history row: {error}"))?
            {
                result.push(
                    summary_from_row(row).map_err(|error| {
                        format!("Повреждена строка pg_settings history: {error}")
                    })?,
                );
            }
        }

        Ok(result)
    }

    pub fn list_pg_settings_changes(
        &mut self,
        connection_id: i64,
        limit: Option<u32>,
    ) -> Result<Vec<PgSettingsChangeEvent>, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection id для pg_settings history",
                "Invalid connection ID for pg_settings history",
            )
            .into());
        }

        let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
        let scan_limit = MAX_HISTORY_LIMIT;
        let snapshots = self.list_pg_settings_snapshots(Some(connection_id), Some(scan_limit))?;
        let mut events = Vec::new();

        for pair in snapshots.windows(2) {
            let newer = &pair[0];
            let older = &pair[1];

            if newer.changed_from_previous == 0 {
                continue;
            }

            let comparison = self.compare_pg_settings_snapshots(older.id, newer.id)?;

            for change in comparison.changes {
                events.push(PgSettingsChangeEvent {
                    detected_at_ms: comparison.newer.collection_completed_at_ms,
                    changed_after_ms: comparison.older.collection_completed_at_ms,
                    changed_before_ms: comparison.newer.collection_completed_at_ms,
                    older_snapshot_id: comparison.older.id,
                    newer_snapshot_id: comparison.newer.id,
                    database: comparison.newer.database.clone(),
                    username: comparison.newer.username.clone(),
                    change,
                });

                if events.len() >= limit {
                    return Ok(events);
                }
            }
        }

        Ok(events)
    }

    /// A bounded, chronological audit of stored settings. Never infer changes
    /// between different targets. Loading errors affect only the relevant pair.
    pub fn pg_settings_history_analysis(
        &mut self,
        connection_id: i64,
        limit: Option<u32>,
    ) -> Result<PgSettingsHistoryAnalysis, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection id для pg_settings history",
                "Invalid connection ID for pg_settings history",
            )
            .into());
        }
        let summaries = self.list_pg_settings_snapshots(
            Some(connection_id),
            Some(limit.unwrap_or(80).clamp(2, 120)),
        )?;
        let mut intervals = Vec::new();
        for pair in summaries.windows(2) {
            let newer = &pair[0];
            let older = &pair[1];
            let mut row = PgSettingsHistoryInterval {
                from_snapshot_id: older.id,
                to_snapshot_id: newer.id,
                from_ms: older.collection_completed_at_ms,
                to_ms: newer.collection_completed_at_ms,
                state: "comparable".into(),
                reason: None,
                value_change_count: 0,
                source_change_count: 0,
                pending_restart_change_count: 0,
                changes: Vec::new(),
            };
            if let Err(reason) = validate_same_target(older, newer) {
                row.state = "target_changed".into();
                row.reason = Some(reason);
            } else if older.schema_version != PG_SETTINGS_SNAPSHOT_SCHEMA_VERSION
                || newer.schema_version != PG_SETTINGS_SNAPSHOT_SCHEMA_VERSION
            {
                row.state = "unsupported_schema".into();
                row.reason = Some(
                    crate::i18n::text(
                        "Версия хотя бы одного снимка не поддерживается",
                        "At least one snapshot has an unsupported version",
                    )
                    .into(),
                );
            } else {
                // A restart doesn't invalidate a settings comparison, but it
                // changes what we can infer about when and why values changed.
                match self.compare_pg_settings_snapshots(older.id, newer.id) {
                    Ok(comparison) => {
                        row.value_change_count = comparison.value_change_count;
                        row.source_change_count = comparison.source_change_count;
                        row.pending_restart_change_count = comparison.pending_restart_change_count;
                        row.changes = comparison.changes;
                        if older.postmaster_start_ms != newer.postmaster_start_ms {
                            row.state = "server_restarted".into();
                            row.reason = Some(crate::i18n::text("Между снимками изменился postmaster_start; точный момент изменения параметров неизвестен", "postmaster_start changed between snapshots; the exact time of the setting change is unknown").into());
                        }
                    }
                    Err(reason) => {
                        row.state = "comparison_unavailable".into();
                        row.reason = Some(reason);
                    }
                }
            }
            intervals.push(row);
        }
        Ok(PgSettingsHistoryAnalysis {
            snapshot_count: summaries.len(),
            intervals,
            limitations: vec![
                crate::i18n::text("Время изменения известно только между двумя снимками, а не до секунды", "The change is known only to have occurred between two snapshots, not at an exact second").into(),
                crate::i18n::text("Снимки отражают параметры диагностической сессии и могут содержать чувствительные значения и пути", "Snapshots reflect diagnostic-session settings and may contain sensitive values and paths").into(),
                crate::i18n::text("system_identifier не сохранён; совпадение логического target не доказывает идентичность физического кластера", "system_identifier was not saved; a matching logical target does not prove the same physical cluster").into(),
            ],
        })
    }

    pub fn pg_settings_snapshot(&mut self, id: i64) -> Result<PgSettingsSnapshot, String> {
        Ok(self.load_pg_settings_snapshot(id)?.snapshot)
    }

    pub(super) fn pg_settings_summary_by_id(
        &mut self,
        id: i64,
    ) -> Result<PgSettingsSnapshotSummary, String> {
        Ok(self.load_pg_settings_snapshot(id)?.summary)
    }

    pub fn compare_pg_settings_snapshots(
        &mut self,
        first_id: i64,
        second_id: i64,
    ) -> Result<PgSettingsComparison, String> {
        if first_id == second_id {
            return Err("Для сравнения выберите два разных pg_settings snapshot".into());
        }

        let first = self.load_pg_settings_snapshot(first_id)?;
        let second = self.load_pg_settings_snapshot(second_id)?;

        let (older, newer) =
            if snapshot_order_key(&first.summary) <= snapshot_order_key(&second.summary) {
                (first, second)
            } else {
                (second, first)
            };

        compare_stored_snapshots(older, newer)
    }

    fn load_latest_pg_settings_snapshot(
        &mut self,
        connection_id: i64,
        host: &str,
        port: i64,
        database_oid: i64,
        username: &str,
    ) -> Result<Option<StoredPgSettingsSnapshot>, String> {
        let id: Option<i64> = self
            .connection
            .query_row(
                r#"
                SELECT id
                FROM pg_settings_snapshots
                WHERE connection_id = ?1
                  AND host = ?2 AND port = ?3
                  AND database_oid = ?4 AND username = ?5
                ORDER BY collection_started_at_ms DESC, id DESC
                LIMIT 1
                "#,
                params![connection_id, host, port, database_oid, username],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Не удалось найти последний pg_settings snapshot: {error}"))?;

        id.map(|id| self.load_pg_settings_snapshot(id)).transpose()
    }

    fn load_pg_settings_snapshot(&mut self, id: i64) -> Result<StoredPgSettingsSnapshot, String> {
        let sql = format!("{} WHERE id = ?1", stored_select_sql());

        self.connection
            .query_row(&sql, params![id], |row| {
                let summary = summary_from_row(row)?;
                let snapshot_json: String = row.get(22)?;
                Ok((summary, snapshot_json))
            })
            .map_err(|error| format!("pg_settings snapshot #{id} не найден или повреждён: {error}"))
            .and_then(|(summary, snapshot_json)| {
                if summary.schema_version != PG_SETTINGS_SNAPSHOT_SCHEMA_VERSION {
                    return Err(format!(
                        "pg_settings snapshot #{} имеет schema_version={}, поддерживается {}",
                        summary.id, summary.schema_version, PG_SETTINGS_SNAPSHOT_SCHEMA_VERSION,
                    ));
                }

                let snapshot = serde_json::from_str::<PgSettingsSnapshot>(&snapshot_json).map_err(
                    |error| {
                        format!(
                            "Не удалось разобрать сохранённый pg_settings snapshot #{}: {error}",
                            summary.id,
                        )
                    },
                )?;

                Ok(StoredPgSettingsSnapshot { summary, snapshot })
            })
    }
}

fn summary_select_sql() -> &'static str {
    r#"
    SELECT
        id,
        schema_version,
        app_version,
        connection_id,
        connection_name,
        host,
        port,
        database_name,
        database_oid,
        username,
        sslmode,
        server_version,
        server_version_num,
        postmaster_start_ms,
        collection_started_at_ms,
        collection_completed_at_ms,
        measured_at,
        setting_count,
        non_default_count,
        pending_restart_count,
        changed_from_previous,
        stored_at_ms
    FROM pg_settings_snapshots
    "#
}

fn stored_select_sql() -> &'static str {
    r#"
    SELECT
        id,
        schema_version,
        app_version,
        connection_id,
        connection_name,
        host,
        port,
        database_name,
        database_oid,
        username,
        sslmode,
        server_version,
        server_version_num,
        postmaster_start_ms,
        collection_started_at_ms,
        collection_completed_at_ms,
        measured_at,
        setting_count,
        non_default_count,
        pending_restart_count,
        changed_from_previous,
        stored_at_ms,
        snapshot_json
    FROM pg_settings_snapshots
    "#
}

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<PgSettingsSnapshotSummary> {
    Ok(PgSettingsSnapshotSummary {
        id: row.get(0)?,
        schema_version: row.get(1)?,
        app_version: row.get(2)?,
        connection_id: row.get(3)?,
        connection_name: row.get(4)?,
        host: row.get(5)?,
        port: row.get(6)?,
        database: row.get(7)?,
        database_oid: row.get(8)?,
        username: row.get(9)?,
        sslmode: row.get(10)?,
        server_version: row.get(11)?,
        server_version_num: row.get(12)?,
        postmaster_start_ms: row.get(13)?,
        collection_started_at_ms: row.get(14)?,
        collection_completed_at_ms: row.get(15)?,
        measured_at: row.get(16)?,
        setting_count: row.get(17)?,
        non_default_count: row.get(18)?,
        pending_restart_count: row.get(19)?,
        changed_from_previous: row.get(20)?,
        stored_at_ms: row.get(21)?,
    })
}

fn compare_stored_snapshots(
    older: StoredPgSettingsSnapshot,
    newer: StoredPgSettingsSnapshot,
) -> Result<PgSettingsComparison, String> {
    validate_same_target(&older.summary, &newer.summary)?;

    let elapsed_ms = newer
        .summary
        .collection_started_at_ms
        .checked_sub(older.summary.collection_started_at_ms)
        .ok_or("Не удалось вычислить интервал pg_settings snapshots")?;
    let changes = setting_changes(&older.snapshot, &newer.snapshot);

    Ok(PgSettingsComparison {
        value_change_count: changes.iter().filter(|change| change.value_changed).count() as i64,
        source_change_count: changes
            .iter()
            .filter(|change| change.source_changed)
            .count() as i64,
        pending_restart_change_count: changes
            .iter()
            .filter(|change| change.pending_restart_changed)
            .count() as i64,
        added_count: changes
            .iter()
            .filter(|change| change.state == "added")
            .count() as i64,
        removed_count: changes
            .iter()
            .filter(|change| change.state == "removed")
            .count() as i64,
        older: older.summary,
        newer: newer.summary,
        elapsed_seconds: elapsed_ms as f64 / 1000.0,
        changes,
    })
}

fn setting_changes(older: &PgSettingsSnapshot, newer: &PgSettingsSnapshot) -> Vec<PgSettingChange> {
    let older_by_name = older
        .settings
        .iter()
        .map(|setting| (setting.name.as_str(), setting))
        .collect::<HashMap<_, _>>();
    let newer_by_name = newer
        .settings
        .iter()
        .map(|setting| (setting.name.as_str(), setting))
        .collect::<HashMap<_, _>>();
    let names = older_by_name
        .keys()
        .chain(newer_by_name.keys())
        .copied()
        .collect::<BTreeSet<_>>();

    names
        .into_iter()
        .filter_map(|name| {
            setting_change(
                name,
                older_by_name.get(name).copied(),
                newer_by_name.get(name).copied(),
            )
        })
        .collect()
}

fn setting_change(
    name: &str,
    older: Option<&PgSetting>,
    newer: Option<&PgSetting>,
) -> Option<PgSettingChange> {
    let value_changed = match (older, newer) {
        (Some(older), Some(newer)) => older.setting != newer.setting || older.unit != newer.unit,
        _ => true,
    };
    let reset_value_changed = match (older, newer) {
        (Some(older), Some(newer)) => older.reset_val != newer.reset_val,
        _ => true,
    };
    let source_changed = match (older, newer) {
        (Some(older), Some(newer)) => {
            older.source != newer.source
                || older.sourcefile != newer.sourcefile
                || older.sourceline != newer.sourceline
        }
        _ => true,
    };
    let pending_restart_changed = match (older, newer) {
        (Some(older), Some(newer)) => older.pending_restart != newer.pending_restart,
        _ => true,
    };

    if older.is_some()
        && newer.is_some()
        && !value_changed
        && !reset_value_changed
        && !source_changed
        && !pending_restart_changed
    {
        return None;
    }

    let state = match (older, newer) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        (Some(_), Some(_)) => "changed",
        (None, None) => return None,
    };
    let reference = newer.or(older)?;

    Some(PgSettingChange {
        name: name.to_owned(),
        category: reference.category.clone(),
        state: state.to_owned(),
        older_value: older.map(|setting| setting.setting.clone()),
        newer_value: newer.map(|setting| setting.setting.clone()),
        older_unit: older.and_then(|setting| setting.unit.clone()),
        newer_unit: newer.and_then(|setting| setting.unit.clone()),
        older_reset_value: older.and_then(|setting| setting.reset_val.clone()),
        newer_reset_value: newer.and_then(|setting| setting.reset_val.clone()),
        older_source: older.map(|setting| setting.source.clone()),
        newer_source: newer.map(|setting| setting.source.clone()),
        older_sourcefile: older.and_then(|setting| setting.sourcefile.clone()),
        newer_sourcefile: newer.and_then(|setting| setting.sourcefile.clone()),
        older_sourceline: older.and_then(|setting| setting.sourceline),
        newer_sourceline: newer.and_then(|setting| setting.sourceline),
        older_pending_restart: older.map(|setting| setting.pending_restart),
        newer_pending_restart: newer.map(|setting| setting.pending_restart),
        value_changed,
        reset_value_changed,
        source_changed,
        pending_restart_changed,
    })
}

fn validate_same_target(
    older: &PgSettingsSnapshotSummary,
    newer: &PgSettingsSnapshotSummary,
) -> Result<(), String> {
    if older.connection_id != newer.connection_id
        || older.host != newer.host
        || older.port != newer.port
        || older.database_oid != newer.database_oid
        || older.username != newer.username
    {
        return Err(format!(
            "Нельзя сравнивать pg_settings snapshots разных targets: #{} = {}:{}/oid {}/user {}, #{} = {}:{}/oid {}/user {}",
            older.id,
            older.host,
            older.port,
            older.database_oid,
            older.username,
            newer.id,
            newer.host,
            newer.port,
            newer.database_oid,
            newer.username,
        ));
    }

    Ok(())
}

fn is_default_source(source: &str) -> bool {
    matches!(source, "default" | "dynamic default")
}

fn snapshot_order_key(summary: &PgSettingsSnapshotSummary) -> (i64, i64) {
    (summary.collection_started_at_ms, summary.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(name: &str, value: &str, source: &str) -> PgSetting {
        PgSetting {
            name: name.to_owned(),
            setting: value.to_owned(),
            unit: Some("MB".to_owned()),
            category: "Resource Usage / Memory".to_owned(),
            vartype: "integer".to_owned(),
            context: "user".to_owned(),
            source: source.to_owned(),
            sourcefile: Some("/etc/postgresql/postgresql.conf".to_owned()),
            sourceline: Some(10),
            pending_restart: false,
            boot_val: Some("4".to_owned()),
            reset_val: Some(value.to_owned()),
            min_val: Some("1".to_owned()),
            max_val: Some("1024".to_owned()),
            enumvals: None,
        }
    }

    fn snapshot(settings: Vec<PgSetting>) -> PgSettingsSnapshot {
        PgSettingsSnapshot {
            measured_at: "2026-09-15 12:00:00+00".to_owned(),
            database: "app".to_owned(),
            database_oid: 16_384,
            username: "pgxray".to_owned(),
            server_version: "17.6".to_owned(),
            server_version_num: 170_006,
            postmaster_start_ms: 1_700_000_000_000,
            settings,
        }
    }

    fn connection(host: &str) -> SavedConnection {
        SavedConnection {
            id: 1,
            name: "test".into(),
            host: host.into(),
            port: 5432,
            database: "app".into(),
            username: "pgxray".into(),
            sslmode: "prefer".into(),
            has_password: false,
            project_id: None,
            shortcut_slot: None,
            last_probe: None,
        }
    }

    fn memory_store() -> DiagnosticStore {
        let connection = Connection::open_in_memory().unwrap();
        init_schema(&connection).unwrap();
        DiagnosticStore { connection }
    }

    #[test]
    fn saves_comparison_against_latest_matching_target() {
        let mut store = memory_store();
        let before = snapshot(vec![setting("work_mem", "4", "default")]);
        let other = snapshot(vec![setting("work_mem", "16", "database")]);
        let after = snapshot(vec![setting("work_mem", "8", "database")]);

        store
            .save_pg_settings_snapshot(&connection("host-a"), &before, 1000, 1100)
            .unwrap();
        store
            .save_pg_settings_snapshot(&connection("host-b"), &other, 2000, 2100)
            .unwrap();
        let collected = store
            .save_pg_settings_snapshot(&connection("host-a"), &after, 3000, 3100)
            .unwrap();
        let comparison = collected
            .comparison
            .expect("matching host-a snapshot exists");
        assert_eq!(comparison.changes.len(), 1);
        assert_eq!(comparison.changes[0].older_value.as_deref(), Some("4"));
        assert_eq!(comparison.changes[0].newer_value.as_deref(), Some("8"));
    }

    #[test]
    fn failed_comparison_rolls_back_new_snapshot() {
        let mut store = memory_store();
        let before = snapshot(vec![setting("work_mem", "4", "default")]);
        let after = snapshot(vec![setting("work_mem", "8", "database")]);

        store
            .save_pg_settings_snapshot(&connection("host-a"), &before, 3000, 3100)
            .unwrap();

        // Повреждаем JSON только у новых INSERT.
        // Первая сохранённая запись остаётся корректной.
        store
            .connection
            .execute_batch(
                r#"
            CREATE TEMP TRIGGER corrupt_new_settings_json
            AFTER INSERT ON pg_settings_snapshots
            BEGIN
                UPDATE pg_settings_snapshots
                   SET snapshot_json = '{invalid_json'
                 WHERE id = NEW.id;
            END;
            "#,
            )
            .unwrap();

        // INSERT выполняется, но последующая загрузка нового снимка
        // должна завершиться ошибкой десериализации.
        let result = store.save_pg_settings_snapshot(&connection("host-a"), &after, 4000, 4100);

        assert!(result.is_err());

        // Вставленная запись должна быть отменена через SAVEPOINT.
        let count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM pg_settings_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn history_analysis_tracks_changes_and_restart_without_claiming_continuity() {
        let mut store = memory_store();
        let first = snapshot(vec![setting("work_mem", "4", "default")]);
        let mut second = snapshot(vec![setting("work_mem", "8", "database")]);
        second.settings[0].pending_restart = true;
        let mut third = snapshot(vec![setting("work_mem", "16", "database")]);
        third.postmaster_start_ms += 10_000;
        store
            .save_pg_settings_snapshot(&connection("host-a"), &first, 1000, 1100)
            .unwrap();
        store
            .save_pg_settings_snapshot(&connection("host-a"), &second, 2000, 2100)
            .unwrap();
        store
            .save_pg_settings_snapshot(&connection("host-a"), &third, 3000, 3100)
            .unwrap();
        let history = store.pg_settings_history_analysis(1, Some(80)).unwrap();
        assert_eq!(history.snapshot_count, 3);
        assert_eq!(history.intervals.len(), 2);
        assert_eq!(history.intervals[0].state, "server_restarted");
        assert_eq!(history.intervals[0].changes.len(), 1);
        assert_eq!(
            history.intervals[0].changes[0].older_value.as_deref(),
            Some("8")
        );
        assert_eq!(history.intervals[1].state, "comparable");
        assert_eq!(history.intervals[1].pending_restart_change_count, 1);
    }

    #[test]
    fn history_analysis_does_not_diff_different_targets() {
        let mut store = memory_store();
        let first = snapshot(vec![setting("work_mem", "4", "default")]);
        let second = snapshot(vec![setting("work_mem", "8", "database")]);
        store
            .save_pg_settings_snapshot(&connection("host-a"), &first, 1000, 1100)
            .unwrap();
        store
            .save_pg_settings_snapshot(&connection("host-b"), &second, 2000, 2100)
            .unwrap();
        let history = store.pg_settings_history_analysis(1, None).unwrap();
        assert_eq!(history.intervals.len(), 1);
        assert_eq!(history.intervals[0].state, "target_changed");
        assert!(history.intervals[0].changes.is_empty());
    }

    #[test]
    fn detects_value_and_source_changes() {
        let older = snapshot(vec![setting("work_mem", "64", "configuration file")]);
        let mut newer_setting = setting("work_mem", "128", "database");
        newer_setting.sourcefile = None;
        newer_setting.sourceline = None;
        let newer = snapshot(vec![newer_setting]);

        let changes = setting_changes(&older, &newer);

        assert_eq!(changes.len(), 1);
        assert!(changes[0].value_changed);
        assert!(changes[0].source_changed);
        assert_eq!(changes[0].older_value.as_deref(), Some("64"));
        assert_eq!(changes[0].newer_value.as_deref(), Some("128"));
    }

    #[test]
    fn distinguishes_metadata_only_change_from_value_change() {
        let older = snapshot(vec![setting(
            "application_name",
            "pgxray-collector",
            "client",
        )]);
        let mut newer_setting = setting("application_name", "pgxray-collector", "client");
        newer_setting.sourcefile = Some("/another/path.conf".into());
        let newer = snapshot(vec![newer_setting]);

        let changes = setting_changes(&older, &newer);
        assert_eq!(changes.len(), 1);
        assert!(!changes[0].value_changed);
        assert!(!changes[0].reset_value_changed);
        assert!(changes[0].source_changed);
        assert!(!changes[0].pending_restart_changed);
        assert_eq!(changes[0].older_value, changes[0].newer_value);
    }

    #[test]
    fn ignores_unchanged_settings() {
        let value = setting("work_mem", "64", "configuration file");
        let older = snapshot(vec![value.clone()]);
        let newer = snapshot(vec![value]);

        assert!(setting_changes(&older, &newer).is_empty());
    }

    #[test]
    fn detects_added_and_removed_settings() {
        let older = snapshot(vec![setting("old_setting", "1", "default")]);
        let newer = snapshot(vec![setting("new_setting", "1", "default")]);

        let changes = setting_changes(&older, &newer);

        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|change| change.state == "added"));
        assert!(changes.iter().any(|change| change.state == "removed"));
    }
}
