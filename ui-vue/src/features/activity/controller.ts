import { t } from "../i18n/locale-service.js";
import { emptyPollMetrics, recordPoll, type PollMetrics } from "../../core/poll-metrics.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";

/** Exact JSON field names returned by pgxray_core::activity (archive 2026-11-10). */
export interface ActivityBackend {
  pid: number;
  database: string | null;
  username: string | null;
  application_name: string;
  client_addr: string | null;
  state: string | null;
  backend_start_ms: number | null;
  xact_start_ms: number | null;
  query_start_ms: number | null;
  state_change_ms: number | null;
  query_age_seconds: number | null;
  xact_age_seconds: number | null;
  wait_event_type: string | null;
  wait_event: string | null;
  backend_type: string | null;
  query: string | null;
  is_pgxray: boolean;
}
export interface ActivitySnapshot {
  captured_at_ms: number;
  total_connections: number;
  active_connections: number;
  idle_connections: number;
  idle_in_transaction_connections: number;
  waiting_active_connections: number;
  pgxray_connections: number;
  backends: ActivityBackend[];
}
export interface ActivityConnection { id: number; name: string; database: string; username: string; host: string; port: number }
export interface ActivitySample { total: number; active: number; waiting: number }
export type ActivityFilter = "all" | "active" | "waiting" | "idle-xact";
export type SignalAction = "cancel" | "terminate";
export const ACTIVITY_PAGE_SIZES = [15, 30, 60] as const;
export const ACTIVITY_INTERVALS = [2000, 5000, 10000] as const;
export const ACTIVITY_DURATIONS = [30000, 60000, 300000] as const;
export const MAX_SIGNAL_SNAPSHOT_AGE_MS = 15_000;

export const ACTIVITY_SQL_PREVIEW_MAX_LINES = 1_500;
export const ACTIVITY_SQL_PREVIEW_MAX_CHARS = 128 * 1024;

/** Bound DOM rendering, not the SQL retained in the current PostgreSQL snapshot. */
export function activitySqlPreview(query: string): { text: string; truncated: boolean } {
  let end = Math.min(query.length, ACTIVITY_SQL_PREVIEW_MAX_CHARS);
  let lines = 1;
  for (let index = 0; index < end; index++) {
    if (query.charCodeAt(index) === 10 && ++lines > ACTIVITY_SQL_PREVIEW_MAX_LINES) {
      end = index + 1; // Preserve the newline terminating the last visible line.
      break;
    }
  }
  // Do not cut a UTF-16 surrogate pair in half at the character limit.
  if (end < query.length && end > 0 && query.charCodeAt(end - 1) >= 0xD800 && query.charCodeAt(end - 1) <= 0xDBFF)
    --end;
  return { text: query.slice(0, end), truncated: end < query.length };
}

/** Visual warning only. An active backend can legitimately run for a long time. */
export function activityQueryDurationTier(backend: ActivityBackend): "watch" | "long" | null {
  if (backend.state !== "active" || backend.query_age_seconds === null ||
      !Number.isFinite(backend.query_age_seconds) || backend.query_age_seconds < 60) return null;
  return backend.query_age_seconds >= 300 ? "long" : "watch";
}

function object(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(t('tsActivity.badResponse', { label }));
  return value as Record<string, unknown>;
}
function text(value: unknown, label: string, nullable = false): string | null {
  if (nullable && value === null) return null;
  if (typeof value !== "string") throw new Error(t('tsActivity.badResponse', { label }));
  return value;
}
function number(value: unknown, label: string, nullable = false, integer = false): number | null {
  if (nullable && value === null) return null;
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || (integer && !Number.isSafeInteger(value)))
    throw new Error(t('tsActivity.badResponse', { label }));
  return value;
}
function positiveInt(value: unknown, label: string): number {
  const result = number(value, label, false, true) as number;
  if (result <= 0) throw new Error(t('tsActivity.badResponse', { label }));
  return result;
}
function optionalInt(value: unknown, label: string): number | null {
  return number(value, label, true, true);
}
function optionalSeconds(value: unknown, label: string): number | null {
  if (value === null) return null;
  // PostgreSQL clock adjustments can briefly produce negative elapsed values.
  // The UI clamps their display to zero, as the legacy formatter did.
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(t('tsActivity.badResponse', { label }));
  return value;
}

