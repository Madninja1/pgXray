use crate::{
    internal_postgres::{self, InternalPostgresRegistry},
    postgres::{ConnectionInput, QueryExecutionPolicy},
};

use super::{collector::query_json, model::*};

const POLICY_MIN_HISTORY_POINTS: usize = 3;
const POLICY_MIN_HISTORY_SPAN_SECONDS: f64 = 1_800.0;
const POLICY_REVIEW_FREEZE_PCT: f64 = 60.0;
const POLICY_FORCE_WINDOW_SECONDS: f64 = 86_400.0;
const POLICY_MIN_FORCE_MARGIN_PCT: f64 = 0.10;
const POLICY_ROUND_XIDS: i64 = 1_000_000;

pub(crate) fn advise_relation_policy(
    overview: &XidOverview,
    relation: &XidRelationAge,
    trigger: Option<&XidAutovacuumTrigger>,
    active_vacuum: Option<&XidVacuum>,
    vacuum_section_available: bool,
    history: &[XidPolicyHistorySample],
) -> XidRelationPolicyAdvice {
    let mut samples = history.to_vec();
    samples.sort_by_key(|sample| sample.collection_started_at_ms);

    let history_points = samples.len();
    let history_span_seconds = match (samples.first(), samples.last()) {
        (Some(oldest), Some(newest))
            if newest.collection_started_at_ms > oldest.collection_started_at_ms =>
        {
            Some(
                (newest.collection_started_at_ms - oldest.collection_started_at_ms) as f64 / 1000.0,
            )
        }
        _ => None,
    };

    let vacuum_segment_start = samples
        .windows(2)
        .rposition(|pair| !vacuum_stats_pair_continuous(&pair[0], &pair[1]))
        .map(|index| index + 1)
        .unwrap_or(0);
    let vacuum_samples = &samples[vacuum_segment_start..];
    let stats_period_continuous = vacuum_samples.len() >= 2;
    let comparable_span_seconds = match (vacuum_samples.first(), vacuum_samples.last()) {
        (Some(oldest), Some(newest))
            if newest.collection_started_at_ms > oldest.collection_started_at_ms =>
        {
            Some(
                (newest.collection_started_at_ms - oldest.collection_started_at_ms) as f64 / 1000.0,
            )
        }
        _ => None,
    };

    let mut limitations = Vec::new();
    if overview.stats_reset.is_none() {
        limitations.push(
            "stats_reset = NULL в pg_stat_database: это не доказывает сброс статистики, но скрытый сброс VACUUM counters исключить нельзя. XID age и freeze thresholds доступны по текущему снимку; исторические VACUUM cycles и числовые Policy Candidates требуют подтверждённой непрерывности. Не вызывайте pg_stat_reset() ради Advisor."
                .into(),
        );
    }
    if vacuum_segment_start > 0 {
        limitations.push(format!(
            "Для VACUUM/autovacuum counters используются последние {} из {} snapshots; более старые точки исключены после неизвестного/изменённого stats_reset, смены эпохи PostgreSQL или нарушения непрерывности cumulative counters.",
            vacuum_samples.len(),
            samples.len(),
        ));
        if samples[..vacuum_segment_start].iter().any(|sample| {
            sample.postmaster_start_time != overview.postmaster_start_time
                || sample.in_recovery != overview.in_recovery
        }) {
            limitations.push(
                "Старые snapshots из другой или неизвестной эпохи PostgreSQL (pg_postmaster_start_time/pg_is_in_recovery) исключены из сопоставимого окна; нужен новый непрерывный baseline на текущем сервере."
                    .into(),
            );
        }
    }

    let (xid_rate_per_hour, relation_age_delta, estimated_horizon_advance) =
        policy_history_deltas(vacuum_samples, &mut limitations);

    let vacuum_cycles_delta = if stats_period_continuous {
        monotonic_vacuum_cycle_delta(vacuum_samples, &mut limitations)
    } else {
        if history_points >= 2 {
            limitations.push(
                "Для VACUUM/autovacuum counters нет двух сопоставимых последних snapshots: stats_reset изменился или cumulative maintenance counter уменьшился."
                    .into(),
            );
        }
        None
    };

    let relation_name = relation_display_name(relation);
    let pct = relation.pct_of_freeze_max.unwrap_or_else(|| {
        if relation.effective_freeze_max_age > 0 {
            relation.xid_age as f64 / relation.effective_freeze_max_age as f64 * 100.0
        } else {
            0.0
        }
    });
    let headroom = relation.effective_freeze_max_age - relation.xid_age;

    let mut observations = vec![
        format!(
            "XID age = {}; effective freeze max = {}; headroom = {} ({pct:.1}%).",
            relation.xid_age, relation.effective_freeze_max_age, headroom,
        ),
        format!(
            "Current policy sources: max={}, table_age={}, min_age={}.",
            relation.freeze_max_source,
            relation.freeze_table_source.as_deref().unwrap_or("unknown"),
            relation.freeze_min_source.as_deref().unwrap_or("unknown"),
        ),
    ];

    if let Some(span) = history_span_seconds {
        observations.push(format!(
            "History: {history_points} point(s), span {:.1} min.",
            span / 60.0,
        ));
    } else {
        observations.push(format!("History: {history_points} point(s)."));
    }

    if let Some(rate) = xid_rate_per_hour {
        observations.push(format!("Observed cluster XID rate ≈ {rate:.0} XID/hour."));
    }

    if let Some(delta) = relation_age_delta {
        observations.push(format!(
            "Relation XID age delta over the window = {delta:+}."
        ));
    }

    if let Some(advance) = estimated_horizon_advance {
        observations.push(format!(
            "Estimated relfrozenxid advance over the window ≈ {advance} XID."
        ));
    }

    if let Some(cycles) = vacuum_cycles_delta {
        observations.push(format!(
            "Observed VACUUM/autovacuum counter delta = {cycles}."
        ));
    }

    if let Some(trigger) = trigger {
        observations.push(format!(
            "Normal autovacuum: enabled={}, dead_trigger={}, insert_trigger={}.",
            trigger.autovacuum_enabled,
            trigger.dead_trigger_reached,
            trigger.insert_trigger_reached,
        ));
    }

    if let Some(all_visible_pct) = relation.all_visible_pct {
        observations.push(format!(
            "pg_class all-visible estimate = {all_visible_pct:.1}%; это не all-frozen coverage."
        ));
    }

    if relation.last_analyze.is_some()
        || relation.last_autoanalyze.is_some()
        || relation.modifications_since_analyze.is_some()
    {
        observations.push(format!(
            "ANALYZE context: last manual={}, last auto={}, modifications since analyze={}. ANALYZE не продвигает relfrozenxid и используется здесь только как контекст.",
            relation.last_analyze.as_deref().unwrap_or("unknown"),
            relation.last_autoanalyze.as_deref().unwrap_or("unknown"),
            relation
                .modifications_since_analyze
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
        ));
    }

    // A restored/replaced server can reuse OIDs and connection details.  Only
    // samples from the current postmaster lifetime may justify a write candidate.
    let same_server_epoch = overview
        .postmaster_start_time
        .as_deref()
        .filter(|start| !start.is_empty())
        .is_some_and(|start| {
            overview.in_recovery == Some(false)
                && !vacuum_samples.is_empty()
                && vacuum_samples.iter().all(|sample| {
                    sample.postmaster_start_time.as_deref() == Some(start)
                        && sample.in_recovery == Some(false)
                })
        });
    if !same_server_epoch {
        limitations.push(
            "История XID не подтверждена для текущей эпохи PostgreSQL (pg_postmaster_start_time/pg_is_in_recovery): после restart/failover/recovery или при legacy snapshots нужен новый baseline на primary."
                .into(),
        );
    }
    if !vacuum_section_available {
        limitations.push(
            "Секция текущего VACUUM недоступна; отсутствие наблюдаемого процесса нельзя считать подтверждённым. Policy candidate не формируется."
                .into(),
        );
    }
    let sufficient_history = same_server_epoch
        && history_points >= POLICY_MIN_HISTORY_POINTS
        && vacuum_samples.len() >= POLICY_MIN_HISTORY_POINTS
        && comparable_span_seconds.is_some_and(|span| span >= POLICY_MIN_HISTORY_SPAN_SECONDS);
    let horizon_advanced = estimated_horizon_advance.is_some_and(|advance| advance > 0);
    let current_vacuum_active = active_vacuum.is_some();

    let mut candidates = Vec::new();
    let mut primary_candidate_id = None;

    let (status, title, summary) = if relation.xid_age >= relation.effective_freeze_max_age {
        (
            "attention",
            crate::i18n::text("Сначала разобраться с текущим freeze pressure", "Investigate current freeze pressure first"),
            "Relation уже достигла effective autovacuum_freeze_max_age. Изменение threshold сейчас не заменяет проверку anti-wraparound VACUUM, blockers и следующего snapshot.",
        )
    } else if current_vacuum_active {
        (
            "observe",
            "VACUUM уже выполняется",
            "Не менять freeze policy во время наблюдаемого VACUUM: дождаться завершения и проверить, продвинулся ли freeze horizon в следующем snapshot.",
        )
    } else if !vacuum_section_available {
        (
            "insufficient_quality",
            "Текущий VACUUM нельзя проверить",
            "Секция VACUUM недоступна: pgXray не знает, выполняется ли обслуживание. Candidate не создаётся до успешного нового снимка.",
        )
    } else if !sufficient_history {
        limitations.push(format!(
            "Policy candidate требует минимум {POLICY_MIN_HISTORY_POINTS} сопоставимых snapshots и {:.0} минут непрерывного наблюдения; при stats_reset=NULL накопительные счётчики VACUUM не подтверждены.",
            POLICY_MIN_HISTORY_SPAN_SECONDS / 60.0,
        ));
        (
            "insufficient_history",
            "Недостаточно history для изменения policy",
            "Текущий snapshot полезен для диагностики, но ещё не показывает устойчивое поведение freeze horizon.",
        )
    } else if horizon_advanced {
        (
            "working",
            "Freeze horizon продвигался",
            "В выбранном history window relfrozenxid уже продвигался. Автоматически занижать thresholds по этим данным не требуется; важнее продолжить наблюдение cadence.",
        )
    } else if pct < POLICY_REVIEW_FREEZE_PCT {
        (
            "observe",
            "Headroom пока большой",
            "Freeze horizon не продвинулся в выбранном окне, но relation ещё далеко от effective freeze max. Накопить более длинную историю полезнее, чем преждевременно менять thresholds.",
        )
    } else if !stats_period_continuous {
        (
            "insufficient_quality",
            "Нельзя надёжно различить maintenance pattern",
            "Freeze horizon не продвинулся, но VACUUM counters принадлежат несопоставимым stats periods. Новый baseline нужен прежде, чем выбирать между earlier aggressive VACUUM и earlier forced anti-wraparound.",
        )
    } else if vacuum_cycles_delta.is_none() {
        (
            "insufficient_quality",
            "VACUUM counters не удалось вычислить",
            "Разница VACUUM/autovacuum counters неизвестна; нельзя приравнивать её к нулю и выбирать forced anti-wraparound candidate.",
        )
    } else if xid_rate_per_hour.is_none() {
        (
            "insufficient_quality",
            "XID rate недоступен",
            "Без сопоставимого current_xid trend pgXray не рассчитывает числовой freeze candidate.",
        )
    } else {
        let cycles = vacuum_cycles_delta.expect("cycle delta validated above");

        if cycles > 0 {
            if let Some(candidate) =
                freeze_table_age_candidate(overview, relation, xid_rate_per_hour.unwrap_or(0.0))
            {
                primary_candidate_id = Some(candidate.id.clone());
                candidates.push(candidate);
                (
                    "candidate",
                    "Есть candidate для earlier aggressive VACUUM",
                    "VACUUM cycles наблюдались, но freeze horizon за окно не продвинулся. Candidate снижает table-age threshold так, чтобы следующий подходящий VACUUM раньше перешёл в aggressive strategy.",
                )
            } else {
                (
                    "review",
                    "VACUUM cycles есть, но готового candidate нет",
                    "Freeze horizon не продвинулся несмотря на наблюдаемые VACUUM cycles, однако текущий freeze_table_age уже не выглядит завышенным относительно relation age. Нужен следующий snapshot и Table Health context.",
                )
            }
        } else if let Some(candidate) =
            freeze_max_age_candidate(overview, relation, xid_rate_per_hour.unwrap_or(0.0))
        {
            primary_candidate_id = Some(candidate.id.clone());
            candidates.push(candidate);
            (
                "candidate",
                "Есть candidate для earlier anti-wraparound protection",
                "За history window не видно ни продвижения freeze horizon, ни VACUUM cycle. Для cold/static relation можно рассмотреть более ранний per-relation freeze max, сохранив запас относительно текущего XID rate.",
            )
        } else {
            (
                "review",
                "Нужен более длинный history window",
                "Freeze horizon не продвинулся и VACUUM cycles не наблюдались, но безопасный числовой threshold из текущих данных не получается без слишком малого headroom.",
            )
        }
    };

    if candidates.is_empty() && pct >= POLICY_REVIEW_FREEZE_PCT {
        limitations.push(
            "Отсутствие candidate не означает, что настройки идеальны: pgXray сознательно не предлагает числовой threshold, когда evidence недостаточно или maintenance уже работает."
                .into(),
        );
    }

    if let (Some(mxid_age), Some(mxid_max)) = (
        relation.mxid_age,
        relation.effective_multixact_freeze_max_age,
    ) {
        observations.push(format!(
            "MultiXact age = {mxid_age}; effective MultiXact freeze max = {mxid_max}."
        ));
        if mxid_age >= mxid_max {
            limitations.push(
                "MultiXact freeze threshold достигнут: XID policy candidate не устраняет MXID pressure. Проверьте независимую диагностику MXID до применения XID-настроек."
                    .into(),
            );
        }
    }
    limitations.push(
        "Этот Policy Advisor изменяет только XID freeze policy. MultiXact имеет отдельные thresholds; здесь они не рассчитываются и не применяются."
            .into(),
    );
    limitations.push(
        "Policy Advisor не использует last ANALYZE как причину XID growth: ANALYZE обновляет planner statistics, но не продвигает relfrozenxid."
            .into(),
    );
    limitations.push(
        "relallvisible — только all-visible estimate из pg_class. Для exact all-frozen coverage нужен отдельный pg_visibility probe; Advisor не считает all-visible страницы frozen."
            .into(),
    );
    limitations.push(
        "Generated SQL — preview. Этот read-only этап ничего не выполняет и не учитывает lock acquisition/ownership; apply-path должен повторно проверить relation OID и current reloptions."
            .into(),
    );

    XidRelationPolicyAdvice {
        relation_oid: relation.relation_oid,
        relation_name,
        status: status.into(),
        title: title.into(),
        summary: summary.into(),
        history_points,
        history_span_seconds,
        xid_rate_per_hour,
        relation_age_delta,
        estimated_horizon_advance,
        vacuum_cycles_delta,
        stats_period_continuous,
        current_policy: XidFreezePolicyView {
            effective_freeze_max_age: relation.effective_freeze_max_age,
            freeze_max_source: relation.freeze_max_source.clone(),
            effective_freeze_table_age: relation.effective_freeze_table_age,
            freeze_table_source: relation.freeze_table_source.clone(),
            effective_freeze_min_age: relation.effective_freeze_min_age,
            freeze_min_source: relation.freeze_min_source.clone(),
        },
        primary_candidate_id,
        candidates,
        observations,
        limitations,
    }
}

