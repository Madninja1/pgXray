use crate::postgres::{create_passfile, ConnectionInput};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
};
use tempfile::{NamedTempFile, TempDir};

#[derive(Clone, Serialize)]
pub struct PsqlSessionInfo {
    pub id: u64,
    pub connection_id: i64,
    pub connection_name: String,
    pub case_id: Option<i64>,
    pub workspace_path: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub sslmode: String,
    // Legacy IPC fields retained for compatibility; no SQL restrictions are imposed.
    pub starts_read_only: bool,
    pub idle_in_transaction_timeout_ms: u64,
    pub running: bool,
    pub psqlrc_path: String,
    pub editor: String,
}

#[derive(Clone, Serialize)]
pub struct PsqlSessionStart {
    pub session: PsqlSessionInfo,
    pub created: bool,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PsqlSessionEvent {
    Output { session_id: u64, data: String },
    Exit { session_id: u64 },
    StreamEnded { session_id: u64 },
    OutputGap { session_id: u64 },
}

// PTY output is sensitive. Keep a bounded *in-memory* tail for WebView reload;
// never write this data to application logs or SQLite.
const REPLAY_LIMIT: usize = 64 * 1024;

#[derive(Default)]
struct OutputRelay {
    tail: String,
    receiver: Option<SyncSender<PsqlSessionEvent>>,
    skipped: bool,
}

impl OutputRelay {
    fn add_output(&mut self, session_id: u64, data: String) {
        self.tail.push_str(&data);
        if self.tail.len() > REPLAY_LIMIT {
            let mut start = self.tail.len() - REPLAY_LIMIT;
            while !self.tail.is_char_boundary(start) {
                start += 1;
            }
            self.tail.drain(..start);
        }
        self.send(session_id, PsqlSessionEvent::Output { session_id, data });
    }

    fn send(&mut self, session_id: u64, event: PsqlSessionEvent) {
        let Some(sender) = self.receiver.as_ref() else {
            return;
        };
        if self.skipped {
            match sender.try_send(PsqlSessionEvent::OutputGap { session_id }) {
                Ok(()) => self.skipped = false,
                Err(TrySendError::Full(_)) => return,
                Err(TrySendError::Disconnected(_)) => {
                    self.receiver = None;
                    return;
                }
            }
        }
        match sender.try_send(event) {
            Ok(()) => (),
            Err(TrySendError::Full(_)) => self.skipped = true,
            Err(TrySendError::Disconnected(_)) => self.receiver = None,
        }
    }

    fn subscribe(&mut self, session_id: u64, sender: SyncSender<PsqlSessionEvent>) {
        // A desktop WebView has one active consumer. A second subscription
        // replaces the stale channel, rather than duplicating sensitive output.
        if !self.tail.is_empty() {
            let _ = sender.try_send(PsqlSessionEvent::Output {
                session_id,
                data: self.tail.clone(),
            });
        }
        self.receiver = Some(sender);
        self.skipped = false;
    }
}

// Holds an unregistered process while its PTY reader/writer are initialized.
// Any error before registration must kill and reap the child.
struct StartingChild(Option<Box<dyn Child + Send + Sync>>);

impl Drop for StartingChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

struct RunningSession {
    info: PsqlSessionInfo,
    child: Option<Box<dyn Child + Send + Sync>>,
    alive: Arc<AtomicBool>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    relay: Arc<Mutex<OutputRelay>>,
    _passfile: NamedTempFile,
    _home_dir: TempDir,
}

impl RunningSession {
    fn running(&mut self) -> bool {
        if !self.alive.load(Ordering::Acquire) {
            return false;
        }
        let Some(child) = self.child.as_mut() else {
            return self.alive.load(Ordering::Acquire);
        };
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => {
                self.alive.store(false, Ordering::Release);
                false
            }
            Err(_) => true, // Unknown state must never release the permit.
        }
    }

