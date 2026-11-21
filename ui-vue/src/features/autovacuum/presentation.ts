import { locale } from "../i18n/locale-service.js";
import { t } from "../i18n/locale-service.js";
import type { AvAdvice } from "./controller.js";

/** Preserve every distinct backend limitation, but do not repeat identical advice on screen. */
export function avUniqueNotes(notes: string[] | null | undefined): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const item of notes ?? []) {
    const note = item.trim().replace(/\s+/g, " ");
    if (!note || seen.has(note)) continue;
    seen.add(note);
    result.push(note);
  }
  return result;
}

/** Presentation guidance only; policy eligibility remains fully controlled by Rust. */
export function avAdviceNextStep(
  advice: AvAdvice | null,
  context?: { savedPoints?: number | null; resetUnknown?: boolean },
): string | null {
  if (!advice) return null;
  if (context?.resetUnknown && (context.savedPoints ?? 0) > advice.history_points) {
    return t('tsFinish.avHistoryUnconfirmed', { saved: String(context.savedPoints), usable: String(advice.history_points) });
  }
  if (advice.history_points < 3) {
    return t('tsFinish.avNeedThree', { count: String(advice.history_points) });
  }
  if (advice.history_span_seconds === null || advice.history_span_seconds < 1800) {
    return t('tsFinish.avNeed30Min');
  }
  if (!advice.stats_period_continuous) {
    return t('tsFinish.avNotContinuous');
  }
  if (advice.status === "investigate") return t('tsFinish.avInvestigate');
  if (advice.status === "candidate") return t('tsFinish.avCandidate');
  if (advice.status === "working") return t('tsFinish.avWorking');
  return t('tsFinish.avReadAdvice');
}

/** Avoid telling users to collect more snapshots when stats_reset is unknown.
 * This changes only the label, never Rust's evidence or Apply eligibility. */
export function avAdvisorEvidenceMessage(
  advice: AvAdvice | null,
  context: { savedPoints: number; resetUnknown: boolean },
): { title: string; message: string } | null {
  if (!advice || advice.status !== "insufficient_history" || !context.resetUnknown || context.savedPoints <= advice.history_points) return null;
  return {
    title: t('tsFinish.avUnknownHistoryTitle'),
    message: t('tsFinish.avEvidenceWarning', { saved: String(context.savedPoints), usable: String(advice.history_points) }),
  };
}

/** Requirements actually used by Rust Advisor before it can evaluate cadence.
 * Diagnostic display only: Rust still decides if a candidate is eligible. */
export function avAdvisorRequirements(
  advice: AvAdvice | null,
  context: { savedPoints: number; trackCounts: boolean; statsReset: string | null },
): { label: string; met: boolean; explanation: string }[] {
  const comparable = advice?.history_points ?? 0;
  const span = advice?.history_span_seconds ?? null;
  return [
    {
      label: t('tsFinish.avCountsEnabled'),
      met: context.trackCounts,
      explanation: context.trackCounts ? "track_counts = on" : t('tsFinish.avCountsOff'),
    },
    {
      label: t('tsFinish.avResetKnown'),
      met: context.statsReset != null,
      explanation: context.statsReset == null
        ? t('tsFinish.avResetNull')
        : t('tsFinish.avResetLabel', { time: String(context.statsReset) }),
    },
    {
      label: t('tsFinish.avThreeSnapshots'),
      met: comparable >= 3,
      explanation: t('tsFinish.avSeriesCount', { saved: String(context.savedPoints), usable: String(comparable) }),
    },
    {
      label: t('tsFinish.avThirtyMinutes'),
      met: span != null && span >= 1800,
      explanation: span == null ? t('tsFinish.avUnknownSpan')
        : t('tsFinish.avSeriesMinutes', { minutes: (span / 60).toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US', { maximumFractionDigits: 1 }) }),
    },
    {
      label: t('tsFinish.avStableCounters'),
      met: advice?.stats_period_continuous === true,
      explanation: t('tsFinish.avRustValidation'),
    },
  ];
}
