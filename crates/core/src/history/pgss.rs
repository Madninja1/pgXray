use super::{unix_time_millis, DiagnosticStore, DEFAULT_HISTORY_LIMIT, MAX_HISTORY_LIMIT};
use crate::{
    connections::SavedConnection,
    pgss::{
        PgssCallTargetCardinality, PgssCommandCardinality, PgssInterpretation,
        PgssInterpretationItem, PgssPerformanceRow, PgssRoleDatabaseCardinality, PgssSection,
        PgssSnapshot,
    },
};
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const PGSS_SNAPSHOT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
pub struct CollectedPgssSnapshot {
    pub history: PgssSnapshotSummary,
    pub snapshot: PgssSnapshot,
}

#[derive(Debug, Serialize)]
pub struct PgssSnapshotSummary {
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
    pub extension_installed: bool,
    pub extension_version: Option<String>,
    pub stats_reset: Option<String>,
    pub dealloc: Option<i64>,
    pub max_entries: Option<i64>,
    pub current_entries: Option<i64>,
    pub fill_pct: Option<f64>,
    pub one_call_entries: Option<i64>,
    pub up_to_5_calls_entries: Option<i64>,
    pub unavailable_sections: i64,
    pub stored_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct PgssRoleDatabaseDelta {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub older_entries: Option<i64>,
    pub newer_entries: Option<i64>,
    pub entries_delta: Option<i64>,
    pub older_one_call_entries: Option<i64>,
    pub newer_one_call_entries: Option<i64>,
    pub one_call_delta: Option<i64>,
    pub sample_state: String,
}

#[derive(Debug, Serialize)]
pub struct PgssCommandDelta {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub command: String,
    pub older_entries: Option<i64>,
    pub newer_entries: Option<i64>,
    pub entries_delta: Option<i64>,
    pub older_one_call_entries: Option<i64>,
    pub newer_one_call_entries: Option<i64>,
    pub one_call_delta: Option<i64>,
    pub sample_state: String,
}

#[derive(Debug, Serialize)]
pub struct PgssCallTargetDelta {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub call_tag: String,
    pub older_entries: Option<i64>,
    pub newer_entries: Option<i64>,
    pub entries_delta: Option<i64>,
    pub older_one_call_entries: Option<i64>,
    pub newer_one_call_entries: Option<i64>,
    pub one_call_delta: Option<i64>,
    pub sample_state: String,
}

#[derive(Debug, Serialize)]
pub struct PgssStatementHistory {
    pub anchor_snapshot_id: i64,
    pub connection_id: i64,
    pub userid: u32,
    pub dbid: u32,
    pub queryid: String,
    pub toplevel: bool,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub anchor_sampled_entries: usize,
    pub anchor_current_entries: Option<i64>,
    pub anchor_sample_coverage_pct: Option<f64>,
    pub anchor_sample_scope: String,
    pub summary: PgssStatementHistorySummary,
    pub change_analysis: PgssStatementChangeAnalysis,
    pub points: Vec<PgssStatementHistoryPoint>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PgssStatementHistorySummary {
    pub history_points: usize,
    pub history_span_seconds: Option<f64>,
    pub comparable_intervals: usize,
    pub latest_delta_state: String,
    pub latest_elapsed_seconds: Option<f64>,
    pub calls_delta: Option<i64>,
    pub calls_per_hour: Option<f64>,
    pub calls_workload_share_pct: Option<f64>,
    pub plans_delta: Option<i64>,
    pub total_exec_time_delta_ms: Option<f64>,
    pub interval_mean_exec_time_ms: Option<f64>,
    pub exec_workload_share_pct: Option<f64>,
    pub total_plan_time_delta_ms: Option<f64>,
    pub interval_mean_plan_time_ms: Option<f64>,
    pub rows_delta: Option<i64>,
    pub shared_blks_read_delta: Option<i64>,
    pub shared_blks_hit_delta: Option<i64>,
    pub temp_blks_read_delta: Option<i64>,
    pub temp_blks_written_delta: Option<i64>,
    pub wal_bytes_delta: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct PgssStatementHistoryPoint {
    pub snapshot_id: i64,
    pub collection_started_at_ms: i64,
    pub measured_at: String,
    pub stats_reset: Option<String>,
    pub dealloc: Option<i64>,
    pub sample_scope: String,
    pub row: PgssPerformanceRow,
    pub delta_state: String,
    pub elapsed_seconds: Option<f64>,
    pub calls_delta: Option<i64>,
    pub calls_per_hour: Option<f64>,
    pub calls_workload_share_pct: Option<f64>,
    pub plans_delta: Option<i64>,
    pub total_exec_time_delta_ms: Option<f64>,
    pub interval_mean_exec_time_ms: Option<f64>,
    pub exec_workload_share_pct: Option<f64>,
    pub total_plan_time_delta_ms: Option<f64>,
    pub interval_mean_plan_time_ms: Option<f64>,
    pub rows_delta: Option<i64>,
    pub shared_blks_read_delta: Option<i64>,
    pub shared_blks_hit_delta: Option<i64>,
    pub temp_blks_read_delta: Option<i64>,
    pub temp_blks_written_delta: Option<i64>,
    pub wal_bytes_delta: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct PgssStatementChangeAnalysis {
    pub state: String,
    pub older_from_snapshot_id: Option<i64>,
    pub older_to_snapshot_id: Option<i64>,
    pub newer_from_snapshot_id: Option<i64>,
    pub newer_to_snapshot_id: Option<i64>,
    pub items: Vec<PgssStatementMetricChange>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PgssStatementMetricChange {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub older_value: f64,
    pub newer_value: f64,
    pub absolute_delta: f64,
    pub relative_change_pct: Option<f64>,
    pub direction: String,
    pub evidence_level: String,
}

#[derive(Debug, Serialize)]
pub struct PgssComparison {
    pub older: PgssSnapshotSummary,
    pub newer: PgssSnapshotSummary,
    pub elapsed_seconds: f64,
    pub stats_period_continuous: bool,
    pub dealloc_delta: Option<i64>,
    pub dealloc_per_hour: Option<f64>,
    pub current_entries_delta: Option<i64>,
    pub fill_pct_delta: Option<f64>,
    pub one_call_entries_delta: Option<i64>,
    pub up_to_5_calls_entries_delta: Option<i64>,
    pub role_database_changes: Vec<PgssRoleDatabaseDelta>,
    pub command_changes: Vec<PgssCommandDelta>,
    pub call_target_changes: Vec<PgssCallTargetDelta>,
    pub interpretation: PgssInterpretation,
    pub limitations: Vec<String>,
}

struct StoredPgssSnapshot {
    summary: PgssSnapshotSummary,
    snapshot: PgssSnapshot,
}

struct PgssStatementSampleContext {
    snapshot_id: i64,
    collection_started_at_ms: i64,
    measured_at: String,
    stats_reset: Option<String>,
    dealloc: Option<i64>,
    sample_scope: String,
    global_total_calls: Option<i64>,
    global_total_exec_time_ms: Option<f64>,
    row: Option<PgssPerformanceRow>,
}

#[derive(Default)]
struct PgssStatementDelta {
    state: String,
    elapsed_seconds: Option<f64>,
    calls_delta: Option<i64>,
    calls_per_hour: Option<f64>,
    calls_workload_share_pct: Option<f64>,
    plans_delta: Option<i64>,
    total_exec_time_delta_ms: Option<f64>,
    interval_mean_exec_time_ms: Option<f64>,
    exec_workload_share_pct: Option<f64>,
    total_plan_time_delta_ms: Option<f64>,
    interval_mean_plan_time_ms: Option<f64>,
    rows_delta: Option<i64>,
    shared_blks_read_delta: Option<i64>,
    shared_blks_hit_delta: Option<i64>,
    temp_blks_read_delta: Option<i64>,
    temp_blks_written_delta: Option<i64>,
    wal_bytes_delta: Option<f64>,
}

/// A bounded historical PGSS analysis. Global rates originate from pg_stat_statements
/// capacity counters; statement rows originate from an incomplete sampled TOP set.
#[derive(Debug, Serialize)]
pub struct PgssWorkloadHistory {
    pub snapshot_count: usize,
    /// The snapshot that ends the interval used for the SQL ranking.
    pub selected_to_snapshot_id: Option<i64>,
    /// Only populated if an immediately preceding, safe interval exists.
    pub compared_to_snapshot_id: Option<i64>,
    pub intervals: Vec<PgssWorkloadInterval>,
    pub statements: Vec<PgssWorkloadStatement>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PgssWorkloadInterval {
    pub from_snapshot_id: i64,
    pub to_snapshot_id: i64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub elapsed_seconds: Option<f64>,
    pub state: String,
    pub sampled_entries: usize,
    pub comparable_statements: usize,
    /// All PGSS entries when capacity counters are available and continuous.
    pub global_calls_per_hour: Option<f64>,
    pub global_exec_ms_per_hour: Option<f64>,
    /// Only statements sampled in BOTH snapshots of this interval.
    pub sampled_wal_bytes_per_hour: Option<f64>,
    pub sampled_temp_blocks_per_hour: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PgssWorkloadMetrics {
    pub calls_per_hour: f64,
    pub exec_ms_per_hour: f64,
    pub mean_exec_ms: Option<f64>,
    pub shared_reads_per_hour: f64,
    pub temp_blocks_per_hour: f64,
    pub wal_bytes_per_hour: f64,
}

#[derive(Debug, Serialize)]
pub struct PgssWorkloadStatement {
    pub userid: u32,
    pub dbid: u32,
    pub queryid: String,
    pub toplevel: bool,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub latest: PgssWorkloadMetrics,
    pub previous: Option<PgssWorkloadMetrics>,
}

type WorkloadKey = (u32, u32, String, bool);

/// Select a safe interval and, if present, its immediately preceding safe
/// interval. Never skip over a reset or an unsampled interval to invent a
/// comparison between nonadjacent observations.
fn workload_interval_indices(
    intervals: &[PgssWorkloadInterval],
    anchor_snapshot_id: Option<i64>,
) -> Result<(Option<usize>, Option<usize>), String> {
    let selected = match anchor_snapshot_id {
        Some(anchor_id) => {
            let index = intervals
                .iter()
                .position(|row| row.to_snapshot_id == anchor_id)
                .ok_or_else(|| format!("PGSS interval ending at snapshot #{anchor_id} is not in the retained history"))?;
            if intervals[index].state != "comparable" {
                return Err(format!(
                    "PGSS interval ending at snapshot #{anchor_id} is not comparable"
                ));
            }
            Some(index)
        }
        None => intervals.iter().rposition(|row| row.state == "comparable"),
    };
    let previous = selected.and_then(|index| {
        if index > 0
            && intervals[index - 1].state == "comparable"
            && intervals[index - 1].to_snapshot_id == intervals[index].from_snapshot_id
        {
            Some(index - 1)
        } else {
            None
        }
    });
    Ok((selected, previous))
}

fn workload_sample_rows(snapshot: &PgssSnapshot) -> HashMap<WorkloadKey, &PgssPerformanceRow> {
    let Some(samples) = snapshot.performance.data.as_ref() else {
        return HashMap::new();
    };
    fn add<'a>(
        rows: &mut HashMap<WorkloadKey, &'a PgssPerformanceRow>,
        row: &'a PgssPerformanceRow,
    ) {
        if let Some(queryid) = row.queryid.as_ref() {
            rows.entry((row.userid, row.dbid, queryid.clone(), row.toplevel))
                .or_insert(row);
        }
    }
    let mut rows = HashMap::new();
    if !samples.history_sample.is_empty() {
        for row in &samples.history_sample {
            add(&mut rows, row);
        }
    } else {
        for row in samples
            .total_exec
            .iter()
            .chain(samples.mean_exec.iter())
            .chain(samples.temp_io.iter())
            .chain(samples.shared_reads.iter())
            .chain(samples.wal.iter())
            .chain(samples.variance.iter())
            .chain(samples.planning.iter())
        {
            add(&mut rows, row);
        }
    }
    rows
}

fn workload_global_rate(old: f64, new: f64, elapsed: f64) -> Option<f64> {
    (old.is_finite() && new.is_finite() && new >= old && elapsed > 0.0)
        .then_some((new - old) * 3600.0 / elapsed)
}

#[cfg(test)]
mod workload_tests {
    use super::workload_global_rate;

    fn interval(from: i64, to: i64, state: &str) -> super::PgssWorkloadInterval {
        super::PgssWorkloadInterval {
            from_snapshot_id: from,
            to_snapshot_id: to,
            from_ms: from * 1_000,
            to_ms: to * 1_000,
            elapsed_seconds: Some(1.0),
            state: state.into(),
            sampled_entries: 2,
            comparable_statements: 2,
            global_calls_per_hour: Some(10.0),
            global_exec_ms_per_hour: Some(1_000.0),
            sampled_wal_bytes_per_hour: Some(0.0),
            sampled_temp_blocks_per_hour: Some(0.0),
        }
    }

    #[test]
    fn workload_selection_never_compares_across_reset_or_unsampled_interval() {
        let intervals = [
            interval(1, 2, "comparable"),
            interval(2, 3, "dealloc_changed"),
            interval(3, 4, "comparable"),
            interval(4, 5, "comparable"),
        ];
        assert_eq!(
            super::workload_interval_indices(&intervals, None).unwrap(),
            (Some(3), Some(2))
        );
        assert_eq!(
            super::workload_interval_indices(&intervals, Some(4)).unwrap(),
            (Some(2), None)
        );
        assert_eq!(
            super::workload_interval_indices(&intervals, Some(2)).unwrap(),
            (Some(0), None)
        );
        assert!(super::workload_interval_indices(&intervals, Some(3)).is_err());
        assert!(super::workload_interval_indices(&intervals, Some(100)).is_err());
        assert_eq!(
            super::workload_interval_indices(&[], None).unwrap(),
            (None, None)
        );
    }

    #[test]
    fn rate_rejects_invalid_counter_or_interval() {
        assert_eq!(workload_global_rate(10.0, 20.0, 3600.0), Some(10.0));
        assert_eq!(workload_global_rate(20.0, 10.0, 3600.0), None);
        assert_eq!(workload_global_rate(10.0, 20.0, 0.0), None);
        assert_eq!(workload_global_rate(f64::NAN, 20.0, 3600.0), None);
        assert_eq!(workload_global_rate(10.0, f64::INFINITY, 3600.0), None);
    }
}

fn workload_interval_state(old: &StoredPgssSnapshot, new: &StoredPgssSnapshot) -> &'static str {
    if new.summary.collection_started_at_ms <= old.summary.collection_started_at_ms {
        return "invalid_interval";
    }
    if old.snapshot.performance.data.is_none() || new.snapshot.performance.data.is_none() {
        return "sample_unavailable";
    }
    if old.summary.stats_reset.is_none() || old.summary.stats_reset != new.summary.stats_reset {
        return "stats_reset_changed";
    }
    match (old.summary.dealloc, new.summary.dealloc) {
        (Some(a), Some(b)) if a == b => "comparable",
        (Some(_), Some(_)) => "dealloc_changed",
        _ => "dealloc_unknown",
    }
}

pub(super) fn init_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS pgss_snapshots (
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
                extension_installed INTEGER NOT NULL,
                extension_version TEXT,
                stats_reset TEXT,
                dealloc INTEGER,
                max_entries INTEGER,
                current_entries INTEGER,
                fill_pct REAL,
                one_call_entries INTEGER,
                up_to_5_calls_entries INTEGER,
                unavailable_sections INTEGER NOT NULL,
                stored_at_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_pgss_snapshots_connection_time
                ON pgss_snapshots (
                    connection_id,
                    collection_started_at_ms DESC,
                    id DESC
                );

            CREATE INDEX IF NOT EXISTS idx_pgss_snapshots_target_time
                ON pgss_snapshots (
                    host,
                    port,
                    database_oid,
                    collection_started_at_ms DESC
                );
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить PGSS history schema: {error}"))
}

impl DiagnosticStore {
    pub fn save_pgss_snapshot(
        &mut self,
        saved_connection: &SavedConnection,
        snapshot: &PgssSnapshot,
        collection_started_at_ms: i64,
        collection_completed_at_ms: i64,
    ) -> Result<PgssSnapshotSummary, String> {
        if collection_completed_at_ms < collection_started_at_ms {
            return Err(crate::i18n::text(
                "Время завершения PGSS snapshot раньше времени начала",
                "PGSS snapshot end time is earlier than its start time",
            )
            .into());
        }

        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|error| format!("Не удалось сериализовать PGSS snapshot: {error}"))?;

        let stats = snapshot.stats.data.as_ref();

        let capacity = snapshot.capacity.data.as_ref();

        let unavailable_sections = pgss_unavailable_section_count(snapshot);

        let stored_at_ms = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT INTO pgss_snapshots (
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
                    extension_installed,
                    extension_version,
                    stats_reset,
                    dealloc,
                    max_entries,
                    current_entries,
                    fill_pct,
                    one_call_entries,
                    up_to_5_calls_entries,
                    unavailable_sections,
                    stored_at_ms,
                    snapshot_json
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                    ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25,
                    ?26, ?27
                )
                "#,
                params![
                    PGSS_SNAPSHOT_SCHEMA_VERSION,
                    env!("CARGO_PKG_VERSION"),
                    saved_connection.id,
                    saved_connection.name,
                    saved_connection.host,
                    i64::from(saved_connection.port),
                    snapshot.overview.database,
                    i64::from(snapshot.overview.database_oid),
                    snapshot.overview.username,
                    saved_connection.sslmode,
                    snapshot.overview.server_version,
                    snapshot.overview.server_version_num,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                    snapshot.overview.measured_at,
                    snapshot.overview.extension_installed,
                    snapshot.overview.extension_version,
                    stats.and_then(|value| value.stats_reset.as_deref()),
                    stats.map(|value| { value.dealloc }),
                    capacity.map(|value| { value.max_entries }),
                    capacity.map(|value| { value.current_entries }),
                    capacity.map(|value| { value.fill_pct }),
                    capacity.map(|value| { value.one_call_entries }),
                    capacity.map(|value| { value.up_to_5_calls_entries }),
                    unavailable_sections,
                    stored_at_ms,
                    snapshot_json,
                ],
            )
            .map_err(|error| format!("Не удалось сохранить PGSS snapshot: {error}"))?;

        let id = self.connection.last_insert_rowid();

        Ok(PgssSnapshotSummary {
            id,
            schema_version: PGSS_SNAPSHOT_SCHEMA_VERSION,
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
            extension_installed: snapshot.overview.extension_installed,
            extension_version: snapshot.overview.extension_version.clone(),
            stats_reset: stats.and_then(|value| value.stats_reset.clone()),
            dealloc: stats.map(|value| value.dealloc),
            max_entries: capacity.map(|value| value.max_entries),
            current_entries: capacity.map(|value| value.current_entries),
            fill_pct: capacity.map(|value| value.fill_pct),
            one_call_entries: capacity.map(|value| value.one_call_entries),
            up_to_5_calls_entries: capacity.map(|value| value.up_to_5_calls_entries),
            unavailable_sections,
            stored_at_ms,
        })
    }

    pub fn list_pgss_snapshots(
        &mut self,
        connection_id: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<PgssSnapshotSummary>, String> {
        let limit = limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);

        let mut result = Vec::new();

        if let Some(connection_id) = connection_id {
            let mut statement = self
                .connection
                .prepare(&format!(
                    "{} WHERE connection_id = ?1 \
                             ORDER BY collection_started_at_ms DESC, id DESC \
                             LIMIT ?2",
                    pgss_summary_select_sql(),
                ))
                .map_err(|error| format!("Не удалось подготовить чтение PGSS history: {error}"))?;

            let mut rows = statement
                .query(params![connection_id, i64::from(limit),])
                .map_err(|error| format!("Не удалось прочитать PGSS history: {error}"))?;

            while let Some(row) = rows
                .next()
                .map_err(|error| format!("Не удалось прочитать строку PGSS history: {error}"))?
            {
                result.push(
                    pgss_summary_from_row(row)
                        .map_err(|error| format!("Повреждена строка PGSS history: {error}"))?,
                );
            }
        } else {
            let mut statement = self
                .connection
                .prepare(&format!(
                    "{} ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?1",
                    pgss_summary_select_sql(),
                ))
                .map_err(|error| format!("Не удалось подготовить чтение PGSS history: {error}"))?;

            let mut rows = statement
                .query(params![i64::from(limit),])
                .map_err(|error| format!("Не удалось прочитать PGSS history: {error}"))?;

            while let Some(row) = rows
                .next()
                .map_err(|error| format!("Не удалось прочитать строку PGSS history: {error}"))?
            {
                result.push(
                    pgss_summary_from_row(row)
                        .map_err(|error| format!("Повреждена строка PGSS history: {error}"))?,
                );
            }
        }

        Ok(result)
    }

    /// Analyse a bounded snapshot series without live SQL or guessed entries.
    /// The selected safe interval can only be compared against its immediately
    /// preceding safe neighbour, for statement keys present in both.
    pub fn pgss_workload_history(
        &mut self,
        connection_id: i64,
        limit: Option<u32>,
        anchor_snapshot_id: Option<i64>,
    ) -> Result<PgssWorkloadHistory, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text("Некорректное подключение", "Invalid connection").into());
        }
        let count = limit.unwrap_or(80).clamp(3, 120);
        let summaries = self.list_pgss_snapshots(Some(connection_id), Some(count))?;
        let mut limitations = vec![
            crate::i18n::text("Строки SQL — ограниченная TOP-выборка (до 300 queryid), не полный pg_stat_statements. Не попавший в выборку запрос не считается отсутствующим.", "SQL statements are a limited TOP sample (up to 300 queryids), not the complete pg_stat_statements view. Absence from the sample does not mean a query is absent.").into(),
            crate::i18n::text("Удельные rates и среднее время рассчитаны только по монотонным счётчикам без смены stats_reset/dealloc. SQL-текст не хранится в этих снимках.", "Per-unit rates and mean time are calculated only from monotonic counters without stats_reset/dealloc changes. SQL text is not stored in these snapshots.").into(),
            crate::i18n::text("Смена физического кластера PostgreSQL с тем же host/port/database_oid не обнаруживается без system_identifier.", "A physical PostgreSQL cluster change with the same host/port/database_oid cannot be detected without system_identifier.").into(),
        ];
        let mut loaded = Vec::new();
        if let Some(anchor) = summaries.first() {
            for summary in &summaries {
                if summary.host != anchor.host
                    || summary.port != anchor.port
                    || summary.database_oid != anchor.database_oid
                    || summary.username != anchor.username
                    || summary.sslmode != anchor.sslmode
                {
                    limitations.push(format!(
                        "История ограничена снимком #{}: изменился PostgreSQL target.",
                        summary.id
                    ));
                    break;
                }
                match self.load_pgss_snapshot(summary.id) {
                    Ok(snapshot) => loaded.push(snapshot),
                    Err(error) => {
                        limitations.push(format!(
                            "История остановлена на snapshot #{}: {error}",
                            summary.id
                        ));
                        break;
                    }
                }
            }
        }
        loaded.reverse();
        let mut intervals = Vec::new();
        let mut by_interval: Vec<HashMap<WorkloadKey, PgssWorkloadMetrics>> = Vec::new();
        for pair in loaded.windows(2) {
            let old = &pair[0];
            let new = &pair[1];
            let state = workload_interval_state(old, new);
            let elapsed = (new.summary.collection_started_at_ms
                - old.summary.collection_started_at_ms) as f64
                / 1000.0;
            let elapsed_seconds = (elapsed > 0.0).then_some(elapsed);
            let old_rows = workload_sample_rows(&old.snapshot);
            let new_rows = workload_sample_rows(&new.snapshot);
            let mut statement_metrics = HashMap::new();
            let mut wal_total = 0.0;
            let mut temp_total = 0.0;
            if state == "comparable" {
                for (key, latest) in &new_rows {
                    let Some(previous) = old_rows.get(key) else {
                        continue;
                    };
                    let old_context = PgssStatementSampleContext {
                        snapshot_id: old.summary.id,
                        collection_started_at_ms: old.summary.collection_started_at_ms,
                        measured_at: old.summary.measured_at.clone(),
                        stats_reset: old.summary.stats_reset.clone(),
                        dealloc: old.summary.dealloc,
                        sample_scope: "history_sample".into(),
                        global_total_calls: old
                            .snapshot
                            .capacity
                            .data
                            .as_ref()
                            .map(|c| c.total_calls),
                        global_total_exec_time_ms: old
                            .snapshot
                            .capacity
                            .data
                            .as_ref()
                            .and_then(|c| c.total_exec_time_ms),
                        row: Some((**previous).clone()),
                    };
                    let new_context = PgssStatementSampleContext {
                        snapshot_id: new.summary.id,
                        collection_started_at_ms: new.summary.collection_started_at_ms,
                        measured_at: new.summary.measured_at.clone(),
                        stats_reset: new.summary.stats_reset.clone(),
                        dealloc: new.summary.dealloc,
                        sample_scope: "history_sample".into(),
                        global_total_calls: new
                            .snapshot
                            .capacity
                            .data
                            .as_ref()
                            .map(|c| c.total_calls),
                        global_total_exec_time_ms: new
                            .snapshot
                            .capacity
                            .data
                            .as_ref()
                            .and_then(|c| c.total_exec_time_ms),
                        row: Some((**latest).clone()),
                    };
                    let delta = pgss_statement_delta(Some(&old_context), &new_context, latest);
                    if delta.state != "comparable" {
                        continue;
                    }
                    let metrics = PgssWorkloadMetrics {
                        calls_per_hour: delta.calls_per_hour.unwrap_or_default(),
                        exec_ms_per_hour: delta.total_exec_time_delta_ms.unwrap_or_default()
                            * 3600.0
                            / elapsed,
                        mean_exec_ms: delta.interval_mean_exec_time_ms,
                        shared_reads_per_hour: delta.shared_blks_read_delta.unwrap_or_default()
                            as f64
                            * 3600.0
                            / elapsed,
                        temp_blocks_per_hour: delta.temp_blks_written_delta.unwrap_or_default()
                            as f64
                            * 3600.0
                            / elapsed,
                        wal_bytes_per_hour: delta.wal_bytes_delta.unwrap_or_default() * 3600.0
                            / elapsed,
                    };
                    if ![
                        metrics.calls_per_hour,
                        metrics.exec_ms_per_hour,
                        metrics.shared_reads_per_hour,
                        metrics.temp_blocks_per_hour,
                        metrics.wal_bytes_per_hour,
                    ]
                    .iter()
                    .all(|v| v.is_finite())
                    {
                        continue;
                    }
                    wal_total += metrics.wal_bytes_per_hour;
                    temp_total += metrics.temp_blocks_per_hour;
                    statement_metrics.insert(key.clone(), metrics);
                }
            }
            let old_capacity = old.snapshot.capacity.data.as_ref();
            let new_capacity = new.snapshot.capacity.data.as_ref();
            let global_calls_per_hour = if state == "comparable" {
                old_capacity
                    .zip(new_capacity)
                    .and_then(|(a, b)| b.total_calls.checked_sub(a.total_calls))
                    .filter(|delta| *delta >= 0 && elapsed > 0.0)
                    .map(|delta| delta as f64 * 3600.0 / elapsed)
            } else {
                None
            };
            let global_exec_ms_per_hour = if state == "comparable" {
                old_capacity
                    .zip(new_capacity)
                    .and_then(|(a, b)| a.total_exec_time_ms.zip(b.total_exec_time_ms))
                    .and_then(|(a, b)| workload_global_rate(a, b, elapsed))
            } else {
                None
            };
            let comparable_statements = statement_metrics.len();
            intervals.push(PgssWorkloadInterval {
                from_snapshot_id: old.summary.id,
                to_snapshot_id: new.summary.id,
                from_ms: old.summary.collection_started_at_ms,
                to_ms: new.summary.collection_started_at_ms,
                elapsed_seconds,
                state: state.into(),
                sampled_entries: new_rows.len(),
                comparable_statements,
                global_calls_per_hour,
                global_exec_ms_per_hour,
                sampled_wal_bytes_per_hour: (comparable_statements > 0).then_some(wal_total),
                sampled_temp_blocks_per_hour: (comparable_statements > 0).then_some(temp_total),
            });
            by_interval.push(statement_metrics);
        }
        // A ranking must have an explicit, safe interval. Never silently reuse
        // results across a reset/deallocation or a gap in the captured sample.
        // An omitted anchor selects the latest *safe* interval, which may not
        // be the latest snapshot. The response tells the UI exactly which one.
        let (selected_index, previous_index) =
            workload_interval_indices(&intervals, anchor_snapshot_id)?;
        let compared_to_snapshot_id = previous_index.map(|index| intervals[index].to_snapshot_id);
        let mut statements = Vec::new();
        if let Some(index) = selected_index {
            let latest = &by_interval[index];
            let previous = previous_index.and_then(|i| by_interval.get(i));
            let last_snapshot = &loaded[index + 1];
            let last_rows = workload_sample_rows(&last_snapshot.snapshot);
            for (key, metrics) in latest {
                let row = last_rows.get(key);
                statements.push(PgssWorkloadStatement {
                    userid: key.0,
                    dbid: key.1,
                    queryid: key.2.clone(),
                    toplevel: key.3,
                    role_name: row.and_then(|r| r.role_name.clone()),
                    database: row.and_then(|r| r.database.clone()),
                    latest: metrics.clone(),
                    previous: previous.and_then(|items| items.get(key)).cloned(),
                });
            }
        }
        statements.sort_by(|a, b| {
            b.latest
                .exec_ms_per_hour
                .total_cmp(&a.latest.exec_ms_per_hour)
        });
        // No silent top cutoff: the bounded history sample already limits entries.
        if intervals.len() < 2 {
            limitations.push(
                "Для сравнения двух соседних интервалов нужны минимум три сохранённых снимка PGSS."
                    .into(),
            );
        }
        if intervals.iter().any(|i| i.state != "comparable") {
            limitations.push("Временные разрывы (reset, dealloc, недоступная секция) исключены из rate. Линии тренда не следует соединять через эти разрывы.".into());
        }
        Ok(PgssWorkloadHistory {
            snapshot_count: loaded.len(),
            selected_to_snapshot_id: selected_index.map(|index| intervals[index].to_snapshot_id),
            compared_to_snapshot_id,
            intervals,
            statements,
            limitations,
        })
    }

