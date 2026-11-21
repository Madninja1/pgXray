use super::{unix_time_millis, DiagnosticStore, DEFAULT_HISTORY_LIMIT, MAX_HISTORY_LIMIT};
use crate::{
    connections::SavedConnection,
    io_wal::{IoWalRelationSize, IoWalSnapshot},
};
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const IO_WAL_SNAPSHOT_SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct CollectedIoWalSnapshot {
    pub history: IoWalSnapshotSummary,
    pub snapshot: IoWalSnapshot,
}

#[derive(Clone, Debug, Serialize)]
pub struct IoWalSnapshotSummary {
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
    pub database_stats_reset: Option<String>,
    pub wal_stats_reset: Option<String>,
    pub checkpoint_stats_reset: Option<String>,
    pub database_size_bytes: Option<i64>,
    pub temp_bytes: Option<i64>,
    pub wal_bytes: Option<f64>,
    pub checkpoint_timed: Option<i64>,
    pub checkpoint_requested: Option<i64>,
    pub relation_count: Option<i64>,
    pub relation_sample_truncated: bool,
    pub unavailable_sections: i64,
    pub stored_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct IoWalRelationSizeChange {
    pub relation_oid: u32,
    pub older_name: Option<String>,
    pub newer_name: Option<String>,
    pub sample_state: String,
    pub older_heap_size_bytes: Option<i64>,
    pub newer_heap_size_bytes: Option<i64>,
    pub heap_size_delta_bytes: Option<i64>,
    pub older_indexes_size_bytes: Option<i64>,
    pub newer_indexes_size_bytes: Option<i64>,
    pub indexes_size_delta_bytes: Option<i64>,
    pub older_toast_size_bytes: Option<i64>,
    pub newer_toast_size_bytes: Option<i64>,
    pub toast_size_delta_bytes: Option<i64>,
    pub older_total_size_bytes: Option<i64>,
    pub newer_total_size_bytes: Option<i64>,
    pub total_size_delta_bytes: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IoWalComparison {
    pub older: IoWalSnapshotSummary,
    pub newer: IoWalSnapshotSummary,
    pub elapsed_seconds: f64,
    pub database_stats_continuous: bool,
    pub wal_stats_continuous: bool,
    pub checkpoint_stats_continuous: bool,
    pub temp_files_delta: Option<i64>,
    pub temp_bytes_delta: Option<i64>,
    pub temp_bytes_per_hour: Option<f64>,
    pub database_blocks_read_delta: Option<i64>,
    pub database_blocks_hit_delta: Option<i64>,
    pub wal_bytes_delta: Option<f64>,
    pub wal_bytes_per_hour: Option<f64>,
    pub wal_buffers_full_delta: Option<i64>,
    pub wal_write_time_delta_ms: Option<f64>,
    pub wal_sync_time_delta_ms: Option<f64>,
    pub checkpoints_timed_delta: Option<i64>,
    pub checkpoints_requested_delta: Option<i64>,
    pub checkpoint_write_time_delta_ms: Option<f64>,
    pub checkpoint_sync_time_delta_ms: Option<f64>,
    pub checkpoint_buffers_written_delta: Option<i64>,
    pub database_size_delta_bytes: Option<i64>,
    pub relation_size_changes: Vec<IoWalRelationSizeChange>,
    pub limitations: Vec<String>,
}

/// One observed point. Database size is a gauge; it never requires stats_reset continuity.
#[derive(Clone, Debug, Serialize)]
pub struct IoWalHistorySizePoint {
    pub snapshot_id: i64,
    pub at_ms: i64,
    pub database_size_bytes: Option<i64>,
}

/// Independent continuity for DB, WAL and checkpoint counters. A missing reset
/// timestamp does not prove a reset happened.
#[derive(Clone, Debug, Serialize)]
pub struct IoWalWorkloadInterval {
    pub from_snapshot_id: i64,
    pub to_snapshot_id: i64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub elapsed_seconds: Option<f64>,
    pub state: String,
    pub database_state: String,
    pub wal_state: String,
    pub checkpoint_state: String,
    pub wal_bytes_per_hour: Option<f64>,
    pub wal_records_per_hour: Option<f64>,
    pub wal_buffers_full_per_hour: Option<f64>,
    pub wal_write_ms_per_hour: Option<f64>,
    pub wal_sync_ms_per_hour: Option<f64>,
    pub temp_bytes_per_hour: Option<f64>,
    pub temp_files_per_hour: Option<f64>,
    pub blocks_read_per_hour: Option<f64>,
    pub blocks_hit_per_hour: Option<f64>,
    pub checkpoints_requested_per_hour: Option<f64>,
    pub checkpoints_timed_per_hour: Option<f64>,
    pub checkpoint_buffers_per_hour: Option<f64>,
    pub database_size_bytes: Option<i64>,
    pub database_size_delta_bytes: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IoWalWorkloadHistory {
    pub snapshot_count: usize,
    pub intervals: Vec<IoWalWorkloadInterval>,
    pub size_points: Vec<IoWalHistorySizePoint>,
    pub selected_to_snapshot_id: Option<i64>,
    pub relation_size_changes: Vec<IoWalRelationSizeChange>,
    pub relation_sample_truncated: bool,
    pub relation_sizes_available: bool,
    pub limitations: Vec<String>,
}

/// True for a source that remained comparable between measurements.
fn history_section_status(
    available: bool,
    before_reset: Option<&str>,
    after_reset: Option<&str>,
    compatible_source: bool,
) -> &'static str {
    if !available {
        "unavailable"
    } else if !compatible_source {
        "source_changed"
    } else {
        match (before_reset, after_reset) {
            (Some(before), Some(after)) if before == after => "comparable",
            (Some(_), Some(_)) => "reset",
            _ => "unknown_reset",
        }
    }
}

fn history_counter_rate(before: Option<i64>, after: Option<i64>, elapsed: f64) -> Option<f64> {
    rate_per_hour(counter_delta(before, after).map(|v| v as f64), elapsed)
}

fn history_float_rate(before: Option<f64>, after: Option<f64>, elapsed: f64) -> Option<f64> {
    rate_per_hour(float_counter_delta(before, after), elapsed)
}

fn history_interval(
    older: &StoredIoWalSnapshot,
    newer: &StoredIoWalSnapshot,
) -> IoWalWorkloadInterval {
    let first = &older.summary;
    let last = &newer.summary;
    let valid_target = ensure_same_target(first, last).is_ok();
    let elapsed = (last.collection_started_at_ms - first.collection_started_at_ms) as f64 / 1000.0;
    let valid_time = elapsed.is_finite() && elapsed > 0.0;
    // A very long span is a difference between two points, not an observed hourly trend.
    let valid_span = valid_time && elapsed <= 6.0 * 3600.0;
    let db_old = older.snapshot.database_stats.data.as_ref();
    let db_new = newer.snapshot.database_stats.data.as_ref();
    let wal_old = older.snapshot.wal.data.as_ref();
    let wal_new = newer.snapshot.wal.data.as_ref();
    let cp_old = older.snapshot.checkpoints.data.as_ref();
    let cp_new = newer.snapshot.checkpoints.data.as_ref();
    let status = |available, old_reset, new_reset, source| {
        if !valid_target {
            "target_changed"
        } else if !valid_time {
            "invalid_time"
        } else if !valid_span {
            "gap"
        } else {
            history_section_status(available, old_reset, new_reset, source)
        }
    };
    let database_state = status(
        db_old.is_some() && db_new.is_some(),
        first.database_stats_reset.as_deref(),
        last.database_stats_reset.as_deref(),
        true,
    );
    let wal_state = status(
        wal_old.is_some() && wal_new.is_some(),
        first.wal_stats_reset.as_deref(),
        last.wal_stats_reset.as_deref(),
        true,
    );
    let cp_same_source = cp_old
        .zip(cp_new)
        .is_some_and(|(a, b)| a.source == b.source);
    let checkpoint_state = status(
        cp_old.is_some() && cp_new.is_some(),
        first.checkpoint_stats_reset.as_deref(),
        last.checkpoint_stats_reset.as_deref(),
        cp_same_source,
    );
    let db_ok = database_state == "comparable";
    let wal_ok = wal_state == "comparable";
    let cp_ok = checkpoint_state == "comparable";
    let usable = [db_ok, wal_ok, cp_ok].iter().filter(|ok| **ok).count();
    let state = if !valid_target {
        "target_changed"
    } else if !valid_time {
        "invalid_time"
    } else if !valid_span {
        "gap"
    } else if usable == 3 {
        "comparable"
    } else if usable > 0 {
        "partial"
    } else {
        "unavailable"
    };
    let db_counter = |f: fn(&crate::io_wal::IoWalDatabaseStats) -> i64| {
        if db_ok {
            history_counter_rate(db_old.map(f), db_new.map(f), elapsed)
        } else {
            None
        }
    };
    let wal_counter = |f: fn(&crate::io_wal::IoWalWalStats) -> i64| {
        if wal_ok {
            history_counter_rate(wal_old.map(f), wal_new.map(f), elapsed)
        } else {
            None
        }
    };
    let wal_float = |f: fn(&crate::io_wal::IoWalWalStats) -> f64| {
        if wal_ok {
            history_float_rate(wal_old.map(f), wal_new.map(f), elapsed)
        } else {
            None
        }
    };
    let cp_counter = |f: fn(&crate::io_wal::IoWalCheckpointStats) -> i64| {
        if cp_ok {
            history_counter_rate(cp_old.map(f), cp_new.map(f), elapsed)
        } else {
            None
        }
    };
    IoWalWorkloadInterval {
        from_snapshot_id: first.id,
        to_snapshot_id: last.id,
        from_ms: first.collection_started_at_ms,
        to_ms: last.collection_started_at_ms,
        elapsed_seconds: valid_time.then_some(elapsed),
        state: state.into(),
        database_state: database_state.into(),
        wal_state: wal_state.into(),
        checkpoint_state: checkpoint_state.into(),
        wal_bytes_per_hour: wal_float(|s| s.wal_bytes),
        wal_records_per_hour: wal_counter(|s| s.wal_records),
        wal_buffers_full_per_hour: wal_counter(|s| s.wal_buffers_full),
        wal_write_ms_per_hour: wal_float(|s| s.wal_write_time_ms),
        wal_sync_ms_per_hour: wal_float(|s| s.wal_sync_time_ms),
        temp_bytes_per_hour: db_counter(|s| s.temp_bytes),
        temp_files_per_hour: db_counter(|s| s.temp_files),
        blocks_read_per_hour: db_counter(|s| s.blks_read),
        blocks_hit_per_hour: db_counter(|s| s.blks_hit),
        checkpoints_requested_per_hour: cp_counter(|s| s.requested),
        checkpoints_timed_per_hour: cp_counter(|s| s.timed),
        checkpoint_buffers_per_hour: cp_counter(|s| s.buffers_written),
        database_size_bytes: last.database_size_bytes,
        database_size_delta_bytes: if valid_target && valid_time {
            signed_gauge_delta(first.database_size_bytes, last.database_size_bytes)
        } else {
            None
        },
    }
}

#[derive(Clone)]
pub(super) struct StoredIoWalSnapshot {
    pub summary: IoWalSnapshotSummary,
    pub snapshot: IoWalSnapshot,
}

pub(super) fn init_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS io_wal_snapshots (
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
                database_stats_reset TEXT,
                wal_stats_reset TEXT,
                checkpoint_stats_reset TEXT,
                database_size_bytes INTEGER,
                temp_bytes INTEGER,
                wal_bytes REAL,
                checkpoint_timed INTEGER,
                checkpoint_requested INTEGER,
                relation_count INTEGER,
                relation_sample_truncated INTEGER NOT NULL,
                unavailable_sections INTEGER NOT NULL,
                stored_at_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_io_wal_snapshots_connection_time
                ON io_wal_snapshots(connection_id, collection_started_at_ms DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_io_wal_snapshots_target_time
                ON io_wal_snapshots(host, port, database_oid, collection_started_at_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_io_wal_snapshots_stored_at
                ON io_wal_snapshots(stored_at_ms);
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить I/O & WAL history schema: {error}"))
}

impl DiagnosticStore {
    pub fn save_io_wal_snapshot(
        &mut self,
        saved_connection: &SavedConnection,
        snapshot: &IoWalSnapshot,
        collection_started_at_ms: i64,
        collection_completed_at_ms: i64,
    ) -> Result<IoWalSnapshotSummary, String> {
        if collection_completed_at_ms < collection_started_at_ms {
            return Err(crate::i18n::text(
                "Время завершения I/O & WAL snapshot раньше времени начала",
                "I/O & WAL snapshot finished before it started",
            )
            .into());
        }

        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|error| format!("Не удалось сериализовать I/O & WAL snapshot: {error}"))?;
        let stored_at_ms = unix_time_millis()?;
        let database_stats = snapshot.database_stats.data.as_ref();
        let wal = snapshot.wal.data.as_ref();
        let checkpoints = snapshot.checkpoints.data.as_ref();
        let sizes = snapshot.relation_sizes.data.as_ref();
        let database_size = snapshot.database_size_bytes.data;
        let unavailable_sections = [
            snapshot.database_stats.error.is_some(),
            snapshot.wal.error.is_some(),
            snapshot.checkpoints.error.is_some(),
            snapshot.database_size_bytes.error.is_some(),
            snapshot.relation_sizes.error.is_some(),
        ]
        .into_iter()
        .filter(|value| *value)
        .count() as i64;

        self.connection
            .execute(
                r#"
                INSERT INTO io_wal_snapshots (
                    schema_version, app_version, connection_id, connection_name, host, port,
                    database_name, database_oid, username, sslmode, server_version,
                    server_version_num, collection_started_at_ms, collection_completed_at_ms,
                    measured_at, database_stats_reset, wal_stats_reset, checkpoint_stats_reset,
                    database_size_bytes, temp_bytes, wal_bytes, checkpoint_timed,
                    checkpoint_requested, relation_count, relation_sample_truncated,
                    unavailable_sections, stored_at_ms, snapshot_json
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28
                )
                "#,
                params![
                    IO_WAL_SNAPSHOT_SCHEMA_VERSION,
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
                    database_stats.and_then(|value| value.stats_reset.as_deref()),
                    wal.map(|value| value.stats_reset.as_str()),
                    checkpoints.map(|value| value.stats_reset.as_str()),
                    database_size,
                    database_stats.map(|value| value.temp_bytes),
                    wal.map(|value| value.wal_bytes),
                    checkpoints.map(|value| value.timed),
                    checkpoints.map(|value| value.requested),
                    sizes.map(|value| value.total_relation_count),
                    sizes.map(|value| value.truncated).unwrap_or(false),
                    unavailable_sections,
                    stored_at_ms,
                    snapshot_json,
                ],
            )
            .map_err(|error| format!("Не удалось сохранить I/O & WAL snapshot: {error}"))?;

        let id = self.connection.last_insert_rowid();
        Ok(IoWalSnapshotSummary {
            id,
            schema_version: IO_WAL_SNAPSHOT_SCHEMA_VERSION,
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
            database_stats_reset: database_stats.and_then(|value| value.stats_reset.clone()),
            wal_stats_reset: wal.map(|value| value.stats_reset.clone()),
            checkpoint_stats_reset: checkpoints.map(|value| value.stats_reset.clone()),
            database_size_bytes: database_size,
            temp_bytes: database_stats.map(|value| value.temp_bytes),
            wal_bytes: wal.map(|value| value.wal_bytes),
            checkpoint_timed: checkpoints.map(|value| value.timed),
            checkpoint_requested: checkpoints.map(|value| value.requested),
            relation_count: sizes.map(|value| value.total_relation_count),
            relation_sample_truncated: sizes.map(|value| value.truncated).unwrap_or(false),
            unavailable_sections,
            stored_at_ms,
        })
    }

    pub fn list_io_wal_snapshots(
        &self,
        connection_id: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<IoWalSnapshotSummary>, String> {
        let limit = limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);
        let sql = if connection_id.is_some() {
            format!("{} WHERE connection_id = ?1 ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?2", SUMMARY_SELECT)
        } else {
            format!(
                "{} ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?1",
                SUMMARY_SELECT
            )
        };
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| format!("Не удалось подготовить I/O & WAL history: {error}"))?;
        let mut result = Vec::new();
        if let Some(connection_id) = connection_id {
            let rows = statement
                .query_map(params![connection_id, i64::from(limit)], summary_from_row)
                .map_err(|error| format!("Не удалось прочитать I/O & WAL history: {error}"))?;
            for row in rows {
                result.push(row.map_err(|error| format!("Повреждена I/O & WAL history: {error}"))?);
            }
        } else {
            let rows = statement
                .query_map(params![i64::from(limit)], summary_from_row)
                .map_err(|error| format!("Не удалось прочитать I/O & WAL history: {error}"))?;
            for row in rows {
                result.push(row.map_err(|error| format!("Повреждена I/O & WAL history: {error}"))?);
            }
        }
        Ok(result)
    }

    /// Return a saved diagnostic snapshot only for the selected connection.
    /// Snapshot IDs alone are not evidence of the PostgreSQL source.
    pub fn io_wal_snapshot(
        &self,
        connection_id: i64,
        snapshot_id: i64,
    ) -> Result<IoWalSnapshot, String> {
        let stored = load_snapshot(&self.connection, snapshot_id)?;
        if stored.summary.connection_id != connection_id {
            return Err(crate::i18n::text(
                "I/O & WAL snapshot принадлежит другому подключению",
                "I/O & WAL snapshot belongs to a different connection",
            )
            .into());
        }
        Ok(stored.snapshot)
    }

    pub fn compare_io_wal_snapshots(
        &self,
        first_id: i64,
        second_id: i64,
    ) -> Result<IoWalComparison, String> {
        let first = load_snapshot(&self.connection, first_id)?;
        let second = load_snapshot(&self.connection, second_id)?;
        let (older, newer) =
            if first.summary.collection_started_at_ms <= second.summary.collection_started_at_ms {
                (first, second)
            } else {
                (second, first)
            };

        ensure_same_target(&older.summary, &newer.summary)?;
        let elapsed_seconds = (newer.summary.collection_started_at_ms
            - older.summary.collection_started_at_ms) as f64
            / 1000.0;
        if elapsed_seconds <= 0.0 {
            return Err(crate::i18n::text(
                "I/O & WAL snapshots имеют одинаковое или некорректное время",
                "I/O & WAL snapshots have identical or invalid timestamps",
            )
            .into());
        }

        let database_stats_continuous = same_reset(
            older.summary.database_stats_reset.as_deref(),
            newer.summary.database_stats_reset.as_deref(),
        );
        let wal_stats_continuous = same_reset(
            older.summary.wal_stats_reset.as_deref(),
            newer.summary.wal_stats_reset.as_deref(),
        );
        let checkpoint_stats_continuous = same_reset(
            older.summary.checkpoint_stats_reset.as_deref(),
            newer.summary.checkpoint_stats_reset.as_deref(),
        ) && older
            .snapshot
            .checkpoints
            .data
            .as_ref()
            .map(|v| v.source.as_str())
            == newer
                .snapshot
                .checkpoints
                .data
                .as_ref()
                .map(|v| v.source.as_str());

        let mut limitations = Vec::new();
        let old_db = older.snapshot.database_stats.data.as_ref();
        let new_db = newer.snapshot.database_stats.data.as_ref();
        let (
            temp_files_delta,
            temp_bytes_delta,
            database_blocks_read_delta,
            database_blocks_hit_delta,
        ) = if database_stats_continuous {
            (
                counter_delta(old_db.map(|v| v.temp_files), new_db.map(|v| v.temp_files)),
                counter_delta(old_db.map(|v| v.temp_bytes), new_db.map(|v| v.temp_bytes)),
                counter_delta(old_db.map(|v| v.blks_read), new_db.map(|v| v.blks_read)),
                counter_delta(old_db.map(|v| v.blks_hit), new_db.map(|v| v.blks_hit)),
            )
        } else {
            limitations.push(
                crate::i18n::text(
                    "pg_stat_database stats_reset изменился; temp/block deltas не рассчитаны.",
                    "pg_stat_database stats_reset changed; temp/block deltas were not calculated.",
                )
                .into(),
            );
            (None, None, None, None)
        };

        let old_wal = older.snapshot.wal.data.as_ref();
        let new_wal = newer.snapshot.wal.data.as_ref();
        let (
            wal_bytes_delta,
            wal_buffers_full_delta,
            wal_write_time_delta_ms,
            wal_sync_time_delta_ms,
        ) = if wal_stats_continuous {
            (
                float_counter_delta(old_wal.map(|v| v.wal_bytes), new_wal.map(|v| v.wal_bytes)),
                counter_delta(
                    old_wal.map(|v| v.wal_buffers_full),
                    new_wal.map(|v| v.wal_buffers_full),
                ),
                float_counter_delta(
                    old_wal.map(|v| v.wal_write_time_ms),
                    new_wal.map(|v| v.wal_write_time_ms),
                ),
                float_counter_delta(
                    old_wal.map(|v| v.wal_sync_time_ms),
                    new_wal.map(|v| v.wal_sync_time_ms),
                ),
            )
        } else {
            limitations.push(
                crate::i18n::text("pg_stat_wal stats_reset изменился/недоступен; WAL deltas не рассчитаны.", "pg_stat_wal stats_reset changed or is unavailable; WAL deltas were not calculated.").into(),
            );
            (None, None, None, None)
        };

        let old_checkpoint = older.snapshot.checkpoints.data.as_ref();
        let new_checkpoint = newer.snapshot.checkpoints.data.as_ref();
        let (
            checkpoints_timed_delta,
            checkpoints_requested_delta,
            checkpoint_write_time_delta_ms,
            checkpoint_sync_time_delta_ms,
            checkpoint_buffers_written_delta,
        ) = if checkpoint_stats_continuous {
            (
                counter_delta(
                    old_checkpoint.map(|v| v.timed),
                    new_checkpoint.map(|v| v.timed),
                ),
                counter_delta(
                    old_checkpoint.map(|v| v.requested),
                    new_checkpoint.map(|v| v.requested),
                ),
                float_counter_delta(
                    old_checkpoint.map(|v| v.write_time_ms),
                    new_checkpoint.map(|v| v.write_time_ms),
                ),
                float_counter_delta(
                    old_checkpoint.map(|v| v.sync_time_ms),
                    new_checkpoint.map(|v| v.sync_time_ms),
                ),
                counter_delta(
                    old_checkpoint.map(|v| v.buffers_written),
                    new_checkpoint.map(|v| v.buffers_written),
                ),
            )
        } else {
            limitations.push(
                crate::i18n::text("Checkpoint statistics source/reset изменился; checkpoint deltas не рассчитаны.", "Checkpoint statistics source/reset changed; checkpoint deltas were not calculated.")
                    .into(),
            );
            (None, None, None, None, None)
        };

        if older.summary.relation_sample_truncated || newer.summary.relation_sample_truncated {
            limitations.push(crate::i18n::text("Relation-size sample был truncated; отсутствие relation в одной точке не означает DROP.", "The relation-size sample was truncated; a relation missing at one point does not imply DROP.").into());
        }

        let relation_size_changes = match (
            older.snapshot.relation_sizes.data.as_ref(),
            newer.snapshot.relation_sizes.data.as_ref(),
        ) {
            (Some(older_sizes), Some(newer_sizes)) => {
                compare_relation_sizes(&older_sizes.relations, &newer_sizes.relations)
            }
            _ => {
                limitations.push(
                    crate::i18n::text("Relation-size section недоступна хотя бы в одном snapshot; сравнение размеров пропущено.", "The relation-size section is unavailable in at least one snapshot; size comparison was skipped.")
                        .into(),
                );
                Vec::new()
            }
        };
        let database_size_delta_bytes = signed_gauge_delta(
            older.summary.database_size_bytes,
            newer.summary.database_size_bytes,
        );

        Ok(IoWalComparison {
            older: older.summary,
            newer: newer.summary,
            elapsed_seconds,
            database_stats_continuous,
            wal_stats_continuous,
            checkpoint_stats_continuous,
            temp_files_delta,
            temp_bytes_delta,
            temp_bytes_per_hour: rate_per_hour(temp_bytes_delta.map(|v| v as f64), elapsed_seconds),
            database_blocks_read_delta,
            database_blocks_hit_delta,
            wal_bytes_delta,
            wal_bytes_per_hour: rate_per_hour(wal_bytes_delta, elapsed_seconds),
            wal_buffers_full_delta,
            wal_write_time_delta_ms,
            wal_sync_time_delta_ms,
            checkpoints_timed_delta,
            checkpoints_requested_delta,
            checkpoint_write_time_delta_ms,
            checkpoint_sync_time_delta_ms,
            checkpoint_buffers_written_delta,
            database_size_delta_bytes,
            relation_size_changes,
            limitations,
        })
    }

    /// Read up to 60 snapshots and compare only adjacent observations.
    /// Rates for DB, WAL and checkpoint counters have independent continuity.
    pub fn io_wal_workload_history(
        &self,
        connection_id: i64,
        limit: Option<u32>,
        selected_to_snapshot_id: Option<i64>,
    ) -> Result<IoWalWorkloadHistory, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection_id I/O & WAL history",
                "Invalid I/O & WAL history connection_id",
            )
            .into());
        }
        let mut summaries = self
            .list_io_wal_snapshots(Some(connection_id), Some(limit.unwrap_or(60).clamp(1, 60)))?;
        summaries.reverse();
        let mut stored = Vec::new();
        let mut limitations = Vec::new();
        for summary in summaries {
            if summary.schema_version != IO_WAL_SNAPSHOT_SCHEMA_VERSION {
                limitations.push(format!(
                    "Snapshot #{}: неподдерживаемая schema_version {}",
                    summary.id, summary.schema_version
                ));
                continue;
            }
            stored.push(load_snapshot(&self.connection, summary.id)?);
        }
        let mut intervals: Vec<_> = stored
            .windows(2)
            .map(|pair| history_interval(&pair[0], &pair[1]))
            .collect();
        intervals.reverse();
        let size_points = stored
            .iter()
            .map(|point| IoWalHistorySizePoint {
                snapshot_id: point.summary.id,
                at_ms: point.summary.collection_started_at_ms,
                database_size_bytes: point.summary.database_size_bytes,
            })
            .collect();
        let selected_to_snapshot_id = selected_to_snapshot_id
            .filter(|id| intervals.iter().any(|i| i.to_snapshot_id == *id))
            .or_else(|| intervals.first().map(|i| i.to_snapshot_id));
        let mut relation_size_changes = Vec::new();
        let mut relation_sample_truncated = false;
        let mut relation_sizes_available = false;
        if let Some(pair) = stored
            .windows(2)
            .find(|pair| Some(pair[1].summary.id) == selected_to_snapshot_id)
        {
            if ensure_same_target(&pair[0].summary, &pair[1].summary).is_ok() {
                let old_sizes = pair[0].snapshot.relation_sizes.data.as_ref();
                let new_sizes = pair[1].snapshot.relation_sizes.data.as_ref();
                relation_sample_truncated = old_sizes.is_some_and(|s| s.truncated)
                    || new_sizes.is_some_and(|s| s.truncated);
                if let (Some(older), Some(newer)) = (old_sizes, new_sizes) {
                    relation_sizes_available = true;
                    relation_size_changes =
                        compare_relation_sizes(&older.relations, &newer.relations)
                            .into_iter()
                            .filter(|row| {
                                row.sample_state != "comparable"
                                    || row.total_size_delta_bytes != Some(0)
                                    || row.heap_size_delta_bytes != Some(0)
                                    || row.indexes_size_delta_bytes != Some(0)
                                    || row.toast_size_delta_bytes != Some(0)
                            })
                            .take(100)
                            .collect();
                } else {
                    limitations.push(
                        crate::i18n::text(
                            "Сравнение размеров объектов: секция отсутствует в одном из снимков",
                            "Object-size comparison: the section is missing from one snapshot",
                        )
                        .into(),
                    );
                }
                if relation_sample_truncated {
                    limitations.push(crate::i18n::text("Список объектов ограничен выборкой; отсутствие relation не означает удаление", "The object list is sampled; a missing relation does not imply deletion").into());
                }
            } else {
                limitations.push(
                    crate::i18n::text(
                        "Размеры объектов нельзя сравнить между разными PostgreSQL targets",
                        "Object sizes cannot be compared across different PostgreSQL targets",
                    )
                    .into(),
                );
            }
        }
        limitations.push(
            crate::i18n::text("WAL/checkpoints — весь экземпляр, temp/block counters и размер базы — выбранная БД", "WAL/checkpoints cover the entire instance; temp/block counters and database size cover the selected database")
                .into(),
        );
        limitations.push(crate::i18n::text("Без system_identifier сохранённые снимки не доказывают идентичность физического кластера", "Without system_identifier, saved snapshots do not prove the physical cluster identity").into());
        Ok(IoWalWorkloadHistory {
            snapshot_count: stored.len(),
            intervals,
            size_points,
            selected_to_snapshot_id,
            relation_size_changes,
            relation_sample_truncated,
            relation_sizes_available,
            limitations,
        })
    }

    pub(super) fn io_wal_snapshots_in_period(
        &self,
        connection_id: i64,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<StoredIoWalSnapshot>, String> {
        let mut statement = self
            .connection
            .prepare(
                r#"
            SELECT id, snapshot_json
            FROM io_wal_snapshots
            WHERE connection_id = ?1
              AND collection_started_at_ms >= ?2
              AND collection_started_at_ms <= ?3
            ORDER BY collection_started_at_ms ASC, id ASC
            "#,
            )
            .map_err(|error| format!("Не удалось подготовить I/O & WAL period history: {error}"))?;
        let rows = statement
            .query_map(params![connection_id, from_ms, to_ms], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("Не удалось прочитать I/O & WAL period history: {error}"))?;
        let mut result = Vec::new();
        for row in rows {
            let (id, json) = row.map_err(|error| error.to_string())?;
            let summary = load_summary(&self.connection, id)?;
            let snapshot = serde_json::from_str(&json)
                .map_err(|error| format!("Повреждён I/O & WAL snapshot #{id}: {error}"))?;
            result.push(StoredIoWalSnapshot { summary, snapshot });
        }
        Ok(result)
    }
}

