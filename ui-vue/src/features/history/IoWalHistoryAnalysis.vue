<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed, ref, shallowRef, watch } from "vue";
import HistorySortTh from "./HistorySortTh.vue";
import { useHistorySorting } from "./use-history-sorting.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { formatHistoryTime } from "./model.js";
import { ioWalChart, ioWalSizeChanges, ioWalStatus,
  type IoWalHistoryInterval, type IoWalHistoryMetric, type IoWalWorkloadHistory } from "./io-wal-insights.js";

const historySort = useHistorySorting();
const props = defineProps<{connectionId: number | null; invoke: PgXrayInvoke; active: boolean}>();
const emit = defineEmits<{openAnalyzer: []}>();
const data = shallowRef<IoWalWorkloadHistory | null>(null);
const busy = ref(false), error = ref("");
const anchor = ref<number | null>(null);
const metric = ref<IoWalHistoryMetric>("wal_bytes_per_hour");
const search = ref(""), page = ref(1);
const pageSize = 15;
let request = 0;
const intervals = computed(() => data.value?.intervals ?? []);
const chosen = computed(() => intervals.value.find(row => row.to_snapshot_id === data.value?.selected_to_snapshot_id) ?? null);
const latest = computed(() => intervals.value[0] ?? null);
const fullyComparable = computed(() => intervals.value.filter(row => row.state === "comparable").length);
const partiallyComparable = computed(() => intervals.value.filter(row => row.state === "partial").length);
const plot = computed(() => ioWalChart(intervals.value, data.value?.size_points ?? [], metric.value));
const changes = computed(() => ioWalSizeChanges(data.value?.relation_size_changes ?? [], search.value));
const sortedChanges = computed(() => historySort.sorted("io-changes",changes.value));
const pages = computed(() => Math.max(1,Math.ceil(changes.value.length / pageSize)));
const visibleChanges = computed(() => sortedChanges.value.slice((page.value-1)*pageSize, page.value*pageSize));
const lastSize = computed(() => {
  const points=data.value?.size_points ?? [];
  return points.length ? points[points.length-1].database_size_bytes : null;
});
const fmt = (v: number | null | undefined): string => v == null || !Number.isFinite(v) ? "—" : v.toLocaleString("ru-RU",{maximumFractionDigits:1});
const bytes = (n: number | null | undefined): string => {
  if (n == null || !Number.isFinite(n)) return "—";
  const a=Math.abs(n), d=a>=1024**4?1024**4:a>=1024**3?1024**3:a>=1024**2?1024**2:a>=1024?1024:1;
  return `${fmt(n/d)} ${d===1024**4?t('histIoFinal.tib'):d===1024**3?t('histIoFinal.gib'):d===1024**2?t('histIoFinal.mib'):d===1024?t('histIoFinal.kib'):t('histIoFinal.b')}`;
};
const signedBytes = (n: number | null | undefined): string => n == null ? "—" : `${n>0?'+':''}${bytes(n)}`;
const metricValue = (v: number | null | undefined): string => ["wal_bytes_per_hour","temp_bytes_per_hour","database_size_bytes"].includes(metric.value) ? bytes(v) : fmt(v);
const rowTitle = (row:IoWalHistoryInterval):string => `#${row.from_snapshot_id} → #${row.to_snapshot_id} · ${formatHistoryTime(row.from_ms)} — ${formatHistoryTime(row.to_ms)}`;
const part = (value:number|null|undefined, unit:string):string => value==null ? "—" : `${fmt(value)} ${unit}`;
async function load():Promise<void> {
  const id=props.connectionId, call=++request;
  if (!props.active || id===null) {data.value=null;busy.value=false;return;}
  busy.value=true;error.value="";
  try {
    const result=await props.invoke<IoWalWorkloadHistory>("io_wal_workload_history",{
      connectionId:id,limit:60,selectedToSnapshotId:anchor.value,
    });
    if (call!==request || !props.active || props.connectionId!==id) return;
    data.value=result;anchor.value=result.selected_to_snapshot_id;
  } catch(e) {if(call===request) error.value=String(e);}
  finally {if(call===request) busy.value=false;}
}
watch(() => [props.connectionId,props.active] as const, () => {anchor.value=null;void load();}, {immediate:true});
watch([search,()=>historySort.get("io-changes")],()=>{page.value=1;});
function selectInterval():void {page.value=1;void load();}
</script>

