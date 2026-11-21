import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from "../dist/core/shell-state.js";
import {
  createUsersController, createUsersState, parseUserSummaries,
  validateNewUsername, validateNewPassword,
} from "../dist/features/settings/users-controller.js";

const users = () => [{ username: "owner", current: true }, { username: "viewer", current: false }];
function setup(respond = async (cmd) => cmd === "list_users" ? users() : undefined) {
  const shell = createShellState();
  enterShellWorkspace(shell, "owner");
  const state = createUsersState();
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return respond(command, args);
  };
  return { shell, state, calls, controller: createUsersController(shell, state, invoke) };
}
function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test("list_users checks schema and strips unknown sensitive fields", async () => {
  const { controller, state, calls } = setup(async () => users().map((u) => ({ ...u, password_hash: "secret" })));
  assert.equal(await controller.refresh(), true);
  assert.deepEqual(state.users, users());
  assert.deepEqual(calls, [{ command: "list_users", args: undefined }]);
  assert.equal(Object.hasOwn(state.users[0], "password_hash"), false);
});

test("malformed user lists do not replace last known good list", async () => {
  let invalid = false;
  const { state, controller } = setup(async () => invalid ? [{ username: "owner", current: "true" }] : users());
  assert.equal(await controller.refresh(), true);
  invalid = true;
  assert.equal(await controller.refresh(), false);
  assert.deepEqual(state.users, users());
  assert.match(state.error, /Некорректный/);
  assert.throws(() => parseUserSummaries([{ username: "duplicate", current: false }, { username: "duplicate", current: false }]));
});

test("username rules match Rust ASCII 1–64 constraint", () => {
  assert.equal(validateNewUsername("a-_09"), null);
  for (const s of ["", "a b", "Русский", "x".repeat(65)]) assert.ok(validateNewUsername(s));
});

test("password rules mirror Unicode scalar count and UTF-8 byte limit", () => {
  assert.equal(validateNewPassword("🔑".repeat(12), "🔑".repeat(12)), null);
  assert.match(validateNewPassword("a".repeat(12), "not-the-same"), /совпадают/);
  assert.match(validateNewPassword("a".repeat(11), "a".repeat(11)), /минимум/);
  assert.match(validateNewPassword("🔑".repeat(257), "🔑".repeat(257)), /1024/);
});

test("create_user invokes exact payload, updates list and never stores password", async () => {
  const { controller, state, calls } = setup();
  await controller.refresh();
  assert.equal(controller.openCreate(), true);
  assert.equal(await controller.create("guest", "some-long-secret", "some-long-secret"), true);
  assert.deepEqual(calls.map((call) => call.command), ["list_users", "create_user", "list_users"]);
  assert.deepEqual(calls[1].args, { username: "guest", password: "some-long-secret" });
  assert.equal(state.editor, null);
  assert.equal(JSON.stringify(state).includes("some-long-secret"), false);
});

test("invalid registration does not invoke Rust", async () => {
  const { controller, state, calls } = setup();
  controller.openCreate();
  assert.equal(await controller.create("wrong user", "short", "short"), false);
  assert.equal(calls.length, 0);
  assert.match(state.error, /Логин/);
});

test("rename_user updates authenticated username and invalidates old auth epoch", async () => {
  const { controller, state, shell, calls } = setup(async (command) => {
    if (command === "list_users") return [{ username: shell.currentUsername, current: true }, { username: "viewer", current: false }];
  });
  await controller.refresh();
  const before = shell.authGeneration;
  assert.equal(controller.openRename("owner"), true);
  assert.equal(await controller.rename("renamed"), true);
  assert.equal(calls[1].command, "rename_user");
  assert.deepEqual(calls[1].args, { username: "owner", newUsername: "renamed" });
  assert.equal(shell.currentUsername, "renamed");
  assert.equal(shell.authGeneration, before + 1);
  assert.equal(state.mutating, false);
});

test("changing other username does not change the active session", async () => {
  const { shell, controller, calls } = setup();
  await controller.refresh();
  const epoch = shell.authGeneration;
  assert.equal(controller.openRename("viewer"), true);
  assert.equal(await controller.rename("analyst"), true);
  assert.equal(shell.currentUsername, "owner");
  assert.equal(shell.authGeneration, epoch);
  assert.deepEqual(calls[1].args, { username: "viewer", newUsername: "analyst" });
});

