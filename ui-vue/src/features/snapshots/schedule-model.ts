import { t } from "../i18n/locale-service.js";
/** Scheduling policy for saved diagnostic snapshots. No PostgreSQL writes. */
export const SNAPSHOT_KINDS = ["xid", "pgss", "autovacuum", "pg_settings", "io_wal", "index_health"] as const;
export type SnapshotKind = (typeof SNAPSHOT_KINDS)[number];
export const KIND_LABELS: Record<SnapshotKind, string> = {
  xid: "XID / Wraparound", pgss: "SQL workload / PGSS", autovacuum: "Autovacuum",
  pg_settings: "Configuration Drift", io_wal: "I/O & WAL", index_health: "Index Health",
};
export const SNAPSHOT_COMMANDS: Record<SnapshotKind, string> = {
  xid: "collect_xid_snapshot", pgss: "collect_pgss_snapshot", autovacuum: "collect_autovacuum_snapshot",
  pg_settings: "collect_pg_settings_snapshot", io_wal: "collect_io_wal_snapshot", index_health: "collect_index_health_snapshot",
};
export type ScheduleMode = "interval" | "daily" | "series";
export type ScheduleStatus = "active" | "complete" | "expired" | "paused";
export interface SavedTarget {
  id: number; name: string; host: string; port: number; database: string; username: string; sslmode: string;
}
export interface ScheduleDraft {
  connectionId: number;
  kind: SnapshotKind;
  mode: ScheduleMode;
  intervalMinutes: number;
  windowStart: string;
  windowEnd: string;
  /** First wall-clock slot of each permitted window; later slots are anchored to it, not to IPC completion. */
  intervalAnchor: string;
  /** JavaScript local weekday: Sunday=0, Monday=1, ... Saturday=6. Overnight windows belong to their start day. */
  daysOfWeek: number[];
  dailyTime: string;
  seriesCount: number;
  seriesSpanMinutes: number;
}
export interface SnapshotSchedule extends ScheduleDraft {
  key: string;
  fingerprint: string;
  targetName: string;
  enabled: boolean;
  status: ScheduleStatus;
  createdAt: number;
  nextAt: number;
  seriesStartedAt: number | null;
  seriesCompleted: number;
  lastAttemptAt: number | null;
  lastSuccessAt: number | null;
  lastDurationMs: number | null;
  lastError: string | null;
}
/** Translate known scheduler messages only when displayed; persisted values remain unchanged. */
export function displayScheduleError(message: string): string {
  switch (message) {
    case "Предыдущий сбор не подтверждён. Проверь History и возобнови вручную.": return t('tsFinish.schedulePrevious');
    case "Серия не завершена в отведённое время.": return t('tsFinish.scheduleExpired');
    case "Профиль подключения удалён или изменился.": return t('tsFinish.scheduleProfileChanged');
    case "Совпадающий слот: серия не завершена.": return t('tsFinish.scheduleOverlapSeries');
    case "Совпадающий слот другого расписания: повторный сбор пропущен.": return t('tsFinish.scheduleOverlap');
    case "После обновления расписания проверь первый слот и дни недели, затем нажми «Продолжить».": return t('tsFinish.scheduleLegacy');
    default: return message; // Preserve external and historical errors verbatim.
  }
}
export const SNAPSHOT_CADENCES = [15, 30, 60, 120, 360, 720, 1440] as const;
export const ALL_WEEKDAYS = [0, 1, 2, 3, 4, 5, 6] as const;
export const MIN_SERIES_GAP_MS = 15 * 60_000;
/** Heavy collectors are deliberately throttled more strictly than XID/settings. */
export const MIN_INTERVAL_BY_KIND: Record<SnapshotKind, number> = {
  xid: 15, pgss: 30, autovacuum: 15, pg_settings: 15, io_wal: 15, index_health: 60,
};
export const SERIES_GRACE_MS = 10 * 60_000;
/** A late WebView wakeup must not turn a missed 09:05 into a 09:17 collection. */
export const SLOT_GRACE_MS = 90_000;
export const CHECK_EVERY_MS = 30_000;
export const CONNECTION_RETRY_MS = 2 * 60_000;

