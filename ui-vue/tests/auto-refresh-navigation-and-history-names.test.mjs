import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createShellState, enterShellWorkspace } from "../dist/core/shell-state.js";
import { createExtraSettingsState, createExtraSettingsController, parseLiveHistoryNames } from "../dist/features/settings/extra-controller.js";
const read = path => readFileSync(new URL(path, import.meta.url), "utf8");

test("connection-only refresh buttons are absent from views that reload on entry", () => {
  for (const path of ["activity/ActivityView.vue", "locks/LocksView.vue", "pgss/PgssView.vue", "xid/XidView.vue", "io-wal/IoWalView.vue", "index-health/IndexHealthView.vue", "pg-settings/PgSettingsView.vue", "what-changed/WhatChangedView.vue"]) {
    const text = read(`../src/features/${path}`);
    assert.doesNotMatch(text, />Обновить подключения<\/button>|>Подключения ↻<\/button>|aria-label="Обновить список подключений"/, path);
  }
  assert.doesNotMatch(read("../src/features/history/HistoryView.vue"), /@click="loadConnections">Обновить<\/button>/);
  assert.doesNotMatch(read("../src/features/settings/SettingsView.vue"), /aria-label="Обновить историю подключений"/);
});

test("Cases and Case sub-tabs reload when revisited and don't leak a validation error", () => {
  const tools = read("../src/features/tools/ToolsView.vue");
  const investigation = read("../src/features/tools/InvestigationPanel.vue");
  assert.doesNotMatch(tools, /Обновить разборы/);
  assert.match(tools, /if \(next === "cases" && props.visible/);
  assert.match(tools, /void actions.refreshConnections\(\)/);
  assert.match(investigation, /watch\(\(\) => props.section, section => \{/);
  assert.match(investigation, /state.error = ""/);
  assert.match(investigation, /void actions.refresh\(\)/);
  assert.doesNotMatch(investigation, /Обновить данные|Обновить журнал Apply/);
});

test("history name lookup does not include non-profile fields", () => {
  assert.deepEqual(parseLiveHistoryNames([{ id: 5, name: "Renamed QA", password: "private" }, { id: 7, name: "Other" }]), { 5: "Renamed QA", 7: "Other" });
  assert.throws(() => parseLiveHistoryNames(null), /Неверный список/);
});

test("Settings history uses current profile names, but keeps historical rows after profile lookup fails", async () => {
  const shell = createShellState(); enterShellWorkspace(shell, "qa");
  const state = createExtraSettingsState();
  const rows = [{ connection_id: 5, connection_name: "Old label", snapshot_count: 3, oldest_at_ms: 100, newest_at_ms: 200 },
    { connection_id: 6, connection_name: "Deleted profile", snapshot_count: 2, oldest_at_ms: 50, newest_at_ms: 150 }];
  let shouldFail = false;
  const invoke = async command => {
    if (command === "history_retention_settings") return { retention_days: null };
    if (command === "list_history_connections") return rows;
    if (command === "list_connections") {
      if (shouldFail) throw new Error("profile locked");
      return [{ id: 5, name: "Renamed QA", host: "localhost" }];
    }
    throw new Error(`Unexpected ${command}`);
  };
  const actions = createExtraSettingsController(shell, state, invoke);
  assert.equal(await actions.refreshHistory(), true);
  assert.equal(state.historyLiveNames[5], "Renamed QA");
  assert.equal(state.historyLiveNames[6], undefined);
  assert.deepEqual(state.historyRows, rows, "immutable history retains original snapshot labels");
  shouldFail = true;
  assert.equal(await actions.refreshHistory(), true, "live profile lookup failure does not erase snapshots");
  assert.deepEqual(state.historyLiveNames, {});
  assert.deepEqual(state.historyRows, rows);
});
