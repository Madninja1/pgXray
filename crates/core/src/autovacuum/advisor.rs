use super::model::*;

const MAINTENANCE_MIN_HISTORY_POINTS: usize = 3;
const MAINTENANCE_MIN_HISTORY_SPAN_SECONDS: f64 = 1_800.0;
const MAINTENANCE_ELIGIBILITY_GRACE_SECONDS: f64 = 120.0;
// An observation gap longer than this cannot support a "repeated pressure"
// finding. This does not prove uninterrupted eligibility between snapshots.
const MAINTENANCE_MAX_OBSERVATION_GAP_SECONDS: f64 = 3_600.0;
const LOW_HOT_PCT: f64 = 5.0;

#[derive(Debug)]
struct MaintenanceMetrics {
    history_points: usize,
    history_span_seconds: Option<f64>,
    stats_period_continuous: bool,
    inserts_per_hour: Option<f64>,
    updates_per_hour: Option<f64>,
    deletes_per_hour: Option<f64>,
    hot_update_pct: Option<f64>,
    vacuum_cycles: Option<i64>,
    autovacuum_cycles: Option<i64>,
    analyze_cycles: Option<i64>,
    autoanalyze_cycles: Option<i64>,
    toast_vacuum_cycles: Option<i64>,
    dead_growth_per_hour: Option<f64>,
    dead_trigger_reached_points: usize,
    dead_trigger_reached_span_seconds: Option<f64>,
    insert_trigger_reached_points: usize,
    insert_trigger_reached_span_seconds: Option<f64>,
    analyze_trigger_reached_points: usize,
    analyze_trigger_reached_span_seconds: Option<f64>,
    toast_dead_trigger_reached_points: usize,
    toast_insert_trigger_reached_points: usize,
    toast_trigger_reached_span_seconds: Option<f64>,
}

