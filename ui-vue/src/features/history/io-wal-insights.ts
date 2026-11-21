import { t } from "../i18n/locale-service.js";
export interface IoWalHistoryInterval {
  from_snapshot_id: number;
  to_snapshot_id: number;
  from_ms: number;
  to_ms: number;
  elapsed_seconds: number | null;
  state: string;
  database_state: string;
  wal_state: string;
  checkpoint_state: string;
  wal_bytes_per_hour: number | null;
  wal_records_per_hour: number | null;
  wal_buffers_full_per_hour: number | null;
  wal_write_ms_per_hour: number | null;
  wal_sync_ms_per_hour: number | null;
  temp_bytes_per_hour: number | null;
  temp_files_per_hour: number | null;
  blocks_read_per_hour: number | null;
  blocks_hit_per_hour: number | null;
  checkpoints_requested_per_hour: number | null;
  checkpoints_timed_per_hour: number | null;
  checkpoint_buffers_per_hour: number | null;
  database_size_bytes: number | null;
  database_size_delta_bytes: number | null;
}
export interface IoWalSizePoint {
  snapshot_id: number;
  at_ms: number;
  database_size_bytes: number | null;
}
export interface IoWalSizeChange {
  relation_oid: number;
  older_name: string | null;
  newer_name: string | null;
  sample_state: string;
  heap_size_delta_bytes: number | null;
  indexes_size_delta_bytes: number | null;
  toast_size_delta_bytes: number | null;
  total_size_delta_bytes: number | null;
}
export interface IoWalWorkloadHistory {
  snapshot_count: number;
  intervals: IoWalHistoryInterval[];
  size_points: IoWalSizePoint[];
  selected_to_snapshot_id: number | null;
  relation_size_changes: IoWalSizeChange[];
  relation_sample_truncated: boolean;
  relation_sizes_available: boolean;
  limitations: string[];
}
export type IoWalHistoryMetric = "wal_bytes_per_hour" | "temp_bytes_per_hour" |
  "checkpoints_requested_per_hour" | "blocks_read_per_hour" | "database_size_bytes";
export interface IoWalChartPoint {
  id: number; ms: number; value: number; x: number; y: number;
}
export interface IoWalChart {
  segments: IoWalChartPoint[][];
  points: IoWalChartPoint[];
  maximum: number;
  minMs: number | null;
  maxMs: number | null;
}
export function ioWalStatus(value: string): string {
  switch (value) {
    case "comparable": return t('tsFinish.ioComparable');
    case "partial": return t('tsFinish.ioPartial');
    case "target_changed": return t('tsFinish.ioTargetChanged');
    case "invalid_time": return t('tsFinish.ioInvalidTime');
    case "gap": return t('tsFinish.ioGap');
    case "unavailable": return t('tsFinish.ioUnavailable');
    case "unknown_reset": return t('tsFinish.ioUnknownReset');
    case "reset": return t('tsFinish.ioReset');
    case "source_changed": return t('tsFinish.ioSourceChanged');
    default: return t('tsFinish.ioSectionUnavailable');
  }
}

export function ioWalChart(rows: readonly IoWalHistoryInterval[], sizes: readonly IoWalSizePoint[], metric: IoWalHistoryMetric): IoWalChart {
  const ordered = (metric === "database_size_bytes"
    ? sizes.map(row => ({id: row.snapshot_id, ms: row.at_ms, value: row.database_size_bytes,
      // A size gauge is meaningful even if a stats counter reset; however not across changed target.
      state: rows.find(i => i.to_snapshot_id === row.snapshot_id)?.state ?? "unavailable"}))
    : rows.map(row => ({id: row.to_snapshot_id, ms: row.to_ms, value: row[metric],
      state: metric === "wal_bytes_per_hour" ? row.wal_state
        : metric === "checkpoints_requested_per_hour" ? row.checkpoint_state : row.database_state})))
    .sort((a,b) => a.ms - b.ms);
  const allTimes = ordered.map(p => p.ms).filter(Number.isFinite);
  if (!allTimes.length) return {segments: [],points:[],maximum:1,minMs:null,maxMs:null};
  const minMs = Math.min(...allTimes), maxMs = Math.max(...allTimes);
  const maximum = Math.max(1,...ordered.map(p => p.value ?? 0).filter(Number.isFinite)) * 1.08;
  const point = (raw: typeof ordered[number]): IoWalChartPoint => ({id:raw.id,ms:raw.ms,value:raw.value as number,
    x:76+(maxMs>minMs ? (raw.ms-minMs)/(maxMs-minMs) : .5)*740,
    y:204-Math.max(0,raw.value as number)/maximum*160});
  const segments: IoWalChartPoint[][] = [];
  const points: IoWalChartPoint[] = [];
  let current: IoWalChartPoint[] = [];
  let previousMs: number | null = null;
  for (const raw of ordered) {
    const valueOk = raw.value !== null && Number.isFinite(raw.value) && raw.value >= 0;
    const stateOk = metric === "database_size_bytes" || raw.state === "comparable";
    if (!valueOk || !stateOk) {
      if (current.length) segments.push(current);
      current = [];
      previousMs = null;
      continue;
    }
    if (previousMs !== null && (raw.ms - previousMs > 6 * 3600_000 ||
      (metric === "database_size_bytes" && ["target_changed", "invalid_time"].includes(raw.state)))) {
      if (current.length) segments.push(current);
      current = [];
    }
    const next = point(raw);
    current.push(next); points.push(next);previousMs = raw.ms;
  }
  if (current.length) segments.push(current);
  return {segments,points,maximum,minMs,maxMs};
}

export function ioWalSizeChanges(rows: readonly IoWalSizeChange[], term: string): IoWalSizeChange[] {
  const needle = term.trim().toLocaleLowerCase();
  return rows.filter(row => !needle || `${row.older_name ?? ""} ${row.newer_name ?? ""} ${row.relation_oid}`.toLocaleLowerCase().includes(needle))
    .sort((a,b) => Math.abs(b.total_size_delta_bytes ?? 0)-Math.abs(a.total_size_delta_bytes ?? 0));
}