    fn stop(&mut self) -> Result<(), String> {
        // A previous stop may already have handed the child to the reaper.
        if self.child.is_none() {
            return Ok(());
        }
        if self.running() {
            let child = self.child.as_mut().expect("checked above");
            if let Err(error) = child.kill() {
                if !matches!(child.try_wait(), Ok(Some(_))) {
                    return Err(format!("Не удалось остановить psql: {error}"));
                }
            }
        }
        self.reap_in_background();
        Ok(())
    }

    fn reap_in_background(&mut self) {
        if let Some(mut child) = self.child.take() {
            let alive = Arc::clone(&self.alive);
            thread::spawn(move || {
                // wait() must never hold the PsqlSessionManager mutex.
                let _ = child.wait();
                alive.store(false, Ordering::Release);
            });
        }
    }
}

impl Drop for RunningSession {
    fn drop(&mut self) {
        if self.stop().is_err() {
            // At shutdown retry termination without blocking the manager.
            if let Some(mut child) = self.child.take() {
                let alive = Arc::clone(&self.alive);
                thread::spawn(move || {
                    let _ = child.kill();
                    let _ = child.wait();
                    alive.store(false, Ordering::Release);
                });
            }
        }
    }
}

pub struct PsqlSessionManager {
    next_id: u64,
    sessions: HashMap<u64, RunningSession>,
    psqlrc_dir: PathBuf,
}

impl PsqlSessionManager {
    pub fn new(config_dir: &Path) -> Result<Self, String> {
        let psqlrc_dir = config_dir.join("psqlrc");

        create_private_directory(&psqlrc_dir)?;
        // Do not create persistent SQL history. SQL can contain credentials.
        Ok(Self {
            next_id: 1,
            sessions: HashMap::new(),
            psqlrc_dir,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "existing call contract; refactor separately with callers"
    )]
    pub fn start(
        &mut self,
        connection_id: i64,
        connection_name: &str,
        case_id: Option<i64>,
        workspace_dir: PathBuf,
        input: ConnectionInput,
        cols: u16,
        rows: u16,
        event_sender: SyncSender<PsqlSessionEvent>,
        force_new: bool,
    ) -> Result<PsqlSessionStart, String> {
        input.validate()?;

        self.cleanup_finished();

        validate_terminal_size(cols, rows)?;

        let workspace_dir = workspace_dir
            .canonicalize()
            .map_err(|error| format!("Рабочий каталог psql недоступен: {error}"))?;

        if !workspace_dir.is_dir() {
            return Err(crate::i18n::text(
                "Рабочий каталог psql не является директорией",
                "psql working directory is not a directory",
            )
            .into());
        }

        if !force_new {
            for session in self.sessions.values_mut() {
                if session.info.connection_id == connection_id
                    && session.info.case_id == case_id
                    && session.running()
                {
                    session.info.running = true;

                    return Ok(PsqlSessionStart {
                        session: session.info.clone(),
                        created: false,
                    });
                }
            }
        }

        let id = self.next_id;

        self.next_id = self.next_id.checked_add(1).ok_or(crate::i18n::text(
            "Исчерпан идентификатор PostgreSQL-сессий",
            "PostgreSQL session identifier space is exhausted",
        ))?;

        let passfile = create_passfile(&input)?;

        let psqlrc_path = ensure_psqlrc(&self.psqlrc_dir, connection_id)?;

        let editor = detect_editor();

        let home_dir = tempfile::tempdir().map_err(|_| {
            crate::i18n::text(
                "Не удалось создать приватный HOME для psql-сессии",
                "Failed to create a private HOME for the psql session",
            )
        })?;

        fs::set_permissions(home_dir.path(), fs::Permissions::from_mode(0o700)).map_err(|_| {
            crate::i18n::text(
                "Не удалось выставить права на HOME psql-сессии",
                "Failed to set permissions on the psql session HOME",
            )
        })?;

        let session_psqlrc_path = create_session_psqlrc(home_dir.path(), &psqlrc_path)?;
        let history_path = home_dir.path().join("psql-history"); // Ephemeral per PTY.

        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Не удалось создать PTY: {error}"))?;

        let mut command = CommandBuilder::new("/usr/bin/psql");

        command.env_clear();

        command.env("TERM", "xterm-256color");

        command.env("LANG", "C.UTF-8");

        command.env("HOME", home_dir.path());

        command.env("PGHOST", &input.host);

        command.env("PGPORT", input.port.to_string());

        command.env("PGDATABASE", &input.database);

        command.env("PGUSER", &input.username);

        command.env("PGCLIENTENCODING", "UTF8");

        command.env("PGPASSFILE", passfile.path());

        command.env("PGSSLMODE", &input.sslmode);

        command.env("PGCONNECT_TIMEOUT", "5");

        command.env("PGAPPNAME", format!("pgxray-session-{id}"));

        command.env("PSQLRC", &session_psqlrc_path);

        command.env("PSQL_HISTORY", &history_path);

        command.env("PSQL_EDITOR", &editor);

        command.env("EDITOR", &editor);

        command.env("VISUAL", &editor);

        command.env("PGSYSCONFDIR", home_dir.path());

        // Do not impose PostgreSQL GUCs via PGOPTIONS. In particular, PgBouncer
        // can reject startup parameters that a direct PostgreSQL server accepts.
        // Database authorization must be enforced by PostgreSQL role privileges.

        command.env("PGXRAY_WORKSPACE", &workspace_dir);

        command.cwd(&workspace_dir);

        command.arg("-w");

        command.arg("-v");

        command.arg(format!("PGXRAY_PROFILE={connection_name}"));

        let mut launching =
            StartingChild(Some(pair.slave.spawn_command(command).map_err(
                |error| format!("Не удалось запустить /usr/bin/psql в PTY: {error}"),
            )?));

        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("Не удалось открыть PTY для чтения: {error}"))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("Не удалось открыть PTY для записи: {error}"))?;

