import { t } from "../i18n/locale-service.js";
import type { LogEntry } from "./extra-controller.js";

/** A display-only model: original JSONL records remain untouched and accessible. */
export type LogResult = "success" | "failed" | "incomplete" | "other";
export type LogStatusFilter = "all" | LogResult;
export type LogDisplayMode = "operations" | "events";
export const LOG_FETCH_LIMITS = [200, 500, 1000] as const;
export const LOG_PAGE_SIZES = [20, 50] as const;
export type LogFetchLimit = (typeof LOG_FETCH_LIMITS)[number];
export type LogPageSize = (typeof LOG_PAGE_SIZES)[number];

export interface LogDisplayRow {
  key: string;
  timestamp_ms: number;
  operation: string;
  component: string;
  actor: string | null;
  request_id: string | null;
  duration_ms: number | null;
  context: string | null;
  error: string | null;
  level: string;
  phase: string;
  result: LogResult;
  source: "ui" | "backend";
  entries: LogEntry[];
}

function resultFromPhase(phase: string): LogResult {
  switch (phase.toLowerCase()) {
    case "succeeded": case "success": case "completed": case "cleared": case "updated": case "query_finished": case "idle_shutdown_finished": return "success";
    case "failed": case "error": case "query_failed": case "query_timeout": case "client_connect_failed": case "idle_shutdown_forced": return "failed";
    case "started": case "query_started": case "idle_shutdown_started": return "incomplete";
    default: return "other";
  }
}

function displayRow(entry: LogEntry, key: string): LogDisplayRow {
  return {
    key, timestamp_ms: entry.timestamp_ms, operation: entry.operation,
    component: entry.component, actor: entry.actor, request_id: entry.request_id,
    duration_ms: entry.duration_ms, context: entry.context, error: entry.error,
    level: entry.level, phase: entry.phase,
    result: entry.component === "application" && entry.operation === "pgxray" && entry.phase === "started"
      ? "other" : resultFromPhase(entry.phase),
    source: entry.component === "ui_reported" ? "ui" : "backend", entries: [entry],
  };
}

/** An untrusted UI request ID is only a correlation hint; the Rust diagnostic
 * operation_id and typed result remain authoritative for the outcome. */
export function logClientRequestId(entry: LogEntry): string | null {
  if (entry.diagnostic?.client_request_id) return entry.diagnostic.client_request_id;
  if (entry.component === "ui_reported" || entry.operation_result) return entry.request_id;
  return null;
}

function isRustOutcome(entry: LogEntry): boolean {
  return entry.component === "diagnostics" || !!entry.operation_result;
}

function rebuildCorrelatedRow(row: LogDisplayRow): void {
  const backend = [...row.entries].reverse().find(e =>
    isRustOutcome(e) && ["succeeded", "failed"].includes(e.phase))
    ?? row.entries.find(e => isRustOutcome(e));
  const reference = backend ?? [...row.entries].reverse().find(e => e.component === "ui_reported")
    ?? row.entries[row.entries.length - 1]!;
  const final = [...row.entries].reverse().find(e => isRustOutcome(e) &&
    ["succeeded", "failed"].includes(e.phase));
  row.timestamp_ms = Math.max(...row.entries.map(e => e.timestamp_ms));
  row.operation = reference.operation;
  row.component = reference.component;
  row.actor = reference.actor ?? row.entries.find(e => e.actor)?.actor ?? null;
  row.request_id = reference.request_id;
  row.duration_ms = final?.duration_ms ?? reference.duration_ms;
  row.context = reference.context;
  row.error = final?.error ?? reference.error;
  row.level = final?.level ?? reference.level;
  row.phase = final?.phase ?? reference.phase;
  row.source = backend ? "backend" : "ui";
  // A UI completion must not turn a Rust operation with no Rust terminal event
  // into an authoritative success.
  row.result = final ? resultFromPhase(final.phase) : backend ? "incomplete" : resultFromPhase(reference.phase);
}

/** Pair started / terminal phases only when their request IDs and event identity match.
 * An unpaired start means "no terminal event IN THE FETCHED SAMPLE", not "still running".
 */
