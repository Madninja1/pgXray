import { t } from "../i18n/locale-service.js";
import type { AvTable } from "./controller.js";
import type { AvWorkloadWindow } from "./workload.js";

export type AvMaintenanceKind = "dead" | "insert" | "analyze";
export type AvMaintenanceState = "unknown" | "disabled" | "reached" | "below";

/** A UI interpretation of SQL-derived triggers, never a scheduling prediction. */
export function avMaintenanceIndicator(table: AvTable, kind: AvMaintenanceKind, reliable: boolean) {
  const observed = kind === "dead" ? table.dead_tuples :
    kind === "insert" ? table.inserts_since_vacuum : table.modifications_since_analyze;
  const trigger = kind === "dead" ? table.dead_tuple_trigger :
    kind === "insert" ? table.insert_trigger : table.analyze_trigger;
  const usedPct = kind === "dead" ? table.dead_trigger_used_pct :
    kind === "insert" ? table.insert_trigger_used_pct : table.analyze_trigger_used_pct;
  const applicable = table.autovacuum_applicable && table.autovacuum_enabled;
  const state: AvMaintenanceState = !applicable || (kind === "insert" && trigger === null)
    ? "disabled"
    : !reliable || !Number.isFinite(observed) || trigger === null || !Number.isFinite(trigger) || trigger <= 0
      ? "unknown"
      : observed >= trigger ? "reached" : "below";
  const remaining = state === "below" && trigger !== null ? Math.max(0, trigger - observed) : null;
  return {
    kind, state,
    observed: !reliable || state === "unknown" ? null : observed,
    trigger: state === "unknown" ? null : trigger,
    usedPct: state === "unknown" || state === "disabled" || usedPct === null || !Number.isFinite(usedPct) ? null : usedPct,
    remaining,
    label: state === "disabled" ? (kind === "insert" && trigger === null ? t('tsFinish.avInsertDisabled') : t('tsFinish.avMaintenanceDisabled')) :
      state === "unknown" ? t('tsFinish.avUnknownEvidence') :
      state === "reached" ? t('tsFinish.avReached') : t('tsFinish.avBelow'),
  };
}

/** HOT ratio is cumulative UPDATE statistics; a 5% threshold is only pgXray's investigation heuristic. */
export function avHotGuidance(table: Pick<AvTable, "hot_update_pct" | "tuples_updated">, reliable: boolean) {
  if (!reliable || !Number.isFinite(table.tuples_updated) || table.tuples_updated < 0 ||
      table.hot_update_pct === null || !Number.isFinite(table.hot_update_pct)) {
    return { state: "unknown" as const, message: t('tsFinish.avHotInsufficient') };
  }
  if (table.tuples_updated === 0) {
    return { state: "no-updates" as const, message: t('tsFinish.avNoUpdates') };
  }
  if (table.hot_update_pct < 5) {
    return { state: "investigate" as const, message: t('tsFinish.avHotInvestigate') };
  }
  return { state: "observed" as const, message: t('tsFinish.avHotObserved') };
}

/** Observations attached to a specific maintenance condition, not a VACUUM schedule. */
export function avMechanismWindowEvidence(kind: AvMaintenanceKind, window: AvWorkloadWindow | null) {
  if (!window) return null;
  const estimate = kind === "dead" ? window.deadTuplesChange :
    kind === "insert" ? window.insertsSinceVacuumChange : window.modificationsSinceAnalyzeChange;
  const rates = kind === "dead"
    ? [{ label: "UPDATE", perHour: window.updates?.averagePerHour ?? null },
       { label: "DELETE", perHour: window.deletes?.averagePerHour ?? null }]
    : kind === "insert"
      ? [{ label: "INSERT", perHour: window.inserts?.averagePerHour ?? null }]
      : [{ label: "INSERT", perHour: window.inserts?.averagePerHour ?? null },
         { label: "UPDATE", perHour: window.updates?.averagePerHour ?? null },
         { label: "DELETE", perHour: window.deletes?.averagePerHour ?? null }];
  return { elapsedMinutes: window.elapsedMinutes, estimate, rates, resetVerified: window.resetVerified };
}
