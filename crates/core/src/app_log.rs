use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const APPLICATION_LOG_500_MIB: u64 = 500 * 1024 * 1024;
pub const APPLICATION_LOG_1_GIB: u64 = 1024 * 1024 * 1024;
pub const APPLICATION_LOG_3_GIB: u64 = 3 * 1024 * 1024 * 1024;

const DEFAULT_RETENTION_DAYS: u32 = 7;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ACTIVE_FILE_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_DELAY_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplicationLogSettings {
    pub max_total_bytes: u64,
    pub retention_days: u32,
}

impl Default for ApplicationLogSettings {
    fn default() -> Self {
        Self {
            max_total_bytes: APPLICATION_LOG_500_MIB,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

impl ApplicationLogSettings {
    pub fn validate(self) -> Result<Self, String> {
        if ![
            APPLICATION_LOG_500_MIB,
            APPLICATION_LOG_1_GIB,
            APPLICATION_LOG_3_GIB,
        ]
        .contains(&self.max_total_bytes)
        {
            return Err(crate::i18n::text(
                "Лимит application log должен быть 500 MiB, 1 GiB или 3 GiB",
                "Application log limit must be 500 MiB, 1 GiB, or 3 GiB",
            )
            .into());
        }

        if ![7, 30, 90].contains(&self.retention_days) {
            return Err(crate::i18n::text(
                "Retention application log должен быть 7, 30 или 90 дней",
                "Application log retention must be 7, 30, or 90 days",
            )
            .into());
        }

        Ok(self)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApplicationLogEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub component: String,
    pub operation: String,
    pub phase: String,
    pub actor: Option<String>,
    pub request_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub context: Option<String>,
    pub error: Option<String>,
    /// Stable Rust operation identifier linking a diagnostic snapshot with SQL calls.
    /// Missing on historic records and UI-only reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    /// Authoritative summary populated only after a Rust diagnostic save succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<DiagnosticLogDetails>,
    /// Typed, Rust-confirmed non-diagnostic outcome. Never contains credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_result: Option<OperationLogResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticLogDetails {
    pub connection_id: i64,
    pub kind: String,
    pub snapshot_id: Option<i64>,
    pub unavailable_sections: Option<i64>,
    pub total_sections: Option<i64>,
    /// Untrusted UI correlation token. Rust operation_id remains authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationLogResult {
    Authentication { authenticated: bool },
    ConnectionProjects { count: Option<u64>, succeeded: bool },
}

pub struct ApplicationLogEvent<'a> {
    pub level: &'a str,
    pub component: &'a str,
    pub operation: &'a str,
    pub phase: &'a str,
    pub actor: Option<&'a str>,
    pub request_id: Option<&'a str>,
    pub duration_ms: Option<u64>,
    pub context: Option<&'a str>,
    pub error: Option<&'a str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationLogStatus {
    pub directory: String,
    pub available: bool,
    pub last_error: Option<String>,
    pub total_bytes: u64,
    pub file_count: usize,
    pub oldest_at_ms: Option<u64>,
    pub newest_at_ms: Option<u64>,
    pub max_file_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationLogState {
    pub settings: ApplicationLogSettings,
    pub status: ApplicationLogStatus,
}

/// One bounded page of the on-disk JSONL journal. Filters are applied in
/// Rust before pagination; results are not restricted to the newest 1000 events.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ApplicationLogQuery {
    pub cursor: Option<String>,
    pub limit: u32,
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub level: Option<String>,
    pub component: Option<String>,
    /// Logical pgXray area. Resolved from recorded operation/component before pagination.
    pub section: Option<String>,
    pub operation: Option<String>,
    pub actor: Option<String>,
    pub request_id: Option<String>,
    pub operation_id: Option<String>,
    /// Match both UI request_id and verified Rust diagnostic.client_request_id.
    pub client_request_id: Option<String>,
    pub problems_only: bool,
    pub min_duration_ms: Option<u64>,
    pub search: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationLogPage {
    pub entries: Vec<ApplicationLogEntry>,
    pub next_cursor: Option<String>,
    pub scanned_bytes: u64,
    pub malformed_lines: u64,
    /// True if the query stopped at a per-call scan budget; follow next_cursor
    /// to continue scanning even if this page contains no matching entries.
    pub scan_limit_reached: bool,
}

const LOG_QUERY_MAX_LIMIT: u32 = 200;
const LOG_QUERY_SCAN_BUDGET: u64 = 64 * 1024 * 1024;
const LOG_QUERY_MAX_FILE_SIZE: u64 = MAX_FILE_BYTES + 16 * 1024;

impl ApplicationLogQuery {
    fn validate(&self) -> Result<(), String> {
        if !(1..=LOG_QUERY_MAX_LIMIT).contains(&self.limit) {
            return Err(crate::i18n::text(
                "Журнал: limit должен быть от 1 до 200",
                "Application log: limit must be between 1 and 200",
            )
            .into());
        }
        if self.from_ms.zip(self.to_ms).is_some_and(|(a, b)| a > b) {
            return Err(crate::i18n::text(
                "Журнал: начало периода позже конца",
                "Application log: start of period is later than end",
            )
            .into());
        }
        for value in [
            &self.level,
            &self.component,
            &self.section,
            &self.operation,
            &self.actor,
            &self.request_id,
            &self.operation_id,
            &self.client_request_id,
            &self.search,
        ] {
            if value
                .as_ref()
                .is_some_and(|v| v.len() > 256 || v.contains('\0'))
            {
                return Err(crate::i18n::text(
                    "Журнал: слишком длинный фильтр",
                    "Application log: filter is too long",
                )
                .into());
            }
        }
        if self.section.as_deref().is_some_and(|s| {
            !matches!(
                s,
                "connections"
                    | "diagnostics"
                    | "pgss"
                    | "xid"
                    | "autovacuum"
                    | "io_wal"
                    | "index_health"
                    | "pg_settings"
                    | "activity"
                    | "locks"
                    | "postgres"
                    | "sessions"
                    | "plans"
                    | "investigations"
                    | "settings"
                    | "auth"
                    | "application"
                    | "other"
            )
        }) {
            return Err(crate::i18n::text(
                "Журнал: неизвестный раздел поиска",
                "Application log: unknown search field",
            )
            .into());
        }
        if self.cursor.as_ref().is_some_and(|v| v.len() > 256) {
            return Err(crate::i18n::text(
                "Журнал: некорректный курсор",
                "Application log: invalid cursor",
            )
            .into());
        }
        Ok(())
    }

    fn matches(&self, entry: &ApplicationLogEntry) -> bool {
        if self.from_ms.is_some_and(|from| entry.timestamp_ms < from)
            || self.to_ms.is_some_and(|to| entry.timestamp_ms > to)
        {
            return false;
        }
        for (wanted, actual) in [
            (&self.level, Some(entry.level.as_str())),
            (&self.component, Some(entry.component.as_str())),
            (&self.operation, Some(entry.operation.as_str())),
            (&self.actor, entry.actor.as_deref()),
            (&self.request_id, entry.request_id.as_deref()),
            (&self.operation_id, entry.operation_id.as_deref()),
        ] {
            if wanted
                .as_ref()
                .is_some_and(|wanted| Some(wanted.as_str()) != actual)
            {
                return false;
            }
        }
        if self.client_request_id.as_ref().is_some_and(|wanted| {
            let linked = entry
                .diagnostic
                .as_ref()
                .and_then(|d| d.client_request_id.as_deref());
            linked != Some(wanted.as_str()) && entry.request_id.as_deref() != Some(wanted.as_str())
        }) {
            return false;
        }
        if self
            .section
            .as_deref()
            .is_some_and(|section| !journal_section_matches(section, entry))
        {
            return false;
        }
        if self
            .min_duration_ms
            .is_some_and(|minimum| entry.duration_ms.unwrap_or(0) < minimum)
        {
            return false;
        }
        if self.problems_only
            && !matches!(
                entry.level.to_ascii_lowercase().as_str(),
                "error" | "warning" | "warn"
            )
            && !entry.phase.contains("failed")
            && !entry.phase.contains("timeout")
            && entry.error.is_none()
        {
            return false;
        }
        let Some(search) = self.search.as_deref().filter(|s| !s.trim().is_empty()) else {
            return true;
        };
        let needle = search.trim().to_lowercase();
        [
            Some(entry.level.as_str()),
            Some(entry.component.as_str()),
            Some(entry.operation.as_str()),
            Some(entry.phase.as_str()),
            entry.actor.as_deref(),
            entry.request_id.as_deref(),
            entry.operation_id.as_deref(),
            entry
                .diagnostic
                .as_ref()
                .and_then(|d| d.client_request_id.as_deref()),
            entry.context.as_deref(),
            entry.error.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(&needle))
    }
}

/// Classification is based on recorded operation/component, not UI text or the
/// SQL content. Generic internal_postgres calls cannot reliably be attributed to
/// PGSS/XID/etc. without a cross-layer correlation ID.
fn journal_section(entry: &ApplicationLogEntry) -> &'static str {
    // SQL events inherit only the verified diagnostic kind recorded by Rust.
    // Historic uncorrelated SQL remains classified as PostgreSQL.
    if let Some(kind) = entry.diagnostic.as_ref().map(|d| d.kind.as_str()) {
        return match kind {
            "pgss" => "pgss",
            "xid" => "xid",
            "autovacuum" => "autovacuum",
            "io_wal" => "io_wal",
            "index_health" => "index_health",
            "pg_settings" => "pg_settings",
            _ => "other",
        };
    }
    let op = entry.operation.to_ascii_lowercase();
    let component = entry.component.as_str();
    if op.contains("pgss") || op.contains("statement") {
        return "pgss";
    }
    if op.contains("xid") || op.contains("freeze") {
        return "xid";
    }
    if op.contains("autovacuum") {
        return "autovacuum";
    }
    if op.contains("io_wal") || op.contains("wal_snapshot") {
        return "io_wal";
    }
    if op.contains("index_health") {
        return "index_health";
    }
    if op.contains("pg_settings") {
        return "pg_settings";
    }
    if op.contains("activity")
        || op.contains("backend_signal")
        || op.contains("signal_postgres_backend")
    {
        return "activity";
    }
    if component == "locks" || op.contains("lock_incident") {
        return "locks";
    }
    if op == "internal_postgres" {
        return "postgres";
    }
    if op.contains("psql") || op.contains("session") {
        return "sessions";
    }
    if op.contains("plan") || op.contains("explain") {
        return "plans";
    }
    if op.contains("privacy")
        || op.contains("case")
        || op.contains("artifact")
        || op.contains("workspace")
        || op.contains("what_changed")
        || component == "what_changed"
    {
        return "investigations";
    }
    if op.contains("connection") || op.contains("probe") || op.contains("project") {
        return "connections";
    }
    if op.contains("user") || op == "login" || op == "logout" {
        return "auth";
    }
    if component == "app_log" {
        return "application";
    }
    if op.contains("settings") || op.contains("directory") || op.contains("retention") {
        return "settings";
    }
    if component == "app_log" || component == "application" || op == "pgxray" || op.contains("log")
    {
        return "application";
    }
    "other"
}

fn journal_section_matches(wanted: &str, entry: &ApplicationLogEntry) -> bool {
    let actual = journal_section(entry);
    actual == wanted
        || (wanted == "diagnostics"
            && matches!(
                actual,
                "pgss"
                    | "xid"
                    | "autovacuum"
                    | "io_wal"
                    | "index_health"
                    | "pg_settings"
                    | "activity"
                    | "locks"
                    | "postgres"
            ))
}

// A cursor contains only the name of a managed log file and a byte position.
// It cannot reference an arbitrary file path. The filename is checked against
// the current managed file list on every request.
fn log_cursor(name: &str, before: usize) -> String {
    format!("{name}|{before}")
}

fn parse_log_cursor(cursor: &str) -> Result<(&str, usize), String> {
    let (name, before) = cursor.rsplit_once('|').ok_or(crate::i18n::text(
        "Журнал: некорректный курсор",
        "Application log: invalid cursor",
    ))?;
    if !is_application_log_file(Path::new(name)) || name.contains('/') || name.contains('\\') {
        return Err(crate::i18n::text(
            "Журнал: некорректный курсор",
            "Application log: invalid cursor",
        )
        .into());
    }
    let before = before.parse::<usize>().map_err(|_| {
        crate::i18n::text(
            "Журнал: некорректный курсор",
            "Application log: invalid cursor",
        )
    })?;
    Ok((name, before))
}

struct ActiveLog {
    path: PathBuf,
    file: File,
    bytes: u64,
    started_at_ms: u64,
    sequence: u32,
}

struct ApplicationLogInner {
    settings: ApplicationLogSettings,
    active: Option<ActiveLog>,
    last_error: Option<String>,
    retry_after_ms: u64,
}

pub struct ApplicationLog {
    directory: RwLock<PathBuf>,
    settings_path: PathBuf,
    inner: Mutex<ApplicationLogInner>,
}

impl ApplicationLog {
    /// Opens the logger without making application startup depend on logging.
    /// Any filesystem/settings error is exposed through `state()`, while normal
    /// pgXray operations continue without a writable application log.
    pub fn open_best_effort(directory: &Path, settings_path: &Path) -> Self {
        let (settings, mut last_error) = load_settings_best_effort(settings_path);
        let active = match initialize_active(directory, settings, 0) {
            Ok(active) => Some(active),
            Err(error) => {
                append_state_error(&mut last_error, error);
                None
            }
        };

        Self {
            directory: RwLock::new(directory.to_path_buf()),
            settings_path: settings_path.to_path_buf(),
            inner: Mutex::new(ApplicationLogInner {
                settings,
                active,
                last_error,
                retry_after_ms: 0,
            }),
        }
    }

    /// Best-effort event recording. Write/rotation failures degrade the logger
    /// but are intentionally not propagated into the operation being observed.
    pub fn record(&self, event: ApplicationLogEvent<'_>) {
        self.record_enriched(event, None, None);
    }

    /// Backwards-compatible JSONL extension for trusted Rust telemetry.
    /// Never put SQL text, parameters, credentials or user-controlled JSON here.
    pub fn record_enriched(
        &self,
        event: ApplicationLogEvent<'_>,
        operation_id: Option<&str>,
        diagnostic: Option<DiagnosticLogDetails>,
    ) {
        self.record_full(event, operation_id, diagnostic, None);
    }

    /// Authoritative outcome for authentication/connection operations.
    pub fn record_outcome(&self, event: ApplicationLogEvent<'_>, result: OperationLogResult) {
        self.record_full(event, None, None, Some(result));
    }

    fn record_full(
        &self,
        event: ApplicationLogEvent<'_>,
        operation_id: Option<&str>,
        diagnostic: Option<DiagnosticLogDetails>,
        operation_result: Option<OperationLogResult>,
    ) {
        let timestamp_ms = match unix_time_millis() {
            Ok(value) => value,
            Err(error) => {
                self.mark_unavailable(error);
                return;
            }
        };

        let entry = ApplicationLogEntry {
            timestamp_ms,
            level: sanitize_field(event.level, 16),
            component: sanitize_field(event.component, 64),
            operation: sanitize_field(event.operation, 128),
            phase: sanitize_field(event.phase, 64),
            actor: event.actor.map(|value| sanitize_field(value, 64)),
            request_id: event.request_id.map(|value| sanitize_field(value, 80)),
            duration_ms: event.duration_ms,
            // UI-reported context is untrusted IPC input. Preserve only the
            // numeric/boolean identifiers already emitted by safeInvokeContext.
            context: event.context.and_then(|value| {
                if event.component == "ui_reported" {
                    safe_ui_context(value)
                } else {
                    Some(sanitize_field(value, 2048))
                }
            }),
            // PostgreSQL errors may echo SQL literals (including credentials).
            // The raw error still reaches the caller; the disk journal stores
            // only a bounded, non-sensitive classification.
            error: event.error.map(safe_log_error),
            operation_id: operation_id.map(|value| sanitize_field(value, 80)),
            diagnostic,
            operation_result,
        };

        let mut line = match serde_json::to_vec(&entry) {
            Ok(line) => line,
            Err(error) => {
                self.mark_unavailable(format!(
                    "Не удалось сериализовать application log entry: {error}"
                ));
                return;
            }
        };
        line.push(b'\n');
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        let directory = self.directory();
        let settings = inner.settings;
        if inner.active.is_none() {
            if entry.timestamp_ms < inner.retry_after_ms {
                return;
            }
            inner.retry_after_ms = entry.timestamp_ms.saturating_add(RETRY_DELAY_MS);
            match initialize_active(&directory, settings, 0) {
                Ok(active) => {
                    inner.active = Some(active);
                    inner.last_error = None;
                }
                Err(error) => {
                    inner.last_error = Some(error);
                    return;
                }
            }
        }
        let Some(active) = inner.active.as_mut() else {
            return;
        };

        let active_age_ms = entry.timestamp_ms.saturating_sub(active.started_at_ms);
        let active_too_old = active_age_ms >= MAX_ACTIVE_FILE_AGE.as_millis() as u64;
        let active_too_large =
            active.bytes > 0 && active.bytes.saturating_add(line.len() as u64) > MAX_FILE_BYTES;

        if active_too_old || active_too_large {
            let next_sequence = active.sequence.wrapping_add(1);

            if let Err(error) = rotate_locked(&directory, &mut inner, next_sequence) {
                inner.last_error = Some(error);
                inner.active = None;
                inner.retry_after_ms = entry.timestamp_ms.saturating_add(RETRY_DELAY_MS);
                return;
            }

            if let Some(active_path) = inner.active.as_ref().map(|active| active.path.clone()) {
                if let Err(error) =
                    cleanup_directory(&directory, settings, Some(active_path.as_path()))
                {
                    inner.last_error = Some(error);
                }
            }
        }

        let Some(active) = inner.active.as_mut() else {
            return;
        };

        if let Err(error) = active.file.write_all(&line) {
            inner.last_error = Some(format!("Не удалось записать application log: {error}"));
            inner.active = None;
            inner.retry_after_ms = entry.timestamp_ms.saturating_add(RETRY_DELAY_MS);
            return;
        }

        active.bytes = active.bytes.saturating_add(line.len() as u64);
    }

    pub fn state(&self) -> ApplicationLogState {
        let (settings, available, mut last_error) = {
            let inner = match self.inner.lock() {
                Ok(inner) => inner,
                Err(poisoned) => poisoned.into_inner(),
            };

            (
                inner.settings,
                inner.active.is_some(),
                inner.last_error.clone(),
            )
        };

        let directory = self.directory();
        let scan = scan_status(&directory);
        let (total_bytes, file_count, oldest_at_ms, newest_at_ms) = match scan {
            Ok(status) => status,
            Err(error) => {
                append_state_error(&mut last_error, error);
                (0, 0, None, None)
            }
        };

        ApplicationLogState {
            settings,
            status: ApplicationLogStatus {
                directory: directory.to_string_lossy().into_owned(),
                available,
                last_error,
                total_bytes,
                file_count,
                oldest_at_ms,
                newest_at_ms,
                max_file_bytes: MAX_FILE_BYTES,
            },
        }
    }

    pub fn set_settings(
        &self,
        settings: ApplicationLogSettings,
    ) -> Result<ApplicationLogState, String> {
        let settings = settings.validate()?;
        save_settings(&self.settings_path, settings)?;

        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.settings = settings;

        let directory = self.directory();

        if inner.active.is_none() {
            match initialize_active(&directory, settings, 0) {
                Ok(active) => {
                    inner.active = Some(active);
                    inner.last_error = None;
                    inner.retry_after_ms = 0;
                }
                Err(error) => inner.last_error = Some(error),
            }
        }

        let active_path = inner.active.as_ref().map(|active| active.path.clone());
        if let Err(error) = cleanup_directory(&directory, settings, active_path.as_deref()) {
            inner.last_error = Some(error);
        }
        drop(inner);

        Ok(self.state())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<ApplicationLogEntry>, String> {
        let limit = limit.clamp(1, 1000);
        // Open the entire snapshot while holding the writer's lock. Readers keep
        // valid file descriptors even when a later clear/rotation unlinks files.
        let files = {
            let _inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let directory = self.directory();
            if !directory.exists() {
                return Ok(Vec::new());
            }
            let mut files = log_files(&directory)?;
            files.sort_by(|left, right| {
                right
                    .modified
                    .cmp(&left.modified)
                    .then_with(|| right.path.cmp(&left.path))
            });
            files
                .into_iter()
                .map(|entry| {
                    let file = File::open(&entry.path).map_err(|error| {
                        format!(
                            "Не удалось открыть application log {}: {error}",
                            entry.path.display()
                        )
                    })?;
                    Ok((entry.path, file))
                })
                .collect::<Result<Vec<_>, String>>()?
        };

        let mut result = Vec::with_capacity(limit);
        for (path, input) in files {
            if result.len() >= limit {
                break;
            }
            let remaining = limit - result.len();
            let reader = BufReader::new(input);
            let mut tail = VecDeque::with_capacity(remaining);
            for line in reader.lines() {
                let line = line.map_err(|error| {
                    format!(
                        "Не удалось прочитать application log {}: {error}",
                        path.display()
                    )
                })?;
                let Ok(entry) = serde_json::from_str::<ApplicationLogEntry>(&line) else {
                    continue;
                };
                if tail.len() == remaining {
                    tail.pop_front();
                }
                tail.push_back(entry);
            }
            while let Some(entry) = tail.pop_back() {
                result.push(entry);
            }
        }
        result.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
        result.truncate(limit);
        Ok(result)
    }

    /// Search backwards through the retained JSONL files. A single request
    /// keeps at most one rotated file in memory (<=16MiB by design), and the
    /// scan budget prevents unbounded work on a multi-GiB journal. Follow the
    /// opaque next_cursor to search older records without an OFFSET scan.
    pub fn query(&self, query: &ApplicationLogQuery) -> Result<ApplicationLogPage, String> {
        query.validate()?;
        let directory = self.directory();
        let mut files = {
            // Snapshot the managed names under the writer lock. The cursor is
            // stable against append/rotation; removal by retention is reported.
            let _guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if !directory.exists() {
                return Ok(ApplicationLogPage {
                    entries: Vec::new(),
                    next_cursor: None,
                    scanned_bytes: 0,
                    malformed_lines: 0,
                    scan_limit_reached: false,
                });
            }
            log_files(&directory)?
        };
        files.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
        let (start_file, start_position) = match query.cursor.as_deref() {
            Some(raw) => {
                let (name, position) = parse_log_cursor(raw)?;
                let index = files
                    .iter()
                    .position(|f| f.path.file_name().and_then(|n| n.to_str()) == Some(name))
                    .ok_or(crate::i18n::text(
                        "Курсор журнала устарел: файл удалён или перемещён. Обнови поиск.",
                        "Log cursor expired: the file was removed or moved. Refresh the search.",
                    ))?;
                (index, Some(position))
            }
            None => (0, None),
        };
        let mut page = ApplicationLogPage {
            entries: Vec::new(),
            next_cursor: None,
            scanned_bytes: 0,
            malformed_lines: 0,
            scan_limit_reached: false,
        };
        for (index, file) in files.iter().enumerate().skip(start_file) {
            if file.size > LOG_QUERY_MAX_FILE_SIZE {
                return Err(crate::i18n::text(
                    "Журнал: размер отдельного JSONL превышает безопасный предел",
                    "Log: an individual JSONL file exceeds the safe size limit",
                )
                .into());
            }
            // O_NOFOLLOW closes the symlink race between listing and opening.
            let input = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&file.path)
                .map_err(|e| format!("Не удалось открыть журнал {}: {e}", file.path.display()))?;
            if !input.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err(crate::i18n::text(
                    "Журнал: файл перестал быть обычным файлом",
                    "Log: the file is no longer a regular file",
                )
                .into());
            }
            let mut data = Vec::new();
            input
                .take(LOG_QUERY_MAX_FILE_SIZE + 1)
                .read_to_end(&mut data)
                .map_err(|e| format!("Не удалось прочитать журнал {}: {e}", file.path.display()))?;
            if data.len() as u64 > LOG_QUERY_MAX_FILE_SIZE {
                return Err(crate::i18n::text(
                    "Журнал: файл изменился и превышает безопасный предел",
                    "Log: the file changed and now exceeds the safe size limit",
                )
                .into());
            }
            let mut position = if index == start_file {
                start_position.unwrap_or(data.len())
            } else {
                data.len()
            };
            if position > data.len() {
                return Err(crate::i18n::text(
                    "Курсор журнала устарел: файл стал короче. Обнови поиск.",
                    "Log cursor expired: the file was truncated. Refresh the search.",
                )
                .into());
            }
            let name = file
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(crate::i18n::text(
                    "Некорректное имя файла журнала",
                    "Invalid log filename",
                ))?;
            while position > 0 {
                let end = if data[position - 1] == b'\n' {
                    position - 1
                } else {
                    position
                };
                let start = data[..end]
                    .iter()
                    .rposition(|b| *b == b'\n')
                    .map_or(0, |i| i + 1);
                let line = &data[start..end];
                position = start;
                page.scanned_bytes = page.scanned_bytes.saturating_add((end - start + 1) as u64);
                if !line.is_empty() {
                    match serde_json::from_slice::<ApplicationLogEntry>(line) {
                        Ok(entry) if query.matches(&entry) => page.entries.push(entry),
                        Err(_) => page.malformed_lines += 1,
                        _ => (),
                    }
                }
                // Peek one matching record to prevent a false "next page" on
                // an exact page boundary; leave that record unread via cursor.
                if page.entries.len() > query.limit as usize {
                    page.entries.pop();
                    page.next_cursor = Some(log_cursor(
                        name,
                        end + usize::from(end < data.len() && data[end] == b'\n'),
                    ));
                    return Ok(page);
                }
                if page.scanned_bytes >= LOG_QUERY_SCAN_BUDGET {
                    page.scan_limit_reached = true;
                    page.next_cursor = Some(log_cursor(name, position));
                    return Ok(page);
                }
            }
        }
        Ok(page)
    }

    pub fn clear(&self) -> Result<ApplicationLogState, String> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active = None;
        let directory = self.directory();
        let clear_error = remove_log_files(&directory).err();
        // Always attempt to reopen, including when an earlier removal failed.
        let reopen_error = match initialize_active(&directory, inner.settings, 0) {
            Ok(active) => {
                inner.active = Some(active);
                inner.retry_after_ms = 0;
                None
            }
            Err(error) => {
                inner.retry_after_ms = 0;
                Some(error)
            }
        };
        let mut error = clear_error;
        if let Some(reopen_error) = reopen_error {
            append_state_error(&mut error, reopen_error);
        }
        inner.last_error = error.clone();
        drop(inner);
        if let Some(error) = error {
            return Err(error);
        }
        Ok(self.state())
    }

    pub fn migrate_directory(&self, destination: &Path) -> Result<ApplicationLogState, String> {
        self.migrate_directory_with_commit(destination, |_| Ok(()))
    }

    /// The callback must persist the selected directory. The source is not
    /// removed until persistence succeeds; on failure, the source stays active.
    pub fn migrate_directory_with_commit<F>(
        &self,
        destination: &Path,
        commit: F,
    ) -> Result<ApplicationLogState, String>
    where
        F: FnOnce(&Path) -> Result<(), String>,
    {
        if !destination.is_absolute() {
            return Err("Application log directory должен быть абсолютным путём".into());
        }

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let source = self.directory();
        let source_canonical = source.canonicalize().unwrap_or_else(|_| source.clone());
        let destination_existed = destination.exists();

        if destination.exists() {
            let candidate = destination.canonicalize().map_err(|error| {
                format!("Не удалось разрешить application log directory: {error}")
            })?;
            if candidate == source_canonical {
                commit(&source_canonical)?;
                drop(inner);
                return Ok(self.state());
            }
            if candidate.starts_with(&source_canonical) {
                return Err(
                    "Application log directory не может находиться внутри текущего каталога журнала"
                        .into(),
                );
            }
        }

        let destination = prepare_empty_log_directory(destination)?;
        if destination.starts_with(&source_canonical) {
            if !destination_existed {
                let _ = fs::remove_dir(&destination);
            }
            return Err(
                "Application log directory не может находиться внутри текущего каталога журнала"
                    .into(),
            );
        }
        // Prevent concurrent record/clear from changing the source during copy.
        inner.active = None;
        let migration = (|| -> Result<ActiveLog, String> {
            if source.exists() {
                for file in log_files(&source)? {
                    let name = file
                        .path
                        .file_name()
                        .ok_or("Некорректное имя application log file")?;
                    let target = destination.join(name);
                    fs::copy(&file.path, &target).map_err(|error| {
                        format!(
                            "Не удалось скопировать application log {} в {}: {error}",
                            file.path.display(),
                            target.display()
                        )
                    })?;
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).map_err(
                        |error| {
                            format!(
                                "Не удалось выставить 0600 для application log {}: {error}",
                                target.display()
                            )
                        },
                    )?;
                }
            }
            let active = initialize_active(&destination, inner.settings, 0)?;
            // No destructive operation on the source before commit.
            commit(&destination)?;
            Ok(active)
        })();

        let active = match migration {
            Ok(active) => active,
            Err(error) => {
                let cleanup_error = remove_log_files(&destination).err();
                if !destination_existed {
                    let _ = fs::remove_dir(&destination);
                }
                match initialize_active(&source, inner.settings, 0) {
                    Ok(active) => inner.active = Some(active),
                    Err(reopen_error) => {
                        inner.last_error = Some(format!(
                            "{error}; дополнительно не удалось снова открыть прежний application log: {reopen_error}"
                        ));
                    }
                }
                inner.retry_after_ms = 0;
                let mut message = Some(error);
                if let Some(cleanup_error) = cleanup_error {
                    append_state_error(&mut message, cleanup_error);
                }
                return Err(message.expect("migration error must exist"));
            }
        };

        {
            let mut directory = self
                .directory
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *directory = destination.clone();
        }
        inner.active = Some(active);
        inner.last_error = None;
        inner.retry_after_ms = 0;
        // Keep the lock until source cleanup is complete so a simultaneous
        // migration cannot begin from an already deleted directory.
        if source != destination && source.exists() {
            if let Err(error) = remove_log_files(&source) {
                inner.last_error = Some(error);
            } else if let Err(error) = fs::remove_dir(&source) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    inner.last_error = Some(format!(
                        "Журнал переключён, но старый каталог {} не удалось удалить: {error}",
                        source.display()
                    ));
                }
            }
        }
        drop(inner);
        Ok(self.state())
    }

    pub fn directory(&self) -> PathBuf {
        self.directory
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn mark_unavailable(&self, error: String) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.last_error = Some(error);
        inner.active = None;
        inner.retry_after_ms = 0;
    }
}

