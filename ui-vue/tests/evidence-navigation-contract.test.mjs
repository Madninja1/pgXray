import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
const read = p => readFileSync(new URL(p, import.meta.url), "utf8");
test("Case navigation retains query identity and snapshot in route", () => {
  const route = read("../src/features/tools/evidence-navigation.ts");
  assert.match(route, /target\.snapshot_id != null/);
  assert.match(route, /view: "pgss-history"/);
  assert.match(route, /view: "pgss-live"/);
  assert.match(route, /relation_oid/);
  assert.match(route, /tsFinish\.evidenceLocksPid/);
  assert.match(read("../src/features/i18n/locales/ru.json"), /PID \{pid\}.*переиспользован/);
  assert.match(read("../src/features/i18n/locales/en.json"), /PID \{pid\}.*reused/);
});
test("Case emits typed evidence route to App and preserves separate file navigation", () => {
  assert.match(read("../src/features/tools/InvestigationPanel.vue"), /navigate-evidence/);
  assert.match(read("../src/features/tools/ToolsView.vue"), /@navigate-evidence="navigateEvidence"/);
  assert.match(read("../src/App.vue"), /@navigate-evidence="navigateCaseEvidence"/);
});
