import assert from "node:assert/strict";
import test from "node:test";
import {readFileSync} from "node:fs";
const read = (path) => readFileSync(new URL(path, import.meta.url),"utf8");
const history=read("../src/features/history/HistoryView.vue");
const component=read("../src/features/history/IndexHealthHistoryAnalysis.vue");
const analysis=read("../src/features/history/index-health-insights.ts");
const indexView=read("../src/features/index-health/IndexHealthView.vue");
const app=read("../src/App.vue");

test("Index Health history is separate from the live diagnostic collector",()=>{
  assert.match(history,/kind === 'index_health'[^>]*><p[^>]*loadErrors\.index_health/);
  assert.match(history,/:snapshots="snapshots.index_health"/);
  assert.match(history,/v-if="kind !== 'index_health'" class="hist-card"/);
  assert.match(component,/compare_index_health_snapshots/);
  assert.doesNotMatch(component,/collect_index_health_snapshot|index_health_reference|table_detail/);
  assert.match(component,/stats_period_continuous/);
  assert.match(analysis,/stats_reset/);
  assert.match(analysis,/MAX_CONTINUOUS_GAP_MS/);
});
test("Index Health navigation preserves chosen connection in both directions",()=>{
  assert.match(indexView,/@click="emit\('openHistory', state\.connectionId\)"/);
  assert.match(app,/@open-history="openIndexHistory"/);
  assert.match(app,/:index-navigation-token="historyIndexNavigationToken"/);
  assert.match(app,/@open-index-analyzer="openIndexAnalyzer"/);
  assert.match(app,/:navigation-token="indexNavigationToken"/);
  assert.match(indexView,/controller\.selectConnection\(id\)/);
});
test("desktop nav and content have separate full-height scroll areas",()=>{
  assert.match(app,/\.pgxray-app\.workspace-mode \{ height:100dvh; display:flex;/);
  assert.match(app,/\.workspace-sidebar \{[^}]*height:100%;[^}]*overflow-y:auto;/s);
  assert.match(app,/\.workspace-content \{[^}]*height:100%; overflow-y:auto;/s);
  assert.match(app,/@media \(max-width:760px\) \{[^}]*overflow:visible/);
});
