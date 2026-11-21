import { reactive } from "vue";
import { nextHistorySort, sortHistoryRows, type HistorySortOrder } from "./sort-rows.js";

/** Independent sort state for each table, including dynamically named comparison sections. */
export function useHistorySorting() {
  const orders = reactive<Record<string, HistorySortOrder>>({});
  return {
    get: (table: string): HistorySortOrder | undefined => orders[table],
    toggle: (table: string, column: string): void => { orders[table] = nextHistorySort(orders[table], column); },
    sorted: <T>(table: string, rows: readonly T[], valueFor?: (row: T, column: string) => unknown): T[] =>
      sortHistoryRows(rows, orders[table], valueFor),
  };
}