    pub fn pgss_statement_history(
        &mut self,
        anchor_snapshot_id: i64,
        userid: u32,
        dbid: u32,
        queryid: String,
        toplevel: bool,
        limit: Option<u32>,
    ) -> Result<PgssStatementHistory, String> {
        if anchor_snapshot_id <= 0 {
            return Err("Некорректный snapshot id для PGSS statement history".into());
        }

        let queryid = queryid
            .parse::<i64>()
            .map_err(|_| "Некорректный queryid для PGSS statement history".to_owned())?
            .to_string();
        let anchor = self.load_pgss_snapshot(anchor_snapshot_id)?;
        let anchor_sampled_entries = pgss_statement_sample_size(&anchor.snapshot);
        let anchor_sample_scope = pgss_statement_sample_scope(&anchor.snapshot);
        let anchor_current_entries = anchor
            .snapshot
            .capacity
            .data
            .as_ref()
            .map(|capacity| capacity.current_entries);
        let anchor_sample_coverage_pct = anchor_current_entries.and_then(|current_entries| {
            (current_entries > 0)
                .then_some(anchor_sampled_entries as f64 / current_entries as f64 * 100.0)
        });
        let anchor_row = pgss_performance_row(
            &anchor.snapshot,
            userid,
            dbid,
            &queryid,
            toplevel,
        )
        .cloned()
        .ok_or_else(|| {
            format!(
                "Statement queryid={queryid} отсутствует в sampled performance snapshot #{anchor_snapshot_id}"
            )
        })?;
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
                    dealloc,
                    snapshot_json
                FROM pgss_snapshots
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
            .map_err(|error| format!("Не удалось подготовить PGSS statement history: {error}"))?;

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
            .map_err(|error| format!("Не удалось прочитать PGSS statement history: {error}"))?;

        let mut contexts = Vec::new();
        let mut limitations = Vec::new();

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("Не удалось прочитать PGSS statement history row: {error}"))?
        {
            let snapshot_id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let schema_version: i64 = row.get(1).map_err(|error| error.to_string())?;
            let collection_started_at_ms: i64 = row.get(2).map_err(|error| error.to_string())?;
            let measured_at: String = row.get(3).map_err(|error| error.to_string())?;
            let stats_reset: Option<String> = row.get(4).map_err(|error| error.to_string())?;
            let dealloc: Option<i64> = row.get(5).map_err(|error| error.to_string())?;
            let snapshot_json: String = row.get(6).map_err(|error| error.to_string())?;

            if schema_version != PGSS_SNAPSHOT_SCHEMA_VERSION {
                limitations.push(format!(
                    "Snapshot #{snapshot_id} не используется для statement delta: schema_version={schema_version}, поддерживается {PGSS_SNAPSHOT_SCHEMA_VERSION}."
                ));
                contexts.push(PgssStatementSampleContext {
                    snapshot_id,
                    collection_started_at_ms,
                    measured_at,
                    stats_reset,
                    dealloc,
                    sample_scope: "unsupported_schema".into(),
                    global_total_calls: None,
                    global_total_exec_time_ms: None,
                    row: None,
                });
                continue;
            }

            let snapshot = serde_json::from_str::<PgssSnapshot>(&snapshot_json).map_err(|error| {
                format!(
                    "Не удалось разобрать PGSS snapshot #{snapshot_id} для statement history: {error}"
                )
            })?;
            let sampled_row =
                pgss_performance_row(&snapshot, userid, dbid, &queryid, toplevel).cloned();
            let sample_scope = pgss_statement_sample_scope(&snapshot);
            let global_total_calls = snapshot
                .capacity
                .data
                .as_ref()
                .map(|capacity| capacity.total_calls);
            let global_total_exec_time_ms = snapshot
                .capacity
                .data
                .as_ref()
                .and_then(|capacity| capacity.total_exec_time_ms);

            contexts.push(PgssStatementSampleContext {
                snapshot_id,
                collection_started_at_ms,
                measured_at,
                stats_reset,
                dealloc,
                sample_scope,
                global_total_calls,
                global_total_exec_time_ms,
                row: sampled_row,
            });
        }

