use super::{unix_time_millis, DiagnosticStore, DEFAULT_HISTORY_LIMIT, MAX_HISTORY_LIMIT};
use crate::{
    autovacuum::{
        self, AutovacuumAnalyzeProgress, AutovacuumInterpretation, AutovacuumInterpretationItem,
        AutovacuumInterpretationSeverity, AutovacuumMaintenanceAdvice,
        AutovacuumMaintenanceHistorySample, AutovacuumPolicyCandidate, AutovacuumPolicyCandidates,
        AutovacuumSection, AutovacuumSnapshot, AutovacuumTable, AutovacuumVacuumProgress,
    },
    connections::SavedConnection,
};
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const AUTOVACUUM_SNAPSHOT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
pub struct CollectedAutovacuumSnapshot {
    pub history: AutovacuumSnapshotSummary,
    pub snapshot: AutovacuumSnapshot,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumSnapshotSummary {
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
    pub collection_started_at_ms: i64,
    pub collection_completed_at_ms: i64,
    pub measured_at: String,
    pub stats_reset: Option<String>,
    pub table_count: i64,
    pub total_size_bytes: i64,
    pub total_dead_tuples: i64,
    pub dead_trigger_reached_count: i64,
    pub insert_trigger_reached_count: i64,
    pub analyze_trigger_reached_count: i64,
    pub autovacuum_disabled_count: i64,
    pub active_vacuum_count: i64,
    pub active_analyze_count: i64,
    pub unavailable_sections: i64,
    pub stored_at_ms: i64,
}

#[derive(Debug, Default, Serialize)]
pub struct AutovacuumRelationDelta {
    pub relation_oid: u32,
    pub older_name: Option<String>,
    pub newer_name: Option<String>,
    pub sample_state: String,
    pub older_live_tuples: Option<i64>,
    pub newer_live_tuples: Option<i64>,
    pub older_dead_tuples: Option<i64>,
    pub newer_dead_tuples: Option<i64>,
    pub dead_tuples_delta: Option<i64>,
    pub older_toast_dead_tuples: Option<i64>,
    pub newer_toast_dead_tuples: Option<i64>,
    pub toast_dead_tuples_delta: Option<i64>,
    pub inserts_delta: Option<i64>,
    pub updates_delta: Option<i64>,
    pub deletes_delta: Option<i64>,
    pub hot_updates_delta: Option<i64>,
    pub vacuum_count_delta: Option<i64>,
    pub autovacuum_count_delta: Option<i64>,
    pub analyze_count_delta: Option<i64>,
    pub autoanalyze_count_delta: Option<i64>,
    pub toast_autovacuum_count_delta: Option<i64>,
    pub vacuum_cycle_observed: bool,
    pub toast_vacuum_cycle_observed: bool,
    pub analyze_cycle_observed: bool,
    pub dead_growth_per_day: Option<f64>,
    pub estimated_days_to_dead_trigger: Option<f64>,
    pub older_dead_trigger: Option<i64>,
    pub newer_dead_trigger: Option<i64>,
    pub older_dead_trigger_used_pct: Option<f64>,
    pub newer_dead_trigger_used_pct: Option<f64>,
    pub older_total_size_bytes: Option<i64>,
    pub newer_total_size_bytes: Option<i64>,
    pub total_size_delta_bytes: Option<i64>,
    pub older_workload_profile: Option<String>,
    pub newer_workload_profile: Option<String>,
    pub interval_workload_profile: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumRelationHistorySummary {
    pub history_points: usize,
    pub history_span_seconds: Option<f64>,
    pub stats_period_continuous: bool,
    pub inserts_per_hour: Option<f64>,
    pub updates_per_hour: Option<f64>,
    pub deletes_per_hour: Option<f64>,
    pub hot_update_pct: Option<f64>,
    pub vacuum_cycles: Option<i64>,
    pub analyze_cycles: Option<i64>,
    pub toast_vacuum_cycles: Option<i64>,
    pub dead_growth_per_hour: Option<f64>,
    pub dead_trigger_reached_points: usize,
    pub analyze_trigger_reached_points: usize,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumRelationHistoryPoint {
    pub snapshot_id: i64,
    pub schema_version: i64,
    pub collection_started_at_ms: i64,
    pub measured_at: String,
    pub stats_reset: Option<String>,
    pub table: AutovacuumTable,
    pub active_vacuum: Option<AutovacuumVacuumProgress>,
    pub active_analyze: Option<AutovacuumAnalyzeProgress>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumRelationHistory {
    pub anchor_snapshot_id: i64,
    pub relation_oid: u32,
    pub relation_name: String,
    pub summary: AutovacuumRelationHistorySummary,
    pub points: Vec<AutovacuumRelationHistoryPoint>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumComparison {
    pub older: AutovacuumSnapshotSummary,
    pub newer: AutovacuumSnapshotSummary,
    pub elapsed_seconds: f64,
    pub stats_period_continuous: bool,
    pub table_changes: Vec<AutovacuumRelationDelta>,
    pub interpretation: AutovacuumInterpretation,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumWorkloadInterval {
    pub from_snapshot_id: i64,
    pub to_snapshot_id: i64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub elapsed_seconds: Option<f64>,
    pub state: String,
    pub comparable_relations: usize,
    pub sampled_relations: usize,
    pub inserts_per_hour: Option<f64>,
    pub updates_per_hour: Option<f64>,
    pub deletes_per_hour: Option<f64>,
    pub vacuum_cycles: Option<i64>,
    pub analyze_cycles: Option<i64>,
    pub vacuum_cycles_per_hour: Option<f64>,
    pub analyze_cycles_per_hour: Option<f64>,
    pub dead_tuples_estimate: Option<i64>,
    pub dead_trigger_reached_count: i64,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumWorkloadRelation {
    pub relation_oid: u32,
    pub relation_name: String,
    pub total_size_bytes: i64,
    pub dead_tuples: i64,
    pub dead_tuple_trigger: i64,
    pub dead_trigger_used_pct: Option<f64>,
    pub insert_trigger_used_pct: Option<f64>,
    pub analyze_trigger_used_pct: Option<f64>,
    pub dead_trigger_reached: bool,
    pub insert_trigger_reached: bool,
    pub analyze_trigger_reached: bool,
    pub autovacuum_enabled: bool,
    pub autovacuum_applicable: bool,
    pub last_autovacuum: Option<String>,
    pub last_autoanalyze: Option<String>,
    // Observations and delta apply to the selected consecutive pair only.
    pub delta: Option<AutovacuumRelationDelta>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumWorkloadHistory {
    pub connection_id: i64,
    pub snapshot_count: usize,
    pub latest_dead_tuples_estimate: Option<i64>,
    pub selected_to_snapshot_id: Option<i64>,
    pub intervals: Vec<AutovacuumWorkloadInterval>,
    pub relations: Vec<AutovacuumWorkloadRelation>,
    pub relation_count: usize,
    pub limitations: Vec<String>,
}

struct StoredAutovacuumSnapshot {
    summary: AutovacuumSnapshotSummary,
    snapshot: AutovacuumSnapshot,
}

pub(super) fn init_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS autovacuum_snapshots (
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
                collection_started_at_ms INTEGER NOT NULL,
                collection_completed_at_ms INTEGER NOT NULL,
                measured_at TEXT NOT NULL,
                stats_reset TEXT,
                table_count INTEGER NOT NULL,
                total_size_bytes INTEGER NOT NULL,
                total_dead_tuples INTEGER NOT NULL,
                dead_trigger_reached_count INTEGER NOT NULL,
                insert_trigger_reached_count INTEGER NOT NULL,
                analyze_trigger_reached_count INTEGER NOT NULL,
                autovacuum_disabled_count INTEGER NOT NULL,
                active_vacuum_count INTEGER NOT NULL,
                active_analyze_count INTEGER NOT NULL,
                unavailable_sections INTEGER NOT NULL,
                stored_at_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_autovacuum_snapshots_connection_time
                ON autovacuum_snapshots (
                    connection_id,
                    collection_started_at_ms DESC,
                    id DESC
                );

            CREATE INDEX IF NOT EXISTS idx_autovacuum_snapshots_target_time
                ON autovacuum_snapshots (
                    host,
                    port,
                    database_oid,
                    collection_started_at_ms DESC
                );
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить Autovacuum history schema: {error}"))
}

impl DiagnosticStore {
    pub fn save_autovacuum_snapshot(
        &mut self,
        saved_connection: &SavedConnection,
        snapshot: &AutovacuumSnapshot,
        collection_started_at_ms: i64,
        collection_completed_at_ms: i64,
    ) -> Result<AutovacuumSnapshotSummary, String> {
        if collection_completed_at_ms < collection_started_at_ms {
            return Err(crate::i18n::text(
                "Время завершения Autovacuum snapshot раньше времени начала",
                "Autovacuum snapshot end time is earlier than its start time",
            )
            .into());
        }

        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|error| format!("Не удалось сериализовать Autovacuum snapshot: {error}"))?;

        let tables = snapshot.tables.data.as_deref().unwrap_or(&[]);

        let table_count = i64::try_from(tables.len()).unwrap_or(i64::MAX);

        let total_size_bytes = tables.iter().try_fold(0_i64, |total, row| {
            total.checked_add(row.total_size_bytes).ok_or_else(|| {
                crate::i18n::text(
                    "Переполнение total_size_bytes при сохранении Autovacuum snapshot",
                    "total_size_bytes overflow while saving an Autovacuum snapshot",
                )
                .to_owned()
            })
        })?;

        let total_dead_tuples = tables.iter().try_fold(0_i64, |total, row| {
            total.checked_add(row.dead_tuples).ok_or_else(|| {
                crate::i18n::text(
                    "Переполнение total_dead_tuples при сохранении Autovacuum snapshot",
                    "total_dead_tuples overflow while saving an Autovacuum snapshot",
                )
                .to_owned()
            })
        })?;

        let dead_trigger_reached_count = count_tables(tables, |row| row.dead_trigger_reached);

        let insert_trigger_reached_count = count_tables(tables, |row| row.insert_trigger_reached);

        let analyze_trigger_reached_count = count_tables(tables, |row| row.analyze_trigger_reached);

        let autovacuum_disabled_count = count_tables(tables, |row| {
            row.autovacuum_applicable && !row.autovacuum_enabled
        });

        let active_vacuum_count = section_len(&snapshot.vacuums);

        let active_analyze_count = section_len(&snapshot.analyzes);

        let unavailable_sections = unavailable_section_count(snapshot);

        let stored_at_ms = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT INTO autovacuum_snapshots (
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
                    collection_started_at_ms,
                    collection_completed_at_ms,
                    measured_at,
                    stats_reset,
                    table_count,
                    total_size_bytes,
                    total_dead_tuples,
                    dead_trigger_reached_count,
                    insert_trigger_reached_count,
                    analyze_trigger_reached_count,
                    autovacuum_disabled_count,
                    active_vacuum_count,
                    active_analyze_count,
                    unavailable_sections,
                    stored_at_ms,
                    snapshot_json
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                    ?25, ?26, ?27, ?28
                )
                "#,
                params![
                    AUTOVACUUM_SNAPSHOT_SCHEMA_VERSION,
                    env!("CARGO_PKG_VERSION"),
                    saved_connection.id,
                    &saved_connection.name,
                    &saved_connection.host,
                    i64::from(saved_connection.port),
                    &snapshot.overview.database,
                    i64::from(snapshot.overview.database_oid),
                    &snapshot.overview.username,
                    &saved_connection.sslmode,
                    &snapshot.overview.server_version,
                    snapshot.overview.server_version_num,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                    &snapshot.overview.measured_at,
                    &snapshot.overview.stats_reset,
                    table_count,
                    total_size_bytes,
                    total_dead_tuples,
                    dead_trigger_reached_count,
                    insert_trigger_reached_count,
                    analyze_trigger_reached_count,
                    autovacuum_disabled_count,
                    active_vacuum_count,
                    active_analyze_count,
                    unavailable_sections,
                    stored_at_ms,
                    snapshot_json,
                ],
            )
            .map_err(|error| format!("Не удалось сохранить Autovacuum snapshot: {error}"))?;

        let id = self.connection.last_insert_rowid();

        Ok(AutovacuumSnapshotSummary {
            id,
            schema_version: AUTOVACUUM_SNAPSHOT_SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            connection_id: saved_connection.id,
            connection_name: saved_connection.name.clone(),
            host: saved_connection.host.clone(),
            port: i64::from(saved_connection.port),
            database: snapshot.overview.database.clone(),
            database_oid: i64::from(snapshot.overview.database_oid),
            username: snapshot.overview.username.clone(),
            sslmode: saved_connection.sslmode.clone(),
            server_version: snapshot.overview.server_version.clone(),
            server_version_num: snapshot.overview.server_version_num,
            collection_started_at_ms,
            collection_completed_at_ms,
            measured_at: snapshot.overview.measured_at.clone(),
            stats_reset: snapshot.overview.stats_reset.clone(),
            table_count,
            total_size_bytes,
            total_dead_tuples,
            dead_trigger_reached_count,
            insert_trigger_reached_count,
            analyze_trigger_reached_count,
            autovacuum_disabled_count,
            active_vacuum_count,
            active_analyze_count,
            unavailable_sections,
            stored_at_ms,
        })
    }

    pub fn list_autovacuum_snapshots(
        &mut self,
        connection_id: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<AutovacuumSnapshotSummary>, String> {
        let limit = limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);

        let mut result = Vec::new();

        if let Some(connection_id) = connection_id {
            let sql =
                format!(
                    "{} WHERE connection_id = ?1 ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?2",
                    summary_select_sql(),
                );

            let mut statement = self.connection.prepare(&sql).map_err(|error| {
                format!("Не удалось подготовить чтение Autovacuum history: {error}")
            })?;

            let mut rows = statement
                .query(params![connection_id, i64::from(limit),])
                .map_err(|error| format!("Не удалось прочитать Autovacuum history: {error}"))?;

            while let Some(row) = rows.next().map_err(|error| {
                format!("Не удалось прочитать строку Autovacuum history: {error}")
            })? {
                result.push(
                    summary_from_row(row).map_err(|error| {
                        format!("Повреждена строка Autovacuum history: {error}")
                    })?,
                );
            }
        } else {
            let sql = format!(
                "{} ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?1",
                summary_select_sql(),
            );

            let mut statement = self.connection.prepare(&sql).map_err(|error| {
                format!("Не удалось подготовить чтение Autovacuum history: {error}")
            })?;

            let mut rows = statement
                .query(params![i64::from(limit),])
                .map_err(|error| format!("Не удалось прочитать Autovacuum history: {error}"))?;

            while let Some(row) = rows.next().map_err(|error| {
                format!("Не удалось прочитать строку Autovacuum history: {error}")
            })? {
                result.push(
                    summary_from_row(row).map_err(|error| {
                        format!("Повреждена строка Autovacuum history: {error}")
                    })?,
                );
            }
        }

        Ok(result)
    }

    /// Build a compact History view from saved observations, never querying PostgreSQL.
    /// Rates describe only the common, comparable sampled relations of a pair.
    pub fn autovacuum_workload_history(
        &mut self,
        connection_id: i64,
        limit: Option<u32>,
        anchor_snapshot_id: Option<i64>,
    ) -> Result<AutovacuumWorkloadHistory, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection_id для Autovacuum History",
                "Invalid connection_id for Autovacuum History",
            )
            .into());
        }
        let limit = limit.unwrap_or(40).clamp(1, 60);
        let summaries = self.list_autovacuum_snapshots(Some(connection_id), Some(limit))?;
        let selected_id = match anchor_snapshot_id {
            Some(id) if summaries.iter().any(|summary| summary.id == id) => Some(id),
            Some(_) => {
                return Err(crate::i18n::text(
                    "Выбранный Autovacuum snapshot не найден среди загруженных измерений",
                    "Selected Autovacuum snapshot was not found among loaded measurements",
                )
                .into())
            }
            None => summaries.first().map(|summary| summary.id),
        };
        let mut result = AutovacuumWorkloadHistory {
            connection_id,
            snapshot_count: summaries.len(),
            latest_dead_tuples_estimate: None,
            selected_to_snapshot_id: selected_id,
            intervals: Vec::new(),
            relations: Vec::new(),
            relation_count: 0,
            limitations: Vec::new(),
        };
        let Some(selected_id) = selected_id else {
            return Ok(result);
        };
        let selected = self.load_autovacuum_snapshot(selected_id)?;
        let selected_tables = selected.snapshot.tables.data.as_ref();
        if selected_tables.is_none() {
            result.limitations.push(if crate::i18n::is_ru() { format!("Таблицы недоступны в выбранном снимке #{selected_id}; рейтинг объектов не построен.") } else { format!("Tables are unavailable in the selected snapshot #{selected_id}; the object ranking was not built.") });
        } else {
            result.latest_dead_tuples_estimate = Some(selected.summary.total_dead_tuples);
        }
        let mut selected_deltas = HashMap::<u32, AutovacuumRelationDelta>::new();

        for pair in summaries.windows(2) {
            let (newer, older) = (&pair[0], &pair[1]);
            let duration =
                (newer.collection_started_at_ms - older.collection_started_at_ms) as f64 / 1000.0;
            let mut interval = AutovacuumWorkloadInterval {
                from_snapshot_id: older.id,
                to_snapshot_id: newer.id,
                from_ms: older.collection_started_at_ms,
                to_ms: newer.collection_started_at_ms,
                elapsed_seconds: (duration > 0.0).then_some(duration),
                state: "unavailable".into(),
                comparable_relations: 0,
                sampled_relations: 0,
                inserts_per_hour: None,
                updates_per_hour: None,
                deletes_per_hour: None,
                vacuum_cycles: None,
                analyze_cycles: None,
                vacuum_cycles_per_hour: None,
                analyze_cycles_per_hour: None,
                dead_tuples_estimate: Some(newer.total_dead_tuples),
                dead_trigger_reached_count: newer.dead_trigger_reached_count,
                limitations: Vec::new(),
            };
            if validate_same_target(older, newer).is_err() {
                interval.state = "target_changed".into();
            } else if duration <= 0.0 {
                interval.state = "invalid_interval".into();
            } else if duration > 86_400.0 {
                interval.state = "observation_gap".into();
            } else if stats_period_limitation(
                older.stats_reset.as_deref(),
                newer.stats_reset.as_deref(),
            )
            .is_some()
            {
                interval.state = "stats_reset_changed".into();
            } else {
                let before = self.load_autovacuum_snapshot(older.id)?;
                let after = self.load_autovacuum_snapshot(newer.id)?;
                let comparison = compare_snapshots(before, after)?;
                interval.sampled_relations = comparison.table_changes.len();
                interval.comparable_relations = comparison
                    .table_changes
                    .iter()
                    .filter(|row| {
                        row.sample_state == "present_in_both"
                            && row.inserts_delta.is_some()
                            && row.updates_delta.is_some()
                            && row.deletes_delta.is_some()
                    })
                    .count();
                interval.limitations = comparison.limitations;
                if comparison.table_changes.is_empty() {
                    interval.state = "no_table_sample".into();
                } else if interval.comparable_relations == 0 {
                    interval.state = "no_comparable_relations".into();
                } else {
                    interval.state = if interval.comparable_relations == interval.sampled_relations
                        && !interval
                            .limitations
                            .iter()
                            .any(|line| line.contains("сменил идентичность"))
                    {
                        "comparable".into()
                    } else {
                        "partial_sample".into()
                    };
                    let complete = comparison
                        .table_changes
                        .iter()
                        .filter(|row| row.sample_state == "present_in_both")
                        .all(|row| {
                            row.inserts_delta.is_some()
                                && row.updates_delta.is_some()
                                && row.deletes_delta.is_some()
                        });
                    if complete {
                        interval.inserts_per_hour =
                            counter_rate_from_rows(&comparison.table_changes, duration, |row| {
                                row.inserts_delta
                            });
                        interval.updates_per_hour =
                            counter_rate_from_rows(&comparison.table_changes, duration, |row| {
                                row.updates_delta
                            });
                        interval.deletes_per_hour =
                            counter_rate_from_rows(&comparison.table_changes, duration, |row| {
                                row.deletes_delta
                            });
                    }
                    interval.vacuum_cycles =
                        sum_comparable_counters(&comparison.table_changes, |row| {
                            row.vacuum_count_delta?
                                .checked_add(row.autovacuum_count_delta?)
                        });
                    interval.analyze_cycles =
                        sum_comparable_counters(&comparison.table_changes, |row| {
                            row.analyze_count_delta?
                                .checked_add(row.autoanalyze_count_delta?)
                        });
                    interval.vacuum_cycles_per_hour = interval
                        .vacuum_cycles
                        .map(|cycles| cycles as f64 * 3600.0 / duration);
                    interval.analyze_cycles_per_hour = interval
                        .analyze_cycles
                        .map(|cycles| cycles as f64 * 3600.0 / duration);
                }
                if newer.id == selected_id
                    && matches!(interval.state.as_str(), "comparable" | "partial_sample")
                {
                    selected_deltas = comparison
                        .table_changes
                        .into_iter()
                        .filter(|row| row.sample_state == "present_in_both")
                        .map(|row| (row.relation_oid, row))
                        .collect();
                }
            }
            result.intervals.push(interval);
        }
        let Some(selected_tables) = selected_tables else {
            return Ok(result);
        };
        result.relation_count = selected_tables.len();
        result.relations = selected_tables
            .iter()
            .map(|table| AutovacuumWorkloadRelation {
                relation_oid: table.relation_oid,
                relation_name: relation_name(table),
                total_size_bytes: table.total_size_bytes,
                dead_tuples: table.dead_tuples,
                dead_tuple_trigger: table.dead_tuple_trigger,
                dead_trigger_used_pct: table.dead_trigger_used_pct,
                insert_trigger_used_pct: table.insert_trigger_used_pct,
                analyze_trigger_used_pct: table.analyze_trigger_used_pct,
                dead_trigger_reached: table.dead_trigger_reached,
                insert_trigger_reached: table.insert_trigger_reached,
                analyze_trigger_reached: table.analyze_trigger_reached,
                autovacuum_enabled: table.autovacuum_enabled,
                autovacuum_applicable: table.autovacuum_applicable,
                last_autovacuum: table.last_autovacuum.clone(),
                last_autoanalyze: table.last_autoanalyze.clone(),
                delta: selected_deltas.remove(&table.relation_oid),
            })
            .collect();
        // Keep a bounded result while prioritising table-level maintenance signals.
        result.relations.sort_by(|left, right| {
            let priority = |row: &AutovacuumWorkloadRelation| {
                (
                    row.autovacuum_applicable && !row.autovacuum_enabled,
                    row.dead_trigger_reached
                        || row.insert_trigger_reached
                        || row.analyze_trigger_reached,
                )
            };
            priority(right)
                .cmp(&priority(left))
                .then_with(|| {
                    right
                        .dead_trigger_used_pct
                        .unwrap_or(-1.0)
                        .total_cmp(&left.dead_trigger_used_pct.unwrap_or(-1.0))
                })
                .then_with(|| right.total_size_bytes.cmp(&left.total_size_bytes))
        });
        result.relations.truncate(300);
        result.limitations.push(
            "История ограничена 60 снимками и 300 таблицами выбранного измерения. Отсутствие объекта в выборке не доказывает его удаление."
                .into(),
        );
        result.limitations.push(
            "n_dead_tup и пороги — оценки в момент снимка. Интервальные rates вычисляются только для пересечения сопоставимых таблиц; это не общая DML-нагрузка сервера."
                .into(),
        );
        Ok(result)
    }

    pub fn autovacuum_relation_history(
        &mut self,
        anchor_snapshot_id: i64,
        relation_oid: u32,
        limit: Option<u32>,
    ) -> Result<AutovacuumRelationHistory, String> {
        if anchor_snapshot_id <= 0 {
            return Err("Некорректный snapshot id для Autovacuum relation history".into());
        }

        let anchor = self.load_autovacuum_snapshot(anchor_snapshot_id)?;
        let anchor_table = anchor
            .snapshot
            .tables
            .data
            .as_ref()
            .and_then(|tables| tables.iter().find(|table| table.relation_oid == relation_oid))
            .ok_or_else(|| {
                format!(
                    "Relation OID {relation_oid} отсутствует в Autovacuum snapshot #{anchor_snapshot_id}"
                )
            })?;
        let anchor_identity = (
            anchor_table.schema_name.clone(),
            anchor_table.relation_name.clone(),
            anchor_table.relation_type.clone(),
        );
        let relation_name = relation_name(anchor_table);
        let limit = limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);

        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT
                    id,
                    schema_version,
                    collection_started_at_ms,
                    measured_at,
                    stats_reset,
                    snapshot_json
                FROM autovacuum_snapshots
                WHERE connection_id = ?1
                  AND host = ?2
                  AND port = ?3
                  AND database_oid = ?4
                  AND (
                        collection_started_at_ms < ?5
                        OR (collection_started_at_ms = ?5 AND id <= ?6)
                  )
                ORDER BY collection_started_at_ms DESC, id DESC
                LIMIT ?7
                "#,
            )
            .map_err(|error| {
                format!("Не удалось подготовить Autovacuum relation history: {error}")
            })?;