        if !matches!(
            launching.0.as_mut().expect("child is owned").try_wait(),
            Ok(None)
        ) {
            return Err(crate::i18n::text(
                "psql завершился сразу после запуска",
                "psql exited immediately after startup",
            )
            .into());
        }

        let relay = Arc::new(Mutex::new(OutputRelay::default()));
        relay
            .lock()
            .map_err(|_| crate::i18n::text("PTY relay недоступен", "PTY relay is unavailable"))?
            .subscribe(id, event_sender);
        let alive = Arc::new(AtomicBool::new(true));
        let reader_killer = launching.0.as_ref().expect("child is owned").clone_killer();
        let child = launching.0.take().expect("child is owned");
        start_reader(id, reader, Arc::clone(&relay), reader_killer);

        let info = PsqlSessionInfo {
            id,
            connection_id,
            connection_name: connection_name.to_owned(),
            case_id,
            workspace_path: workspace_dir.to_string_lossy().into_owned(),
            host: input.host,
            port: input.port,
            database: input.database,
            username: input.username,
            sslmode: input.sslmode,
            starts_read_only: false,
            idle_in_transaction_timeout_ms: 0,
            running: true,
            psqlrc_path: psqlrc_path.to_string_lossy().into_owned(),
            editor: editor.to_string_lossy().into_owned(),
        };

        self.sessions.insert(
            id,
            RunningSession {
                info: info.clone(),
                child: Some(child),
                alive,
                master: pair.master,
                writer,
                relay,
                _passfile: passfile,
                _home_dir: home_dir,
            },
        );

