use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

pub const MAX_TERMINALS_PER_CONNECTION: usize = 3;
pub const MAX_TERMINALS_GLOBAL: usize = 10;

#[derive(Default)]
struct LimitState {
    terminal_total: usize,
    terminals_by_connection: HashMap<i64, usize>,
    diagnostics_running: HashSet<i64>,
}

struct LimitInner {
    state: Mutex<LimitState>,
}

pub struct OperationLimits {
    inner: Arc<LimitInner>,
    terminal_permits: Mutex<HashMap<u64, TerminalPermit>>,
}

pub struct TerminalPermit {
    inner: Arc<LimitInner>,
    connection_id: i64,
}

pub struct DiagnosticPermit {
    inner: Arc<LimitInner>,
    connection_id: i64,
}

impl OperationLimits {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(LimitInner {
                state: Mutex::new(LimitState::default()),
            }),
            terminal_permits: Mutex::new(HashMap::new()),
        }
    }

    pub fn acquire_terminal(&self, connection_id: i64) -> Result<TerminalPermit, String> {
        validate_connection_id(connection_id)?;

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "Лимитер PostgreSQL operations недоступен")?;

        let connection_total = state
            .terminals_by_connection
            .get(&connection_id)
            .copied()
            .unwrap_or_default();

        if state.terminal_total >= MAX_TERMINALS_GLOBAL {
            return Err(format!(
                "Достигнут глобальный лимит PostgreSQL terminal sessions: {MAX_TERMINALS_GLOBAL}. Закройте одну из активных сессий и повторите попытку."
            ));
        }

        if connection_total >= MAX_TERMINALS_PER_CONNECTION {
            return Err(format!(
                "Для connection #{connection_id} уже открыто максимум PostgreSQL terminal sessions: {MAX_TERMINALS_PER_CONNECTION}. Закройте одну из сессий и повторите попытку."
            ));
        }

        state.terminal_total += 1;
        *state
            .terminals_by_connection
            .entry(connection_id)
            .or_default() += 1;

        Ok(TerminalPermit {
            inner: Arc::clone(&self.inner),
            connection_id,
        })
    }

    pub fn acquire_diagnostic(&self, connection_id: i64) -> Result<DiagnosticPermit, String> {
        validate_connection_id(connection_id)?;

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "Лимитер PostgreSQL operations недоступен")?;

        if state.diagnostics_running.contains(&connection_id) {
            return Err(format!(
                "Для connection #{connection_id} уже выполняется PostgreSQL diagnostic operation. pgXray не ставит вторую diagnostics operation в бесконечную очередь; дождитесь завершения текущей операции или перезапустите pgXray, если она зависла."
            ));
        }

        state.diagnostics_running.insert(connection_id);

        Ok(DiagnosticPermit {
            inner: Arc::clone(&self.inner),
            connection_id,
        })
    }

    pub fn attach_terminal(&self, session_id: u64, permit: TerminalPermit) -> Result<(), String> {
        let mut permits = self
            .terminal_permits
            .lock()
            .map_err(|_| "Реестр PostgreSQL terminal permits недоступен")?;

        if permits.contains_key(&session_id) {
            return Err(format!(
                "Terminal permit для session #{session_id} уже зарегистрирован"
            ));
        }

        permits.insert(session_id, permit);

        Ok(())
    }

    pub fn release_terminal(&self, session_id: u64) {
        if let Ok(mut permits) = self.terminal_permits.lock() {
            permits.remove(&session_id);
        }
    }

    pub fn clear_terminals(&self) {
        if let Ok(mut permits) = self.terminal_permits.lock() {
            permits.clear();
        }
    }
}

impl Drop for TerminalPermit {
    fn drop(&mut self) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };

        state.terminal_total = state.terminal_total.saturating_sub(1);

        if let Some(count) = state.terminals_by_connection.get_mut(&self.connection_id) {
            *count = count.saturating_sub(1);

            if *count == 0 {
                state.terminals_by_connection.remove(&self.connection_id);
            }
        }

        drop(state);
    }
}

impl Drop for DiagnosticPermit {
    fn drop(&mut self) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };

        state.diagnostics_running.remove(&self.connection_id);

        drop(state);
    }
}

fn validate_connection_id(connection_id: i64) -> Result<(), String> {
    if connection_id <= 0 {
        return Err("Некорректный connection_id для PostgreSQL operation".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_limit_fails_fast_and_recovers_after_drop() {
        let limits = OperationLimits::new();
        let permit = limits.acquire_diagnostic(42).unwrap();

        assert!(limits.acquire_diagnostic(42).is_err());

        drop(permit);

        assert!(limits.acquire_diagnostic(42).is_ok());
    }

    #[test]
    fn terminal_limit_fails_fast_and_recovers_after_drop() {
        let limits = OperationLimits::new();
        let first = limits.acquire_terminal(7).unwrap();
        let second = limits.acquire_terminal(7).unwrap();
        let third = limits.acquire_terminal(7).unwrap();

        assert!(limits.acquire_terminal(7).is_err());

        drop(first);

        assert!(limits.acquire_terminal(7).is_ok());

        drop(second);
        drop(third);
    }

    #[test]
    fn attached_terminal_permit_is_released_only_explicitly() {
        let limits = OperationLimits::new();
        let active = limits.acquire_terminal(7).unwrap();
        limits.attach_terminal(100, active).unwrap();

        let second = limits.acquire_terminal(7).unwrap();
        let third = limits.acquire_terminal(7).unwrap();
        assert!(limits.acquire_terminal(7).is_err());

        // An attached permit stays reserved until explicit release_terminal().
        // The process watcher is responsible for deciding when to release it.
        limits.release_terminal(100);
        assert_eq!(limits.inner.state.lock().unwrap().terminal_total, 2);

        let replacement = limits.acquire_terminal(7).unwrap();
        assert!(limits.acquire_terminal(7).is_err());

        drop(replacement);
        drop(second);
        drop(third);
        assert_eq!(limits.inner.state.lock().unwrap().terminal_total, 0);
    }

    #[test]
    fn duplicate_terminal_registration_does_not_leak_a_slot() {
        let limits = OperationLimits::new();
        let first = limits.acquire_terminal(9).unwrap();
        limits.attach_terminal(123, first).unwrap();

        let duplicate = limits.acquire_terminal(9).unwrap();
        assert!(limits.attach_terminal(123, duplicate).is_err());
        assert_eq!(limits.inner.state.lock().unwrap().terminal_total, 1);

        limits.release_terminal(123);
        assert_eq!(limits.inner.state.lock().unwrap().terminal_total, 0);
    }
}
