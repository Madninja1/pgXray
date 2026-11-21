import { t } from "../i18n/locale-service.js";
/** Presentation-only helpers; never change execution order or persisted schedules. */
import type { ScheduleMode, SnapshotSchedule } from "./schedule-model.js";

export type ScheduleListSort = "connection" | "mode" | "nextAt";
export type ScheduleListDirection = "asc" | "desc";
export const SCHEDULE_MODE_LABELS: Record<ScheduleMode, string> = {
  interval: t('tsFinish.schedulePeriodic'), daily: t('tsFinish.scheduleDaily'), series: t('tsFinish.scheduleSeries'),
};
const MODE_ORDER: readonly ScheduleMode[] = ["interval", "daily", "series"];

/** Paused/finished rules have no upcoming attempt even if nextAt retains an old timestamp. */
export function upcomingAttempt(rule: SnapshotSchedule): number | null {
  return rule.enabled && rule.status === "active" && Number.isFinite(rule.nextAt) && rule.nextAt > 0
    ? rule.nextAt : null;
}

export function sortedVisibleSchedules(
  rules: readonly SnapshotSchedule[], connectionIds: readonly number[] | null,
  sortBy: ScheduleListSort, direction: ScheduleListDirection,
): SnapshotSchedule[] {
  const selected = connectionIds === null ? null : new Set(connectionIds);
  const sign = direction === "desc" ? -1 : 1;
  const byConnection = (a: SnapshotSchedule, b: SnapshotSchedule) =>
    a.targetName.localeCompare(b.targetName, "ru") || a.connectionId - b.connectionId;
  const byMode = (a: SnapshotSchedule, b: SnapshotSchedule) =>
    MODE_ORDER.indexOf(a.mode) - MODE_ORDER.indexOf(b.mode);
  return rules.filter(rule => selected === null || selected.has(rule.connectionId)).sort((a, b) => {
    let primary: number;
    if (sortBy === "nextAt") {
      const left = upcomingAttempt(a), right = upcomingAttempt(b);
      // No upcoming attempt always goes last, in either direction.
      if (left === null || right === null) primary = left === right ? 0 : left === null ? 1 : -1;
      else primary = (left - right) * sign;
    } else {
      primary = (sortBy === "connection" ? byConnection(a, b) : byMode(a, b)) * sign;
    }
    return primary || byConnection(a, b) || byMode(a, b) ||
      a.kind.localeCompare(b.kind) || a.key.localeCompare(b.key);
  });
}

export interface ScheduleListGroup {
  mode: ScheduleMode | null;
  label: string;
  rules: SnapshotSchedule[];
}
export function scheduleListGroups(rules: SnapshotSchedule[], grouped: boolean,
  reverseModes = false): ScheduleListGroup[] {
  if (!grouped) return [{mode: null, label: "", rules}];
  const modes = reverseModes ? [...MODE_ORDER].reverse() : MODE_ORDER;
  return modes.map(mode => ({mode, label: SCHEDULE_MODE_LABELS[mode],
    rules: rules.filter(rule => rule.mode === mode)})).filter(group => group.rules.length > 0);
}