fn policy_history_deltas(
    samples: &[XidPolicyHistorySample],
    limitations: &mut Vec<String>,
) -> (Option<f64>, Option<i64>, Option<i64>) {
    let (Some(oldest), Some(newest)) = (samples.first(), samples.last()) else {
        return (None, None, None);
    };

    if oldest.snapshot_id == newest.snapshot_id {
        return (None, None, None);
    }

    let elapsed_ms = newest.collection_started_at_ms - oldest.collection_started_at_ms;
    if elapsed_ms <= 0 {
        limitations.push(
            crate::i18n::text(
                "History window не имеет положительного временного интервала.",
                "The history window has no positive time interval.",
            )
            .into(),
        );
        return (None, None, None);
    }

    let older_xid = match oldest.current_xid.parse::<u64>() {
        Ok(value) => value,
        Err(_) => {
            limitations.push(format!(
                "Не удалось разобрать current_xid snapshot #{} как xid8.",
                oldest.snapshot_id,
            ));
            return (None, None, None);
        }
    };
    let newer_xid = match newest.current_xid.parse::<u64>() {
        Ok(value) => value,
        Err(_) => {
            limitations.push(format!(
                "Не удалось разобрать current_xid snapshot #{} как xid8.",
                newest.snapshot_id,
            ));
            return (None, None, None);
        }
    };
    let Some(xid_delta) = newer_xid.checked_sub(older_xid) else {
        limitations.push(
            "newer current_xid меньше older current_xid; snapshots могут принадлежать несопоставимому cluster epoch/recovery state."
                .into(),
        );
        return (None, None, None);
    };
    let Ok(xid_delta_i64) = i64::try_from(xid_delta) else {
        limitations.push(
            crate::i18n::text(
                "XID delta слишком велик для policy calculation.",
                "The XID delta is too large for policy calculation.",
            )
            .into(),
        );
        return (None, None, None);
    };

    let elapsed_seconds = elapsed_ms as f64 / 1000.0;
    let xid_rate_per_hour = xid_delta as f64 / elapsed_seconds * 3600.0;
    let relation_age_delta = newest.relation_xid_age - oldest.relation_xid_age;
    let horizon_advance = xid_delta_i64 - relation_age_delta;

    if horizon_advance < 0 {
        limitations.push(
            "Relation XID age вырос быстрее observed current_xid delta; exact freeze advance по этому window не рассчитывается."
                .into(),
        );
        return (Some(xid_rate_per_hour), Some(relation_age_delta), None);
    }

    (
        Some(xid_rate_per_hour),
        Some(relation_age_delta),
        Some(horizon_advance),
    )
}

