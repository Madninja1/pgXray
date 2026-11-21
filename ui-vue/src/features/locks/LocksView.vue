<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import { LOCKS_INTERVALS, createLocksController, createLocksState, sameLockIncidentEpisode, type LockBackend, type LockPath } from "./controller.js";

const emit = defineEmits<{ "operation-change": [operation: { id: "locks"; title: string; connection: string; startedAt: number; polls?: { requests: number; succeeded: number; failed: number; lastMs: number | null; maxMs: number; totalMs: number } } | null] }>();
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; quickSelection?: QuickConnectionSelection | null;}>();
const state = reactive(createLocksState());
const controller = createLocksController(props.shell, state, props.invoke);
const confirmationDialog = ref<HTMLDialogElement | null>(null);
const confirmationMessage = ref("");
let resolveConfirmation: ((accepted: boolean) => void) | null = null;
function finishConfirmation(accepted: boolean): void {
  const resolve = resolveConfirmation;
  resolveConfirmation = null;
  if (confirmationDialog.value?.open) confirmationDialog.value.close();
  confirmationMessage.value = "";
  resolve?.(accepted);
}
function requestConfirmation(message: string): Promise<boolean> {
  if (resolveConfirmation) return Promise.resolve(false);
  confirmationMessage.value = message;
  return new Promise(resolve => {
    resolveConfirmation = resolve;
    void nextTick().then(() => {
      if (!resolveConfirmation) return;
      try { confirmationDialog.value?.showModal(); }
      catch { finishConfirmation(false); }
      if (!confirmationDialog.value?.open) finishConfirmation(false);
    });
  });
}
watch(() => [props.visible, props.shell.authGeneration] as const, ([visible]) => {
  if (!visible) finishConfirmation(false);
});
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.monitoring || state.signaling || state.saving || state.loading)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.monitoring || state.signaling || state.saving || state.loading) return;
  lastShortcutSerial = request.serial;
  // Never retarget a running collector/monitor or an in-flight database operation.
  if (state.connectionId !== request.connectionId) controller.changeConnection(request.connectionId);
});

const interval = ref(5000);
const selected = computed(() => state.connections.find(c => c.id === state.connectionId));
const blocked = computed(() => state.snapshot?.summary.blocked_backends ?? 0);
const idleTotal = computed(() => state.snapshot?.idle_xact_total ?? 0);
const oldestIdle = computed(() => Math.max(0, ...((state.snapshot?.idle_xacts ?? []).map(tx => tx.xact_age_seconds ?? 0))));
const idleWithWaiters = computed(() => (state.snapshot?.idle_xacts ?? []).filter(tx => tx.is_blocking).length);
function formatAge(seconds: number | null): string {
  if (seconds === null) return "—";
  const whole = Math.max(0, Math.floor(seconds));
  if (whole < 60) return t("locks.seconds", {count: String(whole)});
  if (whole < 3600) return t("locks.minutesSeconds", {minutes: String(Math.floor(whole / 60)),seconds: String(whole % 60)});
  return t("locks.hoursMinutes", {hours: String(Math.floor(whole / 3600)),minutes: String(Math.floor(whole % 3600 / 60))});
}
function idleLabel(seconds: number | null): string {
  if (seconds === null) return t('locks.unknown');
  if (seconds >= 300) return t('locks.investigate');
  if (seconds >= 60) return t('locks.watch');
  return t('locks.recent');
}
let initialized = false;
function bootstrap(): void {
  if (initialized || props.shell.busy || props.shell.screen !== "workspace" || !props.shell.currentUsername) return;
  initialized = true;
  void controller.refreshConnections();
}
watch(() => props.shell.busy, bootstrap);
watch(() => props.shell.screen, screen => {
  if (screen !== "workspace") { controller.reset(); initialized = false; }
  else bootstrap();
});
watch(() => props.shell.authGeneration, () => { controller.reset(); initialized = false; bootstrap(); });
watch(() => props.visible, value => {
  if (!value || state.monitoring) return;
  if (initialized && props.shell.screen === "workspace" && !props.shell.busy)
    void controller.refreshConnections();
  else bootstrap();
});
onMounted(bootstrap);
watch(() => [state.monitoring, state.connectionId, state.startedAt, state.connections, state.pollMetrics.requests] as const, () => {
  emit("operation-change", state.monitoring && state.startedAt !== null
    ? { id: "locks", title: "Locks / Waits", connection: selected.value?.name ?? String(state.connectionId), startedAt: state.startedAt, polls: { ...state.pollMetrics } }
    : null);
}, { deep: true, immediate: true });
onUnmounted(() => { finishConfirmation(false); emit("operation-change", null); controller.dispose(); });
function pathLabel(path: LockPath): string {
  return path.pids.map(pid => pid === 0 ? "Prepared transaction (PID 0)" : String(pid)).join(" → ");
}
function pathStatus(path: LockPath): string {
  if (path.cycle_detected) return t('locks.cycle');
  if (path.prepared_transaction) return "Prepared transaction";
  if (path.incomplete) return t('locks.incomplete');
  return t('locks.rootBlocker');
}
function formatTime(value: number | null): string {
  return value === null ? "—" : new Date(value).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US");
}
function formatDuration(ms: number | null): string {
  if (ms === null) return "—";
  return t("locks.seconds", {count: (ms / 1000).toFixed(1)});
}
async function saveWithConfirmation(): Promise<void> {
  const incident = state.incident;
  const caseId = state.caseId;
  const connectionId = state.connectionId;
  if (!incident || state.saving) return;
  if (!await requestConfirmation(t('locks.confirmation'))) return;
  if (!sameLockIncidentEpisode(incident, state.incident) || state.caseId !== caseId || state.connectionId !== connectionId) {
    state.error = t('locks.saveAborted');
    return;
  }
  const saved = await controller.saveIncident();
  if (!saved && !state.error) state.error = t('locks.saveFailed');
}
function sendSignal(backend: LockBackend, action: "cancel" | "terminate"): void {
  void controller.signal(backend, action, requestConfirmation);
}
function connectionChange(event: Event): void {
  const id = Number((event.target as HTMLSelectElement).value);
  controller.changeConnection(Number.isSafeInteger(id) && id > 0 ? id : null);
}
</script>

