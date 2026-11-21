import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

test("Rust and JS Tauri APIs share a major/minor version", () => {
  const cargo = source("../../Cargo.lock");
  const match = cargo.match(/\[\[package\]\]\s*name = "tauri"\s*version = "(\d+\.\d+\.\d+)"/);
  assert.ok(match, "Cargo.lock must contain tauri");
  const ui = JSON.parse(source("../package-lock.json"));
  const pkg = JSON.parse(source("../package.json"));
  const actual = ui.packages["node_modules/@tauri-apps/api"].version;
  assert.equal(actual.split(".").slice(0, 2).join("."), match[1].split(".").slice(0, 2).join("."));
  assert.equal(pkg.dependencies["@tauri-apps/api"], actual, "pin the JS API version to avoid drift");
  assert.equal(ui.packages[""].dependencies["@tauri-apps/api"], actual);
});

test("pinned Tauri API 2.11.1 has the verified registry tarball digest", () => {
  const ui = JSON.parse(source("../package-lock.json"));
  const api = ui.packages["node_modules/@tauri-apps/api"];
  if (api.version === "2.11.1") {
    assert.equal(
      api.integrity,
      "sha512-M2FPuYND2m+wh5hfW9ZpSdxMPdEJovPBWwoHJmwUpysTYNHaOkVFN419m/K0LIgjb/7KU2vBgsUepJWugQCvAA==",
      "do not reuse a CLI package digest for the API tarball",
    );
  }
});

test("Vite dev allows only the legacy resource folders imported by Vue", () => {
  const config = source("../vite.config.ts");
  assert.match(config, /fs:\s*\{\s*allow:/);
  assert.match(config, /resolve\(root, "styles"\)/);
  assert.match(config, /resolve\(root, "vendor\/xterm"\)/);
  assert.doesNotMatch(config, /resolve\(root, "\.\."\)/, "do not expose the whole repository to dev server");
  assert.match(source("../src/main.ts"), /ui\/styles\/tokens\.css/);
  assert.match(source("../src/main.ts"), /ui\/vendor\/xterm\/xterm\.css/);
});
