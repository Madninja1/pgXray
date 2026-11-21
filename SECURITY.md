# Security policy

## Supported status

pgXray 0.5.5 is a development preview. There are no published security-maintenance commitments or supported binary distributions yet.

## Reporting vulnerabilities

**Do not file a public GitHub issue containing exploit details, credentials or private PostgreSQL data.**

After the repository is published, use GitHub's **Report a vulnerability** / private vulnerability reporting feature if it is enabled. If private reporting is not available, contact the repository maintainer through a private channel listed on the published repository profile. Do not send database dumps or passwords unless a secure channel has been agreed.

Acknowledgment and remediation times are not guaranteed while the project is in preview.

## Operational precautions

- Prefer a dedicated least-privilege PostgreSQL account and a disposable test database for initial evaluation.
- Avoid running diagnostic operations requiring elevated privileges against production until their queries and impact are understood.
- Treat snapshot files, local SQLite data, logs, exports and execution plans as potentially sensitive.
- Do not include real connection strings or passwords in bug reports or example configurations.
- Verify downloaded binaries and their source before installing; no verified official binary release exists yet.
