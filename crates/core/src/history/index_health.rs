use super::{unix_time_millis, DiagnosticStore, DEFAULT_HISTORY_LIMIT, MAX_HISTORY_LIMIT};
use crate::{connections::SavedConnection, index_health::IndexHealthSnapshot};
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const INDEX_HEALTH_SNAPSHOT_SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct CollectedIndexHealthSnapshot {
    pub history: IndexHealthSnapshotSummary,
    pub snapshot: IndexHealthSnapshot,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexHealthSnapshotSummary {
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
    pub total_index_count: i64,
    pub sampled_index_count: i64,
    pub suspiciously_unused_count: i64,
    pub possible_duplicate_count: i64,
    pub high_write_overhead_count: i64,
    pub sample_truncated: bool,
    pub stored_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexHealthDelta {
    pub index_oid: u32,
    pub relation_oid: u32,
    pub relation_name: String,
    pub index_name: String,
    pub sample_state: String,
    pub idx_scan_delta: Option<i64>,
    pub idx_tup_read_delta: Option<i64>,
    pub idx_tup_fetch_delta: Option<i64>,
    pub relation_dml_delta: Option<i64>,
    pub older_size_bytes: Option<i64>,
    pub newer_size_bytes: Option<i64>,
    pub size_delta_bytes: Option<i64>,
    pub newer_classifications: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexHealthComparison {
    pub older: IndexHealthSnapshotSummary,
    pub newer: IndexHealthSnapshotSummary,
    pub elapsed_seconds: f64,
    pub stats_period_continuous: bool,
    pub index_changes: Vec<IndexHealthDelta>,
    pub limitations: Vec<String>,
}

#[derive(Clone)]
pub(super) struct StoredIndexHealthSnapshot {
    pub summary: IndexHealthSnapshotSummary,
    pub snapshot: IndexHealthSnapshot,
}

pub(super) fn init_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS index_health_snapshots (
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
                total_index_count INTEGER NOT NULL,
                sampled_index_count INTEGER NOT NULL,
                suspiciously_unused_count INTEGER NOT NULL,
                possible_duplicate_count INTEGER NOT NULL,
                high_write_overhead_count INTEGER NOT NULL,
                sample_truncated INTEGER NOT NULL,
                stored_at_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_index_health_snapshots_connection_time
                ON index_health_snapshots(connection_id, collection_started_at_ms DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_index_health_snapshots_stored_at
                ON index_health_snapshots(stored_at_ms);
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить Index Health history schema: {error}"))
}

impl DiagnosticStore {
    pub fn save_index_health_snapshot(
        &mut self,
        saved_connection: &SavedConnection,
        snapshot: &IndexHealthSnapshot,
        collection_started_at_ms: i64,
        collection_completed_at_ms: i64,
    ) -> Result<IndexHealthSnapshotSummary, String> {
        if collection_completed_at_ms < collection_started_at_ms {
            return Err(crate::i18n::text(
                "Время завершения Index Health snapshot раньше времени начала",
                "Index Health snapshot finished before it started",
            )
            .into());
        }

        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|error| format!("Не удалось сериализовать Index Health snapshot: {error}"))?;
        let count = |label: &str| {
            snapshot
                .collection
                .indexes
                .iter()
                .filter(|row| row.classifications.iter().any(|value| value == label))
                .count() as i64
        };
        let suspiciously_unused_count = count("suspiciously_unused");
        let possible_duplicate_count = count("possible_duplicate");
        let high_write_overhead_count = count("high_write_overhead");
        let stored_at_ms = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT INTO index_health_snapshots (
                    schema_version, app_version, connection_id, connection_name, host, port,
                    database_name, database_oid, username, sslmode, server_version,
                    server_version_num, collection_started_at_ms, collection_completed_at_ms,
                    measured_at, stats_reset, total_index_count, sampled_index_count,
                    suspiciously_unused_count, possible_duplicate_count,
                    high_write_overhead_count, sample_truncated, stored_at_ms, snapshot_json
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
                )
                "#,
                params![
                    INDEX_HEALTH_SNAPSHOT_SCHEMA_VERSION,
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
                    snapshot.overview.stats_reset,
                    snapshot.collection.total_index_count,
                    snapshot.collection.sampled_index_count,
                    suspiciously_unused_count,
                    possible_duplicate_count,
                    high_write_overhead_count,
                    snapshot.collection.truncated,
                    stored_at_ms,
                    snapshot_json,
                ],
            )
            .map_err(|error| format!("Не удалось сохранить Index Health snapshot: {error}"))?;

        Ok(IndexHealthSnapshotSummary {
            id: self.connection.last_insert_rowid(),
            schema_version: INDEX_HEALTH_SNAPSHOT_SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
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
            total_index_count: snapshot.collection.total_index_count,
            sampled_index_count: snapshot.collection.sampled_index_count,
            suspiciously_unused_count,
            possible_duplicate_count,
            high_write_overhead_count,
            sample_truncated: snapshot.collection.truncated,
            stored_at_ms,
        })
    }

    pub fn list_index_health_snapshots(
        &self,
        connection_id: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<IndexHealthSnapshotSummary>, String> {
        let limit = limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);
        let sql = if connection_id.is_some() {
            format!(
                "{} WHERE connection_id = ?1 ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?2",
                SUMMARY_SELECT,
            )
        } else {
            format!(
                "{} ORDER BY collection_started_at_ms DESC, id DESC LIMIT ?1",
                SUMMARY_SELECT,
            )
        };
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| format!("Не удалось подготовить Index Health history: {error}"))?;
        let mut result = Vec::new();

        if let Some(connection_id) = connection_id {
            let rows = statement
                .query_map(params![connection_id, i64::from(limit)], summary_from_row)
                .map_err(|error| format!("Не удалось прочитать Index Health history: {error}"))?;
            for row in rows {
                result.push(
                    row.map_err(|error| format!("Повреждена Index Health history: {error}"))?,
                );
            }
        } else {
            let rows = statement
                .query_map(params![i64::from(limit)], summary_from_row)
                .map_err(|error| format!("Не удалось прочитать Index Health history: {error}"))?;
            for row in rows {
                result.push(
                    row.map_err(|error| format!("Повреждена Index Health history: {error}"))?,
                );
            }
        }

        Ok(result)
    }

    /// Read a saved diagnostic snapshot only for the requested connection.
    /// The UI never uses a snapshot ID alone as an authority for its source.
    pub fn index_health_snapshot(
        &self,
        connection_id: i64,
        snapshot_id: i64,
    ) -> Result<IndexHealthSnapshot, String> {
        let (summary, snapshot) = load_snapshot(&self.connection, snapshot_id)?;
        if summary.connection_id != connection_id {
            return Err(crate::i18n::text(
                "Index Health snapshot принадлежит другому подключению",
                "Index Health snapshot belongs to a different connection",
            )
            .into());
        }
        Ok(snapshot)
    }

    pub fn compare_index_health_snapshots(
        &self,
        first_id: i64,
        second_id: i64,
    ) -> Result<IndexHealthComparison, String> {
        let first = load_snapshot(&self.connection, first_id)?;
        let second = load_snapshot(&self.connection, second_id)?;
        let (older_summary, older, newer_summary, newer) =
            if first.0.collection_started_at_ms <= second.0.collection_started_at_ms {
                (first.0, first.1, second.0, second.1)
            } else {
                (second.0, second.1, first.0, first.1)
            };

        ensure_same_target(&older_summary, &newer_summary)?;
        let elapsed_seconds = (newer_summary.collection_started_at_ms
            - older_summary.collection_started_at_ms) as f64
            / 1000.0;
        let stats_period_continuous = same_reset(
            older_summary.stats_reset.as_deref(),
            newer_summary.stats_reset.as_deref(),
        );
        let mut limitations = Vec::new();

        if !stats_period_continuous {
            limitations.push(
                crate::i18n::text(
                    "stats_reset изменился/неизвестен; usage/DML deltas не рассчитаны.",
                    "stats_reset changed or is unknown; usage/DML deltas were not calculated.",
                )
                .into(),
            );
        }
        limitations.push(
            crate::i18n::text("Single-object statistics reset не всегда виден через database stats_reset; отрицательные deltas отбрасываются, но reset с последующим ростом может сделать interval неполным.", "A per-object statistics reset is not always visible through database stats_reset; negative deltas are discarded, but a reset followed by growth may leave an interval incomplete.")
                .into(),
        );
        if older_summary.sample_truncated || newer_summary.sample_truncated {
            limitations.push(
                crate::i18n::text("Index sample truncated; отсутствие index в sample не означает DROP/CREATE.", "The index sample was truncated; an index missing from the sample does not imply DROP/CREATE.").into(),
            );
        }

        let old_by_oid: HashMap<u32, _> = older
            .collection
            .indexes
            .iter()
            .map(|row| (row.index_oid, row))
            .collect();
        let new_by_oid: HashMap<u32, _> = newer
            .collection
            .indexes
            .iter()
            .map(|row| (row.index_oid, row))
            .collect();
        let mut index_oids: HashSet<u32> = old_by_oid.keys().copied().collect();
        index_oids.extend(new_by_oid.keys().copied());
        let mut changes = Vec::new();

        for index_oid in index_oids {
            let old = old_by_oid.get(&index_oid).copied();
            let new = new_by_oid.get(&index_oid).copied();
            let same_identity = matches!(
                (old, new),
                (Some(left), Some(right))
                    if left.relation_oid == right.relation_oid
                        && left.schema_name == right.schema_name
                        && left.relation_name == right.relation_name
                        && left.index_name == right.index_name
            );
            let sample_state = match (old, new, same_identity) {
                (Some(_), Some(_), true) => "comparable",
                (Some(_), Some(_), false) => "oid_identity_changed",
                (Some(_), None, _) => "sampled_only_older",
                (None, Some(_), _) => "sampled_only_newer",
                _ => continue,
            };
            let counter_delta = |old_value: Option<i64>, new_value: Option<i64>| {
                if !stats_period_continuous || !same_identity {
                    return None;
                }
                let (old_value, new_value) = (old_value?, new_value?);
                (new_value >= old_value).then_some(new_value - old_value)
            };
            let old_dml = old.map(total_relation_dml);
            let new_dml = new.map(total_relation_dml);

            changes.push(IndexHealthDelta {
                index_oid,
                relation_oid: new.or(old).map(|row| row.relation_oid).unwrap_or(0),
                relation_name: new
                    .or(old)
                    .map(|row| format!("{}.{}", row.schema_name, row.relation_name))
                    .unwrap_or_default(),
                index_name: new
                    .or(old)
                    .map(|row| row.index_name.clone())
                    .unwrap_or_default(),
                sample_state: sample_state.into(),
                idx_scan_delta: counter_delta(
                    old.map(|row| row.idx_scan),
                    new.map(|row| row.idx_scan),
                ),
                idx_tup_read_delta: counter_delta(
                    old.map(|row| row.idx_tup_read),
                    new.map(|row| row.idx_tup_read),
                ),
                idx_tup_fetch_delta: counter_delta(
                    old.map(|row| row.idx_tup_fetch),
                    new.map(|row| row.idx_tup_fetch),
                ),
                relation_dml_delta: counter_delta(old_dml, new_dml),
                older_size_bytes: old.map(|row| row.index_size_bytes),
                newer_size_bytes: new.map(|row| row.index_size_bytes),
                size_delta_bytes: if same_identity {
                    Some(
                        new.expect("same identity requires newer index")
                            .index_size_bytes
                            .saturating_sub(
                                old.expect("same identity requires older index")
                                    .index_size_bytes,
                            ),
                    )
                } else {
                    None
                },
                newer_classifications: new
                    .map(|row| row.classifications.clone())
                    .unwrap_or_default(),
            });
        }

        changes.sort_by(|left, right| {
            right
                .relation_dml_delta
                .unwrap_or(0)
                .cmp(&left.relation_dml_delta.unwrap_or(0))
        });

        Ok(IndexHealthComparison {
            older: older_summary,
            newer: newer_summary,
            elapsed_seconds,
            stats_period_continuous,
            index_changes: changes,
            limitations,
        })
    }

    pub(super) fn index_health_snapshots_in_period(
        &self,
        connection_id: i64,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<StoredIndexHealthSnapshot>, String> {
        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT id
                FROM index_health_snapshots
                WHERE connection_id = ?1
                  AND collection_started_at_ms >= ?2
                  AND collection_started_at_ms <= ?3
                ORDER BY collection_started_at_ms ASC, id ASC
                "#,
            )
            .map_err(|error| {
                format!("Не удалось подготовить Index Health period history: {error}")
            })?;
        let rows = statement
            .query_map(params![connection_id, from_ms, to_ms], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| {
                format!("Не удалось прочитать Index Health period history: {error}")
            })?;
        let mut result = Vec::new();

        for row in rows {
            let id = row.map_err(|error| format!("Повреждена Index Health history: {error}"))?;
            let (summary, snapshot) = load_snapshot(&self.connection, id)?;
            result.push(StoredIndexHealthSnapshot { summary, snapshot });
        }

        Ok(result)
    }
}

const SUMMARY_SELECT: &str = r#"
SELECT
    id, schema_version, app_version, connection_id, connection_name, host, port,
    database_name, database_oid, username, sslmode, server_version, server_version_num,
    collection_started_at_ms, collection_completed_at_ms, measured_at, stats_reset,
    total_index_count, sampled_index_count, suspiciously_unused_count,
    possible_duplicate_count, high_write_overhead_count, sample_truncated, stored_at_ms
FROM index_health_snapshots
"#;

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<IndexHealthSnapshotSummary> {
    Ok(IndexHealthSnapshotSummary {
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
        total_index_count: row.get(17)?,
        sampled_index_count: row.get(18)?,
        suspiciously_unused_count: row.get(19)?,
        possible_duplicate_count: row.get(20)?,
        high_write_overhead_count: row.get(21)?,
        sample_truncated: row.get(22)?,
        stored_at_ms: row.get(23)?,
    })
}

fn load_snapshot(
    connection: &Connection,
    id: i64,
) -> Result<(IndexHealthSnapshotSummary, IndexHealthSnapshot), String> {
    let summary = connection
        .query_row(
            &format!("{} WHERE id = ?1", SUMMARY_SELECT),
            params![id],
            summary_from_row,
        )
        .map_err(|error| format!("Index Health snapshot #{id} не найден: {error}"))?;
    let snapshot_json: String = connection
        .query_row(
            "SELECT snapshot_json FROM index_health_snapshots WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Index Health snapshot #{id} не найден: {error}"))?;
    if summary.schema_version != INDEX_HEALTH_SNAPSHOT_SCHEMA_VERSION {
        return Err(format!(
            "Index Health snapshot #{id} имеет schema_version={}, поддерживается {}",
            summary.schema_version, INDEX_HEALTH_SNAPSHOT_SCHEMA_VERSION
        ));
    }
    let snapshot = serde_json::from_str(&snapshot_json)
        .map_err(|error| format!("Повреждён Index Health snapshot #{id}: {error}"))?;

    Ok((summary, snapshot))
}

fn ensure_same_target(
    older: &IndexHealthSnapshotSummary,
    newer: &IndexHealthSnapshotSummary,
) -> Result<(), String> {
    if older.connection_id != newer.connection_id
        || older.host != newer.host
        || older.port != newer.port
        || older.database_oid != newer.database_oid
    {
        return Err(crate::i18n::text(
            "Index Health snapshots относятся к разным PostgreSQL targets",
            "Index Health snapshots belong to different PostgreSQL targets",
        )
        .into());
    }

    Ok(())
}

fn same_reset(older: Option<&str>, newer: Option<&str>) -> bool {
    matches!((older, newer), (Some(left), Some(right)) if left == right)
}

fn total_relation_dml(row: &crate::index_health::IndexHealthIndex) -> i64 {
    row.tuples_inserted
        .saturating_add(row.tuples_updated)
        .saturating_add(row.tuples_deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_must_be_known_and_equal() {
        assert!(same_reset(Some("a"), Some("a")));
        assert!(!same_reset(Some("a"), Some("b")));
        assert!(!same_reset(None, None));
    }
}
