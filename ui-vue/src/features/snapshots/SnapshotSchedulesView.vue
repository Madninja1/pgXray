<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { locale, t } from "../i18n/locale-service.js";
import { computed, onMounted, reactive, ref } from "vue";
import {
  ALL_WEEKDAYS, KIND_LABELS, MIN_INTERVAL_BY_KIND, SNAPSHOT_CADENCES, SNAPSHOT_KINDS,
  nextDaily, nextIntervalSlot, validateSchedule, displayScheduleError,
  type ScheduleDraft, type SnapshotKind, type SnapshotSchedule,
} from "./schedule-model.js";
import type { SnapshotSchedulerActions, SnapshotSchedulerState } from "./scheduler.js";
import {
  scheduleListGroups, sortedVisibleSchedules, upcomingAttempt,
  type ScheduleListDirection, type ScheduleListSort,
} from "./schedule-list.js";

function localizedMode(mode: "interval" | "daily" | "series"): string {
  return mode === "interval" ? t('snapshots.mode_interval') :
    mode === "daily" ? t('snapshots.mode_daily') : t('snapshots.mode_series');
}
const props = defineProps<{ state: SnapshotSchedulerState; actions: SnapshotSchedulerActions }>();
const draft = reactive<ScheduleDraft>({
  connectionId: 0, kind: "pgss", mode: "interval", intervalMinutes: 60,
  windowStart: "08:00", windowEnd: "21:00", intervalAnchor: "09:05",
  daysOfWeek: [...ALL_WEEKDAYS], dailyTime: "09:00", seriesCount: 3, seriesSpanMinutes: 60,
});
const weekDays = computed(() => [
  { value: 1, label: t('snapshots.mon') }, { value: 2, label: t('snapshots.tue') }, { value: 3, label: t('snapshots.wed') },
  { value: 4, label: t('snapshots.thu') }, { value: 5, label: t('snapshots.fri') }, { value: 6, label: t('snapshots.sat') },
  { value: 0, label: t('snapshots.sun') },
]);
const editingKey = ref<string | null>(null);
const preview = computed(() => {
  if (draft.mode === "series") return [] as number[];
  try {
    validateSchedule({ ...draft, connectionId: draft.connectionId || 1 });
    const slots: number[] = [];
    let cursor = Date.now();
    for (let i = 0; i < 6; i++) {
      cursor = draft.mode === "interval" ? nextIntervalSlot(cursor, draft, i === 0) :
        nextDaily(cursor, draft.dailyTime, draft.daysOfWeek);
      slots.push(cursor);
    }
    return slots;
  } catch { return []; }
});
const saving = computed(() => props.state.owner === null || props.state.loading ||
  !props.state.connections.some(c => c.id === draft.connectionId));
// Filters and sorting are presentation-only: SQLite rules and collector order remain unchanged.
const selectedConnections = ref<number[] | null>(null); // null = all saved connections
const sortBy = ref<ScheduleListSort>("nextAt");
const sortDirection = ref<ScheduleListDirection>("asc");
const groupByMode = ref(false);
const connectionOptions = computed(() => {
  const options = new Map(props.state.connections.map(connection => [connection.id, {
    id: connection.id,
    label: `${connection.name} · ${connection.host}:${connection.port}/${connection.database}`,
  }]));
  // Keep rules from temporarily unavailable/deleted connection profiles filterable.
  for (const rule of props.state.rules) {
    if (!options.has(rule.connectionId))
      options.set(rule.connectionId, {id: rule.connectionId, label: `${rule.targetName} · ID ${rule.connectionId}`});
  }
  return [...options.values()].sort((a, b) => a.label.localeCompare(b.label, "ru") || a.id - b.id);
});
const connectionSummary = computed(() => {
  const selected = selectedConnections.value;
  if (selected === null) return t('snapshots.allConnectionsCount', { count: String(connectionOptions.value.length) });
  if (selected.length === 0) return t('snapshots.none_selected');
  if (selected.length === 1) return connectionOptions.value.find(c => c.id === selected[0])?.label ?? `ID ${selected[0]}`;
  return t('snapshots.selectedOf', { selected: String(selected.length), total: String(connectionOptions.value.length) });
});
function isConnectionSelected(id: number): boolean {
  return selectedConnections.value === null || selectedConnections.value.includes(id);
}
function toggleConnection(id: number): void {
  const current = selectedConnections.value ?? connectionOptions.value.map(c => c.id);
  const next = current.includes(id) ? current.filter(value => value !== id) : [...current, id];
  selectedConnections.value = next.length === connectionOptions.value.length ? null : next;
}
const rules = computed(() => sortedVisibleSchedules(
  props.state.rules, selectedConnections.value, sortBy.value, sortDirection.value,
));
const groups = computed(() => scheduleListGroups(rules.value, groupByMode.value,
  sortBy.value === "mode" && sortDirection.value === "desc"));
