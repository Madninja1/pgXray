use crate::{
    internal_postgres::InternalPostgresRegistry,
    postgres::{ConnectionInput, QueryExecutionPolicy},
};
use serde::{Deserialize, Serialize};

pub const MIN_SUPPORTED_SERVER_VERSION_NUM: i32 = 140_000;
pub const MAX_VERIFIED_SERVER_VERSION_NUM: i32 = 189_999;
pub const SUPPORTED_POSTGRES_RANGE: &str = "14–18";

const READINESS_SQL: &str = r#"
SELECT json_build_object(
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'database', current_database(),
    'username', current_user,
    'is_in_recovery', pg_is_in_recovery(),
    'session_read_only', current_setting('default_transaction_read_only')::boolean,
    'track_counts', current_setting('track_counts')::boolean,
    'track_io_timing', current_setting('track_io_timing')::boolean,
    'track_wal_io_timing', CASE
        WHEN current_setting('track_wal_io_timing', true) IS NULL THEN NULL
        ELSE current_setting('track_wal_io_timing', true)::boolean
    END,
    'pg_stat_statements_installed', EXISTS (
        SELECT 1
        FROM pg_extension
        WHERE extname = 'pg_stat_statements'
    ),
    'pg_stat_statements_preloaded', EXISTS (
        SELECT 1
        FROM unnest(string_to_array(COALESCE(current_setting('shared_preload_libraries', true), ''), ',')) AS item(value)
        WHERE btrim(value) = 'pg_stat_statements'
    ),
    'can_read_all_stats', (
        SELECT rolsuper OR pg_has_role(current_user, 'pg_read_all_stats', 'MEMBER')
        FROM pg_roles
        WHERE rolname = current_user
    ),
    'can_signal_backend', (
        SELECT rolsuper OR pg_has_role(current_user, 'pg_signal_backend', 'MEMBER')
        FROM pg_roles
        WHERE rolname = current_user
    ),
    'has_pg_stat_wal', to_regclass('pg_catalog.pg_stat_wal') IS NOT NULL,
    'has_pg_stat_io', to_regclass('pg_catalog.pg_stat_io') IS NOT NULL,
    'has_progress_vacuum', to_regclass('pg_catalog.pg_stat_progress_vacuum') IS NOT NULL,
    'has_progress_analyze', to_regclass('pg_catalog.pg_stat_progress_analyze') IS NOT NULL
)::text;
"#;

