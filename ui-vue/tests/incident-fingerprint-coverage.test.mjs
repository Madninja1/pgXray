import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const core = readFileSync(new URL("../../crates/core/src/workspace.rs", import.meta.url), "utf8");
const vue = readFileSync(new URL("../src/features/tools/InvestigationPanel.vue", import.meta.url), "utf8");
const readDictionary = lang => JSON.parse(readFileSync(new URL(`../src/features/i18n/locales/${lang}.json`, import.meta.url), "utf8"));
test("similar incidents report search coverage and use deterministic capped score", () => {
  assert.match(core, /similar_search_total_candidates/);
  assert.match(core, /fingerprint_match_score/);
  assert.match(core, /count\.min\(\*cap\)/);
  assert.match(core, /eligible\.into_iter\(\)\.take\(20\)/);
});
test("UI does not present fingerprint match as a confirmed root cause", () => {
  assert.match(vue, /t\('investigation\.similarWarning'/);
  for (const language of ["en", "ru"]) {
    const warning = readDictionary(language).investigation.similarWarning;
    assert.match(warning, language === "ru" ? /Баллы — только для сортировки/ : /Scores are only for ranking/);
    assert.match(warning, language === "ru" ? /OID может переиспользоваться/ : /OIDs may be reused/);
  }
});
