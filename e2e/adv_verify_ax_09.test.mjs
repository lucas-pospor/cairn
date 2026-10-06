// Regression test for FINDING-110 (tab bar: the focused close button of an
// inactive tab was invisible; no arrow keys or Space on tabs).
//
// Beyond the other reproduction (adv_a11y_keyboard.test.mjs, which reads
// computed opacity), this one:
// - moves the pointer away from the tab bar first, so :hover cannot be what
//   shows or hides anything;
// - moves focus from the file tree with real Tab and arrow presses and, if
//   Tab reaches the close button, compares screen pixels around it (padded
//   for a focus ring) with focus on the tab vs focus on its close button:
//   identical pixels mean the focus move is not visible at all;
// - if Tab lands on that close button, checks that Enter there does not
//   close the inactive tab (with the defect, Enter on the invisible control
//   closed it);
// - checks ArrowRight / Space on a focused role=tab with real key events.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_09.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { AxApp, K, EVIDENCE, decodePng, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v09-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

async function twoTabs() {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("garden");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Garden')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.tabs()).length === 2, { message: "two tabs" });
  await eventually(async () => (await app.activeTab()) === "Projects/Garden plan.md", { message: "Garden active" });
  // Park the pointer over the editor, far from the tab bar.
  await app.s.pointer([{ type: "pointerMove", x: 700, y: 500, duration: 0 }]);
  await sleep(150);
}

function crop(img, r, scale, pad = 5) {
  const x0 = Math.max(0, Math.floor((r.x - pad) * scale)), y0 = Math.max(0, Math.floor((r.y - pad) * scale));
  const x1 = Math.min(img.w, Math.ceil((r.x + r.w + pad) * scale)), y1 = Math.min(img.h, Math.ceil((r.y + r.h + pad) * scale));
  const out = [];
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
    const o = (y * img.w + x) * 4;
    out.push(`${img.px[o]},${img.px[o + 1]},${img.px[o + 2]}`);
  }
  return out;
}

test("FINDING-110: a focused inactive tab (or its close button) shows visible focus; Space/arrows work on tabs", async () => {
  await twoTabs();
  const problems = [];
  const log = [];

  await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
  await app.keys(K.tab);
  log.push("Tab 1: " + (await app.focus()));
  // The tab list is one Tab stop (FINDING-216): Tab lands on a tab, and the
  // inactive Ideas tab is reached from there with the arrow keys.
  const onTab = await app.exec(`return document.activeElement?.matches('[data-testid=tab]')`);
  assert.ok(onTab, "first Tab from the tree lands on a tab");
  if (!(await app.exec(`return document.activeElement?.matches('[data-path="Ideas.md"]')`))) await app.keys(K.left);
  const onIdeas = await app.exec(`return document.activeElement?.matches('[data-testid=tab][data-path="Ideas.md"]')`);
  if (!onIdeas) problems.push(`ArrowLeft from the open tab did not reach the inactive Ideas tab (focus: ${await app.focus()})`);
  // :focus-visible does not match under WebDriver input here, so read the focus style the page defines.
  const tabFocus = await app.exec(`return __ax.focusStyle(document.activeElement)`);
  log.push("focus style of the focused tab: " + JSON.stringify(tabFocus));
  if (onIdeas && !tabFocus.indicator) problems.push("the focused Ideas tab shows no focus indicator");
  const r = await app.rectOf(`[data-testid=tab][data-path="Ideas.md"] .close`);
  const scale = await app.exec(`return window.devicePixelRatio || 1`);
  const shotA = await app.s.screenshot();
  const imgA = decodePng(Buffer.isBuffer(shotA) ? shotA : Buffer.from(shotA, "base64"));
  const sx = imgA.w / (await app.exec(`return window.innerWidth`));

  await app.keys(K.tab);
  const st = await app.exec(`const b = document.activeElement; const cs = getComputedStyle(b);
    return { desc: __ax.desc(b), isClose: b.matches('[data-path="Ideas.md"] .close'), opacity: cs.opacity,
             focusVisible: b.matches(':focus-visible'), hover: !!b.closest('.tab')?.matches(':hover'), outline: cs.outlineStyle + ' ' + cs.outlineWidth }`);
  log.push("Tab 2: " + JSON.stringify(st));
  const shotB = await app.s.screenshot();
  const imgB = decodePng(Buffer.isBuffer(shotB) ? shotB : Buffer.from(shotB, "base64"));
  fs.writeFileSync(path.join(EVIDENCE, "AX-09-verify-close-focused.png"), Buffer.isBuffer(shotB) ? shotB : Buffer.from(shotB, "base64"));
  const a = crop(imgA, r, sx), b = crop(imgB, r, sx);
  const diff = a.filter((p, i) => p !== b[i]).length;
  log.push(`pixels around the close button (${a.length} px, scale ${sx}, dpr ${scale}): ${diff} differ between "tab focused" and "close focused"`);
  if (st.isClose && st.opacity === "0") problems.push(`focused close button has opacity 0 (focus-visible=${st.focusVisible}, hover=${st.hover})`);
  if (st.isClose && diff === 0) problems.push("moving focus from the tab to its close button changes no pixels: the focus is invisible");

  // With the defect, Enter on the invisible control closed the inactive tab: check it stays open.
  // (Only pressed there: elsewhere Enter would act on whatever has focus.)
  if (st.isClose) {
    await app.keys(K.enter);
    await sleep(250);
    const left = await app.tabs();
    log.push("after Enter on invisible focused close button, tabs: " + JSON.stringify(left));
    if (!left.includes("Ideas.md")) problems.push("Enter on the invisible focused control closed the Ideas tab");
  }

  // Arrow / Space on a role=tab.
  await twoTabs();
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Ideas.md"]').focus(); return 1`);
  await app.keys(K.right);
  const afterRight = await app.exec(`return document.activeElement?.dataset.path ?? __ax.desc(document.activeElement)`);
  log.push("ArrowRight on Ideas tab, focus: " + afterRight);
  if (afterRight !== "Projects/Garden plan.md") problems.push(`ArrowRight did not move focus to the next tab (focus: ${afterRight})`);
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Ideas.md"]').focus(); return 1`);
  await app.keys(K.space);
  await sleep(200);
  const act = await app.activeTab();
  log.push("Space on Ideas tab, active: " + act);
  if (act !== "Ideas.md") problems.push("Space on a focused tab does not activate it");
  // Control: Enter does.
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Ideas.md"]').focus(); return 1`);
  await app.keys(K.enter);
  await sleep(200);
  log.push("control: Enter on Ideas tab, active: " + (await app.activeTab()));

  console.log(log.join("\n"));
  assert.deepEqual(problems, []);
});
