import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { avUpdateTotals, avUpdatePeriod, avUpdatePlot } from "../dist/features/autovacuum/update.js";
import { avWorkloadInterval } from "../dist/features/autovacuum/workload.js";

const counters = (updated = 200, hot = 40) => ({
  tuples_inserted: 100, tuples_updated: updated, tuples_deleted: 50,
  hot_updates: hot, vacuum_count: 0, autovacuum_count: 1,
  analyze_count: 0, autoanalyze_count: 2,
  dead_tuples: 20, inserts_since_vacuum: 15,
  modifications_since_analyze: 10, estimated_rows: 1000,
});
const point = (id, ms, updated = 200, hot = 40, reset = null) => ({
  snapshot_id: id, collection_started_at_ms: ms, stats_reset: reset,
  table: counters(updated, hot),
});

// Counts are total lifetime counters at read time, not one observation interval.
test("cumulative HOT share is not confused with a share of dead tuples", () => {
  assert.deepEqual(avUpdateTotals(counters(1000, 360)), { updates: 1000, hot: 360, notHot: 640, hotPct: 36 });
  assert.equal(avUpdateTotals(counters(0, 0)).hotPct, null);
});

test("missing or inconsistent cumulative HOT counters do not manufacture a ratio", () => {
  assert.equal(avUpdateTotals(null), null);
  assert.equal(avUpdateTotals(counters(10, 15)), null);
  assert.equal(avUpdateTotals(counters(-5, 0)), null);
  assert.equal(avUpdateTotals(counters(Number.MAX_SAFE_INTEGER + 1, 0)), null);
});

test("observed interval compares HOT with UPDATE from the exact same interval", () => {
  const interval = avWorkloadInterval(point(1, 0), point(2, 7_200_000, 400, 140));
  const period = avUpdatePeriod(interval);
  assert.deepEqual({updates: period.updates, hot: period.hot, notHot: period.notHot, hotPct: period.hotPct},
    {updates: 200, hot: 100, notHot: 100, hotPct: 50});
  assert.equal(period.updatesPerHour, 100);
  assert.equal(period.hotPerHour, 50);
  assert.equal(period.resetVerified, false);
});

test("zero interval UPDATE with zero HOT is displayed as 0, but has no percentage", () => {
  const period = avUpdatePeriod(avWorkloadInterval(point(1, 0), point(2, 3_600_000)));
  assert.equal(period.updatesPerHour, 0);
  assert.equal(period.hotPct, null);
});

test("a HOT increase larger than UPDATE is rejected even when counters grow monotonically", () => {
  const interval = avWorkloadInterval(point(1, 0), point(2, 3_600_000, 220, 100));
  assert.equal(interval.status, "observed");
  assert.equal(avUpdatePeriod(interval), null);
});

test("counter reset and wrong timestamps exclude UPDATE/HOT chart points", () => {
  const reset = avWorkloadInterval(point(1, 0, 200, 40, "a"), point(2, 3_600_000, 300, 50, "b"));
  assert.equal(avUpdatePeriod(reset), null);
  const invalid = avWorkloadInterval(point(1, 0), point(2, 30_000, 300, 50));
  assert.equal(avUpdatePeriod(invalid), null);
});

test("chart bars are intervals in chronological order, not points joined by fabricated lines", () => {
  const first = avWorkloadInterval(point(1, 0), point(2, 3_600_000, 300, 70));
  const second = avWorkloadInterval(point(2, 3_600_000, 300, 70), point(3, 5_400_000, 450, 90));
  const plot = avUpdatePlot([second, first]);
  assert.equal(plot.bars.length, 2);
  assert.deepEqual(plot.bars.map(bar => bar.newerAt), [3_600_000, 5_400_000]);
  assert.deepEqual(plot.bars.map(bar => bar.updatesPerHour), [100, 300]);
  assert.equal(plot.maximum, 300);
  assert.ok(plot.bars.every(bar => bar.hHot <= bar.hUpdate && bar.yHot >= bar.yUpdate));
  assert.ok(plot.bars[1].xUpdate > plot.bars[0].xUpdate);
});

test("chart caps the number of pairs and stays finite when all counts are zero", () => {
  const windows = [];
  for (let i = 0; i < 10; i++) windows.push(avWorkloadInterval(point(i, i * 3_600_000), point(i+1, (i+1)*3_600_000)));
  const plot = avUpdatePlot(windows);
  assert.equal(plot.bars.length, 8);
  assert.equal(plot.maximum, 0);
  assert.ok(plot.bars.every(bar => bar.hHot === 0 && bar.hUpdate === 0));
});

test("UPDATE evidence stays in Analyzer and interval chart is in History", () => {
  const component = readFileSync(new URL("../src/features/autovacuum/AutovacuumView.vue", import.meta.url), "utf8");
  const updateSection = component.indexOf('<section class="subpanel update-health"');
  const indexDetails = component.indexOf("<details class=\"technical-note\"><summary>{{ tr('autovacuumUi.text120') }}");
  assert.ok(updateSection !== -1 && updateSection < indexDetails);
  const history = readFileSync(new URL("../src/features/history/AutovacuumHistoryAnalysis.vue", import.meta.url), "utf8");
  assert.match(component, /avUpdatePeriod\(latestObservedInterval\.value\)/);
  assert.match(history, /t\('historyDetails\.AutovacuumTrend'\)/);
  assert.match(history, /<option value="updates_per_hour">\{\{ t\('historyDetails\.AutovacuumUpdateRate'\) \}\}<\/option>/);
  for (const lang of ["ru", "en"]) {
    const dictionary = JSON.parse(readFileSync(new URL(`../src/features/i18n/locales/${lang}.json`, import.meta.url), "utf8"));
    assert.ok(dictionary.historyDetails.AutovacuumTrend);
    assert.ok(dictionary.historyDetails.AutovacuumUpdateRate);
  }
});
