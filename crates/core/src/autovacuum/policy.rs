use crate::{
    internal_postgres::{self, InternalPostgresRegistry},
    postgres::{ConnectionInput, QueryExecutionPolicy},
};

use super::{
    collector::{query_json, table_detail},
    model::*,
    queries::{table_sql, ANALYZE_PROGRESS_SQL, VACUUM_PROGRESS_SQL},
};

pub async fn simulate_policy(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    relation_oid: u32,
    target_dead_tuples: i64,
) -> Result<AutovacuumPolicySimulation, String> {
    if target_dead_tuples <= 0 {
        return Err(crate::i18n::text(
            "Целевое количество мёртвых строк должно быть больше нуля",
            "The target dead tuple count must be greater than zero",
        )
        .into());
    }

    let detail = table_detail(internal_postgres, connection_id, input, relation_oid).await?;

    let table = detail.table;

    let mut notes =
        vec![
            "Расчёт показывает порог срабатывания по мёртвым строкам; он не учитывает нагрузку дисковой подсистемы, блокировки и фактическую продолжительность VACUUM."
                .to_owned(),
            "Перед изменением настроек на рабочем сервере желательно наблюдать естественный цикл Autovacuum и заранее подготовить откат."
                .to_owned(),
        ];

    // Table detail suppresses fixed candidates if track_counts is disabled or
    // unavailable. Do not reintroduce an unverified numeric policy here.
    let (candidate_scale_factor, resulting_dead_tuple_trigger) = if detail
        .fixed_scale_candidates
        .is_empty()
    {
        notes.push(crate::i18n::text("Нет подтверждённых данных для симуляции: проверьте track_counts и оценку строк. Изменения не предлагаются.", "There is insufficient verified evidence for simulation: check track_counts and the row estimate. No changes are proposed.").into());
        (None, None)
    } else if let Some(projection) =
        policy_projection(&table, "autovacuum_vacuum_scale_factor", false)
    {
        if target_dead_tuples as f64 <= table.effective_vacuum_threshold {
            notes.push(
                "Желаемый порог не выше фиксированного: изменить его одним scale factor нельзя."
                    .into(),
            );
            (None, None)
        } else if projection
            .max_threshold
            .is_some_and(|cap| target_dead_tuples > cap.saturating_add(1))
        {
            notes.push(crate::i18n::text("PostgreSQL 18 ограничивает VACUUM порог параметром autovacuum_vacuum_max_threshold: указанная цель недостижима изменением одного scale factor.", "PostgreSQL 18 caps the VACUUM threshold via autovacuum_vacuum_max_threshold: the requested target cannot be reached by changing only the scale factor.").into());
            (None, None)
        } else {
            let scale = ((target_dead_tuples as f64 - 0.5) - table.effective_vacuum_threshold)
                / projection.effective_rows;
            let scale = scale.max(0.0);
            let trigger = project_trigger(table.effective_vacuum_threshold, scale, projection);
            if !scale.is_finite() || scale > 100.0 || trigger.is_none() {
                notes.push(crate::i18n::text("Не удалось получить допустимый коэффициент и достоверный порог. Проверьте статистику и конфигурацию таблицы.", "Could not calculate a valid scale factor and reliable threshold. Check table statistics and configuration.").into());
                (None, None)
            } else {
                notes.push(crate::i18n::text("Это расчёт по оценке reltuples на момент снимка; итоговый порог требуется проверить после изменения настроек.", "This calculation uses the reltuples estimate at snapshot time; verify the resulting threshold after any settings change.").into());
                (Some(scale), trigger)
            }
        }
    } else {
        notes.push("Для этой версии PostgreSQL недостаёт данных о reltuples или правилах порога. Соберите новый снимок.".into());
        (None, None)
    };

    Ok(AutovacuumPolicySimulation {
        relation_oid,
        relation_name: format!("{}.{}", table.schema_name, table.relation_name,),
        estimated_rows: table.estimated_rows,
        current_threshold: table.effective_vacuum_threshold,
        current_scale_factor: table.effective_vacuum_scale_factor,
        current_dead_tuple_trigger: table.dead_tuple_trigger,
        target_dead_tuples,
        candidate_scale_factor,
        resulting_dead_tuple_trigger,
        fixed_scale_candidates: detail.fixed_scale_candidates,
        notes,
    })
}

// The same projection is used for candidate generation, simulation and
// post-apply verification; do not model PG18 as a plain reltuples product.
#[derive(Clone, Copy, Debug)]
pub(super) struct PolicyProjection {
    effective_rows: f64,
    max_threshold: Option<i64>,
}

pub(super) fn policy_projection(
    table: &AutovacuumTable,
    parameter: &str,
    toast_parameter: bool,
) -> Option<PolicyProjection> {
    let (version, rows, fraction, max) = if toast_parameter {
        let toast = table.toast.as_ref()?;
        (
            toast.server_version_num,
            toast.reltuples_estimate,
            toast.insert_unfrozen_fraction,
            toast.effective_vacuum_max_threshold,
        )
    } else {
        (
            table.server_version_num,
            table.reltuples_estimate,
            table.insert_unfrozen_fraction,
            table.effective_vacuum_max_threshold,
        )
    };
    // Older SQLite snapshots have no versioned inputs. Fail closed; a fresh
    // snapshot is required before suggesting policy changes.
    if version < 160_000 {
        return None;
    }
    let mut effective_rows = rows?;
    if parameter.ends_with("vacuum_insert_scale_factor") && version >= 180_000 {
        effective_rows *= fraction?.clamp(0.0, 1.0);
    }
    if !effective_rows.is_finite() || effective_rows <= 0.0 {
        return None;
    }
    Some(PolicyProjection {
        effective_rows,
        max_threshold: if parameter.ends_with("vacuum_scale_factor") && version >= 180_000 {
            max
        } else {
            None
        },
    })
}