<template>
<section class="iowhist" :aria-label="t('historyAttributes.l16')">
  <header class="iowhist-head"><div><h2>{{ t('historyDetails.IoWalHeading') }}</h2>
    <p class="muted">{{ t('historyDetails.IoWalDescription') }}</p></div>
    <button type="button" class="secondary" @click="emit('openAnalyzer')">{{ t('historyDetails.IoWalOpen') }}</button></header>
  <p v-if="error" role="alert" class="iowhist-error">{{error}}</p>
  <p v-if="busy" class="muted">{{ t('historyDetails.IoWalLoading') }}</p>
  <template v-if="data && !busy">
    <div class="iowhist-cards">
      <article><small>{{ t('historyDetails.IoWalSnapshots') }}</small><strong>{{data.snapshot_count}}</strong><small>{{ t('historyDetails.IoWalLast60') }}</small></article>
      <article><small>{{ t('historyDetails.IoWalContinuity') }}</small><strong>{{fullyComparable}}{{ t('histIoFinal.full') }}{{partiallyComparable}}{{ t('histIoFinal.partial') }}</strong><small>{{ t('histIoFinal.pairs') }}{{intervals.length}}{{ t('histIoFinal.adjacent') }}</small></article>
      <article><small>{{ t('historyDetails.IoWalWalRate') }}</small><strong>{{bytes(chosen?.wal_bytes_per_hour)}}</strong><small>{{ t('historyDetails.IoWalWholeInstance') }}</small></article>
      <article><small>{{ t('historyDetails.IoWalDbSize') }}</small><strong>{{bytes(lastSize)}}</strong><small>{{ t('historyDetails.IoWalLastMeasurement') }}</small></article>
    </div>
    <div class="iowhist-note">{{ t('historyDetails.IoWalScope') }}</div>
    <p v-if="data.snapshot_count < 2" class="muted">{{ t('historyDetails.IoWalNeedTwo') }}</p>
    <template v-if="data.snapshot_count >= 2">
      <section class="iowhist-panel">
        <div class="iowhist-head"><div><h3>{{ t('historyDetails.IoWalTrend') }}</h3><p class="muted">{{ t('historyDetails.IoWalTrendHint') }}</p></div>
          <label>{{ t('historyDetails.IoWalMetric') }}<select v-model="metric"><option value="wal_bytes_per_hour">{{ t('historyDetails.IoWalWalBytes') }}</option>
            <option value="temp_bytes_per_hour">{{ t('historyDetails.IoWalTempBytes') }}</option>
            <option value="blocks_read_per_hour">{{ t('historyDetails.IoWalReads') }}</option>
            <option value="checkpoints_requested_per_hour">{{ t('historyDetails.IoWalCheckpoints') }}</option>
            <option value="database_size_bytes">{{ t('historyDetails.IoWalDbSizeSnapshots') }}</option></select></label></div>
        <svg class="iowhist-chart" viewBox="0 0 880 260" role="img" :aria-label="t('histIoFinal.chartAria', { metric, points: String(plot.points.length) })">
          <line x1="76" y1="204" x2="816" y2="204" stroke="var(--border-strong)"/>
          <text x="76" y="25" fill="var(--text-secondary)" font-size="12">{{metricValue(plot.maximum)}}</text>
          <text x="62" y="210" fill="var(--text-secondary)" font-size="12" text-anchor="end">0</text>
          <text v-if="plot.minMs != null" x="76" y="233" fill="var(--text-secondary)" font-size="12">{{formatHistoryTime(plot.minMs)}}</text>
          <text v-if="plot.maxMs != null" x="816" y="233" text-anchor="end" fill="var(--text-secondary)" font-size="12">{{formatHistoryTime(plot.maxMs)}}</text>
          <polyline v-for="(segment,i) in plot.segments.filter(s=>s.length>1)" :key="i" :points="segment.map(p=>`${p.x},${p.y}`).join(' ')" fill="none" stroke="var(--accent)" stroke-width="2.5" />
          <circle v-for="point in plot.points" :key="point.id" :cx="point.x" :cy="point.y" r="4" :fill="point.id===data.selected_to_snapshot_id?'var(--text)':'var(--accent)'"><title>{{formatHistoryTime(point.ms)}} · {{metricValue(point.value)}}</title></circle>
        </svg>
        <p v-if="!plot.points.length" class="muted">{{ t('historyDetails.IoWalNoPoints') }}</p>
        <p class="muted">{{ t('historyDetails.IoWalTimeHint') }}</p>
      </section>
      <section class="iowhist-panel">
        <h3>{{ t('historyDetails.IoWalInterval') }}</h3>
        <label class="iowhist-selector">{{ t('historyDetails.IoWalEndSnapshot') }}<select v-model.number="anchor" :disabled="busy" @change="selectInterval">
          <option v-for="row in intervals" :key="row.to_snapshot_id" :value="row.to_snapshot_id">{{rowTitle(row)}}</option>
        </select></label>
        <template v-if="chosen">
          <p class="muted">{{ioWalStatus(chosen.state)}} · {{fmt(chosen.elapsed_seconds)}}{{ t('histIoFinal.seconds') }}</p>
          <div class="iowhist-source-grid">
            <article><h4>{{ t('historyDetails.IoWalWalInstance') }}</h4><small>{{ioWalStatus(chosen.wal_state)}}</small><strong>{{bytes(chosen.wal_bytes_per_hour)}}{{ t('ioHistFinish.perHour') }}</strong>
              <p>{{ t('histIoFinal.writes') }}{{part(chosen.wal_records_per_hour, t('ioHistFinish.perHour'))}} · WAL buffers full: {{part(chosen.wal_buffers_full_per_hour, t('ioHistFinish.perHour'))}}</p>
              <p>WAL write/sync: {{fmt(chosen.wal_write_ms_per_hour)}} / {{fmt(chosen.wal_sync_ms_per_hour)}}{{ t('histIoFinal.msPerHour') }}</p></article>
            <article><h4>{{ t('historyDetails.IoWalTempDatabase') }}</h4><small>{{ioWalStatus(chosen.database_state)}}</small><strong>{{bytes(chosen.temp_bytes_per_hour)}}{{ t('ioHistFinish.perHour') }}</strong>
              <p>Temp files: {{part(chosen.temp_files_per_hour, t('ioHistFinish.perHour'))}}</p>
              <p>Block reads/hits: {{fmt(chosen.blocks_read_per_hour)}} / {{fmt(chosen.blocks_hit_per_hour)}}{{ t('histIoFinal.blocksHour') }}</p></article>
            <article><h4>{{ t('historyDetails.IoWalCheckpointInstance') }}</h4><small>{{ioWalStatus(chosen.checkpoint_state)}}</small><strong>Requested: {{part(chosen.checkpoints_requested_per_hour, t('ioHistFinish.perHour'))}}</strong>
              <p>Timed: {{part(chosen.checkpoints_timed_per_hour, t('ioHistFinish.perHour'))}} · buffers written: {{part(chosen.checkpoint_buffers_per_hour, t('ioHistFinish.perHour'))}}</p></article>
          </div>
          <p class="muted">{{ t('histIoFinal.dbAtEnd') }}{{bytes(chosen.database_size_bytes)}}{{ t('histIoFinal.dbChange') }}{{signedBytes(chosen.database_size_delta_bytes)}}{{ t('histIoFinal.dbDeltaCaveat') }}</p>
        </template>
        <details><summary>{{ t('histIoFinal.allIntervals') }}{{intervals.length}})</summary><div class="iowhist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l18')" :column="'to_ms'" :order="historySort.get('io-intervals')" @sort="historySort.toggle('io-intervals', $event)" /><HistorySortTh label="WAL" :column="'wal_state'" :order="historySort.get('io-intervals')" @sort="historySort.toggle('io-intervals', $event)" /><HistorySortTh label="Temp / blocks" :column="'database_state'" :order="historySort.get('io-intervals')" @sort="historySort.toggle('io-intervals', $event)" /><HistorySortTh label="Checkpoint" :column="'checkpoint_state'" :order="historySort.get('io-intervals')" @sort="historySort.toggle('io-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l15')" :column="'wal_bytes_per_hour'" :order="historySort.get('io-intervals')" @sort="historySort.toggle('io-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l14')" :column="'temp_bytes_per_hour'" :order="historySort.get('io-intervals')" @sort="historySort.toggle('io-intervals', $event)" /></tr></thead><tbody>
          <tr v-for="row in historySort.sorted('io-intervals', intervals)" :key="row.to_snapshot_id"><td>{{rowTitle(row)}}</td><td>{{ioWalStatus(row.wal_state)}}</td><td>{{ioWalStatus(row.database_state)}}</td><td>{{ioWalStatus(row.checkpoint_state)}}</td><td>{{bytes(row.wal_bytes_per_hour)}}</td><td>{{bytes(row.temp_bytes_per_hour)}}</td></tr>
        </tbody></table></div></details>
      </section>
      <section class="iowhist-panel">
        <div class="iowhist-head"><div><h3>{{ t('histIoFinal.objectChanges') }}</h3><p class="muted">{{ t('histIoFinal.objectChangesDesc') }}</p></div>
          <label>{{ t('histIoFinal.search') }}<input v-model="search" :placeholder="t('historyAttributes.l19')" /></label></div>
        <p v-if="!data.relation_sizes_available" class="muted">{{ t('histIoFinal.unavailable') }}</p>
        <p v-else-if="!changes.length" class="muted">{{ t('histIoFinal.noChanges') }}</p>
        <p v-if="data.relation_sample_truncated" class="iowhist-warning">{{ t('histIoFinal.sampleTruncated') }}</p>
        <div v-if="visibleChanges.length" class="iowhist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l17')" :column="'newer_name'" :order="historySort.get('io-changes')" @sort="historySort.toggle('io-changes', $event)" /><HistorySortTh label="Δ total" :column="'total_size_delta_bytes'" :order="historySort.get('io-changes')" @sort="historySort.toggle('io-changes', $event)" /><HistorySortTh label="Δ heap" :column="'heap_size_delta_bytes'" :order="historySort.get('io-changes')" @sort="historySort.toggle('io-changes', $event)" /><HistorySortTh label="Δ indexes" :column="'indexes_size_delta_bytes'" :order="historySort.get('io-changes')" @sort="historySort.toggle('io-changes', $event)" /><HistorySortTh label="Δ TOAST" :column="'toast_size_delta_bytes'" :order="historySort.get('io-changes')" @sort="historySort.toggle('io-changes', $event)" /><HistorySortTh :label="t('historyAttributes.l11')" :column="'sample_state'" :order="historySort.get('io-changes')" @sort="historySort.toggle('io-changes', $event)" /></tr></thead><tbody>
          <tr v-for="row in visibleChanges" :key="row.relation_oid"><td><strong>{{row.newer_name ?? row.older_name}}</strong><small>OID {{row.relation_oid}}</small></td>
            <td>{{signedBytes(row.total_size_delta_bytes)}}</td><td>{{signedBytes(row.heap_size_delta_bytes)}}</td><td>{{signedBytes(row.indexes_size_delta_bytes)}}</td><td>{{signedBytes(row.toast_size_delta_bytes)}}</td><td>{{row.sample_state==='comparable'?t('histIoFinal.comparable'):row.sample_state}}</td></tr>
        </tbody></table></div>
        <div v-if="pages>1" class="iowhist-pager"><span>{{page}} / {{pages}}</span><button class="secondary" :disabled="page===1" @click="page--">{{ t('histIoFinal.previous') }}</button><button class="secondary" :disabled="page>=pages" @click="page++">{{ t('histIoFinal.next') }}</button></div>
        <small class="muted">{{ t('histIoFinal.caveat') }}</small>
      </section>
      <details v-if="data.limitations.length" class="iowhist-panel"><summary>{{ t('histIoFinal.method') }}{{data.limitations.length}})</summary><ul><li v-for="line in data.limitations" :key="line">{{line}}</li></ul></details>
    </template>
  </template>
