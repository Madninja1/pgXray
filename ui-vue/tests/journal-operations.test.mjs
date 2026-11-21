import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseJournalPage, journalDescription } from "../dist/features/settings/log-explorer-model.js";
import { logDisplayRows } from "../dist/features/settings/log-view-model.js";
import { createJournalExplorerState, createJournalExplorerController } from "../dist/features/settings/log-explorer-controller.js";
import { createShellState, enterShellWorkspace } from "../dist/core/shell-state.js";

const opId = "diag-42";
const diagnostic = { connection_id: 3, kind: "pgss", snapshot_id: 12, unavailable_sections: 1, total_sections: 6 };
function event(component, operation, phase, request_id, timestamp_ms, details = null) {
  return { timestamp_ms, level: "info", component, operation, phase, actor: null,
    request_id, operation_id: opId, diagnostic: details,
    duration_ms: phase === "started" ? null : 500,
    context: null, error: null };
}
const start = event("diagnostics", "collect_pgss_snapshot", "started", opId, 10, { ...diagnostic, snapshot_id: null });
const end = event("diagnostics", "collect_pgss_snapshot", "succeeded", opId, 40, diagnostic);
const sql = event("postgres", "internal_postgres", "query_finished", "pg-123", 30, { ...diagnostic, snapshot_id: null });
function result(entries) {
  return { entries, next_cursor: null, scanned_bytes: 700, malformed_lines: 0, scan_limit_reached: false };
}

test("new metadata is accepted and old JSONL events remain unchanged", () => {
  const [parsed] = parseJournalPage(result([end])).entries;
  assert.equal(parsed.operation_id, opId);
  assert.deepEqual(parsed.diagnostic, diagnostic);
  const legacy = { ...end }; delete legacy.operation_id; delete legacy.diagnostic;
  assert.deepEqual(parseJournalPage(result([legacy])).entries, [legacy]);
  assert.throws(() => parseJournalPage(result([{ ...end, diagnostic: { ...diagnostic, snapshot_id: "12" } }])), /snapshot_id/);
});

test("Rust diagnostic result is clearly distinguished from UI report", () => {
  const rows = logDisplayRows([end, start], "operations");
  assert.equal(rows.length, 1);
  assert.match(journalDescription(rows[0]), /Rust сохранил pgss snapshot #12/);
});

test("opening a Rust operation queries all child SQL events by operation_id", async () => {
  const shell = createShellState(); enterShellWorkspace(shell, "admin");
  const state = createJournalExplorerState();
  const calls = [];
  const explorer = createJournalExplorerController(shell, state, async (name, args) => {
    calls.push({name, query: args.query});
    return result([end, sql, start]);
  });
  const row = logDisplayRows([end, start], "operations")[0];
  await explorer.open(row);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].query.operation_id, opId);
  assert.equal(calls[0].query.request_id, null);
  assert.deepEqual(state.requestEvents.map(e => e.timestamp_ms), [10, 30, 40]);
});

test("backend uses task-local correlation and confirms persisted snapshot only after save", () => {
  const core = readFileSync(new URL("../../crates/core/src/internal_postgres.rs", import.meta.url), "utf8");
  const rust = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf8");
  assert.match(core, /tokio::task_local!/);
  assert.match(core, /scope_diagnostic_operation/);
  assert.match(rust, /snapshot_id: Some\(snapshot_id\)/);
  for (const kind of ["xid", "pgss", "autovacuum", "pg_settings", "io_wal", "index_health"])
    assert.match(rust, new RegExp(`actor\\.as_deref\\(\\)\\s*,\\s*"${kind}"\\s*,\\s*connection_id`));
});
