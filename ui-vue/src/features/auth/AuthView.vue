<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { hasAuthFormErrors, validateAuthForm, type AuthFieldErrors, type AuthFormMode } from "./validation.js";
import PasswordField from "../../components/PasswordField.vue";
import { t } from "../i18n/locale-service.js";

const props = defineProps<{
  mode: AuthFormMode;
  busy: boolean;
  available: boolean;
  authError: string | null;
  clearAuthError: () => void;
  login: (username: string, password: string) => Promise<boolean>;
  registerInitialUser: (username: string, password: string, confirm: string) => Promise<boolean>;
}>();

const username = ref("");
const password = ref("");
const confirm = ref("");
const errors = ref<AuthFieldErrors>({});
const submitting = ref(false);
const working = computed(() => submitting.value || props.busy);
const passwordError = computed(() => errors.value.password || (props.mode === "login" ? props.authError : null));

function onUsernameInput(): void {
  errors.value.username = undefined;
  props.clearAuthError();
}

function onPasswordInput(): void {
  errors.value.password = undefined;
  props.clearAuthError();
}

function onConfirmInput(): void {
  errors.value.confirm = undefined;
  props.clearAuthError();
}

function clearSecrets(): void {
  password.value = "";
  confirm.value = "";
}

function resetForm(): void {
  username.value = "";
  clearSecrets();
  errors.value = {};
}

watch(() => props.mode, resetForm);
onUnmounted(resetForm);

async function submit(): Promise<void> {
  if (working.value || !props.available) return;
  const mode = props.mode;
  errors.value = validateAuthForm(mode, username.value, password.value, confirm.value, key => t(key));
  if (hasAuthFormErrors(errors.value)) {
    // Preserve input so the user can compare and correct the passwords.
    // Secrets are still cleared after an IPC attempt or when leaving the form.
    return;
  }

  submitting.value = true;
  try {
    const ok = mode === "registration"
      ? await props.registerInitialUser(username.value, password.value, confirm.value)
      : await props.login(username.value, password.value);
    if (ok) username.value = "";
  } catch {
    // The controller handles expected IPC errors. Never include credentials
    // in an unexpected error message rendered or logged by the view.
    errors.value = { password: t("auth.unexpectedError") };
  } finally {
    clearSecrets();
    submitting.value = false;
  }
}
</script>

<template>
  <section class="panel auth-panel" :aria-labelledby="mode === 'registration' ? 'auth-register-heading' : 'auth-login-heading'">
    <template v-if="mode === 'registration'">
      <h1 id="auth-register-heading">{{ t("auth.registrationTitle") }}</h1>
      <p>{{ t("auth.registrationDescription") }}</p>
    </template>
    <template v-else>
      <h1 id="auth-login-heading">{{ t("auth.loginTitle") }}</h1>
      <p>{{ t("auth.loginDescription") }}</p>
    </template>

    <form class="auth-form" novalidate :aria-busy="working" @submit.prevent="submit">
      <label for="auth-username">{{ mode === 'registration' ? t('auth.username') : t('auth.appUsername') }}</label>
      <input id="auth-username" v-model="username" name="username" autocomplete="username"
        maxlength="64" :pattern="mode === 'registration' ? '[A-Za-z0-9_-]{1,64}' : undefined"
        required :disabled="working" :aria-invalid="Boolean(errors.username)"
        :aria-describedby="errors.username ? 'auth-username-error' : undefined"
        @input="onUsernameInput" />
      <small v-if="errors.username" id="auth-username-error" class="field-error" role="alert">{{ errors.username }}</small>

      <label for="auth-password">{{ mode === 'registration' ? t('auth.password') : t('auth.appPassword') }}</label>
      <PasswordField v-model="password" input-id="auth-password" name="password"
        :autocomplete="mode === 'registration' ? 'new-password' : 'current-password'"
        :minlength="mode === 'registration' ? 12 : undefined" :maxlength="1024"
        :disabled="working" :invalid="Boolean(passwordError)"
        :described-by="passwordError ? 'auth-password-error' : undefined"
        @update:model-value="onPasswordInput" />
      <small v-if="passwordError" id="auth-password-error" class="field-error" role="alert">{{ passwordError }}</small>

      <template v-if="mode === 'registration'">
        <label for="auth-confirm">{{ t("auth.confirmPassword") }}</label>
        <PasswordField v-model="confirm" input-id="auth-confirm" name="confirm"
          autocomplete="new-password" :minlength="12" :maxlength="1024"
          :disabled="working" :invalid="Boolean(errors.confirm)"
          :described-by="errors.confirm ? 'auth-confirm-error' : undefined"
          @update:model-value="onConfirmInput" />
        <small v-if="errors.confirm" id="auth-confirm-error" class="field-error" role="alert">{{ errors.confirm }}</small>
        <small v-if="authError" class="field-error" role="alert">{{ authError }}</small>
      </template>

      <button type="submit" :disabled="working || !available">
        <span v-if="working" class="button-spinner" aria-hidden="true"></span>
        {{ working
          ? (mode === 'registration' ? t('auth.creating') : t('auth.loggingIn'))
          : (mode === 'registration' ? t('auth.createUser') : t('auth.login')) }}
      </button>
    </form>
  </section>
</template>

<style scoped>
.auth-form { display: grid; gap: .65rem; max-width: 28rem; }
.auth-form label { font-weight: 500; }
.auth-form input { width: 100%; box-sizing: border-box; }
.auth-form > button[type="submit"] { justify-self: start; margin-top: .5rem; }
.field-error { color: var(--error); }
</style>
