mod operation_limits;

use operation_limits::{DiagnosticPermit, OperationLimits, TerminalPermit};
use pgxray_core::app_log::OperationLogResult;
use pgxray_core::history::IoWalWorkloadHistory;
use pgxray_core::{
    activity::{self, ActivitySnapshot, BackendSignalAction, BackendSignalResult},
    app_log::{
        ApplicationLog, ApplicationLogEntry, ApplicationLogEvent, ApplicationLogPage,
        ApplicationLogQuery, ApplicationLogSettings, ApplicationLogState, DiagnosticLogDetails,
    },
    auth::{Auth, AuthState, Session, UserSummary},
    autovacuum::{
        self, AutovacuumMaintenanceAdvice, AutovacuumPolicyCandidates, AutovacuumPolicySimulation,
        AutovacuumReference, AutovacuumTableDetail,
    },
    connections::{probe_server_version, ConnectionProject, ConnectionStore, SavedConnection},
    evidence::{EvidenceServerContext, ObservationEnvelope},
    history::{
        self, AutovacuumComparison, AutovacuumRelationHistory, AutovacuumSnapshotSummary,
        AutovacuumWorkloadHistory, ChangeMarker, CollectedAutovacuumSnapshot,
        CollectedIndexHealthSnapshot, CollectedIoWalSnapshot, CollectedPgSettingsSnapshot,
        CollectedPgssSnapshot, CollectedXidSnapshot, DiagnosticStore, HistoryConnectionSummary,
        HistoryMaintenanceResult, HistoryRetentionSettings, IndexHealthComparison,
        IndexHealthSnapshotSummary, IoWalComparison, IoWalSnapshotSummary, PgSettingsChangeEvent,
        PgSettingsComparison, PgSettingsHistoryAnalysis, PgSettingsSnapshotSummary, PgssComparison,
        PgssSnapshotSummary, PgssStatementHistory, PgssWorkloadHistory, StorageHealth,
        StorageMaintenance, StoragePolicy, WhatChangedComparison, WhatChangedPeriod, XidComparison,
        XidHistoryOverviewPoint, XidRelationHistoryPoint, XidSnapshotSummary,
    },
    index_health::{self, IndexHealthReference},
    internal_postgres::{self, InternalPostgresRegistry},
    io_wal::{self, IoWalReference},
    locks::{self, LockSnapshot},
    pgss::{self, PgssExplorerFilter, PgssExplorerPage, PgssQueryDetail, PgssReference},
    plan::{
        self, PlanComparison, PlanInspection, PlanReferenceEntry, QueryPlanComparison,
        QueryResultComparison,
    },
    postgres::ConnectionInput,
    postgres_settings::{self, PgSettingsSnapshot},
    privacy::{
        self, PrivacyMappingDeleteResult, PrivacyMappingEntry, PrivacyMappingEntryKey,
        PrivacyMappingInfo, PrivacyMode, PrivacyOperation, PrivacyTransformResult,
        ResultPrivacyConfig,
    },
    readiness::{self, PostgresReadiness, SUPPORTED_POSTGRES_RANGE},
    sessions::{PsqlSessionEvent, PsqlSessionInfo, PsqlSessionManager, PsqlSessionStart},
    workspace::{
        ArtifactRole, CaseArtifact, CaseChangeImpact, CaseComparison, CaseContext,
        CaseIntervention, ComparisonKind, ComparisonPurpose, ComparisonSide, ConnectionWorkspace,
        InvestigationCase, PrivacyScope, ShareArtifactState, WorkspaceStore,
    },
    xid::{self, XidReference, XidRelationPolicyAdvice},
    SQLITE_QUICK_CHECK_SQL,
};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    future::Future,
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command as StdCommand,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{
    ipc::Channel, utils::config::WebviewUrl, webview::WebviewWindowBuilder, AppHandle, Manager,
    State,
};
use tauri_plugin_dialog::DialogExt;

type SharedApplicationLog = Arc<ApplicationLog>;
type SharedAuth = Arc<Mutex<Auth>>;
type SharedConnections = Arc<Mutex<ConnectionStore>>;
type SharedDiagnostics = Arc<Mutex<DiagnosticStore>>;
type SharedWorkspace = Arc<Mutex<WorkspaceStore>>;
type SharedSessions = Arc<Mutex<PsqlSessionManager>>;
type SharedOperationLimits = Arc<OperationLimits>;
type SharedInternalPostgres = Arc<InternalPostgresRegistry>;
type SharedPathSettings = Arc<Mutex<AppPathSettings>>;
// An application-level lifecycle barrier for multi-step operations spanning
// filesystem, SQLite, and the psql session manager.
type SharedLifecycle = Arc<tokio::sync::RwLock<()>>;

const WORKSPACE_ROOT_SETTING_FILE: &str = "workspace-root";
const EXPORT_DIRECTORY_SETTING_FILE: &str = "export-directory";
const APPLICATION_LOG_DIRECTORY_SETTING_FILE: &str = "application-log-directory";

const APPLICATION_LOG_UI_OPERATIONS: &[&str] = &[
    "register_initial_user",
    "login",
    "logout",
    "create_user",
    "rename_user",
    "change_user_password",
    "delete_user",
    "probe",
    "probe_saved_connection",
    "list_connection_projects",
    "create_connection_project",
    "rename_connection_project",
    "delete_connection_project",
    "assign_connection_project",
    "assign_connection_shortcut",
    "probe_connection_draft",
    "save_connection",
    "update_saved_connection",
    "delete_saved_connection",
    "start_psql_session",
    "close_psql_session",
    "set_export_directory",
    "set_application_log_directory",
    "migrate_workspace_root",
    "runtime_readiness",
    "set_history_retention",
    "delete_connection_history",
    "collect_xid_snapshot",
    "export_xid_csv",
    "xid_relation_policy",
    "apply_xid_policy_candidate",
    "collect_pgss_snapshot",
    "load_pgss_snapshot",
    "compare_pgss_snapshots",
    "pgss_statement_history",
    "pgss_workload_history",
    "pgss_query_detail",
    "collect_io_wal_snapshot",
    "io_wal_workload_history",
    "collect_index_health_snapshot",
    "collect_autovacuum_snapshot",
    "autovacuum_workload_history",
    "collect_pg_settings_snapshot",
    "autovacuum_table_detail",
    "autovacuum_maintenance_advice",
    "autovacuum_policy_candidates",
    "apply_autovacuum_policy_candidate",
    "simulate_autovacuum_policy",
    "create_privacy_scope",
    "update_privacy_scope_result_policy",
    "create_investigation_case",
    "update_investigation_case_description",
    "create_case_intervention",
    "list_workspace_changes",
    "link_workspace_change_to_case",
    "delete_case_intervention",
    "record_case_change_impact",
    "delete_case_change_impact",
    "import_case_file",
    "create_case_file",
    "update_case_file",
    "delete_investigation_case",
    "delete_case_artifact",
    "delete_case_comparison",
    "delete_privacy_scope",
    "reset_privacy_mapping",
    "delete_privacy_mapping_entries",
    "privacy_transform",
    "anonymize_case_artifact",
    "set_case_artifact_role",
    "export_case_artifact",
    "reveal_case_artifact",
    "open_plan_workspace",
    "prepare_plan_share",
    "compare_plan_artifacts",
    "compare_case_query_results",
    "compare_case_query_plans",
    "signal_postgres_backend",
    "save_lock_incident_to_case",
    "create_change_marker",
    "delete_change_marker",
    "save_what_changed_to_case",
];

#[derive(Clone, Serialize)]
struct AppPathSettingsView {
    workspace_root: String,
    export_dir: String,
    application_log_dir: String,
}

#[derive(Serialize)]
struct WorkspaceMigrationResult {
    settings: AppPathSettingsView,
    warning: Option<String>,
}

#[derive(Serialize)]
struct LockIncidentCapture {
    snapshot: LockSnapshot,
    activity_snapshot: Option<ActivitySnapshot>,
    activity_error: Option<String>,
}

#[derive(Serialize)]
struct SavedLockIncident {
    case: InvestigationCase,
    artifact: CaseArtifact,
    snapshot: LockSnapshot,
}

#[derive(Serialize)]
struct SavedWhatChanged {
    case: InvestigationCase,
    artifact: CaseArtifact,
    comparison: WhatChangedComparison,
}

struct AppPathSettings {
    settings_dir: PathBuf,
    workspace_database_path: PathBuf,
    workspace_root: PathBuf,
    export_dir: PathBuf,
    application_log_dir: PathBuf,
}

impl AppPathSettings {
    fn open(
        settings_dir: PathBuf,
        workspace_database_path: PathBuf,
        default_workspace_root: PathBuf,
        default_export_dir: PathBuf,
        default_application_log_dir: PathBuf,
    ) -> Result<Self, String> {
        fs::create_dir_all(&settings_dir)
            .map_err(|error| format!("Не удалось создать каталог path settings: {error}"))?;
        fs::set_permissions(&settings_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Не удалось выставить 0700 для path settings: {error}"))?;

        let workspace_root = load_saved_path(
            &settings_dir.join(WORKSPACE_ROOT_SETTING_FILE),
            &default_workspace_root,
        )?;

        if !workspace_root.is_absolute() {
            return Err(pgxray_core::i18n::text(
                "Сохранённый workspace root должен быть абсолютным путём",
                "Saved workspace root must be an absolute path",
            )
            .into());
        }

        if settings_dir.join(WORKSPACE_ROOT_SETTING_FILE).exists() {
            let metadata = fs::metadata(&workspace_root).map_err(|error| {
                format!(
                    "Сохранённый workspace root {} недоступен: {error}",
                    workspace_root.display(),
                )
            })?;

            if !metadata.is_dir() {
                return Err(format!(
                    "Сохранённый workspace root не является каталогом: {}",
                    workspace_root.display(),
                ));
            }
        }

        let export_dir = load_saved_path(
            &settings_dir.join(EXPORT_DIRECTORY_SETTING_FILE),
            &default_export_dir,
        )?;

        if !export_dir.is_absolute() {
            return Err(pgxray_core::i18n::text(
                "Сохранённый export directory должен быть абсолютным путём",
                "Saved export directory must be an absolute path",
            )
            .into());
        }

        let application_log_dir = load_saved_path(
            &settings_dir.join(APPLICATION_LOG_DIRECTORY_SETTING_FILE),
            &default_application_log_dir,
        )?;

        if !application_log_dir.is_absolute() {
            return Err(pgxray_core::i18n::text(
                "Сохранённый application log directory должен быть абсолютным путём",
                "Saved application log directory must be an absolute path",
            )
            .into());
        }

        Ok(Self {
            settings_dir,
            workspace_database_path,
            workspace_root,
            export_dir,
            application_log_dir,
        })
    }

    fn view(&self) -> AppPathSettingsView {
        AppPathSettingsView {
            workspace_root: self.workspace_root.to_string_lossy().into_owned(),
            export_dir: self.export_dir.to_string_lossy().into_owned(),
            application_log_dir: self.application_log_dir.to_string_lossy().into_owned(),
        }
    }

    fn save_workspace_root(&mut self, path: PathBuf) -> Result<(), String> {
        write_path_setting(&self.settings_dir.join(WORKSPACE_ROOT_SETTING_FILE), &path)?;
        self.workspace_root = path;
        Ok(())
    }

    fn save_export_dir(&mut self, path: PathBuf) -> Result<(), String> {
        write_path_setting(
            &self.settings_dir.join(EXPORT_DIRECTORY_SETTING_FILE),
            &path,
        )?;
        self.export_dir = path;
        Ok(())
    }

    fn save_application_log_dir(&mut self, path: PathBuf) -> Result<(), String> {
        write_path_setting(
            &self
                .settings_dir
                .join(APPLICATION_LOG_DIRECTORY_SETTING_FILE),
            &path,
        )?;
        self.application_log_dir = path;
        Ok(())
    }
}

fn load_saved_path(path: &Path, fallback: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Ok(fallback.to_path_buf());
    }

    let value = fs::read_to_string(path)
        .map_err(|error| format!("Не удалось прочитать {}: {error}", path.display()))?;
    let value = value.trim_end_matches('\n').trim_end_matches('\r');

    if value.is_empty() {
        return Err(format!("Path setting пуст: {}", path.display()));
    }

    Ok(PathBuf::from(value))
}

fn write_path_setting(path: &Path, value: &Path) -> Result<(), String> {
    if !value.is_absolute() {
        return Err(pgxray_core::i18n::text(
            "Путь должен быть абсолютным",
            "Path must be absolute",
        )
        .into());
    }

    let value = value.to_str().ok_or(pgxray_core::i18n::text(
        "Путь должен быть валидным UTF-8 для отображения в UI",
        "Path must be valid UTF-8 to display in the UI",
    ))?;
    let temporary = path.with_extension("tmp");

    fs::write(&temporary, value)
        .map_err(|error| format!("Не удалось сохранить path setting: {error}"))?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Не удалось выставить 0600 для path setting: {error}"))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("Не удалось атомарно заменить path setting: {error}"))?;

    Ok(())
}

fn default_export_directory(app_data_dir: &Path) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .map(|home| home.join("Downloads").join("pgxray"))
        .unwrap_or_else(|| app_data_dir.join("exports"))
}

fn prepare_export_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Export directory должен быть абсолютным путём".into());
    }

    let existed = path.exists();

    fs::create_dir_all(path)
        .map_err(|error| format!("Не удалось создать export directory: {error}"))?;

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Export directory недоступен: {error}"))?;

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Export directory должен быть обычным каталогом, а не symlink".into());
    }

    if !existed {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Не удалось выставить 0700 для export directory: {error}"))?;
    }

    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("Не удалось разрешить export directory: {error}"))?;

    probe_directory_writable(&canonical)?;

    Ok(canonical)
}

fn probe_directory_writable(directory: &Path) -> Result<(), String> {
    for suffix in 0..1000_u32 {
        let probe = directory.join(format!(
            ".pgxray-write-test-{}-{suffix}",
            std::process::id(),
        ));

        match OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(file) => {
                drop(file);
                let _ = fs::set_permissions(&probe, fs::Permissions::from_mode(0o600));
                fs::remove_file(&probe)
                    .map_err(|error| format!("Не удалось удалить write probe: {error}"))?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Каталог {} недоступен для записи: {error}",
                    directory.display(),
                ));
            }
        }
    }

    Err("Не удалось подобрать имя для write probe".into())
}

fn export_artifact_no_clobber(
    directory: &Path,
    source: &Path,
    original_name: &str,
) -> Result<PathBuf, String> {
    let safe_name = Path::new(original_name)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Некорректное имя export artifact")?;
    let name = Path::new(safe_name);
    let stem = name
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    let extension = name.extension().and_then(|value| value.to_str());

    let mut source_file = fs::File::open(source)
        .map_err(|error| format!("Не удалось открыть исходный artifact: {error}"))?;

    for index in 0..=10_000_u32 {
        let candidate_name = if index == 0 {
            safe_name.to_owned()
        } else {
            match extension {
                Some(extension) => format!("{stem}-{index}.{extension}"),
                None => format!("{stem}-{index}"),
            }
        };
        let target = directory.join(candidate_name);

        // create_new is atomic: it rejects both existing files and symlinks,
        // including dangling symlinks. fs::copy would overwrite an existing target.
        let mut target_file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("Не удалось создать export artifact: {error}"));
            }
        };

        let result =
            io::copy(&mut source_file, &mut target_file).and_then(|_| target_file.sync_all());
        drop(target_file);
        if let Err(error) = result {
            let _ = fs::remove_file(&target);
            return Err(format!("Не удалось экспортировать artifact: {error}"));
        }
        return Ok(target);
    }

    Err("Не удалось подобрать свободное имя для export artifact".into())
}

// XID CSV exports are user-initiated, local and confined to the configured
// export directory.  Never accept paths from the WebView.
const MAX_XID_CSV_BYTES: usize = 32 * 1024 * 1024;

fn xid_csv_name_valid(name: &str) -> bool {
    let base = match name.strip_suffix(".csv") {
        Some(base) if name.len() <= 120 => base,
        _ => return false,
    };
    let (ids, expected) = if let Some(ids) = base.strip_prefix("pgxray-xid-relations-") {
        (ids, 1)
    } else if let Some(ids) = base.strip_prefix("pgxray-xid-comparison-") {
        (ids, 2)
    } else {
        return false;
    };
    let parts: Vec<&str> = ids.split('-').collect();
    parts.len() == expected
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn write_xid_csv_no_clobber(
    directory: &Path,
    name: &str,
    content: &str,
) -> Result<PathBuf, String> {
    if !xid_csv_name_valid(name) {
        return Err("Некорректное имя XID CSV: ожидается pgxray-xid-relations-ID.csv или pgxray-xid-comparison-ID-ID.csv".into());
    }
    if content.is_empty() || content.len() > MAX_XID_CSV_BYTES {
        return Err("CSV пуст или превышает допустимый размер 32 МиБ".into());
    }
    let stem = name.strip_suffix(".csv").expect("validated extension");
    for index in 0..=10_000_u32 {
        let candidate = if index == 0 {
            name.to_owned()
        } else {
            format!("{stem}-{index}.csv")
        };
        let target = directory.join(candidate);
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Не удалось создать XID CSV: {error}")),
        };
        let result = file
            .write_all(content.as_bytes())
            .and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&target);
            return Err(format!("Не удалось сохранить XID CSV: {error}"));
        }
        return Ok(target);
    }
    Err("Не удалось подобрать свободное имя файла XID CSV".into())
}

fn workspace_destination(
    path: &Path,
    source_root: &Path,
) -> Result<(PathBuf, Option<u32>), String> {
    if !path.is_absolute() {
        return Err("Workspace root должен быть абсолютным путём".into());
    }

    let parent = path
        .parent()
        .ok_or("Workspace root должен иметь parent directory")?;
    let file_name = path
        .file_name()
        .ok_or("Workspace root не может быть корнем файловой системы")?;

    if !parent.exists() || !parent.is_dir() {
        return Err(format!(
            "Parent directory workspace root должен существовать: {}",
            parent.display(),
        ));
    }

    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("Не удалось разрешить parent workspace root: {error}"))?;
    let destination = canonical_parent.join(file_name);
    let existing_mode = if destination.exists() {
        let metadata = fs::symlink_metadata(&destination)
            .map_err(|error| format!("Workspace destination недоступен: {error}"))?;

        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Workspace destination должен быть обычным каталогом".into());
        }

        if fs::read_dir(&destination)
            .map_err(|error| format!("Не удалось проверить workspace destination: {error}"))?
            .next()
            .is_some()
        {
            return Err("Workspace destination должен быть пустым каталогом".into());
        }

        Some(metadata.permissions().mode())
    } else {
        None
    };

    let source_root = fs::canonicalize(source_root)
        .map_err(|error| format!("Текущий workspace root недоступен: {error}"))?;

    if destination != source_root && destination.starts_with(&source_root) {
        return Err("Новый workspace root не может находиться внутри текущего workspace".into());
    }

    Ok((destination, existing_mode))
}

fn create_workspace_stage(parent: &Path) -> Result<PathBuf, String> {
    for suffix in 0..1000_u32 {
        let stage = parent.join(format!(
            ".pgxray-workspace-migration-{}-{suffix}",
            std::process::id(),
        ));

        match fs::create_dir(&stage) {
            Ok(()) => {
                fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).map_err(
                    |error| format!("Не удалось выставить 0700 для migration stage: {error}"),
                )?;
                return Ok(stage);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("Не удалось создать migration stage: {error}"));
            }
        }
    }

    Err("Не удалось подобрать migration stage directory".into())
}

