import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from "../dist/core/shell-state.js";
import {
  MAX_SIGNAL_SNAPSHOT_AGE_MS, ACTIVITY_SQL_PREVIEW_MAX_CHARS, ACTIVITY_SQL_PREVIEW_MAX_LINES,
  activityPage, activityQueryDurationTier, activitySqlPreview, canBootstrapActivityConnections,
  createActivityController, createActivityState,
  parseActivityConnections, parseActivitySnapshot, sampleSummary,
} from "../dist/features/activity/controller.js";

const connection = (id = 1) => ({ id, name: `test-${id}`, host: "127.0.0.1", port: 5432,
  database: "postgres", username: "reader", has_password: true, sslmode: "disable" });
const backend = (override = {}) => ({
  pid: 123, database: "postgres", username: "reader", application_name: "api", client_addr: "127.0.0.1",
  state: "active", backend_start_ms: 1000, xact_start_ms: null, query_start_ms: 2000, state_change_ms: 2000,
  query_age_seconds: 2, xact_age_seconds: null, wait_event_type: "Lock", wait_event: "transactionid",
  backend_type: "client backend", query: "select 'secret-password'", is_pgxray: false, ...override,
});
const snapshot = (captured_at_ms = 1_000_000, items = [backend()]) => ({
  captured_at_ms, total_connections: items.length,
  active_connections: items.filter(b => b.state === "active").length,
  idle_connections: items.filter(b => b.state === "idle").length,
  idle_in_transaction_connections: items.filter(b => b.state?.startsWith("idle in transaction")).length,
  waiting_active_connections: items.filter(b => b.state === "active" && b.wait_event_type !== null).length,
  pgxray_connections: items.filter(b => b.is_pgxray).length,
  backends: items,
});
function deferred() {
  let resolve, reject;
  const promise = new Promise((ok, err) => { resolve = ok; reject = err; });
  return { promise, resolve, reject };
}
function setup(invokeRaw = async (cmd) => cmd === "list_connections" ? [connection()] : snapshot(), opts = {}) {
  const shell = createShellState(); enterShellWorkspace(shell, "owner");
  const state = createActivityState();
  const calls = [];
  const now = opts.now ?? (() => 1_000_100);
  const timers = [];
  const invoke = async (cmd, args) => { calls.push({ cmd, args }); return invokeRaw(cmd, args); };
  const controller = createActivityController(shell, state, invoke, {
    now,
    setTimer: (fn, delay) => { const t = { fn, delay, cancelled: false }; timers.push(t); return t; },
    clearTimer: (timer) => { timer.cancelled = true; },
  });
  return { shell, state, calls, controller, timers };
}

