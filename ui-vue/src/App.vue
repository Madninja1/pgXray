<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";
import { createPgXrayInvoke } from "./api/invoke.js";
import { createAppController } from "./app/controller.js";
import { durationText, aggregatePolls, type BackgroundOperation } from "./core/background-operations.js";
import { createShellState, MAIN_VIEWS, type MainView } from "./core/shell-state.js";
import ConnectionsView from "./features/connections/ConnectionsView.vue";
import type { SavedConnection } from "./features/connections/controller.js";
import { shortcutForKey, type QuickConnectionSelection } from "./core/connection-shortcuts.js";
import AuthView from "./features/auth/AuthView.vue";
import { authInlineError } from "./features/auth/auth-feedback.js";
import SettingsView from "./features/settings/SettingsView.vue";
import ToolsView from "./features/tools/ToolsView.vue";
import { resolveEvidenceRoute } from "./features/tools/evidence-navigation.js";
import type { CaseNavigationTarget } from "./features/tools/investigation.js";
import ActivityView from "./features/activity/ActivityView.vue";
import LocksView from "./features/locks/LocksView.vue";
import IndexHealthView from "./features/index-health/IndexHealthView.vue";
import IoWalView from "./features/io-wal/IoWalView.vue";
import AutovacuumView from "./features/autovacuum/AutovacuumView.vue";
import XidView from "./features/xid/XidView.vue";
import PgssView from "./features/pgss/PgssView.vue";
import PgSettingsView from "./features/pg-settings/PgSettingsView.vue";
import WhatChangedView from "./features/what-changed/WhatChangedView.vue";
import HistoryView from "./features/history/HistoryView.vue";
import { createSnapshotScheduler, createSnapshotSchedulerState } from "./features/snapshots/scheduler.js";
import SessionsView from "./features/sessions/SessionsView.vue";
import type { PsqlSessionInfo } from "./features/sessions/controller.js";
import UiIcon from "./components/UiIcon.vue";
import { SETTINGS_TABS, type SettingsTab } from "./features/settings/extra-controller.js";
import { t } from "./features/i18n/locale-service.js";

