use crate::{
    activity::ActivitySnapshot,
    history,
    internal_postgres::run_readonly_once,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const MAX_LOCK_ROWS: usize = 500;
const MAX_GRAPH_PATHS: usize = 256;
const MAX_TOTAL_GRAPH_PATHS: usize = 2048;
const MAX_GRAPH_DEPTH: usize = 64;
const MAX_IDLE_XACT_ROWS: usize = 50;

const LOCKS_SQL: &str = r#"
WITH current_db AS (
    SELECT oid
    FROM pg_database
    WHERE datname = current_database()
),
activity AS (
    SELECT
        a.pid,
        a.datname,
        a.usename,
        COALESCE(a.application_name, '') AS application_name,
        a.client_addr::text AS client_addr,
        a.state,
        a.backend_start,
        a.xact_start,
        a.backend_xmin,
        a.backend_xid,
        a.query_start,
        a.state_change,
        row_to_json(a)::jsonb ->> 'wait_event_type' AS wait_event_type,
        row_to_json(a)::jsonb ->> 'wait_event' AS wait_event,
        row_to_json(a)::jsonb ->> 'backend_type' AS backend_type,
        row_to_json(a)::jsonb ->> 'query_id' AS query_id,
        a.query,
        COALESCE(pg_blocking_pids(a.pid), ARRAY[]::integer[]) AS blocking_pids
    FROM pg_stat_activity AS a
    WHERE a.pid <> pg_backend_pid()
),
relevant AS (
    SELECT DISTINCT pid
    FROM (
        SELECT a.pid
        FROM activity AS a
        WHERE cardinality(a.blocking_pids) > 0

        UNION ALL

        SELECT unnest(a.blocking_pids) AS pid
        FROM activity AS a
        WHERE cardinality(a.blocking_pids) > 0
    ) AS pids
),
idle_xact_rows AS (
    SELECT json_build_object(
        'pid', a.pid,
        'database', a.datname,
        'username', a.usename,
        'application_name', a.application_name,
        'client_addr', a.client_addr,
        'idle_age_seconds', CASE WHEN a.state_change IS NOT NULL
            THEN extract(epoch FROM (clock_timestamp() - a.state_change)) ELSE NULL END,
        'backend_xmin', a.backend_xmin::text,
        'backend_xid', a.backend_xid::text,
        'xact_age_seconds', CASE WHEN a.xact_start IS NOT NULL
            THEN extract(epoch FROM (clock_timestamp() - a.xact_start)) ELSE NULL END,
        'is_blocking', EXISTS (
            SELECT 1 FROM relevant AS r WHERE r.pid = a.pid
              AND EXISTS (SELECT 1 FROM activity AS blocked WHERE a.pid = ANY(blocked.blocking_pids))
        )
    ) AS value
    FROM activity AS a
    WHERE a.state LIKE 'idle in transaction%'
      AND a.datname = current_database()
    ORDER BY a.xact_start NULLS LAST, a.pid
    LIMIT 50
),
backend_rows AS (
    SELECT json_build_object(
        'pid', a.pid,
        'database', a.datname,
        'username', a.usename,
        'application_name', a.application_name,
        'client_addr', a.client_addr,
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
            WHEN a.query_start IS NULL OR a.state IS DISTINCT FROM 'active' THEN NULL
            ELSE extract(epoch FROM (clock_timestamp() - a.query_start))
        END,
        'xact_age_seconds', CASE
            WHEN a.xact_start IS NULL THEN NULL
            ELSE extract(epoch FROM (clock_timestamp() - a.xact_start))
        END,
        'state_age_seconds', CASE
            WHEN a.state_change IS NULL THEN NULL
            ELSE extract(epoch FROM (clock_timestamp() - a.state_change))
        END,
        'wait_event_type', a.wait_event_type,
        'wait_event', a.wait_event,
        'backend_type', a.backend_type,
        'query_id', a.query_id,
        'query', a.query,
        'is_pgxray', a.application_name LIKE 'pgxray-%',
        'blocking_pids', a.blocking_pids
    ) AS value
    FROM activity AS a
    JOIN relevant AS r ON r.pid = a.pid
),
lock_base AS (
    SELECT
        l.pid,
        l.locktype,
        l.mode,
        l.granted,
        l.fastpath,
        row_to_json(l)::jsonb ->> 'waitstart' AS waitstart,
        l.database::bigint AS database_oid,
        l.relation::bigint AS relation_oid,
        n.nspname::text AS schema_name,
        c.relname::text AS relation_name,
        l.page::bigint AS page,
        l.tuple::bigint AS tuple,
        l.virtualxid::text AS virtualxid,
        l.transactionid::text AS transactionid,
        l.classid::bigint AS classid,
        l.objid::bigint AS objid,
        l.objsubid::integer AS objsubid
    FROM pg_locks AS l
    JOIN relevant AS r ON r.pid = l.pid
    CROSS JOIN current_db AS db
    LEFT JOIN pg_class AS c
        ON c.oid = l.relation
       AND l.database = db.oid
    LEFT JOIN pg_namespace AS n ON n.oid = c.relnamespace
),
lock_rows AS (
    SELECT json_build_object(
        'pid', l.pid,
        'lock_type', l.locktype,
        'mode', l.mode,
        'granted', l.granted,
        'fastpath', l.fastpath,
        'wait_start_ms', CASE
            WHEN l.waitstart IS NULL THEN NULL
            ELSE (extract(epoch FROM l.waitstart::timestamptz) * 1000)::bigint
        END,
        'database_oid', l.database_oid,
        'relation_oid', l.relation_oid,
        'schema_name', l.schema_name,
        'relation_name', l.relation_name,
        'page', l.page,
        'tuple', l.tuple,
        'virtualxid', l.virtualxid,
        'transactionid', l.transactionid,
        'classid', l.classid,
        'objid', l.objid,
        'objsubid', l.objsubid
    ) AS value
    FROM lock_base AS l
    ORDER BY
        l.granted ASC,
        l.pid,
        l.locktype,
        l.mode
    LIMIT 500
)
SELECT json_build_object(
    'measured_at', clock_timestamp(),
    'database', current_database(),
    'database_oid', (SELECT oid::bigint FROM current_db),
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'backends', COALESCE((SELECT json_agg(value) FROM backend_rows), '[]'::json),
    'idle_xact_total', (SELECT count(*) FROM activity WHERE state LIKE 'idle in transaction%' AND datname = current_database()),
    'idle_xacts', COALESCE((SELECT json_agg(value) FROM idle_xact_rows), '[]'::json),
    'locks', COALESCE((SELECT json_agg(value) FROM lock_rows), '[]'::json),
    'lock_count_total', (SELECT count(*)::bigint FROM lock_base)
)::text;
"#;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockBackend {
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
    pub state_age_seconds: Option<f64>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    pub backend_type: Option<String>,
    pub query_id: Option<String>,
    pub query: Option<String>,
    pub is_pgxray: bool,
    pub blocking_pids: Vec<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockDetail {
    pub pid: i32,
    pub lock_type: String,
    pub mode: String,
    pub granted: bool,
    pub fastpath: bool,
    pub wait_start_ms: Option<i64>,
    pub database_oid: Option<i64>,
    pub relation_oid: Option<i64>,
    pub schema_name: Option<String>,
    pub relation_name: Option<String>,
    pub page: Option<i64>,
    pub tuple: Option<i64>,
    pub virtualxid: Option<String>,
    pub transactionid: Option<String>,
    pub classid: Option<i64>,
    pub objid: Option<i64>,
    pub objsubid: Option<i32>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawLockSnapshot {
    measured_at: String,
    database: String,
    database_oid: u32,
    server_version: String,
    server_version_num: i32,
    backends: Vec<LockBackend>,
    locks: Vec<LockDetail>,
    lock_count_total: i64,
    #[serde(default)]
    idle_xacts: Vec<IdleTransaction>,
    #[serde(default)]
    idle_xact_total: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IdleTransaction {
    pub pid: i32,
    pub database: Option<String>,
    pub username: Option<String>,
    pub application_name: String,
    pub xact_age_seconds: Option<f64>,
    pub idle_age_seconds: Option<f64>,
    pub client_addr: Option<String>,
    pub backend_xmin: Option<String>,
    pub backend_xid: Option<String>,
    pub is_blocking: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockEdge {
    pub blocked_pid: i32,
    pub blocker_pid: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockPath {
    pub pids: Vec<i32>,
    pub cycle_detected: bool,
    #[serde(default)]
    pub incomplete: bool,
    #[serde(default)]
    pub prepared_transaction: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockWait {
    pub blocked_pid: i32,
    pub immediate_blocker_pids: Vec<i32>,
    pub root_blocker_pids: Vec<i32>,
    pub max_depth: usize,
    pub paths: Vec<LockPath>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockSummary {
    pub blocked_backends: usize,
    pub direct_blockers: usize,
    pub root_blockers: usize,
    pub idle_in_transaction_roots: usize,
    pub longest_chain_depth: usize,
}

/// Captured connection profile identity, never credentials. Not a cryptographic
/// server identity: DNS resolution and a password-only edit remain unverified.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LockSource {
    pub connection_id: i64,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub sslmode: String,
}

impl LockSource {
    pub fn from_input(connection_id: i64, input: &ConnectionInput) -> Self {
        Self {
            connection_id,
            host: input.host.clone(),
            port: input.port,
            username: input.username.clone(),
            sslmode: input.sslmode.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockSnapshot {
    pub captured_at_ms: i64,
    pub measured_at: String,
    pub database: String,
    pub database_oid: u32,
    pub server_version: String,
    pub server_version_num: i32,
    pub summary: LockSummary,
    pub backends: Vec<LockBackend>,
    pub edges: Vec<LockEdge>,
    pub waits: Vec<LockWait>,
    pub locks: Vec<LockDetail>,
    pub lock_count_total: i64,
    #[serde(default)]
    pub idle_xacts: Vec<IdleTransaction>,
    #[serde(default)]
    pub idle_xact_total: usize,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub paths_truncated: bool,
    #[serde(default)]
    pub source: Option<LockSource>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LockIncidentEvidence {
    pub lock_snapshot: LockSnapshot,
    pub activity_snapshot: Option<ActivitySnapshot>,
    pub activity_error: Option<String>,
}

pub async fn snapshot(input: &ConnectionInput) -> Result<LockSnapshot, String> {
    let output =
        run_readonly_once(input, LOCKS_SQL, QueryExecutionPolicy::activity_snapshot()).await?;

    let raw = serde_json::from_str::<RawLockSnapshot>(output.trim())
        .map_err(|error| format!("Не удалось разобрать Locks / Waits snapshot: {error}"))?;

    build_snapshot(raw, history::unix_time_millis()?)
}

fn build_snapshot(raw: RawLockSnapshot, captured_at_ms: i64) -> Result<LockSnapshot, String> {
    let backend_map = raw
        .backends
        .iter()
        .map(|backend| (backend.pid, backend))
        .collect::<HashMap<_, _>>();

    let mut edges = Vec::new();
    let mut direct_blockers = HashSet::new();

    for backend in &raw.backends {
        let mut blockers = backend.blocking_pids.clone();
        blockers.sort_unstable();
        blockers.dedup();

        for blocker_pid in blockers {
            edges.push(LockEdge {
                blocked_pid: backend.pid,
                blocker_pid,
            });
            direct_blockers.insert(blocker_pid);
        }
    }

    edges.sort_by_key(|edge| (edge.blocked_pid, edge.blocker_pid));

    let mut waits = Vec::new();
    let mut root_blockers = HashSet::new();
    let mut longest_chain_depth = 0usize;
    let mut paths_truncated = false;

    for backend in raw
        .backends
        .iter()
        .filter(|backend| !backend.blocking_pids.is_empty())
    {
        let mut paths = Vec::new();
        let mut visited = HashSet::new();
        visited.insert(backend.pid);
        let used = waits
            .iter()
            .map(|wait: &LockWait| wait.paths.len())
            .sum::<usize>();
        let remaining = MAX_TOTAL_GRAPH_PATHS.saturating_sub(used);
        if remaining == 0 {
            paths_truncated = true;
        } else {
            collect_paths(
                backend.pid,
                &backend_map,
                vec![backend.pid],
                &mut visited,
                &mut paths,
                &mut paths_truncated,
                remaining.min(MAX_GRAPH_PATHS),
            );
        }

        if paths.is_empty() {
            paths.push(LockPath {
                pids: vec![backend.pid],
                cycle_detected: false,
                incomplete: true,
                prepared_transaction: false,
            });
        }

        let mut roots = Vec::new();
        let mut max_depth = 0usize;

        for path in &paths {
            max_depth = max_depth.max(path.pids.len().saturating_sub(1));

            if !path.cycle_detected && !path.incomplete && !path.prepared_transaction {
                if let Some(root_pid) = path.pids.last().copied() {
                    if root_pid != backend.pid {
                        roots.push(root_pid);
                        root_blockers.insert(root_pid);
                    }
                }
            }
        }

        roots.sort_unstable();
        roots.dedup();
        longest_chain_depth = longest_chain_depth.max(max_depth);

        let mut immediate = backend.blocking_pids.clone();
        immediate.sort_unstable();
        immediate.dedup();

        waits.push(LockWait {
            blocked_pid: backend.pid,
            immediate_blocker_pids: immediate,
            root_blocker_pids: roots,
            max_depth,
            paths,
        });
    }

    waits.sort_by_key(|wait| wait.blocked_pid);

    let idle_in_transaction_roots = root_blockers
        .iter()
        .filter(|pid| {
            backend_map
                .get(pid)
                .and_then(|backend| backend.state.as_deref())
                .is_some_and(|state| state.starts_with("idle in transaction"))
        })
        .count();

    let mut limitations = Vec::new();
    if raw.idle_xact_total > raw.idle_xacts.len() {
        limitations.push(format!(
            "Idle in transaction: показаны первые {MAX_IDLE_XACT_ROWS} из {} сессий текущей базы.",
            raw.idle_xact_total
        ));
    }
    if paths_truncated {
        limitations.push(format!(
            "Blocking graph усечён: максимум {MAX_GRAPH_PATHS} путей на backend, {MAX_TOTAL_GRAPH_PATHS} на snapshot и глубина {MAX_GRAPH_DEPTH}; часть root blockers может отсутствовать."
        ));
    }
    if edges.iter().any(|edge| edge.blocker_pid == 0) {
        limitations.push(
            crate::i18n::text(
                "PID 0 означает блокировку prepared transaction, а не обычный backend.",
                "PID 0 represents a lock held by a prepared transaction, not an ordinary backend.",
            )
            .into(),
        );
    }
    if edges
        .iter()
        .any(|edge| edge.blocker_pid != 0 && !backend_map.contains_key(&edge.blocker_pid))
    {
        limitations.push(
            "Не все blocking PIDs присутствуют в pg_stat_activity; такие пути помечены как неполные."
                .into(),
        );
    }

    if raw.lock_count_total > i64::try_from(raw.locks.len()).unwrap_or(i64::MAX) {
        limitations.push(format!(
            "Lock details ограничены первыми {MAX_LOCK_ROWS} строками; всего relevant lock rows = {}.",
            raw.lock_count_total,
        ));
    }

    if raw.backends.iter().any(|backend| {
        backend
            .query
            .as_deref()
            .is_some_and(|query| query.contains("insufficient privilege"))
    }) {
        limitations.push(
            "PostgreSQL скрыл query text как минимум одного backend из-за прав текущей роли."
                .into(),
        );
    }

    if raw.backends.iter().any(|backend| {
        backend
            .database
            .as_deref()
            .is_some_and(|database| database != raw.database)
    }) {
        limitations.push(
            crate::i18n::text("В chain есть backend из другой database; relation names разрешаются только для текущей database.", "The blocking chain includes a backend from another database; relation names are resolved only for the current database.").into(),
        );
    }

    limitations.push(
        "Blocking graph — моментальный снимок: после capture backend/lock может исчезнуть или измениться."
            .into(),
    );

    Ok(LockSnapshot {
        captured_at_ms,
        measured_at: raw.measured_at,
        database: raw.database,
        database_oid: raw.database_oid,
        server_version: raw.server_version,
        server_version_num: raw.server_version_num,
        summary: LockSummary {
            blocked_backends: waits.len(),
            direct_blockers: direct_blockers.len(),
            root_blockers: root_blockers.len(),
            idle_in_transaction_roots,
            longest_chain_depth,
        },
        backends: raw.backends,
        edges,
        waits,
        locks: raw.locks,
        lock_count_total: raw.lock_count_total,
        idle_xacts: raw.idle_xacts,
        idle_xact_total: raw.idle_xact_total,
        limitations,
        paths_truncated,
        source: None,
    })
}

fn collect_paths(
    pid: i32,
    backend_map: &HashMap<i32, &LockBackend>,
    path: Vec<i32>,
    visited: &mut HashSet<i32>,
    output: &mut Vec<LockPath>,
    truncated: &mut bool,
    max_paths: usize,
) {
    if output.len() >= max_paths || path.len() > MAX_GRAPH_DEPTH {
        *truncated = true;
        return;
    }
    if pid == 0 {
        output.push(LockPath {
            pids: path,
            cycle_detected: false,
            incomplete: false,
            prepared_transaction: true,
        });
        return;
    }
    let Some(backend) = backend_map.get(&pid) else {
        output.push(LockPath {
            pids: path,
            cycle_detected: false,
            incomplete: true,
            prepared_transaction: false,
        });
        return;
    };
    let mut blockers = backend.blocking_pids.clone();
    blockers.sort_unstable();
    blockers.dedup();
    if blockers.is_empty() {
        output.push(LockPath {
            pids: path,
            cycle_detected: false,
            incomplete: false,
            prepared_transaction: false,
        });
        return;
    }
    for blocker_pid in blockers {
        if output.len() >= max_paths || path.len() >= MAX_GRAPH_DEPTH {
            *truncated = true;
            break;
        }
        let mut next_path = path.clone();
        next_path.push(blocker_pid);
        if visited.contains(&blocker_pid) {
            output.push(LockPath {
                pids: next_path,
                cycle_detected: true,
                incomplete: false,
                prepared_transaction: false,
            });
            continue;
        }
        visited.insert(blocker_pid);
        collect_paths(
            blocker_pid,
            backend_map,
            next_path,
            visited,
            output,
            truncated,
            max_paths,
        );
        visited.remove(&blocker_pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(pid: i32, state: &str, blocking_pids: &[i32]) -> LockBackend {
        LockBackend {
            pid,
            database: Some("app".into()),
            username: Some("user".into()),
            application_name: "app".into(),
            client_addr: None,
            state: Some(state.into()),
            backend_start_ms: Some(1),
            xact_start_ms: Some(2),
            query_start_ms: Some(3),
            state_change_ms: Some(4),
            query_age_seconds: Some(1.0),
            xact_age_seconds: Some(2.0),
            state_age_seconds: Some(1.0),
            wait_event_type: None,
            wait_event: None,
            backend_type: Some("client backend".into()),
            query_id: None,
            query: None,
            is_pgxray: false,
            blocking_pids: blocking_pids.to_vec(),
        }
    }

    fn raw(backends: Vec<LockBackend>) -> RawLockSnapshot {
        RawLockSnapshot {
            measured_at: "2026-09-16T10:00:00Z".into(),
            database: "app".into(),
            database_oid: 1,
            server_version: "17".into(),
            server_version_num: 170000,
            backends,
            locks: Vec::new(),
            lock_count_total: 0,
            idle_xacts: Vec::new(),
            idle_xact_total: 0,
        }
    }

    #[test]
    fn idle_transaction_without_waiters_is_not_a_root_blocker() {
        let mut input = raw(Vec::new());
        input.idle_xacts = vec![IdleTransaction {
            pid: 91,
            database: Some("app".into()),
            username: Some("alice".into()),
            application_name: "worker".into(),
            xact_age_seconds: Some(120.0),
            idle_age_seconds: Some(85.0),
            client_addr: Some("127.0.0.1".into()),
            backend_xmin: None,
            backend_xid: Some("735".into()),
            is_blocking: false,
        }];
        input.idle_xact_total = 1;
        let snapshot = build_snapshot(input, 123).unwrap();
        assert_eq!(snapshot.summary.blocked_backends, 0);
        assert_eq!(snapshot.summary.root_blockers, 0);
        assert_eq!(snapshot.idle_xact_total, 1);
        assert!(!snapshot.idle_xacts[0].is_blocking);
        assert_eq!(snapshot.idle_xacts[0].idle_age_seconds, Some(85.0));
        assert_eq!(snapshot.idle_xacts[0].backend_xid.as_deref(), Some("735"));
    }

    #[test]
    fn derives_root_blocker_through_chain() {
        let snapshot = build_snapshot(
            raw(vec![
                backend(10, "active", &[20]),
                backend(20, "active", &[30]),
                backend(30, "idle in transaction", &[]),
            ]),
            100,
        )
        .unwrap();

        assert_eq!(snapshot.summary.blocked_backends, 2);
        assert_eq!(snapshot.summary.root_blockers, 1);
        assert_eq!(snapshot.summary.idle_in_transaction_roots, 1);
        assert_eq!(snapshot.summary.longest_chain_depth, 2);
        assert_eq!(snapshot.waits[0].root_blocker_pids, vec![30]);
        assert_eq!(snapshot.waits[0].paths[0].pids, vec![10, 20, 30]);
    }

    #[test]
    fn supports_multiple_root_blockers() {
        let snapshot = build_snapshot(
            raw(vec![
                backend(10, "active", &[20, 30]),
                backend(20, "active", &[]),
                backend(30, "active", &[]),
            ]),
            100,
        )
        .unwrap();

        assert_eq!(snapshot.summary.root_blockers, 2);
        assert_eq!(snapshot.waits[0].root_blocker_pids, vec![20, 30]);
        assert_eq!(snapshot.waits[0].paths.len(), 2);
    }

    #[test]
    fn cycle_is_reported_instead_of_recursing_forever() {
        let snapshot = build_snapshot(
            raw(vec![
                backend(10, "active", &[20]),
                backend(20, "active", &[10]),
            ]),
            100,
        )
        .unwrap();

        assert!(snapshot.waits[0].paths[0].cycle_detected);
    }
    #[test]
    fn prepared_transaction_is_not_an_ordinary_root() {
        let result = build_snapshot(raw(vec![backend(10, "active", &[0])]), 100).unwrap();
        assert_eq!(result.summary.root_blockers, 0);
        assert!(result.waits[0].paths[0].prepared_transaction);
    }

    #[test]
    fn missing_backend_does_not_become_root() {
        let result = build_snapshot(raw(vec![backend(10, "active", &[99])]), 100).unwrap();
        assert!(result.waits[0].root_blocker_pids.is_empty());
        assert!(result.waits[0].paths[0].incomplete);
    }

    #[test]
    fn branching_graph_is_bounded() {
        let mut rows = vec![backend(10, "active", &[20, 30])];
        // Shared layers provide exponentially many distinct paths without
        // exponentially many backends.
        for layer in 0..9 {
            let first = 20 + layer * 10;
            let next = first + 10;
            rows.push(backend(first, "active", &[next, next + 1]));
            rows.push(backend(first + 1, "active", &[next, next + 1]));
        }
        rows.push(backend(110, "idle", &[]));
        rows.push(backend(111, "idle", &[]));
        let result = build_snapshot(raw(rows), 100).unwrap();
        assert!(result.paths_truncated);
        assert!(result.waits[0].paths.len() <= MAX_GRAPH_PATHS);
    }

    #[test]
    fn sql_and_rust_lock_row_limits_are_consistent() {
        assert!(LOCKS_SQL.contains(&format!("LIMIT {MAX_LOCK_ROWS}")));
    }
}
