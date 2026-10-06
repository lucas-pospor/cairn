// Regression tests for FINDING-010 (see also adv_verify_dl_10.test.mjs): how
// narrow is the trigger, and can the user see what "Save my version" is
// about to replace?
//
// 1. Control (expected to pass): a quick delete + recreate (git checkout,
//    an editor's unlink-and-write) inside the watcher debounce never shows the
//    "deleted" banner. The tab gets the "changed on disk" banner, whose button
//    says "overwrite", and nothing is lost. So the bug needed a delete and a
//    recreate that land in separate scans.
// 2. Once the stale "deleted" banner is up and the file has come back,
//    clicking that file in the tree must show the file that is on disk now,
//    or the banner must stop saying "deleted" (with the defect, the click
//    only re-activated the stale tab with the user's own editor text).
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_10_02.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { Env, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("verify-dl10-02");
});
after(async () => {
  await env?.dispose();
});

test("control: a quick delete + recreate while typing gives the 'changed on disk' banner, not 'deleted'", async () => {
  const v = env.vault("v", { "N.md": "original\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("N.md");
    await app.source();
    await sleep(150);
    await app.typeEnd("MINE");
    // unlink + write a few ms apart, like git checkout or vim without backupcopy
    fs.unlinkSync(v.p("N.md"));
    fs.writeFileSync(v.p("N.md"), "QUICK-REPLACE\n");
    const b = await app.waitBanner();
    await sleep(1500);
    const b2 = await app.banner();
    console.log(`banner: ${JSON.stringify(b2)}; disk=${JSON.stringify(v.read("N.md"))}`);
    assert.match(b2 ?? b, /changed on disk/);
    assert.equal(v.read("N.md"), "QUICK-REPLACE\n");
  } finally {
    await app.close();
  }
});

test(
  "stale 'deleted' banner: clicking the recreated file in the tree shows the file on disk or clears the 'deleted' banner",
  async () => {
    const v = env.vault("v", { "N.md": "original\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("N.md");
      await app.source();
      await sleep(150);
      await app.typeEnd("MINE");
      v.rm("N.md");
      assert.match(await app.waitBanner(), /deleted or moved/);
      await sleep(1500);
      const marker = `BACK-${process.pid}-${Date.now()}`;
      fs.writeFileSync(v.p(".N.md.tmp"), `${marker}\n`);
      fs.renameSync(v.p(".N.md.tmp"), v.p("N.md"));
      await sleep(2000);
      // The user notices N.md in the tree again and clicks it to look.
      await app.exec(`document.querySelector('[data-testid=tree-row][data-path="N.md"]').click()`);
      await sleep(800);
      const tabs = await app.tabs();
      const text = await app.editorText();
      const b = await app.banner();
      console.log(`tabs=${JSON.stringify(tabs)} editor=${JSON.stringify(text)} banner=${JSON.stringify(b)} disk=${JSON.stringify(v.read("N.md"))}`);
      assert.ok(
        text.includes(marker) || !/deleted or moved/.test(b ?? ""),
        `the app still claims N.md is deleted and shows only the editor text (${JSON.stringify(text)}) while disk has ${JSON.stringify(v.read("N.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);