function date(value: number | null): string {
  return value && value > 0 ? new Date(value).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US") : "—";
}
function status(rule: SnapshotSchedule): string {
  if (rule.status === "complete") return t('snapshots.status_complete', { done: String(rule.seriesCompleted), total: String(rule.seriesCount) });
  if (rule.status === "expired") return t('snapshots.status_expired', { done: String(rule.seriesCompleted), total: String(rule.seriesCount) });
  if (!rule.enabled) return t('snapshots.paused');
  if (props.state.collectingKey === rule.key) return t('snapshots.collecting');
  if (rule.mode === "series") return t('snapshots.seriesProgress', { done: String(rule.seriesCompleted), total: String(rule.seriesCount) });
  return t('snapshots.automatic_collection_active');
}
async function refresh(): Promise<void> {
  await Promise.all([props.actions.refreshConnections(), props.actions.refreshRules()]);
}
// This component is mounted on each visit to the Settings → Snapshots tab.
// SQLite is authoritative even when rules were edited in another pgXray instance.
onMounted(() => { void refresh(); });
function description(rule: SnapshotSchedule): string {
  const days = rule.daysOfWeek.length === 7 ? t('snapshots.daily') :
    weekDays.value.filter(day => rule.daysOfWeek.includes(day.value)).map(day => day.label).join(", ");
  if (rule.mode === "daily") return t('snapshots.dailyAt', { days, time: rule.dailyTime });
  if (rule.mode === "series") return t('snapshots.measurements', { count: String(rule.seriesCount), minutes: String(rule.seriesSpanMinutes) });
  return t('snapshots.intervalDescription', { minutes: String(rule.intervalMinutes), anchor: rule.intervalAnchor, start: rule.windowStart, end: rule.windowEnd, days });
}
async function save(): Promise<void> {
  if (await props.actions.saveRule({ ...draft, daysOfWeek: [...draft.daysOfWeek] }, editingKey.value)) editingKey.value = null;
}
function edit(rule: SnapshotSchedule): void {
  Object.assign(draft, {
    connectionId: rule.connectionId, kind: rule.kind, mode: rule.mode,
    intervalMinutes: rule.intervalMinutes, windowStart: rule.windowStart, windowEnd: rule.windowEnd,
    intervalAnchor: rule.intervalAnchor, daysOfWeek: [...rule.daysOfWeek], dailyTime: rule.dailyTime,
    seriesCount: rule.seriesCount, seriesSpanMinutes: rule.seriesSpanMinutes,
  });
  editingKey.value = rule.key;
  document.getElementById("snapshot-new-title")?.scrollIntoView({ behavior: "smooth", block: "start" });
}
function changeKind(value: string): void {
  if (SNAPSHOT_KINDS.includes(value as SnapshotKind)) {
    draft.kind = value as SnapshotKind;
    if (draft.intervalMinutes < MIN_INTERVAL_BY_KIND[draft.kind])
      draft.intervalMinutes = Math.min(...SNAPSHOT_CADENCES.filter(v => v >= MIN_INTERVAL_BY_KIND[draft.kind]));
  }
}
function onKindChange(event: Event): void { changeKind((event.target as HTMLSelectElement).value); }
async function remove(rule: SnapshotSchedule): Promise<void> {
  if (await confirmAction(t('snapshots.deleteConfirm', { kind: KIND_LABELS[rule.kind], name: rule.targetName })))
    void props.actions.removeRule(rule.key);
}
</script>

