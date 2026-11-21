import assert from 'node:assert/strict';
import test from 'node:test';
import { createShellState, enterShellWorkspace } from '../dist/core/shell-state.js';
import { createIoWalController, createIoWalState, filteredIoWalPage, sameIoWalTarget, ioWalDocsUrl } from '../dist/features/io-wal/controller.js';

const connection = id => ({ id, name:`pg-${id}`, host:`server-${id}`, port:5432, database:'postgres', username:'reader' });
const summary = (id, connectionId=1, dbOid=120) => ({
  id, connection_id:connectionId, connection_name:`pg-${connectionId}`,host:`server-${connectionId}`,port:5432,
  database:'postgres',database_oid:dbOid,username:'reader',sslmode:'disable',server_version_num:160015,
  collection_started_at_ms:id*1000,collection_completed_at_ms:id*1000+20 });
const relation = i => ({ relation_oid:i, schema_name:'public',relation_name:`t_${i}`,relation_type:'table',
  heap_size_bytes:1024,indexes_size_bytes:1024,toast_size_bytes:0,total_size_bytes:2048 });
const snapshot = { overview:{measured_at:'2026-11-11T10:00:00Z',database:'postgres',database_oid:120,server_version:'16.15',server_version_num:160015,
  username:'reader',track_io_timing:false,track_wal_io_timing:false,checkpoint_timeout:'5min',checkpoint_completion_target:0.9,
  max_wal_size:'1GB',min_wal_size:'80MB',block_size:8192},
  database_stats:{data:null,error:'no permission'},wal:{data:{stats_reset:'2026-01-01',wal_bytes:1000},error:null},
  checkpoints:{data:null,error:'no permission'},database_size_bytes:{data:8192,error:null},
  relation_sizes:{data:{total_relation_count:1000,sampled_relation_count:500,truncated:true,
    relations:Array.from({length:500},(_,i)=>relation(i+1))},error:null},
  interpretation:{items:[],limitations:[]} };
const comparison = (a,b)=>({older:a,newer:b,elapsed_seconds:100,
  database_stats_continuous:false,wal_stats_continuous:true,checkpoint_stats_continuous:false,
  wal_bytes_delta:100,wal_bytes_per_hour:3600,temp_bytes_delta:null,temp_bytes_per_hour:null,
  checkpoint_requested_delta:null,database_size_delta_bytes:1024,relation_size_changes:[],limitations:['stats_reset missing']});