pub(crate) fn advise_maintenance(
    table: &AutovacuumTable,
    track_counts: bool,
    active_vacuum: Option<&AutovacuumVacuumProgress>,
    active_analyze: Option<&AutovacuumAnalyzeProgress>,
    active_toast_vacuum: Option<&AutovacuumVacuumProgress>,
    history: &[AutovacuumMaintenanceHistorySample],
    autovacuum_naptime: Option<&str>,
) -> AutovacuumMaintenanceAdvice {
    let mut limitations = vec![
        crate::i18n::text("n_dead_tup, n_live_tup и reltuples — оценки PostgreSQL statistics/catalog; Advisor не трактует их как physical bloat.", "n_dead_tup, n_live_tup, and reltuples are PostgreSQL statistics/catalog estimates; the Advisor does not treat them as physical bloat.")
            .to_owned(),
        crate::i18n::text("Достижение расчётного порога в нескольких снимках — повод проверить обслуживание, но не доказательство непрерывного превышения или сбоя планировщика.", "Reaching an estimated threshold in several snapshots warrants checking maintenance, but does not prove continuous exceedance or a scheduler failure.")
            .to_owned(),
        crate::i18n::text("Freeze/XID policy здесь не тюнингуется: anti-wraparound lifecycle остаётся задачей XID / Freeze.", "Freeze/XID policy is not tuned here: anti-wraparound lifecycle belongs to XID / Freeze.")
            .to_owned(),
        crate::i18n::text("Advisor не меняет PostgreSQL: рассчитанные варианты настройки требуют отдельной проверки и подтверждения DBA.", "The Advisor does not modify PostgreSQL: calculated setting options require separate DBA review and confirmation.")
            .to_owned(),
    ];
    let grace_seconds = autovacuum_naptime
        .and_then(parse_autovacuum_naptime_seconds)
        .map(|naptime| MAINTENANCE_ELIGIBILITY_GRACE_SECONDS.max(naptime * 2.0));
    limitations.push(
        crate::i18n::text("Повторное превышение порога подтверждается только в моменты снимков: между ними VACUUM/ANALYZE мог выполниться. Это повод проверить планировщик, но не доказательство его сбоя.", "Repeated threshold exceedance is confirmed only at snapshot times: VACUUM/ANALYZE may have run between them. This warrants checking scheduling but does not prove a scheduler failure.").into(),
    );
    limitations.push(
        crate::i18n::text("Advisor не измеряет занятость autovacuum workers во всех базах экземпляра; задержка может быть связана с конкурирующей нагрузкой. Разрывы между снимками более часа не подтверждают повторное превышение порога.", "The Advisor does not measure autovacuum worker activity across all databases on the instance; delays may reflect competing workload. Gaps over an hour do not confirm repeated threshold exceedance.").into(),
    );
    if grace_seconds.is_none() {
        limitations.push(crate::i18n::text("Не удалось определить autovacuum_naptime: устойчивую задержку по истории не подтверждаем.", "Could not determine autovacuum_naptime: a persistent historical delay cannot be confirmed.").into());
    }
    let metrics = if track_counts {
        maintenance_metrics(history, &mut limitations)
    } else {
        limitations.push(crate::i18n::text("track_counts отключён: оценки по счётчикам и рекомендации по обслуживанию не формируются.", "track_counts is disabled: counter-based assessments and maintenance recommendations are unavailable.").into());
        maintenance_metrics(&[], &mut limitations)
    };
    let sufficient_history = metrics.history_points >= MAINTENANCE_MIN_HISTORY_POINTS
        && metrics
            .history_span_seconds
            .is_some_and(|span| span >= MAINTENANCE_MIN_HISTORY_SPAN_SECONDS);
    let mut findings = Vec::new();
    // Missing stats_reset is not evidence of a reset or evidence of continuity.
    // Keep current-snapshot observations useful without turning them into rates
    // or actionable policy candidates.
    if track_counts
        && history
            .last()
            .is_some_and(|sample| sample.stats_reset.is_none())
    {
        limitations.push(
            "stats_reset = NULL: моментальные значения порогов доступны, но скрытый сброс накопительных счётчиков между снимками исключить нельзя. Исторические скорости и Policy Apply по ним не подтверждены; pg_stat_reset() ради Advisor не требуется."
                .into(),
        );
    }

    if !track_counts {
        findings.push(maintenance_finding(
            "track-counts-disabled", "history", "insufficient_quality",
            "Статистика изменений PostgreSQL недоступна",
            "Нельзя оценивать эффективность VACUUM и ANALYZE по счётчикам при track_counts=off.",
            vec!["track_counts=off".into()], Vec::new(),
            vec!["Проверить серверную настройку track_counts и собрать новый снимок после включения статистики.".into()],
        ));
    } else if !table.autovacuum_applicable {
        findings.push(maintenance_finding(
            "partitioned-parent",
            "analyze",
            "review",
            "Родитель секционированной таблицы требует отдельного подхода",
            "Родительская секционированная таблица не обслуживается обычным Autovacuum как физические партиции. Её статистику нужно оценивать отдельно.",
            vec![format!(
                "{}.{}, relation_type={}",
                table.schema_name, table.relation_name, table.relation_type,
            )],
            Vec::new(),
            vec![
                "Проверить физические партиции по отдельности.".into(),
                "При изменении распределения данных оценить необходимость ручного ANALYZE родительской таблицы.".into(),
            ],
        ));
    } else {
        if !table.autovacuum_enabled {
            findings.push(maintenance_finding(
                "autovacuum-disabled",
                "vacuum",
                "investigate",
                "Regular autovacuum для relation выключен",
                "Dead/insert/analyze thresholds не дадут обычный automatic maintenance cycle, пока effective autovacuum_enabled=false.",
                vec![format!(
                    "{}.{}: autovacuum_enabled=false; reloptions={}",
                    table.schema_name,
                    table.relation_name,
                    table
                        .reloptions
                        .as_ref()
                        .map(|values| values.join(", "))
                        .unwrap_or_else(|| "global/default context".into()),
                )],
                Vec::new(),
                vec![
                    "Проверить, является ли отключение осознанной per-table/global policy.".into(),
                    "До threshold tuning определить, какой external/manual maintenance process обслуживает relation.".into(),
                ],
            ));
        }

        if !sufficient_history {
            limitations.push(format!(
                "Для оценки обслуживания нужны минимум {MAINTENANCE_MIN_HISTORY_POINTS} снимка и {:.0} минут наблюдений.",
                MAINTENANCE_MIN_HISTORY_SPAN_SECONDS / 60.0,
            ));
            findings.push(maintenance_finding(
                "history-insufficient",
                "history",
                "insufficient_history",
                "Недостаточно снимков для оценки обслуживания",
                "По имеющейся истории нельзя надёжно оценить регулярность VACUUM и ANALYZE, независимо от того, достигнуты ли пороги.",
                vec![format!(
                    "Сопоставимых снимков: {}; период наблюдений: {}.",
                    metrics.history_points,
                    metrics
                        .history_span_seconds
                        .map(|value| format!("{:.1} мин", value / 60.0))
                        .unwrap_or_else(|| "неизвестен".into()),
                )],
                Vec::new(),
                vec!["Собрать ещё снимки без сброса статистики и изменения настроек.".into()],
            ));
        } else if !metrics.stats_period_continuous {
            limitations.push(
                "Накопительные счётчики и настройки обслуживания не образуют сопоставимый период наблюдений."
                    .into(),
            );
            findings.push(maintenance_finding(
                "history-quality",
                "history",
                "insufficient_quality",
                "История содержит несопоставимые наблюдения",
                "Накопительные счётчики или действующие настройки изменились между снимками; для рекомендаций используем только последнюю сопоставимую часть истории.",
                vec!["stats_period_continuous=false".into()],
                Vec::new(),
                vec!["Собрать новую серию снимков после сброса статистики или изменения настроек.".into()],
            ));
        } else if table.autovacuum_enabled {
            push_vacuum_finding(&mut findings, table, active_vacuum, &metrics, grace_seconds);
            push_insert_finding(&mut findings, table, active_vacuum, &metrics, grace_seconds);
            push_analyze_finding(
                &mut findings,
                table,
                active_analyze,
                &metrics,
                grace_seconds,
            );
            push_toast_finding(
                &mut findings,
                table,
                active_toast_vacuum,
                &metrics,
                grace_seconds,
            );
            push_hot_finding(&mut findings, table, &metrics);
        }
    }

    // Screening is independent of history-based cadence. In particular, when
    // stats_reset is NULL we can still report threshold/progress at this instant,
    // but must never label these signals as a sustained condition or candidate.
    if track_counts && table.autovacuum_applicable && !metrics.stats_period_continuous {
        push_snapshot_screening(&mut findings, table, active_vacuum, active_analyze);
    }

    if findings.is_empty() {
        findings.push(maintenance_finding(
            "maintenance-observe",
            "vacuum",
            "observe",
            "Пока нет явных признаков отставания обслуживания",
            "История не показывает достаточно оснований для изменения настроек этой таблицы.",
            Vec::new(),
            Vec::new(),
            vec!["Продолжить сбор снимков и наблюдать за VACUUM и ANALYZE.".into()],
        ));
    }

    let status = findings
        .iter()
        .max_by_key(|finding| maintenance_status_rank(&finding.status))
        .map(|finding| finding.status.clone())
        .unwrap_or_else(|| "observe".into());
    let (title, summary) = maintenance_advice_summary(&status);

    AutovacuumMaintenanceAdvice {
        relation_oid: table.relation_oid,
        relation_name: format!("{}.{}", table.schema_name, table.relation_name),
        status,
        title: title.into(),
        summary: summary.into(),
        history_points: metrics.history_points,
        history_span_seconds: metrics.history_span_seconds,
        stats_period_continuous: metrics.stats_period_continuous,
        inserts_per_hour: metrics.inserts_per_hour,
        updates_per_hour: metrics.updates_per_hour,
        deletes_per_hour: metrics.deletes_per_hour,
        hot_update_pct: metrics.hot_update_pct,
        vacuum_cycles: metrics.vacuum_cycles,
        autovacuum_cycles: metrics.autovacuum_cycles,
        analyze_cycles: metrics.analyze_cycles,
        autoanalyze_cycles: metrics.autoanalyze_cycles,
        toast_vacuum_cycles: metrics.toast_vacuum_cycles,
        findings,
        limitations,
    }
}