        let mut rows = statement
            .query(params![
                anchor.summary.connection_id,
                &anchor.summary.host,
                anchor.summary.port,
                anchor.summary.database_oid,
                anchor.summary.collection_started_at_ms,
                anchor.summary.id,
                i64::from(limit),
            ])
            .map_err(|error| {
                format!("Не удалось прочитать Autovacuum relation history: {error}")
            })?;

        let mut points = Vec::new();
        let mut limitations = Vec::new();

        while let Some(row) = rows.next().map_err(|error| {
            format!("Не удалось прочитать Autovacuum relation history row: {error}")
        })? {
            let snapshot_id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let schema_version: i64 = row.get(1).map_err(|error| error.to_string())?;
            let collection_started_at_ms: i64 = row.get(2).map_err(|error| error.to_string())?;
            let measured_at: String = row.get(3).map_err(|error| error.to_string())?;
            let stats_reset: Option<String> = row.get(4).map_err(|error| error.to_string())?;
            let snapshot_json: String = row.get(5).map_err(|error| error.to_string())?;

            if schema_version != AUTOVACUUM_SNAPSHOT_SCHEMA_VERSION {
                limitations.push(format!(
                    "Snapshot #{snapshot_id} пропущен: schema_version={schema_version}, поддерживается {AUTOVACUUM_SNAPSHOT_SCHEMA_VERSION}."
                ));
                continue;
            }

            let snapshot = serde_json::from_str::<AutovacuumSnapshot>(&snapshot_json).map_err(
                |error| {
                    format!(
                        "Не удалось разобрать Autovacuum snapshot #{snapshot_id} для relation history: {error}"
                    )
                },
            )?;

            let Some(table) = snapshot
                .tables
                .data
                .as_ref()
                .and_then(|tables| {
                    tables
                        .iter()
                        .find(|table| table.relation_oid == relation_oid)
                })
                .cloned()
            else {
                continue;
            };

            if (
                table.schema_name.as_str(),
                table.relation_name.as_str(),
                table.relation_type.as_str(),
            ) != (
                anchor_identity.0.as_str(),
                anchor_identity.1.as_str(),
                anchor_identity.2.as_str(),
            ) {
                limitations.push(format!(
                    "История остановлена на snapshot #{snapshot_id}: OID {relation_oid} принадлежал другой relation."
                ));
                break;
            }

            let active_vacuum = snapshot
                .vacuums
                .data
                .as_ref()
                .and_then(|rows| rows.iter().find(|row| row.relation_oid == relation_oid))
                .cloned();
            let active_analyze = snapshot
                .analyzes
                .data
                .as_ref()
                .and_then(|rows| rows.iter().find(|row| row.relation_oid == relation_oid))
                .cloned();

            points.push(AutovacuumRelationHistoryPoint {
                snapshot_id,
                schema_version,
                collection_started_at_ms,
                measured_at,
                stats_reset,
                table,
                active_vacuum,
                active_analyze,
            });
        }

