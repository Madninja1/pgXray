use crate::{
    internal_postgres::InternalPostgresRegistry,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const INDEX_SAMPLE_LIMIT: i64 = 1000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexHealthOverview {
    pub measured_at: String,
    pub database: String,
    pub database_oid: u32,
    pub username: String,
    pub server_version: String,
    pub server_version_num: i32,
    pub stats_reset: Option<String>,
    pub stats_age_seconds: Option<i64>,
    pub track_counts: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexHealthIndex {
    pub index_oid: u32,
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub index_name: String,
    pub index_size_bytes: i64,
    pub idx_scan: i64,
    pub idx_tup_read: i64,
    pub idx_tup_fetch: i64,
    pub is_valid: bool,
    pub is_ready: bool,
    pub is_primary: bool,
    pub is_unique: bool,
    pub key_columns: Vec<String>,
    pub predicate: Option<String>,
    pub index_definition: String,
    pub definition_fingerprint: String,
    pub estimated_rows: i64,
    pub tuples_inserted: i64,
    pub tuples_updated: i64,
    pub tuples_deleted: i64,
    pub hot_updates: i64,
    pub last_analyze: Option<String>,
    pub last_autoanalyze: Option<String>,
    #[serde(default)]
    pub classifications: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexHealthCollection {
    pub total_index_count: i64,
    pub sampled_index_count: i64,
    pub truncated: bool,
    pub indexes: Vec<IndexHealthIndex>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexHealthInterpretation {
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexHealthSnapshot {
    pub overview: IndexHealthOverview,
    pub collection: IndexHealthCollection,
    pub interpretation: IndexHealthInterpretation,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexHealthReferenceEntry {
    pub title: String,
    pub purpose: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexHealthReference {
    pub methodology: Vec<String>,
    pub sources: Vec<IndexHealthReferenceEntry>,
}

const INDEX_HEALTH_SQL: &str = r#"
WITH overview AS MATERIALIZED (
SELECT json_build_object(
    'measured_at', clock_timestamp(),
    'database', current_database(),
    'database_oid', d.oid::bigint,
    'username', current_user,
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'stats_reset', s.stats_reset,
    'stats_age_seconds',
        CASE WHEN s.stats_reset IS NULL THEN NULL
        ELSE GREATEST(0, extract(epoch FROM clock_timestamp() - s.stats_reset)::bigint)
        END,
    'track_counts', current_setting('track_counts') = 'on'
) AS data
FROM pg_database AS d
LEFT JOIN pg_stat_database AS s ON s.datid = d.oid
WHERE d.datname = current_database()
),
eligible AS MATERIALIZED (
    SELECT
        i.indexrelid,
        i.indrelid,
        n.nspname::text AS schema_name,
        t.relname::text AS relation_name,
        ci.relname::text AS index_name,
        GREATEST(ci.relpages, 0)::bigint AS estimated_index_pages
    FROM pg_index AS i
    JOIN pg_class AS ci ON ci.oid = i.indexrelid
    JOIN pg_class AS t ON t.oid = i.indrelid
    JOIN pg_namespace AS n ON n.oid = t.relnamespace
    WHERE t.relkind IN ('r', 'm', 'p')
      AND n.nspname NOT IN ('pg_catalog', 'information_schema')
      AND n.nspname !~ '^pg_toast'
),
candidates AS MATERIALIZED (
    SELECT *
    FROM eligible
    ORDER BY estimated_index_pages DESC, indexrelid
    LIMIT __INDEX_SAMPLE_LIMIT__
),
sampled AS (
    SELECT
        c.indexrelid::bigint AS index_oid,
        c.indrelid::bigint AS relation_oid,
        c.schema_name,
        c.relation_name,
        c.index_name,
        pg_relation_size(c.indexrelid)::bigint AS index_size_bytes,
        COALESCE(si.idx_scan, 0)::bigint AS idx_scan,
        COALESCE(si.idx_tup_read, 0)::bigint AS idx_tup_read,
        COALESCE(si.idx_tup_fetch, 0)::bigint AS idx_tup_fetch,
        i.indisvalid AS is_valid,
        i.indisready AS is_ready,
        i.indisprimary AS is_primary,
        i.indisunique AS is_unique,
        ARRAY(
            SELECT pg_get_indexdef(i.indexrelid, k, true)
            FROM generate_series(1, i.indnkeyatts) AS k
            ORDER BY k
        )::text[] AS key_columns,
        pg_get_expr(i.indpred, i.indrelid)::text AS predicate,
        pg_get_indexdef(i.indexrelid)::text AS index_definition,
        md5(concat_ws('|',
            i.indrelid::text,
            ci.relam::text,
            i.indnkeyatts::text,
            i.indnatts::text,
            i.indkey::text,
            i.indclass::text,
            i.indcollation::text,
            i.indoption::text,
            COALESCE(i.indexprs::text, ''),
            COALESCE(i.indpred::text, ''),
            i.indisunique::text,
            i.indisprimary::text
        )) AS definition_fingerprint,
        GREATEST(t.reltuples::bigint, 0) AS estimated_rows,
        COALESCE(st.n_tup_ins, 0)::bigint AS tuples_inserted,
        COALESCE(st.n_tup_upd, 0)::bigint AS tuples_updated,
        COALESCE(st.n_tup_del, 0)::bigint AS tuples_deleted,
        COALESCE(st.n_tup_hot_upd, 0)::bigint AS hot_updates,
        st.last_analyze,
        st.last_autoanalyze
    FROM candidates AS c
    JOIN pg_index AS i ON i.indexrelid = c.indexrelid
    JOIN pg_class AS ci ON ci.oid = c.indexrelid
    JOIN pg_class AS t ON t.oid = c.indrelid
    LEFT JOIN pg_stat_user_indexes AS si ON si.indexrelid = c.indexrelid
    LEFT JOIN pg_stat_user_tables AS st ON st.relid = c.indrelid
)
SELECT json_build_object(
    'overview', (SELECT data FROM overview),
    'collection', json_build_object(
        'total_index_count', (SELECT count(*)::bigint FROM eligible),
        'sampled_index_count', (SELECT count(*)::bigint FROM sampled),
        'truncated', (SELECT count(*) FROM eligible) > __INDEX_SAMPLE_LIMIT__,
        'indexes', COALESCE(
            (SELECT json_agg(row_to_json(s) ORDER BY s.index_size_bytes DESC, s.index_oid) FROM sampled AS s),
            '[]'::json
        )
    )
)::text;
"#;

#[derive(Deserialize)]
struct RawIndexHealthSnapshot {
    overview: IndexHealthOverview,
    collection: IndexHealthCollection,
}

fn index_health_sql() -> String {
    INDEX_HEALTH_SQL.replace("__INDEX_SAMPLE_LIMIT__", &INDEX_SAMPLE_LIMIT.to_string())
}

pub async fn collect(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
) -> Result<IndexHealthSnapshot, String> {
    // The overview and index list are produced by one PostgreSQL statement,
    // avoiding separate collector round trips with different statement snapshots.
    let sql = index_health_sql();
    let raw: RawIndexHealthSnapshot =
        query_json(internal_postgres, connection_id, input, &sql).await?;
    let overview = raw.overview;
    let mut collection = raw.collection;

    classify_indexes(&overview, &mut collection.indexes);

    let mut limitations = vec![
        crate::i18n::text("idx_scan=0 сам по себе не означает, что index можно удалить.", "idx_scan=0 alone does not mean that an index is safe to drop.").into(),
        "Usage counters cumulative и должны сравниваться только внутри непрерывного statistics period; single-object statistics reset может дополнительно нарушить continuity.".into(),
        crate::i18n::text("Partial/expression/constraint semantics требуют проверки DDL и workload перед любым DROP.", "Review partial, expression, and constraint semantics against DDL and workload before any DROP.").into(),
        "High write overhead — косвенный сигнал по DML таблицы, размеру и idx_scan; фактические затраты отдельного индекса не измерялись.".into(),
        "Один SQL statement уменьшает рассогласование снимка, но системные статистические счётчики PostgreSQL не являются атомарным срезом всех backend.".into(),
    ];

    if !overview.track_counts {
        limitations
            .push("track_counts=off: usage/workload evidence недостоверно или отсутствует.".into());
    }

    if overview.stats_age_seconds.is_none() {
        limitations.push("stats_reset недоступен: возраст периода статистики неизвестен. Эвристики, требующие длительной истории, отключены.".into());
    }

    if collection.truncated {
        limitations.push(format!(
            "Index sample ограничен {} кандидатами из {} (по оценке pg_class.relpages). Меньшие индексы могут быть не обследованы.",
            collection.sampled_index_count, collection.total_index_count,
        ));
    }

    Ok(IndexHealthSnapshot {
        overview,
        collection,
        interpretation: IndexHealthInterpretation { limitations },
    })
}

pub fn reference() -> IndexHealthReference {
    IndexHealthReference {
        methodology: vec![
            crate::i18n::text("Начинать с validity/ready/primary/unique, а не с idx_scan.", "Check validity, readiness, primary, and unique properties before interpreting idx_scan.").into(),
            crate::i18n::text("Usage читать вместе с возрастом statistics period и DML pressure relation.", "Interpret usage alongside the statistics period age and relation DML pressure.").into(),
            "Exact index sizes считаются только для bounded candidate sample, предварительно выбранного по pg_class.relpages, чтобы collector не вызывал size functions для всей схемы.".into(),
            "possible duplicate означает совпавший structural fingerprint и требует ручной проверки semantics/constraints.".into(),
            "high_write_overhead — только возможные дополнительные расходы на запись, оцененные по DML таблицы, размеру и idx_scan, НЕ замер per-index overhead и НЕ DROP recommendation.".into(),
            crate::i18n::text("Если stats_reset отсутствует, возраст статистики неизвестен; для возраст-зависимых меток данных недостаточно.", "Without stats_reset, the statistics period age is unknown and age-dependent classifications lack evidence.").into(),
            "Лимит выборки применяется по оценке pg_class.relpages; отсутствие индекса в выборке не означает его здоровое состояние.".into(),
        ],
        sources: vec![
            IndexHealthReferenceEntry {
                title: "pg_stat_user_indexes".into(),
                purpose: "Index scan/read/fetch cumulative statistics.".into(),
                path: "monitoring-stats.html#MONITORING-PG-STAT-ALL-INDEXES-VIEW".into(),
            },
            IndexHealthReferenceEntry {
                title: "pg_index".into(),
                purpose: "Index validity/readiness/uniqueness and catalog metadata.".into(),
                path: "catalog-pg-index.html".into(),
            },
        ],
    }
}

fn classify_indexes(overview: &IndexHealthOverview, indexes: &mut [IndexHealthIndex]) {
    let mut fingerprints: HashMap<String, usize> = HashMap::new();
    for index in indexes.iter() {
        *fingerprints
            .entry(index.definition_fingerprint.clone())
            .or_default() += 1;
    }

    for index in indexes {
        let dml = index
            .tuples_inserted
            .saturating_add(index.tuples_updated)
            .saturating_add(index.tuples_deleted);
        let sufficient_history =
            overview.track_counts && overview.stats_age_seconds.is_some_and(|age| age >= 86_400);
        let duplicate = fingerprints
            .get(&index.definition_fingerprint)
            .copied()
            .unwrap_or(0)
            > 1;

        let mut labels = Vec::new();

        if !index.is_valid || !index.is_ready {
            labels.push("needs_investigation".into());
        }

        if duplicate && !index.is_primary {
            labels.push("possible_duplicate".into());
        }

        if overview.track_counts && index.idx_scan > 0 {
            labels.push("actively_used".into());
        }

        if !sufficient_history || dml < 1_000 {
            labels.push("low_evidence".into());
        } else if index.idx_scan == 0 && dml >= 10_000 && !index.is_primary && !index.is_unique {
            labels.push("suspiciously_unused".into());
        }

        if sufficient_history
            && index.idx_scan <= 5
            && dml >= 100_000
            && index.index_size_bytes >= 64 * 1024 * 1024
            && !index.is_primary
        {
            labels.push("high_write_overhead".into());
        }

        if labels.is_empty() {
            labels.push("needs_investigation".into());
        }

        labels.sort();
        labels.dedup();
        index.classifications = labels;
    }
}

async fn query_json<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    sql: &str,
) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
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
        .map_err(|error| format!("PostgreSQL вернул неожиданный Index Health JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overview(stats_age_seconds: i64) -> IndexHealthOverview {
        IndexHealthOverview {
            measured_at: "now".into(),
            database: "db".into(),
            database_oid: 1,
            username: "user".into(),
            server_version: "17".into(),
            server_version_num: 170_000,
            stats_reset: Some("reset".into()),
            stats_age_seconds: Some(stats_age_seconds),
            track_counts: true,
        }
    }

    fn index(name: &str, fingerprint: &str) -> IndexHealthIndex {
        IndexHealthIndex {
            index_oid: 10,
            relation_oid: 20,
            schema_name: "public".into(),
            relation_name: "orders".into(),
            index_name: name.into(),
            index_size_bytes: 128 * 1024 * 1024,
            idx_scan: 0,
            idx_tup_read: 0,
            idx_tup_fetch: 0,
            is_valid: true,
            is_ready: true,
            is_primary: false,
            is_unique: false,
            key_columns: vec!["customer_id".into()],
            predicate: None,
            index_definition: "CREATE INDEX ...".into(),
            definition_fingerprint: fingerprint.into(),
            estimated_rows: 1_000_000,
            tuples_inserted: 100_000,
            tuples_updated: 100_000,
            tuples_deleted: 0,
            hot_updates: 10_000,
            last_analyze: None,
            last_autoanalyze: None,
            classifications: Vec::new(),
        }
    }

    #[test]
    fn zero_scan_with_old_stats_and_dml_is_only_a_suspicion() {
        let mut indexes = vec![index("idx_orders_customer", "one")];
        classify_indexes(&overview(8 * 86_400), &mut indexes);
        assert!(indexes[0]
            .classifications
            .contains(&"suspiciously_unused".to_owned()));
        assert!(indexes[0]
            .classifications
            .contains(&"high_write_overhead".to_owned()));
    }

    #[test]
    fn structural_duplicate_is_marked_for_investigation() {
        let mut indexes = vec![index("idx_a", "same"), index("idx_b", "same")];
        classify_indexes(&overview(8 * 86_400), &mut indexes);
        assert!(indexes.iter().all(|row| row
            .classifications
            .contains(&"possible_duplicate".to_owned())));
    }

    #[test]
    fn collector_is_bounded() {
        assert!(index_health_sql().contains("LIMIT 1000"));
        assert_eq!(INDEX_SAMPLE_LIMIT, 1000);
    }

    #[test]
    fn duplicate_fingerprint_includes_access_and_key_shape() {
        assert!(INDEX_HEALTH_SQL.contains("ci.relam::text"));
        assert!(INDEX_HEALTH_SQL.contains("i.indnkeyatts::text"));
        assert!(INDEX_HEALTH_SQL.contains("i.indnatts::text"));
    }
    #[test]
    fn unknown_statistics_age_never_produces_usage_or_write_cost_claims() {
        let mut summary = overview(0);
        summary.stats_reset = None;
        summary.stats_age_seconds = None;
        let mut indexes = vec![index("idx_orders_customer", "one")];
        classify_indexes(&summary, &mut indexes);
        let classifications = &indexes[0].classifications;
        assert!(classifications.contains(&"low_evidence".to_owned()));
        assert!(!classifications.contains(&"suspiciously_unused".to_owned()));
        assert!(!classifications.contains(&"high_write_overhead".to_owned()));
    }

    #[test]
    fn disabled_track_counts_never_produces_usage_or_write_cost_claims() {
        let mut summary = overview(8 * 86_400);
        summary.track_counts = false;
        let mut indexes = vec![index("idx_orders_customer", "one")];
        classify_indexes(&summary, &mut indexes);
        assert!(indexes[0]
            .classifications
            .contains(&"low_evidence".to_owned()));
        assert!(!indexes[0]
            .classifications
            .contains(&"high_write_overhead".to_owned()));
    }

    #[test]
    fn single_statement_contains_overview_and_bounded_collection() {
        let sql = index_health_sql();
        assert!(sql.contains("WITH overview AS MATERIALIZED"));
        assert!(sql.contains("'overview', (SELECT data FROM overview)"));
        assert!(sql.contains("'collection', json_build_object("));
        assert!(!sql.contains("__INDEX_SAMPLE_LIMIT__"));
        assert!(sql.contains("LIMIT 1000"));
        assert!(sql.contains("> 1000"));
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL instance (PGXRAY_TEST_*)"]
    async fn integration_index_health_collect_readonly() {
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
            .expect("collect Index Health");
        assert_eq!(snapshot.overview.database, input.database);
        assert!(snapshot.collection.sampled_index_count <= INDEX_SAMPLE_LIMIT);
        assert_eq!(
            snapshot.collection.indexes.len() as i64,
            snapshot.collection.sampled_index_count
        );
        assert_eq!(
            snapshot.collection.truncated,
            snapshot.collection.total_index_count > INDEX_SAMPLE_LIMIT
        );
    }
}