test("parser accepts exact Rust snapshot and no fallback for invalid counters", () => {
  assert.equal(parseActivitySnapshot(snapshot()).backends[0].query, "select 'secret-password'");
  assert.throws(() => parseActivitySnapshot({ ...snapshot(), total_connections: 2 }), /Неконсистентные/);
  assert.throws(() => parseActivitySnapshot(snapshot(1_000_000, [backend(), backend()])), /Повторяющийся PID/);
  assert.throws(() => parseActivitySnapshot(snapshot(1_000_000, [{ ...backend(), backend_start_ms: "1000" }])), /backend_start_ms/);
});
test("connections discard any password/secret properties from IPC response", () => {
  assert.deepEqual(parseActivityConnections([{ ...connection(), password: "secret", token: "private" }]),
    [{ id: 1, port: 5432, name: "test-1", database: "postgres", username: "reader", host: "127.0.0.1" }]);
});
test("filtered paginated table never searches inside SQL text", () => {
  const data = parseActivitySnapshot(snapshot(1_000_000, [backend(), backend({ pid: 456, state: "idle", wait_event_type: null })]));
  assert.equal(activityPage(data, { filter: "all", search: "secret-password", hideIdle: false, page: 1, pageSize: 15 }).total, 0);
  assert.equal(activityPage(data, { filter: "waiting", search: "", hideIdle: false, page: 1, pageSize: 15 }).total, 1);
  assert.equal(activityPage(data, { filter: "all", search: "", hideIdle: true, page: 1, pageSize: 15 }).total, 1);
  assert.equal(activityPage(data, { filter: "all", search: "", hideIdle: false, page: 500, pageSize: 1 }).page, 2);
});
test("monitor statistics describe only accepted samples", () => {
  assert.equal(sampleSummary([]).averageActive, null);
  assert.deepEqual(sampleSummary([{total:3, active:2, waiting:1},{total:4, active:0, waiting:0}]),
    { count: 2, averageActive: 1, maxActive: 2, maxWaiting: 1, maxConnections: 4 });
});
test("refresh binds snapshot to selected connection ID", async () => {
  const { state, calls, controller } = setup();
  assert.equal(await controller.refreshConnections(), true);
  assert.equal(await controller.refresh(), true);
  assert.equal(state.snapshotConnectionId, 1);
  assert.deepEqual(calls.slice(0, 2), [{cmd:"list_connections",args:undefined},
    {cmd:"postgres_activity_snapshot",args:{connectionId:1}}]);
});
test("delayed snapshot is rejected after switching connections", async () => {
  const gate = deferred();
  const { state, controller } = setup(async (cmd) => cmd === "list_connections" ? [connection(1), connection(2)] : gate.promise);
  await controller.refreshConnections();
  const task = controller.refresh();
  controller.changeConnection(2);
  gate.resolve(snapshot());
  assert.equal(await task, false);
  assert.equal(state.snapshot, null);
  assert.equal(state.selectedConnectionId, 2);
});
test("refreshing the connection list invalidates an in-flight snapshot even if ID is unchanged", async () => {
  const gate = deferred();
  const { state, controller } = setup(async (cmd) => cmd === "list_connections" ? [connection()] : gate.promise);
  await controller.refreshConnections();
  const task = controller.refresh();
  await controller.refreshConnections();
  gate.resolve(snapshot());
  assert.equal(await task, false);
  assert.equal(state.snapshot, null);
  assert.equal(state.loading, false);
});
test("logout invalidates pending response and prevents stale data", async () => {
  const gate = deferred();
  const { state, shell, controller } = setup(async (cmd) => cmd === "list_connections" ? [connection()] : gate.promise);
  await controller.refreshConnections();
  const pending = controller.refresh();
  leaveShellWorkspace(shell);
  controller.reset();
  gate.resolve(snapshot());
  assert.equal(await pending, false);
  assert.equal(state.snapshot, null);
  assert.equal(state.connections.length, 0);
});
test("stop during a pending monitor fetch ignores the late response", async () => {
  const gate = deferred();
  const { state, controller, timers } = setup(async cmd => cmd === "list_connections" ? [connection()] : gate.promise);
  await controller.refreshConnections();
  assert.equal(controller.start(2000, 30000), true);
  controller.stop();
  gate.resolve(snapshot());
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(state.snapshot, null);
  assert.equal(state.samples.length, 0);
  assert.equal(timers.filter(t => !t.cancelled).length, 0);
  assert.equal(state.loading, false);
});
test("monitor samples accepted response and schedules next interval", async () => {
  const { state, controller, timers } = setup();
  await controller.refreshConnections();
  assert.equal(controller.start(2000,30000), true);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(state.samples.length, 1);
  assert.equal(timers[0].delay, 2000);
  controller.stop();
  assert.equal(timers[0].cancelled, true);
});
test("signal requires fresh snapshot with same backend identity", async () => {
  let time = 1_000_100;
  const { state, controller, calls } = setup(async cmd => cmd === "list_connections" ? [connection()] : snapshot(), { now: () => time });
  await controller.refreshConnections();
  await controller.refresh();
  assert.equal(controller.canSignal(state.snapshot.backends[0], "cancel"), true);
  time = 1_000_000 + MAX_SIGNAL_SNAPSHOT_AGE_MS + 1;
  assert.equal(controller.canSignal(state.snapshot.backends[0], "terminate"), false);
  assert.equal(await controller.signal(state.snapshot.backends[0], "terminate", () => true), false);
  assert.equal(calls.some(c => c.cmd === "signal_postgres_backend"), false);
});
test("cancel is disabled for idle backend and unknown backend_start", async () => {
  const { state, controller } = setup(async cmd => cmd === "list_connections" ? [connection()] : snapshot(1_000_000,
    [backend({state:"idle",wait_event_type:null})]));
  await controller.refreshConnections(); await controller.refresh();
  assert.equal(controller.canSignal(state.snapshot.backends[0], "cancel"), false);
  assert.equal(controller.canSignal(state.snapshot.backends[0], "terminate"), true);
  state.snapshot.backends[0].backend_start_ms = null;
  assert.equal(controller.canSignal(state.snapshot.backends[0], "terminate"), false);
});
test("confirmation of an old backend cannot signal a newly selected server", async () => {
  const gate = deferred();
  const { state, controller, calls } = setup(async cmd => cmd === "list_connections" ? [connection(1),connection(2)] : snapshot());
  await controller.refreshConnections(); await controller.refresh();
  const pending = controller.signal(state.snapshot.backends[0], "terminate", () => gate.promise);
  controller.changeConnection(2);
  gate.resolve(true);
  assert.equal(await pending, false);
  assert.equal(calls.some(c => c.cmd === "signal_postgres_backend"), false);
});
test("confirmed backend signal uses exact Rust command and then refreshes, never sends SQL text", async () => {
  const { state, controller, calls } = setup(async (cmd, args) => cmd === "list_connections" ? [connection()] :
    cmd === "signal_postgres_backend" ? { pid: args.pid, backend_start_ms: args.backendStartMs, action: args.action, signaled: true } : snapshot());
  await controller.refreshConnections(); await controller.refresh();
  const result = await controller.signal(state.snapshot.backends[0], "cancel", () => true);
  assert.equal(result, true);
  assert.deepEqual(calls.find(c => c.cmd === "signal_postgres_backend"), {
    cmd:"signal_postgres_backend",args:{connectionId:1,pid:123,backendStartMs:1000,action:"cancel"}});
  assert.match(state.notice, /отмена SQL пока не подтверждена/);
});
test("declined signal does not invoke backend", async () => {
  const { state, controller, calls } = setup();
  await controller.refreshConnections(); await controller.refresh();
  assert.equal(await controller.signal(state.snapshot.backends[0], "terminate", () => false), false);
  assert.equal(calls.filter(c => c.cmd === "signal_postgres_backend").length, 0);
});
test("disposing controller invalidates subsequent pending responses", async () => {
  const gate = deferred();
  const { state, controller } = setup(async cmd => cmd === "list_connections" ? [connection()] : gate.promise);
  await controller.refreshConnections();
  const pending = controller.refresh();
  controller.dispose(); gate.resolve(snapshot());
  assert.equal(await pending, false);
  assert.equal(state.snapshot, null);
});
test("backend identity cannot change while an asynchronous confirmation is open", async () => {
  const gate = deferred();
  const { state, controller, calls } = setup();
  await controller.refreshConnections(); await controller.refresh();
  const pending = controller.signal(state.snapshot.backends[0], "terminate", () => gate.promise);
  await controller.refreshConnections();
  await controller.refresh();
  gate.resolve(true);
  assert.equal(await pending, false);
  assert.equal(calls.some(c => c.cmd === "signal_postgres_backend"), false);
});
test("snapshot for a PID without a matching backend_start is not a valid signal target", async () => {
  const { state, controller } = setup();
  await controller.refreshConnections(); await controller.refresh();
  assert.equal(controller.canSignal({ ...state.snapshot.backends[0], backend_start_ms: 3000 }, "terminate"), false);
});
test("snapshot parser tolerates a negative age caused by PostgreSQL clock correction", () => {
  const result = parseActivitySnapshot(snapshot(1_000_000, [backend({ query_age_seconds: -0.2 })]));
  assert.equal(result.backends[0].query_age_seconds, -0.2);
});
test("logout-in-progress prevents additional Activity IPC requests", async () => {
  const { shell, state, controller, calls } = setup();
  shell.busy = true;
  assert.equal(await controller.refreshConnections(), false);
  assert.equal(controller.start(2000,30000), false);
  assert.equal(state.snapshot, null);
  assert.equal(calls.length, 0);
});

