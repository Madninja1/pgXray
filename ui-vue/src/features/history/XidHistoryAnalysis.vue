<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed, nextTick, ref, shallowRef, watch } from "vue";
import HistorySortTh from "./HistorySortTh.vue";
import { useHistorySorting } from "./use-history-sorting.js";
import { historySortValue } from "./sort-rows.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { HistorySnapshot } from "./model.js";
import { formatHistoryTime, humanValue } from "./model.js";
import { xidAgeCohorts, isSystemXidRelation, xidRelations, type XidObservation, type XidRelationAssessment } from "./analysis-model.js";
import { xidChartSegments, xidHistoryChart, xidHistoryTimeline, xidRelationTransitions,
  type XidHistoryOverview, type XidMetric, type XidRelationHistoryPoint, type XidIntervalObservation, type XidRelationTransition } from "./xid-insights.js";

const historySort = useHistorySorting();
const props = defineProps<{ connectionId: number | null; snapshots: HistorySnapshot[]; invoke: PgXrayInvoke; active: boolean }>();
const emit = defineEmits<{ openAnalyzer: [] }>();
const current = shallowRef<XidObservation | null>(null);
const previous = shallowRef<XidObservation | null>(null);
const observations = shallowRef<XidHistoryOverview[]>([]);
const error = ref(""); const trendError = ref(""); const loading = ref(false);
const query = ref("");
const scope = ref<"priority" | "all" | "user" | "system" | "freeze">("priority");
const metric = ref<XidMetric>("xid");
const zoom = ref(false);
const selectedOid = ref<number | null>(null);
const relationHistory = shallowRef<XidRelationHistoryPoint[] | null>(null);
const historyError = ref(""); const historyLoading = ref(false);
const selectedCohortKey = ref<string | null>(null);
const cohortSearch = ref("");
const cohortPage = ref(0);
const relationHistorySection = ref<HTMLElement | null>(null);
const COHORT_PAGE_SIZE = 12;
let request = 0; let relationRequest = 0;

const sorted = computed(() => [...props.snapshots].sort((a,b)=>b.collection_started_at_ms-a.collection_started_at_ms || b.id-a.id));
const latestId = computed(() => sorted.value[0]?.id ?? null);
const timeLabel = (ms:number) => formatHistoryTime(ms);
const timeline = computed(() => xidHistoryTimeline(observations.value));
const intervals = computed(() => timeline.value.intervals.slice().reverse());
const events = computed(() => intervals.value.filter(x => x.horizon_advanced || x.mxid_horizon_advanced || x.threshold_changed));
const xidEventLabel = (event:XidIntervalObservation):string => event.horizon_advanced?t('histXidFinal.eventXid'):event.mxid_horizon_advanced?t('histXidFinal.eventMxid'):t('histXidFinal.eventThreshold');
const shownEvents = computed(() => historySort.sorted("xid-events",events.value,(row,column) =>
  column === "observation" ? xidEventLabel(row) : historySortValue(row,column)).slice(0,15));
