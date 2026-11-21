use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{
    internal_postgres::InternalPostgresRegistry,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::de::DeserializeOwned;

use super::{interpretation::interpret_snapshot, model::*, queries::*};

// Maximum time for the complete eight-query diagnostic, not for each query.
pub(super) const XID_COLLECTION_BUDGET: Duration = Duration::from_secs(60);
pub(super) const XID_PAGE_SIZE: usize = 256;
// Full snapshots are kept in SQLite and UI memory; reject excessive results
// instead of silently storing a truncated relation list as complete.
pub(super) const XID_MAX_SECTION_ROWS: usize = 25_000;
const RELATION_PAGE_MARKER: &str = "/* pgxray:xid-page-relations */";
const TRIGGER_PAGE_MARKER: &str = "/* pgxray:xid-page-triggers */";

trait XidPagedRelation {
    fn oid(&self) -> u32;
    fn xid_age(&self) -> i64;
}

impl XidPagedRelation for XidRelationAge {
    fn oid(&self) -> u32 {
        self.relation_oid
    }
    fn xid_age(&self) -> i64 {
        self.xid_age
    }
}

impl XidPagedRelation for XidAutovacuumTrigger {
    fn oid(&self) -> u32 {
        self.relation_oid
    }
    fn xid_age(&self) -> i64 {
        self.xid_age
    }
}

pub(super) fn paged_sql(sql: &str, marker: &str, last_oid: u32) -> Result<String, String> {
    if sql.matches(marker).count() != 1 {
        return Err(format!(
            "Отсутствует или повторяется SQL marker для XID keyset page: {marker}"
        ));
    }
    let clause = format!("AND c.oid > {last_oid}::oid ORDER BY c.oid LIMIT {XID_PAGE_SIZE}");
    Ok(sql.replacen(marker, &clause, 1))
}

#[derive(Clone, Copy)]
pub(super) struct XidSqlSet {
    pub databases: &'static str,
    pub relations: &'static str,
    pub triggers: &'static str,
    pub backends: &'static str,
    pub slots: &'static str,
    pub prepared: &'static str,
    pub vacuums: &'static str,
}

impl Default for XidSqlSet {
    fn default() -> Self {
        Self {
            databases: DATABASES_SQL,
            relations: RELATIONS_SQL,
            triggers: AUTOVACUUM_TRIGGERS_SQL,
            backends: BACKENDS_SQL,
            slots: SLOTS_SQL,
            prepared: PREPARED_SQL,
            vacuums: VACUUMS_SQL,
        }
    }
}

pub async fn collect(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
) -> Result<XidSnapshot, String> {
    collect_with_queries(
        internal_postgres,
        connection_id,
        input,
        XidSqlSet::default(),
    )
    .await
}

// Allows an isolated invalid/permission-denied section in an integration test
// without altering production SQL or relying on a particular server role.
pub(super) async fn collect_with_queries(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    sql: XidSqlSet,
) -> Result<XidSnapshot, String> {
    let started = Instant::now();
    let overview = query_json::<XidOverview>(internal_postgres, connection_id, input, OVERVIEW_SQL)
        .await
        .map_err(|error| explain_section_error("overview", &error))?;

    let databases = collect_section::<Vec<XidDatabaseAge>>(
        internal_postgres,
        connection_id,
        input,
        "databases",
        sql.databases,
        started,
    )
    .await;
    let relations = collect_paged_section::<XidRelationAge>(
        internal_postgres,
        connection_id,
        input,
        "relations",
        sql.relations,
        RELATION_PAGE_MARKER,
        started,
    )
    .await;
    let autovacuum_triggers = collect_paged_section::<XidAutovacuumTrigger>(
        internal_postgres,
        connection_id,
        input,
        "autovacuum_triggers",
        sql.triggers,
        TRIGGER_PAGE_MARKER,
        started,
    )
    .await;
    let backends = collect_section::<Vec<XidBackend>>(
        internal_postgres,
        connection_id,
        input,
        "backends",
        sql.backends,
        started,
    )
    .await;
    let slots = collect_section::<Vec<XidSlot>>(
        internal_postgres,
        connection_id,
        input,
        "slots",
        sql.slots,
        started,
    )
    .await;
    let prepared = collect_section::<XidPreparedSummary>(
        internal_postgres,
        connection_id,
        input,
        "prepared",
        sql.prepared,
        started,
    )
    .await;
    let vacuums = collect_section::<Vec<XidVacuum>>(
        internal_postgres,
        connection_id,
        input,
        "vacuums",
        sql.vacuums,
        started,
    )
    .await;

    let mut interpretation = interpret_snapshot(
        &overview,
        &relations,
        &autovacuum_triggers,
        &vacuums,
        &backends,
        &slots,
        &prepared,
    );
    if let Some(error) = &databases.error {
        interpretation.limitations.push(if crate::i18n::is_ru() {
            format!("Возраст баз данных недоступен: {error}")
        } else {
            format!("Database age is unavailable: {error}")
        });
    }
    interpretation.limitations.push(
        crate::i18n::text("Снимок XID составлен из последовательных read-only SQL-запросов; секции могут относиться к разным моментам времени. observed_at_ms — время завершения запроса на компьютере pgXray.", "The XID snapshot consists of sequential read-only SQL queries; sections may reflect different moments. observed_at_ms is the query completion time on the pgXray computer.").into(),
    );

    Ok(XidSnapshot {
        overview,
        databases,
        relations,
        autovacuum_triggers,
        vacuums,
        backends,
        slots,
        prepared,
        interpretation,
    })
}

// Read a complete relation set in stable OID order. Each SQL result is bounded,
// and no partially fetched section is exposed as a successful snapshot.
async fn collect_paged_section<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    section: &str,
    sql: &str,
    marker: &str,
    started: Instant,
) -> XidSection<Vec<T>>
where
    T: DeserializeOwned + XidPagedRelation,
{
    let mut all = Vec::new();
    let mut last_oid = 0_u32;
    loop {
        let Some(remaining) = XID_COLLECTION_BUDGET
            .checked_sub(started.elapsed())
            .filter(|remaining| *remaining >= Duration::from_secs(1))
        else {
            return XidSection::failure(
                explain_section_error(section, &format!(
                    "общий лимит времени сбора XID исчерпан после {} relations; неполная выборка отброшена",
                    all.len()
                )),
                client_now_ms(),
            );
        };
        let page_sql = match paged_sql(sql, marker, last_oid) {
            Ok(sql) => sql,
            Err(error) => {
                return XidSection::failure(explain_section_error(section, &error), client_now_ms())
            }
        };
        let policy = limited_query_policy(remaining);
        let page: Vec<T> = match query_json_with_policy(
            internal_postgres, connection_id, input, &page_sql, policy,
        ).await {
            Ok(rows) => rows,
            Err(error) => return XidSection::failure(
                explain_section_error(section, &format!(
                    "ошибка получения страницы после OID {last_oid}; {} строк не считаются полным снимком: {error}",
                    all.len()
                )),
                client_now_ms(),
            ),
        };
        let page_len = page.len();
        if page_len > XID_PAGE_SIZE {
            return XidSection::failure(
                explain_section_error(section, "SQL вернул больше строк, чем размер страницы"),
                client_now_ms(),
            );
        }
        let new_last_oid = page.iter().map(XidPagedRelation::oid).max();
        if new_last_oid.is_some_and(|oid| oid <= last_oid) {
            return XidSection::failure(
                explain_section_error(section, "Нарушен порядок OID при порционной загрузке XID"),
                client_now_ms(),
            );
        }
        all.extend(page);
        if all.len() > XID_MAX_SECTION_ROWS {
            return XidSection::failure(
                explain_section_error(section, &format!(
                    "слишком большой набор relations (> {XID_MAX_SECTION_ROWS}); неполный результат не сохранён"
                )),
                client_now_ms(),
            );
        }
        if page_len < XID_PAGE_SIZE {
            break;
        }
        last_oid = new_last_oid.expect("full XID page contains an OID");
    }
    all.sort_by(|left, right| {
        right
            .xid_age()
            .cmp(&left.xid_age())
            .then_with(|| left.oid().cmp(&right.oid()))
    });
    XidSection::success(all, client_now_ms())
}