        contexts.reverse();

        let mut points = Vec::new();
        for index in 0..contexts.len() {
            let current = &contexts[index];
            let Some(current_row) = current.row.as_ref() else {
                continue;
            };
            let previous = index
                .checked_sub(1)
                .map(|previous_index| &contexts[previous_index]);
            let delta = pgss_statement_delta(previous, current, current_row);

            points.push(PgssStatementHistoryPoint {
                snapshot_id: current.snapshot_id,
                collection_started_at_ms: current.collection_started_at_ms,
                measured_at: current.measured_at.clone(),
                stats_reset: current.stats_reset.clone(),
                dealloc: current.dealloc,
                sample_scope: current.sample_scope.clone(),
                row: current_row.clone(),
                delta_state: delta.state,
                elapsed_seconds: delta.elapsed_seconds,
                calls_delta: delta.calls_delta,
                calls_per_hour: delta.calls_per_hour,
                calls_workload_share_pct: delta.calls_workload_share_pct,
                plans_delta: delta.plans_delta,
                total_exec_time_delta_ms: delta.total_exec_time_delta_ms,
                interval_mean_exec_time_ms: delta.interval_mean_exec_time_ms,
                exec_workload_share_pct: delta.exec_workload_share_pct,
                total_plan_time_delta_ms: delta.total_plan_time_delta_ms,
                interval_mean_plan_time_ms: delta.interval_mean_plan_time_ms,
                rows_delta: delta.rows_delta,
                shared_blks_read_delta: delta.shared_blks_read_delta,
                shared_blks_hit_delta: delta.shared_blks_hit_delta,
                temp_blks_read_delta: delta.temp_blks_read_delta,
                temp_blks_written_delta: delta.temp_blks_written_delta,
                wal_bytes_delta: delta.wal_bytes_delta,
            });
        }

