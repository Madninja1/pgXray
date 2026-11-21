use super::what_changed_maintenance::MaintenanceTargetIdentity;
use super::what_changed_storage::{
    StorageTargetIdentity, WhatChangedIndexHealthComparison, WhatChangedStorageComparison,
};
use super::{
    unix_time_millis, DiagnosticStore, PgSettingChange, PgSettingsChangeEvent,
    PgSettingsComparison, PgSettingsSnapshotSummary, WhatChangedAutovacuumComparison,
    WhatChangedXidComparison,
};
use crate::pgss::{PgssPerformanceRow, PgssSnapshot};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const CHANGE_MARKER_SCHEMA_VERSION: i64 = 1;
const MAX_STATEMENT_CHANGES: usize = 100;
const MAX_TIMELINE_PGSS_EVENTS: usize = 20;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WhatChangedPeriod {
    pub from_ms: i64,
    pub to_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedTarget {
    pub connection_id: i64,
    pub connection_name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub database_oid: i64,
    pub username: String,
    pub server_version: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChangeMarker {
    pub id: i64,
    pub schema_version: i64,
    pub connection_id: i64,
    pub occurred_at_ms: i64,
    pub kind: String,
    pub title: String,
    pub note: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgssPeriodQuality {
    pub state: String,
    pub snapshot_count: usize,
    pub snapshot_ids: Vec<i64>,
    pub safe_interval_count: usize,
    pub unsafe_interval_count: usize,
    pub observed_span_seconds: Option<f64>,
    pub safe_elapsed_seconds: f64,
    pub safe_time_coverage_pct: Option<f64>,
    pub min_sample_coverage_pct: Option<f64>,
    pub max_sample_coverage_pct: Option<f64>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgssStatementPeriodMetrics {
    pub userid: u32,
    pub dbid: u32,
    pub queryid: String,
    pub toplevel: bool,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub first_observed_at_ms: i64,
    pub last_observed_at_ms: i64,
    pub sampled_snapshots: usize,
    pub sampled_snapshot_ids: Vec<i64>,
    pub safe_intervals: usize,
    pub safe_elapsed_seconds: f64,
    pub safe_time_coverage_pct: Option<f64>,
    pub calls_delta: i64,
    pub calls_per_hour: Option<f64>,
    pub plans_delta: i64,
    pub total_exec_time_delta_ms: f64,
    pub total_exec_time_per_hour_ms: Option<f64>,
    pub exec_ms_per_call: Option<f64>,
    pub calls_workload_share_pct: Option<f64>,
    pub exec_workload_share_pct: Option<f64>,
    pub rows_per_call: Option<f64>,
    pub shared_reads_per_call: Option<f64>,
    pub shared_hits_per_call: Option<f64>,
    pub temp_reads_per_call: Option<f64>,
    pub temp_writes_per_call: Option<f64>,
    pub wal_bytes_per_call: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedMetricChange {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub baseline_value: Option<f64>,
    pub incident_value: Option<f64>,
    pub absolute_delta: Option<f64>,
    pub relative_change_pct: Option<f64>,
    pub direction: String,
    pub evidence_level: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedStatementChange {
    pub userid: u32,
    pub dbid: u32,
    pub queryid: String,
    pub toplevel: bool,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub state: String,
    pub evidence_level: String,
    pub observed_at_ms: i64,
    pub baseline: Option<PgssStatementPeriodMetrics>,
    pub incident: Option<PgssStatementPeriodMetrics>,
    pub metrics: Vec<WhatChangedMetricChange>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedPgssComparison {
    pub baseline_quality: PgssPeriodQuality,
    pub incident_quality: PgssPeriodQuality,
    pub statement_changes: Vec<WhatChangedStatementChange>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedSettingChange {
    pub change: PgSettingChange,
    pub first_observed_at_ms: Option<i64>,
    pub changed_after_ms: Option<i64>,
    pub changed_before_ms: Option<i64>,
    pub older_snapshot_id: Option<i64>,
    pub newer_snapshot_id: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedSettingsComparison {
    pub state: String,
    pub baseline_snapshot: Option<PgSettingsSnapshotSummary>,
    pub incident_snapshot: Option<PgSettingsSnapshotSummary>,
    pub changes: Vec<WhatChangedSettingChange>,
    pub observed_events: Vec<PgSettingsChangeEvent>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedTimelineEvent {
    pub occurred_at_ms: i64,
    pub phase: String,
    pub source: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub evidence_level: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedComparison {
    pub generated_at_ms: i64,
    pub connection_id: i64,
    pub target: WhatChangedTarget,
    pub baseline: WhatChangedPeriod,
    pub incident: WhatChangedPeriod,
    pub pgss: WhatChangedPgssComparison,
    pub settings: WhatChangedSettingsComparison,
    pub autovacuum: WhatChangedAutovacuumComparison,
    pub xid: WhatChangedXidComparison,
    pub storage: WhatChangedStorageComparison,
    pub index_health: WhatChangedIndexHealthComparison,
    pub markers: Vec<ChangeMarker>,
    pub timeline: Vec<WhatChangedTimelineEvent>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StatementKey {
    userid: u32,
    dbid: u32,
    queryid: String,
    toplevel: bool,
}

struct StoredPgssPeriodSnapshot {
    id: i64,
    connection_name: String,
    host: String,
    port: i64,
    database: String,
    database_oid: i64,
    username: String,
    server_version: String,
    collection_started_at_ms: i64,
    stats_reset: Option<String>,
    dealloc: Option<i64>,
    snapshot: PgssSnapshot,
}

#[derive(Clone)]
struct TargetIdentity {
    connection_name: String,
    host: String,
    port: i64,
    database: String,
    database_oid: i64,
    username: String,
    server_version: String,
}

#[derive(Default)]
struct StatementAccumulator {
    role_name: Option<String>,
    database: Option<String>,
    first_observed_at_ms: i64,
    last_observed_at_ms: i64,
    sampled_snapshot_ids: HashSet<i64>,
    safe_intervals: usize,
    safe_elapsed_seconds: f64,
    calls_delta: i64,
    plans_delta: i64,
    total_exec_time_delta_ms: f64,
    total_plan_time_delta_ms: f64,
    rows_delta: i64,
    shared_reads_delta: i64,
    shared_hits_delta: i64,
    temp_reads_delta: i64,
    temp_writes_delta: i64,
    wal_bytes_delta: f64,
    denominator_calls_delta: i64,
    denominator_exec_time_delta_ms: f64,
}

struct PgssPeriodAggregate {
    target: Option<TargetIdentity>,
    quality: PgssPeriodQuality,
    statements: HashMap<StatementKey, PgssStatementPeriodMetrics>,
}

pub(super) fn init_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS change_markers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                schema_version INTEGER NOT NULL,
                connection_id INTEGER NOT NULL,
                occurred_at_ms INTEGER NOT NULL,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                note TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_change_markers_connection_time
                ON change_markers(connection_id, occurred_at_ms DESC, id DESC);
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить What Changed marker schema: {error}"))
}

impl DiagnosticStore {
    pub fn create_change_marker(
        &mut self,
        connection_id: i64,
        occurred_at_ms: i64,
        kind: &str,
        title: &str,
        note: &str,
    ) -> Result<ChangeMarker, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection id для marker",
                "Invalid connection ID for marker",
            )
            .into());
        }
        if occurred_at_ms <= 0 {
            return Err(
                crate::i18n::text("Некорректное время marker", "Invalid marker time").into(),
            );
        }

        let kind = validate_marker_kind(kind)?;
        let title = validate_marker_text(
            title,
            crate::i18n::text("Название marker", "Marker title"),
            1,
            160,
        )?;
        let note = validate_marker_text(
            note,
            crate::i18n::text("Описание marker", "Marker description"),
            0,
            4_000,
        )?;
        let created_at_ms = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT INTO change_markers (
                    schema_version,
                    connection_id,
                    occurred_at_ms,
                    kind,
                    title,
                    note,
                    created_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    CHANGE_MARKER_SCHEMA_VERSION,
                    connection_id,
                    occurred_at_ms,
                    kind,
                    title,
                    note,
                    created_at_ms,
                ],
            )
            .map_err(|error| format!("Не удалось сохранить manual marker: {error}"))?;

        self.change_marker_by_id(self.connection.last_insert_rowid())
    }

    pub fn list_change_markers(
        &mut self,
        connection_id: i64,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
    ) -> Result<Vec<ChangeMarker>, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection id для marker history",
                "Invalid connection ID for marker history",
            )
            .into());
        }

        let from_ms = from_ms.unwrap_or(0);
        let to_ms = to_ms.unwrap_or(i64::MAX);
        if from_ms < 0 || to_ms <= 0 || from_ms > to_ms {
            return Err(crate::i18n::text(
                "Некорректный диапазон manual markers",
                "Invalid manual-marker range",
            )
            .into());
        }

        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT
                    id,
                    schema_version,
                    connection_id,
                    occurred_at_ms,
                    kind,
                    title,
                    note,
                    created_at_ms
                FROM change_markers
                WHERE connection_id = ?1
                  AND occurred_at_ms >= ?2
                  AND occurred_at_ms <= ?3
                ORDER BY occurred_at_ms ASC, id ASC
                "#,
            )
            .map_err(|error| format!("Не удалось подготовить чтение manual markers: {error}"))?;

        let rows = statement
            .query_map(params![connection_id, from_ms, to_ms], marker_from_row)
            .map_err(|error| format!("Не удалось прочитать manual markers: {error}"))?;

        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn delete_change_marker(
        &mut self,
        connection_id: i64,
        marker_id: i64,
    ) -> Result<(), String> {
        if connection_id <= 0 || marker_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection/marker id",
                "Invalid connection/marker ID",
            )
            .into());
        }

        let deleted = self
            .connection
            .execute(
                "DELETE FROM change_markers WHERE id = ?1 AND connection_id = ?2",
                params![marker_id, connection_id],
            )
            .map_err(|error| format!("Не удалось удалить manual marker #{marker_id}: {error}"))?;

        if deleted == 0 {
            return Err(format!("Manual marker #{marker_id} не найден"));
        }

        Ok(())
    }

    pub fn compare_what_changed(
        &mut self,
        connection_id: i64,
        baseline: WhatChangedPeriod,
        incident: WhatChangedPeriod,
    ) -> Result<WhatChangedComparison, String> {
        validate_periods(connection_id, &baseline, &incident)?;

        let baseline_pgss = self.pgss_period_aggregate(connection_id, &baseline)?;
        let incident_pgss = self.pgss_period_aggregate(connection_id, &incident)?;
        let settings = self.what_changed_settings(connection_id, &baseline, &incident)?;
        let maintenance =
            self.compare_what_changed_maintenance(connection_id, &baseline, &incident)?;
        let storage = self.compare_what_changed_storage(connection_id, &baseline, &incident)?;

        let target = reconcile_target(
            connection_id,
            baseline_pgss.target.as_ref(),
            incident_pgss.target.as_ref(),
            settings.baseline_snapshot.as_ref(),
            settings.incident_snapshot.as_ref(),
            &maintenance.targets,
            &storage.targets,
        )?;

        let pgss = compare_pgss_periods(baseline_pgss, incident_pgss, &incident);
        let markers =
            self.list_change_markers(connection_id, Some(baseline.from_ms), Some(incident.to_ms))?;
        let mut timeline = build_timeline(
            &baseline,
            &incident,
            &pgss,
            &settings,
            &maintenance.autovacuum,
            &maintenance.xid,
            &storage.storage,
            &storage.index_health,
            &markers,
        );
        timeline.sort_by_key(|event| event.occurred_at_ms);

        let mut limitations = Vec::new();
        limitations.extend(pgss.limitations.iter().cloned());
        limitations.extend(settings.limitations.iter().cloned());
        limitations.extend(maintenance.autovacuum.limitations.iter().cloned());
        limitations.extend(maintenance.xid.limitations.iter().cloned());
        limitations.extend(storage.storage.limitations.iter().cloned());
        limitations.extend(storage.storage.baseline_quality.limitations.iter().cloned());
        limitations.extend(storage.storage.incident_quality.limitations.iter().cloned());
        limitations.extend(storage.index_health.limitations.iter().cloned());
        limitations.push(
            crate::i18n::text("Время событий Timeline неоднородно: manual markers — заявленное время, Settings — момент обнаружения, агрегированные изменения — время измерения/граница периода, а не точный момент изменения.", "Timeline event times have different meanings: manual markers use the declared time, Settings use detection time, and aggregated changes use measurement time or period boundaries rather than the exact change time.").into(),
        );
        limitations.push(
            crate::i18n::text("Идентичность физического PostgreSQL-кластера (system_identifier) не хранится в этих snapshots; после restore/failover история требует новой базы отсчёта.", "These snapshots do not store the physical PostgreSQL cluster identity (system_identifier); after restore/failover, history requires a new baseline.").into(),
        );
        limitations.push(
            crate::i18n::text("What Changed? показывает временные изменения и associated evidence. Он не устанавливает причинность между settings, markers, query workload, table maintenance, XID/freeze, storage/WAL/checkpoint и index context.", "What Changed? shows changes over time and associated evidence. It does not establish causality between settings, markers, query workload, table maintenance, XID/freeze, storage/WAL/checkpoint, and index context.").into(),
        );

        Ok(WhatChangedComparison {
            generated_at_ms: unix_time_millis()?,
            connection_id,
            target,
            baseline,
            incident,
            pgss,
            settings,
            autovacuum: maintenance.autovacuum,
            xid: maintenance.xid,
            storage: storage.storage,
            index_health: storage.index_health,
            markers,
            timeline,
            limitations,
        })
    }

    fn change_marker_by_id(&mut self, marker_id: i64) -> Result<ChangeMarker, String> {
        self.connection
            .query_row(
                r#"
                SELECT
                    id,
                    schema_version,
                    connection_id,
                    occurred_at_ms,
                    kind,
                    title,
                    note,
                    created_at_ms
                FROM change_markers
                WHERE id = ?1
                "#,
                params![marker_id],
                marker_from_row,
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать manual marker #{marker_id}: {error}"))?
            .ok_or_else(|| format!("Manual marker #{marker_id} не найден"))
    }

    fn pgss_period_aggregate(
        &mut self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<PgssPeriodAggregate, String> {
        let (snapshots, skipped_versions) =
            self.load_pgss_period_snapshots(connection_id, period)?;
        let snapshot_count = snapshots.len();
        let target = snapshots.first().map(|snapshot| TargetIdentity {
            connection_name: snapshot.connection_name.clone(),
            host: snapshot.host.clone(),
            port: snapshot.port,
            database: snapshot.database.clone(),
            database_oid: snapshot.database_oid,
            username: snapshot.username.clone(),
            server_version: snapshot.server_version.clone(),
        });

        validate_pgss_period_target(&snapshots)?;

        let mut limitations = Vec::new();
        if skipped_versions > 0 {
            limitations.push(format!(
                "Пропущено PGSS snapshots с неподдерживаемой schema_version: {skipped_versions}."
            ));
        }
        let mut accumulators = HashMap::<StatementKey, StatementAccumulator>::new();
        let mut sample_coverages = Vec::new();

        for snapshot in &snapshots {
            let rows = sampled_statement_rows(&snapshot.snapshot);
            if let Some(capacity) = snapshot.snapshot.capacity.data.as_ref() {
                if capacity.current_entries > 0 {
                    sample_coverages
                        .push(rows.len() as f64 / capacity.current_entries as f64 * 100.0);
                }
            }

            for (key, row) in rows {
                let entry = accumulators.entry(key).or_default();
                if entry.sampled_snapshot_ids.is_empty() {
                    entry.role_name = row.role_name.clone();
                    entry.database = row.database.clone();
                    entry.first_observed_at_ms = snapshot.collection_started_at_ms;
                }
                entry.last_observed_at_ms = snapshot.collection_started_at_ms;
                entry.sampled_snapshot_ids.insert(snapshot.id);
            }
        }

        let mut safe_interval_count = 0usize;
        let mut unsafe_interval_count = 0usize;
        let mut safe_elapsed_seconds = 0.0f64;
        let mut unsafe_reasons = HashSet::<String>::new();

        for pair in snapshots.windows(2) {
            let previous = &pair[0];
            let current = &pair[1];
            let elapsed_seconds = (current.collection_started_at_ms
                - previous.collection_started_at_ms) as f64
                / 1000.0;

            let global = match safe_pgss_interval(previous, current, elapsed_seconds) {
                Ok(global) => global,
                Err(reason) => {
                    unsafe_interval_count += 1;
                    unsafe_reasons.insert(reason);
                    continue;
                }
            };

            safe_interval_count += 1;
            safe_elapsed_seconds += elapsed_seconds;

            let previous_rows = sampled_statement_rows(&previous.snapshot);
            let current_rows = sampled_statement_rows(&current.snapshot);

            for (key, current_row) in &current_rows {
                let Some(previous_row) = previous_rows.get(key) else {
                    continue;
                };
                let Some(delta) = statement_interval_delta(previous_row, current_row) else {
                    continue;
                };

                let entry = accumulators.entry(key.clone()).or_default();
                entry.safe_intervals += 1;
                entry.safe_elapsed_seconds += elapsed_seconds;
                entry.calls_delta = entry.calls_delta.saturating_add(delta.calls);
                entry.plans_delta = entry.plans_delta.saturating_add(delta.plans);
                entry.total_exec_time_delta_ms += delta.total_exec_time_ms;
                entry.total_plan_time_delta_ms += delta.total_plan_time_ms;
                entry.rows_delta = entry.rows_delta.saturating_add(delta.rows);
                entry.shared_reads_delta =
                    entry.shared_reads_delta.saturating_add(delta.shared_reads);
                entry.shared_hits_delta = entry.shared_hits_delta.saturating_add(delta.shared_hits);
                entry.temp_reads_delta = entry.temp_reads_delta.saturating_add(delta.temp_reads);
                entry.temp_writes_delta = entry.temp_writes_delta.saturating_add(delta.temp_writes);
                entry.wal_bytes_delta += delta.wal_bytes;
                if let Some(calls) = global.total_calls_delta {
                    entry.denominator_calls_delta =
                        entry.denominator_calls_delta.saturating_add(calls);
                }
                if let Some(exec) = global.total_exec_time_delta_ms {
                    entry.denominator_exec_time_delta_ms += exec;
                }
            }
        }

        for reason in unsafe_reasons {
            limitations.push(reason);
        }

        if snapshot_count < 2 {
            limitations.push(
                crate::i18n::text("Для периода нужно минимум два PGSS snapshots, иначе interval deltas не рассчитываются.", "A period requires at least two PGSS snapshots; otherwise interval deltas cannot be calculated.").into(),
            );
        }
        if safe_interval_count == 0 && snapshot_count >= 2 {
            limitations.push(
                crate::i18n::text("В периоде нет safe PGSS intervals после проверки stats_reset/dealloc/time continuity.", "No safe PGSS intervals remain in this period after stats_reset/dealloc/time-continuity checks.").into(),
            );
        }
        limitations.push(
            crate::i18n::text("PGSS period comparison использует bounded sampled statements. Отсутствие queryid в sample не доказывает отсутствие query в pg_stat_statements или workload.", "PGSS period comparison uses a bounded sample of statements. A queryid missing from the sample does not prove that the query is absent from pg_stat_statements or the workload.").into(),
        );

        let observed_span_seconds = snapshots
            .first()
            .zip(snapshots.last())
            .map(|(first, last)| {
                (last.collection_started_at_ms - first.collection_started_at_ms) as f64 / 1000.0
            })
            .filter(|seconds| *seconds > 0.0);
        let safe_time_coverage_pct = observed_span_seconds
            .filter(|seconds| *seconds > 0.0)
            .map(|seconds| safe_elapsed_seconds / seconds * 100.0);
        let min_sample_coverage_pct = sample_coverages.iter().copied().reduce(f64::min);
        let max_sample_coverage_pct = sample_coverages.iter().copied().reduce(f64::max);
        let state = if safe_interval_count == 0 {
            "insufficient"
        } else if unsafe_interval_count > 0 || skipped_versions > 0 {
            "limited"
        } else {
            "usable"
        }
        .to_owned();

        let statements = accumulators
            .into_iter()
            .map(|(key, accumulator)| {
                let metrics = accumulator_to_metrics(&key, accumulator, safe_elapsed_seconds);
                (key, metrics)
            })
            .collect();

        Ok(PgssPeriodAggregate {
            target,
            quality: PgssPeriodQuality {
                state,
                snapshot_count,
                snapshot_ids: snapshots.iter().map(|snapshot| snapshot.id).collect(),
                safe_interval_count,
                unsafe_interval_count,
                observed_span_seconds,
                safe_elapsed_seconds,
                safe_time_coverage_pct,
                min_sample_coverage_pct,
                max_sample_coverage_pct,
                limitations,
            },
            statements,
        })
    }

    fn load_pgss_period_snapshots(
        &mut self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<(Vec<StoredPgssPeriodSnapshot>, usize), String> {
        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT
                    id,
                    schema_version,
                    connection_name,
                    host,
                    port,
                    database_name,
                    database_oid,
                    username,
                    server_version,
                    collection_started_at_ms,
                    stats_reset,
                    dealloc,
                    snapshot_json
                FROM pgss_snapshots
                WHERE connection_id = ?1
                  AND collection_started_at_ms >= ?2
                  AND collection_started_at_ms <= ?3
                ORDER BY collection_started_at_ms ASC, id ASC
                "#,
            )
            .map_err(|error| format!("Не удалось подготовить PGSS period history: {error}"))?;
        let mut rows = statement
            .query(params![connection_id, period.from_ms, period.to_ms])
            .map_err(|error| format!("Не удалось прочитать PGSS period history: {error}"))?;
        let mut result = Vec::new();
        let mut skipped_versions = 0usize;

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("Не удалось прочитать PGSS period row: {error}"))?
        {
            let id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let schema_version: i64 = row.get(1).map_err(|error| error.to_string())?;
            if schema_version != 1 {
                skipped_versions += 1;
                continue;
            }
            let snapshot_json: String = row.get(12).map_err(|error| error.to_string())?;
            let snapshot =
                serde_json::from_str::<PgssSnapshot>(&snapshot_json).map_err(|error| {
                    format!(
                        "Не удалось разобрать PGSS snapshot #{id} для period comparison: {error}"
                    )
                })?;

            result.push(StoredPgssPeriodSnapshot {
                id,
                connection_name: row.get(2).map_err(|error| error.to_string())?,
                host: row.get(3).map_err(|error| error.to_string())?,
                port: row.get(4).map_err(|error| error.to_string())?,
                database: row.get(5).map_err(|error| error.to_string())?,
                database_oid: row.get(6).map_err(|error| error.to_string())?,
                username: row.get(7).map_err(|error| error.to_string())?,
                server_version: row.get(8).map_err(|error| error.to_string())?,
                collection_started_at_ms: row.get(9).map_err(|error| error.to_string())?,
                stats_reset: row.get(10).map_err(|error| error.to_string())?,
                dealloc: row.get(11).map_err(|error| error.to_string())?,
                snapshot,
            });
        }

        Ok((result, skipped_versions))
    }

    fn what_changed_settings(
        &mut self,
        connection_id: i64,
        baseline: &WhatChangedPeriod,
        incident: &WhatChangedPeriod,
    ) -> Result<WhatChangedSettingsComparison, String> {
        let baseline_id = self.latest_settings_snapshot_id_in_period(connection_id, baseline)?;
        let incident_id = self.latest_settings_snapshot_id_in_period(connection_id, incident)?;
        let mut limitations = Vec::new();

        let baseline_snapshot = baseline_id
            .map(|id| self.pg_settings_summary_by_id(id))
            .transpose()?;
        let incident_snapshot = incident_id
            .map(|id| self.pg_settings_summary_by_id(id))
            .transpose()?;
        let comparison = match (baseline_id, incident_id) {
            (Some(baseline_id), Some(incident_id)) if baseline_id != incident_id => {
                Some(self.compare_pg_settings_snapshots(baseline_id, incident_id)?)
            }
            (Some(_), Some(_)) => {
                limitations.push(
                    crate::i18n::text("Baseline и incident выбрали один pg_settings snapshot; state comparison не рассчитывается.", "Baseline and incident selected the same pg_settings snapshot; no state comparison was calculated.").into(),
                );
                None
            }
            (baseline_id, incident_id) => {
                if baseline_id.is_none() {
                    limitations.push(
                        crate::i18n::text(
                            "В baseline period нет pg_settings snapshot.",
                            "There is no pg_settings snapshot in the baseline period.",
                        )
                        .into(),
                    );
                }
                if incident_id.is_none() {
                    limitations.push(
                        crate::i18n::text(
                            "В incident period нет pg_settings snapshot.",
                            "There is no pg_settings snapshot in the incident period.",
                        )
                        .into(),
                    );
                }
                None
            }
        };

        if let (Some(older), Some(newer)) = (&baseline_snapshot, &incident_snapshot) {
            if older.postmaster_start_ms != newer.postmaster_start_ms {
                limitations.push(
                    crate::i18n::text("pg_postmaster_start_time отличается между selected settings snapshots; между periods был restart PostgreSQL.", "pg_postmaster_start_time differs between the selected settings snapshots; PostgreSQL restarted between periods.").into(),
                );
            }
        }

        let (observed_events, event_limitations) =
            self.settings_events_in_scope(connection_id, baseline.from_ms, incident.to_ms)?;
        let events_complete = event_limitations.is_empty();
        limitations.extend(event_limitations);
        let baseline_anchor_ms = baseline_snapshot
            .as_ref()
            .map(|snapshot| snapshot.collection_completed_at_ms);
        let changes = comparison
            .map(|comparison| {
                settings_changes_with_events(comparison, &observed_events, baseline_anchor_ms)
            })
            .unwrap_or_default();
        let state = if baseline_snapshot.is_some() && incident_snapshot.is_some() {
            if baseline_id != incident_id && events_complete {
                "usable"
            } else {
                "limited"
            }
        } else {
            "insufficient"
        }
        .to_owned();

        Ok(WhatChangedSettingsComparison {
            state,
            baseline_snapshot,
            incident_snapshot,
            changes,
            observed_events,
            limitations,
        })
    }

    fn latest_settings_snapshot_id_in_period(
        &self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<Option<i64>, String> {
        self.connection
            .query_row(
                r#"
                SELECT id
                FROM pg_settings_snapshots
                WHERE connection_id = ?1
                  AND collection_started_at_ms >= ?2
                  AND collection_started_at_ms <= ?3
                ORDER BY collection_started_at_ms DESC, id DESC
                LIMIT 1
                "#,
                params![connection_id, period.from_ms, period.to_ms],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Не удалось выбрать pg_settings snapshot периода: {error}"))
    }

    fn settings_events_in_scope(
        &mut self,
        connection_id: i64,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<(Vec<PgSettingsChangeEvent>, Vec<String>), String> {
        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT id, collection_started_at_ms
                FROM pg_settings_snapshots
                WHERE connection_id = ?1
                  AND collection_started_at_ms <= ?3
                  AND (
                        collection_started_at_ms >= ?2
                        OR id = (
                            SELECT id
                            FROM pg_settings_snapshots
                            WHERE connection_id = ?1
                              AND collection_started_at_ms < ?2
                            ORDER BY collection_started_at_ms DESC, id DESC
                            LIMIT 1
                        )
                  )
                ORDER BY collection_started_at_ms ASC, id ASC
                "#,
            )
            .map_err(|error| format!("Не удалось подготовить settings event scope: {error}"))?;
        let rows = statement
            .query_map(params![connection_id, from_ms, to_ms], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| format!("Не удалось прочитать settings event scope: {error}"))?;
        let snapshots = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Повреждён settings event scope: {error}"))?;
        drop(statement);

        let mut events = Vec::new();
        let mut limitations = Vec::new();
        for pair in snapshots.windows(2) {
            let older_id = pair[0].0;
            let newer_id = pair[1].0;
            let comparison = match self.compare_pg_settings_snapshots(older_id, newer_id) {
                Ok(comparison) => comparison,
                Err(error) => {
                    limitations.push(format!(
                        "Settings snapshots #{} и #{} не удалось сравнить: {error}",
                        older_id, newer_id
                    ));
                    continue;
                }
            };
            if comparison.newer.collection_completed_at_ms < from_ms
                || comparison.newer.collection_completed_at_ms > to_ms
            {
                continue;
            }
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
            }
        }
        events.sort_by_key(|event| event.detected_at_ms);
        Ok((events, limitations))
    }
}

#[derive(Clone, Copy)]
struct SafePgssGlobalDelta {
    total_calls_delta: Option<i64>,
    total_exec_time_delta_ms: Option<f64>,
}

#[derive(Clone, Copy)]
struct StatementIntervalDelta {
    calls: i64,
    plans: i64,
    total_exec_time_ms: f64,
    total_plan_time_ms: f64,
    rows: i64,
    shared_reads: i64,
    shared_hits: i64,
    temp_reads: i64,
    temp_writes: i64,
    wal_bytes: f64,
}

fn safe_pgss_interval(
    previous: &StoredPgssPeriodSnapshot,
    current: &StoredPgssPeriodSnapshot,
    elapsed_seconds: f64,
) -> Result<SafePgssGlobalDelta, String> {
    if elapsed_seconds <= 0.0 {
        return Err(
            crate::i18n::text("PGSS interval с неположительным elapsed time исключён из period comparison.", "A PGSS interval with nonpositive elapsed time was excluded from the period comparison.").into(),
        );
    }
    match (&previous.stats_reset, &current.stats_reset) {
        (Some(older), Some(newer)) if older == newer => {}
        _ => {
            return Err(
                crate::i18n::text("PGSS stats_reset изменился или недоступен: соответствующий interval исключён из cumulative deltas.", "PGSS stats_reset changed or is unavailable: the affected interval was excluded from cumulative deltas.").into(),
            )
        }
    }
    match (previous.dealloc, current.dealloc) {
        (Some(older), Some(newer)) if older == newer => {}
        (Some(_), Some(_)) => {
            return Err(
                crate::i18n::text("PGSS dealloc изменился: соответствующий interval исключён, потому что sampled entries могли быть deallocated/recreated.", "PGSS dealloc changed: the affected interval was excluded because sampled entries may have been deallocated and recreated.").into(),
            )
        }
        _ => {
            return Err(
                crate::i18n::text("PGSS dealloc недоступен: continuity interval не подтверждается.", "PGSS dealloc is unavailable: interval continuity cannot be confirmed.").into(),
            )
        }
    }

    let total_calls_delta = previous
        .snapshot
        .capacity
        .data
        .as_ref()
        .zip(current.snapshot.capacity.data.as_ref())
        .and_then(|(older, newer)| nonnegative_i64_delta(older.total_calls, newer.total_calls));
    let total_exec_time_delta_ms = previous
        .snapshot
        .capacity
        .data
        .as_ref()
        .and_then(|value| value.total_exec_time_ms)
        .zip(
            current
                .snapshot
                .capacity
                .data
                .as_ref()
                .and_then(|value| value.total_exec_time_ms),
        )
        .and_then(|(older, newer)| nonnegative_f64_delta(older, newer));

    Ok(SafePgssGlobalDelta {
        total_calls_delta,
        total_exec_time_delta_ms,
    })
}

fn statement_interval_delta(
    previous: &PgssPerformanceRow,
    current: &PgssPerformanceRow,
) -> Option<StatementIntervalDelta> {
    Some(StatementIntervalDelta {
        calls: nonnegative_i64_delta(previous.calls, current.calls)?,
        plans: nonnegative_i64_delta(previous.plans, current.plans)?,
        total_exec_time_ms: nonnegative_f64_delta(
            previous.total_exec_time_ms,
            current.total_exec_time_ms,
        )?,
        total_plan_time_ms: nonnegative_f64_delta(
            previous.total_plan_time_ms,
            current.total_plan_time_ms,
        )?,
        rows: nonnegative_i64_delta(previous.rows, current.rows)?,
        shared_reads: nonnegative_i64_delta(previous.shared_blks_read, current.shared_blks_read)?,
        shared_hits: nonnegative_i64_delta(previous.shared_blks_hit, current.shared_blks_hit)?,
        temp_reads: nonnegative_i64_delta(previous.temp_blks_read, current.temp_blks_read)?,
        temp_writes: nonnegative_i64_delta(previous.temp_blks_written, current.temp_blks_written)?,
        wal_bytes: nonnegative_f64_delta(previous.wal_bytes, current.wal_bytes)?,
    })
}

fn sampled_statement_rows(snapshot: &PgssSnapshot) -> HashMap<StatementKey, PgssPerformanceRow> {
    let Some(samples) = snapshot.performance.data.as_ref() else {
        return HashMap::new();
    };

    let mut rows = HashMap::new();
    if !samples.history_sample.is_empty() {
        for row in &samples.history_sample {
            insert_sampled_row(&mut rows, row);
        }
        return rows;
    }

    for group in [
        &samples.total_exec,
        &samples.mean_exec,
        &samples.temp_io,
        &samples.shared_reads,
        &samples.wal,
        &samples.variance,
        &samples.planning,
    ] {
        for row in group {
            insert_sampled_row(&mut rows, row);
        }
    }
    rows
}

fn insert_sampled_row(
    target: &mut HashMap<StatementKey, PgssPerformanceRow>,
    row: &PgssPerformanceRow,
) {
    let Some(queryid) = row.queryid.as_ref() else {
        return;
    };
    target
        .entry(StatementKey {
            userid: row.userid,
            dbid: row.dbid,
            queryid: queryid.clone(),
            toplevel: row.toplevel,
        })
        .or_insert_with(|| row.clone());
}

fn accumulator_to_metrics(
    key: &StatementKey,
    accumulator: StatementAccumulator,
    period_safe_elapsed_seconds: f64,
) -> PgssStatementPeriodMetrics {
    let calls = accumulator.calls_delta as f64;
    let mut sampled_snapshot_ids = accumulator
        .sampled_snapshot_ids
        .iter()
        .copied()
        .collect::<Vec<_>>();
    sampled_snapshot_ids.sort_unstable();
    PgssStatementPeriodMetrics {
        userid: key.userid,
        dbid: key.dbid,
        queryid: key.queryid.clone(),
        toplevel: key.toplevel,
        role_name: accumulator.role_name,
        database: accumulator.database,
        first_observed_at_ms: accumulator.first_observed_at_ms,
        last_observed_at_ms: accumulator.last_observed_at_ms,
        sampled_snapshots: sampled_snapshot_ids.len(),
        sampled_snapshot_ids,
        safe_intervals: accumulator.safe_intervals,
        safe_elapsed_seconds: accumulator.safe_elapsed_seconds,
        safe_time_coverage_pct: (period_safe_elapsed_seconds > 0.0)
            .then_some(accumulator.safe_elapsed_seconds / period_safe_elapsed_seconds * 100.0),
        calls_delta: accumulator.calls_delta,
        calls_per_hour: (accumulator.safe_elapsed_seconds > 0.0)
            .then_some(accumulator.calls_delta as f64 / accumulator.safe_elapsed_seconds * 3600.0),
        plans_delta: accumulator.plans_delta,
        total_exec_time_delta_ms: accumulator.total_exec_time_delta_ms,
        total_exec_time_per_hour_ms: (accumulator.safe_elapsed_seconds > 0.0).then_some(
            accumulator.total_exec_time_delta_ms / accumulator.safe_elapsed_seconds * 3600.0,
        ),
        exec_ms_per_call: (accumulator.calls_delta > 0)
            .then_some(accumulator.total_exec_time_delta_ms / calls),
        calls_workload_share_pct: (accumulator.denominator_calls_delta > 0).then_some(
            accumulator.calls_delta as f64 / accumulator.denominator_calls_delta as f64 * 100.0,
        ),
        exec_workload_share_pct: (accumulator.denominator_exec_time_delta_ms > 0.0).then_some(
            accumulator.total_exec_time_delta_ms / accumulator.denominator_exec_time_delta_ms
                * 100.0,
        ),
        rows_per_call: (accumulator.calls_delta > 0)
            .then_some(accumulator.rows_delta as f64 / calls),
        shared_reads_per_call: (accumulator.calls_delta > 0)
            .then_some(accumulator.shared_reads_delta as f64 / calls),
        shared_hits_per_call: (accumulator.calls_delta > 0)
            .then_some(accumulator.shared_hits_delta as f64 / calls),
        temp_reads_per_call: (accumulator.calls_delta > 0)
            .then_some(accumulator.temp_reads_delta as f64 / calls),
        temp_writes_per_call: (accumulator.calls_delta > 0)
            .then_some(accumulator.temp_writes_delta as f64 / calls),
        wal_bytes_per_call: (accumulator.calls_delta > 0)
            .then_some(accumulator.wal_bytes_delta / calls),
    }
}

fn compare_pgss_periods(
    baseline: PgssPeriodAggregate,
    incident: PgssPeriodAggregate,
    incident_period: &WhatChangedPeriod,
) -> WhatChangedPgssComparison {
    let mut keys = baseline
        .statements
        .keys()
        .chain(incident.statements.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    keys.sort_by(|left, right| left.queryid.cmp(&right.queryid));

    let mut changes = keys
        .into_iter()
        .map(|key| {
            let baseline_metrics = baseline.statements.get(&key).cloned();
            let incident_metrics = incident.statements.get(&key).cloned();
            let state = match (&baseline_metrics, &incident_metrics) {
                (Some(_), Some(_)) => "sampled_in_both",
                (None, Some(_)) => "sampled_in_incident_only",
                (Some(_), None) => "sampled_in_baseline_only",
                (None, None) => unreachable!(),
            }
            .to_owned();
            let role_name = incident_metrics
                .as_ref()
                .and_then(|value| value.role_name.clone())
                .or_else(|| {
                    baseline_metrics
                        .as_ref()
                        .and_then(|value| value.role_name.clone())
                });
            let database = incident_metrics
                .as_ref()
                .and_then(|value| value.database.clone())
                .or_else(|| {
                    baseline_metrics
                        .as_ref()
                        .and_then(|value| value.database.clone())
                });
            let confirmed = baseline_metrics
                .as_ref()
                .zip(incident_metrics.as_ref())
                .is_some_and(|(baseline, incident)| {
                    baseline.safe_intervals > 0 && incident.safe_intervals > 0
                });
            let metrics = statement_metric_changes(
                baseline_metrics.as_ref(),
                incident_metrics.as_ref(),
                confirmed,
            );
            let evidence_level = if confirmed
                && metrics
                    .iter()
                    .any(|metric| matches!(metric.direction.as_str(), "up" | "down"))
            {
                "confirmed_change"
            } else {
                "observed"
            }
            .to_owned();
            let observed_at_ms = incident_metrics
                .as_ref()
                .map(|value| value.last_observed_at_ms)
                .unwrap_or(incident_period.to_ms);

            WhatChangedStatementChange {
                userid: key.userid,
                dbid: key.dbid,
                queryid: key.queryid,
                toplevel: key.toplevel,
                role_name,
                database,
                state,
                evidence_level,
                observed_at_ms,
                baseline: baseline_metrics,
                incident: incident_metrics,
                metrics,
            }
        })
        .collect::<Vec<_>>();

    changes.sort_by(|left, right| {
        statement_change_score(right)
            .partial_cmp(&statement_change_score(left))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.queryid.cmp(&right.queryid))
    });
    changes.truncate(MAX_STATEMENT_CHANGES);

    let mut limitations = Vec::new();
    limitations.extend(baseline.quality.limitations.iter().cloned());
    limitations.extend(incident.quality.limitations.iter().cloned());
    limitations.push(
        crate::i18n::text("sampled_in_incident_only / sampled_in_baseline_only означает только отсутствие queryid в bounded sample второго периода, а не доказанное появление/исчезновение query.", "sampled_in_incident_only / sampled_in_baseline_only only means that queryid is missing from the other period’s bounded sample, not that the query appeared or disappeared.").into(),
    );
    limitations.push(
        crate::i18n::text("Confirmed change для PGSS означает сравнение aggregate safe intervals. Причина изменения latency/reads/temp/WAL из этих данных не устанавливается.", "A confirmed PGSS change means a comparison of aggregated safe intervals. These data cannot establish the cause of a latency/reads/temp/WAL change.").into(),
    );

    WhatChangedPgssComparison {
        baseline_quality: baseline.quality,
        incident_quality: incident.quality,
        statement_changes: changes,
        limitations,
    }
}

fn statement_metric_changes(
    baseline: Option<&PgssStatementPeriodMetrics>,
    incident: Option<&PgssStatementPeriodMetrics>,
    confirmed: bool,
) -> Vec<WhatChangedMetricChange> {
    let mut result = Vec::new();
    for (id, label, unit, baseline_value, incident_value) in [
        (
            "calls_per_hour",
            "Calls rate",
            "calls/hour",
            baseline.and_then(|value| value.calls_per_hour),
            incident.and_then(|value| value.calls_per_hour),
        ),
        (
            "exec_ms_per_call",
            "Interval execution",
            "ms/call",
            baseline.and_then(|value| value.exec_ms_per_call),
            incident.and_then(|value| value.exec_ms_per_call),
        ),
        (
            "total_exec_per_hour",
            "Execution workload",
            "ms/hour",
            baseline.and_then(|value| value.total_exec_time_per_hour_ms),
            incident.and_then(|value| value.total_exec_time_per_hour_ms),
        ),
        (
            "calls_share",
            "Calls workload share",
            "%",
            baseline.and_then(|value| value.calls_workload_share_pct),
            incident.and_then(|value| value.calls_workload_share_pct),
        ),
        (
            "exec_share",
            "Execution workload share",
            "%",
            baseline.and_then(|value| value.exec_workload_share_pct),
            incident.and_then(|value| value.exec_workload_share_pct),
        ),
        (
            "shared_reads_per_call",
            "Shared reads / call",
            "blocks/call",
            baseline.and_then(|value| value.shared_reads_per_call),
            incident.and_then(|value| value.shared_reads_per_call),
        ),
        (
            "temp_writes_per_call",
            "Temp written / call",
            "blocks/call",
            baseline.and_then(|value| value.temp_writes_per_call),
            incident.and_then(|value| value.temp_writes_per_call),
        ),
        (
            "temp_reads_per_call",
            "Temp read / call",
            "blocks/call",
            baseline.and_then(|value| value.temp_reads_per_call),
            incident.and_then(|value| value.temp_reads_per_call),
        ),
        (
            "wal_bytes_per_call",
            "WAL / call",
            "bytes/call",
            baseline.and_then(|value| value.wal_bytes_per_call),
            incident.and_then(|value| value.wal_bytes_per_call),
        ),
        (
            "rows_per_call",
            "Rows / call",
            "rows/call",
            baseline.and_then(|value| value.rows_per_call),
            incident.and_then(|value| value.rows_per_call),
        ),
    ] {
        let absolute_delta = baseline_value
            .zip(incident_value)
            .map(|(older, newer)| newer - older);
        let relative_change_pct = baseline_value
            .zip(incident_value)
            .and_then(|(older, newer)| {
                (older.abs() > f64::EPSILON).then_some((newer - older) / older.abs() * 100.0)
            });
        let direction = match absolute_delta {
            Some(delta) if delta > f64::EPSILON => "up",
            Some(delta) if delta < -f64::EPSILON => "down",
            Some(_) => "flat",
            None if baseline_value.is_none() && incident_value.is_some() => "appeared",
            None if baseline_value.is_some() && incident_value.is_none() => "not_sampled",
            None => "unknown",
        }
        .to_owned();

        let changed = confirmed && matches!(direction.as_str(), "up" | "down");
        result.push(WhatChangedMetricChange {
            id: id.to_owned(),
            label: label.to_owned(),
            unit: unit.to_owned(),
            baseline_value,
            incident_value,
            absolute_delta,
            relative_change_pct,
            direction,
            evidence_level: if changed {
                "confirmed_change"
            } else {
                "observed"
            }
            .to_owned(),
        });
    }
    result
}

fn settings_changes_with_events(
    comparison: PgSettingsComparison,
    events: &[PgSettingsChangeEvent],
    after_ms: Option<i64>,
) -> Vec<WhatChangedSettingChange> {
    comparison
        .changes
        .into_iter()
        .map(|change| {
            let first = events
                .iter()
                .filter(|event| event.change.name == change.name)
                .filter(|event| after_ms.is_none_or(|after_ms| event.detected_at_ms >= after_ms))
                .min_by_key(|event| event.detected_at_ms);
            WhatChangedSettingChange {
                change,
                first_observed_at_ms: first.map(|event| event.detected_at_ms),
                changed_after_ms: first.map(|event| event.changed_after_ms),
                changed_before_ms: first.map(|event| event.changed_before_ms),
                older_snapshot_id: first.map(|event| event.older_snapshot_id),
                newer_snapshot_id: first.map(|event| event.newer_snapshot_id),
            }
        })
        .collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "existing call contract; refactor separately with callers"
)]
fn build_timeline(
    baseline: &WhatChangedPeriod,
    incident: &WhatChangedPeriod,
    pgss: &WhatChangedPgssComparison,
    settings: &WhatChangedSettingsComparison,
    autovacuum: &WhatChangedAutovacuumComparison,
    xid: &WhatChangedXidComparison,
    storage: &WhatChangedStorageComparison,
    index_health: &WhatChangedIndexHealthComparison,
    markers: &[ChangeMarker],
) -> Vec<WhatChangedTimelineEvent> {
    let mut result = Vec::new();

    for marker in markers {
        result.push(WhatChangedTimelineEvent {
            occurred_at_ms: marker.occurred_at_ms,
            phase: timeline_phase(marker.occurred_at_ms, baseline, incident),
            source: "manual".into(),
            kind: marker.kind.clone(),
            title: marker.title.clone(),
            detail: marker.note.clone(),
            evidence_level: "observed".into(),
            evidence_refs: vec![format!("marker:{}", marker.id)],
        });
    }

    for event in &settings.observed_events {
        result.push(WhatChangedTimelineEvent {
            occurred_at_ms: event.detected_at_ms,
            phase: timeline_phase(event.detected_at_ms, baseline, incident),
            source: "pg_settings".into(),
            kind: "setting_change".into(),
            title: format!(
                "{}: {}",
                event.change.name,
                setting_change_summary(&event.change)
            ),
            detail: format_setting_change(&event.change),
            evidence_level: "confirmed_change".into(),
            evidence_refs: vec![
                format!("pg_settings_snapshot:{}", event.older_snapshot_id),
                format!("pg_settings_snapshot:{}", event.newer_snapshot_id),
            ],
        });
    }

    for change in pgss.statement_changes.iter().take(MAX_TIMELINE_PGSS_EVENTS) {
        let detail = change
            .metrics
            .iter()
            .filter(|metric| metric.baseline_value.is_some() && metric.incident_value.is_some())
            .take(3)
            .map(|metric| {
                format!(
                    "{}: {:.3} → {:.3} {}",
                    metric.label,
                    metric.baseline_value.unwrap_or_default(),
                    metric.incident_value.unwrap_or_default(),
                    metric.unit,
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let title = match change.state.as_str() {
            "sampled_in_incident_only" => {
                format!("queryid {} entered incident sample", change.queryid)
            }
            "sampled_in_baseline_only" => {
                format!("queryid {} not sampled in incident", change.queryid)
            }
            _ => format!("queryid {} changed between periods", change.queryid),
        };
        let mut evidence_refs = vec![format!(
            "pgss_statement:{}:{}:{}:{}",
            change.userid, change.dbid, change.queryid, change.toplevel,
        )];
        for snapshot_id in change
            .baseline
            .iter()
            .flat_map(|value| value.sampled_snapshot_ids.iter())
            .chain(
                change
                    .incident
                    .iter()
                    .flat_map(|value| value.sampled_snapshot_ids.iter()),
            )
            .take(8)
        {
            evidence_refs.push(format!("pgss_snapshot:{snapshot_id}"));
        }
        result.push(WhatChangedTimelineEvent {
            occurred_at_ms: change.observed_at_ms,
            phase: timeline_phase(change.observed_at_ms, baseline, incident),
            source: "pg_stat_statements".into(),
            kind: "statement_change".into(),
            title,
            detail,
            evidence_level: change.evidence_level.clone(),
            evidence_refs,
        });
    }

    for change in autovacuum.relation_changes.iter().take(12) {
        let Some(incident_metrics) = change.incident.as_ref() else {
            continue;
        };
        let baseline_metrics = change.baseline.as_ref();
        let mut details = Vec::new();
        if let Some(baseline_metrics) = baseline_metrics {
            if let (Some(before), Some(after)) = (
                baseline_metrics.updates_per_hour,
                incident_metrics.updates_per_hour,
            ) {
                if (before - after).abs() > f64::EPSILON {
                    details.push(format!("UPDATE rate: {before:.1}/h → {after:.1}/h"));
                }
            }
            if baseline_metrics.latest_dead_tuples != incident_metrics.latest_dead_tuples {
                details.push(format!(
                    "dead tuples: {} → {}",
                    baseline_metrics.latest_dead_tuples, incident_metrics.latest_dead_tuples,
                ));
            }
            if baseline_metrics.vacuum_cycles != incident_metrics.vacuum_cycles
                || baseline_metrics.analyze_cycles != incident_metrics.analyze_cycles
            {
                details.push(format!(
                    "maintenance cycles: VACUUM {} → {}; ANALYZE {} → {}",
                    baseline_metrics.vacuum_cycles,
                    incident_metrics.vacuum_cycles,
                    baseline_metrics.analyze_cycles,
                    incident_metrics.analyze_cycles,
                ));
            }
        }
        if change.policy_changed {
            details.push(format!("policy: {}", change.policy_changes.join("; ")));
        }
        if details.is_empty() {
            continue;
        }
        result.push(WhatChangedTimelineEvent {
            occurred_at_ms: change.observed_at_ms,
            phase: timeline_phase(change.observed_at_ms, baseline, incident),
            source: "autovacuum".into(),
            kind: "relation_maintenance_change".into(),
            title: format!("{} maintenance evidence changed", change.relation_name),
            detail: details.join("; "),
            evidence_level: change.evidence_level.clone(),
            evidence_refs: vec![format!("autovacuum_relation:{}", change.relation_oid)],
        });
    }

    for change in xid.relation_changes.iter().take(12) {
        let Some(incident_metrics) = change.incident.as_ref() else {
            continue;
        };
        let Some(baseline_metrics) = change.baseline.as_ref() else {
            continue;
        };
        let detail = format!(
            "XID age: {} → {}; freeze max usage: {} → {}",
            baseline_metrics.latest_xid_age,
            incident_metrics.latest_xid_age,
            format_optional_pct(baseline_metrics.latest_pct_of_freeze_max),
            format_optional_pct(incident_metrics.latest_pct_of_freeze_max),
        );
        result.push(WhatChangedTimelineEvent {
            occurred_at_ms: change.observed_at_ms,
            phase: timeline_phase(change.observed_at_ms, baseline, incident),
            source: "xid".into(),
            kind: "freeze_context_change".into(),
            title: format!("{} freeze context changed", change.relation_name),
            detail,
            evidence_level: change.evidence_level.clone(),
            evidence_refs: vec![format!("xid_relation:{}", change.relation_oid)],
        });
    }

    for change in storage.relation_size_changes.iter().take(12) {
        let Some(delta) = change.total_size_delta_bytes else {
            continue;
        };
        result.push(WhatChangedTimelineEvent {
            occurred_at_ms: incident.to_ms,
            phase: "incident".into(),
            source: "io_wal".into(),
            kind: "relation_size_change".into(),
            title: format!("{} size changed", change.relation_name),
            detail: format!("total size delta = {} bytes", delta),
            evidence_level: change.evidence_level.clone(),
            evidence_refs: vec![format!("relation:{}", change.relation_oid)],
        });
    }

    for change in index_health.changes.iter().take(8) {
        if change.baseline_classifications == change.incident_classifications
            && change.scan_delta.unwrap_or(0) == 0
            && change.relation_dml_delta.unwrap_or(0) == 0
        {
            continue;
        }
        result.push(WhatChangedTimelineEvent {
            occurred_at_ms: incident.to_ms,
            phase: "incident".into(),
            source: "index_health".into(),
            kind: "index_evidence_change".into(),
            title: format!("{} index context changed", change.index_name),
            detail: format!(
                "classifications: {:?} → {:?}; scan delta: {}; relation DML delta: {}",
                change.baseline_classifications,
                change.incident_classifications,
                change
                    .scan_delta
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into()),
                change
                    .relation_dml_delta
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into()),
            ),
            evidence_level: change.evidence_level.clone(),
            evidence_refs: vec![
                format!("index:{}", change.index_oid),
                format!("relation:{}", change.relation_oid),
            ],
        });
    }

    result
}

fn reconcile_target(
    connection_id: i64,
    baseline_pgss: Option<&TargetIdentity>,
    incident_pgss: Option<&TargetIdentity>,
    baseline_settings: Option<&PgSettingsSnapshotSummary>,
    incident_settings: Option<&PgSettingsSnapshotSummary>,
    maintenance_targets: &[MaintenanceTargetIdentity],
    storage_targets: &[StorageTargetIdentity],
) -> Result<WhatChangedTarget, String> {
    let mut targets = Vec::new();
    if let Some(target) = baseline_pgss {
        targets.push(target.clone());
    }
    if let Some(target) = incident_pgss {
        targets.push(target.clone());
    }
    for summary in [baseline_settings, incident_settings].into_iter().flatten() {
        targets.push(TargetIdentity {
            connection_name: summary.connection_name.clone(),
            host: summary.host.clone(),
            port: summary.port,
            database: summary.database.clone(),
            database_oid: summary.database_oid,
            username: summary.username.clone(),
            server_version: summary.server_version.clone(),
        });
    }
    for target in maintenance_targets {
        targets.push(TargetIdentity {
            connection_name: target.connection_name.clone(),
            host: target.host.clone(),
            port: target.port,
            database: target.database.clone(),
            database_oid: target.database_oid,
            username: target.username.clone(),
            server_version: target.server_version.clone(),
        });
    }
    for target in storage_targets {
        targets.push(TargetIdentity {
            connection_name: target.connection_name.clone(),
            host: target.host.clone(),
            port: target.port,
            database: target.database.clone(),
            database_oid: target.database_oid,
            username: target.username.clone(),
            server_version: target.server_version.clone(),
        });
    }

    let Some(first) = targets.first().cloned() else {
        return Err(
            crate::i18n::text("В выбранных periods нет PGSS, pg_settings, Autovacuum, XID, I/O & WAL или Index Health snapshots. Сначала соберите history.", "The selected periods contain no PGSS, pg_settings, Autovacuum, XID, I/O & WAL, or Index Health snapshots. Collect history first.").into(),
        );
    };
    for target in targets.iter().skip(1) {
        validate_target_identity(&first, target)?;
    }
    let port = u16::try_from(first.port)
        .map_err(|_| format!("Некорректный сохранённый PostgreSQL port: {}", first.port))?;

    Ok(WhatChangedTarget {
        connection_id,
        connection_name: first.connection_name,
        host: first.host,
        port,
        database: first.database,
        database_oid: first.database_oid,
        username: first.username,
        server_version: first.server_version,
    })
}

fn validate_pgss_period_target(snapshots: &[StoredPgssPeriodSnapshot]) -> Result<(), String> {
    let Some(first) = snapshots.first() else {
        return Ok(());
    };
    let first = TargetIdentity {
        connection_name: first.connection_name.clone(),
        host: first.host.clone(),
        port: first.port,
        database: first.database.clone(),
        database_oid: first.database_oid,
        username: first.username.clone(),
        server_version: first.server_version.clone(),
    };
    for snapshot in snapshots.iter().skip(1) {
        let current = TargetIdentity {
            connection_name: snapshot.connection_name.clone(),
            host: snapshot.host.clone(),
            port: snapshot.port,
            database: snapshot.database.clone(),
            database_oid: snapshot.database_oid,
            username: snapshot.username.clone(),
            server_version: snapshot.server_version.clone(),
        };
        validate_target_identity(&first, &current)?;
    }
    Ok(())
}

fn validate_target_identity(older: &TargetIdentity, newer: &TargetIdentity) -> Result<(), String> {
    if older.host != newer.host
        || older.port != newer.port
        || older.database_oid != newer.database_oid
        || older.username != newer.username
    {
        return Err(format!(
            "Selected periods содержат разные PostgreSQL targets/visibility contexts: {}:{}/oid {}/user {} vs {}:{}/oid {}/user {}",
            older.host,
            older.port,
            older.database_oid,
            older.username,
            newer.host,
            newer.port,
            newer.database_oid,
            newer.username,
        ));
    }
    Ok(())
}

fn validate_periods(
    connection_id: i64,
    baseline: &WhatChangedPeriod,
    incident: &WhatChangedPeriod,
) -> Result<(), String> {
    if connection_id <= 0 {
        return Err(crate::i18n::text(
            "Некорректный connection id для What Changed",
            "Invalid connection ID for What Changed",
        )
        .into());
    }
    for (label, period) in [("baseline", baseline), ("incident", incident)] {
        if period.from_ms <= 0 || period.to_ms <= 0 || period.from_ms >= period.to_ms {
            return Err(format!("Некорректный {label} period"));
        }
    }
    if baseline.to_ms > incident.from_ms {
        return Err(crate::i18n::text(
            "Baseline и incident periods не должны перекрываться",
            "Baseline and incident periods must not overlap",
        )
        .into());
    }
    Ok(())
}

fn marker_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeMarker> {
    Ok(ChangeMarker {
        id: row.get(0)?,
        schema_version: row.get(1)?,
        connection_id: row.get(2)?,
        occurred_at_ms: row.get(3)?,
        kind: row.get(4)?,
        title: row.get(5)?,
        note: row.get(6)?,
        created_at_ms: row.get(7)?,
    })
}

fn validate_marker_kind(value: &str) -> Result<String, String> {
    let value = value.trim();
    match value {
        "deploy" | "postgresql_config" | "index" | "query" | "application" | "maintenance"
        | "other" => Ok(value.to_owned()),
        _ => Err(format!("Неизвестный kind manual marker: {value}")),
    }
}

fn validate_marker_text(
    value: &str,
    label: &str,
    min_chars: usize,
    max_chars: usize,
) -> Result<String, String> {
    let value = value.trim();
    let count = value.chars().count();
    if count < min_chars || count > max_chars {
        return Err(format!(
            "{label} должен содержать от {min_chars} до {max_chars} символов"
        ));
    }
    Ok(value.to_owned())
}

fn nonnegative_i64_delta(older: i64, newer: i64) -> Option<i64> {
    (newer >= older).then_some(newer - older)
}

fn nonnegative_f64_delta(older: f64, newer: f64) -> Option<f64> {
    (older.is_finite() && newer.is_finite() && newer + 1e-9 >= older)
        .then_some((newer - older).max(0.0))
}

fn statement_change_score(change: &WhatChangedStatementChange) -> f64 {
    // Prioritize positive changes in execution impact, then use workload as a tie breaker.
    let regression = change
        .metrics
        .iter()
        .filter(|metric| {
            matches!(
                metric.id.as_str(),
                "exec_ms_per_call"
                    | "total_exec_per_hour"
                    | "shared_reads_per_call"
                    | "temp_writes_per_call"
                    | "wal_bytes_per_call"
            )
        })
        .filter(|metric| metric.direction == "up")
        .filter_map(|metric| metric.relative_change_pct)
        .filter(|value| value.is_finite() && *value > 0.0)
        .fold(0.0_f64, f64::max)
        .min(1_000_000.0);
    let workload = change
        .incident
        .as_ref()
        .or(change.baseline.as_ref())
        .and_then(|value| value.total_exec_time_per_hour_ms)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.0);
    regression * 1_000_000_000.0 + workload.min(999_999_999.0)
}

