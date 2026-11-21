use crate::{
    app_log::{ApplicationLog, ApplicationLogEvent},
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use futures_util::TryStreamExt;
use openssl::{
    ssl::{SslConnector, SslFiletype, SslMethod, SslVerifyMode},
    x509::{
        store::{X509Lookup, X509StoreBuilder},
        verify::X509VerifyFlags,
    },
};
use postgres_openssl::MakeTlsConnector;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    future::Future,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::Mutex as AsyncMutex,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_postgres::{
    config::SslMode, CancelToken, Client, Config, NoTls, SimpleQueryMessage, SimpleQueryRow,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_OVERALL_TIMEOUT: Duration = Duration::from_secs(12);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECTION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const REGISTRY_SLOT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const INTERNAL_CONNECTION_IDLE_TTL: Duration = Duration::from_secs(2);
const INTERNAL_APPLICATION_NAME_PREFIX: &str = "pgxray-internal";
const PROBE_APPLICATION_NAME: &str = "pgxray-probe";

static NEXT_QUERY_LOG_ID: AtomicU64 = AtomicU64::new(1);
static QUERY_LOG_SESSION: OnceLock<u128> = OnceLock::new();

tokio::task_local! {
    static DIAGNOSTIC_OPERATION_ID: String;
    static DIAGNOSTIC_KIND: String;
    static DIAGNOSTIC_CLIENT_REQUEST_ID: Option<String>;
}

/// Correlates all nested read-only SQL calls with a single authenticated Tauri
/// diagnostic invocation. Task-local scope is dropped on cancellation; no
/// global mutable state can leak between simultaneous connections.
pub async fn scope_diagnostic_operation<F: Future>(
    operation_id: String,
    kind: &'static str,
    client_request_id: Option<String>,
    future: F,
) -> F::Output {
    DIAGNOSTIC_OPERATION_ID
        .scope(operation_id, async move {
            DIAGNOSTIC_KIND
                .scope(kind.to_owned(), async move {
                    DIAGNOSTIC_CLIENT_REQUEST_ID
                        .scope(client_request_id, future)
                        .await
                })
                .await
        })
        .await
}

#[derive(Clone)]
enum InternalTls {
    Disabled,
    OpenSsl(MakeTlsConnector),
}

impl InternalTls {
    async fn cancel(&self, token: &CancelToken) -> Result<(), tokio_postgres::Error> {
        match self {
            Self::Disabled => token.cancel_query(NoTls).await,
            Self::OpenSsl(connector) => token.cancel_query(connector.clone()).await,
        }
    }
}

pub struct InternalPostgresClient {
    client: Client,
    tls: InternalTls,
    connection_task: JoinHandle<()>,
    healthy: bool,
    last_used_at: Instant,
}

type RegistrySlot = Arc<AsyncMutex<InternalPostgresSlot>>;

#[derive(Default)]
struct InternalPostgresSlot {
    signature: Option<[u8; 32]>,
    client: Option<InternalPostgresClient>,
    client_generation: u64,
    idle_epoch: u64,
}

// A generation distinguishes requests issued before and after logout/shutdown.
#[derive(Default)]
struct RegistryState {
    slots: HashMap<i64, RegistrySlot>,
    retiring: HashMap<i64, RegistrySlot>,
    generation: u64,
    shutting_down: bool,
}

#[derive(Default)]
pub struct InternalPostgresRegistry {
    slots: Arc<StdMutex<RegistryState>>,
    application_log: Option<Arc<ApplicationLog>>,
}

impl InternalPostgresRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_application_log(application_log: Arc<ApplicationLog>) -> Self {
        Self {
            slots: Arc::new(StdMutex::new(RegistryState::default())),
            application_log: Some(application_log),
        }
    }

    fn log_event(
        &self,
        connection_id: i64,
        phase: &str,
        duration_ms: Option<u64>,
        context: Option<&str>,
    ) {
        record_internal_postgres_event(
            self.application_log.as_deref(),
            connection_id,
            phase,
            duration_ms,
            context,
        );
    }

    fn log_query_event(
        &self,
        connection_id: i64,
        phase: &str,
        duration_ms: Option<u64>,
        context: &str,
        request_id: &str,
        error: Option<&str>,
    ) {
        record_internal_postgres_event_with_id(
            self.application_log.as_deref(),
            connection_id,
            phase,
            duration_ms,
            Some(context),
            Some(request_id),
            error,
        );
    }

    fn schedule_idle_shutdown(&self, connection_id: i64, slot: RegistrySlot, idle_epoch: u64) {
        let application_log = self.application_log.clone();

        tokio::spawn(async move {
            sleep(INTERNAL_CONNECTION_IDLE_TTL).await;

            let Ok(mut slot) = slot.try_lock() else {
                // An active query owns the slot. It will schedule a fresh idle
                // shutdown when it finishes, so this timer can simply expire.
                return;
            };

            if slot.idle_epoch != idle_epoch {
                return;
            }

            let should_close = slot
                .client
                .as_ref()
                .is_some_and(|client| client.idle_for() >= INTERNAL_CONNECTION_IDLE_TTL);

            if !should_close {
                return;
            }

            slot.signature = None;
            slot.idle_epoch = next_counter(slot.idle_epoch);

            let Some(client) = slot.client.take() else {
                return;
            };

            let context = format!("idle_ttl_ms={}", INTERNAL_CONNECTION_IDLE_TTL.as_millis(),);
            record_internal_postgres_event(
                application_log.as_deref(),
                connection_id,
                "idle_shutdown_started",
                None,
                Some(&context),
            );

            // Keep the per-connection slot locked until the old driver task has
            // finished or has been aborted. A new diagnostics connection must not
            // be created while the previous pgXray backend is still being closed.
            let started = Instant::now();
            let graceful = client.shutdown().await;
            drop(slot);

            record_internal_postgres_event(
                application_log.as_deref(),
                connection_id,
                if graceful {
                    "idle_shutdown_finished"
                } else {
                    "idle_shutdown_forced"
                },
                Some(duration_millis(started)),
                Some(&context),
            );
        });
    }

    pub async fn invalidate(&self, connection_id: i64) {
        let slot = {
            let slots = self
                .slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            slots.slots.get(&connection_id).cloned()
        };

        if let Some(slot) = slot {
            self.log_event(connection_id, "invalidate_started", None, None);

            // Keep the slot identity stable even when an active operation prevents
            // immediate shutdown. Removing it first would let a concurrent caller
            // create a second reusable client for the same saved connection.
            let started = Instant::now();
            let shutdown = shutdown_registry_slot(slot).await;
            self.log_event(
                connection_id,
                if shutdown {
                    "invalidate_finished"
                } else {
                    "invalidate_deferred"
                },
                Some(duration_millis(started)),
                None,
            );
        }
    }

    pub async fn shutdown_all(&self) {
        let slots = {
            let mut state = self
                .slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            if state.shutting_down {
                return;
            }
            state.shutting_down = true;
            state.generation = next_counter(state.generation);
            state.slots.drain().collect::<Vec<_>>()
        };

        for (connection_id, slot) in slots {
            let started = Instant::now();
            let shutdown = shutdown_registry_slot(Arc::clone(&slot)).await;
            if !shutdown {
                // A busy slot must not be replaced with a second connection
                // until its in-flight query has finished and the driver has exited.
                {
                    let mut state = self
                        .slots
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.retiring.insert(connection_id, Arc::clone(&slot));
                }
                let registry_state = Arc::clone(&self.slots);
                tokio::spawn(async move {
                    let mut locked = slot.lock().await;
                    let _ = shutdown_slot_client(&mut locked).await;
                    drop(locked);
                    let mut state = registry_state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.retiring.remove(&connection_id);
                });
            }
            self.log_event(
                connection_id,
                if shutdown {
                    "client_shutdown"
                } else {
                    "client_shutdown_deferred"
                },
                Some(duration_millis(started)),
                Some("reason=application_shutdown"),
            );
        }

        // Logout is not process exit: the registry must accept a later login.
        // Requests from an earlier generation are rejected after taking the slot.
        let mut state = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.shutting_down = false;
    }

    pub(crate) async fn run_readonly_query(
        &self,
        connection_id: i64,
        input: &ConnectionInput,
        sql: &str,
        policy: QueryExecutionPolicy,
    ) -> Result<String, String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection_id для internal PostgreSQL client",
                "Invalid connection_id for internal PostgreSQL client",
            )
            .into());
        }

        input.validate()?;

        let signature = connection_signature(input);
        let (slot_handle, generation) = self.slot_for_query(connection_id)?;
        let slot_wait_started = Instant::now();
        let mut slot = match timeout(REGISTRY_SLOT_ACQUIRE_TIMEOUT, slot_handle.lock()).await {
            Ok(slot) => slot,
            Err(_) => {
                self.log_event(
                    connection_id,
                    "slot_acquire_timeout",
                    Some(duration_millis(slot_wait_started)),
                    None,
                );
                return Err(format!(
                    "Internal PostgreSQL connection #{connection_id} остаётся занятым дольше 5 секунд. Новая операция остановлена вместо бесконечного ожидания; проверьте предыдущую pgXray diagnostics operation."
                ));
            }
        };
        // A queued request must not resurrect a session retired during logout.
        if !self.is_generation_active(generation) {
            return Err(crate::i18n::text(
                "Internal PostgreSQL operation отменена: registry shutdown",
                "Internal PostgreSQL operation cancelled: registry shutdown",
            )
            .into());
        }
        self.log_event(
            connection_id,
            "slot_acquired",
            Some(duration_millis(slot_wait_started)),
            None,
        );

        let signature_changed = slot.signature.is_some() && slot.signature != Some(signature);
        let client_closed = slot
            .client
            .as_ref()
            .map(InternalPostgresClient::is_closed)
            .unwrap_or(true);
        let idle_ttl_expired = slot
            .client
            .as_ref()
            .is_some_and(|client| client.idle_for() >= INTERNAL_CONNECTION_IDLE_TTL);
        let replacement_reason = if signature_changed {
            Some("reason=connection_settings_changed")
        } else if client_closed {
            Some("reason=client_closed_or_unhealthy")
        } else if idle_ttl_expired {
            // A diagnostics connection is intentionally burst-scoped. If the
            // idle timer did not run (for example after OS suspend), replace
            // the old session instead of probing/reusing a long-lived socket.
            Some("reason=idle_ttl_expired")
        } else {
            None
        };

        if let Some(reason) = replacement_reason {
            if slot.client.is_some() {
                self.log_event(connection_id, "client_retired", None, Some(reason));

                // A replaced client must be shut down explicitly. Dropping the JoinHandle
                // only detaches its task and can leave an old PostgreSQL backend alive.
                let shutdown_started = Instant::now();
                let graceful = shutdown_slot_client(&mut slot).await;
                self.log_event(
                    connection_id,
                    if graceful {
                        "client_shutdown"
                    } else {
                        "client_shutdown_forced"
                    },
                    Some(duration_millis(shutdown_started)),
                    Some(reason),
                );
            }
        }

        if slot.signature != Some(signature) || slot.client.is_none() {
            slot.client_generation = next_counter(slot.client_generation);
            let application_name = format!(
                "{INTERNAL_APPLICATION_NAME_PREFIX}-c{connection_id}-p{}-g{}",
                std::process::id(),
                slot.client_generation,
            );
            let connect_started = Instant::now();
            let client =
                match InternalPostgresClient::connect_registry(input, &application_name).await {
                    Ok(client) => client,
                    Err(error) => {
                        self.log_event(
                            connection_id,
                            "client_connect_failed",
                            Some(duration_millis(connect_started)),
                            Some("result=error"),
                        );
                        return Err(error);
                    }
                };
            self.log_event(
                connection_id,
                "client_created",
                Some(duration_millis(connect_started)),
                Some(&format!("application_name={application_name}")),
            );

            slot.signature = Some(signature);
            slot.client = Some(client);
        } else {
            self.log_event(connection_id, "client_reused", None, None);
        }

        if !self.is_generation_active(generation) {
            let _ = shutdown_slot_client(&mut slot).await;
            return Err(crate::i18n::text(
                "Internal PostgreSQL operation отменена: registry shutdown",
                "Internal PostgreSQL operation cancelled: registry shutdown",
            )
            .into());
        }

        // Do not persist SQL text: it can contain literals and credentials.
        // Sizes are real UTF-8 bytes, not counts of characters in the UI.
        let fingerprint = Sha256::digest(sql.as_bytes())[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let query_context = format!(
            "application_name={} overall_timeout_ms={} statement_timeout_ms={} lock_timeout_ms={} sql_bytes={} max_output_bytes={} sql_fingerprint={}",
            policy.application_name,
            policy.overall_timeout_ms,
            policy.statement_timeout_ms,
            policy.lock_timeout_ms,
            sql.len(),
            policy.max_output_bytes,
            fingerprint,
        );
        let session = QUERY_LOG_SESSION.get_or_init(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        });
        let request_id = format!(
            "pg-{}-{session:x}-{}",
            std::process::id(),
            NEXT_QUERY_LOG_ID.fetch_add(1, Ordering::Relaxed),
        );
        self.log_query_event(
            connection_id,
            "query_started",
            None,
            &query_context,
            &request_id,
            None,
        );

        let query_started = Instant::now();
        let result = slot
            .client
            .as_mut()
            .expect("internal PostgreSQL slot must contain a connected client")
            .run_readonly_query(sql, policy)
            .await;
        let query_phase = match &result {
            Ok(_) => "query_finished",
            Err(error) if error.contains("превысил timeout") => "query_timeout",
            Err(_) => "query_failed",
        };
        let finish_context = match &result {
            Ok(output) => format!("{query_context} response_bytes={}", output.len()),
            Err(_) => query_context.clone(),
        };
        self.log_query_event(
            connection_id,
            query_phase,
            Some(duration_millis(query_started)),
            &finish_context,
            &request_id,
            result.as_ref().err().map(String::as_str),
        );

        if slot
            .client
            .as_ref()
            .map(InternalPostgresClient::is_closed)
            .unwrap_or(false)
        {
            self.log_event(
                connection_id,
                "client_retired",
                None,
                Some("reason=unhealthy_after_query"),
            );
            let shutdown_started = Instant::now();
            let graceful = shutdown_slot_client(&mut slot).await;
            self.log_event(
                connection_id,
                if graceful {
                    "client_shutdown"
                } else {
                    "client_shutdown_forced"
                },
                Some(duration_millis(shutdown_started)),
                Some("reason=unhealthy_after_query"),
            );
        }

        let idle_epoch = if slot.client.is_some() {
            slot.idle_epoch = next_counter(slot.idle_epoch);
            Some(slot.idle_epoch)
        } else {
            None
        };

        drop(slot);

        if let Some(idle_epoch) = idle_epoch {
            self.schedule_idle_shutdown(connection_id, Arc::clone(&slot_handle), idle_epoch);
        }

        result
    }

    #[cfg(test)]
    fn slot(&self, connection_id: i64) -> Result<RegistrySlot, String> {
        self.slot_for_query(connection_id).map(|(slot, _)| slot)
    }

    fn slot_for_query(&self, connection_id: i64) -> Result<(RegistrySlot, u64), String> {
        if connection_id <= 0 {
            return Err(crate::i18n::text(
                "Некорректный connection_id для internal PostgreSQL registry",
                "Invalid connection_id for internal PostgreSQL registry",
            )
            .into());
        }

        let mut state = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.shutting_down {
            return Err(crate::i18n::text(
                "Internal PostgreSQL registry закрывает соединения",
                "Internal PostgreSQL registry is closing connections",
            )
            .into());
        }
        if state.retiring.contains_key(&connection_id) {
            return Err(crate::i18n::text(
                "Internal PostgreSQL connection ещё завершает предыдущую операцию",
                "Internal PostgreSQL connection is still completing its previous operation",
            )
            .into());
        }
        let generation = state.generation;
        let slot = Arc::clone(
            state
                .slots
                .entry(connection_id)
                .or_insert_with(|| Arc::new(AsyncMutex::new(InternalPostgresSlot::default()))),
        );
        Ok((slot, generation))
    }

    fn is_generation_active(&self, generation: u64) -> bool {
        let state = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !state.shutting_down && state.generation == generation
    }
}

