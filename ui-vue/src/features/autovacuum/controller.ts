import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { parseActivityConnections, type ActivityConnection } from "../activity/controller.js";

/** Contracts follow crates/core/src/autovacuum/model.rs and history/autovacuum.rs.
 * No VACUUM/ANALYZE/ALTER SQL is constructed or executed by this controller. */
export interface Section<T> { data: T | null; error: string | null }
export interface InterpretationItem {
  id: string; severity: "info" | "attention" | "warning"; title: string;
  observed: string[]; meaning: string[]; next_steps: string[];
}
export interface Interpretation { items: InterpretationItem[]; limitations: string[] }
export interface AvToast {
  relation_oid: number; relation_name: string; dead_tuples: number; total_size_bytes: number;
  autovacuum_count: number; last_autovacuum: string | null;
  dead_tuple_trigger: number; insert_trigger: number | null;
  inserts_since_vacuum?: number;
  dead_trigger_used_pct?: number | null; insert_trigger_used_pct?: number | null;
  dead_trigger_reached: boolean; insert_trigger_reached: boolean;
}
export interface AvTable {
  relation_oid: number; schema_name: string; relation_name: string; relation_type: string;
  autovacuum_applicable: boolean; autovacuum_enabled: boolean;
  estimated_rows: number; live_tuples: number; dead_tuples: number; dead_tuple_pct: number | null;
  total_size_bytes: number; heap_size_bytes: number; indexes_size_bytes: number; index_count: number;
  tuples_inserted: number; tuples_updated: number; tuples_deleted: number;
  hot_updates: number; hot_update_pct: number | null; workload_profile: string;
  modifications_since_analyze: number; inserts_since_vacuum: number;
  dead_tuple_trigger: number; insert_trigger: number | null; analyze_trigger: number;
  dead_trigger_used_pct: number | null; insert_trigger_used_pct: number | null; analyze_trigger_used_pct: number | null;
  dead_trigger_reached: boolean; insert_trigger_reached: boolean; analyze_trigger_reached: boolean;
  last_vacuum: string | null; last_autovacuum: string | null; last_analyze: string | null; last_autoanalyze: string | null;
  vacuum_count: number; autovacuum_count: number; analyze_count: number; autoanalyze_count: number;
  effective_vacuum_scale_factor: number; effective_insert_scale_factor: number; effective_analyze_scale_factor: number;
  toast: AvToast | null;
}
export interface AvProgress {
  pid: number; relation_oid: number; schema_name: string; relation_name: string;
  phase: string; is_autovacuum?: boolean; is_autoanalyze?: boolean;
  scan_pct?: number | null; vacuum_pct?: number | null; sample_pct?: number | null;
  wait_event_type?: string | null; wait_event?: string | null;
}
export interface AvSnapshot {
  overview: { measured_at: string; database: string; database_oid: number; username: string;
    server_version_num: number; server_version: string; track_counts: boolean;
    autovacuum: boolean; autovacuum_naptime: string; stats_reset: string | null;
    autovacuum_max_workers: number };
  tables: Section<AvTable[]>; vacuums: Section<AvProgress[]>; analyzes: Section<AvProgress[]>;
  interpretation: Interpretation;
}
export interface AvSummary {
  id: number; connection_id: number; host: string; port: number; database: string;
  database_oid: number; username: string; sslmode: string; connection_name: string;
  server_version_num: number; collection_started_at_ms: number; collection_completed_at_ms: number;
  table_count: number; dead_trigger_reached_count: number; analyze_trigger_reached_count: number;
  insert_trigger_reached_count: number; unavailable_sections: number;
}
export interface AvIndex {
  index_oid: number; index_name: string; is_primary: boolean; is_unique: boolean;
  index_size_bytes: number; indexed_columns: string[]; included_columns: string[]; predicate: string | null;
}
export interface AvTableDetail {
  table: AvTable; indexes: Section<AvIndex[]>;
  update_statements: Section<{
    queryid: string | null; userid: number | null; dbid: number | null; toplevel: boolean | null;
    relation_oid: number; evidence_level: "parsed_qualified" | "heuristic_unqualified";
    evidence_note: string; calls: number; mean_exec_time_ms: number;
    total_exec_time_ms: number; rows: number; query: string;
  }[]>;
  interpretation: Interpretation;
  fixed_scale_candidates: { scale_factor: number; resulting_dead_tuple_trigger: number }[];
}
export interface AvFinding {
  id: string; area: string; status: string; title: string; summary: string;
  observed: string[]; policy_parameters: string[]; next_steps: string[];
}
export interface AvAdvice {
  relation_oid: number; relation_name: string; status: string; title: string; summary: string;
  history_points: number; history_span_seconds: number | null; stats_period_continuous: boolean;
  inserts_per_hour: number | null; updates_per_hour: number | null; deletes_per_hour: number | null;
  hot_update_pct: number | null; vacuum_cycles: number | null; autovacuum_cycles: number | null;
  analyze_cycles: number | null; autoanalyze_cycles: number | null; toast_vacuum_cycles: number | null;
  findings: AvFinding[]; limitations: string[];
}
export interface AvCandidate {
  id: string; area: string; scenario: string; target_relation: string; parameter: string;
  current_scale_factor: number; candidate_scale_factor: number; current_trigger: number; candidate_trigger: number;
  current_source: string; observed_rate_per_hour: number; sql_preview: string; rationale: string[];
}
export interface AvCandidates {
  relation_oid: number; relation_name: string; status: string; summary: string;
  candidates: AvCandidate[]; limitations: string[];
}
export interface AvSimulation {
  relation_oid: number; relation_name: string; estimated_rows: number;
  current_dead_tuple_trigger: number; target_dead_tuples: number;
  candidate_scale_factor: number | null; resulting_dead_tuple_trigger: number | null; notes: string[];
}
export interface AvRelationHistory {
  anchor_snapshot_id: number; relation_oid: number; relation_name: string;
  summary: { history_points: number; history_span_seconds: number | null; stats_period_continuous: boolean;
    inserts_per_hour: number | null; updates_per_hour: number | null; deletes_per_hour: number | null;
    vacuum_cycles: number | null; analyze_cycles: number | null };
  points: { snapshot_id: number; collection_started_at_ms: number; stats_reset?: string | null; table: AvTable }[];
  limitations: string[];
}
export interface AvComparison {
  older: AvSummary; newer: AvSummary; elapsed_seconds: number; stats_period_continuous: boolean;
  table_changes: { relation_oid: number; newer_name: string | null; older_name: string | null;
    dead_tuples_delta: number | null; inserts_delta: number | null; updates_delta: number | null;
    autovacuum_count_delta: number | null; vacuum_count_delta: number | null;
    autoanalyze_count_delta: number | null; analyze_count_delta: number | null;
    total_size_delta_bytes: number | null }[];
  interpretation: Interpretation; limitations: string[];
}
export interface AvReference {
  methodology: { id: string; title: string; purpose: string; how_to_read: string[] }[];
  queries: { id: string; title: string; purpose: string; sql: string; output: string[]; how_to_read: string[] }[];
  sources: { id: string; title: string; purpose: string; path: string }[];
}
export interface AvApplyResult {
  candidate: AvCandidate;
  verification: { before: { scale_factor: number; absolute_trigger: number; source: string };
    after: { scale_factor: number; absolute_trigger: number; source: string };
    rollback_sql: string | null; rollback_note: string };
  history: AvSummary | null; snapshot: AvSnapshot | null; warning: string | null;
}
export interface AvState {
  connections: ActivityConnection[]; connectionId: number | null;
  histories: AvSummary[]; snapshot: AvSnapshot | null; snapshotId: number | null;
  snapshotConnectionId: number | null; comparison: AvComparison | null;
  compareOlderId: number | null; compareNewerId: number | null;
  selectedTableOid: number | null; detail: AvTableDetail | null;
  advice: AvAdvice | null; candidates: AvCandidates | null; relationHistory: AvRelationHistory | null;
  detailErrors: string[]; simulation: AvSimulation | null; reference: AvReference | null;
  pendingCandidateId: string | null; applyResult: AvApplyResult | null; applyBlocked: boolean;
  loading: boolean; collecting: boolean; comparing: boolean; detailLoading: boolean;
  simulating: boolean; applying: boolean; error: string | null; notice: string | null;
}
export function createAvState(): AvState {
  return { connections: [], connectionId: null, histories: [], snapshot: null, snapshotId: null,
    snapshotConnectionId: null, comparison: null, compareOlderId: null, compareNewerId: null,
    selectedTableOid: null, detail: null, advice: null, candidates: null, relationHistory: null,
    detailErrors: [], simulation: null, reference: null, pendingCandidateId: null, applyResult: null,
    applyBlocked: false, loading: false, collecting: false, comparing: false, detailLoading: false,
    simulating: false, applying: false, error: null, notice: null };
}
export function sameAvTarget(a: AvSummary, b: AvSummary): boolean {
  return a.connection_id === b.connection_id && a.host === b.host && a.port === b.port &&
    a.database === b.database && a.database_oid === b.database_oid && a.username === b.username && a.sslmode === b.sslmode;
}
export function avSummaryMatchesConnection(summary: AvSummary | undefined, connection: ActivityConnection | undefined): boolean {
  return !!summary && !!connection && summary.connection_id === connection.id &&
    summary.host === connection.host && summary.port === connection.port &&
    summary.database === connection.database && summary.username === connection.username;
}
export function avDocsUrl(version: number | null, path: string): string | null {
  if (!/^[a-z0-9.-]+(?:#[A-Za-z0-9_-]+)?$/.test(path)) return null;
  const major = version === null ? 0 : Math.floor(version / 10000);
  return `https://www.postgresql.org/docs/${major >= 10 && major <= 99 ? major : "current"}/${path}`;
}
export type AvTableSortKey = "name" | "size" | "dead" | "vacuum" | "insert" | "analyze" | "status" | "updated" | "inserted" | "deleted" | "hot";
export type AvTablePressureKind = "dead" | "insert" | "analyze";
/** Explicit numerator / denominator: percentage of this specific maintenance trigger,
 * NEVER percentage of relation size, live tuples, or heap bloat. */
export function avTablePressure(table: AvTable, kind: AvTablePressureKind, reliable: boolean):
  { observed: number; trigger: number; percent: number | null } | null {
  if (!reliable) return null;
  const observed = kind === "dead" ? table.dead_tuples :
    kind === "insert" ? table.inserts_since_vacuum : table.modifications_since_analyze;
  const trigger = kind === "dead" ? table.dead_tuple_trigger :
    kind === "insert" ? table.insert_trigger : table.analyze_trigger;
  const percent = kind === "dead" ? table.dead_trigger_used_pct :
    kind === "insert" ? table.insert_trigger_used_pct : table.analyze_trigger_used_pct;
  if (typeof observed !== "number" || !Number.isFinite(observed) || observed < 0 ||
      typeof trigger !== "number" || !Number.isFinite(trigger) || trigger <= 0) return null;
  return { observed, trigger, percent: typeof percent === "number" && Number.isFinite(percent) && percent >= 0 ? percent : null };
}
export type AvTableSortDirection = "asc" | "desc";
const tableNameCollator = new Intl.Collator("ru", { numeric: true, sensitivity: "base" });

/** Sort the entire snapshot before slicing the visible page. No database paging is implied. */
export function avPage(snapshot: AvSnapshot | null, search: string, mode: string, page: number, pageSize: number,
  sortKey: AvTableSortKey = "name", sortDirection: AvTableSortDirection = "asc") {
  const needle = search.trim().toLocaleLowerCase();
  const reliable = !!snapshot?.overview.track_counts && !!snapshot?.tables.data;
  const rows = (snapshot?.tables.data ?? []).filter(t => {
    if (needle && !`${t.schema_name}.${t.relation_name} ${t.relation_oid}`.toLocaleLowerCase().includes(needle)) return false;
    if (mode === "vacuum") return reliable && (t.dead_trigger_reached || t.insert_trigger_reached);
    if (mode === "dead") return reliable && t.dead_trigger_reached;
    if (mode === "insert") return reliable && t.insert_trigger_reached;
    if (mode === "analyze") return reliable && t.analyze_trigger_reached;
    if (mode === "disabled") return t.autovacuum_applicable && !t.autovacuum_enabled;
    return true;
  });
  const numberOrUnknown = (value: number | null | undefined): number | null =>
    typeof value === "number" && Number.isFinite(value) ? value : null;
  const sortValue = (t: AvTable): number | null => {
    if (sortKey === "size") return numberOrUnknown(t.total_size_bytes);
    if (sortKey === "status") {
      if (t.autovacuum_applicable && !t.autovacuum_enabled) return 3;
      if (!reliable) return null;
      return t.dead_trigger_reached || t.insert_trigger_reached || t.analyze_trigger_reached ? 2 : 1;
    }
    if (!reliable) return null;
    if (sortKey === "dead") return numberOrUnknown(t.dead_tuples);
    if (sortKey === "vacuum") return numberOrUnknown(t.dead_trigger_used_pct);
    if (sortKey === "insert") return t.insert_trigger === null ? null : numberOrUnknown(t.insert_trigger_used_pct);
    if (sortKey === "analyze") return numberOrUnknown(t.analyze_trigger_used_pct);
    if (sortKey === "updated") return numberOrUnknown(t.tuples_updated);
    if (sortKey === "inserted") return numberOrUnknown(t.tuples_inserted);
    if (sortKey === "deleted") return numberOrUnknown(t.tuples_deleted);
    if (sortKey === "hot") return numberOrUnknown(t.hot_update_pct);
    return null;
  };
  const byName = (a: AvTable, b: AvTable) =>
    tableNameCollator.compare(`${a.schema_name}.${a.relation_name}`, `${b.schema_name}.${b.relation_name}`) ||
    a.relation_oid - b.relation_oid;
  const direction = sortDirection === "desc" ? -1 : 1;
  rows.sort((a, b) => {
    if (sortKey === "name") return direction * byName(a, b);
    const aValue = sortValue(a), bValue = sortValue(b);
    // Unknown values must remain at the end in BOTH directions, never become zero.
    if (aValue === null) return bValue === null ? byName(a, b) : 1;
    if (bValue === null) return -1;
    return direction * (aValue - bValue) || byName(a, b);
  });
  const size = [10, 25, 50].includes(pageSize) ? pageSize : 25;
  const pages = Math.max(1, Math.ceil(rows.length / size));
  const currentPage = Math.max(1, Math.min(pages, Math.trunc(page) || 1));
  return { rows: rows.slice((currentPage - 1) * size, currentPage * size), page: currentPage, pages, total: rows.length };
}
export function avSummaryCounts(snapshot: AvSnapshot | null) {
  const rows = snapshot?.tables.data;
  const reliable = snapshot?.overview.track_counts === true && !!rows;
  return { total: rows?.length ?? null,
    vacuum: reliable ? rows.filter(t => t.dead_trigger_reached || t.insert_trigger_reached).length : null,
    dead: reliable ? rows.filter(t => t.dead_trigger_reached).length : null,
    insert: reliable ? rows.filter(t => t.insert_trigger_reached).length : null,
    analyze: reliable ? rows.filter(t => t.analyze_trigger_reached).length : null,
    disabled: rows ? rows.filter(t => t.autovacuum_applicable && !t.autovacuum_enabled).length : null,
    activeVacuum: snapshot?.vacuums.data?.length ?? null,
    activeAnalyze: snapshot?.analyzes.data?.length ?? null };
}
/** SQLite snapshot IDs are global, not ordinal numbers for an individual connection. */
export function avSnapshotLabel(summary: AvSummary, includeConnection = false): string {
  const when = new Date(summary.collection_completed_at_ms);
  const stamp = Number.isFinite(when.getTime())
    ? when.toLocaleString("ru-RU") : t('tsFinish.avTimeUnknown');
  const source = summary.connection_name?.trim() || `${summary.host}:${summary.port}/${summary.database}`;
  return includeConnection ? `${source} · ${summary.database} · ${stamp}` : stamp;
}

/** A NULL reset timestamp is not equivalent to disabled statistics or a proven reset.
 * Rust intentionally refuses counter-based maintenance rates without a known reset boundary. */
export function avStatsResetExplanation(trackCounts: boolean, statsReset: string | null): string | null {
  if (!trackCounts) return t('tsFinish.avTrackCountsOff');
  if (statsReset === null) return t('tsFinish.avUnknownReset');
  return null;
}

export function avFormatFactor(value: number | null | undefined): string {
  return value === null || value === undefined || !Number.isFinite(value)
    ? "—" : value.toLocaleString("ru-RU", { maximumSignificantDigits: 6 });
}

/** Read-only comparison: this predicts a threshold, not autovacuum scheduling or I/O cost. */
export function avSimulationComparison(table: AvTable, sim: AvSimulation) {
  const original = table.dead_tuple_trigger;
  const proposed = sim.resulting_dead_tuple_trigger;
  if (!Number.isFinite(original) || original <= 0 || proposed === null || !Number.isFinite(proposed) || proposed <= 0) return null;
  return {
    currentTrigger: original,
    proposedTrigger: proposed,
    deadTuples: table.dead_tuples,
    currentUsage: (table.dead_tuples / original) * 100,
    proposedUsage: (table.dead_tuples / proposed) * 100,
    alreadyReached: table.dead_tuples >= proposed,
    thresholdPercent: (proposed / original) * 100,
    thresholdDelta: proposed - original,
    thresholdChangePct: ((proposed - original) / original) * 100,
    currentRemaining: Math.max(0, original - table.dead_tuples),
    proposedRemaining: Math.max(0, proposed - table.dead_tuples),
  };
}

export function completeMaintenanceDelta(manual: number | null, automatic: number | null): number | null {
  return manual === null || automatic === null ? null : manual + automatic;
}

export function createAvController(shell: ShellState, state: AvState, invoke: PgXrayInvoke) {
  let generation = 0, historyRequest = 0, snapshotRequest = 0, tableRequest = 0, compareRequest = 0, simulationRequest = 0;
  let disposed = false, connecting = false;
  const current = (auth: number, epoch: number) => !disposed && shell.screen === "workspace" &&
    shell.authGeneration === auth && generation === epoch;
  const validHistory = (s: AvSummary) => s.connection_id === state.connectionId;
  const tableFromSnapshot = (oid: number) => state.snapshot?.tables.data?.find(t => t.relation_oid === oid);
  function clearTable(): void {
    tableRequest++; simulationRequest++;
    state.selectedTableOid = null; state.detail = null; state.advice = null; state.candidates = null;
    state.relationHistory = null; state.detailErrors = []; state.simulation = null; state.pendingCandidateId = null;
    state.detailLoading = false; state.simulating = false;
  }
  function clearView(): void {
    historyRequest++; snapshotRequest++; compareRequest++;
    clearTable();
    state.histories = []; state.snapshot = null; state.snapshotId = null; state.snapshotConnectionId = null;
    state.compareOlderId = null; state.compareNewerId = null; state.comparison = null;
    state.loading = false; state.comparing = false; state.collecting = false;
    state.error = null; state.notice = null; state.applyResult = null;
    // Unknown write outcomes require an explicit new collection before another Apply.
    // Do not silently clear applyBlocked on a connection/snapshot change.
  }
  function reset(): void { generation++; Object.assign(state, createAvState()); }
  function dispose(): void { reset(); disposed = true; }
  function selectConnection(id: number | null): void {
    if (state.applying) return;
    generation++; state.connectionId = state.connections.some(c => c.id === id) ? id : null;
    clearView(); void refreshHistory();
  }
  async function refreshConnections(): Promise<void> {
    if (connecting || state.applying || shell.screen !== "workspace") return;
    connecting = true; const auth = shell.authGeneration, epoch = generation;
    try {
      const list = parseActivityConnections(await invoke("list_connections"));
      if (!current(auth, epoch)) return;
      state.connections = list;
      const next = list.some(c => c.id === state.connectionId) ? state.connectionId : (list[0]?.id ?? null);
      if (next !== state.connectionId) selectConnection(next);
      // On return, local history may have grown through a schedule or another pgXray window.
      else if (next !== null) void refreshHistory();
    } catch (e) { if (current(auth, epoch)) state.error = t('tsFinish.autovacuumPrefix0', { error: String(e) }); }
    finally { connecting = false; }
  }
  async function refreshHistory(): Promise<void> {
    const id = state.connectionId;
    if (id === null || shell.screen !== "workspace" || state.applying) return;
    const auth = shell.authGeneration, epoch = generation, request = ++historyRequest;
    state.loading = true; state.error = null;
    try {
      const histories = await invoke<AvSummary[]>("list_autovacuum_snapshots", { connectionId: id, limit: 100 });
      if (!current(auth, epoch) || state.connectionId !== id || request !== historyRequest) return;
      if (!Array.isArray(histories) || histories.some(s => s.connection_id !== id)) throw Error(t('tsFinish.avHistoryWrong'));
      const selected = histories.find(s => s.id === state.snapshotId) ?? histories[0];
      const snapshot = selected ? await invoke<AvSnapshot>("load_autovacuum_snapshot", { connectionId: id, snapshotId: selected.id }) : null;
      if (!current(auth, epoch) || state.connectionId !== id || request !== historyRequest) return;
      if (snapshot && (!selected || snapshot.overview.database_oid !== selected.database_oid || snapshot.overview.database !== selected.database)) throw Error(t('tsFinish.avSnapshotMismatch'));
      const changed = state.snapshotId !== (selected?.id ?? null);
      state.histories = histories; state.snapshot = snapshot; state.snapshotId = selected?.id ?? null;
      state.snapshotConnectionId = selected ? id : null;
      if (changed) clearTable();
      state.comparison = null;
      const [newer, older] = histories;
      state.compareOlderId = older?.id ?? null; state.compareNewerId = newer?.id ?? null;
    } catch(e) { if (current(auth, epoch) && request === historyRequest) state.error = t('tsFinish.autovacuumPrefix1', { error: String(e) }); }
    finally { if (current(auth, epoch) && request === historyRequest) state.loading = false; }
  }
  async function loadSnapshot(snapshotId: number): Promise<boolean> {
    const id = state.connectionId;
    const summary = state.histories.find(s => s.id === snapshotId && s.connection_id === id);
    if (id === null || !summary || state.applying || state.collecting) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++snapshotRequest;
    ++historyRequest; state.loading = true; state.error = null; clearTable();
    try {
      const snap = await invoke<AvSnapshot>("load_autovacuum_snapshot", { connectionId: id, snapshotId });
      if (!current(auth, epoch) || request !== snapshotRequest || state.connectionId !== id) return false;
      if (snap.overview.database !== summary.database || snap.overview.database_oid !== summary.database_oid) throw Error(t('tsFinish.avWrongDatabase'));
      state.snapshot = snap; state.snapshotId = summary.id; state.snapshotConnectionId = id;
      return true;
    } catch(e) { if (current(auth, epoch) && request === snapshotRequest) state.error = t('tsFinish.autovacuumPrefix2', { error: String(e) }); return false; }
    finally { if (current(auth, epoch) && request === snapshotRequest) state.loading = false; }
  }
  async function collect(): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || state.collecting || state.loading || state.applying || shell.screen !== "workspace") return false;
    const auth = shell.authGeneration, epoch = generation;
    state.collecting = true; state.error = null; state.notice = null;
    try {
      const result = await invoke<{ history: AvSummary; snapshot: AvSnapshot }>("collect_autovacuum_snapshot", { connectionId: id });
      if (!current(auth, epoch) || state.connectionId !== id) return false;
      if (result.history.connection_id !== id || result.history.database_oid !== result.snapshot.overview.database_oid) throw Error(t('tsFinish.avCollectedWrong'));
      state.snapshot = result.snapshot; state.snapshotId = result.history.id; state.snapshotConnectionId = id;
      state.applyBlocked = false; clearTable();
      state.notice = t('tsFinish.avStored', { label: avSnapshotLabel(result.history, true) });
      await refreshHistory();
      return true;
    } catch(e) { if (current(auth, epoch)) state.error = t('tsFinish.autovacuumPrefix3', { error: String(e) }); return false; }
    finally { if (current(auth, epoch)) state.collecting = false; }
  }
  async function compare(): Promise<boolean> {
    const id = state.connectionId, older = state.histories.find(s => s.id === state.compareOlderId), newer = state.histories.find(s => s.id === state.compareNewerId);
    if (id === null || !older || !newer || older.id === newer.id || !sameAvTarget(older, newer) || state.comparing || state.loading || state.applying) {
      state.error = t('tsFinish.avChoosePair'); return false;
    }
    const auth = shell.authGeneration, epoch = generation, request = ++compareRequest;
    const firstId = older.id, secondId = newer.id;
    state.comparison = null; state.comparing = true; state.error = null;
    try {
      const comparison = await invoke<AvComparison>("compare_autovacuum_snapshots", { firstId, secondId });
      if (!current(auth, epoch) || request !== compareRequest || state.connectionId !== id ||
        firstId !== state.compareOlderId || secondId !== state.compareNewerId) return false;
      if (!sameAvTarget(comparison.older, comparison.newer) || comparison.older.connection_id !== id ||
        ![firstId, secondId].includes(comparison.older.id) || ![firstId, secondId].includes(comparison.newer.id)) throw Error(t('tsFinish.avComparisonWrong'));
      state.comparison = comparison; return true;
    } catch(e) { if (current(auth, epoch) && request === compareRequest) state.error = t('tsFinish.autovacuumPrefix4', { error: String(e) }); return false; }
    finally { if (current(auth, epoch) && request === compareRequest) state.comparing = false; }
  }
  async function selectTable(oid: number): Promise<boolean> {
    const id = state.connectionId, snapshot = state.snapshot, snapshotId = state.snapshotId;
    if (id === null || snapshotId === null || !snapshot || state.snapshotConnectionId !== id || state.applying || state.loading || !tableFromSnapshot(oid)) return false;
    const auth = shell.authGeneration, epoch = generation;
    clearTable(); const request = tableRequest;
    state.selectedTableOid = oid; state.detailLoading = true; state.error = null;
    const selectedSummary = state.histories.find(s => s.id === snapshotId);
    const saved = state.connections.find(c => c.id === id);
    const canReadLive = avSummaryMatchesConnection(selectedSummary, saved);
    const calls: Promise<unknown>[] = [
      canReadLive ? invoke<AvTableDetail>("autovacuum_table_detail", { connectionId: id, relationOid: oid }) : Promise.reject(Error(t('tsFinish.avLiveUnavailable'))),
      invoke<AvRelationHistory>("autovacuum_relation_history", { snapshotId, relationOid: oid, limit: 100 }),
      invoke<AvAdvice>("autovacuum_maintenance_advice", { snapshotId, relationOid: oid, limit: 100 }),
      invoke<AvCandidates>("autovacuum_policy_candidates", { snapshotId, relationOid: oid, limit: 100 }),
    ];
    const result = await Promise.allSettled(calls);
    if (!current(auth, epoch) || state.connectionId !== id || state.snapshotId !== snapshotId ||
      state.snapshot !== snapshot || state.selectedTableOid !== oid || tableRequest !== request) return false;
    const problems: string[] = [];
    const [detail, history, advice, candidates] = result;
    if (detail.status === "fulfilled") {
      const data = detail.value as AvTableDetail;
      const original = tableFromSnapshot(oid);
      if (data.table.relation_oid === oid && data.table.schema_name === original?.schema_name && data.table.relation_name === original.relation_name) state.detail = data;
      else problems.push(t('tsFinish.avTableChanged'));
    } else problems.push(t('tsFinish.autovacuumPrefix5', { error: String(detail.reason) }));
    if (history.status === "fulfilled") {
      const data = history.value as AvRelationHistory;
      if (data.relation_oid === oid && data.anchor_snapshot_id === snapshotId) state.relationHistory = data;
      else problems.push(t('tsFinish.avWrongTableHistory'));
    } else problems.push(t('tsFinish.autovacuumPrefix6', { error: String(history.reason) }));
    if (advice.status === "fulfilled") {
      const data = advice.value as AvAdvice;
      if (data.relation_oid === oid) state.advice = data; else problems.push(t('tsFinish.avAdviceWrong'));
    } else problems.push(t('tsFinish.autovacuumPrefix7', { error: String(advice.reason) }));
    if (candidates.status === "fulfilled") {
      const data = candidates.value as AvCandidates;
      if (data.relation_oid === oid) state.candidates = data; else problems.push(t('tsFinish.avCandidatesWrong'));
    } else problems.push(t('tsFinish.autovacuumPrefix8', { error: String(candidates.reason) }));
    state.detailErrors = problems; state.detailLoading = false;
    return state.detail !== null || state.advice !== null || state.relationHistory !== null;
  }
  async function simulate(targetDeadTuples: number): Promise<boolean> {
    const id = state.connectionId, oid = state.selectedTableOid, snapshotId = state.snapshotId;
    if (id === null || oid === null || snapshotId === null || !state.detail || state.loading || state.simulating || state.applying ||
      !Number.isSafeInteger(targetDeadTuples) || targetDeadTuples <= 0) { state.error = t('tsFinish.avPositiveTarget'); return false; }
    const auth = shell.authGeneration, epoch = generation, request = ++simulationRequest;
    state.simulating = true; state.error = null; state.simulation = null;
    try {
      const sim = await invoke<AvSimulation>("simulate_autovacuum_policy", { connectionId: id, relationOid: oid, targetDeadTuples });
      if (!current(auth, epoch) || request !== simulationRequest || state.selectedTableOid !== oid || state.snapshotId !== snapshotId) return false;
      if (sim.relation_oid !== oid) throw Error(t('tsFinish.avSimulationWrong'));
      state.simulation = sim; return true;
    } catch(e) { if (current(auth, epoch) && request === simulationRequest) state.error = t('tsFinish.autovacuumPrefix9', { error: String(e) }); return false; }
    finally { if (current(auth, epoch) && request === simulationRequest) state.simulating = false; }
  }
  function prepareApply(candidateId: string): boolean {
    const id = state.connectionId, oid = state.selectedTableOid;
    const summary = state.histories.find(s => s.id === state.snapshotId);
    const connection = state.connections.find(c => c.id === id);
    if (state.applyBlocked || state.loading || state.collecting || state.applying || !state.detail || !state.advice || !state.snapshot?.overview.track_counts ||
      state.snapshotConnectionId !== id || !avSummaryMatchesConnection(summary, connection) ||
      !state.candidates?.candidates.some(c => c.id === candidateId && state.candidates?.relation_oid === oid)) {
      state.error = t('tsFinish.avApplyUnavailable');
      return false;
    }
    state.pendingCandidateId = candidateId; state.error = null;
    return true;
  }
  function cancelApply(): void { if (!state.applying) state.pendingCandidateId = null; }
  async function apply(): Promise<boolean> {
    const id = state.connectionId, oid = state.selectedTableOid, snapshotId = state.snapshotId;
    const candidateId = state.pendingCandidateId;
    const candidate = state.candidates?.candidates.find(c => c.id === candidateId);
    if (id === null || oid === null || snapshotId === null || !candidate || !candidateId || state.applying || state.applyBlocked ||
      !state.snapshot?.overview.track_counts || !state.detail || !state.advice) return false;
    const auth = shell.authGeneration, epoch = generation;
    state.applying = true; state.error = null; state.pendingCandidateId = null; state.applyResult = null;
    try {
      // Single attempt. The Rust backend revalidates connection, snapshot, reloptions and lock.
      const result = await invoke<AvApplyResult>("apply_autovacuum_policy_candidate", {
        connectionId: id, snapshotId, relationOid: oid, candidateId,
      });
      if (!current(auth, epoch) || state.connectionId !== id) return false;
      if (result.candidate.id !== candidateId || result.verification.after.scale_factor !== candidate.candidate_scale_factor)
        throw Error(t('tsFinish.avApplyMismatch'));
      state.applyResult = result;
      state.notice = t('tsFinish.avApplied', { parameter: candidate.parameter, warning: result.warning ?? t('tsFinish.avRollbackReminder') });
      clearTable(); // invalidate old policy candidates; retain apply verification
      await refreshHistoryAfterApply(id);
      return true;
    } catch(e) {
      if (current(auth, epoch)) {
        // A failed/aborted write may have committed. No retries, including for unknown error text.
        state.applyBlocked = true;
        state.error = t('tsFinish.avApplyVerifyFailed', { error: String(e) });
      }
      return false;
    } finally { if (current(auth, epoch)) state.applying = false; }
  }
  async function refreshHistoryAfterApply(id: number): Promise<void> {
    // Apply holds its own busy flag; do not bypass safety checks or assume a snapshot was saved.
    const auth = shell.authGeneration, epoch = generation;
    try {
      const histories = await invoke<AvSummary[]>("list_autovacuum_snapshots", { connectionId: id, limit: 100 });
      if (!current(auth, epoch) || state.connectionId !== id) return;
      if (!Array.isArray(histories) || histories.some(s => s.connection_id !== id)) throw Error(t('tsFinish.avHistoryWrong'));
      state.histories = histories;
      const newSummary = state.applyResult?.history;
      if (newSummary && newSummary.connection_id === id && state.applyResult?.snapshot) {
        state.snapshotId = newSummary.id; state.snapshotConnectionId = id; state.snapshot = state.applyResult.snapshot;
      } else {
        state.snapshot = null; state.snapshotId = null; state.snapshotConnectionId = null;
      }
      state.comparison = null;
    } catch(e) { if (current(auth, epoch)) state.error = t('tsFinish.avHistoryAfterApplyFailed', { error: String(e) }); }
  }
  async function loadReference(): Promise<void> {
    const auth = shell.authGeneration, epoch = generation;
    try { const ref = await invoke<AvReference>("autovacuum_reference"); if (current(auth, epoch)) state.reference = ref; }
    catch(e) { if (current(auth, epoch)) state.error = t('tsFinish.autovacuumPrefix10', { error: String(e) }); }
  }
  return { reset, dispose, selectConnection, refreshConnections, refreshHistory, loadSnapshot, collect,
    compare, selectTable, simulate, prepareApply, cancelApply, apply, loadReference };
}
