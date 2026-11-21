# pgXray

**PostgreSQL Investigation & Change Intelligence Toolkit** — a local desktop application for investigating PostgreSQL health and performance using Rust, Tauri 2 and Vue 3.

> **Development preview (v0.5.7).** Not validated for production use. Do not connect to critical databases before reviewing permissions and operational risks.

## What it is for

- Inspect connections, sessions and activity when investigating a slowdown.
- Explore PostgreSQL statistics, locks, autovacuum, indexes and transaction-ID health.
- Collect and compare local diagnostic snapshots to investigate what changed.
- Review evidence and investigation history instead of relying on one live query.

Available tools and the exact information returned depend on server version, permissions and enabled extensions. The interface is not a guarantee that all operations are read-only; review prompts and SQL before using features against production.

## Local development (Linux)

Requires Rust/Cargo, a compatible Node.js version (`^22.18.0 || >=24.12.0`), npm, PostgreSQL client tools where used, and the [Tauri 2 Linux prerequisites](https://v2.tauri.app/start/prerequisites/). This project has been developed for Ubuntu; installation on other distributions and PostgreSQL versions has not been fully verified.

```bash
npm --prefix ui-vue ci
cargo install tauri-cli --version '^2' --locked
cargo tauri dev
```

`src-tauri/Tauri.toml` starts Vite through `npm run dev` and builds the Vue frontend through `npm run build`. The Vue application is the primary UI; the separate `ui/` directory includes legacy assets and vendored components.

## Checks

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix ui-vue ci
npm --prefix ui-vue run typecheck
npm --prefix ui-vue run build
npm --prefix ui-vue test
```

Some integration tests require an explicitly configured PostgreSQL test instance. Never run tests using production credentials.

## Release status

A publicly distributable installer is **not yet available**: `[bundle] active = false` in `src-tauri/Tauri.toml`. Compatibility, long-session stability, security review, and packaging are still release gates; see [release checklist](docs/RELEASE_CHECKLIST.md).

## Privacy and security

pgXray operates as a local desktop application with PostgreSQL connections and local diagnostic storage. Queries, evidence and exported diagnostics may contain sensitive information. Review all artifacts before sharing. See [security policy](SECURITY.md). A complete field-by-field data handling audit is a release prerequisite, not a claim made by this README.

## Contributing and licensing

See [CONTRIBUTING.md](CONTRIBUTING.md), [CHANGELOG.md](CHANGELOG.md), and [third-party notices](docs/THIRD_PARTY_NOTICES.md).

The first-party project files are intended for distribution under [Apache License 2.0](LICENSE), **subject to verification of the contributor's authority to license them before publication**. Third-party components retain their own licenses; see the notices and license files.
