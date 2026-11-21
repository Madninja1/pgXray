<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed, ref, shallowRef, watch } from "vue";
import HistorySortTh from "./HistorySortTh.vue";
import { useHistorySorting } from "./use-history-sorting.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { formatHistoryTime, humanValue } from "./model.js";
import {
  formatPgssDuration, formatPgssExecRate, pgssTrendSegments, rankPgss,
  type PgssWorkloadHistory, type PgssRank, type PgssStatement, type PgssInterval,
} from "./analysis-model.js";

const historySort = useHistorySorting();
const props = defineProps<{
  connectionId: number | null; invoke: PgXrayInvoke; active: boolean;
  preferredQueryid?: string | null; navigationToken?: number;
}>();
const emit = defineEmits<{ openAnalyzer: [queryid: string | null] }>();
interface HistoryPoint {
  snapshot_id: number; collection_started_at_ms: number; delta_state: string;
  calls_per_hour: number | null; interval_mean_exec_time_ms: number | null;
  total_exec_time_delta_ms: number | null; wal_bytes_delta: number | null;
  temp_blks_written_delta: number | null;
}
interface StatementHistory {
  anchor_snapshot_id: number;
  summary: { comparable_intervals: number };
  points: HistoryPoint[];
  limitations: string[];
  change_analysis: {
    state: string;
    items: { id: string; label: string; unit: string; older_value: number;
      newer_value: number; absolute_delta: number; relative_change_pct: number | null; direction: string }[];
  };
}

const data = shallowRef<PgssWorkloadHistory | null>(null);
const busy = ref(false);
const error = ref("");
const anchorId = ref<number | null>(null);
const rank = ref<PgssRank>("exec");
const query = ref("");
const page = ref(1);
const field = ref<"global_calls_per_hour" | "global_exec_ms_per_hour" | "sampled_wal_bytes_per_hour" | "sampled_temp_blocks_per_hour">("global_exec_ms_per_hour");
const selected = shallowRef<PgssStatement | null>(null);
const detail = shallowRef<StatementHistory | null>(null);
const detailBusy = ref(false);
const detailError = ref("");
let requestSeq = 0;
let detailSeq = 0;

const safeIntervals = computed(() => (data.value?.intervals ?? []).filter(row => row.state === "comparable").reverse());
const selectedInterval = computed(() => data.value?.intervals.find(row => row.to_snapshot_id === data.value?.selected_to_snapshot_id) ?? null);
const previousInterval = computed(() => data.value?.intervals.find(row => row.to_snapshot_id === data.value?.compared_to_snapshot_id) ?? null);
const comparableCount = computed(() => safeIntervals.value.length);
const comparisonReady = computed(() => previousInterval.value !== null);
const ranked = computed(() => rankPgss(data.value?.statements ?? [], rank.value, query.value));
const sorted = computed(() => historySort.sorted("pgss-sql",ranked.value));
const shown = computed(() => sorted.value.slice((page.value - 1) * 20, page.value * 20));
const pages = computed(() => Math.max(1, Math.ceil(sorted.value.length / 20)));
const segments = computed(() => pgssTrendSegments(data.value?.intervals ?? [], field.value));
const lines = computed(() => segments.value.filter(segment => segment.length > 1).map(segment => segment.map(p => `${p.x},${p.y}`).join(" ")));
const validPoints = computed(() => segments.value.flat());
const chartMax = computed(() => Math.max(0, ...validPoints.value.map(p => p.value)));
const chartValue = (value: number): string => field.value === "global_exec_ms_per_hour" ? formatPgssExecRate(value) :
  field.value === "sampled_wal_bytes_per_hour" ? formatBytesRate(value) : humanValue(value);
