use super::{collector, interpretation, model::PgssCapacity, queries, reference};

#[test]
fn detail_sql_accepts_only_numeric_identity() {
    let sql = collector::detail_sql(10, 20, -123, true);

    assert!(sql.contains("s.userid = 10::oid"));
    assert!(sql.contains("s.queryid = -123::bigint"));
    assert!(sql.contains("s.toplevel = true"));
}

#[test]
fn reference_uses_actual_capacity_sql() {
    let reference = reference::reference();

    let capacity = reference
        .queries
        .iter()
        .find(|query| query.id == "capacity")
        .unwrap();

    assert_eq!(capacity.sql, queries::CAPACITY_SQL.trim());
}

#[test]
fn unavailable_section_has_no_data() {
    let section = collector::unavailable_section::<PgssCapacity>("not installed");

    assert!(section.data.is_none());
    assert_eq!(section.error.as_deref(), Some("not installed"));
}

#[test]
fn performance_sql_contains_bounded_history_sample() {
    assert!(queries::PERFORMANCE_SQL.contains("'history_sample'"));
    assert!(queries::PERFORMANCE_SQL.contains("history_candidates AS MATERIALIZED"));
    assert!(queries::PERFORMANCE_SQL.contains("LIMIT 300"));
}

#[test]
fn performance_snapshot_ranks_are_bounded_to_50_without_expanding_history() {
    let sql = queries::PERFORMANCE_SQL;
    let (ranks, history) = sql.split_once("    'history_sample',").unwrap();
    assert_eq!(ranks.matches("LIMIT 50").count(), 15); // 8 history candidates + 7 performance rankings
    assert!(!ranks.contains("LIMIT 25"));
    assert!(history.contains("LIMIT 300"));
}

#[test]
fn explorer_filters_are_applied_before_limit_and_reject_invalid_ranges() {
    let filter = super::model::PgssExplorerFilter {
        userid: Some(10),
        dbid: Some(5),
        command: Some("UPDATE".to_owned()),
        one_call: true,
        min_calls: Some(1),
        max_calls: Some(3),
        min_total_exec_ms: Some(100.5),
        limit: Some(25),
        ..Default::default()
    };
    let (sql, _, _) = collector::explorer_sql(&filter).unwrap();
    for predicate in [
        "s.userid = 10::oid",
        "s.dbid = 5::oid",
        "s.calls = 1",
        "s.calls >= 1::bigint",
        "s.calls <= 3::bigint",
        "s.total_exec_time >= 100.5::double precision",
        "LIMIT 26 OFFSET 0",
    ] {
        assert!(sql.contains(predicate), "missing {predicate}");
    }
    assert!(sql.find("s.calls = 1").unwrap() < sql.find("LIMIT 26 OFFSET 0").unwrap());
    assert!(collector::explorer_sql(&super::model::PgssExplorerFilter {
        min_calls: Some(10),
        max_calls: Some(5),
        ..Default::default()
    })
    .is_err());
    assert!(collector::explorer_sql(&super::model::PgssExplorerFilter {
        min_total_exec_ms: Some(f64::NAN),
        ..Default::default()
    })
    .is_err());
}

#[test]
fn zero_dealloc_does_not_claim_deallocation_happened() {
    let meaning = interpretation::dealloc_snapshot_meaning(0);

    assert!(meaning.contains("dealloc = 0"));
    assert!(meaning.contains("не наблюдались"));
    assert!(!meaning.contains("dealloc > 0"));
}

#[test]
fn positive_dealloc_reports_observed_events_without_calling_it_a_rate() {
    let meaning = interpretation::dealloc_snapshot_meaning(7);

    assert!(meaning.contains("dealloc > 0"));
    assert!(meaning.contains("наблюдались deallocation events"));
    assert!(meaning.contains("без второго snapshot не показывает текущую скорость churn"));
}

#[test]
fn stats_reset_nullable_does_not_disable_stats_section() {
    let value = serde_json::json!({"dealloc": 0, "stats_reset": null, "stats_age_seconds": null});
    let info: super::model::PgssStatsInfo = serde_json::from_value(value).unwrap();
    assert!(info.stats_reset.is_none());
    assert!(info.stats_age_seconds.is_none());
}