#[derive(Clone)]
struct LogFile {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

fn prepare_empty_log_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Application log directory должен быть абсолютным путём".into());
    }

    let existed = path.exists();
    if existed {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Application log directory недоступен: {error}"))?;

        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(
                "Application log directory должен быть обычным каталогом, а не symlink".into(),
            );
        }

        if fs::read_dir(path)
            .map_err(|error| format!("Не удалось прочитать application log directory: {error}"))?
            .next()
            .is_some()
        {
            return Err("Новый application log directory должен быть пустым каталогом".into());
        }
    } else {
        let parent = path
            .parent()
            .ok_or("Application log directory должен иметь parent directory")?;

        if !parent.exists() || !parent.is_dir() {
            return Err(format!(
                "Parent directory application log должен существовать: {}",
                parent.display(),
            ));
        }

        fs::create_dir(path)
            .map_err(|error| format!("Не удалось создать application log directory: {error}"))?;
    }

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Не удалось выставить 0700 для application log: {error}"))?;

    path.canonicalize()
        .map_err(|error| format!("Не удалось разрешить application log directory: {error}"))
}

fn remove_log_files(directory: &Path) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }

    for file in log_files(directory)? {
        fs::remove_file(&file.path).map_err(|error| {
            format!(
                "Не удалось удалить application log {}: {error}",
                file.path.display(),
            )
        })?;
    }

    Ok(())
}

