use super::model::XidPreparedTransaction;
use super::{
    interpretation::interpret_snapshot,
    policy::{
        freeze_max_age_candidate, guarded_policy_apply_sql, policy_ddl_target,
        policy_ddl_target_live, validate_policy_apply_guard,
    },
    queries::RELATIONS_SQL,
    *,
};
use std::collections::HashSet;

#[test]
fn parses_database_age_list() {
    let json = r#"[
        {
            "database_oid": 5,
            "database": "postgres",
            "xid_age": 12345,
            "pct_of_global_freeze_max": 0.1
        }
    ]"#;

    let rows: Vec<XidDatabaseAge> = serde_json::from_str(json).unwrap();

    assert_eq!(rows.len(), 1,);

    assert_eq!(rows[0].database, "postgres",);

    assert_eq!(rows[0].xid_age, 12345,);
}

#[test]
fn parses_autovacuum_trigger() {
    let json = r#"[
        {
            "relation_oid": 16384,
            "schema_name": "public",
            "relation_name": "events",
            "relation_type": "r",
            "xid_age": 100000,
            "autovacuum_enabled": true,
            "estimated_rows": 1000,
            "estimated_live_rows": 1000,
            "estimated_dead_rows": 250,
            "inserts_since_vacuum": 0,
            "tuples_inserted": 1000,
            "tuples_updated": 250,
            "tuples_deleted": 0,
            "hot_updates": 50,
            "effective_vacuum_threshold": 50.0,
            "effective_vacuum_scale_factor": 0.2,
            "effective_insert_threshold": 1000.0,
            "effective_insert_scale_factor": 0.2,
            "estimated_dead_tuple_trigger": 250,
            "estimated_insert_trigger": 1200,
            "dead_trigger_reached": true,
            "insert_trigger_reached": false,
            "last_vacuum": null,
            "last_autovacuum": null,
            "stats_reset": "2026-09-10T00:00:00+00:00"
        }
    ]"#;

    let rows: Vec<XidAutovacuumTrigger> = serde_json::from_str(json).unwrap();

    assert!(rows[0].dead_trigger_reached);

    assert_eq!(rows[0].estimated_insert_trigger, Some(1200),);
}

#[test]
fn relation_age_deserializes_legacy_snapshot_shape() {
    let json = r#"{
        "relation_oid": 16384,
        "schema_name": "public",
        "relation_name": "events",
        "relation_type": "r",
        "xid_age": 100000,
        "effective_freeze_max_age": 200000000,
        "freeze_max_source": "global",
        "pct_of_freeze_max": 0.1,
        "total_size_bytes": 1024,
        "last_vacuum": null,
        "last_autovacuum": null,
        "vacuum_count": 1,
        "autovacuum_count": 2,
        "reloptions": null,
        "parent_oid": null,
        "parent_schema": null,
        "parent_relation": null,
        "parent_xid_age": null
    }"#;

    let relation: XidRelationAge = serde_json::from_str(json).unwrap();

    assert_eq!(relation.relation_oid, 16_384);
    assert!(relation.estimated_rows.is_none());
    assert!(relation.effective_freeze_min_age.is_none());
    assert!(relation.effective_freeze_table_age.is_none());
    assert!(relation.last_analyze.is_none());
    assert!(relation.modifications_since_analyze.is_none());
}

#[test]
fn section_can_report_unavailable_data() {
    let section = XidSection::<Vec<XidRelationAge>> {
        data: None,
        error: Some("permission denied".into()),
        observed_at_ms: None,
    };

    assert!(section.data.is_none());

    assert_eq!(section.error.as_deref(), Some("permission denied"),);
}

#[test]
fn reference_query_ids_are_unique() {
    let reference = reference();

    let mut ids = HashSet::new();

    for query in reference.queries {
        assert!(ids.insert(query.id,));
    }
}

#[test]
fn reference_contains_actual_sql() {
    let reference = reference();

    let relations = reference
        .queries
        .iter()
        .find(|query| query.id == "relations")
        .unwrap();

    assert_eq!(relations.sql, RELATIONS_SQL.trim(),);
}

#[test]
fn reference_query_descriptions_match_sql_contracts() {
    let reference = reference();
    let expected: &[(&str, &str, &[&str])] = &[
        (
            "overview",
            super::queries::OVERVIEW_SQL,
            &[
                "current_xid",
                "database_mxid_age",
                "autovacuum_multixact_freeze_max_age",
            ],
        ),
        (
            "databases",
            super::queries::DATABASES_SQL,
            &["mxid_age", "pct_of_global_multixact_freeze_max"],
        ),
        (
            "relations",
            RELATIONS_SQL,
            &[
                "mxid_age",
                "effective_multixact_freeze_max_age",
                "pct_of_multixact_freeze_max",
                "all_frozen_pages",
            ],
        ),
        (
            "autovacuum-triggers",
            super::queries::AUTOVACUUM_TRIGGERS_SQL,
            &[
                "thresholds_reliable",
                "estimated_dead_tuple_trigger",
                "estimated_insert_trigger",
            ],
        ),
        ("backends", super::queries::BACKENDS_SQL, &["xmin_age"]),
        ("slots", super::queries::SLOTS_SQL, &["catalog_xmin_age"]),
        (
            "prepared",
            super::queries::PREPARED_SQL,
            &["examples", "oldest_age_seconds"],
        ),
        (
            "vacuums",
            super::queries::VACUUMS_SQL,
            &["wraparound_detection", "scan_pct"],
        ),
    ];
    assert_eq!(reference.queries.len(), expected.len());
    for (id, sql, fields) in expected {
        let item = reference
            .queries
            .iter()
            .find(|item| item.id == *id)
            .unwrap();
        assert_eq!(item.sql, sql.trim(), "stale SQL description: {id}");
        for field in *fields {
            assert!(
                item.output.iter().any(|entry| entry.contains(*field)),
                "{id}: missing documented field {field}"
            );
            assert!(
                item.sql.contains(*field),
                "{id}: SQL no longer supplies {field}"
            );
        }
    }
}

#[test]
fn reference_explains_xid_multixact_and_readonly_position() {
    let reference = reference();
    let mxid = reference
        .methodology
        .iter()
        .find(|step| step.id == "multixact-age")
        .unwrap();
    assert!(mxid.purpose.contains("самостоятельных"));
    assert!(mxid
        .how_to_read
        .iter()
        .any(|line| line.contains("relminmxid")));
    assert!(mxid
        .how_to_read
        .iter()
        .any(|line| line.contains("datminmxid")));
    assert!(mxid
        .how_to_read
        .iter()
        .any(|line| line.contains("PostgreSQL 18")));

    let overview = reference
        .queries
        .iter()
        .find(|item| item.id == "overview")
        .unwrap();
    assert!(overview.how_to_read.iter().any(|line| {
        line.contains("pg_snapshot_xmax(pg_current_snapshot())") && line.contains("не XID")
    }));
    let source = reference
        .sources
        .iter()
        .find(|source| source.id == "system-info")
        .unwrap();
    assert!(source.purpose.contains("pg_snapshot_xmax()"));
    assert!(!source.purpose.contains("pg_current_xact_id()"));
}

#[test]
fn reference_documents_real_paging_limits_and_non_atomic_sections() {
    use super::collector::{paged_sql, XID_COLLECTION_BUDGET, XID_MAX_SECTION_ROWS, XID_PAGE_SIZE};
    let reference = reference();
    let limits = reference
        .methodology
        .iter()
        .find(|step| step.id == "collection-limits")
        .unwrap();
    for value in [
        XID_COLLECTION_BUDGET.as_secs().to_string(),
        XID_PAGE_SIZE.to_string(),
        XID_MAX_SECTION_ROWS.to_string(),
    ] {
        assert!(
            limits.purpose.contains(value.as_str()),
            "missing collection limit {value}"
        );
    }
    assert!(limits
        .how_to_read
        .iter()
        .any(|line| line.contains("разные моменты")));
    assert!(limits
        .how_to_read
        .iter()
        .any(|line| line.contains("Неполная выборка")));

    for (id, sql, marker) in [
        (
            "relations",
            RELATIONS_SQL,
            "/* pgxray:xid-page-relations */",
        ),
        (
            "autovacuum-triggers",
            super::queries::AUTOVACUUM_TRIGGERS_SQL,
            "/* pgxray:xid-page-triggers */",
        ),
    ] {
        let item = reference.queries.iter().find(|item| item.id == id).unwrap();
        assert!(item.sql.contains(marker));
        assert!(item.how_to_read.iter().any(|line| line.contains(marker)));
        let paged = paged_sql(sql, marker, 123).unwrap();
        assert!(paged.contains("c.oid > 123::oid"));
        assert!(paged.contains(&format!("LIMIT {XID_PAGE_SIZE}")));
        assert!(!paged.contains(marker));
    }
}

