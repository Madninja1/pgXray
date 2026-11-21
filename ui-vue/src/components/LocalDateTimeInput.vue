<script setup lang="ts">
import { t } from "../features/i18n/locale-service.js";
import { computed } from "vue";
const props = defineProps<{ modelValue: string; label: string; required?: boolean }>();
const emit = defineEmits<{ (event: "update:modelValue", value: string): void }>();
const date = computed(() => props.modelValue.split("T")[0] ?? "");
const time = computed(() => props.modelValue.split("T")[1]?.slice(0, 5) ?? "");
function setDate(value: string): void {
  emit("update:modelValue", value ? `${value}T${time.value || "00:00"}` : "");
}
function setTime(value: string): void {
  if (date.value) emit("update:modelValue", `${date.value}T${value}`);
}
</script>
<template>
  <fieldset class="local-datetime">
    <legend>{{ label }}</legend>
    <label>{{ t("localDateTime.date") }}<input type="date" :value="date" :required="required" @input="setDate(($event.target as HTMLInputElement).value)" /></label>
    <label>{{ t("localDateTime.time") }}<input type="time" :value="time" :required="required" step="60" @input="setTime(($event.target as HTMLInputElement).value)" /></label>
  </fieldset>
</template>
<style scoped>
.local-datetime { min-width:0; display:grid; grid-template-columns:minmax(0, 1.3fr) minmax(100px,1fr); gap:.6rem; border:0; margin:0; padding:0; }
.local-datetime legend { font-weight:500; margin-bottom:.35rem; }
.local-datetime label { display:grid; gap:.3rem; min-width:0; font-size:.85rem; }
.local-datetime input { min-width:0; width:100%; box-sizing:border-box; color:var(--text); background:var(--surface); border:1px solid var(--border-strong); border-radius:.4rem; padding:.55rem; }
@media(max-width:600px) { .local-datetime { grid-template-columns:1fr; } }
</style>
