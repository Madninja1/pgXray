<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { computed, reactive, ref, watch } from "vue";
import { locale, t } from "../i18n/locale-service.js";
import InvestigationPanel from "./InvestigationPanel.vue";
import PrivacyPanel from "./PrivacyPanel.vue";
import type { PrivacyPrefill } from "./privacy.js";
import type { CaseNavigationTarget } from "./investigation.js";
import { resolveEvidenceRoute } from "./evidence-navigation.js";
import PlanPicker from "./PlanPicker.vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import { createToolsController, createToolsState, type ToolCase } from "./controller.js";
import { artifactRoles, canOpenPlan, isEditableSource, createArtifactsController, createArtifactsState, type ArtifactRole } from "./artifacts.js";

const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; quickSelection?: QuickConnectionSelection | null }>();
const emit = defineEmits<{ (e: "navigate-evidence", value: { connectionId: number; target: CaseNavigationTarget }): void }>();
const state = reactive(createToolsState());
const actions = createToolsController(props.shell, state, props.invoke);
const artifactState = reactive(createArtifactsState());
const artifactActions = createArtifactsController(props.shell, artifactState, props.invoke);
const fileName = ref("");
const caseDetails = ref("");
const caseDetailsEditing = ref(false);
const caseActionBusy = ref(false);
const caseActionError = ref("");
const fileKind = ref("sql");
const fileContent = ref("");
const importPath = ref("");
const importKind = ref("import");
const newFileRole = ref<ArtifactRole | "">("");
const importedFileRole = ref<ArtifactRole | "">("");
const createFileDialog = ref<HTMLDialogElement | null>(null);
const importFileDialog = ref<HTMLDialogElement | null>(null);
const choosingFile = ref(false);
const importDialogError = ref("");
const kindOptions = computed(() => [
  { value: "sql", label: t('tools.artifactSql') },
  { value: "explain", label: t('tools.artifactExplain') },
  { value: "text", label: t('tools.artifactText') },
  { value: "source", label: t('tools.artifactSource') },
  { value: "import", label: t('tools.artifactOther') },
] as const);
function suggestedKind(name: string, importing: boolean): string {
  const ext = name.trim().toLowerCase().split(".").pop();
  if (ext === "sql") return "sql";
  if (ext === "txt" || ext === "md") return "text";
  // По расширению .json нельзя установить, является ли файл планом EXPLAIN.
  return importing ? "import" : "source";
}
watch(fileName, value => { fileKind.value = suggestedKind(value, false); });
function openCreateFile(): void {
  fileName.value = ""; fileContent.value = ""; newFileRole.value = "";
  createFileDialog.value?.showModal();
}
function openImportFile(): void {
  importPath.value = ""; importKind.value = "import";
  importedFileRole.value = ""; importDialogError.value = "";
  importFileDialog.value?.showModal();
}
async function browseImportFile(): Promise<void> {
  if (choosingFile.value || artifactState.saving) return;
  choosingFile.value = true;
  importDialogError.value = "";
  try {
    const path = await props.invoke<string | null>("pick_case_import_file");
    if (path) {
      importPath.value = path;
      importKind.value = suggestedKind(path, true);
    }
  } catch (error) { importDialogError.value = String(error); }
  finally { choosingFile.value = false; }
}
const editingArtifactId = ref<number | null>(null);
const editingContent = ref("");
const editingBaseline = ref("");
const editableArtifact = computed(() => isEditableSource(selectedArtifact.value));
const planArtifact = computed(() => canOpenPlan(selectedArtifact.value));
const selectedArtifact = computed(() => artifactState.rows.find(row => row.artifact.id === artifactState.selectedId) ?? null);
const tab = ref<"cases" | "privacy" | "pev2">("cases");
const privacyPrefill = ref<PrivacyPrefill | null>(null);
const planPrefill = ref<{ caseId: number; artifactId: number; name: string } | null>(null);
function switchToolsTab(next: "cases" | "privacy" | "pev2"): void {
  if (tab.value === "privacy" && next !== "privacy") privacyPrefill.value = null;
  tab.value = next;
  if (next === "cases" && props.visible && props.shell.screen === "workspace")
    void actions.refreshConnections();
}
type CaseSection = "files" | "timeline" | "interventions" | "impact" | "comparisons" | "similar";
const caseSection = ref<CaseSection>("files");
const investigationSection = computed(() => caseSection.value === "files" ? "timeline" : caseSection.value);
const showCreateCase = ref(false);
const title = ref("");
const description = ref("");
const currentCase = computed(() => state.cases.find(item => item.id === state.caseId) ?? null);
watch(() => [props.shell.authGeneration, props.shell.screen] as const, () => {
  actions.reset();
  artifactActions.reset();
  if (props.visible && props.shell.screen === "workspace") void actions.refreshConnections();
});
watch(() => props.visible, visible => {
  if (visible && props.shell.screen === "workspace") void actions.refreshConnections();
}, { immediate: true });
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => state.connections.length, () => (state.loading || caseActionBusy.value || artifactState.saving)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!state.connections.some(connection => connection.id === request.connectionId)) return;
  if (state.loading || caseActionBusy.value || artifactState.saving) return;
  lastShortcutSerial = request.serial;
  if (state.connectionId !== request.connectionId) void actions.selectConnection(request.connectionId);
});
watch(() => state.caseId, caseId => {
  caseDetailsEditing.value = false;
  caseSection.value = "files";
  createFileDialog.value?.close();
  importFileDialog.value?.close();
  caseActionError.value = "";
  cancelEditing();
  artifactActions.reset(caseId);
  if (caseId !== null) void artifactActions.refresh(true);
});
watch(() => artifactState.selectedId, () => cancelEditing());
function cancelEditing(): void {
  editingArtifactId.value = null;
  editingContent.value = "";
  editingBaseline.value = "";
}
function beginEditing(): void {
  if (!editableArtifact.value || artifactState.preview === null || !selectedArtifact.value) return;
  editingArtifactId.value = selectedArtifact.value.artifact.id;
  editingContent.value = artifactState.preview;
  editingBaseline.value = artifactState.preview;
}
async function saveEditing(): Promise<void> {
  const id = editingArtifactId.value;
  if (id === null || editingContent.value === editingBaseline.value) return;
  if (await artifactActions.updateFile(id, editingContent.value)) cancelEditing();
}
async function addFile(): Promise<void> {
  const result = await artifactActions.createFileWithRole(fileName.value, fileContent.value, fileKind.value, newFileRole.value || null);
  if (result !== "failed") {
    createFileDialog.value?.close();
    if (result === "success") { fileName.value = ""; fileContent.value = ""; }
  }
}
async function importArtifact(): Promise<void> {
  const result = await artifactActions.importFileWithRole(importPath.value, importKind.value, importedFileRole.value || null);
  if (result !== "failed") {
    importFileDialog.value?.close();
    if (result === "success") importPath.value = "";
  }
}
function changeRole(artifactId: number, event: Event): void {
  const value = (event.target as HTMLSelectElement).value;
  void artifactActions.setRole(artifactId, value ? value as ArtifactRole : null);
}
async function deleteArtifact(artifactId: number): Promise<void> {
  if (await confirmAction(t('tools.deleteArtifactConfirm', { id: String(artifactId) })))
    void artifactActions.deleteArtifact(artifactId);
}
async function anonymizeArtifact(artifactId: number): Promise<void> {
  if (await confirmAction(t('tools.shareableConfirm'))) void artifactActions.anonymizeFile(artifactId);
}
function openArtifactInPlan(artifactId: number): void {
  const item = artifactState.rows.find(row => row.artifact.id === artifactId) ?? null;
  if (!currentCase.value || !item || item.artifact.case_id !== currentCase.value.id || !canOpenPlan(item)) {
    artifactState.error = t('tools.unavailablePlan', { id: String(artifactId) });
    caseSection.value = "files";
    return;
  }
  planPrefill.value = { caseId: currentCase.value.id, artifactId, name: item.artifact.original_name };
  switchToolsTab("pev2");
}
function openPlanInPrivacy(value: { input: string; scopeId: number | null; fileName: string }): void {
  privacyPrefill.value = { ...value, mode: "explain" };
  switchToolsTab("privacy");
}
function openArtifactInPrivacy(artifactId: number): void {
  const artifact = selectedArtifact.value;
  if (!artifact || artifact.artifact.id !== artifactId || artifactState.preview === null || !currentCase.value) return;
  const kind = artifact.artifact.kind;
  privacyPrefill.value = {
    input: artifactState.preview,
    mode: kind === "sql" ? "sql" : kind === "explain" ? "explain" : "text",
    scopeId: currentCase.value.scope_id,
    fileName: artifact.artifact.original_name,
  };
  switchToolsTab("privacy");
}
async function createCase(): Promise<void> {
  if (await actions.createCase(title.value, description.value)) { title.value = ""; description.value = ""; showCreateCase.value = false; }
}
async function updateCaseDescription(): Promise<void> {
  if (!currentCase.value || caseActionBusy.value) return;
  const caseId = currentCase.value.id;
  const auth = props.shell.authGeneration;
  caseActionBusy.value = true; caseActionError.value = "";
  try {
    const updated = await props.invoke<ToolCase>("update_investigation_case_description", { caseId, description: caseDetails.value });
    if (props.shell.authGeneration !== auth || state.caseId !== caseId || !updated) return;
    state.cases = state.cases.map(item => item.id === caseId ? { ...item, description: updated.description } : item);
    caseDetailsEditing.value = false;
  } catch (error) { caseActionError.value = String(error); }
  finally { caseActionBusy.value = false; }
}
async function deleteCase(): Promise<void> {
  const item = currentCase.value;
  if (!item || caseActionBusy.value || !confirmAction(t('tools.deleteCaseConfirm', { id: String(item.id), title: item.title }))) return;
  caseActionBusy.value = true; caseActionError.value = "";
  const auth = props.shell.authGeneration;
  try {
    await props.invoke("delete_investigation_case", { caseId: item.id });
    if (props.shell.authGeneration !== auth || state.caseId !== item.id) return;
    state.caseId = null;
    await actions.refreshConnections();
  } catch (error) { caseActionError.value = String(error); }
  finally { caseActionBusy.value = false; }
}
function selectSimilarCase(caseId: number): void {
  if (state.cases.some(item => item.id === caseId)) state.caseId = caseId;
  else caseActionError.value = t('tools.caseMissingConnection');
}
function navigateEvidence(target: CaseNavigationTarget): void {
  if (!currentCase.value) return;
  const route = resolveEvidenceRoute(currentCase.value.connection_id, target);
  if (route.view === "artifact") { selectEvidenceArtifact(route.artifactId); return; }
  emit("navigate-evidence", { connectionId: currentCase.value.connection_id, target });
}
function selectEvidenceArtifact(artifactId: number): void {
  if (artifactState.rows.some(item => item.artifact.id === artifactId)) {
    caseSection.value = "files";
    void artifactActions.select(artifactId);
  } else {
    artifactState.error = t('tools.missingArtifact', { id: String(artifactId) });
    caseSection.value = "files";
  }
}
async function toggleArtifact(artifactId: number): Promise<void> {
  if (editingArtifactId.value === artifactId && editingContent.value !== editingBaseline.value) {
    if (!confirmAction(t('tools.discardEdit'))) return;
  }
  void artifactActions.select(artifactState.selectedId === artifactId ? null : artifactId);
}
function dateLabel(value: number): string {
  return value ? new Date(value).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US") : t('tools.unknownDate');
}
</script>