fn copy_workspace_tree(source: &Path, destination: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|error| {
        format!(
            "Не удалось прочитать workspace {}: {error}",
            source.display()
        )
    })? {
        let entry =
            entry.map_err(|error| format!("Не удалось прочитать workspace entry: {error}"))?;
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
            format!(
                "Не удалось прочитать metadata {}: {error}",
                source_path.display(),
            )
        })?;

        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Migration workspace не следует symlink: {}",
                source_path.display(),
            ));
        }

        if metadata.is_dir() {
            fs::create_dir(&target_path).map_err(|error| {
                format!("Не удалось создать {}: {error}", target_path.display())
            })?;
            fs::set_permissions(&target_path, metadata.permissions()).map_err(|error| {
                format!(
                    "Не удалось скопировать permissions {}: {error}",
                    target_path.display()
                )
            })?;
            copy_workspace_tree(&source_path, &target_path)?;
            continue;
        }

        if metadata.is_file() {
            fs::copy(&source_path, &target_path).map_err(|error| {
                format!(
                    "Не удалось скопировать {} в {}: {error}",
                    source_path.display(),
                    target_path.display(),
                )
            })?;
            fs::set_permissions(&target_path, metadata.permissions()).map_err(|error| {
                format!(
                    "Не удалось скопировать permissions {}: {error}",
                    target_path.display()
                )
            })?;
            continue;
        }

        return Err(format!(
            "Workspace содержит неподдерживаемый filesystem object: {}",
            source_path.display(),
        ));
    }

    Ok(())
}

fn rollback_workspace_destination(destination: &Path, previous_mode: Option<u32>) {
    let _ = fs::remove_dir_all(destination);

    if let Some(previous_mode) = previous_mode {
        let _ = fs::create_dir(destination);
        let _ = fs::set_permissions(destination, fs::Permissions::from_mode(previous_mode));
    }
}

fn sqlite_readiness_result(key: &str, label: &str, result: Result<(), String>) -> ReadinessCheck {
    match result {
        Ok(()) => ReadinessCheck {
            key: key.into(),
            label: label.into(),
            status: "ready".into(),
            method: Some(SQLITE_QUICK_CHECK_SQL),
            detail: pgxray_core::i18n::text(
                "SQLite вернул «ok». Быстрая проверка целостности SQLite; бизнес-правила, внешние файлы и резервные копии не проверялись.",
                "SQLite returned ‘ok’. This is a quick SQLite integrity check; business rules, external files, and backups were not checked.",
            ).into(),
        },
        Err(error) => ReadinessCheck {
            key: key.into(),
            label: label.into(),
            status: "error".into(),
            method: Some(SQLITE_QUICK_CHECK_SQL),
            detail: error,
        },
    }
}

fn readiness_directory(
    key: &str,
    label: &str,
    path: &Path,
    require_private: bool,
) -> ReadinessCheck {
    let result = (|| {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            if pgxray_core::i18n::is_ru() {
                format!("{} недоступен: {error}", path.display())
            } else {
                format!("{} is unavailable: {error}", path.display())
            }
        })?;

        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(if pgxray_core::i18n::is_ru() {
                format!(
                    "{} должен быть обычным каталогом, а не symlink",
                    path.display()
                )
            } else {
                format!(
                    "{} must be a regular directory, not a symlink",
                    path.display()
                )
            });
        }

        if require_private {
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(if pgxray_core::i18n::is_ru() {
                    format!("{} имеет права {:03o}; для приватного pgXray storage ожидается 0700 или строже", path.display(), mode)
                } else {
                    format!("{} has permissions {:03o}; private pgXray storage requires 0700 or stricter", path.display(), mode)
                });
            }
        }

        probe_directory_writable(path)
    })();

    match result {
        Ok(()) => ReadinessCheck {
            key: key.into(),
            label: label.into(),
            status: "ready".into(),
            method: None,
            detail: if pgxray_core::i18n::is_ru() {
                format!("{} доступен для записи.", path.display())
            } else {
                format!("{} is writable.", path.display())
            },
        },
        Err(error) => ReadinessCheck {
            key: key.into(),
            label: label.into(),
            status: "error".into(),
            method: None,
            detail: error,
        },
    }
}

#[cfg(test)]
mod sqlite_readiness_contract_tests {
    use super::*;

    #[test]
    fn sqlite_check_serializes_the_executed_method_and_verified_result() {
        let check = sqlite_readiness_result("auth_db", "Auth SQLite", Ok(()));
        let value = serde_json::to_value(&check).expect("serialize readiness");
        assert_eq!(value["key"], "auth_db");
        assert_eq!(value["status"], "ready");
        assert_eq!(value["method"], SQLITE_QUICK_CHECK_SQL);
        assert!(value["detail"].as_str().unwrap().contains("SQLite"));
        assert!(
            value["detail"].as_str().unwrap().contains("check")
                || value["detail"].as_str().unwrap().contains("проверка")
        );
    }

    #[test]
    fn sqlite_check_preserves_failure_and_reports_its_method() {
        let check = sqlite_readiness_result(
            "workspace_db",
            "Workspace SQLite",
            Err("SQLite I/O error".into()),
        );
        let value = serde_json::to_value(&check).expect("serialize readiness");
        assert_eq!(value["status"], "error");
        assert_eq!(value["method"], SQLITE_QUICK_CHECK_SQL);
        assert_eq!(value["detail"], "SQLite I/O error");
    }

    #[test]
    fn optional_method_is_omitted_for_non_sqlite_checks() {
        let check = ReadinessCheck {
            key: "application_log".into(),
            label: "Application log".into(),
            status: "ready".into(),
            method: None,
            detail: "Журнал доступен".into(),
        };
        let value = serde_json::to_value(&check).expect("serialize readiness");
        assert!(value.get("method").is_none());
        assert_eq!(value["detail"], "Журнал доступен");
    }
}

async fn with_auth<T, F>(state: SharedAuth, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut Auth) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut auth = state.lock().map_err(|_| "Сервис входа недоступен")?;
        operation(&mut auth)
    })
    .await
    .map_err(|_| "Не удалось завершить операцию входа")?
}

async fn with_connections<T, F>(state: SharedConnections, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut ConnectionStore) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut connections = state
            .lock()
            .map_err(|_| "Хранилище подключений недоступно")?;

        operation(&mut connections)
    })
    .await
    .map_err(|_| "Не удалось завершить операцию с подключениями")?
}

async fn with_sessions<T, F>(state: SharedSessions, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut PsqlSessionManager) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut sessions = state
            .lock()
            .map_err(|_| "Менеджер PostgreSQL-сессий недоступен")?;

        operation(&mut sessions)
    })
    .await
    .map_err(|_| "Не удалось завершить операцию с PostgreSQL-сессией")?
}

async fn with_diagnostics<T, F>(state: SharedDiagnostics, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut DiagnosticStore) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut diagnostics = state
            .lock()
            .map_err(|_| "Хранилище истории диагностики недоступно")?;

        operation(&mut diagnostics)
    })
    .await
    .map_err(|_| "Не удалось завершить операцию с историей диагностики")?
}

async fn with_workspace<T, F>(state: SharedWorkspace, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut WorkspaceStore) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut workspace = state.lock().map_err(|_| "Workspace pgXray недоступен")?;

        operation(&mut workspace)
    })
    .await
    .map_err(|_| "Не удалось завершить workspace operation")?
}

// A compensating rollback for the non-transactional mapping.json + SQLite
// revision update. It covers ordinary errors, not a process/power failure
// between the two writes (which needs a persistent journal).
fn restore_mapping_bytes(path: &Path, previous: Option<&[u8]>) -> Result<(), String> {
    match previous {
        Some(bytes) => {
            for index in 0..1000_u32 {
                let temporary =
                    path.with_extension(format!("rollback-{}-{index}", std::process::id(),));
                let mut file = match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(format!("Mapping rollback file: {error}")),
                };
                let write_result = file.write_all(bytes).and_then(|_| file.sync_all());
                drop(file);
                if let Err(error) = write_result {
                    let _ = fs::remove_file(&temporary);
                    return Err(format!("Mapping rollback write: {error}"));
                }
                return fs::rename(&temporary, path)
                    .map_err(|error| format!("Mapping rollback rename: {error}"));
            }
            Err("Не удалось создать временный файл отката mapping".into())
        }
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Mapping rollback remove: {error}")),
        },
    }
}

async fn with_mapping_revision_update<T, F>(
    workspace: SharedWorkspace,
    scope_id: i64,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut WorkspaceStore, &Path) -> Result<(T, bool), String> + Send + 'static,
{
    with_workspace(workspace, move |store| {
        let path = store.scope_mapping_path(scope_id)?;
        let previous = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("Не удалось сохранить mapping для отката: {error}")),
        };

        let operation_result = operation(store, &path);
        let (result, changed) = match operation_result {
            Ok(value) => value,
            Err(error) => {
                return match restore_mapping_bytes(&path, previous.as_deref()) {
                    Ok(()) => Err(error),
                    Err(restore_error) => Err(format!("{error}; {restore_error}")),
                };
            }
        };
        if changed {
            if let Err(error) = store.bump_scope_revision(scope_id) {
                return match restore_mapping_bytes(&path, previous.as_deref()) {
                    Ok(()) => Err(error),
                    Err(restore_error) => Err(format!("{error}; {restore_error}")),
                };
            }
        }
        Ok(result)
    })
    .await
}

async fn with_path_settings<T, F>(state: SharedPathSettings, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut AppPathSettings) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut settings = state
            .lock()
            .map_err(|_| "Path settings pgXray недоступны")?;

        operation(&mut settings)
    })
    .await
    .map_err(|_| "Не удалось завершить path settings operation")?
}

async fn run_privacy<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| "Не удалось завершить privacy operation")?
}

async fn acquire_terminal_permit(
    limits: SharedOperationLimits,
    connection_id: i64,
) -> Result<TerminalPermit, String> {
    tauri::async_runtime::spawn_blocking(move || limits.acquire_terminal(connection_id))
        .await
        .map_err(|_| "Не удалось дождаться свободного PostgreSQL terminal slot")?
}

async fn acquire_diagnostic_permit(
    limits: SharedOperationLimits,
    connection_id: i64,
) -> Result<DiagnosticPermit, String> {
    tauri::async_runtime::spawn_blocking(move || limits.acquire_diagnostic(connection_id))
        .await
        .map_err(|_| "Не удалось дождаться очереди PostgreSQL diagnostics")?
}

#[derive(Serialize)]
struct ReadinessCheck {
    key: String,
    label: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'static str>,
    detail: String,
}

#[derive(Serialize)]
struct RuntimeReadiness {
    app_version: String,
    supported_postgres: String,
    local_checks: Vec<ReadinessCheck>,
    postgres: Option<PostgresReadiness>,
}

#[derive(Serialize)]
struct CasePrivacyResult {
    artifact: CaseArtifact,
    transform: PrivacyTransformResult,
}

#[derive(Serialize)]
struct XidPolicyApplyResult {
    candidate: xid::XidPolicyCandidate,
    verification: xid::XidPolicyApplyVerification,
    history: Option<XidSnapshotSummary>,
    snapshot: Option<xid::XidSnapshot>,
    warning: Option<String>,
}

#[derive(Serialize)]
struct AutovacuumPolicyApplyResult {
    candidate: autovacuum::AutovacuumPolicyCandidate,
    verification: autovacuum::AutovacuumPolicyApplyVerification,
    history: Option<AutovacuumSnapshotSummary>,
    snapshot: Option<autovacuum::AutovacuumSnapshot>,
    warning: Option<String>,
}

#[derive(Serialize)]
struct PlanWorkspaceData {
    case: InvestigationCase,
    artifact: CaseArtifact,
    context: CaseContext,
    local_plan_json: String,
    anonymous_plan_json: String,
    scope_revision: i64,
    inspection: PlanInspection,
    share: Option<ShareArtifactState>,
}

#[derive(Serialize)]
struct PlanShareResult {
    share: ShareArtifactState,
    transform: PrivacyTransformResult,
}

#[derive(Serialize)]
struct PlanArtifactComparisonResult {
    report: PlanComparison,
    comparison: CaseComparison,
}

#[derive(Serialize)]
struct QueryResultComparisonResult {
    report: QueryResultComparison,
    comparison: CaseComparison,
}

#[derive(Serialize)]
struct QueryPlanComparisonResult {
    report: QueryPlanComparison,
    comparison: CaseComparison,
    baseline_artifact: CaseArtifact,
    candidate_artifact: CaseArtifact,
}

// The PTY stream can close before the process exits. A permit is released
// only after try_wait() or the background reaper confirms process termination.
fn forward_psql_events(
    session_id: u64,
    channel: Channel<PsqlSessionEvent>,
    receiver: mpsc::Receiver<PsqlSessionEvent>,
    sessions: SharedSessions,
    alive: Arc<AtomicBool>,
    operation_limits: SharedOperationLimits,
    owns_permit: bool,
) {
    thread::spawn(move || {
        let mut deliver_to_ui = true;
        loop {
            match receiver.recv_timeout(Duration::from_millis(150)) {
                Ok(event) => {
                    if deliver_to_ui && channel.send(event).is_err() {
                        deliver_to_ui = false;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // A replaced UI subscriber must not leave an idle thread.
                    // Only the original process watcher stays to release the permit.
                    if !owns_permit {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
            // A natural exit is detected by polling Child::try_wait() under
            // the manager lock. After close(), the reaper updates alive.
            if let Ok(mut manager) = sessions.try_lock() {
                let _ = manager.process_status(session_id);
            }
            if !alive.load(Ordering::Acquire) {
                if owns_permit {
                    operation_limits.release_terminal(session_id);
                }
                if deliver_to_ui {
                    let _ = channel.send(PsqlSessionEvent::Exit { session_id });
                }
                break;
            }
        }
    });
}

fn validate_application_log_ui_event(
    operation: &str,
    phase: &str,
    request_id: Option<&str>,
    context: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    if !APPLICATION_LOG_UI_OPERATIONS.contains(&operation) {
        return Err("Недопустимая UI operation для application log".into());
    }

    if !["started", "succeeded", "failed"].contains(&phase) {
        return Err("Недопустимая phase для application log".into());
    }

    if let Some(request_id) = request_id {
        if request_id.len() > 80
            || !request_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_:".contains(&byte))
        {
            return Err("Некорректный request_id application log".into());
        }
    }

    if context.is_some_and(|value| value.len() > 4096) {
        return Err("Слишком большой context application log".into());
    }

    if error.is_some_and(|value| value.len() > 8192) {
        return Err("Слишком большой error application log".into());
    }

    Ok(())
}

async fn current_application_log_actor(auth: SharedAuth) -> Option<String> {
    with_auth(auth, |auth| {
        Ok(auth.state()?.session.map(|session| session.username))
    })
    .await
    .ok()
    .flatten()
}

#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn write_application_log_event(
    log: SharedApplicationLog,
    level: String,
    component: String,
    operation: String,
    phase: String,
    actor: Option<String>,
    request_id: Option<String>,
    duration_ms: Option<u64>,
    context: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        log.record(ApplicationLogEvent {
            level: &level,
            component: &component,
            operation: &operation,
            phase: &phase,
            actor: actor.as_deref(),
            request_id: request_id.as_deref(),
            duration_ms,
            context: context.as_deref(),
            error: error.as_deref(),
        });
    })
    .await
    .map_err(|_| "Не удалось завершить запись application log".to_owned())?;

    Ok(())
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn application_log_event(
    auth: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
    operation: String,
    phase: String,
    request_id: Option<String>,
    duration_ms: Option<u64>,
    context: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    validate_application_log_ui_event(
        &operation,
        &phase,
        request_id.as_deref(),
        context.as_deref(),
        error.as_deref(),
    )?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    let level = if phase == "failed" { "error" } else { "info" }.to_owned();

    write_application_log_event(
        log.inner().clone(),
        level,
        // UI-originated success/failure claims are unverified reports,
        // not authoritative confirmations of completed backend operations.
        "ui_reported".to_owned(),
        operation,
        phase,
        actor,
        request_id,
        duration_ms,
        context,
        error,
    )
    .await
}

/// Synchronize the desktop UI locale before collecting localized diagnostics.
#[tauri::command]
fn set_backend_locale(locale: String) -> Result<(), String> {
    pgxray_core::i18n::set_locale(&locale)
}

#[tauri::command]
async fn application_log_state(
    auth: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
) -> Result<ApplicationLogState, String> {
    require_authenticated(auth.inner().clone()).await?;
    let log = log.inner().clone();

    tauri::async_runtime::spawn_blocking(move || log.state())
        .await
        .map_err(|_| "Не удалось прочитать application log state".to_owned())
}

#[tauri::command]
async fn set_application_log_settings(
    auth: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
    max_total_bytes: u64,
    retention_days: u32,
) -> Result<ApplicationLogState, String> {
    require_authenticated(auth.inner().clone()).await?;
    let actor = current_application_log_actor(auth.inner().clone()).await;
    let log = log.inner().clone();
    let log_for_event = Arc::clone(&log);

    let state = tauri::async_runtime::spawn_blocking(move || {
        log.set_settings(ApplicationLogSettings {
            max_total_bytes,
            retention_days,
        })
    })
    .await
    .map_err(|_| "Не удалось сохранить application log settings".to_owned())??;

    let context = format!("max_total_bytes={max_total_bytes} retention_days={retention_days}");
    log_for_event.record(ApplicationLogEvent {
        level: "info",
        component: "app_log",
        operation: "application_log_settings",
        phase: "updated",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: None,
        context: Some(&context),
        error: None,
    });

    Ok(state)
}

#[tauri::command]
async fn list_application_log_entries(
    auth: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
    limit: Option<u32>,
) -> Result<Vec<ApplicationLogEntry>, String> {
    require_authenticated(auth.inner().clone()).await?;
    let log = log.inner().clone();
    let limit = limit.unwrap_or(200).clamp(1, 1000) as usize;

    tauri::async_runtime::spawn_blocking(move || log.recent(limit))
        .await
        .map_err(|_| "Не удалось прочитать application log".to_owned())?
}

/// Filtered keyset pagination over retained JSONL, not a capped in-memory
/// slice of the last 1000 entries. Read-only and authenticated.
#[tauri::command]
async fn query_application_log(
    auth: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
    query: ApplicationLogQuery,
) -> Result<ApplicationLogPage, String> {
    require_authenticated(auth.inner().clone()).await?;
    let log = log.inner().clone();
    tauri::async_runtime::spawn_blocking(move || log.query(&query))
        .await
        .map_err(|_| "Не удалось завершить поиск по application log".to_owned())?
}

#[tauri::command]
async fn clear_application_log(
    auth: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
) -> Result<ApplicationLogState, String> {
    require_authenticated(auth.inner().clone()).await?;
    let actor = current_application_log_actor(auth.inner().clone()).await;
    let log = log.inner().clone();
    let log_for_event = Arc::clone(&log);

    let state = tauri::async_runtime::spawn_blocking(move || log.clear())
        .await
        .map_err(|_| "Не удалось очистить application log".to_owned())??;

    log_for_event.record(ApplicationLogEvent {
        level: "info",
        component: "app_log",
        operation: "application_log",
        phase: "cleared",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: None,
        context: None,
        error: None,
    });

    Ok(state)
}

#[tauri::command]
async fn reveal_application_log_directory(
    auth: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;
    let directory = log.inner().directory().to_path_buf();
    let directory_for_open = directory.clone();

    tauri::async_runtime::spawn_blocking(move || {
        fs::create_dir_all(&directory_for_open)
            .map_err(|error| format!("Не удалось создать каталог application log: {error}"))?;
        fs::set_permissions(&directory_for_open, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Не удалось выставить 0700 для application log: {error}"))?;

        StdCommand::new("/usr/bin/xdg-open")
            .arg(&directory_for_open)
            .spawn()
            .map_err(|error| {
                format!(
                    "Не удалось открыть каталог application log {}: {error}",
                    directory_for_open.display(),
                )
            })?;

        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "Не удалось запустить file manager".to_owned())??;

    Ok(directory.to_string_lossy().into_owned())
}

// External documentation links must be handed to the system browser. The
// embedded WebView does not reliably open target="_blank" windows.
fn validated_external_url(value: &str) -> Result<tauri::Url, String> {
    let parsed = tauri::Url::parse(value).map_err(|_| "Некорректная внешняя ссылка".to_owned())?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Разрешены только внешние HTTP(S)-ссылки без учётных данных".into());
    }
    Ok(parsed)
}

#[tauri::command]
async fn open_external_url(auth: State<'_, SharedAuth>, url: String) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;
    let target = validated_external_url(&url)?.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        // Pass the URL as a single argument: never execute an interpolated shell.
        let mut child = StdCommand::new("/usr/bin/xdg-open")
            .arg(&target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| format!("Не удалось открыть ссылку в браузере: {error}"))?;
        // Avoid leaving a zombie; xdg-open may outlive the IPC request.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "Не удалось запустить системный браузер".to_owned())?
}

async fn require_authenticated(state: SharedAuth) -> Result<(), String> {
    with_auth(state, |auth| auth.require_authenticated()).await
}

#[tauri::command]
async fn auth_state(state: State<'_, SharedAuth>) -> Result<AuthState, String> {
    with_auth(state.inner().clone(), |auth| auth.state()).await
}

#[tauri::command]
async fn register_initial_user(
    state: State<'_, SharedAuth>,
    username: String,
    password: String,
) -> Result<Session, String> {
    with_auth(state.inner().clone(), move |auth| {
        auth.register_initial_user(&username, &password)
    })
    .await
}

#[tauri::command]
async fn login(
    state: State<'_, SharedAuth>,
    log: State<'_, SharedApplicationLog>,
    username: String,
    password: String,
    client_request_id: Option<String>,
) -> Result<Session, String> {
    let started = Instant::now();
    let result = with_auth(state.inner().clone(), move |auth| {
        auth.login(&username, &password)
    })
    .await;
    let authenticated = result.is_ok();
    let actor = result
        .as_ref()
        .ok()
        .map(|session| session.username.as_str());
    let request_id = client_request_id
        .as_deref()
        .filter(|id| valid_client_request_id(id));
    // The authentication result comes from Rust, not a UI success message.
    // Do not log the password, attempted username on failure, or auth error details.
    log.record_outcome(
        ApplicationLogEvent {
            level: if authenticated { "info" } else { "warning" },
            component: "authentication",
            operation: "login",
            phase: if authenticated { "succeeded" } else { "failed" },
            actor,
            request_id,
            duration_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            context: None,
            error: None,
        },
        OperationLogResult::Authentication { authenticated },
    );
    result
}

#[tauri::command]
async fn list_users(state: State<'_, SharedAuth>) -> Result<Vec<UserSummary>, String> {
    with_auth(state.inner().clone(), |auth| auth.list_users()).await
}

#[tauri::command]
async fn create_user(
    state: State<'_, SharedAuth>,
    username: String,
    password: String,
) -> Result<(), String> {
    with_auth(state.inner().clone(), move |auth| {
        auth.create_user(&username, &password)
    })
    .await
}

#[tauri::command]
async fn rename_user(
    state: State<'_, SharedAuth>,
    username: String,
    new_username: String,
) -> Result<(), String> {
    with_auth(state.inner().clone(), move |auth| {
        auth.rename_user(&username, &new_username)
    })
    .await
}

#[tauri::command]
async fn change_user_password(
    state: State<'_, SharedAuth>,
    username: String,
    password: String,
) -> Result<(), String> {
    with_auth(state.inner().clone(), move |auth| {
        auth.change_user_password(&username, &password)
    })
    .await
}

#[tauri::command]
async fn delete_user(state: State<'_, SharedAuth>, username: String) -> Result<(), String> {
    with_auth(state.inner().clone(), move |auth| {
        auth.delete_user(&username)
    })
    .await
}

#[tauri::command]
async fn logout(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    sessions: State<'_, SharedSessions>,
    internal_postgres: State<'_, SharedInternalPostgres>,
) -> Result<(), String> {
    let _lifecycle = lifecycle.inner().clone().write_owned().await;

    with_sessions(sessions.inner().clone(), |sessions| {
        sessions.close_all();
        Ok(())
    })
    .await?;

    // Process reapers release terminal slots after confirmed exit.
    internal_postgres.shutdown_all().await;

    with_auth(auth.inner().clone(), |auth| {
        auth.logout();
        Ok(())
    })
    .await
}

#[tauri::command]
async fn probe(state: State<'_, SharedAuth>, input: ConnectionInput) -> Result<String, String> {
    require_authenticated(state.inner().clone()).await?;
    internal_postgres::probe(input).await
}

#[tauri::command]
async fn list_connections(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
) -> Result<Vec<SavedConnection>, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_connections(connections.inner().clone(), |store| store.list()).await
}

#[tauri::command]
async fn list_connection_projects(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    application_log: State<'_, SharedApplicationLog>,
    client_request_id: Option<String>,
) -> Result<Vec<ConnectionProject>, String> {
    require_authenticated(auth.inner().clone()).await?;
    let started = Instant::now();
    let result = with_connections(connections.inner().clone(), |store| store.list_projects()).await;
    let actor = current_application_log_actor(auth.inner().clone()).await;
    let request_id = client_request_id
        .as_deref()
        .filter(|id| valid_client_request_id(id));
    let succeeded = result.is_ok();
    let count = result.as_ref().ok().map(|projects| projects.len() as u64);
    application_log.record_outcome(
        ApplicationLogEvent {
            level: if succeeded { "info" } else { "error" },
            component: "connections",
            operation: "list_connection_projects",
            phase: if succeeded { "succeeded" } else { "failed" },
            actor: actor.as_deref(),
            request_id,
            duration_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            context: None,
            error: None, // no project names or raw SQLite errors
        },
        OperationLogResult::ConnectionProjects { count, succeeded },
    );
    result
}

#[tauri::command]
async fn create_connection_project(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    name: String,
) -> Result<ConnectionProject, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_connections(connections.inner().clone(), move |store| {
        store.create_project(&name)
    })
    .await
}

#[tauri::command]
async fn rename_connection_project(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    id: i64,
    name: String,
) -> Result<ConnectionProject, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_connections(connections.inner().clone(), move |store| {
        store.rename_project(id, &name)
    })
    .await
}

