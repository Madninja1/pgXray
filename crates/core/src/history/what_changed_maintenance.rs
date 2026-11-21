use super::what_changed::WhatChangedPeriod;
use super::DiagnosticStore;
use crate::{
    autovacuum::{AutovacuumSnapshot, AutovacuumTable},
    xid::{XidRelationAge, XidSnapshot},
};
use rusqlite::params;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const MAX_AUTOVACUUM_RELATION_CHANGES: usize = 100;
const MAX_XID_RELATION_CHANGES: usize = 100;

#[derive(Clone, Debug)]
pub(super) struct MaintenanceTargetIdentity {
    pub connection_name: String,
    pub host: String,
    pub port: i64,
    pub database: String,
    pub database_oid: i64,
    pub username: String,
    pub server_version: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaintenancePeriodQuality {
    pub state: String,
    pub snapshot_count: usize,
    pub snapshot_ids: Vec<i64>,
    pub safe_interval_count: usize,
    pub unsafe_interval_count: usize,
    pub safe_elapsed_seconds: f64,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutovacuumRelationPeriodMetrics {
    pub relation_oid: u32,
    pub relation_name: String,
    pub first_observed_at_ms: i64,
    pub last_observed_at_ms: i64,
    pub safe_intervals: usize,
    pub safe_elapsed_seconds: f64,
    pub inserts_per_hour: Option<f64>,
    pub updates_per_hour: Option<f64>,
    pub deletes_per_hour: Option<f64>,
    pub hot_update_pct: Option<f64>,
    pub vacuum_cycles: i64,
    pub analyze_cycles: i64,
    pub latest_live_tuples: i64,
    pub latest_dead_tuples: i64,
    pub latest_dead_trigger_used_pct: Option<f64>,
    pub latest_analyze_trigger_used_pct: Option<f64>,
    pub latest_total_size_bytes: i64,
    pub latest_toast_dead_tuples: Option<i64>,
    pub autovacuum_enabled: bool,
    pub effective_vacuum_threshold: f64,
    pub effective_vacuum_scale_factor: f64,
    pub effective_insert_threshold: f64,
    pub effective_insert_scale_factor: f64,
    pub effective_analyze_threshold: f64,
    pub effective_analyze_scale_factor: f64,
    pub effective_vacuum_cost_limit: i64,
    pub effective_vacuum_cost_delay_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedAutovacuumRelationChange {
    pub relation_oid: u32,
    pub relation_name: String,
    pub state: String,
    pub evidence_level: String,
    pub observed_at_ms: i64,
    pub baseline: Option<AutovacuumRelationPeriodMetrics>,
    pub incident: Option<AutovacuumRelationPeriodMetrics>,
    pub policy_changed: bool,
    pub policy_changes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedAutovacuumComparison {
    pub baseline_quality: MaintenancePeriodQuality,
    pub incident_quality: MaintenancePeriodQuality,
    pub relation_changes: Vec<WhatChangedAutovacuumRelationChange>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct XidPeriodSummary {
    pub snapshot_count: usize,
    pub snapshot_ids: Vec<i64>,
    pub state: String,
    pub first_observed_at_ms: Option<i64>,
    pub last_observed_at_ms: Option<i64>,
    pub first_database_xid_age: Option<i64>,
    pub latest_database_xid_age: Option<i64>,
    pub database_xid_age_delta: Option<i64>,
    pub xid_per_hour: Option<f64>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct XidRelationPeriodMetrics {
    pub relation_oid: u32,
    pub relation_name: String,
    pub first_observed_at_ms: i64,
    pub last_observed_at_ms: i64,
    pub first_xid_age: i64,
    pub latest_xid_age: i64,
    pub xid_age_delta: i64,
    pub latest_pct_of_freeze_max: Option<f64>,
    pub latest_effective_freeze_max_age: i64,
    pub latest_total_size_bytes: i64,
    pub latest_freeze_max_source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedXidRelationChange {
    pub relation_oid: u32,
    pub relation_name: String,
    pub state: String,
    pub evidence_level: String,
    pub observed_at_ms: i64,
    pub baseline: Option<XidRelationPeriodMetrics>,
    pub incident: Option<XidRelationPeriodMetrics>,
    pub freeze_policy_changed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct WhatChangedXidComparison {
    pub baseline: XidPeriodSummary,
    pub incident: XidPeriodSummary,
    pub relation_changes: Vec<WhatChangedXidRelationChange>,
    pub limitations: Vec<String>,
}

pub(super) struct MaintenanceComparisonBundle {
    pub autovacuum: WhatChangedAutovacuumComparison,
    pub xid: WhatChangedXidComparison,
    pub targets: Vec<MaintenanceTargetIdentity>,
}

struct StoredAutovacuumPeriodSnapshot {
    id: i64,
    target: MaintenanceTargetIdentity,
    collection_started_at_ms: i64,
    stats_reset: Option<String>,
    snapshot: AutovacuumSnapshot,
}

struct StoredXidPeriodSnapshot {
    id: i64,
    target: MaintenanceTargetIdentity,
    collection_started_at_ms: i64,
    snapshot: XidSnapshot,
}

#[derive(Default)]
struct AutovacuumAccumulator {
    relation_name: String,
    first_observed_at_ms: i64,
    last_observed_at_ms: i64,
    safe_intervals: usize,
    safe_elapsed_seconds: f64,
    inserts_delta: i64,
    updates_delta: i64,
    deletes_delta: i64,
    hot_updates_delta: i64,
    vacuum_cycles: i64,
    analyze_cycles: i64,
    latest: Option<AutovacuumTable>,
}

struct AutovacuumPeriodAggregate {
    target: Option<MaintenanceTargetIdentity>,
    quality: MaintenancePeriodQuality,
    relations: HashMap<u32, AutovacuumRelationPeriodMetrics>,
}

struct XidPeriodAggregate {
    target: Option<MaintenanceTargetIdentity>,
    summary: XidPeriodSummary,
    relations: HashMap<u32, XidRelationPeriodMetrics>,
}

impl DiagnosticStore {
    pub(super) fn compare_what_changed_maintenance(
        &mut self,
        connection_id: i64,
        baseline: &WhatChangedPeriod,
        incident: &WhatChangedPeriod,
    ) -> Result<MaintenanceComparisonBundle, String> {
        let baseline_autovacuum = self.autovacuum_period_aggregate(connection_id, baseline)?;
        let incident_autovacuum = self.autovacuum_period_aggregate(connection_id, incident)?;
        let baseline_xid = self.xid_period_aggregate(connection_id, baseline)?;
        let incident_xid = self.xid_period_aggregate(connection_id, incident)?;

        let mut targets = Vec::new();
        for target in [
            baseline_autovacuum.target.as_ref(),
            incident_autovacuum.target.as_ref(),
            baseline_xid.target.as_ref(),
            incident_xid.target.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            targets.push(target.clone());
        }

        Ok(MaintenanceComparisonBundle {
            autovacuum: compare_autovacuum_periods(baseline_autovacuum, incident_autovacuum),
            xid: compare_xid_periods(baseline_xid, incident_xid),
            targets,
        })
    }

    fn autovacuum_period_aggregate(
        &mut self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<AutovacuumPeriodAggregate, String> {
        let (snapshots, skipped_versions) =
            self.load_autovacuum_period_snapshots(connection_id, period)?;
        validate_maintenance_targets(snapshots.iter().map(|snapshot| &snapshot.target))?;

        let target = snapshots.first().map(|snapshot| snapshot.target.clone());
        let mut limitations = Vec::new();
        if skipped_versions > 0 {
            limitations.push(format!("Пропущено Autovacuum snapshots с неподдерживаемой schema_version: {skipped_versions}."));
        }
        let mut accumulators = HashMap::<u32, AutovacuumAccumulator>::new();

        for snapshot in &snapshots {
            let Some(tables) = snapshot.snapshot.tables.data.as_ref() else {
                limitations.push(format!(
                    "Autovacuum snapshot #{} не содержит table section.",
                    snapshot.id,
                ));
                continue;
            };
            for table in tables {
                let entry = accumulators.entry(table.relation_oid).or_default();
                if entry
                    .latest
                    .as_ref()
                    .is_some_and(|old| !same_autovacuum_relation(old, table))
                {
                    // Previous samples with the same OID belong to a different relation identity.
                    *entry = AutovacuumAccumulator::default();
                    limitations.push(format!("Autovacuum relation OID {} сменил identity внутри периода; прежние интервалы исключены.", table.relation_oid));
                }
                if entry.latest.is_none() {
                    entry.relation_name = qualified_autovacuum_name(table);
                    entry.first_observed_at_ms = snapshot.collection_started_at_ms;
                }
                entry.last_observed_at_ms = snapshot.collection_started_at_ms;
                entry.latest = Some(table.clone());
            }
        }

        let mut safe_interval_count = 0usize;
        let mut unsafe_interval_count = 0usize;
        let mut unsafe_reasons = HashSet::<String>::new();
        let mut safe_elapsed_seconds = 0.0;

        for pair in snapshots.windows(2) {
            let older = &pair[0];
            let newer = &pair[1];
            let elapsed_seconds =
                (newer.collection_started_at_ms - older.collection_started_at_ms) as f64 / 1000.0;
            let safe = elapsed_seconds > 0.0
                && older.stats_reset.is_some()
                && older.stats_reset == newer.stats_reset;
            if !safe {
                unsafe_interval_count += 1;
                unsafe_reasons.insert(
                    crate::i18n::text("Некоторые Autovacuum intervals исключены: stats_reset изменился/неизвестен или elapsed time некорректен.", "Some Autovacuum intervals were excluded because stats_reset changed, is unknown, or the elapsed time is invalid.")
                        .to_owned(),
                );
                continue;
            }

            safe_interval_count += 1;
            safe_elapsed_seconds += elapsed_seconds;
            let older_rows = autovacuum_table_map(&older.snapshot);
            let newer_rows = autovacuum_table_map(&newer.snapshot);

            for (oid, newer_table) in newer_rows {
                let Some(older_table) = older_rows.get(&oid) else {
                    continue;
                };
                if !same_autovacuum_relation(older_table, newer_table) {
                    unsafe_reasons.insert(
                        crate::i18n::text("Relation OID был связан с разной relation identity; такой interval не использован для DML rates.", "Relation OID resolved to different relation identities; the interval was excluded from DML rates.")
                            .to_owned(),
                    );
                    continue;
                }
                let Some(inserts) =
                    monotonic_delta(older_table.tuples_inserted, newer_table.tuples_inserted)
                else {
                    continue;
                };
                let Some(updates) =
                    monotonic_delta(older_table.tuples_updated, newer_table.tuples_updated)
                else {
                    continue;
                };
                let Some(deletes) =
                    monotonic_delta(older_table.tuples_deleted, newer_table.tuples_deleted)
                else {
                    continue;
                };
                let Some(hot) = monotonic_delta(older_table.hot_updates, newer_table.hot_updates)
                else {
                    continue;
                };
                let Some(vacuum_manual) =
                    monotonic_delta(older_table.vacuum_count, newer_table.vacuum_count)
                else {
                    continue;
                };
                let Some(vacuum_auto) =
                    monotonic_delta(older_table.autovacuum_count, newer_table.autovacuum_count)
                else {
                    continue;
                };
                let Some(analyze_manual) =
                    monotonic_delta(older_table.analyze_count, newer_table.analyze_count)
                else {
                    continue;
                };
                let Some(analyze_auto) =
                    monotonic_delta(older_table.autoanalyze_count, newer_table.autoanalyze_count)
                else {
                    continue;
                };

                let entry = accumulators.entry(oid).or_default();
                if older.collection_started_at_ms < entry.first_observed_at_ms
                    || entry
                        .latest
                        .as_ref()
                        .is_none_or(|latest| !same_autovacuum_relation(latest, newer_table))
                {
                    continue;
                }
                entry.safe_intervals += 1;
                entry.safe_elapsed_seconds += elapsed_seconds;
                entry.inserts_delta = entry.inserts_delta.saturating_add(inserts);
                entry.updates_delta = entry.updates_delta.saturating_add(updates);
                entry.deletes_delta = entry.deletes_delta.saturating_add(deletes);
                entry.hot_updates_delta = entry.hot_updates_delta.saturating_add(hot);
                entry.vacuum_cycles = entry
                    .vacuum_cycles
                    .saturating_add(vacuum_manual.saturating_add(vacuum_auto));
                entry.analyze_cycles = entry
                    .analyze_cycles
                    .saturating_add(analyze_manual.saturating_add(analyze_auto));
            }
        }

        limitations.extend(unsafe_reasons);
        if snapshots.len() < 2 {
            limitations.push(
                crate::i18n::text("Для Autovacuum DML/cycle rates нужно минимум два snapshots внутри периода. Latest relation state всё ещё доступен.", "Autovacuum DML and cycle rates require at least two snapshots in the period; the latest relation state remains available.")
                    .into(),
            );
        }
        limitations.push(
            "Dead tuples, trigger usage и relation size — snapshot estimates/state. DML rates и maintenance cycles рассчитываются только по safe cumulative-counter intervals."
                .into(),
        );
        limitations.push(
            "Если cumulative relation counters уменьшились внутри safe global interval, этот relation interval не участвует в DML/cycle rates."
                .into(),
        );
        limitations.push(
            "TOAST в этом уровне What Changed показывается только как latest dead-tuple context родительской relation; детальный TOAST investigation остаётся в Table Health."
                .into(),
        );

        let relations_without_rates = accumulators
            .values()
            .filter(|entry| entry.safe_intervals == 0)
            .count();
        if relations_without_rates > 0 {
            limitations.push(format!("Для {relations_without_rates} relations не найдено безопасных интервалов DML; доступен только latest state."));
        }
        let state = if snapshots.is_empty() {
            "insufficient"
        } else if snapshots.len() < 2
            || unsafe_interval_count > 0
            || safe_interval_count == 0
            || relations_without_rates > 0
            || skipped_versions > 0
        {
            "limited"
        } else {
            "usable"
        }
        .to_owned();

        let relations = accumulators
            .into_iter()
            .filter_map(|(oid, accumulator)| {
                let latest = accumulator.latest?;
                let hours = accumulator.safe_elapsed_seconds / 3600.0;
                let updates = accumulator.updates_delta as f64;
                Some((
                    oid,
                    AutovacuumRelationPeriodMetrics {
                        relation_oid: oid,
                        relation_name: accumulator.relation_name,
                        first_observed_at_ms: accumulator.first_observed_at_ms,
                        last_observed_at_ms: accumulator.last_observed_at_ms,
                        safe_intervals: accumulator.safe_intervals,
                        safe_elapsed_seconds: accumulator.safe_elapsed_seconds,
                        inserts_per_hour: (hours > 0.0)
                            .then_some(accumulator.inserts_delta as f64 / hours),
                        updates_per_hour: (hours > 0.0)
                            .then_some(accumulator.updates_delta as f64 / hours),
                        deletes_per_hour: (hours > 0.0)
                            .then_some(accumulator.deletes_delta as f64 / hours),
                        hot_update_pct: (accumulator.updates_delta > 0)
                            .then_some(accumulator.hot_updates_delta as f64 / updates * 100.0),
                        vacuum_cycles: accumulator.vacuum_cycles,
                        analyze_cycles: accumulator.analyze_cycles,
                        latest_live_tuples: latest.live_tuples,
                        latest_dead_tuples: latest.dead_tuples,
                        latest_dead_trigger_used_pct: latest.dead_trigger_used_pct,
                        latest_analyze_trigger_used_pct: latest.analyze_trigger_used_pct,
                        latest_total_size_bytes: latest.total_size_bytes,
                        latest_toast_dead_tuples: latest
                            .toast
                            .as_ref()
                            .map(|toast| toast.dead_tuples),
                        autovacuum_enabled: latest.autovacuum_enabled,
                        effective_vacuum_threshold: latest.effective_vacuum_threshold,
                        effective_vacuum_scale_factor: latest.effective_vacuum_scale_factor,
                        effective_insert_threshold: latest.effective_insert_threshold,
                        effective_insert_scale_factor: latest.effective_insert_scale_factor,
                        effective_analyze_threshold: latest.effective_analyze_threshold,
                        effective_analyze_scale_factor: latest.effective_analyze_scale_factor,
                        effective_vacuum_cost_limit: latest.effective_vacuum_cost_limit,
                        effective_vacuum_cost_delay_ms: latest.effective_vacuum_cost_delay_ms,
                    },
                ))
            })
            .collect();

        Ok(AutovacuumPeriodAggregate {
            target,
            quality: MaintenancePeriodQuality {
                state,
                snapshot_count: snapshots.len(),
                snapshot_ids: snapshots.iter().map(|snapshot| snapshot.id).collect(),
                safe_interval_count,
                unsafe_interval_count,
                safe_elapsed_seconds,
                limitations,
            },
            relations,
        })
    }

    fn load_autovacuum_period_snapshots(
        &mut self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<(Vec<StoredAutovacuumPeriodSnapshot>, usize), String> {
        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT
                    id, schema_version, connection_name, host, port, database_name,
                    database_oid, username, server_version, collection_started_at_ms,
                    stats_reset, snapshot_json
                FROM autovacuum_snapshots
                WHERE connection_id = ?1
                  AND collection_started_at_ms >= ?2
                  AND collection_started_at_ms <= ?3
                ORDER BY collection_started_at_ms ASC, id ASC
                "#,
            )
            .map_err(|error| {
                format!("Не удалось подготовить Autovacuum period history: {error}")
            })?;
        let mut rows = statement
            .query(params![connection_id, period.from_ms, period.to_ms])
            .map_err(|error| format!("Не удалось прочитать Autovacuum period history: {error}"))?;
        let mut result = Vec::new();
        let mut skipped_versions = 0usize;

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("Не удалось прочитать Autovacuum period row: {error}"))?
        {
            let id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let schema_version: i64 = row.get(1).map_err(|error| error.to_string())?;
            if schema_version != 1 {
                skipped_versions += 1;
                continue;
            }
            let snapshot_json: String = row.get(11).map_err(|error| error.to_string())?;
            let snapshot =
                serde_json::from_str::<AutovacuumSnapshot>(&snapshot_json).map_err(|error| {
                    format!("Не удалось разобрать Autovacuum snapshot #{id}: {error}")
                })?;
            result.push(StoredAutovacuumPeriodSnapshot {
                id,
                target: MaintenanceTargetIdentity {
                    connection_name: row.get(2).map_err(|error| error.to_string())?,
                    host: row.get(3).map_err(|error| error.to_string())?,
                    port: row.get(4).map_err(|error| error.to_string())?,
                    database: row.get(5).map_err(|error| error.to_string())?,
                    database_oid: row.get(6).map_err(|error| error.to_string())?,
                    username: row.get(7).map_err(|error| error.to_string())?,
                    server_version: row.get(8).map_err(|error| error.to_string())?,
                },
                collection_started_at_ms: row.get(9).map_err(|error| error.to_string())?,
                stats_reset: row.get(10).map_err(|error| error.to_string())?,
                snapshot,
            });
        }
        Ok((result, skipped_versions))
    }

    fn xid_period_aggregate(
        &mut self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<XidPeriodAggregate, String> {
        let (snapshots, skipped_versions) =
            self.load_xid_period_snapshots(connection_id, period)?;
        validate_maintenance_targets(snapshots.iter().map(|snapshot| &snapshot.target))?;
        let target = snapshots.first().map(|snapshot| snapshot.target.clone());
        let mut limitations = Vec::new();
        if skipped_versions > 0 {
            limitations.push(format!(
                "Пропущено XID snapshots с неподдерживаемой schema_version: {skipped_versions}."
            ));
        }

        let first = snapshots.first();
        let last = snapshots.last();
        let first_database_xid_age =
            first.map(|snapshot| snapshot.snapshot.overview.database_xid_age);
        let latest_database_xid_age =
            last.map(|snapshot| snapshot.snapshot.overview.database_xid_age);
        let xid_per_hour = xid_rate_per_hour(&snapshots);
        let database_xid_age_delta = if xid_per_hour.is_some() {
            first_database_xid_age
                .zip(latest_database_xid_age)
                .map(|(older, newer)| newer.saturating_sub(older))
        } else {
            None
        };
        if snapshots.len() < 2 {
            limitations.push(
                crate::i18n::text("Для XID consumption rate нужно минимум два snapshots внутри периода. Latest freeze state всё ещё доступен.", "XID consumption rate requires at least two snapshots in the period; the latest freeze state remains available.")
                    .into(),
            );
        } else if xid_per_hour.is_none() {
            limitations.push(
                crate::i18n::text("XID consumption rate не рассчитан: current_xid не удалось безопасно сравнить или elapsed time некорректен.", "XID consumption rate is unavailable: current_xid could not be compared safely or the elapsed time is invalid.")
                    .into(),
            );
        }

        let mut relation_points = HashMap::<u32, Vec<(i64, XidRelationAge)>>::new();
        for snapshot in &snapshots {
            let Some(relations) = snapshot.snapshot.relations.data.as_ref() else {
                limitations.push(format!(
                    "XID snapshot #{} не содержит relations section.",
                    snapshot.id
                ));
                continue;
            };
            for relation in relations {
                relation_points
                    .entry(relation.relation_oid)
                    .or_default()
                    .push((snapshot.collection_started_at_ms, relation.clone()));
            }
        }

        let mut identity_changes = 0usize;
        let relations = relation_points
            .into_iter()
            .filter_map(|(oid, points)| {
                let (first_at, first_relation) = points.first()?.clone();
                let (last_at, last_relation) = points.last()?.clone();
                if points.iter().any(|(_, relation)| {
                    relation.schema_name != first_relation.schema_name
                        || relation.relation_name != first_relation.relation_name
                        || relation.relation_type != first_relation.relation_type
                }) {
                    identity_changes += 1;
                    return None;
                }
                Some((
                    oid,
                    XidRelationPeriodMetrics {
                        relation_oid: oid,
                        relation_name: qualified_xid_name(&last_relation),
                        first_observed_at_ms: first_at,
                        last_observed_at_ms: last_at,
                        first_xid_age: first_relation.xid_age,
                        latest_xid_age: last_relation.xid_age,
                        xid_age_delta: last_relation.xid_age.saturating_sub(first_relation.xid_age),
                        latest_pct_of_freeze_max: last_relation.pct_of_freeze_max,
                        latest_effective_freeze_max_age: last_relation.effective_freeze_max_age,
                        latest_total_size_bytes: last_relation.total_size_bytes,
                        latest_freeze_max_source: last_relation.freeze_max_source,
                    },
                ))
            })
            .collect();

        if identity_changes > 0 {
            limitations.push(format!("Для {identity_changes} XID relations обнаружена смена identity внутри периода; сравнение age исключено."));
        }
        let state = if snapshots.is_empty() {
            "insufficient"
        } else if snapshots.len() == 1
            || xid_per_hour.is_none()
            || skipped_versions > 0
            || identity_changes > 0
        {
            "limited"
        } else {
            "usable"
        }
        .to_owned();

        limitations.push(
            "XID age — safety/freeze context. Рост age сам по себе не означает неисправность; VACUUM может естественно продвигать freeze horizon скачком."
                .into(),
        );

        Ok(XidPeriodAggregate {
            target,
            summary: XidPeriodSummary {
                snapshot_count: snapshots.len(),
                snapshot_ids: snapshots.iter().map(|snapshot| snapshot.id).collect(),
                state,
                first_observed_at_ms: first.map(|snapshot| snapshot.collection_started_at_ms),
                last_observed_at_ms: last.map(|snapshot| snapshot.collection_started_at_ms),
                first_database_xid_age,
                latest_database_xid_age,
                database_xid_age_delta,
                xid_per_hour,
                limitations,
            },
            relations,
        })
    }

    fn load_xid_period_snapshots(
        &mut self,
        connection_id: i64,
        period: &WhatChangedPeriod,
    ) -> Result<(Vec<StoredXidPeriodSnapshot>, usize), String> {
        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT
                    id, schema_version, connection_name, host, port, database_name,
                    database_oid, username, server_version, collection_started_at_ms,
                    snapshot_json
                FROM xid_snapshots
                WHERE connection_id = ?1
                  AND collection_started_at_ms >= ?2
                  AND collection_started_at_ms <= ?3
                ORDER BY collection_started_at_ms ASC, id ASC
                "#,
            )
            .map_err(|error| format!("Не удалось подготовить XID period history: {error}"))?;
        let mut rows = statement
            .query(params![connection_id, period.from_ms, period.to_ms])
            .map_err(|error| format!("Не удалось прочитать XID period history: {error}"))?;
        let mut result = Vec::new();
        let mut skipped_versions = 0usize;

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("Не удалось прочитать XID period row: {error}"))?
        {
            let id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let schema_version: i64 = row.get(1).map_err(|error| error.to_string())?;
            if !(1..=2).contains(&schema_version) {
                skipped_versions += 1;
                continue;
            }
            let snapshot_json: String = row.get(10).map_err(|error| error.to_string())?;
            let snapshot = serde_json::from_str::<XidSnapshot>(&snapshot_json)
                .map_err(|error| format!("Не удалось разобрать XID snapshot #{id}: {error}"))?;
            result.push(StoredXidPeriodSnapshot {
                id,
                target: MaintenanceTargetIdentity {
                    connection_name: row.get(2).map_err(|error| error.to_string())?,
                    host: row.get(3).map_err(|error| error.to_string())?,
                    port: row.get(4).map_err(|error| error.to_string())?,
                    database: row.get(5).map_err(|error| error.to_string())?,
                    database_oid: row.get(6).map_err(|error| error.to_string())?,
                    username: row.get(7).map_err(|error| error.to_string())?,
                    server_version: row.get(8).map_err(|error| error.to_string())?,
                },
                collection_started_at_ms: row.get(9).map_err(|error| error.to_string())?,
                snapshot,
            });
        }
        Ok((result, skipped_versions))
    }
}

fn compare_autovacuum_periods(
    baseline: AutovacuumPeriodAggregate,
    incident: AutovacuumPeriodAggregate,
) -> WhatChangedAutovacuumComparison {
    let mut keys = baseline
        .relations
        .keys()
        .chain(incident.relations.keys())
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    keys.sort_unstable();

    let mut relation_changes = keys
        .into_iter()
        .map(|oid| {
            let baseline_metrics = baseline.relations.get(&oid).cloned();
            let incident_metrics = incident.relations.get(&oid).cloned();
            let relation_name = incident_metrics
                .as_ref()
                .map(|value| value.relation_name.clone())
                .or_else(|| {
                    baseline_metrics
                        .as_ref()
                        .map(|value| value.relation_name.clone())
                })
                .unwrap_or_else(|| format!("OID {oid}"));
            let identity_changed = baseline_metrics
                .as_ref()
                .zip(incident_metrics.as_ref())
                .is_some_and(|(left, right)| left.relation_name != right.relation_name);
            let state = if identity_changed {
                "oid_identity_changed"
            } else {
                match (&baseline_metrics, &incident_metrics) {
                    (Some(_), Some(_)) => "observed_in_both",
                    (None, Some(_)) => "observed_in_incident_only",
                    (Some(_), None) => "observed_in_baseline_only",
                    (None, None) => unreachable!(),
                }
            }
            .to_owned();
            let confirmed_rates = !identity_changed
                && baseline_metrics
                    .as_ref()
                    .zip(incident_metrics.as_ref())
                    .is_some_and(|(left, right)| {
                        left.safe_intervals > 0
                            && right.safe_intervals > 0
                            && (left.inserts_per_hour != right.inserts_per_hour
                                || left.updates_per_hour != right.updates_per_hour
                                || left.deletes_per_hour != right.deletes_per_hour
                                || left.vacuum_cycles != right.vacuum_cycles
                                || left.analyze_cycles != right.analyze_cycles)
                    });
            let policy_changes = if identity_changed {
                Vec::new()
            } else {
                baseline_metrics
                    .as_ref()
                    .zip(incident_metrics.as_ref())
                    .map(|(left, right)| autovacuum_policy_changes(left, right))
                    .unwrap_or_default()
            };
            let observed_at_ms = incident_metrics
                .as_ref()
                .map(|value| value.last_observed_at_ms)
                .or_else(|| {
                    baseline_metrics
                        .as_ref()
                        .map(|value| value.last_observed_at_ms)
                })
                .unwrap_or_default();

            WhatChangedAutovacuumRelationChange {
                relation_oid: oid,
                relation_name,
                state,
                evidence_level: if confirmed_rates || !policy_changes.is_empty() {
                    "confirmed_change"
                } else {
                    "observed"
                }
                .to_owned(),
                observed_at_ms,
                baseline: baseline_metrics,
                incident: incident_metrics,
                policy_changed: !policy_changes.is_empty(),
                policy_changes,
            }
        })
        .collect::<Vec<_>>();

    relation_changes.sort_by(|left, right| {
        autovacuum_change_score(right)
            .partial_cmp(&autovacuum_change_score(left))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.relation_name.cmp(&right.relation_name))
    });
    relation_changes.truncate(MAX_AUTOVACUUM_RELATION_CHANGES);

    let mut limitations = Vec::new();
    limitations.extend(baseline.quality.limitations.iter().cloned());
    limitations.extend(incident.quality.limitations.iter().cloned());
    limitations.push(
        "Autovacuum comparison показывает workload/maintenance/state changes. Он не считает высокий dead tuple count доказательством плохого autovacuum и не устанавливает причинность query regressions."
            .into(),
    );

    WhatChangedAutovacuumComparison {
        baseline_quality: baseline.quality,
        incident_quality: incident.quality,
        relation_changes,
        limitations,
    }
}

fn compare_xid_periods(
    baseline: XidPeriodAggregate,
    incident: XidPeriodAggregate,
) -> WhatChangedXidComparison {
    let mut keys = baseline
        .relations
        .keys()
        .chain(incident.relations.keys())
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    keys.sort_unstable();

    let mut relation_changes = keys
        .into_iter()
        .map(|oid| {
            let baseline_metrics = baseline.relations.get(&oid).cloned();
            let incident_metrics = incident.relations.get(&oid).cloned();
            let relation_name = incident_metrics
                .as_ref()
                .map(|value| value.relation_name.clone())
                .or_else(|| {
                    baseline_metrics
                        .as_ref()
                        .map(|value| value.relation_name.clone())
                })
                .unwrap_or_else(|| format!("OID {oid}"));
            let identity_changed = baseline_metrics
                .as_ref()
                .zip(incident_metrics.as_ref())
                .is_some_and(|(left, right)| left.relation_name != right.relation_name);
            let state = if identity_changed {
                "oid_identity_changed"
            } else {
                match (&baseline_metrics, &incident_metrics) {
                    (Some(_), Some(_)) => "observed_in_both",
                    (None, Some(_)) => "observed_in_incident_only",
                    (Some(_), None) => "observed_in_baseline_only",
                    (None, None) => unreachable!(),
                }
            }
            .to_owned();
            let freeze_policy_changed = !identity_changed
                && baseline_metrics
                    .as_ref()
                    .zip(incident_metrics.as_ref())
                    .is_some_and(|(left, right)| {
                        left.latest_effective_freeze_max_age
                            != right.latest_effective_freeze_max_age
                            || left.latest_freeze_max_source != right.latest_freeze_max_source
                    });
            let observed_at_ms = incident_metrics
                .as_ref()
                .map(|value| value.last_observed_at_ms)
                .or_else(|| {
                    baseline_metrics
                        .as_ref()
                        .map(|value| value.last_observed_at_ms)
                })
                .unwrap_or_default();

            WhatChangedXidRelationChange {
                relation_oid: oid,
                relation_name,
                state,
                evidence_level: if !identity_changed
                    && baseline_metrics
                        .as_ref()
                        .zip(incident_metrics.as_ref())
                        .is_some_and(|(left, right)| {
                            left.latest_xid_age != right.latest_xid_age || freeze_policy_changed
                        }) {
                    "confirmed_change"
                } else {
                    "observed"
                }
                .to_owned(),
                observed_at_ms,
                baseline: baseline_metrics,
                incident: incident_metrics,
                freeze_policy_changed,
            }
        })
        .collect::<Vec<_>>();

    relation_changes.sort_by(|left, right| {
        xid_change_score(right)
            .partial_cmp(&xid_change_score(left))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.relation_name.cmp(&right.relation_name))
    });
    relation_changes.truncate(MAX_XID_RELATION_CHANGES);

    let mut limitations = Vec::new();
    limitations.extend(baseline.summary.limitations.iter().cloned());
    limitations.extend(incident.summary.limitations.iter().cloned());
    limitations.push(
        "XID comparison — safety context. Temporal relation между XID/autovacuum/workload evidence не доказывает, что одно событие вызвало другое."
            .into(),
    );

    WhatChangedXidComparison {
        baseline: baseline.summary,
        incident: incident.summary,
        relation_changes,
        limitations,
    }
}

fn autovacuum_table_map(snapshot: &AutovacuumSnapshot) -> HashMap<u32, &AutovacuumTable> {
    snapshot
        .tables
        .data
        .as_ref()
        .map(|tables| {
            tables
                .iter()
                .map(|table| (table.relation_oid, table))
                .collect()
        })
        .unwrap_or_default()
}

fn same_autovacuum_relation(left: &AutovacuumTable, right: &AutovacuumTable) -> bool {
    left.schema_name == right.schema_name
        && left.relation_name == right.relation_name
        && left.relation_type == right.relation_type
}

fn qualified_autovacuum_name(table: &AutovacuumTable) -> String {
    format!("{}.{}", table.schema_name, table.relation_name)
}

fn qualified_xid_name(relation: &XidRelationAge) -> String {
    format!("{}.{}", relation.schema_name, relation.relation_name)
}

fn monotonic_delta(older: i64, newer: i64) -> Option<i64> {
    (newer >= older).then_some(newer - older)
}

fn autovacuum_policy_changes(
    baseline: &AutovacuumRelationPeriodMetrics,
    incident: &AutovacuumRelationPeriodMetrics,
) -> Vec<String> {
    let mut changes = Vec::new();
    push_f64_change(
        &mut changes,
        "autovacuum_vacuum_threshold",
        baseline.effective_vacuum_threshold,
        incident.effective_vacuum_threshold,
    );
    push_f64_change(
        &mut changes,
        "autovacuum_vacuum_scale_factor",
        baseline.effective_vacuum_scale_factor,
        incident.effective_vacuum_scale_factor,
    );
    push_f64_change(
        &mut changes,
        "autovacuum_vacuum_insert_threshold",
        baseline.effective_insert_threshold,
        incident.effective_insert_threshold,
    );
    push_f64_change(
        &mut changes,
        "autovacuum_vacuum_insert_scale_factor",
        baseline.effective_insert_scale_factor,
        incident.effective_insert_scale_factor,
    );
    push_f64_change(
        &mut changes,
        "autovacuum_analyze_threshold",
        baseline.effective_analyze_threshold,
        incident.effective_analyze_threshold,
    );
    push_f64_change(
        &mut changes,
        "autovacuum_analyze_scale_factor",
        baseline.effective_analyze_scale_factor,
        incident.effective_analyze_scale_factor,
    );
    if baseline.effective_vacuum_cost_limit != incident.effective_vacuum_cost_limit {
        changes.push(format!(
            "autovacuum_vacuum_cost_limit: {} → {}",
            baseline.effective_vacuum_cost_limit, incident.effective_vacuum_cost_limit,
        ));
    }
    push_f64_change(
        &mut changes,
        "autovacuum_vacuum_cost_delay_ms",
        baseline.effective_vacuum_cost_delay_ms,
        incident.effective_vacuum_cost_delay_ms,
    );
    if baseline.autovacuum_enabled != incident.autovacuum_enabled {
        changes.push(format!(
            "autovacuum_enabled: {} → {}",
            baseline.autovacuum_enabled, incident.autovacuum_enabled,
        ));
    }
    changes
}

fn push_f64_change(changes: &mut Vec<String>, label: &str, older: f64, newer: f64) {
    if (older - newer).abs() > 1e-9 {
        changes.push(format!("{label}: {older:.6} → {newer:.6}"));
    }
}

fn autovacuum_change_score(change: &WhatChangedAutovacuumRelationChange) -> f64 {
    let latest = change.incident.as_ref().or(change.baseline.as_ref());
    let trigger = latest
        .and_then(|value| value.latest_dead_trigger_used_pct)
        .unwrap_or(0.0);
    let dml = latest
        .map(|value| {
            value.inserts_per_hour.unwrap_or(0.0)
                + value.updates_per_hour.unwrap_or(0.0)
                + value.deletes_per_hour.unwrap_or(0.0)
        })
        .unwrap_or(0.0);
    trigger * 1_000_000.0 + dml + if change.policy_changed { 1e12 } else { 0.0 }
}

fn xid_change_score(change: &WhatChangedXidRelationChange) -> f64 {
    change
        .incident
        .as_ref()
        .or(change.baseline.as_ref())
        .and_then(|value| value.latest_pct_of_freeze_max)
        .unwrap_or(0.0)
}

fn xid_rate_per_hour(snapshots: &[StoredXidPeriodSnapshot]) -> Option<f64> {
    let first = snapshots.first()?;
    let last = snapshots.last()?;
    if first.id == last.id {
        return None;
    }
    // Postmaster epoch and recovery state must be known and stable for every observation.
    let epoch = first.snapshot.overview.postmaster_start_time.as_deref()?;
    if epoch.is_empty() || first.snapshot.overview.in_recovery != Some(false) {
        return None;
    }
    let mut previous_xid = None;
    for point in snapshots {
        let overview = &point.snapshot.overview;
        if overview.postmaster_start_time.as_deref() != Some(epoch)
            || overview.in_recovery != Some(false)
        {
            return None;
        }
        let xid = overview.current_xid.parse::<u64>().ok()?;
        if previous_xid.is_some_and(|previous| xid < previous) {
            return None;
        }
        previous_xid = Some(xid);
    }
    let older = first.snapshot.overview.current_xid.parse::<u64>().ok()?;
    let newer = previous_xid?;
    let elapsed_seconds =
        (last.collection_started_at_ms - first.collection_started_at_ms) as f64 / 1000.0;
    (elapsed_seconds > 0.0).then_some((newer - older) as f64 / elapsed_seconds * 3600.0)
}

fn validate_maintenance_targets<'a>(
    targets: impl Iterator<Item = &'a MaintenanceTargetIdentity>,
) -> Result<(), String> {
    let mut targets = targets;
    let Some(first) = targets.next() else {
        return Ok(());
    };
    for current in targets {
        if first.host != current.host
            || first.port != current.port
            || first.database_oid != current.database_oid
            || first.username != current.username
        {
            return Err(format!(
                "Maintenance period содержит разные PostgreSQL targets/visibility contexts: {}:{}/oid {}/user {} vs {}:{}/oid {}/user {}",
                first.host,
                first.port,
                first.database_oid,
                first.username,
                current.host,
                current.port,
                current.database_oid,
                current.username,
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(scale: f64) -> AutovacuumRelationPeriodMetrics {
        AutovacuumRelationPeriodMetrics {
            relation_oid: 42,
            relation_name: "public.orders".into(),
            first_observed_at_ms: 1,
            last_observed_at_ms: 2,
            safe_intervals: 1,
            safe_elapsed_seconds: 60.0,
            inserts_per_hour: Some(1.0),
            updates_per_hour: Some(2.0),
            deletes_per_hour: Some(3.0),
            hot_update_pct: Some(50.0),
            vacuum_cycles: 0,
            analyze_cycles: 0,
            latest_live_tuples: 100,
            latest_dead_tuples: 10,
            latest_dead_trigger_used_pct: Some(20.0),
            latest_analyze_trigger_used_pct: Some(30.0),
            latest_total_size_bytes: 1000,
            latest_toast_dead_tuples: None,
            autovacuum_enabled: true,
            effective_vacuum_threshold: 50.0,
            effective_vacuum_scale_factor: scale,
            effective_insert_threshold: 1000.0,
            effective_insert_scale_factor: 0.2,
            effective_analyze_threshold: 50.0,
            effective_analyze_scale_factor: 0.1,
            effective_vacuum_cost_limit: 200,
            effective_vacuum_cost_delay_ms: 2.0,
        }
    }

    #[test]
    fn policy_change_detects_scale_factor() {
        let changes = autovacuum_policy_changes(&metrics(0.2), &metrics(0.05));
        assert!(changes
            .iter()
            .any(|change| change.contains("vacuum_scale_factor")));
    }

    #[test]
    fn monotonic_counter_rejects_reset() {
        assert_eq!(monotonic_delta(10, 15), Some(5));
        assert_eq!(monotonic_delta(15, 10), None);
    }
}
