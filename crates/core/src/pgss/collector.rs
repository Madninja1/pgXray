use crate::{
    internal_postgres::InternalPostgresRegistry,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::{de::DeserializeOwned, Deserialize};
use std::time::{Duration, Instant};

// Soft budget: do not cancel a running internal PostgreSQL query mid-flight.
// Individual queries have their own 15s overall limit.
const PGSS_COLLECTION_BUDGET: Duration = Duration::from_secs(45);

use super::{interpretation::interpret_snapshot, model::*, queries::*};

pub async fn collect(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
) -> Result<PgssSnapshot, String> {
    let started = Instant::now();
    let overview = query_json::<PgssOverview>(
        internal_postgres,
        connection_id,
        input,
        "overview",
        OVERVIEW_SQL,
    )
    .await?;

    let unavailable = crate::i18n::text(
        "pg_stat_statements недоступен в текущей базе",
        "pg_stat_statements is unavailable in the current database",
    );
    let (
        mut stats,
        mut capacity,
        mut role_database,
        mut commands,
        mut call_targets,
        mut performance,
    ) = if overview.extension_installed {
        (
            collect_section::<PgssStatsInfo>(
                internal_postgres,
                connection_id,
                input,
                "stats",
                STATS_SQL,
                started,
            )
            .await,
            collect_section::<PgssCapacity>(
                internal_postgres,
                connection_id,
                input,
                "capacity",
                CAPACITY_SQL,
                started,
            )
            .await,
            collect_section::<Vec<PgssRoleDatabaseCardinality>>(
                internal_postgres,
                connection_id,
                input,
                "role_database",
                ROLE_DATABASE_SQL,
                started,
            )
            .await,
            collect_section::<Vec<PgssCommandCardinality>>(
                internal_postgres,
                connection_id,
                input,
                "commands",
                COMMANDS_SQL,
                started,
            )
            .await,
            collect_section::<Vec<PgssCallTargetCardinality>>(
                internal_postgres,
                connection_id,
                input,
                "call_targets",
                CALL_TARGETS_SQL,
                started,
            )
            .await,
            collect_section::<PgssPerformanceSamples>(
                internal_postgres,
                connection_id,
                input,
                "performance",
                PERFORMANCE_SQL,
                started,
            )
            .await,
        )
    } else {
        (
            unavailable_section(unavailable),
            unavailable_section(unavailable),
            unavailable_section(unavailable),
            unavailable_section(unavailable),
            unavailable_section(unavailable),
            unavailable_section(unavailable),
        )
    };

    // The sections are collected at different instants. A PGSS reset between
    // sections would produce misleading combined statistics and history deltas.
    if let Some(initial) = stats
        .data
        .as_ref()
        .and_then(|info| info.stats_reset.as_deref())
    {
        let epoch_check = if collection_budget_exhausted(started) {
            Err(crate::i18n::text(
                "бюджет сбора исчерпан до повторной проверки stats_reset",
                "Collection budget exhausted before checking stats_reset again",
            )
            .to_owned())
        } else {
            query_json::<PgssStatsInfo>(
                internal_postgres,
                connection_id,
                input,
                "stats_epoch_check",
                STATS_SQL,
            )
            .await
        };
        match epoch_check {
            Ok(end) => match epoch_continuity(Some(initial), end.stats_reset.as_deref()) {
                EpochContinuity::Stable => {}
                EpochContinuity::Changed => {
                    let reason = "pg_stat_statements.stats_reset изменился во время сбора; секции относятся к разным эпохам";
                    stats = unavailable_section(reason);
                    capacity = unavailable_section(reason);
                    role_database = unavailable_section(reason);
                    commands = unavailable_section(reason);
                    call_targets = unavailable_section(reason);
                    performance = unavailable_section(reason);
                }
                EpochContinuity::Unverified => {
                    stats = unavailable_section(
                        crate::i18n::text("Контроль эпохи PGSS не подтверждён: повторный stats_reset неизвестен", "PGSS statistics period could not be verified: the second stats_reset is unknown"),
                    );
                }
            },
            Err(error) => {
                stats = unavailable_section(&format!(
                    "Не удалось проверить непрерывность PGSS statistics period: {error}",
                ));
            }
        }
    }

    let interpretation = interpret_snapshot(
        &overview,
        &stats,
        &capacity,
        &role_database,
        &commands,
        &call_targets,
        &performance,
    );

    Ok(PgssSnapshot {
        overview,
        stats,
        capacity,
        role_database,
        commands,
        call_targets,
        performance,
        interpretation,
    })
}

/// Read-only, bounded explorer of *live* PGSS entries. Does not retrieve SQL text.
/// CALL filtering needs visible query text on the server, but never returns it.
pub async fn explore(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    filters: PgssExplorerFilter,
) -> Result<PgssExplorerPage, String> {
    let (sql, offset, limit) = explorer_sql(&filters)?;
    #[derive(Deserialize)]
    struct ExplorerResponse {
        total: u64,
        rows: Vec<PgssPerformanceRow>,
    }
    // One PostgreSQL statement returns both the exact filtered count and the page.
    // They therefore describe the same MVCC observation; pages can still drift.
    let response: ExplorerResponse =
        query_json(internal_postgres, connection_id, input, "explorer", &sql).await?;
    let mut rows = response.rows;
    let has_more = response.total > u64::from(offset) + u64::from(limit);
    rows.truncate(limit as usize);
    Ok(PgssExplorerPage {
        rows,
        total: response.total,
        offset,
        limit,
        has_more,
    })
}

/// SQL fragments are selected only from fixed options, typed numeric values, or
/// strictly validated ASCII tokens. Never concatenate arbitrary user text.
pub(super) fn explorer_sql(filters: &PgssExplorerFilter) -> Result<(String, u32, u32), String> {
    let limit = filters.limit.unwrap_or(25);
    let offset = filters.offset.unwrap_or(0);
    if !(1..=100).contains(&limit) || offset > 20_000 {
        return Err(crate::i18n::text(
            "PGSS Explorer: limit должен быть 1..100, offset <= 20000",
            "PGSS Explorer: limit must be 1..100 and offset must not exceed 20000",
        )
        .into());
    }

    let mut predicates = Vec::new();
    if let Some(userid) = filters.userid {
        predicates.push(format!("s.userid = {userid}::oid"));
    }
    if let Some(dbid) = filters.dbid {
        predicates.push(format!("s.dbid = {dbid}::oid"));
    }
    if filters
        .min_calls
        .is_some_and(|value| value > i64::MAX as u64)
        || filters
            .max_calls
            .is_some_and(|value| value > i64::MAX as u64)
    {
        return Err(crate::i18n::text(
            "PGSS Explorer: число вызовов выходит за диапазон bigint",
            "PGSS Explorer: call count is outside the bigint range",
        )
        .into());
    }
    if let (Some(low), Some(high)) = (filters.min_calls, filters.max_calls) {
        if low > high {
            return Err(crate::i18n::text(
                "PGSS Explorer: минимум вызовов больше максимума",
                "PGSS Explorer: minimum call count exceeds maximum",
            )
            .into());
        }
    }
    if let Some(min_calls) = filters.min_calls {
        predicates.push(format!("s.calls >= {min_calls}::bigint"));
    }
    if let Some(max_calls) = filters.max_calls {
        predicates.push(format!("s.calls <= {max_calls}::bigint"));
    }
    if let Some(min_exec) = filters.min_total_exec_ms {
        if !min_exec.is_finite() || !(0.0..=1.0e12).contains(&min_exec) {
            return Err(crate::i18n::text(
                "PGSS Explorer: минимальное время должно быть 0..1e12 мс",
                "PGSS Explorer: minimum duration must be between 0 and 1e12 ms",
            )
            .into());
        }
        predicates.push(format!("s.total_exec_time >= {min_exec}::double precision"));
    }
    if filters.one_call {
        predicates.push("s.calls = 1".into());
    }
    if let Some(ref queryid) = filters.queryid {
        let id = queryid
            .parse::<i64>()
            .map_err(|_| "PGSS Explorer: queryid должен быть i64")?;
        predicates.push(format!("s.queryid = {id}::bigint"));
    }
    if let Some(ref command) = filters.command {
        if !matches!(command.as_str(), "<HIDDEN>" | "<WITH/CTE>" | "<OTHER>")
            && (command.is_empty()
                || command.len() > 24
                || !command.bytes().all(|b| b.is_ascii_uppercase()))
        {
            return Err("PGSS Explorer: некорректный тип SQL-команды".into());
        }
        let expression = r#"CASE
            WHEN s.query IS NULL THEN '<HIDDEN>'
            WHEN s.query ~* '^[[:space:]]*WITH([[:space:]]|$)' THEN '<WITH/CTE>'
            ELSE upper(COALESCE((regexp_match(s.query, '^[[:space:]]*([[:alpha:]]+)'))[1], '<OTHER>'))
        END"#;
        predicates.push(format!("({expression}) = '{command}'"));
    }
    if let Some(ref tag) = filters.call_tag {
        if tag.len() != 32 || !tag.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("PGSS Explorer: CALL tag должен быть MD5 (32 hex)".into());
        }
        let tag = tag.to_ascii_lowercase();
        predicates.push(format!(
            r#"s.query IS NOT NULL
            AND s.query ~* '^[[:space:]]*CALL[[:space:]]+'
            AND md5(btrim(split_part(regexp_replace(
                s.query, '^[[:space:]]*CALL[[:space:]]+', '', 'i'
            ), '(', 1))) = '{tag}'"#
        ));
    }

    // SQL ORDER BY is built only from trusted literals, never user expressions.
    // The sort runs in PostgreSQL before LIMIT/OFFSET, across all matching entries.
    let order = match filters.sort.as_deref().unwrap_or("total_exec") {
        "queryid" => "s.queryid::bigint",
        "role" => "s.role_name",
        "total_exec" => "s.total_exec_time_ms",
        "mean_exec" => "s.mean_exec_time_ms",
        "calls" => "s.calls",
        "wal" => "s.wal_bytes",
        "temp" => "(s.temp_blks_read + s.temp_blks_written)",
        _ => return Err("PGSS Explorer: недопустимая сортировка".into()),
    };
    let direction = match filters.direction.as_deref().unwrap_or("desc") {
        "asc" => "ASC",
        "desc" => "DESC",
        _ => return Err("PGSS Explorer: недопустимое направление сортировки".into()),
    };
    let role_tiebreaker = if filters.sort.as_deref() == Some("role") {
        format!("s.database {direction} NULLS LAST, ")
    } else {
        String::new()
    };
    // When filtering by command or CALL target, server-side access to query text is
    // essential; otherwise use the no-text SRF to avoid even retrieving SQL bodies.
    let source = if filters.command.is_some() || filters.call_tag.is_some() {
        "pg_stat_statements AS s"
    } else {
        "pg_stat_statements(false) AS s"
    };
    let where_sql = if predicates.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", predicates.join(" AND "))
    };
    let sql = format!(
        r#"
WITH matched AS MATERIALIZED (
    SELECT s.userid::bigint AS userid, s.dbid::bigint AS dbid,
           r.rolname::text AS role_name, d.datname::text AS database,
           s.toplevel, s.queryid::text AS queryid, s.calls, s.plans,
           s.total_plan_time AS total_plan_time_ms,
           s.mean_plan_time AS mean_plan_time_ms,
           s.stddev_plan_time AS stddev_plan_time_ms,
           s.total_exec_time AS total_exec_time_ms,
           s.min_exec_time AS min_exec_time_ms,
           s.max_exec_time AS max_exec_time_ms,
           s.mean_exec_time AS mean_exec_time_ms,
           s.stddev_exec_time AS stddev_exec_time_ms,
           s.rows, s.shared_blks_hit, s.shared_blks_read,
           s.shared_blks_dirtied, s.shared_blks_written,
           s.temp_blks_read, s.temp_blks_written,
           s.blk_read_time AS blk_read_time_ms,
           s.blk_write_time AS blk_write_time_ms,
           s.wal_records, s.wal_fpi, s.wal_bytes::double precision AS wal_bytes
    FROM {source}
    LEFT JOIN pg_roles AS r ON r.oid = s.userid
    LEFT JOIN pg_database AS d ON d.oid = s.dbid
    {where_sql}
), paged AS (
    SELECT * FROM matched AS s
    ORDER BY {order} {direction} NULLS LAST, {role_tiebreaker}s.calls DESC,
             s.userid, s.dbid, s.queryid NULLS LAST, s.toplevel DESC
    LIMIT {next} OFFSET {offset}
)
SELECT json_build_object(
    'total', (SELECT count(*) FROM matched),
    'rows', COALESCE((SELECT json_agg(row_to_json(paged)) FROM paged), '[]'::json)
)::text;
"#,
        next = limit + 1
    );
    Ok((sql, offset, limit))
}

