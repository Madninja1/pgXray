<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed, nextTick, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { ToolCase } from "./controller.js";
import type { ArtifactContext } from "./artifacts.js";
import ComparisonWorkbench from "./ComparisonWorkbench.vue";
import LocalDateTimeInput from "../../components/LocalDateTimeInput.vue";
import { impactExplanation, impactLabel, impactSignals, formatMetric } from "./impact-explanation.js";
import { createInvestigationController, createInvestigationState, type CaseNavigationTarget, type WorkspaceChange } from "./investigation.js";
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; activeCase: ToolCase | null; artifacts: ArtifactContext[]; visible: boolean; section: "timeline" | "interventions" | "impact" | "comparisons" | "similar" }>();
const emit = defineEmits<{ (e: "select-case", id: number): void; (e: "select-artifact", id: number): void; (e: "open-plan", id: number): void; (e: "artifacts-changed"): void; (e: "navigate-evidence", target: CaseNavigationTarget): void }>();
const state = reactive(createInvestigationState());
const actions = createInvestigationController(props.shell, state, props.invoke);
const kind = ref("deploy");
const title = ref("");
const note = ref("");
const occurredAt = ref("");
const interventionId = ref<number | null>(null);
const beforeFrom = ref(""); const beforeTo = ref(""); const afterFrom = ref(""); const afterTo = ref("");
const localizedGuide = computed(() => ({
  timeline: t('frontendPolish.guideTimeline'),
  interventions: t('frontendPolish.guideInterventions'),
  impact: t('frontendPolish.guideImpact'),
  comparisons: t('frontendPolish.guideComparisons'),
  similar: t('frontendPolish.guideSimilar'),
})[props.section]);
const interventions = computed(() => state.context?.interventions ?? []);
const impacts = computed(() => state.context?.change_impacts ?? []);
const appliedChanges = computed(() => state.context?.applied_changes ?? []);
const pendingChanges = ref<WorkspaceChange[]>([]);
const changeFeedback = ref("");
// Use a WebView-local confirmation instead of a blocked browser API. Never delete silently.
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
onUnmounted(() => finishConfirmation(false));
async function refreshAvailableChanges(): Promise<void> {
  if (!props.activeCase || !props.visible) return;
  const connectionId = props.activeCase.connection_id;
  changeFeedback.value = "";
  try {
    const changes = await props.invoke<WorkspaceChange[]>("list_workspace_changes", { connectionId, limit: 100 });
    if (props.activeCase?.connection_id === connectionId) pendingChanges.value = changes.filter(c => c.case_id === null);
  } catch (error) { changeFeedback.value = String(error); }
}
async function linkChange(id: number): Promise<void> {
  if (!props.activeCase) return;
  changeFeedback.value = "";
  try {
    await props.invoke("link_workspace_change_to_case", { changeId: id, caseId: props.activeCase.id });
    await actions.refresh();
    await refreshAvailableChanges();
    changeFeedback.value = t('investigation.changeLinked', { id: String(id) });
  } catch (error) { changeFeedback.value = String(error); }
}
const timeline = computed(() => state.context?.timeline ?? []);
const comparisons = computed(() => state.context?.comparisons ?? []);
function localDate(timestamp: number): string {
  const value = new Date(timestamp - new Date(timestamp).getTimezoneOffset() * 60_000);
  return value.toISOString().slice(0, 16);
}
function timestamp(text: string): number { return text ? new Date(text).getTime() : NaN; }
function displayDate(ms: number): string { return new Date(ms).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US"); }
watch(() => [props.activeCase?.id, props.shell.authGeneration] as const, () => {
  finishConfirmation(false);
  actions.reset(props.activeCase?.id ?? null);
  if (!occurredAt.value) occurredAt.value = localDate(Date.now());
  if (props.visible && props.activeCase) { void actions.refresh(); void refreshAvailableChanges(); }
}, { immediate: true });
watch(() => props.visible, value => { if (!value) finishConfirmation(false); if (value && props.activeCase) { void actions.refresh(); if (props.section === "interventions") void refreshAvailableChanges(); } });
watch(() => props.section, section => {
  finishConfirmation(false);
  // A validation error belongs to the form where it occurred, not every Case tab.
  state.error = "";
  state.notice = "";
  changeFeedback.value = "";
  if (props.visible && props.activeCase) {
    void actions.refresh();
    if (section === "interventions") void refreshAvailableChanges();
  }
});
watch(interventionId, id => {
  const item = interventions.value.find(value => value.id === id);
  if (!item) return;
  const center = item.occurred_at_ms;
  beforeFrom.value = localDate(center - 30 * 60_000); beforeTo.value = localDate(center);
  afterFrom.value = localDate(center); afterTo.value = localDate(center + 30 * 60_000);
});
async function create(): Promise<void> {
  if (await actions.createIntervention(timestamp(occurredAt.value), kind.value, title.value, note.value)) {
    title.value = ""; note.value = ""; occurredAt.value = localDate(Date.now());
  }
}
async function record(): Promise<void> {
  if (!props.activeCase || interventionId.value === null) return;
  await actions.recordImpact(props.activeCase, interventionId.value, [timestamp(beforeFrom.value), timestamp(beforeTo.value), timestamp(afterFrom.value), timestamp(afterTo.value)]);
}
async function removeIntervention(id: number): Promise<void> {
  const caseId = props.activeCase?.id;
  if (caseId === undefined) return;
  if (await requestConfirmation(t('investigation.deleteInterventionConfirm', { id: String(id) }))
    && props.activeCase?.id === caseId && state.context?.interventions.some(item => item.id === id))
    await actions.deleteIntervention(id);
}
async function removeImpact(id: number): Promise<void> {
  const caseId = props.activeCase?.id;
  if (caseId === undefined) return;
  if (await requestConfirmation(t('investigation.deleteImpactConfirm', { id: String(id) }))
    && props.activeCase?.id === caseId && state.context?.change_impacts.some(item => item.id === id))
    await actions.deleteImpact(id);
}
function navigate(item: CaseNavigationTarget | null): void {
  if (item) emit("navigate-evidence", item);
}
function showNavigationArtifact(item: CaseNavigationTarget | null): void {
  const id = item?.artifact_id;
  if (typeof id === "number") emit("select-artifact", id);
}
</script>
<template>
  <section class="investigation">
    <dialog ref="confirmationDialog" class="investigation-confirm" aria-labelledby="investigation-confirm-title"
      @cancel="$event.preventDefault(); finishConfirmation(false)" @close="finishConfirmation(false)">
      <h3 id="investigation-confirm-title">{{ t('investigation.confirmTitle') }}</h3>
      <p>{{ confirmationMessage }}</p>
      <div class="tools-toolbar">
        <button type="button" @click="finishConfirmation(true)">{{ t('investigation.confirm') }}</button>
        <button type="button" class="secondary" @click="finishConfirmation(false)">{{ t('investigation.cancel') }}</button>
      </div>
    </dialog>
    <header class="investigation-header">
      <div>
        <h3>{{ { timeline: t('investigation.navTimeline'), interventions: t('investigation.navInterventions'), impact: t('investigation.navImpact'), comparisons: t('investigation.navComparisons'), similar: t('investigation.navSimilar') }[section] }}</h3>
        <p class="tools-hint">{{ { timeline: t('investigation.guideTimeline'), interventions: t('investigation.guideInterventions'), impact: t('investigation.guideImpact'), comparisons: t('investigation.guideComparisons'), similar: t('investigation.guideSimilar') }[section] }}</p>
      </div>
    </header>
    <p v-if="state.error" role="alert" class="tools-error">{{ state.error }}</p>
    <p v-if="state.notice" role="status" class="tools-hint">{{ state.notice }}</p>
    <p v-if="state.loading" role="status" class="tools-hint">{{ t('investigation.loadingCase') }}</p>
    <div class="investigation-guide" :aria-label="t('investigation.guidanceAria')">
      <strong>{{ t('investigation.guideTitle') }}</strong>
      <p>{{ localizedGuide }}</p>
      <p v-if="section === 'impact'" class="tools-hint">{{ t('investigation.scenario') }}</p>
    </div>

    <template v-if="section === 'timeline'">
      <p v-if="!timeline.length" class="tools-hint">{{ t('investigation.eventsEmpty') }}</p>
      <article v-for="item in timeline" :key="item.id" class="tools-entry">
        <div class="tools-entry-head"><strong>{{ item.title }}</strong><span class="tools-hint">{{ displayDate(item.occurred_at_ms) }} · {{ item.source }} / {{ item.evidence_level }}</span></div>
        <p>{{ item.detail }}</p>
        <div class="tools-toolbar">
          <button v-if="item.navigation" type="button" class="secondary" @click="navigate(item.navigation)">{{ t('investigation.openEvidence') }} {{ item.navigation.kind }}</button>
          <button v-if="item.navigation?.artifact_id" type="button" class="secondary" @click="showNavigationArtifact(item.navigation)">{{ t('investigation.artifactRef', { id: String(item.navigation.artifact_id) }) }}</button>
          <button v-if="item.kind === 'intervention'" type="button" class="danger" :disabled="state.saving" @click="removeIntervention(Number(item.id.split(':').pop()))">{{ t('investigation.deleteIntervention') }}</button>
        </div>
      </article>
    </template>

    <template v-else-if="section === 'interventions'">
      <section class="tools-entry" :aria-label="t('investigation.policyAppliedAria')">
        <div class="tools-entry-head">
          <strong>{{ t('investigation.appliedTitle') }}</strong>
          <span class="tools-hint">{{ t('investigation.applyNotProof') }}</span>
        </div>
        <p v-if="changeFeedback" role="status">{{ changeFeedback }}</p>
        <p v-if="!appliedChanges.length" class="tools-hint">{{ t('investigation.appliedEmpty') }}</p>
        <article v-for="change in appliedChanges" :key="change.id" class="tools-entry">
          <strong>#{{ change.id }} · {{ change.kind }} · {{ change.status }}</strong>
          <p>{{ change.parameter }}: {{ change.before }} → {{ change.after }} · relation OID {{ change.relation_oid ?? '—' }}</p>
          <p class="tools-hint">Target: {{ change.target.host ?? '—' }}:{{ change.target.port ?? '—' }}/{{ change.target.database ?? '—' }} · DB OID {{ change.target.database_oid ?? '—' }}</p>
          <p class="tools-hint">{{ displayDate(change.started_at_ms) }} · {{ change.actor ?? t('investigation.unknownOperator') }} · {{ t('investigation.snapshotBefore') }} #{{ change.snapshot_id ?? '—' }} / {{ t('investigation.snapshotAfter') }} #{{ change.post_snapshot_id ?? '—' }}</p>
          <p v-if="change.note" class="tools-hint">{{ change.note }}</p>
        </article>
        <h4>{{ t('investigation.pendingTitle', { count: String(pendingChanges.length) }) }}</h4>
        <article v-for="change in pendingChanges" :key="change.id" class="tools-entry">
          <strong>#{{ change.id }} · {{ change.kind }} · {{ change.status }}</strong>
          <p>{{ change.parameter }}: {{ change.before }} → {{ change.after }} · {{ displayDate(change.started_at_ms) }}</p>
          <button type="button" class="secondary" @click="linkChange(change.id)">{{ t('investigation.linkCase') }}</button>
        </article>
      </section>
      <div class="investigation-grid">
        <form class="investigation-form" @submit.prevent="create">
          <h4>{{ t('investigation.recordChange') }}</h4>
          <LocalDateTimeInput v-model="occurredAt" :label="t('investigation.whenChanged')" required />
          <button type="button" class="secondary" @click="occurredAt = localDate(Date.now())">{{ t('investigation.setNow') }}</button>
          <label>{{ t('investigation.typeLabel') }} <select v-model="kind"><option v-for="k in ['deploy','config_change','index_create','index_drop','query_change','vacuum','maintenance','other']" :key="k">{{ k }}</option></select></label>
          <label>{{ t('investigation.titleChanged') }} <input v-model="title" required maxlength="200" /></label>
          <label>{{ t('investigation.context') }} <textarea v-model="note" maxlength="8000" rows="3" /></label>
          <button type="submit" :disabled="state.saving || !title.trim() || !occurredAt">{{ t('investigation.addChange') }}</button>
        </form>
        <div><h4>{{ t('investigation.changesTitle', { count: String(interventions.length) }) }}</h4>
          <p v-if="!interventions.length" class="tools-hint">{{ t('investigation.changesEmpty') }}</p>
          <article v-for="item in interventions" :key="item.id" class="tools-entry">
            <div class="tools-entry-head"><strong>{{ item.title }}</strong><span class="tools-hint">#{{ item.id }} · {{ displayDate(item.occurred_at_ms) }} · {{ item.kind }}</span></div>
            <p v-if="item.note">{{ item.note }}</p>
            <button type="button" class="danger" :disabled="state.saving" @click="removeIntervention(item.id)">{{ t('investigation.delete') }}</button>
          </article>
        </div>
      </div>
    </template>

    <template v-else-if="section === 'impact'">
      <p class="tools-hint">{{ t('investigation.beforeAfterCaution') }}</p>
      <div class="investigation-grid">
        <form class="investigation-form" @submit.prevent="record">
          <h4>{{ t('investigation.comparePeriods') }}</h4>
          <label>{{ t('investigation.change') }} <select v-model.number="interventionId" required><option :value="null">{{ t('investigation.chooseChange') }}</option><option v-for="item in interventions" :key="item.id" :value="item.id">#{{ item.id }} · {{ item.title }}</option></select></label>
          <LocalDateTimeInput v-model="beforeFrom" :label="t('investigation.beforeStart')" required />
          <LocalDateTimeInput v-model="beforeTo" :label="t('investigation.beforeEnd')" required />
          <LocalDateTimeInput v-model="afterFrom" :label="t('investigation.afterStart')" required />
          <LocalDateTimeInput v-model="afterTo" :label="t('investigation.afterEnd')" required />
          <p class="tools-hint">{{ t('investigation.intervalHint') }}</p>
          <button type="submit" :disabled="state.saving || interventionId === null">{{ t('investigation.compareSave') }}</button>
        </form>
        <div><h4>{{ t('investigation.resultsTitle', { count: String(impacts.length) }) }}</h4>
          <p v-if="!impacts.length" class="tools-hint">{{ t('investigation.resultsEmpty') }}</p>
          <article v-for="item in impacts" :key="item.id" class="tools-entry">
            <strong>#{{ item.id }} · {{ impactLabel(item.result) }}</strong>
            <p>{{ t('investigation.changeRange', { id: String(item.intervention_id) }) }} · {{ displayDate(item.before_from_ms) }} – {{ displayDate(item.before_to_ms) }} → {{ displayDate(item.after_from_ms) }} – {{ displayDate(item.after_to_ms) }}</p>
            <p>{{ impactExplanation(item.result) }}</p>
            <p v-if="item.result.comparison?.pgss" class="tools-hint">
              {{ t('investigation.pgssBefore') }} {{ item.result.comparison.pgss.baseline_quality.snapshot_count }} {{ t('investigation.snapshotsCount') }}, {{ item.result.comparison.pgss.baseline_quality.safe_interval_count }} {{ t('investigation.safeIntervals') }};
              {{ t('investigation.pgssAfter') }} {{ item.result.comparison.pgss.incident_quality.snapshot_count }} {{ t('investigation.snapshotsCount') }}, {{ item.result.comparison.pgss.incident_quality.safe_interval_count }} {{ t('investigation.safeIntervals') }}.
            </p>
            <p class="tools-hint">{{ t('investigation.metricSignals') }} {{ item.result.assessment?.improvement_signals ?? 0 }}, {{ t('investigation.regression') }} {{ item.result.assessment?.regression_signals ?? 0 }}. <span v-if="item.result.assessment?.workload_shift_observed">{{ t('investigation.workloadShift') }}</span></p>
            <details v-if="impactSignals(item.result).length" class="investigation-details">
              <summary>{{ t('investigation.metricChanges') }}</summary>
              <table class="investigation-metrics"><thead><tr><th>queryid</th><th>{{ t('investigation.metric') }}</th><th>{{ t('investigation.before') }}</th><th>{{ t('investigation.after') }}</th></tr></thead><tbody>
                <tr v-for="(metric, index) in impactSignals(item.result)" :key="index">
                  <td>{{ metric.queryid }}</td><td>{{ metric.label }} ({{ metric.unit }})</td>
                  <td>{{ formatMetric(metric.before) }}</td><td>{{ formatMetric(metric.after) }}</td>
                </tr>
              </tbody></table>
            </details>
            <details v-if="item.result.comparison?.pgss" class="investigation-details">
              <summary>{{ t('investigation.qualityLimits') }}</summary>
              <p class="tools-hint">{{ t('investigation.signalWarning') }}</p>
              <p class="tools-hint">{{ t('investLast.before') }}{{ item.result.comparison.pgss.baseline_quality.state }} · {{ t('investigation.pgssAfter') }} {{ item.result.comparison.pgss.incident_quality.state }}.</p>
              <ul><li v-for="(reason, index) in [...item.result.comparison.pgss.baseline_quality.limitations, ...item.result.comparison.pgss.incident_quality.limitations]" :key="index">{{ reason }}</li></ul>
            </details>
            <p class="tools-hint">{{ t('investigation.causalityWarning') }}</p>
            <button type="button" class="danger" :disabled="state.saving" @click="removeImpact(item.id)">{{ t('investigation.deleteResult') }}</button>
          </article>
        </div>
      </div>
    </template>

    <template v-else-if="section === 'comparisons'">
      <ComparisonWorkbench v-if="activeCase" :shell="shell" :invoke="invoke" :active-case="activeCase"
        :artifacts="artifacts" :comparisons="comparisons"
        @changed="actions.refresh()"
        @artifacts-changed="emit('artifacts-changed')"
        @select-artifact="emit('select-artifact', $event)" @open-plan="emit('open-plan', $event)" />
    </template>

    <template v-else>
      <h4>{{ t('investigation.fingerprint') }}</h4>
      <pre class="tools-fingerprint">{{ JSON.stringify(state.context?.fingerprint ?? {}, null, 2) }}</pre>
      <h4>{{ t('investigation.similarTitle', { count: String(state.context?.similar_incidents.length ?? 0) }) }}</h4>
      <p class="tools-hint">{{ t('investigation.similarWarning', { scanned: String(state.context?.similar_search_scanned ?? 0), total: String(state.context?.similar_search_total_candidates ?? 0) }) }}</p>
      <p v-if="!state.context?.similar_incidents.length" class="tools-hint">{{ t('investigation.similarEmpty') }}</p>
      <article v-for="item in (state.context?.similar_incidents ?? [])" :key="item.case_id" class="tools-entry">
        <strong>#{{ item.case_id }} · {{ item.title }} · {{ t('investigation.similarScore') }} {{ item.score }}</strong>
        <p>{{ item.matches.join('; ') }}</p>
        <button type="button" class="secondary" @click="$emit('select-case', item.case_id)">{{ t('investigation.openCase') }}</button>
      </article>
    </template>
  </section>
</template>
<style scoped>
.investigation { display:grid; gap:.75rem; min-width:0; }
.investigation-confirm { max-width:min(90vw,480px); padding:1.25rem; border-radius:.7rem; border:1px solid var(--border-strong); background:var(--surface); color:var(--text); }
.investigation-confirm::backdrop { background:rgba(0,0,0,.65); }
.investigation-header { display:flex; justify-content:space-between; align-items:center; gap:1rem; flex-wrap:wrap; }
.investigation-header h3 { margin:0; }
.investigation-header p { margin:.3rem 0 0; }
.tools-hint { color:var(--text-secondary); font-size:.9rem; overflow-wrap:anywhere; }
.investigation-guide { padding:.8rem 1rem; border:1px solid var(--border); border-radius:.6rem; background:var(--surface-2); }
.investigation-guide p { margin:.3rem 0 0; }
.investigation-details { margin:.6rem 0; }
.investigation-details summary { cursor:pointer; }
.investigation-metrics { width:100%; font-size:.86rem; border-collapse:collapse; }
.investigation-metrics th,.investigation-metrics td { padding:.4rem; text-align:left; border-bottom:1px solid var(--border); overflow-wrap:anywhere; }
.tools-error { color:var(--error); }
.tools-toolbar { display:flex; gap:.5rem; flex-wrap:wrap; }
.investigation-grid { display:grid; grid-template-columns:minmax(260px, 2fr) minmax(0, 3fr); gap:1rem; }
.investigation-grid h4, .investigation-form h4 { margin:0 0 .5rem; }
.investigation-form { display:grid; gap:.65rem; align-content:start; background:var(--surface-2); border:1px solid var(--border); padding:1rem; border-radius:.6rem; }
.investigation-form label { display:grid; gap:.35rem; min-width:0; }
.investigation-form :is(input, select, textarea) { box-sizing:border-box; width:100%; min-width:0; padding:.6rem; background:var(--surface); color:var(--text); border:1px solid var(--border-strong); border-radius:.4rem; }
.tools-entry { padding:1rem 0; border-bottom:1px solid var(--border); overflow-wrap:anywhere; }
.tools-entry-head { display:grid; gap:.3rem; }
.tools-entry p { margin:.5rem 0; }
.tools-member { display:flex; justify-content:space-between; align-items:center; gap:.5rem; border-top:1px solid var(--border); padding:.6rem 0; flex-wrap:wrap; }
.tools-stale { color:var(--error); }
.tools-fingerprint { overflow:auto; max-height:300px; background:var(--surface-2); border:1px solid var(--border); padding:1rem; border-radius:.5rem; }
@media (max-width:900px) { .investigation-grid { grid-template-columns:1fr; } }
</style>
