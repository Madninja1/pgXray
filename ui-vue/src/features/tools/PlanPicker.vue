<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { t } from "../i18n/locale-service.js";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";
import type { ToolCase } from "./controller.js";
import type { ArtifactContext } from "./artifacts.js";

type SourceMode = "paste" | "file" | "case";
type ResultSection = "overview" | "tree" | "findings" | "json" | "compare" | "reference" | "prompt" | "pev2";
interface PlanNode {
  id: number; path: string; node_type: string; relation_name: string | null;
  index_name: string | null; total_cost: number | null; actual_total_time_ms: number | null;
  actual_rows: number | null; actual_loops: number | null; children: PlanNode[];
}
interface PlanFinding { severity: "high" | "warning" | "info"; title: string; summary: string; code: string; node_id: number | null; reference_key?: string | null; evidence: string[]; next_checks: string[] }
interface PlanInspection {
  document: { root: PlanNode; node_count: number; execution_time_ms: number | null; planning_time_ms: number | null; jit_total_time_ms?: number | null; source_has_analyze: boolean };
  analysis: { summary: { high_count: number; warning_count: number; info_count: number; root_shared_read_blocks: number | null }; findings: PlanFinding[]; hotspots: {node_id: number; label: string; repeated_time_ms: number | null; shared_read_blocks: number | null}[] };
}
interface MetricDelta {
  key: string; label: string; unit: string; baseline: number | null;
  candidate: number | null; delta: number | null; delta_pct: number | null;
}
interface PlanComparison {
  metrics: MetricDelta[]; summary: string[]; structure_changed: boolean;
  node_types: { node_type: string; baseline_count: number; candidate_count: number }[];
}
interface ReferenceEntry {
  key: string; title: string; category: string; short: string; how_it_works: string;
  good_when: string[]; suspicious_when: string[]; inspect: string[];
  misconceptions: string[]; docs_url: string; related: string[]; terms: string[];
}
interface PlanPrefill { caseId: number; artifactId: number; name: string }
const emit = defineEmits<{ openPrivacy: [value: { input: string; scopeId: number | null; fileName: string }] }>();
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; cases: ToolCase[]; visible: boolean; prefill: PlanPrefill | null }>();
const mode = ref<SourceMode>("paste");
const caseId = ref<number | null>(null);
const artifactId = ref<number | null>(null);
const candidates = ref<ArtifactContext[]>([]);
const artifacts = computed(() => candidates.value.filter(item => item.artifact.visibility === "sensitive" &&
  (item.artifact.kind === "explain" || item.artifact.original_name.toLowerCase().endsWith(".json"))));
const input = ref(""); const sourceName = ref("");
const busy = ref(false); const loadingArtifacts = ref(false);
const error = ref(""); const inspection = ref<PlanInspection | null>(null);
const section = ref<ResultSection>("overview"); const showInput = ref(true);
const expandedNodes = ref<Set<number>>(new Set());
const selectedNodeId = ref<number | null>(null);
const selectedNode = computed(() => {
  function find(node: PlanNode): PlanNode | null {
    if (node.id === selectedNodeId.value) return node;
    for (const child of node.children) { const match = find(child); if (match) return match; }
    return null;
  }
  return inspection.value && selectedNodeId.value !== null ? find(inspection.value.document.root) : null;
});
const selectedNodeFindings = computed(() => inspection.value?.analysis.findings.filter(item => item.node_id === selectedNodeId.value) ?? []);
function inspectNode(id: number): void {
  const root = inspection.value?.document.root;
  if (!root) return;
  const ancestors: number[] = [];
  function findPath(node: PlanNode, path: number[]): boolean {
    if (node.id === id) { ancestors.push(...path); return true; }
    for (const child of node.children) if (findPath(child, [...path, node.id])) return true;
    return false;
  }
  if (!findPath(root, [])) return;
  expandedNodes.value = new Set([...expandedNodes.value, ...ancestors]);
  selectedNodeId.value = id;
  setSection("tree");
}
const showAllHotspots = ref(false);
const candidateInput = ref("");
const comparison = ref<PlanComparison | null>(null);
let comparisonRequest = 0;
const compareBusy = ref(false); const compareError = ref("");
const referenceEntries = ref<ReferenceEntry[]>([]); const referenceBusy = ref(false);
const referenceError = ref(""); const referenceFilter = ref("");
const selectedReference = ref<string | null>(null);
const referenceHistory = ref<string[]>([]);
const referenceLinks = computed(() => {
  const article = activeReference.value;
  if (!article) return [];
  const keys = new Set([...(article.terms ?? []), ...(article.related ?? [])]);
  return [...keys].map(key => referenceEntries.value.find(item => item.key === key)).filter((item): item is ReferenceEntry => item !== undefined && item.key !== article.key);
});
function navigateReference(key: string): void {
  if (!referenceEntries.value.some(item => item.key === key)) return;
  if (selectedReference.value && selectedReference.value !== key) referenceHistory.value = [...referenceHistory.value, selectedReference.value];
  selectedReference.value = key;
}
function backReference(): void {
  const history = [...referenceHistory.value];
  const key = history.pop();
  if (!key) return;
  referenceHistory.value = history;
  selectedReference.value = key;
}
const prompt = ref<string | null>(null); const promptBusy = ref(false); const promptError = ref("");
// The bundled PEV2 is an independent Vue application hosted in a sandboxed frame.
const pevFrame = ref<HTMLIFrameElement | null>(null);
const pevReady = ref(false);
const pevStatus = ref(t('plans.connecting'));
const pevError = ref("");
const pevExpanded = ref(false);
const showPev2 = computed(() => props.visible && section.value === "pev2" && inspection.value !== null);
function sendPev2Plan(): void {
  if (!showPev2.value || !pevReady.value || !pevFrame.value?.contentWindow) return;
  // postMessage is used only to the iframe created by our own bundled PEV2 page.
  pevFrame.value.contentWindow.postMessage({ type: "pgxray-pev2-plan", plan: input.value, query: "" }, "*");
}
function onPev2Message(event: MessageEvent): void {
  if (event.source !== pevFrame.value?.contentWindow) return;
  if (event.data?.type === "pgxray-pev2-error") {
    pevError.value = String(event.data.error ?? t('plans.pevError'));
    pevStatus.value = "";
  } else if (event.data?.type === "pgxray-pev2-ready") {
    pevReady.value = true;
    pevError.value = "";
    pevStatus.value = t('plans.loadingPlan');
    sendPev2Plan();
  } else if (event.data?.type === "pgxray-pev2-mounted") {
    pevError.value = "";
    pevStatus.value = "";
  }
}
onMounted(() => window.addEventListener("message", onPev2Message));
onUnmounted(() => window.removeEventListener("message", onPev2Message));
watch(showPev2, () => { pevReady.value = false; pevError.value = ""; pevStatus.value = t('plans.connecting'); });
watch(input, () => { if (pevReady.value) sendPev2Plan(); });

