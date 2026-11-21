import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { parseActivityConnections, type ActivityConnection } from "../activity/controller.js";

/** Serde contracts: core/pgss/model.rs and core/history/pgss.rs. No SQL analysis in the UI. */
export interface PgssSection<T> { data: T | null; error: string | null }
export interface PgssOverview {
  measured_at: string; database: string; database_oid: number; username: string;
  server_version: string; server_version_num: number; extension_installed: boolean;
  extension_version: string | null; preloaded: boolean; shared_preload_libraries: string;
  compute_query_id: string | null; max_entries: number | null; track: string | null;
  track_utility: boolean | null; track_planning: boolean | null; save: boolean | null;
  track_io_timing: boolean;
}
export interface PgssStats { dealloc: number; stats_reset: string | null; stats_age_seconds: number | null }
export interface PgssCapacity {
  max_entries: number; current_entries: number; fill_pct: number; one_call_entries: number;
  one_call_pct: number; up_to_5_calls_entries: number; up_to_5_calls_pct: number;
  total_calls: number; total_exec_time_ms: number | null;
}
export interface PgssOwner {
  userid: number; dbid: number; role_name: string | null; database: string | null;
  entries: number; entries_pct: number; one_call_entries: number; one_call_pct: number;
  up_to_5_calls_entries: number; up_to_5_calls_pct: number; total_calls: number;
}
export interface PgssCommand extends PgssOwner { command: string }
export interface PgssCallTarget {
  userid: number; dbid: number; role_name: string | null; database: string | null;
  call_tag: string; call_target_name?: string | null; entries: number; entries_pct_of_max: number;
  one_call_entries: number; one_call_pct: number; total_calls: number;
}
export interface PgssRow {
  userid: number; dbid: number; role_name: string | null; database: string | null;
  toplevel: boolean; queryid: string | null; calls: number; plans: number;
  total_plan_time_ms: number; mean_plan_time_ms: number; stddev_plan_time_ms: number;
  total_exec_time_ms: number; min_exec_time_ms: number; max_exec_time_ms: number;
  mean_exec_time_ms: number; stddev_exec_time_ms: number; rows: number;
  shared_blks_hit: number; shared_blks_read: number; shared_blks_dirtied: number;
  shared_blks_written: number; temp_blks_read: number; temp_blks_written: number;
  blk_read_time_ms: number; blk_write_time_ms: number;
  wal_records: number; wal_fpi: number; wal_bytes: number;
}
export type PgssRank = "total_exec" | "mean_exec" | "temp_io" | "shared_reads" | "wal" | "variance" | "planning";
export const PGSS_RANKS: { id: PgssRank; label: string; explanation: string }[] = [
  { id: "total_exec", get label() { return t('tsFinish.rankTotalLabel'); }, get explanation() { return t('tsFinish.rankTotalHelp'); } },
  { id: "mean_exec", get label() { return t('tsFinish.rankMeanLabel'); }, get explanation() { return t('tsFinish.rankMeanHelp'); } },
  { id: "temp_io", label: "Temp I/O", get explanation() { return t('tsFinish.rankTempHelp'); } },
  { id: "shared_reads", label: "Shared reads", get explanation() { return t('tsFinish.rankSharedHelp'); } },
  { id: "wal", label: "WAL", get explanation() { return t('tsFinish.rankWalHelp'); } },
  { id: "variance", get label() { return t('tsFinish.rankVarianceLabel'); }, get explanation() { return t('tsFinish.rankVarianceHelp'); } },
  { id: "planning", get label() { return t('tsFinish.rankPlanLabel'); }, get explanation() { return t('tsFinish.rankPlanHelp'); } },
];
export type PgssCallsFilter = "all" | "one" | "few" | "many";
/** Filter only the entries actually stored in this snapshot's selected TOP ranking. */
export function pgssFilterCalls(rows: readonly PgssRow[], filter: PgssCallsFilter): PgssRow[] {
  if (filter === "one") return rows.filter(row => row.calls === 1);
  if (filter === "few") return rows.filter(row => row.calls >= 2 && row.calls <= 5);
  if (filter === "many") return rows.filter(row => row.calls > 5);
  return [...rows];
}
export interface PgssPerformance extends Record<PgssRank, PgssRow[]> { history_sample: PgssRow[] }
export interface PgssFinding { id: string; title: string; observed: string[]; meaning: string[]; next_steps: string[] }
export interface PgssInterpretation { items: PgssFinding[]; limitations: string[] }
export interface PgssSnapshot {
  overview: PgssOverview; stats: PgssSection<PgssStats>; capacity: PgssSection<PgssCapacity>;
  role_database: PgssSection<PgssOwner[]>; commands: PgssSection<PgssCommand[]>;
  call_targets: PgssSection<PgssCallTarget[]>; performance: PgssSection<PgssPerformance>;
  interpretation: PgssInterpretation;
}
export interface PgssExplorerFilter {
  userid?: number; dbid?: number; command?: string; call_tag?: string;
  queryid?: string; one_call?: boolean;
  min_calls?: number; max_calls?: number; min_total_exec_ms?: number;
  offset?: number; limit?: number;
  sort?: "queryid" | "role" | "total_exec" | "mean_exec" | "calls" | "wal" | "temp";
  direction?: "asc" | "desc";
}
export interface PgssExplorerPage {
  rows: PgssRow[]; total: number; offset: number; limit: number; has_more: boolean;
}
export function pgssExplorerPageValid(page: PgssExplorerPage, filter: PgssExplorerFilter): boolean {
  return !!page && Array.isArray(page.rows) && page.rows.length <= (filter.limit ?? 25) &&
    page.offset === (filter.offset ?? 0) && page.limit === (filter.limit ?? 25) &&
    Number.isSafeInteger(page.total) && page.total >= 0 &&
    (page.rows.length === 0 ? page.offset >= page.total : page.total >= page.offset + page.rows.length) &&
    page.has_more === (page.offset + page.limit < page.total) &&
    page.rows.every(row =>
      Number.isSafeInteger(row.userid) && Number.isSafeInteger(row.dbid) &&
      typeof row.toplevel === "boolean" &&
      (row.queryid === null || (typeof row.queryid === "string" && /^-?\d+$/.test(row.queryid))) &&
      Number.isFinite(row.calls) && (filter.userid == null || row.userid === filter.userid) &&
      (filter.dbid == null || row.dbid === filter.dbid) &&
      (!filter.one_call || row.calls === 1) &&
      (filter.min_calls == null || row.calls >= filter.min_calls) &&
      (filter.max_calls == null || row.calls <= filter.max_calls) &&
      (filter.min_total_exec_ms == null || row.total_exec_time_ms >= filter.min_total_exec_ms) &&
      (filter.queryid == null || row.queryid === filter.queryid));
}