        Ok(PsqlSessionStart {
            session: info,
            created: true,
        })
    }

    pub fn process_status(&mut self, id: u64) -> Option<bool> {
        self.sessions.get_mut(&id).map(|session| session.running())
    }

    pub fn process_token(&self, id: u64) -> Option<Arc<AtomicBool>> {
        self.sessions
            .get(&id)
            .map(|session| Arc::clone(&session.alive))
    }

    pub fn subscribe(
        &mut self,
        id: u64,
        connection_id: i64,
        case_id: Option<i64>,
        sender: SyncSender<PsqlSessionEvent>,
    ) -> Result<(), String> {
        let session = self.sessions.get_mut(&id).ok_or(crate::i18n::text(
            "PostgreSQL-сессия не найдена",
            "PostgreSQL session not found",
        ))?;
        if session.info.connection_id != connection_id || session.info.case_id != case_id {
            return Err(crate::i18n::text(
                "Подписка не соответствует подключению и Case",
                "Subscription does not match the connection and case",
            )
            .into());
        }
        if !session.running() {
            return Err(crate::i18n::text(
                "Процесс psql уже завершён",
                "psql process has already exited",
            )
            .into());
        }
        session
            .relay
            .lock()
            .map_err(|_| crate::i18n::text("PTY relay недоступен", "PTY relay is unavailable"))?
            .subscribe(id, sender);
        Ok(())
    }

    pub fn active_for(
        &mut self,
        connection_id: i64,
        case_id: Option<i64>,
    ) -> Option<PsqlSessionInfo> {
        self.cleanup_finished();

        self.sessions
            .values()
            .find(|session| {
                session.info.connection_id == connection_id && session.info.case_id == case_id
            })
            .map(|session| session.info.clone())
    }

    pub fn list(&mut self) -> Vec<PsqlSessionInfo> {
        self.cleanup_finished();

        let mut result = self
            .sessions
            .values()
            .map(|session| session.info.clone())
            .collect::<Vec<_>>();

        result.sort_by_key(|session| session.id);

        result
    }

    pub fn write(
        &mut self,
        id: u64,
        expected_connection_id: i64,
        expected_case_id: Option<i64>,
        data: &str,
    ) -> Result<(), String> {
        if data.len() > 512 * 1024 {
            return Err(crate::i18n::text(
                "Слишком большой блок ввода для psql",
                "Input chunk is too large for psql",
            )
            .into());
        }

        if data.is_empty() {
            return Ok(());
        }

        let session = self.sessions.get_mut(&id).ok_or(crate::i18n::text(
            "PostgreSQL-сессия не найдена",
            "PostgreSQL session not found",
        ))?;

        if session.info.connection_id != expected_connection_id
            || session.info.case_id != expected_case_id
        {
            return Err(crate::i18n::text(
                "Ввод отклонён: отображаемый terminal не соответствует PostgreSQL session",
                "Input rejected: displayed terminal does not match the PostgreSQL session",
            )
            .into());
        }

        if !session.running() {
            session.info.running = false;

            return Err(crate::i18n::text(
                "Процесс psql уже завершён",
                "psql process has already exited",
            )
            .into());
        }

        session
            .writer
            .write_all(data.as_bytes())
            .map_err(|error| format!("Не удалось записать данные в PTY: {error}"))?;

        session
            .writer
            .flush()
            .map_err(|error| format!("Не удалось передать данные psql: {error}"))?;

        Ok(())
    }

    pub fn resize(&mut self, id: u64, cols: u16, rows: u16) -> Result<(), String> {
        validate_terminal_size(cols, rows)?;

        let session = self.sessions.get_mut(&id).ok_or(crate::i18n::text(
            "PostgreSQL-сессия не найдена",
            "PostgreSQL session not found",
        ))?;

        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Не удалось изменить размер PTY: {error}"))
    }

    pub fn has_connection(&mut self, connection_id: i64) -> bool {
        self.cleanup_finished();

        self.sessions
            .values()
            .any(|session| session.info.connection_id == connection_id)
    }

    pub fn has_case(&mut self, case_id: i64) -> bool {
        self.cleanup_finished();

        self.sessions
            .values()
            .any(|session| session.info.case_id == Some(case_id))
    }

    pub fn close(&mut self, id: u64) -> Result<(), String> {
        let session = self.sessions.get_mut(&id).ok_or(crate::i18n::text(
            "PostgreSQL-сессия не найдена",
            "PostgreSQL session not found",
        ))?;
        session.stop()?;
        self.sessions.remove(&id);
        Ok(())
    }

    pub fn close_all(&mut self) {
        self.sessions.clear();
    }

    fn cleanup_finished(&mut self) {
        self.sessions.retain(|_, session| {
            let running = session.running();

            session.info.running = running;

            running
        });
    }
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("Не удалось создать {}: {error}", path.display(),))?;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        format!(
            "Не удалось выставить права 0700 на {}: {error}",
            path.display(),
        )
    })
}

