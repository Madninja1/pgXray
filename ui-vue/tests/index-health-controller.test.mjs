import assert from 'node:assert/strict';
import test from 'node:test';
import { locale } from '../dist/features/i18n/locale-service.js';
locale.value = 'ru';
import { createShellState, enterShellWorkspace } from '../dist/core/shell-state.js';
import { createIndexHealthController, createIndexHealthState, filteredIndexPage, indexDocsUrl, sameIndexTarget, summaryMatchesConnection } from '../dist/features/index-health/controller.js';
const connection = id => ({ id, name:`server-${id}`, host:`server-${id}`, port:5432, database:'app', username:'reader' });
const summary = (id, connectionId=1, databaseOid=123) => ({ id, connection_id:connectionId, host:`server-${connectionId}`, port:5432, database:'app', database_oid:databaseOid, username:'reader', sslmode:'disable', server_version_num:160015, sampled_index_count:1, total_index_count:1, collection_completed_at_ms:id*1000 });
const row = { index_oid:40, relation_oid:10, schema_name:'public', relation_name:'data', index_name:'idx_data', index_size_bytes:512, idx_scan:0, idx_tup_read:0, idx_tup_fetch:0, is_primary:false, is_unique:false, is_valid:true, is_ready:true, key_columns:[], predicate:null, tuples_inserted:0, tuples_updated:0, tuples_deleted:0, hot_updates:0, last_analyze:null, last_autoanalyze:null, classifications:['suspiciously_unused'] };
const snapshot = { overview:{ measured_at:'2026-11-11T00:00:00Z', database:'app', database_oid:123, server_version:'16', server_version_num:160015, stats_reset:null, stats_age_seconds:null, track_counts:true }, collection:{ total_index_count:1, sampled_index_count:1, truncated:false, indexes:[row]}, interpretation:{limitations:[]} };
function setup(invoke) { const shell=createShellState();enterShellWorkspace(shell,'owner');const state=createIndexHealthState();return {shell,state,controller:createIndexHealthController(shell,state,invoke)}; }
async function flush() { await new Promise(resolve=>setImmediate(resolve)); }
test('Index sample pagination avoids rendering all 1000 rows',()=> {
 const s={...snapshot,collection:{...snapshot.collection,indexes:Array.from({length:1000},(_,i)=>({...row,index_oid:i+1,index_name:`idx_${i}`}))}};
 const result=filteredIndexPage(s,{search:'',mode:'all',page:5,pageSize:25});
 assert.equal(result.total,1000); assert.equal(result.rows.length,25); assert.equal(result.rows[0].index_oid,101);
 assert.equal(filteredIndexPage(s,{search:'idx_5',mode:'all',page:1,pageSize:10}).rows.length,10);
});
test('Doc links follow server version and reject malformed paths',()=> {
 assert.match(indexDocsUrl(160015,'catalog-pg-index.html'),/docs\/16\/catalog/);
 assert.match(indexDocsUrl(null,'catalog-pg-index.html'),/docs\/current\/catalog/);
 assert.equal(indexDocsUrl(160015,'https://malicious.invalid/'),null);
});
test('Different connection, database OID or endpoint cannot be compared',()=> {
 assert.equal(sameIndexTarget(summary(1),summary(2)),true);
 assert.equal(sameIndexTarget(summary(1),summary(2,2)),false);
 assert.equal(sameIndexTarget(summary(1),summary(2,1,456)),false);
});
test('Switching connections rejects a late saved history response',async()=> {
 let resolve; const pending=new Promise(res=>resolve=res);
 const {state,controller}=setup(async (cmd,args)=> {
  if(cmd==='list_connections')return [connection(1),connection(2)];
  if(cmd==='list_index_health_snapshots' && args.connectionId===1)return pending;
  if(cmd==='list_index_health_snapshots')return [];
  throw Error(cmd);
 });
 await controller.refreshConnections();
 assert.equal(state.connectionId,1);
 controller.selectConnection(2); resolve([summary(7)]); await flush();await flush();
 assert.equal(state.connectionId,2);
 assert.deepEqual(state.histories,[]); assert.equal(state.snapshot,null);
});
test('Historical snapshot is loaded only with matching connectionId',async()=> {
 const calls=[];
 const {state,controller}=setup(async (cmd,args)=> {
  calls.push([cmd,args]);
  if(cmd==='list_connections')return [connection(1)];
  if(cmd==='list_index_health_snapshots')return [summary(4),summary(3)];
  if(cmd==='load_index_health_snapshot')return snapshot;
  throw Error(cmd);
 });
 await controller.refreshConnections();await flush();
 assert.equal(state.snapshotId,4);assert.equal(state.snapshotConnectionId,1);
 assert.equal(await controller.loadSnapshot(999),false);
 assert.equal(await controller.loadSnapshot(3),true);
 assert.equal(state.snapshotId,3);
 assert.equal(calls.filter(([cmd])=>cmd==='load_index_health_snapshot').every(([,args])=>args.connectionId===1),true);
});
test('History comparison rejects crossed sources before invoking Rust',async()=> {
 const calls=[];
 const {state,controller}=setup(async (cmd,args)=> { calls.push(cmd);if(cmd==='list_connections')return [connection(1)];if(cmd==='list_index_health_snapshots')return [summary(4),summary(3,1,124)];if(cmd==='load_index_health_snapshot')return snapshot;throw Error(cmd); });
 await controller.refreshConnections();await flush();
 const before=calls.length;
 assert.equal(await controller.compare(),false);
 assert.equal(calls.length,before);
 assert.match(state.error,/одной базы/);
});
test('Opening Table Health uses selected connection only and checks relation identity',async()=> {
 const calls=[];
 const {state,controller}=setup(async (cmd,args)=> {calls.push([cmd,args]); if(cmd==='list_connections')return [connection(1)];if(cmd==='list_index_health_snapshots')return [summary(4)];if(cmd==='load_index_health_snapshot')return snapshot;
  if(cmd==='autovacuum_table_detail')return {table:{relation_oid:10,schema_name:'public',relation_name:'data',live_tuples:100,dead_tuples:5,total_size_bytes:2048,last_autovacuum:null},interpretation:{}};
  throw Error(cmd);
 });
 await controller.refreshConnections();await flush();
 assert.equal(await controller.openTableDetail(row),true);
 assert.deepEqual(calls.at(-1),['autovacuum_table_detail',{connectionId:1,relationOid:10}]);
 assert.equal(state.tableDetail.table.relation_oid,10);
});
test('Connection switch while table detail is in flight discards stale response',async()=> {
 let resolve;const pending=new Promise(res=>resolve=res);
 const {state,controller}=setup(async (cmd,args)=> {
  if(cmd==='list_connections')return [connection(1),connection(2)];
  if(cmd==='list_index_health_snapshots')return args.connectionId===1?[summary(4)]:[];
  if(cmd==='load_index_health_snapshot')return snapshot;
  if(cmd==='autovacuum_table_detail')return pending;
  throw Error(cmd);
 });
 await controller.refreshConnections();await flush();
 const request=controller.openTableDetail(row);await flush();controller.selectConnection(2);
 resolve({table:{relation_oid:10,schema_name:'public',relation_name:'data'}});
 assert.equal(await request,false);
 assert.equal(state.tableDetail,null);
});

test('Historical Index Health from a changed saved profile cannot query live Table Health',async()=> {
 const calls=[];
 const {state,controller}=setup(async (cmd,args)=> {
  calls.push(cmd);
  if(cmd==='list_connections')return [{...connection(1),host:'server-1-new'}];
  if(cmd==='list_index_health_snapshots')return [summary(4)];
  if(cmd==='load_index_health_snapshot')return snapshot;
  throw Error(`Unexpected call: ${cmd}`);
 });
 await controller.refreshConnections(); await flush();
 assert.equal(summaryMatchesConnection(state.histories[0],state.connections[0]),false);
 assert.equal(await controller.openTableDetail(row),false);
 assert.equal(calls.includes('autovacuum_table_detail'),false);
 assert.match(state.error,/прежним параметрам/);
});
