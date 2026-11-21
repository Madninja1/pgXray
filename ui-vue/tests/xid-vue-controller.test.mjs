import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from 'node:assert/strict';
import test from 'node:test';
import { createShellState, enterShellWorkspace } from '../dist/core/shell-state.js';
import { createXidController, createXidState, xidPage, xidDocsUrl, xidSectionStatus,
  sameXidTarget, xidSortRows, xidIsSystemSchema } from '../dist/features/xid/controller.js';

const connection = id => ({id,name:`db-${id}`,host:`host-${id}`,port:5432,database:'postgres',username:'reader'});
const summary = (id,cid=1) => ({id,connection_id:cid,host:`host-${cid}`,port:5432,
  database:'postgres',database_oid:7,username:'reader',sslmode:'prefer',
  collection_started_at_ms:id*1000});
const relation = oid => ({relation_oid:oid,schema_name:'public',relation_name:`tbl_${oid}`,relation_type:'r',
  xid_age:120000000,effective_freeze_max_age:200000000});
const snapshot = {overview:{database:'postgres',database_oid:7,username:'reader',server_version_num:160015},
  relations:{data:[relation(11),relation(12)],error:null},
  vacuums:{data:null,error:'permission denied'},
  interpretation:{items:[],limitations:[]}};
const advice = {relation_oid:11,status:'review',candidates:[{id:'candidate-1',title:'freeze',
  parameter:'autovacuum_freeze_max_age',current_value:200000000,candidate_value:180000000,ddl_target:'public.tbl_11',sql:'ALTER TABLE ...',why:[],tradeoffs:[],expected_effect:[]}]};
const flush = () => new Promise(resolve => setImmediate(resolve));
function setup(invoke) {const shell=createShellState();enterShellWorkspace(shell,'owner');
  const state=createXidState();return {shell,state,controller:createXidController(shell,state,invoke)};}

