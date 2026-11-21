import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from "../dist/core/shell-state.js";
import {
  createExtraSettingsController, createExtraSettingsState,
  parsePaths, parseLogEntries, parseHistoryRows, validateRetentionDays,
  LOG_SIZE_LIMITS,
} from "../dist/features/settings/extra-controller.js";

const PATHS = { workspace_root: "/home/workspace", export_dir: "/home/exports", application_log_dir: "/home/logs" };
const ROWS = [{ connection_id: 7, connection_name: "dev", snapshot_count: 4, oldest_at_ms: 100, newest_at_ms: 200 }];
const LOG_STATE = {
  settings: { max_total_bytes: LOG_SIZE_LIMITS[0], retention_days: 7 },
  status: { directory: "/home/logs", available: true, last_error: null, total_bytes: 12,
    file_count: 1, oldest_at_ms: 1, newest_at_ms: 2, max_file_bytes: 16777216 },
};
const ENTRIES = [{ timestamp_ms: 10, level: "info", component: "app", operation: "boot", phase: "succeeded",
  actor: null, request_id: null, duration_ms: null, context: null, error: null }];
const READINESS = { app_version: "0.5.5", supported_postgres: "14-18", local_checks: [], postgres: null };
const MAINTENANCE = { total_deleted: 4 };
function fixture(custom) {
  const shell = createShellState(); enterShellWorkspace(shell, "owner");
  const state = createExtraSettingsState(); const calls = [];
  const fallback = async (cmd) => ({
    path_settings: PATHS, history_retention_settings: { retention_days: 30 }, list_history_connections: ROWS,
    application_log_state: LOG_STATE, list_application_log_entries: ENTRIES,
    set_export_directory: { ...PATHS, export_dir: "/new/exports" },
    migrate_workspace_root: { settings: { ...PATHS, workspace_root: "/new/workspace" }, warning: "old directory not empty" },
    set_application_log_directory: { ...PATHS, application_log_dir: "/new/log" },
    set_application_log_settings: LOG_STATE, clear_application_log: LOG_STATE,
    set_history_retention: MAINTENANCE, delete_connection_history: MAINTENANCE,
    reveal_application_log_directory: "/home/logs", runtime_readiness: READINESS,
  })[cmd];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return custom ? custom(command, args, fallback) : fallback(command, args);
  };
  const controller = createExtraSettingsController(shell, state, invoke);
  return { shell, state, calls, controller };
}
function deferred() { let resolve; let reject; const promise = new Promise((ok, no) => { resolve = ok; reject = no; }); return { promise, resolve, reject }; }

test("refresh requests all three domains and strips unexpected fields", async () => {
  const { controller, state, calls } = fixture();
  assert.equal(await controller.refresh(), true);
  assert.equal(state.retentionDays, 30);
  assert.deepEqual(state.historyRows, ROWS);
  assert.deepEqual(state.paths, PATHS);
  assert.deepEqual(state.logEntries, ENTRIES);
  assert.deepEqual(calls.map(c => c.command), ["history_retention_settings", "list_history_connections", "path_settings", "application_log_state", "list_application_log_entries", "list_connections"]);
  assert.deepEqual(calls[4].args, { limit: 200 });
  assert.deepEqual(parsePaths({ ...PATHS, password: "secret" }), PATHS);
  assert.deepEqual(parseLogEntries(ENTRIES.map(e => ({ ...e, sql: "select sensitive" }))), ENTRIES);
});

test("history section counts are validated, consistent, and never guess old backend values", () => {
  const row = { ...ROWS[0], section_counts: { xid: 1, pgss: 1, autovacuum: 1, pg_settings: 1, io_wal: 0, index_health: 0 } };
  assert.deepEqual(parseHistoryRows([row]), [row]);
  assert.deepEqual(parseHistoryRows(ROWS), ROWS, "older backend summary keeps total but no fabricated breakdown");
  assert.throws(() => parseHistoryRows([{ ...row, section_counts: { ...row.section_counts, xid: 10 } }]), /не совпадает/);
  assert.throws(() => parseHistoryRows([{ ...row, section_counts: { ...row.section_counts, pgss: -1 } }]), /Некорректный/);
  assert.throws(() => parseHistoryRows([{ ...row, section_counts: { xid: 4 } }]), /Некорректный/);
});

