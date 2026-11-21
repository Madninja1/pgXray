import test from "node:test";
import assert from "node:assert/strict";
import { pathDraftStatus } from "../dist/features/settings/paths-view-model.js";

test("path draft distinguishes empty, unchanged and changed paths", () => {
  assert.equal(pathDraftStatus("", "/home/u/data"), "empty");
  assert.equal(pathDraftStatus("   ", "/home/u/data"), "empty");
  assert.equal(pathDraftStatus(" /home/u/data ", "/home/u/data"), "unchanged");
  assert.equal(pathDraftStatus("/home/u/backup", "/home/u/data"), "changed");
});

test("path draft does not try to infer filesystem validity", () => {
  assert.equal(pathDraftStatus("../relative", "/home/u/data"), "changed");
  assert.equal(pathDraftStatus("/home/u/symlink", "/home/u/data"), "changed");
});
