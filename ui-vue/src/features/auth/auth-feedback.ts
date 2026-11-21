import type { UiMessage } from "../../core/shell-state.js";
import type { AuthFormMode } from "./validation.js";

/** Route authentication errors into the active form, never above it. */
export function authInlineError(mode: AuthFormMode | null, message: UiMessage): string | null {
  return mode !== null && message.error && message.text ? message.text : null;
}
