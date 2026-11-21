<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { formatBytes, formatInteger, formatSnapshotTime } from "../../core/formatters.js";
import { createJournalExplorerController, createJournalExplorerState } from "./log-explorer-controller.js";
import { journalConnectionId, journalConnectionLabel, journalContext, journalContextValue, journalDateBounds, journalDescription, journalModule, journalStatus, type JournalQuery } from "./log-explorer-model.js";
import { logDisplayRows, logOperationLabel, logOperationLabels, type LogDisplayMode, type LogDisplayRow } from "./log-view-model.js";
import UiIcon from "../../components/UiIcon.vue";
import { readinessConnections } from "./settings-view-model.js";

const props = defineProps<{
  invoke: PgXrayInvoke;
  shell: ShellState;
  disabled: boolean;
  reloadKey: number;
  directory: string | null;
}>();
const connectionNames = ref<ReadonlyMap<number, string>>(new Map());
let connectionNamesRevision = 0;
/** Refresh only saved public connection metadata; never keep credentials. */
async function loadConnectionNames(): Promise<void> {
  const revision = ++connectionNamesRevision;
  const generation = props.shell.authGeneration;
  if (props.disabled || props.shell.currentUsername === null) return;
  try {
    const rows = readinessConnections(await props.invoke("list_connections"));
    if (revision !== connectionNamesRevision || generation !== props.shell.authGeneration ||
        props.shell.currentUsername === null || props.disabled || props.shell.screen !== "workspace") return;
    connectionNames.value = new Map(rows.map(({ id, name }) => [id, name]));
  } catch {
    // Journal remains usable even when saved connections cannot be listed.
    if (revision === connectionNamesRevision && generation === props.shell.authGeneration)
      connectionNames.value = new Map();
  }
}
function formatConnection(id: number): string {
  return journalConnectionLabel(id, connectionNames.value);
}
function describe(row: LogDisplayRow): string {
  return journalDescription(row, formatConnection);
}
function displayContextValue(field: { key: string; value: string }): string {
  if ((field.key === "connection_id" || field.key === "connectionId") && /^\d+$/.test(field.value)) {
    const id = Number(field.value);
    if (Number.isSafeInteger(id) && id > 0) return formatConnection(id);
  }
  return field.value;
}
const state = reactive(createJournalExplorerState());
const explorer = createJournalExplorerController(props.shell, state, props.invoke);
const mode = ref<LogDisplayMode>("operations");
const limit = ref(50);
const from = ref("");
const to = ref("");
const search = ref("");
const component = ref("");
const section = ref("");
const actor = ref("");
const operation = ref("");
const searchInput = ref<HTMLInputElement | null>(null);
const investigationDialog = ref<HTMLDialogElement | null>(null);
const knownOperations = Object.keys(logOperationLabels).sort();
const searchHints = computed(() => [
  { title: t('journal.sql_error'), value: "query_failed" },
  { title: t('journal.timeout'), value: "query_timeout" },
  { title: t('journal.connection_id'), value: "connection_id=" },
  { title: "Query ID", value: "queryid=" },
  { title: t('journal.sql_fingerprint'), value: "sql_fingerprint=" },
  { title: t('journal.connection_error'), value: "client_connect_failed" },
]);
function useSearchHint(value: string): void {
  search.value = value;
  searchInput.value?.focus();
}
watch(() => state.selected, selected => {
  const dialog = investigationDialog.value;
  if (!dialog) return;
  if (selected && !dialog.open) dialog.showModal();
  else if (!selected && dialog.open) dialog.close();
}, { flush: "post" });
function dismissInvestigationBackdrop(event: MouseEvent): void {
  if (event.target === investigationDialog.value) explorer.close();
}
const problems = ref(false);
const minDuration = ref(0);
let lastQuery: JournalQuery | null = null;
const rows = computed(() => logDisplayRows(state.page?.entries ?? [], mode.value));
const errors = computed(() => rows.value.filter(r => r.result === "failed" || r.level === "error").length);
const warnings = computed(() => rows.value.filter(r => r.level === "warning" || r.level === "warn").length);
const completed = computed(() => rows.value.filter(r => r.result === "success").length);
const selectedOperationId = computed(() => state.requestEvents.find(e => e.operation_id)?.operation_id
  ?? state.selected?.entries.find(e => e.operation_id)?.operation_id ?? null);
const rustTerminal = computed(() => [...state.requestEvents].reverse().find(e =>
  (e.component === "diagnostics" || !!e.operation_result) && ["succeeded", "failed"].includes(e.phase)) ?? null);
const typedOutcome = computed(() => rustTerminal.value?.operation_result ?? null);
const operationError = computed(() => rustTerminal.value?.error
  ?? state.requestEvents.find(e => e.error)?.error ?? state.selected?.error ?? null);
