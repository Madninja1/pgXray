<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed, onUnmounted, ref, shallowRef, watch } from "vue";
import HistorySortTh from "./HistorySortTh.vue";
import { useHistorySorting } from "./use-history-sorting.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { indexLabel, type IndexComparison } from "../index-health/controller.js";
import { formatHistoryTime, type HistorySnapshot } from "./model.js";
import {
  INDEX_HISTORY_METRICS, indexHistoryChart, indexHistoryChanges, indexHistoryPairs,
  indexHistoryTotals, indexPairState, indexPairStatus, isIndexHistorySummary,
  type IndexChangeFilter, type IndexHistoryMetric,
} from "./index-health-insights.js";

const historySort = useHistorySorting();
const props=defineProps<{connectionId:number|null; snapshots:HistorySnapshot[]; invoke:PgXrayInvoke; active:boolean}>();
const emit=defineEmits<{openAnalyzer:[connectionId:number|null]}>();
const rows=computed(()=>props.snapshots.filter(isIndexHistorySummary).slice(0,60));
const pairs=computed(()=>indexHistoryPairs(rows.value));
const chosenId=ref<number|null>(null);
const chosen=computed(()=>pairs.value.find(pair=>pair.newer.id===chosenId.value) ?? null);
const state=computed(()=>chosen.value ? indexPairState(chosen.value) : null);
const metric=ref<IndexHistoryMetric>("suspiciously_unused_count");
const trend=computed(()=>indexHistoryChart(rows.value,metric.value));
const comparison=shallowRef<IndexComparison|null>(null);
const busy=ref(false),error=ref("");
const filter=ref<IndexChangeFilter>("growth"),search=ref(""),page=ref(1);
const pageSize=20;
const changes=computed(()=>indexHistoryChanges(comparison.value?.index_changes ?? [],filter.value,search.value));
const sortedChanges=computed(()=>historySort.sorted("index-changes", changes.value));
const pages=computed(()=>Math.max(1,Math.ceil(changes.value.length/pageSize)));
const shownChanges=computed(()=>sortedChanges.value.slice((page.value-1)*pageSize,page.value*pageSize));
const totals=computed(()=>indexHistoryTotals(comparison.value?.index_changes ?? []));
const comparablePairs=computed(()=>pairs.value.filter(pair=>indexPairState(pair)==="comparable").length);
const partialPairs=computed(()=>pairs.value.filter(pair=>indexPairState(pair)==="partial_sample").length);
const last=computed(()=>rows.value[0] ?? null);
const selectedElapsed=computed(()=>chosen.value ?
  (chosen.value.newer.collection_started_at_ms-chosen.value.older.collection_started_at_ms)/1000 : null);
let request=0;
const fmt=(v:number|null|undefined):string=>v==null || !Number.isFinite(v)?"—":v.toLocaleString("ru-RU",{maximumFractionDigits:1});
function bytes(value:number|null|undefined):string {
  if (value==null || !Number.isFinite(value)) return "—";
  const n=Math.abs(value),unit=n>=1024**4?1024**4:n>=1024**3?1024**3:n>=1024**2?1024**2:n>=1024?1024:1;
  return `${fmt(n/unit)} ${unit===1024**4?t('indexHistFinish.tib'):unit===1024**3?t('indexHistFinish.gib'):unit===1024**2?t('indexHistFinish.mib'):unit===1024?t('indexHistFinish.kib'):t('indexHistFinish.b')}`;
}
const signedBytes=(v:number|null|undefined):string=>v==null?"—":`${v>0?"+":v<0?"−":""}${bytes(v)}`;
const pairTitle=(id:number):string=>{
  const pair=pairs.value.find(row=>row.newer.id===id);
  return pair?`#${pair.older.id} → #${pair.newer.id} · ${formatHistoryTime(pair.older.collection_started_at_ms)} — ${formatHistoryTime(pair.newer.collection_started_at_ms)}`:"—";
};
async function load():Promise<void> {
  const pair=chosen.value,call=++request;
  comparison.value=null;error.value="";
  if (!props.active || !pair || props.connectionId===null) {busy.value=false;return;}
  if (["target_changed","invalid_time"].includes(indexPairState(pair))) {busy.value=false;return;}
  busy.value=true;
  try {
    // SQLite-only read; unlike the Index Health diagnostic collector this does not query PostgreSQL.
    const result=await props.invoke<IndexComparison>("compare_index_health_snapshots",{
      firstId:pair.older.id,secondId:pair.newer.id,
    });
    if (call!==request || !props.active || props.connectionId!==pair.newer.connection_id ||
      chosenId.value!==pair.newer.id) return;
    if (result.older.id!==pair.older.id || result.newer.id!==pair.newer.id) throw new Error(t('indexHistFinish.mismatchedSnapshots'));
    comparison.value=result;
  } catch(e) {if(call===request) error.value=String(e);}
  finally {if(call===request) busy.value=false;}
}
watch(() => [props.connectionId,props.active,props.snapshots] as const,()=>{
  ++request;comparison.value=null;error.value="";busy.value=false;filter.value="growth";search.value="";page.value=1;
  const latest=pairs.value[0]?.newer.id ?? null;
  chosenId.value=latest;
  if(props.active) void load();
},{immediate:true});
function chooseInterval():void {filter.value="growth";search.value="";page.value=1;void load();}
watch([filter,search,()=>historySort.get("index-changes")],()=>{page.value=1;});
onUnmounted(()=>{++request;});
</script>

