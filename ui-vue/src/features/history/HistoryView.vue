<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed, onMounted, reactive, ref, shallowRef, watch } from "vue";
import HistorySortTh from "./HistorySortTh.vue";
import { useHistorySorting } from "./use-history-sorting.js";
import XidHistoryAnalysis from "./XidHistoryAnalysis.vue";
import { xidCsv } from "../xid/history-tools.js";
import PgssHistoryAnalysis from "./PgssHistoryAnalysis.vue";
import AutovacuumHistoryAnalysis from "./AutovacuumHistoryAnalysis.vue";
import SettingsHistoryAnalysis from "./SettingsHistoryAnalysis.vue";
import IoWalHistoryAnalysis from "./IoWalHistoryAnalysis.vue";
import IndexHealthHistoryAnalysis from "./IndexHealthHistoryAnalysis.vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import { SNAPSHOT_COMMANDS } from "../snapshots/schedule-model.js";
import {
  COMPARE_HISTORY_COMMANDS, KIND_LABELS, LIST_HISTORY_COMMANDS, LOAD_HISTORY_COMMANDS,
  SNAPSHOT_KINDS, SNAPSHOT_METRICS, formatHistoryTime, humanValue, sameHistoryTarget,
  sortedSnapshots, type HistoryConnection, type HistorySnapshot, type LiveConnection, type SnapshotKind,
} from "./model.js";

const historySort = useHistorySorting();
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; quickSelection?: QuickConnectionSelection | null; xidNavigationToken?: number; pgssNavigationToken?: number; pgssPreferredConnectionId?: number | null; pgssPreferredQueryid?: string | null; avNavigationToken?: number; avPreferredConnectionId?: number | null; avPreferredRelationOid?: number | null; settingsNavigationToken?: number; settingsPreferredConnectionId?: number | null; ioWalNavigationToken?: number; ioWalPreferredConnectionId?: number | null; indexNavigationToken?: number; indexPreferredConnectionId?: number | null }>();
const emit = defineEmits<{ openSchedules: []; openXidAnalyzer: [connectionId: number | null]; openPgssAnalyzer: [connectionId: number | null, queryid: string | null]; openAvAnalyzer: [connectionId: number | null, relationOid: number | null]; openSettingsCurrent: []; openIoWalAnalyzer: []; openIndexAnalyzer: [connectionId: number | null] }>();
const connections = ref<LiveConnection[]>([]);
const archived = ref<HistoryConnection[]>([]);
const connectionId = ref<number | null>(null);
const kind = ref<SnapshotKind>("xid");
const snapshots = reactive({} as Record<SnapshotKind, HistorySnapshot[]>);
const loadErrors = reactive({} as Record<SnapshotKind, string>);
for (const item of SNAPSHOT_KINDS) { snapshots[item] = []; loadErrors[item] = ""; }
const firstId = ref<number | null>(null);
const secondId = ref<number | null>(null);
const previewId = ref<number | null>(null);
const preview = shallowRef<unknown>(null);
const showPreviewJson = ref(false);
const showComparisonJson = ref(false);
const comparison = shallowRef<Record<string, unknown> | null>(null);
const busy = reactive({ connections: false, snapshots: false, comparing: false, preview: false, collecting: false });
const error = ref(""); const notice = ref("");
const exportingXid = ref(false);
const page = ref(1); const pageSize = 25; const technicalOpen = ref(false);
let token = 0; let connectionsRequest = 0;

const choices = computed(() => {
  const items = new Map<number, string>();
  for (const item of connections.value) items.set(item.id, `${item.name} · ${item.host}:${item.port}/${item.database}`);
  for (const item of archived.value) if (!items.has(item.connection_id)) {
    items.set(item.connection_id, t('history.archived', { name: item.connection_name, id: String(item.connection_id) }));
  }
  return [...items].map(([id, title]) => ({ id, title })).sort((a, b) => a.title.localeCompare(b.title, "ru"));
});
const hasLiveConnection = computed(() => connections.value.some(c => c.id === connectionId.value));
const selectedRows = computed(() => historySort.sorted("snapshots",sortedSnapshots(snapshots[kind.value])));
const displayedRows = computed(() => selectedRows.value.slice((page.value - 1) * pageSize, page.value * pageSize));
const pageCount = computed(() => Math.max(1, Math.ceil(selectedRows.value.length / pageSize)));
const oldSnapshot = computed(() => selectedRows.value.find(item => item.id === firstId.value));
const newSnapshot = computed(() => selectedRows.value.find(item => item.id === secondId.value));
const comparableSelection = computed(() => !!oldSnapshot.value && !!newSnapshot.value &&
  oldSnapshot.value.id !== newSnapshot.value.id && sameHistoryTarget(oldSnapshot.value, newSnapshot.value));
