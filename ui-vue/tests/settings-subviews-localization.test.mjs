import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";

const read = path => readFileSync(new URL(path, import.meta.url), "utf8");
const files = [
  "../src/features/snapshots/SnapshotSchedulesView.vue",
  "../src/features/settings/ApplicationLogViewer.vue",
  "../src/features/settings/StorageManagementPanel.vue",
];
const en = JSON.parse(read("../src/features/i18n/locales/en.json"));
const ru = JSON.parse(read("../src/features/i18n/locales/ru.json"));
const placeholders = text => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();

test("settings subviews contain localized UI text and matching EN/RU keys", () => {
  for (const file of files) {
    const source = read(file);
    const keys = [...source.matchAll(/\bt\(['"](snapshots|journal|storage)\.([\w]+)['"]/g)];
    assert.ok(keys.length > 20, `${file}: missing translations`);
    for (const [, group, key] of keys) {
      assert.equal(typeof en[group]?.[key], "string", `EN ${group}.${key}`);
      assert.equal(typeof ru[group]?.[key], "string", `RU ${group}.${key}`);
      assert.deepEqual(placeholders(en[group][key]), placeholders(ru[group][key]), `${group}.${key}: interpolated parameters`);
    }
    assert.doesNotMatch(source, /[А-Яа-яЁё]/, `${file}: untranslated literal`);
  }
});

test("SQLite maintenance still requires explicit confirmation for destructive actions", () => {
  const panel = read(files[2]);
  assert.match(panel, /maintenanceActions\[action\]\.destructive/);
  assert.match(panel, /v-if="pendingAction"/);
  assert.match(panel, /@click="confirmPending"/);
  assert.match(panel, /@click="pendingAction=null"/);
  assert.match(panel, /storage\.confirmVacuum/);
  assert.match(panel, /storage\.confirmRetention/);
  assert.match(panel, /storage\.confirmPrune/);
});