const activeReference = computed(() => referenceEntries.value.find(item => item.key === selectedReference.value) ?? null);
const referenceCategory = ref("all");
const referenceCategories = computed(() => [...new Set(referenceEntries.value.map(item => item.category))].sort());
const filteredReferences = computed(() => referenceEntries.value.filter(item =>
  (referenceCategory.value === "all" || item.category === referenceCategory.value) &&
  `${item.title} ${item.category} ${item.short}`.toLocaleLowerCase().includes(referenceFilter.value.trim().toLocaleLowerCase())));
const clipboardFeedback = ref("");
function openPlanInPrivacy(): void {
  if (!input.value.trim()) return;
  const scopeId = mode.value === "case" ? (props.cases.find(item => item.id === caseId.value)?.scope_id ?? null) : null;
  emit("openPrivacy", { input: input.value, scopeId, fileName: sourceName.value || "EXPLAIN JSON" });
}
async function copyText(value: string, label: string): Promise<void> {
  clipboardFeedback.value = "";
  try {
    await navigator.clipboard.writeText(value);
    clipboardFeedback.value = t('plans.copied', { label });
  } catch (cause) { clipboardFeedback.value = t('plans.copyError', { error: String(cause) }); }
}
function openFindingReference(key: string | null | undefined): void {
  if (!key) return;
  if (referenceEntries.value.length) navigateReference(key);
  else selectedReference.value = key;
  setSection("reference");
}
async function copyPromptWithPlan(): Promise<void> {
  if (prompt.value === null || !input.value.trim()) return;
  if (!await confirmAction(t('plans.unsafeCopy'))) return;
  await copyText(`${prompt.value}\n\n## EXPLAIN JSON\n\n\`\`\`json\n${input.value}\n\`\`\``, t('plans.detailsCopiedLabel'));
}

