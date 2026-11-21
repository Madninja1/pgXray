import assert from 'node:assert/strict';
import test from 'node:test';
import { settingsChanges, settingsIntervalIsComparable, settingsParameterHistory, settingsValue } from '../dist/features/history/settings-insights.js';
const change=(overrides={})=>({name:'work_mem',category:'Resource Usage',state:'changed',older_value:'4',newer_value:'8',older_unit:'MB',newer_unit:'MB',older_source:'default',newer_source:'database',older_sourcefile:null,newer_sourcefile:'/hidden/path',older_sourceline:null,newer_sourceline:3,older_pending_restart:false,newer_pending_restart:true,value_changed:true,reset_value_changed:false,source_changed:true,pending_restart_changed:true,...overrides});
const interval=(state,changes=[])=>({from_snapshot_id:1,to_snapshot_id:2,from_ms:1000,to_ms:2000,state,reason:null,value_change_count:1,source_change_count:1,pending_restart_change_count:1,changes});
test('server restart remains comparable for setting values but target switch does not',()=>{
 assert.equal(settingsIntervalIsComparable(interval('server_restarted')),true);
 assert.equal(settingsIntervalIsComparable(interval('target_changed')),false);
 assert.deepEqual(settingsChanges(interval('target_changed',[change()]),'all'),[]);
});
test('history filters value, sources, restart and name without exposing values',()=>{
 const row=interval('comparable',[change(),change({name:'shared_buffers',value_changed:false,source_changed:true,newer_pending_restart:false,pending_restart_changed:false})]);
 assert.deepEqual(settingsChanges(row,'restart').map(x=>x.name),['work_mem']);
 assert.deepEqual(settingsChanges(row,'value').map(x=>x.name),['work_mem']);
 assert.deepEqual(settingsChanges(row,'source','shared').map(x=>x.name),['shared_buffers']);
 assert.equal(settingsValue('private',null,false),'••••');
 assert.equal(settingsValue('4','MB',true),'4 MB');
});
test('history of parameter skips incomparable intervals and other parameters',()=>{
 const data={snapshot_count:3,limitations:[],intervals:[interval('target_changed',[change()]),interval('comparable',[change(),change({name:'other'})])]};
 assert.equal(settingsParameterHistory(data,'work_mem').length,1);
});