export function parseActivitySnapshot(raw: unknown): ActivitySnapshot {
  const r = object(raw, "ActivitySnapshot");
  if (!Array.isArray(r.backends)) throw new Error(t('tsActivity.badBackends'));
  const seen = new Set<number>();
  const backends = r.backends.map((value, index): ActivityBackend => {
    const b = object(value, `backend #${index}`);
    const pid = positiveInt(b.pid, "backend.pid");
    if (seen.has(pid)) throw new Error(t('tsActivity.duplicatePid'));
    seen.add(pid);
    if (typeof b.is_pgxray !== "boolean") throw new Error(t('tsActivity.badIsPgxray'));
    return {
      pid,
      database: text(b.database, "database", true),
      username: text(b.username, "username", true),
      application_name: text(b.application_name, "application_name") as string,
      client_addr: text(b.client_addr, "client_addr", true),
      state: text(b.state, "state", true),
      backend_start_ms: optionalInt(b.backend_start_ms, "backend_start_ms"),
      xact_start_ms: optionalInt(b.xact_start_ms, "xact_start_ms"),
      query_start_ms: optionalInt(b.query_start_ms, "query_start_ms"),
      state_change_ms: optionalInt(b.state_change_ms, "state_change_ms"),
      query_age_seconds: optionalSeconds(b.query_age_seconds, "query_age_seconds"),
      xact_age_seconds: optionalSeconds(b.xact_age_seconds, "xact_age_seconds"),
      wait_event_type: text(b.wait_event_type, "wait_event_type", true),
      wait_event: text(b.wait_event, "wait_event", true),
      backend_type: text(b.backend_type, "backend_type", true),
      query: text(b.query, "query", true),
      is_pgxray: b.is_pgxray,
    };
  });
  const result: ActivitySnapshot = {
    captured_at_ms: positiveInt(r.captured_at_ms, "captured_at_ms"),
    total_connections: number(r.total_connections, "total_connections", false, true) as number,
    active_connections: number(r.active_connections, "active_connections", false, true) as number,
    idle_connections: number(r.idle_connections, "idle_connections", false, true) as number,
    idle_in_transaction_connections: number(r.idle_in_transaction_connections, "idle_in_transaction_connections", false, true) as number,
    waiting_active_connections: number(r.waiting_active_connections, "waiting_active_connections", false, true) as number,
    pgxray_connections: number(r.pgxray_connections, "pgxray_connections", false, true) as number,
    backends,
  };
  if (result.total_connections !== backends.length ||
      [result.active_connections, result.idle_connections, result.idle_in_transaction_connections,
        result.waiting_active_connections, result.pgxray_connections].some(count => count > backends.length))
    throw new Error(t('tsActivity.inconsistent'));
  return result;
}

/** Never retain or display a secret if list_connections adds additional fields. */
export function parseActivityConnections(raw: unknown): ActivityConnection[] {
  if (!Array.isArray(raw)) throw new Error(t('tsActivity.badConnections'));
  const ids = new Set<number>();
  return raw.map((item) => {
    const row = object(item, "connection");
    const id = positiveInt(row.id, "connection.id");
    const port = positiveInt(row.port, "connection.port");
    if (port > 65535 || ids.has(id)) throw new Error(t('tsActivity.badProfile'));
    ids.add(id);
    return {
      id, port, name: text(row.name, "connection.name") as string,
      database: text(row.database, "connection.database") as string,
      username: text(row.username, "connection.username") as string,
      host: text(row.host, "connection.host") as string,
    };
  }).sort((a, b) => a.name.localeCompare(b.name, "ru"));
}

export function activityPage(
  snapshot: ActivitySnapshot | null,
  options: { filter: ActivityFilter; search: string; hideIdle: boolean; page: number; pageSize: number },
) {
  const needle = options.search.trim().toLocaleLowerCase("ru");
  const rows = (snapshot?.backends ?? []).filter(b => {
    if (options.hideIdle && b.state === "idle") return false;
    if (options.filter === "active" && b.state !== "active") return false;
    if (options.filter === "waiting" && !(b.state === "active" && b.wait_event_type !== null)) return false;
    if (options.filter === "idle-xact" && !b.state?.startsWith("idle in transaction")) return false;
    // Do not include potentially secret SQL text in automatic search results.
    return !needle || [b.pid, b.database, b.username, b.application_name,
      b.client_addr, b.state, b.wait_event_type, b.wait_event].some(value => String(value ?? "").toLocaleLowerCase("ru").includes(needle));
  });
  const pages = Math.max(1, Math.ceil(rows.length / options.pageSize));
  const page = Number.isSafeInteger(options.page) ? Math.max(1, Math.min(options.page, pages)) : 1;
  return { rows: rows.slice((page - 1) * options.pageSize, page * options.pageSize),
    total: rows.length, page, pages };
}

