import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from "../dist/core/shell-state.js";
import {
  createConnectionsController, createConnectionsState, connectionFailureFields,
  editConnectionDraft, newConnectionDraft, parseSavedConnections,
  passwordFlags, validateConnectionDraft, activeSessionsForConnection, savedPasswordTargetChanged,
  postgresVersionFromProbe, formatProbeTimestamp, changesActiveSessionTarget,
} from "../dist/features/connections/controller.js";

const example = (overrides = {}) => ({
  id: 42, name: "prod", host: "db.example.org", port: 5432,
  database: "postgres", username: "reader", sslmode: "verify-full",
  has_password: true, ...overrides,
});

function setup(raw = async (command) => command === "list_connections" ? [example()] : "ok", extraOptions = {}) {
  const shell = createShellState();
  enterShellWorkspace(shell, "owner");
  const state = createConnectionsState();
  const calls = [];
  const timers = new Map();
  let nextTimer = 0;
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return raw(command, args);
  };
  const controller = createConnectionsController(shell, state, invoke, {
    setTimer: (fn) => { const id = ++nextTimer; timers.set(id, () => { timers.delete(id); fn(); }); return id; },
    clearTimer: (id) => { timers.delete(id); },
    ...extraOptions,
  });
  return { shell, state, calls, timers, controller };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((ok, no) => { resolve = ok; reject = no; });
  return { promise, resolve, reject };
}

test("list_connections maps only safe fields, excluding accidental secrets", async () => {
  const { state, calls, controller } = setup(async () => [{ ...example(), password: "never-retain" }]);
  assert.equal(await controller.refresh(), true);
  assert.deepEqual(calls, [{ command: "list_connections", args: undefined }]);
  assert.equal(state.items.length, 1);
  assert.equal(Object.hasOwn(state.items[0], "password"), false);
  assert.equal(state.items[0].has_password, true);
});

test("malformed list response cannot replace a valid list", async () => {
  let malformed = false;
  const { state, controller } = setup(async () => malformed ? [{ ...example(), port: "5432" }] : [example()]);
  assert.equal(await controller.refresh(), true);
  malformed = true;
  assert.equal(await controller.refresh(), false);
  assert.deepEqual(state.items, [example()]);
  assert.match(state.error, /Некорректный ответ/);
  assert.throws(() => parseSavedConnections({ fake: true }), /Некорректный/);
});

test("new draft reproduces defaults without persisting a password", () => {
  const draft = newConnectionDraft();
  assert.deepEqual([draft.host, draft.port, draft.database, draft.username, draft.sslmode],
    ["127.0.0.1", "5432", "postgres", "postgres", "verify-full"]);
  assert.equal(draft.password, "");
  assert.equal(draft.removePassword, false);
});

test("editing blanks password but offers explicit retention/removal", () => {
  const draft = editConnectionDraft(example());
  assert.equal(draft.password, "");
  assert.deepEqual(passwordFlags(example(), draft), { useSavedPassword: true, keepExistingPassword: true });
  draft.removePassword = true;
  assert.deepEqual(passwordFlags(example(), draft), { useSavedPassword: false, keepExistingPassword: false });
  draft.removePassword = false;
  draft.password = "replacement";
  assert.deepEqual(passwordFlags(example(), draft), { useSavedPassword: false, keepExistingPassword: false });
});

test("draft validation agrees with Core's port, hostname, TLS and password constraints", () => {
  const draft = newConnectionDraft();
  draft.name = "local";
  assert.deepEqual(validateConnectionDraft(draft), {});
  draft.port = "65536";
  draft.host = "https://example.org";
  draft.sslmode = "unsafe";
  draft.password = "line\nfeed";
  draft.removePassword = true;
  const errors = validateConnectionDraft(draft);
  assert.ok(errors.port && errors.host && errors.sslmode && errors.password);
  draft.port = "5432";
  draft.host = "127.0.0.1";
  draft.sslmode = "verify-full";
  draft.password = "";
  draft.removePassword = false;
  draft.database = "a".repeat(257);
  assert.ok(validateConnectionDraft(draft).database);
});