test("long-query highlighting uses active query age, never idle or arbitrary waits", () => {
  assert.equal(activityQueryDurationTier(backend({ query_age_seconds: 59 })), null);
  assert.equal(activityQueryDurationTier(backend({ query_age_seconds: 60 })), "watch");
  assert.equal(activityQueryDurationTier(backend({ query_age_seconds: 299 })), "watch");
  assert.equal(activityQueryDurationTier(backend({ query_age_seconds: 300 })), "long");
  assert.equal(activityQueryDurationTier(backend({ state: "idle", query_age_seconds: 900 })), null);
  assert.equal(activityQueryDurationTier(backend({ state: "active", query_age_seconds: null, wait_event_type: "Lock" })), null);
  assert.equal(activityQueryDurationTier(backend({ state: "active", query_age_seconds: 900, wait_event_type: "Lock" })), "long");
});

test("SQL preview preserves all server-returned text when it fits limits", () => {
  const text = "SELECT 'кириллица 👋';\nSELECT 2;";
  assert.deepEqual(activitySqlPreview(text), { text, truncated: false });
});
test("SQL preview bounds huge single-line queries without breaking surrogate pairs", () => {
  const query = "x".repeat(ACTIVITY_SQL_PREVIEW_MAX_CHARS - 1) + "👋" + "tail";
  const result = activitySqlPreview(query);
  assert.equal(result.truncated, true);
  assert.equal(result.text, "x".repeat(ACTIVITY_SQL_PREVIEW_MAX_CHARS - 1));
});
test("SQL preview bounds line count without modifying source query", () => {
  const query = Array.from({length: ACTIVITY_SQL_PREVIEW_MAX_LINES + 2}, (_, i) => `SELECT ${i};`).join("\n");
  const original = query;
  const result = activitySqlPreview(query);
  assert.equal(result.truncated, true);
  assert.equal(result.text.split("\n").filter(Boolean).length, ACTIVITY_SQL_PREVIEW_MAX_LINES);
  assert.equal(query, original);
});

