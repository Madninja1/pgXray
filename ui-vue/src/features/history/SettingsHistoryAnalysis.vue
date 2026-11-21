<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed, ref, shallowRef, watch } from "vue";
import HistorySortTh from "./HistorySortTh.vue";
import { useHistorySorting } from "./use-history-sorting.js";
import { historySortValue } from "./sort-rows.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { formatHistoryTime } from "./model.js";
import { settingsChanges, settingsIntervalIsComparable, settingsIntervalLabel,
  settingsParameterHistory, settingsValue,
  type SettingsChangeFilter, type SettingsHistoryAnalysis } from "./settings-insights.js";

const historySort = useHistorySorting();
const props = defineProps<{ connectionId: number | null; active: boolean; invoke: PgXrayInvoke; navigationToken?: number }>();
const emit = defineEmits<{openCurrent: []}>();
const data = shallowRef<SettingsHistoryAnalysis | null>(null);
const error = ref(""); const busy = ref(false);
const selectedId = ref<number | null>(null);
const search = ref(""); const filter = ref<SettingsChangeFilter>("all");
const revealValues = ref(false); const page = ref(1); const pageSize = 20;
const chosenParameter = ref<string | null>(null);
let request = 0;
const intervals = computed(() => data.value?.intervals ?? []);
const selectedInterval = computed(() => intervals.value.find(row => row.to_snapshot_id === selectedId.value) ?? null);
const comparableCount = computed(() => intervals.value.filter(settingsIntervalIsComparable).length);
const changedCount = computed(() => intervals.value.filter(settingsIntervalIsComparable).reduce((sum,row) => sum + row.changes.length,0));
const changedIntervals = computed(() => intervals.value.filter(row => settingsIntervalIsComparable(row) && row.changes.length).length);
const filteredChanges = computed(() => settingsChanges(selectedInterval.value, filter.value, search.value));
const sortedChanges = computed(() => historySort.sorted("settings-changes",filteredChanges.value, (row,column) => column === "value_changed" ? row.value_changed : column === "newer_value" && !revealValues.value ? null : historySortValue(row,column)));
const pages = computed(() => Math.max(1,Math.ceil(filteredChanges.value.length/pageSize)));
const pageRows = computed(() => sortedChanges.value.slice((page.value-1)*pageSize,page.value*pageSize));
const parameterEvents = computed(() => historySort.sorted("settings-parameter",data.value && chosenParameter.value ? settingsParameterHistory(data.value, chosenParameter.value) : [], (row,column)=>column === "change.newer_value" && !revealValues.value ? null : historySortValue(row,column)));
const format = (n: number): string => n.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US");
const windowLabel = (row: {from_snapshot_id:number;to_snapshot_id:number;from_ms:number;to_ms:number}) =>
  `#${row.from_snapshot_id} → #${row.to_snapshot_id} · ${formatHistoryTime(row.from_ms)} — ${formatHistoryTime(row.to_ms)}`;
