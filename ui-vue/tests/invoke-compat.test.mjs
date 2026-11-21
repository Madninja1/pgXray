import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import {
  createPgXrayInvoke,
  LOGGED_COMMANDS,
  safeInvokeContext,
} from "../dist/api/invoke.js";

const originalSource = readFileSync(
  new URL("../../ui/js/invoke.js", import.meta.url), "utf8",
);

function legacyBridge(raw, now = () => 123) {
  const window = { __TAURI__: { core: { invoke: raw } } };
  const context = vm.createContext({ window, performance: { now }, Date });
  vm.runInContext(originalSource, context, { filename: "ui/js/invoke.js" });
  return window.pgxrayInvoke;
}

// The UI log is deliberately fire-and-forget. Allow its microtasks to settle.
const flushLogs = () => new Promise((resolve) => setImmediate(resolve));

function captureRaw(result = { ok: true }) {
  const calls = [];
  const raw = async (command, args) => {
    calls.push({ command, args });
    if (command === "application_log_event") return undefined;
    return result;
  };
  return { raw, calls };
}

function normalizedLogEntries(calls) {
  return calls
    .filter(({ command }) => command === "application_log_event")
    .map(({ args }) => ({
      operation: args.operation,
      phase: args.phase,
      durationMs: args.durationMs,
      context: args.context,
      error: args.error,
      requestIdPresent: typeof args.requestId === "string" && args.requestId.length > 0,
    }));
}

test("logging command allow-list still matches the original", () => {
  const sourceSet = originalSource.match(/const loggedCommands = new Set\(\[([\s\S]*?)\]\);/);
  assert.ok(sourceSet, "legacy command allow-list must be readable");
  const originalNames = [...sourceSet[1].matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
  // Existing commands retain legacy ordering; project operations are new IPC calls.
  const newCommands = new Set([
    "list_connection_projects", "create_connection_project", "rename_connection_project",
    "delete_connection_project", "assign_connection_project", "assign_connection_shortcut", "export_xid_csv",
    "load_pgss_snapshot", "compare_pgss_snapshots", "pgss_statement_history", "pgss_workload_history",
    "collect_io_wal_snapshot", "io_wal_workload_history", "collect_index_health_snapshot", "autovacuum_workload_history", "pg_settings_history_analysis",
    "list_workspace_changes", "link_workspace_change_to_case",
  ]);
  assert.deepEqual([...LOGGED_COMMANDS].filter(command => !newCommands.has(command)), originalNames);
  for (const command of newCommands) assert.equal(LOGGED_COMMANDS.has(command), true);
});

test("missing Tauri invoke keeps bridge unavailable", () => {
  assert.equal(createPgXrayInvoke(null), null);
  assert.equal(createPgXrayInvoke(undefined), null);
});

test("unlogged calls keep their arguments and results, without extra events", async () => {
  const expected = { needs_registration: true, session: null };
  const { raw, calls } = captureRaw(expected);
  const bridge = createPgXrayInvoke(raw);
  const args = { token: "example" };
  assert.equal(await bridge("auth_state", args), expected);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "auth_state");
  assert.equal(calls[0].args, args);
});

test("logged call matches legacy phases and safe context, without logging secrets", async () => {
  const input = {
    connectionId: 12,
    force: true,
    timeoutMs: 2000,
    pid: null,
    password: "NEVER_LOG_THIS",
    sql: "SELECT * FROM private_table",
    path: "/private/path",
    caseId: "string identifiers are excluded",
  };
  assert.equal(safeInvokeContext(input), "connectionId=12 force=true timeoutMs=2000 pid=null");

  const original = captureRaw();
  const migrated = captureRaw();
  const legacy = legacyBridge(original.raw, () => 200);
  const current = createPgXrayInvoke(migrated.raw, () => 200);

  assert.deepEqual(await legacy("probe", input), await current("probe", input));
  await flushLogs();
  await flushLogs();
  assert.deepEqual(normalizedLogEntries(migrated.calls), normalizedLogEntries(original.calls));
  assert.deepEqual(normalizedLogEntries(migrated.calls).map((e) => e.phase), ["started", "succeeded"]);
  assert.equal(migrated.calls.filter((c) => c.command === "probe")[0].args, input);
  assert.ok(!JSON.stringify(normalizedLogEntries(migrated.calls)).includes("NEVER_LOG_THIS"));
});

test("failed invocation preserves exact rejection and legacy log phases", async () => {
  const originalError = new Error("test failed");
  const makeFailing = () => {
    const calls = [];
    return {
      calls,
      raw: async (command, args) => {
        calls.push({ command, args });
        if (command === "application_log_event") return;
        throw originalError;
      },
    };
  };
  const old = makeFailing();
  const newer = makeFailing();
  await assert.rejects(legacyBridge(old.raw)("login", { username: "alice", password: "secret" }), (err) => err === originalError);
  await assert.rejects(createPgXrayInvoke(newer.raw)("login", { username: "alice", password: "secret" }), (err) => err === originalError);
  await flushLogs();
  await flushLogs();
  const oldLogs = normalizedLogEntries(old.calls);
  const newLogs = normalizedLogEntries(newer.calls);
  // Durations depend on each invocation's clock; compare observable phases and errors.
  assert.deepEqual(newLogs.map((l) => [l.operation, l.phase, l.context, l.error]), oldLogs.map((l) => [l.operation, l.phase, l.context, l.error]));
  assert.equal(newLogs.length, 2);
});

test("logging rejection does not prevent user operation from succeeding", async () => {
  const expected = { id: 7 };
  const calls = [];
  const raw = async (command, args) => {
    calls.push({ command, args });
    if (command === "application_log_event") throw new Error("readonly logging directory");
    return expected;
  };
  const bridge = createPgXrayInvoke(raw);
  assert.equal(await bridge("save_connection", { connectionId: 7 }), expected);
  await flushLogs();
  assert.deepEqual(calls.map((c) => c.command), ["save_connection", "application_log_event", "application_log_event"]);
});

test("multiple logged requests serialize their log events", async () => {
  let activeLogs = 0;
  let maxConcurrentLogs = 0;
  const phases = [];
  const raw = async (command, args) => {
    if (command === "application_log_event") {
      activeLogs += 1;
      maxConcurrentLogs = Math.max(activeLogs, maxConcurrentLogs);
      await Promise.resolve();
      phases.push(args.phase);
      activeLogs -= 1;
      return;
    }
    return command;
  };
  const bridge = createPgXrayInvoke(raw);
  assert.deepEqual(await Promise.all([bridge("probe"), bridge("login")]), ["probe", "login"]);
  await flushLogs();
  assert.equal(maxConcurrentLogs, 1);
  assert.deepEqual(phases, ["started", "started", "succeeded", "succeeded"]);
});
