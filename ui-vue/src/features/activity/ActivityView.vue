<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import UiIcon from "../../components/UiIcon.vue";
import { t } from "../i18n/locale-service.js";
import { formatDuration, formatInteger, formatNumber, formatSnapshotTime } from "../../core/formatters.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import {
  ACTIVITY_DURATIONS, ACTIVITY_INTERVALS, ACTIVITY_PAGE_SIZES,
  activityPage, activityQueryDurationTier, activitySqlPreview, canBootstrapActivityConnections,
  createActivityController, createActivityState, sampleSummary,
  type ActivityBackend, type ActivityFilter, type SignalAction,
} from "./controller.js";

const emit = defineEmits<{ "operation-change": [operation: { id: "activity"; title: string; connection: string; startedAt: number; polls?: { requests: number; succeeded: number; failed: number; lastMs: number | null; maxMs: number; totalMs: number } } | null] }>();
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; quickSelection?: QuickConnectionSelection | null;}>();
const state = reactive(createActivityState());
const controller = createActivityController(props.shell, state, props.invoke);
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.monitoring || state.signaling || state.loading)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.monitoring || state.signaling || state.loading) return;
  lastShortcutSerial = request.serial;
  // Never retarget a running collector/monitor or an in-flight database operation.
  if (state.selectedConnectionId !== request.connectionId) controller.changeConnection(request.connectionId);
});

