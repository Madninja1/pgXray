use crate::privacy::ResultPrivacyConfig;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_STATUS: &str = "open";
const ARTIFACT_COMPARISON_MIGRATION: &str = "artifact-comparison-model-v1";
const UNKNOWN_ARTIFACT_SHA256: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_EDITABLE_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct PrivacyScope {
    pub id: i64,
    pub name: String,
    pub connection_id: Option<i64>,
    pub is_default: bool,
    pub mapping_path: String,
    pub result_privacy: ResultPrivacyConfig,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivacyScopeUsage {
    pub case_count: i64,
    pub artifact_count: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct InvestigationCase {
    pub id: i64,
    pub connection_id: i64,
    pub scope_id: i64,
    pub title: String,
    pub description: String,
    pub status: String,
    pub kind: String,
    pub trigger: Option<String>,
    pub detected_at_ms: Option<i64>,
    pub root_path: String,
    pub inbox_path: String,
    pub source_path: String,
    pub share_path: String,
    pub analyses_path: String,
    pub comparisons_path: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseArtifact {
    pub id: i64,
    pub case_id: i64,
    pub kind: String,
    pub visibility: String,
    pub original_name: String,
    pub relative_path: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub created_at_ms: i64,
    pub modified_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArtifactDerivation {
    pub artifact_id: i64,
    pub source_artifact_id: i64,
    pub source_sha256: String,
    pub scope_id: i64,
    pub mapping_revision: i64,
    pub engine_version: String,
    pub transform_mode: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRole {
    Query,
    Ddl,
    Plan,
    Reference,
    Note,
    Other,
}

impl ArtifactRole {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "query" => Ok(Self::Query),
            "ddl" => Ok(Self::Ddl),
            "plan" => Ok(Self::Plan),
            "reference" => Ok(Self::Reference),
            "note" => Ok(Self::Note),
            "other" => Ok(Self::Other),
            role => Err(if crate::i18n::is_ru() {
                format!("Неизвестная роль artifact: {role}")
            } else {
                format!("Unknown artifact role: {role}")
            }),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Ddl => "ddl",
            Self::Plan => "plan",
            Self::Reference => "reference",
            Self::Note => "note",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonPurpose {
    Query,
    Plan,
}

impl ComparisonPurpose {
    fn from_stored(value: &str) -> Result<Self, String> {
        match value {
            "query" => Ok(Self::Query),
            "plan" => Ok(Self::Plan),
            purpose => Err(if crate::i18n::is_ru() {
                format!("Неизвестный purpose comparison member: {purpose}")
            } else {
                format!("Unknown comparison-member purpose: {purpose}")
            }),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Plan => "plan",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonSide {
    Baseline,
    Candidate,
}

impl ComparisonSide {
    fn from_stored(value: &str) -> Result<Self, String> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "candidate" => Ok(Self::Candidate),
            side => Err(if crate::i18n::is_ru() {
                format!("Неизвестный side comparison member: {side}")
            } else {
                format!("Unknown comparison-member side: {side}")
            }),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonKind {
    SavedPlan,
    QueryResult,
    QueryExplain,
    QueryAnalyze,
    LegacyQuery,
    LegacyPlan,
}

impl ComparisonKind {
    fn from_stored(value: &str) -> Result<Self, String> {
        match value {
            "saved_plan" => Ok(Self::SavedPlan),
            "query_result" => Ok(Self::QueryResult),
            "query_explain" => Ok(Self::QueryExplain),
            "query_analyze" => Ok(Self::QueryAnalyze),
            "legacy_query" => Ok(Self::LegacyQuery),
            "legacy_plan" => Ok(Self::LegacyPlan),
            kind => Err(if crate::i18n::is_ru() {
                format!("Неизвестный kind comparison: {kind}")
            } else {
                format!("Unknown comparison kind: {kind}")
            }),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::SavedPlan => "saved_plan",
            Self::QueryResult => "query_result",
            Self::QueryExplain => "query_explain",
            Self::QueryAnalyze => "query_analyze",
            Self::LegacyQuery => "legacy_query",
            Self::LegacyPlan => "legacy_plan",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseComparisonMember {
    pub artifact_id: i64,
    pub purpose: ComparisonPurpose,
    pub side: ComparisonSide,
    pub artifact_sha256: String,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseComparison {
    pub id: i64,
    pub case_id: i64,
    pub kind: ComparisonKind,
    pub created_at_ms: i64,
    pub result: Option<Value>,
    pub members: Vec<CaseComparisonMember>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseArtifactContext {
    pub artifact: CaseArtifact,
    pub role: Option<ArtifactRole>,
    pub derivation: Option<ArtifactDerivation>,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseIntervention {
    pub id: i64,
    pub case_id: i64,
    pub occurred_at_ms: i64,
    pub kind: String,
    pub title: String,
    pub note: String,
    pub created_at_ms: i64,
}

/// Durable database-operation audit, independent of an Investigation Case.
/// Data in `before` / `after` must contain only approved configuration values,
/// never connection credentials or raw SQL.
#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceChange {
    pub id: i64,
    pub connection_id: i64,
    pub case_id: Option<i64>,
    pub intervention_id: Option<i64>,
    pub kind: String,
    pub actor: Option<String>,
    pub relation_oid: Option<i64>,
    pub snapshot_id: Option<i64>,
    pub parameter: String,
    pub target: Value,
    pub before: Value,
    pub after: Value,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub post_snapshot_id: Option<i64>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseChangeImpact {
    pub id: i64,
    pub case_id: i64,
    pub intervention_id: i64,
    pub before_from_ms: i64,
    pub before_to_ms: i64,
    pub after_from_ms: i64,
    pub after_to_ms: i64,
    pub result: Value,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseNavigationTarget {
    pub kind: String,
    pub artifact_id: Option<i64>,
    pub pid: Option<i64>,
    pub userid: Option<u32>,
    pub dbid: Option<u32>,
    pub queryid: Option<String>,
    pub toplevel: Option<bool>,
    pub snapshot_id: Option<i64>,
    pub relation_oid: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseTimelineEvent {
    pub id: String,
    pub occurred_at_ms: i64,
    pub source: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub evidence_level: String,
    pub navigation: Option<CaseNavigationTarget>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct IncidentFingerprint {
    pub dominant_wait_event: Option<String>,
    pub root_blocker_queryids: Vec<String>,
    pub root_blocker_applications: Vec<String>,
    pub relation_oids: Vec<i64>,
    pub regressed_queryids: Vec<String>,
    pub changed_settings: Vec<String>,
    pub autovacuum_relation_oids: Vec<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SimilarIncidentMatch {
    /// Deterministic ranking signal only; never a probability or causal confidence.
    pub score: u32,
    pub case_id: i64,
    pub title: String,
    pub detected_at_ms: Option<i64>,
    pub matches: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseContext {
    pub case: InvestigationCase,
    pub artifacts: Vec<CaseArtifactContext>,
    pub comparisons: Vec<CaseComparison>,
    pub interventions: Vec<CaseIntervention>,
    pub applied_changes: Vec<WorkspaceChange>,
    pub change_impacts: Vec<CaseChangeImpact>,
    pub timeline: Vec<CaseTimelineEvent>,
    pub fingerprint: IncidentFingerprint,
    pub similar_incidents: Vec<SimilarIncidentMatch>,
    pub similar_search_total_candidates: usize,
    pub similar_search_scanned: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareArtifactState {
    pub artifact: CaseArtifact,
    pub derivation: ArtifactDerivation,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionWorkspace {
    pub connection_id: i64,
    pub root_path: String,
    pub inbox_path: String,
    pub default_scope: PrivacyScope,
}

pub struct WorkspaceStore {
    connection: Connection,
    root_dir: PathBuf,
}

impl WorkspaceStore {
    pub fn open(database_path: &Path, root_dir: &Path) -> Result<Self, String> {
        if let Some(parent) = database_path.parent() {
            create_private_directory(parent)?;
        }

        create_private_directory(root_dir)?;

        let mut connection = Connection::open(database_path)
            .map_err(|error| format!("Не удалось открыть workspace SQLite: {error}"))?;

        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| format!("Не удалось настроить busy timeout workspace: {error}"))?;

        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA foreign_keys = ON;

                CREATE TABLE IF NOT EXISTS privacy_scopes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    connection_id INTEGER,
                    is_default INTEGER NOT NULL DEFAULT 0,
                    created_at_ms INTEGER NOT NULL
                );

                CREATE UNIQUE INDEX IF NOT EXISTS idx_privacy_scope_default_connection
                    ON privacy_scopes(connection_id)
                    WHERE connection_id IS NOT NULL
                      AND is_default = 1;

                CREATE TABLE IF NOT EXISTS investigation_cases (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    connection_id INTEGER NOT NULL,
                    scope_id INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL,
                    kind TEXT NOT NULL DEFAULT 'manual',
                    trigger TEXT,
                    detected_at_ms INTEGER,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(scope_id)
                        REFERENCES privacy_scopes(id)
                        ON DELETE RESTRICT
                );

                CREATE INDEX IF NOT EXISTS idx_cases_connection_updated
                    ON investigation_cases(
                        connection_id,
                        updated_at_ms DESC,
                        id DESC
                    );

                CREATE TABLE IF NOT EXISTS case_artifacts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    case_id INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    visibility TEXT NOT NULL,
                    original_name TEXT NOT NULL,
                    relative_path TEXT NOT NULL,
                    media_type TEXT,
                    size_bytes INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    modified_at_ms INTEGER,
                    FOREIGN KEY(case_id)
                        REFERENCES investigation_cases(id)
                        ON DELETE CASCADE,
                    UNIQUE(case_id, relative_path)
                );

                CREATE INDEX IF NOT EXISTS idx_case_artifacts_case
                    ON case_artifacts(case_id, id DESC);

                CREATE TABLE IF NOT EXISTS privacy_scope_state (
                    scope_id INTEGER PRIMARY KEY,
                    revision INTEGER NOT NULL DEFAULT 1,
                    updated_at_ms INTEGER NOT NULL,
                    result_privacy_json TEXT NOT NULL DEFAULT '{"default":"auto","fields":{}}',
                    FOREIGN KEY(scope_id)
                        REFERENCES privacy_scopes(id)
                        ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS artifact_derivations (
                    artifact_id INTEGER PRIMARY KEY,
                    source_artifact_id INTEGER NOT NULL,
                    source_sha256 TEXT NOT NULL,
                    scope_id INTEGER NOT NULL,
                    mapping_revision INTEGER NOT NULL,
                    engine_version TEXT NOT NULL,
                    transform_mode TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(artifact_id)
                        REFERENCES case_artifacts(id)
                        ON DELETE CASCADE,
                    FOREIGN KEY(source_artifact_id)
                        REFERENCES case_artifacts(id)
                        ON DELETE CASCADE,
                    FOREIGN KEY(scope_id)
                        REFERENCES privacy_scopes(id)
                        ON DELETE RESTRICT
                );

                CREATE INDEX IF NOT EXISTS idx_artifact_derivations_source
                    ON artifact_derivations(
                        source_artifact_id,
                        transform_mode,
                        updated_at_ms DESC
                    );

                CREATE TABLE IF NOT EXISTS case_artifact_roles (
                    artifact_id INTEGER PRIMARY KEY,
                    role TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(artifact_id)
                        REFERENCES case_artifacts(id)
                        ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS workspace_migrations (
                    name TEXT PRIMARY KEY,
                    applied_at_ms INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS case_comparisons (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    case_id INTEGER NOT NULL,
                    kind TEXT NOT NULL CHECK(kind IN (
                        'saved_plan',
                        'query_result',
                        'query_explain',
                        'query_analyze',
                        'legacy_query',
                        'legacy_plan'
                    )),
                    result_json TEXT,
                    created_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(case_id)
                        REFERENCES investigation_cases(id)
                        ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_case_comparisons_case
                    ON case_comparisons(case_id, created_at_ms DESC, id DESC);

                CREATE TABLE IF NOT EXISTS case_comparison_members (
                    comparison_id INTEGER NOT NULL,
                    artifact_id INTEGER NOT NULL,
                    purpose TEXT NOT NULL CHECK(purpose IN ('query', 'plan')),
                    side TEXT NOT NULL CHECK(side IN ('baseline', 'candidate')),
                    artifact_sha256 TEXT NOT NULL CHECK(length(artifact_sha256) = 64),
                    PRIMARY KEY(comparison_id, purpose, side),
                    UNIQUE(comparison_id, artifact_id, purpose),
                    FOREIGN KEY(comparison_id)
                        REFERENCES case_comparisons(id)
                        ON DELETE CASCADE,
                    FOREIGN KEY(artifact_id)
                        REFERENCES case_artifacts(id)
                        ON DELETE RESTRICT
                );

                CREATE INDEX IF NOT EXISTS idx_case_comparison_members_artifact
                    ON case_comparison_members(artifact_id, comparison_id);

                CREATE TABLE IF NOT EXISTS workspace_changes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    connection_id INTEGER NOT NULL,
                    case_id INTEGER REFERENCES investigation_cases(id) ON DELETE SET NULL,
                    intervention_id INTEGER REFERENCES case_interventions(id) ON DELETE SET NULL,
                    kind TEXT NOT NULL,
                    actor TEXT,
                    relation_oid INTEGER,
                    snapshot_id INTEGER,
                    parameter TEXT NOT NULL,
                    before_json TEXT NOT NULL,
                    after_json TEXT NOT NULL,
                    status TEXT NOT NULL CHECK(status IN ('requested', 'verified', 'failed')),
                    started_at_ms INTEGER NOT NULL,
                    finished_at_ms INTEGER,
                    post_snapshot_id INTEGER,
                    note TEXT NOT NULL DEFAULT '',
                    target_json TEXT NOT NULL DEFAULT '{}'
                );

                CREATE INDEX IF NOT EXISTS idx_workspace_changes_connection
                    ON workspace_changes(connection_id, started_at_ms DESC, id DESC);
                CREATE INDEX IF NOT EXISTS idx_workspace_changes_case
                    ON workspace_changes(case_id, started_at_ms DESC, id DESC);

                CREATE TABLE IF NOT EXISTS case_interventions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    case_id INTEGER NOT NULL,
                    occurred_at_ms INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    note TEXT NOT NULL DEFAULT '',
                    created_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(case_id)
                        REFERENCES investigation_cases(id)
                        ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_case_interventions_case_time
                    ON case_interventions(case_id, occurred_at_ms ASC, id ASC);

                CREATE TABLE IF NOT EXISTS case_change_impacts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    case_id INTEGER NOT NULL,
                    intervention_id INTEGER NOT NULL,
                    before_from_ms INTEGER NOT NULL,
                    before_to_ms INTEGER NOT NULL,
                    after_from_ms INTEGER NOT NULL,
                    after_to_ms INTEGER NOT NULL,
                    result_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(case_id)
                        REFERENCES investigation_cases(id)
                        ON DELETE CASCADE,
                    FOREIGN KEY(intervention_id)
                        REFERENCES case_interventions(id)
                        ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_case_change_impacts_case
                    ON case_change_impacts(case_id, created_at_ms DESC, id DESC);
                "#,
            )
            .map_err(|error| format!("Не удалось подготовить workspace schema: {error}"))?;

        ensure_workspace_change_target_column(&connection)?;
        ensure_privacy_scope_result_policy_column(&connection)?;
        ensure_case_comparison_schema(&mut connection, root_dir)?;
        ensure_case_comparison_result_column(&connection)?;
        ensure_investigation_case_description_column(&connection)?;
        ensure_investigation_case_incident_columns(&connection)?;
        ensure_case_artifact_modified_at_column(&connection)?;
        backfill_case_artifact_modified_at(&connection, root_dir)?;
        migrate_legacy_artifact_roles(&mut connection, root_dir)?;

        if database_path.exists() {
            fs::set_permissions(database_path, fs::Permissions::from_mode(0o600)).map_err(
                |error| format!("Не удалось выставить 0600 для workspace SQLite: {error}"),
            )?;
        }

        Ok(Self {
            connection,
            root_dir: root_dir.to_path_buf(),
        })
    }

    pub fn integrity_check(&self) -> Result<(), String> {
        let result: String = self
            .connection
            .query_row(crate::SQLITE_QUICK_CHECK_SQL, [], |row| row.get(0))
            .map_err(|error| format!("Не удалось проверить workspace SQLite: {error}"))?;

        if result == "ok" {
            Ok(())
        } else {
            Err(format!("workspace SQLite quick_check: {result}"))
        }
    }

    pub fn ensure_connection_workspace(
        &mut self,
        connection_id: i64,
        connection_name: &str,
    ) -> Result<ConnectionWorkspace, String> {
        validate_positive_id(connection_id, "connection")?;

        let root = self.connection_root(connection_id);

        let inbox = root.join("inbox");

        create_private_directory(&root)?;

        create_private_directory(&inbox)?;

        let default_scope = self.ensure_default_scope(connection_id, connection_name)?;

        Ok(ConnectionWorkspace {
            connection_id,
            root_path: path_string(&root),
            inbox_path: path_string(&inbox),
            default_scope,
        })
    }

    pub fn create_scope(
        &mut self,
        name: &str,
        connection_id: Option<i64>,
    ) -> Result<PrivacyScope, String> {
        let name = validate_name(name, "privacy scope")?;

        if let Some(connection_id) = connection_id {
            validate_positive_id(connection_id, "connection")?;
        }

        let created_at_ms = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT INTO privacy_scopes (
                    name,
                    connection_id,
                    is_default,
                    created_at_ms
                )
                VALUES (?1, ?2, 0, ?3)
                "#,
                params![name, connection_id, created_at_ms,],
            )
            .map_err(|error| format!("Не удалось создать privacy scope: {error}"))?;

        let id = self.connection.last_insert_rowid();

        self.scope_by_id(id)
    }

    pub fn list_scopes(&mut self, connection_id: Option<i64>) -> Result<Vec<PrivacyScope>, String> {
        let mut result = Vec::new();

        if let Some(connection_id) = connection_id {
            let mut statement = self
                .connection
                .prepare(
                    r#"
                        SELECT
                            id,
                            name,
                            connection_id,
                            is_default,
                            created_at_ms
                        FROM privacy_scopes
                        WHERE connection_id = ?1
                           OR connection_id IS NULL
                        ORDER BY
                            is_default DESC,
                            name COLLATE NOCASE,
                            id
                        "#,
                )
                .map_err(|error| format!("Не удалось подготовить список scopes: {error}"))?;

            let rows = statement
                .query_map(params![connection_id,], scope_row)
                .map_err(|error| format!("Не удалось прочитать scopes: {error}"))?;

            for row in rows {
                result.push(self.attach_scope_path(
                    row.map_err(|error| format!("Повреждена строка scope: {error}"))?,
                )?);
            }
        } else {
            let mut statement = self
                .connection
                .prepare(
                    r#"
                        SELECT
                            id,
                            name,
                            connection_id,
                            is_default,
                            created_at_ms
                        FROM privacy_scopes
                        ORDER BY
                            connection_id,
                            is_default DESC,
                            name COLLATE NOCASE,
                            id
                        "#,
                )
                .map_err(|error| format!("Не удалось подготовить список scopes: {error}"))?;

            let rows = statement
                .query_map([], scope_row)
                .map_err(|error| format!("Не удалось прочитать scopes: {error}"))?;

            for row in rows {
                result.push(self.attach_scope_path(
                    row.map_err(|error| format!("Повреждена строка scope: {error}"))?,
                )?);
            }
        }

        Ok(result)
    }

    pub fn create_case(
        &mut self,
        connection_id: i64,
        connection_name: &str,
        title: &str,
        description: &str,
        scope_id: Option<i64>,
    ) -> Result<InvestigationCase, String> {
        self.create_case_internal(
            connection_id,
            connection_name,
            title,
            description,
            scope_id,
            "manual",
            None,
            None,
        )
    }

    pub fn create_incident_case(
        &mut self,
        connection_id: i64,
        connection_name: &str,
        title: &str,
        description: &str,
        trigger: &str,
        detected_at_ms: i64,
    ) -> Result<InvestigationCase, String> {
        if detected_at_ms <= 0 {
            return Err("Некорректный detected_at incident case".into());
        }

        let trigger = validate_case_trigger(trigger)?;

        self.create_case_internal(
            connection_id,
            connection_name,
            title,
            description,
            None,
            "incident",
            Some(&trigger),
            Some(detected_at_ms),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "existing call contract; refactor separately with callers"
    )]
    fn create_case_internal(
        &mut self,
        connection_id: i64,
        connection_name: &str,
        title: &str,
        description: &str,
        scope_id: Option<i64>,
        kind: &str,
        trigger: Option<&str>,
        detected_at_ms: Option<i64>,
    ) -> Result<InvestigationCase, String> {
        validate_positive_id(connection_id, "connection")?;

        let title = validate_name(title, "case")?;
        let description = validate_case_description(description)?;
        let kind = validate_case_kind(kind)?;

        let scope = match scope_id {
            Some(scope_id) => {
                let scope = self.scope_by_id(scope_id)?;

                if let Some(scope_connection) = scope.connection_id {
                    if scope_connection != connection_id {
                        return Err("Privacy scope принадлежит другому connection".into());
                    }
                }

                scope
            }

            None => self.ensure_default_scope(connection_id, connection_name)?,
        };

        let now = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT INTO investigation_cases (
                    connection_id,
                    scope_id,
                    title,
                    description,
                    status,
                    kind,
                    trigger,
                    detected_at_ms,
                    created_at_ms,
                    updated_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                "#,
                params![
                    connection_id,
                    scope.id,
                    title,
                    description,
                    DEFAULT_STATUS,
                    kind,
                    trigger,
                    detected_at_ms,
                    now,
                ],
            )
            .map_err(|error| format!("Не удалось создать investigation case: {error}"))?;

        let id = self.connection.last_insert_rowid();

        let case = self.case_by_id(id)?;

        self.ensure_case_directories(case.connection_id, case.id)?;

        self.case_by_id(id)
    }

    pub fn list_cases(&mut self, connection_id: i64) -> Result<Vec<InvestigationCase>, String> {
        validate_positive_id(connection_id, "connection")?;

        let mut statement = self
            .connection
            .prepare(
                r#"
                    SELECT
                        id,
                        connection_id,
                        scope_id,
                        title,
                        description,
                        status,
                        kind,
                        trigger,
                        detected_at_ms,
                        created_at_ms,
                        updated_at_ms
                    FROM investigation_cases
                    WHERE connection_id = ?1
                    ORDER BY
                        created_at_ms DESC,
                        id DESC
                    "#,
            )
            .map_err(|error| format!("Не удалось подготовить список cases: {error}"))?;

        let rows = statement
            .query_map(params![connection_id,], case_row)
            .map_err(|error| format!("Не удалось прочитать cases: {error}"))?;

        let mut result = Vec::new();

        for row in rows {
            result.push(self.attach_case_paths(
                row.map_err(|error| format!("Повреждена строка case: {error}"))?,
            )?);
        }

        Ok(result)
    }

    pub fn case_by_id(&mut self, case_id: i64) -> Result<InvestigationCase, String> {
        validate_positive_id(case_id, "case")?;

        let row = self
            .connection
            .query_row(
                r#"
                    SELECT
                        id,
                        connection_id,
                        scope_id,
                        title,
                        description,
                        status,
                        kind,
                        trigger,
                        detected_at_ms,
                        created_at_ms,
                        updated_at_ms
                    FROM investigation_cases
                    WHERE id = ?1
                    "#,
                params![case_id,],
                case_row,
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать case #{case_id}: {error}"))?
            .ok_or_else(|| format!("Case #{case_id} не найден"))?;

        self.attach_case_paths(row)
    }

    pub fn update_case_description(
        &mut self,
        case_id: i64,
        description: &str,
    ) -> Result<InvestigationCase, String> {
        self.case_by_id(case_id)?;

        let description = validate_case_description(description)?;
        let now = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                    UPDATE investigation_cases
                    SET
                        description = ?2,
                        updated_at_ms = ?3
                    WHERE id = ?1
                "#,
                params![case_id, description, now,],
            )
            .map_err(|error| format!("Не удалось обновить описание case #{case_id}: {error}"))?;

        self.case_by_id(case_id)
    }

    /// Creates an audit record BEFORE attempting a PostgreSQL write.
    /// If this transaction fails, the caller should not execute Apply.
    #[expect(
        clippy::too_many_arguments,
        reason = "audit fields are explicit to avoid opaque caller data"
    )]
    pub fn begin_workspace_change(
        &mut self,
        connection_id: i64,
        kind: &str,
        actor: Option<&str>,
        relation_oid: Option<i64>,
        snapshot_id: Option<i64>,
        parameter: &str,
        before: &Value,
        after: &Value,
        target: &Value,
    ) -> Result<i64, String> {
        validate_positive_id(connection_id, "connection")?;
        if !matches!(kind, "autovacuum_policy" | "xid_policy") {
            return Err("Неизвестный тип audit операции".into());
        }
        if parameter.is_empty() || parameter.len() > 128 {
            return Err("Некорректный параметр audit операции".into());
        }
        if !target.is_object()
            || target.get("host").and_then(Value::as_str).is_none()
            || target.get("database").and_then(Value::as_str).is_none()
        {
            return Err("Недостаточно сведений о PostgreSQL target для audit".into());
        }
        let target_json = serde_json::to_string(target).map_err(|e| e.to_string())?;
        let now = unix_time_millis()?;
        let before_json = serde_json::to_string(before).map_err(|e| e.to_string())?;
        let after_json = serde_json::to_string(after).map_err(|e| e.to_string())?;
        self.connection.execute(
            "INSERT INTO workspace_changes (connection_id, kind, actor, relation_oid, snapshot_id, parameter, before_json, after_json, status, started_at_ms, target_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'requested', ?9, ?10)",
            params![connection_id, kind, actor, relation_oid, snapshot_id, parameter, before_json, after_json, now, target_json],
        ).map_err(|e| format!("Не удалось создать audit до Apply: {e}"))?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn finish_workspace_change(
        &mut self,
        change_id: i64,
        status: &str,
        post_snapshot_id: Option<i64>,
        note: &str,
    ) -> Result<(), String> {
        validate_positive_id(change_id, "change")?;
        if !matches!(status, "verified" | "failed") {
            return Err("Недопустимый финальный статус audit".into());
        }
        let finished_at_ms = unix_time_millis()?;
        let updated = self.connection.execute(
            "UPDATE workspace_changes SET status = ?2, finished_at_ms = ?3, post_snapshot_id = ?4, note = ?5 WHERE id = ?1 AND status = 'requested'",
            params![change_id, status, finished_at_ms, post_snapshot_id, note],
        ).map_err(|e| format!("Не удалось завершить audit: {e}"))?;
        if updated != 1 {
            return Err("Audit change не найден или уже завершён".into());
        }
        Ok(())
    }

    pub fn list_workspace_changes(
        &mut self,
        connection_id: i64,
        limit: u32,
    ) -> Result<Vec<WorkspaceChange>, String> {
        validate_positive_id(connection_id, "connection")?;
        let mut stmt = self.connection.prepare(
            "SELECT id, connection_id, case_id, intervention_id, kind, actor, relation_oid, snapshot_id, parameter, before_json, after_json, status, started_at_ms, finished_at_ms, post_snapshot_id, note, target_json FROM workspace_changes WHERE connection_id = ?1 ORDER BY started_at_ms DESC, id DESC LIMIT ?2"
        ).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                params![connection_id, i64::from(limit.clamp(1, 200))],
                workspace_change_row,
            )
            .map_err(|e| e.to_string())?;
        let result: rusqlite::Result<Vec<_>> = rows.collect();
        result.map_err(|e| format!("Не удалось загрузить журнал изменений: {e}"))
    }

    pub fn list_case_workspace_changes(
        &mut self,
        case_id: i64,
    ) -> Result<Vec<WorkspaceChange>, String> {
        self.case_by_id(case_id)?;
        let mut stmt = self.connection.prepare(
            "SELECT id, connection_id, case_id, intervention_id, kind, actor, relation_oid, snapshot_id, parameter, before_json, after_json, status, started_at_ms, finished_at_ms, post_snapshot_id, note, target_json FROM workspace_changes WHERE case_id = ?1 ORDER BY started_at_ms ASC, id ASC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([case_id], workspace_change_row)
            .map_err(|e| e.to_string())?;
        let result: rusqlite::Result<Vec<_>> = rows.collect();
        result.map_err(|e| e.to_string())
    }

    /// Explicit opt-in link: also create an Intervention usable by Change Impact.
    /// Both records are committed atomically. Existing audit remains on Case deletion.
    pub fn link_workspace_change_to_case(
        &mut self,
        change_id: i64,
        case_id: i64,
    ) -> Result<(), String> {
        validate_positive_id(change_id, "change")?;
        let case = self.case_by_id(case_id)?;
        let change = self.connection.query_row(
            "SELECT connection_id, case_id, status, kind, parameter, before_json, after_json, started_at_ms FROM workspace_changes WHERE id = ?1",
            [change_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, i64>(7)?))
        ).optional().map_err(|e| e.to_string())?.ok_or("Изменение не найдено")?;
        let (connection_id, linked_case, status, kind, parameter, before, after, started) = change;
        if case.connection_id != connection_id {
            return Err("Изменение относится к другому подключению".into());
        }
        if linked_case.is_some() {
            return Err("Изменение уже привязано к Case".into());
        }
        let now = unix_time_millis()?;
        let note =
            format!("Apply status: {status}; {parameter}: {before} -> {after}; audit #{change_id}");
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO case_interventions (case_id, occurred_at_ms, kind, title, note, created_at_ms) VALUES (?1, ?2, 'config_change', ?3, ?4, ?5)",
            params![case_id, started, format!("{kind} #{change_id}"), note, now],
        ).map_err(|e| e.to_string())?;
        let intervention_id = tx.last_insert_rowid();
        let updated = tx.execute(
            "UPDATE workspace_changes SET case_id = ?2, intervention_id = ?3 WHERE id = ?1 AND case_id IS NULL AND connection_id = ?4",
            params![change_id, case_id, intervention_id, connection_id],
        ).map_err(|e| e.to_string())?;
        if updated != 1 {
            return Err("Изменение уже связано с другим Case".into());
        }
        tx.execute(
            "UPDATE investigation_cases SET updated_at_ms = ?2 WHERE id = ?1",
            params![case_id, now],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn create_case_intervention(
        &mut self,
        case_id: i64,
        occurred_at_ms: i64,
        kind: &str,
        title: &str,
        note: &str,
    ) -> Result<CaseIntervention, String> {
        self.case_by_id(case_id)?;
        if occurred_at_ms <= 0 {
            return Err("Некорректное время intervention".into());
        }

        let kind = validate_intervention_kind(kind)?;
        let title = validate_name(title, "intervention")?;
        let note = validate_case_description(note)?;
        let created_at_ms = unix_time_millis()?;

        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Не удалось начать сохранение intervention: {error}"))?;
        transaction
            .execute(
                r#"
                INSERT INTO case_interventions (
                    case_id,
                    occurred_at_ms,
                    kind,
                    title,
                    note,
                    created_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![case_id, occurred_at_ms, kind, title, note, created_at_ms],
            )
            .map_err(|error| format!("Не удалось сохранить intervention: {error}"))?;
        let intervention_id = transaction.last_insert_rowid();
        transaction
            .execute(
                "UPDATE investigation_cases SET updated_at_ms = ?2 WHERE id = ?1",
                params![case_id, created_at_ms],
            )
            .map_err(|error| format!("Не удалось обновить case после intervention: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Не удалось завершить сохранение intervention: {error}"))?;

        self.case_intervention_by_id(intervention_id)
    }

    pub fn list_case_interventions(
        &mut self,
        case_id: i64,
    ) -> Result<Vec<CaseIntervention>, String> {
        self.case_by_id(case_id)?;
        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT id, case_id, occurred_at_ms, kind, title, note, created_at_ms
                FROM case_interventions
                WHERE case_id = ?1
                ORDER BY occurred_at_ms ASC, id ASC
                "#,
            )
            .map_err(|error| {
                format!("Не удалось подготовить interventions case #{case_id}: {error}")
            })?;
        let rows = statement
            .query_map(params![case_id], intervention_row)
            .map_err(|error| {
                format!("Не удалось прочитать interventions case #{case_id}: {error}")
            })?;

        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn case_intervention_by_id(
        &mut self,
        intervention_id: i64,
    ) -> Result<CaseIntervention, String> {
        validate_positive_id(intervention_id, "intervention")?;
        self.connection
            .query_row(
                r#"
                SELECT id, case_id, occurred_at_ms, kind, title, note, created_at_ms
                FROM case_interventions
                WHERE id = ?1
                "#,
                params![intervention_id],
                intervention_row,
            )
            .optional()
            .map_err(|error| {
                format!("Не удалось прочитать intervention #{intervention_id}: {error}")
            })?
            .ok_or_else(|| format!("Intervention #{intervention_id} не найден"))
    }

    pub fn delete_case_intervention(&mut self, intervention_id: i64) -> Result<(), String> {
        let intervention = self.case_intervention_by_id(intervention_id)?;
        let impact_count = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM case_change_impacts WHERE intervention_id = ?1",
                params![intervention_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                format!(
                    "Не удалось проверить Change Impact intervention #{intervention_id}: {error}"
                )
            })?;
        if impact_count > 0 {
            return Err(format!(
                "Intervention #{intervention_id} используется в {impact_count} Change Impact verification(s). Сначала удалите verification history вместе с case или оставьте marker."
            ));
        }

        let now = unix_time_millis()?;
        let transaction = self.connection.transaction().map_err(|error| {
            format!("Не удалось начать удаление intervention #{intervention_id}: {error}")
        })?;
        transaction
            .execute(
                "DELETE FROM case_interventions WHERE id = ?1",
                params![intervention_id],
            )
            .map_err(|error| {
                format!("Не удалось удалить intervention #{intervention_id}: {error}")
            })?;
        transaction
            .execute(
                "UPDATE investigation_cases SET updated_at_ms = ?2 WHERE id = ?1",
                params![intervention.case_id, now],
            )
            .map_err(|error| {
                format!("Не удалось обновить case после удаления intervention: {error}")
            })?;
        transaction
            .commit()
            .map_err(|error| format!("Не удалось завершить удаление intervention: {error}"))?;
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "existing call contract; refactor separately with callers"
    )]
    pub fn record_change_impact(
        &mut self,
        case_id: i64,
        intervention_id: i64,
        before_from_ms: i64,
        before_to_ms: i64,
        after_from_ms: i64,
        after_to_ms: i64,
        result: &Value,
    ) -> Result<CaseChangeImpact, String> {
        self.case_by_id(case_id)?;
        let intervention = self.case_intervention_by_id(intervention_id)?;
        if intervention.case_id != case_id {
            return Err("Intervention принадлежит другому case".into());
        }
        validate_change_impact_periods(
            before_from_ms,
            before_to_ms,
            after_from_ms,
            after_to_ms,
            intervention.occurred_at_ms,
        )?;
        if !result.is_object() {
            return Err("Change Impact result должен быть JSON object".into());
        }

        let result_json = serde_json::to_string(result)
            .map_err(|error| format!("Не удалось сериализовать Change Impact: {error}"))?;
        let created_at_ms = unix_time_millis()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Не удалось начать сохранение Change Impact: {error}"))?;
        transaction
            .execute(
                r#"
                INSERT INTO case_change_impacts (
                    case_id,
                    intervention_id,
                    before_from_ms,
                    before_to_ms,
                    after_from_ms,
                    after_to_ms,
                    result_json,
                    created_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    case_id,
                    intervention_id,
                    before_from_ms,
                    before_to_ms,
                    after_from_ms,
                    after_to_ms,
                    result_json,
                    created_at_ms,
                ],
            )
            .map_err(|error| format!("Не удалось сохранить Change Impact: {error}"))?;
        let impact_id = transaction.last_insert_rowid();
        transaction
            .execute(
                "UPDATE investigation_cases SET updated_at_ms = ?2 WHERE id = ?1",
                params![case_id, created_at_ms],
            )
            .map_err(|error| format!("Не удалось обновить case после Change Impact: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Не удалось завершить сохранение Change Impact: {error}"))?;

        self.case_change_impact_by_id(impact_id)
    }

    pub fn list_case_change_impacts(
        &mut self,
        case_id: i64,
    ) -> Result<Vec<CaseChangeImpact>, String> {
        self.case_by_id(case_id)?;
        let mut statement = self
            .connection
            .prepare(
                r#"
                SELECT
                    id,
                    case_id,
                    intervention_id,
                    before_from_ms,
                    before_to_ms,
                    after_from_ms,
                    after_to_ms,
                    result_json,
                    created_at_ms
                FROM case_change_impacts
                WHERE case_id = ?1
                ORDER BY created_at_ms DESC, id DESC
                "#,
            )
            .map_err(|error| {
                format!("Не удалось подготовить Change Impact case #{case_id}: {error}")
            })?;
        let rows = statement
            .query_map(params![case_id], change_impact_row)
            .map_err(|error| {
                format!("Не удалось прочитать Change Impact case #{case_id}: {error}")
            })?;

        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn case_change_impact_by_id(&mut self, impact_id: i64) -> Result<CaseChangeImpact, String> {
        validate_positive_id(impact_id, "change impact")?;
        self.connection
            .query_row(
                r#"
                SELECT
                    id,
                    case_id,
                    intervention_id,
                    before_from_ms,
                    before_to_ms,
                    after_from_ms,
                    after_to_ms,
                    result_json,
                    created_at_ms
                FROM case_change_impacts
                WHERE id = ?1
                "#,
                params![impact_id],
                change_impact_row,
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать Change Impact #{impact_id}: {error}"))?
            .ok_or_else(|| format!("Change Impact #{impact_id} не найден"))
    }

    pub fn delete_case_change_impact(&mut self, impact_id: i64) -> Result<(), String> {
        let impact = self.case_change_impact_by_id(impact_id)?;
        let now = unix_time_millis()?;
        let transaction = self.connection.transaction().map_err(|error| {
            format!("Не удалось начать удаление Change Impact #{impact_id}: {error}")
        })?;
        transaction
            .execute(
                "DELETE FROM case_change_impacts WHERE id = ?1",
                params![impact_id],
            )
            .map_err(|error| format!("Не удалось удалить Change Impact #{impact_id}: {error}"))?;
        transaction
            .execute(
                "UPDATE investigation_cases SET updated_at_ms = ?2 WHERE id = ?1",
                params![impact.case_id, now],
            )
            .map_err(|error| {
                format!("Не удалось обновить case после удаления Change Impact: {error}")
            })?;
        transaction
            .commit()
            .map_err(|error| format!("Не удалось завершить удаление Change Impact: {error}"))?;
        Ok(())
    }

    pub fn scope_by_id(&mut self, scope_id: i64) -> Result<PrivacyScope, String> {
        validate_positive_id(scope_id, "scope")?;

        let scope = self
            .connection
            .query_row(
                r#"
                    SELECT
                        id,
                        name,
                        connection_id,
                        is_default,
                        created_at_ms
                    FROM privacy_scopes
                    WHERE id = ?1
                    "#,
                params![scope_id,],
                scope_row,
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать privacy scope #{scope_id}: {error}"))?
            .ok_or_else(|| format!("Privacy scope #{scope_id} не найден"))?;

        self.attach_scope_path(scope)
    }

    pub fn scope_mapping_path(&mut self, scope_id: i64) -> Result<PathBuf, String> {
        let scope = self.scope_by_id(scope_id)?;

        Ok(PathBuf::from(scope.mapping_path))
    }

    pub fn scope_usage(&mut self, scope_id: i64) -> Result<PrivacyScopeUsage, String> {
        self.scope_by_id(scope_id)?;

        let case_count = self
            .connection
            .query_row(
                r#"
                    SELECT count(*)
                    FROM investigation_cases
                    WHERE scope_id = ?1
                    "#,
                params![scope_id,],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                format!("Не удалось посчитать cases privacy scope #{scope_id}: {error}")
            })?;

        let artifact_count = self
            .connection
            .query_row(
                r#"
                    SELECT count(*)
                    FROM case_artifacts AS a
                    JOIN investigation_cases AS c
                      ON c.id = a.case_id
                    WHERE c.scope_id = ?1
                    "#,
                params![scope_id,],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                format!("Не удалось посчитать artifacts privacy scope #{scope_id}: {error}")
            })?;

        Ok(PrivacyScopeUsage {
            case_count,
            artifact_count,
        })
    }

    pub fn delete_scope(&mut self, scope_id: i64) -> Result<(), String> {
        let scope = self.scope_by_id(scope_id)?;

        if scope.is_default {
            return Err(
                crate::i18n::text("Default privacy scope подключения удалить нельзя. Можно очистить его mapping.json.", "Cannot delete the connection default privacy scope; you can clear its mapping.json instead.")
                    .into(),
            );
        }

        let usage = self.scope_usage(scope_id)?;

        if usage.case_count > 0 {
            return Err(
                format!(
                    "Privacy scope используется cases: {}. Сначала удалите или переведите эти cases на другой scope.",
                    usage.case_count,
                ),
            );
        }

        let directory = PathBuf::from(&scope.mapping_path)
            .parent()
            .map(Path::to_path_buf);

        self.connection
            .execute(
                r#"
                DELETE FROM privacy_scopes
                WHERE id = ?1
                "#,
                params![scope_id,],
            )
            .map_err(|error| format!("Не удалось удалить privacy scope #{scope_id}: {error}"))?;

        if let Some(directory) = directory {
            if directory.exists() {
                fs::remove_dir_all(&directory).map_err(|error| {
                    format!(
                        "Privacy scope удалён из SQLite, но не удалось удалить каталог {}: {error}",
                        directory.display(),
                    )
                })?;
            }
        }

        Ok(())
    }

    pub fn delete_case(&mut self, case_id: i64) -> Result<(), String> {
        let case = self.case_by_id(case_id)?;
        let root = self.case_root(case.connection_id, case_id);
        ensure_no_symlink_components(&root)?;

        // Keep the original directory recoverable until the SQLite deletion commits.
        let staged = if root.exists() {
            let staged =
                root.with_file_name(format!(".case-{case_id}.deleting-{}", unix_time_millis()?,));
            fs::rename(&root, &staged).map_err(|error| {
                format!("Не удалось подготовить case #{case_id} к удалению: {error}")
            })?;
            Some(staged)
        } else {
            None
        };

        let db_result = (|| -> Result<(), String> {
            let transaction = self
                .connection
                .transaction()
                .map_err(|error| format!("Не удалось начать удаление case #{case_id}: {error}"))?;
            transaction
                .execute(
                    "DELETE FROM investigation_cases WHERE id = ?1",
                    params![case_id],
                )
                .map_err(|error| format!("Не удалось удалить case #{case_id}: {error}"))?;
            transaction
                .commit()
                .map_err(|error| format!("Не удалось завершить удаление case #{case_id}: {error}"))
        })();

        if let Err(error) = db_result {
            if let Some(staged) = &staged {
                fs::rename(staged, &root).map_err(|restore_error| {
                    format!(
                        "{error}; не удалось восстановить {}: {restore_error}",
                        root.display()
                    )
                })?;
            }
            return Err(error);
        }

        if let Some(staged) = staged {
            // After commit the directory cannot be restored together with its DB row.
            // Keep the staged directory intact on failure for manual recovery.
            fs::remove_dir_all(&staged).map_err(|error| {
                format!(
                    "Case #{case_id} удалён из SQLite, но очистка {} не завершена: {error}",
                    staged.display(),
                )
            })?;
        }
        Ok(())
    }

    pub fn delete_connection_data(&mut self, connection_id: i64) -> Result<(), String> {
        validate_positive_id(connection_id, "connection")?;

        let root = self.connection_root(connection_id);

        // Filesystem and SQLite cannot share one transaction. Move the whole
        // connection workspace aside first: if the SQLite transaction fails,
        // we can restore the directory without losing the case files/mappings.
        let staged_root = if root.exists() {
            let parent = root
                .parent()
                .ok_or("Connection workspace не имеет parent directory")?;

            let staged = parent.join(format!(
                ".connection-{connection_id}.deleting-{}",
                unix_time_millis()?,
            ));

            fs::rename(&root, &staged).map_err(|error| {
                format!(
                    "Не удалось подготовить workspace connection #{connection_id} к удалению: {error}"
                )
            })?;

            Some(staged)
        } else {
            None
        };

        let database_result = (|| -> Result<(), String> {
            let transaction = self.connection.transaction().map_err(|error| {
                format!("Не удалось начать удаление workspace connection #{connection_id}: {error}")
            })?;

            // case_artifacts, artifact roles and derivations are removed by
            // their ON DELETE CASCADE relations.
            transaction
                .execute(
                    r#"
                    DELETE FROM investigation_cases
                    WHERE connection_id = ?1
                    "#,
                    params![connection_id],
                )
                .map_err(|error| {
                    format!("Не удалось удалить cases connection #{connection_id}: {error}")
                })?;

            // Connection-bound scopes are removed after cases because
            // investigation_cases.scope_id uses ON DELETE RESTRICT.
            // Project scopes (connection_id IS NULL) remain untouched.
            transaction
                .execute(
                    r#"
                    DELETE FROM privacy_scopes
                    WHERE connection_id = ?1
                    "#,
                    params![connection_id],
                )
                .map_err(|error| {
                    format!(
                        "Не удалось удалить privacy scopes connection #{connection_id}: {error}"
                    )
                })?;

            transaction.commit().map_err(|error| {
                format!(
                    "Не удалось завершить удаление workspace connection #{connection_id}: {error}"
                )
            })?;

            Ok(())
        })();

        if let Err(error) = database_result {
            if let Some(staged) = &staged_root {
                if let Err(restore_error) = fs::rename(staged, &root) {
                    return Err(format!(
                        "{error}. Дополнительно не удалось вернуть workspace {} -> {}: {restore_error}",
                        staged.display(),
                        root.display(),
                    ));
                }
            }

            return Err(error);
        }

        if let Some(staged) = staged_root {
            if staged.exists() {
                if let Err(error) = fs::remove_dir_all(&staged) {
                    // SQLite cleanup has already committed. Restore the files
                    // to the canonical connection directory so a retry can
                    // finish cleanup instead of leaving a hidden orphan.
                    let restore_result = fs::rename(&staged, &root);

                    return match restore_result {
                        Ok(()) => Err(format!(
                            "Workspace metadata удалены, но каталог {} удалить не удалось: {error}. Файлы возвращены в {} — повторите удаление.",
                            staged.display(),
                            root.display(),
                        )),
                        Err(restore_error) => Err(format!(
                            "Workspace metadata удалены, каталог {} удалить не удалось: {error}; также не удалось вернуть его в {}: {restore_error}",
                            staged.display(),
                            root.display(),
                        )),
                    };
                }
            }
        }

        Ok(())
    }

    pub fn comparisons_for_artifact(
        &mut self,
        artifact_id: i64,
    ) -> Result<Vec<CaseComparison>, String> {
        let artifact = self.artifact_by_id(artifact_id)?;
        let comparisons = self.list_comparisons(artifact.case_id)?;

        Ok(comparisons
            .into_iter()
            .filter(|comparison| {
                comparison
                    .members
                    .iter()
                    .any(|member| member.artifact_id == artifact_id)
            })
            .collect())
    }

    pub fn delete_comparison(&mut self, comparison_id: i64) -> Result<(), String> {
        let comparison = self.comparison_by_id(comparison_id)?;
        let now = unix_time_millis()?;
        let transaction = self.connection.transaction().map_err(|error| {
            format!("Не удалось начать удаление comparison #{comparison_id}: {error}")
        })?;

        transaction
            .execute(
                "DELETE FROM case_comparisons WHERE id = ?1",
                params![comparison_id],
            )
            .map_err(|error| format!("Не удалось удалить comparison #{comparison_id}: {error}"))?;

        transaction
            .execute(
                r#"
                UPDATE investigation_cases
                SET updated_at_ms = ?2
                WHERE id = ?1
                "#,
                params![comparison.case_id, now],
            )
            .map_err(|error| {
                format!("Comparison удалён, но не удалось обновить case timestamp: {error}")
            })?;

        transaction.commit().map_err(|error| {
            format!("Не удалось завершить удаление comparison #{comparison_id}: {error}")
        })
    }

    pub fn delete_artifact(
        &mut self,
        artifact_id: i64,
        delete_comparisons: bool,
    ) -> Result<(), String> {
        let artifact = self.artifact_by_id(artifact_id)?;
        let comparisons = self.comparisons_for_artifact(artifact_id)?;

        if !comparisons.is_empty() && !delete_comparisons {
            let ids = comparisons
                .iter()
                .map(|comparison| format!("#{}", comparison.id))
                .collect::<Vec<_>>()
                .join(", ");

            return Err(format!(
                "Artifact #{artifact_id} используется в comparisons: {ids}. Удалите их отдельно или повторите удаление с явным подтверждением удаления comparison history."
            ));
        }

        let path = self.resolve_relative_path(&artifact.relative_path)?;
        let staged_path = if path.exists() {
            let parent = path
                .parent()
                .ok_or("Artifact path не имеет parent directory")?;
            let stage = parent.join(format!(
                ".pgxray-artifact-{artifact_id}.deleting-{}",
                unix_time_millis()?,
            ));

            fs::rename(&path, &stage).map_err(|error| {
                format!(
                    "Не удалось подготовить artifact-файл {} к удалению: {error}",
                    path.display(),
                )
            })?;

            Some(stage)
        } else {
            None
        };

        let now = unix_time_millis()?;
        let database_result = (|| -> Result<(), String> {
            let transaction = self.connection.transaction().map_err(|error| {
                format!("Не удалось начать удаление artifact #{artifact_id}: {error}")
            })?;

            if delete_comparisons {
                transaction
                    .execute(
                        r#"
                        DELETE FROM case_comparisons
                        WHERE id IN (
                            SELECT comparison_id
                            FROM case_comparison_members
                            WHERE artifact_id = ?1
                        )
                        "#,
                        params![artifact_id],
                    )
                    .map_err(|error| {
                        format!(
                            "Не удалось удалить comparison history artifact #{artifact_id}: {error}"
                        )
                    })?;
            }

            transaction
                .execute(
                    "DELETE FROM case_artifacts WHERE id = ?1",
                    params![artifact_id],
                )
                .map_err(|error| format!("Не удалось удалить artifact #{artifact_id}: {error}"))?;

            transaction
                .execute(
                    r#"
                    UPDATE investigation_cases
                    SET updated_at_ms = ?2
                    WHERE id = ?1
                    "#,
                    params![artifact.case_id, now],
                )
                .map_err(|error| {
                    format!("Artifact удалён, но не удалось обновить case timestamp: {error}")
                })?;

            transaction.commit().map_err(|error| {
                format!("Не удалось завершить удаление artifact #{artifact_id}: {error}")
            })?;

            Ok(())
        })();

        if let Err(error) = database_result {
            if let Some(stage) = &staged_path {
                if let Err(restore_error) = fs::rename(stage, &path) {
                    return Err(format!(
                        "{error}. Дополнительно не удалось вернуть artifact-файл {} -> {}: {restore_error}",
                        stage.display(),
                        path.display(),
                    ));
                }
            }

            return Err(error);
        }

        if let Some(stage) = staged_path {
            fs::remove_file(&stage).map_err(|error| {
                format!(
                    "Artifact metadata удалены, но временный файл {} не удалось удалить: {error}",
                    stage.display(),
                )
            })?;
        }

        Ok(())
    }

    pub fn connection_inbox_path(
        &mut self,
        connection_id: i64,
        connection_name: &str,
    ) -> Result<PathBuf, String> {
        let workspace = self.ensure_connection_workspace(connection_id, connection_name)?;

        Ok(PathBuf::from(workspace.inbox_path))
    }

    pub fn case_inbox_path(&mut self, case_id: i64, connection_id: i64) -> Result<PathBuf, String> {
        let case = self.case_by_id(case_id)?;

        if case.connection_id != connection_id {
            return Err("Case принадлежит другому connection".into());
        }

        Ok(PathBuf::from(case.inbox_path))
    }

    pub fn create_text_artifact(
        &mut self,
        case_id: i64,
        file_name: &str,
        content: &str,
        kind: &str,
    ) -> Result<CaseArtifact, String> {
        if content.len() > MAX_EDITABLE_ARTIFACT_BYTES {
            return Err(format!(
                "Файл слишком большой для встроенного редактора: максимум {MAX_EDITABLE_ARTIFACT_BYTES} bytes"
            ));
        }

        let case = self.case_by_id(case_id)?;
        let file_name = validate_editable_artifact_file_name(file_name)?;
        let kind = validate_editable_artifact_kind(kind)?;
        let directory = PathBuf::from(&case.source_path);

        create_private_directory(&directory)?;

        let target = directory.join(&file_name);

        if target.exists() {
            return Err(format!(
                "Файл {file_name} уже существует в source этого case. Откройте его на редактирование."
            ));
        }

        write_new_private_file(&target, content.as_bytes())?;

        let relative = relative_case_path(case.connection_id, case_id, "source", &file_name);

        register_new_file(&target, || {
            self.register_artifact(
                case_id,
                kind,
                "sensitive",
                &file_name,
                &relative,
                media_type_for_path(&target),
            )
        })
    }

    pub fn update_text_artifact(
        &mut self,
        artifact_id: i64,
        content: &str,
    ) -> Result<CaseArtifact, String> {
        if content.len() > MAX_EDITABLE_ARTIFACT_BYTES {
            return Err(format!(
                "Файл слишком большой для встроенного редактора: максимум {MAX_EDITABLE_ARTIFACT_BYTES} bytes"
            ));
        }

        let artifact = self.artifact_by_id(artifact_id)?;

        if artifact.visibility != "sensitive" {
            return Err("Редактировать можно только локальный sensitive artifact".into());
        }

        if !matches!(
            artifact.kind.as_str(),
            "sql" | "text" | "source" | "import" | "explain"
        ) {
            return Err("Generated evidence artifact нельзя изменять встроенным редактором".into());
        }

        let case = self.case_by_id(artifact.case_id)?;
        let path = self.artifact_path(artifact_id)?;
        let source_dir = PathBuf::from(&case.source_path);

        if path.parent() != Some(source_dir.as_path()) {
            return Err(
                "Встроенный редактор изменяет только source artifacts. Inbox, share и generated artifacts остаются read-only."
                    .into(),
            );
        }

        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Case artifact недоступен для редактирования: {error}"))?;

        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Редактировать можно только обычный source artifact, не symlink".into());
        }

        let staged = path.with_file_name(format!(
            ".artifact-{artifact_id}.updated-{}",
            unix_time_millis()?,
        ));
        write_new_private_file(&staged, content.as_bytes())?;
        let backup = path.with_file_name(format!(
            ".artifact-{artifact_id}.backup-{}",
            unix_time_millis()?,
        ));
        if let Err(error) = fs::rename(&path, &backup) {
            let _ = fs::remove_file(&staged);
            return Err(format!("Не удалось подготовить исходный artifact: {error}"));
        }
        if let Err(error) = fs::rename(&staged, &path) {
            let _ = fs::rename(&backup, &path);
            let _ = fs::remove_file(&staged);
            return Err(format!("Не удалось заменить artifact: {error}"));
        }
        let result = self.register_artifact(
            artifact.case_id,
            &artifact.kind,
            &artifact.visibility,
            &artifact.original_name,
            &artifact.relative_path,
            media_type_for_path(&path),
        );
        match result {
            Ok(artifact) => {
                fs::remove_file(&backup).map_err(|error| {
                    format!("Artifact сохранён, но не удалось удалить backup: {error}")
                })?;
                Ok(artifact)
            }
            Err(error) => {
                let _ = fs::remove_file(&path);
                fs::rename(&backup, &path).map_err(|restore_error| {
                    format!("{error}; невозможно восстановить artifact: {restore_error}")
                })?;
                Err(error)
            }
        }
    }

    pub fn import_file(
        &mut self,
        case_id: i64,
        source_path: &Path,
        kind: &str,
    ) -> Result<CaseArtifact, String> {
        let case = self.case_by_id(case_id)?;

        let source_path = source_path
            .canonicalize()
            .map_err(|error| format!("Не удалось открыть импортируемый файл: {error}"))?;

        let metadata = fs::metadata(&source_path)
            .map_err(|error| format!("Не удалось прочитать импортируемый файл: {error}"))?;

        if !metadata.is_file() {
            return Err("Импортировать можно только обычный файл".into());
        }

        let file_name = source_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Имя файла не является корректным UTF-8")?;

        let target_dir = PathBuf::from(&case.source_path);

        create_private_directory(&target_dir)?;

        let target = unique_destination(&target_dir, file_name);

        // Private permissions are applied at creation, not after copying content.
        let mut source_file = fs::File::open(&source_path)
            .map_err(|error| format!("Не удалось открыть импортируемый файл: {error}"))?;
        ensure_no_symlink_components(&target)?;
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
            .map_err(|error| format!("Не удалось создать импортированный файл: {error}"))?;
        if let Err(error) = std::io::copy(&mut source_file, &mut destination) {
            drop(destination);
            let _ = fs::remove_file(&target);
            return Err(format!(
                "Не удалось скопировать файл в pgXray case: {error}"
            ));
        }
        drop(destination);

        let relative = relative_case_path(
            case.connection_id,
            case_id,
            "source",
            target
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("Некорректное имя импортированного файла")?,
        );

        register_new_file(&target, || {
            self.register_artifact(
                case_id,
                kind,
                "sensitive",
                file_name,
                &relative,
                media_type_for_path(&target),
            )
        })
    }

    pub fn write_evidence_artifact(
        &mut self,
        case_id: i64,
        file_name: &str,
        content: &str,
        kind: &str,
    ) -> Result<CaseArtifact, String> {
        let case = self.case_by_id(case_id)?;
        let directory = PathBuf::from(&case.source_path);

        create_private_directory(&directory)?;

        let safe_name = sanitize_file_name(file_name);
        let target = unique_destination(&directory, &safe_name);

        write_new_private_file(&target, content.as_bytes())?;

        let target_name = target
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Некорректное имя evidence artifact")?;

        let relative = relative_case_path(case.connection_id, case_id, "source", target_name);

        register_new_file(&target, || {
            self.register_artifact(
                case_id,
                kind,
                "sensitive",
                target_name,
                &relative,
                Some("application/json".into()),
            )
        })
    }

    pub fn scan_case_inbox(&mut self, case_id: i64) -> Result<Vec<CaseArtifact>, String> {
        let case = self.case_by_id(case_id)?;

        let inbox = PathBuf::from(&case.inbox_path);

        create_private_directory(&inbox)?;

        let entries = fs::read_dir(&inbox)
            .map_err(|error| format!("Не удалось прочитать case inbox: {error}"))?;

        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Не удалось прочитать файл case inbox: {error}"))?;

            let path = entry.path();

            // Do not follow links imported or placed into inbox by another process.
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("Не удалось прочитать metadata inbox-файла: {error}"))?;

            if !metadata.is_file() {
                continue;
            }

            let file_name = entry.file_name();

            let Some(file_name) = file_name.to_str() else {
                continue;
            };

            let relative = relative_case_path(case.connection_id, case_id, "inbox", file_name);

            // Upsert on every scan so a file opened by psql via \o can grow
            // from 0 bytes and the UI receives the current size.
            self.register_artifact(
                case_id,
                "inbox",
                "sensitive",
                file_name,
                &relative,
                media_type_for_path(&path),
            )?;
        }

        self.list_artifacts(case_id)
    }

    pub fn list_artifacts(&mut self, case_id: i64) -> Result<Vec<CaseArtifact>, String> {
        self.case_by_id(case_id)?;

        let mut statement = self
            .connection
            .prepare(
                r#"
                    SELECT
                        id,
                        case_id,
                        kind,
                        visibility,
                        original_name,
                        relative_path,
                        media_type,
                        size_bytes,
                        created_at_ms,
                        modified_at_ms
                    FROM case_artifacts
                    WHERE case_id = ?1
                    ORDER BY id DESC
                    "#,
            )
            .map_err(|error| format!("Не удалось подготовить список artifacts: {error}"))?;

        let rows = statement
            .query_map(params![case_id,], artifact_row)
            .map_err(|error| format!("Не удалось прочитать artifacts: {error}"))?;

        let mut result = Vec::new();

        for row in rows {
            result.push(row.map_err(|error| format!("Повреждена строка artifact: {error}"))?);
        }

        Ok(result)
    }

    pub fn artifact_by_id(&mut self, artifact_id: i64) -> Result<CaseArtifact, String> {
        validate_positive_id(artifact_id, "artifact")?;

        self.connection
            .query_row(
                r#"
                SELECT
                    id,
                    case_id,
                    kind,
                    visibility,
                    original_name,
                    relative_path,
                    media_type,
                    size_bytes,
                    created_at_ms,
                    modified_at_ms
                FROM case_artifacts
                WHERE id = ?1
                "#,
                params![artifact_id,],
                artifact_row,
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать artifact #{artifact_id}: {error}"))?
            .ok_or_else(|| format!("Artifact #{artifact_id} не найден"))
    }

    pub fn artifact_path(&mut self, artifact_id: i64) -> Result<PathBuf, String> {
        let artifact = self.artifact_by_id(artifact_id)?;

        self.resolve_relative_path(&artifact.relative_path)
    }

    pub fn read_text_artifact(
        &mut self,
        artifact_id: i64,
        max_bytes: usize,
    ) -> Result<String, String> {
        let path = self.artifact_path(artifact_id)?;

        ensure_regular_file_without_symlinks(&path)?;
        let metadata =
            fs::metadata(&path).map_err(|error| format!("Artifact-файл недоступен: {error}"))?;

        let size =
            usize::try_from(metadata.len()).map_err(|_| "Artifact слишком большой".to_owned())?;

        if size > max_bytes {
            return Err(
                format!(
                    "Artifact слишком большой для текстового просмотра: {size} bytes, максимум {max_bytes}"
                ),
            );
        }

        fs::read_to_string(&path)
            .map_err(|error| format!("Artifact не является UTF-8 текстом или не читается: {error}"))
    }

    fn write_share_artifact(
        &mut self,
        case_id: i64,
        source_artifact_id: i64,
        content: &str,
        suffix: &str,
        kind: &str,
    ) -> Result<CaseArtifact, String> {
        let case = self.case_by_id(case_id)?;

        let source = self.artifact_by_id(source_artifact_id)?;

        if source.case_id != case_id {
            return Err("Source artifact принадлежит другому case".into());
        }

        if suffix != "anonymized" || kind != "anonymized" {
            return Err("Share artifact требует подтверждённого anonymized workflow".into());
        }

        let share_dir = PathBuf::from(&case.share_path);

        create_private_directory(&share_dir)?;

        // One derived share artifact per source artifact + suffix.
        // Reopening the same EXPLAIN updates the same file/DB row instead of
        // producing j.anonymized-1.anonymized-2... duplicates.
        let file_name = anonymized_name(&source.original_name, suffix, source_artifact_id);

        let target = share_dir.join(&file_name);

        let staged = share_dir.join(format!(
            ".share-{source_artifact_id}-{}.staging",
            unix_time_millis()?,
        ));
        write_new_private_file(&staged, content.as_bytes())?;
        let backup = share_dir.join(format!(".share-{source_artifact_id}.backup"));
        if fs::symlink_metadata(&backup).is_ok() {
            let _ = fs::remove_file(&staged);
            return Err("Найден старый backup share artifact; требуется восстановление".into());
        }
        let had_previous = fs::symlink_metadata(&target).is_ok();
        if had_previous {
            if let Err(error) = ensure_regular_file_without_symlinks(&target) {
                let _ = fs::remove_file(&staged);
                return Err(error);
            }
            if let Err(error) = fs::rename(&target, &backup) {
                let _ = fs::remove_file(&staged);
                return Err(format!("Не удалось подготовить share artifact: {error}"));
            }
        }
        if let Err(error) = fs::rename(&staged, &target) {
            if had_previous {
                let _ = fs::rename(&backup, &target);
            }
            let _ = fs::remove_file(&staged);
            return Err(format!("Не удалось заменить share artifact: {error}"));
        }

        let relative = relative_case_path(case.connection_id, case_id, "share", &file_name);
        let result = self.register_artifact(
            case_id,
            kind,
            "shareable",
            &file_name,
            &relative,
            media_type_for_path(&target),
        );
        match result {
            Ok(artifact) => {
                // Caller saves lineage in the same SQLite SAVEPOINT. Backup is
                // cleaned by write_share_artifact_with_lineage after commit.
                Ok(artifact)
            }
            Err(error) => {
                let _ = fs::remove_file(&target);
                if had_previous {
                    fs::rename(&backup, &target).map_err(|restore_error| {
                        format!("{error}; невозможно восстановить share artifact: {restore_error}")
                    })?;
                }
                Err(error)
            }
        }
    }

    pub fn set_scope_result_privacy(
        &mut self,
        scope_id: i64,
        result_privacy: ResultPrivacyConfig,
    ) -> Result<PrivacyScope, String> {
        self.scope_by_id(scope_id)?;
        let result_privacy = result_privacy.normalized()?;
        let current = self.scope_result_privacy(scope_id)?;

        if current == result_privacy {
            return self.scope_by_id(scope_id);
        }

        let result_privacy_json = serde_json::to_string(&result_privacy)
            .map_err(|error| format!("Не удалось сериализовать result privacy policy: {error}"))?;
        let _ = self.scope_revision(scope_id)?;
        let now = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                    UPDATE privacy_scope_state
                    SET
                        result_privacy_json = ?2,
                        revision = revision + 1,
                        updated_at_ms = ?3
                    WHERE scope_id = ?1
                "#,
                params![scope_id, result_privacy_json, now],
            )
            .map_err(|error| {
                format!("Не удалось сохранить result privacy policy scope #{scope_id}: {error}")
            })?;

        self.scope_by_id(scope_id)
    }

    pub fn scope_revision(&mut self, scope_id: i64) -> Result<i64, String> {
        self.scope_by_id(scope_id)?;

        let now = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                INSERT OR IGNORE INTO privacy_scope_state (
                    scope_id,
                    revision,
                    updated_at_ms
                )
                VALUES (?1, 1, ?2)
                "#,
                params![scope_id, now,],
            )
            .map_err(|error| {
                format!("Не удалось инициализировать revision privacy scope #{scope_id}: {error}")
            })?;

        self.connection
            .query_row(
                r#"
                SELECT revision
                FROM privacy_scope_state
                WHERE scope_id = ?1
                "#,
                params![scope_id,],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                format!("Не удалось прочитать revision privacy scope #{scope_id}: {error}")
            })
    }

    pub fn bump_scope_revision(&mut self, scope_id: i64) -> Result<i64, String> {
        let _ = self.scope_revision(scope_id)?;

        let now = unix_time_millis()?;

        self.connection
            .execute(
                r#"
                UPDATE privacy_scope_state
                SET
                    revision = revision + 1,
                    updated_at_ms = ?2
                WHERE scope_id = ?1
                "#,
                params![scope_id, now,],
            )
            .map_err(|error| {
                format!("Не удалось увеличить revision privacy scope #{scope_id}: {error}")
            })?;

        self.scope_revision(scope_id)
    }

    pub fn set_artifact_role(
        &mut self,
        artifact_id: i64,
        role: Option<ArtifactRole>,
    ) -> Result<(), String> {
        self.artifact_by_id(artifact_id)?;

        if let Some(role) = role {
            let now = unix_time_millis()?;

            self.connection
                .execute(
                    r#"
                    INSERT INTO case_artifact_roles (
                        artifact_id,
                        role,
                        updated_at_ms
                    )
                    VALUES (?1, ?2, ?3)
                    ON CONFLICT(artifact_id)
                    DO UPDATE SET
                        role = excluded.role,
                        updated_at_ms = excluded.updated_at_ms
                    "#,
                    params![artifact_id, role.as_str(), now],
                )
                .map_err(|error| {
                    format!("Не удалось сохранить role artifact #{artifact_id}: {error}")
                })?;
        } else {
            self.connection
                .execute(
                    r#"
                    DELETE FROM case_artifact_roles
                    WHERE artifact_id = ?1
                    "#,
                    params![artifact_id],
                )
                .map_err(|error| {
                    format!("Не удалось очистить role artifact #{artifact_id}: {error}")
                })?;
        }

        Ok(())
    }

    pub fn artifact_role(&mut self, artifact_id: i64) -> Result<Option<ArtifactRole>, String> {
        self.artifact_by_id(artifact_id)?;

        let role = self
            .connection
            .query_row(
                r#"
                SELECT role
                FROM case_artifact_roles
                WHERE artifact_id = ?1
                "#,
                params![artifact_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| {
                format!("Не удалось прочитать role artifact #{artifact_id}: {error}")
            })?;

        role.map(|role| ArtifactRole::parse(&role)).transpose()
    }

    pub fn list_comparisons(&mut self, case_id: i64) -> Result<Vec<CaseComparison>, String> {
        self.case_by_id(case_id)?;

        let rows = {
            let mut statement = self
                .connection
                .prepare(
                    r#"
                    SELECT
                        id,
                        case_id,
                        kind,
                        created_at_ms,
                        result_json
                    FROM case_comparisons
                    WHERE case_id = ?1
                    ORDER BY created_at_ms DESC, id DESC
                    "#,
                )
                .map_err(|error| format!("Не удалось подготовить список comparisons: {error}"))?;

            let rows = statement
                .query_map(params![case_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(|error| format!("Не удалось прочитать comparisons: {error}"))?;

            let mut result = Vec::new();

            for row in rows {
                result.push(row.map_err(|error| format!("Повреждена строка comparison: {error}"))?);
            }

            result
        };

        let mut result = Vec::with_capacity(rows.len());
        let mut hash_cache = HashMap::new();

        for (id, case_id, kind, created_at_ms, result_json) in rows {
            result.push(CaseComparison {
                id,
                case_id,
                kind: ComparisonKind::from_stored(&kind)?,
                created_at_ms,
                result: parse_comparison_result(id, result_json)?,
                members: self.comparison_members_with_cache(id, &mut hash_cache)?,
            });
        }

        Ok(result)
    }

    pub fn record_comparison(
        &mut self,
        case_id: i64,
        kind: ComparisonKind,
        members: &[(i64, ComparisonPurpose, ComparisonSide)],
    ) -> Result<CaseComparison, String> {
        self.record_comparison_json(case_id, kind, members, None)
    }

    pub fn record_comparison_with_result<T: Serialize>(
        &mut self,
        case_id: i64,
        kind: ComparisonKind,
        members: &[(i64, ComparisonPurpose, ComparisonSide)],
        result: &T,
    ) -> Result<CaseComparison, String> {
        let result_json = serde_json::to_string(result)
            .map_err(|error| format!("Не удалось сериализовать результат comparison: {error}"))?;

        self.record_comparison_json(case_id, kind, members, Some(result_json))
    }

    fn record_comparison_json(
        &mut self,
        case_id: i64,
        kind: ComparisonKind,
        members: &[(i64, ComparisonPurpose, ComparisonSide)],
        result_json: Option<String>,
    ) -> Result<CaseComparison, String> {
        self.case_by_id(case_id)?;

        if !matches!(members.len(), 2 | 4) {
            return Err("Comparison должен содержать 2 или 4 members".into());
        }

        let mut keys = Vec::with_capacity(members.len());
        let mut purposes = Vec::new();
        let mut prepared = Vec::with_capacity(members.len());

        for (artifact_id, purpose, side) in members {
            let artifact = self.artifact_by_id(*artifact_id)?;

            if artifact.case_id != case_id {
                return Err(format!("Artifact #{artifact_id} принадлежит другому case"));
            }

            let key = (*purpose, *side);

            if keys.contains(&key) {
                return Err(format!(
                    "Comparison уже содержит member purpose={}, side={}",
                    purpose.as_str(),
                    side.as_str(),
                ));
            }

            keys.push(key);

            if !purposes.contains(purpose) {
                purposes.push(*purpose);
            }

            prepared.push((
                *artifact_id,
                *purpose,
                *side,
                self.artifact_sha256(*artifact_id)?,
            ));
        }

        for purpose in purposes {
            let baseline = members.iter().find(|(_, current_purpose, side)| {
                *current_purpose == purpose && *side == ComparisonSide::Baseline
            });
            let candidate = members.iter().find(|(_, current_purpose, side)| {
                *current_purpose == purpose && *side == ComparisonSide::Candidate
            });

            let (Some(baseline), Some(candidate)) = (baseline, candidate) else {
                return Err(format!(
                    "Comparison purpose={} требует baseline и candidate",
                    purpose.as_str(),
                ));
            };

            if baseline.0 == candidate.0 {
                return Err(format!(
                    "Comparison purpose={} требует разные baseline и candidate artifacts",
                    purpose.as_str(),
                ));
            }
        }

        let has_query = members
            .iter()
            .any(|(_, purpose, _)| *purpose == ComparisonPurpose::Query);
        let has_plan = members
            .iter()
            .any(|(_, purpose, _)| *purpose == ComparisonPurpose::Plan);
        let shape_is_valid = match kind {
            ComparisonKind::SavedPlan | ComparisonKind::LegacyPlan => {
                has_plan && !has_query && members.len() == 2
            }
            ComparisonKind::QueryResult | ComparisonKind::LegacyQuery => {
                has_query && !has_plan && members.len() == 2
            }
            ComparisonKind::QueryExplain | ComparisonKind::QueryAnalyze => {
                has_query && has_plan && members.len() == 4
            }
        };

        if !shape_is_valid {
            return Err(format!(
                "Некорректный набор members для comparison kind={}",
                kind.as_str(),
            ));
        }

        let now = unix_time_millis()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Не удалось начать сохранение comparison: {error}"))?;

        transaction
            .execute(
                r#"
                INSERT INTO case_comparisons (
                    case_id,
                    kind,
                    result_json,
                    created_at_ms
                )
                VALUES (?1, ?2, ?3, ?4)
                "#,
                params![case_id, kind.as_str(), result_json, now],
            )
            .map_err(|error| format!("Не удалось сохранить comparison: {error}"))?;

        let comparison_id = transaction.last_insert_rowid();

        for (artifact_id, purpose, side, artifact_sha256) in prepared {
            transaction
                .execute(
                    r#"
                    INSERT INTO case_comparison_members (
                        comparison_id,
                        artifact_id,
                        purpose,
                        side,
                        artifact_sha256
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    params![
                        comparison_id,
                        artifact_id,
                        purpose.as_str(),
                        side.as_str(),
                        artifact_sha256,
                    ],
                )
                .map_err(|error| {
                    format!("Не удалось сохранить member comparison #{comparison_id}: {error}")
                })?;
        }

        transaction
            .commit()
            .map_err(|error| format!("Не удалось завершить сохранение comparison: {error}"))?;

        self.comparison_by_id(comparison_id)
    }

    fn comparison_by_id(&mut self, comparison_id: i64) -> Result<CaseComparison, String> {
        validate_positive_id(comparison_id, "comparison")?;

        let row = self
            .connection
            .query_row(
                r#"
                SELECT
                    id,
                    case_id,
                    kind,
                    created_at_ms,
                    result_json
                FROM case_comparisons
                WHERE id = ?1
                "#,
                params![comparison_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать comparison #{comparison_id}: {error}"))?
            .ok_or_else(|| format!("Comparison #{comparison_id} не найден"))?;

        Ok(CaseComparison {
            id: row.0,
            case_id: row.1,
            kind: ComparisonKind::from_stored(&row.2)?,
            created_at_ms: row.3,
            result: parse_comparison_result(comparison_id, row.4)?,
            members: self.comparison_members(comparison_id)?,
        })
    }

    fn comparison_members(
        &mut self,
        comparison_id: i64,
    ) -> Result<Vec<CaseComparisonMember>, String> {
        let mut hash_cache = HashMap::new();
        self.comparison_members_with_cache(comparison_id, &mut hash_cache)
    }

    fn comparison_members_with_cache(
        &mut self,
        comparison_id: i64,
        hash_cache: &mut HashMap<i64, Option<String>>,
    ) -> Result<Vec<CaseComparisonMember>, String> {
        let rows = {
            let mut statement = self
                .connection
                .prepare(
                    r#"
                    SELECT
                        artifact_id,
                        purpose,
                        side,
                        artifact_sha256
                    FROM case_comparison_members
                    WHERE comparison_id = ?1
                    ORDER BY
                        CASE purpose WHEN 'query' THEN 0 ELSE 1 END,
                        CASE side WHEN 'baseline' THEN 0 ELSE 1 END
                    "#,
                )
                .map_err(|error| {
                    format!("Не удалось подготовить members comparison #{comparison_id}: {error}")
                })?;

            let rows = statement
                .query_map(params![comparison_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|error| {
                    format!("Не удалось прочитать members comparison #{comparison_id}: {error}")
                })?;

            let mut result = Vec::new();

            for row in rows {
                result.push(row.map_err(|error| {
                    format!("Повреждён member comparison #{comparison_id}: {error}")
                })?);
            }

            result
        };

        let mut result = Vec::with_capacity(rows.len());

        for (artifact_id, purpose, side, artifact_sha256) in rows {
            let current_hash = match hash_cache.get(&artifact_id) {
                Some(current_hash) => current_hash.clone(),
                None => {
                    let current_hash = self.artifact_sha256(artifact_id).ok();
                    hash_cache.insert(artifact_id, current_hash.clone());
                    current_hash
                }
            };
            let stale = current_hash.as_deref() != Some(artifact_sha256.as_str());

            result.push(CaseComparisonMember {
                artifact_id,
                purpose: ComparisonPurpose::from_stored(&purpose)?,
                side: ComparisonSide::from_stored(&side)?,
                artifact_sha256,
                stale,
            });
        }

        Ok(result)
    }

    pub fn artifact_derivation(
        &mut self,
        artifact_id: i64,
    ) -> Result<Option<ArtifactDerivation>, String> {
        self.connection
            .query_row(
                r#"
                SELECT
                    artifact_id,
                    source_artifact_id,
                    source_sha256,
                    scope_id,
                    mapping_revision,
                    engine_version,
                    transform_mode,
                    created_at_ms,
                    updated_at_ms
                FROM artifact_derivations
                WHERE artifact_id = ?1
                "#,
                params![artifact_id,],
                derivation_row,
            )
            .optional()
            .map_err(|error| {
                format!("Не удалось прочитать derivation artifact #{artifact_id}: {error}")
            })
    }

    pub fn artifact_sha256(&mut self, artifact_id: i64) -> Result<String, String> {
        let path = self.artifact_path(artifact_id)?;

        ensure_regular_file_without_symlinks(&path)?;
        sha256_file(&path)
    }

    pub fn case_context(&mut self, case_id: i64) -> Result<CaseContext, String> {
        let case = self.case_by_id(case_id)?;
        let artifacts = self.list_artifacts(case_id)?;
        let mut result = Vec::with_capacity(artifacts.len());

        for artifact in artifacts {
            let role = self.artifact_role(artifact.id)?;
            let derivation = self.artifact_derivation(artifact.id)?;
            let stale = match &derivation {
                Some(derivation) => self.derivation_is_stale(derivation)?,
                None => false,
            };

            result.push(CaseArtifactContext {
                artifact,
                role,
                derivation,
                stale,
            });
        }

        let comparisons = self.list_comparisons(case_id)?;
        let interventions = self.list_case_interventions(case_id)?;
        let applied_changes = self.list_case_workspace_changes(case_id)?;
        let change_impacts = self.list_case_change_impacts(case_id)?;
        let timeline = self.case_timeline(
            &case,
            &result,
            &interventions,
            &applied_changes,
            &change_impacts,
        )?;
        let fingerprint = self.case_incident_fingerprint(case_id)?;
        let (similar_incidents, similar_search_total_candidates, similar_search_scanned) =
            self.similar_incidents(&case, &fingerprint)?;

        Ok(CaseContext {
            case,
            artifacts: result,
            comparisons,
            interventions,
            applied_changes,
            change_impacts,
            timeline,
            fingerprint,
            similar_incidents,
            similar_search_total_candidates,
            similar_search_scanned,
        })
    }

    fn case_timeline(
        &mut self,
        case: &InvestigationCase,
        artifacts: &[CaseArtifactContext],
        interventions: &[CaseIntervention],
        applied_changes: &[WorkspaceChange],
        change_impacts: &[CaseChangeImpact],
    ) -> Result<Vec<CaseTimelineEvent>, String> {
        let mut timeline = Vec::new();

        if let Some(detected_at_ms) = case.detected_at_ms {
            timeline.push(CaseTimelineEvent {
                id: format!("case:{}:detected", case.id),
                occurred_at_ms: detected_at_ms,
                source: "case".into(),
                kind: "incident_detected".into(),
                title: "Incident detected".into(),
                detail: case
                    .trigger
                    .as_ref()
                    .map(|trigger| format!("trigger: {trigger}"))
                    .unwrap_or_default(),
                evidence_level: "observed".into(),
                navigation: None,
            });
        }

        timeline.push(CaseTimelineEvent {
            id: format!("case:{}:created", case.id),
            occurred_at_ms: case.created_at_ms,
            source: "case".into(),
            kind: "case_created".into(),
            title: "Case created".into(),
            detail: case.title.clone(),
            evidence_level: "observed".into(),
            navigation: None,
        });

        for artifact_context in artifacts {
            let artifact = &artifact_context.artifact;
            let navigation = artifact_navigation(artifact, artifact_context.role);
            timeline.push(CaseTimelineEvent {
                id: format!("artifact:{}", artifact.id),
                occurred_at_ms: artifact.created_at_ms,
                source: "evidence".into(),
                kind: artifact.kind.clone(),
                title: format!("Evidence saved · {}", artifact.original_name),
                detail: format!("{} · {} bytes", artifact.kind, artifact.size_bytes),
                evidence_level: "observed".into(),
                navigation,
            });

            if artifact.kind == "what_changed" {
                if let Ok(value) = self.read_json_artifact_value(artifact.id, 8 * 1024 * 1024) {
                    append_what_changed_timeline(&mut timeline, artifact.id, &value);
                }
            } else if artifact.kind == "lock_incident" {
                if let Ok(value) = self.read_json_artifact_value(artifact.id, 8 * 1024 * 1024) {
                    append_lock_incident_timeline(&mut timeline, artifact.id, &value);
                }
            }
        }

        for intervention in interventions {
            timeline.push(CaseTimelineEvent {
                id: format!("intervention:{}", intervention.id),
                occurred_at_ms: intervention.occurred_at_ms,
                source: if applied_changes
                    .iter()
                    .any(|change| change.intervention_id == Some(intervention.id))
                {
                    "policy_apply".into()
                } else {
                    "manual".into()
                },
                kind: "intervention".into(),
                title: intervention.title.clone(),
                detail: if intervention.note.is_empty() {
                    intervention.kind.clone()
                } else {
                    format!("{} · {}", intervention.kind, intervention.note)
                },
                evidence_level: "observed".into(),
                navigation: None,
            });
        }

        for impact in change_impacts {
            let assessment = impact
                .result
                .get("assessment")
                .and_then(|value| value.get("label"))
                .and_then(Value::as_str)
                .unwrap_or("Before/after verification saved");
            timeline.push(CaseTimelineEvent {
                id: format!("change-impact:{}", impact.id),
                occurred_at_ms: impact.after_to_ms,
                source: "verification".into(),
                kind: "change_impact".into(),
                title: format!("Change Impact · {assessment}"),
                detail: format!(
                    "before {}..{} → after {}..{}; causal attribution is not guaranteed",
                    impact.before_from_ms,
                    impact.before_to_ms,
                    impact.after_from_ms,
                    impact.after_to_ms,
                ),
                evidence_level: "associated_evidence".into(),
                navigation: None,
            });
        }

        timeline.sort_by(|left, right| {
            left.occurred_at_ms
                .cmp(&right.occurred_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(timeline)
    }

    fn read_json_artifact_value(
        &mut self,
        artifact_id: i64,
        max_bytes: u64,
    ) -> Result<Value, String> {
        let artifact = self.artifact_by_id(artifact_id)?;
        if artifact.size_bytes < 0 || artifact.size_bytes as u64 > max_bytes {
            return Err(format!(
                "Artifact #{artifact_id} слишком большой для investigation metadata"
            ));
        }
        let text = self.read_text_artifact(artifact_id, max_bytes as usize)?;
        serde_json::from_str(&text).map_err(|error| {
            format!("Artifact #{artifact_id} не содержит корректный JSON: {error}")
        })
    }

    fn case_incident_fingerprint(&mut self, case_id: i64) -> Result<IncidentFingerprint, String> {
        let artifacts = self.list_artifacts(case_id)?;
        let mut fingerprint = FingerprintBuilder::default();

        for artifact in artifacts
            .into_iter()
            .filter(|artifact| artifact.kind == "lock_incident" || artifact.kind == "what_changed")
            .take(12)
        {
            let Ok(value) = self.read_json_artifact_value(artifact.id, 8 * 1024 * 1024) else {
                continue;
            };
            if artifact.kind == "lock_incident" {
                fingerprint.absorb_lock_incident(&value);
            } else {
                fingerprint.absorb_what_changed(&value);
            }
        }

        Ok(fingerprint.finish())
    }

    fn similar_incidents(
        &mut self,
        current: &InvestigationCase,
        fingerprint: &IncidentFingerprint,
    ) -> Result<(Vec<SimilarIncidentMatch>, usize, usize), String> {
        if current.kind != "incident" || fingerprint_is_empty(fingerprint) {
            return Ok((Vec::new(), 0, 0));
        }

        let candidates = self.list_cases(current.connection_id)?;
        let eligible: Vec<_> = candidates
            .into_iter()
            .filter(|candidate| candidate.id != current.id && candidate.kind == "incident")
            .collect();
        let total = eligible.len();
        let mut scanned = 0;
        let mut matches = Vec::new();
        // Hard cap bounds local artifact parsing. UI displays actual scan coverage.
        for candidate in eligible.into_iter().take(20) {
            scanned += 1;
            let candidate_fingerprint = self.case_incident_fingerprint(candidate.id)?;
            let reasons = fingerprint_matches(fingerprint, &candidate_fingerprint);
            if reasons.is_empty() {
                continue;
            }
            let score = fingerprint_match_score(&reasons);
            matches.push(SimilarIncidentMatch {
                score,
                case_id: candidate.id,
                title: candidate.title,
                detected_at_ms: candidate.detected_at_ms,
                matches: reasons,
            });
        }
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.detected_at_ms.cmp(&left.detected_at_ms))
                .then_with(|| right.case_id.cmp(&left.case_id))
        });
        matches.truncate(8);
        Ok((matches, total, scanned))
    }

    pub fn current_share_for_source(
        &mut self,
        source_artifact_id: i64,
        transform_mode: &str,
    ) -> Result<Option<ShareArtifactState>, String> {
        let derivation = self
            .connection
            .query_row(
                r#"
                    SELECT
                        d.artifact_id,
                        d.source_artifact_id,
                        d.source_sha256,
                        d.scope_id,
                        d.mapping_revision,
                        d.engine_version,
                        d.transform_mode,
                        d.created_at_ms,
                        d.updated_at_ms
                    FROM artifact_derivations AS d
                    JOIN case_artifacts AS a
                      ON a.id = d.artifact_id
                    WHERE d.source_artifact_id = ?1
                      AND d.transform_mode = ?2
                    ORDER BY d.updated_at_ms DESC, d.artifact_id DESC
                    LIMIT 1
                    "#,
                params![source_artifact_id, transform_mode,],
                derivation_row,
            )
            .optional()
            .map_err(|error| {
                format!("Не удалось найти share artifact source #{source_artifact_id}: {error}")
            })?;

        let Some(derivation) = derivation else {
            return Ok(None);
        };

        let artifact = self.artifact_by_id(derivation.artifact_id)?;

        let stale = self.derivation_is_stale(&derivation)?;

        Ok(Some(ShareArtifactState {
            artifact,
            derivation,
            stale,
        }))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "existing call contract; refactor separately with callers"
    )]
    pub fn write_share_artifact_with_lineage(
        &mut self,
        case_id: i64,
        source_artifact_id: i64,
        content: &str,
        suffix: &str,
        kind: &str,
        scope_id: i64,
        mapping_revision: i64,
        engine_version: &str,
        transform_mode: &str,
    ) -> Result<ShareArtifactState, String> {
        let case = self.case_by_id(case_id)?;
        if case.scope_id != scope_id {
            return Err("Share artifact использует чужой privacy scope".into());
        }
        if !matches!(transform_mode, "text" | "sql" | "explain" | "result")
            || engine_version.trim().is_empty()
            || mapping_revision != self.scope_revision(scope_id)?
        {
            return Err("Share artifact имеет недействительный privacy lineage".into());
        }
        let source = self.artifact_by_id(source_artifact_id)?;
        if source.case_id != case_id {
            return Err("Source artifact принадлежит другому case".into());
        }
        let file_name = anonymized_name(&source.original_name, suffix, source_artifact_id);
        let target = PathBuf::from(&case.share_path).join(file_name);
        let backup =
            PathBuf::from(&case.share_path).join(format!(".share-{source_artifact_id}.backup"));
        // Savepoint makes artifact metadata and its derivation one SQLite unit.
        self.connection
            .execute_batch("SAVEPOINT share_artifact_write")
            .map_err(|error| format!("Не удалось начать сохранение share artifact: {error}"))?;
        let mut file_replaced = false;
        let result = (|| -> Result<ShareArtifactState, String> {
            let source_sha256 = self.artifact_sha256(source_artifact_id)?;
            let artifact =
                self.write_share_artifact(case_id, source_artifact_id, content, suffix, kind)?;
            file_replaced = true;
            let now = unix_time_millis()?;

            self.connection
                .execute(
                    r#"
                INSERT INTO artifact_derivations (
                    artifact_id,
                    source_artifact_id,
                    source_sha256,
                    scope_id,
                    mapping_revision,
                    engine_version,
                    transform_mode,
                    created_at_ms,
                    updated_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                ON CONFLICT(artifact_id)
                DO UPDATE SET
                    source_artifact_id = excluded.source_artifact_id,
                    source_sha256 = excluded.source_sha256,
                    scope_id = excluded.scope_id,
                    mapping_revision = excluded.mapping_revision,
                    engine_version = excluded.engine_version,
                    transform_mode = excluded.transform_mode,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                    params![
                        artifact.id,
                        source_artifact_id,
                        source_sha256,
                        scope_id,
                        mapping_revision,
                        engine_version,
                        transform_mode,
                        now,
                    ],
                )
                .map_err(|error| {
                    format!(
                        "Не удалось сохранить lineage share artifact #{}: {error}",
                        artifact.id,
                    )
                })?;

            let derivation = self
                .artifact_derivation(artifact.id)?
                .ok_or("Lineage share artifact не сохранился")?;

            Ok(ShareArtifactState {
                artifact,
                derivation,
                stale: false,
            })
        })();
        let result = result.and_then(|state| {
            self.connection
                .execute_batch("RELEASE SAVEPOINT share_artifact_write")
                .map_err(|error| format!("Не удалось завершить share artifact: {error}"))?;
            Ok(state)
        });
        match result {
            Ok(state) => {
                if backup.exists() {
                    fs::remove_file(&backup).map_err(|error| {
                        format!("Share artifact сохранён, но не удалось очистить backup: {error}")
                    })?;
                }
                Ok(state)
            }
            Err(error) => {
                let _ = self.connection.execute_batch(
                    "ROLLBACK TO SAVEPOINT share_artifact_write; RELEASE SAVEPOINT share_artifact_write"
                );
                if backup.exists() {
                    let _ = fs::remove_file(&target);
                    fs::rename(&backup, &target).map_err(|restore_error| {
                        format!(
                            "{error}; не удалось восстановить предыдущий share: {restore_error}"
                        )
                    })?;
                } else if file_replaced {
                    let _ = fs::remove_file(&target);
                }
                Err(error)
            }
        }
    }

    pub fn write_generated_artifact(
        &mut self,
        case_id: i64,
        file_name: &str,
        content: &str,
        kind: &str,
        role: Option<ArtifactRole>,
    ) -> Result<CaseArtifact, String> {
        let case = self.case_by_id(case_id)?;

        let directory = PathBuf::from(&case.comparisons_path);

        create_private_directory(&directory)?;

        let safe_name = sanitize_file_name(file_name);

        let target = unique_destination(&directory, &safe_name);

        write_new_private_file(&target, content.as_bytes())?;

        let target_name = target
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Некорректное имя generated artifact")?;

        let relative = relative_case_path(case.connection_id, case_id, "comparisons", target_name);

        self.connection
            .execute_batch("SAVEPOINT generated_artifact_write")
            .map_err(|error| {
                format!("Не удалось начать регистрацию generated artifact: {error}")
            })?;
        let result = (|| -> Result<CaseArtifact, String> {
            let artifact = self.register_artifact(
                case_id,
                kind,
                "sensitive",
                target_name,
                &relative,
                media_type_for_path(&target),
            )?;
            self.set_artifact_role(artifact.id, role)?;
            Ok(artifact)
        })();
        match result {
            Ok(artifact) => {
                self.connection
                    .execute_batch("RELEASE SAVEPOINT generated_artifact_write")
                    .map_err(|error| format!("Не удалось завершить generated artifact: {error}"))?;
                Ok(artifact)
            }
            Err(error) => {
                let _ = self.connection.execute_batch(
                    "ROLLBACK TO SAVEPOINT generated_artifact_write; RELEASE SAVEPOINT generated_artifact_write"
                );
                register_new_file(&target, || Err::<CaseArtifact, String>(error))
            }
        }
    }

    pub fn artifact_parent_path(&mut self, artifact_id: i64) -> Result<PathBuf, String> {
        let path = self.artifact_path(artifact_id)?;

        path.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "Artifact path не имеет parent directory".into())
    }

    fn derivation_is_stale(&mut self, derivation: &ArtifactDerivation) -> Result<bool, String> {
        let current_hash = self.artifact_sha256(derivation.source_artifact_id)?;

        let current_revision = self.scope_revision(derivation.scope_id)?;

        Ok(current_hash != derivation.source_sha256
            || current_revision != derivation.mapping_revision
            || derivation.engine_version != env!("CARGO_PKG_VERSION"))
    }

    fn ensure_default_scope(
        &mut self,
        connection_id: i64,
        connection_name: &str,
    ) -> Result<PrivacyScope, String> {
        let existing = self
            .connection
            .query_row(
                r#"
                    SELECT
                        id,
                        name,
                        connection_id,
                        is_default,
                        created_at_ms
                    FROM privacy_scopes
                    WHERE connection_id = ?1
                      AND is_default = 1
                    LIMIT 1
                    "#,
                params![connection_id,],
                scope_row,
            )
            .optional()
            .map_err(|error| format!("Не удалось прочитать default privacy scope: {error}"))?;

        if let Some(scope) = existing {
            return self.attach_scope_path(scope);
        }

        let created_at_ms = unix_time_millis()?;

        let name = format!(
            "Connection: {}",
            validate_name(connection_name, "connection name",)?,
        );

        self.connection
            .execute(
                r#"
                INSERT INTO privacy_scopes (
                    name,
                    connection_id,
                    is_default,
                    created_at_ms
                )
                VALUES (?1, ?2, 1, ?3)
                "#,
                params![name, connection_id, created_at_ms,],
            )
            .map_err(|error| format!("Не удалось создать default privacy scope: {error}"))?;

        let id = self.connection.last_insert_rowid();

        self.scope_by_id(id)
    }

    fn scope_result_privacy(&self, scope_id: i64) -> Result<ResultPrivacyConfig, String> {
        let value = self
            .connection
            .query_row(
                "SELECT result_privacy_json FROM privacy_scope_state WHERE scope_id = ?1",
                params![scope_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| {
                format!("Не удалось прочитать result privacy policy scope #{scope_id}: {error}")
            })?;

        let Some(value) = value else {
            return Ok(ResultPrivacyConfig::default());
        };

        serde_json::from_str::<ResultPrivacyConfig>(&value)
            .map_err(|error| format!("Повреждён result privacy policy scope #{scope_id}: {error}"))?
            .normalized()
    }

    fn attach_scope_path(&self, mut scope: PrivacyScope) -> Result<PrivacyScope, String> {
        let directory = self.scope_root(scope.connection_id, scope.id);

        create_private_directory(&directory)?;

        scope.mapping_path = path_string(&directory.join("mapping.json"));
        scope.result_privacy = self.scope_result_privacy(scope.id)?;

        Ok(scope)
    }

    fn attach_case_paths(&self, mut case: InvestigationCase) -> Result<InvestigationCase, String> {
        let root = self.case_root(case.connection_id, case.id);

        let inbox = root.join("inbox");

        let source = root.join("source");

        let share = root.join("share");

        let analyses = root.join("analyses");

        let comparisons = root.join("comparisons");

        for directory in [&root, &inbox, &source, &share, &analyses, &comparisons] {
            create_private_directory(directory)?;
        }

        case.root_path = path_string(&root);

        case.inbox_path = path_string(&inbox);

        case.source_path = path_string(&source);

        case.share_path = path_string(&share);

        case.analyses_path = path_string(&analyses);

        case.comparisons_path = path_string(&comparisons);

        Ok(case)
    }

    fn ensure_case_directories(&self, connection_id: i64, case_id: i64) -> Result<(), String> {
        let root = self.case_root(connection_id, case_id);

        for directory in [
            root.clone(),
            root.join("inbox"),
            root.join("source"),
            root.join("share"),
            root.join("analyses"),
            root.join("comparisons"),
        ] {
            create_private_directory(&directory)?;
        }

        Ok(())
    }

    fn register_artifact(
        &mut self,
        case_id: i64,
        kind: &str,
        visibility: &str,
        original_name: &str,
        relative_path: &str,
        media_type: Option<String>,
    ) -> Result<CaseArtifact, String> {
        validate_relative_path(relative_path)?;

        let absolute = self.resolve_relative_path(relative_path)?;

        ensure_regular_file_without_symlinks(&absolute)?;
        let metadata = fs::metadata(&absolute)
            .map_err(|error| format!("Не удалось прочитать artifact metadata: {error}"))?;

        let size_bytes =
            i64::try_from(metadata.len()).map_err(|_| "Artifact слишком большой".to_owned())?;

        let modified_at_ms = file_modified_at_millis(&metadata);

        let now = unix_time_millis()?;

        self.connection
            .execute_batch("SAVEPOINT artifact_registration")
            .map_err(|error| format!("Не удалось начать регистрацию artifact: {error}"))?;
        let registration = (|| -> Result<CaseArtifact, String> {
            self.connection
                .execute(
                    r#"
                INSERT INTO case_artifacts (
                    case_id,
                    kind,
                    visibility,
                    original_name,
                    relative_path,
                    media_type,
                    size_bytes,
                    created_at_ms,
                    modified_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(case_id, relative_path)
                DO UPDATE SET
                    kind = excluded.kind,
                    visibility = excluded.visibility,
                    original_name = excluded.original_name,
                    media_type = excluded.media_type,
                    size_bytes = excluded.size_bytes,
                    modified_at_ms = excluded.modified_at_ms
                "#,
                    params![
                        case_id,
                        kind,
                        visibility,
                        original_name,
                        relative_path,
                        media_type,
                        size_bytes,
                        now,
                        modified_at_ms,
                    ],
                )
                .map_err(|error| format!("Не удалось зарегистрировать artifact: {error}"))?;

            self.connection
                .execute(
                    r#"
                UPDATE investigation_cases
                SET updated_at_ms = ?2
                WHERE id = ?1
                "#,
                    params![case_id, now,],
                )
                .map_err(|error| format!("Не удалось обновить case timestamp: {error}"))?;

            self.connection
                .query_row(
                    r#"
                SELECT
                    id,
                    case_id,
                    kind,
                    visibility,
                    original_name,
                    relative_path,
                    media_type,
                    size_bytes,
                    created_at_ms,
                    modified_at_ms
                FROM case_artifacts
                WHERE case_id = ?1
                  AND relative_path = ?2
                "#,
                    params![case_id, relative_path,],
                    artifact_row,
                )
                .map_err(|error| format!("Не удалось перечитать artifact: {error}"))
        })();
        match registration {
            Ok(artifact) => {
                self.connection
                    .execute_batch("RELEASE SAVEPOINT artifact_registration")
                    .map_err(|error| {
                        format!("Не удалось завершить регистрацию artifact: {error}")
                    })?;
                Ok(artifact)
            }
            Err(error) => {
                let _ = self.connection.execute_batch(
                    "ROLLBACK TO SAVEPOINT artifact_registration; RELEASE SAVEPOINT artifact_registration"
                );
                Err(error)
            }
        }
    }

    fn resolve_relative_path(&self, relative_path: &str) -> Result<PathBuf, String> {
        validate_relative_path(relative_path)?;

        let path = self.root_dir.join(relative_path);
        ensure_no_symlink_components(&path)?;
        Ok(path)
    }

    fn connection_root(&self, connection_id: i64) -> PathBuf {
        self.root_dir
            .join("connections")
            .join(format!("connection-{connection_id}"))
    }

    fn scope_root(&self, connection_id: Option<i64>, scope_id: i64) -> PathBuf {
        match connection_id {
            Some(connection_id) => self
                .connection_root(connection_id)
                .join("privacy")
                .join(format!("scope-{scope_id}")),

            None => self
                .root_dir
                .join("scopes")
                .join(format!("scope-{scope_id}")),
        }
    }

    fn case_root(&self, connection_id: i64, case_id: i64) -> PathBuf {
        self.connection_root(connection_id)
            .join("cases")
            .join(format!("case-{case_id}"))
    }
}

#[derive(Default)]
struct FingerprintBuilder {
    wait_event_counts: HashMap<String, usize>,
    root_blocker_queryids: BTreeSet<String>,
    root_blocker_applications: BTreeSet<String>,
    relation_oids: BTreeSet<i64>,
    regressed_queryids: BTreeSet<String>,
    changed_settings: BTreeSet<String>,
    autovacuum_relation_oids: BTreeSet<i64>,
}

impl FingerprintBuilder {
    fn absorb_lock_incident(&mut self, value: &Value) {
        let payload = value.get("payload").unwrap_or(value);
        let Some(snapshot) = payload.get("lock_snapshot") else {
            return;
        };
        let backends = snapshot
            .get("backends")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let waits = snapshot
            .get("waits")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        let mut root_pids = BTreeSet::new();
        let mut blocked_pids = BTreeSet::new();
        for wait in waits {
            if let Some(pid) = wait.get("blocked_pid").and_then(Value::as_i64) {
                blocked_pids.insert(pid);
            }
            for pid in wait
                .get("root_blocker_pids")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
            {
                root_pids.insert(pid);
            }
        }

        for backend in backends {
            let Some(pid) = backend.get("pid").and_then(Value::as_i64) else {
                continue;
            };
            if blocked_pids.contains(&pid) {
                let wait_type = backend
                    .get("wait_event_type")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let wait = backend
                    .get("wait_event")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let label = match (wait_type.is_empty(), wait.is_empty()) {
                    (false, false) => format!("{wait_type}/{wait}"),
                    (false, true) => wait_type.to_owned(),
                    (true, false) => wait.to_owned(),
                    (true, true) => String::new(),
                };
                if !label.is_empty() {
                    *self.wait_event_counts.entry(label).or_insert(0) += 1;
                }
            }
            if root_pids.contains(&pid) {
                if let Some(queryid) = backend.get("query_id").and_then(Value::as_str) {
                    if !queryid.is_empty() {
                        self.root_blocker_queryids.insert(queryid.to_owned());
                    }
                }
                if let Some(application) = backend.get("application_name").and_then(Value::as_str) {
                    if !application.is_empty() {
                        self.root_blocker_applications
                            .insert(application.to_owned());
                    }
                }
            }
        }

        for relation_oid in snapshot
            .get("locks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|lock| lock.get("relation_oid").and_then(Value::as_i64))
        {
            self.relation_oids.insert(relation_oid);
        }
    }

    fn absorb_what_changed(&mut self, value: &Value) {
        let payload = value.get("payload").unwrap_or(value);

        for change in payload
            .get("pgss")
            .and_then(|value| value.get("statement_changes"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let regressed = change
                .get("metrics")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|metric| {
                    let id = metric.get("id").and_then(Value::as_str).unwrap_or("");
                    let direction = metric
                        .get("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    matches!(
                        id,
                        "exec_ms_per_call" | "temp_writes_per_call" | "temp_reads_per_call"
                    ) && direction == "up"
                });
            if regressed {
                if let Some(queryid) = change.get("queryid").and_then(Value::as_str) {
                    self.regressed_queryids.insert(queryid.to_owned());
                }
            }
        }

        for change in payload
            .get("settings")
            .and_then(|value| value.get("changes"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(name) = change
                .get("change")
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
            {
                self.changed_settings.insert(name.to_owned());
            }
        }

        for change in payload
            .get("autovacuum")
            .and_then(|value| value.get("relation_changes"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(oid) = change.get("relation_oid").and_then(Value::as_i64) {
                self.autovacuum_relation_oids.insert(oid);
                self.relation_oids.insert(oid);
            }
        }

        for oid in payload
            .get("storage")
            .and_then(|value| value.get("relation_size_changes"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|change| change.get("relation_oid").and_then(Value::as_i64))
        {
            self.relation_oids.insert(oid);
        }
    }

    fn finish(self) -> IncidentFingerprint {
        let dominant_wait_event = self
            .wait_event_counts
            .into_iter()
            .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
            .map(|(value, _)| value);
        IncidentFingerprint {
            dominant_wait_event,
            root_blocker_queryids: self.root_blocker_queryids.into_iter().collect(),
            root_blocker_applications: self.root_blocker_applications.into_iter().collect(),
            relation_oids: self.relation_oids.into_iter().collect(),
            regressed_queryids: self.regressed_queryids.into_iter().collect(),
            changed_settings: self.changed_settings.into_iter().collect(),
            autovacuum_relation_oids: self.autovacuum_relation_oids.into_iter().collect(),
        }
    }
}

fn artifact_navigation(
    artifact: &CaseArtifact,
    role: Option<ArtifactRole>,
) -> Option<CaseNavigationTarget> {
    let kind = if role == Some(ArtifactRole::Plan) || artifact.kind == "explain" {
        "artifact"
    } else if artifact.kind == "lock_incident" {
        "locks"
    } else if artifact.kind == "what_changed" {
        "changes"
    } else {
        return None;
    };

    Some(CaseNavigationTarget {
        kind: kind.into(),
        artifact_id: Some(artifact.id),
        pid: None,
        userid: None,
        dbid: None,
        queryid: None,
        toplevel: None,
        snapshot_id: None,
        relation_oid: None,
    })
}

fn append_what_changed_timeline(
    timeline: &mut Vec<CaseTimelineEvent>,
    artifact_id: i64,
    value: &Value,
) {
    let payload = value.get("payload").unwrap_or(value);
    let Some(events) = payload.get("timeline").and_then(Value::as_array) else {
        return;
    };

    for (index, event) in events.iter().enumerate() {
        let Some(occurred_at_ms) = event.get("occurred_at_ms").and_then(Value::as_i64) else {
            continue;
        };
        let source = event
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("what_changed");
        let refs = event
            .get("evidence_refs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut navigation = navigation_from_evidence_refs(refs, artifact_id);
        if navigation.is_none() && source == "pg_settings" {
            navigation = Some(CaseNavigationTarget {
                kind: "settings".into(),
                artifact_id: Some(artifact_id),
                pid: None,
                userid: None,
                dbid: None,
                queryid: None,
                toplevel: None,
                snapshot_id: None,
                relation_oid: None,
            });
        }

        timeline.push(CaseTimelineEvent {
            id: format!("artifact:{artifact_id}:what-changed:{index}"),
            occurred_at_ms,
            source: source.to_owned(),
            kind: event
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("change")
                .to_owned(),
            title: event
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Observed change")
                .to_owned(),
            detail: event
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            evidence_level: event
                .get("evidence_level")
                .and_then(Value::as_str)
                .unwrap_or("observed")
                .to_owned(),
            navigation,
        });
    }
}

fn append_lock_incident_timeline(
    timeline: &mut Vec<CaseTimelineEvent>,
    artifact_id: i64,
    value: &Value,
) {
    let captured_at_ms = value
        .get("captured_at_ms")
        .and_then(Value::as_i64)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("lock_snapshot"))
                .and_then(|snapshot| snapshot.get("captured_at_ms"))
                .and_then(Value::as_i64)
        });
    let Some(captured_at_ms) = captured_at_ms else {
        return;
    };
    let payload = value.get("payload").unwrap_or(value);
    let Some(snapshot) = payload.get("lock_snapshot") else {
        return;
    };
    let backends = snapshot
        .get("backends")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut root_pids = BTreeSet::new();
    for wait in snapshot
        .get("waits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for pid in wait
            .get("root_blocker_pids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
        {
            root_pids.insert(pid);
        }
    }

    for pid in root_pids {
        let backend = backends
            .iter()
            .find(|backend| backend.get("pid").and_then(Value::as_i64) == Some(pid));
        let application = backend
            .and_then(|backend| backend.get("application_name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown application");
        let queryid = backend
            .and_then(|backend| backend.get("query_id"))
            .and_then(Value::as_str);
        let wait = backend
            .and_then(|backend| backend.get("wait_event"))
            .and_then(Value::as_str);
        let mut details = vec![format!("application: {application}")];
        if let Some(queryid) = queryid {
            details.push(format!("queryid: {queryid}"));
        }
        if let Some(wait) = wait {
            details.push(format!("wait: {wait}"));
        }
        timeline.push(CaseTimelineEvent {
            id: format!("artifact:{artifact_id}:root-blocker:{pid}"),
            occurred_at_ms: captured_at_ms,
            source: "locks_waits".into(),
            kind: "root_blocker".into(),
            title: format!("Root blocker PID {pid}"),
            detail: details.join(" · "),
            evidence_level: "observed".into(),
            navigation: Some(CaseNavigationTarget {
                kind: "pid".into(),
                artifact_id: Some(artifact_id),
                pid: Some(pid),
                userid: None,
                dbid: None,
                queryid: None,
                toplevel: None,
                snapshot_id: None,
                relation_oid: None,
            }),
        });
    }
}

fn navigation_from_evidence_refs(refs: &[Value], artifact_id: i64) -> Option<CaseNavigationTarget> {
    let mut query_identity = None;
    let mut snapshot_id = None;
    let mut relation_oid = None;

    for reference in refs.iter().filter_map(Value::as_str) {
        if let Some(rest) = reference.strip_prefix("pgss_statement:") {
            let mut parts = rest.split(':');
            let userid = parts.next().and_then(|value| value.parse::<u32>().ok());
            let dbid = parts.next().and_then(|value| value.parse::<u32>().ok());
            let queryid = parts.next().map(str::to_owned);
            let toplevel = parts.next().and_then(|value| value.parse::<bool>().ok());
            if let Some(queryid) = queryid {
                query_identity = Some((userid, dbid, queryid, toplevel));
            }
        } else if let Some(rest) = reference.strip_prefix("pgss_snapshot:") {
            snapshot_id = rest.parse::<i64>().ok().or(snapshot_id);
        }

        for prefix in ["relation:", "autovacuum_relation:", "xid_relation:"] {
            if let Some(rest) = reference.strip_prefix(prefix) {
                relation_oid = rest.parse::<i64>().ok().or(relation_oid);
            }
        }
    }

    if let Some((userid, dbid, queryid, toplevel)) = query_identity {
        return Some(CaseNavigationTarget {
            kind: "query".into(),
            artifact_id: Some(artifact_id),
            pid: None,
            userid,
            dbid,
            queryid: Some(queryid),
            toplevel,
            snapshot_id,
            relation_oid: None,
        });
    }

    relation_oid.map(|relation_oid| CaseNavigationTarget {
        kind: "relation".into(),
        artifact_id: Some(artifact_id),
        pid: None,
        userid: None,
        dbid: None,
        queryid: None,
        toplevel: None,
        snapshot_id: None,
        relation_oid: Some(relation_oid),
    })
}

fn fingerprint_is_empty(fingerprint: &IncidentFingerprint) -> bool {
    fingerprint.dominant_wait_event.is_none()
        && fingerprint.root_blocker_queryids.is_empty()
        && fingerprint.root_blocker_applications.is_empty()
        && fingerprint.relation_oids.is_empty()
        && fingerprint.regressed_queryids.is_empty()
        && fingerprint.changed_settings.is_empty()
        && fingerprint.autovacuum_relation_oids.is_empty()
}

fn fingerprint_matches(left: &IncidentFingerprint, right: &IncidentFingerprint) -> Vec<String> {
    let mut matches = Vec::new();
    if left.dominant_wait_event.is_some() && left.dominant_wait_event == right.dominant_wait_event {
        matches.push(format!(
            "same dominant wait: {}",
            left.dominant_wait_event.as_deref().unwrap_or_default(),
        ));
    }
    append_string_intersection_matches(
        &mut matches,
        "same root blocker queryid",
        &left.root_blocker_queryids,
        &right.root_blocker_queryids,
    );
    append_string_intersection_matches(
        &mut matches,
        "same root blocker application",
        &left.root_blocker_applications,
        &right.root_blocker_applications,
    );
    append_i64_intersection_matches(
        &mut matches,
        "same relation OID",
        &left.relation_oids,
        &right.relation_oids,
    );
    append_string_intersection_matches(
        &mut matches,
        "same regressed queryid",
        &left.regressed_queryids,
        &right.regressed_queryids,
    );
    append_string_intersection_matches(
        &mut matches,
        "same changed setting",
        &left.changed_settings,
        &right.changed_settings,
    );
    append_i64_intersection_matches(
        &mut matches,
        "same autovacuum relation OID",
        &left.autovacuum_relation_oids,
        &right.autovacuum_relation_oids,
    );
    matches
}

/// Heuristic score with per-category caps, avoiding domination by many weak
/// OID matches. It must not be presented as probability of a common root cause.
fn fingerprint_match_score(reasons: &[String]) -> u32 {
    const GROUPS: [(&str, u32, u32); 7] = [
        ("same root blocker queryid:", 5, 2),
        ("same regressed queryid:", 5, 2),
        ("same dominant wait:", 3, 1),
        ("same changed setting:", 3, 2),
        ("same autovacuum relation OID:", 2, 2),
        ("same relation OID:", 1, 3),
        ("same root blocker application:", 1, 2),
    ];
    GROUPS
        .iter()
        .map(|(prefix, weight, cap)| {
            let count = reasons
                .iter()
                .filter(|value| value.starts_with(prefix))
                .count() as u32;
            weight * count.min(*cap)
        })
        .sum()
}

fn append_string_intersection_matches(
    target: &mut Vec<String>,
    label: &str,
    left: &[String],
    right: &[String],
) {
    let right = right.iter().map(String::as_str).collect::<BTreeSet<_>>();
    for value in left
        .iter()
        .map(String::as_str)
        .filter(|value| right.contains(value))
    {
        target.push(format!("{label}: {value}"));
    }
}

fn append_i64_intersection_matches(
    target: &mut Vec<String>,
    label: &str,
    left: &[i64],
    right: &[i64],
) {
    let right = right.iter().copied().collect::<BTreeSet<_>>();
    for value in left.iter().copied().filter(|value| right.contains(value)) {
        target.push(format!("{label}: {value}"));
    }
}

fn scope_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrivacyScope> {
    Ok(PrivacyScope {
        id: row.get(0)?,
        name: row.get(1)?,
        connection_id: row.get(2)?,
        is_default: row.get::<_, i64>(3)? != 0,
        mapping_path: String::new(),
        result_privacy: ResultPrivacyConfig::default(),
        created_at_ms: row.get(4)?,
    })
}

fn case_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InvestigationCase> {
    Ok(InvestigationCase {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        scope_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: row.get(5)?,
        kind: row.get(6)?,
        trigger: row.get(7)?,
        detected_at_ms: row.get(8)?,
        root_path: String::new(),
        inbox_path: String::new(),
        source_path: String::new(),
        share_path: String::new(),
        analyses_path: String::new(),
        comparisons_path: String::new(),
        created_at_ms: row.get(9)?,
        updated_at_ms: row.get(10)?,
    })
}

fn workspace_change_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceChange> {
    let before_json: String = row.get(9)?;
    let after_json: String = row.get(10)?;
    let target_json: String = row.get(16)?;
    let parse = |text: &str| {
        serde_json::from_str(text).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    };
    Ok(WorkspaceChange {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        case_id: row.get(2)?,
        intervention_id: row.get(3)?,
        kind: row.get(4)?,
        actor: row.get(5)?,
        relation_oid: row.get(6)?,
        snapshot_id: row.get(7)?,
        parameter: row.get(8)?,
        target: parse(&target_json)?,
        before: parse(&before_json)?,
        after: parse(&after_json)?,
        status: row.get(11)?,
        started_at_ms: row.get(12)?,
        finished_at_ms: row.get(13)?,
        post_snapshot_id: row.get(14)?,
        note: row.get(15)?,
    })
}

fn intervention_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CaseIntervention> {
    Ok(CaseIntervention {
        id: row.get(0)?,
        case_id: row.get(1)?,
        occurred_at_ms: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        note: row.get(5)?,
        created_at_ms: row.get(6)?,
    })
}

fn change_impact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CaseChangeImpact> {
    let result_json = row.get::<_, String>(7)?;
    let result = serde_json::from_str::<Value>(&result_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(CaseChangeImpact {
        id: row.get(0)?,
        case_id: row.get(1)?,
        intervention_id: row.get(2)?,
        before_from_ms: row.get(3)?,
        before_to_ms: row.get(4)?,
        after_from_ms: row.get(5)?,
        after_to_ms: row.get(6)?,
        result,
        created_at_ms: row.get(8)?,
    })
}

fn artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CaseArtifact> {
    Ok(CaseArtifact {
        id: row.get(0)?,
        case_id: row.get(1)?,
        kind: row.get(2)?,
        visibility: row.get(3)?,
        original_name: row.get(4)?,
        relative_path: row.get(5)?,
        media_type: row.get(6)?,
        size_bytes: row.get(7)?,
        created_at_ms: row.get(8)?,
        modified_at_ms: row.get(9)?,
    })
}

fn derivation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactDerivation> {
    Ok(ArtifactDerivation {
        artifact_id: row.get(0)?,
        source_artifact_id: row.get(1)?,
        source_sha256: row.get(2)?,
        scope_id: row.get(3)?,
        mapping_revision: row.get(4)?,
        engine_version: row.get(5)?,
        transform_mode: row.get(6)?,
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Не удалось открыть {} для SHA-256: {error}", path.display(),))?;

    let mut hasher = Sha256::new();

    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            format!(
                "Не удалось прочитать {} для SHA-256: {error}",
                path.display(),
            )
        })?;

        if read == 0 {
            break;
        }

        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize(),))
}

fn sanitize_file_name(value: &str) -> String {
    let value = Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact.txt");

    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "artifact.txt".into()
    } else {
        sanitized
    }
}

/// Additive migration for installs that ran the earlier journal schema.
fn ensure_workspace_change_target_column(connection: &Connection) -> Result<(), String> {
    let has_target = {
        let mut stmt = connection
            .prepare("PRAGMA table_info(workspace_changes)")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| e.to_string())?;
        let names: rusqlite::Result<Vec<String>> = rows.collect();
        names
            .map_err(|e| e.to_string())?
            .iter()
            .any(|name| name == "target_json")
    };
    if !has_target {
        connection
            .execute(
                "ALTER TABLE workspace_changes ADD COLUMN target_json TEXT NOT NULL DEFAULT '{}'",
                [],
            )
            .map_err(|e| format!("Не удалось мигрировать журнал изменений: {e}"))?;
    }
    connection.execute(
        "INSERT OR IGNORE INTO workspace_migrations (name, applied_at_ms) VALUES ('workspace-change-target-v1', ?1)",
        [unix_time_millis()?],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

fn ensure_privacy_scope_result_policy_column(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(privacy_scope_state)")
        .map_err(|error| format!("Не удалось проверить schema privacy_scope_state: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Не удалось прочитать schema privacy_scope_state: {error}"))?;
    let mut exists = false;

    for column in columns {
        if column.map_err(|error| format!("Повреждена schema privacy_scope_state: {error}"))?
            == "result_privacy_json"
        {
            exists = true;
            break;
        }
    }
    drop(statement);

    if !exists {
        connection
            .execute(
                r#"ALTER TABLE privacy_scope_state
                    ADD COLUMN result_privacy_json TEXT NOT NULL
                    DEFAULT '{"default":"auto","fields":{}}'"#,
                [],
            )
            .map_err(|error| {
                format!("Не удалось добавить privacy_scope_state.result_privacy_json: {error}")
            })?;
    }

    Ok(())
}

fn ensure_case_comparison_schema(
    connection: &mut Connection,
    root_dir: &Path,
) -> Result<(), String> {
    let has_artifact_sha256 = {
        let mut statement = connection
            .prepare("PRAGMA table_info(case_comparison_members)")
            .map_err(|error| format!("Не удалось проверить schema comparison members: {error}"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| format!("Не удалось прочитать schema comparison members: {error}"))?;
        let mut found = false;

        for column in columns {
            if column.map_err(|error| format!("Повреждена schema comparison members: {error}"))?
                == "artifact_sha256"
            {
                found = true;
                break;
            }
        }

        found
    };

    if has_artifact_sha256 {
        return Ok(());
    }

    // The first comparison-model draft created these tables without a SHA-256
    // snapshot and with weaker foreign-key/uniqueness constraints. CREATE TABLE
    // IF NOT EXISTS cannot upgrade that existing schema, so rebuild both tables
    // once while preserving comparison ids and members.
    let comparisons = {
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, case_id, kind, created_at_ms
                FROM case_comparisons
                ORDER BY id
                "#,
            )
            .map_err(|error| format!("Не удалось прочитать старые comparisons: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| format!("Не удалось прочитать старые comparisons: {error}"))?;
        let mut result = Vec::new();

        for row in rows {
            let row = row.map_err(|error| format!("Повреждён старый comparison: {error}"))?;
            ComparisonKind::from_stored(&row.2)?;
            result.push(row);
        }

        result
    };

    let members = {
        let mut statement = connection
            .prepare(
                r#"
                SELECT
                    m.comparison_id,
                    m.artifact_id,
                    m.purpose,
                    m.side,
                    a.relative_path
                FROM case_comparison_members AS m
                JOIN case_artifacts AS a
                  ON a.id = m.artifact_id
                ORDER BY m.comparison_id, m.purpose, m.side
                "#,
            )
            .map_err(|error| format!("Не удалось прочитать старые comparison members: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| format!("Не удалось прочитать старые comparison members: {error}"))?;
        let mut result = Vec::new();

        for row in rows {
            let (comparison_id, artifact_id, purpose, side, relative_path) =
                row.map_err(|error| format!("Повреждён старый comparison member: {error}"))?;
            ComparisonPurpose::from_stored(&purpose)?;
            ComparisonSide::from_stored(&side)?;
            let artifact_sha256 = sha256_file(&root_dir.join(relative_path))
                .unwrap_or_else(|_| UNKNOWN_ARTIFACT_SHA256.to_owned());

            result.push((comparison_id, artifact_id, purpose, side, artifact_sha256));
        }

        result
    };

    let transaction = connection
        .transaction()
        .map_err(|error| format!("Не удалось начать upgrade comparison schema: {error}"))?;

    transaction
        .execute_batch(
            r#"
            ALTER TABLE case_comparison_members
                RENAME TO case_comparison_members_legacy_v1;
            ALTER TABLE case_comparisons
                RENAME TO case_comparisons_legacy_v1;

            CREATE TABLE case_comparisons (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL,
                kind TEXT NOT NULL CHECK(kind IN (
                    'saved_plan',
                    'query_result',
                    'query_explain',
                    'query_analyze',
                    'legacy_query',
                    'legacy_plan'
                )),
                result_json TEXT,
                created_at_ms INTEGER NOT NULL,
                FOREIGN KEY(case_id)
                    REFERENCES investigation_cases(id)
                    ON DELETE CASCADE
            );

            CREATE TABLE case_comparison_members (
                comparison_id INTEGER NOT NULL,
                artifact_id INTEGER NOT NULL,
                purpose TEXT NOT NULL CHECK(purpose IN ('query', 'plan')),
                side TEXT NOT NULL CHECK(side IN ('baseline', 'candidate')),
                artifact_sha256 TEXT NOT NULL CHECK(length(artifact_sha256) = 64),
                PRIMARY KEY(comparison_id, purpose, side),
                UNIQUE(comparison_id, artifact_id, purpose),
                FOREIGN KEY(comparison_id)
                    REFERENCES case_comparisons(id)
                    ON DELETE CASCADE,
                FOREIGN KEY(artifact_id)
                    REFERENCES case_artifacts(id)
                    ON DELETE RESTRICT
            );
            "#,
        )
        .map_err(|error| format!("Не удалось перестроить comparison schema: {error}"))?;

    for (id, case_id, kind, created_at_ms) in comparisons {
        transaction
            .execute(
                r#"
                INSERT INTO case_comparisons (
                    id,
                    case_id,
                    kind,
                    created_at_ms
                )
                VALUES (?1, ?2, ?3, ?4)
                "#,
                params![id, case_id, kind, created_at_ms],
            )
            .map_err(|error| format!("Не удалось перенести comparison #{id}: {error}"))?;
    }

    for (comparison_id, artifact_id, purpose, side, artifact_sha256) in members {
        transaction
            .execute(
                r#"
                INSERT INTO case_comparison_members (
                    comparison_id,
                    artifact_id,
                    purpose,
                    side,
                    artifact_sha256
                )
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![comparison_id, artifact_id, purpose, side, artifact_sha256,],
            )
            .map_err(|error| {
                format!("Не удалось перенести member comparison #{comparison_id}: {error}")
            })?;
    }

    transaction
        .execute_batch(
            r#"
            DROP TABLE case_comparison_members_legacy_v1;
            DROP TABLE case_comparisons_legacy_v1;

            CREATE INDEX idx_case_comparisons_case
                ON case_comparisons(case_id, created_at_ms DESC, id DESC);
            CREATE INDEX idx_case_comparison_members_artifact
                ON case_comparison_members(artifact_id, comparison_id);
            "#,
        )
        .map_err(|error| format!("Не удалось завершить upgrade comparison schema: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("Не удалось зафиксировать upgrade comparison schema: {error}"))?;

    Ok(())
}

fn ensure_case_comparison_result_column(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(case_comparisons)")
        .map_err(|error| format!("Не удалось проверить schema comparisons: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Не удалось прочитать schema comparisons: {error}"))?;
    let mut exists = false;

    for column in columns {
        if column.map_err(|error| format!("Повреждена schema comparisons: {error}"))?
            == "result_json"
        {
            exists = true;
            break;
        }
    }

    drop(statement);

    if !exists {
        connection
            .execute(
                "ALTER TABLE case_comparisons ADD COLUMN result_json TEXT",
                [],
            )
            .map_err(|error| {
                format!("Не удалось добавить case_comparisons.result_json: {error}")
            })?;
    }

    Ok(())
}

fn parse_comparison_result(
    comparison_id: i64,
    result_json: Option<String>,
) -> Result<Option<Value>, String> {
    result_json
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                format!("Повреждён сохранённый результат comparison #{comparison_id}: {error}")
            })
        })
        .transpose()
}

fn migrate_legacy_artifact_roles(
    connection: &mut Connection,
    root_dir: &Path,
) -> Result<(), String> {
    let already_applied = connection
        .query_row(
            "SELECT 1 FROM workspace_migrations WHERE name = ?1",
            params![ARTIFACT_COMPARISON_MIGRATION],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| format!("Не удалось проверить workspace migration: {error}"))?
        .is_some();

    if already_applied {
        return Ok(());
    }

    let legacy_rows = {
        let mut statement = connection
            .prepare(
                r#"
                SELECT
                    a.case_id,
                    r.artifact_id,
                    r.role,
                    r.updated_at_ms,
                    a.relative_path
                FROM case_artifact_roles AS r
                JOIN case_artifacts AS a
                  ON a.id = r.artifact_id
                WHERE r.role IN (
                    'view',
                    'baseline_query',
                    'candidate_query',
                    'baseline_plan',
                    'candidate_plan'
                )
                ORDER BY a.case_id, r.artifact_id
                "#,
            )
            .map_err(|error| format!("Не удалось подготовить legacy artifact roles: {error}"))?;

        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| format!("Не удалось прочитать legacy artifact roles: {error}"))?;

        let mut result = Vec::new();

        for row in rows {
            result.push(row.map_err(|error| format!("Повреждена legacy artifact role: {error}"))?);
        }

        result
    };

    let transaction = connection
        .transaction()
        .map_err(|error| format!("Не удалось начать migration artifact roles: {error}"))?;

    let mut case_ids = Vec::new();

    for (case_id, _, _, _, _) in &legacy_rows {
        if !case_ids.contains(case_id) {
            case_ids.push(*case_id);
        }
    }

    for case_id in case_ids {
        for (baseline_role, candidate_role, purpose, kind) in [
            (
                "baseline_query",
                "candidate_query",
                ComparisonPurpose::Query,
                ComparisonKind::LegacyQuery,
            ),
            (
                "baseline_plan",
                "candidate_plan",
                ComparisonPurpose::Plan,
                ComparisonKind::LegacyPlan,
            ),
        ] {
            let baseline = legacy_rows
                .iter()
                .filter(|(row_case_id, _, role, _, _)| {
                    *row_case_id == case_id && role == baseline_role
                })
                .collect::<Vec<_>>();
            let candidate = legacy_rows
                .iter()
                .filter(|(row_case_id, _, role, _, _)| {
                    *row_case_id == case_id && role == candidate_role
                })
                .collect::<Vec<_>>();

            if baseline.len() != 1 || candidate.len() != 1 {
                continue;
            }

            let baseline_hash = sha256_file(&root_dir.join(&baseline[0].4));
            let candidate_hash = sha256_file(&root_dir.join(&candidate[0].4));
            let (Ok(baseline_hash), Ok(candidate_hash)) = (baseline_hash, candidate_hash) else {
                // Legacy metadata can outlive manually removed files. Normalize the
                // role, but do not invent a historical comparison without a snapshot.
                continue;
            };

            let created_at_ms = baseline[0].3.max(candidate[0].3);

            transaction
                .execute(
                    r#"
                    INSERT INTO case_comparisons (
                        case_id,
                        kind,
                        created_at_ms
                    )
                    VALUES (?1, ?2, ?3)
                    "#,
                    params![case_id, kind.as_str(), created_at_ms],
                )
                .map_err(|error| {
                    format!("Не удалось мигрировать legacy comparison case #{case_id}: {error}")
                })?;

            let comparison_id = transaction.last_insert_rowid();

            for (artifact_id, side, artifact_sha256) in [
                (baseline[0].1, ComparisonSide::Baseline, baseline_hash),
                (candidate[0].1, ComparisonSide::Candidate, candidate_hash),
            ] {
                transaction
                    .execute(
                        r#"
                        INSERT INTO case_comparison_members (
                            comparison_id,
                            artifact_id,
                            purpose,
                            side,
                            artifact_sha256
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5)
                        "#,
                        params![
                            comparison_id,
                            artifact_id,
                            purpose.as_str(),
                            side.as_str(),
                            artifact_sha256,
                        ],
                    )
                    .map_err(|error| {
                        format!(
                            "Не удалось мигрировать legacy comparison member case #{case_id}: {error}"
                        )
                    })?;
            }
        }
    }

    transaction
        .execute(
            r#"
            UPDATE case_artifact_roles
            SET role = CASE role
                WHEN 'view' THEN 'ddl'
                WHEN 'baseline_query' THEN 'query'
                WHEN 'candidate_query' THEN 'query'
                WHEN 'baseline_plan' THEN 'plan'
                WHEN 'candidate_plan' THEN 'plan'
                ELSE role
            END
            WHERE role IN (
                'view',
                'baseline_query',
                'candidate_query',
                'baseline_plan',
                'candidate_plan'
            )
            "#,
            [],
        )
        .map_err(|error| format!("Не удалось нормализовать legacy artifact roles: {error}"))?;

    transaction
        .execute(
            "INSERT INTO workspace_migrations (name, applied_at_ms) VALUES (?1, ?2)",
            params![ARTIFACT_COMPARISON_MIGRATION, unix_time_millis()?],
        )
        .map_err(|error| format!("Не удалось отметить workspace migration: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("Не удалось завершить migration artifact roles: {error}"))?;

    Ok(())
}

fn ensure_investigation_case_description_column(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(investigation_cases)")
        .map_err(|error| format!("Не удалось проверить schema investigation_cases: {error}"))?;

    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Не удалось прочитать schema investigation_cases: {error}"))?;

    let mut exists = false;

    for column in columns {
        if column.map_err(|error| format!("Повреждена schema investigation_cases: {error}"))?
            == "description"
        {
            exists = true;
            break;
        }
    }

    drop(statement);

    if !exists {
        connection
            .execute(
                "ALTER TABLE investigation_cases ADD COLUMN description TEXT NOT NULL DEFAULT ''",
                [],
            )
            .map_err(|error| {
                format!("Не удалось добавить investigation_cases.description: {error}")
            })?;
    }

    Ok(())
}

fn ensure_investigation_case_incident_columns(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(investigation_cases)")
        .map_err(|error| {
            format!("Не удалось проверить incident schema investigation_cases: {error}")
        })?;

    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| {
            format!("Не удалось прочитать incident schema investigation_cases: {error}")
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Повреждена incident schema investigation_cases: {error}"))?;

    drop(statement);

    if !columns.iter().any(|column| column == "kind") {
        connection
            .execute(
                "ALTER TABLE investigation_cases ADD COLUMN kind TEXT NOT NULL DEFAULT 'manual'",
                [],
            )
            .map_err(|error| format!("Не удалось добавить investigation_cases.kind: {error}"))?;
    }

    if !columns.iter().any(|column| column == "trigger") {
        connection
            .execute(
                "ALTER TABLE investigation_cases ADD COLUMN trigger TEXT",
                [],
            )
            .map_err(|error| format!("Не удалось добавить investigation_cases.trigger: {error}"))?;
    }

    if !columns.iter().any(|column| column == "detected_at_ms") {
        connection
            .execute(
                "ALTER TABLE investigation_cases ADD COLUMN detected_at_ms INTEGER",
                [],
            )
            .map_err(|error| {
                format!("Не удалось добавить investigation_cases.detected_at_ms: {error}")
            })?;
    }

    Ok(())
}

fn ensure_case_artifact_modified_at_column(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(case_artifacts)")
        .map_err(|error| format!("Не удалось проверить schema case_artifacts: {error}"))?;

    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Не удалось прочитать schema case_artifacts: {error}"))?;

    let mut exists = false;

    for column in columns {
        if column.map_err(|error| format!("Повреждена schema case_artifacts: {error}"))?
            == "modified_at_ms"
        {
            exists = true;
            break;
        }
    }

    drop(statement);

    if !exists {
        connection
            .execute(
                "ALTER TABLE case_artifacts ADD COLUMN modified_at_ms INTEGER",
                [],
            )
            .map_err(|error| {
                format!("Не удалось добавить case_artifacts.modified_at_ms: {error}")
            })?;
    }

    Ok(())
}

fn backfill_case_artifact_modified_at(
    connection: &Connection,
    root_dir: &Path,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT id, relative_path
            FROM case_artifacts
            WHERE modified_at_ms IS NULL
            "#,
        )
        .map_err(|error| format!("Не удалось подготовить backfill artifact timestamps: {error}"))?;

    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Не удалось прочитать artifacts для backfill: {error}"))?;

    let mut updates = Vec::new();

    for row in rows {
        let (artifact_id, relative_path) =
            row.map_err(|error| format!("Повреждён artifact для backfill: {error}"))?;

        let path = root_dir.join(relative_path);

        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };

        if let Some(modified_at_ms) = file_modified_at_millis(&metadata) {
            updates.push((artifact_id, modified_at_ms));
        }
    }

    drop(statement);

    for (artifact_id, modified_at_ms) in updates {
        connection
            .execute(
                r#"
                UPDATE case_artifacts
                SET modified_at_ms = ?2
                WHERE id = ?1
                "#,
                params![artifact_id, modified_at_ms],
            )
            .map_err(|error| {
                format!(
                    "Не удалось сохранить filesystem timestamp artifact #{artifact_id}: {error}"
                )
            })?;
    }

    Ok(())
}

fn file_modified_at_millis(metadata: &fs::Metadata) -> Option<i64> {
    let duration = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;

    i64::try_from(duration.as_millis()).ok()
}

// Reject symlink components, including ancestor directories and the final file.
// This is a path validation guard, not a race-free openat2/O_NOFOLLOW sandbox.
fn ensure_no_symlink_components(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Symbolic link запрещён в workspace path: {}",
                    ancestor.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Не удалось проверить workspace path: {error}")),
        }
    }
    Ok(())
}

fn ensure_regular_file_without_symlinks(path: &Path) -> Result<(), String> {
    ensure_no_symlink_components(path)?;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("Artifact-файл недоступен: {error}"))?;
    if !metadata.is_file() {
        return Err("Artifact должен быть обычным файлом, не symlink".into());
    }
    Ok(())
}

// A newly created artifact must not survive a failed SQLite registration.
fn register_new_file<T>(
    target: &Path,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    match action() {
        Ok(value) => Ok(value),
        Err(error) => {
            if let Err(cleanup_error) = fs::remove_file(target) {
                return Err(format!(
                    "{error}; не удалось удалить незарегистрированный файл: {cleanup_error}"
                ));
            }
            Err(error)
        }
    }
}

// Write into an exclusively created private file, avoiding an existing symlink.
fn write_new_private_file(path: &Path, content: &[u8]) -> Result<(), String> {
    ensure_no_symlink_components(path)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("Не удалось создать файл {}: {error}", path.display()))?;
    if let Err(error) = file.write_all(content) {
        let _ = fs::remove_file(path);
        return Err(format!(
            "Не удалось записать файл {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    ensure_no_symlink_components(path)?;
    fs::create_dir_all(path).map_err(|error| {
        format!(
            "Не удалось создать приватный каталог {}: {error}",
            path.display(),
        )
    })?;

    ensure_no_symlink_components(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Не удалось выставить 0700 для {}: {error}", path.display(),))
}

fn unique_destination(directory: &Path, file_name: &str) -> PathBuf {
    let first = directory.join(file_name);

    if !first.exists() {
        return first;
    }

    let source = Path::new(file_name);

    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");

    let extension = source.extension().and_then(|value| value.to_str());

    for index in 1..10_000 {
        let candidate_name = match extension {
            Some(extension) => {
                format!("{stem}-{index}.{extension}")
            }

            None => {
                format!("{stem}-{index}")
            }
        };

        let candidate = directory.join(candidate_name);

        if !candidate.exists() {
            return candidate;
        }
    }

    directory.join(format!("{stem}-{}", unix_time_millis().unwrap_or(0),))
}

fn anonymized_name(original: &str, suffix: &str, source_artifact_id: i64) -> String {
    let path = Path::new(original);

    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");

    match path.extension().and_then(|value| value.to_str()) {
        Some(extension) => {
            format!("{stem}.{suffix}.source-{source_artifact_id}.{extension}")
        }

        None => {
            format!("{stem}.{suffix}.source-{source_artifact_id}")
        }
    }
}

fn relative_case_path(
    connection_id: i64,
    case_id: i64,
    subdirectory: &str,
    file_name: &str,
) -> String {
    format!(
        "connections/connection-{connection_id}/cases/case-{case_id}/{subdirectory}/{file_name}"
    )
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);

    if path.is_absolute() {
        return Err("Artifact path должен быть относительным".into());
    }

    for component in path.components() {
        match component {
            Component::Normal(_) => {}

            _ => {
                return Err("Artifact path содержит недопустимые компоненты".into());
            }
        }
    }

    Ok(())
}

fn validate_editable_artifact_kind(value: &str) -> Result<&str, String> {
    match value {
        "sql" | "text" | "source" | "import" | "explain" => Ok(value),
        _ => Err("Некорректный kind для редактируемого case artifact".into()),
    }
}

fn validate_editable_artifact_file_name(value: &str) -> Result<String, String> {
    let value = value.trim();

    if value.is_empty() || value.len() > 200 || value.contains('\0') {
        return Err("Некорректное имя файла".into());
    }

    if value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || sanitize_file_name(value) != value
    {
        return Err(
            "Имя файла может содержать только ASCII-буквы, цифры, '.', '-' и '_' без каталогов"
                .into(),
        );
    }

    Ok(value.to_owned())
}

fn validate_name(value: &str, field: &str) -> Result<String, String> {
    let value = value.trim();

    if value.is_empty() || value.len() > 200 || value.contains('\0') {
        return Err(format!("Некорректное имя {field}"));
    }

    Ok(value.to_owned())
}

fn validate_case_kind(value: &str) -> Result<&str, String> {
    match value {
        "manual" | "incident" => Ok(value),
        _ => Err("Некорректный kind investigation case".into()),
    }
}

fn validate_intervention_kind(value: &str) -> Result<&str, String> {
    match value.trim() {
        "deploy" | "config_change" | "index_create" | "index_drop" | "query_change" | "vacuum"
        | "maintenance" | "other" => Ok(value.trim()),
        _ => Err("Некорректный kind intervention".into()),
    }
}

fn validate_change_impact_periods(
    before_from_ms: i64,
    before_to_ms: i64,
    after_from_ms: i64,
    after_to_ms: i64,
    intervention_at_ms: i64,
) -> Result<(), String> {
    if before_from_ms <= 0
        || before_to_ms <= 0
        || after_from_ms <= 0
        || after_to_ms <= 0
        || before_from_ms > before_to_ms
        || after_from_ms > after_to_ms
    {
        return Err("Некорректные before/after periods для Change Impact".into());
    }
    if before_to_ms > intervention_at_ms || intervention_at_ms > after_from_ms {
        return Err("Intervention должен находиться между before period и after period".into());
    }
    Ok(())
}

fn validate_case_trigger(value: &str) -> Result<String, String> {
    let value = value.trim();

    if value.is_empty() || value.len() > 200 || value.contains('\0') {
        return Err("Некорректный trigger incident case".into());
    }

    Ok(value.to_owned())
}

fn validate_case_description(value: &str) -> Result<String, String> {
    let value = value.trim();

    if value.len() > 8_000 || value.contains('\0') {
        return Err("Некорректное описание case".into());
    }

    Ok(value.to_owned())
}

fn validate_positive_id(id: i64, name: &str) -> Result<(), String> {
    if id <= 0 {
        return Err(format!("Некорректный {name} id"));
    }

    Ok(())
}

fn media_type_for_path(path: &Path) -> Option<String> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("json") => Some("application/json".into()),

        Some("sql") => Some("application/sql".into()),

        Some("md") => Some("text/markdown".into()),

        Some("txt") | Some("log") | Some("csv") | Some("yaml") | Some("yml") => {
            Some("text/plain".into())
        }

        _ => None,
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn unix_time_millis() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Системное время раньше UNIX epoch".to_owned())?;

    i64::try_from(duration.as_millis())
        .map_err(|_| "Системное время не помещается в i64".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_path() {
        assert!(validate_relative_path("../secret",).is_err(),);
    }

    #[test]
    fn anonymized_name_keeps_extension() {
        assert_eq!(
            anonymized_name("plan.json", "anonymized", 42,),
            "plan.anonymized.source-42.json",
        );
    }

    #[test]
    fn records_comparison_members_separately_from_artifact_role() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-comparison-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store
            .create_case(1, "test", "comparison", "", None)
            .unwrap();
        let baseline_path = PathBuf::from(&case.inbox_path).join("baseline.sql");
        let candidate_path = PathBuf::from(&case.inbox_path).join("candidate.sql");

        fs::write(&baseline_path, "SELECT 1").unwrap();
        fs::write(&candidate_path, "SELECT 2").unwrap();

        let artifacts = store.scan_case_inbox(case.id).unwrap();
        let baseline = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "baseline.sql")
            .unwrap();
        let candidate = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "candidate.sql")
            .unwrap();

        store
            .set_artifact_role(baseline.id, Some(ArtifactRole::Query))
            .unwrap();
        store
            .set_artifact_role(candidate.id, Some(ArtifactRole::Query))
            .unwrap();

        let snapshot = serde_json::json!({
            "equivalent": true,
            "baseline_rows": 1,
            "candidate_rows": 1,
        });
        let comparison = store
            .record_comparison_with_result(
                case.id,
                ComparisonKind::QueryResult,
                &[
                    (
                        baseline.id,
                        ComparisonPurpose::Query,
                        ComparisonSide::Baseline,
                    ),
                    (
                        candidate.id,
                        ComparisonPurpose::Query,
                        ComparisonSide::Candidate,
                    ),
                ],
                &snapshot,
            )
            .unwrap();

        assert_eq!(comparison.members.len(), 2);
        assert_eq!(comparison.result, Some(snapshot));
        assert_eq!(
            store.artifact_role(baseline.id).unwrap(),
            Some(ArtifactRole::Query)
        );
        assert_eq!(store.list_comparisons(case.id).unwrap().len(), 1);

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn migrates_unique_legacy_pair_to_comparison() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-legacy-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store.create_case(1, "test", "legacy", "", None).unwrap();
        let baseline_path = PathBuf::from(&case.inbox_path).join("old.sql");
        let candidate_path = PathBuf::from(&case.inbox_path).join("new.sql");

        fs::write(&baseline_path, "SELECT 1").unwrap();
        fs::write(&candidate_path, "SELECT 2").unwrap();

        let artifacts = store.scan_case_inbox(case.id).unwrap();
        let baseline = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "old.sql")
            .unwrap();
        let candidate = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "new.sql")
            .unwrap();
        let now = unix_time_millis().unwrap();

        store
            .connection
            .execute(
                "INSERT INTO case_artifact_roles (artifact_id, role, updated_at_ms) VALUES (?1, 'baseline_query', ?3), (?2, 'candidate_query', ?3)",
                params![baseline.id, candidate.id, now],
            )
            .unwrap();
        store
            .connection
            .execute(
                "DELETE FROM workspace_migrations WHERE name = ?1",
                params![ARTIFACT_COMPARISON_MIGRATION],
            )
            .unwrap();

        let baseline_id = baseline.id;
        let candidate_id = candidate.id;
        let case_id = case.id;

        drop(store);

        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let comparisons = store.list_comparisons(case_id).unwrap();

        assert_eq!(comparisons.len(), 1);
        assert_eq!(comparisons[0].kind, ComparisonKind::LegacyQuery);
        assert_eq!(comparisons[0].members[0].artifact_id, baseline_id);
        assert_eq!(comparisons[0].members[1].artifact_id, candidate_id);
        assert_eq!(
            store.artifact_role(baseline_id).unwrap(),
            Some(ArtifactRole::Query)
        );
        assert_eq!(
            store.artifact_role(candidate_id).unwrap(),
            Some(ArtifactRole::Query)
        );

        drop(store);

        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        assert_eq!(store.list_comparisons(case_id).unwrap().len(), 1);

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn comparison_member_becomes_stale_after_artifact_changes() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-comparison-stale-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store.create_case(1, "test", "stale", "", None).unwrap();
        let baseline_path = PathBuf::from(&case.inbox_path).join("baseline.sql");
        let candidate_path = PathBuf::from(&case.inbox_path).join("candidate.sql");

        fs::write(&baseline_path, "SELECT 1").unwrap();
        fs::write(&candidate_path, "SELECT 2").unwrap();
        let artifacts = store.scan_case_inbox(case.id).unwrap();
        let baseline = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "baseline.sql")
            .unwrap();
        let candidate = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "candidate.sql")
            .unwrap();

        store
            .record_comparison(
                case.id,
                ComparisonKind::QueryResult,
                &[
                    (
                        baseline.id,
                        ComparisonPurpose::Query,
                        ComparisonSide::Baseline,
                    ),
                    (
                        candidate.id,
                        ComparisonPurpose::Query,
                        ComparisonSide::Candidate,
                    ),
                ],
            )
            .unwrap();

        fs::write(&candidate_path, "SELECT 3").unwrap();
        let comparisons = store.list_comparisons(case.id).unwrap();
        let candidate_member = comparisons[0]
            .members
            .iter()
            .find(|member| member.side == ComparisonSide::Candidate)
            .unwrap();

        assert!(candidate_member.stale);

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn artifact_used_by_comparison_requires_explicit_history_deletion() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-comparison-delete-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store.create_case(1, "test", "delete", "", None).unwrap();
        let baseline_path = PathBuf::from(&case.inbox_path).join("baseline.sql");
        let candidate_path = PathBuf::from(&case.inbox_path).join("candidate.sql");

        fs::write(&baseline_path, "SELECT 1").unwrap();
        fs::write(&candidate_path, "SELECT 2").unwrap();
        let artifacts = store.scan_case_inbox(case.id).unwrap();
        let baseline = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "baseline.sql")
            .unwrap();
        let candidate = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "candidate.sql")
            .unwrap();

        store
            .record_comparison(
                case.id,
                ComparisonKind::QueryResult,
                &[
                    (
                        baseline.id,
                        ComparisonPurpose::Query,
                        ComparisonSide::Baseline,
                    ),
                    (
                        candidate.id,
                        ComparisonPurpose::Query,
                        ComparisonSide::Candidate,
                    ),
                ],
            )
            .unwrap();

        assert!(store.delete_artifact(baseline.id, false).is_err());
        store.delete_artifact(baseline.id, true).unwrap();
        assert!(store.list_comparisons(case.id).unwrap().is_empty());

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn upgrades_provisional_comparison_schema_with_sha256_snapshots() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-comparison-upgrade-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store.create_case(1, "test", "upgrade", "", None).unwrap();
        let baseline_path = PathBuf::from(&case.inbox_path).join("baseline.sql");
        let candidate_path = PathBuf::from(&case.inbox_path).join("candidate.sql");

        fs::write(&baseline_path, "SELECT 1").unwrap();
        fs::write(&candidate_path, "SELECT 2").unwrap();
        let artifacts = store.scan_case_inbox(case.id).unwrap();
        let baseline = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "baseline.sql")
            .unwrap();
        let candidate = artifacts
            .iter()
            .find(|artifact| artifact.original_name == "candidate.sql")
            .unwrap();
        let now = unix_time_millis().unwrap();

        store
            .connection
            .execute_batch(
                r#"
                DROP TABLE case_comparison_members;
                DROP TABLE case_comparisons;

                CREATE TABLE case_comparisons (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    case_id INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(case_id)
                        REFERENCES investigation_cases(id)
                        ON DELETE CASCADE
                );

                CREATE TABLE case_comparison_members (
                    comparison_id INTEGER NOT NULL,
                    artifact_id INTEGER NOT NULL,
                    purpose TEXT NOT NULL CHECK(purpose IN ('query', 'plan')),
                    side TEXT NOT NULL CHECK(side IN ('baseline', 'candidate')),
                    PRIMARY KEY(comparison_id, purpose, side),
                    FOREIGN KEY(comparison_id)
                        REFERENCES case_comparisons(id)
                        ON DELETE CASCADE,
                    FOREIGN KEY(artifact_id)
                        REFERENCES case_artifacts(id)
                        ON DELETE CASCADE
                );
                "#,
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO case_comparisons (case_id, kind, created_at_ms) VALUES (?1, 'query_result', ?2)",
                params![case.id, now],
            )
            .unwrap();
        let comparison_id = store.connection.last_insert_rowid();
        store
            .connection
            .execute(
                "INSERT INTO case_comparison_members (comparison_id, artifact_id, purpose, side) VALUES (?1, ?2, 'query', 'baseline'), (?1, ?3, 'query', 'candidate')",
                params![comparison_id, baseline.id, candidate.id],
            )
            .unwrap();

        let case_id = case.id;
        drop(store);

        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let comparisons = store.list_comparisons(case_id).unwrap();

        assert_eq!(comparisons.len(), 1);
        assert_eq!(comparisons[0].members.len(), 2);
        assert!(comparisons[0]
            .members
            .iter()
            .all(|member| { member.artifact_sha256.len() == 64 && !member.stale }));

        let has_sha256 = store
            .connection
            .prepare("PRAGMA table_info(case_comparison_members)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .any(|column| column.unwrap() == "artifact_sha256");
        assert!(has_sha256);

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn incident_case_persists_kind_trigger_and_detected_at() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-incident-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();

        let case = store
            .create_incident_case(1, "test", "lock incident", "captured", "lock_wait", 123_456)
            .unwrap();

        assert_eq!(case.kind, "incident");
        assert_eq!(case.trigger.as_deref(), Some("lock_wait"));
        assert_eq!(case.detected_at_ms, Some(123_456));

        let loaded = store.case_by_id(case.id).unwrap();
        assert_eq!(loaded.kind, "incident");
        assert_eq!(loaded.trigger.as_deref(), Some("lock_wait"));
        assert_eq!(loaded.detected_at_ms, Some(123_456));

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn evidence_artifact_is_saved_as_sensitive_json_source() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-evidence-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store.create_case(1, "test", "evidence", "", None).unwrap();

        let artifact = store
            .write_evidence_artifact(
                case.id,
                "lock-incident.json",
                "{\"schema_version\":1}",
                "lock_incident",
            )
            .unwrap();

        assert_eq!(artifact.kind, "lock_incident");
        assert_eq!(artifact.visibility, "sensitive");
        assert_eq!(artifact.media_type.as_deref(), Some("application/json"));
        assert!(artifact.relative_path.contains("/source/"));

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn creates_and_updates_source_text_artifact() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-editable-artifact-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store.create_case(1, "test", "editor", "", None).unwrap();

        let created = store
            .create_text_artifact(case.id, "query.sql", "SELECT 1;\n", "sql")
            .unwrap();

        assert!(created.relative_path.contains("/source/"));
        assert_eq!(
            store.read_text_artifact(created.id, 1024).unwrap(),
            "SELECT 1;\n"
        );

        let updated = store
            .update_text_artifact(created.id, "SELECT 2;\n")
            .unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(
            store.read_text_artifact(created.id, 1024).unwrap(),
            "SELECT 2;\n"
        );
        assert!(store
            .create_text_artifact(case.id, "../escape.sql", "SELECT 1;", "sql")
            .is_err());

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn result_privacy_policy_is_persisted_per_scope_and_bumps_revision() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-result-privacy-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let workspace = store.ensure_connection_workspace(1, "test").unwrap();
        let scope_id = workspace.default_scope.id;
        let revision = store.scope_revision(scope_id).unwrap();
        let config = ResultPrivacyConfig {
            default: crate::privacy::ResultPrivacyDefault::Preserve,
            keys: None,
            fields: std::collections::BTreeMap::from([(
                "table_name".into(),
                crate::privacy::ResultPrivacyFieldAction::Table,
            )]),
            patterns: Default::default(),
        };

        let updated = store
            .set_scope_result_privacy(scope_id, config.clone())
            .unwrap();
        assert_eq!(updated.result_privacy, config);
        assert_eq!(store.scope_revision(scope_id).unwrap(), revision + 1);

        drop(store);
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        assert_eq!(store.scope_by_id(scope_id).unwrap().result_privacy, config);

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn durable_workspace_changes_survive_restart_and_enforce_case_connection() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-changes-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap()
        ));
        let db = base.join("workspace.sqlite3");
        let root = base.join("files");
        let mut store = WorkspaceStore::open(&db, &root).unwrap();
        let id = store
            .begin_workspace_change(
                1,
                "autovacuum_policy",
                Some("operator"),
                Some(123),
                Some(10),
                "autovacuum_vacuum_scale_factor",
                &serde_json::json!(0.2),
                &serde_json::json!(0.1),
                &serde_json::json!({"host":"127.0.0.1","database":"postgres","database_oid":123}),
            )
            .unwrap();
        assert_eq!(
            store.list_workspace_changes(1, 10).unwrap()[0].status,
            "requested"
        );
        let wrong_case = store.create_case(2, "other", "other", "", None).unwrap();
        assert!(store
            .link_workspace_change_to_case(id, wrong_case.id)
            .is_err());
        store
            .finish_workspace_change(id, "verified", Some(11), "")
            .unwrap();
        assert!(store
            .finish_workspace_change(id, "verified", None, "")
            .is_err());
        let case = store.create_case(1, "test", "incident", "", None).unwrap();
        store.link_workspace_change_to_case(id, case.id).unwrap();
        drop(store);
        let mut store = WorkspaceStore::open(&db, &root).unwrap();
        let changes = store.list_case_workspace_changes(case.id).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].post_snapshot_id, Some(11));
        assert_eq!(changes[0].before, serde_json::json!(0.2));
        assert_eq!(changes[0].target["database_oid"], 123);
        let context = store.case_context(case.id).unwrap();
        assert_eq!(context.applied_changes.len(), 1);
        assert_eq!(context.interventions.len(), 1);
        assert_eq!(
            context.applied_changes[0].intervention_id,
            Some(context.interventions[0].id)
        );
        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn case_intervention_and_change_impact_are_in_timeline() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-change-impact-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let case = store.create_case(1, "test", "impact", "", None).unwrap();
        let intervention = store
            .create_case_intervention(
                case.id,
                200,
                "config_change",
                "Changed work_mem",
                "16 MB -> 64 MB",
            )
            .unwrap();
        let result = serde_json::json!({
            "assessment": {
                "label": "Improvement signals observed",
                "causal_attribution": "not_established"
            }
        });
        let impact = store
            .record_change_impact(case.id, intervention.id, 100, 150, 250, 300, &result)
            .unwrap();

        let context = store.case_context(case.id).unwrap();
        assert_eq!(context.interventions.len(), 1);
        assert_eq!(context.change_impacts.len(), 1);
        assert!(context
            .timeline
            .iter()
            .any(|event| event.id == format!("intervention:{}", intervention.id)));
        assert!(context
            .timeline
            .iter()
            .any(|event| event.id == format!("change-impact:{}", impact.id)));
        assert!(store.delete_case_intervention(intervention.id).is_err());
        store.delete_case_change_impact(impact.id).unwrap();
        store.delete_case_intervention(intervention.id).unwrap();

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn fingerprint_score_caps_weak_relation_matches() {
        let weak = (1..=20)
            .map(|oid| format!("same relation OID: {oid}"))
            .collect::<Vec<_>>();
        let strong = vec!["same root blocker queryid: 123".to_owned()];
        assert_eq!(fingerprint_match_score(&weak), 3);
        assert_eq!(fingerprint_match_score(&strong), 5);
        assert!(fingerprint_match_score(&strong) > fingerprint_match_score(&weak));
    }

    #[test]
    fn similar_incidents_match_deterministic_lock_fingerprint() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-fingerprint-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let database = base.join("workspace.sqlite3");
        let root = base.join("workspaces");
        let mut store = WorkspaceStore::open(&database, &root).unwrap();
        let first = store
            .create_incident_case(1, "test", "first", "", "lock_wait", 1_000)
            .unwrap();
        let second = store
            .create_incident_case(1, "test", "second", "", "lock_wait", 2_000)
            .unwrap();
        let evidence = serde_json::json!({
            "captured_at_ms": 2_000,
            "payload": {
                "lock_snapshot": {
                    "waits": [
                        {"blocked_pid": 10, "root_blocker_pids": [20]}
                    ],
                    "backends": [
                        {
                            "pid": 10,
                            "application_name": "worker",
                            "query_id": "100",
                            "wait_event_type": "Lock",
                            "wait_event": "transactionid"
                        },
                        {
                            "pid": 20,
                            "application_name": "api",
                            "query_id": "4242",
                            "wait_event_type": null,
                            "wait_event": null
                        }
                    ],
                    "locks": [
                        {"relation_oid": 777}
                    ]
                }
            }
        })
        .to_string();
        store
            .write_evidence_artifact(first.id, "first.json", &evidence, "lock_incident")
            .unwrap();
        store
            .write_evidence_artifact(second.id, "second.json", &evidence, "lock_incident")
            .unwrap();

        let context = store.case_context(second.id).unwrap();
        assert_eq!(
            context.fingerprint.dominant_wait_event.as_deref(),
            Some("Lock/transactionid")
        );
        assert_eq!(
            context.fingerprint.root_blocker_queryids,
            vec!["4242".to_owned()]
        );
        assert_eq!(context.fingerprint.relation_oids, vec![777]);
        assert_eq!(context.similar_search_scanned, 1);
        assert_eq!(context.similar_search_total_candidates, 1);
        assert_eq!(context.similar_incidents.len(), 1);
        assert!(context.similar_incidents[0].score > 0);
        assert_eq!(context.similar_incidents[0].case_id, first.id);
        assert!(context.similar_incidents[0]
            .matches
            .iter()
            .any(|value| value.contains("root blocker queryid")));

        drop(store);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn what_changed_evidence_ref_builds_query_navigation() {
        let refs = vec![
            serde_json::Value::String("pgss_statement:10:20:3030:true".into()),
            serde_json::Value::String("pgss_snapshot:77".into()),
        ];
        let navigation = navigation_from_evidence_refs(&refs, 99).unwrap();
        assert_eq!(navigation.kind, "query");
        assert_eq!(navigation.artifact_id, Some(99));
        assert_eq!(navigation.userid, Some(10));
        assert_eq!(navigation.dbid, Some(20));
        assert_eq!(navigation.queryid.as_deref(), Some("3030"));
        assert_eq!(navigation.toplevel, Some(true));
        assert_eq!(navigation.snapshot_id, Some(77));
    }
    #[test]
    fn delete_case_restores_directory_when_sqlite_rejects_delete() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-delete-failure-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let mut store =
            WorkspaceStore::open(&base.join("db.sqlite3"), &base.join("files")).unwrap();
        let case = store.create_case(1, "test", "keep", "", None).unwrap();
        let path = PathBuf::from(&case.source_path).join("proof.txt");
        fs::write(&path, "preserve me").unwrap();
        store
            .connection
            .execute_batch(&format!(
                "CREATE TRIGGER reject_case_delete BEFORE DELETE ON investigation_cases \
             WHEN OLD.id = {} BEGIN SELECT RAISE(ABORT, 'test rejection'); END;",
                case.id,
            ))
            .unwrap();

        assert!(store.delete_case(case.id).is_err());
        assert!(store.case_by_id(case.id).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "preserve me");
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn workspace_rejects_symlinks_in_registered_files_and_inbox() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-symlink-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let mut store =
            WorkspaceStore::open(&base.join("db.sqlite3"), &base.join("files")).unwrap();
        let case = store.create_case(1, "test", "links", "", None).unwrap();
        let outside = base.join("outside.txt");
        fs::write(&outside, "private").unwrap();
        let inbox_link = PathBuf::from(&case.inbox_path).join("linked.txt");
        symlink(&outside, &inbox_link).unwrap();
        let artifacts = store.scan_case_inbox(case.id).unwrap();
        assert!(artifacts
            .iter()
            .all(|item| item.original_name != "linked.txt"));
        assert!(store
            .register_artifact(
                case.id,
                "text",
                "sensitive",
                "linked.txt",
                &relative_case_path(case.connection_id, case.id, "inbox", "linked.txt"),
                Some("text/plain".into()),
            )
            .is_err());
        let created = store
            .create_text_artifact(case.id, "normal.txt", "safe", "text")
            .unwrap();
        let local = store.artifact_path(created.id).unwrap();
        fs::remove_file(&local).unwrap();
        symlink(&outside, &local).unwrap();
        assert!(store.read_text_artifact(created.id, 1024).is_err());
        assert!(store.artifact_sha256(created.id).is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "private");
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn failed_artifact_registration_does_not_leave_a_file() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-register-failure-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let mut store =
            WorkspaceStore::open(&base.join("db.sqlite3"), &base.join("files")).unwrap();
        let case = store.create_case(1, "test", "register", "", None).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_artifact_insert BEFORE INSERT ON case_artifacts \
             BEGIN SELECT RAISE(ABORT, 'test rejection'); END;",
            )
            .unwrap();
        assert!(store
            .create_text_artifact(case.id, "query.sql", "SELECT 1", "sql")
            .is_err());
        assert!(!PathBuf::from(&case.source_path).join("query.sql").exists());
        assert!(store.list_artifacts(case.id).unwrap().is_empty());
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn share_lineage_failure_restores_previous_content_and_metadata() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-share-failure-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let mut store =
            WorkspaceStore::open(&base.join("db.sqlite3"), &base.join("files")).unwrap();
        let case = store.create_case(1, "test", "share", "", None).unwrap();
        let source = store
            .create_text_artifact(case.id, "query.sql", "SELECT private", "sql")
            .unwrap();
        let revision = store.scope_revision(case.scope_id).unwrap();
        let first = store
            .write_share_artifact_with_lineage(
                case.id,
                source.id,
                "SELECT masked",
                "anonymized",
                "anonymized",
                case.scope_id,
                revision,
                "test-engine",
                "sql",
            )
            .unwrap();
        let path = store.artifact_path(first.artifact.id).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_lineage_update BEFORE UPDATE ON artifact_derivations \
             BEGIN SELECT RAISE(ABORT, 'test rejection'); END;",
            )
            .unwrap();
        assert!(store
            .write_share_artifact_with_lineage(
                case.id,
                source.id,
                "SELECT second",
                "anonymized",
                "anonymized",
                case.scope_id,
                revision,
                "test-engine",
                "sql",
            )
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "SELECT masked");
        assert_eq!(
            store
                .artifact_derivation(first.artifact.id)
                .unwrap()
                .unwrap()
                .source_artifact_id,
            source.id
        );
        assert!(store
            .list_artifacts(case.id)
            .unwrap()
            .iter()
            .any(|item| item.id == first.artifact.id));
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn share_rejects_wrong_scope_before_touching_files() {
        let base = std::env::temp_dir().join(format!(
            "pgxray-workspace-share-scope-{}-{}",
            std::process::id(),
            unix_time_millis().unwrap(),
        ));
        let mut store =
            WorkspaceStore::open(&base.join("db.sqlite3"), &base.join("files")).unwrap();
        let first = store.create_case(1, "test", "one", "", None).unwrap();
        let second = store.create_case(2, "test", "two", "", None).unwrap();
        let source = store
            .create_text_artifact(first.id, "query.sql", "SELECT 1", "sql")
            .unwrap();
        assert!(store
            .write_share_artifact_with_lineage(
                first.id,
                source.id,
                "data",
                "anonymized",
                "anonymized",
                second.scope_id,
                1,
                "test-engine",
                "sql",
            )
            .is_err());
        assert!(fs::read_dir(&first.share_path).unwrap().next().is_none());
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }
}
