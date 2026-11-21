import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from 'node:assert/strict';
import test from 'node:test';
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from '../dist/core/shell-state.js';
import { createAvController, createAvState, avPage, avSummaryCounts, sameAvTarget,
  completeMaintenanceDelta, avDocsUrl, avSnapshotLabel, avStatsResetExplanation, avFormatFactor, avSimulationComparison, avTablePressure } from '../dist/features/autovacuum/controller.js';

const connection = (id, host=`db${id}`) => ({id,name:`db-${id}`,host,port:5432,database:'postgres',username:'tester'});
const summary = (id, c=1, oid=124) => ({id,connection_id:c,connection_name:`db-${c}`,host:`db${c}`,port:5432,database:'postgres',database_oid:oid,
  username:'tester',sslmode:'disable',collection_started_at_ms:id*1000,collection_completed_at_ms:id*1000+100,server_version_num:160015});
const table = (oid=5) => ({relation_oid:oid,schema_name:'public',relation_name:`t${oid}`,relation_type:'table',autovacuum_applicable:true,autovacuum_enabled:true,
  dead_tuples:100,dead_tuple_trigger:50,dead_trigger_reached:true,insert_trigger_reached:false,analyze_trigger_reached:true,
  analyze_trigger:30,total_size_bytes:8192,dead_trigger_used_pct:200,analyze_trigger_used_pct:400,modifications_since_analyze:120});
const snapshot = (oid=124,trackCounts=true) => ({overview:{database_oid:oid,database:'postgres',track_counts:trackCounts,server_version_num:160015},
  tables:{data:[table()],error:null},vacuums:{data:[],error:null},analyzes:{data:[],error:null},interpretation:{items:[],limitations:[]}});
const candidate = {id:'candidate-1',area:'vacuum',parameter:'autovacuum_vacuum_scale_factor',target_relation:'public.t5',current_scale_factor:0.2,candidate_scale_factor:0.1,
  current_trigger:200,candidate_trigger:100,rationale:[],sql_preview:'ALTER TABLE public.t5 SET (...)'};