export function logDisplayRows(entries: readonly LogEntry[], mode: LogDisplayMode): LogDisplayRow[] {
  const chronological = entries.map((entry, index) => ({ entry, index }))
    .sort((a, b) => a.entry.timestamp_ms - b.entry.timestamp_ms || b.index - a.index);
  if (mode === "events") {
    return chronological.map(({ entry, index }) => displayRow(entry, `raw-${index}`))
      .sort((a, b) => b.timestamp_ms - a.timestamp_ms);
  }
  const rows: LogDisplayRow[] = [];
  const waiting = new Map<string, LogDisplayRow>();
  const correlated = new Map<string, LogDisplayRow>();
  for (const { entry, index } of chronological) {
    const clientId = logClientRequestId(entry);
    if (clientId) {
      const linked = correlated.get(clientId);
      const uiAnchor = linked?.entries.find(e => e.component === "ui_reported");
      const conflicting = uiAnchor && (
        (entry.component === "ui_reported" &&
          (entry.actor !== uiAnchor.actor || entry.operation !== uiAnchor.operation)) ||
        ((entry.component === "diagnostics" || !!entry.operation_result) &&
          entry.operation !== uiAnchor.operation)
      );
      if (!conflicting) {
        if (linked) {
          linked.entries.push(entry);
          rebuildCorrelatedRow(linked);
        } else {
          const row = displayRow(entry, `client-${clientId}`);
          correlated.set(clientId, row);
          rows.push(row);
        }
        continue;
      }
      // A reused/colliding UI token cannot merge different actors or commands.
      // Continue through the legacy identity-based pairing below.
    }
    // No request ID: never merge unrelated events even if their names match.
    const id = entry.request_id ? JSON.stringify([
      entry.request_id, entry.component, entry.operation, entry.actor,
    ]) : null;
    const result = resultFromPhase(entry.phase);
    const terminal = result === "success" || result === "failed";
    const previous = id ? waiting.get(id) : undefined;
    if (previous && result !== "incomplete") {
      previous.entries.push(entry);
      previous.timestamp_ms = entry.timestamp_ms;
      previous.duration_ms = entry.duration_ms ?? previous.duration_ms;
      previous.context = entry.context ?? previous.context;
      previous.error = entry.error ?? previous.error;
      previous.level = entry.level;
      previous.phase = entry.phase;
      previous.result = terminal ? result : "incomplete";
      if (id && terminal) waiting.delete(id);
    } else {
      const row = displayRow(entry, `event-${index}`);
      rows.push(row);
      if (id && result === "incomplete") waiting.set(id, row);
      else if (id) waiting.delete(id);
    }
  }
  return rows.sort((a, b) => b.timestamp_ms - a.timestamp_ms);
}

export interface LogViewOptions {
  mode: LogDisplayMode;
  status: LogStatusFilter;
  component: string;
  search: string;
  page: number;
  pageSize: LogPageSize;
}
export interface LogViewPage {
  rows: LogDisplayRow[];
  total: number;
  page: number;
  pages: number;
  successes: number;
  failures: number;
  incomplete: number;
  shownStart: number;
  shownEnd: number;
}

/** All filters and page numbers operate on already fetched records; no hidden IPC calls. */
export function logViewPage(entries: readonly LogEntry[], options: LogViewOptions): LogViewPage {
  const all = logDisplayRows(entries, options.mode);
  const successes = all.filter(row => row.result === "success").length;
  const failures = all.filter(row => row.result === "failed").length;
  const incomplete = all.filter(row => row.result === "incomplete").length;
  const q = options.search.trim().toLocaleLowerCase("ru-RU");
  const filtered = all.filter(row => {
    if (options.status !== "all" && row.result !== options.status) return false;
    if (options.component !== "all" && row.component !== options.component) return false;
    if (!q) return true;
    return row.entries.some(e => [e.operation, e.component, e.actor, e.level, e.phase,
      e.request_id, e.context, e.error].filter(Boolean).some(value =>
      String(value).toLocaleLowerCase("ru-RU").includes(q)));
  });
  const size = LOG_PAGE_SIZES.includes(options.pageSize) ? options.pageSize : 20;
  const pages = Math.max(1, Math.ceil(filtered.length / size));
  const page = Number.isSafeInteger(options.page) ? Math.min(pages, Math.max(1, options.page)) : 1;
  const start = (page - 1) * size;
  return {
    rows: filtered.slice(start, start + size), total: filtered.length, page, pages,
    successes, failures, incomplete,
    shownStart: filtered.length ? start + 1 : 0,
    shownEnd: Math.min(start + size, filtered.length),
  };
}