test('XID sections distinguish unavailable, empty and available (UI-01)',()=>{
  assert.equal(xidSectionStatus({data:null,error:'permission denied'}),'unavailable');
  assert.equal(xidSectionStatus({data:[],error:null}),'empty');
  assert.equal(xidSectionStatus({data:[1],error:null}),'available');
  assert.equal(xidSectionStatus(null),'unavailable');
});
test('XID page is bounded and documentation links reject arbitrary URLs',()=>{
  const values=Array.from({length:999},(_,i)=>i);
  const page=xidPage(values,2,20);assert.equal(page.rows.length,20);assert.equal(page.rows[0],20);
  assert.equal(xidPage(values,1,9999).rows.length,20);
  assert.match(xidDocsUrl(160015,'routine-vacuuming.html'),/docs\/16\/routine-vacuuming/);
  assert.equal(xidDocsUrl(160015,'https://evil.invalid/'),null);
});
test('Comparison source validation includes SSL profile and OID',()=>{
  assert.equal(sameXidTarget(summary(1),summary(2)),true);
  assert.equal(sameXidTarget(summary(1),summary(2,2)),false);
  assert.equal(sameXidTarget(summary(1),{...summary(2),sslmode:'require'}),false);
  assert.equal(sameXidTarget(summary(1),{...summary(2),database_oid:99}),false);
});
test('A stale history response cannot replace another connection history (UI-02)',async()=>{
  let resolve;const pending=new Promise(r=>resolve=r);
  const {state,controller}=setup(async(cmd,args)=>{
    if(cmd==='list_connections')return [connection(1),connection(2)];
    if(cmd==='list_xid_snapshots')return args.connectionId===1?pending:[];
    throw Error(cmd);
  });
  await controller.refreshConnections();controller.selectConnection(2);resolve([summary(1)]);
  await flush();await flush();
  assert.equal(state.connectionId,2);assert.deepEqual(state.history,[]);
});
test('Stale relation response is discarded across snapshot changes (UI-02)',async()=>{
  let release;const pending=new Promise(r=>release=r);
  const {state,controller}=setup(async(cmd,args)=>{
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_xid_snapshots')return [];
    if(cmd==='collect_xid_snapshot')return {history:summary(101),snapshot};
    if(cmd==='xid_relation_history')return pending;
    if(cmd==='xid_relation_policy')return advice;
    throw Error(cmd);
  });
  await controller.refreshConnections();await controller.collect();
  const old=controller.selectRelation(11);await flush();
  await controller.collect();release([{snapshot_id:101,relation:relation(11)}]);await old;
  assert.equal(state.selectedRelationOid,null);assert.deepEqual(state.relationHistory,[]);
  assert.equal(state.policy,null);
});
test('An old comparison is invalidated when pair selection changes (UI-02)',async()=>{
  let release;const pending=new Promise(r=>release=r);
  const {state,controller}=setup(async(cmd)=>{
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_xid_snapshots')return [summary(3),summary(2),summary(1)];
    if(cmd==='compare_xid_snapshots')return pending;
    throw Error(cmd);
  });
  await controller.refreshConnections();await flush();
  const inflight=controller.compare();controller.selectCompare(1,3);
  release({older:summary(2),newer:summary(3)});
  assert.equal(await inflight,false);assert.equal(state.comparison,null);
});
test('Apply requires candidate in current advice and sends only backend ID',async()=>{
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push([cmd,args]);
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_xid_snapshots')return [];
    if(cmd==='collect_xid_snapshot')return {history:summary(10),snapshot};
    if(cmd==='xid_relation_history')return [];
    if(cmd==='xid_relation_policy')return advice;
    if(cmd==='apply_xid_policy_candidate')return {candidate:advice.candidates[0],
      verification:{candidate_id:'candidate-1'},history:null,snapshot:null,
      warning:'Post-change snapshot not saved'};
    throw Error(cmd);
  });
  await controller.refreshConnections();await controller.collect();await controller.selectRelation(11);
  assert.equal(await controller.apply('untrusted'),false);
  assert.equal(calls.some(([cmd])=>cmd==='apply_xid_policy_candidate'),false);
  assert.equal(await controller.apply('candidate-1'),true);
  const applyCalls=calls.filter(([cmd])=>cmd==='apply_xid_policy_candidate');
  assert.deepEqual(applyCalls[0][1],{connectionId:1,snapshotId:10,relationOid:11,candidateId:'candidate-1'});
  assert.equal(state.snapshot,null);assert.equal(state.policy,null);
  assert.match(state.notice,/снимок не сохранён/);
});
test('Profile mismatch blocks Policy Apply before backend invocation',async()=>{
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push(cmd);
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_xid_snapshots')return [];
    if(cmd==='collect_xid_snapshot')return {history:{...summary(10),host:'old-host'},snapshot};
    if(cmd==='xid_relation_history')return [];
    if(cmd==='xid_relation_policy')return advice;
    throw Error(cmd);
  });
  await controller.refreshConnections();await controller.collect();await controller.selectRelation(11);
  assert.equal(await controller.apply('candidate-1'),false);
  assert.equal(calls.includes('apply_xid_policy_candidate'),false);
});


test('XID sorting is numeric, stable, does not mutate rows and places missing values last', () => {
  const rows = [
    {id:1, age:10}, {id:2,age:null}, {id:3,age:100},
    {id:4,age:10}, {id:5,age:2}, {id:6,age:undefined},
  ];
  assert.deepEqual(xidSortRows(rows,r=>r.age,'desc').map(r=>r.id),[3,1,4,5,2,6]);
  assert.deepEqual(xidSortRows(rows,r=>r.age,'asc').map(r=>r.id),[5,1,4,3,2,6]);
  assert.deepEqual(rows.map(r=>r.id),[1,2,3,4,5,6]);
});
test('XID sorting precedes pagination (not just the visible 20 rows)', () => {
  const rows = Array.from({length:334},(_,i)=>({id:i, age:i}));
  const sorted = xidSortRows(rows,r=>r.age,'desc');
  const first = xidPage(sorted,1,20);
  assert.equal(first.rows[0].age,333);
  assert.equal(first.rows.at(-1).age,314);
  assert.equal(first.pages,17);
});
test('XID name sorting supports numeric OIDs and schema scope is explicit', () => {
  const rows = ['pg_catalog.tbl_10','public.tbl_2','public.tbl_11'].map(name=>({name}));
  assert.deepEqual(xidSortRows(rows,r=>r.name,'asc').map(r=>r.name),[
    'pg_catalog.tbl_10','public.tbl_2','public.tbl_11',
  ]);
  assert.equal(xidIsSystemSchema('pg_catalog'),true);
  assert.equal(xidIsSystemSchema('pg_toast'),true);
  assert.equal(xidIsSystemSchema('pg_temp_1'),true);
  assert.equal(xidIsSystemSchema('information_schema'),true);
  assert.equal(xidIsSystemSchema('public'),false);
});