const comparableCount = computed(() => intervals.value.filter(x=>x.state === "observed").length);
const chart = computed(() => xidHistoryChart(timeline.value, metric.value, zoom.value));
const chartLines = computed(() => xidChartSegments(chart.value).filter(s=>s.length>1).map(segment=>segment.map(p=>`${p.x},${p.y}`).join(" ")));
const chartHasData = computed(() => chart.value.points.length >= 2);
const freezeMax = computed(() => current.value?.overview.autovacuum_freeze_max_age ?? 0);
const freezePct = computed(() => current.value && freezeMax.value > 0 ? current.value.overview.database_xid_age/freezeMax.value*100 : null);
const mxidPct = computed(() => { const o=current.value?.overview; return o?.database_mxid_age != null && o.autovacuum_multixact_freeze_max_age != null && o.autovacuum_multixact_freeze_max_age > 0 ? o.database_mxid_age/o.autovacuum_multixact_freeze_max_age*100 : null; });
const continuityUnknown = computed(() => {
  // No per-relation delta across a missing snapshot, long gap or XID rewind.
  const last = timeline.value.intervals[timeline.value.intervals.length - 1];
  return !current.value || !previous.value || !last || last.state !== "observed" ||
    last.newer.summary.id !== latestId.value || last.older.summary.id !== sorted.value[1]?.id;
});
const allRelations = computed(() => current.value ? xidRelations(current.value,continuityUnknown.value ? null : previous.value) : []);
const warningCount = computed(() => allRelations.value.filter(row => (row.pct_of_freeze_max ?? -1)>=70 || (row.pct_of_multixact_freeze_max ?? -1)>=70).length);
const freezeCount = computed(() => allRelations.value.filter(row => row.freeze_observed).length);
const filtered = computed(() => allRelations.value.filter(row => {
  if (query.value.trim() && !row.identity.toLocaleLowerCase().includes(query.value.trim().toLocaleLowerCase())) return false;
  const system = isSystemXidRelation(row);
  switch (scope.value) {
    case "system": return system;
    case "user": return !system;
    case "freeze": return row.freeze_observed;
    case "priority": return (row.pct_of_freeze_max ?? -1)>=70 || (row.pct_of_multixact_freeze_max ?? -1)>=70 || row.freeze_observed;
    case "all": return true;
  }
}));
const overviewFallback = computed(() => scope.value === "priority" && !query.value.trim() && filtered.value.length === 0 && allRelations.value.length > 0);
const cohorts = computed(() => {
  const groups=xidAgeCohorts(overviewFallback.value ? allRelations.value : filtered.value);
  if (!overviewFallback.value) return groups;
  return [...groups.filter(g=>g.system).slice(0,3),...groups.filter(g=>!g.system).slice(0,2)].sort((a,b) =>
    (b.representative.pct_of_freeze_max ?? -1)-(a.representative.pct_of_freeze_max ?? -1));
});
const visibleCohorts = computed(() => cohorts.value.slice(0,30));
const selectedCohort = computed(() => cohorts.value.find(group => group.key === selectedCohortKey.value) ?? null);
const selectedMembers = computed(() => {
  const term = cohortSearch.value.trim().toLocaleLowerCase();
  return (selectedCohort.value?.members ?? [])
    .filter(row => !term || row.identity.toLocaleLowerCase().includes(term) || String(row.relation_oid).includes(term))
    .sort((a, b) => b.total_size_bytes - a.total_size_bytes || a.identity.localeCompare(b.identity, "ru"));
});
const cohortPageCount = computed(() => Math.max(1, Math.ceil(selectedMembers.value.length / COHORT_PAGE_SIZE)));
const sortedMembers = computed(() => historySort.sorted("xid-members",selectedMembers.value, (row,column) =>
  column === "last_autovacuum" ? row.last_autovacuum ?? row.last_vacuum : historySortValue(row,column)));
const visibleMembers = computed(() => sortedMembers.value.slice(cohortPage.value * COHORT_PAGE_SIZE, (cohortPage.value + 1) * COHORT_PAGE_SIZE));
function selectCohort(key: string): void {
  if (selectedCohortKey.value === key) { selectedCohortKey.value = null; return; }
  selectedCohortKey.value = key;
  cohortSearch.value = "";
  cohortPage.value = 0;
  selectedOid.value = null;
  relationHistory.value = null;
}
function formatSize(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return "—";
  const units = [t('histXidFinal.bytes'), t('histXidFinal.kib'), t('histXidFinal.mib'), t('histXidFinal.gib'), t('histXidFinal.tib')];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index++; }
  return `${value.toLocaleString("ru-RU", { maximumFractionDigits: index === 0 ? 0 : 1 })} ${units[index]}`;
}
function vacuumLabel(row: XidRelationAssessment): string {
  const raw = row.last_autovacuum ?? row.last_vacuum;
  if (!raw) return t('histXidFinal.unknown');
  const timestamp = Date.parse(raw);
  return `${row.last_autovacuum ? t('histXidFinal.automatic') : t('histXidFinal.manual')} · ${Number.isFinite(timestamp) ? timeLabel(timestamp) : raw}`;
}
watch([scope, query], () => {
  selectedCohortKey.value = null;
  cohortSearch.value = "";
  cohortPage.value = 0;
});
watch([cohortSearch,()=>historySort.get("xid-members")], () => { cohortPage.value = 0; });
const focus = computed(() => allRelations.value.find(r=>r.relation_oid===selectedOid.value) ?? null);
const relationTransitions = computed(() => xidRelationTransitions(relationHistory.value ?? []).reverse());
const xidTransitionLabel = (item:XidRelationTransition):string => item.xid_horizon_moved || item.mxid_horizon_moved ? t('xidHistFinish.horizon') : item.state==='unverified'?t('xidHistFinish.unverified'):t('xidHistFinish.noDecrease');
const shownTransitions = computed(() => historySort.sorted("xid-transitions",relationTransitions.value,(row,column) =>
  column === "observation" ? xidTransitionLabel(row) : historySortValue(row,column)).slice(0,40));
