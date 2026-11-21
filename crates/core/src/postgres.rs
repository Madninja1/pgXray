use serde::Deserialize;
use std::{fs, io::Write, os::unix::fs::PermissionsExt};

const STANDARD_MAX_SQL_BYTES: usize = 256 * 1024;
const HEAVY_MAX_SQL_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_HEAVY_TIMEOUT_MS: u64 = 4 * 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct QueryExecutionPolicy {
    pub(crate) statement_timeout_ms: u64,
    pub(crate) lock_timeout_ms: u64,
    pub(crate) overall_timeout_ms: u64,
    pub(crate) max_sql_bytes: usize,
    pub(crate) max_output_bytes: usize,
    pub(crate) application_name: &'static str,
}

impl QueryExecutionPolicy {
    pub(crate) const fn standard_diagnostic() -> Self {
        Self {
            statement_timeout_ms: 10_000,
            lock_timeout_ms: 3_000,
            overall_timeout_ms: 15_000,
            max_sql_bytes: STANDARD_MAX_SQL_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            application_name: "pgxray-collector",
        }
    }

    pub(crate) const fn probe() -> Self {
        Self {
            statement_timeout_ms: 5_000,
            lock_timeout_ms: 3_000,
            overall_timeout_ms: 12_000,
            max_sql_bytes: STANDARD_MAX_SQL_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            application_name: "pgxray-probe",
        }
    }

    pub(crate) const fn activity_snapshot() -> Self {
        Self {
            statement_timeout_ms: 3_000,
            lock_timeout_ms: 1_000,
            overall_timeout_ms: 5_000,
            max_sql_bytes: STANDARD_MAX_SQL_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            application_name: "pgxray-activity-monitor",
        }
    }

    pub(crate) const fn readiness() -> Self {
        Self {
            statement_timeout_ms: 5_000,
            lock_timeout_ms: 1_000,
            overall_timeout_ms: 7_000,
            max_sql_bytes: STANDARD_MAX_SQL_BYTES,
            max_output_bytes: 256 * 1024,
            application_name: "pgxray-readiness",
        }
    }

    pub(crate) const fn backend_signal() -> Self {
        Self {
            statement_timeout_ms: 5_000,
            lock_timeout_ms: 1_000,
            overall_timeout_ms: 7_000,
            max_sql_bytes: STANDARD_MAX_SQL_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            application_name: "pgxray-backend-signal",
        }
    }

    pub(crate) fn explicit_heavy_query(timeout_ms: u64, application_name: &'static str) -> Self {
        let timeout_ms = timeout_ms.clamp(1_000, MAX_HEAVY_TIMEOUT_MS);

        Self {
            statement_timeout_ms: timeout_ms.saturating_sub(500).max(500),
            lock_timeout_ms: 3_000,
            overall_timeout_ms: timeout_ms.saturating_add(1_000),
            max_sql_bytes: HEAVY_MAX_SQL_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            application_name,
        }
    }
}

#[derive(Deserialize)]
pub struct ConnectionInput {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub sslmode: String,
}

impl ConnectionInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.host.is_empty()
            || self.host.len() > 253
            || !self
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b))
        {
            return Err(crate::i18n::text(
                "Укажите одно DNS-имя или IP-адрес без схемы и порта",
                "Enter a single DNS name or IP address without a scheme or port",
            )
            .into());
        }
        if self.port == 0 {
            return Err(crate::i18n::text(
                "Порт должен быть от 1 до 65535",
                "Port must be between 1 and 65535",
            )
            .into());
        }
        for value in [&self.database, &self.username] {
            if value.is_empty()
                || value.len() > 256
                || value.contains(['\n', '\r', '\0', '='])
                || value.contains("://")
            {
                return Err(crate::i18n::text(
                    "Укажите отдельные имена базы и пользователя, не URI подключения",
                    "Enter separate database and username values, not a connection URI",
                )
                .into());
            }
        }
        if self.password.len() > 1024 || self.password.contains(['\n', '\r', '\0']) {
            return Err(
                crate::i18n::text("Пароль PostgreSQL не должен содержать перевод строки или NUL; максимум 1024 байта", "PostgreSQL password must not contain a newline or NUL and must be at most 1024 bytes")
                    .into(),
            );
        }
        if !["verify-full", "require", "disable"].contains(&self.sslmode.as_str()) {
            return Err(crate::i18n::text("Неизвестный режим TLS", "Unknown TLS mode").into());
        }
        Ok(())
    }
}