pub(super) fn project_trigger(fixed: f64, scale: f64, projection: PolicyProjection) -> Option<i64> {
    if !fixed.is_finite() || !scale.is_finite() || scale < 0.0 || scale > 100.0 {
        return None;
    }
    let mut raw = fixed + scale * projection.effective_rows;
    if let Some(cap) = projection.max_threshold {
        raw = raw.min(cap as f64);
    }
    if !raw.is_finite() || raw < 0.0 || raw >= i64::MAX as f64 {
        return None;
    }
    Some(raw.floor() as i64 + 1)
}

pub(super) fn fixed_scale_candidates(table: &AutovacuumTable) -> Vec<AutovacuumScaleCandidate> {
    let Some(projection) = policy_projection(table, "autovacuum_vacuum_scale_factor", false) else {
        return Vec::new();
    };
    [0.20, 0.10, 0.05, 0.03, 0.02, 0.01]
        .into_iter()
        .filter_map(|scale_factor| {
            project_trigger(table.effective_vacuum_threshold, scale_factor, projection).map(
                |resulting_dead_tuple_trigger| AutovacuumScaleCandidate {
                    scale_factor,
                    resulting_dead_tuple_trigger,
                },
            )
        })
        .collect()
}

const POLICY_RATE_MIN_SPAN_SECONDS: f64 = 1_800.0;
const POLICY_SCENARIOS: [(&str, f64); 2] = [("conservative", 0.75), ("earlier", 0.50)];

#[derive(Debug)]
struct PolicyRateEstimate {
    value_per_hour: f64,
    observed_seconds: f64,
    segments: usize,
}

pub(crate) fn build_policy_candidates(
    table: &AutovacuumTable,
    history: &[AutovacuumMaintenanceHistorySample],
    advice: &AutovacuumMaintenanceAdvice,
    track_counts: bool,
    server_version_num: i32,
) -> AutovacuumPolicyCandidates {
    let relation_name = format!("{}.{}", table.schema_name, table.relation_name);
    let mut candidates = Vec::new();
    let mut limitations = vec![
        "Кандидаты изменяют только scale_factor. Базовый порог, ограничения стоимости и количество рабочих процессов не изменяются."
            .to_owned(),
        "Сценарии 75% и 50% от текущего расчётного порога — варианты для изучения, а не готовые настройки рабочего сервера."
            .to_owned(),
        "Предполагаемый интервал между обслуживаниями оценивается по изменениям счётчиков. Он не учитывает задержку планировщика, занятость workers, блокировки, I/O и длительность VACUUM/ANALYZE."
            .to_owned(),
    ];

    if !track_counts {
        limitations.push("track_counts отключён: нельзя строить кандидаты по недостоверной статистике PostgreSQL.".into());
    } else if table.server_version_num != server_version_num
        || (server_version_num >= 180_000
            && (table.reltuples_estimate.is_none() || table.insert_unfrozen_fraction.is_none()))
    {
        limitations.push("Снимок не содержит версионных данных для точного расчёта порога. Соберите новый снимок Autovacuum.".into());
    } else if !advice.stats_period_continuous {
        limitations.push(
            "stats period не непрерывен: кандидаты не рассчитываются по несопоставимым накопительным счётчикам."
                .into(),
        );
    } else {
        if advice_has_candidate(advice, "vacuum") {
            match estimate_rate(
                history,
                |sample| Some(sample.dead_tuples),
                |sample| Some(sample.vacuum_count.saturating_add(sample.autovacuum_count)),
            ) {
                Some(rate) => push_scale_scenarios(
                    &mut candidates,
                    table,
                    "vacuum",
                    "autovacuum_vacuum_scale_factor",
                    table.effective_vacuum_threshold,
                    table.effective_vacuum_scale_factor,
                    table.dead_tuple_trigger,
                    table.estimated_rows,
                    &relation_name,
                    false,
                    policy_source(table.reloptions.as_deref(), "autovacuum_vacuum_scale_factor", "table"),
                    &rate,
                ),
                None => limitations.push(
                    "Advisor обнаружил повод проверить VACUUM, но нет минимум 30 минут сопоставимых наблюдений роста мёртвых строк без цикла VACUUM. Коэффициент не рассчитан."
                        .into(),
                ),
            }
        }

        if advice_has_candidate(advice, "insert") {
            if let Some(current_trigger) = table.insert_trigger {
                match estimate_rate(
                    history,
                    |sample| Some(sample.inserts_since_vacuum),
                    |sample| Some(sample.vacuum_count.saturating_add(sample.autovacuum_count)),
                ) {
                    Some(rate) => push_scale_scenarios(
                        &mut candidates,
                        table,
                        "insert",
                        "autovacuum_vacuum_insert_scale_factor",
                        table.effective_insert_threshold,
                        table.effective_insert_scale_factor,
                        current_trigger,
                        table.estimated_rows,
                        &relation_name,
                        false,
                        policy_source(table.reloptions.as_deref(), "autovacuum_vacuum_insert_scale_factor", "table"),
                        &rate,
                    ),
                    None => limitations.push(
                        "INSERT candidate есть на уровне Advisor, но history не содержит минимум 30 минут comparable inserts_since_vacuum growth без VACUUM cycle."
                            .into(),
                    ),
                }
            }
        }

        if advice_has_candidate(advice, "analyze") {
            match estimate_rate(
                history,
                |sample| Some(sample.modifications_since_analyze),
                |sample| Some(sample.analyze_count.saturating_add(sample.autoanalyze_count)),
            ) {
                Some(rate) => push_scale_scenarios(
                    &mut candidates,
                    table,
                    "analyze",
                    "autovacuum_analyze_scale_factor",
                    table.effective_analyze_threshold,
                    table.effective_analyze_scale_factor,
                    table.analyze_trigger,
                    table.estimated_rows,
                    &relation_name,
                    false,
                    policy_source(table.reloptions.as_deref(), "autovacuum_analyze_scale_factor", "table"),
                    &rate,
                ),
                None => limitations.push(
                    "Advisor обнаружил повод проверить ANALYZE, но нет минимум 30 минут сопоставимых наблюдений изменений без цикла ANALYZE."
                        .into(),
                ),
            }
        }

        if advice_has_candidate(advice, "toast") {
            limitations.push(
                "История TOAST содержит счётчик автоматических VACUUM, но не отдельный счётчик ручных VACUUM. Неучтённое ручное обслуживание снижает достоверность оценки скорости изменений."
                    .into(),
            );
            if let Some(toast) = table.toast.as_ref() {
                if toast.server_version_num != table.server_version_num
                    || toast.reltuples_estimate.is_none()
                {
                    limitations.push("TOAST: в старом снимке нет сведений для расчёта параметров; требуется новый снимок.".into());
                }
                let toast_dead_repeated = history
                    .iter()
                    .filter(|sample| sample.toast_dead_trigger_reached == Some(true))
                    .count()
                    >= 2;
                if toast.dead_trigger_reached && toast_dead_repeated {
                    match estimate_rate(
                        history,
                        |sample| sample.toast_dead_tuples,
                        |sample| sample.toast_autovacuum_count,
                    ) {
                        Some(rate) => push_scale_scenarios(
                            &mut candidates,
                            table,
                            "toast_vacuum",
                            "toast.autovacuum_vacuum_scale_factor",
                            toast.effective_vacuum_threshold,
                            toast.effective_vacuum_scale_factor,
                            toast.dead_tuple_trigger,
                            toast.estimated_rows,
                            &relation_name,
                            true,
                            policy_source(toast.reloptions.as_deref(), "autovacuum_vacuum_scale_factor", "toast"),
                            &rate,
                        ),
                        None => limitations.push(
                            "TOAST dead-tuple pressure наблюдается, но clean TOAST dead growth между autovacuum cycles недостаточен для числового candidate."
                                .into(),
                        ),
                    }
                }

                let toast_insert_repeated = history
                    .iter()
                    .filter(|sample| sample.toast_insert_trigger_reached == Some(true))
                    .count()
                    >= 2;
                if toast.insert_trigger_reached && toast_insert_repeated {
                    if let Some(current_trigger) = toast.insert_trigger {
                        match estimate_rate(
                            history,
                            |sample| sample.toast_inserts_since_vacuum,
                            |sample| sample.toast_autovacuum_count,
                        ) {
                            Some(rate) => push_scale_scenarios(
                                &mut candidates,
                                table,
                                "toast_insert",
                                "toast.autovacuum_vacuum_insert_scale_factor",
                                toast.effective_insert_threshold,
                                toast.effective_insert_scale_factor,
                                current_trigger,
                                toast.estimated_rows,
                                &relation_name,
                                true,
                                policy_source(toast.reloptions.as_deref(), "autovacuum_vacuum_insert_scale_factor", "toast"),
                                &rate,
                            ),
                            None => limitations.push(
                                "TOAST insert pressure наблюдается, но clean inserts_since_vacuum growth между TOAST autovacuum cycles недостаточен для числового candidate."
                                    .into(),
                            ),
                        }
                    }
                }
            }
        }
    }

    let (status, summary) = if candidates.is_empty() {
        if !track_counts
            || table.server_version_num != server_version_num
            || !advice.stats_period_continuous
            || advice
                .findings
                .iter()
                .any(|finding| finding.status == "candidate")
        {
            (
                "insufficient_evidence",
                "Недостаточно достоверной истории для обоснованного выбора коэффициента scale_factor.",
            )
        } else {
            (
                "not_needed",
                "Advisor не обнаружил достаточных оснований для изменения политики по этому снимку.",
            )
        }
    } else {
        (
            "available",
            "Показаны варианты более раннего запуска обслуживания на основании истории. Применение требует отдельного подтверждения и повторной проверки PostgreSQL.",
        )
    };

    AutovacuumPolicyCandidates {
        relation_oid: table.relation_oid,
        relation_name,
        status: status.into(),
        summary: summary.into(),
        candidates,
        limitations,
    }
}

