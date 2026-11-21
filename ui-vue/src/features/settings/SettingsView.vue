<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { closeOpenDetails, dismissOpenDetailsOutside } from "../../core/disclosure-menu.js";
import type { ShellState } from "../../core/shell-state.js";
import { formatBytes, formatInteger, formatSnapshotTime } from "../../core/formatters.js";
import { createUsersController, createUsersState } from "./users-controller.js";
import { hasUserFormErrors, validateUserForm, type UserFormErrors } from "./user-form-validation.js";
import PasswordField from "../../components/PasswordField.vue";
import UiIcon from "../../components/UiIcon.vue";
import {
  createExtraSettingsController, createExtraSettingsState,
  LOG_RETENTION_DAYS, LOG_SIZE_LIMITS,
  type SettingsTab,
} from "./extra-controller.js";
import { readinessConnections, readinessCheckStatus, readinessStatusLabel, readinessOverview, readinessSupportLabel, retentionDays, retentionSelection, remainingRefreshFeedbackMs, historyOverviewPage, HISTORY_PAGE_SIZES, HISTORY_SECTIONS, type HistoryPageSize } from "./settings-view-model.js";
import ThemeSettings from "../theme/ThemeSettings.vue";
import { t } from "../i18n/locale-service.js";
import ApplicationLogViewer from "./ApplicationLogViewer.vue";
import SnapshotSchedulesView from "../snapshots/SnapshotSchedulesView.vue";
import StorageManagementPanel from "./StorageManagementPanel.vue";
import type { SnapshotSchedulerActions, SnapshotSchedulerState } from "../snapshots/scheduler.js";
import { pathDraftStatus, type PathKind } from "./paths-view-model.js";

const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; selectedTab: SettingsTab; snapshotState: SnapshotSchedulerState; snapshotActions: SnapshotSchedulerActions }>();
const users = reactive(createUsersState());
const extra = reactive(createExtraSettingsState());
const userActions = createUsersController(props.shell, users, props.invoke);
const extraActions = createExtraSettingsController(props.shell, extra, props.invoke);
const locked = computed(() => props.shell.busy || users.mutating || users.confirmingDeletion || extra.busy || extra.loading);
// The SQLite list may load too quickly for a loading animation to be visible.
// Keep only the visual feedback for at least 450 ms, without delaying the IPC result.
const refreshingUsers = ref(false);
let refreshTimer: ReturnType<typeof setTimeout> | null = null;
let refreshRevision = 0;
function clearRefreshFeedback(): void {
  ++refreshRevision;
  if (refreshTimer !== null) clearTimeout(refreshTimer);
  refreshTimer = null;
  refreshingUsers.value = false;
}
async function refreshUserList(): Promise<void> {
  if (locked.value || users.loading || refreshingUsers.value) return;
  const revision = ++refreshRevision;
  const started = Date.now();
  refreshingUsers.value = true;
  try { await userActions.refresh(); }
  finally {
    if (revision !== refreshRevision) return;
    const delay = remainingRefreshFeedbackMs(started, Date.now());
    if (delay === 0) refreshingUsers.value = false;
    else refreshTimer = setTimeout(() => {
      if (revision === refreshRevision) refreshingUsers.value = false;
      refreshTimer = null;
    }, delay);
  }
}
extraActions.changeTab(props.selectedTab);
watch(() => props.selectedTab, tab => {
  extraActions.changeTab(tab);
  if (tab !== "users") closeOpenDetails(usersList.value, "details.user-menu");
  if (tab !== "paths") {
    editingPath.value = null;
    pathFeedback.value = null;
  }
  if (tab === "readiness") void loadReadinessConnections();
  if (tab === "history") void extraActions.refreshHistory();
});

// Credentials are form-local and never enter Pinia, IPC logs or global state.
const username = ref("");
const password = ref("");
const passwordConfirm = ref("");
const fieldErrors = ref<UserFormErrors>({});
const userDialog = ref<HTMLDialogElement | null>(null);
const usersList = ref<HTMLElement | null>(null);
function onDocumentPointerDown(event: PointerEvent): void {
  dismissOpenDetailsOutside(usersList.value, "details.user-menu", event.target);
}
function onDocumentKeyDown(event: KeyboardEvent): void {
  if (event.key === "Escape") closeOpenDetails(usersList.value, "details.user-menu");
}
function clearUserForm(): void {
  username.value = "";
  password.value = "";
  passwordConfirm.value = "";
  fieldErrors.value = {};
}
function selectUserForm(kind: "create" | "rename" | "password", target?: string): void {
  if (locked.value) return;
  clearUserForm();
  if (kind === "create") userActions.openCreate();
  else if (kind === "rename" && target && userActions.openRename(target)) username.value = target;
  else if (kind === "password" && target) userActions.openPassword(target);
}
function cancelUserForm(): void {
  if (locked.value) return;
  userActions.cancelEditor();
  clearUserForm();
}
function cancelDialog(event: Event): void {
  event.preventDefault();
  cancelUserForm();
}
function onBackdropClick(event: MouseEvent): void {
  if (event.target === userDialog.value) cancelUserForm();
}
watch(() => users.editor, async editor => {
  await nextTick();
  const dialog = userDialog.value;
  if (!dialog) return;
  if (editor) {
    if (!dialog.open) dialog.showModal();
    dialog.querySelector<HTMLInputElement>("input")?.focus();
  } else if (dialog.open) {
    dialog.close();
  }
}, { flush: "post" });
async function submitUserForm(): Promise<void> {
  if (locked.value || !users.editor) return;
  const editor = users.editor;
  fieldErrors.value = validateUserForm(editor, username.value, password.value, passwordConfirm.value, users.users.map(user => user.username));
  if (hasUserFormErrors(fieldErrors.value)) return; // Keep the draft on client-side errors.
  // A rejected IPC request must not erase the draft: the user may need to
  // correct just the username (for example, after a uniqueness conflict).
  let succeeded: boolean;
  if (editor.kind === "create") succeeded = await userActions.create(username.value.trim(), password.value, passwordConfirm.value);
  else if (editor.kind === "rename") succeeded = await userActions.rename(username.value.trim());
  else succeeded = await userActions.changePassword(password.value, passwordConfirm.value);
  if (succeeded) {
    password.value = "";
    passwordConfirm.value = "";
  }
}
function closeUserMenu(event: Event): void {
  const menu = (event.currentTarget as HTMLElement).closest<HTMLDetailsElement>("details.user-menu");
  if (menu) menu.open = false;
}
function onUserMenuAction(event: MouseEvent, kind: "rename" | "password" | "delete", name: string): void {
  closeUserMenu(event);
  if (kind === "delete") void removeUser(name);
  else selectUserForm(kind, name);
}
function onUserMenuToggle(event: Event): void {
  const opened = event.currentTarget as HTMLDetailsElement;
  if (!opened.open) return;
  const list = opened.closest(".users-list");
  list?.querySelectorAll<HTMLDetailsElement>("details.user-menu").forEach(menu => {
    if (menu !== opened) menu.open = false;
  });
}
async function removeUser(name: string): Promise<void> {
  if (locked.value) return;
  await userActions.deleteUser(name, target => confirmAction(
    t('settings.usersDeleteConfirm', { name: target }),
  ));
}

