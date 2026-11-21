/**
 * Migration of ui/js/invoke.js (archive 2026-11-09).
 *
 * It deliberately does not import Tauri or register a global. Vue will supply
 * @tauri-apps/api/core's invoke at integration time; legacy UI stays untouched.
 */
export type InvokeArgs = Record<string, unknown>;
export type RawInvoke = (
  command: string,
  args?: InvokeArgs,
) => Promise<unknown>;

/**
 * The caller specifies the expected result type. This is compile-time only;
 * it does NOT validate untrusted data returned by Rust.
 */
export type PgXrayInvoke = <T = unknown>(
  command: string,
  args?: InvokeArgs,
) => Promise<T>;

// Exact compatibility list from ui/js/invoke.js. Adding a Tauri command does not
// automatically add UI event logging; review this list when commands change.
export const LOGGED_COMMANDS = new Set([
  "register_initial_user",
  "login",
  "logout",
  "create_user",
  "rename_user",
  "change_user_password",
  "delete_user",
  "probe",
  "probe_saved_connection",
  "assign_connection_shortcut",
  "list_connection_projects",
  "create_connection_project",
  "rename_connection_project",
  "delete_connection_project",
  "assign_connection_project",
  "probe_connection_draft",
  "save_connection",
  "update_saved_connection",
  "delete_saved_connection",
  "start_psql_session",
  "close_psql_session",
  "set_export_directory",
  "set_application_log_directory",
  "migrate_workspace_root",
  "runtime_readiness",
  "set_history_retention",
  "delete_connection_history",
  "collect_xid_snapshot",
  "export_xid_csv",
  "collect_pgss_snapshot",
  "collect_io_wal_snapshot",
  "io_wal_workload_history",
  "collect_index_health_snapshot",
  "load_pgss_snapshot",
  "compare_pgss_snapshots",
  "pgss_statement_history",
  "pgss_workload_history",
  "pgss_query_detail",
  "collect_autovacuum_snapshot",
  "autovacuum_workload_history",
  "collect_pg_settings_snapshot",
  "pg_settings_history_analysis",
  "autovacuum_table_detail",
  "simulate_autovacuum_policy",
  "create_privacy_scope",
  "create_investigation_case",
  "update_investigation_case_description",
  "create_case_intervention",
  "list_workspace_changes",
  "link_workspace_change_to_case",
  "delete_case_intervention",
  "record_case_change_impact",
  "delete_case_change_impact",
  "import_case_file",
  "create_case_file",
  "update_case_file",
  "delete_investigation_case",
  "delete_case_artifact",
  "delete_case_comparison",
  "delete_privacy_scope",
  "reset_privacy_mapping",
  "delete_privacy_mapping_entries",
  "privacy_transform",
  "anonymize_case_artifact",
  "set_case_artifact_role",
  "export_case_artifact",
  "reveal_case_artifact",
  "open_plan_workspace",
  "prepare_plan_share",
  "compare_plan_artifacts",
  "compare_case_query_results",
  "compare_case_query_plans",
  "signal_postgres_backend",
]);

// Only these operations participate in a verified Rust↔UI correlation.
// Other legacy IPC calls keep their existing arguments and log behavior.
export const CORRELATED_COMMANDS = new Set([
  "login", "list_connection_projects", "collect_xid_snapshot", "collect_pgss_snapshot",
  "collect_autovacuum_snapshot", "collect_pg_settings_snapshot",
  "collect_io_wal_snapshot", "collect_index_health_snapshot",
]);

const SAFE_CONTEXT_KEYS = new Set([
  "id",
  "connectionId",
  "projectId",
  "slot",
  "caseId",
  "artifactId",
  "baselineArtifactId",
  "candidateArtifactId",
  "comparisonId",
  "interventionId",
  "impactId",
  "scopeId",
  "sessionId",
  "relationOid",
  "snapshotId",
  "userid",
  "dbid",
  "pid",
  "backendStartMs",
  "firstId",
  "secondId",
  "analyzeQueries",
  "timeoutMs",
  "scanInbox",
  "deleteComparisons",
  "forceNew",
  "force",
  "useSavedPassword",
  "keepExistingPassword",
  "toplevel",
  "retentionDays",
]);

/** UI log context excludes SQL, credentials, paths and all string arguments. */
export function safeInvokeContext(args?: InvokeArgs): string | null {
  if (!args || typeof args !== "object") return null;

  const parts: string[] = [];
  for (const [key, value] of Object.entries(args)) {
    // PGSS 64-bit queryid must stay a string. Keep only canonical numeric IDs,
    // never log query text, file paths, credentials or arbitrary strings.
    if (key === "queryid" && typeof value === "string" && /^-?[0-9]{1,20}$/.test(value)) {
      parts.push(`queryid=${value}`);
      continue;
    }
    if (!SAFE_CONTEXT_KEYS.has(key)) continue;
    if (typeof value !== "number" && typeof value !== "boolean" && value !== null) continue;
    parts.push(`${key}=${String(value)}`);
  }
  return parts.length > 0 ? parts.join(" ") : null;
}

/**
 * Keep the same log queue and return/error behavior as the legacy wrapper.
 * Raw log calls bypass the wrapper, avoiding recursive logging.
 * The queue is intentionally best-effort and never awaited by a user command.
 */
export function createPgXrayInvoke(
  rawInvoke: RawInvoke | null | undefined,
  now: () => number = () => performance.now(),
): PgXrayInvoke | null {
  if (!rawInvoke) return null;
  const invokeRaw = rawInvoke;

  let requestCounter = 0;
  let logQueue: Promise<unknown> = Promise.resolve();

  function nextRequestId(): string {
    requestCounter = (requestCounter + 1) % 1_000_000;
    return `${Date.now().toString(36)}:${requestCounter.toString(36)}`;
  }

  function queueLog(
    operation: string,
    phase: "started" | "succeeded" | "failed",
    requestId: string,
    durationMs: number | null,
    context: string | null,
    error: string | null,
  ): void {
    logQueue = logQueue
      .then(() => invokeRaw("application_log_event", {
        operation,
        phase,
        requestId,
        durationMs,
        context,
        error,
      }))
      .catch(() => {
        // Application logging is best-effort and never gates the operation.
      });
  }

  return async function invoke<T = unknown>(command: string, args?: InvokeArgs): Promise<T> {
    if (!LOGGED_COMMANDS.has(command)) {
      return invokeRaw(command, args) as Promise<T>;
    }

    const requestId = nextRequestId();
    const context = safeInvokeContext(args);
    queueLog(command, "started", requestId, null, context, null);
    const startedAt = now();

    try {
      const invokeArgs = CORRELATED_COMMANDS.has(command)
        ? { ...(args ?? {}), clientRequestId: requestId }
        : args;
      const result = await invokeRaw(command, invokeArgs);
      const durationMs = Math.max(0, Math.round(now() - startedAt));
      queueLog(command, "succeeded", requestId, durationMs, context, null);
      return result as T;
    } catch (error) {
      const durationMs = Math.max(0, Math.round(now() - startedAt));
      queueLog(command, "failed", requestId, durationMs, context, String(error).slice(0, 4096));
      throw error;
    }
  };
}
