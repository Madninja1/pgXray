<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import type { HistorySortOrder } from "./sort-rows.js";

defineProps<{ label: string; column: string; order?: HistorySortOrder }>();
const emit = defineEmits<{ sort: [column: string] }>();
</script>

<template>
  <th :aria-sort="order?.column === column ? (order.direction === 'asc' ? 'ascending' : 'descending') : 'none'">
    <button class="hist-sort-heading" type="button" :aria-label="t('historyAriaFinish.sort', { label })" @click="emit('sort', column)">
      <span>{{ label }}</span>
      <span class="hist-sort-arrow" aria-hidden="true">{{ order?.column === column ? (order.direction === 'asc' ? '▲' : '▼') : '↕' }}</span>
    </button>
  </th>
</template>

<style scoped>
.hist-sort-heading{display:inline-flex;align-items:center;gap:.4rem;padding:0;border:0;background:transparent;color:inherit;font:inherit;font-weight:inherit;line-height:1.35;text-align:left;cursor:pointer}
.hist-sort-heading:hover{text-decoration:underline;text-underline-offset:2px}
.hist-sort-heading:focus-visible{outline:2px solid var(--accent);outline-offset:3px;border-radius:2px}
.hist-sort-arrow{opacity:.7;font-size:.75em;white-space:nowrap}
[aria-sort="ascending"] .hist-sort-arrow,[aria-sort="descending"] .hist-sort-arrow{opacity:1;color:var(--accent)}
</style>
