<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { Channel } from "@tauri-apps/api/core";
import { t } from "../i18n/locale-service.js";
import UiIcon from "../../components/UiIcon.vue";
import { terminalThemeFromCss } from "../theme/theme-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import xtermScript from "../../../../ui/vendor/xterm/xterm.js?url&no-inline";
import fitScript from "../../../../ui/vendor/xterm/addon-fit.js?url&no-inline";
import {
  createSessionsController, createSessionsState, sessionIsAttached, sessionLabel,
  type PsqlSessionEvent, type PsqlSessionInfo,
} from "./controller.js";

interface TerminalPort {
  cols: number; rows: number;
  options: { theme?: unknown };
  loadAddon(addon: FitAddonPort): void;
  open(host: HTMLElement): void;
  write(chunk: string): void;
  onData(handler: (data: string) => void): void;
  onResize(handler: (size: { cols: number; rows: number }) => void): void;
  attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean): void;
  hasSelection(): boolean;
  getSelection(): string;
  focus(): void;
  dispose(): void;
}
interface FitAddonPort { fit(): void }
interface TerminalGlobals {
  Terminal?: new (options: Record<string, unknown>) => TerminalPort;
  FitAddon?: { FitAddon: new () => FitAddonPort };
}
interface TerminalView { terminal: TerminalPort; addon: FitAddonPort; host: HTMLElement }
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean }>();
const emit = defineEmits<{ (event: "sessions-changed", sessions: PsqlSessionInfo[]): void }>();
const state = reactive(createSessionsState());
const host = ref<HTMLDivElement | null>(null);
const terminals = new Map<number, TerminalView>();
const pendingOutput = new Map<number, string>();
const latestSession = computed(() => state.items.find(s => s.id === state.activeId) ?? null);
const attached = computed(() => latestSession.value !== null && sessionIsAttached(state, latestSession.value.id));
const activeCount = computed(() => state.items.filter(s => s.running).length);
const initError = ref<string | null>(null);
let mounted = true;
let observer: ResizeObserver | null = null;
let resizeTimer: ReturnType<typeof setTimeout> | null = null;
let scriptsPromise: Promise<void> | null = null;
function addScript(url: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = url;
    script.async = false;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(t('sessions.terminalLoadError')));
    document.head.appendChild(script);
  });
}
async function loadTerminalLibrary(): Promise<void> {
  if (!scriptsPromise) scriptsPromise = (async () => {
    const globals = window as unknown as TerminalGlobals;
    if (!globals.Terminal) await addScript(xtermScript);
    if (!globals.FitAddon?.FitAddon) await addScript(fitScript);
    if (!globals.Terminal || !globals.FitAddon?.FitAddon) throw new Error(t('sessions.addonUnavailable'));
  })();
  return scriptsPromise;
}
function deliverOutput(id: number, text: string): void {
  const view = terminals.get(id);
  if (view) { view.terminal.write(text); return; }
  pendingOutput.set(id, ((pendingOutput.get(id) ?? "") + text).slice(-65_536));
}
const controller = createSessionsController(props.shell, state, props.invoke, {
  createChannel: () => new Channel<PsqlSessionEvent>(),
  onOutput: deliverOutput,
  onExit: () => { emit("sessions-changed", [...state.items]); },
});
function clearTerminals(): void {
  for (const terminal of terminals.values()) { terminal.terminal.dispose(); terminal.host.remove(); }
  terminals.clear(); pendingOutput.clear();
}
async function copySelected(target: TerminalPort): Promise<void> {
  if (!target.hasSelection()) return;
  try {
    if (!navigator.clipboard?.writeText) throw new Error(t('sessions.clipboardUnavailable'));
    await navigator.clipboard.writeText(target.getSelection());
  } catch (error) { state.error = t('sessions.copyFailed', { error: String(error) }); }
}
async function createTerminal(id: number): Promise<void> {
  if (terminals.has(id) || !sessionIsAttached(state, id) || !host.value) return;
  await loadTerminalLibrary();
  if (!mounted || !sessionIsAttached(state, id) || !host.value || terminals.has(id)) return;
  const globals = window as unknown as Required<TerminalGlobals>;
  const slot = document.createElement("div");
  slot.className = "sessions-terminal-slot";
  host.value.appendChild(slot);
  const terminal = new globals.Terminal({
    cursorBlink: true, cursorStyle: "block", scrollback: 20000, fontSize: 14, lineHeight: 1.2,
    convertEol: false, fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
    theme: terminalThemeFromCss(),
  });
  const addon = new globals.FitAddon.FitAddon();
  terminal.loadAddon(addon);
  terminal.open(slot);
  terminal.attachCustomKeyEventHandler(event => {
    if (event.type !== "keydown" || event.code !== "KeyC") return true;
    if ((event.ctrlKey || event.metaKey) && terminal.hasSelection()) {
      void copySelected(terminal); return false;
    }
    if (event.ctrlKey && event.shiftKey) return false;
    return true; // Ctrl+C without a selection is sent to psql.
  });
  terminal.onData(data => { if (state.activeId === id) void controller.send(id, data); });
  terminal.onResize(({ cols, rows }) => { if (state.activeId === id) void controller.resize(id, cols, rows); });
  terminals.set(id, { terminal, addon, host: slot });
  const buffered = pendingOutput.get(id);
  if (buffered) { pendingOutput.delete(id); terminal.write(buffered); }
}
async function fitActive(focus = false): Promise<void> {
  if (!props.visible) return;
  await nextTick();
  const id = state.activeId;
  if (id === null || !sessionIsAttached(state, id)) return;
  try {
    await createTerminal(id);
    if (!mounted || state.activeId !== id || !props.visible) return;
    for (const [candidate, view] of terminals) view.host.style.display = candidate === id ? "block" : "none";
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    terminals.get(id)?.addon.fit();
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    terminals.get(id)?.addon.fit();
    if (focus) terminals.get(id)?.terminal.focus();
  } catch (error) { if (mounted) initError.value = String(error); }
}
async function refresh(): Promise<void> {
  if (await controller.refresh()) {
    emit("sessions-changed", [...state.items]);
    await fitActive();
  }
}
async function openConnection(connectionId: number, forceNew = false): Promise<void> {
  await nextTick();
  try {
    await loadTerminalLibrary();
    // Start with stable geometry; FitAddon sends an accurate resize after the PTY opens.
    const session = await controller.start(connectionId, null, forceNew, 120, 30);
    emit("sessions-changed", [...state.items]);
    if (sessionIsAttached(state, session.id)) await fitActive(true);
  } catch (error) { state.error = String(error); throw error; }
}
function selectSession(id: number): void { controller.select(id); void fitActive(true); }
async function closeCurrent(): Promise<void> {
  const session = latestSession.value;
  if (!session) return;
  if (session.running && !await confirmAction(t('sessions.closeConfirm', { id: String(session.id), name: session.connection_name }))) return;
  if (await controller.close(session.id)) {
    const view = terminals.get(session.id);
    view?.terminal.dispose(); view?.host.remove(); terminals.delete(session.id);
    pendingOutput.delete(session.id);
    emit("sessions-changed", [...state.items]);
    await fitActive();
  }
}
function onThemeChange(): void {
  for (const view of terminals.values()) view.terminal.options.theme = terminalThemeFromCss();
}
function onResize(): void {
  if (resizeTimer !== null) clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => { resizeTimer = null; void fitActive(); }, 120);
}
watch(() => state.activeId, () => { void fitActive(true); });
watch(host, element => {
  observer?.disconnect();
  if (element) observer?.observe(element);
});
watch(() => props.visible, show => { if (show) void fitActive(); });
watch(() => props.shell.authGeneration, () => {
  controller.reset(); clearTerminals(); initError.value = null;
  if (props.shell.screen === "workspace") void refresh();
});
onMounted(() => {
  mounted = true;
  observer = new ResizeObserver(onResize);
  if (host.value) observer.observe(host.value);
  window.addEventListener("pgxray-theme-changed", onThemeChange);
  void loadTerminalLibrary().catch(err => { initError.value = String(err); });
  void refresh();
});
onUnmounted(() => {
  mounted = false;
  observer?.disconnect(); observer = null;
  if (resizeTimer !== null) clearTimeout(resizeTimer);
  window.removeEventListener("pgxray-theme-changed", onThemeChange);
  controller.reset(); clearTerminals();
});
defineExpose({ openConnection });
</script>

