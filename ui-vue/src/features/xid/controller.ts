import { locale } from "../i18n/locale-service.js";
import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { parseActivityConnections, type ActivityConnection } from "../activity/controller.js";

/** Wire contracts: crates/core/src/xid/model.rs, history/xid.rs and Tauri commands.
 * Rust alone calculates risk, candidate values and guarded ALTER statements. */
export interface XidSection<T> { data: T | null; error: string | null; observed_at_ms?: number | null }
export interface XidOverview {
  measured_at: string; database: string; database_oid: number; username: string;
  server_version: string; server_version_num: number; current_xid: string;
  autovacuum: boolean; autovacuum_freeze_max_age: number;
  vacuum_freeze_table_age: number; vacuum_freeze_min_age: number;
  autovacuum_max_workers: number; autovacuum_vacuum_cost_limit: number;
  autovacuum_vacuum_cost_delay: string; database_xid_age: number;
  stats_reset: string | null; database_mxid_age?: number | null;
  autovacuum_multixact_freeze_max_age?: number | null;
}
export interface XidDatabase { database_oid: number; database: string; xid_age: number;
  pct_of_global_freeze_max: number | null; mxid_age?: number | null;
  pct_of_global_multixact_freeze_max?: number | null }
export interface XidRelation {
  relation_oid: number; schema_name: string; relation_name: string; relation_type: string;
  xid_age: number; mxid_age?: number | null; effective_multixact_freeze_max_age?: number | null;
  pct_of_multixact_freeze_max?: number | null; effective_freeze_max_age: number;
  freeze_max_source: string; pct_of_freeze_max: number | null; total_size_bytes: number;
  estimated_rows: number | null; heap_pages: number | null; all_visible_pages: number | null;
  all_frozen_pages?: number | null; all_visible_pct: number | null;
  effective_freeze_min_age: number | null; freeze_min_source: string | null;
  effective_freeze_table_age: number | null; freeze_table_source: string | null;
  last_vacuum: string | null; last_autovacuum: string | null;
  last_analyze: string | null; last_autoanalyze: string | null;
  vacuum_count: number; autovacuum_count: number; analyze_count: number | null;
  autoanalyze_count: number | null; modifications_since_analyze: number | null;
  reloptions: string[] | null; parent_oid: number | null; parent_schema: string | null;
  parent_relation: string | null; parent_relation_type: string | null; parent_xid_age: number | null;
}
export interface XidTrigger {
  relation_oid: number; schema_name: string; relation_name: string; relation_type: string;
  xid_age: number; autovacuum_enabled: boolean; estimated_rows: number | null;
  thresholds_reliable?: boolean | null; estimated_live_rows: number; estimated_dead_rows: number;
  inserts_since_vacuum: number; tuples_inserted: number; tuples_updated: number;
  tuples_deleted: number; hot_updates: number; effective_vacuum_threshold: number;
  effective_vacuum_scale_factor: number; effective_insert_threshold: number;
  effective_insert_scale_factor: number; estimated_dead_tuple_trigger: number | null;
  estimated_insert_trigger: number | null; dead_trigger_reached: boolean;
  insert_trigger_reached: boolean; last_vacuum: string | null;
  last_autovacuum: string | null; stats_reset: string | null;
}
export interface XidVacuum { pid: number; relation_oid: number; database: string;
  schema_name: string; relation_name: string; phase: string; xid_age: number;
  heap_blks_scanned: number; heap_blks_total: number; scan_pct: number | null;
  backend_type: string | null; is_wraparound_vacuum: boolean;
  wraparound_detection?: string | null; wait_event_type: string | null; wait_event: string | null }
export interface XidBackend { pid: number; database: string | null; username: string | null;
  application_name: string | null; state: string | null; xmin_age: number;
  transaction_age_seconds: number | null; wait_event_type: string | null; wait_event: string | null }
export interface XidSlot { slot_name: string; slot_type: string; database: string | null;
  active: boolean; xmin_age: number | null; catalog_xmin_age: number | null }
export interface XidPrepared { gid: string; xid: string; database: string; owner: string;
  prepared_at: string; age_seconds: number }
export interface XidPreparedSummary { count: number; oldest_age_seconds: number | null;
  examples?: XidPrepared[] }
export interface XidInterpretationItem { id: string; title: string; observed: string[];
  meaning: string[]; next_steps: string[] }