test('XID entry restores the newest saved snapshot without collecting from PostgreSQL', async () => {
  const calls = [];
  const {state,controller} = setup(async (cmd,args) => {
    calls.push([cmd,args]);
    if (cmd === 'list_connections') return [connection(1)];
    if (cmd === 'list_xid_snapshots') return [summary(18),summary(17)];
    if (cmd === 'load_xid_snapshot') return {history:summary(args.snapshotId),snapshot};
    throw Error(cmd);
  });
  await controller.refreshConnections(true);
  assert.equal(state.connectionId,1);
  assert.equal(state.summary?.id,18);
  assert.equal(state.snapshot,snapshot);
  assert.equal(state.historical,true);
  assert.deepEqual(calls.filter(([cmd])=>cmd === 'load_xid_snapshot').map(([,args])=>args.snapshotId),[18]);
  assert.equal(calls.some(([cmd])=>cmd === 'collect_xid_snapshot'),false);
  assert.match(state.notice,/Архивный снимок #18/);
  await controller.refreshConnections(true);
  assert.equal(state.summary?.id,18);
  assert.equal(calls.some(([cmd])=>cmd === 'collect_xid_snapshot'),false);
});

test('XID selection opens the newest saved snapshot for the newly selected connection', async () => {
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push([cmd,args]);
    if(cmd==='list_connections')return [connection(1),connection(2)];
    if(cmd==='list_xid_snapshots')return args.connectionId===1?[summary(11)]:[summary(28,2),summary(27,2)];
    if(cmd==='load_xid_snapshot')return {history:summary(args.snapshotId,args.connectionId),snapshot};
    throw Error(cmd);
  });
  await controller.refreshConnections(true);
  assert.equal(state.summary?.id,11);
  controller.selectConnection(2);
  await flush();await flush();
  assert.equal(state.connectionId,2);
  assert.equal(state.summary?.id,28);
  assert.equal(state.historical,true);
  assert.deepEqual(calls.filter(([cmd])=>cmd==='load_xid_snapshot').map(([,args])=>[args.connectionId,args.snapshotId]),[[1,11],[2,28]]);
  assert.equal(calls.some(([cmd])=>cmd==='collect_xid_snapshot'),false);
});

test('XID history navigation honors a preferred connection before any snapshot load',async()=>{
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push([cmd,args]);
    if(cmd==='list_connections')return [connection(1),connection(2)];
    if(cmd==='list_xid_snapshots')return [summary(args.connectionId===2?88:77,args.connectionId)];
    if(cmd==='load_xid_snapshot')return {history:summary(args.snapshotId,args.connectionId),snapshot};
    throw Error(cmd);
  });
  await controller.refreshConnections(true,2);
  assert.equal(state.connectionId,2);
  assert.equal(state.summary?.id,88);
  assert.deepEqual(calls.filter(([cmd])=>cmd==='list_xid_snapshots').map(([,args])=>args.connectionId),[2]);
  assert.equal(calls.some(([cmd])=>cmd==='collect_xid_snapshot'),false);
});

test('XID entry never opens an archive snapshot from an obsolete connection profile',async()=>{
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push(cmd);
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_xid_snapshots')return [{...summary(5),host:'old-profile'}];
    throw Error(cmd);
  });
  await controller.refreshConnections(true);
  assert.equal(state.snapshot,null);
  assert.equal(state.summary,null);
  assert.match(state.notice,/текущего профиля/);
  assert.equal(calls.includes('load_xid_snapshot'),false);
});

test('XID failed archive loading clears the previous displayed snapshot',async()=>{
  let fail=false;
  const {state,controller}=setup(async(cmd,args)=>{
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_xid_snapshots')return [summary(fail?19:18)];
    if(cmd==='load_xid_snapshot') {
      if(fail)throw Error('missing from history store');
      return {history:summary(args.snapshotId),snapshot};
    }
    throw Error(cmd);
  });
  await controller.refreshConnections(true);
  assert.equal(state.summary?.id,18);
  fail=true;
  await controller.refreshConnections(true);
  assert.equal(state.snapshot,null);
  assert.equal(state.summary,null);
  assert.match(state.error,/missing from history store/);
});
