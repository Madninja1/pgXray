import { t } from "../features/i18n/locale-service.js";
/** In-memory UI registry: reports ongoing work, but never starts or owns it. */
export interface BackgroundOperation {
  id: "activity" | "locks";
  title: string;
  connection: string;
  startedAt: number;
  polls?: { requests: number; succeeded: number; failed: number; lastMs: number | null; maxMs: number; totalMs: number };
}
export function durationText(startedAt: number, now: number): string {
  const elapsed = Math.max(0, Math.floor((now - startedAt) / 1000));
  const hours = Math.floor(elapsed / 3600);
  const minutes = Math.floor((elapsed % 3600) / 60);
  const seconds = elapsed % 60;
  return hours > 0 ? t('tsFinish.elapsedHours', { hours: String(hours), minutes: String(minutes) }) : t('tsFinish.elapsedMinutes', { minutes: String(minutes), seconds: String(seconds) });
}

/** UI-only aggregate of completed IPC snapshot calls across active monitors. */
export function aggregatePolls(operations: readonly BackgroundOperation[]) {
  return operations.reduce((total, operation) => {
    const p = operation.polls;
    if (!p) return total;
    total.requests += p.requests;
    total.succeeded += p.succeeded;
    total.failed += p.failed;
    total.totalMs += p.totalMs;
    total.maxMs = Math.max(total.maxMs, p.maxMs);
    return total;
  }, { requests: 0, succeeded: 0, failed: 0, totalMs: 0, maxMs: 0 });
}