const advice = {relation_oid:5,status:'candidate'};
const candidates = {relation_oid:5,candidates:[candidate]};
const detail = {table:table(),indexes:{data:[],error:null},update_statements:{data:[],error:null},interpretation:{items:[],limitations:[]}};
const history = {relation_oid:5,anchor_snapshot_id:3,points:[],summary:{history_points:2}};
function setup(invoke) { const shell=createShellState(); enterShellWorkspace(shell,'tester'); const state=createAvState(); const controller=createAvController(shell,state,invoke); return {shell,state,controller}; }
const flush=()=>new Promise(resolve=>setImmediate(resolve));
function api(cmd,args) {
  if(cmd==='list_connections')return [connection(1),connection(2)];
  if(cmd==='list_autovacuum_snapshots')return args.connectionId===1?[summary(3),summary(2)]:[];
  if(cmd==='load_autovacuum_snapshot')return snapshot();
  if(cmd==='autovacuum_table_detail')return detail;
  if(cmd==='autovacuum_relation_history')return history;
  if(cmd==='autovacuum_maintenance_advice')return advice;
  if(cmd==='autovacuum_policy_candidates')return candidates;
  throw Error(cmd);
}
async function withHistory(invoke=(cmd,args)=>Promise.resolve(api(cmd,args))) {
  const x=setup(invoke);await x.controller.refreshConnections();await flush();return x;
}
test('summary does not turn missing or disabled statistics into zero',()=>{
  assert.deepEqual(avSummaryCounts(null),{total:null,vacuum:null,dead:null,insert:null,analyze:null,disabled:null,activeVacuum:null,activeAnalyze:null});
  const s=snapshot(124,false);assert.equal(avSummaryCounts(s).vacuum,null);assert.equal(avSummaryCounts(s).analyze,null);
  assert.equal(avSummaryCounts(s).total,1); assert.equal(avPage(s,'','vacuum',1,25).total,0);
  assert.equal(avSummaryCounts(s).dead,null);assert.equal(avSummaryCounts(s).insert,null);
});
test('dead and insert VACUUM signals remain separate in overview and filters',()=>{
  const s=snapshot();const first=table(5);first.insert_trigger_reached=false;
  const second=table(6);second.dead_trigger_reached=false;second.insert_trigger_reached=true;
  s.tables.data=[first,second];
  const counts=avSummaryCounts(s);
  assert.equal(counts.vacuum,2);assert.equal(counts.dead,1);assert.equal(counts.insert,1);
  assert.deepEqual(avPage(s,'','dead',1,25).rows.map(t=>t.relation_oid),[5]);
  assert.deepEqual(avPage(s,'','insert',1,25).rows.map(t=>t.relation_oid),[6]);
});
test('pagination bounds DOM rows and filters names',()=>{
  const s=snapshot();s.tables.data=Array.from({length:240},(_,i)=>table(i+1));
  assert.equal(avPage(s,'', 'all',3,25).rows.length,25);
  assert.equal(avPage(s,'t201','all',1,25).rows[0].relation_oid,201);
  assert.equal(avPage(s,'','analyze',1,50).rows.length,50);
});
test('partial comparison is not claimed complete and docs URL validated',()=>{
  assert.equal(completeMaintenanceDelta(null,4),null);
  assert.equal(completeMaintenanceDelta(0,4),4);
  assert.equal(avDocsUrl(180001,'progress-reporting.html'),'https://www.postgresql.org/docs/18/progress-reporting.html');
  assert.equal(avDocsUrl(160015,'https://evil'),null);
});
test('saved source identity includes database_oid and connection',()=>{
  assert.equal(sameAvTarget(summary(3),summary(2)),true);
  assert.equal(sameAvTarget(summary(3),summary(2,2)),false);
  assert.equal(sameAvTarget(summary(3),summary(2,1,900)),false);
});
test('history uses connection-scoped loader and rejects cross-source snapshot',async()=>{
  const calls=[];const x=await withHistory(async(cmd,args)=>{calls.push([cmd,args]);if(cmd==='load_autovacuum_snapshot')return snapshot(555);return api(cmd,args);});
  assert.equal(x.state.snapshot,null);assert.match(x.state.error,/Содержимое снимка/);
  assert.equal(calls.filter(([cmd])=>cmd==='load_autovacuum_snapshot').every(([,args])=>args.connectionId===1),true);
});
test('stale history response discarded when connection switches',async()=>{
  let resolve;const promise=new Promise(r=>resolve=r);
  const x=setup(async(cmd,args)=>cmd==='list_connections'?[connection(1),connection(2)]:
    cmd==='list_autovacuum_snapshots'?(args.connectionId===1?promise:[]):api(cmd,args));
  await x.controller.refreshConnections();x.controller.selectConnection(2);resolve([summary(3)]);await flush();await flush();
  assert.equal(x.state.connectionId,2);assert.equal(x.state.snapshot,null);assert.deepEqual(x.state.histories,[]);
});
test('late collect response cannot overwrite another connection',async()=>{
  let resolve;const promise=new Promise(r=>resolve=r);
  const x=await withHistory(async(cmd,args)=>cmd==='collect_autovacuum_snapshot'?promise:api(cmd,args));
  const pending=x.controller.collect();x.controller.selectConnection(2);resolve({history:summary(5),snapshot:snapshot()});
  assert.equal(await pending,false);assert.equal(x.state.connectionId,2);assert.equal(x.state.snapshot,null);
});
test('snapshot load denies id not present in connection history',async()=>{
  const x=await withHistory();assert.equal(await x.controller.loadSnapshot(999),false);
  assert.equal(await x.controller.loadSnapshot(2),true);assert.equal(x.state.snapshotId,2);
});
test('late table details for a different relation do not replace current table',async()=>{
  let resolve;const waiting=new Promise(r=>resolve=r);
  const x=await withHistory(async(cmd,args)=>{
    if(cmd==='autovacuum_table_detail' && args.relationOid===5)return waiting;
    return api(cmd,args);
  });
  const older=x.controller.selectTable(5);await flush();
  x.controller.selectConnection(2);resolve(detail);
  assert.equal(await older,false);assert.equal(x.state.selectedTableOid,null);assert.equal(x.state.detail,null);
});
test('partial table detail failures retain history and advisor',async()=>{
  const x=await withHistory(async(cmd,args)=>{
    if(cmd==='autovacuum_table_detail') throw Error('offline');
    return api(cmd,args);
  });
  assert.equal(await x.controller.selectTable(5),true);
  assert.equal(x.state.detail,null);assert.equal(x.state.advice.status,'candidate');
  assert.match(x.state.detailErrors[0],/offline/);
});
test('historical connection mismatch blocks live detail and Apply',async()=>{
  const x=await withHistory();x.state.connections=[connection(1,'changed-host')];
  await x.controller.selectTable(5);
  assert.equal(x.state.detail,null);assert.equal(x.controller.prepareApply(candidate.id),false);
});
test('cross-source compare denied before IPC',async()=>{
  const calls=[];const x=await withHistory(async(cmd,args)=>{
    calls.push(cmd);if(cmd==='list_autovacuum_snapshots')return [summary(3),summary(2,1,500)];return api(cmd,args);
  });
  const count=calls.length;assert.equal(await x.controller.compare(),false);
  assert.equal(calls.length,count);
});
test('comparison response ignored if selection changed while pending',async()=>{
  let resolve;const waiting=new Promise(r=>resolve=r);
  const x=await withHistory(async(cmd,args)=>cmd==='compare_autovacuum_snapshots'?waiting:api(cmd,args));
  const pending=x.controller.compare();x.state.compareOlderId=3;
  resolve({older:summary(2),newer:summary(3),table_changes:[],limitations:[],elapsed_seconds:60});
  assert.equal(await pending,false);assert.equal(x.state.comparison,null);
});
test('Apply requires strong preconditions, uses candidate ID, never retries on ambiguous failure',async()=>{
  const calls=[];const x=await withHistory(async(cmd,args)=>{
    calls.push([cmd,args]);
    if(cmd==='apply_autovacuum_policy_candidate')throw Error('write outcome UNKNOWN');
    return api(cmd,args);
  });
  assert.equal(x.controller.prepareApply(candidate.id),false);
  await x.controller.selectTable(5);
  assert.equal(x.controller.prepareApply(candidate.id),true);
  assert.equal(await x.controller.apply(),false);
  assert.equal(x.state.applyBlocked,true);
  assert.match(x.state.error,/НЕ повторяй Apply/);
  assert.equal(calls.filter(([c])=>c==='apply_autovacuum_policy_candidate').length,1);
  assert.equal(x.controller.prepareApply(candidate.id),false);
  assert.deepEqual(calls.find(([c])=>c==='apply_autovacuum_policy_candidate')[1],
    {connectionId:1,snapshotId:3,relationOid:5,candidateId:'candidate-1'});
});
test('Apply success preserves verification, invalidates candidates and prevents stale reuse',async()=>{
  const x=await withHistory(async(cmd,args)=>{
    if(cmd==='apply_autovacuum_policy_candidate')return {candidate,verification:{before:{scale_factor:0.2,absolute_trigger:200,source:'global'},after:{scale_factor:0.1,absolute_trigger:100,source:'table'},rollback_sql:'DO $$...$$;',rollback_note:'Manual'},history:null,snapshot:null,warning:'Collect manually'};
    return api(cmd,args);
  });
  await x.controller.selectTable(5);x.controller.prepareApply(candidate.id);assert.equal(await x.controller.apply(),true);
  assert.equal(x.state.applyResult.verification.after.scale_factor,0.1);
  assert.equal(x.state.candidates,null);assert.equal(x.state.snapshot,null);
});
test('auth change stops pending responses',async()=>{
  let resolve;const pending=new Promise(r=>resolve=r);
  const x=setup(async(cmd,args)=>cmd==='list_autovacuum_snapshots'?pending:api(cmd,args));
  await x.controller.refreshConnections();leaveShellWorkspace(x.shell);resolve([summary(3)]);
  await flush();await flush();assert.equal(x.state.snapshot,null);
});