#[test]
fn reference_explains_unknown_diagnostic_data() {
    let reference = reference();
    let relations = reference
        .queries
        .iter()
        .find(|query| query.id == "relations")
        .unwrap();
    let triggers = reference
        .queries
        .iter()
        .find(|query| query.id == "autovacuum-triggers")
        .unwrap();
    let vacuums = reference
        .queries
        .iter()
        .find(|query| query.id == "vacuums")
        .unwrap();
    let prepared = reference
        .queries
        .iter()
        .find(|query| query.id == "prepared")
        .unwrap();
    assert!(relations
        .how_to_read
        .iter()
        .any(|line| { line.contains("all_frozen_pages") && line.contains("NULL") }));
    assert!(triggers
        .how_to_read
        .iter()
        .any(|line| { line.contains("thresholds_reliable=false") && line.contains("NULL") }));
    assert!(vacuums
        .how_to_read
        .iter()
        .any(|line| { line.contains("wraparound_detection=unknown") }));
    assert!(prepared
        .output
        .iter()
        .any(|value| value.contains("examples")));
}

#[test]
fn interpretation_does_not_call_unreached_triggers_a_failure() {
    let overview = test_overview();

    let relation = test_relation(90, 100);

    let trigger = test_trigger(false, false);

    let interpretation = interpret_snapshot(
        &overview,
        &section(vec![relation]),
        &section(vec![trigger]),
        &section(Vec::<XidVacuum>::new()),
        &section(Vec::<XidBackend>::new()),
        &section(Vec::<XidSlot>::new()),
        &section(XidPreparedSummary {
            count: 0,
            oldest_age_seconds: None,
            examples: vec![],
        }),
    );

    let trigger_item = interpretation
        .items
        .iter()
        .find(|item| item.id == "normal-autovacuum-trigger")
        .unwrap();

    assert!(trigger_item.meaning.iter().any(|line| {
        line.contains("не означает неисправность autovacuum")
    }));
}

#[test]
fn interpretation_reports_configured_freeze_threshold_without_local_160m_rule() {
    let overview = test_overview();

    let interpretation = interpret_snapshot(
        &overview,
        &section(vec![test_relation(100, 100)]),
        &section(vec![test_trigger(false, false)]),
        &section(Vec::<XidVacuum>::new()),
        &section(Vec::<XidBackend>::new()),
        &section(Vec::<XidSlot>::new()),
        &section(XidPreparedSummary {
            count: 0,
            oldest_age_seconds: None,
            examples: vec![],
        }),
    );

    let oldest = interpretation
        .items
        .iter()
        .find(|item| item.id == "oldest-relation")
        .unwrap();

    assert!(oldest
        .meaning
        .iter()
        .any(|line| { line.contains("configured autovacuum_freeze_max_age",) }));

    let rendered = format!("{interpretation:?}");

    assert!(!rendered.contains("160M"));

    assert!(!rendered.contains("180M"));
}

#[test]
fn interpretation_marks_backend_xmin_as_candidate_not_proof() {
    let overview = test_overview();

    let interpretation = interpret_snapshot(
        &overview,
        &section(vec![test_relation(90, 100)]),
        &section(vec![test_trigger(false, false)]),
        &section(Vec::<XidVacuum>::new()),
        &section(vec![XidBackend {
            pid: 42,
            database: Some("postgres".into()),
            username: Some("app".into()),
            application_name: None,
            state: Some("idle in transaction".into()),
            xmin_age: 80,
            transaction_age_seconds: Some(3600),
            wait_event_type: None,
            wait_event: None,
        }]),
        &section(Vec::<XidSlot>::new()),
        &section(XidPreparedSummary {
            count: 0,
            oldest_age_seconds: None,
            examples: vec![],
        }),
    );

    let blocker = interpretation
        .items
        .iter()
        .find(|item| item.id == "backend-xmin-candidate")
        .unwrap();

    assert!(blocker
        .meaning
        .iter()
        .any(|line| { line.contains("кандидатом",) }));

    assert!(blocker
        .meaning
        .iter()
        .any(|line| { line.contains("не доказывает",) }));
}

// Regression coverage for XID-INT-01..08: unknown observations must not be
// mistaken for negative evidence, and MXID is independent of XID ordering.
fn interpretation_case(
    overview: &XidOverview,
    relations: Vec<XidRelationAge>,
    triggers: Vec<XidAutovacuumTrigger>,
    vacuums: XidSection<Vec<XidVacuum>>,
    backends: Vec<XidBackend>,
    prepared: XidPreparedSummary,
) -> XidInterpretation {
    interpret_snapshot(
        overview,
        &section(relations),
        &section(triggers),
        &vacuums,
        &section(backends),
        &section(Vec::<XidSlot>::new()),
        &section(prepared),
    )
}

fn no_prepared_transactions() -> XidPreparedSummary {
    XidPreparedSummary {
        count: 0,
        oldest_age_seconds: None,
        examples: vec![],
    }
}

fn vacuum_with_detection(detection: Option<&str>) -> XidVacuum {
    XidVacuum {
        pid: 101,
        relation_oid: 16_384,
        database: "postgres".into(),
        schema_name: "public".into(),
        relation_name: "events".into(),
        phase: "scanning heap".into(),
        xid_age: 100,
        heap_blks_scanned: 0,
        heap_blks_total: 100,
        scan_pct: Some(0.0),
        backend_type: Some("autovacuum worker".into()),
        is_wraparound_vacuum: false,
        wraparound_detection: detection.map(str::to_owned),
        wait_event_type: None,
        wait_event: None,
    }
}

#[test]
fn interpretation_does_not_call_unknown_dml_thresholds_unreached() {
    let mut trigger = test_trigger(false, false);
    trigger.estimated_dead_tuple_trigger = None;
    trigger.estimated_insert_trigger = None;
    trigger.thresholds_reliable = Some(false);
    let interpretation = interpretation_case(
        &test_overview(),
        vec![test_relation(90, 100)],
        vec![trigger],
        section(Vec::new()),
        vec![],
        no_prepared_transactions(),
    );
    let normal = interpretation
        .items
        .iter()
        .find(|item| item.id == "normal-autovacuum-trigger")
        .unwrap();
    assert!(normal
        .meaning
        .iter()
        .any(|line| line.contains("порог неизвестен")));
    assert!(!normal
        .meaning
        .iter()
        .any(|line| line.starts_with("Обычные dead-tuple")));
    assert!(!interpretation
        .items
        .iter()
        .any(|item| item.id == "workload-pattern"));
}

#[test]
fn interpretation_distinguishes_disabled_insert_from_unknown_insert() {
    let mut trigger = test_trigger(false, false);
    trigger.effective_insert_threshold = -1.0;
    trigger.estimated_insert_trigger = None;
    let interpretation = interpretation_case(
        &test_overview(),
        vec![test_relation(90, 100)],
        vec![trigger],
        section(Vec::new()),
        vec![],
        no_prepared_transactions(),
    );
    let normal = interpretation
        .items
        .iter()
        .find(|item| item.id == "normal-autovacuum-trigger")
        .unwrap();
    assert!(normal
        .meaning
        .iter()
        .any(|line| line.contains("не достигнуты")));
    assert!(normal.observed.iter().any(|line| line.contains("disabled")));
}

