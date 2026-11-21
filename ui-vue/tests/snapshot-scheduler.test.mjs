import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from '../dist/core/shell-state.js';
import { createSnapshotScheduler, createSnapshotSchedulerState } from '../dist/features/snapshots/scheduler.js';
import { makeSchedule, snapshotTargetKey, inCaptureWindow, validateSchedule, seriesExpired, scheduleDue, nextIntervalSlot, upgradeSavedSchedule } from '../dist/features/snapshots/schedule-model.js';

const target = (id=1) => ({id, name:`pg-${id}`, host:'db.internal', port:5432, database:'postgres', username:'reader',sslmode:'require'});
const draft = (more={}) => ({connectionId:1,kind:'pgss',mode:'interval',intervalMinutes:60,windowStart:'00:00',windowEnd:'00:00',intervalAnchor:'09:00',daysOfWeek:[0,1,2,3,4,5,6],dailyTime:'09:00',seriesCount:3,seriesSpanMinutes:60,...more});
const slotTime = (day=17,hours=9,minutes=0) => new Date(2026,10,day,hours,minutes).getTime();
const settle = () => new Promise(resolve=>setImmediate(resolve));
const fakeBackend = () => {
  const rows = new Map();
  const claims = new Set();
  const forOwner = owner => rows.get(owner) ?? new Map();
  return {
    rows, claims,
    seed(owner, rules) { rows.set(owner, new Map(rules.map(r=>[r.key,{...r,dbRevision:r.dbRevision??1}]))); },
    async invoke(cmd,args={},owner='dba') {
      const state=forOwner(owner);
      if(cmd==='list_snapshot_schedules') return [...state.values()];
      if(cmd==='put_snapshot_schedule') {
        const {rule,expectedRevision}=args;
        const old=state.get(rule.key);
        if(expectedRevision===null ? old!==undefined : !old||old.dbRevision!==expectedRevision)
          throw Error('Расписание изменено в другом экземпляре');
        if(!old&&state.size>=60)throw Error('Максимум 60');
        const value={...rule,dbRevision:(old?.dbRevision??0)+1};
        state.set(rule.key,value);rows.set(owner,state);return value;
      }
      if(cmd==='remove_snapshot_schedule') {
        if(state.get(args.key)?.dbRevision!==args.expectedRevision)throw Error('Конфликт');
        state.delete(args.key);return null;
      }
      if(cmd==='claim_snapshot_schedule') {
        const old=state.get(args.rule.key);
        const claim=`${args.rule.connectionId}:${args.rule.kind}:${args.slotAtMs}`;
        if(!old||old.dbRevision!==args.expectedRevision||!old.enabled||old.nextAt!==args.slotAtMs||claims.has(claim))return null;
        claims.add(claim);
        const value={...args.rule,dbRevision:old.dbRevision+1};state.set(args.rule.key,value);return value;
      }
      if(cmd==='history_maintenance_tick')return {action:'scheduled',deleted:0};
      return undefined;
    },
  };
};
function setup(now=slotTime(), handler=async cmd => cmd==='list_connections'?[target()]:{}, backend=fakeBackend()) {
  let t=now;
  const shell=createShellState();enterShellWorkspace(shell,'dba');
  const state=createSnapshotSchedulerState();
  const calls=[];
  const actions=createSnapshotScheduler(shell,state,async(cmd,args)=>{
    calls.push([cmd,args]);
    if(['list_snapshot_schedules','put_snapshot_schedule','remove_snapshot_schedule',
      'claim_snapshot_schedule','history_maintenance_tick'].includes(cmd))
      return backend.invoke(cmd,args,shell.currentUsername??'dba');
    return handler(cmd,args);
  },()=>t);
  return {shell,state,store:backend,actions,calls,advance:ms=>{t+=ms;},setTime:value=>{t=value;}};
}

test('Disabled by default; schedule validation limits heavy collectors and series rate', async()=>{
  const s=setup(); await s.actions.syncAuth(); await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd.startsWith('collect_')).length,0);
  assert.throws(()=>validateSchedule(draft({intervalMinutes:15})),/Интервал/);
  assert.throws(()=>validateSchedule(draft({mode:'series',seriesCount:3,seriesSpanMinutes:30})),/не менее 30/);
  assert.throws(()=>validateSchedule(draft({kind:'index_health',mode:'series',seriesCount:3,seriesSpanMinutes:60})),/не менее 60/);
  assert.doesNotThrow(()=>validateSchedule(draft({mode:'series'})));
});

