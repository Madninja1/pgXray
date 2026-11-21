import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import {
  journalModule, journalDescription, journalContext, journalContextValue, journalConnectionId, journalConnectionLabel,
  journalDateBounds, parseJournalPage,
} from "../dist/features/settings/log-explorer-model.js";
import { logDisplayRows } from "../dist/features/settings/log-view-model.js";
import { createJournalExplorerState, createJournalExplorerController } from "../dist/features/settings/log-explorer-controller.js";
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from "../dist/core/shell-state.js";

const entry = (timestamp_ms, phase, request_id = "req-1", component = "postgres") => ({
  timestamp_ms, phase, request_id, component, level: "info", operation: "internal_postgres",
  actor: null, duration_ms: phase === "query_started" ? null : 15,
  context: phase === "query_finished" ? "connection_id=7 sql_bytes=120 response_bytes=650 sql_fingerprint=abc123" : "connection_id=7 sql_bytes=120",
  error: null,
});
function result(entries, cursor = null, scan = false) {
  return { entries, next_cursor: cursor, scanned_bytes: 4096, malformed_lines: 0, scan_limit_reached: scan };
}
function fixture(invoke) {
  const shell = createShellState();
  enterShellWorkspace(shell, "admin");
  const state = createJournalExplorerState();
  const controller = createJournalExplorerController(shell, state, invoke);
  return { shell, state, controller };
}
function query() {
  return {
    limit: 50, cursor: null, from_ms: null, to_ms: null, component: null, section: null,
    level: null, operation: null, actor: null, request_id: null,
    problems_only: false, min_duration_ms: null, search: null,
  };
}
function deferred() {
  let resolve;
  const promise = new Promise(r => { resolve = r; });
  return { resolve, promise };
}

test("parses page strictly and does not accept secrets from extra backend fields", () => {
  const raw = { ...result([entry(20, "query_finished")]), password: "secret" };
  assert.deepEqual(parseJournalPage(raw).entries, raw.entries);
  assert.equal("password" in parseJournalPage(raw), false);
  assert.throws(() => parseJournalPage({ ...raw, scanned_bytes: "bad" }));
  assert.throws(() => parseJournalPage({ ...raw, next_cursor: {} }));
});

test("pairs backend SQL phases with request id and exposes lengths without SQL text", () => {
  const events = [entry(20, "query_finished"), entry(10, "query_started")];
  const row = logDisplayRows(events, "operations")[0];
  assert.equal(row.result, "success");
  assert.equal(row.entries.length, 2);
  assert.equal(journalContextValue(row.context, "response_bytes"), "650");
  assert.equal(journalContextValue(row.context, "sql_fingerprint"), "abc123");
  assert.equal(journalModule(row.operation, row.component), "PostgreSQL");
  assert.match(journalDescription(row), /Выполнен read-only SQL/);
  assert.deepEqual(journalContext("sql_bytes=120 nonsense response_bytes=650"), [
    { key: "sql_bytes", value: "120" }, { key: "response_bytes", value: "650" },
  ]);
});

test("local date bounds reject rollover and reversed dates", () => {
  const period = journalDateBounds("2026-11-10", "2026-11-15");
  assert.ok(period.to_ms > period.from_ms);
  assert.throws(() => journalDateBounds("2026-02-31", ""));
  assert.throws(() => journalDateBounds("2026-11-15", "2026-11-10"));
  assert.deepEqual(journalDateBounds("", ""), { from_ms: null, to_ms: null });
});

test("server-side cursor is sent for deep pages, previous page restores cursor", async () => {
  const requests = [];
  const { controller, state } = fixture((cmd, args) => {
    assert.equal(cmd, "query_application_log");
    requests.push(args.query.cursor);
    return result([entry(21 - requests.length, "query_finished")], requests.length <= 2 ? `file|${100 - requests.length}` : null);
  });
  assert.equal(await controller.load(query()), true);
  assert.equal(await controller.load(query(), "next"), true);
  assert.equal(await controller.load(query(), "next"), true);
  assert.equal(state.pageIndex, 2);
  assert.equal(await controller.load(query(), "previous"), true);
  assert.deepEqual(requests, [null, "file|99", "file|98", "file|99"]);
});

test("late results after logout never restore sensitive journal records", async () => {
  const pending = deferred();
  const { controller, state, shell } = fixture(() => pending.promise);
  const loading = controller.load(query());
  leaveShellWorkspace(shell);
  controller.reset();
  pending.resolve(result([entry(10, "query_finished")]));
  assert.equal(await loading, false);
  assert.equal(state.page, null);
});

test("failures retain last known page and do not silently show an empty log", async () => {
  let fails = false;
  const { controller, state } = fixture(() => fails ? Promise.reject("permission denied") : Promise.resolve(result([entry(22, "query_finished")])));
  assert.equal(await controller.load(query()), true);
  fails = true;
  assert.equal(await controller.load(query()), false);
  assert.equal(state.page.entries.length, 1);
  assert.match(state.error, /permission denied/);
});

