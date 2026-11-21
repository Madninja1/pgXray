import assert from "node:assert/strict";
import test from "node:test";
import {
  isIndexHistorySummary, indexHistoryPairs, indexPairState, indexHistoryChart,
  indexHistoryChanges, indexHistoryTotals,
} from "../dist/features/history/index-health-insights.js";

const hour=3600_000;
const snapshot=(id, t, extra={})=>({
  id,connection_id:10,connection_name:"prod",collection_started_at_ms:t,
  host:"127.0.0.1",port:5432,database:"postgres",database_oid:5,username:"app",sslmode:"require",
  total_index_count:100,sampled_index_count:100,suspiciously_unused_count:3,
  possible_duplicate_count:1,high_write_overhead_count:2,sample_truncated:false,
  stats_reset:"reset-a",...extra,
});
const delta=(oid, extra={})=>({index_oid:oid,relation_oid:50,relation_name:"public.orders",index_name:`idx_${oid}`,
  sample_state:"comparable",idx_scan_delta:10,idx_tup_read_delta:100,idx_tup_fetch_delta:5,
  relation_dml_delta:20,older_size_bytes:100,newer_size_bytes:140,size_delta_bytes:40,
  newer_classifications:[],...extra,
});

test("Index history summary validates SQLite metadata (no PostgreSQL collector)",()=>{
  assert.equal(isIndexHistorySummary(snapshot(1,1000)),true);
  assert.equal(isIndexHistorySummary({...snapshot(1,1000),stats_reset:undefined}),false);
  assert.equal(isIndexHistorySummary({...snapshot(1,1000),sample_truncated:"false"}),false);
});
test("Index history pairs use adjacent samples in time, not input order",()=>{
  const pairs=indexHistoryPairs([snapshot(1,1*hour),snapshot(3,3*hour),snapshot(2,2*hour)]);
  assert.deepEqual(pairs.map(p=>[p.older.id,p.newer.id]),[[2,3],[1,2]]);
  assert.equal(indexPairState(pairs[0]),"comparable");
});
test("Index history rejects changed server target, unknown reset, resets and wrong time",()=>{
  const older=snapshot(1,1*hour);
  const classify=(newer)=>indexPairState({older,newer});
  assert.equal(classify(snapshot(2,2*hour,{host:"other"})),"target_changed");
  assert.equal(classify(snapshot(2,2*hour,{sslmode:"disable"})),"target_changed");
  assert.equal(classify(snapshot(2,1*hour)),"invalid_time");
  assert.equal(classify(snapshot(2,2*hour,{stats_reset:null})),"unknown_reset");
  assert.equal(classify(snapshot(2,2*hour,{stats_reset:"reset-b"})),"reset");
  assert.equal(classify(snapshot(2,2*hour,{sample_truncated:true})),"partial_sample");
  assert.equal(classify(snapshot(2,8*hour)),"gap");
});
test("Index chart never joins gaps/changed targets or incomplete classification samples",()=>{
  const rows=[snapshot(1,1*hour),snapshot(2,2*hour),snapshot(3,9*hour),
    snapshot(4,10*hour,{sample_truncated:true}),snapshot(5,11*hour)];
  const classified=indexHistoryChart(rows,"suspiciously_unused_count");
  assert.equal(classified.points.length,5);
  assert.deepEqual(classified.segments.map(s=>s.map(p=>p.id)),[[1,2],[3],[4],[5]]);
  const total=indexHistoryChart(rows,"total_index_count");
  assert.deepEqual(total.segments.map(s=>s.map(p=>p.id)),[[1,2],[3,4,5]]);
  const moved=indexHistoryChart([snapshot(1,1*hour),snapshot(2,2*hour,{database_oid:6})],"total_index_count");
  assert.equal(moved.segments.length,2);
  const reset=indexHistoryChart([snapshot(1,1*hour),snapshot(2,2*hour,{stats_reset:"reset-b"})],"suspiciously_unused_count");
  assert.equal(reset.segments.length,2);
  const catalog=indexHistoryChart([snapshot(1,1*hour),snapshot(2,2*hour,{stats_reset:"reset-b"})],"total_index_count");
  assert.equal(catalog.segments.length,1);
});
test("Index change filters distinguish size growth, scans and DML without scans",()=>{
  const rows=[delta(1),delta(2,{size_delta_bytes:-100,idx_scan_delta:0,relation_dml_delta:200}),
    delta(3,{sample_state:"sampled_only_newer",size_delta_bytes:null,idx_scan_delta:null,relation_dml_delta:null}),
    delta(4,{size_delta_bytes:0,idx_scan_delta:null,relation_dml_delta:10})];
  assert.deepEqual(indexHistoryChanges(rows,"growth","").map(r=>r.index_oid),[2,1]);
  assert.deepEqual(indexHistoryChanges(rows,"scans","").map(r=>r.index_oid),[1]);
  assert.deepEqual(indexHistoryChanges(rows,"silent","ORDER").map(r=>r.index_oid),[2]);
  assert.deepEqual(indexHistoryChanges(rows,"coverage","").map(r=>r.index_oid),[3]);
  const totals=indexHistoryTotals(rows);
  assert.equal(totals.matched,3);
  assert.equal(totals.unmatched,1);
  assert.equal(totals.measured,2);
  assert.equal(totals.scans,10);
  assert.equal(totals.netSizeBytes,-60);
  assert.equal(totals.silentWithDml,1);
  assert.equal(indexHistoryTotals([delta(1,{idx_scan_delta:null})]).scans,null);
});
