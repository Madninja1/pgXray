<script setup lang="ts">
import { locale, t as tr } from "../i18n/locale-service.js";
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import { createXidController, createXidState, xidDocsUrl, xidPage,
  xidSectionStatus, xidSortRows, xidIsSystemSchema, xidSummaryMatchesConnection,
  type XidSortDirection, type XidCandidate, type XidRelation,
  type XidSection, type XidTrigger } from "./controller.js";
import { xidRelationShare, xidSnapshotFacts, xidTopRelations } from "./presentation.js";
import { xidFilterRelations, xidTriggerState, xidTriggerStateLabel,
  xidRelationPicker, type XidPickerScope,
  type XidRelationKind, type XidRelationHorizon, type XidTriggerState } from "./investigation.js";
import { xidCsv } from "./history-tools.js";

const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; preferredConnectionId?: number | null; quickSelection?: QuickConnectionSelection | null;}>();
const emit = defineEmits<{ openHistory: [] }>();
const state = reactive(createXidState());
const controller = createXidController(props.shell, state, props.invoke);
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.collecting || state.applying || state.loading || state.loadingSnapshot)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.collecting || state.applying || state.loading || state.loadingSnapshot) return;
  lastShortcutSerial = request.serial;
  // Never retarget a running collector/monitor or an in-flight database operation.
  if (state.connectionId !== request.connectionId) controller.selectConnection(request.connectionId);
});

type Tab = "snapshot" | "relation" | "reference";
const tab = ref<Tab>("snapshot");
const tabs = computed<{ id: Tab; title: string }[]>(() => [
  {id:"snapshot",title:tr('xidFinal.tabSnapshot')},{id:"relation",title:"Relation / Policy"},
  {id:"reference",title:tr('xidFinal.tabReference')},
]);
const relationFilter = ref(""); const relationScope = ref<"all" | "user" | "system">("all");
const relationKind = ref<XidRelationKind>("all");
const relationHorizon = ref<XidRelationHorizon>("either");
const relationMinPct = ref(0);
const relationPage = ref(1); const triggerPage = ref(1);
const triggerFilter = ref("");
const triggerScope = ref<"all" | XidTriggerState>("all");
const historyExpanded = ref(false);
const pickerSearch = ref("");
const pickerScope = ref<XidPickerScope>("all");
const csvSaving = ref(false);
const csvExportPath = ref("");
const csvExportError = ref("");
let entryEpoch = 0;
let entryActive = false;
const applyPhrase = ref("");
const tablePageSize = ref(20);
type SortState<K extends string> = { key: K; direction: XidSortDirection };
type RelationSortKey = "name" | "xid_age" | "pct_of_freeze_max" | "mxid_age" |
  "pct_of_multixact_freeze_max" | "total_size_bytes";
type TriggerSortKey = "name" | "estimated_dead_rows" | "estimated_dead_tuple_trigger" |
  "inserts_since_vacuum" | "estimated_insert_trigger" | "reached";
const relationSort = ref<SortState<RelationSortKey>>({ key: "xid_age", direction: "desc" });
const triggerSort = ref<SortState<TriggerSortKey>>({ key: "estimated_dead_rows", direction: "desc" });
function toggled<K extends string>(current: SortState<K>, key: K, initial: XidSortDirection = "desc"): SortState<K> {
  return { key, direction: current.key === key ? current.direction === "asc" ? "desc" : "asc" : initial };
}
function sortAria<K extends string>(current: SortState<K>, key: K): "none" | "ascending" | "descending" {
  return current.key !== key ? "none" : current.direction === "asc" ? "ascending" : "descending";
}
function sortMark<K extends string>(current: SortState<K>, key: K): string {
  return current.key !== key ? "↕" : current.direction === "asc" ? "↑" : "↓";
}
function toggleRelationSort(key: RelationSortKey): void { relationSort.value = toggled(relationSort.value, key, key === "name" ? "asc" : "desc"); relationPage.value = 1; }
function toggleTriggerSort(key: TriggerSortKey): void { triggerSort.value = toggled(triggerSort.value, key, key === "name" ? "asc" : "desc"); triggerPage.value = 1; }
const candidateToApply = ref<XidCandidate | null>(null);
const confirmDialog = ref<HTMLDialogElement | null>(null);
watch(candidateToApply, async candidate => {
  if (!candidate) return;
  await nextTick();
  if (candidateToApply.value === candidate && confirmDialog.value && !confirmDialog.value.open)
    confirmDialog.value.showModal();
});
const selectedConnection = computed(() => state.connections.find(c => c.id === state.connectionId));
const snap = computed(() => state.snapshot);
const facts = computed(() => snap.value ? xidSnapshotFacts(snap.value) : null);
const topXid = computed(() => xidTopRelations(snap.value?.relations.data ?? [], "xid"));
const topMxid = computed(() => xidTopRelations(snap.value?.relations.data ?? [], "mxid"));
const currentRelation = computed(() => snap.value?.relations.data?.find(r => r.relation_oid === state.selectedRelationOid) ?? null);
const currentTrigger = computed(() => snap.value?.autovacuum_triggers.data?.find(t => t.relation_oid === state.selectedRelationOid) ?? null);
const currentVacuum = computed(() => snap.value?.vacuums.data?.find(v => v.relation_oid === state.selectedRelationOid) ?? null);
const matchingSnapshot = computed(() => !!state.summary && !state.historical && xidSummaryMatchesConnection(state.summary, selectedConnection.value));
const pickerRelations = computed(() => xidSortRows(
  xidRelationPicker(snap.value?.relations.data ?? [], pickerScope.value, pickerSearch.value),
  r => `${r.schema_name}.${r.relation_name}`, "asc"));
const pickerOptions = computed(() => {
  const selected = currentRelation.value;
  return selected && !pickerRelations.value.some(r => r.relation_oid === selected.relation_oid)
    ? [selected, ...pickerRelations.value] : pickerRelations.value;
});
const filteredRelations = computed(() => xidFilterRelations(snap.value?.relations.data ?? [], {
  query: relationFilter.value, schemaScope: relationScope.value,
  kind: relationKind.value, horizon: relationHorizon.value, minPct: relationMinPct.value,
}));
const sortedRelations = computed(() => xidSortRows(filteredRelations.value, r => {
  const key = relationSort.value.key;
  return key === "name" ? `${r.schema_name}.${r.relation_name}` : r[key];
}, relationSort.value.direction));
const relationRows = computed(() => xidPage(sortedRelations.value, relationPage.value, tablePageSize.value));
const filteredTriggers = computed(() => (snap.value?.autovacuum_triggers.data ?? []).filter(t =>
  (triggerScope.value === "all" || xidTriggerState(t) === triggerScope.value) &&
  (!triggerFilter.value.trim() || `${t.schema_name}.${t.relation_name} ${t.relation_oid}`
    .toLocaleLowerCase("ru").includes(triggerFilter.value.trim().toLocaleLowerCase("ru")))));
