import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const root = new URL('../', import.meta.url);
const view = readFileSync(new URL('src/features/xid/XidView.vue', root), 'utf8');
const rust = readFileSync(new URL('../src-tauri/src/main.rs', root), 'utf8');
const invoke = readFileSync(new URL('src/api/invoke.ts', root), 'utf8');

test('XID export awaits an authenticated Tauri save and reports the actual filesystem path', () => {
  assert.match(view, /await props\.invoke<string>\("export_xid_csv"/);
  assert.match(view, /csvExportPath\.value = savedPath/);
  assert.match(view, /csvExportError\.value =/);
  assert.match(view, /tr\('xidUi\.text060'\)/);
  assert.doesNotMatch(view, /URL\.createObjectURL|anchor\.download/);
  assert.match(invoke, /"export_xid_csv"/);
  assert.match(rust, /async fn export_xid_csv\(/);
  assert.match(rust, /require_authenticated\(auth\.inner\(\)\.clone\(\)\)\.await\?/);
  assert.match(rust, /write_xid_csv_no_clobber\(&directory, &filename, &content\)/);
});

test('XID CSV export accepts only known filenames and uses no-clobber private writes', () => {
  assert.match(rust, /fn xid_csv_name_valid\(/);
  assert.match(rust, /pgxray-xid-relations-/);
  assert.match(rust, /pgxray-xid-comparison-/);
  assert.match(rust, /\.create_new\(true\)/);
  assert.match(rust, /\.mode\(0o600\)/);
  assert.match(rust, /MAX_XID_CSV_BYTES/);
});