        if points.is_empty()
            || points.last().map(|point| point.snapshot_id) != Some(anchor_snapshot_id)
        {
            return Err(format!(
                "Anchor statement queryid={queryid} не найден в PGSS statement history"
            ));
        }

        let summary = pgss_statement_history_summary(&points);
        let change_analysis = pgss_statement_change_analysis(&points);

        if points.len() < 2 {
            limitations.push(
                "Для statement доступна только одна sampled point; временная delta пока не рассчитывается."
                    .into(),
            );
        }
        if points.iter().any(|point| point.delta_state == "sample_gap") {
            limitations.push(
                "Statement отсутствовал хотя бы в одном соседнем broad performance sample. Это не доказывает отсутствие entry в pg_stat_statements: snapshot хранит только union TOP-25 rankings."
                    .into(),
            );
        }
        if points
            .iter()
            .any(|point| point.delta_state == "dealloc_changed")
        {
            limitations.push(
                "dealloc изменился хотя бы на одном соседнем interval; pgXray не вычитает cumulative counters statement через такой interval, потому что entry мог быть deallocated и появиться заново."
                    .into(),
            );
        }
        if points
            .iter()
            .any(|point| point.delta_state == "stats_reset_changed")
        {
            limitations.push(
                "stats_reset изменился или недоступен хотя бы на одном interval; cumulative counters разных PGSS periods не сравниваются."
                    .into(),
            );
        }
        if points
            .iter()
            .any(|point| point.delta_state == "dealloc_unknown")
        {
            limitations.push(
                "dealloc недоступен хотя бы на одном соседнем interval; continuity statement не подтверждается и delta не рассчитывается."
                    .into(),
            );
        }
        if points
            .iter()
            .any(|point| point.delta_state == "counter_reset")
        {
            limitations.push(
                "Cumulative counter statement уменьшился хотя бы на одном interval. Такой interval не используется как confirmed delta."
                    .into(),
            );
        }
        if points
            .iter()
            .any(|point| point.sample_scope == "legacy_rankings")
        {
            limitations.push(
                "Часть старых snapshots не содержит расширенный history sample. Для них statement ищется только в legacy union TOP-25 rankings."
                    .into(),
            );
        }
        limitations.push(
            "Новые snapshots сохраняют bounded history sample: до 300 queryid из top-50 по нескольким performance dimensions. Это расширяет покрытие, но всё ещё не является полным pg_stat_statements."
                .into(),
        );
        limitations.push(
            "Query text не сохраняется в broad snapshot. SQL в Statement Investigation запрашивается live и может быть уже недоступен после deallocation/reset."
                .into(),
        );
        limitations.push(
            "mean/max/stddev в сохранённой row являются cumulative PGSS gauges. Для latest safe interval pgXray отдельно считает interval mean = total_exec_time delta / calls delta."
                .into(),
        );

        Ok(PgssStatementHistory {
            anchor_snapshot_id,
            connection_id: anchor.summary.connection_id,
            userid,
            dbid,
            queryid,
            toplevel,
            role_name: anchor_row.role_name,
            database: anchor_row.database,
            anchor_sampled_entries,
            anchor_current_entries,
            anchor_sample_coverage_pct,
            anchor_sample_scope,
            summary,
            change_analysis,
            points,
            limitations,
        })
    }

    pub fn compare_pgss_snapshots(
        &mut self,
        first_id: i64,
        second_id: i64,
    ) -> Result<PgssComparison, String> {
        if first_id == second_id {
            return Err("Для сравнения выберите два разных PGSS snapshot".into());
        }

        let first = self.load_pgss_snapshot(first_id)?;

        let second = self.load_pgss_snapshot(second_id)?;

        let (older, newer) = if pgss_snapshot_order_key(&first.summary)
            <= pgss_snapshot_order_key(&second.summary)
        {
            (first, second)
        } else {
            (second, first)
        };

        compare_pgss(older, newer)
    }

    /// Read an archived PGSS snapshot without querying PostgreSQL.
    /// Reject snapshots from another saved connection.
    pub fn pgss_snapshot(
        &mut self,
        connection_id: i64,
        snapshot_id: i64,
    ) -> Result<CollectedPgssSnapshot, String> {
        let stored = self.load_pgss_snapshot(snapshot_id)?;
        if stored.summary.connection_id != connection_id {
            return Err("PGSS snapshot принадлежит другому подключению".into());
        }
        Ok(CollectedPgssSnapshot {
            history: stored.summary,
            snapshot: stored.snapshot,
        })
    }

    fn load_pgss_snapshot(&mut self, id: i64) -> Result<StoredPgssSnapshot, String> {
        let sql = format!("{} WHERE id = ?1", pgss_stored_select_sql(),);

        self.connection
            .query_row(&sql, params![id], |row| {
                let summary = pgss_summary_from_row(row)?;

                let snapshot_json: String = row.get(27)?;

                Ok((summary, snapshot_json))
            })
            .map_err(|error| format!("PGSS snapshot #{id} не найден или повреждён: {error}"))
            .and_then(|(summary, snapshot_json)| {
                if summary.schema_version != PGSS_SNAPSHOT_SCHEMA_VERSION {
                    return Err(format!(
                        "PGSS snapshot #{} имеет schema_version={}, поддерживается {}",
                        summary.id, summary.schema_version, PGSS_SNAPSHOT_SCHEMA_VERSION,
                    ));
                }

                let snapshot =
                    serde_json::from_str::<PgssSnapshot>(&snapshot_json).map_err(|error| {
                        format!(
                            "Не удалось разобрать сохранённый PGSS snapshot #{}: {error}",
                            summary.id,
                        )
                    })?;

                Ok(StoredPgssSnapshot { summary, snapshot })
            })
    }
}

fn pgss_summary_select_sql() -> &'static str {
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
        extension_installed,
        extension_version,
        stats_reset,
        dealloc,
        max_entries,
        current_entries,
        fill_pct,
        one_call_entries,
        up_to_5_calls_entries,
        unavailable_sections,
        stored_at_ms
    FROM pgss_snapshots
    "#
}

fn pgss_stored_select_sql() -> &'static str {
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
        extension_installed,
        extension_version,
        stats_reset,
        dealloc,
        max_entries,
        current_entries,
        fill_pct,
        one_call_entries,
        up_to_5_calls_entries,
        unavailable_sections,
        stored_at_ms,
        snapshot_json
    FROM pgss_snapshots
    "#
}

fn pgss_summary_from_row(row: &Row<'_>) -> rusqlite::Result<PgssSnapshotSummary> {
    Ok(PgssSnapshotSummary {
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
        extension_installed: row.get(16)?,
        extension_version: row.get(17)?,
        stats_reset: row.get(18)?,
        dealloc: row.get(19)?,
        max_entries: row.get(20)?,
        current_entries: row.get(21)?,
        fill_pct: row.get(22)?,
        one_call_entries: row.get(23)?,
        up_to_5_calls_entries: row.get(24)?,
        unavailable_sections: row.get(25)?,
        stored_at_ms: row.get(26)?,
    })
}

