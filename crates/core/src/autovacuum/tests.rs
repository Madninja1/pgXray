use super::{
    advisor::advise_maintenance,
    interpretation::{interpret_snapshot, interpret_table},
    model::{
        AutovacuumAnalyzeProgress, AutovacuumInterpretationItem, AutovacuumInterpretationSeverity,
        AutovacuumMaintenanceHistorySample, AutovacuumMaintenancePolicySignature,
        AutovacuumOverview, AutovacuumPolicyApplyObservedState, AutovacuumSection, AutovacuumTable,
        AutovacuumToast, AutovacuumVacuumProgress,
    },
    policy::{
        build_policy_candidates, fixed_scale_candidates, guarded_policy_apply_sql,
        guarded_policy_rollback_sql, policy_projection, project_trigger,
        validate_policy_apply_guard,
    },
};

#[test]
fn policy_pg18_projections_respect_cap_and_unfrozen_fraction() {
    let mut table = test_table();
    table.server_version_num = 180_000;
    table.effective_vacuum_max_threshold = Some(150);
    table.insert_unfrozen_fraction = Some(0.25);
    table.reltuples_estimate = Some(1_000.0);

    let dead = policy_projection(&table, "autovacuum_vacuum_scale_factor", false).unwrap();
    assert_eq!(project_trigger(50.0, 0.2, dead), Some(151));
    assert_eq!(project_trigger(50.0, 0.01, dead), Some(61));

    let insert = policy_projection(&table, "autovacuum_vacuum_insert_scale_factor", false).unwrap();
    assert_eq!(project_trigger(1_000.0, 0.2, insert), Some(1_051));

    table.insert_unfrozen_fraction = None;
    assert!(policy_projection(&table, "autovacuum_vacuum_insert_scale_factor", false).is_none());
    table.reltuples_estimate = None;
    assert!(policy_projection(&table, "autovacuum_vacuum_scale_factor", false).is_none());
}

#[test]
fn policy_disabled_track_counts_never_returns_concrete_candidates() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    let samples = vec![
        sample(1, 0, 200, true, false, false, 0, 0),
        sample(2, 1_800_000, 300, true, false, false, 0, 0),
        sample(3, 3_600_000, 400, true, false, false, 0, 0),
    ];
    let advice = advise_maintenance(&table, false, None, None, None, &samples, Some("1min"));
    assert!(advice
        .findings
        .iter()
        .any(|finding| finding.id == "track-counts-disabled"));
    assert!(!advice
        .findings
        .iter()
        .any(|finding| finding.status == "candidate"));
    let candidates = build_policy_candidates(&table, &samples, &advice, false, 160_000);
    assert_eq!(candidates.status, "insufficient_evidence");
    assert!(candidates.candidates.is_empty());
}

#[test]
fn guarded_rollback_restores_global_source_with_reset_and_local_with_set() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    let samples = vec![
        sample(1, 0, 200, true, false, false, 0, 0),
        sample(2, 1_800_000, 300, true, false, false, 0, 0),
        sample(3, 3_600_000, 100, true, false, false, 1, 0),
        sample(4, 5_400_000, 200, true, false, false, 1, 0),
    ];
    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let candidate = build_policy_candidates(&table, &samples, &advice, true, 160_000)
        .candidates
        .into_iter()
        .next()
        .unwrap();

    let make_state =
        |scale_factor: f64, source: &str, trigger: i64| AutovacuumPolicyApplyObservedState {
            relation_oid: table.relation_oid,
            subject_relation: "public.events".into(),
            parameter: candidate.parameter.clone(),
            scale_factor,
            source: source.into(),
            fixed_threshold: candidate.fixed_threshold,
            absolute_trigger: trigger,
            estimated_rows: table.estimated_rows,
        };
    let before = make_state(
        candidate.current_scale_factor,
        "global",
        candidate.current_trigger,
    );
    let after = make_state(
        candidate.candidate_scale_factor,
        "table",
        candidate.candidate_trigger,
    );
    let reset = guarded_policy_rollback_sql(12_345, &table, &candidate, &before, &after).unwrap();
    assert!(reset.contains("RESET (autovacuum_vacuum_scale_factor)"));
    assert!(reset.contains("LOCK TABLE ONLY"));
    assert!(reset.contains("track_counts disabled"));
    assert!(reset.contains("pgXray guard failed: autovacuum policy changed"));

    let old_local = make_state(
        candidate.current_scale_factor,
        "table",
        candidate.current_trigger,
    );
    let set = guarded_policy_rollback_sql(12_345, &table, &candidate, &old_local, &after).unwrap();
    assert!(set.contains("SET (autovacuum_vacuum_scale_factor = 0.2)"));
    assert!(!set.contains("RESET (autovacuum_vacuum_scale_factor)"));
}

// Interpretation must distinguish unavailable evidence from an observed zero.
#[test]
fn interpretation_reports_failed_vacuum_and_analyze_sections() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    let interpretation = interpret_snapshot(
        &test_overview(true),
        &AutovacuumSection::success(vec![table]),
        &AutovacuumSection::<Vec<AutovacuumVacuumProgress>>::failure("VACUUM timeout"),
        &AutovacuumSection::<Vec<AutovacuumAnalyzeProgress>>::failure("ANALYZE permission denied"),
    );

    assert!(interpretation.items.iter().any(|item| {
        item.id == "thresholds-reached"
            && item.severity == AutovacuumInterpretationSeverity::Attention
    }));
    assert!(interpretation
        .limitations
        .iter()
        .any(|message| message.contains("VACUUM") && message.contains("timeout")));
    assert!(interpretation
        .limitations
        .iter()
        .any(|message| message.contains("ANALYZE") && message.contains("permission denied")));
}

#[test]
fn interpretation_omits_counter_based_thresholds_when_track_counts_off() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    table.insert_trigger_reached = true;
    table.analyze_trigger_reached = true;
    table.autovacuum_enabled = false;
    let interpretation = interpret_snapshot(
        &test_overview(false),
        &AutovacuumSection::success(vec![table]),
        &AutovacuumSection::success(Vec::<AutovacuumVacuumProgress>::new()),
        &AutovacuumSection::success(Vec::<AutovacuumAnalyzeProgress>::new()),
    );

    assert!(!interpretation
        .items
        .iter()
        .any(|item| item.id == "thresholds-reached"));
    assert!(interpretation.items.iter().any(|item| {
        item.id == "track-counts-off" && item.severity == AutovacuumInterpretationSeverity::Warning
    }));
    // Configuration-based evidence is still valid with track_counts disabled.
    assert!(interpretation
        .items
        .iter()
        .any(|item| item.id == "tables-disabled"));
    assert!(interpretation
        .limitations
        .iter()
        .any(|message| message.contains("пропущены")));
}

#[test]
fn table_interpretation_never_calls_missing_hot_percentage_zero() {
    let mut table = test_table();
    table.hot_update_pct = None;
    let indexes = AutovacuumSection::success(Vec::new());
    let interpretation = interpret_table(&table, &indexes, Some(true));
    assert!(!interpretation.items.iter().any(|item| item.id == "low-hot"));

    table.hot_update_pct = Some(1.0);
    let interpretation = interpret_table(&table, &indexes, Some(true));
    assert!(interpretation.items.iter().any(|item| {
        item.id == "low-hot" && item.severity == AutovacuumInterpretationSeverity::Attention
    }));
}