fn timeline_phase(
    timestamp_ms: i64,
    baseline: &WhatChangedPeriod,
    incident: &WhatChangedPeriod,
) -> String {
    if timestamp_ms >= baseline.from_ms && timestamp_ms <= baseline.to_ms {
        "baseline"
    } else if timestamp_ms >= incident.from_ms && timestamp_ms <= incident.to_ms {
        "incident"
    } else {
        "between"
    }
    .to_owned()
}

fn format_optional_pct(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "—".into())
}

fn setting_change_summary(change: &PgSettingChange) -> String {
    match change.state.as_str() {
        "added" => {
            return crate::i18n::text(
                "параметр появился в выборке",
                "the setting appeared in the sample",
            )
            .into()
        }
        "removed" => {
            return crate::i18n::text(
                "параметр исчез из выборки",
                "the setting disappeared from the sample",
            )
            .into()
        }
        _ => {}
    }
    let mut reasons = Vec::new();
    if change.value_changed {
        reasons.push(crate::i18n::text(
            "значение или единица измерения",
            "value or unit",
        ));
    }
    if change.reset_value_changed {
        reasons.push(crate::i18n::text("значение сброса", "reset value"));
    }
    if change.source_changed {
        reasons.push(crate::i18n::text(
            "источник настройки / файл / строка",
            "setting source / file / line",
        ));
    }
    if change.pending_restart_changed {
        reasons.push(crate::i18n::text(
            "признак перезапуска",
            "restart requirement",
        ));
    }
    if reasons.is_empty() {
        crate::i18n::text("изменены метаданные", "metadata changed").into()
    } else {
        if crate::i18n::is_ru() {
            format!("изменилось: {}", reasons.join(", "))
        } else {
            format!("changed: {}", reasons.join(", "))
        }
    }
}

