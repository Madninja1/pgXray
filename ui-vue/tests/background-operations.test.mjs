import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { durationText } from "../dist/core/background-operations.js";
test("elapsed running time never becomes negative", () => {
  assert.equal(durationText(1000, 0), "0 мин 0 сек");
  assert.equal(durationText(1000, 62000), "1 мин 1 сек");
  assert.equal(durationText(1000, 3602000), "1 ч 0 мин");
});

import { aggregatePolls } from "../dist/core/background-operations.js";
test("aggregates only the active monitor client-side IPC counters", () => {
  const polls = aggregatePolls([
    { id: "activity", title: "Activity", connection: "a", startedAt: 100, polls: { requests: 3, succeeded: 2, failed: 1, lastMs: 5, totalMs: 15, maxMs: 10 } },
    { id: "locks", title: "Locks", connection: "b", startedAt: 200, polls: { requests: 2, succeeded: 2, failed: 0, lastMs: 5, totalMs: 12, maxMs: 8 } },
  ]);
  assert.deepEqual(polls, { requests: 5, succeeded: 4, failed: 1, totalMs: 27, maxMs: 10 });
});