test("detail mode loads matching request phases without mixing another actor", async () => {
  const { controller, state } = fixture((cmd, args) => {
    assert.equal(args.query.request_id, "req-1");
    return result([entry(10, "query_started"), entry(20, "query_finished"),
      { ...entry(30, "query_finished"), actor: "someone" }]);
  });
  const row = logDisplayRows([entry(20, "query_finished")], "operations")[0];
  await controller.open(row);
  assert.deepEqual(state.requestEvents.map(e => e.timestamp_ms), [10, 20]);
  controller.close();
  assert.equal(state.selected, null);
});

test("settings screen uses new paginated API, not client-only 1000 record limit", () => {
  const source = readFileSync(new URL("../src/features/settings/SettingsView.vue", import.meta.url), "utf8");
  const explorer = readFileSync(new URL("../src/features/settings/ApplicationLogViewer.vue", import.meta.url), "utf8");
  assert.match(source, /ApplicationLogViewer :invoke="props.invoke"/);
  const en = JSON.parse(readFileSync(new URL("../src/features/i18n/locales/en.json", import.meta.url), "utf8"));
  const ru = JSON.parse(readFileSync(new URL("../src/features/i18n/locales/ru.json", import.meta.url), "utf8"));
  const labels = [
    ["from_date", "С даты", "From date"],
    ["application_problems", "Проблемы приложения", "Application problems"],
    ["continue_searching_older_files", "Продолжить поиск в старых файлах", "Continue searching older files"],
  ];
  for (const [key, russian, english] of labels) {
    assert.ok(explorer.includes(`t('journal.${key}')`), `Missing localization call for ${key}`);
    assert.equal(ru.journal[key], russian);
    assert.equal(en.journal[key], english);
  }
  assert.doesNotMatch(explorer, /LOG_FETCH_LIMITS/);
});


test("new PGSS journal operations are admitted by the backend whitelist", () => {
  const backend = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf8");
  for (const operation of ["load_pgss_snapshot", "compare_pgss_snapshots", "pgss_statement_history"]) {
    assert.match(backend, new RegExp(`"${operation}"`));
  }
  assert.match(backend, /async fn query_application_log\(/);
  assert.match(backend, /\squery_application_log,/);
});

test("missing new Rust command reports the exact rebuild requirement", async () => {
  const { controller, state } = fixture(() => Promise.reject("Command query_application_log not found"));
  assert.equal(await controller.load(query()), false);
  assert.match(state.error, /пересборка и перезапуск Rust/);
});

test("logical journal section remains a server-side filter through older cursor pages", async () => {
  const seen = [];
  const { controller } = fixture((cmd, args) => {
    assert.equal(cmd, "query_application_log");
    seen.push({ section: args.query.section, component: args.query.component, cursor: args.query.cursor });
    return result([entry(100 - seen.length, "query_finished")], seen.length === 1 ? "file|120" : null);
  });
  const filters = { ...query(), section: "pgss", component: "ui_reported" };
  assert.equal(await controller.load(filters), true);
  assert.equal(await controller.load(filters, "next"), true);
  assert.deepEqual(seen, [
    { section: "pgss", component: "ui_reported", cursor: null },
    { section: "pgss", component: "ui_reported", cursor: "file|120" },
  ]);
});


test("journal displays saved connection name without inventing history", () => {
  const names = new Map([[7, "demo-db-a"], [2, "demo-db-b"]]);
  assert.equal(journalConnectionLabel(7, names), "demo-db-a (#7)");
  assert.equal(journalConnectionLabel(9, names), "Подключение #9 (имя недоступно)");
  const row = logDisplayRows([entry(20, "query_finished")], "operations")[0];
  assert.equal(journalConnectionId(row.entries), 7);
  assert.match(journalDescription(row, id => journalConnectionLabel(id, names)), /demo-db-a \(\#7\)/);
  assert.equal(journalConnectionId([{ ...entry(21, "query_finished"), context: "connection_id=bogus" }]), null);
  assert.equal(journalConnectionId([{ ...entry(22, "query_finished"), context: "connectionId=2" }]), 2);
  const diagnostic = { ...entry(23, "succeeded", "d-1", "diagnostics"),
    operation: "collect_pgss_snapshot", diagnostic: { connection_id: 2, kind: "pgss", snapshot_id: 8 } };
  assert.equal(journalConnectionId([diagnostic]), 2);
  assert.equal(journalConnectionId([
    { ...entry(23, "started", "ui-1", "ui_reported"), context: "connectionId=7" },
    diagnostic,
  ]), 2, "a UI context must not override the confirmed Rust diagnostic connection");
  const diagRow = logDisplayRows([diagnostic], "operations")[0];
  assert.match(journalDescription(diagRow, id => journalConnectionLabel(id, names)), /demo-db-b \(\#2\)/);
  assert.match(journalDescription(diagRow), /подключение #2/);
});

test("journal UI resolves names via authenticated saved profiles and clears on logout", () => {
  const vue = readFileSync(new URL("../src/features/settings/ApplicationLogViewer.vue", import.meta.url), "utf8");
  assert.match(vue, /props\.invoke\("list_connections"\)/);
  assert.match(vue, /generation !== props\.shell\.authGeneration/);
  assert.match(vue, /connectionNames\.value = new Map\(\)/);
  assert.match(vue, /formatConnection\(confirmedSnapshot\.connection_id\)/);
  assert.match(vue, /displayContextValue\(field\)/);
});
