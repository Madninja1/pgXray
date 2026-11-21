<script setup lang="ts">
import { locale, t as tr } from "../i18n/locale-service.js";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { avWorkloadWindows } from "./workload.js";
import { avUpdateTotals, avUpdatePeriod } from "./update.js";
import { avMaintenanceIndicator, avHotGuidance, avMechanismWindowEvidence } from "./mechanisms.js";
import { avComparisonOverview } from "./trends.js";
import { avUnverifiedPolicyOptions } from "./policy-preview.js";
import { avUniqueNotes, avAdviceNextStep, avAdvisorEvidenceMessage, avAdvisorRequirements } from "./presentation.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import {
  createAvController, createAvState, avPage, avSummaryCounts, avSummaryMatchesConnection,
  avDocsUrl, completeMaintenanceDelta, avSnapshotLabel, avStatsResetExplanation, avFormatFactor, avSimulationComparison, avTablePressure, type AvCandidate, type AvTable, type AvTableSortKey, type AvTableSortDirection, type AvTablePressureKind,
} from "./controller.js";

const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; navigationToken?: number; preferredConnectionId?: number | null; preferredRelationOid?: number | null; quickSelection?: QuickConnectionSelection | null;}>();
const emit = defineEmits<{openHistory: [connectionId: number | null, relationOid: number | null]; openPgss: [connectionId: number, queryid: string]} >();
function openPgssEvidence(queryid: string | null): void {
  const connectionId = state.connectionId;
  if (connectionId !== null && queryid) emit("openPgss", connectionId, queryid);
}
const state = reactive(createAvState());
const controller = createAvController(props.shell, state, props.invoke);
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.collecting || state.applying || state.loading)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.collecting || state.applying || state.loading) return;
  lastShortcutSerial = request.serial;
  // Never retarget a running collector/monitor or an in-flight database operation.
  if (state.connectionId !== request.connectionId) controller.selectConnection(request.connectionId);
});

const search = ref("");
const mode = ref("all");
const page = ref(1);
const pageSize = ref(25);
const tableMode = ref<"maintenance" | "workload">("maintenance");
type AvDetailTab = "state" | "advisor" | "policy";
const detailTab = ref<AvDetailTab>("state");
const pressureKinds: readonly AvTablePressureKind[] = ["dead", "insert", "analyze"];
const tableSort = ref<AvTableSortKey>("vacuum");
const sortDirection = ref<AvTableSortDirection>("desc");
const compareSearch = ref("");
const targetDeadTuples = ref("");
const confirmText = ref("");
const understood = ref(false);
const selected = computed(() => state.connections.find(c => c.id === state.connectionId));
const saved = computed(() => state.histories.find(s => s.id === state.snapshotId));
const liveAllowed = computed(() => avSummaryMatchesConnection(saved.value, selected.value));
const snapshot = computed(() => state.snapshot);
const counts = computed(() => avSummaryCounts(snapshot.value));
const tablePage = computed(() => avPage(snapshot.value, search.value, mode.value, page.value, pageSize.value, tableSort.value, sortDirection.value));
const activeTable = computed(() => snapshot.value?.tables.data?.find(t => t.relation_oid === state.selectedTableOid));
const shownComparison = computed(() => {
  const comparison = state.comparison;
  return comparison && state.compareOlderId === comparison.older.id &&
    state.compareNewerId === comparison.newer.id ? comparison : null;
});
const comparisonOverview = computed(() => avComparisonOverview(shownComparison.value));
const changes = computed(() => (shownComparison.value?.table_changes ?? []).filter(item =>
  `${item.newer_name ?? ""} ${item.older_name ?? ""} ${item.relation_oid}`.toLocaleLowerCase().includes(compareSearch.value.toLocaleLowerCase())).slice(0, 50));
const pending = computed(() => state.candidates?.candidates.find(c => c.id === state.pendingCandidateId));
const busy = computed(() => state.loading || state.collecting || state.applying);
const evidence = computed(() => snapshot.value?.overview.track_counts === true && !!snapshot.value?.tables.data);
const observedIntervals = computed(() => evidence.value ? avWorkloadWindows(state.relationHistory) : []);
const latestObservedInterval = computed(() => observedIntervals.value[0] ?? null);
const updateTotals = computed(() => evidence.value ? avUpdateTotals(state.detail?.table ?? null) : null);
const sortedIndexes = computed(() => [...(state.detail?.indexes.data ?? [])].sort(
  (left, right) => (right.index_size_bytes - left.index_size_bytes) || left.index_name.localeCompare(right.index_name, "ru"),
));
const recentUpdatePeriod = computed(() => avUpdatePeriod(latestObservedInterval.value));
function dateTime(ms: number): string { return new Date(ms).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US"); }
function duration(minutes: number): string {
  const value = (minutes >= 120 ? minutes / 60 : minutes).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 1 });
  return minutes >= 120 ? tr('autovacuumUi.dynamicHours', { value }) : tr('autovacuumUi.dynamicMinutes', { value });
}
function formatExecutionTime(milliseconds: number): string {
  return milliseconds.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US");
}
function observedRate(value: number | null | undefined): string { return value == null || !Number.isFinite(value) ? "—" : `≈ ${value.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", {maximumFractionDigits: 1})}`; }
const mechanismCards = computed(() => {
  const table = state.detail?.table;
  if (!table) return [];
  return [
    { key: "dead" as const, title: tr('autovacuumUi.mechanismDeadTitle'), metric: tr('autovacuumUi.mechanismDeadMetric'), help: tr('autovacuumUi.mechanismDeadHelp'), result: avMaintenanceIndicator(table, "dead", evidence.value), interval: avMechanismWindowEvidence("dead", latestObservedInterval.value) },
    { key: "insert" as const, title: tr('autovacuumUi.mechanismInsertTitle'), metric: tr('autovacuumUi.mechanismInsertMetric'), help: tr('autovacuumUi.mechanismInsertHelp'), result: avMaintenanceIndicator(table, "insert", evidence.value), interval: avMechanismWindowEvidence("insert", latestObservedInterval.value) },
    { key: "analyze" as const, title: tr('autovacuumUi.mechanismAnalyzeTitle'), metric: tr('autovacuumUi.mechanismAnalyzeMetric'), help: tr('autovacuumUi.mechanismAnalyzeHelp'), result: avMaintenanceIndicator(table, "analyze", evidence.value), interval: avMechanismWindowEvidence("analyze", latestObservedInterval.value) },
  ];
});
const hotGuidance = computed(() => state.detail ? avHotGuidance(state.detail.table, evidence.value) : null);
function switchTableMode(next: "maintenance" | "workload"): void {
  if (tableMode.value === next) return;
  tableMode.value = next;
  tableSort.value = next === "maintenance" ? "vacuum" : "updated";
  sortDirection.value = "desc";
  page.value = 1;
}
function tablePressure(t: AvTable, kind: AvTablePressureKind) {
  return avTablePressure(t, kind, evidence.value);
}
function sortTables(key: AvTableSortKey): void {
  if (tableSort.value === key) sortDirection.value = sortDirection.value === "asc" ? "desc" : "asc";
  else { tableSort.value = key; sortDirection.value = key === "name" ? "asc" : "desc"; }
  page.value = 1;
}
function sortSymbol(key: AvTableSortKey): string { return tableSort.value !== key ? "↕" : sortDirection.value === "asc" ? "↑" : "↓"; }
function ariaSort(key: AvTableSortKey): "ascending" | "descending" | "none" {
  return tableSort.value !== key ? "none" : sortDirection.value === "asc" ? "ascending" : "descending";
}
function progressWidth(value: number | null): string { return `${Math.min(100, Math.max(0, value ?? 0))}%`; }
const resetExplanation = computed(() => snapshot.value
  ? avStatsResetExplanation(snapshot.value.overview.track_counts, snapshot.value.overview.stats_reset) : null);
const adviceLimitations = computed(() => avUniqueNotes(state.advice?.limitations));
const candidateLimitations = computed(() => avUniqueNotes(state.candidates?.limitations));
// A NULL stats_reset does not validate historical rates. Offer current-state
// threshold scenarios for DBA review without creating actionable candidates.
const statsResetUnknown = computed(() => snapshot.value?.overview.track_counts === true && snapshot.value?.overview.stats_reset == null);
const unverifiedPolicyOptions = computed(() => avUnverifiedPolicyOptions(
  snapshot.value?.overview.stats_reset ?? null,
  snapshot.value?.overview.track_counts === true,
  state.detail,
));
const advisorEvidenceMessage = computed(() => avAdvisorEvidenceMessage(state.advice, {
  savedPoints: state.relationHistory?.summary.history_points ?? 0,
  resetUnknown: snapshot.value?.overview.stats_reset == null,
}));
const advisorRequirements = computed(() => avAdvisorRequirements(state.advice, {
  savedPoints: state.relationHistory?.summary.history_points ?? 0,
  trackCounts: snapshot.value?.overview.track_counts === true,
  statsReset: snapshot.value?.overview.stats_reset ?? null,
}));
const simulationComparison = computed(() => state.detail && state.simulation
  ? avSimulationComparison(state.detail.table, state.simulation) : null);