test("failure hints preserve old categories", () => {
  assert.deepEqual(Object.keys(connectionFailureFields("password authentication failed")), ["username", "password"]);
  assert.ok(connectionFailureFields("database does not exist").database);
  assert.ok(connectionFailureFields("TLS certificate mismatch").sslmode);
  assert.ok(connectionFailureFields("connection refused").host);
});

test("draft probe keeps original camelCase IPC arg names and saved password", async () => {
  const { controller, calls, state } = setup();
  await controller.refresh();
  assert.equal(controller.openEditEditor(42), true);
  assert.equal(await controller.probeDraft(), true);
  assert.equal(calls[1].command, "probe_connection_draft");
  assert.equal(calls[1].args.connectionId, 42);
  assert.equal(calls[1].args.useSavedPassword, true);
  assert.equal(calls[1].args.input.password, "");
  assert.equal(state.editorStatus.kind, "success");
});

test("probe errors identify fields without hiding original error", async () => {
  const { state, controller } = setup(async (command) => {
    if (command === "list_connections") return [example()];
    throw new Error("password authentication failed");
  });
  await controller.refresh();
  controller.openEditEditor(42);
  assert.equal(await controller.probeDraft(), false);
  assert.ok(state.fieldErrors.username);
  assert.match(state.editorStatus.text, /password authentication failed/);
});

test("save_connection uses exact existing API shape and clears secret on success", async () => {
  const { controller, calls, state } = setup(async (command) =>
    command === "list_connections" ? [example({ id: 43, has_password: true })] : example({ id: 43 }));
  controller.openNewEditor();
  state.editor.name = "local";
  state.editor.password = "do-not-print";
  assert.equal(await controller.saveDraft(), true);
  assert.equal(calls[0].command, "save_connection");
  assert.deepEqual(Object.keys(calls[0].args), ["name", "input"]);
  assert.equal(calls[0].args.input.password, "do-not-print");
  assert.equal(state.editor, null);
  assert.equal(calls[1].command, "list_connections");
});

test("update keeps stored password when editor password is empty", async () => {
  const { controller, calls } = setup();
  await controller.refresh();
  controller.openEditEditor(42);
  assert.equal(await controller.saveDraft(), true);
  assert.equal(calls[1].command, "update_saved_connection");
  assert.deepEqual(Object.keys(calls[1].args), ["id", "name", "input", "keepExistingPassword"]);
  assert.equal(calls[1].args.keepExistingPassword, true);
  assert.equal(calls[1].args.input.password, "");
});

test("editing a live connection detects credential changes but permits a rename", () => {
  const existing = example();
  const draft = editConnectionDraft(existing);
  draft.name = "new label";
  assert.equal(changesActiveSessionTarget(existing, draft), false);
  for (const [field, value] of [
    ["host", "new.example.org"], ["port", "6432"], ["database", "archive"],
    ["username", "operator"], ["sslmode", "disable"],
    ["password", "replacement"], ["removePassword", true],
  ]) {
    assert.equal(changesActiveSessionTarget(existing, { ...draft, [field]: value }), true, field);
  }
});

test("failed save keeps the backend denial inside the editor, not behind the modal", async () => {
  const { state, controller } = setup(async command => {
    if (command === "list_connections") return [example()];
    if (command === "update_saved_connection") throw new Error(
      "Нельзя менять сервер или пароль, пока открыт SQL-терминал. Сначала закройте сессию.",
    );
    return "ok";
  }, { confirmPasswordReuse: () => true });
  await controller.refresh();
  controller.openEditEditor(42);
  state.editor.host = "other.example.org";
  assert.equal(await controller.saveDraft(), false);
  assert.equal(state.error, null);
  assert.equal(state.mutating, false);
  assert.equal(state.editor.host, "other.example.org");
  assert.equal(state.editorStatus.kind, "error");
  assert.match(state.editorStatus.text, /Нельзя менять сервер или пароль/);
  assert.equal(state.editor.password, "");
});

test("obsolete editor cannot inherit a late failed save", async () => {
  const pending = deferred();
  const invoked = deferred();
  const { controller, state } = setup(async command => {
    if (command === "list_connections") return [example()];
    if (command === "update_saved_connection") { invoked.resolve(); return pending.promise; }
    return "ok";
  });
  await controller.refresh();
  controller.openEditEditor(42);
  const saving = controller.saveDraft();
  // Simulate a dialog replaced while a real IPC save is still in progress.
  await invoked.promise;
  controller.closeEditor();
  pending.reject(new Error("old save failed"));
  assert.equal(await saving, false);
  assert.equal(state.editorStatus, null);
  assert.equal(state.error, null);
});

