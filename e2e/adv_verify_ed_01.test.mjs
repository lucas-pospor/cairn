// Reproduction for FINDING-037 (undo after an external reload).
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ed_01.test.mjs
//
// 1. In a long note the external change is off screen: Ctrl+Z must not
//    revert it (if it did, the external lines would be deleted on disk
//    600 ms later with nothing visible near the cursor).
// 2. Ctrl+Z undoes only the user's own edit, Ctrl+Shift+Z redoes it, and the
//    external line stays on disk throughout.

import { test } from "node:test";
import assert from "node:assert/strict";
import { withApp, eventually, sleep, Key } from "./adv_editor_lib.mjs";

const LONG = "first line\n" + Array.from({ length: 300 }, (_, i) => `filler line ${i} with some words in it`).join("\n") + "\n";

test(
  "FINDING-037: Ctrl+Z after an off-screen external change keeps the external line on disk",
  async () => {
    await withApp({ "Long.md": LONG }, async (app, env) => {
      await app.open("Long.md");
      await app.setSel(10); // end of "first line"
      await app.keys(" mine");
      const mine = LONG.replace("first line", "first line mine");
      await eventually(() => env.vault.read("Long.md") === mine, { message: "autosave of my edit" });
      await sleep(800);
      // Sync / another device appends a line at the bottom of the note.
      env.vault.write("Long.md", mine + "EXTERNAL LINE AT THE BOTTOM\n");
      await eventually(async () => (await app.text()).includes("EXTERNAL LINE"), { message: "clean tab reloaded" });
      await sleep(300);
      await app.chord(Key.ctrl, "z");
      await sleep(1500);
      const disk = env.vault.read("Long.md");
      const info = await app.exec(`
        const v = document.querySelector('.cm-editor').__cairnView;
        const r = v.scrollDOM.getBoundingClientRect();
        const end = v.coordsAtPos(v.state.doc.length);
        const head = v.state.selection.main.head;
        return { head, line: v.state.doc.lineAt(head).text, endVisible: !!end && end.top < r.bottom && end.bottom > r.top,
                 banner: document.querySelector('[data-testid=conflict-banner]')?.textContent ?? null };`);
      assert.ok(
        disk.includes("EXTERNAL LINE"),
        `external line removed from disk with no notice. cursor line: ${JSON.stringify(info.line)}, end of note visible: ${info.endVisible}, banner: ${info.banner}`,
      );
    }, { shot: "ED-01-verify-offscreen" });
  },
);

test("FINDING-037: Ctrl+Z and Ctrl+Shift+Z undo and redo only the user's own edit; the external line stays on disk", async () => {
  await withApp({ "Note.md": "line one\n" }, async (app, env) => {
    await app.open("Note.md");
    await app.setSel(8);
    await app.keys(" mine");
    await eventually(() => env.vault.read("Note.md") === "line one mine\n", { message: "autosave of my edit" });
    await sleep(800);
    env.vault.write("Note.md", "line one mine\nEXTERNAL LINE FROM ANOTHER DEVICE\n");
    await eventually(async () => (await app.text()).includes("EXTERNAL LINE"), { message: "clean tab reloaded" });
    await sleep(300);
    await app.chord(Key.ctrl, "z");
    await eventually(() => env.vault.read("Note.md") === "line one\nEXTERNAL LINE FROM ANOTHER DEVICE\n", { message: "undo of my edit saved with the external line" });
    await app.chord(Key.ctrl, Key.shift, "z");
    await eventually(() => env.vault.read("Note.md") === "line one mine\nEXTERNAL LINE FROM ANOTHER DEVICE\n", { message: "redo of my edit saved with the external line" });
  });
});
