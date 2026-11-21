import assert from "node:assert/strict";
import test from "node:test";
import { createShellState, enterShellWorkspace, leaveShellWorkspace } from "../dist/core/shell-state.js";
import { createSessionsController, createSessionsState, existingSession } from "../dist/features/sessions/controller.js";
import { locale } from "../dist/features/i18n/locale-service.js";

// These existing assertions explicitly verify the Russian wording.
// The application itself continues to default to English.
locale.value = "ru";

const example = (id = 1, override = {}) => ({
  id, connection_id: 10, connection_name: "test-dba", case_id: null,
  workspace_path: "/tmp/pgxray-test", host: "127.0.0.1", port: 5432,
  database: "postgres", username: "reader", sslmode: "verify-full",
  starts_read_only: true, idle_in_transaction_timeout_ms: 300000,
  running: true, psqlrc_path: "/tmp/test.psqlrc", editor: "/bin/vi", ...override,
});
function deferred() {
  let resolve;
  const promise = new Promise(ok => { resolve = ok; });
  return { promise, resolve };
}
function setup(invokeRaw) {
  const shell = createShellState(); enterShellWorkspace(shell, "operator");
  const state = createSessionsState();
  const calls = []; const channels = []; const output = [];
  const invoke = async (cmd, args) => { calls.push({ cmd, args }); return invokeRaw(cmd, args); };
  const controller = createSessionsController(shell, state, invoke, {
    createChannel: () => { const ch = { onmessage: null }; channels.push(ch); return ch; },
    onOutput: (id, text) => output.push([id, text]),
  });
  return { state, shell, controller, calls, channels, output };
}
test("refresh reads Rust session list and chooses active session", async () => {
  const ctx = setup(async () => [example()]);
  assert.equal(await ctx.controller.refresh(), true);
  assert.equal(ctx.state.activeId, 1);
  assert.deepEqual(ctx.state.attachedIds, [1]);
  assert.equal(ctx.calls[1].cmd, "subscribe_psql_session");
  assert.equal(existingSession(ctx.state, 10, null)?.id, 1);
});
test("existing sessions recover output subscription without another psql process", async () => {
  const ctx = setup(async cmd => cmd === "list_psql_sessions" ? [example()] : undefined);
  await ctx.controller.refresh();
  assert.equal((await ctx.controller.start(10)).id, 1);
  assert.deepEqual(ctx.state.attachedIds, [1]);
  assert.equal(ctx.calls.filter(c => c.cmd === "start_psql_session").length, 0);
});