#[test]
fn interpretation_does_not_call_missing_vacuum_section_no_worker() {
    let interpretation = interpretation_case(
        &test_overview(),
        vec![test_relation(100, 100)],
        vec![test_trigger(true, false)],
        XidSection::failure("permission denied".into(), None),
        vec![],
        no_prepared_transactions(),
    );
    assert!(interpretation
        .items
        .iter()
        .any(|item| item.id == "freeze-max-vacuum-unavailable"));
    assert!(!interpretation
        .items
        .iter()
        .any(|item| item.id == "freeze-max-no-vacuum"));
    let normal = interpretation
        .items
        .iter()
        .find(|item| item.id == "normal-autovacuum-trigger")
        .unwrap();
    assert!(normal
        .meaning
        .iter()
        .any(|line| line.contains("секция текущего VACUUM недоступна")));
    assert!(interpretation
        .limitations
        .iter()
        .any(|line| line.contains("permission denied")));
}

#[test]
fn interpretation_does_not_call_unknown_wraparound_detection_no_worker() {
    for detection in [Some("unknown"), None] {
        let interpretation = interpretation_case(
            &test_overview(),
            vec![test_relation(100, 100)],
            vec![test_trigger(false, false)],
            section(vec![vacuum_with_detection(detection)]),
            vec![],
            no_prepared_transactions(),
        );
        assert!(interpretation
            .items
            .iter()
            .any(|item| item.id == "freeze-max-vacuum-unconfirmed"));
        assert!(!interpretation
            .items
            .iter()
            .any(|item| item.id == "freeze-max-no-vacuum"));
    }
    let interpretation = interpretation_case(
        &test_overview(),
        vec![test_relation(100, 100)],
        vec![],
        section(vec![vacuum_with_detection(Some("query_text"))]),
        vec![],
        no_prepared_transactions(),
    );
    assert!(interpretation
        .items
        .iter()
        .any(|item| item.id == "freeze-max-no-vacuum"));
}

#[test]
fn interpretation_finds_high_mxid_on_relation_other_than_highest_xid() {
    let mut highest_xid = test_relation(90, 100);
    highest_xid.mxid_age = Some(10);
    highest_xid.effective_multixact_freeze_max_age = Some(100);
    let mut highest_mxid = test_relation(30, 100);
    highest_mxid.relation_oid = 16_385;
    highest_mxid.relation_name = "mxid_risk".into();
    highest_mxid.mxid_age = Some(105);
    highest_mxid.effective_multixact_freeze_max_age = Some(100);

    let interpretation = interpretation_case(
        &test_overview(),
        vec![highest_xid, highest_mxid],
        vec![],
        section(Vec::new()),
        vec![],
        no_prepared_transactions(),
    );
    let item = interpretation
        .items
        .iter()
        .find(|item| item.id == "multixact-freeze-horizon")
        .unwrap();
    assert!(item.title.contains("Достигнут"));
    assert!(item
        .observed
        .iter()
        .any(|line| line.contains("mxid_risk") && line.contains("MXID age = 105")));
}

#[test]
fn interpretation_reports_database_mxid_even_without_relation_data() {
    let mut overview = test_overview();
    overview.database_mxid_age = Some(200);
    overview.autovacuum_multixact_freeze_max_age = Some(150);
    let interpretation = interpretation_case(
        &overview,
        vec![],
        vec![],
        section(Vec::new()),
        vec![],
        no_prepared_transactions(),
    );
    assert!(interpretation.items.iter().any(|item| {
        item.id == "multixact-freeze-horizon" && item.title.contains("Достигнут")
    }));
}

#[test]
fn interpretation_warns_on_mxid_approaching_threshold_before_exceeding_it() {
    let mut relation = test_relation(30, 100);
    relation.mxid_age = Some(95);
    relation.effective_multixact_freeze_max_age = Some(100);
    let interpretation = interpretation_case(
        &test_overview(),
        vec![relation],
        vec![],
        section(Vec::new()),
        vec![],
        no_prepared_transactions(),
    );
    let item = interpretation
        .items
        .iter()
        .find(|item| item.id == "multixact-freeze-horizon")
        .unwrap();
    assert!(item.title.contains("близок"));
    assert!(item.meaning.iter().any(|line| line.contains("90%")));
}

#[test]
fn interpretation_only_flags_significant_backend_xmin() {
    let make_backend = |pid, xmin_age| XidBackend {
        pid,
        database: Some("postgres".into()),
        username: Some("app".into()),
        application_name: None,
        state: Some("active".into()),
        xmin_age,
        transaction_age_seconds: None,
        wait_event_type: None,
        wait_event: None,
    };
    let interpret = |xmin_age| {
        interpretation_case(
            &test_overview(),
            vec![test_relation(90, 100)],
            vec![],
            section(Vec::new()),
            vec![make_backend(42, xmin_age)],
            no_prepared_transactions(),
        )
    };
    assert!(!interpret(49)
        .items
        .iter()
        .any(|item| item.id == "backend-xmin-candidate"));
    assert!(interpret(50)
        .items
        .iter()
        .any(|item| item.id == "backend-xmin-candidate"));
    assert!(super::queries::BACKENDS_SQL.contains("a.pid <> pg_backend_pid()"));
}

#[test]
fn interpretation_includes_prepared_transaction_examples() {
    let prepared = XidPreparedSummary {
        count: 1,
        oldest_age_seconds: Some(300),
        examples: vec![XidPreparedTransaction {
            gid: "payment-42".into(),
            xid: "1234".into(),
            database: "postgres".into(),
            owner: "service".into(),
            prepared_at: "2026-11-14T18:00:00Z".into(),
            age_seconds: 300,
        }],
    };
    let interpretation = interpretation_case(
        &test_overview(),
        vec![],
        vec![],
        section(Vec::new()),
        vec![],
        prepared,
    );
    let item = interpretation
        .items
        .iter()
        .find(|item| item.id == "prepared-xact-candidate")
        .unwrap();
    assert!(item
        .observed
        .iter()
        .any(|line| line.contains("payment-42") && line.contains("owner=service")));
}

#[test]
fn interpretation_unknown_stats_reset_has_no_cold_workload_claim() {
    let mut trigger = test_trigger(false, false);
    trigger.stats_reset = None;
    let interpretation = interpretation_case(
        &test_overview(),
        vec![test_relation(90, 100)],
        vec![trigger],
        section(Vec::new()),
        vec![],
        no_prepared_transactions(),
    );
    let workload = interpretation
        .items
        .iter()
        .find(|item| item.id == "workload-pattern")
        .unwrap();
    assert!(workload.title.contains("окно неизвестно"));
    assert!(!workload.title.contains("static/cold"));
    assert!(workload
        .meaning
        .iter()
        .any(|line| line.contains("stats_reset неизвестен")));
}

#[test]
fn interpretation_with_known_stats_reset_retains_cumulative_pattern() {
    let mut trigger = test_trigger(false, false);
    trigger.stats_reset = Some("2026-11-14T18:00:00Z".into());
    trigger.tuples_inserted = 0;
    let interpretation = interpretation_case(
        &test_overview(),
        vec![test_relation(90, 100)],
        vec![trigger],
        section(Vec::new()),
        vec![],
        no_prepared_transactions(),
    );
    let item = interpretation
        .items
        .iter()
        .find(|item| item.id == "workload-pattern")
        .unwrap();
    assert!(item.title.contains("static/cold"));
}

#[test]
fn policy_advisor_requires_history_before_candidate() {
    let overview = test_overview();
    let relation = test_relation(140_000_000, 200_000_000);
    let samples = vec![policy_sample(1, 0, 100_000_000, 140_000_000, 0)];

    let advice = advise_relation_policy(
        &overview,
        &relation,
        Some(&test_trigger(false, false)),
        None,
        true,
        &samples,
    );

    assert_eq!(advice.status, "insufficient_history");
    assert!(advice.candidates.is_empty());
}

#[test]
fn policy_advisor_keeps_policy_when_freeze_horizon_advanced() {
    let overview = test_overview();
    let relation = test_relation(120_000_000, 200_000_000);
    let samples = vec![
        policy_sample(1, 0, 100_000_000, 118_000_000, 0),
        policy_sample(2, 1_800_000, 102_000_000, 119_000_000, 0),
        policy_sample(3, 3_600_000, 104_000_000, 120_000_000, 1),
    ];

    let advice = advise_relation_policy(
        &overview,
        &relation,
        Some(&test_trigger(false, false)),
        None,
        true,
        &samples,
    );

    assert_eq!(advice.status, "working");
    assert_eq!(advice.estimated_horizon_advance, Some(2_000_000));
    assert!(advice.candidates.is_empty());
}

