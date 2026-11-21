<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import { createIoWalController, createIoWalState, filteredIoWalPage, ioWalDocsUrl,
  ioWalSummaryMatchesConnection, type IoWalRelationSort } from "./controller.js";

const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; quickSelection?: QuickConnectionSelection | null;}>();
const emit = defineEmits<{openHistory: [connectionId: number | null]}>();
const state = reactive(createIoWalState());
const controller = createIoWalController(props.shell, state, props.invoke);
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
const page = ref(1);
const pageSize = ref(25);
const sizeSort = ref<IoWalRelationSort>("total");
const sizeDescending = ref(true);
function sortSizes(column: IoWalRelationSort): void {
  if (sizeSort.value === column) sizeDescending.value = !sizeDescending.value;
  else { sizeSort.value = column; sizeDescending.value = column !== "name"; }
  page.value = 1;
}
function sortLabel(column: IoWalRelationSort): string {
  return sizeSort.value === column ? (sizeDescending.value ? " ↓" : " ↑") : " ↕";
}
const selected = computed(() => state.connections.find(c => c.id === state.connectionId));
const selectedSummary = computed(() => state.histories.find(s => s.id === state.snapshotId));
const historicalProfileChanged = computed(() => !!selectedSummary.value && !ioWalSummaryMatchesConnection(selectedSummary.value, selected.value));
const snap = computed(() => state.snapshot);
const db = computed(() => snap.value?.database_stats.data);
const wal = computed(() => snap.value?.wal.data);
const checkpoints = computed(() => snap.value?.checkpoints.data);
const sizes = computed(() => snap.value?.relation_sizes.data);
const pageData = computed(() => filteredIoWalPage(
  snap.value, search.value, page.value, pageSize.value, sizeSort.value, sizeDescending.value,
));
const scopeWarnings = computed(() => {
  const s = snap.value;
  if (!s) return [];
  const items: string[] = [];
  for (const [title, section] of [
    [t('ioWal.dbStats'), s.database_stats], [t('ioWal.serverWal'), s.wal],
    [t('ioWal.checkpoints'), s.checkpoints], [t('ioWal.dbSize'), s.database_size_bytes],
    [t('ioWal.objectSizes'), s.relation_sizes],
  ] as const) {
    if (section.error) items.push(`${title}: ${section.error}`);
  }
  if (!s.overview.track_io_timing) items.push(t('ioWal.trackIo'));
  if (s.overview.track_wal_io_timing === false) items.push(t('ioWal.trackWal'));
  if (sizes.value?.truncated) items.push(t('ioWal.truncated', {sampled: String(sizes.value.sampled_relation_count),total: String(sizes.value.total_relation_count)}));
  if (wal.value && wal.value.wal_bytes >= 2 ** 53) items.push(t('ioWal.walPrecision'));
  return items;
});
function number(value: number | null | undefined): string {
  return value === null || value === undefined || !Number.isFinite(value) ? "—" : new Intl.NumberFormat(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 1 }).format(value);
}
function bytes(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return "—";
  const n = Math.abs(value);
  const unit = n < 1024 ? 1 : n < 1024 ** 2 ? 1024 : n < 1024 ** 3 ? 1024 ** 2 : n < 1024 ** 4 ? 1024 ** 3 : 1024 ** 4;
  const suffix = unit === 1 ? t('ioWal.byte') : unit === 1024 ? t('ioWal.kib') : unit === 1024 ** 2 ? t('ioWal.mib') : unit === 1024 ** 3 ? t('ioWal.gib') : t('ioWal.tib');
  return `${(value / unit).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 1 })} ${suffix}`;
}
function time(value: number | null): string { return value === null || !Number.isFinite(value) ? "—" : new Date(value).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US"); }
function onConnection(e: Event): void {
  const id = Number((e.target as HTMLSelectElement).value);
  controller.selectConnection(Number.isSafeInteger(id) && id > 0 ? id : null);
  page.value = 1;
}
function bootstrap(): void {
  if (props.shell.screen === "workspace" && !props.shell.busy && props.shell.currentUsername && !state.connections.length) {
    void controller.refreshConnections().then(() => controller.loadReference());
  }
}
watch([search, pageSize], () => { page.value = 1; });
watch(() => props.shell.authGeneration, () => { controller.reset(); bootstrap(); });
watch(() => props.shell.screen, screen => { if (screen !== "workspace") controller.reset(); else bootstrap(); });
watch(() => props.shell.busy, bootstrap);
watch(() => props.visible, visible => {
  if (!visible) return;
  if (state.connections.length && props.shell.screen === "workspace" && !props.shell.busy)
    void controller.refreshConnections();
  else bootstrap();
});
onMounted(bootstrap);
onUnmounted(() => controller.dispose());
</script>

<template>
<section class="io-view" :aria-label="t('ioWal.aria')">
  <header class="section-head">
    <div><h1>{{ t('ioWal.text000') }} <small>· I/O & WAL</small></h1>
      <p class="muted">{{ t('ioWal.text001') }}</p></div>
    <span class="tag">{{ t('ioWal.text002') }}</span>
  </header>
  <div class="filters">
    <label>{{ t('ioWal.text003') }}
      <select :value="state.connectionId ?? ''" @change="onConnection"><option value="">{{ t('ioWal.text004') }}</option>
        <option v-for="c in state.connections" :key="c.id" :value="c.id">{{ c.name }} · {{ c.host }}:{{ c.port }}/{{ c.database }}</option></select>
    </label>
    <button type="button" :disabled="state.connectionId === null || state.collecting || state.loading" @click="controller.collect">{{ state.collecting ? t('ioWal.collecting') : t('ioWal.collect') }}</button>
  </div>
  <p class="muted note">{{ t('ioWal.text005') }}</p>
  <p v-if="state.error" role="alert" class="error">{{ state.error }}</p>
  <p v-if="state.notice" class="notice">{{ state.notice }}</p>
  <p v-if="state.loading && !state.snapshot" class="muted">{{ t('ioWal.text006') }}</p>
  <p v-if="state.connectionId !== null && !state.snapshot && !state.loading" class="muted">{{ t('ioWal.text007') }}</p>

  <template v-if="snap">
    <section class="panel">
      <div class="section-head"><div><h2>{{ t('ioWal.text008') }}</h2>
        <p class="muted">{{ t('ioWal.checkLabel', {id: String(state.snapshotId ?? '—'),time: time(selectedSummary?.collection_completed_at_ms ?? null),database: snap.overview.database,version: snap.overview.server_version}) }}</p></div>
        <span class="tag">{{ t('ioWal.text009') }}</span></div>
      <p v-if="historicalProfileChanged" class="warning">{{ t('ioWal.text010') }}</p>
      <div class="metrics">
        <article><span>{{ t('ioWal.text011') }}</span><strong>{{ bytes(db?.temp_bytes) }}</strong>
          <small>{{ t('ioWal.text012') }}</small></article>
        <article><span>{{ t('ioWal.text013') }}</span><strong>{{ bytes(snap.database_size_bytes.data) }}</strong><small>{{ t('ioWal.text014') }}</small></article>
        <article><span>{{ t('ioWal.text015') }}</span><strong>{{ bytes(wal?.wal_bytes) }}</strong><small>{{ t('ioWal.text016') }}</small></article>
        <article><span>{{ t('ioWal.text017') }}</span><strong>{{ checkpoints ? number(checkpoints.timed + checkpoints.requested) : '—' }}</strong><small>{{ t('ioWal.checkpointTotals', {timed: number(checkpoints?.timed),requested: number(checkpoints?.requested)}) }}</small></article>
      </div>
      <div class="assessment">
        <h3>{{ t('ioWal.text018') }}</h3>
        <p>{{ t('ioWal.text019') }}</p>
        <p v-if="!scopeWarnings.length" class="muted">{{ t('ioWal.text020') }}</p>
        <p v-else class="warning">{{ t('ioWal.scopeIssues', {count: String(scopeWarnings.length)}) }}</p>
      </div>
      <div v-if="scopeWarnings.length" class="quality" role="note"><h3>{{ t('ioWal.text021') }}</h3>
        <ul><li v-for="(warning, i) in scopeWarnings" :key="i">{{ warning }}</li></ul>
      </div>
    </section>

    <section class="panel"><h2>{{ t('ioWal.text022') }}</h2>
      <p class="muted">{{ t('ioWal.text023') }}</p>
      <p v-if="snap.database_stats.error" class="warning">{{ t('ioWal.sectionError') }} {{ snap.database_stats.error }}</p>
      <div v-if="db" class="metrics">
        <article><span>{{ t('ioWal.text024') }}</span><strong>{{ number(db.temp_files) }}</strong><small>{{ t('ioWal.volume') }} {{ bytes(db.temp_bytes) }}</small></article>
        <article><span>{{ t('ioWal.text025') }}</span><strong>{{ number(db.blks_read) }} / {{ number(db.blks_hit) }}</strong><small>{{ t('ioWal.text026') }}</small></article>
        <article><span>{{ t('ioWal.text027') }}</span><strong>{{ number(db.xact_commit) }} / {{ number(db.xact_rollback) }}</strong></article>
        <article><span>{{ t('ioWal.text028') }}</span><strong>{{ number(db.deadlocks) }} / {{ number(db.conflicts) }}</strong></article>
      </div>
      <p v-if="db && snap.overview.track_io_timing" class="muted">{{ t('ioWal.readWrite') }} {{ number(db.blk_read_time_ms) }} / {{ number(db.blk_write_time_ms) }} {{ t('diagResidual.iomillis') }}.</p>
      <p v-else-if="db" class="warning">{{ t('ioWal.text029') }}</p>
    </section>

    <section class="panel"><h2>{{ t('ioWal.text030') }}</h2>
      <p class="muted">{{ t('ioWal.text031') }}</p>
      <p v-if="snap.wal.error" class="warning">{{ t('ioWal.sectionError') }} {{ snap.wal.error }}</p>
      <div v-if="wal" class="metrics">
        <article><span>{{ t('ioWal.text032') }}</span><strong>{{ bytes(wal.wal_bytes) }}</strong><small>{{ t('ioWal.text033') }}</small></article>
        <article><span>{{ t('ioWal.text034') }}</span><strong>{{ number(wal.wal_records) }}</strong></article>
        <article><span>{{ t('ioWal.text035') }}</span><strong>{{ number(wal.wal_fpi) }}</strong></article>
        <article><span>{{ t('ioWal.text036') }}</span><strong>{{ number(wal.wal_buffers_full) }}</strong></article>
      </div>
      <p v-if="wal && snap.overview.track_wal_io_timing" class="muted">{{ t('ioWal.walTime') }} {{ number(wal.wal_write_time_ms) }} / {{ number(wal.wal_sync_time_ms) }} {{ t('diagResidual.iomillis') }}.</p>
      <p v-else-if="wal" class="muted">{{ t('ioWal.text037') }}</p>
    </section>

    <section class="panel"><h2>{{ t('ioWal.text038') }}</h2>
      <p class="muted">{{ t('ioWal.text039') }}</p>
      <p v-if="snap.checkpoints.error" class="warning">{{ t('ioWal.sectionError') }} {{ snap.checkpoints.error }}</p>
      <div v-if="checkpoints" class="metrics">
        <article><span>{{ t('ioWal.text040') }}</span><strong>{{ number(checkpoints.timed) }}</strong></article>
        <article><span>{{ t('ioWal.text041') }}</span><strong>{{ number(checkpoints.requested) }}</strong></article>
        <article><span>{{ t('ioWal.text042') }}</span><strong>{{ number(checkpoints.write_time_ms) }} / {{ number(checkpoints.sync_time_ms) }} {{ t('diagResidual.iomillis') }}</strong></article>
        <article><span>{{ t('ioWal.text043') }}</span><strong>{{ number(checkpoints.buffers_written) }}</strong></article>
      </div>
      <p v-if="checkpoints" class="muted">{{ t('diagResidual.ioSource') }} {{ checkpoints.source }} · checkpoint_timeout {{ snap.overview.checkpoint_timeout }} · max_wal_size {{ snap.overview.max_wal_size }}</p>
    </section>

    <section class="panel"><div class="section-head"><div><h2>{{ t('ioWal.text044') }}</h2>
      <p class="muted">{{ t('ioWal.text045') }}</p></div></div>
      <p v-if="snap.relation_sizes.error" class="warning">{{ t('ioWal.sizesError') }} {{ snap.relation_sizes.error }}</p>
      <p v-if="sizes" class="muted">{{ t('ioWal.sizesCount', {count: String(sizes.sampled_relation_count), total: String(sizes.total_relation_count)}) }}{{ sizes.truncated ? t('ioWal.partial') : '' }}</p>
      <div v-if="sizes" class="filters"><label>{{ t('ioWal.text046') }} <input v-model="search" placeholder="public.orders, OID…" /></label>
        <label>{{ t('ioWal.text047') }} <select v-model.number="pageSize"><option :value="10">10</option><option :value="25">25</option><option :value="50">50</option></select></label></div>
      <p v-if="sizes" class="muted">{{ t('ioWal.text048') }}</p>
      <div v-if="sizes" class="table-scroll"><table><thead><tr>
        <th><button class="size-sort" type="button" @click="sortSizes('name')">{{ t('diagResidual.ioobject') }}{{ sortLabel('name') }}</button></th>
        <th><button class="size-sort" type="button" @click="sortSizes('total')">{{ t('diagResidual.iototal') }}{{ sortLabel('total') }}</button></th>
        <th><button class="size-sort" type="button" @click="sortSizes('heap')">{{ t('diagResidual.ioheap') }}{{ sortLabel('heap') }}</button></th>
        <th><button class="size-sort" type="button" @click="sortSizes('indexes')">{{ t('diagResidual.ioindexes') }}{{ sortLabel('indexes') }}</button></th>
        <th><button class="size-sort" type="button" @click="sortSizes('toast')">TOAST{{ sortLabel('toast') }}</button></th>
      </tr></thead><tbody>
        <tr v-for="item in pageData.rows" :key="item.relation_oid"><td><strong>{{ item.schema_name }}.{{ item.relation_name }}</strong><small>{{ item.relation_type }} · OID {{ item.relation_oid }}</small></td>
          <td>{{ bytes(item.total_size_bytes) }}</td><td>{{ bytes(item.heap_size_bytes) }}</td><td>{{ bytes(item.indexes_size_bytes) }}</td><td>{{ bytes(item.toast_size_bytes) }}</td></tr>
        <tr v-if="!pageData.rows.length"><td colspan="5">{{ t('ioWal.text049') }}</td></tr>
      </tbody></table></div>
      <div v-if="sizes && pageData.total" class="pager"><button type="button" class="secondary" :disabled="pageData.page <= 1" @click="page = pageData.page - 1">{{ t('ioWal.text050') }}</button>
        <span>{{ t('diagResidual.iofound', { page: String(pageData.page), pages: String(pageData.pages), total: String(pageData.total) }) }}</span>
        <button type="button" class="secondary" :disabled="pageData.page >= pageData.pages" @click="page = pageData.page + 1">{{ t('ioWal.text051') }}</button></div>
      <button v-if="sizes" type="button" class="secondary" @click="emit('openHistory', state.connectionId)">{{ t('ioWal.text052') }}</button>
    </section>
  </template>

  <section class="panel"><div class="section-head"><div><h2>{{ t('ioWal.text053') }}</h2><p class="muted">{{ t('ioWal.text054') }}</p></div>
    </div>
    <details :open="state.histories.length > 0 && state.histories.length <= 3" class="technical-note"><summary>{{ t('diagResidual.ioSnapshotList', { count: String(state.histories.length) }) }}</summary>
      <div class="table-scroll"><table><thead><tr><th>№</th><th>{{ t('ioWal.text055') }}</th><th>{{ t('ioWal.text056') }}</th><th>{{ t('ioWal.text057') }}</th><th>{{ t('ioWal.text058') }}</th><th></th></tr></thead><tbody>
        <tr v-for="s in state.histories" :key="s.id"><td>№{{ s.id }}</td><td>{{ time(s.collection_completed_at_ms) }}</td><td>{{ bytes(s.database_size_bytes) }}</td><td>{{ bytes(s.wal_bytes) }}</td><td>{{ s.unavailable_sections }}</td>
          <td><button type="button" class="secondary" :disabled="state.loading || state.collecting" @click="controller.loadSnapshot(s.id)">{{ s.id === state.snapshotId ? t('ioWal.opened') : t('ioWal.open') }}</button></td></tr>
        <tr v-if="!state.histories.length"><td colspan="6">{{ t('ioWal.text059') }}</td></tr>
      </tbody></table></div>
    </details>
    <button type="button" class="secondary" @click="emit('openHistory',state.connectionId)">{{ t('ioWal.text060') }}</button>
    <p class="muted note">{{ t('ioWal.text061') }}</p>
  </section>
  <details class="panel technical-note"><summary>{{ t('ioWal.text062') }}</summary>
    <p class="muted">{{ t('ioWal.text063') }}</p>
    <template v-if="snap"><h3>{{ t('ioWal.text064') }}</h3><div v-for="item in snap.interpretation.items" :key="item.id"><strong>{{ item.title }}</strong>
      <p v-for="(line, i) in item.meaning" :key="i">{{ line }}</p><p v-for="(line, i) in item.next_steps" :key="i">{{ t('ioWal.nextStep') }} {{ line }}</p></div>
      <h3>{{ t('ioWal.text065') }}</h3><ul><li v-for="(item, i) in snap.interpretation.limitations" :key="i">{{ item }}</li></ul></template>
    <h3>{{ t('ioWal.text066') }}</h3><p v-for="item in state.reference?.methodology ?? []" :key="item">{{ item }}</p>
    <p v-for="source in state.reference?.sources ?? []" :key="source.id"><strong>{{ source.title }}</strong> · {{ source.purpose }}
      <a v-if="ioWalDocsUrl(snap?.overview.server_version_num ?? null, source.path)" :href="ioWalDocsUrl(snap?.overview.server_version_num ?? null, source.path) ?? undefined" target="_blank" rel="noopener noreferrer">{{ t('ioWal.text066') }}</a></p>
  </details>
</section>
</template>

<style scoped>
.io-view{display:grid;gap:1rem;min-width:0}
h1,h2{margin:0}h3{margin:.25rem 0 .65rem}h1 small{font-size:.6em;color:var(--text-secondary);font-weight:500}
.panel{border:1px solid var(--border-strong);border-radius:.75rem;background:var(--surface);padding:1rem;min-width:0}
.section-head{display:flex;align-items:flex-start;justify-content:space-between;flex-wrap:wrap;gap:1rem}
.muted{color:var(--text-secondary);font-size:.88rem;line-height:1.45}.note{margin:.2rem 0}
.tag{border:1px solid var(--border-strong);background:var(--surface-2);padding:.35rem .65rem;border-radius:.5rem;font-size:.82rem}
.filters{display:flex;align-items:end;flex-wrap:wrap;gap:.7rem;margin-top:.3rem}
.filters label{display:grid;gap:.35rem;flex:1;min-width:190px}
.filters select,.filters input{width:100%;box-sizing:border-box;padding:.7rem;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.45rem}
.secondary{background:var(--button-secondary);color:var(--button-secondary-text)}
.metrics{display:grid;grid-template-columns:repeat(auto-fit,minmax(205px,1fr));gap:.75rem;margin:1rem 0}
.metrics article{border:1px solid var(--border);border-radius:.6rem;background:var(--surface-2);padding:.9rem;display:grid;gap:.4rem;align-content:start}
.metrics span{font-size:.83rem;color:var(--text-secondary)}.metrics strong{font-size:1.25rem}.metrics small{color:var(--text-secondary);font-size:.78rem}
.assessment,.quality{padding:1rem;border:1px solid var(--border);border-radius:.55rem;background:var(--surface-2);margin-top:1rem}
.assessment p,.quality p{margin:.3rem 0;line-height:1.5}.quality ul{margin:.3rem 0;padding-left:1.2rem;font-size:.85rem;line-height:1.5}
.warning{color:var(--message);font-size:.9rem}.error{color:var(--error)}.notice{padding:.7rem 1rem;background:var(--surface-2);border-radius:.5rem}
.table-scroll{overflow:auto;max-height:570px;margin-top:.65rem}.table-scroll table{border-collapse:collapse;width:100%;font-size:.85rem}
.table-scroll td,.table-scroll th{padding:.65rem;border-bottom:1px solid var(--border);text-align:left;vertical-align:top}
.table-scroll th{position:sticky;top:0;background:var(--surface)}.size-sort{border:0;background:none;color:inherit;font:inherit;font-weight:650;cursor:pointer;padding:0;text-align:left;white-space:nowrap}.size-sort:hover{text-decoration:underline}.table-scroll small{display:block;color:var(--text-secondary);font-size:.75rem;margin-top:.2rem}
.pager{display:flex;align-items:center;justify-content:center;gap:1rem;margin-top:.7rem}.technical-note summary{cursor:pointer;font-weight:600}
.technical-note p{font-size:.86rem;line-height:1.45}.compare-area{padding-top:1rem;margin-top:1rem;border-top:1px solid var(--border)}
@media(max-width:680px){.metrics{grid-template-columns:repeat(2,minmax(0,1fr))}}
</style>
