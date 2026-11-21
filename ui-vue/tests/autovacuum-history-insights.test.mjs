import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from "node:test";
import assert from "node:assert/strict";
import { avTrendLines, avIntervalLabel, needsAvAttention, rankAutovacuum } from "../dist/features/history/autovacuum-insights.js";
const row = (id, values={}) => ({ relation_oid:id, relation_name:`public.t${id}`, total_size_bytes:1024,
  dead_tuples:1,dead_tuple_trigger:100,dead_trigger_used_pct:1,insert_trigger_used_pct:0,
  analyze_trigger_used_pct:0,dead_trigger_reached:false,insert_trigger_reached:false,
  analyze_trigger_reached:false,autovacuum_applicable:true,autovacuum_enabled:true,delta:null,...values });
const interval = (from,to,state="comparable",updates=100) => ({
  from_snapshot_id:from,to_snapshot_id:to,from_ms:from*3600_000,to_ms:to*3600_000,
  state,updates_per_hour:updates,inserts_per_hour:0,deletes_per_hour:0,dead_tuples_estimate:100,
  comparable_relations:1,sampled_relations:1,elapsed_seconds:3600,dead_trigger_reached_count:0,
  vacuum_cycles:0,analyze_cycles:0,limitations:[]
});
test("Autovacuum attention marks threshold and disabled separately", () => {
  assert.equal(needsAvAttention(row(1)),false);
  assert.equal(needsAvAttention(row(2,{dead_trigger_reached:true})),true);
  assert.equal(needsAvAttention(row(3,{autovacuum_enabled:false})),true);
  assert.equal(needsAvAttention(row(4,{autovacuum_enabled:false,autovacuum_applicable:false})),false);
});
test("Autovacuum ranking prioritizes disabled and reached trigger", () => {
  const ranked=rankAutovacuum([row(1,{dead_trigger_used_pct:99}),row(2,{dead_trigger_reached:true,dead_trigger_used_pct:101})],"attention","");
  assert.deepEqual(ranked.map(x=>x.relation_oid),[2]);
});
test("Autovacuum trend does not join through reset or missing interval", () => {
  const result=avTrendLines([interval(1,2),interval(2,3,"stats_reset_changed"),interval(3,4)],"updates_per_hour");
  assert.equal(result.points.length,2);
  assert.equal(result.lines.length,2);
  assert.equal(result.lines.every(s=>s.length===1),true);
});
test("Autovacuum trend joins adjacent safe intervals with true time spacing", () => {
  const result=avTrendLines([interval(1,2),interval(2,3)],"updates_per_hour");
  assert.equal(result.lines.length,1);
  assert.equal(result.lines[0].length,2);
  assert.ok(result.points[0].x<result.points[1].x);
  assert.equal(avIntervalLabel("partial_sample"),"Неполное пересечение");
});