</section>
</template>
<style scoped>
.iowhist{display:grid;gap:1rem;min-width:0}.iowhist-head{display:flex;align-items:start;justify-content:space-between;gap:1rem;flex-wrap:wrap}.iowhist h2{font-size:1.35rem;margin:.2rem 0 .5rem}.iowhist h3{font-size:1.07rem;margin:.15rem 0 .5rem}.iowhist h4{font-size:.98rem;margin:0}.iowhist p{line-height:1.4}.iowhist .muted,.iowhist small{font-size:.88rem;color:var(--text-secondary)}.iowhist-error,.iowhist-warning{color:var(--error)}.iowhist-note{padding:.8rem;background:var(--surface-2);border-radius:.4rem}.iowhist-panel{border:1px solid var(--border-strong);border-radius:.6rem;padding:1rem;min-width:0}.iowhist-cards,.iowhist-source-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(190px,1fr));gap:.7rem}.iowhist-cards article,.iowhist-source-grid article{background:var(--surface-2);border-radius:.45rem;padding:.8rem;display:grid;gap:.35rem}.iowhist-cards strong,.iowhist-source-grid strong{font-size:1.15rem}.iowhist-source-grid p{margin:.2rem 0}.iowhist-chart{width:100%;height:245px;max-width:1050px}.iowhist label{display:grid;gap:.35rem}.iowhist select,.iowhist input{background:var(--surface-2);color:var(--text);padding:.55rem;border:1px solid var(--border-strong);border-radius:.4rem;max-width:100%}.iowhist-selector{max-width:540px}.iowhist-scroll{overflow:auto;max-height:510px}.iowhist table{width:100%;border-collapse:collapse;font-size:.87rem;text-align:left}.iowhist th,.iowhist td{padding:.65rem .5rem;border-bottom:1px solid var(--border);vertical-align:top}.iowhist td small{display:block}.iowhist-pager{display:flex;justify-content:flex-end;align-items:center;gap:.7rem;margin-top:.8rem}.iowhist details{margin-top:.8rem}.iowhist summary{cursor:pointer}
</style>