test("update without password retention sends false explicitly", async () => {
  const { controller, calls, state } = setup();
  await controller.refresh();
  controller.openEditEditor(42);
  state.editor.removePassword = true;
  assert.equal(await controller.saveDraft(), true);
  assert.equal(calls[1].args.keepExistingPassword, false);
});

test("ambiguous password replacement and deletion sends no IPC", async () => {
  const { controller, calls, state } = setup();
  await controller.refresh();
  controller.openEditEditor(42);
  state.editor.password = "new password";
  state.editor.removePassword = true;
  assert.equal(await controller.saveDraft(), false);
  assert.equal(calls.length, 1);
  assert.ok(state.fieldErrors.password);
});

test("cancelled deletion does not call Rust", async () => {
  const { controller, calls } = setup();
  await controller.refresh();
  assert.equal(await controller.deleteSaved(42, async () => false), false);
  assert.equal(calls.length, 1);
});

test("confirmed deletion calls exact IPC and refreshes only on success", async () => {
  const { controller, calls, state } = setup(async (command) =>
    command === "list_connections" ? [example()] : undefined);
  await controller.refresh();
  controller.openEditEditor(42);
  assert.equal(await controller.deleteSaved(42, async (connection) => connection.name === "prod"), true);
  assert.deepEqual(calls.map((item) => item.command), ["list_connections", "delete_saved_connection", "list_connections"]);
  assert.deepEqual(calls[1].args, { id: 42 });
  assert.equal(state.editor, null);
});

test("failed deletion does not lie about local cleanup", async () => {
  const { controller, state } = setup(async (command) => {
    if (command === "list_connections") return [example()];
    throw new Error("workspace cleanup failed");
  });
  await controller.refresh();
  assert.equal(await controller.deleteSaved(42, () => true), false);
  assert.equal(state.items[0].id, 42);
  assert.match(state.error, /workspace cleanup failed/);
});

test("stale list response from previous auth generation is rejected", async () => {
  const pending = deferred();
  const { shell, state, controller } = setup(async () => pending.promise);
  const request = controller.refresh();
  leaveShellWorkspace(shell);
  controller.reset();
  enterShellWorkspace(shell, "next-user");
  pending.resolve([example()]);
  assert.equal(await request, false);
  assert.deepEqual(state.items, []);
  assert.equal(state.loading, false);
});

test("an older list result never overwrites a newer one", async () => {
  const earlier = deferred();
  let count = 0;
  const { state, controller } = setup(async () => ++count === 1 ? earlier.promise : [example({ name: "current" })]);
  const first = controller.refresh();
  assert.equal(await controller.refresh(), true);
  earlier.resolve([example({ name: "outdated" })]);
  assert.equal(await first, false);
  assert.equal(state.items[0].name, "current");
});

test("a late probe result cannot repopulate status after logout", async () => {
  const pending = deferred();
  const { shell, state, controller } = setup(async (command) =>
    command === "list_connections" ? [example()] : pending.promise);
  await controller.refresh();
  const probe = controller.probeSaved(42);
  leaveShellWorkspace(shell);
  controller.reset();
  pending.resolve("connected");
  assert.equal(await probe, false);
  assert.deepEqual(state.statusById, {});
});

test("last successful check remains visible after transient status expires, then clears on logout/reset", async () => {
  const { state, controller, timers } = setup(async (command) =>
    command === "list_connections" ? [example()] : "postgres|reader|16.15 (Debian 16.15)", {
      now: () => 1_000_000,
    });
  await controller.refresh();
  assert.equal(await controller.probeSaved(42), true);
  assert.equal(state.lastProbeById[42].kind, "success");
  assert.equal(state.lastProbeById[42].checkedAt, 1_000_000);
  assert.equal(state.lastProbeById[42].serverVersion, "16.15");
  assert.equal(timers.size, 1);
  for (const fn of timers.values()) fn();
  assert.deepEqual(state.statusById, {});
  assert.equal(state.lastProbeById[42].kind, "success");
  controller.reset();
  assert.deepEqual(state.lastProbeById, {});
  assert.equal(timers.size, 0);
});

