import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { groupTimeline, timelineSourceLabel } from '../dist/features/what-changed/timeline-groups.js';
const root = new URL('../src/features/what-changed/', import.meta.url);
const view = readFileSync(new URL('WhatChangedView.vue', root), 'utf8');
const date = readFileSync(new URL('DateTimeField.vue', root), 'utf8');
const event = (source, at, n, phase = 'incident', kind = 'changed') => ({
  occurred_at_ms: at, phase, source, kind, evidence_level: 'confirmed_change',
  title: `Event ${n}`, detail: `Detail ${n}`, evidence_refs: [`xid_relation:${n}`],
});

test('consecutive XID observations at the same instant collapse without losing events or Evidence', () => {
  const events = [event('xid', 1000, 1), event('xid', 1000, 2), event('xid', 1000, 3)];
  const groups = groupTimeline(events);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].events.length, 3);
  assert.deepEqual(groups[0].events.flatMap(item => item.evidence_refs), events.flatMap(item => item.evidence_refs));
  assert.deepEqual(events.map(item => item.title), ['Event 1', 'Event 2', 'Event 3']);
});

test('different timestamps, sources, phases or event kinds are never merged', () => {
  const rows = [event('xid', 1000, 1), event('xid', 1001, 2), event('pg_stat_statements', 1001, 3),
    event('xid', 1001, 4), event('xid', 1001, 5, 'baseline'), event('xid', 1001, 6, 'baseline', 'other')];
  const groups = groupTimeline(rows);
  assert.equal(groups.length, 6);
  assert.deepEqual(groups.flatMap(group => group.events), rows);
  assert.deepEqual(groupTimeline([]), []);
  assert.equal(timelineSourceLabel('pg_stat_statements'), 'PGSS · SQL-запросы');
});

test('timeline uses reversible grouped disclosure and keeps Evidence navigation', () => {
  assert.match(view, /const timelineGroups = computed\(\(\) => groupTimeline/);
  assert.match(view, /group\.events\.length === 1 \|\| expandedTimeline/);
  assert.match(view, /t\('changes\.groupExpand'\)/);
  assert.match(view, /t\('changes\.groupCollapse'\)/);
  for (const lang of ["ru", "en"]) {
    const dictionary = JSON.parse(readFileSync(new URL(`../src/features/i18n/locales/${lang}.json`, import.meta.url), "utf8"));
    assert.ok(dictionary.changes.groupExpand);
    assert.ok(dictionary.changes.groupCollapse);
  }
  assert.match(view, /evidenceId\(ref\)/);
  assert.match(view, /showEvidence\(ref\)/);
});

test('each comparison period places beginning and end on separate rows without overflowing date/time inputs', () => {
  assert.match(view, /\.wc-period-fields\{display:grid;grid-template-columns:minmax\(0,1fr\)/);
  assert.match(view, /\.wc-periods\{display:grid;grid-template-columns:repeat\(2,minmax\(0,1fr\)\)/);
  assert.match(view, /@media\(max-width:900px\)\{\.wc-periods\{grid-template-columns:minmax\(0,1fr\)/);
  assert.match(date, /container-type:inline-size/);
  assert.match(date, /box-sizing:border-box/);
  assert.match(date, /@container \(max-width:360px\)/);
});
