<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import UiIcon from "../../components/UiIcon.vue";
import { t } from "../i18n/locale-service.js";
import { closeOpenDetails, dismissOpenDetailsOutside } from "../../core/disclosure-menu.js";
import { loadPinnedProjectIds, savePinnedProjectIds, sortProjectGroups, type ProjectSort } from "./project-view-preferences.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import {
  activeSessionsForConnection,
  changesActiveSessionTarget,
  createConnectionsController,
  createConnectionsState,
  formatProbeTimestamp,
  parseSavedConnections,
  savedPasswordTargetChanged,
  type ConnectionField,
  type SavedConnection,
  type SessionSummary,
} from "./controller.js";
import {
  createConnectionProjectsController, createConnectionProjectsState,
  validateProjectName, type ConnectionProject,
} from "./projects-controller.js";

const props = defineProps<{
  shell: ShellState;
  invoke: PgXrayInvoke;
  shellBusy?: boolean;
  /** Integration is deferred until the tracked Sessions module is migrated. */
  sessions?: readonly SessionSummary[];
  openSession?: (id: number, forceNew: boolean) => Promise<void>;
}>();
const emit = defineEmits<{
  (event: "connections-changed", connections: SavedConnection[]): void;
}>();

const state = reactive(createConnectionsState());
const shortcutSavingId = ref<number | null>(null);
async function setShortcut(connection: SavedConnection, raw: string): Promise<void> {
  if (shortcutSavingId.value !== null || working.value) return;
  const slot = raw === "" ? null : Number(raw);
  if (slot !== null && (!Number.isInteger(slot) || slot < 0 || slot > 9)) return;
  shortcutSavingId.value = connection.id;
  try {
    const rows = parseSavedConnections(await props.invoke<unknown>("assign_connection_shortcut", { connectionId: connection.id, slot }));
    state.items = rows;
    emit("connections-changed", rows);
    notice.value = slot === null ? t("connections.shortcutRemoved", { name: connection.name }) : t("connections.shortcutAssigned", { slot: String(slot), name: connection.name });
    state.error = null;
  } catch (error) { state.error = t("connections.shortcutError", { error: String(error) }); }
  finally { shortcutSavingId.value = null; }
}

const notice = ref("");
let noticeTimer: ReturnType<typeof setTimeout> | null = null;
watch(notice, value => {
  if (noticeTimer !== null) clearTimeout(noticeTimer);
  noticeTimer = value ? setTimeout(() => { notice.value = ""; }, 4500) : null;
});
const sessionPending = ref<number[]>([]);
const editorNameField = ref<HTMLInputElement | null>(null);
const editorDialog = ref<HTMLDialogElement | null>(null);
const editorStatusElement = ref<HTMLParagraphElement | null>(null);
const showDraftPassword = ref(false);
// Use a WebView-local dialog: the browser confirmation API is blocked in this Tauri build.
// In particular, never silently authorize reuse of a stored PostgreSQL password.
const confirmationDialog = ref<HTMLDialogElement | null>(null);
const confirmationMessage = ref("");
let resolveConfirmation: ((approved: boolean) => void) | null = null;
function finishConfirmation(approved: boolean): void {
  const resolve = resolveConfirmation;
  resolveConfirmation = null;
  if (confirmationDialog.value?.open) confirmationDialog.value.close();
  confirmationMessage.value = "";
  resolve?.(approved);
}
async function requestConfirmation(message: string): Promise<boolean> {
  if (resolveConfirmation) return false;
  confirmationMessage.value = message;
  return new Promise<boolean>(resolve => {
    resolveConfirmation = resolve;
    void nextTick().then(() => {
      if (!resolveConfirmation) return;
      const dialog = confirmationDialog.value;
      if (!dialog) { finishConfirmation(false); return; }
      try { dialog.showModal(); } catch { finishConfirmation(false); }
    });
  });
}
const controller = createConnectionsController(props.shell, state, props.invoke, {
  onConnectionsChanged: (connections) => emit("connections-changed", connections),
  confirmPasswordReuse: (existing, draft, action) => requestConfirmation(
    t("connections.reusePasswordConfirm", {
      name: existing.name,
      previous: `${existing.username}@${existing.host}:${existing.port}/${existing.database} (${existing.sslmode})`,
      next: `${draft.username}@${draft.host}:${draft.port}/${draft.database} (${draft.sslmode})`,
      action: t(action === "probe" ? "connections.checkVerb" : "connections.saveVerb"),
    }),
  ),
});