const sortedTriggers = computed(() => xidSortRows(filteredTriggers.value, t => {
  const key = triggerSort.value.key;
  return key === "name" ? `${t.schema_name}.${t.relation_name}` :
    key === "reached" ? Number(t.dead_trigger_reached || t.insert_trigger_reached) : t[key];
}, triggerSort.value.direction));
const triggerRows = computed(() => xidPage(sortedTriggers.value, triggerPage.value, tablePageSize.value));
const visibleRelationHistory = computed(() => historyExpanded.value ? state.relationHistory : state.relationHistory.slice(0, 15));
const sectionProblems = computed(() => {
  const s = snap.value; if (!s) return [];
  return ([
    [tr('xidFinal.secDb'),s.databases],[tr('xidFinal.secRelations'),s.relations],
    [tr('xidFinal.secThresholds'),s.autovacuum_triggers],[tr('xidFinal.secVacuum'),s.vacuums],
    ["Backend XMIN",s.backends],["Replication slots",s.slots],
    ["Prepared transactions",s.prepared],
  ] as const).filter(([,section]) => section.error || section.data === null).map(([name,section]) => `${name}: ${section.error || tr('xidFinal.noData')}`);
});
function n(value: number | null | undefined, digits=0): string {
  return typeof value !== "number" || !Number.isFinite(value) ? "—" :
    new Intl.NumberFormat(locale.value === "ru" ? "ru-RU" : "en-US", {maximumFractionDigits:digits}).format(value);
}
function pct(value: number | null | undefined): string { return value == null ? "—" : `${n(value,1)}%`; }
function agePct(age: number | null | undefined, max: number | null | undefined, reported: number | null | undefined): string {
  if (age != null && max != null && max > 0 && age > 0 && age / max * 100 < 0.05) return locale.value === "ru" ? "<0,1%" : "<0.1%";
  return pct(reported);
}
function time(value: number | string | null | undefined): string {
  if (value == null) return "—";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "—" : date.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US");
}
function size(value: number | null | undefined): string {
  if (value == null) return "—";
  const unit = Math.abs(value) >= 1024**3 ? 1024**3 : Math.abs(value) >= 1024**2 ? 1024**2 : Math.abs(value) >= 1024 ? 1024 : 1;
  return `${n(value/unit,1)} ${unit===1 ? tr('xidFinal.unitB') : unit===1024 ? tr('xidFinal.unitKiB') : unit===1024**2 ? tr('xidFinal.unitMiB') : tr('xidFinal.unitGiB')}`;
}
function name(r: XidRelation): string {
  const full = `${r.schema_name}.${r.relation_name}`;
  return r.relation_type === "t" && r.parent_relation ? `${full} → ${r.parent_schema}.${r.parent_relation}` : full;
}

function sectionMessage(section: XidSection<unknown[]>): string | null {
  const status = xidSectionStatus(section);
  return status === "available" ? null : status === "unavailable"
    ? tr('xidFinal.secUnavailable', { error: section.error ?? tr('xidFinal.sectionMissing') })
    : tr('xidFinal.sectionEmpty');
}
function onConnection(event: Event): void {
  const value = Number((event.target as HTMLSelectElement).value);
  controller.selectConnection(Number.isSafeInteger(value) && value > 0 ? value : null);
  tab.value = "snapshot"; relationFilter.value = ""; triggerFilter.value = "";
  relationScope.value = "all"; relationKind.value = "all"; relationHorizon.value = "either"; relationMinPct.value = 0;
  triggerScope.value = "all"; pickerSearch.value = ""; pickerScope.value = "all";
  relationPage.value = 1; triggerPage.value = 1;
}
function onRelation(event: Event): void {
  const value = Number((event.target as HTMLSelectElement).value);
  void controller.selectRelation(Number.isSafeInteger(value) && value > 0 ? value : null);
}
function openRelation(oid: number): void { tab.value = "relation"; historyExpanded.value = false; void controller.selectRelation(oid); }
function hasRelation(oid: number): boolean { return !!snap.value?.relations.data?.some(r => r.relation_oid === oid); }
async function saveCsv(filename: string, content: string): Promise<void> {
  if (csvSaving.value) return;
  csvSaving.value = true;
  csvExportPath.value = "";
  csvExportError.value = "";
  try {
    const savedPath = await props.invoke<string>("export_xid_csv", {
      filename,
      content: "\uFEFF" + content,
    });
    csvExportPath.value = savedPath;
    state.notice = tr('xidFinal.csvSaved', { path: savedPath });
  } catch (error) {
    csvExportError.value = tr('xidFinal.csvFailed', { error: String(error) });
  } finally {
    csvSaving.value = false;
  }
}
function exportRelations(): void {
  if (!snap.value?.relations.data || !state.summary) return;
  const csv = xidCsv(["schema","relation","OID","type","XID age","XID freeze max","XID pct","MXID age","MXID freeze max","MXID pct","size bytes"],
    sortedRelations.value.map(r => [r.schema_name,r.relation_name,r.relation_oid,r.relation_type,
      r.xid_age,r.effective_freeze_max_age,xidRelationShare(r,"xid"),
      r.mxid_age,r.effective_multixact_freeze_max_age,xidRelationShare(r,"mxid"),r.total_size_bytes]));
  void saveCsv(`pgxray-xid-relations-${state.summary.id}.csv`, csv);
}
async function copySql(sql: string): Promise<void> {
  try { await navigator.clipboard.writeText(sql); state.notice = tr('xidFinal.sqlCopied'); }
  catch { state.error = tr('xidFinal.sqlCopyFailed'); }
}
function openTab(next: Tab): void {
  tab.value = next;
  if (next === "reference") void controller.loadReference();
}
function confirmApply(candidate: XidCandidate): void {
  if (!matchingSnapshot.value || state.applying || state.loadingRelation || state.policy?.candidates.every(c=>c.id !== candidate.id)) return;
  applyPhrase.value = "";
  candidateToApply.value = candidate;
}
function cancelApply(): void { candidateToApply.value = null; applyPhrase.value = ""; }
async function applyConfirmed(): Promise<void> {
  const candidate = candidateToApply.value;
  if (!candidate || applyPhrase.value !== "APPLY") return;
  cancelApply();
  if (await controller.apply(candidate.id)) tab.value = "snapshot";
}
async function bootstrap(): Promise<void> {
  if (!props.visible || props.shell.screen !== "workspace" || !props.shell.currentUsername) {
    entryActive = false; ++entryEpoch; return;
  }
  if (props.shell.busy || entryActive) return;
  entryActive = true;
  const entry = ++entryEpoch;
  // Open the latest saved snapshot for this profile; do not query PostgreSQL on navigation.
  // History navigation supplies its preferred connection before loading the snapshot.
  await controller.refreshConnections(true, props.preferredConnectionId ?? null);
  if (entry !== entryEpoch) return;
}
watch([relationFilter, relationScope, relationKind, relationHorizon, relationMinPct, tablePageSize], () => { relationPage.value = 1; });
watch([() => state.summary?.id, () => state.selectedRelationOid, () => state.connectionId], cancelApply);
watch([triggerFilter, triggerScope, tablePageSize], () => { triggerPage.value = 1; });
watch(() => props.shell.authGeneration, () => { cancelApply(); controller.reset(); entryActive = false; void bootstrap(); });
watch(() => props.shell.screen, screen => { if (screen !== "workspace") { cancelApply(); controller.reset(); } void bootstrap(); });
watch(() => props.shell.busy, () => { void bootstrap(); });
watch(() => props.visible, value => { if (!value) cancelApply(); void bootstrap(); });
onMounted(() => { void bootstrap(); });
onUnmounted(() => { ++entryEpoch; controller.dispose(); });
</script>