const state = reactive(createShellState());
const bridge = isTauri() ? createPgXrayInvoke(tauriInvoke) : null;
const controller = createAppController(state, bridge);
const quickProfiles = ref<SavedConnection[]>([]);
function setQuickProfiles(profiles: SavedConnection[]): void { quickProfiles.value = profiles; }
const quickSelection = ref<QuickConnectionSelection | null>(null);
const shortcutNotice = ref("");
let shortcutNoticeTimer: ReturnType<typeof setTimeout> | null = null;
let quickLoadSequence = 0;
let quickSerial = 0;
async function loadQuickProfiles(): Promise<void> {
  if (!bridge || state.screen !== "workspace" || state.busy) return;
  const authGeneration = state.authGeneration, call = ++quickLoadSequence;
  try {
    const profiles = await bridge<SavedConnection[]>("list_connections");
    if (call === quickLoadSequence && state.authGeneration === authGeneration && state.screen === "workspace") quickProfiles.value = profiles;
  } catch { /* A keyboard helper must not interfere with normal navigation. */ }
}
function onQuickKeyDown(event: KeyboardEvent): void {
  if (state.screen !== "workspace" || state.mainView === "sessions" || state.busy || !bridge) return;
  const connectionId = shortcutForKey(event, quickProfiles.value, document.querySelector("dialog[open]") !== null);
  if (connectionId === null) return;
  event.preventDefault();
  const profile = quickProfiles.value.find(item => item.id === connectionId);
  quickSelection.value = { connectionId, serial: ++quickSerial };
  shortcutNotice.value = t("shellExtra.shortcut", { key: event.key, profile: profile?.name ?? `#${connectionId}` });
  if (shortcutNoticeTimer !== null) clearTimeout(shortcutNoticeTimer);
  shortcutNoticeTimer = setTimeout(() => { shortcutNotice.value = ""; shortcutNoticeTimer = null; }, 4500);
}
watch(() => [state.screen, state.authGeneration, state.busy] as const, () => {
  ++quickLoadSequence;
  if (state.screen !== "workspace") { quickProfiles.value = []; quickSelection.value = null; shortcutNotice.value = ""; }
  else void loadQuickProfiles();
});
const snapshotState = reactive(createSnapshotSchedulerState());
const snapshotActions = createSnapshotScheduler(state, snapshotState, bridge);
watch(() => [state.screen, state.authGeneration], () => { void snapshotActions.syncAuth(); });
const running = reactive<Partial<Record<BackgroundOperation["id"], BackgroundOperation>>>({});
const runningOperations = computed(() => Object.values(running).filter((item): item is BackgroundOperation => !!item));
const activePolls = computed(() => aggregatePolls(runningOperations.value));
const clock = ref(Date.now());
let clockTimer: ReturnType<typeof setInterval> | null = null;
function updateOperation(operation: BackgroundOperation | null, id: BackgroundOperation["id"]): void {
  if (operation) running[id] = operation;
  else delete running[id];
}
function openOperation(id: BackgroundOperation["id"]): void {
  if (id === "locks") diagnosticsTab.value = "locks";
  controller.selectMainView(id === "locks" ? "diagnostics" : "activity");
}
onMounted(() => {
  clockTimer = setInterval(() => { clock.value = Date.now(); }, 1000);
  window.addEventListener("keydown", onQuickKeyDown, true);
  window.addEventListener("focus", loadQuickProfiles);
  snapshotActions.start();
  void loadQuickProfiles();
});
onUnmounted(() => {
  if (clockTimer !== null) clearInterval(clockTimer);
  window.removeEventListener("keydown", onQuickKeyDown, true);
  window.removeEventListener("focus", loadQuickProfiles);
  if (shortcutNoticeTimer !== null) clearTimeout(shortcutNoticeTimer);
  snapshotActions.stop();
});
const authMode = computed(() => state.screen === "registration" || state.screen === "login" ? state.screen : null);
const authError = computed(() => authInlineError(authMode.value, state.message));
function clearAuthError(): void {
  if (authError.value !== null) state.message = { text: "", error: false };
}
const labels = computed<Record<MainView, string>>(() => ({
  connections: t("navigation.connections"),
  diagnostics: t("navigation.diagnostics"),
  activity: t("navigation.activity"),
  history: t("navigation.history"),
  changes: t("navigation.changes"),
  tools: t("navigation.tools"),
  sessions: t("navigation.sessions"),
  settings: t("navigation.settings"),
}));

const settingsTab = ref<SettingsTab>("users");
const changesTab = ref<"what-changed" | "config">("what-changed");
const diagnosticsTab = ref<"locks" | "index-health" | "io-wal" | "autovacuum" | "xid" | "pgss">("locks");
const historyXidNavigationToken = ref(0);
const historyAvNavigationToken = ref(0);
const historyAvPreferredConnectionId = ref<number | null>(null);
const historyAvPreferredRelationOid = ref<number | null>(null);
const avNavigationToken = ref(0);
const avPreferredConnectionId = ref<number | null>(null);
const avPreferredRelationOid = ref<number | null>(null);
const indexNavigationToken = ref(0);
const indexPreferredConnectionId = ref<number | null>(null);
const historyIndexNavigationToken = ref(0);
const historyIndexPreferredConnectionId = ref<number | null>(null);
const historyIoWalNavigationToken = ref(0);
const historyIoWalPreferredConnectionId = ref<number | null>(null);
const historySettingsNavigationToken = ref(0);
const historySettingsPreferredConnectionId = ref<number | null>(null);
const historyPgssNavigationToken = ref(0);
const historyPgssPreferredConnectionId = ref<number | null>(null);
const historyPgssPreferredQueryid = ref<string | null>(null);
const pgssNavigationToken = ref(0);
const pgssPreferredConnectionId = ref<number | null>(null);
const pgssPreferredQueryid = ref<string | null>(null);
const xidPreferredConnectionId = ref<number | null>(null);
const sessionsView = ref<InstanceType<typeof SessionsView> | null>(null);
const sessions = ref<PsqlSessionInfo[]>([]);
async function openConnectionSession(connectionId: number, forceNew: boolean): Promise<void> {
  if (state.busy || state.screen !== "workspace") return;
  controller.selectMainView("sessions");
  await nextTick();
  if (!sessionsView.value) throw new Error(t("shellExtra.terminalUnready"));
  await sessionsView.value.openConnection(connectionId, forceNew);
}

