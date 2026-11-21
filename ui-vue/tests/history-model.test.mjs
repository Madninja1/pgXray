import assert from 'node:assert/strict';
import test from 'node:test';
import {
  SNAPSHOT_KINDS, LIST_HISTORY_COMMANDS, PERIOD_MINIMUM, countInRange,
  sameHistoryTarget, sortedSnapshots,
} from '../dist/features/history/model.js';

const sample = (id, timestamp, more = {}) => ({
  id, collection_started_at_ms: timestamp, connection_id: 7,
  host: 'db.internal', port: 5432, database: 'orders', database_oid: 16384,
  username: 'reader', sslmode: 'require', connection_name: 'prod', ...more,
});

test('History supports all six snapshot kinds and their existing IPC commands', () => {
  assert.equal(SNAPSHOT_KINDS.length, 6);
  assert.equal(new Set(SNAPSHOT_KINDS).size, 6);
  assert.equal(Object.keys(LIST_HISTORY_COMMANDS).length, 6);
  assert.equal(PERIOD_MINIMUM.pgss, 2);
  assert.equal(PERIOD_MINIMUM.pg_settings, 1);
});

test('Coverage counts inclusive boundaries using actual collection time', () => {
  const rows = [sample(1, 1000), sample(2, 2000), sample(3, 3000), sample(4, 4000)];
  assert.equal(countInRange(rows, 1000, 2000), 2);
  assert.equal(countInRange(rows, 3000, 4000), 2);
  assert.equal(countInRange(rows, 2001, 2999), 0);
});

test('Sorting is non-mutating, with stable timestamp tie-breaker', () => {
  const rows = [sample(1, 100), sample(3, 200), sample(2, 200)];
  assert.deepEqual(sortedSnapshots(rows).map(row => row.id), [3, 2, 1]);
  assert.deepEqual(rows.map(row => row.id), [1, 3, 2]);
});

test('Same logical target needs matching connection and database identity', () => {
  const a = sample(1, 1000);
  assert.equal(sameHistoryTarget(a, sample(2, 2000)), true);
  assert.equal(sameHistoryTarget(a, sample(3, 2000, { database_oid: 9000 })), false);
  assert.equal(sameHistoryTarget(a, sample(4, 2000, { username: 'admin' })), false);
  assert.equal(sameHistoryTarget(a, sample(5, 2000, { connection_id: 8 })), false);
});
