<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed, ref, shallowRef, watch } from "vue";
import HistorySortTh from "./HistorySortTh.vue";
import { useHistorySorting } from "./use-history-sorting.js";
import { historySortValue } from "./sort-rows.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { formatHistoryTime } from "./model.js";
import { avIntervalLabel, avTrendLines, needsAvAttention, rankAutovacuum,
  type AvHistoryInterval, type AvHistoryRelation, type AvMetric, type AvRanking,
  type AvWorkloadHistory } from "./autovacuum-insights.js";

const historySort = useHistorySorting();
const props = defineProps<{
  connectionId: number | null; invoke: PgXrayInvoke; active: boolean;
  preferredRelationOid?: number | null; navigationToken?: number;
}>();
const emit = defineEmits<{openAnalyzer: [relationOid: number | null]} >();
interface RelationPoint {
  snapshot_id: number; collection_started_at_ms: number; stats_reset: string | null;
  table: {dead_tuples:number; dead_tuple_trigger:number; last_autovacuum:string|null;
    autovacuum_count:number; last_autoanalyze:string|null; autoanalyze_count:number;
    tuples_inserted:number; tuples_updated:number; tuples_deleted:number;};
}
interface RelationHistory {
  relation_oid: number; relation_name: string;
  points: RelationPoint[];
  summary: {stats_period_continuous:boolean; inserts_per_hour:number|null;
    updates_per_hour:number|null; deletes_per_hour:number|null; vacuum_cycles:number|null;
    analyze_cycles:number|null; history_points:number};
  limitations: string[];
}
const data = shallowRef<AvWorkloadHistory|null>(null);
const error = ref(""); const busy = ref(false);
const anchorId = ref<number|null>(null);
const selectedOid = ref<number|null>(null);
const detail = shallowRef<RelationHistory|null>(null);
const detailError = ref(""); const detailBusy = ref(false);
const ranking = ref<AvRanking>("attention"); const search = ref("");
const page = ref(1); const pageSize = 15;
const metric = ref<AvMetric>("updates_per_hour");
let requestId = 0; let detailRequestId = 0;

const intervals = computed(() => data.value?.intervals ?? []);
const selectedInterval = computed(() => intervals.value.find(row => row.to_snapshot_id === data.value?.selected_to_snapshot_id) ?? null);
const comparableCount = computed(() => intervals.value.filter(row => row.state === "comparable").length);
const attentionCount = computed(() => data.value?.relations.filter(needsAvAttention).length ?? 0);
const noAttentionFallback = computed(() => ranking.value === "attention" && !search.value.trim() && attentionCount.value === 0);
const ranked = computed(() => rankAutovacuum(data.value?.relations ?? [], noAttentionFallback.value ? "dead" : ranking.value, search.value));
const sortedRelations = computed(() => historySort.sorted("av-relations", noAttentionFallback.value ? ranked.value.slice(0, 10) : ranked.value, (row, column) => {
  if (column === "attention") return avStatus(row);
  if (column === "dml_total") return row.delta && [row.delta.inserts_delta,row.delta.updates_delta,row.delta.deletes_delta].every(v=>v!=null)
    ? row.delta.inserts_delta! + row.delta.updates_delta! + row.delta.deletes_delta! : null;
  if (column === "vacuum_total") {
    const counters=[row.delta?.vacuum_count_delta,row.delta?.autovacuum_count_delta,row.delta?.analyze_count_delta,row.delta?.autoanalyze_count_delta];
    return counters.every(v=>v!=null) ? counters.reduce<number>((sum,v)=>sum+(v ?? 0),0) : null;
  }
  return historySortValue(row,column);
}));
const rows = computed(() => sortedRelations.value.slice((page.value-1)*pageSize,page.value*pageSize));
const pages = computed(() => Math.max(1,Math.ceil((noAttentionFallback.value ? Math.min(10,ranked.value.length) : ranked.value.length)/pageSize)));
const trend = computed(() => avTrendLines(intervals.value,metric.value));
const plotLines = computed(() => trend.value.lines.filter(line => line.length>1).map(line => line.map(pt => `${pt.x},${pt.y}`).join(" ")));
const chartText = (value:number): string => metric.value === "dead_tuples_estimate" ? fmt(value) : t('historyResidual.avHourValue', { value: fmt(value) });
const fmt = (n:number|null|undefined): string => n==null || !Number.isFinite(n) ? "—" : n.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US",{maximumFractionDigits:1});
const percent = (n:number|null|undefined):string => n==null || !Number.isFinite(n) ? "—" : `${fmt(n)}%`;
function bytes(n:number):string {
  const d = Math.abs(n) >= 1024**3 ? 1024**3 : Math.abs(n) >= 1024**2 ? 1024**2 : 1024;
  return `${fmt(n/d)} ${d===1024**3 ? t("historyResidual.avGib") : d===1024**2 ? t("historyResidual.avMib") : t("historyResidual.avKib")}`;
}
const cycles = (manual:number|null|undefined,auto:number|null|undefined):string =>
  manual==null || auto==null ? "—" : fmt(manual+auto);