const navIcons = {
  connections: "plug", diagnostics: "activity", activity: "chart", history: "history",
  changes: "compare", tools: "tools", sessions: "terminal", settings: "settings",
} as const;
const settingsIcons = {
  users: "users", interface: "palette", history: "history", snapshots: "database", logs: "scroll",
  paths: "folder", readiness: "shield",
} as const;
const settingsLabels = computed<Record<SettingsTab, string>>(() => ({
  users: t("settingsTabs.users"),
  interface: t("settingsTabs.interface"),
  history: t("settingsTabs.history"),
  snapshots: t("settingsTabs.snapshots"),
  logs: t("settingsTabs.logs"),
  paths: t("settingsTabs.paths"),
  readiness: t("settingsTabs.readiness"),
}));
function openXidAnalyzer(connectionId: number | null): void {
  if (state.busy) return;
  xidPreferredConnectionId.value = connectionId;
  diagnosticsTab.value = "xid";
  controller.selectMainView("diagnostics");
}
function openXidHistory(): void {
  if (state.busy) return;
  historyXidNavigationToken.value++;
  controller.selectMainView("history");
}
function openAvHistory(connectionId: number | null, relationOid: number | null): void {
  if (state.busy) return;
  historyAvPreferredConnectionId.value = connectionId;
  historyAvPreferredRelationOid.value = relationOid;
  historyAvNavigationToken.value++;
  controller.selectMainView("history");
}
function openAvAnalyzer(connectionId: number | null, relationOid: number | null): void {
  if (state.busy) return;
  avPreferredConnectionId.value = connectionId;
  avPreferredRelationOid.value = relationOid;
  avNavigationToken.value++;
  diagnosticsTab.value = "autovacuum";
  controller.selectMainView("diagnostics");
}
function openIoWalHistory(connectionId: number | null): void {
  if (state.busy) return;
  historyIoWalPreferredConnectionId.value = connectionId;
  historyIoWalNavigationToken.value++;
  controller.selectMainView("history");
}
function openIndexHistory(connectionId: number | null): void {
  if (state.busy) return;
  historyIndexPreferredConnectionId.value = connectionId;
  historyIndexNavigationToken.value++;
  controller.selectMainView("history");
}
function openIndexAnalyzer(connectionId: number | null): void {
  if (state.busy) return;
  indexPreferredConnectionId.value = connectionId;
  indexNavigationToken.value++;
  diagnosticsTab.value = "index-health";
  controller.selectMainView("diagnostics");
}
function openIoWalAnalyzer(): void {
  if (state.busy) return;
  diagnosticsTab.value = "io-wal";
  controller.selectMainView("diagnostics");
}
function openSettingsHistory(connectionId: number | null): void {
  if (state.busy) return;
  historySettingsPreferredConnectionId.value = connectionId;
  historySettingsNavigationToken.value++;
  controller.selectMainView("history");
}
function openCurrentSettings(): void {
  if (state.busy) return;
  changesTab.value = "config";
  controller.selectMainView("changes");
}
function openPgssHistory(connectionId: number | null, queryid: string | null): void {
  if (state.busy) return;
  historyPgssPreferredConnectionId.value = connectionId;
  historyPgssPreferredQueryid.value = queryid;
  historyPgssNavigationToken.value++;
  controller.selectMainView("history");
}
function openPgssAnalyzer(connectionId: number | null, queryid: string | null): void {
  if (state.busy) return;
  pgssPreferredConnectionId.value = connectionId;
  pgssPreferredQueryid.value = queryid;
  pgssNavigationToken.value++;
  diagnosticsTab.value = "pgss";
  controller.selectMainView("diagnostics");
}
const evidenceNotice = ref("");
function navigateCaseEvidence(value: { connectionId: number; target: CaseNavigationTarget }): void {
  if (state.busy || state.screen !== "workspace") return;
  const route = resolveEvidenceRoute(value.connectionId, value.target);
  if (route.view === "pgss-history") openPgssHistory(route.connectionId, route.queryid);
  else if (route.view === "pgss-live") openPgssAnalyzer(route.connectionId, route.queryid);
  else if (route.view === "autovacuum") openAvAnalyzer(route.connectionId, route.relationOid);
  else if (route.view === "locks") { diagnosticsTab.value = "locks"; controller.selectMainView("diagnostics"); }
  else if (route.view === "artifact") return; // Same artifact is available inside its Case.
  evidenceNotice.value = "notice" in route ? route.notice : "";
}
function selectSettings(tab: SettingsTab): void {
  if (state.busy) return;
  settingsTab.value = tab;
  if (state.mainView !== "settings") controller.selectMainView("settings");
}
onMounted(() => { void controller.initialize(); });
</script>

