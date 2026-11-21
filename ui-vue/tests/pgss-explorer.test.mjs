import assert from 'node:assert/strict';
import test from 'node:test';
import { locale } from '../dist/features/i18n/locale-service.js';
locale.value = 'ru';
import { readFileSync } from 'node:fs';
import { createShellState, enterShellWorkspace } from '../dist/core/shell-state.js';
import {
  createPgssController, createPgssState, pgssExplorerPageValid,
} from '../dist/features/pgss/controller.js';

const connection = { id: 2, name:'demo-db-a', host:'db.example', port:5432, database:'postgres', username:'reader' };
const summary = {
  id:8, connection_id:2, host:'db.example', port:5432, database:'postgres', username:'reader',
  sslmode:'disable', database_oid:5,
};
const row = {
  userid:10, dbid:5, toplevel:true, queryid:'-987654321012345678',
  calls:1, plans:0, total_exec_time_ms:2500, mean_exec_time_ms:2500,
  max_exec_time_ms:2500, temp_blks_read:0, temp_blks_written:0, wal_bytes:0,
};
function harness(invoke) {
  const shell = createShellState(); enterShellWorkspace(shell, 'admin');
  const state = createPgssState(); state.connections = [connection]; state.connectionId=2;
  state.snapshotSummary = summary;
  return { state, controller:createPgssController(shell, state, invoke) };
}
const tick = () => new Promise(resolve => setImmediate(resolve));

test('One-call filters are sent to the backend, not applied to a local TOP-25', async () => {
  const calls = [];
  const {state, controller} = harness(async (command, args) => {
    calls.push({command,args});
    return {rows:[row],total:1,offset:0,limit:25,has_more:false};
  });
  const filter = {one_call:true,userid:10,dbid:5,offset:0,limit:25,sort:'calls'};
  assert.equal(await controller.refreshExplorer(filter), true);
  assert.equal(calls.length,1);
  assert.deepEqual(calls[0], {command:'pgss_explore',args:{connectionId:2,filters:filter}});
  assert.deepEqual(state.explorer?.rows,[row]);
  assert.equal(state.explorerLoading,false);
  assert.equal(state.explorer?.total,1);
});

test('CALL tag filter remains exact and a live entry is not invented in SQLite history', async () => {
  const tag = 'ebf098eff46959c81f238423e09db4e1';
  const {state,controller} = harness(async (command,args) => {
    assert.equal(args.filters.call_tag,tag);
    return {rows:[row],total:1,offset:0,limit:25,has_more:false};
  });
  await controller.refreshExplorer({userid:10,dbid:5,call_tag:tag,offset:0,limit:25});
  controller.selectLiveStatement(row);
  assert.equal(state.selectedLive,true);
  assert.equal(state.statement,null);
  assert.equal(state.selectedRow?.queryid,row.queryid);
  assert.match(state.statementError,/текущего PGSS/);
});

test('Explorer rejects cross-user rows, prevents publishing a stale response on connection switch', async () => {
  const { state, controller } = harness(async () => ({
    rows:[{...row,userid:999}], total:1, offset:0, limit:25, has_more:false,
  }));
  assert.equal(await controller.refreshExplorer({userid:10,offset:0,limit:25}),false);
  assert.equal(state.explorer,null);
  assert.match(state.explorerError,/Несоответствие/);

  let release;
  const pending = new Promise(resolve => { release = resolve; });
  const slow = harness(async () => pending);
  const request = slow.controller.refreshExplorer({limit:25,offset:0});
  slow.controller.selectConnection(null);
  release({rows:[row],total:1,offset:0,limit:25,has_more:false});
  await request; await tick();
  assert.equal(slow.state.explorer,null);
});

test('Explorer validates bounded shape, with queryid remaining an exact string', () => {
  const filters = {one_call:true,limit:25,offset:0,userid:10};
  assert.equal(pgssExplorerPageValid({rows:[row],total:1,offset:0,limit:25,has_more:false},filters),true);
  assert.equal(pgssExplorerPageValid({rows:[{...row,calls:2}],total:1,offset:0,limit:25,has_more:false},filters),false);
  assert.equal(pgssExplorerPageValid({rows:[{...row,queryid:12}],total:1,offset:0,limit:25,has_more:false},filters),false);
  assert.equal(pgssExplorerPageValid({rows:[row],total:1,offset:0,limit:100,has_more:false},filters),false);
});

