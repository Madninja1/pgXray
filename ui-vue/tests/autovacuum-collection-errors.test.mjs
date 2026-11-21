import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

const js = readFileSync(new URL('../../ui/js/autovacuum.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../../ui/views/diagnostics.html', import.meta.url), 'utf8');
const helperSource = js.match(/function autovacuumCollectionIssues\(snapshot\) \{[\s\S]*?\n\}/)?.[0];
assert.ok(helperSource, 'collection status helper must exist');
const issues = runInNewContext(`(${helperSource})`);

const complete = (tables = [], vacuums = [], analyzes = []) => ({
  tables: {data: tables, error: null},
  vacuums: {data: vacuums, error: null},
  analyzes: {data: analyzes, error: null},
});

test('empty-but-successful sections do not look like collection failures', () => {
  assert.equal(issues(complete()).length, 0);
});

test('failed table collection is visible even when other sections succeeded', () => {
  const snapshot = complete();
  snapshot.tables = {data: null, error: '[AV-COL-OUTPUT-LIMIT] превышен лимит 32 МиБ'};
  const result = issues(snapshot);
  assert.equal(result.length, 1);
  assert.match(result[0].label, /Таблицы/);
  assert.match(result[0].error, /OUTPUT-LIMIT/);
});

test('missing data without a specific backend error is still unavailable', () => {
  const snapshot = complete();
  snapshot.analyzes = {data: null, error: null};
  assert.match(issues(snapshot)[0].error, /не был собран/);
});

test('UI exposes collection failures next to summary and never substitutes zeros', () => {
  assert.match(html, /id="autovacuum-collection-status"/);
  assert.match(html, /id="autovacuum-collection-issues"/);
  assert.match(html, /id="autovacuum-collect-error"/);
  assert.match(js, /const hasTables = Array\.isArray\(snapshot\.tables\?\.data\)/);
  assert.match(js, /hasTables \? formatInteger\(tables\.length\) : "—"/);
  assert.match(js, /Array\.isArray\(snapshot\.vacuums\?\.data\)/);
  assert.match(js, /renderAutovacuumCollectionStatus\(snapshot\)/);
});
