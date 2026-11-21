import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import { isCurrentAuthGeneration, type ShellState } from "../../core/shell-state.js";
import type { ToolCase } from "./controller.js";

export interface CaseNavigationTarget {
  kind: string; artifact_id?: number | null; pid?: number | null;
  userid?: number | null; dbid?: number | null; queryid?: string | null;
  toplevel?: boolean | null; snapshot_id?: number | null; relation_oid?: number | null;
}
export interface TimelineItem {
  id: string; occurred_at_ms: number; source: string; kind: string; title: string;
  detail: string; evidence_level: string;
  navigation: CaseNavigationTarget | null;
}
export interface Intervention {
  id: number; occurred_at_ms: number; title: string; kind: string; note: string;
}
export interface WorkspaceChange {
  id: number; connection_id: number; case_id: number | null; intervention_id: number | null;
  kind: string; actor: string | null; relation_oid: number | null;
  snapshot_id: number | null; parameter: string; before: unknown; after: unknown;
  status: "requested" | "verified" | "failed";
  target: { host?: string; port?: number; database?: string; username?: string; sslmode?: string; database_oid?: number };
  started_at_ms: number; finished_at_ms: number | null;
  post_snapshot_id: number | null; note: string;
}
export interface ImpactPeriodQuality {
  state: string; snapshot_count: number; safe_interval_count: number; unsafe_interval_count: number;
  snapshot_ids: number[]; limitations: string[];
}
export interface ImpactMetric {
  id: string; label: string; unit: string; baseline_value: number | null; incident_value: number | null;
  direction: string; evidence_level: string;
}
export interface ImpactStatement {
  queryid: string; metrics: ImpactMetric[];
}
export interface ImpactResult {
  assessment?: {
    state?: string; label?: string; improvement_signals?: number; regression_signals?: number;
    workload_shift_observed?: boolean;
  };
  comparison?: {
    pgss?: {
      baseline_quality: ImpactPeriodQuality; incident_quality: ImpactPeriodQuality;
      statement_changes: ImpactStatement[];
    };
  };
  other_simultaneous_changes?: Record<string, number>;
}
export interface ChangeImpact {
  id: number; intervention_id: number; before_from_ms: number; before_to_ms: number;
  after_from_ms: number; after_to_ms: number;
  result: ImpactResult;
}
export interface CaseComparison {
  id: number; kind: string; created_at_ms: number;
  members: { artifact_id: number; purpose: string; side: string; stale: boolean }[];
  result: unknown;
}
export interface InvestigationContext {
  timeline: TimelineItem[];
  interventions: Intervention[];
  applied_changes: WorkspaceChange[];
  change_impacts: ChangeImpact[];
  comparisons: CaseComparison[];
  fingerprint: Record<string, string | string[] | number[] | null>;
  similar_incidents: { case_id: number; title: string; score: number; matches: string[]; detected_at_ms: number | null }[];
  similar_search_total_candidates: number; similar_search_scanned: number;
}
export interface InvestigationState {
  caseId: number | null;
  context: InvestigationContext | null;
  loading: boolean; saving: boolean; error: string; notice: string;
}
export function createInvestigationState(): InvestigationState {
  return { caseId: null, context: null, loading: false, saving: false, error: "", notice: "" };
}
export function createInvestigationController(shell: ShellState, state: InvestigationState, invoke: PgXrayInvoke) {
  let revision = 0;
  const valid = (request: number, auth: number, caseId: number) =>
    request === revision && state.caseId === caseId && isCurrentAuthGeneration(shell, auth);
  function reset(caseId: number | null = null) {
    ++revision;
    Object.assign(state, createInvestigationState(), { caseId });
  }
  async function refresh(): Promise<void> {
    const caseId = state.caseId;
    if (caseId === null) return;
    const request = ++revision;
    const auth = shell.authGeneration;
    state.loading = true;
    state.error = "";
    try {
      const context = await invoke<InvestigationContext>("case_context", { caseId });
      if (valid(request, auth, caseId)) state.context = context;
    } catch (error) {
      if (valid(request, auth, caseId)) state.error = String(error);
    } finally {
      if (valid(request, auth, caseId)) state.loading = false;
    }
  }
  async function mutate(action: () => Promise<unknown>, message: string): Promise<boolean> {
    const caseId = state.caseId;
    if (caseId === null || state.saving) return false;
    const auth = shell.authGeneration;
    state.saving = true;
    state.error = "";
    state.notice = "";
    try {
      await action();
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, auth)) {
        state.notice = message;
        await refresh();
      }
      return true;
    } catch (error) {
      if (state.caseId === caseId && isCurrentAuthGeneration(shell, auth)) state.error = String(error);
      return false;
    } finally { state.saving = false; }
  }
  function createIntervention(occurredAtMs: number, kind: string, title: string, note: string) {
    const caseId = state.caseId;
    if (caseId === null || !Number.isFinite(occurredAtMs) || occurredAtMs <= 0 || !title.trim()) return Promise.resolve(false);
    return mutate(() => invoke("create_case_intervention", { caseId, occurredAtMs, kind, title: title.trim(), note: note.trim() }), t('tsFinish.caseInterventionRecorded'));
  }
  function deleteIntervention(interventionId: number) {
    if (!state.context?.interventions.some(item => item.id === interventionId)) return Promise.resolve(false);
    return mutate(() => invoke("delete_case_intervention", { interventionId }), t('tsFinish.caseInterventionDeleted'));
  }
  function recordImpact(caseData: ToolCase, interventionId: number, periods: [number, number, number, number]) {
    if (state.caseId !== caseData.id || !state.context?.interventions.some(item => item.id === interventionId)) return Promise.resolve(false);
    const [beforeFromMs, beforeToMs, afterFromMs, afterToMs] = periods;
    const intervention = state.context.interventions.find(item => item.id === interventionId)!;
    if (periods.some(value => !Number.isFinite(value) || value <= 0)
      || beforeFromMs > beforeToMs || afterFromMs > afterToMs
      || beforeToMs > intervention.occurred_at_ms || intervention.occurred_at_ms > afterFromMs) {
      state.error = t('tsFinish.caseInvalidPeriods');
      return Promise.resolve(false);
    }
    return mutate(() => invoke("record_case_change_impact", {
      connectionId: caseData.connection_id, caseId: caseData.id, interventionId,
      beforeFromMs, beforeToMs, afterFromMs, afterToMs,
    }), t('tsFinish.caseImpactSaved'));
  }
  function deleteImpact(impactId: number) {
    if (!state.context?.change_impacts.some(item => item.id === impactId)) return Promise.resolve(false);
    return mutate(() => invoke("delete_case_change_impact", { impactId }), t('tsFinish.caseImpactDeleted'));
  }
  return { reset, refresh, createIntervention, deleteIntervention, recordImpact, deleteImpact };
}