<template>
<section class="xid-view" aria-label="XID / Wraparound">
  <header class="title-row">
    <div><h1>XID / Wraparound</h1>
      <p class="muted">{{ tr('xidUi.text000') }}</p></div>
    <span class="tag">{{ tr('xidUi.text001') }}</span>
  </header>
  <div class="toolbar">
    <label>{{ tr('xidUi.text002') }}
      <select :value="state.connectionId ?? ''" :disabled="state.collecting || state.applying || state.loadingSnapshot" @change="onConnection">
        <option value="">{{ tr('xidUi.text003') }}</option>
        <option v-for="c in state.connections" :key="c.id" :value="c.id">{{ c.name }} · {{ c.host }}:{{ c.port }}/{{ c.database }}</option>
      </select>
    </label>
    <button type="button" :disabled="state.connectionId === null || state.collecting || state.applying || state.loading || state.loadingSnapshot" @click="controller.collect">
      {{ state.collecting ? tr('xidFinal.collecting') : tr('xidFinal.collect') }}
    </button>
  </div>
  <p class="muted">{{ tr('xidUi.text004') }}</p>
  <p v-if="state.historical" class="archive-banner" role="status">{{ tr('xidFinish.archive') }}{{ state.summary?.id }} · {{ time(state.summary?.collection_started_at_ms) }}. {{ tr('xidFinal.historical') }} <button class="secondary" type="button" :disabled="state.collecting" @click="controller.collect">{{ tr('xidUi.text005') }}</button></p>
  <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
  <p v-if="state.notice" class="notice" role="status">{{ state.notice }}</p>
  <p v-if="state.loading" class="muted" role="status">{{ tr('xidUi.text006') }}</p>
  <p v-if="state.loadingSnapshot" class="muted" role="status">{{ tr('xidUi.text007') }}</p>
  <nav class="tabs" :aria-label="tr('xidFinal.navTabs')">
    <button v-for="item in tabs"
      :key="item.id" type="button" class="tab" :class="{selected:tab===item.id}" :aria-current="tab===item.id ? 'page' : undefined" @click="openTab(item.id)">{{ item.title }}</button>
    <button type="button" class="tab history-link" @click="emit('openHistory')">{{ tr('xidUi.text008') }}</button>
  </nav>
  <section v-if="tab==='snapshot'">
    <p v-if="!snap" class="muted panel">{{ tr('xidUi.text009') }}</p>
    <template v-else>
      <section class="panel dba-brief" :aria-label="tr('xidFinal.summaryAria')">
        <div class="brief-heading"><div><h2>{{ tr('xidUi.text010') }}</h2>
          <p class="muted">{{ tr('xidFinal.snapHeader') }}{{ state.summary?.id ?? '—' }} · {{ time(state.summary?.collection_started_at_ms) }} · {{ snap.overview.database }}. {{ tr('xidFinal.ageExplanation') }}</p></div>
          <span class="tag">{{ facts?.missingSections.length ?? 0 }}{{ tr('xidFinal.noSections') }}</span></div>
        <div class="metrics brief-metrics">
          <article><span>{{ tr('xidUi.text011') }}</span><strong>{{ pct(facts?.databaseXidPct) }}</strong><small>{{ n(snap.overview.database_xid_age) }} / {{ n(snap.overview.autovacuum_freeze_max_age) }}</small>
            <progress v-if="facts?.databaseXidPct != null" :value="Math.min(facts.databaseXidPct,100)" max="100" :aria-label="tr('xidFinal.xidFreezePct')"></progress></article>
          <article><span>{{ tr('xidUi.text012') }}</span><strong>{{ agePct(snap.overview.database_mxid_age, snap.overview.autovacuum_multixact_freeze_max_age, facts?.databaseMxidPct) }}</strong><small>{{ n(snap.overview.database_mxid_age) }} / {{ n(snap.overview.autovacuum_multixact_freeze_max_age) }}</small>
            <progress v-if="facts?.databaseMxidPct != null" :value="Math.min(facts.databaseMxidPct,100)" max="100" :aria-label="tr('xidFinal.mxidFreezePct')"></progress></article>
          <article><span>{{ tr('xidUi.text013') }}</span><strong>{{ n(facts?.relationCount) }}</strong><small>{{ tr('xidUi.text014') }}</small></article>
          <article><span>VACUUM / backends</span><strong>{{ n(facts?.activeVacuumCount) }} / {{ n(facts?.backendCount) }}</strong><small>{{ tr('xidUi.text015') }}</small></article>
          <article><span>Slots / prepared</span><strong>{{ n(facts?.slotCount) }} / {{ n(facts?.preparedCount) }}</strong><small>{{ tr('xidUi.text016') }}</small></article>
        </div>
        <div class="brief-columns">
          <div><h3>{{ tr('xidUi.text017') }}</h3>
            <p v-if="snap.relations.error || snap.relations.data===null" class="warning">{{ tr('xidFinal.relationUnavailable') }}{{ snap.relations.error ?? tr('xidFinish.noData') }}</p>
            <p v-else-if="!topXid.length" class="muted">{{ tr('xidUi.text018') }}</p>
            <ol v-else class="quick-relations"><li v-for="r in topXid" :key="r.relation_oid"><button type="button" class="inline-link" @click="openRelation(r.relation_oid)">{{ name(r) }}</button><span>{{ pct(xidRelationShare(r,'xid')) }}</span></li></ol>
          </div>
          <div><h3>{{ tr('xidUi.text019') }}</h3>
            <p v-if="snap.relations.error || snap.relations.data===null" class="warning">{{ tr('xidUi.text020') }}</p>
            <p v-else-if="!topMxid.length" class="muted">{{ tr('xidUi.text021') }}</p>
            <ol v-else class="quick-relations"><li v-for="r in topMxid" :key="r.relation_oid"><button type="button" class="inline-link" @click="openRelation(r.relation_oid)">{{ name(r) }}</button><span>{{ agePct(r.mxid_age,r.effective_multixact_freeze_max_age,xidRelationShare(r,'mxid')) }}</span></li></ol>
          </div>
        </div>
        <p class="muted brief-note">{{ tr('xidUi.text022') }}</p>
      </section>
      <details class="panel overview-details"><summary>{{ tr('xidUi.text023') }}</summary><h2>{{ tr('xidUi.text024') }}</h2>
        <p class="muted">Snapshot #{{ state.summary?.id ?? '—' }} · {{ time(state.summary?.collection_started_at_ms) }} · {{ snap.overview.database }} (OID {{ snap.overview.database_oid }}) · PG {{ snap.overview.server_version }}</p>
        <p v-if="!matchingSnapshot" class="warning">{{ tr('xidUi.text025') }}</p>
        <div class="metrics">
          <article><span>{{ tr('xidUi.text026') }}</span><strong>{{ snap.overview.current_xid }}</strong></article>
          <article><span>{{ tr('xidUi.text027') }}</span><strong>{{ n(snap.overview.database_xid_age) }}</strong></article>
          <article><span>{{ tr('xidUi.text028') }}</span><strong>{{ n(snap.overview.database_mxid_age) }}</strong></article>
          <article><span>{{ tr('xidUi.text029') }}</span><strong>{{ n(snap.overview.autovacuum_freeze_max_age) }}</strong></article>
          <article><span>{{ tr('xidUi.text030') }}</span><strong>{{ n(snap.overview.autovacuum_multixact_freeze_max_age) }}</strong></article>
          <article><span>Autovacuum</span><strong>{{ snap.overview.autovacuum ? 'on' : 'off' }}</strong></article>
          <article><span>vacuum_freeze_table_age</span><strong>{{ n(snap.overview.vacuum_freeze_table_age) }}</strong></article>
          <article><span>vacuum_freeze_min_age</span><strong>{{ n(snap.overview.vacuum_freeze_min_age) }}</strong></article>
          <article><span>Autovacuum max workers</span><strong>{{ n(snap.overview.autovacuum_max_workers) }}</strong></article>
          <article><span>{{ tr('xidUi.text031') }}</span><strong>{{ time(snap.overview.stats_reset) }}</strong></article>
        </div>
      </details>
      <nav class="snapshot-links" :aria-label="tr('xidFinish.navigation')">
        <span class="muted">{{ tr('xidUi.text032') }}</span>
        <a href="#xid-findings">{{ tr('xidFinal.findings') }}{{ snap.interpretation.items.length }})</a>
        <a href="#xid-relations">{{ tr('xidFinal.relations') }}{{ snap.relations.data?.length ?? '—' }})</a>
        <a href="#xid-triggers">Autovacuum</a>
        <a href="#xid-blockers">Blockers</a>
      </nav>
      <details v-if="sectionProblems.length || snap.interpretation.limitations.length" class="panel quality-details" :open="sectionProblems.length > 0">
        <summary>{{ tr('xidFinal.sectionsLabel') }}{{ sectionProblems.length }}{{ tr('xidFinal.noSections') }} · {{ snap.interpretation.limitations.length }}{{ tr('xidFinal.limitationsUnit') }}</summary>
        <ul class="warnings"><li v-for="(msg,i) in [...sectionProblems,...snap.interpretation.limitations]" :key="i">{{ msg }}</li></ul>
      </details>
      <section id="xid-findings" class="panel"><h2>{{ tr('xidUi.text033') }} <span class="section-count">{{ snap.interpretation.items.length }}</span></h2>
        <p class="muted">{{ tr('xidUi.text034') }}</p>
        <p v-if="!snap.interpretation.items.length" class="muted">{{ tr('xidUi.text035') }}</p>
        <details v-for="item in snap.interpretation.items" :key="item.id" class="finding">
          <summary>{{ item.title }}</summary>
          <h3>{{ tr('xidUi.text036') }}</h3><ul><li v-for="(s,i) in item.observed" :key="i">{{ s }}</li></ul>
          <h3>{{ tr('xidUi.text037') }}</h3><ul><li v-for="(s,i) in item.meaning" :key="i">{{ s }}</li></ul>
          <h3>{{ tr('xidUi.text038') }}</h3><ul><li v-for="(s,i) in item.next_steps" :key="i">{{ s }}</li></ul>
          <button v-if="item.id==='oldest-relation' && snap.relations.data?.[0]" class="secondary" type="button" @click="openRelation(snap.relations.data[0].relation_oid)">{{ tr('xidUi.text039') }}</button>
        </details>
      </section>
      <section class="panel"><h2>{{ tr('xidUi.text040') }}</h2>
        <p v-if="sectionMessage(snap.databases)" :class="snap.databases.error ? 'warning':'muted'">{{ sectionMessage(snap.databases) }}</p>
        <div v-else class="scroll"><table><thead><tr><th>{{ tr('xidUi.text041') }}</th><th>XID age</th><th>% freeze max</th><th>MXID age</th><th>% MXID max</th></tr></thead>
          <tbody><tr v-for="db in snap.databases.data ?? []" :key="db.database_oid"><td>{{ db.database }} · {{ db.database_oid }}</td><td>{{ n(db.xid_age) }}</td><td>{{ pct(db.pct_of_global_freeze_max) }}</td><td>{{ n(db.mxid_age) }}</td><td>{{ agePct(db.mxid_age, snap.overview.autovacuum_multixact_freeze_max_age, db.pct_of_global_multixact_freeze_max) }}</td></tr></tbody></table></div>
      </section>
      <section id="xid-relations" class="panel"><h2>{{ tr('xidUi.text042') }}</h2>
        <p v-if="sectionMessage(snap.relations)" :class="snap.relations.error ? 'warning':'muted'">{{ sectionMessage(snap.relations) }}</p>
        <template v-else>
          <div class="toolbar table-tools">
            <label>{{ tr('xidUi.text043') }} <input v-model="relationFilter" :placeholder="tr('xidFinal.relationPlaceholder')"></label>
            <label>{{ tr('xidUi.text044') }} <select v-model="relationScope"><option value="all">{{ tr('xidUi.text045') }}</option><option value="user">{{ tr('xidUi.text046') }}</option><option value="system">{{ tr('xidUi.text047') }}</option></select></label>
            <label>{{ tr('xidUi.text048') }} <select v-model="relationKind"><option value="all">{{ tr('xidUi.text049') }}</option><option value="toast">{{ tr('xidUi.text050') }}</option><option value="non-toast">{{ tr('xidUi.text051') }}</option></select></label>
            <label>{{ tr('xidUi.text052') }} <select v-model="relationHorizon"><option value="either">{{ tr('xidUi.text053') }}</option><option value="xid">{{ tr('xidUi.text054') }}</option><option value="mxid">{{ tr('xidUi.text055') }}</option></select></label>
            <label>{{ tr('xidUi.text056') }} <select v-model.number="relationMinPct"><option :value="0">{{ tr('xidUi.text057') }}</option><option :value="25">25%</option><option :value="50">50%</option><option :value="75">75%</option><option :value="90">90%</option></select></label>
            <label>{{ tr('xidUi.text058') }} <select v-model.number="tablePageSize"><option v-for="s in [20,50,100]" :key="s" :value="s">{{ s }}</option></select></label>
          </div>
          <div class="table-actions"><p class="muted table-hint">{{ tr('xidFinal.matches') }}{{ relationRows.total }}{{ tr('xidFinal.of') }}{{ snap.relations.data?.length ?? 0 }}{{ tr('xidFinal.shown') }}{{ relationRows.rows.length }}{{ tr('xidFinal.allFiltered') }}</p>
            <button type="button" class="secondary" :disabled="!relationRows.total || csvSaving" @click="exportRelations">{{ csvSaving ? tr('xidFinish.csvSaving') : tr('xidFinish.csvExport', { count: String(relationRows.total) }) }}</button></div>
          <p class="muted">{{ tr('xidUi.text059') }}</p>
          <p v-if="csvExportPath" class="notice" role="status">{{ tr('xidUi.text060') }} <code>{{ csvExportPath }}</code></p>
          <p v-if="csvExportError" class="error" role="alert">{{ csvExportError }}</p>
          <div class="scroll"><table><thead><tr>
            <th :aria-sort="sortAria(relationSort,'name')"><button class="sort-heading" type="button" @click="toggleRelationSort('name')">{{ tr('xidUi.text061') }} <span aria-hidden="true">{{ sortMark(relationSort,'name') }}</span></button></th>
            <th :aria-sort="sortAria(relationSort,'xid_age')"><button class="sort-heading" type="button" @click="toggleRelationSort('xid_age')">XID age <span aria-hidden="true">{{ sortMark(relationSort,'xid_age') }}</span></button></th>
            <th :aria-sort="sortAria(relationSort,'pct_of_freeze_max')"><button class="sort-heading" type="button" @click="toggleRelationSort('pct_of_freeze_max')">% freeze <span aria-hidden="true">{{ sortMark(relationSort,'pct_of_freeze_max') }}</span></button></th>
            <th :aria-sort="sortAria(relationSort,'mxid_age')"><button class="sort-heading" type="button" @click="toggleRelationSort('mxid_age')">MXID age <span aria-hidden="true">{{ sortMark(relationSort,'mxid_age') }}</span></button></th>
            <th :aria-sort="sortAria(relationSort,'pct_of_multixact_freeze_max')"><button class="sort-heading" type="button" @click="toggleRelationSort('pct_of_multixact_freeze_max')">% MXID freeze <span aria-hidden="true">{{ sortMark(relationSort,'pct_of_multixact_freeze_max') }}</span></button></th>
            <th :aria-sort="sortAria(relationSort,'total_size_bytes')"><button class="sort-heading" type="button" @click="toggleRelationSort('total_size_bytes')">{{ tr('xidUi.text062') }} <span aria-hidden="true">{{ sortMark(relationSort,'total_size_bytes') }}</span></button></th><th>{{ tr('xidUi.text063') }}</th></tr></thead>
            <tbody><tr v-for="r in relationRows.rows" :key="r.relation_oid"><td>{{ name(r) }} · OID {{ r.relation_oid }}</td><td>{{ n(r.xid_age) }}</td><td>{{ pct(r.pct_of_freeze_max) }}</td><td>{{ n(r.mxid_age) }}</td><td>{{ agePct(r.mxid_age, r.effective_multixact_freeze_max_age, r.pct_of_multixact_freeze_max) }}</td><td>{{ size(r.total_size_bytes) }}</td><td><button class="secondary" type="button" @click="openRelation(r.relation_oid)">{{ tr('xidUi.text064') }}</button></td></tr></tbody></table></div>
          <p v-if="!relationRows.total" class="muted">{{ tr('xidUi.text065') }}</p>
          <div class="pager"><button class="secondary" :disabled="relationRows.page<=1" @click="relationPage--">{{ tr('xidUi.text066') }}</button><span>{{ relationRows.page }} / {{ relationRows.pages }}</span><button class="secondary" :disabled="relationRows.page>=relationRows.pages" @click="relationPage++">{{ tr('xidUi.text067') }}</button></div>
        </template>
      </section>
      <section id="xid-triggers" class="panel"><h2>{{ tr('xidUi.text068') }}</h2>
        <p v-if="sectionMessage(snap.autovacuum_triggers)" :class="snap.autovacuum_triggers.error ? 'warning':'muted'">{{ sectionMessage(snap.autovacuum_triggers) }}</p>
        <template v-else>
          <div class="toolbar table-tools"><label>{{ tr('xidUi.text043') }} <input v-model="triggerFilter" :placeholder="tr('xidFinal.relationPlaceholder')"></label>
            <label>{{ tr('xidUi.text069') }} <select v-model="triggerScope"><option value="all">{{ tr('xidUi.text070') }}</option><option value="reached">{{ tr('xidUi.text071') }}</option><option value="below">{{ tr('xidUi.text072') }}</option><option value="unknown">{{ tr('xidUi.text073') }}</option><option value="disabled">{{ tr('xidUi.text074') }}</option></select></label></div>
          <p class="muted table-hint">{{ tr('xidFinal.matches') }}{{ triggerRows.total }}{{ tr('xidFinal.of') }}{{ snap.autovacuum_triggers.data?.length ?? 0 }}{{ tr('xidFinal.sortingBeforePagination') }}</p>
          <div class="scroll"><table><thead><tr>
            <th :aria-sort="sortAria(triggerSort,'name')"><button class="sort-heading" type="button" @click="toggleTriggerSort('name')">{{ tr('xidUi.text061') }} <span aria-hidden="true">{{ sortMark(triggerSort,'name') }}</span></button></th>
            <th :aria-sort="sortAria(triggerSort,'estimated_dead_rows')"><button class="sort-heading" type="button" @click="toggleTriggerSort('estimated_dead_rows')">Dead tuples <span aria-hidden="true">{{ sortMark(triggerSort,'estimated_dead_rows') }}</span></button></th>
            <th :aria-sort="sortAria(triggerSort,'estimated_dead_tuple_trigger')"><button class="sort-heading" type="button" @click="toggleTriggerSort('estimated_dead_tuple_trigger')">{{ tr('xidUi.text075') }} <span aria-hidden="true">{{ sortMark(triggerSort,'estimated_dead_tuple_trigger') }}</span></button></th>
            <th :aria-sort="sortAria(triggerSort,'inserts_since_vacuum')"><button class="sort-heading" type="button" @click="toggleTriggerSort('inserts_since_vacuum')">INSERT since VACUUM <span aria-hidden="true">{{ sortMark(triggerSort,'inserts_since_vacuum') }}</span></button></th>
            <th :aria-sort="sortAria(triggerSort,'estimated_insert_trigger')"><button class="sort-heading" type="button" @click="toggleTriggerSort('estimated_insert_trigger')">{{ tr('xidUi.text076') }} <span aria-hidden="true">{{ sortMark(triggerSort,'estimated_insert_trigger') }}</span></button></th>
            <th :aria-sort="sortAria(triggerSort,'reached')"><button class="sort-heading" type="button" @click="toggleTriggerSort('reached')">{{ tr('xidUi.text071') }} <span aria-hidden="true">{{ sortMark(triggerSort,'reached') }}</span></button></th><th>{{ tr('xidUi.text063') }}</th></tr></thead>
            <tbody><tr v-for="t in triggerRows.rows" :key="t.relation_oid"><td>{{ t.schema_name }}.{{ t.relation_name }}</td><td>{{ n(t.estimated_dead_rows) }}</td><td>{{ n(t.estimated_dead_tuple_trigger) }}</td><td>{{ n(t.inserts_since_vacuum) }}</td><td>{{ n(t.estimated_insert_trigger) }}</td><td>{{ xidTriggerStateLabel(t) }}</td><td><button class="secondary" type="button" :disabled="!hasRelation(t.relation_oid)" @click="openRelation(t.relation_oid)">{{ tr('xidUi.text064') }}</button></td></tr></tbody></table></div>
          <p v-if="!triggerRows.total" class="muted">{{ tr('xidUi.text077') }}</p>
          <div class="pager"><button class="secondary" :disabled="triggerRows.page<=1" @click="triggerPage--">{{ tr('xidUi.text066') }}</button><span>{{ triggerRows.page }} / {{ triggerRows.pages }}</span><button class="secondary" :disabled="triggerRows.page>=triggerRows.pages" @click="triggerPage++">{{ tr('xidUi.text067') }}</button></div>
        </template>
      </section>
      <section class="panel"><h2>{{ tr('xidUi.text078') }}</h2>
        <p v-if="sectionMessage(snap.vacuums)" :class="snap.vacuums.error ? 'warning':'muted'">{{ sectionMessage(snap.vacuums) }}</p>
        <div v-else class="scroll"><table><thead><tr><th>PID</th><th>Relation</th><th>{{ tr('xidUi.text079') }}</th><th>{{ tr('xidUi.text080') }}</th><th>Wraparound detection</th><th>{{ tr('xidUi.text081') }}</th></tr></thead>
          <tbody><tr v-for="v in snap.vacuums.data ?? []" :key="v.pid"><td>{{ v.pid }}</td><td><button v-if="hasRelation(v.relation_oid)" type="button" class="inline-link" @click="openRelation(v.relation_oid)">{{ v.schema_name }}.{{ v.relation_name }}</button><span v-else>{{ v.schema_name }}.{{ v.relation_name }}</span></td><td>{{ v.phase }} · {{ v.backend_type ?? '—' }}</td><td>{{ pct(v.scan_pct) }} ({{ n(v.heap_blks_scanned) }}/{{ n(v.heap_blks_total) }}{{ tr('vueRemaining.blocks') }})</td><td>{{ v.is_wraparound_vacuum ? tr('vueRemaining.confirmed') : v.wraparound_detection === 'unknown' ? tr('vueRemaining.unknown') : (v.wraparound_detection ?? tr('vueRemaining.notDetected')) }}</td><td>{{ v.wait_event_type ?? '—' }} / {{ v.wait_event ?? '—' }}</td></tr></tbody></table></div>
      </section>
      <section id="xid-blockers" class="panel"><h2>{{ tr('xidUi.text082') }}</h2>
        <h3>Backend XMIN</h3><p v-if="sectionMessage(snap.backends)" :class="snap.backends.error ? 'warning':'muted'">{{ sectionMessage(snap.backends) }}</p>
        <div v-else class="scroll"><table><thead><tr><th>PID</th><th>{{ tr('xidUi.text083') }}</th><th>Application</th><th>xmin age</th><th>{{ tr('xidUi.text084') }}</th><th>{{ tr('xidUi.text085') }}</th></tr></thead>
          <tbody><tr v-for="b in snap.backends.data ?? []" :key="b.pid"><td>{{ b.pid }}</td><td>{{ b.database ?? '—' }} / {{ b.username ?? '—' }}</td><td>{{ b.application_name ?? '—' }}</td><td>{{ n(b.xmin_age) }}</td><td>{{ n(b.transaction_age_seconds) }}</td><td>{{ b.state ?? '—' }} / {{ b.wait_event_type ?? '—' }}:{{ b.wait_event ?? '—' }}</td></tr></tbody></table></div>
        <h3>Replication slots</h3><p v-if="sectionMessage(snap.slots)" :class="snap.slots.error ? 'warning':'muted'">{{ sectionMessage(snap.slots) }}</p>
        <div v-else class="scroll"><table><thead><tr><th>Slot</th><th>{{ tr('xidUi.text048') }}</th><th>{{ tr('xidUi.text086') }}</th><th>xmin age</th><th>catalog xmin age</th></tr></thead>
          <tbody><tr v-for="s in snap.slots.data ?? []" :key="s.slot_name"><td>{{ s.slot_name }}</td><td>{{ s.slot_type }}</td><td>{{ s.active ? tr('xidFinal.yes'):tr('xidFinal.no') }}</td><td>{{ n(s.xmin_age) }}</td><td>{{ n(s.catalog_xmin_age) }}</td></tr></tbody></table></div>
        <h3>Prepared transactions</h3>
        <p v-if="snap.prepared.error || !snap.prepared.data" class="warning">{{ tr('xidFinish.prep') }}{{ snap.prepared.error ?? tr('xidFinal.sectionMissing') }}</p>
        <template v-else-if="snap.prepared.data"><p>{{ tr('xidFinal.count') }}{{ n(snap.prepared.data.count) }}{{ tr('xidFinish.oldest') }}{{ n(snap.prepared.data.oldest_age_seconds) }}{{ tr('xidFinal.seconds') }}</p>
          <div v-if="snap.prepared.data.examples?.length" class="scroll"><table><thead><tr><th>GID</th><th>XID</th><th>{{ tr('xidUi.text087') }}</th><th>{{ tr('xidUi.text088') }}</th><th>{{ tr('xidUi.text089') }}</th><th>{{ tr('xidUi.text090') }}</th></tr></thead>
            <tbody><tr v-for="p in snap.prepared.data.examples" :key="p.gid"><td>{{ p.gid }}</td><td>{{ p.xid }}</td><td>{{ p.database }}</td><td>{{ p.owner }}</td><td>{{ n(p.age_seconds) }}</td><td>{{ time(p.prepared_at) }}</td></tr></tbody></table></div>
        </template>
      </section>
    </template>
  </section>
  <section v-else-if="tab==='relation'" class="panel">
    <h2>{{ tr('xidUi.text091') }}</h2>
    <p v-if="!snap?.relations.data" class="warning">{{ tr('xidUi.text092') }}</p>
    <template v-else>
      <div class="toolbar relation-picker-tools">
        <label>{{ tr('xidUi.text043') }} <input v-model="pickerSearch" type="search" :placeholder="tr('xidFinal.namePlaceholder')"></label>
        <label>{{ tr('xidUi.text093') }} <select v-model="pickerScope">
          <option value="all">{{ tr('xidUi.text070') }}</option><option value="user">{{ tr('xidUi.text046') }}</option>
          <option value="system">{{ tr('xidUi.text047') }}</option><option value="toast">TOAST</option>
          <option value="other">{{ tr('xidUi.text094') }}</option>
        </select></label>
        <label>{{ tr('xidUi.text061') }}
        <select :value="state.selectedRelationOid ?? ''" :disabled="state.applying" @change="onRelation">
          <option value="">{{ tr('xidUi.text095') }}</option>
          <option v-for="r in pickerOptions" :key="r.relation_oid" :value="r.relation_oid">{{ name(r) }} · {{ r.relation_oid }}</option>
        </select></label></div>
      <p class="muted">{{ tr('xidFinal.suitable') }}{{ pickerRelations.length }}{{ tr('xidFinal.of') }}{{ snap.relations.data?.length ?? 0 }}{{ tr('xidFinal.pickerKeeps') }}</p>
      <p v-if="!currentRelation" class="muted">{{ tr('xidUi.text096') }}</p>
      <template v-else>
        <div class="metrics">
          <article><span>XID age / freeze max</span><strong>{{ n(currentRelation.xid_age) }} / {{ n(currentRelation.effective_freeze_max_age) }}</strong><small>{{ pct(currentRelation.pct_of_freeze_max) }}</small></article>
          <article><span>MXID age / freeze max</span><strong>{{ n(currentRelation.mxid_age) }} / {{ n(currentRelation.effective_multixact_freeze_max_age) }}</strong><small>{{ agePct(currentRelation.mxid_age, currentRelation.effective_multixact_freeze_max_age, currentRelation.pct_of_multixact_freeze_max) }}</small></article>
          <article><span>{{ tr('xidUi.text062') }}</span><strong>{{ size(currentRelation.total_size_bytes) }}</strong></article>
          <article><span>VACUUM / Autovacuum</span><strong>{{ n(currentRelation.vacuum_count) }} / {{ n(currentRelation.autovacuum_count) }}</strong><small>{{ tr('xidUi.text097') }}</small></article>
          <article><span>Last VACUUM</span><strong>{{ time(currentRelation.last_vacuum) }}</strong></article>
          <article><span>Last Autovacuum</span><strong>{{ time(currentRelation.last_autovacuum) }}</strong></article>
        </div>
        <details class="relation-evidence"><summary>{{ tr('xidUi.text098') }}</summary>
          <dl class="relation-facts">
            <div><dt>{{ tr('xidUi.text099') }}</dt><dd>{{ currentRelation.relation_type }} / {{ currentRelation.relation_oid }}</dd></div>
            <div><dt>Freeze max (source)</dt><dd>{{ n(currentRelation.effective_freeze_max_age) }} ({{ currentRelation.freeze_max_source }})</dd></div>
            <div><dt>Freeze table age (source)</dt><dd>{{ n(currentRelation.effective_freeze_table_age) }} ({{ currentRelation.freeze_table_source ?? '—' }})</dd></div>
            <div><dt>Freeze min age (source)</dt><dd>{{ n(currentRelation.effective_freeze_min_age) }} ({{ currentRelation.freeze_min_source ?? '—' }})</dd></div>
            <div><dt>MXID freeze max</dt><dd>{{ n(currentRelation.effective_multixact_freeze_max_age) }}</dd></div>
            <div><dt>{{ tr('xidUi.text100') }}</dt><dd>{{ n(currentRelation.estimated_rows) }}</dd></div>
            <div><dt>Heap pages / all-visible / all-frozen</dt><dd>{{ n(currentRelation.heap_pages) }} / {{ n(currentRelation.all_visible_pages) }} / {{ n(currentRelation.all_frozen_pages) }}</dd></div>
            <div><dt>All-visible %</dt><dd>{{ pct(currentRelation.all_visible_pct) }}</dd></div>
            <div><dt>{{ tr('xidUi.text101') }}</dt><dd>{{ time(currentRelation.last_analyze) }} / {{ time(currentRelation.last_autoanalyze) }}</dd></div>
            <div><dt>ANALYZE / Autoanalyze count</dt><dd>{{ n(currentRelation.analyze_count) }} / {{ n(currentRelation.autoanalyze_count) }}</dd></div>
            <div><dt>{{ tr('xidUi.text102') }}</dt><dd>{{ n(currentRelation.modifications_since_analyze) }}</dd></div>
            <div><dt>TOAST parent</dt><dd><button v-if="currentRelation.parent_oid && hasRelation(currentRelation.parent_oid)" type="button" class="inline-link" @click="openRelation(currentRelation.parent_oid)">{{ currentRelation.parent_schema }}.{{ currentRelation.parent_relation }} ({{ currentRelation.parent_oid }})</button><span v-else>{{ currentRelation.parent_schema && currentRelation.parent_relation ? `${currentRelation.parent_schema}.${currentRelation.parent_relation}` : '—' }}</span></dd></div>
            <div><dt>Relation reloptions</dt><dd>{{ currentRelation.reloptions?.join(', ') || '—' }}</dd></div>
          </dl>
          <p class="muted">{{ tr('xidUi.text103') }}</p>
        </details>
        <p v-if="snap.autovacuum_triggers.error" class="warning">{{ tr('xidFinal.thresholdFailure') }}{{ snap.autovacuum_triggers.error }}</p>
        <p v-else-if="currentTrigger" class="muted">Dead {{ n(currentTrigger.estimated_dead_rows) }} / {{ n(currentTrigger.estimated_dead_tuple_trigger) }} · INSERT {{ n(currentTrigger.inserts_since_vacuum) }} / {{ n(currentTrigger.estimated_insert_trigger) }}</p>
        <p v-if="snap.vacuums.error" class="warning">{{ tr('xidFinal.vacuumUnknown') }}{{ snap.vacuums.error }}</p>
        <p v-else-if="currentVacuum" class="notice">VACUUM PID {{ currentVacuum.pid }}, {{ tr('xidFinal.phase') }}{{ currentVacuum.phase }}, {{ pct(currentVacuum.scan_pct) }}</p>
        <p v-else class="muted">{{ tr('xidUi.text104') }}</p>
        <p v-if="state.loadingRelation" class="muted">{{ tr('xidUi.text105') }}</p>
        <p v-if="state.relationError" class="warning" role="alert">{{ state.relationError }}</p>
        <h3>{{ tr('xidUi.text106') }}</h3>
        <p class="muted">{{ tr('xidUi.text107') }}</p>
        <p v-if="!state.loadingRelation && !state.relationHistory.length" class="muted">{{ tr('xidUi.text108') }}</p>
        <div v-else class="scroll"><table><thead><tr><th>Snapshot</th><th>{{ tr('xidUi.text109') }}</th><th>XID age</th><th>MXID age</th><th>VACUUM</th><th>Stats reset</th></tr></thead>
          <tbody><tr v-for="point in visibleRelationHistory" :key="point.snapshot_id"><td>#{{ point.snapshot_id }}</td><td>{{ time(point.collection_started_at_ms) }}</td><td>{{ n(point.relation.xid_age) }}</td><td>{{ n(point.relation.mxid_age) }}</td><td>{{ point.active_vacuum ? `PID ${point.active_vacuum.pid}`:'—' }}</td><td>{{ time(point.stats_reset) }}</td></tr></tbody></table></div>
        <button v-if="state.relationHistory.length > 15" type="button" class="secondary" @click="historyExpanded=!historyExpanded">{{ historyExpanded ? tr('xidFinish.historyCollapse') : tr('xidFinal.showAll', { count: String(state.relationHistory.length) }) }}</button>
        <h3>Freeze Policy Advisor</h3>
        <p v-if="snap.overview.stats_reset == null" class="warning" role="status">
          <strong>{{ tr('xidUi.text110') }}</strong>
          {{ tr('xidFinal.observations') }}
          {{ tr('xidFinal.noApply') }}
        </p>
        <p v-if="!state.policy && !state.loadingRelation" class="muted">{{ tr('xidUi.text111') }}</p>
        <template v-if="state.policy">
          <div class="assessment"><strong>{{ state.policy.title }}</strong><p>{{ state.policy.summary }}</p>
            <p class="muted">{{ tr('xidFinal.currentStatus') }}{{ state.policy.status }}{{ tr('xidFinal.points') }}{{ n(state.policy.history_points) }}{{ tr('xidFinal.window') }}{{ n(state.policy.history_span_seconds,1) }}{{ tr('xidFinal.seconds') }}{{ tr('xidFinal.rate') }}{{ n(state.policy.xid_rate_per_hour,1) }} XID/{{ tr('vueRemaining.hour') }}{{ tr('remainingVue.cycles') }}{{ n(state.policy.vacuum_cycles_delta) }}</p></div>
          <details><summary>{{ tr('xidUi.text112') }}</summary>
            <h4>{{ tr('xidUi.text113') }}</h4><ul><li v-for="(line,i) in state.policy.observations" :key="i">{{ line }}</li></ul>
            <h4>{{ tr('xidUi.text114') }}</h4><ul><li v-for="(line,i) in state.policy.limitations" :key="i">{{ line }}</li></ul>
          </details>
          <p class="muted">{{ tr('xidFinal.currentPolicy') }}freeze max {{ n(state.policy.current_policy.effective_freeze_max_age) }} ({{ state.policy.current_policy.freeze_max_source }}), freeze table {{ n(state.policy.current_policy.effective_freeze_table_age) }}, freeze min {{ n(state.policy.current_policy.effective_freeze_min_age) }}.</p>
          <p v-if="!state.policy.candidates.length" class="muted">{{ tr('xidUi.text115') }}</p>
          <article v-for="c in state.policy.candidates" :key="c.id" class="candidate">
            <h4>{{ c.title }} <span v-if="c.id===state.policy.primary_candidate_id" class="tag">{{ tr('xidUi.text116') }}</span></h4>
            <p>{{ c.ddl_target }} · {{ c.parameter }}: {{ n(c.current_value) }} → {{ n(c.candidate_value) }}</p>
            <details><summary>{{ tr('xidFinal.sqlPreview') }}</summary>
              <pre>{{ c.sql }}</pre>
              <h4>{{ tr('xidFinal.why') }}</h4><ul><li v-for="(line,i) in c.why" :key="i">{{ line }}</li></ul>
              <h4>{{ tr('xidFinal.effect') }}</h4><ul><li v-for="(line,i) in c.expected_effect" :key="i">{{ line }}</li></ul>
              <h4>{{ tr('xidFinal.tradeoffs') }}</h4><ul><li v-for="(line,i) in c.tradeoffs" :key="i">{{ line }}</li></ul>
            </details>
            <button type="button" :disabled="!matchingSnapshot || state.loadingRelation || state.applying || state.collecting" @click="confirmApply(c)">{{ tr('xidFinal.applyPolicy') }}</button>
          </article>
        </template>
      </template>
    </template>
  </section>
  <section v-else-if="tab==='reference'" class="panel">
    <h2>{{ tr('xidFinal.methodologyTitle') }}</h2><p v-if="state.referenceLoading">{{ tr('xidFinal.methodologyLoading') }}</p>
    <template v-if="state.reference">
      <details v-for="step in state.reference.methodology" :key="step.id" class="finding"><summary>{{ step.title }}</summary><p>{{ step.purpose }}</p><ul><li v-for="(line,i) in step.how_to_read" :key="i">{{ line }}</li></ul></details>
      <h3>{{ tr('xidFinal.collectorQueries') }}</h3>
      <details v-for="q in state.reference.queries" :key="q.id" class="finding"><summary>{{ q.title }}</summary><p>{{ q.purpose }}</p>
        <h4>{{ tr('xidFinal.outputFields') }}</h4><ul><li v-for="(line,i) in q.output" :key="i">{{ line }}</li></ul>
        <h4>{{ tr('xidFinal.interpretation') }}</h4><ul><li v-for="(line,i) in q.how_to_read" :key="i">{{ line }}</li></ul>
        <button type="button" class="secondary" @click="copySql(q.sql)">{{ tr('xidFinal.copySql') }}</button>
        <pre>{{ q.sql }}</pre></details>
      <h3>{{ tr('xidFinal.docs') }}</h3>
      <ul><li v-for="source in state.reference.sources" :key="source.id"><a v-if="xidDocsUrl(snap?.overview.server_version_num ?? 0,source.path)" :href="xidDocsUrl(snap?.overview.server_version_num ?? 0,source.path) ?? undefined" target="_blank" rel="noopener noreferrer">{{ source.title }}</a><span v-else>{{ source.title }}</span> — {{ source.purpose }}</li></ul>
    </template>
  </section>
  <dialog v-if="candidateToApply" ref="confirmDialog" class="confirm" role="alertdialog" aria-modal="true" aria-labelledby="xid-apply-title" aria-describedby="xid-apply-description" @cancel="cancelApply" @close="cancelApply">
      <h2 id="xid-apply-title">{{ tr('xidFinal.confirmTitle') }}</h2>
      <p id="xid-apply-description">{{ tr('xidFinal.confirmDescription') }}</p>
      <p><strong>{{ candidateToApply.ddl_target }}</strong><br>{{ candidateToApply.parameter }}: {{ n(candidateToApply.current_value) }} → {{ n(candidateToApply.candidate_value) }}</p>
      <pre>{{ candidateToApply.sql }}</pre>
      <p class="warning">{{ tr('xidFinal.confirmWarning') }}</p>
      <label class="confirm-phrase">{{ tr('xidFinal.confirmPhrase') }}<strong>APPLY</strong>
        <input v-model="applyPhrase" autocomplete="off" spellcheck="false" :aria-label="tr('xidFinal.applyAria')" placeholder="APPLY"></label>
      <div class="toolbar"><button type="button" class="secondary" @click="cancelApply">{{ tr('xidFinal.cancel') }}</button><button type="button" :disabled="state.applying || state.loadingRelation || applyPhrase !== 'APPLY'" @click="applyConfirmed">{{ tr('xidFinal.confirmApply') }}</button></div>
  </dialog>