fn vacuum_stats_pair_continuous(
    older: &XidPolicyHistorySample,
    newer: &XidPolicyHistorySample,
) -> bool {
    matches!(
        (&older.stats_reset, &newer.stats_reset),
        (Some(older_reset), Some(newer_reset))
            if !older_reset.is_empty() && older_reset == newer_reset
    ) && matches!(
        (&older.postmaster_start_time, &newer.postmaster_start_time),
        (Some(older_start), Some(newer_start))
            if !older_start.is_empty() && older_start == newer_start
    ) && older.in_recovery == Some(false)
        && newer.in_recovery == Some(false)
        && newer.collection_started_at_ms > older.collection_started_at_ms
        && newer.vacuum_count >= older.vacuum_count
        && newer.autovacuum_count >= older.autovacuum_count
}

fn monotonic_vacuum_cycle_delta(
    samples: &[XidPolicyHistorySample],
    limitations: &mut Vec<String>,
) -> Option<i64> {
    let (Some(oldest), Some(newest)) = (samples.first(), samples.last()) else {
        return None;
    };

    let Some(older) = oldest.vacuum_count.checked_add(oldest.autovacuum_count) else {
        limitations.push(
            crate::i18n::text(
                "VACUUM counters переполнились в первом snapshot; cycle delta неизвестна.",
                "VACUUM counters overflowed in the first snapshot; the cycle delta is unknown.",
            )
            .into(),
        );
        return None;
    };
    let Some(newer) = newest.vacuum_count.checked_add(newest.autovacuum_count) else {
        limitations.push(
            crate::i18n::text(
                "VACUUM counters переполнились в последнем snapshot; cycle delta неизвестна.",
                "VACUUM counters overflowed in the last snapshot; the cycle delta is unknown.",
            )
            .into(),
        );
        return None;
    };

    match newer.checked_sub(older) {
        Some(delta) if delta >= 0 => Some(delta),
        _ => {
            limitations.push(
                "VACUUM/autovacuum counters уменьшились внутри непрерывного stats period; cycle delta не используется."
                    .into(),
            );
            None
        }
    }
}