test("bad IPC payload does not replace the last known good state", async () => {
  let fail = false;
  const { controller, state } = fixture((cmd, args, fallback) =>
    fail && cmd === "list_history_connections" ? [{ connection_id: "7" }] : fallback(cmd, args));
  assert.equal(await controller.refresh(), true);
  fail = true;
  assert.equal(await controller.refresh(), false);
  assert.deepEqual(state.historyRows, ROWS);
  assert.match(state.error, /История/);
  assert.throws(() => parseHistoryRows([{ connection_id: 0, connection_name: "bad", snapshot_count: 1, oldest_at_ms: 1, newest_at_ms: 2 }]));
});

test("log read failure is not displayed as an empty log", async () => {
  let fail = false;
  const { controller, state } = fixture(async (cmd, args, fallback) => {
    if (fail && cmd === "list_application_log_entries") throw new Error("permission denied");
    return fallback(cmd, args);
  });
  await controller.refresh(); fail = true;
  assert.equal(await controller.refresh(), false);
  assert.deepEqual(state.logEntries, ENTRIES);
  assert.match(state.logEntriesError, /permission denied/);
});

test("logout ignores late responses", async () => {
  const pending = deferred();
  const { shell, state, controller } = fixture((cmd, args, fallback) => cmd === "path_settings" ? pending.promise : fallback(cmd, args));
  const request = controller.refresh();
  leaveShellWorkspace(shell); controller.reset();
  pending.resolve(PATHS);
  assert.equal(await request, false);
  assert.equal(state.paths, null);
  assert.equal(state.loading, false);
});

test("later refresh wins over older refresh", async () => {
  const old = deferred(); let first = true;
  const { controller, state } = fixture((cmd, args, fallback) => {
    if (cmd === "path_settings" && first) { first = false; return old.promise; }
    if (cmd === "path_settings") return { ...PATHS, export_dir: "/current" };
    return fallback(cmd, args);
  });
  const request = controller.refresh();
  assert.equal(await controller.refresh(), true);
  old.resolve(PATHS); await request;
  assert.equal(state.paths.export_dir, "/current");
});

test("history retention validates before IPC and marks old rows stale, not empty", async () => {
  const { controller, state, calls } = fixture();
  await controller.refresh();
  assert.equal(validateRetentionDays(1.2), false);
  assert.equal(await controller.setHistoryRetention(36501), false);
  assert.equal(await controller.setHistoryRetention(null), true);
  assert.deepEqual(calls.findLast(call => call.command === "set_history_retention"), { command: "set_history_retention", args: { retentionDays: null } });
  assert.equal(state.historyStale, false, "history must reload automatically after retention update");
  assert.equal(state.historyLoaded, true);
  assert.equal(state.historyError, null);
  assert.deepEqual(state.historyRows, ROWS);
  assert.match(state.notice, /Удалено снимков: 4/);
  assert.doesNotMatch(state.notice, /Обновите/);
  assert.deepEqual(calls.slice(-4).map(c => c.command), ["set_history_retention", "history_retention_settings", "list_history_connections", "list_connections"]);
});

test("history delete shows all six snapshot categories and uses exact args", async () => {
  const { controller, state, calls } = fixture(); await controller.refresh();
  let message;
  assert.equal(await controller.deleteHistory(7, text => { message = text; return true; }), true);
  assert.match(message, /IO\/WAL/);
  assert.match(message, /PostgreSQL settings/);
  assert.match(message, /Index Health/);
  assert.match(message, /маркеры изменений/);
  assert.deepEqual(calls.at(-1), { command: "delete_connection_history", args: { connectionId: 7 } });
  assert.deepEqual(state.historyRows, []);
});

