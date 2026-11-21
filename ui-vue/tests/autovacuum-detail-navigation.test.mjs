import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { avAdvisorEvidenceMessage } from '../dist/features/autovacuum/presentation.js';
const view = readFileSync(new URL('../src/features/autovacuum/AutovacuumView.vue', import.meta.url), 'utf8');

test('Analyzer keeps current state and advisor tabs; history is centralized', () => {
  for (const id of ['state','advisor','policy']) {
    assert.match(view, new RegExp(`id="av-tab-${id}"`));
    assert.match(view, new RegExp(`id="av-pane-${id}"`));
    assert.match(view, new RegExp(`aria-controls="av-pane-${id}"`));
    assert.match(view, new RegExp(`v-show="detailTab === '${id}'"`));
  }
  assert.match(view, /watch\(\(\) => state\.selectedTableOid/);
  assert.match(view, /watch\(\(\) => state\.connectionId/);
});

test('history is in HistoryView while the Analyzer retains advisor evidence', () => {
  const history = readFileSync(new URL('../src/features/history/HistoryView.vue', import.meta.url), 'utf8');
  const screen = readFileSync(new URL('../src/features/history/AutovacuumHistoryAnalysis.vue', import.meta.url), 'utf8');
  assert.match(history, /<AutovacuumHistoryAnalysis/);
  assert.match(screen, /autovacuum_workload_history/);
  assert.match(view, /emit\('openHistory',state\.connectionId,state\.selectedTableOid\)/);
  assert.doesNotMatch(view, /id="av-tab-history"/);
});

test('unknown stats reset is not rebranded as a shortage of snapshot collection', () => {
  const advice={ status:'insufficient_history', history_points:1 };
  const msg=avAdvisorEvidenceMessage(advice,{ savedPoints:8, resetUnknown:true });
  assert.equal(msg.title,'Неподтверждённая история счётчиков');
  assert.match(msg.message,/stats_reset = NULL/);
  assert.match(msg.message,/8 наблюдений/);
  assert.equal(avAdvisorEvidenceMessage(advice,{ savedPoints:8, resetUnknown:false }),null);
  assert.equal(avAdvisorEvidenceMessage({ status:'investigate', history_points:1 },{ savedPoints:8, resetUnknown:true }),null);
});

test('view-only simulation and guarded apply remain separate', () => {
  assert.match(view, /tr\('autovacuumUi\.text162'\)/);
  assert.match(view, /confirmText\.value\.trim\(\) !== "ПРИМЕНИТЬ"/);
  assert.match(view, /controller\.apply\(\)/);
  assert.match(view, /tr\('autovacuumUi\.text007'\)/);
});

test('history caveat is consolidated and technical limitations remain available',()=>{
  const ru = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/ru.json', import.meta.url), 'utf8'));
  const en = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/en.json', import.meta.url), 'utf8'));
  assert.match(view,/av-evidence-caveat/);
  for (const key of ['advisorLimitations', 'calcLimitations']) {
    assert.match(view, new RegExp(`tr\\('avFinal\\.${key}'\\)`));
    assert.ok(ru.avFinal[key]);
    assert.ok(en.avFinal[key]);
  }
  assert.match(ru.avFinal.advisorLimitations, /Технические ограничения Advisor/);
  assert.match(ru.avFinal.calcLimitations, /Технические ограничения расчёта/);
  assert.doesNotMatch(view,/<strong>Основное ограничение:<\/strong>/);
});

test('TOAST metrics and large index lists are compact, with no loss of detail',()=>{
  assert.match(view,/TOAST · \{\{ state\.detail\.table\.toast\.relation_name \}\}/);
  assert.match(view,/state\.detail\.table\.toast\.insert_trigger/);
  assert.match(view,/last_autovacuum/);
  assert.match(view,/v-for="index in sortedIndexes"/);
  assert.match(view,/index\.included_columns\.join/);
  assert.match(view,/av-index-scroll/);
  assert.doesNotMatch(view,/<ul><li v-for="index in state\.detail\.indexes\.data/);
});

test('policy simulation presents the two threshold margins and its evidence limit',()=>{
  assert.match(view,/simulationComparison\.thresholdDelta/);
  assert.match(view,/simulationComparison\.currentRemaining/);
  assert.match(view,/simulationComparison\.proposedRemaining/);
  assert.match(view,/tr\('autovacuumUi\.text174'\)/);
  assert.match(view,/state\.simulating/);
  assert.match(view,/controller\.apply\(\)/);
});
