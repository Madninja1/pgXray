import assert from 'node:assert/strict';
import test from 'node:test';
import { locale } from '../dist/features/i18n/locale-service.js';
locale.value = 'ru';
import { readFileSync } from 'node:fs';
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from '../dist/core/shell-state.js';
import { createPgssState, createPgssController, pgssSameTarget, pgssMatchesConnection,
  pgssDocsUrl, pgssPage, pgssIdentity, pgssSortRows, pgssFilterCalls } from '../dist/features/pgss/controller.js';

const connection = id => ({ id, name:`pg-${id}`, host:`server-${id}`, port:5432, database:'postgres', username:'reader' });
const summary = (id, connectionId=1, oid=5) => ({
  id, connection_id:connectionId, connection_name:`pg-${connectionId}`,host:`server-${connectionId}`,port:5432,
  database:'postgres',database_oid:oid,username:'reader',sslmode:'disable',server_version_num:160015,
  server_version:'16.15',collection_started_at_ms:id*1000,collection_completed_at_ms:id*1000+20,
  stats_reset:'2026-10-01',dealloc:0,max_entries:5000,current_entries:100,fill_pct:2,
  unavailable_sections:0,
});
const row = (queryid='-98765432109876543', calls=10) => ({
  userid:10,dbid:5,role_name:'app',database:'postgres',toplevel:true,queryid,calls,
  plans:1,total_exec_time_ms:300,mean_exec_time_ms:30,max_exec_time_ms:100,
  temp_blks_read:4,temp_blks_written:5,wal_bytes:2048,
});
const snapshot = () => ({overview:{ database:'postgres',database_oid:5,server_version:'16.15',
  server_version_num:160015,track_planning:false,track_io_timing:false,extension_installed:true },
  stats:{data:{dealloc:0,stats_reset:'2026-10-01'},error:null},
  capacity:{data:{max_entries:5000,current_entries:100,fill_pct:2,total_calls:1000},error:null},
  role_database:{data:[],error:null},commands:{data:[],error:null},call_targets:{data:[],error:null},
  performance:{data:{total_exec:[row()],mean_exec:[],temp_io:[],shared_reads:[],wal:[],variance:[],
    planning:[],history_sample:[row()]},error:null},interpretation:{items:[],limitations:[]}});
const tick = () => new Promise(resolve => setImmediate(resolve));
function setup(invoke) {
  const shell = createShellState(); enterShellWorkspace(shell,'DBA');
  const state = createPgssState();
  return {shell,state,controller:createPgssController(shell,state,invoke)};
}
const archive = (id=3) => ({history:summary(id),snapshot:snapshot()});

