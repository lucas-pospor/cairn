// Reproduction for FINDING-001.
// Opening another note in the same tab while that tab's save is blocked
// (conflict banner) silently drops the unsaved edits.
//
// These variants use real key events, keep typing after the banner (so the
// loss is not limited to the 600 ms autosave window), and reach openNote
// through the quick switcher and a plain click on a wikilink in Live Preview,
// plus the "deleted outside Cairn" banner that offers an explicit
// "Discard and close" button.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_01_s2.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, sleep, Key } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("verify-dl01");
});
after(async () => {
  await env?.dispose();
});

function everywhere(app, v, ed) {
  return [ed, ...v.listDisk().filter((p) => p.endsWith(".md")).map((p) => v.read(p))];
}

test(
  "real typing, external write, user keeps typing after the banner, quick switcher to B: typed text survives",
  async () => {
    const v = env.vault("v", { "A.md": "alpha\n", "B.md": "bravo\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("A.md");
      await app.source();
      await sleep(300);
      await app.typeEnd("first words ");
      v.write("A.md", "THEIRS\n"); // another program / sync writes while the tab is dirty
      await app.waitBanner();
      // The user does not act on the banner and keeps writing.
      await app.s.keys("A WHOLE PARAGRAPH TYPED AFTER THE BANNER");
      await sleep(900);
      const status = await app.exec(`return document.querySelector('.statusbar, [class*=status]')?.textContent ?? ''`);
      const tabsBefore = await app.tabs();
      // Quick switcher: Ctrl+O, type B, Enter (same tab).
      await app.s.keys({ chord: [Key.ctrl, "o"] });
      await app.s.type(await app.s.findWait("[data-testid=switcher-input]"), "B");
      await sleep(250);
      await app.s.keys(Key.enter);
      await eventually(async () => (await app.activeTab()) === "B.md", { message: "B active" });
      const tabsAfter = await app.tabs();
      await app.openFromTree("A.md");
      let ed = await app.editorText();
      if (!ed.includes("A WHOLE PARAGRAPH")) {
        // Try undo too: maybe the history kept it.
        await app.focusEnd();
        await app.s.keys({ chord: [Key.ctrl, "z"] });
        await sleep(200);
        ed = await app.editorText();
      }
      const all = everywhere(app, v, ed);
      assert.ok(
        all.some((t) => t.includes("A WHOLE PARAGRAPH")),
        `edit gone: status=${JSON.stringify(status)} tabsBefore=${JSON.stringify(tabsBefore)} tabsAfter=${JSON.stringify(tabsAfter)} editor=${JSON.stringify(ed)} diskA=${JSON.stringify(v.read("A.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "conflict banner, then a plain click on a wikilink in my own note (Live Preview): typed text survives",
  async () => {
    const v = env.vault("v", { "A.md": "See [[B]]\n\nalpha\n", "B.md": "bravo\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("A.md");
      await sleep(300);
      await app.typeEnd("MY LIVE PREVIEW EDIT");
      v.write("A.md", "See [[B]]\n\nTHEIRS\n");
      await app.waitBanner();
      const link = await app.s.findWait(".cm-lp-wikilink[data-wiki='B']");
      await app.s.click(link);
      await eventually(async () => (await app.activeTab()) === "B.md", { message: "B active via link" });
      const tabs = await app.tabs();
      await app.openFromTree("A.md");
      const ed = await app.editorText();
      assert.ok(
        everywhere(app, v, ed).some((t) => t.includes("MY LIVE PREVIEW EDIT")),
        `edit gone: tabs after link click=${JSON.stringify(tabs)} editor=${JSON.stringify(ed)} diskA=${JSON.stringify(v.read("A.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "note deleted outside Cairn while dirty (banner offers 'Discard and close'), plain tree click on B: text survives",
  async () => {
    const v = env.vault("v", { "A.md": "alpha\n", "B.md": "bravo\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("A.md");
      await app.source();
      await sleep(300);
      await app.typeEnd("ONLY COPY OF MY TEXT");
      v.rm("A.md");
      const b = await app.waitBanner();
      assert.match(b, /deleted or moved/);
      await app.openFromTree("B.md");
      await sleep(800);
      const tabs = await app.tabs();
      const ed = await app.editorText();
      assert.ok(
        everywhere(app, v, ed).some((t) => t.includes("ONLY COPY OF MY TEXT")) || tabs.some((t) => t.path === "A.md"),
        `edit gone: tabs=${JSON.stringify(tabs)} disk=${JSON.stringify(v.listDisk())} editor=${JSON.stringify(ed)}`,
      );
    } finally {
      await app.close();
    }
  },
);

// Control: the same flow without a blocked save keeps the edits (autosave
// flushes on switch), so the loss is specific to the blocked-save state.
test("control: dirty but saveable tab, plain click on B, edits are flushed to disk", async () => {
  const v = env.vault("v", { "A.md": "alpha\n", "B.md": "bravo\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("A.md");
    await app.source();
    await sleep(300);
    await app.typeEnd("SAVEABLE EDIT");
    await app.openFromTree("B.md");
    await eventually(() => v.read("A.md").includes("SAVEABLE EDIT"), { message: "flushed" });
  } finally {
    await app.close();
  }
});
