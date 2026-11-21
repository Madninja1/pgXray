import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { parseJournalPage, type JournalPage, type JournalQuery } from "./log-explorer-model.js";
import type { LogEntry } from "./extra-controller.js";
import { logClientRequestId, type LogDisplayRow } from "./log-view-model.js";

export interface JournalExplorerState {
  page: JournalPage | null;
  pageIndex: number;
  cursors: Array<string | null>;
  loading: boolean;
  error: string | null;
  selected: LogDisplayRow | null;
  requestEvents: LogEntry[];
  requestCursor: string | null;
  requestLoading: boolean;
  requestError: string | null;
  requestScanLimitReached: boolean;
}
export function createJournalExplorerState(): JournalExplorerState {
  return {
    page: null, pageIndex: 0, cursors: [null], loading: false, error: null,
    selected: null, requestEvents: [], requestCursor: null,
    requestLoading: false, requestError: null, requestScanLimitReached: false,
  };
}
export function createJournalExplorerController(
  shell: ShellState, state: JournalExplorerState, invoke: PgXrayInvoke,
) {
  let revision = 0;
  let requestRevision = 0;
  const fresh = (generation: number, seq: number) =>
    seq === revision && shell.screen === "workspace" && shell.currentUsername !== null && shell.authGeneration === generation;

  function reset(): void {
    revision++;
    requestRevision++;
    Object.assign(state, createJournalExplorerState());
  }

  async function load(query: JournalQuery, direction: "reset" | "next" | "previous" = "reset"): Promise<boolean> {
    if (shell.screen !== "workspace" || shell.currentUsername === null || state.loading) return false;
    const generation = shell.authGeneration, seq = ++revision;
    const targetPage = direction === "reset" ? 0 : direction === "next" ? state.pageIndex + 1 : Math.max(0, state.pageIndex - 1);
    const cursor = direction === "reset" ? null : direction === "next" ? state.page?.next_cursor ?? null : state.cursors[targetPage] ?? null;
    if (direction === "next" && !cursor) return false;
    state.loading = true;
    state.error = null;
    try {
      const page = parseJournalPage(await invoke("query_application_log", { query: { ...query, cursor } }));
      if (!fresh(generation, seq)) return false;
      state.page = page;
      state.pageIndex = targetPage;
      if (direction === "reset") state.cursors = [null];
      else if (direction === "next") state.cursors = [...state.cursors.slice(0, targetPage), cursor];
      return true;
    } catch (error) {
      if (fresh(generation, seq)) {
        const message = String(error);
        state.error = /command\s+query_application_log\s+not found/i.test(message)
          ? t('tsLogExplorer.missingBackend')
          : t('tsLogExplorer.searchFailed', { error: message });
      }
      return false;
    } finally {
      if (fresh(generation, seq)) state.loading = false;
    }
  }

  async function open(row: LogDisplayRow): Promise<void> {
    const generation = shell.authGeneration, seq = ++requestRevision;
    state.selected = row;
    state.requestEvents = [...row.entries];
    state.requestCursor = null;
    state.requestError = null;
    state.requestScanLimitReached = false;
    state.requestLoading = false;
    const operationId = row.entries.find(e => e.operation_id)?.operation_id ?? null;
    const clientId = row.entries.map(logClientRequestId).find(Boolean) ?? null;
    if ((!row.request_id && !operationId && !clientId) || shell.currentUsername === null) return;
    await fetchRequest(row.request_id, operationId, clientId, null, generation, seq, false);
  }

  async function fetchRequest(requestId: string | null, operationId: string | null, clientId: string | null, cursor: string | null, generation: number, seq: number, append: boolean): Promise<void> {
    state.requestLoading = true;
    try {
      const page = parseJournalPage(await invoke("query_application_log", { query: {
        limit: 100, cursor, from_ms: null, to_ms: null,
        level: null, component: null, section: null, operation: null, actor: null,
        request_id: clientId || operationId ? null : requestId,
        operation_id: clientId ? null : operationId,
        client_request_id: clientId,
        problems_only: false, min_duration_ms: null, search: null,
      } }));
      if (seq !== requestRevision || shell.authGeneration !== generation || shell.currentUsername === null) return;
      // Never mix phases belonging to another operation/actor: the request id
      // alone is a search key, not an independently authenticated operation ID.
      const selected = state.selected;
      const matches = page.entries.filter(e => selected && (clientId
        ? logClientRequestId(e) === clientId
        : operationId ? e.operation_id === operationId
        : e.request_id === requestId && e.operation === selected.operation &&
          e.component === selected.component && e.actor === selected.actor));
      const earlier = append ? state.requestEvents : [];
      const combined = [...earlier, ...matches];
      const distinct = new Set<string>();
      state.requestEvents = combined.filter(e => {
        const key = JSON.stringify(e);
        if (distinct.has(key)) return false;
        distinct.add(key);
        return true;
      }).sort((a, b) => a.timestamp_ms - b.timestamp_ms);
      state.requestCursor = page.next_cursor;
      state.requestScanLimitReached = page.scan_limit_reached;
      state.requestError = null;
    } catch (error) {
      if (seq === requestRevision && shell.authGeneration === generation)
        state.requestError = t('tsLogExplorer.timelineFailed', { error: String(error) });
    } finally {
      if (seq === requestRevision && shell.authGeneration === generation)
        state.requestLoading = false;
    }
  }

  async function continueRequest(): Promise<void> {
    const cursor = state.requestCursor, row = state.selected;
    const operationId = row?.entries.find(e => e.operation_id)?.operation_id ?? null;
    const clientId = row?.entries.map(logClientRequestId).find(Boolean) ?? null;
    if (!cursor || (!row?.request_id && !operationId && !clientId) || state.requestLoading || shell.currentUsername === null) return;
    await fetchRequest(row?.request_id ?? null, operationId, clientId, cursor, shell.authGeneration, requestRevision, true);
  }

  function close(): void {
    ++requestRevision;
    state.selected = null;
    state.requestEvents = [];
    state.requestCursor = null;
    state.requestLoading = false;
    state.requestError = null;
  }
  return { reset, load, open, continueRequest, close };
}
