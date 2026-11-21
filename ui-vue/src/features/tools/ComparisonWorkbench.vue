<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { t } from "../i18n/locale-service.js";
import { computed, nextTick, onUnmounted, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";
import type { ArtifactContext } from "./artifacts.js";
import type { ToolCase } from "./controller.js";
import type { CaseComparison } from "./investigation.js";
import {
  COMPARISON_TIMEOUT_OPTIONS, comparisonRequest, comparisonSources,
  type ComparisonMode,
} from "./case-comparisons.js";
import ComparisonReport from "./ComparisonReport.vue";

const props = defineProps<{
  shell: ShellState; invoke: PgXrayInvoke; activeCase: ToolCase;
  artifacts: ArtifactContext[]; comparisons: CaseComparison[];
}>();
const emit = defineEmits<{
  (e: "changed"): void;
  (e: "artifacts-changed"): void;
  (e: "select-artifact", id: number): void;
  (e: "open-plan", id: number): void;
}>();
const mode = ref<ComparisonMode>("query_explain");
const baselineId = ref<number | null>(null);
const candidateId = ref<number | null>(null);
const timeoutMs = ref(300_000);
const confirmExecution = ref(false);
const busy = ref(false);
const error = ref("");
const notice = ref("");
const latest = ref<{ id: number; kind: ComparisonMode; report: unknown; warnings: string[] } | null>(null);
const expandedId = ref<number | null>(null);
let revision = 0;
const sources = computed(() => comparisonSources(props.artifacts, props.activeCase.id, mode.value));
watch(mode, () => {
  baselineId.value = null; candidateId.value = null; confirmExecution.value = false;
  error.value = ""; latest.value = null;
});
watch(() => [props.activeCase.id, props.shell.authGeneration] as const, () => {
  ++revision;
  baselineId.value = null; candidateId.value = null; confirmExecution.value = false;
  error.value = ""; notice.value = ""; latest.value = null;
  expandedId.value = null; busy.value = false;
});
onUnmounted(() => { ++revision; });
function current(request: number, caseId: number, auth: number): boolean {
  return revision === request && props.activeCase.id === caseId && isCurrentAuthGeneration(props.shell, auth);
}
function comparisonName(kind: string): string {
  return ({ saved_plan: t('comparison.savedPlans'), query_explain: "EXPLAIN", query_analyze: "EXPLAIN ANALYZE",
    query_result: "EXCEPT ALL", legacy_plan: t('comparison.legacyPlans'), legacy_query: t('comparison.legacySql') } as Record<string, string>)[kind] ?? kind;
}
async function run(): Promise<void> {
  if (busy.value) return;
  const caseId = props.activeCase.id;
  let command: ReturnType<typeof comparisonRequest>;
  try {
    command = comparisonRequest(props.artifacts, caseId, mode.value, baselineId.value, candidateId.value, timeoutMs.value, confirmExecution.value);
  } catch (cause) { error.value = String(cause); return; }
  const auth = props.shell.authGeneration;
  const request = ++revision;
  const selectedMode = mode.value;
  // Explicit consent is single-use; a retry must be confirmed again.
  if (selectedMode === "query_analyze" || selectedMode === "query_result") confirmExecution.value = false;
  busy.value = true; error.value = ""; notice.value = ""; latest.value = null;
  try {
    const result = await props.invoke<{ report: unknown; comparison: CaseComparison }>(command.command, command.args);
    if (!current(request, caseId, auth)) return;
    const isSqlPlan = selectedMode === "query_explain" || selectedMode === "query_analyze";
    const report = isSqlPlan ? (result.report as { comparison: unknown }).comparison : result.report;
    const warnings = isSqlPlan ? (result.report as { warnings: string[] }).warnings : [];
    latest.value = { id: result.comparison.id, kind: selectedMode, report, warnings };
    notice.value = t('comparison.savedNotice', { id: String(result.comparison.id) });
    emit("changed");
    if (selectedMode === "query_explain" || selectedMode === "query_analyze") emit("artifacts-changed");
  } catch (cause) { if (current(request, caseId, auth)) error.value = String(cause); }
  finally { if (current(request, caseId, auth)) busy.value = false; }
}
async function deleteComparison(item: CaseComparison): Promise<void> {
  if (busy.value || !props.comparisons.some(row => row.id === item.id)) return;
  if (!await confirmAction(t('comparison.deleteConfirm', { id: String(item.id) }))) return;
  const caseId = props.activeCase.id;
  const auth = props.shell.authGeneration;
  const request = ++revision;
  busy.value = true; error.value = ""; notice.value = "";
  try {
    await props.invoke("delete_case_comparison", { comparisonId: item.id });
    if (!current(request, caseId, auth)) return;
    if (expandedId.value === item.id) expandedId.value = null;
    if (latest.value?.id === item.id) latest.value = null;
    notice.value = t('comparison.deletedNotice', { id: String(item.id) });
    emit("changed");
  } catch (cause) { if (current(request, caseId, auth)) error.value = String(cause); }
  finally { if (current(request, caseId, auth)) busy.value = false; }
}
async function reuse(item: CaseComparison): Promise<void> {
  const next: ComparisonMode = (["saved_plan", "query_explain", "query_analyze", "query_result"] as string[]).includes(item.kind)
    ? item.kind as ComparisonMode : item.kind === "legacy_plan" ? "saved_plan" : "query_explain";
  mode.value = next;
  await nextTick();
  const members = item.members;
  const purpose = next === "saved_plan" ? "plan" : "query";
  const candidate = members.find(member => member.purpose === purpose && member.side === "candidate")?.artifact_id ?? null;
  const baseline = members.find(member => member.purpose === purpose && member.side === "baseline")?.artifact_id ?? null;
  const available = sources.value.map(row => row.artifact.id);
  baselineId.value = baseline !== null && available.includes(baseline) ? baseline : null;
  candidateId.value = candidate !== null && available.includes(candidate) ? candidate : null;
  confirmExecution.value = false;
  notice.value = t('comparison.filesLoaded');
  if (baselineId.value === null || candidateId.value === null) {
    error.value = t('comparison.sourceMissing');
  } else error.value = "";
}
function toggleSaved(item: CaseComparison): void {
  if (item.result === null || item.result === undefined) return;
  expandedId.value = expandedId.value === item.id ? null : item.id;
}
function openPlan(item: CaseComparison, side: "baseline" | "candidate"): void {
  const member = item.members.find(value => value.purpose === "plan" && value.side === side);
  if (member) emit("open-plan", member.artifact_id);
}
</script>

<template>
  <section class="comparison-workbench">
    <h4>{{ t('comparison.new') }}</h4>
    <p class="comparison-hint">{{ t('comparison.intro') }}</p>
    <form class="comparison-form" @submit.prevent="run">
      <label>{{ t('comparison.compareWhat') }}
        <select v-model="mode" :disabled="busy">
          <option value="saved_plan">{{ t('comparison.savedExplain') }}</option>
          <option value="query_explain">{{ t('comparison.sqlExplain') }}</option>
          <option value="query_analyze">{{ t('comparison.sqlAnalyze') }}</option>
          <option value="query_result">{{ t('comparison.sqlExcept') }}</option>
        </select>
      </label>
      <div class="comparison-pair">
        <label>Baseline
          <select v-model.number="baselineId" :disabled="busy">
            <option :value="null">{{ t('comparison.chooseFile') }}</option>
            <option v-for="row in sources" :key="row.artifact.id" :value="row.artifact.id">#{{ row.artifact.id }} · {{ row.artifact.original_name }}</option>
          </select>
        </label>
        <label>Candidate
          <select v-model.number="candidateId" :disabled="busy">
            <option :value="null">{{ t('comparison.chooseFile') }}</option>
            <option v-for="row in sources" :key="row.artifact.id" :value="row.artifact.id">#{{ row.artifact.id }} · {{ row.artifact.original_name }}</option>
          </select>
        </label>
      </div>
      <p v-if="sources.length < 2" class="comparison-hint">{{ t('comparison.needFiles') }}</p>
      <template v-if="mode !== 'saved_plan'">
        <label>{{ t('comparison.timeout') }}
          <select v-model.number="timeoutMs" :disabled="busy">
            <option v-for="value in COMPARISON_TIMEOUT_OPTIONS" :key="value" :value="value">{{ t('comparison.minute', { minutes: String(value / 60_000) }) }}</option>
          </select>
        </label>
        <p class="comparison-hint">{{ t('comparison.timeoutHint') }}</p>
      </template>
      <div v-if="mode === 'query_analyze' || mode === 'query_result'" class="comparison-caution">
        <strong>{{ t('comparison.executionWarning') }}</strong>
        <p>{{ t('comparison.sqlWarning') }}</p>
        <label class="comparison-confirm"><input v-model="confirmExecution" type="checkbox" :disabled="busy" /> {{ t('comparison.confirmSql') }}</label>
      </div>
      <p v-else class="comparison-hint">{{ mode === 'saved_plan' ? t('comparison.localPlan') : t('comparison.explainHint') }}</p>
      <button type="submit" :disabled="busy || sources.length < 2 || baselineId === null || candidateId === null || baselineId === candidateId || ((mode === 'query_analyze' || mode === 'query_result') && !confirmExecution)">
        {{ busy ? t('comparison.running') : mode === 'query_analyze' ? t('comparison.runAnalyze') : mode === 'query_result' ? t('comparison.runExcept') : t('comparison.compareAndSave') }}
      </button>
    </form>
    <p v-if="error" class="comparison-error" role="alert">{{ error }}</p>
    <p v-if="notice" role="status" class="comparison-hint">{{ notice }}</p>
    <section v-if="latest" :aria-label="t('comparison.latestAria')">
      <h4>{{ t('comparison.resultNumber', { id: String(latest.id) }) }} · {{ comparisonName(latest.kind) }}</h4>
      <ComparisonReport :report="latest.report" />
      <p v-for="(warning, index) in latest.warnings" :key="index" class="comparison-hint">{{ warning }}</p>
    </section>
    <h4>{{ t('comparison.historyTitle', { count: String(comparisons.length) }) }}</h4>
    <p class="comparison-hint">{{ t('comparison.staleExplanation') }}</p>
    <p v-if="!comparisons.length" class="comparison-hint">{{ t('comparison.noSaved') }}</p>
    <article v-for="item in comparisons" :key="item.id" class="comparison-history">
      <div class="comparison-head">
        <strong>#{{ item.id }} · {{ comparisonName(item.kind) }}</strong>
        <span class="comparison-hint">{{ new Date(item.created_at_ms).toLocaleString('ru-RU') }}</span>
      </div>
      <div v-for="member in item.members" :key="`${member.artifact_id}-${member.side}-${member.purpose}`" class="comparison-member">
        <span>{{ member.side }} / {{ member.purpose }} · {{ t('comparison.fileNumber', { id: String(member.artifact_id) }) }} <strong v-if="member.stale" class="comparison-error">STALE</strong></span>
        <div class="comparison-buttons">
          <button type="button" class="secondary" @click="emit('select-artifact', member.artifact_id)">{{ t('comparison.openFile') }}</button>
          <button v-if="member.purpose === 'plan'" type="button" class="secondary" @click="openPlan(item, member.side as 'baseline' | 'candidate')">{{ t('comparison.openPlan') }}</button>
        </div>
      </div>
      <div class="comparison-buttons">
        <button v-if="item.result !== null && item.result !== undefined" type="button" class="secondary" @click="toggleSaved(item)">{{ expandedId === item.id ? t('comparison.hideResult') : t('comparison.showResult') }}</button>
        <span v-else class="comparison-hint">{{ t('comparison.oldResultMissing') }}</span>
        <button type="button" class="secondary" :disabled="busy" @click="reuse(item)">{{ t('comparison.reuseFiles') }}</button>
        <button type="button" class="danger" :disabled="busy" @click="deleteComparison(item)">{{ t('comparison.deleteHistory') }}</button>
      </div>
      <ComparisonReport v-if="expandedId === item.id && item.result != null" :report="item.result" />
    </article>
  </section>
</template>

<style scoped>
.comparison-workbench { display:grid; gap:.75rem; min-width:0; }
.comparison-workbench h4 { margin:.5rem 0 .1rem; }
.comparison-form { padding:1rem; border:1px solid var(--border); background:var(--surface-2); border-radius:.6rem; display:grid; gap:.75rem; }
.comparison-form label:not(.comparison-confirm) { display:grid; gap:.3rem; min-width:0; }
.comparison-form select { box-sizing:border-box; min-width:0; width:100%; padding:.6rem; color:var(--text); background:var(--surface); border:1px solid var(--border-strong); border-radius:.4rem; }
.comparison-pair { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:.75rem; }
.comparison-hint { font-size:.85rem; color:var(--text-secondary); margin:.2rem 0; overflow-wrap:anywhere; }
.comparison-caution { padding:.75rem; border:1px solid var(--border-strong); border-radius:.4rem; }
.comparison-caution p { margin:.4rem 0; }
.comparison-confirm { display:flex; align-items:center; gap:.5rem; }
.comparison-error { color:var(--error); overflow-wrap:anywhere; }
.comparison-history { border-top:1px solid var(--border); padding:.75rem 0; min-width:0; }
.comparison-head { display:flex; justify-content:space-between; flex-wrap:wrap; gap:.5rem; }
.comparison-member { display:flex; align-items:center; justify-content:space-between; gap:.5rem; flex-wrap:wrap; padding:.45rem 0; }
.comparison-buttons { display:flex; flex-wrap:wrap; gap:.45rem; align-items:center; margin-top:.4rem; }
@media (max-width:680px) { .comparison-pair { grid-template-columns:1fr; } }
</style>
