import assert from 'node:assert/strict';
import test from 'node:test';
import { avMaintenanceIndicator, avHotGuidance } from '../dist/features/autovacuum/mechanisms.js';

const table = () => ({ autovacuum_applicable:true, autovacuum_enabled:true,
  dead_tuples:80, dead_tuple_trigger:101, dead_trigger_used_pct:79.2,
  inserts_since_vacuum:400, insert_trigger:500, insert_trigger_used_pct:80,
  modifications_since_analyze:120, analyze_trigger:100, analyze_trigger_used_pct:120,
  tuples_updated:1000, hot_update_pct:35.2,
});

test('three maintenance triggers have distinct observed counters and thresholds',()=>{
  const t=table();
  const dead=avMaintenanceIndicator(t,'dead',true), insert=avMaintenanceIndicator(t,'insert',true), analyze=avMaintenanceIndicator(t,'analyze',true);
  assert.deepEqual([dead.observed, insert.observed, analyze.observed],[80,400,120]);
  assert.deepEqual([dead.trigger,insert.trigger,analyze.trigger],[101,500,100]);
  assert.deepEqual([dead.state,insert.state,analyze.state],['below','below','reached']);
  assert.deepEqual([dead.remaining,insert.remaining,analyze.remaining],[21,100,null]);
});

test('missing and disabled insert threshold is not called zero or reached',()=>{
  const t=table();t.insert_trigger=null;t.insert_trigger_used_pct=null;
  const insert=avMaintenanceIndicator(t,'insert',true);
  assert.equal(insert.state,'disabled');assert.equal(insert.trigger,null);assert.equal(insert.usedPct,null);
});

test('unreliable statistics cannot become a false maintenance threshold',()=>{
  for (const kind of ['dead','insert','analyze']) {
    const result=avMaintenanceIndicator(table(),kind,false);
    assert.equal(result.state,'unknown');assert.equal(result.observed,null);assert.equal(result.usedPct,null);
  }
});

test('HOT ratio is not automatically problematic at 35% and undefined is not zero',()=>{
  assert.equal(avHotGuidance(table(),true).state,'observed');
  assert.equal(avHotGuidance({...table(),hot_update_pct:4.9},true).state,'investigate');
  assert.equal(avHotGuidance({...table(),hot_update_pct:null},true).state,'unknown');
  assert.equal(avHotGuidance(table(),false).state,'unknown');
  assert.equal(avHotGuidance({...table(),tuples_updated:0,hot_update_pct:0},true).state,'no-updates');
});

test('Vue has three explicit maintenance cards and does not invent direct XID/PGSS navigation', async()=>{
  const { readFileSync }=await import('node:fs');
  const { fileURLToPath }=await import('node:url');
  const { resolve,dirname }=await import('node:path');
  const source=readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../src/features/autovacuum/AutovacuumView.vue'),'utf8');
  const en=JSON.parse(readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../src/features/i18n/locales/en.json'),'utf8'));
  const ru=JSON.parse(readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../src/features/i18n/locales/ru.json'),'utf8'));
  for (const key of ['mechanismDeadTitle','mechanismInsertTitle','mechanismAnalyzeTitle']) {
    assert.match(source,new RegExp(`tr\\('autovacuumUi\\.${key}'\\)`));
    assert.ok(en.autovacuumUi[key]);
    assert.ok(ru.autovacuumUi[key]);
  }
  assert.equal(ru.autovacuumUi.mechanismDeadTitle,'VACUUM по мёртвым строкам');
  assert.equal(ru.autovacuumUi.mechanismInsertTitle,'VACUUM после вставок');
  assert.equal(ru.autovacuumUi.mechanismAnalyzeTitle,'ANALYZE для планировщика');
  assert.match(source, /tr\('autovacuumUi\.text114'\)/);
  assert.match(source, /tr\('autovacuumUi\.text118'\)/);
  assert.match(source, /tr\('autovacuumUi\.text113'\)/);
  assert.match(source,/avMechanismWindowEvidence\("insert"/);
});

test('maintenance cards have their OWN observational interval context',async()=>{
  const { avMechanismWindowEvidence }=await import('../dist/features/autovacuum/mechanisms.js');
  const w={elapsedMinutes:15,resetVerified:false,deadTuplesChange:12,insertsSinceVacuumChange:40,modificationsSinceAnalyzeChange:-300,
    inserts:{averagePerHour:160},updates:{averagePerHour:200},deletes:{averagePerHour:12}};
  assert.equal(avMechanismWindowEvidence('dead',w).estimate,12);
  assert.deepEqual(avMechanismWindowEvidence('dead',w).rates.map(r=>r.label),['UPDATE','DELETE']);
  assert.equal(avMechanismWindowEvidence('insert',w).rates[0].perHour,160);
  assert.equal(avMechanismWindowEvidence('analyze',w).estimate,-300);
  assert.deepEqual(avMechanismWindowEvidence('analyze',w).rates.map(r=>r.label),['INSERT','UPDATE','DELETE']);
  assert.equal(avMechanismWindowEvidence('insert',null),null);
  assert.equal(avMechanismWindowEvidence('analyze',{...w,updates:null}).rates[1].perHour,null);
});