const projectState = reactive(createConnectionProjectsState());
const projectController = createConnectionProjectsController(
  props.shell, projectState, props.invoke, () => controller.refresh(),
);
const projectDialog = ref<HTMLDialogElement | null>(null);
const projectName = ref("");
const projectEditingId = ref<number | null>(null);
const projectSort = ref<ProjectSort>("name-asc");
const pinnedProjectIds = ref<number[]>([]);
const expandedGroups = reactive<Record<string, boolean>>({});
const connectionList = ref<HTMLElement | null>(null);
const projectGroups = computed(() => sortProjectGroups([
  ...projectState.items.map(project => ({
    id: project.id, name: project.name,
    connections: state.items.filter(connection => connection.project_id === project.id),
  })),
  {
    id: null as number | null, name: t("connections.noProject"),
    connections: state.items.filter(connection => connection.project_id == null ||
      !projectState.items.some(project => project.id === connection.project_id)),
  },
], projectSort.value));
function groupKey(id: number | null): string { return id === null ? "ungrouped" : `project-${id}`; }
function isPinned(id: number | null): boolean { return id !== null && pinnedProjectIds.value.includes(id); }
function groupExpanded(id: number | null): boolean {
  // Ungrouped connections stay visible by default; named projects start closed.
  if (id === null) return expandedGroups.ungrouped !== false;
  return isPinned(id) || expandedGroups[groupKey(id)] === true;
}
function toggleGroup(id: number | null): void {
  if (isPinned(id)) return; // Use the project menu to release an always-open group.
  const key = groupKey(id);
  expandedGroups[key] = !groupExpanded(id);
}
function projectPreferenceStorage(): Storage | null {
  try { return window.localStorage; } catch { return null; }
}
function loadProjectViewPreferences(): void {
  Object.keys(expandedGroups).forEach(key => { delete expandedGroups[key]; });
  projectSort.value = "name-asc";
  pinnedProjectIds.value = loadPinnedProjectIds(projectPreferenceStorage(), props.shell.currentUsername);
}
function togglePinnedProject(id: number): void {
  pinnedProjectIds.value = isPinned(id)
    ? pinnedProjectIds.value.filter(value => value !== id)
    : [...pinnedProjectIds.value, id];
  delete expandedGroups[groupKey(id)];
  savePinnedProjectIds(projectPreferenceStorage(), props.shell.currentUsername, pinnedProjectIds.value);
}
function onConnectionMenuToggle(event: Event): void {
  const opened = event.currentTarget as HTMLDetailsElement;
  if (!opened.open) return;
  connectionList.value?.querySelectorAll<HTMLDetailsElement>("details.connection-menu")
    .forEach(menu => { if (menu !== opened) menu.open = false; });
}
function onDocumentPointerDown(event: PointerEvent): void {
  dismissOpenDetailsOutside(connectionList.value, "details.connection-menu", event.target);
}
function onDocumentKeyDown(event: KeyboardEvent): void {
  if (event.key === "Escape") closeOpenDetails(connectionList.value, "details.connection-menu");
}
async function openProjectEditor(project: ConnectionProject | null = null): Promise<void> {
  if (projectState.mutating) return;
  projectEditingId.value = project?.id ?? null;
  projectName.value = project?.name ?? "";
  projectState.error = null;
  await nextTick();
  projectDialog.value?.showModal();
}
function onProjectBackdrop(event: MouseEvent): void {
  if (event.target === projectDialog.value) closeProjectEditor();
}
function closeProjectEditor(): void {
  if (projectState.mutating) return;
  projectDialog.value?.close();
  projectName.value = "";
  projectEditingId.value = null;
}
async function saveProject(): Promise<void> {
  const issue = validateProjectName(projectName.value);
  if (issue) { projectState.error = issue; return; }
  const editing = projectEditingId.value;
  const name = projectName.value.trim();
  const saved = editing === null
    ? await projectController.create(name)
    : await projectController.rename(editing, name);
  if (saved) {
    closeProjectEditor();
    notice.value = editing === null ? t("connections.projectCreated", { name }) : t("connections.projectRenamed", { name });
  }
}
function onProjectMenuAction(event: Event, action: "edit" | "delete" | "pin", id: number | null): void {
  const details = (event.currentTarget as HTMLElement).closest<HTMLDetailsElement>("details");
  if (details) details.open = false;
  const project = projectState.items.find(item => item.id === id);
  if (!project) return;
  if (action === "edit") void openProjectEditor(project);
  else if (action === "pin") togglePinnedProject(project.id);
  else void removeProject(project);
}
async function removeProject(project: ConnectionProject): Promise<void> {
  if (!await requestConfirmation(t("connections.deleteProjectConfirm", { name: project.name }))) return;
  if (await projectController.remove(project.id)) {
    pinnedProjectIds.value = pinnedProjectIds.value.filter(id => id !== project.id);
    savePinnedProjectIds(projectPreferenceStorage(), props.shell.currentUsername, pinnedProjectIds.value);
    delete expandedGroups[groupKey(project.id)];
    notice.value = t("connections.projectDeleted", { name: project.name });
  }
}
async function moveConnection(event: Event, connectionId: number): Promise<void> {
  const select = event.currentTarget as HTMLSelectElement;
  const projectId = select.value === "" ? null : Number(select.value);
  if (await projectController.move(connectionId, projectId)) notice.value = t("connections.movedToProject");
  else select.value = String(state.items.find(item => item.id === connectionId)?.project_id ?? "");
}

const editingConnection = computed(() => state.items.find((x) => x.id === state.editingId) ?? null);
const reusedPasswordOnNewTarget = computed(() =>
  state.editor !== null && savedPasswordTargetChanged(editingConnection.value, state.editor),
);
const editingActiveSessions = computed(() => editingConnection.value
  ? sessionsFor(editingConnection.value.id).length : 0);
const changingActiveTarget = computed(() => Boolean(editingConnection.value && state.editor &&
  editingActiveSessions.value > 0 && changesActiveSessionTarget(editingConnection.value, state.editor)));
const working = computed(() => Boolean(props.shellBusy || state.mutating || state.probingDraft));