#[tauri::command]
async fn delete_connection_project(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;
    with_connections(connections.inner().clone(), move |store| {
        store.delete_project(id)
    })
    .await
}

#[tauri::command]
async fn assign_connection_project(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    connection_id: i64,
    project_id: Option<i64>,
) -> Result<SavedConnection, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_connections(connections.inner().clone(), move |store| {
        store.assign_project(connection_id, project_id)
    })
    .await
}

#[tauri::command]
async fn assign_connection_shortcut(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    connection_id: i64,
    slot: Option<u8>,
) -> Result<Vec<SavedConnection>, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_connections(connections.inner().clone(), move |store| {
        store.assign_shortcut(connection_id, slot)
    })
    .await
}

#[tauri::command]
async fn save_connection(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    name: String,
    input: ConnectionInput,
) -> Result<SavedConnection, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_connections(connections.inner().clone(), move |store| {
        store.save(&name, input)
    })
    .await
}

#[tauri::command]
async fn probe_saved_connection(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    operation_limits: State<'_, SharedOperationLimits>,
    id: i64,
) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;

    let _diagnostic = acquire_diagnostic_permit(operation_limits.inner().clone(), id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(id)
    })
    .await?;

    let result = internal_postgres::probe(input).await;
    // A logout while PostgreSQL was responding must not write a new observation.
    require_authenticated(auth.inner().clone()).await?;
    let version = result
        .as_ref()
        .ok()
        .and_then(|reply| probe_server_version(reply));
    let successful = result.is_ok();
    let persisted = with_connections(connections.inner().clone(), move |store| {
        store.record_probe_result(id, successful, version.as_deref())
    })
    .await;
    if let Err(storage_error) = persisted {
        return Err(match result {
            Ok(_) => format!(
                "PostgreSQL ответил, но не удалось сохранить результат проверки: {storage_error}"
            ),
            Err(probe_error) => format!(
                "{probe_error}; также не удалось сохранить результат проверки: {storage_error}"
            ),
        });
    }
    result
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn delete_saved_connection(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    workspace: State<'_, SharedWorkspace>,
    sessions: State<'_, SharedSessions>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    // Profile deletion must not race a reusable diagnostic client that is
    // still executing against the same saved connection.
    let _diagnostic = acquire_diagnostic_permit(operation_limits.inner().clone(), id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let active = with_sessions(sessions.inner().clone(), move |sessions| {
        Ok(sessions.has_connection(id))
    })
    .await?;

    if active {
        return Err("Сначала отключите активную сессию этого подключения".into());
    }

    // Saved profile is removed last. If local cleanup fails, the connection
    // remains visible and the destructive operation can be retried.
    with_workspace(workspace.inner().clone(), move |store| {
        store.delete_connection_data(id)
    })
    .await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.delete_connection_history(id)?;
        Ok(())
    })
    .await?;

    with_connections(connections.inner().clone(), move |store| store.delete(id)).await?;
    internal_postgres.invalidate(id).await;

    Ok(())
}

#[tauri::command]
async fn probe_connection_draft(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    connection_id: Option<i64>,
    use_saved_password: bool,
    mut input: ConnectionInput,
) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;

    if use_saved_password {
        let id = connection_id.ok_or("Не указан профиль для сохранённого пароля")?;

        let saved_input = with_connections(connections.inner().clone(), move |store| {
            store.load_input(id)
        })
        .await?;

        input.password = saved_input.password;
    }

    internal_postgres::probe(input).await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn update_saved_connection(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    sessions: State<'_, SharedSessions>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    id: i64,
    name: String,
    input: ConnectionInput,
    keep_existing_password: bool,
) -> Result<SavedConnection, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection profile mutation serialized with machine-controlled
    // diagnostics so invalidate() cannot race an active reusable client.
    let _diagnostic = acquire_diagnostic_permit(operation_limits.inner().clone(), id).await?;

    require_authenticated(auth.inner().clone()).await?;

    // Changing the connection target or credentials while psql is alive would
    // make the saved profile appear to refer to the old server. A name-only
    // edit is safe; every credential replacement (including a new password)
    // is treated as a target change.
    let current = with_connections(connections.inner().clone(), move |store| {
        store.load_input(id)
    })
    .await?;
    let target_changed = current.host != input.host
        || current.port != input.port
        || current.database != input.database
        || current.username != input.username
        || current.sslmode != input.sslmode
        || !keep_existing_password;
    if target_changed {
        let active = with_sessions(sessions.inner().clone(), move |manager| {
            Ok(manager.has_connection(id))
        })
        .await?;
        if active {
            return Err("Нельзя менять сервер или пароль, пока открыт SQL-терминал. Сначала закройте сессию.".into());
        }
    }

    let connection = with_connections(connections.inner().clone(), move |store| {
        store.update(id, &name, input, keep_existing_password)
    })
    .await?;

    internal_postgres.invalidate(id).await;

    Ok(connection)
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn start_psql_session(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    sessions: State<'_, SharedSessions>,
    operation_limits: State<'_, SharedOperationLimits>,
    connection_id: i64,
    case_id: Option<i64>,
    cols: u16,
    rows: u16,
    force_new: bool,
    on_event: Channel<PsqlSessionEvent>,
) -> Result<PsqlSessionStart, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    if !force_new {
        let existing = with_sessions(sessions.inner().clone(), move |manager| {
            Ok(manager.active_for(connection_id, case_id))
        })
        .await?;
        if let Some(session) = existing {
            let (sender, receiver) = mpsc::sync_channel(64);
            let alive = with_sessions(sessions.inner().clone(), move |manager| {
                manager.subscribe(session.id, connection_id, case_id, sender)?;
                manager
                    .process_token(session.id)
                    .ok_or("PostgreSQL-сессия не найдена".into())
            })
            .await?;
            forward_psql_events(
                session.id,
                on_event,
                receiver,
                sessions.inner().clone(),
                alive,
                operation_limits.inner().clone(),
                false,
            );
            return Ok(PsqlSessionStart {
                session,
                created: false,
            });
        }
    }

    let terminal_permit =
        acquire_terminal_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let (connection, input) = with_connections(connections.inner().clone(), move |store| {
        Ok((store.get(connection_id)?, store.load_input(connection_id)?))
    })
    .await?;

    let connection_name = connection.name.clone();
    let workspace_dir = with_workspace(workspace.inner().clone(), move |store| match case_id {
        Some(case_id) => store.case_inbox_path(case_id, connection_id),
        None => store.connection_inbox_path(connection_id, &connection_name),
    })
    .await?;

    let (sender, receiver) = mpsc::sync_channel(64);
    let limits_for_start = operation_limits.inner().clone();
    // Permit registration remains in the same session-manager critical section.
    let (started, alternate_receiver, alive) =
        with_sessions(sessions.inner().clone(), move |manager| {
            let started = manager.start(
                connection.id,
                &connection.name,
                case_id,
                workspace_dir,
                input,
                cols,
                rows,
                sender,
                force_new,
            )?;
            let alternate_receiver = if started.created {
                if let Err(error) =
                    limits_for_start.attach_terminal(started.session.id, terminal_permit)
                {
                    let _ = manager.close(started.session.id);
                    return Err(error);
                }
                None
            } else {
                // A second simultaneous start may have reused the session under
                // the manager lock, after our initial lookup.
                let (sender, receiver) = mpsc::sync_channel(64);
                manager.subscribe(started.session.id, connection_id, case_id, sender)?;
                Some(receiver)
            };
            let alive = manager
                .process_token(started.session.id)
                .ok_or("PostgreSQL-сессия не найдена")?;
            Ok((started, alternate_receiver, alive))
        })
        .await?;

    forward_psql_events(
        started.session.id,
        on_event,
        alternate_receiver.unwrap_or(receiver),
        sessions.inner().clone(),
        alive,
        operation_limits.inner().clone(),
        started.created,
    );
    Ok(started)
}

#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "Tauri IPC command signature")]
async fn subscribe_psql_session(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    sessions: State<'_, SharedSessions>,
    operation_limits: State<'_, SharedOperationLimits>,
    session_id: u64,
    connection_id: i64,
    case_id: Option<i64>,
    on_event: Channel<PsqlSessionEvent>,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;
    let (sender, receiver) = mpsc::sync_channel(64);
    let alive = with_sessions(sessions.inner().clone(), move |manager| {
        manager.subscribe(session_id, connection_id, case_id, sender)?;
        manager
            .process_token(session_id)
            .ok_or("PostgreSQL-сессия не найдена".into())
    })
    .await?;
    forward_psql_events(
        session_id,
        on_event,
        receiver,
        sessions.inner().clone(),
        alive,
        operation_limits.inner().clone(),
        false,
    );
    Ok(())
}

#[tauri::command]
async fn list_psql_sessions(
    auth: State<'_, SharedAuth>,
    sessions: State<'_, SharedSessions>,
) -> Result<Vec<PsqlSessionInfo>, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_sessions(sessions.inner().clone(), |manager| Ok(manager.list())).await
}

#[tauri::command]
async fn send_psql_session_input(
    auth: State<'_, SharedAuth>,
    sessions: State<'_, SharedSessions>,
    session_id: u64,
    connection_id: i64,
    case_id: Option<i64>,
    data: String,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    with_sessions(sessions.inner().clone(), move |sessions| {
        sessions.write(session_id, connection_id, case_id, &data)
    })
    .await
}

#[tauri::command]
async fn resize_psql_session(
    auth: State<'_, SharedAuth>,
    sessions: State<'_, SharedSessions>,
    session_id: u64,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    with_sessions(sessions.inner().clone(), move |sessions| {
        sessions.resize(session_id, cols, rows)
    })
    .await
}

#[tauri::command]
async fn close_psql_session(
    auth: State<'_, SharedAuth>,
    sessions: State<'_, SharedSessions>,
    session_id: u64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;
    // The PTY manager requests termination; its process watcher releases the
    // quota only after Child::wait() has completed.
    with_sessions(sessions.inner().clone(), move |manager| {
        manager.close(session_id)
    })
    .await
}

#[tauri::command]
async fn path_settings(
    auth: State<'_, SharedAuth>,
    paths: State<'_, SharedPathSettings>,
) -> Result<AppPathSettingsView, String> {
    require_authenticated(auth.inner().clone()).await?;

    let paths = paths.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let paths = paths
            .lock()
            .map_err(|_| "Path settings pgXray недоступны")?;

        Ok(paths.view())
    })
    .await
    .map_err(|_| "Не удалось прочитать path settings")?
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn runtime_readiness(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    workspace: State<'_, SharedWorkspace>,
    paths: State<'_, SharedPathSettings>,
    log: State<'_, SharedApplicationLog>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: Option<i64>,
) -> Result<RuntimeReadiness, String> {
    require_authenticated(auth.inner().clone()).await?;

    let auth_check = with_auth(auth.inner().clone(), |store| {
        Ok(sqlite_readiness_result(
            "auth_db",
            "Auth SQLite",
            store.integrity_check(),
        ))
    })
    .await?;

    let connections_check = with_connections(connections.inner().clone(), |store| {
        Ok(sqlite_readiness_result(
            "connections_db",
            "Connections SQLite",
            store.integrity_check(),
        ))
    })
    .await?;

    let diagnostics_check = with_diagnostics(diagnostics.inner().clone(), |store| {
        Ok(sqlite_readiness_result(
            "diagnostics_db",
            "Diagnostics SQLite",
            store.integrity_check(),
        ))
    })
    .await?;

    let workspace_check = with_workspace(workspace.inner().clone(), |store| {
        Ok(sqlite_readiness_result(
            "workspace_db",
            "Workspace SQLite",
            store.integrity_check(),
        ))
    })
    .await?;

    let (workspace_root, export_dir, application_log_dir) =
        with_path_settings(paths.inner().clone(), |settings| {
            Ok((
                settings.workspace_root.clone(),
                settings.export_dir.clone(),
                settings.application_log_dir.clone(),
            ))
        })
        .await?;

    let log_state = log.inner().state();
    let log_check = if log_state.status.available {
        ReadinessCheck {
            key: "application_log".into(),
            label: "Application log".into(),
            status: "ready".into(),
            method: None,
            detail: if pgxray_core::i18n::is_ru() {
                format!("Logger пишет в {}.", log_state.status.directory)
            } else {
                format!("Logger writes to {}.", log_state.status.directory)
            },
        }
    } else {
        ReadinessCheck {
            key: "application_log".into(),
            label: "Application log".into(),
            status: "warning".into(),
            method: None,
            detail: log_state.status.last_error.unwrap_or_else(|| {
                pgxray_core::i18n::text(
                    "Logger временно недоступен для записи.",
                    "Logger is temporarily unavailable for writing.",
                )
                .into()
            }),
        }
    };

    let local_checks = vec![
        auth_check,
        connections_check,
        diagnostics_check,
        workspace_check,
        readiness_directory("workspace_root", "Workspace root", &workspace_root, true),
        readiness_directory("export_directory", "Export directory", &export_dir, false),
        readiness_directory(
            "application_log_directory",
            "Application log directory",
            &application_log_dir,
            true,
        ),
        log_check,
    ];

    let postgres = if let Some(connection_id) = connection_id {
        let _diagnostic =
            acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

        require_authenticated(auth.inner().clone()).await?;

        let input = with_connections(connections.inner().clone(), move |store| {
            store.load_input(connection_id)
        })
        .await?;

        Some(
            readiness::collect_postgres_readiness(internal_postgres.inner(), connection_id, &input)
                .await?,
        )
    } else {
        None
    };

    Ok(RuntimeReadiness {
        app_version: env!("CARGO_PKG_VERSION").into(),
        supported_postgres: SUPPORTED_POSTGRES_RANGE.into(),
        local_checks,
        postgres,
    })
}

