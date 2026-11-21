import assert from "node:assert/strict";
import test from "node:test";
import { nextHistorySort, sortHistoryRows } from "../dist/features/history/sort-rows.js";

test("History sort toggles independently between ascending and descending", () => {
  assert.deepEqual(nextHistorySort(undefined,"size"), {column:"size",direction:"asc"});
  assert.deepEqual(nextHistorySort({column:"size",direction:"asc"},"size"), {column:"size",direction:"desc"});
  assert.deepEqual(nextHistorySort({column:"size",direction:"desc"},"time"), {column:"time",direction:"asc"});
});

test("History sort compares raw numbers, not formatted decimal strings", () => {
  const rows = [{id:1,bytes:1024},{id:2,bytes:80},{id:3,bytes:12000}];
  assert.deepEqual(sortHistoryRows(rows,{column:"bytes",direction:"asc"}).map(r=>r.id),[2,1,3]);
  assert.deepEqual(sortHistoryRows(rows,{column:"bytes",direction:"desc"}).map(r=>r.id),[3,1,2]);
  assert.deepEqual(rows.map(r=>r.id),[1,2,3],"original persisted order must stay unchanged");
});

test("History sort compares timestamps by time, including offsets", () => {
  const rows=[{id:1,at:"2026-11-18T10:00:00+03:00"},{id:2,at:"2026-11-18T07:30:00Z"},
    {id:3,at:"2026-11-18T09:00:00+01:00"}];
  assert.deepEqual(sortHistoryRows(rows,{column:"at",direction:"asc"}).map(r=>r.id),[1,2,3]);
});

test("History sort handles numeric-aware names, nested fields, booleans and stable ties", () => {
  const rows=[{name:"index10",info:{count:2},partial:false},{name:"index2",info:{count:5},partial:true},
    {name:"index1",info:{count:5},partial:false}];
  assert.deepEqual(sortHistoryRows(rows,{column:"name",direction:"asc"}).map(r=>r.name),["index1","index2","index10"]);
  assert.deepEqual(sortHistoryRows(rows,{column:"info.count",direction:"desc"}).map(r=>r.name),["index2","index1","index10"]);
  assert.deepEqual(sortHistoryRows(rows,{column:"partial",direction:"asc"}).map(r=>r.name),["index10","index1","index2"]);
});

test("Unknown/reset metrics always remain last, in both directions", () => {
  const rows=[{id:1,v:null},{id:2,v:10},{id:3,v:undefined},{id:4,v:2},{id:5,v:Number.NaN}];
  assert.deepEqual(sortHistoryRows(rows,{column:"v",direction:"asc"}).map(r=>r.id),[4,2,1,3,5]);
  assert.deepEqual(sortHistoryRows(rows,{column:"v",direction:"desc"}).map(r=>r.id),[2,4,1,3,5]);
});

test("History sorting applies to the entire filtered dataset before pagination", () => {
  const rows=Array.from({length:30},(_,id)=>({id,delta:id*10}));
  const page1=sortHistoryRows(rows,{column:"delta",direction:"desc"}).slice(0,20);
  assert.deepEqual(page1.map(row=>row.id),Array.from({length:20},(_,i)=>29-i));
});

test("History sort allows safe derived values without altering snapshots", () => {
  const rows=[{id:1,stats:{inserts:2,updates:3}},{id:2,stats:{inserts:8,updates:1}}];
  const result=sortHistoryRows(rows,{column:"dml",direction:"desc"},row=>row.stats.inserts+row.stats.updates);
  assert.deepEqual(result.map(row=>row.id),[2,1]);
});