fn initialize_active(
    directory: &Path,
    settings: ApplicationLogSettings,
    sequence: u32,
) -> Result<ActiveLog, String> {
    if directory.exists() {
        let metadata = fs::symlink_metadata(directory)
            .map_err(|error| format!("Application log directory недоступен: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(
                "Application log directory должен быть обычным каталогом, а не symlink".into(),
            );
        }
    } else {
        fs::create_dir_all(directory)
            .map_err(|error| format!("Не удалось создать каталог application log: {error}"))?;
    }

    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Не удалось выставить 0700 для application log: {error}"))?;

    cleanup_directory(directory, settings, None)?;
    create_active_log(directory, sequence)
}

fn rotate_locked(
    directory: &Path,
    inner: &mut ApplicationLogInner,
    sequence: u32,
) -> Result<(), String> {
    if let Some(active) = inner.active.as_mut() {
        active
            .file
            .flush()
            .map_err(|error| format!("Не удалось flush application log перед rotation: {error}"))?;
    }

    inner.active = Some(create_active_log(directory, sequence)?);
    Ok(())
}

fn create_active_log(directory: &Path, sequence: u32) -> Result<ActiveLog, String> {
    let started_at_ms = unix_time_millis()?;
    let (path, file) = create_log_file(directory, sequence)?;

    Ok(ActiveLog {
        path,
        file,
        bytes: 0,
        started_at_ms,
        sequence,
    })
}