function sessionsFor(id: number): SessionSummary[] {
  return activeSessionsForConnection(props.sessions ?? [], id);
}
function clearField(field: ConnectionField): void {
  delete state.fieldErrors[field];
  state.editorStatus = null;
}
function closeEditor(): void {
  if (!working.value) controller.closeEditor();
}
function onEditorCancel(event: Event): void {
  event.preventDefault();
  closeEditor();
}
function onEditorBackdrop(event: MouseEvent): void {
  if (event.target === editorDialog.value) closeEditor();
}
function onMenuAction(event: Event, action: "probe" | "edit" | "delete", id: number): void {
  const menu = (event.currentTarget as HTMLElement).closest<HTMLDetailsElement>("details.connection-menu");
  if (menu) menu.open = false;
  if (action === "probe") void probeSaved(id);
  else if (action === "edit") edit(id);
  else void removeSaved(id);
}
function createNew(): void {
  notice.value = "";
  controller.openNewEditor();
}
function edit(id: number): void {
  notice.value = "";
  controller.openEditEditor(id);
}
async function probeDraft(): Promise<void> {
  notice.value = "";
  await controller.probeDraft();
}
async function saveDraft(): Promise<void> {
  if (!state.editor) return;
  const wasEditing = state.editingId !== null;
  const name = state.editor.name.trim();
  notice.value = "";
  if (await controller.saveDraft()) {
    notice.value = wasEditing ? t("connections.updated") : t("connections.savedNamed", { name });
  }
}
async function probeSaved(id: number): Promise<void> {
  notice.value = "";
  await controller.probeSaved(id);
}
async function removeSaved(id: number): Promise<void> {
  notice.value = "";
  const item = state.items.find((x) => x.id === id);
  if (!item) return;
  if (await controller.deleteSaved(id, (connection) => requestConfirmation(
    t("connections.deleteConnectionConfirm", { name: connection.name }),
  ))) notice.value = t("connections.deletedNamed", { name: item.name });
}
async function connect(id: number, forceNew: boolean): Promise<void> {
  if (!props.openSession || sessionPending.value.includes(id)) return;
  sessionPending.value.push(id);
  try {
    await props.openSession(id, forceNew);
    notice.value = t("connections.sessionOpened", { id: String(id) });
  } catch (error) {
    state.error = String(error);
  } finally {
    sessionPending.value = sessionPending.value.filter((item) => item !== id);
  }
}
// An IPC read may complete before WebView paints even a single animation frame.
// Keep only the visual indicator visible briefly; the requests are not delayed.
const refreshingList = ref(false);
let refreshVisualTimer: ReturnType<typeof setTimeout> | null = null;
let refreshRun = 0;
function clearRefreshIndicator(): void {
  ++refreshRun;
  if (refreshVisualTimer !== null) clearTimeout(refreshVisualTimer);
  refreshVisualTimer = null;
  refreshingList.value = false;
}
function refresh(): void {
  if (refreshingList.value || state.loading || projectState.loading) return;
  const run = ++refreshRun;
  const startedAt = Date.now();
  refreshingList.value = true;
  void Promise.all([controller.refresh(), projectController.refresh()]).finally(() => {
    if (run !== refreshRun) return;
    const remaining = Math.max(0, 650 - (Date.now() - startedAt));
    refreshVisualTimer = setTimeout(() => {
      if (run !== refreshRun) return;
      refreshingList.value = false;
      refreshVisualTimer = null;
    }, remaining);
  });
}
watch(() => state.editorStatus, async status => {
  if (status?.kind !== "error" || !state.editor || !editorDialog.value?.open) return;
  await nextTick();
  editorStatusElement.value?.scrollIntoView({ block: "nearest" });
});
watch(() => state.editor, async editor => {
  showDraftPassword.value = false;
  await nextTick();
  const dialog = editorDialog.value;
  if (!dialog) return;
  if (editor) {
    if (!dialog.open) dialog.showModal();
    editorNameField.value?.focus();
  } else if (dialog.open) dialog.close();
});

onMounted(() => {
  loadProjectViewPreferences();
  refresh();
  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  document.addEventListener("keydown", onDocumentKeyDown);
});
watch(() => props.shell.authGeneration, () => {
  finishConfirmation(false);
  clearRefreshIndicator();
  controller.reset();
  projectController.reset();
  closeOpenDetails(connectionList.value, "details.connection-menu");
  loadProjectViewPreferences();
  if (projectDialog.value?.open) projectDialog.value.close();
  notice.value = "";
  sessionPending.value = [];
  if (props.shell.screen === "workspace") refresh();
});
onUnmounted(() => {
  finishConfirmation(false);
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  document.removeEventListener("keydown", onDocumentKeyDown);
  clearRefreshIndicator();
  if (noticeTimer !== null) clearTimeout(noticeTimer);
  if (editorDialog.value?.open) editorDialog.value.close();
  controller.reset();
  projectController.reset();
  if (projectDialog.value?.open) projectDialog.value.close();
  sessionPending.value = [];
});
</script>