export function snapshotTargetKey(c: SavedTarget): string {
  // Name and passwords are deliberately excluded. A change of server, role, DB
  // or TLS mode invalidates the schedule instead of targeting another database.
  return JSON.stringify([c.host, c.port, c.database, c.username, c.sslmode]);
}
/** Legacy keys are kept readable so previously saved rules survive upgrades. */
export function scheduleKey(connectionId: number, kind: SnapshotKind): string {
  return `${connectionId}:${kind}`;
}
/** New rules get independent IDs; no collision when two are saved in one millisecond. */
export function newScheduleKey(connectionId: number, kind: SnapshotKind, now: number,
  existingKeys: ReadonlySet<string>): string {
  const prefix = `${scheduleKey(connectionId, kind)}:${now}:`;
  let sequence = 0;
  while (existingKeys.has(`${prefix}${sequence}`)) sequence++;
  return `${prefix}${sequence}`;
}
export function validClock(value: string): boolean {
  return typeof value === "string" && /^(?:[01]\d|2[0-3]):[0-5]\d$/.test(value);
}
export function clockMinute(value: string): number {
  if (!validClock(value)) throw new Error(t('tsScheduleModel.badTime'));
  const [hours, minutes] = value.split(":").map(Number);
  return hours * 60 + minutes;
}
export function inCaptureWindow(now: number, start: string, end: string): boolean {
  const from = clockMinute(start), to = clockMinute(end);
  if (from === to) return true; // equal clocks = 24-hour window
  const date = new Date(now);
  const minute = date.getHours() * 60 + date.getMinutes();
  return from < to ? minute >= from && minute < to : minute >= from || minute < to;
}
export function nextWindowStart(now: number, start: string): number {
  const minute = clockMinute(start);
  const date = new Date(now);
  date.setHours(Math.floor(minute / 60), minute % 60, 0, 0);
  if (date.getTime() <= now) date.setDate(date.getDate() + 1);
  return date.getTime();
}
function validWeekdays(days: number[]): boolean {
  return Array.isArray(days) && days.length > 0 && days.length <= 7 &&
    days.every(day => Number.isInteger(day) && day >= 0 && day <= 6) &&
    new Set(days).size === days.length;
}
/** Next local wall-clock interval slot. `inclusive` permits a slot exactly at `after`.
 *
 * Only slots from the configured anchor until the *exclusive* window end are
 * generated. No interval is computed from the last actual attempt, so neither
 * slow SQL nor a closed WebView causes schedule drift or catch-up bursts.
 */