#[tauri::command]
async fn set_export_directory(
    auth: State<'_, SharedAuth>,
    paths: State<'_, SharedPathSettings>,
    directory: String,
) -> Result<AppPathSettingsView, String> {
    require_authenticated(auth.inner().clone()).await?;

    let paths = paths.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let directory = prepare_export_directory(&PathBuf::from(directory))?;
        let mut paths = paths
            .lock()
            .map_err(|_| "Path settings pgXray недоступны")?;

        paths.save_export_dir(directory)?;

        Ok(paths.view())
    })
    .await
    .map_err(|_| "Не удалось сохранить export directory")?
}

#[tauri::command]
async fn set_application_log_directory(
    auth: State<'_, SharedAuth>,
    paths: State<'_, SharedPathSettings>,
    log: State<'_, SharedApplicationLog>,
    directory: String,
) -> Result<AppPathSettingsView, String> {
    require_authenticated(auth.inner().clone()).await?;

    let requested = PathBuf::from(directory);
    if !requested.is_absolute() {
        return Err("Application log directory должен быть абсолютным путём".into());
    }

    let log = log.inner().clone();
    let paths = paths.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut saved_view = None;
        log.migrate_directory_with_commit(&requested, |canonical| {
            let mut paths = paths
                .lock()
                .map_err(|_| "Path settings pgXray недоступны".to_owned())?;
            paths.save_application_log_dir(canonical.to_path_buf())?;
            saved_view = Some(paths.view());
            Ok(())
        })?;
        saved_view.ok_or_else(|| "Не удалось сохранить application log directory".to_owned())
    })
    .await
    .map_err(|_| "Не удалось завершить перенос application log".to_owned())?
}

#[tauri::command]
async fn migrate_workspace_root(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    sessions: State<'_, SharedSessions>,
    paths: State<'_, SharedPathSettings>,
    directory: String,
) -> Result<WorkspaceMigrationResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let workspace = workspace.inner().clone();
    let sessions = sessions.inner().clone();
    let paths = paths.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut sessions = sessions
            .lock()
            .map_err(|_| "Менеджер PostgreSQL-сессий недоступен")?;

        if !sessions.list().is_empty() {
            return Err(
                "Нельзя переносить workspace при открытых psql-сессиях. Сначала закройте их."
                    .into(),
            );
        }

        let mut workspace = workspace
            .lock()
            .map_err(|_| "Workspace pgXray недоступен")?;
        let mut paths = paths
            .lock()
            .map_err(|_| "Path settings pgXray недоступны")?;

        let source_root = fs::canonicalize(&paths.workspace_root)
            .map_err(|error| format!("Текущий workspace root недоступен: {error}"))?;
        let requested = PathBuf::from(directory);
        let (destination, destination_previous_mode) =
            workspace_destination(&requested, &source_root)?;
        let destination_existed = destination_previous_mode.is_some();

        if destination == source_root {
            return Ok(WorkspaceMigrationResult {
                settings: paths.view(),
                warning: None,
            });
        }

        let parent = destination
            .parent()
            .ok_or("Workspace destination не имеет parent directory")?;
        let stage = create_workspace_stage(parent)?;

        if let Err(error) = copy_workspace_tree(&source_root, &stage) {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }

        if destination_existed {
            fs::remove_dir(&destination).map_err(|error| {
                let _ = fs::remove_dir_all(&stage);
                format!("Не удалось подготовить пустой workspace destination: {error}")
            })?;
        }

        if let Err(error) = fs::rename(&stage, &destination) {
            let _ = fs::remove_dir_all(&stage);

            if let Some(previous_mode) = destination_previous_mode {
                let _ = fs::create_dir(&destination);
                let _ =
                    fs::set_permissions(&destination, fs::Permissions::from_mode(previous_mode));
            }

            return Err(format!(
                "Не удалось активировать новый workspace root: {error}"
            ));
        }

        let destination = fs::canonicalize(&destination).map_err(|error| {
            rollback_workspace_destination(&destination, destination_previous_mode);
            format!("Не удалось разрешить новый workspace root: {error}")
        })?;

        let new_workspace = WorkspaceStore::open(&paths.workspace_database_path, &destination)
            .map_err(|error| {
                rollback_workspace_destination(&destination, destination_previous_mode);
                format!("Новый workspace не прошёл проверку: {error}")
            })?;

        if let Err(error) = paths.save_workspace_root(destination.clone()) {
            rollback_workspace_destination(&destination, destination_previous_mode);
            return Err(error);
        }

        *workspace = new_workspace;

        let warning = match fs::remove_dir_all(&source_root) {
            Ok(()) => None,
            Err(error) => Some(format!(
                "Workspace переключён, но старый каталог {} не удалён: {error}",
                source_root.display(),
            )),
        };

        Ok(WorkspaceMigrationResult {
            settings: paths.view(),
            warning,
        })
    })
    .await
    .map_err(|_| "Не удалось завершить migration workspace root")?
}

async fn scheduler_owner(auth: SharedAuth) -> Result<String, String> {
    with_auth(auth, |auth| {
        auth.require_authenticated()?;
        auth.state()?
            .session
            .map(|s| s.username)
            .ok_or("Сначала войдите в приложение".into())
    })
    .await
}

#[tauri::command]
async fn list_snapshot_schedules(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
) -> Result<Vec<serde_json::Value>, String> {
    let owner = scheduler_owner(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_schedule_rules(&owner)
    })
    .await
}
#[tauri::command]
async fn put_snapshot_schedule(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    rule: serde_json::Value,
    expected_revision: Option<i64>,
) -> Result<serde_json::Value, String> {
    let owner = scheduler_owner(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.put_schedule_rule(&owner, rule, expected_revision)
    })
    .await
}
#[tauri::command]
async fn remove_snapshot_schedule(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    key: String,
    expected_revision: i64,
) -> Result<(), String> {
    let owner = scheduler_owner(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.remove_schedule_rule(&owner, &key, expected_revision)
    })
    .await
}
#[tauri::command]
async fn claim_snapshot_schedule(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    rule: serde_json::Value,
    expected_revision: i64,
    slot_at_ms: i64,
    claimed_at_ms: i64,
) -> Result<Option<serde_json::Value>, String> {
    let owner = scheduler_owner(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.claim_schedule_slot(&owner, rule, expected_revision, slot_at_ms, claimed_at_ms)
    })
    .await
}
#[tauri::command]
async fn history_storage_health(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
) -> Result<StorageHealth, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), |store| store.storage_health()).await
}
#[tauri::command]
async fn update_history_storage_policy(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    max_bytes: Option<u64>,
    auto_prune: bool,
) -> Result<StoragePolicy, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.update_storage_policy(max_bytes, auto_prune)
    })
    .await
}
#[tauri::command]
async fn maintain_history_storage(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    action: String,
) -> Result<StorageMaintenance, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.run_storage_maintenance(&action)
    })
    .await
}
#[tauri::command]
async fn protect_history_snapshot(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    section: String,
    snapshot_id: i64,
    protect: bool,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.protect_snapshot(&section, snapshot_id, protect)
    })
    .await
}
#[tauri::command]
async fn history_maintenance_tick(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
) -> Result<StorageMaintenance, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.maintenance_tick()
    })
    .await
}

#[tauri::command]
async fn history_retention_settings(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
) -> Result<HistoryRetentionSettings, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), |store| {
        store.history_retention_settings()
    })
    .await
}

#[tauri::command]
async fn set_history_retention(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    retention_days: Option<u32>,
) -> Result<HistoryMaintenanceResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.set_history_retention(retention_days)
    })
    .await
}

#[tauri::command]
async fn list_history_connections(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
) -> Result<Vec<HistoryConnectionSummary>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), |store| {
        store.list_history_connections()
    })
    .await
}

#[tauri::command]
async fn delete_connection_history(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    operation_limits: State<'_, SharedOperationLimits>,
    connection_id: i64,
) -> Result<HistoryMaintenanceResult, String> {
    require_authenticated(auth.inner().clone()).await?;
    // Deleting a connection's history must not race a collector's final save.
    let _diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.delete_connection_history(connection_id)
    })
    .await
}

#[tauri::command]
async fn postgres_activity_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    connection_id: i64,
) -> Result<ActivitySnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    activity::snapshot(&input).await
}

#[tauri::command]
async fn postgres_lock_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    connection_id: i64,
) -> Result<LockSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    let mut snapshot = locks::snapshot(&input).await?;
    snapshot.source = Some(locks::LockSource::from_input(connection_id, &input));
    Ok(snapshot)
}

#[tauri::command]
async fn postgres_lock_incident_capture(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    connection_id: i64,
) -> Result<LockIncidentCapture, String> {
    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    let mut snapshot = locks::snapshot(&input).await?;
    snapshot.source = Some(locks::LockSource::from_input(connection_id, &input));

    if snapshot.summary.blocked_backends == 0 {
        return Ok(LockIncidentCapture {
            snapshot,
            activity_snapshot: None,
            activity_error: None,
        });
    }

    let (activity_snapshot, activity_error) = match activity::snapshot(&input).await {
        Ok(activity_snapshot) => (Some(activity_snapshot), None),
        Err(error) => (None, Some(error)),
    };

    Ok(LockIncidentCapture {
        snapshot,
        activity_snapshot,
        activity_error,
    })
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn save_lock_incident_to_case(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    log: State<'_, SharedApplicationLog>,
    connection_id: i64,
    case_id: Option<i64>,
    snapshot: LockSnapshot,
    activity_snapshot: Option<serde_json::Value>,
    activity_error: Option<String>,
) -> Result<SavedLockIncident, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection/workspace creation behind the deletion barrier.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    if snapshot.summary.blocked_backends == 0 {
        return Err("В snapshot нет blocked backends; lock incident evidence не сохранён".into());
    }

    let actor = current_application_log_actor(auth.inner().clone()).await;
    let application_log = log.inner().clone();

    let connection = with_connections(connections.inner().clone(), move |store| {
        store.get(connection_id)
    })
    .await?;

    if snapshot.database != connection.database {
        return Err("Lock snapshot принадлежит другой database. Получите новый snapshot.".into());
    }
    let expected_source = locks::LockSource {
        connection_id,
        host: connection.host.clone(),
        port: connection.port,
        username: connection.username.clone(),
        sslmode: connection.sslmode.clone(),
    };
    if snapshot.source.as_ref() != Some(&expected_source) {
        return Err(
            "Профиль подключения изменился после Locks capture. Получите новый snapshot.".into(),
        );
    }

    let mut limitations = snapshot.limitations.clone();
    if let Some(error) = activity_error.as_deref() {
        limitations.push(format!(
            "Companion pg_stat_activity snapshot не сохранился: {error}"
        ));
    }

    let server_context = EvidenceServerContext {
        connection_name: connection.name.clone(),
        host: connection.host.clone(),
        port: connection.port,
        database: snapshot.database.clone(),
        database_oid: Some(snapshot.database_oid),
        server_version: Some(snapshot.server_version.clone()),
    };

    let payload = serde_json::json!({
        "lock_snapshot": snapshot.clone(),
        "activity_snapshot": activity_snapshot,
        "activity_error": activity_error,
    });

    let envelope = ObservationEnvelope::observed(
        connection_id,
        snapshot.captured_at_ms,
        "locks_waits",
        "locks-v1",
        server_context,
        limitations,
        payload,
    );

    let json = serde_json::to_string_pretty(&envelope)
        .map_err(|error| format!("Не удалось сериализовать lock incident evidence: {error}"))?;

    let captured_at_ms = snapshot.captured_at_ms;
    let title = format!(
        "Lock incident · {} · {}",
        connection.database, snapshot.measured_at
    );
    let file_name = format!("lock-incident-{captured_at_ms}.json");

    let (case, artifact) = with_workspace(workspace.inner().clone(), move |store| {
        let case = match case_id {
            Some(case_id) => {
                let case = store.case_by_id(case_id)?;
                if case.connection_id != connection_id {
                    return Err("Case принадлежит другому connection".into());
                }
                case
            }
            None => store.create_incident_case(
                connection.id,
                &connection.name,
                &title,
                "Captured live Locks / Waits evidence.",
                "lock_wait",
                captured_at_ms,
            )?,
        };

        let artifact =
            store.write_evidence_artifact(case.id, &file_name, &json, "lock_incident")?;
        store.set_artifact_role(artifact.id, Some(ArtifactRole::Reference))?;

        Ok((store.case_by_id(case.id)?, artifact))
    })
    .await?;

    let context = format!(
        "connection_id={connection_id} case_id={} artifact_id={} blocked={} root_blockers={}",
        case.id, artifact.id, snapshot.summary.blocked_backends, snapshot.summary.root_blockers,
    );
    application_log.record(ApplicationLogEvent {
        level: "info",
        component: "locks",
        operation: "save_lock_incident_to_case",
        phase: "finished",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: None,
        context: Some(&context),
        error: None,
    });

    Ok(SavedLockIncident {
        case,
        artifact,
        snapshot,
    })
}

#[tauri::command]
async fn signal_postgres_backend(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    log: State<'_, SharedApplicationLog>,
    connection_id: i64,
    pid: i32,
    backend_start_ms: i64,
    action: BackendSignalAction,
) -> Result<BackendSignalResult, String> {
    require_authenticated(auth.inner().clone()).await?;
    let actor = current_application_log_actor(auth.inner().clone()).await;
    let application_log = log.inner().clone();

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    require_authenticated(auth.inner().clone()).await?;

    let context = format!(
        "connection_id={connection_id} pid={pid} backend_start_ms={backend_start_ms} action={}",
        action.as_str(),
    );
    application_log.record(ApplicationLogEvent {
        level: "warning",
        component: "postgres",
        operation: "backend_signal",
        phase: "requested",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: None,
        context: Some(&context),
        error: None,
    });

    let started = std::time::Instant::now();
    let result = activity::signal_backend(&input, pid, backend_start_ms, action).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match &result {
        Ok(result) => {
            let result_context = format!("{context} signaled={}", result.signaled,);
            application_log.record(ApplicationLogEvent {
                level: if result.signaled { "warning" } else { "info" },
                component: "postgres",
                operation: "backend_signal",
                phase: "finished",
                actor: actor.as_deref(),
                request_id: None,
                duration_ms: Some(duration_ms),
                context: Some(&result_context),
                error: None,
            });
        }
        Err(error) => {
            application_log.record(ApplicationLogEvent {
                level: "error",
                component: "postgres",
                operation: "backend_signal",
                phase: "failed",
                actor: actor.as_deref(),
                request_id: None,
                duration_ms: Some(duration_ms),
                context: Some(&context),
                error: Some(error),
            });
        }
    }

    result
}

// A stable, Rust-generated ID for a single collection+durable-save operation.
// Independent of UI-reported request IDs and the IDs of individual SQL calls.
static NEXT_DIAGNOSTIC_OPERATION: AtomicU64 = AtomicU64::new(1);

// Correlation only: a UI-generated ID is never trusted as a Rust operation ID.
fn valid_client_request_id(id: &str) -> bool {
    let mut parts = id.split(':');
    let (Some(prefix), Some(suffix), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    (6..=20).contains(&prefix.len())
        && (1..=8).contains(&suffix.len())
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && suffix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

async fn audit_snapshot<T, F, G>(
    application_log: SharedApplicationLog,
    actor: Option<&str>,
    kind: &'static str,
    connection_id: i64,
    client_request_id: Option<&str>,
    future: F,
    details_for: G,
) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
    G: FnOnce(&T) -> (i64, Option<i64>, Option<i64>),
{
    let id = format!(
        "diag-{}-{}-{}",
        std::process::id(),
        history::unix_time_millis().unwrap_or_default(),
        NEXT_DIAGNOSTIC_OPERATION.fetch_add(1, Ordering::Relaxed),
    );
    let operation = format!("collect_{kind}_snapshot");
    let start_details = DiagnosticLogDetails {
        connection_id,
        kind: kind.to_owned(),
        snapshot_id: None,
        unavailable_sections: None,
        total_sections: None,
        client_request_id: client_request_id
            .filter(|id| valid_client_request_id(id))
            .map(str::to_owned),
    };
    application_log.record_enriched(
        ApplicationLogEvent {
            level: "info",
            component: "diagnostics",
            operation: &operation,
            phase: "started",
            actor,
            request_id: Some(&id),
            duration_ms: None,
            context: None,
            error: None,
        },
        Some(&id),
        Some(start_details.clone()),
    );

    let started = Instant::now();
    let result = internal_postgres::scope_diagnostic_operation(
        id.clone(),
        kind,
        start_details.client_request_id.clone(),
        future,
    )
    .await;
    let terminal_details = match result.as_ref() {
        Ok(value) => {
            let (snapshot_id, unavailable_sections, total_sections) = details_for(value);
            DiagnosticLogDetails {
                snapshot_id: Some(snapshot_id),
                unavailable_sections,
                total_sections,
                ..start_details
            }
        }
        Err(_) => start_details,
    };
    let error = result.as_ref().err().map(String::as_str);
    application_log.record_enriched(
        ApplicationLogEvent {
            level: if error.is_some() { "error" } else { "info" },
            component: "diagnostics",
            operation: &operation,
            phase: if error.is_some() {
                "failed"
            } else {
                "succeeded"
            },
            actor,
            request_id: Some(&id),
            duration_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            context: None,
            error,
        },
        Some(&id),
        Some(terminal_details),
    );
    result
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn collect_xid_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    client_request_id: Option<String>,
    application_log: State<'_, SharedApplicationLog>,
) -> Result<CollectedXidSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    audit_snapshot(
        application_log.inner().clone(),
        actor.as_deref(),
        "xid",
        connection_id,
        client_request_id.as_deref(),
        async {
            let _diagnostic =
                acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

            require_authenticated(auth.inner().clone()).await?;

            let (connection, input) = with_connections(connections.inner().clone(), move |store| {
                Ok((store.get(connection_id)?, store.load_input(connection_id)?))
            })
            .await?;

            let collection_started_at_ms = history::unix_time_millis()?;

            let snapshot =
                xid::collect(internal_postgres.inner().as_ref(), connection_id, &input).await?;

            let collection_completed_at_ms = history::unix_time_millis()?;

            // Keep the permit until the snapshot is durably stored. A connection
            // deletion must not remove history while collection is still committing.
            let (summary, snapshot) = with_diagnostics(diagnostics.inner().clone(), move |store| {
                let summary = store.save_xid_snapshot(
                    &connection,
                    &snapshot,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                )?;

                Ok((summary, snapshot))
            })
            .await?;

            Ok(CollectedXidSnapshot {
                history: summary,
                snapshot,
            })
        },
        |result: &CollectedXidSnapshot| {
            (
                result.history.id,
                Some(result.history.unavailable_sections),
                None,
            )
        },
    )
    .await
}

#[tauri::command]
async fn xid_reference(auth: State<'_, SharedAuth>) -> Result<XidReference, String> {
    require_authenticated(auth.inner().clone()).await?;

    Ok(xid::reference())
}

#[tauri::command]
async fn list_xid_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<XidSnapshotSummary>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_xid_snapshots(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn xid_history_overview(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    limit: Option<u32>,
) -> Result<Vec<XidHistoryOverviewPoint>, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.xid_history_overview(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn load_xid_snapshot(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    snapshot_id: i64,
) -> Result<CollectedXidSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.xid_snapshot(connection_id, snapshot_id)
    })
    .await
}

