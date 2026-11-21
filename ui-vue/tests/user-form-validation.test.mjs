import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { validateUserForm, hasUserFormErrors } from "../dist/features/settings/user-form-validation.js";

const good = "VerySafeTestPassword123!";
test("create: preserves the password for missing confirmation", () => {
  const errors = validateUserForm({ kind: "create" }, "test_user", good, "");
  assert.equal(errors.confirm, "Повторите пароль.");
  assert.equal(errors.password, undefined);
  assert.equal(hasUserFormErrors(errors), true);
});
test("create: mismatching passwords are field errors", () => {
  const errors = validateUserForm({ kind: "create" }, "test_user", good, good + "x");
  assert.equal(errors.confirm, "Пароли не совпадают.");
});
test("create: invalid username is reported independently", () => {
  const errors = validateUserForm({ kind: "create" }, "invalid!", good, good);
  assert.match(errors.username ?? "", /Логин/);
});
test("rename: does not require password", () => {
  assert.deepEqual(validateUserForm({ kind: "rename", username: "old" }, "new", "", ""), {});
});
test("change password: needs correct confirmation", () => {
  assert.equal(validateUserForm({ kind: "password", username: "test" }, "", good, good).password, undefined);
  assert.deepEqual(validateUserForm({ kind: "password", username: "test" }, "", good, good), {});
});
test("password utf8 maximum checked before sending to Rust", () => {
  const errors = validateUserForm({ kind: "create" }, "new", "😀".repeat(300), "😀".repeat(300));
  assert.match(errors.password ?? "", /1024/);
});

test("create: an existing login is reported next to the username before IPC", () => {
  const errors = validateUserForm({ kind: "create" }, " admin ", good, good, ["admin", "viewer"]);
  assert.equal(errors.username, "Пользователь с таким логином уже существует.");
  assert.equal(errors.password, undefined);
  assert.equal(errors.confirm, undefined);
});
test("create: a new login is not blocked by the known user list", () => {
  assert.deepEqual(validateUserForm({ kind: "create" }, "newuser", good, good, ["admin"]), {});
});
test("rename: existing login precheck does not alter rename semantics", () => {
  assert.deepEqual(validateUserForm({ kind: "rename", username: "admin" }, "admin", "", "", ["admin"]), {});
});
