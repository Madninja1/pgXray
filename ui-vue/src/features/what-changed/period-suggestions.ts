import { PERIOD_MINIMUM, SNAPSHOT_KINDS, type HistorySnapshot, type SnapshotKind } from "../history/model.js";

export type SnapshotCollections = Partial<Record<SnapshotKind, readonly HistorySnapshot[]>>;
export interface SnapshotSession {
  fromMs: number;
  toMs: number;
  total: number;
  kinds: SnapshotKind[];
}
export interface SuggestedPeriods {
  baselineFromMs: number;
  baselineToMs: number;
  incidentFromMs: number;
  incidentToMs: number;
  sessions: SnapshotSession[];
  usedSessions: number;
  readySources: number;
}

const SESSION_GAP_MS = 2 * 60_000;
const RANGE_PADDING_MS = 30_000;

function targetKey(row: HistorySnapshot): string {
  return JSON.stringify([row.host, row.port, row.database, row.database_oid, row.username]);
}

/**
 * Suggest two ordered periods from actual saved snapshots, never from wall-clock defaults.
 * Proximity groups are only a UI aid, not a claim that PostgreSQL counters are comparable.
 */
export function suggestComparisonPeriods(collections: SnapshotCollections): SuggestedPeriods | null {
  const entries = SNAPSHOT_KINDS.flatMap(kind => (collections[kind] ?? [])
    .filter(row => Number.isFinite(row.collection_started_at_ms) && row.collection_started_at_ms > 0)
    .map(row => ({ kind, row, ms: row.collection_started_at_ms })));
  if (!entries.length) return null;

  // An unchanged connection ID can point to a new PostgreSQL target over time.
  // Do not combine the older target with the most recent target in an automatic preset.
  const newest = entries.reduce((best, entry) => entry.ms > best.ms ? entry : best);
  const key = targetKey(newest.row);
  const rows = entries.filter(entry => targetKey(entry.row) === key).sort((a, b) => a.ms - b.ms);
  const groups: { fromMs: number; toMs: number; kinds: Set<SnapshotKind>; total: number }[] = [];
  for (const { kind, ms } of rows) {
    const previous = groups[groups.length - 1];
    if (!previous || ms - previous.toMs > SESSION_GAP_MS) {
      groups.push({ fromMs: ms, toMs: ms, kinds: new Set([kind]), total: 1 });
    } else {
      previous.toMs = ms;
      previous.kinds.add(kind);
      previous.total++;
    }
  }
  if (groups.length < 2) return null;

  const selected = groups.slice(-4);
  const split = selected.length >= 4 ? 2 : 1;
  const baseline = selected.slice(0, split);
  const incident = selected.slice(split);
  const baselineFromMs = baseline[0].fromMs - RANGE_PADDING_MS;
  const baselineToMs = baseline[baseline.length - 1].toMs + RANGE_PADDING_MS;
  const incidentFromMs = incident[0].fromMs - RANGE_PADDING_MS;
  const incidentToMs = incident[incident.length - 1].toMs + RANGE_PADDING_MS;
  const count = (kind: SnapshotKind, from: number, to: number) => (collections[kind] ?? []).filter(row =>
    targetKey(row) === key && row.collection_started_at_ms >= from && row.collection_started_at_ms <= to).length;
  const readySources = SNAPSHOT_KINDS.filter(kind => count(kind, baselineFromMs, baselineToMs) >= PERIOD_MINIMUM[kind] &&
    count(kind, incidentFromMs, incidentToMs) >= PERIOD_MINIMUM[kind]).length;
  return {
    baselineFromMs, baselineToMs, incidentFromMs, incidentToMs,
    sessions: groups.slice(-8).reverse().map(group => ({ ...group, kinds: [...group.kinds] })),
    usedSessions: selected.length, readySources,
  };
}
