// Regression test for FINDING-048 without any test hook: an in-app flow where
// a structural change is followed within a few ms by a save.
//
// Clicking a broken wikilink in Live Preview creates the note
// (create_note emits a "created" batch) and then openNote() flushes the
// current tab (app.svelte.ts). If that tab still has unsaved typing (the
// 600 ms autosave has not fired yet), write_note emits a "modified" batch a few
// ms later. With the defect, handleChanges (app.svelte.ts) replaced the
// pending structural refresh timer with a content-only one, so the new note
// existed on disk and in the backend index but never appeared in the tree /
// quick switcher, and the link that created it kept its "unresolved" style.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_13.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("v13");
});
after(async () => {
  await env?.dispose();
});

const linkSel = `.cm-lp-wikilink[data-wiki="Fresh idea"]`;
const rowSel = `[data-testid=tree-row][data-path="Fresh idea.md"]`;

async function typeLinkAndClick(app, { waitForAutosave }) {
  await app.focusEnd();
  // A user edit (marks the tab dirty and arms the 600 ms autosave), then the
  // cursor moves to a new line so the link renders as a clickable widget.
  const t0 = Date.now();
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView;
    v.dispatch({ changes: { from: v.state.doc.length, insert: 'See [[Fresh idea]]\\nmore' }, userEvent: 'input.type' });
    v.dispatch({ selection: { anchor: v.state.doc.length } });`);
  if (waitForAutosave) await app.waitSaved();
  const el = await app.s.findWait(linkSel, 3000);
  const dirtyAtClick = (await app.tabs()).find((t) => t.active)?.dirty;
  await app.s.click(el); // real WebDriver click: a plain click opens the link in Live Preview
  const elapsed = Date.now() - t0;
  return { elapsed, dirtyAtClick };
}

test(
  "a note created by clicking a broken link right after typing shows up in the tree",
  async () => {
    const v = env.vault("v", { "Open.md": "open\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("Open.md");
      const { elapsed, dirtyAtClick } = await typeLinkAndClick(app, { waitForAutosave: false });
      console.log("click after", elapsed, "ms; tab dirty at click:", dirtyAtClick);
      assert.ok(dirtyAtClick, "precondition: the open note still had unsaved typing when the link was clicked");
      await eventually(async () => (await app.activeTab()) === "Fresh idea.md", { message: "new note opened" });
      await eventually(() => v.exists("Fresh idea.md"), { message: "new note on disk" });
      assert.equal(v.read("Open.md"), "open\nSee [[Fresh idea]]\nmore", "typing was saved by the flush");
      await sleep(1500);
      const inIndex = (await app.invoke("list_entries")).ok.some((e) => e.path === "Fresh idea.md");
      const inTree = await app.exec(`return !!document.querySelector(arguments[0])`, rowSel);
      console.log("inIndex", inIndex, "inTree", inTree);
      assert.ok(inIndex, "backend index has the new note");
      assert.ok(inTree, "new note is indexed and on disk but missing from the tree");
    } finally {
      await app.close();
    }
  },
);

test("control: same flow after autosave has landed shows the new note", async () => {
  const v = env.vault("v", { "Open.md": "open\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("Open.md");
    const { dirtyAtClick } = await typeLinkAndClick(app, { waitForAutosave: true });
    assert.equal(dirtyAtClick, false, "tab was clean at click");
    await eventually(async () => (await app.activeTab()) === "Fresh idea.md", { message: "new note opened" });
    await sleep(1500);
    const inTree = await app.exec(`return !!document.querySelector(arguments[0])`, rowSel);
    assert.ok(inTree, "control: new note shows in the tree");
  } finally {
    await app.close();
  }
});
