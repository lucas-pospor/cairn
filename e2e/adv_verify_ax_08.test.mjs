// Regression test for FINDING-109 (focus was not restored after closing
// overlays). The other reproduction (adv_a11y_keyboard.test.mjs) only reads
// document.activeElement; this one
// checks what a user notices: after Ctrl+P / Ctrl+O / inline rename are
// cancelled with Escape, typed text still reaches the note (with the defect,
// it was lost and the next Tab started at the top of the page; the test logs
// where focus and the next Tab go). Control: with no overlay in between, the
// same keys reach the note.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_08.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v08-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

const doc = () => app.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
const refocusEnd = () =>
  app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } }); return 1`);

test("FINDING-109: after Escape from palette/switcher/rename, typed text reaches the note", async () => {
  await app.reset();
  await app.openNote("garden", "Projects/Garden plan.md");

  // Control: typing straight into the editor works.
  await refocusEnd();
  await app.keys("zz0");
  await sleep(200);
  assert.ok((await doc()).includes("zz0"), "control: keys reach the editor when nothing was opened in between");

  const results = [];
  const step = async (label, open, waitCss) => {
    await refocusEnd();
    await open();
    await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(waitCss)})`);
    await app.keys(K.esc);
    await sleep(200);
    const marker = `q${results.length}x`;
    const focusAfterEsc = await app.focus();
    await app.keys(marker);
    await sleep(200);
    const typedReached = (await doc()).includes(marker);
    await app.keys(K.tab);
    const firstTab = await app.focus();
    results.push({ label, focusAfterEsc, typedReached, firstTab });
  };

  await step("command palette (Ctrl+P), Escape", () => app.chord(K.ctrl, "p"), "[data-testid=palette-input]");
  await step("quick switcher (Ctrl+O), Escape", () => app.chord(K.ctrl, "o"), "[data-testid=switcher-input]");
  await step("inline rename (palette: rename current), Escape", () => app.palette("rename current"), "[data-testid=rename-input]");

  console.log(JSON.stringify(results, null, 1));
  const lost = results.filter((r) => !r.typedReached);
  assert.deepEqual(lost, [], "after cancelling an overlay with Escape, typed characters no longer reach the open note");
});
