// Reproduction for FINDING-106 (quick switcher and command palette can be
// open at once; closing the top one leaves the other orphaned with no
// keyboard focus, so Escape no longer closes it).
//
// Beyond the other reproduction (adv_a11y_keyboard.test.mjs), this one:
// - runs a control (Ctrl+O, Escape closes the switcher normally) so the Escape
//   key is known to be delivered;
// - logs every keydown on window to prove the second Escape reached the page;
// - records focus after the first Escape, checks which overlay is on top
//   (elementFromPoint), and whether Ctrl+O re-focuses the orphaned switcher;
// - counts how many Tab presses a keyboard user needs to get back into it;
// - checks the reverse order (Ctrl+P, then Ctrl+O): the switcher takes focus
//   while it is rendered underneath the palette.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_05.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v05-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

const state = () =>
  app.exec(`
    const sw = document.querySelector('[data-testid=switcher-input]');
    const pa = document.querySelector('[data-testid=palette-input]');
    const top = document.elementFromPoint(innerWidth / 2, innerHeight * 0.12 + 30);
    return {
      switcher: !!sw,
      palette: !!pa,
      onTop: top?.closest('[role=dialog]')?.getAttribute('aria-label') ?? null,
      focus: __ax.desc(document.activeElement),
      keys: (window.__v05keys ?? []).slice(),
    };`);

const logKeys = () =>
  app.exec(`window.__v05keys = []; addEventListener('keydown', e => window.__v05keys.push(e.key + '@' + (e.target?.dataset?.testid ?? e.target?.tagName)), true); return 1`);

test("control: Ctrl+O then Escape closes the quick switcher", async () => {
  await app.reset();
  await logKeys();
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys(K.esc);
  await sleep(200);
  const st = await state();
  assert.equal(st.switcher, false, JSON.stringify(st));
});

test("FINDING-106: Ctrl+O, Ctrl+P, Escape, Escape leaves the switcher open with focus on <body>", async () => {
  await app.reset();
  await logKeys();
  const log = {};
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.chord(K.ctrl, "p");
  await sleep(250);
  log.afterCtrlP = await state();
  await app.keys(K.esc);
  await sleep(250);
  log.afterEsc1 = await state();
  await app.keys(K.esc);
  await sleep(250);
  log.afterEsc2 = await state();
  // Does the switcher hotkey bring focus back? (switcherOpen is already true.)
  await app.chord(K.ctrl, "o");
  await sleep(250);
  log.afterCtrlO = await state();
  // Can a keyboard user Tab back into it?
  let tabs = 0;
  for (; tabs < 60; tabs++) {
    if (await app.exec(`return document.activeElement?.dataset.testid === 'switcher-input'`)) break;
    await app.keys(K.tab);
  }
  log.tabsToReachSwitcher = tabs < 60 ? tabs : ">=60";
  await app.shot("AX-05-verify-orphaned.png");
  console.log(JSON.stringify(log, null, 1));
  // Clean up for the next test.
  await app.exec(`document.querySelector('[data-testid=switcher-input]')?.focus(); return 1`);
  await app.keys(K.esc);
  assert.ok(!log.afterEsc2.switcher && !log.afterEsc2.palette, `after two Escapes: ${JSON.stringify(log)}`);
});

test("FINDING-106 (reverse order): Ctrl+P then Ctrl+O focuses the switcher while it is drawn under the palette", async () => {
  await app.reset();
  await logKeys();
  const log = {};
  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'palette-input'`);
  await app.chord(K.ctrl, "o");
  await sleep(250);
  log.afterCtrlO = await state();
  await app.keys("ideas");
  await sleep(200);
  log.typedInto = await app.exec(`return { switcher: document.querySelector('[data-testid=switcher-input]')?.value ?? null, palette: document.querySelector('[data-testid=palette-input]')?.value ?? null }`);
  await app.keys(K.esc);
  await sleep(250);
  log.afterEsc1 = await state();
  await app.keys(K.esc);
  await sleep(250);
  log.afterEsc2 = await state();
  await app.shot("AX-05-verify-reverse.png");
  console.log(JSON.stringify(log, null, 1));
  assert.ok(!(log.afterCtrlO.switcher && log.afterCtrlO.palette), `both overlays open at once: ${JSON.stringify(log)}`);
});