<template>
  <div class="pgxray-app" :class="{ 'workspace-mode': state.screen === 'workspace' }">
  <header class="header">
    <div class="brand"><UiIcon name="database" :size="21" /><strong>pgXray</strong><span class="preview-tag">Vue Preview</span></div>
    <div v-if="state.screen === 'workspace'" class="account-controls">
      <div class="background-operations" :aria-label="t('shellExtra.active')">
        <span v-if="runningOperations.length" class="poll-summary" :title="t('shellExtra.pollTitle')">{{ t("shellExtra.pollSummary", { monitors: String(runningOperations.length), requests: String(activePolls.requests), errors: String(activePolls.failed), avg: String(activePolls.requests ? Math.round(activePolls.totalMs / activePolls.requests) : 0) }) }}</span>
        <button v-for="operation in runningOperations" :key="operation.id" type="button" class="background-operation"
          :title="t('shellExtra.open', { title: operation.title })" @click="openOperation(operation.id)">
          <span class="live-dot" aria-hidden="true"></span>
          {{ operation.title }} · {{ operation.connection }} · {{ durationText(operation.startedAt, clock) }}
        </button>
      </div>
      <span class="account-name"><UiIcon name="users" :size="16" />{{ state.currentUsername }}</span>
      <button type="button" class="secondary logout-button" :disabled="state.busy" :aria-busy="state.busy" @click="controller.logout">
        <span v-if="state.busy" class="button-spinner" aria-hidden="true"></span>
        <UiIcon v-else name="logout" :size="17" />
        {{ state.busy ? t("shellExtra.loggingOut") : t("shellExtra.logout") }}
      </button>
    </div>
  </header>
  <main class="layout" :class="{ 'layout-workspace': state.screen === 'workspace' }">
    <p v-if="state.message.error && state.message.text && authError === null" class="status error" role="alert">
      {{ state.message.text }}
    </p>
    <AuthView v-if="authMode" :mode="authMode" :busy="state.busy" :available="bridge !== null"
      :login="controller.login" :register-initial-user="controller.registerInitialUser"
      :auth-error="authError" :clear-auth-error="clearAuthError" />
    <div v-else-if="state.screen === 'workspace'" class="workspace-shell">
      <aside class="workspace-sidebar" aria-label="pgXray navigation">
        <nav class="nav" aria-label="Main sections">
          <template v-for="view in MAIN_VIEWS" :key="view">
            <button type="button" class="nav-item" :class="{ selected: state.mainView === view }"
              :aria-current="state.mainView === view ? 'page' : undefined"
              :disabled="state.busy" @click="controller.selectMainView(view)">
              <UiIcon :name="navIcons[view]" :size="18" /> <span>{{ labels[view] }}</span>
            </button>
            <nav v-if="view === 'diagnostics' && state.mainView === 'diagnostics'" class="subnav" :aria-label="t('shellExtra.diagnosticsAria')">
              <button type="button" class="subnav-item" :class="{ selected: diagnosticsTab === 'locks' }" @click="diagnosticsTab = 'locks'">Locks / Waits</button>
              <button type="button" class="subnav-item" :class="{ selected: diagnosticsTab === 'index-health' }" @click="diagnosticsTab = 'index-health'">Index Health</button>
              <button type="button" class="subnav-item" :class="{ selected: diagnosticsTab === 'io-wal' }" @click="diagnosticsTab = 'io-wal'">I/O & WAL</button>
              <button type="button" class="subnav-item" :class="{ selected: diagnosticsTab === 'autovacuum' }" @click="diagnosticsTab = 'autovacuum'">Autovacuum / Autoanalyze</button>
              <button type="button" class="subnav-item" :class="{ selected: diagnosticsTab === 'xid' }" @click="diagnosticsTab = 'xid'">XID / Wraparound</button>
              <button type="button" class="subnav-item" :class="{ selected: diagnosticsTab === 'pgss' }" @click="diagnosticsTab = 'pgss'">SQL workload / PGSS</button>
            </nav>
            <nav v-if="view === 'changes' && state.mainView === 'changes'" class="subnav" :aria-label="t('shellExtra.changesAria')">
              <button type="button" class="subnav-item" :class="{ selected: changesTab === 'what-changed' }" @click="changesTab = 'what-changed'">What Changed?</button>
              <button type="button" class="subnav-item" :class="{ selected: changesTab === 'config' }" @click="changesTab = 'config'">Configuration Drift</button>
            </nav>
            <nav v-if="view === 'settings' && state.mainView === 'settings'" class="subnav" aria-label="Settings sections">
              <button v-for="tab in SETTINGS_TABS" :key="tab" type="button" class="subnav-item"
                :class="{ selected: settingsTab === tab }" :aria-current="settingsTab === tab ? 'page' : undefined"
                :disabled="state.busy" @click="selectSettings(tab)">
                <UiIcon :name="settingsIcons[tab]" :size="16" /> {{ settingsLabels[tab] }}
              </button>
            </nav>
          </template>
        </nav>
      </aside>
      <div class="workspace-content">
        <p v-if="shortcutNotice" role="status" class="evidence-navigation-notice">{{ shortcutNotice }} <button type="button" class="secondary" @click="shortcutNotice = ''">{{ t("shellExtra.close") }}</button></p>
        <p v-if="evidenceNotice && (state.mainView === 'history' || state.mainView === 'diagnostics')" role="status" class="evidence-navigation-notice">{{ evidenceNotice }} <button type="button" class="secondary" @click="evidenceNotice = ''">{{ t("shellExtra.close") }}</button></p>
        <SessionsView v-if="bridge" ref="sessionsView" v-show="state.mainView === 'sessions'"
          :shell="state" :invoke="bridge" :visible="state.mainView === 'sessions'"
          @sessions-changed="sessions = $event" />
        <ActivityView v-if="bridge" v-show="state.mainView === 'activity'"
          :shell="state" :invoke="bridge" :visible="state.mainView === 'activity'"
          @operation-change="updateOperation($event, 'activity')" :quick-selection="quickSelection" />
        <LocksView v-if="bridge" v-show="state.mainView === 'diagnostics' && diagnosticsTab === 'locks'" :shell="state" :invoke="bridge" :visible="state.mainView === 'diagnostics' && diagnosticsTab === 'locks'"
          @operation-change="updateOperation($event, 'locks')" :quick-selection="quickSelection" />
        <IndexHealthView v-if="bridge" v-show="state.mainView === 'diagnostics' && diagnosticsTab === 'index-health'" :shell="state" :invoke="bridge" :visible="state.mainView === 'diagnostics' && diagnosticsTab === 'index-health'" :navigation-token="indexNavigationToken" :preferred-connection-id="indexPreferredConnectionId" @open-history="openIndexHistory" :quick-selection="quickSelection" />
        <IoWalView v-if="bridge" v-show="state.mainView === 'diagnostics' && diagnosticsTab === 'io-wal'" :shell="state" :invoke="bridge" :visible="state.mainView === 'diagnostics' && diagnosticsTab === 'io-wal'" @open-history="openIoWalHistory" :quick-selection="quickSelection" />
        <AutovacuumView v-if="bridge" v-show="state.mainView === 'diagnostics' && diagnosticsTab === 'autovacuum'" :shell="state" :invoke="bridge" :visible="state.mainView === 'diagnostics' && diagnosticsTab === 'autovacuum'" :navigation-token="avNavigationToken" :preferred-connection-id="avPreferredConnectionId" :preferred-relation-oid="avPreferredRelationOid" @open-history="openAvHistory" @open-pgss="openPgssAnalyzer" :quick-selection="quickSelection" />
        <XidView v-if="bridge" v-show="state.mainView === 'diagnostics' && diagnosticsTab === 'xid'" :shell="state" :invoke="bridge" :visible="state.mainView === 'diagnostics' && diagnosticsTab === 'xid'" :preferred-connection-id="xidPreferredConnectionId" @open-history="openXidHistory" :quick-selection="quickSelection" />
        <PgssView v-if="bridge" v-show="state.mainView === 'diagnostics' && diagnosticsTab === 'pgss'" :shell="state" :invoke="bridge" :visible="state.mainView === 'diagnostics' && diagnosticsTab === 'pgss'" :navigation-token="pgssNavigationToken" :preferred-connection-id="pgssPreferredConnectionId" :preferred-queryid="pgssPreferredQueryid" @open-history="openPgssHistory" :quick-selection="quickSelection" />
        <HistoryView v-if="bridge" v-show="state.mainView === 'history'" :shell="state" :invoke="bridge" :visible="state.mainView === 'history'" :xid-navigation-token="historyXidNavigationToken" :pgss-navigation-token="historyPgssNavigationToken" :pgss-preferred-connection-id="historyPgssPreferredConnectionId" :pgss-preferred-queryid="historyPgssPreferredQueryid" :av-navigation-token="historyAvNavigationToken" :av-preferred-connection-id="historyAvPreferredConnectionId" :av-preferred-relation-oid="historyAvPreferredRelationOid" :settings-navigation-token="historySettingsNavigationToken" :settings-preferred-connection-id="historySettingsPreferredConnectionId" :io-wal-navigation-token="historyIoWalNavigationToken" :io-wal-preferred-connection-id="historyIoWalPreferredConnectionId" :index-navigation-token="historyIndexNavigationToken" :index-preferred-connection-id="historyIndexPreferredConnectionId" @open-index-analyzer="openIndexAnalyzer" @open-settings-current="openCurrentSettings" @open-io-wal-analyzer="openIoWalAnalyzer" @open-av-analyzer="openAvAnalyzer" @open-schedules="selectSettings('snapshots')" @open-xid-analyzer="openXidAnalyzer" @open-pgss-analyzer="openPgssAnalyzer" :quick-selection="quickSelection" />
        <WhatChangedView v-if="bridge" v-show="state.mainView === 'changes' && changesTab === 'what-changed'" :shell="state" :invoke="bridge" :visible="state.mainView === 'changes' && changesTab === 'what-changed'" @open-history="controller.selectMainView('history')" @open-schedules="selectSettings('snapshots')" :quick-selection="quickSelection" />
        <PgSettingsView v-if="bridge" v-show="state.mainView === 'changes' && changesTab === 'config'" :shell="state" :invoke="bridge" :visible="state.mainView === 'changes' && changesTab === 'config'" @open-history="openSettingsHistory" :quick-selection="quickSelection" />
        <ToolsView v-if="bridge" v-show="state.mainView === 'tools'" :shell="state" :invoke="bridge" :visible="state.mainView === 'tools'" @navigate-evidence="navigateCaseEvidence" :quick-selection="quickSelection" />
        <ConnectionsView v-if="state.mainView === 'connections' && bridge" :shell="state" :invoke="bridge"
          :shell-busy="state.busy" :sessions="sessions" :open-session="openConnectionSession" @connections-changed="setQuickProfiles" />
        <SettingsView v-else-if="state.mainView === 'settings' && bridge" :shell="state" :invoke="bridge" :selected-tab="settingsTab" :snapshot-state="snapshotState" :snapshot-actions="snapshotActions" />
        <section v-else-if="state.mainView !== 'sessions' && state.mainView !== 'activity' && state.mainView !== 'diagnostics' && state.mainView !== 'changes' && state.mainView !== 'history' && state.mainView !== 'tools'" class="placeholder">
          <h1>{{ labels[state.mainView] }}</h1>
          <p>{{ t("shellExtra.placeholder") }}</p>
        </section>
      </div>
    </div>
    <p v-else-if="!state.message.error" class="loading" role="status">
      <span class="button-spinner" aria-hidden="true"></span>{{ t("shellExtra.loading") }}
    </p>
  </main>
  </div>
