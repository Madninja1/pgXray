import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const script = readFileSync(new URL("../../ui/pev2/bridge.js", import.meta.url), "utf8");

function harness() {
  const handlers = {};
  const root = {
    children: [],
    replaceChildren() { this.children = []; },
    append(child) { this.children.push(child); },
  };
  const mounts = [];
  const sent = [];
  const parent = { postMessage(message) { sent.push(message); } };
  const window = {
    parent,
    Vue: { h(component, props, children) { return {component,props,children}; }, createApp(config) {
      return {
        component() {},
        config: {},
        mount() { mounts.push(config); config.render(); },
        unmount() {},
      };
    } },
    pev2: { Plan: {} },
    addEventListener(name, callback) { handlers[name] = callback; },
  };
  const document = {
    getElementById() { return root; },
    createElement() {
      return { children: [], append(child) { this.children.push(child); } };
    },
  };
  runInNewContext(script, { window, document });
  return { handlers, root, mounts, parent, sent };
}

test("PEV2 ignores messages from non-parent windows", () => {
  const app = harness();
  app.handlers.message({ source: {}, data: { type: "pgxray-pev2-plan", plan: "{}" } });
  assert.equal(app.mounts.length, 0);
  assert.equal(app.root.children.length, 0);
});

test("PEV2 rejects multiple top-level EXPLAIN results", () => {
  const app = harness();
  app.handlers.message({ source: app.parent, data: { type: "pgxray-pev2-plan", plan: '[{"Plan":{"Node Type":"Seq Scan"}},{"Plan":{"Node Type":"Index Scan"}}]' } });
  assert.equal(app.mounts.length, 0);
  assert.match(app.root.children[0].children[0].textContent, /ровно один/);
});

test("PEV2 mounts a valid plan only from its parent", () => {
  const app = harness();
  app.handlers.message({ source: app.parent, data: { type: "pgxray-pev2-plan", plan: '[{"Plan":{"Node Type":"Seq Scan"}}]' } });
  assert.equal(app.mounts.length, 1);
});

test("PEV2 renders without a runtime template (strict CSP)", () => {
  const app = harness();
  app.handlers.message({ source: app.parent, data: { type: "pgxray-pev2-plan", plan: '[{"Plan":{"Node Type":"Seq Scan"}}]' } });
  assert.equal(app.mounts.length, 1);
  assert.equal(typeof app.mounts[0].render, "function");
  assert.equal(app.mounts[0].template, undefined);
  assert.ok(app.sent.some(item => item.type === "pgxray-pev2-mounted"));
});
