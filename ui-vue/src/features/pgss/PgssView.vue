<script setup lang="ts">
import { locale, t as tr } from "../i18n/locale-service.js";
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import {
  createPgssController, createPgssState, pgssDocsUrl, pgssIdentity,
  pgssMatchesConnection, pgssPage, pgssFilterCalls, PGSS_RANKS,
  type PgssCallsFilter, type PgssExplorerFilter, type PgssRank, type PgssRow, type PgssSection,
} from "./controller.js";

const props = defineProps<{
  shell: ShellState; invoke: PgXrayInvoke; visible: boolean;
  navigationToken?: number; preferredConnectionId?: number | null; preferredQueryid?: string | null;
  quickSelection?: QuickConnectionSelection | null;
}>();
const emit = defineEmits<{ openHistory: [connectionId: number | null, queryid: string | null] }>();
const state = reactive(createPgssState());
const controller = createPgssController(props.shell, state, props.invoke);
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.collecting || state.loadingSnapshot || state.loadingHistory)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.collecting || state.loadingSnapshot || state.loadingHistory) return;
  lastShortcutSerial = request.serial;
  // Never retarget a running collector/monitor or an in-flight database operation.
  if (state.connectionId !== request.connectionId) controller.selectConnection(request.connectionId);
});

type Tab = "health" | "cardinality" | "performance" | "explorer" | "reference";
const tab = ref<Tab>("health");
const tabs = computed<{ id: Tab; label: string }[]>(() => [
  { id:"health", label:tr('pgssFinal.health') },
  { id:"cardinality", label:"Cardinality" },
  { id:"performance", label:"Performance TOP-50" },
  { id:"explorer", label:"PGSS Explorer (live)" },
  { id:"reference", label:tr('pgssFinal.reference') },
]);
const snapshot = computed(() => state.snapshot);
const capacity = computed(() => snapshot.value?.capacity.data ?? null);
const stats = computed(() => snapshot.value?.stats.data ?? null);
const perf = computed(() => snapshot.value?.performance.data ?? null);
const leadTotal = computed(() => perf.value?.total_exec?.[0] ?? null);
const leadMean = computed(() => perf.value?.mean_exec?.[0] ?? null);
const sampledCoverage = computed(() => {
  const size = perf.value?.history_sample.length ?? 0;
  const entries = capacity.value?.current_entries;
  return entries && entries > 0 ? 100 * size / entries : null;
});
const currentConnection = computed(() => state.connections.find(c => c.id === state.connectionId));
const profileMatches = computed(() => pgssMatchesConnection(state.snapshotSummary, currentConnection.value));
const canExplore = computed(() => !!currentConnection.value && (!state.snapshotSummary || profileMatches.value));
const isArchived = computed(() => state.snapshotSource === "archive");
const incomplete = computed(() => {
  if (!snapshot.value) return [];
  return [
    [tr('pgssFinal.statSection'), snapshot.value.stats],
    ["Capacity", snapshot.value.capacity],
    ["Role / database", snapshot.value.role_database],
    ["Commands", snapshot.value.commands],
    ["CALL", snapshot.value.call_targets],
    ["Performance", snapshot.value.performance],
  ].filter(([,value]) => (value as PgssSection<unknown>).data === null)
    .map(([name,value]) => `${name}: ${(value as PgssSection<unknown>).error ?? tr('pgssFinal.noData')}`);
});
const cardMode = ref<"owners" | "commands" | "calls">("owners");
const cardSearch = ref(""); const cardPage = ref(1); const cardSize = ref(25);
type CardSort = "label" | "role_name" | "scope" | "entries" | "entries_pct" | "one_call_entries" | "one_call_pct" | "total_calls";
const cardSort = ref<CardSort>("entries"); const cardDescending = ref(true);
interface CardRow { key: string; label: string; userid: number; dbid: number;
  command?: string; call_tag?: string; role_name: string | null; database: string | null;
  entries: number; entries_pct: number; one_call_entries: number; one_call_pct: number; total_calls: number; scope: string }
const cardinality = computed<CardRow[]>(() => {
  const s = snapshot.value;
  if (!s) return [];
  if (cardMode.value === "owners") return (s.role_database.data ?? []).map(r => ({
    key:`${r.userid}:${r.dbid}`, userid:r.userid, dbid:r.dbid, label:`${r.role_name ?? `OID ${r.userid}`} / ${r.database ?? `OID ${r.dbid}`}`,
    role_name:r.role_name, database:r.database, entries:r.entries, entries_pct:r.entries_pct, one_call_entries:r.one_call_entries,
    one_call_pct:r.one_call_pct, total_calls:r.total_calls, scope:`${r.userid} / ${r.dbid}`,
  }));
  if (cardMode.value === "commands") return (s.commands.data ?? []).map(r => ({
    key:`${r.userid}:${r.dbid}:${r.command}`, userid:r.userid, dbid:r.dbid, command:r.command, label:r.command, role_name:r.role_name, database:r.database,
    entries:r.entries, entries_pct:r.entries_pct, one_call_entries:r.one_call_entries, one_call_pct:r.one_call_pct,
    total_calls:r.total_calls, scope:`${r.userid} / ${r.dbid}`,
  }));
  return (s.call_targets.data ?? []).map(r => ({
    key:`${r.userid}:${r.dbid}:${r.call_tag}`, userid:r.userid, dbid:r.dbid, call_tag:r.call_tag,
    label:r.call_target_name ?? `CALL tag ${r.call_tag}`, role_name:r.role_name, database:r.database,
    entries:r.entries, entries_pct:r.entries_pct_of_max, one_call_entries:r.one_call_entries, one_call_pct:r.one_call_pct,
    total_calls:r.total_calls, scope:`${r.userid} / ${r.dbid}`,
  }));
});
const cardRows = computed(() => pgssPage(cardinality.value, cardSearch.value, cardSort.value, cardDescending.value, cardPage.value, cardSize.value));
const cardError = computed(() => cardMode.value === "owners" ? snapshot.value?.role_database.error :
  cardMode.value === "commands" ? snapshot.value?.commands.error : snapshot.value?.call_targets.error);