fn escape_pgpass_field(value: &str) -> String {
    value.replace('\\', "\\\\").replace(':', "\\:")
}

pub(crate) fn create_passfile(input: &ConnectionInput) -> Result<tempfile::NamedTempFile, String> {
    let mut passfile = tempfile::NamedTempFile::new().map_err(|_| {
        crate::i18n::text(
            "Не удалось создать временный файл пароля",
            "Failed to create temporary password file",
        )
    })?;

    writeln!(
        passfile,
        "{}:{}:{}:{}:{}",
        escape_pgpass_field(&input.host),
        input.port,
        escape_pgpass_field(&input.database),
        escape_pgpass_field(&input.username),
        escape_pgpass_field(&input.password),
    )
    .map_err(|_| {
        crate::i18n::text(
            "Не удалось записать временный пароль",
            "Failed to write temporary password",
        )
    })?;

    passfile.flush().map_err(|_| {
        crate::i18n::text(
            "Не удалось сохранить временный пароль",
            "Failed to persist temporary password",
        )
    })?;

    fs::set_permissions(passfile.path(), fs::Permissions::from_mode(0o600)).map_err(|_| {
        crate::i18n::text(
            "Не удалось выставить права 0600 на временный пароль",
            "Failed to set mode 0600 on temporary password file",
        )
    })?;

    Ok(passfile)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pgpass_escape() {
        assert_eq!(escape_pgpass_field("a:b\\c"), "a\\:b\\\\c");
    }
    #[test]
    fn passfile_is_scoped_to_connection() {
        let input = ConnectionInput {
            host: "2001:db8::1".into(),
            port: 5432,
            database: "app:db".into(),
            username: "user:name".into(),
            password: "p:a\\ss".into(),
            sslmode: "require".into(),
        };

        let passfile = create_passfile(&input).unwrap();
        let content = fs::read_to_string(passfile.path()).unwrap();

        assert_eq!(
            content,
            "2001\\:db8\\:\\:1:5432:app\\:db:user\\:name:p\\:a\\\\ss\n",
        );
    }

    #[test]
    fn standard_diagnostic_policy_keeps_current_limits() {
        let policy = QueryExecutionPolicy::standard_diagnostic();

        assert_eq!(policy.statement_timeout_ms, 10_000);
        assert_eq!(policy.lock_timeout_ms, 3_000);
        assert_eq!(policy.overall_timeout_ms, 15_000);
        assert_eq!(policy.application_name, "pgxray-collector");
    }

    #[test]
    fn probe_policy_keeps_short_connection_test_limits() {
        let policy = QueryExecutionPolicy::probe();

        assert_eq!(policy.statement_timeout_ms, 5_000);
        assert_eq!(policy.overall_timeout_ms, 12_000);
        assert_eq!(policy.application_name, "pgxray-probe");
    }

    #[test]
    fn readiness_policy_is_short_and_bounded() {
        let policy = QueryExecutionPolicy::readiness();

        assert_eq!(policy.statement_timeout_ms, 5_000);
        assert_eq!(policy.overall_timeout_ms, 7_000);
        assert_eq!(policy.max_output_bytes, 256 * 1024);
        assert_eq!(policy.application_name, "pgxray-readiness");
    }

    #[test]
    fn heavy_policy_clamps_timeout_and_leaves_outer_timeout_margin() {
        let low = QueryExecutionPolicy::explicit_heavy_query(100, "pgxray-test");
        assert_eq!(low.statement_timeout_ms, 500);
        assert_eq!(low.overall_timeout_ms, 2_000);

        let high = QueryExecutionPolicy::explicit_heavy_query(24 * 60 * 60 * 1_000, "pgxray-test");
        assert_eq!(high.statement_timeout_ms, 14_399_500);
        assert_eq!(high.overall_timeout_ms, 14_401_000);
    }

    #[test]
    fn rejects_conninfo_and_multiline_password() {
        let mut input = ConnectionInput {
            host: "127.0.0.1".into(),
            port: 5432,
            database: "postgres".into(),
            username: "postgres".into(),
            password: "test".into(),
            sslmode: "disable".into(),
        };
        assert!(input.validate().is_ok());
        input.database = "host=elsewhere".into();
        assert!(input.validate().is_err());
        input.database = "postgres".into();
        input.password = "test\ninjected".into();
        assert!(input.validate().is_err());
    }
}