fn pgss_unavailable_section_count(snapshot: &PgssSnapshot) -> i64 {
    [
        snapshot.stats.error.is_some(),
        snapshot.capacity.error.is_some(),
        snapshot.role_database.error.is_some(),
        snapshot.commands.error.is_some(),
        snapshot.call_targets.error.is_some(),
        snapshot.performance.error.is_some(),
    ]
    .into_iter()
    .filter(|unavailable| *unavailable)
    .count() as i64
}

fn pgss_performance_row<'a>(
    snapshot: &'a PgssSnapshot,
    userid: u32,
    dbid: u32,
    queryid: &str,
    toplevel: bool,
) -> Option<&'a PgssPerformanceRow> {
    let samples = snapshot.performance.data.as_ref()?;
    let matches = |row: &PgssPerformanceRow| {
        row.userid == userid
            && row.dbid == dbid
            && row.queryid.as_deref() == Some(queryid)
            && row.toplevel == toplevel
    };

    samples
        .history_sample
        .iter()
        .find(|row| matches(row))
        .or_else(|| {
            samples
                .total_exec
                .iter()
                .chain(samples.mean_exec.iter())
                .chain(samples.temp_io.iter())
                .chain(samples.shared_reads.iter())
                .chain(samples.wal.iter())
                .chain(samples.variance.iter())
                .chain(samples.planning.iter())
                .find(|row| matches(row))
        })
}

fn pgss_statement_sample_scope(snapshot: &PgssSnapshot) -> String {
    match snapshot.performance.data.as_ref() {
        Some(samples) if !samples.history_sample.is_empty() => "history_sample".into(),
        Some(_) => "legacy_rankings".into(),
        None => "unavailable".into(),
    }
}

fn pgss_statement_sample_size(snapshot: &PgssSnapshot) -> usize {
    let Some(samples) = snapshot.performance.data.as_ref() else {
        return 0;
    };

    if !samples.history_sample.is_empty() {
        return samples.history_sample.len();
    }

    let mut identities = HashSet::new();
    for row in samples
        .total_exec
        .iter()
        .chain(samples.mean_exec.iter())
        .chain(samples.temp_io.iter())
        .chain(samples.shared_reads.iter())
        .chain(samples.wal.iter())
        .chain(samples.variance.iter())
        .chain(samples.planning.iter())
    {
        if let Some(queryid) = row.queryid.as_deref() {
            identities.insert((row.userid, row.dbid, queryid.to_owned(), row.toplevel));
        }
    }

    identities.len()
}

fn pgss_statement_delta(
    previous: Option<&PgssStatementSampleContext>,
    current: &PgssStatementSampleContext,
    current_row: &PgssPerformanceRow,
) -> PgssStatementDelta {
    let Some(previous) = previous else {
        return PgssStatementDelta {
            state: "baseline".into(),
            ..PgssStatementDelta::default()
        };
    };
    let Some(previous_row) = previous.row.as_ref() else {
        return PgssStatementDelta {
            state: "sample_gap".into(),
            ..PgssStatementDelta::default()
        };
    };

    let elapsed_ms = current.collection_started_at_ms - previous.collection_started_at_ms;
    if elapsed_ms <= 0 {
        return PgssStatementDelta {
            state: "invalid_interval".into(),
            ..PgssStatementDelta::default()
        };
    }
    let elapsed_seconds = elapsed_ms as f64 / 1000.0;

    match (
        previous.stats_reset.as_deref(),
        current.stats_reset.as_deref(),
    ) {
        (Some(older), Some(newer)) if older == newer => {}
        _ => {
            return PgssStatementDelta {
                state: "stats_reset_changed".into(),
                elapsed_seconds: Some(elapsed_seconds),
                ..PgssStatementDelta::default()
            };
        }
    }

    match (previous.dealloc, current.dealloc) {
        (Some(older), Some(newer)) if older == newer => {}
        (Some(_), Some(_)) => {
            return PgssStatementDelta {
                state: "dealloc_changed".into(),
                elapsed_seconds: Some(elapsed_seconds),
                ..PgssStatementDelta::default()
            };
        }
        _ => {
            return PgssStatementDelta {
                state: "dealloc_unknown".into(),
                elapsed_seconds: Some(elapsed_seconds),
                ..PgssStatementDelta::default()
            };
        }
    }

    if !pgss_statement_counters_nondecreasing(previous_row, current_row) {
        return PgssStatementDelta {
            state: "counter_reset".into(),
            elapsed_seconds: Some(elapsed_seconds),
            ..PgssStatementDelta::default()
        };
    }

    let calls_delta = current_row.calls - previous_row.calls;
    let plans_delta = current_row.plans - previous_row.plans;
    let total_exec_time_delta_ms = current_row.total_exec_time_ms - previous_row.total_exec_time_ms;
    let total_plan_time_delta_ms = current_row.total_plan_time_ms - previous_row.total_plan_time_ms;
    let calls_per_hour = calls_delta as f64 / elapsed_seconds * 3600.0;
    let interval_mean_exec_time_ms =
        (calls_delta > 0).then_some(total_exec_time_delta_ms / calls_delta as f64);
    let interval_mean_plan_time_ms =
        (plans_delta > 0).then_some(total_plan_time_delta_ms / plans_delta as f64);
    let calls_workload_share_pct = match (previous.global_total_calls, current.global_total_calls) {
        (Some(older), Some(newer)) if newer > older && calls_delta >= 0 => {
            Some(calls_delta as f64 / (newer - older) as f64 * 100.0)
        }
        _ => None,
    };
    let exec_workload_share_pct = match (
        previous.global_total_exec_time_ms,
        current.global_total_exec_time_ms,
    ) {
        (Some(older), Some(newer)) if newer > older && total_exec_time_delta_ms >= 0.0 => {
            Some(total_exec_time_delta_ms / (newer - older) * 100.0)
        }
        _ => None,
    };

    PgssStatementDelta {
        state: "comparable".into(),
        elapsed_seconds: Some(elapsed_seconds),
        calls_delta: Some(calls_delta),
        calls_per_hour: Some(calls_per_hour),
        calls_workload_share_pct,
        plans_delta: Some(plans_delta),
        total_exec_time_delta_ms: Some(total_exec_time_delta_ms),
        interval_mean_exec_time_ms,
        exec_workload_share_pct,
        total_plan_time_delta_ms: Some(total_plan_time_delta_ms),
        interval_mean_plan_time_ms,
        rows_delta: Some(current_row.rows - previous_row.rows),
        shared_blks_read_delta: Some(current_row.shared_blks_read - previous_row.shared_blks_read),
        shared_blks_hit_delta: Some(current_row.shared_blks_hit - previous_row.shared_blks_hit),
        temp_blks_read_delta: Some(current_row.temp_blks_read - previous_row.temp_blks_read),
        temp_blks_written_delta: Some(
            current_row.temp_blks_written - previous_row.temp_blks_written,
        ),
        wal_bytes_delta: Some(current_row.wal_bytes - previous_row.wal_bytes),
    }
}

fn pgss_statement_counters_nondecreasing(
    older: &PgssPerformanceRow,
    newer: &PgssPerformanceRow,
) -> bool {
    newer.calls >= older.calls
        && newer.plans >= older.plans
        && nondecreasing_f64(older.total_plan_time_ms, newer.total_plan_time_ms)
        && nondecreasing_f64(older.total_exec_time_ms, newer.total_exec_time_ms)
        && newer.rows >= older.rows
        && newer.shared_blks_hit >= older.shared_blks_hit
        && newer.shared_blks_read >= older.shared_blks_read
        && newer.shared_blks_dirtied >= older.shared_blks_dirtied
        && newer.shared_blks_written >= older.shared_blks_written
        && newer.temp_blks_read >= older.temp_blks_read
        && newer.temp_blks_written >= older.temp_blks_written
        && nondecreasing_f64(older.blk_read_time_ms, newer.blk_read_time_ms)
        && nondecreasing_f64(older.blk_write_time_ms, newer.blk_write_time_ms)
        && newer.wal_records >= older.wal_records
        && newer.wal_fpi >= older.wal_fpi
        && nondecreasing_f64(older.wal_bytes, newer.wal_bytes)
}

fn nondecreasing_f64(older: f64, newer: f64) -> bool {
    newer + f64::EPSILON >= older
}

fn pgss_statement_history_summary(
    points: &[PgssStatementHistoryPoint],
) -> PgssStatementHistorySummary {
    let history_span_seconds = match (points.first(), points.last()) {
        (Some(first), Some(last)) if points.len() >= 2 => {
            let elapsed_ms = last.collection_started_at_ms - first.collection_started_at_ms;
            (elapsed_ms > 0).then_some(elapsed_ms as f64 / 1000.0)
        }
        _ => None,
    };
    let latest = points.last();

    PgssStatementHistorySummary {
        history_points: points.len(),
        history_span_seconds,
        comparable_intervals: points
            .iter()
            .filter(|point| point.delta_state == "comparable")
            .count(),
        latest_delta_state: latest
            .map(|point| point.delta_state.clone())
            .unwrap_or_else(|| "unavailable".into()),
        latest_elapsed_seconds: latest.and_then(|point| point.elapsed_seconds),
        calls_delta: latest.and_then(|point| point.calls_delta),
        calls_per_hour: latest.and_then(|point| point.calls_per_hour),
        calls_workload_share_pct: latest.and_then(|point| point.calls_workload_share_pct),
        plans_delta: latest.and_then(|point| point.plans_delta),
        total_exec_time_delta_ms: latest.and_then(|point| point.total_exec_time_delta_ms),
        interval_mean_exec_time_ms: latest.and_then(|point| point.interval_mean_exec_time_ms),
        exec_workload_share_pct: latest.and_then(|point| point.exec_workload_share_pct),
        total_plan_time_delta_ms: latest.and_then(|point| point.total_plan_time_delta_ms),
        interval_mean_plan_time_ms: latest.and_then(|point| point.interval_mean_plan_time_ms),
        rows_delta: latest.and_then(|point| point.rows_delta),
        shared_blks_read_delta: latest.and_then(|point| point.shared_blks_read_delta),
        shared_blks_hit_delta: latest.and_then(|point| point.shared_blks_hit_delta),
        temp_blks_read_delta: latest.and_then(|point| point.temp_blks_read_delta),
        temp_blks_written_delta: latest.and_then(|point| point.temp_blks_written_delta),
        wal_bytes_delta: latest.and_then(|point| point.wal_bytes_delta),
    }
}