<template>
  <section class="tools-root">
    <header class="tools-header">
      <div>
        <h1>{{ t('tools.pageTitle') }}</h1>
        <p class="tools-hint">{{ t('tools.pageHint') }}</p>
      </div>
      <nav class="tools-tabs" :aria-label="t('tools.tabsAria')">
        <button v-for="item in (['cases', 'privacy', 'pev2'] as const)" :key="item" type="button"
          class="secondary" :aria-pressed="tab === item" @click="switchToolsTab(item)">
          {{ { cases: t('tools.navCases'), privacy: t('tools.navPrivacy'), pev2: t('tools.navPlans') }[item] }}
        </button>
      </nav>
    </header>

    <template v-if="tab === 'cases'">
      <div class="tools-toolbar tools-connection-toolbar">
        <label>{{ t('tools.connection') }}
          <select :value="state.connectionId ?? ''" :disabled="state.loading"
            @change="actions.selectConnection(Number(($event.target as HTMLSelectElement).value) || null)">
            <option value="">{{ t('tools.chooseConnection') }}</option>
            <option v-for="item in state.connections" :key="item.id" :value="item.id">{{ item.name }}</option>
          </select>
        </label>
      </div>
      <p v-if="state.error" role="alert" class="tools-error">{{ state.error }}</p>
      <p v-if="state.notice" role="status" class="tools-status">{{ state.notice }}</p>
      <div class="tools-workspace">
        <aside class="tools-case-sidebar" :aria-label="t('tools.caseListAria')">
          <div class="tools-side-head">
            <h2>{{ t('tools.cases') }} <span class="tools-count">{{ state.cases.length }}</span></h2>
            <button type="button" class="secondary" :aria-expanded="showCreateCase" :disabled="!state.connectionId"
              @click="showCreateCase = !showCreateCase">{{ showCreateCase ? t('tools.tabCancel') : t('tools.tabCreate') }}</button>
          </div>
          <form v-if="showCreateCase" class="tools-compact-form" @submit.prevent="createCase">
            <label>{{ t('tools.name') }} <input v-model="title" maxlength="200" required :disabled="state.loading" /></label>
            <label>{{ t('tools.description') }} <textarea v-model="description" maxlength="8000" rows="3" :disabled="state.loading" /></label>
            <label>Privacy scope
              <select v-model.number="state.scopeId" :disabled="state.loading || !state.scopes.length">
                <option :value="null">{{ t('tools.chooseScope') }}</option>
                <option v-for="scope in state.scopes" :key="scope.id" :value="scope.id">{{ scope.name }}</option>
              </select>
            </label>
            <button type="submit" :disabled="state.loading || !state.connectionId || !state.scopeId">{{ t('tools.createCase') }}</button>
          </form>
          <div class="tools-case-list">
            <button v-for="item in state.cases" :key="item.id" type="button" class="tools-case-item"
              :aria-pressed="item.id === state.caseId" @click="state.caseId = item.id">
              <strong>{{ item.title }}</strong>
              <small>#{{ item.id }} · {{ item.kind }} · {{ dateLabel(item.created_at_ms) }}</small>
            </button>
            <p v-if="!state.loading && !state.cases.length" class="tools-hint">{{ t('tools.noCases') }}</p>
          </div>
        </aside>

        <main class="tools-case-main">
          <template v-if="currentCase">
            <header class="tools-case-header">
              <div class="tools-title-block">
                <h2>{{ currentCase.title }}</h2>
                <p class="tools-hint">{{ t('tools.caseNumber', { id: String(currentCase.id) }) }} · {{ currentCase.status }} · privacy scope #{{ currentCase.scope_id }}</p>
                <p v-if="!caseDetailsEditing" class="tools-description">{{ currentCase.description || t('tools.noDescription') }}</p>
              </div>
              <div class="tools-toolbar tools-case-actions">
                <button type="button" class="secondary" @click="caseDetails = currentCase.description; caseDetailsEditing = !caseDetailsEditing">{{ caseDetailsEditing ? t('tools.closeButton') : t('tools.descriptionButton') }}</button>
                <button type="button" class="danger" :disabled="caseActionBusy" @click="deleteCase">{{ t('tools.deleteCase') }}</button>
              </div>
            </header>
            <p v-if="caseActionError" role="alert" class="tools-error">{{ caseActionError }}</p>
            <div v-if="caseDetailsEditing" class="tools-description-editor">
              <label>{{ t('tools.caseDescription') }} <textarea v-model="caseDetails" maxlength="8000" rows="3" /></label>
              <button type="button" :disabled="caseActionBusy" @click="updateCaseDescription">{{ t('tools.saveDescription') }}</button>
            </div>
            <nav class="tools-case-navigation" :aria-label="t('tools.caseNavAria')">
              <button v-for="item in ([
                { id: 'files', label: t('tools.navFiles') },
                { id: 'timeline', label: t('tools.navTimeline') },
                { id: 'interventions', label: t('tools.navInterventions') },
                { id: 'impact', label: t('tools.navEffect') },
                { id: 'comparisons', label: t('tools.navComparisons') },
                { id: 'similar', label: t('tools.navSimilar') }
              ] as const)" :key="item.id" type="button" class="tools-section-button"
                :aria-pressed="caseSection === item.id" @click="caseSection = item.id">{{ item.label }}</button>
            </nav>

            <section v-if="caseSection === 'files'" class="tools-pane" :aria-label="t('tools.filesAria')">
              <div class="tools-pane-header">
                <div><h3>{{ t('tools.filesTitle') }} <span class="tools-count">{{ artifactState.rows.length }}</span></h3>
                  <p class="tools-hint">{{ t('tools.filesHint') }}</p>
                </div>
                <div class="tools-toolbar">
                  <button type="button" class="secondary" :disabled="artifactState.loading || artifactState.saving"
                    :title="t('tools.refreshKnownTitle')"
                    @click="artifactActions.refresh(false, true)">{{ t('tools.refreshList') }}</button>
                  <button type="button" class="secondary" :disabled="artifactState.loading || artifactState.saving"
                    :title="t('tools.scanInboxTitle')"
                    @click="artifactActions.refresh(true, true)">{{ t('tools.scanInbox') }}</button>
                </div>
              </div>
              <p v-if="artifactState.error" role="alert" class="tools-error">{{ artifactState.error }}</p>
              <p v-if="artifactState.notice" role="status" class="tools-status">{{ artifactState.notice }}</p>
              <p v-if="artifactState.loading" role="status" class="tools-hint">{{ t('tools.loadingFiles') }}</p>
              <div class="tools-file-list">
                <div v-for="row in artifactState.rows" :key="row.artifact.id" class="tools-file-row">
                  <button type="button" class="tools-file-select" :aria-expanded="artifactState.selectedId === row.artifact.id"
                    :aria-pressed="artifactState.selectedId === row.artifact.id" @click="toggleArtifact(row.artifact.id)">
                    <strong>{{ row.artifact.original_name }}</strong>
                    <small>#{{ row.artifact.id }} · {{ row.artifact.kind }} · {{ row.artifact.visibility }} · {{ row.artifact.size_bytes }} {{ t('tools.bytes') }}
                      <span v-if="row.derivation"> · {{ row.stale ? 'STALE' : 'derived' }} ({{ t('tools.derivedSource', { id: String(row.derivation.source_artifact_id) }) }})</span>
                    </small>
                  </button>
                  <label class="tools-role-label">{{ t('tools.role') }}
                    <select :value="row.role ?? ''" :disabled="artifactState.saving" @change="changeRole(row.artifact.id, $event)">
                      <option value="">{{ t('tools.noRole') }}</option>
                      <option v-for="role in artifactRoles" :key="role" :value="role">{{ role }}</option>
                    </select>
                  </label>
                </div>
                <p v-if="!artifactState.loading && !artifactState.rows.length" class="tools-hint">{{ t('tools.filesEmpty') }}</p>
              </div>
              <section v-if="selectedArtifact" class="tools-artifact-viewer">
                <header class="tools-pane-header">
                  <div><h4>{{ selectedArtifact.artifact.original_name }}</h4>
                    <p class="tools-hint">{{ selectedArtifact.artifact.relative_path }}</p>
                  </div>
                  <button type="button" class="secondary" @click="toggleArtifact(selectedArtifact.artifact.id)">{{ t('tools.closePreview') }}</button>
                </header>
                <p v-if="selectedArtifact.derivation" class="tools-hint">{{ t('tools.derivedFrom', { id: String(selectedArtifact.derivation.source_artifact_id) }) }} {{ selectedArtifact.stale ? t('tools.sourceStale') : t('tools.sourceCurrent') }}</p>
                <template v-if="editingArtifactId === selectedArtifact.artifact.id">
                  <label>{{ t('tools.fileContent') }} <textarea v-model="editingContent" class="tools-editor" rows="12" :disabled="artifactState.saving" /></label>
                  <div class="tools-toolbar"><button type="button" :disabled="artifactState.saving || editingContent === editingBaseline" @click="saveEditing">{{ t('tools.save') }}</button><button type="button" class="secondary" @click="cancelEditing">{{ t('tools.cancel') }}</button></div>
                  <p class="tools-hint">{{ t('tools.sourceWarning') }}</p>
                </template>
                <template v-else>
                  <pre v-if="artifactState.preview !== null" class="tools-preview">{{ artifactState.preview }}</pre>
                  <p v-else class="tools-hint">{{ artifactState.loading ? t('tools.tabLoading') : t('tools.tabPreviewEmpty') }}</p>
                  <div class="tools-toolbar tools-file-actions">
                    <button type="button" class="secondary" :disabled="!editableArtifact || artifactState.preview === null || artifactState.saving" @click="beginEditing">{{ t('tools.edit') }}</button>
                    <button type="button" class="secondary" :disabled="artifactState.saving" @click="artifactActions.exportFile(selectedArtifact.artifact.id)">{{ t('tools.export') }}</button>
                    <button type="button" class="secondary" :disabled="artifactState.saving" @click="artifactActions.revealFile(selectedArtifact.artifact.id)">{{ t('tools.showInFolder') }}</button>
                    <button type="button" class="secondary" :disabled="artifactState.saving" @click="artifactActions.copyFile(selectedArtifact.artifact.id)">{{ t('tools.copy') }}</button>
                    <button v-if="selectedArtifact.artifact.visibility === 'sensitive'" type="button" class="secondary" :disabled="artifactState.saving" @click="openArtifactInPrivacy(selectedArtifact.artifact.id)">{{ t('tools.openInPrivacy') }}</button>
                    <button v-if="selectedArtifact.artifact.visibility === 'sensitive'" type="button" class="secondary" :disabled="artifactState.saving" @click="anonymizeArtifact(selectedArtifact.artifact.id)">{{ t('tools.makeShareable') }}</button>
                    <button v-if="planArtifact" type="button" class="secondary" :disabled="artifactState.saving" @click="openArtifactInPlan(selectedArtifact.artifact.id)">{{ t('tools.analyzePlan') }}</button>
                    <button type="button" class="danger" :disabled="artifactState.saving" @click="deleteArtifact(selectedArtifact.artifact.id)">{{ t('tools.delete') }}</button>
                  </div>
                </template>
              </section>
              <div class="tools-add-actions">
                <button type="button" class="secondary" :disabled="artifactState.saving" @click="openCreateFile">{{ t('tools.createFileButton') }}</button>
                <button type="button" class="secondary" :disabled="artifactState.saving" @click="openImportFile">{{ t('tools.importFile') }}</button>
              </div>
              <dialog ref="createFileDialog" class="tools-file-dialog" aria-labelledby="tools-new-file-title" @cancel="artifactState.saving && $event.preventDefault()">
                <form class="tools-dialog-content" @submit.prevent="addFile">
                  <div class="tools-dialog-head"><h3 id="tools-new-file-title">{{ t('tools.newFile') }}</h3>
                    <button type="button" class="secondary" :disabled="artifactState.saving" @click="createFileDialog?.close()">{{ t('tools.close') }}</button></div>
                  <label>{{ t('tools.fileName') }} <input v-model="fileName" required maxlength="200" placeholder="query.sql" :disabled="artifactState.saving" /></label>
                  <div class="tools-dialog-grid">
                    <label>{{ t('tools.type') }}
                      <select v-model="fileKind" :disabled="artifactState.saving">
                        <option v-for="option in kindOptions" :key="option.value" :value="option.value">{{ option.label }}</option>
                      </select>
                    </label>
                    <label>{{ t('tools.roleInCase') }}
                      <select v-model="newFileRole" :disabled="artifactState.saving">
                        <option value="">{{ t('tools.noRole') }}</option>
                        <option v-for="role in artifactRoles" :key="role" :value="role">{{ role }}</option>
                      </select>
                    </label>
                  </div>
                  <label>{{ t('tools.content') }} <textarea v-model="fileContent" class="tools-dialog-editor" rows="12" :disabled="artifactState.saving" /></label>
                  <p class="tools-hint">{{ t('tools.createLimit') }}</p>
                  <div class="tools-dialog-actions"><button type="button" class="secondary" :disabled="artifactState.saving" @click="createFileDialog?.close()">{{ t('tools.cancel') }}</button>
                    <button type="submit" :disabled="artifactState.saving || !fileName.trim()">{{ artifactState.saving ? t('tools.tabCreating') : t('tools.tabCreateFile') }}</button></div>
                </form>
              </dialog>
              <dialog ref="importFileDialog" class="tools-file-dialog" aria-labelledby="tools-import-file-title" @cancel="(choosingFile || artifactState.saving) && $event.preventDefault()">
                <form class="tools-dialog-content" @submit.prevent="importArtifact">
                  <div class="tools-dialog-head"><h3 id="tools-import-file-title">{{ t('tools.importTitle') }}</h3>
                    <button type="button" class="secondary" :disabled="choosingFile || artifactState.saving" @click="importFileDialog?.close()">{{ t('tools.close') }}</button></div>
                  <button type="button" class="secondary tools-file-browse" :disabled="choosingFile || artifactState.saving" @click="browseImportFile">{{ choosingFile ? t('tools.openingDialog') : t('tools.selectFileButton') }}</button>
                  <label>{{ t('tools.chooseFile') }}
                    <input v-model="importPath" :placeholder="t('tools.importPathPlaceholder')" required :disabled="artifactState.saving || choosingFile" />
                  </label>
                  <p v-if="importDialogError" class="tools-error" role="alert">{{ importDialogError }}</p>
                  <div class="tools-dialog-grid">
                    <label>{{ t('tools.type') }}
                      <select v-model="importKind" :disabled="artifactState.saving">
                        <option v-for="option in kindOptions" :key="option.value" :value="option.value">{{ option.label }}</option>
                      </select>
                    </label>
                    <label>{{ t('tools.roleInCase') }}
                      <select v-model="importedFileRole" :disabled="artifactState.saving">
                        <option value="">{{ t('tools.noRole') }}</option>
                        <option v-for="role in artifactRoles" :key="role" :value="role">{{ role }}</option>
                      </select>
                    </label>
                  </div>
                  <p class="tools-hint">{{ t('tools.importHint') }}</p>
                  <div class="tools-dialog-actions"><button type="button" class="secondary" :disabled="artifactState.saving || choosingFile" @click="importFileDialog?.close()">{{ t('tools.cancel') }}</button>
                    <button type="submit" :disabled="artifactState.saving || choosingFile || !importPath.trim()">{{ artifactState.saving ? t('tools.tabImporting') : t('tools.tabImport') }}</button></div>
                </form>
              </dialog>
            </section>
            <InvestigationPanel v-else :key="currentCase.id" :shell="shell" :invoke="invoke" :active-case="currentCase"
              :artifacts="artifactState.rows" :section="investigationSection" :visible="visible && tab === 'cases'"
              @select-case="selectSimilarCase" @select-artifact="selectEvidenceArtifact" @open-plan="openArtifactInPlan" @navigate-evidence="navigateEvidence"
              @artifacts-changed="artifactActions.refresh(false)" />
          </template>
          <div v-else class="tools-empty-state"><h2>{{ t('tools.selectCase') }}</h2><p class="tools-hint">{{ t('tools.selectCaseHint') }}</p></div>
        </main>
      </div>
    </template>
    <!-- Keep unsaved privacy input/output in memory across Tools tabs. -->
    <PrivacyPanel v-show="tab === 'privacy'" :shell="shell" :invoke="invoke" :connection-id="state.connectionId" :visible="visible && tab === 'privacy'" :prefill="privacyPrefill" />
    <!-- Keep the analysis in memory while visiting Cases or Privacy. -->
    <PlanPicker v-show="tab === 'pev2'" :shell="shell" :invoke="invoke" :cases="state.cases" :visible="visible && tab === 'pev2'" :prefill="planPrefill" @open-privacy="openPlanInPrivacy" />
  </section>