const explorerFilters = ref<PgssExplorerFilter>({ sort:"total_exec", direction:"desc", limit:25, offset:0 });
const explorerQueryid = ref("");
// Selectable labels come from the saved PGSS cardinality snapshot; identifiers
// remain the exact server-side OIDs. Live values may have changed since capture.
const roleOptions = computed(() => {
  const names = new Map<number, string>();
  for (const item of snapshot.value?.role_database.data ?? []) names.set(item.userid, item.role_name ?? `OID ${item.userid}`);
  for (const item of state.explorer?.rows ?? []) if (!names.has(item.userid)) names.set(item.userid, item.role_name ?? `OID ${item.userid}`);
  return [...names].map(([id, name]) => ({ id, name })).sort((a,b) => a.name.localeCompare(b.name));
});
const databaseOptions = computed(() => {
  const names = new Map<number, string>();
  for (const item of snapshot.value?.role_database.data ?? []) names.set(item.dbid, item.database ?? `OID ${item.dbid}`);
  for (const item of state.explorer?.rows ?? []) if (!names.has(item.dbid)) names.set(item.dbid, item.database ?? `OID ${item.dbid}`);
  return [...names].map(([id, name]) => ({ id, name })).sort((a,b) => a.name.localeCompare(b.name));
});
const callTargetOptions = computed(() => (snapshot.value?.call_targets.data ?? []).map(item => ({
  tag:item.call_tag, label:item.call_target_name ?? `CALL tag ${item.call_tag}`,
})).sort((a,b) => a.label.localeCompare(b.label)));
const commandOptions = computed(() => [...new Set([
  "SELECT", "UPDATE", "INSERT", "DELETE", "CALL", "COPY", "SET", "BEGIN", "COMMIT",
  ...(snapshot.value?.commands.data ?? []).map(item => item.command),
])].sort());
function roleName(id: number): string { return roleOptions.value.find(item => item.id === id)?.name ?? `OID ${id}`; }
function databaseName(id: number): string { return databaseOptions.value.find(item => item.id === id)?.name ?? `OID ${id}`; }
const statementDialog = ref<HTMLDialogElement | null>(null);
const statementOpen = ref(false);
async function showStatementDialog(): Promise<void> {
  statementOpen.value = true;
  await nextTick();
  if (statementOpen.value && statementDialog.value && !statementDialog.value.open) statementDialog.value.showModal();
}
function onStatementDialogClosed(): void {
  statementOpen.value = false;
  controller.clearStatement();
}
function closeStatementDialog(): void {
  statementDialog.value?.close();
  onStatementDialogClosed();
}
const explorerUserId = ref("");
const explorerDbId = ref("");
const explorerCommand = ref("");
const explorerCallTag = ref("");
const explorerOneCall = ref(false);
const explorerMinCalls = ref("");
const explorerMaxCalls = ref("");
const explorerMinExecMs = ref("");
const explorerScope = ref(tr('pgssFinal.scopeAll'));
const explorerSort = ref<NonNullable<PgssExplorerFilter["sort"]>>("total_exec");
const explorerDirection = ref<"asc" | "desc">("desc");
const explorerLimit = ref(25);
function openExplorer(filter: PgssExplorerFilter = {}, label = tr('pgssFinal.scopeAll')): void {
  explorerFilters.value = { ...filter, sort:explorerSort.value, direction:explorerDirection.value, limit:explorerLimit.value, offset:0 };
  explorerQueryid.value = filter.queryid ?? "";
  explorerUserId.value = filter.userid == null ? "" : String(filter.userid);
  explorerDbId.value = filter.dbid == null ? "" : String(filter.dbid);
  explorerCommand.value = filter.command ?? "";
  explorerCallTag.value = filter.call_tag ?? "";
  explorerOneCall.value = filter.one_call === true;
  explorerMinCalls.value = filter.min_calls == null ? "" : String(filter.min_calls);
  explorerMaxCalls.value = filter.max_calls == null ? "" : String(filter.max_calls);
  explorerMinExecMs.value = filter.min_total_exec_ms == null ? "" : String(filter.min_total_exec_ms);
  explorerScope.value = label;
  tab.value = "explorer";
  void loadExplorer(0);
}
function optionalPgssNumber(value: string, label: string, max: number, fractional = false): number | undefined {
  const text = value.trim();
  if (!text) return undefined;
  if (!(fractional ? /^(?:\d+)(?:\.\d+)?$/.test(text) : /^\d+$/.test(text))) {
    throw Error(tr('pgssFinal.numberNonnegative', { label }));
  }
  const parsed = Number(text);
  if (!Number.isFinite(parsed) || parsed > max || (!fractional && !Number.isSafeInteger(parsed))) {
    throw Error(tr('pgssFinal.numberRange', { label }));
  }
  return parsed;
}
async function loadExplorer(offset = 0, useAppliedFilters = false): Promise<void> {
  if (useAppliedFilters) {
    const filter = { ...explorerFilters.value, offset };
    await controller.refreshExplorer(filter);
    return;
  }
  const queryid = explorerQueryid.value.trim();
  if (queryid && !/^-?\d+$/.test(queryid)) {
    state.explorerError = tr('pgssFinal.badQueryid');
    return;
  }
  let filter: PgssExplorerFilter;
  try {
    const command = explorerCommand.value.trim().toUpperCase();
    if (command && !(/^[A-Z]{1,24}$/.test(command) || ["<HIDDEN>", "<WITH/CTE>", "<OTHER>"].includes(command))) {
      throw Error(tr('pgssFinal.invalidSqlCommand'));
    }
    const min_calls = optionalPgssNumber(explorerMinCalls.value, tr('pgssFinal.minCalls'), Number.MAX_SAFE_INTEGER);
    const max_calls = optionalPgssNumber(explorerMaxCalls.value, tr('pgssFinal.maxCalls'), Number.MAX_SAFE_INTEGER);
    if (min_calls != null && max_calls != null && min_calls > max_calls) {
      throw Error(tr('pgssFinal.minCallsExceeds'));
    }
    filter = {
      ...explorerFilters.value, offset, sort:explorerSort.value, direction:explorerDirection.value, limit:explorerLimit.value,
      userid:optionalPgssNumber(explorerUserId.value, tr('pgssFinal.roleOid'), 4294967295),
      dbid:optionalPgssNumber(explorerDbId.value, tr('pgssFinal.dbOid'), 4294967295),
      command:command || undefined,
      call_tag:explorerCallTag.value || undefined,
      queryid:queryid || undefined,
      one_call:explorerOneCall.value,
      min_calls, max_calls,
      min_total_exec_ms:optionalPgssNumber(explorerMinExecMs.value, tr('pgssFinal.minExec'), 1e12, true),
    };
  } catch (error) {
    state.explorerError = String(error);
    return;
  }
  explorerFilters.value = filter;
  await controller.refreshExplorer(filter);
}
/** Header clicks sort the entire live selection on PostgreSQL, not the visible page.
 * Keep the last applied predicates even if the form contains unsubmitted edits. */
