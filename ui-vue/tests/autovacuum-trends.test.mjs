import assert from "node:assert/strict";
import test from "node:test";
import { avBuildTrend, avLastSampleChange, avComparisonOverview } from "../dist/features/autovacuum/trends.js";

const point = (id, ms, dead, vacuumThreshold, changed, analyzeThreshold) => ({
  snapshot_id:id, collection_started_at_ms:ms,
  table:{dead_tuples:dead,dead_tuple_trigger:vacuumThreshold,
    modifications_since_analyze:changed,analyze_trigger:analyzeThreshold,
    vacuum_count:0,autovacuum_count:1,analyze_count:2,autoanalyze_count:3},
});

test("sampled estimates display even when stats_reset continuity is unknown", () => {
  const history={summary:{stats_period_continuous:false},points:[point(2,2000,350,400,12,200),point(1,1000,200,400,10,200)]};
  const trend=avBuildTrend(history,"vacuum");
  assert.deepEqual(trend.points.map(p=>p.snapshotId),[1,2]);
  assert.equal(trend.points[0].observed,200);
  assert.equal(trend.points[1].observed,350);
  assert.equal(trend.ceiling,400);
  assert.match(trend.observedLine,/^M/);
  assert.match(trend.thresholdLine,/L/);
  const analyze=avBuildTrend(history,"analyze");
  assert.deepEqual(analyze.points.map(p=>p.observed),[10,12]);
});

test("one sample does not invent an interpolated trend", () => {
  const trend=avBuildTrend({points:[point(1,1000,100,200,30,50)]},"vacuum");
  assert.equal(trend.observedLine,"");
  assert.equal(trend.points.length,1);
});

test("invalid estimates and thresholds are omitted instead of becoming zero", () => {
  const trend=avBuildTrend({points:[point(1,1000,10,0,0,50),point(2,2000,20,40,0,50)]},"vacuum");
  assert.equal(trend.points.length,1);
  assert.equal(trend.ceiling,40);
});

test("comparison counts unknown pairs separately and never interprets them as stable",()=>{
  const overview=avComparisonOverview({stats_period_continuous:false,table_changes:[
    {dead_tuples_delta:3},{dead_tuples_delta:-1},{dead_tuples_delta:null},{dead_tuples_delta:0}
  ]});
  assert.deepEqual(overview,{total:4,increased:1,decreased:1,unavailable:1,reliableCounters:false});
  assert.equal(avComparisonOverview(null),null);
});

test("Historical trends are centralized and the Analyzer retains evidence caveats", async()=>{
  const {readFileSync}=await import('node:fs');
  const {fileURLToPath}=await import('node:url');
  const {resolve,dirname}=await import('node:path');
  const source=readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../src/features/autovacuum/AutovacuumView.vue'),'utf8');
  const history=readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../src/features/history/AutovacuumHistoryAnalysis.vue'),'utf8');
  assert.match(source,/avAdvisorEvidenceMessage/);
  assert.match(history,/t\('historyDetails\.AutovacuumTrend'\)/);
  assert.match(history, /historyDetails/);
  assert.doesNotMatch(source,/id="av-tab-history"/);
});

test("insert-triggered VACUUM uses a third graph without inventing an INSERT rate", () => {
  const first = point(1,1000,100,150,5,10);
  const second = point(2,61000,100,150,6,10);
  first.table.inserts_since_vacuum=20;first.table.insert_trigger=100;
  second.table.inserts_since_vacuum=25;second.table.insert_trigger=100;
  const history={points:[second,first]};
  const trend=avBuildTrend(history,"insert");
  assert.deepEqual(trend.points.map(p=>p.observed),[20,25]);
  assert.deepEqual(trend.points.map(p=>p.threshold),[100,100]);
  assert.deepEqual(avLastSampleChange(history,"insert"),{delta:5,elapsedMinutes:1});
  second.table.insert_trigger=null;
  assert.equal(avBuildTrend(history,"insert").points.length,1);
  assert.equal(avLastSampleChange(history,"insert"),null);
});