const hasRegression = computed(() => data.value?.statements.some(row => row.previous !== null) ?? false);
function formatBytesRate(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return "—";
  const abs = Math.abs(bytes);
  const div = abs >= 1024 ** 3 ? 1024 ** 3 : abs >= 1024 ** 2 ? 1024 ** 2 : abs >= 1024 ? 1024 : 1;
  const unit = div === 1024 ** 3 ? t('pgssHistFinish.gib') : div === 1024 ** 2 ? t('pgssHistFinish.mib') : div === 1024 ? t('pgssHistFinish.kib') : t('pgssHistFinish.b');
  return `${humanValue(bytes / div)} ${unit}${t('remainingVue.perHour')}`;
}
function percentDiff(current: number | null | undefined, older: number | null | undefined): string {
  if (current == null || older == null || older <= 0) return "—";
  const value = (current - older) / older * 100;
  return Number.isFinite(value) ? `${value > 0 ? "+" : ""}${humanValue(value)}%` : "—";
}
function intervalName(row: PgssInterval): string {
  return `${formatHistoryTime(row.from_ms)} — ${formatHistoryTime(row.to_ms)}`;
}
function intervalLabel(row: PgssInterval): string {
  return `#${row.from_snapshot_id} → #${row.to_snapshot_id} · ${intervalName(row)}`;
}
async function load(): Promise<void> {
  const id = props.connectionId;
  const request = ++requestSeq;
  ++detailSeq;
  selected.value = null;
  detail.value = null;
  detailBusy.value = false;
  detailError.value = "";
  error.value = "";
  data.value = null;
  if (!props.active || id === null) { busy.value = false; return; }
  busy.value = true;
  try {
    const result = await props.invoke<PgssWorkloadHistory>("pgss_workload_history", {
      connectionId: id, limit: 100, anchorSnapshotId: anchorId.value,
    });
    if (request !== requestSeq) return;
    data.value = result;
    anchorId.value = result.selected_to_snapshot_id;
    if (props.preferredQueryid && !query.value) query.value = props.preferredQueryid;
  } catch (cause) {
    if (request === requestSeq) error.value = String(cause);
  } finally {
    if (request === requestSeq) busy.value = false;
  }
}
function selectAnchor(): void {
  if (anchorId.value !== null) void load();
}
async function openStatement(row: PgssStatement): Promise<void> {
  const snapshotId = data.value?.selected_to_snapshot_id;
  if (snapshotId == null) return;
  const request = ++detailSeq;
  selected.value = row;
  detail.value = null;
  detailError.value = "";
  detailBusy.value = true;
  try {
    const result = await props.invoke<StatementHistory>("pgss_statement_history", {
      snapshotId, userid: row.userid, dbid: row.dbid, queryid: row.queryid,
      toplevel: row.toplevel, limit: 100,
    });
    if (request === detailSeq) {
      if (result.anchor_snapshot_id !== snapshotId) throw Error(t('histPgssFinal.wrongSnapshot'));
      detail.value = result;
    }
  } catch (cause) {
    if (request === detailSeq) detailError.value = String(cause);
  } finally {
    if (request === detailSeq) detailBusy.value = false;
  }
}
watch(() => [props.active, props.connectionId] as const, () => {
  anchorId.value = null;
  query.value = "";
  page.value = 1;
  void load();
}, { immediate: true });
watch(() => props.navigationToken, (current, previous) => {
  if (current !== previous && props.active) {
    anchorId.value = null;
    query.value = props.preferredQueryid ?? "";
    void load();
  }
});
watch([rank, query,()=>historySort.get("pgss-sql")], () => { page.value = 1; });
</script>