export function nextIntervalSlot(after: number, draft: Pick<ScheduleDraft,
  "intervalMinutes" | "windowStart" | "windowEnd" | "intervalAnchor" | "daysOfWeek">,
  inclusive = false): number {
  const from = clockMinute(draft.windowStart);
  const to = clockMinute(draft.windowEnd);
  const anchor = clockMinute(draft.intervalAnchor);
  const fullDay = from === to;
  const end = fullDay ? 1440 : from < to ? to : to + 1440;
  if (!Number.isSafeInteger(draft.intervalMinutes) || draft.intervalMinutes <= 0 ||
      !validWeekdays(draft.daysOfWeek) ||
      (!fullDay && (anchor < from || anchor >= end)))
    throw new Error(t('tsScheduleModel.badCadence'));
  // For overnight windows the first slot belongs to the start calendar day.
  // Anchors before midnight are required to avoid ambiguous day attribution.
  if (from > to && anchor < from) throw new Error(t('tsScheduleModel.badOvernightAnchor'));
  const base = new Date(after);
  base.setHours(0, 0, 0, 0);
  for (let dayOffset = -1; dayOffset <= 8; dayOffset++) {
    const day = new Date(base.getFullYear(), base.getMonth(), base.getDate() + dayOffset);
    if (!draft.daysOfWeek.includes(day.getDay())) continue;
    for (let minute = anchor; minute < end; minute += draft.intervalMinutes) {
      const slot = new Date(day.getFullYear(), day.getMonth(), day.getDate(),
        Math.floor(minute / 60), minute % 60, 0, 0);
      // Skip nonexistent wall-clock minutes on DST spring-forward transitions.
      if (slot.getHours() !== Math.floor((minute % 1440) / 60) ||
          slot.getMinutes() !== minute % 60) continue;
      const timestamp = slot.getTime();
      if (inclusive ? timestamp >= after : timestamp > after) return timestamp;
    }
  }
  throw new Error(t('tsScheduleModel.nextSlotMissing'));
}
export function isIntervalSlot(rule: SnapshotSchedule, timestamp: number): boolean {
  return nextIntervalSlot(timestamp, rule, true) === timestamp;
}
export function nextDaily(now: number, time: string, days: readonly number[] = ALL_WEEKDAYS): number {
  const minute = clockMinute(time);
  if (!validWeekdays([...days])) throw new Error(t('tsScheduleModel.chooseDay'));
  const start = new Date(now);
  for (let offset = 0; offset <= 7; offset++) {
    const date = new Date(start.getFullYear(), start.getMonth(), start.getDate() + offset,
      Math.floor(minute / 60), minute % 60, 0, 0);
    if (date.getHours() !== Math.floor(minute / 60) || date.getMinutes() !== minute % 60) continue;
    if (days.includes(date.getDay()) && date.getTime() > now) return date.getTime();
  }
  throw new Error(t('tsScheduleModel.dailyRunMissing'));
}
export function scheduleDue(schedule: SnapshotSchedule, now: number): boolean {
  return schedule.enabled && schedule.status === "active" && now >= schedule.nextAt &&
    (schedule.mode !== "interval" || (now <= schedule.nextAt + SLOT_GRACE_MS &&
      inCaptureWindow(now, schedule.windowStart, schedule.windowEnd)));
}
export function seriesDeadline(schedule: SnapshotSchedule): number {
  return (schedule.seriesStartedAt ?? schedule.createdAt) + schedule.seriesSpanMinutes * 60_000;
}
export function seriesExpired(schedule: SnapshotSchedule, now: number): boolean {
  return schedule.mode === "series" && schedule.status === "active" &&
    (now > seriesDeadline(schedule) + SERIES_GRACE_MS || now > schedule.nextAt + SERIES_GRACE_MS);
}
export function seriesNextAt(schedule: SnapshotSchedule): number {
  const start = schedule.seriesStartedAt ?? schedule.createdAt;
  return start + Math.round(schedule.seriesCompleted * schedule.seriesSpanMinutes * 60_000 /
    (schedule.seriesCount - 1));
}
export function validateSchedule(draft: ScheduleDraft): void {
  if (!Number.isSafeInteger(draft.connectionId) || draft.connectionId <= 0 ||
      !SNAPSHOT_KINDS.includes(draft.kind) || !["interval", "daily", "series"].includes(draft.mode))
    throw new Error(t('tsScheduleModel.badDraft'));
  if (!validWeekdays(draft.daysOfWeek)) throw new Error(t('tsScheduleModel.chooseValidDay'));
  if (draft.mode === "interval") {
    if (!SNAPSHOT_CADENCES.includes(draft.intervalMinutes as typeof SNAPSHOT_CADENCES[number]) ||
        !validClock(draft.windowStart) || !validClock(draft.windowEnd) ||
        !validClock(draft.intervalAnchor) ||
        draft.intervalMinutes < MIN_INTERVAL_BY_KIND[draft.kind])
      throw new Error(t('tsScheduleModel.badWindow'));
    const from = clockMinute(draft.windowStart), to = clockMinute(draft.windowEnd);
    const anchor = clockMinute(draft.intervalAnchor);
    if (from !== to && (anchor < from || (from < to && anchor >= to)))
      throw new Error(t('tsScheduleModel.firstSlotOutside'));
  }
  if (draft.mode === "daily" && !validClock(draft.dailyTime))
    throw new Error(t('tsScheduleModel.badDailyTime'));
  if (draft.mode === "series" &&
      (!Number.isSafeInteger(draft.seriesCount) || draft.seriesCount < 2 || draft.seriesCount > 10 ||
       !Number.isSafeInteger(draft.seriesSpanMinutes) || draft.seriesSpanMinutes > 1440 ||
       draft.seriesSpanMinutes * 60_000 < (draft.seriesCount - 1) *
         Math.max(MIN_SERIES_GAP_MS, MIN_INTERVAL_BY_KIND[draft.kind] * 60_000)))
    throw new Error(t('tsScheduleModel.seriesLimits', { minutes: String(MIN_INTERVAL_BY_KIND[draft.kind]) }));
}
export function makeSchedule(draft: ScheduleDraft, target: SavedTarget, now: number): SnapshotSchedule {
  validateSchedule(draft);
  if (draft.connectionId !== target.id) throw new Error(t('tsScheduleModel.targetMismatch'));
  return {
    ...draft, daysOfWeek: [...draft.daysOfWeek],
    key: scheduleKey(draft.connectionId, draft.kind), fingerprint: snapshotTargetKey(target),
    targetName: target.name, enabled: true, status: "active", createdAt: now,
    nextAt: draft.mode === "daily" ? nextDaily(now, draft.dailyTime, draft.daysOfWeek) :
      draft.mode === "interval" ? nextIntervalSlot(now, draft, true) : now,
    seriesStartedAt: draft.mode === "series" ? now : null, seriesCompleted: 0,
    lastAttemptAt: null, lastSuccessAt: null, lastDurationMs: null, lastError: null,
  };
}
/** Upgrade v1 persisted plans without the new calendar fields. Existing XID etc.
 * switch from drifting `last attempt + period` to fixed slots at windowStart.
 * Interval rules are paused for review: the original floating phase cannot
 * be reconstructed safely from the old format. No automatic extra SQL is run.
 */
