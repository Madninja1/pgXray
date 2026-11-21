import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";

/** Mirrors crates/core/src/auth.rs::UserSummary; contains no password hashes. */
export interface UserSummary {
  username: string;
  current: boolean;
}

export type UserEditor =
  | { kind: "create" }
  | { kind: "rename"; username: string }
  | { kind: "password"; username: string };

export interface UsersState {
  users: UserSummary[];
  loading: boolean;
  mutating: boolean;
  confirmingDeletion: boolean;
  editor: UserEditor | null;
  error: string | null;
  notice: string | null;
}

export function createUsersState(): UsersState {
  return { users: [], loading: false, mutating: false, confirmingDeletion: false, editor: null, error: null, notice: null };
}

export function parseUserSummaries(raw: unknown): UserSummary[] {
  if (!Array.isArray(raw)) throw new Error(t('settingsTs.invalidUsersResponse'));
  const names = new Set<string>();
  return raw.map((item: unknown) => {
    if (!item || typeof item !== "object") throw new Error(t('settingsTs.invalidUsersResponse'));
    const user = item as Record<string, unknown>;
    if (typeof user.username !== "string" || !/^[A-Za-z0-9_-]{1,64}$/.test(user.username) ||
        typeof user.current !== "boolean" || names.has(user.username)) {
      throw new Error(t('settingsTs.invalidUsersResponse'));
    }
    names.add(user.username);
    // Whitelist only public metadata. Do not retain unexpected fields.
    return { username: user.username, current: user.current };
  });
}

export function validateNewUsername(username: string): string | null {
  return /^[A-Za-z0-9_-]{1,64}$/.test(username)
    ? null : t('settingsTs.invalidUsername');
}

export function validateNewPassword(password: string, confirm: string): string | null {
  if ([...password].length < 12 || new TextEncoder().encode(password).length > 1024) {
    return t('settingsTs.invalidPassword');
  }
  if (password !== confirm) return t('settingsTs.passwordMismatch');
  return null;
}

/**
 * Headless port of ui/js/settings.js. Keep password values in temporary
 * component/form arguments, never in UsersState. Rust Auth is authoritative.
 * The future settings.html migration will render this controller in Vue.
 */