fn push_snapshot_screening(
    findings: &mut Vec<AutovacuumMaintenanceFinding>,
    table: &AutovacuumTable,
    active_vacuum: Option<&AutovacuumVacuumProgress>,
    active_analyze: Option<&AutovacuumAnalyzeProgress>,
) {
    let mut observed = Vec::new();
    if table.dead_trigger_reached {
        observed.push(format!(
            "Текущий dead-tuple trigger: {} / {} (оценка, не физический bloat).",
            table.dead_tuples, table.dead_tuple_trigger,
        ));
    }
    if table.insert_trigger_reached {
        if let Some(trigger) = table.insert_trigger {
            observed.push(format!(
                "Текущий INSERT trigger: {} / {}.",
                table.inserts_since_vacuum, trigger,
            ));
        }
    }
    if table.analyze_trigger_reached {
        observed.push(format!(
            "Текущий ANALYZE trigger: {} / {}.",
            table.modifications_since_analyze, table.analyze_trigger,
        ));
    }
    if let Some(vacuum) = active_vacuum {
        observed.push(format!(
            "VACUUM наблюдается сейчас: PID {}, phase={}",
            vacuum.pid, vacuum.phase,
        ));
    }
    if let Some(analyze) = active_analyze {
        observed.push(format!("ANALYZE наблюдается сейчас: PID {}", analyze.pid));
    }
    if observed.is_empty() {
        return;
    }
    findings.push(maintenance_finding(
        "snapshot-only-signals",
        "history",
        "review",
        "Сигналы текущего снимка (без подтверждённой истории)",
        "Это наблюдения в момент сбора. Без сопоставимой истории нельзя утверждать, что обслуживание систематически запаздывает, рассчитывать частоту или предлагать числовую Policy.",
        observed,
        Vec::new(),
        vec![
            "Проверить live VACUUM/ANALYZE, locks и занятость workers, прежде чем изменять настройки.".into(),
            "Для подтверждённого cadence нужны сопоставимые snapshots с известным stats_reset. Не сбрасывать статистику production только ради Advisor.".into(),
        ],
    ));
}

fn push_vacuum_finding(
    findings: &mut Vec<AutovacuumMaintenanceFinding>,
    table: &AutovacuumTable,
    active_vacuum: Option<&AutovacuumVacuumProgress>,
    metrics: &MaintenanceMetrics,
    grace_seconds: Option<f64>,
) {
    let cycles = metrics.autovacuum_cycles.unwrap_or(0);
    let manual_cycles = metrics.vacuum_cycles.unwrap_or(0).saturating_sub(cycles);
    let repeated = metrics.dead_trigger_reached_points >= 2;
    let sustained =
        eligibility_is_sustained(metrics.dead_trigger_reached_span_seconds, grace_seconds);
    let current = table.dead_trigger_reached;

    if let Some(vacuum) = active_vacuum {
        findings.push(maintenance_finding(
            "vacuum-active",
            "vacuum",
            "working",
            "VACUUM уже выполняется",
            "Сначала дождаться результата текущего cycle; менять threshold поверх active VACUUM не требуется.",
            vec![format!(
                "PID {} · phase={} · scan={} / {} blocks",
                vacuum.pid, vacuum.phase, vacuum.heap_blks_scanned, vacuum.heap_blks_total,
            )],
            Vec::new(),
            vec!["После завершения собрать snapshot и проверить dead tuples и следующий trigger state.".into()],
        ));
        return;
    }

    if current && repeated && sustained && cycles == 0 {
        findings.push(maintenance_finding(
            "dead-trigger-no-cycle",
            "vacuum",
            "investigate",
            "Порог VACUUM достигнут в нескольких снимках, автоматические циклы не обнаружены",
            "Таблица превышала порог при нескольких проверках; отсутствие автоматического VACUUM требует расследования. Ручное обслуживание учитывается отдельно. Уменьшать порог без выяснения причины преждевременно.",
            vec![
                format!(
                    "dead tuples={} / trigger={} ({:.1}% used)",
                    table.dead_tuples,
                    table.dead_tuple_trigger,
                    table.dead_trigger_used_pct.unwrap_or(0.0),
                ),
                format!(
                    "порог превышен в {} из {} снимков; интервал наблюдений={}; автоматических VACUUM=0; ручных VACUUM={manual_cycles}",
                    metrics.dead_trigger_reached_points,
                    metrics.history_points,
                    format_eligibility_span(metrics.dead_trigger_reached_span_seconds),
                ),
            ],
            Vec::new(),
            vec![
                "Проверить autovacuum workers, locks, competing databases, cost throttling, PostgreSQL logs и I/O pressure.".into(),
                "Не снижать scale factor только потому, что relation уже выше текущего trigger.".into(),
            ],
        ));
    } else if current && repeated && sustained && cycles > 0 {
        findings.push(maintenance_finding(
            "dead-trigger-repeated-with-cycles",
            "vacuum",
            "candidate",
            "Autovacuum выполнялся, но порог VACUUM достигается повторно",
            "Это candidate для per-table VACUUM cadence review. Concrete Policy Candidates могут построить scale-factor scenarios только при наличии clean rate evidence; применение остаётся отдельным explicit guarded action.",
            vec![
                format!("Observed autovacuum cycles={cycles}."),
                format!(
                    "dead trigger reached in {} / {} history points; current dead={} / trigger={}.",
                    metrics.dead_trigger_reached_points,
                    metrics.history_points,
                    table.dead_tuples,
                    table.dead_tuple_trigger,
                ),
            ],
            vec![
                "autovacuum_vacuum_threshold".into(),
                "autovacuum_vacuum_scale_factor".into(),
            ],
            vec![
                "Сравнить history-derived scenarios с допустимым числом dead tuples / желаемым cadence; Manual calculator оставить для собственного target.".into(),
                "Перед изменением проверить длительность/стоимость естественного VACUUM cycle и index pressure.".into(),
            ],
        ));
    } else if current {
        findings.push(maintenance_finding(
            "dead-trigger-current",
            "vacuum",
            "observe",
            "Dead-tuple trigger достигнут в anchor snapshot",
            "Одной/редкой точки выше trigger недостаточно, чтобы отличить штатную eligibility от устойчивого отставания.",
            vec![format!(
                "dead tuples={} / trigger={}",
                table.dead_tuples, table.dead_tuple_trigger,
            )],
            Vec::new(),
            vec!["Проверить следующий snapshot и появление VACUUM counter/progress.".into()],
        ));
    } else if cycles > 0 {
        findings.push(maintenance_finding(
            "vacuum-cycle-observed",
            "vacuum",
            "working",
            "Autovacuum lifecycle наблюдается",
            "В history есть autovacuum cycle, а anchor snapshot находится ниже dead-tuple trigger.",
            vec![format!(
                "Observed autovacuum cycles={cycles}; current dead={} / trigger={}.",
                table.dead_tuples, table.dead_tuple_trigger,
            )],
            Vec::new(),
            vec!["Продолжить наблюдение, особенно после изменения workload.".into()],
        ));
    } else if let Some(rate) = metrics.dead_growth_per_hour.filter(|rate| *rate > 0.0) {
        let remaining = (table.dead_tuple_trigger - table.dead_tuples).max(0);
        let eta = remaining as f64 / rate;
        findings.push(maintenance_finding(
            "dead-growth",
            "vacuum",
            "observe",
            "Dead tuples растут к текущему trigger",
            "Пока trigger не достигнут и observed VACUUM cycle в window нет; history можно использовать для оценки времени до eligibility.",
            vec![
                format!("Dead growth ≈ {rate:.0}/h."),
                format!("До trigger осталось ≈ {remaining}; ETA ≈ {eta:.1} h."),
            ],
            Vec::new(),
            vec!["Сравнить фактический cycle с ETA после достижения trigger.".into()],
        ));
    }
}

