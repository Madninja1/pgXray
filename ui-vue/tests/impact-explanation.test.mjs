import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from "node:test";
import assert from "node:assert/strict";
import { impactLabel, impactExplanation, impactSignals, IMPACT_GUIDE } from "../dist/features/tools/impact-explanation.js";

const quality = (count = 2, safe = 1) => ({ state: safe ? "usable" : "insufficient", snapshot_count: count, safe_interval_count: safe, unsafe_interval_count: 0, snapshot_ids: [1, 2], limitations: [] });
const result = (assessment, before = quality(), after = quality(), metrics = []) => ({
  assessment,
  comparison: { pgss: { baseline_quality: before, incident_quality: after, statement_changes: [{ queryid: "123", metrics }] } },
});
test("guides explain purpose instead of showing an unexplained verdict", () => {
  assert.match(IMPACT_GUIDE.impact, /SQLite/);
  assert.match(IMPACT_GUIDE.interventions, /не выполняет SQL/);
  assert.match(IMPACT_GUIDE.similar, /не определяют общую первопричину/);
});
test("no clear before-after signal explains absence of confirmed per-call changes", () => {
  const value = result({ state: "no_clear_signal", improvement_signals: 0, regression_signals: 0 });
  assert.match(impactLabel(value), /Нет явного сигнала/);
  assert.match(impactExplanation(value), /пяти метрик на вызов/);
  assert.equal(impactSignals(value).length, 0);
});
test("incomplete snapshot series explains data quality instead of misleading users", () => {
  assert.match(impactExplanation(result({ state: "no_clear_signal" }, quality(1, 0))), /меньше двух/);
  assert.match(impactExplanation(result({ state: "no_clear_signal" }, quality(2, 0))), /Нет безопасных/);
});
test("only five confirmed direction metrics are displayed, not calls or unconfirmed observations", () => {
  const metric = (id, level = "confirmed_change") => ({ id, label: id, unit: "ms/call", baseline_value: 2, incident_value: 1, direction: "down", evidence_level: level });
  const value = result({ state: "improvement_observed", improvement_signals: 1 }, quality(), quality(), [
    metric("exec_ms_per_call"), metric("calls_per_hour"), metric("wal_bytes_per_call", "observed"),
  ]);
  assert.match(impactLabel(value), /улучшения/);
  assert.deepEqual(impactSignals(value).map(item => item.label), ["exec_ms_per_call"]);
});