const avStatus = (row:AvHistoryRelation):string =>
  row.autovacuum_applicable&&!row.autovacuum_enabled?t('historyResidual.avOff'):
  row.dead_trigger_reached?'VACUUM dead':row.insert_trigger_reached?'VACUUM insert':
  row.analyze_trigger_reached?'ANALYZE':t('historyResidual.avMonitoring');
const detailLabel = (value:string|null|undefined):string => value || t('historyResidual.avUnknown');
function intervalTitle(row:AvHistoryInterval): string {
  return `#${row.from_snapshot_id} → #${row.to_snapshot_id} · ${formatHistoryTime(row.from_ms)} — ${formatHistoryTime(row.to_ms)}`;
}
function invalidateDetail():void {++detailRequestId; selectedOid.value=null;detail.value=null;detailError.value="";detailBusy.value=false;}
async function load():Promise<void> {
  const id = props.connectionId, call = ++requestId;
  invalidateDetail(); data.value=null;error.value="";
  if (!props.active || id===null) {busy.value=false;return;}
  busy.value=true;
  try {
    const result=await props.invoke<AvWorkloadHistory>("autovacuum_workload_history",{
      connectionId:id,limit:60,anchorSnapshotId:anchorId.value,
    });
    if (call!==requestId || !props.active) return;
    data.value=result;anchorId.value=result.selected_to_snapshot_id;
    if (props.preferredRelationOid != null && result.relations.some(r=>r.relation_oid===props.preferredRelationOid))
      void openRelation(props.preferredRelationOid);
  } catch(e) {if(call===requestId) error.value=String(e);}
  finally {if(call===requestId) busy.value=false;}
}
async function openRelation(oid:number):Promise<void> {
  const snapshotId=data.value?.selected_to_snapshot_id;
  if (snapshotId==null) return;
  const call=++detailRequestId;
  selectedOid.value=oid;detail.value=null;detailError.value="";detailBusy.value=true;
  try {
    const result=await props.invoke<RelationHistory>("autovacuum_relation_history",{
      snapshotId,relationOid:oid,limit:80,
    });
    if (call===detailRequestId && props.active && selectedOid.value===oid) detail.value=result;
  } catch(e) {if(call===detailRequestId) detailError.value=String(e);}
  finally {if(call===detailRequestId) detailBusy.value=false;}
}
watch(() => [props.connectionId,props.active] as const, () => {anchorId.value=null;void load();}, {immediate:true});
watch(() => props.navigationToken, (next,prev) => {if(next!==prev) {anchorId.value=null;void load();}});
watch([ranking,search,()=>historySort.get("av-relations")],()=>{page.value=1;});
function selectAnchor():void {void load();}
</script>

