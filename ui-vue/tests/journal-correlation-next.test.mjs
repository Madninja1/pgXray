import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createPgXrayInvoke } from "../dist/api/invoke.js";
import { logDisplayRows } from "../dist/features/settings/log-view-model.js";
import { parseLogEntries } from "../dist/features/settings/extra-controller.js";
import { journalDescription } from "../dist/features/settings/log-explorer-model.js";

const flush = () => new Promise(resolve => setImmediate(resolve));
const event = (component, phase, request_id, diagnostic = null, operation_result = null) => ({
  timestamp_ms: Date.now(), level: "info", component, operation: "collect_pgss_snapshot",
  phase, actor: component === "ui_reported" ? "admin" : null,
  request_id, duration_ms: phase === "succeeded" ? 120 : null,
  context: null, error: null, operation_id: diagnostic ? "diag-trusted" : null,
  diagnostic, operation_result,
});

test("bridge sends a UI correlation token without leaking the credential or altering ordinary calls", async () => {
  const calls = [];
  const bridge = createPgXrayInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "application_log_event") return undefined;
    return { ok: true };
  });
  await bridge("collect_pgss_snapshot", { connectionId: 4, password: "private" });
  await flush(); await flush();
  const dispatched = calls.find(([command]) => command === "collect_pgss_snapshot")[1];
  const logged = calls.find(([command, args]) => command === "application_log_event" && args.phase === "started")[1];
  assert.equal(dispatched.clientRequestId, logged.requestId);
  assert.ok(/^[a-z0-9]{6,20}:[a-z0-9]{1,8}$/.test(dispatched.clientRequestId));
  assert.equal(dispatched.password, "private");
  assert.ok(!JSON.stringify(calls.filter(([command]) => command === "application_log_event")).includes("private"));
  await bridge("probe", { connectionId: 4 });
  assert.deepEqual(calls.find(([command]) => command === "probe")[1], { connectionId: 4 });
});

test("client-linked UI, Rust and SQL are one row whose success is confirmed by Rust", () => {
  const details = { kind: "pgss", connection_id: 4, snapshot_id: 9,
    unavailable_sections: 0, total_sections: 6, client_request_id: "mwxabc123:1" };
  const entries = [
    event("ui_reported", "started", "mwxabc123:1"),
    event("diagnostics", "started", "diag-trusted", { ...details, snapshot_id: null }),
    event("postgres", "query_finished", "pg-query-1", details),
    event("diagnostics", "succeeded", "diag-trusted", details),
    event("ui_reported", "succeeded", "mwxabc123:1"),
  ];
  const rows = logDisplayRows(entries, "operations");
  assert.equal(rows.length, 1);
  assert.equal(rows[0].source, "backend");
  assert.equal(rows[0].result, "success");
  assert.equal(rows[0].entries.length, 5);
  assert.match(journalDescription(rows[0]), /Rust сохранил pgss snapshot #9/);
});

test("UI success cannot turn unconfirmed Rust start into a confirmed success", () => {
  const details = { kind: "xid", connection_id: 4, snapshot_id: null,
    unavailable_sections: null, total_sections: null, client_request_id: "mwxabc123:2" };
  const entries = [event("ui_reported", "succeeded", "mwxabc123:2"),
    event("diagnostics", "started", "diag-trusted", details)];
  const [row] = logDisplayRows(entries, "operations");
  assert.equal(row.result, "incomplete");
  assert.equal(row.source, "backend");
});

test("typed backend outcomes are parsed and represented as facts", () => {
  const raw = event("authentication", "succeeded", "mwxabc123:3", null,
    { kind: "authentication", authenticated: true });
  raw.operation = "login";
  const [entry] = parseLogEntries([raw]);
  assert.equal(entry.operation_result.authenticated, true);
  const [row] = logDisplayRows([entry], "operations");
  assert.match(journalDescription(row), /Rust подтвердил успешный вход/);
});

test("journal search panel has explicit full-width override of the global form limit", () => {
  const vue = readFileSync(new URL("../src/features/settings/ApplicationLogViewer.vue", import.meta.url), "utf8");
  assert.match(vue, /\.journal-query\{[^}]*width:100%;max-width:none/);
  assert.match(vue, /grid-template-columns:repeat\(12,minmax\(0,1fr\)\)/);
  assert.match(vue, /typedOutcome/);
});

test("PG Settings collector uses the instrumented read-only SQL executor", () => {
  const core = readFileSync(new URL("../../crates/core/src/postgres_settings.rs", import.meta.url), "utf8");
  assert.match(
    core,
    /run_readonly_query\(\s*connection_id\s*,\s*input\s*,\s*SETTINGS_SQL\s*,\s*QueryExecutionPolicy::standard_diagnostic\(\)\s*,?\s*\)/,
  );
});

test("opening an UI-only row searches full cross-layer history by client ID", async () => {
  const { createJournalExplorerState, createJournalExplorerController } =
    await import("../dist/features/settings/log-explorer-controller.js");
  const { createShellState, enterShellWorkspace } =
    await import("../dist/core/shell-state.js");
  const shell = createShellState();
  enterShellWorkspace(shell, "admin");
  const state = createJournalExplorerState();
  const ui = event("ui_reported", "succeeded", "mwxabc123:6");
  const diag = { kind: "pgss", connection_id: 3, snapshot_id: 7, unavailable_sections: 0,
    total_sections: 6, client_request_id: "mwxabc123:6" };
  const root = event("diagnostics", "succeeded", "diag-6", diag);
  const sql = event("postgres", "query_finished", "pg-6", diag);
  const queries = [];
  const controller = createJournalExplorerController(shell, state, async (command, args) => {
    assert.equal(command, "query_application_log");
    queries.push(args.query);
    return { entries: [root, sql, ui], next_cursor: null, scanned_bytes: 1024,
      malformed_lines: 0, scan_limit_reached: false };
  });
  const row = logDisplayRows([ui], "operations")[0];
  await controller.open(row);
  assert.equal(queries[0].client_request_id, "mwxabc123:6");
  assert.equal(state.requestEvents.length, 3);
  assert.ok(state.requestEvents.some(e => e.component === "diagnostics" && e.diagnostic.snapshot_id === 7));
});

test("ambiguous or different client request IDs cannot join operation histories", async () => {
  const { createJournalExplorerState, createJournalExplorerController } =
    await import("../dist/features/settings/log-explorer-controller.js");
  const { createShellState, enterShellWorkspace } =
    await import("../dist/core/shell-state.js");
  const shell = createShellState();
  enterShellWorkspace(shell, "admin");
  const state = createJournalExplorerState();
  const wanted = event("ui_reported", "succeeded", "mwxabc123:7");
  const foreign = event("diagnostics", "succeeded", "diag-foreign", {
    kind: "pgss", connection_id: 5, snapshot_id: 11, unavailable_sections: 0,
    total_sections: 6, client_request_id: "mwxabc123:8",
  });
  const controller = createJournalExplorerController(shell, state, async () => ({
    entries: [wanted, foreign], next_cursor: null, scanned_bytes: 1024,
    malformed_lines: 0, scan_limit_reached: false,
  }));
  await controller.open(logDisplayRows([wanted], "operations")[0]);
  assert.equal(state.requestEvents.length, 1);
  assert.equal(state.requestEvents[0].component, "ui_reported");
});