<template>
  <section class="connections-view" aria-labelledby="connections-title">
    <header class="heading">
      <div>
        <h2 id="connections-title">{{ t("connections.title") }}</h2>
        <p class="hint">{{ t("connections.description") }}</p>
      </div>
      <div class="toolbar-actions">
        <button type="button" class="secondary icon-action" :disabled="working || refreshingList || state.loading || projectState.loading"
          :aria-busy="refreshingList || state.loading || projectState.loading"
          :aria-label="t('connections.refresh')" :title="t('connections.refresh')" @click="refresh">
          <span class="refresh-glyph" :class="{ spinning: refreshingList || state.loading || projectState.loading }" aria-hidden="true">
            <UiIcon name="refresh" :size="19" />
          </span>
        </button>
        <button type="button" class="secondary" :disabled="working || projectState.mutating" @click="openProjectEditor()">
          <UiIcon name="folder" :size="18" /> {{ t("connections.newProject") }}
        </button>
        <button type="button" class="add-action" :disabled="working" @click="createNew">
          <UiIcon name="plus" :size="18" /> {{ t("connections.add") }}
        </button>
      </div>
    </header>

    <div class="project-list-options">
      <label>{{ t("connections.projectSorting") }}
        <select v-model="projectSort" :aria-label="t('connections.projectSorting')">
          <option value="name-asc">{{ t("connections.sortNameAsc") }}</option>
          <option value="name-desc">{{ t("connections.sortNameDesc") }}</option>
          <option value="connections-desc">{{ t("connections.sortCount") }}</option>
        </select>
      </label>
      <span class="hint">{{ t("connections.projectSortHint") }}</span>
    </div>

    <p v-if="notice" class="notice toast" role="status" aria-live="polite">{{ notice }}</p>
    <p v-if="state.error && !state.editor" class="inline-status error" role="alert">{{ state.error }}</p>
    <p v-if="projectState.error && !projectDialog?.open" class="inline-status error" role="alert">{{ projectState.error }}</p>
    <p v-if="state.loading" class="hint" role="status">{{ t("connections.loading") }}</p>
    <div ref="connectionList" class="connection-list" :aria-busy="state.loading">
      <div v-if="!state.loading && state.items.length === 0 && projectState.items.length === 0" class="empty-state">
        <UiIcon name="database" :size="28" />
        <strong>{{ t("connections.emptyTitle") }}</strong>
        <span>{{ t("connections.emptyHint") }}</span>
      </div>
      <section v-for="group in projectGroups.filter(g => g.id !== null || g.connections.length > 0)"
        :key="groupKey(group.id)" class="project-group">
        <header class="project-heading">
          <button type="button" class="project-collapse" :aria-expanded="groupExpanded(group.id)"
            :disabled="isPinned(group.id)"
            :title="isPinned(group.id) ? t('connections.pinnedHint') : undefined"
            :aria-label="isPinned(group.id) ? t('connections.pinnedProjectLabel', { name: group.name }) : t(groupExpanded(group.id) ? 'connections.collapseProject' : 'connections.expandProject', { name: group.name })"
            @click="toggleGroup(group.id)">
            <span class="project-chevron">{{ groupExpanded(group.id) ? '▾' : '▸' }}</span>
            <UiIcon name="folder" :size="19" />
            <strong>{{ group.name }}</strong>
            <span class="project-count">{{ group.connections.length }}</span>
            <span v-if="isPinned(group.id)" class="project-pin-label">{{ t("connections.alwaysOpen") }}</span>
          </button>
          <details v-if="group.id !== null" class="connection-menu project-actions" @toggle="onConnectionMenuToggle">
            <summary :aria-label="t('connections.manageProject', { name: group.name })" :title="t('connections.manageProject', { name: group.name })">
              <UiIcon name="ellipsis" :size="19" />
            </summary>
            <div class="connection-menu-items">
              <button type="button" :disabled="projectState.mutating"
                @click="onProjectMenuAction($event, 'edit', group.id)">
                <UiIcon name="pencil" :size="16" /> {{ t("connections.rename") }}
              </button>
              <button type="button" :disabled="projectState.mutating"
                @click="onProjectMenuAction($event, 'pin', group.id)">
                <UiIcon name="pin" :size="16" /> {{ isPinned(group.id) ? t("connections.unpin") : t("connections.pin") }}
              </button>
              <button type="button" class="menu-danger" :disabled="projectState.mutating"
                @click="onProjectMenuAction($event, 'delete', group.id)">
                <UiIcon name="trash" :size="16" /> {{ t("connections.deleteProject") }}
              </button>
            </div>
          </details>
        </header>
        <div v-if="groupExpanded(group.id)" class="project-contents">
          <p v-if="group.connections.length === 0" class="project-empty">{{ t("connections.projectEmpty") }}</p>
      <article v-for="connection in group.connections" :key="connection.id" class="connection-card">
        <div class="connection-card-header">
          <div class="connection-identity">
            <span class="connection-emblem"><UiIcon name="database" :size="22" /></span>
            <div class="identity-text">
              <strong class="connection-title">{{ connection.name }}</strong>
              <span class="address">{{ connection.username }}@{{ connection.host }}:{{ connection.port }}/{{ connection.database }}</span>
            </div>
          </div>
          <details class="connection-menu" @toggle="onConnectionMenuToggle">
            <summary :aria-label="t('connections.connectionActions', { name: connection.name })" :title="t('connections.connectionActions', { name: connection.name })">
              <UiIcon name="ellipsis" :size="20" />
            </summary>
            <div class="connection-menu-items">
              <button v-if="props.openSession" type="button" :disabled="working || state.probingIds.includes(connection.id)"
                @click="onMenuAction($event, 'probe', connection.id)">
                <UiIcon name="refresh" :size="17" /> {{ t("connections.checkConnection") }}
              </button>
              <button type="button" :disabled="working" @click="onMenuAction($event, 'edit', connection.id)">
                <UiIcon name="pencil" :size="17" /> {{ t("connections.edit") }}
              </button>
              <label class="connection-move-label">{{ t("connections.moveToProject") }}
                <select :value="connection.project_id ?? ''" :disabled="working || projectState.mutating"
                  :aria-label="t('connections.connectionProject', { name: connection.name })" @change="moveConnection($event, connection.id)">
                  <option value="">{{ t("connections.noProject") }}</option>
                  <option v-for="project in projectState.items" :key="project.id" :value="project.id">{{ project.name }}</option>
                </select>
              </label>
              <button type="button" class="menu-danger" :disabled="working"
                @click="onMenuAction($event, 'delete', connection.id)">
                <UiIcon name="trash" :size="17" /> {{ t("connections.delete") }}
              </button>
            </div>
          </details>
        </div>
        <div class="connection-shortcut-row">
          <label :for="`quick-slot-${connection.id}`">{{ t("connections.shortcut") }}
            <select :id="`quick-slot-${connection.id}`" :value="connection.shortcut_slot ?? ''"
              :disabled="working || shortcutSavingId !== null"
              @change="setShortcut(connection, ($event.target as HTMLSelectElement).value)">
              <option value="">{{ t("connections.notAssigned") }}</option>
              <option v-for="slot in [1,2,3,4,5,6,7,8,9,0]" :key="slot" :value="slot">{{ slot }}</option>
            </select>
          </label>
          <small v-if="connection.shortcut_slot != null">{{ t("connections.shortcutHint", { slot: String(connection.shortcut_slot) }) }}</small>
        </div>
        <div class="connection-meta">
          <span class="meta-tag">{{ connection.sslmode === 'disable' ? t("connections.noTls") : `TLS: ${connection.sslmode}` }}</span>
          <span class="meta-tag">{{ connection.has_password ? t("connections.passwordSaved") : t("connections.noSavedPassword") }}</span>
          <span v-if="props.sessions" class="meta-tag">{{ t("connections.sessionsCount", { count: String(sessionsFor(connection.id).length) }) }}</span>
          <span v-if="state.probingIds.includes(connection.id)" class="meta-tag probe-meta pending" role="status">
            <UiIcon name="refresh" :size="14" class="spinning" /> {{ t("connections.checking") }}
          </span>
          <template v-else-if="state.lastProbeById[connection.id]">
            <span class="meta-tag probe-meta" :data-result="state.lastProbeById[connection.id]?.kind"
              :title="t('connections.lastProbeTooltip', { date: formatProbeTimestamp(state.lastProbeById[connection.id]!.checkedAt) })">
              <UiIcon :name="state.lastProbeById[connection.id]?.kind === 'success' ? 'shield' : 'x'" :size="14" />
              {{ state.lastProbeById[connection.id]?.kind === 'success' ? t("connections.checkSuccess") : t("connections.checkFailed") }}
            </span>
            <span v-if="state.lastProbeById[connection.id]?.serverVersion" class="meta-tag">
              PostgreSQL {{ state.lastProbeById[connection.id]?.serverVersion }}
            </span>
            <time class="probe-checked-at" :datetime="new Date(state.lastProbeById[connection.id]!.checkedAt).toISOString()">
              {{ t("connections.lastCheck", { date: formatProbeTimestamp(state.lastProbeById[connection.id]!.checkedAt) }) }}
            </time>
          </template>
        </div>
        <div class="connection-card-footer">
          <span v-if="!props.openSession" class="session-hint">{{ t("connections.sessionUnavailable") }}</span>
          <span v-else class="session-hint">{{ sessionsFor(connection.id).length ? t("connections.activeSessions") : t("connections.terminalClosed") }}</span>
          <div class="connection-primary-actions">
            <button v-if="props.openSession && sessionsFor(connection.id).length" type="button"
              class="secondary icon-action" :disabled="working || sessionPending.includes(connection.id)"
              :aria-label="t('connections.newSession')" :title="t('connections.newSession')" @click="connect(connection.id, true)">
              <UiIcon name="plus" :size="18" />
            </button>
            <button v-if="props.openSession" type="button" class="connect-action"
              :disabled="working || sessionPending.includes(connection.id)"
              :aria-busy="sessionPending.includes(connection.id)" @click="connect(connection.id, false)">
              <span v-if="sessionPending.includes(connection.id)" class="button-spinner" aria-hidden="true"></span>
              <UiIcon v-else name="terminal" :size="18" />
              {{ sessionsFor(connection.id).length ? t("connections.openSession") : t("connections.connect") }}
            </button>
            <button v-else type="button" class="connect-action"
              :disabled="working || state.probingIds.includes(connection.id)"
              :aria-busy="state.probingIds.includes(connection.id)" @click="probeSaved(connection.id)">
              <span v-if="state.probingIds.includes(connection.id)" class="button-spinner" aria-hidden="true"></span>
              <UiIcon v-else name="plug" :size="18" />
              {{ state.probingIds.includes(connection.id) ? t("connections.checking") : t("connections.checkConnection") }}
            </button>
          </div>
        </div>
        <details v-if="!state.probingIds.includes(connection.id) && state.lastProbeById[connection.id]?.response"
          class="probe-details">
          <summary>{{ t("connections.checkDetails") }}</summary>
          <pre>{{ state.lastProbeById[connection.id]?.response }}</pre>
        </details>
      </article>
        </div>
      </section>
    </div>

    <dialog ref="confirmationDialog" class="editor-dialog pgx-confirm-dialog" aria-labelledby="pgx-confirm-title"
      @cancel="$event.preventDefault(); finishConfirmation(false)">
      <div class="editor">
        <h3 id="pgx-confirm-title">{{ t("connections.confirmTitle") }}</h3>
        <p class="pgx-confirm-message">{{ confirmationMessage }}</p>
        <div class="form-actions">
          <button type="button" @click="finishConfirmation(true)">{{ t("connections.confirm") }}</button>
          <button type="button" class="secondary" @click="finishConfirmation(false)">{{ t("connections.cancel") }}</button>
        </div>
      </div>
    </dialog>

    <dialog ref="projectDialog" class="project-dialog" aria-labelledby="project-editor-title"
      @cancel="$event.preventDefault(); closeProjectEditor()" @click="onProjectBackdrop">
      <form novalidate class="project-editor" @submit.prevent="saveProject">
        <h3 id="project-editor-title">{{ projectEditingId === null ? t("connections.newProject") : t("connections.renameProject") }}</h3>
        <label>{{ t("connections.projectName") }}
          <input v-model="projectName" maxlength="80" required :disabled="projectState.mutating"
            autocomplete="off" :placeholder="t('connections.sortNameExample')" />
        </label>
        <p v-if="projectState.error" class="field-error" role="alert">{{ projectState.error }}</p>
        <p class="hint">{{ t("connections.projectHint") }}</p>
        <div class="form-actions">
          <button type="submit" :disabled="projectState.mutating">{{ projectEditingId === null ? t("connections.create") : t("connections.save") }}</button>
          <button type="button" class="secondary" :disabled="projectState.mutating" @click="closeProjectEditor">{{ t("connections.cancel") }}</button>
        </div>
      </form>
    </dialog>

    <dialog ref="editorDialog" class="editor-dialog" aria-labelledby="editor-title"
      @cancel="onEditorCancel" @click="onEditorBackdrop">
    <section v-if="state.editor" class="editor" aria-labelledby="editor-title">
      <h3 id="editor-title">{{ editingConnection ? t("connections.editNamed", { name: editingConnection.name }) : t("connections.newConnection") }}</h3>
      <form novalidate @submit.prevent="probeDraft">
        <label>{{ t("connections.connectionName") }}
          <input ref="editorNameField" v-model="state.editor.name" name="name" maxlength="80" required
            :aria-invalid="Boolean(state.fieldErrors.name)" aria-describedby="connection-name-error"
            :disabled="working" @input="clearField('name')" />
          <small v-if="state.fieldErrors.name" id="connection-name-error" class="field-error">{{ state.fieldErrors.name }}</small>
        </label>
        <div class="form-grid">
          <label>{{ t("connections.host") }}
            <input v-model="state.editor.host" name="host" maxlength="253" required
              :aria-invalid="Boolean(state.fieldErrors.host)" aria-describedby="connection-host-error"
              :disabled="working" @input="clearField('host')" />
            <small v-if="state.fieldErrors.host" id="connection-host-error" class="field-error">{{ state.fieldErrors.host }}</small>
          </label>
          <label>{{ t("connections.port") }}
            <input v-model="state.editor.port" name="port" type="text" inputmode="numeric" maxlength="5" required
              :aria-invalid="Boolean(state.fieldErrors.port)" aria-describedby="connection-port-error"
              :disabled="working" @input="clearField('port')" />
            <small v-if="state.fieldErrors.port" id="connection-port-error" class="field-error">{{ state.fieldErrors.port }}</small>
          </label>
          <label>{{ t("connections.database") }}
            <input v-model="state.editor.database" name="database" maxlength="256" required
              :aria-invalid="Boolean(state.fieldErrors.database)" aria-describedby="connection-database-error"
              :disabled="working" @input="clearField('database')" />
            <small v-if="state.fieldErrors.database" id="connection-database-error" class="field-error">{{ state.fieldErrors.database }}</small>
          </label>
          <label>{{ t("connections.pgUser") }}
            <input v-model="state.editor.username" name="username" autocomplete="off" maxlength="256" required
              :aria-invalid="Boolean(state.fieldErrors.username)" aria-describedby="connection-username-error"
              :disabled="working" @input="clearField('username')" />
            <small v-if="state.fieldErrors.username" id="connection-username-error" class="field-error">{{ state.fieldErrors.username }}</small>
          </label>
          <label>{{ t("connections.pgPassword") }}
            <span class="connection-password-wrap">
              <input v-model="state.editor.password" name="password" :type="showDraftPassword ? 'text' : 'password'"
                autocomplete="off" maxlength="1024"
                :placeholder="editingConnection?.has_password ? t('connections.keepPassword') : t('connections.passwordNotSaved')"
                :aria-invalid="Boolean(state.fieldErrors.password)" aria-describedby="connection-password-error"
                :disabled="working || state.editor.removePassword" @input="clearField('password')" />
              <button type="button" class="password-toggle" :disabled="working || state.editor.removePassword"
                :aria-pressed="showDraftPassword" :aria-label="showDraftPassword ? t('auth.hidePassword') : t('auth.showPassword')"
                :title="showDraftPassword ? t('auth.hidePassword') : t('auth.showPassword')" @click="showDraftPassword = !showDraftPassword">
                <UiIcon :name="showDraftPassword ? 'eye-off' : 'eye'" :size="19" />
              </button>
            </span>
            <small v-if="state.fieldErrors.password" id="connection-password-error" class="field-error">{{ state.fieldErrors.password }}</small>
          </label>
          <label>TLS
            <select v-model="state.editor.sslmode" name="sslmode" :disabled="working"
              :aria-invalid="Boolean(state.fieldErrors.sslmode)" aria-describedby="connection-sslmode-error"
              @change="clearField('sslmode')">
              <option value="verify-full">{{ t("connections.tlsVerifyFull") }}</option>
              <option value="require">{{ t("connections.tlsRequire") }}</option>
              <option value="disable">{{ t("connections.tlsDisable") }}</option>
            </select>
            <small v-if="state.fieldErrors.sslmode" id="connection-sslmode-error" class="field-error">{{ state.fieldErrors.sslmode }}</small>
          </label>
        </div>
        <label v-if="editingConnection?.has_password" class="check-row">
          <input v-model="state.editor.removePassword" type="checkbox" name="remove_password" :disabled="working" @change="clearField('password')" />
          {{ t("connections.removePassword") }}
        </label>
        <p v-if="reusedPasswordOnNewTarget" class="warning" role="status">
          {{ t("connections.targetChangedWarning") }}
        </p>
        <p v-if="editingActiveSessions" class="hint" role="note">
          {{ t("connections.activeTerminalsHint", { count: String(editingActiveSessions) }) }}
        </p>
        <p v-if="changingActiveTarget" class="warning" role="alert">
          {{ t("connections.activeTerminalsWarning") }}
        </p>
        <p class="hint">{{ t("connections.storageHint") }}</p>
        <p v-if="state.editorStatus" ref="editorStatusElement" class="inline-status" :class="state.editorStatus.kind"
          :role="state.editorStatus.kind === 'error' ? 'alert' : 'status'" aria-live="polite">
          {{ state.editorStatus.text }}
        </p>
        <div class="form-actions">
          <button type="submit" :disabled="working">{{ t("connections.checkDetailsButton") }}</button>
          <button type="button" class="secondary" :disabled="working" @click="saveDraft">
            {{ editingConnection ? t("connections.saveChanges") : t("connections.saveConnection") }}
          </button>
          <button type="button" class="secondary" :disabled="working" @click="closeEditor">{{ t("connections.cancel") }}</button>
        </div>
      </form>
    </section>
    </dialog>
  </section>
