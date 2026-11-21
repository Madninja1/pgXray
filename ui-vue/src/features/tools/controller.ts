import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";
import type { SavedConnection } from "../connections/controller.js";

export interface ToolCase {
  id: number;
  connection_id: number;
  scope_id: number;
  title: string;
  description: string;
  kind: string;
  status: string;
  created_at_ms: number;
  inbox_path: string;
}
export interface ToolScope { id: number; name: string; is_default: boolean; connection_id: number | null }
export interface ToolsState {
  connections: SavedConnection[];
  connectionId: number | null;
  scopes: ToolScope[];
  scopeId: number | null;
  cases: ToolCase[];
  caseId: number | null;
  loading: boolean;
  error: string;
  notice: string;
}
export function createToolsState(): ToolsState {
  return { connections: [], connectionId: null, scopes: [], scopeId: null, cases: [], caseId: null, loading: false, error: "", notice: "" };
}
export function createToolsController(shell: ShellState, state: ToolsState, invoke: PgXrayInvoke) {
  let requestId = 0;
  const valid = (generation: number, request: number) => isCurrentAuthGeneration(shell, generation) && request === requestId;
  function reset(): void {
    ++requestId;
    Object.assign(state, createToolsState());
  }
  async function refreshConnections(): Promise<void> {
    const request = ++requestId;
    const generation = shell.authGeneration;
    state.loading = true;
    state.error = "";
    try {
      const connections = await invoke<SavedConnection[]>("list_connections");
      if (!valid(generation, request)) return;
      state.connections = connections;
      if (!connections.some(item => item.id === state.connectionId)) state.connectionId = connections[0]?.id ?? null;
      await loadCases(request, generation);
    } catch (error) {
      if (valid(generation, request)) state.error = String(error);
    } finally {
      if (valid(generation, request)) state.loading = false;
    }
  }
  async function loadCases(request: number, generation: number): Promise<void> {
    const connectionId = state.connectionId;
    if (!connectionId) {
      state.scopes = []; state.scopeId = null; state.cases = []; state.caseId = null;
      return;
    }
    await invoke("ensure_connection_workspace", { connectionId });
    const [scopes, cases] = await Promise.all([
      invoke<ToolScope[]>("list_privacy_scopes", { connectionId }),
      invoke<ToolCase[]>("list_investigation_cases", { connectionId }),
    ]);
    if (!valid(generation, request)) return;
    state.scopes = scopes;
    if (!scopes.some(scope => scope.id === state.scopeId)) state.scopeId = scopes.find(scope => scope.is_default)?.id ?? scopes[0]?.id ?? null;
    state.cases = [...cases].sort((a, b) => (b.created_at_ms ?? b.id) - (a.created_at_ms ?? a.id));
    if (!state.cases.some(item => item.id === state.caseId)) state.caseId = null;
  }
  async function selectConnection(id: number | null): Promise<void> {
    state.connectionId = id;
    state.caseId = null;
    state.scopeId = null;
    state.cases = [];
    state.scopes = [];
    const request = ++requestId;
    const generation = shell.authGeneration;
    state.loading = true;
    state.error = "";
    try { await loadCases(request, generation); }
    catch (error) { if (valid(generation, request)) state.error = String(error); }
    finally { if (valid(generation, request)) state.loading = false; }
  }
  async function createCase(title: string, description: string): Promise<boolean> {
    const connectionId = state.connectionId, scopeId = state.scopeId;
    if (!connectionId || !scopeId || !title.trim()) { state.error = t('tsFinish.caseRequired'); return false; }
    const request = ++requestId;
    const generation = shell.authGeneration;
    state.loading = true;
    state.error = "";
    try {
      const created = await invoke<ToolCase>("create_investigation_case", {
        connectionId, scopeId, title: title.trim(), description: description.trim(),
      });
      if (!valid(generation, request)) return false;
      await loadCases(request, generation);
      if (!valid(generation, request)) return false;
      state.caseId = created.id;
      state.notice = t('tsFinish.caseCreated', { id: String(created.id) });
      return true;
    } catch (error) {
      if (valid(generation, request)) state.error = String(error);
      return false;
    } finally {
      if (valid(generation, request)) state.loading = false;
    }
  }
  return { refreshConnections, selectConnection, createCase, reset };
}