export interface XidSnapshot {
  overview: XidOverview; databases: XidSection<XidDatabase[]>;
  relations: XidSection<XidRelation[]>; autovacuum_triggers: XidSection<XidTrigger[]>;
  vacuums: XidSection<XidVacuum[]>; backends: XidSection<XidBackend[]>;
  slots: XidSection<XidSlot[]>; prepared: XidSection<XidPreparedSummary>;
  interpretation: { items: XidInterpretationItem[]; limitations: string[] };
}
export interface XidSummary {
  id: number; schema_version: number; app_version: string; connection_id: number;
  connection_name: string; host: string; port: number; database: string; database_oid: number;
  username: string; sslmode: string; server_version: string; server_version_num: number;
  collection_started_at_ms: number; collection_completed_at_ms: number;
  measured_at: string; current_xid: string; database_xid_age: number;
  oldest_relation: { relation_oid: number; relation_name: string;
    xid_age: number; effective_freeze_max_age: number } | null;
  unavailable_sections: number; stored_at_ms: number;
}
export interface XidHistoryPoint { snapshot_id: number; schema_version: number;
  collection_started_at_ms: number; measured_at: string; current_xid: string;
  stats_reset: string | null; relation: XidRelation; trigger: XidTrigger | null;
  active_vacuum: XidVacuum | null }
export interface XidCandidate { id: string; title: string; parameter: string;
  current_value: number; candidate_value: number; ddl_target: string; sql: string;
  why: string[]; expected_effect: string[]; tradeoffs: string[] }
export interface XidPolicyAdvice { relation_oid: number; relation_name: string;
  status: string; title: string; summary: string; history_points: number;
  history_span_seconds: number | null; xid_rate_per_hour: number | null;
  relation_age_delta: number | null; estimated_horizon_advance: number | null;
  vacuum_cycles_delta: number | null; stats_period_continuous: boolean;
  current_policy: { effective_freeze_max_age: number; freeze_max_source: string;
    effective_freeze_table_age: number | null; freeze_table_source: string | null;
    effective_freeze_min_age: number | null; freeze_min_source: string | null };
  primary_candidate_id: string | null; candidates: XidCandidate[];
  observations: string[]; limitations: string[] }
export interface XidComparison {
  older: XidSummary; newer: XidSummary; elapsed_seconds: number;
  xid_rate: { xid_delta: number; elapsed_seconds: number;
    xid_per_second: number; xid_per_minute: number; xid_per_hour: number } | null;
  database_xid_age_delta: number;
  oldest_relation: { older: XidSummary['oldest_relation']; newer: XidSummary['oldest_relation'];
    xid_age_delta: number | null; older_configured_headroom: number | null;
    newer_configured_headroom: number | null };
  relation_changes: { relation_oid: number; older_name: string | null; newer_name: string | null;
    older_xid_age: number | null; newer_xid_age: number | null; xid_age_delta: number | null;
    older_pct_of_freeze_max: number | null; newer_pct_of_freeze_max: number | null;
    sample_state: string }[];
  blockers: { older_backend_count: number | null; newer_backend_count: number | null;
    backend_new_in_sample: string[]; backend_not_in_new_sample: string[];
    older_slot_count: number | null; newer_slot_count: number | null;
    slots_new_in_sample: string[]; slots_not_in_new_sample: string[];
    older_prepared_count: number | null; newer_prepared_count: number | null };
  vacuums: { older_active_count: number | null; newer_active_count: number | null;
    older_wraparound_count: number | null; newer_wraparound_count: number | null;
    active_new_in_sample: string[]; active_not_in_new_sample: string[] };
  limitations: string[];
}
export interface XidReference {
  methodology: { id: string; title: string; purpose: string; how_to_read: string[] }[];
  queries: { id: string; title: string; purpose: string; output: string[];
    how_to_read: string[]; sql: string }[];
  sources: { id: string; title: string; purpose: string; path: string }[];
}
export interface XidPolicyApplyResult { candidate: XidCandidate;
  verification: { candidate_id: string; parameter: string; candidate_value: number };
  history: XidSummary | null; snapshot: XidSnapshot | null; warning: string | null }