fn ensure_psqlrc(directory: &Path, connection_id: i64) -> Result<PathBuf, String> {
    let path = directory.join(format!("{connection_id}.psqlrc"));

    // O_NOFOLLOW rejects both valid and dangling symlinks. Open the existing
    // file before changing permissions so a path swap cannot redirect chmod.
    // O_NONBLOCK avoids waiting forever if a non-regular file appears here.
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
                .map_err(|error| format!("Не удалось создать {}: {error}", path.display()))?;

            write_default_psqlrc(&mut file)?;
            file.flush()
                .map_err(|error| format!("Не удалось сохранить {}: {error}", path.display()))?;
            file
        }
        Err(error) => {
            return Err(format!("Не удалось открыть {}: {error}", path.display()));
        }
    };

    if !file
        .metadata()
        .map_err(|error| format!("Не удалось проверить {}: {error}", path.display()))?
        .is_file()
    {
        return Err(format!(
            "Файл настроек psql должен быть обычным файлом: {}",
            path.display()
        ));
    }

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| {
            format!(
                "Не удалось выставить права 0600 на {}: {error}",
                path.display()
            )
        })?;

    Ok(path)
}

fn write_default_psqlrc(file: &mut impl Write) -> Result<(), String> {
    writeln!(file, r"\set QUIET 1").map_err(|_| "Не удалось записать psqlrc")?;

    writeln!(file, r"\pset pager off").map_err(|_| "Не удалось записать psqlrc")?;

    writeln!(file, r"\pset null '∅'").map_err(|_| "Не удалось записать psqlrc")?;

    writeln!(file, r"\x auto").map_err(|_| "Не удалось записать psqlrc")?;

    writeln!(file, r"\timing on").map_err(|_| "Не удалось записать psqlrc")?;

    writeln!(
        file,
        r"\set PROMPT1 '%[%033[1;36m%]pgxray[%:PGXRAY_PROFILE:/%/]%x%R%# %[%033[0m%]'"
    )
    .map_err(|_| "Не удалось записать PROMPT1")?;

    writeln!(
        file,
        r"\set PROMPT2 '%[%033[1;33m%]pgxray[%:PGXRAY_PROFILE:/%/]%x%R%# %[%033[0m%]'"
    )
    .map_err(|_| "Не удалось записать PROMPT2")?;

    writeln!(file, r"\set PROMPT3 '>> '").map_err(|_| "Не удалось записать PROMPT3")?;

    writeln!(file, r"\unset QUIET").map_err(|_| "Не удалось записать psqlrc")?;

    Ok(())
}

fn create_session_psqlrc(home_dir: &Path, persistent_psqlrc: &Path) -> Result<PathBuf, String> {
    let path = home_dir.join("pgxray-session.psqlrc");
    // Reopen without following symlinks: the path may have changed since
    // ensure_psqlrc() verified it.
    let mut source = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(persistent_psqlrc)
        .map_err(|error| {
            format!(
                "Не удалось открыть {}: {error}",
                persistent_psqlrc.display()
            )
        })?;
    if !source
        .metadata()
        .map_err(|error| {
            format!(
                "Не удалось проверить {}: {error}",
                persistent_psqlrc.display()
            )
        })?
        .is_file()
    {
        return Err("Конфигурация psql должна быть обычным файлом".into());
    }
    let mut persistent = String::new();
    source.read_to_string(&mut persistent).map_err(|error| {
        format!(
            "Не удалось прочитать {}: {error}",
            persistent_psqlrc.display()
        )
    })?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("Не удалось создать {}: {error}", path.display()))?;

    file.write_all(persistent.as_bytes())
        .map_err(|error| format!("Не удалось записать {}: {error}", path.display()))?;

    if !persistent.ends_with('\n') {
        writeln!(file).map_err(|_| "Не удалось записать psqlrc")?;
    }

    // Preserve the user's psql preferences verbatim. pgXray does not append
    // implicit SET commands; user-managed psqlrc files may still contain SQL.

    file.flush()
        .map_err(|error| format!("Не удалось сохранить {}: {error}", path.display()))?;

    Ok(path)
}