function changeExplorerSort(key: NonNullable<PgssExplorerFilter["sort"]>): void {
  if (state.explorerLoading || !canExplore.value) return;
  const appliedSort = explorerFilters.value.sort ?? "total_exec";
  const appliedDirection = explorerFilters.value.direction ?? "desc";
  const direction = appliedSort === key
    ? (appliedDirection === "desc" ? "asc" : "desc")
    : (key === "queryid" || key === "role" ? "asc" : "desc");
  explorerSort.value = key;
  explorerDirection.value = direction;
  const filter: PgssExplorerFilter = { ...explorerFilters.value, sort:key, direction, offset:0 };
  explorerFilters.value = filter;
  void controller.refreshExplorer(filter);
}
function exploreCard(row: CardRow, onlyOneCall = false): void {
  const filter: PgssExplorerFilter = { userid: row.userid, dbid: row.dbid, one_call: onlyOneCall };
  if (row.command) filter.command = row.command;
  if (row.call_tag) filter.call_tag = row.call_tag;
  openExplorer(filter, `${row.label} · ${onlyOneCall ? "one-call" : tr('pgssFinish.scopeEntries')}`);
}
async function exploreLiveStatement(row: PgssRow): Promise<void> {
  if (!row.queryid) return;
  controller.selectLiveStatement(row);
  await showStatementDialog();
  if (statementOpen.value) await controller.refreshDetail();
}
const rank = ref<PgssRank>("total_exec");
const perfSearch = ref(""); const perfPage = ref(1); const perfSize = ref(25);
const perfCalls = ref<PgssCallsFilter>("all");
type PerfSort = "queryid" | "role_name" | "metric" | "calls" | "total_exec_time_ms" | "mean_exec_time_ms" | "temp_blocks" | "wal_bytes";
const perfSort = ref<PerfSort>("metric");
const perfDescending = ref(true);
function metric(row: PgssRow, mode: PgssRank): number {
  switch(mode) {
    case "total_exec": return row.total_exec_time_ms;
    case "mean_exec": return row.mean_exec_time_ms;
    case "temp_io": return row.temp_blks_read + row.temp_blks_written;
    case "shared_reads": return row.shared_blks_read;
    case "wal": return row.wal_bytes;
    case "variance": return row.stddev_exec_time_ms;
    case "planning": return row.total_plan_time_ms;
  }
}
const perfRows = computed(() => {
  const planningUnavailable = rank.value === "planning" && snapshot.value?.overview.track_planning !== true;
  const rows: PgssRow[] = planningUnavailable ? [] : (perf.value?.[rank.value] ?? []);
  const selected = pgssFilterCalls(rows, perfCalls.value);
  return pgssPage(selected.map(r => ({ ...r, metric: metric(r, rank.value), temp_blocks: r.temp_blks_read + r.temp_blks_written })), perfSearch.value,
    perfSort.value, perfDescending.value, perfPage.value, perfSize.value);
});
const localizedRanks = computed(() => PGSS_RANKS.map(item => ({
  ...item,
  label: ({
    total_exec: tr('frontendPolish.rankTotal'),
    mean_exec: tr('frontendPolish.rankMean'),
    temp_io: tr('frontendPolish.rankTemp'),
    shared_reads: tr('frontendPolish.rankShared'),
    wal: tr('frontendPolish.rankWal'),
    variance: tr('frontendPolish.rankVariance'),
    planning: tr('frontendPolish.rankPlanning'),
  })[item.id],
  explanation: ({
    total_exec: tr('frontendPolish.explainTotal'),
    mean_exec: tr('frontendPolish.explainMean'),
    temp_io: tr('frontendPolish.explainTemp'),
    shared_reads: tr('frontendPolish.explainShared'),
    wal: tr('frontendPolish.explainWal'),
    variance: tr('frontendPolish.explainVariance'),
    planning: tr('frontendPolish.explainPlanning'),
  })[item.id],
})));
const selectedRank = computed(() => localizedRanks.value.find(r => r.id === rank.value)!);
const referenceQuery = ref<string | null>(null);
function sortMark(key: string, current: string, desc: boolean): string { return key === current ? (desc ? "↓" : "↑") : "↕"; }
function nextSort<T extends string>(key: T, current: { value: T }, direction: { value: boolean }): void {
  if (current.value === key) direction.value = !direction.value;
  else { current.value = key; direction.value = !["label", "role_name", "scope", "queryid", "database", "sample_state", "delta_state", "stats_reset", "evidence_level"].includes(key); }
}
function changeCardSort(key: CardSort): void { nextSort(key, cardSort, cardDescending); }
function changePerfSort(key: PerfSort): void { nextSort(key, perfSort, perfDescending); }
const sourceVersion = computed(() => snapshot.value?.overview.server_version_num ?? state.snapshotSummary?.server_version_num ?? null);
const profileWarning = computed(() => state.snapshotSummary && !profileMatches.value);
function n(value: number | null | undefined, digits=0): string {
  return value == null || !Number.isFinite(value) ? "—" :
    new Intl.NumberFormat(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits:digits }).format(value);
}
function pct(value: number | null | undefined): string { return value == null ? "—" : `${n(value, 1)}%`; }
function ms(value: number | null | undefined): string { return value == null ? "—" : tr('pgssFinish.msUnit', { value: n(value, value >= 10 ? 1 : 3) }); }
/** Readable units for accumulated execution time; exact milliseconds remain in tooltips. */
function duration(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return "—";
  if (Math.abs(value) >= 3_600_000) return tr('pgssFinish.hoursUnit', { value: n(value / 3_600_000, 1) });
  if (Math.abs(value) >= 60_000) return tr('pgssFinish.minutesUnit', { value: n(value / 60_000, 1) });
  if (Math.abs(value) >= 10_000) return tr('pgssFinish.secondsUnit', { value: n(value / 1_000, 1) });
  return ms(value);
}
function bytes(value: number | null | undefined): string {
  if (value == null) return "—";
  const abs = Math.abs(value), unit = abs >= 1024**3 ? 1024**3 : abs >= 1024**2 ? 1024**2 : abs >= 1024 ? 1024 : 1;
  return `${n(value / unit, 1)} ${unit === 1 ? tr('pgssFinal.unitB') : unit === 1024 ? tr('pgssFinal.unitKiB') : unit === 1024**2 ? tr('pgssFinal.unitMiB') : tr('pgssFinal.unitGiB')}`;
}
function date(value: string | number | null | undefined): string {
  if (value == null) return "—";
  const v = new Date(value); return Number.isNaN(v.valueOf()) ? "—" : v.toLocaleString("ru-RU");
}
function rankMetric(row: PgssRow): string {
  const value = metric(row, rank.value);
  return ["total_exec", "mean_exec", "variance", "planning"].includes(rank.value) ? duration(value) :
    rank.value === "wal" ? bytes(value) : n(value);
}
function openTab(value: Tab): void {
  tab.value = value;
  if (value === "reference") void controller.loadReference();
  if (value === "explorer" && !state.explorer && !state.explorerLoading && canExplore.value)
    void loadExplorer(0);
}
function onConnection(event: Event): void {
  const id = Number((event.target as HTMLSelectElement).value);
  closeStatementDialog();
  controller.selectConnection(Number.isSafeInteger(id) && id > 0 ? id : null);
  tab.value = "health"; cardPage.value = 1; perfPage.value = 1; explorerScope.value = tr('pgssFinal.scopeAll'); explorerQueryid.value = "";
}
async function onCollect(): Promise<void> { closeStatementDialog(); if (await controller.collect()) tab.value = "health"; }
async function investigate(row: PgssRow): Promise<void> {
  if (!row.queryid) return;
  controller.selectLiveStatement(row);
  await showStatementDialog();
}
async function copySql(value: string): Promise<void> {
  try {
    if (!navigator.clipboard?.writeText) throw Error(tr('pgssFinal.clipboardMissing'));
    await navigator.clipboard.writeText(value);
    state.notice = tr('pgssFinal.sqlCopied');
  } catch (error) { state.error = tr('pgssFinal.copyFailed', { error: String(error) }); }
}
function docs(path: string): string | null { return pgssDocsUrl(sourceVersion.value, path); }
function bootstrap(): void {
  if (props.visible && props.shell.screen === "workspace" && !props.shell.busy &&
      props.shell.currentUsername && !state.connections.length && !state.loadingConnections)
    void controller.refreshConnections();
}
watch([cardMode, cardSearch, cardSize, cardSort, cardDescending], () => { cardPage.value = 1; });
watch([rank, perfSearch, perfCalls, perfSize, perfSort, perfDescending], () => { perfPage.value = 1; });
watch(() => props.shell.authGeneration, () => { closeStatementDialog(); controller.reset(); bootstrap(); });
watch(() => props.shell.screen, screen => { if (screen !== "workspace") { closeStatementDialog(); controller.reset(); } else bootstrap(); });
watch(() => props.shell.busy, bootstrap);
watch(() => props.visible, visible => {
  if (visible && props.shell.screen === "workspace" && !props.shell.busy && props.shell.currentUsername)
    void controller.refreshConnections();
  else if (!visible) closeStatementDialog();
});
let handledNavigationToken = props.navigationToken ?? 0;
watch([() => props.navigationToken, () => props.visible, () => state.connections.length], () => {
  const requested = props.navigationToken ?? 0;
  if (!props.visible || !state.connections.length || !requested || requested === handledNavigationToken) return;
  handledNavigationToken = requested;
  const id = props.preferredConnectionId;
  // An archived connection must never become a live PGSS query on a different server.
  if (id != null && !state.connections.some(c => c.id === id)) {
    state.error = tr('pgssFinal.missingConnection');
    return;
  }
  if (id != null && state.connectionId !== id) controller.selectConnection(id);
  const queryid = props.preferredQueryid;
  if (queryid && /^-?\d+$/.test(queryid)) openExplorer({queryid}, tr('pgssFinal.historyQuery', { queryid }));
  else tab.value = "health";
});
onMounted(bootstrap);
onUnmounted(() => { closeStatementDialog(); controller.dispose(); });
</script>