function setup(invoke) { const shell=createShellState();enterShellWorkspace(shell,'owner');const state=createIoWalState();return {shell,state,controller:createIoWalController(shell,state,invoke)}; }
const flush=()=>new Promise(resolve=>setImmediate(resolve));
test('Page limits DOM rows and search uses index/oid',()=>{
  const result=filteredIoWalPage(snapshot,'',3,25);
  assert.equal(result.total,500); assert.equal(result.rows.length,25);assert.equal(result.rows[0].relation_oid,51);
  assert.equal(filteredIoWalPage(snapshot,'t_200',1,10).rows[0].relation_oid,200);
});
test('Relation size sorting uses full filtered sample before pagination',()=>{
  const varied={...snapshot,relation_sizes:{...snapshot.relation_sizes,data:{
    ...snapshot.relation_sizes.data,relations:[
      {...relation(1),relation_name:'z',total_size_bytes:100,indexes_size_bytes:90},
      {...relation(2),relation_name:'a',total_size_bytes:500,indexes_size_bytes:10},
      {...relation(3),relation_name:'m',total_size_bytes:300,indexes_size_bytes:60},
    ],
  }}};
  assert.deepEqual(filteredIoWalPage(varied,'',1,25,'total',true).rows.map(r=>r.relation_oid),[2,3,1]);
  assert.deepEqual(filteredIoWalPage(varied,'',1,25,'indexes',true).rows.map(r=>r.relation_oid),[1,3,2]);
  assert.deepEqual(filteredIoWalPage(varied,'',1,25,'name',false).rows.map(r=>r.relation_oid),[2,3,1]);
  assert.equal(filteredIoWalPage(varied,'',2,10,'total',true).page,1);
  assert.deepEqual(filteredIoWalPage(varied,'z',1,10,'total',true).rows.map(r=>r.relation_oid),[1]);
  const many={...varied,relation_sizes:{...varied.relation_sizes,data:{
    ...varied.relation_sizes.data,relations:[...varied.relation_sizes.data.relations,
      ...Array.from({length:10},(_,i)=>({...relation(i+4),total_size_bytes:1000+i}))],
  }}};
  assert.deepEqual(filteredIoWalPage(many,'',2,10,'total',true).rows.map(r=>r.relation_oid),[2,3,1]);
});
test('Documentation links are versioned and prevent arbitrary URLs',()=>{
  assert.match(ioWalDocsUrl(160015,'monitoring-stats.html'),/docs\/16\/monitoring/);
  assert.equal(ioWalDocsUrl(null,'http://evil.test/'),null);
});
test('Comparisons reject different saved sources',()=>{
  assert.equal(sameIoWalTarget(summary(4),summary(3)),true);
  assert.equal(sameIoWalTarget(summary(4),summary(3,2)),false);
  assert.equal(sameIoWalTarget(summary(4),summary(3,1,999)),false);
});
test('Old history response is discarded after connection switch',async()=>{
  let resolve;const pending=new Promise(r=>resolve=r);
  const {state,controller}=setup(async(cmd,args)=>{
    if(cmd==='list_connections')return [connection(1),connection(2)];
    if(cmd==='list_io_wal_snapshots')return args.connectionId===1?pending:[];
    throw Error(cmd);
  });
  await controller.refreshConnections(); controller.selectConnection(2);resolve([summary(2)]);await flush();await flush();
  assert.equal(state.connectionId,2);assert.equal(state.snapshot,null);assert.deepEqual(state.histories,[]);
});
test('Loaded snapshot is bound to connection and contents validated',async()=>{
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push([cmd,args]);
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_io_wal_snapshots')return [summary(3),summary(2)];
    if(cmd==='load_io_wal_snapshot')return snapshot;
    throw Error(cmd);
  });
  await controller.refreshConnections(); await flush();
  assert.equal(state.snapshotId,3);assert.equal(await controller.loadSnapshot(999),false);
  assert.equal(await controller.loadSnapshot(2),true);
  assert.equal(calls.filter(([c])=>c==='load_io_wal_snapshot').every(([,args])=>args.connectionId===1),true);
});
test('Cross target comparison is blocked before Tauri',async()=>{
  const calls=[];
  const {state,controller}=setup(async(cmd,args)=>{
    calls.push(cmd);
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_io_wal_snapshots')return [summary(3),summary(2,1,888)];
    if(cmd==='load_io_wal_snapshot')return snapshot;
    throw Error(cmd);
  });
  await controller.refreshConnections();await flush();const count=calls.length;
  assert.equal(await controller.compare(),false);assert.equal(calls.length,count);
});
test('Stale comparison response is not assigned to new pair',async()=>{
  let resolve;const pending=new Promise(r=>resolve=r);
  const {state,controller}=setup(async(cmd,args)=>{
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_io_wal_snapshots')return [summary(3),summary(2),summary(1)];
    if(cmd==='load_io_wal_snapshot')return snapshot;
    if(cmd==='compare_io_wal_snapshots')return pending;
    throw Error(cmd);
  });
  await controller.refreshConnections();await flush();
  const inFlight=controller.compare();await flush();state.compareOlderId=1;
  resolve(comparison(summary(2),summary(3)));
  assert.equal(await inFlight,false);assert.equal(state.comparison,null);
});
test('Partial section failures are preserved by snapshot view, not fatal',async()=>{
  const {state,controller}=setup(async(cmd,args)=>{
    if(cmd==='list_connections')return [connection(1)];
    if(cmd==='list_io_wal_snapshots')return [summary(3)];
    if(cmd==='load_io_wal_snapshot')return snapshot;
    throw Error(cmd);
  });
  await controller.refreshConnections();await flush();
  assert.equal(state.snapshot.database_stats.error,'no permission');
  assert.equal(state.snapshot.wal.data.wal_bytes,1000);
});