        // Keep *all* sampled observations for the UI charts. They are point-in-time
        // estimates, not continuous counters. Advisor and Policy independently
        // select the latest comparable segment before calculating rates/candidates.
        let samples = maintenance_samples(&points);
        let comparable_samples = autovacuum::latest_continuous_maintenance_samples(&samples);
        if comparable_samples.len() < points.len() {
            let excluded = points.len() - comparable_samples.len();
            limitations.push(format!(
                "Для расчёта скоростей и рекомендаций не подходят {excluded} более ранних наблюдений: непрерывность счётчиков или настроек не подтверждена. На графиках сохранены все {} отдельные оценки.",
                points.len(),
            ));
        }

        let summary = relation_history_summary(&points);
        if !summary.stats_period_continuous && summary.history_points >= 2 {
            limitations.push(
                "Накопительные счётчики INSERT/UPDATE/DELETE, VACUUM и ANALYZE между этими снимками нельзя достоверно сопоставить; скорости и разности циклов не рассчитываются."
                    .into(),
            );
        }
        if summary.vacuum_cycles.unwrap_or(0) > 0 {
            limitations.push(
                "dead tuple delta через interval с VACUUM cycle не считается generation rate: VACUUM мог уменьшить n_dead_tup между точками."
                    .into(),
            );
        }
        limitations.push(
            "n_dead_tup, n_live_tup и reltuples — статистические оценки; history показывает operational trend, а не physical bloat."
                .into(),
        );