const metrics = computed(() => SNAPSHOT_METRICS[kind.value]);
const comparisonMetrics = computed<[string, unknown][]>(() => {
  const c = comparison.value;
  if (!c) return [];
  const key = kind.value;
  const common: [string, unknown][] = [[t('history.elapsedSeconds'), c["elapsed_seconds"]]];
  switch (key) {
    case "xid": {
      const blockers = (c["blockers"] as Record<string, unknown> | undefined) ?? {};
      const vacuum = (c["vacuums"] as Record<string, unknown> | undefined) ?? {};
      return [...common, [t('history.ageDelta'), c["database_xid_age_delta"]],
        [t('history.xidRate'), (c["xid_rate"] as Record<string, unknown> | null)?.["xid_per_hour"]],
        [t('history.backendXmin'), `${humanValue(blockers["older_backend_count"])} → ${humanValue(blockers["newer_backend_count"])}`],
        [t('history.slots'), `${humanValue(blockers["older_slot_count"])} → ${humanValue(blockers["newer_slot_count"])}`],
        [t('history.vacuumBefore'), `${humanValue(vacuum["older_active_count"])} → ${humanValue(vacuum["newer_active_count"])}`]];
    }
    case "pgss": return [...common, ["Δ dealloc", c["dealloc_delta"]], [t('history.deallocRate'), c["dealloc_per_hour"]],
      [t('history.entryDelta'), c["current_entries_delta"]]];
    case "autovacuum": return [...common, [t('history.sourceLabel'), c["stats_period_continuous"]]];
    case "pg_settings": return [...common, [t('history.changedValues'), c["value_change_count"]],
      [t('history.changedSources'), c["source_change_count"]], [t('history.restartRequired'), c["pending_restart_change_count"]]];
    case "io_wal": return [...common, [t('history.walRate'), c["wal_bytes_per_hour"]],
      [t('history.tempRate'), c["temp_bytes_per_hour"]], [t('history.databaseSizeDelta'), c["database_size_delta_bytes"]]];
    case "index_health": return [...common, [t('history.sourceLabel'), c["stats_period_continuous"]]];
  }
});
interface DiffSection { name: string; fields: string[]; rows: Record<string, unknown>[] }
const comparisonSections = computed<DiffSection[]>(() => {
  const c = comparison.value;
  if (!c) return [];
  const definitions: [string, string, string[]][] = kind.value === "pgss" ? [
    [t('history.roles'), "role_database_changes", ["role_name", "database", "entries_delta", "sample_state"]],
    [t('history.commands'), "command_changes", ["command", "entries_delta", "one_call_delta", "sample_state"]],
    ["CALL targets", "call_target_changes", ["call_tag", "entries_delta", "sample_state"]],
  ] : kind.value === "pg_settings" ? [
    [t('history.settingsChanges'), "changes", ["name", "older_value", "newer_value", "state", "source_changed"]],
  ] : [
    [t('history.objectChanges'), kind.value === "index_health" ? "index_changes" : kind.value === "io_wal" ? "relation_size_changes" : "relation_changes",
      kind.value === "index_health" ? ["index_name", "relation_name", "idx_scan_delta", "size_delta_bytes", "sample_state"] :
      kind.value === "io_wal" ? ["relation_name", "total_size_delta_bytes", "sample_state"] :
      kind.value === "xid" ? ["newer_name", "xid_age_delta", "sample_state"] :
      ["newer_name", "dead_tuples_delta", "inserts_delta", "updates_delta", "sample_state"]],
  ];
  return definitions.map(([name, field, fields]) => ({ name, fields,
    rows: Array.isArray(c[field]) ? (c[field] as Record<string, unknown>[]) : [],
  }));
});
const limitationLines = computed(() => {
  const lines = comparison.value?.["limitations"];
  return Array.isArray(lines) ? [...new Set(lines.filter((x): x is string => typeof x === "string"))] : [];
});

