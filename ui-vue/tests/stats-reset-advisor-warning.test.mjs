import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const source = path => readFileSync(new URL(path, import.meta.url), 'utf8');
const av = source('../src/features/autovacuum/AutovacuumView.vue');
const xid = source('../src/features/xid/XidView.vue');
const pgss = source('../src/features/pgss/PgssView.vue');
const en = JSON.parse(source('../src/features/i18n/locales/en.json'));
const ru = JSON.parse(source('../src/features/i18n/locales/ru.json'));
const avAdvice = source('../../crates/core/src/autovacuum/advisor.rs');
const avPolicy = source('../../crates/core/src/autovacuum/policy.rs');
const xidPolicy = source('../../crates/core/src/xid/policy.rs');

test('Autovacuum Advisor explicitly distinguishes snapshot signals from verified history when stats_reset is NULL', () => {
  assert.match(av, /tr\('autovacuumUi\.text131'\)/);
  assert.match(av, /tr\('autovacuumUi\.text132'\)/);
  assert.match(avAdvice, /fn push_snapshot_screening\(/);
  assert.match(avAdvice, /"snapshot-only-signals"/);
  assert.match(avAdvice, /!metrics\.stats_period_continuous/);
  assert.match(avPolicy, /else if !advice\.stats_period_continuous/);
});

test('XID Policy Advisor retains guarded candidate selection and shows NULL warning', () => {
  assert.match(xid, /tr\('xidUi\.text110'\)/);
  assert.match(xidPolicy, /overview\.stats_reset\.is_none\(\)/);
  assert.match(xidPolicy, /else if !stats_period_continuous/);
  assert.match(xid, /role="alertdialog"/);
});

test('PGSS explains its independent stats_reset and retains the current snapshot interpretation', () => {
  assert.match(pgss, /tr\('pgssFinal\.statsUnknown'\)/);
  assert.match(pgss, /tr\('pgssFinal\.nullExplanation'\)/);
  assert.match(pgss, /tr\('pgssFinal\.deltaExplanation'\)/);
  assert.match(pgss, /snapshot\.interpretation\.items/);
  for (const dict of [en, ru]) {
    assert.match(dict.pgssFinal.statsUnknown, /PGSS stats_reset = NULL/);
    assert.match(dict.pgssFinal.nullExplanation, /pg_stat_statements/);
    assert.match(dict.pgssFinal.deltaExplanation, /deltas|дельты/);
  }
});