        Ok(AutovacuumRelationHistory {
            anchor_snapshot_id,
            relation_oid,
            relation_name,
            summary,
            points,
            limitations,
        })
    }

    pub fn autovacuum_maintenance_advice(
        &mut self,
        anchor_snapshot_id: i64,
        relation_oid: u32,
        limit: Option<u32>,
    ) -> Result<AutovacuumMaintenanceAdvice, String> {
        let anchor = self.load_autovacuum_snapshot(anchor_snapshot_id)?;
        let table = anchor
            .snapshot
            .tables
            .data
            .as_ref()
            .and_then(|tables| tables.iter().find(|table| table.relation_oid == relation_oid))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Relation OID {relation_oid} отсутствует в Autovacuum snapshot #{anchor_snapshot_id}"
                )
            })?;
        let active_vacuum = anchor
            .snapshot
            .vacuums
            .data
            .as_ref()
            .and_then(|rows| rows.iter().find(|row| row.relation_oid == relation_oid))
            .cloned();
        let active_analyze = anchor
            .snapshot
            .analyzes
            .data
            .as_ref()
            .and_then(|rows| rows.iter().find(|row| row.relation_oid == relation_oid))
            .cloned();
        let active_toast_vacuum = table.toast.as_ref().and_then(|toast| {
            anchor
                .snapshot
                .vacuums
                .data
                .as_ref()
                .and_then(|rows| {
                    rows.iter()
                        .find(|row| row.relation_oid == toast.relation_oid)
                })
                .cloned()
        });

        let history = self.autovacuum_relation_history(anchor_snapshot_id, relation_oid, limit)?;
        let samples = maintenance_samples(&history.points);

        Ok(autovacuum::advise_maintenance(
            &table,
            anchor.snapshot.overview.track_counts,
            active_vacuum.as_ref(),
            active_analyze.as_ref(),
            active_toast_vacuum.as_ref(),
            &samples,
            Some(anchor.snapshot.overview.autovacuum_naptime.as_str()),
        ))
    }

    pub fn autovacuum_policy_candidates(
        &mut self,
        anchor_snapshot_id: i64,
        relation_oid: u32,
        limit: Option<u32>,
    ) -> Result<AutovacuumPolicyCandidates, String> {
        let anchor = self.load_autovacuum_snapshot(anchor_snapshot_id)?;
        let table = anchor
            .snapshot
            .tables
            .data
            .as_ref()
            .and_then(|tables| tables.iter().find(|table| table.relation_oid == relation_oid))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Relation OID {relation_oid} отсутствует в Autovacuum snapshot #{anchor_snapshot_id}"
                )
            })?;

        let history = self.autovacuum_relation_history(anchor_snapshot_id, relation_oid, limit)?;
        let samples = maintenance_samples(&history.points);

        let active_vacuum = anchor
            .snapshot
            .vacuums
            .data
            .as_ref()
            .and_then(|rows| rows.iter().find(|row| row.relation_oid == relation_oid));
        let active_analyze = anchor
            .snapshot
            .analyzes
            .data
            .as_ref()
            .and_then(|rows| rows.iter().find(|row| row.relation_oid == relation_oid));
        let active_toast_vacuum = table.toast.as_ref().and_then(|toast| {
            anchor.snapshot.vacuums.data.as_ref().and_then(|rows| {
                rows.iter()
                    .find(|row| row.relation_oid == toast.relation_oid)
            })
        });
        let advice = autovacuum::advise_maintenance(
            &table,
            anchor.snapshot.overview.track_counts,
            active_vacuum,
            active_analyze,
            active_toast_vacuum,
            &samples,
            Some(anchor.snapshot.overview.autovacuum_naptime.as_str()),
        );

        Ok(autovacuum::build_policy_candidates(
            &table,
            &samples,
            &advice,
            anchor.snapshot.overview.track_counts,
            anchor.snapshot.overview.server_version_num,
        ))
    }

    pub fn autovacuum_policy_candidate_context(
        &mut self,
        anchor_snapshot_id: i64,
        relation_oid: u32,
        candidate_id: &str,
        limit: Option<u32>,
    ) -> Result<
        (
            AutovacuumPolicyCandidate,
            AutovacuumTable,
            AutovacuumSnapshotSummary,
        ),
        String,
    > {
        let candidates =
            self.autovacuum_policy_candidates(anchor_snapshot_id, relation_oid, limit)?;
        let candidate = candidates
            .candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| {
                format!(
                    "Autovacuum policy candidate {candidate_id:?} больше не доступен для relation OID {relation_oid} в snapshot #{anchor_snapshot_id}. Пересчитайте Concrete Policy Candidates."
                )
            })?;

        let anchor = self.load_autovacuum_snapshot(anchor_snapshot_id)?;
        let table = anchor
            .snapshot
            .tables
            .data
            .as_ref()
            .and_then(|tables| tables.iter().find(|table| table.relation_oid == relation_oid))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Relation OID {relation_oid} отсутствует в Autovacuum snapshot #{anchor_snapshot_id}"
                )
            })?;

        Ok((candidate, table, anchor.summary))
    }

    pub fn compare_autovacuum_snapshots(
        &mut self,
        first_id: i64,
        second_id: i64,
    ) -> Result<AutovacuumComparison, String> {
        if first_id == second_id {
            return Err("Для сравнения выберите два разных Autovacuum snapshot".into());
        }

        let first = self.load_autovacuum_snapshot(first_id)?;

        let second = self.load_autovacuum_snapshot(second_id)?;

        let (older, newer) =
            if snapshot_order_key(&first.summary) <= snapshot_order_key(&second.summary) {
                (first, second)
            } else {
                (second, first)
            };

        compare_snapshots(older, newer)
    }

    /// Load only a snapshot from the explicitly selected saved connection.
    /// The snapshot's PostgreSQL identity is still checked by the caller
    /// before comparing it with a currently configured live connection.
    pub fn autovacuum_snapshot(
        &mut self,
        connection_id: i64,
        snapshot_id: i64,
    ) -> Result<AutovacuumSnapshot, String> {
        let stored = self.load_autovacuum_snapshot(snapshot_id)?;
        if stored.summary.connection_id != connection_id {
            return Err("Autovacuum snapshot принадлежит другому подключению".into());
        }
        Ok(stored.snapshot)
    }

    fn load_autovacuum_snapshot(&mut self, id: i64) -> Result<StoredAutovacuumSnapshot, String> {
        let sql = format!("{} WHERE id = ?1", stored_select_sql(),);

        self.connection
            .query_row(&sql, params![id], |row| {
                let summary = summary_from_row(row)?;

                let snapshot_json: String = row.get(28)?;

                Ok((summary, snapshot_json))
            })
            .map_err(|error| format!("Autovacuum snapshot #{id} не найден или повреждён: {error}"))
            .and_then(|(summary, snapshot_json)| {
                if summary.schema_version != AUTOVACUUM_SNAPSHOT_SCHEMA_VERSION {
                    return Err(format!(
                        "Autovacuum snapshot #{} имеет schema_version={}, поддерживается {}",
                        summary.id, summary.schema_version, AUTOVACUUM_SNAPSHOT_SCHEMA_VERSION,
                    ));
                }

                let snapshot = serde_json::from_str::<AutovacuumSnapshot>(&snapshot_json).map_err(
                    |error| {
                        format!(
                            "Не удалось разобрать сохранённый Autovacuum snapshot #{}: {error}",
                            summary.id,
                        )
                    },
                )?;

                Ok(StoredAutovacuumSnapshot { summary, snapshot })
            })
    }
}