<template>
<section class="pgss" aria-label="pg_stat_statements">
  <header class="heading">
    <div>
      <h1>SQL workload <small>· pg_stat_statements</small></h1>
      <p class="muted">{{ tr('pgssUi.text000') }}</p>
    </div>
    <div class="toolbar compact"><span class="pill">{{ tr('pgssUi.text001') }}</span><button type="button" class="secondary" @click="emit('openHistory', state.connectionId, null)">{{ tr('pgssUi.text002') }}</button></div>
  </header>
  <div class="toolbar">
    <label>{{ tr('pgssUi.text003') }}
      <select :value="state.connectionId ?? ''" :disabled="state.collecting || state.loadingSnapshot" @change="onConnection">
        <option value="">{{ tr('pgssUi.text004') }}</option>
        <option v-for="c in state.connections" :key="c.id" :value="c.id">{{ c.name }} · {{ c.username }}@{{ c.host }}:{{ c.port }}/{{ c.database }}</option>
      </select>
    </label>
    <button :disabled="state.connectionId === null || state.collecting || state.loadingSnapshot || state.loadingHistory" @click="onCollect">
      <span v-if="state.collecting" class="button-spinner" aria-hidden="true"></span>{{ state.collecting ? tr('pgssFinal.collecting') : tr('pgssFinal.collect') }}
    </button>
  </div>
  <p class="muted note">{{ tr('pgssUi.text005') }}</p>
  <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
  <p v-if="state.notice" class="notice" role="status">{{ state.notice }}</p>
  <p v-if="state.loadingSnapshot" class="muted" role="status">{{ tr('pgssUi.text006') }}</p>
  <p v-if="!snapshot && !state.loadingSnapshot" class="empty">
    {{ state.connectionId === null ? tr('pgssFinal.selectConnection') : tr('pgssFinal.noSnapshot') }}
  </p>

  <template v-if="snapshot && state.snapshotSummary">
    <div class="snapshot-strip">
      <div><strong>Snapshot #{{ state.snapshotSummary.id }}</strong> · {{ date(state.snapshotSummary.collection_completed_at_ms) }}
        <span class="muted"> · {{ snapshot.overview.database }} (OID {{ snapshot.overview.database_oid }}) · PG {{ snapshot.overview.server_version }}</span>
      </div>
      <span class="pill" :class="{ warn: isArchived }">{{ isArchived ? tr('pgssFinal.saved') : tr('pgssFinal.new') }}</span>
    </div>
    <p v-if="isArchived" class="notice">{{ tr('pgssUi.text007') }}</p>
    <p v-if="profileWarning" class="warning">{{ tr('pgssUi.text008') }}</p>
    <div class="kpis">
      <article><span>Entries PGSS</span><strong>{{ n(capacity?.current_entries) }} / {{ n(capacity?.max_entries) }}</strong><small>{{ pct(capacity?.fill_pct) }}{{ tr('pgssFinal.capacityFill') }}</small></article>
      <article><span>One-call entries</span><strong>{{ n(capacity?.one_call_entries) }}</strong><small>{{ pct(capacity?.one_call_pct) }}{{ tr('pgssFinal.totalEntries') }}</small><button type="button" class="secondary small" :disabled="!canExplore" @click="openExplorer({one_call:true}, 'One-call entries')">{{ tr('pgssUi.text009') }}</button></article>
      <article><span>{{ tr('pgssUi.text010') }}</span><strong>{{ n(stats?.dealloc) }}</strong><small>{{ tr('pgssUi.text011') }}</small></article>
      <article><span>{{ tr('pgssUi.text012') }}</span><strong>{{ 6 - incomplete.length }} / 6</strong><small>{{ incomplete.length ? tr('pgssFinal.someUnavailable') : tr('pgssFinal.allSections') }}</small></article>
    </div>
  </template>

  <nav class="tabs" :aria-label="tr('pgssFinal.sectionsAria')">
    <button v-for="t in tabs" :key="t.id" type="button" class="tab" :class="{ selected:tab === t.id }"
      :aria-current="tab === t.id ? 'page' : undefined" @click="openTab(t.id)">{{ t.label }}</button>
  </nav>

  <template v-if="tab === 'health' && snapshot">
    <section class="panel">
      <h2>{{ tr('pgssUi.text013') }}</h2>
      <p class="muted">{{ tr('pgssUi.text014') }}</p>
      <div class="triage-grid">
        <article v-if="leadTotal" class="triage-item">
          <span class="muted">{{ tr('pgssUi.text015') }}</span>
          <strong :title="ms(leadTotal.total_exec_time_ms)">{{ duration(leadTotal.total_exec_time_ms) }}</strong>
          <span class="mono id-text">queryid {{ leadTotal.queryid ?? tr('pgssFinal.noQueryid') }}</span>
          <span>{{ n(leadTotal.calls) }}{{ tr('pgssFinal.calls') }}{{ duration(leadTotal.mean_exec_time_ms) }}</span>
          <button class="secondary small" :disabled="!leadTotal.queryid || state.loadingStatement" @click="investigate(leadTotal)">{{ tr('pgssUi.text016') }}</button>
        </article>
        <article v-if="leadMean && pgssIdentity(leadMean) !== (leadTotal ? pgssIdentity(leadTotal) : '')" class="triage-item">
          <span class="muted">{{ tr('pgssUi.text017') }}</span>
          <strong :title="ms(leadMean.mean_exec_time_ms)">{{ duration(leadMean.mean_exec_time_ms) }}{{ tr('pgssFinal.perCall') }}</strong>
          <span class="mono id-text">queryid {{ leadMean.queryid ?? tr('pgssFinal.noQueryid') }}</span>
          <span>{{ n(leadMean.calls) }}{{ tr('pgssFinal.callsRelevance') }}</span>
          <button class="secondary small" :disabled="!leadMean.queryid || state.loadingStatement" @click="investigate(leadMean)">{{ tr('pgssUi.text016') }}</button>
        </article>
        <article class="triage-item">
          <span class="muted">{{ tr('pgssUi.text018') }}</span>
          <strong>{{ n(state.summaries.length) }}{{ tr('pgssFinal.snapshotCount') }}</strong>
          <span>{{ state.summaries.length < 2 ? tr('pgssFinal.needTwoSnapshots') : tr('pgssFinal.compareEpochs') }}</span>
          <span>History sample: {{ n(perf?.history_sample.length) }} entries · {{ pct(sampledCoverage) }}{{ tr('pgssFinal.historySample') }}</span>
          <button class="secondary small" @click="emit('openHistory', state.connectionId, null)">{{ tr('pgssUi.text019') }}</button>
        </article>
      </div>
    </section>
    <section class="panel">
      <h2>{{ tr('pgssUi.text020') }}</h2>
      <div class="properties">
        <div><span>{{ tr('pgssUi.text021') }}</span><strong>{{ snapshot.overview.extension_installed ? tr('pgssFinal.installed', { version: snapshot.overview.extension_version ?? tr('pgssFinal.versionUnknown') }) : tr('pgssFinal.notInstalled') }}</strong></div>
        <div><span>Preload</span><strong>{{ snapshot.overview.preloaded ? tr('pgssFinal.detected') : tr('pgssFinal.notDetected') }}</strong></div>
        <div><span>compute_query_id</span><strong>{{ snapshot.overview.compute_query_id ?? '—' }}</strong></div>
        <div><span>track / utility</span><strong>{{ snapshot.overview.track ?? '—' }} / {{ snapshot.overview.track_utility == null ? '—' : snapshot.overview.track_utility ? 'on' : 'off' }}</strong></div>
        <div><span>track_planning</span><strong>{{ snapshot.overview.track_planning == null ? tr('pgssFinal.unknown') : snapshot.overview.track_planning ? 'on' : 'off' }}</strong></div>
        <div><span>track_io_timing</span><strong>{{ snapshot.overview.track_io_timing ? 'on' : 'off' }}</strong></div>
        <div><span>stats_reset</span><strong>{{ date(stats?.stats_reset) }}</strong></div>
        <div><span>{{ tr('pgssUi.text022') }}</span><strong>{{ stats?.stats_age_seconds == null ? tr('pgssFinal.unknownMasculine') : tr('pgssFinish.ageHours', { value: n(stats.stats_age_seconds / 3600, 1) }) }}</strong></div>
        <div><span>{{ tr('pgssUi.text023') }}</span><strong>{{ n(capacity?.total_calls) }}</strong></div>
        <div><span>{{ tr('pgssUi.text024') }}</span><strong>{{ n(capacity?.up_to_5_calls_entries) }} ({{ pct(capacity?.up_to_5_calls_pct) }})</strong></div>
      </div>
      <p v-if="stats && !stats.stats_reset" class="warning" role="status">
        <strong>{{ tr('pgssFinal.statsUnknown') }}</strong> {{ tr('pgssFinal.nullExplanation') }}
        {{ tr('pgssFinal.continuityExplanation') }}
        {{ tr('pgssFinal.deltaExplanation') }}
      </p>
      <p v-if="!snapshot.overview.track_io_timing || snapshot.overview.track_planning !== true" class="muted">{{ tr('pgssUi.text025') }}</p>
    </section>
    <section v-if="incomplete.length" class="panel"><h2>{{ tr('pgssUi.text026') }}</h2><ul><li v-for="(message,i) in incomplete" :key="i">{{ message }}</li></ul></section>
    <section class="panel">
      <h2>{{ tr('pgssUi.text027') }}</h2>
      <p class="muted">{{ tr('pgssUi.text028') }}</p>
      <div v-if="!snapshot.interpretation.items.length" class="empty">{{ tr('pgssUi.text029') }}</div>
      <div class="findings compact-findings">
        <details v-for="(item,i) in snapshot.interpretation.items" :key="item.id" class="finding" :open="i === 0">
          <summary>{{ item.title }}</summary>
          <div v-if="item.observed.length"><strong>{{ tr('pgssUi.text030') }}</strong><ul><li v-for="(line,j) in item.observed" :key="j">{{ line }}</li></ul></div>
          <div v-if="item.meaning.length"><strong>{{ tr('pgssUi.text031') }}</strong><ul><li v-for="(line,j) in item.meaning" :key="j">{{ line }}</li></ul></div>
          <div v-if="item.next_steps.length"><strong>{{ tr('pgssUi.text032') }}</strong><ul><li v-for="(line,j) in item.next_steps" :key="j">{{ line }}</li></ul></div>
        </details>
      </div>
      <details v-if="snapshot.interpretation.limitations.length" class="disclosure">
        <summary>{{ tr('pgssFinal.dataLimitations') }}{{ snapshot.interpretation.limitations.length }})</summary>
        <ul><li v-for="(line,i) in snapshot.interpretation.limitations" :key="i">{{ line }}</li></ul>
      </details>
    </section>
  </template>

  <template v-if="tab === 'cardinality' && snapshot">
    <section class="panel">
      <h2>{{ tr('pgssUi.text033') }}</h2>
      <p class="muted">{{ tr('pgssUi.text034') }}</p>
      <div class="toolbar compact">
        <label>{{ tr('pgssUi.text035') }} <select v-model="cardMode"><option value="owners">{{ tr('pgssUi.text036') }}</option><option value="commands">{{ tr('pgssUi.text037') }}</option><option value="calls">CALL target</option></select></label>
        <label>{{ tr('pgssUi.text038') }} <input v-model="cardSearch" :placeholder="tr('pgssFinal.cardSearchPlaceholder')" /></label>
        <label>{{ tr('pgssUi.text039') }} <select v-model="cardSort"><option value="entries">Entries</option><option value="one_call_pct">One-call %</option><option value="total_calls">Calls</option><option value="label">{{ tr('pgssUi.text040') }}</option></select></label>
        <label>{{ tr('pgssUi.text041') }} <select v-model.number="cardSize"><option :value="10">10</option><option :value="25">25</option><option :value="50">50</option><option :value="100">100</option></select></label>
      </div>
      <p class="muted note">{{ tr('pgssUi.text042') }}</p>
      <p v-if="cardMode === 'commands'" class="muted">{{ tr('pgssFinal.hiddenSql') }}</p>
      <p v-if="cardMode === 'calls'" class="muted">{{ tr('pgssFinal.callsExplanation') }}</p>
      <p v-if="cardError" class="warning">{{ tr('pgssFinal.sectionUnavailable') }}{{ cardError }}</p>
      <div class="table-scroll"><table><thead><tr><th :aria-sort="cardSort === 'label' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('label')">{{ cardMode === 'owners' ? tr('pgssFinal.roleDatabase') : cardMode === 'commands' ? tr('pgssFinal.command') : 'CALL target' }}<span aria-hidden="true">{{ sortMark('label', cardSort, cardDescending) }}</span></button></th><th v-if="cardMode !== 'owners'" :aria-sort="cardSort === 'role_name' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('role_name')">{{ tr('pgssUi.text043') }}<span aria-hidden="true">{{ sortMark('role_name', cardSort, cardDescending) }}</span></button></th><th :aria-sort="cardSort === 'scope' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('scope')">OID role/db<span aria-hidden="true">{{ sortMark('scope', cardSort, cardDescending) }}</span></button></th><th class="num" :aria-sort="cardSort === 'entries' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('entries')">Entries<span aria-hidden="true">{{ sortMark('entries', cardSort, cardDescending) }}</span></button></th><th class="num" :aria-sort="cardSort === 'entries_pct' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('entries_pct')">{{ cardMode === 'calls' ? tr('vueRemaining.percentOfMax') : '% entries' }}<span aria-hidden="true">{{ sortMark('entries_pct', cardSort, cardDescending) }}</span></button></th><th class="num" :aria-sort="cardSort === 'one_call_entries' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('one_call_entries')">One-call<span aria-hidden="true">{{ sortMark('one_call_entries', cardSort, cardDescending) }}</span></button></th><th class="num" :aria-sort="cardSort === 'one_call_pct' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('one_call_pct')">One-call %<span aria-hidden="true">{{ sortMark('one_call_pct', cardSort, cardDescending) }}</span></button></th><th class="num" :aria-sort="cardSort === 'total_calls' ? (cardDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changeCardSort('total_calls')">Calls<span aria-hidden="true">{{ sortMark('total_calls', cardSort, cardDescending) }}</span></button></th><th>{{ tr('pgssUi.text044') }}</th></tr></thead>
        <tbody><tr v-for="row in cardRows.rows" :key="row.key"><td><strong>{{ row.label }}</strong><small v-if="row.call_tag" class="mono">{{ row.call_tag }}</small></td><td v-if="cardMode !== 'owners'">{{ row.role_name ?? '—' }} / {{ row.database ?? '—' }}</td><td class="mono">{{ row.scope }}</td><td class="num">{{ n(row.entries) }}</td><td class="num">{{ pct(row.entries_pct) }}</td><td class="num">{{ n(row.one_call_entries) }}</td><td class="num">{{ pct(row.one_call_pct) }}</td><td class="num">{{ n(row.total_calls) }}</td><td><button class="secondary small" :disabled="!canExplore" @click="exploreCard(row)">{{ tr('pgssUi.text045') }}</button> <button class="secondary small" :disabled="!canExplore || row.one_call_entries === 0" @click="exploreCard(row, true)">One-call</button></td></tr>
        <tr v-if="!cardRows.rows.length"><td :colspan="cardMode === 'owners' ? 8 : 9">{{ cardError ? tr('pgssFinal.collectUnavailable') : tr('pgssFinal.noSearchRecords') }}</td></tr></tbody></table></div>
      <div class="pager"><span>{{ cardRows.total }}{{ tr('pgssFinal.groupsPage') }}{{ cardRows.page }} / {{ cardRows.pages }}</span><button class="secondary" :disabled="cardRows.page <= 1" @click="cardPage--">{{ tr('pgssUi.text046') }}</button><button class="secondary" :disabled="cardRows.page >= cardRows.pages" @click="cardPage++">{{ tr('pgssUi.text047') }}</button></div>
    </section>
  </template>

  <template v-if="tab === 'performance' && snapshot">
    <section class="panel">
      <h2>Performance hotspots</h2>
      <p class="muted">{{ tr('pgssUi.text048') }}</p>
      <button type="button" class="secondary small" :disabled="!canExplore" @click="openExplorer()">{{ tr('pgssUi.text049') }}</button>
      <div class="rank-tabs"><button v-for="r in localizedRanks" :key="r.id" type="button" class="secondary" :class="{ active:rank===r.id }" @click="rank=r.id">{{ r.label }}</button></div>
      <p class="muted">{{ selectedRank.explanation }}</p>
      <p v-if="rank === 'planning' && snapshot.overview.track_planning !== true" class="warning">{{ tr('pgssUi.text050') }}</p>
      <p v-if="snapshot.performance.error" class="warning">{{ tr('pgssFinal.performanceUnavailable') }}{{ snapshot.performance.error }}</p>
      <div class="toolbar compact">
        <label>{{ tr('pgssUi.text038') }} <input v-model="perfSearch" :placeholder="tr('pgssFinal.perfSearchPlaceholder')" /></label>
        <label>{{ tr('pgssUi.text051') }} <select v-model="perfCalls"><option value="all">{{ tr('pgssUi.text052') }}</option><option value="one">{{ tr('pgssUi.text053') }}</option><option value="few">{{ tr('pgssUi.text054') }}</option><option value="many">{{ tr('pgssUi.text055') }}</option></select></label>
        <label>{{ tr('pgssUi.text039') }} <select v-model="perfSort"><option value="metric">{{ tr('pgssUi.text056') }}</option><option value="total_exec_time_ms">Total exec</option><option value="mean_exec_time_ms">Mean exec</option><option value="calls">Calls</option></select></label>
        <button class="secondary" :aria-label="perfDescending ? tr('pgssFinal.sortAscending') : tr('pgssFinal.sortDescending')" @click="perfDescending=!perfDescending">{{ perfDescending ? tr('pgssFinal.sortDown') : tr('pgssFinal.sortUp') }}</button>
        <label>{{ tr('pgssUi.text041') }} <select v-model.number="perfSize"><option :value="10">10</option><option :value="25">25</option><option :value="50">50</option></select></label>
      </div>
      <div class="table-scroll"><table><thead><tr><th :aria-sort="perfSort === 'queryid' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('queryid')">Statement<span aria-hidden="true">{{ sortMark('queryid', perfSort, perfDescending) }}</span></button></th><th :aria-sort="perfSort === 'role_name' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('role_name')">{{ tr('pgssUi.text057') }}<span aria-hidden="true">{{ sortMark('role_name', perfSort, perfDescending) }}</span></button></th><th class="num" :aria-sort="perfSort === 'metric' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('metric')">{{ selectedRank.label }}<span aria-hidden="true">{{ sortMark('metric', perfSort, perfDescending) }}</span></button></th><th class="num" :aria-sort="perfSort === 'calls' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('calls')">Calls<span aria-hidden="true">{{ sortMark('calls', perfSort, perfDescending) }}</span></button></th><th class="num" :aria-sort="perfSort === 'total_exec_time_ms' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('total_exec_time_ms')">Total exec<span aria-hidden="true">{{ sortMark('total_exec_time_ms', perfSort, perfDescending) }}</span></button></th><th class="num" :aria-sort="perfSort === 'mean_exec_time_ms' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('mean_exec_time_ms')">Mean / Max<span aria-hidden="true">{{ sortMark('mean_exec_time_ms', perfSort, perfDescending) }}</span></button></th><th class="num" :aria-sort="perfSort === 'temp_blocks' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('temp_blocks')">Temp blocks<span aria-hidden="true">{{ sortMark('temp_blocks', perfSort, perfDescending) }}</span></button></th><th class="num" :aria-sort="perfSort === 'wal_bytes' ? (perfDescending ? 'descending' : 'ascending') : 'none'"><button type="button" class="sort-head" @click="changePerfSort('wal_bytes')">WAL<span aria-hidden="true">{{ sortMark('wal_bytes', perfSort, perfDescending) }}</span></button></th><th>{{ tr('pgssUi.text058') }}</th></tr></thead>
        <tbody><tr v-for="row in perfRows.rows" :key="pgssIdentity(row)"><td class="mono">{{ row.queryid ?? tr('pgssFinal.queryidUnavailable') }}<small>{{ row.toplevel ? 'top-level' : 'nested' }} · {{ row.userid }}/{{ row.dbid }}</small></td><td>{{ row.role_name ?? 'OID '+row.userid }}<small>{{ row.database ?? 'OID '+row.dbid }}</small></td><td class="num strong" :title="ms(metric(row, rank))">{{ rankMetric(row) }}</td><td class="num">{{ n(row.calls) }}</td><td class="num" :title="ms(row.total_exec_time_ms)">{{ duration(row.total_exec_time_ms) }}</td><td class="num" :title="ms(row.mean_exec_time_ms) + ' / ' + ms(row.max_exec_time_ms)">{{ duration(row.mean_exec_time_ms) }} / {{ duration(row.max_exec_time_ms) }}</td><td class="num">{{ n(row.temp_blks_read + row.temp_blks_written) }}</td><td class="num">{{ bytes(row.wal_bytes) }}</td><td><button class="secondary small" :disabled="!row.queryid || state.loadingStatement" :title="!row.queryid ? tr('vueRemaining.queryHistoryUnavailable') : ''" @click="investigate(row)">{{ tr('pgssUi.text059') }}</button></td></tr>
        <tr v-if="!perfRows.rows.length"><td colspan="9">{{ tr('pgssUi.text060') }}</td></tr></tbody></table></div>
      <div class="pager"><span>{{ perfRows.total }}{{ tr('pgssFinal.recordsPage') }}{{ perfRows.page }} / {{ perfRows.pages }}</span><button class="secondary" :disabled="perfRows.page<=1" @click="perfPage--">{{ tr('pgssUi.text046') }}</button><button class="secondary" :disabled="perfRows.page>=perfRows.pages" @click="perfPage++">{{ tr('pgssUi.text047') }}</button></div>
      <p class="muted">{{ tr('pgssUi.text061') }}</p>
    </section>
  </template>

  <template v-if="tab === 'explorer'">
    <section class="panel">
      <h2>{{ tr('pgssUi.text062') }}</h2>
      <p class="warning">{{ tr('pgssFinal.liveWarning') }}{{ state.snapshotSummary?.id ?? '—' }}.
        {{ tr('pgssFinal.liveWarningDetail') }}</p>
      <p v-if="!canExplore" class="warning">{{ tr('pgssUi.text063') }}</p>
      <p><strong>{{ tr('pgssUi.text064') }}</strong> {{ explorerScope }}
        <span v-if="explorerFilters.userid != null" class="pill">{{ tr('pgssFinal.role') }}{{ roleName(explorerFilters.userid) }} (OID {{ explorerFilters.userid }})</span>
        <span v-if="explorerFilters.dbid != null" class="pill">{{ tr('pgssFinal.database') }}{{ databaseName(explorerFilters.dbid) }} (OID {{ explorerFilters.dbid }})</span>
        <span v-if="explorerFilters.command" class="pill">{{ explorerFilters.command }}</span>
        <span v-if="explorerFilters.one_call" class="pill">calls = 1</span>
        <span v-if="explorerFilters.min_calls != null" class="pill">calls ≥ {{ explorerFilters.min_calls }}</span>
        <span v-if="explorerFilters.max_calls != null" class="pill">calls ≤ {{ explorerFilters.max_calls }}</span>
        <span v-if="explorerFilters.min_total_exec_ms != null" class="pill">total ≥ {{ explorerFilters.min_total_exec_ms }}{{ tr('pgssFinish.milliseconds') }}</span>
        <span v-if="explorerFilters.call_tag" class="pill">CALL: {{ callTargetOptions.find(item => item.tag === explorerFilters.call_tag)?.label ?? explorerFilters.call_tag }}</span></p>
      <div class="toolbar compact explorer-toolbar">
        <label>{{ tr('pgssUi.text065') }} <input v-model="explorerQueryid" :placeholder="tr('pgssFinal.exampleQueryid')" @keyup.enter="loadExplorer(0)" /></label>
        <label>{{ tr('pgssUi.text066') }} <select v-model="explorerUserId"><option value="">{{ tr('pgssUi.text067') }}</option><option v-for="item in roleOptions" :key="item.id" :value="String(item.id)">{{ item.name }} (OID {{ item.id }})</option></select></label>
        <label>{{ tr('pgssUi.text068') }} <select v-model="explorerDbId"><option value="">{{ tr('pgssUi.text069') }}</option><option v-for="item in databaseOptions" :key="item.id" :value="String(item.id)">{{ item.name }} (OID {{ item.id }})</option></select></label>
        <label>{{ tr('pgssUi.text070') }} <select v-model="explorerCommand"><option value="">{{ tr('pgssUi.text071') }}</option><option v-for="item in commandOptions" :key="item" :value="item">{{ item }}</option></select></label>
        <label>CALL target <select v-model="explorerCallTag"><option value="">{{ tr('pgssUi.text072') }}</option><option v-for="item in callTargetOptions" :key="item.tag" :value="item.tag">{{ item.label }}</option></select></label>
        <label>{{ tr('pgssUi.text073') }} <select v-model="explorerOneCall"><option :value="false">{{ tr('pgssUi.text074') }}</option><option :value="true">{{ tr('pgssUi.text075') }}</option></select></label>
        <label>{{ tr('pgssUi.text076') }} <input v-model="explorerMinCalls" inputmode="numeric" :placeholder="tr('pgssFinal.notSet')" @keyup.enter="loadExplorer(0)" /></label>
        <label>{{ tr('pgssUi.text077') }} <input v-model="explorerMaxCalls" inputmode="numeric" :placeholder="tr('pgssFinal.notSet')" @keyup.enter="loadExplorer(0)" /></label>
        <label>{{ tr('pgssUi.text078') }} <input v-model="explorerMinExecMs" inputmode="decimal" :placeholder="tr('pgssFinal.notSet')" @keyup.enter="loadExplorer(0)" /></label>
        <label>{{ tr('pgssUi.text079') }} <select v-model="explorerSort"><option value="queryid">Statement (queryid)</option><option value="role">{{ tr('pgssUi.text057') }}</option><option value="total_exec">Total time</option><option value="mean_exec">Mean time</option><option value="calls">Calls</option><option value="wal">WAL</option><option value="temp">Temp blocks</option></select></label>
        <label>{{ tr('pgssUi.text080') }} <select v-model="explorerDirection"><option value="desc">{{ tr('pgssUi.text081') }}</option><option value="asc">{{ tr('pgssUi.text082') }}</option></select></label>
        <label>{{ tr('pgssUi.text083') }} <select v-model.number="explorerLimit"><option :value="10">10</option><option :value="25">25</option><option :value="50">50</option><option :value="100">100</option></select></label>
        <button type="button" :disabled="state.explorerLoading || !canExplore" @click="loadExplorer(0)">{{ state.explorerLoading ? tr('pgssFinal.reading') : tr('pgssFinal.searchRefresh') }}</button>
        <button type="button" class="secondary" :disabled="state.explorerLoading" @click="openExplorer()">{{ tr('pgssUi.text084') }}</button>
      </div>
      <p class="muted note">{{ tr('pgssUi.text085') }}</p>
      <p v-if="state.explorerError" class="warning" role="alert">{{ state.explorerError }}</p>
      <p v-if="state.explorerLoading" class="muted" role="status">{{ tr('pgssUi.text086') }}</p>
      <template v-if="state.explorer">
        <div class="table-scroll"><table><thead><tr><th :aria-sort="explorerFilters.sort === 'queryid' ? (explorerFilters.direction === 'asc' ? 'ascending' : 'descending') : 'none'"><button type="button" class="sort-head" :disabled="state.explorerLoading || !canExplore" @click="changeExplorerSort('queryid')">Statement<span aria-hidden="true">{{ sortMark('queryid', explorerFilters.sort ?? 'total_exec', explorerFilters.direction !== 'asc') }}</span></button></th><th :aria-sort="explorerFilters.sort === 'role' ? (explorerFilters.direction === 'asc' ? 'ascending' : 'descending') : 'none'"><button type="button" class="sort-head" :disabled="state.explorerLoading || !canExplore" @click="changeExplorerSort('role')">{{ tr('pgssUi.text057') }}<span aria-hidden="true">{{ sortMark('role', explorerFilters.sort ?? 'total_exec', explorerFilters.direction !== 'asc') }}</span></button></th><th class="num" :aria-sort="explorerFilters.sort === 'calls' ? (explorerFilters.direction === 'asc' ? 'ascending' : 'descending') : 'none'"><button type="button" class="sort-head" :disabled="state.explorerLoading || !canExplore" @click="changeExplorerSort('calls')">Calls<span aria-hidden="true">{{ sortMark('calls', explorerFilters.sort ?? 'total_exec', explorerFilters.direction !== 'asc') }}</span></button></th><th class="num" :aria-sort="explorerFilters.sort === 'total_exec' ? (explorerFilters.direction === 'asc' ? 'ascending' : 'descending') : 'none'"><button type="button" class="sort-head" :disabled="state.explorerLoading || !canExplore" @click="changeExplorerSort('total_exec')">Total exec<span aria-hidden="true">{{ sortMark('total_exec', explorerFilters.sort ?? 'total_exec', explorerFilters.direction !== 'asc') }}</span></button></th><th class="num" :aria-sort="explorerFilters.sort === 'mean_exec' ? (explorerFilters.direction === 'asc' ? 'ascending' : 'descending') : 'none'"><button type="button" class="sort-head" :disabled="state.explorerLoading || !canExplore" :title="tr('vueRemaining.meanSort')" @click="changeExplorerSort('mean_exec')">Mean / max<span aria-hidden="true">{{ sortMark('mean_exec', explorerFilters.sort ?? 'total_exec', explorerFilters.direction !== 'asc') }}</span></button></th><th class="num" :aria-sort="explorerFilters.sort === 'temp' ? (explorerFilters.direction === 'asc' ? 'ascending' : 'descending') : 'none'"><button type="button" class="sort-head" :disabled="state.explorerLoading || !canExplore" @click="changeExplorerSort('temp')">Temp blocks<span aria-hidden="true">{{ sortMark('temp', explorerFilters.sort ?? 'total_exec', explorerFilters.direction !== 'asc') }}</span></button></th><th class="num" :aria-sort="explorerFilters.sort === 'wal' ? (explorerFilters.direction === 'asc' ? 'ascending' : 'descending') : 'none'"><button type="button" class="sort-head" :disabled="state.explorerLoading || !canExplore" @click="changeExplorerSort('wal')">WAL<span aria-hidden="true">{{ sortMark('wal', explorerFilters.sort ?? 'total_exec', explorerFilters.direction !== 'asc') }}</span></button></th><th>{{ tr('pgssUi.text059') }}</th></tr></thead>
        <tbody><tr v-for="row in state.explorer.rows" :key="pgssIdentity(row)">
          <td class="mono">{{ row.queryid ?? tr('pgssFinal.queryidUnavailable') }}<small>{{ row.toplevel ? 'top-level' : 'nested' }} · {{ row.userid }}/{{ row.dbid }}</small></td>
          <td>{{ row.role_name ?? `OID ${row.userid}` }}<small>{{ row.database ?? `OID ${row.dbid}` }}</small></td>
          <td class="num">{{ n(row.calls) }}</td><td class="num">{{ duration(row.total_exec_time_ms) }}</td><td class="num">{{ duration(row.mean_exec_time_ms) }} / {{ duration(row.max_exec_time_ms) }}</td>
          <td class="num">{{ n(row.temp_blks_read + row.temp_blks_written) }}</td><td class="num">{{ bytes(row.wal_bytes) }}</td>
          <td><button type="button" class="secondary small" :disabled="!row.queryid || !canExplore" @click="exploreLiveStatement(row)">{{ tr('pgssUi.text059') }}</button></td>
        </tr><tr v-if="!state.explorer.rows.length"><td colspan="8">{{ tr('pgssUi.text087') }}</td></tr></tbody></table></div>
        <div class="pager"><span><strong>{{ tr('pgssFinal.totalFound') }}{{ n(state.explorer.total) }}</strong>{{ tr('pgssFinal.results') }}{{ n(state.explorer.offset + (state.explorer.rows.length ? 1 : 0)) }}–{{ n(state.explorer.offset + state.explorer.rows.length) }}{{ tr('pgssFinal.offsetLimit') }}</span>
          <button type="button" class="secondary" :disabled="state.explorerLoading || state.explorer.offset === 0" @click="loadExplorer(Math.max(0, state.explorer.offset - state.explorer.limit), true)">{{ tr('pgssUi.text046') }}</button>
          <button type="button" class="secondary" :disabled="state.explorerLoading || !state.explorer.has_more || state.explorer.offset + state.explorer.limit > 20000" @click="loadExplorer(state.explorer.offset + state.explorer.limit, true)">{{ tr('pgssUi.text047') }}</button>
        </div>
      </template>
    </section>
  </template>

  <dialog v-if="statementOpen" ref="statementDialog" class="pgss-statement-modal" aria-label="Statement Investigation" @close="onStatementDialogClosed">
    <section class="panel">
      <div class="detail-head"><h2>Statement Investigation</h2><button type="button" class="secondary" @click="closeStatementDialog">{{ tr('pgssUi.text088') }}</button></div>
      <p v-if="!state.selectedRow" class="empty">{{ tr('pgssUi.text089') }}</p>
      <template v-else>
        <p class="mono id-text">queryid {{ state.selectedRow.queryid }} · userid {{ state.selectedRow.userid }} · dbid {{ state.selectedRow.dbid }} · {{ state.selectedRow.toplevel ? 'top-level' : 'nested' }}</p>
        <p class="muted">{{ tr('pgssUi.text090') }}</p>
        <button class="secondary small" type="button" @click="emit('openHistory', state.connectionId, state.selectedRow.queryid); closeStatementDialog()">{{ tr('pgssUi.text091') }}</button>
        <div class="toolbar compact"><button class="secondary" :disabled="state.loadingDetail || !canExplore" @click="controller.refreshDetail()">{{ state.loadingDetail ? tr('pgssFinal.loadingSql') : tr('pgssFinal.getCurrentSql') }}</button></div>
        <p v-if="!canExplore" class="warning">{{ tr('pgssUi.text092') }}</p>
        <p v-if="state.detailError" class="warning">{{ state.detailError }}{{ tr('pgssFinal.disappeared') }}</p>
        <div v-if="state.detail" class="detail">
          <div class="detail-head"><h3>{{ tr('pgssUi.text093') }}</h3><span class="pill warn">{{ tr('pgssUi.text094') }}</span></div>
          <p v-if="state.detail.query_truncated === true" class="warning">{{ tr('pgssUi.text095') }}</p>
          <p v-else-if="state.detail.query_truncated == null" class="muted">{{ tr('pgssUi.text096') }}</p>
          <div v-if="state.detail.query"><pre class="sql">{{ state.detail.query }}</pre><button class="secondary small" @click="copySql(state.detail.query ?? '')">{{ tr('pgssUi.text097') }}</button></div>
          <p v-else class="warning">{{ tr('pgssUi.text098') }}</p>
          <div class="properties"><div><span>Calls</span><strong>{{ n(state.detail.calls) }}</strong></div><div><span>Total exec</span><strong>{{ ms(state.detail.total_exec_time_ms) }}</strong></div><div><span>Mean exec</span><strong>{{ ms(state.detail.mean_exec_time_ms) }}</strong></div><div><span>WAL</span><strong>{{ bytes(state.detail.wal_bytes) }}</strong></div></div>
        </div>
      </template>
    </section>
  </dialog>

  <template v-if="tab === 'reference'">
    <section class="panel">
      <h2>{{ tr('pgssUi.text099') }}</h2>
      <p class="muted">{{ tr('pgssUi.text100') }}</p>
      <p v-if="state.loadingReference" role="status">{{ tr('pgssUi.text101') }}</p>
      <button v-if="!state.reference && !state.loadingReference" class="secondary" @click="controller.loadReference()">{{ tr('pgssUi.text102') }}</button>
      <template v-if="state.reference">
        <h3>{{ tr('pgssUi.text103') }}</h3>
        <div class="method-list compact-method"><details v-for="(step,i) in state.reference.methodology" :key="step.id" class="finding" :open="i === 0"><summary>{{ i+1 }}. {{ step.title }}</summary><p>{{ step.purpose }}</p><ul><li v-for="(line,j) in step.how_to_read" :key="j">{{ line }}</li></ul></details></div>
        <h3>{{ tr('pgssUi.text104') }}</h3>
        <div class="toolbar compact"><label>{{ tr('pgssUi.text105') }} <select v-model="referenceQuery"><option :value="null">{{ tr('pgssUi.text106') }}</option><option v-for="q in state.reference.queries" :key="q.id" :value="q.id">{{ q.title }}</option></select></label></div>
        <article v-for="q in state.reference.queries.filter(q=>q.id===referenceQuery)" :key="q.id" class="finding"><h4>{{ q.title }}</h4><p>{{ q.purpose }}</p><pre class="sql">{{ q.sql }}</pre><button class="secondary small" @click="copySql(q.sql)">{{ tr('pgssUi.text097') }}</button><ul><li v-for="(line,i) in q.how_to_read" :key="i">{{ line }}</li></ul></article>
        <h3>{{ tr('pgssUi.text107') }}</h3>
        <ul><li v-for="source in state.reference.sources" :key="source.id"><a v-if="docs(source.path)" :href="docs(source.path)!" target="_blank" rel="noopener noreferrer">{{ source.title }}</a><span v-else>{{ source.title }}</span> — {{ source.purpose }}</li></ul>
      </template>
    </section>
  </template>
