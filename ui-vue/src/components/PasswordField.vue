<script setup lang="ts">
import { onUnmounted, ref, watch } from "vue";
import UiIcon from "./UiIcon.vue";
import { t } from "../features/i18n/locale-service.js";

const model = defineModel<string>({ required: true });
const props = withDefaults(defineProps<{
  inputId: string;
  name?: string;
  autocomplete?: string;
  disabled?: boolean;
  invalid?: boolean;
  describedBy?: string;
  minlength?: number;
  maxlength?: number;
}>(), { name: "password", autocomplete: "new-password", disabled: false, invalid: false });

const visible = ref(false);
// Closing a form or clearing credentials must also hide the former secret.
watch(model, value => { if (!value) visible.value = false; });
onUnmounted(() => { visible.value = false; });
function update(event: Event): void {
  model.value = (event.target as HTMLInputElement).value;
}
</script>

<template>
  <div class="password-input">
    <input :id="props.inputId" :name="props.name" :value="model" :type="visible ? 'text' : 'password'"
      :autocomplete="props.autocomplete" :disabled="props.disabled" :aria-invalid="props.invalid || undefined"
      :aria-describedby="props.describedBy" :minlength="props.minlength" :maxlength="props.maxlength"
      required @input="update" />
    <button type="button" class="password-eye" :disabled="props.disabled"
      :aria-controls="props.inputId" :aria-pressed="visible"
      :aria-label="visible ? t('auth.hidePassword') : t('auth.showPassword')"
      :title="visible ? t('auth.hidePassword') : t('auth.showPassword')" @click="visible = !visible">
      <UiIcon :name="visible ? 'eye-off' : 'eye'" :size="19" />
    </button>
  </div>
</template>

<style scoped>
.password-input { position: relative; min-width: 0; width: 100%; }
.password-input input { box-sizing: border-box; width: 100%; min-width: 0; padding-right: 3.25rem; }
.password-eye {
  position: absolute; top: 50%; right: .35rem; transform: translateY(-50%);
  display: inline-grid; place-items: center; width: 2.45rem; height: 2.45rem;
  margin: 0; padding: 0; border: 0; border-radius: .35rem;
  background: transparent; color: var(--text-secondary, var(--text));
}
.password-eye:hover:not(:disabled) { background: var(--surface-2); }
.password-eye:focus-visible { outline: 2px solid var(--focus-ring, var(--accent)); outline-offset: 1px; }
</style>