test('Fingerprint includes TLS and rejects changed connection before issuing SQL', async()=>{
  const started=slotTime();
  const x=setup(started,async cmd=>cmd==='list_connections'?[{...target(),sslmode:'verify-full'}]:{});
  const persisted=makeSchedule(draft(),target(),started);
  x.store.seed('dba',[persisted]);
  await x.actions.syncAuth();
  assert.notEqual(persisted.fingerprint,snapshotTargetKey({...target(),sslmode:'verify-full'}));
  assert.equal(x.calls.some(([cmd])=>cmd==='collect_pgss_snapshot'),false);
  assert.equal(x.state.rules[0].enabled,false);
  assert.match(x.state.rules[0].lastError,/изменился/);
});

test('Recurring schedule collects once, saves next attempt and does not catch up burst',async()=>{
  const start=slotTime();const s=setup(start);
  await s.actions.syncAuth(); assert.equal(await s.actions.saveRule(draft()),true);
  await settle(); await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,1);
  assert.equal(s.state.rules[0].lastSuccessAt,start);
  await s.actions.check(); assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,1);
  s.advance(4*60*60_000); await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,2,'only current slot is collected, no backfill burst');
  await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,2);
});

test('Series 3 snapshots in 60 minutes: zero, midpoint and end; then stop',async()=>{
  const s=setup(slotTime()); await s.actions.syncAuth();await s.actions.saveRule(draft({mode:'series'}));
  await settle(); await s.actions.check();assert.equal(s.state.rules[0].seriesCompleted,1);
  s.advance(30*60_000-1000);await s.actions.check();assert.equal(s.state.rules[0].seriesCompleted,1);
  s.advance(1000);await s.actions.check();assert.equal(s.state.rules[0].seriesCompleted,2);
  s.advance(30*60_000);await s.actions.check();
  assert.equal(s.state.rules[0].seriesCompleted,3);
  assert.equal(s.state.rules[0].status,'complete');
  assert.equal(s.state.rules[0].enabled,false);
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,3);
});

test('Expired series never backfills missed runs after app is reopened',async()=>{
  const started=slotTime();
  const s=setup(started+75*60_000);
  const saved=makeSchedule(draft({mode:'series'}),target(),started);
  s.store.seed('dba',[saved]);
  await s.actions.syncAuth();
  assert.equal(s.calls.some(([cmd])=>cmd==='collect_pgss_snapshot'),false);
  assert.equal(s.state.rules[0].status,'expired');
  assert.equal(s.state.rules[0].seriesCompleted,0);
});

test('No capture after logout; account settings are isolated',async()=>{
  const s=setup();await s.actions.syncAuth();await s.actions.saveRule(draft());
  leaveShellWorkspace(s.shell);await s.actions.syncAuth();await s.actions.check();
  assert.equal(s.calls.some(([cmd])=>cmd==='collect_pgss_snapshot'),false);
  enterShellWorkspace(s.shell,'other');await s.actions.syncAuth();
  assert.equal(s.state.rules.length,0);
});

test('Failed captures have persistent error and a future retry, no tight loop',async()=>{
  const s=setup(slotTime(),async cmd=>{if(cmd==='list_connections')return [target()];if(cmd==='collect_pgss_snapshot')throw Error('database unavailable');return {};});
  await s.actions.syncAuth();await s.actions.saveRule(draft());await settle();await s.actions.check();
  assert.match(s.state.rules[0].lastError,/database unavailable/);
  await s.actions.check();assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,1);
  assert.equal(s.store.rows.get('dba').size,1);
});

test('Overnight windows and daily clock use local time instead of uptime',()=>{
  const local=(hours,minutes)=>new Date(2026,10,15,hours,minutes).getTime();
  assert.equal(inCaptureWindow(local(23,0),'22:00','06:00'),true);
  assert.equal(inCaptureWindow(local(3,0),'22:00','06:00'),true);
  assert.equal(inCaptureWindow(local(12,0),'22:00','06:00'),false);
  const rule=makeSchedule(draft({mode:'daily',dailyTime:'09:00'}),target(),local(12,0));
  assert.equal(new Date(rule.nextAt).getDate(),16);
  assert.equal(new Date(rule.nextAt).getHours(),9);
});

