// Regression tests for FINDING-105 (global hotkeys ran behind the Settings
// modal). Differences from the other reproduction (adv_a11y_keyboard.test.mjs):
// - a control first: with a DialogHost confirm dialog open, Ctrl+O is ignored
//   (Workspace.onKey returns early on app.dialog), so the expectation is the
//   app's own behaviour for its other modal;
// - the hidden switcher is checked by hit-testing every corner of the input,
//   not just its centre;
// - Ctrl+N is checked anywhere in the vault (new notes go into the folder of
//   the active note, so a root-only "Untitled.md" check could not
//   see the note after the switcher had opened Projects/Garden plan.md);
// - Ctrl+W (close tab) behind Settings is checked too.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_04.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { AxApp, K, sleep, eventually } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v04-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

function untitled() {
  return fs
    .readdirSync(app.vault, { recursive: true })
    .map(String)
    .filter((p) => path.basename(p).startsWith("Untitled"));
}

const settingsOpen = () => app.exec(`return !!document.querySelector('[data-testid=settings]')`);
const switcherOpen = () => app.exec(`return !!document.querySelector('[data-testid=switcher-input]')`);

async function openSettings() {
  await app.chord(K.ctrl, ",");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`, { message: "settings open" });
}

test("FINDING-105 control: Ctrl+O is ignored while a DialogHost confirm dialog is open", async () => {
  await app.reset();
  app.write("Victim.md", "keep\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Victim.md"]')`);
  await app.openNote("victim", "Victim.md");
  await app.palette("delete current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`, { message: "confirm dialog focused" });
  await app.chord(K.ctrl, "o");
  await sleep(300);
  const sw = await switcherOpen();
  await app.keys(K.esc);
  await sleep(300);
  assert.equal(sw, false, "Ctrl+O opened the switcher behind a DialogHost dialog");
  assert.ok(app.exists("Victim.md"), "Escape should have cancelled the delete");
});

test("FINDING-105: Ctrl+O / Ctrl+N / Ctrl+W do nothing behind the open Settings modal", async () => {
  await app.reset();
  const problems = [];
  const before = untitled();
  try {
    // 1) Ctrl+O: the switcher opens under Settings, focused but invisible.
    await app.openNote("ideas", "Ideas.md");
    await openSettings();
    await app.chord(K.ctrl, "o");
    await sleep(300);
    const sw = await app.exec(`
      const i = document.querySelector('[data-testid=switcher-input]');
      if (!i) return null;
      const r = i.getBoundingClientRect();
      const pts = [[r.left + 4, r.top + 4], [r.right - 4, r.top + 4], [r.left + 4, r.bottom - 4], [r.right - 4, r.bottom - 4], [r.x + r.width / 2, r.y + r.height / 2]];
      const hits = pts.map(([x, y]) => { const e = document.elementFromPoint(x, y); return { onSwitcher: !!e?.closest('.switcher'), inSettings: !!e?.closest('[data-testid=settings]') }; });
      return { focused: document.activeElement === i, anyVisible: hits.some(h => h.onSwitcher), allUnderSettings: hits.every(h => h.inSettings), rect: [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)] };`);
    await app.shot("AX-04-verify-switcher-hidden.png");
    if (sw) problems.push(`Ctrl+O with Settings open opened the quick switcher: ${JSON.stringify(sw)}`);
    if (sw) {
      await app.keys("garden", K.enter);
      await sleep(600);
      const t = await app.activeTab();
      if (t !== "Ideas.md") problems.push(`typing "garden" + Enter (Settings still open: ${await settingsOpen()}) replaced the note behind Settings with ${t}`);
    }
    await app.keys(K.esc);
    await sleep(300);
    if ((await settingsOpen()) || (await switcherOpen())) problems.push("one Escape did not close Settings and the hidden switcher");

    // 2) Ctrl+N: a new note is created behind Settings (in the active note's folder).
    await openSettings();
    await app.chord(K.ctrl, "n");
    await sleep(1000);
    const created = untitled().filter((p) => !before.includes(p));
    if (created.length) {
      problems.push(
        `Ctrl+N with Settings open created ${created.join(", ")} behind the modal; settings still open: ${await settingsOpen()}; focus: ${await app.focus()}`,
      );
    }
    await app.keys(K.esc);
    await sleep(300);
    if (await settingsOpen()) {
      await app.keys(K.esc);
      await sleep(300);
    }

    // 3) Ctrl+W: closes the tab behind Settings.
    await app.openNote("welcome", "Welcome.md");
    const tabsBefore = await app.tabs();
    await openSettings();
    await app.chord(K.ctrl, "w");
    await sleep(500);
    const tabsAfter = await app.tabs();
    if (tabsAfter.length < tabsBefore.length) problems.push(`Ctrl+W with Settings open closed a tab behind it: ${JSON.stringify(tabsBefore)} -> ${JSON.stringify(tabsAfter)}`);
    await app.keys(K.esc);
    await sleep(200);
  } finally {
    for (const p of untitled()) if (!before.includes(p)) fs.rmSync(app.p(p), { force: true });
  }
  assert.deepEqual(problems, []);
});