export function sampleSummary(samples: readonly ActivitySample[]) {
  return {
    count: samples.length,
    averageActive: samples.length ? samples.reduce((n, s) => n + s.active, 0) / samples.length : null,
    maxActive: samples.length ? Math.max(...samples.map(s => s.active)) : null,
    maxWaiting: samples.length ? Math.max(...samples.map(s => s.waiting)) : null,
    maxConnections: samples.length ? Math.max(...samples.map(s => s.total)) : null,
  };
}

export interface ActivityState {
  connections: ActivityConnection[];
  selectedConnectionId: number | null;
  snapshot: ActivitySnapshot | null;
  snapshotConnectionId: number | null;
  samples: ActivitySample[];
  pollMetrics: PollMetrics;
  monitoring: boolean;
  loading: boolean;
  connectionsLoading: boolean;
  signaling: boolean;
  monitorStartedAt: number | null;
  monitorEndedAt: number | null;
  error: string | null;
  notice: string | null;
}
export function createActivityState(): ActivityState {
  return { connections: [], selectedConnectionId: null, snapshot: null, snapshotConnectionId: null,
    samples: [], pollMetrics: emptyPollMetrics(), monitoring: false, loading: false, connectionsLoading: false, signaling: false,
    monitorStartedAt: null, monitorEndedAt: null, error: null, notice: null };
}
/** The keep-alive view may mount before login's busy flag is released. */
export function canBootstrapActivityConnections(
  shell: Pick<ShellState, "screen" | "currentUsername" | "busy">,
  pending: boolean,
): boolean {
  return pending && shell.screen === "workspace" && shell.currentUsername !== null && !shell.busy;
}

export interface ActivityDependencies {
  now?: () => number;
  setTimer?: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>;
  clearTimer?: (id: ReturnType<typeof setTimeout>) => void;
}

