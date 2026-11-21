<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { t } from "../i18n/locale-service.js";
import { computed, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import { createPrivacyController, createPrivacyState, type PrivacyMode, type PrivacyPolicy, type MappingEntry, type PrivacyPrefill } from "./privacy.js";

const props = defineProps<{
  shell: ShellState; invoke: PgXrayInvoke; connectionId: number | null; visible: boolean;
  prefill?: PrivacyPrefill | null;
}>();
const state = reactive(createPrivacyState());
const actions = createPrivacyController(props.shell, state, props.invoke);
type Page = "transform" | "policy" | "mapping";
const page = ref<Page>("transform");
const name = ref("");
const connectionOnly = ref(true);
const creatingScope = ref(false);
const input = ref("");
const mode = ref<PrivacyMode>("sql");
const sourceLabel = ref("");
const search = ref("");
const category = ref("all");
const policyText = ref("");
const newField = ref("");
const newPattern = ref("");
const guideOpen = ref(false);
const exampleId = ref<"columns" | "regex" | "dynamic" | "json_cell" | "selective">("columns");
const policyExamples = computed(() => ({
  columns: {
    title: t('privacyTools.exampleColumns'),
    purpose: t('privacyTools.columnsPurpose'),
    input: '[{"customer_code":987654321,"table_name":"secret_orders","calls":350,"rows":1200}]',
    notes: t('privacyTools.columnsNotes'),
    policy: { default: "auto", fields: { customer_code: "value", table_name: "table", calls: "preserve", rows: "preserve" }, patterns: [] },
  },
  regex: {
    title: t('privacyTools.exampleRegex'),
    purpose: t('privacyTools.regexPurpose'),
    input: '[{"customer_code":987654321,"employee_number":87542,"calls":350}]',
    notes: t('privacyTools.regexNotes'),
    policy: { default: "auto", fields: { calls: "preserve" }, patterns: [{ pattern: "^(customer|employee)_(code|number)$", action: "value" }] },
  },
  dynamic: {
    title: t('privacyTools.exampleDynamic'),
    purpose: t('privacyTools.dynamicPurpose'),
    input: '{"customer-secret-prod":{"database_name":"finance_private","tables":{"private_orders_2026":{"table_name":"private_orders_2026","rows":100}}}}',
    notes: t('privacyTools.dynamicNotes'),
    policy: { default: "auto", keys: "value", fields: {}, patterns: [{ pattern: "^customer-.*$", action: { default: "auto", fields: { tables: { default: "auto", keys: "table" } } } }] },
  },
  json_cell: {
    title: t('privacyTools.exampleCell'),
    purpose: t('privacyTools.cellPurpose'),
    input: 'payload | {"customer_id":987654321,"status":"private"}\n(1 row)',
    notes: t('privacyTools.cellNotes'),
    policy: { default: "auto", fields: { payload: { json: true, default: "auto", fields: { customer_id: "value" } } }, patterns: [] },
  },
  selective: {
    title: t('privacyTools.exampleSelective'),
    purpose: t('privacyTools.selectivePurpose'),
    input: '[{"table_name":"secret_orders","user_name":"admin","calls":350}]',
    notes: t('privacyTools.selectiveNotes'),
    policy: { default: "preserve", fields: { table_name: "table" }, patterns: [] },
  },
} as const));
const selectedPolicyExample = computed(() => policyExamples.value[exampleId.value]);
const selectedPolicyJson = computed(() => JSON.stringify(selectedPolicyExample.value.policy, null, 2));
async function insertExamplePolicy(): Promise<void> {
  if (policyChanged.value && !await confirmAction(t('privacyTools.replaceUnsaved'))) return;
  policyText.value = selectedPolicyJson.value;
  state.error = "";
  guideOpen.value = false;
}
const fieldActions = ["auto", "preserve", "value", "schema", "table", "column", "columns", "index", "database", "role", "application", "address", "tablespace", "sql", "sql_fragment"] as const;
type SimpleAction = typeof fieldActions[number];
function parseDraftPolicy(): PrivacyPolicy | null {
  try {
    const value: unknown = JSON.parse(policyText.value);
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    const policy = value as Record<string, unknown>;
    if (!policy.fields || typeof policy.fields !== "object" || Array.isArray(policy.fields)) return null;
    if (!Array.isArray(policy.patterns)) return null;
    return policy as unknown as PrivacyPolicy;
  } catch { return null; }
}
const draftPolicy = computed(parseDraftPolicy);
const policyChanged = computed(() => {
  if (!scope.value || !draftPolicy.value) return false;
  return JSON.stringify(draftPolicy.value) !== JSON.stringify(scope.value.result_privacy);
});
const policyIsPreserve = computed(() => draftPolicy.value?.default === "preserve");
const actionNames = computed<Record<SimpleAction, string>>(() => ({
  auto: t('privacyTools.actionAuto'), preserve: t('privacyTools.actionPreserve'),
  value: t('privacyTools.actionValue'), schema: t('privacyTools.actionSchema'), table: t('privacyTools.actionTable'),
  column: t('privacyTools.actionColumn'), columns: t('privacyTools.actionColumns'),
  index: t('privacyTools.actionIndex'), database: t('privacyTools.actionDatabase'), role: t('privacyTools.actionRole'),
  application: t('privacyTools.actionApp'), address: t('privacyTools.actionAddress'),
  tablespace: t('privacyTools.actionTablespace'), sql: t('privacyTools.actionSql'),
  sql_fragment: t('privacyTools.actionFragment'),
}));
async function restoreSavedPolicy(): Promise<void> {
  if (!scope.value) return;
  if (policyChanged.value && !await confirmAction(t('privacyTools.resetRulesConfirm'))) return;
  policyText.value = JSON.stringify(scope.value.result_privacy, null, 2);
  state.error = "";
}
async function returnToTransform(): Promise<void> {
  if (policyChanged.value && !await confirmAction(t('privacyTools.backUnsavedConfirm'))) return;
  page.value = "transform";
}
const rootKeyAction = computed(() => (draftPolicy.value as (PrivacyPolicy & { keys?: string }) | null)?.keys ?? "preserve");
const draftFields = computed(() => Object.entries(draftPolicy.value?.fields ?? {}));
const draftPatterns = computed(() => (draftPolicy.value?.patterns ?? []) as Array<{ pattern: string; action: unknown }>);
function editPolicyDraft(edit: (draft: PrivacyPolicy) => void): void {
  const draft = parseDraftPolicy();
  if (!draft) { state.error = t('privacyTools.draftInvalid'); return; }
  edit(draft);
  policyText.value = JSON.stringify(draft, null, 2);
  state.error = "";
}
function changeDefault(value: string): void {
  editPolicyDraft(draft => { draft.default = value; });
}
function changeRootKeys(value: string): void {
  editPolicyDraft(draft => {
    const config = draft as PrivacyPolicy & { keys?: string };
    if (value === "preserve") delete config.keys;
    else config.keys = value;
  });
}
function addField(): void {
  const field = newField.value.trim();
  if (!field || field.length > 128) { state.error = t('privacyTools.invalidField'); return; }
  editPolicyDraft(draft => { if (!(field in draft.fields)) draft.fields[field] = "value"; });
  newField.value = "";
}
function updateField(field: string, action: string): void {
  editPolicyDraft(draft => { draft.fields[field] = action; });
}
function removeField(field: string): void {
  editPolicyDraft(draft => { delete draft.fields[field]; });
}
function addPattern(): void {
  const pattern = newPattern.value.trim();
  if (!pattern) return;
  editPolicyDraft(draft => { draft.patterns.push({ pattern, action: "value" }); });
  newPattern.value = "";
}
function updatePattern(index: number, action: string): void {
  editPolicyDraft(draft => { (draft.patterns[index] as { action: unknown }).action = action; });
}
function removePattern(index: number): void {
  editPolicyDraft(draft => { draft.patterns.splice(index, 1); });
}
function insertNestedExample(): void {
  editPolicyDraft(draft => {
    (draft as PrivacyPolicy & { keys?: string }).keys = "value";
    draft.patterns.push({ pattern: "^customer-.*$", action: {
      keys: "value", default: "auto", fields: {
        tables: { keys: "table", default: "auto" },
      },
    } });
  });
}
const selected = ref<string[]>([]);
const scope = computed(() => state.scopes.find(item => item.id === state.scopeId) ?? null);
const mappingCount = computed(() => state.mapping ? Object.values(state.mapping).filter((v): v is number => typeof v === "number").reduce((a, b) => a + b, 0) : 0);
const key = (item: MappingEntry): string => JSON.stringify([item.category, item.original]);
const categories = computed(() => [...new Set(state.entries.map(item => item.category))]);

watch(() => [props.connectionId, props.shell.authGeneration] as const, () => {
  actions.reset(props.connectionId);
  if (props.visible) void actions.refreshScopes();
}, { immediate: true });
watch(() => props.visible, visible => { if (visible && props.connectionId) void actions.refreshScopes(); });
watch(() => state.scopeId, () => {
  state.output = "";
  state.warnings = [];
  selected.value = [];
  policyText.value = JSON.stringify(scope.value?.result_privacy ?? { default: "auto", fields: {}, patterns: [] }, null, 2);
  if (state.scopeId) void actions.refreshMapping();
});
watch([input, mode], () => {
  state.output = "";
  state.warnings = [];
});
watch(() => props.prefill, value => {
  if (!value) return;
  input.value = value.input;
  mode.value = value.mode;
  sourceLabel.value = value.fileName;
  page.value = "transform";
  state.output = "";
  state.warnings = [];
}, { immediate: true });
watch(() => [state.scopes, props.prefill] as const, ([scopes, value]) => {
  if (value?.scopeId && scopes.some(item => item.id === value.scopeId) && state.scopeId !== value.scopeId) {
    state.scopeId = value.scopeId;
  }
}, { immediate: true });

async function create(): Promise<void> {
  if (await actions.createScope(name.value, connectionOnly.value)) { name.value = ""; creatingScope.value = false; }
}
async function savePolicy(): Promise<void> {
  try {
    const parsed: unknown = JSON.parse(policyText.value);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error(t('privacyTools.policyInvalid'));
    await actions.savePolicy(parsed as PrivacyPolicy);
  } catch (error) { state.error = t('privacyTools.resultPolicyError', { error: String(error) }); }
}
async function resetMapping(): Promise<void> {
  if (await confirmAction(t('privacyTools.resetMappingConfirm'))) void actions.resetMapping();
}
async function removeEntries(): Promise<void> {
  const entries = state.entries.filter(item => selected.value.includes(key(item)));
  if (!entries.length) return;
  if (await confirmAction(t('privacyTools.deleteMappingsConfirm', { count: String(entries.length) }))) {
    void actions.deleteEntries(entries).then(ok => { if (ok) selected.value = []; });
  }
}
async function removeScope(): Promise<void> {
  if (await confirmAction(t('privacyTools.deleteScopeConfirm', { name: scope.value?.name ?? '' }))) void actions.deleteScope();
}
async function copyOutput(): Promise<void> {
  try { await navigator.clipboard.writeText(state.output); state.notice = t('privacyTools.copied'); }
  catch (error) { state.error = String(error); }
}
function useOutputAsInput(): void {
  if (!state.output) return;
  input.value = state.output;
  sourceLabel.value = "";
  state.output = "";
  state.warnings = [];
}
function clearInput(): void {
  input.value = "";
  sourceLabel.value = "";
  state.output = "";
  state.warnings = [];
}
</script>

<template>
  <section class="privacy-workspace" :aria-label="t('privacyTools.scopeAria')">
    <header class="privacy-head">
      <div><h2>{{ t('privacyTools.title') }}</h2><p class="privacy-muted">{{ t('privacyTools.introSubtitle') }}</p></div>
      <span class="privacy-local">{{ t('privacyTools.localMapping') }}</span>
    </header>
    <p class="privacy-warning">{{ t('privacyTools.warning') }}</p>
    <p v-if="state.error" class="privacy-error" role="alert">{{ state.error }}</p>
    <p v-if="state.notice" class="privacy-notice" role="status">{{ state.notice }}</p>
    <div class="privacy-layout">
      <aside class="privacy-sidebar">
        <div class="privacy-sidebar-top"><h3>{{ t('privacyTools.allScope') }}</h3><button type="button" class="secondary privacy-small" :aria-expanded="creatingScope" @click="creatingScope = !creatingScope">{{ creatingScope ? t('privacyTools.cancel') : '+ Scope' }}</button></div>
        <label>Privacy scope
          <select v-model.number="state.scopeId" :disabled="state.busy || !connectionId">
            <option :value="null">{{ t('privacyTools.chooseScope') }}</option>
            <option v-for="item in state.scopes" :key="item.id" :value="item.id">#{{ item.id }} · {{ item.name }}{{ item.is_default ? ' (default)' : '' }}</option>
          </select>
        </label>
        <form v-if="creatingScope" class="privacy-create" @submit.prevent="create">
          <label>{{ t('privacyTools.newScopeName') }}<input v-model="name" maxlength="200" required /></label>
          <label class="privacy-check"><input v-model="connectionOnly" type="checkbox" /> {{ t('privacyTools.connectionOnly') }}</label>
          <button type="submit" :disabled="state.busy || !connectionId || !name.trim()">{{ t('privacyTools.createScope') }}</button>
        </form>
        <template v-if="scope">
          <div class="privacy-scope-meta"><strong>{{ scope.name }}</strong><small>Scope #{{ scope.id }} · {{ scope.is_default ? t('privacyTools.scopeDefault') : t('privacyTools.scopeCustom') }}</small><small>{{ scope.connection_id ? t('privacyTools.scopeSingle') : t('privacyTools.scopeAll') }}</small></div>
          <nav class="privacy-nav" :aria-label="t('privacyTools.sectionsAria')">
            <button type="button" :aria-pressed="page === 'transform'" @click="page === 'policy' ? returnToTransform() : page = 'transform'"><strong>{{ t('privacyTools.transform') }}</strong><small>{{ t('privacyTools.transformHelp') }}</small></button>
            <button type="button" :aria-pressed="page === 'mapping'" @click="page = 'mapping'"><strong>Mapping</strong><small>{{ mappingCount }} {{ t('privacyTools.mappingEntries') }}</small></button>
            <button type="button" :aria-pressed="page === 'policy'" @click="page = 'policy'"><strong>{{ t('privacyTools.rules') }}</strong><small>{{ t('privacyTools.resultPolicyMenu') }}</small></button>
          </nav>
        </template>
      </aside>
      <main class="privacy-main">
        <div v-if="!scope" class="privacy-empty">{{ t('privacyTools.choosePrivacy') }}</div>
        <template v-else-if="page === 'transform'">
          <div class="privacy-section-head"><div><h3>{{ t('privacyTools.transformData') }}</h3><p class="privacy-muted">{{ t('privacyTools.transformDescription') }}</p></div><button type="button" class="secondary" :disabled="!input" @click="clearInput">{{ t('privacyTools.clear') }}</button></div>
          <p v-if="sourceLabel" class="privacy-source">{{ t('privacyTools.sourceLabel') }} <strong>{{ sourceLabel }}</strong> · {{ t('privacyTools.sourceUnchanged') }}</p>
          <div class="privacy-settings"><label>{{ t('privacyTools.formatLabel') }}<select v-model="mode"><option value="sql">{{ t('privacyTools.sqlQuery') }}</option><option value="explain">EXPLAIN JSON</option><option value="text">{{ t('privacyTools.textAll') }}</option><option value="text_known">{{ t('privacyTools.textKnown') }}</option><option value="result">{{ t('privacyTools.queryResult') }}</option></select></label><span class="privacy-muted">{{ t('privacyTools.formatScope') }} #{{ scope.id }}</span></div>
          <p v-if="mode === 'text_known'" class="privacy-warning">{{ t('privacyTools.knownOnlyWarning') }}</p>
          <p v-if="mode === 'text'" class="privacy-muted">{{ t('privacyTools.knownWarning') }}</p>
          <p v-if="mode === 'result'" class="privacy-muted">{{ t('privacyTools.resultHelp') }} <code>\gx</code>. {{ t('privacyTools.resultSettings') }} <button type="button" class="secondary privacy-small" @click="page = 'policy'">{{ t('privacyTools.configureColumns') }}</button></p>
          <div class="privacy-editors">
            <label class="privacy-editor">{{ t('privacyTools.sourceData') }} <textarea v-model="input" rows="14" spellcheck="false" :placeholder="t('privacyTools.sourcePlaceholder')" /></label>
            <label class="privacy-editor">{{ t('privacyTools.result') }} <textarea :value="state.output" rows="14" readonly spellcheck="false" :placeholder="t('privacyTools.resultPlaceholder')" /></label>
          </div>
          <div class="privacy-toolbar"><button type="button" :disabled="state.busy || !input" @click="actions.transform(mode, 'anonymize', input)">{{ t('privacyTools.anonymize') }}</button><button type="button" class="secondary" :disabled="state.busy || !input" @click="actions.transform(mode, 'deanonymize', input)">{{ t('privacyTools.deanonymize') }}</button><button type="button" class="secondary" :disabled="state.busy || !state.output" @click="useOutputAsInput">{{ t('privacyTools.resultToInput') }}</button><button type="button" class="secondary" :disabled="state.busy || !state.output" @click="copyOutput">{{ t('privacyTools.copyResult') }}</button></div>
          <div v-if="state.warnings.length" class="privacy-warning"><p v-for="warning in state.warnings" :key="warning">{{ warning }}</p></div>
          <p class="privacy-muted">{{ t('privacyTools.transformWarning') }}</p>
        </template>
        <template v-else-if="page === 'mapping'">
          <div class="privacy-section-head"><div><h3>Mapping</h3><p class="privacy-muted">{{ t('privacyTools.mappingDescription') }}</p></div><button type="button" class="secondary" :disabled="state.busy" @click="actions.refreshMapping(search, category === 'all' ? null : category)">{{ t('privacyTools.refresh') }}</button></div>
          <div class="privacy-stats"><span>{{ t('privacyTools.identifiers') }} <strong>{{ state.mapping?.identifiers ?? 0 }}</strong></span><span>{{ t('privacyTools.strings') }} <strong>{{ state.mapping?.strings ?? 0 }}</strong></span><span>{{ t('privacyTools.numbers') }} <strong>{{ (state.mapping?.integers ?? 0) + (state.mapping?.floats ?? 0) }}</strong></span><span>{{ t('privacyTools.redactions') }} <strong>{{ state.mapping?.redactions ?? 0 }}</strong></span></div>
          <div class="privacy-toolbar privacy-filter"><input v-model="search" :placeholder="t('privacyTools.mappingSearch')" :aria-label="t('privacyTools.mappingSearchAria')" /><select v-model="category" :aria-label="t('privacyTools.searchCategory')"><option value="all">{{ t('privacyTools.allCategories') }}</option><option v-for="item in categories" :key="item" :value="item">{{ item }}</option></select><button type="button" class="secondary" :disabled="state.busy" @click="actions.refreshMapping(search, category === 'all' ? null : category)">{{ t('privacyTools.find') }}</button></div>
          <div class="privacy-table-wrap"><table><thead><tr><th><span class="sr-only">{{ t('privacyTools.select') }}</span></th><th>{{ t('privacyTools.category') }}</th><th>{{ t('privacyTools.type') }}</th><th>{{ t('privacyTools.original') }}</th><th>{{ t('privacyTools.alias') }}</th></tr></thead><tbody><tr v-for="item in state.entries" :key="key(item)"><td><input v-model="selected" type="checkbox" :value="key(item)" :aria-label="t('vueFinalPolish.selectEntry', { alias: item.alias })" /></td><td>{{ item.category }}</td><td>{{ item.kind ?? '—' }}</td><td><code>{{ item.original }}</code></td><td><code>{{ item.alias }}</code></td></tr><tr v-if="!state.entries.length"><td colspan="5" class="privacy-muted">{{ t('privacyTools.mappingEmpty') }}</td></tr></tbody></table></div>
          <details class="privacy-danger-zone"><summary>{{ t('privacyTools.mappingDanger') }}</summary><p class="privacy-warning">{{ t('privacyTools.mappingDeleteWarning') }}</p><div class="privacy-toolbar"><button type="button" class="danger" :disabled="state.busy || !selected.length" @click="removeEntries">{{ t('privacyTools.mappingDeleteSelection', { count: String(selected.length) }) }}</button><button type="button" class="danger" :disabled="state.busy" @click="resetMapping">{{ t('privacyTools.clearMapping') }}</button><button type="button" class="danger" :disabled="state.busy || scope.is_default" @click="removeScope">{{ t('privacyTools.deleteScope') }}</button></div></details>
          <p class="privacy-muted privacy-path">{{ t('privacyTools.mappingPath') }} {{ state.mapping?.path ?? '—' }}</p>
        </template>
        <template v-else>
          <div class="privacy-section-head privacy-policy-header">
            <div><h3>{{ t('privacyTools.policyTitle') }}</h3><p class="privacy-muted">{{ t('privacyTools.jsonProcessingDescription') }} <code>\gx</code> · scope #{{ scope.id }}</p></div>
            <div class="privacy-policy-header-actions">
              <span v-if="policyChanged" class="privacy-unsaved" role="status">{{ t('privacyTools.unsaved') }}</span>
              <button type="button" class="secondary privacy-small" @click="returnToTransform">{{ t('privacyTools.returnTransform') }}</button>
            </div>
          </div>
          <p class="privacy-muted">{{ t('privacyTools.policyHelp') }} <strong>Auto</strong> {{ t('privacyTools.autoHelp') }} <strong>Preserve</strong> {{ t('privacyTools.preserveHelp') }}</p>
          <p v-if="policyIsPreserve" class="privacy-warning" role="status">{{ t('privacyTools.preserveWarning') }}</p>
          <div class="privacy-policy-help-bar">
            <div><strong>{{ t('privacyTools.guidanceTitle') }}</strong><p class="privacy-muted">{{ t('privacyTools.guidanceHint') }}</p></div>
            <button type="button" class="secondary" :aria-expanded="guideOpen" @click="guideOpen = !guideOpen">{{ guideOpen ? t('privacyTools.guideHide') : t('privacyTools.guideShow') }}</button>
          </div>
          <section v-if="guideOpen" class="privacy-policy-guide" :aria-label="t('privacyTools.guideAria')">
            <div class="privacy-policy-guide-intro">
              <h4>{{ t('privacyTools.policyGuide') }}</h4>
              <ol>
                <li><strong>default</strong> — {{ t('privacyTools.guideDefault') }} <code>auto</code> {{ t('privacyTools.guideRecommended') }} <code>preserve</code>.</li>
                <li><strong>fields</strong> — {{ t('privacyTools.guideFields') }} <code>"table_name": "table"</code>.</li>
                <li><strong>patterns</strong> — {{ t('privacyTools.guidePatterns') }} <em>{{ t('privacyTools.guideFieldName') }}</em>, {{ t('privacyTools.guidePatternOrder') }} <code>fields</code>.</li>
                <li><strong>keys</strong> — {{ t('privacyTools.guideKeys') }}</li>
                <li><strong>values</strong> — {{ t('privacyTools.guideValues') }} <strong>json: true</strong> — {{ t('privacyTools.guideJsonTrue') }}</li>
              </ol>
              <p class="privacy-muted">{{ t('privacyTools.nestedRules') }}</p>
            </div>
            <div class="privacy-policy-example-picker">
              <label>{{ t('privacyTools.exampleSelect') }}
                <select v-model="exampleId">
                  <option value="columns">{{ t('privacyTools.exampleColumnLabel') }}</option>
                  <option value="regex">{{ t('privacyTools.exampleRegexLabel') }}</option>
                  <option value="dynamic">{{ t('privacyTools.exampleDynamicLabel') }}</option>
                  <option value="json_cell">{{ t('privacyTools.exampleCellLabel') }}</option>
                  <option value="selective">{{ t('privacyTools.exampleSelectiveLabel') }}</option>
                </select>
              </label>
              <div class="privacy-policy-example-description">
                <strong>{{ selectedPolicyExample.title }}</strong>
                <p>{{ selectedPolicyExample.purpose }}</p>
              </div>
            </div>
            <div class="privacy-policy-example-grid">
              <div><strong>{{ t('privacyTools.sampleInput') }}</strong><pre>{{ selectedPolicyExample.input }}</pre></div>
              <div><strong>Policy JSON</strong><pre>{{ selectedPolicyJson }}</pre></div>
            </div>
            <p class="privacy-muted">{{ selectedPolicyExample.notes }}</p>
            <div class="privacy-policy-guide-actions">
              <button type="button" class="secondary" @click="insertExamplePolicy">{{ t('privacyTools.saveExample') }}</button>
              <span class="privacy-muted">{{ t('privacyTools.exampleHelp') }}</span>
            </div>
          </section>
          <div class="privacy-policy-steps" :aria-label="t('privacyTools.guideStepAria')">
            <span><strong>1</strong> {{ t('privacyTools.guideStepOne') }}</span>
            <span><strong>2</strong> {{ t('privacyTools.guideStepTwo') }}</span>
            <span><strong>3</strong> {{ t('privacyTools.guideStepThree') }}</span>
          </div>
          <template v-if="draftPolicy">
            <label class="privacy-policy-label">{{ t('privacyTools.defaultRuleLabel') }}
              <select :value="draftPolicy.default" @change="changeDefault(($event.target as HTMLSelectElement).value)">
                <option value="auto">{{ t('privacyTools.actionAutoLabel') }}</option>
                <option value="preserve">{{ t('privacyTools.actionPreserveLabel') }}</option>
              </select>
            </label>
            <label class="privacy-policy-label">{{ t('privacyTools.topKeys') }}
              <select :value="rootKeyAction" @change="changeRootKeys(($event.target as HTMLSelectElement).value)">
                <option value="preserve">{{ t('privacyTools.keepFieldNames') }}</option>
                <option value="value">{{ t('privacyTools.anonymizeTopKeys') }}</option>
                <option value="table">{{ t('privacyTools.anonymizeTopTables') }}</option>
              </select>
            </label>
            <p class="privacy-muted">{{ t('privacyTools.ruleDanger') }}</p>
            <h4 class="privacy-policy-subtitle">{{ t('privacyTools.columnRules') }}</h4>
            <p class="privacy-muted">{{ t('privacyTools.columnRuleHelp') }}</p>
            <div class="privacy-policy-list" :aria-label="t('privacyTools.columnRulesAria')">
            <div v-for="[field, action] in draftFields" :key="field" class="privacy-policy-row">
              <code :title="field">{{ field }}</code>
              <select v-if="typeof action === 'string'" :value="action" :aria-label="t('privacyTools.fieldAction', { field })" @change="updateField(field, ($event.target as HTMLSelectElement).value)">
                <option v-for="item in fieldActions" :key="item" :value="item">{{ actionNames[item] }}</option>
              </select>
              <span v-else class="privacy-muted">{{ t('privacyTools.nestedRule') }}</span>
              <button type="button" class="secondary privacy-small" @click="removeField(field)" :aria-label="t('privacyTools.removeField', { field })">{{ t('privacyTools.delete') }}</button>
            </div>
            <p v-if="!draftFields.length" class="privacy-muted">{{ t('privacyTools.noColumnRules') }}</p>
            </div>
            <div class="privacy-policy-add">
              <input v-model="newField" maxlength="128" :placeholder="t('privacyTools.newColumnPlaceholder')" :aria-label="t('privacyTools.newColumnAria')" @keyup.enter="addField" />
              <button type="button" class="secondary" @click="addField">{{ t('privacyTools.addColumn') }}</button>
            </div>
            <h4 class="privacy-policy-subtitle">{{ t('privacyTools.columnPatterns') }}</h4>
            <p class="privacy-muted">{{ t('privacyTools.patternRuleHelp') }} <code>^(customer|employee)_.*$</code>.</p>
            <div class="privacy-policy-list" :aria-label="t('privacyTools.regexRulesAria')">
            <div v-for="(rule, index) in draftPatterns" :key="index" class="privacy-policy-row">
              <code :title="rule.pattern">{{ rule.pattern }}</code>
              <select v-if="typeof rule.action === 'string'" :value="rule.action" :aria-label="t('privacyTools.patternAction', { pattern: rule.pattern })" @change="updatePattern(index, ($event.target as HTMLSelectElement).value)">
                <option v-for="item in fieldActions" :key="item" :value="item">{{ actionNames[item] }}</option>
              </select>
              <span v-else class="privacy-muted">{{ t('privacyTools.nestedRule') }}</span>
              <button type="button" class="secondary privacy-small" @click="removePattern(index)">{{ t('privacyTools.delete') }}</button>
            </div>
            <p v-if="!draftPatterns.length" class="privacy-muted">{{ t('privacyTools.noPatterns') }}</p>
            </div>
            <div class="privacy-policy-add">
              <input v-model="newPattern" :placeholder="t('privacyTools.newRegexPlaceholder')" :aria-label="t('privacyTools.newPatternAria')" @keyup.enter="addPattern" />
              <button type="button" class="secondary" @click="addPattern">{{ t('privacyTools.addPattern') }}</button>
            </div>
            <details class="privacy-policy-advanced">
              <summary>{{ t('privacyTools.nestedSettings') }}</summary>
              <p class="privacy-muted">{{ t('privacyTools.dynamicKeyHelp') }}</p>
              <button type="button" class="secondary privacy-small" @click="insertNestedExample">{{ t('privacyTools.addNestedExample') }}</button>
              <p class="privacy-muted">{{ t('privacyTools.nestedExampleHelp') }}</p>
              <label class="privacy-editor">{{ t('privacyTools.policyEditorLabel') }}
                <textarea v-model="policyText" rows="11" spellcheck="false" />
              </label>
            </details>
          </template>
          <div v-else>
            <p class="privacy-error">{{ t('privacyTools.jsonPolicyError') }}</p>
            <label class="privacy-editor">JSON policy<textarea v-model="policyText" rows="11" spellcheck="false" /></label>
          </div>
          <div class="privacy-policy-footer">
            <button type="button" :disabled="state.busy || !draftPolicy || !policyChanged" @click="savePolicy">{{ t('privacyTools.saveRules') }}</button>
            <button type="button" class="secondary" :disabled="state.busy" @click="restoreSavedPolicy">{{ t('privacyTools.resetChanges') }}</button>
            <span class="privacy-muted">{{ t('privacyTools.policyValidated') }}</span>
          </div>
        </template>
      </main>
    </div>
  </section>
</template>

<style scoped>
.privacy-workspace { display:grid;gap:1rem;min-width:0; }
.privacy-head,.privacy-sidebar-top,.privacy-section-head { display:flex;align-items:flex-start;justify-content:space-between;gap:1rem; }
.privacy-head h2,.privacy-section-head h3 { margin:0 0 .4rem; }
.privacy-muted { color:var(--text-secondary);font-size:.9rem;margin:.25rem 0;overflow-wrap:anywhere; }
.privacy-local { color:var(--text-secondary);font-size:.8rem;align-self:center; }
.privacy-warning { border:1px solid var(--border-strong);border-radius:.5rem;padding:.8rem 1rem;margin:0;overflow-wrap:anywhere; }
.privacy-warning p { margin:.15rem 0; }
.privacy-error { color:var(--error);overflow-wrap:anywhere; }
.privacy-notice { color:var(--text-secondary); }
.privacy-policy-help-bar { display:flex;justify-content:space-between;align-items:center;gap:1rem;flex-wrap:wrap;border:1px solid var(--border);border-radius:.65rem;padding:.85rem 1rem;background:var(--surface-2); }
.privacy-policy-help-bar p { margin:.2rem 0 0; }
.privacy-policy-guide { display:grid;gap:1rem;border:1px solid var(--border);border-radius:.7rem;padding:1rem;min-width:0; }
.privacy-policy-guide h4 { margin:0 0 .65rem; }
.privacy-policy-guide ol { margin:.4rem 0;padding-left:1.4rem;display:grid;gap:.4rem; }
.privacy-policy-example-picker { display:grid;grid-template-columns:minmax(210px,1fr) minmax(0,1.4fr);gap:1rem;align-items:end; }
.privacy-policy-example-picker label { display:grid;gap:.4rem; }
.privacy-policy-example-picker select { width:100%; }
.privacy-policy-example-description p { margin:.25rem 0; }
.privacy-policy-example-grid { display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:.7rem; }
.privacy-policy-example-grid > div { min-width:0;display:grid;grid-template-rows:auto 1fr;gap:.4rem; }
.privacy-policy-example-grid pre { background:var(--surface-2);border:1px solid var(--border);border-radius:.4rem;padding:.75rem;white-space:pre-wrap;overflow-wrap:anywhere;max-height:260px;overflow:auto;font-size:.8rem;margin:0; }
.privacy-policy-guide-actions { display:flex;align-items:center;gap:.7rem;flex-wrap:wrap; }
.privacy-policy-steps { display:flex;flex-wrap:wrap;gap:.55rem; }
.privacy-policy-steps span { background:var(--surface-2);border:1px solid var(--border);border-radius:.5rem;padding:.5rem .7rem;font-size:.85rem; }
.privacy-policy-steps strong { padding-right:.3rem; }
.privacy-policy-header { align-items:center;flex-wrap:wrap; }
.privacy-policy-header-actions { display:flex;align-items:center;gap:.7rem;flex-wrap:wrap; }
.privacy-unsaved { font-size:.85rem;color:var(--text-secondary); }
.privacy-policy-list { display:grid;gap:.4rem;margin:.4rem 0 1rem; }
.privacy-policy-row { display:grid;grid-template-columns:minmax(140px,1fr) minmax(200px,1.4fr) auto;align-items:center;gap:.6rem;max-width:780px;padding:.35rem .5rem;border:1px solid var(--border);border-radius:.4rem; }
.privacy-policy-row code { min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap; }
.privacy-policy-row select { width:100%;min-width:0; }
.privacy-policy-add { display:grid;grid-template-columns:minmax(0,1fr) auto;gap:.6rem;align-items:center;max-width:780px; }
.privacy-policy-add input { width:100%;min-width:0; }
.privacy-policy-label { display:grid;gap:.4rem;max-width:36rem; }
.privacy-policy-subtitle { margin:.7rem 0 0; }
.privacy-policy-advanced { border:1px solid var(--border);border-radius:.5rem;padding:.8rem 1rem; }
.privacy-policy-advanced summary { cursor:pointer;font-weight:600; }
.privacy-policy-advanced .privacy-editor { margin-top:1rem; }
.privacy-policy-footer { display:flex;align-items:center;gap:.6rem;flex-wrap:wrap; }
.privacy-layout { display:grid;grid-template-columns:minmax(220px,270px) minmax(0,1fr);gap:1rem;align-items:start; }
.privacy-sidebar,.privacy-main { border:1px solid var(--border);border-radius:.75rem;background:var(--surface);min-width:0;padding:1.1rem; }
.privacy-sidebar { display:grid;gap:1rem; }
.privacy-sidebar h3 { margin:.2rem 0;font-size:1rem; }
.privacy-sidebar label,.privacy-create label,.privacy-editor,.privacy-settings label { display:grid;gap:.4rem;min-width:0; }
.privacy-sidebar select,.privacy-sidebar input:not([type=checkbox]),.privacy-create input:not([type=checkbox]),.privacy-settings select,.privacy-editor textarea,.privacy-filter input,.privacy-filter select { width:100%;box-sizing:border-box;min-width:0; }
.privacy-create { max-width:none;margin:0;display:grid;gap:.7rem;padding:.8rem;background:var(--surface-2);border-radius:.5rem; }
.privacy-check { display:flex!important;align-items:center;gap:.5rem; }
.privacy-check input { width:auto; }
.privacy-small { padding:.45rem .65rem;white-space:nowrap; }
.privacy-scope-meta { display:grid;gap:.2rem;border-top:1px solid var(--border);padding-top:.8rem;overflow-wrap:anywhere; }
.privacy-scope-meta small { color:var(--text-secondary); }
.privacy-nav { display:grid;gap:.35rem; }
.privacy-nav button { display:grid;gap:.3rem;width:100%;text-align:left;background:transparent;color:inherit;border:1px solid transparent;padding:.75rem; }
.privacy-nav button small { color:var(--text-secondary); }
.privacy-nav button:hover,.privacy-nav button[aria-pressed=true] { background:var(--surface-2);border-color:var(--border-strong); }
.privacy-main { display:grid;gap:1rem; }
.privacy-empty { padding:3rem 1rem;text-align:center;color:var(--text-secondary); }
.privacy-source { padding:.65rem .8rem;background:var(--surface-2);border-radius:.5rem;margin:0;overflow-wrap:anywhere; }
.privacy-settings { display:flex;align-items:end;gap:1rem;flex-wrap:wrap; }
.privacy-settings label { min-width:190px; }
.privacy-editors { display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1rem; }
.privacy-editor textarea { resize:vertical;min-height:280px;font-family:monospace;font-size:.85rem;line-height:1.4;background:var(--surface-2);color:var(--text);border:1px solid var(--border-strong);border-radius:.5rem;padding:.8rem; }
.privacy-toolbar { display:flex;gap:.5rem;align-items:center;flex-wrap:wrap; }
.privacy-stats { display:flex;gap:.6rem;flex-wrap:wrap; }
.privacy-stats span { padding:.6rem .75rem;border:1px solid var(--border);border-radius:.5rem; }
.privacy-filter input { flex:1 1 220px; }.privacy-filter select { flex:0 1 190px; }
.privacy-table-wrap { overflow:auto;max-height:440px;border:1px solid var(--border);border-radius:.5rem; }
.privacy-table-wrap table { width:100%;border-collapse:collapse; }
.privacy-table-wrap th,.privacy-table-wrap td { padding:.65rem;text-align:left;border-bottom:1px solid var(--border);vertical-align:top; }
.privacy-table-wrap code { overflow-wrap:anywhere;word-break:break-word; }
.privacy-danger-zone { padding:.9rem;border:1px solid var(--border-strong);border-radius:.5rem; }
.privacy-danger-zone summary { cursor:pointer;font-weight:600; }.privacy-danger-zone .privacy-toolbar { margin-top:.8rem; }
.privacy-path { margin-top:.2rem; }.sr-only { position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap; }
@media(max-width:1100px) { .privacy-editors { grid-template-columns:1fr; } }
@media(max-width:760px) {
  .privacy-policy-example-grid,.privacy-policy-example-picker { grid-template-columns:1fr; }
  .privacy-layout { grid-template-columns:1fr; }
  .privacy-head { flex-wrap:wrap; }
  .privacy-policy-row { grid-template-columns:minmax(0,1fr) minmax(0,1fr); }
  .privacy-policy-row button { grid-column:2;justify-self:end; }
}
</style>
