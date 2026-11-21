import { t } from "../i18n/locale-service.js";
import { ptyDiagnosticText } from "./pty-text.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";

export interface PsqlSessionInfo {
  id: number;
  connection_id: number;
  connection_name: string;
  case_id: number | null;
  workspace_path: string;
  host: string;
  port: number;
  database: string;
  username: string;
  sslmode: string;
  starts_read_only: boolean;
  idle_in_transaction_timeout_ms: number;
  running: boolean;
  psqlrc_path: string;
  editor: string;
}
export interface PsqlSessionStart { session: PsqlSessionInfo; created: boolean }
export type PsqlSessionEvent =
  | { kind: "output"; session_id: number; data: string }
  | { kind: "exit"; session_id: number }
  | { kind: "stream_ended"; session_id: number }
  | { kind: "output_gap"; session_id: number };

export interface SessionsState {
  items: PsqlSessionInfo[];
  activeId: number | null;
  attachedIds: number[];
  loading: boolean;
  starting: boolean;
  closingId: number | null;
  error: string | null;
  exitOutput: Record<number, string>;
}
export interface TerminalChannel { onmessage: ((event: PsqlSessionEvent) => void) | null }
export interface SessionDependencies {
  createChannel(): TerminalChannel;
  onOutput(sessionId: number, output: string): void;
  onExit?(sessionId: number): void;
}
export function createSessionsState(): SessionsState {
  return { items: [], activeId: null, attachedIds: [], loading: false, starting: false, closingId: null, error: null, exitOutput: {} };
}
export function sessionIsAttached(state: SessionsState, id: number): boolean {
  return state.attachedIds.includes(id);
}
export function existingSession(state: SessionsState, connectionId: number, caseId: number | null): PsqlSessionInfo | null {
  return state.items.find(x => x.running && x.connection_id === connectionId && x.case_id === caseId) ?? null;
}
export function sessionLabel(session: PsqlSessionInfo): string {
  return `${session.connection_name} · ${session.username}@${session.host}:${session.port}/${session.database}`;
}
function isSessionInfo(value: unknown): value is PsqlSessionInfo {
  if (!value || typeof value !== "object") return false;
  const s = value as Partial<PsqlSessionInfo>;
  return Number.isSafeInteger(s.id) && (s.id as number) > 0 &&
    Number.isSafeInteger(s.connection_id) && (s.connection_id as number) > 0 &&
    Number.isInteger(s.port) && (s.port as number) >= 1 && (s.port as number) <= 65535 &&
    ["connection_name", "workspace_path", "host", "database", "username", "sslmode", "psqlrc_path", "editor"]
      .every(key => typeof (s as Record<string, unknown>)[key] === "string") &&
    typeof s.starts_read_only === "boolean" && typeof s.running === "boolean" &&
    Number.isSafeInteger(s.idle_in_transaction_timeout_ms) && (s.idle_in_transaction_timeout_ms as number) >= 0 &&
    (s.case_id === null || (Number.isSafeInteger(s.case_id) && (s.case_id as number) > 0));
}
export function createSessionsController(
  shell: ShellState, state: SessionsState, invoke: PgXrayInvoke, deps: SessionDependencies,
) {
  let requestVersion = 0;
  const channels = new Map<number, TerminalChannel>();
  const pending = new Map<number, string>();
  const recentOutput = new Map<number, string>();
  const inputQueue = new Map<number, Promise<void>>();
  // Bounded local bootstrap buffer; the Rust relay keeps its own 64 KiB tail.
  const OUTPUT_LIMIT = 64 * 1024;
  const authorized = (generation: number) => shell.screen === "workspace" && shell.authGeneration === generation;
  const setError = (error: unknown) => { state.error = String(error); };
  function enqueueOutput(sessionId: number, data: string): void {
    pending.set(sessionId, ((pending.get(sessionId) ?? "") + data).slice(-OUTPUT_LIMIT));
  }
  function flushOutput(sessionId: number): void {
    const chunk = pending.get(sessionId);
    if (!chunk) return;
    pending.delete(sessionId);
    deps.onOutput(sessionId, chunk);
  }
  function bindChannel(channel: TerminalChannel, generation: number, expected: () => number | null,
                       earlyExits: Set<number>): void {
    channel.onmessage = event => {
      if (!authorized(generation) || !event || !Number.isSafeInteger(event.session_id)) return;
      const expectedId = expected();
      if (expectedId !== null && event.session_id !== expectedId) return;
      if (event.kind === "output" && typeof event.data === "string") {
        // Retain a bounded diagnostic tail even if psql exits before xterm mounts.
        recentOutput.set(event.session_id, ((recentOutput.get(event.session_id) ?? "") + event.data).slice(-8192));
        if (expectedId === null) enqueueOutput(event.session_id, event.data);
        else deps.onOutput(event.session_id, event.data);
      } else if (event.kind === "output_gap") {
        state.error = t('sessions.droppedOutput', { id: String(event.session_id) });
      } else if (event.kind === "stream_ended") {
        // EOF is not a failure or proof of exit. Rust sends a separate Exit
        // after the child actually terminates; do not display a false alarm.
      } else if (event.kind === "exit") {
        if (expectedId === null) earlyExits.add(event.session_id);
        // Keep the last 8 KiB in memory for diagnosis. Never write PTY output to the app log.
        const tail = recentOutput.get(event.session_id);
        if (tail) state.exitOutput[event.session_id] = ptyDiagnosticText(tail);
        recentOutput.delete(event.session_id);
        pending.delete(event.session_id);
        const session = state.items.find(s => s.id === event.session_id);
        if (session) session.running = false;
        state.attachedIds = state.attachedIds.filter(id => id !== event.session_id);
        const finishedChannel = channels.get(event.session_id);
        if (finishedChannel) finishedChannel.onmessage = null;
        channels.delete(event.session_id);
        deps.onExit?.(event.session_id);
      }
    };
  }

  async function attach(session: PsqlSessionInfo, generation: number): Promise<boolean> {
    if (!session.running || !authorized(generation)) return false;
    const channel = deps.createChannel();
    const early = new Set<number>();
    bindChannel(channel, generation, () => session.id, early);
    try {
      await invoke<void>("subscribe_psql_session", {
        sessionId: session.id, connectionId: session.connection_id,
        caseId: session.case_id, onEvent: channel,
      });
      if (!authorized(generation) || !state.items.some(item => item.id === session.id && item.running)) {
        channel.onmessage = null;
        return false;
      }
      if (early.has(session.id)) {
        channel.onmessage = null;
        return false;
      }
      channels.set(session.id, channel);
      state.attachedIds = [...channels.keys()];
      flushOutput(session.id);
      return true;
    } catch (error) {
      channel.onmessage = null;
      if (authorized(generation)) setError(error);
      return false;
    }
  }

  function reset(): void {
    requestVersion += 1;
    channels.forEach(ch => { ch.onmessage = null; });
    channels.clear(); pending.clear(); recentOutput.clear(); inputQueue.clear();
    state.items = []; state.activeId = null; state.attachedIds = [];
    state.loading = false; state.starting = false; state.closingId = null; state.error = null; state.exitOutput = {};
  }
  async function refresh(): Promise<boolean> {
    const generation = shell.authGeneration;
    const version = ++requestVersion;
    state.loading = true; state.error = null;
    try {
      const result = await invoke<unknown>("list_psql_sessions");
      if (!authorized(generation) || version !== requestVersion) return false;
      if (!Array.isArray(result) || !result.every(isSessionInfo)) throw new Error(t('sessions.invalidList'));
      const sessions = result as PsqlSessionInfo[];
      state.items = sessions;
      for (const id of Object.keys(state.exitOutput)) {
        if (!sessions.some(s => s.id === Number(id))) delete state.exitOutput[Number(id)];
      }
      const ids = new Set(sessions.filter(s => s.running).map(s => s.id));
      for (const id of channels.keys()) {
        if (!ids.has(id)) { channels.get(id)!.onmessage = null; channels.delete(id); pending.delete(id); }
      }
      state.attachedIds = [...channels.keys()];
      if (state.activeId !== null && !sessions.some(s => s.id === state.activeId)) state.activeId = null;
      if (state.activeId === null) state.activeId = sessions.find(s => s.running)?.id ?? sessions[0]?.id ?? null;
      // WebView reload does not destroy the OS process. Recover the current
      // output stream, with a bounded replay from Rust. Never attach to a
      // session from a previous auth generation or stale list response.
      for (const session of sessions) {
        if (session.running && !channels.has(session.id)) {
          await attach(session, generation);
          if (!authorized(generation) || version !== requestVersion) return false;
        }
      }
      return true;
    } catch (error) {
      if (authorized(generation) && version === requestVersion) setError(error);
      return false;
    } finally {
      if (version === requestVersion) state.loading = false;
    }
  }
  async function start(connectionId: number, caseId: number | null = null, forceNew = false, cols = 120, rows = 30): Promise<PsqlSessionInfo> {
    const generation = shell.authGeneration;
    if (!authorized(generation)) throw new Error(t('sessions.authRequired'));
    if (state.starting || state.closingId !== null) throw new Error(t('sessions.operationPending'));
    if (!forceNew) {
      const previous = existingSession(state, connectionId, caseId);
      if (previous) {
        state.activeId = previous.id;
        if (sessionIsAttached(state, previous.id)) return previous;
        if (await attach(previous, generation)) return previous;
        throw new Error(t('sessions.subscribeFailed'));
      }
    }
    ++requestVersion; // Invalidate a list response started before this session.
    state.loading = false;
    state.starting = true; state.error = null;
    const channel = deps.createChannel();
    let expectedId: number | null = null;
    const earlyExits = new Set<number>();
    bindChannel(channel, generation, () => expectedId, earlyExits);
    try {
      const result = await invoke<PsqlSessionStart>("start_psql_session", {
        connectionId, caseId, cols, rows, forceNew, onEvent: channel,
      });
      if (!authorized(generation)) throw new Error(t('sessions.authChanged'));
      if (!result || !isSessionInfo(result.session) || typeof result.created !== "boolean") throw new Error(t('sessions.invalidStart'));
      expectedId = result.session.id;
      if (earlyExits.has(expectedId)) result.session.running = false;
      state.items = [...state.items.filter(s => s.id !== expectedId), result.session].sort((a, b) => a.id - b.id);
      state.activeId = expectedId;
      if (result.session.running) {
        channels.set(expectedId, channel);
        state.attachedIds = [...channels.keys()];
        flushOutput(expectedId);
      } else {
        if (result.created) state.error = t('sessions.outputClosed');
        channel.onmessage = null;
        pending.delete(expectedId);
        if (!result.created) state.error = t('sessions.existingEnded');
      }
      return result.session;
    } catch (error) {
      channel.onmessage = null;
      if (authorized(generation)) setError(error);
      throw error;
    } finally {
      if (authorized(generation)) state.starting = false;
    }
  }
  function select(id: number): void {
    if (state.items.some(s => s.id === id)) { state.activeId = id; state.error = null; }
  }
  async function send(id: number, data: string): Promise<void> {
    const generation = shell.authGeneration;
    const session = state.items.find(s => s.id === id);
    if (!session || !session.running || !sessionIsAttached(state, id) || !authorized(generation)) return;
    if (new TextEncoder().encode(data).length > 512 * 1024) {
      state.error = t('sessions.inputTooLarge');
      return;
    }
    const previous = inputQueue.get(id) ?? Promise.resolve();
    const current = previous.catch(() => undefined).then(async () => {
      if (!authorized(generation) || !sessionIsAttached(state, id)) return;
      await invoke<void>("send_psql_session_input", {
        sessionId: session.id, connectionId: session.connection_id, caseId: session.case_id, data,
      });
    });
    inputQueue.set(id, current);
    try { await current; } catch (error) { if (authorized(generation)) setError(error); }
    finally { if (inputQueue.get(id) === current) inputQueue.delete(id); }
  }
  async function resize(id: number, cols: number, rows: number): Promise<void> {
    const generation = shell.authGeneration;
    if (!sessionIsAttached(state, id) || !authorized(generation)) return;
    if (![cols, rows].every(n => Number.isInteger(n) && n >= 1 && n <= 500)) return;
    try { await invoke<void>("resize_psql_session", { sessionId: id, cols, rows }); }
    catch (error) { if (authorized(generation)) setError(error); }
  }
  async function close(id: number): Promise<boolean> {
    const generation = shell.authGeneration;
    if (!authorized(generation) || state.closingId !== null) return false;
    const target = state.items.find(s => s.id === id);
    if (!target) return false;
    ++requestVersion; // A stale list must not resurrect a closed terminal.
    state.loading = false;
    state.closingId = id; state.error = null;
    try {
      try {
        // Even after PTY EOF, ask Rust to close the process if it still exists.
        await invoke<void>("close_psql_session", { sessionId: id });
      } catch (error) {
        // Rust cleanup_finished() may have removed an already-exited process.
        // Do not suppress other errors, or errors for sessions marked running.
        if (target.running || !/^(?:Error:\s*)?PostgreSQL-сессия не найдена$/.test(String(error).trim())) throw error;
      }
      if (!authorized(generation)) return false;
      channels.get(id) && (channels.get(id)!.onmessage = null);
      channels.delete(id); pending.delete(id); recentOutput.delete(id); inputQueue.delete(id); delete state.exitOutput[id];
      state.attachedIds = [...channels.keys()];
      state.items = state.items.filter(s => s.id !== id);
      if (state.activeId === id) state.activeId = state.items.find(s => s.running)?.id ?? null;
      return true;
    } catch (error) { if (authorized(generation)) setError(error); return false; }
    finally { if (authorized(generation)) state.closingId = null; }
  }
  return { reset, refresh, start, select, send, resize, close };
}
