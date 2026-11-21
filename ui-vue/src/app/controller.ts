import { t } from "../features/i18n/locale-service.js";
import type { PgXrayInvoke } from "../api/invoke.js";
import {
  enterShellWorkspace,
  isCurrentAuthGeneration,
  leaveShellWorkspace,
  type MainView,
  type ShellState,
} from "../core/shell-state.js";

/** Matches the serde::Serialize payloads in crates/core/src/auth.rs. */
export interface AuthSession {
  username: string;
}
export interface AuthState {
  needs_registration: boolean;
  session: AuthSession | null;
}

function parseSession(value: unknown): AuthSession {
  if (
    typeof value !== "object" || value === null ||
    typeof (value as Record<string, unknown>).username !== "string" ||
    (value as Record<string, unknown>).username === ""
  ) {
    throw new Error(t('tsFinish.appSessionUnavailable'));
  }
  return { username: (value as AuthSession).username };
}

function parseAuthState(value: unknown): AuthState {
  if (typeof value !== "object" || value === null) {
    throw new Error(t('tsFinish.appAuthUnavailable'));
  }
  const candidate = value as Record<string, unknown>;
  if (typeof candidate.needs_registration !== "boolean") {
    throw new Error(t('tsFinish.appRegistration'));
  }
  return {
    needs_registration: candidate.needs_registration,
    session: candidate.session == null ? null : parseSession(candidate.session),
  };
}

/**
 * Headless port of app.js' initial auth flow. Safe to test without Vue/Tauri.
 * Feature initializers, refreshers and cleanup are NOT wired here yet.
 * Do not switch the production frontend to this migration preview.
 */
export function createAppController(state: ShellState, invoke: PgXrayInvoke | null) {
  async function run(operation: () => Promise<void>): Promise<boolean> {
    if (state.busy) return false;
    state.busy = true;
    // Only errors are user-visible; progress is represented by state.busy.
    state.message = { text: "", error: false };
    try {
      if (!invoke) throw new Error(t('tsFinish.appTauriRequired'));
      await operation();
      return true;
    } catch (error) {
      state.message = { text: String(error), error: true };
      return false;
    } finally {
      state.busy = false;
    }
  }

  async function initialize(): Promise<boolean> {
    const generation = state.authGeneration;
    return run(async () => {
      const result = parseAuthState(await invoke!("auth_state"));
      if (!isCurrentAuthGeneration(state, generation)) return;
      if (result.session) {
        enterShellWorkspace(state, result.session.username);
      } else if (result.needs_registration) {
        state.screen = "registration";
      } else {
        state.screen = "login";
      }
    });
  }

  async function registerInitialUser(username: string, password: string, confirm: string): Promise<boolean> {
    return run(async () => {
      if (password !== confirm) throw new Error(t('tsFinish.appPasswordsMismatch'));
      const session = parseSession(await invoke!("register_initial_user", { username, password }));
      enterShellWorkspace(state, session.username);
    });
  }

  async function login(username: string, password: string): Promise<boolean> {
    return run(async () => {
      const session = parseSession(await invoke!("login", { username, password }));
      enterShellWorkspace(state, session.username);
    });
  }

  async function logout(): Promise<boolean> {
    return run(async () => {
      await invoke!("logout");
      // No feature stores are mounted yet. When they are, cleanup of all their
      // subscriptions/resources must be added BEFORE this preview can replace UI.
      leaveShellWorkspace(state);
    });
  }

  function selectMainView(view: MainView): void {
    if (state.screen === "workspace" && !state.busy) state.mainView = view;
  }

  return { initialize, registerInitialUser, login, logout, selectMainView };
}
