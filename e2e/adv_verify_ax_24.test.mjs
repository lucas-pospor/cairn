// Regression test for FINDING-208 (the quick switcher's Ctrl+N / Ctrl+P
// "next/previous result" keys were taken by global hotkeys first).
// adv_a11y_keyboard.test.mjs checks Ctrl+N; this checks Ctrl+P (default:
// command palette), the other half of the same QuickSwitcher.onKey branch.
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ax_24.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v24-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

// By design, the switcher's own Ctrl+N / Ctrl+P win over global hotkeys. On
// macOS the palette is Cmd+P, which still swaps the switcher for the palette.
test("FINDING-208: Ctrl+P inside the quick switcher moves up instead of opening the command palette", async () => {
  await app.reset();
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("e");
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=switcher-item]').length >= 2`);
  await app.keys(K.down);
  await sleep(150);
  const before = await app.exec(`return document.querySelector('.switcher [role=option][aria-selected=true]')?.dataset.i`);
  await app.chord(K.ctrl, "p");
  await sleep(600);
  const st = await app.exec(`return { highlighted: document.querySelector('.switcher [role=option][aria-selected=true]')?.dataset.i ?? null, switcherOpen: !!document.querySelector('.switcher'), paletteOpen: !!document.querySelector('[data-testid=palette-input]'), focus: __ax.desc(document.activeElement) }`);
  st.before = before;
  console.log(JSON.stringify(st));
  await app.keys(K.esc);
  await sleep(150);
  await app.keys(K.esc);
  assert.ok(st.before === "1" && st.highlighted === "0" && !st.paletteOpen, JSON.stringify(st));
});
