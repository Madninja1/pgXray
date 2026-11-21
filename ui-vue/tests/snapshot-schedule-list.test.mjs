import assert from 'node:assert/strict';
import test from 'node:test';
import { makeSchedule } from '../dist/features/snapshots/schedule-model.js';
import { scheduleListGroups, sortedVisibleSchedules, upcomingAttempt } from '../dist/features/snapshots/schedule-list.js';

const target = (id, name) => ({ id, name, host: 'localhost', port: 5432,
  database: 'qa', username: 'test', sslmode: 'disable' });
function rule(key, id, name, mode, nextAt, extra = {}) {
  return { ...makeSchedule({ connectionId: id, kind: 'xid', mode, intervalMinutes: 60,
    windowStart: '00:00', windowEnd: '00:00', intervalAnchor: '08:00',
    daysOfWeek: [0,1,2,3,4,5,6], dailyTime: '08:00', seriesCount: 2,
    seriesSpanMinutes: 60 }, target(id, name), Date.UTC(2026,10,20,7,0)),
    key, nextAt, ...extra };
}
const rules = [
  rule('late', 2, 'demo-db-b', 'interval', 300),
  rule('early', 1, 'demo-db-a', 'daily', 100),
  rule('paused', 2, 'demo-db-b', 'series', 999, {enabled:false, status:'paused'}),
  rule('middle', 1, 'demo-db-a', 'interval', 200),
  rule('done', 1, 'demo-db-a', 'series', 50, {status:'complete'}),
];

test('all connections by default; any subset of connections may be selected', () => {
  assert.equal(sortedVisibleSchedules(rules, null, 'connection', 'asc').length, 5);
  assert.deepEqual(sortedVisibleSchedules(rules, [2], 'connection', 'asc').map(r=>r.key), ['late','paused']);
  assert.equal(sortedVisibleSchedules(rules, [], 'connection', 'asc').length, 0);
  assert.equal(sortedVisibleSchedules(rules, [1,2], 'connection', 'asc').length, 5);
});

test('upcoming attempt ascending and descending; paused and completed always last', () => {
  assert.deepEqual(sortedVisibleSchedules(rules, null, 'nextAt', 'asc').map(r=>r.key),
    ['early','middle','late','done','paused']);
  assert.deepEqual(sortedVisibleSchedules(rules, null, 'nextAt', 'desc').map(r=>r.key),
    ['late','middle','early','done','paused']);
  assert.equal(upcomingAttempt(rules[2]), null);
  assert.equal(upcomingAttempt(rules[4]), null);
  assert.equal(upcomingAttempt(rules[1]), 100);
});

test('sort by schedule mode and group by mode, without mutating source', () => {
  const original = rules.map(r=>r.key);
  const byMode = sortedVisibleSchedules(rules, null, 'mode', 'asc');
  assert.deepEqual(byMode.map(r=>r.mode), ['interval','interval','daily','series','series']);
  assert.deepEqual(sortedVisibleSchedules(rules, null, 'mode', 'desc').map(r=>r.mode),
    ['series','series','daily','interval','interval']);
  const grouped = scheduleListGroups(byMode, true);
  assert.deepEqual(grouped.map(g=>[g.mode,g.rules.length]), [['interval',2],['daily',1],['series',2]]);
  assert.deepEqual(scheduleListGroups(byMode,true,true).map(g=>g.mode), ['series','daily','interval']);
  assert.equal(scheduleListGroups(byMode,false).length,1);
  assert.deepEqual(rules.map(r=>r.key), original);
});

test('Vue schedule list wires connection filter, mode grouping and upcoming-time sorting', async () => {
  const { readFileSync } = await import('node:fs');
  const view = readFileSync(new URL('../src/features/snapshots/SnapshotSchedulesView.vue', import.meta.url), 'utf8');
  assert.match(view, /:checked="isConnectionSelected\(connection\.id\)"/);
  assert.match(view, /@change="toggleConnection\(connection\.id\)"/);
  assert.match(view, /v-model="sortBy"/);
  assert.match(view, /v-model="sortDirection"/);
  assert.match(view, /v-model="groupByMode"/);
  assert.match(view, /v-for="group in groups"/);
  assert.match(view, /date\(upcomingAttempt\(rule\)\)/);
});
