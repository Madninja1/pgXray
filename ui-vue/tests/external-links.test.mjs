import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { installExternalLinkHandler, safeExternalUrl } from "../dist/core/external-links.js";

test("external link parser permits docs but forbids local, credentials and custom schemes", () => {
  assert.equal(safeExternalUrl("https://www.postgresql.org/docs/16/monitoring-stats.html"),
    "https://www.postgresql.org/docs/16/monitoring-stats.html");
  assert.equal(safeExternalUrl("http://www.postgresql.org/"), "http://www.postgresql.org/");
  for (const value of ["javascript:alert(1)", "file:///etc/passwd", "mailto:a@b.com",
    "https://user:secret@example.com/", "ftp://example.org", "", "not url"])
    assert.equal(safeExternalUrl(value), null, value);
});

test("external link bridge is shared by Vue and legacy HTML", () => {
  const main = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
  const legacy = readFileSync(new URL("../../ui/index.html", import.meta.url), "utf8");
  const rust = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf8");
  assert.match(main, /installExternalLinkHandler\(document/);
  assert.match(legacy, /js\/external-links\.js/);
  assert.match(rust, /async fn open_external_url\(/);
  assert.match(rust, /StdCommand::new\("\/usr\/bin\/xdg-open"\)/);
});

test("delegated external click opens once, while internal anchors navigate normally", async () => {
  const oldElement = globalThis.Element;
  class FakeElement {
    constructor(href) { this.href = href; }
    closest(selector) { return selector === "a[href]" ? this : null; }
    getAttribute(name) { return name === "href" ? this.href : null; }
  }
  globalThis.Element = FakeElement;
  let listener;
  const opened = [], errors = [];
  const root = {
    addEventListener(name, callback, capture) {
      assert.equal(name, "click"); assert.equal(capture, true); listener = callback;
    },
    removeEventListener(name, callback) { assert.equal(callback, listener); listener = undefined; },
  };
  try {
    const detach = installExternalLinkHandler(root,
      async (url) => { opened.push(url); }, (message) => errors.push(message));
    const click = (href) => {
      const event = { defaultPrevented: false, button: 0,
        target: new FakeElement(href), preventDefault() { this.defaultPrevented = true; } };
      listener(event);
      return event.defaultPrevented;
    };
    assert.equal(click("#xid-relations"), false);
    assert.equal(click("https://www.postgresql.org/docs/16/"), true);
    assert.deepEqual(opened, ["https://www.postgresql.org/docs/16/"]);
    assert.deepEqual(errors, []);
    detach();
    assert.equal(listener, undefined);
  } finally { globalThis.Element = oldElement; }
});