fn freeze_table_age_candidate(
    overview: &XidOverview,
    relation: &XidRelationAge,
    xid_rate_per_hour: f64,
) -> Option<XidPolicyCandidate> {
    if overview.in_recovery != Some(false) {
        return None;
    }
    let current = relation.effective_freeze_table_age?;
    let minimum = relation
        .effective_freeze_min_age
        .unwrap_or(0)
        .saturating_add((relation.effective_freeze_max_age / 20).max(POLICY_ROUND_XIDS));
    let lead = (xid_rate_per_hour.ceil() as i64)
        .max(relation.effective_freeze_max_age / 100)
        .max(POLICY_ROUND_XIDS);
    let desired = relation.xid_age.saturating_sub(lead).max(minimum);
    let candidate_value = round_down_xids(desired, POLICY_ROUND_XIDS);

    if candidate_value <= 0 || candidate_value >= current {
        return None;
    }

    let ddl = policy_ddl_target(relation)?;
    let parameter = ddl.parameter("autovacuum_freeze_table_age");
    let sql = ddl.sql(&parameter, candidate_value);

    Some(XidPolicyCandidate {
        expected_postmaster_start_time: overview.postmaster_start_time.clone()?,
        expected_in_recovery: overview.in_recovery?,
        id: "earlier-aggressive-vacuum".into(),
        title: "Earlier aggressive VACUUM".into(),
        parameter,
        current_value: current,
        candidate_value,
        ddl_target: ddl.display_name,
        sql,
        why: vec![
            crate::i18n::text("В history есть VACUUM/autovacuum cycles, но observed freeze horizon не продвинулся.", "VACUUM/autovacuum cycles were observed in history, but the observed freeze horizon did not advance.").into(),
            "Candidate ставит aggressive threshold немного ниже текущего XID age с запасом относительно observed XID rate.".into(),
        ],
        expected_effect: vec![
            "Следующий подходящий VACUUM после достижения threshold должен использовать aggressive strategy и сканировать страницы, которые обычный VACUUM мог пропускать по visibility map.".into(),
            "Если eligible tuples действительно могут быть frozen, следующий snapshot должен показать продвижение relfrozenxid / уменьшение XID age.".into(),
        ],
        tradeoffs: vec![
            "Aggressive VACUUM читает больше heap pages и может увеличить I/O/WAL по сравнению с обычным VACUUM.".into(),
            "Это не меняет normal dead/insert triggers; оно меняет поведение уже запущенного VACUUM.".into(),
        ],
    })
}