export function createUsersController(
  shell: ShellState,
  state: UsersState,
  invoke: PgXrayInvoke | null,
) {
  let revision = 0;
  let listSequence = 0;
  const allowed = () => shell.screen === "workspace" && shell.currentUsername !== null && !shell.busy && invoke !== null;
  const fresh = (generation: number, rev: number) =>
    allowed() && isCurrentAuthGeneration(shell, generation) && revision === rev;

  function reset(): void {
    revision += 1;
    listSequence += 1;
    Object.assign(state, createUsersState());
  }

  async function loadUsers(updateError: boolean): Promise<boolean> {
    if (!allowed()) return false;
    const generation = shell.authGeneration;
    const rev = revision;
    const sequence = ++listSequence;
    state.loading = true;
    if (updateError) state.error = null;
    try {
      const users = parseUserSummaries(await invoke!("list_users"));
      if (!fresh(generation, rev) || sequence !== listSequence) return false;
      state.users = users;
      return true;
    } catch (error) {
      if (fresh(generation, rev) && sequence === listSequence && updateError) state.error = String(error);
      return false;
    } finally {
      if (fresh(generation, rev) && sequence === listSequence) state.loading = false;
    }
  }

  async function refresh(): Promise<boolean> { return loadUsers(true); }

  function openCreate(): boolean {
    if (!allowed() || state.mutating || state.confirmingDeletion) return false;
    state.editor = { kind: "create" };
    state.error = null;
    return true;
  }
  function openRename(username: string): boolean {
    if (!allowed() || state.mutating || state.confirmingDeletion || !state.users.some((user) => user.username === username)) return false;
    state.editor = { kind: "rename", username };
    state.error = null;
    return true;
  }
  function openPassword(username: string): boolean {
    if (!allowed() || state.mutating || state.confirmingDeletion || !state.users.some((user) => user.username === username)) return false;
    state.editor = { kind: "password", username };
    state.error = null;
    return true;
  }
  function cancelEditor(): void {
    if (!state.mutating && !state.confirmingDeletion) state.editor = null;
  }

  /** Return false if no complete UI-visible success occurred; do not auto-retry IPC. */
  async function mutate(
    command: string,
    args: Record<string, unknown>,
    applyLocal: () => void,
    successText: string,
  ): Promise<boolean> {
    if (!allowed() || state.mutating || state.confirmingDeletion) return false;
    const generation = shell.authGeneration;
    const rev = revision;
    state.mutating = true;
    state.error = null;
    state.notice = null;
    try {
      await invoke!(command, args);
      if (!fresh(generation, rev)) return false;
      // The backend operation succeeded. Reflect it even if list_users fails.
      applyLocal();
      state.editor = null;
      state.notice = successText;
      if (!(await loadUsers(false))) {
        if (fresh(shell.authGeneration, rev)) {
          state.notice = `${successText} ${t('settingsTs.refreshUsersFailed')}`;
        }
      }
      return true;
    } catch (error) {
      if (fresh(generation, rev)) state.error = String(error);
      return false;
    } finally {
      // A self-rename changes auth generation intentionally; do not leave busy stuck.
      if (revision === rev && allowed()) state.mutating = false;
    }
  }

  async function create(username: string, password: string, confirm: string): Promise<boolean> {
    if (state.editor?.kind !== "create") return false;
    const error = validateNewUsername(username) ?? validateNewPassword(password, confirm);
    if (error) { state.error = error; return false; }
    return mutate("create_user", { username, password }, () => {
      state.users = [...state.users, { username, current: false }];
    }, t('settingsTs.userCreated'));
  }

  async function rename(newUsername: string): Promise<boolean> {
    if (state.editor?.kind !== "rename") return false;
    const oldUsername = state.editor.username;
    const error = validateNewUsername(newUsername);
    if (error) { state.error = error; return false; }
    return mutate("rename_user", { username: oldUsername, newUsername }, () => {
      const current = shell.currentUsername === oldUsername;
      if (current) {
        // Rename changes the authenticated user identity, not their privileges.
        shell.currentUsername = newUsername;
        shell.authGeneration += 1;
      }
      state.users = state.users.map((user) => user.username === oldUsername
        ? { username: newUsername, current } : user);
    }, t('settingsTs.userRenamed'));
  }

  async function changePassword(password: string, confirm: string): Promise<boolean> {
    if (state.editor?.kind !== "password") return false;
    const username = state.editor.username;
    const error = validateNewPassword(password, confirm);
    if (error) { state.error = error; return false; }
    return mutate("change_user_password", { username, password }, () => {},
      t('settingsTs.passwordChanged', { username }));
  }

  async function deleteUser(username: string, confirm: (username: string) => boolean | Promise<boolean>): Promise<boolean> {
    if (!allowed() || state.mutating || state.confirmingDeletion) return false;
    if (username === shell.currentUsername) {
      state.error = t('settingsTs.cannotDeleteCurrent');
      return false;
    }
    if (!state.users.some((user) => user.username === username)) return false;
    const generation = shell.authGeneration;
    const rev = revision;
    state.confirmingDeletion = true;
    let approved: boolean;
    try {
      approved = await confirm(username);
    } catch (error) {
      if (fresh(generation, rev)) state.error = String(error);
      return false;
    } finally {
      if (fresh(generation, rev)) state.confirmingDeletion = false;
    }
    if (!approved || !fresh(generation, rev) || !state.users.some((user) => user.username === username)) return false;
    return mutate("delete_user", { username }, () => {
      state.users = state.users.filter((user) => user.username !== username);
    }, t('settingsTs.userDeleted', { username }));
  }

  return { reset, refresh, openCreate, openRename, openPassword, cancelEditor,
    create, rename, changePassword, deleteUser };
}