test('Daily schedules skip missed wall-clock slots rather than collecting after startup', async()=>{
  const local=(day,hours,minutes)=>new Date(2026,10,day,hours,minutes).getTime();
  const s=setup(local(16,14,0));
  const saved=makeSchedule(draft({mode:'daily',dailyTime:'09:00'}),target(),local(15,8,0));
  s.store.seed('dba',[saved]);
  await s.actions.syncAuth();
  assert.equal(s.calls.some(([cmd])=>cmd==='collect_pgss_snapshot'),false);
  assert.equal(new Date(s.state.rules[0].nextAt).getDate(),17);
  assert.equal(new Date(s.state.rules[0].nextAt).getHours(),9);
});

test('Interval outside active hours waits for the next window',async()=>{
  const local=(hours,minutes)=>new Date(2026,10,15,hours,minutes).getTime();
  const s=setup(local(12,0));
  const saved=makeSchedule(draft({windowStart:'20:00',windowEnd:'23:00',intervalAnchor:'20:00'}),target(),local(11,0));
  s.store.seed('dba',[saved]);
  await s.actions.syncAuth();
  assert.equal(s.calls.some(([cmd])=>cmd==='collect_pgss_snapshot'),false);
  assert.equal(new Date(s.state.rules[0].nextAt).getHours(),20);
  s.setTime(local(20,0));await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,1);
});

test('Unconfirmed operation after restart is paused instead of duplicated',async()=>{
  const started=slotTime();const s=setup(started+35*60_000);
  const saved=makeSchedule(draft(),target(),started);
  saved.lastAttemptAt=started;
  saved.lastSuccessAt=null;
  saved.lastError=null;
  saved.nextAt=started+60*60_000;
  s.store.seed('dba',[saved]);
  await s.actions.syncAuth();
  assert.equal(s.calls.some(([cmd])=>cmd==='collect_pgss_snapshot'),false);
  assert.equal(s.state.rules[0].enabled,false);
  assert.match(s.state.rules[0].lastError,/не подтверждён/);
});

