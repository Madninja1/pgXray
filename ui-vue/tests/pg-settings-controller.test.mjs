import assert from 'node:assert/strict';
import test from 'node:test';
import { createShellState, enterShellWorkspace } from '../dist/core/shell-state.js';
import { createPgSettingsController, createPgSettingsState, isDue, sameTarget, AUTO_INTERVAL_MS } from '../dist/features/pg-settings/controller.js';
const c = (id) => ({id, name:`server-${id}`, host:'localhost', port:5432, database:'postgres', username:'reader'});
const summary = (id=1, at=1000) => ({id,connection_id:1,host:'localhost',port:5432,database:'postgres',username:'reader',sslmode:'disable',postmaster_start_ms:1,collection_completed_at_ms:at});
function init(invoke) { const shell=createShellState(); enterShellWorkspace(shell,'owner'); const state=createPgSettingsState(); const storage={getItem:()=>null,setItem:()=>{}}; return {shell,state, controller:createPgSettingsController(shell,state,invoke,storage,()=>1000+AUTO_INTERVAL_MS)}; }
test('Auto collection defaults off, and same target requires same server boot',()=>{
  assert.equal(isDue(undefined,c(1),10000),true);
  assert.equal(isDue(summary(),c(1),1001),false);
  assert.equal(isDue(summary(),c(1),1000+AUTO_INTERVAL_MS),true);
  assert.equal(sameTarget(summary(), {...summary(),postmaster_start_ms:2}),false);
});
test('No background collection without explicit opt-in',async()=>{
 const calls=[];const {state,controller}=init(async(cmd)=>{calls.push(cmd);return cmd==='list_connections'?[c(1)]:[]});
 await controller.refreshConnections();await controller.checkSchedule();
 assert.equal(state.autoIds.length,0);assert.equal(calls.includes('collect_pg_settings_snapshot'),false);
});
test('Enabled collection runs and records error without stopping other connections',async()=>{
 const calls=[];const {state,controller}=init(async(cmd,args)=>{calls.push([cmd,args?.connectionId]);if(cmd==='list_connections')return [c(1),c(2)]; if(cmd==='list_pg_settings_snapshots')return [];if(cmd==='list_pg_settings_changes')return [];if(cmd==='collect_pg_settings_snapshot' && args.connectionId===1)throw Error('offline');return {}});
 await controller.refreshConnections();controller.setAuto(1,true);controller.setAuto(2,true);await controller.checkSchedule();
 assert.match(state.autoLastError[1],/offline/);assert.equal(calls.filter(x=>x[0]==='collect_pg_settings_snapshot').length,2);
});
test('Switching connection rejects stale history response',async()=>{
 let release; const pending=new Promise(res=>{release=res;});
 const {controller,state}=init(async(cmd,args)=>cmd==='list_connections'?[c(1),c(2)]:cmd==='list_pg_settings_snapshots' && args.connectionId===1?pending:[]);
 // Avoid waiting for initial background refresh.
 const initial=controller.refreshConnections(); await new Promise(res=>setImmediate(res));
 controller.selectConnection(2); release([summary()]); await initial;
 assert.equal(state.connectionId,2); assert.equal(state.summaries.some(s=>s.connection_id===1),false);
});
