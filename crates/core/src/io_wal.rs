use crate::{
    internal_postgres::InternalPostgresRegistry,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub const RELATION_SIZE_SAMPLE_LIMIT: i64 = 500;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalOverview {
    pub measured_at: String,
    pub database: String,
    pub database_oid: u32,
    pub username: String,
    pub server_version: String,
    pub server_version_num: i32,
    pub block_size: i64,
    pub track_io_timing: bool,
    pub track_wal_io_timing: Option<bool>,
    pub checkpoint_timeout: String,
    pub checkpoint_completion_target: f64,
    pub max_wal_size: String,
    pub min_wal_size: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalDatabaseStats {
    pub stats_reset: Option<String>,
    pub temp_files: i64,
    pub temp_bytes: i64,
    pub blks_read: i64,
    pub blks_hit: i64,
    pub blk_read_time_ms: f64,
    pub blk_write_time_ms: f64,
    pub xact_commit: i64,
    pub xact_rollback: i64,
    pub deadlocks: i64,
    pub conflicts: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalWalStats {
    pub stats_reset: String,
    pub wal_records: i64,
    pub wal_fpi: i64,
    pub wal_bytes: f64,
    pub wal_buffers_full: i64,
    pub wal_write_time_ms: f64,
    pub wal_sync_time_ms: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalCheckpointStats {
    pub source: String,
    pub stats_reset: String,
    pub timed: i64,
    pub requested: i64,
    pub write_time_ms: f64,
    pub sync_time_ms: f64,
    pub buffers_written: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalRelationSize {
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub relation_type: String,
    pub heap_size_bytes: i64,
    pub indexes_size_bytes: i64,
    pub toast_size_bytes: i64,
    pub total_size_bytes: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalRelationSizes {
    pub total_relation_count: i64,
    pub sampled_relation_count: i64,
    pub truncated: bool,
    pub relations: Vec<IoWalRelationSize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalSection<T> {
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalInterpretationItem {
    pub id: String,
    pub title: String,
    pub observed: Vec<String>,
    pub meaning: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalInterpretation {
    pub items: Vec<IoWalInterpretationItem>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IoWalSnapshot {
    pub overview: IoWalOverview,
    pub database_stats: IoWalSection<IoWalDatabaseStats>,
    pub wal: IoWalSection<IoWalWalStats>,
    pub checkpoints: IoWalSection<IoWalCheckpointStats>,
    pub database_size_bytes: IoWalSection<i64>,
    pub relation_sizes: IoWalSection<IoWalRelationSizes>,
    pub interpretation: IoWalInterpretation,
}

#[derive(Clone, Debug, Serialize)]
pub struct IoWalReferenceEntry {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct IoWalReference {
    pub methodology: Vec<String>,
    pub sources: Vec<IoWalReferenceEntry>,
}

const OVERVIEW_SQL: &str = r#"
SELECT json_build_object(
    'measured_at', clock_timestamp(),
    'database', current_database(),
    'database_oid', d.oid::bigint,
    'username', current_user,
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'block_size', current_setting('block_size')::bigint,
    'track_io_timing', current_setting('track_io_timing') = 'on',
    'track_wal_io_timing',
        CASE current_setting('track_wal_io_timing', true)
            WHEN 'on' THEN true
            WHEN 'off' THEN false
            ELSE NULL
        END,
    'checkpoint_timeout', current_setting('checkpoint_timeout'),
    'checkpoint_completion_target', current_setting('checkpoint_completion_target')::double precision,
    'max_wal_size', current_setting('max_wal_size'),
    'min_wal_size', current_setting('min_wal_size')
)::text
FROM pg_database AS d
WHERE d.datname = current_database();
"#;

const DATABASE_STATS_SQL: &str = r#"
SELECT json_build_object(
    'stats_reset', s.stats_reset,
    'temp_files', s.temp_files,
    'temp_bytes', s.temp_bytes,
    'blks_read', s.blks_read,
    'blks_hit', s.blks_hit,
    'blk_read_time_ms', s.blk_read_time,
    'blk_write_time_ms', s.blk_write_time,
    'xact_commit', s.xact_commit,
    'xact_rollback', s.xact_rollback,
    'deadlocks', s.deadlocks,
    'conflicts', s.conflicts
)::text
FROM pg_stat_database AS s
WHERE s.datname = current_database();
"#;

const WAL_SQL: &str = r#"
SELECT json_build_object(
    'stats_reset', s.stats_reset,
    'wal_records', s.wal_records,
    'wal_fpi', s.wal_fpi,
    'wal_bytes', s.wal_bytes::double precision,
    'wal_buffers_full', s.wal_buffers_full,
    'wal_write_time_ms', s.wal_write_time,
    'wal_sync_time_ms', s.wal_sync_time
)::text
FROM pg_stat_wal AS s;
"#;

const CHECKPOINTS_PRE17_SQL: &str = r#"
SELECT json_build_object(
    'source', 'pg_stat_bgwriter',
    'stats_reset', s.stats_reset,
    'timed', s.checkpoints_timed,
    'requested', s.checkpoints_req,
    'write_time_ms', s.checkpoint_write_time,
    'sync_time_ms', s.checkpoint_sync_time,
    'buffers_written', s.buffers_checkpoint
)::text
FROM pg_stat_bgwriter AS s;
"#;

const CHECKPOINTS_17_SQL: &str = r#"
SELECT json_build_object(
    'source', 'pg_stat_checkpointer',
    'stats_reset', s.stats_reset,
    'timed', s.num_timed,
    'requested', s.num_requested,
    'write_time_ms', s.write_time,
    'sync_time_ms', s.sync_time,
    'buffers_written', s.buffers_written
)::text
FROM pg_stat_checkpointer AS s;
"#;

const DATABASE_SIZE_SQL: &str = r#"
SELECT pg_database_size(current_database())::bigint::text;
"#;

const RELATION_SIZES_SQL: &str = r#"
WITH eligible AS MATERIALIZED (
    SELECT
        c.oid,
        n.nspname::text AS schema_name,
        c.relname::text AS relation_name,
        c.relkind,
        GREATEST(c.relpages, 0)::bigint AS estimated_heap_pages
    FROM pg_class AS c
    JOIN pg_namespace AS n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'm')
      AND n.nspname NOT IN ('pg_catalog', 'information_schema')
      AND n.nspname !~ '^pg_toast'
),
candidates AS MATERIALIZED (
    SELECT *
    FROM eligible
    ORDER BY estimated_heap_pages DESC, oid
    LIMIT __RELATION_SIZE_LIMIT__
),
sampled AS (
    SELECT
        c.oid::bigint AS relation_oid,
        c.schema_name,
        c.relation_name,
        CASE c.relkind
            WHEN 'r' THEN 'table'
            WHEN 'p' THEN 'partitioned table'
            WHEN 'm' THEN 'materialized view'
            ELSE c.relkind::text
        END AS relation_type,
        GREATEST(
            pg_table_size(c.oid)::bigint
            - CASE
                WHEN cls.reltoastrelid = 0 THEN 0::bigint
                ELSE pg_total_relation_size(cls.reltoastrelid)::bigint
              END,
            0::bigint
        ) AS heap_size_bytes,
        pg_indexes_size(c.oid)::bigint AS indexes_size_bytes,
        CASE
            WHEN cls.reltoastrelid = 0 THEN 0::bigint
            ELSE pg_total_relation_size(cls.reltoastrelid)::bigint
        END AS toast_size_bytes,
        pg_total_relation_size(c.oid)::bigint AS total_size_bytes
    FROM candidates AS c
    JOIN pg_class AS cls ON cls.oid = c.oid
)
SELECT json_build_object(
    'total_relation_count', (SELECT count(*)::bigint FROM eligible),
    'sampled_relation_count', (SELECT count(*)::bigint FROM sampled),
    'truncated', (SELECT count(*) FROM eligible) > __RELATION_SIZE_LIMIT__,
    'relations', COALESCE(
        (SELECT json_agg(row_to_json(s) ORDER BY s.total_size_bytes DESC, s.relation_oid) FROM sampled AS s),
        '[]'::json
    )
)::text;
"#;

fn relation_sizes_sql() -> String {
    RELATION_SIZES_SQL.replace(
        "__RELATION_SIZE_LIMIT__",
        &RELATION_SIZE_SAMPLE_LIMIT.to_string(),
    )
}

pub async fn collect(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
) -> Result<IoWalSnapshot, String> {
    let overview =
        query_json::<IoWalOverview>(internal_postgres, connection_id, input, OVERVIEW_SQL).await?;

    let database_stats = collect_section::<IoWalDatabaseStats>(
        internal_postgres,
        connection_id,
        input,
        DATABASE_STATS_SQL,
    )
    .await;

    let wal = if overview.server_version_num >= 140_000 {
        collect_section::<IoWalWalStats>(internal_postgres, connection_id, input, WAL_SQL).await
    } else {
        unavailable_section(crate::i18n::text(
            "pg_stat_wal доступен начиная с PostgreSQL 14",
            "pg_stat_wal is available starting with PostgreSQL 14",
        ))
    };

    let checkpoint_sql = if overview.server_version_num >= 170_000 {
        CHECKPOINTS_17_SQL
    } else {
        CHECKPOINTS_PRE17_SQL
    };

    let checkpoints = collect_section::<IoWalCheckpointStats>(
        internal_postgres,
        connection_id,
        input,
        checkpoint_sql,
    )
    .await;

    let database_size_bytes =
        collect_section::<i64>(internal_postgres, connection_id, input, DATABASE_SIZE_SQL).await;

    let relation_sizes_query = relation_sizes_sql();
    let relation_sizes = collect_section::<IoWalRelationSizes>(
        internal_postgres,
        connection_id,
        input,
        &relation_sizes_query,
    )
    .await;

    let interpretation = interpret(
        &overview,
        &database_stats,
        &wal,
        &checkpoints,
        &relation_sizes,
    );

    Ok(IoWalSnapshot {
        overview,
        database_stats,
        wal,
        checkpoints,
        database_size_bytes,
        relation_sizes,
        interpretation,
    })
}

pub fn reference() -> IoWalReference {
    IoWalReference {
        methodology: vec![
            crate::i18n::text("Сравнивать cumulative counters только между snapshots с неизменным stats_reset соответствующего source.", "Compare cumulative counters only between snapshots with an unchanged stats_reset for the relevant source.").into(),
            crate::i18n::text("Temp/WAL/checkpoint totals сами по себе не являются проблемой: основной сигнал — delta/rate за интересующий interval.", "Temp, WAL, and checkpoint totals are not inherently problematic; the meaningful signal is the delta or rate over the selected interval.").into(),
            "Checkpoint requested/timed нужно читать вместе с WAL generation, write/sync time и checkpoint_timeout/max_wal_size context; max_wal_size не является жёстким лимитом WAL.".into(),
            "Relation size change — observed storage change. Heap здесь означает table storage без indexes и отдельно учтённого TOAST; он не объясняет причину без DML/VACUUM/query context.".into(),
            "Чтобы не выполнять size functions по всей схеме, exact sizes считаются только для bounded candidate sample, предварительно выбранного по pg_class.relpages.".into(),
        ],
        sources: vec![
            IoWalReferenceEntry {
                id: "monitoring".into(),
                title: "Cumulative Statistics System".into(),
                purpose: "pg_stat_database, pg_stat_wal, pg_stat_bgwriter/checkpointer и reset semantics.".into(),
                path: "monitoring-stats.html".into(),
            },
            IoWalReferenceEntry {
                id: "checkpoint".into(),
                title: "WAL Configuration / Checkpoints".into(),
                purpose: "Checkpoint cadence, max_wal_size и completion_target context.".into(),
                path: "runtime-config-wal.html#RUNTIME-CONFIG-WAL-CHECKPOINTS".into(),
            },
            IoWalReferenceEntry {
                id: "size".into(),
                title: "Database Object Size Functions".into(),
                purpose: "pg_database_size/pg_table_size/pg_indexes_size/pg_total_relation_size.".into(),
                path: "functions-admin.html#FUNCTIONS-ADMIN-DBSIZE".into(),
            },
        ],
    }
}

fn interpret(
    overview: &IoWalOverview,
    database_stats: &IoWalSection<IoWalDatabaseStats>,
    wal: &IoWalSection<IoWalWalStats>,
    checkpoints: &IoWalSection<IoWalCheckpointStats>,
    relation_sizes: &IoWalSection<IoWalRelationSizes>,
) -> IoWalInterpretation {
    let mut items = Vec::new();
    let mut limitations = Vec::new();

    if let Some(stats) = database_stats.data.as_ref() {
        items.push(IoWalInterpretationItem {
            id: "database-io".into(),
            title: "Database I/O and temp counters".into(),
            observed: vec![
                format!("temp = {} files / {} bytes", stats.temp_files, stats.temp_bytes),
                format!("shared blocks read/hit = {} / {}", stats.blks_read, stats.blks_hit),
            ],
            meaning: vec!["Это cumulative state текущего statistics period; без второго snapshot скорость не известна.".into()],
            next_steps: vec!["Собрать следующий snapshot и смотреть interval deltas/rates.".into()],
        });
    }

    if let Some(checkpoint) = checkpoints.data.as_ref() {
        items.push(IoWalInterpretationItem {
            id: "checkpoints".into(),
            title: "Checkpoint counters".into(),
            observed: vec![format!(
                "{}: timed={} requested={} write={} ms sync={} ms",
                checkpoint.source,
                checkpoint.timed,
                checkpoint.requested,
                checkpoint.write_time_ms,
                checkpoint.sync_time_ms,
            )],
            meaning: vec!["Абсолютные counters не показывают текущую checkpoint pressure.".into()],
            next_steps: vec![crate::i18n::text(
                "Сравнивать requested/timed и write/sync deltas вместе с WAL generation.",
                "Compare requested/timed and write/sync deltas together with WAL generation.",
            )
            .into()],
        });
    }

    if wal.data.is_none() {
        limitations
            .push("WAL statistics недоступны; WAL generation rate не может быть рассчитан.".into());
    }

    if !overview.track_io_timing {
        limitations.push(
            "track_io_timing=off: database I/O timing counters не дают полезного time evidence."
                .into(),
        );
    }

    if overview.track_wal_io_timing == Some(false) {
        limitations.push("track_wal_io_timing=off: WAL write/sync time остаётся zero и не должно интерпретироваться как отсутствие I/O cost.".into());
    }

    limitations.push(
        crate::i18n::text("Разделы I/O, WAL, checkpoints и размеров читаются последовательными запросами: снимок не атомарен.", "I/O, WAL, checkpoint, and size sections are collected with sequential queries; the snapshot is not atomic.").into(),
    );
    if wal
        .data
        .as_ref()
        .is_some_and(|value| value.wal_bytes >= 9_007_199_254_740_992.0)
    {
        limitations.push(
            crate::i18n::text("WAL bytes передаётся как f64: при очень больших накопленных значениях возможна потеря точности единиц байт.", "WAL bytes are represented as f64; very large cumulative values may lose byte-level precision.").into(),
        );
    }

    if let Some(sizes) = relation_sizes.data.as_ref() {
        if sizes.truncated {
            limitations.push(format!(
                "Размеры рассчитаны для {} из {} объектов, выбранных по оценке relpages основного отношения: объекты с небольшим heap и крупными индексами/TOAST могут отсутствовать.",
                sizes.sampled_relation_count, sizes.total_relation_count,
            ));
        }
    }

    for (title, section_error) in [
        ("Database stats", database_stats.error.as_deref()),
        ("WAL", wal.error.as_deref()),
        ("Checkpoints", checkpoints.error.as_deref()),
        ("Relation sizes", relation_sizes.error.as_deref()),
    ] {
        if let Some(error) = section_error {
            limitations.push(format!("{title}: {error}"));
        }
    }

    IoWalInterpretation { items, limitations }
}

fn unavailable_section<T>(error: &str) -> IoWalSection<T> {
    IoWalSection {
        data: None,
        error: Some(error.to_owned()),
    }
}

async fn collect_section<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    sql: &str,
) -> IoWalSection<T>
where
    T: DeserializeOwned,
{
    match query_json::<T>(internal_postgres, connection_id, input, sql).await {
        Ok(data) => IoWalSection {
            data: Some(data),
            error: None,
        },
        Err(error) => IoWalSection {
            data: None,
            error: Some(error),
        },
    }
}

async fn query_json<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    sql: &str,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let output = internal_postgres
        .run_readonly_query(
            connection_id,
            input,
            sql,
            QueryExecutionPolicy::standard_diagnostic(),
        )
        .await?;

    serde_json::from_str(&output)
        .map_err(|error| format!("PostgreSQL вернул неожиданный I/O & WAL JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_17_uses_checkpointer_view() {
        assert!(CHECKPOINTS_17_SQL.contains("pg_stat_checkpointer"));
        assert!(CHECKPOINTS_PRE17_SQL.contains("pg_stat_bgwriter"));
    }

    #[test]
    fn relation_size_collector_is_bounded() {
        let sql = relation_sizes_sql();
        assert_eq!(RELATION_SIZE_SAMPLE_LIMIT, 500);
        assert!(sql.contains("LIMIT 500"));
        assert!(sql.contains("'truncated', (SELECT count(*) FROM eligible) > 500"));
        assert!(!sql.contains("__RELATION_SIZE_LIMIT__"));
    }

    #[test]
    fn relation_size_collector_separates_toast_from_heap() {
        assert!(RELATION_SIZES_SQL.contains("pg_table_size(c.oid)::bigint"));
        assert!(RELATION_SIZES_SQL.contains("- CASE"));
        assert!(RELATION_SIZES_SQL.contains("pg_total_relation_size(cls.reltoastrelid)"));
    }

    #[test]
    fn overview_captures_checkpoint_configuration() {
        assert!(OVERVIEW_SQL.contains("checkpoint_timeout"));
        assert!(OVERVIEW_SQL.contains("checkpoint_completion_target"));
        assert!(OVERVIEW_SQL.contains("max_wal_size"));
    }

    #[test]
    fn wal_collector_captures_timing_context() {
        assert!(WAL_SQL.contains("wal_buffers_full"));
        assert!(WAL_SQL.contains("wal_write_time"));
        assert!(WAL_SQL.contains("wal_sync_time"));
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL instance (PGXRAY_TEST_*)"]
    async fn integration_io_wal_collect_readonly() {
        let input = ConnectionInput {
            host: std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST required"),
            port: std::env::var("PGXRAY_TEST_PORT")
                .unwrap_or_else(|_| "5432".into())
                .parse()
                .expect("valid test port"),
            database: std::env::var("PGXRAY_TEST_DB").unwrap_or_else(|_| "postgres".into()),
            username: std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER required"),
            password: std::env::var("PGXRAY_TEST_PASSWORD").unwrap_or_default(),
            sslmode: std::env::var("PGXRAY_TEST_SSLMODE").unwrap_or_else(|_| "disable".into()),
        };
        let registry = InternalPostgresRegistry::new();
        if std::env::var("PGXRAY_TEST_EXPECT_UNPRIVILEGED").as_deref() == Ok("1") {
            let is_unprivileged: bool = query_json(
                &registry,
                1,
                &input,
                "SELECT to_json(current_setting('is_superuser') = 'off')::text",
            )
            .await
            .expect("read privilege status");
            assert!(is_unprivileged, "test role must not be SUPERUSER");
        }
        let snapshot = collect(&registry, 1, &input)
            .await
            .expect("collect I/O & WAL");
        assert_eq!(snapshot.overview.database, input.database);
        assert!(snapshot.overview.server_version_num >= 120_000);
        if let Some(sizes) = snapshot.relation_sizes.data {
            assert!(sizes.sampled_relation_count <= RELATION_SIZE_SAMPLE_LIMIT);
            assert_eq!(sizes.relations.len() as i64, sizes.sampled_relation_count);
            assert_eq!(
                sizes.truncated,
                sizes.total_relation_count > RELATION_SIZE_SAMPLE_LIMIT
            );
        } else {
            assert!(snapshot.relation_sizes.error.is_some());
        }
    }
}