fn record_internal_postgres_event(
    application_log: Option<&ApplicationLog>,
    connection_id: i64,
    phase: &str,
    duration_ms: Option<u64>,
    context: Option<&str>,
) {
    record_internal_postgres_event_with_id(
        application_log,
        connection_id,
        phase,
        duration_ms,
        context,
        None,
        None,
    );
}

fn record_internal_postgres_event_with_id(
    application_log: Option<&ApplicationLog>,
    connection_id: i64,
    phase: &str,
    duration_ms: Option<u64>,
    context: Option<&str>,
    request_id: Option<&str>,
    error: Option<&str>,
) {
    let Some(log) = application_log else {
        return;
    };

    let connection_context = match context {
        Some(context) if !context.is_empty() => {
            format!("connection_id={connection_id} {context}")
        }
        _ => format!("connection_id={connection_id}"),
    };

    let level = if phase.contains("failed") || phase.contains("timeout") {
        "error"
    } else if phase.contains("deferred") || phase.contains("forced") || phase == "client_retired" {
        "warning"
    } else {
        "info"
    };

    let operation_id = DIAGNOSTIC_OPERATION_ID.try_with(Clone::clone).ok();
    let kind = DIAGNOSTIC_KIND.try_with(Clone::clone).ok();
    log.record_enriched(
        ApplicationLogEvent {
            level,
            component: "postgres",
            operation: "internal_postgres",
            phase,
            actor: None,
            request_id,
            duration_ms,
            context: Some(&connection_context),
            error,
        },
        operation_id.as_deref(),
        kind.map(|kind| crate::app_log::DiagnosticLogDetails {
            connection_id,
            kind,
            snapshot_id: None,
            unavailable_sections: None,
            total_sections: None,
            client_request_id: DIAGNOSTIC_CLIENT_REQUEST_ID
                .try_with(Clone::clone)
                .ok()
                .flatten(),
        }),
    );
}

