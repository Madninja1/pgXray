import { locale, t } from "../i18n/locale-service.js";
import type { HistorySnapshot } from "./model.js";

export interface XidRelation {
  relation_oid: number;
  schema_name: string;
  relation_name: string;
  relation_type: string;
  xid_age: number;
  effective_freeze_max_age: number;
  pct_of_freeze_max: number | null;
  mxid_age: number | null;
  pct_of_multixact_freeze_max: number | null;
  total_size_bytes: number;
  last_autovacuum: string | null;
  last_vacuum: string | null;
}
export interface XidObservation { overview: {
  database_xid_age: number;
  autovacuum_freeze_max_age: number;
  database_mxid_age?: number | null;
  autovacuum_multixact_freeze_max_age?: number | null;
  postmaster_start_time?: string | null;
  in_recovery?: boolean | null;
}; relations: { data: XidRelation[] | null; error?: string | null }; }
export interface XidRelationAssessment extends XidRelation {
  identity: string;
  age_delta: number | null;
  freeze_observed: boolean;
}
export function xidRelations(current: XidObservation, previous?: XidObservation | null): XidRelationAssessment[] {
  const older = new Map((previous?.relations.data ?? []).map(r => [r.relation_oid, r]));
  return (current.relations.data ?? []).map(row => {
    const before = older.get(row.relation_oid);
    const same = before && before.schema_name === row.schema_name && before.relation_name === row.relation_name && before.relation_type === row.relation_type;
    const age_delta = same && before ? row.xid_age - before.xid_age : null;
    return { ...row, identity: `${row.schema_name}.${row.relation_name}`, age_delta, freeze_observed: age_delta !== null && age_delta < 0 };
  }).sort((a,b) => (b.pct_of_freeze_max ?? -1) - (a.pct_of_freeze_max ?? -1) || b.xid_age - a.xid_age);
}
/** Same snapshot age can occur on many relations with a shared relfrozenxid.
 * Keep MXID, effective threshold and freeze observations distinct, otherwise
 * grouping can conceal a relation that needs attention. */
export interface XidAgeCohort {
  key: string;
  representative: XidRelationAssessment;
  members: XidRelationAssessment[];
  system: boolean;
}
export function isSystemXidRelation(row: XidRelationAssessment): boolean {
  return row.schema_name === "pg_catalog" || row.schema_name === "information_schema" || row.schema_name.startsWith("pg_toast");
}
export function xidAgeCohorts(rows: readonly XidRelationAssessment[]): XidAgeCohort[] {
  const groups = new Map<string, XidAgeCohort>();
  for (const row of rows) {
    const system = isSystemXidRelation(row);
    // Use exact ages, not rounded percentages: rounded values can hide differences.
    const key = JSON.stringify([system, row.xid_age, row.effective_freeze_max_age,
      row.mxid_age, row.pct_of_multixact_freeze_max, row.age_delta, row.freeze_observed]);
    const existing = groups.get(key);
    if (existing) existing.members.push(row);
    else groups.set(key, {key, representative: row, members: [row], system});
  }
  return [...groups.values()].sort((a,b) =>
    Math.max(b.representative.pct_of_freeze_max ?? -1, b.representative.pct_of_multixact_freeze_max ?? -1) -
    Math.max(a.representative.pct_of_freeze_max ?? -1, a.representative.pct_of_multixact_freeze_max ?? -1) ||
    b.representative.xid_age - a.representative.xid_age || b.members.length - a.members.length);
}

/** A timeline gap is not an observed trajectory. Do not join observations
 * separated by more than six hours or three times the shortest observed cadence. */
export function xidTrendSegments(points: readonly ChartPoint[]): ChartPoint[][] {
  if (!points.length) return [];
  const ordered = [...points].sort((a,b) => a.ms-b.ms);
  const gaps = ordered.slice(1).map((p,i) => p.ms-ordered[i].ms).filter(g => g>0);
  const cadence = gaps.length ? Math.min(...gaps) : 0;
  const cutoff = Math.max(6 * 3600_000, 3 * cadence);
  const result: ChartPoint[][] = [];
  let segment: ChartPoint[] = [];
  for (const point of ordered) {
    if (segment.length && point.ms-segment[segment.length-1].ms>cutoff) {
      result.push(segment);
      segment=[];
    }
    segment.push(point);
  }
  if (segment.length) result.push(segment);
  return result;
}