</template>

<style>
:root { font-family: system-ui, sans-serif; background: var(--bg); color: var(--text); }
body { margin: 0; }
.pgxray-app.workspace-mode { height:100dvh; display:flex; flex-direction:column; overflow:hidden; }
.pgxray-app.workspace-mode > .header { flex:none; }
button, input { font: inherit; }
button { cursor: pointer; padding: .65rem 1rem; border: 1px solid var(--border-strong); border-radius: .5rem; background: var(--button-primary); color: var(--button-primary-text); }
button:disabled { cursor: not-allowed; opacity: .55; }
/* WebView can draw a platform-colored focus ring after pointer clicks.
   Keep an explicit keyboard focus indicator, without a mouse-click halo. */
button:focus:not(:focus-visible), summary:focus:not(:focus-visible) { outline:none; }
button:focus-visible, summary:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible {
  outline:2px solid var(--focus-ring, var(--accent)); outline-offset:2px;
}
input { padding: .65rem; border-radius: .5rem; border: 1px solid var(--border-strong); background: var(--surface-2); color: var(--text); }
label { display: grid; gap: .4rem; }
form { display: grid; gap: 1rem; max-width: 28rem; }
.header { display:flex; justify-content:space-between; align-items:center; gap:1rem; padding:.7rem 1.6rem; background:var(--surface); border-bottom:1px solid var(--border); }
.brand,.account-controls,.account-name { display:flex; align-items:center; gap:.6rem; }
.brand strong { letter-spacing:.02em; }
.preview-tag { color:var(--text-secondary); border:1px solid var(--border); padding:.2rem .5rem; border-radius:.4rem; font-size:.72rem; }
.account-name { color:var(--text-secondary); font-size:.9rem; }
.logout-button { display:inline-flex; align-items:center; gap:.3rem; }
.layout { max-width:1080px; margin:1.5rem auto; padding:0 1rem; }
.layout-workspace { max-width:none; margin:0; padding:0; flex:1; min-height:0; overflow:hidden; display:flex; flex-direction:column; }
.status { color:var(--message); padding:0 1.5rem; }
.status.error { color:var(--error); }
.secondary { background:var(--button-secondary); color:var(--button-secondary-text); }
.workspace-shell { display:grid; grid-template-columns:220px minmax(0,1fr); flex:1; min-height:0; }
.workspace-sidebar { border-right:1px solid var(--border); background:var(--surface); padding:1.15rem .7rem;
  align-self:stretch; box-sizing:border-box; height:100%; min-height:0;
  overflow-y:auto; overscroll-behavior:contain; scrollbar-gutter:stable; }