<template>
<section class="avhist" :aria-label="t('historyResidual.avAria')">
  <header class="avhist-head"><div><h2>{{ t('historyDetails.AutovacuumHeading') }}</h2>
    <p class="muted">{{ t('historyDetails.AutovacuumDescription') }}</p></div>
    <button type="button" class="secondary" @click="emit('openAnalyzer',selectedOid)">{{ t('historyDetails.AutovacuumOpen') }}</button>
  </header>
  <p v-if="error" role="alert" class="avhist-error">{{error}}</p>
  <p v-if="busy" class="muted">{{ t('historyDetails.AutovacuumLoading') }}</p>
  <template v-if="data && !busy">
    <div v-if="!data.snapshot_count" class="avhist-note">{{ t('historyDetails.AutovacuumEmpty') }}</div>
    <template v-else>
      <div class="avhist-cards">
        <article><small>{{ t('historyDetails.AutovacuumSnapshots') }}</small><strong>{{data.snapshot_count}}</strong><small>{{ t('historyDetails.AutovacuumLast60') }}</small></article>
        <article><small>{{ t('historyDetails.AutovacuumIntervals') }}</small><strong>{{comparableCount}} / {{intervals.length}}</strong><small>{{ t('historyDetails.AutovacuumResets') }}</small></article>
        <article><small>{{ t('historyDetails.AutovacuumDeadTuples') }}</small><strong>{{fmt(data.latest_dead_tuples_estimate)}}</strong><small>{{ t('historyDetails.AutovacuumNotBloat') }}</small></article>
        <article><small>{{ t('historyDetails.AutovacuumAttention') }}</small><strong>{{attentionCount}}</strong><small>{{ t('historyDetails.AutovacuumAttentionHint') }}</small></article>
      </div>
      <div class="avhist-panel">
        <div class="avhist-head"><div><h3>{{ t('historyDetails.AutovacuumTrend') }}</h3><p class="muted">{{ t('historyDetails.AutovacuumTrendHint') }}</p></div>
          <label>{{ t('historyDetails.AutovacuumMetric') }}<select v-model="metric">
            <option value="updates_per_hour">{{ t('historyDetails.AutovacuumUpdateRate') }}</option><option value="inserts_per_hour">{{ t('historyDetails.AutovacuumInsertRate') }}</option>
            <option value="deletes_per_hour">{{ t('historyDetails.AutovacuumDeleteRate') }}</option><option value="vacuum_cycles_per_hour">{{ t('historyDetails.AutovacuumVacuumRate') }}</option>
            <option value="analyze_cycles_per_hour">{{ t('historyDetails.AutovacuumAnalyzeRate') }}</option><option value="dead_tuples_estimate">{{ t('historyDetails.AutovacuumDeadTuples') }}</option>
          </select></label></div>
        <svg class="avhist-chart" viewBox="0 0 880 225" role="img" :aria-label="t('historyResidual.avTrendAria')">
          <line x1="80" y1="180" x2="830" y2="180" stroke="var(--border-strong)"/>
          <text x="80" y="25" fill="var(--text)" font-size="13">{{chartText(trend.max)}}</text>
          <text x="62" y="185" fill="var(--text)" font-size="12" text-anchor="end">0</text>
          <polyline v-for="(line,i) in plotLines" :key="i" :points="line" fill="none" stroke="var(--accent)" stroke-width="2.5"/>
          <circle v-for="point in trend.points" :key="point.id" :cx="point.x" :cy="point.y" r="4" fill="var(--accent)"><title>{{formatHistoryTime(point.ms)}} · {{chartText(point.value)}}</title></circle>
          <template v-if="trend.points.length"><text x="80" y="207" fill="var(--text-secondary)" font-size="12">{{formatHistoryTime(trend.points[0]!.ms)}}</text>
            <text x="830" y="207" fill="var(--text-secondary)" font-size="12" text-anchor="end">{{formatHistoryTime(trend.points[trend.points.length-1]!.ms)}}</text></template>
        </svg>
        <p class="muted">{{ t('historyResidual.avAxis') }}</p>
      </div>
      <div class="avhist-panel">
        <h3>{{ t('historyResidual.avInspect') }}</h3>
        <label class="avhist-anchor">{{ t('historyResidual.avAnchor') }}<select v-model.number="anchorId" @change="selectAnchor">
          <option v-for="row in intervals" :key="row.to_snapshot_id" :value="row.to_snapshot_id">{{intervalTitle(row)}}</option>
        </select></label>
        <p v-if="selectedInterval" :class="{'avhist-error':selectedInterval.state!=='comparable'}">{{avIntervalLabel(selectedInterval.state)}}{{ t('histAvFinal.comparable') }}{{selectedInterval.comparable_relations}}{{ t('histAvFinal.objects') }}{{selectedInterval.sampled_relations}}{{ t('histAvFinal.objectsSample') }}</p>
        <div v-if="selectedInterval" class="avhist-cards avhist-detail-cards">
          <article><small>{{ t('histAvFinal.inserts') }}</small><strong>{{fmt(selectedInterval.inserts_per_hour)}}</strong></article>
          <article><small>{{ t('histAvFinal.updates') }}</small><strong>{{fmt(selectedInterval.updates_per_hour)}}</strong></article>
          <article><small>{{ t('histAvFinal.deletes') }}</small><strong>{{fmt(selectedInterval.deletes_per_hour)}}</strong></article>
          <article><small>VACUUM / ANALYZE cycles</small><strong>{{fmt(selectedInterval.vacuum_cycles)}} / {{fmt(selectedInterval.analyze_cycles)}}</strong><small>{{ t('historyResidual.avRates') }}</small></article>
        </div>
        <details v-if="intervals.length"><summary>{{ t('historyResidual.avPeriodCount', { count: String(intervals.length) }) }}</summary>
          <div class="avhist-scroll"><table><thead><tr><HistorySortTh :label="t('historyResidual.avPeriod')" :column="'to_ms'" :order="historySort.get('av-intervals')" @sort="historySort.toggle('av-intervals', $event)" /><HistorySortTh :label="t('historyResidual.avQuality')" :column="'state'" :order="historySort.get('av-intervals')" @sort="historySort.toggle('av-intervals', $event)" /><HistorySortTh :label="t('vueFinalPolish.insertRate')" :column="'inserts_per_hour'" :order="historySort.get('av-intervals')" @sort="historySort.toggle('av-intervals', $event)" /><HistorySortTh :label="t('vueFinalPolish.updateRate')" :column="'updates_per_hour'" :order="historySort.get('av-intervals')" @sort="historySort.toggle('av-intervals', $event)" /><HistorySortTh label="VACUUM" :column="'vacuum_cycles'" :order="historySort.get('av-intervals')" @sort="historySort.toggle('av-intervals', $event)" /><HistorySortTh :label="t('historyResidual.avDeadEstimate')" :column="'dead_tuples_estimate'" :order="historySort.get('av-intervals')" @sort="historySort.toggle('av-intervals', $event)" /></tr></thead><tbody>
            <tr v-for="row in historySort.sorted('av-intervals',intervals)" :key="row.to_snapshot_id"><td>{{intervalTitle(row)}}</td><td>{{avIntervalLabel(row.state)}}</td><td>{{fmt(row.inserts_per_hour)}}</td><td>{{fmt(row.updates_per_hour)}}</td><td>{{fmt(row.vacuum_cycles)}}</td><td>{{fmt(row.dead_tuples_estimate)}}</td></tr>
          </tbody></table></div>
        </details>
        <ul v-if="selectedInterval?.limitations.length"><li v-for="(line,i) in selectedInterval.limitations.slice(0,5)" :key="i" class="muted">{{line}}</li></ul>
      </div>
      <div class="avhist-panel">
        <div class="avhist-head"><div><h3>{{ t('historyResidual.avTables') }}</h3><p class="muted">{{ t('historyResidual.avDeadNote') }}</p></div>
          <div class="avhist-controls"><label>{{ t('historyResidual.avSorting') }}<select v-model="ranking"><option value="attention">{{ t('historyResidual.avAttention') }}</option><option value="dead">{{ t('historyResidual.avDead') }}</option>
            <option value="dml">{{ t('historyResidual.avDml') }}</option><option value="size">{{ t('historyResidual.avSize') }}</option><option value="all">{{ t('historyResidual.avAll') }}</option></select></label>
            <label>{{ t('historyResidual.avSearch') }}<input v-model="search" :placeholder="t('historyResidual.avSearchHint')" /></label></div></div>
        <p v-if="noAttentionFallback" class="muted">{{ t('historyResidual.avFallback') }}</p>
        <p v-if="data.relation_count > data.relations.length" class="muted">{{ t('historyResidual.avCount', { shown: String(data.relations.length), total: String(data.relation_count) }) }}</p>
        <div v-if="!rows.length" class="muted">{{ t('historyResidual.avEmpty') }}</div>
        <div v-else class="avhist-scroll"><table><thead><tr><HistorySortTh :label="t('historyResidual.avTable')" :column="'relation_name'" :order="historySort.get('av-relations')" @sort="historySort.toggle('av-relations', $event)" /><HistorySortTh :label="t('historyResidual.avDeadThreshold')" :column="'dead_trigger_used_pct'" :order="historySort.get('av-relations')" @sort="historySort.toggle('av-relations', $event)" /><HistorySortTh :label="t('historyResidual.avStatus')" :column="'attention'" :order="historySort.get('av-relations')" @sort="historySort.toggle('av-relations', $event)" /><HistorySortTh :label="t('historyResidual.avDmlInterval')" :column="'dml_total'" :order="historySort.get('av-relations')" @sort="historySort.toggle('av-relations', $event)" /><HistorySortTh label="VACUUM / ANALYZE" :column="'vacuum_total'" :order="historySort.get('av-relations')" @sort="historySort.toggle('av-relations', $event)" /><th></th></tr></thead><tbody>
          <tr v-for="row in rows" :key="row.relation_oid"><td><strong>{{row.relation_name}}</strong><small>OID {{row.relation_oid}} · {{bytes(row.total_size_bytes)}}</small></td>
            <td>{{fmt(row.dead_tuples)}} / {{fmt(row.dead_tuple_trigger)}}<small>{{percent(row.dead_trigger_used_pct)}}</small></td>
            <td>{{avStatus(row)}}</td>
            <td>{{fmt(row.delta?.inserts_delta)}} / {{fmt(row.delta?.updates_delta)}} / {{fmt(row.delta?.deletes_delta)}}<small>INSERT / UPDATE / DELETE</small></td>
            <td>{{cycles(row.delta?.vacuum_count_delta,row.delta?.autovacuum_count_delta)}} / {{cycles(row.delta?.analyze_count_delta,row.delta?.autoanalyze_count_delta)}}</td>
            <td><button class="secondary" type="button" @click="openRelation(row.relation_oid)">{{ t('historyResidual.avHistory') }}</button></td></tr>
        </tbody></table></div>
        <div v-if="pages>1" class="avhist-pagination"><span>{{ t('historyResidual.avPage', { page: String(page), pages: String(pages) }) }}</span><button type="button" class="secondary" :disabled="page<=1" @click="page--">{{ t('historyResidual.avPrevious') }}</button><button type="button" class="secondary" :disabled="page>=pages" @click="page++">{{ t('historyResidual.avNext') }}</button></div>
      </div>
      <div v-if="selectedOid != null" class="avhist-panel">
        <div class="avhist-head"><h3>{{ t('historyResidual.avHistoryTitle', { oid: String(selectedOid) }) }}</h3><button type="button" class="secondary" @click="emit('openAnalyzer',selectedOid)">{{ t('historyResidual.avAnalyzer') }}</button></div>
        <p v-if="detailBusy" class="muted">{{ t('historyResidual.avDetailLoading') }}</p><p v-if="detailError" role="alert" class="avhist-error">{{detailError}}</p>
        <template v-if="detail"><p>{{detail.relation_name}} · {{ t('historyResidual.avDataPoints', { count: String(detail.summary.history_points) }) }} · {{detail.summary.stats_period_continuous?t('historyResidual.avContinuous'):t('historyResidual.avNotContinuous')}}</p>
          <p class="muted">{{ t('historyResidual.avRatesPerHour', { insert: fmt(detail.summary.inserts_per_hour), update: fmt(detail.summary.updates_per_hour), delete: fmt(detail.summary.deletes_per_hour) }) }}</p>
          <div class="avhist-scroll"><table><thead><tr><HistorySortTh :label="t('historyResidual.avSnapshot')" :column="'collection_started_at_ms'" :order="historySort.get('av-detail')" @sort="historySort.toggle('av-detail', $event)" /><HistorySortTh :label="t('historyResidual.avEstimate')" :column="'table.dead_tuples'" :order="historySort.get('av-detail')" @sort="historySort.toggle('av-detail', $event)" /><HistorySortTh :label="t('historyResidual.avThreshold')" :column="'table.dead_tuple_trigger'" :order="historySort.get('av-detail')" @sort="historySort.toggle('av-detail', $event)" /><HistorySortTh label="Autovacuum count" :column="'table.autovacuum_count'" :order="historySort.get('av-detail')" @sort="historySort.toggle('av-detail', $event)" /><HistorySortTh label="Autoanalyze count" :column="'table.autoanalyze_count'" :order="historySort.get('av-detail')" @sort="historySort.toggle('av-detail', $event)" /><HistorySortTh :label="t('historyResidual.avLast')" :column="'table.last_autovacuum'" :order="historySort.get('av-detail')" @sort="historySort.toggle('av-detail', $event)" /></tr></thead><tbody>
            <tr v-for="pt in historySort.sorted('av-detail',detail.points)" :key="pt.snapshot_id"><td>#{{pt.snapshot_id}} · {{formatHistoryTime(pt.collection_started_at_ms)}}</td><td>{{fmt(pt.table.dead_tuples)}}</td><td>{{fmt(pt.table.dead_tuple_trigger)}}</td><td>{{fmt(pt.table.autovacuum_count)}}</td><td>{{fmt(pt.table.autoanalyze_count)}}</td><td>{{detailLabel(pt.table.last_autovacuum)}}</td></tr>
          </tbody></table></div>
          <details v-if="detail.limitations.length"><summary>{{ t('historyResidual.avLimits', { count: String(detail.limitations.length) }) }}</summary><ul><li v-for="line in detail.limitations" :key="line">{{line}}</li></ul></details>
        </template>
      </div>
      <details v-if="data.limitations.length"><summary>{{ t('historyResidual.avMethod') }}</summary><ul><li v-for="line in data.limitations" :key="line">{{line}}</li></ul></details>
    </template>
  </template>
