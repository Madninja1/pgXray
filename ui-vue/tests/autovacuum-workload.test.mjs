import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { avWorkloadInterval, avWorkloadWindows } from "../dist/features/autovacuum/workload.js";

const table = (values = {}) => ({
  tuples_inserted: 100, tuples_updated: 200, tuples_deleted: 50,
  hot_updates: 40, vacuum_count: 0, autovacuum_count: 1,
  analyze_count: 0, autoanalyze_count: 2,
  dead_tuples: 20, inserts_since_vacuum: 15,
  modifications_since_analyze: 10, estimated_rows: 1000,
  ...values,
});
const p = (id, ms, t = {}, reset = null) => ({
  snapshot_id: id, collection_started_at_ms: ms, stats_reset: reset, table: table(t),
});

test("NULL stats_reset allows explicitly unverified observational INSERT/UPDATE/DELETE rates", () => {
  const result = avWorkloadInterval(p(1, 0), p(2, 3_600_000,
    {tuples_inserted: 130, tuples_updated: 320, tuples_deleted: 56, hot_updates: 75,
      dead_tuples: 40, inserts_since_vacuum: 30, modifications_since_analyze: 25}));
  assert.equal(result.status, "observed");
  assert.equal(result.resetVerified, false);
  assert.deepEqual(result.inserts, {delta: 30, averagePerHour: 30});
  assert.deepEqual(result.updates, {delta: 120, averagePerHour: 120});
  assert.deepEqual(result.deletes, {delta: 6, averagePerHour: 6});
  assert.equal(result.hotUpdates.averagePerHour, 35);
  assert.equal(result.deadTuplesChange, 20);
  assert.match(result.reason, /неизвестен/);
});

test("rate uses exact elapsed time, not number of snapshots or instantaneous speed", () => {
  const result = avWorkloadInterval(p(1, 0), p(2, 900_000, {tuples_inserted: 145}));
  assert.equal(result.elapsedMinutes, 15);
  assert.equal(result.inserts.averagePerHour, 180);
});

test("decreasing any cumulative counter prevents misleading rates", () => {
  const result = avWorkloadInterval(p(1, 0), p(2, 3_600_000, {autoanalyze_count: 1}));
  assert.equal(result.status, "unavailable");
  assert.equal(result.inserts, null);
  assert.match(result.reason, /уменьшился/);
});

test("explicitly changed stats_reset blocks the observational calculation", () => {
  const result = avWorkloadInterval(p(1, 0, {}, "reset-a"), p(2, 3_600_000, {}, "reset-b"));
  assert.equal(result.status, "unavailable");
  assert.match(result.reason, /изменился/);
});

test("one unknown reset boundary is not treated as stable", () => {
  const result = avWorkloadInterval(p(1, 0, {}, null), p(2, 3_600_000, {}, "2026"));
  assert.equal(result.status, "unavailable");
});

test("short or duplicate intervals do not invent hourly rates", () => {
  assert.equal(avWorkloadInterval(p(1, 0), p(2, 0)).inserts, null);
  assert.equal(avWorkloadInterval(p(1, 0), p(2, 59_000)).inserts, null);
});

test("non-safe JS integers are rejected, including suspicious cumulative overflow", () => {
  const result = avWorkloadInterval(p(1, 0), p(2, 3_600_000, {tuples_inserted: Number.MAX_SAFE_INTEGER + 1}));
  assert.equal(result.inserts, null);
});

test("a genuine known matching reset permits observational average but never changes Advisor", () => {
  const result = avWorkloadInterval(p(1, 0, {}, "2026"), p(2, 3_600_000, {}, "2026"));
  assert.equal(result.resetVerified, true);
  assert.equal(result.inserts.averagePerHour, 0);
});

test("adjacent intervals are returned newest first, preserving long gaps", () => {
  const history = {points: [
    p(1, 0), p(2, 600_000, {tuples_inserted: 150}),
    p(3, 2_400_000, {tuples_inserted: 200}),
  ]};
  const rows = avWorkloadWindows(history);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows.map(row => row.newerSnapshotId), [3, 2]);
  assert.deepEqual(rows.map(row => row.elapsedMinutes), [30, 10]);
  assert.deepEqual(rows.map(row => row.inserts.averagePerHour), [100, 300]);
  assert.deepEqual(avWorkloadWindows(history, 1).map(row => row.newerSnapshotId), [3]);
});


test("counter rates do not depend on dead-tuple or post-ANALYZE estimates staying monotonic", () => {
  const result = avWorkloadInterval(p(1, 0, {modifications_since_analyze: 900}),
    p(2, 3_600_000, {modifications_since_analyze: 5, dead_tuples: 2, tuples_updated: 225}));
  assert.equal(result.status, "observed");
  assert.equal(result.updates.averagePerHour, 25);
  assert.equal(result.modificationsSinceAnalyzeChange, -895);
});

test("observational rates do not mutate confirmed history/Advisor data", () => {
  const history = {summary:{stats_period_continuous:false, inserts_per_hour:null}, points:[
    p(2, 3_600_000, {tuples_inserted:110}), p(1, 0),
  ]};
  assert.equal(avWorkloadWindows(history)[0].inserts.averagePerHour, 10);
  assert.equal(history.summary.inserts_per_hour, null);
  assert.equal(history.summary.stats_period_continuous, false);
});