pub(super) fn freeze_max_age_candidate(
    overview: &XidOverview,
    relation: &XidRelationAge,
    xid_rate_per_hour: f64,
) -> Option<XidPolicyCandidate> {
    if xid_rate_per_hour <= 0.0 || overview.in_recovery != Some(false) {
        return None;
    }

    let current = relation.effective_freeze_max_age;
    let rate_margin = (xid_rate_per_hour / 3600.0 * POLICY_FORCE_WINDOW_SECONDS).ceil() as i64;
    let pct_margin = (current as f64 * POLICY_MIN_FORCE_MARGIN_PCT).ceil() as i64;
    let margin = rate_margin.max(pct_margin).max(POLICY_ROUND_XIDS);
    let desired = relation.xid_age.checked_add(margin)?;
    let candidate_value = round_up_xids(desired, POLICY_ROUND_XIDS);

    if candidate_value <= relation.xid_age
        || candidate_value >= current
        || candidate_value > overview.autovacuum_freeze_max_age
    {
        return None;
    }

    let ddl = policy_ddl_target(relation)?;
    let parameter = ddl.parameter("autovacuum_freeze_max_age");
    let sql = ddl.sql(&parameter, candidate_value);

    Some(XidPolicyCandidate {
        expected_postmaster_start_time: overview.postmaster_start_time.clone()?,
        expected_in_recovery: overview.in_recovery?,
        id: "earlier-anti-wraparound".into(),
        title: "Earlier anti-wraparound protection".into(),
        parameter,
        current_value: current,
        candidate_value,
        ddl_target: ddl.display_name,
        sql,
        why: vec![
            "Freeze horizon не продвинулся, а VACUUM/autovacuum cycle в сопоставимом stats period не наблюдался.".into(),
            "Candidate рассчитывается как current relation age плюс больший из двух запасов: около суток observed XID growth или 10% текущего freeze max; если такой запас уже не помещается ниже current threshold, candidate не создаётся.".into(),
        ],
        expected_effect: vec![
            "Для этой relation anti-wraparound autovacuum станет eligible раньше, даже если normal dead/insert triggers не срабатывают.".into(),
            "Это особенно применимо к cold/static relation, которая редко получает normal VACUUM.".into(),
        ],
        tradeoffs: vec![
            "Lower freeze max означает более частые forced aggressive autovacuum для этой relation.".into(),
            "Если новый freeze max становится ниже current freeze_table_age, PostgreSQL также ограничивает effective aggressive threshold относительно freeze max; фактический cadence aggressive VACUUM может измениться сильнее одного параметра.".into(),
            "На большой relation это может заметно увеличить I/O; перед apply нужно проверить размер, текущий VACUUM и maintenance window.".into(),
        ],
    })
}

pub(super) fn relation_policy_live_sql(relation_oid: u32) -> String {
    format!(
        r#"
WITH global_cfg AS (
    SELECT
        current_setting('autovacuum_freeze_max_age')::bigint AS freeze_max_age,
        current_setting('vacuum_freeze_min_age')::bigint AS freeze_min_age,
        current_setting('vacuum_freeze_table_age')::bigint AS freeze_table_age
),
target AS (
    SELECT
        c.oid,
        c.relnamespace,
        c.relname,
        c.relkind,
        c.relfrozenxid,
        c.reloptions
    FROM pg_class AS c
    JOIN pg_namespace AS n
        ON n.oid = c.relnamespace
    WHERE c.oid = {relation_oid}::oid
      AND c.relkind IN ('r', 'm', 't')
      AND n.nspname !~ '^pg_temp_'
      AND n.nspname !~ '^pg_toast_temp_'
),
effective AS (
    SELECT
        t.*,
        LEAST(COALESCE(opts.freeze_max_age, cfg.freeze_max_age),
              cfg.freeze_max_age)::bigint AS effective_freeze_max_age,
        CASE WHEN opts.freeze_max_age IS NULL THEN 'global' ELSE 'table' END
            AS freeze_max_source,
        LEAST(COALESCE(opts.freeze_table_age, cfg.freeze_table_age),
              floor(0.95 * LEAST(COALESCE(opts.freeze_max_age, cfg.freeze_max_age),
                                  cfg.freeze_max_age)))::bigint AS effective_freeze_table_age,
        CASE WHEN opts.freeze_table_age IS NULL THEN 'global' ELSE 'table' END
            AS freeze_table_source,
        LEAST(COALESCE(opts.freeze_min_age, cfg.freeze_min_age),
              cfg.freeze_max_age / 2)::bigint AS effective_freeze_min_age,
        CASE WHEN opts.freeze_min_age IS NULL THEN 'global' ELSE 'table' END
            AS freeze_min_source
    FROM target AS t
    CROSS JOIN global_cfg AS cfg
    LEFT JOIN LATERAL (
        SELECT
            max(CASE WHEN o.option_name = 'autovacuum_freeze_max_age'
                THEN o.option_value::bigint END) AS freeze_max_age,
            max(CASE WHEN o.option_name = 'autovacuum_freeze_table_age'
                THEN o.option_value::bigint END) AS freeze_table_age,
            max(CASE WHEN o.option_name = 'autovacuum_freeze_min_age'
                THEN o.option_value::bigint END) AS freeze_min_age
        FROM pg_options_to_table(t.reloptions) AS o
    ) AS opts ON true
)
SELECT COALESCE(
    (
        SELECT row_to_json(q)::json
        FROM (
            SELECT
                EXTRACT(EPOCH FROM pg_postmaster_start_time())::numeric::text
                    AS postmaster_start_time,
                pg_is_in_recovery() AS in_recovery,
                e.oid::bigint AS relation_oid,
                n.nspname::text AS schema_name,
                e.relname::text AS relation_name,
                e.relkind::text AS relation_type,
                age(e.relfrozenxid)::bigint AS xid_age,
                e.effective_freeze_max_age,
                e.freeze_max_source,
                e.effective_freeze_table_age,
                e.freeze_table_source,
                e.effective_freeze_min_age,
                e.freeze_min_source,
                parent.oid::bigint AS parent_oid,
                parent_ns.nspname::text AS parent_schema,
                parent.relname::text AS parent_relation,
                parent.relkind::text AS parent_relation_type,
                progress.pid AS active_vacuum_pid,
                progress.phase::text AS active_vacuum_phase
            FROM effective AS e
            JOIN pg_namespace AS n
                ON n.oid = e.relnamespace
            LEFT JOIN pg_class AS parent
                ON parent.reltoastrelid = e.oid
            LEFT JOIN pg_namespace AS parent_ns
                ON parent_ns.oid = parent.relnamespace
            LEFT JOIN LATERAL (
                SELECT p.pid, p.phase
                FROM pg_stat_progress_vacuum AS p
                WHERE p.relid = e.oid
                ORDER BY p.pid
                LIMIT 1
            ) AS progress ON true
        ) AS q
    ),
    'null'::json
)::text;
"#,
    )
}

