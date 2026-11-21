/**
 * Typed state boundaries inferred from ui/js/core.js and ui/js/app.js.
 * These are data definitions, not a second running UI or an auth authority.
 */
export const MAIN_VIEWS = [
  "connections", "diagnostics", "activity", "history",
  "changes", "tools", "sessions", "settings",
] as const;
export type MainView = (typeof MAIN_VIEWS)[number];

export type Screen = "registration" | "login" | "workspace";
export type DiagnosticModule = "xid" | "pgss" | "autovacuum" | "io-wal" | "index-health";
export type HistoryModule = "xid" | "pgss" | "autovacuum";

export interface UiMessage {
  text: string;
  error: boolean;
}

export interface ShellState {
  screen: Screen | null;
  currentUsername: string | null;
  mainView: MainView;
  diagnosticModule: DiagnosticModule;
  historyModule: HistoryModule;
  busy: boolean;
  message: UiMessage;
  /** Incremented at auth-context changes to identify stale async UI responses. */
  authGeneration: number;
}

export function createShellState(): ShellState {
  return {
    screen: null,
    currentUsername: null,
    mainView: "connections",
    diagnosticModule: "xid",
    historyModule: "xid",
    busy: false,
    message: { text: "", error: false },
    authGeneration: 0,
  };
}

/** Pure state transition; Vue can wrap this state in reactive() later. */
export function enterShellWorkspace(state: ShellState, username: string): void {
  state.authGeneration += 1;
  state.currentUsername = username;
  state.screen = "workspace";
  state.mainView = "connections";
}

/** Only resets shell state. Feature stores and resources must reset separately. */
export function leaveShellWorkspace(state: ShellState): void {
  state.authGeneration += 1;
  state.currentUsername = null;
  state.screen = "login";
  state.mainView = "connections";
  state.diagnosticModule = "xid";
  state.historyModule = "xid";
}

/** A response may update the UI only if it belongs to the current auth epoch. */
export function isCurrentAuthGeneration(state: ShellState, generation: number): boolean {
  return state.authGeneration === generation;
}