<template>
  <section class="locks-view" aria-label="Locks / Waits PostgreSQL">
    <dialog ref="confirmationDialog" class="locks-confirm" aria-labelledby="locks-confirm-title"
      @cancel="$event.preventDefault(); finishConfirmation(false)" @close="finishConfirmation(false)">
      <h2 id="locks-confirm-title">{{ t('locks.confirmTitle') }}</h2>
      <p>{{ confirmationMessage }}</p>
      <div class="locks-confirm-actions">
        <button type="button" @click="finishConfirmation(true)">{{ t('locks.yes') }}</button>
        <button type="button" class="secondary" @click="finishConfirmation(false)">{{ t('locks.cancel') }}</button>
      </div>
    </dialog>
    <div class="locks-heading">
      <div><h1>Locks / Waits</h1><p class="muted">{{ t('locks.subtitle') }}</p></div>
    </div>
    <div class="locks-controls">
      <label>{{ t('locks.connection') }}
        <select :value="state.connectionId ?? ''" :disabled="state.monitoring || state.signaling || state.saving" @change="connectionChange">
          <option value="">{{ t('locks.choose') }}</option>
          <option v-for="connection in state.connections" :key="connection.id" :value="connection.id">{{ connection.name }} · {{ connection.host }}:{{ connection.port }}/{{ connection.database }}</option>
        </select>
      </label>
      <label>{{ t('locks.interval') }}
        <select v-model.number="interval" :disabled="state.monitoring">
          <option v-for="ms in LOCKS_INTERVALS" :key="ms" :value="ms">{{ t('locks.seconds', {count: String(ms / 1000)}) }}</option>
        </select>
      </label>
      <button v-if="!state.monitoring" type="button" :disabled="state.connectionId === null || state.loading || state.signaling" @click="controller.start(interval)">{{ t('locks.start') }}</button>
      <button v-else class="secondary" type="button" @click="controller.stop">{{ t('locks.stop') }}</button>
      <button type="button" class="secondary" :disabled="state.connectionId === null || state.monitoring || state.loading || state.signaling" @click="controller.refresh()">{{ t('locks.once') }}</button>
    </div>
    <p class="muted">{{ t('locks.monitorWarning') }}</p>
    <p v-if="state.error" role="alert" class="error">{{ state.error }}</p>
    <p v-if="state.notice" role="status">{{ state.notice }}</p>
    <p v-if="state.pollMetrics.requests" class="muted">{{ t('locks.captureCount') }}: {{ state.pollMetrics.requests }} · {{ t('locks.ok') }}: {{ state.pollMetrics.succeeded }} · {{ t('locks.errors') }}: {{ state.pollMetrics.failed }} · {{ t('locks.mean') }}: {{ Math.round(state.pollMetrics.totalMs / state.pollMetrics.requests) }} {{ t('locks.ms') }} · {{ t('locks.maximum') }}: {{ Math.round(state.pollMetrics.maxMs) }} {{ t('locks.ms') }}. {{ t('locks.captureExplained') }}</p>
    <p v-if="state.monitoring || state.loading" role="status">{{ state.monitoring ? t('locks.monitorRunning', {count: String(state.samples)}) : t('locks.snapshotLoading') }}</p>
    <section class="locks-panel" :aria-label="t('locks.statusAria')">
      <div class="locks-section-head">
        <div>
          <h2>{{ t('locks.stateHeading') }} <small v-if="selected">· {{ selected.name }}</small></h2>
          <p class="muted">{{ state.snapshot ? t('locks.snapshotTime', {time: formatTime(state.snapshot.captured_at_ms)}) : t('locks.snapshotWait') }}</p>
        </div>
        <span v-if="state.snapshot" class="locks-chip" :class="blocked > 0 ? 'locks-chip-danger' : 'locks-chip-ok'">
          {{ blocked > 0 ? t('locks.waitSnapshot', {count: String(blocked)}) : t('locks.noBlocked') }}
        </span>
      </div>
      <div class="locks-metrics locks-overview">
        <article><span>{{ t('locks.blocked') }}</span><strong>{{ state.snapshot?.summary.blocked_backends ?? '—' }}</strong><small>{{ t('locks.blockedHint') }}</small></article>
        <article><span>Idle in transaction</span><strong>{{ state.snapshot?.idle_xact_total ?? '—' }}</strong><small>{{ t('locks.idleHint') }}</small></article>
        <article><span>{{ t('locks.oldest') }}</span><strong>{{ state.snapshot && idleTotal ? formatAge(oldestIdle) : '—' }}</strong><small>{{ t('locks.oldestHint') }}</small></article>
        <article><span>{{ t('locks.chain') }}</span><strong>{{ state.snapshot?.summary.longest_chain_depth ?? '—' }}</strong><small>{{ t('locks.chainHint') }}</small></article>
      </div>
      <p v-for="message in state.snapshot?.limitations ?? []" :key="message" class="locks-warning">{{ message }}</p>
    </section>

    <section class="locks-panel" :aria-label="t('locks.idleAria')">
      <div class="locks-section-head">
        <div>
          <h2>{{ t('locks.idleHeading') }}</h2>
          <p class="muted">{{ t('locks.idleExplained') }}</p>
        </div>
        <span class="locks-chip" :class="idleWithWaiters ? 'locks-chip-danger' : idleTotal ? 'locks-chip-warn' : 'locks-chip-ok'">
          {{ idleWithWaiters ? t('locks.withWaiters', {count: String(idleWithWaiters)}) : idleTotal ? t('locks.found', {count: String(idleTotal)}) : t('locks.noIdle') }}
        </span>
      </div>
      <p v-if="!state.snapshot" class="muted">{{ t('locks.loading') }}</p>
      <p v-else-if="idleTotal === 0" class="locks-empty">{{ t('locks.idleEmpty') }}</p>
      <template v-else>
        <p class="muted">{{ t('locks.txListHint', {count: String(state.snapshot?.idle_xacts.length ?? 0),total: String(idleTotal)}) }}</p>
        <div class="locks-scroll"><table><thead><tr>
          <th>{{ t('locks.pidRole') }}</th><th>{{ t('locks.appClient') }}</th><th>{{ t('locks.transaction') }}</th><th>{{ t('locks.idle') }}</th><th>{{ t('locks.waits') }}</th><th>{{ t('locks.horizon') }}</th>
        </tr></thead><tbody>
          <tr v-for="tx in state.snapshot?.idle_xacts ?? []" :key="tx.pid">
            <td><strong>{{ tx.pid }}</strong><small>{{ tx.username ?? '—' }} / {{ tx.database ?? '—' }}</small></td>
            <td>{{ tx.application_name || t('locks.notSpecified') }}<small>{{ tx.client_addr ?? t('locks.addressUnavailable') }}</small></td>
            <td><strong>{{ formatAge(tx.xact_age_seconds) }}</strong><small :class="tx.xact_age_seconds !== null && tx.xact_age_seconds >= 300 ? 'locks-risk' : 'muted'">{{ idleLabel(tx.xact_age_seconds) }}</small></td>
            <td>{{ formatAge(tx.idle_age_seconds) }}</td>
            <td><span class="locks-chip" :class="tx.is_blocking ? 'locks-chip-danger' : 'locks-chip-ok'">{{ tx.is_blocking ? t('locks.hasWaiters') : t('locks.none') }}</span></td>
            <td><span>xmin: {{ tx.backend_xmin ?? '—' }}</span><small>xid: {{ tx.backend_xid ?? '—' }}</small></td>
          </tr>
        </tbody></table></div>
        <p class="muted">{{ t('locks.idleDisclaimer') }}</p>
      </template>
    </section>

    <section class="locks-panel" :aria-label="t('locks.chainAria')">
      <div class="locks-section-head"><div><h2>{{ t('locks.chainHeading') }}</h2><p class="muted">{{ t('locks.chainDesc') }}</p></div>
        <span v-if="state.snapshot && blocked" class="locks-chip locks-chip-danger">{{ t('locks.root') }}: {{ state.snapshot.summary.root_blockers }}</span>
      </div>
      <p v-if="!state.snapshot?.waits.length" class="locks-empty">{{ t('locks.chainEmpty') }}</p>
      <div v-for="wait in state.snapshot?.waits ?? []" :key="wait.blocked_pid" class="locks-wait">
        <strong>PID {{ wait.blocked_pid }}</strong> · {{ t('locks.direct') }}: {{ wait.immediate_blocker_pids.join(', ') }} · {{ t('locks.depth') }}: {{ wait.max_depth }}
        <div v-for="(path, index) in wait.paths" :key="index"><code>{{ pathLabel(path) }}</code> <span class="muted">{{ pathStatus(path) }}</span></div>
      </div>
      <details v-if="state.snapshot?.backends.length" class="locks-details">
        <summary>{{ t('locks.members') }} ({{ state.snapshot?.backends.length }}) · Cancel / Terminate</summary>
        <div class="locks-scroll"><table><thead><tr><th>PID</th><th>{{ t('locks.roleDb') }}</th><th>{{ t('locks.state') }}</th><th>{{ t('locks.application') }}</th><th>{{ t('locks.actions') }}</th></tr></thead><tbody>
          <tr v-for="backend in state.snapshot?.backends ?? []" :key="backend.pid">
            <td>{{ backend.pid }}</td><td>{{ backend.username ?? '—' }} / {{ backend.database ?? '—' }}</td>
            <td>{{ backend.state ?? '—' }}</td><td>{{ backend.application_name }}</td>
            <td class="locks-actions"><button class="secondary" type="button" :disabled="state.loading || state.signaling || backend.backend_start_ms === null || backend.backend_type !== 'client backend' || backend.state !== 'active'" @click="sendSignal(backend, 'cancel')">Cancel</button>
              <button class="secondary" type="button" :disabled="state.loading || state.signaling || backend.backend_start_ms === null || backend.backend_type !== 'client backend'" @click="sendSignal(backend, 'terminate')">Terminate</button></td>
          </tr>
        </tbody></table></div>
        <p class="muted">{{ t('locks.signalWarning') }}</p>
      </details>
      <details v-if="state.snapshot?.locks.length" class="locks-details">
        <summary>{{ t('locks.details') }} ({{ state.snapshot?.locks.length }} / {{ state.snapshot?.lock_count_total }})</summary>
        <div class="locks-scroll"><table><thead><tr><th>PID</th><th>{{ t('locks.type') }}</th><th>{{ t('locks.mode') }}</th><th>{{ t('locks.status') }}</th><th>{{ t('locks.object') }}</th></tr></thead><tbody>
          <tr v-for="(lock, index) in state.snapshot?.locks ?? []" :key="index"><td>{{ lock.pid ?? '—' }}</td><td>{{ lock.lock_type }}</td><td>{{ lock.mode }}</td><td>{{ lock.granted ? t('locks.granted') : t('locks.waiting') }}</td><td>{{ [lock.schema_name, lock.relation_name].filter(Boolean).join('.') || '—' }}</td></tr>
        </tbody></table></div>
      </details>
    </section>
    <section class="locks-panel" aria-label="Best Incident Snapshot">
      <div class="locks-section-head"><div><h2>{{ t('locks.incidentHeading') }}</h2><p class="muted">{{ t('locks.incidentDesc') }}</p></div>
        <span v-if="state.incident" class="locks-chip" :class="state.incident.endedAt === null ? 'locks-chip-warn' : 'locks-chip-ok'">{{ state.incident.endedAt === null ? t('locks.observed') : t('locks.ended') }}</span>
      </div>
      <p class="muted">{{ t('locks.timeDisclaimer') }}</p>
      <div v-if="state.incident" class="locks-metrics">
        <article><span>{{ t('locks.firstSeen') }}</span><strong>{{ formatTime(state.incident.startedAt) }}</strong></article>
        <article><span>{{ t('locks.lastSeen') }}</span><strong>{{ formatTime(state.incident.endedAt) }}</strong></article>
        <article><span>{{ t('locks.peakBlocked') }}</span><strong>{{ state.incident.peakBlocked }}</strong></article>
        <article><span>{{ t('locks.maxWait') }}</span><strong>{{ formatDuration(state.incident.longestWaitMs) }}</strong></article>
      </div>
      <p v-else class="muted">{{ t('locks.incidentEmpty') }}</p>
      <div class="locks-controls">
        <label>Case
          <select v-model.number="state.caseId" :disabled="state.saving || !state.incident">
            <option :value="null">{{ t('locks.newCase') }}</option>
            <option v-for="item in state.cases" :key="item.id" :value="item.id">#{{ item.id }} · {{ item.title }}</option>
          </select>
        </label>
        <button type="button" :disabled="!state.incident || state.saving || state.signaling" @click="saveWithConfirmation">{{ state.saving ? t('locks.saveBusy') : t('locks.saveEvidence') }}</button>
      </div>
      <p class="muted">{{ t('locks.saveHint') }}</p>
      <p v-if="state.notice?.startsWith('Сохранено в Case')" role="status">{{ state.notice }} · {{ t('locks.openFiles') }}</p>
      <p v-if="state.error?.startsWith('Сохранение')" role="alert" class="error">{{ state.error }}</p>
    </section>
  </section>
