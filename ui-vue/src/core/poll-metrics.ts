/** Client-side timing of snapshot IPC calls, not a PostgreSQL server query counter. */
export interface PollMetrics {
  requests: number;
  succeeded: number;
  failed: number;
  totalMs: number;
  maxMs: number;
  lastMs: number | null;
}
export function emptyPollMetrics(): PollMetrics {
  return { requests: 0, succeeded: 0, failed: 0, totalMs: 0, maxMs: 0, lastMs: null };
}
export function recordPoll(metrics: PollMetrics, succeeded: boolean, elapsedMs: number): void {
  const elapsed = Math.max(0, Number.isFinite(elapsedMs) ? elapsedMs : 0);
  ++metrics.requests;
  if (succeeded) ++metrics.succeeded;
  else ++metrics.failed;
  metrics.totalMs += elapsed;
  metrics.maxMs = Math.max(metrics.maxMs, elapsed);
  metrics.lastMs = elapsed;
}