#[test]
fn policy_advisor_proposes_earlier_freeze_max_for_cold_relation() {
    let overview = test_overview();
    let relation = test_relation(150_000_000, 200_000_000);
    let samples = vec![
        policy_sample(1, 0, 100_000_000, 149_000_000, 0),
        policy_sample(2, 1_800_000, 100_500_000, 149_500_000, 0),
        policy_sample(3, 3_600_000, 101_000_000, 150_000_000, 0),
    ];

    let advice = advise_relation_policy(
        &overview,
        &relation,
        Some(&test_trigger(false, false)),
        None,
        true,
        &samples,
    );

    assert_eq!(advice.status, "candidate");
    assert_eq!(
        advice.primary_candidate_id.as_deref(),
        Some("earlier-anti-wraparound")
    );
    let candidate = advice.candidates.first().unwrap();
    assert_eq!(candidate.parameter, "autovacuum_freeze_max_age");
    assert!(candidate.candidate_value > relation.xid_age);
    assert!(candidate.candidate_value < relation.effective_freeze_max_age);
    assert!(candidate.sql.contains("ALTER TABLE \"public\".\"events\""));
}

#[test]
fn policy_advisor_proposes_earlier_aggressive_vacuum_when_cycles_do_not_advance_horizon() {
    let overview = test_overview();
    let mut relation = test_relation(140_000_000, 200_000_000);
    relation.effective_freeze_table_age = Some(150_000_000);
    let samples = vec![
        policy_sample(1, 0, 100_000_000, 136_000_000, 4),
        policy_sample(2, 1_800_000, 102_000_000, 138_000_000, 5),
        policy_sample(3, 3_600_000, 104_000_000, 140_000_000, 6),
    ];

    let advice = advise_relation_policy(
        &overview,
        &relation,
        Some(&test_trigger(false, false)),
        None,
        true,
        &samples,
    );

    assert_eq!(advice.status, "candidate");
    assert_eq!(
        advice.primary_candidate_id.as_deref(),
        Some("earlier-aggressive-vacuum")
    );
    let candidate = advice.candidates.first().unwrap();
    assert_eq!(candidate.parameter, "autovacuum_freeze_table_age");
    assert!(candidate.candidate_value < 150_000_000);
}

#[test]
fn policy_advisor_rejects_null_stats_reset_even_when_counters_increase() {
    let mut overview = test_overview();
    overview.stats_reset = None;
    let relation = test_relation(140_000_000, 200_000_000);
    let mut samples = vec![
        policy_sample(1, 0, 100_000_000, 136_000_000, 4),
        policy_sample(2, 1_800_000, 102_000_000, 138_000_000, 5),
        policy_sample(3, 3_600_000, 104_000_000, 140_000_000, 6),
    ];
    for sample in &mut samples {
        sample.stats_reset = None;
    }

    let advice = advise_relation_policy(
        &overview,
        &relation,
        Some(&test_trigger(false, false)),
        None,
        true,
        &samples,
    );

    assert!(!advice.stats_period_continuous);
    assert_eq!(advice.vacuum_cycles_delta, None);
    assert!(advice.candidates.is_empty());
    assert!(advice
        .limitations
        .iter()
        .any(|line| line.contains("stats_reset = NULL") && line.contains("XID age")));
}

#[test]
fn policy_advisor_uses_latest_comparable_vacuum_segment_after_hidden_reset() {
    let overview = test_overview();
    let relation = test_relation(140_000_000, 200_000_000);
    let mut samples = vec![
        policy_sample(1, 0, 100_000_000, 136_000_000, 10),
        policy_sample(2, 1_800_000, 102_000_000, 138_000_000, 11),
        policy_sample(3, 3_600_000, 104_000_000, 139_000_000, 0),
        policy_sample(4, 5_400_000, 106_000_000, 140_000_000, 1),
    ];
    for sample in &mut samples {
        sample.stats_reset = Some("2026-09-10T00:00:00+00:00".into());
    }

    let advice = advise_relation_policy(
        &overview,
        &relation,
        Some(&test_trigger(false, false)),
        None,
        true,
        &samples,
    );

    assert!(advice.stats_period_continuous);
    assert_eq!(advice.vacuum_cycles_delta, Some(1));
    assert!(advice
        .limitations
        .iter()
        .any(|line| { line.contains("последние 2 из 4 snapshots") }));
}

#[test]
fn policy_advisor_rejects_single_snapshot_after_vacuum_counter_reset() {
    let overview = test_overview();
    let relation = test_relation(140_000_000, 200_000_000);
    let mut samples = vec![
        policy_sample(1, 0, 100_000_000, 136_000_000, 10),
        policy_sample(2, 1_800_000, 102_000_000, 138_000_000, 11),
        policy_sample(3, 3_600_000, 104_000_000, 140_000_000, 0),
    ];
    for sample in &mut samples {
        sample.stats_reset = None;
    }

    let advice = advise_relation_policy(
        &overview,
        &relation,
        Some(&test_trigger(false, false)),
        None,
        true,
        &samples,
    );

    assert!(!advice.stats_period_continuous);
    assert_eq!(advice.vacuum_cycles_delta, None);
    assert!(advice
        .limitations
        .iter()
        .any(|line| line.contains("нет двух сопоставимых")));
}

#[test]
fn policy_candidate_for_toast_targets_parent_storage_parameter() {
    let overview = test_overview();
    let mut relation = test_relation(150_000_000, 200_000_000);
    relation.relation_type = "t".into();
    relation.schema_name = "pg_toast".into();
    relation.relation_name = "pg_toast_123".into();
    relation.parent_oid = Some(123);
    relation.parent_schema = Some("public".into());
    relation.parent_relation = Some("orders".into());
    relation.parent_relation_type = Some("r".into());
    let samples = vec![
        policy_sample(1, 0, 100_000_000, 149_000_000, 0),
        policy_sample(2, 1_800_000, 100_500_000, 149_500_000, 0),
        policy_sample(3, 3_600_000, 101_000_000, 150_000_000, 0),
    ];

    let advice = advise_relation_policy(&overview, &relation, None, None, true, &samples);
    let candidate = advice.candidates.first().unwrap();

    assert_eq!(candidate.parameter, "toast.autovacuum_freeze_max_age");
    assert!(candidate.sql.contains("ALTER TABLE \"public\".\"orders\""));
}

#[test]
fn policy_apply_guard_rejects_changed_effective_value() {
    let relation = test_relation(150_000_000, 200_000_000);
    let candidate = freeze_max_age_candidate(&test_overview(), &relation, 1_000_000.0).unwrap();
    let mut live = test_live_policy_state(&relation);
    live.effective_freeze_max_age = 190_000_000;

    let error = validate_policy_apply_guard(&relation, &live, &candidate).unwrap_err();

    assert!(error.contains("Freeze policy изменилась"));
}

#[test]
fn policy_apply_guard_rejects_active_vacuum() {
    let relation = test_relation(150_000_000, 200_000_000);
    let candidate = freeze_max_age_candidate(&test_overview(), &relation, 1_000_000.0).unwrap();
    let mut live = test_live_policy_state(&relation);
    live.active_vacuum_pid = Some(42);
    live.active_vacuum_phase = Some("scanning heap".into());

    let error = validate_policy_apply_guard(&relation, &live, &candidate).unwrap_err();

    assert!(error.contains("VACUUM PID 42"));
}

#[test]
fn policy_apply_guard_rejects_stale_freeze_max_candidate() {
    let relation = test_relation(150_000_000, 200_000_000);
    let candidate = freeze_max_age_candidate(&test_overview(), &relation, 1_000_000.0).unwrap();
    let mut live = test_live_policy_state(&relation);
    live.xid_age = candidate.candidate_value;

    let error = validate_policy_apply_guard(&relation, &live, &candidate).unwrap_err();

    assert!(error.contains("candidate устарел"));
}

