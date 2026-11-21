import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";

/** Mirrors SavedConnection in crates/core/src/connections.rs; never includes the password. */
export interface SavedConnection {
  id: number;
  name: string;
  host: string;
  port: number;
  database: string;
  username: string;
  sslmode: string;
  has_password: boolean;
  project_id?: number | null;
  shortcut_slot?: number | null;
  last_probe?: PersistedConnectionProbe | null;
}

/** Mirrors ConnectionInput in crates/core/src/postgres.rs. */
export interface ConnectionInput {
  host: string;
  port: number;
  database: string;
  username: string;
  password: string;
  sslmode: "verify-full" | "require" | "disable";
}

export type ConnectionField = "name" | "host" | "port" | "database" | "username" | "password" | "sslmode";
export type ConnectionFieldErrors = Partial<Record<ConnectionField, string>>;
export type StatusKind = "pending" | "success" | "error";
export interface ConnectionStatus {
  kind: StatusKind;
  text: string;
}

/** Persisted historical outcome from Rust; never implies an active session. */
export interface PersistedConnectionProbe {
  kind: "success" | "error";
  checked_at_ms: number;
  server_version: string | null;
}

/** Last observed probe (detailed output is only kept in memory). */
export interface LastConnectionProbe {
  kind: "success" | "error";
  checkedAt: number;
  response: string;
  serverVersion: string | null;
  restored?: boolean;
}

/** The probe IPC returns three text columns: database|user|server_version. */
export function postgresVersionFromProbe(response: string): string | null {
  const parts = response.trim().split("|");
  if (parts.length !== 3 || /[\r\n]/.test(response)) return null;
  const match = /^(\d{1,3}(?:\.\d{1,2}){0,2})(?:\s|$)/.exec(parts[2].trim());
  return match?.[1] ?? null;
}

export function formatProbeTimestamp(checkedAt: number): string {
  return new Intl.DateTimeFormat("ru-RU", {
    day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit",
  }).format(checkedAt);
}

function targetFingerprint(connection: SavedConnection): string {
  return JSON.stringify([
    connection.host, connection.port, connection.database, connection.username,
    connection.sslmode, connection.has_password,
  ]);
}

/** Password is only held in the transient editor. Never put it in SavedConnection. */
export interface ConnectionDraft {
  name: string;
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  sslmode: ConnectionInput["sslmode"];
  removePassword: boolean;
}

export interface ConnectionsState {
  items: SavedConnection[];
  loading: boolean;
  mutating: boolean;
  probingIds: number[];
  probingDraft: boolean;
  statusById: Record<number, ConnectionStatus>;
  lastProbeById: Record<number, LastConnectionProbe>;
  editor: ConnectionDraft | null;
  editingId: number | null;
  fieldErrors: ConnectionFieldErrors;
  editorStatus: ConnectionStatus | null;
  error: string | null;
}

export interface SessionSummary {
  id: number;
  connection_id: number;
  running: boolean;
}

export function activeSessionsForConnection(sessions: readonly SessionSummary[], connectionId: number): SessionSummary[] {
  return sessions.filter((session) => session.connection_id === connectionId && session.running);
}

/** A running psql session must keep the target and credentials with which it was started. */
export function changesActiveSessionTarget(existing: SavedConnection, draft: ConnectionDraft): boolean {
  return existing.host !== draft.host.trim() ||
    existing.port !== Number(draft.port) ||
    existing.database !== draft.database.trim() ||
    existing.username !== draft.username.trim() ||
    existing.sslmode !== draft.sslmode ||
    draft.password !== "" || draft.removePassword;
}

export function createConnectionsState(): ConnectionsState {
  return {
    items: [], loading: false, mutating: false, probingIds: [], probingDraft: false,
    statusById: {}, lastProbeById: {}, editor: null, editingId: null, fieldErrors: {}, editorStatus: null, error: null,
  };
}

export function newConnectionDraft(): ConnectionDraft {
  return {
    name: "", host: "127.0.0.1", port: "5432", database: "postgres",
    username: "postgres", password: "", sslmode: "verify-full", removePassword: false,
  };
}

export function editConnectionDraft(connection: SavedConnection): ConnectionDraft {
  return {
    name: connection.name, host: connection.host, port: String(connection.port),
    database: connection.database, username: connection.username, password: "",
    sslmode: connection.sslmode as ConnectionInput["sslmode"], removePassword: false,
  };
}

