# Installation and local development

Status: **source/development instructions for pgXray 0.5.5**, based on the project tree archived on 2026-11-20. No downloadable installer has been validated. Target discussed here: Linux/Ubuntu; other operating systems are **not verified**.

## Prerequisites

- Rust toolchain (`rustc`, `cargo`), and Tauri CLI **v2** (`cargo tauri`).
- Node.js matching `ui-vue/package.json`: `^22.18.0 || >=24.12.0`, and npm.
- PostgreSQL command-line client with `/usr/bin/psql` present (interactive terminal uses this **absolute path**).
- Linux build dependencies for Tauri 2 / WebKitGTK (package availability depends on your Ubuntu release). Typical Debian/Ubuntu development packages include `build-essential`, `pkg-config`, `libwebkit2gtk-4.1-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`, and `file`.

Use the official [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/) for your distribution, and [rustup](https://rustup.rs/) to install Rust. These prerequisites are guidance, **not a tested compatibility guarantee** for every Ubuntu release.

Example Ubuntu setup (review packages before installing):

```bash
sudo apt update
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
  file postgresql-client
cargo install tauri-cli --version '^2' --locked
```

Check the local versions:

```bash
rustc --version
cargo --version
cargo tauri --version
node --version
npm --version
/usr/bin/psql --version
```

## Build the frontend and run in development

From the **repository root**:

```bash
npm --prefix ui-vue ci
npm --prefix ui-vue run build
cargo test -p pgxray-core
cargo tauri dev
```

`src-tauri/Tauri.toml` declares `ui-vue` as the primary UI, runs `npm run dev` for the Tauri dev process, and uses `../ui-vue/build` as the frontend distribution. `scripts/run.sh` also executes `cargo tauri dev`; it does not install npm dependencies for you.

The Vite development server binds to **127.0.0.1:5173**. Do not expose it to a public interface. Opening an HTML file in the browser is not equivalent to using Tauri commands.

Additional local checks (not proof of a production-ready release):

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix ui-vue run typecheck
npm --prefix ui-vue test
npm --prefix ui-vue run build
```

Run ignored integration tests only against an **explicitly provisioned, disposable PostgreSQL** instance. The tests use `PGXRAY_TEST_HOST`, `PGXRAY_TEST_PORT`, `PGXRAY_TEST_DB`, `PGXRAY_TEST_USER`, `PGXRAY_TEST_PASSWORD`, and `PGXRAY_TEST_SSLMODE` (see `crates/core/src/internal_postgres.rs`). Never point integration tests at production.

## First start and PostgreSQL access

On first launch, the local authentication database may require registration of the initial pgXray user. This **local application account** is distinct from the PostgreSQL database account. Configure a PostgreSQL connection explicitly in the application. Available connection TLS modes are `verify-full`, `require`, and `disable`; see [Privacy & security](PRIVACY.md).

Some diagnostics use internal read-only queries; the interactive psql terminal is a separate capability and may execute user-entered SQL. Use a least-privilege PostgreSQL role for routine diagnostics.

## Installation packages are not available yet

`src-tauri/Tauri.toml` currently has `[bundle] active = false` and a development identifier `dev.pgxray.desktop`. Therefore this guide **does not** claim that `.deb` or AppImage artifacts have been built or installed successfully. Do not change packaging configuration just to make the documentation appear complete. Packaging, installation, desktop integration and upgrade tests remain on the release checklist.

## Related documents

- [Architecture](ARCHITECTURE.md)
- [PostgreSQL support](POSTGRESQL_SUPPORT.md)
- [Privacy & security](PRIVACY.md)
- [Release checklist](RELEASE_CHECKLIST.md)
