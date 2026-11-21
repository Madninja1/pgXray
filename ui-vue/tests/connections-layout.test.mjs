import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const view = readFileSync(new URL("../src/features/connections/ConnectionsView.vue", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.vue", import.meta.url), "utf8");

test("connection list refresh has an accessible icon and a progress state", () => {
  assert.match(view, /:aria-label="t\('connections\.refresh'\)"/);
  assert.match(view, /:aria-busy="refreshingList \|\| state\.loading \|\| projectState\.loading"/);
  assert.match(view, /class="refresh-glyph"/);
  assert.match(view, /spinning: refreshingList \|\| state\.loading \|\| projectState\.loading/);
  assert.match(view, /Promise\.all\(\[controller\.refresh\(\), projectController\.refresh\(\)\]\)/);
  assert.match(view, /Math\.max\(0, 650 - \(Date\.now\(\) - startedAt\)\)/);
  assert.match(view, /if \(refreshingList\.value \|\| state\.loading \|\| projectState\.loading\) return/);
  assert.match(view, /clearRefreshIndicator\(\);\s+controller\.reset\(\)/);
  assert.match(view, /onUnmounted\(\(\) => \{[\s\S]*?clearRefreshIndicator\(\)/);
});

test("dangerous actions are in the contextual menu, with confirmation retained", () => {
  assert.match(view, /<details class="connection-menu"/);
  assert.match(view, /:aria-label="t\('connections\.connectionActions', \{ name: connection\.name \}\)"/);
  assert.match(view, /onMenuAction\(\$event, 'edit', connection\.id\)/);
  assert.match(view, /onMenuAction\(\$event, 'delete', connection\.id\)/);
  assert.match(view, /controller\.deleteSaved\(id, \(connection\) => requestConfirmation/);
});

test("main action uses real Sessions integration when it is available", () => {
  assert.match(view, /v-if="props\.openSession" type="button" class="connect-action"/);
  assert.match(view, /v-else type="button" class="connect-action"/);
  assert.match(view, /@click="probeSaved\(connection\.id\)"/);
  assert.match(app, /:open-session="openConnectionSession"/);
  assert.match(app, /<SessionsView v-if="bridge" ref="sessionsView" v-show=/);
  assert.match(app, /await sessionsView\.value\.openConnection\(connectionId, forceNew\)/);
});

test("editor is modal, protects running operations, and has a visible password toggle", () => {
  assert.match(view, /<dialog ref="editorDialog"/);
  assert.match(view, /@cancel="onEditorCancel"/);
  assert.match(view, /if \(!working\.value\) controller\.closeEditor\(\)/);
  assert.match(view, /:type="showDraftPassword \? 'text' : 'password'"/);
  assert.match(view, /:aria-pressed="showDraftPassword"/);
  assert.match(view, /savedPasswordTargetChanged/);
});


test("save errors and active-session restrictions appear inside the connection dialog", () => {
  const start = view.indexOf('<dialog ref="editorDialog"');
  const end = view.indexOf('</dialog>', start);
  const editor = view.slice(start, end);
  assert.match(editor, /editingActiveSessions/);
  assert.match(editor, /changingActiveTarget/);
  assert.match(editor, /ref="editorStatusElement"/);
  assert.match(editor, /:role="state\.editorStatus\.kind === 'error' \? 'alert' : 'status'"/);
  assert.ok(editor.indexOf('ref="editorStatusElement"') < editor.indexOf('class="form-actions"'));
  assert.match(view, /state\.error && !state\.editor/);
});

test("connection probe displays a timestamped status and hides raw response in details", () => {
  assert.match(view, /state\.lastProbeById\[connection\.id\]/);
  assert.match(view, /t\("connections\.checkSuccess"\)/);
  assert.match(view, /t\("connections\.lastCheck", \{ date: formatProbeTimestamp/);
  assert.match(view, /t\('connections\.lastProbeTooltip', \{ date: formatProbeTimestamp/);
  assert.match(view, /class="probe-details"/);
  assert.match(view, /formatProbeTimestamp/);
});


test("last probe badges are in the existing TLS/password metadata row, not a separate panel", () => {
  const start = view.indexOf('<div class="connection-meta">');
  const footer = view.indexOf('<div class="connection-card-footer">', start);
  const metadata = view.slice(start, footer);
  assert.match(metadata, /t\("connections\.noTls"\)/);
  assert.match(metadata, /t\("connections\.passwordSaved"\)/);
  assert.match(metadata, /t\("connections\.checkSuccess"\)/);
  assert.match(metadata, /PostgreSQL \{\{ state\.lastProbeById/);
  assert.match(metadata, /t\("connections\.lastCheck", \{ date: formatProbeTimestamp/);
  assert.doesNotMatch(view, /class="probe-result"/);
  // Historical results now come from SQLite through list_connections.
  assert.doesNotMatch(view, /probeStorage: browserProbeStorage\(\)/);
  assert.match(view, /<details v-if="!state\.probingIds/);
});


test("connection projects are grouped and can be managed without touching profile secrets", () => {
  assert.match(view, /createConnectionProjectsController/);
  assert.match(view, /projectGroups/);
  assert.match(view, /t\("connections\.noProject"\)/);
  assert.match(view, /onProjectMenuAction/);
  assert.match(view, /moveConnection\(\$event, connection\.id\)/);
});