<template>
<section class="analysis" :aria-label="t('historyAttributes.l33')">
  <header class="analysis-head">
    <div>
      <h2>{{ t('historyDetails.PgssHeading') }}</h2>
      <p class="muted">{{ t('historyDetails.PgssDescription') }}</p>
    </div>
    <button class="secondary" type="button" @click="emit('openAnalyzer', null)">{{ t('historyDetails.PgssOpen') }}</button>
  </header>
  <p v-if="busy" role="status" class="muted">{{ t('historyDetails.PgssLoading') }}</p>
  <p v-if="error" role="alert" class="hist-error">{{error}}</p>
  <template v-if="data">
    <div class="metric-grid">
      <div class="metric"><small>{{ t('historyDetails.PgssCallsHour') }}</small><strong>{{humanValue(selectedInterval?.global_calls_per_hour)}}</strong><span>{{ t('historyDetails.PgssSelectedInterval') }}</span></div>
      <div class="metric"><small>{{ t('historyDetails.PgssSqlTime') }}</small><strong>{{formatPgssExecRate(selectedInterval?.global_exec_ms_per_hour)}}</strong><span>{{ t('historyDetails.PgssExecutionTime') }}</span></div>
      <div class="metric"><small>{{ t('historyDetails.PgssWalHour') }}</small><strong>{{formatBytesRate(selectedInterval?.sampled_wal_bytes_per_hour)}}</strong><span>{{ t('historyDetails.PgssMatchingQueries') }}</span></div>
      <div class="metric"><small>{{ t('historyDetails.PgssIntervals') }}</small><strong>{{comparableCount}} / {{data.intervals.length}}</strong><span>{{data.snapshot_count}}{{ t('histPgssFinal.snapshots') }}</span></div>
    </div>
    <p v-if="!comparableCount" class="warning" role="status">{{ t('historyDetails.PgssUnavailable') }}</p>
    <p class="muted">{{ t('historyDetails.PgssConcurrent') }}</p>

    <section class="trend">
      <div class="analysis-head"><h3>{{ t('historyDetails.PgssTrend') }}</h3>
        <label>{{ t('historyDetails.PgssMetric') }}
          <select v-model="field">
            <option value="global_exec_ms_per_hour">{{ t('historyDetails.PgssSqlHour') }}</option>
            <option value="global_calls_per_hour">{{ t('historyDetails.PgssCallsHourOption') }}</option>
            <option value="sampled_wal_bytes_per_hour">{{ t('historyDetails.PgssWalSample') }}</option>
            <option value="sampled_temp_blocks_per_hour">{{ t('historyDetails.PgssTempSample') }}</option>
          </select>
        </label>
      </div>
      <svg v-if="validPoints.length" viewBox="0 0 860 230" role="img" :aria-label="t('historyAttributes.l22')">
        <line x1="60" y1="188" x2="820" y2="188" stroke="currentColor" opacity=".4" />
        <text x="60" y="17" font-size="12" fill="currentColor">{{chartValue(chartMax)}}</text>
        <text x="55" y="190" text-anchor="end" font-size="12" fill="currentColor">0</text>
        <polyline v-for="(line, i) in lines" :key="i" :points="line" fill="none" stroke="var(--accent)" stroke-width="2.5" />
        <circle v-for="point in validPoints" :key="point.id" :cx="point.x" :cy="point.y" r="4" :fill="point.id === data.selected_to_snapshot_id ? 'var(--text)' : 'var(--accent)'"><title>{{formatHistoryTime(point.ms)}} · {{chartValue(point.value)}}</title></circle>
        <text x="60" y="217" font-size="12" fill="currentColor">{{formatHistoryTime(data.intervals[0].to_ms)}}</text>
        <text x="820" y="217" text-anchor="end" font-size="12" fill="currentColor">{{formatHistoryTime(data.intervals[data.intervals.length-1].to_ms)}}</text>
      </svg>
      <p v-else class="muted">{{ t('historyDetails.PgssNoIntervals') }}</p>
      <p class="muted">{{ t('historyDetails.PgssTimeline') }}</p>
    </section>

    <section class="trend">
      <div class="analysis-head"><div><h3>{{ t('historyDetails.PgssInvestigate') }}</h3><p class="muted">{{ t('historyDetails.PgssRanking') }}</p></div>
        <label>{{ t('historyDetails.PgssInterval') }}
          <select v-model.number="anchorId" :disabled="!safeIntervals.length || busy" @change="selectAnchor">
            <option v-for="item in safeIntervals" :key="item.to_snapshot_id" :value="item.to_snapshot_id">{{intervalLabel(item)}}</option>
          </select>
        </label>
      </div>
      <p v-if="selectedInterval" class="muted"><strong>{{ t('historyDetails.PgssSelected') }}</strong> {{intervalLabel(selectedInterval)}} · {{selectedInterval.comparable_statements}}{{ t('histPgssFinal.matching') }}{{selectedInterval.sampled_entries}}{{ t('histPgssFinal.sampled') }}</p>
      <p v-if="previousInterval" class="notice">{{ t('histPgssFinal.adjacent') }}{{intervalLabel(previousInterval)}}{{ t('histPgssFinal.regression') }}</p>
      <p v-else-if="selectedInterval" class="warning">{{ t('historyDetails.PgssNoPrevious') }}</p>
      <details><summary>{{ t('histPgssFinal.allIntervals') }}{{data.intervals.length}})</summary>
        <div class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l32')" :column="'to_ms'" :order="historySort.get('pgss-intervals')" @sort="historySort.toggle('pgss-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l36')" :column="'state'" :order="historySort.get('pgss-intervals')" @sort="historySort.toggle('pgss-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l35')" :column="'comparable_statements'" :order="historySort.get('pgss-intervals')" @sort="historySort.toggle('pgss-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l20')" :column="'global_calls_per_hour'" :order="historySort.get('pgss-intervals')" @sort="historySort.toggle('pgss-intervals', $event)" /><HistorySortTh :label="t('historyAttributes.l24')" :column="'global_exec_ms_per_hour'" :order="historySort.get('pgss-intervals')" @sort="historySort.toggle('pgss-intervals', $event)" /></tr></thead><tbody>
          <tr v-for="item in historySort.sorted('pgss-intervals', [...data.intervals].reverse())" :key="item.to_snapshot_id"><td>{{intervalLabel(item)}}</td><td>{{item.state}}</td><td>{{item.comparable_statements}} / {{item.sampled_entries}}</td><td>{{humanValue(item.global_calls_per_hour)}}</td><td>{{formatPgssExecRate(item.global_exec_ms_per_hour)}}</td></tr>
        </tbody></table></div>
      </details>
    </section>

    <section class="trend">
      <div class="analysis-head">
        <div><h3>{{ t('historyDetails.PgssSqlRanking') }}</h3><p class="muted">{{ t('historyDetails.PgssRankingHint') }}</p></div>
        <div class="filters">
          <label>{{ t('historyDetails.PgssShow') }}
            <select v-model="rank"><option value="exec">{{ t('historyDetails.PgssMaxSql') }}</option><option value="impact" :disabled="!comparisonReady">{{ t('historyDetails.PgssSqlGrowth') }}</option><option value="latency" :disabled="!comparisonReady">{{ t('historyDetails.PgssLatencyGrowth') }}</option><option value="calls">{{ t('historyDetails.PgssCallsGrowth') }}</option><option value="wal">{{ t('historyDetails.PgssWalGrowth') }}</option><option value="temp">{{ t('historyDetails.PgssTempGrowth') }}</option></select>
          </label>
          <label>{{ t('historyDetails.PgssSearch') }}<input v-model="query" :placeholder="t('historyAttributes.l38')" /></label>
        </div>
      </div>
      <p v-if="comparisonReady && !hasRegression" class="muted">{{ t('historyDetails.PgssNoMatches') }}</p>
      <div class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l23')" :column="'queryid'" :order="historySort.get('pgss-sql')" @sort="historySort.toggle('pgss-sql', $event)" /><HistorySortTh :label="t('historyAttributes.l25')" :column="'latest.exec_ms_per_hour'" :order="historySort.get('pgss-sql')" @sort="historySort.toggle('pgss-sql', $event)" /><HistorySortTh :label="t('historyAttributes.l29')" :column="'latest.mean_exec_ms'" :order="historySort.get('pgss-sql')" @sort="historySort.toggle('pgss-sql', $event)" /><HistorySortTh :label="t('historyAttributes.l30')" :column="'latest.calls_per_hour'" :order="historySort.get('pgss-sql')" @sort="historySort.toggle('pgss-sql', $event)" /><HistorySortTh :label="t('historyAttributes.l26')" :column="'latest.wal_bytes_per_hour'" :order="historySort.get('pgss-sql')" @sort="historySort.toggle('pgss-sql', $event)" /><th>{{ t('historyDetails.PgssAction') }}</th></tr></thead><tbody>
        <tr v-for="row in shown" :key="`${row.userid}/${row.dbid}/${row.queryid}/${row.toplevel}`">
          <td><strong>queryid {{row.queryid}}</strong><small>{{row.role_name ?? row.userid}} / {{row.database ?? row.dbid}} · {{row.toplevel ? 'top-level' : 'nested'}}</small></td>
          <td>{{formatPgssExecRate(row.latest.exec_ms_per_hour)}}<small v-if="row.previous">{{ t('histPgssFinal.change') }}{{formatPgssExecRate(row.latest.exec_ms_per_hour-row.previous.exec_ms_per_hour)}}</small></td>
          <td>{{formatPgssDuration(row.latest.mean_exec_ms)}}<small v-if="row.previous">{{percentDiff(row.latest.mean_exec_ms,row.previous.mean_exec_ms)}}</small></td>
          <td>{{humanValue(row.latest.calls_per_hour)}}</td><td>{{formatBytesRate(row.latest.wal_bytes_per_hour)}}</td>
          <td><button type="button" class="secondary" @click="openStatement(row)">{{ t('historyDetails.PgssSqlHistory') }}</button></td>
        </tr>
        <tr v-if="!shown.length"><td colspan="6">{{ t('historyDetails.PgssNoQueries') }}</td></tr>
      </tbody></table></div>
      <div class="pager"><span>{{ t('histPgssFinal.queries') }}{{sorted.length}} · {{page}} / {{pages}}</span><button class="secondary" type="button" :disabled="page<=1" @click="page--">{{ t('historyDetails.PgssPrevious') }}</button><button class="secondary" type="button" :disabled="page>=pages" @click="page++">{{ t('historyDetails.PgssNext') }}</button></div>
    </section>

    <section v-if="selected" class="trend">
      <div class="analysis-head"><div><h3>{{ t('histPgssFinal.queryHistory') }}{{selected.queryid}}</h3><p class="muted">{{selected.role_name ?? selected.userid}} / {{selected.database ?? selected.dbid}}{{ t('histPgssFinal.dataBound') }}{{data.selected_to_snapshot_id}}</p></div>
        <button class="secondary" type="button" @click="emit('openAnalyzer', selected.queryid)">{{ t('historyDetails.PgssOpenLive') }}</button>
      </div>
      <p class="muted">{{ t('historyDetails.PgssSqlPrivacy') }}</p>
      <p v-if="detailBusy" class="muted">{{ t('historyDetails.PgssLoadingStatement') }}</p>
      <p v-if="detailError" role="alert" class="hist-error">{{detailError}}</p>
      <template v-if="detail">
        <p>{{ t('histPgssFinal.comparableIntervals') }}{{detail.summary.comparable_intervals}}</p>
        <div class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l28')" :column="'collection_started_at_ms'" :order="historySort.get('pgss-points')" @sort="historySort.toggle('pgss-points', $event)" /><HistorySortTh :label="t('historyAttributes.l36')" :column="'delta_state'" :order="historySort.get('pgss-points')" @sort="historySort.toggle('pgss-points', $event)" /><HistorySortTh :label="t('historyAttributes.l21')" :column="'calls_per_hour'" :order="historySort.get('pgss-points')" @sort="historySort.toggle('pgss-points', $event)" /><HistorySortTh :label="t('historyAttributes.l29')" :column="'interval_mean_exec_time_ms'" :order="historySort.get('pgss-points')" @sort="historySort.toggle('pgss-points', $event)" /><HistorySortTh label="Δ WAL" :column="'wal_bytes_delta'" :order="historySort.get('pgss-points')" @sort="historySort.toggle('pgss-points', $event)" /><HistorySortTh label="Δ temp blocks" :column="'temp_blks_written_delta'" :order="historySort.get('pgss-points')" @sort="historySort.toggle('pgss-points', $event)" /></tr></thead><tbody>
          <tr v-for="item in historySort.sorted('pgss-points', [...detail.points].reverse()).slice(0,40)" :key="item.snapshot_id"><td>{{formatHistoryTime(item.collection_started_at_ms)}}</td><td>{{item.delta_state}}</td><td>{{humanValue(item.calls_per_hour)}}</td><td>{{formatPgssDuration(item.interval_mean_exec_time_ms)}}</td><td>{{humanValue(item.wal_bytes_delta)}}</td><td>{{humanValue(item.temp_blks_written_delta)}}</td></tr>
        </tbody></table></div>
        <details v-if="detail.change_analysis.items.length"><summary>{{ t('histPgssFinal.metricChanges') }}{{detail.change_analysis.items.length}})</summary><div class="hist-scroll"><table><thead><tr><HistorySortTh :label="t('historyAttributes.l34')" :column="'label'" :order="historySort.get('pgss-metrics')" @sort="historySort.toggle('pgss-metrics', $event)" /><HistorySortTh :label="t('historyAttributes.l27')" :column="'older_value'" :order="historySort.get('pgss-metrics')" @sort="historySort.toggle('pgss-metrics', $event)" /><HistorySortTh :label="t('historyAttributes.l37')" :column="'newer_value'" :order="historySort.get('pgss-metrics')" @sort="historySort.toggle('pgss-metrics', $event)" /><HistorySortTh :label="t('historyAttributes.l31')" :column="'relative_change_pct'" :order="historySort.get('pgss-metrics')" @sort="historySort.toggle('pgss-metrics', $event)" /></tr></thead><tbody><tr v-for="item in historySort.sorted('pgss-metrics',detail.change_analysis.items)" :key="item.id"><td>{{item.label}} · {{item.unit}}</td><td>{{humanValue(item.older_value)}}</td><td>{{humanValue(item.newer_value)}}</td><td>{{percentDiff(item.newer_value,item.older_value)}}</td></tr></tbody></table></div></details>
        <details v-if="detail.limitations.length"><summary>{{ t('historyDetails.PgssStatementLimitations') }}</summary><ul><li v-for="item in detail.limitations" :key="item">{{item}}</li></ul></details>
      </template>
    </section>
    <details class="limitations"><summary>{{ t('histPgssFinal.method') }}</summary><ul><li v-for="item in data.limitations" :key="item">{{item}}</li></ul></details>
  </template>