test("active terminal summary is filtered without creating a terminal", () => {
  assert.deepEqual(activeSessionsForConnection([
    { id: 1, connection_id: 42, running: true },
    { id: 2, connection_id: 42, running: false },
    { id: 3, connection_id: 3, running: true },
  ], 42).map((session) => session.id), [1]);
});

test("a draft probe response cannot attach to a newly opened editor", async () => {
  const pending = deferred();
  const { controller, state } = setup(async (command) =>
    command === "list_connections" ? [example()] : pending.promise);
  await controller.refresh();
  controller.openEditEditor(42);
  const probe = controller.probeDraft();
  controller.openNewEditor();
  pending.resolve("old probe");
  assert.equal(await probe, false);
  assert.equal(state.editorStatus, null);
  assert.equal(state.editor.name, "");
});

test("a probe result for a changed draft is not reported as a valid new draft", async () => {
  const pending = deferred();
  const { controller, state } = setup(async (command) =>
    command === "list_connections" ? [example()] : pending.promise);
  await controller.refresh();
  controller.openEditEditor(42);
  const probe = controller.probeDraft();
  state.editor.host = "another.example.org";
  pending.resolve("old host success");
  assert.equal(await probe, false);
  assert.equal(state.editorStatus, null);
  assert.equal(state.probingDraft, false);
});

test("a second mutation cannot overlap confirmation and confirmed deletion", async () => {
  const confirmation = deferred();
  const { controller, calls } = setup();
  await controller.refresh();
  const pendingDeletion = controller.deleteSaved(42, async () => confirmation.promise);
  controller.openEditEditor(42);
  assert.equal(await controller.saveDraft(), false);
  confirmation.resolve(true);
  assert.equal(await pendingDeletion, true);
  assert.equal(calls.filter((call) => call.command === "delete_saved_connection").length, 1);
});

test("successful draft check status expires after 10 seconds", async () => {
  const { controller, state, timers } = setup();
  await controller.refresh();
  controller.openEditEditor(42);
  assert.equal(await controller.probeDraft(), true);
  assert.equal(timers.size, 1);
  for (const timer of timers.values()) timer();
  assert.equal(state.editorStatus, null);
  assert.equal(timers.size, 0);
});


test("saved-password destination guard detects target changes, not harmless rename", () => {
  const original = example();
  const draft = editConnectionDraft(original);
  assert.equal(savedPasswordTargetChanged(original, draft), false);
  draft.name = "renamed";
  assert.equal(savedPasswordTargetChanged(original, draft), false);
  for (const [key, value] of [
    ["host", "other.example.org"], ["port", "6432"], ["database", "otherdb"],
    ["username", "other_user"], ["sslmode", "require"],
  ]) {
    const changed = { ...draft, [key]: value };
    assert.equal(savedPasswordTargetChanged(original, changed), true, key);
    changed.removePassword = true;
    assert.equal(savedPasswordTargetChanged(original, changed), false, `remove ${key}`);
    changed.removePassword = false;
    changed.password = "a replacement password";
    assert.equal(savedPasswordTargetChanged(original, changed), false, `replacement ${key}`);
  }
});

test("changed destination fails closed without consent callback for draft probe and save", async () => {
  const { controller, state, calls } = setup();
  await controller.refresh();
  controller.openEditEditor(42);
  state.editor.host = "other.example.org";
  assert.equal(await controller.probeDraft(), false);
  assert.match(state.editorStatus.text, /подтверждение/);
  assert.equal(await controller.saveDraft(), false);
  assert.deepEqual(calls.map((call) => call.command), ["list_connections"]);
  assert.equal(state.mutating, false);
  assert.equal(state.probingDraft, false);
});

test("explicit consent permits reuse with original IPC contract", async () => {
  const actions = [];
  const { controller, state, calls } = setup(undefined, {
    confirmPasswordReuse: (old, draft, action) => {
      assert.equal(old.host, "db.example.org");
      assert.equal(draft.host, "other.example.org");
      assert.equal(draft.password, "");
      actions.push(action);
      return true;
    },
  });
  await controller.refresh();
  controller.openEditEditor(42);
  state.editor.host = "other.example.org";
  assert.equal(await controller.probeDraft(), true);
  assert.equal(await controller.saveDraft(), true);
  assert.deepEqual(actions, ["probe", "save"]);
  assert.equal(calls[1].args.useSavedPassword, true);
  assert.equal(calls[2].args.keepExistingPassword, true);
});