function resetPrivateState():void {revealValues.value=false;chosenParameter.value=null;page.value=1;}
async function load():Promise<void> {
  const id=props.connectionId, call=++request;
  resetPrivateState();data.value=null;error.value="";
  if (id===null || !props.active) {busy.value=false;return;}
  busy.value=true;
  try {
    const result=await props.invoke<SettingsHistoryAnalysis>("pg_settings_history_analysis",{connectionId:id,limit:80});
    if (call!==request || !props.active || props.connectionId!==id) return;
    data.value=result;
    if (!result.intervals.some(row=>row.to_snapshot_id===selectedId.value)) {
      selectedId.value=result.intervals.find(row=>settingsIntervalIsComparable(row) && row.changes.length)?.to_snapshot_id
        ?? result.intervals[0]?.to_snapshot_id ?? null;
    }
  } catch (e) {if(call===request) error.value=String(e);}
  finally {if(call===request) busy.value=false;}
}
watch(() => [props.connectionId,props.active,props.navigationToken] as const, () => {selectedId.value=null;void load();}, {immediate:true});
watch([filter,search,selectedId],()=>{page.value=1;chosenParameter.value=null;revealValues.value=false;});
watch(() => historySort.get("settings-changes"),()=>{page.value=1;});
</script>
<template>
<section class="sethist" :aria-label="t('historyResidual.settingsAria')">
  <header class="sethist-head"><div><h2>{{ t('historyDetails.SettingsHeading') }}</h2>
    <p class="muted">{{ t('historyDetails.SettingsDescription') }}</p></div>
    <button type="button" class="secondary" @click="emit('openCurrent')">{{ t('historyDetails.SettingsOpen') }}</button>
  </header>
  <p v-if="error" role="alert" class="sethist-error">{{error}}</p>
  <p v-if="busy" class="muted">{{ t('historyDetails.SettingsLoading') }}</p>
  <template v-if="data && !busy">
    <div class="sethist-metrics">
      <article><small>{{ t('historyDetails.SettingsSnapshots') }}</small><strong>{{format(data.snapshot_count)}}</strong><small>{{ t('historyDetails.SettingsLast80') }}</small></article>
      <article><small>{{ t('historyDetails.SettingsComparisons') }}</small><strong>{{comparableCount}} / {{intervals.length}}</strong><small>{{ t('historyDetails.SettingsRestartSeparate') }}</small></article>
      <article><small>{{ t('historyDetails.SettingsChangedIntervals') }}</small><strong>{{changedIntervals}}</strong><small>{{ t('historyDetails.SettingsComparable') }}</small></article>
      <article><small>{{ t('historyDetails.SettingsEvents') }}</small><strong>{{changedCount}}</strong><small>{{ t('historyDetails.SettingsEventsHint') }}</small></article>
    </div>
    <p v-if="data.snapshot_count < 2" class="sethist-note">{{ t('historyDetails.SettingsNeedTwo') }}</p>
    <div v-else class="sethist-panel">
      <h3>{{ t('historyDetails.SettingsTimeline') }}</h3>
      <p class="muted">{{ t('historyDetails.SettingsTimeHint') }}</p>
      <div class="sethist-timeline" role="list" :aria-label="t('historyResidual.settingsIntervalsAria')">
        <button v-for="row in intervals" :key="row.to_snapshot_id" type="button" role="listitem"
          :class="{selected:selectedId===row.to_snapshot_id, gap:!settingsIntervalIsComparable(row)}"
          @click="selectedId=row.to_snapshot_id">
          <strong>{{formatHistoryTime(row.to_ms)}}</strong>
          <span>#{{row.from_snapshot_id}} → #{{row.to_snapshot_id}}</span>
          <small>{{settingsIntervalLabel(row.state)}}</small>
          <small v-if="settingsIntervalIsComparable(row)">{{ t('historyResidual.settingsCount', { count: String(row.changes.length) }) }}</small>
        </button>
      </div>
    </div>
    <div v-if="selectedInterval" class="sethist-panel">
      <h3>{{ t('historyDetails.SettingsSelectedChanges') }}</h3>
      <p class="muted">{{windowLabel(selectedInterval)}} · {{settingsIntervalLabel(selectedInterval.state)}}</p>
      <p v-if="selectedInterval.reason" class="sethist-note">{{selectedInterval.reason}}</p>
      <div v-if="settingsIntervalIsComparable(selectedInterval)" class="sethist-mini">
        <span>{{ t('historyDetails.SettingsValues') }} <strong>{{selectedInterval.value_change_count}}</strong></span>
        <span>{{ t('historyDetails.SettingsSources') }} <strong>{{selectedInterval.source_change_count}}</strong></span>
        <span>{{ t('historyDetails.SettingsRestart') }} <strong>{{selectedInterval.pending_restart_change_count}}</strong></span>
      </div>
      <div v-if="settingsIntervalIsComparable(selectedInterval)" class="sethist-controls">
        <label>{{ t('historyDetails.SettingsFilter') }}<select v-model="filter">
          <option value="all">{{ t('historyDetails.SettingsAllChanges') }}</option><option value="value">{{ t('historyDetails.SettingsValueChanged') }}</option>
          <option value="source">{{ t('historyDetails.SettingsSourceChanged') }}</option><option value="restart">{{ t('historyDetails.SettingsRestartChanged') }}</option>
          <option value="added">{{ t('historyDetails.SettingsAddedRemoved') }}</option>
        </select></label>
        <label>{{ t('historyDetails.SettingsSearch') }}<input v-model="search" :placeholder="t('historyDetails.SettingsSearchHint')" /></label>
        <label class="sethist-reveal"><input v-model="revealValues" type="checkbox" /> {{ t('historyDetails.SettingsReveal') }}</label>
      </div>
      <p class="muted">{{ t('historyDetails.SettingsPrivacy') }}</p>
      <p v-if="settingsIntervalIsComparable(selectedInterval) && !selectedInterval.changes.length" class="sethist-note">{{ t('historyDetails.SettingsNoDifferences') }}</p>
      <p v-else-if="settingsIntervalIsComparable(selectedInterval) && !filteredChanges.length" class="muted">{{ t('historyDetails.SettingsNoFilterMatch') }}</p>
      <div v-if="pageRows.length" class="sethist-scroll"><table><thead><tr><HistorySortTh :label="t('historyResidual.settingsParam')" :column="'name'" :order="historySort.get('settings-changes')" @sort="historySort.toggle('settings-changes', $event)" /><HistorySortTh :label="t('historyResidual.settingsBefore')" :column="revealValues ? 'newer_value' : 'value_changed'" :order="historySort.get('settings-changes')" @sort="historySort.toggle('settings-changes', $event)" /><HistorySortTh :label="t('historyResidual.settingsSource')" :column="'newer_source'" :order="historySort.get('settings-changes')" @sort="historySort.toggle('settings-changes', $event)" /><HistorySortTh label="Restart" :column="'newer_pending_restart'" :order="historySort.get('settings-changes')" @sort="historySort.toggle('settings-changes', $event)" /><th></th></tr></thead><tbody>
        <tr v-for="change in pageRows" :key="change.name">
          <td><strong>{{change.name}}</strong><small>{{change.category}} · {{change.state}}</small></td>
          <td>{{settingsValue(change.older_value,change.older_unit,revealValues)}} → {{settingsValue(change.newer_value,change.newer_unit,revealValues)}}
            <small v-if="!change.value_changed && (change.source_changed || change.pending_restart_changed)">{{ t('historyDetails.SettingsUnchanged') }}</small></td>
          <td>{{change.older_source ?? '—'}} → {{change.newer_source ?? '—'}}
            <small v-if="revealValues && (change.older_sourcefile || change.newer_sourcefile)">{{ t('historyResidual.settingsFiles') }} {{change.older_sourcefile ?? '—'}} → {{change.newer_sourcefile ?? '—'}}</small></td>
          <td>{{change.older_pending_restart === null ? '—' : change.older_pending_restart ? t('historyResidual.settingsYes') : t('historyResidual.settingsNo')}} → {{change.newer_pending_restart === null ? '—' : change.newer_pending_restart ? t('historyResidual.settingsYes') : t('historyResidual.settingsNo')}}</td>
          <td><button class="secondary" type="button" @click="chosenParameter=change.name">{{ t('historyDetails.SettingsHistory') }}</button></td>
        </tr>
      </tbody></table></div>
      <div v-if="filteredChanges.length>pageSize" class="sethist-pager"><span>{{ t('historyResidual.settingsPage', { count: format(filteredChanges.length), page: String(page), pages: String(pages) }) }}</span><button type="button" class="secondary" :disabled="page<=1" @click="page--">{{ t('historyDetails.SettingsPrevious') }}</button><button type="button" class="secondary" :disabled="page>=pages" @click="page++">{{ t('historyDetails.SettingsNext') }}</button></div>
    </div>
    <div v-if="chosenParameter" class="sethist-panel">
      <div class="sethist-head"><div><h3>{{ t('historyResidual.settingsTitle', { name: chosenParameter }) }}</h3><p class="muted">{{ t('historyDetails.SettingsParameterHint') }}</p></div><button type="button" class="secondary" @click="chosenParameter=null">{{ t('historyDetails.SettingsClose') }}</button></div>
      <div v-if="parameterEvents.length" class="sethist-scroll"><table><thead><tr><HistorySortTh :label="t('historyResidual.settingsWindow')" :column="'interval.to_ms'" :order="historySort.get('settings-parameter')" @sort="historySort.toggle('settings-parameter', $event)" /><HistorySortTh :label="t('historyResidual.settingsValue')" :column="'change.newer_value'" :order="historySort.get('settings-parameter')" @sort="historySort.toggle('settings-parameter', $event)" /><HistorySortTh :label="t('historyResidual.settingsSource')" :column="'change.newer_source'" :order="historySort.get('settings-parameter')" @sort="historySort.toggle('settings-parameter', $event)" /><HistorySortTh label="Restart" :column="'change.newer_pending_restart'" :order="historySort.get('settings-parameter')" @sort="historySort.toggle('settings-parameter', $event)" /></tr></thead><tbody>
        <tr v-for="row in parameterEvents" :key="row.interval.to_snapshot_id">
          <td>{{windowLabel(row.interval)}}<small>{{settingsIntervalLabel(row.interval.state)}}</small></td>
          <td>{{settingsValue(row.change.older_value,row.change.older_unit,revealValues)}} → {{settingsValue(row.change.newer_value,row.change.newer_unit,revealValues)}}</td>
          <td>{{row.change.older_source ?? '—'}} → {{row.change.newer_source ?? '—'}}</td>
          <td>{{row.change.newer_pending_restart ? t('historyResidual.settingsPending') : t('historyResidual.settingsNo')}}</td>
        </tr>
      </tbody></table></div>
    </div>
    <details class="sethist-panel"><summary>{{ t('historyDetails.SettingsMethodology') }}</summary><ul><li v-for="line in data.limitations" :key="line">{{line}}</li></ul></details>
  </template>