function severity(r:XidRelationAssessment):string {
  const p=Math.max(r.pct_of_freeze_max??0,r.pct_of_multixact_freeze_max??0);
  return p>=90?t('histXidFinal.urgent'):p>=70?t('histXidFinal.attention'):r.freeze_observed?t('histXidFinal.decreased'):t('histXidFinal.observation');
}
function intervalLabel(state:string):string {
  return ({observed:t('histXidFinal.eventComparable'),gap:t('histXidFinal.eventGap'),restart:t('histXidFinal.eventRestart'),
    recovery:t('histXidFinal.eventRecovery'),xid_rewind:t('histXidFinal.eventRewind'),unknown_continuity:t('histXidFinal.eventUnknown')} as Record<string,string>)[state] ?? state;
}
function signed(value:number|null):string { return value===null?"—":`${value>0?"+":""}${humanValue(value)}`; }
function pct(value:number|null|undefined):string { return value==null?"—":`${humanValue(value)}%`; }

async function load():Promise<void> {
  const seq=++request; relationRequest++;
  selectedOid.value=null; relationHistory.value=null; current.value=null; previous.value=null;
  observations.value=[]; trendError.value=""; error.value=""; selectedCohortKey.value=null; cohortSearch.value=""; cohortPage.value=0;
  if(!props.active || props.connectionId===null || latestId.value===null) { loading.value=false; return; }
  loading.value=true;
  const id=props.connectionId;
  const first=sorted.value[0], second=sorted.value[1];
  try {
    const [snapshot, history] = await Promise.allSettled([
      props.invoke<{snapshot:XidObservation}>("load_xid_snapshot",{connectionId:id,snapshotId:first.id}),
      props.invoke<XidHistoryOverview[]>("xid_history_overview",{connectionId:id,limit:120}),
    ]);
    if (seq!==request) return;
    if(snapshot.status === "fulfilled") current.value=snapshot.value.snapshot;
    else error.value=t('histXidFinal.latestError', { error: String(snapshot.reason) });
    if(history.status === "fulfilled") observations.value=history.value;
    else trendError.value=t('histXidFinal.historyError', { error: String(history.reason) });
    if(second && current.value && first.host===second.host && first.port===second.port &&
      first.database_oid===second.database_oid && first.username===second.username && first.sslmode===second.sslmode) {
      try {
        const result=await props.invoke<{snapshot:XidObservation}>("load_xid_snapshot",{connectionId:id,snapshotId:second.id});
        if(seq===request) previous.value=result.snapshot;
      } catch { /* Latest snapshot remains useful. */ }
    }
  } finally { if(seq===request) loading.value=false; }
}
async function investigate(row:XidRelationAssessment):Promise<void> {
  const snapshotId=latestId.value; if(snapshotId===null) return;
  const seq=++relationRequest; selectedOid.value=row.relation_oid;
  relationHistory.value=null; historyError.value=""; historyLoading.value=true;
  await nextTick();
  relationHistorySection.value?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  try {
    const points=await props.invoke<XidRelationHistoryPoint[]>("xid_relation_history",{snapshotId,relationOid:row.relation_oid,limit:120});
    if(seq===relationRequest) relationHistory.value=points;
  } catch(e) { if(seq===relationRequest) historyError.value=String(e); }
  finally { if(seq===relationRequest) historyLoading.value=false; }
}
watch(()=>[props.active,props.connectionId,latestId.value,props.snapshots.length] as const,()=>{void load();},{immediate:true});
</script>