// Only one path can be edited at a time; write operations remain guarded by Core.
const editingPath = ref<PathKind | null>(null);
const activePathRequest = ref<PathKind | null>(null);
const pathFeedback = ref<{ kind: PathKind; type: "success" | "error"; text: string } | null>(null);
const pathCards = computed<ReadonlyArray<{
  kind: PathKind; icon: "folder" | "database" | "scroll";
  title: string; subtitle: string; button: string; detail: string;
  danger: boolean;
}>>(() => [
  { kind: "export", icon: "folder", title: t('settings.pathExportTitle'),
    subtitle: t('settings.pathExportSubtitle'), button: t('settings.pathSave'),
    detail: t('settings.pathExportDetail'), danger: false },
  { kind: "workspace", icon: "database", title: t('settings.pathWorkspaceTitle'),
    subtitle: t('settings.pathWorkspaceSubtitle'), button: t('settings.pathWorkspaceMove'),
    detail: t('settings.pathWorkspaceDetail'), danger: true },
  { kind: "log", icon: "scroll", title: t('settings.logsTitle'),
    subtitle: t('settings.pathLogSubtitle'), button: t('settings.pathLogMove'),
    detail: t('settings.pathLogDetail'), danger: true },
]);
const exportDir = ref("");
const workspaceRoot = ref("");
const logDir = ref("");
const retentionChoice = ref("forever");
const customRetention = ref("");
const logMaxBytes = ref(LOG_SIZE_LIMITS[0]);
const logRetention = ref<number>(LOG_RETENTION_DAYS[0]);
const formError = ref("");
watch(() => extra.paths, paths => {
  if (!paths) return;
  // Refresh must not overwrite a path currently being edited.
  if (editingPath.value !== "export") exportDir.value = paths.export_dir;
  if (editingPath.value !== "workspace") workspaceRoot.value = paths.workspace_root;
  if (editingPath.value !== "log") logDir.value = paths.application_log_dir;
});
function savedPath(kind: PathKind): string {
  if (!extra.paths) return "";
  if (kind === "export") return extra.paths.export_dir;
  if (kind === "workspace") return extra.paths.workspace_root;
  return extra.paths.application_log_dir;
}
function draftPath(kind: PathKind): string {
  if (kind === "export") return exportDir.value;
  if (kind === "workspace") return workspaceRoot.value;
  return logDir.value;
}
function setDraftPath(kind: PathKind, value: string): void {
  if (kind === "export") exportDir.value = value;
  else if (kind === "workspace") workspaceRoot.value = value;
  else logDir.value = value;
  if (pathFeedback.value?.kind === kind) pathFeedback.value = null;
}
function onPathInput(kind: PathKind, event: Event): void {
  setDraftPath(kind, (event.target as HTMLInputElement).value);
}
function editPath(kind: PathKind): void {
  if (locked.value || editingPath.value !== null || !extra.paths) return;
  editingPath.value = kind;
  pathFeedback.value = null;
  setDraftPath(kind, savedPath(kind));
}
function cancelPathEdit(): void {
  if (locked.value) return;
  if (editingPath.value !== null) setDraftPath(editingPath.value, savedPath(editingPath.value));
  editingPath.value = null;
  pathFeedback.value = null;
}
async function submitPath(kind: PathKind): Promise<void> {
  if (locked.value || editingPath.value !== kind || !extra.paths ||
      pathDraftStatus(draftPath(kind), savedPath(kind)) !== "changed") return;
  activePathRequest.value = kind;
  pathFeedback.value = null;
  try {
    const value = draftPath(kind).trim();
    let succeeded = false;
    if (kind === "export") succeeded = await extraActions.saveExportDirectory(value);
    else if (kind === "workspace") succeeded = await extraActions.migrateWorkspace(value, confirmDanger);
    else succeeded = await extraActions.migrateLogDirectory(value, confirmDanger);
    if (succeeded) {
      editingPath.value = null;
      pathFeedback.value = { kind, type: "success", text: extra.notice ?? t('settings.pathUpdated') };
    } else if (extra.error) {
      // Failure does not prove a filesystem migration was rolled back.
      pathFeedback.value = { kind, type: "error", text: extra.error };
    }
  } finally {
    activePathRequest.value = null;
  }
}
watch(() => extra.retentionDays, days => {
  const next = retentionSelection(days);
  retentionChoice.value = next.choice;
  customRetention.value = next.custom;
});
watch(() => extra.log, log => {
  if (!log) return;
  logMaxBytes.value = log.settings.max_total_bytes;
  logRetention.value = log.settings.retention_days;
});
function confirmDanger(message: string): Promise<boolean> { return confirmAction(message); }
async function saveRetention(): Promise<void> {
  const days = retentionDays(retentionChoice.value, customRetention.value);
  if (days === undefined) {
    formError.value = t('settings.retentionInvalid');
    return;
  }
  formError.value = "";
  await extraActions.setHistoryRetention(days);
}
async function saveLogSettings(): Promise<void> {
  await extraActions.setLogSettings(logMaxBytes.value, logRetention.value);
}
const journalRevision = ref(0);
async function clearApplicationLog(): Promise<void> {
  if (await extraActions.clearLog(confirmDanger)) {
    journalRevision.value++;
  }
}
// The selected connection is a view filter only: no changes to Rust IPC or storage.
const historyConnection = ref("all");
const historyPageNumber = ref(1);
const historyPageSize = ref<HistoryPageSize>(10);
const namedHistoryRows = computed(() => extra.historyRows.map(row => ({
  ...row,
  // The historical name is preserved in SQLite. Only the UI uses the current profile name.
  connection_name: extra.historyLiveNames[row.connection_id] ?? row.connection_name,
})));
const historyOptions = computed(() => [...namedHistoryRows.value].sort((a, b) => a.connection_name.localeCompare(b.connection_name, "ru")));
const historyPage = computed(() => historyOverviewPage(namedHistoryRows.value, historyConnection.value, historyPageNumber.value, historyPageSize.value));
watch([historyConnection, historyPageSize], () => { historyPageNumber.value = 1; });
watch(() => extra.historyRows.map(row => row.connection_id).join(","), () => {
  if (historyConnection.value !== "all" && !extra.historyRows.some(row => String(row.connection_id) === historyConnection.value))
    historyConnection.value = "all";
  historyPageNumber.value = historyPage.value.page;
});

const connectionOptions = ref<{ id: number; name: string }[]>([]);
const readinessConnection = ref("");
const readinessOptionsError = ref("");
const checkingReadiness = ref(false);
const refreshingReadinessConnections = ref(false);
// A completed report belongs to the connection it was requested for. Do not
// present the previous database's report after the user switches the selector.
const checkedReadinessConnection = ref<string | null>(null);
const shownReadiness = computed(() =>
  checkedReadinessConnection.value === readinessConnection.value ? extra.readiness : null);
