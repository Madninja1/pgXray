import { t } from "../i18n/locale-service.js";
import type { ImpactResult, ImpactMetric } from "./investigation.js";

// Keep these five metric ids aligned with build_change_impact_result in src-tauri/src/main.rs.
const SIGNAL_METRICS = new Set([
  "exec_ms_per_call", "shared_reads_per_call", "temp_reads_per_call",
  "temp_writes_per_call", "wal_bytes_per_call",
]);

export const IMPACT_GUIDE = {
  get timeline() { return t('tsFinish.guideTimeline'); },
  get interventions() { return t('tsFinish.guideInterventions'); },
  get impact() { return t('tsFinish.guideImpact'); },
  get comparisons() { return t('tsFinish.guideComparisons'); },
  get similar() { return t('tsFinish.guideSimilar'); },
} as const;

export function impactLabel(result: ImpactResult): string {
  switch (result.assessment?.state) {
    case "improvement_observed": return t('tsFinish.impactImproved');
    case "regression_observed": return t('tsFinish.impactRegression');
    case "mixed": return t('tsFinish.impactMixed');
    case "no_clear_signal": return t('tsFinish.impactNoSignal');
    default: return result.assessment?.label ?? t('tsFinish.impactAssessment');
  }
}

export function impactExplanation(result: ImpactResult): string {
  const pgss = result.comparison?.pgss;
  if (!pgss) return t('tsFinish.impactMissingPgss');
  const before = pgss.baseline_quality;
  const after = pgss.incident_quality;
  if (before.snapshot_count < 2 || after.snapshot_count < 2)
    return t('tsFinish.impactFewSnapshots');
  if (before.safe_interval_count === 0 || after.safe_interval_count === 0)
    return t('tsFinish.impactNoIntervals');
  const good = result.assessment?.improvement_signals ?? 0;
  const bad = result.assessment?.regression_signals ?? 0;
  if (good === 0 && bad === 0)
    return t('tsFinish.impactNoConfirmed');
  if (good > 0 && bad > 0)
    return t('tsFinish.impactMixedSignals');
  return t('tsFinish.impactNotCausal');
}

export interface ImpactSignal {
  queryid: string; label: string; unit: string; before: number; after: number;
  direction: "up" | "down";
}
export function impactSignals(result: ImpactResult): ImpactSignal[] {
  const signals: ImpactSignal[] = [];
  for (const statement of result.comparison?.pgss?.statement_changes ?? []) {
    for (const metric of statement.metrics) {
      if (!isSignalMetric(metric)) continue;
      signals.push({
        queryid: statement.queryid, label: metric.label, unit: metric.unit,
        before: metric.baseline_value!, after: metric.incident_value!,
        direction: metric.direction as "up" | "down",
      });
    }
  }
  return signals.slice(0, 12);
}
function isSignalMetric(metric: ImpactMetric): boolean {
  return SIGNAL_METRICS.has(metric.id)
    && metric.evidence_level === "confirmed_change"
    && (metric.direction === "up" || metric.direction === "down")
    && typeof metric.baseline_value === "number" && Number.isFinite(metric.baseline_value)
    && typeof metric.incident_value === "number" && Number.isFinite(metric.incident_value);
}

export function formatMetric(value: number): string {
  return new Intl.NumberFormat("ru-RU", { maximumFractionDigits: 3 }).format(value);
}
