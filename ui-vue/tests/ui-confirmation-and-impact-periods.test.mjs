import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const read = path => readFileSync(new URL(path, import.meta.url), "utf8");
test("saved password reuse requires an explicit in-app confirmation", () => {
  const view = read("../src/features/connections/ConnectionsView.vue");
  assert.match(view, /confirmPasswordReuse: .*requestConfirmation/);
  assert.match(view, /confirmationDialog/);
  assert.match(view, /finishConfirmation\(true\)/);
  assert.match(view, /finishConfirmation\(false\)/);
  assert.doesNotMatch(view, /window\.confirm/);
});
test("change impact dates and times are separate fields", () => {
  const form = read("../src/features/tools/InvestigationPanel.vue");
  const input = read("../src/components/LocalDateTimeInput.vue");
  for (const name of ["beforeFrom", "beforeTo", "afterFrom", "afterTo", "occurredAt"])
    assert.match(form, new RegExp(`LocalDateTimeInput v-model="${name}"`));
  assert.match(input, /type="date"/);
  assert.match(input, /type="time"/);
});
test("diagnostic connection lists can be refreshed without restarting the app", () => {
  const autovacuum = read("../src/features/autovacuum/AutovacuumView.vue");
  assert.doesNotMatch(autovacuum, />Обновить подключения<\/button>/);
  assert.match(autovacuum, /watch\(\(\) => props\.visible, visible =>/);
  assert.match(read("../src/features/pgss/PgssView.vue"), /if \(visible && props\.shell\.screen === "workspace"/);
  assert.match(read("../src/features/pg-settings/PgSettingsView.vue"), /watch\(\(\) => props\.visible/);
});
