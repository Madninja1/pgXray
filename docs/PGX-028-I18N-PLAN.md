# PGX-028 — Internationalization (EN/RU)

Status: inventory completed against the 2026-11-20 source archive; implementation not started.

## Scope and architecture

The current UI is Vue 3 (`ui-vue/src`), while `ui/` contains older HTML/JavaScript sources that are **excluded from the default localization inventory**. Rust code in `crates/` and `src-tauri/src/` contains user-visible errors and explanations. Do not assume that every phrase in the legacy UI is used by the current application; confirm its call path before migrating it.

The npm manifest in the examined archive does not contain a translation library. The app already has local-setting patterns (for example, its theme controller), but locale persistence has not yet been implemented.

## Automated inventory

Run from the repository root (active Vue UI and Rust only):

```bash
python3 scripts/i18n-audit.py --output /tmp/pgxray-i18n-audit
```

To investigate legacy sources separately, use `--include-legacy`. Keep `ui/` in the repository until call sites and asset references are verified; remove only demonstrably unused files as a separate cleanup task.

Review `summary.json` for scope and `candidates.csv` for file/line/text candidates. The scanner only identifies Cyrillic-looking strings. It is **heuristic**: it can include non-UI strings, expressions, SQL examples, internal errors, duplicate/overlapping candidates, and strings inside comments; it can also miss English-only user-facing strings. This is not an automatically generated translation catalog.

The exported CSV/JSON can contain application strings and potentially example data. **Review before uploading or publishing**. Prefer keeping full inventories local instead of committing them as generated data.

## Proposed migration (incremental)

1. Classify candidates in `ui-vue/src` into user-visible text, technical identifiers, comments, and non-translatable content. Do not translate `ui/` as part of PGX-028; audit references before any later deletion.
2. Add a single Vue-level translation service (evaluate `vue-i18n` against existing conventions) and dictionaries `en` / `ru`. Do not introduce a second state manager solely for locale.
3. Default locale for **new installations**: `en`. Preserve `ru` for existing users during migration where a language preference is absent; otherwise an update would unexpectedly change their UI. Define the migration condition from actual persisted settings before coding.
4. Localize shared navigation, dialogs, connection management and settings first, then each diagnostic view and reusable formatter. Preserve SQL identifiers and backend field names.
5. Replace user-facing backend messages with stable error identifiers/parameters where feasible. Do not localize log contents or raw PostgreSQL server messages in transit.
6. Add coverage checks: both locale dictionaries have matching keys; no untranslated keys; dynamic parameters work; pluralization, dates and numbers follow locale; language choice persists after restart; tests for representative screens.
7. Check exact packaging and on-disk migration in the installed Tauri app, then publish a **new release version** with updated binaries rather than silently swapping an existing tagged release asset.

## Release policy

`v0.5.5` must remain labeled as Russian-interface pre-release until a translated binary is actually built and tested. GitHub release notes can be edited, but editing notes or uploading files does not translate binaries already downloaded. For substantial EN/RU changes, use a new version/tag (`v0.5.6` or a later agreed version) and a new release.
