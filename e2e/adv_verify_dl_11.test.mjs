// Not a defect (by design): an external delete followed by a recreate more
// than about 250-312 ms later (one notify-debouncer-full timeout plus one
// tick) reaches the UI as two batches: Deleted, then Created. handleChanges
// closes the clean tab on Deleted and ignores Created, so the tab does not
// come back.
//
// The controls show where the line is: the same replace with a 100 ms gap,
// and real editor saves (vim, perl -i), keep the tab.
//
// Closing a clean tab on an external delete is intended (e2e/app.test.mjs
// "reflects files created, changed and deleted outside the app" asserts
// it). The todo cases record the alternative behaviour; the 2000 ms case
// shows it amounts to "keep tabs of deleted files open", which is a design
// change rather than a bug fix.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_11.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { Env, sleep, eventually } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("verify-dl11");
});
after(async () => {
  await env?.dispose();
});

async function replaceWithGap(gapMs) {
  const v = env.vault("v", { "N.md": "original\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("N.md");
    await sleep(400);
    v.rm("N.md");
    await sleep(gapMs);
    v.write("N.md", "replaced\n");
    await sleep(1500);
    const tabs = await app.tabs();
    const inIndex = (await app.invoke("list_entries")).ok.some((e) => e.path === "N.md");
    return { tabs, inIndex, disk: v.read("N.md") };
  } finally {
    await app.close();
  }
}

test("control: delete + recreate 100 ms later keeps the clean tab and reloads it", async () => {
  const r = await replaceWithGap(100);
  assert.ok(r.inIndex);
  assert.deepEqual(r.tabs.map((t) => t.path), ["N.md"]);
});

test("control: real vim and perl -i saves keep the clean tab", async () => {
  const v = env.vault("v", { "N.md": "original\n", "P.md": "x\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("N.md");
    await app.openFromTree("P.md", { newTab: true });
    await sleep(400);
    execFileSync("vim", ["-u", "NONE", "-N", "-es", "-c", "set backupcopy=no writebackup", "-c", "%s/original/vimmed/", "-c", "wq", "N.md"], { cwd: v.root });
    execFileSync("perl", ["-pi", "-e", "s/x/perled/", "P.md"], { cwd: v.root });
    await sleep(1500);
    assert.deepEqual((await app.tabs()).map((t) => t.path), ["N.md", "P.md"]);
    await eventually(async () => (await app.editorText()) === "perled\n", { message: "perl -i save reloaded" });
  } finally {
    await app.close();
  }
});

for (const gap of [400, 600, 2000]) {
  test(
    `delete + recreate ${gap} ms later keeps the clean tab`,
    { todo: "not a defect (by design): Deleted closes the clean tab; a later Created for the same path is ignored" },
    async () => {
      const r = await replaceWithGap(gap);
      assert.ok(r.inIndex, "file is back in the index");
      assert.equal(r.disk, "replaced\n");
      assert.ok(r.tabs.some((t) => t.path === "N.md"), `tab closed although the file is back: ${JSON.stringify(r.tabs)}`);
    },
  );
}
