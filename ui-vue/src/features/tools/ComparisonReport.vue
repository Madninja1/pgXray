<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import { computed } from "vue";
import { isPlanComparison, isQueryResultComparison } from "./case-comparisons.js";

const props = defineProps<{ report: unknown }>();
const plan = computed(() => isPlanComparison(props.report) ? props.report : null);
const result = computed(() => isQueryResultComparison(props.report) ? props.report : null);
function format(value: number | null | undefined): string {
  return value === null || value === undefined ? "—" : value.toLocaleString("ru-RU", { maximumFractionDigits: 2 });
}
const other = computed(() => JSON.stringify(props.report, null, 2)?.slice(0, 12_000) ?? t('compareReport.unsaved'));
</script>

<template>
  <div class="comparison-report">
    <template v-if="plan">
      <p v-for="(line, index) in plan.summary" :key="index">{{ line }}</p>
      <div class="comparison-report-scroll">
        <table>
          <thead><tr><th>{{ t('compareReport.metric') }}</th><th>Baseline</th><th>Candidate</th><th>Δ</th></tr></thead>
          <tbody>
            <tr v-for="item in plan.metrics" :key="item.key">
              <td>{{ item.label }} <small>{{ item.unit }}</small></td>
              <td>{{ format(item.baseline) }}</td><td>{{ format(item.candidate) }}</td>
              <td>{{ item.delta_pct !== null ? `${format(item.delta_pct)}%` : format(item.delta) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <p v-if="plan.structure_changed" class="comparison-warning">{{ t('compareReport.structureChanged') }}</p>
      <div v-if="plan.relation_strategies?.some(item => item.changed)">
        <h5>{{ t('compareReport.accessChanges') }}</h5>
        <p v-for="item in plan.relation_strategies.filter(item => item.changed)" :key="item.relation">
          {{ item.relation }}: {{ item.baseline_strategies.join(', ') || '—' }} → {{ item.candidate_strategies.join(', ') || '—' }}
        </p>
      </div>
    </template>
    <template v-else-if="result">
      <p :class="result.equivalent ? 'comparison-equivalent' : 'comparison-warning'">
        {{ result.equivalent ? t('compareReport.equal') : t('compareReport.different') }}
      </p>
      <p>{{ t('compareReport.rowCount', { baseline: format(result.baseline_rows), candidate: format(result.candidate_rows) }) }}</p>
      <p>{{ t('compareReport.onlyRows', { baseline: format(result.baseline_only_rows), candidate: format(result.candidate_only_rows) }) }}</p>
      <template v-if="!result.equivalent">
        <h5>{{ t('compareReport.onlyBaselineSample') }}</h5><pre>{{ JSON.stringify(result.baseline_only_sample, null, 2) }}</pre>
        <h5>{{ t('compareReport.onlyCandidateSample') }}</h5><pre>{{ JSON.stringify(result.candidate_only_sample, null, 2) }}</pre>
      </template>
      <p v-for="(warning, index) in result.warnings" :key="index" class="comparison-note">{{ warning }}</p>
    </template>
    <pre v-else>{{ other }}</pre>
  </div>
</template>

<style scoped>
.comparison-report { margin-top:.8rem; border:1px solid var(--border); border-radius:.5rem; padding:.85rem; min-width:0; }
.comparison-report h5 { margin:.8rem 0 .35rem; }
.comparison-report p { margin:.4rem 0; overflow-wrap:anywhere; }
.comparison-report-scroll { overflow-x:auto; }
.comparison-report table { border-collapse:collapse; width:100%; font-size:.85rem; }
.comparison-report :is(th, td) { border-bottom:1px solid var(--border); padding:.45rem; text-align:left; }
.comparison-report pre { max-height:280px; overflow:auto; white-space:pre-wrap; overflow-wrap:anywhere; background:var(--surface-2); padding:.65rem; }
.comparison-warning { color:var(--error); font-weight:600; }
.comparison-equivalent { font-weight:600; }
.comparison-note { color:var(--text-secondary); font-size:.85rem; }
</style>