export interface PgssSummary {
  id: number; schema_version: number; connection_id: number; connection_name: string;
  host: string; port: number; database: string; database_oid: number; username: string;
  sslmode: string; server_version: string; server_version_num: number;
  collection_started_at_ms: number; collection_completed_at_ms: number; measured_at: string;
  extension_installed: boolean; extension_version: string | null;
  stats_reset: string | null; dealloc: number | null; max_entries: number | null;
  current_entries: number | null; fill_pct: number | null; one_call_entries: number | null;
  up_to_5_calls_entries: number | null; unavailable_sections: number;
}
export interface PgssDelta {
  userid: number; dbid: number; role_name: string | null; database: string | null;
  older_entries: number | null; newer_entries: number | null;
  entries_delta: number | null; older_one_call_entries: number | null;
  newer_one_call_entries: number | null; one_call_delta: number | null; sample_state: string;
}
export interface PgssCommandDelta extends PgssDelta { command: string }
export interface PgssCallDelta extends PgssDelta { call_tag: string }
export interface PgssComparison {
  older: PgssSummary; newer: PgssSummary; elapsed_seconds: number;
  stats_period_continuous: boolean; dealloc_delta: number | null;
  dealloc_per_hour: number | null; current_entries_delta: number | null;
  fill_pct_delta: number | null; one_call_entries_delta: number | null;
  up_to_5_calls_entries_delta: number | null;
  role_database_changes: PgssDelta[]; command_changes: PgssCommandDelta[];
  call_target_changes: PgssCallDelta[]; interpretation: PgssInterpretation;
  limitations: string[];
}
export interface PgssDetail extends Pick<PgssRow,
  "userid" | "dbid" | "role_name" | "database" | "toplevel" | "calls" | "plans" |
  "total_plan_time_ms" | "mean_plan_time_ms" | "stddev_plan_time_ms" |
  "total_exec_time_ms" | "min_exec_time_ms" | "max_exec_time_ms" | "mean_exec_time_ms" |
  "stddev_exec_time_ms" | "rows" | "shared_blks_hit" | "shared_blks_read" |
  "temp_blks_read" | "temp_blks_written" | "wal_bytes"> {
  queryid: string; query: string | null; query_truncated?: boolean | null;
}
export interface PgssStatementPoint {
  snapshot_id: number; measured_at: string; stats_reset: string | null; dealloc: number | null;
  sample_scope: string; row: PgssRow; delta_state: string; elapsed_seconds: number | null;
  calls_delta: number | null; calls_per_hour: number | null;
  calls_workload_share_pct: number | null; plans_delta: number | null;
  total_exec_time_delta_ms: number | null; interval_mean_exec_time_ms: number | null;
  exec_workload_share_pct: number | null; total_plan_time_delta_ms: number | null;
  interval_mean_plan_time_ms: number | null; rows_delta: number | null;
  shared_blks_read_delta: number | null; shared_blks_hit_delta: number | null;
  temp_blks_read_delta: number | null; temp_blks_written_delta: number | null;
  wal_bytes_delta: number | null;
}
export interface PgssStatementHistory {
  anchor_snapshot_id: number; connection_id: number; userid: number; dbid: number;
  queryid: string; toplevel: boolean; role_name: string | null; database: string | null;
  anchor_sampled_entries: number; anchor_current_entries: number | null;
  anchor_sample_coverage_pct: number | null; anchor_sample_scope: string;
  summary: {
    history_points: number; history_span_seconds: number | null; comparable_intervals: number;
    latest_delta_state: string; latest_elapsed_seconds: number | null;
    calls_delta: number | null; calls_per_hour: number | null;
    calls_workload_share_pct: number | null; plans_delta: number | null;
    total_exec_time_delta_ms: number | null; interval_mean_exec_time_ms: number | null;
    exec_workload_share_pct: number | null; total_plan_time_delta_ms: number | null;
    interval_mean_plan_time_ms: number | null; rows_delta: number | null;
    shared_blks_read_delta: number | null; shared_blks_hit_delta: number | null;
    temp_blks_read_delta: number | null; temp_blks_written_delta: number | null;
    wal_bytes_delta: number | null;
  };
  change_analysis: {
    state: string; items: { id: string; label: string; unit: string; older_value: number;
      newer_value: number; absolute_delta: number; relative_change_pct: number | null;
      direction: string; evidence_level: string }[];
    limitations: string[];
  };
  points: PgssStatementPoint[]; limitations: string[];
}
export interface PgssReference {
  methodology: { id: string; title: string; purpose: string; how_to_read: string[] }[];
  queries: { id: string; title: string; purpose: string; output: string[]; how_to_read: string[]; sql: string }[];
  sources: { id: string; title: string; purpose: string; path: string }[];
}
export interface PgssState {
  connections: ActivityConnection[]; connectionId: number | null;
  summaries: PgssSummary[]; snapshot: PgssSnapshot | null; snapshotSummary: PgssSummary | null;
  snapshotSource: "collected" | "archive" | null;
  loadingConnections: boolean; loadingHistory: boolean; loadingSnapshot: boolean;
  collecting: boolean; comparing: boolean; loadingReference: boolean;
  reference: PgssReference | null; comparison: PgssComparison | null;
  olderId: number | null; newerId: number | null;
  explorer: PgssExplorerPage | null; explorerLoading: boolean; explorerError: string | null;
  selectedLive: boolean; selectedRow: PgssRow | null; statement: PgssStatementHistory | null;
  detail: PgssDetail | null; loadingStatement: boolean; loadingDetail: boolean;
  statementError: string | null; detailError: string | null;
  error: string | null; notice: string | null;
}
export function createPgssState(): PgssState {
  return {
    connections: [], connectionId: null, summaries: [], snapshot: null, snapshotSummary: null,
    snapshotSource: null,
    loadingConnections: false, loadingHistory: false, loadingSnapshot: false,
    collecting: false, comparing: false, loadingReference: false,
    reference: null, comparison: null, olderId: null, newerId: null,
    explorer: null, explorerLoading: false, explorerError: null, selectedLive: false,
    selectedRow: null, statement: null, detail: null, loadingStatement: false, loadingDetail: false,
    statementError: null, detailError: null, error: null, notice: null,
  };
}
export function pgssSameTarget(a: PgssSummary, b: PgssSummary): boolean {
  return a.connection_id === b.connection_id && a.host === b.host && a.port === b.port &&
    a.database === b.database && a.database_oid === b.database_oid &&
    a.username === b.username && a.sslmode === b.sslmode;
}
export function pgssMatchesConnection(s: PgssSummary | null, c: ActivityConnection | undefined): boolean {
  return !!s && !!c && s.connection_id === c.id && s.host === c.host &&
    s.port === c.port && s.database === c.database && s.username === c.username;
}
export function pgssIdentity(row: PgssRow): string {
  return `${row.userid}:${row.dbid}:${row.queryid ?? "NULL"}:${row.toplevel}`;
}
export function pgssDocsUrl(serverVersionNum: number | null, path: string): string | null {
  if (!/^[a-z0-9._-]+(?:#[A-Za-z0-9_-]+)?$/.test(path)) return null;
  const major = Math.floor((serverVersionNum ?? 0) / 10000);
  return `https://www.postgresql.org/docs/${major >= 10 && major <= 99 ? major : "current"}/${path}`;
}
/** Stable, immutable sorting for DBA tables; unavailable values always appear last. */
export function pgssSortRows<T extends object>(input: readonly T[], sort: keyof T, descending: boolean): T[] {
  return input.map((row, index) => ({ row, index })).sort((a, b) => {
    const av = a.row[sort], bv = b.row[sort];
    if (av == null || (typeof av === "number" && !Number.isFinite(av))) {
      return bv == null || (typeof bv === "number" && !Number.isFinite(bv)) ? a.index - b.index : 1;
    }
    if (bv == null || (typeof bv === "number" && !Number.isFinite(bv))) return -1;
    const cmp = typeof av === "number" && typeof bv === "number"
      ? Math.sign(av - bv)
      : String(av).localeCompare(String(bv), "ru", { numeric: true });
    return (descending ? -Math.sign(cmp) : Math.sign(cmp)) || a.index - b.index;
  }).map(entry => entry.row);
}

export type PgssSortKey = keyof PgssRow;
export function pgssPage<T extends object>(
  input: readonly T[], search: string, sort: keyof T, descending: boolean, page: number, size: number,
): { rows: T[]; total: number; page: number; pages: number } {
  const needle = search.trim().toLocaleLowerCase("ru");
  const rows = input.filter(row => !needle ||
    ["role_name", "database", "queryid", "userid", "dbid", "command", "call_tag", "label", "scope"]
      .some(key => String((row as Record<string, unknown>)[key] ?? "").toLocaleLowerCase("ru").includes(needle)));
  const ordered = pgssSortRows(rows, sort, descending);
  const pageSize = [10, 25, 50, 100].includes(size) ? size : 25;
  const pages = Math.max(1, Math.ceil(ordered.length / pageSize));
  const current = Math.min(pages, Math.max(1, Math.trunc(page) || 1));
  return { rows: ordered.slice((current - 1) * pageSize, current * pageSize), total: ordered.length, page: current, pages };
}

export function createPgssController(shell: ShellState, state: PgssState, invoke: PgXrayInvoke) {
  let generation = 0, connectionRequest = 0, historyRequest = 0, snapshotRequest = 0;
  let statementRequest = 0, detailRequest = 0, comparisonRequest = 0, referenceRequest = 0, explorerRequest = 0;
  let disposed = false;
  const current = (auth: number, epoch: number) =>
    !disposed && shell.screen === "workspace" && shell.authGeneration === auth && generation === epoch;
  function clearStatement(): void {
    statementRequest++; detailRequest++;
    state.selectedLive = false; state.selectedRow = null; state.statement = null; state.detail = null;
    state.loadingStatement = false; state.loadingDetail = false;
    state.statementError = null; state.detailError = null;
  }
  function clearSnapshot(): void {
    snapshotRequest++; comparisonRequest++;
    state.snapshot = null; state.snapshotSummary = null; state.snapshotSource = null; state.comparison = null;
    state.loadingSnapshot = false; state.comparing = false;
    explorerRequest++; state.explorer = null; state.explorerLoading = false; state.explorerError = null;
    clearStatement();
  }
  function reset(): void {
    generation++; connectionRequest++; historyRequest++; referenceRequest++;
    clearSnapshot(); Object.assign(state, createPgssState());
  }
  function dispose(): void { reset(); disposed = true; }
  function selectConnection(id: number | null): void {
    generation++; historyRequest++;
    state.connectionId = state.connections.some(c => c.id === id) ? id : null;
    state.summaries = []; state.olderId = null; state.newerId = null;
    state.error = null; state.notice = null; state.loadingHistory = false;
    clearSnapshot();
    if (state.connectionId !== null) void refreshHistory(true);
  }
  async function refreshConnections(): Promise<void> {
    if (state.loadingConnections || shell.screen !== "workspace") return;
    const auth = shell.authGeneration, request = ++connectionRequest;
    state.loadingConnections = true;
    try {
      const connections = parseActivityConnections(await invoke("list_connections"));
      if (disposed || shell.authGeneration !== auth || shell.screen !== "workspace" || request !== connectionRequest) return;
      state.connections = connections;
      const id = connections.some(c => c.id === state.connectionId) ? state.connectionId : (connections[0]?.id ?? null);
      if (id !== state.connectionId) selectConnection(id);
      else if (id !== null) await refreshHistory(false);
      state.notice = t('diagTs.connectionsUpdated', { count: String(connections.length) });
    } catch (error) {
      if (!disposed && shell.authGeneration === auth && request === connectionRequest) state.error = t('diagTs.connectionsError', { error: String(error) });
    } finally {
      if (!disposed && shell.authGeneration === auth && request === connectionRequest) state.loadingConnections = false;
    }
  }
  async function refreshHistory(openIfEmpty = false): Promise<void> {
    const id = state.connectionId;
    if (id === null || shell.screen !== "workspace") return;
    const auth = shell.authGeneration, epoch = generation, request = ++historyRequest;
    state.loadingHistory = true; state.error = null;
    try {
      const summaries = await invoke<PgssSummary[]>("list_pgss_snapshots", { connectionId: id, limit: 100 });
      if (!current(auth, epoch) || request !== historyRequest || state.connectionId !== id) return;
      if (!Array.isArray(summaries) || summaries.some(s => s.connection_id !== id)) throw Error(t('diagTs.historyMismatch'));
      state.summaries = summaries;
      if (!summaries.some(s => s.id === state.olderId)) state.olderId = summaries[1]?.id ?? null;
      if (!summaries.some(s => s.id === state.newerId)) state.newerId = summaries[0]?.id ?? null;
      if (openIfEmpty && !state.snapshot && summaries.length) void openSnapshot(summaries[0]!.id);
      state.notice = t('diagTs.pgssHistoryUpdated', { count: String(summaries.length) });
    } catch (error) {
      if (current(auth, epoch) && request === historyRequest) state.error = t('diagTs.pgssHistoryError', { error: String(error) });
    } finally {
      if (current(auth, epoch) && request === historyRequest) state.loadingHistory = false;
    }
  }
  async function openSnapshot(snapshotId: number): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || !state.summaries.some(s => s.id === snapshotId) || state.collecting) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++snapshotRequest;
    state.loadingSnapshot = true; state.error = null;
    clearStatement(); state.comparison = null;
    try {
      const value = await invoke<{ history: PgssSummary; snapshot: PgssSnapshot }>("load_pgss_snapshot", { connectionId: id, snapshotId });
      if (!current(auth, epoch) || request !== snapshotRequest || state.connectionId !== id) return false;
      if (value.history.id !== snapshotId || value.history.connection_id !== id ||
          value.history.database_oid !== value.snapshot.overview.database_oid ||
          value.history.database !== value.snapshot.overview.database)
        throw Error(t('diagTs.pgssArchiveMismatch'));
      state.snapshot = value.snapshot; state.snapshotSummary = value.history; state.snapshotSource = "archive";
      state.notice = t('diagTs.pgssArchived', { id: String(snapshotId) });
      return true;
    } catch (error) {
      if (current(auth, epoch) && request === snapshotRequest) {
        const message = String(error);
        state.error = /Command load_pgss_snapshot not found/i.test(message)
          ? t('diagTs.pgssTauriOutdated')
          : t('diagTs.pgssOpenError', { error: message });
      }
      return false;
    } finally {
      if (current(auth, epoch) && request === snapshotRequest) state.loadingSnapshot = false;
    }
  }
  async function collect(): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || state.collecting || state.loadingSnapshot || shell.screen !== "workspace") return false;
    const auth = shell.authGeneration, epoch = generation;
    state.collecting = true; state.error = null; state.notice = null;
    try {
      const result = await invoke<{ history: PgssSummary; snapshot: PgssSnapshot }>("collect_pgss_snapshot", { connectionId: id });
      if (!current(auth, epoch) || state.connectionId !== id) return false;
      if (result.history.connection_id !== id || result.history.database_oid !== result.snapshot.overview.database_oid)
        throw Error(t('diagTs.pgssCollectedMismatch'));
      snapshotRequest++; clearStatement();
      state.snapshot = result.snapshot; state.snapshotSummary = result.history; state.snapshotSource = "collected";
      state.comparison = null;
      state.notice = t('diagTs.pgssSaved', { id: String(result.history.id) });
      await refreshHistory(false);
      if (current(auth, epoch)) state.notice = t('diagTs.pgssSaved', { id: String(result.history.id) });
      return true;
    } catch (error) {
      if (current(auth, epoch)) state.error = t('diagTs.pgssCollectError', { error: String(error) });
      return false;
    } finally { if (current(auth, epoch)) state.collecting = false; }
  }
  async function compare(): Promise<boolean> {
    const first = state.olderId, second = state.newerId;
    const a = state.summaries.find(s => s.id === first), b = state.summaries.find(s => s.id === second);
    if (!a || !b || first === second || !pgssSameTarget(a, b) || state.comparing) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++comparisonRequest;
    state.comparing = true; state.error = null; state.comparison = null;
    try {
      const result = await invoke<PgssComparison>("compare_pgss_snapshots", { firstId: a.id, secondId: b.id });
      if (!current(auth, epoch) || request !== comparisonRequest ||
          state.olderId !== a.id || state.newerId !== b.id) return false;
      if (result.older.id !== a.id && result.older.id !== b.id) throw Error(t('diagTs.pgssUnexpectedComparison'));
      if (![a.id, b.id].includes(result.newer.id) || result.older.id === result.newer.id) throw Error(t('diagTs.pgssPairMismatch'));
      state.comparison = result;
      state.notice = t('diagTs.pgssCompared', { older: String(result.older.id), newer: String(result.newer.id) });
      return true;
    } catch (error) {
      if (current(auth, epoch) && request === comparisonRequest) state.error = t('diagTs.pgssCompareError', { error: String(error) });
      return false;
    } finally { if (current(auth, epoch) && request === comparisonRequest) state.comparing = false; }
  }
  async function refreshExplorer(filters: PgssExplorerFilter): Promise<boolean> {
    const id = state.connectionId, anchor = state.snapshotSummary;
    const profile = state.connections.find(c => c.id === id);
    if (id === null || !profile || shell.screen !== "workspace" ||
        (anchor !== null && !pgssMatchesConnection(anchor, profile))) {
      state.explorerError = t('diagTs.pgssExplorerUnavailable');
      return false;
    }
    const auth = shell.authGeneration, epoch = generation, request = ++explorerRequest;
    state.explorerLoading = true; state.explorerError = null; state.explorer = null;
    try {
      const result = await invoke<PgssExplorerPage>("pgss_explore", { connectionId: id, filters });
      if (!current(auth, epoch) || request !== explorerRequest || state.connectionId !== id) return false;
      if (!pgssExplorerPageValid(result, filters)) throw Error(t('diagTs.pgssExplorerInvalid'));
      state.explorer = result;
      return true;
    } catch (error) {
      if (current(auth, epoch) && request === explorerRequest) state.explorerError = `PGSS Explorer: ${String(error)}`;
      return false;
    } finally {
      if (current(auth, epoch) && request === explorerRequest) state.explorerLoading = false;
    }
  }
  function selectLiveStatement(row: PgssRow): void {
    clearStatement();
    state.selectedLive = true;
    state.selectedRow = row;
    state.statementError = t('diagTs.pgssLiveStatement');
  }
  async function selectStatement(row: PgssRow): Promise<boolean> {
    const anchor = state.snapshotSummary;
    if (!row.queryid || !anchor || state.connectionId !== anchor.connection_id) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++statementRequest;
    detailRequest++; state.selectedLive = false; state.selectedRow = row; state.statement = null; state.detail = null;
    state.statementError = null; state.detailError = null; state.loadingStatement = true;
    try {
      const result = await invoke<PgssStatementHistory>("pgss_statement_history", {
        snapshotId: anchor.id, userid: row.userid, dbid: row.dbid,
        queryid: row.queryid, toplevel: row.toplevel, limit: 100,
      });
      if (!current(auth, epoch) || request !== statementRequest) return false;
      if (result.anchor_snapshot_id !== anchor.id || result.connection_id !== anchor.connection_id ||
          result.userid !== row.userid || result.dbid !== row.dbid || result.queryid !== row.queryid ||
          result.toplevel !== row.toplevel) throw Error(t('diagTs.pgssStatementMismatch'));
      state.statement = result;
      return true;
    } catch (error) {
      if (current(auth, epoch) && request === statementRequest) state.statementError = t('diagTs.pgssStatementHistoryError', { error: String(error) });
      return false;
    } finally { if (current(auth, epoch) && request === statementRequest) state.loadingStatement = false; }
  }
  async function refreshDetail(): Promise<boolean> {
    const row = state.selectedRow, anchor = state.snapshotSummary, id = state.connectionId;
    const profile = state.connections.find(c => c.id === id);
    if (!row?.queryid || id === null || !profile || state.loadingDetail ||
        (anchor !== null && !pgssMatchesConnection(anchor, profile))) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++detailRequest;
    state.loadingDetail = true; state.detailError = null; state.detail = null;
    try {
      const result = await invoke<PgssDetail>("pgss_query_detail", {
        connectionId: id, userid: row.userid, dbid: row.dbid, queryid: row.queryid, toplevel: row.toplevel,
      });
      if (!current(auth, epoch) || request !== detailRequest || !state.selectedRow ||
          pgssIdentity(state.selectedRow) !== pgssIdentity(row)) return false;
      if (result.userid !== row.userid || result.dbid !== row.dbid || result.queryid !== row.queryid ||
          result.toplevel !== row.toplevel) throw Error(t('diagTs.pgssStatementResponseMismatch'));
      state.detail = result;
      return true;
    } catch (error) {
      if (current(auth, epoch) && request === detailRequest) state.detailError = t('diagTs.pgssLiveDetailError', { error: String(error) });
      return false;
    } finally { if (current(auth, epoch) && request === detailRequest) state.loadingDetail = false; }
  }
  async function loadReference(): Promise<void> {
    if (state.reference || state.loadingReference || shell.screen !== "workspace") return;
    const auth = shell.authGeneration, epoch = generation, request = ++referenceRequest;
    state.loadingReference = true;
    try {
      const result = await invoke<PgssReference>("pgss_reference");
      if (current(auth, epoch) && request === referenceRequest) state.reference = result;
    } catch (error) {
      if (current(auth, epoch) && request === referenceRequest) state.error = `PGSS Reference: ${String(error)}`;
    } finally { if (current(auth, epoch) && request === referenceRequest) state.loadingReference = false; }
  }
  return { reset, dispose, selectConnection, refreshConnections, refreshHistory, openSnapshot,
    collect, compare, refreshExplorer, selectLiveStatement, selectStatement, refreshDetail, loadReference, clearStatement };
}
