# PostgreSQL support matrix

Status: **declared compatibility range, not a completed release qualification matrix**, based on `crates/core/src/readiness.rs` in pgXray 0.5.5 (archive dated 2026-11-20).

## Range declared by the current source

The readiness module defines `MIN_SUPPORTED_SERVER_VERSION_NUM = 140000`, `MAX_VERIFIED_SERVER_VERSION_NUM = 189999`, and `SUPPORTED_POSTGRES_RANGE = "14–18"`. `support_for_version()` classifies lower versions as `unsupported` and newer major versions as `unverified`. This is an **application policy implemented in code**, not evidence that every release and feature was successfully tested against each PostgreSQL version.

| PostgreSQL major | Readiness classification in source | End-to-end verified for public release |
| --- | --- | --- |
| 13 and older | `unsupported` | No test evidence supplied |
| 14 | `supported` | Not documented |
| 15 | `supported` | Not documented |
| 16 | `supported` | Not documented |
| 17 | `supported` | Not documented |
| 18 | `supported` | Not documented |
| 19 and newer | `unverified` (best effort) | Not documented |

**Do not treat “supported” as release-certified.** The status in the final column must be replaced only after repeatable integration/dogfooding results have been collected for that version.

## Feature prerequisites and version-specific differences

- `pg_stat_statements`: the readiness probe checks for extension installation in the connected database and its presence in `shared_preload_libraries`. Workload analysis will be unavailable or limited without prerequisites and sufficient privileges.
- Cross-session views: PostgreSQL may hide query text and other details unless the role has sufficient monitoring privileges (`pg_read_all_stats` or equivalent).
- Table/index statistics: `track_counts=off` limits the value of collected evidence.
- I/O/WAL: the readiness probe checks view availability, including `pg_stat_wal` and `pg_stat_io`, rather than assuming every view exists on every major release.
- Maintenance/XID: availability of progress views and relevant server settings affects completeness.
- TLS: connection security is configured independently from server-version support.

See `crates/core/src/readiness.rs` for concrete checks, and [Privacy & security](PRIVACY.md) for access and data considerations.

## Suggested release qualification procedure (not yet performed)

For each major version you intend to claim as verified:

1. Provision a **separate disposable** PostgreSQL instance with a known exact minor version. Record server version, OS, pgXray revision, and test configuration.
2. Run application readiness/probe and basic saved-connection lifecycle.
3. Test Activity / Locks, Table / Index health, XID / Autovacuum, I/O / WAL, and `pg_stat_statements` with the documented prerequisites. Record expected limitations separately from defects.
4. Exercise history/snapshot, export, privacy and investigation workflows with **synthetic** data.
5. Run explicit read-only diagnostics and opt-in operations separately. Do not send mutating SQL to a live production server for qualification.
6. Record pass/fail, feature exclusions and reproducibility evidence, then update this table.

Automated tests do not substitute for installation and prolonged-use tests; `PGX-002` (long-running slowdown) remains a separate release blocker until verified resolved.

See [Installation](INSTALLATION.md), [Architecture](ARCHITECTURE.md), and [Release checklist](RELEASE_CHECKLIST.md).