#[test]
fn policy_live_ddl_for_toast_targets_parent() {
    let mut relation = test_relation(150_000_000, 200_000_000);
    relation.relation_type = "t".into();
    relation.schema_name = "pg_toast".into();
    relation.relation_name = "pg_toast_123".into();
    relation.parent_oid = Some(123);
    relation.parent_schema = Some("public".into());
    relation.parent_relation = Some("orders".into());
    relation.parent_relation_type = Some("r".into());
    let live = test_live_policy_state(&relation);
    let ddl = policy_ddl_target_live(&live).unwrap();

    assert_eq!(
        ddl.parameter("autovacuum_freeze_max_age"),
        "toast.autovacuum_freeze_max_age"
    );
    assert!(ddl
        .sql("toast.autovacuum_freeze_max_age", 170_000_000)
        .contains("ALTER TABLE \"public\".\"orders\""));
}

#[test]
fn policy_live_sql_uses_same_effective_caps_as_collector() {
    let sql = super::policy::relation_policy_live_sql(16_384);
    assert!(sql.contains("LEAST(COALESCE(opts.freeze_max_age, cfg.freeze_max_age)"));
    assert!(sql.contains("floor(0.95 * LEAST(COALESCE(opts.freeze_max_age"));
    assert!(sql.contains("AS postmaster_start_time"));
    assert!(super::queries::RELATIONS_SQL.contains("floor(0.95 * LEAST("));
}

#[test]
fn policy_advisor_rejects_unavailable_vacuum_section() {
    let overview = test_overview();
    let relation = test_relation(150_000_000, 200_000_000);
    let samples = vec![
        policy_sample(1, 0, 100_000_000, 149_000_000, 0),
        policy_sample(2, 1_800_000, 100_500_000, 149_500_000, 0),
        policy_sample(3, 3_600_000, 101_000_000, 150_000_000, 0),
    ];
    let advice = advise_relation_policy(&overview, &relation, None, None, false, &samples);
    assert_eq!(advice.status, "insufficient_quality");
    assert!(advice.candidates.is_empty());
    assert!(advice
        .limitations
        .iter()
        .any(|s| s.contains("Секция текущего VACUUM недоступна")));
}

#[test]
fn policy_advisor_rejects_history_from_previous_server_lifetime() {
    let overview = test_overview();
    let relation = test_relation(150_000_000, 200_000_000);
    let mut samples = vec![
        policy_sample(1, 0, 100_000_000, 149_000_000, 0),
        policy_sample(2, 1_800_000, 100_500_000, 149_500_000, 0),
        policy_sample(3, 3_600_000, 101_000_000, 150_000_000, 0),
    ];
    samples[1].postmaster_start_time = Some("previous-server-epoch".into());
    let advice = advise_relation_policy(&overview, &relation, None, None, true, &samples);
    assert!(advice.candidates.is_empty());
    assert_eq!(advice.status, "insufficient_history");
    assert!(advice
        .limitations
        .iter()
        .any(|s| s.contains("pg_postmaster_start_time")));
}

#[test]
fn policy_advisor_rejects_legacy_history_without_server_lifetime() {
    let mut overview = test_overview();
    overview.postmaster_start_time = None;
    let relation = test_relation(150_000_000, 200_000_000);
    let samples = vec![
        policy_sample(1, 0, 100_000_000, 149_000_000, 0),
        policy_sample(2, 1_800_000, 100_500_000, 149_500_000, 0),
        policy_sample(3, 3_600_000, 101_000_000, 150_000_000, 0),
    ];
    let advice = advise_relation_policy(&overview, &relation, None, None, true, &samples);
    assert_eq!(advice.status, "insufficient_history");
    assert!(advice.candidates.is_empty());
}

#[test]
fn policy_advisor_does_not_treat_cycle_overflow_as_zero() {
    let overview = test_overview();
    let relation = test_relation(150_000_000, 200_000_000);
    let mut samples = vec![
        policy_sample(1, 0, 100_000_000, 149_000_000, i64::MAX - 2),
        policy_sample(2, 1_800_000, 100_500_000, 149_500_000, i64::MAX - 1),
        policy_sample(3, 3_600_000, 101_000_000, 150_000_000, i64::MAX),
    ];
    // Sum of the monotonic counters overflows even though each field is valid.
    for sample in &mut samples {
        sample.vacuum_count = 10;
    }
    let advice = advise_relation_policy(&overview, &relation, None, None, true, &samples);
    assert_eq!(advice.status, "insufficient_quality");
    assert!(advice.candidates.is_empty());
    assert_eq!(advice.vacuum_cycles_delta, None);
    assert!(advice
        .limitations
        .iter()
        .any(|s| s.contains("переполнились")));
}

#[test]
fn policy_guard_rejects_different_server_epoch() {
    let relation = test_relation(140_000_000, 200_000_000);
    let candidate = freeze_max_age_candidate(&test_overview(), &relation, 1_000_000.0).unwrap();
    let mut live = test_live_policy_state(&relation);
    live.postmaster_start_time = Some("replacement-postmaster".into());
    let error = validate_policy_apply_guard(&relation, &live, &candidate).unwrap_err();
    assert!(error.contains("перезапущен или сменился"));
}

#[test]
fn policy_advisor_explains_multixact_is_outside_xid_apply_scope() {
    let overview = test_overview();
    let mut relation = test_relation(150_000_000, 200_000_000);
    relation.mxid_age = Some(400_000_000);
    relation.effective_multixact_freeze_max_age = Some(400_000_000);
    let advice = advise_relation_policy(&overview, &relation, None, None, true, &[]);
    assert!(advice
        .limitations
        .iter()
        .any(|s| s.contains("MultiXact freeze threshold достигнут")));
    assert!(advice
        .limitations
        .iter()
        .any(|s| s.contains("только XID freeze policy")));
}

#[test]
fn policy_advisor_rejects_replica_to_primary_history_transition() {
    let overview = test_overview();
    let relation = test_relation(150_000_000, 200_000_000);
    let mut samples = vec![
        policy_sample(1, 0, 100_000_000, 149_000_000, 0),
        policy_sample(2, 1_800_000, 100_500_000, 149_500_000, 0),
        policy_sample(3, 3_600_000, 101_000_000, 150_000_000, 0),
    ];
    samples[0].in_recovery = Some(true);
    let advice = advise_relation_policy(&overview, &relation, None, None, true, &samples);
    assert!(advice.candidates.is_empty());
    assert_eq!(advice.status, "insufficient_history");
}

#[test]
fn policy_sql_quotes_unusual_identifiers_and_literals() {
    let mut relation = test_relation(150_000_000, 200_000_000);
    relation.schema_name = "test\".schema".into();
    relation.relation_name = "it's \"odd\" $pgxray$ \\foo".into();
    let candidate = freeze_max_age_candidate(&test_overview(), &relation, 1_000_000.0).unwrap();
    let ddl = policy_ddl_target(&relation).unwrap();
    let sql = guarded_policy_apply_sql(5, &relation, &candidate, &ddl).unwrap();
    assert!(sql.contains("\"test\"\".schema\""));
    assert!(sql.contains("\"it's \"\"odd\"\" $pgxray$ \\foo\""));
    assert!(sql.contains("E'it\\'s \"odd\" $pgxray$ \\\\foo'"));
    assert!(sql.contains("DO $pgxray_0$"));
    assert!(sql.contains("pgXray guard failed: server epoch changed"));
    assert!(sql.contains("* 95 / 100"));
}

#[test]
fn guarded_policy_sql_rechecks_identity_policy_and_vacuum() {
    let relation = test_relation(150_000_000, 200_000_000);
    let candidate = freeze_max_age_candidate(&test_overview(), &relation, 1_000_000.0).unwrap();
    let ddl = policy_ddl_target(&relation).unwrap();
    let sql = guarded_policy_apply_sql(5, &relation, &candidate, &ddl).unwrap();

    assert!(sql.contains("LOCK TABLE ONLY \"public\".\"events\" IN SHARE UPDATE EXCLUSIVE MODE"));
    assert!(sql.contains("pgXray guard failed: database identity changed"));
    assert!(sql.contains("5::oid"));
    assert!(sql.contains("pgXray guard failed: freeze policy changed"));
    assert!(sql.contains("FROM pg_stat_progress_vacuum"));
    assert!(sql.contains("ALTER TABLE \"public\".\"events\""));
}