fn advice_has_candidate(advice: &AutovacuumMaintenanceAdvice, area: &str) -> bool {
    advice
        .findings
        .iter()
        .any(|finding| finding.area == area && finding.status == "candidate")
}

#[allow(clippy::too_many_arguments)]
fn push_scale_scenarios(
    candidates: &mut Vec<AutovacuumPolicyCandidate>,
    table: &AutovacuumTable,
    area: &str,
    parameter: &str,
    fixed_threshold: f64,
    current_scale_factor: f64,
    current_trigger: i64,
    estimated_rows: i64,
    target_relation: &str,
    toast_parameter: bool,
    current_source: &str,
    rate: &PolicyRateEstimate,
) {
    if estimated_rows <= 0 || current_trigger <= 0 || rate.value_per_hour <= 0.0 {
        return;
    }

    let Some(projection) = policy_projection(table, parameter, toast_parameter) else {
        return;
    };
    let mut last_trigger = None;
    for (scenario, fraction) in POLICY_SCENARIOS {
        let requested_trigger = ((current_trigger as f64) * fraction).round() as i64;
        if requested_trigger <= 0 || requested_trigger as f64 <= fixed_threshold {
            continue;
        }

        let candidate_scale =
            ((requested_trigger as f64 - 0.5) - fixed_threshold) / projection.effective_rows;
        if !candidate_scale.is_finite()
            || candidate_scale < 0.0
            || candidate_scale + 1e-12 >= current_scale_factor
        {
            continue;
        }
        let Some(candidate_trigger) = project_trigger(fixed_threshold, candidate_scale, projection)
        else {
            continue;
        };
        if candidate_trigger >= current_trigger || last_trigger == Some(candidate_trigger) {
            continue;
        }
        last_trigger = Some(candidate_trigger);

        let sql_preview = policy_sql_preview(
            &table.schema_name,
            &table.relation_name,
            parameter,
            candidate_scale,
        );
        let target_relation = if toast_parameter {
            format!("{target_relation} · TOAST")
        } else {
            target_relation.to_owned()
        };

        candidates.push(AutovacuumPolicyCandidate {
            id: format!("{area}-{scenario}"),
            area: area.into(),
            scenario: scenario.into(),
            target_relation,
            parameter: parameter.into(),
            current_scale_factor,
            current_source: current_source.into(),
            candidate_scale_factor: candidate_scale,
            fixed_threshold,
            current_trigger,
            candidate_trigger,
            observed_rate_per_hour: rate.value_per_hour,
            current_volume_hours: current_trigger as f64 / rate.value_per_hour,
            candidate_volume_hours: candidate_trigger as f64 / rate.value_per_hour,
            sql_preview,
            rationale: vec![
                format!(
                    "Целевой порог сценария — {:.0}% от текущего; постоянная часть порога остаётся {:.0}.",
                    fraction * 100.0,
                    fixed_threshold,
                ),
                format!(
                    "Наблюдаемая скорость ≈ {:.0}/ч по {} сопоставимым интервалам, {:.1} ч без соответствующего цикла обслуживания.",
                    rate.value_per_hour,
                    rate.segments,
                    rate.observed_seconds / 3600.0,
                ),
                "Ожидаемый интервал рассчитан по объёму изменений; он не учитывает задержку планировщика и продолжительность обслуживания."
                    .into(),
            ],
        });
    }
}