const intervalMs = ref(5000);
const durationMs = ref(60000);
const hideIdle = ref(false);
const filter = ref<ActivityFilter>("all");
const search = ref("");
const page = ref(1);
const pageSize = ref<number>(15);
// SQL must be explicitly opened; never keep a duplicate of the query outside the snapshot.
const sqlDialog = ref<HTMLDialogElement | null>(null);
const sqlTarget = ref<{ connectionId: number; authGeneration: number; capturedAt: number; backend: ActivityBackend } | null>(null);
const sqlBackend = computed(() => {
  const target = sqlTarget.value;
  return target && state.selectedConnectionId === target.connectionId &&
    props.shell.authGeneration === target.authGeneration ? target.backend : null;
});
const sqlPreview = computed(() => activitySqlPreview(sqlBackend.value?.query ?? ""));
const sqlCopyStatus = ref<string | null>(null);
async function copySql(): Promise<void> {
  const target = sqlTarget.value;
  const sql = sqlBackend.value?.query;
  if (!target || !sql) return;
  try {
    if (!navigator.clipboard?.writeText) throw new Error(t('activity.clipboardMissing'));
    await navigator.clipboard.writeText(sql);
    if (sqlTarget.value === target) sqlCopyStatus.value = t('activity.sqlCopied');
  } catch {
    if (sqlTarget.value === target) sqlCopyStatus.value = t('activity.sqlCopyFailed');
  }
}
function closeSql(): void {
  if (sqlDialog.value?.open) sqlDialog.value.close();
  sqlTarget.value = null;
  sqlCopyStatus.value = null;
}
function onSqlBackdropClick(event: MouseEvent): void {
  // Native <dialog> sends backdrop clicks to the dialog element itself.
  // Inside-panel clicks target its descendants and must not close it.
  if (event.target === sqlDialog.value) closeSql();
}
async function openSql(backend: ActivityBackend): Promise<void> {
  const snapshot = state.snapshot;
  const connectionId = state.selectedConnectionId;
  if (!backend.query || !snapshot || connectionId === null || state.snapshotConnectionId !== connectionId ||
      !snapshot.backends.some(b => b.pid === backend.pid && b.backend_start_ms === backend.backend_start_ms)) return;
  // Freeze the displayed snapshot, so polling does not close the panel or silently replace SQL.
  sqlTarget.value = { connectionId, authGeneration: props.shell.authGeneration, capturedAt: snapshot.captured_at_ms, backend };
  sqlCopyStatus.value = null;
  await nextTick();
  if (props.visible && sqlBackend.value && sqlDialog.value && !sqlDialog.value.open) sqlDialog.value.showModal();
}
// Fast SQLite/UI reads may finish before the browser paints a single spinner frame.
// Keep feedback visible without delaying the IPC request itself.
const refreshVisual = ref(false);
const connectionsVisual = ref(false);
const connectionsLoaded = ref(false);
let initialConnectionsPending = true;
let mounted = true;
let visualTimer: ReturnType<typeof setTimeout> | null = null;
let connectionTimer: ReturnType<typeof setTimeout> | null = null;
function clearVisuals(): void {
  if (visualTimer !== null) clearTimeout(visualTimer);
  if (connectionTimer !== null) clearTimeout(connectionTimer);
  visualTimer = null;
  connectionTimer = null;
  refreshVisual.value = false;
  connectionsVisual.value = false;
}
async function visibleRefresh(kind: "snapshot" | "connections", operation: () => Promise<unknown>): Promise<void> {
  const started = Date.now();
  if (kind === "snapshot") refreshVisual.value = true;
  else connectionsVisual.value = true;
  try { await operation(); }
  finally {
    if (!mounted) return;
    const remaining = Math.max(0, 450 - (Date.now() - started));
    const timer = setTimeout(() => {
      if (!mounted) return;
      if (kind === "snapshot") { refreshVisual.value = false; visualTimer = null; }
      else { connectionsVisual.value = false; connectionTimer = null; }
    }, remaining);
    if (kind === "snapshot") visualTimer = timer;
    else connectionTimer = timer;
  }
}
async function refreshConnections(): Promise<void> {
  if (connectionsVisual.value) return;
  await visibleRefresh("connections", async () => {
    if (await controller.refreshConnections()) connectionsLoaded.value = true;
  });
}
function loadInitialConnectionsWhenReady(): void {
  if (!mounted || !canBootstrapActivityConnections(props.shell, initialConnectionsPending)) return;
  initialConnectionsPending = false;
  void refreshConnections();
}
const table = computed(() => activityPage(state.snapshot, {
  filter: filter.value, search: search.value, hideIdle: hideIdle.value,
  page: page.value, pageSize: pageSize.value,
}));
const samples = computed(() => sampleSummary(state.samples));
const selected = computed(() => state.connections.find(c => c.id === state.selectedConnectionId));
watch(() => [state.monitoring, state.selectedConnectionId, state.monitorStartedAt, state.connections, state.pollMetrics.requests] as const, () => {
  emit("operation-change", state.monitoring && state.monitorStartedAt !== null
    ? { id: "activity", title: "Activity", connection: selected.value?.name ?? String(state.selectedConnectionId), startedAt: state.monitorStartedAt, polls: { ...state.pollMetrics } }
    : null);
}, { deep: true, immediate: true });
watch([filter, search, hideIdle, pageSize], () => { page.value = 1; });
watch(() => state.selectedConnectionId, closeSql);
// The monitor keeps polling while hidden, but its modal must not cover another tab.
watch(() => props.visible, visible => { if (!visible) closeSql(); });
// The keep-alive ActivityView mounts during login while shell.busy is still true.
// Retry the initial list after login completes, without clearing a running monitor on navigation.
watch(() => props.shell.busy, loadInitialConnectionsWhenReady);
watch(() => props.shell.authGeneration, () => {
  closeSql(); clearVisuals(); controller.reset();
  connectionsLoaded.value = false;
  initialConnectionsPending = true;
  loadInitialConnectionsWhenReady();
});
watch(() => props.shell.screen, screen => {
  if (screen !== "workspace") {
    closeSql(); clearVisuals(); controller.reset();
    connectionsLoaded.value = false;
    initialConnectionsPending = true;
  } else loadInitialConnectionsWhenReady();
});
watch(() => props.visible, visible => {
  // A profile can be added, edited or removed while this keep-alive view is hidden.
  // Never reset a running monitor or its samples just to refresh the selector.
  if (!visible || props.shell.screen !== "workspace" || props.shell.busy) return;
  if (initialConnectionsPending) { loadInitialConnectionsWhenReady(); return; }
  if (!state.monitoring && !state.signaling) void refreshConnections();
});
onMounted(loadInitialConnectionsWhenReady);
onUnmounted(() => { emit("operation-change", null); mounted = false; closeSql(); clearVisuals(); controller.dispose(); });