<template>
  <section class="sessions-view" :aria-label="t('sessions.region')">
    <header class="sessions-heading">
      <div>
        <h1>{{ t('sessions.title') }}</h1>
        <p class="sessions-muted">{{ t('sessions.subtitle') }}</p>
      </div>
      <button type="button" class="secondary sessions-icon-button" :title="t('sessions.refresh')" :aria-label="t('sessions.refreshAria')"
        :disabled="state.loading || state.starting" @click="refresh">
        <UiIcon name="refresh" :size="18" :class="{ 'sessions-spin': state.loading }" />
      </button>
    </header>
    <p class="sessions-warning">{{ t('sessions.securityLead') }} <strong>{{ t('sessions.rolePrivileges') }}</strong>. {{ t('sessions.securityWarning') }}</p>
    <p v-if="initError" class="sessions-error" role="alert">{{ initError }}</p>
    <p v-if="state.error" class="sessions-error" role="alert">{{ state.error }}</p>
    <div class="sessions-tabs" role="group" :aria-label="t('sessions.listAria')">
      <button v-for="session in state.items" :key="session.id" type="button" class="sessions-tab"
        :class="{ selected: state.activeId === session.id }" :aria-pressed="state.activeId === session.id"
        @click="selectSession(session.id)">
        <UiIcon name="terminal" :size="15" /> #{{ session.id }} · {{ session.connection_name }}
        <span v-if="!session.running" class="sessions-muted">{{ t('sessions.closedTab') }}</span>
        <span v-else-if="!sessionIsAttached(state, session.id)" class="sessions-unattached">{{ t('sessions.noOutput') }}</span>
      </button>
    </div>
    <div v-if="state.items.length === 0" class="sessions-empty">
      <UiIcon name="terminal" :size="30" />
      <strong>{{ t('sessions.emptyTitle') }}</strong>
      <span>{{ t('sessions.emptyHelp') }}</span>
    </div>
    <template v-else-if="latestSession">
      <div class="sessions-toolbar">
        <div class="sessions-toolbar-info">
          <strong>{{ sessionLabel(latestSession) }}</strong>
          <span class="sessions-muted">#{{ latestSession.id }} · {{ t('sessions.roleInfo') }}</span>
        </div>
        <button type="button" class="sessions-close" :disabled="state.closingId !== null" @click="closeCurrent">
          <UiIcon name="x" :size="16" /> {{ state.closingId !== null ? t('sessions.closing') : (latestSession.running ? t('sessions.disconnect') : t('sessions.removeTab')) }}
        </button>
      </div>
      <p v-if="!latestSession.running" class="sessions-warning">
        {{ t('sessions.terminatedNotice') }}
      </p>
      <div v-else-if="!attached" class="sessions-warning">
        {{ t('sessions.unattached') }}
      </div>
      <details v-if="!latestSession.running && state.exitOutput[latestSession.id]" class="sessions-info">
        <summary>{{ t('sessions.exitOutput') }}</summary>
        <pre class="sessions-startup-output">{{ state.exitOutput[latestSession.id] }}</pre>
      </details>
      <div ref="host" class="sessions-terminal-host" :class="{ 'sessions-terminal-hidden': !attached }"
        role="region" :aria-label="t('sessions.terminalAria')"></div>
      <details class="sessions-info">
        <summary>{{ t('sessions.technicalData') }}</summary>
        <p>Workspace: {{ latestSession.workspace_path }}</p>
        <p>.psqlrc: {{ latestSession.psqlrc_path }}</p>
        <p>Editor: {{ latestSession.editor }}</p>
        <p>{{ t('sessions.psqlrcWarning') }}</p>
      </details>
    </template>
  </section>
