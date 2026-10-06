// Regression tests for FINDING-129: an external folder rename was reported
// as delete+create when a file inside the folder had a watcher event (even an
// IN_OPEN from a plain read) shortly before, so open tabs did not follow.
//
// 1. Control: nothing touched the folder before the rename -> tab follows.
// 2. Gap sweep: a read of dir/C.md, then the rename after various gaps, in one
//    app session; the tab must follow after every gap.
// 3. Cairn's own reads: an external write to the open (clean) note makes
//    Cairn re-read it (watcher rescan + tab reload), which queues a fresh
//    IN_OPEN. The tab must still follow a folder rename ~0.45 s after the
//    external write (with the defect, the tab did not follow it, although the
//    external write's own event was already flushed).
//
// Run:
//   RUST_LOG=info,cairn_app_lib=debug scripts/e2e-headless.sh --test-reporter=spec e2e/adv_verify_dl_09.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { Env, eventually, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("v09");
});
after(async () => {
  if (env) {
    const lines = env.log().split("\n").filter((l) => l.includes("external changes"));
    console.log("watcher log (external changes):\n" + lines.slice(-40).join("\n"));
  }
  await env?.dispose();
});

test("control: folder renamed with no recent activity inside: open tab follows", async () => {
  const v = env.vault("v", { "dir/C.md": "charlie\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("dir/C.md");
    await sleep(1000);
    v.mv("dir", "dir2");
    await sleep(1500);
    const tabs = (await app.tabs()).map((t) => t.path);
    assert.deepEqual(tabs, ["dir2/C.md"]);
  } finally {
    await app.close();
  }
});

test(
  "gap sweep: read a file in the folder, then rename the folder after N ms",
  async () => {
    const v = env.vault("v", { "d0/C.md": "charlie\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    const results = [];
    let cur = "d0";
    let n = 0;
    try {
      for (const gap of [0, 0, 0, 30, 30, 150, 150, 230, 230, 400, 400, 700]) {
        const tabs = (await app.tabs()).map((t) => t.path);
        if (!tabs.includes(`${cur}/C.md`)) {
          await app.closeAllTabs().catch(() => {});
          await app.openFromTree(`${cur}/C.md`);
        }
        await sleep(1000);
        fs.readFileSync(v.p(`${cur}/C.md`));
        if (gap) await sleep(gap);
        const next = `d${++n}`;
        v.mv(cur, next);
        cur = next;
        await sleep(1500);
        const after = (await app.tabs()).map((t) => t.path);
        results.push({ gap, followed: after.includes(`${cur}/C.md`), tabs: after });
      }
    } finally {
      console.log("gap sweep:", JSON.stringify(results));
      await app.close();
    }
    const broken = results.filter((r) => !r.followed);
    assert.deepEqual(broken, [], `tab did not follow for: ${JSON.stringify(broken.map((r) => r.gap))}`);
  },
);

test(
  "folder rename 450 ms after an external write (and Cairn's own re-read): the open tab follows",
  async () => {
    const v = env.vault("v", { "dir/C.md": "charlie\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("dir/C.md");
      await sleep(1000);
      v.write("dir/C.md", "charlie edited elsewhere\n");
      await sleep(450);
      v.mv("dir", "dir2");
      await sleep(1500);
      const tabs = (await app.tabs()).map((t) => t.path);
      assert.deepEqual(tabs, ["dir2/C.md"], `tab did not follow: ${JSON.stringify(tabs)}`);
      await eventually(async () => (await app.editorText()) === "charlie edited elsewhere\n", { message: "reloaded" });
    } finally {
      await app.close();
    }
  },
);