fn detect_editor() -> PathBuf {
    for candidate in ["/usr/bin/nano", "/bin/nano", "/usr/bin/vi", "/bin/vi"] {
        let path = PathBuf::from(candidate);

        if path.is_file() {
            return path;
        }
    }

    PathBuf::from("/usr/bin/nano")
}

fn validate_terminal_size(cols: u16, rows: u16) -> Result<(), String> {
    if cols < 20 || rows < 5 || cols > 1000 || rows > 500 {
        return Err(
            crate::i18n::text("Некорректный размер терминала", "Invalid terminal size").into(),
        );
    }

    Ok(())
}

// Preserve multibyte UTF-8 characters split across PTY reads. Invalid bytes
// are replaced once; incomplete suffixes are retained until the next read.
#[derive(Default)]
struct Utf8StreamDecoder {
    pending: Vec<u8>,
}
impl Utf8StreamDecoder {
    fn feed(&mut self, data: &[u8], eof: bool) -> String {
        self.pending.extend_from_slice(data);
        let mut result = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    result.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    result
                        .push_str(std::str::from_utf8(&self.pending[..valid_up_to]).unwrap_or(""));
                    self.pending.drain(..valid_up_to);
                    if let Some(invalid_len) = error.error_len() {
                        self.pending.drain(..invalid_len);
                        result.push('\u{fffd}');
                    } else if eof {
                        self.pending.clear();
                        result.push('\u{fffd}');
                        break;
                    } else {
                        break;
                    }
                }
            }
        }
        result
    }
}

