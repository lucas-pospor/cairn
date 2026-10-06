// Regression tests for FINDING-002 (in addition to
// adv_verify_dl_02.test.mjs). Two extra routes to the same root cause
// (closeTab / closeVault dropped a tab whose flush() could not save):
//  1. The conflict happens on a BACKGROUND tab. The banner is only drawn for
//     the active tab, so the user never sees it: the only hint is the small
//     unsaved dot. Clicking that tab's X (which does not activate the tab)
//     must ask first or keep the edit.
//  2. A failed save (read-only folder, like a full disk or a lost mount),
//     then a single click on the vault name in the status bar. The app must
//     ask first, or the edit must survive reopening the vault.
// A control case shows the same background-tab close saves fine when nothing
// blocks the save.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_02_bgtab.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { Env, eventually, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl02bg");
});
after(async () => {
  await env?.dispose();
});

const tabSel = (p) => `[data-testid=tab][data-path="${p}"]`;

async function twoTabs(app) {
  await app.openFromTree("A.md");
  await app.source();
  await app.openFromTree("Z.md", { newTab: true });
  await app.s.click(await app.s.find(tabSel("A.md")));
  await app.waitLoaded("A.md");
}

test("control: closing a dirty background tab (save not blocked) saves it", async () => {
  const v = env.vault("v", { "A.md": "alpha\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await twoTabs(app);
    await app.insertEnd("BACKGROUND EDIT");
    await app.s.click(await app.s.find(tabSel("Z.md")));
    await app.exec(`document.querySelector(arguments[0] + ' .close').click()`, tabSel("A.md"));
    await eventually(() => v.read("A.md") === "alpha\nBACKGROUND EDIT", { message: "background tab flushed on close" });
  } finally {
    await app.close();
  }
});

test(
  "conflict on a background tab: closing it with X asks first (or keeps the edit) though no banner was ever shown",
  async () => {
    const v = env.vault("v", { "A.md": "alpha\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await twoTabs(app);
      await app.insertEnd("MY IMPORTANT EDIT");
      await app.s.click(await app.s.find(tabSel("Z.md"))); // look at the other note
      v.write("A.md", "THEIRS\n"); // e.g. sync or another program, inside the 600 ms autosave window
      await sleep(1500);
      assert.equal(await app.activeTab(), "Z.md");
      assert.equal(await app.banner(), null, "no banner is shown for the background tab");
      const tabs = await app.tabs();
      assert.ok(tabs.find((t) => t.path === "A.md")?.dirty, "only the unsaved dot hints at a problem");
      assert.equal(v.read("A.md"), "THEIRS\n", "save is blocked by the conflict");

      await app.exec(`document.querySelector(arguments[0] + ' .close').click()`, tabSel("A.md"));
      await sleep(600);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillOpen = (await app.tabs()).some((t) => t.path === "A.md");
      if (!asked && !stillOpen) {
        await app.openFromTree("A.md");
        const ed = await app.editorText();
        assert.ok(
          ed.includes("MY IMPORTANT EDIT") || v.read("A.md").includes("MY IMPORTANT EDIT"),
          `edit gone, no prompt, banner never shown: editor=${JSON.stringify(ed)} disk=${JSON.stringify(v.read("A.md"))}`,
        );
      }
    } finally {
      await app.close();
    }
  },
);

test(
  "failed save, then one click on the vault name in the status bar: the edit survives reopening the vault",
  async () => {
    const v = env.vault("v", { "d/A.md": "alpha\n", "B.md": "b\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("d/A.md");
      await app.source();
      fs.chmodSync(v.p("d"), 0o555);
      await app.insertEnd("MY IMPORTANT EDIT");
      await eventually(async () => (await app.toasts()).some((t) => /Could not save/.test(t)), { message: "error toast" });
      await app.s.click(await app.s.find('button.vault[title="Switch notebook"]'));
      await sleep(800);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillInVault = await app.exec(`return !!document.querySelector('[data-testid=file-tree]')`);
      fs.chmodSync(v.p("d"), 0o755); // the problem goes away (space freed, mount back)
      if (!asked && !stillInVault) {
        await app.s.findWait("[data-testid=vault-path]");
        await app.s.type(await app.s.find("[data-testid=vault-path]"), v.root);
        await app.s.click(await app.s.find("[data-testid=vault-open]"));
        await app.s.findWait("[data-testid=file-tree]");
        await sleep(800);
        const tabs = await app.tabs();
        let ed = "";
        if (tabs.some((t) => t.path === "d/A.md")) {
          await app.s.click(await app.s.find(tabSel("d/A.md")));
          await app.waitLoaded("d/A.md");
          ed = await app.editorText();
        }
        assert.ok(
          ed.includes("MY IMPORTANT EDIT") || v.read("d/A.md").includes("MY IMPORTANT EDIT"),
          `vault closed with no prompt and the edit is gone after reopening: tabs=${JSON.stringify(tabs)} editor=${JSON.stringify(ed)} disk=${JSON.stringify(v.read("d/A.md"))}`,
        );
      }
    } finally {
      try {
        fs.chmodSync(v.p("d"), 0o755);
      } catch {}
      await app.close();
    }
  },
);
