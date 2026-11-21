import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { locale } from "../dist/features/i18n/locale-service.js";
import { newConnectionDraft, validateConnectionDraft } from "../dist/features/connections/controller.js";
import { parseActivitySnapshot } from "../dist/features/activity/controller.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "../src");
const en = JSON.parse(readFileSync(join(root, "features/i18n/locales/en.json"), "utf8"));
const ru = JSON.parse(readFileSync(join(root, "features/i18n/locales/ru.json"), "utf8"));
const placeholders = text => [...text.matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort();

test("connections and activity have identical localized keys and placeholders", () => {
  for (const group of ["tsConnections", "tsActivity"]) {
    assert.deepEqual(Object.keys(en[group]).sort(), Object.keys(ru[group]).sort());
    for (const key of Object.keys(en[group]))
      assert.deepEqual(placeholders(en[group][key]), placeholders(ru[group][key]), `${group}.${key}`);
  }
});

test("connection validation and Activity parser respond to locale changes", () => {
  const before = locale.value;
  try {
    locale.value = "en";
    assert.match(validateConnectionDraft(newConnectionDraft()).name, /Connection name:/);
    assert.throws(() => parseActivitySnapshot(null), /Invalid response/);
    locale.value = "ru";
    assert.match(validateConnectionDraft(newConnectionDraft()).name, /Название подключения:/);
    assert.throws(() => parseActivitySnapshot(null), /Некорректный ответ/);
  } finally {
    locale.value = before;
  }
});

test("the signal confirmation still checks target identity and consent", () => {
  const source = readFileSync(join(root, "features/activity/controller.ts"), "utf8");
  assert.match(source, /if \(!await confirm\(question\)\) return false/);
  assert.match(source, /state\.snapshot !== snapshot/);
  assert.match(source, /!canSignal\(backend, action\)/);
  assert.match(source, /tsActivity\.terminateWarning/);
  assert.match(source, /tsActivity\.cancelWarning/);
});
