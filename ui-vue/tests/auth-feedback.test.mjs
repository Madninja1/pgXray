import assert from "node:assert/strict";
import test from "node:test";
import { authInlineError } from "../dist/features/auth/auth-feedback.js";

test("failed login is presented inline instead of in the global status", () => {
  assert.equal(authInlineError("login", { text: "Неверный логин или пароль", error: true }), "Неверный логин или пароль");
});

test("registration backend errors remain inside registration form", () => {
  assert.equal(authInlineError("registration", { text: "Регистрация недоступна", error: true }), "Регистрация недоступна");
});

test("normal status messages and non-authentication screens stay outside form", () => {
  assert.equal(authInlineError("login", { text: "Войдите в pgXray", error: false }), null);
  assert.equal(authInlineError(null, { text: "Операция не выполнена", error: true }), null);
  assert.equal(authInlineError("login", { text: "", error: true }), null);
});
