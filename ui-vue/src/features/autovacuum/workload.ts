import { t } from "../i18n/locale-service.js";
import type { AvRelationHistory, AvTable } from "./controller.js";

export type AvWorkloadStatus = "observed" | "unavailable";
export interface AvWorkloadDelta {
  delta: number;
  averagePerHour: number;
}
export interface AvWorkloadWindow {
  olderSnapshotId: number;
  newerSnapshotId: number;
  olderAt: number;
  newerAt: number;
  elapsedMinutes: number;
  status: AvWorkloadStatus;
  reason: string;
  resetVerified: boolean;
  inserts: AvWorkloadDelta | null;
  updates: AvWorkloadDelta | null;
  deletes: AvWorkloadDelta | null;
  hotUpdates: AvWorkloadDelta | null;
  deadTuplesChange: number | null;
  insertsSinceVacuumChange: number | null;
  modificationsSinceAnalyzeChange: number | null;
  estimatedRowsChange: number | null;
}

interface WorkloadPoint {
  snapshot_id: number;
  collection_started_at_ms: number;
  stats_reset?: string | null;
  table: AvTable;
}

const cumulativeFields = [
  "tuples_inserted", "tuples_updated", "tuples_deleted", "hot_updates",
  "vacuum_count", "autovacuum_count", "analyze_count", "autoanalyze_count",
] as const;

function safeCounter(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

/** A point-in-time change in PostgreSQL's *estimates*. Never a DML rate. */
function estimateChange(before: number, after: number): number | null {
  return Number.isSafeInteger(before) && Number.isSafeInteger(after)
    && before >= 0 && after >= 0 && Number.isSafeInteger(after - before)
    ? after - before : null;
}

/**
 * Observational interval arithmetic only. This result MUST NOT feed Advisor,
 * Policy or Apply: NULL stats_reset cannot prove cumulative-counter continuity.
 * Even monotonic counters can conceal an intervening reset and subsequent growth.
 */
export function avWorkloadInterval(older: WorkloadPoint, newer: WorkloadPoint): AvWorkloadWindow {
  const elapsedMs = newer.collection_started_at_ms - older.collection_started_at_ms;
  const elapsedMinutes = elapsedMs / 60_000;
  const base = {
    olderSnapshotId: older.snapshot_id,
    newerSnapshotId: newer.snapshot_id,
    olderAt: older.collection_started_at_ms,
    newerAt: newer.collection_started_at_ms,
    elapsedMinutes: Number.isFinite(elapsedMinutes) ? elapsedMinutes : 0,
    deadTuplesChange: estimateChange(older.table.dead_tuples, newer.table.dead_tuples),
    insertsSinceVacuumChange: estimateChange(older.table.inserts_since_vacuum, newer.table.inserts_since_vacuum),
    modificationsSinceAnalyzeChange: estimateChange(older.table.modifications_since_analyze, newer.table.modifications_since_analyze),
    estimatedRowsChange: estimateChange(older.table.estimated_rows, newer.table.estimated_rows),
  };
  const unavailable = (reason: string): AvWorkloadWindow => ({
    ...base, status: "unavailable", reason, resetVerified: false,
    inserts: null, updates: null, deletes: null, hotUpdates: null,
  });
  if (!Number.isSafeInteger(older.collection_started_at_ms) || !Number.isSafeInteger(newer.collection_started_at_ms)
      || !Number.isFinite(elapsedMs) || elapsedMs < 60_000) {
    return unavailable(t('tsFinish.avShortInterval'));
  }
  const oldReset = older.stats_reset ?? null;
  const newReset = newer.stats_reset ?? null;
  if (oldReset !== newReset) {
    return unavailable(oldReset && newReset
      ? t('tsFinish.avResetChanged')
      : t('tsFinish.avResetOne'));
  }
  for (const field of cumulativeFields) {
    const before = older.table[field];
    const after = newer.table[field];
    if (!safeCounter(before) || !safeCounter(after)) {
      return unavailable(t('tsFinish.avCountersMissing'));
    }
    if (after < before) {
      return unavailable(t('tsFinish.avCounterDecreased', { field }));
    }
  }
  const elapsedHours = elapsedMs / 3_600_000;
  const metric = (field: "tuples_inserted" | "tuples_updated" | "tuples_deleted" | "hot_updates"): AvWorkloadDelta => {
    const delta = newer.table[field] - older.table[field];
    return { delta, averagePerHour: delta / elapsedHours };
  };
  return {
    ...base,
    status: "observed",
    resetVerified: oldReset !== null,
    reason: oldReset === null
      ? t('tsFinish.avUnknownBoth')
      : t('tsFinish.avResetMatches'),
    inserts: metric("tuples_inserted"),
    updates: metric("tuples_updated"),
    deletes: metric("tuples_deleted"),
    hotUpdates: metric("hot_updates"),
  };
}

/** Newest first; compare adjacent snapshots, never pretend a long gap is a live rate. */
export function avWorkloadWindows(history: AvRelationHistory | null, limit = 12): AvWorkloadWindow[] {
  const points = [...(history?.points ?? [])].sort((a, b) =>
    a.collection_started_at_ms - b.collection_started_at_ms || a.snapshot_id - b.snapshot_id);
  const result: AvWorkloadWindow[] = [];
  for (let index = points.length - 1; index > 0 && result.length < limit; index--) {
    result.push(avWorkloadInterval(points[index - 1], points[index]));
  }
  return result;
}
