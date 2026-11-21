use crate::{
    internal_postgres::InternalPostgresRegistry,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::de::DeserializeOwned;

use super::{
    interpretation::{interpret_snapshot, interpret_table},
    model::*,
    policy::fixed_scale_candidates,
    queries::*,
    query_relation::{identify_update_targets, PgssUpdateCandidate},
};

pub async fn collect(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
) -> Result<AutovacuumSnapshot, String> {
    let overview =
        query_json::<AutovacuumOverview>(internal_postgres, connection_id, input, OVERVIEW_SQL)
            .await
            .map_err(|error| {
                explain_section_error(
                    crate::i18n::text("Общие настройки Autovacuum", "General Autovacuum settings"),
                    &error,
                )
            })?;

    let tables = collect_section::<Vec<AutovacuumTable>>(
        internal_postgres,
        connection_id,
        input,
        TABLES_SQL,
        crate::i18n::text("Таблицы", "Tables"),
    )
    .await;

    let vacuums = collect_section::<Vec<AutovacuumVacuumProgress>>(
        internal_postgres,
        connection_id,
        input,
        VACUUM_PROGRESS_SQL,
        crate::i18n::text("Активные VACUUM", "Active VACUUM"),
    )
    .await;

    let analyzes = collect_section::<Vec<AutovacuumAnalyzeProgress>>(
        internal_postgres,
        connection_id,
        input,
        ANALYZE_PROGRESS_SQL,
        crate::i18n::text("Активные ANALYZE", "Active ANALYZE"),
    )
    .await;

    let interpretation = interpret_snapshot(&overview, &tables, &vacuums, &analyzes);

    Ok(AutovacuumSnapshot {
        overview,
        tables,
        vacuums,
        analyzes,
        interpretation,
    })
}

pub async fn table_detail(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    relation_oid: u32,
) -> Result<AutovacuumTableDetail, String> {
    let sql = table_sql(relation_oid)?;
    let tables = query_json::<Vec<AutovacuumTable>>(internal_postgres, connection_id, input, &sql)
        .await
        .map_err(|error| {
            explain_section_error(
                crate::i18n::text("Детализация таблицы", "Table details"),
                &error,
            )
        })?;

    let table = tables
        .into_iter()
        .find(|row| row.relation_oid == relation_oid)
        .ok_or_else(|| format!("Relation OID {relation_oid} не найдена среди user tables"))?;

    let indexes = collect_section::<Vec<AutovacuumIndex>>(
        internal_postgres,
        connection_id,
        input,
        &indexes_sql(relation_oid),
        crate::i18n::text("Индексы таблицы", "Table indexes"),
    )
    .await;

    // Distinguish a missing extension from a failed availability check.
    let update_statements = match query_json::<bool>(
        internal_postgres,
        connection_id,
        input,
        "SELECT to_json(EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements'))::text;",
    )
    .await
    {
        Ok(true) => {
            let raw = collect_section::<Vec<PgssUpdateCandidate>>(
                internal_postgres, connection_id, input,
                &update_statements_sql(relation_oid), "UPDATE (pg_stat_statements)",
            ).await;
            match raw.data {
                Some(candidates) => AutovacuumSection::success(identify_update_targets(
                    candidates, relation_oid, &table.schema_name, &table.relation_name,
                )),
                None => AutovacuumSection::failure(raw.error.unwrap_or_else(|| "PGSS candidates unavailable".into())),
            }
        }
        Ok(false) => unavailable_section(
            "[AV-COL-PGSS-ABSENT] pg_stat_statements не установлено в этой базе. \
             Это необязательный раздел; остальные данные таблицы доступны.",
        ),
        Err(error) => unavailable_section(explain_section_error(
            crate::i18n::text("Проверка pg_stat_statements", "pg_stat_statements check"),
            &error,
        )),
    };

    // Table detail does not carry AutovacuumOverview. Check track_counts
    // independently so unknown or disabled statistics cannot create confident
    // per-table threshold/HOT findings. Keep table detail available if this
    // cheap read-only check fails, but report the limitation explicitly.
    let track_counts = query_json::<bool>(
        internal_postgres,
        connection_id,
        input,
        "SELECT to_json(current_setting('track_counts') = 'on')::text;",
    )
    .await;
    let statistics_available = track_counts.as_ref().ok().copied() == Some(true);
    let fixed_scale_candidates = if statistics_available {
        fixed_scale_candidates(&table)
    } else {
        Vec::new()
    };
    let mut interpretation = interpret_table(&table, &indexes, track_counts.as_ref().ok().copied());
    if let Err(error) = track_counts {
        interpretation.limitations.push(explain_section_error(
            crate::i18n::text(
                "Проверка track_counts для детализации таблицы",
                "track_counts check for table details",
            ),
            &error,
        ));
    }

    Ok(AutovacuumTableDetail {
        table,
        indexes,
        update_statements,
        fixed_scale_candidates,
        interpretation,
    })
}

fn unavailable_section<T>(error: impl Into<String>) -> AutovacuumSection<T> {
    AutovacuumSection::failure(error)
}

async fn collect_section<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    sql: &str,
    section_name: &str,
) -> AutovacuumSection<T>
where
    T: DeserializeOwned,
{
    match query_json::<T>(internal_postgres, connection_id, input, sql).await {
        Ok(data) => AutovacuumSection::success(data),
        Err(error) => unavailable_section(explain_section_error(section_name, &error)),
    }
}

