import { t } from "../i18n/locale-service.js";
import { parseLogEntries, type LogEntry } from "./extra-controller.js";
import { logOperationLabel, type LogDisplayRow } from "./log-view-model.js";

/** All filters run on the Rust side, before returning a page. */
export interface JournalQuery {
  limit: number;
  cursor: string | null;
  from_ms: number | null;
  to_ms: number | null;
  component: string | null;
  section: string | null;
  level: string | null;
  operation: string | null;
  actor: string | null;
  request_id: string | null;
  operation_id: string | null;
  client_request_id?: string | null;
  problems_only: boolean;
  min_duration_ms: number | null;
  search: string | null;
}
export interface JournalPage {
  entries: LogEntry[];
  next_cursor: string | null;
  scanned_bytes: number;
  malformed_lines: number;
  scan_limit_reached: boolean;
}
export function parseJournalPage(raw: unknown): JournalPage {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) throw new Error(t('tsJournal.badResponse'));
  const p = raw as Record<string, unknown>;
  for (const key of ["scanned_bytes", "malformed_lines"]) {
    if (typeof p[key] !== "number" || !Number.isSafeInteger(p[key]) || p[key] < 0)
      throw new Error(t('tsJournal.badField', { key }));
  }
  if (typeof p.scan_limit_reached !== "boolean" ||
    (p.next_cursor !== null && (typeof p.next_cursor !== "string" || p.next_cursor.length > 256)))
    throw new Error(t('tsJournal.badCursor'));
  return {
    entries: parseLogEntries(p.entries),
    next_cursor: p.next_cursor as string | null,
    scanned_bytes: p.scanned_bytes as number,
    malformed_lines: p.malformed_lines as number,
    scan_limit_reached: p.scan_limit_reached,
  };
}

export function journalModule(operation: string, component: string): string {
  const op = operation.toLowerCase();
  if (op.includes("pgss") || op.includes("statement")) return "PGSS / SQL";
  if (op.includes("xid") || op.includes("freeze")) return "XID / Freeze";
  if (op.includes("autovacuum") || op.includes("vacuum")) return "Autovacuum";
  if (op.includes("io_wal") || op.includes("wal")) return "I/O / WAL";
  if (op.includes("index_health")) return "Index Health";
  if (op.includes("pg_settings")) return "PG Settings";
  if (op.includes("activity") || op.includes("backend") || op === "internal_postgres") return "PostgreSQL";
  if (op.includes("plan") || op.includes("explain")) return "Plans / EXPLAIN";
  if (op.includes("psql") || op.includes("session")) return "psql Sessions";
  if (op.includes("privacy") || op.includes("case") || op.includes("artifact") || op.includes("workspace")) return "Investigations";
  if (op.includes("connection") || op.includes("probe") || op.includes("project")) return "Connections";
  if (op.includes("user") || op === "login" || op === "logout") return "Authentication";
  if (component === "app_log" || op.includes("log")) return "Application Log";
  if (op.includes("settings") || op.includes("directory") || op.includes("retention")) return "Settings";
  if (component === "application") return "Application";
  return component === "ui_reported" ? t('tsJournal.otherUi') : component;
}

export function journalContext(context: string | null): { key: string; value: string }[] {
  if (!context) return [];
  return Array.from(context.matchAll(/(?:^|\s)([a-z][a-z0-9_]{0,48})=([^\s]+)/gi),
    match => ({ key: match[1]!, value: match[2]! }));
}
export function journalContextValue(context: string | null, key: string): string | null {
  return journalContext(context).find(item => item.key === key)?.value ?? null;
}
/** Prefer trusted diagnostic/SQL IDs over UI context. Never infer an ID from time. */
export function journalConnectionId(entries: readonly LogEntry[]): number | null {
  const validId = (candidate: string | number | null | undefined): number | null => {
    if (candidate == null) return null;
    const id = typeof candidate === "number" ? candidate : Number(candidate);
    return Number.isSafeInteger(id) && id > 0 ? id : null;
  };
  for (const entry of entries) {
    const id = validId(entry.diagnostic?.connection_id);
    if (id !== null && entry.component === "diagnostics") return id;
  }
  for (const entry of entries) {
    if (entry.component === "ui_reported") continue;
    const id = validId(journalContextValue(entry.context, "connection_id"))
      ?? validId(journalContextValue(entry.context, "connectionId"));
    if (id !== null) return id;
  }
  for (const entry of entries) {
    const id = validId(journalContextValue(entry.context, "connection_id"))
      ?? validId(journalContextValue(entry.context, "connectionId"));
    if (id !== null) return id;
  }
  return null;
}

/** Display the current saved profile name, never its credentials. Historical
 * events do not store a name: renamed/deleted profiles cannot be reconstructed. */