<template>
  <section class="snapshot-settings" aria-labelledby="snapshot-settings-title">
    <header class="snapshot-header">
      <div>
        <h2 id="snapshot-settings-title">{{ t('snapshots.snapshots_schedules') }}</h2>
        <p class="snapshot-muted">{{ t('snapshots.shared_automated_diagnostic_collection_for_saved_pos') }}</p>
      </div>
    </header>

    <div class="snapshot-note">
      <strong>{{ t('snapshots.when_it_runs') }}</strong> {{ t('snapshots.only_while_pgxray_is_open_and_the_user_is_signed_in_') }}
    </div>
    <p v-if="state.error" class="snapshot-error" role="alert">{{ state.error }}</p>
    <p v-if="state.notice" class="snapshot-notice" role="status">{{ state.notice }}</p>

    <section class="snapshot-panel" aria-labelledby="snapshot-new-title">
      <h3 id="snapshot-new-title">{{ editingKey ? t('snapshots.edit_schedule') : t('snapshots.add_schedule_or_series') }}</h3>
      <div class="snapshot-form-grid">
        <label>{{ t('snapshots.connection') }}
          <select v-model.number="draft.connectionId" :disabled="editingKey !== null">
            <option :value="0">{{ t('snapshots.select_a_connection') }}</option>
            <option v-for="c in state.connections" :key="c.id" :value="c.id">
              {{ c.name }} · {{ c.host }}:{{ c.port }}/{{ c.database }}
            </option>
          </select>
        </label>
        <label>{{ t('snapshots.diagnostic_module') }}
          <select :value="draft.kind" :disabled="editingKey !== null" @change="onKindChange">
            <option v-for="kind in SNAPSHOT_KINDS" :key="kind" :value="kind">{{ KIND_LABELS[kind] }}</option>
          </select>
        </label>
        <label>{{ t('snapshots.mode') }}
          <select v-model="draft.mode">
            <option value="interval">{{ t('snapshots.at_intervals_during_allowed_hours') }}</option>
            <option value="daily">{{ t('snapshots.once_a_day_at_a_set_time') }}</option>
            <option value="series">{{ t('snapshots.snapshot_series_over_a_period') }}</option>
          </select>
        </label>
        <template v-if="draft.mode === 'interval'">
          <label>{{ t('snapshots.interval') }}
            <select v-model.number="draft.intervalMinutes">
              <option v-for="minutes in SNAPSHOT_CADENCES.filter(value => value >= MIN_INTERVAL_BY_KIND[draft.kind])" :key="minutes" :value="minutes">
                {{ minutes < 60 ? t('snapshots.minutes', { count: String(minutes) }) : minutes === 1440 ? t('snapshots.24_hours') : t('snapshots.hours', { count: String(minutes / 60) }) }}
              </option>
            </select>
          </label>
          <label>{{ t('snapshots.allowed_window_start') }} <input v-model="draft.windowStart" type="time" /></label>
          <label>{{ t('snapshots.allowed_window_end_exclusive') }} <input v-model="draft.windowEnd" type="time" /></label>
          <label>{{ t('snapshots.first_slot_of_each_day') }} <input v-model="draft.intervalAnchor" type="time" /></label>
          <p class="snapshot-muted snapshot-full">{{ t('snapshots.windowExample', { minutes: String(MIN_INTERVAL_BY_KIND[draft.kind]) }) }}</p>
        </template>
        <template v-else-if="draft.mode === 'daily'">
          <label>{{ t('snapshots.daily_snapshot_time') }} <input v-model="draft.dailyTime" type="time" /></label>
          <p class="snapshot-muted snapshot-full">{{ t('snapshots.missed_snapshots_are_not_collected_retroactively_whe') }}</p>
        </template>
        <template v-else>
          <label>{{ t('snapshots.number_of_snapshots') }}
            <select v-model.number="draft.seriesCount">
              <option v-for="count in [2,3,4,5,6,7,8,9,10]" :key="count" :value="count">{{ count }}</option>
            </select>
          </label>
          <label>{{ t('snapshots.over_how_many_minutes') }} <input v-model.number="draft.seriesSpanMinutes" type="number" min="15" max="1440" step="15" /></label>
          <p class="snapshot-muted snapshot-full">{{ t('snapshots.seriesExample', { minutes: String(MIN_INTERVAL_BY_KIND[draft.kind]) }) }}</p>
        </template>
        <fieldset v-if="draft.mode !== 'series'" class="snapshot-weekdays snapshot-full">
          <legend>{{ t('snapshots.allowed_weekdays') }}</legend>
          <label v-for="day in weekDays" :key="day.value" class="snapshot-weekday">
            <input v-model="draft.daysOfWeek" type="checkbox" :value="day.value" />{{ day.label }}
          </label>
        </fieldset>
        <div v-if="draft.mode !== 'series'" class="snapshot-preview snapshot-full">
          <strong>{{ t('snapshots.next_scheduled_slots_local_time') }}</strong>
          <span v-if="preview.length">{{ preview.map(date).join(' · ') }}</span>
          <span v-else class="snapshot-error">{{ t('snapshots.check_the_window_first_slot_and_selected_weekdays') }}</span>
        </div>
      </div>
      <div class="snapshot-actions">
        <button type="button" :disabled="saving" @click="save">{{ draft.mode === 'series' ? t('snapshots.start_series') : editingKey ? t('snapshots.save_changes') : t('snapshots.save_schedule') }}</button>
        <button v-if="editingKey" type="button" class="secondary" @click="editingKey = null">{{ t('snapshots.cancel_editing') }}</button>
        <small class="snapshot-muted">{{ t('snapshots.you_can_create_multiple_schedules_per_connection_and') }}</small>
      </div>
    </section>

    <section class="snapshot-panel" aria-labelledby="snapshot-list-title">
      <div class="snapshot-heading"><h3 id="snapshot-list-title">{{ t('snapshots.configured_schedules') }}</h3><span>{{ t('snapshots.shownPlans', { count: String(state.rules.length), shown: String(rules.length) }) }}</span></div>
      <div v-if="state.rules.length" class="snapshot-list-controls">
        <div class="snapshot-list-filter">
          <span>{{ t('snapshots.connections') }}</span>
          <details class="snapshot-connection-select">
            <summary :title="connectionSummary">{{ connectionSummary }}</summary>
            <div class="snapshot-connection-options">
              <button type="button" class="secondary" @click="selectedConnections = null">{{ t('snapshots.show_all') }}</button>
              <div v-for="connection in connectionOptions" :key="connection.id" class="snapshot-connection-option">
                <label>
                  <input type="checkbox" :checked="isConnectionSelected(connection.id)" @change="toggleConnection(connection.id)" />
                  <span>{{ connection.label }}</span>
                </label>
                <button type="button" class="secondary" @click="selectedConnections = [connection.id]">{{ t('snapshots.only_this') }}</button>
              </div>
            </div>
          </details>
        </div>
        <label class="snapshot-list-filter">{{ t('snapshots.sort_by') }}
          <select v-model="sortBy">
            <option value="nextAt">{{ t('snapshots.next_attempt') }}</option>
            <option value="mode">{{ t('snapshots.mode_2') }}</option>
            <option value="connection">{{ t('snapshots.connection_2') }}</option>
          </select>
        </label>
        <label class="snapshot-list-filter">{{ t('snapshots.order') }}
          <select v-model="sortDirection">
            <option value="asc">{{ t('snapshots.ascending') }}</option>
            <option value="desc">{{ t('snapshots.descending') }}</option>
          </select>
        </label>
        <label class="snapshot-list-filter">{{ t('snapshots.grouping') }}
          <select v-model="groupByMode">
            <option :value="false">{{ t('snapshots.no_grouping') }}</option>
            <option :value="true">{{ t('snapshots.by_mode') }}</option>
          </select>
        </label>
      </div>
      <p v-if="!state.rules.length" class="snapshot-muted">{{ t('snapshots.automatic_collection_is_off_add_a_schedule_above_pgx') }}</p>
      <p v-else-if="!rules.length" class="snapshot-muted">{{ t('snapshots.no_schedules_for_selected_connections_select_other_c') }}</p>
      <div v-else class="snapshot-table-scroll">
        <table class="snapshot-table">
          <thead><tr><th>{{ t('snapshots.connection_module') }}</th><th>{{ t('snapshots.mode') }}</th><th>{{ t('snapshots.status') }}</th><th>{{ t('snapshots.next_attempt_2') }}</th><th>{{ t('snapshots.last_result') }}</th><th>{{ t('snapshots.controls') }}</th></tr></thead>
          <tbody v-for="group in groups" :key="group.mode ?? 'all'">
            <tr v-if="group.mode !== null" class="snapshot-group-row"><th colspan="6" scope="rowgroup">{{ group.mode === null ? group.label : localizedMode(group.mode) }} · {{ group.rules.length }}</th></tr>
            <tr v-for="rule in group.rules" :key="rule.key">
              <td><strong>{{ rule.targetName }}</strong><small>{{ KIND_LABELS[rule.kind] }} · ID {{ rule.connectionId }}</small></td>
              <td><strong>{{ localizedMode(rule.mode) }}</strong><small>{{ description(rule) }}</small></td>
              <td>{{ status(rule) }}</td>
              <td>{{ date(upcomingAttempt(rule)) }}</td>
              <td>
                <span v-if="rule.lastSuccessAt">{{ t('snapshots.success') }} {{ date(rule.lastSuccessAt) }}<small v-if="rule.lastDurationMs !== null">{{ t('snapshots.ms', { count: String(rule.lastDurationMs) }) }}</small></span>
                <span v-else>{{ t('snapshots.no_successful_runs_yet') }}</span>
                <small v-if="rule.lastAttemptAt">{{ t('snapshots.attempt') }} {{ date(rule.lastAttemptAt) }}</small>
                <small v-if="rule.lastError" class="snapshot-error">{{ displayScheduleError(rule.lastError ?? "") }}</small>
              </td>
              <td class="snapshot-row-actions">
                <button type="button" class="secondary" :disabled="state.collectingKey === rule.key" @click="edit(rule)">{{ t('snapshots.edit') }}</button>
                <button v-if="rule.status === 'active' || rule.status === 'paused'" type="button" class="secondary" :disabled="state.collectingKey === rule.key" @click="actions.setEnabled(rule.key, !rule.enabled)">{{ rule.enabled ? t('snapshots.pause') : t('snapshots.resume') }}</button>
                <button type="button" class="secondary" :disabled="state.collectingKey === rule.key" @click="remove(rule)">{{ t('snapshots.delete') }}</button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <p class="snapshot-muted">{{ t('snapshots.filters_sorting_and_grouping_affect_only_this_table_') }}</p>
    </section>
  </section>