fn push_insert_finding(
    findings: &mut Vec<AutovacuumMaintenanceFinding>,
    table: &AutovacuumTable,
    active_vacuum: Option<&AutovacuumVacuumProgress>,
    metrics: &MaintenanceMetrics,
    grace_seconds: Option<f64>,
) {
    let Some(trigger) = table.insert_trigger else {
        if table.workload_profile == "append_only_observed"
            && metrics.inserts_per_hour.is_some_and(|rate| rate > 0.0)
        {
            findings.push(maintenance_finding(
                "insert-trigger-disabled",
                "insert",
                "review",
                "Append-only workload наблюдается при disabled insert trigger",
                "Это не ошибка само по себе, но regular VACUUM cadence для append-only relation не будет инициироваться insert-trigger механизмом.",
                vec![format!(
                    "Observed INSERT rate ≈ {:.0}/h; insert trigger disabled.",
                    metrics.inserts_per_hour.unwrap_or(0.0),
                )],
                Vec::new(),
                vec![
                    "Понять, нужен ли regular VACUUM этой append-only relation помимо XID/freeze maintenance.".into(),
                    "Не включать insert trigger только ради снижения XID age; freeze policy рассматривается отдельно.".into(),
                ],
            ));
        }
        return;
    };

    if active_vacuum.is_some() {
        return;
    }

    let cycles = metrics.autovacuum_cycles.unwrap_or(0);
    let manual_cycles = metrics.vacuum_cycles.unwrap_or(0).saturating_sub(cycles);
    let repeated = metrics.insert_trigger_reached_points >= 2;
    let sustained =
        eligibility_is_sustained(metrics.insert_trigger_reached_span_seconds, grace_seconds);
    if table.insert_trigger_reached && repeated && sustained && cycles == 0 {
        findings.push(maintenance_finding(
            "insert-trigger-no-cycle",
            "insert",
            "investigate",
            "Порог VACUUM по вставкам достигнут в нескольких снимках без автоматических циклов",
            "Сначала проверить причины отсутствия автоматического обслуживания с учётом ручных VACUUM; снижение коэффициента не устранит занятость workers или блокировки.",
            vec![format!(
                "вставок после VACUUM={} / порог={}; превышен в {} из {} снимков; интервал наблюдений={}; автоматических VACUUM=0; ручных VACUUM={manual_cycles}",
                table.inserts_since_vacuum,
                trigger,
                metrics.insert_trigger_reached_points,
                metrics.history_points,
                format_eligibility_span(metrics.insert_trigger_reached_span_seconds),
            )],
            Vec::new(),
            vec!["Проверить scheduler/workers/locks/cost и следующий snapshot.".into()],
        ));
    } else if table.insert_trigger_reached && repeated && sustained && cycles > 0 {
        findings.push(maintenance_finding(
            "insert-trigger-repeated-with-cycles",
            "insert",
            "candidate",
            "Insert trigger повторно достигается между VACUUM cycles",
            "Для append-heavy workload это candidate для отдельного insert-cadence review; concrete scenario строится только из clean inserts_since_vacuum growth, а dead-tuple policy не подменяет insert policy.",
            vec![format!(
                "Observed autovacuum cycles={cycles}; inserts since vacuum={} / trigger={}.",
                table.inserts_since_vacuum, trigger,
            )],
            vec![
                "autovacuum_vacuum_insert_threshold".into(),
                "autovacuum_vacuum_insert_scale_factor".into(),
            ],
            vec!["Сопоставить history-derived insert scenarios с допустимым interval/объёмом inserts между VACUUM cycles.".into()],
        ));
    } else if table.insert_trigger_reached {
        findings.push(maintenance_finding(
            "insert-trigger-current",
            "insert",
            "observe",
            "Insert trigger достигнут в anchor snapshot",
            "Нужен следующий snapshot, чтобы понять, появился ли естественный VACUUM cycle.",
            vec![format!(
                "inserts since vacuum={} / trigger={}",
                table.inserts_since_vacuum, trigger,
            )],
            Vec::new(),
            vec!["Повторить snapshot после autovacuum_naptime/cycle.".into()],
        ));
    } else if cycles == 0 {
        if let Some(rate) = metrics.inserts_per_hour.filter(|rate| *rate > 0.0) {
            let remaining = (trigger - table.inserts_since_vacuum).max(0);
            findings.push(maintenance_finding(
                "insert-growth",
                "insert",
                "observe",
                "INSERT workload движется к insert trigger",
                "При отсутствии VACUUM cycle в текущем window можно приблизительно оценить время до insert eligibility.",
                vec![
                    format!("INSERT rate ≈ {rate:.0}/h."),
                    format!(
                        "До trigger осталось ≈ {remaining}; ETA ≈ {:.1} h.",
                        remaining as f64 / rate,
                    ),
                ],
                Vec::new(),
                vec!["Проверить, совпадёт ли фактический VACUUM cadence с этой оценкой.".into()],
            ));
        }
    }
}

