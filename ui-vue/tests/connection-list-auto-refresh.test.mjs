import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
const read = path => readFileSync(new URL(path, import.meta.url), "utf8");

test("snapshot schedules reload SQLite rules and PostgreSQL profiles on every tab entry", () => {
  const view = read("../src/features/snapshots/SnapshotSchedulesView.vue");
  const settings = read("../src/features/settings/SettingsView.vue");
  assert.match(settings, /v-if="extra.tab === 'snapshots'"/);
  assert.match(view, /onMounted\(\(\) => \{ void refresh\(\); \}\)/);
  assert.match(view, /props.actions.refreshConnections\(\)/);
  assert.match(view, /props.actions.refreshRules\(\)/);
  assert.doesNotMatch(view, />Обновить подключения<\/button>/);
});

test("Settings History refreshes upon reopening tab and counts only profiles with snapshots", () => {
  const settings = read("../src/features/settings/SettingsView.vue");
  assert.match(settings, /if \(tab === "history"\) void extraActions.refreshHistory\(\)/);
  assert.match(settings, /t\('settings\.historySummary', \{ connections: String\(historyPage\.totalConnections\), snapshots: formatInteger\(historyPage\.totalSnapshots\) \}\)/);
  const ru = JSON.parse(read("../src/features/i18n/locales/ru.json"));
  const en = JSON.parse(read("../src/features/i18n/locales/en.json"));
  assert.match(ru.settings.historySummary, /Подключений с историей: \{connections\}/);
  assert.match(en.settings.historySummary, /Connections with history: \{connections\}/);
  assert.match(ru.settings.historySummary, /\{snapshots\}/);
  assert.match(en.settings.historySummary, /\{snapshots\}/);
});

test("Monitoring refreshes an existing nonempty list after navigation, but never interrupts a monitor", () => {
  const view = read("../src/features/activity/ActivityView.vue");
  assert.match(view, /watch\(\(\) => props.visible, visible => \{[\s\S]*?if \(initialConnectionsPending\)/);
  assert.match(view, /if \(!state.monitoring && !state.signaling\) void refreshConnections\(\)/);
  assert.doesNotMatch(view, /connectionsLoaded.value && state.connections.length === 0/);
});
