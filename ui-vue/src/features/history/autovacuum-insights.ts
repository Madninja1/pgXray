import { t } from "../i18n/locale-service.js";
export interface AvHistoryInterval {
  from_snapshot_id: number;
  to_snapshot_id: number;
  from_ms: number;
  to_ms: number;
  elapsed_seconds: number | null;
  state: string;
  comparable_relations: number;
  sampled_relations: number;
  inserts_per_hour: number | null;
  updates_per_hour: number | null;
  deletes_per_hour: number | null;
  vacuum_cycles: number | null;
  analyze_cycles: number | null;
  vacuum_cycles_per_hour: number | null;
  analyze_cycles_per_hour: number | null;
  dead_tuples_estimate: number | null;
  dead_trigger_reached_count: number;
  limitations: string[];
}
export interface AvHistoryDelta {
  relation_oid: number;
  sample_state: string;
  dead_tuples_delta: number | null;
  inserts_delta: number | null;
  updates_delta: number | null;
  deletes_delta: number | null;
  hot_updates_delta: number | null;
  vacuum_count_delta: number | null;
  autovacuum_count_delta: number | null;
  analyze_count_delta: number | null;
  autoanalyze_count_delta: number | null;
  vacuum_cycle_observed: boolean;
  analyze_cycle_observed: boolean;
  total_size_delta_bytes: number | null;
}
export interface AvHistoryRelation {
  relation_oid: number;
  relation_name: string;
  total_size_bytes: number;
  dead_tuples: number;
  dead_tuple_trigger: number;
  dead_trigger_used_pct: number | null;
  insert_trigger_used_pct: number | null;
  analyze_trigger_used_pct: number | null;
  dead_trigger_reached: boolean;
  insert_trigger_reached: boolean;
  analyze_trigger_reached: boolean;
  autovacuum_enabled: boolean;
  autovacuum_applicable: boolean;
  last_autovacuum: string | null;
  last_autoanalyze: string | null;
  delta: AvHistoryDelta | null;
}
export interface AvWorkloadHistory {
  connection_id: number;
  snapshot_count: number;
  latest_dead_tuples_estimate: number | null;
  selected_to_snapshot_id: number | null;
  intervals: AvHistoryInterval[];
  relations: AvHistoryRelation[];
  relation_count: number;
  limitations: string[];
}
export type AvRanking = "attention" | "dead" | "dml" | "size" | "all";
export type AvMetric = "inserts_per_hour" | "updates_per_hour" | "deletes_per_hour" | "vacuum_cycles_per_hour" | "analyze_cycles_per_hour" | "dead_tuples_estimate";

/** A trigger crossing is evidence for investigation, not proof of a failed VACUUM. */
export function needsAvAttention(row: AvHistoryRelation): boolean {
  return (row.autovacuum_applicable && !row.autovacuum_enabled) ||
    row.dead_trigger_reached || row.insert_trigger_reached || row.analyze_trigger_reached;
}
export function rankAutovacuum(rows: readonly AvHistoryRelation[], kind: AvRanking, query: string): AvHistoryRelation[] {
  const search = query.trim().toLocaleLowerCase();
  const match = rows.filter(row => (!search || row.relation_name.toLocaleLowerCase().includes(search) ||
    String(row.relation_oid).includes(search)) && (kind !== "attention" || needsAvAttention(row)));
  return match.sort((a,b) => {
    if (kind === "attention" || kind === "all") {
      const priority = Number(needsAvAttention(b)) - Number(needsAvAttention(a));
      if (priority !== 0) return priority;
    }
    const score = (row: AvHistoryRelation): number => {
      if (kind === "size") return row.total_size_bytes;
      if (kind === "dml") return (row.delta?.inserts_delta ?? 0) + (row.delta?.updates_delta ?? 0) + (row.delta?.deletes_delta ?? 0);
      return row.dead_trigger_used_pct ?? -1;
    };
    return score(b)-score(a) || b.total_size_bytes-a.total_size_bytes || a.relation_name.localeCompare(b.relation_name,"ru");
  });
}
export function avIntervalLabel(state: string): string {
  const states: Record<string,string> = {
    comparable:t('tsFinish.avHistComparable'), partial_sample:t('tsFinish.avHistPartial'), stats_reset_changed:t('tsFinish.avHistReset'),
    target_changed:t('tsFinish.avHistTarget'), observation_gap:t('tsFinish.avHistGap'), invalid_interval:t('tsFinish.avHistTime'),
    no_table_sample:t('tsFinish.avHistNoTable'), no_comparable_relations:t('tsFinish.avHistNoCommon'), unavailable:t('tsFinish.avHistUnavailable'),
  };
  return states[state] ?? state;
}
export interface AvChartPoint { x: number; y: number; ms: number; value: number; id: number }
/** Only connect directly adjacent usable pairs with no excessive gap. */
export function avTrendLines(intervals: readonly AvHistoryInterval[], metric: AvMetric): {lines: AvChartPoint[][]; points: AvChartPoint[]; max: number} {
  const rows = [...intervals].sort((a,b) => a.to_ms-b.to_ms);
  const valid = rows.filter(r => (metric === "dead_tuples_estimate" || r.state === "comparable" || r.state === "partial_sample") &&
    r[metric] != null && Number.isFinite(r[metric]));
  const max = Math.max(1, ...valid.map(row => Number(row[metric])));
  const start = valid[0]?.to_ms ?? 0, end = valid[valid.length-1]?.to_ms ?? start;
  const span = end-start;
  const points = valid.map(row => ({id:row.to_snapshot_id, ms:row.to_ms,value:Number(row[metric]),
    x:80+(span>0 ? (row.to_ms-start)/span : .5)*750,
    y:180-Math.max(0, Number(row[metric]))/max*140}));
  const lines: AvChartPoint[][] = [];
  let segment: AvChartPoint[] = [];
  for (const point of points) {
    const current = rows.find(row => row.to_snapshot_id===point.id);
    const prev = segment[segment.length-1];
    const preceding = prev ? rows.find(row => row.to_snapshot_id===prev.id) : null;
    // Adjacent snapshots are required: never connect over reset, missing sample or a long gap.
    if (prev && (!preceding || !current || preceding.to_snapshot_id!==current.from_snapshot_id ||
      !["comparable","partial_sample"].includes(preceding.state) ||
      !["comparable","partial_sample"].includes(current.state) ||
      current.to_ms-prev.ms>6*3600_000)) {
      lines.push(segment);
      segment=[];
    }
    segment.push(point);
  }
  if (segment.length) lines.push(segment);
  return {lines,points,max};
}
