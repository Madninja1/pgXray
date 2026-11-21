use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct XidOverview {
    pub measured_at: String,
    pub database: String,
    pub database_oid: u32,
    pub username: String,
    pub server_version: String,
    pub server_version_num: i32,
    /// Server lifetime boundary: changes on restart, recovery or failover.
    /// Legacy snapshots do not contain this value and cannot justify a new policy.
    #[serde(default)]
    pub postmaster_start_time: Option<String>,
    #[serde(default)]
    pub in_recovery: Option<bool>,
    /// Upper XID bound of the observation snapshot (legacy JSON key: current_xid).
    /// Obtained without allocating a transaction ID in a read-only diagnostic.
    pub current_xid: String,
    pub autovacuum: bool,
    pub autovacuum_freeze_max_age: i64,
    pub vacuum_freeze_table_age: i64,
    pub vacuum_freeze_min_age: i64,
    pub autovacuum_max_workers: i32,
    pub autovacuum_vacuum_cost_limit: i32,
    pub autovacuum_vacuum_cost_delay: String,
    pub database_xid_age: i64,
    pub stats_reset: Option<String>,
    #[serde(default)]
    pub database_mxid_age: Option<i64>,
    #[serde(default)]
    pub autovacuum_multixact_freeze_max_age: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidDatabaseAge {
    pub database_oid: u32,
    pub database: String,
    pub xid_age: i64,
    pub pct_of_global_freeze_max: Option<f64>,
    #[serde(default)]
    pub mxid_age: Option<i64>,
    #[serde(default)]
    pub pct_of_global_multixact_freeze_max: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XidRelationKind {
    Table,
    MaterializedView,
    Toast,
    PartitionedParent,
}

impl XidRelationKind {
    pub(crate) fn from_catalog(value: &str) -> Option<Self> {
        match value {
            "r" => Some(Self::Table),
            "m" => Some(Self::MaterializedView),
            "t" => Some(Self::Toast),
            "p" => Some(Self::PartitionedParent),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct XidRelationAge {
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub relation_type: String,
    pub xid_age: i64,
    #[serde(default)]
    pub mxid_age: Option<i64>,
    #[serde(default)]
    pub effective_multixact_freeze_max_age: Option<i64>,
    #[serde(default)]
    pub pct_of_multixact_freeze_max: Option<f64>,
    pub effective_freeze_max_age: i64,
    pub freeze_max_source: String,
    pub pct_of_freeze_max: Option<f64>,
    pub total_size_bytes: i64,
    pub estimated_rows: Option<i64>,
    pub heap_pages: Option<i64>,
    pub all_visible_pages: Option<i64>,
    #[serde(default)]
    pub all_frozen_pages: Option<i64>, // NULL on PostgreSQL 16-17
    pub all_visible_pct: Option<f64>,
    pub effective_freeze_min_age: Option<i64>,
    pub freeze_min_source: Option<String>,
    pub effective_freeze_table_age: Option<i64>,
    pub freeze_table_source: Option<String>,
    pub last_vacuum: Option<String>,
    pub last_autovacuum: Option<String>,
    pub last_analyze: Option<String>,
    pub last_autoanalyze: Option<String>,
    pub vacuum_count: i64,
    pub autovacuum_count: i64,
    pub analyze_count: Option<i64>,
    pub autoanalyze_count: Option<i64>,
    pub modifications_since_analyze: Option<i64>,
    pub reloptions: Option<Vec<String>>,
    pub parent_oid: Option<u32>,
    pub parent_schema: Option<String>,
    pub parent_relation: Option<String>,
    pub parent_relation_type: Option<String>,
    pub parent_xid_age: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct XidAutovacuumTrigger {
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub relation_type: String,
    pub xid_age: i64,
    pub autovacuum_enabled: bool,
    pub estimated_rows: Option<i64>,
    #[serde(default)]
    pub thresholds_reliable: Option<bool>,
    pub estimated_live_rows: i64,
    pub estimated_dead_rows: i64,
    pub inserts_since_vacuum: i64,
    pub tuples_inserted: i64,
    pub tuples_updated: i64,
    pub tuples_deleted: i64,
    pub hot_updates: i64,
    pub effective_vacuum_threshold: f64,
    pub effective_vacuum_scale_factor: f64,
    pub effective_insert_threshold: f64,
    pub effective_insert_scale_factor: f64,
    pub estimated_dead_tuple_trigger: Option<i64>,
    pub estimated_insert_trigger: Option<i64>,
    pub dead_trigger_reached: bool,
    pub insert_trigger_reached: bool,
    pub last_vacuum: Option<String>,
    pub last_autovacuum: Option<String>,
    pub stats_reset: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct XidVacuum {
    pub pid: i32,
    pub relation_oid: u32,
    pub database: String,
    pub schema_name: String,
    pub relation_name: String,
    pub phase: String,
    pub xid_age: i64,
    pub heap_blks_scanned: i64,
    pub heap_blks_total: i64,
    pub scan_pct: Option<f64>,
    pub backend_type: Option<String>,
    pub is_wraparound_vacuum: bool,
    #[serde(default)]
    pub wraparound_detection: Option<String>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidBackend {
    pub pid: i32,
    pub database: Option<String>,
    pub username: Option<String>,
    pub application_name: Option<String>,
    pub state: Option<String>,
    pub xmin_age: i64,
    pub transaction_age_seconds: Option<i64>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidSlot {
    pub slot_name: String,
    pub slot_type: String,
    pub database: Option<String>,
    pub active: bool,
    pub xmin_age: Option<i64>,
    pub catalog_xmin_age: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidPreparedTransaction {
    pub gid: String,
    pub xid: String,
    pub database: String,
    pub owner: String,
    pub prepared_at: String,
    pub age_seconds: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidPreparedSummary {
    pub count: i64,
    pub oldest_age_seconds: Option<i64>,
    /// Old JSON history remains readable. At most 10 oldest prepared transactions.
    #[serde(default)]
    pub examples: Vec<XidPreparedTransaction>,
}

#[derive(Debug, Serialize)]
pub struct XidSection<T> {
    pub data: Option<T>,
    pub error: Option<String>,
    /// Collection completion time on the pgXray client, in Unix milliseconds.
    /// Older saved snapshots have no value here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
}

#[derive(Deserialize)]
struct XidSectionWire<T> {
    data: Option<T>,
    error: Option<String>,
    #[serde(default)]
    observed_at_ms: Option<u64>,
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for XidSection<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = XidSectionWire::<T>::deserialize(deserializer)?;
        if wire.data.is_some() == wire.error.is_some() {
            return Err(serde::de::Error::custom(
                "XID section must contain either data or error, not both/neither",
            ));
        }
        Ok(Self {
            data: wire.data,
            error: wire.error,
            observed_at_ms: wire.observed_at_ms,
        })
    }
}

impl<T> XidSection<T> {
    pub(crate) fn success(data: T, observed_at_ms: Option<u64>) -> Self {
        Self {
            data: Some(data),
            error: None,
            observed_at_ms,
        }
    }

    pub(crate) fn failure(error: String, observed_at_ms: Option<u64>) -> Self {
        Self {
            data: None,
            error: Some(error),
            observed_at_ms,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidInterpretationItem {
    pub id: String,
    pub title: String,
    pub observed: Vec<String>,
    pub meaning: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidInterpretation {
    pub items: Vec<XidInterpretationItem>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XidSnapshot {
    pub overview: XidOverview,
    pub databases: XidSection<Vec<XidDatabaseAge>>,
    pub relations: XidSection<Vec<XidRelationAge>>,
    pub autovacuum_triggers: XidSection<Vec<XidAutovacuumTrigger>>,
    pub vacuums: XidSection<Vec<XidVacuum>>,
    pub backends: XidSection<Vec<XidBackend>>,
    pub slots: XidSection<Vec<XidSlot>>,
    pub prepared: XidSection<XidPreparedSummary>,
    pub interpretation: XidInterpretation,
}

#[derive(Clone, Debug)]
pub(crate) struct XidPolicyHistorySample {
    pub snapshot_id: i64,
    pub collection_started_at_ms: i64,
    pub current_xid: String,
    pub postmaster_start_time: Option<String>,
    pub in_recovery: Option<bool>,
    pub stats_reset: Option<String>,
    pub relation_xid_age: i64,
    pub vacuum_count: i64,
    pub autovacuum_count: i64,
}

#[derive(Debug, Serialize)]
pub struct XidFreezePolicyView {
    pub effective_freeze_max_age: i64,
    pub freeze_max_source: String,
    pub effective_freeze_table_age: Option<i64>,
    pub freeze_table_source: Option<String>,
    pub effective_freeze_min_age: Option<i64>,
    pub freeze_min_source: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct XidPolicyCandidate {
    /// The server lifetime for which this recommendation was calculated.
    pub expected_postmaster_start_time: String,
    pub expected_in_recovery: bool,
    pub id: String,
    pub title: String,
    pub parameter: String,
    pub current_value: i64,
    pub candidate_value: i64,
    pub ddl_target: String,
    pub sql: String,
    pub why: Vec<String>,
    pub expected_effect: Vec<String>,
    pub tradeoffs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct XidPolicyLiveState {
    #[serde(default)]
    pub postmaster_start_time: Option<String>,
    #[serde(default)]
    pub in_recovery: Option<bool>,
    pub relation_oid: u32,
    pub schema_name: String,
    pub relation_name: String,
    pub relation_type: String,
    pub xid_age: i64,
    pub effective_freeze_max_age: i64,
    pub freeze_max_source: String,
    pub effective_freeze_table_age: Option<i64>,
    pub freeze_table_source: Option<String>,
    pub effective_freeze_min_age: Option<i64>,
    pub freeze_min_source: Option<String>,
    pub parent_oid: Option<u32>,
    pub parent_schema: Option<String>,
    pub parent_relation: Option<String>,
    pub parent_relation_type: Option<String>,
    pub active_vacuum_pid: Option<i32>,
    pub active_vacuum_phase: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct XidPolicyApplyVerification {
    pub candidate_id: String,
    pub parameter: String,
    pub candidate_value: i64,
    pub before: XidPolicyLiveState,
    pub after: XidPolicyLiveState,
}

#[derive(Debug, Serialize)]
pub struct XidRelationPolicyAdvice {
    pub relation_oid: u32,
    pub relation_name: String,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub history_points: usize,
    pub history_span_seconds: Option<f64>,
    pub xid_rate_per_hour: Option<f64>,
    pub relation_age_delta: Option<i64>,
    pub estimated_horizon_advance: Option<i64>,
    pub vacuum_cycles_delta: Option<i64>,
    pub stats_period_continuous: bool,
    pub current_policy: XidFreezePolicyView,
    pub primary_candidate_id: Option<String>,
    pub candidates: Vec<XidPolicyCandidate>,
    pub observations: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct XidMethodStep {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub how_to_read: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct XidQueryReference {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub output: Vec<String>,
    pub how_to_read: Vec<String>,
    pub sql: String,
}

#[derive(Debug, Serialize)]
pub struct XidSourceReference {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct XidReference {
    pub methodology: Vec<XidMethodStep>,
    pub queries: Vec<XidQueryReference>,
    pub sources: Vec<XidSourceReference>,
}