#[derive(Clone, Debug, Serialize)]
pub struct FeatureReadiness {
    pub key: String,
    pub label: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PostgresReadiness {
    pub server_version: String,
    pub server_version_num: i32,
    pub database: String,
    pub username: String,
    pub support_status: String,
    pub support_detail: String,
    pub is_in_recovery: bool,
    pub session_read_only: bool,
    pub features: Vec<FeatureReadiness>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ReadinessProbe {
    server_version: String,
    server_version_num: i32,
    database: String,
    username: String,
    is_in_recovery: bool,
    session_read_only: bool,
    track_counts: bool,
    track_io_timing: bool,
    track_wal_io_timing: Option<bool>,
    pg_stat_statements_installed: bool,
    pg_stat_statements_preloaded: bool,
    can_read_all_stats: bool,
    can_signal_backend: bool,
    has_pg_stat_wal: bool,
    has_pg_stat_io: bool,
    has_progress_vacuum: bool,
    has_progress_analyze: bool,
}

pub async fn collect_postgres_readiness(
    registry: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
) -> Result<PostgresReadiness, String> {
    let output = registry
        .run_readonly_query(
            connection_id,
            input,
            READINESS_SQL,
            QueryExecutionPolicy::readiness(),
        )
        .await?;

    parse_probe(&output).map(build_readiness)
}

fn parse_probe(output: &str) -> Result<ReadinessProbe, String> {
    serde_json::from_str(output).map_err(|error| {
        if crate::i18n::is_ru() {
            format!("PostgreSQL readiness probe вернул неожиданный JSON: {error}")
        } else {
            format!("PostgreSQL readiness probe returned unexpected JSON: {error}")
        }
    })
}

fn build_readiness(probe: ReadinessProbe) -> PostgresReadiness {
    let (support_status, support_detail) = support_for_version(probe.server_version_num);
    let mut warnings = Vec::new();

    if support_status == "unsupported" {
        warnings.push(if crate::i18n::is_ru() {
            format!("PostgreSQL {} находится вне поддерживаемого диапазона pgXray 0.5.0 ({SUPPORTED_POSTGRES_RANGE}).", probe.server_version)
        } else {
            format!("PostgreSQL {} is outside the supported pgXray 0.5.0 range ({SUPPORTED_POSTGRES_RANGE}).", probe.server_version)
        });
    } else if support_status == "unverified" {
        warnings.push(if crate::i18n::is_ru() {
            format!("PostgreSQL {} новее проверенного диапазона pgXray 0.5.0; диагностические collectors работают в best-effort режиме.", probe.server_version)
        } else {
            format!("PostgreSQL {} is newer than the verified pgXray 0.5.0 range; diagnostic collectors operate on a best-effort basis.", probe.server_version)
        });
    }

    if !probe.can_read_all_stats {
        warnings.push(
            crate::i18n::text("Текущая роль не superuser и не член pg_read_all_stats: часть cross-session SQL/query text может быть скрыта PostgreSQL.", "Current role is neither superuser nor a member of pg_read_all_stats; PostgreSQL may hide some cross-session SQL and query text.")
                .into(),
        );
    }

    if !probe.track_counts {
        warnings.push(
            crate::i18n::text("track_counts=off: table/index/autovacuum workload evidence неполно или недостоверно.", "track_counts=off: table, index, and autovacuum workload evidence is incomplete or unreliable.")
                .into(),
        );
    }

    if !probe.session_read_only {
        warnings.push(
            crate::i18n::text("Internal diagnostic session не подтверждает default_transaction_read_only=on. Read-only invariant требует проверки.", "Internal diagnostic session does not confirm default_transaction_read_only=on; verify the read-only invariant.")
                .into(),
        );
    }

    let mut features = Vec::new();
    features.push(feature(
        "live_activity",
        "Activity / Locks",
        if probe.can_read_all_stats {
            "ready"
        } else {
            "limited"
        },
        if probe.can_read_all_stats {
            crate::i18n::text(
                "Cross-session statistics и query context доступны роли.",
                "Cross-session statistics and query context are available to the role.",
            )
        } else {
            crate::i18n::text(
                "PostgreSQL может скрывать query text и часть cross-session контекста.",
                "PostgreSQL may hide query text and parts of the cross-session context.",
            )
        },
    ));

    let (pgss_status, pgss_detail) = if !probe.pg_stat_statements_installed {
        (
            "unavailable",
            crate::i18n::text(
                "Extension pg_stat_statements не установлено в текущей database.",
                "The pg_stat_statements extension is not installed in the current database.",
            ),
        )
    } else if !probe.pg_stat_statements_preloaded {
        (
            "limited",
            crate::i18n::text("Extension установлено, но pg_stat_statements не найден в shared_preload_libraries.", "The extension is installed, but pg_stat_statements was not found in shared_preload_libraries."),
        )
    } else {
        (
            "ready",
            crate::i18n::text(
                "pg_stat_statements установлено и найдено в shared_preload_libraries.",
                "pg_stat_statements is installed and present in shared_preload_libraries.",
            ),
        )
    };
    features.push(feature(
        "pgss",
        "pg_stat_statements",
        pgss_status,
        pgss_detail,
    ));

    features.push(feature(
        "table_health",
        "Table / Index health",
        if probe.track_counts {
            "ready"
        } else {
            "limited"
        },
        if probe.track_counts {
            crate::i18n::text(
                "track_counts=on; workload counters доступны.",
                "track_counts=on; workload counters are available.",
            )
        } else {
            crate::i18n::text(
                "track_counts=off; usage counters не дают надёжного evidence.",
                "track_counts=off; usage counters do not provide reliable evidence.",
            )
        },
    ));

    features.push(feature(
        "io_wal",
        "I/O & WAL",
        if probe.has_pg_stat_wal {
            "ready"
        } else {
            "limited"
        },
        if probe.has_pg_stat_io {
            crate::i18n::text(
                "pg_stat_wal и pg_stat_io доступны; расширенный I/O context включён.",
                "pg_stat_wal and pg_stat_io are available; extended I/O context is enabled.",
            )
        } else if probe.has_pg_stat_wal {
            crate::i18n::text(
                "pg_stat_wal доступен; pg_stat_io отсутствует на этой версии PostgreSQL.",
                "pg_stat_wal is available, but pg_stat_io is absent on this PostgreSQL version.",
            )
        } else {
            crate::i18n::text(
                "pg_stat_wal недоступен; WAL evidence будет ограничено.",
                "pg_stat_wal is unavailable; WAL evidence will be limited.",
            )
        },
    ));

    features.push(feature(
        "autovacuum",
        "Autovacuum / XID",
        if probe.has_progress_vacuum && probe.has_progress_analyze {
            if probe.track_counts {
                "ready"
            } else {
                "limited"
            }
        } else {
            "limited"
        },
        if probe.has_progress_vacuum && probe.has_progress_analyze {
            if probe.track_counts {
                crate::i18n::text("Progress views и table statistics доступны.", "Progress views and table statistics are available.")
            } else {
                crate::i18n::text("Progress views доступны, но track_counts=off ограничивает table workload evidence.", "Progress views are available, but track_counts=off limits table workload evidence.")
            }
        } else {
            crate::i18n::text("Одна или несколько progress views отсутствуют; live maintenance context ограничен.", "One or more progress views are missing; live maintenance context is limited.")
        },
    ));

    features.push(feature(
        "io_timing",
        "I/O timing evidence",
        if probe.track_io_timing {
            "ready"
        } else {
            "limited"
        },
        if probe.track_io_timing {
            if probe.track_wal_io_timing.unwrap_or(false) {
                crate::i18n::text(
                    "track_io_timing=on и track_wal_io_timing=on.",
                    "track_io_timing=on and track_wal_io_timing=on.",
                )
            } else {
                crate::i18n::text(
                    "track_io_timing=on; WAL I/O timing выключен или недоступен на сервере.",
                    "track_io_timing=on; WAL I/O timing is disabled or unavailable on the server.",
                )
            }
        } else {
            crate::i18n::text(
                "track_io_timing=off; timing-based I/O evidence ограничено.",
                "track_io_timing=off; timing-based I/O evidence is limited.",
            )
        },
    ));

    features.push(feature(
        "backend_control",
        "Cancel / Terminate",
        if probe.can_signal_backend { "ready" } else { "unavailable" },
        if probe.can_signal_backend {
            crate::i18n::text("Роль может сигнализировать другим PostgreSQL backend; pgXray всё равно требует явного действия пользователя.", "Role can signal other PostgreSQL backends; pgXray still requires explicit user action.")
        } else {
            crate::i18n::text("Роль не имеет pg_signal_backend/superuser privileges для cross-session Cancel/Terminate.", "Role lacks pg_signal_backend or superuser privileges for cross-session Cancel/Terminate.")
        },
    ));

    PostgresReadiness {
        server_version: probe.server_version,
        server_version_num: probe.server_version_num,
        database: probe.database,
        username: probe.username,
        support_status: support_status.into(),
        support_detail,
        is_in_recovery: probe.is_in_recovery,
        session_read_only: probe.session_read_only,
        features,
        warnings,
    }
}

fn support_for_version(server_version_num: i32) -> (&'static str, String) {
    if server_version_num < MIN_SUPPORTED_SERVER_VERSION_NUM {
        return (
            "unsupported",
            if crate::i18n::is_ru() {
                format!("pgXray 0.5.0 поддерживает PostgreSQL {SUPPORTED_POSTGRES_RANGE}; эта версия старее диапазона.")
            } else {
                format!("pgXray 0.5.0 supports PostgreSQL {SUPPORTED_POSTGRES_RANGE}; this server version is too old.")
            },
        );
    }

    if server_version_num > MAX_VERIFIED_SERVER_VERSION_NUM {
        return (
            "unverified",
            crate::i18n::text("Сервер новее PostgreSQL 18. pgXray не блокирует диагностику, но совместимость этой версии ещё не зафиксирована.", "The server is newer than PostgreSQL 18. pgXray allows diagnostics, but compatibility with this version has not been verified.")
                .into(),
        );
    }

    (
        "supported",
        if crate::i18n::is_ru() {
            format!("Версия входит в зафиксированный диапазон PostgreSQL {SUPPORTED_POSTGRES_RANGE} для pgXray 0.5.0.")
        } else {
            format!("This version is within the verified PostgreSQL {SUPPORTED_POSTGRES_RANGE} range for pgXray 0.5.0.")
        },
    )
}

fn feature(key: &str, label: &str, status: &str, detail: &str) -> FeatureReadiness {
    FeatureReadiness {
        key: key.into(),
        label: label.into(),
        status: status.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(server_version_num: i32) -> ReadinessProbe {
        ReadinessProbe {
            server_version: format!("{}", server_version_num / 10_000),
            server_version_num,
            database: "app".into(),
            username: "reader".into(),
            is_in_recovery: false,
            session_read_only: true,
            track_counts: true,
            track_io_timing: true,
            track_wal_io_timing: Some(false),
            pg_stat_statements_installed: true,
            pg_stat_statements_preloaded: true,
            can_read_all_stats: true,
            can_signal_backend: false,
            has_pg_stat_wal: true,
            has_pg_stat_io: server_version_num >= 160_000,
            has_progress_vacuum: true,
            has_progress_analyze: true,
        }
    }

    #[test]
    fn supported_range_is_14_through_18() {
        assert_eq!(support_for_version(139_999).0, "unsupported");
        assert_eq!(support_for_version(140_000).0, "supported");
        assert_eq!(support_for_version(180_999).0, "supported");
        assert_eq!(support_for_version(190_000).0, "unverified");
    }

    #[test]
    fn missing_stats_permission_is_reported_as_limitation() {
        let mut value = probe(170_000);
        value.can_read_all_stats = false;

        let readiness = build_readiness(value);
        let activity = readiness
            .features
            .iter()
            .find(|feature| feature.key == "live_activity")
            .unwrap();

        assert_eq!(activity.status, "limited");
        assert!(readiness
            .warnings
            .iter()
            .any(|warning| warning.contains("pg_read_all_stats")));
    }

    #[test]
    fn pgss_requires_extension_and_preload_for_ready_status() {
        let mut value = probe(170_000);
        value.pg_stat_statements_preloaded = false;

        let readiness = build_readiness(value);
        let pgss = readiness
            .features
            .iter()
            .find(|feature| feature.key == "pgss")
            .unwrap();

        assert_eq!(pgss.status, "limited");
    }

    #[test]
    fn readiness_probe_json_parses_single_column_output() {
        let value = r#"{"server_version":"18.6","server_version_num":180006,"database":"app","username":"reader","is_in_recovery":false,"session_read_only":true,"track_counts":true,"track_io_timing":false,"track_wal_io_timing":true,"pg_stat_statements_installed":true,"pg_stat_statements_preloaded":true,"can_read_all_stats":true,"can_signal_backend":false,"has_pg_stat_wal":true,"has_pg_stat_io":true,"has_progress_vacuum":true,"has_progress_analyze":true}"#;
        let parsed = parse_probe(value).unwrap();

        assert_eq!(parsed.server_version_num, 180_006);
        assert_eq!(parsed.database, "app");
        assert!(parsed.session_read_only);
    }
}
