// Reproductions for FINDING-131 (in-app delete closes dirty tabs
// with skipSave before calling deleteEntry).
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_14.test.mjs
//
// 1. Control (expected to pass): the autosave timer (600 ms) keeps running
//    while the confirm dialog is open, so a user who takes longer than 600 ms
//    from the last keystroke to clicking "Delete" gets the latest text in the
//    trash. The latest text is lost only when the whole delete + confirm
//    happens within 600 ms of the last keystroke.
// 2. Failed delete: in a read-only folder the autosave has already failed
//    with a "Could not save" toast, so the tab is the only copy of the
//    edits. Deleting the note still closes that tab before the delete is
//    attempted; the delete then fails and the edits are gone.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { Env, eventually, sleep, Key } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl14");
});
after(async () => {
  await env?.dispose();
});

function trashCopies(v, prefix) {
  const trashDir = `${env.tmp}/data/Trash/files`;
  const copies = fs.existsSync(trashDir)
    ? fs.readdirSync(trashDir).filter((f) => f.startsWith(prefix)).map((f) => fs.readFileSync(`${trashDir}/${f}`, "utf8"))
    : [];
  const vaultTrash = v.exists(".trash") ? fs.readdirSync(v.p(".trash")).map((f) => v.read(`.trash/${f}`)) : [];
  return [...copies, ...vaultTrash];
}

test("control: when the confirm dialog stays open past the autosave delay, the trash has the latest text", async () => {
  const v = env.vault("v", { "Del2.md": "saved text\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("Del2.md");
    await app.source();
    await app.insertEnd("typed just now");
    await app.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
    await app.s.keys(Key.delete);
    const ok = await app.s.findWait("[data-testid=dialog-ok]");
    await sleep(1000); // a human reading the dialog
    await app.s.click(ok);
    await eventually(() => !v.exists("Del2.md"), { message: "deleted" });
    await sleep(800);
    const all = trashCopies(v, "Del2");
    assert.ok(all.length > 0, "no trash copy found");
    assert.ok(all.some((t) => t.includes("typed just now")), `trash has only ${JSON.stringify(all)}`);
  } finally {
    await app.close();
  }
});

test(
  "read-only folder: autosave already failed, then a failed in-app delete still discards the tab and its edits",
  async () => {
    const v = env.vault("v", { "ro/Del.md": "saved text\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("ro/Del.md");
      await app.source();
      await sleep(300);
      fs.chmodSync(v.p("ro"), 0o555);
      await app.insertEnd(" UNSAVED");
      // Wait for the autosave to fail, as a user would see it.
      const saveToast = await eventually(async () => (await app.toasts()).find((t) => /Could not save/.test(t)), {
        timeout: 5000,
        message: "Could not save toast",
      });
      assert.equal(v.read("ro/Del.md"), "saved text\n", "precondition: edits are not on disk");
      await app.exec(`document.querySelector('[data-testid=tree-row][data-path="ro/Del.md"]').click()`);
      await app.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
      await app.s.keys(Key.delete);
      await app.s.click(await app.s.findWait("[data-testid=dialog-ok]"));
      await sleep(1000);
      const toasts = await app.toasts();
      const tabs = await app.tabs();
      assert.ok(v.exists("ro/Del.md"), "precondition: the delete failed");
      assert.ok(
        tabs.some((t) => t.path === "ro/Del.md"),
        `autosave toast ${JSON.stringify(saveToast)}; delete failed (${JSON.stringify(toasts)}) but the tab holding the only copy of " UNSAVED" was closed; disk=${JSON.stringify(v.read("ro/Del.md"))}`,
      );
    } finally {
      fs.chmodSync(v.p("ro"), 0o755);
      await app.close();
    }
  },
);