fn policy_parameter_base(parameter: &str) -> Option<&str> {
    parameter
        .strip_prefix("toast.")
        .or(Some(parameter))
        .filter(|value| {
            matches!(
                *value,
                "autovacuum_freeze_max_age" | "autovacuum_freeze_table_age"
            )
        })
}

fn relation_policy_value_source<'a>(
    relation: &'a XidRelationAge,
    parameter: &str,
) -> Option<(i64, &'a str)> {
    match parameter {
        "autovacuum_freeze_max_age" => Some((
            relation.effective_freeze_max_age,
            relation.freeze_max_source.as_str(),
        )),
        "autovacuum_freeze_table_age" => Some((
            relation.effective_freeze_table_age?,
            relation.freeze_table_source.as_deref()?,
        )),
        _ => None,
    }
}

fn live_policy_value_source<'a>(
    state: &'a XidPolicyLiveState,
    parameter: &str,
) -> Option<(i64, &'a str)> {
    match parameter {
        "autovacuum_freeze_max_age" => Some((
            state.effective_freeze_max_age,
            state.freeze_max_source.as_str(),
        )),
        "autovacuum_freeze_table_age" => Some((
            state.effective_freeze_table_age?,
            state.freeze_table_source.as_deref()?,
        )),
        _ => None,
    }
}

fn same_policy_relation_identity(anchor: &XidRelationAge, live: &XidPolicyLiveState) -> bool {
    anchor.relation_oid == live.relation_oid
        && anchor.schema_name == live.schema_name
        && anchor.relation_name == live.relation_name
        && anchor.relation_type == live.relation_type
        && anchor.parent_oid == live.parent_oid
        && anchor.parent_schema == live.parent_schema
        && anchor.parent_relation == live.parent_relation
        && anchor.parent_relation_type == live.parent_relation_type
}

pub(super) fn validate_policy_apply_guard(
    anchor: &XidRelationAge,
    live: &XidPolicyLiveState,
    candidate: &XidPolicyCandidate,
) -> Result<(), String> {
    if candidate.expected_in_recovery
        || live.in_recovery != Some(false)
        || candidate.expected_postmaster_start_time.is_empty()
        || live.postmaster_start_time.as_deref()
            != Some(candidate.expected_postmaster_start_time.as_str())
    {
        return Err(
            "Сервер PostgreSQL перезапущен или сменился после XID snapshot. Соберите новый XID snapshot и историю перед apply."
                .into(),
        );
    }

    if !same_policy_relation_identity(anchor, live) {
        return Err(
            "Relation изменилась после snapshot: OID/name/type/TOAST parent больше не совпадают. Соберите новый XID snapshot перед apply."
                .into(),
        );
    }

    let parameter = policy_parameter_base(&candidate.parameter)
        .ok_or("Policy candidate содержит неподдерживаемый parameter")?;
    let (anchor_value, anchor_source) = relation_policy_value_source(anchor, parameter)
        .ok_or("В anchor snapshot отсутствует policy value для candidate")?;
    let (live_value, live_source) = live_policy_value_source(live, parameter)
        .ok_or("Текущую effective policy relation не удалось прочитать")?;

    if anchor_value != candidate.current_value {
        return Err("Policy candidate не соответствует anchor snapshot".into());
    }

    if live_value != anchor_value || live_source != anchor_source {
        return Err(format!(
            "Freeze policy изменилась после snapshot: было {anchor_value} ({anchor_source}), сейчас {live_value} ({live_source}). Соберите новый snapshot и пересчитайте candidate."
        ));
    }

    if let Some(pid) = live.active_vacuum_pid {
        return Err(format!(
            "Для relation сейчас виден VACUUM PID {pid}{}; pgXray не меняет freeze policy во время active VACUUM. Дождитесь завершения и соберите новый snapshot.",
            live.active_vacuum_phase
                .as_deref()
                .map(|phase| format!(" · {phase}"))
                .unwrap_or_default(),
        ));
    }

    if parameter == "autovacuum_freeze_max_age" && candidate.candidate_value <= live.xid_age {
        return Err(format!(
            "Candidate autovacuum_freeze_max_age={} уже не выше текущего XID age={}; candidate устарел. Соберите новый snapshot.",
            candidate.candidate_value, live.xid_age,
        ));
    }

    Ok(())
}

pub(super) fn policy_ddl_target_live(state: &XidPolicyLiveState) -> Option<XidPolicyDdlTarget> {
    match XidRelationKind::from_catalog(&state.relation_type)? {
        XidRelationKind::Table => Some(XidPolicyDdlTarget {
            command: "ALTER TABLE",
            qualified_name: qualified_identifier(&state.schema_name, &state.relation_name),
            display_name: format!("{}.{}", state.schema_name, state.relation_name),
            toast_prefix: false,
        }),
        XidRelationKind::MaterializedView => Some(XidPolicyDdlTarget {
            command: "ALTER MATERIALIZED VIEW",
            qualified_name: qualified_identifier(&state.schema_name, &state.relation_name),
            display_name: format!("{}.{}", state.schema_name, state.relation_name),
            toast_prefix: false,
        }),
        XidRelationKind::Toast => {
            let schema = state.parent_schema.as_deref()?;
            let name = state.parent_relation.as_deref()?;
            let command = match state.parent_relation_type.as_deref()? {
                "r" | "p" => "ALTER TABLE",
                "m" => "ALTER MATERIALIZED VIEW",
                _ => return None,
            };

            Some(XidPolicyDdlTarget {
                command,
                qualified_name: qualified_identifier(schema, name),
                display_name: format!("{schema}.{name} (TOAST)"),
                toast_prefix: true,
            })
        }
        _ => None,
    }
}