</template>

<style scoped>
.connections-view { padding:.25rem; background:transparent; border:0; }
.heading,.toolbar-actions,.form-actions { display:flex; flex-wrap:wrap; align-items:center; gap:.75rem; }
.heading { justify-content:space-between; }
.heading h2 { margin:0 0 .35rem; }
.hint { color:var(--muted,#b3c0ca); margin:.5rem 0; }
.toolbar-actions button,.connection-primary-actions button { display:inline-flex; justify-content:center; align-items:center; gap:.45rem; }
.icon-action { width:2.8rem; height:2.7rem; padding:.55rem; }
.refresh-glyph { display:inline-flex; align-items:center; justify-content:center; transform-origin:center; }
.spinning { animation:connection-rotate .7s linear infinite; }
@keyframes connection-rotate { to { transform:rotate(360deg); } }
.connection-list { display:grid; gap:.85rem; margin:1rem 0; }
.project-group { display:grid; gap:.65rem; }
.project-list-options { display:flex; justify-content:space-between; align-items:end; gap:.8rem; flex-wrap:wrap; margin:.65rem 0 .25rem; }
.project-list-options label { display:grid; gap:.25rem; font-size:.82rem; color:var(--text-secondary); }
.project-list-options select { background:var(--surface-2); border:1px solid var(--border); border-radius:.45rem; color:var(--text); padding:.45rem .55rem; min-width:13rem; }
.project-list-options .hint { font-size:.79rem; margin:0; }
.project-pin-label { color:var(--text-secondary); font-size:.74rem; font-weight:400; }
.project-heading { display:flex; align-items:center; justify-content:space-between; gap:.7rem; padding:.45rem .2rem; border-bottom:1px solid var(--border); }
.project-collapse { display:flex; align-items:center; gap:.6rem; border:0; padding:.45rem; background:transparent; color:var(--text); text-align:left; cursor:pointer; }
.project-chevron { width:.7rem; }
.project-collapse:disabled { opacity:1; cursor:default; }
.project-count { border:1px solid var(--border); border-radius:99px; padding:.1rem .45rem; color:var(--text-secondary); font-size:.75rem; }
.project-contents { display:grid; gap:.75rem; }
.project-empty { padding:.6rem 1rem; color:var(--text-secondary); font-size:.88rem; }
.project-actions { margin-left:auto; }
.connection-move-label { display:grid; gap:.3rem; padding:.55rem .7rem; font-size:.82rem; color:var(--text-secondary); }
.connection-move-label select { width:100%; min-width:12rem; color:var(--text); background:var(--input-bg); border:1px solid var(--border); border-radius:.4rem; padding:.45rem; }
.project-dialog { width:min(440px,calc(100vw - 2rem)); padding:0; border:1px solid var(--border-strong); border-radius:.8rem; color:var(--text); background:var(--surface); }
.project-dialog::backdrop { background:rgba(0,0,0,.67); }
.project-editor { padding:1.3rem; display:grid; gap:1rem; }
.project-editor h3 { margin:0; }
.project-editor label { display:grid; gap:.45rem; }
.project-editor input { box-sizing:border-box; width:100%; padding:.7rem; color:var(--text); background:var(--input-bg); border:1px solid var(--border); border-radius:.5rem; }

.empty-state { display:flex; flex-direction:column; align-items:center; justify-content:center; gap:.65rem; text-align:center; color:var(--text-secondary); padding:3rem 1.5rem; border:1px dashed var(--border); border-radius:.75rem; }
.empty-state strong { color:var(--text); }
.empty-state span { max-width:34rem; }
.connection-card { padding:1.15rem 1.25rem; border:1px solid var(--border,#435461); border-radius:.75rem; background:var(--surface,#192632); }
.connection-card-header { display:flex; justify-content:space-between; align-items:flex-start; gap:.75rem; }
.connection-identity { display:flex; align-items:center; gap:.85rem; min-width:0; }
.connection-emblem { display:inline-grid; place-items:center; flex-shrink:0; width:2.8rem; height:2.8rem; border:1px solid var(--border); border-radius:.65rem; background:var(--surface-2); color:var(--accent); }
.identity-text { display:flex; flex-direction:column; gap:.3rem; min-width:0; }
.connection-title { font-size:1.05rem; }
.address { color:var(--text-secondary); font-size:.87rem; overflow-wrap:anywhere; }
.connection-meta { display:flex; align-items:center; flex-wrap:wrap; gap:.5rem; margin:.9rem 0; }
.meta-tag { padding:.25rem .55rem; border:1px solid var(--border); border-radius:50px; color:var(--text-secondary); font-size:.78rem; }
.connection-card-footer { display:flex; align-items:center; justify-content:space-between; flex-wrap:wrap; gap:.75rem; padding-top:.65rem; border-top:1px solid var(--border); }
.session-hint { color:var(--text-secondary); font-size:.82rem; }
.connection-primary-actions { display:flex; align-items:center; gap:.5rem; margin-left:auto; }
/* A historical observation belongs beside TLS/password metadata, not in a second panel. */
.probe-meta { display:inline-flex; align-items:center; gap:.35rem; }
.probe-meta[data-result="success"] { color:var(--success); border-color:currentColor; }
.probe-meta[data-result="error"] { color:var(--error); border-color:currentColor; }
.probe-meta.pending { color:var(--text-secondary); }
.probe-checked-at { color:var(--text-secondary); font-size:.78rem; margin-left:auto; }
.probe-details { margin-top:.65rem; font-size:.85rem; }

.probe-details summary { cursor:pointer; width:fit-content; color:var(--text-secondary); }
.probe-details pre { margin:.6rem 0 0; padding:.65rem; border-radius:.5rem; background:var(--surface-2); overflow-x:auto; white-space:pre-wrap; overflow-wrap:anywhere; font:inherit; }
.connection-menu { position:relative; flex-shrink:0; }
.connection-menu summary { display:grid; place-items:center; width:2.6rem; height:2.6rem; border:1px solid var(--border); border-radius:.5rem; cursor:pointer; color:var(--text); background:var(--button-secondary); list-style:none; }
.connection-menu summary::-webkit-details-marker { display:none; }
.connection-menu summary:hover { background:var(--surface-3); }
.connection-menu[open] summary { background:var(--surface-3); }
.connection-menu-items { position:absolute; right:0; top:calc(100% + .4rem); z-index:20; width:max-content; min-width:13rem; padding:.3rem; border:1px solid var(--border-strong); border-radius:.65rem; background:var(--surface); box-shadow:0 12px 30px #0006; }
.connection-menu-items button { display:flex; align-items:center; justify-content:flex-start; gap:.7rem; width:100%; padding:.65rem .75rem; border:0; border-radius:.4rem; text-align:left; background:transparent; color:var(--text); font-size:.9rem; }
.connection-menu-items button:hover:not(:disabled) { background:var(--surface-2); }
.connection-menu-items .menu-danger { color:var(--error); }
.editor-dialog { width:min(760px,calc(100vw - 2rem)); max-height:min(90vh,900px); padding:0; border:1px solid var(--border-strong); border-radius:.8rem; color:var(--text); background:var(--surface); box-shadow:0 20px 70px #0008; overflow:auto; }
.editor-dialog::backdrop { background:rgba(0,0,0,.67); }
.pgx-confirm-dialog { width:min(580px,calc(100vw - 2rem)); }
.pgx-confirm-message { white-space:pre-line; overflow-wrap:anywhere; line-height:1.5; }
.pgx-confirm-dialog .form-actions { display:flex; gap:.65rem; flex-wrap:wrap; }
.editor { padding:1.4rem; }
.editor h3 { margin:0 0 1.15rem; font-size:1.22rem; }
.editor form,.editor label { display:grid; gap:.45rem; }
.editor form { max-width:none; gap:1rem; }
.editor input:not([type="checkbox"]),.editor select { box-sizing:border-box; width:100%; min-width:0; padding:.65rem; border-radius:.5rem; border:1px solid var(--border,#526675); background:var(--input-bg,#101921); color:inherit; }
.form-grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:1rem; }
.connection-password-wrap { position:relative; min-width:0; }
.connection-password-wrap input { padding-right:3.25rem !important; }
.password-toggle { position:absolute; right:.35rem; top:50%; transform:translateY(-50%); display:grid; place-items:center; width:2.4rem; height:2.4rem; padding:0; background:transparent; border:0; color:var(--text-secondary); }
.password-toggle:hover:not(:disabled) { background:var(--surface-2); }
.editor input[aria-invalid="true"],.editor select[aria-invalid="true"] { border-color:var(--error-focus); }
.field-error { color:var(--error); }
.check-row { display:flex !important; align-items:center; }
.check-row input { flex:0 0 auto; }
.form-actions { margin-top:.25rem; }
.inline-status { white-space:pre-wrap; overflow-wrap:anywhere; padding:.5rem 0 0; }
.inline-status.success,.notice { color:var(--success); }
.toast { position:fixed; right:1.3rem; bottom:1.2rem; z-index:100; max-width:min(28rem,calc(100vw - 2rem)); padding:.8rem 1rem; border:1px solid var(--border-strong); border-inline-start:3px solid var(--success); border-radius:.6rem; background:var(--surface); box-shadow:0 10px 28px #0005; }
.inline-status.error { color:var(--error); }
.inline-status.pending { color:var(--text-secondary); }
.warning { border-inline-start:3px solid var(--border-strong); padding:.6rem .8rem; background:var(--surface-3); }
button:disabled { opacity:.55; cursor:not-allowed; }
@media (prefers-reduced-motion:reduce) { .spinning { animation:none; } }
@media (max-width:600px) {
  .form-grid { grid-template-columns:1fr; }
  .connection-card-footer { align-items:flex-start; }
  .connection-primary-actions { margin-left:0; }
  .editor { padding:1rem; }
}
.connection-shortcut-row{display:flex;align-items:center;gap:.75rem;flex-wrap:wrap;margin:.5rem 0}.connection-shortcut-row label{display:flex;align-items:center;gap:.5rem;font-size:.85rem}.connection-shortcut-row select{min-width:9rem}.connection-shortcut-row small{color:var(--muted)}
</style>