fn sum_comparable_counters(
    rows: &[AutovacuumRelationDelta],
    value: impl Fn(&AutovacuumRelationDelta) -> Option<i64>,
) -> Option<i64> {
    rows.iter()
        .filter(|row| row.sample_state == "present_in_both")
        .try_fold(0_i64, |sum, row| sum.checked_add(value(row)?))
}

fn counter_rate_from_rows(
    rows: &[AutovacuumRelationDelta],
    elapsed_seconds: f64,
    value: impl Fn(&AutovacuumRelationDelta) -> Option<i64>,
) -> Option<f64> {
    if elapsed_seconds <= 0.0 || !elapsed_seconds.is_finite() {
        return None;
    }
    sum_comparable_counters(rows, value).map(|delta| delta as f64 * 3600.0 / elapsed_seconds)
}

fn maintenance_samples(
    points: &[AutovacuumRelationHistoryPoint],
) -> Vec<AutovacuumMaintenanceHistorySample> {
    points
        .iter()
        .map(|point| AutovacuumMaintenanceHistorySample {
            collection_started_at_ms: point.collection_started_at_ms,
            stats_reset: point.stats_reset.clone(),
            policy: Some(
                autovacuum::AutovacuumMaintenancePolicySignature::from_table(&point.table),
            ),
            dead_tuples: point.table.dead_tuples,
            inserts_since_vacuum: point.table.inserts_since_vacuum,
            modifications_since_analyze: point.table.modifications_since_analyze,
            tuples_inserted: point.table.tuples_inserted,
            tuples_updated: point.table.tuples_updated,
            tuples_deleted: point.table.tuples_deleted,
            hot_updates: point.table.hot_updates,
            vacuum_count: point.table.vacuum_count,
            autovacuum_count: point.table.autovacuum_count,
            analyze_count: point.table.analyze_count,
            autoanalyze_count: point.table.autoanalyze_count,
            dead_trigger_reached: point.table.dead_trigger_reached,
            insert_trigger_reached: point.table.insert_trigger_reached,
            analyze_trigger_reached: point.table.analyze_trigger_reached,
            toast_autovacuum_count: point
                .table
                .toast
                .as_ref()
                .map(|toast| toast.autovacuum_count),
            toast_dead_tuples: point.table.toast.as_ref().map(|toast| toast.dead_tuples),
            toast_inserts_since_vacuum: point
                .table
                .toast
                .as_ref()
                .map(|toast| toast.inserts_since_vacuum),
            toast_dead_trigger_reached: point
                .table
                .toast
                .as_ref()
                .map(|toast| toast.dead_trigger_reached),
            toast_insert_trigger_reached: point
                .table
                .toast
                .as_ref()
                .map(|toast| toast.insert_trigger_reached),
        })
        .collect()
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
        collection_started_at_ms,
        collection_completed_at_ms,
        measured_at,
        stats_reset,
        table_count,
        total_size_bytes,
        total_dead_tuples,
        dead_trigger_reached_count,
        insert_trigger_reached_count,
        analyze_trigger_reached_count,
        autovacuum_disabled_count,
        active_vacuum_count,
        active_analyze_count,
        unavailable_sections,
        stored_at_ms
    FROM autovacuum_snapshots
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
        collection_started_at_ms,
        collection_completed_at_ms,
        measured_at,
        stats_reset,
        table_count,
        total_size_bytes,
        total_dead_tuples,
        dead_trigger_reached_count,
        insert_trigger_reached_count,
        analyze_trigger_reached_count,
        autovacuum_disabled_count,
        active_vacuum_count,
        active_analyze_count,
        unavailable_sections,
        stored_at_ms,
        snapshot_json
    FROM autovacuum_snapshots
    "#
}

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<AutovacuumSnapshotSummary> {
    Ok(AutovacuumSnapshotSummary {
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
        collection_started_at_ms: row.get(13)?,
        collection_completed_at_ms: row.get(14)?,
        measured_at: row.get(15)?,
        stats_reset: row.get(16)?,
        table_count: row.get(17)?,
        total_size_bytes: row.get(18)?,
        total_dead_tuples: row.get(19)?,
        dead_trigger_reached_count: row.get(20)?,
        insert_trigger_reached_count: row.get(21)?,
        analyze_trigger_reached_count: row.get(22)?,
        autovacuum_disabled_count: row.get(23)?,
        active_vacuum_count: row.get(24)?,
        active_analyze_count: row.get(25)?,
        unavailable_sections: row.get(26)?,
        stored_at_ms: row.get(27)?,
    })
}

