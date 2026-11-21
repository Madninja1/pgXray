import { t } from "../i18n/locale-service.js";
import { validateNewPassword, validateNewUsername, type UserEditor } from "./users-controller.js";

export interface UserFormErrors {
  username?: string;
  password?: string;
  confirm?: string;
}

/** Mirrors core validation before IPC without replacing the Rust authority. */
export function validateUserForm(
  editor: UserEditor,
  username: string,
  password: string,
  confirmation: string,
  existingUsernames: readonly string[] = [],
): UserFormErrors {
  const errors: UserFormErrors = {};
  if (editor.kind !== "password") {
    const error = validateNewUsername(username.trim());
    if (error) errors.username = error;
    else if (editor.kind === "create" && existingUsernames.includes(username.trim())) {
      errors.username = t('settingsTs.userExists');
    }
  }
  if (editor.kind !== "rename") {
    const error = validateNewPassword(password, password);
    if (error) errors.password = error;
    else if (!confirmation) errors.confirm = t('settingsTs.confirmPassword');
    else if (password !== confirmation) errors.confirm = t('settingsTs.passwordMismatch');
  }
  return errors;
}

export function hasUserFormErrors(errors: UserFormErrors): boolean {
  return Boolean(errors.username || errors.password || errors.confirm);
}
