import type { AvTable } from "./controller.js";
import type { AvWorkloadWindow } from "./workload.js";

export interface AvUpdateTotals {
  updates: number;
  hot: number;
  notHot: number;
  hotPct: number | null;
}

export interface AvUpdatePeriod {
  olderAt: number;
  newerAt: number;
  elapsedMinutes: number;
  resetVerified: boolean;
  updates: number;
  hot: number;
  notHot: number;
  hotPct: number | null;
  updatesPerHour: number;
  hotPerHour: number;
  notHotPerHour: number;
}

export interface AvUpdatePlotBar extends AvUpdatePeriod {
  xUpdate: number;
  xHot: number;
  yUpdate: number;
  yHot: number;
  hUpdate: number;
  hHot: number;
}

export interface AvUpdatePlot {
  bars: AvUpdatePlotBar[];
  width: number;
  height: number;
  left: number;
  baseline: number;
  maximum: number;
  barWidth: number;
}

const validCount = (value: number): boolean => Number.isSafeInteger(value) && value >= 0;
const share = (hot: number, updates: number): number | null => updates > 0 ? hot * 100 / updates : null;

/** A live cumulative-counter ratio, not the share in the last observation interval. */
export function avUpdateTotals(table: Pick<AvTable, "tuples_updated" | "hot_updates"> | null): AvUpdateTotals | null {
  if (!table || !validCount(table.tuples_updated) || !validCount(table.hot_updates)
      || table.hot_updates > table.tuples_updated) return null;
  return {
    updates: table.tuples_updated,
    hot: table.hot_updates,
    notHot: table.tuples_updated - table.hot_updates,
    hotPct: share(table.hot_updates, table.tuples_updated),
  };
}

/** Observational arithmetic only: must never feed the policy advisor or Apply. */
export function avUpdatePeriod(window: AvWorkloadWindow | null): AvUpdatePeriod | null {
  if (!window || window.status !== "observed" || !window.updates || !window.hotUpdates
      || !Number.isFinite(window.elapsedMinutes) || window.elapsedMinutes < 1
      || !validCount(window.updates.delta) || !validCount(window.hotUpdates.delta)
      || window.hotUpdates.delta > window.updates.delta
      || !Number.isFinite(window.updates.averagePerHour) || window.updates.averagePerHour < 0
      || !Number.isFinite(window.hotUpdates.averagePerHour) || window.hotUpdates.averagePerHour < 0) return null;
  const updatesPerHour = window.updates.delta * 60 / window.elapsedMinutes;
  const hotPerHour = window.hotUpdates.delta * 60 / window.elapsedMinutes;
  if (!Number.isFinite(updatesPerHour) || !Number.isFinite(hotPerHour)) return null;
  return {
    olderAt: window.olderAt,
    newerAt: window.newerAt,
    elapsedMinutes: window.elapsedMinutes,
    resetVerified: window.resetVerified,
    updates: window.updates.delta,
    hot: window.hotUpdates.delta,
    notHot: window.updates.delta - window.hotUpdates.delta,
    hotPct: share(window.hotUpdates.delta, window.updates.delta),
    updatesPerHour,
    hotPerHour,
    notHotPerHour: updatesPerHour - hotPerHour,
  };
}

/** Each pair of bars describes a separate time interval, with no interpolation between gaps. */
export function avUpdatePlot(windows: AvWorkloadWindow[], limit = 8): AvUpdatePlot {
  const width = 900, height = 230, left = 70, baseline = 178, top = 20, right = 18;
  const selected = windows.map(avUpdatePeriod).filter((period): period is AvUpdatePeriod => period !== null)
    .sort((a, b) => b.newerAt - a.newerAt).slice(0, Math.max(0, Math.min(12, Math.trunc(limit)))).reverse();
  const maximum = Math.max(0, ...selected.map(period => period.updatesPerHour));
  const groupWidth = (width - left - right) / Math.max(1, selected.length);
  const barWidth = Math.min(24, groupWidth * .26);
  const bars = selected.map((period, index): AvUpdatePlotBar => {
    const center = left + groupWidth * (index + .5);
    const hUpdate = maximum > 0 ? period.updatesPerHour / maximum * (baseline - top) : 0;
    const hHot = maximum > 0 ? period.hotPerHour / maximum * (baseline - top) : 0;
    return {
      ...period,
      xUpdate: center - barWidth - 2,
      xHot: center + 2,
      yUpdate: baseline - hUpdate,
      yHot: baseline - hHot,
      hUpdate, hHot,
    };
  });
  return { bars, width, height, left, baseline, maximum, barWidth };
}
