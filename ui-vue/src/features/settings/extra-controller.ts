import { t as tr } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";

/** Headless port of ui/js/settings-extra.js (2026-11-09 archive).
 * UI forms and theme DOM handling are deliberately deferred to settings.html/theme.js.
 * This code is not an authorization boundary; Rust remains authoritative.
 */
export const SETTINGS_TABS = ["users", "interface", "history", "snapshots", "logs", "paths", "readiness"] as const;
export type SettingsTab = (typeof SETTINGS_TABS)[number];
export const LOG_SIZE_LIMITS = [500 * 1024 * 1024, 1024 * 1024 * 1024, 3 * 1024 * 1024 * 1024] as const;
export const LOG_RETENTION_DAYS = [7, 30, 90] as const;
export const HISTORY_PRESET_DAYS = [7, 30, 90, 180, 365] as const;

export interface HistorySectionCounts {
  xid: number;
  pgss: number;
  autovacuum: number;
  pg_settings: number;
  io_wal: number;
  index_health: number;
}
export interface HistoryRow {
  connection_id: number;
  connection_name: string;
  snapshot_count: number;
  oldest_at_ms: number;
  newest_at_ms: number;
  // Optional only for compatibility with an older running Tauri backend.
  section_counts?: HistorySectionCounts;
}
export interface Paths {
  workspace_root: string;
  export_dir: string;
  application_log_dir: string;
}
export interface LogSettings { max_total_bytes: number; retention_days: number }
export interface LogStatus {
  directory: string;
  available: boolean;
  last_error: string | null;
  total_bytes: number;
  file_count: number;
  oldest_at_ms: number | null;
  newest_at_ms: number | null;
  max_file_bytes: number;
}
export interface LogState { settings: LogSettings; status: LogStatus }
export interface DiagnosticLogDetails {
  connection_id: number;
  kind: string;
  snapshot_id: number | null;
  unavailable_sections: number | null;
  total_sections: number | null;
  client_request_id?: string | null;
}
export type OperationLogResult =
  | { kind: "authentication"; authenticated: boolean }
  | { kind: "connection_projects"; count: number | null; succeeded: boolean };
export interface LogEntry {
  timestamp_ms: number;
  level: string;
  component: string;
  operation: string;
  phase: string;
  actor: string | null;
  request_id: string | null;
  duration_ms: number | null;
  context: string | null;
  error: string | null;
  operation_id?: string | null;
  diagnostic?: DiagnosticLogDetails | null;
  operation_result?: OperationLogResult | null;
}
export interface ReadinessCheck { key: string; label: string; status: string; detail: string; method?: string }
export interface PostgresReadiness {
  server_version: string;
  database: string;
  username: string;
  support_status: string;
  support_detail: string;
  features: ReadinessCheck[];
  warnings: string[];
}
export interface Readiness {
  app_version: string;
  supported_postgres: string;
  local_checks: ReadinessCheck[];
  postgres: PostgresReadiness | null;
}
export interface MaintenanceResult { total_deleted: number }
export interface ExtraSettingsState {
  tab: SettingsTab;
  paths: Paths | null;
  retentionDays: number | null;
  historyRows: HistoryRow[];
  historyLiveNames: Record<number, string>;
  historyStale: boolean;
  historyLoaded: boolean;
  historyError: string | null;
  log: LogState | null;
  logEntries: LogEntry[];
  logEntriesLimit: number;
  logEntriesError: string | null;
  logEntriesLoading: boolean;
  readiness: Readiness | null;
  loading: boolean;
  busy: boolean;
  error: string | null;
  notice: string | null;
}
export function createExtraSettingsState(): ExtraSettingsState {
  return {
    tab: "users", paths: null, retentionDays: null, historyRows: [], historyLiveNames: {}, historyStale: false, historyLoaded: false, historyError: null, log: null,
    logEntries: [], logEntriesLimit: 200, logEntriesError: null, logEntriesLoading: false, readiness: null,
    loading: false, busy: false, error: null, notice: null,
  };
}