test('SQLite snapshot ID is not presented as per-connection ordinal', () => {
  const a = avSnapshotLabel(summary(15, 2), true);
  assert.match(a, /db-2/);
  assert.doesNotMatch(a, /№15/);
});

test('NULL stats_reset is explained without claiming disabled statistics', () => {
  const note = avStatsResetExplanation(true, null);
  assert.match(note, /не означает.*выключен/);
  assert.match(note, /Не сбрасывай статистику/);
  assert.equal(avStatsResetExplanation(true, '2026-11-13T10:00:00Z'), null);
  assert.match(avStatsResetExplanation(false, null), /отключён/);
});

test('simulation comparison distinguishes a mathematical threshold from scheduling', () => {
  const t = table(); t.dead_tuples = 120; t.dead_tuple_trigger = 200;
  const s = {resulting_dead_tuple_trigger: 100, candidate_scale_factor: 0.003711824464993254};
  const result = avSimulationComparison(t, s);
  assert.equal(result.currentUsage, 60);
  assert.equal(result.proposedUsage, 120);
  assert.equal(result.alreadyReached, true);
  assert.equal(avFormatFactor(s.candidate_scale_factor).length < 20, true);
  assert.equal(avSimulationComparison(t, {...s, resulting_dead_tuple_trigger:null}), null);
});