test('PGSS Analyzer keeps live tools and links to unified History without a second history tab',()=>{
  const app=readFileSync(new URL('../src/App.vue',import.meta.url),'utf8');
  const view=readFileSync(new URL('../src/features/pgss/PgssView.vue',import.meta.url),'utf8');
  const core=readFileSync(new URL('../src/features/pgss/controller.ts',import.meta.url),'utf8');
  assert.match(app,/PgssView/); assert.match(app,/diagnosticsTab === 'pgss'/);
  for(const name of ['collect_pgss_snapshot','load_pgss_snapshot','list_pgss_snapshots',
    'compare_pgss_snapshots','pgss_statement_history','pgss_query_detail','pgss_reference'])
    assert.ok(core.includes(`"${name}"`),`IPC command ${name} missing`);
  for(const text of ["tr('pgssFinal.health')",'Cardinality','Performance','Statement',"tr('pgssFinal.reference')",
    'query_truncated','pgssDocsUrl',"tr('pgssUi.text002')"]) assert.ok(view.includes(text),text);
  assert.doesNotMatch(view,/История \/ Delta/);
  assert.match(view,/emit\('openHistory'/);
  const unified=readFileSync(new URL('../src/features/history/PgssHistoryAnalysis.vue',import.meta.url),'utf8');
  assert.match(unified,/pgss_workload_history/);
  assert.match(unified,/pgss_statement_history/);
  assert.doesNotMatch(view,/compare_pgss_snapshots/);
  assert.match(view,/tr\('pgssFinal\.missingConnection'\)/);
  assert.match(view,/state\.connections\.some\(c => c\.id === id\)/);
  assert.doesNotMatch(view,/v-html=/);
  assert.doesNotMatch(view,/\bALTER TABLE\b|\bVACUUM FULL\b/);
});

test('PGSS docs reject external schemes and use correct server major',()=>{
  assert.equal(pgssDocsUrl(160015,'pgstatstatements.html'),'https://www.postgresql.org/docs/16/pgstatstatements.html');
  assert.match(pgssDocsUrl(160015,'runtime-config-client.html#RUNTIME-CONFIG-CLIENT-PRELOAD'),/docs\/16\/runtime-config/);
  for(const bad of ['https://evil.example','javascript:alert(1)','../secret','https://postgresql.org/docs','foo/bar'])
    assert.equal(pgssDocsUrl(160015,bad),null);
});

test('Performance sorting, filtering and bounded pages do not mutate source',()=>{
  const rows=Array.from({length:129},(_,i)=>({...row(String(i)),metric:i,role_name:i===9?'special':'other'}));
  const page=pgssPage(rows,'','metric',true,3,25);
  assert.equal(page.total,129);assert.equal(page.rows[0].metric,78);assert.equal(page.rows.length,25);
  assert.equal(rows[0].metric,0);
  assert.equal(pgssPage(rows,'special','metric',true,1,10).total,1);
  assert.equal(pgssPage(rows,'','metric',true,999,100).rows.length,29);
});

test('Performance TOP-50 applies calls filter before paging and never modifies saved data', () => {
  const rows = Array.from({length:50}, (_,i) => row(String(i), i === 0 ? 1 : i <= 4 ? 3 : 10));
  const one = pgssFilterCalls(rows,'one');
  const few = pgssFilterCalls(rows,'few');
  const many = pgssFilterCalls(rows,'many');
  assert.equal(one.length,1);
  assert.equal(few.length,4);
  assert.equal(many.length,45);
  assert.equal(pgssPage(pgssFilterCalls(rows,'all'),'','calls',true,2,25).rows.length,25);
  assert.equal(rows.length,50);
  const view=readFileSync(new URL('../src/features/pgss/PgssView.vue',import.meta.url),'utf8');
  assert.match(view,/Performance TOP-50/);
  assert.match(view,/v-model="perfCalls"/);
});

test('Identity includes userid, dbid, queryid and top-level status',()=>{
  assert.notEqual(pgssIdentity(row()),pgssIdentity({...row(),userid:11}));
  assert.notEqual(pgssIdentity(row()),pgssIdentity({...row(),toplevel:false}));
  assert.equal(pgssIdentity(row()),'10:5:-98765432109876543:true');
});

test('PGSS comparison and live detail reject mismatched profile before IPC', async()=>{
  const calls=[];
  const {state,controller}=setup(async(name,args)=>{
    calls.push(name);
    if(name==='list_connections')return [connection(1)];
    if(name==='list_pgss_snapshots')return [summary(3),summary(2,1,999)];
    if(name==='load_pgss_snapshot')return archive();
    throw Error(name);
  });
  await controller.refreshConnections();await tick();
  assert.equal(pgssMatchesConnection(state.snapshotSummary,connection(1)),true);
  assert.equal(pgssSameTarget(summary(3),summary(2,1,999)),false);
  assert.equal(await controller.compare(),false);
  assert.equal(calls.includes('compare_pgss_snapshots'),false);
});

test('Archived snapshot loads only from the selected connection and checks database OID', async()=>{
  const calls=[];
  const {state,controller}=setup(async(name,args)=>{
    calls.push([name,args]);
    if(name==='list_connections')return [connection(1)];
    if(name==='list_pgss_snapshots')return [summary(3),summary(2)];
    if(name==='load_pgss_snapshot')return args.snapshotId===2
      ? {history:summary(2),snapshot:{...snapshot(),overview:{database:'postgres',database_oid:999}}}
      : archive();
    throw Error(name);
  });
  await controller.refreshConnections();await tick();
  assert.equal(state.snapshotSummary?.id,3);
  assert.equal(await controller.openSnapshot(2),false);
  assert.match(state.error,/Несовпадение/);
  assert.equal(state.snapshotSummary?.id,3);
  assert.equal(await controller.openSnapshot(444),false);
  assert.equal(calls.filter(([name])=>name==='load_pgss_snapshot').every(([,args])=>args.connectionId===1),true);
});

test('Statement history is requested from SQLite; live detail is strictly opt-in',async()=>{
  const calls=[];
  const {state,controller}=setup(async(name,args)=>{
    calls.push([name,args]);
    if(name==='list_connections')return [connection(1)];
    if(name==='list_pgss_snapshots')return [summary(3)];
    if(name==='load_pgss_snapshot')return archive();
    if(name==='pgss_statement_history')return {
      anchor_snapshot_id:3,connection_id:1,userid:10,dbid:5,queryid:row().queryid,toplevel:true,
      summary:{history_points:1,comparable_intervals:0,latest_delta_state:'baseline'},
      points:[],change_analysis:{state:'insufficient',items:[],limitations:[]},limitations:[]
    };
    if(name==='pgss_query_detail')return {...row(),query:'SELECT 1',query_truncated:true};
    throw Error(name);
  });
  await controller.refreshConnections();await tick();
  assert.equal(await controller.selectStatement(row()),true);
  assert.equal(calls.filter(([name])=>name==='pgss_statement_history').length,1);
  assert.equal(calls.some(([name])=>name==='pgss_query_detail'),false);
  assert.equal(await controller.refreshDetail(),true);
  assert.equal(calls.filter(([name])=>name==='pgss_query_detail').length,1);
  assert.equal(state.detail?.query_truncated,true);
});

test('Missing queryid never triggers live or historical statement calls', async()=>{
  const calls=[];
  const {state,controller}=setup(async(name,args)=>{
    calls.push(name);
    if(name==='list_connections')return [connection(1)];
    if(name==='list_pgss_snapshots')return [summary(3)];
    if(name==='load_pgss_snapshot')return archive();
    throw Error(name);
  });
  await controller.refreshConnections();await tick();
  assert.equal(await controller.selectStatement(row(null)),false);
  assert.equal(await controller.refreshDetail(),false);
  assert.equal(calls.includes('pgss_query_detail'),false);
});

test('Switching connection discards stale archive response',async()=>{
  let complete;
  const delayed=new Promise(resolve=>{complete=resolve});
  const {state,controller}=setup(async(name,args)=>{
    if(name==='list_connections')return [connection(1),connection(2)];
    if(name==='list_pgss_snapshots')return args.connectionId===1 ? [summary(3)] : [];
    if(name==='load_pgss_snapshot')return delayed;
    throw Error(name);
  });
  await controller.refreshConnections();await tick();
  controller.selectConnection(2);
  complete(archive());await tick();await tick();
  assert.equal(state.connectionId,2);assert.equal(state.snapshot,null);
});

test('Authentication transition discards pending details and resets data',async()=>{
  let complete;
  const delayed=new Promise(resolve=>{complete=resolve});
  const {shell,state,controller}=setup(async(name,args)=>{
    if(name==='list_connections')return [connection(1)];
    if(name==='list_pgss_snapshots')return [summary(3)];
    if(name==='load_pgss_snapshot')return archive();
    if(name==='pgss_statement_history')return delayed;
    throw Error(name);
  });
  await controller.refreshConnections();await tick();
  const pending=controller.selectStatement(row());
  leaveShellWorkspace(shell);controller.reset();complete({anchor_snapshot_id:3});
  await pending;assert.equal(state.statement,null);assert.equal(state.selectedRow,null);
});

test('Reference is cached and detached from PostgreSQL collection',async()=>{
  const calls=[];
  const {state,controller}=setup(async(name,args)=>{
    calls.push(name);
    if(name==='pgss_reference')return {methodology:[],queries:[],sources:[]};
    throw Error(name);
  });
  await controller.loadReference();await controller.loadReference();
  assert.equal(calls.length,1);assert.equal(state.reference.methodology.length,0);
});

test('PGSS DBA sort is immutable and stable; missing metrics stay last in both directions', () => {
  const input = [
    { label: 'b', metric: null, other: 2 },
    { label: 'a', metric: 20, other: 3 },
    { label: 'c', metric: 5, other: 1 },
    { label: 'd', metric: 20, other: 4 },
  ];
  const ascending = pgssSortRows(input, 'metric', false);
  assert.deepEqual(ascending.map(x => x.label), ['c', 'a', 'd', 'b']);
  const descending = pgssSortRows(input, 'metric', true);
  assert.deepEqual(descending.map(x => x.label), ['a', 'd', 'c', 'b']);
  assert.deepEqual(input.map(x => x.label), ['b', 'a', 'c', 'd']);
  assert.deepEqual(pgssPage(input, '', 'metric', true, 1, 10).rows.map(x => x.label), ['a', 'd', 'c', 'b']);
  assert.deepEqual(pgssSortRows(input, 'label', false).map(x => x.label), ['a', 'b', 'c', 'd']);
});

test('PGSS Analyzer keeps active header sorting; History is a separate screen', () => {
  const view=readFileSync(new URL('../src/features/pgss/PgssView.vue',import.meta.url),'utf8');
  for (const name of ['Card','Perf']) {
    assert.ok(view.includes(`change${name}Sort(`),`${name} header sorting missing`);
  }
  assert.match(view, /aria-sort=/);
  assert.match(view, /class="sort-head"/);
  const controller=readFileSync(new URL('../src/features/pgss/controller.ts',import.meta.url),'utf8');
  assert.match(controller, /t\('diagTs\.pgssOpenError'/);
  const history=readFileSync(new URL('../src/features/history/PgssHistoryAnalysis.vue',import.meta.url),'utf8');
  assert.match(history, /t\('historyDetails\.PgssSqlHistory'\)/);
  assert.match(history, /t\('historyDetails\.PgssInvestigate'\)/);
});

test('PGSS missing Rust snapshot command gives actionable restart instructions', async () => {
  const {state,controller}=setup(async(name,args)=>{
    if(name==='list_connections') return [connection(1)];
    if(name==='list_pgss_snapshots') return [summary(3)];
    if(name==='load_pgss_snapshot') throw Error('Command load_pgss_snapshot not found');
    throw Error(name);
  });
  await controller.refreshConnections();await tick();
  assert.equal(await controller.openSnapshot(3),false);
  assert.match(state.error,/Rust\/Tauri backend/);
  assert.match(state.error,/пересобери/);
});

test('PGSS DBA view prioritizes grounded candidates and avoids empty one-point charts', () => {
  const view=readFileSync(new URL('../src/features/pgss/PgssView.vue',import.meta.url),'utf8');
  assert.match(view, /tr\('pgssUi\.text013'\)/);
  assert.match(view,/leadTotal\.total_exec_time_ms/);
  assert.match(view,/leadMean\.mean_exec_time_ms/);
  const history=readFileSync(new URL('../src/features/history/PgssHistoryAnalysis.vue',import.meta.url),'utf8');
  assert.match(history,/selectedInterval/);
  assert.match(history, /t\('historyDetails\.PgssIntervals'\)/);
  const ru = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/ru.json', import.meta.url), 'utf8'));
  const en = JSON.parse(readFileSync(new URL('../src/features/i18n/locales/en.json', import.meta.url), 'utf8'));
  assert.match(ru.historyDetails.PgssIntervals, /Сопоставимых интервалов/);
  assert.ok(en.historyDetails.PgssIntervals);
  assert.match(view,/details v-for="\(item,i\) in snapshot\.interpretation\.items"/);
  assert.match(view,/details v-for="\(step,i\) in state\.reference\.methodology"/);
  assert.match(history,/selected_to_snapshot_id/);
});