pub async fn query_detail(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    userid: u32,
    dbid: u32,
    queryid: String,
    toplevel: bool,
) -> Result<PgssQueryDetail, String> {
    let queryid = queryid
        .parse::<i64>()
        .map_err(|_| "Некорректный queryid".to_owned())?;

    let sql = detail_sql(userid, dbid, queryid, toplevel);

    let detail = query_json::<Option<PgssQueryDetail>>(
        internal_postgres,
        connection_id,
        input,
        "query_detail",
        &sql,
    )
    .await?;

    detail.ok_or_else(|| {
        crate::i18n::text("Statement entry больше не найдена в pg_stat_statements. Она могла быть deallocated или сброшена.", "Statement entry is no longer present in pg_stat_statements; it may have been deallocated or reset.")
            .to_owned()
    })
}

pub(super) fn detail_sql(userid: u32, dbid: u32, queryid: i64, toplevel: bool) -> String {
    format!(
        r#"
        SELECT COALESCE(
            (
                SELECT row_to_json(q)
                FROM (
                    SELECT
                        s.userid::bigint AS userid,
                        s.dbid::bigint AS dbid,
                        r.rolname::text AS role_name,
                        d.datname::text AS database,
                        s.toplevel,
                        s.queryid::text AS queryid,
                        left(s.query, 4000) AS query,
                        CASE WHEN s.query IS NULL THEN NULL
                             ELSE length(s.query) > 4000
                        END AS query_truncated,
                        s.calls,
                        s.plans,
                        s.total_plan_time AS total_plan_time_ms,
                        s.mean_plan_time AS mean_plan_time_ms,
                        s.stddev_plan_time AS stddev_plan_time_ms,
                        s.total_exec_time AS total_exec_time_ms,
                        s.min_exec_time AS min_exec_time_ms,
                        s.max_exec_time AS max_exec_time_ms,
                        s.mean_exec_time AS mean_exec_time_ms,
                        s.stddev_exec_time AS stddev_exec_time_ms,
                        s.rows,
                        s.shared_blks_hit,
                        s.shared_blks_read,
                        s.temp_blks_read,
                        s.temp_blks_written,
                        s.wal_bytes::double precision AS wal_bytes
                    FROM pg_stat_statements AS s
                    LEFT JOIN pg_roles AS r
                        ON r.oid = s.userid
                    LEFT JOIN pg_database AS d
                        ON d.oid = s.dbid
                    WHERE s.userid = {userid}::oid
                      AND s.dbid = {dbid}::oid
                      AND s.queryid = {queryid}::bigint
                      AND s.toplevel = {toplevel}
                    LIMIT 1
                ) AS q
            ),
            'null'::json
        )::text;
        "#,
        toplevel = if toplevel { "true" } else { "false" },
    )
}

