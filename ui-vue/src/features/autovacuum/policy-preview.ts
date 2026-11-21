interface UnverifiedPolicyDetail {
  table: {
    autovacuum_applicable: boolean;
    autovacuum_enabled: boolean;
    effective_vacuum_scale_factor: number;
    dead_tuple_trigger: number;
  };
  fixed_scale_candidates: { scale_factor: number; resulting_dead_tuple_trigger: number }[];
}

/**
 * Read-only threshold scenarios; these do not imply continuous statistics,
 * observed rates, an Advisor recommendation, or permission to Apply.
 * Numerical thresholds are calculated by the Rust collector for the current
 * PostgreSQL version (including PG18 threshold caps).
 */
export function avUnverifiedPolicyOptions(
  statsReset: string | null,
  trackCounts: boolean,
  detail: UnverifiedPolicyDetail | null,
): UnverifiedPolicyDetail["fixed_scale_candidates"] {
  if (statsReset !== null || !trackCounts || !detail?.table.autovacuum_applicable || !detail.table.autovacuum_enabled) return [];

  const currentFactor = detail.table.effective_vacuum_scale_factor;
  const currentTrigger = detail.table.dead_tuple_trigger;
  if (!Number.isFinite(currentFactor) || currentFactor <= 0 || !Number.isFinite(currentTrigger) || currentTrigger <= 0) return [];

  return detail.fixed_scale_candidates
    .filter(({ scale_factor, resulting_dead_tuple_trigger }) =>
      Number.isFinite(scale_factor) && scale_factor >= 0 && scale_factor < currentFactor &&
      Number.isSafeInteger(resulting_dead_tuple_trigger) && resulting_dead_tuple_trigger > 0 &&
      resulting_dead_tuple_trigger < currentTrigger)
    .sort((a, b) => b.scale_factor - a.scale_factor);
}
