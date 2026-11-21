import { t } from "../i18n/locale-service.js";
import type { ArtifactContext } from "./artifacts.js";

export type ComparisonMode = "saved_plan" | "query_explain" | "query_analyze" | "query_result";

export interface ComparisonRequest {
  command: string;
  args: Record<string, unknown>;
}

export const COMPARISON_TIMEOUT_OPTIONS = [30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 7_200_000, 14_400_000] as const;

// Match the old Plan Workspace selectors, without accepting shareable copies of SQL/plans.
export function comparisonSources(artifacts: ArtifactContext[], caseId: number, mode: ComparisonMode): ArtifactContext[] {
  return artifacts.filter(({ artifact }) => {
    if (artifact.case_id !== caseId || artifact.visibility !== "sensitive") return false;
    const name = artifact.original_name.toLowerCase();
    return mode === "saved_plan"
      ? artifact.kind === "explain" || name.endsWith(".json")
      : artifact.kind === "sql" || name.endsWith(".sql");
  });
}

export function comparisonRequest(
  sources: ArtifactContext[], caseId: number, mode: ComparisonMode,
  baselineId: number | null, candidateId: number | null,
  timeoutMs: number, executionConfirmed: boolean,
): ComparisonRequest {
  if (!Number.isSafeInteger(caseId) || caseId <= 0) throw new Error(t('tsFinish.caseChoose'));
  if (baselineId === null || candidateId === null) throw new Error(t('tsFinish.caseChoosePair'));
  if (baselineId === candidateId) throw new Error(t('tsFinish.casePairDifferent'));
  const permitted = comparisonSources(sources, caseId, mode);
  if (!permitted.some(item => item.artifact.id === baselineId) || !permitted.some(item => item.artifact.id === candidateId)) {
    throw new Error(t('tsFinish.caseLocalArtifacts'));
  }
  if (mode === "saved_plan") {
    return { command: "compare_plan_artifacts", args: { baselineArtifactId: baselineId, candidateArtifactId: candidateId } };
  }
  if (!COMPARISON_TIMEOUT_OPTIONS.includes(timeoutMs as (typeof COMPARISON_TIMEOUT_OPTIONS)[number])) {
    throw new Error(t('tsFinish.caseTimeout'));
  }
  if ((mode === "query_analyze" || mode === "query_result") && !executionConfirmed) {
    throw new Error(t('tsFinish.caseConfirmSql'));
  }
  const ids = { caseId, baselineArtifactId: baselineId, candidateArtifactId: candidateId, timeoutMs };
  if (mode === "query_result") {
    return { command: "compare_case_query_results", args: ids };
  }
  return { command: "compare_case_query_plans", args: { ...ids, analyzeQueries: mode === "query_analyze" } };
}

export interface PlanMetric {
  key: string; label: string; unit: string;
  baseline: number | null; candidate: number | null; delta: number | null; delta_pct: number | null;
}
export interface PlanComparisonReport {
  summary: string[];
  metrics: PlanMetric[];
  node_types: { node_type: string; baseline_count: number; candidate_count: number; delta: number }[];
  relation_strategies: { relation: string; baseline_strategies: string[]; candidate_strategies: string[]; changed: boolean }[];
  structure_changed: boolean;
}
export interface QueryResultReport {
  equivalent: boolean;
  baseline_rows: number; candidate_rows: number;
  baseline_only_rows: number; candidate_only_rows: number;
  baseline_only_sample: unknown; candidate_only_sample: unknown;
  warnings: string[];
}
export function isPlanComparison(value: unknown): value is PlanComparisonReport {
  if (value === null || typeof value !== "object") return false;
  const result = value as Partial<PlanComparisonReport>;
  return Array.isArray(result.summary) && Array.isArray(result.metrics);
}
export function isQueryResultComparison(value: unknown): value is QueryResultReport {
  if (value === null || typeof value !== "object") return false;
  const result = value as Partial<QueryResultReport>;
  return typeof result.equivalent === "boolean" && typeof result.baseline_rows === "number" && typeof result.candidate_rows === "number";
}
