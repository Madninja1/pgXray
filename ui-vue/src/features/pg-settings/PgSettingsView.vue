<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import { createPgSettingsState, createPgSettingsController } from "./controller.js";
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; quickSelection?: QuickConnectionSelection | null;}>();
const emit = defineEmits<{openHistory: [connectionId: number | null]}>();
const state = reactive(createPgSettingsState());
const controller = createPgSettingsController(props.shell, state, props.invoke, window.localStorage);
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.collecting || state.loading)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.collecting || state.loading) return;
  lastShortcutSerial = request.serial;
  // Never retarget a running collector/monitor or an in-flight database operation.
  if (state.connectionId !== request.connectionId) controller.selectConnection(request.connectionId);
});

const search = ref("");
const filter = ref("all");
const showSensitive = ref(false);
const selected = computed(() => state.connections.find(c => c.id === state.connectionId));
const changedNames = computed(() => new Set(state.comparison?.changes.map(c => c.name) ?? []));
const filtered = computed(() => (state.snapshot?.settings ?? []).filter(s => {
  if (filter.value === "changed" && !changedNames.value.has(s.name)) return false;
  if (filter.value === "non-default" && ["default", "dynamic default"].includes(s.source)) return false;
  if (filter.value === "pending" && !s.pending_restart) return false;
  return `${s.name} ${s.category} ${s.source}`.toLocaleLowerCase().includes(search.value.toLocaleLowerCase());
}));
function formatDate(ms: number): string { return new Date(ms).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US"); }
function onConnection(e: Event): void { const id = Number((e.target as HTMLSelectElement).value); controller.selectConnection(Number.isSafeInteger(id) && id > 0 ? id : null); }
function initialize(): void { if (props.shell.screen === "workspace" && !props.shell.busy && props.shell.currentUsername) void controller.refreshConnections(); }
watch(() => props.shell.authGeneration, () => { controller.reset(); initialize(); });
watch(() => props.shell.screen, screen => { if (screen !== "workspace") controller.reset(); else initialize(); });
watch(() => props.shell.busy, busy => { if (!busy && !state.connections.length) initialize(); });
watch(() => props.visible, visible => { if (visible) initialize(); });
onMounted(() => { initialize(); });
onUnmounted(() => { controller.dispose(); });
</script>
<template>
<section class="pg-config" :aria-label="t('pgConfig.aria')">
  <header><h1>Configuration Drift</h1><p>{{ t('pgConfig.intro') }}</p><button type="button" class="secondary" @click="emit('openHistory',state.connectionId)">{{ t('pgConfig.openHistory') }}</button></header>
  <div class="pg-config-controls">
    <label>{{ t('pgConfig.connection') }} <select :value="state.connectionId ?? ''" @change="onConnection"><option value="">{{ t('pgConfig.choose') }}</option><option v-for="c in state.connections" :key="c.id" :value="c.id">{{ c.name }} · {{ c.host }}:{{ c.port }}/{{ c.database }}</option></select></label>
    <button type="button" class="secondary" :disabled="state.connectionId === null || state.loading" @click="controller.refresh">{{ t('pgConfig.refresh') }}</button>
    <button type="button" :disabled="state.connectionId === null || state.collecting" @click="state.connectionId !== null && controller.collect(state.connectionId)">{{ state.collecting ? t('pgConfig.collecting') : t('pgConfig.collect') }}</button>
  </div>
  <div class="pg-config-panel">
    <p class="muted">{{ t('pgConfig.scheduleLead') }} <strong>{{ t('pgConfig.schedule') }}</strong>. {{ t('pgConfig.scheduleTail') }}</p>
  </div>
  <p class="muted">{{ t('pgConfig.scope') }}</p>
  <p class="muted">{{ t('pgConfig.sensitiveWarning') }}</p>
  <p v-if="state.error" class="error" role="alert">{{ state.error }}</p><p v-if="state.notice" role="status">{{ state.notice }}</p>
  <div class="pg-config-metrics"><article><small>{{ t('pgConfig.count') }}</small><strong>{{ state.summaries[0]?.setting_count ?? '—' }}</strong></article><article><small>{{ t('pgConfig.nondefault') }}</small><strong>{{ state.summaries[0]?.non_default_count ?? '—' }}</strong><small>{{ t('pgConfig.nondefaultHint') }}</small></article><article><small>{{ t('pgConfig.restart') }}</small><strong>{{ state.summaries[0]?.pending_restart_count ?? '—' }}</strong></article><article><small>{{ t('pgConfig.changed') }}</small><strong>{{ state.comparison ? state.comparison.changes.length : '—' }}</strong><small v-if="!state.comparison">{{ t('pgConfig.notComparable') }}</small></article></div>
  <section class="pg-config-panel"><h2>{{ t('pgConfig.last') }} <small v-if="state.summaries[0]">· {{ formatDate(state.summaries[0].collection_completed_at_ms) }}</small></h2>
    <div class="pg-config-controls"><input v-model="search" :aria-label="t('pgConfig.searchAria')" :placeholder="t('pgConfig.searchPlaceholder')" /><select v-model="filter" :aria-label="t('pgConfig.filterAria')"><option value="all">{{ t('pgConfig.all') }}</option><option value="changed">{{ t('pgConfig.changedOption') }}</option><option value="non-default">{{ t('pgConfig.configuredOption') }}</option><option value="pending">{{ t('pgConfig.restartOption') }}</option></select><label class="pg-config-toggle"><input v-model="showSensitive" type="checkbox" /> {{ t('pgConfig.showSensitive') }}</label></div>
    <p v-if="!state.snapshot" class="muted">{{ t('pgConfig.empty') }}</p>
    <div v-else class="pg-config-scroll"><table><thead><tr><th>{{ t('pgConfig.parameter') }}</th><th>{{ t('pgConfig.value') }}</th><th>{{ t('pgConfig.source') }}</th><th>{{ t('pgConfig.file') }}</th><th>Restart</th></tr></thead><tbody><tr v-for="s in filtered" :key="s.name"><td>{{ s.name }}</td><td>{{ showSensitive ? `${s.setting}${s.unit ? ` ${s.unit}` : ''}` : '••••' }}</td><td>{{ s.source }}</td><td>{{ !showSensitive ? t('pgConfig.hidden') : s.sourcefile ? `${s.sourcefile}${s.sourceline ? ':' + s.sourceline : ''}` : (s.source === 'configuration file' ? t('pgConfig.pathUnavailable') : '—') }}</td><td>{{ s.pending_restart ? t('pgConfig.yes') : t('pgConfig.no') }}</td></tr></tbody></table></div>
  </section>
  <section class="pg-config-panel"><h2>{{ t('pgConfig.changesHeading') }}</h2><p v-if="state.summaries.length >= 2 && !state.comparison" class="muted">{{ t('pgConfig.different') }}</p><p v-else-if="!state.comparison" class="muted">{{ t('pgConfig.second') }}</p><p v-else-if="state.comparison.changes.length === 0" class="muted">{{ t('pgConfig.noChanges') }}</p>
    <div v-else class="pg-config-scroll"><table><thead><tr><th>{{ t('pgConfig.parameter') }}</th><th>{{ t('pgConfig.before') }}</th><th>{{ t('pgConfig.after') }}</th><th>{{ t('pgConfig.state') }}</th></tr></thead><tbody><tr v-for="c in state.comparison.changes" :key="c.name"><td>{{ c.name }}</td><td>{{ showSensitive ? c.older_value ?? '—' : '••••' }}</td><td>{{ showSensitive ? c.newer_value ?? '—' : '••••' }}</td><td>{{ c.state }}</td></tr></tbody></table></div>
  </section>
</section>
</template>
<style scoped>
.pg-config{display:grid;gap:1rem;min-width:0}.pg-config header h1{margin:0}.pg-config header p{color:var(--muted)}.pg-config-controls{display:flex;align-items:end;gap:.65rem;flex-wrap:wrap}.pg-config-controls label{display:grid;gap:.35rem}.pg-config-controls select,.pg-config-controls input{min-width:185px}.pg-config-panel{border:1px solid var(--border-strong);border-radius:.65rem;padding:1rem;min-width:0}.pg-config-toggle{display:flex!important;align-items:center;gap:.6rem!important}.pg-config-toggle input{min-width:0}.pg-config-metrics{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:.65rem}.pg-config-metrics article{background:var(--surface-2);border-radius:.6rem;padding:.9rem;display:grid;gap:.3rem}.pg-config-metrics strong{font-size:1.5rem}.pg-config-scroll{overflow:auto;max-height:420px}.pg-config table{width:100%;border-collapse:collapse;text-align:left}.pg-config th,.pg-config td{padding:.55rem;border-bottom:1px solid var(--border-strong);vertical-align:top}.pg-config small,.pg-config .muted{color:var(--muted)}.pg-config details summary{cursor:pointer;font-weight:600}
</style>
