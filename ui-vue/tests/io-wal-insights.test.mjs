import assert from "node:assert/strict";
import test from "node:test";
import { ioWalChart, ioWalSizeChanges, ioWalStatus } from "../dist/features/history/io-wal-insights.js";

const interval = (id, at, extras = {}) => ({
  from_snapshot_id:id-1,to_snapshot_id:id,from_ms:at-60_000,to_ms:at,
  state:"comparable",database_state:"comparable",wal_state:"comparable",checkpoint_state:"comparable",
  wal_bytes_per_hour:100*id,temp_bytes_per_hour:5*id,blocks_read_per_hour:1000,
  checkpoints_requested_per_hour:1,...extras,
});

test("IO/WAL chart separates independently reset WAL intervals", () => {
  const rows=[interval(2,1_000_000),interval(3,1_060_000,{wal_state:"reset",wal_bytes_per_hour:null}),interval(4,1_120_000)];
  const wal=ioWalChart(rows,[],"wal_bytes_per_hour");
  assert.equal(wal.points.length,2);
  assert.equal(wal.segments.length,2);
  assert.equal(wal.segments[0].length,1);
  const db=ioWalChart(rows,[],"temp_bytes_per_hour");
  assert.equal(db.points.length,3);
  assert.equal(db.segments.length,1);
});

test("IO/WAL chart does not bridge six hour gaps", () => {
  const rows=[interval(2,1_000_000),interval(3,1_000_000+7*3600_000)];
  assert.equal(ioWalChart(rows,[],"wal_bytes_per_hour").segments.length,2);
});

test("IO/WAL size is a gauge even without continuous counter statistics", () => {
  const rows=[interval(3,1_060_000,{wal_state:"unknown_reset",state:"unavailable"})];
  const sizes=[{snapshot_id:2,at_ms:1_000_000,database_size_bytes:1_000},
    {snapshot_id:3,at_ms:1_060_000,database_size_bytes:2_000}];
  const chart=ioWalChart(rows,sizes,"database_size_bytes");
  assert.equal(chart.points.length,2);
  assert.equal(chart.segments.length,1);
});

test("I/O WAL status distinguishes unknown reset from confirmed reset", () => {
  assert.notEqual(ioWalStatus("unknown_reset"),ioWalStatus("reset"));
});

test("Relation size filter ranks signed changes by magnitude", () => {
  const data=[{relation_oid:1,older_name:"public.a",newer_name:"public.a",total_size_delta_bytes:-50},
    {relation_oid:2,older_name:"public.b",newer_name:"public.b",total_size_delta_bytes:100}];
  assert.deepEqual(ioWalSizeChanges(data,"public").map(r=>r.relation_oid),[2,1]);
  assert.deepEqual(ioWalSizeChanges(data,"PUBLIC.A").map(r=>r.relation_oid),[1]);
});