const localReadinessOverview = computed(() => readinessOverview(shownReadiness.value?.local_checks ?? []));
const postgresReadinessOverview = computed(() => readinessOverview(shownReadiness.value?.postgres?.features ?? []));
let optionsSequence = 0;
async function loadReadinessConnections(): Promise<void> {
  if (refreshingReadinessConnections.value) return;
  const sequence = ++optionsSequence;
  const generation = props.shell.authGeneration;
  refreshingReadinessConnections.value = true;
  try {
    const options = readinessConnections(await props.invoke("list_connections"));
    if (sequence !== optionsSequence || generation !== props.shell.authGeneration || props.shell.screen !== "workspace") return;
    connectionOptions.value = options;
    if (readinessConnection.value && !options.some(c => String(c.id) === readinessConnection.value)) readinessConnection.value = "";
    readinessOptionsError.value = "";
  } catch (error) {
    if (sequence === optionsSequence && generation === props.shell.authGeneration) {
      readinessOptionsError.value = t('settings.readinessLoadError', { error: String(error) });
      connectionOptions.value = [];
    }
  } finally {
    if (sequence === optionsSequence) refreshingReadinessConnections.value = false;
  }
}
async function runReadiness(): Promise<void> {
  const selectedConnection = readinessConnection.value;
  const id = selectedConnection ? Number(selectedConnection) : null;
  if (id !== null && !connectionOptions.value.some(c => c.id === id)) {
    readinessOptionsError.value = t('settings.readinessSelectExisting');
    return;
  }
  if (locked.value || checkingReadiness.value) return;
  checkingReadiness.value = true;
  checkedReadinessConnection.value = null;
  try {
    if (await extraActions.runReadiness(id))
      checkedReadinessConnection.value = selectedConnection;
  } finally {
    checkingReadiness.value = false;
  }
}
function reset(): void {
  editingPath.value = null;
  pathFeedback.value = null;
  activePathRequest.value = null;
  clearRefreshFeedback();
  optionsSequence += 1;
  connectionOptions.value = [];
  readinessConnection.value = "";
  readinessOptionsError.value = "";
  checkingReadiness.value = false;
  refreshingReadinessConnections.value = false;
  checkedReadinessConnection.value = null;
  formError.value = "";
  historyConnection.value = "all";
  historyPageNumber.value = 1;
  historyPageSize.value = 10;
  clearUserForm();
  userActions.reset();
  extraActions.reset();
  extraActions.changeTab(props.selectedTab);
}
function load(): void {
  void userActions.refresh();
  void extraActions.refresh();
  if (extra.tab === "readiness") void loadReadinessConnections();
}
onMounted(() => {
  load();
  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  document.addEventListener("keydown", onDocumentKeyDown);
});
watch(() => props.shell.authGeneration, () => {
  reset();
  if (props.shell.screen === "workspace") load();
});
onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  document.removeEventListener("keydown", onDocumentKeyDown);
  reset();
});
</script>