// Preserve stable operation IDs for filtering, while evaluating labels at access
// time so locale changes are reflected in the open journal immediately.
const operationLabelKeys: Readonly<Record<string, Parameters<typeof t>[0]>> = {
  login: "tsLogOperations.login",
  logout: "tsLogOperations.logout",
  register_initial_user: "tsLogOperations.register_initial_user",
  create_user: "tsLogOperations.create_user",
  rename_user: "tsLogOperations.rename_user",
  delete_user: "tsLogOperations.delete_user",
  change_user_password: "tsLogOperations.change_user_password",
  set_history_retention: "tsLogOperations.set_history_retention",
  set_application_log_settings: "tsLogOperations.set_application_log_settings",
  clear_application_log: "tsLogOperations.clear_application_log",
  save_connection: "tsLogOperations.save_connection",
  update_saved_connection: "tsLogOperations.update_saved_connection",
  delete_saved_connection: "tsLogOperations.delete_saved_connection",
  probe_saved_connection: "tsLogOperations.probe_saved_connection",
  collect_xid_snapshot: "tsLogOperations.collect_xid_snapshot",
  collect_pgss_snapshot: "tsLogOperations.collect_pgss_snapshot",
  collect_autovacuum_snapshot: "tsLogOperations.collect_autovacuum_snapshot",
  collect_pg_settings_snapshot: "tsLogOperations.collect_pg_settings_snapshot",
  pgss_query_detail: "tsLogOperations.pgss_query_detail",
  pgss_statement_history: "tsLogOperations.pgss_statement_history",
  compare_pgss_snapshots: "tsLogOperations.compare_pgss_snapshots",
  load_pgss_snapshot: "tsLogOperations.load_pgss_snapshot",
  export_xid_csv: "tsLogOperations.export_xid_csv",
  runtime_readiness: "tsLogOperations.runtime_readiness",
  internal_postgres: "tsLogOperations.internal_postgres",
  postgres_activity_snapshot: "tsLogOperations.postgres_activity_snapshot",
  signal_postgres_backend: "tsLogOperations.signal_postgres_backend",
  create_investigation_case: "tsLogOperations.create_investigation_case",
  create_case_file: "tsLogOperations.create_case_file",
  export_case_artifact: "tsLogOperations.export_case_artifact",
  start_psql_session: "tsLogOperations.start_psql_session",
  close_psql_session: "tsLogOperations.close_psql_session",
  application_log: "tsLogOperations.application_log",
  application_log_settings: "tsLogOperations.application_log_settings",
  open_plan_workspace: "tsLogOperations.open_plan_workspace",
  prepare_plan_share: "tsLogOperations.prepare_plan_share",
  compare_plan_artifacts: "tsLogOperations.compare_plan_artifacts",
  compare_case_query_plans: "tsLogOperations.compare_case_query_plans",
  compare_case_query_results: "tsLogOperations.compare_case_query_results",
};
export const logOperationLabels: Readonly<Record<string, string>> = new Proxy(operationLabelKeys, {
  get(target, property) {
    if (typeof property !== "string") return Reflect.get(target, property);
    const key = target[property];
    return key === undefined ? undefined : t(key);
  },
});
export function logOperationLabel(operation: string): string {
  return logOperationLabels[operation] ?? operation.replace(/_/g, " ");
}
export function logStatusLabel(result: LogResult): string {
  switch (result) {
    case "success": return t('tsLogStatus.success');
    case "failed": return t('tsLogStatus.failed');
    case "incomplete": return t('tsLogStatus.incomplete');
    case "other": return t('tsLogStatus.other');
  }
}
