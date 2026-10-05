// Reproduction for FINDING-001.
// Opening another note in the same tab throws away unsaved edits when that
// tab's save is blocked. Beyond a click on another note while the conflict
// banner is up, these cases cover:
//  - the "deleted outside Cairn" banner variant,
//  - work typed AFTER the banner appeared (so the loss is not bounded by the
//    600 ms autosave window), lost through the quick switcher (Enter),
//  - undo cannot bring the text back after returning to the note.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_01.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, sleep, Key } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl01");
});
after(async () => {
  await env?.dispose();
});

async function openFresh(files, rel) {
  const v = env.vault("v", files);
  const app = await env.launch(v);
  await app.openFromTree(rel);
  await app.source();
  await sleep(150);
  return { v, app };
}

function diskHas(v, needle) {
  return v
    .listDisk()
    .filter((p) => p.endsWith(".md"))
    .some((p) => v.read(p).includes(needle));
}

test(
  "deleted-outside-Cairn banner, then a plain click on another note keeps my unsaved edits",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "B.md": "bravo\n" }, "A.md");
    try {
      await app.insertEnd("MY IMPORTANT EDIT");
      v.rm("A.md");
      const b = await app.waitBanner();
      assert.match(b, /deleted or moved/);
      await app.openFromTree("B.md");
      await sleep(800);
      const tabs = await app.tabs();
      // A.md is gone from disk and from the tree: come back to it through its
      // tab, if it is still open.
      if (tabs.some((t) => t.path === "A.md")) {
        await app.s.click(await app.s.find('[data-testid=tab][data-path="A.md"]'));
        await app.waitLoaded("A.md");
      }
      const ed = await app.editorText();
      assert.ok(
        ed.includes("MY IMPORTANT EDIT") || diskHas(v, "MY IMPORTANT EDIT"),
        `edit gone: tabs=${JSON.stringify(tabs)} editor=${JSON.stringify(ed)} disk=${JSON.stringify(v.listDisk())}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "text typed after the conflict banner appeared survives opening another note via the quick switcher, or undo brings it back",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "Bravo.md": "bravo\n" }, "A.md");
    try {
      await app.insertEnd("first words ");
      v.write("A.md", "THEIRS\n");
      await app.waitBanner();
      // The banner does not block editing: the user keeps writing.
      const para = "A WHOLE PARAGRAPH WRITTEN AFTER THE BANNER. ".repeat(20);
      await app.insertEnd(para);
      await sleep(1500); // several autosave periods: nothing is written
      assert.equal(v.read("A.md"), "THEIRS\n", "save stays blocked while the banner is up");
      // Quick switcher, Enter on the first result.
      await app.s.keys({ chord: [Key.ctrl, "o"] });
      await app.s.type(await app.s.findWait("[data-testid=switcher-input]"), "Bravo");
      await sleep(300);
      await app.s.keys(Key.enter);
      await eventually(async () => (await app.activeTab()) === "Bravo.md", { message: "Bravo active" });
      const tabsAfter = await app.tabs();
      await app.openFromTree("A.md");
      let ed = await app.editorText();
      if (!ed.includes("WRITTEN AFTER THE BANNER")) {
        // Try to recover with undo.
        await app.focusEnd();
        for (let i = 0; i < 5; i++) await app.s.keys({ chord: [Key.ctrl, "z"] });
        await sleep(300);
        ed = await app.editorText();
      }
      assert.ok(
        ed.includes("WRITTEN AFTER THE BANNER") || diskHas(v, "WRITTEN AFTER THE BANNER"),
        `paragraph gone: tabs after switch=${JSON.stringify(tabsAfter)} editor=${JSON.stringify(ed)} disk A=${JSON.stringify(v.read("A.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);
