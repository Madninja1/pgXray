/** UI-only sorting and suppression of unchanged metrics. Never modify saved comparisons. */
export type Value = string | number | null | undefined;
export type PeriodValues = Record<string, unknown> | null | undefined;

export function changed(before: PeriodValues, after: PeriodValues, key: string): boolean {
  const left = before?.[key] ?? null;
  const right = after?.[key] ?? null;
  return !Object.is(left, right);
}

export const autovacuumFields = [
  "inserts_per_hour", "updates_per_hour", "deletes_per_hour",
  "vacuum_cycles", "analyze_cycles", "latest_dead_tuples",
] as const;

export function autovacuumHasChanges(row: {
  state: string; baseline: PeriodValues; incident: PeriodValues;
  policy_changed?: boolean; policy_changes?: string[];
}): boolean {
  return row.state !== "observed_in_both" || row.policy_changed === true
    || (row.policy_changes?.length ?? 0) > 0
    || autovacuumFields.some(key => changed(row.baseline, row.incident, key));
}

export function xidHasChanges(row: {
  state: string; baseline: PeriodValues; incident: PeriodValues; freeze_policy_changed?: boolean;
}): boolean {
  return row.state !== "observed_in_both" || row.freeze_policy_changed === true
    || ["latest_xid_age", "latest_pct_of_freeze_max", "latest_effective_freeze_max_age"]
      .some(key => changed(row.baseline, row.incident, key));
}

export function pgssMetricChanged(metric: { direction: string }): boolean {
  return metric.direction === "up" || metric.direction === "down"
    || metric.direction === "appeared" || metric.direction === "not_sampled";
}

export function orderRows<T>(rows: readonly T[], selector: (row: T) => Value, descending = false): T[] {
  return rows.map((row, index) => ({ row, index, value: selector(row) }))
    .sort((a, b) => {
      const missingA = a.value === null || a.value === undefined || a.value === "";
      const missingB = b.value === null || b.value === undefined || b.value === "";
      if (missingA !== missingB) return missingA ? 1 : -1; // unavailable data last in either direction
      let compared = 0;
      if (!missingA && !missingB) {
        if (typeof a.value === "number" && typeof b.value === "number") {
          compared = a.value < b.value ? -1 : a.value > b.value ? 1 : 0;
        } else {
          compared = String(a.value).localeCompare(String(b.value), "ru", { numeric: true });
        }
      }
      return (descending ? -compared : compared) || a.index - b.index;
    })
    .map(entry => entry.row);
}
