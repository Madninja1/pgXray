import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { parseActivityConnections, type ActivityConnection } from "../activity/controller.js";

export const AUTO_INTERVAL_MS = 24 * 60 * 60 * 1000;
export const SCHEDULE_CHECK_MS = 60 * 60 * 1000;
export interface Setting { name: string; setting: string; unit: string | null; category: string; source: string; sourcefile: string | null; sourceline: number | null; context: string; pending_restart: boolean; reset_val: string | null; boot_val: string | null }
export interface Snapshot { measured_at: string; database: string; username: string; settings: Setting[] }
export interface SnapshotSummary { id: number; connection_id: number; host: string; port: number; database: string; username: string; sslmode: string; postmaster_start_ms: number; collection_completed_at_ms: number; setting_count: number; non_default_count: number; pending_restart_count: number; changed_from_previous: number }
export interface SettingChange { name: string; state: string; older_value: string | null; newer_value: string | null; value_changed: boolean; source_changed: boolean; pending_restart_changed: boolean }
export interface Comparison { changes: SettingChange[] }
export interface ChangeEvent { detected_at_ms: number; changed_after_ms: number; changed_before_ms: number; change: SettingChange }
export interface AutoEvent { connectionId: number; at: number; ok: boolean; detail: string }
export interface PgSettingsState { connections: ActivityConnection[]; connectionId: number | null; summaries: SnapshotSummary[]; snapshot: Snapshot | null; comparison: Comparison | null; changes: ChangeEvent[]; loading: boolean; collecting: boolean; error: string | null; notice: string | null; autoIds: number[]; autoLastError: Record<number, string>; autoLastSuccess: Record<number, number>; autoRunning: boolean; autoEvents: AutoEvent[] }
export function createPgSettingsState(): PgSettingsState { return { connections: [], connectionId: null, summaries: [], snapshot: null, comparison: null, changes: [], loading: false, collecting: false, error: null, notice: null, autoIds: [], autoLastError: {}, autoLastSuccess: {}, autoRunning: false, autoEvents: [] }; }
export function sameTarget(a: SnapshotSummary | undefined, b: SnapshotSummary | undefined): boolean { return !!a && !!b && a.connection_id === b.connection_id && a.host === b.host && a.port === b.port && a.database === b.database && a.username === b.username && a.sslmode === b.sslmode && a.postmaster_start_ms === b.postmaster_start_ms; }
export function isDue(latest: SnapshotSummary | undefined, connection: ActivityConnection, now: number): boolean { return !latest || latest.host !== connection.host || latest.port !== connection.port || latest.database !== connection.database || latest.username !== connection.username || now - latest.collection_completed_at_ms >= AUTO_INTERVAL_MS; }
export function createPgSettingsController(shell: ShellState, state: PgSettingsState, invoke: PgXrayInvoke, storage: Pick<Storage, "getItem" | "setItem"> | null = null, now = Date.now) {
  let revision = 0; let disposed = false; let checking = false;
  const busyIds = new Set<number>();
  const valid = (auth: number, rev: number) => !disposed && shell.screen === "workspace" && shell.authGeneration === auth && revision === rev;
  const storageKey = () => `pgxray.pg_settings.auto.v1.${encodeURIComponent(shell.currentUsername ?? "")}`;
  function loadPreferences(): void { try { const value = JSON.parse(storage?.getItem(storageKey()) ?? "[]") as unknown; state.autoIds = Array.isArray(value) ? value.filter((x): x is number => Number.isSafeInteger(x) && x > 0) : []; } catch { state.autoIds = []; } }
  function savePreferences(): void { try { storage?.setItem(storageKey(), JSON.stringify(state.autoIds)); } catch { state.error = t('diagTs.cfgSaveSettingsError'); } }
  function setAuto(id: number, enabled: boolean): void { if (!state.connections.some(c => c.id === id)) return; state.autoIds = enabled ? [...new Set([...state.autoIds, id])] : state.autoIds.filter(x => x !== id); savePreferences(); }
  function event(id: number, ok: boolean, detail: string): void { state.autoEvents.unshift({ connectionId: id, at: now(), ok, detail }); state.autoEvents.splice(20); }
  function reset(): void { revision++; Object.assign(state, createPgSettingsState()); busyIds.clear(); }
  function dispose(): void { reset(); disposed = true; }
  function selectConnection(id: number | null): void { revision++; state.connectionId = state.connections.some(c => c.id === id) ? id : null; state.summaries = []; state.snapshot = null; state.comparison = null; state.changes = []; state.error = null; state.notice = null; void refresh(); }
  async function refreshConnections(): Promise<void> { const auth = shell.authGeneration, rev = revision; try { const connections = parseActivityConnections(await invoke("list_connections")); if (!valid(auth, rev)) return; state.connections = connections; loadPreferences(); state.autoIds = state.autoIds.filter(id => connections.some(c => c.id === id)); if (!connections.some(c => c.id === state.connectionId)) state.connectionId = connections[0]?.id ?? null; await refresh(); } catch(e) { if (valid(auth, rev)) state.error = t('diagTs.connectionsError', { error: String(e) }); } }
  async function refresh(): Promise<void> { const id = state.connectionId; if (id === null || shell.screen !== "workspace") return; const auth = shell.authGeneration, rev = revision; state.loading = true; state.error = null; try {
    const summaries = await invoke<SnapshotSummary[]>("list_pg_settings_snapshots", { connectionId: id, limit: 100 });
    const snapshot = summaries[0] ? await invoke<Snapshot>("load_pg_settings_snapshot", { snapshotId: summaries[0].id }) : null;
    const comparison = sameTarget(summaries[1], summaries[0]) ? await invoke<Comparison>("compare_pg_settings_snapshots", { firstId: summaries[1].id, secondId: summaries[0].id }) : null;
    if (!valid(auth, rev) || state.connectionId !== id) return;
    state.summaries = summaries; state.changes = []; state.snapshot = snapshot; state.comparison = comparison;
  } catch (e) { if (valid(auth, rev) && state.connectionId === id) state.error = t('diagTs.cfgHistoryError', { error: String(e) }); } finally { if (valid(auth, rev) && state.connectionId === id) state.loading = false; } }
  async function collect(id: number, automatic = false): Promise<boolean> { if (busyIds.has(id) || shell.screen !== "workspace") return false; busyIds.add(id); const auth = shell.authGeneration, rev = revision; if (state.connectionId === id) state.collecting = true; try {
    await invoke("collect_pg_settings_snapshot", { connectionId: id });
    if (valid(auth, rev)) { state.autoLastSuccess[id] = now(); delete state.autoLastError[id]; if (automatic) event(id, true, t('diagTs.cfgSnapshotSaved')); if (state.connectionId === id) { state.notice = automatic ? t('diagTs.cfgAutomaticSaved') : t('diagTs.cfgSaved'); await refresh(); } }
    return true;
  } catch(e) { if (valid(auth, rev)) { state.autoLastError[id] = String(e); if (automatic) event(id, false, String(e)); if (state.connectionId === id) state.error = t('diagTs.cfgCollectError', { error: String(e) }); } return false; } finally { busyIds.delete(id); if (valid(auth, rev) && state.connectionId === id) state.collecting = false; } }
  async function checkSchedule(): Promise<void> { if (checking || shell.screen !== "workspace" || !state.autoIds.length) return; checking = true; state.autoRunning = true; const auth = shell.authGeneration, rev = revision;
    try { for (const id of [...state.autoIds]) { if (!valid(auth, rev) || !state.autoIds.includes(id)) break; const connection = state.connections.find(c => c.id === id); if (!connection || busyIds.has(id)) continue; try { const list = await invoke<SnapshotSummary[]>("list_pg_settings_snapshots", { connectionId: id, limit: 1 }); if (!valid(auth, rev) || !state.autoIds.includes(id)) break; if (!busyIds.has(id) && isDue(list[0], connection, now())) await collect(id, true); } catch (e) { if (valid(auth, rev)) { state.autoLastError[id] = String(e); event(id, false, String(e)); } } } } finally { checking = false; if (valid(auth, rev)) state.autoRunning = false; }
  }
  return { refreshConnections, refresh, selectConnection, collect, setAuto, checkSchedule, reset, dispose };
}