fn create_log_file(directory: &Path, sequence: u32) -> Result<(PathBuf, File), String> {
    let timestamp = unix_time_millis()?;

    for suffix in 0..1000_u32 {
        let path = directory.join(format!(
            "pgxray-{timestamp}-{}-{sequence}-{suffix}.jsonl",
            std::process::id(),
        ));

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|error| {
                    format!("Не удалось выставить 0600 для application log: {error}")
                })?;
                return Ok((path, file));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("Не удалось создать application log file: {error}"));
            }
        }
    }

    Err("Не удалось подобрать имя application log file".into())
}

fn load_settings_best_effort(path: &Path) -> (ApplicationLogSettings, Option<String>) {
    if !path.exists() {
        let settings = ApplicationLogSettings::default();
        let error = save_settings(path, settings).err();
        return (settings, error);
    }

    let result = fs::read_to_string(path)
        .map_err(|error| format!("Не удалось прочитать application log settings: {error}"))
        .and_then(|raw| {
            serde_json::from_str::<ApplicationLogSettings>(&raw)
                .map_err(|error| format!("Некорректный application log settings: {error}"))
        })
        .and_then(ApplicationLogSettings::validate);

    match result {
        Ok(settings) => (settings, None),
        Err(error) => (ApplicationLogSettings::default(), Some(error)),
    }
}