fn format_setting_change(change: &PgSettingChange) -> String {
    let mut details = Vec::new();
    if change.value_changed || change.state != "changed" {
        details.push(format!(
            "value: {} ({}) → {} ({})",
            change.older_value.as_deref().unwrap_or("<missing>"),
            change
                .older_unit
                .as_deref()
                .unwrap_or(crate::i18n::text("без единицы", "no unit")),
            change.newer_value.as_deref().unwrap_or("<missing>"),
            change
                .newer_unit
                .as_deref()
                .unwrap_or(crate::i18n::text("без единицы", "no unit")),
        ));
    } else {
        details.push(
            crate::i18n::text(
                "значение параметра не менялось",
                "setting value did not change",
            )
            .into(),
        );
    }
    if change.reset_value_changed {
        details.push(format!(
            "reset_val: {} → {}",
            change.older_reset_value.as_deref().unwrap_or("<missing>"),
            change.newer_reset_value.as_deref().unwrap_or("<missing>"),
        ));
    }
    if change.source_changed {
        details.push(format!(
            "source: {} ({}:{}) → {} ({}:{})",
            change.older_source.as_deref().unwrap_or("<missing>"),
            change
                .older_sourcefile
                .as_deref()
                .unwrap_or(crate::i18n::text("нет файла", "no file")),
            change
                .older_sourceline
                .map_or("—".into(), |line| line.to_string()),
            change.newer_source.as_deref().unwrap_or("<missing>"),
            change
                .newer_sourcefile
                .as_deref()
                .unwrap_or(crate::i18n::text("нет файла", "no file")),
            change
                .newer_sourceline
                .map_or("—".into(), |line| line.to_string()),
        ));
    }
    if change.pending_restart_changed {
        details.push(format!(
            "pending_restart: {:?} → {:?}",
            change.older_pending_restart, change.newer_pending_restart,
        ));
    }
    details.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_timeline_explains_metadata_without_claiming_value_changed() {
        let mut change = PgSettingChange {
            name: "application_name".into(),
            category: "Client Connection Defaults".into(),
            state: "changed".into(),
            older_value: Some("pgxray-collector".into()),
            newer_value: Some("pgxray-collector".into()),
            older_unit: None,
            newer_unit: None,
            older_reset_value: Some("".into()),
            newer_reset_value: Some("".into()),
            older_source: Some("client".into()),
            newer_source: Some("client".into()),
            older_sourcefile: Some("/etc/old.conf".into()),
            newer_sourcefile: Some("/etc/new.conf".into()),
            older_sourceline: Some(3),
            newer_sourceline: Some(5),
            older_pending_restart: Some(false),
            newer_pending_restart: Some(false),
            value_changed: false,
            reset_value_changed: false,
            source_changed: true,
            pending_restart_changed: false,
        };
        assert!(setting_change_summary(&change).contains("источник настройки"));
        assert!(format_setting_change(&change).contains("значение параметра не менялось"));
        assert!(format_setting_change(&change).contains("/etc/new.conf:5"));
        change.source_changed = false;
        change.reset_value_changed = true;
        change.newer_reset_value = Some("new".into());
        assert!(setting_change_summary(&change).contains("значение сброса"));
        assert!(format_setting_change(&change).contains("reset_val"));
    }

    #[test]
    fn periods_must_not_overlap() {
        let error = validate_periods(
            1,
            &WhatChangedPeriod {
                from_ms: 100,
                to_ms: 300,
            },
            &WhatChangedPeriod {
                from_ms: 200,
                to_ms: 400,
            },
        )
        .unwrap_err();
        assert!(error.contains("не должны перекрываться"));
    }

    #[test]
    fn marker_kind_is_bounded() {
        assert_eq!(validate_marker_kind("deploy").unwrap(), "deploy");
        assert!(validate_marker_kind("anything").is_err());
    }

    #[test]
    fn cumulative_delta_rejects_counter_decrease() {
        assert_eq!(nonnegative_i64_delta(10, 12), Some(2));
        assert_eq!(nonnegative_i64_delta(12, 10), None);
        assert_eq!(nonnegative_f64_delta(10.0, 12.5), Some(2.5));
        assert_eq!(nonnegative_f64_delta(12.5, 10.0), None);
        assert_eq!(nonnegative_f64_delta(f64::NAN, 10.0), None);
        assert_eq!(nonnegative_f64_delta(10.0, f64::INFINITY), None);
    }

    #[test]
    fn manual_marker_round_trip() {
        let connection = Connection::open_in_memory().unwrap();
        init_schema(&connection).unwrap();
        let mut store = DiagnosticStore { connection };

        let marker = store
            .create_change_marker(7, 1_700_000_000_000, "deploy", "backend v2", "release")
            .unwrap();
        assert_eq!(marker.connection_id, 7);

        let markers = store
            .list_change_markers(7, Some(1_699_999_000_000), Some(1_700_001_000_000))
            .unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].title, "backend v2");

        assert!(store.delete_change_marker(999, marker.id).is_err());
        store.delete_change_marker(7, marker.id).unwrap();
        assert!(store.list_change_markers(7, None, None).unwrap().is_empty());
    }
}