export function journalConnectionLabel(id: number, names: ReadonlyMap<number, string>): string {
  const name = names.get(id)?.trim();
  return name ? `${name} (#${id})` : t('tsJournal.connectionMissing', { id: String(id) });
}

export function journalStatus(row: LogDisplayRow): string {
  if (row.result === "failed" || row.level === "error") return t('tsJournal.failed');
  if (["warning", "warn"].includes(row.level)) return t('tsJournal.warning');
  return row.result === "incomplete" ? t('tsJournal.incomplete') :
    row.result === "success" ? t('tsJournal.success') : t('tsJournal.event');
}

export function journalDescription(row: LogDisplayRow, connectionLabel?: (id: number) => string): string {
  const name = logOperationLabel(row.operation);
  const phase = row.phase.toLowerCase();
  if (row.operation === "internal_postgres") {
    const connection = journalContextValue(row.context, "connection_id");
    const id = connection === null ? null : Number(connection);
    const target = id !== null && Number.isSafeInteger(id) && id > 0
      ? ` · ${connectionLabel?.(id) ?? t('tsJournal.connectionId', { id: String(id) })}` : "";
    if (row.result === "failed" || phase.includes("timeout"))
      return t('tsJournal.sqlFailed', { target, detail: row.error ? `: ${row.error}` : ` (${row.phase})` });
    if (row.result === "success" && phase === "query_finished")
      return t('tsJournal.sqlSuccess', { target });
    if (row.result === "incomplete")
      return t('tsJournal.sqlIncomplete', { target });
    const messages: Record<string, string> = {
      client_created: t('tsJournal.clientCreated'),
      client_reused: t('tsJournal.clientReused'),
      client_retired: t('tsJournal.clientRetired'),
      client_shutdown: t('tsJournal.clientShutdown'),
      idle_shutdown_finished: t('tsJournal.idleShutdown'),
      idle_shutdown_forced: t('tsJournal.idleForced'),
    };
    return (messages[phase] ?? `${name}: ${row.phase}`) + target;
  }
  const outcome = [...row.entries].reverse().find(e => e.operation_result)?.operation_result;
  if (outcome?.kind === "authentication")
    return outcome.authenticated ? t('tsJournal.authSucceeded') : t('tsJournal.authRejected');
  if (outcome?.kind === "connection_projects")
    return outcome.succeeded ? t('tsJournal.projectsLoaded', { count: String(outcome.count) }) :
      t('tsJournal.projectsFailed');
  if (row.component === "application" && row.operation === "pgxray" && row.phase === "started")
    return t('tsJournal.startedApp');
  if (row.component === "diagnostics") {
    const details = [...row.entries].reverse().find(e => e.diagnostic?.snapshot_id != null)?.diagnostic;
    if (row.result === "success" && details?.snapshot_id != null)
      return t('tsJournal.snapshotSaved', { kind: details.kind, id: String(details.snapshot_id), connection: connectionLabel?.(details.connection_id) ?? t('tsJournal.connectionId', { id: String(details.connection_id) }) });
    if (row.result === "failed")
      return t('tsJournal.collectionFailed', { detail: row.error ? `: ${row.error}` : '' });
    return t('tsJournal.collectionStarted');
  }
  if (row.source === "ui") {
    if (row.result === "failed" || row.error)
      return t('tsJournal.uiFailed', { name, detail: row.error ? ` — ${row.error}` : '' });
    if (row.result === "success") return t('tsJournal.uiSuccess', { name });
    if (row.result === "incomplete") return t('tsJournal.uiIncomplete', { name });
    return t('tsJournal.uiMessage', { name, phase: row.phase });
  }
  if (row.result === "failed" || row.error) return t('tsJournal.operationFailed', { name, detail: row.error ? ` — ${row.error}` : ` (${row.phase})` });
  if (row.result === "success") return t('tsJournal.operationDone', { name });
  if (row.result === "incomplete") return t('tsJournal.operationIncomplete', { name });
  return `${name}: ${row.phase}`;
}
export function journalDateBounds(from: string, to: string): { from_ms: number | null; to_ms: number | null } {
  const convert = (date: string, endOfDay: boolean): number | null => {
    if (!date) return null;
    if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw new Error(t('tsJournal.badDateFormat'));
    const [y, m, d] = date.split("-").map(Number);
    const value = new Date(y!, m! - 1, d!, endOfDay ? 23 : 0, endOfDay ? 59 : 0, endOfDay ? 59 : 0, endOfDay ? 999 : 0);
    if (value.getFullYear() !== y || value.getMonth() !== m! - 1 || value.getDate() !== d)
      throw new Error(t('tsJournal.badDate'));
    return value.getTime();
  };
  const from_ms = convert(from, false), to_ms = convert(to, true);
  if (from_ms !== null && to_ms !== null && from_ms > to_ms) throw new Error(t('tsJournal.reversedDates'));
  return { from_ms, to_ms };
}
