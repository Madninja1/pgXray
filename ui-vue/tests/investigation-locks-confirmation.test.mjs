import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const read = name => readFileSync(new URL(`../src/features/${name}`, import.meta.url), "utf8");
const dictionary = lang => JSON.parse(readFileSync(new URL(`../src/features/i18n/locales/${lang}.json`, import.meta.url), "utf8"));
test("Investigation Case requires a WebView-safe confirmation for destructive actions", () => {
  const vue = read("tools/InvestigationPanel.vue");
  assert.doesNotMatch(vue, /window\.confirm/);
  assert.match(vue, /requestConfirmation\(t\('investigation\.deleteInterventionConfirm'/);
  assert.match(vue, /requestConfirmation\(t\('investigation\.deleteImpactConfirm'/);
  for (const lang of ["en", "ru"]) {
    const messages = dictionary(lang).investigation;
    assert.match(messages.deleteInterventionConfirm, /\{id\}/);
    assert.match(messages.deleteImpactConfirm, /\{id\}/);
    assert.match(messages.deleteInterventionConfirm, lang === "ru" ? /Удалить/ : /Delete/);
    assert.match(messages.deleteImpactConfirm, lang === "ru" ? /Удалить/ : /Delete/);
  }
  assert.match(vue, /@cancel="\$event\.preventDefault\(\); finishConfirmation\(false\)"/);
});
test("Locks Evidence and backend signals require WebView-safe explicit confirmation", () => {
  const vue = read("locks/LocksView.vue");
  assert.doesNotMatch(vue, /window\.confirm/);
  assert.match(vue, /await requestConfirmation\(t\('locks\.confirmation'\)\)/);
  assert.match(vue, /controller\.signal\(backend, action, requestConfirmation\)/);
  assert.match(vue, /finishConfirmation\(true\)/);
  assert.match(vue, /finishConfirmation\(false\)/);
  assert.match(vue, /sameLockIncidentEpisode\(incident, state\.incident\)/);
  assert.match(vue, /state\.notice\?\.startsWith\('Сохранено в Case'\)/);
  assert.match(vue, /t\('locks\.openFiles'\)/);
});