pub(super) async fn query_json<T>(
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
        .map_err(|error| format!("PostgreSQL вернул неожиданный Autovacuum JSON: {error}"))
}

// The collection is intentionally bounded. Provide actionable reasons while
// retaining the original database error for troubleshooting. No automatic
// timeout/output-limit escalation or telemetry upload happens here.
pub(super) fn explain_section_error(section: &str, error: &str) -> String {
    let lower = error.to_lowercase();
    let (code, explanation, next_step) = if lower.contains("output слишком большой")
        || lower.contains("output too large")
    {
        (
            "AV-COL-OUTPUT-LIMIT",
            crate::i18n::text("Ответ PostgreSQL превысил ограничение размера (32 МиБ). Секция не собрана; нулевые значения не означают отсутствие проблем.", "PostgreSQL response exceeded the 32 MiB size limit. The section was not collected; zero values do not mean that no issues exist."),
            "Большое количество таблиц может увеличить ответ. Повтори сбор при необходимости; для проблемной таблицы используй «Подробнее». Если ограничение повторяется, создай Issue в репозитории pgXray с версией PostgreSQL и этим кодом ошибки.",
        )
    } else if lower.contains("timeout")
        || lower.contains("57014")
        || lower.contains("превышено время ожидания")
        || lower.contains("statement_timeout")
        || lower.contains("lock_timeout")
        || lower.contains("55p03")
    {
        (
            "AV-COL-TIMEOUT",
            "Запрос диагностики прерван или отменён (например, по таймауту либо из-за ожидания блокировки). Секция не собрана; это не означает, что база неисправна.",
            "Попробуй позже при меньшей нагрузке и проверь блокировки PostgreSQL. Большое число объектов тоже может увеличивать время сбора. Если ошибка повторяется, создай Issue в репозитории pgXray, указав версию PostgreSQL и этот код.",
        )
    } else {
        (
            "AV-COL-QUERY-ERROR",
            crate::i18n::text("Не удалось получить этот раздел диагностики. Остальные разделы снимка могут быть доступны.", "Unable to collect this diagnostic section. Other snapshot sections may still be available."),
            "Проверь доступы диагностической роли и повтори сбор. Если ошибка повторяется, создай Issue в репозитории pgXray с версией PostgreSQL и кодом ошибки.",
        )
    };
    format!(
        "[{code}] {section}: {explanation} Что сделать: {next_step} Техническая причина: {error}"
    )
}
