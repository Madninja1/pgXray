import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from 'node:test';
import assert from 'node:assert/strict';
import { retentionSelection, retentionDays, readinessConnections, readinessCheckStatus, readinessStatusLabel, readinessSupportLabel, readinessOverview, remainingRefreshFeedbackMs } from '../dist/features/settings/settings-view-model.js';

test('retention: forever and presets round-trip', () => {
  for (const days of [null, 7, 30, 90, 180, 365]) {
    const selected = retentionSelection(days);
    assert.equal(retentionDays(selected.choice, selected.custom), days);
  }
});
test('retention: custom valid and invalid values', () => {
  assert.deepEqual(retentionSelection(400), { choice: 'custom', custom: '400' });
  assert.equal(retentionDays('custom','400'), 400);
  for (const v of ['', '0', '36501', '-2', '1.5', 'Infinity', '  ', 'NaN']) {
    assert.equal(retentionDays('custom',v), undefined);
  }
  assert.equal(retentionDays('999', ''), undefined);
});
test('readiness selector retains only id and name', () => {
  assert.deepEqual(readinessConnections([{id:1,name:'prod',password:'secret'}]), [{id:1,name:'prod'}]);
});
test('readiness selector rejects malformed or duplicate identifiers', () => {
  for (const input of [null, {}, [{id:0,name:'x'}], [{id:1,name:'x'},{id:1,name:'y'}], [{id:'1',name:'x'}]]) {
    assert.throws(() => readinessConnections(input));
  }
});
test('readiness check statuses normalize supported labels', () => {
  for (const [status,expected] of [['ok','ok'],['passed','ok'],['WARN','warning'],['failed','error'],['unknown','other']]) {
    assert.equal(readinessCheckStatus({key:'x', label:'x', status, detail:''}), expected);
  }
});

test('a quick user refresh remains visibly animated for at least 450 ms', () => {
  assert.equal(remainingRefreshFeedbackMs(1000, 1015), 435);
  assert.equal(remainingRefreshFeedbackMs(1000, 1450), 0);
  assert.equal(remainingRefreshFeedbackMs(1000, 1800), 0);
  assert.equal(remainingRefreshFeedbackMs(1000, 999), 450);
});

// Readiness status codes come from Rust; avoid calling healthy checks "unknown".
test('readiness statuses from Rust are localized and counted without losing unknown values', () => {
  const checks = [
    {key:'a', label:'SQLite', status:'ready', detail:'ok'},
    {key:'b', label:'PGSS', status:'limited', detail:'extension absent'},
    {key:'c', label:'Signal', status:'unavailable', detail:'no privilege'},
    {key:'d', label:'DB', status:'error', detail:'failed'},
    {key:'e', label:'future', status:'pending', detail:'new status'},
  ];
  assert.equal(readinessCheckStatus(checks[0]), 'ok');
  assert.equal(readinessStatusLabel('ready'), 'Готово');
  assert.equal(readinessStatusLabel('limited'), 'Ограничено');
  assert.equal(readinessStatusLabel('unavailable'), 'Недоступно');
  assert.equal(readinessSupportLabel('unverified'), 'Версия не проверена');
  assert.deepEqual(readinessOverview(checks), {total:5,ok:1,warning:2,error:1,other:1});
  assert.equal(readinessStatusLabel('pending'), 'pending');
  assert.deepEqual(readinessOverview([]), {total:0,ok:0,warning:0,error:0,other:0});
});


test('readiness data keeps backend-provided method and detail without synthesizing results', () => {
  const check = {
    key: 'auth_db', label: 'Auth SQLite', status: 'ready',
    method: 'PRAGMA quick_check(1)', detail: 'SQLite вернул «ok».',
  };
  assert.equal(readinessCheckStatus(check), 'ok');
  assert.equal(check.method, 'PRAGMA quick_check(1)');
  assert.equal(check.detail, 'SQLite вернул «ok».');
  const oldBackend = { key: 'auth_db', label: 'Auth SQLite', status: 'ready', detail: 'Проверка пройдена.' };
  assert.equal(readinessCheckStatus(oldBackend), 'ok');
  assert.equal(oldBackend.method, undefined);
});
