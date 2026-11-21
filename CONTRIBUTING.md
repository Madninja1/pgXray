# Contributing to pgXray

Thank you for considering a contribution. The project is in development preview; compatibility and APIs may change.

## Before you start

Open an issue describing the bug or proposed change before starting a large change. Do not include real database dumps, credentials, customer SQL, internal hostnames or proprietary source code in issues or pull requests.

Use a personal test PostgreSQL instance with synthetic data. Never reproduce incidents against production without explicit authorization.

## Development

Install the prerequisites in [README.md](README.md), then run:

```bash
npm --prefix ui-vue ci
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix ui-vue run build
npm --prefix ui-vue test
```

Tests requiring PostgreSQL must use a dedicated disposable instance. PRs should include motivation, the minimal code change, tests and a note on possible impact to PostgreSQL load, local storage and privacy.

## License of contributions

By submitting a contribution, you agree it may be distributed under the project's Apache-2.0 license, and certify that you have the necessary rights to submit it. Do not copy incompatible code or confidential employer material. Preserve existing third-party copyright and license notices.

For sensitive vulnerabilities follow [SECURITY.md](SECURITY.md), not public issues.
