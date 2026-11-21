import assert from "node:assert/strict";
import test from "node:test";
import { historyOverviewPage, HISTORY_PAGE_SIZES, HISTORY_SECTIONS, historySectionTotals } from "../dist/features/settings/settings-view-model.js";

const ROWS = Array.from({ length: 27 }, (_, n) => ({
  connection_id: n + 1, connection_name: `connection-${n + 1}`,
  snapshot_count: n + 2, oldest_at_ms: 10, newest_at_ms: n * 100,
}));

test("all connections: 10 per page, descending last snapshot and accurate summary", () => {
  const page = historyOverviewPage(ROWS, "all", 1, 10);
  assert.deepEqual(page.rows.map(row => row.connection_id), [27,26,25,24,23,22,21,20,19,18]);
  assert.equal(page.totalConnections, 27);
  assert.equal(page.totalSnapshots, ROWS.reduce((a, row) => a + row.snapshot_count, 0));
  assert.equal(page.pageCount, 3);
  assert.equal(page.from, 1);
  assert.equal(page.to, 10);
  assert.equal(page.page, 1);
  assert.equal(ROWS[0].connection_id, 1, "original IPC array must not be reordered");
});

test("selected connection filters to one row and resets pagination scope", () => {
  const page = historyOverviewPage(ROWS, "7", 3, 10);
  assert.deepEqual(page.rows.map(row => row.connection_id), [7]);
  assert.equal(page.totalConnections, 1);
  assert.equal(page.totalSnapshots, 8);
  assert.equal(page.page, 1);
  assert.equal(page.from, 1);
  assert.equal(page.to, 1);
});

test("pagination clamps after deletion and supports all configured page sizes", () => {
  assert.deepEqual(HISTORY_PAGE_SIZES, [10,25,50]);
  const last = historyOverviewPage(ROWS, "all", 999, 10);
  assert.equal(last.page, 3);
  assert.equal(last.from, 21);
  assert.equal(last.to, 27);
  assert.equal(last.rows.length, 7);
  const large = historyOverviewPage(ROWS, "all", 1, 50);
  assert.equal(large.rows.length, 27);
  assert.equal(large.pageCount, 1);
});

test("no history is different from one connection with zero snapshots", () => {
  const empty = historyOverviewPage([], "all", 5, 25);
  assert.equal(empty.totalConnections, 0);
  assert.equal(empty.totalSnapshots, 0);
  assert.equal(empty.from, 0);
  assert.equal(empty.to, 0);
  assert.equal(empty.pageCount, 1);
});

const WITH_COUNTS = [
  { connection_id: 1, connection_name: "a", snapshot_count: 4, oldest_at_ms: 1, newest_at_ms: 10,
    section_counts: { xid: 2, pgss: 1, autovacuum: 0, pg_settings: 1, io_wal: 0, index_health: 0 } },
  { connection_id: 2, connection_name: "b", snapshot_count: 3, oldest_at_ms: 2, newest_at_ms: 20,
    section_counts: { xid: 0, pgss: 1, autovacuum: 1, pg_settings: 0, io_wal: 1, index_health: 0 } },
];

test("section breakdown includes all six sections and totals the filtered scope, not one page", () => {
  assert.equal(HISTORY_SECTIONS.length, 6);
  const all = historyOverviewPage(WITH_COUNTS, "all", 1, 10);
  assert.deepEqual(all.totalBySection, { xid: 2, pgss: 2, autovacuum: 1, pg_settings: 1, io_wal: 1, index_health: 0 });
  assert.equal(Object.values(all.totalBySection).reduce((a, b) => a + b, 0), all.totalSnapshots);
  const filtered = historyOverviewPage(WITH_COUNTS, "2", 1, 10);
  assert.deepEqual(filtered.totalBySection, WITH_COUNTS[1].section_counts);
  assert.equal(historySectionTotals([WITH_COUNTS[0], ROWS[0]]), null, "mixed old/new backend rows must not produce partial totals");
  assert.equal(historySectionTotals([]), null);
});
