<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed } from "vue";
import type { AvRelationHistory } from "./controller.js";
import { avBuildTrend, avLastSampleChange, type AvTrendKind } from "./trends.js";

const props = defineProps<{ history: AvRelationHistory; kind: AvTrendKind }>();
const data = computed(() => avBuildTrend(props.history, props.kind));
const title = computed(() => props.kind === "vacuum" ? t("avTrend.titleVacuum") : props.kind === "insert"
  ? t("avTrend.titleInsert") : t("avTrend.titleAnalyze"));
const sampleChange = computed(() => avLastSampleChange(props.history, props.kind));
function number(value: number): string { return value.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 0 }); }
function date(value: number): string { return new Date(value).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US"); }
</script>

<template>
  <section class="trend" :aria-label="title">
    <h4>{{ title }}</h4>
    <p class="hint">{{ t("avTrend.hint") }}</p>
    <p v-if="data.points.length < 2" class="hint">{{ kind === 'insert' && data.points.length === 0 ? t("avTrend.disabled") : t("avTrend.few") }}</p>
    <template v-else>
      <p v-if="sampleChange" class="hint">{{ t("avTrend.sample", { minutes: number(Math.round(sampleChange.elapsedMinutes)), delta: `${sampleChange.delta > 0 ? "+" : ""}${number(sampleChange.delta)}` }) }}</p>
      <div class="legend"><span class="observed">{{ t("avTrend.estimate") }}</span><span class="threshold">{{ t("avTrend.threshold") }}</span></div>
      <svg :viewBox="`0 0 ${data.width} ${data.height}`" role="img" :aria-label="t('avTrend.aria', { title, count: String(data.points.length) })" preserveAspectRatio="xMidYMid meet">
        <line :x1="data.left" :x2="data.width - 12" :y1="data.height - data.bottom" :y2="data.height - data.bottom" stroke="currentColor" opacity="0.2"/>
        <text :x="data.left - 8" :y="data.top + 5" text-anchor="end" font-size="11" fill="currentColor">{{ number(data.ceiling) }}</text>
        <text :x="data.left - 8" :y="data.height - data.bottom" text-anchor="end" font-size="11" fill="currentColor">0</text>
        <path :d="data.thresholdLine" class="threshold-line" />
        <path :d="data.observedLine" class="observed-line" />
        <g v-for="point in data.points" :key="point.snapshotId">
          <circle :cx="point.x" :cy="point.yObserved" r="3.5" class="observed-dot"><title>{{ t("avTrend.pointEstimate", { date: date(point.measuredAt), value: number(point.observed) }) }}</title></circle>
          <circle :cx="point.x" :cy="point.yThreshold" r="2.8" class="threshold-dot"><title>{{ t("avTrend.pointThreshold", { date: date(point.measuredAt), value: number(point.threshold) }) }}</title></circle>
        </g>
      </svg>
      <div class="axis"><span>{{ date(data.earliest) }}</span><span>{{ date(data.latest) }}</span></div>
      <details><summary>{{ t("avTrend.details") }}</summary>
        <div class="table-scroll"><table><thead><tr><th>{{ t("avTrend.time") }}</th><th>{{ t("avTrend.estimate") }}</th><th>{{ t("avTrend.thresholdCol") }}</th><th>{{ t("avTrend.manual") }}</th><th>{{ t("avTrend.automatic") }}</th></tr></thead><tbody>
          <tr v-for="point in [...data.points].reverse()" :key="point.snapshotId"><td>{{ date(point.measuredAt) }}</td><td>{{ number(point.observed) }}</td><td>{{ number(point.threshold) }}</td><td>{{ number(point.manualCycles) }}</td><td>{{ number(point.automaticCycles) }}</td></tr>
        </tbody></table></div>
      </details>
      <p class="hint">{{ t("avTrend.caveat") }}</p>
    </template>
  </section>
</template>

<style scoped>
.trend{min-width:0;padding:.75rem;border:1px solid var(--border);border-radius:.6rem}.trend h4{margin:0 0 .45rem;font-size:1rem}.hint{color:var(--text-secondary);font-size:.82rem;line-height:1.45}.legend{display:flex;gap:1rem;margin:.6rem 0;font-size:.82rem}.legend span:before{content:"";display:inline-block;width:.8rem;height:.2rem;vertical-align:middle;margin-right:.4rem;background:currentColor}.legend .observed{color:var(--text)}.legend .threshold{color:#e7a861}svg{width:100%;height:auto;display:block;overflow:visible}.observed-line{fill:none;stroke:var(--text);stroke-width:2;stroke-dasharray:4 3}.threshold-line{fill:none;stroke:#e7a861;stroke-width:2;stroke-dasharray:6 4}.observed-dot{fill:var(--text)}.threshold-dot{fill:#e7a861}.axis{display:flex;justify-content:space-between;font-size:.74rem;color:var(--text-secondary);margin-top:.3rem}.table-scroll{overflow:auto;max-height:18rem}table{border-collapse:collapse;width:100%;font-size:.82rem}td,th{text-align:left;padding:.45rem;border-bottom:1px solid var(--border)}summary{cursor:pointer;margin:.6rem 0;font-weight:600}
</style>