/** PGSS time is accumulated over all invocations; time per hour may exceed one hour. */
export function formatPgssDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  if (Math.abs(ms) < 1000) return t('tsFinish.unitMilliseconds', { value: ms.toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US', { maximumFractionDigits: 2 }) });
  return t('tsFinish.unitSeconds', { value: (ms / 1000).toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US', { maximumFractionDigits: 1 }) });
}
export function formatPgssExecRate(msPerHour: number | null | undefined): string {
  if (msPerHour == null || !Number.isFinite(msPerHour)) return "—";
  return t('tsFinish.unitMinutesHour', { value: (msPerHour / 60_000).toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US', { maximumFractionDigits: 2 }) });
}

export interface ChartPoint { x: number; y: number; id: number; ms: number; value: number }
/** Time-based x axis; stable 0-based y axis with an actual freeze threshold. */
export function xidTrendPoints(rows: readonly HistorySnapshot[], freezeMaxAge: number): { points: ChartPoint[]; scaleMax: number; thresholdY: number | null } {
  const ordered = rows.slice().sort((a,b) => b.collection_started_at_ms-a.collection_started_at_ms);
  const anchor = ordered[0];
  const contiguous: HistorySnapshot[] = [];
  for (const row of ordered) {
    if (anchor && (row.connection_id!==anchor.connection_id || row.host!==anchor.host || row.port!==anchor.port ||
      row.database_oid!==anchor.database_oid || row.username!==anchor.username || row.sslmode!==anchor.sslmode)) break;
    if (typeof row.database_xid_age === "number" && Number.isFinite(row.database_xid_age)) contiguous.push(row);
  }
  const eligible = contiguous.slice(0,120).reverse();
  const maxAge = Math.max(0, ...eligible.map(r => Number(r.database_xid_age)));
  const scaleMax = Math.max(maxAge * 1.10, freezeMaxAge > 0 ? freezeMaxAge * 1.05 : 1, 1);
  const first = eligible[0]?.collection_started_at_ms ?? 0;
  const last = eligible[eligible.length-1]?.collection_started_at_ms ?? first;
  const span = last-first;
  const points = eligible.map(row => ({ id:row.id, ms:row.collection_started_at_ms,
    value:Number(row.database_xid_age), x:62+(span>0 ? (row.collection_started_at_ms-first)/span : .5)*764,
    y:200-Math.min(scaleMax, Math.max(0,Number(row.database_xid_age)))/scaleMax*170 }));
  return { points, scaleMax, thresholdY: freezeMaxAge>0 ? 200-freezeMaxAge/scaleMax*170 : null };
}
export interface PgssInterval {
  from_snapshot_id: number; to_snapshot_id: number; from_ms: number; to_ms: number;
  elapsed_seconds: number | null; state: string; sampled_entries: number; comparable_statements: number;
  global_calls_per_hour: number | null; global_exec_ms_per_hour: number | null;
  sampled_wal_bytes_per_hour: number | null; sampled_temp_blocks_per_hour: number | null;
}
export interface PgssMetrics {
  calls_per_hour: number; exec_ms_per_hour: number; mean_exec_ms: number | null;
  shared_reads_per_hour: number; temp_blocks_per_hour: number; wal_bytes_per_hour: number;
}
export interface PgssStatement {
  userid: number; dbid: number; queryid: string; toplevel: boolean;
  role_name: string | null; database: string | null;
  latest: PgssMetrics; previous: PgssMetrics | null;
}
export interface PgssWorkloadHistory {
  snapshot_count: number;
  selected_to_snapshot_id: number | null;
  compared_to_snapshot_id: number | null;
  intervals: PgssInterval[];
  statements: PgssStatement[];
  limitations: string[];
}
export type PgssRank = "latency" | "impact" | "exec" | "calls" | "wal" | "temp";
export function rankPgss(rows: readonly PgssStatement[], rank: PgssRank, search = ""): PgssStatement[] {
  const term = search.trim().toLocaleLowerCase();
  const score = (r: PgssStatement): number => {
    switch(rank) {
      case "latency": return r.previous?.mean_exec_ms != null && r.latest.mean_exec_ms != null ? r.latest.mean_exec_ms-r.previous.mean_exec_ms : -Infinity;
      case "impact": return r.previous ? r.latest.exec_ms_per_hour-r.previous.exec_ms_per_hour : -Infinity;
      case "exec": return r.latest.exec_ms_per_hour;
      case "calls": return r.latest.calls_per_hour;
      case "wal": return r.latest.wal_bytes_per_hour;
      case "temp": return r.latest.temp_blocks_per_hour;
    }
  };
  return rows.filter(r => {
    if (term && ![r.queryid,r.role_name ?? "",r.database ?? ""].some(v=>v.toLocaleLowerCase().includes(term))) return false;
    return score(r) > 0;
  }).sort((a,b) => score(b)-score(a) || b.latest.exec_ms_per_hour-a.latest.exec_ms_per_hour);
}
/** Break lines whenever observations are invalid, rather than interpolating a reset. */
export function pgssTrendSegments(rows: readonly PgssInterval[], field: keyof PgssInterval): ChartPoint[][] {
  const valid = rows.filter(r=>Number.isFinite(r.to_ms));
  if (!valid.length) return [];
  const tMin = valid[0].to_ms, tMax = valid[valid.length-1].to_ms;
  const values = valid.map(r => r[field]);
  const max = Math.max(1, ...values.filter((v): v is number => typeof v === "number" && Number.isFinite(v)));
  const pieces: ChartPoint[][] = [];
  let segment: ChartPoint[] = [];
  // Do not draw a continuous trajectory across days without observations.
  // A long interval still has a valid average rate, but the intervening trend is unknown.
  const gapLimitMs = 6 * 3600_000;
  for (const r of valid) {
    const value = r[field];
    if (r.state !== "comparable" || typeof value !== "number" || !Number.isFinite(value)) {
      if (segment.length) pieces.push(segment);
      segment = [];
      continue;
    }
    if (segment.length && r.to_ms - segment[segment.length - 1].ms > gapLimitMs) {
      pieces.push(segment);
      segment = [];
    }
    segment.push({id:r.to_snapshot_id,ms:r.to_ms,value,
      x:60+(tMax>tMin ? (r.to_ms-tMin)/(tMax-tMin) : .5)*760,
      y:188-Math.max(0,value)/max*156});
  }
  if(segment.length) pieces.push(segment);
  return pieces;
}