</template>

<style scoped>
.locks-view { display:grid; gap:1rem; }
.locks-confirm { max-width:min(90vw,500px); padding:1.25rem; border-radius:.7rem; border:1px solid var(--border-strong); background:var(--surface); color:var(--text); }
.locks-confirm::backdrop { background:rgba(0,0,0,.65); }
.locks-confirm-actions { display:flex; gap:.6rem; flex-wrap:wrap; }
.locks-section-head { display:flex; flex-wrap:wrap; justify-content:space-between; align-items:start; gap:.7rem; margin-bottom:1rem; }
.locks-section-head h2 { margin-bottom:.3rem; }
.locks-section-head p { margin:.2rem 0; max-width:65rem; }
.locks-chip { display:inline-block; white-space:nowrap; padding:.35rem .6rem; border:1px solid var(--border); border-radius:999px; font-size:.77rem; }
.locks-chip-ok { color:var(--text-secondary); }
.locks-chip-warn { color:var(--message); }
.locks-chip-danger,.locks-risk { color:var(--error); }
.locks-empty { padding:1rem; border:1px dashed var(--border); border-radius:.5rem; color:var(--text-secondary); }
.locks-details { margin-top:.8rem; border-top:1px solid var(--border); padding-top:.8rem; }
.locks-details summary { cursor:pointer; font-weight:600; }
.locks-overview article small { color:var(--text-secondary); font-size:.73rem; }
.locks-scroll td small { display:block; margin-top:.25rem; font-size:.75rem; color:var(--text-secondary); }
.locks-heading { display:flex; justify-content:space-between; align-items:start; gap:1rem; }
.locks-heading h1 { margin:0; }
.locks-controls { display:flex; flex-wrap:wrap; align-items:end; gap:.7rem; }
.locks-controls label { min-width:160px; flex:1; }
.locks-controls select { width:100%; padding:.6rem; border:1px solid var(--border-strong); border-radius:.5rem; background:var(--surface-2); color:var(--text); }
.locks-panel { border:1px solid var(--border); background:var(--surface); border-radius:.7rem; padding:1rem; }
.locks-panel h2 { margin-top:0; }
.locks-panel h3 { margin-bottom:.5rem; }
.locks-panel small,.muted { color:var(--text-secondary); font-size:.85rem; }
.locks-metrics { display:flex; flex-wrap:wrap; gap:.7rem; }
.locks-metrics article { display:grid; gap:.5rem; border:1px solid var(--border); border-radius:.5rem; padding:.8rem; min-width:140px; flex:1; }
.locks-metrics article span { color:var(--text-secondary); font-size:.8rem; }
.locks-metrics article strong { font-size:1.1rem; }
.locks-scroll { overflow-x:auto; }
.locks-scroll table { border-collapse:collapse; width:100%; font-size:.85rem; }
.locks-scroll th,.locks-scroll td { border-bottom:1px solid var(--border); padding:.55rem; text-align:left; }
.locks-actions { white-space:nowrap; }
.locks-actions button { padding:.35rem; margin-right:.3rem; }
.locks-wait { border-bottom:1px solid var(--border); padding:.6rem 0; }
.locks-wait div { margin-top:.35rem; }
.locks-warning { color:var(--message); }
.error { color:var(--error); }
</style>