#[test]
fn table_interpretation_requires_trustworthy_track_counts_for_counter_findings() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    table.insert_trigger_reached = true;
    table.analyze_trigger_reached = true;
    table.toast = Some(test_toast());
    table.workload_profile = "append_only_observed".into();
    table.dead_tuples = 0;
    let indexes = AutovacuumSection::success(Vec::new());

    for track_counts in [Some(false), None] {
        let interpretation = interpret_table(&table, &indexes, track_counts);
        for id in [
            "dead-trigger-reached",
            "insert-trigger-reached",
            "analyze-trigger-reached",
            "low-hot",
            "toast-trigger-reached",
            "append-only",
        ] {
            assert!(
                !interpretation.items.iter().any(|item| item.id == id),
                "untrusted statistics produced {id}"
            );
        }
        assert!(interpretation
            .limitations
            .iter()
            .any(|message| message.contains("track_counts")));
    }
}

#[test]
fn table_interpretation_is_version_neutral_for_partitioned_parent() {
    let mut table = test_table();
    table.relation_type = "partitioned_table".into();
    table.autovacuum_applicable = false;
    let interpretation =
        interpret_table(&table, &AutovacuumSection::success(Vec::new()), Some(true));
    let item = interpretation
        .items
        .iter()
        .find(|item| item.id == "partitioned-parent")
        .expect("partitioned parent explanation");
    assert_eq!(item.severity, AutovacuumInterpretationSeverity::Info);
    assert!(!item.meaning.join(" ").contains("PostgreSQL 14"));
}

#[test]
fn interpretation_severity_is_compatible_with_saved_snapshots() {
    // Snapshots stored before this change have no severity property.
    let older = r#"{"id":"active-autoanalyze","title":"autoanalyze","observed":[],"meaning":[],"next_steps":[]}"#;
    let decoded: AutovacuumInterpretationItem = serde_json::from_str(older).unwrap();
    assert_eq!(decoded.severity, AutovacuumInterpretationSeverity::Info);
    let encoded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(encoded["severity"], "info");
    let attention: AutovacuumInterpretationItem = serde_json::from_value(serde_json::json!({
        "id": "analyze-trigger-reached",
        "title": "ANALYZE",
        "severity": "attention",
        "observed": [],
        "meaning": [],
        "next_steps": [],
    }))
    .unwrap();
    assert_eq!(
        attention.severity,
        AutovacuumInterpretationSeverity::Attention
    );
}

fn test_overview(track_counts: bool) -> AutovacuumOverview {
    AutovacuumOverview {
        measured_at: "2026-11-13T00:00:00Z".into(),
        database: "postgres".into(),
        database_oid: 1,
        username: "test".into(),
        server_version: "16.15".into(),
        server_version_num: 160015,
        stats_reset: Some("2026-11-01T00:00:00Z".into()),
        autovacuum: true,
        track_counts,
        autovacuum_max_workers: 3,
        autovacuum_naptime: "1min".into(),
        autovacuum_vacuum_threshold: 50,
        autovacuum_vacuum_scale_factor: 0.2,
        autovacuum_vacuum_max_threshold: None,
        autovacuum_vacuum_insert_threshold: 1_000,
        autovacuum_vacuum_insert_scale_factor: 0.2,
        autovacuum_analyze_threshold: 50,
        autovacuum_analyze_scale_factor: 0.1,
        autovacuum_vacuum_cost_limit: 200,
        autovacuum_vacuum_cost_delay: "2ms".into(),
        vacuum_cost_limit: 200,
        block_size: 8192,
        log_autovacuum_min_duration: "10min".into(),
    }
}

#[test]
fn fixed_scale_candidates_translate_percent_to_absolute_trigger() {
    let mut table = test_table();
    table.estimated_rows = 10_000_000;
    table.reltuples_estimate = Some(10_000_000.0);
    let candidates = fixed_scale_candidates(&table);

    assert_eq!(candidates[2].resulting_dead_tuple_trigger, 500_051,);
}

#[test]
fn maintenance_advisor_requires_history_before_cadence_conclusion() {
    let table = test_table();
    let samples = vec![sample(1, 0, 100, false, false, false, 0, 0)];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));

    assert_eq!(advice.status, "insufficient_history");
    assert!(advice
        .findings
        .iter()
        .any(|finding| finding.id == "history-insufficient"));
}

#[test]
fn maintenance_advisor_missing_history_does_not_claim_threshold_reached() {
    let table = test_table();
    assert!(!table.dead_trigger_reached);
    assert!(!table.analyze_trigger_reached);
    let samples = vec![sample(1, 0, 100, false, false, false, 0, 0)];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let finding = advice
        .findings
        .iter()
        .find(|finding| finding.id == "history-insufficient")
        .expect("missing-history finding");

    assert!(!finding.summary.contains("показывает достижение порога"));
    assert!(finding
        .summary
        .contains("независимо от того, достигнуты ли пороги"));
    assert!(finding
        .observed
        .iter()
        .any(|value| value.contains("Сопоставимых снимков: 1")));
}

#[test]
fn maintenance_advisor_investigates_repeated_dead_trigger_without_vacuum_cycle() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    table.dead_trigger_used_pct = Some(160.0);
    let samples = vec![
        sample(1, 0, 260, true, false, false, 0, 0),
        sample(2, 1_800_000, 330, true, false, false, 0, 0),
        sample(3, 3_600_000, 400, true, false, false, 0, 0),
    ];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let finding = advice
        .findings
        .iter()
        .find(|finding| finding.id == "dead-trigger-no-cycle")
        .unwrap();

    assert_eq!(advice.status, "investigate");
    assert_eq!(finding.status, "investigate");
    assert!(finding.policy_parameters.is_empty());
}

#[test]
fn maintenance_advisor_marks_repeated_dead_pressure_with_cycles_as_policy_candidate() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    table.dead_trigger_used_pct = Some(160.0);
    let samples = vec![
        sample(1, 0, 260, true, false, false, 0, 0),
        sample(2, 1_800_000, 330, true, false, false, 1, 0),
        sample(3, 3_600_000, 400, true, false, false, 2, 0),
    ];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let finding = advice
        .findings
        .iter()
        .find(|finding| finding.id == "dead-trigger-repeated-with-cycles")
        .unwrap();

    assert_eq!(advice.status, "candidate");
    assert!(finding
        .policy_parameters
        .iter()
        .any(|parameter| parameter == "autovacuum_vacuum_scale_factor"));
}

#[test]
fn policy_candidates_translate_history_pressure_into_read_only_scale_scenarios() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    table.dead_trigger_used_pct = Some(160.0);
    let samples = vec![
        sample(1, 0, 200, true, false, false, 0, 0),
        sample(2, 1_800_000, 300, true, false, false, 0, 0),
        sample(3, 3_600_000, 100, true, false, false, 1, 0),
        sample(4, 5_400_000, 200, true, false, false, 1, 0),
    ];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let result = build_policy_candidates(&table, &samples, &advice, true, 160_000);

    assert_eq!(result.status, "available");
    assert_eq!(result.candidates.len(), 2);
    assert!(result
        .candidates
        .iter()
        .all(|candidate| candidate.parameter == "autovacuum_vacuum_scale_factor"));
    assert!(result
        .candidates
        .iter()
        .all(|candidate| candidate.candidate_trigger < candidate.current_trigger));
    assert!(result.candidates[0]
        .sql_preview
        .contains("ALTER TABLE \"public\".\"events\""));
}

