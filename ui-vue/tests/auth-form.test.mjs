import assert from "node:assert/strict";
import test from "node:test";
import { hasAuthFormErrors, validateAuthForm } from "../dist/features/auth/validation.js";

const validate = (mode, username, password, confirm = "") =>
  validateAuthForm(mode, username, password, confirm);

test("registration accepts Rust-compatible ASCII username and 12-symbol password", () => {
  assert.deepEqual(validate("registration", "test_user-2", "пароль-123456", "пароль-123456"), {});
});

test("registration rejects invalid usernames without trimming them", () => {
  for (const name of ["", " leading", "trailing ", "имя", "a".repeat(65), "a.b"]) {
    assert.ok(validate("registration", name, "123456789012", "123456789012").username);
  }
  assert.deepEqual(validate("registration", "x".repeat(64), "123456789012", "123456789012"), {});
});

test("registration enforces Unicode scalar minimum, not UTF-16 units", () => {
  assert.ok(validate("registration", "owner", "😀".repeat(11), "😀".repeat(11)).password);
  assert.deepEqual(validate("registration", "owner", "😀".repeat(12), "😀".repeat(12)), {});
});

test("registration enforces UTF-8 byte maximum", () => {
  const accepted = "é".repeat(512);
  const rejected = "é".repeat(513);
  assert.deepEqual(validate("registration", "owner", accepted, accepted), {});
  assert.ok(validate("registration", "owner", rejected, rejected).password);
});

test("registration rejects mismatched confirmation before IPC", () => {
  assert.equal(validate("registration", "owner", "123456789012", "123456789013").confirm, "Пароли не совпадают.");
});

test("login does not impose registration minimum on previously stored credentials", () => {
  assert.deepEqual(validate("login", "owner", "old"), {});
  assert.ok(validate("login", "owner", "").password);
  assert.ok(validate("login", "owner", "a".repeat(1025)).password);
  assert.ok(validate("login", "u".repeat(65), "valid").username);
});

test("hasAuthFormErrors detects field errors", () => {
  assert.equal(hasAuthFormErrors({}), false);
  assert.equal(hasAuthFormErrors({ confirm: "Пароли не совпадают." }), true);
});

test("registration requires confirmation without invalidating the entered password", () => {
  const result = validate("registration", "owner", "valid-password-123", "");
  assert.equal(result.confirm, "Повторите пароль.");
  assert.equal(result.password, undefined);
});

test("registration keeps mismatch on confirmation instead of password field", () => {
  const result = validate("registration", "owner", "valid-password-123", "valid-password-124");
  assert.equal(result.confirm, "Пароли не совпадают.");
  assert.equal(result.password, undefined);
});
