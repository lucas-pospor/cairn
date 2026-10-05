// Reproduction for FINDING-112 (narrow-layout drawers: no focus move, no
// Escape, no aria-expanded, Tab goes behind the backdrop).
//
// Beyond the other reproduction (adv_a11y_keyboard.test.mjs), this one:
// - logs keydown events at window level (capture) so we know Escape really
//   reached the page while the drawer was open;
// - checks with elementFromPoint that the control Tab lands on ("Find note")
//   is really covered by the backdrop;
// - runs controls that bound the impact: Enter on the toggle again closes the
//   drawer (focus never left it), and Shift+Tab from the toggle reaches the
//   drawer's controls (aside.left precedes the centre column in DOM order);
// - repeats the checks for the right drawer ("Links and outline") and counts
//   the Tab presses needed to reach it.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_11.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v11-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop().catch(() => {});
});

async function narrow() {
  await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
  await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
  await app.exec(`window.__keys = []; addEventListener('keydown', e => __keys.push(e.key + '@' + (e.target.tagName || '?')), true); return 1`);
}
async function wide() {
  await app.s.cmd("POST", "/window/maximize", {});
  await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`).catch(() => {});
}

test("FINDING-112: left drawer opened from the keyboard", async () => {
  await app.reset();
  const problems = [];
  const info = {};
  try {
    await narrow();
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !document.querySelector('aside.left').classList.contains('hidden')`, { message: "drawer open" });
    const st = await app.exec(`const b = document.querySelector('[data-testid=mobile-files]');
      return { focus: __ax.desc(document.activeElement), inDrawer: !!document.activeElement?.closest('aside.left'),
               expanded: b.getAttribute('aria-expanded'), controls: b.getAttribute('aria-controls'),
               asideRole: document.querySelector('aside.left').getAttribute('role'), asideModal: document.querySelector('aside.left').getAttribute('aria-modal'),
               centerInert: document.querySelector('main.center').inert }`);
    info.open = st;
    if (!st.inDrawer) problems.push(`focus after opening: ${st.focus}`);
    if (st.expanded !== "true") problems.push(`aria-expanded=${st.expanded}`);

    // Tab forward: where does it go, and is that control under the backdrop?
    await app.keys(K.tab);
    const t = await app.exec(`const a = document.activeElement; const r = a.getBoundingClientRect();
      const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return { focus: __ax.desc(a), inDrawer: !!a.closest('aside.left'), topAtCentre: top ? __ax.desc(top) : null, covered: !!top && !a.contains(top) }`);
    info.tab = t;
    if (!t.inDrawer) problems.push(`Tab -> ${t.focus} (element on top at its centre: ${t.topAtCentre})`);

    // Escape.
    await app.exec(`window.__keys.length = 0; return 1`);
    await app.keys(K.esc);
    await sleep(250);
    const esc = await app.exec(`return { keys: [...__keys], open: !document.querySelector('aside.left').classList.contains('hidden') }`);
    info.esc = esc;
    if (esc.open) problems.push(`Escape (${esc.keys.join(",")}) leaves the drawer open`);

    // Control 1: Shift+Tab from the toggle reaches the drawer's controls.
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    let back = null;
    for (let i = 1; i <= 15; i++) {
      await app.chord(K.shift, K.tab);
      if (await app.exec(`return !!document.activeElement?.closest('aside.left')`)) {
        back = { presses: i, focus: await app.focus() };
        break;
      }
    }
    info.shiftTab = back;

    // Control 2: Enter on the toggle closes the drawer again.
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await sleep(250);
    info.enterAgainCloses = await app.exec(`return document.querySelector('aside.left').classList.contains('hidden')`);
    await app.shot("AX-11-verify-left.png");
  } finally {
    await wide();
  }
  console.log("AX-11 left drawer:", JSON.stringify(info, null, 2));
  assert.deepEqual(problems, []);
});

test("FINDING-112: right drawer (Links and outline) opened from the keyboard", async () => {
  await app.reset();
  const problems = [];
  const info = {};
  try {
    await narrow();
    await app.exec(`[...document.querySelectorAll('.mobile-bar button')].find(b => b.title === 'Links and outline').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !document.querySelector('aside.right').classList.contains('hidden')`, { message: "right drawer open" });
    const st = await app.exec(`const b = [...document.querySelectorAll('.mobile-bar button')].find(b => b.title === 'Links and outline');
      return { focus: __ax.desc(document.activeElement), inDrawer: !!document.activeElement?.closest('aside.right'), expanded: b.getAttribute('aria-expanded') }`);
    info.open = st;
    if (!st.inDrawer) problems.push(`focus after opening: ${st.focus}`);
    if (st.expanded !== "true") problems.push(`aria-expanded=${st.expanded}`);
    const visited = [];
    let reached = null;
    for (let i = 1; i <= 25; i++) {
      await app.keys(K.tab);
      const r = await app.exec(`return { d: __ax.desc(document.activeElement), inDrawer: !!document.activeElement?.closest('aside.right') }`);
      visited.push(r.d);
      if (r.inDrawer) {
        reached = i;
        break;
      }
    }
    info.tabsToReachDrawer = reached;
    info.visited = visited;
    if (reached !== 1) problems.push(`Tab from the toggle needs ${reached ?? ">25"} presses to reach the drawer; visited: ${visited.join(" | ")}`);
    await app.keys(K.esc);
    await sleep(250);
    if (!(await app.exec(`return document.querySelector('aside.right').classList.contains('hidden')`))) problems.push("Escape leaves the right drawer open");
  } finally {
    await wide();
  }
  console.log("AX-11 right drawer:", JSON.stringify(info, null, 2));
  assert.deepEqual(problems, []);
});