const SUMMARY_SELECT: &str = r#"
SELECT
    id, schema_version, app_version, connection_id, connection_name, host, port,
    database_name, database_oid, username, sslmode, server_version, server_version_num,
    collection_started_at_ms, collection_completed_at_ms, measured_at,
    database_stats_reset, wal_stats_reset, checkpoint_stats_reset, database_size_bytes,
    temp_bytes, wal_bytes, checkpoint_timed, checkpoint_requested, relation_count,
    relation_sample_truncated, unavailable_sections, stored_at_ms
FROM io_wal_snapshots
"#;

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<IoWalSnapshotSummary> {
    Ok(IoWalSnapshotSummary {
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
        database_stats_reset: row.get(16)?,
        wal_stats_reset: row.get(17)?,
        checkpoint_stats_reset: row.get(18)?,
        database_size_bytes: row.get(19)?,
        temp_bytes: row.get(20)?,
        wal_bytes: row.get(21)?,
        checkpoint_timed: row.get(22)?,
        checkpoint_requested: row.get(23)?,
        relation_count: row.get(24)?,
        relation_sample_truncated: row.get(25)?,
        unavailable_sections: row.get(26)?,
        stored_at_ms: row.get(27)?,
    })
}

fn load_summary(connection: &Connection, id: i64) -> Result<IoWalSnapshotSummary, String> {
    connection
        .query_row(
            &format!("{} WHERE id = ?1", SUMMARY_SELECT),
            params![id],
            summary_from_row,
        )
        .map_err(|error| format!("I/O & WAL snapshot #{id} не найден: {error}"))
}

