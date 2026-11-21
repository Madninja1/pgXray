import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// Guard against a regression where the same notice appeared above and below
// the log settings card, far from the action that produced it.
const source = readFileSync(new URL("../src/features/settings/SettingsView.vue", import.meta.url), "utf8");
const start = source.indexOf('<section v-else-if="extra.tab === \'logs\'"');
const end = source.indexOf('<section v-else-if="extra.tab === \'paths\'"', start);
assert.ok(start >= 0 && end > start, "log settings section exists");
const section = source.slice(start, end);

test("log settings notices and errors render once, within the settings card", () => {
  assert.equal((section.match(/v-if="extra\.notice"/g) ?? []).length, 1);
  assert.equal((section.match(/v-if="extra\.error"/g) ?? []).length, 1);
  const actions = section.indexOf('<div class="settings-actions">');
  const notice = section.indexOf('v-if="extra.notice"');
  const error = section.indexOf('v-if="extra.error"');
  const closingCard = section.indexOf('</div>\n          <div v-if="extra.log"');
  assert.ok(actions >= 0 && closingCard > actions);
  assert.ok(notice > actions && notice < closingCard);
  assert.ok(error > actions && error < closingCard);
});
