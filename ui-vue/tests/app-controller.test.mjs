import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { createAppController } from "../dist/app/controller.js";
import { createShellState, enterShellWorkspace } from "../dist/core/shell-state.js";

function setup(raw) {
  const state = createShellState();
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return raw(command, args);
  };
  return { state, calls, controller: createAppController(state, invoke) };
}

test("auth_state selects registration screen on first install", async () => {
  const { state, calls, controller } = setup(async () => ({ needs_registration: true, session: null }));
  assert.equal(await controller.initialize(), true);
  assert.equal(state.screen, "registration");
  assert.deepEqual(calls.map((c) => c.command), ["auth_state"]);
  assert.deepEqual(state.message, { text: "", error: false });
});

test("auth_state selects login when no user is authenticated", async () => {
  const { state, controller } = setup(async () => ({ needs_registration: false, session: null }));
  assert.equal(await controller.initialize(), true);
  assert.equal(state.screen, "login");
  assert.equal(state.message.text, "");
});

test("auth_state restores an existing session", async () => {
  const { state, controller } = setup(async () => ({ needs_registration: false, session: { username: "owner" } }));
  assert.equal(await controller.initialize(), true);
  assert.equal(state.screen, "workspace");
  assert.equal(state.message.text, "");
  assert.equal(state.currentUsername, "owner");
  assert.equal(state.authGeneration, 1);
});

test("login calls Rust command with original argument shape", async () => {
  const { state, calls, controller } = setup(async () => ({ username: "owner" }));
  assert.equal(await controller.login("owner", "secret-123456"), true);
  assert.deepEqual(calls[0], { command: "login", args: { username: "owner", password: "secret-123456" } });
  assert.equal(state.screen, "workspace");
  assert.equal(state.message.text, "");
});

test("register rejects password mismatch before IPC", async () => {
  const { state, calls, controller } = setup(async () => ({ username: "owner" }));
  assert.equal(await controller.registerInitialUser("owner", "first-password", "wrong-password"), false);
  assert.equal(calls.length, 0);
  assert.equal(state.message.text, "Error: Пароли не совпадают");
});

test("successful registration enters workspace", async () => {
  const { state, calls, controller } = setup(async () => ({ username: "owner" }));
  assert.equal(await controller.registerInitialUser("owner", "example-password", "example-password"), true);
  assert.equal(calls[0].command, "register_initial_user");
  assert.equal(state.screen, "workspace");
  assert.equal(state.message.text, "");
});

test("logout clears shell identity only after successful IPC", async () => {
  const { state, calls, controller } = setup(async () => undefined);
  enterShellWorkspace(state, "owner");
  const before = state.authGeneration;
  assert.equal(await controller.logout(), true);
  assert.equal(calls[0].command, "logout");
  assert.equal(state.currentUsername, null);
  assert.equal(state.screen, "login");
  assert.equal(state.authGeneration, before + 1);
  assert.equal(state.message.text, "");
});

test("failed logout does not falsely claim a cleared backend session", async () => {
  const { state, controller } = setup(async () => { throw new Error("backend logout failed"); });
  enterShellWorkspace(state, "owner");
  assert.equal(await controller.logout(), false);
  assert.equal(state.currentUsername, "owner");
  assert.equal(state.screen, "workspace");
});

test("invalid Rust payload does not create a fake authenticated session", async () => {
  const { state, controller } = setup(async () => ({ username: "owner" }));
  assert.equal(await controller.login("owner", "secret"), true);
  // Compare with a malformed payload.
  const bad = setup(async () => ({ username: 17 }));
  assert.equal(await bad.controller.login("owner", "secret"), false);
  assert.equal(bad.state.screen, null);
  assert.equal(bad.state.currentUsername, null);
  assert.equal(bad.state.message.error, true);
});

test("second action cannot overlap a pending login", async () => {
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  const { state, calls, controller } = setup(async () => pending);
  const first = controller.login("owner", "some-password");
  assert.equal(state.busy, true);
  assert.deepEqual(state.message, { text: "", error: false });
  assert.equal(await controller.logout(), false);
  assert.equal(calls.length, 1);
  release({ username: "owner" });
  assert.equal(await first, true);
  assert.equal(state.busy, false);
});

test("a stale initialization response cannot overwrite a newer session", async () => {
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  const { state, controller } = setup(async () => pending);
  const first = controller.initialize();
  enterShellWorkspace(state, "new-user");
  release({ needs_registration: true, session: null });
  assert.equal(await first, true);
  assert.equal(state.screen, "workspace");
  assert.equal(state.currentUsername, "new-user");
});

test("navigation changes only authenticated and idle shell state", async () => {
  const { state, controller } = setup(async () => undefined);
  controller.selectMainView("settings");
  assert.equal(state.mainView, "connections");
  enterShellWorkspace(state, "owner");
  controller.selectMainView("settings");
  assert.equal(state.mainView, "settings");
});

test("missing Tauri bridge is surfaced rather than silently faking auth", async () => {
  const state = createShellState();
  const controller = createAppController(state, null);
  assert.equal(await controller.initialize(), false);
  assert.equal(state.currentUsername, null);
  assert.equal(state.message.error, true);
});


test("progress remains local to the form, and a prior auth error clears when retrying", async () => {
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  let attempts = 0;
  const { state, controller } = setup(async () => {
    attempts += 1;
    if (attempts === 1) throw new Error("bad credentials");
    return pending;
  });
  assert.equal(await controller.login("owner", "bad"), false);
  assert.equal(state.message.error, true);
  const retry = controller.login("owner", "valid");
  assert.equal(state.busy, true);
  assert.deepEqual(state.message, { text: "", error: false });
  release({ username: "owner" });
  assert.equal(await retry, true);
  assert.equal(state.busy, false);
  assert.deepEqual(state.message, { text: "", error: false });
});