fn push_analyze_finding(
    findings: &mut Vec<AutovacuumMaintenanceFinding>,
    table: &AutovacuumTable,
    active_analyze: Option<&AutovacuumAnalyzeProgress>,
    metrics: &MaintenanceMetrics,
    grace_seconds: Option<f64>,
) {
    if let Some(analyze) = active_analyze {
        findings.push(maintenance_finding(
            "analyze-active",
            "analyze",
            "working",
            "ANALYZE уже выполняется",
            "Planner statistics maintenance активен; сначала дождаться результата текущего cycle.",
            vec![format!("PID {} · phase={}", analyze.pid, analyze.phase)],
            Vec::new(),
            vec!["После завершения проверить modifications_since_analyze и новый snapshot.".into()],
        ));
        return;
    }

    let cycles = metrics.autoanalyze_cycles.unwrap_or(0);
    let manual_cycles = metrics.analyze_cycles.unwrap_or(0).saturating_sub(cycles);
    let repeated = metrics.analyze_trigger_reached_points >= 2;
    let sustained =
        eligibility_is_sustained(metrics.analyze_trigger_reached_span_seconds, grace_seconds);
    if table.analyze_trigger_reached && repeated && sustained && cycles == 0 {
        findings.push(maintenance_finding(
            "analyze-trigger-no-cycle",
            "analyze",
            "investigate",
            "Порог ANALYZE достигнут в нескольких снимках без автоматических циклов",
            "Проверить запуск Autoanalyze и наличие ручных ANALYZE. Уменьшение порога не устранит проблему планировщика или ожидания блокировки.",
            vec![format!(
                "изменений после ANALYZE={} / порог={}; превышен в {} из {} снимков; интервал наблюдений={}; автоматических ANALYZE=0; ручных ANALYZE={manual_cycles}",
                table.modifications_since_analyze,
                table.analyze_trigger,
                metrics.analyze_trigger_reached_points,
                metrics.history_points,
                format_eligibility_span(metrics.analyze_trigger_reached_span_seconds),
            )],
            Vec::new(),
            vec!["Проверить autoanalyze scheduling, locks, workers и следующий snapshot.".into()],
        ));
    } else if table.analyze_trigger_reached && repeated && sustained && cycles > 0 {
        findings.push(maintenance_finding(
            "analyze-trigger-repeated-with-cycles",
            "analyze",
            "candidate",
            "ANALYZE cycles есть, но relation повторно достигает analyze trigger",
            "Это candidate для planner-statistics cadence review. Concrete scenario может быть рассчитан из clean modifications_since_analyze growth, но query/plan evidence всё равно нужен до production change.",
            vec![format!(
                "Observed autoanalyze cycles={cycles}; modifications={} / trigger={}.",
                table.modifications_since_analyze, table.analyze_trigger,
            )],
            vec![
                "autovacuum_analyze_threshold".into(),
                "autovacuum_analyze_scale_factor".into(),
            ],
            vec![
                "Сопоставить с plan/cardinality evidence перед уменьшением analyze threshold.".into(),
                "VACUUM cadence не менять автоматически вместе с ANALYZE cadence.".into(),
            ],
        ));
    } else if table.analyze_trigger_reached {
        findings.push(maintenance_finding(
            "analyze-trigger-current",
            "analyze",
            "observe",
            "ANALYZE trigger достигнут в anchor snapshot",
            "Одна точка eligibility ещё не показывает устойчивое отставание autoanalyze.",
            vec![format!(
                "modifications since analyze={} / trigger={}",
                table.modifications_since_analyze, table.analyze_trigger,
            )],
            Vec::new(),
            vec!["Проверить следующий ANALYZE counter и состояние planner statistics.".into()],
        ));
    } else if cycles > 0 {
        findings.push(maintenance_finding(
            "analyze-cycle-observed",
            "analyze",
            "working",
            "ANALYZE lifecycle наблюдается",
            "В history есть ANALYZE cycle, а anchor snapshot ниже текущего analyze trigger.",
            vec![format!("Observed autoanalyze cycles={cycles}." )],
            Vec::new(),
            vec!["Тюнинг analyze cadence нужен только при подтверждённой проблеме planner statistics.".into()],
        ));
    }
}

