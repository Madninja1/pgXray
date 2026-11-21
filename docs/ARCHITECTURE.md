# Architecture overview

Status: source-grounded description of **pgXray 0.5.5**, from the 2026-11-20 source archive. It describes the current Rust/Tauri/Vue implementation, not the older HTML-only README narrative. Architecture is subject to change before the first public binary release.

## Runtime flow

```text
Local user
   │
   ▼
Vue application (ui-vue/src/, Vite)
   │ Tauri IPC / commands
   ▼
src-tauri/src/main.rs  ─── application state, auth gates, operation limits
   │
   ├── pgxray-core::auth, connections, workspace, history ──► local SQLite/files
   │
   ├── pgxray-core::{activity, locks, pgss, xid, autovacuum,
   │                 io_wal, index_health, postgres_settings, plan}
   │                                 │
   │                                 ▼
   │                         PostgreSQL (internal client)
   │
   └── pgxray-core::sessions ──► /usr/bin/psql (interactive PTY)
```

This is a **simplified data-flow map**, not an exhaustive Tauri command graph. For precise call signatures inspect `src-tauri/src/main.rs` and the modules in `crates/core/src/`.

## Code layout

| Path | Responsibility |
| --- | --- |
| `Cargo.toml` | Rust workspace with `crates/core` and `src-tauri` |
| `crates/core/src/lib.rs` | Public core module wiring |
| `crates/core/src/internal_postgres.rs` | Internal PostgreSQL connection and execution policies |
| `crates/core/src/postgres.rs` | Connection validation and query execution policy helpers |
| `crates/core/src/connections.rs` | Saved connection metadata and separate credential files |
| `crates/core/src/auth.rs` | Local pgXray login, Argon2 password verification, SQLite |
| `crates/core/src/history/` | Saved diagnostic snapshots and history |
| `crates/core/src/workspace.rs` | Investigation cases, artifacts, and workspace data |
| `crates/core/src/privacy/` | Anonymization/de-anonymization and privacy mappings |
| `crates/core/src/sessions.rs` | Terminal psql sessions through a PTY |
| `src-tauri/src/main.rs` | Tauri bootstrap, command handlers, state/lifecycle coordination |
| `ui-vue/src/` | Primary Vue UI |
| `ui-vue/vite.config.ts` | Vite build; copies selected locally vendored PEV2 assets |
| `ui/vendor/pev2`, `ui/vendor/xterm` | Local third-party frontend assets with their own licenses |
| `ui/` | Legacy/static UI assets and shared vendored resources; not the primary app entrypoint |

The current desktop configuration is `src-tauri/Tauri.toml` (`frontendDist = "../ui-vue/build"`, `devUrl = "http://127.0.0.1:5173"`). `Tauri.vue-preview.toml` is a development-only overlay, with a distinct application identifier.

## Local state

Tauri startup obtains its application-data directory from `app.path().app_data_dir()` and initializes:

- `auth.sqlite3` — local application users;
- `connections.sqlite3` + `credentials/` — connection metadata and **separate plaintext password files** with restricted POSIX permissions;
- `diagnostics.sqlite3` — diagnostic history;
- `workspace.sqlite3` + `workspaces/` — investigation metadata and workspace files;
- `logs/`, `exports/`, `path-settings/` — application logging, export and directory preferences.

The workspace, export and log roots may be configurable; do not assume all artifacts stay under the initial application-data directory. Consult [Privacy & security](PRIVACY.md) before sharing workspaces or backups.

## Operational boundaries

- Regular internal diagnostic queries have execution limits defined by `QueryExecutionPolicy` and the internal PostgreSQL registry.
- PostgreSQL roles and server settings determine visibility of system statistics and extension functionality.
- Some explicitly invoked operations **can modify a PostgreSQL server**, including user-entered psql commands and policy application paths. Do **not** describe the entire application as technically read-only.
- The app uses local SQLite and filesystem writes for history, investigation cases and settings.
- No hosted telemetry service is identified in the inspected startup/configuration paths; this is **not** a comprehensive assertion that every module has been audited for networking.

## Limitations

This document does not assert that Linux installers, PostgreSQL 14–18 compatibility or long-running performance have been validated. See [Support](POSTGRESQL_SUPPORT.md) and [Release checklist](RELEASE_CHECKLIST.md).