let revision = 0;
const valid = (id: number, auth: number): boolean => id === revision && props.visible && isCurrentAuthGeneration(props.shell, auth);
function invalidate(): number { return ++revision; }
function chooseMode(next: SourceMode): void {
  invalidate(); mode.value = next; inspection.value = null; error.value = "";
  input.value = ""; sourceName.value = ""; showInput.value = true;
}
watch(input, () => { inspection.value = null; showInput.value = true; });
watch(() => props.shell.authGeneration, () => {
  invalidate(); candidates.value = []; caseId.value = null; artifactId.value = null;
  input.value = ""; inspection.value = null; error.value = ""; sourceName.value = "";
});
watch(() => props.visible, visible => { if (!visible) invalidate(); });
let consumedPrefill: PlanPrefill | null = null;
watch(() => [props.prefill, props.visible] as const, async ([prefill, visible]) => {
  if (!prefill || !visible || prefill === consumedPrefill) return;
  consumedPrefill = prefill;
  chooseMode("case"); caseId.value = prefill.caseId; artifactId.value = prefill.artifactId;
  await nextTick(); // Даём обработчику смены Case сбросить прежние запросы.
  await loadArtifact(prefill.artifactId, prefill.name);
}, { immediate: true });
watch(caseId, async id => {
  const request = invalidate(), auth = props.shell.authGeneration;
  candidates.value = []; artifactId.value = null; error.value = "";
  input.value = ""; inspection.value = null;
  if (id === null || !props.visible) return;
  loadingArtifacts.value = true;
  try {
    const context = await props.invoke<{ artifacts: ArtifactContext[] }>("case_context", { caseId: id });
    if (valid(request, auth) && caseId.value === id) candidates.value = context.artifacts;
  } catch (cause) { if (valid(request, auth)) error.value = String(cause); }
  finally { if (request === revision) loadingArtifacts.value = false; }
});
async function loadArtifact(id: number, name: string): Promise<void> {
  const request = invalidate(), auth = props.shell.authGeneration;
  busy.value = true; error.value = ""; inspection.value = null; input.value = "";
  try {
    const content = await props.invoke<string>("read_case_artifact", { artifactId: id });
    if (!valid(request, auth)) return;
    input.value = content; sourceName.value = name; await analyze();
  } catch (cause) { if (valid(request, auth)) error.value = String(cause); }
  finally { if (request === revision) busy.value = false; }
}
async function selectArtifact(): Promise<void> {
  const item = artifacts.value.find(row => row.artifact.id === artifactId.value);
  if (item) await loadArtifact(item.artifact.id, item.artifact.original_name);
}
async function loadLocalFile(event: Event): Promise<void> {
  const field = event.target as HTMLInputElement;
  const file = field.files?.[0]; field.value = "";
  if (!file) return;
  const request = invalidate(), auth = props.shell.authGeneration;
  input.value = ""; inspection.value = null; error.value = "";
  if (file.size > 32 * 1024 * 1024) { error.value = t('plans.fileLimit'); return; }
  busy.value = true;
  try {
    const content = await file.text();
    if (!valid(request, auth)) return;
    input.value = content; sourceName.value = file.name; await analyze();
  } catch (cause) { if (valid(request, auth)) error.value = String(cause); }
  finally { if (request === revision) busy.value = false; }
}
async function analyze(): Promise<void> {
  if (!input.value.trim()) { error.value = t('plans.pasteJson'); return; }
  if (new Blob([input.value]).size > 32 * 1024 * 1024) { error.value = t('plans.jsonLimit'); return; }
  const request = invalidate(), auth = props.shell.authGeneration;
  busy.value = true; error.value = ""; inspection.value = null;
  try {
    const result = await props.invoke<PlanInspection>("inspect_plan_json", { input: input.value });
    if (!valid(request, auth)) return;
    inspection.value = result; section.value = "overview"; showInput.value = false;
    expandedNodes.value = new Set([result.document.root.id]); selectedNodeId.value = null; showAllHotspots.value = false;
  } catch (cause) { if (valid(request, auth)) error.value = String(cause); }
  finally { if (request === revision) busy.value = false; }
}
async function clear(): Promise<void> {
  if ((input.value.trim() || inspection.value) && !await confirmAction(t('plans.resetConfirm'))) return;
  invalidate(); input.value = ""; sourceName.value = ""; inspection.value = null;
  comparison.value = null; candidateInput.value = ""; error.value = ""; showInput.value = true;
  section.value = "overview"; expandedNodes.value = new Set(); selectedNodeId.value = null;
}
watch(input, () => { ++comparisonRequest; comparison.value = null; compareError.value = ""; });
watch(candidateInput, () => { ++comparisonRequest; comparison.value = null; compareError.value = ""; });
watch(() => props.shell.authGeneration, () => {
  comparison.value = null; candidateInput.value = ""; compareError.value = "";
  referenceEntries.value = []; selectedReference.value = null; referenceHistory.value = []; referenceError.value = "";
  prompt.value = null; promptError.value = "";
});
async function compare(): Promise<void> {
  if (!input.value.trim() || !candidateInput.value.trim()) {
    compareError.value = t('plans.compareSourceError'); return;
  }
  if (new Blob([input.value]).size > 32*1024*1024 || new Blob([candidateInput.value]).size > 32*1024*1024) {
    compareError.value = t('plans.compareLimit'); return;
  }
  const auth = props.shell.authGeneration, request = revision, comparisonId = ++comparisonRequest;
  compareBusy.value = true; compareError.value = ""; comparison.value = null;
  try {
    const result = await props.invoke<PlanComparison>("compare_plan_json", {baseline: input.value, candidate: candidateInput.value});
    if (valid(request, auth) && comparisonId === comparisonRequest) comparison.value = result;
  } catch (cause) { if (valid(request, auth) && comparisonId === comparisonRequest) compareError.value = String(cause); }
  finally { compareBusy.value = false; }
}
async function loadReference(): Promise<void> {
  if (referenceEntries.value.length || referenceBusy.value) return;
  const auth = props.shell.authGeneration;
  referenceBusy.value = true; referenceError.value = "";
  try {
    const items = await props.invoke<ReferenceEntry[]>("plan_reference");
    if (props.visible && isCurrentAuthGeneration(props.shell, auth)) {
      referenceEntries.value = items;
      selectedReference.value = selectedReference.value ?? items[0]?.key ?? null;
    }
  } catch (cause) { if (isCurrentAuthGeneration(props.shell, auth)) referenceError.value = String(cause); }
  finally { referenceBusy.value = false; }
}
async function loadPrompt(): Promise<void> {
  if (prompt.value !== null || promptBusy.value) return;
  const auth = props.shell.authGeneration;
  promptBusy.value = true; promptError.value = "";
  try {
    const content = await props.invoke<string>("get_explain_analysis_prompt");
    if (props.visible && isCurrentAuthGeneration(props.shell, auth)) prompt.value = content;
  } catch (cause) { if (isCurrentAuthGeneration(props.shell, auth)) promptError.value = String(cause); }
  finally { promptBusy.value = false; }
}
function setSection(next: ResultSection): void {
  section.value = next;
  if (next === "reference") void loadReference();
  if (next === "prompt") void loadPrompt();
}
const visibleNodes = computed(() => {
  const result: { node: PlanNode; depth: number }[] = [];
  function walk(node: PlanNode, depth: number): void {
    result.push({ node, depth });
    if (expandedNodes.value.has(node.id)) for (const child of node.children) walk(child, depth + 1);
  }
  if (inspection.value) walk(inspection.value.document.root, 0);
  return result;
});
function toggleNode(id: number): void {
  const next = new Set(expandedNodes.value);
  if (next.has(id)) next.delete(id); else next.add(id);
  expandedNodes.value = next;
}
function expandAllNodes(): void {
  const nodes = new Set<number>();
  function walk(node: PlanNode): void { if (node.children.length) nodes.add(node.id); for (const child of node.children) walk(child); }
  if (inspection.value) walk(inspection.value.document.root);
  expandedNodes.value = nodes;
}
const findingGroups = computed(() => {
  const groups = new Map<string, { code: string; title: string; severity: string; items: PlanFinding[] }>();
  for (const item of inspection.value?.analysis.findings ?? []) {
    let group = groups.get(item.code);
    if (!group) { group = {code: item.code, title: item.title, severity: item.severity, items: []}; groups.set(item.code, group); }
    group.items.push(item);
  }
  return [...groups.values()];
});
const jitShare = computed(() => {
  const document = inspection.value?.document;
  const jit = document?.jit_total_time_ms;
  const execution = document?.execution_time_ms;
  return jit != null && execution != null && execution > 0 ? (jit / execution * 100) : null;
});
function metric(value: number | null | undefined, suffix = ""): string {
  return value === null || value === undefined ? "—" : `${value.toLocaleString("ru-RU", { maximumFractionDigits: 2 })}${suffix}`;
}
</script>