fn save_settings(path: &Path, settings: ApplicationLogSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Не удалось создать каталог log settings: {error}"))?;
    }

    let temporary = path.with_extension("tmp");
    let data = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("Не удалось сериализовать application log settings: {error}"))?;

    fs::write(&temporary, data)
        .map_err(|error| format!("Не удалось сохранить application log settings: {error}"))?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Не удалось выставить 0600 для log settings: {error}"))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("Не удалось заменить application log settings: {error}"))?;

    Ok(())
}

fn cleanup_directory(
    directory: &Path,
    settings: ApplicationLogSettings,
    active_path: Option<&Path>,
) -> Result<(), String> {
    cleanup_directory_at(directory, settings, active_path, SystemTime::now())
}

fn cleanup_directory_at(
    directory: &Path,
    settings: ApplicationLogSettings,
    active_path: Option<&Path>,
    now: SystemTime,
) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }

    let max_age = Duration::from_secs(u64::from(settings.retention_days) * 24 * 60 * 60);
    let mut files = log_files(directory)?;

    for file in &files {
        if active_path.is_some_and(|active| active == file.path.as_path()) {
            continue;
        }

        if now.duration_since(file.modified).unwrap_or_default() > max_age {
            fs::remove_file(&file.path).map_err(|error| {
                format!(
                    "Не удалось удалить устаревший application log {}: {error}",
                    file.path.display(),
                )
            })?;
        }
    }

    files = log_files(directory)?;
    let mut total = files.iter().map(|file| file.size).sum::<u64>();

    if total <= settings.max_total_bytes {
        return Ok(());
    }

    files.sort_by_key(|file| file.modified);

    for file in files {
        if total <= settings.max_total_bytes {
            break;
        }

        if active_path.is_some_and(|active| active == file.path.as_path()) {
            continue;
        }

        fs::remove_file(&file.path).map_err(|error| {
            format!(
                "Не удалось удалить application log {} при size cleanup: {error}",
                file.path.display(),
            )
        })?;
        total = total.saturating_sub(file.size);
    }

    Ok(())
}

fn scan_status(directory: &Path) -> Result<(u64, usize, Option<u64>, Option<u64>), String> {
    if !directory.exists() {
        return Ok((0, 0, None, None));
    }

    let files = log_files(directory)?;
    let total_bytes = files.iter().map(|file| file.size).sum();
    let oldest_at_ms = files
        .iter()
        .map(|file| system_time_millis(file.modified))
        .min()
        .transpose()?;
    let newest_at_ms = files
        .iter()
        .map(|file| system_time_millis(file.modified))
        .max()
        .transpose()?;

    Ok((total_bytes, files.len(), oldest_at_ms, newest_at_ms))
}

fn is_application_log_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(stem) = name
        .strip_prefix("pgxray-")
        .and_then(|name| name.strip_suffix(".jsonl"))
    else {
        return false;
    };
    let mut parts = stem.split('-');
    // Only files produced by create_log_file are managed by this subsystem.
    (0..4).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    }) && parts.next().is_none()
}

fn log_files(directory: &Path) -> Result<Vec<LogFile>, String> {
    let mut files = Vec::new();

    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Не удалось прочитать каталог application log: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Не удалось прочитать log entry: {error}"))?;
        let path = entry.path();

        if !is_application_log_file(&path) {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|error| format!("Не удалось прочитать тип application log: {error}"))?
            .is_file()
        {
            continue;
        }

        let metadata = entry
            .metadata()
            .map_err(|error| format!("Не удалось прочитать metadata application log: {error}"))?;

        if !metadata.is_file() {
            continue;
        }

        files.push(LogFile {
            path,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(UNIX_EPOCH),
        });
    }

    Ok(files)
}

/// Keep only UI context fields that are identifiers or boolean flags.
/// `application_log_event` is callable independently of the Vue invoke wrapper,
/// so the Rust sink must not trust its free-form context argument.
fn safe_ui_context(value: &str) -> Option<String> {
    let mut fields = Vec::new();
    for token in value.split_whitespace().take(64) {
        let Some((key, raw)) = token.split_once('=') else {
            continue;
        };
        if !matches!(
            key,
            "id" | "connectionId"
                | "projectId"
                | "slot"
                | "caseId"
                | "artifactId"
                | "baselineArtifactId"
                | "candidateArtifactId"
                | "comparisonId"
                | "interventionId"
                | "impactId"
                | "scopeId"
                | "sessionId"
                | "relationOid"
                | "snapshotId"
                | "userid"
                | "dbid"
                | "pid"
                | "backendStartMs"
                | "firstId"
                | "secondId"
                | "analyzeQueries"
                | "timeoutMs"
                | "scanInbox"
                | "deleteComparisons"
                | "forceNew"
                | "force"
                | "useSavedPassword"
                | "keepExistingPassword"
                | "toplevel"
                | "retentionDays"
                | "queryid"
        ) {
            continue;
        }
        // No SQL fragments, paths, arbitrary strings, or huge numeric tokens.
        let numeric = !raw.is_empty()
            && raw.len() <= 21
            && raw
                .strip_prefix('-')
                .unwrap_or(raw)
                .bytes()
                .all(|b| b.is_ascii_digit())
            && raw != "-";
        if numeric || matches!(raw, "true" | "false" | "null") {
            fields.push(format!("{key}={raw}"));
        }
    }
    (!fields.is_empty()).then(|| fields.join(" "))
}

/// Never persist raw error messages: PostgreSQL detail/hint and UI exceptions
/// can contain values taken directly from SQL or connection settings.
fn safe_log_error(value: &str) -> String {
    let sqlstate = value
        .split("SQLSTATE ")
        .nth(1)
        .and_then(|suffix| suffix.get(..5))
        .filter(|code| {
            code.bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        });
    match sqlstate {
        Some(code) => format!("Error details omitted (SQLSTATE {code})"),
        None => "Error details omitted (may contain SQL or credentials)".to_owned(),
    }
}

fn sanitize_field(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .take(max_chars)
        .collect()
}

