// Reproduction for FINDING-008 (see also adv_verify_ax_31.test.mjs)
// checking the harm done: what a keyboard user who presses Ctrl+G and
// starts typing a node name (expecting the graph's find box) does to the
// note behind the graph, whether they can see it, and whether Ctrl+Z or
// anything else brings it back.
//
// The defect: focus stays on <body>; "tom" goes nowhere visible (the
// graph's find box stays empty) and the single Backspace erases the whole
// hidden note, which autosave writes to disk as "". Back on the note tab
// the editor shows the empty note and one Ctrl+Z restores it (and autosave
// writes it back). Once the tab is closed (or the app quit), the undo
// history is gone and the note stays empty on disk; nothing else keeps a
// copy unless sync version history is set up.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_31_02.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-vax31b-");
const BODY = "# Garden plan\n\nTomatoes go in the south bed.\nBeans along the fence.\n";
const BACKSPACE = "";

before(async () => {
  await app.start();
});
after(async () => {
  await app.stop();
});

const doc = () => app.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);

async function openFresh(file, query) {
  app.write(file, BODY);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="${file}"]')`);
  await app.openNote(query, file);
  // Cursor at the end of the last line, as after typing there.
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length - 1 } }); return 1`);
  await sleep(200);
}

async function graphThenType(...keys) {
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`);
  await sleep(300);
  const focus = await app.focus();
  await app.keys(...keys);
  await sleep(1400); // autosave is 1 s after the last edit
  return focus;
}

async function backToTab(file) {
  await app.exec(`document.querySelector('[data-testid=tab][data-path="${file}"]').click(); return 1`);
  await eventually(async () => (await app.activeTab()) === file);
  await sleep(300);
}

test("FINDING-008 impact: Ctrl+G, type a node name, one Backspace; what reaches disk, what the user sees, and whether Ctrl+Z restores it", async () => {
  await app.reset();
  const file = "Qv plan.md";
  const out = [];
  await openFresh(file, "qv plan");
  const focus = await graphThenType("tom", BACKSPACE);
  const tabLabel = await app.exec(`return document.querySelector('[data-testid=tab][data-path="${file}"]').textContent.trim()`);
  out.push(`focus while graph shown: ${focus}; note tab label: ${JSON.stringify(tabLabel)}`);
  const diskHidden = app.read(file);
  out.push(`disk after "tom"+Backspace behind graph: ${JSON.stringify(diskHidden)}`);
  out.push(`graph find box value: ${JSON.stringify(await app.exec(`return document.querySelector('[data-testid=graph-view] input.text-input').value`))}`);

  await backToTab(file);
  out.push(`back on the note tab, the editor shows: ${JSON.stringify(await doc())}`);
  await app.shot("AX-31-02-back-on-note.png");

  // Undo in the editor until it matches the original (at most 10 presses).
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  let presses = 0;
  while ((await doc()) !== BODY && presses < 10) {
    await app.chord(K.ctrl, "z");
    presses++;
    await sleep(100);
  }
  await sleep(1400);
  out.push(`after ${presses} x Ctrl+Z: editor ${JSON.stringify(await doc())}; disk ${JSON.stringify(app.read(file))}`);
  console.log(out.join("\n"));
  assert.equal(diskHidden, BODY, "the hidden note must not change");
});

test("FINDING-008 impact: after the hidden edit is autosaved, closing the tab loses the undo history; reopening shows the damaged note", async () => {
  await app.reset();
  const file = "Garden close.md";
  const out = [];
  await openFresh(file, "garden close");
  await graphThenType("tom", BACKSPACE);
  const disk1 = app.read(file);
  out.push(`disk after "tom"+Backspace behind graph: ${JSON.stringify(disk1)}`);
  await backToTab(file);
  await app.chord(K.ctrl, "w");
  await eventually(async () => !(await app.tabs()).includes(file), { message: "tab closed" });
  await sleep(300);
  await app.openNote("garden close", file);
  await app.chord(K.ctrl, "z");
  await sleep(1400);
  out.push(`reopened + Ctrl+Z: editor ${JSON.stringify(await doc())}; disk ${JSON.stringify(app.read(file))}`);
  console.log(out.join("\n"));
  assert.equal(app.read(file), BODY);
});