fn policy_source<'a>(
    reloptions: Option<&'a [String]>,
    parameter: &str,
    local_source: &'a str,
) -> &'a str {
    let overridden = reloptions
        .unwrap_or(&[])
        .iter()
        .filter_map(|option| option.split_once('='))
        .any(|(name, _)| name == parameter);

    if overridden {
        local_source
    } else {
        "global"
    }
}

fn estimate_rate<Metric, Cycle>(
    history: &[AutovacuumMaintenanceHistorySample],
    metric: Metric,
    cycle: Cycle,
) -> Option<PolicyRateEstimate>
where
    Metric: Fn(&AutovacuumMaintenanceHistorySample) -> Option<i64>,
    Cycle: Fn(&AutovacuumMaintenanceHistorySample) -> Option<i64>,
{
    let samples = latest_continuous_maintenance_samples(history);

    let mut total_delta = 0_i64;
    let mut observed_seconds = 0.0_f64;
    let mut segments = 0_usize;

    for pair in samples.windows(2) {
        let older = &pair[0];
        let newer = &pair[1];
        if !maintenance_stats_pair_continuous(older, newer) {
            continue;
        }
        let elapsed_seconds =
            (newer.collection_started_at_ms - older.collection_started_at_ms) as f64 / 1000.0;
        if elapsed_seconds <= 0.0 {
            continue;
        }
        let (Some(older_cycle), Some(newer_cycle)) = (cycle(older), cycle(newer)) else {
            continue;
        };
        if newer_cycle != older_cycle {
            continue;
        }
        let (Some(older_value), Some(newer_value)) = (metric(older), metric(newer)) else {
            continue;
        };
        let Some(delta) = newer_value
            .checked_sub(older_value)
            .filter(|delta| *delta >= 0)
        else {
            continue;
        };

        total_delta = total_delta.saturating_add(delta);
        observed_seconds += elapsed_seconds;
        segments += 1;
    }

    if segments == 0 || observed_seconds < POLICY_RATE_MIN_SPAN_SECONDS || total_delta <= 0 {
        return None;
    }

    Some(PolicyRateEstimate {
        value_per_hour: total_delta as f64 / (observed_seconds / 3600.0),
        observed_seconds,
        segments,
    })
}

const APPLY_SCALE_EPSILON: f64 = 1e-8;

struct AutovacuumPolicyParameterSpec {
    base_parameter: &'static str,
    fixed_parameter: &'static str,
    toast: bool,
    local_source: &'static str,
}

fn policy_parameter_spec(parameter: &str) -> Option<AutovacuumPolicyParameterSpec> {
    match parameter {
        "autovacuum_vacuum_scale_factor" => Some(AutovacuumPolicyParameterSpec {
            base_parameter: "autovacuum_vacuum_scale_factor",
            fixed_parameter: "autovacuum_vacuum_threshold",
            toast: false,
            local_source: "table",
        }),
        "autovacuum_vacuum_insert_scale_factor" => Some(AutovacuumPolicyParameterSpec {
            base_parameter: "autovacuum_vacuum_insert_scale_factor",
            fixed_parameter: "autovacuum_vacuum_insert_threshold",
            toast: false,
            local_source: "table",
        }),
        "autovacuum_analyze_scale_factor" => Some(AutovacuumPolicyParameterSpec {
            base_parameter: "autovacuum_analyze_scale_factor",
            fixed_parameter: "autovacuum_analyze_threshold",
            toast: false,
            local_source: "table",
        }),
        "toast.autovacuum_vacuum_scale_factor" => Some(AutovacuumPolicyParameterSpec {
            base_parameter: "autovacuum_vacuum_scale_factor",
            fixed_parameter: "autovacuum_vacuum_threshold",
            toast: true,
            local_source: "toast",
        }),
        "toast.autovacuum_vacuum_insert_scale_factor" => Some(AutovacuumPolicyParameterSpec {
            base_parameter: "autovacuum_vacuum_insert_scale_factor",
            fixed_parameter: "autovacuum_vacuum_insert_threshold",
            toast: true,
            local_source: "toast",
        }),
        _ => None,
    }
}