function record(raw: unknown, label: string): Record<string, unknown> {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) throw new Error(tr('tsSettingsExtra.badResponse', { label }));
  return raw as Record<string, unknown>;
}
function textValue(value: unknown, label: string): string {
  if (typeof value !== "string") throw new Error(tr('tsSettingsExtra.badResponse', { label }));
  return value;
}
function integer(value: unknown, label: string, nullable = false): number | null {
  if (nullable && value == null) return null;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0)
    throw new Error(tr('tsSettingsExtra.badResponse', { label }));
  return value;
}
function intValue(value: unknown, label: string): number {
  return integer(value, label) as number;
}
function optionalText(value: unknown, label: string): string | null {
  return value == null ? null : textValue(value, label);
}
function arrayOf<T>(raw: unknown, label: string, parser: (item: unknown) => T): T[] {
  if (!Array.isArray(raw)) throw new Error(tr('tsSettingsExtra.badResponse', { label }));
  return raw.map(parser);
}
export function parsePaths(raw: unknown): Paths {
  const r = record(raw, "path_settings");
  return {
    workspace_root: textValue(r.workspace_root, "workspace_root"),
    export_dir: textValue(r.export_dir, "export_dir"),
    application_log_dir: textValue(r.application_log_dir, "application_log_dir"),
  };
}
export function parseHistoryRows(raw: unknown): HistoryRow[] {
  return arrayOf(raw, "list_history_connections", (value) => {
    const r = record(value, "history row");
    const connection_id = intValue(r.connection_id, "connection_id");
    if (connection_id === 0) throw new Error(tr('tsSettingsExtra.badConnectionId'));
    const snapshot_count = intValue(r.snapshot_count, "snapshot_count");
    let section_counts: HistorySectionCounts | undefined;
    if (r.section_counts !== undefined) {
      const counts = record(r.section_counts, "section_counts");
      section_counts = {
        xid: intValue(counts.xid, "section_counts.xid"),
        pgss: intValue(counts.pgss, "section_counts.pgss"),
        autovacuum: intValue(counts.autovacuum, "section_counts.autovacuum"),
        pg_settings: intValue(counts.pg_settings, "section_counts.pg_settings"),
        io_wal: intValue(counts.io_wal, "section_counts.io_wal"),
        index_health: intValue(counts.index_health, "section_counts.index_health"),
      };
      if (Object.values(section_counts).reduce((total, count) => total + count, 0) !== snapshot_count)
        throw new Error(tr('tsSettingsExtra.badSectionTotal'));
    }
    return {
      connection_id,
      connection_name: textValue(r.connection_name, "connection_name"),
      snapshot_count,
      oldest_at_ms: intValue(r.oldest_at_ms, "oldest_at_ms"),
      newest_at_ms: intValue(r.newest_at_ms, "newest_at_ms"),
      ...(section_counts ? { section_counts } : {}),
    };
  });
}
export function parseRetention(raw: unknown): number | null {
  const r = record(raw, "history_retention_settings");
  const days = integer(r.retention_days, "retention_days", true);
  if (days !== null && (days < 1 || days > 36500)) throw new Error(tr('tsSettingsExtra.badRetention'));
  return days;
}
export function parseMaintenance(raw: unknown): MaintenanceResult {
  const r = record(raw, "history maintenance");
  return { total_deleted: intValue(r.total_deleted, "total_deleted") };
}
export function parseLogState(raw: unknown): LogState {
  const r = record(raw, "application_log_state");
  const s = record(r.settings, "log settings");
  const t = record(r.status, "log status");
  if (typeof t.available !== "boolean") throw new Error(tr('tsSettingsExtra.badLogAvailability'));
  return {
    settings: {
      max_total_bytes: intValue(s.max_total_bytes, "max_total_bytes"),
      retention_days: intValue(s.retention_days, "retention_days"),
    },
    status: {
      directory: textValue(t.directory, "log directory"), available: t.available,
      last_error: optionalText(t.last_error, "last_error"),
      total_bytes: intValue(t.total_bytes, "total_bytes"), file_count: intValue(t.file_count, "file_count"),
      oldest_at_ms: integer(t.oldest_at_ms, "oldest_at_ms", true),
      newest_at_ms: integer(t.newest_at_ms, "newest_at_ms", true),
      max_file_bytes: intValue(t.max_file_bytes, "max_file_bytes"),
    },
  };
}
function parseDiagnosticDetails(raw: unknown): DiagnosticLogDetails {
  const d = record(raw, "diagnostic log details");
  return {
    connection_id: intValue(d.connection_id, "diagnostic.connection_id"),
    kind: textValue(d.kind, "diagnostic.kind"),
    snapshot_id: integer(d.snapshot_id, "diagnostic.snapshot_id", true),
    unavailable_sections: integer(d.unavailable_sections, "diagnostic.unavailable_sections", true),
    total_sections: integer(d.total_sections, "diagnostic.total_sections", true),
    ...(d.client_request_id === undefined ? {} : { client_request_id: optionalText(d.client_request_id, "diagnostic.client_request_id") }),
  };
}
export function parseOperationResult(raw: unknown): OperationLogResult {
  const r = record(raw, "operation result");
  if (r.kind === "authentication" && typeof r.authenticated === "boolean")
    return { kind: "authentication", authenticated: r.authenticated };
  if (r.kind === "connection_projects" && typeof r.succeeded === "boolean")
    return { kind: "connection_projects", succeeded: r.succeeded,
      count: integer(r.count, "operation_result.count", true) };
  throw new Error(tr('tsSettingsExtra.badRustResult'));
}

