// Regression tests for FINDING-008: keys typed while the graph view was shown
// reached the hidden CodeMirror view and were autosaved into the note.
// Adds: the DOM selection check (mechanism), destructive keys (Backspace,
// Ctrl+A + a letter), and reading view entered from source mode (to see
// whether reading view entered from Live Preview is safe only by accident).
//
// With the defect, when the graph tab was active (Ctrl+G) the hidden editor
// kept a DOM selection inside .cm-content while focus was on <body>. Letters
// were inserted, and a single Backspace emptied the whole note, which autosave
// wrote to disk (""). Reading view held up only when entered from Live
// Preview; from source mode, letters, Space and Backspace reached the hidden
// note in the same way (Ctrl+Z in the editor afterwards restored it while
// the tab was still open). Ctrl+A + a letter changed nothing.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_31.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-vax31-");
const BODY = "# Hidden\n\nfirst line of the note\nsecond line of the note\n";

before(async () => {
  await app.start();
});
after(async () => {
  await app.stop();
});

async function openFresh(file, query) {
  app.write(file, BODY);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="${file}"]')`);
  await app.openNote(query, file);
}

const selState = `const s = getSelection(); const a = document.activeElement;
  return { focus: __ax.desc(a), ranges: s.rangeCount, anchorInCm: !!(s.anchorNode && (s.anchorNode.nodeType === 1 ? s.anchorNode : s.anchorNode.parentElement)?.closest('.cm-content')), editorDisplay: getComputedStyle(document.querySelector('[data-testid=editor]')).display }`;

test("FINDING-008: behind the graph view, typing letters, Backspace or Ctrl+A+key leaves the note on disk unchanged", async () => {
  await app.reset();
  const problems = [];

  // 1. letters
  await openFresh("Vx letters.md", "vx letters");
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`);
  await sleep(300);
  const st1 = await app.exec(selState);
  console.log("after Ctrl+G:", JSON.stringify(st1));
  await app.keys("abc");
  await sleep(1300);
  const d1 = app.read("Vx letters.md");
  if (d1 !== BODY) problems.push(`letters behind graph (${JSON.stringify(st1)}): disk is ${JSON.stringify(d1)}`);

  // 2. Backspace
  await openFresh("Vx back.md", "vx back");
  await app.keys(K.down, K.down, K.down);
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`);
  await sleep(300);
  await app.keys("\uE003", "\uE003", "\uE003", "\uE003"); // Backspace x4
  await sleep(1300);
  const d2 = app.read("Vx back.md");
  console.log("after Backspace x4:", JSON.stringify(d2));
  if (d2 !== BODY) problems.push(`Backspace x4 behind graph: disk is ${JSON.stringify(d2)}`);

  // 3. Ctrl+A then a letter
  await openFresh("Vx all.md", "vx all");
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`);
  await sleep(300);
  await app.chord(K.ctrl, "a");
  await sleep(100);
  const selAll = await app.exec(`return getSelection().toString().length`);
  console.log("Ctrl+A selected chars:", selAll);
  await app.keys("z");
  await sleep(1300);
  const d3 = app.read("Vx all.md");
  console.log("after Ctrl+A z:", JSON.stringify(d3));
  if (d3 !== BODY) problems.push(`Ctrl+A (selected ${selAll} chars) then "z" behind graph: disk is ${JSON.stringify(d3)}`);
  await app.shot("AX-31-verify-ctrl-a.png");

  console.log(problems.join("\n"));
  assert.deepEqual(problems, []);
});

test("FINDING-008: reading view (Ctrl+E) entered from source mode", async () => {
  await app.reset();
  await openFresh("Vx src.md", "vx src");
  await app.palette("source mode");
  await eventually(() => app.exec(`return !document.querySelector('[data-testid=editor] .cm-editor').classList.contains('cm-live-preview')`), { message: "source mode" });
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  await sleep(200);
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] h1')`);
  await sleep(300);
  const st = await app.exec(selState);
  console.log("after Ctrl+E from source mode:", JSON.stringify(st));
  await app.keys("qq");
  await sleep(1300);
  const d = app.read("Vx src.md");
  console.log("disk:", JSON.stringify(d));
  assert.equal(d, BODY, `reading view from source mode: typing changed the note (${JSON.stringify(st)})`);
});