/// An unknown reset time is not evidence that a reset actually happened.
/// Both cases prevent cumulative-counter deltas, but need different messages.
fn stats_period_limitation(older: Option<&str>, newer: Option<&str>) -> Option<&'static str> {
    match (older, newer) {
        (Some(before), Some(after)) if before == after => None,
        (Some(_), Some(_)) => Some(
            "Время сброса статистики различается между снимками. Разности накопительных счётчиков INSERT/UPDATE/DELETE, VACUUM и ANALYZE не рассчитываются.",
        ),
        _ => Some(
            "Время сброса статистики (stats_reset) неизвестно хотя бы в одном снимке. Это не доказывает сброс, но не позволяет достоверно рассчитать разности накопительных счётчиков INSERT/UPDATE/DELETE, VACUUM и ANALYZE.",
        ),
    }
}

fn compare_snapshots(
    older: StoredAutovacuumSnapshot,
    newer: StoredAutovacuumSnapshot,
) -> Result<AutovacuumComparison, String> {
    validate_same_target(&older.summary, &newer.summary)?;

    let elapsed_seconds = (newer.summary.collection_started_at_ms
        - older.summary.collection_started_at_ms) as f64
        / 1000.0;

    if elapsed_seconds <= 0.0 {
        return Err("Интервал между Autovacuum snapshots должен быть больше нуля".into());
    }

    let continuity_limitation = stats_period_limitation(
        older.summary.stats_reset.as_deref(),
        newer.summary.stats_reset.as_deref(),
    );
    let stats_period_continuous = continuity_limitation.is_none();

    let mut limitations = Vec::new();

    if let Some(reason) = continuity_limitation {
        limitations.push(reason.to_owned());
    }

    if older.snapshot.tables.error.is_some() || newer.snapshot.tables.error.is_some() {
        limitations.push(
            "Table section недоступна хотя бы в одном snapshot; per-table comparison неполон."
                .into(),
        );
    }

    let table_changes =
        if older.snapshot.tables.data.is_some() && newer.snapshot.tables.data.is_some() {
            compare_tables(
                older.snapshot.tables.data.as_deref(),
                newer.snapshot.tables.data.as_deref(),
                elapsed_seconds,
                stats_period_continuous,
                &mut limitations,
            )
        } else {
            Vec::new()
        };

    let interpretation = comparison_interpretation(
        &older.summary,
        &newer.summary,
        stats_period_continuous,
        &table_changes,
    );

    Ok(AutovacuumComparison {
        older: older.summary,
        newer: newer.summary,
        elapsed_seconds,
        stats_period_continuous,
        table_changes,
        interpretation,
        limitations,
    })
}