export function parseLogEntries(raw: unknown): LogEntry[] {
  return arrayOf(raw, "list_application_log_entries", (value) => {
    const e = record(value, "application log entry");
    return {
      timestamp_ms: intValue(e.timestamp_ms, "timestamp_ms"),
      level: textValue(e.level, "level"), component: textValue(e.component, "component"),
      operation: textValue(e.operation, "operation"), phase: textValue(e.phase, "phase"),
      actor: optionalText(e.actor, "actor"), request_id: optionalText(e.request_id, "request_id"),
      duration_ms: integer(e.duration_ms, "duration_ms", true),
      context: optionalText(e.context, "context"), error: optionalText(e.error, "error"),
      ...(e.operation_id === undefined ? {} : { operation_id: optionalText(e.operation_id, "operation_id") }),
      ...(e.diagnostic === undefined ? {} : { diagnostic: e.diagnostic == null ? null : parseDiagnosticDetails(e.diagnostic) }),
      ...(e.operation_result === undefined ? {} : { operation_result: e.operation_result == null ? null : parseOperationResult(e.operation_result) }),
    };
  });
}
function parseCheck(value: unknown): ReadinessCheck {
  const r = record(value, "readiness check");
  return {
    key: textValue(r.key, "key"), label: textValue(r.label, "label"),
    status: textValue(r.status, "status"), detail: textValue(r.detail, "detail"),
  };
}
export function parseReadiness(raw: unknown): Readiness {
  const r = record(raw, "runtime_readiness");
  const postgres = r.postgres == null ? null : record(r.postgres, "postgres readiness");
  return {
    app_version: textValue(r.app_version, "app_version"),
    supported_postgres: textValue(r.supported_postgres, "supported_postgres"),
    local_checks: arrayOf(r.local_checks, "local_checks", parseCheck),
    postgres: postgres == null ? null : {
      server_version: textValue(postgres.server_version, "server_version"),
      database: textValue(postgres.database, "database"),
      username: textValue(postgres.username, "username"),
      support_status: textValue(postgres.support_status, "support_status"),
      support_detail: textValue(postgres.support_detail, "support_detail"),
      features: arrayOf(postgres.features, "features", parseCheck),
      warnings: arrayOf(postgres.warnings, "warnings", (value) => textValue(value, "warning")),
    },
  };
}

/** Current saved profile names are display-only: historical snapshot metadata is immutable. */
export function parseLiveHistoryNames(value: unknown): Record<number, string> {
  if (!Array.isArray(value)) throw new Error(tr('tsSettingsExtra.badConnectionList'));
  const names: Record<number, string> = {};
  for (const item of value) {
    if (item === null || typeof item !== "object") continue;
    const row = item as Record<string, unknown>;
    if (typeof row.id === "number" && Number.isSafeInteger(row.id) && row.id > 0 &&
        typeof row.name === "string" && row.name.trim()) names[row.id] = row.name;
  }
  return names;
}

export function validateRetentionDays(value: number | null): boolean {
  return value === null || (Number.isSafeInteger(value) && value >= 1 && value <= 36500);
}
export type ConfirmAction = (message: string) => boolean | Promise<boolean>;