fn load_snapshot(connection: &Connection, id: i64) -> Result<StoredIoWalSnapshot, String> {
    let summary = load_summary(connection, id)?;
    let json: String = connection
        .query_row(
            "SELECT snapshot_json FROM io_wal_snapshots WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| format!("I/O & WAL snapshot #{id} не найден: {error}"))?;
    let snapshot = serde_json::from_str(&json)
        .map_err(|error| format!("Повреждён I/O & WAL snapshot #{id}: {error}"))?;
    Ok(StoredIoWalSnapshot { summary, snapshot })
}

fn ensure_same_target(
    older: &IoWalSnapshotSummary,
    newer: &IoWalSnapshotSummary,
) -> Result<(), String> {
    if older.connection_id != newer.connection_id
        || older.host != newer.host
        || older.port != newer.port
        || older.database_oid != newer.database_oid
        || older.database != newer.database
        || older.username != newer.username
        || older.sslmode != newer.sslmode
    {
        return Err(crate::i18n::text(
            "I/O & WAL snapshots относятся к разным PostgreSQL targets",
            "I/O & WAL snapshots belong to different PostgreSQL targets",
        )
        .into());
    }
    Ok(())
}

fn same_reset(older: Option<&str>, newer: Option<&str>) -> bool {
    matches!((older, newer), (Some(left), Some(right)) if left == right)
}

fn counter_delta(older: Option<i64>, newer: Option<i64>) -> Option<i64> {
    let (older, newer) = (older?, newer?);
    (newer >= older).then_some(newer - older)
}

fn float_counter_delta(older: Option<f64>, newer: Option<f64>) -> Option<f64> {
    let (older, newer) = (older?, newer?);
    if !older.is_finite() || !newer.is_finite() {
        return None;
    }
    (newer + 1e-9 >= older).then_some((newer - older).max(0.0))
}

fn signed_gauge_delta(older: Option<i64>, newer: Option<i64>) -> Option<i64> {
    Some(newer?.saturating_sub(older?))
}

fn rate_per_hour(delta: Option<f64>, elapsed_seconds: f64) -> Option<f64> {
    let delta = delta?;
    (elapsed_seconds > 0.0).then_some(delta * 3600.0 / elapsed_seconds)
}

fn relation_name(row: &IoWalRelationSize) -> String {
    format!("{}.{}", row.schema_name, row.relation_name)
}

fn compare_relation_sizes(
    older: &[IoWalRelationSize],
    newer: &[IoWalRelationSize],
) -> Vec<IoWalRelationSizeChange> {
    let older_map: HashMap<u32, &IoWalRelationSize> =
        older.iter().map(|row| (row.relation_oid, row)).collect();
    let newer_map: HashMap<u32, &IoWalRelationSize> =
        newer.iter().map(|row| (row.relation_oid, row)).collect();
    let mut ids: HashSet<u32> = older_map.keys().copied().collect();
    ids.extend(newer_map.keys().copied());
    let mut result = Vec::new();

    for oid in ids {
        let old = older_map.get(&oid).copied();
        let new = newer_map.get(&oid).copied();
        let same_identity = matches!((old, new), (Some(a), Some(b)) if a.schema_name == b.schema_name && a.relation_name == b.relation_name && a.relation_type == b.relation_type);
        let state = match (old, new, same_identity) {
            (Some(_), Some(_), true) => "comparable",
            (Some(_), Some(_), false) => "oid_identity_changed",
            (Some(_), None, _) => "sampled_only_older",
            (None, Some(_), _) => "sampled_only_newer",
            _ => continue,
        };
        let delta = |old: Option<i64>, new: Option<i64>| {
            if same_identity {
                signed_gauge_delta(old, new)
            } else {
                None
            }
        };
        result.push(IoWalRelationSizeChange {
            relation_oid: oid,
            older_name: old.map(relation_name),
            newer_name: new.map(relation_name),
            sample_state: state.into(),
            older_heap_size_bytes: old.map(|v| v.heap_size_bytes),
            newer_heap_size_bytes: new.map(|v| v.heap_size_bytes),
            heap_size_delta_bytes: delta(
                old.map(|v| v.heap_size_bytes),
                new.map(|v| v.heap_size_bytes),
            ),
            older_indexes_size_bytes: old.map(|v| v.indexes_size_bytes),
            newer_indexes_size_bytes: new.map(|v| v.indexes_size_bytes),
            indexes_size_delta_bytes: delta(
                old.map(|v| v.indexes_size_bytes),
                new.map(|v| v.indexes_size_bytes),
            ),
            older_toast_size_bytes: old.map(|v| v.toast_size_bytes),
            newer_toast_size_bytes: new.map(|v| v.toast_size_bytes),
            toast_size_delta_bytes: delta(
                old.map(|v| v.toast_size_bytes),
                new.map(|v| v.toast_size_bytes),
            ),
            older_total_size_bytes: old.map(|v| v.total_size_bytes),
            newer_total_size_bytes: new.map(|v| v.total_size_bytes),
            total_size_delta_bytes: delta(
                old.map(|v| v.total_size_bytes),
                new.map(|v| v.total_size_bytes),
            ),
        });
    }

    result.sort_by(|left, right| {
        right
            .total_size_delta_bytes
            .unwrap_or(0)
            .abs()
            .cmp(&left.total_size_delta_bytes.unwrap_or(0).abs())
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_distinguishes_absent_reset_from_real_reset() {
        assert_eq!(
            history_section_status(true, Some("r1"), Some("r1"), true),
            "comparable"
        );
        assert_eq!(
            history_section_status(true, Some("r1"), Some("r2"), true),
            "reset"
        );
        assert_eq!(
            history_section_status(true, None, None, true),
            "unknown_reset"
        );
        assert_eq!(
            history_section_status(false, None, None, true),
            "unavailable"
        );
        assert_eq!(
            history_section_status(true, Some("r1"), Some("r1"), false),
            "source_changed"
        );
    }

    #[test]
    fn workload_rate_rejects_counter_decrease_and_nonfinite() {
        assert_eq!(history_counter_rate(Some(10), Some(20), 3600.0), Some(10.0));
        assert_eq!(history_counter_rate(Some(20), Some(10), 3600.0), None);
        assert_eq!(history_float_rate(Some(f64::NAN), Some(2.0), 3600.0), None);
    }

    #[test]
    fn counter_delta_rejects_reset() {
        assert_eq!(counter_delta(Some(10), Some(15)), Some(5));
        assert_eq!(counter_delta(Some(15), Some(10)), None);
    }

    #[test]
    fn float_delta_rejects_nonfinite_values() {
        assert_eq!(float_counter_delta(Some(1.0), Some(3.0)), Some(2.0));
        assert_eq!(float_counter_delta(Some(f64::NAN), Some(3.0)), None);
        assert_eq!(float_counter_delta(Some(1.0), Some(f64::INFINITY)), None);
        assert_eq!(
            float_counter_delta(Some(f64::NEG_INFINITY), Some(3.0)),
            None
        );
    }

    #[test]
    fn reset_must_be_known_and_equal() {
        assert!(same_reset(Some("a"), Some("a")));
        assert!(!same_reset(Some("a"), Some("b")));
        assert!(!same_reset(None, None));
    }
}