</template>

<style scoped>
.snapshot-settings{display:grid;gap:1rem;min-width:0}
.snapshot-header,.snapshot-heading{display:flex;justify-content:space-between;align-items:start;gap:1rem;flex-wrap:wrap}
.snapshot-header h2,.snapshot-panel h3,.snapshot-heading h3{margin:0}
.snapshot-muted{color:var(--text-secondary);font-size:.88rem;line-height:1.5}
.snapshot-note{padding:.85rem 1rem;border:1px solid var(--border);border-radius:.6rem;background:var(--surface-2);font-size:.9rem;line-height:1.5}
.snapshot-error{color:var(--error);font-size:.86rem;overflow-wrap:anywhere}
.snapshot-notice{color:var(--accent);font-size:.88rem}
.snapshot-panel{border:1px solid var(--border);border-radius:.7rem;padding:1rem;display:grid;gap:1rem;min-width:0}
.snapshot-form-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(205px,1fr));gap:.8rem}
.snapshot-form-grid label{display:grid;gap:.3rem;font-size:.9rem}
.snapshot-form-grid select,.snapshot-form-grid input{width:100%;min-width:0;box-sizing:border-box;padding:.6rem;border-radius:.4rem;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong)}
.snapshot-full{grid-column:1/-1;margin:0}
.snapshot-actions{display:flex;align-items:center;gap:1rem;flex-wrap:wrap}
.snapshot-list-controls{display:grid;grid-template-columns:minmax(230px,2fr) repeat(3,minmax(150px,1fr));gap:.75rem;align-items:end}
.snapshot-list-filter{display:grid;gap:.35rem;font-size:.88rem;min-width:0}
.snapshot-list-filter select,.snapshot-connection-select summary{width:100%;min-width:0;box-sizing:border-box;padding:.6rem;border-radius:.4rem;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);font:inherit}
.snapshot-connection-select{position:relative;min-width:0}
.snapshot-connection-select summary{cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;list-style-position:inside}
.snapshot-connection-options{position:absolute;top:calc(100% + .25rem);left:0;z-index:10;width:max(100%,310px);max-width:min(520px,calc(100vw - 4rem));max-height:320px;overflow:auto;box-sizing:border-box;padding:.6rem;border:1px solid var(--border-strong);border-radius:.5rem;background:var(--surface-2);box-shadow:0 8px 24px #0005;display:grid;gap:.5rem}
.snapshot-connection-options > button{justify-self:start}
.snapshot-connection-option{display:flex;align-items:center;justify-content:space-between;gap:.6rem}
.snapshot-connection-option label{display:flex;align-items:center;gap:.5rem;min-width:0;overflow-wrap:anywhere;cursor:pointer}
.snapshot-connection-option label input{flex:0 0 auto}
.snapshot-connection-option button{white-space:nowrap;font-size:.77rem;padding:.3rem .45rem}
.snapshot-group-row th{background:var(--surface-2);font-weight:700}
@media(max-width:920px){.snapshot-list-controls{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:520px){.snapshot-list-controls{grid-template-columns:minmax(0,1fr)}}
.snapshot-table-scroll{overflow-x:auto}
.snapshot-table{width:100%;border-collapse:collapse;text-align:left;font-size:.88rem;min-width:850px}
.snapshot-table th,.snapshot-table td{padding:.65rem .55rem;border-bottom:1px solid var(--border);vertical-align:top}
.snapshot-table td small{display:block;color:var(--text-secondary);font-size:.77rem;margin-top:.3rem}
.snapshot-table td small.snapshot-error{color:var(--error)}
.snapshot-row-actions{white-space:nowrap}
.snapshot-row-actions button{font-size:.78rem;padding:.4rem .55rem;margin:.1rem}
.snapshot-weekdays{border:1px solid var(--border);border-radius:.45rem;display:flex;align-items:center;gap:.8rem;flex-wrap:wrap;padding:.6rem .8rem}
.snapshot-weekdays legend{padding:0 .3rem;font-size:.9rem}
.snapshot-weekday{display:inline-flex!important;align-items:center;gap:.3rem!important;white-space:nowrap}
.snapshot-weekday input{width:auto!important;min-width:auto!important}
.snapshot-preview{display:grid;gap:.3rem;padding:.6rem .75rem;border:1px solid var(--border);border-radius:.4rem;font-size:.87rem}
.snapshot-preview span{color:var(--text-secondary);line-height:1.5}
</style>