fn pgss_statement_change_analysis(
    points: &[PgssStatementHistoryPoint],
) -> PgssStatementChangeAnalysis {
    let mut result = PgssStatementChangeAnalysis {
        state: "insufficient_history".into(),
        older_from_snapshot_id: None,
        older_to_snapshot_id: None,
        newer_from_snapshot_id: None,
        newer_to_snapshot_id: None,
        items: Vec::new(),
        limitations: vec![
            "Change Analysis сравнивает только два последних последовательных confirmed intervals. Для него нужны минимум три sampled points.".into(),
        ],
    };

    if points.len() < 3 {
        return result;
    }

    let older_from = &points[points.len() - 3];
    let older_to = &points[points.len() - 2];
    let newer_to = &points[points.len() - 1];

    if newer_to.delta_state != "comparable" {
        result.state = "latest_interval_not_comparable".into();
        result.limitations = vec![format!(
            "Последний interval заканчивается snapshot #{} и имеет state={}; current change analysis не строится.",
            newer_to.snapshot_id, newer_to.delta_state,
        )];
        return result;
    }

    if older_to.delta_state != "comparable" {
        result.state = "previous_interval_not_comparable".into();
        result.limitations = vec![format!(
            "Предыдущий interval заканчивается snapshot #{} и имеет state={}; два последовательных confirmed intervals отсутствуют.",
            older_to.snapshot_id, older_to.delta_state,
        )];
        return result;
    }

    result.state = "ready".into();
    result.older_from_snapshot_id = Some(older_from.snapshot_id);
    result.older_to_snapshot_id = Some(older_to.snapshot_id);
    result.newer_from_snapshot_id = Some(older_to.snapshot_id);
    result.newer_to_snapshot_id = Some(newer_to.snapshot_id);
    result.limitations = vec![
        "Confirmed change означает подтверждённую арифметическую разницу между двумя safe PGSS intervals, а не установленную причину изменения performance.".into(),
        "Per-call показатели нормализуют изменение workload volume, но не заменяют EXPLAIN, lock, I/O или table evidence.".into(),
    ];

    push_statement_metric_change(
        &mut result.items,
        "calls_rate",
        "Calls rate",
        "per_hour",
        older_to.calls_per_hour,
        newer_to.calls_per_hour,
    );
    push_statement_metric_change(
        &mut result.items,
        "interval_latency",
        "Interval execution",
        "ms_per_call",
        older_to.interval_mean_exec_time_ms,
        newer_to.interval_mean_exec_time_ms,
    );
    push_statement_metric_change(
        &mut result.items,
        "calls_share",
        "Calls workload share",
        "percent",
        older_to.calls_workload_share_pct,
        newer_to.calls_workload_share_pct,
    );
    push_statement_metric_change(
        &mut result.items,
        "exec_share",
        "Execution workload share",
        "percent",
        older_to.exec_workload_share_pct,
        newer_to.exec_workload_share_pct,
    );
    push_statement_metric_change(
        &mut result.items,
        "shared_reads_per_call",
        "Shared reads / call",
        "blocks_per_call",
        per_call(older_to.shared_blks_read_delta, older_to.calls_delta),
        per_call(newer_to.shared_blks_read_delta, newer_to.calls_delta),
    );
    push_statement_metric_change(
        &mut result.items,
        "temp_written_per_call",
        "Temp written / call",
        "blocks_per_call",
        per_call(older_to.temp_blks_written_delta, older_to.calls_delta),
        per_call(newer_to.temp_blks_written_delta, newer_to.calls_delta),
    );
    push_statement_metric_change(
        &mut result.items,
        "temp_read_per_call",
        "Temp read / call",
        "blocks_per_call",
        per_call(older_to.temp_blks_read_delta, older_to.calls_delta),
        per_call(newer_to.temp_blks_read_delta, newer_to.calls_delta),
    );
    push_statement_metric_change(
        &mut result.items,
        "wal_per_call",
        "WAL / call",
        "bytes_per_call",
        per_call_f64(older_to.wal_bytes_delta, older_to.calls_delta),
        per_call_f64(newer_to.wal_bytes_delta, newer_to.calls_delta),
    );
    push_statement_metric_change(
        &mut result.items,
        "rows_per_call",
        "Rows / call",
        "rows_per_call",
        per_call(older_to.rows_delta, older_to.calls_delta),
        per_call(newer_to.rows_delta, newer_to.calls_delta),
    );
    push_statement_metric_change(
        &mut result.items,
        "planning",
        "Planning / plan",
        "ms_per_plan",
        older_to.interval_mean_plan_time_ms,
        newer_to.interval_mean_plan_time_ms,
    );

    if result.items.is_empty() {
        result.state = "no_comparable_metrics".into();
        result.limitations.push(
            "Safe intervals есть, но для них не удалось получить две сопоставимые metric values."
                .into(),
        );
    }

    result
}

fn push_statement_metric_change(
    target: &mut Vec<PgssStatementMetricChange>,
    id: &str,
    label: &str,
    unit: &str,
    older: Option<f64>,
    newer: Option<f64>,
) {
    let (Some(older), Some(newer)) = (older, newer) else {
        return;
    };
    if !older.is_finite() || !newer.is_finite() {
        return;
    }

    let absolute_delta = newer - older;
    let tolerance = older.abs().max(newer.abs()).max(1.0) * 1e-9;
    let direction = if absolute_delta.abs() <= tolerance {
        "unchanged"
    } else if older.abs() <= tolerance && newer > tolerance {
        "appeared"
    } else if newer.abs() <= tolerance && older > tolerance {
        "disappeared"
    } else if absolute_delta > 0.0 {
        "increased"
    } else {
        "decreased"
    };
    let relative_change_pct =
        (older.abs() > tolerance).then_some(absolute_delta / older.abs() * 100.0);

    target.push(PgssStatementMetricChange {
        id: id.into(),
        label: label.into(),
        unit: unit.into(),
        older_value: older,
        newer_value: newer,
        absolute_delta,
        relative_change_pct,
        direction: direction.into(),
        evidence_level: if direction == "unchanged" {
            "observed".into()
        } else {
            "confirmed_change".into()
        },
    });
}

fn per_call(value: Option<i64>, calls: Option<i64>) -> Option<f64> {
    match (value, calls) {
        (Some(value), Some(calls)) if calls > 0 => Some(value as f64 / calls as f64),
        _ => None,
    }
}

fn per_call_f64(value: Option<f64>, calls: Option<i64>) -> Option<f64> {
    match (value, calls) {
        (Some(value), Some(calls)) if calls > 0 => Some(value / calls as f64),
        _ => None,
    }
}