test("cancelled reuse confirmation cannot send the stored password", async () => {
  const { controller, state, calls } = setup(undefined, { confirmPasswordReuse: () => false });
  await controller.refresh();
  controller.openEditEditor(42);
  state.editor.database = "otherdb";
  assert.equal(await controller.probeDraft(), false);
  assert.equal(await controller.saveDraft(), false);
  assert.equal(calls.length, 1);
});

test("stale consent after logout cannot start a Tauri call", async () => {
  const gate = deferred();
  const { controller, state, shell, calls } = setup(undefined, {
    confirmPasswordReuse: () => gate.promise,
  });
  await controller.refresh();
  controller.openEditEditor(42);
  state.editor.host = "another.example.org";
  const pending = controller.saveDraft();
  leaveShellWorkspace(shell);
  controller.reset();
  gate.resolve(true);
  assert.equal(await pending, false);
  assert.equal(calls.length, 1);
  assert.equal(state.items.length, 0);
  assert.equal(state.editorStatus, null);
});

test("editor draft is cleared when the controller resets", async () => {
  const { controller, state } = setup();
  controller.openNewEditor();
  state.editor.password = "transient-secret";
  const draft = state.editor;
  controller.reset();
  assert.equal(state.editor, null);
  assert.equal(draft.password, "");
});


test("saved connection status timer works with Vue-like reactive proxy identity", async () => {
  const shell = createShellState();
  enterShellWorkspace(shell, "owner");
  const state = createConnectionsState();
  const rawStatuses = {};
  state.statusById = new Proxy(rawStatuses, {
    get(target, key) {
      const value = Reflect.get(target, key);
      return value && typeof value === "object" ? new Proxy(value, {}) : value;
    },
  });
  const timers = new Map();
  let next = 0;
  const controller = createConnectionsController(shell, state, async (command) =>
    command === "list_connections" ? [example()] : "connected", {
    setTimer: (fn) => { const id = ++next; timers.set(id, fn); return id; },
    clearTimer: (id) => { timers.delete(id); },
  });
  await controller.refresh();
  assert.equal(await controller.probeSaved(42), true);
  assert.equal(rawStatuses[42].kind, "success");
  const task = [...timers.values()][0];
  task();
  assert.equal(Object.hasOwn(rawStatuses, 42), false);
  assert.equal(state.lastProbeById[42].kind, "success");
});


test("version is shown only when the real three-column probe response contains one", () => {
  assert.equal(postgresVersionFromProbe("postgres|reader|16.15 (Debian 16.15)"), "16.15");
  assert.equal(postgresVersionFromProbe("postgres|reader|17"), "17");
  assert.equal(postgresVersionFromProbe("ok"), null);
  assert.equal(postgresVersionFromProbe("postgres|reader|n/a"), null);
  assert.equal(postgresVersionFromProbe("postgres|reader|16\n17"), null);
  assert.match(formatProbeTimestamp(1_000_000), /1970/);
});

test("failure retains timestamp and technical error as previous check, not an active session", async () => {
  const { state, controller } = setup(async (command) => {
    if (command === "list_connections") return [example()];
    throw new Error("connection refused");
  }, { now: () => 2_000_000 });
  await controller.refresh();
  assert.equal(await controller.probeSaved(42), false);
  assert.equal(state.lastProbeById[42].checkedAt, 2_000_000);
  assert.equal(state.lastProbeById[42].kind, "error");
  assert.match(state.lastProbeById[42].response, /connection refused/);
});

test("changing the profile invalidates its check, even if saved password flag stays the same", async () => {
  const { state, controller } = setup(async (command) =>
    command === "list_connections" ? [example()] : "postgres|reader|16.15");
  await controller.refresh();
  assert.equal(await controller.probeSaved(42), true);
  controller.openEditEditor(42);
  state.editor.password = "different-secret";
  assert.equal(await controller.saveDraft(), true);
  assert.deepEqual(state.lastProbeById, {});
});