const byteLength = (text: string): number => new TextEncoder().encode(text).length;
const validSsmode = (mode: string): mode is ConnectionInput["sslmode"] =>
  mode === "verify-full" || mode === "require" || mode === "disable";

/** Client feedback mirrors the Core restrictions; Core remains authoritative. */
export function validateConnectionDraft(draft: ConnectionDraft): ConnectionFieldErrors {
  const errors: ConnectionFieldErrors = {};
  const name = draft.name.trim();
  const host = draft.host.trim();
  const database = draft.database.trim();
  const username = draft.username.trim();
  const port = Number(draft.port);

  if (!name || [...name].length > 80 || /\p{Cc}/u.test(name)) {
    errors.name = t('tsConnections.nameLength');
  }
  if (!host || byteLength(host) > 253 || !/^[A-Za-z0-9.:-]+$/.test(host)) {
    errors.host = t('tsConnections.hostFormat');
  }
  if (!draft.port.trim() || !Number.isInteger(port) || port < 1 || port > 65535) {
    errors.port = t('tsConnections.portRange');
  }
  for (const [key, text] of [["database", database], ["username", username]] as const) {
    if (!text || byteLength(text) > 256 || /[\n\r\x00=]/.test(text) || text.includes("://")) {
      errors[key] = t('tsConnections.nameNotUri');
    }
  }
  if (byteLength(draft.password) > 1024 || /[\n\r\x00]/.test(draft.password)) {
    errors.password = t('tsConnections.passwordLength');
  }
  if (!validSsmode(draft.sslmode)) errors.sslmode = t('tsConnections.unknownTls');
  if (draft.removePassword && draft.password !== "") {
    errors.password = t('tsConnections.passwordConflict');
  }
  return errors;
}

export function connectionInput(draft: ConnectionDraft): ConnectionInput {
  return {
    host: draft.host.trim(), port: Number(draft.port), database: draft.database.trim(),
    username: draft.username.trim(), password: draft.password, sslmode: draft.sslmode,
  };
}

/** Preserve both *different* Tauri argument names for testing vs saving. */
export function passwordFlags(existing: SavedConnection | null, draft: ConnectionDraft): {
  useSavedPassword: boolean;
  keepExistingPassword: boolean;
} {
  const keep = Boolean(existing?.has_password && !draft.removePassword && draft.password === "");
  return { useSavedPassword: keep, keepExistingPassword: keep };
}

/** Field markers from the legacy form; diagnostic hints, not proof of the cause. */
export function connectionFailureFields(error: string): ConnectionFieldErrors {
  const text = error.toLowerCase();
  if (text.includes("password authentication failed") || text.includes("аутентификац") || text.includes("парол")) {
    return { username: t('tsConnections.verifyUser'), password: t('tsConnections.verifyPassword') };
  }
  if (text.includes("database") && (text.includes("does not exist") || text.includes("не существует"))) {
    return { database: t('tsConnections.verifyDatabase') };
  }
  if (["certificate", "ssl", "tls", "verify-full"].some((x) => text.includes(x))) {
    return { sslmode: t('tsConnections.verifyTls'), host: t('tsConnections.verifyHostname') };
  }
  if (["connection refused", "timeout", "timed out", "could not connect", "не удалось подключ", "адрес", "порт"].some((x) => text.includes(x))) {
    return { host: t('tsConnections.verifyHost'), port: t('tsConnections.verifyPort') };
  }
  return {};
}