.nav { display:flex; flex-direction:column; gap:.2rem; }
.nav-item,.subnav-item { display:flex; align-items:center; gap:.65rem; width:100%; padding:.65rem .8rem;
  border:1px solid transparent; background:transparent; color:var(--text-secondary); text-align:left; border-radius:.5rem; font-size:.92rem; }
.nav-item:hover:not(:disabled),.subnav-item:hover:not(:disabled) { background:var(--surface-2); color:var(--text); }
.nav-item.selected { background:var(--button-secondary); color:var(--text); border-color:var(--border-strong); }
.nav-item:focus-visible,.subnav-item:focus-visible { outline:2px solid var(--focus-ring, var(--accent)); outline-offset:1px; }
.subnav { display:flex; flex-direction:column; gap:.12rem; margin:.2rem 0 .45rem 1rem; padding-left:.5rem; border-left:1px solid var(--border-strong); }
.subnav-item { padding:.48rem .6rem; font-size:.84rem; }
.subnav-item.selected { background:var(--surface-2); color:var(--text); font-weight:600; }
.workspace-content { min-width:0; width:100%; max-width:1500px; box-sizing:border-box; padding:1.65rem clamp(1rem,3vw,3rem); height:100%; overflow-y:auto; overscroll-behavior:contain; }
.placeholder { border:1px dashed var(--border-strong); border-radius:.5rem; padding:1rem; }
/* Shared with authentication and settings; no spinner dependency required. */
.button-spinner { display:inline-block; width:1em; height:1em; margin-right:.45rem; box-sizing:border-box; border:.14em solid currentColor; border-right-color:transparent; border-radius:50%; vertical-align:-.12em; animation:pgxray-spin .7s linear infinite; }
@keyframes pgxray-spin { to { transform:rotate(360deg); } }
@media (prefers-reduced-motion:reduce) { .button-spinner { animation:none; } }
@media (max-width:760px) { .pgxray-app.workspace-mode { height:auto; min-height:100dvh; display:block; overflow:visible; } .layout-workspace { overflow:visible; } .workspace-shell { grid-template-columns:1fr; height:auto; } .workspace-sidebar { position:static; height:auto; overflow-y:visible; scrollbar-gutter:auto; align-self:stretch; border-right:0; border-bottom:1px solid var(--border); padding:.6rem; } .nav { flex-direction:row; flex-wrap:wrap; } .nav-item { width:auto; } .subnav { width:100%; flex-direction:row; flex-wrap:wrap; margin:0; padding:.4rem 0; border-left:0; } .subnav-item { width:auto; } .workspace-content { padding:1rem; height:auto; overflow:visible; } }
</style>

<style scoped>
.poll-summary { font-size:.75rem; opacity:.85; white-space:nowrap; }
.background-operations { display:flex; flex-wrap:wrap; align-items:center; gap:.4rem; }
.background-operation { display:flex; align-items:center; gap:.4rem; padding:.3rem .55rem;
  border:1px solid var(--border-strong); border-radius:1rem;
  background:var(--surface-2); color:var(--text); font-size:.75rem; }
.live-dot { width:.45rem; height:.45rem; border-radius:100%; background:#35a66b; flex-shrink:0; }
@media (max-width:780px) { .header { flex-wrap:wrap; } .background-operations { order:3; width:100%; } }
</style>