fn observed_policy_state(
    table: &AutovacuumTable,
    candidate: &AutovacuumPolicyCandidate,
) -> Result<AutovacuumPolicyApplyObservedState, String> {
    let spec = policy_parameter_spec(&candidate.parameter)
        .ok_or("Policy candidate содержит неподдерживаемый Autovacuum parameter")?;

    if spec.toast {
        let toast = table
            .toast
            .as_ref()
            .ok_or("TOAST candidate больше не имеет TOAST relation в текущем состоянии")?;
        let (scale_factor, fixed_threshold, absolute_trigger) = match spec.base_parameter {
            "autovacuum_vacuum_scale_factor" => (
                toast.effective_vacuum_scale_factor,
                toast.effective_vacuum_threshold,
                toast.dead_tuple_trigger,
            ),
            "autovacuum_vacuum_insert_scale_factor" => (
                toast.effective_insert_scale_factor,
                toast.effective_insert_threshold,
                toast
                    .insert_trigger
                    .ok_or("TOAST insert trigger сейчас disabled")?,
            ),
            _ => return Err("Неподдерживаемый TOAST policy parameter".into()),
        };

        Ok(AutovacuumPolicyApplyObservedState {
            relation_oid: toast.relation_oid,
            subject_relation: format!("{}.{}", toast.schema_name, toast.relation_name),
            parameter: candidate.parameter.clone(),
            scale_factor,
            source: policy_source(
                toast.reloptions.as_deref(),
                spec.base_parameter,
                spec.local_source,
            )
            .into(),
            fixed_threshold,
            absolute_trigger,
            estimated_rows: toast.estimated_rows,
        })
    } else {
        let (scale_factor, fixed_threshold, absolute_trigger) = match spec.base_parameter {
            "autovacuum_vacuum_scale_factor" => (
                table.effective_vacuum_scale_factor,
                table.effective_vacuum_threshold,
                table.dead_tuple_trigger,
            ),
            "autovacuum_vacuum_insert_scale_factor" => (
                table.effective_insert_scale_factor,
                table.effective_insert_threshold,
                table
                    .insert_trigger
                    .ok_or("Insert trigger сейчас disabled")?,
            ),
            "autovacuum_analyze_scale_factor" => (
                table.effective_analyze_scale_factor,
                table.effective_analyze_threshold,
                table.analyze_trigger,
            ),
            _ => return Err("Неподдерживаемый table policy parameter".into()),
        };

        Ok(AutovacuumPolicyApplyObservedState {
            relation_oid: table.relation_oid,
            subject_relation: format!("{}.{}", table.schema_name, table.relation_name),
            parameter: candidate.parameter.clone(),
            scale_factor,
            source: policy_source(
                table.reloptions.as_deref(),
                spec.base_parameter,
                spec.local_source,
            )
            .into(),
            fixed_threshold,
            absolute_trigger,
            estimated_rows: table.estimated_rows,
        })
    }
}

fn same_relation_identity(anchor: &AutovacuumTable, live: &AutovacuumTable) -> bool {
    anchor.relation_oid == live.relation_oid
        && anchor.schema_name == live.schema_name
        && anchor.relation_name == live.relation_name
        && anchor.relation_type == live.relation_type
}

fn same_toast_identity(anchor: &AutovacuumTable, live: &AutovacuumTable) -> bool {
    match (anchor.toast.as_ref(), live.toast.as_ref()) {
        (Some(anchor), Some(live)) => {
            anchor.relation_oid == live.relation_oid
                && anchor.schema_name == live.schema_name
                && anchor.relation_name == live.relation_name
        }
        (None, None) => true,
        _ => false,
    }
}

fn float_matches(left: f64, right: f64) -> bool {
    (left - right).abs() <= APPLY_SCALE_EPSILON
}

pub(super) fn validate_policy_apply_guard(
    anchor: &AutovacuumTable,
    live: &AutovacuumTable,
    candidate: &AutovacuumPolicyCandidate,
) -> Result<AutovacuumPolicyApplyObservedState, String> {
    if !same_relation_identity(anchor, live) {
        return Err(
            "Relation изменилась после snapshot: OID/name/type больше не совпадают. Соберите новый Autovacuum snapshot перед apply."
                .into(),
        );
    }

    let spec = policy_parameter_spec(&candidate.parameter)
        .ok_or("Policy candidate содержит неподдерживаемый Autovacuum parameter")?;
    if spec.toast && !same_toast_identity(anchor, live) {
        return Err(
            "TOAST relation изменилась после snapshot. Соберите новый Autovacuum snapshot перед apply."
                .into(),
        );
    }

    let anchor_state = observed_policy_state(anchor, candidate)?;
    if !float_matches(anchor_state.scale_factor, candidate.current_scale_factor)
        || anchor_state.source != candidate.current_source
        || !float_matches(anchor_state.fixed_threshold, candidate.fixed_threshold)
        || anchor_state.absolute_trigger != candidate.current_trigger
    {
        return Err(
            "Policy candidate больше не соответствует anchor snapshot. Пересчитайте Concrete Policy Candidates."
                .into(),
        );
    }

    let projection = policy_projection(anchor, &candidate.parameter, spec.toast)
        .ok_or("В снимке отсутствуют необходимые данные для проверки порога. Соберите новый снимок перед Apply.")?;
    if project_trigger(
        candidate.fixed_threshold,
        candidate.candidate_scale_factor,
        projection,
    ) != Some(candidate.candidate_trigger)
    {
        return Err(
            "Кандидат содержит неверный расчётный порог. Пересчитайте его по новому снимку.".into(),
        );
    }

    let expected_sql = policy_sql_preview(
        &anchor.schema_name,
        &anchor.relation_name,
        &candidate.parameter,
        candidate.candidate_scale_factor,
    );
    if expected_sql != candidate.sql_preview {
        return Err(
            "SQL preview candidate не соответствует anchor relation. Пересчитайте Concrete Policy Candidates."
                .into(),
        );
    }

    let live_state = observed_policy_state(live, candidate)?;
    if !float_matches(live_state.scale_factor, candidate.current_scale_factor)
        || live_state.source != candidate.current_source
        || !float_matches(live_state.fixed_threshold, candidate.fixed_threshold)
        || live_state.absolute_trigger != candidate.current_trigger
    {
        return Err(format!(
            "Autovacuum policy изменилась после snapshot: ожидались scale={} ({}), fixed threshold {:.6}, trigger {}; сейчас scale={} ({}), fixed threshold {:.6}, trigger {}. Соберите новый snapshot и пересчитайте candidate.",
            format_scale(candidate.current_scale_factor),
            candidate.current_source,
            candidate.fixed_threshold,
            candidate.current_trigger,
            format_scale(live_state.scale_factor),
            live_state.source,
            live_state.fixed_threshold,
            live_state.absolute_trigger,
        ));
    }

    if !live.autovacuum_enabled || !live.autovacuum_applicable {
        return Err(
            "Regular autovacuum для relation больше не применим или выключен. Перед policy apply нужен новый snapshot."
                .into(),
        );
    }
    if spec.toast
        && !live
            .toast
            .as_ref()
            .map(|toast| toast.autovacuum_enabled)
            .unwrap_or(false)
    {
        return Err(
            "Regular autovacuum для TOAST relation больше не включён. Перед policy apply нужен новый snapshot."
                .into(),
        );
    }

    Ok(live_state)
}