function parseSavedConnection(value: unknown): SavedConnection {
  if (!value || typeof value !== "object") throw new Error(t('tsConnections.badList'));
  const v = value as Record<string, unknown>;
  if (!Number.isSafeInteger(v.id) || (v.id as number) <= 0 ||
      !Number.isInteger(v.port) || (v.port as number) < 1 || (v.port as number) > 65535 ||
      typeof v.has_password !== "boolean" ||
      ["name", "host", "database", "username", "sslmode"].some((key) => typeof v[key] !== "string")) {
    throw new Error(t('tsConnections.badList'));
  }
  if (v.project_id !== undefined && v.project_id !== null &&
      (!Number.isSafeInteger(v.project_id) || (v.project_id as number) <= 0)) {
    throw new Error(t('tsConnections.badProjectId'));
  }
  if (v.shortcut_slot !== undefined && v.shortcut_slot !== null &&
      (!Number.isInteger(v.shortcut_slot) || (v.shortcut_slot as number) < 0 || (v.shortcut_slot as number) > 9)) {
    throw new Error(t('tsConnections.badList'));
  }
  // The Rust response carries only a bounded historical summary, never secrets.
  let last_probe: PersistedConnectionProbe | null | undefined;
  if (Object.prototype.hasOwnProperty.call(v, "last_probe")) {
    if (v.last_probe === null) last_probe = null;
    else if (v.last_probe && typeof v.last_probe === "object") {
      const item = v.last_probe as Record<string, unknown>;
      if ((item.kind !== "success" && item.kind !== "error") ||
          !Number.isSafeInteger(item.checked_at_ms) || (item.checked_at_ms as number) <= 0 ||
          (item.server_version !== null && (typeof item.server_version !== "string" ||
            !/^\d{1,3}(?:\.\d{1,3}){0,2}$/.test(item.server_version)))) {
        throw new Error(t('tsConnections.badProbe'));
      }
      last_probe = {
        kind: item.kind,
        checked_at_ms: item.checked_at_ms as number,
        server_version: item.server_version as string | null,
      };
    } else throw new Error(t('tsConnections.badProbe'));
  }
  // Deliberately whitelist fields to avoid retaining a future accidental secret.
  return {
    id: v.id as number, name: v.name as string, host: v.host as string,
    port: v.port as number, database: v.database as string, username: v.username as string,
    sslmode: v.sslmode as string, has_password: v.has_password as boolean,
    ...(v.project_id !== undefined ? { project_id: v.project_id as number | null } : {}),
    ...(v.shortcut_slot !== undefined ? { shortcut_slot: v.shortcut_slot as number | null } : {}),
    ...(last_probe !== undefined ? { last_probe } : {}),
  };
}

export function parseSavedConnections(value: unknown): SavedConnection[] {
  if (!Array.isArray(value)) throw new Error(t('tsConnections.badList'));
  return value.map(parseSavedConnection);
}

/** Reusing a stored password after changing its destination requires consent. */
export function savedPasswordTargetChanged(
  existing: SavedConnection | null,
  draft: Readonly<ConnectionDraft>,
): boolean {
  if (!existing || !passwordFlags(existing, draft).keepExistingPassword) return false;
  return existing.host !== draft.host.trim() ||
    existing.port !== Number(draft.port) ||
    existing.database !== draft.database.trim() ||
    existing.username !== draft.username.trim() ||
    existing.sslmode !== draft.sslmode;
}

export interface ConnectionControllerOptions {
  confirmPasswordReuse?: (
    existing: SavedConnection,
    draft: Readonly<ConnectionDraft>,
    action: "probe" | "save",
  ) => Promise<boolean> | boolean;
  onConnectionsChanged?: (connections: SavedConnection[]) => void;
  setTimer?: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>;
  clearTimer?: (handle: ReturnType<typeof setTimeout>) => void;
  now?: () => number;
}

