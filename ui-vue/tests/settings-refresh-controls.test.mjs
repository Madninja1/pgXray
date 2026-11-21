import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const settings = readFileSync(new URL("../src/features/settings/SettingsView.vue", import.meta.url), "utf8");
const log = readFileSync(new URL("../src/features/settings/ApplicationLogViewer.vue", import.meta.url), "utf8");

test("readiness does not render nullable result while request is in progress", () => {
  assert.match(settings, /<div v-if="!shownReadiness" class="readiness-empty"/);
  assert.match(settings, /<template v-else>\s*<div class="readiness-section-heading">/);
  assert.doesNotMatch(settings, /v-if="!shownReadiness && !checkingReadiness"/);
});

test("refresh controls use accessible icon labels and expose progress", () => {
  assert.doesNotMatch(settings, /aria-label="Обновить историю подключений"/);
  assert.match(settings, /:aria-label="t\('settings\.readinessRefreshAria'\)"/);
  assert.match(settings, /:aria-busy="refreshingReadinessConnections"/);
  assert.match(settings, /refresh-spinning': refreshingReadinessConnections/);
  assert.match(log, /:aria-label="t\('journal\.refresh_journal_events'\)"/);
  assert.match(log, /'journal-spin': state.loading/);
});