</section>
</template>

<style scoped>
.pgss { width:100%; min-width:0; display:grid; gap:1rem; color:var(--text); }
.heading, .snapshot-strip, .detail-head { display:flex; flex-wrap:wrap; align-items:center; justify-content:space-between; gap:.8rem; }
h1 { margin:0; font-size:1.65rem; } h1 small { font-size:.85rem; font-weight:400; color:var(--text-secondary); }
h2 { margin:0 0 .65rem; font-size:1.2rem; } h3 { margin:.8rem 0 .55rem; font-size:1rem; } h4 { margin:.3rem 0 .4rem; }
p { line-height:1.55; } p:first-child { margin-top:0; }
.muted { color:var(--text-secondary); } .note { margin:-.45rem 0 0; font-size:.84rem; }
.pill { display:inline-block; white-space:nowrap; border:1px solid var(--border-strong); border-radius:2rem; padding:.35rem .7rem; font-size:.78rem; color:var(--text-secondary); }
.pill.warn { border-color:var(--warning, var(--border-strong)); }
.toolbar, .tabs, .rank-tabs { display:flex; flex-wrap:wrap; align-items:flex-end; gap:.6rem; }
.toolbar label { display:grid; gap:.3rem; color:var(--text-secondary); font-size:.82rem; min-width:135px; }
.toolbar select, .toolbar input { min-height:38px; max-width:100%; border:1px solid var(--border-strong); border-radius:.5rem; padding:.45rem .6rem; background:var(--surface-2); color:var(--text); }
.toolbar:not(.compact)>label:first-child { flex:1; min-width:260px; }
.compact { margin:.7rem 0; }
.compact input { width:230px; }
.explorer-toolbar input { width:270px; }
.kpis .small { width:max-content; max-width:100%; }
.tabs { border-bottom:1px solid var(--border-strong); gap:.2rem; }
.tab { border:0; border-radius:.5rem .5rem 0 0; background:transparent; color:var(--text-secondary); padding:.65rem .85rem; }
.tab:hover, .tab.selected { background:var(--surface-2); color:var(--text); }
.tab.selected { box-shadow:inset 0 -2px var(--accent); font-weight:650; }
.snapshot-strip { border:1px solid var(--border); padding:.7rem .9rem; border-radius:.6rem; background:var(--surface-2); font-size:.9rem; }
.kpis { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:.7rem; }
.kpis article { min-width:0; padding:.9rem; border:1px solid var(--border); border-radius:.65rem; background:var(--surface); display:grid; gap:.3rem; }
.kpis span { color:var(--text-secondary); font-size:.8rem; }
.kpis strong { font-size:1.25rem; overflow-wrap:anywhere; font-variant-numeric:tabular-nums; }
.kpis small { font-size:.74rem; color:var(--text-secondary); }
.small-kpis strong { font-size:1rem; }
.panel { min-width:0; border:1px solid var(--border); background:var(--surface); padding:1rem; border-radius:.8rem; }
.properties { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:.45rem 1.2rem; }
.properties div { display:flex; gap:.4rem; justify-content:space-between; border-bottom:1px solid var(--border); padding:.5rem 0; align-items:center; }
.properties span { color:var(--text-secondary); font-size:.86rem; }
.properties strong { text-align:right; overflow-wrap:anywhere; }
.error, .warning, .notice, .empty { padding:.75rem .85rem; border-radius:.55rem; border:1px solid var(--border-strong); overflow-wrap:anywhere; }
.error { color:var(--error); } .warning { color:var(--warning, var(--text)); background:var(--surface-2); }
.notice { color:var(--text); background:var(--surface-2); }
.empty { color:var(--text-secondary); border-style:dashed; }
.findings, .method-list { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,370px),1fr)); gap:.7rem; }
.finding { min-width:0; border:1px solid var(--border); padding:.85rem; border-radius:.65rem; background:var(--surface-2); overflow-wrap:anywhere; }
.finding ul { padding-left:1.2rem; margin:.4rem 0 .8rem; } li { margin:.3rem 0; line-height:1.45; }
.compact-findings, .compact-method { grid-template-columns:repeat(auto-fill,minmax(min(100%,350px),1fr)); align-items:start; }
.compact-findings .finding summary, .compact-method .finding summary { cursor:pointer; font-weight:650; line-height:1.45; padding:.2rem; }
.compact-findings .finding[open], .compact-method .finding[open] { grid-column:span 1; }
.triage-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,255px),1fr)); gap:.65rem; margin-top:.7rem; }
.triage-item { display:flex; flex-direction:column; min-width:0; gap:.45rem; border:1px solid var(--border); border-radius:.65rem; background:var(--surface-2); padding:.85rem; font-size:.86rem; }
.triage-item strong { font-size:1.25rem; font-variant-numeric:tabular-nums; }
.triage-item .small { align-self:flex-start; margin-top:auto; }
.disclosure { margin:.8rem 0; padding:.7rem; border:1px solid var(--border); border-radius:.5rem; }
summary { cursor:pointer; font-weight:600; }
.rank-tabs { margin:.7rem 0; gap:.35rem; } .rank-tabs button { padding:.5rem .7rem; font-size:.83rem; }
.rank-tabs .active { border-color:var(--accent); background:var(--surface-2); font-weight:700; }
.table-scroll { overflow:auto; max-width:100%; margin:.6rem 0; }
table { width:100%; border-collapse:collapse; min-width:760px; font-size:.85rem; }
th,td { text-align:left; padding:.65rem .6rem; border-bottom:1px solid var(--border); vertical-align:top; }
th { color:var(--text-secondary); font-weight:650; white-space:nowrap; background:var(--surface-2); }
tr.current td { background:var(--surface-2); }
.sort-head { display:inline-flex; gap:.3rem; align-items:center; border:0; background:transparent; color:inherit; font:inherit; font-weight:inherit; padding:0; cursor:pointer; text-align:inherit; white-space:nowrap; }
.sort-head:hover, .sort-head:focus-visible { color:var(--text); text-decoration:underline; text-underline-offset:3px; }
.sort-head span { color:var(--accent); font-size:.85em; }
th.num .sort-head { justify-content:flex-end; }
.num { text-align:right; white-space:nowrap; font-variant-numeric:tabular-nums; }
.strong { font-weight:700; }
.mono, .sql { font-family:ui-monospace,SFMono-Regular,Consolas,monospace; font-size:.83rem; overflow-wrap:anywhere; }
.id-text { overflow-wrap:anywhere; }
td small { display:block; font-size:.73rem; color:var(--text-secondary); margin-top:.2rem; }
.sql { max-height:440px; overflow:auto; white-space:pre; padding:1rem; border-radius:.5rem; background:var(--surface-2); border:1px solid var(--border); }
.pager { display:flex; gap:.5rem; align-items:center; justify-content:flex-end; color:var(--text-secondary); font-size:.83rem; margin-top:.6rem; }
.pager span { margin-right:auto; }
.small { padding:.4rem .6rem; font-size:.78rem; }
.detail { border:1px solid var(--border); border-radius:.6rem; padding:.85rem; margin:.8rem 0; }
a { color:var(--accent); text-decoration:underline; text-underline-offset:3px; }
dialog.pgss-statement-modal { width:min(1120px, calc(100vw - 28px)); max-width:calc(100vw - 28px); max-height:90vh; overflow:auto; padding:0; border:1px solid var(--border-strong); border-radius:.8rem; background:var(--surface); color:var(--text); }
dialog.pgss-statement-modal::backdrop { background:rgba(0,0,0,.75); }
.pgss-statement-modal .panel { border:0; }
.pgss-statement-modal .detail-head { position:sticky; top:0; z-index:1; background:var(--surface); padding:.3rem 0 .65rem; border-bottom:1px solid var(--border); }
@media(max-width:1000px) { .kpis { grid-template-columns:repeat(2,minmax(0,1fr)); } }
@media(max-width:640px) { .properties, .kpis { grid-template-columns:1fr; } .toolbar>* { flex:1; } .rank-tabs button { flex:1; } }
</style>