fn limited_query_policy(remaining: Duration) -> QueryExecutionPolicy {
    let mut policy = QueryExecutionPolicy::standard_diagnostic();
    policy.overall_timeout_ms = policy
        .overall_timeout_ms
        .min(u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX));
    policy.statement_timeout_ms = policy
        .statement_timeout_ms
        .min(policy.overall_timeout_ms.saturating_sub(200).max(1));
    policy.lock_timeout_ms = policy.lock_timeout_ms.min(policy.statement_timeout_ms);
    policy
}

fn client_now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

async fn collect_section<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    section: &str,
    sql: &str,
    started: Instant,
) -> XidSection<T>
where
    T: DeserializeOwned,
{
    let Some(remaining) = XID_COLLECTION_BUDGET
        .checked_sub(started.elapsed())
        .filter(|remaining| *remaining >= Duration::from_secs(1))
    else {
        return XidSection::failure(
            explain_section_error(section, "общий лимит времени сбора XID исчерпан"),
            None,
        );
    };
    // Use the internal executor's own timeout and cancellation/cleanup protocol.
    // Dropping an outer timeout future could leave a PostgreSQL session mid-query.
    let policy = limited_query_policy(remaining);
    let result =
        query_json_with_policy::<T>(internal_postgres, connection_id, input, sql, policy).await;
    let observed_at_ms = client_now_ms();
    match result {
        Ok(data) => XidSection::success(data, observed_at_ms),
        Err(error) => XidSection::failure(explain_section_error(section, &error), observed_at_ms),
    }
}