fn test_live_policy_state(relation: &XidRelationAge) -> XidPolicyLiveState {
    XidPolicyLiveState {
        postmaster_start_time: Some("test-server-epoch".into()),
        in_recovery: Some(false),
        relation_oid: relation.relation_oid,
        schema_name: relation.schema_name.clone(),
        relation_name: relation.relation_name.clone(),
        relation_type: relation.relation_type.clone(),
        xid_age: relation.xid_age,
        effective_freeze_max_age: relation.effective_freeze_max_age,
        freeze_max_source: relation.freeze_max_source.clone(),
        effective_freeze_table_age: relation.effective_freeze_table_age,
        freeze_table_source: relation.freeze_table_source.clone(),
        effective_freeze_min_age: relation.effective_freeze_min_age,
        freeze_min_source: relation.freeze_min_source.clone(),
        parent_oid: relation.parent_oid,
        parent_schema: relation.parent_schema.clone(),
        parent_relation: relation.parent_relation.clone(),
        parent_relation_type: relation.parent_relation_type.clone(),
        active_vacuum_pid: None,
        active_vacuum_phase: None,
    }
}

fn policy_sample(
    snapshot_id: i64,
    collection_started_at_ms: i64,
    current_xid: u64,
    relation_xid_age: i64,
    vacuum_cycles: i64,
) -> XidPolicyHistorySample {
    XidPolicyHistorySample {
        snapshot_id,
        collection_started_at_ms,
        current_xid: current_xid.to_string(),
        postmaster_start_time: Some("test-server-epoch".into()),
        in_recovery: Some(false),
        stats_reset: Some("2026-09-10T00:00:00+00:00".into()),
        relation_xid_age,
        vacuum_count: 0,
        autovacuum_count: vacuum_cycles,
    }
}

fn section<T>(data: T) -> XidSection<T> {
    XidSection::success(data, None)
}

fn test_overview() -> XidOverview {
    XidOverview {
        measured_at: "2026-09-10T00:00:00+00:00".into(),
        database: "postgres".into(),
        database_oid: 5,
        username: "postgres".into(),
        server_version: "16.10".into(),
        server_version_num: 160010,
        postmaster_start_time: Some("test-server-epoch".into()),
        in_recovery: Some(false),
        current_xid: "1000".into(),
        autovacuum: true,
        autovacuum_freeze_max_age: 200_000_000,
        vacuum_freeze_table_age: 150_000_000,
        vacuum_freeze_min_age: 50_000_000,
        autovacuum_max_workers: 3,
        autovacuum_vacuum_cost_limit: 200,
        autovacuum_vacuum_cost_delay: "2ms".into(),
        database_xid_age: 100,
        stats_reset: None,
        database_mxid_age: None,
        autovacuum_multixact_freeze_max_age: None,
    }
}

fn test_relation(xid_age: i64, freeze_max: i64) -> XidRelationAge {
    XidRelationAge {
        relation_oid: 16_384,
        schema_name: "public".into(),
        relation_name: "events".into(),
        relation_type: "r".into(),
        xid_age,
        mxid_age: None,
        effective_multixact_freeze_max_age: None,
        pct_of_multixact_freeze_max: None,
        effective_freeze_max_age: freeze_max,
        freeze_max_source: "global".into(),
        pct_of_freeze_max: Some(xid_age as f64 / freeze_max as f64 * 100.0),
        total_size_bytes: 1024,
        estimated_rows: Some(1000),
        heap_pages: Some(8),
        all_visible_pages: Some(4),
        all_frozen_pages: None,
        all_visible_pct: Some(50.0),
        effective_freeze_min_age: Some(50),
        freeze_min_source: Some("global".into()),
        effective_freeze_table_age: Some(75),
        freeze_table_source: Some("global".into()),
        last_vacuum: None,
        last_autovacuum: None,
        last_analyze: None,
        last_autoanalyze: None,
        vacuum_count: 0,
        autovacuum_count: 0,
        analyze_count: Some(0),
        autoanalyze_count: Some(0),
        modifications_since_analyze: Some(0),
        reloptions: None,
        parent_oid: None,
        parent_schema: None,
        parent_relation: None,
        parent_relation_type: None,
        parent_xid_age: None,
    }
}

fn test_trigger(dead: bool, insert: bool) -> XidAutovacuumTrigger {
    XidAutovacuumTrigger {
        relation_oid: 16_384,
        schema_name: "public".into(),
        relation_name: "events".into(),
        relation_type: "r".into(),
        xid_age: 90,
        autovacuum_enabled: true,
        estimated_rows: Some(1000),
        thresholds_reliable: Some(true),
        estimated_live_rows: 1000,
        estimated_dead_rows: if dead { 300 } else { 10 },
        inserts_since_vacuum: if insert { 1300 } else { 10 },
        tuples_inserted: 10,
        tuples_updated: 0,
        tuples_deleted: 0,
        hot_updates: 0,
        effective_vacuum_threshold: 50.0,
        effective_vacuum_scale_factor: 0.2,
        effective_insert_threshold: 1000.0,
        effective_insert_scale_factor: 0.2,
        estimated_dead_tuple_trigger: Some(250),
        estimated_insert_trigger: Some(1200),
        dead_trigger_reached: dead,
        insert_trigger_reached: insert,
        last_vacuum: None,
        last_autovacuum: None,
        stats_reset: None,
    }
}

#[test]
fn xid_overview_sql_uses_readonly_xid_position() {
    use super::queries::OVERVIEW_SQL;
    assert!(OVERVIEW_SQL.contains("pg_snapshot_xmax(pg_current_snapshot())::text"));
    assert!(!OVERVIEW_SQL.contains("pg_current_xact_id()"));
}

#[test]
fn xid_policy_requires_three_comparable_points_after_counter_reset() {
    let overview = test_overview();
    let relation = test_relation(140_000_000, 200_000_000);
    let samples = vec![
        policy_sample(1, 0, 100_000_000, 136_000_000, 3),
        policy_sample(2, 1_800_000, 102_000_000, 138_000_000, 4),
        policy_sample(3, 3_600_000, 104_000_000, 139_000_000, 0),
        policy_sample(4, 5_400_000, 106_000_000, 140_000_000, 1),
    ];
    let advice = advise_relation_policy(&overview, &relation, None, None, true, &samples);
    assert!(advice.stats_period_continuous);
    assert_eq!(advice.vacuum_cycles_delta, Some(1));
    assert!(advice.candidates.is_empty()); // Only 2 comparable points in last segment
    assert_eq!(advice.status, "insufficient_history");
}

// Run against PostgreSQL 16, 17 and 18 with PGXRAY_TEST_* set.
// Uses a read-only transaction and does not create or modify DB objects.
#[tokio::test]
#[ignore = "requires PostgreSQL 16/17/18 with PGXRAY_TEST_* variables"]
async fn integration_xid_readonly_overview_does_not_assign_transaction_id() {
    use crate::{internal_postgres::InternalPostgresRegistry, postgres::ConnectionInput};
    let input = ConnectionInput {
        host: std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST required"),
        port: std::env::var("PGXRAY_TEST_PORT")
            .unwrap_or_else(|_| "5432".into())
            .parse()
            .expect("invalid port"),
        database: std::env::var("PGXRAY_TEST_DB").unwrap_or_else(|_| "postgres".into()),
        username: std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER required"),
        password: std::env::var("PGXRAY_TEST_PASSWORD").unwrap_or_default(),
        sslmode: std::env::var("PGXRAY_TEST_SSLMODE").unwrap_or_else(|_| "disable".into()),
    };
    let registry = InternalPostgresRegistry::new();
    let overview: XidOverview =
        super::collector::query_json(&registry, 1044, &input, super::queries::OVERVIEW_SQL)
            .await
            .expect("XID Overview must work within BEGIN READ ONLY");
    assert!(!overview.current_xid.is_empty());
    let evidence: serde_json::Value = super::collector::query_json(
        &registry,
        1044,
        &input,
        "SELECT json_build_object('snapshot_xmax', pg_snapshot_xmax(pg_current_snapshot())::text, 'assigned', pg_current_xact_id_if_assigned()::text)::text",
    )
    .await
    .expect("read-only XID snapshot query");
    assert!(
        evidence["assigned"].is_null(),
        "read-only probe allocated an XID"
    );
    assert!(evidence["snapshot_xmax"].is_string());
}

