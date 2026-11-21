# Privacy and security

Status: implementation notes for **pgXray 0.5.5**, based on the 2026-11-20 source archive. This is a technical disclosure, not a security certification or a promise that all sensitive fields are redacted in every pathway.

## What pgXray processes

The app can access PostgreSQL connection identifiers, database/user/host information, server statistics, SQL text and query plans, diagnostic snapshots, investigation cases and artifacts. Depending on how you use it, these can contain business-confidential or personal information. **Use only databases and data you are authorized to inspect.**

## Local storage: important distinctions

- **pgXray login passwords:** `crates/core/src/auth.rs` hashes these with Argon2 using an individual random salt; they are not PostgreSQL passwords.
- **Saved PostgreSQL connection passwords:** `crates/core/src/connections.rs` saves these as **plaintext UTF-8** in separate `credentials/<id>.secret` files. It creates the credentials directory with mode `0700` and writes files with mode `0600` on POSIX systems. **These are permission-restricted, not encrypted or stored in an OS keyring.** A local account compromise, privileged backup process or filesystem snapshot may expose them.
- **Connection settings and operational data:** stored in local SQLite databases, including `auth.sqlite3`, `connections.sqlite3`, `diagnostics.sqlite3`, and `workspace.sqlite3`, initialized under the Tauri application-data directory in `src-tauri/src/main.rs`.
- **Workspace and exports:** investigation artifacts, SQL and diagnostic outputs may be written to configurable filesystem paths. Workspace paths and exported files can outlive a session and must be handled as sensitive.
- **Interactive psql:** `crates/core/src/sessions.rs` starts `/usr/bin/psql` in a PTY. Its bounded output replay tail is kept in memory per code comments; user-entered SQL may read or change data according to PostgreSQL permissions. Do not assume that console output and manually exported artifacts are anonymized automatically.
- **Application logs:** the app has a configurable log directory. Treat logs as potentially sensitive and review them before submitting bug reports.

The app sets restricted POSIX permissions on some files and directories at initialization. These permissions are **not equivalent to encryption at rest**, and the user remains responsible for workstation/backup security.

## Privacy transforms

`crates/core/src/privacy/` implements text, SQL, EXPLAIN and result transformations, with privacy mappings and de-anonymization capabilities. Results depend on the selected operation and privacy configuration. A mapping file can contain original values: **protect it like confidential source data**. Never assume a transformed artifact has been fully de-identified until you manually review it.

Before publishing an incident example, manually remove or replace database names, account names, internal domains, IP addresses, business identifiers, SQL literals and result rows, and verify that no residual identifiers remain in filenames or metadata.

## PostgreSQL connection security

The input model currently accepts `verify-full`, `require`, and `disable` TLS modes. In the internal Rust client, `verify-full` loads a trusted CA from `~/.postgresql/root.crt` and enables peer verification; `require` uses encrypted transport **without** peer/hostname verification in that path; `disable` uses plaintext transport. Use `verify-full` for remote instances when configured correctly. `disable` is intended only for explicitly trusted/local lab environments.

Note that interactive psql sessions are a separate execution path and may inherit relevant client behavior; validate actual TLS characteristics when connecting to sensitive servers.

## Least privilege and operational safety

Prefer a dedicated, minimally privileged PostgreSQL role for diagnostics. The app has read-only internal diagnostic commands, but also an **interactive psql terminal** and explicit administrative/policy operations; do not promise universal read-only enforcement. A query launched with `EXPLAIN ANALYZE` may actually execute statements. Perform risky tests only in a disposable environment.

For bug reports, share synthetic minimal reproduction data. Never upload authentic `.sqlite3`, `.secret`, `.pgpass`, logs, private SQL or workspace artifacts. Do not include connection strings, private hostnames or user-identifiable values in screenshots.

## What has not been validated

This document does **not** certify absence of telemetry, leakage or vulnerabilities. A full network/code security review and a release artifact review have not been completed. Before a public release, complete the secret/history audit, verify packaged resource contents and review actual retention/export behavior.

See also [Architecture](ARCHITECTURE.md), [Security policy](../SECURITY.md), and [Release checklist](RELEASE_CHECKLIST.md).