fn quote_sql_literal(value: &str) -> String {
    // Explicit E'' strings preserve both quotes and backslashes regardless of
    // the session's standard_conforming_strings setting.
    format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

pub(super) fn guarded_policy_apply_sql(
    expected_database_oid: u32,
    anchor: &XidRelationAge,
    candidate: &XidPolicyCandidate,
    ddl: &XidPolicyDdlTarget,
) -> Result<String, String> {
    let base_parameter = policy_parameter_base(&candidate.parameter)
        .ok_or("Policy candidate содержит неподдерживаемый parameter")?;
    let (expected_value, expected_source) = relation_policy_value_source(anchor, base_parameter)
        .ok_or("В anchor snapshot отсутствует policy value для candidate")?;
    let global_setting = match base_parameter {
        "autovacuum_freeze_max_age" => "autovacuum_freeze_max_age",
        "autovacuum_freeze_table_age" => "vacuum_freeze_table_age",
        _ => return Err("Неподдерживаемый freeze parameter для guarded apply".into()),
    };
    let (target_oid, target_schema, target_name) = if anchor.relation_type == "t" {
        (
            anchor
                .parent_oid
                .ok_or("TOAST candidate не содержит parent OID")?,
            anchor
                .parent_schema
                .as_deref()
                .ok_or("TOAST candidate не содержит parent schema")?,
            anchor
                .parent_relation
                .as_deref()
                .ok_or("TOAST candidate не содержит parent relation")?,
        )
    } else {
        (
            anchor.relation_oid,
            anchor.schema_name.as_str(),
            anchor.relation_name.as_str(),
        )
    };
    let parameter = ddl.parameter(base_parameter);
    let alter = ddl.sql(&parameter, candidate.candidate_value);

    let body = format!(
        r#"DECLARE
    current_target_oid oid;
    current_schema text;
    current_name text;
    current_type text;
    current_value bigint;
    current_source text;
BEGIN
    IF (SELECT oid FROM pg_database WHERE datname = current_database())
       IS DISTINCT FROM {expected_database_oid}::oid
    THEN
        RAISE EXCEPTION 'pgXray guard failed: database identity changed';
    END IF;

    IF pg_is_in_recovery() IS DISTINCT FROM {expected_in_recovery}
       OR EXTRACT(EPOCH FROM pg_postmaster_start_time())::numeric::text
          IS DISTINCT FROM {expected_postmaster_start_time}
    THEN
        RAISE EXCEPTION 'pgXray guard failed: server epoch changed';
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

    SELECT
        n.nspname::text,
        c.relname::text,
        c.relkind::text,
        CASE
            WHEN {base_parameter} = 'autovacuum_freeze_max_age' THEN
                LEAST(
                    COALESCE(
                        (SELECT o.option_value::bigint
                         FROM pg_options_to_table(c.reloptions) AS o
                         WHERE o.option_name = {base_parameter}
                         LIMIT 1),
                        current_setting({global_setting})::bigint
                    ),
                    current_setting('autovacuum_freeze_max_age')::bigint
                )
            ELSE
                LEAST(
                    COALESCE(
                        (SELECT o.option_value::bigint
                         FROM pg_options_to_table(c.reloptions) AS o
                         WHERE o.option_name = {base_parameter}
                         LIMIT 1),
                        current_setting({global_setting})::bigint
                    ),
                    LEAST(
                        COALESCE(
                            (SELECT o.option_value::bigint
                             FROM pg_options_to_table(c.reloptions) AS o
                             WHERE o.option_name = 'autovacuum_freeze_max_age'
                             LIMIT 1),
                            current_setting('autovacuum_freeze_max_age')::bigint
                        ),
                        current_setting('autovacuum_freeze_max_age')::bigint
                    ) * 95 / 100
                )
        END,
        CASE
            WHEN EXISTS (
                SELECT 1
                FROM pg_options_to_table(c.reloptions) AS o
                WHERE o.option_name = {base_parameter}
            ) THEN 'table'
            ELSE 'global'
        END
    INTO
        current_schema,
        current_name,
        current_type,
        current_value,
        current_source
    FROM pg_class AS c
    JOIN pg_namespace AS n ON n.oid = c.relnamespace
    WHERE c.oid = {subject_oid}::oid;

    IF current_schema IS DISTINCT FROM {subject_schema}
       OR current_name IS DISTINCT FROM {subject_name}
       OR current_type IS DISTINCT FROM {subject_type}
    THEN
        RAISE EXCEPTION 'pgXray guard failed: relation identity changed';
    END IF;

    IF current_value IS DISTINCT FROM {expected_value}
       OR current_source IS DISTINCT FROM {expected_source}
    THEN
        RAISE EXCEPTION 'pgXray guard failed: freeze policy changed';
    END IF;

    IF {base_parameter} = 'autovacuum_freeze_max_age'
       AND {candidate_value} > current_setting('autovacuum_freeze_max_age')::bigint
    THEN
        RAISE EXCEPTION 'pgXray guard failed: candidate exceeds global freeze max';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM pg_stat_progress_vacuum
        WHERE relid = {subject_oid}::oid
    ) THEN
        RAISE EXCEPTION 'pgXray guard failed: VACUUM is active';
    END IF;

    {alter}
END"#,
        expected_database_oid = expected_database_oid,
        expected_postmaster_start_time =
            quote_sql_literal(&candidate.expected_postmaster_start_time),
        expected_in_recovery = candidate.expected_in_recovery,
        candidate_value = candidate.candidate_value,
        target = ddl.qualified_name,
        target_schema = quote_sql_literal(target_schema),
        target_name = quote_sql_literal(target_name),
        target_oid = target_oid,
        base_parameter = quote_sql_literal(base_parameter),
        global_setting = quote_sql_literal(global_setting),
        subject_oid = anchor.relation_oid,
        subject_schema = quote_sql_literal(&anchor.schema_name),
        subject_name = quote_sql_literal(&anchor.relation_name),
        subject_type = quote_sql_literal(&anchor.relation_type),
        expected_value = expected_value,
        expected_source = quote_sql_literal(expected_source),
        alter = alter,
    );

    // Dollar-quoted DO bodies must not use a delimiter found anywhere in
    // caller-controlled relation identifiers or literals.
    let mut delimiter = "$pgxray$".to_owned();
    let mut suffix = 0_u64;
    while body.contains(&delimiter) {
        delimiter = format!("$pgxray_{suffix}$");
        suffix = suffix.saturating_add(1);
    }
    Ok(format!("DO {delimiter}\n{body}\n{delimiter};"))
}

