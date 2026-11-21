# Dependency license audit — PGX-027

Scope: `pgxray(20261120-091258).zip`, reviewed 2026-11-20.

This document records evidence, not legal clearance. The project intends to use Apache-2.0 for original code, subject to confirmation of rights. Third-party packages and bundled assets retain their own licenses.

## Verified from the supplied source archive

- `Cargo.lock`: 586 `[[package]]` entries, including the workspace packages; no license fields in the lockfile.
- `ui-vue/package-lock.json`: 77 entries including the root, hence 76 installed-dependency entries; no license fields in this lockfile.
- `ui/vendor/pev2/PEV2-LICENSE` exists (Dalibo notice). The adjacent `VERSIONS.txt` lists PEV2 1.25.0, Vue 3.5.28 and Bootstrap 5.3.8. Their separate license files exist.
- `ui/vendor/xterm/LICENSE.xterm` and `LICENSE.addon-fit` exist. Exact bundled xterm artifact versions remain unverified.
- Core declares `rusqlite` with `bundled`, `openssl`, and `pg_query` in `crates/core/Cargo.toml`; investigate their native/bundled components when reviewing redistributable packages.

## Not yet verified

- License expressions and full license texts for all locked Rust packages, including native transitive components.
- License expressions and required notices for all npm dependencies and emitted frontend assets.
- Which packages and system libraries are included in the final `.deb`/AppImage.
- Full Git history and original-code publishing rights.

## Reproduce the inventory locally (no CI)

Run from the repository root with Python 3.11+, an installed Rust/Cargo toolchain and installed npm dependencies:

```bash
# Use the committed lockfile; installs npm dependencies locally.
npm --prefix ui-vue ci

# Cargo may require previously cached crates to run offline.
# If not already cached, fetch the locked crates once:
cargo fetch --locked

python3 scripts/license-audit.py --output /tmp/pgxray-license-inventory.md
```

The script reads repository assets, `cargo metadata --offline --locked`, and installed `node_modules/**/package.json`. It writes only dependency names, versions, license metadata and presence of license files; it makes no network calls and does not modify project files. `cargo fetch` and `npm ci` are separate preparatory commands that download/install dependencies. Review `/tmp/pgxray-license-inventory.md` locally; it is evidence, not an approval. `UNKNOWN` and `NOT INSTALLED` need investigation.

**Important:** `npm ci` installs dependencies for the host environment; platform-specific optional packages from the lockfile may remain uninstalled. Some packages have `license` in a separate license file rather than `package.json`. Do not automatically treat missing metadata as a license violation or a license grant.

Next: resolve unresolved license entries against their exact package/version and upstream license texts, audit redistribution of native and bundled components, and ensure required third-party notices are shipped with release binaries. CI remains intentionally deferred.