fn start_reader(
    session_id: u64,
    mut reader: Box<dyn Read + Send>,
    relay: Arc<Mutex<OutputRelay>>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut decoder = Utf8StreamDecoder::default();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let text = decoder.feed(&buffer[..read], false);
                    if !text.is_empty() {
                        if let Ok(mut relay) = relay.lock() {
                            relay.add_output(session_id, text);
                        }
                    }
                }
                Err(error) => {
                    if let Ok(mut relay) = relay.lock() {
                        relay.add_output(session_id, format!("\r\n[PTY read error: {error}]\r\n"));
                    }
                    break;
                }
            }
        }
        // A process without a readable PTY must not keep accepting blind SQL
        // input. Terminate it; the process watcher will confirm exit and only
        // then release its quota. An already-exited child ignores this kill.
        let _ = killer.kill();
        let tail = decoder.feed(&[], true);
        if let Ok(mut relay) = relay.lock() {
            if !tail.is_empty() {
                relay.add_output(session_id, tail);
            }
            // EOF is not proof that the child exited; a separate process poll
            // controls terminal permits and the final Exit event.
            relay.send(session_id, PsqlSessionEvent::StreamEnded { session_id });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_is_validated() {
        assert!(validate_terminal_size(120, 40,).is_ok());

        assert!(validate_terminal_size(10, 40,).is_err());
    }

    #[test]
    fn psqlrc_is_persistent() {
        let dir = tempfile::tempdir().unwrap();

        let manager = PsqlSessionManager::new(dir.path()).unwrap();

        let path = ensure_psqlrc(&manager.psqlrc_dir, 42).unwrap();

        assert!(path.exists());

        let first = fs::read_to_string(&path).unwrap();

        fs::write(&path, "\\timing off\n").unwrap();

        let path_again = ensure_psqlrc(&manager.psqlrc_dir, 42).unwrap();

        assert_eq!(fs::read_to_string(path_again,).unwrap(), "\\timing off\n",);

        assert_ne!(first, "\\timing off\n",);
    }

    #[test]
    fn session_psqlrc_preserves_user_config_without_injected_sql() {
        let dir = tempfile::tempdir().unwrap();
        let persistent = dir.path().join("persistent.psqlrc");
        let home = dir.path().join("home");

        fs::create_dir(&home).unwrap();
        fs::write(&persistent, "\\timing off\n").unwrap();

        let session_psqlrc = create_session_psqlrc(&home, &persistent).unwrap();
        let content = fs::read_to_string(session_psqlrc).unwrap();

        assert_eq!(content, "\\timing off\n");
    }

    #[test]
    fn session_psqlrc_rejects_symlink_source() {
        let dir = tempfile::tempdir().unwrap();
        let persistent = dir.path().join("persistent.psqlrc");
        let target = dir.path().join("target.psqlrc");
        let home = dir.path().join("home");

        fs::create_dir(&home).unwrap();
        fs::write(&target, "\\timing off\n").unwrap();
        std::os::unix::fs::symlink(&target, &persistent).unwrap();

        assert!(create_session_psqlrc(&home, &persistent).is_err());
        assert!(!home.join("pgxray-session.psqlrc").exists());
    }

    #[test]
    fn utf8_stream_handles_split_cyrillic_and_invalid_bytes() {
        let mut decoder = Utf8StreamDecoder::default();
        let word = "Привет".as_bytes();
        assert_eq!(decoder.feed(&word[..1], false), "");
        assert_eq!(decoder.feed(&word[1..], false), "Привет");
        assert_eq!(decoder.feed(&[0xff], false), "�");
        assert_eq!(decoder.feed(&[0xd0], true), "�");
    }

    #[test]
    fn replay_is_bounded_and_does_not_split_unicode() {
        let mut relay = OutputRelay::default();
        relay.add_output(1, "ж".repeat(REPLAY_LIMIT).to_string());
        assert!(relay.tail.len() <= REPLAY_LIMIT);
        assert!(relay.tail.starts_with('ж'));
    }

    #[test]
    fn output_queue_reports_lost_chunks_after_recovery() {
        let mut relay = OutputRelay::default();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        relay.subscribe(3, sender);
        relay.add_output(3, "first".into());
        relay.add_output(3, "skipped".into());
        assert!(relay.skipped);
        assert!(matches!(
            receiver.try_recv(),
            Ok(PsqlSessionEvent::Output { .. })
        ));
        relay.add_output(3, "next".into());
        assert!(matches!(
            receiver.try_recv(),
            Ok(PsqlSessionEvent::OutputGap { session_id: 3 })
        ));
    }

    #[test]
    fn history_storage_is_not_created_persistently() {
        let dir = tempfile::tempdir().unwrap();
        PsqlSessionManager::new(dir.path()).unwrap();
        assert!(!dir.path().join("history").exists());
    }

    #[test]
    fn psqlrc_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let manager = PsqlSessionManager::new(dir.path()).unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "\\set foo bar\n").unwrap();
        let initial_permissions = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        std::os::unix::fs::symlink(&target, manager.psqlrc_dir.join("51.psqlrc")).unwrap();
        assert!(ensure_psqlrc(&manager.psqlrc_dir, 51).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "\\set foo bar\n");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            initial_permissions
        );
    }

    #[test]
    fn psqlrc_rejects_dangling_symlink_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let manager = PsqlSessionManager::new(dir.path()).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("missing-target"),
            manager.psqlrc_dir.join("52.psqlrc"),
        )
        .unwrap();
        assert!(ensure_psqlrc(&manager.psqlrc_dir, 52).is_err());

        fs::create_dir(manager.psqlrc_dir.join("53.psqlrc")).unwrap();
        assert!(ensure_psqlrc(&manager.psqlrc_dir, 53).is_err());
    }

    #[test]
    fn default_psqlrc_only_contains_display_preferences() {
        let mut config = Vec::new();
        write_default_psqlrc(&mut config).unwrap();
        let text = String::from_utf8(config).unwrap();
        assert!(text.contains("\\timing on"));
        assert!(text.contains("PROMPT1"));
        assert!(!text.contains("SET "));
        assert!(!text.contains("set_config("));
    }
}
