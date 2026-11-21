<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed, reactive, ref, shallowRef, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { KIND_LABELS, LIST_HISTORY_COMMANDS, PERIOD_MINIMUM, SNAPSHOT_KINDS, countInRange, type HistorySnapshot, type SnapshotKind } from "../history/model.js";

interface Periods { baselineFromMs: number; baselineToMs: number; incidentFromMs: number; incidentToMs: number }
const props = defineProps<{ invoke: PgXrayInvoke; connectionId: number | null; range: Periods | null; refreshToken: number; authGeneration: number; visible: boolean }>();
const emit = defineEmits<{ openHistory: []; openSchedules: []; snapshotsLoaded: [payload: { connectionId: number; rows: Partial<Record<SnapshotKind, HistorySnapshot[]>> }] }>();
interface Coverage { baseline: number; incident: number; loaded: number; error: string | null }
const coverage = reactive({} as Record<SnapshotKind, Coverage>);
for (const kind of SNAPSHOT_KINDS) coverage[kind] = { baseline: 0, incident: 0, loaded: 0, error: null };
const pending = ref(false);
const cached = shallowRef<Partial<Record<SnapshotKind, HistorySnapshot[]>>>({});
let sequence = 0;
const eligible = computed(() => props.connectionId !== null && props.range !== null);
const ready = computed(() => SNAPSHOT_KINDS.filter(kind => coverage[kind].error === null &&
  coverage[kind].baseline >= PERIOD_MINIMUM[kind] && coverage[kind].incident >= PERIOD_MINIMUM[kind]).length);
function recalculate(): void {
  const range = props.range;
  for (const kind of SNAPSHOT_KINDS) {
    const rows = cached.value[kind];
    coverage[kind] = { baseline: range && rows ? countInRange(rows, range.baselineFromMs, range.baselineToMs) : 0,
      incident: range && rows ? countInRange(rows, range.incidentFromMs, range.incidentToMs) : 0,
      loaded: rows?.length ?? 0, error: failures[kind] ?? null };
  }
}
const failures = reactive({} as Record<SnapshotKind, string | null>);
for (const kind of SNAPSHOT_KINDS) failures[kind] = null;
async function reload(): Promise<void> {
  const seq = ++sequence, id = props.connectionId, auth = props.authGeneration;
  cached.value = {};
  for (const kind of SNAPSHOT_KINDS) failures[kind] = null;
  recalculate();
  if (id === null || !props.visible) { pending.value = false; return; }
  pending.value = true;
  const rows: Partial<Record<SnapshotKind, HistorySnapshot[]>> = {};
  await Promise.all(SNAPSHOT_KINDS.map(async kind => {
    try {
      const items = await props.invoke<HistorySnapshot[]>(LIST_HISTORY_COMMANDS[kind], { connectionId: id, limit: 500 });
      if (seq !== sequence || props.authGeneration !== auth) return;
      rows[kind] = items;
    } catch (e) { if (seq === sequence && props.authGeneration === auth) failures[kind] = String(e); }
  }));
  if (seq !== sequence || props.authGeneration !== auth || props.connectionId !== id || !props.visible) return;
  cached.value = rows;
  recalculate();
  pending.value = false;
  emit("snapshotsLoaded", { connectionId: id, rows });
}
watch(() => [props.connectionId, props.refreshToken, props.authGeneration, props.visible], () => { void reload(); }, { immediate: true });
watch(() => props.range, recalculate);
function enough(kind: SnapshotKind): boolean {
  const row = coverage[kind];
  return row.error === null && row.baseline >= PERIOD_MINIMUM[kind] && row.incident >= PERIOD_MINIMUM[kind];
}
</script>

<template>
<section class="sc-coverage" :aria-label="t('coverage.aria')">
  <header><div><h2>{{ t('coverage.question') }}</h2><p>{{ t('coverage.source') }}</p></div><span v-if="pending" role="status">{{ t('coverage.checking') }}</span><strong v-else-if="eligible">{{ t('coverage.sourceCount', {ready: String(ready),total: String(SNAPSHOT_KINDS.length)}) }}</strong></header>
  <p v-if="!eligible" class="sc-muted">{{ t('coverage.select') }}</p>
  <template v-else><div class="sc-table"><table><thead><tr><th>{{ t('coverage.kind') }}</th><th>Baseline</th><th>Incident</th><th>{{ t('coverage.sufficient') }}</th></tr></thead><tbody><tr v-for="kind in SNAPSHOT_KINDS" :key="kind"><td>{{ KIND_LABELS[kind] }}</td><td>{{ coverage[kind].baseline }}</td><td>{{ coverage[kind].incident }}</td><td><span v-if="coverage[kind].error" class="sc-error" :title="coverage[kind].error">{{ t('coverage.loadError') }}</span><span v-else-if="enough(kind)" class="sc-ready">{{ t('coverage.ready') }}</span><span v-else>{{ t('coverage.needCount', {count: String(PERIOD_MINIMUM[kind])}) }}</span></td></tr></tbody></table></div>
  <p class="sc-muted">{{ t('coverage.hint') }}</p>
  <p v-if="!pending && ready === 0" class="sc-muted">{{ t('coverage.noData') }}</p>
  <div class="sc-actions"><button class="secondary" type="button" @click="emit('openHistory')">{{ t('coverage.view') }}</button><button class="secondary" type="button" @click="emit('openSchedules')">{{ t('coverage.schedule') }}</button><button class="secondary" type="button" :disabled="pending" @click="reload">{{ t('coverage.recheck') }}</button></div></template>
</section>
</template>

<style scoped>
.sc-coverage{padding:1rem;border:1px solid var(--border-strong);border-radius:.65rem;display:grid;gap:.7rem;background:var(--surface)}.sc-coverage header{display:flex;align-items:start;justify-content:space-between;gap:1rem;flex-wrap:wrap}.sc-coverage h2{font-size:1.08rem;margin:0}.sc-coverage p{margin:.25rem 0;line-height:1.5}.sc-muted{font-size:.87rem;color:var(--text-secondary)}.sc-table{overflow-x:auto}.sc-coverage table{width:100%;border-collapse:collapse;text-align:left;font-size:.86rem}.sc-coverage th,.sc-coverage td{border-bottom:1px solid var(--border);padding:.5rem}.sc-coverage td:not(:first-child){font-variant-numeric:tabular-nums}.sc-ready{color:var(--accent)}.sc-error{color:var(--error)}.sc-actions{display:flex;gap:.5rem;flex-wrap:wrap}.sc-actions button{font-size:.84rem}
</style>