fn next_counter(value: u64) -> u64 {
    let next = value.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

fn duration_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn shutdown_registry_slot(slot: RegistrySlot) -> bool {
    let client = match timeout(REGISTRY_SLOT_ACQUIRE_TIMEOUT, slot.lock()).await {
        Ok(mut slot) => {
            slot.signature = None;
            slot.idle_epoch = next_counter(slot.idle_epoch);
            slot.client.take()
        }
        Err(_) => return false,
    };

    if let Some(client) = client {
        let _ = client.shutdown().await;
    }

    true
}

async fn shutdown_slot_client(slot: &mut InternalPostgresSlot) -> bool {
    slot.signature = None;
    slot.idle_epoch = next_counter(slot.idle_epoch);

    match slot.client.take() {
        Some(client) => client.shutdown().await,
        None => true,
    }
}

impl InternalPostgresClient {
    async fn connect_registry(
        input: &ConnectionInput,
        application_name: &str,
    ) -> Result<Self, String> {
        Self::connect_with_mode(input, true, application_name).await
    }

    async fn connect_probe(input: &ConnectionInput) -> Result<Self, String> {
        Self::connect_with_mode(input, true, PROBE_APPLICATION_NAME).await
    }

    async fn connect_action(
        input: &ConnectionInput,
        application_name: &str,
    ) -> Result<Self, String> {
        Self::connect_with_mode(input, false, application_name).await
    }

    async fn connect_with_mode(
        input: &ConnectionInput,
        starts_read_only: bool,
        application_name: &str,
    ) -> Result<Self, String> {
        input.validate()?;

        let tls = build_tls(input)?;
        let config = build_config(input, starts_read_only, application_name);

        let (client, connection_task) = match &tls {
            InternalTls::Disabled => {
                let (client, connection) = timeout(CONNECT_OVERALL_TIMEOUT, config.connect(NoTls))
                    .await
                    .map_err(|_| "Подключение PostgreSQL превысило 12 секунд".to_owned())?
                    .map_err(format_postgres_error)?;

                let task = tokio::spawn(async move {
                    let _ = connection.await;
                });

                (client, task)
            }
            InternalTls::OpenSsl(connector) => {
                let (client, connection) =
                    timeout(CONNECT_OVERALL_TIMEOUT, config.connect(connector.clone()))
                        .await
                        .map_err(|_| "TLS-подключение PostgreSQL превысило 12 секунд".to_owned())?
                        .map_err(format_postgres_error)?;

                let task = tokio::spawn(async move {
                    let _ = connection.await;
                });

                (client, task)
            }
        };

        Ok(Self {
            client,
            tls,
            connection_task,
            healthy: true,
            last_used_at: Instant::now(),
        })
    }

    async fn shutdown(self) -> bool {
        let Self {
            client,
            tls: _,
            mut connection_task,
            healthy: _,
            last_used_at: _,
        } = self;

        drop(client);

        if timeout(CONNECTION_SHUTDOWN_TIMEOUT, &mut connection_task)
            .await
            .is_ok()
        {
            return true;
        }

        connection_task.abort();
        let _ = connection_task.await;
        false
    }

    pub fn is_closed(&self) -> bool {
        !self.healthy || self.client.is_closed() || self.connection_task.is_finished()
    }

    fn idle_for(&self) -> Duration {
        self.last_used_at.elapsed()
    }

    pub(crate) async fn run_readonly_query(
        &mut self,
        sql: &str,
        policy: QueryExecutionPolicy,
    ) -> Result<String, String> {
        self.run_readonly_query_encoded(sql, policy, OutputEncoding::Legacy)
            .await
    }

    async fn run_readonly_query_encoded(
        &mut self,
        sql: &str,
        policy: QueryExecutionPolicy,
        encoding: OutputEncoding,
    ) -> Result<String, String> {
        self.ensure_usable()?;
        let sql = validate_sql(sql, policy)?;

        let execution = timeout(
            Duration::from_millis(policy.overall_timeout_ms),
            self.run_readonly_query_inner(sql, policy, encoding),
        )
        .await;

        let (result, should_cancel, connection_is_usable) = match execution {
            Ok(Ok(output)) => {
                let output = output.trim();

                if output.is_empty() {
                    (
                        Err("Read-only PostgreSQL запрос не вернул данные".into()),
                        false,
                        true,
                    )
                } else {
                    (Ok(output.to_owned()), false, true)
                }
            }
            Ok(Err(InternalExecutionError::Postgres {
                message,
                connection_is_usable,
            })) => (Err(message), false, connection_is_usable),
            Ok(Err(InternalExecutionError::OutputTooLarge)) => (
                Err(format!(
                    "PostgreSQL output слишком большой; максимум {} bytes",
                    policy.max_output_bytes,
                )),
                true,
                false,
            ),
            Err(_) => (
                Err(format!(
                    "PostgreSQL запрос превысил timeout {} ms",
                    policy.overall_timeout_ms,
                )),
                true,
                false,
            ),
        };

        if should_cancel || !connection_is_usable {
            let cancel_error = if should_cancel {
                self.cancel_current_query().await.err()
            } else {
                None
            };

            // After a client-side timeout, truncated result stream, or protocol/transport
            // error we do not try to reuse this session. Disconnecting rolls back the open
            // transaction and avoids racing cleanup against an uncertain protocol state.
            self.healthy = false;

            return match result {
                Ok(output) => Ok(output),
                Err(error) => Err(append_cancel_error(error, cancel_error)),
            };
        }

        let cleanup_result = self.cleanup_session().await;

        match cleanup_result {
            Ok(()) => {
                self.last_used_at = Instant::now();
                result
            }
            Err(cleanup_error) => {
                self.healthy = false;

                let error = match result {
                    Ok(_) => format!(
                        "PostgreSQL запрос завершён, но internal session не удалось очистить: {cleanup_error}"
                    ),
                    Err(error) => {
                        format!("{error}; internal session cleanup failed: {cleanup_error}")
                    }
                };

                Err(error)
            }
        }
    }

    async fn run_readonly_query_inner(
        &mut self,
        sql: &str,
        policy: QueryExecutionPolicy,
        encoding: OutputEncoding,
    ) -> Result<String, InternalExecutionError> {
        self.client
            .batch_execute("SET default_transaction_read_only = on")
            .await
            .map_err(postgres_execution_error)?;

        self.client
            .batch_execute("BEGIN READ ONLY")
            .await
            .map_err(postgres_execution_error)?;

        let setup = format!(
            "SET LOCAL statement_timeout = {}; \
             SET LOCAL lock_timeout = {}; \
             SET LOCAL application_name = '{}';",
            policy.statement_timeout_ms,
            policy.lock_timeout_ms,
            escape_sql_literal(policy.application_name),
        );

        self.client
            .batch_execute(&setup)
            .await
            .map_err(postgres_execution_error)?;

        collect_simple_query_output_encoded(&self.client, sql, policy.max_output_bytes, encoding)
            .await
    }

    async fn run_action(&mut self, sql: &str, policy: QueryExecutionPolicy) -> Result<(), String> {
        self.run_action_output(sql, policy).await.map(|_| ())
    }

    async fn run_action_output(
        &mut self,
        sql: &str,
        policy: QueryExecutionPolicy,
    ) -> Result<String, String> {
        self.ensure_usable()?;
        let sql = validate_sql(sql, policy)?;

        let execution = timeout(
            Duration::from_millis(policy.overall_timeout_ms),
            self.run_action_output_inner(sql, policy),
        )
        .await;

        match execution {
            Ok(Ok(output)) => Ok(output.trim().to_owned()),
            Ok(Err(InternalExecutionError::Postgres {
                message,
                connection_is_usable,
            })) => {
                if !connection_is_usable {
                    self.healthy = false;
                    Err(action_outcome_unknown(&message, None))
                } else {
                    Err(message)
                }
            }
            Ok(Err(InternalExecutionError::OutputTooLarge)) => {
                let cancel_error = self.cancel_current_query().await.err();
                self.healthy = false;
                Err(action_outcome_unknown(
                    &format!(
                        "PostgreSQL output слишком большой; максимум {} bytes",
                        policy.max_output_bytes
                    ),
                    cancel_error,
                ))
            }
            Err(_) => {
                let cancel_error = self.cancel_current_query().await.err();
                self.healthy = false;
                Err(action_outcome_unknown(
                    &format!(
                        "PostgreSQL запрос превысил timeout {} ms",
                        policy.overall_timeout_ms
                    ),
                    cancel_error,
                ))
            }
        }
    }

    async fn run_action_output_inner(
        &mut self,
        sql: &str,
        policy: QueryExecutionPolicy,
    ) -> Result<String, InternalExecutionError> {
        let setup = format!(
            "SET statement_timeout = {}; \
             SET lock_timeout = {}; \
             SET application_name = '{}';",
            policy.statement_timeout_ms,
            policy.lock_timeout_ms,
            escape_sql_literal(policy.application_name),
        );

        self.client
            .batch_execute(&setup)
            .await
            .map_err(postgres_execution_error)?;

        collect_simple_query_output(&self.client, sql, policy.max_output_bytes).await
    }

    async fn cancel_current_query(&self) -> Result<(), String> {
        let token = self.client.cancel_token();

        timeout(CANCEL_TIMEOUT, self.tls.cancel(&token))
            .await
            .map_err(|_| "CancelRequest PostgreSQL превысил 5 секунд".to_owned())?
            .map_err(format_postgres_error)
    }

    async fn cleanup_session(&self) -> Result<(), String> {
        timeout(CLEANUP_TIMEOUT, async {
            self.client
                .batch_execute("ROLLBACK")
                .await
                .map_err(format_postgres_error)?;

            // We reuse the TCP/PostgreSQL connection, not mutable session state.
            // DISCARD ALL also releases session advisory locks and drops temp objects.
            self.client
                .batch_execute("DISCARD ALL")
                .await
                .map_err(format_postgres_error)?;

            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "Очистка internal PostgreSQL session превысила 10 секунд".to_owned())?
    }

    fn ensure_usable(&self) -> Result<(), String> {
        if self.is_closed() {
            return Err(crate::i18n::text(
                "Internal PostgreSQL connection закрыто; требуется новое подключение",
                "Internal PostgreSQL connection is closed; reconnect",
            )
            .into());
        }

        Ok(())
    }
}

#[derive(Debug)]
enum InternalExecutionError {
    Postgres {
        message: String,
        connection_is_usable: bool,
    },
    OutputTooLarge,
}

// Legacy output is preserved for existing single-column JSON and probe callers.
// Use JsonRows for SQL that returns arbitrary columns, NULLs, pipes or newlines.
#[derive(Clone, Copy)]
enum OutputEncoding {
    Legacy,
    JsonRows,
}

fn action_outcome_unknown(reason: &str, cancel_error: Option<String>) -> String {
    append_cancel_error(
        format!("{reason}; результат write-операции НЕИЗВЕСТЕН: PostgreSQL мог уже зафиксировать изменения. Проверьте состояние БД перед повтором; не запускайте действие автоматически повторно"),
        cancel_error,
    )
}

async fn collect_simple_query_output(
    client: &Client,
    sql: &str,
    max_output_bytes: usize,
) -> Result<String, InternalExecutionError> {
    collect_simple_query_output_encoded(client, sql, max_output_bytes, OutputEncoding::Legacy).await
}

async fn collect_simple_query_output_encoded(
    client: &Client,
    sql: &str,
    max_output_bytes: usize,
    encoding: OutputEncoding,
) -> Result<String, InternalExecutionError> {
    let stream = client
        .simple_query_raw(sql)
        .await
        .map_err(postgres_execution_error)?;
    tokio::pin!(stream);

    let mut output = String::new();
    if matches!(encoding, OutputEncoding::JsonRows) {
        append_limited(&mut output, "[", max_output_bytes)?;
    }

    let mut row_count = 0;
    while let Some(message) = stream.try_next().await.map_err(postgres_execution_error)? {
        if let SimpleQueryMessage::Row(row) = message {
            match encoding {
                OutputEncoding::Legacy => append_psql_row(&mut output, &row, max_output_bytes)?,
                OutputEncoding::JsonRows => {
                    append_json_row(&mut output, &row, row_count > 0, max_output_bytes)?;
                }
            }
            row_count += 1;
        }
    }
    if matches!(encoding, OutputEncoding::JsonRows) {
        append_limited(&mut output, "]", max_output_bytes)?;
    }

    Ok(output)
}

fn append_json_row(
    output: &mut String,
    row: &SimpleQueryRow,
    has_previous: bool,
    max_output_bytes: usize,
) -> Result<(), InternalExecutionError> {
    if has_previous {
        append_limited(output, ",", max_output_bytes)?;
    }
    append_limited(output, "[", max_output_bytes)?;
    for index in 0..row.len() {
        if index > 0 {
            append_limited(output, ",", max_output_bytes)?;
        }
        append_json_cell(output, row.get(index), max_output_bytes)?;
    }
    append_limited(output, "]", max_output_bytes)
}

fn append_json_cell(
    output: &mut String,
    value: Option<&str>,
    max_output_bytes: usize,
) -> Result<(), InternalExecutionError> {
    let encoded =
        serde_json::to_string(&value).expect("serializing an optional string to JSON cannot fail");
    append_limited(output, &encoded, max_output_bytes)
}

fn append_psql_row(
    output: &mut String,
    row: &SimpleQueryRow,
    max_output_bytes: usize,
) -> Result<(), InternalExecutionError> {
    for index in 0..row.len() {
        if index > 0 {
            append_limited(output, "|", max_output_bytes)?;
        }

        if let Some(value) = row.get(index) {
            append_limited(output, value, max_output_bytes)?;
        }
    }

    append_limited(output, "\n", max_output_bytes)
}

fn append_limited(
    output: &mut String,
    value: &str,
    max_output_bytes: usize,
) -> Result<(), InternalExecutionError> {
    if output.len().saturating_add(value.len()) > max_output_bytes {
        return Err(InternalExecutionError::OutputTooLarge);
    }

    output.push_str(value);
    Ok(())
}

pub async fn probe(input: ConnectionInput) -> Result<String, String> {
    timeout(CONNECT_OVERALL_TIMEOUT, async {
        let mut client = InternalPostgresClient::connect_probe(&input).await?;

        let result = client
            .run_readonly_query(
                "SELECT current_database(), current_user, current_setting('server_version');",
                QueryExecutionPolicy::probe(),
            )
            .await;

        let _ = client.shutdown().await;
        result
    })
    .await
    .map_err(|_| "Проверка PostgreSQL превысила 12 секунд".to_owned())?
}

pub(crate) async fn run_readonly_once(
    input: &ConnectionInput,
    sql: &str,
    policy: QueryExecutionPolicy,
) -> Result<String, String> {
    let mut client =
        InternalPostgresClient::connect_with_mode(input, true, policy.application_name).await?;

    let result = client.run_readonly_query(sql, policy).await;
    let _ = client.shutdown().await;
    result
}

/// Lossless, JSON-encoded rows for callers that need columns/NULL to remain distinct.
/// Legacy diagnostic callers deliberately keep their existing text contract.
#[allow(dead_code)]
pub(crate) async fn run_readonly_once_json_rows(
    input: &ConnectionInput,
    sql: &str,
    policy: QueryExecutionPolicy,
) -> Result<String, String> {
    let mut client =
        InternalPostgresClient::connect_with_mode(input, true, policy.application_name).await?;
    let result = client
        .run_readonly_query_encoded(sql, policy, OutputEncoding::JsonRows)
        .await;
    let _ = client.shutdown().await;
    result
}

pub(crate) async fn run_action_once_output(
    input: &ConnectionInput,
    sql: &str,
    policy: QueryExecutionPolicy,
) -> Result<String, String> {
    let mut client = InternalPostgresClient::connect_action(input, policy.application_name).await?;

    let result = client.run_action_output(sql, policy).await;
    let _ = client.shutdown().await;
    result
}

pub(crate) async fn run_action_once(
    input: &ConnectionInput,
    sql: &str,
    policy: QueryExecutionPolicy,
) -> Result<(), String> {
    let mut client = InternalPostgresClient::connect_action(input, policy.application_name).await?;

    let result = client.run_action(sql, policy).await;
    let _ = client.shutdown().await;
    result
}

fn connection_signature(input: &ConnectionInput) -> [u8; 32] {
    let mut hasher = Sha256::new();

    hash_signature_field(&mut hasher, input.host.as_bytes());
    hasher.update(input.port.to_be_bytes());
    hash_signature_field(&mut hasher, input.database.as_bytes());
    hash_signature_field(&mut hasher, input.username.as_bytes());
    hash_signature_field(&mut hasher, input.password.as_bytes());
    hash_signature_field(&mut hasher, input.sslmode.as_bytes());

    hasher.finalize().into()
}

fn hash_signature_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn validate_sql(sql: &str, policy: QueryExecutionPolicy) -> Result<&str, String> {
    let sql = sql.trim();

    if sql.is_empty() || sql.len() > policy.max_sql_bytes || sql.contains('\0') {
        return Err("Некорректный SQL".into());
    }

    let statements = pg_query::split_with_parser(sql)
        .map_err(|error| format!("PostgreSQL parser отклонил internal SQL: {error}"))?;

    if statements.len() != 1 {
        return Err("Internal PostgreSQL executor принимает ровно один SQL statement".into());
    }

    Ok(sql)
}

fn build_config(input: &ConnectionInput, starts_read_only: bool, application_name: &str) -> Config {
    let mut config = Config::new();

    config
        .host(&input.host)
        .port(input.port)
        .dbname(&input.database)
        .user(&input.username)
        .password(&input.password)
        .connect_timeout(CONNECT_TIMEOUT)
        .application_name(application_name);

    if starts_read_only {
        config.options("-c client_encoding=UTF8 -c default_transaction_read_only=on");
    } else {
        config.options("-c client_encoding=UTF8");
    }

    match input.sslmode.as_str() {
        "disable" => {
            config.ssl_mode(SslMode::Disable);
        }
        "require" | "verify-full" => {
            config.ssl_mode(SslMode::Require);
        }
        _ => unreachable!("ConnectionInput::validate rejects unknown sslmode"),
    }

    config
}

fn build_tls(input: &ConnectionInput) -> Result<InternalTls, String> {
    match input.sslmode.as_str() {
        "disable" => Ok(InternalTls::Disabled),
        "require" => build_openssl_connector(false).map(InternalTls::OpenSsl),
        "verify-full" => build_openssl_connector(true).map(InternalTls::OpenSsl),
        _ => unreachable!("ConnectionInput::validate rejects unknown sslmode"),
    }
}

fn build_openssl_connector(verify_full: bool) -> Result<MakeTlsConnector, String> {
    let mut builder =
        SslConnector::builder(SslMethod::tls()).map_err(|error| format!("TLS init: {error}"))?;

    load_default_client_identity(&mut builder)?;

    if verify_full {
        let root_cert = default_postgres_tls_dir()?
            .ok_or(crate::i18n::text(
                "Для verify-full нужен HOME с ~/.postgresql/root.crt",
                "verify-full requires HOME with ~/.postgresql/root.crt",
            ))?
            .join("root.crt");

        if !root_cert.is_file() {
            return Err(if crate::i18n::is_ru() {
                format!(
                    "Для verify-full нужен доверенный CA-файл {}",
                    root_cert.display()
                )
            } else {
                format!(
                    "verify-full requires a trusted CA certificate file {}",
                    root_cert.display()
                )
            });
        }

        let mut store =
            X509StoreBuilder::new().map_err(|error| format!("TLS CA store: {error}"))?;
        let lookup = store
            .add_lookup(X509Lookup::file())
            .map_err(|error| format!("TLS CA lookup: {error}"))?;
        lookup
            .load_cert_file(&root_cert, SslFiletype::PEM)
            .map_err(|error| {
                if crate::i18n::is_ru() {
                    format!("Не удалось загрузить {}: {error}", root_cert.display())
                } else {
                    format!("Could not load {}: {error}", root_cert.display())
                }
            })?;

        let root_crl = root_cert.with_file_name("root.crl");

        if root_crl.is_file() {
            lookup
                .load_crl_file(&root_crl, SslFiletype::PEM)
                .map_err(|error| {
                    if crate::i18n::is_ru() {
                        format!("Не удалось загрузить {}: {error}", root_crl.display())
                    } else {
                        format!("Could not load {}: {error}", root_crl.display())
                    }
                })?;
            store
                .set_flags(X509VerifyFlags::CRL_CHECK | X509VerifyFlags::CRL_CHECK_ALL)
                .map_err(|error| format!("Не удалось включить проверку PostgreSQL CRL: {error}"))?;
        }

        builder.set_cert_store(store.build());
        builder.set_verify(SslVerifyMode::PEER);

        return Ok(MakeTlsConnector::new(builder.build()));
    }

    builder.set_verify(SslVerifyMode::NONE);
    let mut connector = MakeTlsConnector::new(builder.build());
    connector.set_callback(|configuration, _| {
        configuration.set_verify_hostname(false);
        Ok(())
    });

    Ok(connector)
}

fn load_default_client_identity(
    builder: &mut openssl::ssl::SslConnectorBuilder,
) -> Result<(), String> {
    let Some(directory) = default_postgres_tls_dir()? else {
        return Ok(());
    };

    let certificate = directory.join("postgresql.crt");
    let private_key = directory.join("postgresql.key");
    let certificate_exists = certificate.is_file();
    let private_key_exists = private_key.is_file();

    if !certificate_exists && !private_key_exists {
        return Ok(());
    }

    if certificate_exists != private_key_exists {
        return Err(format!(
            "PostgreSQL client TLS требует оба файла: {} и {}",
            certificate.display(),
            private_key.display(),
        ));
    }

    validate_private_key_permissions(&private_key)?;

    builder
        .set_certificate_chain_file(&certificate)
        .map_err(|error| format!("Не удалось загрузить {}: {error}", certificate.display()))?;
    builder
        .set_private_key_file(&private_key, SslFiletype::PEM)
        .map_err(|error| format!("Не удалось загрузить {}: {error}", private_key.display()))?;
    builder.check_private_key().map_err(|error| {
        format!("PostgreSQL TLS private key не соответствует certificate: {error}")
    })?;

    Ok(())
}

fn validate_private_key_permissions(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Не удалось проверить {}: {error}", path.display()))?;
    let mode = metadata.permissions().mode() & 0o777;
    let owner_only = mode & 0o077 == 0;
    let root_owned_group_read = metadata.uid() == 0 && mode & 0o037 == 0;

    if !owner_only && !root_owned_group_read {
        return Err(format!(
            "PostgreSQL TLS private key {} имеет небезопасные права {:03o}; используйте 0600 или root-owned 0640",
            path.display(),
            mode,
        ));
    }

    Ok(())
}

fn default_postgres_tls_dir() -> Result<Option<PathBuf>, String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(None);
    };

    let home = PathBuf::from(home);

    if !home.is_absolute() {
        return Err("HOME должен быть абсолютным путём для PostgreSQL TLS files".into());
    }

    Ok(Some(home.join(".postgresql")))
}