test("declined deletion does not invoke Rust", async () => {
  const { controller, calls } = fixture(); await controller.refresh();
  const before = calls.length;
  assert.equal(await controller.deleteHistory(7, () => false), false);
  assert.equal(calls.length, before);
});

test("pending confirmation blocks another action", async () => {
  const decision = deferred();
  const { controller, state, calls } = fixture(); await controller.refresh();
  const deleting = controller.deleteHistory(7, () => decision.promise);
  assert.equal(state.busy, true);
  assert.equal(await controller.saveExportDirectory("/new/exports"), false);
  decision.resolve(true);
  assert.equal(await deleting, true);
  assert.equal(state.busy, false);
  assert.equal(calls.some(c => c.command === "set_export_directory"), false);
});

test("logout during confirmation cancels destructive IPC", async () => {
  const decision = deferred();
  const { shell, controller, calls } = fixture(); await controller.refresh();
  const deleting = controller.deleteHistory(7, () => decision.promise);
  leaveShellWorkspace(shell); controller.reset(); decision.resolve(true);
  assert.equal(await deleting, false);
  assert.equal(calls.some(c => c.command === "delete_connection_history"), false);
});

test("workspace migration requires confirmation and records backend warning", async () => {
  const { controller, state, calls } = fixture(); await controller.refresh();
  assert.equal(await controller.migrateWorkspace("/new/workspace", () => false), false);
  assert.equal(await controller.migrateWorkspace("/new/workspace", () => true), true);
  assert.deepEqual(calls.at(-1), { command: "migrate_workspace_root", args: { directory: "/new/workspace" } });
  assert.match(state.notice, /old directory not empty/);
  assert.equal(state.paths.workspace_root, "/new/workspace");
});

test("log directory move, settings and clear preserve exact IPC contract", async () => {
  const { controller, state, calls } = fixture(); await controller.refresh();
  assert.equal(await controller.migrateLogDirectory("/new/log", () => true), true);
  assert.deepEqual(calls.at(-1), { command: "set_application_log_directory", args: { directory: "/new/log" } });
  assert.equal(await controller.setLogSettings(123, 11), false);
  assert.equal(await controller.setLogSettings(LOG_SIZE_LIMITS[0], 30), true);
  assert.deepEqual(calls.at(-1), { command: "set_application_log_settings", args: { maxTotalBytes: LOG_SIZE_LIMITS[0], retentionDays: 30 } });
  assert.equal(await controller.clearLog(() => true), true);
  assert.equal(calls.at(-1).command, "clear_application_log");
  assert.deepEqual(state.logEntries, []);
  assert.equal(state.logEntriesError, null);
});

test("readiness checks command and reply shape", async () => {
  const { controller, calls, state } = fixture();
  assert.equal(await controller.runReadiness(-1), false);
  assert.equal(await controller.runReadiness(null), true);
  assert.deepEqual(calls.at(-1), { command: "runtime_readiness", args: { connectionId: null } });
  assert.deepEqual(state.readiness, READINESS);
});

test("invalid reply after successful mutation flags uncertain result, never retry silently", async () => {
  const { state, controller, calls } = fixture((cmd, args, fallback) => cmd === "set_export_directory" ? { export_dir: "/new" } : fallback(cmd, args));
  assert.equal(await controller.saveExportDirectory("/new"), false);
  assert.match(state.error, /могла выполниться/);
  assert.equal(calls.filter(c => c.command === "set_export_directory").length, 1);
});

test("reset clears pending state and invalidates in-flight readiness", async () => {
  const pending = deferred();
  const { controller, state } = fixture((cmd, args, fallback) => cmd === "runtime_readiness" ? pending.promise : fallback(cmd, args));
  const checking = controller.runReadiness(12);
  controller.reset(); pending.resolve(READINESS);
  assert.equal(await checking, false);
  assert.equal(state.readiness, null);
  assert.equal(state.busy, false);
});