export interface XidState {
  connections: ActivityConnection[]; connectionId: number | null;
  history: XidSummary[]; summary: XidSummary | null; snapshot: XidSnapshot | null;
  selectedRelationOid: number | null; relationHistory: XidHistoryPoint[];
  policy: XidPolicyAdvice | null; comparison: XidComparison | null;
  firstId: number | null; secondId: number | null; reference: XidReference | null;
  loading: boolean; loadingConnections: boolean; loadingSnapshot: boolean; historical: boolean; collecting: boolean; loadingRelation: boolean; comparing: boolean;
  applying: boolean; referenceLoading: boolean; error: string | null;
  relationError: string | null; notice: string | null;
}
export function createXidState(): XidState {
  return { connections: [], connectionId: null, history: [], summary: null, snapshot: null,
    selectedRelationOid: null, relationHistory: [], policy: null, comparison: null,
    firstId: null, secondId: null, reference: null, loading: false, loadingConnections: false, loadingSnapshot: false, historical: false, collecting: false,
    loadingRelation: false, comparing: false, applying: false, referenceLoading: false,
    error: null, relationError: null, notice: null };
}
export type XidSortDirection = "asc" | "desc";
export type XidSortValue = number | string | null | undefined;

/** Sort the full dataset *before* pagination. Missing values always go last.
 * Never mutate the Collector/history arrays; equal values preserve source order. */
export function xidSortRows<T>(rows: readonly T[], valueOf: (row: T) => XidSortValue,
  direction: XidSortDirection): T[] {
  const collator = new Intl.Collator("ru-RU", { numeric: true, sensitivity: "base" });
  const sign = direction === "asc" ? 1 : -1;
  return rows.map((row, index) => ({ row, index, value: valueOf(row) })).sort((a, b) => {
    const missingA = a.value == null || (typeof a.value === "number" && !Number.isFinite(a.value));
    const missingB = b.value == null || (typeof b.value === "number" && !Number.isFinite(b.value));
    if (missingA !== missingB) return missingA ? 1 : -1;
    if (missingA && missingB) return a.index - b.index;
    const result = typeof a.value === "number" && typeof b.value === "number"
      ? a.value - b.value : collator.compare(String(a.value), String(b.value));
    return sign * Math.sign(result) || a.index - b.index;
  }).map(item => item.row);
}

export function xidIsSystemSchema(schema: string): boolean {
  return schema === "information_schema" || schema.startsWith("pg_");
}

