use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssOverview {
    pub measured_at: String,
    pub database: String,
    pub database_oid: u32,
    pub username: String,
    pub server_version: String,
    pub server_version_num: i32,
    pub extension_installed: bool,
    pub extension_version: Option<String>,
    pub preloaded: bool,
    pub shared_preload_libraries: String,
    pub compute_query_id: Option<String>,
    pub max_entries: Option<i64>,
    pub track: Option<String>,
    pub track_utility: Option<bool>,
    pub track_planning: Option<bool>,
    pub save: Option<bool>,
    pub track_io_timing: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssStatsInfo {
    pub dealloc: i64,
    // PostgreSQL may not yet have a recorded statistics reset timestamp.
    #[serde(default)]
    pub stats_reset: Option<String>,
    #[serde(default)]
    pub stats_age_seconds: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssCapacity {
    pub max_entries: i64,
    pub current_entries: i64,
    pub fill_pct: f64,
    pub one_call_entries: i64,
    pub one_call_pct: f64,
    pub up_to_5_calls_entries: i64,
    pub up_to_5_calls_pct: f64,
    pub total_calls: i64,
    #[serde(default)]
    pub total_exec_time_ms: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssRoleDatabaseCardinality {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub entries: i64,
    pub entries_pct: f64,
    pub one_call_entries: i64,
    pub one_call_pct: f64,
    pub up_to_5_calls_entries: i64,
    pub up_to_5_calls_pct: f64,
    pub total_calls: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssCommandCardinality {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub command: String,
    pub entries: i64,
    pub entries_pct: f64,
    pub one_call_entries: i64,
    pub one_call_pct: f64,
    pub up_to_5_calls_entries: i64,
    pub up_to_5_calls_pct: f64,
    pub total_calls: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssCallTargetCardinality {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub call_tag: String,
    // Short identifier extracted from visible CALL text; not the SQL body or an OID.
    #[serde(default)]
    pub call_target_name: Option<String>,
    pub entries: i64,
    pub entries_pct_of_max: f64,
    pub one_call_entries: i64,
    pub one_call_pct: f64,
    pub total_calls: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PgssPerformanceRow {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub toplevel: bool,
    pub queryid: Option<String>,
    pub calls: i64,
    pub plans: i64,
    pub total_plan_time_ms: f64,
    pub mean_plan_time_ms: f64,
    pub stddev_plan_time_ms: f64,
    pub total_exec_time_ms: f64,
    pub min_exec_time_ms: f64,
    pub max_exec_time_ms: f64,
    pub mean_exec_time_ms: f64,
    pub stddev_exec_time_ms: f64,
    pub rows: i64,
    pub shared_blks_hit: i64,
    pub shared_blks_read: i64,
    pub shared_blks_dirtied: i64,
    pub shared_blks_written: i64,
    pub temp_blks_read: i64,
    pub temp_blks_written: i64,
    pub blk_read_time_ms: f64,
    pub blk_write_time_ms: f64,
    pub wal_records: i64,
    pub wal_fpi: i64,
    pub wal_bytes: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssPerformanceSamples {
    pub total_exec: Vec<PgssPerformanceRow>,
    pub mean_exec: Vec<PgssPerformanceRow>,
    pub temp_io: Vec<PgssPerformanceRow>,
    pub shared_reads: Vec<PgssPerformanceRow>,
    pub wal: Vec<PgssPerformanceRow>,
    pub variance: Vec<PgssPerformanceRow>,
    pub planning: Vec<PgssPerformanceRow>,
    #[serde(default)]
    pub history_sample: Vec<PgssPerformanceRow>,
}

/// Live PGSS browser. All predicates are validated before SQL construction.
/// A page is not a saved snapshot; the server may change between requests.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PgssExplorerFilter {
    pub userid: Option<u32>,
    pub dbid: Option<u32>,
    pub command: Option<String>,
    pub call_tag: Option<String>,
    pub queryid: Option<String>,
    pub min_calls: Option<u64>,
    pub max_calls: Option<u64>,
    pub min_total_exec_ms: Option<f64>,
    #[serde(default)]
    pub one_call: bool,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub direction: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PgssExplorerPage {
    pub rows: Vec<PgssPerformanceRow>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
    pub has_more: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssQueryDetail {
    pub userid: u32,
    pub dbid: u32,
    pub role_name: Option<String>,
    pub database: Option<String>,
    pub toplevel: bool,
    pub queryid: String,
    pub query: Option<String>,
    // None means that the SQL text was not visible to this role.
    #[serde(default)]
    pub query_truncated: Option<bool>,
    pub calls: i64,
    pub plans: i64,
    pub total_plan_time_ms: f64,
    pub mean_plan_time_ms: f64,
    pub stddev_plan_time_ms: f64,
    pub total_exec_time_ms: f64,
    pub min_exec_time_ms: f64,
    pub max_exec_time_ms: f64,
    pub mean_exec_time_ms: f64,
    pub stddev_exec_time_ms: f64,
    pub rows: i64,
    pub shared_blks_hit: i64,
    pub shared_blks_read: i64,
    pub temp_blks_read: i64,
    pub temp_blks_written: i64,
    pub wal_bytes: f64,
}

#[derive(Debug, Serialize)]
pub struct PgssSection<T> {
    pub data: Option<T>,
    pub error: Option<String>,
}

// Treat older snapshots with neither field as unavailable, never as "empty".
// A section with both data and error is contradictory and must not be trusted.
impl<'de, T: Deserialize<'de>> Deserialize<'de> for PgssSection<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire<T> {
            data: Option<T>,
            error: Option<String>,
        }
        let wire = Wire::<T>::deserialize(deserializer)?;
        match (wire.data, wire.error) {
            (Some(data), None) => Ok(Self {
                data: Some(data),
                error: None,
            }),
            (None, Some(error)) => Ok(Self {
                data: None,
                error: Some(error),
            }),
            (None, None) => Ok(missing_pgss_section()),
            (Some(_), Some(_)) => Err(serde::de::Error::custom(
                "PGSS section cannot contain both data and error",
            )),
        }
    }
}

fn missing_pgss_section<T>() -> PgssSection<T> {
    PgssSection {
        data: None,
        error: Some(
            crate::i18n::text(
                "Секция отсутствует в сохранённом PGSS snapshot",
                "Section is missing from the saved PGSS snapshot",
            )
            .into(),
        ),
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssInterpretationItem {
    pub id: String,
    pub title: String,
    pub observed: Vec<String>,
    pub meaning: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct PgssInterpretation {
    #[serde(default)]
    pub items: Vec<PgssInterpretationItem>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PgssSnapshot {
    pub overview: PgssOverview,
    #[serde(default = "missing_pgss_section")]
    pub stats: PgssSection<PgssStatsInfo>,
    #[serde(default = "missing_pgss_section")]
    pub capacity: PgssSection<PgssCapacity>,
    #[serde(default = "missing_pgss_section")]
    pub role_database: PgssSection<Vec<PgssRoleDatabaseCardinality>>,
    #[serde(default = "missing_pgss_section")]
    pub commands: PgssSection<Vec<PgssCommandCardinality>>,
    #[serde(default = "missing_pgss_section")]
    pub call_targets: PgssSection<Vec<PgssCallTargetCardinality>>,
    #[serde(default = "missing_pgss_section")]
    pub performance: PgssSection<PgssPerformanceSamples>,
    #[serde(default)]
    pub interpretation: PgssInterpretation,
}

#[derive(Debug, Serialize)]
pub struct PgssMethodStep {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub how_to_read: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PgssQueryReference {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub output: Vec<String>,
    pub how_to_read: Vec<String>,
    pub sql: String,
}

#[derive(Debug, Serialize)]
pub struct PgssSourceReference {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct PgssReference {
    pub methodology: Vec<PgssMethodStep>,
    pub queries: Vec<PgssQueryReference>,
    pub sources: Vec<PgssSourceReference>,
}
