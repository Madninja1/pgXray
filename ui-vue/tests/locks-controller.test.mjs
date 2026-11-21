import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { createShellState, enterShellWorkspace } from "../dist/core/shell-state.js";
import { advanceIncident, createLocksController, createLocksState, parseLockCapture, sameLockIncidentEpisode } from "../dist/features/locks/controller.js";

const connection = id => ({ id, name: `db-${id}`, host: "localhost", port: 5432, database: "postgres", username: "tester" });
const backend = (pid = 10) => ({ pid, username: "tester", database: "postgres", application_name: "worker", state: "active", backend_start_ms: 123, backend_type: "client backend", query_age_seconds: 2, query: "select 1", is_pgxray: false });
const capture = (at = 1000, blocked = 1, age = 200, depth = 1) => ({ snapshot: {
  captured_at_ms: at, measured_at: "2026-11-11T10:00:00Z", database: "postgres", server_version: "16", server_version_num: 160000, database_oid: 5,
  source: { connection_id: 1, host: "localhost", port: 5432, username: "tester", sslmode: "disable" },
  summary: { blocked_backends: blocked, direct_blockers: blocked, root_blockers: blocked, idle_in_transaction_roots: 0, longest_chain_depth: depth },
  backends: [backend()], edges: [], waits: blocked ? [{ blocked_pid: 10, immediate_blocker_pids: [20], root_blocker_pids: [20], max_depth: depth,
    paths: [{ pids: [10, 20], cycle_detected: false, incomplete: false, prepared_transaction: false }] }] : [],
  locks: blocked ? [{ pid: 10, lock_type: "transactionid", mode: "ShareLock", granted: false, wait_start_ms: at - age, schema_name: null, relation_name: null }] : [],
  lock_count_total: blocked ? 1 : 0, limitations: [], paths_truncated: false,
}, activity_snapshot: null, activity_error: null });
function deferred() { let resolve; const promise = new Promise(ok => { resolve = ok; }); return { resolve, promise }; }
function setup(invokeRaw = async cmd => cmd === "list_connections" ? [connection(1), connection(2)] : capture()) {
  const shell = createShellState(); enterShellWorkspace(shell, "owner");
  const state = createLocksState(), timers = [];
  const controller = createLocksController(shell, state, invokeRaw, { now: () => 1100,
    setTimer: (fn, ms) => { const t = { fn, ms, cancelled: false }; timers.push(t); return t; },
    clearTimer: t => { t.cancelled = true; } });
  return { shell, state, controller, timers };
}
test("parsing preserves full raw evidence and distinguishes prepared paths", () => {
  const raw = capture(); raw.snapshot.waits[0].paths[0] = { pids: [10, 0], cycle_detected: false, prepared_transaction: true };
  const parsed = parseLockCapture(raw);
  assert.equal(parsed.rawSnapshot, raw.snapshot);
  assert.equal(parsed.snapshot.waits[0].paths[0].prepared_transaction, true);
  assert.throws(() => parseLockCapture({ ...raw, snapshot: { ...raw.snapshot, lock_count_total: -1 } }), /целое/);
});
test("incident tracks first/last observation, best capture and maximum observed wait", () => {
  let incident = advanceIncident(null, parseLockCapture(capture(1000, 1, 100)), 1);
  incident = advanceIncident(incident, parseLockCapture(capture(2000, 1, 500, 2)), 1);
  incident = advanceIncident(incident, parseLockCapture(capture(3000, 0)), 1);
  assert.equal(incident.startedAt, 1000);
  assert.equal(incident.endedAt, 3000);
  assert.equal(incident.longestWaitMs, 500);
  assert.equal(incident.best.snapshot.captured_at_ms, 2000);
});
test("saving evidence remains valid when a live capture updates the same lock episode", () => {
  const original = advanceIncident(null, parseLockCapture(capture(1000, 1, 100)), 1);
  const updated = advanceIncident(original, parseLockCapture(capture(2000, 1, 500, 2)), 1);
  assert.notEqual(updated, original, "the monitor replaces the object each tick");
  assert.equal(sameLockIncidentEpisode(original, updated), true);
  const ended = advanceIncident(updated, parseLockCapture(capture(3000, 0)), 1);
  assert.equal(sameLockIncidentEpisode(original, ended), true, "a finished lock episode can still be saved");
  const nextEpisode = advanceIncident(ended, parseLockCapture(capture(4000, 1, 100)), 1);
  assert.equal(sameLockIncidentEpisode(original, nextEpisode), false, "a new episode must be reconfirmed");
  assert.equal(sameLockIncidentEpisode(original, { ...updated, connectionId: 2 }), false);
  assert.equal(sameLockIncidentEpisode(original, null), false);
});
test("stale response cannot replace snapshot after changing connection", async () => {
  const wait = deferred(); const { controller, state } = setup(cmd => cmd === "list_connections" ? [connection(1), connection(2)] : wait.promise);
  await controller.refreshConnections();
  const task = controller.refresh();
  controller.changeConnection(2);
  wait.resolve(capture());
  assert.equal(await task, false);
  assert.equal(state.snapshot, null);
  assert.equal(state.connectionId, 2);
});
test("stopping while a request is pending does not schedule an old loop", async () => {
  const wait = deferred(); const { controller, timers, state } = setup(cmd => cmd === "list_connections" ? [connection(1)] : wait.promise);
  await controller.refreshConnections();
  assert.equal(controller.start(2000), true);
  controller.stop();
  wait.resolve(capture());
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(state.monitoring, false);
  assert.equal(timers.length, 0);
});
test("monitor continues without a visible property and stops on logout reset", async () => {
  const { controller, state, shell, timers } = setup();
  await controller.refreshConnections();
  controller.start(5000);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(state.monitoring, true);
  assert.equal(timers.length, 1);
  shell.authGeneration += 1;
  controller.reset();
  assert.equal(state.monitoring, false);
  assert.equal(timers[0].cancelled, true);
});

test("Locks parses idle transactions without treating them as blocking paths", () => {
  const raw = capture(2000, 0);
  raw.snapshot.idle_xact_total = 1;
  raw.snapshot.idle_xacts = [{ pid: 99, database: "postgres", username: "tester", application_name: "worker", xact_age_seconds: 145, idle_age_seconds: 100, client_addr: "127.0.0.1", backend_xmin: "735", backend_xid: null, is_blocking: false }];
  const parsed = parseLockCapture(raw);
  assert.equal(parsed.snapshot.summary.blocked_backends, 0);
  assert.equal(parsed.snapshot.idle_xact_total, 1);
  assert.equal(parsed.snapshot.idle_xacts[0].pid, 99);
  assert.equal(parsed.snapshot.idle_xacts[0].is_blocking, false);
  assert.equal(parsed.snapshot.idle_xacts[0].idle_age_seconds, 100);
  assert.equal(parsed.snapshot.idle_xacts[0].backend_xmin, "735");
});
test("Locks capture statistics count successful calls and failures", async () => {
  let fail = false;
  const { controller, state } = setup(async cmd => {
    if (cmd === "list_connections") return [connection(1)];
    if (fail) throw Error("database offline");
    return capture();
  });
  await controller.refreshConnections();
  await controller.refresh();
  fail = true;
  await controller.refresh();
  assert.equal(state.pollMetrics.requests, 2);
  assert.equal(state.pollMetrics.succeeded, 1);
  assert.equal(state.pollMetrics.failed, 1);
});