const currentHistoryLabel = computed(() => saved.value ? avSnapshotLabel(saved.value, true) : null);
function simulateDesired(): void {
  const text = targetDeadTuples.value.trim();
  void controller.simulate(text ? Number(text) : Number.NaN);
}
function inspectUnverifiedPolicy(trigger: number): void {
  targetDeadTuples.value = String(trigger);
  simulateDesired();
}
function chooseExample(percent: number): void {
  const trigger = state.detail?.table.dead_tuple_trigger;
  if (trigger && Number.isFinite(trigger) && trigger > 0) targetDeadTuples.value = String(Math.max(1, Math.round(trigger * percent / 100)));
}
const failures = computed(() => {
  const s = snapshot.value;
  if (!s) return [];
  const result: string[] = [];
  if (!s.overview.track_counts) result.push(tr('autovacuumUi.trackCountsOff'));
  for (const [name, part] of [[tr('autovacuumUi.sectionTables'), s.tables], [tr('autovacuumUi.sectionVacuum'), s.vacuums], [tr('autovacuumUi.sectionAnalyze'), s.analyzes]] as const) {
    if (part.error) result.push(`${name}: ${part.error}`);
  }
  return result;
});
const important = computed(() => snapshot.value?.interpretation.items.filter(i => i.severity !== "info") ?? []);
const pendingDisabled = computed(() => state.applying || state.applyBlocked || confirmText.value.trim() !== "ПРИМЕНИТЬ" || !understood.value || !liveAllowed.value);
function int(value: number | null | undefined): string {
  return value === null || value === undefined || !Number.isFinite(value) ? "—" : value.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 1 });
}
function percent(value: number | null | undefined): string { return value === null || value === undefined ? "—" : `${int(value)}%`; }
function bytes(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return "—";
  const abs = Math.abs(value), unit = abs < 1024 ? 1 : abs < 1024 ** 2 ? 1024 : abs < 1024 ** 3 ? 1024 ** 2 : abs < 1024 ** 4 ? 1024 ** 3 : 1024 ** 4;
  const suffix = unit === 1 ? tr('autovacuumUi.unitBytes') : unit === 1024 ? tr('autovacuumUi.unitKib') : unit === 1024 ** 2 ? tr('autovacuumUi.unitMib') : unit === 1024 ** 3 ? tr('autovacuumUi.unitGib') : tr('autovacuumUi.unitTib');
  return `${int(value / unit)} ${suffix}`;
}
function date(value: number | string | null | undefined): string {
  if (value === null || value === undefined || value === "") return "—";
  const timestamp = typeof value === "number" ? value : Date.parse(value);
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US") : String(value);
}
function signed(value: number | null | undefined): string { return value === null || value === undefined ? "—" : `${value > 0 ? "+" : ""}${int(value)}`; }
function tag(t: AvTable): string {
  if (t.autovacuum_applicable && !t.autovacuum_enabled) return tr('autovacuumUi.disabledStatus');
  if (!evidence.value) return tr('autovacuumUi.countersUnavailable');
  const reached = [t.dead_trigger_reached && "VACUUM dead", t.insert_trigger_reached && "VACUUM INSERT",
    t.analyze_trigger_reached && "ANALYZE"].filter(Boolean);
  return reached.length ? reached.join(" · ") : tr('autovacuumUi.noThresholdSignals');
}
function findingLabel(status: string): string {
  const names: Record<string, string> = { investigate: tr('autovacuumUi.findingInvestigate'), candidate: tr('autovacuumUi.findingCandidate'), review: tr('autovacuumUi.findingReview'),
    insufficient_quality: tr('autovacuumUi.findingInsufficientQuality'), insufficient_history: tr('autovacuumUi.findingInsufficientHistory'), observe: tr('autovacuumUi.findingObserve'), working: tr('autovacuumUi.findingWorking') };
  return names[status] ?? status;
}
function areaLabel(area: string): string {
  const names: Record<string, string> = { vacuum: "VACUUM", insert: tr('autovacuumUi.insertVacuum'), analyze: "ANALYZE", toast: "TOAST", hot: "HOT" };
  return names[area] ?? area;
}
function onConnection(event: Event): void {
  const raw = Number((event.target as HTMLSelectElement).value);
  controller.selectConnection(Number.isSafeInteger(raw) && raw > 0 ? raw : null);
  page.value = 1;
}
function bootstrap(): void {
  if (props.shell.screen === "workspace" && !props.shell.busy && props.shell.currentUsername && !state.connections.length) {
    void controller.refreshConnections().then(() => controller.loadReference());
  }
}
async function openTable(t: AvTable): Promise<void> {
  detailTab.value = "state";
  await controller.selectTable(t.relation_oid);
}
async function reloadTable(): Promise<void> { if (state.selectedTableOid !== null) await controller.selectTable(state.selectedTableOid); }
function prepare(candidate: AvCandidate): void {
  confirmText.value = ""; understood.value = false;
  controller.prepareApply(candidate.id);
}
async function confirmApply(): Promise<void> {
  if (pendingDisabled.value) return;
  await controller.apply();
  confirmText.value = ""; understood.value = false;
}
function cancelApply(): void { controller.cancelApply(); confirmText.value = ""; understood.value = false; }
watch([search, mode, pageSize], () => { page.value = 1; });
watch(() => state.selectedTableOid, () => { targetDeadTuples.value = ""; detailTab.value = "state"; });
watch(() => state.connectionId, () => { detailTab.value = "state"; });
watch(() => state.snapshotId, () => { detailTab.value = "state"; });
watch(() => props.shell.authGeneration, () => { controller.reset(); bootstrap(); });
watch(() => props.shell.screen, screen => { if (screen !== "workspace") controller.reset(); else bootstrap(); });
watch(() => props.shell.busy, bootstrap);
watch(() => props.visible, visible => {
  if (!visible) return;
  if (state.connections.length && props.shell.screen === "workspace" && !props.shell.busy)
    void controller.refreshConnections();
  else bootstrap();
});
let pendingNavigation = false;
watch(() => props.navigationToken, (next, previous) => {
  if (next === previous || !props.visible) return;
  pendingNavigation = true;
  void controller.refreshConnections();
});
watch([() => props.navigationToken, () => state.connections.length, () => state.snapshotId], () => {
  if (!pendingNavigation || !props.visible) return;
  const id = props.preferredConnectionId;
  if (id != null && state.connections.some(c => c.id === id) && state.connectionId !== id) {
    controller.selectConnection(id);
    return;
  }
  if (!state.snapshot || state.loading) return;
  pendingNavigation = false;
  const oid = props.preferredRelationOid;
  if (oid != null && state.snapshot.tables.data?.some(t => t.relation_oid === oid))
    void controller.selectTable(oid);
});
onMounted(bootstrap);
onUnmounted(() => controller.dispose());
</script>