test('sortable snapshot table sorts ALL 1000 rows before paging, without touching the input',()=>{
  const s=snapshot();s.tables.data=Array.from({length:1000},(_,i)=>({
    ...table(i+1),total_size_bytes:(i+1)*1024,dead_tuples:i+1,
    insert_trigger:100,insert_trigger_used_pct:i/10,analyze_trigger_used_pct:i/20,
  }));
  const originalIds=s.tables.data.map(t=>t.relation_oid);
  const top=avPage(s,'','all',1,25,'size','desc');
  const next=avPage(s,'','all',2,25,'size','desc');
  assert.equal(top.total,1000);assert.equal(top.pages,40);
  assert.deepEqual(top.rows.map(t=>t.relation_oid),Array.from({length:25},(_,i)=>1000-i));
  assert.equal(next.rows[0].relation_oid,975);
  assert.equal(avPage(s,'t99','all',1,25,'name','asc').total,11);
  assert.deepEqual(s.tables.data.map(t=>t.relation_oid),originalIds);
});

test('sort treats missing percentages as unknown (last in both directions)',()=>{
  const s=snapshot();s.tables.data=[
    {...table(1),insert_trigger:100,insert_trigger_used_pct:20},
    {...table(2),insert_trigger:null,insert_trigger_used_pct:null},
    {...table(3),insert_trigger:100,insert_trigger_used_pct:50},
  ];
  assert.deepEqual(avPage(s,'','all',1,25,'insert','desc').rows.map(t=>t.relation_oid),[3,1,2]);
  assert.deepEqual(avPage(s,'','all',1,25,'insert','asc').rows.map(t=>t.relation_oid),[1,3,2]);
  s.overview.track_counts=false;
  assert.deepEqual(avPage(s,'','all',1,25,'vacuum','desc').rows.map(t=>t.relation_oid),[1,2,3]);
});

test('natural name order and status sorting are deterministic',()=>{
  const s=snapshot();s.tables.data=[
    {...table(11),dead_trigger_reached:false,analyze_trigger_reached:false},
    {...table(2),dead_trigger_reached:false,analyze_trigger_reached:false},
    {...table(10),autovacuum_enabled:false},
    table(3),
  ];
  assert.deepEqual(avPage(s,'','all',1,25,'name','asc').rows.map(t=>t.relation_oid),[2,3,10,11]);
  assert.deepEqual(avPage(s,'','all',1,25,'status','desc').rows.map(t=>t.relation_oid),[10,3,2,11]);
});

test('sortable headers and accessible order remain before table page slicing',async()=>{
  const { readFileSync }=await import('node:fs');
  const { fileURLToPath }=await import('node:url');
  const { resolve,dirname }=await import('node:path');
  const source=readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../src/features/autovacuum/AutovacuumView.vue'),'utf8');
  assert.match(source,/ariaSort\('size'\)/);
  assert.match(source,/@click="sortTables\('analyze'\)"/);
  assert.match(source,/tableSort\.value, sortDirection\.value/);
});