test("a refreshed profile with different destination invalidates an old successful check", async () => {
  let changed = false;
  const { state, controller } = setup(async (command) => {
    if (command === "list_connections") return [example({ host: changed ? "other.example.org" : "db.example.org" })];
    return "postgres|reader|16.15";
  });
  await controller.refresh();
  assert.equal(await controller.probeSaved(42), true);
  changed = true;
  await controller.refresh();
  assert.deepEqual(state.lastProbeById, {});
});

test("a late old-server probe must not publish a result after an external profile change", async () => {
  const pending = deferred();
  let changed = false;
  const { state, controller } = setup(async (command) => {
    if (command === "list_connections") return [example({ host: changed ? "other.example.org" : "db.example.org" })];
    return pending.promise;
  });
  await controller.refresh();
  const probe = controller.probeSaved(42);
  changed = true;
  await controller.refresh();
  pending.resolve("postgres|reader|16.15");
  assert.equal(await probe, false);
  assert.deepEqual(state.lastProbeById, {});
});

test("rechecking replaces an earlier success with the actual later failure", async () => {
  let second = false;
  const { state, controller } = setup(async (command) => {
    if (command === "list_connections") return [example()];
    if (second) throw new Error("server unavailable");
    return "postgres|reader|16.15";
  });
  await controller.refresh();
  assert.equal(await controller.probeSaved(42), true);
  assert.equal(state.lastProbeById[42].serverVersion, "16.15");
  second = true;
  assert.equal(await controller.probeSaved(42), false);
  assert.equal(state.lastProbeById[42].kind, "error");
  assert.equal(state.lastProbeById[42].serverVersion, null);
  assert.match(state.lastProbeById[42].response, /server unavailable/);
});

test("refreshing unchanged connection metadata retains the in-memory last check", async () => {
  const { state, controller } = setup(async (command) =>
    command === "list_connections" ? [example()] : "postgres|reader|16.15");
  await controller.refresh();
  await controller.probeSaved(42);
  assert.equal(await controller.refresh(), true);
  assert.equal(state.lastProbeById[42].serverVersion, "16.15");
});


test("persisted probe summary is restored from Rust without retaining secrets", async () => {
  const saved = example({
    last_probe: { kind: "success", checked_at_ms: 1_800_000_000_000, server_version: "16.15" },
    password: "must-never-retain", extra_error: "sensitive",
  });
  const { state, controller } = setup(async () => [saved]);
  assert.equal(await controller.refresh(), true);
  assert.deepEqual(state.lastProbeById[42], {
    kind: "success", checkedAt: 1_800_000_000_000,
    response: "", serverVersion: "16.15", restored: true,
  });
  assert.equal(Object.hasOwn(state.items[0], "password"), false);
  assert.equal(Object.hasOwn(state.items[0], "extra_error"), false);
});

test("Rust null observation removes an old success, including after a password change", async () => {
  let persisted = { kind: "success", checked_at_ms: 1_800_000_000_000, server_version: "16.15" };
  const { state, controller } = setup(async () => [example({ last_probe: persisted })]);
  await controller.refresh();
  assert.equal(state.lastProbeById[42].kind, "success");
  persisted = null;
  await controller.refresh();
  assert.equal(state.lastProbeById[42], undefined);
});

test("malformed persisted observation does not overwrite the current list", async () => {
  let invalid = false;
  const { state, controller } = setup(async () => [example({ last_probe: invalid ? { kind: "success", checked_at_ms: "x", server_version: "16" } : null })]);
  assert.equal(await controller.refresh(), true);
  invalid = true;
  assert.equal(await controller.refresh(), false);
  assert.match(state.error, /last_probe/);
});

test("logout clears in-memory probe, next login reloads from SQLite", async () => {
  const value = { kind: "success", checked_at_ms: 1_800_000_000_000, server_version: "16.15" };
  const { shell, state, controller } = setup(async () => [example({ last_probe: value })]);
  await controller.refresh();
  leaveShellWorkspace(shell);
  controller.reset();
  assert.deepEqual(state.lastProbeById, {});
  enterShellWorkspace(shell, "owner");
  await controller.refresh();
  assert.equal(state.lastProbeById[42].kind, "success");
});
