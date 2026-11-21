import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve, dirname } from "node:path";
const dir=resolve(dirname(fileURLToPath(import.meta.url)),"../src/features/history");
const files=["HistoryView","XidHistoryAnalysis","PgssHistoryAnalysis","AutovacuumHistoryAnalysis",
  "SettingsHistoryAnalysis","IoWalHistoryAnalysis","IndexHealthHistoryAnalysis"];

test("All 19 History tables have interactive sortable data headers", () => {
  let tables=0;
  for (const name of files) {
    const source=readFileSync(resolve(dir,`${name}.vue`),"utf8");
    const matches=[...source.matchAll(/<table(?:\s[^>]*)?>[\s\S]*?<\/table>/g)];
    assert.ok(matches.length,`${name} must contain tables`);
    for (const [index, match] of matches.entries()) {
      tables++;
      assert.match(match[0],/<HistorySortTh\b/,`${name} table ${index} should have sortable headings`);
      assert.match(match[0],/historySort\.(?:sorted|toggle)/,`${name} table ${index} should wire up sorting`);
    }
  }
  assert.equal(tables,19);
});

test("History tables with pagination sort before slicing the result", () => {
  for (const name of ["IndexHealthHistoryAnalysis","IoWalHistoryAnalysis","PgssHistoryAnalysis",
    "SettingsHistoryAnalysis","AutovacuumHistoryAnalysis","XidHistoryAnalysis","HistoryView"]) {
    const source=readFileSync(resolve(dir,`${name}.vue`),"utf8");
    assert.match(source,/historySort\.sorted\(/,name);
  }
  const settings=readFileSync(resolve(dir,"SettingsHistoryAnalysis.vue"),"utf8");
  assert.match(settings,/sortedChanges\.value\.slice\(/);
  const index=readFileSync(resolve(dir,"IndexHealthHistoryAnalysis.vue"),"utf8");
  assert.match(index,/sortedChanges\.value\.slice\(/);
  const av=readFileSync(resolve(dir,"AutovacuumHistoryAnalysis.vue"),"utf8");
  assert.match(av,/sortedRelations\.value\.slice\(/);
});