pub(super) fn unavailable_section<T>(error: &str) -> PgssSection<T> {
    PgssSection {
        data: None,
        error: Some(error.to_owned()),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum EpochContinuity {
    Stable,
    Changed,
    Unverified,
}

pub(super) fn epoch_continuity(first: Option<&str>, last: Option<&str>) -> EpochContinuity {
    match (first, last) {
        (Some(a), Some(b)) if a == b => EpochContinuity::Stable,
        (Some(_), Some(_)) => EpochContinuity::Changed,
        _ => EpochContinuity::Unverified,
    }
}

pub(super) fn collection_budget_exhausted(started: Instant) -> bool {
    started.elapsed() >= PGSS_COLLECTION_BUDGET
}

async fn collect_section<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    label: &str,
    sql: &str,
    started: Instant,
) -> PgssSection<T>
where
    T: DeserializeOwned,
{
    if collection_budget_exhausted(started) {
        return unavailable_section(&format!(
            "PGSS {label}: секция пропущена — общий бюджет сбора исчерпан",
        ));
    }
    section_from_result(query_json::<T>(internal_postgres, connection_id, input, label, sql).await)
}

pub(super) fn section_from_result<T>(result: Result<T, String>) -> PgssSection<T> {
    match result {
        Ok(data) => PgssSection {
            data: Some(data),
            error: None,
        },
        Err(error) => unavailable_section(&error),
    }
}

pub(super) fn parse_json<T: DeserializeOwned>(label: &str, output: &str) -> Result<T, String> {
    serde_json::from_str(output)
        .map_err(|error| format!("PGSS {label}: PostgreSQL вернул неожиданный JSON: {error}"))
}

async fn query_json<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    label: &str,
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
        .await
        .map_err(|error| format!("PGSS {label}: {error}"))?;

    parse_json(label, &output)
}
