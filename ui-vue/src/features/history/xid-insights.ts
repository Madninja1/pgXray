import { sameHistoryTarget, type HistorySnapshot } from "./model.js";

/** Read-only overview from stored XID snapshots. No live PostgreSQL query. */
export interface XidHistoryOverview {
  summary: HistorySnapshot & {
    current_xid: string;
    database_xid_age: number;
  };
  database_mxid_age: number | null;
  autovacuum_freeze_max_age: number;
  autovacuum_multixact_freeze_max_age: number | null;
  postmaster_start_time: string | null;
  in_recovery: boolean | null;
}
export type XidMetric = "xid" | "mxid";
export type XidIntervalState = "observed" | "gap" | "restart" | "recovery" | "xid_rewind" | "unknown_continuity";
export interface XidIntervalObservation {
  older: XidHistoryOverview;
  newer: XidHistoryOverview;
  state: XidIntervalState;
  elapsed_seconds: number;
  xid_age_delta: number | null;
  mxid_age_delta: number | null;
  consumed_xids: bigint | null;
  xid_per_hour: number | null;
  horizon_advanced: boolean;
  mxid_horizon_advanced: boolean;
  threshold_changed: boolean;
}
export interface XidTimeline {
  points: XidHistoryOverview[];
  intervals: XidIntervalObservation[];
  ignoredTargets: number;
}

function xidCounter(raw: string): bigint | null {
  // xid8 may exceed Number.MAX_SAFE_INTEGER; never parse it through Number.
  if (!/^\d+$/.test(raw)) return null;
  try { return BigInt(raw); } catch { return null; }
}

/** A difference in XID age is evidence about a snapshot, not proof of VACUUM.
 * The cluster's system_identifier is not stored, so even observed intervals
 * remain conditional rather than cryptographically/physically verified. */
export function xidHistoryTimeline(source: readonly XidHistoryOverview[]): XidTimeline {
  const sorted = [...source].sort((a,b) => a.summary.collection_started_at_ms - b.summary.collection_started_at_ms || a.summary.id - b.summary.id);
  const anchor = sorted[sorted.length-1];
  if (!anchor) return { points: [], intervals: [], ignoredTargets: 0 };
  // Keep only the latest contiguous logical-target segment. Crossing a target
  // change must never draw a continuous trend even if the connection ID is reused.
  let start = sorted.length-1;
  while (start > 0 && sameHistoryTarget(sorted[start-1].summary, anchor.summary) &&
    sorted[start-1].summary.sslmode === anchor.summary.sslmode) start--;
  const points = sorted.slice(start);
  const gaps = points.slice(1).map((point,i) => point.summary.collection_started_at_ms - points[i].summary.collection_started_at_ms)
    .filter(g => g > 0);
  const cadence = gaps.length ? Math.min(...gaps) : 0;
  // Never join long gaps even when the only observed cadence is days apart.
  const maxGap = cadence > 0 ? Math.min(6 * 60 * 60_000, 3 * cadence) : 6 * 60 * 60_000;
  const intervals: XidIntervalObservation[] = [];
  for (let i=1; i<points.length; i++) {
    const older=points[i-1], newer=points[i];
    const dt=newer.summary.collection_started_at_ms-older.summary.collection_started_at_ms;
    const oldCounter=xidCounter(older.summary.current_xid), newCounter=xidCounter(newer.summary.current_xid);
    let state: XidIntervalState = "observed";
    if (dt <= 0 || dt > maxGap) state="gap";
    else if (!older.postmaster_start_time || !newer.postmaster_start_time ||
      older.in_recovery == null || newer.in_recovery == null) state="unknown_continuity";
    else if (older.postmaster_start_time !== newer.postmaster_start_time) state="restart";
    else if (older.in_recovery !== newer.in_recovery) state="recovery";
    else if (oldCounter === null || newCounter === null) state="unknown_continuity";
    else if (newCounter < oldCounter) state="xid_rewind";
    const observed=state === "observed";
    const xid_age_delta=observed ? newer.summary.database_xid_age-older.summary.database_xid_age : null;
    const mxid_age_delta=observed && older.database_mxid_age != null && newer.database_mxid_age != null
      ? newer.database_mxid_age-older.database_mxid_age : null;
    const consumed_xids=observed && oldCounter !== null && newCounter !== null ? newCounter-oldCounter : null;
    const xid_per_hour=consumed_xids !== null && dt>0 && consumed_xids<=BigInt(Number.MAX_SAFE_INTEGER)
      ? Number(consumed_xids)*3_600_000/dt : null;
    intervals.push({older,newer,state,elapsed_seconds:dt/1000,
      xid_age_delta,mxid_age_delta,consumed_xids,xid_per_hour,
      horizon_advanced:xid_age_delta !== null && xid_age_delta < 0,
      mxid_horizon_advanced:mxid_age_delta !== null && mxid_age_delta < 0,
      threshold_changed:older.autovacuum_freeze_max_age!==newer.autovacuum_freeze_max_age ||
        older.autovacuum_multixact_freeze_max_age!==newer.autovacuum_multixact_freeze_max_age,
    });
  }
  return { points, intervals, ignoredTargets: start };
}

