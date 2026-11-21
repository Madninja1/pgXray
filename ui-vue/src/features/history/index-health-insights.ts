import { t } from "../i18n/locale-service.js";
import type { IndexDelta } from "../index-health/controller.js";
import type { HistorySnapshot } from "./model.js";

/** Index Health summaries are stored in SQLite; this view never collects from PostgreSQL. */
export interface IndexHistorySummary extends HistorySnapshot {
  stats_reset: string | null;
  sslmode: string;
  total_index_count: number;
  sampled_index_count: number;
  suspiciously_unused_count: number;
  possible_duplicate_count: number;
  high_write_overhead_count: number;
  sample_truncated: boolean;
}
export function isIndexHistorySummary(row: HistorySnapshot): row is IndexHistorySummary {
  return typeof row.sslmode === "string" &&
    (row.stats_reset === null || typeof row.stats_reset === "string") &&
    typeof row.sample_truncated === "boolean" &&
    ["total_index_count", "sampled_index_count", "suspiciously_unused_count",
      "possible_duplicate_count", "high_write_overhead_count"].every(key =>
      typeof row[key] === "number" && Number.isFinite(row[key]));
}
export type IndexHistoryMetric = "total_index_count" | "sampled_index_count" |
  "suspiciously_unused_count" | "possible_duplicate_count" | "high_write_overhead_count";
export const INDEX_HISTORY_METRICS: readonly { key: IndexHistoryMetric; label: string }[] = [
  {key: "total_index_count", label: t('tsFinish.totalIndexCount')},
  {key: "sampled_index_count", label: t('tsFinish.indexSampled')},
  {key: "suspiciously_unused_count", label: t('tsFinish.indexUnused')},
  {key: "possible_duplicate_count", label: t('tsFinish.indexDuplicates')},
  {key: "high_write_overhead_count", label: t('tsFinish.indexWriteCost')},
];
export interface IndexHistoryPair { older: IndexHistorySummary; newer: IndexHistorySummary }
export type IndexPairState = "comparable" | "partial_sample" | "gap" | "reset" |
  "unknown_reset" | "target_changed" | "invalid_time";
export const MAX_CONTINUOUS_GAP_MS = 6 * 3600_000;

export function sameIndexHistoryTarget(a: IndexHistorySummary, b: IndexHistorySummary): boolean {
  // Physical server identity cannot be proven by host/port/DB OID alone.
  return a.connection_id === b.connection_id && a.host === b.host && a.port === b.port &&
    a.database === b.database && a.database_oid === b.database_oid &&
    a.username === b.username && a.sslmode === b.sslmode;
}
export function indexHistoryPairs(rows: readonly IndexHistorySummary[]): IndexHistoryPair[] {
  const ordered = [...rows].sort((a,b) => b.collection_started_at_ms-a.collection_started_at_ms || b.id-a.id);
  return ordered.slice(0,-1).map((newer,i) => ({older:ordered[i+1],newer}));
}
export function indexPairState(pair: IndexHistoryPair): IndexPairState {
  const {older, newer} = pair;
  if (!sameIndexHistoryTarget(older,newer)) return "target_changed";
  const diff = newer.collection_started_at_ms - older.collection_started_at_ms;
  if (!Number.isFinite(diff) || diff <= 0) return "invalid_time";
  if (older.stats_reset === null || newer.stats_reset === null) return "unknown_reset";
  if (older.stats_reset !== newer.stats_reset) return "reset";
  if (diff > MAX_CONTINUOUS_GAP_MS) return "gap";
  if (older.sample_truncated || newer.sample_truncated) return "partial_sample";
  return "comparable";
}
export function indexPairStatus(value: IndexPairState): string {
  switch(value) {
    case "comparable": return t('tsFinish.indexComparable');
    case "partial_sample": return t('tsFinish.indexPartialSample');
    case "gap": return t('tsFinish.indexGap');
    case "reset": return t('tsFinish.indexReset');
    case "unknown_reset": return t('tsFinish.indexResetUnknown');
    case "target_changed": return t('tsFinish.indexTargetChanged');
    case "invalid_time": return t('tsFinish.indexInvalidTime');
  }
}
export interface IndexHistoryPoint {id: number; ms: number; value: number; x: number; y: number}
export interface IndexHistoryChart {
  points: IndexHistoryPoint[]; segments: IndexHistoryPoint[][];
  maximum: number; minMs: number | null; maxMs: number | null;
}
/** Counts are gauges at collection time, not deltas/rates. Break lines at gaps/target changes.
 * Classification counts from truncated samples do not form continuous evidence. */
