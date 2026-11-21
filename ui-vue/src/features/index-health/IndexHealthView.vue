<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import { createIndexHealthController, createIndexHealthState, filteredIndexPage, indexDocsUrl, indexLabel, summaryMatchesConnection, type IndexRow } from "./controller.js";

const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; navigationToken?: number; preferredConnectionId?: number | null; quickSelection?: QuickConnectionSelection | null;}>();
const emit = defineEmits<{openHistory: [connectionId: number | null]}>();
const state = reactive(createIndexHealthState());
const controller = createIndexHealthController(props.shell, state, props.invoke);
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.collecting || state.loading)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.collecting || state.loading) return;
  lastShortcutSerial = request.serial;
  // Never retarget a running collector/monitor or an in-flight database operation.
  if (state.connectionId !== request.connectionId) controller.selectConnection(request.connectionId);
});

const search = ref("");
const mode = ref("all");
const page = ref(1);
const pageSize = ref(25);
const selected = computed(() => state.connections.find(c => c.id === state.connectionId));
const selectedSummary = computed(() => state.histories.find(s => s.id === state.snapshotId));
const sameLiveTarget = computed(() => summaryMatchesConnection(selectedSummary.value, selected.value));
const serverVersion = computed(() => state.snapshot?.overview.server_version_num ?? selectedSummary.value?.server_version_num ?? null);
const pageData = computed(() => filteredIndexPage(state.snapshot, { search: search.value, mode: mode.value, page: page.value, pageSize: pageSize.value }));
const shownComparison = computed(() => {
  const comparison = state.comparison;
  const ids = [state.compareOlderId, state.compareNewerId];
  return comparison && ids.includes(comparison.older.id) && ids.includes(comparison.newer.id) ? comparison : null;
});
const compareRows = computed(() => shownComparison.value?.index_changes.slice(0, 50) ?? []);
const counts = computed(() => {
  const rows = state.snapshot?.collection.indexes ?? [];
  const count = (label: string) => rows.filter(row => row.classifications.includes(label)).length;
  return {
    used: count("actively_used"), unused: count("suspiciously_unused"),
    duplicate: count("possible_duplicate"), overhead: count("high_write_overhead"),
    invalid: rows.filter(row => !row.is_valid || !row.is_ready).length,
  };
});
const usageEvidenceAvailable = computed(() => {
  const overview = state.snapshot?.overview;
  return !!overview && overview.track_counts && overview.stats_age_seconds !== null;
});
const hasReviewSignals = computed(() => counts.value.invalid > 0 || counts.value.duplicate > 0 || (usageEvidenceAvailable.value && counts.value.unused > 0));
function shortIndexLabel(label: string): string {
  const names: Record<string, string> = {
    actively_used: t('indexHealth.activelyUsed'),
    low_evidence: t('indexHealth.lowEvidence'),
    suspiciously_unused: t('indexHealth.unused'),
    possible_duplicate: t('indexHealth.duplicate'),
    high_write_overhead: t('indexHealth.writeOverhead'),
    needs_investigation: t('indexHealth.investigate'),
  };
  return names[label] ?? label;
}
function indexExplanation(label: string): string {
  const explanations: Record<string, string> = {
    actively_used: t('indexHealth.activeReason'),
    low_evidence: t('indexHealth.lowReason'),
    suspiciously_unused: t('indexHealth.unusedReason'),
    possible_duplicate: t('indexHealth.duplicateReason'),
    high_write_overhead: t('indexHealth.writeReason'),
    needs_investigation: t('indexHealth.investigateReason'),
  };
  return explanations[label] ?? indexLabel(label);
}
function time(ms: number | null): string { return ms === null || !Number.isFinite(ms) ? "—" : new Date(ms).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US"); }
function date(value: string | null): string { if (!value) return "—"; const parsed = Date.parse(value); return Number.isFinite(parsed) ? time(parsed) : value; }
function int(value: number | null | undefined): string { return value === null || value === undefined ? "—" : new Intl.NumberFormat(locale.value === "ru" ? "ru-RU" : "en-US").format(value); }
function size(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  const abs = Math.abs(value);
  if (abs < 1024) return `${int(value)} ${t('indexHealth.byte')}`;
  const unit = abs < 1048576 ? 1024 : abs < 1073741824 ? 1048576 : 1073741824;
  return `${(value / unit).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 1 })} ${unit === 1024 ? t('indexHealth.kib') : unit === 1048576 ? t('indexHealth.mib') : t('indexHealth.gib')}`;
}
function onConnection(event: Event): void { const n = Number((event.target as HTMLSelectElement).value); controller.selectConnection(Number.isSafeInteger(n) && n > 0 ? n : null); page.value = 1; }
function bootstrap(): void {
  if (props.shell.screen === "workspace" && !props.shell.busy && props.shell.currentUsername && !state.connections.length) {
    void controller.refreshConnections().then(() => controller.loadReference());
  }
}
watch([search, mode, pageSize], () => { page.value = 1; });
watch(() => props.shell.authGeneration, () => { controller.reset(); bootstrap(); });
watch(() => props.shell.screen, screen => { if (screen !== "workspace") controller.reset(); else bootstrap(); });
watch(() => props.shell.busy, bootstrap);
watch(() => props.navigationToken, () => { if (props.visible) bootstrap(); });
watch([() => state.connections, () => props.navigationToken], () => {
  if (!props.navigationToken || !props.visible) return;
  const id = props.preferredConnectionId;
  if (id !== null && id !== undefined && state.connections.some(c => c.id === id) && state.connectionId !== id) {
    controller.selectConnection(id);
  }
});
watch(() => props.visible, visible => {
  if (!visible) return;
  if (state.connections.length && props.shell.screen === "workspace" && !props.shell.busy)
    void controller.refreshConnections();
  else bootstrap();
});
onMounted(bootstrap);
onUnmounted(() => controller.dispose());
async function detail(row: IndexRow): Promise<void> { await controller.openTableDetail(row); }
</script>

<template>
<section class="index-view" aria-label="Index Health">
  <header class="index-heading">
    <div>
      <h1>{{ t('indexHealth.text000') }} <small>· Index Health</small></h1>
      <p class="muted">{{ t('indexHealth.text001') }}</p>
    </div>
    <div class="index-heading-actions"><span class="readonly">{{ t('indexHealth.text002') }}</span>
      <button type="button" class="secondary" @click="emit('openHistory', state.connectionId)">{{ t('indexHealth.text003') }}</button>
    </div>
  </header>

  <div class="index-controls index-toolbar">
    <label>{{ t('indexHealth.text004') }}
      <select :value="state.connectionId ?? ''" :disabled="state.collecting" @change="onConnection">
        <option value="">{{ t('indexHealth.text005') }}</option>
        <option v-for="c in state.connections" :key="c.id" :value="c.id">{{ c.name }} · {{ c.host }}:{{ c.port }}/{{ c.database }}</option>
      </select>
    </label>
    <button type="button" :disabled="state.connectionId === null || state.collecting || state.loading" @click="controller.collect">{{ state.collecting ? t('indexHealth.checking') : t('indexHealth.collect') }}</button>
  </div>
  <p class="muted helper">{{ t('indexHealth.text006') }}</p>
  <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
  <p v-if="state.notice" role="status" class="notice">{{ state.notice }}</p>
  <p v-if="state.loading" role="status">{{ t('indexHealth.text007') }}</p>

  <section class="index-panel" :aria-label="t('indexHealth.aria')">
    <div class="section-head">
      <div>
        <h2>{{ t('indexHealth.text008') }} <small v-if="selected">· {{ selected.name }}</small></h2>
        <p v-if="state.snapshot" class="muted">{{ t('indexFinish.snapshot') }}{{ state.snapshotId }} · {{ date(state.snapshot.overview.measured_at) }} · PostgreSQL {{ state.snapshot.overview.server_version }}{{ t('remainingVue.source') }}{{ selectedSummary?.host ?? "—" }}:{{ selectedSummary?.port ?? "—" }}/{{ selectedSummary?.database ?? state.snapshot.overview.database }}</p>
        <p v-else class="muted">{{ t('indexHealth.text009') }}</p>
      </div>
      <span v-if="state.snapshot" class="status-chip" :class="hasReviewSignals ? 'status-attention' : 'status-neutral'">{{ hasReviewSignals ? t('indexHealth.reviewSignals') : !usageEvidenceAvailable ? t('indexHealth.limited') : t('indexHealth.preliminary') }}</span>
    </div>
    <p v-if="state.snapshot && !sameLiveTarget" class="warning">{{ t('indexHealth.text010') }}</p>

    <template v-if="state.snapshot">
      <div class="metrics">
        <article><span>{{ t('indexHealth.text011') }}</span><strong>{{ int(state.snapshot.collection.sampled_index_count) }} <small>{{ t('diagResidual.idxhas') }} {{ int(state.snapshot.collection.total_index_count) }}</small></strong><small>{{ t('indexHealth.text012') }}</small></article>
        <article><span>{{ t('indexHealth.text013') }}</span><strong>{{ int(counts.used) }}</strong><small>{{ t('indexHealth.text014') }}</small></article>
        <article><span>{{ t('indexHealth.text015') }}</span><strong>{{ int(counts.duplicate) }}</strong><small>{{ t('indexHealth.text016') }}</small></article>
        <article><span>{{ t('indexHealth.text017') }}</span><strong>{{ usageEvidenceAvailable ? int(counts.unused) : t('indexHealth.notEvaluated') }}</strong><small>{{ usageEvidenceAvailable ? t('indexHealth.candidates') : t('indexHealth.insufficient') }}</small></article>
      </div>

      <div class="assessment" role="status">
        <h3>{{ t('indexHealth.text018') }}</h3>
        <p v-if="counts.invalid" class="warning"><strong>{{ int(counts.invalid) }} {{ t('diagResidual.idxindexes') }}</strong> {{ t('indexHealth.text019') }}</p>
        <p v-if="counts.duplicate"><strong>{{ int(counts.duplicate) }}{{ t('indexFinish.indexCount') }}</strong> {{ t('indexHealth.text020') }}</p>
        <p v-if="!usageEvidenceAvailable" class="warning">{{ t('indexFinish.usageUnavailable') }}{{ !state.snapshot.overview.track_counts ? t('indexHealth.trackCounts') : t('indexHealth.statsAge') }}{{ t('remainingVue.unusedCaveat') }}</p>
        <p v-else-if="counts.unused"><strong>{{ int(counts.unused) }}{{ t('indexFinish.indexCount') }}</strong> {{ t('indexHealth.text021') }}</p>
        <p v-if="state.snapshot.collection.truncated" class="warning">{{ t('indexFinish.sampleFrom') }}{{ int(state.snapshot.collection.total_index_count) }}{{ t('indexFinish.examined') }}{{ int(state.snapshot.collection.sampled_index_count) }}{{ t('remainingVue.samplingCaveat') }}</p>
        <p v-if="!counts.invalid && !counts.duplicate && usageEvidenceAvailable && !counts.unused && !state.snapshot.collection.truncated">{{ t('indexHealth.text022') }}</p>
        <details class="technical-note"><summary>{{ t('indexHealth.text023') }}</summary>
          <p>{{ t('indexHealth.text024') }} <code>idx_scan</code> {{ t('indexHealth.text025') }}</p>
          <p v-if="state.snapshot.overview.stats_reset">{{ t('diagResidual.idxreset') }} {{ date(state.snapshot.overview.stats_reset) }}.</p>
          <p>{{ t('indexHealth.text026') }}</p>
          <ul><li v-for="(limitation, i) in state.snapshot.interpretation.limitations" :key="i">{{ limitation }}</li></ul>
        </details>
      </div>
    </template>
  </section>

  <section class="index-panel" :aria-label="t('indexHealth.listAria')">
    <div class="section-head">
      <div><h2>{{ t('indexHealth.text027') }}</h2><p class="muted">{{ t('indexHealth.text028') }}</p></div>
    </div>
    <div class="index-controls">
      <label>{{ t('diagResidual.search') }} <input v-model="search" :placeholder="t('indexHealth.searchPlaceholder')" /></label>
      <label>{{ t('diagResidual.filter') }} <select v-model="mode"><option value="all">{{ t('indexHealth.text029') }}</option><option value="possible_duplicate">{{ t('indexHealth.text030') }}</option><option value="suspiciously_unused">{{ t('indexHealth.text017') }}</option><option value="low_evidence">{{ t('indexHealth.text031') }}</option><option value="needs_investigation">{{ t('indexHealth.text032') }}</option><option value="high_write_overhead">{{ t('indexHealth.text033') }}</option><option value="actively_used">{{ t('indexHealth.text034') }}</option></select></label>
      <label>{{ t('diagResidual.pageSize') }} <select v-model.number="pageSize"><option :value="10">10</option><option :value="25">25</option><option :value="50">50</option></select></label>
    </div>
    <p class="muted">{{ t('diagResidual.idxfound') }} {{ pageData.total }} · {{ t('diagResidual.idxpage') }} {{ pageData.page }} {{ t('diagResidual.idxofPage') }} {{ pageData.pages }}</p>
    <div class="table-scroll"><table><thead><tr><th>{{ t('indexHealth.text035') }}</th><th>{{ t('indexHealth.text036') }}</th><th>{{ t('indexHealth.text037') }}</th><th>{{ t('indexHealth.text038') }}</th><th>{{ t('indexHealth.text039') }}</th></tr></thead><tbody>
      <tr v-for="row in pageData.rows" :key="row.index_oid">
        <td><strong>{{ row.schema_name }}.{{ row.relation_name }}</strong><div>{{ row.index_name }}</div><small>OID {{ row.index_oid }} <span v-if="row.is_primary">· PRIMARY KEY</span><span v-if="row.is_unique">· UNIQUE</span><span v-if="!row.is_valid || !row.is_ready" class="warning">· INVALID / Not ready</span></small></td>
        <td>{{ size(row.index_size_bytes) }}</td>
        <td>{{ int(row.idx_scan) }}<small class="muted">{{ t('indexHealth.text040') }}</small></td>
        <td><div v-for="label in row.classifications" :key="label" class="tag" :title="indexExplanation(label)">{{ shortIndexLabel(label) }}</div><span v-if="!row.classifications.length" class="muted">{{ t('indexHealth.text041') }}</span></td>
        <td><button type="button" class="secondary small-button" :disabled="state.detailLoading || state.loading || state.collecting || !sameLiveTarget" @click="detail(row)">{{ t('indexHealth.text042') }}</button></td>
      </tr>
      <tr v-if="!pageData.rows.length"><td colspan="5" class="muted">{{ t('indexHealth.text043') }}</td></tr>
    </tbody></table></div>
    <div class="pager"><button type="button" class="secondary" :disabled="pageData.page <= 1" @click="page = pageData.page - 1">{{ t('indexHealth.text044') }}</button><span>{{ pageData.page }} / {{ pageData.pages }}</span><button type="button" class="secondary" :disabled="pageData.page >= pageData.pages" @click="page = pageData.page + 1">{{ t('indexHealth.text045') }}</button></div>
    <details class="technical-note"><summary>{{ t('indexHealth.text046') }}</summary>
      <dl class="glossary"><template v-for="label in ['actively_used', 'low_evidence', 'suspiciously_unused', 'possible_duplicate', 'high_write_overhead', 'needs_investigation']" :key="label"><dt>{{ shortIndexLabel(label) }}</dt><dd>{{ indexExplanation(label) }}</dd></template></dl>
    </details>
    <details v-if="state.snapshot" class="technical-note"><summary>{{ t('indexHealth.text047') }}</summary>
      <p class="muted">{{ t('diagResidual.sensitive') }}</p>
      <div class="table-scroll"><table><thead><tr><th>{{ t('diagResidual.index') }}</th><th>{{ t('diagResidual.keys') }}</th><th>Read / Fetch</th><th>{{ t('diagResidual.tableDml') }}</th><th>HOT</th><th>{{ t('diagResidual.analyze') }}</th></tr></thead><tbody><tr v-for="row in pageData.rows" :key="row.index_oid"><td>{{ row.index_name }}</td><td>{{ row.key_columns.join(', ') }}{{ row.predicate ? ` WHERE ${row.predicate}` : '' }}</td><td>{{ int(row.idx_tup_read) }} / {{ int(row.idx_tup_fetch) }}</td><td>{{ int(row.tuples_inserted) }} / {{ int(row.tuples_updated) }} / {{ int(row.tuples_deleted) }}</td><td>{{ int(row.hot_updates) }}</td><td>{{ date(row.last_analyze) }} / {{ date(row.last_autoanalyze) }}</td></tr></tbody></table></div>
    </details>
  </section>

  <section v-if="state.detailLoading || state.tableDetail" class="index-panel" :aria-label="t('indexHealth.detailAria')">
    <div class="index-heading"><h2>{{ t('diagResidual.currentTable') }}</h2><button type="button" class="secondary" @click="controller.closeTableDetail">{{ t('diagResidual.close') }}</button></div>
    <p class="muted">{{ t('diagResidual.currentNote') }}</p>
    <p v-if="state.detailLoading">{{ t('diagResidual.loading') }}</p>
    <template v-if="state.tableDetail"><h3>{{ state.tableDetail.table.schema_name }}.{{ state.tableDetail.table.relation_name }}</h3>
      <div class="metrics"><article><span>{{ t('indexFinish.tableIndexSize') }}</span><strong>{{ size(state.tableDetail.table.total_size_bytes) }}</strong></article><article><span>{{ t('indexFinish.liveRows') }}</span><strong>{{ int(state.tableDetail.table.live_tuples) }}</strong></article><article><span>{{ t('vueRemaining.deadRows') }}</span><strong>{{ int(state.tableDetail.table.dead_tuples) }}</strong></article><article><span>{{ t('vueRemaining.lastAv') }}</span><strong>{{ date(state.tableDetail.table.last_autovacuum) }}</strong></article></div>
    </template>
  </section>

  <section class="index-panel" :aria-label="t('indexHealth.historyAria')">
    <div class="section-head"><div><h2>{{ t('diagResidual.savedChecks') }}</h2><p class="muted">{{ t('diagResidual.snapshotCount', { count: String(state.histories.length) }) }}</p></div></div>
    <details :open="state.histories.length > 0 && state.histories.length <= 3" class="technical-note"><summary>{{ t('diagResidual.snapshotList', { count: String(state.histories.length) }) }}</summary>
      <div class="table-scroll history"><table><thead><tr><th>№</th><th>{{ t('diagResidual.date') }}</th><th>{{ t('diagResidual.sampled') }}</th><th>{{ t('diagResidual.possiblyUnused') }}</th><th>{{ t('diagResidual.duplicates') }}</th><th></th></tr></thead><tbody>
        <tr v-for="s in state.histories" :key="s.id"><td>№{{ s.id }}</td><td>{{ time(s.collection_completed_at_ms) }}</td><td>{{ s.sampled_index_count }}{{ t('remainingVue.countFrom') }}{{ s.total_index_count }}{{ s.sample_truncated ? t('indexHealth.partial') : '' }}</td><td>{{ s.suspiciously_unused_count }}</td><td>{{ s.possible_duplicate_count }}</td><td><button class="secondary small-button" type="button" :disabled="state.loading || state.collecting" @click="controller.loadSnapshot(s.id)">{{ s.id === state.snapshotId ? t('indexHealth.opened') : t('indexHealth.open') }}</button></td></tr>
        <tr v-if="!state.histories.length"><td colspan="6" class="muted">{{ t('diagResidual.noChecks') }}</td></tr>
      </tbody></table></div>
    </details>
    <div class="compare-area">
      <h3>{{ t('diagResidual.changes') }}</h3>
      <p class="muted">{{ t('diagResidual.compareHint') }}</p>
      <div class="index-controls">
        <label>{{ t('diagResidual.earlier') }} <select v-model.number="state.compareOlderId"><option :value="null">{{ t('diagResidual.pick') }}</option><option v-for="s in state.histories" :key="s.id" :value="s.id">№{{ s.id }} · {{ time(s.collection_completed_at_ms) }}</option></select></label>
        <label>{{ t('diagResidual.later') }} <select v-model.number="state.compareNewerId"><option :value="null">{{ t('diagResidual.pick') }}</option><option v-for="s in state.histories" :key="s.id" :value="s.id">№{{ s.id }} · {{ time(s.collection_completed_at_ms) }}</option></select></label>
        <button type="button" :disabled="state.comparing || state.loading || state.histories.length < 2" @click="controller.compare">{{ state.comparing ? t('indexHealth.comparing') : t('indexHealth.compare') }}</button>
      </div>
      <template v-if="shownComparison">
        <p><strong>{{ t('indexFinish.comparison') }}{{ shownComparison.older.id }} → №{{ shownComparison.newer.id }}</strong>{{ t('indexFinish.interval') }}{{ Math.round(shownComparison.elapsed_seconds) }}{{ t('indexFinish.seconds') }}{{ t('indexFinish.stats') }}{{ shownComparison.stats_period_continuous ? t('indexHealth.statsContinuous') : t('indexHealth.statsBroken') }}</p>
        <p v-for="(warning, i) in shownComparison.limitations" :key="i" class="warning">{{ warning }}</p>
        <div class="table-scroll"><table><thead><tr><th>{{ t('diagResidual.tableIndex') }}</th><th>{{ t('diagResidual.samplePresence') }}</th><th>{{ t('diagResidual.scanDelta') }}</th><th>{{ t('diagResidual.dml') }}</th><th>{{ t('diagResidual.sizeDelta') }}</th></tr></thead><tbody><tr v-for="row in compareRows" :key="row.index_oid"><td>{{ row.relation_name }} · {{ row.index_name }}</td><td>{{ row.sample_state }}</td><td>{{ int(row.idx_scan_delta) }}</td><td>{{ int(row.relation_dml_delta) }}</td><td>{{ size(row.size_delta_bytes) }}</td></tr><tr v-if="!compareRows.length"><td colspan="5">{{ t('remainingVue.noDifferences') }}</td></tr></tbody></table></div>
        <p v-if="shownComparison.index_changes.length > 50" class="muted">{{ t('diagResidual.moreChanges', { count: String(shownComparison.index_changes.length) }) }}</p>
      </template>
    </div>
    <p class="muted helper">{{ t('diagResidual.historyCaution') }}</p>
  </section>

  <details class="index-panel"><summary>{{ t('diagResidual.docsHeading') }}</summary>
    <p v-for="(item, i) in state.reference?.methodology ?? []" :key="i">{{ item }}</p>
    <div v-for="source in state.reference?.sources ?? []" :key="source.path"><strong>{{ source.title }}</strong> — {{ source.purpose }}
      <a v-if="indexDocsUrl(serverVersion, source.path)" :href="indexDocsUrl(serverVersion, source.path) ?? undefined" target="_blank" rel="noopener noreferrer">{{ t('diagResidual.docs') }}</a>
    </div>
  </details>
</section>
</template>

<style scoped>
.index-view {display:grid;gap:1rem;min-width:0}
.index-heading-actions{display:flex;align-items:center;gap:.65rem;flex-wrap:wrap}
.index-heading,.section-head {display:flex;justify-content:space-between;align-items:start;gap:1rem;flex-wrap:wrap}
.index-heading h1,.index-heading h2,.section-head h2{margin:0}
.index-heading h1 small,.section-head h2 small{font-size:.62em;font-weight:500;color:var(--text-secondary)}
.index-panel{border:1px solid var(--border-strong);border-radius:.75rem;background:var(--surface);padding:1rem;min-width:0}
.index-panel h2{margin-top:0}.index-panel h3{margin:.2rem 0 .6rem}
.index-panel small,.muted{color:var(--text-secondary);font-size:.85rem}
.readonly,.tag,.status-chip{display:inline-block;border:1px solid var(--border-strong);background:var(--surface-2);border-radius:.45rem;padding:.25rem .5rem;font-size:.8rem}
.status-attention{border-color:var(--message);color:var(--message)}
.status-neutral{color:var(--text-secondary)}
.tag{margin:.15rem .2rem .15rem 0;max-width:260px;white-space:normal}
.metrics{display:grid;grid-template-columns:repeat(auto-fit,minmax(195px,1fr));gap:.7rem;margin-top:1rem}
.metrics article{border:1px solid var(--border);background:var(--surface-2);border-radius:.55rem;padding:.85rem;display:grid;align-content:start;gap:.35rem}
.metrics strong{font-size:1.2rem}.metrics strong small{font-size:.76rem;font-weight:400}
.metrics span{font-size:.82rem;color:var(--text-secondary)}
.metrics article>small{font-size:.75rem;line-height:1.35}
.index-controls{display:flex;flex-wrap:wrap;gap:.75rem;align-items:end}
.index-controls label{display:grid;gap:.3rem;flex:1;min-width:180px}
.index-controls select,.index-controls input{width:100%;box-sizing:border-box;padding:.65rem;border:1px solid var(--border-strong);background:var(--surface-2);color:var(--text);border-radius:.5rem}
.index-toolbar{margin-top:.25rem}.helper{margin:.1rem 0 0}
.assessment{margin-top:1rem;border:1px solid var(--border);border-radius:.55rem;background:var(--surface-2);padding:1rem}
.assessment p{margin:.45rem 0;line-height:1.5}
.technical-note{margin-top:.75rem}.technical-note summary{cursor:pointer;font-weight:600}
.technical-note p,.technical-note ul{font-size:.85rem;line-height:1.5}
.technical-note ul{padding-left:1.25rem}
.glossary{display:grid;grid-template-columns:minmax(125px,230px) 1fr;gap:.55rem 1rem;font-size:.85rem}
.glossary dt{font-weight:600}.glossary dd{margin:0;color:var(--text-secondary)}
.table-scroll{overflow-x:auto;max-height:570px}
.table-scroll table{width:100%;border-collapse:collapse;font-size:.84rem}
.table-scroll td,.table-scroll th{padding:.7rem;border-bottom:1px solid var(--border);text-align:left;vertical-align:top}
.table-scroll td small{display:block;margin-top:.25rem;font-size:.75rem}
.table-scroll th{position:sticky;top:0;background:var(--surface)}
.history{max-height:300px}.small-button{font-size:.8rem;padding:.35rem .5rem;white-space:nowrap}
.pager{display:flex;align-items:center;justify-content:center;gap:1rem;margin-top:.75rem}
.compare-area{margin-top:1rem;border-top:1px solid var(--border);padding-top:1rem}
.warning{color:var(--message);font-size:.9rem}
.notice{padding:.65rem .85rem;border-radius:.5rem;background:var(--surface-2)}
.error{color:var(--error)}
@media(max-width:680px){.metrics{grid-template-columns:repeat(2,minmax(0,1fr))}.glossary{grid-template-columns:1fr}.glossary dd{padding-bottom:.5rem}}
</style>
