use crate::{
    internal_postgres::InternalPostgresRegistry,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::{Deserialize, Serialize};

const SETTINGS_SQL: &str = r#"
SELECT json_build_object(
    'measured_at', clock_timestamp()::text,
    'database', current_database(),
    'database_oid', (
        SELECT oid::bigint
        FROM pg_database
        WHERE datname = current_database()
    ),
    'username', current_user,
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'postmaster_start_ms', (
        extract(epoch FROM pg_postmaster_start_time()) * 1000
    )::bigint,
    'settings', COALESCE(
        (
            SELECT json_agg(
                json_build_object(
                    'name', s.name,
                    'setting', s.setting,
                    'unit', s.unit,
                    'category', s.category,
                    'vartype', s.vartype,
                    'context', s.context,
                    'source', s.source,
                    'sourcefile', s.sourcefile,
                    'sourceline', s.sourceline,
                    'pending_restart', s.pending_restart,
                    'boot_val', s.boot_val,
                    'reset_val', s.reset_val,
                    'min_val', s.min_val,
                    'max_val', s.max_val,
                    'enumvals', to_json(s.enumvals)
                )
                ORDER BY s.category, s.name
            )
            FROM pg_settings AS s
        ),
        '[]'::json
    )
)::text;
"#;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PgSetting {
    pub name: String,
    pub setting: String,
    pub unit: Option<String>,
    pub category: String,
    pub vartype: String,
    pub context: String,
    pub source: String,
    pub sourcefile: Option<String>,
    pub sourceline: Option<i64>,
    pub pending_restart: bool,
    pub boot_val: Option<String>,
    pub reset_val: Option<String>,
    pub min_val: Option<String>,
    pub max_val: Option<String>,
    pub enumvals: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PgSettingsSnapshot {
    pub measured_at: String,
    pub database: String,
    pub database_oid: i64,
    pub username: String,
    pub server_version: String,
    pub server_version_num: i32,
    pub postmaster_start_ms: i64,
    pub settings: Vec<PgSetting>,
}

pub async fn collect(
    registry: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
) -> Result<PgSettingsSnapshot, String> {
    // Reuse the audited read-only registry so pg_settings participates in the
    // same task-local operation trace as PGSS/XID. SQL/policy remain unchanged.
    let output = registry
        .run_readonly_query(
            connection_id,
            input,
            SETTINGS_SQL,
            QueryExecutionPolicy::standard_diagnostic(),
        )
        .await?;

    parse_settings_output(&output)
}

fn parse_settings_output(output: &str) -> Result<PgSettingsSnapshot, String> {
    serde_json::from_str(output.trim()).map_err(|error| {
        if crate::i18n::is_ru() {
            format!("Не удалось разобрать pg_settings snapshot: {error}")
        } else {
            format!("Failed to parse pg_settings snapshot: {error}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pg_settings_snapshot() {
        let snapshot = parse_settings_output(
            r#"{
                "measured_at":"2026-09-15 12:00:00+00",
                "database":"app",
                "database_oid":16384,
                "username":"pgxray",
                "server_version":"17.6",
                "server_version_num":170006,
                "postmaster_start_ms":1700000000000,
                "settings":[{
                    "name":"work_mem",
                    "setting":"64",
                    "unit":"MB",
                    "category":"Resource Usage / Memory",
                    "vartype":"integer",
                    "context":"user",
                    "source":"configuration file",
                    "sourcefile":"/etc/postgresql/postgresql.conf",
                    "sourceline":42,
                    "pending_restart":false,
                    "boot_val":"4",
                    "reset_val":"64",
                    "min_val":"64",
                    "max_val":"2147483647",
                    "enumvals":null
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(snapshot.database, "app");
        assert_eq!(snapshot.settings.len(), 1);
        assert_eq!(snapshot.settings[0].name, "work_mem");
        assert_eq!(
            snapshot.settings[0].sourcefile.as_deref(),
            Some("/etc/postgresql/postgresql.conf")
        );
    }

    #[test]
    fn parses_nullable_string_defaults() {
        let snapshot = parse_settings_output(
            r#"{
                "measured_at":"2026-09-15 12:00:00+00",
                "database":"app",
                "database_oid":16384,
                "username":"pgxray",
                "server_version":"17.6",
                "server_version_num":170006,
                "postmaster_start_ms":1700000000000,
                "settings":[{
                    "name":"extension.string_setting",
                    "setting":"",
                    "unit":null,
                    "category":"Customized Options",
                    "vartype":"string",
                    "context":"user",
                    "source":"default",
                    "sourcefile":null,
                    "sourceline":null,
                    "pending_restart":false,
                    "boot_val":null,
                    "reset_val":null,
                    "min_val":null,
                    "max_val":null,
                    "enumvals":null
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(snapshot.settings[0].boot_val, None);
        assert_eq!(snapshot.settings[0].reset_val, None);
    }

    // Use a dedicated unprivileged role in disposable PostgreSQL when possible.
    // This test only reads pg_settings and requires no schema changes.
    #[tokio::test]
    #[ignore = "requires PGXRAY_TEST_* credentials for a disposable PostgreSQL"]
    async fn integration_pg_settings_unprivileged_visibility() {
        let input = ConnectionInput {
            host: std::env::var("PGXRAY_TEST_HOST").expect("PGXRAY_TEST_HOST"),
            port: std::env::var("PGXRAY_TEST_PORT")
                .unwrap_or_else(|_| "5432".into())
                .parse()
                .unwrap(),
            database: std::env::var("PGXRAY_TEST_DB").unwrap_or_else(|_| "postgres".into()),
            username: std::env::var("PGXRAY_TEST_USER").expect("PGXRAY_TEST_USER"),
            password: std::env::var("PGXRAY_TEST_PASSWORD").unwrap_or_default(),
            sslmode: std::env::var("PGXRAY_TEST_SSLMODE").unwrap_or_else(|_| "disable".into()),
        };
        let registry = InternalPostgresRegistry::new();
        let snapshot = collect(&registry, 1, &input)
            .await
            .expect("pg_settings snapshot");
        assert_eq!(snapshot.database, input.database);
        assert!(!snapshot.settings.is_empty());
        assert!(snapshot.settings.iter().any(|s| s.name == "work_mem"));
        // Non-superusers may receive NULL sourcefile/sourceline: it is not an error.
        assert!(snapshot.settings.iter().all(|s| !s.name.is_empty()));
    }
}