test('Explorer validates calls and duration filters in returned rows', () => {
  const filters = {min_calls:2,max_calls:5,min_total_exec_ms:100,limit:25,offset:0};
  const validRow = {...row,calls:3,total_exec_time_ms:200};
  assert.equal(pgssExplorerPageValid({rows:[validRow],total:1,offset:0,limit:25,has_more:false},filters),true);
  assert.equal(pgssExplorerPageValid({rows:[{...validRow,calls:1}],total:1,offset:0,limit:25,has_more:false},filters),false);
  assert.equal(pgssExplorerPageValid({rows:[{...validRow,calls:6}],total:1,offset:0,limit:25,has_more:false},filters),false);
  assert.equal(pgssExplorerPageValid({rows:[{...validRow,total_exec_time_ms:50}],total:1,offset:0,limit:25,has_more:false},filters),false);
});

test('Vue exposes clear navigation and does not load SQL bodies into list results', () => {
  const view = readFileSync(new URL('../src/features/pgss/PgssView.vue', import.meta.url),'utf8');
  const backend = readFileSync(new URL('../../crates/core/src/pgss/collector.rs',import.meta.url),'utf8');
  const tauri = readFileSync(new URL('../../src-tauri/src/main.rs',import.meta.url),'utf8');
  assert.match(view,/PGSS Explorer \(live\)/);
  assert.match(view, /tr\('pgssUi\.text009'\)/);
  assert.match(view,/exploreCard\(row, true\)/);
  assert.match(view,/await controller\.refreshDetail\(\)/);
  assert.match(backend,/pg_stat_statements\(false\) AS s/);
  assert.doesNotMatch(backend,/s\.query AS query/);
  assert.match(tauri,/pgss_explore,/);
});

test('exact filtered total validates pagination and empty out-of-range pages', () => {
  const filter = { limit:25, offset:25 };
  assert.equal(pgssExplorerPageValid({rows:[row],total:27,offset:25,limit:25,has_more:false},filter),true);
  assert.equal(pgssExplorerPageValid({rows:[],total:12,offset:25,limit:25,has_more:false},filter),true);
  assert.equal(pgssExplorerPageValid({rows:[],total:99,offset:25,limit:25,has_more:false},filter),false);
  assert.equal(pgssExplorerPageValid({rows:[row],total:0,offset:25,limit:25,has_more:false},filter),false);
  assert.equal(pgssExplorerPageValid({rows:[row],total:1.5,offset:25,limit:25,has_more:false},filter),false);
  assert.equal(pgssExplorerPageValid({rows:[row],offset:25,limit:25,has_more:false},filter),false);
});

test('PGSS Explorer provides dropdown filters, total and a native statement dialog', () => {
  const view = readFileSync(new URL('../src/features/pgss/PgssView.vue',import.meta.url),'utf8');
  assert.match(view,/v-model="explorerUserId"/);
  assert.match(view,/v-model="explorerDbId"/);
  assert.match(view,/v-model="explorerCommand"/);
  assert.match(view,/roleOptions/);
  assert.match(view,/databaseOptions/);
  assert.match(view,/commandOptions/);
  assert.match(view,/v-model="explorerCallTag"/);
  assert.match(view,/callTargetOptions/);
  assert.match(view,/state\.explorer\.total/);
  assert.match(view,/<dialog v-if="statementOpen"/);
  assert.match(view,/@close="onStatementDialogClosed"/);
  assert.doesNotMatch(view,/tab === 'statement'/);
  assert.match(view,/controller\.clearStatement\(\)/);
});

test('Explorer clickable column headers drive server sorting across pages', () => {
  const view = readFileSync(new URL('../src/features/pgss/PgssView.vue',import.meta.url),'utf8');
  const backend = readFileSync(new URL('../../crates/core/src/pgss/collector.rs',import.meta.url),'utf8');
  for (const key of ['queryid','role','calls','total_exec','mean_exec','temp','wal']) {
    assert.ok(view.includes(`@click="changeExplorerSort('${key}')"`), `missing clickable header ${key}`);
  }
  assert.match(view,/const direction = appliedSort === key/);
  assert.match(view,/const filter: PgssExplorerFilter = \{ \.\.\.explorerFilters\.value, sort:key, direction, offset:0 \}/);
  assert.match(view,/void controller\.refreshExplorer\(filter\)/);
  assert.match(view,/:aria-sort="explorerFilters\.sort/);
  assert.match(view,/v-model="explorerDirection"/);
  assert.match(backend,/ORDER BY \{order\} \{direction\} NULLS LAST/);
  assert.match(backend,/"asc" => "ASC"/);
  assert.match(backend,/"desc" => "DESC"/);
});
