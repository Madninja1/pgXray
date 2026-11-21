use super::{unix_time_millis, DiagnosticStore, DEFAULT_HISTORY_LIMIT, MAX_HISTORY_LIMIT};
use crate::{
    connections::SavedConnection,
    xid::{
        self, XidAutovacuumTrigger, XidBackend, XidPolicyCandidate, XidPolicyHistorySample,
        XidPreparedSummary, XidRelationAge, XidRelationPolicyAdvice, XidSection, XidSlot,
        XidSnapshot, XidVacuum,
    },
};
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const XID_SNAPSHOT_SCHEMA_VERSION: i64 = 2;
const XID_SNAPSHOT_MIN_SUPPORTED_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
pub struct CollectedXidSnapshot {
    pub history: XidSnapshotSummary,
    pub snapshot: XidSnapshot,
}

#[derive(Debug, Serialize)]
pub struct XidSnapshotSummary {
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
    pub current_xid: String,
    pub database_xid_age: i64,
    pub oldest_relation: Option<XidRelationSummary>,
    pub unavailable_sections: i64,
    pub stored_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct XidRelationSummary {
    pub relation_oid: i64,
    pub relation_name: String,
    pub xid_age: i64,
    pub effective_freeze_max_age: i64,
}

/// Lightweight, historical context for an XID/MXID chart. The large relation
/// collections stay inside the SQLite snapshots and are not sent to the UI.
#[derive(Debug, Serialize)]
pub struct XidHistoryOverviewPoint {
    pub summary: XidSnapshotSummary,
    pub database_mxid_age: Option<i64>,
    pub autovacuum_freeze_max_age: i64,
    pub autovacuum_multixact_freeze_max_age: Option<i64>,
    pub postmaster_start_time: Option<String>,
    pub in_recovery: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct XidRelationHistoryPoint {
    pub snapshot_id: i64,
    pub schema_version: i64,
    pub collection_started_at_ms: i64,
    pub measured_at: String,
    pub current_xid: String,
    pub postmaster_start_time: Option<String>,
    pub in_recovery: Option<bool>,
    pub stats_reset: Option<String>,
    pub relation: XidRelationAge,
    pub trigger: Option<XidAutovacuumTrigger>,
    pub active_vacuum: Option<XidVacuum>,
}

#[derive(Debug, Serialize)]
pub struct XidRate {
    pub xid_delta: u64,
    pub elapsed_seconds: f64,
    pub xid_per_second: f64,
    pub xid_per_minute: f64,
    pub xid_per_hour: f64,
}

#[derive(Debug, Serialize)]
pub struct XidRelationDelta {
    pub relation_oid: u32,
    pub older_name: Option<String>,
    pub newer_name: Option<String>,
    pub older_xid_age: Option<i64>,
    pub newer_xid_age: Option<i64>,
    pub xid_age_delta: Option<i64>,
    pub older_pct_of_freeze_max: Option<f64>,
    pub newer_pct_of_freeze_max: Option<f64>,
    pub sample_state: String,
}

#[derive(Debug, Serialize)]
pub struct XidBlockerComparison {
    pub older_backend_count: Option<usize>,
    pub newer_backend_count: Option<usize>,
    pub backend_new_in_sample: Vec<String>,
    pub backend_not_in_new_sample: Vec<String>,
    pub older_slot_count: Option<usize>,
    pub newer_slot_count: Option<usize>,
    pub slots_new_in_sample: Vec<String>,
    pub slots_not_in_new_sample: Vec<String>,
    pub older_prepared_count: Option<i64>,
    pub newer_prepared_count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct XidVacuumComparison {
    pub older_active_count: Option<usize>,
    pub newer_active_count: Option<usize>,
    pub older_wraparound_count: Option<usize>,
    pub newer_wraparound_count: Option<usize>,
    pub active_new_in_sample: Vec<String>,
    pub active_not_in_new_sample: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct XidOldestRelationComparison {
    pub older: Option<XidRelationSummary>,
    pub newer: Option<XidRelationSummary>,
    pub xid_age_delta: Option<i64>,
    pub older_configured_headroom: Option<i64>,
    pub newer_configured_headroom: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct XidComparison {
    pub older: XidSnapshotSummary,
    pub newer: XidSnapshotSummary,
    pub elapsed_seconds: f64,
    pub xid_rate: Option<XidRate>,
    pub database_xid_age_delta: i64,
    pub oldest_relation: XidOldestRelationComparison,
    pub relation_changes: Vec<XidRelationDelta>,
    pub blockers: XidBlockerComparison,
    pub vacuums: XidVacuumComparison,
    pub limitations: Vec<String>,
}

struct StoredXidSnapshot {
    summary: XidSnapshotSummary,
    snapshot: XidSnapshot,
}

pub(super) fn init_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS xid_snapshots (
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
                current_xid TEXT NOT NULL,
                database_xid_age INTEGER NOT NULL,
                oldest_relation_oid INTEGER,
                oldest_relation_name TEXT,
                oldest_relation_xid_age INTEGER,
                oldest_relation_freeze_max_age INTEGER,
                unavailable_sections INTEGER NOT NULL,
                stored_at_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_xid_snapshots_connection_time
                ON xid_snapshots (
                    connection_id,
                    collection_started_at_ms DESC,
                    id DESC
                );

            CREATE INDEX IF NOT EXISTS idx_xid_snapshots_target_time
                ON xid_snapshots (
                    host,
                    port,
                    database_oid,
                    collection_started_at_ms DESC
                );
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить XID history schema: {error}"))
}

impl DiagnosticStore {
    pub fn save_xid_snapshot(
        &mut self,
        saved_connection: &SavedConnection,
        snapshot: &XidSnapshot,
        collection_started_at_ms: i64,
        collection_completed_at_ms: i64,
    ) -> Result<XidSnapshotSummary, String> {
        if collection_completed_at_ms < collection_started_at_ms {
            return Err(crate::i18n::text(
                "Время завершения XID snapshot раньше времени начала",
                "XID snapshot finished before it started",
            )
            .into());
        }

        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|error| format!("Не удалось сериализовать XID snapshot: {error}"))?;

        let oldest_relation = oldest_relation_summary(&snapshot.relations);

        let unavailable_sections = unavailable_section_count(snapshot);

        let stored_at_ms = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT INTO xid_snapshots (
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
                    current_xid,
                    database_xid_age,
                    oldest_relation_oid,
                    oldest_relation_name,
                    oldest_relation_xid_age,
                    oldest_relation_freeze_max_age,
                    unavailable_sections,
                    stored_at_ms,
                    snapshot_json
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
                )
                "#,
                params![
                    XID_SNAPSHOT_SCHEMA_VERSION,
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
                    &snapshot.overview.current_xid,
                    snapshot.overview.database_xid_age,
                    oldest_relation
                        .as_ref()
                        .map(|relation| { relation.relation_oid }),
                    oldest_relation
                        .as_ref()
                        .map(|relation| { relation.relation_name.as_str() }),
                    oldest_relation
                        .as_ref()
                        .map(|relation| { relation.xid_age }),
                    oldest_relation
                        .as_ref()
                        .map(|relation| { relation.effective_freeze_max_age }),
                    unavailable_sections,
                    stored_at_ms,
                    snapshot_json,
                ],
            )
            .map_err(|error| format!("Не удалось сохранить XID snapshot: {error}"))?;

        let id = self.connection.last_insert_rowid();

        Ok(XidSnapshotSummary {
            id,
            schema_version: XID_SNAPSHOT_SCHEMA_VERSION,
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
            current_xid: snapshot.overview.current_xid.clone(),
            database_xid_age: snapshot.overview.database_xid_age,
            oldest_relation,
            unavailable_sections,
            stored_at_ms,
        })
    }

    pub fn list_xid_snapshots(
        &mut self,
        connection_id: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<XidSnapshotSummary>, String> {
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
                    summary_select_sql(),
                ))
                .map_err(|error| format!("Не удалось подготовить чтение XID history: {error}"))?;

            let mut rows = statement
                .query(params![connection_id, i64::from(limit),])
                .map_err(|error| format!("Не удалось прочитать XID history: {error}"))?;

            while let Some(row) = rows
                .next()
                .map_err(|error| format!("Не удалось прочитать строку XID history: {error}"))?
            {
                result.push(
                    summary_from_row(row)
                        .map_err(|error| format!("Повреждена строка XID history: {error}"))?,
                );
            }
        } else {
            let mut statement = self
                .connection
                .prepare(&format!(
                    "{} ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?1",
                    summary_select_sql(),
                ))
                .map_err(|error| format!("Не удалось подготовить чтение XID history: {error}"))?;

            let mut rows = statement
                .query(params![i64::from(limit),])
                .map_err(|error| format!("Не удалось прочитать XID history: {error}"))?;

            while let Some(row) = rows
                .next()
                .map_err(|error| format!("Не удалось прочитать строку XID history: {error}"))?
            {
                result.push(
                    summary_from_row(row)
                        .map_err(|error| format!("Повреждена строка XID history: {error}"))?,
                );
            }
        }

