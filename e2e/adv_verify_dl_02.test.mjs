// Reproduction for FINDING-002.
// Closing a tab, or leaving the vault, drops unsaved edits without asking when
// the tab's save is blocked (conflict banner or a failed write). Beyond
// closing a tab while the conflict banner is up, these cases cover:
//  - work typed AFTER the banner appeared (so the loss is not bounded by the
//    600 ms autosave window), then the tab's X: the text is nowhere afterwards;
//  - the "deleted outside Cairn" banner offers an explicit "Discard and close"
//    button, yet the tab's X does the same discard with no question;
//  - a single click on the vault name in the status bar ("Switch vault");
//  - middle-click on a tab whose save failed (read-only folder).
// A control case shows that closing a dirty tab whose save is NOT blocked
// does save, so only the blocked-save path loses the text.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_02.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { Env, eventually, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl02");
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

const PARA = "A WHOLE PARAGRAPH WRITTEN AFTER THE BANNER. ".repeat(20);

test("control: closing a dirty tab whose save is not blocked saves the edits first", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
  try {
    await app.insertEnd("QUICK EDIT");
    await app.closeActiveTab(); // within the 600 ms debounce
    await eventually(() => v.read("A.md") === "alpha\nQUICK EDIT", { message: "flushed on close" });
  } finally {
    await app.close();
  }
});

test(
  "text typed after the conflict banner appeared survives clicking the tab's X",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
    try {
      await app.insertEnd("first words ");
      v.write("A.md", "THEIRS\n");
      await app.waitBanner();
      await app.insertEnd(PARA); // the banner does not block editing
      await sleep(1500); // several autosave periods: nothing is written
      assert.equal(v.read("A.md"), "THEIRS\n", "save stays blocked while the banner is up");
      const before = await app.tabs();
      assert.ok(before.some((t) => t.path === "A.md" && t.dirty), "tab shows the unsaved dot");
      await app.closeActiveTab();
      await sleep(600);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const tabs = await app.tabs();
      if (!asked && !tabs.some((t) => t.path === "A.md")) {
        // Reopen the note: what does the user get back?
        await app.openFromTree("A.md");
        const ed = await app.editorText();
        assert.ok(
          ed.includes("A WHOLE PARAGRAPH") || diskHas(v, "A WHOLE PARAGRAPH"),
          `paragraph gone, no prompt: editor=${JSON.stringify(ed)} disk=${JSON.stringify(v.listDisk())}`,
        );
      }
    } finally {
      await app.close();
    }
  },
);

test(
  "deleted-outside-Cairn banner: the tab's X asks first, like the banner's own 'Discard and close'",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
    try {
      await app.insertEnd("MY IMPORTANT EDIT");
      v.rm("A.md");
      assert.match(await app.waitBanner(), /deleted or moved/);
      await app.closeActiveTab();
      await sleep(800);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillOpen = (await app.tabs()).some((t) => t.path === "A.md");
      assert.ok(
        asked || stillOpen || diskHas(v, "MY IMPORTANT EDIT"),
        `tab closed with no prompt and the edit is nowhere: disk=${JSON.stringify(v.listDisk())}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "single click on the vault name in the status bar with a conflicted tab asks first (or keeps the edits)",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
    try {
      await app.insertEnd("first words ");
      v.write("A.md", "THEIRS\n");
      await app.waitBanner();
      await app.insertEnd(PARA);
      await app.s.click(await app.s.find('button.vault[title="Switch vault"]'));
      await sleep(800);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillInVault = await app.exec(`return !!document.querySelector('[data-testid=file-tree]')`);
      assert.ok(
        asked || stillInVault || diskHas(v, "A WHOLE PARAGRAPH"),
        `vault closed with no prompt; disk A=${JSON.stringify(v.read("A.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "middle-click on a tab whose save failed (read-only folder) asks first (or keeps the edits)",
  async () => {
    const v = env.vault("v", { "d/A.md": "alpha\n", "B.md": "b\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("d/A.md");
      await app.source();
      fs.chmodSync(v.p("d"), 0o555);
      await app.insertEnd(PARA);
      await eventually(async () => (await app.toasts()).some((t) => /Could not save/.test(t)), { message: "error toast" });
      await app.exec(
        `document.querySelector('[data-testid=tab][aria-selected=true]').dispatchEvent(new MouseEvent('auxclick', { bubbles: true, button: 1 }))`,
      );
      await sleep(600);
      fs.chmodSync(v.p("d"), 0o755);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillOpen = (await app.tabs()).some((t) => t.path === "d/A.md");
      assert.ok(
        asked || stillOpen || diskHas(v, "A WHOLE PARAGRAPH"),
        `tab closed with no prompt; disk=${JSON.stringify(v.read("d/A.md"))}`,
      );
    } finally {
      try {
        fs.chmodSync(v.p("d"), 0o755);
      } catch {}
      await app.close();
    }
  },
);
