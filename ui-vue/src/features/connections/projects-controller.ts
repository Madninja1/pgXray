import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";

/** Project is a local grouping; it does not grant access to PostgreSQL or data. */
export interface ConnectionProject { id: number; name: string; }
export interface ConnectionProjectsState {
  items: ConnectionProject[];
  loading: boolean;
  mutating: boolean;
  error: string | null;
}
export function createConnectionProjectsState(): ConnectionProjectsState {
  return { items: [], loading: false, mutating: false, error: null };
}
export function parseConnectionProjects(input: unknown): ConnectionProject[] {
  if (!Array.isArray(input)) throw new Error(t('tsFinish.projectInvalidList'));
  return input.map(item => {
    if (!item || typeof item !== "object") throw new Error(t('tsFinish.projectInvalid'));
    const row = item as Record<string, unknown>;
    if (!Number.isSafeInteger(row.id) || (row.id as number) <= 0 ||
      typeof row.name !== "string" || !row.name.trim() ||
      [...row.name].length > 80) throw new Error(t('tsFinish.projectInvalid'));
    return { id: row.id as number, name: row.name as string };
  });
}
export function validateProjectName(name: string): string | null {
  const trimmed = name.trim();
  return !trimmed || [...trimmed].length > 80 || /\p{Cc}/u.test(trimmed)
    ? t('tsFinish.projectName') : null;
}
export function createConnectionProjectsController(
  shell: ShellState,
  state: ConnectionProjectsState,
  invoke: PgXrayInvoke,
  onConnectionsChanged: () => Promise<boolean>,
) {
  let revision = 0;
  let request = 0;
  const fresh = (epoch: number, currentRevision: number) =>
    shell.screen === "workspace" && isCurrentAuthGeneration(shell, epoch) && currentRevision === revision;

  function reset(): void {
    revision += 1;
    Object.assign(state, createConnectionProjectsState());
  }
  async function refresh(): Promise<boolean> {
    if (shell.screen !== "workspace") return false;
    const epoch = shell.authGeneration, rev = revision, activeRequest = ++request;
    state.loading = true;
    try {
      const items = parseConnectionProjects(await invoke("list_connection_projects"));
      if (!fresh(epoch, rev) || request !== activeRequest) return false;
      state.items = items;
      state.error = null;
      return true;
    } catch (error) {
      if (fresh(epoch, rev) && request === activeRequest) state.error = String(error);
      return false;
    } finally {
      if (fresh(epoch, rev) && request === activeRequest) state.loading = false;
    }
  }
  async function mutate(command: string, args: Record<string, unknown>, refreshConnections: boolean): Promise<boolean> {
    if (shell.screen !== "workspace" || state.mutating) return false;
    const epoch = shell.authGeneration, rev = revision;
    state.mutating = true;
    state.error = null;
    try {
      await invoke(command, args);
      if (!fresh(epoch, rev)) return false;
      const projectsOk = await refresh();
      if (!fresh(epoch, rev)) return false;
      const connectionsOk = !refreshConnections || await onConnectionsChanged();
      if (!fresh(epoch, rev)) return false;
      if (!projectsOk || !connectionsOk) {
        state.error = t('tsFinish.projectRefresh');
      }
      return true;
    } catch (error) {
      if (fresh(epoch, rev)) state.error = String(error);
      return false;
    } finally {
      if (fresh(epoch, rev)) state.mutating = false;
    }
  }
  async function create(name: string): Promise<boolean> {
    const issue = validateProjectName(name);
    if (issue) { state.error = issue; return false; }
    return mutate("create_connection_project", { name: name.trim() }, false);
  }
  async function rename(id: number, name: string): Promise<boolean> {
    const issue = validateProjectName(name);
    if (issue) { state.error = issue; return false; }
    return mutate("rename_connection_project", { id, name: name.trim() }, false);
  }
  async function remove(id: number): Promise<boolean> {
    return mutate("delete_connection_project", { id }, true);
  }
  async function move(connectionId: number, projectId: number | null): Promise<boolean> {
    return mutate("assign_connection_project", { connectionId, projectId }, true);
  }
  return { reset, refresh, create, rename, remove, move };
}
