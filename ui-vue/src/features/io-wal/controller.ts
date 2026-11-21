import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { parseActivityConnections, type ActivityConnection } from "../activity/controller.js";

/** Rust JSON contracts: crates/core/src/io_wal.rs and history/io_wal.rs. */
export interface IoWalSection<T> { data: T | null; error: string | null }
export interface IoWalRelation {
  relation_oid: number; schema_name: string; relation_name: string; relation_type: string;
  heap_size_bytes: number; indexes_size_bytes: number; toast_size_bytes: number; total_size_bytes: number;
}
export interface IoWalSnapshot {
  overview: { measured_at: string; database: string; database_oid: number; username: string;
    server_version: string; server_version_num: number; track_io_timing: boolean;
    track_wal_io_timing: boolean | null; checkpoint_timeout: string;
    checkpoint_completion_target: number; max_wal_size: string; min_wal_size: string; block_size: number };
  database_stats: IoWalSection<{ stats_reset: string | null; temp_files: number; temp_bytes: number;
    blks_read: number; blks_hit: number; blk_read_time_ms: number; blk_write_time_ms: number;
    xact_commit: number; xact_rollback: number; deadlocks: number; conflicts: number }>;
  wal: IoWalSection<{ stats_reset: string; wal_records: number; wal_fpi: number; wal_bytes: number;
    wal_buffers_full: number; wal_write_time_ms: number; wal_sync_time_ms: number }>;
  checkpoints: IoWalSection<{ source: string; stats_reset: string; timed: number; requested: number;
    write_time_ms: number; sync_time_ms: number; buffers_written: number }>;
  database_size_bytes: IoWalSection<number>;
  relation_sizes: IoWalSection<{ total_relation_count: number; sampled_relation_count: number;
    truncated: boolean; relations: IoWalRelation[] }>;
  interpretation: { items: { id: string; title: string; observed: string[]; meaning: string[]; next_steps: string[] }[];
    limitations: string[] };
}
export interface IoWalSummary {
  id: number; connection_id: number; connection_name: string; host: string; port: number;
  database: string; database_oid: number; username: string; sslmode: string;
  server_version_num: number; server_version: string;
  collection_started_at_ms: number; collection_completed_at_ms: number; measured_at: string;
  database_size_bytes: number | null; temp_bytes: number | null; wal_bytes: number | null;
  checkpoint_timed: number | null; checkpoint_requested: number | null;
  relation_count: number | null; relation_sample_truncated: boolean; unavailable_sections: number;
}
export interface IoWalRelationChange {
  relation_oid: number; older_name: string | null; newer_name: string | null;
  sample_state: string; older_total_size_bytes: number | null;
  newer_total_size_bytes: number | null; total_size_delta_bytes: number | null;
}
export interface IoWalComparison {
  older: IoWalSummary; newer: IoWalSummary; elapsed_seconds: number;
  database_stats_continuous: boolean; wal_stats_continuous: boolean; checkpoint_stats_continuous: boolean;
  temp_files_delta: number | null; temp_bytes_delta: number | null; temp_bytes_per_hour: number | null;
  database_blocks_read_delta: number | null; database_blocks_hit_delta: number | null;
  wal_bytes_delta: number | null; wal_bytes_per_hour: number | null;
  wal_buffers_full_delta: number | null; wal_write_time_delta_ms: number | null;
  wal_sync_time_delta_ms: number | null; checkpoints_timed_delta: number | null;
  checkpoints_requested_delta: number | null; checkpoint_write_time_delta_ms: number | null;
  checkpoint_sync_time_delta_ms: number | null; checkpoint_buffers_written_delta: number | null;
  database_size_delta_bytes: number | null; relation_size_changes: IoWalRelationChange[];
  limitations: string[];
}
export interface IoWalReference { methodology: string[]; sources: { id: string; title: string; purpose: string; path: string }[] }
export interface IoWalState {
  connections: ActivityConnection[]; connectionId: number | null; histories: IoWalSummary[];
  snapshot: IoWalSnapshot | null; snapshotId: number | null; comparison: IoWalComparison | null;
  compareOlderId: number | null; compareNewerId: number | null; reference: IoWalReference | null;
  loading: boolean; collecting: boolean; comparing: boolean; error: string | null; notice: string | null;
}
export function createIoWalState(): IoWalState {
  return { connections: [], connectionId: null, histories: [], snapshot: null, snapshotId: null,
    comparison: null, compareOlderId: null, compareNewerId: null, reference: null,
    loading: false, collecting: false, comparing: false, error: null, notice: null };
}
/** Configuration identity is a defensive check, not a proof that the server never changed. */
export function sameIoWalTarget(a: IoWalSummary, b: IoWalSummary): boolean {
  return a.connection_id === b.connection_id && a.host === b.host && a.port === b.port &&
    a.database === b.database && a.database_oid === b.database_oid &&
    a.username === b.username && a.sslmode === b.sslmode;
}
export function ioWalSummaryMatchesConnection(summary: IoWalSummary | undefined, connection: ActivityConnection | undefined): boolean {
  return !!summary && !!connection && summary.connection_id === connection.id &&
    summary.host === connection.host && summary.port === connection.port &&
    summary.database === connection.database && summary.username === connection.username;
}
export type IoWalRelationSort = "name" | "total" | "heap" | "indexes" | "toast";
export function filteredIoWalPage(
  snapshot: IoWalSnapshot | null, search: string, page: number, pageSize: number,
  sort: IoWalRelationSort = "total", descending = true,
) {
  const needle = search.trim().toLocaleLowerCase();
  const rows = (snapshot?.relation_sizes.data?.relations ?? []).filter(row =>
    !needle || `${row.schema_name}.${row.relation_name} ${row.relation_type} ${row.relation_oid}`.toLocaleLowerCase().includes(needle));
  const sizeKey: Record<Exclude<IoWalRelationSort, "name">, keyof IoWalRelation> = {
    total: "total_size_bytes", heap: "heap_size_bytes", indexes: "indexes_size_bytes", toast: "toast_size_bytes",
  };
  // Sort the entire saved sample before pagination; no extra PostgreSQL requests.
  rows.sort((a, b) => {
    const comparison = sort === "name"
      ? `${a.schema_name}.${a.relation_name}`.localeCompare(`${b.schema_name}.${b.relation_name}`, "ru")
      : Number(a[sizeKey[sort]]) - Number(b[sizeKey[sort]]);
    return (descending ? -comparison : comparison) || a.relation_oid - b.relation_oid;
  });
  const size = [10, 25, 50].includes(pageSize) ? pageSize : 25;
  const pages = Math.max(1, Math.ceil(rows.length / size));
  const currentPage = Math.max(1, Math.min(pages, Math.trunc(page) || 1));
  return { rows: rows.slice((currentPage - 1) * size, currentPage * size),
    page: currentPage, pages, total: rows.length };
}
export function ioWalDocsUrl(versionNum: number | null, path: string): string | null {
  if (!/^[a-z0-9.-]+(?:#[A-Za-z0-9_-]+)?$/.test(path)) return null;
  const major = versionNum === null ? 0 : Math.floor(versionNum / 10000);
  return `https://www.postgresql.org/docs/${major >= 10 && major <= 99 ? major : "current"}/${path}`;
}

export function createIoWalController(shell: ShellState, state: IoWalState, invoke: PgXrayInvoke) {
  let generation = 0, historyRequest = 0, snapshotRequest = 0, compareRequest = 0;
  let connecting = false, disposed = false;
  const current = (auth: number, epoch: number) => !disposed && shell.screen === "workspace" &&
    shell.authGeneration === auth && generation === epoch;
  function clearView(): void {
    state.histories = []; state.snapshot = null; state.snapshotId = null; state.comparison = null;
    state.compareOlderId = null; state.compareNewerId = null;
    state.loading = false; state.collecting = false; state.comparing = false;
    state.error = null; state.notice = null;
  }
  function reset(): void {
    generation++; historyRequest++; snapshotRequest++; compareRequest++;
    Object.assign(state, createIoWalState());
  }
  function dispose(): void { reset(); disposed = true; }
  function selectConnection(id: number | null): void {
    generation++; historyRequest++; snapshotRequest++; compareRequest++;
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
      const id = connections.some(c => c.id === state.connectionId) ? state.connectionId : (connections[0]?.id ?? null);
      if (id !== state.connectionId) selectConnection(id);
      else if (id !== null) await refreshHistory();
    } catch (error) { if (current(auth, epoch)) state.error = t('diagTs.connectionsError', { error: String(error) }); }
    finally { connecting = false; }
  }
  async function refreshHistory(): Promise<void> {
    const id = state.connectionId;
    if (id === null || shell.screen !== "workspace") return;
    const auth = shell.authGeneration, epoch = generation, request = ++historyRequest;
    state.loading = true; state.error = null;
    try {
      const histories = await invoke<IoWalSummary[]>("list_io_wal_snapshots", { connectionId: id, limit: 100 });
      if (!current(auth, epoch) || request !== historyRequest || state.connectionId !== id) return;
      if (!Array.isArray(histories) || histories.some(s => s.connection_id !== id)) throw Error(t('diagTs.ioHistoryMismatch'));
      const chosen = histories.find(s => s.id === state.snapshotId) ?? histories[0];
      const snapshot = chosen ? await invoke<IoWalSnapshot>("load_io_wal_snapshot", { connectionId: id, snapshotId: chosen.id }) : null;
      if (!current(auth, epoch) || request !== historyRequest || state.connectionId !== id) return;
      if (snapshot && (snapshot.overview.database !== chosen?.database || snapshot.overview.database_oid !== chosen.database_oid))
        throw Error(t('diagTs.ioArchiveMismatch'));
      state.histories = histories; state.snapshot = snapshot; state.snapshotId = chosen?.id ?? null;
      // A new list invalidates previous comparison; defaults are two latest IDs.
      state.comparison = null; state.compareNewerId = histories[0]?.id ?? null;
      state.compareOlderId = histories[1]?.id ?? null;
    } catch (error) { if (current(auth, epoch) && request === historyRequest) state.error = t('diagTs.ioHistoryError', { error: String(error) }); }
    finally { if (current(auth, epoch) && request === historyRequest) state.loading = false; }
  }
  async function collect(): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || state.collecting || state.loading || shell.screen !== "workspace") return false;
    const auth = shell.authGeneration, epoch = generation;
    state.collecting = true; state.error = null;
    try {
      const result = await invoke<{ history: IoWalSummary; snapshot: IoWalSnapshot }>("collect_io_wal_snapshot", { connectionId: id });
      if (!current(auth, epoch) || state.connectionId !== id) return false;
      if (result.history.connection_id !== id || result.history.database !== result.snapshot.overview.database ||
        result.history.database_oid !== result.snapshot.overview.database_oid) throw Error(t('diagTs.ioCollectedWrongSource'));
      state.snapshot = result.snapshot; state.snapshotId = result.history.id;
      state.notice = t('diagTs.ioSaved', { id: String(result.history.id) });
      await refreshHistory();
      return true;
    } catch (error) { if (current(auth, epoch)) state.error = t('diagTs.ioCollectError', { error: String(error) }); return false; }
    finally { if (current(auth, epoch)) state.collecting = false; }
  }
  async function loadSnapshot(snapshotId: number): Promise<boolean> {
    const id = state.connectionId;
    if (id === null || state.loading || state.collecting || !state.histories.some(s => s.id === snapshotId && s.connection_id === id)) return false;
    const auth = shell.authGeneration, epoch = generation, request = ++snapshotRequest;
    state.loading = true; state.error = null;
    try {
      const snapshot = await invoke<IoWalSnapshot>("load_io_wal_snapshot", { connectionId: id, snapshotId });
      if (!current(auth, epoch) || request !== snapshotRequest || state.connectionId !== id) return false;
      const summary = state.histories.find(s => s.id === snapshotId);
      if (!summary || summary.database !== snapshot.overview.database || summary.database_oid !== snapshot.overview.database_oid)
        throw Error(t('diagTs.ioSnapshotWrongSource'));
      state.snapshot = snapshot; state.snapshotId = snapshotId; state.comparison = null; return true;
    } catch (error) { if (current(auth, epoch) && request === snapshotRequest) state.error = t('diagTs.ioSnapshotOpenError', { error: String(error) }); return false; }
    finally { if (current(auth, epoch) && request === snapshotRequest) state.loading = false; }
  }
  async function compare(): Promise<boolean> {
    const id = state.connectionId, olderId = state.compareOlderId, newerId = state.compareNewerId;
    const older = state.histories.find(s => s.id === olderId), newer = state.histories.find(s => s.id === newerId);
    if (id === null || !older || !newer || older.id === newer.id || !sameIoWalTarget(older, newer) ||
      older.collection_started_at_ms >= newer.collection_started_at_ms || state.comparing || state.loading) {
      state.error = t('diagTs.ioSelectPair'); return false;
    }
    const auth = shell.authGeneration, epoch = generation, request = ++compareRequest;
    state.comparing = true; state.comparison = null; state.error = null;
    try {
      const comparison = await invoke<IoWalComparison>("compare_io_wal_snapshots", { firstId: olderId, secondId: newerId });
      if (!current(auth, epoch) || request !== compareRequest || state.connectionId !== id ||
        state.compareOlderId !== olderId || state.compareNewerId !== newerId) return false;
      if (comparison.older.id !== olderId || comparison.newer.id !== newerId ||
        !sameIoWalTarget(comparison.older, comparison.newer) || comparison.older.connection_id !== id)
        throw Error(t('diagTs.ioComparisonMismatch'));
      state.comparison = comparison; return true;
    } catch (error) { if (current(auth, epoch) && request === compareRequest) state.error = t('diagTs.ioCompareError', { error: String(error) }); return false; }
    finally { if (current(auth, epoch) && request === compareRequest) state.comparing = false; }
  }
  async function loadReference(): Promise<void> {
    const auth = shell.authGeneration, epoch = generation;
    try {
      const reference = await invoke<IoWalReference>("io_wal_reference");
      if (current(auth, epoch)) state.reference = reference;
    } catch (error) { if (current(auth, epoch)) state.error = t('diagTs.ioMethodologyError', { error: String(error) }); }
  }
  return { reset, dispose, selectConnection, refreshConnections, refreshHistory, collect, loadSnapshot, compare, loadReference };
}
