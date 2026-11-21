# Third-party notices

pgXray is intended to be distributed under Apache License 2.0 for original project code, subject to confirmation that the publisher owns the necessary rights. **Bundled third-party works are licensed separately**. The project license does not relicense those works.

## Assets physically bundled in this repository

| Component | Bundled location | Preserved license | Source of version evidence |
| --- | --- | --- | --- |
| PEV2 | `ui/vendor/pev2/pev2.umd.js`, `pev2.css` | `ui/vendor/pev2/PEV2-LICENSE` (Dalibo permissive notice) | `ui/vendor/pev2/VERSIONS.txt` reports `1.25.0` |
| Vue (PEV2 asset) | `ui/vendor/pev2/vue.global.prod.js` | `ui/vendor/pev2/VUE-LICENSE` (MIT) | `VERSIONS.txt` reports `3.5.28` |
| Bootstrap (PEV2 asset) | `ui/vendor/pev2/bootstrap.min.css` | `ui/vendor/pev2/BOOTSTRAP-LICENSE` (MIT) | `VERSIONS.txt` reports `5.3.8` |
| xterm.js | `ui/vendor/xterm/xterm.js`, `xterm.css` | `ui/vendor/xterm/LICENSE.xterm` (MIT) | Exact artifact version not verified |
| xterm fit addon | `ui/vendor/xterm/addon-fit.js` | `ui/vendor/xterm/LICENSE.addon-fit` (MIT) | Exact artifact version not verified |

The original license files must remain with the bundled assets in source distributions and must be included or appropriately reproduced with redistributed binaries, as required by each license. This inventory does not assert that the bundled artifacts are byte-for-byte identical to upstream releases.

## Installed and build-time dependencies

- Rust: `Cargo.lock` contains **586** package entries in the reviewed archive dated 2026-11-20. Cargo lockfiles do not record the package license for these entries. Dependencies can include native or statically linked components; license review must use package manifests, upstream license text, and the actual release build.
- JavaScript: `ui-vue/package-lock.json` contains **77** package entries (including the root package) in the reviewed archive. This lockfile does not provide license information for those entries. Production and development dependencies should be distinguished when recording distribution obligations.
- Tauri/WebView and system libraries have their own licensing and redistribution conditions, which depend on the final packaging and platform.

## Locked dependency license findings (local inventory, 2026-11-20)

The following information is transcribed from the local `pgxray-license-inventory.md` report. License expressions are **package metadata**, not a verification of all upstream license texts or a determination of which files are contained in the final distributed binary.

- The only Rust package entries reported as `UNKNOWN` are the two pgXray workspace packages, `pgxray-core` and `pgxray-desktop` (0.5.5). Their Cargo manifests are marked `license = "Apache-2.0"` by the accompanying change, subject to the publisher's right to license original project code.
- Rust crates declaring `MPL-2.0`: `cssparser` 0.36.0, `cssparser-macros` 0.6.1, `dtoa-short` 0.3.5, `option-ext` 0.2.0, `selectors` 0.36.1. If covered source or executables are distributed, review MPL-2.0 source-availability and notice obligations for the actual package contents.
- npm entries declaring `MPL-2.0` in installed metadata: `lightningcss` 1.33.0, `lightningcss-linux-x64-gnu` 1.33.0, `lightningcss-linux-x64-musl` 1.33.0. These packages may be build tools; **do not assume** they are present in the shipped frontend or Tauri binary. Audit the final artifact.
- Other noteworthy reported license identifiers include `Unicode-3.0` (ICU and associated Unicode packages), `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `BSL-1.0`, `Zlib`, `0BSD`, `Unlicense`, `CC0-1.0`, and combined expressions. Select permitted alternatives for `OR` expressions only after confirming the corresponding license terms; retain required copyright notices.
- `r-efi` 5.3.0 and 6.0.0 declare `MIT OR Apache-2.0 OR LGPL-2.1-or-later`. The metadata offers permissive alternatives; this alone does not require choosing LGPL.
- 23 of 76 npm dependency entries were reported as `NOT INSTALLED` on the audit machine; primarily platform-specific optional packages. That status does **not** establish their licenses or whether they are used on other target platforms.
- Bundled/native components brought in by `rusqlite` with its `bundled` feature, OpenSSL and `pg_query` require review of the actual binary distribution (including compiled C/C++ components), independently of Rust crate metadata.

### Release notice requirements

Before providing public binaries, determine the precise set of files and libraries **actually redistributed**, collect the corresponding copyright/license texts, retain existing notices, and make any required MPL-covered source available. For components with additional license or patent notices, preserve those notices in the manner required by their licenses. Keep a versioned third-party notice bundle next to or inside each published installer, as appropriate for that format.

Do not treat this file as a blanket certification that every dependency is cleared for distribution. Re-run the inventory and examine new or changed dependencies after lockfile updates.

## Publication and redistribution gate (not yet completed)

1. Verify rights to publish original pgXray code and all committed third-party or copied snippets.
2. Verify exact versions and the corresponding upstream licenses of PEV2/xterm artifacts, including any notices embedded in generated bundles.
3. Generate and review a direct **and transitive** dependency license report for the locked Rust and npm versions; investigate unknown, dual, copyleft, and non-standard licenses individually.
4. Confirm distribution obligations for the exact `.deb`, AppImage, or other release artifacts, and ship required license texts/notices with them.
5. Repeat this audit when dependency lockfiles or bundled assets change.

**Current status: preliminary inventory only.** No claim is made that every third-party dependency is cleared for release.