function active(auth: number, seq: number): boolean {
  return props.shell.screen === "workspace" && props.shell.authGeneration === auth && token === seq;
}
function clearSelection(): void {
  previewId.value = null; preview.value = null; comparison.value = null; page.value = 1;
  showPreviewJson.value = false; showComparisonJson.value = false;
  error.value = ""; notice.value = "";
}
function chooseDefaults(): void {
  const rows = selectedRows.value;
  secondId.value = rows[0]?.id ?? null;
  firstId.value = rows[1]?.id ?? null;
}
async function loadConnections(): Promise<void> {
  const auth = props.shell.authGeneration, seq = ++token, call = ++connectionsRequest;
  busy.connections = true; error.value = "";
  try {
    const [live, old] = await Promise.all([
      props.invoke<LiveConnection[]>("list_connections"),
      props.invoke<HistoryConnection[]>("list_history_connections"),
    ]);
    if (!active(auth, seq)) return;
    connections.value = live; archived.value = old;
    if (!choices.value.some(item => item.id === connectionId.value)) connectionId.value = choices.value[0]?.id ?? null;
    else void loadAll();
  } catch (e) { if (active(auth, seq)) error.value = t('history.connectionError', { error: String(e) }); }
  finally { if (props.shell.authGeneration === auth && connectionsRequest === call) busy.connections = false; }
}
async function loadAll(): Promise<void> {
  const id = connectionId.value;
  const auth = props.shell.authGeneration, seq = ++token;
  clearSelection();
  for (const item of SNAPSHOT_KINDS) { snapshots[item] = []; loadErrors[item] = ""; }
  if (id === null) return;
  busy.snapshots = true;
  await Promise.all(SNAPSHOT_KINDS.map(async (item) => {
    try {
      const rows = await props.invoke<HistorySnapshot[]>(LIST_HISTORY_COMMANDS[item], { connectionId: id, limit: 500 });
      if (active(auth, seq)) snapshots[item] = rows;
    } catch (e) { if (active(auth, seq)) loadErrors[item] = String(e); }
  }));
  if (active(auth, seq)) { chooseDefaults(); busy.snapshots = false; }
}
async function collect(): Promise<void> {
  const id = connectionId.value;
  if (id === null || !hasLiveConnection.value) return;
  const auth = props.shell.authGeneration, seq = token;
  busy.collecting = true; error.value = "";
  try {
    const value = await props.invoke<{ history: HistorySnapshot }>(SNAPSHOT_COMMANDS[kind.value], { connectionId: id });
    if (!active(auth, seq)) return;
    await loadAll();
    if (props.shell.authGeneration === auth) notice.value = t('history.snapshotSaved', { id: String(value.history.id) });
  } catch (e) { if (active(auth, seq)) error.value = t('history.collectError', { error: String(e) }); }
  finally { if (props.shell.authGeneration === auth) busy.collecting = false; }
}
async function openSnapshot(id: number): Promise<void> {
  const connection = connectionId.value;
  if (connection === null) return;
  const auth = props.shell.authGeneration, seq = token;
  const source = kind.value;
  busy.preview = true; error.value = ""; preview.value = null;
  try {
    const args = source === "pg_settings" ? { snapshotId: id } : { connectionId: connection, snapshotId: id };
    const value = await props.invoke<unknown>(LOAD_HISTORY_COMMANDS[source], args);
    if (!active(auth, seq) || kind.value !== source) return;
    previewId.value = id; preview.value = value; showPreviewJson.value = false;
  } catch (e) { if (active(auth, seq)) error.value = t('history.snapshotError', { id: String(id), error: String(e) }); }
  finally { if (props.shell.authGeneration === auth) busy.preview = false; }
}
async function exportXidComparison(): Promise<void> {
  if (kind.value !== "xid" || !comparison.value || firstId.value === null || secondId.value === null || exportingXid.value) return;
  const rows = comparison.value["relation_changes"];
  if (!Array.isArray(rows)) return;
  const content = xidCsv(["OID", "Relation before", "Relation after", "XID before", "XID after", "XID delta", "Sample state"],
    rows.map((row: Record<string, unknown>) => [row["relation_oid"], row["older_name"], row["newer_name"], row["older_xid_age"], row["newer_xid_age"], row["xid_age_delta"], row["sample_state"]]
      .map(value => typeof value === "number" || typeof value === "string" ? value : null)));
  exportingXid.value = true;
  try {
    const result = await props.invoke<string>("export_xid_csv", {
      filename: `pgxray-xid-comparison-${firstId.value}-${secondId.value}.csv`,
      content: "\uFEFF" + content,
    });
    notice.value = t('history.csvSaved', { path: String(result) });
  } catch (e) { error.value = t('history.exportError', { error: String(e) }); }
  finally { exportingXid.value = false; }
}
async function compare(): Promise<void> {
  if (!comparableSelection.value || firstId.value === null || secondId.value === null) {
    error.value = t('history.chooseTwo'); return;
  }
  const auth = props.shell.authGeneration, seq = token, source = kind.value;
  busy.comparing = true; error.value = ""; comparison.value = null;
  try {
    const value = await props.invoke<Record<string, unknown>>(COMPARE_HISTORY_COMMANDS[source], {
      firstId: firstId.value, secondId: secondId.value,
    });
    if (active(auth, seq) && kind.value === source) { comparison.value = value; showComparisonJson.value = false; }
  } catch (e) { if (active(auth, seq)) error.value = t('history.comparisonError', { error: String(e) }); }
  finally { if (props.shell.authGeneration === auth) busy.comparing = false; }
}
function reset(): void {
  token++; connectionsRequest++; connections.value = []; archived.value = []; connectionId.value = null;
  for (const item of SNAPSHOT_KINDS) { snapshots[item] = []; loadErrors[item] = ""; }
  busy.connections = false; busy.snapshots = false; busy.comparing = false; busy.preview = false; busy.collecting = false;
  clearSelection();
}
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => choices.value.length, () => (busy.collecting || busy.comparing || busy.preview)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!choices.value.some(connection => connection.id === request.connectionId)) return;
  if (busy.collecting || busy.comparing || busy.preview) return;
  lastShortcutSerial = request.serial;
  if (connectionId.value !== request.connectionId) connectionId.value = request.connectionId;
});
watch(connectionId, (id, old) => { if (id !== old) void loadAll(); });
watch(() => props.xidNavigationToken, (newToken, oldToken) => {
  if (newToken !== oldToken) kind.value = "xid";
});
watch(() => props.avNavigationToken, (newToken, oldToken) => {
  if (newToken !== oldToken) kind.value = "autovacuum";
});
watch(() => props.pgssNavigationToken, (newToken, oldToken) => {
  if (newToken !== oldToken) kind.value = "pgss";
});
watch([choices, () => props.pgssNavigationToken], () => {
  if (!props.pgssNavigationToken || kind.value !== "pgss") return;
  const wanted = props.pgssPreferredConnectionId;
  if (wanted != null && choices.value.some(c => c.id === wanted) && connectionId.value !== wanted) connectionId.value = wanted;
});
watch([choices, () => props.avNavigationToken], () => {
  if (!props.avNavigationToken || kind.value !== "autovacuum") return;
  const wanted = props.avPreferredConnectionId;
  if (wanted != null && choices.value.some(c => c.id === wanted) && connectionId.value !== wanted) connectionId.value = wanted;
});
watch(() => props.settingsNavigationToken, (next, previous) => {
  if (next !== previous) kind.value = "pg_settings";
});
watch([choices, () => props.settingsNavigationToken], () => {
  if (!props.settingsNavigationToken || kind.value !== "pg_settings") return;
  const wanted = props.settingsPreferredConnectionId;
  if (wanted != null && choices.value.some(c => c.id === wanted) && connectionId.value !== wanted) connectionId.value = wanted;
});
watch(() => props.ioWalNavigationToken, (next, previous) => {
  if (next !== previous) kind.value = "io_wal";
});
watch([choices, () => props.ioWalNavigationToken], () => {
  if (!props.ioWalNavigationToken || kind.value !== "io_wal") return;
  const id = props.ioWalPreferredConnectionId;
  if (id != null && choices.value.some(item => item.id === id) && connectionId.value !== id) connectionId.value = id;
});
watch(() => props.indexNavigationToken, (next, previous) => {
  if (next !== previous) kind.value = "index_health";
});
watch([choices, () => props.indexNavigationToken], () => {
  if (!props.indexNavigationToken || kind.value !== "index_health") return;
  const id = props.indexPreferredConnectionId;
  if (id != null && choices.value.some(item => item.id === id) && connectionId.value !== id) connectionId.value = id;
});
watch(kind, () => { clearSelection(); chooseDefaults(); technicalOpen.value = false; });
watch([firstId, secondId], () => { comparison.value = null; showComparisonJson.value = false; });
watch(() => props.shell.authGeneration, () => { reset(); if (props.visible) void loadConnections(); });
watch(() => props.visible, visible => { if (visible && props.shell.screen === "workspace") void loadConnections(); });
onMounted(() => { if (props.visible && props.shell.screen === "workspace") void loadConnections(); });
</script>