#[test]
fn policy_candidates_reject_null_stats_reset_even_when_counters_are_monotonic() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    table.dead_trigger_used_pct = Some(160.0);
    let mut samples = vec![
        sample(1, 0, 200, true, false, false, 0, 0),
        sample(2, 1_800_000, 300, true, false, false, 0, 0),
        sample(3, 3_600_000, 100, true, false, false, 1, 0),
        sample(4, 5_400_000, 200, true, false, false, 1, 0),
    ];
    for sample in &mut samples {
        sample.stats_reset = None;
    }

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let result = build_policy_candidates(&table, &samples, &advice, true, 160_000);

    assert!(!advice.stats_period_continuous);
    assert_eq!(advice.history_points, 1);
    assert!(advice.findings.iter().any(|finding| {
        finding.id == "snapshot-only-signals"
            && finding
                .observed
                .iter()
                .any(|line| line.contains("dead-tuple trigger"))
            && finding.policy_parameters.is_empty()
    }));
    assert!(advice
        .limitations
        .iter()
        .any(|line| line.contains("stats_reset = NULL")));
    assert_eq!(result.status, "insufficient_evidence");
    assert!(result.candidates.is_empty());
    assert!(result
        .limitations
        .iter()
        .any(|limitation| limitation.contains("stats period")));
}

#[test]
fn policy_candidates_remember_effective_source_for_guarded_apply() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    table.dead_trigger_used_pct = Some(160.0);
    let samples = vec![
        sample(1, 0, 200, true, false, false, 0, 0),
        sample(2, 1_800_000, 300, true, false, false, 0, 0),
        sample(3, 3_600_000, 100, true, false, false, 1, 0),
        sample(4, 5_400_000, 200, true, false, false, 1, 0),
    ];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let result = build_policy_candidates(&table, &samples, &advice, true, 160_000);
    assert_eq!(result.candidates[0].current_source, "global");

    table.reloptions = Some(vec!["autovacuum_vacuum_scale_factor=0.2".into()]);
    let result = build_policy_candidates(&table, &samples, &advice, true, 160_000);
    assert_eq!(result.candidates[0].current_source, "table");
}

#[test]
fn guarded_apply_rejects_live_policy_drift_and_checks_maintenance_inside_lock() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    table.dead_trigger_used_pct = Some(160.0);
    let samples = vec![
        sample(1, 0, 200, true, false, false, 0, 0),
        sample(2, 1_800_000, 300, true, false, false, 0, 0),
        sample(3, 3_600_000, 100, true, false, false, 1, 0),
        sample(4, 5_400_000, 200, true, false, false, 1, 0),
    ];
    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let candidates = build_policy_candidates(&table, &samples, &advice, true, 160_000);
    let candidate = &candidates.candidates[0];

    let sql = guarded_policy_apply_sql(12_345, &table, candidate).unwrap();
    assert!(sql.contains("LOCK TABLE ONLY \"public\".\"events\" IN SHARE UPDATE EXCLUSIVE MODE"));
    assert!(sql.contains("pg_stat_progress_vacuum"));
    assert!(sql.contains("pg_stat_progress_analyze"));
    assert!(sql.contains("database identity changed"));
    assert!(sql.contains(&candidate.sql_preview));

    let mut live = table.clone();
    live.effective_vacuum_scale_factor = 0.15;
    live.dead_tuple_trigger = 200;
    let error = validate_policy_apply_guard(&table, &live, candidate).unwrap_err();
    assert!(error.contains("policy изменилась"));
}

#[test]
fn guarded_insert_policy_sql_avoids_pg18_only_catalog_columns() {
    let mut table = test_table();
    table.dead_tuples = 400;
    table.dead_trigger_reached = true;
    table.dead_trigger_used_pct = Some(160.0);
    let samples = vec![
        sample(1, 0, 200, true, false, false, 0, 0),
        sample(2, 1_800_000, 300, true, false, false, 0, 0),
        sample(3, 3_600_000, 100, true, false, false, 1, 0),
        sample(4, 5_400_000, 200, true, false, false, 1, 0),
    ];
    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let mut candidate = build_policy_candidates(&table, &samples, &advice, true, 160_000)
        .candidates
        .into_iter()
        .next()
        .expect("vacuum candidate for SQL construction");

    // Exercise only the insert-specific guard expression. This is not an
    // executable policy recommendation; no statement is sent to PostgreSQL.
    candidate.parameter = "autovacuum_vacuum_insert_scale_factor".into();
    candidate.current_scale_factor = table.effective_insert_scale_factor;
    candidate.fixed_threshold = table.effective_insert_threshold;
    candidate.current_trigger = table.insert_trigger.unwrap();
    let sql = guarded_policy_apply_sql(12_345, &table, &candidate).unwrap();
    assert!(sql.contains("to_jsonb(c)->>'relallfrozen'"));
    assert!(!sql.contains("c.relallfrozen"));
}

#[test]
fn guarded_apply_uses_parent_table_for_toast_candidate() {
    let mut table = test_table();
    table.toast = Some(test_toast());

    let mut first = sample(1, 0, 100, false, false, false, 0, 0);
    first.toast_autovacuum_count = Some(0);
    first.toast_dead_tuples = Some(200);
    first.toast_dead_trigger_reached = Some(true);
    let mut second = sample(2, 1_800_000, 100, false, false, false, 0, 0);
    second.toast_autovacuum_count = Some(0);
    second.toast_dead_tuples = Some(250);
    second.toast_dead_trigger_reached = Some(true);
    let mut third = sample(3, 3_600_000, 100, false, false, false, 0, 0);
    third.toast_autovacuum_count = Some(1);
    third.toast_dead_tuples = Some(100);
    third.toast_dead_trigger_reached = Some(true);
    let mut fourth = sample(4, 5_400_000, 100, false, false, false, 0, 0);
    fourth.toast_autovacuum_count = Some(1);
    fourth.toast_dead_tuples = Some(150);
    fourth.toast_dead_trigger_reached = Some(true);
    let samples = vec![first, second, third, fourth];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let candidates = build_policy_candidates(&table, &samples, &advice, true, 160_000);
    let candidate = candidates
        .candidates
        .iter()
        .find(|candidate| candidate.area == "toast_vacuum")
        .unwrap();

    let sql = guarded_policy_apply_sql(12_345, &table, candidate).unwrap();
    assert!(sql.contains("LOCK TABLE ONLY \"public\".\"events\""));
    assert!(sql.contains("reltoastrelid"));
    assert!(sql.contains("toast.autovacuum_vacuum_scale_factor"));
    assert!(sql.contains("16385::oid"));
}

#[test]
fn maintenance_advisor_keeps_analyze_separate_from_vacuum() {
    let mut table = test_table();
    table.modifications_since_analyze = 500;
    table.analyze_trigger_reached = true;
    table.analyze_trigger_used_pct = Some(200.0);
    let samples = vec![
        sample(1, 0, 100, false, false, true, 0, 0),
        sample(2, 1_800_000, 100, false, false, true, 0, 0),
        sample(3, 3_600_000, 100, false, false, true, 0, 0),
    ];

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let finding = advice
        .findings
        .iter()
        .find(|finding| finding.id == "analyze-trigger-no-cycle")
        .unwrap();

    assert_eq!(finding.area, "analyze");
    assert!(finding.policy_parameters.is_empty());
    assert_eq!(advice.status, "investigate");
}