export const XID_PAGE_SIZES = [20, 50, 100] as const;
export function xidPage<T>(items: readonly T[], page: number, pageSize: number): {rows:T[];page:number;pages:number;total:number} {
  const size = XID_PAGE_SIZES.includes(pageSize as (typeof XID_PAGE_SIZES)[number]) ? pageSize : 20;
  const pages = Math.max(1, Math.ceil(items.length / size));
  const at = Math.max(1, Math.min(pages, Math.trunc(page) || 1));
  return { rows: items.slice((at-1)*size, at*size), page: at, pages, total: items.length };
}
export function xidSectionStatus<T>(section: XidSection<T[]> | null | undefined): "unavailable"|"empty"|"available" {
  if (!section || section.error !== null || section.data === null || !Array.isArray(section.data)) return "unavailable";
  return section.data.length ? "available" : "empty";
}
export function sameXidTarget(a: XidSummary, b: XidSummary): boolean {
  return a.connection_id === b.connection_id && a.host === b.host && a.port === b.port &&
    a.database === b.database && a.database_oid === b.database_oid &&
    a.username === b.username && a.sslmode === b.sslmode;
}
export function xidSummaryMatchesConnection(s: XidSummary, c: ActivityConnection | undefined): boolean {
  return !!c && s.connection_id === c.id && s.host === c.host && s.port === c.port &&
    s.database === c.database && s.username === c.username;
}
export function xidDocsUrl(version: number, path: string): string | null {
  if (!/^[a-z0-9.-]+(?:#[A-Za-z0-9_-]+)?$/.test(path)) return null;
  const major = Math.floor(version / 10000);
  return `https://www.postgresql.org/docs/${major >= 10 && major <= 99 ? major : "current"}/${path}`;
}

export function createXidController(shell: ShellState, state: XidState, invoke: PgXrayInvoke) {
  let generation = 0, historyRequest = 0, relationRequest = 0, compareRequest = 0;
  let connecting = false, disposed = false;
  const current = (auth: number, epoch: number) => !disposed && shell.screen === "workspace" &&
    shell.authGeneration === auth && generation === epoch;
  function clearRelation(): void {
    relationRequest++; state.selectedRelationOid = null; state.relationHistory = [];
    state.policy = null; state.loadingRelation = false; state.relationError = null;
  }
  function clearContent(): void {
    historyRequest++; compareRequest++; clearRelation();
    state.history = []; state.summary = null; state.snapshot = null; state.historical = false;
    state.firstId = null; state.secondId = null; state.comparison = null;
    state.error = null; state.notice = null; state.loading = false; state.comparing = false;
  }
  function reset(): void { generation++; historyRequest++; relationRequest++; compareRequest++;
    Object.assign(state, createXidState()); }
  function dispose(): void { reset(); disposed = true; }
  function selectConnection(id: number | null, loadHistory = true): void {
    if (state.applying || state.collecting || state.loadingSnapshot) return;
    generation++;
    state.connectionId = state.connections.some(c => c.id === id) ? id : null;
    clearContent(); if (loadHistory) void refreshHistory(true);
  }
  async function refreshConnections(openLatest = false, preferredConnectionId: number | null = null): Promise<void> {
    if (connecting || state.collecting || state.applying || state.loadingSnapshot || shell.screen !== "workspace") return;
    connecting = true; state.loadingConnections = true; state.error = null; state.notice = t('tsFinish.xidConnectionsLoading');
    const auth = shell.authGeneration, epoch = generation;
    try {
      const connections = parseActivityConnections(await invoke("list_connections"));
      if (!current(auth, epoch)) return;
      state.connections = connections;
      const preferred = connections.find(c => c.id === preferredConnectionId)?.id;
      const id = preferred ?? (connections.some(c => c.id === state.connectionId)
        ? state.connectionId : connections[0]?.id ?? null);
      if (id !== state.connectionId) {
        selectConnection(id, false);
        if (id !== null) {
          if (openLatest) await refreshHistory(true);
          else void refreshHistory();
        }
      } else if (id !== null) await refreshHistory(openLatest);
      if (current(auth, generation) && !state.error && (!openLatest || id === null))
        state.notice = t('tsFinish.xidConnectionsUpdated', { count: String(connections.length), detail: id === null ? t('tsFinish.xidSelectConnection') : t('tsFinish.xidHistoryLoaded') });
    } catch (error) { if (current(auth, epoch)) { state.error = t('tsFinish.xidPrefix0', { error: String(error) }); state.notice = null; } }
    finally { connecting = false; if (!disposed) state.loadingConnections = false; }
  }
  async function refreshHistory(openLatest = false): Promise<void> {
    const id = state.connectionId;
    if (id === null || state.applying || shell.screen !== "workspace") return;
    const auth = shell.authGeneration, epoch = generation, request = ++historyRequest;
    state.loading = true; state.error = null; state.notice = t('tsFinish.xidHistoryLoading');
    try {
      const history = await invoke<XidSummary[]>("list_xid_snapshots", { connectionId: id, limit: 500 });
      if (!current(auth, epoch) || request !== historyRequest || state.connectionId !== id) return;
      if (!Array.isArray(history) || history.some(s => s.connection_id !== id))
        throw Error(t('tsFinish.xidWrongHistory'));
      state.history = history;
      if (!state.firstId || !history.some(s => s.id === state.firstId)) state.firstId = history[1]?.id ?? null;
      if (!state.secondId || !history.some(s => s.id === state.secondId)) state.secondId = history[0]?.id ?? null;
      if (state.firstId === state.secondId) state.firstId = history.find(s => s.id !== state.secondId)?.id ?? null;
      state.comparison = null;
      state.notice = t('tsFinish.xidHistoryUpdated', { count: String(history.length) });
      if (openLatest) {
        // The store returns newest first. Never show an old profile if a connection ID was reused.
        const connection = state.connections.find(c => c.id === id);
        const latest = history.find(s => xidSummaryMatchesConnection(s, connection));
        // A failed archive load must never leave the previous snapshot on screen.
        clearRelation(); state.snapshot = null; state.summary = null; state.historical = false;
        if (latest) await openSavedSnapshot(latest.id);
        else {
          state.notice = history.length
            ? t('tsFinish.xidNoStoredForProfile')
            : t('tsFinish.xidNoStored');
        }
      }
    } catch (error) { if (current(auth, epoch) && request === historyRequest) { state.error = t('tsFinish.xidPrefix1', { error: String(error) }); state.notice = null; } }
    finally { if (current(auth, epoch) && request === historyRequest) state.loading = false; }
  }
  function acceptSnapshot(history: XidSummary, snapshot: XidSnapshot, id: number): void {
    if (history.connection_id !== id || history.database !== snapshot.overview.database ||
        history.database_oid !== snapshot.overview.database_oid ||
        history.username !== snapshot.overview.username)
      throw Error(t('tsFinish.xidWrongDatabase'));
    clearRelation(); compareRequest++; state.comparison = null;
    state.snapshot = snapshot; state.summary = history; state.historical = false;
  }
  async function openSavedSnapshot(snapshotId: number): Promise<boolean> {
    const id = state.connectionId;
    const known = state.history.find(item => item.id === snapshotId);
    if (id === null || !known || state.collecting || state.applying || state.loadingSnapshot || shell.screen !== "workspace") return false;
    const auth = shell.authGeneration, epoch = generation;
    state.loadingSnapshot = true; state.error = null; state.notice = t('tsFinish.xidOpenArchive', { id: String(snapshotId) });
    try {
      const result = await invoke<{history:XidSummary;snapshot:XidSnapshot}>("load_xid_snapshot", {connectionId:id,snapshotId});
      if (!current(auth,epoch) || state.connectionId !== id) return false;
      if (result.history.id !== snapshotId || result.history.connection_id !== id ||
          !sameXidTarget(known, result.history)) throw Error(t('tsFinish.xidWrongArchive'));
      acceptSnapshot(result.history, result.snapshot, id);
      state.historical = true;
      state.notice = t('tsFinish.xidArchived', { id: String(snapshotId), time: new Date(result.history.collection_started_at_ms).toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US') });
      return true;
    } catch (error) {
      if (current(auth,epoch)) { state.error = t('tsFinish.xidPrefix2', { error: String(error) }); state.notice = null; }
      return false;
    } finally { if (current(auth,epoch)) state.loadingSnapshot = false; }
  }
  async function collect(): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || state.collecting || state.applying || state.loading || state.loadingSnapshot || shell.screen !== "workspace") return false;
    const auth = shell.authGeneration, epoch = generation;
    state.collecting = true; state.error = null; state.notice = t('tsFinish.xidCollecting');
    try {
      const result = await invoke<{history:XidSummary;snapshot:XidSnapshot}>("collect_xid_snapshot", {connectionId:id});
      if (!current(auth, epoch) || state.connectionId !== id) return false;
      acceptSnapshot(result.history, result.snapshot, id);
      await refreshHistory();
      state.notice = t('tsFinish.xidSaved', { id: String(result.history.id), time: new Date(result.history.collection_started_at_ms).toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US') });
      return true;
    } catch (error) { if (current(auth, epoch)) { state.error = t('tsFinish.xidPrefix3', { error: String(error) }); state.notice = null; } return false; }
    finally { if (current(auth, epoch)) state.collecting = false; }
  }
  async function selectRelation(oid: number | null): Promise<void> {
    if (state.applying) return;
    const snap = state.snapshot, summary = state.summary;
    clearRelation();
    if (!snap || !summary || oid === null || !snap.relations.data?.some(r => r.relation_oid === oid)) return;
    state.selectedRelationOid = oid;
    const auth = shell.authGeneration, epoch = generation, request = relationRequest;
    const valid = () => current(auth, epoch) && request === relationRequest &&
      state.summary?.id === summary.id && state.selectedRelationOid === oid;
    state.loadingRelation = true;
    const jobs = await Promise.allSettled([
      invoke<XidHistoryPoint[]>("xid_relation_history", {snapshotId:summary.id,relationOid:oid,limit:100}),
      invoke<XidPolicyAdvice>("xid_relation_policy", {snapshotId:summary.id,relationOid:oid,limit:100}),
    ]);
    if (!valid()) return;
    const [history, advice] = jobs;
    if (history.status === "fulfilled" && Array.isArray(history.value)) state.relationHistory = history.value;
    if (advice.status === "fulfilled" && advice.value.relation_oid === oid) state.policy = advice.value;
    const failures = [history.status === "rejected" ? t('tsFinish.xidPrefix4', { error: String(history.reason) }) : null,
      advice.status === "rejected" ? `Policy: ${String(advice.reason)}` : null].filter(Boolean);
    state.relationError = failures.join("; ") || null;
    state.loadingRelation = false;
  }
  async function compare(): Promise<boolean> {
    const id = state.connectionId, a = state.history.find(s => s.id === state.firstId),
      b = state.history.find(s => s.id === state.secondId);
    if (id === null || !a || !b || a.id === b.id || state.comparing || !sameXidTarget(a,b)) {
      state.error = t('tsFinish.xidChoosePair');
      return false;
    }
    const auth = shell.authGeneration, epoch = generation, request = ++compareRequest;
    state.comparing = true; state.error = null; state.comparison = null;
    try {
      const result = await invoke<XidComparison>("compare_xid_snapshots", {firstId:a.id,secondId:b.id});
      if (!current(auth,epoch) || request !== compareRequest || state.firstId !== a.id || state.secondId !== b.id) return false;
      if (![a.id,b.id].includes(result.older.id) || ![a.id,b.id].includes(result.newer.id) ||
          !sameXidTarget(result.older,result.newer)) throw Error(t('tsFinish.xidWrongComparison'));
      state.comparison = result;
      return true;
    } catch (error) { if (current(auth,epoch) && request === compareRequest) state.error = t('tsFinish.xidPrefix5', { error: String(error) }); return false; }
    finally { if (current(auth,epoch) && request === compareRequest) state.comparing = false; }
  }
  function selectCompare(first: number | null, second: number | null): void {
    compareRequest++; state.firstId = first; state.secondId = second; state.comparison = null;
  }
  async function loadReference(): Promise<void> {
    if (state.reference || state.referenceLoading || shell.screen !== "workspace") return;
    const auth = shell.authGeneration, epoch = generation;
    state.referenceLoading = true;
    try {
      const reference = await invoke<XidReference>("xid_reference");
      if (current(auth,epoch)) state.reference = reference;
    } catch (error) { if (current(auth,epoch)) state.error = t('tsFinish.xidPrefix6', { error: String(error) }); }
    finally { if (current(auth,epoch)) state.referenceLoading = false; }
  }
  /** Called ONLY after a deliberate user confirmation in the View. The backend
   * recomputes candidate and performs guarded ALTER and verification. */
  async function apply(candidateId: string): Promise<boolean> {
    const id = state.connectionId, summary = state.summary, oid = state.selectedRelationOid;
    const candidate = state.policy?.candidates.find(c => c.id === candidateId);
    const connection = state.connections.find(c => c.id === id);
    if (id === null || !summary || oid === null || !candidate || !connection ||
        state.historical || !xidSummaryMatchesConnection(summary, connection) || state.applying || state.collecting ||
        state.loadingRelation || state.policy?.relation_oid !== oid || shell.screen !== "workspace") {
      state.error = t('tsFinish.xidApplyUnavailable');
      return false;
    }
    const auth = shell.authGeneration, epoch = generation;
    state.applying = true; state.error = null; state.notice = null;
    try {
      const result = await invoke<XidPolicyApplyResult>("apply_xid_policy_candidate", {
        connectionId:id, snapshotId:summary.id, relationOid:oid, candidateId,
      });
      if (!current(auth,epoch)) return false;
      if (result.candidate.id !== candidateId || result.verification.candidate_id !== candidateId)
        throw Error(t('tsFinish.xidApplyUnexpected'));
      // A successful ALTER can return without a saved post-change snapshot.
      clearRelation();
      if (result.history && result.snapshot) {
        acceptSnapshot(result.history, result.snapshot, id);
        state.notice = t('tsFinish.xidPolicySaved', { id: String(result.history.id), warning: result.warning ?? '' });
        await refreshHistoryAfterApply(id, auth, epoch);
      } else {
        state.snapshot = null; state.summary = null;
        state.notice = t('tsFinish.xidPolicyNoSnapshot', { warning: result.warning ?? t('tsFinish.xidRecollectAfterApply') });
      }
      return true;
    } catch (error) { if (current(auth,epoch)) state.error = t('tsFinish.xidPrefix7', { error: String(error) }); return false; }
    finally { if (current(auth,epoch)) state.applying = false; }
  }
  async function refreshHistoryAfterApply(id: number, auth: number, epoch: number): Promise<void> {
    const request = ++historyRequest;
    try {
      const history = await invoke<XidSummary[]>("list_xid_snapshots", {connectionId:id,limit:500});
      if (current(auth,epoch) && request === historyRequest && state.connectionId === id) {
        if (!Array.isArray(history) || history.some(s=>s.connection_id !== id)) throw Error(t('tsFinish.xidForeignHistory'));
        state.history = history; state.firstId = history[1]?.id ?? null; state.secondId = history[0]?.id ?? null;
      }
    } catch (error) { if (current(auth,epoch)) state.error = t('tsFinish.xidPrefix8', { error: String(error) }); }
  }
  return { reset, dispose, selectConnection, refreshConnections, refreshHistory, collect,
    openSavedSnapshot, selectRelation, selectCompare, compare, loadReference, apply };
}