<template>
<section class="hist" :aria-label="t('history.pageAria')">
  <header class="hist-head"><div><h1>{{ t('history.title') }}</h1><p class="muted">{{ t('history.subtitle') }}</p></div></header>
  <section class="hist-card hist-intro">
    <div><strong>{{ t('history.introTitle') }}</strong><p>{{ t('history.intro') }}</p></div>
    <button class="secondary" type="button" @click="emit('openSchedules')">{{ t('history.configure') }}</button>
  </section>
  <section class="hist-card hist-controls"><label>{{ t('history.connection') }}<select v-model.number="connectionId"><option :value="null">{{ t('history.select') }}</option><option v-for="item in choices" :key="item.id" :value="item.id">{{ item.title }}</option></select></label><span class="muted" v-if="busy.snapshots">{{ t('history.loadingSnapshots') }}</span><span class="muted" v-else>{{ t('history.limit') }}</span></section>
  <p v-if="error" class="hist-error" role="alert">{{ error }}</p><p v-if="notice" class="hist-notice" role="status">{{ notice }}</p>
  <nav class="hist-tabs" :aria-label="t('history.tabsAria')"><button v-for="source in SNAPSHOT_KINDS" :key="source" type="button" class="secondary" :class="{ selected: kind === source }" @click="kind = source">{{ KIND_LABELS[source] }} <small>({{ snapshots[source].length }})</small></button></nav>
  <section v-if="kind === 'xid'" class="hist-card"><XidHistoryAnalysis :connection-id="connectionId" :snapshots="snapshots.xid" :invoke="invoke" :active="visible && kind === 'xid'" @open-analyzer="emit('openXidAnalyzer', connectionId)" /></section>
  <section v-if="kind === 'pgss'" class="hist-card"><PgssHistoryAnalysis :connection-id="connectionId" :invoke="invoke" :active="visible && kind === 'pgss'" :preferred-queryid="pgssPreferredQueryid ?? null" :navigation-token="pgssNavigationToken" @open-analyzer="emit('openPgssAnalyzer', connectionId, $event)" /></section>
  <section v-if="kind === 'autovacuum'" class="hist-card"><AutovacuumHistoryAnalysis :connection-id="connectionId" :invoke="invoke" :active="visible && kind === 'autovacuum'"
    :navigation-token="avNavigationToken" :preferred-relation-oid="avPreferredRelationOid" @open-analyzer="emit('openAvAnalyzer', connectionId, $event)" /></section>
  <section v-if="kind === 'pg_settings'" class="hist-card"><SettingsHistoryAnalysis :connection-id="connectionId" :invoke="invoke" :active="visible && kind === 'pg_settings'" :navigation-token="settingsNavigationToken" @open-current="emit('openSettingsCurrent')" /></section>
  <section v-if="kind === 'io_wal'" class="hist-card"><IoWalHistoryAnalysis :connection-id="connectionId" :invoke="invoke" :active="visible && kind === 'io_wal'" @open-analyzer="emit('openIoWalAnalyzer')" /></section>
  <section v-if="kind === 'index_health'" class="hist-card"><p v-if="loadErrors.index_health" role="alert" class="hist-error">{{loadErrors.index_health}}</p><IndexHealthHistoryAnalysis :connection-id="connectionId" :snapshots="snapshots.index_health" :invoke="invoke" :active="visible && kind === 'index_health'" @open-analyzer="emit('openIndexAnalyzer', $event)" /></section>
  <section v-if="kind !== 'index_health'" class="hist-card">
    <div class="hist-row"><div><h2>{{ KIND_LABELS[kind] }}</h2><p class="muted">{{ t('history.noSnapshotHint') }}</p></div><button type="button" :disabled="!hasLiveConnection || busy.collecting || busy.snapshots" @click="collect">{{ busy.collecting ? t('history.collecting') : t('history.collect') }}</button></div>
    <p v-if="loadErrors[kind]" class="hist-error" role="alert">{{ loadErrors[kind] }}</p>
    <p v-if="!selectedRows.length && !busy.snapshots" class="muted">{{ t('history.empty') }}</p>
    <button v-if="kind === 'xid' || kind === 'pgss' || kind === 'autovacuum' || kind === 'pg_settings' || kind === 'io_wal'" class="secondary" type="button" @click="technicalOpen = !technicalOpen">{{ technicalOpen ? t('history.hideTechnical') : t('history.showTechnical') }}</button>
    <template v-if="selectedRows.length && (kind !== 'xid' && kind !== 'pgss' && kind !== 'autovacuum' && kind !== 'pg_settings' && kind !== 'io_wal' && kind !== 'index_health' || technicalOpen)">
      <div class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('history.snapshotTime')" :column="'collection_started_at_ms'" :order="historySort.get('snapshots')" @sort="historySort.toggle('snapshots', $event)" /><HistorySortTh v-for="[name, key] in metrics" :key="key" :label="name" :column="key" :order="historySort.get('snapshots')" @sort="historySort.toggle('snapshots', $event)" /><th>{{ t('history.action') }}</th></tr></thead><tbody><tr v-for="s in displayedRows" :key="s.id"><td><strong>#{{ s.id }}</strong><small>{{ formatHistoryTime(s.collection_started_at_ms) }}</small><small>{{ s.host }}:{{ s.port }}/{{ s.database }}</small></td><td v-for="[, key] in metrics" :key="key">{{ humanValue(s[key]) }}</td><td><button class="secondary" type="button" :disabled="busy.preview" @click="openSnapshot(s.id)">{{ t('history.open') }}</button></td></tr></tbody></table></div>
      <div class="hist-pager"><button class="secondary" type="button" :disabled="page <= 1" @click="page--">{{ t('history.previous') }}</button><span>{{ page }} / {{ pageCount }}</span><button class="secondary" type="button" :disabled="page >= pageCount" @click="page++">{{ t('history.next') }}</button></div>
    </template>
  </section>
  <section v-if="selectedRows.length >= 2 && (kind !== 'xid' && kind !== 'pgss' && kind !== 'autovacuum' && kind !== 'pg_settings' && kind !== 'io_wal' && kind !== 'index_health' || technicalOpen)" class="hist-card"><h2>{{ t('history.compareTitle') }}</h2><p class="muted">{{ t('history.compareHint') }}</p><div class="hist-compare-form"><label>{{ t('history.earlier') }}<select v-model.number="firstId"><option v-for="item in selectedRows" :key="item.id" :value="item.id">#{{ item.id }} · {{ formatHistoryTime(item.collection_started_at_ms) }}</option></select></label><label>{{ t('history.later') }}<select v-model.number="secondId"><option v-for="item in selectedRows" :key="item.id" :value="item.id">#{{ item.id }} · {{ formatHistoryTime(item.collection_started_at_ms) }}</option></select></label><button type="button" :disabled="!comparableSelection || busy.comparing" @click="compare">{{ busy.comparing ? t('history.comparing') : t('history.compare') }}</button></div><p v-if="!comparableSelection" class="muted">{{ t('history.different') }}</p>
    <template v-if="comparison">
      <div v-if="kind === 'xid'" class="hist-row"><p class="muted">{{ t('history.xidHint') }}</p>
        <button type="button" class="secondary" :disabled="exportingXid" @click="exportXidComparison">{{ exportingXid ? t('history.exporting') : t('history.exportCsv') }}</button></div>
      <div class="hist-metrics"><div v-for="[name, value] in comparisonMetrics" :key="name"><small>{{ name }}</small><strong>{{ humanValue(value) }}</strong></div></div><div v-for="section in comparisonSections" :key="section.name"><h3>{{ section.name }} · {{ section.rows.length }}</h3><p v-if="!section.rows.length" class="muted">{{ t('history.noDiff') }}</p><div v-else class="hist-scroll"><table><thead><tr><HistorySortTh v-for="field in section.fields" :key="field" :label="field" :column="field" :order="historySort.get('comparison:' + section.name)" @sort="historySort.toggle('comparison:' + section.name, $event)" /></tr></thead><tbody><tr v-for="(row, i) in historySort.sorted('comparison:' + section.name,section.rows).slice(0, 200)" :key="i"><td v-for="field in section.fields" :key="field">{{ humanValue(row[field]) }}</td></tr></tbody></table></div><small v-if="section.rows.length > 200" class="muted">{{ t('history.first200') }}</small></div><details v-if="limitationLines.length"><summary>{{ t('history.limitsCount', { count: String(limitationLines.length) }) }}</summary><ul><li v-for="line in limitationLines" :key="line">{{ line }}</li></ul></details><details @toggle="showComparisonJson = ($event.target as HTMLDetailsElement).open"><summary>{{ t('history.jsonWarning') }}</summary><pre v-if="showComparisonJson">{{ JSON.stringify(comparison, null, 2) }}</pre></details></template>
  </section>
  <section v-if="previewId !== null && preview !== null && (kind !== 'xid' && kind !== 'pgss' && kind !== 'autovacuum' && kind !== 'pg_settings' && kind !== 'io_wal' && kind !== 'index_health' || technicalOpen)" class="hist-card"><h2>{{ t('history.savedSnapshot', { id: String(previewId) }) }}</h2><p class="muted">{{ t('history.previewWarning') }}</p><details @toggle="showPreviewJson = ($event.target as HTMLDetailsElement).open"><summary>{{ t('history.rawJson') }}</summary><pre v-if="showPreviewJson">{{ JSON.stringify(preview, null, 2) }}</pre></details></section>