<template>
  <section class="settings-view" aria-labelledby="settings-title">
    <h1 id="settings-title" class="settings-page-title">{{ t('settings.title') }}</h1>
    <div class="settings-content">

      <SnapshotSchedulesView v-if="extra.tab === 'snapshots'" :state="snapshotState" :actions="snapshotActions" />

      <section v-if="extra.tab === 'users'" aria-labelledby="settings-users-title">
        <div class="users-topline">
          <div><h2 id="settings-users-title">{{ t('settings.usersTitle') }}</h2>
            <p class="settings-hint">{{ t('settings.usersHint') }}</p></div>
          <div class="users-tools">
            <button type="button" class="secondary icon-action" :title="t('settings.usersRefresh')"
              :aria-label="users.loading || refreshingUsers ? t('settings.usersRefreshing') : t('settings.usersRefresh')"
              :aria-busy="users.loading || refreshingUsers" :disabled="locked || users.loading || refreshingUsers" @click="refreshUserList">
              <UiIcon name="refresh" :size="18" :class="{ 'refresh-spinning': users.loading || refreshingUsers }" />
            </button>
            <button type="button" class="add-user" :disabled="locked" @click="selectUserForm('create')">
              <UiIcon name="plus" :size="18" /> {{ t('settings.usersAdd') }}
            </button>
          </div>
        </div>
        <p v-if="users.error && !users.editor" role="alert" class="settings-error">{{ users.error }}</p>
        <p v-if="users.notice" role="status" class="settings-feedback">{{ users.notice }}</p>
        <p v-if="users.loading" role="status">{{ t('settings.usersLoading') }}</p>
        <div ref="usersList" class="users-list" role="list" :aria-label="t('settings.usersList')">
          <div v-for="user in users.users" :key="user.username" class="user-row" role="listitem">
            <div class="user-ident">
              <UiIcon name="users" :size="20" />
              <span class="user-name">{{ user.username }}</span>
              <span v-if="user.current" class="current-user">{{ t('settings.usersYou') }}</span>
            </div>
            <details class="user-menu" @toggle="onUserMenuToggle" @keydown.esc="closeUserMenu">
              <summary class="menu-trigger" :aria-label="t('settings.usersActions', { name: user.username })"
                :title="t('settings.usersActions', { name: user.username })"><UiIcon name="ellipsis" :size="19" /></summary>
              <div class="menu-content">
                <button type="button" :disabled="locked" @click="onUserMenuAction($event, 'rename', user.username)"><UiIcon name="pencil" :size="16" /> {{ t('settings.usersRename') }}</button>
                <button type="button" :disabled="locked" @click="onUserMenuAction($event, 'password', user.username)"><UiIcon name="key" :size="16" /> {{ t('settings.usersPassword') }}</button>
                <div class="menu-divider" aria-hidden="true"></div>
                <button type="button" class="menu-danger" :disabled="locked || user.current"
                  :title="user.current ? t('settings.usersCannotDeleteSelf') : t('settings.usersDeleteTitle')"
                  @click="onUserMenuAction($event, 'delete', user.username)"><UiIcon name="trash" :size="16" /> {{ t('settings.usersDelete') }}</button>
              </div>
            </details>
          </div>
          <p v-if="!users.loading && !users.users.length">{{ t('settings.usersEmpty') }}</p>
        </div>
      </section>

        <section v-else-if="extra.tab === 'interface'" aria-labelledby="theme-settings-title">
          <ThemeSettings />
        </section>

        <section v-else-if="extra.tab === 'history'" aria-labelledby="settings-history-title">
          <h2 id="settings-history-title">{{ t('settings.historyTitle') }}</h2>
          <StorageManagementPanel :invoke="props.invoke" />
          <p class="settings-hint">{{ t('settings.historyHint') }}</p>
          <div class="settings-subpanel">
            <h3>{{ t('settings.historyRetentionTitle') }}</h3>
            <label>{{ t('settings.historyRetain') }}
              <select v-model="retentionChoice" :disabled="locked">
                <option value="forever">{{ t('settings.historyForever') }}</option>
                <option v-for="days in [7, 30, 90, 180, 365]" :key="days" :value="String(days)">{{ t('settings.historyDays', { days: String(days) }) }}</option>
                <option value="custom">{{ t('settings.historyCustom') }}</option>
              </select>
            </label>
            <label v-if="retentionChoice === 'custom'">{{ t('settings.historyCustomDays') }}
              <input v-model="customRetention" inputmode="numeric" type="number" min="1" max="36500" :disabled="locked" />
            </label>
            <div class="settings-actions">
              <button type="button" :disabled="locked" @click="saveRetention">
                <span v-if="extra.busy" class="button-spinner" aria-hidden="true"></span>
                {{ t('settings.historySave') }}
              </button>
            </div>
            <p v-if="formError" role="alert" class="settings-error">{{ formError }}</p>
            <p v-if="extra.notice" class="settings-feedback" role="status">{{ extra.notice }}</p>
            <p v-if="extra.error && !extra.historyError" class="settings-error" role="alert">{{ extra.error }}</p>
            <p v-if="extra.historyError" class="settings-error" role="alert">{{ extra.historyError }}</p>
            <p v-if="extra.historyStale" role="status" class="settings-hint">{{ t('settings.historyStale') }}</p>
            <p v-else-if="extra.historyError && extra.historyLoaded" class="settings-hint">{{ t('settings.historyPreviousData') }}</p>
          </div>
          <div class="history-toolbar">
            <div>
              <h3>{{ t('settings.historyByConnections') }}</h3>
              <p class="settings-hint">{{ t('settings.historyByConnectionsHint') }}</p>
            </div>
            <div class="history-filters">
              <label for="history-connection-filter">{{ t('settings.connection') }}
                <select id="history-connection-filter" v-model="historyConnection" :disabled="locked">
                  <option value="all">{{ t('settings.allConnections') }}</option>
                  <option v-for="row in historyOptions" :key="row.connection_id" :value="String(row.connection_id)">{{ row.connection_name }} (#{{ row.connection_id }})</option>
                </select>
              </label>
              <label for="history-page-size">{{ t('settings.rowsPerPage') }}
                <select id="history-page-size" v-model.number="historyPageSize" :disabled="locked">
                  <option v-for="size in HISTORY_PAGE_SIZES" :key="size" :value="size">{{ size }}</option>
                </select>
              </label>
            </div>
          </div>
          <p class="history-summary" role="status">
            {{ t('settings.historySummary', { connections: String(historyPage.totalConnections), snapshots: formatInteger(historyPage.totalSnapshots) }) }}
            <span v-if="extra.loading"> · {{ t('settings.refreshing') }}</span>
          </p>
          <div v-if="historyPage.totalBySection" class="history-section-summary" :aria-label="t('settings.historySectionsLabel')">
            <span v-for="section in HISTORY_SECTIONS" :key="section.key" class="history-section-chip">
              {{ section.label }} <strong>{{ formatInteger(historyPage.totalBySection[section.key]) }}</strong>
            </span>
          </div>
          <p v-else-if="extra.historyLoaded && historyPage.totalConnections > 0" class="settings-hint">
            {{ t('settings.historySectionsUnavailable') }}
          </p>
          <div class="settings-table-wrap"><table>
            <thead><tr><th>ID</th><th>{{ t('settings.connection') }}</th><th>{{ t('settings.snapshotsCount') }}</th><th>{{ t('settings.oldest') }}</th><th>{{ t('settings.latest') }}</th><th>{{ t('settings.actions') }}</th></tr></thead>
            <tbody>
              <tr v-for="row in historyPage.rows" :key="row.connection_id">
                <td>#{{ row.connection_id }}</td><td>{{ row.connection_name }}</td>
                <td class="history-count-cell">
                  <strong>{{ formatInteger(row.snapshot_count) }}</strong>
                  <details v-if="row.section_counts" class="history-count-details">
                    <summary>{{ t('settings.bySections') }}</summary>
                    <div class="history-count-list">
                      <div v-for="section in HISTORY_SECTIONS" :key="section.key">
                        <span>{{ section.label }}</span><strong>{{ formatInteger(row.section_counts[section.key]) }}</strong>
                      </div>
                    </div>
                  </details>
                </td>
                <td>{{ formatSnapshotTime(row.oldest_at_ms) }}</td><td>{{ formatSnapshotTime(row.newest_at_ms) }}</td>
                <td><button type="button" class="danger" :disabled="locked" :aria-label="t('settings.historyDeleteAria', { name: row.connection_name })" @click="extraActions.deleteHistory(row.connection_id, confirmDanger)">{{ t('settings.historyDelete') }}</button></td>
              </tr>
              <tr v-if="extra.historyLoaded && historyPage.totalConnections === 0 && !extra.loading">
                <td colspan="6">{{ historyConnection === "all" ? t('settings.historyEmptyAll') : t('settings.historyEmptyConnection') }}</td>
              </tr>
              <tr v-if="!extra.historyLoaded && !extra.loading"><td colspan="6">{{ t('settings.historyNotLoaded') }}</td></tr>
            </tbody>
          </table></div>
          <nav v-if="historyPage.totalConnections > historyPageSize" class="history-pagination" :aria-label="t('settings.historyPagesAria')">
            <span>{{ t('settings.historyPagination', { from: String(historyPage.from), to: String(historyPage.to), total: String(historyPage.totalConnections), page: String(historyPage.page), pages: String(historyPage.pageCount) }) }}</span>
            <div class="history-page-buttons">
              <button type="button" class="secondary" :disabled="historyPage.page <= 1 || locked" @click="historyPageNumber = historyPage.page - 1">{{ t('settings.back') }}</button>
              <button type="button" class="secondary" :disabled="historyPage.page >= historyPage.pageCount || locked" @click="historyPageNumber = historyPage.page + 1">{{ t('settings.forward') }}</button>
            </div>
          </nav>
        </section>

        <section v-else-if="extra.tab === 'logs'" aria-labelledby="settings-logs-title">
          <h3 id="settings-logs-title">{{ t('settings.logsTitle') }}</h3>
          <p class="settings-hint">{{ t('settings.logsHint') }}</p>
          <div class="settings-subpanel">
            <label>{{ t('settings.logsMaxSize') }}
              <select v-model.number="logMaxBytes" :disabled="locked">
                <option v-for="size in LOG_SIZE_LIMITS" :key="size" :value="size">{{ formatBytes(size) }}</option>
              </select>
            </label>
            <label>{{ t('settings.logsRetain') }}
              <select v-model.number="logRetention" :disabled="locked">
                <option v-for="days in LOG_RETENTION_DAYS" :key="days" :value="days">{{ t('settings.historyDays', { days: String(days) }) }}</option>
              </select>
            </label>
            <div class="settings-actions">
              <button type="button" :disabled="locked" @click="saveLogSettings">{{ t('settings.save') }}</button>
              <button type="button" class="secondary inline-icon-button" :disabled="locked" @click="extraActions.revealLog()"><UiIcon name="folder" :size="16" /> {{ t('settings.logsOpenDir') }}</button>
              <button type="button" class="danger inline-icon-button" :disabled="locked" @click="clearApplicationLog"><UiIcon name="trash" :size="16" /> {{ t('settings.logsClear') }}</button>
            </div>
            <p v-if="extra.error" role="alert" class="settings-error settings-log-feedback">{{ extra.error }}</p>
            <p v-if="extra.notice" role="status" class="settings-feedback settings-log-feedback">{{ extra.notice }}</p>
          </div>
          <div v-if="extra.log" class="settings-subpanel log-status-panel">
            <div class="settings-heading"><h4>{{ t('settings.logsStorage') }}</h4><span :class="extra.log.status.available ? 'log-available' : 'settings-error'">{{ extra.log.status.available ? t('settings.logsWritable') : t('settings.logsUnavailable') }}</span></div>
            <p class="settings-hint">{{ formatBytes(extra.log.status.total_bytes) }} / {{ formatBytes(extra.log.settings.max_total_bytes) }} · {{ t('settings.logsStorageStats', { files: formatInteger(extra.log.status.file_count), days: String(extra.log.settings.retention_days) }) }}</p>
            <details class="log-storage-details"><summary>{{ t('settings.logsDirAndPeriod') }}</summary>
              <p>{{ t('settings.logsDirectory') }} <span class="log-directory">{{ extra.log.status.directory }}</span></p>
              <p>{{ t('settings.logsPeriod') }} {{ formatSnapshotTime(extra.log.status.oldest_at_ms) }} — {{ formatSnapshotTime(extra.log.status.newest_at_ms) }}</p>
            </details>
            <p v-if="extra.log.status.last_error" role="alert" class="settings-error">{{ t('settings.logsLastError') }} {{ extra.log.status.last_error }}</p>
          </div>
          <ApplicationLogViewer :invoke="props.invoke" :shell="props.shell" :disabled="locked"
            :reload-key="journalRevision" :directory="extra.log?.status.directory ?? null" />
        </section>

        <section v-else-if="extra.tab === 'paths'" aria-labelledby="settings-paths-title" class="paths-page">
          <h2 id="settings-paths-title">{{ t('settings.pathsTitle') }}</h2>
          <p class="settings-hint">{{ t('settings.pathsHint') }}</p>
          <p v-if="extra.error && !pathFeedback" role="alert" class="settings-error">{{ extra.error }}</p>
          <p v-if="!extra.paths && extra.loading" role="status" class="settings-hint">{{ t('settings.pathsLoading') }}</p>
          <p v-else-if="!extra.paths" role="status" class="settings-hint">{{ t('settings.pathsUnavailable') }}</p>
          <div v-if="extra.paths" class="paths-grid">
            <article v-for="card in pathCards" :key="card.kind" class="path-card" :class="{ 'path-card-danger': card.danger }">
              <div class="path-card-header">
                <span class="path-card-icon"><UiIcon :name="card.icon" :size="20" /></span>
                <div class="path-card-titles">
                  <h3>{{ card.title }}</h3>
                  <p class="settings-hint">{{ card.subtitle }}</p>
                </div>
                <span v-if="card.danger" class="path-risk-label">{{ t('settings.pathsRisk') }}</span>
              </div>
              <div class="path-current">
                <span class="path-caption">{{ t('settings.pathsCurrent') }}</span>
                <code :title="savedPath(card.kind)">{{ savedPath(card.kind) }}</code>
              </div>
              <div v-if="editingPath === card.kind" class="path-editor">
                <label :for="`path-edit-${card.kind}`">{{ t('settings.pathsNew') }}</label>
                <input :id="`path-edit-${card.kind}`" :value="draftPath(card.kind)"
                  autocomplete="off" spellcheck="false" :disabled="locked"
                  :aria-invalid="pathDraftStatus(draftPath(card.kind), savedPath(card.kind)) === 'empty'"
                  @input="onPathInput(card.kind, $event)" />
                <p v-if="pathDraftStatus(draftPath(card.kind), savedPath(card.kind)) === 'empty'" class="settings-error">{{ t('settings.pathsRequired') }}</p>
                <p v-else-if="pathDraftStatus(draftPath(card.kind), savedPath(card.kind)) === 'unchanged'" class="settings-hint">{{ t('settings.pathsUnchanged') }}</p>
                <p v-if="card.danger" class="path-warning">{{ t('settings.pathsWarning') }}</p>
                <div class="path-editor-actions">
                  <button type="button" :class="card.danger ? 'secondary' : ''"
                    :disabled="locked || pathDraftStatus(draftPath(card.kind), savedPath(card.kind)) !== 'changed'"
                    @click="submitPath(card.kind)">
                    <span v-if="activePathRequest === card.kind" class="button-spinner" aria-hidden="true"></span>
                    {{ activePathRequest === card.kind ? t('settings.pathsApplying') : card.button }}
                  </button>
                  <button type="button" class="secondary" :disabled="locked" @click="cancelPathEdit">{{ t('settings.cancel') }}</button>
                </div>
              </div>
              <div v-else class="path-toolbar">
                <details class="path-help"><summary>{{ t('settings.details') }}</summary><p class="settings-hint">{{ card.detail }}</p></details>
                <button type="button" class="secondary path-edit-button"
                  :disabled="locked || editingPath !== null" @click="editPath(card.kind)">
                  <UiIcon name="pencil" :size="16" /> {{ t('settings.change') }}
                </button>
              </div>
              <p v-if="pathFeedback?.kind === card.kind" :class="pathFeedback.type === 'error' ? 'settings-error' : 'settings-feedback'"
                :role="pathFeedback.type === 'error' ? 'alert' : 'status'">{{ pathFeedback.text }}</p>
            </article>
          </div>
        </section>

        <section v-else-if="extra.tab === 'readiness'" class="readiness-view" aria-labelledby="settings-readiness-title">
          <header class="readiness-intro">
            <h2 id="settings-readiness-title">{{ t('settings.readinessTitle') }}</h2>
            <p class="settings-hint">{{ t('settings.readinessHint') }}</p>
          </header>

          <div class="readiness-controls settings-subpanel">
            <label for="readiness-connection">{{ t('settings.readinessScope') }}</label>
            <select id="readiness-connection" v-model="readinessConnection" :disabled="locked || checkingReadiness">
              <option value="">{{ t('settings.readinessLocalData') }}</option>
              <option v-for="connection in connectionOptions" :key="connection.id" :value="String(connection.id)">{{ connection.name }} (#{{ connection.id }})</option>
            </select>
            <p class="settings-hint">{{ readinessConnection ? t('settings.readinessScopePg') : t('settings.readinessScopeLocal') }}</p>
            <div class="settings-actions readiness-buttons">
              <button type="button" class="inline-icon-button" :disabled="locked || checkingReadiness || refreshingReadinessConnections" :aria-busy="checkingReadiness" @click="runReadiness">
                <span v-if="checkingReadiness" class="button-spinner" aria-hidden="true"></span>
                <UiIcon v-else name="shield" :size="17" />
                {{ checkingReadiness ? t('settings.readinessChecking') : t('settings.readinessStart') }}
              </button>
              <button type="button" class="secondary icon-action" :disabled="locked || checkingReadiness || refreshingReadinessConnections"
                :aria-busy="refreshingReadinessConnections" :aria-label="t('settings.readinessRefreshAria')"
                :title="t('settings.readinessRefreshTitle')" @click="loadReadinessConnections">
                <UiIcon name="refresh" :size="18" :class="{ 'refresh-spinning': refreshingReadinessConnections }" />
              </button>
            </div>
            <p v-if="readinessOptionsError" role="alert" class="settings-error">{{ readinessOptionsError }}</p>
            <p v-if="extra.error" role="alert" class="settings-error">{{ extra.error }}</p>
          </div>

          <div v-if="!shownReadiness" class="readiness-empty" role="status">
            <p v-if="checkingReadiness" class="settings-hint">{{ t('settings.readinessInProgress') }}</p>
            <p v-else class="settings-hint">{{ t('settings.readinessNotStarted') }}</p>
          </div>
          <template v-else>
            <div class="readiness-section-heading">
              <div>
                <h3>{{ t('settings.readinessLocal') }}</h3>
                <p class="settings-hint">pgXray {{ shownReadiness.app_version }} · {{ t('settings.readinessSupportedPg') }} {{ shownReadiness.supported_postgres }}</p>
              </div>
              <span class="readiness-total">{{ t('settings.readinessChecked', { count: String(localReadinessOverview.total) }) }}</span>
            </div>
            <div class="readiness-overview" role="status" :aria-label="t('settings.readinessLocalResults')">
              <span><strong>{{ localReadinessOverview.ok }}</strong> {{ t('settings.readinessOk') }}</span>
              <span><strong>{{ localReadinessOverview.warning }}</strong> {{ t('settings.readinessLimited') }}</span>
              <span><strong>{{ localReadinessOverview.error }}</strong> {{ t('settings.readinessErrors') }}</span>
              <span v-if="localReadinessOverview.other"><strong>{{ localReadinessOverview.other }}</strong> {{ t('settings.readinessOther') }}</span>
            </div>
            <div class="readiness-grid">
              <article v-for="check in shownReadiness.local_checks" :key="check.key" class="readiness-item">
                <div class="readiness-item-heading">
                  <h4>{{ check.label }}</h4>
                  <span class="readiness-status" :data-status="readinessCheckStatus(check)">{{ readinessStatusLabel(check.status) }}</span>
                </div>
                <template v-if="readinessCheckStatus(check) !== 'ok'">
                  <p v-if="check.method" class="readiness-item-detail">{{ t('settings.readinessMethod') }} <code>{{ check.method }}</code></p>
                  <p class="readiness-item-detail">{{ check.detail }}</p>
                </template>
                <details v-else class="readiness-details">
                  <summary>{{ t('settings.readinessWhatChecked') }}</summary>
                  <p v-if="check.method">{{ t('settings.readinessMethod') }} <code>{{ check.method }}</code></p>
                  <p>{{ check.detail }}</p>
                </details>
              </article>
            </div>

            <section v-if="shownReadiness.postgres" class="readiness-postgres" aria-labelledby="readiness-postgres-title">
              <div class="readiness-section-heading">
                <div>
                  <h3 id="readiness-postgres-title">PostgreSQL</h3>
                  <p class="settings-hint">{{ shownReadiness.postgres.database }} · {{ shownReadiness.postgres.username }} · {{ shownReadiness.postgres.server_version }}</p>
                </div>
                <span class="readiness-status" :data-status="shownReadiness.postgres.support_status === 'supported' ? 'ok' : shownReadiness.postgres.support_status === 'unsupported' ? 'error' : 'warning'">{{ readinessSupportLabel(shownReadiness.postgres.support_status) }}</span>
              </div>
              <p class="settings-hint">{{ shownReadiness.postgres.support_detail }}</p>
              <div class="readiness-overview" role="status" :aria-label="t('settings.readinessPgResults')">
                <span><strong>{{ postgresReadinessOverview.ok }}</strong> {{ t('settings.readinessAvailable') }}</span>
                <span><strong>{{ postgresReadinessOverview.warning }}</strong> {{ t('settings.readinessLimited') }}</span>
                <span><strong>{{ postgresReadinessOverview.error }}</strong> {{ t('settings.readinessErrors') }}</span>
                <span v-if="postgresReadinessOverview.other"><strong>{{ postgresReadinessOverview.other }}</strong> {{ t('settings.readinessOther') }}</span>
              </div>
              <div class="readiness-grid">
                <article v-for="check in shownReadiness.postgres.features" :key="check.key" class="readiness-item">
                  <div class="readiness-item-heading">
                    <h4>{{ check.label }}</h4>
                    <span class="readiness-status" :data-status="readinessCheckStatus(check)">{{ readinessStatusLabel(check.status) }}</span>
                  </div>
                  <p v-if="readinessCheckStatus(check) !== 'ok'" class="readiness-item-detail">{{ check.detail }}</p>
                  <details v-else class="readiness-details"><summary>{{ t('settings.readinessDetails') }}</summary><p>{{ check.detail }}</p></details>
                </article>
              </div>
              <details v-if="shownReadiness.postgres.warnings.length" class="readiness-warnings">
                <summary>{{ t('settings.readinessWarnings', { count: String(shownReadiness.postgres.warnings.length) }) }}</summary>
                <ul><li v-for="(warning, index) in shownReadiness.postgres.warnings" :key="index">{{ warning }}</li></ul>
              </details>
            </section>
            <p v-else class="settings-hint readiness-no-postgres">{{ t('settings.readinessNotChecked') }}</p>
          </template>
        </section>
    </div>
    <dialog ref="userDialog" class="user-dialog" aria-labelledby="user-dialog-title"
      @cancel="cancelDialog" @click="onBackdropClick">
      <form v-if="users.editor" class="user-dialog-form" novalidate autocomplete="off" @submit.prevent="submitUserForm">
        <div class="dialog-heading">
          <h2 id="user-dialog-title">{{ users.editor.kind === "create" ? t('settings.userDialogAdd') : users.editor.kind === "rename" ? t('settings.usersRename') : t('settings.usersPassword') }}</h2>
          <button type="button" class="secondary icon-action" :aria-label="t('settings.userDialogClose')" :disabled="locked" @click="cancelUserForm"><UiIcon name="x" /></button>
        </div>
        <p v-if="users.editor.kind !== 'create'" class="settings-hint">{{ t('settings.userDialogUser') }} {{ users.editor.username }}</p>
        <p v-if="users.error" class="settings-error" role="alert">{{ users.error }}</p>
        <div v-if="users.editor.kind !== 'password'" class="form-field">
          <label for="settings-user-name">{{ t('settings.userLogin') }}</label>
          <input id="settings-user-name" v-model="username" maxlength="64" autocomplete="off"
            :disabled="locked" :aria-invalid="Boolean(fieldErrors.username)"
            :aria-describedby="fieldErrors.username ? 'settings-user-name-error' : undefined"
            @input="fieldErrors.username = undefined; users.error = null" />
          <small v-if="fieldErrors.username" id="settings-user-name-error" class="settings-error" role="alert">{{ fieldErrors.username }}</small>
        </div>
        <template v-if="users.editor.kind !== 'rename'">
          <div class="form-field">
            <label for="settings-user-password">{{ t('settings.userNewPassword') }}</label>
            <PasswordField v-model="password" input-id="settings-user-password" name="new-password" autocomplete="new-password"
              :minlength="12" :maxlength="1024" :disabled="locked" :invalid="Boolean(fieldErrors.password)"
              :described-by="fieldErrors.password ? 'settings-user-password-error' : undefined"
              @update:model-value="fieldErrors.password = undefined; users.error = null" />
            <small v-if="fieldErrors.password" id="settings-user-password-error" class="settings-error" role="alert">{{ fieldErrors.password }}</small>
          </div>
          <div class="form-field">
            <label for="settings-user-confirm">{{ t('settings.userConfirmPassword') }}</label>
            <PasswordField v-model="passwordConfirm" input-id="settings-user-confirm" name="confirm-password" autocomplete="new-password"
              :minlength="12" :maxlength="1024" :disabled="locked" :invalid="Boolean(fieldErrors.confirm)"
              :described-by="fieldErrors.confirm ? 'settings-user-confirm-error' : undefined"
              @update:model-value="fieldErrors.confirm = undefined; users.error = null" />
            <small v-if="fieldErrors.confirm" id="settings-user-confirm-error" class="settings-error" role="alert">{{ fieldErrors.confirm }}</small>
          </div>
        </template>
        <div class="dialog-footer">
          <button type="button" class="secondary" :disabled="locked" @click="cancelUserForm">{{ t('settings.cancel') }}</button>
          <button type="submit" :disabled="locked">
            <span v-if="users.mutating" class="button-spinner" aria-hidden="true"></span>
            {{ users.mutating ? t('settings.userSaving') : users.editor.kind === "create" ? t('settings.userCreate') : t('settings.save') }}
          </button>
        </div>
      </form>
    </dialog>
  </section>
</template>

<style scoped>
.settings-view { width:100%; min-width:0; }
.settings-page-title { font-size:1.45rem; margin:0 0 1.25rem; }
.settings-content { min-width:0; }
.settings-hint { color:var(--text-secondary); font-size:.88rem; line-height:1.45; margin:.3rem 0 0; }
.settings-heading,.settings-actions { display:flex;gap:.65rem;flex-wrap:wrap;align-items:center;justify-content:space-between; }
.settings-subpanel { display:grid;gap:.8rem;padding:1rem;margin:1rem 0;background:var(--surface-2);border:1px solid var(--border);border-radius:.65rem; }
.settings-subpanel label { display:grid;gap:.4rem; }
.settings-subpanel input,.settings-subpanel select { min-width:0;width:100%;box-sizing:border-box; }
.settings-table-wrap { width:100%;overflow-x:auto;margin:1rem 0; }
table { width:100%;border-collapse:collapse; }
th,td { padding:.6rem;border-bottom:1px solid var(--border);text-align:left;vertical-align:top; }
.settings-actions { justify-content:flex-start; }
.settings-actions button { margin:.15rem; }
.inline-icon-button { display:inline-flex; align-items:center; justify-content:center; gap:.45rem; }
.settings-actions > .icon-action { flex:none; }
.danger { background:var(--button-danger);color:var(--button-danger-text); }
.settings-error { color:var(--error);white-space:pre-wrap; }
.log-status-panel { gap:.45rem; }
.log-status-panel h4 { margin:0; }
.log-available { color:var(--accent); }
.log-storage-details summary { cursor:pointer; color:var(--text-secondary); }
.log-directory { overflow-wrap:anywhere; }
.settings-feedback { color:var(--success); margin:.6rem 0; line-height:1.45; }
.settings-subpanel > .settings-log-feedback { margin:0; }
.history-toolbar { display:flex; justify-content:space-between; align-items:end; flex-wrap:wrap; gap:1.2rem; margin:1.1rem 0 .4rem; }
.history-toolbar h3 { margin:0; }
.history-filters { display:flex; align-items:end; gap:.8rem; flex-wrap:wrap; }
.history-filters label { min-width:180px; }
.history-filters select { width:100%; min-width:0; padding:.55rem .7rem; color:var(--text); background:var(--surface-2); border:1px solid var(--border-strong); border-radius:.45rem; }
.settings-subpanel select { color:var(--text); background:var(--surface-2); border:1px solid var(--border-strong); border-radius:.45rem; padding:.55rem; }
.history-summary { margin:.7rem 0; color:var(--text-secondary); }
.history-section-summary { display:flex; flex-wrap:wrap; gap:.4rem .6rem; margin:.65rem 0 .8rem; }
.history-section-chip { display:inline-flex; align-items:center; gap:.45rem; padding:.3rem .55rem; border:1px solid var(--border); border-radius:.5rem; color:var(--text-secondary); font-size:.85rem; }
.history-section-chip strong { color:var(--text); font-variant-numeric:tabular-nums; }
.history-count-cell { min-width:125px; }
.history-count-details { margin-top:.3rem; font-size:.83rem; }
.history-count-details summary { cursor:pointer; color:var(--text-secondary); white-space:nowrap; }
.history-count-list { display:grid; gap:.35rem; margin:.55rem 0 .2rem; min-width:175px; }
.history-count-list > div { display:flex; justify-content:space-between; gap:.75rem; }
.history-count-list strong { font-variant-numeric:tabular-nums; }
.history-pagination { display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:.7rem; margin:.8rem 0; color:var(--text-secondary); font-size:.9rem; }
.history-page-buttons { display:flex; gap:.5rem; }
@media (max-width:760px) { .history-filters { width:100%; } .history-filters label { flex:1; } }

/* Readiness: keep statuses and details local to the check that produced them. */
.readiness-intro h2 { font-size:1.25rem; margin:0; }
.readiness-controls { max-width:none; margin:.9rem 0 1.25rem; gap:.55rem; }
.readiness-controls label { font-weight:600; }
.readiness-buttons { margin-top:.35rem; }
.readiness-section-heading { display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:.7rem; margin:1.2rem 0 .6rem; }
.readiness-section-heading h3 { font-size:1.06rem; margin:0; }
.readiness-total { font-size:.85rem; color:var(--text-secondary); }
.readiness-overview { display:flex; flex-wrap:wrap; gap:.5rem 1.4rem; border:1px solid var(--border); background:var(--surface-2); border-radius:.6rem; padding:.8rem 1rem; font-size:.87rem; color:var(--text-secondary); }
.readiness-overview strong { color:var(--text); margin-right:.2rem; font-size:1.05rem; }
.readiness-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,300px),1fr)); gap:.75rem; }
.readiness-item { min-width:0; padding:.85rem 1rem; border:1px solid var(--border); border-radius:.6rem; background:var(--surface-2); }
.readiness-item-heading { display:flex; align-items:center; justify-content:space-between; gap:.7rem; }
.readiness-item h4 { margin:0; font-size:.94rem; overflow-wrap:anywhere; }
.readiness-status { flex-shrink:0; display:inline-flex; align-items:center; border:1px solid var(--border-strong); border-radius:1rem; padding:.18rem .55rem; font-size:.76rem; color:var(--text-secondary); }
.readiness-status[data-status="ok"] { color:var(--success); border-color:var(--success); }
.readiness-status[data-status="warning"] { color:var(--message); }
.readiness-status[data-status="error"] { color:var(--error); border-color:var(--error); }
.readiness-details { margin-top:.55rem; font-size:.83rem; color:var(--text-secondary); }
.readiness-details summary,.readiness-warnings summary { cursor:pointer; }
.readiness-details p,.readiness-item-detail { margin:.5rem 0 0; overflow-wrap:anywhere; font-size:.84rem; line-height:1.5; color:var(--text-secondary); }
.readiness-postgres { margin-top:1.6rem; }
.readiness-warnings { margin-top:.85rem; padding:.75rem 1rem; border:1px solid var(--border); border-radius:.6rem; font-size:.87rem; }
.readiness-warnings li { margin:.4rem 0; overflow-wrap:anywhere; }
.readiness-no-postgres { margin-top:1rem; }
.readiness-empty { padding:1.1rem; border:1px dashed var(--border-strong); border-radius:.6rem; }
.users-topline { display:flex; justify-content:space-between; align-items:center; gap:1rem; flex-wrap:wrap; }
.users-topline h2 { margin:0; font-size:1.25rem; }
.users-tools { display:flex; align-items:center; gap:.5rem; }
.users-tools button { display:inline-flex; align-items:center; justify-content:center; gap:.4rem; }
.refresh-spinning { animation:pgxray-spin .7s linear infinite; transform-origin:center; }
@media (prefers-reduced-motion: reduce) { .refresh-spinning { animation:none; } }
.icon-action { width:2.65rem; height:2.65rem; padding:0; display:inline-grid;place-items:center; }
.users-list { margin-top:1rem; border-top:1px solid var(--border); }
.user-row { display:flex; align-items:center; justify-content:space-between; gap:1rem; border-bottom:1px solid var(--border); min-height:3.7rem; padding:.2rem .5rem; }
.user-ident { display:flex; align-items:center; gap:.65rem; min-width:0; }
.user-name { font-weight:600; overflow-wrap:anywhere; }
.current-user { font-size:.74rem; color:var(--text-secondary); padding:.15rem .45rem; border:1px solid var(--border); border-radius:1rem; }
.user-menu { position:relative; flex-shrink:0; }
.menu-trigger { list-style:none; display:grid; place-items:center; width:2.5rem; height:2.5rem; cursor:pointer; border:1px solid transparent; border-radius:.45rem; color:var(--text-secondary); }
.menu-trigger::-webkit-details-marker { display:none; }
.menu-trigger:hover,.user-menu[open] .menu-trigger { color:var(--text); background:var(--surface-2); border-color:var(--border); }
.menu-trigger:focus-visible { outline:2px solid var(--focus-ring, var(--accent)); }
.menu-content { position:absolute; z-index:10; top:100%; right:0; width:200px; padding:.35rem; border:1px solid var(--border-strong); border-radius:.6rem; background:var(--surface); box-shadow:0 10px 32px #0005; }
.menu-content button { display:flex; align-items:center; gap:.6rem; width:100%; background:transparent; border:0; color:var(--text); padding:.6rem; text-align:left; font-size:.86rem; }
.menu-content button:hover:not(:disabled) { background:var(--surface-2); }
.menu-content .menu-danger { color:var(--error); }
.menu-divider { border-top:1px solid var(--border); margin:.3rem 0; }
.user-dialog { width:min(94vw,460px); max-height:min(90vh,760px); box-sizing:border-box; overflow-y:auto; border:1px solid var(--border-strong); border-radius:.8rem; background:var(--surface); color:var(--text); padding:1.3rem; box-shadow:0 18px 60px #0008; }
.user-dialog::backdrop { background:rgba(3,9,15,.7); }
.user-dialog-form { display:grid; gap:1rem; width:100%; max-width:none; margin:0; }
.dialog-heading { display:flex; justify-content:space-between; align-items:center; gap:.5rem; }
.dialog-heading h2 { font-size:1.25rem; margin:0; }
.form-field { display:grid; gap:.4rem; }
.form-field label { font-weight:500; }
.form-field input { width:100%; min-width:0; box-sizing:border-box; }
.dialog-footer { display:flex; justify-content:flex-end; gap:.6rem; border-top:1px solid var(--border); padding-top:.9rem; }
/* Path settings: read-first layout; editing is explicit and destructive actions are subdued. */
.paths-page > h2 { margin:0 0 .4rem; font-size:1.25rem; }
.paths-grid { display:grid; gap:.8rem; margin-top:1rem; max-width:1120px; }
.path-card { background:var(--surface-2); border:1px solid var(--border); border-radius:.75rem; padding:1rem 1.1rem; display:grid; gap:.75rem; }
.path-card-danger { border-inline-start:3px solid var(--border-strong); }
.path-card-header { display:flex; align-items:center; gap:.85rem; min-width:0; }
.path-card-icon { width:2.5rem; height:2.5rem; flex:none; border-radius:.6rem; display:grid; place-items:center; color:var(--text-secondary); background:var(--surface); border:1px solid var(--border); }
.path-card-titles { min-width:0; flex:1; }
.path-card-titles h3 { font-size:1rem; margin:0; }
.path-card-titles p { margin:.25rem 0 0; }
.path-risk-label { font-size:.75rem; color:var(--text-secondary); border:1px solid var(--border); padding:.2rem .5rem; border-radius:1rem; white-space:nowrap; }
.path-current { display:grid; gap:.3rem; background:var(--surface); border:1px solid var(--border); border-radius:.5rem; padding:.65rem .85rem; }
.path-caption { font-size:.74rem; color:var(--text-secondary); }
.path-current code { font-family:ui-monospace,SFMono-Regular,Consolas,monospace; font-size:.84rem; color:var(--text); white-space:normal; overflow-wrap:anywhere; user-select:text; }
.path-toolbar { display:flex; justify-content:space-between; align-items:center; gap:1rem; }
.path-edit-button { display:inline-flex; align-items:center; gap:.45rem; white-space:nowrap; }
.path-help { min-width:0; flex:1; }
.path-help summary { width:max-content; max-width:100%; cursor:pointer; color:var(--text-secondary); font-size:.84rem; }
.path-help p { padding-top:.5rem; }
.path-editor { border-top:1px solid var(--border); padding-top:.8rem; display:grid; gap:.5rem; }
.path-editor label { font-weight:500; font-size:.86rem; }
.path-editor input { width:100%; box-sizing:border-box; }
.path-editor-actions { display:flex; align-items:center; flex-wrap:wrap; gap:.55rem; padding-top:.4rem; }
.path-warning { font-size:.83rem; line-height:1.4; color:var(--text-secondary); margin:.2rem 0; }
.path-card > .settings-feedback, .path-card > .settings-error { margin:0; }
@media (max-width:650px) { .path-card-header { flex-wrap:wrap; } .path-risk-label { margin-left:3.3rem; } .path-editor-actions button { flex:1; } }
@media (max-width:650px) { .users-topline { align-items:flex-start; } .user-dialog { padding:1rem; } }
</style>
