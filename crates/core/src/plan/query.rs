use crate::{
    internal_postgres::{run_action_once, InternalPostgresRegistry},
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use pg_query::NodeRef;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_SQL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct ReadonlyQueryInfo {
    pub fingerprint: String,
    pub tables: Vec<String>,
    pub functions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallActionInfo {
    pub fingerprint: String,
    pub functions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallActionExecution {
    pub fingerprint: String,
    pub functions: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResultComparison {
    pub equivalent: bool,
    pub baseline_rows: i64,
    pub candidate_rows: i64,
    pub baseline_only_rows: i64,
    pub candidate_only_rows: i64,
    pub baseline_only_sample: Value,
    pub candidate_only_sample: Value,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
struct RawResultComparison {
    baseline_rows: i64,
    candidate_rows: i64,
    baseline_only_rows: i64,
    candidate_only_rows: i64,
    baseline_only_sample: Value,
    candidate_only_sample: Value,
}

pub fn validate_readonly_select(sql: &str) -> Result<ReadonlyQueryInfo, String> {
    let sql = validate_sql_text(sql)?;

    let statements = pg_query::split_with_parser(sql)
        .map_err(|error| format!("PostgreSQL parser отклонил SQL: {error}"))?;

    if statements.len() != 1 {
        return Err(crate::i18n::text(
            "Для сравнения разрешён ровно один SQL statement",
            "Exactly one SQL statement is allowed for comparison",
        )
        .into());
    }

    let parsed =
        pg_query::parse(sql).map_err(|error| format!("PostgreSQL parser отклонил SQL: {error}"))?;

    let nodes = parsed.protobuf.nodes();
    let Some((first, _, _, _)) = nodes.first() else {
        return Err(crate::i18n::text(
            "PostgreSQL parser не вернул statement",
            "The PostgreSQL parser returned no statement",
        )
        .into());
    };

    let NodeRef::SelectStmt(select) = first else {
        return Err(crate::i18n::text(
            "Разрешён только read-only SELECT/WITH ... SELECT",
            "Only read-only SELECT/WITH ... SELECT is allowed",
        )
        .into());
    };

    if select.into_clause.is_some() {
        return Err(crate::i18n::text(
            "SELECT INTO запрещён: он создаёт/изменяет объект",
            "SELECT INTO is forbidden because it creates or modifies an object",
        )
        .into());
    }

    if !select.locking_clause.is_empty() {
        return Err(crate::i18n::text(
            "SELECT ... FOR UPDATE/SHARE не используется в comparison workspace",
            "SELECT ... FOR UPDATE/SHARE is not allowed in the comparison workspace",
        )
        .into());
    }

    for (node, _, _, _) in &nodes {
        if matches!(
            node,
            NodeRef::InsertStmt(_)
                | NodeRef::UpdateStmt(_)
                | NodeRef::DeleteStmt(_)
                | NodeRef::MergeStmt(_)
                | NodeRef::CallStmt(_)
                | NodeRef::DoStmt(_)
                | NodeRef::CopyStmt(_)
                | NodeRef::CreateTableAsStmt(_)
        ) {
            return Err(
                "Запрос содержит writable/utility statement внутри SELECT/CTE и отклонён".into(),
            );
        }
    }

    let fingerprint = pg_query::fingerprint(sql)
        .map_err(|error| format!("Не удалось fingerprint SQL: {error}"))?
        .hex;

    Ok(ReadonlyQueryInfo {
        fingerprint,
        tables: parsed.tables(),
        functions: parsed.functions(),
    })
}

pub fn validate_call_action(sql: &str) -> Result<CallActionInfo, String> {
    let sql = validate_sql_text(sql)?;

    let statements = pg_query::split_with_parser(sql)
        .map_err(|error| format!("PostgreSQL parser отклонил SQL: {error}"))?;

    if statements.len() != 1 {
        return Err("Для comparison action разрешён ровно один SQL statement".into());
    }

    let parsed =
        pg_query::parse(sql).map_err(|error| format!("PostgreSQL parser отклонил SQL: {error}"))?;

    let nodes = parsed.protobuf.nodes();
    let Some((first, _, _, _)) = nodes.first() else {
        return Err(crate::i18n::text(
            "PostgreSQL parser не вернул statement",
            "The PostgreSQL parser returned no statement",
        )
        .into());
    };

    if !matches!(first, NodeRef::CallStmt(_)) {
        return Err("Для comparison action сейчас разрешён только CALL procedure(...)".into());
    }

    let fingerprint = pg_query::fingerprint(sql)
        .map_err(|error| format!("Не удалось fingerprint SQL: {error}"))?
        .hex;

    Ok(CallActionInfo {
        fingerprint,
        functions: parsed.functions(),
    })
}

pub async fn execute_call_action(
    input: &ConnectionInput,
    sql: &str,
    timeout_ms: u64,
) -> Result<CallActionExecution, String> {
    let info = validate_call_action(sql)?;

    run_action_once(
        input,
        trim_statement(sql),
        QueryExecutionPolicy::explicit_heavy_query(timeout_ms, "pgxray-compare-action"),
    )
    .await?;

    Ok(CallActionExecution {
        fingerprint: info.fingerprint,
        functions: info.functions,
        warnings: vec![
            "CALL реально выполнен в PostgreSQL и может изменить данные.".into(),
            "pgXray не откатывает изменения автоматически: comparison workspace должен явно управлять baseline/candidate состоянием базы.".into(),
            "Результат CALL сам по себе не доказывает эквивалентность данных; для этого используйте отдельный read-only verification SELECT.".into(),
        ],
    })
}

pub async fn explain_query(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    sql: &str,
    analyze: bool,
    timeout_ms: u64,
) -> Result<String, String> {
    validate_readonly_select(sql)?;

    let sql = trim_statement(sql);
    let explain = if analyze {
        format!("EXPLAIN (ANALYZE, BUFFERS, VERBOSE, WAL, SETTINGS, TIMING, SUMMARY, FORMAT JSON) {sql}")
    } else {
        format!("EXPLAIN (VERBOSE, COSTS ON, SETTINGS ON, FORMAT JSON) {sql}")
    };

    internal_postgres
        .run_readonly_query(
            connection_id,
            input,
            &explain,
            QueryExecutionPolicy::explicit_heavy_query(timeout_ms, "pgxray-explain"),
        )
        .await
}

pub async fn compare_results(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    baseline_sql: &str,
    candidate_sql: &str,
    timeout_ms: u64,
) -> Result<QueryResultComparison, String> {
    validate_readonly_select(baseline_sql)?;
    validate_readonly_select(candidate_sql)?;

    let baseline = trim_statement(baseline_sql);
    let candidate = trim_statement(candidate_sql);

    let comparison_sql = format!(
        r#"
WITH
baseline AS MATERIALIZED (
{baseline}
),
candidate AS MATERIALIZED (
{candidate}
),
baseline_only AS MATERIALIZED (
    SELECT * FROM baseline
    EXCEPT ALL
    SELECT * FROM candidate
),
candidate_only AS MATERIALIZED (
    SELECT * FROM candidate
    EXCEPT ALL
    SELECT * FROM baseline
)
SELECT json_build_object(
    'baseline_rows', (SELECT count(*) FROM baseline),
    'candidate_rows', (SELECT count(*) FROM candidate),
    'baseline_only_rows', (SELECT count(*) FROM baseline_only),
    'candidate_only_rows', (SELECT count(*) FROM candidate_only),
    'baseline_only_sample', COALESCE(
        (
            SELECT json_agg(row_to_json(sample))
            FROM (
                SELECT *
                FROM baseline_only
                LIMIT 20
            ) AS sample
        ),
        '[]'::json
    ),
    'candidate_only_sample', COALESCE(
        (
            SELECT json_agg(row_to_json(sample))
            FROM (
                SELECT *
                FROM candidate_only
                LIMIT 20
            ) AS sample
        ),
        '[]'::json
    )
)::text;
"#,
    );

    let output = internal_postgres
        .run_readonly_query(
            connection_id,
            input,
            &comparison_sql,
            QueryExecutionPolicy::explicit_heavy_query(timeout_ms, "pgxray-compare"),
        )
        .await?;
    let raw: RawResultComparison = serde_json::from_str(output.trim()).map_err(|error| {
        format!("Не удалось разобрать результат EXCEPT ALL comparison: {error}")
    })?;

    let equivalent = raw.baseline_only_rows == 0
        && raw.candidate_only_rows == 0
        && raw.baseline_rows == raw.candidate_rows;

    let mut warnings = vec![
        "Сравнение выполняет оба SELECT и EXCEPT ALL в одном statement/snapshot. Для тяжёлых production queries это реальная нагрузка.".into(),
        "EXCEPT ALL сравнивает весь row shape и сохраняет multiplicity; обе стороны должны иметь union-compatible column types.".into(),
    ];

    if !equivalent {
        warnings.push(
            "Результаты различаются. Не переходите к performance rewrite как к эквивалентному без разбора sample differences."
                .into(),
        );
    }

    Ok(QueryResultComparison {
        equivalent,
        baseline_rows: raw.baseline_rows,
        candidate_rows: raw.candidate_rows,
        baseline_only_rows: raw.baseline_only_rows,
        candidate_only_rows: raw.candidate_only_rows,
        baseline_only_sample: raw.baseline_only_sample,
        candidate_only_sample: raw.candidate_only_sample,
        warnings,
    })
}

fn validate_sql_text(sql: &str) -> Result<&str, String> {
    let sql = sql.trim();

    if sql.is_empty() {
        return Err("SQL пуст".into());
    }

    if sql.len() > MAX_SQL_BYTES {
        return Err(format!(
            "SQL слишком большой: {} bytes; максимум {MAX_SQL_BYTES}",
            sql.len(),
        ));
    }

    if sql.contains('\0') {
        return Err("SQL содержит NUL byte".into());
    }

    Ok(sql)
}

fn trim_statement(sql: &str) -> &str {
    sql.trim().trim_end_matches(';').trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_select_and_rejects_update() {
        assert!(validate_readonly_select("SELECT * FROM public.orders").is_ok());
        assert!(validate_readonly_select("UPDATE public.orders SET x = 1").is_err());
    }

    #[test]
    fn rejects_writable_cte() {
        assert!(validate_readonly_select(
            "WITH deleted AS (DELETE FROM public.orders RETURNING *) SELECT * FROM deleted"
        )
        .is_err());
    }

    #[test]
    fn accepts_call_action() {
        assert!(validate_call_action("CALL public.refresh_orders()").is_ok());
    }

    #[test]
    fn call_action_rejects_select_and_multiple_statements() {
        assert!(validate_call_action("SELECT 1").is_err());
        assert!(validate_call_action("CALL a(); CALL b();").is_err());
    }

    #[test]
    fn readonly_comparison_still_rejects_call() {
        assert!(validate_readonly_select("CALL public.refresh_orders()").is_err());
    }
}
