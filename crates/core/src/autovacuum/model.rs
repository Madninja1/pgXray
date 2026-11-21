use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct AutovacuumOverview {
    pub measured_at: String,
    pub database: String,
    pub database_oid: u32,
    pub username: String,
    pub server_version: String,
    pub server_version_num: i32,
    pub stats_reset: Option<String>,
    pub autovacuum: bool,
    pub track_counts: bool,
    pub autovacuum_max_workers: i32,
    pub autovacuum_naptime: String,
    pub autovacuum_vacuum_threshold: i64,
    pub autovacuum_vacuum_scale_factor: f64,
    #[serde(default)]
    pub autovacuum_vacuum_max_threshold: Option<i64>,
    pub autovacuum_vacuum_insert_threshold: i64,
    pub autovacuum_vacuum_insert_scale_factor: f64,
    pub autovacuum_analyze_threshold: i64,
    pub autovacuum_analyze_scale_factor: f64,
    pub autovacuum_vacuum_cost_limit: i64,
    pub autovacuum_vacuum_cost_delay: String,
    pub vacuum_cost_limit: i64,
    pub block_size: i64,
    pub log_autovacuum_min_duration: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AutovacuumToast {
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub total_size_bytes: i64,
    pub estimated_rows: i64,
    pub live_tuples: i64,
    pub dead_tuples: i64,
    pub inserts_since_vacuum: i64,
    pub last_autovacuum: Option<String>,
    pub autovacuum_count: i64,
    pub xid_age: i64,
    pub reloptions: Option<Vec<String>>,
    pub autovacuum_enabled: bool,
    pub effective_vacuum_threshold: f64,
    pub effective_vacuum_scale_factor: f64,
    #[serde(default)]
    pub effective_vacuum_max_threshold: Option<i64>,
    // PG18+ uses a fraction of unfrozen pages for insert-triggered VACUUM.
    // Missing fields in older snapshots are treated as unknown, not 1.0.
    #[serde(default)]
    pub server_version_num: i32,
    #[serde(default)]
    pub reltuples_estimate: Option<f64>,
    #[serde(default)]
    pub insert_unfrozen_fraction: Option<f64>,
    pub effective_insert_threshold: f64,
    pub effective_insert_scale_factor: f64,
    pub effective_freeze_max_age: i64,
    pub dead_tuple_trigger: i64,
    pub insert_trigger: Option<i64>,
    pub dead_trigger_used_pct: Option<f64>,
    pub insert_trigger_used_pct: Option<f64>,
    pub dead_trigger_reached: bool,
    pub insert_trigger_reached: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AutovacuumTable {
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub relation_type: String,
    pub parent_oid: Option<u32>,
    pub parent_schema: Option<String>,
    pub parent_relation: Option<String>,
    pub heap_size_bytes: i64,
    pub indexes_size_bytes: i64,
    pub total_size_bytes: i64,
    pub index_count: i64,
    pub estimated_rows: i64,
    pub live_tuples: i64,
    pub dead_tuples: i64,
    pub dead_tuple_pct: Option<f64>,
    pub tuples_inserted: i64,
    pub tuples_updated: i64,
    pub tuples_deleted: i64,
    pub hot_updates: i64,
    pub hot_update_pct: Option<f64>,
    pub inserts_since_vacuum: i64,
    pub modifications_since_analyze: i64,
    pub last_vacuum: Option<String>,
    pub last_autovacuum: Option<String>,
    pub last_analyze: Option<String>,
    pub last_autoanalyze: Option<String>,
    pub vacuum_count: i64,
    pub autovacuum_count: i64,
    pub analyze_count: i64,
    pub autoanalyze_count: i64,
    pub xid_age: Option<i64>,
    pub reloptions: Option<Vec<String>>,
    pub toast: Option<AutovacuumToast>,
    pub autovacuum_enabled: bool,
    pub autovacuum_applicable: bool,
    pub effective_vacuum_threshold: f64,
    pub effective_vacuum_scale_factor: f64,
    #[serde(default)]
    pub effective_vacuum_max_threshold: Option<i64>,
    // PG18+ uses a fraction of unfrozen pages for insert-triggered VACUUM.
    // Missing fields in older snapshots are treated as unknown, not 1.0.
    #[serde(default)]
    pub server_version_num: i32,
    #[serde(default)]
    pub reltuples_estimate: Option<f64>,
    #[serde(default)]
    pub insert_unfrozen_fraction: Option<f64>,
    pub effective_insert_threshold: f64,
    pub effective_insert_scale_factor: f64,
    pub effective_analyze_threshold: f64,
    pub effective_analyze_scale_factor: f64,
    pub effective_vacuum_cost_limit: i64,
    pub effective_vacuum_cost_delay_ms: f64,
    pub effective_freeze_max_age: i64,
    pub dead_tuple_trigger: i64,
    pub insert_trigger: Option<i64>,
    pub analyze_trigger: i64,
    pub dead_trigger_used_pct: Option<f64>,
    pub insert_trigger_used_pct: Option<f64>,
    pub analyze_trigger_used_pct: Option<f64>,
    pub dead_trigger_reached: bool,
    pub insert_trigger_reached: bool,
    pub analyze_trigger_reached: bool,
    pub workload_profile: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutovacuumVacuumProgress {
    pub pid: i32,
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub phase: String,
    pub heap_blks_total: i64,
    pub heap_blks_scanned: i64,
    pub heap_blks_vacuumed: i64,
    pub scan_pct: Option<f64>,
    pub vacuum_pct: Option<f64>,
    pub index_vacuum_count: i64,
    // PostgreSQL <=16: counts of dead tuples; unavailable on >=17.
    pub max_dead_tuples: Option<i64>,
    pub num_dead_tuples: Option<i64>,
    // PostgreSQL >=17: bytes and dead item identifiers, different units.
    #[serde(default)]
    pub max_dead_tuple_bytes: Option<i64>,
    #[serde(default)]
    pub dead_tuple_bytes: Option<i64>,
    #[serde(default)]
    pub num_dead_item_ids: Option<i64>,
    pub backend_type: Option<String>,
    pub is_autovacuum: bool,
    pub query_age_seconds: Option<i64>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutovacuumAnalyzeProgress {
    pub pid: i32,
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub phase: String,
    pub sample_blks_total: i64,
    pub sample_blks_scanned: i64,
    pub sample_pct: Option<f64>,
    pub ext_stats_total: i64,
    pub ext_stats_computed: i64,
    pub child_tables_total: i64,
    pub child_tables_done: i64,
    pub backend_type: Option<String>,
    pub is_autoanalyze: bool,
    pub query_age_seconds: Option<i64>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutovacuumIndex {
    pub index_oid: u32,
    pub index_name: String,
    pub is_primary: bool,
    pub is_unique: bool,
    pub has_expression: bool,
    pub is_partial: bool,
    pub index_size_bytes: i64,
    pub indexed_columns: Vec<String>,
    #[serde(default)]
    pub included_columns: Vec<String>,
    pub predicate: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutovacuumUpdateStatement {
    pub queryid: Option<String>,
    pub userid: Option<i64>,
    pub dbid: Option<i64>,
    pub toplevel: Option<bool>,
    pub relation_oid: u32,
    /// parsed_qualified or heuristic_unqualified; never an OID-verified dependency.
    pub evidence_level: String,
    pub evidence_note: String,
    pub calls: i64,
    pub total_exec_time_ms: f64,
    pub mean_exec_time_ms: f64,
    pub rows: i64,
    pub query: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutovacuumSection<T> {
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> AutovacuumSection<T> {
    pub(crate) fn success(data: T) -> Self {
        Self {
            data: Some(data),
            error: None,
        }
    }

    pub(crate) fn failure(error: impl Into<String>) -> Self {
        Self {
            data: None,
            error: Some(error.into()),
        }
    }

    // Keep the existing JSON shape for saved snapshots and Tauri clients.
    // Deserializing historical data may still yield an invalid pair; check
    // it before consuming data from untrusted or externally generated JSON.
    pub fn is_valid(&self) -> bool {
        self.data.is_some() != self.error.is_some()
    }
}

// A finding is not a diagnosis: "attention" asks the user to investigate,
// while "warning" indicates a configuration state requiring explicit review.
// Historical SQLite snapshots do not have this property; default to "info".
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutovacuumInterpretationSeverity {
    #[default]
    Info,
    Attention,
    Warning,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutovacuumInterpretationItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub severity: AutovacuumInterpretationSeverity,
    pub observed: Vec<String>,
    pub meaning: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutovacuumInterpretation {
    pub items: Vec<AutovacuumInterpretationItem>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutovacuumSnapshot {
    pub overview: AutovacuumOverview,
    pub tables: AutovacuumSection<Vec<AutovacuumTable>>,
    pub vacuums: AutovacuumSection<Vec<AutovacuumVacuumProgress>>,
    pub analyzes: AutovacuumSection<Vec<AutovacuumAnalyzeProgress>>,
    pub interpretation: AutovacuumInterpretation,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumScaleCandidate {
    pub scale_factor: f64,
    pub resulting_dead_tuple_trigger: i64,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumPolicySimulation {
    pub relation_oid: u32,
    pub relation_name: String,
    pub estimated_rows: i64,
    pub current_threshold: f64,
    pub current_scale_factor: f64,
    pub current_dead_tuple_trigger: i64,
    pub target_dead_tuples: i64,
    pub candidate_scale_factor: Option<f64>,
    pub resulting_dead_tuple_trigger: Option<i64>,
    pub fixed_scale_candidates: Vec<AutovacuumScaleCandidate>,
    pub notes: Vec<String>,
}

// Effective settings are part of the evidence period. Unchanged statistics
// counters alone do not prove that the same maintenance policy was in force.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AutovacuumMaintenancePolicySignature {
    pub autovacuum_enabled: bool,
    pub reloptions: Option<Vec<String>>,
    pub vacuum_threshold: f64,
    pub vacuum_scale_factor: f64,
    pub vacuum_max_threshold: Option<i64>,
    pub insert_threshold: f64,
    pub insert_scale_factor: f64,
    pub analyze_threshold: f64,
    pub analyze_scale_factor: f64,
    pub vacuum_cost_limit: i64,
    pub vacuum_cost_delay_ms: f64,
    pub toast: Option<AutovacuumToastPolicySignature>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AutovacuumToastPolicySignature {
    pub relation_oid: u32,
    pub autovacuum_enabled: bool,
    pub reloptions: Option<Vec<String>>,
    pub vacuum_threshold: f64,
    pub vacuum_scale_factor: f64,
    pub vacuum_max_threshold: Option<i64>,
    pub insert_threshold: f64,
    pub insert_scale_factor: f64,
}

impl AutovacuumMaintenancePolicySignature {
    pub(crate) fn from_table(table: &AutovacuumTable) -> Self {
        Self {
            autovacuum_enabled: table.autovacuum_enabled,
            reloptions: table.reloptions.clone(),
            vacuum_threshold: table.effective_vacuum_threshold,
            vacuum_scale_factor: table.effective_vacuum_scale_factor,
            vacuum_max_threshold: table.effective_vacuum_max_threshold,
            insert_threshold: table.effective_insert_threshold,
            insert_scale_factor: table.effective_insert_scale_factor,
            analyze_threshold: table.effective_analyze_threshold,
            analyze_scale_factor: table.effective_analyze_scale_factor,
            vacuum_cost_limit: table.effective_vacuum_cost_limit,
            vacuum_cost_delay_ms: table.effective_vacuum_cost_delay_ms,
            toast: table
                .toast
                .as_ref()
                .map(|toast| AutovacuumToastPolicySignature {
                    relation_oid: toast.relation_oid,
                    autovacuum_enabled: toast.autovacuum_enabled,
                    reloptions: toast.reloptions.clone(),
                    vacuum_threshold: toast.effective_vacuum_threshold,
                    vacuum_scale_factor: toast.effective_vacuum_scale_factor,
                    vacuum_max_threshold: toast.effective_vacuum_max_threshold,
                    insert_threshold: toast.effective_insert_threshold,
                    insert_scale_factor: toast.effective_insert_scale_factor,
                }),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AutovacuumMaintenanceHistorySample {
    pub collection_started_at_ms: i64,
    pub stats_reset: Option<String>,
    pub policy: Option<AutovacuumMaintenancePolicySignature>,
    pub dead_tuples: i64,
    pub inserts_since_vacuum: i64,
    pub modifications_since_analyze: i64,
    pub tuples_inserted: i64,
    pub tuples_updated: i64,
    pub tuples_deleted: i64,
    pub hot_updates: i64,
    pub vacuum_count: i64,
    pub autovacuum_count: i64,
    pub analyze_count: i64,
    pub autoanalyze_count: i64,
    pub dead_trigger_reached: bool,
    pub insert_trigger_reached: bool,
    pub analyze_trigger_reached: bool,
    pub toast_autovacuum_count: Option<i64>,
    pub toast_dead_tuples: Option<i64>,
    pub toast_inserts_since_vacuum: Option<i64>,
    pub toast_dead_trigger_reached: Option<bool>,
    pub toast_insert_trigger_reached: Option<bool>,
}

pub(crate) fn maintenance_stats_pair_continuous(
    older: &AutovacuumMaintenanceHistorySample,
    newer: &AutovacuumMaintenanceHistorySample,
) -> bool {
    // Missing reset timestamps cannot establish a continuous stats period,
    // even if cumulative counters happen to remain monotonic.
    match (&older.stats_reset, &newer.stats_reset) {
        (Some(older_reset), Some(newer_reset)) if older_reset == newer_reset => {}
        _ => return false,
    }
    // A missing or changed policy also breaks the interval. This protects
    // both Advisor conclusions and Policy rate estimates that reuse it.
    match (&older.policy, &newer.policy) {
        (Some(older_policy), Some(newer_policy)) if older_policy == newer_policy => {}
        _ => return false,
    }

    [
        (older.tuples_inserted, newer.tuples_inserted),
        (older.tuples_updated, newer.tuples_updated),
        (older.tuples_deleted, newer.tuples_deleted),
        (older.hot_updates, newer.hot_updates),
        (older.vacuum_count, newer.vacuum_count),
        (older.autovacuum_count, newer.autovacuum_count),
        (older.analyze_count, newer.analyze_count),
        (older.autoanalyze_count, newer.autoanalyze_count),
    ]
    .into_iter()
    .all(|(older, newer)| newer >= older)
}

pub(crate) fn latest_continuous_maintenance_samples(
    history: &[AutovacuumMaintenanceHistorySample],
) -> Vec<AutovacuumMaintenanceHistorySample> {
    let mut samples = history.to_vec();
    samples.sort_by_key(|sample| sample.collection_started_at_ms);

    let start = samples
        .windows(2)
        .rposition(|pair| !maintenance_stats_pair_continuous(&pair[0], &pair[1]))
        .map(|index| index + 1)
        .unwrap_or(0);

    samples.split_off(start)
}

#[derive(Clone, Debug, Serialize)]
pub struct AutovacuumPolicyCandidate {
    pub id: String,
    pub area: String,
    pub scenario: String,
    pub target_relation: String,
    pub parameter: String,
    pub current_scale_factor: f64,
    pub current_source: String,
    pub candidate_scale_factor: f64,
    pub fixed_threshold: f64,
    pub current_trigger: i64,
    pub candidate_trigger: i64,
    pub observed_rate_per_hour: f64,
    pub current_volume_hours: f64,
    pub candidate_volume_hours: f64,
    pub sql_preview: String,
    pub rationale: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumPolicyCandidates {
    pub relation_oid: u32,
    pub relation_name: String,
    pub status: String,
    pub summary: String,
    pub candidates: Vec<AutovacuumPolicyCandidate>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumPolicyApplyObservedState {
    pub relation_oid: u32,
    pub subject_relation: String,
    pub parameter: String,
    pub scale_factor: f64,
    pub source: String,
    pub fixed_threshold: f64,
    pub absolute_trigger: i64,
    pub estimated_rows: i64,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumPolicyApplyVerification {
    pub candidate_id: String,
    pub parameter: String,
    pub target_relation: String,
    pub candidate_scale_factor: f64,
    pub before: AutovacuumPolicyApplyObservedState,
    pub after: AutovacuumPolicyApplyObservedState,
    // Protected, explicitly executed rollback SQL. This is NOT run automatically.
    pub rollback_sql: Option<String>,
    pub rollback_note: String,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumMaintenanceFinding {
    pub id: String,
    pub area: String,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub observed: Vec<String>,
    pub policy_parameters: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumMaintenanceAdvice {
    pub relation_oid: u32,
    pub relation_name: String,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub history_points: usize,
    pub history_span_seconds: Option<f64>,
    pub stats_period_continuous: bool,
    pub inserts_per_hour: Option<f64>,
    pub updates_per_hour: Option<f64>,
    pub deletes_per_hour: Option<f64>,
    pub hot_update_pct: Option<f64>,
    pub vacuum_cycles: Option<i64>,
    pub autovacuum_cycles: Option<i64>,
    pub analyze_cycles: Option<i64>,
    pub autoanalyze_cycles: Option<i64>,
    pub toast_vacuum_cycles: Option<i64>,
    pub findings: Vec<AutovacuumMaintenanceFinding>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumTableDetail {
    pub table: AutovacuumTable,
    pub indexes: AutovacuumSection<Vec<AutovacuumIndex>>,
    pub update_statements: AutovacuumSection<Vec<AutovacuumUpdateStatement>>,
    pub fixed_scale_candidates: Vec<AutovacuumScaleCandidate>,
    pub interpretation: AutovacuumInterpretation,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumMethodStep {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub how_to_read: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumQueryReference {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub output: Vec<String>,
    pub how_to_read: Vec<String>,
    pub sql: String,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumSourceReference {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct AutovacuumReference {
    pub methodology: Vec<AutovacuumMethodStep>,
    pub queries: Vec<AutovacuumQueryReference>,
    pub sources: Vec<AutovacuumSourceReference>,
}