test("FINDING-008 control: after clicking the graph's find box, typing goes there and the note is unchanged", async () => {
  await app.reset();
  await openFresh("Vx find.md", "vx find");
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`);
  const r = await app.rectOf("[data-testid=graph-view] input.text-input");
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(r.x + r.w / 2), y: Math.round(r.y + r.h / 2), duration: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
  await sleep(200);
  await app.keys("welc");
  await sleep(1300);
  const v = await app.exec(`return document.querySelector('[data-testid=graph-view] input.text-input').value`);
  assert.equal(v, "welc");
  assert.equal(app.read("Vx find.md"), BODY);
});

test("FINDING-008: pressing Space (to scroll) in reading view entered from source mode leaves the note unchanged", async () => {
  await app.reset();
  await openFresh("Vx space.md", "vx space");
  await app.palette("source mode");
  await eventually(() => app.exec(`return !document.querySelector('[data-testid=editor] .cm-editor').classList.contains('cm-live-preview')`), { message: "source mode" });
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  await sleep(200);
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] h1')`);
  await sleep(300);
  await app.keys(K.space, K.space);
  await sleep(1300);
  const d = app.read("Vx space.md");
  console.log("disk after Space x2 in reading view:", JSON.stringify(d));
  assert.equal(d, BODY);
});

test("FINDING-008: Backspace behind the graph view leaves the hidden note intact (step by step)", async () => {
  await app.reset();
  const file = "Vx back2.md";
  await openFresh(file, "vx back2");
  const doc = () => app.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
  const before = await doc();
  console.log("editor doc before:", JSON.stringify(before), "disk:", JSON.stringify(app.read(file)));
  assert.equal(before, BODY);
  // Cursor at the end of the second body line, as after typing there.
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length - 1 } }); return 1`);
  await sleep(200);
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`);
  await sleep(300);
  console.log("after Ctrl+G:", JSON.stringify(await app.exec(selState)));
  const steps = [];
  for (let i = 1; i <= 3; i++) {
    await app.keys("");
    await sleep(150);
    steps.push(`Backspace ${i}: editor doc ${JSON.stringify(await doc())}`);
  }
  await sleep(1300);
  const disk = app.read(file);
  steps.push(`disk after autosave: ${JSON.stringify(disk)}`);
  // Back to the note: what does the user see, and does Ctrl+Z bring it back?
  await app.exec(`document.querySelector('[data-testid=tab][data-path="${file}"]').click(); return 1`);
  await eventually(async () => (await app.activeTab()) === file);
  await sleep(300);
  await app.shot("AX-31-verify-backspace-after.png");
  steps.push(`back on the note tab: editor doc ${JSON.stringify(await doc())}`);
  console.log(steps.join("\n"));
  assert.equal(disk, BODY);
});

test("FINDING-008: Backspace in reading view and behind Settings (mouse); Ctrl+Z afterwards", async () => {
  await app.reset();
  const doc = () => app.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
  const out = [];

  // a) reading view from Live Preview (the case that holds up), Backspace
  await openFresh("Vx rl.md", "vx rl");
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] h1')`);
  await sleep(300);
  const sa = await app.exec(selState);
  await app.keys("");
  await sleep(1300);
  out.push(`reading view from Live Preview ${JSON.stringify(sa)}: disk ${JSON.stringify(app.read("Vx rl.md"))}`);
  await app.chord(K.ctrl, "e");
  await sleep(300);

  // b) reading view from source mode, Backspace
  await openFresh("Vx rs.md", "vx rs");
  await app.palette("source mode");
  await eventually(() => app.exec(`return !document.querySelector('[data-testid=editor] .cm-editor').classList.contains('cm-live-preview')`), { message: "source mode" });
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  await sleep(200);
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] h1')`);
  await sleep(300);
  await app.keys("");
  await sleep(1300);
  out.push(`reading view from source mode: disk ${JSON.stringify(app.read("Vx rs.md"))}`);
  // back to editing and Ctrl+Z
  await app.chord(K.ctrl, "e");
  await sleep(400);
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  await app.chord(K.ctrl, "z");
  await sleep(1300);
  out.push(`  after Ctrl+E and Ctrl+Z in the editor: editor ${JSON.stringify(await doc())}, disk ${JSON.stringify(app.read("Vx rs.md"))}`);

  // c) Settings opened with the mouse while editing, Backspace
  await openFresh("Vx set.md", "vx set");
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  await sleep(200);
  const g = await app.rectOf("[data-testid=open-settings]");
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(g.x + g.w / 2), y: Math.round(g.y + g.h / 2), duration: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await sleep(200);
  const sc = await app.exec(selState);
  await app.keys("");
  await sleep(1300);
  out.push(`behind Settings (mouse) ${JSON.stringify(sc)}: disk ${JSON.stringify(app.read("Vx set.md"))}`);
  await app.keys(K.esc);
  console.log(out.join("\n"));
  assert.ok(out.every((l) => !l.includes("disk \"\"")), out.join("\n"));
});
