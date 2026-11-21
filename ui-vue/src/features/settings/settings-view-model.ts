import { t } from "../i18n/locale-service.js";
import { HISTORY_PRESET_DAYS, type HistoryRow, type HistorySectionCounts, type ReadinessCheck } from "./extra-controller.js";

/** Keep the original retention presets while validating custom values. */
export function retentionSelection(days: number | null): { choice: string; custom: string } {
  if (days === null) return { choice: "forever", custom: "" };
  if (HISTORY_PRESET_DAYS.includes(days as (typeof HISTORY_PRESET_DAYS)[number])) {
    return { choice: String(days), custom: "" };
  }
  return { choice: "custom", custom: String(days) };
}

export function retentionDays(choice: string, custom: string): number | null | undefined {
  if (choice === "forever") return null;
  const text = choice === "custom" ? custom.trim() : choice;
  if (!/^\d+$/.test(text)) return undefined;
  const days = Number(text);
  if (!Number.isSafeInteger(days) || days < 1 || days > 36500) return undefined;
  if (choice !== "custom" && !HISTORY_PRESET_DAYS.includes(days as (typeof HISTORY_PRESET_DAYS)[number])) return undefined;
  return days;
}

/** The readiness selector needs only public connection metadata, never secrets. */
export function readinessConnections(raw: unknown): { id: number; name: string }[] {
  if (!Array.isArray(raw)) throw new Error(t('settingsTs.invalidReadinessConnections'));
  const ids = new Set<number>();
  return raw.map((item) => {
    if (!item || typeof item !== "object" || Array.isArray(item)) throw new Error(t('settingsTs.invalidReadinessConnection'));
    const value = item as Record<string, unknown>;
    if (typeof value.id !== "number" || !Number.isSafeInteger(value.id) || value.id <= 0 ||
        typeof value.name !== "string" || ids.has(value.id)) throw new Error(t('settingsTs.invalidReadinessConnection'));
    ids.add(value.id);
    return { id: value.id, name: value.name };
  });
}

/** Rust reports readiness as ready/limited/unavailable, not only ok/warning/error.
 * Unknown future statuses stay visible without being misclassified as success.
 */
export function readinessCheckStatus(check: ReadinessCheck): "ok" | "warning" | "error" | "other" {
  switch (check.status.toLowerCase()) {
    case "ready": case "ok": case "pass": case "passed": return "ok";
    case "limited": case "unavailable": case "unverified": case "warning": case "warn": return "warning";
    case "unsupported": case "error": case "failed": case "fail": return "error";
    default: return "other";
  }
}

export function readinessStatusLabel(status: string): string {
  switch (status.toLowerCase()) {
    case "ready": case "ok": case "pass": case "passed": return t('settingsTs.ready');
    case "limited": return t('settingsTs.limited');
    case "unavailable": return t('settingsTs.unavailable');
    case "unverified": return t('settingsTs.unverified');
    case "warning": case "warn": return t('settingsTs.warning');
    case "unsupported": return t('settingsTs.unsupported');
    case "error": case "failed": case "fail": return t('settingsTs.error');
    default: return status;
  }
}

export function readinessSupportLabel(status: string): string {
  switch (status.toLowerCase()) {
    case "supported": return t('settingsTs.versionSupported');
    case "unsupported": return t('settingsTs.versionUnsupported');
    case "unverified": return t('settingsTs.versionUnverified');
    default: return status;
  }
}

export function readinessOverview(checks: readonly ReadinessCheck[]): {
  total: number; ok: number; warning: number; error: number; other: number;
} {
  const summary = { total: checks.length, ok: 0, warning: 0, error: 0, other: 0 };
  for (const check of checks) summary[readinessCheckStatus(check)] += 1;
  return summary;
}

/** Keep a fast local refresh visibly acknowledged without delaying IPC itself. */
export function remainingRefreshFeedbackMs(startedAt: number, finishedAt: number): number {
  const elapsed = Math.max(0, finishedAt - startedAt);
  return Math.max(0, 450 - elapsed);
}

/** list_history_connections returns ONE summary per connection, not snapshots. */
export const HISTORY_SECTIONS = [
  { key: "xid", label: "XID / Freeze" },
  { key: "pgss", label: "PGSS" },
  { key: "autovacuum", label: "Autovacuum" },
  { key: "pg_settings", label: "PG Settings" },
  { key: "io_wal", label: "I/O & WAL" },
  { key: "index_health", label: "Index Health" },
] as const;

export function historySectionTotals(rows: readonly HistoryRow[]): HistorySectionCounts | null {
  // Never display a misleading partial total when an older backend lacks the breakdown.
  if (!rows.length || rows.some(row => !row.section_counts)) return null;
  const totals: HistorySectionCounts = {
    xid: 0, pgss: 0, autovacuum: 0, pg_settings: 0, io_wal: 0, index_health: 0,
  };
  for (const row of rows) {
    for (const section of HISTORY_SECTIONS) totals[section.key] += row.section_counts![section.key];
  }
  return totals;
}

export const HISTORY_PAGE_SIZES = [10, 25, 50] as const;
export type HistoryPageSize = (typeof HISTORY_PAGE_SIZES)[number];
export interface HistoryOverviewPage {
  rows: HistoryRow[];
  totalConnections: number;
  totalSnapshots: number;
  totalBySection: HistorySectionCounts | null;
  page: number;
  pageCount: number;
  from: number;
  to: number;
}
export function historyOverviewPage(
  rows: readonly HistoryRow[], selectedConnection: string, requestedPage: number,
  pageSize: HistoryPageSize,
): HistoryOverviewPage {
  const selected = selectedConnection === "all" ? null : Number(selectedConnection);
  const filtered = rows.filter(row => selected === null || row.connection_id === selected);
  // The Rust query already orders by newest snapshot; make this invariant local
  // so mock data and later API implementations do not change the UI order.
  const ordered = [...filtered].sort((a, b) => b.newest_at_ms - a.newest_at_ms || b.connection_id - a.connection_id);
  const totalConnections = ordered.length;
  const pageCount = Math.max(1, Math.ceil(totalConnections / pageSize));
  const page = Number.isSafeInteger(requestedPage) ? Math.min(Math.max(1, requestedPage), pageCount) : 1;
  const start = (page - 1) * pageSize;
  return {
    rows: ordered.slice(start, start + pageSize), totalConnections,
    totalSnapshots: ordered.reduce((count, row) => count + row.snapshot_count, 0),
    totalBySection: historySectionTotals(ordered),
    page, pageCount,
    from: totalConnections ? start + 1 : 0,
    to: Math.min(start + pageSize, totalConnections),
  };
}