</section>
</template>

<style scoped>
.xid-view { min-width:0; max-width:100%; }
h1 { margin:0 0 .35rem; } h2 { margin:.2rem 0 1rem; font-size:1.15rem; } h3 { font-size:1rem; margin:1.3rem 0 .7rem; }
h4 { margin:.9rem 0 .5rem; } p { line-height:1.5; }
.title-row,.toolbar { display:flex; flex-wrap:wrap; align-items:end; gap:.8rem; justify-content:space-between; }
.title-row { align-items:start; }
.toolbar { justify-content:flex-start; margin:.9rem 0; }
.toolbar label { display:grid; gap:.3rem; min-width:180px; font-size:.84rem; color:var(--text-secondary); }
select { padding:.6rem .75rem; max-width:min(550px,85vw); background:var(--surface-2); color:var(--text); border:1px solid var(--border-strong); border-radius:.5rem; }
.muted { color:var(--text-secondary); } .error,.warning { color:var(--error); } .notice { color:var(--text); padding:.6rem .8rem; border-left:3px solid var(--accent); background:var(--surface-2); }
.tag { color:var(--text-secondary); background:var(--surface-2); border:1px solid var(--border); border-radius:.5rem; font-size:.75rem; padding:.35rem .55rem; white-space:nowrap; }
.tabs { display:flex; flex-wrap:wrap; gap:.35rem; border-bottom:1px solid var(--border); margin:1.2rem 0 1rem; }
.tab { border:0; background:transparent; color:var(--text-secondary); border-radius:.5rem .5rem 0 0; }
.tab.selected { border-bottom:3px solid var(--accent); color:var(--text); }
.panel { border:1px solid var(--border); background:var(--surface); border-radius:.7rem; padding:1.1rem; margin:1.1rem 0; }
.metrics { display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:.65rem; }
.metrics article { display:grid; gap:.3rem; background:var(--surface-2); border:1px solid var(--border); padding:.8rem; border-radius:.55rem; min-width:0; }
.metrics article span,.metrics article small { font-size:.78rem; color:var(--text-secondary); }
.metrics article strong { font-size:1.07rem; overflow-wrap:anywhere; }
.scroll { overflow-x:auto; max-width:100%; }
table { width:100%; border-collapse:collapse; font-size:.84rem; text-align:left; }
th,td { padding:.65rem; border-bottom:1px solid var(--border); vertical-align:top; } th { white-space:nowrap; color:var(--text-secondary); } tr:hover { background:var(--surface-2); }
.finding,.candidate { padding:.75rem 1rem; border:1px solid var(--border); border-radius:.5rem; margin:.6rem 0; }
.finding summary,.candidate summary { cursor:pointer; font-weight:600; }
.finding h3 { margin:1rem 0 .35rem; }
.pager { display:flex; align-items:center; justify-content:flex-end; gap:.7rem; margin:.8rem 0; }
.warnings { color:var(--text-secondary); } .warnings li { margin:.4rem 0; }
.snapshot-links { display:flex; gap:.5rem .85rem; flex-wrap:wrap; align-items:center; margin:.9rem 0; font-size:.85rem; }
.snapshot-links a { color:var(--text); border:1px solid var(--border); background:var(--surface-2); border-radius:.5rem; padding:.4rem .65rem; text-decoration:none; }
.snapshot-links a:hover,.snapshot-links a:focus-visible { border-color:var(--accent); }
.quality-details > summary { cursor:pointer; font-weight:600; }
.quality-details[open] > summary { margin-bottom:.65rem; }
.section-count { font-size:.78rem; color:var(--text-secondary); font-weight:400; }
.table-tools { justify-content:flex-start; }
.table-tools input { width:min(290px,65vw); box-sizing:border-box; }
.table-hint { font-size:.83rem; margin:.45rem 0 .75rem; }
.table-actions { display:flex; align-items:center; flex-wrap:wrap; justify-content:space-between; gap:.5rem; }
.table-actions button { white-space:nowrap; }
.history-scatter { border:1px solid var(--border); border-radius:.6rem; padding:.75rem; margin:.8rem 0; }
.history-scatter h3 { margin:.25rem 0; }
.age-scatter { display:block; width:100%; height:auto; max-height:250px; }
.chart-axis { stroke:var(--border-strong); stroke-width:1.5; }
.point-database { fill:var(--accent); }
.point-relation { fill:var(--text-secondary); }
.chart-legend { display:flex; flex-wrap:wrap; gap:1rem; font-size:.8rem; }
.legend-db { color:var(--accent); }
.legend-relation { color:var(--text-secondary); }
.chart-captions { display:flex; flex-wrap:wrap; justify-content:space-between; gap:.4rem; font-size:.78rem; color:var(--text-secondary); }
.brief-heading { display:flex; align-items:flex-start; justify-content:space-between; flex-wrap:wrap; gap:.6rem; }
.brief-heading p { margin:.1rem 0 .8rem; }
.brief-metrics { grid-template-columns:repeat(auto-fit,minmax(190px,1fr)); }
.brief-metrics progress { width:100%; height:.6rem; accent-color:var(--accent); }
.brief-columns { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:1rem; margin-top:.85rem; }
.brief-columns h3 { margin:.2rem 0 .55rem; }
.quick-relations { padding-left:1.35rem; margin:.2rem 0; }
.quick-relations li { display:flex; align-items:baseline; justify-content:space-between; gap:.5rem; padding:.35rem .2rem; border-bottom:1px solid var(--border); }
.quick-relations li span { font-variant-numeric:tabular-nums; white-space:nowrap; }
.inline-link { border:0; background:transparent; color:var(--text); padding:.1rem; text-align:left; cursor:pointer; text-decoration:underline; text-decoration-color:var(--border-strong); overflow-wrap:anywhere; }
.inline-link:hover,.inline-link:focus-visible { color:var(--accent); }
.brief-note { font-size:.8rem; margin-bottom:0; }
.relation-evidence { border:1px solid var(--border); border-radius:.5rem; padding:.7rem; margin:.8rem 0; }
.relation-evidence > summary { cursor:pointer; font-weight:600; }
.relation-facts { display:grid; grid-template-columns:repeat(auto-fit,minmax(250px,1fr)); gap:.5rem .9rem; }
.relation-facts div { min-width:0; border-bottom:1px solid var(--border); padding:.4rem 0; }
.relation-facts dt { font-size:.78rem; color:var(--text-secondary); }
.relation-facts dd { margin:.25rem 0 0; overflow-wrap:anywhere; }
.confirm-phrase { display:grid; gap:.4rem; margin:1rem 0; }
.confirm-phrase input { max-width:230px; }
.overview-details > summary { cursor:pointer; font-weight:600; }
.overview-details[open] > summary { margin-bottom:1rem; }
@media(max-width:780px) { .brief-columns {grid-template-columns:1fr;} }
th[aria-sort] { padding:0; }
.sort-heading { display:inline-flex; align-items:center; gap:.35rem; width:auto; text-align:left; justify-content:flex-start;
  border:none; border-radius:0; color:var(--text-secondary); background:transparent; font:inherit; font-weight:600; padding:.65rem; cursor:pointer; white-space:nowrap; }