test("reattachment failure leaves a running session without a fabricated channel", async () => {
  const ctx = setup(async cmd => {
    if (cmd === "list_psql_sessions") return [example()];
    if (cmd === "subscribe_psql_session") throw Error("PTY channel unavailable");
  });
  await ctx.controller.refresh();
  assert.deepEqual(ctx.state.attachedIds, []);
  await assert.rejects(ctx.controller.start(10), /восстановить подписку/);
  assert.match(ctx.state.error, /PTY channel unavailable/);
});
test("new session receives channel, preserves early output and supplies correct inputs", async () => {
  const ctx = setup(async (cmd, args) => {
    if (cmd === "start_psql_session") {
      args.onEvent.onmessage({ kind: "output", session_id: 5, data: "output-before-start" });
      return { created: true, session: example(5) };
    }
    if (cmd === "send_psql_session_input") return undefined;
    return [];
  });
  const session = await ctx.controller.start(10, null, false, 110, 34);
  assert.equal(session.id, 5);
  assert.deepEqual(ctx.output, [[5, "output-before-start"]]);
  assert.equal(ctx.state.attachedIds.includes(5), true);
  await ctx.controller.send(5, "select 1;\r");
  assert.equal(ctx.calls[1].cmd, "send_psql_session_input");
  assert.deepEqual(ctx.calls[1].args, {
    sessionId: 5, connectionId: 10, caseId: null, data: "select 1;\r",
  });
});
test("input requests are serialized, preserving typing and paste order", async () => {
  const gate = deferred();
  const ctx = setup(async (cmd) => {
    if (cmd === "start_psql_session") return { created: true, session: example(5) };
    if (cmd === "send_psql_session_input" && ctx.calls.filter(x => x.cmd === cmd).length === 1) return gate.promise;
    return [];
  });
  await ctx.controller.start(10);
  const first = ctx.controller.send(5, "a");
  const second = ctx.controller.send(5, "b");
  await Promise.resolve(); await Promise.resolve();
  assert.equal(ctx.calls.filter(c => c.cmd === "send_psql_session_input").length, 1);
  gate.resolve();
  await Promise.all([first, second]);
  assert.deepEqual(ctx.calls.filter(c => c.cmd === "send_psql_session_input").map(c => c.args.data), ["a", "b"]);
});
test("logout prevents publishing delayed session refresh", async () => {
  const gate = deferred();
  const ctx = setup(async () => gate.promise);
  const refreshing = ctx.controller.refresh();
  leaveShellWorkspace(ctx.shell);
  ctx.controller.reset();
  gate.resolve([example()]);
  assert.equal(await refreshing, false);
  assert.deepEqual(ctx.state.items, []);
});
test("logout ignores late PTY output", async () => {
  const ctx = setup(async () => ({ created: true, session: example(6) }));
  await ctx.controller.start(10);
  leaveShellWorkspace(ctx.shell);
  ctx.channels[0].onmessage({ kind: "output", session_id: 6, data: "private" });
  assert.deepEqual(ctx.output, []);
});
test("Tauri created=false attaches the real output channel", async () => {
  const ctx = setup(async () => ({ created: false, session: example(8) }));
  await ctx.controller.start(10);
  assert.deepEqual(ctx.state.attachedIds, [8]);
  assert.equal(typeof ctx.channels[0].onmessage, "function");
  ctx.channels[0].onmessage({ kind: "output", session_id: 8, data: "reconnected" });
  assert.deepEqual(ctx.output, [[8, "reconnected"]]);
});

test("PTY stream end does not release a session; confirmed process exit does", async () => {
  const ctx = setup(async () => ({ created: true, session: example(24) }));
  await ctx.controller.start(10);
  ctx.channels[0].onmessage({ kind: "stream_ended", session_id: 24 });
  assert.equal(ctx.state.items[0].running, true);
  ctx.channels[0].onmessage({ kind: "exit", session_id: 24 });
  assert.equal(ctx.state.items[0].running, false);
});