#[test]
fn maintenance_advisor_treats_toast_as_independent_policy_context() {
    let mut table = test_table();
    table.toast = Some(test_toast());
    let mut first = sample(1, 0, 100, false, false, false, 0, 0);
    first.toast_autovacuum_count = Some(0);
    first.toast_dead_trigger_reached = Some(true);
    let mut second = sample(2, 1_800_000, 100, false, false, false, 0, 0);
    second.toast_autovacuum_count = Some(1);
    second.toast_dead_trigger_reached = Some(true);
    let mut third = sample(3, 3_600_000, 100, false, false, false, 0, 0);
    third.toast_autovacuum_count = Some(2);
    third.toast_dead_trigger_reached = Some(true);

    let advice = advise_maintenance(
        &table,
        true,
        None,
        None,
        None,
        &[first, second, third],
        Some("1min"),
    );
    let finding = advice
        .findings
        .iter()
        .find(|finding| finding.id == "toast-trigger-repeated-with-cycles")
        .unwrap();

    assert_eq!(finding.status, "candidate");
    assert!(finding
        .policy_parameters
        .iter()
        .any(|parameter| parameter == "toast.autovacuum_vacuum_scale_factor"));
}

#[test]
fn maintenance_history_refuses_rates_when_stats_reset_is_unknown() {
    let table = test_table();
    let mut samples = vec![
        sample(1, 0, 100, false, false, false, 0, 0),
        sample(2, 1_800_000, 120, false, false, false, 0, 0),
        sample(3, 3_600_000, 140, false, false, false, 0, 0),
    ];
    for sample in &mut samples {
        sample.stats_reset = None;
    }

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));

    assert!(!advice.stats_period_continuous);
    assert_eq!(advice.history_points, 1);
    assert!(advice.updates_per_hour.is_none());
}

#[test]
fn maintenance_history_uses_latest_comparable_segment_after_reset_boundary() {
    let table = test_table();
    let mut old_first = sample(1, 0, 100, false, false, false, 0, 0);
    let mut old_second = sample(2, 1_800_000, 120, false, false, false, 0, 0);
    old_first.stats_reset = Some("old".into());
    old_second.stats_reset = Some("old".into());

    let mut new_first = sample(3, 3_600_000, 140, false, false, false, 0, 0);
    let mut new_second = sample(4, 5_400_000, 160, false, false, false, 0, 0);
    let mut new_third = sample(5, 7_200_000, 180, false, false, false, 0, 0);
    new_first.stats_reset = Some("new".into());
    new_second.stats_reset = Some("new".into());
    new_third.stats_reset = Some("new".into());

    let advice = advise_maintenance(
        &table,
        true,
        None,
        None,
        None,
        &[old_first, old_second, new_first, new_second, new_third],
        Some("1min"),
    );

    assert!(advice.stats_period_continuous);
    assert_eq!(advice.history_points, 3);
    assert_eq!(advice.history_span_seconds, Some(3_600.0));
    assert!(advice
        .limitations
        .iter()
        .any(|value| value.contains("Снимки до последнего изменения счётчиков или настроек")));
}

#[test]
fn maintenance_policy_change_invalidates_history_even_with_monotonic_counters() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    let mut samples = vec![
        sample(1, 0, 260, true, false, false, 0, 0),
        sample(2, 1_800_000, 330, true, false, false, 1, 0),
        sample(3, 3_600_000, 400, true, false, false, 2, 0),
    ];
    // A DBA changed the effective factor between the second and third
    // observation, without resetting the cumulative PostgreSQL counters.
    samples[2].policy.as_mut().unwrap().vacuum_scale_factor = 0.05;

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    assert_eq!(advice.history_points, 1);
    assert!(!advice.stats_period_continuous);
    assert!(advice.updates_per_hour.is_none());
    assert!(!advice
        .findings
        .iter()
        .any(|finding| finding.status == "candidate"));
    let candidates = build_policy_candidates(&table, &samples, &advice, true, 160_000);
    assert!(candidates.candidates.is_empty());
}

#[test]
fn maintenance_unknown_policy_does_not_produce_rates() {
    let table = test_table();
    let mut samples = vec![
        sample(1, 0, 100, false, false, false, 0, 0),
        sample(2, 1_800_000, 110, false, false, false, 0, 0),
        sample(3, 3_600_000, 120, false, false, false, 0, 0),
    ];
    samples[1].policy = None;
    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    assert_eq!(advice.history_points, 1);
    assert!(advice.updates_per_hour.is_none());
}

#[test]
fn maintenance_large_gap_is_not_sustained_eligibility() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    let samples = vec![
        sample(1, 0, 260, true, false, false, 0, 0),
        sample(2, 1_800_000, 330, true, false, false, 0, 0),
        sample(3, 14_400_000, 400, true, false, false, 0, 0),
    ];
    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    assert!(!advice
        .findings
        .iter()
        .any(|finding| finding.id == "dead-trigger-no-cycle"));
    assert!(advice
        .findings
        .iter()
        .any(|finding| finding.id == "dead-trigger-current"));
}

#[test]
fn maintenance_naptime_changes_required_observation_window() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    let samples = vec![
        sample(1, 0, 260, true, false, false, 0, 0),
        sample(2, 1_800_000, 330, true, false, false, 0, 0),
        sample(3, 3_600_000, 400, true, false, false, 0, 0),
    ];
    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("40min"));
    assert!(!advice
        .findings
        .iter()
        .any(|finding| finding.id == "dead-trigger-no-cycle"));
    let missing = advise_maintenance(&table, true, None, None, None, &samples, None);
    assert!(!missing
        .findings
        .iter()
        .any(|finding| finding.id == "dead-trigger-no-cycle"));
}

#[test]
fn toast_missing_cycles_does_not_mean_zero_cycles() {
    let mut table = test_table();
    table.toast = Some(test_toast());
    let mut samples = vec![
        sample(1, 0, 100, false, false, false, 0, 0),
        sample(2, 1_800_000, 100, false, false, false, 0, 0),
        sample(3, 3_600_000, 100, false, false, false, 0, 0),
    ];
    for sample in &mut samples {
        sample.toast_dead_trigger_reached = Some(true);
        sample.toast_autovacuum_count = None;
    }
    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    assert!(advice
        .findings
        .iter()
        .any(|finding| finding.id == "toast-cycles-unavailable"));
    assert!(!advice
        .findings
        .iter()
        .any(|finding| finding.id == "toast-trigger-no-cycle"));
}

#[test]
fn effective_policy_source_change_breaks_history_even_with_same_factor() {
    let mut samples = [
        sample(1, 0, 100, false, false, false, 0, 0),
        sample(2, 1_800_000, 120, false, false, false, 0, 0),
    ];
    samples[1].policy.as_mut().unwrap().reloptions =
        Some(vec!["autovacuum_vacuum_scale_factor=0.2".into()]);
    assert!(!super::model::maintenance_stats_pair_continuous(
        &samples[0],
        &samples[1]
    ));
}

#[test]
fn manual_vacuum_and_analyze_are_not_reported_as_absent_maintenance() {
    let mut table = test_table();
    table.dead_trigger_reached = true;
    table.analyze_trigger_reached = true;
    let mut samples = vec![
        sample(1, 0, 260, true, false, true, 0, 0),
        sample(2, 1_800_000, 330, true, false, true, 0, 0),
        sample(3, 3_600_000, 400, true, false, true, 0, 0),
    ];
    samples[2].vacuum_count = 1;
    samples[2].analyze_count = 1;

    let advice = advise_maintenance(&table, true, None, None, None, &samples, Some("1min"));
    let vacuum = advice
        .findings
        .iter()
        .find(|finding| finding.id == "dead-trigger-no-cycle")
        .expect("an investigation finding for automatic VACUUM");
    assert!(vacuum
        .observed
        .iter()
        .any(|text| text.contains("ручных VACUUM=1")));
    let analyze = advice
        .findings
        .iter()
        .find(|finding| finding.id == "analyze-trigger-no-cycle")
        .expect("an investigation finding for autoanalyze");
    assert!(analyze
        .observed
        .iter()
        .any(|text| text.contains("ручных ANALYZE=1")));
}