#[tauri::command]
async fn xid_relation_history(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    snapshot_id: i64,
    relation_oid: u32,
    limit: Option<u32>,
) -> Result<Vec<XidRelationHistoryPoint>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.xid_relation_history(snapshot_id, relation_oid, limit)
    })
    .await
}

#[tauri::command]
async fn xid_relation_policy(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    snapshot_id: i64,
    relation_oid: u32,
    limit: Option<u32>,
) -> Result<XidRelationPolicyAdvice, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.xid_relation_policy(snapshot_id, relation_oid, limit)
    })
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn apply_xid_policy_candidate(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    workspace: State<'_, SharedWorkspace>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    log: State<'_, SharedApplicationLog>,
    connection_id: i64,
    snapshot_id: i64,
    relation_oid: u32,
    candidate_id: String,
) -> Result<XidPolicyApplyResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    if candidate_id.is_empty() || candidate_id.len() > 80 {
        return Err("Некорректный XID policy candidate id".into());
    }

    let diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    let application_log = log.inner().clone();
    let (connection, input) = with_connections(connections.inner().clone(), move |store| {
        Ok((store.get(connection_id)?, store.load_input(connection_id)?))
    })
    .await?;

    let candidate_id_for_lookup = candidate_id.clone();
    let (candidate, anchor_relation, anchor_summary) =
        with_diagnostics(diagnostics.inner().clone(), move |store| {
            store.xid_policy_candidate_context(
                snapshot_id,
                relation_oid,
                &candidate_id_for_lookup,
                Some(100),
            )
        })
        .await?;

    if anchor_summary.connection_id != connection_id
        || anchor_summary.host != connection.host
        || anchor_summary.port != i64::from(connection.port)
        || anchor_summary.database != connection.database
        || anchor_summary.username != connection.username
        || anchor_summary.sslmode != connection.sslmode
    {
        return Err(
            "Сохранённое подключение больше не совпадает с target XID snapshot. Соберите новый snapshot перед apply."
                .into(),
        );
    }
    let database_oid = u32::try_from(anchor_summary.database_oid)
        .map_err(|_| "Некорректный database OID в XID snapshot".to_owned())?;

    let context = format!(
        "connection_id={connection_id} snapshot_id={snapshot_id} relation_oid={relation_oid} candidate_id={} parameter={} current_value={} candidate_value={}",
        candidate.id, candidate.parameter, candidate.current_value, candidate.candidate_value,
    );
    application_log.record(ApplicationLogEvent {
        level: "warning",
        component: "postgres",
        operation: "xid_policy_apply",
        phase: "requested",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: None,
        context: Some(&context),
        error: None,
    });

    // Durable audit is registered before PostgreSQL is modified.
    // Abort Apply if the journal is unavailable: an untracked write is unsafe.
    let audit_id = {
        let actor = actor.clone();
        let parameter = candidate.parameter.clone();
        let before = serde_json::json!(candidate.current_value);
        let after = serde_json::json!(candidate.candidate_value);
        let target = serde_json::json!({
            "host": connection.host.clone(), "port": connection.port,
            "database": connection.database.clone(), "username": connection.username.clone(),
            "sslmode": connection.sslmode.clone(), "database_oid": database_oid,
        });
        with_workspace(workspace.inner().clone(), move |store| {
            store.begin_workspace_change(
                connection_id,
                "xid_policy",
                actor.as_deref(),
                Some(i64::from(relation_oid)),
                Some(snapshot_id),
                &parameter,
                &before,
                &after,
                &target,
            )
        })
        .await?
    };

    let started = std::time::Instant::now();
    let verification = xid::apply_policy_candidate(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        database_oid,
        &anchor_relation,
        &candidate,
    )
    .await;

    let verification = match verification {
        Ok(verification) => verification,
        Err(error) => {
            let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            application_log.record(ApplicationLogEvent {
                level: "error",
                component: "postgres",
                operation: "xid_policy_apply",
                phase: "failed",
                actor: actor.as_deref(),
                request_id: None,
                duration_ms: Some(duration_ms),
                context: Some(&context),
                error: Some(&error),
            });
            let audit_note =
                "Apply завершился ошибкой; подробности см. в Application Log".to_owned();
            let audit_result = with_workspace(workspace.inner().clone(), move |store| {
                store.finish_workspace_change(audit_id, "failed", None, &audit_note)
            })
            .await;
            drop(diagnostic);
            if let Err(audit_error) = audit_result {
                return Err(format!(
                    "Apply завершился ошибкой; audit тоже не обновлён: {audit_error}"
                ));
            }
            return Err(error);
        }
    };

    let (history, snapshot, warning) = match history::unix_time_millis() {
        Ok(collection_started_at_ms) => {
            let post_snapshot = xid::collect(
                internal_postgres.inner().as_ref(),
                connection_id,
                &input,
            )
            .await;

            match post_snapshot {
                Ok(snapshot) => match history::unix_time_millis() {
                    Ok(collection_completed_at_ms) => {
                        let saved = with_diagnostics(
                            diagnostics.inner().clone(),
                            move |store| {
                                store.save_xid_snapshot(
                                    &connection,
                                    &snapshot,
                                    collection_started_at_ms,
                                    collection_completed_at_ms,
                                )
                                .map(|history| (history, snapshot))
                            },
                        )
                        .await;

                        match saved {
                            Ok((history, snapshot)) => (Some(history), Some(snapshot), None),
                            Err(error) => (
                                None,
                                None,
                                Some(format!(
                                    "Freeze policy применена и verified, но post-change XID snapshot не сохранён: {error}. Соберите snapshot вручную."
                                )),
                            ),
                        }
                    }
                    Err(error) => (
                        None,
                        None,
                        Some(format!(
                            "Freeze policy применена и verified, но post-change snapshot не сохранён из-за ошибки времени: {error}. Соберите snapshot вручную."
                        )),
                    ),
                },
                Err(error) => (
                    None,
                    None,
                    Some(format!(
                        "Freeze policy применена и verified, но post-change XID snapshot не собран: {error}. Соберите snapshot вручную."
                    )),
                ),
            }
        }
        Err(error) => (
            None,
            None,
            Some(format!(
                "Freeze policy применена и verified, но post-change snapshot не запущен из-за ошибки времени: {error}. Соберите snapshot вручную."
            )),
        ),
    };

    // Verified means the live Apply verification succeeded. Missing post-snapshot
    // remains an explicit caveat, never silent evidence of improvement.
    let post_snapshot_id = history.as_ref().map(|item| item.id);
    let audit_note = warning.clone().unwrap_or_default();
    let audit_update = with_workspace(workspace.inner().clone(), move |store| {
        store.finish_workspace_change(audit_id, "verified", post_snapshot_id, &audit_note)
    })
    .await;
    // Failure to finalize an audit must be visible; it cannot undo a PostgreSQL write.
    let warning = match audit_update {
        Ok(()) => warning,
        Err(error) => Some(match warning {
            Some(existing) => format!("{existing}; Не удалось завершить audit: {error}"),
            None => format!("Не удалось завершить audit после Apply: {error}"),
        }),
    };

    drop(diagnostic);

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let finished_context = match history.as_ref() {
        Some(history) => format!("{context} post_snapshot_id={}", history.id),
        None => format!("{context} post_snapshot_id=none"),
    };
    application_log.record(ApplicationLogEvent {
        level: "warning",
        component: "postgres",
        operation: "xid_policy_apply",
        phase: "finished",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: Some(duration_ms),
        context: Some(&finished_context),
        error: warning.as_deref(),
    });

    Ok(XidPolicyApplyResult {
        candidate,
        verification,
        history,
        snapshot,
        warning,
    })
}

#[tauri::command]
async fn compare_xid_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    first_id: i64,
    second_id: i64,
) -> Result<XidComparison, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_xid_snapshots(first_id, second_id)
    })
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn collect_pgss_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    client_request_id: Option<String>,
    application_log: State<'_, SharedApplicationLog>,
) -> Result<CollectedPgssSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    audit_snapshot(
        application_log.inner().clone(),
        actor.as_deref(),
        "pgss",
        connection_id,
        client_request_id.as_deref(),
        async {
            let _diagnostic =
                acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

            require_authenticated(auth.inner().clone()).await?;

            let (connection, input) = with_connections(connections.inner().clone(), move |store| {
                Ok((store.get(connection_id)?, store.load_input(connection_id)?))
            })
            .await?;

            let collection_started_at_ms = history::unix_time_millis()?;

            let snapshot =
                pgss::collect(internal_postgres.inner().as_ref(), connection_id, &input).await?;

            let collection_completed_at_ms = history::unix_time_millis()?;

            // Keep the permit until the snapshot is durably stored. A connection
            // deletion must not remove history while collection is still committing.
            let (summary, snapshot) = with_diagnostics(diagnostics.inner().clone(), move |store| {
                let summary = store.save_pgss_snapshot(
                    &connection,
                    &snapshot,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                )?;

                Ok((summary, snapshot))
            })
            .await?;

            Ok(CollectedPgssSnapshot {
                history: summary,
                snapshot,
            })
        },
        |result: &CollectedPgssSnapshot| {
            (
                result.history.id,
                Some(result.history.unavailable_sections),
                Some(6),
            )
        },
    )
    .await
}

#[tauri::command]
async fn pgss_reference(auth: State<'_, SharedAuth>) -> Result<PgssReference, String> {
    require_authenticated(auth.inner().clone()).await?;

    Ok(pgss::reference())
}

#[tauri::command]
async fn list_pgss_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<PgssSnapshotSummary>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_pgss_snapshots(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn load_pgss_snapshot(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    snapshot_id: i64,
) -> Result<CollectedPgssSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.pgss_snapshot(connection_id, snapshot_id)
    })
    .await
}

#[tauri::command]
async fn compare_pgss_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    first_id: i64,
    second_id: i64,
) -> Result<PgssComparison, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_pgss_snapshots(first_id, second_id)
    })
    .await
}

#[tauri::command]
async fn pgss_workload_history(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    limit: Option<u32>,
    anchor_snapshot_id: Option<i64>,
) -> Result<PgssWorkloadHistory, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.pgss_workload_history(connection_id, limit, anchor_snapshot_id)
    })
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn pgss_statement_history(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    snapshot_id: i64,
    userid: u32,
    dbid: u32,
    queryid: String,
    toplevel: bool,
    limit: Option<u32>,
) -> Result<PgssStatementHistory, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.pgss_statement_history(snapshot_id, userid, dbid, queryid, toplevel, limit)
    })
    .await
}

#[tauri::command]
async fn pgss_explore(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    filters: PgssExplorerFilter,
) -> Result<PgssExplorerPage, String> {
    require_authenticated(auth.inner().clone()).await?;
    let _diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;
    require_authenticated(auth.inner().clone()).await?;
    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;
    pgss::explore(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        filters,
    )
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn pgss_query_detail(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    userid: u32,
    dbid: u32,
    queryid: String,
    toplevel: bool,
) -> Result<PgssQueryDetail, String> {
    require_authenticated(auth.inner().clone()).await?;

    let _diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    pgss::query_detail(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        userid,
        dbid,
        queryid,
        toplevel,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn collect_io_wal_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    client_request_id: Option<String>,
    application_log: State<'_, SharedApplicationLog>,
) -> Result<CollectedIoWalSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    audit_snapshot(
        application_log.inner().clone(),
        actor.as_deref(),
        "io_wal",
        connection_id,
        client_request_id.as_deref(),
        async {
            let _diagnostic =
                acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;
            require_authenticated(auth.inner().clone()).await?;
            let (connection, input) = with_connections(connections.inner().clone(), move |store| {
                Ok((store.get(connection_id)?, store.load_input(connection_id)?))
            })
            .await?;
            let collection_started_at_ms = history::unix_time_millis()?;
            let snapshot =
                io_wal::collect(internal_postgres.inner().as_ref(), connection_id, &input).await?;
            let collection_completed_at_ms = history::unix_time_millis()?;
            let (history, snapshot) = with_diagnostics(diagnostics.inner().clone(), move |store| {
                let history = store.save_io_wal_snapshot(
                    &connection,
                    &snapshot,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                )?;
                Ok((history, snapshot))
            })
            .await?;
            Ok(CollectedIoWalSnapshot { history, snapshot })
        },
        |result: &CollectedIoWalSnapshot| {
            (
                result.history.id,
                Some(result.history.unavailable_sections),
                None,
            )
        },
    )
    .await
}

#[tauri::command]
async fn io_wal_reference(auth: State<'_, SharedAuth>) -> Result<IoWalReference, String> {
    require_authenticated(auth.inner().clone()).await?;
    Ok(io_wal::reference())
}

#[tauri::command]
async fn list_io_wal_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<IoWalSnapshotSummary>, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_io_wal_snapshots(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn load_io_wal_snapshot(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    snapshot_id: i64,
) -> Result<io_wal::IoWalSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.io_wal_snapshot(connection_id, snapshot_id)
    })
    .await
}

#[tauri::command]
async fn compare_io_wal_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    first_id: i64,
    second_id: i64,
) -> Result<IoWalComparison, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_io_wal_snapshots(first_id, second_id)
    })
    .await
}

#[tauri::command]
async fn io_wal_workload_history(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    limit: Option<u32>,
    selected_to_snapshot_id: Option<i64>,
) -> Result<IoWalWorkloadHistory, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.io_wal_workload_history(connection_id, limit, selected_to_snapshot_id)
    })
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn collect_index_health_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    client_request_id: Option<String>,
    application_log: State<'_, SharedApplicationLog>,
) -> Result<CollectedIndexHealthSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    audit_snapshot(
        application_log.inner().clone(),
        actor.as_deref(),
        "index_health",
        connection_id,
        client_request_id.as_deref(),
        async {
            let _diagnostic =
                acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;
            require_authenticated(auth.inner().clone()).await?;
            let (connection, input) = with_connections(connections.inner().clone(), move |store| {
                Ok((store.get(connection_id)?, store.load_input(connection_id)?))
            })
            .await?;
            let collection_started_at_ms = history::unix_time_millis()?;
            let snapshot =
                index_health::collect(internal_postgres.inner().as_ref(), connection_id, &input)
                    .await?;
            let collection_completed_at_ms = history::unix_time_millis()?;
            let (history, snapshot) = with_diagnostics(diagnostics.inner().clone(), move |store| {
                let history = store.save_index_health_snapshot(
                    &connection,
                    &snapshot,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                )?;
                Ok((history, snapshot))
            })
            .await?;
            Ok(CollectedIndexHealthSnapshot { history, snapshot })
        },
        |result: &CollectedIndexHealthSnapshot| (result.history.id, None, None),
    )
    .await
}

#[tauri::command]
async fn index_health_reference(
    auth: State<'_, SharedAuth>,
) -> Result<IndexHealthReference, String> {
    require_authenticated(auth.inner().clone()).await?;
    Ok(index_health::reference())
}

#[tauri::command]
async fn list_index_health_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<IndexHealthSnapshotSummary>, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_index_health_snapshots(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn load_index_health_snapshot(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    snapshot_id: i64,
) -> Result<index_health::IndexHealthSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.index_health_snapshot(connection_id, snapshot_id)
    })
    .await
}

#[tauri::command]
async fn compare_index_health_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    first_id: i64,
    second_id: i64,
) -> Result<IndexHealthComparison, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_index_health_snapshots(first_id, second_id)
    })
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn collect_autovacuum_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    client_request_id: Option<String>,
    application_log: State<'_, SharedApplicationLog>,
) -> Result<CollectedAutovacuumSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    audit_snapshot(
        application_log.inner().clone(),
        actor.as_deref(),
        "autovacuum",
        connection_id,
        client_request_id.as_deref(),
        async {
            let _diagnostic =
                acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

            require_authenticated(auth.inner().clone()).await?;

            let (connection, input) = with_connections(connections.inner().clone(), move |store| {
                Ok((store.get(connection_id)?, store.load_input(connection_id)?))
            })
            .await?;

            let collection_started_at_ms = history::unix_time_millis()?;

            let snapshot =
                autovacuum::collect(internal_postgres.inner().as_ref(), connection_id, &input)
                    .await?;

            let collection_completed_at_ms = history::unix_time_millis()?;

            // Keep the permit until the snapshot is durably stored. A connection
            // deletion must not remove history while collection is still committing.
            let (summary, snapshot) = with_diagnostics(diagnostics.inner().clone(), move |store| {
                let summary = store.save_autovacuum_snapshot(
                    &connection,
                    &snapshot,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                )?;

                Ok((summary, snapshot))
            })
            .await?;

            Ok(CollectedAutovacuumSnapshot {
                history: summary,
                snapshot,
            })
        },
        |result: &CollectedAutovacuumSnapshot| {
            (
                result.history.id,
                Some(result.history.unavailable_sections),
                None,
            )
        },
    )
    .await
}

#[tauri::command]
async fn autovacuum_reference(auth: State<'_, SharedAuth>) -> Result<AutovacuumReference, String> {
    require_authenticated(auth.inner().clone()).await?;

    Ok(autovacuum::reference())
}

#[tauri::command]
async fn autovacuum_table_detail(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    relation_oid: u32,
) -> Result<AutovacuumTableDetail, String> {
    require_authenticated(auth.inner().clone()).await?;

    let _diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    autovacuum::table_detail(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        relation_oid,
    )
    .await
}

#[tauri::command]
async fn simulate_autovacuum_policy(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    relation_oid: u32,
    target_dead_tuples: i64,
) -> Result<AutovacuumPolicySimulation, String> {
    require_authenticated(auth.inner().clone()).await?;

    let _diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    autovacuum::simulate_policy(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        relation_oid,
        target_dead_tuples,
    )
    .await
}

#[tauri::command]
async fn list_autovacuum_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<AutovacuumSnapshotSummary>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_autovacuum_snapshots(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn load_autovacuum_snapshot(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    snapshot_id: i64,
) -> Result<autovacuum::AutovacuumSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.autovacuum_snapshot(connection_id, snapshot_id)
    })
    .await
}

#[tauri::command]
async fn autovacuum_workload_history(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    limit: Option<u32>,
    anchor_snapshot_id: Option<i64>,
) -> Result<AutovacuumWorkloadHistory, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.autovacuum_workload_history(connection_id, limit, anchor_snapshot_id)
    })
    .await
}

#[tauri::command]
async fn autovacuum_relation_history(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    snapshot_id: i64,
    relation_oid: u32,
    limit: Option<u32>,
) -> Result<AutovacuumRelationHistory, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.autovacuum_relation_history(snapshot_id, relation_oid, limit)
    })
    .await
}

#[tauri::command]
async fn autovacuum_maintenance_advice(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    snapshot_id: i64,
    relation_oid: u32,
    limit: Option<u32>,
) -> Result<AutovacuumMaintenanceAdvice, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.autovacuum_maintenance_advice(snapshot_id, relation_oid, limit)
    })
    .await
}