fn compare_pgss(
    older: StoredPgssSnapshot,
    newer: StoredPgssSnapshot,
) -> Result<PgssComparison, String> {
    validate_same_pgss_target(&older.summary, &newer.summary)?;

    let elapsed_ms =
        newer.summary.collection_started_at_ms - older.summary.collection_started_at_ms;

    if elapsed_ms <= 0 {
        return Err("Между PGSS snapshots нет положительного временного интервала".into());
    }

    let elapsed_seconds = elapsed_ms as f64 / 1000.0;

    let mut limitations = Vec::new();

    let stats_period_continuous = match (
        older.summary.stats_reset.as_deref(),
        newer.summary.stats_reset.as_deref(),
    ) {
        (Some(older_reset), Some(newer_reset)) => older_reset == newer_reset,

        _ => false,
    };

    if !stats_period_continuous {
        limitations.push(
            "stats_reset отличается или недоступен хотя бы в одном snapshot; dealloc delta/rate не рассчитывается."
                .into(),
        );
    }

    let dealloc_delta = if stats_period_continuous {
        monotonic_delta(
            older.summary.dealloc,
            newer.summary.dealloc,
            "dealloc",
            &mut limitations,
        )
    } else {
        None
    };

    let dealloc_per_hour = dealloc_delta.map(|delta| delta as f64 / elapsed_seconds * 3600.0);

    let role_database_changes = compare_pgss_role_database(
        &older.snapshot.role_database,
        &newer.snapshot.role_database,
        &mut limitations,
    );

    let command_changes = compare_pgss_commands(
        &older.snapshot.commands,
        &newer.snapshot.commands,
        &mut limitations,
    );

    let call_target_changes = compare_pgss_call_targets(
        &older.snapshot.call_targets,
        &newer.snapshot.call_targets,
        &mut limitations,
    );

    if older.summary.extension_version != newer.summary.extension_version {
        limitations.push(
            format!(
                "Версия extension изменилась: {:?} → {:?}. Сравнение counters нужно интерпретировать осторожно.",
                older.summary.extension_version,
                newer.summary.extension_version,
            ),
        );
    }

    limitations.push(
        "current_entries/fill/one-call — gauges текущего содержимого pg_stat_statements, а не monotonic counters."
            .into(),
    );

    limitations.push(
        "Role/database, command и CALL-target comparison основан на агрегатах текущих entries. Deallocation между snapshots может менять состав sample."
            .into(),
    );

    limitations.push(
        "Двухточечный dealloc rate описывает только выбранный интервал и не является долгосрочным retention forecast."
            .into(),
    );

    let interpretation = pgss_comparison_interpretation(
        &older.summary,
        &newer.summary,
        elapsed_seconds,
        stats_period_continuous,
        dealloc_delta,
        dealloc_per_hour,
        &role_database_changes,
        &call_target_changes,
    );

    Ok(PgssComparison {
        current_entries_delta: optional_delta(
            older.summary.current_entries,
            newer.summary.current_entries,
        ),
        fill_pct_delta: optional_f64_delta(older.summary.fill_pct, newer.summary.fill_pct),
        one_call_entries_delta: optional_delta(
            older.summary.one_call_entries,
            newer.summary.one_call_entries,
        ),
        up_to_5_calls_entries_delta: optional_delta(
            older.summary.up_to_5_calls_entries,
            newer.summary.up_to_5_calls_entries,
        ),
        older: older.summary,
        newer: newer.summary,
        elapsed_seconds,
        stats_period_continuous,
        dealloc_delta,
        dealloc_per_hour,
        role_database_changes,
        command_changes,
        call_target_changes,
        interpretation,
        limitations,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "existing call contract; refactor separately with callers"
)]
fn pgss_comparison_interpretation(
    older: &PgssSnapshotSummary,
    newer: &PgssSnapshotSummary,
    elapsed_seconds: f64,
    stats_period_continuous: bool,
    dealloc_delta: Option<i64>,
    dealloc_per_hour: Option<f64>,
    role_changes: &[PgssRoleDatabaseDelta],
    call_changes: &[PgssCallTargetDelta],
) -> PgssInterpretation {
    let mut items = Vec::new();

    if !stats_period_continuous {
        items.push(
            pgss_comparison_item(
                "stats-reset-changed",
                "PGSS statistics period изменился",
                vec![
                    format!(
                        "stats_reset: {:?} → {:?}",
                        older.stats_reset,
                        newer.stats_reset,
                    ),
                ],
                vec![
                    "Cumulative dealloc принадлежит разным measurement periods, поэтому прямой delta/rate был бы некорректен."
                        .into(),
                ],
                vec![
                    "Использовать newer snapshot как новый baseline и сравнить со следующим snapshot при неизменном stats_reset."
                        .into(),
                ],
            ),
        );
    } else if let Some(delta) = dealloc_delta {
        let interval_minutes = elapsed_seconds / 60.0;

        if delta > 0 {
            let mut observed = vec![format!(
                "dealloc вырос на {delta} events за {:.1} min.",
                interval_minutes,
            )];

            if let Some(rate) = dealloc_per_hour {
                observed.push(format!(
                    "Измеренный rate = {:.2} dealloc events/hour.",
                    rate,
                ));
            }

            if let (Some(entries), Some(max_entries)) = (newer.current_entries, newer.max_entries) {
                observed.push(format!(
                    "Newer capacity = {entries} / {max_entries}; fill = {}.",
                    newer
                        .fill_pct
                        .map(|value| { format!("{value:.1}%") })
                        .unwrap_or_else(|| { "unknown".into() }),
                ));
            }

            if let (Some(one_call), Some(entries)) = (newer.one_call_entries, newer.current_entries)
            {
                if entries > 0 {
                    observed.push(format!(
                        "Newer one-shot share = {:.1}% ({} / {}).",
                        one_call as f64 / entries as f64 * 100.0,
                        one_call,
                        entries,
                    ));
                }
            }

            items.push(
                pgss_comparison_item(
                    "dealloc-growth",
                    "В интервале наблюдался PGSS churn",
                    observed,
                    vec![
                        "Рост dealloc при неизменном stats_reset означает, что pg_stat_statements выполнял deallocation least-executed entries в выбранном интервале."
                            .into(),
                        "Это observability/retention signal; он не доказывает performance-проблему SQL или нехватку RAM."
                            .into(),
                    ],
                    vec![
                        "Посмотреть, какая role/database группа и command увеличили cardinality."
                            .into(),
                        "Если доминирует CALL, проверить CALL target changes и track_utility."
                            .into(),
                        "После исправления сравнивать dealloc rate на representative workload window."
                            .into(),
                    ],
                ),
            );
        } else {
            items.push(
                pgss_comparison_item(
                    "no-dealloc-growth",
                    "В выбранном интервале dealloc не вырос",
                    vec![
                        format!(
                            "dealloc delta = 0 за {:.1} min.",
                            interval_minutes,
                        ),
                    ],
                    vec![
                        "В этом интервале collector не наблюдает новых deallocation events."
                            .into(),
                        "Это хороший факт для retention, но один короткий интервал не доказывает устойчивое отсутствие churn."
                            .into(),
                    ],
                    vec![
                        "Повторить comparison на representative workload window, если проблема была периодической."
                            .into(),
                    ],
                ),
            );
        }
    }

    if let Some(change) = role_changes
        .iter()
        .filter_map(|row| {
            row.entries_delta
                .filter(|delta| *delta > 0)
                .map(|delta| (delta, row))
        })
        .max_by_key(|(delta, _)| *delta)
    {
        let (delta, row) = change;

        items.push(
            pgss_comparison_item(
                "cardinality-growth-owner",
                "Наибольший рост cardinality в role/database sample",
                vec![
                    format!(
                        "role={} database={} entries: {} → {} (Δ +{}).",
                        row.role_name
                            .as_deref()
                            .unwrap_or(
                                "<unknown>",
                            ),
                        row.database
                            .as_deref()
                            .unwrap_or(
                                "<unknown>",
                            ),
                        row.older_entries
                            .map(|value| {
                                value.to_string()
                            })
                            .unwrap_or_else(|| {
                                "not in older sample"
                                    .into()
                            }),
                        row.newer_entries
                            .map(|value| {
                                value.to_string()
                            })
                            .unwrap_or_else(|| {
                                "not in newer sample"
                                    .into()
                            }),
                        delta,
                    ),
                ],
                vec![
                    "Эта group дала крупнейшее положительное изменение среди сохранённых role/database aggregate rows."
                        .into(),
                    "Это current-sample attribution, а не доказательство, что именно эта group создала все dealloc events."
                        .into(),
                ],
                vec![
                    "Посмотреть command changes для той же role/database."
                        .into(),
                ],
            ),
        );
    }

    if let Some(change) = call_changes
        .iter()
        .filter_map(|row| {
            row.entries_delta
                .filter(|delta| *delta > 0)
                .map(|delta| (delta, row))
        })
        .max_by_key(|(delta, _)| *delta)
    {
        let (delta, row) = change;

        items.push(
            pgss_comparison_item(
                "call-target-growth",
                "CALL target увеличил cardinality sample",
                vec![
                    format!(
                        "call_tag={} role={} database={} Δ entries = +{}.",
                        &row.call_tag[..row.call_tag.len().min(12)],
                        row.role_name
                            .as_deref()
                            .unwrap_or(
                                "<unknown>",
                            ),
                        row.database
                            .as_deref()
                            .unwrap_or(
                                "<unknown>",
                            ),
                        delta,
                    ),
                ],
                vec![
                    "Рост одного CALL target согласуется с high-cardinality utility workload, особенно если одновременно растёт dealloc."
                        .into(),
                    "Comparison не определяет client binding mode."
                        .into(),
                ],
                vec![
                    "Проверить driver и способ формирования CALL на стороне приложения."
                        .into(),
                ],
            ),
        );
    }

    PgssInterpretation {
        items,
        limitations: Vec::new(),
    }
}

fn pgss_comparison_item(
    id: &str,
    title: &str,
    observed: Vec<String>,
    meaning: Vec<String>,
    next_steps: Vec<String>,
) -> PgssInterpretationItem {
    PgssInterpretationItem {
        id: id.to_owned(),
        title: title.to_owned(),
        observed,
        meaning,
        next_steps,
    }
}

fn validate_same_pgss_target(
    older: &PgssSnapshotSummary,
    newer: &PgssSnapshotSummary,
) -> Result<(), String> {
    if older.host != newer.host
        || older.port != newer.port
        || older.database_oid != newer.database_oid
    {
        return Err(
            format!(
                "Нельзя сравнить PGSS snapshots разных targets: #{} = {}:{}/{} (db oid {}), #{} = {}:{}/{} (db oid {})",
                older.id,
                older.host,
                older.port,
                older.database,
                older.database_oid,
                newer.id,
                newer.host,
                newer.port,
                newer.database,
                newer.database_oid,
            ),
        );
    }

    Ok(())
}

fn monotonic_delta(
    older: Option<i64>,
    newer: Option<i64>,
    name: &str,
    limitations: &mut Vec<String>,
) -> Option<i64> {
    match (older, newer) {
        (Some(older), Some(newer)) if newer >= older => Some(newer - older),

        (Some(older), Some(newer)) => {
            limitations.push(format!(
                "{name} уменьшился ({older} → {newer}) при том же stats_reset; delta не рассчитан."
            ));

            None
        }

        _ => None,
    }
}

fn optional_delta(older: Option<i64>, newer: Option<i64>) -> Option<i64> {
    match (older, newer) {
        (Some(older), Some(newer)) => Some(newer - older),

        _ => None,
    }
}

fn optional_f64_delta(older: Option<f64>, newer: Option<f64>) -> Option<f64> {
    match (older, newer) {
        (Some(older), Some(newer)) => Some(newer - older),

        _ => None,
    }
}

fn compare_pgss_role_database(
    older: &PgssSection<Vec<PgssRoleDatabaseCardinality>>,
    newer: &PgssSection<Vec<PgssRoleDatabaseCardinality>>,
    limitations: &mut Vec<String>,
) -> Vec<PgssRoleDatabaseDelta> {
    let (Some(older_rows), Some(newer_rows)) = (older.data.as_ref(), newer.data.as_ref()) else {
        limitations
            .push("Role/database cardinality delta недоступна хотя бы для одного snapshot.".into());

        return Vec::new();
    };

    let older_map: HashMap<(u32, u32), &PgssRoleDatabaseCardinality> = older_rows
        .iter()
        .map(|row| ((row.userid, row.dbid), row))
        .collect();

    let newer_map: HashMap<(u32, u32), &PgssRoleDatabaseCardinality> = newer_rows
        .iter()
        .map(|row| ((row.userid, row.dbid), row))
        .collect();

    let mut keys: HashSet<(u32, u32)> = older_map.keys().copied().collect();

    keys.extend(newer_map.keys().copied());

    let mut result = keys
        .into_iter()
        .map(|key| {
            let older_row = older_map.get(&key);

            let newer_row = newer_map.get(&key);

            PgssRoleDatabaseDelta {
                userid: key.0,
                dbid: key.1,
                role_name: newer_row
                    .and_then(|row| row.role_name.clone())
                    .or_else(|| older_row.and_then(|row| row.role_name.clone())),
                database: newer_row
                    .and_then(|row| row.database.clone())
                    .or_else(|| older_row.and_then(|row| row.database.clone())),
                older_entries: older_row.map(|row| row.entries),
                newer_entries: newer_row.map(|row| row.entries),
                entries_delta: sampled_delta(
                    older_row.map(|row| row.entries),
                    newer_row.map(|row| row.entries),
                ),
                older_one_call_entries: older_row.map(|row| row.one_call_entries),
                newer_one_call_entries: newer_row.map(|row| row.one_call_entries),
                one_call_delta: sampled_delta(
                    older_row.map(|row| row.one_call_entries),
                    newer_row.map(|row| row.one_call_entries),
                ),
                sample_state: sample_state(older_row.is_some(), newer_row.is_some()),
            }
        })
        .collect::<Vec<_>>();

    result
        .sort_by_key(|row| std::cmp::Reverse(row.newer_entries.or(row.older_entries).unwrap_or(0)));

    result
}

fn compare_pgss_commands(
    older: &PgssSection<Vec<PgssCommandCardinality>>,
    newer: &PgssSection<Vec<PgssCommandCardinality>>,
    limitations: &mut Vec<String>,
) -> Vec<PgssCommandDelta> {
    let (Some(older_rows), Some(newer_rows)) = (older.data.as_ref(), newer.data.as_ref()) else {
        limitations
            .push("Command cardinality delta недоступна хотя бы для одного snapshot.".into());

        return Vec::new();
    };

    let older_map: HashMap<(u32, u32, String), &PgssCommandCardinality> = older_rows
        .iter()
        .map(|row| ((row.userid, row.dbid, row.command.clone()), row))
        .collect();

    let newer_map: HashMap<(u32, u32, String), &PgssCommandCardinality> = newer_rows
        .iter()
        .map(|row| ((row.userid, row.dbid, row.command.clone()), row))
        .collect();

    let mut keys: HashSet<(u32, u32, String)> = older_map.keys().cloned().collect();

    keys.extend(newer_map.keys().cloned());

    let mut result = keys
        .into_iter()
        .map(|key| {
            let older_row = older_map.get(&key);

            let newer_row = newer_map.get(&key);

            PgssCommandDelta {
                userid: key.0,
                dbid: key.1,
                command: key.2,
                role_name: newer_row
                    .and_then(|row| row.role_name.clone())
                    .or_else(|| older_row.and_then(|row| row.role_name.clone())),
                database: newer_row
                    .and_then(|row| row.database.clone())
                    .or_else(|| older_row.and_then(|row| row.database.clone())),
                older_entries: older_row.map(|row| row.entries),
                newer_entries: newer_row.map(|row| row.entries),
                entries_delta: sampled_delta(
                    older_row.map(|row| row.entries),
                    newer_row.map(|row| row.entries),
                ),
                older_one_call_entries: older_row.map(|row| row.one_call_entries),
                newer_one_call_entries: newer_row.map(|row| row.one_call_entries),
                one_call_delta: sampled_delta(
                    older_row.map(|row| row.one_call_entries),
                    newer_row.map(|row| row.one_call_entries),
                ),
                sample_state: sample_state(older_row.is_some(), newer_row.is_some()),
            }
        })
        .collect::<Vec<_>>();

    result
        .sort_by_key(|row| std::cmp::Reverse(row.newer_entries.or(row.older_entries).unwrap_or(0)));

    result
}

fn compare_pgss_call_targets(
    older: &PgssSection<Vec<PgssCallTargetCardinality>>,
    newer: &PgssSection<Vec<PgssCallTargetCardinality>>,
    limitations: &mut Vec<String>,
) -> Vec<PgssCallTargetDelta> {
    let (Some(older_rows), Some(newer_rows)) = (older.data.as_ref(), newer.data.as_ref()) else {
        limitations.push("CALL target delta недоступна хотя бы для одного snapshot.".into());

        return Vec::new();
    };

    let older_map: HashMap<(u32, u32, String), &PgssCallTargetCardinality> = older_rows
        .iter()
        .map(|row| ((row.userid, row.dbid, row.call_tag.clone()), row))
        .collect();

    let newer_map: HashMap<(u32, u32, String), &PgssCallTargetCardinality> = newer_rows
        .iter()
        .map(|row| ((row.userid, row.dbid, row.call_tag.clone()), row))
        .collect();

    let mut keys: HashSet<(u32, u32, String)> = older_map.keys().cloned().collect();

    keys.extend(newer_map.keys().cloned());

    let mut result = keys
        .into_iter()
        .map(|key| {
            let older_row = older_map.get(&key);

            let newer_row = newer_map.get(&key);

            PgssCallTargetDelta {
                userid: key.0,
                dbid: key.1,
                call_tag: key.2,
                role_name: newer_row
                    .and_then(|row| row.role_name.clone())
                    .or_else(|| older_row.and_then(|row| row.role_name.clone())),
                database: newer_row
                    .and_then(|row| row.database.clone())
                    .or_else(|| older_row.and_then(|row| row.database.clone())),
                older_entries: older_row.map(|row| row.entries),
                newer_entries: newer_row.map(|row| row.entries),
                entries_delta: sampled_delta(
                    older_row.map(|row| row.entries),
                    newer_row.map(|row| row.entries),
                ),
                older_one_call_entries: older_row.map(|row| row.one_call_entries),
                newer_one_call_entries: newer_row.map(|row| row.one_call_entries),
                one_call_delta: sampled_delta(
                    older_row.map(|row| row.one_call_entries),
                    newer_row.map(|row| row.one_call_entries),
                ),
                sample_state: sample_state(older_row.is_some(), newer_row.is_some()),
            }
        })
        .collect::<Vec<_>>();

    result
        .sort_by_key(|row| std::cmp::Reverse(row.newer_entries.or(row.older_entries).unwrap_or(0)));

    result
}

fn sampled_delta(older: Option<i64>, newer: Option<i64>) -> Option<i64> {
    match (older, newer) {
        (Some(older), Some(newer)) => Some(newer - older),

        _ => None,
    }
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

fn pgss_snapshot_order_key(summary: &PgssSnapshotSummary) -> (i64, i64) {
    (summary.collection_started_at_ms, summary.id)
}

#[cfg(test)]
mod statement_history_tests {
    use super::*;

    fn performance_row(calls: i64, total_exec_time_ms: f64) -> PgssPerformanceRow {
        PgssPerformanceRow {
            userid: 10,
            dbid: 20,
            role_name: Some("app".into()),
            database: Some("db".into()),
            toplevel: true,
            queryid: Some("-123".into()),
            calls,
            plans: calls,
            total_plan_time_ms: calls as f64,
            mean_plan_time_ms: 1.0,
            stddev_plan_time_ms: 0.0,
            total_exec_time_ms,
            min_exec_time_ms: 1.0,
            max_exec_time_ms: 10.0,
            mean_exec_time_ms: total_exec_time_ms / calls.max(1) as f64,
            stddev_exec_time_ms: 0.0,
            rows: calls * 2,
            shared_blks_hit: calls * 3,
            shared_blks_read: calls * 4,
            shared_blks_dirtied: calls,
            shared_blks_written: calls,
            temp_blks_read: calls,
            temp_blks_written: calls * 2,
            blk_read_time_ms: calls as f64,
            blk_write_time_ms: calls as f64,
            wal_records: calls,
            wal_fpi: calls,
            wal_bytes: calls as f64 * 100.0,
        }
    }

    fn context(
        snapshot_id: i64,
        timestamp_ms: i64,
        dealloc: Option<i64>,
        row: Option<PgssPerformanceRow>,
    ) -> PgssStatementSampleContext {
        let global_total_calls = row.as_ref().map(|value| value.calls * 10);
        let global_total_exec_time_ms = row.as_ref().map(|value| value.total_exec_time_ms * 10.0);

        PgssStatementSampleContext {
            snapshot_id,
            collection_started_at_ms: timestamp_ms,
            measured_at: "2026-09-16T10:00:00Z".into(),
            stats_reset: Some("2026-09-16T09:00:00Z".into()),
            dealloc,
            sample_scope: "history_sample".into(),
            global_total_calls,
            global_total_exec_time_ms,
            row,
        }
    }

    #[test]
    fn statement_delta_uses_only_adjacent_stable_interval() {
        let older = context(1, 0, Some(7), Some(performance_row(10, 100.0)));
        let newer_row = performance_row(30, 500.0);
        let newer = context(2, 3_600_000, Some(7), Some(newer_row.clone()));

        let delta = pgss_statement_delta(Some(&older), &newer, &newer_row);

        assert_eq!(delta.state, "comparable");
        assert_eq!(delta.calls_delta, Some(20));
        assert!((delta.calls_per_hour.unwrap() - 20.0).abs() < 1e-9);
        assert_eq!(delta.total_exec_time_delta_ms, Some(400.0));
        assert_eq!(delta.interval_mean_exec_time_ms, Some(20.0));
        assert_eq!(delta.calls_workload_share_pct, Some(10.0));
        assert_eq!(delta.exec_workload_share_pct, Some(10.0));
        assert_eq!(delta.plans_delta, Some(20));
        assert_eq!(delta.interval_mean_plan_time_ms, Some(1.0));
        assert_eq!(delta.shared_blks_read_delta, Some(80));
        assert_eq!(delta.temp_blks_written_delta, Some(40));
        assert_eq!(delta.wal_bytes_delta, Some(2_000.0));
    }

    fn history_point(
        snapshot_id: i64,
        calls_per_hour: Option<f64>,
        interval_mean_exec_time_ms: Option<f64>,
        shared_reads_delta: Option<i64>,
        temp_written_delta: Option<i64>,
        wal_bytes_delta: Option<f64>,
        state: &str,
    ) -> PgssStatementHistoryPoint {
        PgssStatementHistoryPoint {
            snapshot_id,
            collection_started_at_ms: snapshot_id * 3_600_000,
            measured_at: "2026-09-16T10:00:00Z".into(),
            stats_reset: Some("2026-09-16T09:00:00Z".into()),
            dealloc: Some(7),
            sample_scope: "history_sample".into(),
            row: performance_row(snapshot_id * 10, snapshot_id as f64 * 100.0),
            delta_state: state.into(),
            elapsed_seconds: Some(3_600.0),
            calls_delta: calls_per_hour.map(|value| value as i64),
            calls_per_hour,
            calls_workload_share_pct: Some(10.0),
            plans_delta: Some(10),
            total_exec_time_delta_ms: Some(100.0),
            interval_mean_exec_time_ms,
            exec_workload_share_pct: Some(12.0),
            total_plan_time_delta_ms: Some(10.0),
            interval_mean_plan_time_ms: Some(1.0),
            rows_delta: Some(20),
            shared_blks_read_delta: shared_reads_delta,
            shared_blks_hit_delta: Some(30),
            temp_blks_read_delta: Some(0),
            temp_blks_written_delta: temp_written_delta,
            wal_bytes_delta,
        }
    }

    #[test]
    fn statement_change_analysis_compares_two_latest_safe_intervals() {
        let points = vec![
            history_point(1, None, None, None, None, None, "baseline"),
            history_point(
                2,
                Some(20.0),
                Some(10.0),
                Some(40),
                Some(0),
                Some(2_000.0),
                "comparable",
            ),
            history_point(
                3,
                Some(40.0),
                Some(25.0),
                Some(160),
                Some(80),
                Some(8_000.0),
                "comparable",
            ),
        ];

        let analysis = pgss_statement_change_analysis(&points);

        assert_eq!(analysis.state, "ready");
        assert_eq!(analysis.older_from_snapshot_id, Some(1));
        assert_eq!(analysis.older_to_snapshot_id, Some(2));
        assert_eq!(analysis.newer_to_snapshot_id, Some(3));
        let latency = analysis
            .items
            .iter()
            .find(|item| item.id == "interval_latency")
            .unwrap();
        assert_eq!(latency.direction, "increased");
        assert_eq!(latency.evidence_level, "confirmed_change");
        let temp = analysis
            .items
            .iter()
            .find(|item| item.id == "temp_written_per_call")
            .unwrap();
        assert_eq!(temp.direction, "appeared");
    }

    #[test]
    fn statement_change_analysis_requires_latest_safe_interval() {
        let points = vec![
            history_point(1, None, None, None, None, None, "baseline"),
            history_point(
                2,
                Some(20.0),
                Some(10.0),
                Some(40),
                Some(0),
                Some(2_000.0),
                "comparable",
            ),
            history_point(3, None, None, None, None, None, "dealloc_changed"),
        ];

        let analysis = pgss_statement_change_analysis(&points);

        assert_eq!(analysis.state, "latest_interval_not_comparable");
        assert!(analysis.items.is_empty());
    }

    #[test]
    fn statement_delta_rejects_interval_with_dealloc_change() {
        let older = context(1, 0, Some(7), Some(performance_row(10, 100.0)));
        let newer_row = performance_row(30, 500.0);
        let newer = context(2, 3_600_000, Some(8), Some(newer_row.clone()));

        let delta = pgss_statement_delta(Some(&older), &newer, &newer_row);

        assert_eq!(delta.state, "dealloc_changed");
        assert!(delta.calls_delta.is_none());
    }

    #[test]
    fn statement_delta_rejects_sample_gap() {
        let older = context(1, 0, Some(7), None);
        let newer_row = performance_row(30, 500.0);
        let newer = context(2, 3_600_000, Some(7), Some(newer_row.clone()));

        let delta = pgss_statement_delta(Some(&older), &newer, &newer_row);

        assert_eq!(delta.state, "sample_gap");
        assert!(delta.calls_delta.is_none());
    }
}