async fn live_table(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    relation_oid: u32,
) -> Result<AutovacuumTable, String> {
    let sql = table_sql(relation_oid)?;
    let tables =
        query_json::<Vec<AutovacuumTable>>(internal_postgres, connection_id, input, &sql).await?;

    tables
        .into_iter()
        .find(|table| table.relation_oid == relation_oid)
        .ok_or_else(|| format!("Relation OID {relation_oid} больше не найдена среди user tables"))
}

async fn ensure_no_active_maintenance(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    subject_oid: u32,
) -> Result<(), String> {
    let vacuums = query_json::<Vec<AutovacuumVacuumProgress>>(
        internal_postgres,
        connection_id,
        input,
        VACUUM_PROGRESS_SQL,
    )
    .await?;
    if let Some(vacuum) = vacuums.iter().find(|row| row.relation_oid == subject_oid) {
        return Err(format!(
            "Для policy target сейчас виден VACUUM PID {} · {}. Дождитесь завершения и соберите новый snapshot.",
            vacuum.pid, vacuum.phase,
        ));
    }

    let analyzes = query_json::<Vec<AutovacuumAnalyzeProgress>>(
        internal_postgres,
        connection_id,
        input,
        ANALYZE_PROGRESS_SQL,
    )
    .await?;
    if let Some(analyze) = analyzes.iter().find(|row| row.relation_oid == subject_oid) {
        return Err(format!(
            "Для policy target сейчас виден ANALYZE PID {} · {}. Дождитесь завершения и соберите новый snapshot.",
            analyze.pid, analyze.phase,
        ));
    }

    Ok(())
}

fn quote_sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_float(value: f64) -> String {
    format!("{value:.17}")
}

pub(super) fn guarded_policy_apply_sql(
    database_oid: u32,
    anchor: &AutovacuumTable,
    candidate: &AutovacuumPolicyCandidate,
) -> Result<String, String> {
    let alter_sql = policy_sql_preview(
        &anchor.schema_name,
        &anchor.relation_name,
        &candidate.parameter,
        candidate.candidate_scale_factor,
    );
    guarded_policy_change_sql(database_oid, anchor, candidate, &alter_sql)
}

// Rollback is deliberately a guarded SQL script to execute explicitly, never
// an automatic retry. The source of the old setting determines SET vs RESET.
pub(super) fn guarded_policy_rollback_sql(
    database_oid: u32,
    anchor: &AutovacuumTable,
    candidate: &AutovacuumPolicyCandidate,
    before: &AutovacuumPolicyApplyObservedState,
    after: &AutovacuumPolicyApplyObservedState,
) -> Result<String, String> {
    let spec =
        policy_parameter_spec(&candidate.parameter).ok_or("Неподдерживаемый параметр отката")?;
    if after.source != spec.local_source {
        return Err("После применения настройка не имеет ожидаемый локальный источник; откат не сформирован".into());
    }
    let mut inverse = candidate.clone();
    inverse.current_scale_factor = after.scale_factor;
    inverse.current_source = after.source.clone();
    inverse.fixed_threshold = after.fixed_threshold;
    inverse.current_trigger = after.absolute_trigger;
    let target = format!(
        "{}.{}",
        quote_ident(&anchor.schema_name),
        quote_ident(&anchor.relation_name),
    );
    let alter = if before.source == "global" {
        // RESET must restore the original *effective* value. If another DBA
        // changed the global setting since Apply, abort rather than inheriting
        // an unexpected new value.
        format!(
            "IF abs(current_setting({})::numeric - {}::numeric) > 0.000000001::numeric THEN\n        RAISE EXCEPTION 'pgXray rollback guard failed: inherited global value changed';\n    END IF;\n    ALTER TABLE {target} RESET ({});",
            quote_sql_literal(spec.base_parameter),
            sql_float(before.scale_factor),
            candidate.parameter,
        )
    } else if before.source == spec.local_source {
        policy_sql_preview(
            &anchor.schema_name,
            &anchor.relation_name,
            &candidate.parameter,
            before.scale_factor,
        )
    } else {
        return Err(
            "Исходный источник настройки неизвестен; автоматический SQL отката не сформирован"
                .into(),
        );
    };
    guarded_policy_change_sql(database_oid, anchor, &inverse, &alter)
}