#[tauri::command]
async fn autovacuum_policy_candidates(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    snapshot_id: i64,
    relation_oid: u32,
    limit: Option<u32>,
) -> Result<AutovacuumPolicyCandidates, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.autovacuum_policy_candidates(snapshot_id, relation_oid, limit)
    })
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn apply_autovacuum_policy_candidate(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    workspace: State<'_, SharedWorkspace>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    log: State<'_, SharedApplicationLog>,
    connection_id: i64,
    snapshot_id: i64,
    relation_oid: u32,
    candidate_id: String,
) -> Result<AutovacuumPolicyApplyResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    if candidate_id.is_empty() || candidate_id.len() > 80 {
        return Err("Некорректный Autovacuum policy candidate id".into());
    }

    let diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    let application_log = log.inner().clone();
    let (connection, input) = with_connections(connections.inner().clone(), move |store| {
        Ok((store.get(connection_id)?, store.load_input(connection_id)?))
    })
    .await?;

    let candidate_id_for_lookup = candidate_id.clone();
    let (candidate, anchor_table, anchor_summary) =
        with_diagnostics(diagnostics.inner().clone(), move |store| {
            store.autovacuum_policy_candidate_context(
                snapshot_id,
                relation_oid,
                &candidate_id_for_lookup,
                Some(100),
            )
        })
        .await?;

    if anchor_summary.connection_id != connection_id
        || anchor_summary.host != connection.host
        || anchor_summary.port != i64::from(connection.port)
        || anchor_summary.database != connection.database
    {
        return Err(
            "Сохранённое подключение больше не совпадает с target Autovacuum snapshot. Соберите новый snapshot перед apply."
                .into(),
        );
    }
    let database_oid = u32::try_from(anchor_summary.database_oid)
        .map_err(|_| "Некорректный database OID в Autovacuum snapshot".to_owned())?;

    let context = format!(
        "connection_id={connection_id} snapshot_id={snapshot_id} relation_oid={relation_oid} candidate_id={} parameter={} current_scale={} candidate_scale={}",
        candidate.id,
        candidate.parameter,
        candidate.current_scale_factor,
        candidate.candidate_scale_factor,
    );
    application_log.record(ApplicationLogEvent {
        level: "warning",
        component: "postgres",
        operation: "autovacuum_policy_apply",
        phase: "requested",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: None,
        context: Some(&context),
        error: None,
    });

    // Durable audit is registered before PostgreSQL is modified.
    // Abort Apply if the journal is unavailable: an untracked write is unsafe.
    let audit_id = {
        let actor = actor.clone();
        let parameter = candidate.parameter.clone();
        let before = serde_json::json!(candidate.current_scale_factor);
        let after = serde_json::json!(candidate.candidate_scale_factor);
        let target = serde_json::json!({
            "host": connection.host.clone(), "port": connection.port,
            "database": connection.database.clone(), "username": connection.username.clone(),
            "sslmode": connection.sslmode.clone(), "database_oid": database_oid,
        });
        with_workspace(workspace.inner().clone(), move |store| {
            store.begin_workspace_change(
                connection_id,
                "autovacuum_policy",
                actor.as_deref(),
                Some(i64::from(relation_oid)),
                Some(snapshot_id),
                &parameter,
                &before,
                &after,
                &target,
            )
        })
        .await?
    };

    let started = std::time::Instant::now();
    let verification = autovacuum::apply_policy_candidate(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        database_oid,
        &anchor_table,
        &candidate,
    )
    .await;

    let verification = match verification {
        Ok(verification) => verification,
        Err(error) => {
            let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            application_log.record(ApplicationLogEvent {
                level: "error",
                component: "postgres",
                operation: "autovacuum_policy_apply",
                phase: "failed",
                actor: actor.as_deref(),
                request_id: None,
                duration_ms: Some(duration_ms),
                context: Some(&context),
                error: Some(&error),
            });
            let audit_note =
                "Apply завершился ошибкой; подробности см. в Application Log".to_owned();
            let audit_result = with_workspace(workspace.inner().clone(), move |store| {
                store.finish_workspace_change(audit_id, "failed", None, &audit_note)
            })
            .await;
            drop(diagnostic);
            if let Err(audit_error) = audit_result {
                return Err(format!(
                    "Apply завершился ошибкой; audit тоже не обновлён: {audit_error}"
                ));
            }
            return Err(error);
        }
    };

    let (history, snapshot, warning) = match history::unix_time_millis() {
        Ok(collection_started_at_ms) => {
            let post_snapshot = autovacuum::collect(
                internal_postgres.inner().as_ref(),
                connection_id,
                &input,
            )
            .await;

            match post_snapshot {
                Ok(snapshot) => match history::unix_time_millis() {
                    Ok(collection_completed_at_ms) => {
                        let saved = with_diagnostics(
                            diagnostics.inner().clone(),
                            move |store| {
                                store.save_autovacuum_snapshot(
                                    &connection,
                                    &snapshot,
                                    collection_started_at_ms,
                                    collection_completed_at_ms,
                                )
                                .map(|history| (history, snapshot))
                            },
                        )
                        .await;

                        match saved {
                            Ok((history, snapshot)) => (Some(history), Some(snapshot), None),
                            Err(error) => (
                                None,
                                None,
                                Some(format!(
                                    "Autovacuum policy применена и verified, но post-change snapshot не сохранён: {error}. Соберите snapshot вручную."
                                )),
                            ),
                        }
                    }
                    Err(error) => (
                        None,
                        None,
                        Some(format!(
                            "Autovacuum policy применена и verified, но post-change snapshot не сохранён из-за ошибки времени: {error}. Соберите snapshot вручную."
                        )),
                    ),
                },
                Err(error) => (
                    None,
                    None,
                    Some(format!(
                        "Autovacuum policy применена и verified, но post-change snapshot не собран: {error}. Соберите snapshot вручную."
                    )),
                ),
            }
        }
        Err(error) => (
            None,
            None,
            Some(format!(
                "Autovacuum policy применена и verified, но post-change snapshot не запущен из-за ошибки времени: {error}. Соберите snapshot вручную."
            )),
        ),
    };

    // Verified means the live Apply verification succeeded. Missing post-snapshot
    // remains an explicit caveat, never silent evidence of improvement.
    let post_snapshot_id = history.as_ref().map(|item| item.id);
    let audit_note = warning.clone().unwrap_or_default();
    let audit_update = with_workspace(workspace.inner().clone(), move |store| {
        store.finish_workspace_change(audit_id, "verified", post_snapshot_id, &audit_note)
    })
    .await;
    // Failure to finalize an audit must be visible; it cannot undo a PostgreSQL write.
    let warning = match audit_update {
        Ok(()) => warning,
        Err(error) => Some(match warning {
            Some(existing) => format!("{existing}; Не удалось завершить audit: {error}"),
            None => format!("Не удалось завершить audit после Apply: {error}"),
        }),
    };

    drop(diagnostic);

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let finished_context = match history.as_ref() {
        Some(history) => format!("{context} post_snapshot_id={}", history.id),
        None => format!("{context} post_snapshot_id=none"),
    };
    application_log.record(ApplicationLogEvent {
        level: "warning",
        component: "postgres",
        operation: "autovacuum_policy_apply",
        phase: "finished",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: Some(duration_ms),
        context: Some(&finished_context),
        error: warning.as_deref(),
    });

    Ok(AutovacuumPolicyApplyResult {
        candidate,
        verification,
        history,
        snapshot,
        warning,
    })
}

#[tauri::command]
async fn compare_autovacuum_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    first_id: i64,
    second_id: i64,
) -> Result<AutovacuumComparison, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_autovacuum_snapshots(first_id, second_id)
    })
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn collect_pg_settings_snapshot(
    auth: State<'_, SharedAuth>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    connection_id: i64,
    client_request_id: Option<String>,
    application_log: State<'_, SharedApplicationLog>,
) -> Result<CollectedPgSettingsSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    audit_snapshot(
        application_log.inner().clone(),
        actor.as_deref(),
        "pg_settings",
        connection_id,
        client_request_id.as_deref(),
        async {
            let _diagnostic =
                acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

            require_authenticated(auth.inner().clone()).await?;

            let (connection, input) = with_connections(connections.inner().clone(), move |store| {
                Ok((store.get(connection_id)?, store.load_input(connection_id)?))
            })
            .await?;

            let collection_started_at_ms = history::unix_time_millis()?;

            let snapshot = postgres_settings::collect(
                internal_postgres.inner().as_ref(),
                connection_id,
                &input,
            )
            .await?;

            let collection_completed_at_ms = history::unix_time_millis()?;

            with_diagnostics(diagnostics.inner().clone(), move |store| {
                store.save_pg_settings_snapshot(
                    &connection,
                    &snapshot,
                    collection_started_at_ms,
                    collection_completed_at_ms,
                )
            })
            .await
        },
        |result: &CollectedPgSettingsSnapshot| (result.history.id, None, None),
    )
    .await
}

#[tauri::command]
async fn list_pg_settings_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<PgSettingsSnapshotSummary>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_pg_settings_snapshots(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn list_pg_settings_changes(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    limit: Option<u32>,
) -> Result<Vec<PgSettingsChangeEvent>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_pg_settings_changes(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn pg_settings_history_analysis(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    limit: Option<u32>,
) -> Result<PgSettingsHistoryAnalysis, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.pg_settings_history_analysis(connection_id, limit)
    })
    .await
}

#[tauri::command]
async fn load_pg_settings_snapshot(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    snapshot_id: i64,
) -> Result<PgSettingsSnapshot, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.pg_settings_snapshot(snapshot_id)
    })
    .await
}

#[tauri::command]
async fn compare_pg_settings_snapshots(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    first_id: i64,
    second_id: i64,
) -> Result<PgSettingsComparison, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_pg_settings_snapshots(first_id, second_id)
    })
    .await
}

#[tauri::command]
async fn compare_what_changed(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    baseline_from_ms: i64,
    baseline_to_ms: i64,
    incident_from_ms: i64,
    incident_to_ms: i64,
) -> Result<WhatChangedComparison, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_what_changed(
            connection_id,
            WhatChangedPeriod {
                from_ms: baseline_from_ms,
                to_ms: baseline_to_ms,
            },
            WhatChangedPeriod {
                from_ms: incident_from_ms,
                to_ms: incident_to_ms,
            },
        )
    })
    .await
}

#[tauri::command]
async fn list_change_markers(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
) -> Result<Vec<ChangeMarker>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.list_change_markers(connection_id, from_ms, to_ms)
    })
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn create_change_marker(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    occurred_at_ms: i64,
    kind: String,
    title: String,
    note: String,
) -> Result<ChangeMarker, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection/workspace creation behind the deletion barrier.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    with_connections(connections.inner().clone(), move |store| {
        store.get(connection_id).map(|_| ())
    })
    .await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.create_change_marker(connection_id, occurred_at_ms, &kind, &title, &note)
    })
    .await
}

#[tauri::command]
async fn delete_change_marker(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    connection_id: i64,
    marker_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.delete_change_marker(connection_id, marker_id)
    })
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn save_what_changed_to_case(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    diagnostics: State<'_, SharedDiagnostics>,
    workspace: State<'_, SharedWorkspace>,
    log: State<'_, SharedApplicationLog>,
    connection_id: i64,
    case_id: Option<i64>,
    baseline_from_ms: i64,
    baseline_to_ms: i64,
    incident_from_ms: i64,
    incident_to_ms: i64,
) -> Result<SavedWhatChanged, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection/workspace creation behind the deletion barrier.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let actor = current_application_log_actor(auth.inner().clone()).await;
    let application_log = log.inner().clone();
    let connection = with_connections(connections.inner().clone(), move |store| {
        store.get(connection_id)
    })
    .await?;

    let comparison = with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_what_changed(
            connection_id,
            WhatChangedPeriod {
                from_ms: baseline_from_ms,
                to_ms: baseline_to_ms,
            },
            WhatChangedPeriod {
                from_ms: incident_from_ms,
                to_ms: incident_to_ms,
            },
        )
    })
    .await?;

    if connection.host != comparison.target.host
        || connection.port != comparison.target.port
        || connection.database != comparison.target.database
    {
        return Err(
            "Сохранённое connection теперь указывает на другой PostgreSQL target. Создайте новое сравнение для актуального connection."
                .into(),
        );
    }

    let server_context = EvidenceServerContext {
        connection_name: connection.name.clone(),
        host: comparison.target.host.clone(),
        port: comparison.target.port,
        database: comparison.target.database.clone(),
        database_oid: u32::try_from(comparison.target.database_oid).ok(),
        server_version: Some(comparison.target.server_version.clone()),
    };
    let envelope = ObservationEnvelope::observed(
        connection_id,
        comparison.generated_at_ms,
        "what_changed",
        "what-changed-v1",
        server_context,
        comparison.limitations.clone(),
        comparison.clone(),
    );
    let json = serde_json::to_string_pretty(&envelope)
        .map_err(|error| format!("Не удалось сериализовать What Changed evidence: {error}"))?;
    let file_name = format!("what-changed-{incident_to_ms}.json");
    let title = format!("What Changed · {}", comparison.target.database);
    let description = format!(
        "Baseline {}..{}; incident {}..{}.",
        baseline_from_ms, baseline_to_ms, incident_from_ms, incident_to_ms,
    );

    let (case, artifact) = with_workspace(workspace.inner().clone(), move |store| {
        let case = match case_id {
            Some(case_id) => {
                let case = store.case_by_id(case_id)?;
                if case.connection_id != connection_id {
                    return Err("Case принадлежит другому connection".into());
                }
                case
            }
            None => store.create_incident_case(
                connection.id,
                &connection.name,
                &title,
                &description,
                "what_changed",
                incident_from_ms,
            )?,
        };
        let artifact = store.write_generated_artifact(
            case.id,
            &file_name,
            &json,
            "what_changed",
            Some(ArtifactRole::Reference),
        )?;
        Ok((store.case_by_id(case.id)?, artifact))
    })
    .await?;

    let context = format!(
        "connection_id={connection_id} case_id={} artifact_id={} baseline={}..{} incident={}..{}",
        case.id, artifact.id, baseline_from_ms, baseline_to_ms, incident_from_ms, incident_to_ms,
    );
    application_log.record(ApplicationLogEvent {
        level: "info",
        component: "what_changed",
        operation: "save_what_changed_to_case",
        phase: "finished",
        actor: actor.as_deref(),
        request_id: None,
        duration_ms: None,
        context: Some(&context),
        error: None,
    });

    Ok(SavedWhatChanged {
        case,
        artifact,
        comparison,
    })
}

#[tauri::command]
async fn ensure_connection_workspace(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    connection_id: i64,
) -> Result<ConnectionWorkspace, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection/workspace creation behind the deletion barrier.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let connection = with_connections(connections.inner().clone(), move |store| {
        store.get(connection_id)
    })
    .await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.ensure_connection_workspace(connection.id, &connection.name)
    })
    .await
}

#[tauri::command]
async fn list_privacy_scopes(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    connection_id: Option<i64>,
) -> Result<Vec<PrivacyScope>, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection/workspace creation behind the deletion barrier.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    if let Some(connection_id) = connection_id {
        let connection = with_connections(connections.inner().clone(), move |store| {
            store.get(connection_id)
        })
        .await?;

        with_workspace(workspace.inner().clone(), move |store| {
            store.ensure_connection_workspace(connection.id, &connection.name)?;

            store.list_scopes(Some(connection.id))
        })
        .await
    } else {
        with_workspace(workspace.inner().clone(), |store| store.list_scopes(None)).await
    }
}

#[tauri::command]
async fn create_privacy_scope(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    name: String,
    connection_id: Option<i64>,
) -> Result<PrivacyScope, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection/workspace creation behind the deletion barrier.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    if let Some(connection_id) = connection_id {
        with_connections(connections.inner().clone(), move |store| {
            store.get(connection_id)?;

            Ok(())
        })
        .await?;
    }

    with_workspace(workspace.inner().clone(), move |store| {
        store.create_scope(&name, connection_id)
    })
    .await
}

#[tauri::command]
async fn update_privacy_scope_result_policy(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    scope_id: i64,
    result_privacy: ResultPrivacyConfig,
) -> Result<PrivacyScope, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.set_scope_result_privacy(scope_id, result_privacy)
    })
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn create_investigation_case(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    connection_id: i64,
    title: String,
    description: String,
    scope_id: Option<i64>,
) -> Result<InvestigationCase, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep connection/workspace creation behind the deletion barrier.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let connection = with_connections(connections.inner().clone(), move |store| {
        store.get(connection_id)
    })
    .await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.create_case(
            connection.id,
            &connection.name,
            &title,
            &description,
            scope_id,
        )
    })
    .await
}

#[tauri::command]
async fn list_investigation_cases(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    connection_id: i64,
) -> Result<Vec<InvestigationCase>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.list_cases(connection_id)
    })
    .await
}

#[tauri::command]
async fn update_investigation_case_description(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    description: String,
) -> Result<InvestigationCase, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.update_case_description(case_id, &description)
    })
    .await
}

fn build_change_impact_result(
    comparison: &WhatChangedComparison,
    intervention: &CaseIntervention,
) -> serde_json::Value {
    let mut improvement_signals = 0_usize;
    let mut regression_signals = 0_usize;
    let mut workload_shift_observed = false;

    for statement in &comparison.pgss.statement_changes {
        for metric in &statement.metrics {
            if metric.id == "calls_per_hour"
                && metric
                    .relative_change_pct
                    .is_some_and(|value| value.abs() >= 25.0)
            {
                workload_shift_observed = true;
            }

            if metric.evidence_level != "confirmed_change" {
                continue;
            }
            if !matches!(
                metric.id.as_str(),
                "exec_ms_per_call"
                    | "shared_reads_per_call"
                    | "temp_writes_per_call"
                    | "temp_reads_per_call"
                    | "wal_bytes_per_call"
            ) {
                continue;
            }
            match metric.direction.as_str() {
                "down" => improvement_signals += 1,
                "up" => regression_signals += 1,
                _ => {}
            }
        }
    }

    let (state, label) = match improvement_signals.cmp(&regression_signals) {
        std::cmp::Ordering::Greater if improvement_signals > 0 => {
            ("improvement_observed", "Improvement signals observed")
        }
        std::cmp::Ordering::Less if regression_signals > 0 => {
            ("regression_observed", "Regression signals observed")
        }
        _ if improvement_signals > 0 || regression_signals > 0 => {
            ("mixed", "Mixed before/after signals")
        }
        _ => ("no_clear_signal", "No clear before/after signal"),
    };

    serde_json::json!({
        "schema_version": 1,
        "assessment": {
            "state": state,
            "label": label,
            "improvement_signals": improvement_signals,
            "regression_signals": regression_signals,
            "workload_shift_observed": workload_shift_observed,
            "causal_attribution": "not_established",
            "caveat": "Observed before/after changes are associated evidence. Causal attribution to the intervention is not guaranteed."
        },
        "intervention": {
            "id": intervention.id,
            "occurred_at_ms": intervention.occurred_at_ms,
            "kind": intervention.kind.clone(),
            "title": intervention.title.clone(),
        },
        "other_simultaneous_changes": {
            "settings": comparison.settings.changes.len(),
            "autovacuum_relations": comparison.autovacuum.relation_changes.len(),
            "storage_relations": comparison.storage.relation_size_changes.len(),
            "index_changes": comparison.index_health.changes.len(),
        },
        "comparison": comparison,
    })
}

#[tauri::command]
async fn list_workspace_changes(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    connection_id: i64,
    limit: Option<u32>,
) -> Result<Vec<pgxray_core::workspace::WorkspaceChange>, String> {
    require_authenticated(auth.inner().clone()).await?;
    with_workspace(workspace.inner().clone(), move |store| {
        store.list_workspace_changes(connection_id, limit.unwrap_or(100))
    })
    .await
}

#[tauri::command]
async fn link_workspace_change_to_case(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    change_id: i64,
    case_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;
    with_workspace(workspace.inner().clone(), move |store| {
        store.link_workspace_change_to_case(change_id, case_id)
    })
    .await
}

#[tauri::command]
async fn create_case_intervention(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    occurred_at_ms: i64,
    kind: String,
    title: String,
    note: String,
) -> Result<CaseIntervention, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.create_case_intervention(case_id, occurred_at_ms, &kind, &title, &note)
    })
    .await
}

