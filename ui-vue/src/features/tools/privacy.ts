import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";
import type { ToolScope } from "./controller.js";
export type PrivacyMode = "text" | "text_known" | "sql" | "explain" | "result";
export interface PrivacyPrefill { input: string; mode: PrivacyMode; scopeId: number | null; fileName: string }
export type PrivacyPolicy = { default: string; fields: Record<string, unknown>; patterns: unknown[] };
export interface PrivacyScope extends ToolScope {
  mapping_path: string;
  result_privacy: PrivacyPolicy;
}
export interface MappingInfo {
  path: string; identifiers: number; strings: number; integers: number;
  floats: number; bitstrings: number; redactions: number;
}
export interface MappingEntry { category: string; original: string; alias: string; kind: string | null }
export interface PrivacyState {
  connectionId: number | null; scopeId: number | null; scopes: PrivacyScope[];
  mapping: MappingInfo | null; entries: MappingEntry[]; output: string; warnings: string[];
  busy: boolean; error: string; notice: string;
}
export function createPrivacyState(): PrivacyState {
  return { connectionId: null, scopeId: null, scopes: [], mapping: null, entries: [], output: "", warnings: [], busy: false, error: "", notice: "" };
}
export function createPrivacyController(shell: ShellState, state: PrivacyState, invoke: PgXrayInvoke) {
  let revision = 0;
  const valid = (id: number, generation: number, connectionId: number | null, scopeId: number | null) =>
    revision === id && shell.authGeneration === generation && isCurrentAuthGeneration(shell, generation)
    && state.connectionId === connectionId && state.scopeId === scopeId;
  function reset(connectionId: number | null = null) {
    ++revision;
    Object.assign(state, createPrivacyState(), { connectionId });
  }
  async function refreshScopes(): Promise<void> {
    const connectionId = state.connectionId;
    const id = ++revision, auth = shell.authGeneration;
    if (!connectionId) { state.scopes = []; state.scopeId = null; state.mapping = null; state.entries = []; return; }
    state.busy = true; state.error = "";
    try {
      const scopes = await invoke<PrivacyScope[]>("list_privacy_scopes", { connectionId });
      if (valid(id, auth, connectionId, state.scopeId)) {
        state.scopes = scopes;
        if (!scopes.some(s => s.id === state.scopeId)) state.scopeId = scopes.find(s => s.is_default)?.id ?? scopes[0]?.id ?? null;
      }
    } catch (error) { if (valid(id, auth, connectionId, state.scopeId)) state.error = String(error); }
    finally { if (revision === id && isCurrentAuthGeneration(shell, auth)) state.busy = false; }
    if (state.connectionId === connectionId && isCurrentAuthGeneration(shell, auth)) await refreshMapping();
  }
  async function refreshMapping(query = "", category: string | null = null): Promise<void> {
    const connectionId = state.connectionId, scopeId = state.scopeId;
    const id = ++revision, auth = shell.authGeneration;
    state.mapping = null; state.entries = [];
    if (!scopeId) return;
    state.busy = true; state.error = "";
    try {
      const [mapping, entries] = await Promise.all([
        invoke<MappingInfo>("privacy_mapping_info", { scopeId }),
        invoke<MappingEntry[]>("privacy_mapping_entries", { scopeId, query: query.trim() || null, category, limit: 500 }),
      ]);
      if (valid(id, auth, connectionId, scopeId)) { state.mapping = mapping; state.entries = entries; }
    } catch (error) { if (valid(id, auth, connectionId, scopeId)) state.error = String(error); }
    finally { if (valid(id, auth, connectionId, scopeId)) state.busy = false; }
  }
  async function operate(work: (scopeId: number) => Promise<unknown>, notice: string, reload = true): Promise<boolean> {
    const scopeId = state.scopeId, connectionId = state.connectionId;
    if (!scopeId || state.busy) return false;
    const id = ++revision, auth = shell.authGeneration;
    state.busy = true; state.error = ""; state.notice = "";
    try {
      await work(scopeId);
      if (!valid(id, auth, connectionId, scopeId)) return false;
      state.notice = notice;
      state.busy = false;
      if (reload) await refreshMapping();
      return true;
    } catch (error) { if (valid(id, auth, connectionId, scopeId)) state.error = String(error); return false; }
    finally { if (valid(id, auth, connectionId, scopeId)) state.busy = false; }
  }
  async function transform(mode: PrivacyMode, operation: "anonymize" | "deanonymize", input: string): Promise<boolean> {
    if (!input) { state.error = t('tsFinish.privacyEnterText'); return false; }
    const generation = shell.authGeneration;
    return operate(async scopeId => {
      const result = await invoke<{ output: string; mapping: MappingInfo; warnings: string[] }>("privacy_transform", { scopeId, mode, operation, input });
      if (state.scopeId === scopeId && isCurrentAuthGeneration(shell, generation)) { state.output = result.output; state.mapping = result.mapping; state.warnings = result.warnings; }
    }, t('tsFinish.privacyCompleted'), false);
  }
  async function createScope(name: string, connectionOnly: boolean): Promise<boolean> {
    if (!state.connectionId || !name.trim() || state.busy) return false;
    const connectionId = state.connectionId, auth = shell.authGeneration;
    state.busy = true; state.error = "";
    try {
      const created = await invoke<PrivacyScope>("create_privacy_scope", { name: name.trim(), connectionId: connectionOnly ? connectionId : null });
      if (state.connectionId !== connectionId || !isCurrentAuthGeneration(shell, auth)) return false;
      state.busy = false;
      await refreshScopes();
      state.scopeId = created.id;
      await refreshMapping();
      state.notice = t('tsFinish.privacyScopeCreated', { id: String(created.id) });
      return true;
    } catch (error) { if (state.connectionId === connectionId && isCurrentAuthGeneration(shell, auth)) state.error = String(error); return false; }
    finally { state.busy = false; }
  }
  async function savePolicy(policy: PrivacyPolicy): Promise<boolean> {
    const ok = await operate(async scopeId => {
      const updated = await invoke<PrivacyScope>("update_privacy_scope_result_policy", { scopeId, resultPrivacy: policy });
      state.scopes = state.scopes.map(item => item.id === updated.id ? updated : item);
    }, t('tsFinish.privacySaved'), false);
    if (ok) { state.output = ""; state.warnings = []; }
    return ok;
  }
  async function resetMapping(): Promise<boolean> {
    return operate(scopeId => invoke("reset_privacy_mapping", { scopeId, force: true }), t('tsFinish.privacyReset'));
  }
  async function deleteEntries(entries: MappingEntry[]): Promise<boolean> {
    if (!entries.length) return false;
    return operate(scopeId => invoke("delete_privacy_mapping_entries", {
      scopeId, entries: entries.map(item => ({ category: item.category, original: item.original })), force: true,
    }), t('tsFinish.privacyMappingRemoved', { count: String(entries.length) }));
  }
  async function deleteScope(): Promise<boolean> {
    const scope = state.scopes.find(item => item.id === state.scopeId);
    if (!scope || scope.is_default) return false;
    const scopeId = scope.id;
    const ok = await operate(() => invoke("delete_privacy_scope", { scopeId }), t('tsFinish.privacyDeleted'), false);
    if (ok) { state.scopeId = null; await refreshScopes(); }
    return ok;
  }
  return { reset, refreshScopes, refreshMapping, transform, createScope, savePolicy, resetMapping, deleteEntries, deleteScope };
}