</section>
</template>
<style scoped>
.analysis{display:grid;gap:1rem;min-width:0}
.analysis-head{display:flex;justify-content:space-between;align-items:start;gap:1rem;flex-wrap:wrap}
.analysis h2,.analysis h3{margin:.15rem 0 .5rem}.analysis .muted,.analysis small{color:var(--text-secondary);font-size:.86rem}
.metric-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.65rem}
.metric{min-width:0;display:grid;gap:.4rem;background:var(--surface-2);border-radius:.5rem;padding:.85rem}
.metric strong{font-variant-numeric:tabular-nums;font-size:1.2rem}.metric span,.metric small{color:var(--text-secondary);font-size:.8rem}
.trend{min-width:0;border:1px solid var(--border);border-radius:.5rem;padding:1rem}.trend svg{width:100%;max-width:1000px;height:auto;display:block}
.filters{display:flex;gap:.65rem;flex-wrap:wrap}.analysis label{display:grid;gap:.3rem;font-size:.85rem}
.analysis select,.analysis input{min-width:160px;max-width:100%;background:var(--surface-2);border:1px solid var(--border-strong);border-radius:.3rem;padding:.5rem;color:var(--text)}
.hist-scroll{max-height:480px;overflow:auto}.hist-scroll table{width:100%;border-collapse:collapse;text-align:left;font-size:.85rem}
.hist-scroll th,.hist-scroll td{padding:.6rem;border-bottom:1px solid var(--border);vertical-align:top}.hist-scroll small{display:block;margin:.35rem 0}
.pager{display:flex;gap:.7rem;justify-content:flex-end;align-items:center;margin-top:.7rem}.hist-error{color:var(--error)}.warning{color:var(--accent)}.notice{color:var(--accent)}
@media(max-width:1150px){.metric-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:640px){.metric-grid{grid-template-columns:1fr}.trend{padding:.65rem}}
</style>
