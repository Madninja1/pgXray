import { locale, t } from "../i18n/locale-service.js";
import { KIND_LABELS, SNAPSHOT_KINDS, type SnapshotKind } from "../snapshots/schedule-model.js";

export { KIND_LABELS, SNAPSHOT_KINDS };
export type { SnapshotKind };

export interface HistorySnapshot {
  id: number;
  connection_id: number;
  connection_name: string;
  collection_started_at_ms: number;
  host: string;
  port: number;
  database: string;
  database_oid: number;
  username: string;
  sslmode: string;
  unavailable_sections?: number;
  [key: string]: unknown;
}
export interface HistoryConnection {
  connection_id: number;
  connection_name: string;
  snapshot_count: number;
}
export interface LiveConnection {
  id: number;
  name: string;
  host: string;
  port: number;
  database: string;
}
export const LIST_HISTORY_COMMANDS: Record<SnapshotKind, string> = {
  xid: "list_xid_snapshots", pgss: "list_pgss_snapshots", autovacuum: "list_autovacuum_snapshots",
  pg_settings: "list_pg_settings_snapshots", io_wal: "list_io_wal_snapshots",
  index_health: "list_index_health_snapshots",
};
export const LOAD_HISTORY_COMMANDS: Record<SnapshotKind, string> = {
  xid: "load_xid_snapshot", pgss: "load_pgss_snapshot", autovacuum: "load_autovacuum_snapshot",
  pg_settings: "load_pg_settings_snapshot", io_wal: "load_io_wal_snapshot",
  index_health: "load_index_health_snapshot",
};
export const COMPARE_HISTORY_COMMANDS: Record<SnapshotKind, string> = {
  xid: "compare_xid_snapshots", pgss: "compare_pgss_snapshots", autovacuum: "compare_autovacuum_snapshots",
  pg_settings: "compare_pg_settings_snapshots", io_wal: "compare_io_wal_snapshots",
  index_health: "compare_index_health_snapshots",
};
export const SNAPSHOT_METRICS: Record<SnapshotKind, readonly [string, string][]> = {
  get xid() { return [["XID", "current_xid"], [t('tsFinish.historyDbAge'), "database_xid_age"], [t('tsFinish.historyUnavailable'), "unavailable_sections"]] as [string, string][]; },
  get pgss() { return [["Dealloc", "dealloc"], [t('tsFinish.historyPgssCount'), "current_entries"], [t('tsFinish.historyFilled'), "fill_pct"]] as [string, string][]; },
  get autovacuum() { return [[t('tsFinish.historyTables'), "table_count"], ["Dead tuples", "total_dead_tuples"], [t('tsFinish.historyVacuum'), "active_vacuum_count"]] as [string, string][]; },
  get pg_settings() { return [[t('tsFinish.historySettings'), "setting_count"], [t('tsFinish.historyNonDefault'), "non_default_count"], [t('tsFinish.historyRestart'), "pending_restart_count"]] as [string, string][]; },
  get io_wal() { return [["WAL bytes", "wal_bytes"], [t('tsFinish.historyDbSize'), "database_size_bytes"], ["Temp bytes", "temp_bytes"]] as [string, string][]; },
  get index_health() { return [[t('tsFinish.historyIndexes'), "total_index_count"], [t('tsFinish.historyUnused'), "suspiciously_unused_count"], [t('tsFinish.historyDuplicates'), "possible_duplicate_count"]] as [string, string][]; },
};
/** Number of samples to estimate an intra-period rate, not a guarantee of comparable counters. */
export const PERIOD_MINIMUM: Record<SnapshotKind, number> = {
  xid: 2, pgss: 2, autovacuum: 2, pg_settings: 1, io_wal: 2, index_health: 1,
};
export function sameHistoryTarget(a: HistorySnapshot, b: HistorySnapshot): boolean {
  return a.connection_id === b.connection_id && a.host === b.host && a.port === b.port &&
    a.database_oid === b.database_oid && a.username === b.username;
}
export function countInRange(items: readonly HistorySnapshot[], from: number, to: number): number {
  return items.filter(item => item.collection_started_at_ms >= from && item.collection_started_at_ms <= to).length;
}
export function humanValue(value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  if (typeof value === "number") return Number.isFinite(value) ? value.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 2 }) : "—";
  if (typeof value === "boolean") return value ? t('tsFinish.historyYes') : t('tsFinish.historyNo');
  if (Array.isArray(value)) return t('tsFinish.historyRecords', { count: String(value.length) });
  if (typeof value === "object") return t('tsFinish.historyDetails');
  return String(value);
}
export function formatHistoryTime(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? new Date(value).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US") : "—";
}
export function sortedSnapshots(items: readonly HistorySnapshot[]): HistorySnapshot[] {
  return [...items].sort((a, b) => b.collection_started_at_ms - a.collection_started_at_ms || b.id - a.id);
}
