import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const view = fs.readFileSync(path.join(root, 'ui/views/diagnostics.html'), 'utf8');
const controller = fs.readFileSync(path.join(root, 'ui/js/xid.js'), 'utf8');

test('XID and MultiXact UI fields have matching view anchors', () => {
  for (const id of [
    'xid-database-mxid-age', 'xid-mxid-freeze-max',
    'xid-relation-mxid-age', 'xid-relation-mxid-pct',
    'xid-relation-all-frozen-pages', 'xid-prepared-examples',
  ]) {
    assert.match(view, new RegExp(`id="${id}"`));
    assert.ok(controller.includes(`"${id}"`), `rendering ${id}`);
  }
});

test('XID UI distinguishes unavailable and missing threshold estimates', () => {
  assert.match(controller, /thresholds_reliable === false/);
  assert.match(controller, /Секция отношений недоступна/);
  assert.match(controller, /Секция возрастов баз недоступна/);
  assert.match(controller, /wraparound_detection === "unknown"/);
});

test('Read-only XID position is not described as an assigned transaction ID', () => {
  assert.match(view, /Граница XID снимка \(xmax\)/);
  assert.match(view, /pg_snapshot_xmax\(pg_current_snapshot\(\)\)/);
});