#[test]
fn maintenance_history_detects_counter_reset_even_when_stats_reset_is_null() {
    let mut first = sample(1, 0, 100, false, false, false, 0, 0);
    let mut second = sample(2, 1_800_000, 120, false, false, false, 0, 0);
    let mut third = sample(3, 3_600_000, 140, false, false, false, 0, 0);
    first.stats_reset = None;
    second.stats_reset = None;
    third.stats_reset = None;
    first.tuples_updated = 10_000;
    second.tuples_updated = 20;
    third.tuples_updated = 40;

    let samples = super::model::latest_continuous_maintenance_samples(&[first, second, third]);

    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].tuples_updated, 40);
}

#[expect(
    clippy::too_many_arguments,
    reason = "test fixture keeps parameters explicit"
)]
fn sample(
    snapshot_id: i64,
    collection_started_at_ms: i64,
    dead_tuples: i64,
    dead_trigger_reached: bool,
    insert_trigger_reached: bool,
    analyze_trigger_reached: bool,
    vacuum_cycles: i64,
    analyze_cycles: i64,
) -> AutovacuumMaintenanceHistorySample {
    AutovacuumMaintenanceHistorySample {
        collection_started_at_ms,
        stats_reset: Some("2026-09-16T00:00:00+00:00".into()),
        policy: Some(AutovacuumMaintenancePolicySignature::from_table(
            &test_table(),
        )),
        dead_tuples,
        inserts_since_vacuum: snapshot_id * 500,
        modifications_since_analyze: snapshot_id * 800,
        tuples_inserted: snapshot_id * 1_000,
        tuples_updated: snapshot_id * 2_000,
        tuples_deleted: snapshot_id * 100,
        hot_updates: snapshot_id * 20,
        vacuum_count: 0,
        autovacuum_count: vacuum_cycles,
        analyze_count: 0,
        autoanalyze_count: analyze_cycles,
        dead_trigger_reached,
        insert_trigger_reached,
        analyze_trigger_reached,
        toast_autovacuum_count: None,
        toast_dead_tuples: None,
        toast_inserts_since_vacuum: None,
        toast_dead_trigger_reached: None,
        toast_insert_trigger_reached: None,
    }
}

fn test_table() -> AutovacuumTable {
    AutovacuumTable {
        relation_oid: 16_384,
        schema_name: "public".into(),
        relation_name: "events".into(),
        relation_type: "table".into(),
        parent_oid: None,
        parent_schema: None,
        parent_relation: None,
        heap_size_bytes: 1_024,
        indexes_size_bytes: 512,
        total_size_bytes: 1_536,
        index_count: 2,
        estimated_rows: 1_000,
        live_tuples: 1_000,
        dead_tuples: 100,
        dead_tuple_pct: Some(9.1),
        tuples_inserted: 3_000,
        tuples_updated: 6_000,
        tuples_deleted: 300,
        hot_updates: 60,
        hot_update_pct: Some(1.0),
        inserts_since_vacuum: 300,
        modifications_since_analyze: 300,
        last_vacuum: None,
        last_autovacuum: None,
        last_analyze: None,
        last_autoanalyze: None,
        vacuum_count: 0,
        autovacuum_count: 0,
        analyze_count: 0,
        autoanalyze_count: 0,
        xid_age: Some(100),
        reloptions: None,
        toast: None,
        autovacuum_enabled: true,
        autovacuum_applicable: true,
        effective_vacuum_threshold: 50.0,
        effective_vacuum_scale_factor: 0.2,
        effective_vacuum_max_threshold: None,
        server_version_num: 160_000,
        reltuples_estimate: Some(1_000.0),
        insert_unfrozen_fraction: None,
        effective_insert_threshold: 1_000.0,
        effective_insert_scale_factor: 0.2,
        effective_analyze_threshold: 50.0,
        effective_analyze_scale_factor: 0.1,
        effective_vacuum_cost_limit: 200,
        effective_vacuum_cost_delay_ms: 2.0,
        effective_freeze_max_age: 200_000_000,
        dead_tuple_trigger: 250,
        insert_trigger: Some(1_200),
        analyze_trigger: 150,
        dead_trigger_used_pct: Some(40.0),
        insert_trigger_used_pct: Some(25.0),
        analyze_trigger_used_pct: Some(50.0),
        dead_trigger_reached: false,
        insert_trigger_reached: false,
        analyze_trigger_reached: false,
        workload_profile: "update_delete_heavy".into(),
    }
}

fn test_toast() -> AutovacuumToast {
    AutovacuumToast {
        relation_oid: 16_385,
        schema_name: "pg_toast".into(),
        relation_name: "pg_toast_16384".into(),
        total_size_bytes: 512,
        estimated_rows: 500,
        live_tuples: 500,
        dead_tuples: 300,
        inserts_since_vacuum: 100,
        last_autovacuum: None,
        autovacuum_count: 2,
        xid_age: 100,
        reloptions: None,
        autovacuum_enabled: true,
        effective_vacuum_threshold: 50.0,
        effective_vacuum_scale_factor: 0.2,
        effective_vacuum_max_threshold: None,
        server_version_num: 160_000,
        reltuples_estimate: Some(500.0),
        insert_unfrozen_fraction: None,
        effective_insert_threshold: 1_000.0,
        effective_insert_scale_factor: 0.2,
        effective_freeze_max_age: 200_000_000,
        dead_tuple_trigger: 150,
        insert_trigger: Some(1_100),
        dead_trigger_used_pct: Some(200.0),
        insert_trigger_used_pct: Some(9.0),
        dead_trigger_reached: true,
        insert_trigger_reached: false,
    }
}

#[test]
fn collection_sql_is_backward_compatible_and_version_aware() {
    use super::queries::{ANALYZE_PROGRESS_SQL, OVERVIEW_SQL, TABLES_SQL, VACUUM_PROGRESS_SQL};
    assert!(OVERVIEW_SQL.contains("current_setting('autovacuum_vacuum_max_threshold', true)"));
    assert!(TABLES_SQL.contains("effective_vacuum_max_threshold"));
    assert!(TABLES_SQL.contains("server_version_num >= 180000"));
    assert!(TABLES_SQL.contains("to_jsonb(c)->>'relallfrozen'"));
    assert!(TABLES_SQL.contains("to_jsonb(tc)->>'relallfrozen'"));
    assert!(!TABLES_SQL.contains("c.relallfrozen"));
    assert!(!TABLES_SQL.contains("tc.relallfrozen"));
    assert!(TABLES_SQL.contains("floor("));
    assert!(TABLES_SQL.contains("n_mod_since_analyze"));
    assert!(TABLES_SQL.contains("last_autoanalyze"));
    assert!(TABLES_SQL.contains("autoanalyze_count"));
    assert!(VACUUM_PROGRESS_SQL.contains("to_jsonb(p)->>'max_dead_tuples'"));
    assert!(VACUUM_PROGRESS_SQL.contains("to_jsonb(p)->>'dead_tuple_bytes'"));
    assert!(ANALYZE_PROGRESS_SQL.contains("pg_stat_progress_analyze"));
}