test("output gap is visible and does not reveal dropped SQL", async () => {
  const ctx = setup(async () => ({ created: true, session: example(25) }));
  await ctx.controller.start(10);
  ctx.channels[0].onmessage({ kind: "output_gap", session_id: 25 });
  assert.match(ctx.state.error, /Часть вывода терминала #25 была пропущена/);
});
test("close rejects errors without claiming the session was closed", async () => {
  const ctx = setup(async cmd => {
    if (cmd === "start_psql_session") return { created: true, session: example(9) };
    if (cmd === "close_psql_session") throw Error("PTY busy");
  });
  await ctx.controller.start(10);
  assert.equal(await ctx.controller.close(9), false);
  assert.equal(ctx.state.items.length, 1);
  assert.match(ctx.state.error, /PTY busy/);
});
test("close success removes attached session and clears event handler", async () => {
  const ctx = setup(async cmd => cmd === "start_psql_session" ? { created: true, session: example(9) } : undefined);
  await ctx.controller.start(10);
  assert.equal(await ctx.controller.close(9), true);
  assert.equal(ctx.state.items.length, 0);
  assert.equal(ctx.channels[0].onmessage, null);
});
test("early psql exit is not mistaken for a live attached channel", async () => {
  const ctx = setup(async (cmd, args) => {
    args.onEvent.onmessage({ kind: "exit", session_id: 11 });
    return { created: true, session: example(11) };
  });
  const result = await ctx.controller.start(10);
  assert.equal(result.running, false);
  assert.deepEqual(ctx.state.attachedIds, []);
  assert.match(ctx.state.error, /Поток вывода psql закрылся/);
});
test("malformed Tauri session response is rejected", async () => {
  const ctx = setup(async () => [example(1, { port: -1 })]);
  assert.equal(await ctx.controller.refresh(), false);
  assert.match(ctx.state.error, /Некорректный ответ/);
});
test("stale list cannot resurrect a session after close", async () => {
  const gate = deferred();
  const ctx = setup(async cmd => {
    if (cmd === "start_psql_session") return { created: true, session: example(12) };
    if (cmd === "list_psql_sessions") return gate.promise;
  });
  await ctx.controller.start(10);
  const reading = ctx.controller.refresh();
  assert.equal(await ctx.controller.close(12), true);
  gate.resolve([example(12)]);
  assert.equal(await reading, false);
  assert.equal(ctx.state.items.length, 0);
});
test("early process exit retains PTY output for diagnosing failed startup", async () => {
  const ctx = setup(async (cmd, args) => {
    if (cmd === "start_psql_session") {
      args.onEvent.onmessage({ kind: "output", session_id: 14, data: "psql: error: connection refused\\r\\n" });
      args.onEvent.onmessage({ kind: "exit", session_id: 14 });
      return { created: true, session: example(14) };
    }
    return [];
  });
  const session = await ctx.controller.start(10);
  assert.equal(session.running, false);
  assert.deepEqual(ctx.state.attachedIds, []);
  assert.match(ctx.state.exitOutput[14], /connection refused/);
  assert.match(ctx.state.error, /закрылся при запуске/);
  assert.equal(ctx.state.items.find(s => s.id === 14)?.running, false);
  ctx.controller.reset();
  assert.deepEqual(ctx.state.exitOutput, {});
});
test("PTY output is retained if process exits after start reply but before terminal mounts", async () => {
  const ctx = setup(async () => ({ created: true, session: example(15) }));
  await ctx.controller.start(10);
  ctx.channels[0].onmessage({ kind: "output", session_id: 15, data: "FATAL: connection limit reached" });
  ctx.channels[0].onmessage({ kind: "exit", session_id: 15 });
  assert.equal(ctx.state.items.find(s => s.id === 15)?.running, false);
  assert.match(ctx.state.exitOutput[15], /connection limit reached/);
  assert.deepEqual(ctx.state.attachedIds, []);
});


test("terminal starts with a single role-based mode, without compatibility flags", async () => {
  const ctx = setup(async cmd => cmd === "start_psql_session" ? { created: true, session: example(21, { starts_read_only: false, idle_in_transaction_timeout_ms: 0 }) } : undefined);
  await ctx.controller.start(10, null, true, 120, 30);
  const args = ctx.calls.find(c => c.cmd === "start_psql_session")?.args;
  assert.equal(Object.hasOwn(args, "poolerCompatibility"), false);
  assert.equal(ctx.state.items[0].starts_read_only, false);
  assert.equal(ctx.state.items[0].idle_in_transaction_timeout_ms, 0);
});

test("close reconciles an already-finished backend cleaned up by Rust", async () => {
  const ctx = setup(async (cmd, args) => {
    if (cmd === "start_psql_session") return { created: true, session: example(22) };
    if (cmd === "close_psql_session") throw "PostgreSQL-сессия не найдена";
  });
  await ctx.controller.start(10);
  ctx.channels[0].onmessage({ kind: "exit", session_id: 22 });
  assert.equal(ctx.state.items[0].running, false);
  assert.equal(await ctx.controller.close(22), true);
  assert.equal(ctx.state.items.length, 0);
});

test("close does not swallow a missing process for a session still marked running", async () => {
  const ctx = setup(async cmd => {
    if (cmd === "start_psql_session") return { created: true, session: example(23) };
    if (cmd === "close_psql_session") throw "PostgreSQL-сессия не найдена";
  });
  await ctx.controller.start(10);
  assert.equal(await ctx.controller.close(23), false);
  assert.equal(ctx.state.items.length, 1);
  assert.match(ctx.state.error, /не найдена/);
});


test("terminal starts with a single role-based mode, without compatibility flags", async () => {
  const ctx = setup(async cmd => cmd === "start_psql_session" ? { created: true, session: example(21, { starts_read_only: false, idle_in_transaction_timeout_ms: 0 }) } : undefined);
  await ctx.controller.start(10, null, true, 120, 30);
  const args = ctx.calls.find(c => c.cmd === "start_psql_session")?.args;
  assert.equal(Object.hasOwn(args, "poolerCompatibility"), false);
  assert.equal(ctx.state.items[0].starts_read_only, false);
  assert.equal(ctx.state.items[0].idle_in_transaction_timeout_ms, 0);
});

test("close reconciles an already-finished backend cleaned up by Rust", async () => {
  const ctx = setup(async (cmd, args) => {
    if (cmd === "start_psql_session") return { created: true, session: example(22) };
    if (cmd === "close_psql_session") throw "PostgreSQL-сессия не найдена";
  });
  await ctx.controller.start(10);
  ctx.channels[0].onmessage({ kind: "exit", session_id: 22 });
  assert.equal(ctx.state.items[0].running, false);
  assert.equal(await ctx.controller.close(22), true);
  assert.equal(ctx.state.items.length, 0);
});

test("close does not swallow a missing process for a session still marked running", async () => {
  const ctx = setup(async cmd => {
    if (cmd === "start_psql_session") return { created: true, session: example(23) };
    if (cmd === "close_psql_session") throw "PostgreSQL-сессия не найдена";
  });
  await ctx.controller.start(10);
  assert.equal(await ctx.controller.close(23), false);
  assert.equal(ctx.state.items.length, 1);
  assert.match(ctx.state.error, /не найдена/);
});

test("exited psql diagnostic output does not expose ANSI terminal commands", async () => {
  const ctx = setup(async () => ({ created: true, session: example(35) }));
  await ctx.controller.start(10);
  const raw = "psql 16\r\n\x1b[?2004h\x1b[1;36mpgxray=# \x1b[0m\x1b[7mSELECT 1;\x1b[27m\r\n";
  ctx.channels[0].onmessage({ kind: "output", session_id: 35, data: raw });
  ctx.channels[0].onmessage({ kind: "exit", session_id: 35 });
  assert.match(ctx.state.exitOutput[35], /psql 16/);
  assert.match(ctx.state.exitOutput[35], /SELECT 1;/);
  assert.doesNotMatch(ctx.state.exitOutput[35], /\x1b|\?2004h|\[1;36m/);
  // The live PTY output remains unchanged for xterm.js to interpret.
  assert.equal(ctx.output.at(-1)[1], raw);
});

test("exited psql diagnostic output does not expose ANSI terminal commands", async () => {
  const ctx = setup(async () => ({ created: true, session: example(35) }));
  await ctx.controller.start(10);
  const raw = "psql 16\r\n\x1b[?2004h\x1b[1;36mpgxray=# \x1b[0m\x1b[7mSELECT 1;\x1b[27m\r\n";
  ctx.channels[0].onmessage({ kind: "output", session_id: 35, data: raw });
  ctx.channels[0].onmessage({ kind: "exit", session_id: 35 });
  assert.match(ctx.state.exitOutput[35], /psql 16/);
  assert.match(ctx.state.exitOutput[35], /SELECT 1;/);
  assert.doesNotMatch(ctx.state.exitOutput[35], /\x1b|\?2004h|\[1;36m/);
  // The live PTY output remains unchanged for xterm.js to interpret.
  assert.equal(ctx.output.at(-1)[1], raw);
});
