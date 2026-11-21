/** Pure, stable sorting for persisted History tables; never changes the source order. */
export type HistorySortDirection = "asc" | "desc";
export interface HistorySortOrder { column: string; direction: HistorySortDirection }

export function historySortValue(row: unknown, path: string): unknown {
  let value = row;
  for (const key of path.split(".")) {
    if (value === null || typeof value !== "object" || !(key in value)) return null;
    value = (value as Record<string, unknown>)[key];
  }
  return value;
}

function normalized(value: unknown): string | number | null {
  if (value === null || value === undefined || value === "") return null;
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "boolean") return value ? 1 : 0;
  if (typeof value === "bigint") return Number(value);
  if (Array.isArray(value)) return value.join(", ");
  if (typeof value === "string") {
    // Timestamp strings with an explicit date and time, including PostgreSQL ISO timestamps.
    if (/^\d{4}-\d\d-\d\d[T ]\d\d:\d\d/.test(value)) {
      const timestamp = Date.parse(value);
      if (Number.isFinite(timestamp)) return timestamp;
    }
    return value;
  }
  return null;
}

const collator = new Intl.Collator("ru", { numeric: true, sensitivity: "base" });

export function sortHistoryRows<T>(
  rows: readonly T[], order: HistorySortOrder | undefined,
  valueFor?: (row: T, column: string) => unknown,
): T[] {
  if (!order) return [...rows];
  const multiplier = order.direction === "asc" ? 1 : -1;
  return rows.map((row, index) => ({ row, index, value: normalized(valueFor ? valueFor(row, order.column) : historySortValue(row, order.column)) }))
    .sort((a, b) => {
      // Unknown/reset metrics are always last, irrespective of direction.
      if (a.value === null || b.value === null) {
        if (a.value === null && b.value !== null) return 1;
        if (b.value === null && a.value !== null) return -1;
        return a.index - b.index;
      }
      const result = typeof a.value === "number" && typeof b.value === "number"
        ? a.value - b.value
        : collator.compare(String(a.value), String(b.value));
      return result * multiplier || a.index - b.index;
    })
    .map(({ row }) => row);
}

export function nextHistorySort(current: HistorySortOrder | undefined, column: string): HistorySortOrder {
  return { column, direction: current?.column === column && current.direction === "asc" ? "desc" : "asc" };
}