#[test]
fn xid_queries_follow_pg18_autovacuum_thresholds() {
    let sql = super::queries::AUTOVACUUM_TRIGGERS_SQL;
    assert!(sql.contains("autovacuum_vacuum_max_threshold"));
    assert!(sql.contains("server_version_num >= 180000"));
    assert!(sql.contains("to_jsonb(c)->>'relallfrozen'"));
    assert!(sql.contains("LEAST(e.relallfrozen, e.relpages)"));
    assert!(sql.contains("floor("));
    assert!(sql.contains("+ 1"));
    assert!(sql.contains("FROM thresholds AS e"));
    assert!(sql.contains("thresholds_reliable"));
}

#[test]
fn xid_relations_include_multixact_without_breaking_old_snapshots() {
    let sql = super::queries::RELATIONS_SQL;
    assert!(sql.contains("mxid_age(e.relminmxid)"));
    assert!(sql.contains("effective_multixact_freeze_max_age"));
    assert!(sql.contains("AS all_frozen_pages"));
    assert!(sql.contains("LEAST(COALESCE(opts.table_freeze_min_age"));
    assert!(sql.contains("cfg.freeze_max_age / 2"));
    assert!(sql.contains("0.95 *"));

    // Legacy records stored without MXID and all-frozen data must remain readable.
    let old = serde_json::json!({
        "relation_oid": 123, "schema_name": "public", "relation_name": "events",
        "relation_type": "r", "xid_age": 100,
        "effective_freeze_max_age": 200000000, "freeze_max_source": "global",
        "pct_of_freeze_max": 0.0, "total_size_bytes": 100,
        "estimated_rows": null, "heap_pages": null,
        "all_visible_pages": null, "all_visible_pct": null,
        "effective_freeze_min_age": null, "freeze_min_source": null,
        "effective_freeze_table_age": null, "freeze_table_source": null,
        "last_vacuum": null, "last_autovacuum": null, "last_analyze": null,
        "last_autoanalyze": null, "vacuum_count": 0, "autovacuum_count": 0,
        "analyze_count": null, "autoanalyze_count": null,
        "modifications_since_analyze": null, "reloptions": null,
        "parent_oid": null, "parent_schema": null, "parent_relation": null,
        "parent_relation_type": null, "parent_xid_age": null
    });
    let relation: XidRelationAge = serde_json::from_value(old).unwrap();
    assert_eq!(relation.mxid_age, None);
    assert_eq!(relation.all_frozen_pages, None);
}

#[test]
fn xid_unknown_reltuples_can_be_deserialized_as_missing() {
    let mut value = serde_json::to_value(test_trigger(false, false)).unwrap();
    value["estimated_rows"] = serde_json::Value::Null;
    value["estimated_dead_tuple_trigger"] = serde_json::Value::Null;
    value["thresholds_reliable"] = serde_json::Value::Bool(false);
    let trigger: XidAutovacuumTrigger = serde_json::from_value(value).unwrap();
    assert_eq!(trigger.estimated_rows, None);
    assert_eq!(trigger.estimated_dead_tuple_trigger, None);
    assert_eq!(trigger.thresholds_reliable, Some(false));
}