<template>
  <section class="plans-root">
    <header class="plans-header">
      <div><h2>{{ t('plans.pageTitle') }}</h2><p class="plans-muted">{{ t('plans.intro') }}</p></div>
      <span class="plans-badge">{{ t('plans.localAnalysis') }}</span>
    </header>
    <nav class="plans-modes" :aria-label="t('plans.sourceAria')">
      <button v-for="item in ([{id:'paste', label:t('plans.pasteInput')}, {id:'file', label:t('plans.openFile')}, {id:'case', label:t('plans.fromCase')}] as const)"
        :key="item.id" type="button" class="secondary" :aria-pressed="mode === item.id" @click="chooseMode(item.id)">{{ item.label }}</button>
    </nav>
    <section class="plans-input-card">
      <div class="plans-input-head"><h3>{{ t('plans.sourceTitle') }} <span v-if="sourceName" class="plans-muted">· {{ sourceName }}</span></h3>
        <button v-if="inspection" type="button" class="secondary" @click="showInput = !showInput">{{ showInput ? t('plans.hideJson') : t('plans.showJson') }}</button>
      </div>
      <div v-if="mode === 'case'" class="plans-pickers">
        <label>{{ t('plans.caseLabel') }}<select v-model.number="caseId" :disabled="loadingArtifacts || busy"><option :value="null">{{ t('plans.chooseCase') }}</option><option v-for="item in cases" :key="item.id" :value="item.id">#{{ item.id }} · {{ item.title }}</option></select></label>
        <label>{{ t('plans.explainArtifact') }}<select v-model.number="artifactId" :disabled="loadingArtifacts || busy"><option :value="null">{{ t('plans.chooseJson') }}</option><option v-for="item in artifacts" :key="item.artifact.id" :value="item.artifact.id">{{ item.artifact.original_name }}</option></select></label>
        <button type="button" class="secondary" :disabled="!artifactId || busy || loadingArtifacts" @click="selectArtifact">{{ t('plans.loadPlan') }}</button>
      </div>
      <div v-if="mode === 'file'" class="plans-file-load"><label>{{ t('plans.fileExplain') }} <input type="file" accept=".json,application/json,.txt" :disabled="busy" @change="loadLocalFile" /></label><p class="plans-muted">{{ t('plans.sourceHint') }}</p></div>
      <label v-if="showInput || !inspection" class="plans-json-label">EXPLAIN JSON
        <textarea v-model="input" :readonly="busy" spellcheck="false" class="plans-json" rows="11" placeholder='[{"Plan":{"Node Type":"Seq Scan","Relation Name":"orders","Plan Rows":10}}]' />
      </label>
      <div class="plans-toolbar">
        <button type="button" class="secondary" :disabled="busy || (!input && !inspection)" @click="clear">{{ t('plans.resetAnalysis') }}</button>
        <button type="button" :disabled="busy || !input.trim()" @click="analyze">{{ busy ? t('plans.analysis') : t('plans.analyze') }}</button>
      </div>
      <p v-if="error" role="alert" class="plans-error">{{ error }}</p>
    </section>
    <section v-if="inspection" class="plans-results" :aria-label="t('plans.resultAria')">
      <div class="plans-stats">
        <article><small>{{ t('plans.rootNode') }}</small><strong>{{ inspection.document.root.node_type }}</strong></article>
        <article><small>{{ t('plans.nodeCount') }}</small><strong>{{ inspection.document.node_count }}</strong></article>
        <article><small>{{ t('plans.executionTime') }}</small><strong>{{ metric(inspection.document.execution_time_ms, t('planExtra.ms')) }}</strong></article>
        <article><small>{{ t('plans.warnings') }}</small><strong>{{ t("planExtra.highCount", { high: String(inspection.analysis.summary.high_count), medium: String(inspection.analysis.summary.warning_count) }) }}</strong></article>
      </div>
      <p v-if="!inspection.document.source_has_analyze" class="plans-muted">{{ t('plans.noAnalyze') }}</p>
      <nav class="plans-modes" :aria-label="t('plans.analysisResultsAria')">
        <button v-for="item in ([{id:'overview',label:t('plans.overview')}, {id:'tree',label:t('plans.tree')}, {id:'findings',label:t('plans.findings')}, {id:'compare',label:t('plans.comparison')}, {id:'pev2',label:'PEV2'}, {id:'reference',label:t('plans.reference')}, {id:'prompt',label:t('plans.prompt')}, {id:'json',label:'JSON'}] as const)"
          :key="item.id" type="button" class="secondary" :aria-pressed="section===item.id" @click="setSection(item.id)">{{ item.label }}</button>
      </nav>
      <div v-if="section === 'overview'" class="plans-panel">
        <h3>{{ t('plans.keyMetrics') }}</h3>
        <p>{{ t("planExtra.planning") }} {{ metric(inspection.document.planning_time_ms, t("planExtra.ms")) }} · Shared Read Blocks: {{ metric(inspection.analysis.summary.root_shared_read_blocks) }}</p>
        <p v-if="jitShare !== null" class="plans-jit-note"><strong>JIT:</strong> {{ metric(inspection.document.jit_total_time_ms, t('planExtra.ms')) }} ({{ metric(jitShare, '%') }} {{ t("planExtra.jitHint") }}</p>
        <div class="plans-input-head"><h3>{{ t('plans.hotspots') }}</h3><button v-if="inspection.analysis.hotspots.length > 5" type="button" class="secondary" @click="showAllHotspots = !showAllHotspots">{{ showAllHotspots ? t('plans.firstFive') : t('plans.showAll') }}</button></div>
        <p class="plans-muted">{{ t('plans.repeatWork') }}</p>
        <p v-if="!inspection.analysis.hotspots.length" class="plans-muted">{{ t('plans.noActualCost') }}</p>
        <div v-for="spot in (showAllHotspots ? inspection.analysis.hotspots : inspection.analysis.hotspots.slice(0,5))" :key="spot.node_id" class="plans-spot"><button type="button" class="secondary plans-node-link" @click="inspectNode(spot.node_id)">{{ spot.label }} · {{ t("planExtra.openNode") }}</button><span>{{ metric(spot.repeated_time_ms, t('planExtra.repeatedMs')) }} · Read {{ metric(spot.shared_read_blocks) }}</span></div>
      </div>
      <div v-else-if="section === 'tree'" class="plans-panel">
        <div class="plans-input-head"><h3>{{ t('plans.executionTree') }}</h3><div class="plans-modes"><button type="button" class="secondary" @click="expandedNodes = new Set([inspection.document.root.id])">{{ t('plans.collapseBranches') }}</button><button type="button" class="secondary" @click="expandAllNodes">{{ t('plans.expandAll') }}</button></div></div>
        <p class="plans-muted">{{ t('plans.treeHint') }}</p>
        <div v-for="item in visibleNodes" :key="item.node.id" class="plans-node" :class="{'plans-node-selected': item.node.id === selectedNodeId}" :style="{paddingLeft:`${Math.min(item.depth,15)*16+8}px`}">
          <button v-if="item.node.children.length" type="button" class="secondary plans-tree-toggle" :aria-expanded="expandedNodes.has(item.node.id)" :aria-label="`${expandedNodes.has(item.node.id) ? t('plans.collapse') : t('plans.expand')} ${item.node.node_type}`" @click="toggleNode(item.node.id)">{{ expandedNodes.has(item.node.id) ? '▾' : '▸' }}</button>
          <span v-else class="plans-tree-leaf">·</span>
          <button type="button" class="plans-node-name" :aria-pressed="selectedNodeId === item.node.id" @click="selectedNodeId = item.node.id">#{{ item.node.id }} {{ item.node.node_type }}</button><span v-if="item.node.relation_name"> · {{ item.node.relation_name }}</span>
          <small>Cost {{ metric(item.node.total_cost) }} · Rows {{ metric(item.node.actual_rows ?? null) }} · Loops {{ metric(item.node.actual_loops) }} <strong v-if="item.node.actual_loops === 0"> · {{ t('plans.noExecution') }}</strong></small>
        </div>
        <aside v-if="selectedNode" class="plans-selected-node">
          <div class="plans-input-head"><h4>{{ t('plans.treeNode', { id: String(selectedNode.id) }) }} · {{ selectedNode.node_type }}</h4><button type="button" class="secondary" @click="selectedNodeId = null">{{ t('plans.close') }}</button></div>
          <p v-if="selectedNode.relation_name">{{ t('plans.tableLabel') }} {{ selectedNode.relation_name }} <span v-if="selectedNode.index_name"> · {{ t('plans.indexLabel') }} {{ selectedNode.index_name }}</span></p>
          <p>Actual Total Time: {{ metric(selectedNode.actual_total_time_ms, t('planExtra.msLoop')) }} · Actual Rows: {{ metric(selectedNode.actual_rows) }} · Loops: {{ metric(selectedNode.actual_loops) }} · Cost: {{ metric(selectedNode.total_cost) }}</p>
          <div v-if="selectedNodeFindings.length"><strong>{{ t('plans.nodeFindings') }}</strong><p v-for="finding in selectedNodeFindings" :key="finding.code">{{ finding.severity.toUpperCase() }} · {{ finding.title }}</p></div>
          <p v-else class="plans-muted">{{ t('plans.noNodeFindings') }}</p>
        </aside>
      </div>
      <div v-else-if="section === 'findings'" class="plans-panel"><h3>{{ t('plans.diagnostics') }}</h3>
        <p v-if="!findingGroups.length" class="plans-muted">{{ t('plans.estimateWarning') }}</p>
        <p v-else class="plans-muted">{{ t('plans.numberOfFindings', { findings: String(inspection.analysis.findings.length), groups: String(findingGroups.length) }) }}</p>
        <details v-for="group in findingGroups" :key="group.code" class="plans-finding plans-finding-group">
          <summary><strong>{{ group.severity.toUpperCase() }} · {{ group.title }}</strong> <span class="plans-muted">{{ group.items.length }} {{ t('plans.nodes') }}</span></summary>
          <article v-for="finding in group.items" :key="`${finding.code}-${finding.node_id}`" class="plans-finding-entry">
            <button v-if="finding.node_id != null" type="button" class="secondary plans-node-link" @click="inspectNode(finding.node_id)">{{ t('plans.nodeWithId', { id: String(finding.node_id) }) }}</button><p>{{ finding.summary }}</p>
            <button v-if="finding.reference_key" type="button" class="secondary plans-reference-link" @click="openFindingReference(finding.reference_key)">{{ t('plans.openReference') }}</button>
            <details v-if="finding.evidence.length || finding.next_checks.length"><summary>{{ t('plans.evidenceAndChecks') }}</summary><p v-for="line in finding.evidence" :key="line">{{ line }}</p><p v-for="line in finding.next_checks" :key="line">{{ t('plans.nextCheck') }} {{ line }}</p></details>
          </article>
        </details>
      </div>
      <div v-else-if="section === 'compare'" class="plans-panel">
        <h3>{{ t('plans.planCompare') }}</h3>
        <p class="plans-muted">{{ t('plans.comparisonHint') }}</p>
        <label class="plans-json-label">Candidate JSON
          <textarea v-model="candidateInput" class="plans-json" rows="9" spellcheck="false" placeholder='[{"Plan":{"Node Type":"Index Scan"}}]' />
        </label>
        <div class="plans-toolbar"><button type="button" :disabled="compareBusy || !input.trim() || !candidateInput.trim()" @click="compare">{{ compareBusy ? t('plans.comparing') : t('plans.compare') }}</button></div>
        <p v-if="compareError" class="plans-error" role="alert">{{ compareError }}</p>
        <template v-if="comparison">
          <p class="plans-muted">{{ comparison.structure_changed ? t('plans.structureChanged') : t('plans.sameStructure') }}. {{ t('plans.comparisonNotProof') }}</p>
          <p v-for="line in comparison.summary" :key="line">{{ line }}</p>
          <div class="plans-table-wrap"><table class="plans-table"><thead><tr><th>{{ t('plans.metric') }}</th><th>Baseline</th><th>Candidate</th><th>{{ t('plans.difference') }}</th></tr></thead><tbody>
            <tr v-for="row in comparison.metrics" :key="row.key"><td>{{ row.label }}</td><td>{{ metric(row.baseline) }} {{ row.unit }}</td><td>{{ metric(row.candidate) }} {{ row.unit }}</td><td>{{ metric(row.delta_pct, '%') }}</td></tr>
          </tbody></table></div>
        </template>
      </div>
      <div v-else-if="section === 'pev2'" class="plans-panel plans-pev2-panel" :class="{'plans-pev2-expanded': pevExpanded}">
        <div class="plans-input-head"><h3>{{ t('plans.pev2') }}</h3><button type="button" class="secondary" @click="pevExpanded = !pevExpanded">{{ pevExpanded ? t('plans.normalSize') : t('plans.expandSize') }}</button></div>
        <p class="plans-muted">{{ t('plans.pevWarning') }}</p>
        <p v-if="pevError" role="alert" class="plans-error">{{ pevError }}</p>
        <p v-else-if="pevStatus" role="status" class="plans-muted">{{ pevStatus }}</p>
        <iframe v-if="showPev2" ref="pevFrame" :title="t('plans.pevTitle')" class="plans-pev2-frame"
          src="/pev2/index.html" sandbox="allow-scripts allow-same-origin allow-downloads" referrerpolicy="no-referrer" />
      </div>
      <div v-else-if="section === 'reference'" class="plans-panel">
        <h3>{{ t('plans.postgresReference') }}</h3>
        <p v-if="referenceBusy" class="plans-muted">{{ t('plans.loadingReference') }}</p>
        <p v-if="referenceError" role="alert" class="plans-error">{{ referenceError }}</p>
        <div v-if="referenceEntries.length" class="plans-ref-layout">
          <aside class="plans-ref-list" :aria-label="t('plans.refArticlesAria')">
            <input v-model="referenceFilter" :placeholder="t('plans.refSearch')" :aria-label="t('plans.refSearchAria')" />
            <select v-model="referenceCategory" :aria-label="t('plans.refCategoryAria')"><option value="all">{{ t('plans.choiceAllCategories', { count: String(referenceEntries.length) }) }}</option><option v-for="category in referenceCategories" :key="category" :value="category">{{ category }}</option></select>
            <div class="plans-ref-items">
              <button v-for="entry in filteredReferences" :key="entry.key" type="button" class="secondary plans-ref-choice" :aria-pressed="selectedReference===entry.key" @click="navigateReference(entry.key)"><strong>{{ entry.title }}</strong><small>{{ entry.short }}</small></button>
              <p v-if="!filteredReferences.length" class="plans-muted">{{ t('plans.nothingFound') }}</p>
            </div>
          </aside>
          <article v-if="activeReference" class="plans-ref-article">
            <div class="plans-ref-navigation">
              <button type="button" class="secondary" :disabled="!referenceHistory.length" @click="backReference">{{ t('plans.referenceBack') }}</button>
              <span class="plans-muted">{{ referenceHistory.length ? `${t('plans.chainStep', { count: String(referenceHistory.length + 1) })}` : t('plans.chainStart') }}</span>
            </div>
            <p class="plans-muted">{{ activeReference.category }}</p><h4>{{ activeReference.title }}</h4><p class="plans-ref-intro">{{ activeReference.short }}</p><p>{{ activeReference.how_it_works }}</p>
            <section v-if="activeReference.good_when.length"><h4>{{ t('plans.goodWhen') }}</h4><ul><li v-for="line in activeReference.good_when" :key="line">{{ line }}</li></ul></section>
            <section v-if="activeReference.suspicious_when.length"><h4>{{ t('plans.suspiciousWhen') }}</h4><ul><li v-for="line in activeReference.suspicious_when" :key="line">{{ line }}</li></ul></section>
            <section v-if="activeReference.inspect.length"><h4>{{ t('plans.checkWhat') }}</h4><ul><li v-for="line in activeReference.inspect" :key="line">{{ line }}</li></ul></section>
            <section v-if="activeReference.misconceptions.length"><h4>{{ t('plans.misconceptions') }}</h4><ul><li v-for="line in activeReference.misconceptions" :key="line">{{ line }}</li></ul></section>
            <section v-if="referenceLinks.length" class="plans-related">
              <h4>{{ t('plans.relatedTopics') }}</h4>
              <div class="plans-related-links"><button v-for="entry in referenceLinks" :key="entry.key" type="button" class="secondary" @click="navigateReference(entry.key)">{{ entry.title }}</button></div>
            </section>
            <a :href="activeReference.docs_url" target="_blank" rel="noopener noreferrer">{{ t('plans.postgresDocs') }}</a>
          </article>
        </div>
      </div>
      <div v-else-if="section === 'prompt'" class="plans-panel">
        <h3>{{ t('plans.aiInstructions') }}</h3>
        <p class="plans-muted">{{ t('plans.externalAiHint') }}</p>
        <p v-if="promptBusy">{{ t('plans.loading') }}</p><p v-if="promptError" role="alert" class="plans-error">{{ promptError }}</p>
        <p class="plans-muted">{{ t('plans.aiWarning') }}</p>
        <div v-if="prompt !== null" class="plans-toolbar plans-prompt-actions">
          <button type="button" class="secondary" @click="copyText(prompt, t('plans.prompt'))">{{ t('plans.copyPrompt') }}</button>
          <button type="button" class="secondary" :disabled="!input.trim()" @click="openPlanInPrivacy">{{ t('plans.anonymizeFirst') }}</button>
          <button type="button" :disabled="!input.trim()" @click="copyPromptWithPlan">{{ t('plans.copyRawPrompt') }}</button>
        </div>
        <p v-if="clipboardFeedback" role="status" class="plans-muted">{{ clipboardFeedback }}</p>
        <pre v-if="prompt !== null" class="plans-raw">{{ prompt }}</pre>
      </div>
      <div v-else class="plans-panel"><h3>{{ t('plans.rawJson') }}</h3><pre class="plans-raw">{{ input }}</pre></div>
    </section>
  </section>
</template>
<style scoped>
.plans-root { display:grid;gap:1.15rem;min-width:0; }
.plans-header,.plans-input-head { display:flex;align-items:center;justify-content:space-between;gap:1rem;flex-wrap:wrap; }
.plans-header h2,.plans-input-head h3,.plans-panel h3 { margin:0 0 .35rem; }
.plans-muted { color:var(--text-secondary);font-size:.9rem;overflow-wrap:anywhere; }
.plans-badge { border:1px solid var(--border);border-radius:999px;padding:.4rem .75rem;font-size:.8rem; }
.plans-modes,.plans-toolbar { display:flex;gap:.5rem;flex-wrap:wrap; }
.plans-modes button[aria-pressed=true] { background:var(--surface-2);border-color:var(--border-strong);font-weight:700; }
.plans-input-card,.plans-panel { background:var(--surface);border:1px solid var(--border);border-radius:.75rem;padding:1.15rem;min-width:0; }
.plans-input-card { display:grid;gap:1rem; }
.plans-pickers { display:grid;grid-template-columns:repeat(2,minmax(0,1fr)) auto;gap:.7rem;align-items:end; }
.plans-pickers label,.plans-file-load label,.plans-json-label { display:grid;gap:.45rem;min-width:0; }
.plans-pickers select,.plans-json { box-sizing:border-box;width:100%;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.5rem;padding:.7rem; }
.plans-file-load input { max-width:100%; }
.plans-json { resize:vertical;min-height:220px;font-family:monospace;font-size:.85rem; }
.plans-toolbar { justify-content:flex-end; }
.plans-error { color:var(--error); }
.plans-results { display:grid;gap:1rem; }
.plans-stats { display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.7rem; }
.plans-stats article { min-width:0;background:var(--surface);border:1px solid var(--border);border-radius:.7rem;padding:.9rem;display:grid;gap:.55rem;overflow-wrap:anywhere; }
.plans-stats small,.plans-node small,.plans-spot span { color:var(--text-secondary); }
.plans-stats strong { font-size:1.05rem; }
.plans-panel { display:grid;gap:.65rem; }
.plans-spot,.plans-node { border-bottom:1px solid var(--border);padding:.55rem 0;overflow-wrap:anywhere; }
.plans-spot { display:flex;justify-content:space-between;gap:.7rem;flex-wrap:wrap; }
.plans-node small { display:block;margin-top:.3rem; }
.plans-node-name { padding:0;border:0;background:transparent;color:var(--text);font:inherit;font-weight:700;cursor:pointer;text-align:left; }
.plans-node-name:hover { text-decoration:underline; }
.plans-node-selected { border-left:3px solid var(--border-strong);background:var(--surface-2); }
.plans-node-link { white-space:normal;text-align:left; }
.plans-selected-node { border:1px solid var(--border-strong);border-radius:.6rem;padding:1rem;margin-top:1rem; }
.plans-selected-node h4 { margin:0; }
.plans-finding { border:1px solid var(--border);padding:.8rem;border-radius:.6rem; }
.plans-table-wrap { overflow-x:auto; }
.plans-table { width:100%;border-collapse:collapse;text-align:left; }
.plans-table th,.plans-table td { padding:.6rem;border-bottom:1px solid var(--border);white-space:nowrap; }
.plans-ref-layout { display:grid;grid-template-columns:minmax(250px,310px) minmax(0,1fr);gap:1.2rem;align-items:start; }
.plans-ref-list { display:grid;gap:.6rem;min-width:0; }
.plans-ref-list input,.plans-ref-list select { width:100%;max-width:100%;box-sizing:border-box; }
.plans-ref-items { display:grid;gap:.4rem;max-height:600px;overflow-y:auto;padding-right:.25rem; }
.plans-ref-choice { display:grid;text-align:left;gap:.25rem;min-width:0;white-space:normal; }
.plans-ref-choice small { color:var(--text-secondary);line-height:1.4; }
.plans-ref-choice[aria-pressed=true] { border-color:var(--border-strong);font-weight:700; }
.plans-ref-article { min-width:0;max-height:680px;overflow:auto;line-height:1.6;padding-right:.5rem;overflow-wrap:anywhere; }
.plans-ref-article h4 { margin:1rem 0 .4rem; }
.plans-ref-navigation,.plans-related-links { display:flex;align-items:center;gap:.6rem;flex-wrap:wrap; }
.plans-ref-navigation { position:sticky;top:0;background:var(--surface);padding:.4rem 0 .7rem;z-index:1;border-bottom:1px solid var(--border); }
.plans-related-links button { white-space:normal;text-align:left; }
.plans-ref-article section { margin-top:1.25rem; }
.plans-ref-article li { margin-bottom:.35rem; }
.plans-ref-intro { font-weight:600; }
.plans-reference-link { margin:.5rem 0; }
.plans-prompt-actions { justify-content:flex-start; }
.plans-jit-note { padding:.75rem;border:1px solid var(--border);border-radius:.5rem; }
.plans-tree-toggle { padding:.15rem .45rem;min-width:2rem;margin-right:.4rem; }
.plans-tree-leaf { display:inline-block;min-width:2rem;text-align:center;margin-right:.4rem; }
.plans-finding-group summary { cursor:pointer; }
.plans-finding-group > summary { display:flex;flex-wrap:wrap;justify-content:space-between;gap:.5rem; }
.plans-finding-entry { border-top:1px solid var(--border);padding:.8rem 0; }
@media(max-width:650px){.plans-ref-layout{grid-template-columns:1fr}}
/* Contain the PEV2 scrolling and CSS in its own frame. */
.plans-pev2-panel { min-height: min(78vh, 860px); }
.plans-pev2-expanded { position:fixed;inset:1rem;z-index:1000;overflow:auto;background:var(--surface);box-shadow:0 0 0 100vmax #000a; }
.plans-pev2-expanded .plans-pev2-frame { height:calc(100vh - 12rem);min-height:520px; }
.plans-pev2-frame { width:100%;height:min(73vh,800px);min-height:480px;border:1px solid var(--border);border-radius:.6rem;background:white; }
.plans-raw { max-height:460px;overflow:auto;white-space:pre;font-size:.8rem;background:var(--surface-2);padding:.8rem;border-radius:.5rem; }
@media(max-width:950px){.plans-stats{grid-template-columns:repeat(2,minmax(0,1fr))}.plans-pickers{grid-template-columns:1fr}}
@media(max-width:530px){.plans-stats{grid-template-columns:1fr}}
</style>
