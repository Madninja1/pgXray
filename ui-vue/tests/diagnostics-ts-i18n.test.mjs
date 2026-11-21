import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { locale, t } from '../dist/features/i18n/locale-service.js';
import { indexLabel } from '../dist/features/index-health/controller.js';

const ru = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/ru.json', import.meta.url), 'utf8'));
const en = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/en.json', import.meta.url), 'utf8'));
const placeholders = text => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();

test('diagnostics controller translations exist in both languages with matching arguments', () => {
  assert.deepEqual(Object.keys(ru.diagTs).sort(), Object.keys(en.diagTs).sort());
  for (const key of Object.keys(en.diagTs)) {
    assert.deepEqual(placeholders(ru.diagTs[key]), placeholders(en.diagTs[key]), key);
    assert.ok(en.diagTs[key].trim(), key);
    assert.ok(ru.diagTs[key].trim(), key);
  }
});

test('Index Health classification changes language without changing classification ID', () => {
  const previous = locale.value;
  try {
    locale.value = 'en';
    assert.equal(indexLabel('suspiciously_unused'), en.diagTs.indexUnused);
    assert.match(indexLabel('suspiciously_unused'), /not a DROP recommendation/);
    locale.value = 'ru';
    assert.equal(indexLabel('suspiciously_unused'), ru.diagTs.indexUnused);
    assert.equal(indexLabel('unknown_classification'), 'unknown_classification');
  } finally { locale.value = previous; }
});

test('diagnostic messages interpolate counts without changing ids', () => {
  const previous = locale.value;
  try {
    locale.value = 'en';
    assert.match(t('diagTs.pgssArchived', { id: '25' }), /#25/);
    locale.value = 'ru';
    assert.match(t('diagTs.pgssArchived', { id: '25' }), /#25/);
  } finally { locale.value = previous; }
});
