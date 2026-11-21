import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const src = path => readFileSync(new URL(`../src/${path}`,import.meta.url),'utf8');
const connections = src('features/connections/ConnectionsView.vue');
const settings = src('features/settings/SettingsView.vue');
const activity = src('features/activity/ActivityView.vue');

test("project UI starts collapsed, supports sorting and persistent explicit pinning", () => {
  assert.match(connections, /expandedGroups\[groupKey\(id\)\] === true/);
  assert.match(connections, /id === null\) return expandedGroups\.ungrouped !== false/);
  assert.match(connections, /const projectSort = ref<ProjectSort>\("name-asc"\)/);
  assert.match(connections, /sortProjectGroups/);
  assert.match(connections, /connections-desc/);
  assert.match(connections, /loadPinnedProjectIds/);
  assert.match(connections, /savePinnedProjectIds/);
  assert.match(connections, /t\("connections\.pin"\)/);
  assert.match(connections, /:disabled="isPinned\(group\.id\)"/);
});
test("connections and user context menus close outside and detach their listeners", () => {
  for (const view of [connections,settings]) {
    assert.match(view,/dismissOpenDetailsOutside/);
    assert.match(view,/document\.addEventListener\("pointerdown", onDocumentPointerDown, true\)/);
    assert.match(view,/document\.removeEventListener\("pointerdown", onDocumentPointerDown, true\)/);
    assert.match(view,/document\.addEventListener\("keydown", onDocumentKeyDown\)/);
    assert.match(view,/document\.removeEventListener\("keydown", onDocumentKeyDown\)/);
  }
});
test("SQL modal dismisses backdrop clicks and explains volatile activity snapshots", () => {
  assert.match(activity, /if \(event\.target === sqlDialog\.value\) closeSql\(\)/);
  assert.match(activity, /@click="onSqlBackdropClick"/);
  assert.match(activity, /t\('activity\.explainer'\)/);
  assert.match(activity, /t\('activity\.sampleCount'/);
  for (const lang of ["ru", "en"]) {
    const dictionary = JSON.parse(readFileSync(new URL(`../src/features/i18n/locales/${lang}.json`, import.meta.url), "utf8"));
    assert.match(dictionary.activity.explainer, lang === "ru" ? /SQL в историю не сохраняются/ : /SQL are not saved to history/);
    assert.ok(dictionary.activity.sampleCount.includes("{count}"));
  }
  assert.match(activity, /<pre class="activity-sql-text">\{\{ sqlPreview\.text \}\}<\/pre>/);
});