<template>
<section class="analysis" :aria-label="t('historyAttributes.l44')">
  <header class="analysis-head">
    <div><h2>{{ t('historyDetails.XidHeading') }}</h2><p class="muted">{{ t('historyDetails.XidDescription') }}</p></div>
    <button type="button" class="secondary" @click="emit('openAnalyzer')">{{ t('historyDetails.XidOpen') }}</button>
  </header>
  <p v-if="loading" class="muted" role="status">{{ t('historyDetails.XidLoading') }}</p>
  <p v-if="error" class="hist-error" role="alert">{{error}}</p>
  <p v-if="trendError" class="hist-error" role="alert">{{trendError}}</p>
  <p v-if="!current && !loading && !error" class="muted">{{ t('historyDetails.XidEmpty') }}</p>
  <template v-if="current">
    <div class="metric-grid">
      <div class="metric"><small>{{ t('historyDetails.XidDbAge') }}</small><strong>{{humanValue(current.overview.database_xid_age)}}</strong><span>{{pct(freezePct)}}{{ t('histXidFinal.ofFreeze') }}</span></div>
      <div class="metric"><small>{{ t('historyDetails.XidFreezeMax') }}</small><strong>{{freezeMax>0?humanValue(freezeMax-current.overview.database_xid_age):'—'}}</strong><span>{{ t('historyDetails.XidNotWraparound') }}</span></div>
      <div class="metric"><small>{{ t('historyDetails.XidMxidAge') }}</small><strong>{{humanValue(current.overview.database_mxid_age)}}</strong><span>{{pct(mxidPct)}}{{ t('histXidFinal.ofMxid') }}</span></div>
      <div class="metric"><small>{{ t('historyDetails.XidIntervals') }}</small><strong>{{timeline.points.length}} / {{comparableCount}}</strong><span>{{ t('historyDetails.XidComparable') }}</span></div>
    </div>
    <div class="trend">
      <div class="analysis-head"><div><h3>{{ t('historyDetails.XidTrend') }}</h3><p class="muted">{{ t('historyDetails.XidTrendHint') }}</p></div>
        <div class="filters"><label>{{ t('historyDetails.XidMetric') }}<select v-model="metric"><option value="xid">XID age</option><option value="mxid">MXID age</option></select></label>
          <label class="zoom"><input v-model="zoom" type="checkbox"/> {{ t('historyDetails.XidZoom') }}</label></div>
      </div>
      <svg v-if="chartHasData" viewBox="0 0 870 245" role="img" :aria-label="metric==='xid'?t('xidHistFinish.chartXid'):t('xidHistFinish.chartMxid')">
        <line x1="62" y1="200" x2="826" y2="200" stroke="currentColor" opacity=".45"/>
        <line x1="62" y1="30" x2="62" y2="200" stroke="currentColor" opacity=".45"/>
        <line v-if="chart.thresholdY!==null" x1="62" :y1="chart.thresholdY" x2="826" :y2="chart.thresholdY" stroke="var(--accent)" stroke-dasharray="7 4"/>
        <text x="70" y="17" font-size="12" fill="currentColor">{{humanValue(chart.yMax)}}</text>
        <text x="4" y="202" font-size="12" fill="currentColor">{{humanValue(chart.yMin)}}</text>
        <text v-if="chart.thresholdY!==null" x="826" y="17" text-anchor="end" font-size="12" fill="var(--accent)">{{ t('histXidFinal.lastFreeze') }}{{humanValue(chart.threshold)}}</text>
        <polyline v-for="(segment,i) in chartLines" :key="i" :points="segment" fill="none" stroke="var(--accent)" stroke-width="2"/>
        <circle v-for="point in chart.points" :key="point.id" :cx="point.x" :cy="point.y" r="3.5" fill="var(--accent)">
          <title>#{{point.id}} · {{timeLabel(point.ms)}} · age {{humanValue(point.value)}}</title>
        </circle>
        <text x="62" y="228" font-size="12" fill="currentColor">{{timeLabel(chart.points[0].ms)}}</text>
        <text x="826" y="228" text-anchor="end" font-size="12" fill="currentColor">{{timeLabel(chart.points[chart.points.length-1].ms)}}</text>
      </svg>
      <p v-else class="muted">{{ t('historyDetails.XidNotEnough') }}</p>
      <p v-if="zoom" class="warning">{{ t('historyDetails.XidZoomHint') }}</p>
      <p v-if="timeline.ignoredTargets" class="warning">{{timeline.ignoredTargets}}{{ t('histXidFinal.excluded') }}</p>
      <p class="muted">{{ t('historyDetails.XidIdWarning') }}</p>
    </div>
    <div class="trend">
      <h3>{{ t('historyDetails.XidGaps') }}</h3>
      <p class="muted">{{ t('historyDetails.XidGapsHint') }}</p>
      <p v-if="!events.length" class="muted">{{ t('historyDetails.XidNoDecrease') }}</p>
      <div v-else class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l45')" :column="'newer.summary.collection_started_at_ms'" :order="historySort.get('xid-events')" @sort="historySort.toggle('xid-events', $event)" /><HistorySortTh :label="t('historyAttributes.l46')" :column="'observation'" :order="historySort.get('xid-events')" @sort="historySort.toggle('xid-events', $event)" /><HistorySortTh :label="t('historyAttributes.l43')" :column="'xid_age_delta'" :order="historySort.get('xid-events')" @sort="historySort.toggle('xid-events', $event)" /></tr></thead><tbody>
        <tr v-for="event in shownEvents" :key="event.newer.summary.id">
          <td>{{timeLabel(event.older.summary.collection_started_at_ms)}} → {{timeLabel(event.newer.summary.collection_started_at_ms)}}</td>
          <td><strong>{{xidEventLabel(event)}}</strong></td>
          <td>Δ XID: {{signed(event.xid_age_delta)}} · Δ MXID: {{signed(event.mxid_age_delta)}}<span v-if="event.threshold_changed">{{ t('histXidFinal.thresholdChanged') }}</span></td>
        </tr>
      </tbody></table></div>
      <details v-if="intervals.length"><summary>{{ t('histXidFinal.allIntervals') }}{{intervals.length}})</summary>
        <div class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l49')" :column="'newer.summary.id'" :order="historySort.get('xid-intervals')" @sort="historySort.toggle('xid-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l28')" :column="'newer.summary.collection_started_at_ms'" :order="historySort.get('xid-intervals')" @sort="historySort.toggle('xid-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l40')" :column="'xid_per_hour'" :order="historySort.get('xid-intervals')" @sort="historySort.toggle('xid-intervals', $event)" /><HistorySortTh label="Δ XID age" :column="'xid_age_delta'" :order="historySort.get('xid-intervals')" @sort="historySort.toggle('xid-intervals', $event)" /><HistorySortTh label="Δ MXID age" :column="'mxid_age_delta'" :order="historySort.get('xid-intervals')" @sort="historySort.toggle('xid-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l36')" :column="'state'" :order="historySort.get('xid-intervals')" @sort="historySort.toggle('xid-intervals', $event)" /></tr></thead><tbody>
          <tr v-for="interval in historySort.sorted('xid-intervals',intervals).slice(0,60)" :key="interval.newer.summary.id"><td>#{{interval.older.summary.id}} → #{{interval.newer.summary.id}}</td><td>{{timeLabel(interval.newer.summary.collection_started_at_ms)}}</td>
            <td>{{humanValue(interval.xid_per_hour)}}</td><td>{{signed(interval.xid_age_delta)}}</td><td>{{signed(interval.mxid_age_delta)}}</td><td>{{intervalLabel(interval.state)}}</td></tr>
        </tbody></table></div>
        <p class="muted">{{ t('histXidFinal.rateCaveat') }}</p>
      </details>
    </div>
    <div class="relation-area">
      <div class="analysis-head"><div><h3>{{ t('historyDetails.XidRelations') }}</h3>
        <p class="muted">{{ t('histXidFinal.lastSnapshot') }}{{allRelations.length}}{{ t('histXidFinal.objects') }}{{warningCount}}{{ t('histXidFinal.warningThreshold') }}{{freezeCount}}{{ t('histXidFinal.freezeObserved') }}</p>
      </div><div class="filters"><label>{{ t('historyDetails.XidShow') }}<select v-model="scope"><option value="priority">{{ t('historyDetails.XidAttention') }}</option><option value="all">{{ t('historyDetails.XidAllGroups') }}</option><option value="freeze">{{ t('historyDetails.XidAgeDecrease') }}</option><option value="user">{{ t('historyDetails.XidUser') }}</option><option value="system">{{ t('historyDetails.XidSystem') }}</option></select></label>
        <label>{{ t('historyDetails.XidSearch') }}<input v-model="query" :placeholder="t('historyAttributes.l51')"/></label></div></div>
      <p v-if="warningCount === 0 && freezeCount === 0" class="status-note">{{ t('historyDetails.XidNoAttention') }}</p>
      <p v-if="continuityUnknown" class="warning">{{ t('historyDetails.XidNoLastPair') }}</p>
      <p v-if="overviewFallback" class="muted">{{ t('historyDetails.XidTopFive') }}</p>
      <details class="reading-guide"><summary>{{ t('historyDetails.XidGroupGuide') }}</summary>
        <p>{{ t('histXidFinal.groupExplainer') }}<strong>{{ t('histXidFinal.groupSame') }}</strong>{{ t('histXidFinal.groupLimitation') }} {{ t('remainingVue.exampleMeaning') }} <code>relfrozenxid</code>.</p>
        <p>{{ t('histXidFinal.groupUsage') }} {{ t('xidHistFinish.thresholds') }}</p>
      </details>
      <div class="cohort-workspace">
        <div class="cohort-groups" :aria-label="t('historyAttributes.l42')">
          <p class="column-heading">{{ t('histXidFinal.groups') }}{{visibleCohorts.length}}{{ t('xidHistFinish.of') }}{{cohorts.length}}</p>
          <button v-for="group in visibleCohorts" :key="group.key" type="button" class="cohort-choice"
            :class="{'cohort-choice-selected':selectedCohortKey===group.key}" :aria-pressed="selectedCohortKey===group.key"
            @click="selectCohort(group.key)">
            <span class="cohort-title">{{group.system?t('histXidFinal.statesystem'):t('histXidFinal.stateuser')}} · {{group.members.length}} {{group.members.length===1?t('histXidFinal.stateobject'):t('histXidFinal.stateobjects')}}</span>
            <span class="cohort-stats"><strong>{{pct(group.representative.pct_of_freeze_max)}}</strong>{{ t('xidHistFinish.freeze') }} · XID {{humanValue(group.representative.xid_age)}}</span>
            <span class="cohort-sub">MXID {{pct(group.representative.pct_of_multixact_freeze_max)}} · {{severity(group.representative)}}</span>
            <span class="cohort-example" :title="group.members.map(r=>r.identity).join(', ')">{{group.members.slice(0,2).map(r=>r.identity).join(', ')}}{{group.members.length>2?' …':''}}</span>
          </button>
          <p v-if="!visibleCohorts.length" class="muted">{{ t('historyDetails.XidNoGroups') }}</p>
        </div>
        <div class="cohort-explorer">
          <template v-if="selectedCohort">
            <div class="analysis-head">
              <div><h4>{{selectedCohort.system?t('histXidFinal.statesystem'):t('histXidFinal.stateuser')}}{{ t('xidHistFinish.objects') }}{{selectedCohort.members.length}}</h4>
                <p class="muted">XID age {{humanValue(selectedCohort.representative.xid_age)}} ({{pct(selectedCohort.representative.pct_of_freeze_max)}}{{ t('xidHistFinish.freeze') }}). {{ t('histXidFinal.freezeSame') }}</p></div>
              <label class="cohort-search">{{ t('historyDetails.XidFindRelation') }}<input v-model="cohortSearch" :placeholder="t('historyAttributes.l50')"/></label>
            </div>
            <div class="members-table-wrap"><table class="members-table"><thead><tr><HistorySortTh label="Relation" :column="'identity'" :order="historySort.get('xid-members')" @sort="historySort.toggle('xid-members', $event)" /><HistorySortTh :label="t('historyAttributes.l48')" :column="'total_size_bytes'" :order="historySort.get('xid-members')" @sort="historySort.toggle('xid-members', $event)" /><HistorySortTh :label="t('historyAttributes.l47')" :column="'last_autovacuum'" :order="historySort.get('xid-members')" @sort="historySort.toggle('xid-members', $event)" /><th></th></tr></thead><tbody>
              <tr v-for="relation in visibleMembers" :key="relation.relation_oid">
                <td><strong :title="relation.identity">{{relation.identity}}</strong><small>OID {{relation.relation_oid}} · {{relation.relation_type}}</small></td>
                <td>{{formatSize(relation.total_size_bytes)}}</td>
                <td>{{vacuumLabel(relation)}}</td>
                <td><button type="button" class="secondary" @click="investigate(relation)">{{ t('historyDetails.XidHistory') }}</button></td>
              </tr>
            </tbody></table></div>
            <p v-if="!visibleMembers.length" class="muted">{{ t('historyDetails.XidNoMatches') }}</p>
            <div class="cohort-pagination"><span class="muted">{{selectedMembers.length}} {{ t('histXidFinal.objectsLabel') }}{{ t('xidHistFinish.page') }}{{cohortPage+1}}{{ t('xidHistFinish.of') }}{{cohortPageCount}}</span>
              <div><button type="button" class="secondary" :disabled="cohortPage===0" @click="cohortPage--">{{ t('historyDetails.XidPrevious') }}</button>
                <button type="button" class="secondary" :disabled="cohortPage+1>=cohortPageCount" @click="cohortPage++">{{ t('historyDetails.XidNext') }}</button></div>
            </div>
            <p class="muted"><small>{{ t('histXidFinal.vacuumDate') }}</small></p>
          </template>
          <div v-else class="cohort-placeholder"><strong>{{ t('histXidFinal.pickGroup') }}</strong><p class="muted">{{ t('histXidFinal.groupPlaceholder') }} {{ t('remainingVue.noLongGrid') }}</p></div>
        </div>
      </div>
      <section v-if="focus" ref="relationHistorySection" class="drill"><h3>{{focus.identity}}{{ t('remainingVue.freezeHistory') }}</h3>
        <span v-if="historyLoading" class="muted" role="status">{{ t('histXidFinal.loadingRelation') }}</span><p v-if="historyError" class="hist-error" role="alert">{{historyError}}</p>
        <template v-if="relationHistory"><p class="muted">{{relationHistory.length}}{{ t('remainingVue.measurements') }}</p>
          <div class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l28')" :column="'point.collection_started_at_ms'" :order="historySort.get('xid-transitions')" @sort="historySort.toggle('xid-transitions', $event)" /><HistorySortTh :label="t('historyAttributes.l41')" :column="'point.relation.xid_age'" :order="historySort.get('xid-transitions')" @sort="historySort.toggle('xid-transitions', $event)" /><HistorySortTh :label="t('historyAttributes.l39')" :column="'point.relation.mxid_age'" :order="historySort.get('xid-transitions')" @sort="historySort.toggle('xid-transitions', $event)" /><HistorySortTh :label="t('historyAttributes.l46')" :column="'observation'" :order="historySort.get('xid-transitions')" @sort="historySort.toggle('xid-transitions', $event)" /></tr></thead><tbody>
            <tr v-for="item in shownTransitions" :key="item.point.snapshot_id">
              <td><small>#{{item.point.snapshot_id}}</small>{{timeLabel(item.point.collection_started_at_ms)}}</td>
              <td>{{humanValue(item.point.relation.xid_age)}}<small>Δ {{signed(item.xid_delta)}}</small></td>
              <td>{{humanValue(item.point.relation.mxid_age)}}<small>Δ {{signed(item.mxid_delta)}}</small></td>
              <td>{{xidTransitionLabel(item)}}</td>
            </tr>
          </tbody></table></div>
        </template>
      </section>
    </div>
  </template>