#[tauri::command]
async fn delete_case_intervention(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    intervention_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.delete_case_intervention(intervention_id)
    })
    .await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn record_case_change_impact(
    auth: State<'_, SharedAuth>,
    diagnostics: State<'_, SharedDiagnostics>,
    workspace: State<'_, SharedWorkspace>,
    connection_id: i64,
    case_id: i64,
    intervention_id: i64,
    before_from_ms: i64,
    before_to_ms: i64,
    after_from_ms: i64,
    after_to_ms: i64,
) -> Result<CaseChangeImpact, String> {
    require_authenticated(auth.inner().clone()).await?;

    let (case, intervention) = with_workspace(workspace.inner().clone(), move |store| {
        Ok((
            store.case_by_id(case_id)?,
            store.case_intervention_by_id(intervention_id)?,
        ))
    })
    .await?;

    if case.connection_id != connection_id {
        return Err("Case принадлежит другому connection".into());
    }
    if intervention.case_id != case_id {
        return Err("Intervention принадлежит другому case".into());
    }
    if before_from_ms <= 0
        || before_to_ms <= 0
        || after_from_ms <= 0
        || after_to_ms <= 0
        || before_from_ms > before_to_ms
        || after_from_ms > after_to_ms
    {
        return Err("Некорректные before/after periods для Change Impact".into());
    }
    if before_to_ms > intervention.occurred_at_ms || intervention.occurred_at_ms > after_from_ms {
        return Err("Intervention должен находиться между before period и after period".into());
    }

    let comparison = with_diagnostics(diagnostics.inner().clone(), move |store| {
        store.compare_what_changed(
            connection_id,
            WhatChangedPeriod {
                from_ms: before_from_ms,
                to_ms: before_to_ms,
            },
            WhatChangedPeriod {
                from_ms: after_from_ms,
                to_ms: after_to_ms,
            },
        )
    })
    .await?;

    let result = build_change_impact_result(&comparison, &intervention);
    with_workspace(workspace.inner().clone(), move |store| {
        store.record_change_impact(
            case_id,
            intervention_id,
            before_from_ms,
            before_to_ms,
            after_from_ms,
            after_to_ms,
            &result,
        )
    })
    .await
}

#[tauri::command]
async fn delete_case_change_impact(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    impact_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.delete_case_change_impact(impact_id)
    })
    .await
}

#[tauri::command]
async fn list_case_artifacts(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    scan_inbox: bool,
) -> Result<Vec<CaseArtifact>, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        if scan_inbox {
            store.scan_case_inbox(case_id)
        } else {
            store.list_artifacts(case_id)
        }
    })
    .await
}

// The path is selected by the native dialog, not provided by browser File APIs.
// Authentication is checked before opening a user-visible file picker.
#[tauri::command]
async fn pick_case_import_file(
    app: AppHandle,
    auth: State<'_, SharedAuth>,
) -> Result<Option<String>, String> {
    require_authenticated(auth.inner().clone()).await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_file(move |selected| {
        let result = selected
            .map(|file_path| file_path.into_path().map_err(|error| error.to_string()))
            .transpose();
        let _ = sender.send(result);
    });
    let path = receiver
        .await
        .map_err(|_| "Диалог выбора файла был закрыт без ответа".to_owned())??;
    require_authenticated(auth.inner().clone()).await?;
    Ok(path.map(|value| value.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn import_case_file(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    source_path: String,
    kind: String,
) -> Result<CaseArtifact, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.import_file(case_id, &PathBuf::from(source_path), &kind)
    })
    .await
}

#[tauri::command]
async fn create_case_file(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    file_name: String,
    content: String,
    kind: String,
) -> Result<CaseArtifact, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.create_text_artifact(case_id, &file_name, &content, &kind)
    })
    .await
}

#[tauri::command]
async fn update_case_file(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    artifact_id: i64,
    content: String,
) -> Result<CaseArtifact, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.update_text_artifact(artifact_id, &content)
    })
    .await
}

#[tauri::command]
async fn read_case_artifact(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    artifact_id: i64,
) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.read_text_artifact(artifact_id, 32 * 1024 * 1024)
    })
    .await
}

#[tauri::command]
async fn delete_investigation_case(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    sessions: State<'_, SharedSessions>,
    case_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let case_has_running_session = with_sessions(sessions.inner().clone(), move |sessions| {
        Ok(sessions.has_case(case_id))
    })
    .await?;

    if case_has_running_session {
        return Err(
            "Case используется активной psql-сессией. Сначала отключите эту сессию.".into(),
        );
    }

    with_workspace(workspace.inner().clone(), move |store| {
        store.delete_case(case_id)
    })
    .await
}

#[tauri::command]
async fn delete_case_artifact(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    artifact_id: i64,
    delete_comparisons: bool,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.delete_artifact(artifact_id, delete_comparisons)
    })
    .await
}

#[tauri::command]
async fn delete_case_comparison(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    comparison_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.delete_comparison(comparison_id)
    })
    .await
}

#[tauri::command]
async fn delete_privacy_scope(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    scope_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.delete_scope(scope_id)
    })
    .await
}

#[tauri::command]
async fn reset_privacy_mapping(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    scope_id: i64,
    force: bool,
) -> Result<PrivacyMappingInfo, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    with_mapping_revision_update(workspace.inner().clone(), scope_id, move |store, mapping_path| {
        let usage = store.scope_usage(scope_id)?;
        if (usage.case_count > 0 || usage.artifact_count > 0) && !force {
            return Err(format!(
                "Этот mapping используется {} case(s) и {} artifact(s). Очистка сделает существующие анонимизированные данные необратимыми. Повторите с явным подтверждением.",
                usage.case_count, usage.artifact_count,
            ));
        }
        let info = privacy::reset_mapping(mapping_path)?;
        Ok((info, true))
    })
    .await
}

#[tauri::command]
async fn privacy_mapping_info(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    scope_id: i64,
) -> Result<PrivacyMappingInfo, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let mapping_path = with_workspace(workspace.inner().clone(), move |store| {
        store.scope_mapping_path(scope_id)
    })
    .await?;

    run_privacy(move || privacy::mapping_info_from_path(&mapping_path)).await
}

#[tauri::command]
async fn privacy_mapping_entries(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    scope_id: i64,
    query: Option<String>,
    category: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<PrivacyMappingEntry>, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let mapping_path = with_workspace(workspace.inner().clone(), move |store| {
        store.scope_mapping_path(scope_id)
    })
    .await?;

    let limit = limit.unwrap_or(500).clamp(1, 5_000) as usize;

    run_privacy(move || {
        privacy::list_mapping_entries(&mapping_path, query.as_deref(), category.as_deref(), limit)
    })
    .await
}

#[tauri::command]
async fn delete_privacy_mapping_entries(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    scope_id: i64,
    entries: Vec<PrivacyMappingEntryKey>,
    force: bool,
) -> Result<PrivacyMappingDeleteResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().write_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    with_mapping_revision_update(workspace.inner().clone(), scope_id, move |store, mapping_path| {
        let usage = store.scope_usage(scope_id)?;
        if (usage.case_count > 0 || usage.artifact_count > 0) && !force {
            return Err(format!(
                "Этот mapping используется {} case(s) и {} artifact(s). Удаление отдельных entries может сделать существующие анонимизированные данные необратимыми. Повторите с явным подтверждением.",
                usage.case_count, usage.artifact_count,
            ));
        }
        let result = privacy::delete_mapping_entries(mapping_path, &entries)?;
        let changed = result.deleted > 0;
        Ok((result, changed))
    })
    .await
}

#[tauri::command]
async fn privacy_transform(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    scope_id: i64,
    mode: PrivacyMode,
    operation: PrivacyOperation,
    input: String,
) -> Result<PrivacyTransformResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let (mapping_path, result_privacy) = with_workspace(workspace.inner().clone(), move |store| {
        let scope = store.scope_by_id(scope_id)?;
        Ok((PathBuf::from(scope.mapping_path), scope.result_privacy))
    })
    .await?;

    run_privacy(move || {
        privacy::transform_with_result_config(
            &mapping_path,
            mode,
            operation,
            &input,
            &result_privacy,
        )
    })
    .await
}

#[tauri::command]
async fn anonymize_case_artifact(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    artifact_id: i64,
    mode: PrivacyMode,
) -> Result<CasePrivacyResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let (scope_id, mapping_path, input_path, mapping_revision, result_privacy) =
        with_workspace(workspace.inner().clone(), move |store| {
            let case = store.case_by_id(case_id)?;

            let artifact = store.artifact_by_id(artifact_id)?;

            if artifact.case_id != case_id {
                return Err("Artifact принадлежит другому case".into());
            }

            let scope = store.scope_by_id(case.scope_id)?;

            Ok((
                case.scope_id,
                PathBuf::from(scope.mapping_path),
                store.artifact_path(artifact_id)?,
                store.scope_revision(case.scope_id)?,
                scope.result_privacy,
            ))
        })
        .await?;

    let transform = run_privacy(move || {
        privacy::transform_file_with_result_config(
            &mapping_path,
            mode,
            PrivacyOperation::Anonymize,
            &input_path,
            &result_privacy,
        )
    })
    .await?;

    let output = transform.output.clone();

    let engine_version = privacy::engine_version().to_owned();

    let transform_mode = match mode {
        PrivacyMode::Text => "text",
        PrivacyMode::TextKnown => "text_known",
        PrivacyMode::Sql => "sql",
        PrivacyMode::Explain => "explain",
        PrivacyMode::Result => "result",
    }
    .to_owned();

    let share = with_workspace(workspace.inner().clone(), move |store| {
        store.write_share_artifact_with_lineage(
            case_id,
            artifact_id,
            &output,
            "anonymized",
            "anonymized",
            scope_id,
            mapping_revision,
            &engine_version,
            &transform_mode,
        )
    })
    .await?;

    Ok(CasePrivacyResult {
        artifact: share.artifact,
        transform,
    })
}

#[tauri::command]
async fn case_context(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
) -> Result<CaseContext, String> {
    require_authenticated(auth.inner().clone()).await?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.case_context(case_id)
    })
    .await
}

#[tauri::command]
async fn set_case_artifact_role(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    artifact_id: i64,
    role: Option<String>,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    let role = role.as_deref().map(ArtifactRole::parse).transpose()?;

    with_workspace(workspace.inner().clone(), move |store| {
        store.set_artifact_role(artifact_id, role)
    })
    .await
}

#[tauri::command]
async fn export_xid_csv(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    paths: State<'_, SharedPathSettings>,
    filename: String,
    content: String,
) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;
    let paths = paths.inner().clone();
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let export_dir = paths
            .lock()
            .map_err(|_| "Path settings pgXray недоступны")?
            .export_dir
            .clone();
        let directory = prepare_export_directory(&export_dir)?;
        write_xid_csv_no_clobber(&directory, &filename, &content)
    })
    .await
    .map_err(|_| "Не удалось завершить XID CSV export")??;
    Ok(saved.to_string_lossy().into_owned())
}

#[tauri::command]
async fn export_case_artifact(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    paths: State<'_, SharedPathSettings>,
    artifact_id: i64,
) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let workspace = workspace.inner().clone();
    let paths = paths.inner().clone();

    let path = tauri::async_runtime::spawn_blocking(move || {
        let mut workspace = workspace
            .lock()
            .map_err(|_| "Workspace pgXray недоступен")?;
        let paths = paths
            .lock()
            .map_err(|_| "Path settings pgXray недоступны")?;

        let artifact = workspace.artifact_by_id(artifact_id)?;
        let source = workspace.artifact_path(artifact_id)?;
        let directory = prepare_export_directory(&paths.export_dir)?;
        let target = export_artifact_no_clobber(&directory, &source, &artifact.original_name)?;

        Ok::<PathBuf, String>(target)
    })
    .await
    .map_err(|_| "Не удалось завершить export artifact")??;

    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn reveal_case_artifact(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    artifact_id: i64,
) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;

    let directory = with_workspace(workspace.inner().clone(), move |store| {
        store.artifact_parent_path(artifact_id)
    })
    .await?;

    let directory_for_open = directory.clone();

    tauri::async_runtime::spawn_blocking(move || {
        StdCommand::new("/usr/bin/xdg-open")
            .arg(&directory_for_open)
            .spawn()
            .map_err(|error| {
                format!(
                    "Не удалось открыть каталог {}: {error}",
                    directory_for_open.display(),
                )
            })?;

        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "Не удалось запустить file manager".to_owned())??;

    Ok(directory.to_string_lossy().into_owned())
}

#[tauri::command]
async fn open_plan_workspace(
    app: AppHandle,
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    artifact_id: i64,
) -> Result<(), String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let artifact = with_workspace(workspace.inner().clone(), move |store| {
        let artifact = store.artifact_by_id(artifact_id)?;

        if artifact.case_id != case_id {
            return Err("Artifact принадлежит другому case".into());
        }

        Ok(artifact)
    })
    .await?;

    let label = format!("plan-{case_id}-{artifact_id}");

    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;

        return Ok(());
    }

    let context_script = format!(
        r#"history.replaceState(
        null,
        "",
        "plan.html?case_id={case_id}&artifact_id={artifact_id}"
    );"#,
    );

    WebviewWindowBuilder::new(&app, label, WebviewUrl::App("plan.html".into()))
        .initialization_script(context_script)
        .title(format!("pgXray · {}", artifact.original_name,))
        .inner_size(1700.0, 1000.0)
        .min_inner_size(1100.0, 700.0)
        .resizable(true)
        .build()
        .map_err(|error| format!("Не удалось открыть Plan Workspace: {error}"))?;

    Ok(())
}

#[tauri::command]
async fn load_plan_workspace(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    artifact_id: i64,
) -> Result<PlanWorkspaceData, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let (case, artifact, context, local_plan_json, mapping_path, scope_revision, share) =
        with_workspace(workspace.inner().clone(), move |store| {
            let case = store.case_by_id(case_id)?;

            let artifact = store.artifact_by_id(artifact_id)?;

            if artifact.case_id != case_id {
                return Err("Artifact принадлежит другому case".into());
            }

            let local_plan_json = store.read_text_artifact(artifact_id, 32 * 1024 * 1024)?;

            let mapping_path = store.scope_mapping_path(case.scope_id)?;

            let scope_revision = store.scope_revision(case.scope_id)?;

            let share = store.current_share_for_source(artifact_id, "explain")?;

            let context = store.case_context(case_id)?;

            Ok((
                case,
                artifact,
                context,
                local_plan_json,
                mapping_path,
                scope_revision,
                share,
            ))
        })
        .await?;

    // Local Plan Workspace never persists a new mapping just because a plan
    // was opened. We build an in-memory anonymous preview for the mixed view.
    // Stable mappings are persisted only by the explicit `prepare_plan_share`.
    let local_for_preview = local_plan_json.clone();
    let mapping_for_preview = mapping_path.clone();

    let preview = run_privacy(move || {
        privacy::preview_anonymize(
            &mapping_for_preview,
            PrivacyMode::Explain,
            &local_for_preview,
        )
    })
    .await?;

    let anonymous_plan_json = preview.transform.output.clone();

    let local_for_inspection = local_plan_json.clone();

    let inspection = run_privacy(move || plan::inspect_json(&local_for_inspection)).await?;

    Ok(PlanWorkspaceData {
        case,
        artifact,
        context,
        local_plan_json,
        anonymous_plan_json,
        scope_revision,
        inspection,
        share,
    })
}

#[tauri::command]
async fn prepare_plan_share(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    workspace: State<'_, SharedWorkspace>,
    case_id: i64,
    artifact_id: i64,
) -> Result<PlanShareResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let (scope_id, mapping_path, input_path, mapping_revision) =
        with_workspace(workspace.inner().clone(), move |store| {
            let case = store.case_by_id(case_id)?;

            let artifact = store.artifact_by_id(artifact_id)?;

            if artifact.case_id != case_id {
                return Err("Artifact принадлежит другому case".into());
            }

            Ok((
                case.scope_id,
                store.scope_mapping_path(case.scope_id)?,
                store.artifact_path(artifact_id)?,
                store.scope_revision(case.scope_id)?,
            ))
        })
        .await?;

    let transform = run_privacy(move || {
        privacy::transform_file(
            &mapping_path,
            PrivacyMode::Explain,
            PrivacyOperation::Anonymize,
            &input_path,
        )
    })
    .await?;

    let output = transform.output.clone();
    let engine_version = privacy::engine_version().to_owned();

    let share = with_workspace(workspace.inner().clone(), move |store| {
        store.write_share_artifact_with_lineage(
            case_id,
            artifact_id,
            &output,
            "anonymized",
            "anonymized",
            scope_id,
            mapping_revision,
            &engine_version,
            "explain",
        )
    })
    .await?;

    Ok(PlanShareResult { share, transform })
}

#[tauri::command]
async fn inspect_plan_json(
    auth: State<'_, SharedAuth>,
    input: String,
) -> Result<PlanInspection, String> {
    require_authenticated(auth.inner().clone()).await?;
    if input.len() > 32 * 1024 * 1024 {
        return Err("EXPLAIN JSON превышает лимит 32 MiB".into());
    }
    run_privacy(move || plan::inspect_json(&input)).await
}

#[tauri::command]
async fn compare_plan_json(
    auth: State<'_, SharedAuth>,
    baseline: String,
    candidate: String,
) -> Result<PlanComparison, String> {
    require_authenticated(auth.inner().clone()).await?;
    const MAX_JSON: usize = 32 * 1024 * 1024;
    if baseline.len() > MAX_JSON || candidate.len() > MAX_JSON {
        return Err("Каждый EXPLAIN JSON должен быть не больше 32 MiB".into());
    }
    run_privacy(move || plan::compare_json(&baseline, &candidate)).await
}

#[tauri::command]
async fn inspect_plan_artifact(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    artifact_id: i64,
) -> Result<PlanInspection, String> {
    require_authenticated(auth.inner().clone()).await?;

    let content = with_workspace(workspace.inner().clone(), move |store| {
        store.read_text_artifact(artifact_id, 32 * 1024 * 1024)
    })
    .await?;

    run_privacy(move || plan::inspect_json(&content)).await
}

#[tauri::command]
async fn preview_plan_artifacts(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    baseline_artifact_id: i64,
    candidate_artifact_id: i64,
) -> Result<PlanComparison, String> {
    require_authenticated(auth.inner().clone()).await?;

    let (baseline, candidate) = with_workspace(workspace.inner().clone(), move |store| {
        let baseline_artifact = store.artifact_by_id(baseline_artifact_id)?;
        let candidate_artifact = store.artifact_by_id(candidate_artifact_id)?;

        if baseline_artifact.case_id != candidate_artifact.case_id {
            return Err("Планы для сравнения должны принадлежать одному case".into());
        }

        Ok((
            store.read_text_artifact(baseline_artifact_id, 32 * 1024 * 1024)?,
            store.read_text_artifact(candidate_artifact_id, 32 * 1024 * 1024)?,
        ))
    })
    .await?;

    run_privacy(move || plan::compare_json(&baseline, &candidate)).await
}

