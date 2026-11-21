import test from "node:test";
import assert from "node:assert/strict";
import { dismissOpenDetailsOutside, closeOpenDetails } from "../dist/core/disclosure-menu.js";

function fakeMenu(inside) {
  return { open: true, contains(target) { return target === inside; } };
}
test("pointer inside menu preserves it; other open menus close", () => {
  const within = {name:'inside'};
  const m1 = fakeMenu(within), m2 = fakeMenu({name:'other'});
  const root = {querySelectorAll(selector){assert.equal(selector,'details.connection-menu');return [m1,m2];}};
  dismissOpenDetailsOutside(root,'details.connection-menu',within);
  assert.equal(m1.open,true);
  assert.equal(m2.open,false);
});
test("outside click dismisses all menus and Escape helper closes all", () => {
  const m1=fakeMenu({}),m2=fakeMenu({});
  const root={querySelectorAll(){return [m1,m2];}};
  dismissOpenDetailsOutside(root,'details.user-menu',{});
  assert.equal(m1.open,false);assert.equal(m2.open,false);
  m1.open=true;m2.open=true;
  closeOpenDetails(root,'details.user-menu');
  assert.equal(m1.open,false);assert.equal(m2.open,false);
});
test("null target or unmounted root is a no-op", () => {
  const m=fakeMenu({});
  const root={querySelectorAll(){return [m];}};
  dismissOpenDetailsOutside(root,'details.user-menu',null);
  dismissOpenDetailsOutside(null,'details.user-menu',{});
  assert.equal(m.open,true);
});