<template>
<section class="ixhist" :aria-label="t('historyAttributes.l7')">
  <header class="ixhist-head"><div><h2>{{ t('historyDetails.IndexHealthHeading') }}</h2>
    <p class="muted">{{ t('historyDetails.IndexHealthDescription') }}</p></div>
    <button type="button" class="secondary" @click="emit('openAnalyzer',connectionId)">{{ t('historyDetails.IndexHealthOpen') }}</button>
  </header>
  <div class="ixhist-cards">
    <article><small>{{ t('historyDetails.IndexHealthSnapshots') }}</small><strong>{{rows.length}}</strong><small>{{ t('historyDetails.IndexHealthLast60') }}</small></article>
    <article><small>{{ t('historyDetails.IndexHealthIntervalQuality') }}</small><strong>{{comparablePairs}}{{ t('indexHistFinish.full') }}{{partialPairs}}{{ t('indexHistFinish.partial') }}</strong><small>{{ t('indexHistFinish.pairs') }}{{pairs.length}}{{ t('indexHistFinish.pairsCount') }}{{ t('remainingVue.otherCaveats') }}</small></article>
    <article><small>{{ t('historyDetails.IndexHealthLastIndexes') }}</small><strong>{{fmt(last?.total_index_count)}}</strong><small>{{ t('indexHistFinish.surveyed') }}{{fmt(last?.sampled_index_count)}}{{last?.sample_truncated?t('remainingVue.partialLiteral'):''}}</small></article>
    <article><small>{{ t('historyDetails.IndexHealthSignals') }}</small><strong>{{fmt(last?.suspiciously_unused_count)}} / {{fmt(last?.possible_duplicate_count)}}</strong><small>{{ t('historyDetails.IndexHealthSignalsHint') }}</small></article>
  </div>
  <p class="ixhist-note">{{ t('indexHistFinish.indexAccumulated') }}<code>idx_scan</code>{{ t('indexHistFinish.caveat') }} {{ t('remainingVue.indexSizeCaveat') }}</p>
  <p v-if="!rows.length" class="muted">{{ t('historyDetails.IndexHealthEmpty') }}</p>
  <section v-if="rows.length" class="ixhist-panel">
    <div class="ixhist-head"><div><h3>{{ t('historyDetails.IndexHealthTrend') }}</h3><p class="muted">{{ t('historyDetails.IndexHealthTrendHint') }}</p></div>
      <label>{{ t('historyDetails.IndexHealthMetric') }}<select v-model="metric"><option v-for="item in INDEX_HISTORY_METRICS" :key="item.key" :value="item.key">{{item.label}}</option></select></label></div>
    <svg class="ixhist-chart" viewBox="0 0 880 260" role="img" :aria-label="t('indexHistFinish.chartAria', { metric, count: String(trend.points.length) })">
      <line x1="76" y1="204" x2="816" y2="204" stroke="var(--border-strong)"/>
      <text x="76" y="25" fill="var(--text-secondary)" font-size="12">{{fmt(trend.maximum)}}</text>
      <text x="62" y="210" fill="var(--text-secondary)" font-size="12" text-anchor="end">0</text>
      <text v-if="trend.minMs !== null" x="76" y="233" fill="var(--text-secondary)" font-size="12">{{formatHistoryTime(trend.minMs)}}</text>
      <text v-if="trend.maxMs !== null" x="816" y="233" text-anchor="end" fill="var(--text-secondary)" font-size="12">{{formatHistoryTime(trend.maxMs)}}</text>
      <polyline v-for="(segment,i) in trend.segments.filter(part=>part.length>1)" :key="i" :points="segment.map(p=>`${p.x},${p.y}`).join(' ')" fill="none" stroke="var(--accent)" stroke-width="2.5"/>
      <circle v-for="point in trend.points" :key="point.id" :cx="point.x" :cy="point.y" r="4" :fill="point.id===chosenId?'var(--text)':'var(--accent)'"><title>#{{point.id}} · {{formatHistoryTime(point.ms)}} · {{fmt(point.value)}}</title></circle>
    </svg>
    <p class="muted">{{ t('historyDetails.IndexHealthCaution') }}</p>
  </section>
  <section v-if="pairs.length" class="ixhist-panel">
    <h3>{{ t('historyDetails.IndexHealthInterval') }}</h3>
    <label class="ixhist-selector">{{ t('historyDetails.IndexHealthEndSnapshot') }}<select v-model.number="chosenId" :disabled="busy" @change="chooseInterval">
      <option v-for="pair in pairs" :key="pair.newer.id" :value="pair.newer.id">{{pairTitle(pair.newer.id)}}</option>
    </select></label>
    <template v-if="chosen">
      <p class="muted">{{state ? indexPairStatus(state) : '—'}} · {{fmt(selectedElapsed)}}{{ t('indexHistFinish.seconds') }}{{chosen.older.id}}{{ t('indexHistFinish.to') }}{{chosen.newer.id}}.</p>
      <p v-if="chosen.older.sample_truncated || chosen.newer.sample_truncated" class="ixhist-warning">{{ t('historyDetails.IndexHealthPartial') }}</p>
      <p v-if="state==='gap'" class="ixhist-warning">{{ t('historyDetails.IndexHealthGap') }}</p>
      <p v-if="state==='reset' || state==='unknown_reset'" class="ixhist-warning">{{ t('historyDetails.IndexHealthReset') }}</p>
      <p v-if="state==='target_changed' || state==='invalid_time'" class="ixhist-warning">{{ t('historyDetails.IndexHealthUnavailable') }}</p>
    </template>
    <p v-if="busy" class="muted" role="status">{{ t('indexHistFinish.comparing') }}</p>
    <p v-if="error" class="ixhist-error" role="alert">{{error}}</p>
    <template v-if="comparison && !busy">
      <div class="ixhist-cards ixhist-interval-cards">
        <article><small>{{ t('indexHistFinish.matching') }}</small><strong>{{fmt(totals.matched)}}</strong><small>{{ t('indexHistFinish.unmatched') }}{{fmt(totals.unmatched)}}</small></article>
        <article><small>{{ t('indexHistFinish.comparableScans') }}</small><strong>{{fmt(comparison.stats_period_continuous?totals.scans:null)}}</strong><small>{{fmt(totals.measured)}}{{ t('indexHistFinish.withValidDelta') }}</small></article>
        <article><small>{{ t('histIndexFinal.indexDelta') }}</small><strong>{{signedBytes(totals.netSizeBytes)}}</strong><small>{{ t('histIndexFinal.growing') }}{{fmt(totals.growing)}}{{ t('histIndexFinal.compared') }}</small></article>
        <article><small>{{ t('histIndexFinal.noScans') }}</small><strong>{{comparison.stats_period_continuous?fmt(totals.silentWithDml):'—'}}</strong><small>{{ t('histIndexFinal.candidates') }}</small></article>
      </div>
      <div class="ixhist-head ixhist-filter-head"><div><h3>{{ t('histIndexFinal.changes') }}</h3><p class="muted">{{ t('histIndexFinal.deltaCaveat') }}</p></div>
        <div class="ixhist-filters"><label>{{ t('histIndexFinal.filter') }}<select v-model="filter">
          <option value="growth">{{ t('histIndexFinal.changedSize') }}</option><option value="scans">{{ t('histIndexFinal.hadScans') }}</option>
          <option value="silent">{{ t('histIndexFinal.noScansDml') }}</option><option value="coverage">{{ t('histIndexFinal.coverage') }}</option><option value="all">{{ t('histIndexFinal.all') }}</option>
        </select></label><label>{{ t('histIndexFinal.search') }}<input v-model="search" :placeholder="t('historyAttributes.l13')" type="search"/></label></div>
      </div>
      <div class="ixhist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l12')" :column="'relation_name'" :order="historySort.get('index-changes')" @sort="historySort.toggle('index-changes', $event)" /><HistorySortTh :label="t('historyAttributes.l11')" :column="'sample_state'" :order="historySort.get('index-changes')" @sort="historySort.toggle('index-changes', $event)" /><HistorySortTh label="Δ idx_scan" :column="'idx_scan_delta'" :order="historySort.get('index-changes')" @sort="historySort.toggle('index-changes', $event)" /><HistorySortTh :label="t('historyAttributes.l1')" :column="'relation_dml_delta'" :order="historySort.get('index-changes')" @sort="historySort.toggle('index-changes', $event)" /><HistorySortTh :label="t('historyAttributes.l2')" :column="'size_delta_bytes'" :order="historySort.get('index-changes')" @sort="historySort.toggle('index-changes', $event)" /><HistorySortTh :label="t('historyAttributes.l9')" :column="'newer_classifications'" :order="historySort.get('index-changes')" @sort="historySort.toggle('index-changes', $event)" /></tr></thead><tbody>
        <tr v-for="row in shownChanges" :key="row.index_oid"><td><strong>{{row.relation_name}}</strong><small>{{row.index_name}} · OID {{row.index_oid}}</small></td>
          <td>{{row.sample_state==='comparable'?t('histIndexFinal.comparable'):row.sample_state==='sampled_only_older'?t('histIndexFinal.older'):row.sample_state==='sampled_only_newer'?t('histIndexFinal.newer'):t('histIndexFinal.identity')}}</td>
          <td>{{fmt(row.idx_scan_delta)}}</td><td>{{fmt(row.relation_dml_delta)}}</td><td>{{signedBytes(row.size_delta_bytes)}}</td><td>{{row.newer_classifications.map(indexLabel).join(', ') || '—'}}</td></tr>
        <tr v-if="!changes.length"><td colspan="6" class="muted">{{ t('histIndexFinal.empty') }}</td></tr>
      </tbody></table></div>
      <div v-if="pages>1" class="ixhist-pager"><button class="secondary" type="button" :disabled="page<=1" @click="page--">{{ t('histIndexFinal.previous') }}</button><span>{{page}} / {{pages}} · {{changes.length}}{{ t('histIndexFinal.records') }}</span><button class="secondary" type="button" :disabled="page>=pages" @click="page++">{{ t('histIndexFinal.next') }}</button></div>
      <details v-if="comparison.limitations.length"><summary>{{ t('histIndexFinal.limitations') }}{{comparison.limitations.length}})</summary><ul><li v-for="(line,i) in comparison.limitations" :key="i">{{line}}</li></ul></details>
    </template>
  </section>
  <p v-if="rows.length===1" class="muted">{{ t('histIndexFinal.singleSnapshot') }}</p>
  <details v-if="rows.length" class="ixhist-panel"><summary>{{ t('histIndexFinal.saved') }}{{rows.length}})</summary>
    <div class="ixhist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l10')" :column="'collection_started_at_ms'" :order="historySort.get('index-summaries')" @sort="historySort.toggle('index-summaries', $event)" /><HistorySortTh :label="t('historyAttributes.l6')" :column="'sampled_index_count'" :order="historySort.get('index-summaries')" @sort="historySort.toggle('index-summaries', $event)" /><HistorySortTh :label="t('historyAttributes.l3')" :column="'suspiciously_unused_count'" :order="historySort.get('index-summaries')" @sort="historySort.toggle('index-summaries', $event)" /><HistorySortTh :label="t('historyAttributes.l4')" :column="'possible_duplicate_count'" :order="historySort.get('index-summaries')" @sort="historySort.toggle('index-summaries', $event)" /><HistorySortTh :label="t('historyAttributes.l8')" :column="'high_write_overhead_count'" :order="historySort.get('index-summaries')" @sort="historySort.toggle('index-summaries', $event)" /><HistorySortTh :label="t('historyAttributes.l5')" :column="'sample_truncated'" :order="historySort.get('index-summaries')" @sort="historySort.toggle('index-summaries', $event)" /></tr></thead><tbody>
      <tr v-for="row in historySort.sorted('index-summaries', rows)" :key="row.id"><td>#{{row.id}}<small>{{formatHistoryTime(row.collection_started_at_ms)}}</small></td><td>{{fmt(row.sampled_index_count)}} / {{fmt(row.total_index_count)}}</td><td>{{fmt(row.suspiciously_unused_count)}}</td><td>{{fmt(row.possible_duplicate_count)}}</td><td>{{fmt(row.high_write_overhead_count)}}</td><td>{{row.sample_truncated?t('vueFinalPolish.partial'):t('vueFinalPolish.full')}}</td></tr>
    </tbody></table></div>
  </details>