fn push_toast_finding(
    findings: &mut Vec<AutovacuumMaintenanceFinding>,
    table: &AutovacuumTable,
    active_toast_vacuum: Option<&AutovacuumVacuumProgress>,
    metrics: &MaintenanceMetrics,
    grace_seconds: Option<f64>,
) {
    let Some(toast) = table.toast.as_ref() else {
        return;
    };

    if !toast.autovacuum_enabled {
        findings.push(maintenance_finding(
            "toast-autovacuum-disabled",
            "toast",
            "investigate",
            "Regular autovacuum для TOAST выключен",
            "TOAST — отдельная relation; состояние parent table не компенсирует disabled TOAST maintenance policy.",
            vec![format!("{}.{}: autovacuum_enabled=false", toast.schema_name, toast.relation_name)],
            Vec::new(),
            vec!["Проверить toast.autovacuum_* reloptions parent table и внешний maintenance process.".into()],
        ));
        return;
    }

    if let Some(vacuum) = active_toast_vacuum {
        findings.push(maintenance_finding(
            "toast-vacuum-active",
            "toast",
            "working",
            "TOAST VACUUM уже выполняется",
            "Для TOAST relation виден active VACUUM; сначала проверить результат cycle.",
            vec![format!("PID {} · phase={}", vacuum.pid, vacuum.phase)],
            Vec::new(),
            vec!["После завершения проверить TOAST dead/insert trigger state.".into()],
        ));
        return;
    }

    let current = toast.dead_trigger_reached || toast.insert_trigger_reached;
    let Some(cycles) = metrics.toast_vacuum_cycles else {
        if current {
            findings.push(maintenance_finding(
                "toast-cycles-unavailable", "toast", "insufficient_quality",
                "Недостаточно данных о VACUUM для TOAST",
                "Счётчик циклов TOAST не удалось сопоставить между снимками. Отсутствие данных не означает, что очистки не было.",
                vec![format!("TOAST: {}.{}", toast.schema_name, toast.relation_name)],
                Vec::new(),
                vec!["Собрать сопоставимые снимки и проверить последний autovacuum и его прогресс для TOAST.".into()],
            ));
        }
        return;
    };
    let repeated = metrics.toast_dead_trigger_reached_points >= 2
        || metrics.toast_insert_trigger_reached_points >= 2;
    let sustained =
        eligibility_is_sustained(metrics.toast_trigger_reached_span_seconds, grace_seconds);
    if current && repeated && sustained && cycles == 0 {
        findings.push(maintenance_finding(
            "toast-trigger-no-cycle",
            "toast",
            "investigate",
            "Порог TOAST достигнут в нескольких снимках без автоматических циклов",
            "Сначала проверить обслуживание TOAST и возможные задержки, а не автоматически уменьшать пороги.",
            vec![format!(
                "TOAST dead={} / {}; inserts={} / {}",
                toast.dead_tuples,
                toast.dead_tuple_trigger,
                toast.inserts_since_vacuum,
                toast
                    .insert_trigger
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "disabled".into()),
            )],
            Vec::new(),
            vec!["Проверить workers/locks/cost и TOAST progress в следующем snapshot.".into()],
        ));
    } else if current && repeated && sustained && cycles > 0 {
        findings.push(maintenance_finding(
            "toast-trigger-repeated-with-cycles",
            "toast",
            "candidate",
            "TOAST maintenance требует отдельного cadence review",
            "TOAST autovacuum cycles наблюдаются, но trigger pressure повторяется. Concrete TOAST scenario рассчитывается отдельно; parent table policy нельзя механически переносить на TOAST.",
            vec![format!("Observed TOAST autovacuum cycles={cycles}." )],
            vec![
                "toast.autovacuum_vacuum_threshold".into(),
                "toast.autovacuum_vacuum_scale_factor".into(),
                "toast.autovacuum_vacuum_insert_threshold".into(),
                "toast.autovacuum_vacuum_insert_scale_factor".into(),
            ],
            vec!["Сначала выбрать, какой именно dead/insert trigger является причиной pressure, и рассчитывать его отдельно.".into()],
        ));
    } else if current {
        findings.push(maintenance_finding(
            "toast-trigger-current",
            "toast",
            "observe",
            "TOAST trigger достигнут в anchor snapshot",
            "Нужен следующий snapshot, чтобы отличить transient eligibility от устойчивого TOAST maintenance lag.",
            vec![format!(
                "dead reached={}; insert reached={}",
                toast.dead_trigger_reached, toast.insert_trigger_reached,
            )],
            Vec::new(),
            vec!["Проверить следующий TOAST autovacuum_count/progress.".into()],
        ));
    } else if cycles > 0 {
        findings.push(maintenance_finding(
            "toast-cycle-observed",
            "toast",
            "working",
            "TOAST maintenance lifecycle наблюдается",
            "В history есть TOAST autovacuum cycle, а anchor snapshot ниже TOAST triggers.",
            vec![format!("Observed TOAST autovacuum cycles={cycles}.")],
            Vec::new(),
            vec!["Продолжить наблюдение отдельно от parent relation.".into()],
        ));
    }
}

fn push_hot_finding(
    findings: &mut Vec<AutovacuumMaintenanceFinding>,
    table: &AutovacuumTable,
    metrics: &MaintenanceMetrics,
) {
    let Some(rate) = metrics.updates_per_hour.filter(|rate| *rate > 0.0) else {
        return;
    };
    let Some(hot_pct) = metrics.hot_update_pct else {
        return;
    };
    if hot_pct >= LOW_HOT_PCT {
        return;
    }

    findings.push(maintenance_finding(
        "low-hot-interval",
        "hot",
        "review",
        "Низкая доля HOT-обновлений за наблюдаемый период",
        "Менее 5% обновлений за наблюдаемый период прошли как HOT. Это эвристика для изучения индексов и обновляемых столбцов, а не универсальная норма и не основание автоматически менять настройки.",
        vec![
            format!("Observed UPDATE rate ≈ {rate:.0}/h."),
            format!("Interval HOT ≈ {hot_pct:.1}%; indexes={}.", table.index_count),
        ],
        Vec::new(),
        vec![
            "Проверить, обновляются ли indexed columns; это может делать HOT невозможным независимо от fillfactor.".into(),
            "Рассматривать fillfactor только после анализа индексов и характера UPDATE-запросов.".into(),
        ],
    ));
}

