import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import {
  THEME_KEY, DEFAULT_THEME, THEME_CHOICES,
  createThemeController, createThemeState, normalizeThemeName, terminalThemeFromProperties,
} from "../dist/features/theme/theme-controller.js";

function mock(initial = {}) {
  const data = new Map(Object.entries(initial));
  const styles = new Map();
  const events = [];
  const root = {
    dataset: {},
    style: { removeProperty(key) { styles.delete(key); } },
  };
  const storage = {
    getItem: key => data.get(key) ?? null,
    setItem(key, value) { data.set(key, value); },
  };
  const state = createThemeState();
  const manager = createThemeController(state, { root, storage, notifyChanged: theme => events.push(theme) });
  return { data, styles, events, root, storage, state, manager };
}

test("exactly four named themes are offered, without Custom", () => {
  assert.deepEqual(THEME_CHOICES.map(t => t.key), ["dba", "ubuntu", "light", "dark"]);
  assert.equal(DEFAULT_THEME, "dba");
});

test("old theme names migrate to a known preset without affecting the old UI", () => {
  const old = {
    "graphite-teal": "dba", "forest-mint": "dba", "indigo-cyan": "dark",
    "light": "light", "custom": "dba",
  };
  for (const [previous, next] of Object.entries(old)) assert.equal(normalizeThemeName(previous), next);
  for (const unknown of [null, undefined, "", "nope", 123, {}, "LIGHT"]) {
    assert.equal(normalizeThemeName(unknown), DEFAULT_THEME);
  }
});

test("legacy custom colors remain stored, but their inline styles are cleared", () => {
  const originalSource = readFileSync(new URL("../../ui/js/theme.js", import.meta.url), "utf8");
  const originalFields = vm.runInNewContext(originalSource + "\nCUSTOM_THEME_FIELDS", {
    localStorage: { getItem: () => null },
    document: { documentElement: { dataset: {}, style: { removeProperty() {} } } },
    syncTerminalTheme() {}, window: { dispatchEvent() {} }, CustomEvent: class {},
  });
  const m = mock({ [THEME_KEY]: "custom", "pgxray.customTheme": '{"--bg":"#123456"}' });
  for (const [name] of originalFields) m.styles.set(name, "#000000");
  m.styles.set("--unrelated", "keep");
  m.manager.initialize();
  assert.equal(m.state.selected, "dba");
  assert.equal(m.root.dataset.theme, "dba");
  for (const [name] of originalFields) assert.equal(m.styles.has(name), false, name);
  assert.equal(m.styles.get("--unrelated"), "keep");
  assert.equal(m.data.get("pgxray.customTheme"), '{"--bg":"#123456"}');
  assert.equal(m.data.get(THEME_KEY), "custom", "startup should not rewrite legacy settings");
});

test("selecting every preset updates the document and saved setting", () => {
  const m = mock(); m.manager.initialize();
  for (const { key } of THEME_CHOICES) {
    m.manager.selectTheme(key);
    assert.equal(m.state.selected, key);
    assert.equal(m.root.dataset.theme, key);
    assert.equal(m.data.get(THEME_KEY), key);
  }
  assert.deepEqual(m.events, [DEFAULT_THEME, ...THEME_CHOICES.map(t => t.key)]);
});

test("initializing a saved modern preset does not rewrite storage", () => {
  const m = mock({ [THEME_KEY]: "ubuntu" }); m.manager.initialize();
  assert.equal(m.root.dataset.theme, "ubuntu");
  assert.equal(m.data.get(THEME_KEY), "ubuntu");
  assert.equal(m.data.size, 1);
});

test("read failure falls back to DBA without preventing startup", () => {
  const m = mock();
  m.storage.getItem = () => { throw Error("blocked"); };
  m.manager.initialize();
  assert.equal(m.state.selected, "dba");
  assert.equal(m.root.dataset.theme, "dba");
});

test("unavailable storage still applies a theme and reports that it is not persisted", () => {
  const m = mock(); m.manager.initialize();
  m.storage.setItem = () => { throw Error("blocked"); };
  m.manager.selectTheme("dark");
  assert.equal(m.root.dataset.theme, "dark");
  assert.match(m.state.storageError, /не сохраниться/);
});

test("the four presets define complete main backgrounds, text, and focus indicators", () => {
  const css = readFileSync(new URL("../src/features/theme/presets.css", import.meta.url), "utf8");
  assert.match(css, /data-theme="dba"/);
  assert.match(css, /data-theme="ubuntu"/);
  assert.match(css, /data-theme="dark"/);
  assert.match(css, /data-theme="light"/);
  assert.match(css, /--focus-ring/);
  for (const name of ["ubuntu", "dark"]) {
    const cssBlock = css.match(new RegExp(`:root\\[data-theme="${name}"\\]\\s*\\{([^}]+)\\}`))?.[1] ?? "";
    for (const property of ["--bg", "--surface", "--text", "--accent", "--accent-contrast", "--focus-ring"]) {
      assert.ok(cssBlock.includes(property + ":"), `${name} is missing ${property}`);
    }
  }
});

test("terminal theme retains the established CSS contract", () => {
  const styles = new Map([["--terminal-bg", "  #123456  "], ["--terminal-text", "#ffffff"]]);
  assert.deepEqual(terminalThemeFromProperties(key => styles.get(key) ?? ""), {
    background: "#123456", foreground: "#ffffff",
    cursor: "#70d7c0", selectionBackground: "#314157",
  });
});
