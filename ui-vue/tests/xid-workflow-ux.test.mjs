import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from 'node:test';
import assert from 'node:assert/strict';
import {createShellState, enterShellWorkspace} from '../dist/core/shell-state.js';
import {createXidController, createXidState} from '../dist/features/xid/controller.js';
import {xidRelationPicker} from '../dist/features/xid/investigation.js';

const conn = {id:1,name:'server',host:'host',port:5432,database:'postgres',username:'reader'};
const brief = id => ({id,connection_id:1,host:'host',port:5432,database:'postgres',
  database_oid:7,username:'reader',sslmode:'prefer',collection_started_at_ms:1000*id});
const snapshot = {overview:{database:'postgres',database_oid:7,username:'reader'},
  relations:{data:[{relation_oid:11,schema_name:'public',relation_name:'test',relation_type:'r'}],error:null}};
function setup(invoke) {
  const shell=createShellState();enterShellWorkspace(shell,'owner');
  const state=createXidState();return {state,controller:createXidController(shell,state,invoke)};
}
test('relation picker filters user/system/TOAST/materialized view and search',()=>{
  const relations = [
    {schema_name:'public',relation_name:'orders',relation_oid:1,relation_type:'r'},
    {schema_name:'pg_catalog',relation_name:'pg_class',relation_oid:2,relation_type:'r'},
    {schema_name:'pg_toast',relation_name:'pg_toast_3',relation_oid:3,relation_type:'t'},
    {schema_name:'public',relation_name:'orders_agg',relation_oid:4,relation_type:'m'},
  ];
  const ids=(scope,query='')=>xidRelationPicker(relations,scope,query).map(r=>r.relation_oid);
  assert.deepEqual(ids('all'),[1,2,3,4]);
  assert.deepEqual(ids('user'),[1,4]);
  assert.deepEqual(ids('system'),[2,3]);
  assert.deepEqual(ids('toast'),[3]);
  assert.deepEqual(ids('other'),[4]);
  assert.deepEqual(ids('all','ORDERS'),[1,4]);
  assert.deepEqual(ids('all','  2 '),[2]);
});
test('refresh buttons expose progress and success notice',async()=>{
  let unblock;const pending=new Promise(resolve=>unblock=resolve);
  const {state,controller}=setup(async cmd=>{
    if(cmd==='list_connections')return pending;
    if(cmd==='list_xid_snapshots')return [];
    throw Error(cmd);
  });
  const process=controller.refreshConnections();
  assert.equal(state.loadingConnections,true);
  assert.match(state.notice,/Обновляем/);
  unblock([conn]);await process;
  assert.equal(state.loadingConnections,false);
  assert.match(state.notice,/Подключения обновлены|История обновлена/);
  await controller.refreshHistory();
  assert.match(state.notice,/История обновлена: 0/);
});
test('historical snapshot is view-only and cannot reach Apply',async()=>{
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push(cmd);
    if(cmd==='list_connections')return [conn];
    if(cmd==='list_xid_snapshots')return [brief(2),brief(1)];
    if(cmd==='load_xid_snapshot')return {history:brief(args.snapshotId),snapshot};
    if(cmd==='xid_relation_history')return [];
    if(cmd==='xid_relation_policy')return {relation_oid:11,candidates:[{id:'c1'}]};
    throw Error(cmd);
  });
  await controller.refreshConnections();
  assert.equal(await controller.openSavedSnapshot(1),true);
  assert.equal(state.historical,true);
  assert.equal(state.summary.id,1);
  await controller.selectRelation(11);
  assert.equal(await controller.apply('c1'),false);
  assert.equal(calls.includes('apply_xid_policy_candidate'),false);
});
test('historical snapshot response from a different profile is rejected',async()=>{
  const {state,controller}=setup(async(cmd,args)=>{
    if(cmd==='list_connections')return [conn];
    if(cmd==='list_xid_snapshots')return [brief(1)];
    if(cmd==='load_xid_snapshot')return {history:{...brief(args.snapshotId),host:'wrong-host'},snapshot};
    throw Error(cmd);
  });
  await controller.refreshConnections();
  assert.equal(await controller.openSavedSnapshot(1),false);
  assert.match(state.error,/другого подключения/);
  assert.equal(state.snapshot,null);
});
