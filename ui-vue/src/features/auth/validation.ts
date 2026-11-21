/**
 * Client-side feedback for the forms in legacy ui/views/auth.html.
 * Rust Auth remains authoritative for both validation and authentication.
 */
export type AuthFormMode = "registration" | "login";
export type AuthField = "username" | "password" | "confirm";
export type AuthFieldErrors = Partial<Record<AuthField, string>>;

export type AuthValidationMessageKey =
  | "authValidation.registrationUsername"
  | "authValidation.registrationPassword"
  | "authValidation.confirmRequired"
  | "authValidation.confirmMismatch"
  | "authValidation.loginUsername"
  | "authValidation.loginPassword";

// Preserve existing validator callers and their Russian error text.
// The Vue form supplies a translator for the selected locale.
const defaultMessages: Record<AuthValidationMessageKey, string> = {
  "authValidation.registrationUsername": "Логин: 1–64 латинских букв, цифр, символов _ или -.",
  "authValidation.registrationPassword": "Пароль: минимум 12 символов, максимум 1024 байта UTF-8.",
  "authValidation.confirmRequired": "Повторите пароль.",
  "authValidation.confirmMismatch": "Пароли не совпадают.",
  "authValidation.loginUsername": "Укажите логин (до 64 байт).",
  "authValidation.loginPassword": "Укажите пароль (до 1024 байт UTF-8).",
};

const usernamePattern = /^[A-Za-z0-9_-]{1,64}$/;
const utf8Length = (value: string): number => new TextEncoder().encode(value).length;

export function validateAuthForm(
  mode: AuthFormMode,
  username: string,
  password: string,
  confirm = "",
  translate: (key: AuthValidationMessageKey) => string = key => defaultMessages[key],
): AuthFieldErrors {
  const errors: AuthFieldErrors = {};

  if (mode === "registration") {
    if (!usernamePattern.test(username)) {
      errors.username = translate("authValidation.registrationUsername");
    }
    // Rust uses chars().count() (Unicode scalars) and password.len() (UTF-8 bytes).
    if ([...password].length < 12 || utf8Length(password) > 1024) {
      errors.password = translate("authValidation.registrationPassword");
    }
    if (!confirm) {
      errors.confirm = translate("authValidation.confirmRequired");
    } else if (password !== confirm) {
      errors.confirm = translate("authValidation.confirmMismatch");
    }
  } else {
    // Existing credentials should not be subjected to new registration rules.
    if (!username || utf8Length(username) > 64) {
      errors.username = translate("authValidation.loginUsername");
    }
    if (!password || utf8Length(password) > 1024) {
      errors.password = translate("authValidation.loginPassword");
    }
  }

  return errors;
}

export function hasAuthFormErrors(errors: AuthFieldErrors): boolean {
  return Object.values(errors).some((value) => Boolean(value));
}