fn append_state_error(target: &mut Option<String>, error: String) {
    match target {
        Some(existing) if existing != &error => {
            existing.push_str("; ");
            existing.push_str(&error);
        }
        Some(_) => {}
        None => *target = Some(error),
    }
}

fn unix_time_millis() -> Result<u64, String> {
    system_time_millis(SystemTime::now())
}

fn system_time_millis(time: SystemTime) -> Result<u64, String> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Системное время находится до Unix epoch")?;
    u64::try_from(duration.as_millis()).map_err(|_| "Unix timestamp не помещается в u64".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn event(operation: &'static str, phase: &'static str) -> ApplicationLogEvent<'static> {
        ApplicationLogEvent {
            level: "info",
            component: "test",
            operation,
            phase,
            actor: Some("owner"),
            request_id: Some("test-request"),
            duration_ms: None,
            context: Some("connection_id=7"),
            error: None,
        }
    }

    fn journal_query(limit: u32) -> ApplicationLogQuery {
        ApplicationLogQuery {
            limit,
            ..Default::default()
        }
    }

    #[test]
    fn redacts_postgres_and_ui_errors_without_losing_safe_context() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let log = ApplicationLog::open_best_effort(&logs, &directory.path().join("settings"));
        let mut ui_event = event("collect_pgss_snapshot", "failed");
        ui_event.component = "ui_reported";
        ui_event.context = Some(
            "connectionId=12 force=true password=SECRET SELECT * FROM private_table; queryid=-42",
        );
        ui_event.error = Some("password=SECRET; SELECT * FROM private_table");
        log.record(ui_event);

        let mut postgres_event = event("internal_postgres", "query_failed");
        postgres_event.component = "postgres";
        postgres_event.context = Some("connection_id=12 sql_fingerprint=0123456789abcdef");
        postgres_event.error = Some(
            "duplicate key value (SECRET) (SQLSTATE 23505); detail: SELECT * FROM private_table",
        );
        log.record(postgres_event);

        let entries = log.recent(10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].error.as_deref(),
            Some("Error details omitted (SQLSTATE 23505)")
        );
        assert_eq!(
            entries[1].context.as_deref(),
            Some("connectionId=12 force=true queryid=-42")
        );
        assert!(entries.iter().all(|entry| {
            !serde_json::to_string(entry).unwrap().contains("SECRET")
                && !serde_json::to_string(entry)
                    .unwrap()
                    .contains("private_table")
        }));
        let written = fs::read_dir(&logs)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!written.contains("SECRET"));
        assert!(!written.contains("private_table"));
    }

    #[test]
    fn ui_context_rejects_free_text_and_does_not_leak_secrets() {
        assert_eq!(
            safe_ui_context("sql=SELECT password=12345 context=private"),
            None
        );
        assert_eq!(
            safe_ui_context("connectionId=7 pid=null force=false"),
            Some("connectionId=7 pid=null force=false".into())
        );
        assert_eq!(
            safe_log_error("arbitrary error containing token SECRET"),
            "Error details omitted (may contain SQL or credentials)"
        );
    }

    #[test]
    fn diagnostic_operation_links_children_and_keeps_old_jsonl_compatible() {
        let directory = tempdir().unwrap();
        let log = ApplicationLog::open_best_effort(
            &directory.path().join("logs"),
            &directory.path().join("settings"),
        );
        let info = DiagnosticLogDetails {
            connection_id: 7,
            kind: "pgss".into(),
            snapshot_id: None,
            unavailable_sections: None,
            total_sections: None,
            client_request_id: None,
        };
        log.record_enriched(
            event("collect_pgss_snapshot", "started"),
            Some("diag-42"),
            Some(info.clone()),
        );
        let mut child = event("internal_postgres", "query_finished");
        child.request_id = Some("pg-unique-query");
        log.record_enriched(child, Some("diag-42"), Some(info.clone()));
        log.record_enriched(
            event("collect_pgss_snapshot", "succeeded"),
            Some("diag-42"),
            Some(DiagnosticLogDetails {
                snapshot_id: Some(18),
                unavailable_sections: Some(1),
                total_sections: Some(6),
                ..info
            }),
        );
        let page = log
            .query(&ApplicationLogQuery {
                operation_id: Some("diag-42".into()),
                ..journal_query(20)
            })
            .unwrap();
        assert_eq!(page.entries.len(), 3);
        assert_eq!(
            page.entries[0].diagnostic.as_ref().unwrap().snapshot_id,
            Some(18)
        );
        let pgss = log
            .query(&ApplicationLogQuery {
                section: Some("pgss".into()),
                operation_id: Some("diag-42".into()),
                ..journal_query(20)
            })
            .unwrap();
        assert_eq!(pgss.entries.len(), 3); // child SQL is classified by verified parent
        assert!(log
            .query(&ApplicationLogQuery {
                operation_id: Some("wrong".into()),
                ..journal_query(20)
            })
            .unwrap()
            .entries
            .is_empty());
        let old = r#"{"timestamp_ms":1,"level":"info","component":"test","operation":"old","phase":"succeeded","actor":null,"request_id":null,"duration_ms":null,"context":null,"error":null}"#;
        let parsed: ApplicationLogEntry = serde_json::from_str(old).unwrap();
        assert!(parsed.operation_id.is_none());
        assert!(parsed.diagnostic.is_none());
    }

    #[test]
    fn searches_beyond_1000_entries_with_stable_cursor_and_filters() {
        let directory = tempdir().unwrap();
        let log_dir = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&log_dir, &settings);
        for index in 0..1_230 {
            log.record(ApplicationLogEvent {
                level: if index == 111 { "error" } else { "info" },
                component: "test",
                operation: if index == 111 {
                    "target_problem"
                } else {
                    "some_operation"
                },
                phase: if index == 111 { "failed" } else { "succeeded" },
                actor: Some("owner"),
                request_id: None,
                duration_ms: Some(index),
                context: Some("connection_id=7"),
                error: if index == 111 {
                    Some("target-marker")
                } else {
                    None
                },
            });
        }
        let mut query = journal_query(75);
        let mut indices = Vec::new();
        loop {
            let page = log.query(&query).unwrap();
            indices.extend(page.entries.iter().map(|entry| entry.duration_ms.unwrap()));
            if let Some(cursor) = page.next_cursor {
                query.cursor = Some(cursor);
            } else {
                break;
            }
        }
        assert_eq!(indices.len(), 1_230);
        assert_eq!(indices[0], 1_229);
        assert_eq!(*indices.last().unwrap(), 0);
        // The raw error text is intentionally not persisted after PGX-008.
        // Search remains available through the safe, recorded operation name.
        let matches = log
            .query(&ApplicationLogQuery {
                search: Some("target_problem".to_owned()),
                problems_only: true,
                ..journal_query(10)
            })
            .unwrap();
        assert_eq!(matches.entries.len(), 1);
        assert_eq!(matches.entries[0].operation, "target_problem");
        assert_eq!(
            matches.entries[0].error.as_deref(),
            Some("Error details omitted (may contain SQL or credentials)")
        );
        assert!(log
            .query(&ApplicationLogQuery {
                search: Some("target-marker".to_owned()),
                problems_only: true,
                ..journal_query(10)
            })
            .unwrap()
            .entries
            .is_empty());
        let long = log
            .query(&ApplicationLogQuery {
                min_duration_ms: Some(1_200),
                ..journal_query(50)
            })
            .unwrap();
        assert_eq!(long.entries.len(), 30);
    }

    #[test]
    fn filters_application_sections_before_pagination_and_distinguishes_origin() {
        let directory = tempdir().unwrap();
        let log = ApplicationLog::open_best_effort(
            &directory.path().join("logs"),
            &directory.path().join("settings"),
        );
        for (component, operation) in [
            ("ui_reported", "save_connection"),
            ("ui_reported", "collect_pgss_snapshot"),
            ("postgres", "internal_postgres"),
            ("postgres", "xid_policy_apply"),
            ("app_log", "application_log_settings"),
        ] {
            log.record(ApplicationLogEvent {
                level: "info",
                component,
                operation,
                phase: "succeeded",
                actor: None,
                request_id: None,
                duration_ms: None,
                context: None,
                error: None,
            });
        }
        let section = |id: &str| {
            log.query(&ApplicationLogQuery {
                section: Some(id.to_owned()),
                ..journal_query(100)
            })
            .unwrap()
            .entries
        };
        assert_eq!(section("connections").len(), 1);
        assert_eq!(section("pgss").len(), 1);
        assert_eq!(section("xid").len(), 1);
        assert_eq!(section("postgres").len(), 1);
        assert_eq!(section("diagnostics").len(), 3);
        assert_eq!(section("application").len(), 1);
        let ui_pgss = log
            .query(&ApplicationLogQuery {
                section: Some("pgss".into()),
                component: Some("ui_reported".into()),
                ..journal_query(100)
            })
            .unwrap();
        assert_eq!(ui_pgss.entries.len(), 1);
        assert!(log
            .query(&ApplicationLogQuery {
                section: Some("../invalid".into()),
                ..journal_query(100)
            })
            .is_err());
    }

    #[test]
    fn query_filters_by_time_and_rejects_bad_cursor() {
        let directory = tempdir().unwrap();
        let log = ApplicationLog::open_best_effort(
            &directory.path().join("logs"),
            &directory.path().join("config"),
        );
        log.record(event("first", "started"));
        let now = unix_time_millis().unwrap();
        assert_eq!(
            log.query(&ApplicationLogQuery {
                from_ms: Some(now + 60_000),
                ..journal_query(20)
            })
            .unwrap()
            .entries
            .len(),
            0
        );
        assert!(log
            .query(&ApplicationLogQuery {
                from_ms: Some(100),
                to_ms: Some(10),
                ..journal_query(20)
            })
            .is_err());
        assert!(log
            .query(&ApplicationLogQuery {
                cursor: Some("../secrets|1".to_owned()),
                ..journal_query(20)
            })
            .is_err());
        assert!(log
            .query(&ApplicationLogQuery {
                cursor: Some("pgxray-1-1-0-0.jsonl|123".to_owned()),
                ..journal_query(20)
            })
            .is_err());
        assert!(log.query(&journal_query(201)).is_err());
    }

    #[test]
    fn client_correlation_links_ui_rust_and_sql_without_claiming_ui_is_authoritative() {
        let directory = tempdir().unwrap();
        let log = ApplicationLog::open_best_effort(
            &directory.path().join("logs"),
            &directory.path().join("settings"),
        );
        let mut ui_event = event("collect_pgss_snapshot", "succeeded");
        ui_event.component = "ui_reported";
        ui_event.request_id = Some("mwjabcdef:1");
        log.record(ui_event);
        let details = DiagnosticLogDetails {
            connection_id: 7,
            kind: "pgss".into(),
            snapshot_id: Some(22),
            unavailable_sections: Some(0),
            total_sections: Some(6),
            client_request_id: Some("mwjabcdef:1".into()),
        };
        let mut confirmed = event("collect_pgss_snapshot", "succeeded");
        confirmed.component = "diagnostics";
        confirmed.request_id = Some("diag-confirmed");
        log.record_enriched(confirmed, Some("diag-confirmed"), Some(details.clone()));
        let mut sql = event("internal_postgres", "query_finished");
        sql.component = "postgres";
        sql.request_id = Some("pg-query-1");
        log.record_enriched(sql, Some("diag-confirmed"), Some(details));
        log.record(event("other_operation", "succeeded"));
        let page = log
            .query(&ApplicationLogQuery {
                client_request_id: Some("mwjabcdef:1".into()),
                ..journal_query(10)
            })
            .unwrap();
        assert_eq!(page.entries.len(), 3);
        assert!(page.entries.iter().any(|e| e.component == "ui_reported"));
        assert!(page.entries.iter().any(|e| e.component == "diagnostics"
            && e.diagnostic.as_ref().unwrap().snapshot_id == Some(22)));
        assert!(page.entries.iter().any(|e| e.component == "postgres"));
        assert!(log
            .query(&ApplicationLogQuery {
                client_request_id: Some("mwjabcdef:other".into()),
                ..journal_query(10)
            })
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn typed_rust_outcomes_are_explicit_and_historic_jsonl_remains_readable() {
        let directory = tempdir().unwrap();
        let log = ApplicationLog::open_best_effort(
            &directory.path().join("logs"),
            &directory.path().join("settings"),
        );
        log.record_outcome(
            event("login", "succeeded"),
            OperationLogResult::Authentication {
                authenticated: true,
            },
        );
        log.record_outcome(
            event("list_connection_projects", "succeeded"),
            OperationLogResult::ConnectionProjects {
                count: Some(7),
                succeeded: true,
            },
        );
        let page = log.query(&journal_query(10)).unwrap();
        assert!(page.entries.iter().any(|e| matches!(
            e.operation_result.as_ref(),
            Some(OperationLogResult::Authentication {
                authenticated: true
            })
        )));
        assert!(page.entries.iter().any(|e| matches!(
            e.operation_result.as_ref(),
            Some(OperationLogResult::ConnectionProjects {
                count: Some(7),
                succeeded: true
            })
        )));
        let old = r#"{"timestamp_ms":1,"level":"info","component":"ui_reported","operation":"login","phase":"succeeded","actor":null,"request_id":"old","duration_ms":1,"context":null,"error":null}"#;
        let entry: ApplicationLogEntry = serde_json::from_str(old).unwrap();
        assert!(entry.operation_result.is_none());
    }

    #[test]
    fn cursor_moves_across_rotated_files_and_skips_malformed_lines() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let log = ApplicationLog::open_best_effort(&logs, &directory.path().join("config"));
        let older = logs.join("pgxray-1-1-0-0.jsonl");
        let sample = ApplicationLogEntry {
            timestamp_ms: 1,
            level: "info".to_owned(),
            component: "test".to_owned(),
            operation: "previous_week".to_owned(),
            phase: "succeeded".to_owned(),
            actor: None,
            request_id: None,
            duration_ms: None,
            context: None,
            error: None,
            operation_id: None,
            diagnostic: None,
            operation_result: None,
        };
        fs::write(
            &older,
            format!("not-json\n{}\n", serde_json::to_string(&sample).unwrap()),
        )
        .unwrap();
        log.record(event("newest", "succeeded"));
        let first = log.query(&journal_query(1)).unwrap();
        assert_eq!(first.entries[0].operation, "newest");
        let second = log
            .query(&ApplicationLogQuery {
                cursor: first.next_cursor,
                ..journal_query(1)
            })
            .unwrap();
        assert_eq!(second.entries[0].operation, "previous_week");
        assert_eq!(second.malformed_lines, 1);
        assert_eq!(second.next_cursor, None);
    }

    #[test]
    fn query_does_not_read_foreign_files_or_follow_symlinks() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let log = ApplicationLog::open_best_effort(&logs, &directory.path().join("config"));
        log.record(event("safe", "succeeded"));
        fs::write(
            logs.join("private.txt"),
            serde_json::to_string(&ApplicationLogEntry {
                timestamp_ms: 1,
                level: "error".into(),
                component: "private".into(),
                operation: "secret".into(),
                phase: "failed".into(),
                actor: None,
                request_id: None,
                duration_ms: None,
                context: None,
                error: None,
                operation_id: None,
                diagnostic: None,
                operation_result: None,
            })
            .unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            logs.join("private.txt"),
            logs.join("pgxray-9999999-9-0-0.jsonl"),
        )
        .unwrap();
        let page = log.query(&journal_query(20)).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].operation, "safe");
    }

    #[test]
    fn records_and_reads_recent_entries() {
        let directory = tempdir().unwrap();
        let log_dir = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&log_dir, &settings);

        log.record(event("collect_xid_snapshot", "started"));
        log.record(ApplicationLogEvent {
            duration_ms: Some(42),
            phase: "succeeded",
            ..event("collect_xid_snapshot", "succeeded")
        });

        let entries = log.recent(10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].phase, "succeeded");
        assert_eq!(entries[0].duration_ms, Some(42));
        assert_eq!(entries[0].actor.as_deref(), Some("owner"));
        assert_eq!(entries[1].phase, "started");
    }

    #[test]
    fn clear_removes_previous_entries() {
        let directory = tempdir().unwrap();
        let log_dir = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&log_dir, &settings);

        log.record(event("before", "succeeded"));
        log.clear().unwrap();

        let entries = log.recent(10).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn accepts_only_supported_storage_settings() {
        for max_total_bytes in [
            APPLICATION_LOG_500_MIB,
            APPLICATION_LOG_1_GIB,
            APPLICATION_LOG_3_GIB,
        ] {
            for retention_days in [7, 30, 90] {
                assert!(ApplicationLogSettings {
                    max_total_bytes,
                    retention_days,
                }
                .validate()
                .is_ok());
            }
        }

        assert!(ApplicationLogSettings {
            max_total_bytes: 1024,
            retention_days: 7,
        }
        .validate()
        .is_err());
        assert!(ApplicationLogSettings {
            max_total_bytes: APPLICATION_LOG_500_MIB,
            retention_days: 1,
        }
        .validate()
        .is_err());
    }

    #[test]
    fn migrates_directory_and_keeps_previous_entries() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("logs");
        let destination = directory.path().join("moved-logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&source, &settings);

        log.record(event("before_move", "succeeded"));
        let state = log.migrate_directory(&destination).unwrap();
        log.record(event("after_move", "succeeded"));

        assert_eq!(
            PathBuf::from(state.status.directory),
            destination.canonicalize().unwrap(),
        );
        assert!(!source.exists());
        let entries = log.recent(10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].operation, "after_move");
        assert_eq!(entries[1].operation, "before_move");
    }

    #[test]
    fn rejects_nested_log_directory_without_leaving_target() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("logs");
        let destination = source.join("nested");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&source, &settings);

        assert!(log.migrate_directory(&destination).is_err());
        assert!(!destination.exists());

        log.record(event("still_on_source", "succeeded"));
        assert_eq!(log.recent(10).unwrap()[0].operation, "still_on_source");
    }

    #[test]
    fn corrupt_settings_degrade_to_defaults_without_disabling_logging() {
        let directory = tempdir().unwrap();
        let log_dir = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        fs::write(&settings, "not-json").unwrap();

        let log = ApplicationLog::open_best_effort(&log_dir, &settings);
        let state = log.state();

        assert_eq!(state.settings, ApplicationLogSettings::default());
        assert!(state.status.available);
        assert!(state.status.last_error.is_some());

        log.record(event("still_works", "succeeded"));
        assert_eq!(log.recent(10).unwrap().len(), 1);
    }

    #[test]
    fn retention_cleanup_removes_old_files_but_preserves_active_file() {
        let directory = tempdir().unwrap();
        let log_dir = directory.path().join("logs");
        fs::create_dir(&log_dir).unwrap();

        let old = log_dir.join("pgxray-100-1-0-0.jsonl");
        let active = log_dir.join("pgxray-200-1-0-0.jsonl");
        fs::write(&old, "old").unwrap();
        fs::write(&active, "active").unwrap();

        let future = SystemTime::now() + Duration::from_secs(8 * 24 * 60 * 60);
        cleanup_directory_at(
            &log_dir,
            ApplicationLogSettings::default(),
            Some(&active),
            future,
        )
        .unwrap();

        assert!(!old.exists());
        assert!(active.exists());
    }

    #[test]
    fn size_cleanup_preserves_active_file() {
        let directory = tempdir().unwrap();
        let log_dir = directory.path().join("logs");
        fs::create_dir(&log_dir).unwrap();

        let old = log_dir.join("pgxray-100-1-0-0.jsonl");
        let active = log_dir.join("pgxray-200-1-0-0.jsonl");
        fs::write(&old, vec![b'x'; 16]).unwrap();
        fs::write(&active, vec![b'y'; 16]).unwrap();

        cleanup_directory_at(
            &log_dir,
            ApplicationLogSettings {
                max_total_bytes: 16,
                retention_days: 90,
            },
            Some(&active),
            SystemTime::now(),
        )
        .unwrap();

        assert!(!old.exists());
        assert!(active.exists());
    }
    #[test]
    fn clear_and_cleanup_do_not_delete_unrelated_jsonl_files() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&logs, &settings);
        let foreign = logs.join("unrelated.jsonl");
        fs::write(&foreign, "keep this data").unwrap();
        log.record(event("before_clear", "succeeded"));
        log.clear().unwrap();
        assert_eq!(fs::read_to_string(&foreign).unwrap(), "keep this data");
        assert!(log.recent(10).unwrap().is_empty());

        cleanup_directory_at(
            &logs,
            ApplicationLogSettings {
                max_total_bytes: 0,
                retention_days: 7,
            },
            None,
            SystemTime::now() + Duration::from_secs(8 * 24 * 60 * 60),
        )
        .unwrap();
        assert!(foreign.exists());
    }

    #[test]
    fn only_own_regular_files_are_managed() {
        let directory = tempdir().unwrap();
        let logs = directory.path();
        let valid = logs.join("pgxray-123-456-0-1.jsonl");
        let unrelated = logs.join("pgxray-other.jsonl");
        fs::write(&valid, "ok").unwrap();
        fs::write(&unrelated, "untouched").unwrap();
        let symlink = logs.join("pgxray-789-456-0-1.jsonl");
        std::os::unix::fs::symlink(&valid, &symlink).unwrap();

        let files = log_files(logs).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, valid);
        remove_log_files(logs).unwrap();
        assert!(unrelated.exists());
        assert!(fs::symlink_metadata(&symlink)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn failed_persistence_keeps_source_and_entries() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("logs");
        let destination = directory.path().join("new-logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&source, &settings);
        log.record(event("before_failed_migration", "succeeded"));

        let result = log.migrate_directory_with_commit(&destination, |_| {
            Err("simulated settings failure".to_owned())
        });
        assert!(result.unwrap_err().contains("simulated settings failure"));
        assert!(source.exists());
        assert!(!destination.exists());
        assert_eq!(log.directory(), source);
        log.record(event("after_failed_migration", "succeeded"));
        let entries = log.recent(10).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn commit_happens_before_old_files_are_removed() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("logs");
        let destination = directory.path().join("new-logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&source, &settings);
        log.record(event("old_entry", "succeeded"));
        let mut committed = false;
        log.migrate_directory_with_commit(&destination, |new_path| {
            assert!(source.exists());
            assert!(!log_files(&source).unwrap().is_empty());
            assert_eq!(new_path, destination.canonicalize().unwrap());
            committed = true;
            Ok(())
        })
        .unwrap();
        assert!(committed);
        assert!(!source.exists());
        assert_eq!(log.recent(10).unwrap()[0].operation, "old_entry");
    }

    #[test]
    fn logger_recovers_after_failed_write() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&logs, &settings);
        {
            let mut inner = log.inner.lock().unwrap();
            let active = inner.active.as_mut().unwrap();
            // A read-only descriptor reliably causes write_all to fail.
            active.file = File::open(&active.path).unwrap();
        }
        log.record(event("write_fails", "failed"));
        assert!(!log.state().status.available);
        {
            let mut inner = log.inner.lock().unwrap();
            // Simulate expiration of the retry backoff without sleeping.
            inner.retry_after_ms = 0;
        }
        log.record(event("write_recovers", "succeeded"));
        assert!(log.state().status.available);
        assert_eq!(log.recent(10).unwrap()[0].operation, "write_recovers");
    }

    #[test]
    fn concurrent_read_record_and_clear_does_not_fail() {
        use std::sync::Arc;
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        let log = Arc::new(ApplicationLog::open_best_effort(&logs, &settings));
        let reader_log = Arc::clone(&log);
        let reader = std::thread::spawn(move || {
            for _ in 0..100 {
                reader_log.recent(50).unwrap();
            }
        });
        for _ in 0..100 {
            log.record(event("concurrent", "succeeded"));
            log.clear().unwrap();
        }
        reader.join().unwrap();
        assert!(log.state().status.available);
    }

    #[test]
    fn unsuccessful_clear_reopens_active_file() {
        let directory = tempdir().unwrap();
        let logs = directory.path().join("logs");
        let settings = directory.path().join("settings.json");
        let log = ApplicationLog::open_best_effort(&logs, &settings);
        log.record(event("before_clear", "succeeded"));
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o500)).unwrap();
        let result = log.clear();
        // On privileged CI the removal may succeed despite permissions.
        if result.is_err() {
            assert!(log.state().status.available);
            log.record(event("after_failed_clear", "succeeded"));
            assert!(log
                .recent(10)
                .unwrap()
                .iter()
                .any(|entry| entry.operation == "after_failed_clear"));
        }
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    }
}
