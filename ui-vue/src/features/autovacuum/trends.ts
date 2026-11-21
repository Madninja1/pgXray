import type { AvComparison, AvRelationHistory } from "./controller.js";

export type AvTrendKind = "vacuum" | "insert" | "analyze";
export interface AvTrendPoint {
  snapshotId: number;
  measuredAt: number;
  observed: number;
  threshold: number;
  x: number;
  yObserved: number;
  yThreshold: number;
  manualCycles: number;
  automaticCycles: number;
}

/** Historical points are individual estimates, not a continuous series or a maintenance rate.
 * In particular, pg_stat_database.stats_reset=NULL never authorizes cumulative deltas. */
export function avBuildTrend(history: AvRelationHistory | null, kind: AvTrendKind) {
  const width = 680, height = 210, left = 58, right = 12, top = 14, bottom = 31;
  const rows = (history?.points ?? []).map(point => ({
    snapshotId: point.snapshot_id,
    measuredAt: point.collection_started_at_ms,
    observed: kind === "vacuum" ? point.table.dead_tuples : kind === "insert"
      ? point.table.inserts_since_vacuum : point.table.modifications_since_analyze,
    threshold: kind === "vacuum" ? point.table.dead_tuple_trigger : kind === "insert"
      ? (point.table.insert_trigger ?? Number.NaN) : point.table.analyze_trigger,
    manualCycles: kind === "analyze" ? point.table.analyze_count : point.table.vacuum_count,
    automaticCycles: kind === "analyze" ? point.table.autoanalyze_count : point.table.autovacuum_count,
  })).filter(p => Number.isFinite(p.measuredAt) && Number.isFinite(p.observed) && p.observed >= 0 &&
    Number.isFinite(p.threshold) && p.threshold > 0)
    .sort((a, b) => a.measuredAt - b.measuredAt || a.snapshotId - b.snapshotId)
    .slice(-60);
  const earliest = rows[0]?.measuredAt ?? 0;
  const latest = rows[rows.length - 1]?.measuredAt ?? earliest;
  const span = Math.max(1, latest - earliest);
  const ceiling = Math.max(1, ...rows.flatMap(p => [p.observed, p.threshold]));
  const x = (ms: number) => left + (ms - earliest) / span * (width - left - right);
  const y = (value: number) => top + (1 - value / ceiling) * (height - top - bottom);
  const points: AvTrendPoint[] = rows.map(p => ({...p, x: x(p.measuredAt), yObserved: y(p.observed), yThreshold: y(p.threshold)}));
  const line = (key: "yObserved" | "yThreshold") => points.length < 2 ? "" :
    points.map((p, i) => `${i ? "L" : "M"}${p.x.toFixed(2)},${p[key].toFixed(2)}`).join(" ");
  return { width, height, left, top, bottom, ceiling, earliest, latest, points,
    observedLine: line("yObserved"), thresholdLine: line("yThreshold") };
}

/** A summary of sampled estimates, not a computed DML rate. */
export function avComparisonOverview(comparison: AvComparison | null) {
  if (!comparison) return null;
  const rows = comparison.table_changes;
  return {
    total: rows.length,
    increased: rows.filter(r => r.dead_tuples_delta !== null && r.dead_tuples_delta > 0).length,
    decreased: rows.filter(r => r.dead_tuples_delta !== null && r.dead_tuples_delta < 0).length,
    unavailable: rows.filter(r => r.dead_tuples_delta === null).length,
    reliableCounters: comparison.stats_period_continuous,
  };
}

/** Change of TWO sampled estimates, not a DML rate or an ETA until VACUUM/ANALYZE.
 * A VACUUM or ANALYZE between points can reset these values. */
export function avLastSampleChange(history: AvRelationHistory | null, kind: AvTrendKind) {
  const rows = avBuildTrend(history, kind).points;
  if (rows.length < 2) return null;
  const before = rows[rows.length - 2];
  const after = rows[rows.length - 1];
  const elapsedMinutes = (after.measuredAt - before.measuredAt) / 60_000;
  if (!Number.isFinite(elapsedMinutes) || elapsedMinutes <= 0) return null;
  return { delta: after.observed - before.observed, elapsedMinutes };
}