#[tauri::command]
async fn compare_plan_artifacts(
    auth: State<'_, SharedAuth>,
    workspace: State<'_, SharedWorkspace>,
    baseline_artifact_id: i64,
    candidate_artifact_id: i64,
) -> Result<PlanArtifactComparisonResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    let (case_id, baseline, candidate) = with_workspace(workspace.inner().clone(), move |store| {
        let baseline_artifact = store.artifact_by_id(baseline_artifact_id)?;
        let candidate_artifact = store.artifact_by_id(candidate_artifact_id)?;

        if baseline_artifact.case_id != candidate_artifact.case_id {
            return Err("Планы для сравнения должны принадлежать одному case".into());
        }

        Ok((
            baseline_artifact.case_id,
            store.read_text_artifact(baseline_artifact_id, 32 * 1024 * 1024)?,
            store.read_text_artifact(candidate_artifact_id, 32 * 1024 * 1024)?,
        ))
    })
    .await?;

    let report = run_privacy(move || plan::compare_json(&baseline, &candidate)).await?;
    let report_for_history = report.clone();

    let comparison = with_workspace(workspace.inner().clone(), move |store| {
        store.record_comparison_with_result(
            case_id,
            ComparisonKind::SavedPlan,
            &[
                (
                    baseline_artifact_id,
                    ComparisonPurpose::Plan,
                    ComparisonSide::Baseline,
                ),
                (
                    candidate_artifact_id,
                    ComparisonPurpose::Plan,
                    ComparisonSide::Candidate,
                ),
            ],
            &report_for_history,
        )
    })
    .await?;

    Ok(PlanArtifactComparisonResult { report, comparison })
}

#[tauri::command]
async fn plan_reference(auth: State<'_, SharedAuth>) -> Result<Vec<PlanReferenceEntry>, String> {
    require_authenticated(auth.inner().clone()).await?;

    Ok(plan::plan_reference())
}

#[tauri::command]
async fn get_explain_analysis_prompt(auth: State<'_, SharedAuth>) -> Result<String, String> {
    require_authenticated(auth.inner().clone()).await?;

    Ok(plan::explain_analysis_prompt().to_owned())
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn compare_case_query_results(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    case_id: i64,
    baseline_artifact_id: i64,
    candidate_artifact_id: i64,
    timeout_ms: u64,
) -> Result<QueryResultComparisonResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let (connection_id, baseline_sql, candidate_sql) =
        with_workspace(workspace.inner().clone(), move |store| {
            let case = store.case_by_id(case_id)?;

            let baseline = store.artifact_by_id(baseline_artifact_id)?;
            let candidate = store.artifact_by_id(candidate_artifact_id)?;

            if baseline.case_id != case_id || candidate.case_id != case_id {
                return Err("SQL artifacts должны принадлежать выбранному case".into());
            }

            Ok((
                case.connection_id,
                store.read_text_artifact(baseline_artifact_id, 2 * 1024 * 1024)?,
                store.read_text_artifact(candidate_artifact_id, 2 * 1024 * 1024)?,
            ))
        })
        .await?;

    let _diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    let report = plan::compare_results(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        &baseline_sql,
        &candidate_sql,
        timeout_ms,
    )
    .await?;
    let report_for_history = report.clone();

    let comparison = with_workspace(workspace.inner().clone(), move |store| {
        store.record_comparison_with_result(
            case_id,
            ComparisonKind::QueryResult,
            &[
                (
                    baseline_artifact_id,
                    ComparisonPurpose::Query,
                    ComparisonSide::Baseline,
                ),
                (
                    candidate_artifact_id,
                    ComparisonPurpose::Query,
                    ComparisonSide::Candidate,
                ),
            ],
            &report_for_history,
        )
    })
    .await?;

    Ok(QueryResultComparisonResult { report, comparison })
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command/call signature must remain compatible"
)]
async fn compare_case_query_plans(
    auth: State<'_, SharedAuth>,
    lifecycle: State<'_, SharedLifecycle>,
    connections: State<'_, SharedConnections>,
    workspace: State<'_, SharedWorkspace>,
    operation_limits: State<'_, SharedOperationLimits>,
    internal_postgres: State<'_, SharedInternalPostgres>,
    case_id: i64,
    baseline_artifact_id: i64,
    candidate_artifact_id: i64,
    analyze_queries: bool,
    timeout_ms: u64,
) -> Result<QueryPlanComparisonResult, String> {
    require_authenticated(auth.inner().clone()).await?;

    // Keep multi-step operations serialized with destructive lifecycle changes.
    let _lifecycle = lifecycle.inner().clone().read_owned().await;
    require_authenticated(auth.inner().clone()).await?;

    let (connection_id, baseline_sql, candidate_sql) =
        with_workspace(workspace.inner().clone(), move |store| {
            let case = store.case_by_id(case_id)?;

            let baseline = store.artifact_by_id(baseline_artifact_id)?;
            let candidate = store.artifact_by_id(candidate_artifact_id)?;

            if baseline.case_id != case_id || candidate.case_id != case_id {
                return Err("SQL artifacts должны принадлежать выбранному case".into());
            }

            Ok((
                case.connection_id,
                store.read_text_artifact(baseline_artifact_id, 2 * 1024 * 1024)?,
                store.read_text_artifact(candidate_artifact_id, 2 * 1024 * 1024)?,
            ))
        })
        .await?;

    let _diagnostic =
        acquire_diagnostic_permit(operation_limits.inner().clone(), connection_id).await?;

    require_authenticated(auth.inner().clone()).await?;

    let input = with_connections(connections.inner().clone(), move |store| {
        store.load_input(connection_id)
    })
    .await?;

    let report = plan::compare_query_plans(
        internal_postgres.inner().as_ref(),
        connection_id,
        &input,
        &baseline_sql,
        &candidate_sql,
        analyze_queries,
        timeout_ms,
    )
    .await?;

    let baseline_plan = report.baseline_plan_json.clone();
    let candidate_plan = report.candidate_plan_json.clone();
    let comparison_report_for_history = report.comparison.clone();

    let prefix = if analyze_queries {
        "analyze"
    } else {
        "explain"
    };

    let comparison_kind = if analyze_queries {
        ComparisonKind::QueryAnalyze
    } else {
        ComparisonKind::QueryExplain
    };

    // The artifacts and comparison are one logical operation. Compensate for
    // ordinary errors by deleting newly-created files/rows before returning.
    let (baseline_artifact, candidate_artifact, comparison) =
        with_workspace(workspace.inner().clone(), move |store| {
            let baseline_artifact = store.write_generated_artifact(
                case_id,
                &format!("{prefix}-baseline.json"),
                &baseline_plan,
                "explain",
                Some(ArtifactRole::Plan),
            )?;

            let candidate_artifact = match store.write_generated_artifact(
                case_id,
                &format!("{prefix}-candidate.json"),
                &candidate_plan,
                "explain",
                Some(ArtifactRole::Plan),
            ) {
                Ok(artifact) => artifact,
                Err(error) => {
                    let rollback = store.delete_artifact(baseline_artifact.id, false);
                    return match rollback {
                        Ok(()) => Err(error),
                        Err(rollback_error) => Err(format!(
                            "{error}; также не удалось удалить baseline artifact после сбоя: {rollback_error}"
                        )),
                    };
                }
            };

            let comparison = store.record_comparison_with_result(
                case_id,
                comparison_kind,
                &[
                    (baseline_artifact_id, ComparisonPurpose::Query, ComparisonSide::Baseline),
                    (candidate_artifact_id, ComparisonPurpose::Query, ComparisonSide::Candidate),
                    (baseline_artifact.id, ComparisonPurpose::Plan, ComparisonSide::Baseline),
                    (candidate_artifact.id, ComparisonPurpose::Plan, ComparisonSide::Candidate),
                ],
                &comparison_report_for_history,
            );
            match comparison {
                Ok(comparison) => Ok((baseline_artifact, candidate_artifact, comparison)),
                Err(error) => {
                    let candidate_rollback = store.delete_artifact(candidate_artifact.id, false);
                    let baseline_rollback = store.delete_artifact(baseline_artifact.id, false);
                    match (candidate_rollback, baseline_rollback) {
                        (Ok(()), Ok(())) => Err(error),
                        (candidate_error, baseline_error) => Err(format!(
                            "{error}; очистка generated artifacts не завершена: candidate={candidate_error:?}, baseline={baseline_error:?}"
                        )),
                    }
                }
            }
        })
        .await?;

    Ok(QueryPlanComparisonResult {
        report,
        comparison,
        baseline_artifact,
        candidate_artifact,
    })
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let directory = app.path().app_data_dir()?;

            fs::create_dir_all(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;

            let auth_path = directory.join("auth.sqlite3");
            let auth = Auth::open(&auth_path).map_err(std::io::Error::other)?;

            fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600))?;

            app.manage(Arc::new(Mutex::new(auth)));

            let connections_path = directory.join("connections.sqlite3");

            let credentials_dir = directory.join("credentials");

            let connections = ConnectionStore::open(&connections_path, &credentials_dir)
                .map_err(std::io::Error::other)?;

            fs::set_permissions(&connections_path, fs::Permissions::from_mode(0o600))?;

            app.manage(Arc::new(Mutex::new(connections)));

            let diagnostics_path = directory.join("diagnostics.sqlite3");

            let diagnostics =
                DiagnosticStore::open(&diagnostics_path).map_err(std::io::Error::other)?;

            fs::set_permissions(&diagnostics_path, fs::Permissions::from_mode(0o600))?;

            app.manage(Arc::new(Mutex::new(diagnostics)));

            let workspace_path = directory.join("workspace.sqlite3");
            let path_settings_dir = directory.join("path-settings");
            let default_workspace_root = directory.join("workspaces");
            let default_export_dir = default_export_directory(&directory);
            let default_application_log_dir = directory.join("logs");
            let path_settings = AppPathSettings::open(
                path_settings_dir,
                workspace_path.clone(),
                default_workspace_root,
                default_export_dir,
                default_application_log_dir,
            )
            .map_err(std::io::Error::other)?;
            let workspace_root = path_settings.workspace_root.clone();
            let application_log_dir = path_settings.application_log_dir.clone();

            let application_log = Arc::new(ApplicationLog::open_best_effort(
                &application_log_dir,
                &directory.join("app-log-settings.json"),
            ));
            app.manage(Arc::clone(&application_log));

            let workspace = WorkspaceStore::open(&workspace_path, &workspace_root)
                .map_err(std::io::Error::other)?;

            fs::set_permissions(&workspace_path, fs::Permissions::from_mode(0o600))?;

            app.manage(Arc::new(Mutex::new(workspace)));
            app.manage(Arc::new(Mutex::new(path_settings)));

            app.manage(Arc::new(tokio::sync::RwLock::new(())));
            app.manage(Arc::new(OperationLimits::new()));
            app.manage(Arc::new(InternalPostgresRegistry::with_application_log(
                Arc::clone(&application_log),
            )));

            let psql_config_dir = directory.join("psql");

            let sessions =
                PsqlSessionManager::new(&psql_config_dir).map_err(std::io::Error::other)?;

            app.manage(Arc::new(Mutex::new(sessions)));

            application_log.record(ApplicationLogEvent {
                level: "info",
                component: "application",
                operation: "pgxray",
                phase: "started",
                actor: None,
                request_id: None,
                duration_ms: None,
                context: None,
                error: None,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_backend_locale,
            open_external_url,
            application_log_event,
            application_log_state,
            set_application_log_settings,
            list_application_log_entries,
            query_application_log,
            clear_application_log,
            reveal_application_log_directory,
            auth_state,
            register_initial_user,
            login,
            list_users,
            create_user,
            rename_user,
            change_user_password,
            delete_user,
            logout,
            probe,
            list_connections,
            list_connection_projects,
            create_connection_project,
            rename_connection_project,
            delete_connection_project,
            assign_connection_project,
            assign_connection_shortcut,
            save_connection,
            probe_saved_connection,
            delete_saved_connection,
            probe_connection_draft,
            update_saved_connection,
            start_psql_session,
            subscribe_psql_session,
            list_psql_sessions,
            send_psql_session_input,
            resize_psql_session,
            close_psql_session,
            path_settings,
            runtime_readiness,
            set_export_directory,
            set_application_log_directory,
            migrate_workspace_root,
            list_snapshot_schedules,
            put_snapshot_schedule,
            remove_snapshot_schedule,
            claim_snapshot_schedule,
            history_storage_health,
            update_history_storage_policy,
            maintain_history_storage,
            protect_history_snapshot,
            history_maintenance_tick,
            history_retention_settings,
            set_history_retention,
            list_history_connections,
            delete_connection_history,
            postgres_activity_snapshot,
            postgres_lock_snapshot,
            postgres_lock_incident_capture,
            save_lock_incident_to_case,
            signal_postgres_backend,
            collect_xid_snapshot,
            export_xid_csv,
            xid_reference,
            list_xid_snapshots,
            xid_history_overview,
            load_xid_snapshot,
            xid_relation_history,
            xid_relation_policy,
            apply_xid_policy_candidate,
            compare_xid_snapshots,
            collect_pgss_snapshot,
            pgss_reference,
            list_pgss_snapshots,
            load_pgss_snapshot,
            compare_pgss_snapshots,
            pgss_statement_history,
            pgss_workload_history,
            pgss_query_detail,
            pgss_explore,
            collect_io_wal_snapshot,
            io_wal_reference,
            list_io_wal_snapshots,
            load_io_wal_snapshot,
            compare_io_wal_snapshots,
            io_wal_workload_history,
            collect_index_health_snapshot,
            index_health_reference,
            list_index_health_snapshots,
            load_index_health_snapshot,
            compare_index_health_snapshots,
            collect_autovacuum_snapshot,
            autovacuum_reference,
            autovacuum_table_detail,
            simulate_autovacuum_policy,
            list_autovacuum_snapshots,
            load_autovacuum_snapshot,
            autovacuum_relation_history,
            autovacuum_workload_history,
            autovacuum_maintenance_advice,
            autovacuum_policy_candidates,
            apply_autovacuum_policy_candidate,
            compare_autovacuum_snapshots,
            collect_pg_settings_snapshot,
            list_pg_settings_snapshots,
            list_pg_settings_changes,
            pg_settings_history_analysis,
            load_pg_settings_snapshot,
            compare_pg_settings_snapshots,
            compare_what_changed,
            list_change_markers,
            create_change_marker,
            delete_change_marker,
            save_what_changed_to_case,
            ensure_connection_workspace,
            list_privacy_scopes,
            create_privacy_scope,
            update_privacy_scope_result_policy,
            create_investigation_case,
            list_investigation_cases,
            update_investigation_case_description,
            create_case_intervention,
            list_workspace_changes,
            link_workspace_change_to_case,
            delete_case_intervention,
            record_case_change_impact,
            delete_case_change_impact,
            list_case_artifacts,
            pick_case_import_file,
            import_case_file,
            create_case_file,
            update_case_file,
            read_case_artifact,
            privacy_mapping_info,
            privacy_mapping_entries,
            delete_privacy_mapping_entries,
            privacy_transform,
            anonymize_case_artifact,
            delete_investigation_case,
            delete_case_artifact,
            delete_case_comparison,
            delete_privacy_scope,
            reset_privacy_mapping,
            case_context,
            set_case_artifact_role,
            export_case_artifact,
            reveal_case_artifact,
            open_plan_workspace,
            load_plan_workspace,
            prepare_plan_share,
            inspect_plan_json,
            compare_plan_json,
            inspect_plan_artifact,
            preview_plan_artifacts,
            compare_plan_artifacts,
            plan_reference,
            get_explain_analysis_prompt,
            compare_case_query_results,
            compare_case_query_plans,
        ])
        .build(tauri::generate_context!())
        .expect("Не удалось собрать pgXray application");

    app.run(|app_handle, event| {
        if !matches!(
            &event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            return;
        }

        if let Some(application_log) = app_handle.try_state::<SharedApplicationLog>() {
            let phase = if matches!(&event, tauri::RunEvent::ExitRequested { .. }) {
                "exit_requested"
            } else {
                "exited"
            };
            application_log.record(ApplicationLogEvent {
                level: "info",
                component: "application",
                operation: "pgxray",
                phase,
                actor: None,
                request_id: None,
                duration_ms: None,
                context: None,
                error: None,
            });
        }

        if let Some(sessions) = app_handle.try_state::<SharedSessions>() {
            if let Ok(mut sessions) = sessions.inner().lock() {
                sessions.close_all();
            }
        }

        if let Some(operation_limits) = app_handle.try_state::<SharedOperationLimits>() {
            operation_limits.clear_terminals();
        }

        if let Some(internal_postgres) = app_handle.try_state::<SharedInternalPostgres>() {
            let internal_postgres = Arc::clone(internal_postgres.inner());
            tauri::async_runtime::block_on(async move {
                internal_postgres.shutdown_all().await;
            });
        }
    });
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pgxray-desktop-safety-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn xid_csv_export_rejects_arbitrary_paths_and_names() {
        for bad in [
            "../../postgresql.conf",
            "pgxray-xid-relations-1.json",
            "pgxray-xid-relations-1/2.csv",
            "pgxray-xid-relations-.csv",
            "pgxray-xid-comparison-1.csv",
            "pgxray-xid-relations-1-2.csv",
        ] {
            assert!(!xid_csv_name_valid(bad), "unexpected valid name: {bad}");
        }
        assert!(xid_csv_name_valid("pgxray-xid-relations-17.csv"));
        assert!(xid_csv_name_valid("pgxray-xid-comparison-17-19.csv"));
    }

    #[test]
    fn external_link_validation_rejects_unsafe_schemes_and_credentials() {
        assert!(
            validated_external_url("https://www.postgresql.org/docs/16/monitoring-stats.html")
                .is_ok()
        );
        assert!(validated_external_url("http://www.postgresql.org/docs/").is_ok());
        for link in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "mailto:user@example.org",
            "https://user:secret@example.org/path",
            "https://",
            "not a url",
        ] {
            assert!(validated_external_url(link).is_err(), "unsafe link: {link}");
        }
    }

    #[test]
    fn xid_csv_export_preserves_existing_file() {
        let root = test_directory();
        let csv = "schema,relation\npublic,orders\n";
        let first = write_xid_csv_no_clobber(&root, "pgxray-xid-relations-5.csv", csv).unwrap();
        let second = write_xid_csv_no_clobber(&root, "pgxray-xid-relations-5.csv", csv).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(first).unwrap(), csv);
        assert_eq!(fs::read_to_string(second).unwrap(), csv);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_never_follows_or_overwrites_an_existing_symlink() {
        let root = test_directory();
        let source = root.join("source.json");
        let protected = root.join("protected.json");
        let directory = root.join("exports");
        fs::create_dir(&directory).unwrap();
        fs::write(&source, b"new content").unwrap();
        fs::write(&protected, b"protected content").unwrap();
        symlink(&protected, directory.join("result.json")).unwrap();

        let exported = export_artifact_no_clobber(&directory, &source, "result.json").unwrap();
        assert_eq!(
            exported.file_name().unwrap().to_str().unwrap(),
            "result-1.json"
        );
        assert_eq!(fs::read(&protected).unwrap(), b"protected content");
        assert_eq!(fs::read(&exported).unwrap(), b"new content");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_skips_dangling_symlink() {
        let root = test_directory();
        let source = root.join("source.txt");
        let directory = root.join("exports");
        fs::create_dir(&directory).unwrap();
        fs::write(&source, b"data").unwrap();
        symlink(root.join("nonexistent"), directory.join("result.txt")).unwrap();

        let exported = export_artifact_no_clobber(&directory, &source, "result.txt").unwrap();
        assert_eq!(
            exported.file_name().unwrap().to_str().unwrap(),
            "result-1.txt"
        );
        assert_eq!(fs::read(exported).unwrap(), b"data");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lifecycle_write_is_blocked_by_active_reader() {
        let lifecycle = Arc::new(tokio::sync::RwLock::new(()));
        let read = Arc::clone(&lifecycle).try_read_owned().unwrap();
        assert!(Arc::clone(&lifecycle).try_write_owned().is_err());
        drop(read);
        assert!(Arc::clone(&lifecycle).try_write_owned().is_ok());
    }

    #[test]
    fn mapping_restore_replaces_modified_file_or_removes_new_file() {
        let root = test_directory();
        let path = root.join("mapping.json");
        fs::write(&path, b"changed").unwrap();
        restore_mapping_bytes(&path, Some(b"original")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"original");
        restore_mapping_bytes(&path, None).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