#[test]
fn pgss_sections_reject_contradictory_data_and_error() {
    use super::model::PgssSection;
    let conflicting = r#"{"data":[],"error":"permission denied"}"#;
    assert!(serde_json::from_str::<PgssSection<Vec<i32>>>(conflicting).is_err());

    let legacy_unknown: PgssSection<Vec<i32>> =
        serde_json::from_str(r#"{"data":null,"error":null}"#).unwrap();
    assert!(legacy_unknown.data.is_none());
    assert!(legacy_unknown.error.is_some());

    let empty: PgssSection<Vec<i32>> = serde_json::from_str(r#"{"data":[],"error":null}"#).unwrap();
    assert_eq!(empty.data, Some(Vec::new()));
}

#[test]
fn pgss_old_snapshot_missing_sections_is_explicitly_unavailable() {
    let overview = serde_json::json!({
        "measured_at":"2026-11-15T12:00:00+03:00", "database":"postgres",
        "database_oid":5, "username":"postgres", "server_version":"16.15",
        "server_version_num":160015, "extension_installed":true,
        "extension_version":"1.10", "preloaded":true,
        "shared_preload_libraries":"pg_stat_statements", "compute_query_id":"auto",
        "max_entries":5000, "track":"top", "track_utility":true,
        "track_planning":false, "save":true, "track_io_timing":false
    });
    let snapshot: super::model::PgssSnapshot = serde_json::from_value(serde_json::json!({
        "overview": overview
    }))
    .unwrap();
    assert!(snapshot.performance.data.is_none());
    assert!(snapshot.performance.error.is_some());
    assert!(snapshot.interpretation.items.is_empty());
}

#[test]
fn epoch_check_distinguishes_reset_and_unknown() {
    use collector::{epoch_continuity, EpochContinuity};
    assert_eq!(
        epoch_continuity(Some("A"), Some("A")),
        EpochContinuity::Stable
    );
    assert_eq!(
        epoch_continuity(Some("A"), Some("B")),
        EpochContinuity::Changed
    );
    assert_eq!(
        epoch_continuity(Some("A"), None),
        EpochContinuity::Unverified
    );
    assert_eq!(epoch_continuity(None, None), EpochContinuity::Unverified);
}

#[test]
fn collector_preserves_partial_errors_and_labels_json_failures() {
    let section = collector::section_from_result::<u64>(Err("PGSS capacity: timeout".into()));
    assert!(section.data.is_none());
    assert!(section.error.unwrap().contains("capacity"));
    let error = collector::parse_json::<super::model::PgssCapacity>("capacity", "{").unwrap_err();
    assert!(error.contains("capacity"));
}

#[test]
fn detail_sql_flags_truncation_without_fetching_all_text() {
    let sql = collector::detail_sql(10, 20, -123, true);
    assert!(sql.contains("length(s.query) > 4000"));
    assert!(sql.contains("AS query_truncated"));
    let mut json = serde_json::json!({
        "userid": 10, "dbid": 20, "role_name": null, "database": "postgres",
        "toplevel": true, "queryid": "123", "query": "SELECT 1",
        "query_truncated": true, "calls": 1, "plans": 0,
        "total_plan_time_ms": 0.0, "mean_plan_time_ms": 0.0,
        "stddev_plan_time_ms": 0.0, "total_exec_time_ms": 1.0,
        "min_exec_time_ms": 1.0, "max_exec_time_ms": 1.0,
        "mean_exec_time_ms": 1.0, "stddev_exec_time_ms": 0.0,
        "rows": 1, "shared_blks_hit": 0, "shared_blks_read": 0,
        "temp_blks_read": 0, "temp_blks_written": 0, "wal_bytes": 0.0
    });
    let detail: super::model::PgssQueryDetail = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(detail.query_truncated, Some(true));
    json.as_object_mut().unwrap().remove("query_truncated");
    let old_detail: super::model::PgssQueryDetail = serde_json::from_value(json).unwrap();
    assert!(old_detail.query_truncated.is_none());
}

#[test]
fn command_labels_separate_hidden_text_and_ctes() {
    assert!(queries::COMMANDS_SQL.contains("WHEN s.query IS NULL THEN '<HIDDEN>'"));
    assert!(queries::COMMANDS_SQL.contains("THEN '<WITH/CTE>'"));
    assert!(queries::CALL_TARGETS_SQL.contains("s.query IS NOT NULL"));
}

#[test]
fn history_top_n_is_bounded_before_rank() {
    assert!(queries::PERFORMANCE_SQL.contains("history_candidates AS MATERIALIZED"));
    assert_eq!(queries::PERFORMANCE_SQL.matches("LIMIT 50").count(), 15);
    assert!(queries::PERFORMANCE_SQL.contains("LIMIT 300"));
}

// Read-only integration check. Requires pg_stat_statements installed and loaded.
// Run only against a dedicated PG instance, never an overloaded production server.
#[tokio::test]
#[ignore = "requires a test PostgreSQL with PGSS_TEST_DSN and pg_stat_statements"]
async fn pgss_sql_executes_on_test_postgres() {
    use tokio_postgres::NoTls;
    let dsn = std::env::var("PGSS_TEST_DSN").expect("set PGSS_TEST_DSN for isolated PostgreSQL");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute("SET default_transaction_read_only = on")
        .await
        .unwrap();

    let overview_json: String = client
        .query_one(queries::OVERVIEW_SQL, &[])
        .await
        .unwrap()
        .get(0);
    let overview: super::model::PgssOverview = serde_json::from_str(&overview_json).unwrap();
    assert!(
        overview.extension_installed,
        "PGSS extension must be installed in test DB"
    );

    let sqls = [
        queries::STATS_SQL,
        queries::CAPACITY_SQL,
        queries::ROLE_DATABASE_SQL,
        queries::COMMANDS_SQL,
        queries::CALL_TARGETS_SQL,
        queries::PERFORMANCE_SQL,
    ];
    for sql in sqls {
        let json: String = client.query_one(sql, &[]).await.unwrap().get(0);
        let _: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
    let sample: String = client
        .query_one(queries::PERFORMANCE_SQL, &[])
        .await
        .unwrap()
        .get(0);
    let sample: super::model::PgssPerformanceSamples = serde_json::from_str(&sample).unwrap();
    assert!(sample.history_sample.len() <= 300);
}

#[test]
fn collector_budget_skips_future_sections_after_deadline() {
    use std::time::{Duration, Instant};
    assert!(!collector::collection_budget_exhausted(Instant::now()));
    assert!(collector::collection_budget_exhausted(
        Instant::now() - Duration::from_secs(50)
    ));
}

#[test]
fn pgss_archive_roundtrip_requires_original_connection() {
    use crate::{connections::SavedConnection, history::DiagnosticStore};

    let mut store = DiagnosticStore::open(std::path::Path::new(":memory:")).unwrap();
    let connection = SavedConnection {
        id: 42,
        name: "test".into(),
        host: "127.0.0.1".into(),
        port: 5432,
        database: "postgres".into(),
        username: "reader".into(),
        sslmode: "disable".into(),
        has_password: false,
        project_id: None,
        shortcut_slot: None,
        last_probe: None,
    };
    let snapshot: super::model::PgssSnapshot = serde_json::from_value(serde_json::json!({
        "overview": {
            "measured_at": "2026-11-15T17:00:00+03:00",
            "database": "postgres",
            "database_oid": 5,
            "username": "reader",
            "server_version": "16.15",
            "server_version_num": 160015,
            "extension_installed": true,
            "extension_version": "1.10",
            "preloaded": true,
            "shared_preload_libraries": "pg_stat_statements",
            "compute_query_id": "auto",
            "max_entries": 5000,
            "track": "top",
            "track_utility": true,
            "track_planning": false,
            "save": true,
            "track_io_timing": true
        }
    }))
    .unwrap();
    let saved = store
        .save_pgss_snapshot(&connection, &snapshot, 1_000, 1_001)
        .unwrap();

    let reopened = store.pgss_snapshot(connection.id, saved.id).unwrap();
    assert_eq!(reopened.history.id, saved.id);
    assert_eq!(reopened.history.connection_id, connection.id);
    assert_eq!(reopened.snapshot.overview.database_oid, 5);
    assert_eq!(reopened.snapshot.overview.server_version, "16.15");
    assert!(reopened.snapshot.stats.data.is_none());

    let wrong_connection = store.pgss_snapshot(43, saved.id).unwrap_err();
    assert!(wrong_connection.contains("другому подключению"));
    assert!(store.pgss_snapshot(42, saved.id + 1).is_err());
}

#[test]
fn explorer_query_is_bounded_and_has_no_sql_text_projection() {
    use super::model::PgssExplorerFilter;
    let (sql, offset, limit) = collector::explorer_sql(&PgssExplorerFilter {
        one_call: true,
        userid: Some(10),
        dbid: Some(5),
        sort: Some("calls".into()),
        offset: Some(50),
        limit: Some(25),
        ..Default::default()
    })
    .unwrap();
    assert_eq!((offset, limit), (50, 25));
    assert!(sql.contains("pg_stat_statements(false) AS s"));
    assert!(sql.contains("s.calls = 1"));
    assert!(sql.contains("s.userid = 10::oid"));
    assert!(sql.contains("LIMIT 26 OFFSET 50"));
    assert!(!sql.contains("s.query AS query"));
}

#[test]
fn explorer_rejects_injection_and_unbounded_requests() {
    use super::model::PgssExplorerFilter;
    for filters in [
        PgssExplorerFilter {
            command: Some("SELECT'; DROP TABLE x".into()),
            ..Default::default()
        },
        PgssExplorerFilter {
            call_tag: Some("x' OR 1=1".into()),
            ..Default::default()
        },
        PgssExplorerFilter {
            queryid: Some("1;select".into()),
            ..Default::default()
        },
        PgssExplorerFilter {
            sort: Some("random()".into()),
            ..Default::default()
        },
        PgssExplorerFilter {
            limit: Some(101),
            ..Default::default()
        },
        PgssExplorerFilter {
            offset: Some(20001),
            ..Default::default()
        },
    ] {
        assert!(collector::explorer_sql(&filters).is_err());
    }
}

#[test]
fn explorer_call_filter_matches_existing_tag_expression() {
    use super::model::PgssExplorerFilter;
    let tag = "ebf098eff46959c81f238423e09db4e1";
    let (sql, _, _) = collector::explorer_sql(&PgssExplorerFilter {
        call_tag: Some(tag.into()),
        command: Some("CALL".into()),
        ..Default::default()
    })
    .unwrap();
    assert!(sql.contains("pg_stat_statements AS s"));
    assert!(sql.contains("md5(btrim(split_part(regexp_replace("));
    assert!(sql.contains(tag));
    assert!(!sql.contains("s.query AS query"));
}
#[test]
fn explorer_exact_total_and_page_share_a_single_bounded_readonly_query() {
    let (sql, _, _) = collector::explorer_sql(&super::model::PgssExplorerFilter {
        one_call: true,
        limit: Some(25),
        offset: Some(25),
        ..Default::default()
    })
    .unwrap();
    assert!(sql.contains("WITH matched AS MATERIALIZED"));
    assert!(sql.contains("'total', (SELECT count(*) FROM matched)"));
    assert!(sql.contains("'rows', COALESCE((SELECT json_agg(row_to_json(paged)) FROM paged)"));
    assert!(sql.contains("LIMIT 26 OFFSET 25"));
    assert!(!sql.contains("s.query AS query"));
}

#[test]
fn explorer_header_sorting_is_global_stable_and_rejects_sql_expressions() {
    use super::model::PgssExplorerFilter;
    let cases = [
        ("queryid", "asc", "s.queryid::bigint ASC NULLS LAST"),
        (
            "role",
            "asc",
            "s.role_name ASC NULLS LAST, s.database ASC NULLS LAST",
        ),
        ("calls", "asc", "s.calls ASC NULLS LAST"),
        ("total_exec", "desc", "s.total_exec_time_ms DESC NULLS LAST"),
        ("mean_exec", "asc", "s.mean_exec_time_ms ASC NULLS LAST"),
        (
            "temp",
            "desc",
            "(s.temp_blks_read + s.temp_blks_written) DESC NULLS LAST",
        ),
        ("wal", "desc", "s.wal_bytes DESC NULLS LAST"),
    ];
    for (sort, direction, expected) in cases {
        let (sql, _, _) = collector::explorer_sql(&PgssExplorerFilter {
            sort: Some(sort.into()),
            direction: Some(direction.into()),
            offset: Some(25),
            limit: Some(25),
            one_call: true,
            ..Default::default()
        })
        .unwrap();
        assert!(sql.contains(expected), "missing {expected}");
        assert!(sql.find(expected).unwrap() < sql.find("LIMIT 26 OFFSET 25").unwrap());
        assert!(sql.contains("s.userid, s.dbid, s.queryid NULLS LAST, s.toplevel DESC"));
        assert!(sql.contains("s.calls = 1"));
    }
    for direction in ["desc; DROP TABLE t", "DESC", "", "random()"] {
        assert!(collector::explorer_sql(&PgssExplorerFilter {
            direction: Some(direction.into()),
            ..Default::default()
        })
        .is_err());
    }
    assert!(collector::explorer_sql(&PgssExplorerFilter {
        sort: Some("s.calls".into()),
        ..Default::default()
    })
    .is_err());
}