export function upgradeSavedSchedule(raw: unknown, now: number): unknown {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return raw;
  const rule = raw as Partial<SnapshotSchedule>;
  if (typeof rule.intervalAnchor === "string" && Array.isArray(rule.daysOfWeek)) return rule;
  const upgraded: SnapshotSchedule = { ...rule,
    intervalAnchor: rule.intervalAnchor ?? rule.windowStart ?? "09:00", daysOfWeek: [...ALL_WEEKDAYS],
  } as SnapshotSchedule;
  if (upgraded.mode === "interval") {
    try { upgraded.nextAt = nextIntervalSlot(now, upgraded, true); } catch { return raw; }
    if (upgraded.enabled && upgraded.status === "active") {
      upgraded.enabled = false;
      upgraded.status = "paused";
      upgraded.lastError = "После обновления расписания проверь первый слот и дни недели, затем нажми «Продолжить».";
    }
  }
  return upgraded;
}
export function isValidSavedSchedule(raw: unknown): raw is SnapshotSchedule {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return false;
  const r = raw as Partial<SnapshotSchedule>;
  try {
    validateSchedule(r as ScheduleDraft);
    const baseKey = scheduleKey(r.connectionId!, r.kind!);
    // Accept old `id:kind` plans and new `id:kind:timestamp:sequence` plans.
    const validKey = typeof r.key === "string" &&
      (r.key === baseKey || new RegExp(`^${baseKey}:[0-9]{1,16}:[0-9]{1,3}$`).test(r.key));
    return validKey && typeof r.fingerprint === "string" &&
      r.fingerprint.length <= 2048 && typeof r.targetName === "string" && r.targetName.length < 512 &&
      typeof r.enabled === "boolean" && ["active", "complete", "expired", "paused"].includes(r.status!) &&
      [r.createdAt, r.nextAt, r.seriesCompleted].every(v => typeof v === "number" && Number.isSafeInteger(v) && v >= 0) &&
      (r.seriesCompleted! <= r.seriesCount!) &&
      (r.seriesStartedAt === null || (typeof r.seriesStartedAt === "number" && Number.isSafeInteger(r.seriesStartedAt))) &&
      (r.lastAttemptAt === null || (typeof r.lastAttemptAt === "number" && Number.isSafeInteger(r.lastAttemptAt))) &&
      (r.lastSuccessAt === null || (typeof r.lastSuccessAt === "number" && Number.isSafeInteger(r.lastSuccessAt))) &&
      (r.lastDurationMs === null || (typeof r.lastDurationMs === "number" && Number.isSafeInteger(r.lastDurationMs))) &&
      (r.lastError === null || (typeof r.lastError === "string" && r.lastError.length <= 1000));
  } catch { return false; }
}