.sort-heading:hover,.sort-heading:focus-visible { color:var(--text); background:var(--surface-2); }
.sort-heading span { color:var(--accent); min-width:1rem; text-align:center; }
.archive-banner { border-left:3px solid var(--accent); background:var(--surface-2); padding:.75rem; display:flex; flex-wrap:wrap; align-items:center; gap:.6rem; }
.relation-picker-tools label { min-width:190px; }
.relation-picker-tools input { width:min(300px,80vw); }
tr:has(.sort-heading) th { vertical-align:middle; }
pre { background:var(--surface-2); border:1px solid var(--border); border-radius:.4rem; padding:.85rem; white-space:pre-wrap; overflow-wrap:anywhere; max-height:400px; overflow:auto; font-size:.8rem; }
.assessment { border-left:3px solid var(--accent); padding:.65rem 1rem; background:var(--surface-2); }
.confirm { width:min(720px,calc(100% - 2rem)); box-sizing:border-box; max-height:90vh; overflow:auto; padding:1.5rem; border-radius:.85rem; border:1px solid var(--border-strong); background:var(--surface); color:var(--text); box-shadow:0 10px 40px rgba(0,0,0,.25); }
.confirm::backdrop { background:rgba(0,0,0,.7); }
@media(max-width:800px) { .panel {padding:.7rem;} .toolbar label {flex:1; min-width:145px;} }
</style>
