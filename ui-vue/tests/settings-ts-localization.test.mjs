import test from "node:test";
import assert from "node:assert/strict";
import en from "../src/features/i18n/locales/en.json" with { type: "json" };
import ru from "../src/features/i18n/locales/ru.json" with { type: "json" };
import { locale } from "../dist/features/i18n/locale-service.js";
import { validateNewUsername, createUsersController, createUsersState } from "../dist/features/settings/users-controller.js";
import { readinessStatusLabel, readinessSupportLabel } from "../dist/features/settings/settings-view-model.js";
import { maintenanceActions, maintenanceConfirmation, maintenanceResultLabel } from "../dist/features/settings/storage-maintenance-ui.js";

const params = text => [...text.matchAll(/\{([A-Za-z]+)\}/g)].map(m => m[1]).sort();
test("settings controller translations have matching keys and placeholders", () => {
  assert.deepEqual(Object.keys(en.settingsTs).sort(), Object.keys(ru.settingsTs).sort());
  for (const key of Object.keys(en.settingsTs)) {
    assert.deepEqual(params(en.settingsTs[key]), params(ru.settingsTs[key]), key);
  }
});
test("TypeScript settings helpers react to language changes without recreating objects", () => {
  try {
    locale.value = "en";
    assert.match(validateNewUsername("bad name"), /Username/);
    assert.equal(readinessStatusLabel("ready"), "Ready");
    assert.equal(readinessSupportLabel("unverified"), "Version not verified");
    assert.equal(maintenanceActions.check.title, "Check integrity");
    assert.match(maintenanceConfirmation("retention"), /permanently deleted/);
    assert.match(maintenanceResultLabel("retention", "done", 3), /Snapshots deleted: 3/);
    locale.value = "ru";
    assert.match(validateNewUsername("bad name"), /Логин/);
    assert.equal(readinessStatusLabel("ready"), "Готово");
    assert.equal(maintenanceActions.check.title, "Проверить целостность");
    assert.match(maintenanceConfirmation("retention"), /безвозвратно удалены/);
    assert.match(maintenanceResultLabel("retention", "готово", 3), /Удалено снимков: 3/);
  } finally { locale.value = "en"; }
});
