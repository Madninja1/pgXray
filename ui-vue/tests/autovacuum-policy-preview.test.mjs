import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { avUnverifiedPolicyOptions } from '../dist/features/autovacuum/policy-preview.js';

const view = readFileSync(new URL('../src/features/autovacuum/AutovacuumView.vue', import.meta.url), 'utf8');
const queries = readFileSync(new URL('../../crates/core/src/autovacuum/queries.rs', import.meta.url), 'utf8');
const policy = readFileSync(new URL('../../crates/core/src/autovacuum/policy.rs', import.meta.url), 'utf8');

const detail = {
  table: { autovacuum_applicable: true, autovacuum_enabled: true,
    effective_vacuum_scale_factor: 0.2, dead_tuple_trigger: 100050 },
  fixed_scale_candidates: [
    { scale_factor: 0.2, resulting_dead_tuple_trigger: 100050 },
    { scale_factor: 0.1, resulting_dead_tuple_trigger: 50050 },
    { scale_factor: 0.05, resulting_dead_tuple_trigger: 25050 },
    { scale_factor: 0.3, resulting_dead_tuple_trigger: 150050 },
  ],
};

test('unknown stats reset exposes only lower read-only threshold scenarios', () => {
  assert.deepEqual(avUnverifiedPolicyOptions(null, true, detail), detail.fixed_scale_candidates.slice(1, 3));
});

test('verified reset, disabled stats and disabled autovacuum do not enable unverified scenarios', () => {
  assert.deepEqual(avUnverifiedPolicyOptions('2026-11-18', true, detail), []);
  assert.deepEqual(avUnverifiedPolicyOptions(null, false, detail), []);
  assert.deepEqual(avUnverifiedPolicyOptions(null, true, null), []);
  assert.deepEqual(avUnverifiedPolicyOptions(null, true, {...detail, table: {...detail.table, autovacuum_enabled: false}}), []);
});

test('previews reject ineffective or invalid trigger calculations', () => {
  const variants = { ...detail, fixed_scale_candidates: [
    { scale_factor: 0.15, resulting_dead_tuple_trigger: 100050 },
    { scale_factor: 0.10, resulting_dead_tuple_trigger: -1 },
    { scale_factor: 0.05, resulting_dead_tuple_trigger: Number.NaN },
    { scale_factor: 0.02, resulting_dead_tuple_trigger: 10050 },
  ]};
  assert.deepEqual(avUnverifiedPolicyOptions(null, true, variants), [variants.fixed_scale_candidates[3]]);
});

test('unknown mode opens expert calculation but cannot create Apply candidates', () => {
  assert.match(view, /<details class="subpanel expert-simulation" :open="statsResetUnknown">/);
  assert.match(view, /tr\('autovacuumUi\.text150'\)/);
  assert.match(view, /tr\('autovacuumUi\.text155'\)/);
  assert.match(policy, /else if !advice\.stats_period_continuous/);
});

test('TOAST heading identifies its parent and metadata is read via reltoastrelid', () => {
  const ru = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/ru.json', import.meta.url), 'utf8'));
  const en = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/en.json', import.meta.url), 'utf8'));
  assert.match(view, /tr\('avFinal\.toastSystem'\)/);
  assert.match(view, /state\.detail\.table\.schema_name/);
  assert.match(view, /state\.detail\.table\.relation_name/);
  assert.match(ru.avFinal.toastSystem, /служебная таблица для/);
  assert.match(en.avFinal.toastSystem, /internal table for/);
  assert.match(queries, /tc\.oid = t\.reltoastrelid/);
});