#[test]
fn table_detail_sql_filters_before_relation_size_calculations() {
    use super::queries::{table_sql, TABLES_SQL};
    assert!(!TABLES_SQL.contains("AND c.oid = 12345::oid"));
    let sql = table_sql(12_345).expect("table_sql marker must be present exactly once");
    assert!(sql.contains("WHERE c.relkind IN ('r', 'p')"));
    assert!(sql.contains("AND c.oid = 12345::oid"));
    assert!(!sql.contains("/* pgxray:table-oid-filter */"));
    assert_eq!(sql.matches("AND c.oid = 12345::oid").count(), 1);
}

#[test]
fn update_evidence_limits_raw_candidates_for_local_structural_parsing() {
    let sql = super::queries::update_statements_sql(12_345);
    assert!(sql.contains("s.dbid ="));
    assert!(sql.contains("LIMIT 200"));
    assert!(sql.contains("length(s.query) <= 8192"));
    assert!(sql.contains("position(lower(t.relname)"));
    assert!(!sql.contains("ILIKE '%'"));
}

#[test]
fn index_keys_and_include_columns_are_distinct() {
    let sql = super::queries::indexes_sql(12_345);
    assert!(sql.contains("k.ordinality <= i.indnkeyatts"));
    assert!(sql.contains("k.ordinality > i.indnkeyatts"));
}

#[test]
fn section_constructors_preserve_data_or_error_contract() {
    let success = super::model::AutovacuumSection::success(vec![1, 2]);
    assert!(success.is_valid());
    assert_eq!(success.data, Some(vec![1, 2]));
    let failure: super::model::AutovacuumSection<Vec<i32>> =
        super::model::AutovacuumSection::failure("not available");
    assert!(failure.is_valid());
    assert_eq!(failure.error.as_deref(), Some("not available"));
    let malformed = super::model::AutovacuumSection::<i32> {
        data: None,
        error: None,
    };
    assert!(!malformed.is_valid());
}

#[test]
fn historical_vacuum_progress_without_pg17_fields_remains_readable() {
    let value = serde_json::json!({
        "pid": 1234, "relation_oid": 12345,
        "schema_name": "public", "relation_name": "events", "phase": "scanning heap",
        "heap_blks_total": 10, "heap_blks_scanned": 5, "heap_blks_vacuumed": 0,
        "scan_pct": 50.0, "vacuum_pct": 0.0, "index_vacuum_count": 0,
        "max_dead_tuples": 1000, "num_dead_tuples": 10,
        "backend_type": "autovacuum worker", "is_autovacuum": true,
        "query_age_seconds": 30, "wait_event_type": null, "wait_event": null
    });
    let progress: super::model::AutovacuumVacuumProgress = serde_json::from_value(value).unwrap();
    assert_eq!(progress.num_dead_tuples, Some(10));
    assert_eq!(progress.dead_tuple_bytes, None);
}