test("successful retention write stays successful when automatic history refresh fails", async () => {
  let fail = false;
  const { controller, state, calls } = fixture((command, args, fallback) => {
    if (fail && command === "list_history_connections") throw new Error("read failed");
    return fallback(command, args);
  });
  await controller.refresh();
  fail = true;
  assert.equal(await controller.setHistoryRetention(7), true);
  assert.equal(state.retentionDays, 7);
  assert.equal(state.historyStale, true);
  assert.deepEqual(state.historyRows, ROWS, "last valid summary must remain visible");
  assert.match(state.notice, /Срок хранения сохранён/);
  assert.match(state.historyError, /read failed/);
  assert.equal(calls.filter(call => call.command === "set_history_retention").length, 1);
});

test("targeted history refresh neither loads logs nor paths", async () => {
  const { controller, calls, state } = fixture();
  assert.equal(await controller.refreshHistory(), true);
  assert.deepEqual(calls.map(call => call.command), ["history_retention_settings", "list_history_connections", "list_connections"]);
  assert.equal(state.historyLoaded, true);
  assert.equal(state.historyError, null);
});

test("failure to fetch history must not be displayed as empty data", async () => {
  const { controller, state } = fixture((command, args, fallback) =>
    command === "list_history_connections" ? Promise.reject(new Error("database busy")) : fallback(command, args));
  assert.equal(await controller.refreshHistory(), false);
  assert.equal(state.historyLoaded, false);
  assert.equal(state.historyRows.length, 0);
  assert.match(state.historyError, /database busy/);
});

test("history request completing after logout cannot restore old data", async () => {
  const pending = deferred();
  const { controller, shell, state } = fixture((command, args, fallback) =>
    command === "list_history_connections" ? pending.promise : fallback(command, args));
  const request = controller.refreshHistory();
  leaveShellWorkspace(shell);
  controller.reset();
  pending.resolve(ROWS);
  assert.equal(await request, false);
  assert.deepEqual(state.historyRows, []);
  assert.equal(state.historyLoaded, false);
  assert.equal(state.loading, false);
});

test("settings tab change hides stale operation feedback", async () => {
  const { controller, state } = fixture();
  await controller.setHistoryRetention(90);
  assert.match(state.notice, /Срок хранения/);
  controller.changeTab("history");
  assert.equal(state.notice, null);
  assert.equal(state.error, null);
});

test("log viewer can request a larger bounded sample without refreshing history or paths", async () => {
  const { controller, calls, state } = fixture();
  assert.equal(await controller.refreshLogEntries(1000), true);
  assert.deepEqual(calls.map(x => x.command), ["application_log_state", "list_application_log_entries"]);
  assert.deepEqual(calls[1].args, { limit: 1000 });
  assert.deepEqual(state.logEntries, ENTRIES);
  assert.equal(state.logEntriesLoading, false);
  assert.equal(await controller.refreshLogEntries(1001), false);
  assert.equal(calls.length, 2);
});

test("log read failure preserves previous records for inspection", async () => {
  let failed = false;
  const { controller, state } = fixture((cmd, args, fallback) => {
    if (cmd === "list_application_log_entries" && failed) throw Error("read failure");
    return fallback(cmd, args);
  });
  await controller.refreshLogEntries(200);
  failed = true;
  assert.equal(await controller.refreshLogEntries(500), false);
  assert.deepEqual(state.logEntries, ENTRIES);
  assert.match(state.logEntriesError, /read failure/);
  assert.equal(state.logEntriesLoading, false);
});


test("log response completing after logout never exposes previous account events", async () => {
  const wait = deferred();
  const { shell, controller, state } = fixture((cmd, args, fallback) => {
    if (cmd === "list_application_log_entries") return wait.promise;
    return fallback(cmd, args);
  });
  const loading = controller.refreshLogEntries(500);
  leaveShellWorkspace(shell);
  controller.reset();
  wait.resolve(ENTRIES);
  assert.equal(await loading, false);
  assert.deepEqual(state.logEntries, []);
  assert.equal(state.logEntriesLoading, false);
});