test('maintenance overview labels ratios by their own triggers, not table size', () => {
  const t = table(16871);
  Object.assign(t, {
    total_size_bytes: 11.9 * 1024 ** 3,
    dead_tuples: 1236638, dead_tuple_trigger: 1355401, dead_trigger_used_pct: 91.2,
    inserts_since_vacuum: 698513, insert_trigger: 5422401, insert_trigger_used_pct: 12.9,
    modifications_since_analyze: 191892, analyze_trigger: 542191, analyze_trigger_used_pct: 35.4,
  });
  assert.deepEqual(avTablePressure(t, 'dead', true), {
    observed: 1236638, trigger: 1355401, percent: 91.2,
  });
  assert.deepEqual(avTablePressure(t, 'insert', true), {
    observed: 698513, trigger: 5422401, percent: 12.9,
  });
  assert.deepEqual(avTablePressure(t, 'analyze', true), {
    observed: 191892, trigger: 542191, percent: 35.4,
  });
});

test('unknown maintenance values never become zero or fake percentages', () => {
  const t = table(5);
  assert.equal(avTablePressure(t, 'dead', false), null);
  assert.equal(avTablePressure(t, 'insert', true), null);
  t.inserts_since_vacuum = 0;
  t.insert_trigger = null;
  t.insert_trigger_used_pct = null;
  assert.equal(avTablePressure(t, 'insert', true), null);
  t.insert_trigger = 101;
  assert.deepEqual(avTablePressure(t, 'insert', true), {observed: 0, trigger: 101, percent: null});
});

test('write activity sorts all snapshot tables before pagination, unknown statistics last', () => {
  const s = snapshot();
  const rows = Array.from({length: 60}, (_, i) => ({...table(i + 1), tuples_updated: i * 20, tuples_inserted: i * 10, tuples_deleted: i * 5, hot_update_pct: i}));
  s.tables.data = rows;
  const page1 = avPage(s, '', 'all', 1, 25, 'updated', 'desc');
  const page2 = avPage(s, '', 'all', 2, 25, 'updated', 'desc');
  assert.equal(page1.rows.length,25);
  assert.equal(page1.rows[0].relation_oid,60);
  assert.equal(page1.rows[24].relation_oid,36);
  assert.equal(page2.rows[0].relation_oid,35);
  assert.equal(avPage(s,'','all',1,25,'hot','desc').rows[0].relation_oid,60);
  s.tables.data[59].tuples_updated = null;
  assert.equal(avPage(s,'','all',1,25,'updated','desc').rows[0].relation_oid,59);
  assert.equal(avPage(s,'','all',3,25,'updated','desc').rows.at(-1).relation_oid,60);
  const disabled = snapshot(124, false);
  disabled.tables.data = rows;
  assert.deepEqual(avPage(disabled,'','all',1,25,'updated','desc').rows.map(r=>r.relation_oid).slice(0,3),[1,2,3]);
});


test('overview separates maintenance thresholds from accumulated DML workload', async () => {
  const {readFileSync}=await import('node:fs');
  const url=new URL('../src/features/autovacuum/AutovacuumView.vue',import.meta.url);
  const view=readFileSync(url,'utf8');
  assert.match(view,/tr\('autovacuumUi\.text038'\)/);
  assert.match(view,/tr\('autovacuumUi\.text039'\)/);
  assert.match(view,/tablePressure\(t, kind\)\?\.observed/);
  assert.match(view,/tablePressure\(t, kind\)\?\.trigger/);
  assert.match(view,/tr\('autovacuumUi\.text083'\)/);
  assert.match(view,/tr\('autovacuumUi\.text065'\)/);
  assert.match(view,/tr\('autovacuumUi\.text046'\)/);
  assert.doesNotMatch(view,/VACUUM · dead <span/);
});

test('simulation explains amount of threshold movement but does not claim IO or scheduling benefit',()=>{
  const t=table(); t.dead_tuples=90; t.dead_tuple_trigger=200;
  const result=avSimulationComparison(t,{resulting_dead_tuple_trigger:100});
  assert.equal(result.thresholdDelta,-100);
  assert.equal(result.thresholdChangePct,-50);
  assert.equal(result.currentRemaining,110);
  assert.equal(result.proposedRemaining,10);
  assert.equal(avSimulationComparison(t,{resulting_dead_tuple_trigger:null}),null);
});