fn postgres_execution_error(error: tokio_postgres::Error) -> InternalExecutionError {
    let connection_is_usable = error.as_db_error().is_some();
    let message = format_postgres_error(error);

    InternalExecutionError::Postgres {
        message,
        connection_is_usable,
    }
}

fn format_postgres_error(error: tokio_postgres::Error) -> String {
    if let Some(database_error) = error.as_db_error() {
        let mut message = format!(
            "{} (SQLSTATE {})",
            database_error.message(),
            database_error.code().code(),
        );

        if let Some(detail) = database_error.detail() {
            message.push_str("; detail: ");
            message.push_str(detail);
        }

        if let Some(hint) = database_error.hint() {
            message.push_str("; hint: ");
            message.push_str(hint);
        }

        return message;
    }

    format!("PostgreSQL connection/protocol error: {error}")
}

fn append_cancel_error(error: String, cancel_error: Option<String>) -> String {
    match cancel_error {
        Some(cancel_error) => format!("{error}; CancelRequest failed: {cancel_error}"),
        None => error,
    }
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_executor_accepts_one_statement_only() {
        let policy = QueryExecutionPolicy::standard_diagnostic();

        assert!(validate_sql("SELECT 1;", policy).is_ok());
        assert!(validate_sql("SELECT 1; SELECT 2;", policy).is_err());
    }

    #[test]
    fn psql_compatible_output_preserves_null_and_columns() {
        let mut output = String::new();

        append_limited(&mut output, "a", 16).unwrap();
        append_limited(&mut output, "|", 16).unwrap();
        append_limited(&mut output, "", 16).unwrap();
        append_limited(&mut output, "|b\n", 16).unwrap();

        assert_eq!(output, "a||b\n");
    }

    #[test]
    fn output_limit_is_checked_before_append() {
        let mut output = "1234".to_owned();

        assert!(matches!(
            append_limited(&mut output, "56", 5),
            Err(InternalExecutionError::OutputTooLarge),
        ));
        assert_eq!(output, "1234");
    }

    #[test]
    fn config_keeps_explicit_connection_fields() {
        let input = ConnectionInput {
            host: "db.example.test".into(),
            port: 5544,
            database: "app".into(),
            username: "pgxray".into(),
            password: "secret".into(),
            sslmode: "disable".into(),
        };

        let config = build_config(&input, true, INTERNAL_APPLICATION_NAME_PREFIX);

        assert_eq!(config.get_dbname(), Some("app"));
        assert_eq!(config.get_user(), Some("pgxray"));
        assert_eq!(config.get_ports(), &[5544]);
        assert_eq!(
            config.get_application_name(),
            Some(INTERNAL_APPLICATION_NAME_PREFIX)
        );
        assert_eq!(
            config.get_options(),
            Some("-c client_encoding=UTF8 -c default_transaction_read_only=on"),
        );
        assert_eq!(config.get_ssl_mode(), SslMode::Disable);
    }

    #[tokio::test]
    async fn registry_keeps_slot_identity_across_invalidate() {
        let registry = InternalPostgresRegistry::new();
        let first = registry.slot(7).unwrap();
        let second = registry.slot(7).unwrap();

        assert!(Arc::ptr_eq(&first, &second));

        registry.invalidate(7).await;

        let third = registry.slot(7).unwrap();
        assert!(Arc::ptr_eq(&first, &third));
    }

    #[tokio::test]
    async fn registry_keeps_connections_independent() {
        let registry = InternalPostgresRegistry::new();
        let first = registry.slot(1).unwrap();
        let second = registry.slot(2).unwrap();

        assert!(!Arc::ptr_eq(&first, &second));

        registry.shutdown_all().await;

        let first_after_clear = registry.slot(1).unwrap();
        assert!(!Arc::ptr_eq(&first, &first_after_clear));
    }

    #[test]
    fn connection_signature_changes_when_credentials_change() {
        let first = ConnectionInput {
            host: "db.example.test".into(),
            port: 5432,
            database: "app".into(),
            username: "pgxray".into(),
            password: "first".into(),
            sslmode: "require".into(),
        };
        let mut second = ConnectionInput {
            host: "db.example.test".into(),
            port: 5432,
            database: "app".into(),
            username: "pgxray".into(),
            password: "first".into(),
            sslmode: "require".into(),
        };

        assert_eq!(connection_signature(&first), connection_signature(&second));

        second.password = "second".into();
        assert_ne!(connection_signature(&first), connection_signature(&second));
    }

    #[test]
    fn reusable_connection_is_only_kept_for_short_query_bursts() {
        assert_eq!(INTERNAL_CONNECTION_IDLE_TTL, Duration::from_secs(2));
    }

    #[test]
    fn counters_never_use_zero_after_wrap() {
        assert_eq!(next_counter(0), 1);
        assert_eq!(next_counter(u64::MAX), 1);
    }

    #[test]
    fn client_private_key_rejects_group_or_world_access() {
        let key = tempfile::NamedTempFile::new().unwrap();

        fs::set_permissions(key.path(), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(validate_private_key_permissions(key.path()).is_ok());

        fs::set_permissions(key.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_private_key_permissions(key.path()).is_err());
    }
    #[test]
    fn json_rows_preserve_null_delimiters_and_newlines() {
        let mut output = String::from("[[");
        append_json_cell(&mut output, Some("a|b"), 128).unwrap();
        output.push(',');
        append_json_cell(&mut output, None, 128).unwrap();
        output.push(',');
        append_json_cell(&mut output, Some("first\nsecond"), 128).unwrap();
        output.push_str("]]");

        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed, serde_json::json!([["a|b", null, "first\nsecond"]]));
    }

    #[test]
    fn json_rows_limit_counts_escaped_bytes() {
        let mut output = String::new();
        assert!(matches!(
            append_json_cell(&mut output, Some("a\nb"), 5),
            Err(InternalExecutionError::OutputTooLarge),
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn uncertain_write_error_warns_not_to_retry() {
        let error = action_outcome_unknown("SQL timeout", Some("cancel failed".into()));
        assert!(error.contains("НЕИЗВЕСТЕН"));
        assert!(error.contains("перед повтором"));
        assert!(error.contains("cancel failed"));
    }

    #[tokio::test]
    async fn shutdown_changes_generation_and_allows_later_login() {
        let registry = InternalPostgresRegistry::new();
        let (_old_slot, old_generation) = registry.slot_for_query(7).unwrap();
        registry.shutdown_all().await;
        assert!(!registry.is_generation_active(old_generation));
        let (_new_slot, new_generation) = registry.slot_for_query(7).unwrap();
        assert!(registry.is_generation_active(new_generation));
    }

    #[tokio::test]
    async fn concurrent_shutdown_rejects_new_operations_until_complete() {
        let registry = Arc::new(InternalPostgresRegistry::new());
        let busy = registry.slot(17).unwrap();
        let guard = busy.lock().await;
        let shutting_down = Arc::clone(&registry);
        let shutdown = tokio::spawn(async move { shutting_down.shutdown_all().await });

        // The active slot prevents shutdown from finishing immediately.
        for _ in 0..100 {
            if registry.slots.lock().unwrap().shutting_down {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(registry.slots.lock().unwrap().shutting_down);
        assert!(registry.slot_for_query(17).is_err());
        assert!(registry.slot_for_query(18).is_err());

        drop(guard);
        shutdown.await.unwrap();
        assert!(registry.slot_for_query(17).is_ok());
    }

    #[test]
    fn shutdown_and_retiring_slots_reject_new_requests() {
        let registry = InternalPostgresRegistry::new();
        let slot = registry.slot(7).unwrap();
        {
            let mut state = registry.slots.lock().unwrap();
            state.shutting_down = true;
        }
        assert!(registry.slot_for_query(8).is_err());
        {
            let mut state = registry.slots.lock().unwrap();
            state.shutting_down = false;
            state.retiring.insert(7, slot);
        }
        assert!(registry.slot_for_query(7).is_err());
        assert!(registry.slot_for_query(8).is_ok());
    }

    // Run explicitly against a disposable PostgreSQL instance:
    // PGXRAY_TEST_HOST=127.0.0.1 PGXRAY_TEST_USER=postgres ... cargo test ... --ignored
    fn integration_input() -> ConnectionInput {
        ConnectionInput {
            host: std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST required"),
            port: std::env::var("PGXRAY_TEST_PORT")
                .unwrap_or_else(|_| "5432".into())
                .parse()
                .expect("PGXRAY_TEST_PORT must be a port"),
            database: std::env::var("PGXRAY_TEST_DB").unwrap_or_else(|_| "postgres".into()),
            username: std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER required"),
            password: std::env::var("PGXRAY_TEST_PASSWORD").unwrap_or_default(),
            sslmode: std::env::var("PGXRAY_TEST_SSLMODE").unwrap_or_else(|_| "disable".into()),
        }
    }

    #[tokio::test]
    #[ignore = "requires explicitly configured PostgreSQL instance"]
    async fn integration_structured_rows_preserve_values() {
        let output = run_readonly_once_json_rows(
            &integration_input(),
            "SELECT 'a|b'::text, NULL::text, E'line\nnext'::text",
            QueryExecutionPolicy::standard_diagnostic(),
        )
        .await
        .unwrap();
        let rows: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(rows, serde_json::json!([["a|b", null, "line\nnext"]]));
    }

    #[tokio::test]
    #[ignore = "requires explicitly configured PostgreSQL instance"]
    async fn integration_readonly_timeout_retires_connection() {
        let input = integration_input();
        let mut client = InternalPostgresClient::connect_probe(&input).await.unwrap();
        let mut policy = QueryExecutionPolicy::standard_diagnostic();
        policy.overall_timeout_ms = 100;
        policy.statement_timeout_ms = 10_000;
        let error = client
            .run_readonly_query("SELECT pg_sleep(2)", policy)
            .await
            .unwrap_err();
        assert!(error.contains("timeout"));
        assert!(client.is_closed());
        let _ = client.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "requires explicitly configured PostgreSQL instance"]
    async fn integration_action_timeout_reports_uncertain_outcome() {
        let input = integration_input();
        let mut client = InternalPostgresClient::connect_action(&input, "pgxray-test-action")
            .await
            .unwrap();
        let mut policy = QueryExecutionPolicy::standard_diagnostic();
        policy.overall_timeout_ms = 100;
        policy.statement_timeout_ms = 10_000;
        let error = client
            .run_action_output("SELECT pg_sleep(2)", policy)
            .await
            .unwrap_err();
        assert!(error.contains("НЕИЗВЕСТЕН"));
        assert!(client.is_closed());
        let _ = client.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "requires explicitly configured PostgreSQL instance"]
    async fn integration_readonly_registry_reuses_clean_session() {
        let input = integration_input();
        let registry = InternalPostgresRegistry::new();
        let policy = QueryExecutionPolicy::standard_diagnostic();
        assert_eq!(
            registry
                .run_readonly_query(1, &input, "SELECT 1", policy)
                .await
                .unwrap(),
            "1",
        );
        assert_eq!(
            registry
                .run_readonly_query(
                    1,
                    &input,
                    "SELECT current_setting('transaction_read_only')",
                    policy
                )
                .await
                .unwrap(),
            "on",
        );
        registry.shutdown_all().await;
    }
}