/** Only Vue state is reset locally; all authorization and PostgreSQL privileges are enforced by Rust. */
export function createActivityController(
  shell: ShellState, state: ActivityState, invoke: PgXrayInvoke, deps: ActivityDependencies = {},
) {
  const now = deps.now ?? Date.now;
  const setTimer = deps.setTimer ?? setTimeout;
  const clearTimer = deps.clearTimer ?? clearTimeout;
  let revision = 0;
  let connectionLoad = 0;
  let requestSeq = 0;
  let inFlight = false;
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let durationDeadline: number | null = null;
  let intervalMs = 5000;
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingTerminate: { pid: number; backendStartMs: number; connectionId: number } | null = null;
  function clearNoticeTimer(): void {
    if (noticeTimer !== null) clearTimer(noticeTimer);
    noticeTimer = null;
  }
  function temporaryNotice(message: string): void {
    clearNoticeTimer();
    state.notice = message;
    const auth = shell.authGeneration, rev = revision;
    noticeTimer = setTimer(() => {
      noticeTimer = null;
      if (valid(auth, rev) && state.notice === message) state.notice = null;
    }, 10_000);
  }

  const active = () => !disposed && shell.screen === "workspace" && shell.currentUsername !== null;
  const valid = (auth: number, rev: number) => active() && shell.authGeneration === auth && revision === rev;
  function cancelTimer() {
    if (timer !== null) clearTimer(timer);
    timer = null;
  }
  function stop(silent = false): void {
    cancelTimer();
    clearNoticeTimer();
    pendingTerminate = null;
    ++revision; // A pending IPC response must never resurrect a stopped monitor.
    state.loading = inFlight;
    if (state.monitoring) state.monitorEndedAt = now();
    state.monitoring = false;
    durationDeadline = null;
    if (!silent) state.notice = t('tsActivity.stopped');
  }
  function reset(): void {
    stop(true);
    ++connectionLoad;
    ++requestSeq;
    Object.assign(state, createActivityState());
  }
  function dispose(): void { reset(); disposed = true; }
  function changeConnection(id: number | null): void {
    if (state.monitoring || state.signaling || state.selectedConnectionId === id) return;
    if (id !== null && !state.connections.some(c => c.id === id)) id = null;
    ++revision;
    clearNoticeTimer();
    pendingTerminate = null;
    ++requestSeq;
    state.loading = inFlight;
    state.selectedConnectionId = id;
    state.snapshotConnectionId = null;
    state.snapshot = null;
    state.samples = [];
    state.pollMetrics = emptyPollMetrics();
    state.monitorStartedAt = null;
    state.monitorEndedAt = null;
    state.notice = null;
    state.error = null;
  }

  async function refreshConnections(): Promise<boolean> {
    if (!active() || shell.busy || state.connectionsLoading || state.monitoring || state.signaling) return false;
    const auth = shell.authGeneration, rev = revision, seq = ++connectionLoad;
    state.connectionsLoading = true;
    try {
      const result = parseActivityConnections(await invoke("list_connections"));
      if (!valid(auth, rev) || seq !== connectionLoad) return false;
      // A profile (including its password) might have changed without changing its ID.
      // Invalidate the snapshot every time the connection list is refreshed.
      const wanted = state.selectedConnectionId;
      state.connections = result;
      state.selectedConnectionId = wanted !== null && result.some(c => c.id === wanted)
        ? wanted : (result[0]?.id ?? null);
      ++requestSeq; // Any pending snapshot of this profile is now obsolete.
      state.loading = inFlight;
      state.snapshot = null;
      state.snapshotConnectionId = null;
      state.samples = [];
      state.pollMetrics = emptyPollMetrics();
      clearNoticeTimer(); pendingTerminate = null;
      state.error = null;
      return true;
    } catch (error) {
      if (valid(auth, rev) && seq === connectionLoad) state.error = t('tsActivity.connectionsFailed', { error: String(error) });
      return false;
    } finally {
      if (valid(auth, rev) && seq === connectionLoad) state.connectionsLoading = false;
    }
  }

  async function refresh(recordSample = false): Promise<boolean> {
    if (!active() || shell.busy || inFlight || state.signaling || state.selectedConnectionId === null ||
        (state.monitoring && !recordSample)) return false;
    const connectionId = state.selectedConnectionId;
    const auth = shell.authGeneration, rev = revision, seq = ++requestSeq, requestedAt = now();
    inFlight = true;
    state.loading = true;
    state.error = null;
    try {
      const snapshot = parseActivitySnapshot(await invoke("postgres_activity_snapshot", { connectionId }));
      if (!valid(auth, rev) || seq !== requestSeq || state.selectedConnectionId !== connectionId) return false;
      recordPoll(state.pollMetrics, true, now() - requestedAt);
      state.snapshot = snapshot;
      state.snapshotConnectionId = connectionId;
      if (pendingTerminate && pendingTerminate.connectionId === connectionId) {
        const pending = pendingTerminate;
        // The same PID with a different start time is a different backend.
        if (!snapshot.backends.some(b => b.pid === pending.pid && b.backend_start_ms === pending.backendStartMs)) {
          pendingTerminate = null;
          temporaryNotice(t('tsActivity.terminateConfirmed', { pid: String(pending.pid) }));
        }
      }
      if (recordSample) state.samples.push({ total: snapshot.total_connections,
        active: snapshot.active_connections, waiting: snapshot.waiting_active_connections });
      return true;
    } catch (error) {
      if (valid(auth, rev) && seq === requestSeq) { recordPoll(state.pollMetrics, false, now() - requestedAt); state.error = t('tsActivity.snapshotFailed', { error: String(error) }); }
      return false;
    } finally {
      inFlight = false;
      state.loading = false;
    }
  }

  async function tick(): Promise<void> {
    if (!state.monitoring || !active()) return;
    const auth = shell.authGeneration, rev = revision, connectionId = state.selectedConnectionId;
    if (state.signaling) {
      // A signal operation should not race with an automatic refresh.
      timer = setTimer(() => { void tick(); }, intervalMs);
      return;
    }
    await refresh(true);
    if (!valid(auth, rev) || !state.monitoring || state.selectedConnectionId !== connectionId) return;
    if (durationDeadline !== null && now() >= durationDeadline) {
      stop(true);
      state.notice = t('tsActivity.monitorDone', { count: String(state.samples.length) });
      return;
    }
    timer = setTimer(() => { void tick(); }, intervalMs);
  }
  function start(interval: number, duration: number): boolean {
    if (!active() || shell.busy || state.monitoring || state.signaling || inFlight || state.selectedConnectionId === null ||
      !ACTIVITY_INTERVALS.includes(interval as (typeof ACTIVITY_INTERVALS)[number]) ||
      !ACTIVITY_DURATIONS.includes(duration as (typeof ACTIVITY_DURATIONS)[number])) return false;
    cancelTimer();
    ++revision;
    state.monitoring = true;
    state.samples = [];
    state.pollMetrics = emptyPollMetrics();
    state.error = null;
    state.notice = null;
    state.monitorStartedAt = now();
    state.monitorEndedAt = null;
    intervalMs = interval;
    durationDeadline = now() + duration;
    void tick();
    return true;
  }

  function canSignal(backend: ActivityBackend, action: SignalAction): boolean {
    const snapshot = state.snapshot;
    if (!active() || state.signaling || state.loading || shell.busy ||
      !snapshot || state.snapshotConnectionId !== state.selectedConnectionId ||
      backend.backend_start_ms === null || backend.backend_start_ms <= 0 ||
      (backend.backend_type !== null && backend.backend_type !== "client backend") ||
      (action === "cancel" && backend.state !== "active")) return false;
    const age = now() - snapshot.captured_at_ms;
    if (age < 0 || age > MAX_SIGNAL_SNAPSHOT_AGE_MS) return false;
    return snapshot.backends.some(b => b.pid === backend.pid && b.backend_start_ms === backend.backend_start_ms);
  }
  async function signal(backend: ActivityBackend, action: SignalAction, confirm: (message: string) => boolean | Promise<boolean>): Promise<boolean> {
    if (!canSignal(backend, action)) {
      state.error = t('tsActivity.staleSnapshot');
      return false;
    }
    const snapshot = state.snapshot;
    const connectionId = state.selectedConnectionId;
    const rev = revision, auth = shell.authGeneration;
    if (!snapshot || connectionId === null || backend.backend_start_ms === null) return false;
    const warning = action === "terminate"
      ? t('tsActivity.terminateWarning')
      : t('tsActivity.cancelWarning');
    const question = t('tsActivity.confirmSignal', {
      action: action === "terminate" ? t('tsActivity.terminateTitle') : t('tsActivity.cancelTitle'),
      pid: String(backend.pid),
      connection: String(state.connections.find(c => c.id === connectionId)?.name ?? connectionId),
      role: backend.username ?? "—", database: backend.database ?? "—",
      application: backend.application_name || "—", warning,
      pgxray: backend.is_pgxray ? t('tsActivity.pgxrayWarning') : "",
    });
    if (!await confirm(question)) return false;
    // Do not let a delayed confirmation target a new snapshot / user / server.
    if (!valid(auth, rev) || state.snapshot !== snapshot || state.selectedConnectionId !== connectionId ||
      !canSignal(backend, action)) {
      state.error = t('tsActivity.targetChanged');
      return false;
    }
    const backendStartMs = backend.backend_start_ms;
    state.signaling = true;
    state.error = null;
    state.notice = null;
    let signaled = false;
    try {
      const raw = object(await invoke("signal_postgres_backend", { connectionId, pid: backend.pid,
        backendStartMs, action }), "backend signal");
      if (typeof raw.signaled !== "boolean" || raw.pid !== backend.pid || raw.backend_start_ms !== backendStartMs || raw.action !== action)
        throw new Error(t('tsActivity.badSignal'));
      if (valid(auth, rev) && state.selectedConnectionId === connectionId) {
        signaled = raw.signaled;
        if (signaled && action === "terminate") {
          pendingTerminate = { pid: backend.pid, backendStartMs, connectionId };
          temporaryNotice(t('tsActivity.terminateSent', { pid: String(backend.pid) }));
        } else {
          pendingTerminate = null;
          temporaryNotice(signaled
            ? t('tsActivity.cancelSent', { pid: String(backend.pid) })
            : t('tsActivity.signalNotSent', { pid: String(backend.pid) }));
        }
      }
    } catch (error) {
      if (valid(auth, rev)) state.error = t('tsActivity.signalFailed', { error: String(error) });
    } finally {
      state.signaling = false;
      // Never enable a second signal using a pre-signal snapshot.
      if (valid(auth, rev) && state.selectedConnectionId === connectionId) {
        state.snapshot = null;
        state.snapshotConnectionId = null;
      }
    }
    if (valid(auth, rev) && state.selectedConnectionId === connectionId) {
      // A subsequent accepted snapshot confirms whether the targeted backend disappeared.
      await refresh(state.monitoring);
    }
    return signaled;
  }
  return { reset, dispose, changeConnection, refreshConnections, refresh, start, stop, canSignal, signal };
}