        Ok(result)
    }

    /// Read-only, bounded trend from saved XID snapshots. Unlike live XID
    /// diagnostics, this never queries PostgreSQL or allocates an XID.
    pub fn xid_history_overview(
        &mut self,
        connection_id: i64,
        limit: Option<u32>,
    ) -> Result<Vec<XidHistoryOverviewPoint>, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection id для XID history",
                "Invalid connection ID for XID history",
            )
            .into());
        }
        let limit = limit.unwrap_or(120).clamp(1, MAX_HISTORY_LIMIT);
        let summaries = self.list_xid_snapshots(Some(connection_id), Some(limit))?;
        let mut output = Vec::with_capacity(summaries.len());
        for summary in summaries {
            // Reuse the existing schema-version check and JSON deserializer.
            let stored = self.load_xid_snapshot(summary.id)?;
            if stored.summary.connection_id != connection_id {
                return Err(crate::i18n::text(
                    "XID snapshot принадлежит другому подключению",
                    "XID snapshot belongs to a different connection",
                )
                .into());
            }
            let overview = stored.snapshot.overview;
            output.push(XidHistoryOverviewPoint {
                summary: stored.summary,
                database_mxid_age: overview.database_mxid_age,
                autovacuum_freeze_max_age: overview.autovacuum_freeze_max_age,
                autovacuum_multixact_freeze_max_age: overview.autovacuum_multixact_freeze_max_age,
                postmaster_start_time: overview.postmaster_start_time,
                in_recovery: overview.in_recovery,
            });
        }
        Ok(output)
    }

    pub fn xid_relation_history(
        &mut self,
        anchor_snapshot_id: i64,
        relation_oid: u32,
        limit: Option<u32>,
    ) -> Result<Vec<XidRelationHistoryPoint>, String> {
        if anchor_snapshot_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный snapshot id для XID relation history",
                "Invalid snapshot ID for XID relation history",
            )
            .into());
        }

        let anchor = self.load_xid_snapshot(anchor_snapshot_id)?;
        let anchor_relation = anchor
            .snapshot
            .relations
            .data
            .as_ref()
            .and_then(|rows| rows.iter().find(|row| row.relation_oid == relation_oid))
            .ok_or_else(|| {
                format!("Relation OID {relation_oid} отсутствует в опорном XID snapshot")
            })?;
        let anchor_identity = (
            anchor_relation.schema_name.clone(),
            anchor_relation.relation_name.clone(),
            anchor_relation.relation_type.clone(),
        );
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
                    snapshot_json
                FROM xid_snapshots
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
            .map_err(|error| format!("Не удалось подготовить XID relation history: {error}"))?;

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
            .map_err(|error| format!("Не удалось прочитать XID relation history: {error}"))?;

        let mut result = Vec::new();

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("Не удалось прочитать XID relation history row: {error}"))?
        {
            let snapshot_id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let schema_version: i64 = row.get(1).map_err(|error| error.to_string())?;
            let collection_started_at_ms: i64 = row.get(2).map_err(|error| error.to_string())?;
            let measured_at: String = row.get(3).map_err(|error| error.to_string())?;
            let snapshot_json: String = row.get(4).map_err(|error| error.to_string())?;

            if !(XID_SNAPSHOT_MIN_SUPPORTED_SCHEMA_VERSION..=XID_SNAPSHOT_SCHEMA_VERSION)
                .contains(&schema_version)
            {
                continue;
            }

            let snapshot =
                serde_json::from_str::<XidSnapshot>(&snapshot_json).map_err(|error| {
                    format!(
                    "Не удалось разобрать XID snapshot #{snapshot_id} для relation history: {error}"
                )
                })?;

            let Some(relation) = snapshot
                .relations
                .data
                .as_ref()
                .and_then(|relations| {
                    relations
                        .iter()
                        .find(|relation| relation.relation_oid == relation_oid)
                })
                .cloned()
            else {
                continue;
            };

            if (
                relation.schema_name.as_str(),
                relation.relation_name.as_str(),
                relation.relation_type.as_str(),
            ) != (
                anchor_identity.0.as_str(),
                anchor_identity.1.as_str(),
                anchor_identity.2.as_str(),
            ) {
                // OID мог быть использован повторно; не соединяем разные объекты.
                break;
            }

            let trigger = snapshot
                .autovacuum_triggers
                .data
                .as_ref()
                .and_then(|triggers| {
                    triggers
                        .iter()
                        .find(|trigger| trigger.relation_oid == relation_oid)
                })
                .cloned();

            let active_vacuum = snapshot
                .vacuums
                .data
                .as_ref()
                .and_then(|vacuums| {
                    vacuums
                        .iter()
                        .find(|vacuum| vacuum.relation_oid == relation_oid)
                })
                .cloned();

            result.push(XidRelationHistoryPoint {
                snapshot_id,
                schema_version,
                collection_started_at_ms,
                measured_at,
                current_xid: snapshot.overview.current_xid.clone(),
                postmaster_start_time: snapshot.overview.postmaster_start_time.clone(),
                in_recovery: snapshot.overview.in_recovery,
                stats_reset: snapshot.overview.stats_reset.clone(),
                relation,
                trigger,
                active_vacuum,
            });
        }

        Ok(result)
    }

    pub fn xid_relation_policy(
        &mut self,
        anchor_snapshot_id: i64,
        relation_oid: u32,
        limit: Option<u32>,
    ) -> Result<XidRelationPolicyAdvice, String> {
        if anchor_snapshot_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный snapshot id для XID Policy Advisor",
                "Invalid snapshot ID for XID Policy Advisor",
            )
            .into());
        }

        let anchor = self.load_xid_snapshot(anchor_snapshot_id)?;
        let relation = anchor
            .snapshot
            .relations
            .data
            .as_ref()
            .and_then(|relations| {
                relations
                    .iter()
                    .find(|relation| relation.relation_oid == relation_oid)
            })
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Relation OID {relation_oid} не найдена в XID snapshot #{anchor_snapshot_id}"
                )
            })?;

        let trigger = anchor
            .snapshot
            .autovacuum_triggers
            .data
            .as_ref()
            .and_then(|triggers| {
                triggers
                    .iter()
                    .find(|trigger| trigger.relation_oid == relation_oid)
            })
            .cloned();
        let active_vacuum = anchor
            .snapshot
            .vacuums
            .data
            .as_ref()
            .and_then(|vacuums| {
                vacuums
                    .iter()
                    .find(|vacuum| vacuum.relation_oid == relation_oid)
            })
            .cloned();

        let history = self.xid_relation_history(anchor_snapshot_id, relation_oid, limit)?;
        let samples = history
            .iter()
            .map(|point| XidPolicyHistorySample {
                snapshot_id: point.snapshot_id,
                collection_started_at_ms: point.collection_started_at_ms,
                current_xid: point.current_xid.clone(),
                postmaster_start_time: point.postmaster_start_time.clone(),
                in_recovery: point.in_recovery,
                stats_reset: point.stats_reset.clone(),
                relation_xid_age: point.relation.xid_age,
                vacuum_count: point.relation.vacuum_count,
                autovacuum_count: point.relation.autovacuum_count,
            })
            .collect::<Vec<_>>();

        Ok(xid::advise_relation_policy(
            &anchor.snapshot.overview,
            &relation,
            trigger.as_ref(),
            active_vacuum.as_ref(),
            anchor.snapshot.vacuums.error.is_none() && anchor.snapshot.vacuums.data.is_some(),
            &samples,
        ))
    }

    pub fn xid_policy_candidate_context(
        &mut self,
        anchor_snapshot_id: i64,
        relation_oid: u32,
        candidate_id: &str,
        limit: Option<u32>,
    ) -> Result<(XidPolicyCandidate, XidRelationAge, XidSnapshotSummary), String> {
        let advice = self.xid_relation_policy(anchor_snapshot_id, relation_oid, limit)?;
        let candidate = advice
            .candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| {
                format!(
                    "Policy candidate {candidate_id:?} больше не доступен для relation OID {relation_oid} в snapshot #{anchor_snapshot_id}. Пересчитайте Policy Advisor."
                )
            })?;

        let anchor = self.load_xid_snapshot(anchor_snapshot_id)?;
        let relation = anchor
            .snapshot
            .relations
            .data
            .as_ref()
            .and_then(|relations| {
                relations
                    .iter()
                    .find(|relation| relation.relation_oid == relation_oid)
            })
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Relation OID {relation_oid} не найдена в XID snapshot #{anchor_snapshot_id}"
                )
            })?;

        Ok((candidate, relation, anchor.summary))
    }

    pub fn compare_xid_snapshots(
        &mut self,
        first_id: i64,
        second_id: i64,
    ) -> Result<XidComparison, String> {
        if first_id == second_id {
            return Err(crate::i18n::text(
                "Для сравнения выберите два разных XID snapshot",
                "Select two different XID snapshots to compare",
            )
            .into());
        }

        let first = self.load_xid_snapshot(first_id)?;

        let second = self.load_xid_snapshot(second_id)?;

        let (older, newer) =
            if snapshot_order_key(&first.summary) <= snapshot_order_key(&second.summary) {
                (first, second)
            } else {
                (second, first)
            };

        compare_snapshots(older, newer)
    }

    /// Return one stored snapshot only for its original connection. No PostgreSQL query.
    pub fn xid_snapshot(
        &mut self,
        connection_id: i64,
        snapshot_id: i64,
    ) -> Result<CollectedXidSnapshot, String> {
        let stored = self.load_xid_snapshot(snapshot_id)?;
        if stored.summary.connection_id != connection_id {
            return Err(crate::i18n::text(
                "XID snapshot принадлежит другому подключению",
                "XID snapshot belongs to a different connection",
            )
            .into());
        }
        Ok(CollectedXidSnapshot {
            history: stored.summary,
            snapshot: stored.snapshot,
        })
    }

    fn load_xid_snapshot(&mut self, id: i64) -> Result<StoredXidSnapshot, String> {
        let sql = format!("{} WHERE id = ?1", stored_select_sql(),);

        self.connection
            .query_row(&sql, params![id], |row| {
                let summary = summary_from_row(row)?;

                let snapshot_json: String = row.get(24)?;

                Ok((summary, snapshot_json))
            })
            .map_err(|error| format!("XID snapshot #{id} не найден или повреждён: {error}"))
            .and_then(|(summary, snapshot_json)| {
                if !(XID_SNAPSHOT_MIN_SUPPORTED_SCHEMA_VERSION..=XID_SNAPSHOT_SCHEMA_VERSION)
                    .contains(&summary.schema_version)
                {
                    return Err(format!(
                        "XID snapshot #{} имеет schema_version={}, поддерживаются {}..={}",
                        summary.id,
                        summary.schema_version,
                        XID_SNAPSHOT_MIN_SUPPORTED_SCHEMA_VERSION,
                        XID_SNAPSHOT_SCHEMA_VERSION,
                    ));
                }

                let snapshot =
                    serde_json::from_str::<XidSnapshot>(&snapshot_json).map_err(|error| {
                        format!(
                            "Не удалось разобрать сохранённый XID snapshot #{}: {error}",
                            summary.id,
                        )
                    })?;

                Ok(StoredXidSnapshot { summary, snapshot })
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
        collection_started_at_ms,
        collection_completed_at_ms,
        measured_at,
        current_xid,
        database_xid_age,
        oldest_relation_oid,
        oldest_relation_name,
        oldest_relation_xid_age,
        oldest_relation_freeze_max_age,
        unavailable_sections,
        stored_at_ms
    FROM xid_snapshots
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
        current_xid,
        database_xid_age,
        oldest_relation_oid,
        oldest_relation_name,
        oldest_relation_xid_age,
        oldest_relation_freeze_max_age,
        unavailable_sections,
        stored_at_ms,
        snapshot_json
    FROM xid_snapshots
    "#
}

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<XidSnapshotSummary> {
    let oldest_relation_oid: Option<i64> = row.get(18)?;

    let oldest_relation_name: Option<String> = row.get(19)?;

    let oldest_relation_xid_age: Option<i64> = row.get(20)?;

    let oldest_relation_freeze_max_age: Option<i64> = row.get(21)?;

    let oldest_relation = match (
        oldest_relation_oid,
        oldest_relation_name,
        oldest_relation_xid_age,
        oldest_relation_freeze_max_age,
    ) {
        (
            Some(relation_oid),
            Some(relation_name),
            Some(xid_age),
            Some(effective_freeze_max_age),
        ) => Some(XidRelationSummary {
            relation_oid,
            relation_name,
            xid_age,
            effective_freeze_max_age,
        }),

        _ => None,
    };

    Ok(XidSnapshotSummary {
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
        current_xid: row.get(16)?,
        database_xid_age: row.get(17)?,
        oldest_relation,
        unavailable_sections: row.get(22)?,
        stored_at_ms: row.get(23)?,
    })
}

fn oldest_relation_summary(
    section: &XidSection<Vec<XidRelationAge>>,
) -> Option<XidRelationSummary> {
    section
        .data
        .as_ref()
        .and_then(|rows| rows.first())
        .map(|relation| XidRelationSummary {
            relation_oid: i64::from(relation.relation_oid),
            relation_name: relation_name(relation),
            xid_age: relation.xid_age,
            effective_freeze_max_age: relation.effective_freeze_max_age,
        })
}

fn unavailable_section_count(snapshot: &XidSnapshot) -> i64 {
    [
        snapshot.databases.error.is_some(),
        snapshot.relations.error.is_some(),
        snapshot.autovacuum_triggers.error.is_some(),
        snapshot.vacuums.error.is_some(),
        snapshot.backends.error.is_some(),
        snapshot.slots.error.is_some(),
        snapshot.prepared.error.is_some(),
    ]
    .into_iter()
    .filter(|unavailable| *unavailable)
    .count() as i64
}

fn compare_snapshots(
    older: StoredXidSnapshot,
    newer: StoredXidSnapshot,
) -> Result<XidComparison, String> {
    validate_same_target(&older.summary, &newer.summary)?;

    let elapsed_ms =
        newer.summary.collection_started_at_ms - older.summary.collection_started_at_ms;

    if elapsed_ms <= 0 {
        return Err(crate::i18n::text(
            "Между XID snapshots нет положительного временного интервала",
            "There is no positive time interval between the XID snapshots",
        )
        .into());
    }

    let elapsed_seconds = elapsed_ms as f64 / 1000.0;

    let mut limitations = Vec::new();

    let xid_rate = xid_rate(
        &older.summary.current_xid,
        &newer.summary.current_xid,
        elapsed_seconds,
        &mut limitations,
    );

    let oldest_relation = compare_oldest_relation(&older.summary, &newer.summary);

    let relation_changes = compare_relations(
        &older.snapshot.relations,
        &newer.snapshot.relations,
        &mut limitations,
    );

    let blockers = compare_blockers(&older.snapshot, &newer.snapshot, &mut limitations);

    let vacuums = compare_vacuums(
        &older.snapshot.vacuums,
        &newer.snapshot.vacuums,
        &mut limitations,
    );

    if older.snapshot.overview.stats_reset != newer.snapshot.overview.stats_reset {
        limitations.push(
            crate::i18n::text("Между snapshots изменился pg_stat_database.stats_reset; cumulative DML counters нельзя напрямую сравнивать между этими точками.", "pg_stat_database.stats_reset changed between snapshots; cumulative DML counters cannot be compared directly across these points.")
                .into(),
        );
    }

    limitations.push(
        crate::i18n::text("XID rate рассчитан по двум точкам `pg_current_xact_id()` и локальному интервалу между началами collection. Короткий интервал не является долгосрочным capacity forecast.", "XID rate is calculated from two `pg_current_xact_id()` readings and the local interval between collection starts. A short interval is not a long-term capacity forecast.")
            .into(),
    );

    if older.summary.schema_version < 2 || newer.summary.schema_version < 2 {
        limitations.push(
            crate::i18n::text("Как минимум один XID snapshot имеет schema_version=1 и содержит только TOP-15 старейших relations. Отсутствие relation в таком snapshot не означает её удаление.", "At least one XID snapshot has schema_version=1 and contains only the TOP-15 oldest relations. A missing relation in such a snapshot does not imply deletion.")
                .into(),
        );
    }

    limitations.push(
        crate::i18n::text("Backend XMIN comparison основан на TOP-10 backend_xmin. PID может быть переиспользован, поэтому появление/исчезновение PID — только изменение наблюдаемого sample.", "Backend XMIN comparison uses the TOP-10 backend_xmin sample. PIDs can be reused, so a PID appearing or disappearing only changes the observed sample.")
            .into(),
    );

    limitations.push(
        crate::i18n::text("VACUUM comparison сопоставляет два мгновенных состояния; процесс мог стартовать и завершиться между snapshots.", "VACUUM comparison contrasts two instantaneous states; a process may have started and finished between snapshots.")
            .into(),
    );

    Ok(XidComparison {
        database_xid_age_delta: newer.summary.database_xid_age - older.summary.database_xid_age,
        older: older.summary,
        newer: newer.summary,
        elapsed_seconds,
        xid_rate,
        oldest_relation,
        relation_changes,
        blockers,
        vacuums,
        limitations,
    })
}

fn validate_same_target(
    older: &XidSnapshotSummary,
    newer: &XidSnapshotSummary,
) -> Result<(), String> {
    if older.connection_id != newer.connection_id
        || older.host != newer.host
        || older.port != newer.port
        || older.database_oid != newer.database_oid
        || older.username != newer.username
        || older.sslmode != newer.sslmode
    {
        return Err(
            format!(
                "Нельзя сравнить snapshots разных PostgreSQL targets: #{} = {}:{}/{} (db oid {}), #{} = {}:{}/{} (db oid {})",
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

fn xid_rate(
    older_xid: &str,
    newer_xid: &str,
    elapsed_seconds: f64,
    limitations: &mut Vec<String>,
) -> Option<XidRate> {
    let older = match older_xid.parse::<u64>() {
        Ok(value) => value,

        Err(_) => {
            limitations.push(format!(
                "Не удалось разобрать older current_xid='{older_xid}' как xid8."
            ));

            return None;
        }
    };

    let newer = match newer_xid.parse::<u64>() {
        Ok(value) => value,

        Err(_) => {
            limitations.push(format!(
                "Не удалось разобрать newer current_xid='{newer_xid}' как xid8."
            ));

            return None;
        }
    };

    let xid_delta = match newer.checked_sub(older) {
        Some(value) => value,

        None => {
            limitations.push(
                    "newer current_xid меньше older current_xid. Возможна смена/восстановление кластера или несопоставимые measurements; rate не рассчитан."
                        .into(),
                );

            return None;
        }
    };

    let xid_per_second = xid_delta as f64 / elapsed_seconds;

    Some(XidRate {
        xid_delta,
        elapsed_seconds,
        xid_per_second,
        xid_per_minute: xid_per_second * 60.0,
        xid_per_hour: xid_per_second * 3600.0,
    })
}

fn compare_oldest_relation(
    older: &XidSnapshotSummary,
    newer: &XidSnapshotSummary,
) -> XidOldestRelationComparison {
    let xid_age_delta = match (
        older.oldest_relation.as_ref(),
        newer.oldest_relation.as_ref(),
    ) {
        (Some(older_relation), Some(newer_relation))
            if older_relation.relation_oid == newer_relation.relation_oid =>
        {
            Some(newer_relation.xid_age - older_relation.xid_age)
        }

        _ => None,
    };

    XidOldestRelationComparison {
        older: older.oldest_relation.as_ref().map(clone_relation_summary),
        newer: newer.oldest_relation.as_ref().map(clone_relation_summary),
        xid_age_delta,
        older_configured_headroom: configured_headroom(older.oldest_relation.as_ref()),
        newer_configured_headroom: configured_headroom(newer.oldest_relation.as_ref()),
    }
}

fn configured_headroom(relation: Option<&XidRelationSummary>) -> Option<i64> {
    relation.map(|relation| relation.effective_freeze_max_age - relation.xid_age)
}

fn clone_relation_summary(relation: &XidRelationSummary) -> XidRelationSummary {
    XidRelationSummary {
        relation_oid: relation.relation_oid,
        relation_name: relation.relation_name.clone(),
        xid_age: relation.xid_age,
        effective_freeze_max_age: relation.effective_freeze_max_age,
    }
}

fn compare_relations(
    older: &XidSection<Vec<XidRelationAge>>,
    newer: &XidSection<Vec<XidRelationAge>>,
    limitations: &mut Vec<String>,
) -> Vec<XidRelationDelta> {
    let (Some(older_rows), Some(newer_rows)) = (older.data.as_ref(), newer.data.as_ref()) else {
        limitations.push(
            crate::i18n::text("Relation delta недоступна, потому что секция relations отсутствует хотя бы в одном snapshot.", "Relation delta is unavailable because the relations section is missing from at least one snapshot.")
                .into(),
        );

        return Vec::new();
    };

    let older_by_oid: HashMap<u32, &XidRelationAge> = older_rows
        .iter()
        .map(|relation| (relation.relation_oid, relation))
        .collect();

    let newer_by_oid: HashMap<u32, &XidRelationAge> = newer_rows
        .iter()
        .map(|relation| (relation.relation_oid, relation))
        .collect();

    let mut oids: HashSet<u32> = older_by_oid.keys().copied().collect();

    oids.extend(newer_by_oid.keys().copied());

    let mut result = oids
        .into_iter()
        .map(|oid| {
            let older_relation = older_by_oid.get(&oid);

            let newer_relation = newer_by_oid.get(&oid);

            let same_identity = matches!(
                (older_relation, newer_relation),
                (Some(old), Some(new))
                    if old.schema_name == new.schema_name
                        && old.relation_name == new.relation_name
                        && old.relation_type == new.relation_type
            );
            let sample_state = match (older_relation, newer_relation) {
                (Some(_), Some(_)) if same_identity => "present_in_both",
                (Some(_), Some(_)) => "oid_identity_changed",

                (None, Some(_)) => "entered_new_sample",

                (Some(_), None) => "not_in_new_sample",

                (None, None) => unreachable!(),
            };

            XidRelationDelta {
                relation_oid: oid,
                older_name: older_relation.map(|relation| relation_name(relation)),
                newer_name: newer_relation.map(|relation| relation_name(relation)),
                older_xid_age: older_relation.map(|relation| relation.xid_age),
                newer_xid_age: newer_relation.map(|relation| relation.xid_age),
                xid_age_delta: match (older_relation, newer_relation) {
                    (Some(older_relation), Some(newer_relation)) if same_identity => {
                        newer_relation.xid_age.checked_sub(older_relation.xid_age)
                    }

                    _ => None,
                },
                older_pct_of_freeze_max: older_relation
                    .and_then(|relation| relation.pct_of_freeze_max),
                newer_pct_of_freeze_max: newer_relation
                    .and_then(|relation| relation.pct_of_freeze_max),
                sample_state: sample_state.to_owned(),
            }
        })
        .collect::<Vec<_>>();

    result.sort_by(|left, right| {
        right
            .newer_xid_age
            .or(right.older_xid_age)
            .unwrap_or(i64::MIN)
            .cmp(
                &left
                    .newer_xid_age
                    .or(left.older_xid_age)
                    .unwrap_or(i64::MIN),
            )
    });

    result
}

fn compare_blockers(
    older: &XidSnapshot,
    newer: &XidSnapshot,
    limitations: &mut Vec<String>,
) -> XidBlockerComparison {
    let (backend_new_in_sample, backend_not_in_new_sample) = compare_identity_set(
        older
            .backends
            .data
            .as_ref()
            .map(|rows| rows.iter().map(backend_identity).collect::<Vec<_>>()),
        newer
            .backends
            .data
            .as_ref()
            .map(|rows| rows.iter().map(backend_identity).collect::<Vec<_>>()),
    );

    if older.backends.data.is_none() || newer.backends.data.is_none() {
        limitations.push(
            crate::i18n::text(
                "Backend XMIN delta частично недоступна.",
                "Backend XMIN delta is partially unavailable.",
            )
            .into(),
        );
    }

    let (slots_new_in_sample, slots_not_in_new_sample) = compare_identity_set(
        older
            .slots
            .data
            .as_ref()
            .map(|rows| rows.iter().map(slot_identity).collect::<Vec<_>>()),
        newer
            .slots
            .data
            .as_ref()
            .map(|rows| rows.iter().map(slot_identity).collect::<Vec<_>>()),
    );

    if older.slots.data.is_none() || newer.slots.data.is_none() {
        limitations.push(
            crate::i18n::text(
                "Replication slot delta частично недоступна.",
                "Replication slot delta is partially unavailable.",
            )
            .into(),
        );
    }

    XidBlockerComparison {
        older_backend_count: section_len(&older.backends),
        newer_backend_count: section_len(&newer.backends),
        backend_new_in_sample,
        backend_not_in_new_sample,
        older_slot_count: section_len(&older.slots),
        newer_slot_count: section_len(&newer.slots),
        slots_new_in_sample,
        slots_not_in_new_sample,
        older_prepared_count: prepared_count(&older.prepared),
        newer_prepared_count: prepared_count(&newer.prepared),
    }
}

fn compare_vacuums(
    older: &XidSection<Vec<XidVacuum>>,
    newer: &XidSection<Vec<XidVacuum>>,
    limitations: &mut Vec<String>,
) -> XidVacuumComparison {
    let (active_new_in_sample, active_not_in_new_sample) = compare_identity_set(
        older
            .data
            .as_ref()
            .map(|rows| rows.iter().map(vacuum_identity).collect::<Vec<_>>()),
        newer
            .data
            .as_ref()
            .map(|rows| rows.iter().map(vacuum_identity).collect::<Vec<_>>()),
    );

    if older.data.is_none() || newer.data.is_none() {
        limitations.push(
            crate::i18n::text(
                "VACUUM delta частично недоступна.",
                "VACUUM delta is partially unavailable.",
            )
            .into(),
        );
    }

    XidVacuumComparison {
        older_active_count: section_len(older),
        newer_active_count: section_len(newer),
        older_wraparound_count: wraparound_count(older),
        newer_wraparound_count: wraparound_count(newer),
        active_new_in_sample,
        active_not_in_new_sample,
    }
}

fn compare_identity_set(
    older: Option<Vec<String>>,
    newer: Option<Vec<String>>,
) -> (Vec<String>, Vec<String>) {
    let (Some(older), Some(newer)) = (older, newer) else {
        return (Vec::new(), Vec::new());
    };

    let older_set: HashSet<_> = older.into_iter().collect();

    let newer_set: HashSet<_> = newer.into_iter().collect();

    let mut added = newer_set
        .difference(&older_set)
        .cloned()
        .collect::<Vec<_>>();

    let mut removed = older_set
        .difference(&newer_set)
        .cloned()
        .collect::<Vec<_>>();

    added.sort();
    removed.sort();

    (added, removed)
}

fn backend_identity(backend: &XidBackend) -> String {
    format!(
        "pid={} user={} db={} app={}",
        backend.pid,
        backend.username.as_deref().unwrap_or("-"),
        backend.database.as_deref().unwrap_or("-"),
        backend.application_name.as_deref().unwrap_or("-"),
    )
}

fn slot_identity(slot: &XidSlot) -> String {
    format!(
        "{} ({}, db={})",
        slot.slot_name,
        slot.slot_type,
        slot.database.as_deref().unwrap_or("-"),
    )
}

fn vacuum_identity(vacuum: &XidVacuum) -> String {
    format!(
        "{}.{} (oid={}, pid={}, {})",
        vacuum.schema_name,
        vacuum.relation_name,
        vacuum.relation_oid,
        vacuum.pid,
        if vacuum.is_wraparound_vacuum {
            "anti-wraparound"
        } else {
            "vacuum"
        },
    )
}

fn relation_name(relation: &XidRelationAge) -> String {
    format!("{}.{}", relation.schema_name, relation.relation_name,)
}

fn section_len<T>(section: &XidSection<Vec<T>>) -> Option<usize> {
    section.data.as_ref().map(Vec::len)
}

fn prepared_count(section: &XidSection<XidPreparedSummary>) -> Option<i64> {
    section.data.as_ref().map(|summary| summary.count)
}

fn wraparound_count(section: &XidSection<Vec<XidVacuum>>) -> Option<usize> {
    section.data.as_ref().map(|rows| {
        rows.iter()
            .filter(|vacuum| vacuum.is_wraparound_vacuum)
            .count()
    })
}

fn snapshot_order_key(summary: &XidSnapshotSummary) -> (i64, i64) {
    (summary.collection_started_at_ms, summary.id)
}

#[cfg(test)]
mod history_overview_tests {
    use super::*;

    #[test]
    fn empty_history_returns_empty_overviews_without_live_postgres() {
        let connection = Connection::open_in_memory().unwrap();
        init_schema(&connection).unwrap();
        let mut store = DiagnosticStore { connection };
        assert!(store.xid_history_overview(7, Some(10)).unwrap().is_empty());
        assert!(store.xid_history_overview(0, Some(10)).is_err());
    }
}
