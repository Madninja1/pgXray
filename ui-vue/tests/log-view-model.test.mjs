import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import {
  logDisplayRows, logViewPage, logOperationLabel, logStatusLabel,
} from "../dist/features/settings/log-view-model.js";

const entry = (time, request, phase, overrides = {}) => ({
  timestamp_ms: time, level: "info", component: "ui_reported", operation: "login",
  phase, actor: "admin", request_id: request, duration_ms: phase === "started" ? null : 12,
  context: null, error: phase === "failed" ? "bad credentials" : null, ...overrides,
});

test("pairs matching start/end phases into one operation without losing raw events", () => {
  const entries = [entry(20, "req1", "succeeded"), entry(10, "req1", "started")];
  const rows = logDisplayRows(entries, "operations");
  assert.equal(rows.length, 1);
  assert.equal(rows[0].result, "success");
  assert.equal(rows[0].entries.length, 2);
  assert.equal(rows[0].duration_ms, 12);
  assert.equal(logDisplayRows(entries, "events").length, 2);
});

test("unpaired start is not mistaken for a confirmed running command", () => {
  const row = logDisplayRows([entry(1, "id", "started")], "operations")[0];
  assert.equal(row.result, "incomplete");
  assert.equal(logStatusLabel(row.result), "Нет итоговой записи");
});

test("a different user/component/operation or missing request ID never merges", () => {
  const data = [entry(1, "same", "started"),
    entry(2, "same", "succeeded", { actor: "other" }),
    entry(3, "same", "succeeded", { component: "app_log" }),
    entry(4, "same", "failed", { operation: "logout" }),
    entry(5, null, "started"), entry(6, null, "failed")];
  assert.equal(logDisplayRows(data, "operations").length, 6);
});

test("a backend event without request ID remains separate from UI reports", () => {
  const data = [entry(1, null, "started", { component: "application" }),
    entry(2, "id", "succeeded")];
  assert.deepEqual(logDisplayRows(data, "operations").map(x => x.source), ["ui", "backend"]);
});

test("searches full fields including errors and returns clamped filtered pagination", () => {
  const items = Array.from({ length: 63 }, (_, i) => entry(i + 1, `req${i}`, i % 2 ? "failed" : "succeeded", {
    operation: i === 5 ? "delete_user" : "login",
  }));
  let p = logViewPage(items, { mode: "events", search: "bad credentials", status: "failed", component: "all", page: 2, pageSize: 20 });
  assert.equal(p.total, 31);
  assert.equal(p.page, 2);
  assert.equal(p.shownStart, 21);
  assert.equal(p.shownEnd, 31);
  p = logViewPage(items, { mode: "operations", search: "delete user", status: "all", component: "all", page: 99, pageSize: 20 });
  assert.equal(p.total, 0);
  p = logViewPage(items, { mode: "operations", search: "delete_user", status: "all", component: "all", page: 99, pageSize: 20 });
  assert.equal(p.page, 1);
  assert.equal(p.total, 1);
  assert.equal(logOperationLabel("delete_user"), "Удаление пользователя");
});

test("raw logs keep original error/context values available for details", () => {
  const original = entry(10, "r", "failed", { context: "db=example", error: "a < b & private" });
  const row = logDisplayRows([original], "events")[0];
  assert.equal(row.error, original.error);
  assert.equal(row.entries[0], original);
});


test("intermediate phases do not prematurely end an operation", () => {
  const events = [entry(1, "mid", "started"), entry(2, "mid", "progress"), entry(3, "mid", "succeeded")];
  const rows = logDisplayRows(events, "operations");
  assert.equal(rows.length, 1);
  assert.equal(rows[0].result, "success");
  assert.equal(rows[0].entries.length, 3);
});