fn compare_tables(
    older: Option<&[AutovacuumTable]>,
    newer: Option<&[AutovacuumTable]>,
    elapsed_seconds: f64,
    stats_period_continuous: bool,
    limitations: &mut Vec<String>,
) -> Vec<AutovacuumRelationDelta> {
    let older_map: HashMap<u32, &AutovacuumTable> = older
        .unwrap_or(&[])
        .iter()
        .map(|row| (row.relation_oid, row))
        .collect();

    let newer_map: HashMap<u32, &AutovacuumTable> = newer
        .unwrap_or(&[])
        .iter()
        .map(|row| (row.relation_oid, row))
        .collect();

    let mut ids: HashSet<u32> = older_map.keys().copied().collect();

    ids.extend(newer_map.keys().copied());

    let mut result = Vec::with_capacity(ids.len());

    for relation_oid in ids {
        let old = older_map.get(&relation_oid).copied();

        let new = newer_map.get(&relation_oid).copied();

        if let (Some(left), Some(right)) = (old, new) {
            if left.schema_name != right.schema_name
                || left.relation_name != right.relation_name
                || left.relation_type != right.relation_type
            {
                limitations.push(format!(
                    "Autovacuum relation OID {relation_oid} сменил идентичность; deltas для него исключены."
                ));
                continue;
            }
        }

        let vacuum_count_delta = monotonic_counter_delta(
            old.map(|row| row.vacuum_count),
            new.map(|row| row.vacuum_count),
            stats_period_continuous,
        );

        let autovacuum_count_delta = monotonic_counter_delta(
            old.map(|row| row.autovacuum_count),
            new.map(|row| row.autovacuum_count),
            stats_period_continuous,
        );

        let analyze_count_delta = monotonic_counter_delta(
            old.map(|row| row.analyze_count),
            new.map(|row| row.analyze_count),
            stats_period_continuous,
        );

        let autoanalyze_count_delta = monotonic_counter_delta(
            old.map(|row| row.autoanalyze_count),
            new.map(|row| row.autoanalyze_count),
            stats_period_continuous,
        );

        let toast_autovacuum_count_delta = monotonic_counter_delta(
            old.and_then(|row| row.toast.as_ref().map(|toast| toast.autovacuum_count)),
            new.and_then(|row| row.toast.as_ref().map(|toast| toast.autovacuum_count)),
            stats_period_continuous,
        );

        let toast_vacuum_cycle_observed = toast_autovacuum_count_delta.unwrap_or(0) > 0;

        let vacuum_cycle_observed =
            vacuum_count_delta.unwrap_or(0) > 0 || autovacuum_count_delta.unwrap_or(0) > 0;

        let analyze_cycle_observed =
            analyze_count_delta.unwrap_or(0) > 0 || autoanalyze_count_delta.unwrap_or(0) > 0;

        let dead_tuples_delta = sampled_delta(
            old.map(|row| row.dead_tuples),
            new.map(|row| row.dead_tuples),
        );

        let dead_growth_per_day = if stats_period_continuous && !vacuum_cycle_observed {
            dead_tuples_delta
                .filter(|delta| *delta >= 0)
                .map(|delta| delta as f64 / elapsed_seconds * 86_400.0)
        } else {
            None
        };

        let estimated_days_to_dead_trigger = match (new, dead_growth_per_day) {
            (Some(new), Some(rate)) if rate > 0.0 => {
                let remaining = (new.dead_tuple_trigger - new.dead_tuples).max(0);

                Some(remaining as f64 / rate)
            }
            _ => None,
        };

        let inserts_delta = monotonic_counter_delta(
            old.map(|row| row.tuples_inserted),
            new.map(|row| row.tuples_inserted),
            stats_period_continuous,
        );

        let updates_delta = monotonic_counter_delta(
            old.map(|row| row.tuples_updated),
            new.map(|row| row.tuples_updated),
            stats_period_continuous,
        );

        let deletes_delta = monotonic_counter_delta(
            old.map(|row| row.tuples_deleted),
            new.map(|row| row.tuples_deleted),
            stats_period_continuous,
        );

        let hot_updates_delta = monotonic_counter_delta(
            old.map(|row| row.hot_updates),
            new.map(|row| row.hot_updates),
            stats_period_continuous,
        );

        let interval_workload_profile =
            interval_workload_profile(inserts_delta, updates_delta, deletes_delta);

        result.push(AutovacuumRelationDelta {
            relation_oid,
            older_name: old.map(relation_name),
            newer_name: new.map(relation_name),
            sample_state: sample_state(old.is_some(), new.is_some()),
            older_live_tuples: old.map(|row| row.live_tuples),
            newer_live_tuples: new.map(|row| row.live_tuples),
            older_dead_tuples: old.map(|row| row.dead_tuples),
            newer_dead_tuples: new.map(|row| row.dead_tuples),
            dead_tuples_delta,
            older_toast_dead_tuples: old
                .and_then(|row| row.toast.as_ref().map(|toast| toast.dead_tuples)),
            newer_toast_dead_tuples: new
                .and_then(|row| row.toast.as_ref().map(|toast| toast.dead_tuples)),
            toast_dead_tuples_delta: sampled_delta(
                old.and_then(|row| row.toast.as_ref().map(|toast| toast.dead_tuples)),
                new.and_then(|row| row.toast.as_ref().map(|toast| toast.dead_tuples)),
            ),
            inserts_delta,
            updates_delta,
            deletes_delta,
            hot_updates_delta,
            vacuum_count_delta,
            autovacuum_count_delta,
            analyze_count_delta,
            autoanalyze_count_delta,
            toast_autovacuum_count_delta,
            vacuum_cycle_observed,
            toast_vacuum_cycle_observed,
            analyze_cycle_observed,
            dead_growth_per_day,
            estimated_days_to_dead_trigger,
            older_dead_trigger: old.map(|row| row.dead_tuple_trigger),
            newer_dead_trigger: new.map(|row| row.dead_tuple_trigger),
            older_dead_trigger_used_pct: old.and_then(|row| row.dead_trigger_used_pct),
            newer_dead_trigger_used_pct: new.and_then(|row| row.dead_trigger_used_pct),
            older_total_size_bytes: old.map(|row| row.total_size_bytes),
            newer_total_size_bytes: new.map(|row| row.total_size_bytes),
            total_size_delta_bytes: sampled_delta(
                old.map(|row| row.total_size_bytes),
                new.map(|row| row.total_size_bytes),
            ),
            older_workload_profile: old.map(|row| row.workload_profile.clone()),
            newer_workload_profile: new.map(|row| row.workload_profile.clone()),
            interval_workload_profile,
        });
    }

    result.sort_by(|a, b| {
        let b_activity = activity_delta(b);
        let a_activity = activity_delta(a);

        b_activity.cmp(&a_activity).then_with(|| {
            b.newer_dead_trigger_used_pct
                .unwrap_or(0.0)
                .partial_cmp(&a.newer_dead_trigger_used_pct.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    result
}

fn relation_history_summary(
    points: &[AutovacuumRelationHistoryPoint],
) -> AutovacuumRelationHistorySummary {
    let history_points = points.len();
    let (newest, oldest) = (points.first(), points.last());
    let history_span_seconds = match (oldest, newest) {
        (Some(oldest), Some(newest))
            if newest.collection_started_at_ms > oldest.collection_started_at_ms =>
        {
            Some(
                (newest.collection_started_at_ms - oldest.collection_started_at_ms) as f64 / 1000.0,
            )
        }
        _ => None,
    };

    let samples = maintenance_samples(points);
    let stats_period_continuous = history_points >= 2
        && samples
            .windows(2)
            .all(|pair| autovacuum::maintenance_stats_pair_continuous(&pair[1], &pair[0]));

    let elapsed_hours = history_span_seconds.map(|seconds| seconds / 3600.0);
    let counter_rate = |older: Option<i64>, newer: Option<i64>| -> Option<f64> {
        let hours = elapsed_hours.filter(|hours| *hours > 0.0)?;
        let delta = monotonic_counter_delta(older, newer, stats_period_continuous)?;
        Some(delta as f64 / hours)
    };

    let inserts_per_hour = counter_rate(
        oldest.map(|point| point.table.tuples_inserted),
        newest.map(|point| point.table.tuples_inserted),
    );
    let updates_per_hour = counter_rate(
        oldest.map(|point| point.table.tuples_updated),
        newest.map(|point| point.table.tuples_updated),
    );
    let deletes_per_hour = counter_rate(
        oldest.map(|point| point.table.tuples_deleted),
        newest.map(|point| point.table.tuples_deleted),
    );

    let updates_delta = monotonic_counter_delta(
        oldest.map(|point| point.table.tuples_updated),
        newest.map(|point| point.table.tuples_updated),
        stats_period_continuous,
    );
    let hot_updates_delta = monotonic_counter_delta(
        oldest.map(|point| point.table.hot_updates),
        newest.map(|point| point.table.hot_updates),
        stats_period_continuous,
    );
    let hot_update_pct = match (hot_updates_delta, updates_delta) {
        (Some(hot), Some(updates)) if updates > 0 => Some(hot as f64 / updates as f64 * 100.0),
        _ => None,
    };

    let vacuum_count_delta = monotonic_counter_delta(
        oldest.map(|point| point.table.vacuum_count),
        newest.map(|point| point.table.vacuum_count),
        stats_period_continuous,
    );
    let autovacuum_count_delta = monotonic_counter_delta(
        oldest.map(|point| point.table.autovacuum_count),
        newest.map(|point| point.table.autovacuum_count),
        stats_period_continuous,
    );
    let vacuum_cycles = match (vacuum_count_delta, autovacuum_count_delta) {
        (Some(manual), Some(auto)) => manual.checked_add(auto),
        _ => None,
    };

    let analyze_count_delta = monotonic_counter_delta(
        oldest.map(|point| point.table.analyze_count),
        newest.map(|point| point.table.analyze_count),
        stats_period_continuous,
    );
    let autoanalyze_count_delta = monotonic_counter_delta(
        oldest.map(|point| point.table.autoanalyze_count),
        newest.map(|point| point.table.autoanalyze_count),
        stats_period_continuous,
    );
    let analyze_cycles = match (analyze_count_delta, autoanalyze_count_delta) {
        (Some(manual), Some(auto)) => manual.checked_add(auto),
        _ => None,
    };

    let toast_vacuum_cycles = monotonic_counter_delta(
        oldest.and_then(|point| {
            point
                .table
                .toast
                .as_ref()
                .map(|toast| toast.autovacuum_count)
        }),
        newest.and_then(|point| {
            point
                .table
                .toast
                .as_ref()
                .map(|toast| toast.autovacuum_count)
        }),
        stats_period_continuous,
    );

    let dead_growth_per_hour = match (oldest, newest, elapsed_hours, vacuum_cycles) {
        (Some(oldest), Some(newest), Some(hours), Some(0)) if hours > 0.0 => {
            let delta = newest.table.dead_tuples - oldest.table.dead_tuples;
            (delta >= 0).then_some(delta as f64 / hours)
        }
        _ => None,
    };

    AutovacuumRelationHistorySummary {
        history_points,
        history_span_seconds,
        stats_period_continuous,
        inserts_per_hour,
        updates_per_hour,
        deletes_per_hour,
        hot_update_pct,
        vacuum_cycles,
        analyze_cycles,
        toast_vacuum_cycles,
        dead_growth_per_hour,
        dead_trigger_reached_points: points
            .iter()
            .filter(|point| point.table.dead_trigger_reached)
            .count(),
        analyze_trigger_reached_points: points
            .iter()
            .filter(|point| point.table.analyze_trigger_reached)
            .count(),
    }
}

fn comparison_interpretation(
    older: &AutovacuumSnapshotSummary,
    newer: &AutovacuumSnapshotSummary,
    stats_period_continuous: bool,
    changes: &[AutovacuumRelationDelta],
) -> AutovacuumInterpretation {
    let mut items = Vec::new();

    let mut limitations =
        vec![
            "dead growth/day рассчитывается только для relations, где между snapshots не наблюдался VACUUM counter increment."
                .to_owned(),
            "Расчёт days-to-trigger использует изменение n_dead_tup estimate между двумя точками; это operational approximation, не forecast guarantee."
                .to_owned(),
        ];

    if !stats_period_continuous {
        let reset_changed = older.stats_reset.is_some() && newer.stats_reset.is_some();
        items.push(interpretation_item(
            if reset_changed {
                "stats-reset-changed"
            } else {
                "stats-reset-unknown"
            },
            if reset_changed {
                "Время сброса статистики изменилось"
            } else {
                "Непрерывность статистики не подтверждена"
            },
            vec![format!(
                "stats_reset: {:?} → {:?}",
                older.stats_reset, newer.stats_reset,
            )],
            vec![
                if reset_changed {
                    "Снимки относятся к разным периодам накопительной статистики."
                } else {
                    "Время сброса неизвестно хотя бы для одного снимка. Сам факт сброса не установлен."
                }
                .into(),
            ],
            vec![
                "Сравнивать отдельные оценки таблиц можно, но не рассчитывать скорости и разности накопительных счётчиков без подтверждённой непрерывности."
                    .into(),
            ],
        ));
    }

    let vacuum_cycles = changes
        .iter()
        .filter(|row| row.vacuum_cycle_observed)
        .count();

    let toast_vacuum_cycles = changes
        .iter()
        .filter(|row| row.toast_vacuum_cycle_observed)
        .count();

    if vacuum_cycles > 0 {
        items.push(
            interpretation_item(
                "vacuum-cycles",
                "Между snapshots наблюдались VACUUM cycles",
                vec![
                    format!("relations with vacuum counter increment: {vacuum_cycles}"),
                ],
                vec![
                    "Для этих relations простой dead_tuples delta нельзя интерпретировать как скорость генерации мусора."
                        .into(),
                ],
                vec![
                    "Смотреть autovacuum_count delta, newer dead tuples и trigger usage после cycle."
                        .into(),
                ],
            ),
        );
    }

    if toast_vacuum_cycles > 0 {
        items.push(
            interpretation_item(
                "toast-vacuum-cycles",
                "Между snapshots обслуживались TOAST relations",
                vec![
                    format!("relations with TOAST autovacuum counter increment: {toast_vacuum_cycles}"),
                ],
                vec![
                    "TOAST имеет собственные maintenance counters и может обслуживаться независимо от основной table."
                        .into(),
                ],
                vec![
                    "Для этих tables открыть detail и проверить TOAST reloptions/thresholds."
                        .into(),
                ],
            ),
        );
    }

    if let Some(row) = changes
        .iter()
        .filter_map(|row| {
            row.dead_growth_per_day
                .filter(|rate| *rate > 0.0)
                .map(|rate| (rate, row))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    {
        let (rate, row) = row;

        items.push(interpretation_item(
            "dead-growth",
            "Есть relation с измеримым ростом dead tuple estimate",
            vec![
                format!(
                    "{}: ~{rate:.0} dead tuples/day",
                    row.newer_name
                        .as_deref()
                        .or(row.older_name.as_deref())
                        .unwrap_or("<unknown>"),
                ),
                row.estimated_days_to_dead_trigger
                    .map(|days| format!("estimated time to current trigger: {days:.2} days"))
                    .unwrap_or_else(|| "time to trigger: unknown".into()),
            ],
            vec![
                "Это двухточечная оценка при отсутствии наблюдаемого VACUUM cycle в интервале."
                    .into(),
            ],
            vec![
                "Повторить измерение на representative workload window перед policy change.".into(),
            ],
        ));
    }

    if newer.dead_trigger_reached_count > older.dead_trigger_reached_count {
        items.push(
            interpretation_item(
                "more-dead-triggers",
                "Стало больше tables на/выше dead-tuple trigger",
                vec![
                    format!(
                        "{} → {}",
                        older.dead_trigger_reached_count,
                        newer.dead_trigger_reached_count,
                    ),
                ],
                vec![
                    "Это повод проверить workers/cadence, но не автоматическое доказательство неправильного scale factor."
                        .into(),
                ],
                vec![
                    "Открыть top table changes и проверить active/observed vacuum cycles."
                        .into(),
                ],
            ),
        );
    }

    if changes.is_empty() {
        limitations
            .push("Per-table comparison пуст: таблицы могли быть недоступны в snapshot.".into());
    }

    AutovacuumInterpretation { items, limitations }
}

fn interpretation_item(
    id: &str,
    title: &str,
    observed: Vec<String>,
    meaning: Vec<String>,
    next_steps: Vec<String>,
) -> AutovacuumInterpretationItem {
    AutovacuumInterpretationItem {
        id: id.to_owned(),
        title: title.to_owned(),
        severity: AutovacuumInterpretationSeverity::Info,
        observed,
        meaning,
        next_steps,
    }
}

fn validate_same_target(
    first: &AutovacuumSnapshotSummary,
    second: &AutovacuumSnapshotSummary,
) -> Result<(), String> {
    if first.connection_id != second.connection_id
        || first.host != second.host
        || first.port != second.port
        || first.database_oid != second.database_oid
        || first.username != second.username
        || first.sslmode != second.sslmode
    {
        return Err("Autovacuum snapshots относятся к разным PostgreSQL targets".into());
    }

    Ok(())
}

fn monotonic_counter_delta(
    older: Option<i64>,
    newer: Option<i64>,
    stats_period_continuous: bool,
) -> Option<i64> {
    if !stats_period_continuous {
        return None;
    }

    match (older, newer) {
        (Some(older), Some(newer)) if newer >= older => Some(newer - older),
        _ => None,
    }
}

fn sampled_delta(older: Option<i64>, newer: Option<i64>) -> Option<i64> {
    match (older, newer) {
        (Some(older), Some(newer)) => Some(newer - older),
        _ => None,
    }
}

fn interval_workload_profile(
    inserts: Option<i64>,
    updates: Option<i64>,
    deletes: Option<i64>,
) -> Option<String> {
    let (inserts, updates, deletes) = (inserts?, updates?, deletes?);

    let profile = if inserts == 0 && updates == 0 && deletes == 0 {
        "no_activity"
    } else if inserts > 0 && updates == 0 && deletes == 0 {
        "append_only_observed"
    } else if updates + deletes >= inserts {
        "update_delete_heavy"
    } else {
        "mixed"
    };

    Some(profile.to_owned())
}

fn activity_delta(row: &AutovacuumRelationDelta) -> i64 {
    row.inserts_delta.unwrap_or(0) + row.updates_delta.unwrap_or(0) + row.deletes_delta.unwrap_or(0)
}

fn relation_name(row: &AutovacuumTable) -> String {
    format!("{}.{}", row.schema_name, row.relation_name,)
}

fn sample_state(older: bool, newer: bool) -> String {
    match (older, newer) {
        (true, true) => "present_in_both",
        (false, true) => "entered_new_sample",
        (true, false) => "not_in_new_sample",
        (false, false) => unreachable!(),
    }
    .to_owned()
}

fn count_tables<F>(rows: &[AutovacuumTable], predicate: F) -> i64
where
    F: Fn(&AutovacuumTable) -> bool,
{
    i64::try_from(rows.iter().filter(|row| predicate(row)).count()).unwrap_or(i64::MAX)
}

fn section_len<T>(section: &AutovacuumSection<Vec<T>>) -> i64 {
    section
        .data
        .as_ref()
        .and_then(|rows| i64::try_from(rows.len()).ok())
        .unwrap_or(0)
}

fn unavailable_section_count(snapshot: &AutovacuumSnapshot) -> i64 {
    [
        snapshot.tables.error.is_some(),
        snapshot.vacuums.error.is_some(),
        snapshot.analyzes.error.is_some(),
    ]
    .into_iter()
    .filter(|value| *value)
    .count() as i64
}

fn snapshot_order_key(summary: &AutovacuumSnapshotSummary) -> (i64, i64) {
    (summary.collection_started_at_ms, summary.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_history_rates_require_non_reset_counters() {
        let intact = AutovacuumRelationDelta {
            sample_state: "present_in_both".into(),
            inserts_delta: Some(720),
            updates_delta: Some(100),
            deletes_delta: Some(10),
            vacuum_count_delta: Some(0),
            autovacuum_count_delta: Some(2),
            ..AutovacuumRelationDelta::default()
        };
        assert_eq!(
            counter_rate_from_rows(&[intact], 3600.0, |row| row.inserts_delta),
            Some(720.0)
        );
        let reset = AutovacuumRelationDelta {
            sample_state: "present_in_both".into(),
            inserts_delta: None,
            ..AutovacuumRelationDelta::default()
        };
        assert_eq!(
            counter_rate_from_rows(&[reset], 3600.0, |row| row.inserts_delta),
            None
        );
        assert_eq!(
            counter_rate_from_rows(&[], 0.0, |row| row.inserts_delta),
            None
        );
    }

    #[test]
    fn dead_growth_is_not_called_rate_when_vacuum_happened() {
        assert_eq!(monotonic_counter_delta(Some(1), Some(2), true,), Some(1),);

        assert_eq!(monotonic_counter_delta(Some(2), Some(1), true,), None,);
    }

    #[test]
    fn unknown_stats_reset_is_not_called_an_observed_reset() {
        let unknown =
            stats_period_limitation(None, None).expect("unknown reset cannot prove continuity");
        assert!(unknown.contains("не доказывает сброс"));
        assert!(!unknown.contains("различается"));
        assert!(stats_period_limitation(Some("2026-11-01"), None).is_some());
        assert!(stats_period_limitation(None, Some("2026-11-01")).is_some());
    }

    #[test]
    fn matching_and_changed_known_reset_timestamps_are_distinguished() {
        assert!(stats_period_limitation(Some("2026-11-01"), Some("2026-11-01")).is_none());
        let changed = stats_period_limitation(Some("2026-11-01"), Some("2026-11-13"))
            .expect("different known timestamps must break continuity");
        assert!(changed.contains("различается"));
    }
}
