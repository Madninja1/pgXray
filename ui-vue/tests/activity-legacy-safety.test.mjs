import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createContext, runInContext } from "node:vm";
const script = readFileSync(new URL("../../ui/js/activity.js", import.meta.url), "utf8");
function deferred() {
  let resolve;
  const promise = new Promise(ok => { resolve = ok; });
  return { promise, resolve };
}
function runtime(invoke) {
  const selected = { value: "1" };
  const calls = [];
  const context = createContext({
    byId: (key) => key === "activity-connection" ? selected : { textContent: "", className: "" },
    invoke: (command, args) => { calls.push({command, args}); return invoke(command, args); },
    window: { confirm: () => true },
    message() {}, clearTimeout() {}, setTimeout() {},
  });
  runInContext(script, context);
  runInContext("renderActivitySnapshot = () => {};", context);
  return { context, selected, calls };
}
const snapshot = (at = Date.now()) => ({
  captured_at_ms: at, total_connections: 1,
  backends: [{pid:123,backend_start_ms:555,backend_type:"client backend",state:"active",
    username:"reader",database:"test",application_name:"api"}],
});

test("legacy: a late Activity snapshot never crosses connection selection", async () => {
  const gate = deferred();
  const { context, selected } = runtime(() => gate.promise);
  const pending = runInContext("refreshActivitySnapshot()", context);
  selected.value = "2";
  runInContext("activityRevision++", context);
  gate.resolve(snapshot());
  assert.equal(await pending, false);
  assert.equal(runInContext("currentActivitySnapshot", context), null);
});
test("legacy: reset invalidates pending snapshot", async () => {
  const gate = deferred();
  const { context } = runtime(() => gate.promise);
  const pending = runInContext("refreshActivitySnapshot()", context);
  runInContext("stopActivityMonitoring(false)", context);
  gate.resolve(snapshot());
  assert.equal(await pending, false);
});
test("legacy: stale or mismatched PID/backend_start cannot trigger signal", async () => {
  const { context, calls } = runtime(() => ({signaled:true}));
  context.mockSnapshot = snapshot(Date.now() - 30_000);
  runInContext("currentActivitySnapshot = mockSnapshot", context);
  await assert.rejects(runInContext('signalActivityBackend(123, 555, "terminate")', context), /устарел/);
  context.mockSnapshot = snapshot();
  runInContext("currentActivitySnapshot = mockSnapshot", context);
  await assert.rejects(runInContext('signalActivityBackend(123, 999, "terminate")', context), /устарел/);
  assert.equal(calls.length, 0);
});
