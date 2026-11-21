<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed } from "vue";
import type { PgssStatementPoint } from "./controller.js";
const props = defineProps<{ points: PgssStatementPoint[] }>();
const bars = computed(() => {
  // Do not connect incomparable cumulative samples or invent rates for gaps.
  const points = props.points.slice(-36);
  const largest = Math.max(1, ...points.map(p => p.delta_state === "comparable" ? p.calls_per_hour ?? 0 : 0));
  return points.map((point, index) => ({
    key: `${point.snapshot_id}:${index}`, x: 14 + index * (540 / Math.max(1, points.length)),
    width: Math.max(3, Math.min(12, 540 / Math.max(1, points.length) - 3)),
    height: point.delta_state === "comparable" && point.calls_per_hour != null
      ? Math.max(2, Math.round(105 * point.calls_per_hour / largest)) : 0,
    valid: point.delta_state === "comparable" && point.calls_per_hour != null,
    title: t("pgssTrend.calls", { snapshot: String(point.snapshot_id), state: point.delta_state, rate: String(point.calls_per_hour ?? "—") }),
  }));
});
</script>
<template>
  <div class="trend" role="img" :aria-label="t('pgssTrend.aria')">
    <svg viewBox="0 0 570 135" width="100%" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
      <line x1="10" y1="117" x2="562" y2="117" class="axis" />
      <template v-for="bar in bars" :key="bar.key">
        <rect v-if="bar.valid" :x="bar.x" :y="117 - bar.height" :width="bar.width" :height="bar.height" rx="2" class="bar"><title>{{ bar.title }}</title></rect>
        <circle v-else :cx="bar.x + bar.width / 2" cy="116" r="2.5" class="gap"><title>{{ bar.title }}</title></circle>
      </template>
    </svg>
    <p class="legend">{{ t("pgssTrend.legend") }}</p>
  </div>
</template>
<style scoped>
.trend { border:1px solid var(--border); padding:.65rem; border-radius:.7rem; background:var(--surface-2); }
.axis { stroke:var(--border-strong); stroke-width:2; }
.bar { fill:var(--accent); }
.gap { fill:var(--text-secondary); opacity:.65; }
.legend { margin:.25rem 0 0; font-size:.76rem; color:var(--text-secondary); }
</style>
