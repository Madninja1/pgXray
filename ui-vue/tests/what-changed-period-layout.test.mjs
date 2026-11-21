import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
const root = new URL("../src/features/what-changed/", import.meta.url);
const view = readFileSync(new URL("WhatChangedView.vue",root),"utf8");
const date = readFileSync(new URL("DateTimeField.vue",root),"utf8");
const coverage = readFileSync(new URL("SnapshotCoverage.vue",root),"utf8");
test("period selection uses native date/time separately with blur on commit", () => {
  assert.doesNotMatch(view,/type="datetime-local"/);
  assert.match(date,/type="date"/);
  assert.match(date,/type="time"/);
  assert.match(date,/input\.blur\(\)/);
});
test("What Changed has instructions, saved-data proposal and editable comparison periods", () => {
  assert.match(view,/t\('changes\.howTo'\)/);
  assert.match(view,/t\('changes\.autoPeriods'\)/);
  assert.match(view,/@snapshots-loaded="snapshotsLoaded"/);
  assert.equal((view.match(/<DateTimeField v-model="time\./g) ?? []).length,4);
  assert.match(view,/v-if="periodError"/);
});
test("coverage caches snapshots when changing date ranges but reloads on selection change", () => {
  assert.match(coverage,/watch\(\(\) => props\.range, recalculate\)/);
  assert.match(coverage,/emit\("snapshotsLoaded"/);
  assert.match(coverage,/cached\.value = rows;/);
});

test("comparison warns when incident ends after measurement time", () => {
  assert.match(view,/futurePeriodWarning/);
  assert.match(view,/incidentToMs/);
  assert.match(view,/generated_at_ms/);
  assert.match(view,/t\('changes\.futurePeriod'/);
});
test("setting comparisons expose the field that actually changed", () => {
  assert.match(view,/settingReasons\(item\.change\)/);
  assert.match(view,/fmtSettingSource\(item\.change, 'older'\)/);
  assert.match(view,/reset_value_changed/);
  assert.match(view,/pending_restart_changed/);
});
test("index usage and diagnostic findings are separate", () => {
  assert.match(view,/t\('changes\.indexUsageTitle'/);
  assert.match(view,/usage_changes/);
  assert.match(view,/t\('changes\.indexHealthTitle'/);
  assert.match(view,/blocks_hit_observed_seconds/);
});