// Shared guard for both forward change and explicit reverse change.
fn guarded_policy_change_sql(
    database_oid: u32,
    anchor: &AutovacuumTable,
    candidate: &AutovacuumPolicyCandidate,
    alter: &str,
) -> Result<String, String> {
    let spec = policy_parameter_spec(&candidate.parameter)
        .ok_or("Policy candidate содержит неподдерживаемый Autovacuum parameter")?;
    let anchor_state = observed_policy_state(anchor, candidate)?;
    let (subject_schema, subject_name, subject_relkind) = if spec.toast {
        let toast = anchor
            .toast
            .as_ref()
            .ok_or("TOAST candidate не содержит anchor TOAST relation")?;
        (
            toast.schema_name.as_str(),
            toast.relation_name.as_str(),
            "t",
        )
    } else {
        (
            anchor.schema_name.as_str(),
            anchor.relation_name.as_str(),
            "r",
        )
    };
    let subject_oid = anchor_state.relation_oid;
    let target = format!(
        "{}.{}",
        quote_ident(&anchor.schema_name),
        quote_ident(&anchor.relation_name),
    );
    let toast_relation_guard = if spec.toast {
        format!(
            r#"
    IF (SELECT reltoastrelid FROM pg_class WHERE oid = {target_oid}::oid)
       IS DISTINCT FROM {subject_oid}::oid
    THEN
        RAISE EXCEPTION 'pgXray guard failed: TOAST relation changed';
    END IF;
"#,
            target_oid = anchor.relation_oid,
            subject_oid = subject_oid,
        )
    } else {
        String::new()
    };

    // Calculate the first eligible integer count from the live PostgreSQL
    // catalog, including PG18's max VACUUM threshold and unfrozen fraction.
    // This is checked again inside the same guarded ALTER statement.
    let fixed_expr = format!(
        "COALESCE((SELECT option_value::numeric FROM pg_options_to_table(c.reloptions) WHERE option_name = {fixed_parameter} LIMIT 1), current_setting({fixed_parameter})::numeric)",
        fixed_parameter = quote_sql_literal(spec.fixed_parameter),
    );
    let scale_expr = format!(
        "COALESCE((SELECT option_value::numeric FROM pg_options_to_table(c.reloptions) WHERE option_name = {base_parameter} LIMIT 1), current_setting({base_parameter})::numeric)",
        base_parameter = quote_sql_literal(spec.base_parameter),
    );
    // relallfrozen only exists as a pg_class column on PostgreSQL 18+.
    // Access the optional catalog field without referencing it as a SQL column,
    // so the guarded statement also parses on PostgreSQL 16 and 17.
    let relallfrozen = "COALESCE((to_jsonb(c)->>'relallfrozen')::numeric, 0)";
    let unfrozen = format!(
        "CASE WHEN current_setting('server_version_num')::integer >= 180000 AND c.relpages > 0 AND ({relallfrozen}) > 0 THEN 1 - LEAST(({relallfrozen}), c.relpages) / c.relpages ELSE 1 END"
    );
    let raw_threshold = if spec.base_parameter == "autovacuum_vacuum_insert_scale_factor" {
        format!("({fixed_expr} + {scale_expr} * GREATEST(c.reltuples::numeric, 0) * ({unfrozen}))")
    } else {
        format!("({fixed_expr} + {scale_expr} * GREATEST(c.reltuples::numeric, 0))")
    };
    let current_trigger_expression = if spec.base_parameter == "autovacuum_vacuum_scale_factor" {
        let max_expr = "COALESCE((SELECT option_value::numeric FROM pg_options_to_table(c.reloptions) WHERE option_name = 'autovacuum_vacuum_max_threshold' LIMIT 1), COALESCE(NULLIF(current_setting('autovacuum_vacuum_max_threshold', true), '')::numeric, -1))";
        format!("floor(CASE WHEN ({max_expr}) >= 0 THEN LEAST({raw_threshold}, ({max_expr})) ELSE {raw_threshold} END)::bigint + 1")
    } else {
        format!("floor({raw_threshold})::bigint + 1")
    };

    Ok(format!(
        r#"DO $pgxray$
DECLARE
    current_target_oid oid;
    current_schema text;
    current_name text;
    current_relkind text;
    current_scale numeric;
    current_source text;
    current_fixed numeric;
    current_trigger bigint;
BEGIN
    IF current_setting('track_counts') <> 'on' THEN
        RAISE EXCEPTION 'pgXray guard failed: track_counts disabled';
    END IF;

    IF (SELECT oid FROM pg_database WHERE datname = current_database())
       IS DISTINCT FROM {database_oid}::oid
    THEN
        RAISE EXCEPTION 'pgXray guard failed: database identity changed';
    END IF;

    LOCK TABLE ONLY {target} IN SHARE UPDATE EXCLUSIVE MODE;

    SELECT c.oid
    INTO current_target_oid
    FROM pg_class AS c
    JOIN pg_namespace AS n ON n.oid = c.relnamespace
    WHERE n.nspname = {target_schema}
      AND c.relname = {target_name};

    IF current_target_oid IS DISTINCT FROM {target_oid}::oid THEN
        RAISE EXCEPTION 'pgXray guard failed: DDL target identity changed';
    END IF;
{toast_relation_guard}
    SELECT
        n.nspname::text,
        c.relname::text,
        c.relkind::text,
        COALESCE(
            (
                SELECT o.option_value::numeric
                FROM pg_options_to_table(c.reloptions) AS o
                WHERE o.option_name = {base_parameter}
                LIMIT 1
            ),
            current_setting({base_parameter})::numeric
        ),
        CASE
            WHEN EXISTS (
                SELECT 1
                FROM pg_options_to_table(c.reloptions) AS o
                WHERE o.option_name = {base_parameter}
            ) THEN {local_source}
            ELSE 'global'
        END,
        COALESCE(
            (
                SELECT o.option_value::numeric
                FROM pg_options_to_table(c.reloptions) AS o
                WHERE o.option_name = {fixed_parameter}
                LIMIT 1
            ),
            current_setting({fixed_parameter})::numeric
        ),
        {current_trigger_expression}
    INTO
        current_schema,
        current_name,
        current_relkind,
        current_scale,
        current_source,
        current_fixed,
        current_trigger
    FROM pg_class AS c
    JOIN pg_namespace AS n ON n.oid = c.relnamespace
    WHERE c.oid = {subject_oid}::oid;

    IF current_schema IS DISTINCT FROM {subject_schema}
       OR current_name IS DISTINCT FROM {subject_name}
       OR current_relkind IS DISTINCT FROM {subject_relkind}
    THEN
        RAISE EXCEPTION 'pgXray guard failed: policy subject identity changed';
    END IF;

    IF abs(current_scale - {expected_scale}::numeric) > 0.000000001::numeric
       OR current_source IS DISTINCT FROM {expected_source}
       OR abs(current_fixed - {expected_fixed}::numeric) > 0.000000001::numeric
       OR current_trigger IS DISTINCT FROM {expected_trigger}::bigint
    THEN
        RAISE EXCEPTION 'pgXray guard failed: autovacuum policy changed';
    END IF;

    IF EXISTS (
        SELECT 1 FROM pg_stat_progress_vacuum WHERE relid = {subject_oid}::oid
    ) THEN
        RAISE EXCEPTION 'pgXray guard failed: VACUUM is active';
    END IF;

    IF EXISTS (
        SELECT 1 FROM pg_stat_progress_analyze WHERE relid = {subject_oid}::oid
    ) THEN
        RAISE EXCEPTION 'pgXray guard failed: ANALYZE is active';
    END IF;

    {alter}
END
$pgxray$;"#,
        database_oid = database_oid,
        target = target,
        target_schema = quote_sql_literal(&anchor.schema_name),
        target_name = quote_sql_literal(&anchor.relation_name),
        target_oid = anchor.relation_oid,
        toast_relation_guard = toast_relation_guard,
        base_parameter = quote_sql_literal(spec.base_parameter),
        fixed_parameter = quote_sql_literal(spec.fixed_parameter),
        local_source = quote_sql_literal(spec.local_source),
        subject_oid = subject_oid,
        subject_schema = quote_sql_literal(subject_schema),
        subject_name = quote_sql_literal(subject_name),
        subject_relkind = quote_sql_literal(subject_relkind),
        expected_scale = sql_float(candidate.current_scale_factor),
        expected_source = quote_sql_literal(&candidate.current_source),
        expected_fixed = sql_float(candidate.fixed_threshold),
        expected_trigger = candidate.current_trigger,
        current_trigger_expression = current_trigger_expression,
        alter = alter,
    ))
}

