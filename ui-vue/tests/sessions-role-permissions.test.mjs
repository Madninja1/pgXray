import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";

const core = readFileSync(new URL("../../crates/core/src/sessions.rs", import.meta.url), "utf8");
const tauri = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf8");
const view = readFileSync(new URL("../src/features/sessions/SessionsView.vue", import.meta.url), "utf8");
const legacyView = readFileSync(new URL("../../ui/views/sessions.html", import.meta.url), "utf8");
const legacyController = readFileSync(new URL("../../ui/js/sessions.js", import.meta.url), "utf8");

// These source-level guards supplement (not replace) Rust and PostgreSQL tests.
test("pgXray does not inject SQL limits into psql at startup", () => {
  assert.doesNotMatch(core, /command\.env\(\s*"PGOPTIONS"/);
  assert.doesNotMatch(core, /terminal_startup_options|set_config\(\s*'idle_session_timeout'/);
  assert.match(core, /command\.env\("PSQLRC", &session_psqlrc_path\)/);
  assert.match(core, /starts_read_only: false/);
  assert.match(core, /idle_in_transaction_timeout_ms: 0/);
});

test("no compatibility mode remains in Tauri or Vue", () => {
  assert.doesNotMatch(tauri, /pooler_compatibility/);
  assert.doesNotMatch(view, /poolerStartupRejected|retryWithoutStartupOptions/);
  assert.match(view, /t\('sessions\.rolePrivileges'\)/);
});

test("legacy UI no longer claims automatic SQL limits", () => {
  assert.doesNotMatch(legacyView, /Сессия стартует в read-only режиме/);
  assert.doesNotMatch(legacyController, /idle-in-transaction:/);
  assert.match(legacyView, /с правами выбранной роли PostgreSQL/);
});