// Integration test for a deliberate policy SET, post-apply verification and
// guarded RESET. Never run on production. Ignored plus an explicit opt-in,
// and only a dedicated database named pgxray_policy_test is accepted.
#[tokio::test]
#[ignore = "writes a disposable table; requires pgxray_policy_test database and explicit opt-in"]
async fn integration_autovacuum_policy_apply_and_guarded_rollback() {
    use crate::{
        internal_postgres::{self, InternalPostgresRegistry},
        postgres::{ConnectionInput, QueryExecutionPolicy},
    };
    if std::env::var("PGXRAY_TEST_ALLOW_POLICY_WRITE").as_deref() != Ok("YES_DISPOSABLE_DB") {
        panic!("Set PGXRAY_TEST_ALLOW_POLICY_WRITE=YES_DISPOSABLE_DB only for a disposable test database");
    }
    let database = std::env::var("PGXRAY_TEST_DB").unwrap_or_default();
    assert_eq!(
        database, "pgxray_policy_test",
        "refusing to write outside the dedicated disposable database"
    );
    let input = ConnectionInput {
        host: std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST required"),
        port: std::env::var("PGXRAY_TEST_PORT")
            .unwrap_or_else(|_| "5432".into())
            .parse()
            .expect("invalid PGXRAY_TEST_PORT"),
        database,
        username: std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER required"),
        password: std::env::var("PGXRAY_TEST_PASSWORD").unwrap_or_default(),
        sslmode: std::env::var("PGXRAY_TEST_SSLMODE").unwrap_or_else(|_| "disable".into()),
    };
    let registry = InternalPostgresRegistry::new();
    let table_name = format!("pgxray_policy_test_{}", std::process::id());
    let target = format!(r#""public"."{table_name}""#);
    let action_policy =
        || QueryExecutionPolicy::explicit_heavy_query(5_000, "pgxray-policy-write-test");
    internal_postgres::run_action_once(
        &input,
        &format!("CREATE TABLE {target} (id bigint NOT NULL)"),
        action_policy(),
    )
    .await
    .expect("creating disposable test table");

    let exercise: Result<(), String> = async {
        internal_postgres::run_action_once(
            &input,
            &format!("INSERT INTO {target} SELECT generate_series(1, 100)"),
            action_policy(),
        )
        .await?;
        internal_postgres::run_action_once(&input, &format!("ANALYZE {target}"), action_policy())
            .await?;
        let snapshot = super::collector::collect(&registry, 4321, &input).await?;
        let table = snapshot
            .tables
            .data
            .as_ref()
            .ok_or("tables section unavailable in write test")?
            .iter()
            .find(|t| t.relation_name == table_name)
            .ok_or("test relation missing from snapshot")?
            .clone();
        let source = if table.reloptions.as_ref().is_some_and(|opts| {
            opts.iter()
                .any(|opt| opt.starts_with("autovacuum_vacuum_scale_factor="))
        }) {
            "table"
        } else {
            "global"
        };
        let scale = table.effective_vacuum_scale_factor / 2.0;
        if scale <= 0.0 {
            return Err("test requires positive starting autovacuum_vacuum_scale_factor".into());
        }
        let projection = policy_projection(&table, "autovacuum_vacuum_scale_factor", false)
            .ok_or("missing projection data on disposable test relation")?;
        let candidate_trigger =
            project_trigger(table.effective_vacuum_threshold, scale, projection)
                .ok_or("unable to project candidate trigger")?;
        let candidate = super::model::AutovacuumPolicyCandidate {
            id: "write-test".into(),
            area: "vacuum".into(),
            scenario: "test".into(),
            target_relation: format!("public.{table_name}"),
            parameter: "autovacuum_vacuum_scale_factor".into(),
            current_scale_factor: table.effective_vacuum_scale_factor,
            current_source: source.into(),
            candidate_scale_factor: scale,
            fixed_threshold: table.effective_vacuum_threshold,
            current_trigger: table.dead_tuple_trigger,
            candidate_trigger,
            observed_rate_per_hour: 1.0,
            current_volume_hours: 1.0,
            candidate_volume_hours: 1.0,
            sql_preview: super::policy::policy_sql_preview(
                "public",
                &table_name,
                "autovacuum_vacuum_scale_factor",
                scale,
            ),
            rationale: vec!["Только тест".into()],
        };
        let result = super::policy::apply_policy_candidate(
            &registry,
            4321,
            &input,
            snapshot.overview.database_oid,
            &table,
            &candidate,
        )
        .await?;
        assert_eq!(result.after.absolute_trigger, candidate_trigger);
        let rollback = result
            .rollback_sql
            .ok_or("guarded rollback SQL not produced")?;
        internal_postgres::run_action_once(&input, &rollback, action_policy()).await?;
        let restored = super::collector::table_detail(&registry, 4321, &input, table.relation_oid)
            .await?
            .table;
        assert!(
            (restored.effective_vacuum_scale_factor - table.effective_vacuum_scale_factor).abs()
                < 1e-8
        );
        if source == "global" {
            assert!(!restored
                .reloptions
                .unwrap_or_default()
                .iter()
                .any(|opt| opt.starts_with("autovacuum_vacuum_scale_factor=")));
        }
        Ok(())
    }
    .await;
    let cleanup = internal_postgres::run_action_once(
        &input,
        &format!("DROP TABLE IF EXISTS {target}"),
        action_policy(),
    )
    .await;
    assert!(
        cleanup.is_ok(),
        "could not clean up disposable test table: {cleanup:?}"
    );
    assert!(
        exercise.is_ok(),
        "policy write/verify/rollback failed: {exercise:?}"
    );
}

// The concurrent DBA changes the table while guarded Apply waits for its
// table lock. After the other transaction commits, the guard must reject the
// stale candidate instead of overwriting the new policy. Requires a *disposable*
// PostgreSQL database; this test creates and drops a real relation.
#[tokio::test]
#[ignore = "requires disposable pgxray_policy_test and explicit write opt-in"]
async fn integration_autovacuum_guarded_apply_rejects_concurrent_policy_change() {
    use crate::{
        internal_postgres::{self, InternalPostgresRegistry},
        postgres::{ConnectionInput, QueryExecutionPolicy},
    };
    use tokio::time::{sleep, Duration};

    assert_eq!(
        std::env::var("PGXRAY_TEST_ALLOW_POLICY_WRITE").as_deref(),
        Ok("YES_DISPOSABLE_DB"),
        "write test requires PGXRAY_TEST_ALLOW_POLICY_WRITE=YES_DISPOSABLE_DB"
    );
    let db = std::env::var("PGXRAY_TEST_DB").unwrap_or_default();
    assert_eq!(
        db, "pgxray_policy_test",
        "refusing to write outside disposable DB"
    );
    let input = ConnectionInput {
        host: std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST required"),
        port: std::env::var("PGXRAY_TEST_PORT")
            .unwrap_or_else(|_| "5432".into())
            .parse()
            .expect("PGXRAY_TEST_PORT must be numeric"),
        database: db,
        username: std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER required"),
        password: std::env::var("PGXRAY_TEST_PASSWORD").unwrap_or_default(),
        sslmode: std::env::var("PGXRAY_TEST_SSLMODE").unwrap_or_else(|_| "disable".into()),
    };
    assert_eq!(
        input.sslmode, "disable",
        "concurrency test uses a local NoTls client"
    );
    assert!(
        matches!(input.host.as_str(), "localhost" | "127.0.0.1" | "::1"),
        "concurrency test must use a local disposable PostgreSQL instance"
    );
    let registry = InternalPostgresRegistry::new();
    let table_name = format!("pgxray_policy_race_{}", std::process::id());
    let target = format!(r#""public"."{table_name}""#);
    let setup_policy = || QueryExecutionPolicy::explicit_heavy_query(5_000, "pgxray-race-setup");
    internal_postgres::run_action_once(
        &input,
        &format!("CREATE TABLE {target} (id bigint NOT NULL)"),
        setup_policy(),
    )
    .await
    .expect("create disposable race-test table");

    let exercise: Result<(), String> = async {
        internal_postgres::run_action_once(
            &input,
            &format!("INSERT INTO {target} SELECT generate_series(1, 100)"),
            setup_policy(),
        )
        .await?;
        internal_postgres::run_action_once(&input, &format!("ANALYZE {target}"), setup_policy())
            .await?;
        let snapshot = super::collector::collect(&registry, 7123, &input).await?;
        let table = snapshot
            .tables
            .data
            .as_ref()
            .ok_or("table statistics not available for race test")?
            .iter()
            .find(|table| table.relation_name == table_name)
            .ok_or("race-test relation not present in snapshot")?
            .clone();
        let scale = table.effective_vacuum_scale_factor * 0.5;
        let drift = table.effective_vacuum_scale_factor * 0.75;
        let projection = policy_projection(&table, "autovacuum_vacuum_scale_factor", false)
            .ok_or("missing table projection for race test")?;
        let candidate_trigger = project_trigger(table.effective_vacuum_threshold, scale, projection)
            .ok_or("missing candidate trigger")?;
        let candidate = super::model::AutovacuumPolicyCandidate {
            id: "race-test".into(),
            area: "vacuum".into(),
            scenario: "test".into(),
            target_relation: format!("public.{table_name}"),
            parameter: "autovacuum_vacuum_scale_factor".into(),
            current_scale_factor: table.effective_vacuum_scale_factor,
            current_source: "global".into(),
            candidate_scale_factor: scale,
            fixed_threshold: table.effective_vacuum_threshold,
            current_trigger: table.dead_tuple_trigger,
            candidate_trigger,
            observed_rate_per_hour: 1.0,
            current_volume_hours: 1.0,
            candidate_volume_hours: 1.0,
            sql_preview: super::policy::policy_sql_preview(
                "public",
                &table_name,
                "autovacuum_vacuum_scale_factor",
                scale,
            ),
            rationale: vec!["Concurrent DDL test only".into()],
        };
        let guarded_sql =
            guarded_policy_apply_sql(snapshot.overview.database_oid, &table, &candidate)?;

        // A separate connection obtains the DDL lock and changes the live
        // reloptions, but deliberately does not commit immediately.
        let mut config = tokio_postgres::Config::new();
        config.host(&input.host);
        config.port(input.port);
        config.dbname(&input.database);
        config.user(&input.username);
        config.password(&input.password);
        let (mut blocker, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .map_err(|err| err.to_string())?;
        let connection_task = tokio::spawn(async move {
            let _ = connection.await;
        });
        let tx = blocker.transaction().await.map_err(|err| err.to_string())?;
        tx.batch_execute(&format!(
            "ALTER TABLE {target} SET (autovacuum_vacuum_scale_factor = {drift:.12})"
        ))
        .await
        .map_err(|err| err.to_string())?;

        let action_input = ConnectionInput {
            host: input.host.clone(),
            port: input.port,
            database: input.database.clone(),
            username: input.username.clone(),
            password: input.password.clone(),
            sslmode: input.sslmode.clone(),
        };
        let apply = tokio::spawn(async move {
            internal_postgres::run_action_once(
                &action_input,
                &guarded_sql,
                QueryExecutionPolicy::explicit_heavy_query(8_000, "pgxray-policy-race-test"),
            )
            .await
        });
        let mut saw_lock_wait = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(50)).await;
            tx.batch_execute("SELECT pg_stat_clear_snapshot()")
                .await
                .map_err(|err| err.to_string())?;
            let row = tx.query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE application_name = 'pgxray-policy-race-test' AND wait_event_type = 'Lock')",
                &[],
            )
            .await
            .map_err(|err| err.to_string())?;
            saw_lock_wait = row.get::<_, bool>(0);
            if saw_lock_wait || apply.is_finished() {
                break;
            }
        }
        // Release the lock and let the guarded operation observe the drift.
        tx.commit().await.map_err(|err| err.to_string())?;
        drop(blocker);
        let result = apply.await.map_err(|err| err.to_string())?;
        connection_task.abort();
        if !saw_lock_wait {
            return Err(format!(
                "guarded Apply never entered PostgreSQL lock wait: {result:?}"
            ));
        }
        let error = match result {
            Ok(()) => return Err("guarded Apply overwrote concurrent DDL".into()),
            Err(error) => error,
        };
        if !error.contains("autovacuum policy changed") {
            return Err(format!("unexpected guarded Apply error: {error}"));
        }
        let live = super::collector::table_detail(&registry, 7123, &input, table.relation_oid)
            .await?
            .table;
        if (live.effective_vacuum_scale_factor - drift).abs() > 1e-8 {
            return Err("concurrent DBA setting was overwritten".into());
        }
        Ok(())
    }
    .await;
    let cleanup = internal_postgres::run_action_once(
        &input,
        &format!("DROP TABLE IF EXISTS {target}"),
        setup_policy(),
    )
    .await;
    assert!(
        cleanup.is_ok(),
        "race-test table cleanup failed: {cleanup:?}"
    );
    assert!(
        exercise.is_ok(),
        "concurrent guarded Apply test failed: {exercise:?}"
    );
}