fn maintenance_metrics(
    history: &[AutovacuumMaintenanceHistorySample],
    limitations: &mut Vec<String>,
) -> MaintenanceMetrics {
    let samples = latest_continuous_maintenance_samples(history);
    if samples.len() < history.len() {
        limitations.push(
            "Снимки до последнего изменения счётчиков или настроек обслуживания исключены из расчётов. Для оценки требуется новый сопоставимый период."
                .into(),
        );
    }
    let history_points = samples.len();
    let (oldest, newest) = (samples.first(), samples.last());
    let history_span_seconds = match (oldest, newest) {
        (Some(oldest), Some(newest))
            if newest.collection_started_at_ms > oldest.collection_started_at_ms =>
        {
            Some(
                (newest.collection_started_at_ms - oldest.collection_started_at_ms) as f64 / 1000.0,
            )
        }
        _ => None,
    };
    let stats_period_continuous = history_points >= 2
        && samples
            .windows(2)
            .all(|pair| maintenance_stats_pair_continuous(&pair[0], &pair[1]));
    let elapsed_hours = history_span_seconds.map(|seconds| seconds / 3600.0);
    let rate = |older: Option<i64>, newer: Option<i64>| -> Option<f64> {
        let hours = elapsed_hours.filter(|hours| *hours > 0.0)?;
        let delta = monotonic_delta(older?, newer?)?;
        Some(delta as f64 / hours)
    };

    let inserts_per_hour = rate(
        oldest.map(|sample| sample.tuples_inserted),
        newest.map(|sample| sample.tuples_inserted),
    )
    .filter(|_| stats_period_continuous);
    let updates_per_hour = rate(
        oldest.map(|sample| sample.tuples_updated),
        newest.map(|sample| sample.tuples_updated),
    )
    .filter(|_| stats_period_continuous);
    let deletes_per_hour = rate(
        oldest.map(|sample| sample.tuples_deleted),
        newest.map(|sample| sample.tuples_deleted),
    )
    .filter(|_| stats_period_continuous);

    let updates_delta = match (oldest, newest) {
        (Some(oldest), Some(newest)) if stats_period_continuous => {
            monotonic_delta(oldest.tuples_updated, newest.tuples_updated)
        }
        _ => None,
    };
    let hot_delta = match (oldest, newest) {
        (Some(oldest), Some(newest)) if stats_period_continuous => {
            monotonic_delta(oldest.hot_updates, newest.hot_updates)
        }
        _ => None,
    };
    let hot_update_pct = match (hot_delta, updates_delta) {
        (Some(hot), Some(updates)) if updates > 0 => Some(hot as f64 / updates as f64 * 100.0),
        _ => None,
    };

    let manual_vacuum_cycles = counter_delta(oldest, newest, stats_period_continuous, |sample| {
        sample.vacuum_count
    });
    let autovacuum_cycles = counter_delta(oldest, newest, stats_period_continuous, |sample| {
        sample.autovacuum_count
    });
    let vacuum_cycles = match (manual_vacuum_cycles, autovacuum_cycles) {
        (Some(manual), Some(auto)) => manual.checked_add(auto),
        _ => None,
    };
    let manual_analyze_cycles = counter_delta(oldest, newest, stats_period_continuous, |sample| {
        sample.analyze_count
    });
    let autoanalyze_cycles = counter_delta(oldest, newest, stats_period_continuous, |sample| {
        sample.autoanalyze_count
    });
    let analyze_cycles = match (manual_analyze_cycles, autoanalyze_cycles) {
        (Some(manual), Some(auto)) => manual.checked_add(auto),
        _ => None,
    };
    let toast_vacuum_cycles = match (oldest, newest) {
        (Some(oldest), Some(newest)) if stats_period_continuous => {
            match (oldest.toast_autovacuum_count, newest.toast_autovacuum_count) {
                (Some(older), Some(newer)) => monotonic_delta(older, newer),
                _ => None,
            }
        }
        _ => None,
    };

    let dead_growth_per_hour = match (oldest, newest, elapsed_hours, vacuum_cycles) {
        (Some(oldest), Some(newest), Some(hours), Some(0)) if hours > 0.0 => {
            let delta = newest.dead_tuples - oldest.dead_tuples;
            (delta >= 0).then_some(delta as f64 / hours)
        }
        _ => None,
    };

    if stats_period_continuous && history_points >= 2 {
        if inserts_per_hour.is_none() || updates_per_hour.is_none() || deletes_per_hour.is_none() {
            limitations.push(
                "Один или несколько накопительных счётчиков DML уменьшились; скорости по ним не рассчитываются."
                    .into(),
            );
        }
        if vacuum_cycles.is_none() || analyze_cycles.is_none() {
            limitations.push(
                "Счётчики VACUUM или ANALYZE уменьшились; оценка частоты обслуживания ограничена."
                    .into(),
            );
        }
    }

    let dead_trigger_reached_span_seconds =
        trailing_reached_span_seconds(&samples, |sample| sample.dead_trigger_reached);
    let insert_trigger_reached_span_seconds =
        trailing_reached_span_seconds(&samples, |sample| sample.insert_trigger_reached);
    let analyze_trigger_reached_span_seconds =
        trailing_reached_span_seconds(&samples, |sample| sample.analyze_trigger_reached);
    let toast_trigger_reached_span_seconds = trailing_reached_span_seconds(&samples, |sample| {
        sample.toast_dead_trigger_reached == Some(true)
            || sample.toast_insert_trigger_reached == Some(true)
    });

    MaintenanceMetrics {
        history_points,
        history_span_seconds,
        stats_period_continuous,
        inserts_per_hour,
        updates_per_hour,
        deletes_per_hour,
        hot_update_pct,
        vacuum_cycles,
        autovacuum_cycles,
        analyze_cycles,
        autoanalyze_cycles,
        toast_vacuum_cycles,
        dead_growth_per_hour,
        dead_trigger_reached_points: samples
            .iter()
            .filter(|sample| sample.dead_trigger_reached)
            .count(),
        dead_trigger_reached_span_seconds,
        insert_trigger_reached_points: samples
            .iter()
            .filter(|sample| sample.insert_trigger_reached)
            .count(),
        insert_trigger_reached_span_seconds,
        analyze_trigger_reached_points: samples
            .iter()
            .filter(|sample| sample.analyze_trigger_reached)
            .count(),
        analyze_trigger_reached_span_seconds,
        toast_dead_trigger_reached_points: samples
            .iter()
            .filter(|sample| sample.toast_dead_trigger_reached == Some(true))
            .count(),
        toast_insert_trigger_reached_points: samples
            .iter()
            .filter(|sample| sample.toast_insert_trigger_reached == Some(true))
            .count(),
        toast_trigger_reached_span_seconds,
    }
}

fn trailing_reached_span_seconds<F>(
    samples: &[AutovacuumMaintenanceHistorySample],
    reached: F,
) -> Option<f64>
where
    F: Fn(&AutovacuumMaintenanceHistorySample) -> bool,
{
    let newest = samples.last()?;
    if !reached(newest) {
        return None;
    }

    // Never bridge a long gap between observations: a maintenance cycle may
    // have completed between snapshots without being visible at either end.
    let mut oldest_ms = newest.collection_started_at_ms;
    let mut previous_ms = newest.collection_started_at_ms;
    for sample in samples.iter().rev().skip(1) {
        let gap = (previous_ms - sample.collection_started_at_ms) as f64 / 1000.0;
        if !reached(sample) || !(0.0..=MAINTENANCE_MAX_OBSERVATION_GAP_SECONDS).contains(&gap) {
            break;
        }
        oldest_ms = sample.collection_started_at_ms;
        previous_ms = oldest_ms;
    }
    Some((newest.collection_started_at_ms - oldest_ms).max(0) as f64 / 1000.0)
}

fn parse_autovacuum_naptime_seconds(value: &str) -> Option<f64> {
    let value = value.trim().to_ascii_lowercase();
    let split = value.find(|ch: char| !ch.is_ascii_digit() && ch != '.')?;
    let amount = value[..split].parse::<f64>().ok()?;
    let unit = value[split..].trim();
    let multiplier = match unit {
        "ms" => 0.001,
        "s" | "sec" | "second" | "seconds" => 1.0,
        "min" | "mins" | "minute" | "minutes" => 60.0,
        "h" | "hr" | "hour" | "hours" => 3600.0,
        _ => return None,
    };
    (amount.is_finite() && amount > 0.0).then_some(amount * multiplier)
}