pub async fn apply_policy_candidate(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    database_oid: u32,
    anchor: &AutovacuumTable,
    candidate: &AutovacuumPolicyCandidate,
) -> Result<AutovacuumPolicyApplyVerification, String> {
    let current_database_oid = query_json::<u32>(
        internal_postgres,
        connection_id,
        input,
        "SELECT to_json(oid::bigint)::text FROM pg_database WHERE datname = current_database();",
    )
    .await?;
    if current_database_oid != database_oid {
        return Err(format!(
            "Database identity изменилась после snapshot: ожидался OID {database_oid}, сейчас {current_database_oid}. Соберите новый Autovacuum snapshot перед apply."
        ));
    }

    let track_counts = query_json::<bool>(
        internal_postgres,
        connection_id,
        input,
        "SELECT to_json(current_setting('track_counts') = 'on')::text;",
    )
    .await?;
    if !track_counts {
        return Err("track_counts отключён: применение Autovacuum policy запрещено до получения достоверной статистики.".into());
    }

    let live_before =
        live_table(internal_postgres, connection_id, input, anchor.relation_oid).await?;
    let before = validate_policy_apply_guard(anchor, &live_before, candidate)?;
    ensure_no_active_maintenance(internal_postgres, connection_id, input, before.relation_oid)
        .await?;

    let guarded_sql = guarded_policy_apply_sql(database_oid, anchor, candidate)?;
    internal_postgres::run_action_once(
        input,
        &guarded_sql,
        QueryExecutionPolicy::explicit_heavy_query(5_000, "pgxray-autovacuum-policy"),
    )
    .await?;

    let live_after = live_table(internal_postgres, connection_id, input, anchor.relation_oid)
        .await
        .map_err(|error| {
            format!(
                "ALTER завершён, но post-change Autovacuum verification не выполнен: {error}. Перед повторным apply соберите новый snapshot."
            )
        })?;
    if !same_relation_identity(anchor, &live_after)
        || (policy_parameter_spec(&candidate.parameter)
            .map(|spec| spec.toast)
            .unwrap_or(false)
            && !same_toast_identity(anchor, &live_after))
    {
        return Err(
            "ALTER завершён, но relation identity изменилась до post-change verification. Перед дальнейшими действиями соберите новый snapshot."
                .into(),
        );
    }

    let after = observed_policy_state(&live_after, candidate)?;
    let expected_source = policy_parameter_spec(&candidate.parameter)
        .map(|spec| spec.local_source)
        .ok_or("ALTER завершён, но candidate parameter больше не поддерживается")?;
    if !float_matches(after.scale_factor, candidate.candidate_scale_factor)
        || after.source != expected_source
    {
        return Err(format!(
            "ALTER завершён, но verification ожидал scale={} ({expected_source}), а получил scale={} ({}). Перед повторным apply соберите новый snapshot.",
            format_scale(candidate.candidate_scale_factor),
            format_scale(after.scale_factor),
            after.source,
        ));
    }

    if after.absolute_trigger != candidate.candidate_trigger {
        return Err(format!(
            "ALTER выполнен и scale factor подтверждён, но фактический порог {} не совпадает с расчётным {}. Возможно, изменилась оценка reltuples/доля незамороженных страниц. НЕ повторяйте Apply; соберите новый снимок.",
            after.absolute_trigger, candidate.candidate_trigger,
        ));
    }

    // A rollback can be offered only after the forward change was verified.
    // It is never executed automatically; the user must run the guarded SQL
    // explicitly and collect a new snapshot afterwards.
    let rollback_sql =
        guarded_policy_rollback_sql(database_oid, anchor, candidate, &before, &after).ok();
    let rollback_note = if rollback_sql.is_some() {
        "Откат не выполняется автоматически. Для возврата предыдущей настройки выполните rollback_sql отдельно после проверки текущего состояния; при изменении настройки другим DBA защитный SQL откажет.".to_owned()
    } else {
        "Не удалось сформировать защищённый SQL отката. Не повторяйте Apply; проверьте текущие reloptions и восстановите исходные настройки вручную.".to_owned()
    };

    Ok(AutovacuumPolicyApplyVerification {
        rollback_sql,
        rollback_note,
        candidate_id: candidate.id.clone(),
        parameter: candidate.parameter.clone(),
        target_relation: candidate.target_relation.clone(),
        candidate_scale_factor: candidate.candidate_scale_factor,
        before,
        after,
    })
}

pub(super) fn policy_sql_preview(
    schema_name: &str,
    relation_name: &str,
    parameter: &str,
    value: f64,
) -> String {
    format!(
        "ALTER TABLE {}.{}\nSET ({} = {});",
        quote_ident(schema_name),
        quote_ident(relation_name),
        parameter,
        format_scale(value),
    )
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('\"', "\"\""))
}

fn format_scale(value: f64) -> String {
    let formatted = format!("{value:.8}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.into()
    }
}
