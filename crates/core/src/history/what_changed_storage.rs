use super::{
    index_health::StoredIndexHealthSnapshot, io_wal::StoredIoWalSnapshot,
    what_changed::WhatChangedPeriod, DiagnosticStore,
};
use crate::{index_health::IndexHealthIndex, io_wal::IoWalRelationSize};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const MAX_RELATION_SIZE_CHANGES: usize = 100;
const MAX_INDEX_HEALTH_CHANGES: usize = 100;

#[derive(Clone, Debug, Serialize)]
pub struct StoragePeriodQuality {
    pub state: String,
    pub snapshot_count: usize,
    pub snapshot_ids: Vec<i64>,
    pub database_safe_intervals: usize,
    pub wal_safe_intervals: usize,
    pub checkpoint_safe_intervals: usize,
    pub blocks_hit_observed_seconds: f64,
    pub wal_bytes_observed_seconds: f64,
    pub checkpoints_timed_observed_seconds: f64,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct StoragePeriodMetrics {
    pub temp_files_per_hour: Option<f64>,
    pub temp_bytes_per_hour: Option<f64>,
    pub database_blocks_read_per_hour: Option<f64>,
    pub database_blocks_hit_per_hour: Option<f64>,
    pub wal_bytes_per_hour: Option<f64>,
    pub wal_records_per_hour: Option<f64>,
    pub wal_buffers_full_per_hour: Option<f64>,
    pub wal_write_time_ms_per_hour: Option<f64>,
    pub wal_sync_time_ms_per_hour: Option<f64>,
    pub checkpoints_timed_per_hour: Option<f64>,
    pub checkpoints_requested_per_hour: Option<f64>,
    pub checkpoint_write_time_ms_per_hour: Option<f64>,
    pub checkpoint_sync_time_ms_per_hour: Option<f64>,
    pub checkpoint_buffers_written_per_hour: Option<f64>,
    pub latest_database_size_bytes: Option<i64>,
    pub checkpoint_timeout: Option<String>,
    pub checkpoint_completion_target: Option<f64>,
    pub max_wal_size: Option<String>,
    pub min_wal_size: Option<String>,
    pub track_io_timing: Option<bool>,
    pub track_wal_io_timing: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedRelationSizeChange {
    pub relation_oid: u32,
    pub relation_name: String,
    pub state: String,
    pub evidence_level: String,
    pub baseline_total_size_bytes: Option<i64>,
    pub incident_total_size_bytes: Option<i64>,
    pub total_size_delta_bytes: Option<i64>,
    pub heap_size_delta_bytes: Option<i64>,
    pub indexes_size_delta_bytes: Option<i64>,
    pub toast_size_delta_bytes: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedStorageComparison {
    pub baseline_quality: StoragePeriodQuality,
    pub incident_quality: StoragePeriodQuality,
    pub baseline: StoragePeriodMetrics,
    pub incident: StoragePeriodMetrics,
    pub relation_size_changes: Vec<WhatChangedRelationSizeChange>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedIndexHealthChange {
    pub index_oid: u32,
    pub relation_oid: u32,
    pub relation_name: String,
    pub index_name: String,
    pub state: String,
    pub evidence_level: String,
    pub baseline_classifications: Vec<String>,
    pub incident_classifications: Vec<String>,
    pub scan_delta: Option<i64>,
    pub relation_dml_delta: Option<i64>,
    pub size_delta_bytes: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedIndexHealthComparison {
    pub state: String,
    pub baseline_snapshot_id: Option<i64>,
    pub incident_snapshot_id: Option<i64>,
    pub stats_period_continuous: bool,
    pub changes: Vec<WhatChangedIndexHealthChange>,
    /// Observed scan-counter growth, not a diagnostic classification or a per-period rate.
    pub usage_changes: Vec<WhatChangedIndexHealthChange>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct StorageTargetIdentity {
    pub connection_name: String,
    pub host: String,
    pub port: i64,
    pub database: String,
    pub database_oid: i64,
    pub username: String,
    pub server_version: String,
}

pub(super) struct StorageComparisonBundle {
    pub storage: WhatChangedStorageComparison,
    pub index_health: WhatChangedIndexHealthComparison,
    pub targets: Vec<StorageTargetIdentity>,
}

#[derive(Default)]
struct MetricAccumulator {
    delta: f64,
    elapsed_seconds: f64,
}

impl MetricAccumulator {
    fn add(&mut self, delta: f64, elapsed_seconds: f64) {
        self.delta += delta;
        self.elapsed_seconds += elapsed_seconds;
    }

    fn per_hour(&self) -> Option<f64> {
        (self.elapsed_seconds > 0.0).then_some(self.delta * 3600.0 / self.elapsed_seconds)
    }
}

struct StoragePeriodAggregate {
    quality: StoragePeriodQuality,
    metrics: StoragePeriodMetrics,
    latest_relations: Vec<IoWalRelationSize>,
    latest_relations_truncated: bool,
    latest_relations_available: bool,
}

impl DiagnosticStore {
    pub(super) fn compare_what_changed_storage(
        &self,
        connection_id: i64,
        baseline: &WhatChangedPeriod,
        incident: &WhatChangedPeriod,
    ) -> Result<StorageComparisonBundle, String> {
        let baseline_storage = self.storage_period_aggregate(connection_id, baseline)?;
        let incident_storage = self.storage_period_aggregate(connection_id, incident)?;
        let index_health = self.compare_index_health_periods(connection_id, baseline, incident)?;

        let mut limitations = Vec::new();
        if baseline_storage.latest_relations_truncated
            || incident_storage.latest_relations_truncated
        {
            limitations.push(
                crate::i18n::text("Relation-size sample truncated: отсутствие relation в одном периоде не доказывает CREATE/DROP.", "Relation-size sample is truncated: an absent relation in one period does not prove CREATE/DROP.")
                    .into(),
            );
        }

        let relation_size_changes = if baseline_storage.latest_relations_available
            && incident_storage.latest_relations_available
        {
            compare_relation_size_periods(
                &baseline_storage.latest_relations,
                &incident_storage.latest_relations,
            )
        } else {
            limitations.push(
                crate::i18n::text("Relation-size сравнение пропущено: секция отсутствует хотя бы в одном периоде.", "Relation-size comparison skipped: the section is missing in at least one period.")
                    .into(),
            );
            Vec::new()
        };

        let mut targets = Vec::new();
        let baseline_io =
            self.io_wal_snapshots_in_period(connection_id, baseline.from_ms, baseline.to_ms)?;
        let incident_io =
            self.io_wal_snapshots_in_period(connection_id, incident.from_ms, incident.to_ms)?;
        for rows in [&baseline_io, &incident_io] {
            if let Some(row) = rows.first() {
                targets.push(StorageTargetIdentity {
                    connection_name: row.summary.connection_name.clone(),
                    host: row.summary.host.clone(),
                    port: row.summary.port,
                    database: row.summary.database.clone(),
                    database_oid: row.summary.database_oid,
                    username: row.summary.username.clone(),
                    server_version: row.summary.server_version.clone(),
                });
            }
        }
        let baseline_indexes =
            self.index_health_snapshots_in_period(connection_id, baseline.from_ms, baseline.to_ms)?;
        let incident_indexes =
            self.index_health_snapshots_in_period(connection_id, incident.from_ms, incident.to_ms)?;
        for rows in [&baseline_indexes, &incident_indexes] {
            if let Some(row) = rows.first() {
                targets.push(StorageTargetIdentity {
                    connection_name: row.summary.connection_name.clone(),
                    host: row.summary.host.clone(),
                    port: row.summary.port,
                    database: row.summary.database.clone(),
                    database_oid: row.summary.database_oid,
                    username: row.summary.username.clone(),
                    server_version: row.summary.server_version.clone(),
                });
            }
        }

        Ok(StorageComparisonBundle {
            storage: WhatChangedStorageComparison {
                baseline_quality: baseline_storage.quality,
                incident_quality: incident_storage.quality,
                baseline: baseline_storage.metrics,
                incident: incident_storage.metrics,
                relation_size_changes,
                limitations,
            },
            index_health,
            targets,
        })
    }

    fn storage_period_aggregate(
        &self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<StoragePeriodAggregate, String> {
        let snapshots =
            self.io_wal_snapshots_in_period(connection_id, period.from_ms, period.to_ms)?;
        validate_storage_targets(&snapshots)?;

        let mut temp_files = MetricAccumulator::default();
        let mut temp_bytes = MetricAccumulator::default();
        let mut db_reads = MetricAccumulator::default();
        let mut db_hits = MetricAccumulator::default();
        let mut wal_bytes = MetricAccumulator::default();
        let mut wal_records = MetricAccumulator::default();
        let mut wal_buffers_full = MetricAccumulator::default();
        let mut wal_write_time = MetricAccumulator::default();
        let mut wal_sync_time = MetricAccumulator::default();
        let mut checkpoint_timed = MetricAccumulator::default();
        let mut checkpoint_requested = MetricAccumulator::default();
        let mut checkpoint_write = MetricAccumulator::default();
        let mut checkpoint_sync = MetricAccumulator::default();
        let mut checkpoint_buffers_written = MetricAccumulator::default();
        let mut database_safe_intervals = 0usize;
        let mut wal_safe_intervals = 0usize;
        let mut checkpoint_safe_intervals = 0usize;
        let mut limitations = HashSet::new();

        for pair in snapshots.windows(2) {
            let older = &pair[0];
            let newer = &pair[1];
            let elapsed = (newer.summary.collection_started_at_ms
                - older.summary.collection_started_at_ms) as f64
                / 1000.0;
            if elapsed <= 0.0 {
                limitations
                    .insert("Некорректный elapsed interval исключён из storage rates.".to_owned());
                continue;
            }

            if same_reset(
                older.summary.database_stats_reset.as_deref(),
                newer.summary.database_stats_reset.as_deref(),
            ) {
                if let (Some(a), Some(b)) = (
                    older.snapshot.database_stats.data.as_ref(),
                    newer.snapshot.database_stats.data.as_ref(),
                ) {
                    if let Some(value) = monotonic_delta(a.temp_files, b.temp_files) {
                        temp_files.add(value as f64, elapsed);
                    }
                    if let Some(value) = monotonic_delta(a.temp_bytes, b.temp_bytes) {
                        temp_bytes.add(value as f64, elapsed);
                    }
                    if let Some(value) = monotonic_delta(a.blks_read, b.blks_read) {
                        db_reads.add(value as f64, elapsed);
                    }
                    if let Some(value) = monotonic_delta(a.blks_hit, b.blks_hit) {
                        db_hits.add(value as f64, elapsed);
                    }
                    database_safe_intervals += 1;
                }
            } else {
                limitations.insert("pg_stat_database reset изменился/неизвестен; часть temp/block intervals исключена.".to_owned());
            }

            if same_reset(
                older.summary.wal_stats_reset.as_deref(),
                newer.summary.wal_stats_reset.as_deref(),
            ) {
                if let (Some(a), Some(b)) = (
                    older.snapshot.wal.data.as_ref(),
                    newer.snapshot.wal.data.as_ref(),
                ) {
                    if let Some(value) = monotonic_f64_delta(a.wal_bytes, b.wal_bytes) {
                        wal_bytes.add(value, elapsed);
                    }
                    if let Some(value) = monotonic_delta(a.wal_records, b.wal_records) {
                        wal_records.add(value as f64, elapsed);
                    }
                    if let Some(value) = monotonic_delta(a.wal_buffers_full, b.wal_buffers_full) {
                        wal_buffers_full.add(value as f64, elapsed);
                    }
                    if let Some(value) =
                        monotonic_f64_delta(a.wal_write_time_ms, b.wal_write_time_ms)
                    {
                        wal_write_time.add(value, elapsed);
                    }
                    if let Some(value) = monotonic_f64_delta(a.wal_sync_time_ms, b.wal_sync_time_ms)
                    {
                        wal_sync_time.add(value, elapsed);
                    }
                    wal_safe_intervals += 1;
                }
            } else {
                limitations.insert(
                    crate::i18n::text("pg_stat_wal reset изменился/недоступен; часть WAL intervals исключена.", "pg_stat_wal reset changed or is unavailable; some WAL intervals were excluded.")
                        .to_owned(),
                );
            }

            let older_checkpoint = older.snapshot.checkpoints.data.as_ref();
            let newer_checkpoint = newer.snapshot.checkpoints.data.as_ref();
            let same_checkpoint_source = matches!((older_checkpoint, newer_checkpoint), (Some(a), Some(b)) if a.source == b.source);
            if same_checkpoint_source
                && same_reset(
                    older.summary.checkpoint_stats_reset.as_deref(),
                    newer.summary.checkpoint_stats_reset.as_deref(),
                )
            {
                if let (Some(a), Some(b)) = (older_checkpoint, newer_checkpoint) {
                    if let Some(value) = monotonic_delta(a.timed, b.timed) {
                        checkpoint_timed.add(value as f64, elapsed);
                    }
                    if let Some(value) = monotonic_delta(a.requested, b.requested) {
                        checkpoint_requested.add(value as f64, elapsed);
                    }
                    if let Some(value) = monotonic_f64_delta(a.write_time_ms, b.write_time_ms) {
                        checkpoint_write.add(value, elapsed);
                    }
                    if let Some(value) = monotonic_f64_delta(a.sync_time_ms, b.sync_time_ms) {
                        checkpoint_sync.add(value, elapsed);
                    }
                    if let Some(value) = monotonic_delta(a.buffers_written, b.buffers_written) {
                        checkpoint_buffers_written.add(value as f64, elapsed);
                    }
                    checkpoint_safe_intervals += 1;
                }
            } else {
                limitations.insert("Checkpoint source/reset изменился/неизвестен; часть checkpoint intervals исключена.".to_owned());
            }
        }

        let latest = snapshots.last();
        let latest_relations = latest
            .and_then(|row| row.snapshot.relation_sizes.data.as_ref())
            .map(|sizes| sizes.relations.clone())
            .unwrap_or_default();
        let latest_relations_truncated = latest
            .and_then(|row| row.snapshot.relation_sizes.data.as_ref())
            .map(|sizes| sizes.truncated)
            .unwrap_or(false);
        if latest.is_some_and(|row| row.snapshot.relation_sizes.data.is_none()) {
            limitations.insert(
                crate::i18n::text("Relation-size секция недоступна: отсутствие таблиц в выборке не доказывает CREATE/DROP.", "Relation-size section is unavailable: missing tables in the sample do not prove CREATE/DROP.")
                    .to_owned(),
            );
        }

        if snapshots.len() < 2 {
            limitations.insert(
                crate::i18n::text(
                    "Для storage rates нужно минимум два I/O & WAL snapshots внутри периода.",
                    "Storage rates require at least two I/O & WAL snapshots within the period.",
                )
                .into(),
            );
        }

        let any_safe = database_safe_intervals + wal_safe_intervals + checkpoint_safe_intervals > 0;
        let state = if snapshots.is_empty() {
            "insufficient"
        } else if snapshots.len() < 2 || !any_safe || !limitations.is_empty() {
            "limited"
        } else {
            "usable"
        };

        Ok(StoragePeriodAggregate {
            quality: StoragePeriodQuality {
                state: state.into(),
                snapshot_count: snapshots.len(),
                snapshot_ids: snapshots.iter().map(|row| row.summary.id).collect(),
                database_safe_intervals,
                wal_safe_intervals,
                checkpoint_safe_intervals,
                blocks_hit_observed_seconds: db_hits.elapsed_seconds,
                wal_bytes_observed_seconds: wal_bytes.elapsed_seconds,
                checkpoints_timed_observed_seconds: checkpoint_timed.elapsed_seconds,
                limitations: limitations.into_iter().collect(),
            },
            metrics: StoragePeriodMetrics {
                temp_files_per_hour: temp_files.per_hour(),
                temp_bytes_per_hour: temp_bytes.per_hour(),
                database_blocks_read_per_hour: db_reads.per_hour(),
                database_blocks_hit_per_hour: db_hits.per_hour(),
                wal_bytes_per_hour: wal_bytes.per_hour(),
                wal_records_per_hour: wal_records.per_hour(),
                wal_buffers_full_per_hour: wal_buffers_full.per_hour(),
                wal_write_time_ms_per_hour: wal_write_time.per_hour(),
                wal_sync_time_ms_per_hour: wal_sync_time.per_hour(),
                checkpoints_timed_per_hour: checkpoint_timed.per_hour(),
                checkpoints_requested_per_hour: checkpoint_requested.per_hour(),
                checkpoint_write_time_ms_per_hour: checkpoint_write.per_hour(),
                checkpoint_sync_time_ms_per_hour: checkpoint_sync.per_hour(),
                checkpoint_buffers_written_per_hour: checkpoint_buffers_written.per_hour(),
                latest_database_size_bytes: latest.and_then(|row| row.summary.database_size_bytes),
                checkpoint_timeout: latest
                    .map(|row| row.snapshot.overview.checkpoint_timeout.clone()),
                checkpoint_completion_target: latest
                    .map(|row| row.snapshot.overview.checkpoint_completion_target),
                max_wal_size: latest.map(|row| row.snapshot.overview.max_wal_size.clone()),
                min_wal_size: latest.map(|row| row.snapshot.overview.min_wal_size.clone()),
                track_io_timing: latest.map(|row| row.snapshot.overview.track_io_timing),
                track_wal_io_timing: latest
                    .and_then(|row| row.snapshot.overview.track_wal_io_timing),
            },
            latest_relations,
            latest_relations_truncated,
            latest_relations_available: latest
                .is_some_and(|row| row.snapshot.relation_sizes.data.is_some()),
        })
    }

    fn compare_index_health_periods(
        &self,
        connection_id: i64,
        baseline: &WhatChangedPeriod,
        incident: &WhatChangedPeriod,
    ) -> Result<WhatChangedIndexHealthComparison, String> {
        let baseline_rows =
            self.index_health_snapshots_in_period(connection_id, baseline.from_ms, baseline.to_ms)?;
        let incident_rows =
            self.index_health_snapshots_in_period(connection_id, incident.from_ms, incident.to_ms)?;
        validate_index_targets(baseline_rows.iter().chain(incident_rows.iter()))?;

        let baseline_latest = baseline_rows.last();
        let incident_latest = incident_rows.last();
        let Some(incident_latest) = incident_latest else {
            return Ok(WhatChangedIndexHealthComparison {
                state: "insufficient".into(),
                baseline_snapshot_id: baseline_latest.map(|row| row.summary.id),
                incident_snapshot_id: None,
                stats_period_continuous: false,
                changes: Vec::new(),
                usage_changes: Vec::new(),
                limitations: vec!["Incident period не содержит Index Health snapshot.".into()],
            });
        };

        let Some(baseline_latest) = baseline_latest else {
            return Ok(WhatChangedIndexHealthComparison {
                state: "limited".into(),
                baseline_snapshot_id: None,
                incident_snapshot_id: Some(incident_latest.summary.id),
                stats_period_continuous: false,
                changes: incident_latest.snapshot.collection.indexes.iter()
                    .filter(|row| interesting_classifications(&row.classifications))
                    .take(MAX_INDEX_HEALTH_CHANGES)
                    .map(|row| index_change(None, Some(row), false))
                    .collect(),
                usage_changes: Vec::new(),
                limitations: vec!["Baseline period не содержит Index Health snapshot; показан только incident context.".into()],
            });
        };

        let continuous = same_reset(
            baseline_latest.summary.stats_reset.as_deref(),
            incident_latest.summary.stats_reset.as_deref(),
        );
        let mut limitations = Vec::new();
        if !continuous {
            limitations.push("Index Health stats_reset между baseline/incident различается; scan/DML deltas не рассчитаны.".into());
        }
        if baseline_latest.summary.sample_truncated || incident_latest.summary.sample_truncated {
            limitations.push(
                crate::i18n::text(
                    "Index Health sample truncated; sampled-only state не доказывает CREATE/DROP.",
                    "Index Health sample truncated; sampled-only state does not prove CREATE/DROP.",
                )
                .into(),
            );
        }

        let old_map: HashMap<u32, &IndexHealthIndex> = baseline_latest
            .snapshot
            .collection
            .indexes
            .iter()
            .map(|row| (row.index_oid, row))
            .collect();
        let new_map: HashMap<u32, &IndexHealthIndex> = incident_latest
            .snapshot
            .collection
            .indexes
            .iter()
            .map(|row| (row.index_oid, row))
            .collect();
        let mut ids: HashSet<u32> = old_map.keys().copied().collect();
        ids.extend(new_map.keys().copied());
        let mut changes = Vec::new();
        let mut usage_changes = Vec::new();
        for id in ids {
            let old = old_map.get(&id).copied();
            let new = new_map.get(&id).copied();
            let classifications_changed =
                old.map(|row| &row.classifications) != new.map(|row| &row.classifications);
            let concerning = new
                .map(|row| interesting_classifications(&row.classifications))
                .unwrap_or(false);
            let change = index_change(old, new, continuous);
            // Scan activity is not itself evidence that an index is unhealthy.
            // Only identical indexes with continuous, nondecreasing counters qualify.
            if change.scan_delta.is_some_and(|delta| delta > 0) {
                usage_changes.push(change.clone());
            }
            if classifications_changed || concerning || old.is_none() || new.is_none() {
                changes.push(change);
            }
        }
        changes.sort_by(|a, b| {
            b.relation_dml_delta
                .unwrap_or(0)
                .cmp(&a.relation_dml_delta.unwrap_or(0))
        });
        changes.truncate(MAX_INDEX_HEALTH_CHANGES);
        usage_changes.sort_by(|a, b| {
            b.scan_delta
                .unwrap_or(0)
                .cmp(&a.scan_delta.unwrap_or(0))
                .then_with(|| a.index_oid.cmp(&b.index_oid))
        });
        usage_changes.truncate(MAX_INDEX_HEALTH_CHANGES);

        Ok(WhatChangedIndexHealthComparison {
            state: if limitations.is_empty() {
                "usable".into()
            } else {
                "limited".into()
            },
            baseline_snapshot_id: Some(baseline_latest.summary.id),
            incident_snapshot_id: Some(incident_latest.summary.id),
            stats_period_continuous: continuous,
            changes,
            usage_changes,
            limitations,
        })
    }
}

fn validate_storage_targets(rows: &[StoredIoWalSnapshot]) -> Result<(), String> {
    let Some(first) = rows.first() else {
        return Ok(());
    };

    for row in rows.iter().skip(1) {
        if row.summary.host != first.summary.host
            || row.summary.port != first.summary.port
            || row.summary.database_oid != first.summary.database_oid
        {
            return Err("I/O & WAL period содержит snapshots разных PostgreSQL targets".into());
        }
    }

    Ok(())
}

fn validate_index_targets<'a>(
    rows: impl Iterator<Item = &'a StoredIndexHealthSnapshot>,
) -> Result<(), String> {
    let mut target: Option<(&str, i64, i64)> = None;

    for row in rows {
        let current = (
            row.summary.host.as_str(),
            row.summary.port,
            row.summary.database_oid,
        );
        if let Some(expected) = target {
            if expected != current {
                return Err(crate::i18n::text(
                    "Index Health periods содержат snapshots разных PostgreSQL targets",
                    "Index Health periods contain snapshots from different PostgreSQL targets",
                )
                .into());
            }
        } else {
            target = Some(current);
        }
    }

    Ok(())
}

fn same_reset(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(a), Some(b)) if a == b)
}

fn monotonic_delta(older: i64, newer: i64) -> Option<i64> {
    (newer >= older).then_some(newer - older)
}

fn monotonic_f64_delta(older: f64, newer: f64) -> Option<f64> {
    if !older.is_finite() || !newer.is_finite() {
        return None;
    }
    (newer + 1e-9 >= older).then_some((newer - older).max(0.0))
}

fn qualified_relation(row: &IoWalRelationSize) -> String {
    format!("{}.{}", row.schema_name, row.relation_name)
}

fn compare_relation_size_periods(
    older: &[IoWalRelationSize],
    newer: &[IoWalRelationSize],
) -> Vec<WhatChangedRelationSizeChange> {
    let older_map: HashMap<u32, &IoWalRelationSize> =
        older.iter().map(|row| (row.relation_oid, row)).collect();
    let newer_map: HashMap<u32, &IoWalRelationSize> =
        newer.iter().map(|row| (row.relation_oid, row)).collect();
    let mut relation_oids: HashSet<u32> = older_map.keys().copied().collect();
    relation_oids.extend(newer_map.keys().copied());
    let mut changes = Vec::new();

    for relation_oid in relation_oids {
        let old = older_map.get(&relation_oid).copied();
        let new = newer_map.get(&relation_oid).copied();
        let same_identity = matches!(
            (old, new),
            (Some(left), Some(right))
                if left.schema_name == right.schema_name
                    && left.relation_name == right.relation_name
                    && left.relation_type == right.relation_type
        );
        let state = match (old, new, same_identity) {
            (Some(_), Some(_), true) => "comparable",
            (Some(_), Some(_), false) => "oid_identity_changed",
            (Some(_), None, _) => "sampled_only_baseline",
            (None, Some(_), _) => "sampled_only_incident",
            _ => continue,
        };
        let gauge_delta = |old_value: Option<i64>, new_value: Option<i64>| {
            if !same_identity {
                return None;
            }
            Some(new_value?.saturating_sub(old_value?))
        };
        let total_size_delta_bytes = gauge_delta(
            old.map(|row| row.total_size_bytes),
            new.map(|row| row.total_size_bytes),
        );

        if same_identity
            && total_size_delta_bytes == Some(0)
            && gauge_delta(
                old.map(|row| row.heap_size_bytes),
                new.map(|row| row.heap_size_bytes),
            ) == Some(0)
            && gauge_delta(
                old.map(|row| row.indexes_size_bytes),
                new.map(|row| row.indexes_size_bytes),
            ) == Some(0)
            && gauge_delta(
                old.map(|row| row.toast_size_bytes),
                new.map(|row| row.toast_size_bytes),
            ) == Some(0)
        {
            continue;
        }

        changes.push(WhatChangedRelationSizeChange {
            relation_oid,
            relation_name: new.or(old).map(qualified_relation).unwrap_or_default(),
            state: state.into(),
            evidence_level: if same_identity {
                "confirmed_change".into()
            } else {
                "observed".into()
            },
            baseline_total_size_bytes: old.map(|row| row.total_size_bytes),
            incident_total_size_bytes: new.map(|row| row.total_size_bytes),
            total_size_delta_bytes,
            heap_size_delta_bytes: gauge_delta(
                old.map(|row| row.heap_size_bytes),
                new.map(|row| row.heap_size_bytes),
            ),
            indexes_size_delta_bytes: gauge_delta(
                old.map(|row| row.indexes_size_bytes),
                new.map(|row| row.indexes_size_bytes),
            ),
            toast_size_delta_bytes: gauge_delta(
                old.map(|row| row.toast_size_bytes),
                new.map(|row| row.toast_size_bytes),
            ),
        });
    }

    changes.sort_by(|left, right| {
        right
            .total_size_delta_bytes
            .unwrap_or(0)
            .abs()
            .cmp(&left.total_size_delta_bytes.unwrap_or(0).abs())
    });
    changes.truncate(MAX_RELATION_SIZE_CHANGES);
    changes
}

fn interesting_classifications(values: &[String]) -> bool {
    values.iter().any(|value| {
        matches!(
            value.as_str(),
            "suspiciously_unused"
                | "possible_duplicate"
                | "high_write_overhead"
                | "needs_investigation"
        )
    })
}

fn index_change(
    old: Option<&IndexHealthIndex>,
    new: Option<&IndexHealthIndex>,
    continuous: bool,
) -> WhatChangedIndexHealthChange {
    let row = new.or(old).expect("index change requires one row");
    let same_identity = matches!(
        (old, new),
        (Some(left), Some(right))
            if left.relation_oid == right.relation_oid
                && left.schema_name == right.schema_name
                && left.relation_name == right.relation_name
                && left.index_name == right.index_name
    );
    let state = match (old, new, same_identity) {
        (Some(_), Some(_), true) => "comparable",
        (Some(_), Some(_), false) => "oid_identity_changed",
        (Some(_), None, _) => "sampled_only_baseline",
        (None, Some(_), _) => "sampled_only_incident",
        _ => "unknown",
    };
    let counter_delta = |old_value: Option<i64>, new_value: Option<i64>| {
        if !continuous || !same_identity {
            return None;
        }
        let (old_value, new_value) = (old_value?, new_value?);
        (new_value >= old_value).then_some(new_value - old_value)
    };
    let old_dml = old.map(total_index_relation_dml);
    let new_dml = new.map(total_index_relation_dml);

    WhatChangedIndexHealthChange {
        index_oid: row.index_oid,
        relation_oid: row.relation_oid,
        relation_name: format!("{}.{}", row.schema_name, row.relation_name),
        index_name: row.index_name.clone(),
        state: state.into(),
        evidence_level: "observed".into(),
        baseline_classifications: old
            .map(|value| value.classifications.clone())
            .unwrap_or_default(),
        incident_classifications: new
            .map(|value| value.classifications.clone())
            .unwrap_or_default(),
        scan_delta: counter_delta(
            old.map(|value| value.idx_scan),
            new.map(|value| value.idx_scan),
        ),
        relation_dml_delta: counter_delta(old_dml, new_dml),
        size_delta_bytes: if same_identity {
            Some(
                new.expect("same identity requires incident index")
                    .index_size_bytes
                    .saturating_sub(
                        old.expect("same identity requires baseline index")
                            .index_size_bytes,
                    ),
            )
        } else {
            None
        },
    }
}

fn total_index_relation_dml(row: &IndexHealthIndex) -> i64 {
    row.tuples_inserted
        .saturating_add(row.tuples_updated)
        .saturating_add(row.tuples_deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_index(scans: i64) -> IndexHealthIndex {
        IndexHealthIndex {
            index_oid: 42,
            relation_oid: 7,
            schema_name: "public".into(),
            relation_name: "orders".into(),
            index_name: "orders_pkey".into(),
            index_size_bytes: 8192,
            idx_scan: scans,
            idx_tup_read: 0,
            idx_tup_fetch: 0,
            is_valid: true,
            is_ready: true,
            is_primary: true,
            is_unique: true,
            key_columns: vec!["id".into()],
            predicate: None,
            index_definition: "CREATE UNIQUE INDEX orders_pkey ON public.orders USING btree (id)"
                .into(),
            definition_fingerprint: "same".into(),
            estimated_rows: 100,
            tuples_inserted: 0,
            tuples_updated: 0,
            tuples_deleted: 0,
            hot_updates: 0,
            last_analyze: None,
            last_autoanalyze: None,
            classifications: Vec::new(),
        }
    }

    #[test]
    fn index_usage_requires_safe_continuous_counters_and_identical_identity() {
        let old = example_index(6);
        let mut new = example_index(26);
        assert_eq!(
            index_change(Some(&old), Some(&new), true).scan_delta,
            Some(20)
        );
        assert_eq!(index_change(Some(&old), Some(&new), false).scan_delta, None);
        new.idx_scan = 3;
        assert_eq!(index_change(Some(&old), Some(&new), true).scan_delta, None);
        new.idx_scan = 26;
        new.relation_oid += 1;
        assert_eq!(index_change(Some(&old), Some(&new), true).scan_delta, None);
    }

    #[test]
    fn source_reset_must_be_known_and_equal() {
        assert!(same_reset(Some("a"), Some("a")));
        assert!(!same_reset(Some("a"), Some("b")));
        assert!(!same_reset(None, None));
    }

    #[test]
    fn monotonic_float_rejects_nonfinite_values() {
        assert_eq!(monotonic_f64_delta(1.0, 3.0), Some(2.0));
        assert_eq!(monotonic_f64_delta(f64::NAN, 3.0), None);
        assert_eq!(monotonic_f64_delta(1.0, f64::INFINITY), None);
    }

    #[test]
    fn monotonic_counter_rejects_reset() {
        assert_eq!(monotonic_delta(10, 15), Some(5));
        assert_eq!(monotonic_delta(15, 10), None);
    }
}