fn eligibility_is_sustained(span_seconds: Option<f64>, grace_seconds: Option<f64>) -> bool {
    matches!((span_seconds, grace_seconds), (Some(span), Some(grace)) if span >= grace)
}

fn format_eligibility_span(span_seconds: Option<f64>) -> String {
    span_seconds
        .map(|span| format!("{span:.0} с между наблюдениями"))
        .unwrap_or_else(|| "нет сопоставимых наблюдений".into())
}

fn counter_delta<F>(
    oldest: Option<&AutovacuumMaintenanceHistorySample>,
    newest: Option<&AutovacuumMaintenanceHistorySample>,
    continuous: bool,
    value: F,
) -> Option<i64>
where
    F: Fn(&AutovacuumMaintenanceHistorySample) -> i64,
{
    let (Some(oldest), Some(newest)) = (oldest, newest) else {
        return None;
    };
    if !continuous {
        return None;
    }
    monotonic_delta(value(oldest), value(newest))
}

fn monotonic_delta(older: i64, newer: i64) -> Option<i64> {
    newer.checked_sub(older).filter(|delta| *delta >= 0)
}

#[expect(
    clippy::too_many_arguments,
    reason = "existing call contract; refactor separately with callers"
)]
fn maintenance_finding(
    id: &str,
    area: &str,
    status: &str,
    title: &str,
    summary: &str,
    observed: Vec<String>,
    policy_parameters: Vec<String>,
    next_steps: Vec<String>,
) -> AutovacuumMaintenanceFinding {
    AutovacuumMaintenanceFinding {
        id: id.into(),
        area: area.into(),
        status: status.into(),
        title: title.into(),
        summary: summary.into(),
        observed,
        policy_parameters,
        next_steps,
    }
}

fn maintenance_status_rank(status: &str) -> u8 {
    match status {
        "investigate" => 6,
        "candidate" => 5,
        "review" => 4,
        "insufficient_quality" => 3,
        "insufficient_history" => 2,
        "observe" => 1,
        "working" => 0,
        _ => 1,
    }
}

fn maintenance_advice_summary(status: &str) -> (&'static str, &'static str) {
    match status {
        "investigate" => (
            "Проверить возможную задержку обслуживания",
            "Порог достигнут в нескольких наблюдениях, но автоматические циклы не обнаружены. Нужно проверить планировщик, блокировки и ресурсы; это ещё не доказательство сбоя.",
        ),
        "candidate" => (
            "Есть основания рассмотреть изменение настроек",
            "По нескольким снимкам видно повторное достижение порога при работающем обслуживании. Перед изменением нужно оценить допустимую частоту и стоимость VACUUM или ANALYZE.",
        ),
        "review" => (
            "Нужно уточнить обстоятельства обслуживания",
            "Нужны дополнительные данные о таблице, запросах и индексах. Пока нет оснований изменять настройки.",
        ),
        "insufficient_quality" => (
            "Данные истории недостаточно надёжны",
            "Счётчики или настройки изменились между наблюдениями. Нужна новая сопоставимая серия снимков.",
        ),
        "insufficient_history" => (
            "Нужно собрать дополнительные снимки",
            "Одного снимка или короткого интервала недостаточно для оценки регулярности VACUUM и ANALYZE.",
        ),
        "working" => (
            "Обслуживание таблицы наблюдается",
            "В истории есть циклы обслуживания, а текущие данные не обосновывают изменение настроек.",
        ),
        _ => (
            "Продолжить наблюдение",
            "Пока недостаточно оснований для изменения настроек обслуживания.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at_ms: i64, insert_trigger_reached: bool) -> AutovacuumMaintenanceHistorySample {
        AutovacuumMaintenanceHistorySample {
            collection_started_at_ms: at_ms,
            stats_reset: None,
            policy: None,
            dead_tuples: 0,
            inserts_since_vacuum: 0,
            modifications_since_analyze: 0,
            tuples_inserted: 0,
            tuples_updated: 0,
            tuples_deleted: 0,
            hot_updates: 0,
            vacuum_count: 0,
            autovacuum_count: 0,
            analyze_count: 0,
            autoanalyze_count: 0,
            dead_trigger_reached: false,
            insert_trigger_reached,
            analyze_trigger_reached: false,
            toast_autovacuum_count: None,
            toast_dead_tuples: None,
            toast_inserts_since_vacuum: None,
            toast_dead_trigger_reached: None,
            toast_insert_trigger_reached: None,
        }
    }

    #[test]
    fn naptime_parsing_is_conservative() {
        assert_eq!(parse_autovacuum_naptime_seconds("1min"), Some(60.0));
        assert_eq!(parse_autovacuum_naptime_seconds(" 120s "), Some(120.0));
        assert_eq!(parse_autovacuum_naptime_seconds("500ms"), Some(0.5));
        assert_eq!(parse_autovacuum_naptime_seconds("unsupported"), None);
        assert_eq!(parse_autovacuum_naptime_seconds("0s"), None);
    }

    #[test]
    fn short_repeated_eligibility_is_not_sustained() {
        let history = vec![sample(0, true), sample(34_000, true)];
        let span = trailing_reached_span_seconds(&history, |sample| sample.insert_trigger_reached);

        assert_eq!(span, Some(34.0));
        assert!(!eligibility_is_sustained(span, Some(120.0)));
    }

    #[test]
    fn eligibility_becomes_sustained_after_grace_window() {
        let history = vec![sample(0, true), sample(121_000, true)];
        let span = trailing_reached_span_seconds(&history, |sample| sample.insert_trigger_reached);

        assert_eq!(span, Some(121.0));
        assert!(eligibility_is_sustained(span, Some(120.0)));
    }

    #[test]
    fn eligibility_streak_resets_after_trigger_clears() {
        let history = vec![
            sample(0, true),
            sample(180_000, true),
            sample(181_000, false),
            sample(300_000, true),
        ];
        let span = trailing_reached_span_seconds(&history, |sample| sample.insert_trigger_reached);

        assert_eq!(span, Some(0.0));
        assert!(!eligibility_is_sustained(span, Some(120.0)));
    }
}
