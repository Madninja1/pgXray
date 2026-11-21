<script setup lang="ts">
import { confirmAction } from "../../core/confirm-action.js";
import { locale, t } from "../i18n/locale-service.js";
import { computed, nextTick, onMounted, reactive, ref, shallowRef, watch } from "vue";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import type { QuickConnectionSelection } from "../../core/connection-shortcuts.js";
import SnapshotCoverage from "./SnapshotCoverage.vue";
import DateTimeField from "./DateTimeField.vue";
import { suggestComparisonPeriods, type SnapshotCollections, type SuggestedPeriods } from "./period-suggestions.js";
import { autovacuumHasChanges, changed, orderRows, pgssMetricChanged, xidHasChanges, type Value } from "./table-display.js";
import { groupTimeline, timelineSourceLabel } from "./timeline-groups.js";

type Entry = Record<string, unknown>;
interface Connection { id: number; name: string; host: string; port: number; database: string }
interface HistoryConnection { connection_id: number; connection_name: string; snapshot_count: number }
interface Case { id: number; title: string; connection_id: number }
interface Marker { id: number; connection_id: number; occurred_at_ms: number; kind: string; title: string; note: string }
interface Quality {
  state: string; snapshot_count: number; limitations: string[]; safe_interval_count?: number;
  safe_elapsed_seconds?: number; blocks_hit_observed_seconds?: number;
  wal_bytes_observed_seconds?: number; checkpoints_timed_observed_seconds?: number;
}
interface Event { occurred_at_ms: number; phase: string; source: string; kind: string; title: string; detail: string; evidence_level: string; evidence_refs: string[] }
interface MetricChange { id: string; label: string; direction: string; baseline_value: number | null; incident_value: number | null; unit: string }
interface Statement { userid: number; dbid: number; queryid: string; toplevel: boolean; role_name: string | null; database: string | null; state: string; evidence_level: string; metrics: MetricChange[]; baseline?: Entry | null; incident?: Entry | null }
interface RelationChange { relation_oid: number; relation_name: string; state: string; evidence_level: string; baseline: Entry | null; incident: Entry | null; policy_changed?: boolean; policy_changes?: string[]; freeze_policy_changed?: boolean }
interface SettingsChange { change: Entry; first_observed_at_ms: number | null; changed_after_ms: number | null; changed_before_ms: number | null }
interface Comparison {
  generated_at_ms: number; connection_id: number;
  baseline: { from_ms: number; to_ms: number }; incident: { from_ms: number; to_ms: number };
  target: { host: string; port: number; database: string; username: string };
  pgss: { baseline_quality: Quality; incident_quality: Quality; statement_changes: Statement[]; limitations: string[] };
  settings: { state: string; changes: SettingsChange[]; limitations: string[] };
  autovacuum: { baseline_quality: Quality; incident_quality: Quality; relation_changes: RelationChange[]; limitations: string[] };
  xid: { baseline: Entry; incident: Entry; relation_changes: RelationChange[]; limitations: string[] };
  storage: { baseline: Entry; incident: Entry; baseline_quality: Quality; incident_quality: Quality; relation_size_changes: Entry[]; limitations: string[] };
  index_health: { changes: Entry[]; usage_changes?: Entry[]; stats_period_continuous: boolean; limitations: string[] };
  markers: Marker[]; timeline: Event[]; limitations: string[];
}
interface Saved { case: Case; artifact: { id: number }; comparison: Comparison }
const props = defineProps<{ shell: ShellState; invoke: PgXrayInvoke; visible: boolean; quickSelection?: QuickConnectionSelection | null }>();
const emit = defineEmits<{ openHistory: []; openSchedules: [] }>();
const coverageToken = ref(0);
const suggestion = shallowRef<SuggestedPeriods | null>(null);
const snapshotsAvailable = ref(false);
let timeEdited = false;
const coverageRange = computed(() => { try { return periods() as { baselineFromMs: number; baselineToMs: number; incidentFromMs: number; incidentToMs: number }; } catch { return null; } });
const periodError = computed(() => {
  try { periods(); return ""; } catch { return t('changes.periodError'); }
});
const comparisonClock = ref(Date.now());
const futurePeriodWarning = computed(() => {
  if (periodError.value) return "";
  const end = periods().incidentToMs;
  const observedAt = result.value?.generated_at_ms ?? comparisonClock.value;
  return end > observedAt
    ? t('changes.futurePeriod', { end: fmtDate(end), at: fmtDate(observedAt) })
    : "";
});
const connections = ref<Connection[]>([]);
const historyConnections = ref<HistoryConnection[]>([]);
const connectionId = ref<number | null>(null);
const cases = ref<Case[]>([]);
const caseId = ref<number | null>(null);
const markers = ref<Marker[]>([]);
const result = shallowRef<Comparison | null>(null);
const expandedTimeline = ref(false);
const timelineGroups = computed(() => groupTimeline(result.value?.timeline ?? []));
const collapsibleTimelineGroups = computed(() => timelineGroups.value.filter(group => group.events.length > 1).length);
const pending = reactive({ loading: false, comparing: false, saving: false, marker: false });
const error = ref(""); const markerError = ref(""); const notice = ref(""); const caseError = ref("");
const time = reactive({ baselineFrom: "", baselineTo: "", incidentFrom: "", incidentTo: "" });
const markerDraft = reactive({ kind: "deploy", title: "", note: "", at: "" });
let revision = 0; let requestId = 0; let bootId = 0;
function local(ms: number): string {
  const date = new Date(ms);
  return new Date(ms - date.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
}
function fmtDate(ms: number | null | undefined): string {
  return typeof ms === "number" && Number.isFinite(ms) ? new Date(ms).toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US") : "—";
}
function fmt(value: unknown, digits = 1): string {
  if (typeof value === "number") return Number.isFinite(value) ? value.toLocaleString(locale.value === "ru" ? "ru-RU" : "en-US", { maximumFractionDigits: digits }) : "—";
  return value === null || value === undefined || value === "" ? "—" : String(value);
}
function fmtPair(before: Entry | null | undefined, after: Entry | null | undefined, key: string): string {
  return `${fmt(before?.[key])} → ${fmt(after?.[key])}`;
}
function settingReasons(change: Entry): string {
  if (change.state === "added") return t('changes.changeAdded');
  if (change.state === "removed") return t('changes.changeRemoved');
  const reasons = [
    change.value_changed && t('changes.reasonValue'),
    change.reset_value_changed && t('changes.reasonReset'),
    change.source_changed && t('changes.reasonSource'),
    change.pending_restart_changed && t('changes.reasonRestart'),
  ].filter(Boolean);
  return reasons.length ? reasons.join("; ") : t('changes.metadata');
}
function fmtSettingSource(change: Entry, prefix: "older" | "newer"): string {
  return `${fmt(change[`${prefix}_source`])} · ${fmt(change[`${prefix}_sourcefile`])}:${fmt(change[`${prefix}_sourceline`])}`;
}
function fmtDuration(seconds: number | undefined): string {
  return typeof seconds === "number" && Number.isFinite(seconds) && seconds > 0
    ? t('whatLast.minutes', { value: (seconds / 60).toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US', { maximumFractionDigits: 1 }) }) : t('changes.noSafeInterval');
}
// Presentation controls: filtering never changes comparisons stored in Rust/SQLite.
type SortSection = "pgss" | "settings" | "autovacuum" | "xid" | "storage" | "sizes" | "usage" | "index";
const onlyDifferences = ref(true);
const sorts = reactive<Record<SortSection, { key: string; descending: boolean } | null>>({
  pgss: { key: "workload_delta", descending: true }, settings: null,
  autovacuum: { key: "dml_incident", descending: true }, xid: null,
  storage: null, sizes: null, usage: null, index: null,
});
function toggleSort(section: SortSection, key: string): void {
  const previous = sorts[section];
  sorts[section] = previous?.key === key
    ? { key, descending: !previous.descending }
    : { key, descending: !["name", "state", "queryid", "reason"].includes(key) };
}
function sortArrow(section: SortSection, key: string): string {
  const current = sorts[section];
  return current?.key === key ? (current.descending ? " ↓" : " ↑") : " ↕";
}
function tableRows<T>(section: SortSection, rows: readonly T[], value: (item: T, key: string) => Value): T[] {
  const rule = sorts[section];
  return rule ? orderRows(rows, item => value(item, rule.key), rule.descending) : [...rows];
}
function number(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
function delta(left: unknown, right: unknown): number | null {
  const a = number(left), b = number(right);
  return a === null || b === null ? null : b - a;
}
function fieldDiff(before: Entry | null | undefined, after: Entry | null | undefined, key: string): string {
  if (onlyDifferences.value) {
    if (!changed(before, after, key)) return "—";
    // XID freeze percentage is shown with one decimal: do not present rounding noise as a visible change.
    if (key === "latest_pct_of_freeze_max" && fmt(before?.[key]) === fmt(after?.[key])) return "—";
  }
  return fmtPair(before, after, key);
}
function avLines(row: RelationChange, fields: readonly (readonly [string, string])[]): string[] {
  return fields.filter(([, key]) => !onlyDifferences.value || changed(row.baseline, row.incident, key))
    .map(([label, key]) => `${label}: ${fmtMetricPair(number(row.baseline?.[key]), number(row.incident?.[key]))}`);
}
function visibleStatementMetrics(statement: Statement): MetricChange[] {
  return onlyDifferences.value ? statement.metrics.filter(pgssMetricChanged) : statement.metrics;
}
function pgssMetric(statement: Statement, id: string): MetricChange | undefined {
  return statement.metrics.find(metric => metric.id === id);
}
function pgssSortValue(statement: Statement, key: string): Value {
  if (key === "queryid") return statement.queryid;
  if (key === "state") return statement.state;
  if (key === "workload_delta") {
    const metric = pgssMetric(statement, "total_exec_per_hour");
    return delta(metric?.baseline_value, metric?.incident_value);
  }
  const metric = pgssMetric(statement, key);
  return metric?.incident_value ?? null;
}
const visiblePgss = computed(() => tableRows("pgss", (result.value?.pgss.statement_changes ?? [])
  .filter(item => !onlyDifferences.value || item.state !== "sampled_in_both" || item.metrics.some(pgssMetricChanged)), pgssSortValue));
const visibleSettings = computed(() => tableRows("settings", result.value?.settings.changes ?? [], (item, key) => {
  if (key === "name") return String(item.change["name"] ?? "");
  if (key === "reason") return settingReasons(item.change);
  return item.first_observed_at_ms;
}));
const visibleAutovacuum = computed(() => tableRows("autovacuum", (result.value?.autovacuum.relation_changes ?? [])
  .filter(item => !onlyDifferences.value || autovacuumHasChanges(item)), (item, key) => {
    if (key === "name") return item.relation_name;
    if (key === "dead_delta") return delta(item.baseline?.["latest_dead_tuples"], item.incident?.["latest_dead_tuples"]);
    if (key === "vacuum_incident") return number(item.incident?.["vacuum_cycles"]);
    if (key === "policy") return item.policy_changed ? 1 : 0;
    const metricKeys = ["inserts_per_hour", "updates_per_hour", "deletes_per_hour"];
    return metricKeys.reduce((sum, field) => sum + (number(item.incident?.[field]) ?? 0), 0);
  }));
const visibleXid = computed(() => tableRows("xid", (result.value?.xid.relation_changes ?? [])
  .filter(item => !onlyDifferences.value || xidHasChanges(item)), (item, key) => {
    if (key === "name") return item.relation_name;
    if (key === "age_delta") return delta(item.baseline?.["latest_xid_age"], item.incident?.["latest_xid_age"]);
    if (key === "pct") return number(item.incident?.["latest_pct_of_freeze_max"]);
    return number(item.incident?.["latest_xid_age"]);
  }));
const visibleStorage = computed(() => {
  const storage = result.value?.storage;
  if (!storage) return [];
  return tableRows("storage", storageMetrics.filter(([, key]) => !onlyDifferences.value
    || changed(storage.baseline, storage.incident, key)), ([name, key], sortKey) => {
    if (sortKey === "name") return name;
    const a = storage.baseline[key], b = storage.incident[key];
    return sortKey === "delta" ? delta(a, b) : number(b);
  });
});
const visibleSizes = computed(() => tableRows("sizes", result.value?.storage.relation_size_changes ?? [], (item, key) => {
  if (key === "name") return String(item["relation_name"] ?? "");
  return number(item[key === "size" ? "incident_total_size_bytes" : "total_size_delta_bytes"]);
}));
const visibleUsage = computed(() => tableRows("usage", result.value?.index_health.usage_changes ?? [], (item, key) => {
  if (key === "name") return String(item["index_name"] ?? "");
  return number(item[key === "dml" ? "relation_dml_delta" : "scan_delta"]);
}));
const visibleIndex = computed(() => tableRows("index", result.value?.index_health.changes ?? [], (item, key) => {
  if (key === "name") return String(item["index_name"] ?? "");
  if (key === "state") return String(item["state"] ?? "");
  return number(item[key === "size" ? "size_delta_bytes" : "scan_delta"]);
}));
function fmtMetric(value: number | null): string {
  if (value === null) return "—";
  if (value === 0) return "0";
  const magnitude = Math.abs(value);
  const digits = magnitude < 0.001 ? 6 : magnitude < 1 ? 3 : 1;
  return fmt(value, digits);
}
function fmtMetricPair(before: number | null, after: number | null): string {
  let older = fmtMetric(before), newer = fmtMetric(after);
  if (before !== null && after !== null && before !== after && older === newer) {
    for (let digits = 2; digits <= 9 && older === newer; digits++) {
      older = fmt(before, digits);
      newer = fmt(after, digits);
    }
    if (older === newer) {
      older = before.toExponential(3);
      newer = after.toExponential(3);
    }
  }
  return `${older} → ${newer}`;
}
function defaults(): void {
  const now = Date.now();
  comparisonClock.value = now;
  time.baselineFrom = local(now - 3 * 3_600_000);
  time.baselineTo = local(now - 2 * 3_600_000);
  time.incidentFrom = local(now - 3_600_000);
  time.incidentTo = local(now);
  markerDraft.at = local(now);
}
function periods(): Record<string, number> {
  const numbers = [time.baselineFrom, time.baselineTo, time.incidentFrom, time.incidentTo].map(value => new Date(value).getTime());
  const [a, b, c, d] = numbers;
  if ([a, b, c, d].some(value => !Number.isFinite(value) || value <= 0) || !(a < b && b <= c && c < d)) {
    throw new Error(t('changes.invalidPeriods'));
  }
  return { baselineFromMs: a, baselineToMs: b, incidentFromMs: c, incidentToMs: d };
}
function invalidate(): void {
  revision++; requestId++; result.value = null; notice.value = ""; error.value = ""; pending.comparing = false;
}
const activeConnection = computed(() => connections.value.find(item => item.id === connectionId.value));
const choices = computed(() => {
  const result = new Map<number, string>();
  for (const item of connections.value) result.set(item.id, `${item.name} · ${item.host}:${item.port}/${item.database}`);
  for (const item of historyConnections.value) if (!result.has(item.connection_id)) {
    result.set(item.connection_id, t('changes.archive', { name: item.connection_name, id: String(item.connection_id) }));
  }
  return [...result].map(([id, title]) => ({ id, title })).sort((a, b) => a.title.localeCompare(b.title, "ru"));
});
function current(auth: number, rev: number): boolean {
  return props.shell.screen === "workspace" && props.shell.authGeneration === auth && revision === rev;
}
async function refreshCases(): Promise<void> {
  const id = connectionId.value;
  if (id === null || !activeConnection.value) { cases.value = []; caseId.value = null; return; }
  const auth = props.shell.authGeneration, rev = revision;
  try {
    const entries = await props.invoke<Case[]>("list_investigation_cases", { connectionId: id });
    if (!current(auth, rev)) return;
    cases.value = entries;
    if (!cases.value.some(item => item.id === caseId.value)) caseId.value = null;
    caseError.value = "";
  } catch (e) { if (current(auth, rev)) caseError.value = `Cases: ${String(e)}`; }
}
async function refreshMarkers(): Promise<void> {
  const id = connectionId.value;
  if (id === null) { markers.value = []; return; }
  const auth = props.shell.authGeneration, rev = revision;
  let range: Record<string, number>;
  try { range = periods(); } catch { markerError.value = ""; markers.value = []; return; }
  try {
    const entries = await props.invoke<Marker[]>("list_change_markers", { connectionId: id, fromMs: range.baselineFromMs, toMs: range.incidentToMs });
    if (current(auth, rev)) { markers.value = entries; markerError.value = ""; }
  } catch (e) { if (current(auth, rev)) markerError.value = `Markers: ${String(e)}`; }
}
async function refreshConnections(): Promise<void> {
  const auth = props.shell.authGeneration, seq = ++bootId;
  pending.loading = true;
  try {
    const [live, saved] = await Promise.all([
      props.invoke<Connection[]>("list_connections"), props.invoke<HistoryConnection[]>("list_history_connections"),
    ]);
    if (props.shell.authGeneration !== auth || seq !== bootId || props.shell.screen !== "workspace") return;
    connections.value = live; historyConnections.value = saved;
    if (!choices.value.some(item => item.id === connectionId.value)) {
      connectionId.value = choices.value[0]?.id ?? null;
      clearSnapshotSelection();
      defaults();
    }
    invalidate(); coverageToken.value++;
    await Promise.all([refreshCases(), refreshMarkers()]);
  } catch (e) { if (props.shell.authGeneration === auth && seq === bootId) error.value = t('changes.connError', { error: String(e) }); }
  finally { if (props.shell.authGeneration === auth && seq === bootId) pending.loading = false; }
}
function selectConnection(raw: string): void {
  const parsed = Number(raw);
  connectionId.value = Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
  clearSnapshotSelection(); defaults();
  invalidate(); coverageToken.value++; cases.value = []; markers.value = []; caseId.value = null;
  void Promise.all([refreshCases(), refreshMarkers()]);
}
function clearSnapshotSelection(): void {
  suggestion.value = null; snapshotsAvailable.value = false; timeEdited = false;
}
function applySuggestion(): void {
  const next = suggestion.value;
  if (!next) return;
  time.baselineFrom = local(next.baselineFromMs);
  time.baselineTo = local(next.baselineToMs);
  time.incidentFrom = local(next.incidentFromMs);
  time.incidentTo = local(next.incidentToMs);
  timeEdited = true;
  periodChanged(false);
}
function snapshotsLoaded(payload: { connectionId: number; rows: SnapshotCollections }): void {
  if (connectionId.value !== payload.connectionId) return;
  snapshotsAvailable.value = true;
  suggestion.value = suggestComparisonPeriods(payload.rows);
  // Initial view or a newly selected connection: use actual SQLite history, not arbitrary "now".
  if (!timeEdited && suggestion.value) applySuggestion();
}
function periodChanged(manual = true): void {
  if (manual) timeEdited = true;
  comparisonClock.value = Date.now();
  invalidate(); markers.value = [];
  void refreshMarkers();
}
async function compare(): Promise<void> {
  const id = connectionId.value;
  if (id === null) { error.value = t('changes.connectionRequired'); return; }
  let range: Record<string, number>;
  try { range = periods(); } catch (e) { error.value = String(e); return; }
  comparisonClock.value = Date.now();
  const auth = props.shell.authGeneration, rev = revision, seq = ++requestId;
  pending.comparing = true; error.value = ""; notice.value = "";
  try {
    const comparison = await props.invoke<Comparison>("compare_what_changed", { connectionId: id, ...range });
    if (!current(auth, rev) || seq !== requestId) return;
    expandedTimeline.value = false;
    result.value = comparison;
    notice.value = t('changes.comparisonSuccess', { date: fmtDate(comparison.generated_at_ms) });
    // Auxiliary calls have independent error channels and cannot invalidate comparison.
    void refreshMarkers(); void refreshCases();
  } catch (e) { if (current(auth, rev) && seq === requestId) error.value = String(e); }
  finally { if (current(auth, rev) && seq === requestId) pending.comparing = false; }
}
async function createMarker(): Promise<void> {
  const id = connectionId.value; if (id === null) return;
  const at = new Date(markerDraft.at).getTime();
  if (!Number.isFinite(at) || at <= 0 || !markerDraft.title.trim()) { markerError.value = t('changes.markerRequired'); return; }
  const auth = props.shell.authGeneration, rev = revision;
  pending.marker = true; markerError.value = "";
  try {
    await props.invoke("create_change_marker", { connectionId: id, occurredAtMs: at, kind: markerDraft.kind, title: markerDraft.title, note: markerDraft.note });
    if (!current(auth, rev)) return;
    markerDraft.title = ""; markerDraft.note = "";
    await refreshMarkers();
    if (result.value) await compare();
  } catch (e) { if (current(auth, rev)) markerError.value = String(e); }
  finally { if (props.shell.authGeneration === auth) pending.marker = false; }
}
async function deleteMarker(markerId: number): Promise<void> {
  const id = connectionId.value; if (id === null || !await confirmAction(t('changes.markerDelete', { id: String(markerId) }))) return;
  const auth = props.shell.authGeneration, rev = revision;
  pending.marker = true; markerError.value = "";
  try {
    await props.invoke("delete_change_marker", { connectionId: id, markerId });
    if (!current(auth, rev)) return;
    await refreshMarkers();
    if (result.value) await compare();
  } catch (e) { if (current(auth, rev)) markerError.value = String(e); }
  finally { if (props.shell.authGeneration === auth) pending.marker = false; }
}
async function save(): Promise<void> {
  const id = connectionId.value;
  if (!result.value || id === null || !activeConnection.value) return;
  let range: Record<string, number>;
  try { range = periods(); } catch (e) { error.value = String(e); return; }
  const auth = props.shell.authGeneration, rev = revision, seq = ++requestId;
  pending.saving = true; pending.comparing = false; error.value = "";
  try {
    const saved = await props.invoke<Saved>("save_what_changed_to_case", { connectionId: id, caseId: caseId.value, ...range });
    if (!current(auth, rev) || seq !== requestId) return;
    result.value = saved.comparison; // Display exactly the persisted server-side comparison.
    caseId.value = saved.case.id;
    notice.value = t('changes.savedCase', { caseId: String(saved.case.id), artifactId: String(saved.artifact.id) });
    void refreshCases(); void refreshMarkers();
  } catch (e) { if (current(auth, rev) && seq === requestId) error.value = t('changes.saveError', { error: String(e) }); }
  finally { if (props.shell.authGeneration === auth) pending.saving = false; }
}
const storageMetrics = [
  ["WAL bytes / hour", "wal_bytes_per_hour"], ["WAL records / hour", "wal_records_per_hour"],
  ["WAL buffers full / hour", "wal_buffers_full_per_hour"], ["WAL write ms / hour", "wal_write_time_ms_per_hour"],
  ["WAL sync ms / hour", "wal_sync_time_ms_per_hour"], ["Temp bytes / hour", "temp_bytes_per_hour"],
  ["Temp files / hour", "temp_files_per_hour"], ["Blocks read / hour", "database_blocks_read_per_hour"],
  ["Blocks hit / hour", "database_blocks_hit_per_hour"], ["Requested checkpoints / hour", "checkpoints_requested_per_hour"],
  ["Timed checkpoints / hour", "checkpoints_timed_per_hour"], ["DB size", "latest_database_size_bytes"],
  ["checkpoint_timeout", "checkpoint_timeout"], ["max_wal_size", "max_wal_size"],
] as const;
const sections = computed(() => result.value ? [
  { title: "PGSS", lines: result.value.pgss.limitations },
  { title: "Settings", lines: result.value.settings.limitations },
  { title: "Autovacuum", lines: result.value.autovacuum.limitations },
  { title: "XID", lines: result.value.xid.limitations },
  { title: "I/O & WAL", lines: result.value.storage.limitations },
  { title: "Index Health", lines: result.value.index_health.limitations },
].map(section => ({ ...section, lines: [...new Set(section.lines)] })).filter(section => section.lines.length > 0) : []);
const generalLimitations = computed(() => {
  if (!result.value) return [];
  const perSource = new Set(sections.value.flatMap(section => section.lines));
  return [...new Set(result.value.limitations)].filter(line => !perSource.has(line));
});
const storageHasData = computed(() => {
  if (!result.value) return false;
  return storageMetrics.some(([, key]) => result.value!.storage.baseline[key] != null || result.value!.storage.incident[key] != null);
});
function evidenceId(reference: string): string | null {
  if (!result.value) return null;
  if (reference.startsWith("pgss_statement:")) {
    const suffix = reference.slice("pgss_statement:".length);
    if (result.value.pgss.statement_changes.some(item => `${item.userid}:${item.dbid}:${item.queryid}:${item.toplevel}` === suffix)) return `pgss-${suffix}`;
  }
  const kinds: [string, string, number[]][] = [
    ["autovacuum_relation:", "autovacuum-", result.value.autovacuum.relation_changes.map(item => item.relation_oid)],
    ["xid_relation:", "xid-", result.value.xid.relation_changes.map(item => item.relation_oid)],
    ["relation:", "size-", result.value.storage.relation_size_changes.map(item => Number(item["relation_oid"]))],
    ["index:", "index-", result.value.index_health.changes.map(item => Number(item["index_oid"]))],
  ];
  for (const [prefix, idPrefix, ids] of kinds) {
    if (!reference.startsWith(prefix)) continue;
    const oid = Number(reference.slice(prefix.length));
    if (Number.isSafeInteger(oid) && ids.includes(oid)) return `${idPrefix}${oid}`;
  }
  return null;
}
async function showEvidence(ref: string): Promise<void> {
  const target = evidenceId(ref); if (!target) return;
  // Timeline links must remain reachable even when the row is hidden by the UI filter.
  if (onlyDifferences.value) onlyDifferences.value = false;
  await nextTick();
  const item = document.getElementById(target);
  if (!item) return;
  const detail = item.closest("details");
  if (detail) detail.open = true;
  await nextTick();
  item.scrollIntoView({ behavior: "smooth", block: "center" });
}
function timeMeaning(event: Event): string {
  if (event.source === "manual") return t('changes.sourceManual');
  if (event.source === "pg_settings") return t('changes.sourceSettings');
  if (["io_wal", "index_health"].includes(event.source)) return t('changes.sourceBoundary');
  return t('changes.sourceObservation');
}
function reset(): void {
  bootId++; invalidate(); connections.value = []; historyConnections.value = []; clearSnapshotSelection();
  connectionId.value = null; caseId.value = null; cases.value = []; markers.value = [];
  error.value = ""; markerError.value = ""; caseError.value = "";
  pending.loading = false; pending.comparing = false; pending.saving = false; pending.marker = false;
  defaults();
}
watch(() => props.shell.authGeneration, () => { reset(); if (props.shell.screen === "workspace") void refreshConnections(); });
let lastShortcutSerial = 0;
watch([() => props.quickSelection?.serial, () => props.visible, () => choices.value.length, () => (pending.comparing || pending.saving)], () => {
  const request = props.quickSelection;
  if (!request || !props.visible || request.serial <= lastShortcutSerial) return;
  if (!choices.value.some(item => item.id === request.connectionId)) return;
  if (pending.comparing || pending.saving) return;
  lastShortcutSerial = request.serial;
  if (connectionId.value !== request.connectionId) selectConnection(String(request.connectionId));
});
watch(() => props.visible, visible => { if (visible && props.shell.screen === "workspace") void refreshConnections(); });
onMounted(() => { defaults(); if (props.visible && props.shell.screen === "workspace") void refreshConnections(); });
</script>

<template>
<section class="wc" aria-label="What Changed">
  <header class="wc-header"><div><h1>What Changed?</h1><p>{{ t('changes.subtitle') }}</p></div></header>
  <details class="wc-panel wc-help" open>
    <summary>{{ t('changes.howTo') }}</summary>
    <ol>
      <li><strong>{{ t('changes.baselinePart') }}</strong> {{ t('changes.howBaseline') }}</li>
      <li><strong>{{ t('changes.incidentPart') }}</strong> {{ t('changes.howIncident') }}</li>
      <li>{{ t('changes.instructions') }}</li>
    </ol>
    <p class="wc-muted">{{ t('changes.example') }}</p>
  </details>
  <div class="wc-panel wc-time-setup">
    <div class="wc-setup-header">
      <label class="wc-connection">{{ t('changes.connection') }}
        <select :value="connectionId ?? ''" @change="selectConnection(($event.target as HTMLSelectElement).value)"><option value="">{{ t('changes.selectConnection') }}</option><option v-for="item in choices" :key="item.id" :value="item.id">{{ item.title }}</option></select>
      </label>
      <button class="secondary" type="button" :disabled="!suggestion || pending.comparing || pending.saving" @click="applySuggestion">{{ t('changes.autoPeriods') }}</button>
    </div>
    <p v-if="suggestion" class="wc-muted">{{ t('changes.sessionStats', { found: String(suggestion.sessions.length), used: String(suggestion.usedSessions), ready: String(suggestion.readySources) }) }}</p>
    <details v-if="suggestion" class="wc-sessions"><summary>{{ t('changes.sessionDates', { count: String(suggestion.sessions.length) }) }}</summary>
      <ul><li v-for="session in suggestion.sessions" :key="session.fromMs">{{ fmtDate(session.fromMs) }}<template v-if="session.toMs !== session.fromMs"> — {{ fmtDate(session.toMs) }}</template> · {{ t('changes.sessionSizes', { count: String(session.total), types: String(session.kinds.length) }) }}</li></ul>
    </details>
    <p v-else-if="snapshotsAvailable" class="wc-muted">{{ t('changes.noSessions') }}</p>
    <div class="wc-periods">
      <fieldset class="wc-period"><legend>1. Baseline <small>{{ t('changes.baselineBefore') }}</small></legend>
        <div class="wc-period-fields"><DateTimeField v-model="time.baselineFrom" :label="t('changes.from')" @commit="periodChanged" /><DateTimeField v-model="time.baselineTo" :label="t('changes.to')" @commit="periodChanged" /></div>
      </fieldset>
      <fieldset class="wc-period"><legend>2. Incident <small>{{ t('changes.incidentDuring') }}</small></legend>
        <div class="wc-period-fields"><DateTimeField v-model="time.incidentFrom" :label="t('changes.from')" @commit="periodChanged" /><DateTimeField v-model="time.incidentTo" :label="t('changes.to')" @commit="periodChanged" /></div>
      </fieldset>
    </div>
    <p v-if="periodError" class="wc-error" role="status">{{ periodError }}</p>
    <p v-if="futurePeriodWarning" class="wc-warning" role="status">{{ futurePeriodWarning }}</p>
    <div class="wc-actions"><button type="button" :disabled="connectionId === null || !!periodError || pending.comparing || pending.saving" @click="compare">{{ pending.comparing ? t('changes.comparing') : t('changes.compare') }}</button><span class="wc-muted">{{ t('changes.sqliteOnly') }}</span></div>
  </div>
  <SnapshotCoverage :invoke="invoke" :connection-id="connectionId" :range="coverageRange" :refresh-token="coverageToken" :auth-generation="shell.authGeneration" :visible="visible" @snapshots-loaded="snapshotsLoaded" @open-history="emit('openHistory')" @open-schedules="emit('openSchedules')" />
  <p v-if="error" class="wc-error" role="alert">{{ error }}</p><p v-if="notice" role="status">{{ notice }}</p>
  <div class="wc-panel wc-marker">
    <div><h2>{{ t('changes.markers') }}</h2><p>{{ t('changes.markersHintActual') }}</p></div>
    <form @submit.prevent="createMarker"><DateTimeField v-model="markerDraft.at" :label="t('changes.when')" /><label>{{ t('changes.type') }}<select v-model="markerDraft.kind"><option v-for="kind in ['deploy','postgresql_config','index','query','application','maintenance','other']" :key="kind" :value="kind">{{ kind }}</option></select></label><label>{{ t('changes.name') }}<input v-model="markerDraft.title" maxlength="160" required /></label><label>{{ t('changes.description') }}<input v-model="markerDraft.note" maxlength="4000" /></label><button type="submit" :disabled="!activeConnection || pending.marker">{{ t('changes.add') }}</button></form>
    <p v-if="markerError" role="alert" class="wc-error">{{ markerError }}</p>
    <p v-if="!markers.length" class="wc-muted">{{ t('changes.noMarkers') }}</p>
    <div v-for="marker in markers" :key="marker.id" class="wc-list-item"><span>{{ fmtDate(marker.occurred_at_ms) }} · {{ marker.kind }} · {{ marker.title }}<small v-if="marker.note"> — {{ marker.note }}</small></span><button type="button" class="secondary" :disabled="pending.marker" @click="deleteMarker(marker.id)">{{ t('changes.delete') }}</button></div>
  </div>
  <template v-if="result">
    <section class="wc-panel wc-save"><div><h2>{{ t('changes.result') }}</h2><p>{{ result.target.username }}@{{ result.target.host }}:{{ result.target.port }}/{{ result.target.database }} · {{ fmtDate(result.generated_at_ms) }}</p><p class="wc-muted">{{ t('changes.safetyPrivacy') }}</p></div><label>Investigation Case<select v-model.number="caseId" :disabled="!activeConnection"><option :value="null">{{ t('changes.caseNew') }}</option><option v-for="item in cases" :key="item.id" :value="item.id">#{{ item.id }} · {{ item.title }}</option></select></label><button type="button" :disabled="!activeConnection || pending.saving" @click="save">{{ pending.saving ? t('changes.saving') : t('changes.saveToCase') }}</button><p v-if="caseError" class="wc-error">{{ caseError }}</p><p class="wc-muted">{{ t('changes.caseSaveHint') }}</p></section>
    <div class="wc-panel"><h3>{{ t('changes.measured') }}</h3><p class="wc-muted">{{ t('changes.measurementExplanation') }}</p>
      <p>PGSS: {{ fmtDuration(result.pgss.baseline_quality.safe_elapsed_seconds) }} / {{ fmtDuration(result.pgss.incident_quality.safe_elapsed_seconds) }}</p>
      <p>Blocks hit: {{ fmtDuration(result.storage.baseline_quality.blocks_hit_observed_seconds) }} / {{ fmtDuration(result.storage.incident_quality.blocks_hit_observed_seconds) }}</p>
      <p>WAL bytes: {{ fmtDuration(result.storage.baseline_quality.wal_bytes_observed_seconds) }} / {{ fmtDuration(result.storage.incident_quality.wal_bytes_observed_seconds) }}</p>
      <p>Timed checkpoints: {{ fmtDuration(result.storage.baseline_quality.checkpoints_timed_observed_seconds) }} / {{ fmtDuration(result.storage.incident_quality.checkpoints_timed_observed_seconds) }}</p>
      <small>{{ t('changes.valueLabel') }}</small>
    </div>
    <div class="wc-table-controls wc-panel">
      <label><input v-model="onlyDifferences" type="checkbox" /> {{ t('changes.differenceOnly') }}</label>
      <span class="wc-muted">{{ t('changes.fullListHint') }}</span>
    </div>
    <div class="wc-metrics"><article><span>PGSS baseline / incident</span><strong>{{ result.pgss.baseline_quality.state }} / {{ result.pgss.incident_quality.state }}</strong><small>{{ result.pgss.statement_changes.length }} SQL</small></article><article><span>{{ t('changes.settingsTitle') }}</span><strong>{{ result.settings.state }}</strong><small>{{ result.settings.changes.length }} {{ t('changes.countChanges') }}</small></article><article><span>Autovacuum</span><strong>{{ result.autovacuum.baseline_quality.state }} / {{ result.autovacuum.incident_quality.state }}</strong><small>{{ result.autovacuum.relation_changes.length }} {{ t('changes.countTables') }}</small></article><article><span>XID</span><strong>{{ fmt(result.xid.baseline['state']) }} / {{ fmt(result.xid.incident['state']) }}</strong><small>{{ result.xid.relation_changes.length }} {{ t('changes.countTables') }}</small></article><article><span>Storage</span><strong>{{ result.storage.baseline_quality.state }} / {{ result.storage.incident_quality.state }}</strong><small>{{ result.storage.relation_size_changes.length }} {{ t('changes.countSizes') }}</small></article><article><span>Index Health</span><strong>{{ result.index_health.changes.length }}</strong><small>{{ t('changes.diagnosticCount', { count: String(result.index_health.usage_changes?.length ?? 0) }) }}</small></article></div>
    <section class="wc-panel">
      <div class="wc-timeline-heading">
        <h2>{{ t('changes.timelineCounts', { events: String(result.timeline.length), groups: String(timelineGroups.length) }) }}</h2>
        <button v-if="collapsibleTimelineGroups" class="secondary" type="button" @click="expandedTimeline = !expandedTimeline">{{ expandedTimeline ? t('changes.groupCollapse') : t('changes.groupExpand') }}</button>
      </div>
      <p class="wc-muted">{{ t('changes.timelineDescription') }}</p>
      <p v-if="!timelineGroups.length" class="wc-muted">{{ t('changes.noEvents') }}</p>
      <details v-for="group in timelineGroups" :key="group.key" class="wc-timeline-group" :open="group.events.length === 1 || expandedTimeline">
        <summary>
          <span class="wc-timeline-summary-time">{{ fmtDate(group.occurred_at_ms) }} · {{ group.phase }} · {{ timelineSourceLabel(group.source) }}</span>
          <strong>{{ group.events.length === 1 ? group.events[0]?.title : t('changes.showGroupDetails', { count: String(group.events.length) }) }}</strong>
        </summary>
        <article v-for="(event, i) in group.events" :key="i" class="wc-list-item wc-event">
          <h3 v-if="group.events.length > 1">{{ event.title }}</h3>
          <p>{{ event.detail }}</p>
          <small>{{ event.evidence_level }} · {{ timeMeaning(event) }}</small>
          <div v-if="event.evidence_refs.length" class="wc-refs">
            <template v-for="ref in event.evidence_refs" :key="ref">
              <button v-if="evidenceId(ref)" class="secondary" type="button" @click="showEvidence(ref)">{{ ref }}</button>
              <code v-else>{{ ref }}</code>
            </template>
          </div>
        </article>
      </details>
    </section>
    <details class="wc-panel"><summary>{{ t('changes.pgssRows', { shown: String(visiblePgss.length), total: String(result.pgss.statement_changes.length) }) }}</summary>
      <div class="wc-scroll"><table><thead><tr>
        <th><button class="wc-sort" @click="toggleSort('pgss', 'queryid')">Statement{{ sortArrow('pgss', 'queryid') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('pgss', 'state')">{{ t('changes.sqlState') }}{{ sortArrow('pgss', 'state') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('pgss', 'workload_delta')">Δ execution workload{{ sortArrow('pgss', 'workload_delta') }}</button><button class="wc-sort" @click="toggleSort('pgss', 'calls_per_hour')">Calls/hour{{ sortArrow('pgss', 'calls_per_hour') }}</button></th>
        <th>Evidence</th></tr></thead><tbody>
        <tr v-for="s in visiblePgss" :id="`pgss-${s.userid}:${s.dbid}:${s.queryid}:${s.toplevel}`" :key="`${s.userid}:${s.dbid}:${s.queryid}:${s.toplevel}`">
          <td>{{ s.queryid }}<small>{{ s.database }} / {{ s.role_name }}</small></td><td>{{ s.state }}</td>
          <td><div v-for="metric in visibleStatementMetrics(s)" :key="metric.id">{{ metric.label }}: {{ fmtMetricPair(metric.baseline_value, metric.incident_value) }} {{ metric.unit }} ({{ metric.direction }})</div><span v-if="!visibleStatementMetrics(s).length" class="wc-muted">{{ t('whatFinish.noDifference') }}{{ t('vueRemaining.comparableMetrics') }}</span></td><td>{{ s.evidence_level }}</td>
        </tr></tbody></table></div>
      <p v-if="!visiblePgss.length" class="wc-muted">{{ t('changes.noSimilarSql') }}</p>
    </details>
    <details class="wc-panel"><summary>Settings · {{ visibleSettings.length }}</summary>
      <p class="wc-muted">{{ t('changes.settingsHintFull') }}</p>
      <div class="wc-scroll"><table><thead><tr>
        <th><button class="wc-sort" @click="toggleSort('settings', 'name')">{{ t('changes.settingParameter') }}{{ sortArrow('settings', 'name') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('settings', 'reason')">{{ t('changes.settingReason') }}{{ sortArrow('settings', 'reason') }}</button></th>
        <th>{{ t('changes.before') }}</th><th>{{ t('changes.after') }}</th><th><button class="wc-sort" @click="toggleSort('settings', 'date')">{{ t('changes.settingDetected') }}{{ sortArrow('settings', 'date') }}</button></th>
        </tr></thead><tbody><tr v-for="item in visibleSettings" :key="`${item.change['name']}:${item.first_observed_at_ms}`"><td>{{ fmt(item.change['name']) }}</td><td>{{ settingReasons(item.change) }}<details><summary>{{ t('changes.settingFields') }}</summary><p v-if="item.change['value_changed']">{{ t('changes.unit') }}: {{ fmt(item.change['older_unit']) }} → {{ fmt(item.change['newer_unit']) }}</p><p v-if="item.change['reset_value_changed']">reset_val: {{ fmt(item.change['older_reset_value']) }} → {{ fmt(item.change['newer_reset_value']) }}</p><p v-if="item.change['source_changed']">{{ t('changes.source') }}: {{ fmtSettingSource(item.change, 'older') }} → {{ fmtSettingSource(item.change, 'newer') }}</p><p v-if="item.change['pending_restart_changed']">pending_restart: {{ fmt(item.change['older_pending_restart']) }} → {{ fmt(item.change['newer_pending_restart']) }}</p></details></td><td>{{ fmt(item.change['older_value']) }}</td><td>{{ fmt(item.change['newer_value']) }}</td><td>{{ fmtDate(item.first_observed_at_ms) }}</td></tr></tbody></table></div>
    </details>
    <details class="wc-panel"><summary>{{ t('changes.autovacuumRows', { shown: String(visibleAutovacuum.length), total: String(result.autovacuum.relation_changes.length) }) }}</summary>
      <p class="wc-muted">{{ t('changes.autovacuumHint') }}</p>
      <div class="wc-scroll"><table><thead><tr>
        <th><button class="wc-sort" @click="toggleSort('autovacuum', 'name')">{{ t('changes.table') }}{{ sortArrow('autovacuum', 'name') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('autovacuum', 'dml_incident')">{{ t('whatFinish.dml') }}{{ sortArrow('autovacuum', 'dml_incident') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('autovacuum', 'vacuum_incident')">VACUUM / ANALYZE{{ sortArrow('autovacuum', 'vacuum_incident') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('autovacuum', 'dead_delta')">Δ Dead tuples{{ sortArrow('autovacuum', 'dead_delta') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('autovacuum', 'policy')">{{ t('changes.policy') }}{{ sortArrow('autovacuum', 'policy') }}</button></th>
      </tr></thead><tbody><tr v-for="item in visibleAutovacuum" :id="`autovacuum-${item.relation_oid}`" :key="item.relation_oid">
        <td>{{ item.relation_name }}<small>{{ item.state }} · {{ item.evidence_level }}</small></td>
        <td><div v-for="line in avLines(item, [['INSERT', 'inserts_per_hour'], ['UPDATE', 'updates_per_hour'], ['DELETE', 'deletes_per_hour']])" :key="line">{{ line }}</div><span v-if="!avLines(item, [['INSERT', 'inserts_per_hour'], ['UPDATE', 'updates_per_hour'], ['DELETE', 'deletes_per_hour']]).length">—</span></td>
        <td><div v-for="line in avLines(item, [['VACUUM', 'vacuum_cycles'], ['ANALYZE', 'analyze_cycles']])" :key="line">{{ line }}</div><span v-if="!avLines(item, [['VACUUM', 'vacuum_cycles'], ['ANALYZE', 'analyze_cycles']]).length">—</span></td>
        <td>{{ fieldDiff(item.baseline, item.incident, 'latest_dead_tuples') }}</td><td>{{ item.policy_changes?.join('; ') || '—' }}</td>
      </tr></tbody></table></div>
      <p v-if="!visibleAutovacuum.length" class="wc-muted">{{ t('changes.autovacuumNoDiff') }}</p>
    </details>
    <details class="wc-panel"><summary>{{ t('changes.xidRows', { shown: String(visibleXid.length), total: String(result.xid.relation_changes.length) }) }}</summary>
      <p class="wc-muted">{{ t('changes.xidHintFull') }}</p>
      <div class="wc-scroll"><table><thead><tr>
        <th><button class="wc-sort" @click="toggleSort('xid', 'name')">{{ t('changes.table') }}{{ sortArrow('xid', 'name') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('xid', 'age_delta')">Age / Δ Age{{ sortArrow('xid', 'age_delta') }}</button></th>
        <th><button class="wc-sort" @click="toggleSort('xid', 'pct')">Freeze max %{{ sortArrow('xid', 'pct') }}</button></th>
        <th>Freeze threshold</th><th>{{ t('changes.context') }}</th></tr></thead><tbody><tr v-for="item in visibleXid" :id="`xid-${item.relation_oid}`" :key="item.relation_oid"><td>{{ item.relation_name }}</td><td>{{ fieldDiff(item.baseline, item.incident, 'latest_xid_age') }}</td><td>{{ fieldDiff(item.baseline, item.incident, 'latest_pct_of_freeze_max') }}</td><td>{{ fieldDiff(item.baseline, item.incident, 'latest_effective_freeze_max_age') }}</td><td>{{ item.state }} · {{ item.evidence_level }}</td></tr></tbody></table></div>
    </details>
    <details class="wc-panel"><summary>{{ t('changes.storageRows', { shown: String(visibleStorage.length), total: String(storageMetrics.length) }) }}</summary>
      <p v-if="!storageHasData" class="wc-muted">{{ t('changes.noStats') }}</p>
      <div v-else class="wc-scroll"><table><thead><tr><th><button class="wc-sort" @click="toggleSort('storage', 'name')">{{ t('changes.storageMetric') }}{{ sortArrow('storage', 'name') }}</button></th><th>{{ t('changes.baselinePart') }}</th><th><button class="wc-sort" @click="toggleSort('storage', 'incident')">Incident{{ sortArrow('storage', 'incident') }}</button></th><th><button class="wc-sort" @click="toggleSort('storage', 'delta')">Δ{{ sortArrow('storage', 'delta') }}</button></th></tr></thead><tbody><tr v-for="[name, key] in visibleStorage" :key="key"><td>{{ name }}</td><td>{{ fmt(result.storage.baseline[key]) }}</td><td>{{ fmt(result.storage.incident[key]) }}</td><td>{{ fmt(delta(result.storage.baseline[key], result.storage.incident[key])) }}</td></tr></tbody></table></div>
      <p v-if="storageHasData && !visibleStorage.length" class="wc-muted">{{ t('changes.storageNoDiff') }}</p>
    </details>
    <details class="wc-panel"><summary>Relation sizes · {{ visibleSizes.length }}</summary>
      <div class="wc-scroll"><table><thead><tr><th><button class="wc-sort" @click="toggleSort('sizes', 'name')">Relation{{ sortArrow('sizes', 'name') }}</button></th><th>Baseline bytes</th><th><button class="wc-sort" @click="toggleSort('sizes', 'size')">Incident bytes{{ sortArrow('sizes', 'size') }}</button></th><th><button class="wc-sort" @click="toggleSort('sizes', 'delta')">Δ bytes{{ sortArrow('sizes', 'delta') }}</button></th><th>{{ t('changes.sortingState') }}</th></tr></thead><tbody><tr v-for="item in visibleSizes" :id="`size-${item['relation_oid']}`" :key="String(item['relation_oid'])"><td>{{ fmt(item['relation_name']) }}</td><td>{{ fmt(item['baseline_total_size_bytes']) }}</td><td>{{ fmt(item['incident_total_size_bytes']) }}</td><td>{{ fmt(item['total_size_delta_bytes']) }}</td><td>{{ fmt(item['state']) }}</td></tr></tbody></table></div>
    </details>
    <details class="wc-panel"><summary>{{ t('changes.indexUsageTitle', { count: String(visibleUsage.length) }) }}</summary>
      <p class="wc-muted">{{ t('changes.indexUsageExplanation') }}</p>
      <p v-if="!result.index_health.stats_period_continuous" class="wc-warning">{{ t('changes.indexStatsReset') }}</p>
      <div class="wc-scroll"><table><thead><tr><th><button class="wc-sort" @click="toggleSort('usage', 'name')">{{ t('changes.index') }}{{ sortArrow('usage', 'name') }}</button></th><th><button class="wc-sort" @click="toggleSort('usage', 'scans')">Δ idx_scan{{ sortArrow('usage', 'scans') }}</button></th><th><button class="wc-sort" @click="toggleSort('usage', 'dml')">{{ t('vueRemaining.tableDml') }}{{ sortArrow('usage', 'dml') }}</button></th><th>{{ t('changes.context') }}</th></tr></thead><tbody><tr v-for="item in visibleUsage" :key="String(item['index_oid'])"><td>{{ fmt(item['index_name']) }}<small>{{ fmt(item['relation_name']) }}</small></td><td>{{ fmt(item['scan_delta']) }}</td><td>{{ fmt(item['relation_dml_delta']) }}</td><td>{{ fmt(item['state']) }}</td></tr></tbody></table></div>
      <p v-if="!visibleUsage.length" class="wc-muted">{{ t('changes.indexNoUsage') }}</p>
    </details>
    <details class="wc-panel"><summary>{{ t('changes.indexHealthTitle', { count: String(visibleIndex.length) }) }}</summary>
      <p class="wc-muted">{{ t('changes.indexHealthHint') }}</p>
      <div class="wc-scroll"><table><thead><tr><th><button class="wc-sort" @click="toggleSort('index', 'name')">Index{{ sortArrow('index', 'name') }}</button></th><th>{{ t('changes.classification') }}</th><th><button class="wc-sort" @click="toggleSort('index', 'scans')">Δ scans{{ sortArrow('index', 'scans') }}</button></th><th><button class="wc-sort" @click="toggleSort('index', 'size')">Δ bytes{{ sortArrow('index', 'size') }}</button></th><th><button class="wc-sort" @click="toggleSort('index', 'state')">{{ t('changes.sqlState') }}{{ sortArrow('index', 'state') }}</button></th></tr></thead><tbody><tr v-for="item in visibleIndex" :id="`index-${item['index_oid']}`" :key="String(item['index_oid'])"><td>{{ fmt(item['index_name']) }}<small>{{ fmt(item['relation_name']) }}</small></td><td>{{ fmt(item['baseline_classifications']) }} → {{ fmt(item['incident_classifications']) }}</td><td>{{ fmt(item['scan_delta']) }}</td><td>{{ fmt(item['size_delta_bytes']) }}</td><td>{{ fmt(item['state']) }}</td></tr></tbody></table></div>
    </details>
    <details class="wc-panel"><summary>{{ t('changes.limitsTitle', { count: String(sections.reduce((total, item) => total + item.lines.length, 0) + generalLimitations.length) }) }}</summary><div v-for="section in sections" :key="section.title"><h3>{{ section.title }}</h3><ul><li v-for="line in section.lines" :key="line">{{ line }}</li></ul></div><template v-if="generalLimitations.length"><h3>{{ t('changes.commonLimitations') }}</h3><ul><li v-for="line in generalLimitations" :key="line">{{ line }}</li></ul></template></details>
  </template>
  <p v-else-if="!pending.comparing" class="wc-muted">{{ t('changes.emptyResult') }}</p>
</section>
</template>
<style scoped>
.wc{display:grid;gap:1rem;min-width:0}.wc-header{display:flex;justify-content:space-between;align-items:start;gap:1rem;flex-wrap:wrap}.wc h1,.wc h2,.wc h3{margin:.15rem 0 .35rem}.wc h2{font-size:1.1rem}.wc h3{font-size:.97rem}.wc p{margin:.3rem 0}.wc-header p,.wc-muted,.wc small,.wc-marker p{color:var(--text-secondary)}.wc-panel{min-width:0;padding:1rem;border:1px solid var(--border-strong);border-radius:.7rem;background:var(--surface)}.wc select,.wc input{min-width:0;max-width:100%;padding:.55rem;background:var(--surface-2);border:1px solid var(--border-strong);border-radius:.45rem;color:var(--text)}.wc button{width:auto}.wc-marker form{display:flex;flex-wrap:wrap;align-items:end;gap:.6rem;max-width:none}.wc-marker form label{flex:1 1 160px}.wc-marker form button{flex:0 0 auto}.wc-list-item{display:flex;justify-content:space-between;align-items:start;gap:1rem;padding:.6rem 0;border-bottom:1px solid var(--border)}.wc-event{display:block}.wc-event p{white-space:pre-wrap}.wc-refs{display:flex;flex-wrap:wrap;gap:.45rem;padding-top:.5rem}.wc-refs code{font-size:.8rem;padding:.35rem;background:var(--surface-2);border-radius:.25rem}.wc-refs button{font-size:.75rem;padding:.35rem .55rem}.wc-error{color:var(--error)}.wc-save{display:flex;align-items:end;gap:1rem;flex-wrap:wrap}.wc-save>div{flex:2 1 260px}.wc-save label{flex:1 1 240px}.wc-metrics{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:.7rem}.wc-metrics article{background:var(--surface-2);border-radius:.55rem;padding:.8rem;display:grid;gap:.3rem}.wc-metrics span,.wc-metrics small{color:var(--text-secondary)}.wc-metrics strong{overflow-wrap:anywhere}.wc details summary{cursor:pointer;font-weight:600}.wc-scroll{overflow-x:auto;margin-top:.6rem;max-height:520px}.wc table{width:100%;border-collapse:collapse;text-align:left;font-size:.88rem}.wc th,.wc td{padding:.55rem;vertical-align:top;border-bottom:1px solid var(--border-strong)}.wc td small{display:block;font-size:.78rem;margin-top:.25rem}.wc tr:target{background:var(--surface-2)}

.wc-time-setup{display:grid;gap:.85rem}.wc-setup-header{display:flex;gap:.8rem;flex-wrap:wrap;align-items:end}.wc-connection{flex:1 1 260px;display:grid;gap:.3rem}.wc-periods{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:.7rem}.wc-period{min-width:0;margin:0;border:1px solid var(--border-strong);border-radius:.5rem;padding:.7rem}.wc-period legend{font-weight:650;padding:0 .35rem}.wc-period legend small{font-weight:400;color:var(--text-secondary);margin-left:.4rem}.wc-period-fields{display:grid;grid-template-columns:minmax(0,1fr);gap:.85rem}.wc-help ol{margin:.65rem 0;padding-left:1.4rem;line-height:1.55}.wc-help summary,.wc-sessions summary{cursor:pointer;font-weight:650}.wc-sessions ul{margin:.55rem 0;padding-left:1.3rem;color:var(--text-secondary)}.wc-actions{display:flex;align-items:center;gap:.7rem;flex-wrap:wrap}.wc-marker form>*.dt-field{flex:1 1 260px}
@media(max-width:900px){.wc-periods{grid-template-columns:minmax(0,1fr)}}
.wc-table-controls{display:flex;flex-wrap:wrap;align-items:center;gap:.8rem}.wc-table-controls label{display:flex;gap:.5rem;align-items:center;white-space:nowrap;font-weight:600}.wc-table-controls input{width:auto}.wc-table-controls .wc-muted{flex:1 1 360px}.wc th .wc-sort{border:0;background:none;color:inherit;cursor:pointer;font:inherit;font-weight:650;padding:0;text-align:left;white-space:nowrap;margin:0 .8rem 0 0}.wc th .wc-sort:hover{text-decoration:underline}.wc-timeline-heading{display:flex;align-items:center;justify-content:space-between;gap:.75rem;flex-wrap:wrap}.wc-timeline-group{min-width:0;padding:.65rem 0;border-bottom:1px solid var(--border)}.wc-timeline-group>summary{display:grid;gap:.25rem;overflow-wrap:anywhere}.wc-timeline-group>summary strong{font-size:.95rem}.wc-timeline-summary-time{font-size:.78rem;color:var(--text-secondary);font-weight:400}.wc-timeline-group .wc-list-item:last-child{border-bottom:0}.wc-timeline-group .wc-event{margin-left:.65rem;padding-left:.7rem;border-left:2px solid var(--border-strong)}.wc-warning{color:var(--warning,var(--text));border-left:3px solid var(--warning,#e4a04c);padding:.65rem .85rem;background:var(--surface-2)}
</style>