</section>
</template>

<style scoped>
.analysis{display:grid;gap:1rem}.analysis-head{display:flex;align-items:start;justify-content:space-between;gap:1rem;flex-wrap:wrap}.analysis h2,.analysis h3{margin:.2rem 0 .5rem}.muted,.analysis small{color:var(--text-secondary);font-size:.86rem}.metric-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.65rem}.metric{background:var(--surface-2);border-radius:.5rem;padding:.85rem;display:grid;gap:.5rem;min-width:0}.metric strong{font-size:1.25rem;font-variant-numeric:tabular-nums}.metric small,.metric span{color:var(--text-secondary);font-size:.8rem}.trend,.drill{border:1px solid var(--border);border-radius:.5rem;padding:.85rem;min-width:0}.trend svg{display:block;width:100%;max-width:1000px;height:auto}.warning{color:var(--accent)}.filters{display:flex;gap:.65rem;flex-wrap:wrap}.filters label{display:grid;gap:.3rem;font-size:.85rem}.filters select,.filters input{background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);padding:.5rem;border-radius:.35rem}.filters .zoom{display:flex;align-items:center;gap:.4rem;align-self:end;padding:.55rem;white-space:nowrap}.zoom input{accent-color:var(--accent)}.hist-scroll{overflow:auto;max-height:640px}.hist-scroll table{width:100%;border-collapse:collapse;text-align:left;font-size:.85rem}.hist-scroll th,.hist-scroll td{padding:.6rem;border-bottom:1px solid var(--border);vertical-align:middle}.hist-scroll small{display:block;margin:.25rem 0}.bar{height:6px;background:var(--surface-2);border:1px solid var(--border);border-radius:5px;overflow:hidden;width:125px}.bar span{display:block;height:100%;background:var(--accent)}/* Master-detail layout: the member list stays bounded and never expands a table row. */
.relation-area{display:grid;gap:.8rem;min-width:0}.status-note{padding:.75rem .9rem;background:var(--surface-2);border-radius:.4rem}.reading-guide{padding:.65rem .8rem;border:1px solid var(--border);border-radius:.4rem}.reading-guide p{max-width:95ch;line-height:1.5}.cohort-workspace{display:grid;grid-template-columns:minmax(250px,.38fr) minmax(0,.62fr);gap:.75rem;align-items:start;min-width:0}.cohort-groups,.cohort-explorer{border:1px solid var(--border);border-radius:.5rem;padding:.7rem;min-width:0}.cohort-groups{max-height:620px;overflow-y:auto;display:grid;align-content:start;gap:.4rem}.column-heading{font-weight:600;margin:.1rem 0 .4rem}.cohort-choice{width:100%;text-align:left;display:grid;gap:.35rem;padding:.65rem .75rem;background:var(--surface-2);border:1px solid var(--border);border-radius:.45rem;color:var(--text);cursor:pointer;min-width:0}.cohort-choice-selected{border-color:var(--accent)}.cohort-choice:hover{border-color:var(--accent)}.cohort-title{font-weight:600}.cohort-stats,.cohort-sub,.cohort-example{font-size:.83rem;overflow-wrap:anywhere}.cohort-sub,.cohort-example{color:var(--text-secondary)}.cohort-explorer{min-height:160px}.cohort-explorer h4{margin:.1rem 0 .3rem}.cohort-search{display:grid;gap:.35rem;min-width:170px;font-size:.85rem}.cohort-search input{min-width:0;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);padding:.5rem;border-radius:.35rem}.members-table-wrap{width:100%;overflow-x:auto}.members-table{width:100%;border-collapse:collapse;font-size:.85rem;text-align:left}.members-table th,.members-table td{padding:.55rem .45rem;border-bottom:1px solid var(--border);text-align:left;vertical-align:middle}.members-table td:first-child{max-width:230px;overflow-wrap:anywhere}.members-table td:last-child{text-align:right}.members-table td button{white-space:nowrap}.members-table small{display:block;color:var(--text-secondary);margin-top:.25rem}.cohort-pagination{display:flex;justify-content:space-between;gap:.5rem;align-items:center;flex-wrap:wrap;margin:.7rem 0}.cohort-pagination>div{display:flex;gap:.4rem}.cohort-placeholder{padding:1rem;max-width:55ch}.cohort-placeholder p{line-height:1.5}
@media(max-width:1100px){.cohort-workspace{grid-template-columns:minmax(0,1fr)}.cohort-groups{max-height:350px}.members-table td:first-child{max-width:none}}
.hist-error{color:var(--error)}@media(max-width:1000px){.metric-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}
</style>
