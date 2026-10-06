// Regression test for FINDING-202: after a narrow -> wide window
// resize, both sidebars stayed hidden. This version checks that both sidebars
// were open before the resize and are open again after it, and also checks
// that the inner width really crossed the 760px narrow breakpoint in both
// directions.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_13.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v13-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop("verify-ax13-final.png");
});

const state = `return {
  innerWidth: window.innerWidth,
  narrow: !!document.querySelector('.workspace.narrow'),
  left: !document.querySelector('aside.left').classList.contains('hidden'),
  right: !document.querySelector('aside.right').classList.contains('hidden'),
}`;

test("FINDING-202: sidebars open before a narrow -> wide resize are open again afterwards", async () => {
  await app.reset();
  const pre = await app.exec(state);
  assert.equal(pre.narrow, false, `precondition: wide layout ${JSON.stringify(pre)}`);
  assert.equal(pre.left && pre.right, true, `precondition: both sidebars open ${JSON.stringify(pre)}`);
  let mid;
  try {
    await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
    await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
    mid = await app.exec(state);
  } finally {
    await app.s.cmd("POST", "/window/maximize", {});
  }
  await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`, { message: "wide layout again" });
  await sleep(300);
  const post = await app.exec(state);
  console.log(JSON.stringify({ pre, mid, post }));
  assert.ok(post.innerWidth > 760, `wide again: ${post.innerWidth}`);
  assert.deepEqual({ left: post.left, right: post.right }, { left: pre.left, right: pre.right });
});