</section>
</template>
<style scoped>
.sethist{display:grid;gap:1rem;min-width:0}.sethist-head{display:flex;justify-content:space-between;align-items:start;gap:1rem;flex-wrap:wrap}.sethist h2,.sethist h3{margin:.15rem 0 .5rem}.sethist h2{font-size:1.25rem}.sethist .muted,.sethist small{color:var(--text-secondary);font-size:.87rem}.sethist p{line-height:1.45}.sethist-metrics{display:grid;grid-template-columns:repeat(auto-fit,minmax(165px,1fr));gap:.7rem}.sethist-metrics article{display:grid;gap:.35rem;background:var(--surface-2);padding:1rem;border-radius:.55rem}.sethist-metrics strong{font-size:1.25rem}.sethist-panel{border:1px solid var(--border-strong);border-radius:.6rem;padding:1rem;min-width:0}.sethist-note{background:var(--surface-2);padding:.7rem;border-radius:.5rem}.sethist-timeline{display:flex;gap:.5rem;overflow-x:auto;padding:.4rem 0 .7rem}.sethist-timeline button{flex:0 0 190px;text-align:left;display:grid;gap:.3rem;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.5rem;padding:.7rem}.sethist-timeline button.selected{border-color:var(--accent)}.sethist-timeline button.gap{border-style:dashed}.sethist-timeline span,.sethist-timeline small{font-size:.79rem}.sethist-controls{display:flex;align-items:end;gap:.7rem;flex-wrap:wrap;margin:.7rem 0}.sethist-controls label{display:grid;gap:.25rem}.sethist-controls input,.sethist-controls select{max-width:100%;min-width:180px}.sethist-controls .sethist-reveal{display:flex;align-items:center;gap:.5rem}.sethist-reveal input{min-width:0}.sethist-mini{display:flex;gap:1.3rem;flex-wrap:wrap;margin:.8rem 0}.sethist-scroll{overflow:auto;max-height:480px}.sethist table{width:100%;text-align:left;border-collapse:collapse;font-size:.87rem}.sethist th,.sethist td{padding:.65rem;border-bottom:1px solid var(--border);vertical-align:top}.sethist td small{display:block;margin-top:.3rem}.sethist-pager{display:flex;align-items:center;justify-content:flex-end;gap:.6rem;margin-top:.6rem}.sethist-error{color:var(--error)}.sethist details summary{cursor:pointer;font-weight:600}@media(max-width:900px){.sethist-timeline button{flex-basis:165px}}
</style>