<template>
<section class="av-view" :aria-label="tr('autovacuumUi.viewAria')">
  <header class="section-head">
    <div><h1>{{ tr('autovacuumUi.text000') }} <small>{{ tr('autovacuumUi.text001') }}</small></h1>
      <p class="muted">{{ tr('autovacuumUi.text002') }}</p></div>
    <span class="tag">{{ tr('autovacuumUi.text003') }}</span>
  </header>

  <section class="panel controls">
    <label>{{ tr('autovacuumUi.text004') }}
      <select :value="state.connectionId ?? ''" :disabled="state.applying" @change="onConnection">
        <option value="">{{ tr('autovacuumUi.text005') }}</option>
        <option v-for="c in state.connections" :key="c.id" :value="c.id">{{ c.name }} · {{ c.host }}:{{ c.port }}/{{ c.database }}</option>
      </select>
    </label>
    <button type="button" :disabled="state.connectionId === null || busy" @click="controller.collect">{{ state.collecting ? tr('autovacuumUi.collecting') : tr('autovacuumUi.collectSnapshot') }}</button>
    <button type="button" class="secondary" @click="emit('openHistory',state.connectionId,state.selectedTableOid)">{{ tr('autovacuumUi.text006') }}</button>
    <span v-if="snapshot" class="muted">{{ tr('avFinal.snapshot') }}{{ currentHistoryLabel ?? date(snapshot.overview.measured_at) }} · PostgreSQL {{ snapshot.overview.server_version }}</span>
  </section>
  <p v-if="state.error" class="message error" role="alert">{{ state.error }}</p>
  <p v-if="state.notice" class="message" role="status">{{ state.notice }}</p>
  <p v-if="state.applyBlocked" class="message error" role="alert"><strong>{{ tr('autovacuumUi.text007') }}</strong> {{ tr('autovacuumUi.text008') }}</p>
  <p v-if="!snapshot && !state.loading && !state.collecting" class="empty">{{ tr('autovacuumUi.text009') }}</p>

  <template v-if="snapshot">
    <section class="panel">
      <div class="section-head"><div><h2>{{ tr('autovacuumUi.text010') }}</h2><p class="muted">{{ tr('autovacuumUi.text011') }}</p></div>
        <span class="tag">{{ snapshot.overview.autovacuum ? tr('autovacuumUi.maintenanceEnabled') : tr('autovacuumUi.maintenanceDisabled') }}</span></div>
      <div class="metrics">
        <article><span>{{ tr('autovacuumUi.text012') }}</span><strong>{{ int(counts.total) }}</strong><small>{{ tr('autovacuumUi.text013') }}</small></article>
        <article><span>{{ tr('autovacuumUi.text014') }}</span><strong>{{ int(counts.dead) }}</strong><small>{{ tr('autovacuumUi.text015') }}</small></article>
        <article><span>{{ tr('autovacuumUi.text016') }}</span><strong>{{ int(counts.insert) }}</strong><small>{{ tr('autovacuumUi.text017') }}</small></article>
        <article><span>{{ tr('autovacuumUi.text018') }}</span><strong>{{ int(counts.analyze) }}</strong><small>{{ tr('autovacuumUi.text019') }}</small></article>
        <article><span>{{ tr('autovacuumUi.text020') }}</span><strong>{{ int(counts.disabled) }}</strong><small>{{ tr('autovacuumUi.text021') }}</small></article>
        <article><span>{{ tr('autovacuumUi.text022') }}</span><strong>{{ int(counts.activeVacuum) }}</strong><small>{{ tr('autovacuumUi.text023') }}</small></article>
        <article><span>{{ tr('autovacuumUi.text024') }}</span><strong>{{ int(counts.activeAnalyze) }}</strong><small>{{ tr('autovacuumUi.text025') }}</small></article>
      </div>
      <div class="assessment"><h3>{{ tr('autovacuumUi.text026') }}</h3>
        <p v-if="!evidence" class="warning">{{ tr('autovacuumUi.text027') }}</p>
        <p v-else-if="(counts.vacuum ?? 0) > 0 || (counts.analyze ?? 0) > 0">{{ tr('autovacuumUi.text028') }}</p>
        <p v-else>{{ tr('autovacuumUi.text029') }}</p>
        <p v-if="resetExplanation" :class="snapshot.overview.stats_reset == null && snapshot.overview.track_counts ? 'warning' : 'muted'" role="status">{{ resetExplanation }}</p>
        <p class="muted">{{ tr('autovacuumUi.text030') }}</p>
      </div>
      <div v-if="failures.length" class="quality"><h3>{{ tr('autovacuumUi.text031') }}</h3><ul><li v-for="(e, i) in failures" :key="i">{{ e }}</li></ul>
        <p class="muted">{{ tr('autovacuumUi.text032') }}</p></div>
      <div v-if="important.length" class="insights"><article v-for="item in important.slice(0, 8)" :key="item.id" :class="item.severity"><span class="severity">{{ item.severity === 'warning' ? tr('autovacuumUi.needsReview') : tr('autovacuumUi.needsAttention') }}</span><strong>{{ item.title }}</strong>
        <p v-for="(line, i) in item.meaning.slice(0, 2)" :key="i">{{ line }}</p>
        <small v-for="(line, i) in item.next_steps.slice(0, 2)" :key="`next-${i}`">{{ tr('autovacuumUi.whatToCheck') }} {{ line }}</small></article></div>
    </section>

    <section class="panel av-table-overview">
      <div class="section-head"><div><h2>{{ tr('autovacuumUi.text033') }}</h2><p class="muted">{{ tr('autovacuumUi.text034') }} <strong>{{ tr('autovacuumUi.text035') }}</strong> {{ tr('autovacuumUi.text036') }} <strong>{{ tr('autovacuumUi.text037') }}</strong>.</p></div>
        <span class="tag">{{ tr('autovacuumUi.foundCount', { count: String(tablePage.total) }) }}</span></div>
      <div class="av-table-modes" role="group" :aria-label="tr('autovacuumUi.tableModeAria')">
        <button type="button" :class="{ active: tableMode === 'maintenance' }" :aria-pressed="tableMode === 'maintenance'" @click="switchTableMode('maintenance')">{{ tr('autovacuumUi.text038') }}</button>
        <button type="button" :class="{ active: tableMode === 'workload' }" :aria-pressed="tableMode === 'workload'" @click="switchTableMode('workload')">{{ tr('autovacuumUi.text039') }}</button>
      </div>
      <p v-if="tableMode === 'maintenance'" class="av-table-explanation"><strong>{{ tr('autovacuumUi.text040') }}</strong> {{ tr('autovacuumUi.text041') }} <strong>{{ tr('autovacuumUi.text042') }}</strong> {{ tr('autovacuumUi.text036') }} <strong>{{ tr('autovacuumUi.text043') }}</strong>{{ tr('autovacuumUi.text044') }}</p>
      <p v-else class="av-table-explanation"><strong>{{ tr('autovacuumUi.text040') }}</strong> {{ tr('autovacuumUi.text045') }} <strong>{{ tr('autovacuumUi.text046') }}</strong> {{ tr('autovacuumUi.text047') }}</p>
      <div class="filters"><label>{{ tr('autovacuumUi.text048') }}<input v-model="search" placeholder="public.orders" /></label>
        <label>{{ tr('autovacuumUi.text049') }}<select v-model="mode"><option value="all">{{ tr('autovacuumUi.text050') }}</option><option value="vacuum">{{ tr('autovacuumUi.text051') }}</option><option value="dead">{{ tr('autovacuumUi.text052') }}</option><option value="insert">{{ tr('autovacuumUi.text053') }}</option><option value="analyze">{{ tr('autovacuumUi.text018') }}</option><option value="disabled">{{ tr('autovacuumUi.text054') }}</option></select></label>
        <label>{{ tr('autovacuumUi.text055') }}<select v-model.number="pageSize"><option :value="10">10</option><option :value="25">25</option><option :value="50">50</option></select></label></div>
      <p class="muted av-table-hint">{{ tr('autovacuumUi.text056') }} <strong>{{ tr('autovacuumUi.text057') }}</strong> {{ tr('autovacuumUi.text058') }}</p>
      <div class="table-scroll"><table class="av-overview-table"><thead><tr>
        <th scope="col" :aria-sort="ariaSort('name')"><button type="button" class="table-sort" @click="sortTables('name')">{{ tr('autovacuumUi.text059') }} <span aria-hidden="true">{{ sortSymbol('name') }}</span></button></th>
        <th scope="col" :aria-sort="ariaSort('size')"><button type="button" class="table-sort" @click="sortTables('size')">{{ tr('autovacuumUi.text060') }} <span aria-hidden="true">{{ sortSymbol('size') }}</span></button></th>
        <template v-if="tableMode === 'maintenance'">
          <th scope="col" :aria-sort="ariaSort('vacuum')"><button type="button" class="table-sort" @click="sortTables('vacuum')">{{ tr('autovacuumUi.text061') }} <span aria-hidden="true">{{ sortSymbol('vacuum') }}</span></button></th>
          <th scope="col" :aria-sort="ariaSort('insert')"><button type="button" class="table-sort" @click="sortTables('insert')">{{ tr('autovacuumUi.text062') }} <span aria-hidden="true">{{ sortSymbol('insert') }}</span></button></th>
          <th scope="col" :aria-sort="ariaSort('analyze')"><button type="button" class="table-sort" @click="sortTables('analyze')">{{ tr('autovacuumUi.text063') }} <span aria-hidden="true">{{ sortSymbol('analyze') }}</span></button></th>
          <th scope="col" :aria-sort="ariaSort('status')"><button type="button" class="table-sort" @click="sortTables('status')">{{ tr('autovacuumUi.text064') }} <span aria-hidden="true">{{ sortSymbol('status') }}</span></button></th>
        </template>
        <template v-else>
          <th scope="col" :aria-sort="ariaSort('updated')"><button type="button" class="table-sort" @click="sortTables('updated')">{{ tr('autovacuumUi.text065') }} <span aria-hidden="true">{{ sortSymbol('updated') }}</span></button></th>
          <th scope="col" :aria-sort="ariaSort('inserted')"><button type="button" class="table-sort" @click="sortTables('inserted')">{{ tr('autovacuumUi.text066') }} <span aria-hidden="true">{{ sortSymbol('inserted') }}</span></button></th>
          <th scope="col" :aria-sort="ariaSort('deleted')"><button type="button" class="table-sort" @click="sortTables('deleted')">{{ tr('autovacuumUi.text067') }} <span aria-hidden="true">{{ sortSymbol('deleted') }}</span></button></th>
          <th scope="col" :aria-sort="ariaSort('hot')"><button type="button" class="table-sort" @click="sortTables('hot')">{{ tr('autovacuumUi.text068') }} <span aria-hidden="true">{{ sortSymbol('hot') }}</span></button></th>
          <th scope="col" :aria-sort="ariaSort('dead')"><button type="button" class="table-sort" @click="sortTables('dead')">{{ tr('autovacuumUi.text069') }} <span aria-hidden="true">{{ sortSymbol('dead') }}</span></button></th>
        </template>
        <th scope="col"><span class="sr-only">{{ tr('autovacuumUi.text070') }}</span></th>
      </tr></thead><tbody>
        <tr v-for="t in tablePage.rows" :key="t.relation_oid"><td class="av-relation"><strong>{{ t.schema_name }}.{{ t.relation_name }}</strong><small>OID {{ t.relation_oid }} · {{ t.relation_type }}</small></td>
          <td class="av-cell-num">{{ bytes(t.total_size_bytes) }}</td>
          <template v-if="tableMode === 'maintenance'">
            <td v-for="kind in pressureKinds" :key="kind" class="av-pressure-cell">
              <template v-if="tablePressure(t, kind)">
                <strong>{{ int(tablePressure(t, kind)?.observed) }} / {{ int(tablePressure(t, kind)?.trigger) }}</strong>
                <small>{{ percent(tablePressure(t, kind)?.percent) }} {{ tr('autovacuumUi.ofThreshold') }}</small>
              </template>
              <template v-else><span>—</span><small>{{ kind === 'insert' && t.insert_trigger === null && evidence ? tr('autovacuumUi.conditionNotDefined') : tr('autovacuumUi.noData') }}</small></template>
            </td>
            <td class="av-cell-status">{{ tag(t) }}</td>
          </template>
          <template v-else>
            <td class="av-cell-num">{{ evidence ? int(t.tuples_updated) : '—' }}<small>{{ tr('autovacuumUi.text071') }}</small></td>
            <td class="av-cell-num">{{ evidence ? int(t.tuples_inserted) : '—' }}</td>
            <td class="av-cell-num">{{ evidence ? int(t.tuples_deleted) : '—' }}</td>
            <td class="av-cell-num">{{ evidence ? percent(t.hot_update_pct) : '—' }}<small v-if="evidence">{{ int(t.hot_updates) }} HOT</small></td>
            <td class="av-cell-num">{{ evidence ? int(t.dead_tuples) : '—' }}<small>{{ tr('autovacuumUi.text072') }}</small></td>
          </template>
          <td><button type="button" class="secondary" :disabled="busy" @click="openTable(t)">{{ tr('autovacuumUi.text073') }}</button></td></tr>
        <tr v-if="!tablePage.rows.length"><td :colspan="tableMode === 'maintenance' ? 7 : 8">{{ tr('autovacuumUi.text074') }}</td></tr>
      </tbody></table></div>
      <div class="pager"><button type="button" class="secondary" :disabled="tablePage.page <= 1" @click="page = tablePage.page - 1">{{ tr('autovacuumUi.text075') }}</button>
        <span>{{ tablePage.page }} / {{ tablePage.pages }}</span><button type="button" class="secondary" :disabled="tablePage.page >= tablePage.pages" @click="page = tablePage.page + 1">{{ tr('autovacuumUi.text076') }}</button></div>
    </section>

    <section v-if="state.selectedTableOid !== null" class="panel table-detail">
      <div class="section-head"><div><h2>{{ tr('avFinal.tableAnalysis') }}{{ activeTable?.schema_name }}.{{ activeTable?.relation_name }}</h2>
        <p class="muted">{{ tr('avFinal.detailForSnapshot') }}{{ currentHistoryLabel ?? date(snapshot.overview.measured_at) }}. {{ tr('avFinal.currentLive') }}</p></div>
        <button type="button" class="secondary" :disabled="state.applying" @click="reloadTable">{{ tr('autovacuumUi.text077') }}</button></div>
      <p v-if="state.detailLoading" class="muted">{{ tr('autovacuumUi.text078') }}</p>
      <div v-if="state.detailErrors.length" class="quality"><h3>{{ tr('autovacuumUi.text079') }}</h3><p v-for="(e,i) in state.detailErrors" :key="i">{{ e }}</p></div>
      <nav class="av-detail-tabs" role="tablist" :aria-label="tr('autovacuumUi.detailsAria')">
        <button type="button" role="tab" id="av-tab-state" aria-controls="av-pane-state" :aria-selected="detailTab === 'state'" :class="{ active: detailTab === 'state' }" @click="detailTab = 'state'">{{ tr('autovacuumUi.text064') }}</button>

        <button type="button" role="tab" id="av-tab-advisor" aria-controls="av-pane-advisor" :aria-selected="detailTab === 'advisor'" :class="{ active: detailTab === 'advisor' }" @click="detailTab = 'advisor'">{{ tr('autovacuumUi.text080') }}</button>
        <button type="button" role="tab" id="av-tab-policy" aria-controls="av-pane-policy" :aria-selected="detailTab === 'policy'" :class="{ active: detailTab === 'policy' }" @click="detailTab = 'policy'">{{ tr('autovacuumUi.text081') }}</button>
      </nav>
      <div id="av-pane-state" role="tabpanel" aria-labelledby="av-tab-state" v-show="detailTab === 'state'" class="av-detail-pane">
      <template v-if="state.detail">
        <p class="av-state-intro">{{ tr('autovacuumUi.text082') }} <strong>{{ tr('autovacuumUi.text083') }}</strong>{{ tr('autovacuumUi.text084') }}</p>
        <div class="metrics av-service-dates">
          <article><span>{{ tr('autovacuumUi.text085') }}</span><strong class="smaller">{{ evidence ? date(state.detail.table.last_autovacuum) : '—' }}</strong><small>{{ tr('avFinal.cycles') }}{{ evidence ? int(state.detail.table.autovacuum_count) : '—' }}</small></article>
          <article><span>{{ tr('autovacuumUi.text086') }}</span><strong class="smaller">{{ evidence ? date(state.detail.table.last_autoanalyze) : '—' }}</strong><small>{{ tr('avFinal.cycles') }}{{ evidence ? int(state.detail.table.autoanalyze_count) : '—' }}</small></article>
        </div>
        <div class="indicator-grid" :aria-label="tr('autovacuumUi.indicatorAria')">
          <article v-for="item in mechanismCards" :key="item.key" class="subpanel indicator-card">
            <h3>{{ item.title }}</h3>
            <p>{{ item.metric }}: <strong>{{ item.result.observed === null ? '—' : int(item.result.observed) }}</strong></p>
            <p>{{ tr('autovacuumUi.text087') }} <strong>{{ item.result.trigger === null ? '—' : int(item.result.trigger) }}</strong></p>
            <p>{{ tr('autovacuumUi.text088') }} <strong>{{ percent(item.result.usedPct) }}</strong></p>
            <div v-if="item.result.usedPct !== null" class="av-threshold-bar" role="img" :aria-label="tr('avFinal.thresholdUse', { value: percent(item.result.usedPct) })">
              <span :style="{ width: progressWidth(item.result.usedPct) }"></span>
            </div>
            <p :class="{ warning: item.result.state === 'reached' }"><strong>{{ item.result.label }}</strong><template v-if="item.result.remaining !== null"> · {{ tr('autovacuumUi.remaining') }} {{ int(item.result.remaining) }}</template></p>
            <details class="av-card-explainer"><summary>{{ tr('autovacuumUi.text089') }}</summary><p class="muted">{{ item.help }}</p>
              <p v-if="item.key === 'insert'" class="muted">{{ tr('autovacuumUi.text090') }}</p></details>
            <details v-if="item.interval" class="indicator-window">
              <summary>{{ tr('avFinal.observedChange') }}{{ signed(item.interval.estimate) }}{{ tr('avFinish.for') }}{{ duration(item.interval.elapsedMinutes) }}</summary>
              <p class="muted">{{ tr('avFinal.interval') }}{{ duration(item.interval.elapsedMinutes) }}{{ tr('avFinal.rateDisclaimer') }}</p>
              <p>{{ tr('autovacuumUi.text091') }} <strong>{{ signed(item.interval.estimate) }}</strong> <small class="muted">{{ tr('autovacuumUi.text092') }}</small></p>
              <p v-for="rate in item.interval.rates" :key="rate.label">{{ rate.label }}: <strong>{{ observedRate(rate.perHour) }} {{ tr('autovacuumUi.ratePerHour') }}</strong></p>

            </details>
            <p v-else class="muted">{{ tr('autovacuumUi.text093') }}</p>
          </article>
        </div>
        <p class="muted">{{ tr('autovacuumUi.text094') }}</p>
        <section class="subpanel update-health" :aria-label="tr('avFinal.updateAria')">
          <h3>{{ tr('autovacuumUi.text095') }}</h3>
          <p class="muted">{{ tr('autovacuumUi.text096') }}</p>
          <div class="metrics">
            <article><span>{{ tr('autovacuumUi.text097') }}</span><strong>{{ int(updateTotals?.updates) }}</strong><small>{{ tr('autovacuumUi.text098') }}</small></article>
            <article><span>{{ tr('autovacuumUi.text099') }}</span><strong>{{ int(updateTotals?.hot) }}</strong><small>{{ percent(updateTotals?.hotPct) }}{{ tr('avFinal.hotShare') }}</small></article>
            <article><span>{{ tr('autovacuumUi.text100') }}</span><strong>{{ int(updateTotals?.notHot) }}</strong><small>{{ tr('autovacuumUi.text101') }}</small></article>
            <article><span>{{ tr('autovacuumUi.text102') }}</span><strong>{{ evidence ? int(state.detail.table.dead_tuples) : '—' }}</strong><small>{{ tr('autovacuumUi.text103') }}</small></article>
          </div>
          <template v-if="recentUpdatePeriod">
            <p class="muted">{{ tr('avFinal.lastInterval') }}{{ dateTime(recentUpdatePeriod.olderAt) }} → {{ dateTime(recentUpdatePeriod.newerAt) }} ({{ duration(recentUpdatePeriod.elapsedMinutes) }}). {{ tr('avFinish.thisIs') }}<strong>{{ tr('autovacuumUi.text104') }}</strong>{{ tr('autovacuumUi.text105') }}</p>
            <div class="update-period-stats">
              <span>UPDATE: <strong>{{ signed(recentUpdatePeriod.updates) }}</strong> · {{ observedRate(recentUpdatePeriod.updatesPerHour) }}{{ tr('avFinal.perHour') }}</span>
              <span>{{ tr('autovacuumUi.text106') }} <strong>{{ signed(recentUpdatePeriod.hot) }}</strong> · {{ observedRate(recentUpdatePeriod.hotPerHour) }}{{ tr('avFinal.perHour') }}</span>
              <span>{{ tr('autovacuumUi.text107') }} <strong>{{ percent(recentUpdatePeriod.hotPct) }}</strong></span>
              <span>{{ tr('autovacuumUi.text108') }} <strong>{{ signed(recentUpdatePeriod.notHot) }}</strong></span>
            </div>
            <p v-if="!recentUpdatePeriod.resetVerified" class="warning">{{ tr('autovacuumUi.text109') }}</p>
          </template>
          <p v-else class="muted">{{ tr('autovacuumUi.text110') }}</p>
          <p class="muted">{{ hotGuidance?.message }}</p>
          <details class="technical-note"><summary>{{ tr('autovacuumUi.text111') }}</summary>
            <p>{{ tr('autovacuumUi.text112') }}</p>
            <p>{{ tr('autovacuumUi.text113') }}</p>
          </details>
        </section>
        <details class="technical-note"><summary>{{ tr('autovacuumUi.text114') }}</summary>
          <p><strong>Visibility map:</strong> {{ tr('autovacuumUi.text115') }}</p>
          <p><strong>Freeze/XID:</strong> {{ tr('autovacuumUi.text116') }}</p>
          <p><strong>{{ tr('autovacuumUi.text117') }}</strong> {{ tr('autovacuumUi.text118') }}</p>
          <p>{{ tr('autovacuumUi.text119') }}
            <a :href="avDocsUrl(snapshot.overview.server_version_num, 'routine-vacuuming.html') ?? undefined" target="_blank" rel="noopener noreferrer">VACUUM, ANALYZE, visibility map</a> ·
            <a :href="avDocsUrl(snapshot.overview.server_version_num, 'storage-hot.html') ?? undefined" target="_blank" rel="noopener noreferrer">HOT</a>
          </p>
        </details>
        <details class="technical-note"><summary>{{ tr('autovacuumUi.text120') }}</summary>
          <section v-if="state.detail.table.toast" class="av-toast-summary">
            <h4>TOAST · {{ state.detail.table.toast.relation_name }} <small>{{ tr('avFinal.toastSystem') }}{{ state.detail.table.schema_name }}.{{ state.detail.table.relation_name }}</small></h4>
            <p class="muted">{{ tr('autovacuumUi.text121') }}</p>
            <div class="av-toast-grid">
              <div><span>{{ tr('autovacuumUi.text060') }}</span><strong>{{ bytes(state.detail.table.toast.total_size_bytes) }}</strong></div>
              <div><span>{{ tr('autovacuumUi.text122') }}</span><strong>{{ evidence ? `${int(state.detail.table.toast.dead_tuples)} / ${int(state.detail.table.toast.dead_tuple_trigger)}` : '—' }}</strong><small v-if="evidence">{{ state.detail.table.toast.dead_trigger_reached ? tr('vueRemaining.reached') : tr('vueRemaining.notReached') }}</small></div>
              <div><span>{{ tr('autovacuumUi.text123') }}</span><strong>{{ evidence && state.detail.table.toast.insert_trigger != null && state.detail.table.toast.inserts_since_vacuum != null ? `${int(state.detail.table.toast.inserts_since_vacuum)} / ${int(state.detail.table.toast.insert_trigger)}` : '—' }}</strong><small v-if="evidence">{{ state.detail.table.toast.insert_trigger == null ? tr('vueRemaining.noTrigger') : state.detail.table.toast.insert_trigger_reached ? tr('vueRemaining.reached') : tr('vueRemaining.notReached') }}</small></div>
              <div><span>{{ tr('autovacuumUi.text085') }}</span><strong>{{ date(state.detail.table.toast.last_autovacuum) }}</strong><small>{{ tr('avFinal.toastCycles') }}{{ int(state.detail.table.toast.autovacuum_count) }}</small></div>
            </div>
          </section>
          <p v-else class="muted">{{ tr('autovacuumUi.text124') }}</p>
          <p v-if="state.detail.indexes.error" class="warning">{{ tr('avFinal.indexesUnavailable') }}{{ state.detail.indexes.error }}</p>
          <details v-else class="av-index-details"><summary>{{ tr('avFinal.indexes') }}{{ sortedIndexes.length }}){{ tr('avFinal.sizeSort') }}</summary>
            <p class="muted">{{ tr('avFinal.indexTotal') }}{{ bytes(state.detail.table.indexes_size_bytes) }}{{ tr('avFinal.indexSortExplanation') }}</p>
            <div class="table-scroll av-index-scroll"><table><thead><tr><th>{{ tr('autovacuumUi.text125') }}</th><th>{{ tr('autovacuumUi.text060') }}</th><th>{{ tr('autovacuumUi.text126') }}</th><th>INCLUDE</th><th>{{ tr('autovacuumUi.text127') }}</th></tr></thead><tbody>
              <tr v-for="index in sortedIndexes" :key="index.index_oid">
                <td>{{ index.index_name }}</td><td class="av-cell-num">{{ bytes(index.index_size_bytes) }}</td>
                <td>{{ index.indexed_columns.join(', ') || '—' }}</td><td>{{ index.included_columns.join(', ') || '—' }}</td>
                <td>{{ [index.is_primary ? 'PRIMARY KEY' : '', index.is_unique ? 'UNIQUE' : '', index.predicate ? tr('avFinal.partialIndex') : ''].filter(Boolean).join(' · ') || tr('avFinal.ordinaryIndex') }}</td>
              </tr>
              <tr v-if="!sortedIndexes.length"><td colspan="5">{{ tr('autovacuumUi.text128') }}</td></tr>
            </tbody></table></div>
          </details>
          <p v-if="state.detail.update_statements.error">{{ tr('avFinal.updateUnavailable') }}{{ state.detail.update_statements.error }}</p>
          <template v-else>
            <p>{{ tr('avFinal.possibleUpdates') }}{{ state.detail.update_statements.data?.length ?? 0 }}{{ tr('avFinal.updateEvidence') }}</p>
            <div v-for="item in (state.detail.update_statements.data ?? [])" :key="`${item.queryid}:${item.userid}:${item.dbid}:${item.toplevel}`" class="mini-finding">
              <strong>queryid {{ item.queryid ?? '—' }} · {{ item.evidence_level === 'parsed_qualified' ? tr('avFinal.structuralEvidence') : tr('avFinal.heuristicEvidence') }}</strong>
              <p>Calls {{ item.calls }} · Total execution {{ formatExecutionTime(item.total_exec_time_ms) }}{{ tr('avLast.minutes') }}</p>
              <p class="muted">{{ item.evidence_note }}</p>
              <button v-if="item.queryid && state.connectionId != null" type="button" class="secondary" @click="openPgssEvidence(item.queryid)">{{ tr('autovacuumUi.text129') }}</button>
            </div>
          </template>
          <div v-for="item in state.detail.interpretation.items" :key="item.id" class="mini-finding"><strong>{{ item.title }}</strong><p v-for="(line,i) in item.meaning" :key="i">{{ line }}</p></div>
        </details>
      </template>
      <p v-else-if="!state.detailLoading" class="muted">{{ tr('autovacuumUi.text130') }}</p>
      </div>
      <div id="av-pane-advisor" role="tabpanel" aria-labelledby="av-tab-advisor" v-show="detailTab === 'advisor'" class="av-detail-pane">
      <div v-if="state.advice" class="subpanel"><h3>{{ tr('avFinal.advisorConclusion') }}{{ advisorEvidenceMessage?.title ?? findingLabel(state.advice.status) }}</h3>
        <div v-if="snapshot?.overview.stats_reset == null && snapshot?.overview.track_counts === true" class="quality" role="status">
          <strong>{{ tr('autovacuumUi.text131') }}</strong>
          <p>{{ tr('autovacuumUi.text132') }}</p>
          <p>{{ tr('autovacuumUi.text133') }}</p>
        </div>
        <p v-if="advisorEvidenceMessage">{{ advisorEvidenceMessage.message }}</p>
        <p v-else><strong>{{ state.advice.title }}</strong> — {{ state.advice.summary }}</p>
        <p class="next-action"><strong>{{ tr('autovacuumUi.text134') }}</strong> {{ avAdviceNextStep(state.advice, { savedPoints: state.relationHistory?.summary.history_points, resetUnknown: snapshot?.overview.stats_reset == null }) }}</p>

        <div class="av-requirements" :aria-label="tr('autovacuumUi.advisorConditionsAria')">
          <h4>{{ tr('autovacuumUi.text135') }}</h4>
          <p class="muted">{{ tr('autovacuumUi.text136') }}</p>
          <ul><li v-for="requirement in advisorRequirements" :key="requirement.label">
            <span :class="requirement.met ? 'av-condition-ok' : 'av-condition-missing'">{{ requirement.met ? tr('autovacuumUi.requirementMet') : tr('autovacuumUi.requirementUnknown') }}</span>
            <strong>{{ requirement.label }}</strong><small>{{ requirement.explanation }}</small>
          </li></ul>
          <p v-if="snapshot?.overview.stats_reset == null" class="muted">{{ tr('autovacuumUi.text137') }}</p>
        </div>
        <button type="button" class="secondary" @click="emit('openHistory',state.connectionId,state.selectedTableOid)">{{ tr('autovacuumUi.text138') }}</button>
        <div v-if="state.advice.findings.length" class="advisor-findings">
          <article v-for="f in state.advice.findings" :key="f.id" class="mini-finding"><strong>{{ areaLabel(f.area) }} · {{ findingLabel(f.status) }}</strong><p>{{ f.summary }}</p>
            <details><summary>{{ tr('avFinal.evidenceActions') }}{{ f.title }}</summary>
              <h4>{{ tr('autovacuumUi.text139') }}</h4><ul><li v-for="(line,i) in f.observed" :key="`o-${i}`">{{ line }}</li></ul>
              <h4>{{ tr('autovacuumUi.text140') }}</h4><ul><li v-for="(line,i) in f.next_steps" :key="`n-${i}`">{{ line }}</li></ul>
            </details>
          </article>
        </div>
        <details v-if="adviceLimitations.length" class="technical-note"><summary>{{ tr('avFinal.advisorLimitations') }}{{ adviceLimitations.length }})</summary>
          <ul><li v-for="(line,i) in adviceLimitations" :key="i">{{ line }}</li></ul>
        </details>
      </div>
      <p v-else-if="!state.detailLoading" class="muted">{{ tr('autovacuumUi.text141') }}</p>
      </div>
      <div id="av-pane-policy" role="tabpanel" aria-labelledby="av-tab-policy" v-show="detailTab === 'policy'" class="av-detail-pane">
      <div class="av-policy-guide">
        <h3>{{ tr('autovacuumUi.text142') }}</h3>
        <p>{{ tr('autovacuumUi.text143') }} <strong>{{ tr('autovacuumUi.text144') }}</strong>{{ tr('autovacuumUi.text145') }}</p>
        <p v-if="snapshot?.overview.stats_reset == null && snapshot?.overview.track_counts === true" class="warning">{{ tr('autovacuumUi.text146') }}</p>
        <p v-if="state.candidates && !state.candidates.candidates.length" class="muted">{{ tr('autovacuumUi.text147') }}</p>
        <p v-if="state.applyBlocked" class="warning">{{ tr('autovacuumUi.text148') }}</p>
        <button type="button" class="secondary" @click="detailTab = 'advisor'">{{ tr('autovacuumUi.text149') }}</button>
      </div>
      <section v-if="statsResetUnknown && state.detail" class="subpanel" :aria-label="tr('autovacuumUi.expertPolicyAria')">
        <h3>{{ tr('autovacuumUi.text150') }}</h3>
        <p class="warning">{{ tr('autovacuumUi.text151') }} <strong>{{ tr('autovacuumUi.text152') }}</strong> {{ tr('autovacuumUi.text036') }} <strong>{{ tr('autovacuumUi.text153') }}</strong> {{ tr('autovacuumUi.text154') }}</p>
        <p>{{ tr('autovacuumUi.text155') }}</p>
        <template v-if="unverifiedPolicyOptions.length">
          <p class="muted">{{ tr('autovacuumUi.text156') }} <code>autovacuum_vacuum_scale_factor</code> {{ tr('autovacuumUi.text157') }}</p>
          <div class="table-scroll"><table><thead><tr><th>Scale factor</th><th>{{ tr('autovacuumUi.text158') }}</th><th>{{ tr('autovacuumUi.text070') }}</th></tr></thead><tbody>
            <tr v-for="option in unverifiedPolicyOptions" :key="option.scale_factor">
              <td>{{ avFormatFactor(option.scale_factor) }}</td><td>{{ int(option.resulting_dead_tuple_trigger) }}</td>
              <td><button type="button" class="secondary" :disabled="busy || state.simulating || !liveAllowed" @click="inspectUnverifiedPolicy(option.resulting_dead_tuple_trigger)">{{ tr('autovacuumUi.text159') }}</button></td>
            </tr>
          </tbody></table></div>
        </template>
        <p v-else class="muted">{{ tr('autovacuumUi.text160') }}</p>
        <p class="muted">{{ tr('autovacuumUi.text161') }}</p>
      </section>
      <details class="subpanel expert-simulation" :open="statsResetUnknown"><summary>{{ tr('autovacuumUi.text162') }}</summary>
        <p class="muted">{{ tr('autovacuumUi.text163') }}</p>
        <div class="filters"><label>{{ tr('autovacuumUi.text164') }}<input v-model="targetDeadTuples" type="number" min="1" step="1" :placeholder="tr('avFinal.customThresholdPlaceholder')" /></label>
          <button type="button" class="secondary" :disabled="!state.detail || busy" @click="chooseExample(75)">{{ tr('autovacuumUi.text165') }}</button>
          <button type="button" class="secondary" :disabled="!state.detail || busy" @click="chooseExample(50)">{{ tr('autovacuumUi.text166') }}</button>
          <button type="button" :disabled="!state.detail || state.simulating || busy || !liveAllowed || !targetDeadTuples.trim()" @click="simulateDesired">{{ state.simulating ? tr('autovacuumUi.simulatePending') : tr('autovacuumUi.compareThresholds') }}</button></div>
        <p class="muted">{{ tr('autovacuumUi.text167') }}</p>
        <div v-if="state.simulation" class="assessment">
          <div v-if="simulationComparison" class="av-simulation-impact">
            <p><strong>{{ tr('autovacuumUi.text168') }}</strong></p>
            <p :class="{ warning: simulationComparison.alreadyReached }">
              {{ simulationComparison.thresholdDelta < 0 ? tr('autovacuumUi.thresholdLower') : simulationComparison.thresholdDelta > 0 ? tr('autovacuumUi.thresholdHigher') : tr('autovacuumUi.thresholdSame') }}
              <strong>{{ signed(simulationComparison.thresholdDelta) }}{{ tr('avFinish.estimatedRows') }}{{ percent(Math.abs(simulationComparison.thresholdChangePct)) }})</strong> {{ tr('autovacuumUi.text169') }}
            </p>
            <div class="simulation-grid">
              <article><span>{{ tr('autovacuumUi.text170') }}</span><strong>{{ int(simulationComparison.currentTrigger) }}</strong>
                <small>{{ tr('avFinal.thresholdEstimate') }}{{ int(simulationComparison.deadTuples) }}{{ tr('avFinal.used') }}{{ percent(simulationComparison.currentUsage) }}</small>
                <div class="av-threshold-bar" role="img" :aria-label="tr('avFinal.currentUse', { value: percent(simulationComparison.currentUsage) })"><span :style="{width: progressWidth(simulationComparison.currentUsage)}"></span></div>
                <small>{{ tr('avFinal.remaining') }}{{ int(simulationComparison.currentRemaining) }}</small></article>
              <article><span>{{ tr('autovacuumUi.text171') }}</span><strong>{{ int(simulationComparison.proposedTrigger) }}</strong>
                <small>{{ tr('avLast.sameEstimate') }}{{ tr('avFinal.used') }}{{ percent(simulationComparison.proposedUsage) }}</small>
                <div class="av-threshold-bar" role="img" :aria-label="tr('avFinal.proposedUse', { value: percent(simulationComparison.proposedUsage) })"><span :style="{width: progressWidth(simulationComparison.proposedUsage)}"></span></div>
                <small>{{ simulationComparison.alreadyReached ? tr('avFinal.reachedState') : tr('avFinal.remainingValue', { value: int(simulationComparison.proposedRemaining) }) }}</small></article>
            </div>
            <p class="muted">{{ tr('autovacuumUi.text172') }} <strong>{{ tr('autovacuumUi.text173') }}</strong> {{ tr('autovacuumUi.text174') }}</p>
          </div>
          <p v-if="simulationComparison?.alreadyReached" class="warning"><strong>{{ tr('autovacuumUi.text175') }}</strong> {{ tr('autovacuumUi.text176') }}</p>
          <p v-else-if="simulationComparison" class="muted">{{ tr('autovacuumUi.text177') }}</p>
          <p>{{ tr('autovacuumUi.text178') }} <strong>{{ avFormatFactor(state.simulation.candidate_scale_factor) }}</strong></p>
          <details class="technical-note"><summary>{{ tr('autovacuumUi.text179') }}</summary>
            <p>{{ tr('autovacuumUi.text180') }} <code>{{ state.simulation.candidate_scale_factor ?? tr('autovacuumUi.notCalculated') }}</code></p>
            <p v-for="(note,i) in state.simulation.notes" :key="i" class="muted">{{ note }}</p>
          </details>
        </div>
      </details>
      <details v-if="state.candidates" class="subpanel policy-expert"><summary>{{ tr('avFinal.expertPolicies') }}{{ state.candidates.candidates.length }}{{ tr('avFinal.calculated') }}</summary>
        <p class="muted">{{ state.candidates.summary }}{{ tr('avFinal.notRecommendation') }}</p>
        <details v-if="candidateLimitations.length"><summary>{{ tr('avFinal.calcLimitations') }}{{ candidateLimitations.length }})</summary>
          <ul><li v-for="(lim,i) in candidateLimitations" :key="i">{{ lim }}</li></ul>
        </details>
        <article v-for="c in state.candidates.candidates" :key="c.id" class="candidate"><strong>{{ areaLabel(c.area) }} · {{ c.parameter }}</strong>
          <p>{{ tr('avFinal.scale') }}{{ avFormatFactor(c.current_scale_factor) }} → <strong>{{ avFormatFactor(c.candidate_scale_factor) }}</strong>{{ tr('avFinal.threshold') }}{{ int(c.current_trigger) }} → {{ int(c.candidate_trigger) }}</p>
          <details><summary>{{ tr('autovacuumUi.text181') }}</summary>
            <p v-for="(line,i) in c.rationale" :key="i" class="muted">{{ line }}</p>
            <pre>{{ c.sql_preview }}</pre>
          </details>
          <button type="button" class="danger-button" :disabled="!state.detail || busy || state.applyBlocked || !liveAllowed || !snapshot.overview.track_counts" @click="prepare(c)">{{ tr('autovacuumUi.text182') }}</button>
        </article>
        <p v-if="!state.candidates.candidates.length" class="muted">{{ tr('autovacuumUi.text183') }}</p>
      </details>
      </div>
    </section>

    <section class="panel"><div class="section-head"><div><h2>{{ tr('autovacuumUi.text184') }}</h2><p class="muted">{{ tr('autovacuumUi.text185') }}</p></div></div>
      <details class="technical-note"><summary>{{ tr('avFinal.snapshots') }}{{ state.histories.length }})</summary><div class="table-scroll"><table><thead><tr><th>№</th><th>{{ tr('autovacuumUi.text186') }}</th><th>{{ tr('autovacuumUi.text187') }}</th><th>{{ tr('autovacuumUi.text188') }}</th><th></th></tr></thead><tbody>
        <tr v-for="(s, index) in state.histories" :key="s.id"><td>{{ state.histories.length - index }}</td><td>{{ avSnapshotLabel(s, true) }}</td><td>{{ int(s.table_count) }}</td><td>{{ int(s.unavailable_sections) }}</td>
          <td><button type="button" class="secondary" :disabled="busy" @click="controller.loadSnapshot(s.id)">{{ s.id === state.snapshotId ? tr('avFinal.openState') : tr('avFinal.openStateAlt') }}</button></td></tr>
        <tr v-if="!state.histories.length"><td colspan="5">{{ tr('autovacuumUi.text189') }}</td></tr>
      </tbody></table></div></details>
      <h3>{{ tr('autovacuumUi.text190') }}</h3><div class="filters"><label>{{ tr('autovacuumUi.text191') }}<select v-model.number="state.compareOlderId"><option :value="null">{{ tr('autovacuumUi.text192') }}</option><option v-for="s in state.histories" :key="s.id" :value="s.id">{{ avSnapshotLabel(s) }}</option></select></label>
        <label>{{ tr('autovacuumUi.text193') }}<select v-model.number="state.compareNewerId"><option :value="null">{{ tr('autovacuumUi.text192') }}</option><option v-for="s in state.histories" :key="s.id" :value="s.id">{{ avSnapshotLabel(s) }}</option></select></label>
        <button type="button" :disabled="busy || state.comparing || state.histories.length < 2" @click="controller.compare">{{ state.comparing ? tr('avFinal.comparingState') : tr('avFinal.compareState') }}</button></div>
      <template v-if="shownComparison"><div class="assessment"><strong>{{ avSnapshotLabel(shownComparison.older) }} → {{ avSnapshotLabel(shownComparison.newer) }}</strong> · {{ int(shownComparison.elapsed_seconds / 60) }}{{ tr('avFinish.minutes') }}
          <p>{{ shownComparison.stats_period_continuous ? tr('avFinal.verifiedState') : tr('avFinal.unverifiedState') }}</p>
          <div v-if="comparisonOverview" class="comparison-metrics"><span>{{ tr('autovacuumUi.text194') }} <strong>{{ int(comparisonOverview.total) }}</strong></span>
            <span>{{ tr('autovacuumUi.text195') }} <strong>{{ int(comparisonOverview.increased) }}</strong></span>
            <span>{{ tr('autovacuumUi.text196') }} <strong>{{ int(comparisonOverview.decreased) }}</strong></span>
            <span>{{ tr('autovacuumUi.text197') }} <strong>{{ int(comparisonOverview.unavailable) }}</strong></span></div>
          <p class="muted">{{ tr('autovacuumUi.text198') }}</p>
          <details v-if="shownComparison.limitations.length" class="technical-note"><summary>{{ tr('avFinal.compareLimitations') }}{{ shownComparison.limitations.length }})</summary>
            <p v-for="(line,i) in shownComparison.limitations" :key="i" class="muted">{{ line }}</p>
          </details></div>
        <label class="compare-search">{{ tr('autovacuumUi.text199') }}<input v-model="compareSearch" placeholder="public.orders" /></label>
        <div class="table-scroll"><table><thead><tr><th>{{ tr('autovacuumUi.text059') }}</th><th>{{ tr('autovacuumUi.text200') }}</th><th>INSERT</th><th>UPDATE</th><th>{{ tr('autovacuumUi.text201') }}</th><th>{{ tr('autovacuumUi.text202') }}</th></tr></thead><tbody>
          <tr v-for="r in changes" :key="r.relation_oid"><td>{{ r.newer_name ?? r.older_name ?? r.relation_oid }}</td><td>{{ signed(r.dead_tuples_delta) }}</td><td>{{ signed(r.inserts_delta) }}</td><td>{{ signed(r.updates_delta) }}</td>
            <td>{{ signed(completeMaintenanceDelta(r.vacuum_count_delta, r.autovacuum_count_delta)) }}</td>
            <td>{{ signed(completeMaintenanceDelta(r.analyze_count_delta, r.autoanalyze_count_delta)) }}</td></tr>
          <tr v-if="!changes.length"><td colspan="6">{{ tr('autovacuumUi.text203') }}</td></tr>
        </tbody></table></div><p class="muted">{{ tr('autovacuumUi.text204') }}</p>
      </template>
    </section>

    <section class="panel"><h2>{{ tr('autovacuumUi.text205') }}</h2>
      <div class="columns"><div><h3>VACUUM</h3><p v-if="snapshot.vacuums.error" class="warning">{{ tr('avFinal.unavailable') }}{{ snapshot.vacuums.error }}</p><p v-else-if="!snapshot.vacuums.data?.length">{{ tr('autovacuumUi.text206') }}</p>
        <article v-for="p in snapshot.vacuums.data ?? []" :key="p.pid" class="mini-finding"><strong>{{ p.schema_name }}.{{ p.relation_name }}</strong><p>PID {{ p.pid }} · {{ p.phase }}{{ tr('avFinal.scan') }}{{ percent(p.scan_pct) }}</p></article></div>
        <div><h3>ANALYZE</h3><p v-if="snapshot.analyzes.error" class="warning">{{ tr('avFinal.unavailable') }}{{ snapshot.analyzes.error }}</p><p v-else-if="!snapshot.analyzes.data?.length">{{ tr('autovacuumUi.text206') }}</p>
          <article v-for="p in snapshot.analyzes.data ?? []" :key="p.pid" class="mini-finding"><strong>{{ p.schema_name }}.{{ p.relation_name }}</strong><p>PID {{ p.pid }} · {{ p.phase }}{{ tr('avFinal.sample') }}{{ percent(p.sample_pct) }}</p></article></div></div>
    </section>
  </template>

  <section v-if="state.applyResult" class="panel"><h2>{{ tr('autovacuumUi.text207') }}</h2><p>{{ tr('avFinal.before') }}{{ state.applyResult.verification.before.scale_factor }}{{ tr('avFinal.trigger') }}{{ int(state.applyResult.verification.before.absolute_trigger) }}.</p>
    <p>{{ tr('avFinal.after') }}{{ state.applyResult.verification.after.scale_factor }}{{ tr('avFinal.trigger') }}{{ int(state.applyResult.verification.after.absolute_trigger) }}.</p>
    <p v-if="state.applyResult.warning" class="warning">{{ state.applyResult.warning }}</p>
    <details class="technical-note"><summary>{{ tr('autovacuumUi.text208') }}</summary><p class="warning">{{ tr('autovacuumUi.text209') }}</p>
      <pre v-if="state.applyResult.verification.rollback_sql">{{ state.applyResult.verification.rollback_sql }}</pre><p>{{ state.applyResult.verification.rollback_note }}</p></details>
  </section>

  <details class="panel technical-note"><summary>{{ tr('autovacuumUi.text210') }}</summary>
    <p class="muted">{{ tr('autovacuumUi.text211') }}</p>
    <details><summary>{{ tr('avFinal.readDiagnosis') }}{{ state.reference?.methodology.length ?? 0 }}{{ tr('avFinal.topics') }}</summary>
      <div v-for="step in state.reference?.methodology ?? []" :key="step.id" class="reference-step"><h3>{{ step.title }}</h3><p>{{ step.purpose }}</p>
        <details><summary>{{ tr('autovacuumUi.text212') }}</summary><p v-for="(line,i) in step.how_to_read" :key="i" class="muted">{{ line }}</p></details>
      </div>
    </details>
    <details><summary>{{ tr('avFinal.sqlQueries') }}{{ state.reference?.queries.length ?? 0 }}</summary>
      <p class="muted">{{ tr('autovacuumUi.text213') }}</p>
      <details v-for="q in state.reference?.queries ?? []" :key="q.id"><summary>{{ q.title }}</summary><p>{{ q.purpose }}</p><pre>{{ q.sql }}</pre></details>
    </details>
    <details><summary>{{ tr('avFinal.pgDocs') }}{{ state.reference?.sources.length ?? 0 }}{{ tr('avFinal.sources') }}</summary>
      <p v-for="source in state.reference?.sources ?? []" :key="source.id">{{ source.title }} · {{ source.purpose }}
        <a v-if="avDocsUrl(snapshot?.overview.server_version_num ?? null, source.path)" :href="avDocsUrl(snapshot?.overview.server_version_num ?? null, source.path) ?? undefined" target="_blank" rel="noopener noreferrer">{{ tr('autovacuumUi.text214') }}</a></p>
    </details>
  </details>

  <div v-if="pending" class="confirm-overlay" role="presentation"><section class="confirm-dialog" role="dialog" aria-modal="true" :aria-label="tr('avFinal.confirmAria')">
    <h2>{{ tr('autovacuumUi.text215') }}</h2><p class="warning">{{ tr('autovacuumUi.text216') }}</p>
    <p><strong>{{ pending.target_relation }}</strong> · {{ pending.parameter }}</p>
    <p>{{ tr('avFinal.scale') }}{{ avFormatFactor(pending.current_scale_factor) }} → <strong>{{ avFormatFactor(pending.candidate_scale_factor) }}</strong></p>
    <p class="muted">{{ tr('autovacuumUi.text217') }}</p>
    <p>{{ tr('avFinal.estimatedThreshold') }}{{ int(pending.current_trigger) }} → <strong>{{ int(pending.candidate_trigger) }}</strong></p>
    <p v-for="(line,i) in pending.rationale" :key="i" class="muted">{{ line }}</p>
    <details><summary>{{ tr('autovacuumUi.text218') }}</summary><pre>{{ pending.sql_preview }}</pre></details>
    <label class="acknowledge"><input v-model="understood" type="checkbox" /> {{ tr('autovacuumUi.text219') }}</label>
    <label>{{ tr('autovacuumUi.text220') }} <strong>{{ tr('autovacuumUi.text221') }}</strong><input v-model="confirmText" autocomplete="off" /></label>
    <div class="filters"><button type="button" class="danger-button" :disabled="pendingDisabled" @click="confirmApply">{{ state.applying ? tr('avFinish.applyingState') : tr('avFinish.confirmState') }}</button>
      <button type="button" class="secondary" :disabled="state.applying" @click="cancelApply">{{ tr('autovacuumUi.text222') }}</button></div>
    <p class="muted">{{ tr('autovacuumUi.text223') }}</p>
  </section></div>
