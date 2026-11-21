import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
const view = readFileSync(new URL("../src/features/activity/ActivityView.vue", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.vue", import.meta.url), "utf8");

test("activity is kept mounted while navigating, but only visible in Monitoring", () => {
  assert.match(app, /<ActivityView v-if="bridge" v-show="state\.mainView === 'activity'"/);
  assert.match(app, /:visible="state\.mainView === 'activity'"/);
  assert.doesNotMatch(app, /<ActivityView v-else-if=/);
  assert.match(view, /watch\(\(\) => props\.visible, visible => \{ if \(!visible\) closeSql\(\); \}\)/);
  assert.match(view, /t\('activity\.title'\)/);
});
test("SQL text stays hidden unless manually expanded; no HTML injection or SQL in search", () => {
  assert.match(view, /@click="openSql\(backend\)"/);
  assert.match(view, /<dialog ref="sqlDialog"/);
  assert.match(view, /<pre class="activity-sql-text">\{\{ sqlPreview\.text \}\}<\/pre>/);
  assert.doesNotMatch(view, /<details v-if="backend\.query"/);
  assert.doesNotMatch(view, /v-html/);
  assert.doesNotMatch(view, /:title="backend\.query"/);
});
test("signals require controller decision, disabled buttons and explicit confirmation", () => {
  assert.match(view, /!controller\.canSignal\(backend, 'cancel'\)/);
  assert.match(view, /!controller\.canSignal\(backend, 'terminate'\)/);
  assert.match(view, /import \{ confirmAction \} from "\.\.\/\.\.\/core\/confirm-action\.js"/);
  assert.match(view, /controller\.signal\(backend, action, \(question\) => confirmAction\(question\)\)/);
  assert.doesNotMatch(view, /window\.confirm/);
});
test("monitor has the original interval and duration presets plus local refresh feedback", () => {
  assert.match(view, /ACTIVITY_INTERVALS/);
  assert.match(view, /ACTIVITY_DURATIONS/);
  assert.match(view, /refreshVisual/);
  assert.match(view, /connectionsVisual/);
});

test("current snapshot and monitor sample summaries are visibly separate", () => {
  assert.match(view, /id="activity-current-title">\{\{ t\('activity\.currentSnapshot'\) \}\}/);
  assert.match(view, /t\('activity\.summary'\)/);
  assert.match(view, /t\('activity\.sampleCount'/);
});
test("duration warnings are for active queries, and signaling stays under controller guard", () => {
  assert.match(view, /activityQueryDurationTier\(backend\)/);
  assert.match(view, /t\('activity\.overFive'\)/);
  assert.match(view, /t\('activity\.cancelQuery'\)/);
  assert.match(view, /t\('activity\.terminate'\)/);
});
test("SQL panel uses native modal dialog and warns when idle query may be stale", () => {
  assert.match(view, /sqlDialog\.value\.showModal\(\)/);
  assert.match(view, /authGeneration: props\.shell\.authGeneration/);
  assert.match(view, /t\('activity\.sqlSnapshot'/);
  assert.match(view, /sqlBackend\.state !== 'active'/);
  assert.match(view, /t\('activity\.lastSqlWarning'\)/);
  assert.doesNotMatch(view, /v-html/);
});

test("SQL drawer limits DOM preview but copies full backend text explicitly", () => {
  assert.match(view, /activitySqlPreview\(sqlBackend\.value\?\.query \?\? ""\)/);
  assert.match(view, /navigator\.clipboard\.writeText\(sql\)/);
  assert.match(view, /t\('activity\.sqlPrivacy'\)/);
  assert.match(view, /t\('activity\.truncated'\)/);
  assert.doesNotMatch(view, /v-html/);
});


test("activity keep-alive retries its initial connection list after authentication busy clears", () => {
  assert.match(view, /watch\(\(\) => props\.shell\.busy, loadInitialConnectionsWhenReady\)/);
  assert.match(view, /onMounted\(loadInitialConnectionsWhenReady\)/);
  assert.match(view, /canBootstrapActivityConnections\(props\.shell, initialConnectionsPending\)/);
  assert.match(view, /if \(initialConnectionsPending\) \{ loadInitialConnectionsWhenReady\(\); return; \}/);
  assert.match(view, /if \(!state\.monitoring && !state\.signaling\) void refreshConnections\(\)/);
});


test("empty connection hint appears only after a completed load and resets on logout", () => {
  assert.match(view, /const connectionsLoaded = ref\(false\)/);
  assert.match(view, /if \(await controller\.refreshConnections\(\)\) connectionsLoaded\.value = true/);
  assert.match(view, /connectionsLoaded\.value = false/);
  assert.match(view, /v-if="connectionsLoaded && !state\.connections\.length/);
});