pub async fn apply_policy_candidate(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    expected_database_oid: u32,
    anchor_relation: &XidRelationAge,
    candidate: &XidPolicyCandidate,
) -> Result<XidPolicyApplyVerification, String> {
    let before: Option<XidPolicyLiveState> = query_json(
        internal_postgres,
        connection_id,
        input,
        &relation_policy_live_sql(anchor_relation.relation_oid),
    )
    .await?;
    let before = before.ok_or_else(|| {
        format!(
            "Relation OID {} больше не существует в текущей базе",
            anchor_relation.relation_oid,
        )
    })?;

    validate_policy_apply_guard(anchor_relation, &before, candidate)?;

    let base_parameter = policy_parameter_base(&candidate.parameter)
        .ok_or("Policy candidate содержит неподдерживаемый parameter")?;
    let ddl = policy_ddl_target_live(&before)
        .ok_or("Текущий relation type не поддерживает guarded policy apply")?;
    let parameter = ddl.parameter(base_parameter);
    let sql = ddl.sql(&parameter, candidate.candidate_value);

    if parameter != candidate.parameter
        || ddl.display_name != candidate.ddl_target
        || sql != candidate.sql
    {
        return Err(
            "DDL target candidate больше не совпадает с текущей relation. Соберите новый XID snapshot перед apply."
                .into(),
        );
    }

    let guarded_sql =
        guarded_policy_apply_sql(expected_database_oid, anchor_relation, candidate, &ddl)?;

    internal_postgres::run_action_once(
        input,
        &guarded_sql,
        QueryExecutionPolicy::explicit_heavy_query(5_000, "pgxray-xid-policy"),
    )
    .await?;

    let after: Option<XidPolicyLiveState> = query_json(
        internal_postgres,
        connection_id,
        input,
        &relation_policy_live_sql(anchor_relation.relation_oid),
    )
    .await
    .map_err(|error| {
        format!(
            "ALTER завершён, но post-change verification не выполнен: {error}. Перед повторным apply сначала соберите новый XID snapshot."
        )
    })?;
    let after = after.ok_or_else(|| {
        "ALTER завершён, но relation исчезла до post-change verification. Перед дальнейшими действиями соберите новый snapshot."
            .to_owned()
    })?;

    if !same_policy_relation_identity(anchor_relation, &after) {
        return Err(
            "ALTER завершён, но identity relation изменилась до post-change verification. Перед дальнейшими действиями соберите новый snapshot."
                .into(),
        );
    }

    let (after_value, after_source) = live_policy_value_source(&after, base_parameter)
        .ok_or("ALTER завершён, но effective policy после изменения недоступна")?;

    if after_value != candidate.candidate_value || after_source != "table" {
        return Err(format!(
            "ALTER завершён, но verification ожидал {} (table), а получил {} ({}). Перед повторным apply соберите новый snapshot.",
            candidate.candidate_value, after_value, after_source,
        ));
    }

    Ok(XidPolicyApplyVerification {
        candidate_id: candidate.id.clone(),
        parameter: candidate.parameter.clone(),
        candidate_value: candidate.candidate_value,
        before,
        after,
    })
}

pub(super) struct XidPolicyDdlTarget {
    command: &'static str,
    qualified_name: String,
    display_name: String,
    toast_prefix: bool,
}

impl XidPolicyDdlTarget {
    pub(super) fn parameter(&self, name: &str) -> String {
        if self.toast_prefix {
            format!("toast.{name}")
        } else {
            name.to_owned()
        }
    }

    pub(super) fn sql(&self, parameter: &str, value: i64) -> String {
        format!(
            "{} {}\nSET ({} = {});",
            self.command, self.qualified_name, parameter, value,
        )
    }
}

pub(super) fn policy_ddl_target(relation: &XidRelationAge) -> Option<XidPolicyDdlTarget> {
    match XidRelationKind::from_catalog(&relation.relation_type)? {
        XidRelationKind::Table => Some(XidPolicyDdlTarget {
            command: "ALTER TABLE",
            qualified_name: qualified_identifier(&relation.schema_name, &relation.relation_name),
            display_name: format!("{}.{}", relation.schema_name, relation.relation_name),
            toast_prefix: false,
        }),
        XidRelationKind::MaterializedView => Some(XidPolicyDdlTarget {
            command: "ALTER MATERIALIZED VIEW",
            qualified_name: qualified_identifier(&relation.schema_name, &relation.relation_name),
            display_name: format!("{}.{}", relation.schema_name, relation.relation_name),
            toast_prefix: false,
        }),
        XidRelationKind::Toast => {
            let schema = relation.parent_schema.as_deref()?;
            let name = relation.parent_relation.as_deref()?;
            let parent_type = relation.parent_relation_type.as_deref()?;
            let command = match parent_type {
                "r" | "p" => "ALTER TABLE",
                "m" => "ALTER MATERIALIZED VIEW",
                _ => return None,
            };

            Some(XidPolicyDdlTarget {
                command,
                qualified_name: qualified_identifier(schema, name),
                display_name: format!("{schema}.{name} (TOAST)"),
                toast_prefix: true,
            })
        }
        _ => None,
    }
}

fn qualified_identifier(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(name))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn round_down_xids(value: i64, step: i64) -> i64 {
    value / step * step
}

fn round_up_xids(value: i64, step: i64) -> i64 {
    if value <= 0 {
        return 0;
    }

    value
        .saturating_add(step - 1)
        .checked_div(step)
        .unwrap_or(i64::MAX)
        .saturating_mul(step)
}

fn relation_display_name(relation: &XidRelationAge) -> String {
    let name = format!("{}.{}", relation.schema_name, relation.relation_name);

    if relation.relation_type == "t" {
        if let (Some(parent_schema), Some(parent_relation)) = (
            relation.parent_schema.as_deref(),
            relation.parent_relation.as_deref(),
        ) {
            return format!("{name} → {parent_schema}.{parent_relation}");
        }
    }

    name
}