</template>

<style scoped>
.sessions-view { display:grid; gap:1rem; min-width:0; }
.sessions-heading { display:flex; justify-content:space-between; align-items:center; gap:1rem; }
.sessions-heading h1 { margin:0 0 .3rem; }
.sessions-muted { color:var(--text-secondary); font-size:.85rem; overflow-wrap:anywhere; }
.sessions-warning { padding:.75rem 1rem; border:1px solid var(--border-strong); border-radius:.55rem; color:var(--text-secondary); background:var(--surface); margin:0; }
.sessions-error { color:var(--error); border-left:3px solid var(--error); padding:.6rem .85rem; margin:0; }
.sessions-icon-button { display:inline-flex; align-items:center; justify-content:center; width:42px; height:42px; padding:.5rem; }
.sessions-spin { animation:sessions-spin .7s linear infinite; }
@keyframes sessions-spin { to { transform:rotate(360deg); } }
.sessions-tabs { display:flex; gap:.45rem; flex-wrap:wrap; }
.sessions-tab { display:inline-flex; align-items:center; gap:.35rem; background:var(--surface); color:var(--text); border:1px solid var(--border); }
.sessions-tab.selected { border-color:var(--accent); background:var(--surface-2); }
.sessions-unattached { font-size:.7rem; color:var(--error); }
.sessions-empty { display:flex; flex-direction:column; align-items:center; gap:.6rem; padding:4rem 1rem; border:1px dashed var(--border); border-radius:.65rem; color:var(--text-secondary); }
.sessions-toolbar { display:flex; justify-content:space-between; flex-wrap:wrap; gap:1rem; align-items:center; background:var(--surface); border:1px solid var(--border); border-radius:.6rem; padding:.8rem 1rem; }
.sessions-toolbar-info { display:grid; gap:.3rem; }
.sessions-close { display:inline-flex; align-items:center; gap:.4rem; background:var(--button-secondary); color:var(--error); }
.sessions-terminal-host { min-height:430px; height:min(62vh,700px); padding:.6rem; overflow:hidden; border:1px solid var(--border-strong); border-radius:.6rem; background:var(--surface); }
.sessions-terminal-hidden { display:none; }
.sessions-terminal-host :deep(.sessions-terminal-slot) { width:100%; height:100%; }
.sessions-info { font-size:.8rem; overflow-wrap:anywhere; }
.sessions-info summary { cursor:pointer; }
.sessions-info p { margin:.45rem 0; color:var(--text-secondary); }
@media (prefers-reduced-motion: reduce) { .sessions-spin { animation:none; } }
.sessions-startup-output { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 18rem; overflow: auto; font: inherit; }
</style>
