import { locale, t } from "../features/i18n/locale-service.js";
/**
 * Formatting helpers migrated from ui/js/core.js (2026-11-09 archive).
 * Keep the legacy output intact during the Vue/TypeScript migration.
 */
export type NumericInput = number | bigint | string | null | undefined;

export function formatInteger(value: NumericInput): string {
  if (value === null || value === undefined) {
    return "—";
  }

  // Keep the original Intl.NumberFormat coercion, including bigint/string inputs.
  return new Intl.NumberFormat(locale.value === "ru" ? "ru-RU" : "en-US").format(value as number);
}

export function formatNumber(value: NumericInput, digits = 2): string {
  if (value === null || value === undefined) {
    return "—";
  }

  return new Intl.NumberFormat(locale.value === "ru" ? "ru-RU" : "en-US", {
    maximumFractionDigits: digits,
  }).format(Number(value));
}

export function formatPercent(value: NumericInput): string {
  if (value === null || value === undefined) {
    return "—";
  }

  return `${Number(value).toFixed(1)}%`;
}

export function formatBytes(value: NumericInput): string {
  if (value === null || value === undefined) {
    return "—";
  }

  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let size = Number(value);
  let unit = 0;

  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }

  const digits = unit === 0 || size >= 100 ? 0 : 1;
  return `${size.toFixed(digits)} ${units[unit]}`;
}

export function formatDuration(seconds: NumericInput): string {
  if (seconds === null || seconds === undefined) {
    return "—";
  }

  let rest = Math.max(0, Number(seconds));
  const days = Math.floor(rest / 86400);
  rest %= 86400;
  const hours = Math.floor(rest / 3600);
  rest %= 3600;
  const minutes = Math.floor(rest / 60);
  rest %= 60;

  const parts: string[] = [];
  if (days) parts.push(t('tsFinish.shortDays', { value: String(days) }));
  if (hours) parts.push(t('tsFinish.shortHours', { value: String(hours) }));
  if (minutes) parts.push(t('tsFinish.shortMinutes', { value: String(minutes) }));
  if (parts.length === 0) parts.push(t('tsFinish.shortSeconds', { value: String(Math.floor(rest)) }));

  return parts.join(" ");
}

export function formatTimestamp(value: string | number | null | undefined): string {
  if (!value) {
    return "—";
  }

  const date = typeof value === "string" ? new Date(value) : new Date(value);
  if (Number.isNaN(date.getTime())) {
    return String(value);
  }

  return date.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US");
}

export function yesNo(value: unknown): string {
  return value ? t('tsFinish.fmtYes') : t('tsFinish.fmtNo');
}

export function formatSignedInteger(value: NumericInput): string {
  if (value === null || value === undefined) {
    return "—";
  }

  const number = Number(value);
  const formatted = formatInteger(Math.abs(number));
  if (number > 0) return `+${formatted}`;
  if (number < 0) return `-${formatted}`;
  return "0";
}

export function formatSnapshotTime(milliseconds: NumericInput): string {
  if (milliseconds === null || milliseconds === undefined) {
    return "—";
  }

  const date = new Date(Number(milliseconds));
  if (Number.isNaN(date.getTime())) {
    return "—";
  }

  return date.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US");
}

export function relationSampleStateLabel(state: string): string {
  if (state === "present_in_both") return t('tsFinish.fmtBothTop');
  if (state === "entered_new_sample") return t('tsFinish.fmtEnteredTop');
  if (state === "not_in_new_sample") return t('tsFinish.fmtNotNewTop');
  return state;
}
