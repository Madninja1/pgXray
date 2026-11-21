import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const view = readFileSync(new URL("../src/features/settings/SettingsView.vue", import.meta.url), "utf8");
const en = JSON.parse(readFileSync(new URL("../src/features/i18n/locales/en.json", import.meta.url), "utf8"));
const ru = JSON.parse(readFileSync(new URL("../src/features/i18n/locales/ru.json", import.meta.url), "utf8"));

function placeholders(message) {
  return [...message.matchAll(/\{([a-zA-Z][a-zA-Z0-9_]*)\}/g)].map(match => match[1]).sort();
}

test("SettingsView references existing translations in both languages", () => {
  const keys = [...view.matchAll(/t\(['"]settings\.([a-zA-Z0-9]+)['"]/g)].map(match => match[1]);
  assert.ok(keys.length > 100, "SettingsView should use its translation keys");
  for (const key of keys) {
    assert.equal(typeof en.settings[key], "string", `Missing English: ${key}`);
    assert.equal(typeof ru.settings[key], "string", `Missing Russian: ${key}`);
  }
  for (const key of Object.keys(ru.settings)) {
    assert.equal(typeof en.settings[key], "string", `Missing English key: ${key}`);
    assert.deepEqual(placeholders(en.settings[key]), placeholders(ru.settings[key]), `Different parameters: ${key}`);
  }
});

test("SettingsView renders accessibility labels from locale keys", () => {
  assert.match(view, /:aria-label="t\('settings\.usersList'\)"/);
  assert.match(view, /:aria-label="t\('settings\.readinessRefreshAria'\)"/);
  assert.match(view, /:aria-label="t\('settings\.historyDeleteAria'/);
});
