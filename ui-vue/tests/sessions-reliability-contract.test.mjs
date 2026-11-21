import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";

const rust = readFileSync(new URL("../../crates/core/src/sessions.rs", import.meta.url), "utf8");
const tauri = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf8");
const ui = readFileSync(new URL("../src/features/sessions/controller.ts", import.meta.url), "utf8");

// Source-level contract tests are useful regression guards, NOT a substitute
// for cargo test or a real PTY / PostgreSQL integration test.
test("unregistered psql child has a process cleanup guard", () => {
  assert.match(rust, /struct StartingChild/);
  assert.match(rust, /impl Drop for StartingChild/);
  assert.match(rust, /child\.kill\(\)/);
  assert.match(rust, /child\.wait\(\)/);
});
test("PTY EOF is not interpreted as process exit or permit release", () => {
  assert.match(rust, /StreamEnded \{ session_id/);
  assert.match(tauri, /alive\.load\(Ordering::Acquire\)/);
  assert.match(tauri, /operation_limits\.release_terminal\(session_id\)/);
});
test("sessions have bounded in-memory replay and resubscribe command", () => {
  assert.match(rust, /REPLAY_LIMIT: usize = 64 \* 1024/);
  assert.match(rust, /SyncSender<PsqlSessionEvent>/);
  assert.match(tauri, /async fn subscribe_psql_session/);
  assert.match(ui, /"subscribe_psql_session"/);
});
test("SQL history is temporary, with no per-connection persistent history", () => {
  assert.match(rust, /home_dir\.path\(\)\.join\("psql-history"\)/);
  assert.doesNotMatch(rust, /ensure_history_file\(/);
});
test("new errors never claim that psql guaranteed read-only", () => {
  assert.doesNotMatch(rust, /command\.env\("PGOPTIONS"/);
  assert.match(tauri, /Нельзя менять сервер или пароль/);
});
