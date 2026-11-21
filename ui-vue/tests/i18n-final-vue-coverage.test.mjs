import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = path => readFileSync(new URL(path, import.meta.url), 'utf8');
const en = JSON.parse(source('../src/features/i18n/locales/en.json'));
const ru = JSON.parse(source('../src/features/i18n/locales/ru.json'));
const placeholders = text => [...text.matchAll(/\{([a-zA-Z_]\w*)\}/g)].map(match => match[1]).sort();

const groups = [
  'xidFinal', 'xidFinish', 'avFinal', 'avFinish', 'pgssFinal', 'pgssFinish',
  'histXidFinal', 'histIoFinal', 'histIndexFinal', 'histPgssFinal', 'histAvFinal',
  'vueRemaining', 'frontendPolish', 'vueFinalPolish',
];

test('final Vue translation groups exist in English and Russian with matching placeholders', () => {
  for (const group of groups) {
    assert.ok(en[group] && ru[group], `missing group ${group}`);
    assert.deepEqual(Object.keys(en[group]).sort(), Object.keys(ru[group]).sort(), `${group} keys differ`);
    for (const name of Object.keys(en[group])) {
      assert.deepEqual(placeholders(en[group][name]), placeholders(ru[group][name]), `${group}.${name} placeholders differ`);
    }
  }
});

test('XID retains its guarded APPLY confirmation while localizing diagnostics', () => {
  const view = source('../src/features/xid/XidView.vue');
  assert.match(view, /applyPhrase !== 'APPLY'/);
  assert.match(view, /tr\('xidFinal\.confirmWarning'\)/);
  assert.match(view, /tr\('xidFinal\.sqlCopied'\)/);
  assert.match(en.xidFinal.confirmWarning, /PostgreSQL maintenance/);
});

test('Autovacuum preserves the required Russian safety phrase and localizes thresholds', () => {
  const view = source('../src/features/autovacuum/AutovacuumView.vue');
  assert.match(view, /confirmText\.value\.trim\(\) !== "ПРИМЕНИТЬ"/);
  assert.match(view, /tr\('autovacuumUi\.mechanismDeadTitle'\)/);
  assert.match(view, /tr\('avFinal\.thresholdUse'/);
});

test('PGSS ranking and investigation guidance update reactively on locale change', () => {
  const pgss = source('../src/features/pgss/PgssView.vue');
  const caseView = source('../src/features/tools/InvestigationPanel.vue');
  assert.match(pgss, /const localizedRanks = computed/);
  assert.match(pgss, /tr\('frontendPolish\.explainPlanning'\)/);
  assert.match(caseView, /const localizedGuide = computed/);
  assert.match(caseView, /t\('frontendPolish\.guideSimilar'\)/);
  assert.match(en.frontendPolish.guideSimilar, /root cause/);
});
