import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { parseActivityConnections, type ActivityConnection } from "../activity/controller.js";

/** JSON contract from crates/core/src/index_health.rs and history/index_health.rs. */
export interface IndexRow {
  index_oid: number; relation_oid: number; schema_name: string; relation_name: string;
  index_name: string; index_size_bytes: number; idx_scan: number; idx_tup_read: number;
  idx_tup_fetch: number; is_primary: boolean; is_unique: boolean; is_valid: boolean;
  is_ready: boolean; key_columns: string[]; predicate: string | null;
  tuples_inserted: number; tuples_updated: number; tuples_deleted: number;
  hot_updates: number; last_analyze: string | null; last_autoanalyze: string | null;
  classifications: string[];
}
export interface IndexSnapshot {
  overview: { measured_at: string; database: string; database_oid: number; server_version: string; server_version_num: number; stats_reset: string | null; stats_age_seconds: number | null; track_counts: boolean };
  collection: { total_index_count: number; sampled_index_count: number; truncated: boolean; indexes: IndexRow[] };
  interpretation: { limitations: string[] };
}
export interface IndexSummary {
  id: number; connection_id: number; connection_name: string; host: string; port: number;
  database: string; database_oid: number; username: string; sslmode: string;
  collection_completed_at_ms: number; server_version_num: number; sampled_index_count: number;
  total_index_count: number; suspiciously_unused_count: number; possible_duplicate_count: number;
  high_write_overhead_count: number; sample_truncated: boolean;
}
export interface IndexDelta {
  index_oid: number; relation_oid: number; relation_name: string; index_name: string;
  sample_state: string; idx_scan_delta: number | null; idx_tup_read_delta: number | null;
  idx_tup_fetch_delta: number | null; relation_dml_delta: number | null;
  older_size_bytes: number | null; newer_size_bytes: number | null;
  size_delta_bytes: number | null; newer_classifications: string[];
}
export interface IndexComparison {
  older: IndexSummary; newer: IndexSummary; elapsed_seconds: number;
  stats_period_continuous: boolean; index_changes: IndexDelta[]; limitations: string[];
}
export interface IndexReference { methodology: string[]; sources: { title: string; purpose: string; path: string }[] }
export interface TableDetail { table: { relation_oid: number; schema_name: string; relation_name: string; live_tuples: number; dead_tuples: number; total_size_bytes: number; last_autovacuum: string | null }; interpretation: { limitations?: string[] } }
export interface IndexHealthState {
  connections: ActivityConnection[]; connectionId: number | null; histories: IndexSummary[];
  snapshot: IndexSnapshot | null; snapshotId: number | null; snapshotConnectionId: number | null;
  comparison: IndexComparison | null; compareOlderId: number | null; compareNewerId: number | null;
  reference: IndexReference | null; tableDetail: TableDetail | null; detailIndex: IndexRow | null;
  loading: boolean; collecting: boolean; comparing: boolean; detailLoading: boolean;
  error: string | null; notice: string | null;
}
export function createIndexHealthState(): IndexHealthState {
  return { connections: [], connectionId: null, histories: [], snapshot: null, snapshotId: null,
    snapshotConnectionId: null, comparison: null, compareOlderId: null, compareNewerId: null,
    reference: null, tableDetail: null, detailIndex: null, loading: false, collecting: false,
    comparing: false, detailLoading: false, error: null, notice: null };
}
export function sameIndexTarget(a: IndexSummary, b: IndexSummary): boolean {
  // This is a minimum sanity check; host/port and DB OID are not an immutable server identity.
  return a.connection_id === b.connection_id && a.host === b.host && a.port === b.port &&
    a.database_oid === b.database_oid && a.database === b.database &&
    a.username === b.username && a.sslmode === b.sslmode;
}
export function summaryMatchesConnection(summary: IndexSummary | undefined, connection: ActivityConnection | undefined): boolean {
  return !!summary && !!connection && summary.connection_id === connection.id &&
    summary.host === connection.host && summary.port === connection.port &&
    summary.database === connection.database && summary.username === connection.username;
}
export const INDEX_LABELS: Record<string, string> = {
  actively_used: t('tsFinish.indexScanned'), low_evidence: t('tsFinish.indexLowEvidence'),
  suspiciously_unused: t('tsFinish.indexMayUnused'),
  possible_duplicate: t('tsFinish.indexPossibleDuplicate'),
  high_write_overhead: t('tsFinish.indexHighWrite'),
  needs_investigation: t('tsFinish.indexInvestigate'),
};
const INDEX_I18N_KEYS: Record<string, "diagTs.indexActive" | "diagTs.indexLowEvidence" | "diagTs.indexUnused" | "diagTs.indexDuplicate" | "diagTs.indexWriteOverhead" | "diagTs.indexInvestigate"> = {
  actively_used: "diagTs.indexActive", low_evidence: "diagTs.indexLowEvidence",
  suspiciously_unused: "diagTs.indexUnused", possible_duplicate: "diagTs.indexDuplicate",
  high_write_overhead: "diagTs.indexWriteOverhead", needs_investigation: "diagTs.indexInvestigate",
};
export function indexLabel(label: string): string {
  const key = INDEX_I18N_KEYS[label];
  return key ? t(key) : label;
}
export interface IndexFilter { search: string; mode: string; page: number; pageSize: number }
export function filteredIndexPage(snapshot: IndexSnapshot | null, filter: IndexFilter) {
  const search = filter.search.trim().toLocaleLowerCase();
  const rows = (snapshot?.collection.indexes ?? []).filter(row =>
    (filter.mode === "all" || row.classifications.includes(filter.mode)) &&
    (!search || `${row.schema_name}.${row.relation_name} ${row.index_name} ${row.index_oid}`.toLocaleLowerCase().includes(search)));
  const pageSize = [10, 25, 50].includes(filter.pageSize) ? filter.pageSize : 25;
  const pages = Math.max(1, Math.ceil(rows.length / pageSize));
  const page = Math.min(Math.max(1, Math.trunc(filter.page) || 1), pages);
  return { total: rows.length, page, pages, rows: rows.slice((page - 1) * pageSize, page * pageSize) };
}
export function indexDocsUrl(versionNum: number | null, path: string): string | null {
  if (!/^[a-z0-9.-]+(?:#[A-Za-z0-9_-]+)?$/.test(path)) return null;
  const major = versionNum === null ? 0 : Math.floor(versionNum / 10000);
  return `https://www.postgresql.org/docs/${major >= 10 && major <= 99 ? major : "current"}/${path}`;
}
export function createIndexHealthController(shell: ShellState, state: IndexHealthState, invoke: PgXrayInvoke) {
  let generation = 0, historyRequest = 0, snapshotRequest = 0, compareRequest = 0, detailRequest = 0;
  let disposed = false, connecting = false;
  const current = (auth: number, epoch: number) => !disposed && shell.screen === "workspace" &&
    shell.authGeneration === auth && generation === epoch;
  function clearView(): void {
    state.histories = []; state.snapshot = null; state.snapshotId = null; state.snapshotConnectionId = null;
    state.comparison = null; state.compareOlderId = null; state.compareNewerId = null;
    state.tableDetail = null; state.detailIndex = null; state.loading = false;
    state.comparing = false; state.detailLoading = false; state.error = null; state.notice = null;
  }
  function reset(): void { generation++; historyRequest++; snapshotRequest++; compareRequest++; detailRequest++; Object.assign(state, createIndexHealthState()); }
  function dispose(): void { reset(); disposed = true; }
  function selectConnection(id: number | null): void {
    if (state.collecting) return;
    generation++; historyRequest++; snapshotRequest++; compareRequest++; detailRequest++;
    state.connectionId = state.connections.some(c => c.id === id) ? id : null;
    clearView(); void refreshHistory();
  }
  async function refreshConnections(): Promise<void> {
    if (connecting || state.collecting || shell.screen !== "workspace") return;
    connecting = true;
    const auth = shell.authGeneration, epoch = generation;
    try {
      const connections = parseActivityConnections(await invoke("list_connections"));
      if (!current(auth, epoch)) return;
      state.connections = connections;
      const next = connections.some(c => c.id === state.connectionId) ? state.connectionId : (connections[0]?.id ?? null);
      if (next !== state.connectionId) selectConnection(next);
      else if (next !== null) await refreshHistory();
    } catch (e) { if (current(auth, epoch)) state.error = t('diagTs.connectionsError', { error: String(e) }); }
    finally { connecting = false; }
  }
  async function refreshHistory(): Promise<void> {
    const id = state.connectionId;
    if (id === null || shell.screen !== "workspace") return;
    const auth = shell.authGeneration, epoch = generation, request = ++historyRequest;
    state.loading = true; state.error = null;
    try {
      const histories = await invoke<IndexSummary[]>("list_index_health_snapshots", { connectionId: id, limit: 100 });
      if (!current(auth, epoch) || request !== historyRequest || state.connectionId !== id) return;
      if (!Array.isArray(histories) || histories.some(s => s.connection_id !== id)) throw new Error(t('diagTs.indexHistoryMismatch'));
      const selected = histories.find(s => s.id === state.snapshotId) ?? histories[0];
      const snapshot = selected ? await invoke<IndexSnapshot>("load_index_health_snapshot", { connectionId: id, snapshotId: selected.id }) : null;
      if (!current(auth, epoch) || request !== historyRequest || state.connectionId !== id) return;
      state.histories = histories;
      state.snapshot = snapshot;
      state.snapshotId = selected?.id ?? null;
      state.snapshotConnectionId = selected ? id : null;
      state.comparison = null; state.tableDetail = null; state.detailIndex = null;
      const [older, newer] = histories;
      state.compareOlderId = older && newer ? newer.id : null;
      state.compareNewerId = older?.id ?? null;
    } catch (e) { if (current(auth, epoch) && request === historyRequest) state.error = t('diagTs.indexHistoryError', { error: String(e) }); }
    finally { if (current(auth, epoch) && request === historyRequest) state.loading = false; }
  }
  async function loadSnapshot(snapshotId: number): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || state.loading || state.collecting || !state.histories.some(s => s.id === snapshotId && s.connection_id === id)) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++snapshotRequest;
    state.loading = true; state.error = null;
    try {
      const snapshot = await invoke<IndexSnapshot>("load_index_health_snapshot", { connectionId: id, snapshotId });
      if (!current(auth, epoch) || request !== snapshotRequest || state.connectionId !== id) return false;
      state.snapshot = snapshot; state.snapshotId = snapshotId; state.snapshotConnectionId = id;
      state.tableDetail = null; state.detailIndex = null;
      return true;
    } catch(e) { if (current(auth, epoch)) state.error = t('diagTs.indexOpenError', { error: String(e) }); return false; }
    finally { if (current(auth, epoch) && request === snapshotRequest) state.loading = false; }
  }
  async function collect(): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || state.collecting || state.loading || shell.screen !== "workspace") return false;
    const auth = shell.authGeneration, epoch = generation;
    state.collecting = true; state.error = null;
    try {
      const result = await invoke<{ history: IndexSummary; snapshot: IndexSnapshot }>("collect_index_health_snapshot", { connectionId: id });
      if (!current(auth, epoch) || state.connectionId !== id) return false;
      if (result.history.connection_id !== id) throw Error(t('diagTs.indexForeignSnapshot'));
      state.snapshotId = result.history.id;
      state.snapshot = result.snapshot; state.snapshotConnectionId = id;
      state.notice = t('diagTs.indexSaved', { id: String(result.history.id) });
      await refreshHistory();
      return true;
    } catch(e) { if (current(auth, epoch)) state.error = t('diagTs.indexCollectError', { error: String(e) }); return false; }
    finally { state.collecting = false; }
  }
  async function compare(): Promise<boolean> {
    const id = state.connectionId, olderId = state.compareOlderId, newerId = state.compareNewerId;
    const older = state.histories.find(s => s.id === olderId), newer = state.histories.find(s => s.id === newerId);
    if (id === null || !older || !newer || older.id === newer.id || !sameIndexTarget(older, newer) || state.comparing || state.loading) {
      state.error = t('diagTs.indexChooseSnapshots'); return false;
    }
    const auth = shell.authGeneration, epoch = generation, request = ++compareRequest;
    state.comparing = true; state.error = null; state.comparison = null;
    try {
      const result = await invoke<IndexComparison>("compare_index_health_snapshots", { firstId: older.id, secondId: newer.id });
      if (!current(auth, epoch) || request !== compareRequest || state.connectionId !== id ||
        state.compareOlderId !== olderId || state.compareNewerId !== newerId) return false;
      if (result.older.connection_id !== id || result.newer.connection_id !== id || !sameIndexTarget(result.older, result.newer)) throw Error(t('diagTs.indexComparisonWrongSource'));
      state.comparison = result; return true;
    } catch(e) { if (current(auth, epoch)) state.error = t('tsFinish.indexComparisonFailed', { error: String(e) }); return false; }
    finally { if (current(auth, epoch) && request === compareRequest) state.comparing = false; }
  }
  async function loadReference(): Promise<void> {
    const auth = shell.authGeneration, epoch = generation;
    try { const ref = await invoke<IndexReference>("index_health_reference"); if (current(auth, epoch)) state.reference = ref; }
    catch(e) { if (current(auth, epoch)) state.error = t('tsFinish.indexDocsFailed', { error: String(e) }); }
  }
  function closeTableDetail(): void { detailRequest++; state.tableDetail = null; state.detailIndex = null; state.detailLoading = false; }
  async function openTableDetail(index: IndexRow): Promise<boolean> {
    const id = state.connectionId, snapshot = state.snapshot;
    const saved = state.histories.find(s => s.id === state.snapshotId);
    const selected = state.connections.find(c => c.id === id);
    if (id !== null && !summaryMatchesConnection(saved, selected)) {
      state.error = t('diagTs.indexOldProfile');
      return false;
    }
    if (id === null || !snapshot || state.snapshotConnectionId !== id || state.collecting || state.loading || state.detailLoading ||
      !snapshot.collection.indexes.some(row => row.index_oid === index.index_oid && row.relation_oid === index.relation_oid)) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++detailRequest;
    state.detailLoading = true; state.detailIndex = index; state.tableDetail = null; state.error = null;
    try {
      // Live Table Health is requested with the selected connection only. Never reuse a historical autovacuum snapshot ID.
      const detail = await invoke<TableDetail>("autovacuum_table_detail", { connectionId: id, relationOid: index.relation_oid });
      if (!current(auth, epoch) || request !== detailRequest || state.connectionId !== id || state.snapshot !== snapshot) return false;
      if (detail.table.relation_oid !== index.relation_oid || detail.table.schema_name !== index.schema_name || detail.table.relation_name !== index.relation_name) throw Error(t('diagTs.indexTableChanged'));
      state.tableDetail = detail; return true;
    } catch(e) { if (current(auth, epoch) && request === detailRequest) state.error = `Table Health: ${String(e)}`; return false; }
    finally { if (current(auth, epoch) && request === detailRequest) state.detailLoading = false; }
  }
  return { reset, dispose, selectConnection, refreshConnections, refreshHistory, loadSnapshot, collect, compare, loadReference, openTableDetail, closeTableDetail };
}
