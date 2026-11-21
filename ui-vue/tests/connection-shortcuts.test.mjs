import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { shortcutForKey } from '../dist/core/connection-shortcuts.js';

const profiles = [
  { id: 11, shortcut_slot: 1 },
  { id: 22, shortcut_slot: 0 },
  { id: 33, shortcut_slot: null },
];
const event = (key, overrides = {}) => ({
  key, altKey: false, ctrlKey: false, metaKey: false, shiftKey: false,
  repeat: false, target: null, ...overrides,
});

test('quick keys 1 and 0 map to saved connection IDs; unbound slots are inert', () => {
  assert.equal(shortcutForKey(event('1'), profiles, false), 11);
  assert.equal(shortcutForKey(event('0'), profiles, false), 22);
  assert.equal(shortcutForKey(event('2'), profiles, false), null);
});

test('shortcuts do not steal modifier keys, repeats or keyboard input from dialogs', () => {
  for (const changed of [{ ctrlKey: true }, { altKey: true }, { metaKey: true },
    { shiftKey: true }, { repeat: true }]) {
    assert.equal(shortcutForKey(event('1', changed), profiles, false), null);
  }
  assert.equal(shortcutForKey(event('1'), profiles, true), null);
  assert.equal(shortcutForKey(event('F1'), profiles, false), null);
});

test('shortcuts never steal typing from form fields or rich editors', () => {
  const previous = globalThis.Element;
  try {
    class TestElement { constructor(selectorMatches) { this.selectorMatches = selectorMatches; }
      closest() { return this.selectorMatches ? this : null; } }
    globalThis.Element = TestElement;
    assert.equal(shortcutForKey(event('1', { target: new TestElement(true) }), profiles, false), null);
    assert.equal(shortcutForKey(event('1', { target: new TestElement(false) }), profiles, false), 11);
  } finally { if (previous === undefined) delete globalThis.Element; else globalThis.Element = previous; }
});

test('shortcut selection is routed only to visible views and avoids running collectors', () => {
  const root = new URL('../src/features/', import.meta.url);
  const cases = [
    ['activity/ActivityView.vue', 'state.monitoring'],
    ['locks/LocksView.vue', 'state.monitoring'],
    ['autovacuum/AutovacuumView.vue', 'state.applying'],
    ['xid/XidView.vue', 'state.applying'],
    ['pgss/PgssView.vue', 'state.collecting'],
  ];
  for (const [filename, busyGuard] of cases) {
    const source = readFileSync(new URL(filename, root), 'utf8');
    assert.match(source, /!props\.visible/);
    assert.ok(source.includes(busyGuard), filename);
  }
});