</section>
</template>

<style scoped>
.hist{display:grid;gap:1rem;min-width:0}.hist-head,.hist-row{display:flex;align-items:start;justify-content:space-between;gap:1rem;flex-wrap:wrap}.hist h1,.hist h2,.hist h3{margin:.2rem 0 .4rem}.hist h2{font-size:1.1rem}.hist p{line-height:1.45}.hist-card{border:1px solid var(--border-strong);border-radius:.65rem;padding:1rem;background:var(--surface);min-width:0}.hist-intro{display:flex;align-items:center;justify-content:space-between;gap:1rem;flex-wrap:wrap}.hist-intro div{flex:1 1 360px}.hist-controls{display:flex;align-items:end;gap:1rem;flex-wrap:wrap}.hist-controls label{flex:1 1 360px}.hist label{display:grid;gap:.4rem}.hist select{min-width:0;width:100%;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.4rem;padding:.6rem}.hist .muted,.hist small{color:var(--text-secondary);font-size:.88rem}.hist-tabs{display:flex;flex-wrap:wrap;gap:.4rem}.hist-tabs button.selected{border-color:var(--accent);background:var(--surface-2);font-weight:700}.hist-tabs small{font-size:.8rem}.hist-scroll{overflow:auto;max-height:560px}.hist table{width:100%;border-collapse:collapse;text-align:left;font-size:.87rem}.hist th,.hist td{padding:.6rem;border-bottom:1px solid var(--border);vertical-align:top}.hist td small{display:block;margin-top:.25rem}.hist-pager{display:flex;align-items:center;justify-content:flex-end;gap:1rem;margin-top:.6rem}.hist-compare-form{display:grid;grid-template-columns:repeat(2,minmax(180px,1fr)) auto;gap:.65rem;align-items:end}.hist-metrics{display:flex;gap:.65rem;flex-wrap:wrap;margin:1rem 0}.hist-metrics>div{background:var(--surface-2);border-radius:.4rem;padding:.65rem;display:grid;gap:.3rem;min-width:130px}.hist-metrics strong{font-size:1.15rem}.hist-error{color:var(--error)}.hist-notice{color:var(--accent)}.hist-chart{width:100%;max-width:860px;height:180px}.hist details{margin-top:.7rem}.hist pre{white-space:pre-wrap;overflow:auto;max-height:500px;word-break:break-word;background:var(--surface-2);padding:1rem;border-radius:.4rem;font-size:.8rem}@media(max-width:800px){.hist-compare-form{grid-template-columns:1fr}}
</style>
