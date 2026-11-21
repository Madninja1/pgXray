import assert from "node:assert/strict";
import test from "node:test";
import { sortProjectGroups, parsePinnedProjectIds, loadPinnedProjectIds, savePinnedProjectIds } from "../dist/features/connections/project-view-preferences.js";

const groups = [
  { id: 3, name: "pgXray", connections: [1,2] },
  { id: 2, name: "dba-trainer", connections: [3] },
  { id: null, name: "Без проекта", connections: [4,5,6] },
  { id: 1, name: "Analytics", connections: [7,8] },
];
test("projects sort by name or connection count; ungrouped remains last", () => {
  assert.deepEqual(sortProjectGroups(groups, "name-asc").map(x => x.id), [1,2,3,null]);
  assert.deepEqual(sortProjectGroups(groups, "name-desc").map(x => x.id), [3,2,1,null]);
  assert.deepEqual(sortProjectGroups(groups, "connections-desc").map(x => x.id), [1,3,2,null]);
  assert.deepEqual(groups.map(x=>x.id),[3,2,null,1], "sorting must not mutate source");
});
test("pinned projects are valid positive IDs only; corrupted storage fails closed", () => {
  assert.deepEqual(parsePinnedProjectIds('[1,1,-2,0,2.5,"3",2]'), [1,2]);
  assert.deepEqual(parsePinnedProjectIds('not json'), []);
  assert.deepEqual(parsePinnedProjectIds('{"password":"x"}'), []);
  assert.deepEqual(parsePinnedProjectIds(JSON.stringify(Array.from({length:1001},(_,i)=>i+1))), []);
});
test("per-user persistence survives component remount; never stores connection secrets", () => {
  const map = new Map();
  const storage = { getItem: key=>map.get(key) ?? null, setItem:(key,value)=>map.set(key,value) };
  assert.equal(savePinnedProjectIds(storage,'admin',[2,1,1]), true);
  assert.deepEqual(loadPinnedProjectIds(storage,'admin'),[2,1]);
  assert.deepEqual(loadPinnedProjectIds(storage,'other'),[]);
  assert.deepEqual(loadPinnedProjectIds(storage,null),[]);
  assert.equal(map.size,1);
  assert.doesNotMatch([...map.values()].join(' '), /password|host|postgres|dba-trainer/i);
});
test("local storage rejection does not block project browsing", () => {
  const storage = {getItem(){throw Error('blocked');},setItem(){throw Error('blocked');}};
  assert.deepEqual(loadPinnedProjectIds(storage,'admin'),[]);
  assert.equal(savePinnedProjectIds(storage,'admin',[1]),false);
});
