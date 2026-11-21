import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

// Cross-layer regression: a Vue invocation alone does not register a Tauri command.
test('PGSS archive loading is implemented and registered in Rust', () => {
  const main = readFileSync(new URL('../../src-tauri/src/main.rs', import.meta.url), 'utf8');
  const history = readFileSync(new URL('../../crates/core/src/history/pgss.rs', import.meta.url), 'utf8');
  assert.match(main, /async fn load_pgss_snapshot\s*\(/);
  const handler = main.slice(main.indexOf('.invoke_handler(tauri::generate_handler!['));
  assert.match(handler, /\bload_pgss_snapshot\s*,/);
  assert.match(history, /pub fn pgss_snapshot\s*\(/);
  assert.match(main, /store\.pgss_snapshot\(connection_id, snapshot_id\)/);
});