test("monitor continues collecting samples while navigating to Sessions and back", async () => {
  const { shell, state, controller, timers } = setup();
  await controller.refreshConnections();
  assert.equal(controller.start(2000, 30000), true);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(state.samples.length, 1);
  shell.mainView = "sessions";
  assert.equal(state.monitoring, true);
  await timers[0].fn();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(state.samples.length, 2);
  shell.mainView = "activity";
  assert.equal(state.monitoring, true);
  controller.stop();
});


test("activity connection bootstrap waits for login to release busy", async () => {
  const shell = createShellState();
  shell.busy = true; // Login calls enterShellWorkspace before its finally block clears busy.
  enterShellWorkspace(shell, "owner");
  const { state, controller } = setup();
  assert.equal(canBootstrapActivityConnections(shell, true), false);
  shell.busy = false;
  assert.equal(canBootstrapActivityConnections(shell, true), true);
  assert.equal(canBootstrapActivityConnections(shell, false), false);
  assert.equal(await controller.refreshConnections(), true);
  assert.equal(state.connections.length, 1);
  leaveShellWorkspace(shell);
  assert.equal(canBootstrapActivityConnections(shell, true), false);
});

test("terminate notice is confirmed by a new snapshot and expires", async () => {
  let removed = false;
  const { controller, state, timers } = setup(async (cmd, args) => {
    if (cmd === "list_connections") return [connection()];
    if (cmd === "signal_postgres_backend") { removed = true; return { pid: args.pid, backend_start_ms: args.backendStartMs, action: args.action, signaled: true }; }
    return snapshot(1_000_000, removed ? [] : [backend()]);
  });
  await controller.refreshConnections(); await controller.refresh();
  assert.equal(await controller.signal(state.snapshot.backends[0], "terminate", () => true), true);
  assert.match(state.notice, /подтверждено новым Activity snapshot/);
  const expiry = timers.find(t => t.delay === 10_000 && !t.cancelled);
  assert.ok(expiry);
  expiry.fn();
  assert.equal(state.notice, null);
});
test("Activity poll statistics measure accepted responses and errors", async () => {
  let fail = false;
  const { state, controller } = setup(async cmd => {
    if (cmd === "list_connections") return [connection()];
    if (fail) throw Error("offline");
    return snapshot();
  });
  await controller.refreshConnections();
  await controller.refresh();
  fail = true;
  await controller.refresh();
  assert.equal(state.pollMetrics.requests, 2);
  assert.equal(state.pollMetrics.succeeded, 1);
  assert.equal(state.pollMetrics.failed, 1);
});