export function indexHistoryChart(rows: readonly IndexHistorySummary[], metric: IndexHistoryMetric): IndexHistoryChart {
  const ordered = [...rows].sort((a,b) => a.collection_started_at_ms-b.collection_started_at_ms || a.id-b.id);
  const minMs = ordered[0]?.collection_started_at_ms ?? null;
  const maxMs = ordered[ordered.length-1]?.collection_started_at_ms ?? null;
  const maximum = Math.max(1,...ordered.map(r => r[metric]).filter(Number.isFinite))*1.08;
  const points: IndexHistoryPoint[] = [], segments: IndexHistoryPoint[][] = [];
  let section: IndexHistoryPoint[] = [];
  for (let i=0;i<ordered.length;i++) {
    const row=ordered[i], value=row[metric];
    const valid = Number.isFinite(value) && value >= 0 && Number.isFinite(row.collection_started_at_ms);
    if (!valid) { if(section.length) segments.push(section);section=[];continue; }
    const p:IndexHistoryPoint={id:row.id,ms:row.collection_started_at_ms,value,
      x:76+(minMs !== null && maxMs !== null && maxMs>minMs ? (row.collection_started_at_ms-minMs)/(maxMs-minMs) : .5)*740,
      y:204-value/maximum*160};
    const previous=ordered[i-1];
    if (previous && (row.collection_started_at_ms-previous.collection_started_at_ms>MAX_CONTINUOUS_GAP_MS ||
      !sameIndexHistoryTarget(previous,row) ||
      (metric !== "total_index_count" && (row.sample_truncated || previous.sample_truncated ||
        row.stats_reset === null || previous.stats_reset === null || row.stats_reset !== previous.stats_reset)))) {
      if(section.length) segments.push(section);section=[];
    }
    section.push(p);points.push(p);
  }
  if(section.length) segments.push(section);
  return {points,segments,maximum,minMs,maxMs};
}
export type IndexChangeFilter = "growth" | "scans" | "silent" | "coverage" | "all";
export function indexHistoryChanges(rows: readonly IndexDelta[], mode: IndexChangeFilter, search: string): IndexDelta[] {
  const needle=search.trim().toLocaleLowerCase();
  return rows.filter(row => {
    const text=`${row.relation_name} ${row.index_name} ${row.index_oid}`.toLocaleLowerCase();
    if (needle && !text.includes(needle)) return false;
    if (mode === "growth") return row.sample_state === "comparable" && row.size_delta_bytes !== null && row.size_delta_bytes !== 0;
    if (mode === "scans") return row.sample_state === "comparable" && row.idx_scan_delta !== null && row.idx_scan_delta > 0;
    if (mode === "silent") return row.sample_state === "comparable" && row.idx_scan_delta === 0 &&
      row.relation_dml_delta !== null && row.relation_dml_delta > 0;
    if (mode === "coverage") return row.sample_state !== "comparable";
    return true;
  }).sort((a,b) => {
    if (mode === "scans") return (b.idx_scan_delta ?? -1)-(a.idx_scan_delta ?? -1);
    if (mode === "silent") return (b.relation_dml_delta ?? 0)-(a.relation_dml_delta ?? 0);
    return Math.abs(b.size_delta_bytes ?? 0)-Math.abs(a.size_delta_bytes ?? 0) ||
      (b.idx_scan_delta ?? 0)-(a.idx_scan_delta ?? 0);
  });
}
export function indexHistoryTotals(rows: readonly IndexDelta[]) {
  const comparable=rows.filter(r=>r.sample_state === "comparable");
  const measured=comparable.filter(r=>r.idx_scan_delta !== null);
  const sizeRows=comparable.filter(r=>r.size_delta_bytes !== null);
  return {
    matched: comparable.length,
    unmatched: rows.length-comparable.length,
    measured: measured.length,
    scans: measured.length ? measured.reduce((sum,row)=>sum+(row.idx_scan_delta ?? 0),0) : null,
    netSizeBytes: sizeRows.length ? sizeRows.reduce((sum,row)=>sum+(row.size_delta_bytes ?? 0),0) : null,
    growing: sizeRows.filter(row=>(row.size_delta_bytes ?? 0)>0).length,
    silentWithDml: measured.filter(row=>row.idx_scan_delta === 0 && (row.relation_dml_delta ?? 0)>0).length,
  };
}