test('Vue integration has one centralized timer, no duplicate Configuration Drift scheduler',()=>{
  const app=readFileSync(new URL('../src/App.vue',import.meta.url),'utf8');
  const settings=readFileSync(new URL('../src/features/settings/SettingsView.vue',import.meta.url),'utf8');
  const config=readFileSync(new URL('../src/features/pg-settings/PgSettingsView.vue',import.meta.url),'utf8');
  assert.match(app,/snapshotActions\.start\(\)/);
  assert.match(settings,/SnapshotSchedulesView/);
  assert.doesNotMatch(config,/setInterval\(/);
  assert.doesNotMatch(config,/controller\.checkSchedule\(/);
});


test('Anchored slots match requested 09:05/11:05 and never propose 21:51 outside 08–21', () => {
  const xid = draft({kind:'xid',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05'});
  const pgss = draft({intervalMinutes:60,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:03'});
  const times = (rule, from, count) => {
    const result=[];let at=from;
    for(let i=0;i<count;i++){at=nextIntervalSlot(at,rule,true);result.push(new Date(at).getHours().toString().padStart(2,'0')+':'+new Date(at).getMinutes().toString().padStart(2,'0'));at++;}
    return result;
  };
  assert.deepEqual(times(xid,slotTime(17,8,0),6),['09:05','11:05','13:05','15:05','17:05','19:05']);
  assert.deepEqual(times(pgss,slotTime(17,8,0),4),['09:03','10:03','11:03','12:03']);
  const next = nextIntervalSlot(slotTime(17,19,51),xid,true);
  assert.equal(new Date(next).getDate(),18);
  assert.equal(new Date(next).getHours(),9);
  assert.equal(new Date(next).getMinutes(),5);
  assert.ok(!inCaptureWindow(slotTime(17,21,51),xid.windowStart,xid.windowEnd));
});

test('Actual 09:05:42 start does not shift the following slot to 11:05:42', async () => {
  const s=setup(slotTime(17,9,5));await s.actions.syncAuth();
  assert.equal(await s.actions.saveRule(draft({kind:'xid',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05'})),true);
  s.advance(42_000); await settle(); await s.actions.check();
  const runs=s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot');
  assert.equal(runs.length,1);
  assert.equal(s.state.rules[0].nextAt,slotTime(17,11,5));
  s.setTime(slotTime(17,11,5));await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot').length,2);
});

test('Late or overnight missed slots are skipped, not run outside window',async()=>{
  const s=setup(slotTime(17,19,5));await s.actions.syncAuth();
  await s.actions.saveRule(draft({kind:'xid',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05'}));
  await settle();await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot').length,1);
  assert.equal(s.state.rules[0].nextAt,slotTime(18,9,5));
  s.setTime(slotTime(17,21,51));await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot').length,1);
  assert.equal(s.state.rules[0].nextAt,slotTime(18,9,5));
  s.setTime(slotTime(18,9,9));await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot').length,1);
  assert.equal(s.state.rules[0].nextAt,slotTime(18,11,5));
});

test('Calendar respects selected weekdays and overnight windows',()=>{
  const weekday=draft({kind:'xid',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05',daysOfWeek:[1,3,5]});
  const next=nextIntervalSlot(slotTime(17,12,0),weekday,true); // Tue -> Wed
  assert.equal(new Date(next).getDay(),3);
  assert.equal(new Date(next).getHours(),9);
  const night=draft({kind:'xid',intervalMinutes:120,windowStart:'22:00',windowEnd:'06:00',intervalAnchor:'22:05',daysOfWeek:[1]});
  const first=nextIntervalSlot(slotTime(16,22,0),night,true);
  const second=nextIntervalSlot(first,night);
  const third=nextIntervalSlot(second,night);
  assert.equal(new Date(first).getHours(),22);
  assert.equal(new Date(second).getHours(),0);
  assert.equal(new Date(second).getDay(),2); // Tue morning belongs to Monday window
  assert.equal(new Date(third).getHours(),2);
  const after=nextIntervalSlot(slotTime(17,5,59),night,true);
  assert.equal(new Date(after).getDay(),1); // Next Monday
});

test('Invalid anchor or empty weekday set cannot be saved',()=>{
  assert.throws(()=>validateSchedule(draft({daysOfWeek:[]})),/день недели/);
  assert.throws(()=>validateSchedule(draft({windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'21:51'})),/Первый слот/);
  assert.throws(()=>validateSchedule(draft({windowStart:'22:00',windowEnd:'06:00',intervalAnchor:'01:05'})),/Первый слот/);
});


test('Independent modules keep separate minute offsets without a shared drifting timer', async()=>{
  const s=setup(slotTime(17,8,59));await s.actions.syncAuth();
  await s.actions.saveRule(draft({kind:'xid',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05'}));
  await s.actions.saveRule(draft({kind:'pgss',intervalMinutes:60,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:03'}));
  await settle();
  s.setTime(slotTime(17,9,3));await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_pgss_snapshot').length,1);
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot').length,0);
  s.setTime(slotTime(17,9,5));await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot').length,1);
  assert.equal(s.state.rules.find(r=>r.kind==='pgss').nextAt,slotTime(17,10,3));
  assert.equal(s.state.rules.find(r=>r.kind==='xid').nextAt,slotTime(17,11,5));
});

test('Multiple schedules for one connection and kind are independent and persist across login',async()=>{
  const s=setup(slotTime(17,8,59));await s.actions.syncAuth();
  assert.equal(await s.actions.saveRule(draft({kind:'xid',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05'})),true);
  assert.equal(await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'19:05'})),true);
  assert.equal(s.state.rules.length,2);
  const keys=s.state.rules.map(r=>r.key);
  assert.equal(new Set(keys).size,2);
  assert.ok(keys.every(k=>k.startsWith('1:xid:')));
  const restored=setup(slotTime(17,8,59));
  restored.store.seed('dba',s.state.rules);
  await restored.actions.syncAuth();
  assert.equal(restored.state.rules.length,2);
  assert.deepEqual(restored.state.rules.map(r=>r.key),keys);
  assert.equal(restored.calls.some(([cmd])=>cmd==='collect_xid_snapshot'),false);
});

test('Editing an existing schedule modifies only its own entry',async()=>{
  const s=setup(slotTime(17,8,59));await s.actions.syncAuth();
  await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'09:05'}));
  await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'18:05'}));
  const first=s.state.rules.find(r=>r.dailyTime==='09:05');
  const other=s.state.rules.find(r=>r.dailyTime==='18:05');
  assert.equal(await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'10:05'}),first.key),true);
  assert.equal(s.state.rules.length,2);
  assert.equal(s.state.rules.find(r=>r.key===first.key).dailyTime,'10:05');
  assert.equal(s.state.rules.find(r=>r.key===other.key).dailyTime,'18:05');
  assert.equal(await s.actions.saveRule(draft({kind:'pgss',mode:'daily',dailyTime:'10:05'}),first.key),false);
  assert.match(s.state.error,/новое расписание/);
  assert.equal(s.state.rules.length,2);
  assert.equal(await s.actions.removeRule(first.key),true);
  assert.equal(s.state.rules.length,1);
  assert.equal(s.state.rules[0].key,other.key);
});

test('Same-millisecond saves get unique keys and identical active schedule is refused',async()=>{
  const s=setup(slotTime(17,8,59));await s.actions.syncAuth();
  assert.equal(await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'09:05'})),true);
  assert.equal(await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'10:05'})),true);
  assert.equal(s.state.rules.length,2);
  assert.equal(new Set(s.state.rules.map(r=>r.key)).size,2);
  assert.equal(await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'09:05'})),false);
  assert.match(s.state.error,/уже активно/);
  assert.equal(s.state.rules.length,2);
});

test('Coincident slots of different rules for same module collect only once',async()=>{
  const s=setup(slotTime(17,8,59));await s.actions.syncAuth();
  assert.equal(await s.actions.saveRule(draft({kind:'xid',mode:'interval',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05'})),true);
  assert.equal(await s.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'09:05'})),true);
  s.setTime(slotTime(17,9,5));await s.actions.check();await s.actions.check();
  assert.equal(s.calls.filter(([cmd])=>cmd==='collect_xid_snapshot').length,1);
  assert.equal(s.state.rules.filter(r=>r.lastError?.includes('Совпадающий слот')).length,1);
  assert.equal(s.state.rules.every(r=>r.enabled),true);
  assert.equal(s.state.rules.every(r=>r.nextAt>slotTime(17,9,5)),true);
});

test('Two processes sharing SQLite perform each slot at most once',async()=>{
  const db=fakeBackend();const at=slotTime(17,9,5);
  const first=setup(at,async cmd=>cmd==='list_connections'?[target()]:{},db);
  const second=setup(at,async cmd=>cmd==='list_connections'?[target()]:{},db);
  await first.actions.syncAuth();await second.actions.syncAuth();
  await first.actions.saveRule(draft({kind:'xid',intervalMinutes:120,windowStart:'08:00',windowEnd:'21:00',intervalAnchor:'09:05'}));
  await Promise.all([first.actions.check(),second.actions.check()]);
  const count=[...first.calls,...second.calls].filter(([name])=>name==='collect_xid_snapshot').length;
  assert.equal(count,1);
  assert.equal(db.claims.size,1);
});
test('SQLite optimistic revision prevents silent overwrites by another process',async()=>{
  const db=fakeBackend();const first=setup(slotTime(17,8,59),undefined,db);
  const second=setup(slotTime(17,8,59),undefined,db);
  await first.actions.syncAuth();await first.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'19:05'}));
  await second.actions.syncAuth();
  const key=first.state.rules[0].key;
  await first.actions.saveRule(draft({kind:'xid',mode:'daily',dailyTime:'20:05'}),key);
  assert.equal(await second.actions.setEnabled(key,false),false);
  await second.actions.refreshRules();
  assert.equal(second.state.rules[0].dailyTime,'20:05');
});
test('New scheduler never reads old localStorage keys',async()=>{
  const s=setup();await s.actions.syncAuth();
  assert.equal(s.state.rules.length,0);
  const source=readFileSync(new URL('../src/features/snapshots/scheduler.ts',import.meta.url),'utf8');
  assert.doesNotMatch(source,/localStorage|storage\.getItem|STORAGE_PREFIX/);
});
