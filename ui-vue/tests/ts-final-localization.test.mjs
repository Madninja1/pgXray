import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { locale } from "../dist/features/i18n/locale-service.js";
import { avAdviceNextStep } from "../dist/features/autovacuum/presentation.js";
import { avIntervalLabel } from "../dist/features/history/autovacuum-insights.js";
import { indexPairStatus } from "../dist/features/history/index-health-insights.js";
import { PGSS_RANKS } from "../dist/features/pgss/controller.js";
import { SNAPSHOT_METRICS } from "../dist/features/history/model.js";
import { THEME_CHOICES } from "../dist/features/theme/theme-controller.js";
import { displayScheduleError } from "../dist/features/snapshots/schedule-model.js";
import { durationText } from "../dist/core/background-operations.js";
import { yesNo } from "../dist/core/formatters.js";

const savedScheduleError = "Профиль подключения удалён или изменился.";

test("diagnostic TypeScript labels change with locale without changing status IDs", () => {
  try {
    locale.value = "en";
    assert.equal(indexPairStatus("partial_sample"), "Partial index sample");
    assert.equal(avIntervalLabel("partial_sample"), "Partial overlap");
    assert.equal(PGSS_RANKS[0].label, "Total execution time");
    assert.equal(SNAPSHOT_METRICS.xid[1][0], "Database age");
    assert.equal(THEME_CHOICES[2].label, "Light");
    assert.equal(yesNo(true), "yes");
    assert.match(avAdviceNextStep({history_points: 1}), /three snapshots/i);

    locale.value = "ru";
    assert.equal(indexPairStatus("partial_sample"), "Неполная выборка индексов");
    assert.equal(avIntervalLabel("partial_sample"), "Неполное пересечение");
    assert.equal(PGSS_RANKS[0].label, "Общее время");
    assert.equal(SNAPSHOT_METRICS.xid[1][0], "Возраст БД");
    assert.equal(THEME_CHOICES[2].label, "Светлая");
    assert.equal(yesNo(true), "да");
    assert.match(avAdviceNextStep({history_points: 1}), /Нужно минимум 3 снимка/);
  } finally { locale.value = "en"; }
});

test("stored scheduler messages are translated at display time without rewriting SQLite payloads", () => {
  try {
    locale.value = "en";
    assert.equal(displayScheduleError(savedScheduleError), "Connection profile was deleted or changed.");
    assert.equal(displayScheduleError("Custom PostgreSQL error"), "Custom PostgreSQL error");
    locale.value = "ru";
    assert.equal(displayScheduleError(savedScheduleError), savedScheduleError);
  } finally { locale.value = "en"; }
});

test("time unit labels follow the current locale", () => {
  try {
    locale.value = "en";
    assert.equal(durationText(0, 65_000), "1 min 5 s");
    locale.value = "ru";
    assert.equal(durationText(0, 65_000), "1 мин 5 сек");
  } finally { locale.value = "en"; }
});

test("remaining TypeScript message dictionaries have matching keys and parameters", () => {
  const dir = new URL("../src/features/i18n/locales/", import.meta.url);
  const en = JSON.parse(readFileSync(new URL("en.json", dir), "utf8"));
  const ru = JSON.parse(readFileSync(new URL("ru.json", dir), "utf8"));
  assert.deepEqual(Object.keys(en.tsFinish).sort(), Object.keys(ru.tsFinish).sort());
  const placeholders = message => [...message.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();
  for (const key of Object.keys(en.tsFinish)) {
    assert.deepEqual(placeholders(en.tsFinish[key]), placeholders(ru.tsFinish[key]), key);
  }
});
