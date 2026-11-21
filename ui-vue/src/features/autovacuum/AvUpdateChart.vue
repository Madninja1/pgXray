<script setup lang="ts">
import { locale, t } from "../i18n/locale-service.js";
import { computed } from "vue";
import { avUpdatePlot } from "./update.js";
import type { AvWorkloadWindow } from "./workload.js";

const props = defineProps<{ windows: AvWorkloadWindow[] }>();
const plot = computed(() => avUpdatePlot(props.windows));
function int(value: number): string { return value.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: 1 }); }
function stamp(value: number): string { return new Date(value).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US"); }
function shortStamp(value: number): string { return new Date(value).toLocaleDateString(locale.value === "ru" ? "ru-RU" : "en-US", { day: "2-digit", month: "2-digit" }); }
</script>

<template>
  <section class="update-trend" :aria-label="t('avUpdateTrend.aria')">
    <h4>{{ t("avUpdateTrend.title") }}</h4>
    <p class="hint">{{ t("avUpdateTrend.hint") }}</p>
    <p v-if="plot.bars.length === 0" class="hint">{{ t("avUpdateTrend.empty") }}</p>
    <template v-else>
      <div class="legend"><span class="updates">{{ t("avUpdateTrend.all") }}</span><span class="hot">{{ t("avUpdateTrend.hot") }}</span></div>
      <div class="chart-scroll">
        <svg :viewBox="`0 0 ${plot.width} ${plot.height}`" role="img" :aria-label="t('avUpdateTrend.chartAria', { count: String(plot.bars.length) })" preserveAspectRatio="xMidYMid meet">
          <line :x1="plot.left" :x2="plot.width - 18" :y1="plot.baseline" :y2="plot.baseline" stroke="currentColor" opacity=".35" />
          <text :x="plot.left - 9" :y="25" text-anchor="end" font-size="12" fill="currentColor">{{ int(plot.maximum) }}</text>
          <text :x="plot.left - 9" :y="plot.baseline + 4" text-anchor="end" font-size="12" fill="currentColor">0</text>
          <g v-for="bar in plot.bars" :key="`${bar.olderAt}-${bar.newerAt}`">
            <rect :x="bar.xUpdate" :y="bar.yUpdate" :width="plot.barWidth" :height="bar.hUpdate" class="update-bar">
              <title>{{ t("avUpdateTrend.updateTip", { from: stamp(bar.olderAt), to: stamp(bar.newerAt), rate: int(bar.updatesPerHour), delta: int(bar.updates) }) }}</title>
            </rect>
            <rect :x="bar.xHot" :y="bar.yHot" :width="plot.barWidth" :height="bar.hHot" class="hot-bar">
              <title>{{ t("avUpdateTrend.hotTip", { from: stamp(bar.olderAt), to: stamp(bar.newerAt), rate: int(bar.hotPerHour), delta: int(bar.hot) }) }}</title>
            </rect>
            <text :x="(bar.xUpdate + bar.xHot + plot.barWidth) / 2" :y="plot.baseline + 18" text-anchor="middle" font-size="11" fill="currentColor">{{ shortStamp(bar.newerAt) }}</text>
            <text :x="(bar.xUpdate + bar.xHot + plot.barWidth) / 2" :y="plot.baseline + 34" text-anchor="middle" font-size="10" fill="currentColor">{{ t("avUpdateTrend.hours", { value: int(bar.elapsedMinutes / 60) }) }}</text>
          </g>
        </svg>
      </div>
      <p class="hint">{{ t("avUpdateTrend.axes") }}</p>
      <p v-if="plot.bars.some(bar => !bar.resetVerified)" class="hint warning">{{ t("avUpdateTrend.caveat") }}</p>
    </template>
  </section>
</template>

<style scoped>
.update-trend{min-width:0;margin:1rem 0;padding:.9rem;border:1px solid var(--border);border-radius:.65rem}
.update-trend h4{margin:0 0 .4rem;font-size:1.05rem}.hint{color:var(--text-secondary);font-size:.84rem;line-height:1.5}
.legend{display:flex;gap:1.2rem;font-size:.85rem;flex-wrap:wrap}.legend span:before{content:"";display:inline-block;width:.85rem;height:.7rem;margin-right:.4rem;vertical-align:middle;background:currentColor;border-radius:.15rem}.legend .updates{color:#e7a861}.legend .hot{color:#66bcb6}
.chart-scroll{overflow-x:auto}.chart-scroll svg{width:100%;min-width:540px;height:auto;display:block}.update-bar{fill:#e7a861}.hot-bar{fill:#66bcb6}
.warning{color:var(--message)}
</style>