const correlatedUi = computed(() => state.requestEvents.some(e => e.component === "ui_reported")
  && state.requestEvents.some(e => e.component === "diagnostics" || !!e.operation_result));
const confirmedSnapshot = computed(() => [...state.requestEvents].reverse()
  .find(e => e.component === "diagnostics" && e.phase === "succeeded" && e.diagnostic?.snapshot_id != null)?.diagnostic ?? null);
const relatedSql = computed(() => state.requestEvents.filter(e =>
  e.component === "postgres" && ["query_finished", "query_failed", "query_timeout"].includes(e.phase)));
const partialOperationHistory = computed(() => !!state.requestCursor || state.requestScanLimitReached);

const filteredActive = computed(() => [from.value, to.value, search.value, component.value, actor.value, operation.value, section.value].some(Boolean) || problems.value || minDuration.value > 0);

function dateString(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
function recent(days: number): void {
  const day = new Date();
  day.setDate(day.getDate() - days + 1);
  from.value = dateString(day);
  to.value = dateString(new Date());
  void startSearch();
}
async function startSearch(): Promise<void> {
  if (props.disabled) return;
  void loadConnectionNames();
  let bounds: { from_ms: number | null; to_ms: number | null };
  try { bounds = journalDateBounds(from.value, to.value); }
  catch (error) { state.error = String(error); return; }
  const query: JournalQuery = {
    limit: limit.value, cursor: null, ...bounds,
    component: component.value.trim() || null,
    section: section.value || null,
    level: null,
    operation: operation.value.trim() || null,
    actor: actor.value.trim() || null,
    request_id: null,
    operation_id: null,
    problems_only: problems.value,
    min_duration_ms: minDuration.value || null,
    search: search.value.trim() || null,
  };
  if (await explorer.load(query, "reset")) lastQuery = query;
}
async function go(direction: "next" | "previous"): Promise<void> {
  if (lastQuery && !props.disabled) await explorer.load(lastQuery, direction);
}
function clearFilters(): void {
  from.value = "";
  to.value = "";
  search.value = "";
  component.value = "";
  section.value = "";
  actor.value = "";
  operation.value = "";
  problems.value = false;
  minDuration.value = 0;
  void startSearch();
}
function showProblems(): void {
  // The shortcut investigates all retained errors, not just the previous
  // operation/actor filter accidentally left on the screen.
  from.value = "";
  to.value = "";
  search.value = "";
  component.value = "";
  section.value = "";
  actor.value = "";
  operation.value = "";
  minDuration.value = 0;
  problems.value = true;
  void startSearch();
}
const selectedFields = computed(() => journalContext(state.selected?.context ?? null));
const selectedConnectionId = computed(() => journalConnectionId([
  ...(state.selected?.entries ?? []), ...state.requestEvents,
]));
function rowModule(row: LogDisplayRow): string {
  const kind = row.entries.find(e => e.diagnostic)?.diagnostic?.kind;
  if (kind) return ({ xid: "XID / Freeze", pgss: "PGSS / SQL", autovacuum: "Autovacuum",
    io_wal: "I/O & WAL", index_health: "Index Health", pg_settings: "PG Settings" } as Record<string, string>)[kind] ?? kind;
  return journalModule(row.operation, row.component);
}
function responseSize(row: LogDisplayRow): string {
  const value = journalContextValue(row.context, "response_bytes");
  return value !== null && /^\d+$/.test(value) ? formatBytes(Number(value)) : "—";
}
function sqlBytes(row: LogDisplayRow): string {
  const value = journalContextValue(row.context, "sql_bytes");
  return value !== null && /^\d+$/.test(value) ? formatBytes(Number(value)) : "—";
}
function requestTimeline() {
  return state.requestEvents;
}
function eventStage(event: { component: string; phase: string }): string {
  const stages: Record<string, string> = {
    started: t('journal.started'), succeeded: t('journal.completed'), failed: t('journal.error'),
    query_started: t('journal.sql_started'), query_finished: t('journal.sql_completed'),
    query_failed: t('journal.sql_error_2'), query_timeout: t('journal.sql_timeout'),
  };
  const source = event.component === "ui_reported" ? "UI" :
    event.component === "diagnostics" ? "Rust" :
    event.component === "postgres" ? "PostgreSQL" : "Rust";
  return `${source} · ${stages[event.phase] ?? event.phase}`;
}

watch(() => [props.shell.authGeneration, props.shell.currentUsername, props.shell.screen], () => {
  ++connectionNamesRevision;
  connectionNames.value = new Map();
  explorer.reset();
  lastQuery = null;
  if (props.shell.screen === "workspace" && props.shell.currentUsername !== null && !props.disabled)
    void startSearch();
});
onMounted(() => { if (!props.disabled) void startSearch(); });
watch(() => props.disabled, (disabled, prev) => {
  if (!disabled && prev && !state.page && !state.loading) void startSearch();
});
watch(() => props.reloadKey, () => {
  explorer.reset();
  if (!props.disabled) void startSearch();
});
watch(() => props.directory, (next, old) => {
  if (next !== old && old !== null) {
    explorer.reset();
    if (!props.disabled) void startSearch();
  }
});
onUnmounted(() => { ++connectionNamesRevision; explorer.reset(); });
</script>

<template>
  <section class="log-explorer" :aria-label="t('journal.pgxray_diagnostic_journal')">
    <header class="journal-top">
      <div>
        <h4>{{ t('journal.operations_and_diagnostics_journal') }}</h4>
        <p class="journal-muted">{{ t('journal.local_jsonl_files_search_across_the_retained_period_') }}</p>
      </div>
      <div class="journal-actions">
        <button type="button" class="secondary" :aria-label="t('journal.refresh_journal_events')" :aria-busy="state.loading" :disabled="disabled || state.loading" @click="startSearch"><UiIcon name="refresh" :size="16" :class="{ 'journal-spin': state.loading }" /> {{ t('journal.refresh') }}</button>
        <button type="button" class="secondary" :disabled="disabled || state.loading" @click="showProblems">{{ t('journal.application_problems') }}</button>
      </div>
    </header>

    <form class="journal-query" @submit.prevent="startSearch">
      <label class="wide">{{ t('journal.search_entire_journal') }}
        <input ref="searchInput" v-model="search" type="search" list="journal-search-hints" :placeholder="t('journal.for_example_timeout_connection_id_7_queryid')" autocomplete="off" />
        <datalist id="journal-search-hints"><option v-for="hint in searchHints" :key="hint.value" :value="hint.value">{{ hint.title }}</option></datalist>
        <span class="journal-hint">{{ t('journal.searches_recorded_event_fields_operation_phase_reque') }}</span>
      </label>
      <label class="journal-date">{{ t('journal.from_date') }}
        <input v-model="from" type="date" />
      </label>
      <label class="journal-date">{{ t('journal.to_date') }}
        <input v-model="to" type="date" />
      </label>
      <label>{{ t('journal.pgxray_module') }}
        <select v-model="section">
          <option value="">{{ t('journal.all_modules') }}</option>
          <optgroup :label="t('journal.working_with_postgresql')">
            <option value="connections">{{ t('journal.connections') }}</option>
            <option value="diagnostics">{{ t('journal.diagnostics_all_modules') }}</option>
            <option value="pgss">↳ PGSS / SQL</option>
            <option value="xid">↳ XID / Freeze</option>
            <option value="autovacuum">↳ Autovacuum</option>
            <option value="io_wal">↳ I/O & WAL</option>
            <option value="index_health">↳ Index Health</option>
            <option value="pg_settings">↳ PG Settings</option>
            <option value="activity">↳ Activity / backend signals</option>
            <option value="locks">↳ Locks</option>
            <option value="postgres">{{ t('journal.general_backend_sql_events') }}</option>
          </optgroup>
          <optgroup :label="t('journal.other_modules')">
            <option value="sessions">psql Sessions</option>
            <option value="plans">{{ t('journal.plans_explain') }}</option>
            <option value="investigations">{{ t('journal.investigations_files') }}</option>
            <option value="auth">{{ t('journal.users_and_sign_in') }}</option>
            <option value="settings">{{ t('journal.settings') }}</option>
            <option value="application">{{ t('journal.application_journal') }}</option>
            <option value="other">{{ t('journal.other_unclassified') }}</option>
          </optgroup>
        </select>
      </label>
      <label>{{ t('journal.technical_source_optional') }}
        <select v-model="component">
          <option value="">{{ t('journal.any_ui_or_rust') }}</option>
          <option value="ui_reported">{{ t('journal.ui_interface_message') }}</option>
          <option value="postgres">Rust — PostgreSQL</option>
          <option value="app_log">{{ t('journal.rust_journal') }}</option>
          <option value="application">{{ t('journal.rust_application') }}</option>
          <option value="locks">Rust — locks</option>
          <option value="what_changed">Rust — what_changed</option>
        </select>
      </label>
      <label>{{ t('journal.operation_exact_name') }}
        <input v-model="operation" list="journal-known-operations" :placeholder="t('journal.select_or_enter_a_command_name')" autocomplete="off" />
        <datalist id="journal-known-operations"><option v-for="op in knownOperations" :key="op" :value="op">{{ logOperationLabel(op) }}</option></datalist>
      </label>
      <label>{{ t('journal.user') }}
        <input v-model="actor" :placeholder="t('journal.username')" />
      </label>
      <label class="journal-small">{{ t('journal.minimum_duration') }}
        <select v-model.number="minDuration"><option :value="0">{{ t('journal.all') }}</option><option :value="500">{{ t('journal.duration500') }}</option><option :value="1000">{{ t('journal.1_sec') }}</option><option :value="5000">{{ t('journal.5_sec') }}</option><option :value="30000">{{ t('journal.30_sec') }}</option></select>
      </label>
      <label class="journal-small">{{ t('journal.events_per_query') }}
        <select v-model.number="limit"><option :value="25">25</option><option :value="50">50</option><option :value="100">100</option><option :value="200">200</option></select>
      </label>
      <div class="journal-filters-actions">
        <label class="inline"><input v-model="problems" type="checkbox" /> {{ t('journal.errors_and_warnings_only') }}</label>
        <button type="submit" :disabled="disabled || state.loading">{{ t('journal.search') }}</button>
        <button type="button" class="secondary" :disabled="disabled || state.loading || !filteredActive" @click="clearFilters">{{ t('journal.reset') }}</button>
      </div>
    </form>
    <div class="journal-presets journal-search-examples" :aria-label="t('journal.search_examples')">
      <span class="journal-muted">{{ t('journal.quick_templates_you_can_append_a_value') }}</span>
      <button v-for="hint in searchHints" :key="hint.value" type="button" class="secondary small"
        :title="hint.value" @click="useSearchHint(hint.value)">{{ hint.title }}</button>
    </div>
    <p class="journal-muted journal-scope-note">{{ t('journal.modules_are_inferred_from_saved_operation_and_compon') }}</p>
    <div class="journal-presets">
      <span class="journal-muted">{{ t('journal.period') }}</span>
      <button type="button" class="secondary small" :disabled="disabled || state.loading" @click="recent(1)">{{ t('journal.today') }}</button>
      <button type="button" class="secondary small" :disabled="disabled || state.loading" @click="recent(7)">{{ t('journal.7_days') }}</button>
      <button type="button" class="secondary small" :disabled="disabled || state.loading" @click="recent(30)">{{ t('journal.30_days') }}</button>
      <button type="button" class="secondary small" :disabled="disabled || state.loading" @click="recent(90)">{{ t('journal.90_days') }}</button>
      <label class="journal-inline-toggle"><input v-model="mode" type="radio" value="operations" /> {{ t('journal.operations') }}</label>
      <label class="journal-inline-toggle"><input v-model="mode" type="radio" value="events" /> {{ t('journal.events') }}</label>
    </div>

    <p v-if="state.error" role="alert" class="journal-error">{{ state.error }}</p>
    <p v-if="state.loading" role="status" class="journal-muted">{{ t('journal.searching_journal_files') }}</p>

    <div class="journal-stats">
      <div><strong>{{ formatInteger(rows.length) }}</strong><span>{{ t('journal.records_groups_per_page') }}</span></div>
      <div><strong>{{ errors }}</strong><span>{{ t('journal.errors_on_page') }}</span></div>
      <div><strong>{{ warnings }}</strong><span>{{ t('journal.warnings') }}</span></div>
      <div><strong>{{ completed }}</strong><span>{{ t('journal.completed_operations') }}</span></div>
    </div>
    <div class="journal-meta" v-if="state.page">
      <span>{{ t('journal.pageSummary', { page: String(state.pageIndex + 1), events: String(state.page.entries.length), bytes: formatBytes(state.page.scanned_bytes) }) }}</span>
      <span v-if="state.page.scan_limit_reached">{{ t('journal.scan_limit_reached_for_this_query_you_can_continue_s') }}</span>
      <span v-if="state.page.malformed_lines" class="journal-error">{{ t('journal.malformed', { count: String(state.page.malformed_lines) }) }}</span>
      <span v-if="state.page.next_cursor">{{ t('journal.older_archive_entries_available') }}</span>
      <span v-else>{{ t('journal.end_of_retained_archive_matching_filters') }}</span>
    </div>

    <div class="journal-table-wrap">
      <table class="journal-table">
        <thead><tr><th>{{ t('journal.date_and_time') }}</th><th>{{ t('journal.module_action') }}</th><th>{{ t('journal.result_meaning') }}</th><th>{{ t('journal.user_source') }}</th><th>{{ t('journal.duration') }}</th><th>{{ t('journal.response') }}</th><th>{{ t('journal.inspect') }}</th></tr></thead>
        <tbody>
          <tr v-for="row in rows" :key="row.key" :class="{ 'journal-problem': row.result === 'failed' || row.level === 'error' }">
            <td class="journal-time">{{ formatSnapshotTime(row.timestamp_ms) }}</td>
            <td><strong>{{ rowModule(row) }}</strong><small>{{ logOperationLabel(row.operation) }}</small></td>
            <td><span class="journal-status" :data-result="row.result">{{ journalStatus(row) }}</span><div class="journal-story">{{ describe(row) }}</div></td>
            <td>{{ row.actor ?? '—' }}<small>{{ row.source === 'ui' ? t('journal.ui_not_independently_verified') : t('journal.rust_backend') }}</small></td>
            <td class="journal-num">{{ row.duration_ms === null ? '—' : t('journal.rowDuration', { count: formatInteger(row.duration_ms) }) }}</td>
            <td class="journal-num">{{ responseSize(row) }}</td>
            <td><button type="button" class="secondary small" @click="explorer.open(row)">{{ t('journal.inspect_2') }}</button></td>
          </tr>
          <tr v-if="!rows.length"><td colspan="7">{{ state.loading ? t('journal.searching') : state.page ? t('journal.no_matches_in_this_part_of_the_archive_use_the_curso') : t('journal.start_a_search') }}</td></tr>
        </tbody>
      </table>
    </div>
    <nav class="journal-pager" :aria-label="t('journal.journal_navigation')">
      <span>{{ t('journal.search_runs_from_newest_to_oldest_filters_are_applie') }}</span>
      <button type="button" class="secondary journal-nav-icon" :aria-label="t('journal.newer_events')"
        :title="t('journal.newer_events')" :disabled="disabled || state.loading || state.pageIndex === 0" @click="go('previous')"><UiIcon name="chevron-left" :size="20" /></button>
      <span class="journal-page-number">{{ state.pageIndex + 1 }}</span>
      <button type="button" class="secondary journal-nav-icon" :aria-label="t('journal.older_events')"
        :title="state.page?.scan_limit_reached ? t('journal.continue_searching_older_files') : t('journal.older_events')"
        :disabled="disabled || state.loading || !state.page?.next_cursor" @click="go('next')"><UiIcon name="chevron-right" :size="20" /></button>
    </nav>

    <dialog ref="investigationDialog" class="journal-investigation" aria-labelledby="journal-investigation-title"
      @click="dismissInvestigationBackdrop" @close="explorer.close()">
      <div v-if="state.selected" class="journal-investigation-body">
      <header class="journal-investigation-heading">
        <div><h5 id="journal-investigation-title">{{ t('journal.operation_details') }} {{ logOperationLabel(state.selected.operation) }}</h5><p class="journal-muted">{{ journalModule(state.selected.operation, state.selected.component) }} · {{ formatSnapshotTime(state.selected.timestamp_ms) }}</p></div>
        <button type="button" class="secondary journal-close" :aria-label="t('journal.close_details')" :title="t('journal.close_esc')" @click="explorer.close()"><UiIcon name="x" :size="19" /></button>
      </header>
      <p class="journal-explanation">{{ describe(state.selected) }}</p>
      <p v-if="selectedConnectionId !== null && !confirmedSnapshot" class="journal-muted">
        {{ t('journal.connection_3') }} <strong>{{ formatConnection(selectedConnectionId) }}</strong>
      </p>
      <div v-if="confirmedSnapshot || typedOutcome || selectedOperationId" class="journal-operation-result">
        <h6>{{ rustTerminal ? t('journal.result_confirmed_by_rust') : t('journal.rust_trace') }}</h6>
        <template v-if="confirmedSnapshot">
          <div class="journal-outcome-grid">
            <div><span>{{ t('journal.snapshot') }}</span><strong>{{ confirmedSnapshot.kind }} #{{ confirmedSnapshot.snapshot_id }}</strong></div>
            <div><span>{{ t('journal.connection') }}</span><strong>{{ formatConnection(confirmedSnapshot.connection_id) }}</strong></div>
            <div v-if="confirmedSnapshot.total_sections !== null && confirmedSnapshot.unavailable_sections !== null">
              <span>{{ t('journal.sections_retrieved') }}</span><strong>{{ Math.max(0, confirmedSnapshot.total_sections - confirmedSnapshot.unavailable_sections) }} / {{ confirmedSnapshot.total_sections }}</strong>
            </div>
            <div><span>{{ t('journal.completed_sql_calls') }}</span><strong>{{ relatedSql.length }}{{ partialOperationHistory ? t('journal.partial_sample') : '' }}</strong></div>
          </div>
          <p class="journal-muted">{{ t('journal.rust_confirmed_the_snapshot_was_saved_in_sqlite_sql_') }}</p>
        </template>
        <p v-else-if="typedOutcome?.kind === 'authentication'">
          {{ typedOutcome.authenticated ? t('journal.rust_confirmed_successful_user_authentication') : t('journal.rust_rejected_the_sign_in_attempt_credentials_and_er') }}
        </p>
        <p v-else-if="typedOutcome?.kind === 'connection_projects'">
          <template v-if="typedOutcome.succeeded">{{ t('journal.rust_successfully_loaded_connection_projects') }} <strong>{{ typedOutcome.count }}</strong>.</template>
          <template v-else>{{ t('journal.rust_could_not_load_connection_projects') }}</template>
        </p>
        <p v-else-if="!rustTerminal" class="journal-muted">{{ t('journal.no_rust_outcome_was_found_in_the_loaded_timeline_thi') }}</p>
        <p v-if="selectedOperationId" class="journal-muted">Rust Operation ID: <code>{{ selectedOperationId }}</code></p>
        <p v-if="correlatedUi" class="journal-muted">{{ t('journal.the_ui_message_is_linked_to_a_rust_confirmed_operati') }}</p>
        <div v-if="relatedSql.length" class="journal-sql-list">
          <div v-for="query in relatedSql" :key="query.request_id ?? query.timestamp_ms">
            <code>{{ query.request_id ?? 'SQL' }}</code> · {{ eventStage(query) }} · {{ t('journal.queryDuration', { count: String(query.duration_ms ?? '—') }) }}
            <span v-if="journalContextValue(query.context, 'response_bytes')"> · {{ t('journal.responseBytes', { size: formatBytes(Number(journalContextValue(query.context, 'response_bytes'))) }) }}</span>
          </div>
        </div>
      </div>
      <div class="journal-facts">
        <div><span>{{ t('journal.outcome') }}</span><strong>{{ rustTerminal ? (rustTerminal.phase === 'succeeded' ? t('journal.rustSuccess') : t('journal.rustError')) : journalStatus(state.selected) }}</strong></div>
        <div><span>{{ t('journal.source') }}</span><strong>{{ correlatedUi ? 'UI + Rust' : state.selected.source === 'ui' ? t('journal.uiOnly') : t('journal.rust_backend') }}</strong></div>
        <div v-if="state.selected.actor"><span>{{ t('journal.user') }}</span><strong>{{ state.selected.actor }}</strong></div>
        <div v-if="rustTerminal?.duration_ms != null || state.selected.duration_ms != null"><span>{{ t('journal.duration') }}</span><strong>{{ t('journal.durationValue', { count: formatInteger(rustTerminal?.duration_ms ?? state.selected.duration_ms ?? 0) }) }}</strong></div>
        <div v-if="sqlBytes(state.selected) !== '—'"><span>{{ t('journal.sqlSize') }}</span><strong>{{ sqlBytes(state.selected) }}</strong></div>
        <div v-if="responseSize(state.selected) !== '—'"><span>{{ t('journal.responseSize') }}</span><strong>{{ responseSize(state.selected) }}</strong></div>
      </div>
      <div v-if="selectedFields.length || operationError" class="journal-detail-columns">
        <div v-if="selectedFields.length"><h6>{{ t('journal.context') }}</h6>
          <dl class="journal-context">
            <template v-for="field in selectedFields" :key="field.key"><dt>{{ field.key }}</dt><dd>{{ displayContextValue(field) }}</dd></template>
          </dl>
          <details v-if="state.selected.context"><summary class="journal-muted">{{ t('journal.raw_context') }}</summary><pre class="journal-raw">{{ state.selected.context }}</pre></details>
        </div>
        <div v-if="operationError"><h6>{{ t('journal.error_and_diagnostics') }}</h6>
          <pre class="journal-raw journal-error">{{ operationError }}</pre>
          <p class="journal-muted">{{ t('journal.the_error_may_contain_sensitive_details_review_it_be') }}</p>
        </div>
      </div>
      <h6>{{ selectedOperationId || correlatedUi ? t('journal.operation_timeline_and_sql') : t('journal.event_timeline') }} <span class="journal-muted">{{ t('journal.eventsCount', { count: String(state.requestEvents.length) }) }}</span></h6>
      <p v-if="state.requestLoading" class="journal-muted">{{ t('journal.searching_all_phases_by_request_id_across_the_retain') }}</p>
      <p v-if="state.requestError" class="journal-error">{{ state.requestError }}</p>
      <ol class="journal-timeline">
        <li v-for="(event, index) in requestTimeline()" :key="index">
          <strong>{{ formatSnapshotTime(event.timestamp_ms) }} · {{ eventStage(event) }}</strong>
          <span>{{ event.level }}<template v-if="event.duration_ms !== null"> · {{ t('journal.eventDuration', { count: String(event.duration_ms) }) }}</template></span>
          <p v-if="event.error" class="journal-error">{{ event.error }}</p>
          <p v-if="event.context && !selectedFields.length" class="journal-muted">{{ event.context }}</p>
        </li>
      </ol>
      <button v-if="state.requestCursor" type="button" class="secondary" :disabled="state.requestLoading" @click="explorer.continueRequest()">{{ t('journal.continue_searching_phases_in_older_files') }}</button>
      <p v-else class="journal-muted">{{ selectedOperationId || correlatedUi ? t('journal.linked_event_search_completed_within_the_retained_jo') : state.selected.request_id ? t('journal.no_other_phases_found_within_the_retained_journal') : t('journal.this_entry_has_no_correlation_id_and_cannot_safely_b') }}</p>
      <p v-if="selectedConnectionId !== null" class="journal-muted">{{ t('journal.connection_names_come_from_the_currently_saved_profi') }}</p>
      <details class="journal-raw-records"><summary>{{ t('journal.raw_event_json_all_fields') }}</summary><pre class="journal-raw">{{ JSON.stringify(state.requestEvents, null, 2) }}</pre></details>
      </div>
    </dialog>
    <p class="journal-muted">{{ t('journal.the_journal_records_selected_application_events_not_') }}</p>
  </section>
</template>

<style scoped>
.log-explorer{display:grid;gap:1rem;min-width:0;margin-top:1.25rem}
.journal-top{display:flex;justify-content:space-between;gap:1rem;flex-wrap:wrap;align-items:center}
.journal-top h4{margin:.1rem 0 .4rem;font-size:1.1rem}
.journal-muted{color:var(--text-secondary);font-size:.8rem;line-height:1.5;margin:.2rem 0;overflow-wrap:anywhere}
.journal-actions,.journal-presets,.journal-filters-actions,.journal-pager{display:flex;gap:.5rem;align-items:center;flex-wrap:wrap}
.journal-actions button{display:inline-flex;gap:.4rem;align-items:center}
.journal-spin{animation:journal-refresh .8s linear infinite}
@keyframes journal-refresh{to{transform:rotate(360deg)}}
@media(prefers-reduced-motion:reduce){.journal-spin{animation:none}}
.journal-query{display:grid;box-sizing:border-box;width:100%;max-width:none;min-width:0;grid-template-columns:repeat(12,minmax(0,1fr));gap:.65rem .85rem;border:1px solid var(--border);border-radius:.6rem;padding:1rem;background:var(--surface-2)}
.journal-query>label{grid-column:span 3;display:grid;align-content:start;gap:.3rem;min-width:0;font-size:.78rem;color:var(--text-secondary)}
.journal-query>.wide{grid-column:span 6}
.journal-query>.journal-date{grid-column:span 3}
.journal-query>.journal-small{grid-column:span 2}
.journal-query input,.journal-query select{width:100%;min-width:0;box-sizing:border-box}
.journal-query .inline{display:flex;align-items:center;gap:.3rem;color:var(--text-primary);font-size:.84rem}
.journal-query .inline input{width:auto}
.journal-filters-actions{grid-column:span 8;justify-content:flex-end;align-self:end}
.journal-inline-toggle{display:flex;align-items:center;gap:.3rem;font-size:.84rem;margin-left:.55rem}
.journal-stats{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.6rem}
.journal-stats>div{display:grid;gap:.15rem;padding:.75rem;border:1px solid var(--border);border-radius:.6rem;background:var(--surface-2)}
.journal-stats strong{font-size:1.35rem;font-variant-numeric:tabular-nums}
.journal-stats span{color:var(--text-secondary);font-size:.78rem}
.journal-meta{display:flex;gap:.7rem;flex-wrap:wrap;font-size:.8rem;color:var(--text-secondary)}
.journal-table-wrap{min-width:0;overflow-x:auto}
.journal-table{width:100%;border-collapse:collapse;font-size:.86rem}
.journal-table th,.journal-table td{padding:.65rem .55rem;text-align:left;vertical-align:top;border-bottom:1px solid var(--border)}
.journal-table th{color:var(--text-secondary);font-size:.78rem;white-space:nowrap}
.journal-table td strong,.journal-table td small{display:block}
.journal-table td small{margin-top:.22rem;color:var(--text-secondary);overflow-wrap:anywhere}
.journal-table td:nth-child(3){min-width:14rem;max-width:26rem}
.journal-time,.journal-num{white-space:nowrap;font-variant-numeric:tabular-nums}
.journal-story{font-size:.8rem;color:var(--text-secondary);overflow-wrap:anywhere;line-height:1.4;margin-top:.3rem;max-height:4rem;overflow:hidden}
.journal-problem{background:color-mix(in srgb,var(--error) 6%,transparent)}
.journal-status{display:inline-block;border:1px solid var(--border);border-radius:1rem;font-size:.76rem;padding:.1rem .42rem}
.journal-status[data-result=failed],.journal-error{color:var(--error)}
.journal-status[data-result=success]{color:var(--accent)}
.journal-error{white-space:pre-wrap;overflow-wrap:anywhere}
.journal-pager{justify-content:flex-end}
.journal-pager>span:first-child{margin-right:auto;color:var(--text-secondary);font-size:.8rem}
.journal-investigation{width:min(1080px,calc(100vw - 2rem));max-width:none;max-height:min(88vh,950px);box-sizing:border-box;overflow:auto;border:1px solid var(--border-strong);border-radius:.8rem;padding:1.15rem;background:var(--surface);color:var(--text);box-shadow:0 18px 75px #0009}
.journal-investigation::backdrop{background:rgba(5,5,12,.78)}
.journal-investigation-body{display:grid;gap:1rem}
.journal-investigation-heading{position:sticky;top:-1.15rem;z-index:1;background:var(--surface);padding:.5rem 0 .75rem;border-bottom:1px solid var(--border)}
.journal-close,.journal-nav-icon{display:inline-flex;align-items:center;justify-content:center}
.journal-close{min-width:2.5rem;min-height:2.5rem;flex:none}
.journal-nav-icon{width:2.6rem;height:2.6rem;padding:.4rem}
.journal-page-number{min-width:2ch;text-align:center;font-variant-numeric:tabular-nums}
.journal-hint{font-size:.72rem;line-height:1.35;color:var(--text-secondary)}
.journal-outcome-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.65rem}
.journal-outcome-grid>div{display:grid;gap:.2rem;min-width:0}
.journal-outcome-grid span{font-size:.76rem;color:var(--text-secondary)}
.journal-outcome-grid strong{overflow-wrap:anywhere}
.journal-search-examples{margin-top:-.3rem}
.journal-scope-note{font-size:.73rem}
.journal-investigation-heading{display:flex;justify-content:space-between;gap:1rem;align-items:start}
.journal-investigation h5{font-size:1.12rem;margin:0 0 .3rem}
.journal-investigation h6{font-size:.98rem;margin:.3rem 0}
.journal-explanation{font-weight:600;margin:0}
.journal-facts{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.55rem}
.journal-facts>div{display:grid;gap:.25rem;padding:.65rem;background:var(--surface-2);border:1px solid var(--border);border-radius:.4rem;min-width:0}
.journal-facts span{font-size:.75rem;color:var(--text-secondary)}
.journal-facts strong,.journal-facts code{font-size:.85rem;overflow-wrap:anywhere}
.journal-detail-columns{display:grid;grid-template-columns:1fr 1fr;gap:1rem}
.journal-context{display:grid;grid-template-columns:minmax(7rem,1fr) 2fr;gap:.3rem .6rem;font-size:.82rem}
.journal-context dt{color:var(--text-secondary)}
.journal-context dd{margin:0;overflow-wrap:anywhere}
.journal-raw{border:1px solid var(--border);background:var(--surface-2);padding:.7rem;border-radius:.4rem;font-size:.78rem;line-height:1.4;overflow:auto;max-height:16rem;white-space:pre-wrap;overflow-wrap:anywhere}
.journal-timeline{list-style:none;padding-left:1.2rem;border-left:2px solid var(--border);margin:0;display:grid;gap:.55rem}
.journal-timeline li{position:relative;padding-left:.4rem;font-size:.82rem}
.journal-timeline li::before{content:"";width:.5rem;height:.5rem;border-radius:50%;background:var(--accent);position:absolute;left:-1.5rem;top:.4rem}
.journal-timeline span{margin-left:.5rem;color:var(--text-secondary)}
.journal-raw-records summary{cursor:pointer;color:var(--accent)}
@media(max-width:1180px){.journal-query{grid-template-columns:repeat(6,minmax(0,1fr))}.journal-query>.wide{grid-column:span 6}.journal-query>label{grid-column:span 3}.journal-query>.journal-small{grid-column:span 3}.journal-filters-actions{grid-column:span 6}.journal-facts{grid-template-columns:repeat(2,minmax(0,1fr))}.journal-outcome-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:650px){.journal-investigation{padding:.7rem}.journal-query{grid-template-columns:1fr}.journal-query>.wide,.journal-query>label,.journal-query>.journal-small,.journal-filters-actions{grid-column:1}.journal-stats,.journal-facts,.journal-detail-columns,.journal-outcome-grid{grid-template-columns:1fr 1fr}.journal-detail-columns{grid-template-columns:1fr}}
</style>

<style scoped>
.journal-operation-result{border:1px solid var(--border);border-radius:.65rem;padding:.8rem 1rem;margin:.6rem 0;background:var(--surface-2)}
.journal-operation-result h6{margin:0 0 .5rem}
.journal-operation-result p{margin:.28rem 0}
.journal-sql-list{margin-top:.5rem;max-height:12rem;overflow:auto;font-size:.81rem;display:grid;gap:.25rem}
</style>