#[test]
fn xid_section_rejects_ambiguous_json_but_accepts_legacy_shape() {
    let success: XidSection<Vec<i64>> =
        serde_json::from_str(r#"{"data":[1,2],"error":null}"#).unwrap();
    assert_eq!(success.data, Some(vec![1, 2]));
    assert!(success.observed_at_ms.is_none());

    let failed: XidSection<Vec<i64>> =
        serde_json::from_str(r#"{"data":null,"error":"permission denied"}"#).unwrap();
    assert!(failed.data.is_none());
    assert!(failed.error.is_some());

    for invalid in [
        r#"{"data":null,"error":null}"#,
        r#"{"data":[1],"error":"unexpected"}"#,
    ] {
        assert!(serde_json::from_str::<XidSection<Vec<i64>>>(invalid).is_err());
    }
}

#[test]
fn xid_collector_errors_have_stable_section_ids() {
    use super::collector::explain_section_error;
    let timeout = explain_section_error("relations", "statement timeout");
    assert!(timeout.contains("[XID-COL-TIMEOUT/relations]"));
    let too_large = explain_section_error("databases", "output слишком большой");
    assert!(too_large.contains("[XID-COL-OUTPUT-LIMIT/databases]"));
    let denied = explain_section_error("slots", "permission denied (42501)");
    assert!(denied.contains("[XID-COL-QUERY-ERROR/slots]"));
    assert!(denied.contains("permission denied"));
}

#[test]
fn xid_prepared_summary_accepts_legacy_json() {
    let old: XidPreparedSummary =
        serde_json::from_str(r#"{"count":3,"oldest_age_seconds":1234}"#).unwrap();
    assert_eq!(old.count, 3);
    assert!(old.examples.is_empty());
    assert!(super::queries::PREPARED_SQL.contains("LIMIT 10"));
}

// Run against PostgreSQL 16/17/18 with PGXRAY_TEST_*.
// A deliberately invalid optional query tests the actual partial collector path and SQLite save.
#[tokio::test]
#[ignore = "requires PostgreSQL 16/17/18 with PGXRAY_TEST_* variables"]
async fn integration_xid_collect_partial_failure_and_save_history() {
    use crate::{
        connections::SavedConnection, history::DiagnosticStore,
        internal_postgres::InternalPostgresRegistry, postgres::ConnectionInput,
    };
    let input = ConnectionInput {
        host: std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST required"),
        port: std::env::var("PGXRAY_TEST_PORT")
            .unwrap_or_else(|_| "5432".into())
            .parse()
            .expect("invalid PGXRAY_TEST_PORT"),
        database: std::env::var("PGXRAY_TEST_DB").unwrap_or_else(|_| "postgres".into()),
        username: std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER required"),
        password: std::env::var("PGXRAY_TEST_PASSWORD").unwrap_or_default(),
        sslmode: std::env::var("PGXRAY_TEST_SSLMODE").unwrap_or_else(|_| "disable".into()),
    };
    let registry = InternalPostgresRegistry::new();
    let sql = super::collector::XidSqlSet {
        slots: "SELECT xid_collector_nonexistent_column FROM pg_replication_slots",
        ..Default::default()
    };
    let snapshot = super::collector::collect_with_queries(&registry, 1047, &input, sql)
        .await
        .expect("optional section failure must not abort XID collection");
    assert!(snapshot.slots.data.is_none());
    assert!(snapshot
        .slots
        .error
        .as_deref()
        .unwrap_or("")
        .contains("/slots]"));
    assert!(
        snapshot.databases.data.is_some(),
        "{:?}",
        snapshot.databases.error
    );
    assert!(
        snapshot.relations.data.is_some(),
        "{:?}",
        snapshot.relations.error
    );
    assert!(snapshot
        .interpretation
        .limitations
        .iter()
        .any(|message| message.contains("последовательных")));

    let directory = tempfile::tempdir().expect("temp history directory");
    let mut store = DiagnosticStore::open(&directory.path().join("xid-test.db"))
        .expect("init diagnostic history");
    let saved_connection = SavedConnection {
        id: 1047,
        name: "temporary XID integration".into(),
        host: input.host.clone(),
        port: input.port,
        database: input.database.clone(),
        username: input.username.clone(),
        sslmode: input.sslmode.clone(),
        has_password: false,
        project_id: None,
        shortcut_slot: None,
        last_probe: None,
    };
    let stored = store
        .save_xid_snapshot(&saved_connection, &snapshot, 1_000, 2_000)
        .expect("partial snapshot must persist");
    assert!(stored.unavailable_sections >= 1);
    assert_eq!(
        store
            .list_xid_snapshots(Some(1047), Some(10))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn xid_relation_kinds_are_explicit_at_policy_boundary() {
    use super::model::XidRelationKind;
    assert_eq!(
        XidRelationKind::from_catalog("r"),
        Some(XidRelationKind::Table)
    );
    assert_eq!(
        XidRelationKind::from_catalog("m"),
        Some(XidRelationKind::MaterializedView)
    );
    assert_eq!(
        XidRelationKind::from_catalog("t"),
        Some(XidRelationKind::Toast)
    );
    assert!(XidRelationKind::from_catalog("unknown").is_none());
    let mut relation = test_relation(100, 200);
    relation.relation_type = "unexpected".into();
    assert!(policy_ddl_target(&relation).is_none());
}

#[test]
fn xid_keyset_paging_restricts_base_relations_before_joins() {
    use super::collector::paged_sql;
    use super::queries::{AUTOVACUUM_TRIGGERS_SQL, RELATIONS_SQL};
    let relations = paged_sql(RELATIONS_SQL, "/* pgxray:xid-page-relations */", 16384)
        .expect("XID relations paging marker");
    assert!(relations.contains("AND c.oid > 16384::oid ORDER BY c.oid LIMIT 256"));
    assert!(!relations.contains("/* pgxray:xid-page-relations */"));
    assert!(relations.contains("FROM relations AS r"));
    let triggers = paged_sql(AUTOVACUUM_TRIGGERS_SQL, "/* pgxray:xid-page-triggers */", 0)
        .expect("XID triggers paging marker");
    assert!(triggers.contains("AND c.oid > 0::oid ORDER BY c.oid LIMIT 256"));
    assert!(triggers.contains("FROM targets AS t"));
    assert!(paged_sql(RELATIONS_SQL, "/* missing */", 0).is_err());
    assert!(paged_sql("/* same */ /* same */", "/* same */", 0).is_err());
}

async fn read_policy_live_for_test(
    client: &tokio_postgres::Client,
    oid: u32,
) -> XidPolicyLiveState {
    let sql = super::policy::relation_policy_live_sql(oid);
    let row = client
        .query_one(sql.as_str(), &[])
        .await
        .expect("live SQL must run");
    let json: String = row.get(0);
    serde_json::from_str::<XidPolicyLiveState>(&json).expect("live state JSON")
}

// Run ONLY against an isolated disposable PostgreSQL 16/17/18 test database.
// DDL is executed inside one transaction, which is rolled back.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL with CREATE SCHEMA privilege and PGXRAY_TEST_* variables"]
async fn integration_xid_guarded_policy_apply_with_unusual_identifiers() {
    use tokio_postgres::NoTls;

    let mut config = tokio_postgres::Config::new();
    let host = std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST required");
    let user = std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER required");
    config
        .host(&host)
        .user(&user)
        .dbname(std::env::var("PGXRAY_TEST_DB").unwrap_or_else(|_| "postgres".into()))
        .port(
            std::env::var("PGXRAY_TEST_PORT")
                .unwrap_or_else(|_| "5432".into())
                .parse()
                .expect("invalid PGXRAY_TEST_PORT"),
        );
    if let Ok(password) = std::env::var("PGXRAY_TEST_PASSWORD") {
        config.password(password);
    }
    let (client, connection) = config.connect(NoTls).await.expect("PostgreSQL connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("pgxray_policy_guard_{}_{}", std::process::id(), now);
    let table = "odd\"table's_$pgxray$_\\name";
    let qualified = format!("\"{schema}\".\"{}\"", table.replace('\"', "\"\""));

    client
        .batch_execute("BEGIN")
        .await
        .expect("start rollback-only transaction");
    client
        .batch_execute(&format!(
            "CREATE SCHEMA \"{schema}\"; CREATE TABLE {qualified} (n bigint) WITH (autovacuum_freeze_table_age = 80000000);"
        ))
        .await
        .expect("create test object with quoted identifier");

    let oid: i64 = client
        .query_one(
            "SELECT c.oid::bigint FROM pg_class AS c JOIN pg_namespace AS n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await.expect("table oid").get(0);
    let database_oid: i64 = client
        .query_one(
            "SELECT oid::bigint FROM pg_database WHERE datname = current_database()",
            &[],
        )
        .await
        .expect("database oid")
        .get(0);

    let before = read_policy_live_for_test(&client, oid as u32).await;
    let mut anchor = test_relation(before.xid_age, before.effective_freeze_max_age);
    anchor.relation_oid = before.relation_oid;
    anchor.schema_name = before.schema_name.clone();
    anchor.relation_name = before.relation_name.clone();
    anchor.relation_type = before.relation_type.clone();
    anchor.freeze_max_source = before.freeze_max_source.clone();
    anchor.freeze_table_source = before.freeze_table_source.clone();
    anchor.effective_freeze_table_age = before.effective_freeze_table_age;
    anchor.freeze_min_source = before.freeze_min_source.clone();
    anchor.effective_freeze_min_age = before.effective_freeze_min_age;
    let ddl = policy_ddl_target_live(&before).expect("supported regular table");
    let candidate_value = 40_000_000;
    let parameter = ddl.parameter("autovacuum_freeze_table_age");
    let candidate = XidPolicyCandidate {
        id: "integration-test".into(),
        title: "integration-test".into(),
        expected_postmaster_start_time: before.postmaster_start_time.clone().expect("server epoch"),
        expected_in_recovery: before.in_recovery.expect("recovery state"),
        parameter: parameter.clone(),
        current_value: before.effective_freeze_table_age.expect("freeze table age"),
        candidate_value,
        ddl_target: format!("{}.{}", before.schema_name, before.relation_name),
        sql: ddl.sql(&parameter, candidate_value),
        why: vec![],
        expected_effect: vec![],
        tradeoffs: vec![],
    };
    validate_policy_apply_guard(&anchor, &before, &candidate).expect("preflight");
    let guarded = guarded_policy_apply_sql(database_oid as u32, &anchor, &candidate, &ddl)
        .expect("generate guarded SQL");
    assert!(
        guarded.starts_with("DO $pgxray_0$"),
        "the name contains $pgxray$"
    );

    client
        .batch_execute(&guarded)
        .await
        .expect("guarded ALTER should succeed");
    let after = read_policy_live_for_test(&client, oid as u32).await;
    assert_eq!(after.effective_freeze_table_age, Some(candidate_value));
    assert_eq!(after.freeze_table_source.as_deref(), Some("table"));

    // PostgreSQL silently clamps oversized aggressive thresholds to 95% of
    // effective freeze max, even though the reloption itself was accepted.
    client
        .batch_execute(&format!(
            "ALTER TABLE {qualified} SET (autovacuum_freeze_table_age = 2000000000)"
        ))
        .await
        .expect("set deliberately oversized table age");
    let capped = read_policy_live_for_test(&client, oid as u32).await;
    assert_eq!(
        capped.effective_freeze_table_age,
        Some(capped.effective_freeze_max_age * 95 / 100),
    );

    let repeated = client
        .batch_execute(&guarded)
        .await
        .expect_err("stale guard must reject retry");
    assert!(repeated.to_string().contains("freeze policy changed"));
    // The transaction is now aborted due to the expected guard exception.
    client
        .batch_execute("ROLLBACK")
        .await
        .expect("roll back all test DDL");
}

#[test]
fn saved_xid_snapshot_can_be_opened_but_not_from_other_connection() {
    use crate::{connections::SavedConnection, history::DiagnosticStore};

    let directory = tempfile::tempdir().expect("temp history directory");
    let mut store =
        DiagnosticStore::open(&directory.path().join("xid-read-test.db")).expect("init history");
    let connection = SavedConnection {
        id: 1047,
        name: "test XID".into(),
        host: "localhost".into(),
        port: 5432,
        database: "postgres".into(),
        username: "postgres".into(),
        sslmode: "disable".into(),
        has_password: false,
        project_id: None,
        shortcut_slot: None,
        last_probe: None,
    };
    let snapshot = XidSnapshot {
        overview: test_overview(),
        databases: section(vec![]),
        relations: section(vec![test_relation(105, 200_000_000)]),
        autovacuum_triggers: section(vec![]),
        vacuums: section(vec![]),
        backends: section(vec![]),
        slots: section(vec![]),
        prepared: section(XidPreparedSummary {
            count: 0,
            oldest_age_seconds: None,
            examples: vec![],
        }),
        interpretation: XidInterpretation {
            items: vec![],
            limitations: vec![],
        },
    };
    let stored = store
        .save_xid_snapshot(&connection, &snapshot, 1000, 2000)
        .unwrap();
    let opened = store.xid_snapshot(connection.id, stored.id).unwrap();
    assert_eq!(opened.history.id, stored.id);
    assert_eq!(
        opened.snapshot.relations.data.unwrap()[0].relation_oid,
        16384
    );
    assert!(store.xid_snapshot(connection.id + 1, stored.id).is_err());
}
