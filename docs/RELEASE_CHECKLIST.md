# PGX-027 — Public open-source release checklist

Status: **IN PROGRESS**. No release or public-readiness claim is implied by this document.

## Ready to review

- [x] Current snapshot identified as version `0.5.5` (Cargo and Tauri configuration).
- [x] Public documentation baseline prepared (README, CONTRIBUTING, SECURITY, CHANGELOG).
- [x] Project Apache-2.0 license text and third-party notice inventory drafted; publication rights still require confirmation.
- [ ] CI / GitHub Actions: explicitly deferred by maintainer; not required for the current documentation/licensing work. Reconsider before public release.

## Must finish before publishing sources

- [ ] Confirm rights to distribute original and contributed code, including any employment-related IP obligations.
- [ ] Audit **all Git history**, current files, examples, screenshots and test fixtures for secrets and proprietary information. The ZIP does not include `.git`.
- [ ] Verify all direct/transitive Rust and npm dependencies, vendored assets, and the licenses/notices actually shipped in release packages. See `docs/THIRD_PARTY_NOTICES.md`.
- [ ] Complete and review Installation Guide, Architecture Overview, Privacy & Security Documentation, and a PostgreSQL support matrix based on *executed* compatibility tests.
- [ ] Validate clean-machine developer setup and run documented manual Rust / Vue checks. CI remains deferred pending maintainer decision.
- [ ] Review PostgreSQL permissions, operational safety, sensitive local storage and exported evidence.

## Before binary beta (not part of this patch)

- [ ] Stabilize long-running UI (PGX-002), reconnection, resource cleanup, error handling and local storage lifecycle.
- [ ] Test real DBA investigation workflows in non-production environments (dogfooding).
- [ ] Enable Tauri bundling in a separate reviewed change; build and install `.deb` / AppImage on clean supported systems.
- [ ] Prepare synthetic PostgreSQL sample datasets, screenshots and short demos without confidential data.
- [ ] Validate release artifacts, checksums, OS prerequisites and PostgreSQL compatibility matrix.

Do not mark PGX-027 complete until all release acceptance criteria selected for the intended release are met. Advanced features from the backlog are **not** prerequisites by default.
