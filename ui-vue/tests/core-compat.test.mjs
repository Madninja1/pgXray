import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import * as migrated from "../dist/core/formatters.js";
import {
  createShellState,
  enterShellWorkspace,
  isCurrentAuthGeneration,
  leaveShellWorkspace,
  MAIN_VIEWS,
} from "../dist/core/shell-state.js";

// Evaluate the original core.js as a comparison oracle; it declares globals
// but performs no DOM operations until a function is called.
const legacy = readFileSync(new URL("../../ui/js/core.js", import.meta.url), "utf8");
const context = vm.createContext({ window: {}, Intl, Date });
vm.runInContext(legacy, context, { filename: "ui/js/core.js" });
function original(name, ...args) {
  context.__args = args;
  return vm.runInContext(`${name}(...__args)`, context);
}

const numericCases = [null, undefined, 0, 1, -1, 12.5, 1024, 1234567, "123", "-55", NaN, Infinity];
for (const method of [
  "formatInteger", "formatNumber", "formatPercent", "formatBytes",
  "formatDuration", "formatSignedInteger", "formatSnapshotTime",
]) {
  test(`${method}: parity with legacy core.js`, () => {
    for (const value of numericCases) {
      assert.equal(migrated[method](value), original(method, value), `input: ${String(value)}`);
    }
  });
}

test("formatNumber retains optional decimal digits", () => {
  for (const value of [0, 1.234, 123456.789]) {
    for (const digits of [0, 1, 2, 3]) {
      assert.equal(migrated.formatNumber(value, digits), original("formatNumber", value, digits));
    }
  }
});

test("formatTimestamp retains legacy handling of zero and invalid dates", () => {
  for (const value of [null, undefined, 0, 1700000000000, "2026-11-09T12:00:00Z", "not-a-date", ""]) {
    assert.equal(migrated.formatTimestamp(value), original("formatTimestamp", value));
  }
});

test("yesNo and relation sample labels retain legacy output", () => {
  for (const value of [null, undefined, false, true, 0, 1, "", "false"]) {
    assert.equal(migrated.yesNo(value), original("yesNo", value));
  }
  for (const state of ["present_in_both", "entered_new_sample", "not_in_new_sample", "custom"]) {
    assert.equal(migrated.relationSampleStateLabel(state), original("relationSampleStateLabel", state));
  }
});

test("shell starts without an authenticated user and supports all eight main views", () => {
  const state = createShellState();
  assert.equal(state.currentUsername, null);
  assert.equal(state.screen, null);
  assert.equal(state.mainView, "connections");
  assert.equal(MAIN_VIEWS.length, 8);
});

test("auth generation invalidates responses from earlier sessions", () => {
  const state = createShellState();
  const oldGeneration = state.authGeneration;
  enterShellWorkspace(state, "alice");
  assert.equal(state.currentUsername, "alice");
  assert.equal(isCurrentAuthGeneration(state, oldGeneration), false);
  const loggedInGeneration = state.authGeneration;
  leaveShellWorkspace(state);
  assert.equal(state.currentUsername, null);
  assert.equal(state.screen, "login");
  assert.equal(isCurrentAuthGeneration(state, loggedInGeneration), false);
});