</section>
</template>

<style scoped>
.ixhist{display:grid;gap:1rem;min-width:0}.ixhist-head{display:flex;justify-content:space-between;align-items:start;gap:1rem;flex-wrap:wrap}.ixhist h2,.ixhist h3{margin:.2rem 0 .45rem}.ixhist h2{font-size:1.1rem}.ixhist p{line-height:1.45}.muted,.ixhist small{color:var(--text-secondary);font-size:.88rem}.ixhist-cards{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.65rem}.ixhist-cards article{min-width:0;border-radius:.55rem;background:var(--surface-2);padding:.8rem;display:grid;gap:.45rem;align-content:start}.ixhist-cards strong{font-size:1.2rem}.ixhist-interval-cards{margin:1rem 0}.ixhist-note{background:var(--surface-2);border-radius:.5rem;padding:.85rem}.ixhist-panel{border:1px solid var(--border-strong);border-radius:.65rem;padding:1rem;min-width:0}.ixhist-head label,.ixhist-selector{display:grid;gap:.35rem;min-width:min(270px,100%)}.ixhist select,.ixhist input{box-sizing:border-box;width:100%;min-width:0;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.4rem;padding:.6rem}.ixhist-selector{max-width:550px;margin:.6rem 0}.ixhist-chart{width:100%;height:auto;max-height:290px}.ixhist-filters{display:flex;flex-wrap:wrap;gap:.65rem}.ixhist-filters label{min-width:190px}.ixhist-scroll{overflow:auto;max-height:550px}.ixhist table{width:100%;border-collapse:collapse;font-size:.87rem;text-align:left}.ixhist th,.ixhist td{padding:.65rem;border-bottom:1px solid var(--border);vertical-align:top}.ixhist th{position:sticky;top:0;background:var(--surface);white-space:nowrap}.ixhist td small{display:block;margin-top:.25rem}.ixhist-pager{display:flex;gap:.8rem;justify-content:flex-end;align-items:center;margin-top:.7rem}.ixhist-warning{color:var(--message)}.ixhist-error{color:var(--error)}.ixhist details,.ixhist-panel summary{cursor:pointer}.ixhist-panel ul{padding-left:1.25rem}@media(max-width:1100px){.ixhist-cards{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:600px){.ixhist-cards{grid-template-columns:1fr}.ixhist-filters{width:100%}.ixhist-filters label{flex:1}}
</style>
