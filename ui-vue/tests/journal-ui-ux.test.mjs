import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";

const view = readFileSync(new URL("../src/features/settings/ApplicationLogViewer.vue", import.meta.url), "utf8");
const backend = readFileSync(new URL("../../crates/core/src/app_log.rs", import.meta.url), "utf8");
const icons = readFileSync(new URL("../src/components/UiIcon.vue", import.meta.url), "utf8");

test("investigation uses an accessible native modal rather than a section below long lists", () => {
  assert.match(view, /<dialog ref="investigationDialog" class="journal-investigation" aria-labelledby=/);
  assert.match(view, /dialog\.showModal\(\)/);
  assert.match(view, /@close="explorer\.close\(\)"/);
  assert.match(view, /@click="dismissInvestigationBackdrop"/);
  assert.match(view, /@click="explorer\.open\(row\)"/);
  assert.doesNotMatch(view, /<section v-if="state\.selected" class="journal-investigation"/);
});

test("pagination exposes accessible arrows, not text-only older/newer buttons", () => {
  assert.match(view, /name="chevron-left"/);
  assert.match(view, /name="chevron-right"/);
  assert.match(icons, /"chevron-left": \[/);
  assert.match(icons, /"chevron-right": \[/);
  assert.match(view, /:aria-label="t\('journal\.newer_events'\)"/);
  assert.match(view, /:aria-label="t\('journal\.older_events'\)"/);
});

test("separate logical section and physical component filter, with search suggestions", () => {
  assert.match(view, /<select v-model="section">/);
  for (const id of ["connections", "diagnostics", "pgss", "xid", "autovacuum", "io_wal", "index_health", "pg_settings", "activity", "locks", "postgres", "sessions", "plans", "investigations", "auth", "settings", "application", "other"])
    assert.match(view, new RegExp(`value="${id}"`));
  assert.match(view, /section: section\.value \|\| null/);
  assert.match(view, /<select v-model="component">/);
  assert.match(view, /<datalist id="journal-known-operations">/);
  assert.match(view, /<datalist id="journal-search-hints">/);
  assert.match(view, /journal-search-examples/);
  assert.match(backend, /fn journal_section_matches\(/);
  assert.match(backend, /journal_section_matches\(section, entry\)/);
});