</section>
</template>

<style scoped>
.av-simulation-impact{border:1px solid var(--border);border-radius:.55rem;padding:.75rem;margin:.6rem 0}.av-simulation-impact>.simulation-grid{margin:.75rem 0}.av-simulation-impact .av-threshold-bar{width:100%}

.av-toast-summary{border:1px solid var(--border);border-radius:.55rem;padding:.8rem;margin:.8rem 0}.av-toast-summary h4{margin:0 0 .45rem}.av-toast-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(165px,1fr));gap:.6rem}.av-toast-grid>div{display:grid;gap:.35rem;background:var(--surface);padding:.65rem;border-radius:.5rem;border:1px solid var(--border)}.av-toast-grid span,.av-toast-grid small{font-size:.8rem;color:var(--text-secondary)}.av-toast-grid strong{overflow-wrap:anywhere}.av-index-details{margin:.8rem 0}.av-index-scroll{max-height:380px}.av-index-scroll table{min-width:720px}.av-index-scroll td:first-child{overflow-wrap:anywhere;max-width:220px}

.av-evidence-caveat{border-left:3px solid var(--border-strong);background:var(--surface);padding:.6rem .8rem;line-height:1.5}

.av-requirements{border:1px solid var(--border);border-radius:.55rem;padding:.85rem;margin:1rem 0}.av-requirements h4{margin:0 0 .4rem}.av-requirements ul{list-style:none;padding:0;margin:.6rem 0}.av-requirements li{display:grid;grid-template-columns:110px minmax(0,1fr);column-gap:.65rem;row-gap:.2rem;padding:.55rem 0;border-top:1px solid var(--border)}.av-requirements li small{grid-column:2;color:var(--text-secondary);line-height:1.5}.av-condition-ok,.av-condition-missing{font-size:.79rem;font-weight:650}.av-condition-missing{color:var(--message)}@media(max-width:650px){.av-requirements li{grid-template-columns:1fr}.av-requirements li small{grid-column:1}}

