<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed } from "vue";

const props = defineProps<{ modelValue: string; label: string }>();
const emit = defineEmits<{ "update:modelValue": [value: string]; commit: [] }>();
const day = computed(() => props.modelValue.slice(0, 10));
const clock = computed(() => props.modelValue.slice(11, 16));
function dateChanged(event: Event): void {
  const input = event.target as HTMLInputElement;
  emit("update:modelValue", input.value ? `${input.value}T${clock.value || "00:00"}` : "");
  input.blur(); // Date-only picker closes on a selected day, without waiting for a time selection.
  emit("commit");
}
function timeChanged(event: Event): void {
  const input = event.target as HTMLInputElement;
  emit("update:modelValue", day.value && input.value ? `${day.value}T${input.value}` : "");
  input.blur();
  emit("commit");
}
</script>
<template>
<div class="dt-field"><span class="dt-title">{{ label }}</span><div class="dt-controls">
  <label><span>{{ t('dateTime.date') }}</span><input :aria-label="`${label}: ${t('dateTime.dateAria')}`" type="date" :value="day" required @change="dateChanged" @keydown.esc="($event.target as HTMLInputElement).blur()" /></label>
  <label><span>{{ t('dateTime.time') }}</span><input :aria-label="`${label}: ${t('dateTime.timeAria')}`" type="time" :value="clock" step="60" required @change="timeChanged" @keydown.esc="($event.target as HTMLInputElement).blur()" /></label>
</div></div>
</template>
<style scoped>
.dt-field{display:grid;gap:.3rem;min-width:0;container-type:inline-size}.dt-title{font-weight:500}.dt-controls{display:grid;grid-template-columns:minmax(0,1.5fr) minmax(110px,1fr);gap:.65rem;min-width:0}.dt-controls label{display:grid;gap:.2rem;min-width:0}.dt-controls label span{color:var(--text-secondary);font-size:.76rem}.dt-controls input{box-sizing:border-box;min-width:0;width:100%;max-width:100%;padding:.55rem;background:var(--surface-2);border:1px solid var(--border-strong);border-radius:.45rem;color:var(--text)}
@container (max-width:360px){.dt-controls{grid-template-columns:minmax(0,1fr)}}
@media(max-width:560px){.dt-controls{grid-template-columns:minmax(0,1fr)}}
</style>
