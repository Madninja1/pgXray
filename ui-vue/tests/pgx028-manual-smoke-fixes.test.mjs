import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = name => readFileSync(new URL(name, import.meta.url), "utf8");

test("confirmation uses an explicit asynchronous dialog instead of a restricted window.confirm", () => {
  const util = source("../src/core/confirm-action.ts");
  assert.match(util, /dialog\.showModal\(\)/);
  assert.match(util, /dialog\.returnValue === "approve"/);
  assert.match(util, /resolve\(false\)/);
  const files = ["settings/SettingsView.vue", "sessions/SessionsView.vue", "activity/ActivityView.vue", "snapshots/SnapshotSchedulesView.vue", "what-changed/WhatChangedView.vue", "tools/ToolsView.vue", "tools/ComparisonWorkbench.vue", "tools/PrivacyPanel.vue", "tools/PlanPicker.vue"];
  for (const file of files) {
    const view = source(`../src/features/${file}`);
    assert.doesNotMatch(view, /window\.confirm\(/, file);
    assert.match(view, /confirmAction/, file);
  }
});

test("readiness and maintenance have both language branches", () => {
  for (const file of ["../../src-tauri/src/main.rs", "../../crates/core/src/readiness.rs", "../../crates/core/src/history/management.rs", "../../crates/core/src/internal_postgres.rs"]) {
    const rust = source(file);
    assert.match(rust, /i18n::(?:text|is_ru)/, file);
  }
});

test("pinned snapshot table header has a separate translation key", () => {
  const panel = source("../src/features/settings/StorageManagementPanel.vue");
  assert.match(panel, /t\('storage\.pinnedCount'\)/);
  for (const lang of ["en", "ru"]) {
    const dictionary = JSON.parse(source(`../src/features/i18n/locales/${lang}.json`));
    assert.ok(dictionary.storage.pinnedCount);
    assert.ok(dictionary.storage.pinned.includes("{id}"));
  }
});
