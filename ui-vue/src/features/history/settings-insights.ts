import { t } from "../i18n/locale-service.js";
/** Settings history has no exact change timestamp, only an observation window. */
export interface SettingsHistoryChange {
  name: string; category: string; state: "added" | "removed" | "changed" | string;
  older_value: string | null; newer_value: string | null;
  older_unit: string | null; newer_unit: string | null;
  older_source: string | null; newer_source: string | null;
  older_sourcefile: string | null; newer_sourcefile: string | null;
  older_sourceline: number | null; newer_sourceline: number | null;
  older_pending_restart: boolean | null; newer_pending_restart: boolean | null;
  value_changed: boolean; reset_value_changed: boolean;
  source_changed: boolean; pending_restart_changed: boolean;
}
export interface SettingsHistoryInterval {
  from_snapshot_id: number; to_snapshot_id: number;
  from_ms: number; to_ms: number;
  state: "comparable" | "server_restarted" | "target_changed" | "unsupported_schema" | "comparison_unavailable" | string;
  reason: string | null;
  value_change_count: number; source_change_count: number; pending_restart_change_count: number;
  changes: SettingsHistoryChange[];
}
export interface SettingsHistoryAnalysis {
  snapshot_count: number;
  intervals: SettingsHistoryInterval[];
  limitations: string[];
}
export type SettingsChangeFilter = "all" | "value" | "source" | "restart" | "added";
export function settingsIntervalLabel(state: string): string {
  switch (state) {
    case "comparable": return t('tsFinish.settingsComparable');
    case "server_restarted": return t('tsFinish.settingsRestart');
    case "target_changed": return t('tsFinish.settingsTargetChanged');
    case "unsupported_schema": return t('tsFinish.settingsUnsupportedSchema');
    case "comparison_unavailable": return t('tsFinish.settingsCompareUnavailable');
    default: return t('tsFinish.settingsUnconfirmed');
  }
}
export function settingsIntervalIsComparable(interval: SettingsHistoryInterval | null): boolean {
  return interval !== null && (interval.state === "comparable" || interval.state === "server_restarted");
}
export function settingsChanges(interval: SettingsHistoryInterval | null, filter: SettingsChangeFilter, search = ""): SettingsHistoryChange[] {
  if (!interval || !settingsIntervalIsComparable(interval)) return [];
  const term = search.trim().toLocaleLowerCase();
  return interval.changes.filter(change => {
    if (filter === "value" && !change.value_changed) return false;
    if (filter === "source" && !change.source_changed) return false;
    if (filter === "restart" && !(change.pending_restart_changed || change.newer_pending_restart === true)) return false;
    if (filter === "added" && change.state !== "added" && change.state !== "removed") return false;
    return `${change.name} ${change.category} ${change.older_source ?? ""} ${change.newer_source ?? ""}`.toLocaleLowerCase().includes(term);
  }).sort((a,b) => Number(b.newer_pending_restart === true) - Number(a.newer_pending_restart === true) ||
    Number(b.pending_restart_changed) - Number(a.pending_restart_changed) || a.name.localeCompare(b.name, "ru"));
}
export function settingsParameterHistory(data: SettingsHistoryAnalysis, name: string): { interval: SettingsHistoryInterval; change: SettingsHistoryChange }[] {
  return data.intervals.flatMap(interval => settingsIntervalIsComparable(interval) ? interval.changes
    .filter(change => change.name === name).map(change => ({interval,change})) : []);
}
export function settingsValue(value: string | null, unit: string | null, revealed: boolean): string {
  if (!revealed) return "••••";
  return value === null ? "—" : `${value}${unit ? ` ${unit}` : ""}`;
}