function onConnectionChange(event: Event): void {
  const value = Number((event.target as HTMLSelectElement).value);
  controller.changeConnection(Number.isSafeInteger(value) && value > 0 ? value : null);
  page.value = 1;
}
async function refreshOnce(): Promise<void> { if (refreshVisual.value) return; await visibleRefresh("snapshot", () => controller.refresh()); page.value = 1; }
function startMonitor(): void { controller.start(intervalMs.value, durationMs.value); }
function backendIdentity(backend: ActivityBackend): string {
  return `${backend.username ?? "—"} / ${backend.database ?? "—"}`;
}
function durationLabel(ms: number): string {
  return ms < 60000 ? t('activity.seconds', { value: String(ms / 1000) }) : t('activity.minutes', { value: String(ms / 60000) });
}
function stateLabel(backend: ActivityBackend): string {
  if (backend.state?.startsWith("idle in transaction")) return "Idle in transaction";
  return backend.state ?? t('activity.unknown');
}
function backendStateClass(backend: ActivityBackend): string {
  if (backend.state === "active") return "active";
  if (backend.state?.startsWith("idle in transaction")) return "warning";
  return "neutral";
}
function waitLabel(backend: ActivityBackend): string {
  return backend.wait_event_type ? `${backend.wait_event_type}: ${backend.wait_event ?? "—"}` : "—";
}
function onAction(backend: ActivityBackend, action: SignalAction): void {
  void controller.signal(backend, action, (question) => confirmAction(question));
}
</script>