pub(super) fn explain_section_error(section: &str, error: &str) -> String {
    let message = error.to_ascii_lowercase();
    let (code, guidance) = if message.contains("лимит времени")
        || message.contains("timeout")
        || message.contains("timed out")
        || message.contains("statement timeout")
        || message.contains("canceling statement due to statement timeout")
    {
        ("XID-COL-TIMEOUT", crate::i18n::text("Запрос превысил лимит времени. Попробуй повторить сбор позже или уменьшить нагрузку на сервер.", "The query exceeded the time limit. Retry later or reduce server load."))
    } else if message.contains("too large")
        || message.contains("слишком большой")
        || message.contains("максимальный размер")
        || message.contains("максимум")
        || message.contains("output limit")
    {
        ("XID-COL-OUTPUT-LIMIT", crate::i18n::text("Ответ превышает допустимый размер. Это не означает отсутствие проблем; понадобится ограниченный сбор для большой базы.", "The response exceeded the allowed size. This does not mean there are no problems; a bounded collection is needed for a large database."))
    } else {
        ("XID-COL-QUERY-ERROR", crate::i18n::text("Секцию получить не удалось. Проверь права доступа и доступность PostgreSQL; попробуй повторить сбор позже.", "The section could not be collected. Check permissions and PostgreSQL availability, then retry later."))
    };
    if crate::i18n::is_ru() {
        format!("[{code}/{section}] {guidance} Техническая причина: {error}")
    } else {
        format!("[{code}/{section}] {guidance} Technical cause: {error}")
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
    query_json_with_policy(
        internal_postgres,
        connection_id,
        input,
        sql,
        QueryExecutionPolicy::standard_diagnostic(),
    )
    .await
}

async fn query_json_with_policy<T>(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    sql: &str,
    policy: QueryExecutionPolicy,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let output = internal_postgres
        .run_readonly_query(connection_id, input, sql, policy)
        .await?;

    serde_json::from_str(&output)
        .map_err(|error| format!("Ответ SQL не соответствует ожидаемой Rust-модели JSON: {error}"))
}
