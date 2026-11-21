import assert from 'node:assert/strict';
import test from 'node:test';
import { changed, autovacuumHasChanges, xidHasChanges, pgssMetricChanged, orderRows } from '../dist/features/what-changed/table-display.js';
import { readFileSync } from 'node:fs';
const vue = readFileSync(new URL('../src/features/what-changed/WhatChangedView.vue', import.meta.url), 'utf8');

test('unchanged zero-to-zero and missing-to-missing are not presented as changes', () => {
  assert.equal(changed({ inserts_per_hour: 0 }, { inserts_per_hour: 0 }, 'inserts_per_hour'), false);
  assert.equal(changed(null, null, 'inserts_per_hour'), false);
  assert.equal(changed({ inserts_per_hour: 0 }, { inserts_per_hour: 174036.2 }, 'inserts_per_hour'), true);
  assert.equal(changed({ inserts_per_hour: 0 }, null, 'inserts_per_hour'), true);
});

test('autovacuum preserves policy changes and observed differences, but hides unaffected rows', () => {
  const empty = { state: 'observed_in_both', baseline: { inserts_per_hour:0, updates_per_hour:0, deletes_per_hour:0, vacuum_cycles:0, analyze_cycles:0, latest_dead_tuples:0 }, incident: { inserts_per_hour:0, updates_per_hour:0, deletes_per_hour:0, vacuum_cycles:0, analyze_cycles:0, latest_dead_tuples:0 }, policy_changed:false };
  assert.equal(autovacuumHasChanges(empty), false);
  assert.equal(autovacuumHasChanges({...empty, incident: {...empty.incident, inserts_per_hour:174036.2}}),true);
  assert.equal(autovacuumHasChanges({...empty, policy_changed:true}),true);
  assert.equal(autovacuumHasChanges({...empty, state:'observed_in_incident_only'}),true);
});

test('XID preserves real age deltas and changes to freeze policy', () => {
  const same = { state:'observed_in_both', baseline:{latest_xid_age:200}, incident:{latest_xid_age:200}, freeze_policy_changed:false };
  assert.equal(xidHasChanges(same),false);
  assert.equal(xidHasChanges({...same, incident:{latest_xid_age:263}}),true);
  assert.equal(xidHasChanges({...same, freeze_policy_changed:true}),true);
});

test('PGSS unchanged metrics are suppressible without hiding real changes', () => {
  assert.equal(pgssMetricChanged({direction:'flat'}), false);
  assert.equal(pgssMetricChanged({direction:'unknown'}), false);
  assert.equal(pgssMetricChanged({direction:'up'}), true);
  assert.equal(pgssMetricChanged({direction:'appeared'}), true);
});

test('sort uses numerical comparisons, stable tie order and retains missing values at the end', () => {
  const values = [{id:'a',n:100},{id:'b',n:2},{id:'c',n:null},{id:'d',n:2}];
  assert.deepEqual(orderRows(values, row=>row.n, true).map(row=>row.id),['a','b','d','c']);
  assert.deepEqual(orderRows(values, row=>row.n).map(row=>row.id),['b','d','a','c']);
  assert.deepEqual(values.map(row=>row.id),['a','b','c','d']);
  assert.deepEqual(orderRows([{name:'idx10'}, {name:'idx2'}],row=>row.name).map(row=>row.name),['idx2','idx10']);
});

test('What Changed supports a reversible table filter, sortable headings and hidden Evidence links', () => {
  assert.match(vue, /const onlyDifferences = ref\(true\)/);
  assert.match(vue, /@click="toggleSort\('autovacuum', 'dead_delta'\)"/);
  assert.match(vue, /@click="toggleSort\('pgss', 'workload_delta'\)"/);
  assert.match(vue, /onlyDifferences\.value = false;\s*await nextTick\(\)/);
  assert.match(vue, /v-for="item in visibleAutovacuum"/);
  assert.match(vue, /v-for="metric in visibleStatementMetrics\(s\)"/);
});