/** State changes are gated against stale user sessions and resets. */
export function createExtraSettingsController(
  shell: ShellState, state: ExtraSettingsState, invoke: PgXrayInvoke | null,
) {
  let revision = 0;
  let loadId = 0;
  let logLoadId = 0;
  const active = () => shell.screen === "workspace" && shell.currentUsername !== null && invoke !== null;
  const allowed = () => active() && !shell.busy;
  const fresh = (generation: number, rev: number) =>
    active() && isCurrentAuthGeneration(shell, generation) && rev === revision;

  function reset(): void {
    revision += 1;
    loadId += 1;
    logLoadId += 1;
    Object.assign(state, createExtraSettingsState());
  }
  function changeTab(tab: SettingsTab): void {
    if (!SETTINGS_TABS.includes(tab) || state.tab === tab) return;
    state.tab = tab;
    state.error = null;
    state.notice = null;
  }

  async function refresh(): Promise<boolean> {
    if (!allowed() || state.busy) return false;
    const generation = shell.authGeneration, rev = revision, seq = ++loadId;
    state.loading = true;
    state.error = null;
    const [history, paths, log, liveNames] = await Promise.allSettled([
      Promise.all([invoke!("history_retention_settings"), invoke!("list_history_connections")]),
      invoke!("path_settings"),
      Promise.allSettled([invoke!("application_log_state"), invoke!("list_application_log_entries", { limit: 200 })]),
      invoke!("list_connections"),
    ]);
    if (!fresh(generation, rev) || seq !== loadId) return false;
    const failures: string[] = [];
    // A profile lookup failure must not prevent reading or displaying SQLite history.
    try { state.historyLiveNames = liveNames.status === "fulfilled" ? parseLiveHistoryNames(liveNames.value) : {}; }
    catch { state.historyLiveNames = {}; }
    try {
      if (history.status === "fulfilled") {
        const days = parseRetention(history.value[0]);
        const rows = parseHistoryRows(history.value[1]);
        state.retentionDays = days;
        state.historyRows = rows;
        state.historyStale = false;
        state.historyLoaded = true;
        state.historyError = null;
      } else {
        state.historyError = tr('tsSettingsExtra.historyRefreshFailed', { error: String(history.reason) });
        failures.push(state.historyError);
      }
    } catch (error) {
      state.historyError = tr('tsSettingsExtra.historyRefreshFailed', { error: String(error) });
      failures.push(state.historyError);
    }
    try {
      if (paths.status === "fulfilled") state.paths = parsePaths(paths.value);
      else failures.push(tr('tsSettingsExtra.pathsFailure', { error: String(paths.reason) }));
    } catch (error) { failures.push(tr('tsSettingsExtra.pathsFailure', { error: String(error) })); }
    if (log.status === "fulfilled") {
      const [logState, entries] = log.value;
      try {
        if (logState.status === "fulfilled") state.log = parseLogState(logState.value);
        else failures.push(tr('tsSettingsExtra.logFailure', { error: String(logState.reason) }));
      } catch (error) { failures.push(tr('tsSettingsExtra.logFailure', { error: String(error) })); }
      try {
        if (entries.status === "fulfilled") {
          state.logEntries = parseLogEntries(entries.value);
          state.logEntriesLimit = 200;
          state.logEntriesError = null;
        } else state.logEntriesError = String(entries.reason);
      } catch (error) { state.logEntriesError = String(error); }
    } else failures.push(tr('tsSettingsExtra.logFailure', { error: String(log.reason) }));
    state.error = failures.length ? failures.join(" | ") : null;
    state.loading = false;
    return failures.length === 0 && !state.logEntriesError;
  }

  /** Reload only the retention settings and aggregated history. Never turn a
   * failed read into an empty list, and never mask a successful write.
   */
  async function refreshHistory(): Promise<boolean> {
    if (!allowed() || state.busy || state.loading) return false;
    const generation = shell.authGeneration, rev = revision, seq = ++loadId;
    state.loading = true;
    state.historyError = null;
    try {
      const [history, liveNames] = await Promise.all([
        Promise.all([invoke!("history_retention_settings"), invoke!("list_history_connections")]),
        invoke!("list_connections").then(parseLiveHistoryNames).catch(() => ({} as Record<number, string>)),
      ]);
      const days = parseRetention(history[0]);
      const parsed = parseHistoryRows(history[1]);
      if (!fresh(generation, rev) || seq !== loadId) return false;
      state.retentionDays = days;
      state.historyRows = parsed;
      state.historyLiveNames = liveNames;
      state.historyLoaded = true;
      state.historyStale = false;
      return true;
    } catch (error) {
      if (fresh(generation, rev) && seq === loadId)
        state.historyError = tr('tsSettingsExtra.historyRefreshFailed', { error: String(error) });
      return false;
    } finally {
      if (fresh(generation, rev) && seq === loadId) state.loading = false;
    }
  }

  /** Read log state and a bounded sample without reloading history or paths.
   * The requested sample size is enforced by the existing Rust IPC (1..1000).
   * Keep previous entries if the read fails; do not present errors as empty data.
   */
  async function refreshLogEntries(limit: number = 200): Promise<boolean> {
    if (!allowed() || state.busy || !Number.isSafeInteger(limit) || limit < 1 || limit > 1000) return false;
    const generation = shell.authGeneration, rev = revision, seq = ++logLoadId;
    state.logEntriesLoading = true;
    const [logState, entries] = await Promise.allSettled([
      invoke!("application_log_state"),
      invoke!("list_application_log_entries", { limit }),
    ]);
    if (!fresh(generation, rev) || seq !== logLoadId) return false;
    let success = true;
    try {
      if (logState.status === "fulfilled") state.log = parseLogState(logState.value);
      else throw logState.reason;
    } catch (error) {
      state.logEntriesError = tr('tsSettingsExtra.logStateFailure', { error: String(error) });
      success = false;
    }
    try {
      if (entries.status === "fulfilled") {
        state.logEntries = parseLogEntries(entries.value);
        state.logEntriesLimit = limit;
        if (success) state.logEntriesError = null;
      } else throw entries.reason;
    } catch (error) {
      state.logEntriesError = String(error);
      success = false;
    }
    state.logEntriesLoading = false;
    return success;
  }

  /** A successful Rust mutation is never reported as a failed mutation just
   * because the subsequent UI refresh failed. Do not auto-retry the mutation.
   */
  async function mutate<T>(
    command: string, args: Record<string, unknown> | undefined,
    parse: (raw: unknown) => T, onSuccess: (result: T) => void,
    successMessage: (result: T) => string,
    confirmation?: { callback: ConfirmAction; message: string },
  ): Promise<boolean> {
    if (!allowed() || state.busy || state.loading) return false;
    const generation = shell.authGeneration, rev = revision;
    state.busy = true; // Hold the exclusive UI guard across confirm and IPC.
    state.error = null;
    state.notice = null;
    try {
      if (confirmation) {
        if (!await confirmation.callback(confirmation.message) || !fresh(generation, rev)) return false;
      }
      // A rejected IPC call is not proof of rollback; do not auto-retry writes.
      const raw: unknown = await invoke!(command, args);
      if (!fresh(generation, rev)) return false;
      let result: T;
      try { result = parse(raw); }
      catch (error) {
        state.error = tr('tsSettingsExtra.uncertainOperation', { command, error: String(error) });
        return false;
      }
      onSuccess(result);
      state.notice = successMessage(result);
      return true;
    } catch (error) {
      if (fresh(generation, rev)) state.error = String(error);
      return false;
    } finally {
      if (fresh(generation, rev)) state.busy = false;
    }
  }
  async function saveExportDirectory(directory: string): Promise<boolean> {
    directory = directory.trim();
    if (!directory) { state.error = tr('tsSettingsExtra.exportRequired'); return false; }
    return mutate("set_export_directory", { directory }, parsePaths,
      paths => { state.paths = paths; }, paths => tr('tsSettingsExtra.exportSaved', { path: paths.export_dir }));
  }
  async function migrateWorkspace(directory: string, confirm: ConfirmAction): Promise<boolean> {
    directory = directory.trim();
    if (!directory || !state.paths || directory === state.paths.workspace_root) return false;
    return mutate("migrate_workspace_root", { directory }, (raw) => {
      const r = record(raw, "migrate_workspace_root");
      return { settings: parsePaths(r.settings), warning: optionalText(r.warning, "warning") };
    }, result => { state.paths = result.settings; }, result =>
      tr('tsSettingsExtra.workspaceMoved', { path: result.settings.workspace_root, warning: result.warning ? ` ${result.warning}` : '' }),
      { callback: confirm, message: tr('tsSettingsExtra.workspaceConfirm', { directory }) });
  }
  async function migrateLogDirectory(directory: string, confirm: ConfirmAction): Promise<boolean> {
    directory = directory.trim();
    if (!directory || !state.paths || directory === state.paths.application_log_dir) return false;
    return mutate("set_application_log_directory", { directory }, parsePaths,
      paths => { state.paths = paths; state.log = null; state.logEntriesError = tr('tsSettingsExtra.logMoved'); },
      paths => tr('tsSettingsExtra.logMovedTo', { path: paths.application_log_dir }),
      { callback: confirm, message: tr('tsSettingsExtra.logMoveConfirm', { directory }) });
  }
  async function setLogSettings(maxTotalBytes: number, retentionDays: number): Promise<boolean> {
    if (!LOG_SIZE_LIMITS.includes(maxTotalBytes as typeof LOG_SIZE_LIMITS[number]) ||
        !LOG_RETENTION_DAYS.includes(retentionDays as typeof LOG_RETENTION_DAYS[number])) {
      state.error = tr('tsSettingsExtra.badLogSettings');
      return false;
    }
    return mutate("set_application_log_settings", { maxTotalBytes, retentionDays }, parseLogState,
      value => { state.log = value; }, () => tr('tsSettingsExtra.logSaved'));
  }
  async function clearLog(confirm: ConfirmAction): Promise<boolean> {
    return mutate("clear_application_log", undefined, parseLogState, value => {
      ++logLoadId; // Ignore reads started before the destructive clear.
      state.log = value; state.logEntries = []; state.logEntriesLimit = 200;
      state.logEntriesError = null;
    }, () => tr('tsSettingsExtra.logCleared'),
    { callback: confirm, message: tr('tsSettingsExtra.logClearConfirm') });
  }
  async function revealLog(): Promise<boolean> {
    return mutate("reveal_application_log_directory", undefined,
      value => textValue(value, "reveal_application_log_directory"),
      () => {}, directory => tr('tsSettingsExtra.logDirectoryOpen', { directory }));
  }
  async function setHistoryRetention(retentionDays: number | null): Promise<boolean> {
    if (!validateRetentionDays(retentionDays)) { state.error = tr('tsSettingsExtra.badHistoryRetention'); return false; }
    const saved = await mutate("set_history_retention", { retentionDays }, parseMaintenance,
      () => { state.retentionDays = retentionDays; state.historyStale = true; },
      result => tr('tsSettingsExtra.retentionSaved', { count: String(result.total_deleted) }));
    // A failure to reload is a separate read failure, not a failed mutation.
    if (saved) await refreshHistory();
    return saved;
  }
  async function deleteHistory(connectionId: number, confirm: ConfirmAction): Promise<boolean> {
    if (!Number.isSafeInteger(connectionId) || connectionId <= 0) return false;
    const row = state.historyRows.find(item => item.connection_id === connectionId);
    if (!row) return false;
    const message = tr('tsSettingsExtra.historyDeleteConfirm', { name: row.connection_name, id: String(connectionId) });
    return mutate("delete_connection_history", { connectionId }, parseMaintenance,
      () => { state.historyRows = state.historyRows.filter(item => item.connection_id !== connectionId); },
      result => tr('tsSettingsExtra.historyDeleted', { count: String(result.total_deleted) }),
      { callback: confirm, message });
  }
  async function runReadiness(connectionId: number | null): Promise<boolean> {
    if (connectionId !== null && (!Number.isSafeInteger(connectionId) || connectionId <= 0)) {
      state.error = tr('tsSettingsExtra.badConnection'); return false;
    }
    return mutate("runtime_readiness", { connectionId }, parseReadiness,
      value => { state.readiness = value; }, () => "");
  }
  return { reset, changeTab, refresh, refreshLogEntries, refreshHistory, saveExportDirectory, migrateWorkspace, migrateLogDirectory,
    setLogSettings, clearLog, revealLog, setHistoryRetention, deleteHistory, runReadiness };
}
