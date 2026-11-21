import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
const component = readFileSync(new URL("../src/features/sessions/SessionsView.vue", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.vue", import.meta.url), "utf8");
test("terminal uses real xterm.js and FitAddon from existing vendored assets", () => {
  assert.match(component, /ui\/vendor\/xterm\/xterm\.js\?url&no-inline/);
  assert.match(component, /ui\/vendor\/xterm\/addon-fit\.js\?url&no-inline/);
  assert.match(component, /new Channel<PsqlSessionEvent>\(\)/);
});
test("terminal stays mounted when navigating and sends channel only for new sessions", () => {
  assert.match(app, /<SessionsView v-if="bridge" ref="sessionsView" v-show=/);
  assert.match(app, /:open-session="openConnectionSession"/);
  assert.match(component, /!sessionIsAttached\(state, id\)/);
});
test("SQL terminal clearly warns about PostgreSQL and OS privileges and requires explicit close", () => {
  assert.match(component, /t\('sessions\.rolePrivileges'\)/);
  assert.match(component, /t\('sessions\.securityWarning'\)/);
  assert.match(component, /import \{ confirmAction \} from "\.\.\/\.\.\/core\/confirm-action\.js"/);
  assert.match(component, /if \(session\.running && !await confirmAction\(t\('sessions\.closeConfirm'/);
  assert.doesNotMatch(component, /window\.confirm/);
  assert.match(component, /t\('sessions\.unattached'\)/);
});
test("terminated session is distinct from lost output subscription and explains rollback", () => {
  assert.match(component, /v-if="!latestSession\.running"/);
  assert.match(component, /v-else-if="!attached"/);
  assert.match(component, /state\.exitOutput\[latestSession\.id\]/);
  assert.match(component, /t\('sessions\.closeConfirm'/);
});


test("single terminal mode does not expose pooler compatibility switch or stale close notice", () => {
  assert.doesNotMatch(component, /poolerStartupRejected|retryWithoutStartupOptions/);
  assert.doesNotMatch(component, /Запустить в режиме совместимости/);
  assert.doesNotMatch(component, /status\.value|Терминал закрыт\./);
  assert.match(component, /t\('sessions\.roleInfo'\)/);
});


test("terminal UI uses role privileges without obsolete pooler compatibility mode", () => {
  assert.doesNotMatch(component, /poolerStartupRejected|retryWithoutStartupOptions/);
  assert.doesNotMatch(component, /Запустить в режиме совместимости/);
  assert.match(component, /t\('sessions\.rolePrivileges'\)/);
  assert.match(component, /t\('sessions\.securityWarning'\)/);
});