.av-policy-guide{border:1px solid var(--border);background:var(--surface-2);border-radius:.65rem;padding:1rem;margin:1rem 0;line-height:1.5}.av-policy-guide h3{margin:0 0 .6rem}.av-policy-guide p{margin:.4rem 0}.av-policy-guide button{margin-top:.7rem}
.av-history-modes{display:flex;gap:.5rem;flex-wrap:wrap;margin:1rem 0}.av-history-modes button{border:1px solid var(--border-strong);border-radius:.5rem;background:var(--surface-2);color:var(--text);padding:.55rem .8rem;cursor:pointer}.av-history-modes button.active{background:var(--button-secondary);font-weight:700}.av-history-modes button:focus-visible{outline:2px solid var(--text);outline-offset:2px}.av-history-pane{min-width:0}.av-history-pane .trend-grid{grid-template-columns:repeat(auto-fit,minmax(min(100%,350px),1fr))}
.av-state-intro{background:var(--surface-2);border-left:3px solid var(--border-strong);padding:.75rem 1rem;line-height:1.5;margin:1rem 0}.av-service-dates{grid-template-columns:repeat(2,minmax(180px,1fr))}.av-card-explainer{font-size:.85rem;color:var(--text-secondary);margin:.6rem 0}.av-card-explainer p{margin:.5rem 0}.indicator-window summary{font-weight:650}.indicator-window[open] summary{margin-bottom:.5rem}@media(max-width:650px){.av-service-dates{grid-template-columns:1fr}}
.av-detail-tabs{display:flex;gap:.45rem;flex-wrap:wrap;border-bottom:1px solid var(--border);padding:.55rem 0;margin:1rem 0}.av-detail-tabs button{border:1px solid transparent;border-radius:.5rem .5rem 0 0;background:transparent;color:var(--text-secondary);padding:.65rem .9rem;cursor:pointer}.av-detail-tabs button.active{background:var(--surface-2);border-color:var(--border-strong);border-bottom-color:var(--surface-2);color:var(--text);font-weight:700}.av-detail-tabs button:focus-visible{outline:2px solid var(--text);outline-offset:2px}.av-detail-pane{min-width:0}.av-detail-pane[hidden]{display:none}