test("changing another user's password uses exact IPC without retaining secrets", async () => {
  const { controller, state, calls } = setup();
  await controller.refresh();
  controller.openPassword("viewer");
  assert.equal(await controller.changePassword("new-long-password", "new-long-password"), true);
  assert.equal(calls[1].command, "change_user_password");
  assert.deepEqual(calls[1].args, { username: "viewer", password: "new-long-password" });
  assert.equal(JSON.stringify(state).includes("new-long-password"), false);
});

test("delete_user requires confirmation and cannot delete current user", async () => {
  const { controller, state, calls } = setup();
  await controller.refresh();
  assert.equal(await controller.deleteUser("owner", () => true), false);
  assert.match(state.error, /Нельзя удалить/);
  assert.equal(await controller.deleteUser("viewer", () => false), false);
  assert.equal(calls.length, 1);
  assert.equal(await controller.deleteUser("viewer", () => true), true);
  assert.deepEqual(calls[1], { command: "delete_user", args: { username: "viewer" } });
});

test("successful mutation followed by failed refresh is reported as success with warning", async () => {
  let count = 0;
  const { controller, state } = setup(async (cmd) => {
    if (cmd === "list_users") {
      if (++count === 2) throw new Error("SQLite is busy");
      return users();
    }
  });
  await controller.refresh();
  controller.openCreate();
  assert.equal(await controller.create("newuser", "longlongpassword", "longlongpassword"), true);
  assert.match(state.notice, /Пользователь создан/);
  assert.match(state.notice, /Не удалось обновить список/);
  assert.equal(state.users.some((u) => u.username === "newuser"), true);
});

test("backend error does not claim success or modify cached list", async () => {
  const { controller, state } = setup(async (cmd) => {
    if (cmd === "list_users") return users();
    throw new Error("SQLite operation failed");
  });
  await controller.refresh();
  controller.openCreate();
  assert.equal(await controller.create("newuser", "longlongpassword", "longlongpassword"), false);
  assert.deepEqual(state.users, users());
  assert.match(state.error, /SQLite operation failed/);
  assert.equal(state.notice, null);
});

test("late list_users response after logout cannot repopulate users", async () => {
  const wait = deferred();
  const { shell, controller, state } = setup(async () => wait.promise);
  const inFlight = controller.refresh();
  leaveShellWorkspace(shell);
  controller.reset();
  wait.resolve(users());
  assert.equal(await inFlight, false);
  assert.deepEqual(state.users, []);
});

test("stale user deletion confirmation cannot delete after logout", async () => {
  const wait = deferred();
  const { shell, state, controller, calls } = setup();
  await controller.refresh();
  const pending = controller.deleteUser("viewer", async () => wait.promise);
  leaveShellWorkspace(shell);
  controller.reset();
  wait.resolve(true);
  assert.equal(await pending, false);
  assert.deepEqual(calls.map((c) => c.command), ["list_users"]);
  assert.deepEqual(state.users, []);
});

test("overlapping mutations are rejected rather than duplicating writes", async () => {
  const wait = deferred();
  const { controller, state, calls } = setup(async (cmd) => cmd === "list_users" ? users() : wait.promise);
  await controller.refresh();
  controller.openCreate();
  const first = controller.create("guest", "some-long-secret", "some-long-secret");
  assert.equal(state.mutating, true);
  assert.equal(await controller.create("guest", "some-long-secret", "some-long-secret"), false);
  assert.equal(calls.filter((c) => c.command === "create_user").length, 1);
  wait.resolve(undefined);
  assert.equal(await first, true);
});

test("pending deletion confirmation blocks concurrent edits and writes", async () => {
  const wait = deferred();
  const { controller, state, calls } = setup();
  await controller.refresh();
  const first = controller.deleteUser("viewer", () => wait.promise);
  assert.equal(state.confirmingDeletion, true);
  assert.equal(controller.openCreate(), false);
  assert.equal(controller.openRename("viewer"), false);
  assert.equal(controller.openPassword("viewer"), false);
  assert.equal(await controller.deleteUser("viewer", () => true), false);
  assert.deepEqual(calls.map((c) => c.command), ["list_users"]);
  wait.resolve(false);
  assert.equal(await first, false);
  assert.equal(state.confirmingDeletion, false);
});

test("rejected confirmation returns an error without any destructive IPC", async () => {
  const { controller, state, calls } = setup();
  await controller.refresh();
  assert.equal(await controller.deleteUser("viewer", () => { throw new Error("dialog unavailable"); }), false);
  assert.equal(state.confirmingDeletion, false);
  assert.match(state.error, /dialog unavailable/);
  assert.deepEqual(calls.map((c) => c.command), ["list_users"]);
});