<template>
  <section class="activity-view" :aria-label="t('activity.viewAria')">
    <div class="activity-heading">
      <div>
        <h1>{{ t('activity.title') }}</h1>
        <p class="activity-muted">{{ t('activity.subtitle') }}</p>
      </div>
    </div>

    <p class="activity-muted activity-snapshot-explainer">{{ t('activity.explainer') }}</p>

    <div class="activity-controls">
      <label class="control-connection">{{ t('activity.connection') }}
        <select :value="state.selectedConnectionId ?? ''" :disabled="state.monitoring || state.signaling || state.connectionsLoading"
          @change="onConnectionChange">
          <option value="" disabled>{{ t('activity.chooseConnection') }}</option>
          <option v-for="conn in state.connections" :key="conn.id" :value="conn.id">
            {{ conn.name }} · {{ conn.username }}@{{ conn.host }}:{{ conn.port }}/{{ conn.database }}
          </option>
        </select>
      </label>
      <label>{{ t('activity.interval') }}
        <select v-model.number="intervalMs" :disabled="state.monitoring">
          <option v-for="value in ACTIVITY_INTERVALS" :key="value" :value="value">{{ t('activity.seconds', { value: String(value / 1000) }) }}</option>
        </select>
      </label>
      <label>{{ t('activity.duration') }}
        <select v-model.number="durationMs" :disabled="state.monitoring">
          <option v-for="value in ACTIVITY_DURATIONS" :key="value" :value="value">{{ durationLabel(value) }}</option>
        </select>
      </label>
      <div class="activity-buttons">
        <button v-if="!state.monitoring" type="button" :disabled="state.selectedConnectionId === null || state.loading || state.signaling" @click="startMonitor">
          <UiIcon name="activity" :size="17" /> {{ t('activity.start') }}
        </button>
        <button v-else type="button" class="secondary" @click="controller.stop(false)">{{ t('activity.stop') }}</button>
        <button type="button" class="secondary refresh-icon" :disabled="state.selectedConnectionId === null || state.loading || state.monitoring || state.signaling || refreshVisual"
          :title="t('activity.singleSnapshot')" :aria-label="t('activity.singleSnapshot')" @click="refreshOnce">
          <UiIcon name="refresh" :size="18" :class="{ 'activity-spin': refreshVisual || state.loading }" />
        </button>
      </div>
    </div>

    <p v-if="state.error" class="activity-feedback error" role="alert">{{ state.error }}</p>
    <p v-if="state.notice" class="activity-feedback" role="status">{{ state.notice }}</p>
    <p v-if="state.pollMetrics.requests" class="muted">{{ t('activity.polling', { calls: String(state.pollMetrics.requests), success: String(state.pollMetrics.succeeded), errors: String(state.pollMetrics.failed), average: String(Math.round(state.pollMetrics.totalMs / state.pollMetrics.requests)), max: String(Math.round(state.pollMetrics.maxMs)) }) }}</p>
    <p v-if="state.monitoring || state.loading" class="activity-feedback pending" role="status">
      <span class="button-spinner" aria-hidden="true"></span>
      {{ state.monitoring ? t('activity.monitoringCount', { count: String(state.samples.length) }) : t('activity.loadingSnapshot') }}
    </p>
    <p v-if="connectionsLoaded && !state.connections.length && !state.connectionsLoading && !state.error" class="activity-muted">
      {{ t('activity.noConnection') }}
    </p>

    <section class="activity-current" aria-labelledby="activity-current-title">
      <div class="activity-section-heading">
        <h2 id="activity-current-title">{{ t('activity.currentSnapshot') }}</h2>
        <span class="activity-muted" v-if="state.snapshot">{{ formatSnapshotTime(state.snapshot.captured_at_ms) }}<template v-if="selected"> · {{ selected.name }}</template></span>
      </div>
    <div class="activity-metrics" :aria-label="t('activity.metricsAria')">
      <article v-for="m in [
        { label: 'Client backend', value: state.snapshot?.total_connections },
        { label: t('activity.active'), value: state.snapshot?.active_connections },
        { label: t('activity.activeWaiting'), value: state.snapshot?.waiting_active_connections },
        { label: 'Idle in transaction', value: state.snapshot?.idle_in_transaction_connections },
        { label: t('activity.pgxrayByName'), value: state.snapshot?.pgxray_connections },
      ]" :key="m.label" class="activity-metric">
        <span>{{ m.label }}</span>
        <strong>{{ m.value === undefined ? '—' : formatInteger(m.value) }}</strong>
      </article>
    </div>

    </section>
    <section v-if="samples.count" class="activity-monitor-summary" aria-labelledby="activity-monitor-summary-title">
      <div class="activity-section-heading">
        <h2 id="activity-monitor-summary-title">{{ t('activity.summary') }}</h2>
        <span class="activity-muted">{{ t('activity.sampleCount', { count: String(samples.count) }) }}</span>
      </div>
      <div class="activity-sample-bar">
        <span>{{ t('activity.avgActive', { value: formatNumber(samples.averageActive, 1) }) }}</span>
        <span>{{ t('activity.maxActive', { value: String(samples.maxActive) }) }}</span>
        <span>{{ t('activity.maxWaiting', { value: String(samples.maxWaiting) }) }}</span>
        <span>{{ t('activity.maxConnections', { value: String(samples.maxConnections) }) }}</span>
        <span>{{ t('activity.elapsed', { value: formatDuration(Math.max(0, (state.monitorEndedAt ?? Date.now()) - (state.monitorStartedAt ?? Date.now())) / 1000) }) }}</span>
      </div>
    </section>

    <div class="activity-table-head">
      <div>
        <h2>{{ t('activity.backends') }}</h2>
        <p class="activity-muted">{{ t('activity.backendHint') }}</p>
      </div>
    </div>
    <div class="activity-filterbar">
      <label>{{ t('activity.search') }}
        <input v-model="search" type="search" :placeholder="t('activity.searchPlaceholder')" autocomplete="off" />
      </label>
      <label>{{ t('activity.state') }}
        <select v-model="filter">
          <option value="all">{{ t('activity.all') }}</option>
          <option value="active">Active</option>
          <option value="waiting">Active + wait event</option>
          <option value="idle-xact">Idle in transaction</option>
        </select>
      </label>
      <label>{{ t('activity.rows') }}
        <select v-model.number="pageSize">
          <option v-for="size in ACTIVITY_PAGE_SIZES" :key="size" :value="size">{{ size }}</option>
        </select>
      </label>
      <label class="activity-check"><input v-model="hideIdle" type="checkbox" /> {{ t('activity.hideIdle') }}</label>
    </div>

    <p class="activity-count">{{ t('activity.shown', { first: String(table.total ? ((table.page - 1) * pageSize + 1) : 0), last: String(Math.min(table.page * pageSize, table.total)), total: String(table.total) }) }}</p>
    <div class="activity-scroll">
      <table class="activity-backends-table">
        <thead>
          <tr><th>PID</th><th>{{ t('activity.roleDatabase') }}</th><th>{{ t('activity.appClient') }}</th><th>{{ t('activity.state') }}</th>
            <th>{{ t('activity.query') }}</th><th>{{ t('activity.transaction') }}</th><th>{{ t('activity.wait') }}</th><th>SQL</th><th>{{ t('activity.control') }}</th></tr>
        </thead>
        <tbody>
          <tr v-if="table.rows.length === 0"><td colspan="9" class="activity-muted">
            {{ state.snapshot ? t('activity.noBackends') : t('activity.firstSnapshot') }}
          </td></tr>
          <tr v-for="backend in table.rows" :key="`${backend.pid}:${backend.backend_start_ms ?? 'unknown'}`" :class="{ 'activity-own': backend.is_pgxray }">
            <td class="activity-mono">{{ backend.pid }}</td>
            <td>{{ backendIdentity(backend) }}</td>
            <td><strong>{{ backend.application_name || '—' }}</strong><div class="activity-muted">{{ backend.client_addr ?? 'local' }}</div></td>
            <td><span class="activity-badge" :data-state="backendStateClass(backend)">{{ stateLabel(backend) }}</span></td>
            <td class="activity-query-age" :data-duration="activityQueryDurationTier(backend) ?? undefined">
              {{ formatDuration(backend.query_age_seconds) }}
              <span v-if="activityQueryDurationTier(backend) === 'watch'" class="activity-query-age-hint">{{ t('activity.overMinute') }}</span>
              <span v-if="activityQueryDurationTier(backend) === 'long'" class="activity-query-age-hint">{{ t('activity.overFive') }}</span>
            </td>
            <td>{{ formatDuration(backend.xact_age_seconds) }}</td>
            <td :class="{ 'activity-lock': backend.wait_event_type === 'Lock' }">{{ waitLabel(backend) }}</td>
            <td>
              <button v-if="backend.query" type="button" class="secondary activity-sql-open"
                :aria-label="`${backend.state === 'idle' ? t('activity.showLastSql') : t('activity.showSql')} PID ${backend.pid}`"
                @click="openSql(backend)">SQL</button>
              <span v-else>—</span>
            </td>
            <td class="activity-row-actions">
              <button type="button" class="secondary" :disabled="!controller.canSignal(backend, 'cancel')"
                :title="t('activity.cancelPid', { pid: String(backend.pid) })" @click="onAction(backend, 'cancel')">{{ t('activity.cancelQuery') }}</button>
              <button type="button" class="activity-danger" :disabled="!controller.canSignal(backend, 'terminate')"
                :title="t('activity.terminatePid', { pid: String(backend.pid) })" @click="onAction(backend, 'terminate')">{{ t('activity.terminate') }}</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <div class="activity-footer">
      <p class="activity-muted">{{ t('activity.actionHint') }}</p>
      <div class="activity-pager">
        <button type="button" class="secondary" :disabled="table.page <= 1" @click="page--">{{ t('activity.previous') }}</button>
        <span>{{ table.page }} / {{ table.pages }}</span>
        <button type="button" class="secondary" :disabled="table.page >= table.pages" @click="page++">{{ t('activity.next') }}</button>
      </div>
    </div>
    <dialog ref="sqlDialog" class="activity-sql-dialog" aria-labelledby="activity-sql-title"
      @click="onSqlBackdropClick" @close="sqlTarget = null">
      <div v-if="sqlBackend" class="activity-sql-panel">
        <div class="activity-sql-header">
          <div>
            <h2 id="activity-sql-title">SQL · PID {{ sqlBackend.pid }}</h2>
            <p class="activity-muted">{{ backendIdentity(sqlBackend) }} · {{ sqlBackend.application_name || t('activity.appUnknown') }}</p>
          </div>
          <button type="button" class="secondary activity-sql-close" :aria-label="t('activity.closeSql')" @click="closeSql">
            <UiIcon name="x" :size="18" />
          </button>
        </div>
        <p class="activity-muted">{{ t('activity.sqlSnapshot', { date: sqlTarget ? formatSnapshotTime(sqlTarget.capturedAt) : '—' }) }}</p>
        <p class="activity-muted">{{ stateLabel(sqlBackend) }} · {{ t('activity.sqlMeta', { age: formatDuration(sqlBackend.query_age_seconds), wait: waitLabel(sqlBackend) }) }}</p>
        <p v-if="sqlBackend.state !== 'active'" class="activity-sql-warning">{{ t('activity.lastSqlWarning') }}</p>
        <p class="activity-muted">{{ t('activity.sqlPrivacy') }}</p>
        <div class="activity-sql-tools">
          <button type="button" class="secondary" @click="copySql">{{ t('activity.copyReceived') }}</button>
          <span v-if="sqlPreview.truncated" class="activity-sql-warning" role="status">{{ t('activity.truncated') }}</span>
        </div>
        <p v-if="sqlCopyStatus" class="activity-muted" role="status">{{ sqlCopyStatus }}</p>
        <pre class="activity-sql-text">{{ sqlPreview.text }}</pre>
      </div>
    </dialog>
    <p class="activity-muted activity-disclaimer">{{ t('activity.disclaimer') }}</p>
  </section>
