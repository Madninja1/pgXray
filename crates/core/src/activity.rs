use crate::{
    history,
    internal_postgres::{run_action_once_output, run_readonly_once},
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::{Deserialize, Serialize};

const ACTIVITY_SQL: &str = r#"
SELECT COALESCE(
    json_agg(activity_row ORDER BY active_rank, query_start_sort NULLS LAST, pid_sort),
    '[]'::json
)::text
FROM (
    SELECT
        json_build_object(
            'pid', a.pid,
            'database', a.datname,
            'username', a.usename,
            'application_name', COALESCE(a.application_name, ''),
            'client_addr', a.client_addr::text,
            'state', a.state,
            'backend_start_ms', CASE
                WHEN a.backend_start IS NULL THEN NULL
                ELSE (extract(epoch FROM a.backend_start) * 1000)::bigint
            END,
            'xact_start_ms', CASE
                WHEN a.xact_start IS NULL THEN NULL
                ELSE (extract(epoch FROM a.xact_start) * 1000)::bigint
            END,
            'query_start_ms', CASE
                WHEN a.query_start IS NULL THEN NULL
                ELSE (extract(epoch FROM a.query_start) * 1000)::bigint
            END,
            'state_change_ms', CASE
                WHEN a.state_change IS NULL THEN NULL
                ELSE (extract(epoch FROM a.state_change) * 1000)::bigint
            END,
            'query_age_seconds', CASE
                WHEN a.state <> 'active' OR a.query_start IS NULL THEN NULL
                ELSE extract(epoch FROM (clock_timestamp() - a.query_start))
            END,
            'xact_age_seconds', CASE
                WHEN a.xact_start IS NULL THEN NULL
                ELSE extract(epoch FROM (clock_timestamp() - a.xact_start))
            END,
            'wait_event_type', row_to_json(a)::jsonb ->> 'wait_event_type',
            'wait_event', row_to_json(a)::jsonb ->> 'wait_event',
            'backend_type', row_to_json(a)::jsonb ->> 'backend_type',
            'query', a.query,
            'is_pgxray', COALESCE(a.application_name, '') LIKE 'pgxray-%'
        ) AS activity_row,
        CASE WHEN a.state = 'active' THEN 0 ELSE 1 END AS active_rank,
        a.query_start AS query_start_sort,
        a.pid AS pid_sort
    FROM pg_stat_activity AS a
    WHERE a.pid <> pg_backend_pid()
      AND COALESCE(row_to_json(a)::jsonb ->> 'backend_type', 'client backend') = 'client backend'
) AS activity_rows;
"#;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ActivityBackend {
    pub pid: i32,
    pub database: Option<String>,
    pub username: Option<String>,
    pub application_name: String,
    pub client_addr: Option<String>,
    pub state: Option<String>,
    pub backend_start_ms: Option<i64>,
    pub xact_start_ms: Option<i64>,
    pub query_start_ms: Option<i64>,
    pub state_change_ms: Option<i64>,
    pub query_age_seconds: Option<f64>,
    pub xact_age_seconds: Option<f64>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    pub backend_type: Option<String>,
    pub query: Option<String>,
    pub is_pgxray: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActivitySnapshot {
    pub captured_at_ms: i64,
    pub total_connections: usize,
    pub active_connections: usize,
    pub idle_connections: usize,
    pub idle_in_transaction_connections: usize,
    pub waiting_active_connections: usize,
    pub pgxray_connections: usize,
    pub backends: Vec<ActivityBackend>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendSignalAction {
    Cancel,
    Terminate,
}

impl BackendSignalAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Terminate => "terminate",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BackendSignalResult {
    pub pid: i32,
    pub backend_start_ms: i64,
    pub action: BackendSignalAction,
    pub signaled: bool,
}

pub async fn snapshot(input: &ConnectionInput) -> Result<ActivitySnapshot, String> {
    let output = run_readonly_once(
        input,
        ACTIVITY_SQL,
        QueryExecutionPolicy::activity_snapshot(),
    )
    .await?;

    let backends = parse_activity_output(&output)?;
    Ok(build_snapshot(backends, history::unix_time_millis()?))
}

pub async fn signal_backend(
    input: &ConnectionInput,
    pid: i32,
    backend_start_ms: i64,
    action: BackendSignalAction,
) -> Result<BackendSignalResult, String> {
    if pid <= 0 {
        return Err(crate::i18n::text(
            "Некорректный PostgreSQL backend PID",
            "Invalid PostgreSQL backend PID",
        )
        .into());
    }
    if backend_start_ms <= 0 {
        return Err(crate::i18n::text(
            "Некорректный PostgreSQL backend_start для backend signal",
            "Invalid PostgreSQL backend start time for backend signaling",
        )
        .into());
    }

    let sql = backend_signal_sql(pid, backend_start_ms, action);
    let output =
        run_action_once_output(input, &sql, QueryExecutionPolicy::backend_signal()).await?;
    let signaled = parse_signal_output(&output)?;

    Ok(BackendSignalResult {
        pid,
        backend_start_ms,
        action,
        signaled,
    })
}

fn backend_signal_sql(pid: i32, backend_start_ms: i64, action: BackendSignalAction) -> String {
    let function = match action {
        BackendSignalAction::Cancel => "pg_cancel_backend",
        BackendSignalAction::Terminate => "pg_terminate_backend",
    };

    format!(
        "SELECT CASE WHEN EXISTS (\
             SELECT 1 FROM pg_stat_activity AS a \
             WHERE a.pid = {pid} \
               AND (extract(epoch FROM a.backend_start) * 1000)::bigint = {backend_start_ms} \
               AND COALESCE(row_to_json(a)::jsonb ->> 'backend_type', 'client backend') = 'client backend'\
         ) THEN {function}({pid}) ELSE false END;"
    )
}

fn parse_activity_output(output: &str) -> Result<Vec<ActivityBackend>, String> {
    serde_json::from_str::<Vec<ActivityBackend>>(output.trim())
        .map_err(|error| format!("Не удалось разобрать pg_stat_activity snapshot: {error}"))
}

fn build_snapshot(backends: Vec<ActivityBackend>, captured_at_ms: i64) -> ActivitySnapshot {
    let active_connections = backends
        .iter()
        .filter(|backend| backend.state.as_deref() == Some("active"))
        .count();
    let idle_connections = backends
        .iter()
        .filter(|backend| backend.state.as_deref() == Some("idle"))
        .count();
    let idle_in_transaction_connections = backends
        .iter()
        .filter(|backend| {
            backend
                .state
                .as_deref()
                .is_some_and(|state| state.starts_with("idle in transaction"))
        })
        .count();
    let waiting_active_connections = backends
        .iter()
        .filter(|backend| {
            backend.state.as_deref() == Some("active") && backend.wait_event_type.is_some()
        })
        .count();
    let pgxray_connections = backends.iter().filter(|backend| backend.is_pgxray).count();

    ActivitySnapshot {
        captured_at_ms,
        total_connections: backends.len(),
        active_connections,
        idle_connections,
        idle_in_transaction_connections,
        waiting_active_connections,
        pgxray_connections,
        backends,
    }
}

fn parse_signal_output(output: &str) -> Result<bool, String> {
    match output.trim() {
        "t" => Ok(true),
        "f" => Ok(false),
        value => Err(format!(
            "PostgreSQL вернул неожиданный результат backend signal: {value:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_activity_snapshot_and_builds_summary() {
        let output = r#"[{"pid":101,"database":"app","username":"alice","application_name":"api","client_addr":"10.0.0.1","state":"active","backend_start_ms":1000,"xact_start_ms":2000,"query_start_ms":3000,"state_change_ms":3000,"query_age_seconds":2.5,"xact_age_seconds":3.5,"wait_event_type":"Lock","wait_event":"transactionid","backend_type":"client backend","query":"select 1","is_pgxray":false},{"pid":102,"database":"app","username":"alice","application_name":"pgxray-internal-c7","client_addr":null,"state":"idle","backend_start_ms":1000,"xact_start_ms":null,"query_start_ms":3000,"state_change_ms":4000,"query_age_seconds":4.0,"xact_age_seconds":null,"wait_event_type":"Client","wait_event":"ClientRead","backend_type":"client backend","query":"DISCARD ALL","is_pgxray":true}]"#;

        let backends = parse_activity_output(output).unwrap();
        let snapshot = build_snapshot(backends, 5_000);

        assert_eq!(snapshot.total_connections, 2);
        assert_eq!(snapshot.active_connections, 1);
        assert_eq!(snapshot.idle_connections, 1);
        assert_eq!(snapshot.waiting_active_connections, 1);
        assert_eq!(snapshot.pgxray_connections, 1);
        assert_eq!(snapshot.backends[0].pid, 101);
    }

    #[test]
    fn empty_activity_snapshot_is_supported() {
        let backends = parse_activity_output("[]").unwrap();
        let snapshot = build_snapshot(backends, 5_000);

        assert_eq!(snapshot.total_connections, 0);
        assert_eq!(snapshot.active_connections, 0);
        assert!(snapshot.backends.is_empty());
    }

    #[test]
    fn backend_signal_sql_binds_pid_to_snapshot_identity() {
        let cancel = backend_signal_sql(123, 1_700_000_000_000, BackendSignalAction::Cancel);
        let terminate = backend_signal_sql(456, 1_700_000_100_000, BackendSignalAction::Terminate);

        assert!(cancel.contains("a.pid = 123"));
        assert!(cancel.contains("= 1700000000000"));
        assert!(cancel.contains("pg_cancel_backend(123)"));
        assert!(terminate.contains("a.pid = 456"));
        assert!(terminate.contains("= 1700000100000"));
        assert!(terminate.contains("pg_terminate_backend(456)"));
    }

    #[test]
    fn parses_backend_signal_boolean() {
        assert!(parse_signal_output("t\n").unwrap());
        assert!(!parse_signal_output("f").unwrap());
        assert!(parse_signal_output("unexpected").is_err());
    }
}