.indicator-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(230px,1fr));gap:.7rem;margin:.8rem 0}.indicator-card{min-width:0;margin:0}.indicator-card h3{font-size:1.05rem}.indicator-window{border-top:1px solid var(--border);padding-top:.7rem;margin-top:.8rem;font-size:.87rem}.indicator-window strong{font-weight:650}.indicator-window small{display:block;line-height:1.5}.table-sort{padding:0;border:0;background:transparent;color:inherit;font:inherit;font-weight:650;text-align:left;cursor:pointer}.table-sort:hover{text-decoration:underline}.table-sort:focus-visible{outline:2px solid var(--text);outline-offset:3px}.table-sort span{color:var(--text-secondary);white-space:nowrap}.av-threshold-bar{height:.5rem;overflow:hidden;background:var(--border);border-radius:1rem;margin:.55rem 0}.av-threshold-bar span{display:block;height:100%;background:#e7a861;border-radius:1rem;max-width:100%}
.av-table-modes{display:flex;flex-wrap:wrap;gap:.45rem;margin:.9rem 0 .5rem}.av-table-modes button{border:1px solid var(--border-strong);background:var(--surface-2);color:var(--text);border-radius:.5rem;padding:.55rem .9rem;font-size:.88rem;cursor:pointer}.av-table-modes button.active{background:var(--button-secondary);border-color:var(--text-secondary);font-weight:650}.av-table-modes button:focus-visible{outline:2px solid var(--text);outline-offset:2px}.av-table-explanation{font-size:.88rem;line-height:1.55;padding:.7rem .8rem;border-left:3px solid var(--border-strong);background:var(--surface-2);margin:.65rem 0 .9rem}.av-table-hint{margin:.65rem 0}.av-overview-table{min-width:900px}.av-pressure-cell{min-width:170px;font-variant-numeric:tabular-nums}.av-pressure-cell strong{display:block;font-size:.9rem;font-weight:650;white-space:nowrap}.av-overview-table .av-cell-num{font-variant-numeric:tabular-nums;white-space:nowrap}.av-overview-table .av-relation{min-width:145px;overflow-wrap:anywhere}.av-overview-table .av-cell-status{min-width:140px}.av-overview-table tbody tr:hover{background:var(--surface-2)}
.av-view{display:grid;gap:1rem;min-width:0}.section-head{display:flex;align-items:flex-start;justify-content:space-between;gap:1rem;flex-wrap:wrap}h1,h2{margin:0}h1 small{font-size:.62em;color:var(--text-secondary);font-weight:500}h3{margin:.25rem 0 .75rem}.muted{color:var(--text-secondary);font-size:.88rem;line-height:1.5}.panel{border:1px solid var(--border-strong);border-radius:.8rem;background:var(--surface);padding:1rem;min-width:0}.controls,.filters{display:flex;flex-wrap:wrap;gap:.75rem;align-items:end}.controls label,.filters label,.confirm-dialog label{display:grid;gap:.4rem;flex:1;min-width:165px}.filters select,.filters input,.controls select,.confirm-dialog input:not([type=checkbox]),.compare-search input{box-sizing:border-box;width:100%;padding:.68rem;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.4rem}.secondary{background:var(--button-secondary);color:var(--button-secondary-text)}.tag{font-size:.8rem;color:var(--text-secondary);padding:.5rem .7rem;border-radius:.5rem;border:1px solid var(--border-strong);background:var(--surface-2)}.metrics{display:grid;grid-template-columns:repeat(auto-fit,minmax(185px,1fr));gap:.65rem;margin:1rem 0}.metrics article{border:1px solid var(--border);background:var(--surface-2);border-radius:.65rem;padding:.85rem;display:grid;gap:.4rem}.metrics span{font-size:.83rem;color:var(--text-secondary)}.metrics strong{font-size:1.3rem}.metrics strong.smaller{font-size:.93rem}.metrics small{font-size:.77rem;color:var(--text-secondary)}.assessment,.quality,.subpanel{padding:1rem;border:1px solid var(--border);border-radius:.65rem;background:var(--surface-2);margin:.75rem 0}.assessment p,.quality p,.subpanel p{line-height:1.5;margin:.3rem 0}.quality ul{padding-left:1.1rem}.warning{color:var(--message)}.message,.empty{padding:.8rem 1rem;border-radius:.55rem;background:var(--surface-2)}.error{color:var(--error)}.table-scroll{overflow:auto;max-height:570px;margin-top:.75rem}.table-scroll table{width:100%;border-collapse:collapse;font-size:.85rem}.table-scroll td,.table-scroll th{padding:.65rem;border-bottom:1px solid var(--border);text-align:left;vertical-align:top}.table-scroll th{position:sticky;top:0;background:var(--surface)}.table-scroll small{display:block;color:var(--text-secondary);font-size:.75rem;margin-top:.3rem}.pager{display:flex;gap:1rem;align-items:center;justify-content:center;margin-top:1rem}.columns{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:1rem}.insights article{padding-left:.65rem;border-left:3px solid var(--border-strong)}.insights article.warning{border-left-color:var(--error)}.severity{display:block;font-size:.75rem;color:var(--text-secondary);margin-bottom:.35rem}.mini-finding,.candidate,.insights article{padding:.7rem 0;border-top:1px solid var(--border);line-height:1.5}.mini-finding p,.candidate p,.insights p{margin:.25rem 0}.mini-finding small{display:block;color:var(--text-secondary);font-size:.83rem}.reference-step{padding:.7rem 0;border-bottom:1px solid var(--border)}.reference-step h3{font-size:1rem}.policy-expert summary{cursor:pointer;font-weight:600}.next-action{padding:.7rem;border-left:3px solid var(--border-strong);background:var(--surface);line-height:1.5}.advisor-findings{display:grid;gap:.5rem}.advisor-findings h4{margin:.5rem 0}.comparison-metrics{display:flex;gap:.6rem 1.25rem;flex-wrap:wrap;margin:.7rem 0}.comparison-metrics span{font-size:.84rem}.trend-grid{margin:1rem 0}.update-health{margin:1rem 0}.update-period-stats{display:flex;flex-wrap:wrap;gap:.6rem 1.5rem;margin:.75rem 0;font-size:.9rem}.update-period-stats span{line-height:1.5}.update-health .warning{font-size:.88rem}.observed-workload{margin:1rem 0;padding:.85rem;border:1px solid var(--border);border-radius:.6rem}.observed-workload h4{margin:0 0 .5rem;font-size:1.05rem}.observed-workload .table-scroll{max-height:20rem}.simulation-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:.75rem;margin:.75rem 0}.simulation-grid article{display:grid;gap:.3rem;padding:.8rem;border:1px solid var(--border);border-radius:.5rem}.simulation-grid span,.simulation-grid small{color:var(--text-secondary);font-size:.85rem}.simulation-grid strong{font-size:1.25rem}.expert-simulation summary{cursor:pointer;font-weight:600}.candidate{padding:1rem 0}.candidate button{margin-top:.6rem}.technical-note summary,details summary{cursor:pointer;font-weight:600}.technical-note{line-height:1.5}.technical-note details{margin:.7rem 0}.technical-note p{font-size:.9rem}pre{max-width:100%;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;padding:1rem;background:var(--surface-2);border-radius:.5rem;font-size:.77rem}.danger-button{background:var(--error);color:white}.compare-search{display:grid;gap:.4rem;margin:1rem 0}.confirm-overlay{position:fixed;inset:0;z-index:2000;background:rgba(0,0,0,.7);display:grid;place-items:center;padding:1rem}.confirm-dialog{background:var(--surface);color:var(--text);border:1px solid var(--border-strong);border-radius:.9rem;padding:1.5rem;width:min(620px,100%);max-height:90vh;overflow:auto;box-sizing:border-box;box-shadow:0 20px 80px rgba(0,0,0,.35)}.confirm-dialog p{line-height:1.5}.confirm-dialog .acknowledge{display:flex;align-items:start;gap:.75rem;margin:1rem 0}.confirm-dialog .acknowledge input{flex:none;margin-top:.25rem}@media(max-width:700px){.metrics{grid-template-columns:repeat(2,minmax(0,1fr))}.panel{padding:.75rem}}
</style>
