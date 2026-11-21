import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const view = readFileSync(new URL('../src/features/settings/SettingsView.vue', import.meta.url), 'utf8');
const readiness = view.split("v-else-if=\"extra.tab === 'readiness'\"")[1]?.split("\n        </section>")[0];

test('readiness UI has local controls and no generic success banner above the form', () => {
  assert.ok(readiness, 'readiness section exists');
  assert.doesNotMatch(readiness, /extra\.notice/);
  assert.match(readiness, /class="readiness-controls settings-subpanel"/);
  assert.match(readiness, /checkingReadiness/);
  assert.match(readiness, /extra\.error/);
  assert.match(readiness, /readiness-grid/);
});