// Execute this ignored test against separate PostgreSQL 16, 17, and 18
// instances using PGXRAY_TEST_* variables. It changes no DB objects.
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL instance (run on PG16, PG17, PG18)"]
async fn integration_autovacuum_collect_and_table_detail_readonly() {
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
    let started = std::time::Instant::now();
    let snapshot = super::collector::collect(&registry, 1042, &input)
        .await
        .unwrap();
    eprintln!(
        "PostgreSQL {}: Autovacuum collection took {:?}",
        snapshot.overview.server_version_num,
        started.elapsed()
    );
    assert!(snapshot.tables.is_valid());
    assert!(snapshot.vacuums.is_valid());
    assert!(snapshot.analyzes.is_valid());
    assert!(
        snapshot.tables.error.is_none(),
        "{:?}",
        snapshot.tables.error
    );
    assert!(
        snapshot.vacuums.error.is_none(),
        "{:?}",
        snapshot.vacuums.error
    );
    assert!(
        snapshot.analyzes.error.is_none(),
        "{:?}",
        snapshot.analyzes.error
    );
    if snapshot.overview.server_version_num >= 180_000 {
        assert!(snapshot.overview.autovacuum_vacuum_max_threshold.is_some());
    } else {
        assert!(snapshot.overview.autovacuum_vacuum_max_threshold.is_none());
    }
    // Do not silently skip this half of the integration test on an empty DB.
    // The PG16/17/18 CI job creates a real user table before this test runs.
    let table = snapshot
        .tables
        .data
        .as_ref()
        .and_then(|rows| rows.first())
        .expect("Autovacuum integration test needs at least one user table; use a disposable DB and create the fixture before running");
    let started = std::time::Instant::now();
    let detail = super::collector::table_detail(&registry, 1042, &input, table.relation_oid)
        .await
        .expect("single-table detail must succeed");
    eprintln!("Single-table detail took {:?}", started.elapsed());
    assert_eq!(detail.table.relation_oid, table.relation_oid);
    assert!(detail.indexes.is_valid());
    assert!(detail.update_statements.is_valid());
    assert!(detail.indexes.error.is_none(), "{:?}", detail.indexes.error);
}

#[test]
fn collector_errors_distinguish_limits_from_postgresql_errors() {
    use super::collector::explain_section_error;

    let too_large = explain_section_error(
        "Таблицы",
        "PostgreSQL output слишком большой; максимум 33554432 bytes",
    );
    assert!(too_large.contains("AV-COL-OUTPUT-LIMIT"));
    assert!(too_large.contains("32 МиБ"));
    assert!(too_large.contains("Техническая причина"));

    let timed_out = explain_section_error("Таблицы", "PostgreSQL запрос превысил timeout 15000 ms");
    assert!(timed_out.contains("AV-COL-TIMEOUT"));
    assert!(timed_out.contains("не означает, что база неисправна"));

    let pg_error = explain_section_error("Активные ANALYZE", "permission denied (SQLSTATE 42501)");
    assert!(pg_error.contains("AV-COL-QUERY-ERROR"));
    assert!(pg_error.contains("SQLSTATE 42501"));
    assert!(!pg_error.contains("AV-COL-TIMEOUT"));
}

#[test]
fn reference_includes_collection_and_table_detail_queries() {
    use std::collections::HashSet;

    use super::{queries, reference::reference};

    let reference = reference();
    let ids: HashSet<_> = reference
        .queries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(
        ids.len(),
        reference.queries.len(),
        "duplicate reference query ID"
    );
    assert_eq!(
        ids,
        HashSet::from([
            "overview",
            "tables",
            "table-detail",
            "vacuum-progress",
            "analyze-progress",
            "table-indexes",
            "update-evidence",
        ]),
    );

    for entry in &reference.queries {
        assert!(!entry.title.trim().is_empty(), "{}: no title", entry.id);
        assert!(!entry.purpose.trim().is_empty(), "{}: no purpose", entry.id);
        assert!(
            !entry.how_to_read.is_empty(),
            "{}: no usage notes",
            entry.id
        );
        assert!(!entry.sql.trim().is_empty(), "{}: no SQL", entry.id);
    }

    let find = |id: &str| {
        reference
            .queries
            .iter()
            .find(|entry| entry.id == id)
            .unwrap()
    };
    assert_eq!(find("overview").sql, queries::OVERVIEW_SQL);
    assert_eq!(find("tables").sql, queries::TABLES_SQL);
    assert_eq!(find("vacuum-progress").sql, queries::VACUUM_PROGRESS_SQL);
    assert_eq!(find("analyze-progress").sql, queries::ANALYZE_PROGRESS_SQL);

    let oid = u32::MAX;
    let expected = |sql: String| sql.replace(&format!("{oid}::oid"), "<relation_oid>::oid");
    assert_eq!(
        find("table-detail").sql,
        expected(queries::table_sql(oid).unwrap())
    );
    assert_eq!(
        find("table-indexes").sql,
        expected(queries::indexes_sql(oid))
    );
    assert_eq!(
        find("update-evidence").sql,
        expected(queries::update_statements_sql(oid))
    );
    for id in ["table-detail", "table-indexes", "update-evidence"] {
        let sql = &find(id).sql;
        assert!(
            sql.contains("<relation_oid>::oid"),
            "{id}: placeholder missing"
        );
        assert!(
            !sql.contains(&format!("{oid}::oid")),
            "{id}: example OID leaked"
        );
    }
}

#[test]
fn reference_explains_threshold_versions_and_evidence_limits() {
    let reference = super::reference::reference();
    let steps: Vec<_> = reference
        .methodology
        .iter()
        .map(|step| step.id.as_str())
        .collect();
    assert_eq!(
        steps,
        [
            "profile",
            "effective-settings",
            "history",
            "advisor",
            "hot",
            "policy"
        ],
    );
    let all_notes = reference
        .methodology
        .iter()
        .flat_map(|step| step.how_to_read.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    for expected in [
        "floor(порог) + 1",
        "PostgreSQL 18+",
        "autovacuum_vacuum_max_threshold",
        "незамороженных страниц",
        "ANALYZE",
        "stats_reset",
        "guarded", // ensure apply checks remain mentioned? See below.
    ] {
        assert!(all_notes.contains(expected), "methodology omits {expected}");
    }
    assert!(!all_notes.contains("candidate ≈ (target - threshold) / reltuples"));
    assert_eq!(reference.sources.len(), 6);
    for source in &reference.sources {
        assert!(!source.path.trim().is_empty());
        assert!(!source.path.starts_with("http"));
    }
}