export interface XidChartPoint { id:number; ms:number; value:number; x:number; y:number; connectToPrevious:boolean }
export interface XidChart { points:XidChartPoint[]; yMin:number; yMax:number; thresholdY:number | null; threshold:number|null }
export function xidHistoryChart(timeline:XidTimeline, metric:XidMetric, zoom=false): XidChart {
  const valueOf=(p:XidHistoryOverview)=>metric==="xid" ? p.summary.database_xid_age : p.database_mxid_age;
  const thresholdOf=(p:XidHistoryOverview)=>metric==="xid" ? p.autovacuum_freeze_max_age : p.autovacuum_multixact_freeze_max_age;
  const eligible=timeline.points.filter(p=>{
    const v=valueOf(p); return typeof v==="number" && Number.isFinite(v) && v>=0;
  });
  const latest=eligible[eligible.length-1], threshold=latest ? thresholdOf(latest) : null;
  const validThreshold=typeof threshold==="number" && Number.isFinite(threshold) && threshold>0 ? threshold : null;
  const ages=eligible.map(p=>valueOf(p) as number);
  const maxAge=Math.max(0,...ages);
  const minAge=Math.min(...ages);
  const spread=Math.max(1,maxAge-minAge);
  const yMin=zoom && ages.length>=2 ? Math.max(0,minAge-spread*.2) : 0;
  const yMax=zoom && ages.length>=2 ? maxAge+spread*.2 : Math.max(1,maxAge*1.05,(validThreshold??0)*1.05);
  const tMin=eligible[0]?.summary.collection_started_at_ms ?? 0;
  const tMax=latest?.summary.collection_started_at_ms ?? 0;
  const intervalsByLater=new Map(timeline.intervals.map(x=>[x.newer.summary.id,x]));
  const pts=eligible.map((p,index) => {
    const value=valueOf(p) as number, ms=p.summary.collection_started_at_ms;
    const interval=intervalsByLater.get(p.summary.id);
    const sourceIndex=timeline.points.findIndex(row=>row.summary.id===p.summary.id);
    const prior=sourceIndex>0 ? timeline.points[sourceIndex-1] : null;
    return {id:p.summary.id,ms,value,
      x:62+(tMax>tMin?(ms-tMin)/(tMax-tMin):.5)*764,
      y:200-(value-yMin)/(yMax-yMin)*170,
      connectToPrevious:index>0 && prior?.summary.id===eligible[index-1]?.summary.id && interval?.state==="observed"};
  });
  const thresholdY=validThreshold !== null && validThreshold>=yMin && validThreshold<=yMax
    ? 200-(validThreshold-yMin)/(yMax-yMin)*170 : null;
  return { points:pts, yMin, yMax, thresholdY, threshold:validThreshold };
}

export function xidChartSegments(chart: XidChart): XidChartPoint[][] {
  const segments:XidChartPoint[][]=[];
  let segment:XidChartPoint[]=[];
  for (const point of chart.points) {
    if (!point.connectToPrevious && segment.length) {segments.push(segment);segment=[];}
    segment.push(point);
  }
  if(segment.length) segments.push(segment);
  return segments;
}

export interface XidRelationHistoryPoint {
  snapshot_id: number;
  collection_started_at_ms: number;
  current_xid: string;
  postmaster_start_time: string | null;
  in_recovery: boolean | null;
  relation: {
    schema_name: string;
    relation_name: string;
    relation_type: string;
    xid_age: number;
    pct_of_freeze_max: number | null;
    mxid_age: number | null;
    pct_of_multixact_freeze_max: number | null;
    last_vacuum: string | null;
    last_autovacuum: string | null;
  };
}
export interface XidRelationTransition {
  point: XidRelationHistoryPoint;
  xid_delta: number | null;
  mxid_delta: number | null;
  state: "observed" | "unverified";
  xid_horizon_moved: boolean;
  mxid_horizon_moved: boolean;
}
export function xidRelationTransitions(source: readonly XidRelationHistoryPoint[]): XidRelationTransition[] {
  const points=[...source].sort((a,b)=>a.collection_started_at_ms-b.collection_started_at_ms || a.snapshot_id-b.snapshot_id);
  return points.map((point,index)=>{
    const prev=points[index-1];
    const comparable=!!prev && point.collection_started_at_ms>prev.collection_started_at_ms &&
      !!prev.postmaster_start_time && prev.postmaster_start_time===point.postmaster_start_time &&
      prev.in_recovery!=null && prev.in_recovery===point.in_recovery &&
      point.relation.schema_name===prev.relation.schema_name && point.relation.relation_name===prev.relation.relation_name &&
      point.relation.relation_type===prev.relation.relation_type &&
      xidCounter(point.current_xid)!==null && xidCounter(prev.current_xid)!==null &&
      (xidCounter(point.current_xid) as bigint)>=(xidCounter(prev.current_xid) as bigint) &&
      point.collection_started_at_ms-prev.collection_started_at_ms<=6*3_600_000;
    const xid_delta=comparable ? point.relation.xid_age-prev.relation.xid_age : null;
    const mxid_delta=comparable && point.relation.mxid_age!=null && prev.relation.mxid_age!=null
      ? point.relation.mxid_age-prev.relation.mxid_age : null;
    return {point,xid_delta,mxid_delta,state:comparable?"observed":"unverified",
      xid_horizon_moved:xid_delta!==null && xid_delta<0,
      mxid_horizon_moved:mxid_delta!==null && mxid_delta<0};
  });
}
