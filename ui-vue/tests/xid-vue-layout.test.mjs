import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
const app = readFileSync(new URL('../src/App.vue',import.meta.url),'utf8');
const view = readFileSync(new URL('../src/features/xid/XidView.vue',import.meta.url),'utf8');
const logic = readFileSync(new URL('../src/features/xid/controller.ts',import.meta.url),'utf8');

test('XID view is available through the Vue diagnostics navigation',()=>{
  assert.match(app,/import XidView from .*features\/xid\/XidView.vue/);
  assert.match(app,/diagnosticsTab === 'xid'/);
  assert.match(app,/XID \/ Wraparound/);
  assert.match(view,/aria-label="XID \/ Wraparound"/);
});
test('XID Vue renders all diagnostic sections with explicit absence/availability distinction',()=>{
  for(const field of ['databases','relations','autovacuum_triggers','vacuums','backends','slots','prepared'])
    assert.match(view,new RegExp(`snap\\.${field}`));
  assert.match(view,/sectionMessage\(snap\.vacuums\)/);
  assert.match(view,/sectionMessage\(snap\.backends\)/);
  assert.match(view,/snap\.prepared\.error \|\| !snap\.prepared\.data/);
  assert.doesNotMatch(view,/v-html=/);
});
test('XID Policy Apply is explicit confirmation and relies on guarded Rust IPC',()=>{
  assert.match(view,/role="alertdialog"/);
  assert.match(view,/candidateToApply\.sql/);
  assert.match(logic,/"apply_xid_policy_candidate"/);
  assert.match(logic,/candidateId/);
  assert.doesNotMatch(view,/\bALTER\s+TABLE\s+\$\{/);
  assert.doesNotMatch(logic,/exec_sql|execute_query|run_query/);
});


test('XID diagnostics sorts live relations and links to the one History screen',()=>{
  for(const name of ['toggleRelationSort','toggleTriggerSort'])
    assert.match(view,new RegExp(name));
  assert.match(view,/aria-sort="sortAria\(relationSort/);
  assert.match(view,/v-model="relationScope"/);
  assert.match(view,/triggerRows\.rows/);
  assert.match(view,/emit\('openHistory'\)/);
  assert.doesNotMatch(view,/tab===['"]history['"]/);
  assert.match(app,/@open-history="openXidHistory"/);
  assert.match(app,/:xid-navigation-token="historyXidNavigationToken"/);
  assert.match(app,/:preferred-connection-id="xidPreferredConnectionId"/);
  assert.match(view,/props\.preferredConnectionId/);
  assert.match(logic,/xidSortRows/);
});
test('XID DBA investigation exposes relation evidence and explicit destructive confirmation',()=>{
  for(const phrase of ['relationMinPct','relationKind','relationHorizon','triggerScope','currentRelation.all_frozen_pages','currentRelation.reloptions','currentRelation.modifications_since_analyze','applyPhrase !== \'APPLY\''])
    assert.ok(view.includes(phrase), `missing DBA control: ${phrase}`);
  assert.match(view,/xidTriggerStateLabel\(t\)/);
  assert.match(view,/hasRelation\(t\.relation_oid\)/);
});
test('History owns XID trend and auxiliary two-point comparison',()=>{
  const history = readFileSync(new URL('../src/features/history/HistoryView.vue',import.meta.url),'utf8');
  const trend = readFileSync(new URL('../src/features/history/XidHistoryAnalysis.vue',import.meta.url),'utf8');
  assert.match(history,/compare_xid_snapshots|COMPARE_HISTORY_COMMANDS/);
  assert.match(history,/exportXidComparison/);
  assert.match(trend,/xid_history_overview/);
  assert.match(trend,/xid_relation_history/);
  assert.match(trend,/emit\('openAnalyzer'\)/);
  assert.match(view,/exportRelations/);
  assert.match(view,/copySql\(q\.sql\)/);
});


test('XID navigation only loads saved snapshots; fresh collections require an explicit click',()=>{
  assert.match(view,/await controller\.refreshConnections\(true, props\.preferredConnectionId/);
  assert.doesNotMatch(view,/await controller\.collect\(\)/);
  assert.match(view,/@click="controller\.collect"/);
  assert.match(view, /tr\('xidUi\.text004'\)/);
});

test('Desktop sidebar has its own scroll, and Tauri defaults to Vue',()=>{
  const tauri=readFileSync(new URL('../../src-tauri/Tauri.toml',import.meta.url),'utf8');
  assert.match(app,/\.pgxray-app\.workspace-mode \{[^}]*height:100dvh;[^}]*display:flex;/s);
  assert.match(app,/\.workspace-sidebar \{[^}]*height:100%;[^}]*overflow-y:auto;/s);
  assert.match(app,/\.workspace-content \{[^}]*height:100%; overflow-y:auto;/s);
  assert.match(app,/@media \(max-width:760px\).*overflow:visible;/s);
  assert.match(tauri,/frontendDist = "\.\.\/ui-vue\/build"/);
  assert.match(tauri,/beforeDevCommand = "npm run dev"/);
  assert.match(tauri,/beforeBuildCommand = "npm run build"/);
  assert.doesNotMatch(tauri,/before(?:Dev|Build)Command = "npm --prefix ui-vue /);
});