</template>

<style scoped>
.tools-root { padding: 1.25rem; min-width: 0; }
.tools-header, .tools-case-header, .tools-pane-header, .tools-side-head { display:flex; align-items:flex-start; justify-content:space-between; gap:1rem; }
.tools-header { align-items:center; margin-bottom: 1rem; }
.tools-header h1 { margin:0; }
.tools-header p { margin:.3rem 0 0; }
.tools-header .tools-tabs { margin:0; }
.tools-tabs, .tools-toolbar, .tools-add-actions { display:flex; align-items:center; gap:.5rem; flex-wrap:wrap; }
.tools-tabs [aria-pressed="true"], .tools-section-button[aria-pressed="true"] { border-color:var(--border-strong); background:var(--surface-2); font-weight:700; }
.tools-connection-toolbar { margin:0 0 1rem; }
.tools-connection-toolbar label { display:flex; align-items:center; gap:.75rem; }
.tools-hint { color:var(--text-secondary); overflow-wrap:anywhere; font-size:.9rem; }
.tools-error { color:var(--error); overflow-wrap:anywhere; }
.tools-status { color:var(--text-secondary); overflow-wrap:anywhere; }
.tools-workspace { display:grid; grid-template-columns:minmax(240px, 290px) minmax(0, 1fr); gap:1rem; align-items:start; }
.tools-case-sidebar, .tools-case-main { min-width:0; border:1px solid var(--border); border-radius:.75rem; background:var(--surface); }
.tools-case-sidebar { padding:.9rem; position:sticky; top:.5rem; max-height:calc(100vh - 7rem); overflow-y:auto; }
.tools-side-head { align-items:center; }
.tools-side-head h2 { margin:0; font-size:1.15rem; }
.tools-side-head button { white-space:nowrap; }
.tools-count { display:inline-block; font-size:.78rem; font-weight:600; opacity:.8; border:1px solid var(--border); border-radius:999px; padding:.05rem .45rem; vertical-align:middle; }
.tools-case-list { display:grid; gap:.4rem; margin-top:.8rem; }
.tools-case-item { display:grid; gap:.25rem; text-align:left; width:100%; padding:.75rem; background:transparent; border:1px solid transparent; border-radius:.5rem; color:inherit; cursor:pointer; overflow-wrap:anywhere; }
.tools-case-item:hover, .tools-case-item[aria-pressed="true"] { background:var(--surface-2); border-color:var(--border-strong); }
.tools-case-item small, .tools-file-select small { color:var(--text-secondary); font-size:.78rem; }
.tools-case-main { padding:1rem 1.15rem; }
.tools-case-header h2 { font-size:1.2rem; margin:0 0 .35rem; overflow-wrap:anywhere; }
.tools-title-block { min-width:0; }
.tools-title-block p { margin:.25rem 0 .45rem; }
.tools-description { overflow-wrap:anywhere; }
.tools-case-actions { flex-shrink:0; }
.tools-case-navigation { display:flex; overflow-x:auto; gap:.35rem; border-bottom:1px solid var(--border); padding:1rem 0 .5rem; margin-bottom:1rem; }
.tools-section-button { white-space:nowrap; flex:0 0 auto; border-radius:.5rem; padding:.6rem .75rem; background:transparent; border:1px solid transparent; color:inherit; cursor:pointer; }
.tools-pane { min-width:0; }
.tools-pane-header { align-items:center; margin-bottom:.85rem; }
.tools-pane-header h3, .tools-pane-header h4 { margin:0 0 .3rem; }
.tools-pane-header p { margin:0; }
.tools-file-list { display:grid; gap:.35rem; }
.tools-file-row { display:grid; grid-template-columns:minmax(0,1fr) 135px; gap:.5rem; align-items:center; border-bottom:1px solid var(--border); padding:.3rem 0; }
.tools-file-select { width:100%; display:grid; gap:.3rem; text-align:left; border:1px solid transparent; border-radius:.5rem; padding:.65rem .8rem; background:transparent; color:inherit; cursor:pointer; overflow-wrap:anywhere; }
.tools-file-select[aria-pressed="true"], .tools-file-select:hover { background:var(--surface-2); border-color:var(--border-strong); }
.tools-role-label { display:grid; gap:.2rem; font-size:.78rem; color:var(--text-secondary); }
.tools-role-label select { width:100%; }
.tools-artifact-viewer { margin-top:.75rem; border:1px solid var(--border); border-radius:.6rem; padding:1rem; min-width:0; }
.tools-preview { max-height:390px; overflow:auto; white-space:pre; font-size:.83rem; background:var(--surface-2); border:1px solid var(--border); padding:.8rem; border-radius:.5rem; }
.tools-file-actions { margin-top:.8rem; }
.tools-add-actions { border-top:1px solid var(--border); margin-top:1rem; padding-top:1rem; }
.tools-compact-form, .tools-description-editor { display:grid; gap:.65rem; margin-top:1rem; padding:1rem; border:1px solid var(--border); border-radius:.6rem; }
.tools-compact-form h4 { margin:0; }
.tools-compact-form label, .tools-description-editor label, .tools-editor { display:grid; gap:.35rem; min-width:0; }
.tools-compact-form input, .tools-compact-form textarea, .tools-compact-form select, .tools-description-editor textarea, .tools-editor, .tools-role-label select { box-sizing:border-box; width:100%; background:var(--surface-2); color:var(--text); border:1px solid var(--border-strong); border-radius:.4rem; padding:.6rem; }
.tools-empty-state { padding:2rem; text-align:center; }
.tools-file-dialog { box-sizing:border-box; width:min(760px, calc(100vw - 2rem)); max-height:calc(100vh - 3rem); overflow:auto; border:1px solid var(--border-strong); border-radius:.85rem; padding:0; background:var(--surface); color:var(--text); box-shadow:0 18px 60px rgba(0,0,0,.38); }
.tools-file-dialog::backdrop { background:rgba(0,0,0,.62); }
.tools-dialog-content { display:grid; gap:1rem; padding:1.4rem; max-width:none; width:100%; box-sizing:border-box; margin:0; }
.tools-dialog-content label { display:grid; gap:.4rem; min-width:0; }
.tools-dialog-content input, .tools-dialog-content select, .tools-dialog-content textarea { box-sizing:border-box; width:100%; color:var(--text); background:var(--surface-2); border:1px solid var(--border-strong); border-radius:.45rem; padding:.7rem; }
.tools-dialog-head, .tools-dialog-actions { display:flex; justify-content:space-between; align-items:center; gap:.7rem; }
.tools-dialog-head h3 { margin:0; }
.tools-dialog-grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:1rem; }
.tools-dialog-editor { resize:vertical; min-height:200px; font-family:monospace; }
.tools-dialog-actions { justify-content:flex-end; }
.tools-file-browse { width:100%; padding:1rem; border-style:dashed; }
@media (max-width:560px) { .tools-dialog-grid { grid-template-columns:1fr; } .tools-dialog-content { padding:1rem; } }
@media (max-width: 1020px) { .tools-workspace { grid-template-columns:minmax(200px, 240px) minmax(0,1fr); } .tools-header { flex-wrap:wrap; } .tools-case-header { flex-wrap:wrap; } }
@media (max-width: 760px) { .tools-workspace { grid-template-columns:1fr; } .tools-case-sidebar { position:static; max-height:none; } .tools-pane-header { flex-wrap:wrap; } }
</style>