</section>
</template>

<style scoped>
.avhist{display:grid;gap:1rem;min-width:0}.avhist-head{display:flex;justify-content:space-between;align-items:flex-start;gap:1rem;flex-wrap:wrap}.avhist-head h2,.avhist h3{margin:.15rem 0 .5rem}.avhist h2{font-size:1.35rem}.avhist h3{font-size:1.05rem}.avhist .muted,.avhist small{color:var(--text-secondary);font-size:.88rem}.avhist p{line-height:1.45}.avhist-error{color:var(--error)}.avhist-note{padding:1rem;background:var(--surface-2);border-radius:.5rem}.avhist-panel{min-width:0;padding:1rem;border:1px solid var(--border-strong);border-radius:.5rem}.avhist-cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:.65rem;margin:.7rem 0}.avhist-cards article{background:var(--surface-2);border-radius:.45rem;padding:.75rem;display:grid;gap:.25rem}.avhist-cards strong{font-size:1.25rem}.avhist-detail-cards strong{font-size:1.1rem}.avhist-chart{width:100%;height:240px;min-width:0}.avhist label{display:grid;gap:.3rem}.avhist select,.avhist input{background:var(--surface-2);color:var(--text);padding:.55rem;border:1px solid var(--border-strong);border-radius:.4rem;max-width:100%}.avhist-anchor{max-width:520px}.avhist-controls{display:flex;flex-wrap:wrap;gap:.65rem}.avhist-controls label{max-width:220px}.avhist-scroll{overflow:auto;max-height:530px}.avhist table{width:100%;border-collapse:collapse;font-size:.87rem;text-align:left}.avhist th,.avhist td{padding:.6rem .5rem;border-bottom:1px solid var(--border);vertical-align:top}.avhist td small{display:block}.avhist-pagination{display:flex;justify-content:flex-end;align-items:center;gap:.7rem;margin-top:.7rem}.avhist details{margin:.65rem 0}.avhist summary{cursor:pointer}
</style>