</template>

<style scoped>
.activity-view{display:grid;gap:1.15rem;min-width:0}.activity-view h1{font-size:1.55rem;margin:0 0 .35rem}.activity-view h2{font-size:1.12rem;margin:0 0 .3rem}
.activity-muted{color:var(--text-secondary);font-size:.84rem;line-height:1.5;margin:0}
.activity-heading,.activity-table-head{display:flex;justify-content:space-between;align-items:start;gap:1rem}.activity-snapshot-explainer{padding:.55rem .8rem;border-left:2px solid var(--border-strong);background:var(--surface);border-radius:.35rem}
.activity-controls{display:grid;grid-template-columns:minmax(260px,2fr) minmax(130px,.55fr) minmax(130px,.65fr) auto;align-items:end;gap:.7rem;background:var(--surface);border:1px solid var(--border);border-radius:.75rem;padding:1rem}
.activity-controls label,.activity-filterbar label{display:grid;gap:.4rem;font-size:.85rem;min-width:0}
.activity-view select,.activity-view input[type=search]{width:100%;min-width:0;box-sizing:border-box;padding:.65rem;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.45rem}
.activity-buttons{display:flex;align-items:center;gap:.4rem}.activity-buttons button{white-space:nowrap;display:flex;gap:.35rem;align-items:center;justify-content:center}
.refresh-icon{width:2.65rem;height:2.65rem;padding:.5rem;display:inline-flex;justify-content:center;align-items:center}
.activity-spin{animation:activity-spin .75s linear infinite}@keyframes activity-spin{to{transform:rotate(360deg)}}
.activity-feedback{padding:.65rem .9rem;border-radius:.5rem;background:var(--surface-2);margin:0;color:var(--accent)}.activity-feedback.error{color:var(--error)}.activity-feedback.pending{color:var(--text-secondary)}
.activity-metrics{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:.65rem}.activity-metric{display:flex;flex-direction:column;gap:.5rem;padding:1rem;border:1px solid var(--border);border-radius:.65rem;background:var(--surface)}.activity-metric span{color:var(--text-secondary);font-size:.8rem}.activity-metric strong{font-size:1.7rem;font-variant-numeric:tabular-nums}
.activity-current,.activity-monitor-summary{display:grid;gap:.6rem;min-width:0}
.activity-section-heading{display:flex;align-items:baseline;justify-content:space-between;gap:1rem;flex-wrap:wrap}
.activity-section-heading h2{margin:0}
.activity-sample-bar{display:flex;flex-wrap:wrap;gap:.7rem 1.4rem;border-bottom:1px solid var(--border);padding:.5rem 0;color:var(--text-secondary);font-size:.82rem}
.activity-filterbar{display:grid;grid-template-columns:minmax(200px,2fr) minmax(150px,1fr) 100px auto;gap:.65rem;align-items:end}.activity-check{display:flex!important;flex-direction:row;align-items:center;gap:.4rem!important;padding-bottom:.6rem;white-space:nowrap}
.activity-count{font-size:.78rem;color:var(--text-secondary);margin:0}.activity-scroll{overflow:auto;border:1px solid var(--border);border-radius:.7rem}
.activity-backends-table{width:100%;border-collapse:collapse;text-align:left;font-size:.82rem}.activity-backends-table th,.activity-backends-table td{padding:.75rem .65rem;border-bottom:1px solid var(--border);vertical-align:top;min-width:60px}.activity-backends-table th{background:var(--surface);white-space:nowrap}.activity-backends-table tr:last-child td{border-bottom:0}.activity-own{background:var(--surface-2)}
.activity-mono{font-variant-numeric:tabular-nums}.activity-badge{display:inline-block;border:1px solid var(--border-strong);border-radius:999px;padding:.2rem .5rem;white-space:nowrap}.activity-badge[data-state=active]{color:var(--accent)}.activity-badge[data-state=warning],.activity-lock{color:var(--warning,var(--accent))}
.activity-query-age{font-variant-numeric:tabular-nums}.activity-query-age[data-duration=watch]{color:var(--warning,var(--accent))}.activity-query-age[data-duration=long]{color:var(--error)}.activity-query-age-hint{display:block;font-size:.7rem;font-weight:600;white-space:nowrap}
.activity-sql-open{font-size:.78rem;padding:.35rem .7rem}.activity-sql-dialog{box-sizing:border-box;position:fixed;inset:0 0 0 auto;margin:0;height:100dvh;max-height:100dvh;width:min(680px,100vw);max-width:none;padding:0;background:var(--surface);color:var(--text);border:0;border-left:1px solid var(--border-strong);box-shadow:-12px 0 35px rgba(0,0,0,.3);overflow:auto}.activity-sql-dialog::backdrop{background:rgba(0,0,0,.62)}.activity-sql-panel{display:grid;gap:1rem;padding:1.3rem;min-width:0}.activity-sql-header{display:flex;align-items:start;justify-content:space-between;gap:1rem}.activity-sql-header h2{margin:0 0 .4rem}.activity-sql-close{display:flex;align-items:center;justify-content:center;flex:none;padding:.5rem}.activity-sql-warning{border-left:3px solid var(--warning,var(--accent));padding:.5rem .7rem;background:var(--surface-2);font-size:.84rem;margin:0}.activity-sql-tools{display:flex;align-items:center;gap:.65rem;flex-wrap:wrap}.activity-sql-tools button{flex:none}.activity-sql-tools .activity-sql-warning{flex:1;min-width:240px}.activity-sql-text{white-space:pre;overflow:auto;max-height:calc(100dvh - 210px);padding:1rem;border:1px solid var(--border);border-radius:.5rem;background:var(--surface-2);font-size:.82rem;line-height:1.5;margin:0;tab-size:2}.activity-row-actions{white-space:nowrap}.activity-row-actions button{font-size:.76rem;padding:.4rem .5rem;margin:.05rem;white-space:normal}.activity-danger{color:var(--error);background:var(--surface);border:1px solid var(--border-strong)}
.activity-footer{display:flex;justify-content:space-between;align-items:center;gap:1rem;flex-wrap:wrap}.activity-pager{display:flex;align-items:center;gap:.5rem;font-size:.82rem}.activity-pager button{padding:.4rem .7rem}.activity-disclaimer{border-top:1px solid var(--border);padding-top:.65rem}
@media(max-width:1100px){.activity-controls{grid-template-columns:1fr 1fr}.control-connection{grid-column:1/-1}.activity-buttons{justify-content:end}.activity-metrics{grid-template-columns:repeat(3,minmax(0,1fr))}.activity-filterbar{grid-template-columns:1fr 1fr}}
@media(max-width:650px){.activity-controls,.activity-filterbar{grid-template-columns:1fr}.control-connection{grid-column:auto}.activity-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(prefers-reduced-motion:reduce){.activity-spin{animation:none}}
</style>
