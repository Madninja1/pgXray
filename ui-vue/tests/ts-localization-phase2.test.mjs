import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "../src");
const en = JSON.parse(readFileSync(join(root, "features/i18n/locales/en.json"), "utf8"));
const ru = JSON.parse(readFileSync(join(root, "features/i18n/locales/ru.json"), "utf8"));
const groups = ["tsSettingsExtra", "tsScheduleModel", "tsScheduler", "tsLogExplorer", "tsJournal", "tsLogOperations", "tsLogStatus"];
const placeholders = value => [...value.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();

test("TypeScript controller translations have matching keys and placeholders", () => {
  for (const group of groups) {
    assert.deepEqual(Object.keys(en[group]).sort(), Object.keys(ru[group]).sort(), group);
    for (const [key, english] of Object.entries(en[group])) {
      assert.ok(english.trim(), `${group}.${key}: English is empty`);
      assert.ok(ru[group][key].trim(), `${group}.${key}: Russian is empty`);
      assert.deepEqual(placeholders(english), placeholders(ru[group][key]), `${group}.${key}`);
    }
  }
});

test("journal operations use stable codes with reactive labels", () => {
  const source = readFileSync(join(root, "features/settings/log-view-model.ts"), "utf8");
  assert.match(source, /new Proxy\(operationLabelKeys/);
  assert.match(source, /return key === undefined \? undefined : t\(key\)/);
  assert.match(source, /return logOperationLabels\[operation\]/);
  assert.equal(Object.keys(en.tsLogOperations).length, 39);
});

test("scheduler keeps saved outcomes and uses translated runtime feedback", () => {
  const source = readFileSync(join(root, "features/snapshots/scheduler.ts"), "utf8");
  assert.match(source, /lastError:"Предыдущий сбор не подтверждён/);
  assert.match(source, /t\('tsScheduler\.added'/);
  assert.match(source, /t\('tsScheduler\.updated'/);
  assert.match(source, /t\('tsScheduler\.collectionError'/);
});

test("journal labels follow a live locale switch without changing operation IDs", async () => {
  const { locale } = await import("../dist/features/i18n/locale-service.js");
  const { logOperationLabel, logOperationLabels, logStatusLabel } = await import("../dist/features/settings/log-view-model.js");
  const original = locale.value;
  try {
    locale.value = "en";
    assert.equal(logOperationLabel("delete_user"), "Delete user");
    assert.equal(logStatusLabel("success"), "Success");
    locale.value = "ru";
    assert.equal(logOperationLabel("delete_user"), "Удаление пользователя");
    assert.equal(logStatusLabel("success"), "Успешно");
    assert.ok(Object.keys(logOperationLabels).includes("delete_user"));
  } finally {
    locale.value = original;
  }
});