/** Headless port of connections.js; components come with views/connections.html next. */
export function createConnectionsController(
  shell: ShellState,
  state: ConnectionsState,
  invoke: PgXrayInvoke | null,
  options: ConnectionControllerOptions = {},
) {
  const timers = new Map<number, ReturnType<typeof setTimeout>>();
  const setTimer = options.setTimer ?? setTimeout;
  const clearTimer = options.clearTimer ?? clearTimeout;
  const now = options.now ?? Date.now;
  const probeTokens = new Map<number, number>();
  let revision = 0;
  let editorRevision = 0;
  let editorTimer: ReturnType<typeof setTimeout> | undefined;
  const allowed = () => shell.screen === "workspace" && invoke !== null;
  const fresh = (authEpoch: number, rev: number) =>
    allowed() && isCurrentAuthGeneration(shell, authEpoch) && revision === rev;

  function cancelTimer(id: number): void {
    const timer = timers.get(id);
    if (timer !== undefined) clearTimer(timer);
    timers.delete(id);
  }

  function clearEditorTimer(): void {
    if (editorTimer !== undefined) clearTimer(editorTimer);
    editorTimer = undefined;
  }

  function reset(): void {
    if (state.editor) state.editor.password = "";
    revision += 1;
    editorRevision += 1;
    clearEditorTimer();
    for (const id of timers.keys()) cancelTimer(id);
    probeTokens.clear();
    Object.assign(state, createConnectionsState());
  }

  function invalidateProbe(id: number): void {
    probeTokens.set(id, (probeTokens.get(id) ?? 0) + 1);
    cancelTimer(id);
    delete state.statusById[id];
    delete state.lastProbeById[id];
  }

  function setStatus(id: number, status: ConnectionStatus, epoch: number, rev: number): void {
    cancelTimer(id);
    state.statusById[id] = status;
    if (status.kind === "success") {
      const timer = setTimer(() => {
        // Vue reactive() may return a Proxy for the stored status, so object
        // identity (state.statusById[id] === status) is not reliable here.
        if (timers.get(id) !== timer) return;
        timers.delete(id);
        if (fresh(epoch, rev)) delete state.statusById[id];
      }, 10_000);
      timers.set(id, timer);
    }
  }

  async function refresh(): Promise<boolean> {
    if (!allowed()) return false;
    const epoch = shell.authGeneration;
    const rev = revision;
    const request = ++latestRequest;
    state.loading = true;
    try {
      const items = parseSavedConnections(await invoke!("list_connections"));
      if (!fresh(epoch, rev) || request !== latestRequest) return false;
      const previous = new Map(state.items.map((item) => [item.id, item]));
      state.items = items;
      state.error = null;
      const currentById = new Map(items.map((item) => [item.id, item]));
      const trackedIds = new Set([
        ...Object.keys(state.statusById).map(Number),
        ...Object.keys(state.lastProbeById).map(Number),
      ]);
      for (const id of trackedIds) {
        const current = currentById.get(id);
        const earlier = previous.get(id);
        if (!current || (earlier && targetFingerprint(earlier) !== targetFingerprint(current))) {
          invalidateProbe(id);
        }
      }
      // The SQLite summary is authoritative. In particular, a null value must
      // remove an old green badge after a password replacement or profile edit.
      for (const connection of items) {
        if (connection.last_probe !== undefined) {
          const persisted = connection.last_probe;
          if (persisted) {
            state.lastProbeById[connection.id] = {
              kind: persisted.kind, checkedAt: persisted.checked_at_ms,
              serverVersion: persisted.server_version, response: "", restored: true,
            };
          } else {
            delete state.lastProbeById[connection.id];
          }
        }
      }
      options.onConnectionsChanged?.(items);
      return true;
    } catch (error) {
      if (fresh(epoch, rev) && request === latestRequest) state.error = String(error);
      return false;
    } finally {
      if (fresh(epoch, rev) && request === latestRequest) state.loading = false;
    }
  }
  let latestRequest = 0;

  function openNewEditor(): void {
    if (state.mutating) return;
    state.error = null;
    editorRevision += 1;
    clearEditorTimer();
    state.editingId = null;
    state.editor = newConnectionDraft();
    state.fieldErrors = {};
    state.editorStatus = null;
  }
  function openEditEditor(id: number): boolean {
    if (state.mutating) return false;
    const item = state.items.find((candidate) => candidate.id === id);
    if (!item) return false;
    state.error = null;
    editorRevision += 1;
    clearEditorTimer();
    state.editingId = id;
    state.editor = editConnectionDraft(item);
    state.fieldErrors = {};
    state.editorStatus = null;
    return true;
  }
  function closeEditor(): void {
    editorRevision += 1;
    clearEditorTimer();
    if (state.editor) state.editor.password = "";
    state.editor = null;
    state.editingId = null;
    state.fieldErrors = {};
    state.editorStatus = null;
  }
  function validate(): boolean {
    if (!state.editor) return false;
    state.fieldErrors = validateConnectionDraft(state.editor);
    return Object.keys(state.fieldErrors).length === 0;
  }

  /** Deny by default if a stored secret would be used with a changed target. */
  async function allowSavedPasswordReuse(
    existing: SavedConnection | null,
    draft: ConnectionDraft,
    action: "probe" | "save",
  ): Promise<boolean> {
    if (!savedPasswordTargetChanged(existing, draft)) return true;
    const confirmed = await options.confirmPasswordReuse?.(
      existing!,
      { ...draft, password: "" },
      action,
    );
    return confirmed === true;
  }

  async function probeDraft(): Promise<boolean> {
    if (!allowed() || state.probingDraft || state.mutating || !validate()) return false;
    const epoch = shell.authGeneration, rev = revision;
    const draft = state.editor!;
    const draftSnapshot = { ...draft };
    const editorEpoch = editorRevision;
    const id = state.editingId;
    const existing = state.items.find((c) => c.id === id) ?? null;
    const input = connectionInput(draft);
    const stillEditingSameDraft = () => editorEpoch === editorRevision && state.editor === draft &&
      Object.keys(draftSnapshot).every((key) => draft[key as keyof ConnectionDraft] === draftSnapshot[key as keyof ConnectionDraft]);
    const useSavedPassword = passwordFlags(existing, draft).useSavedPassword;
    state.probingDraft = true;
    clearEditorTimer();
    try {
      if (!(await allowSavedPasswordReuse(existing, draft, "probe"))) {
        if (fresh(epoch, rev) && stillEditingSameDraft()) state.editorStatus = {
          kind: "error",
          text: t('tsConnections.cancelReuse'),
        };
        return false;
      }
      if (!fresh(epoch, rev) || !stillEditingSameDraft()) return false;
      state.editorStatus = { kind: "pending", text: t('tsConnections.testing') };
      const result = await invoke!<string>("probe_connection_draft", { connectionId: id, useSavedPassword, input });
      if (!fresh(epoch, rev)) return false;
      if (!stillEditingSameDraft()) {
        if (editorEpoch === editorRevision && state.editor === draft) state.editorStatus = null;
        return false;
      }
      if (typeof result !== "string") throw new Error(t('tsConnections.badDraftProbe'));
      state.editorStatus = { kind: "success", text: t('tsConnections.probeSuccess', { result }) };
      editorTimer = setTimer(() => {
        editorTimer = undefined;
        if (fresh(epoch, rev) && stillEditingSameDraft()) state.editorStatus = null;
      }, 10_000);
      return true;
    } catch (error) {
      if (fresh(epoch, rev) && !stillEditingSameDraft()) {
        if (editorEpoch === editorRevision && state.editor === draft) state.editorStatus = null;
      } else if (fresh(epoch, rev)) {
        state.fieldErrors = connectionFailureFields(String(error));
        state.editorStatus = { kind: "error", text: t('tsConnections.connectFailed', { error: String(error) }) };
      }
      return false;
    } finally {
      if (fresh(epoch, rev)) state.probingDraft = false;
    }
  }

  async function saveDraft(): Promise<boolean> {
    if (!allowed() || state.mutating || state.probingDraft) return false;
    clearEditorTimer();
    state.editorStatus = null;
    if (!validate()) return false;
    const epoch = shell.authGeneration, rev = revision;
    const draft = state.editor!;
    const draftSnapshot = { ...draft };
    const editorEpoch = editorRevision;
    const id = state.editingId;
    const existing = state.items.find((c) => c.id === id) ?? null;
    if (id !== null && !existing) {
      state.editorStatus = { kind: "error", text: t('tsConnections.editingGone') };
      return false;
    }
    const name = draft.name.trim();
    const input = connectionInput(draft);
    const keepExistingPassword = passwordFlags(existing, draft).keepExistingPassword;
    state.mutating = true;
    try {
      if (!(await allowSavedPasswordReuse(existing, draft, "save"))) {
        if (fresh(epoch, rev) && editorEpoch === editorRevision && state.editor === draft) state.editorStatus = {
          kind: "error",
          text: t('tsConnections.cancelReuse'),
        };
        return false;
      }
      if (!fresh(epoch, rev) || editorEpoch !== editorRevision || state.editor !== draft ||
          !Object.keys(draftSnapshot).every((key) => draft[key as keyof ConnectionDraft] === draftSnapshot[key as keyof ConnectionDraft])) return false;
      if (id === null) {
        await invoke!("save_connection", { name, input });
      } else {
        await invoke!("update_saved_connection", { id, name, input, keepExistingPassword });
      }
      if (!fresh(epoch, rev)) return false;
      // A successful edit may change credentials even if the public metadata is identical.
      // Never display a check made before the edit as if it validated the new profile.
      if (id !== null && existing && (
        targetFingerprint(existing) !== JSON.stringify([
          input.host, input.port, input.database, input.username, input.sslmode,
          input.password !== "" ? true : keepExistingPassword && existing.has_password,
        ]) || input.password !== "" || draft.removePassword
      )) invalidateProbe(id);
      // Preserve a different editor, or user edits made while IPC was running.
      // The saved payload was captured before the await.
      if (editorEpoch === editorRevision && state.editor === draft &&
          Object.keys(draftSnapshot).every((key) => draft[key as keyof ConnectionDraft] === draftSnapshot[key as keyof ConnectionDraft])) {
        closeEditor(); // discard the plaintext password from the editor after success
      } else if (editorEpoch === editorRevision && state.editor === draft) {
        state.editorStatus = { kind: "success", text: t('tsConnections.savedOlder') };
      }
      if (!(await refresh())) {
        if (fresh(epoch, rev)) state.error = t('tsConnections.savedRefreshFailed');
      }
      return true;
    } catch (error) {
      // Keep the reason inside the open editor. Never report a stale rejection in
      // another editor (or after logout) if an IPC request completed late.
      if (fresh(epoch, rev) && editorEpoch === editorRevision && state.editor === draft) {
        state.editorStatus = { kind: "error", text: t('tsConnections.saveFailed', { error: String(error) }) };
      }
      return false;
    } finally {
      if (fresh(epoch, rev)) state.mutating = false;
    }
  }

  async function probeSaved(id: number): Promise<boolean> {
    if (!allowed() || state.mutating || state.probingIds.includes(id)) return false;
    const connection = state.items.find((item) => item.id === id);
    if (!connection) return false;
    const epoch = shell.authGeneration, rev = revision;
    const target = targetFingerprint(connection);
    const token = (probeTokens.get(id) ?? 0) + 1;
    probeTokens.set(id, token);
    const stillCurrent = () => fresh(epoch, rev) && probeTokens.get(id) === token &&
      state.items.some((item) => item.id === id && targetFingerprint(item) === target);
    state.probingIds.push(id);
    delete state.lastProbeById[id];
    setStatus(id, { kind: "pending", text: t('tsConnections.testing') }, epoch, rev);
    try {
      const result = await invoke!<string>("probe_saved_connection", { id });
      if (!stillCurrent()) return false;
      if (typeof result !== "string") throw new Error(t('tsConnections.badSavedProbe'));
      state.lastProbeById[id] = {
        kind: "success", checkedAt: now(), response: result,
        serverVersion: postgresVersionFromProbe(result),
      };
      setStatus(id, { kind: "success", text: t('tsConnections.probeSuccess', { result }) }, epoch, rev);
      return true;
    } catch (error) {
      if (stillCurrent()) {
        const response = String(error);
        state.lastProbeById[id] = { kind: "error", checkedAt: now(), response, serverVersion: null };
          setStatus(id, { kind: "error", text: t('tsConnections.connectFailed', { error: response }) }, epoch, rev);
      }
      return false;
    } finally {
      if (fresh(epoch, rev)) state.probingIds = state.probingIds.filter((item) => item !== id);
    }
  }

  /** Must only be called after the UI has displayed the full destructive warning. */
  async function deleteSaved(id: number, confirm: (connection: SavedConnection) => Promise<boolean> | boolean): Promise<boolean> {
    if (!allowed() || state.mutating) return false;
    const connection = state.items.find((item) => item.id === id);
    if (!connection) return false;
    const epoch = shell.authGeneration, rev = revision;
    // Reserve the mutation slot before awaiting an asynchronous confirmation.
    state.mutating = true;
    try {
      if (!(await confirm(connection)) || !fresh(epoch, rev)) return false;
      await invoke!("delete_saved_connection", { id });
      if (!fresh(epoch, rev)) return false;
      invalidateProbe(id);
      if (state.editingId === id) closeEditor();
      if (!(await refresh())) {
        if (fresh(epoch, rev)) state.error = t('tsConnections.removedRefreshFailed');
      }
      return true;
    } catch (error) {
      if (fresh(epoch, rev)) state.error = String(error);
      return false;
    } finally {
      if (fresh(epoch, rev)) state.mutating = false;
    }
  }

  return { reset, refresh, openNewEditor, openEditEditor, closeEditor, validate, probeDraft, saveDraft, probeSaved, deleteSaved };
}
