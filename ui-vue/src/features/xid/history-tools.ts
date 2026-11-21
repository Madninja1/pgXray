import type { XidComparison, XidSummary } from "./controller.js";

export interface XidPlotPoint { id: number; at: number; databaseY: number; oldestY: number | null; x: number }
/** Scatter rather than line: summaries cannot prove continuous server epoch after restart/failover. */
export function xidHistoryScatter(history: readonly XidSummary[], limit = 40): {
  points: XidPlotPoint[]; minAge: number | null; maxAge: number | null;
  firstAt: number | null; lastAt: number | null;
} {
  const samples = history.filter(s => Number.isFinite(s.collection_started_at_ms) &&
    Number.isFinite(s.database_xid_age) && s.database_xid_age >= 0)
    .sort((a,b) => a.collection_started_at_ms - b.collection_started_at_ms)
    .slice(-Math.max(1, limit));
  if (!samples.length) return { points: [], minAge: null, maxAge: null, firstAt: null, lastAt: null };
  const ages = samples.flatMap(s => [s.database_xid_age,
    ...(s.oldest_relation && Number.isFinite(s.oldest_relation.xid_age) && s.oldest_relation.xid_age >= 0
      ? [s.oldest_relation.xid_age] : [])]);
  const minAge = Math.min(...ages), maxAge = Math.max(...ages);
  const firstAt = samples[0].collection_started_at_ms;
  const lastAt = samples[samples.length - 1].collection_started_at_ms;
  const x = (at: number) => 24 + (lastAt === firstAt ? .5 : (at - firstAt) / (lastAt - firstAt)) * 652;
  const y = (age: number) => 155 - (maxAge === minAge ? .5 : (age - minAge) / (maxAge - minAge)) * 130;
  return { points: samples.map(s => ({ id:s.id, at:s.collection_started_at_ms,
    databaseY:y(s.database_xid_age),
    oldestY:s.oldest_relation && Number.isFinite(s.oldest_relation.xid_age) && s.oldest_relation.xid_age >= 0
      ? y(s.oldest_relation.xid_age) : null,
    x:x(s.collection_started_at_ms),
  })), minAge, maxAge, firstAt, lastAt };
}

export type XidChangeFilter = "all" | "changed" | "increased" | "decreased" | "sample";
export type XidRelationChange = XidComparison["relation_changes"][number];
export function xidFilterChanges(rows: readonly XidRelationChange[], filter: XidChangeFilter): XidRelationChange[] {
  if (filter === "all") return [...rows];
  return rows.filter(row => {
    const sample = row.sample_state !== "present_in_both";
    if (filter === "sample") return sample;
    const delta = row.xid_age_delta;
    const known = typeof delta === "number" && Number.isFinite(delta);
    if (filter === "increased") return known && delta > 0;
    if (filter === "decreased") return known && delta < 0;
    return sample || known && delta !== 0;
  });
}

/** Escapes delimiters and spreadsheet formula prefixes on user-controlled strings. */
export function xidCsvCell(value: string | number | null | undefined): string {
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : "";
  const text = value == null ? "" : String(value);
  const safe = /^\s*[=+@-]/.test(text) ? `'${text}` : text;
  return `"${safe.replace(/"/g,'""')}"`;
}
export function xidCsv(headers: readonly string[], rows: readonly (readonly (string | number | null | undefined)[])[]): string {
  return [headers, ...rows].map(row => row.map(xidCsvCell).join(';')).join('\r\n') + '\r\n';
}
