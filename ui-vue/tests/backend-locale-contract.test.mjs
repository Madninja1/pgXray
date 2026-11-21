import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = path => readFileSync(new URL(path, import.meta.url), "utf8");

test("Rust locale command is registered and frontend synchronizes before mount", () => {
  const main = source("../../src-tauri/src/main.rs");
  const frontend = source("../src/main.ts");
  assert.match(main, /fn set_backend_locale\(locale: String\)/);
  assert.match(main, /generate_handler!\[\s*set_backend_locale,/);
  assert.match(frontend, /await invoke\("set_backend_locale", \{ locale: locale\.value \}\)/);
  assert.match(frontend, /watch\(locale, value =>/);
  assert.ok(frontend.indexOf('await invoke("set_backend_locale"') < frontend.indexOf('createApp(App).mount'));
});

test("Rust translation helper is restricted to supported languages", () => {
  const core = source("../../crates/core/src/i18n.rs");
  assert.match(core, /"en" => EN/);
  assert.match(core, /"ru" => RU/);
  assert.match(core, /Unsupported locale/);
  assert.match(core, /Ordering::Acquire/);
});

test("Rust locale migration only translates explanatory text, not SQL or machine keys", () => {
  const activity = source("../../crates/core/src/activity.rs");
  const pgss = source("../../crates/core/src/pgss/collector.rs");
  const index = source("../../crates/core/src/index_health.rs");
  assert.match(activity, /crate::i18n::text\(\s*"Некорректный PostgreSQL backend PID"/);
  assert.match(pgss, /crate::i18n::text\(\s*"PGSS Explorer: limit/);
  assert.match(index, /crate::i18n::text\(\s*"idx_scan=0/);
  assert.match(activity, /"pg_stat_activity"|ACTIVITY_SQL/);
});
