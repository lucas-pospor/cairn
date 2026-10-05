// Not a defect (by design), checked through the REAL UI (not raw IPC):
// a vault holds `linked -> <outside>/dir`. The tree shows linked/victim.md as
// an ordinary vault note. The user selects it, presses Delete and confirms the
// dialog. Where does the file go?
//
// The first test asserts the stricter expectation (the outside file stays
// put) and is marked todo: by design it fails, the file goes to the trash.
// The second test checks the impact: the dialog promised "the trash", and the
// file is in the freedesktop trash (XDG_DATA_HOME/Trash, isolated to a temp
// dir) with a .trashinfo naming its real original path, so it is restorable.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_as_05_ui.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-vas05ui-"));
const vault = path.join(tmp, "vault");
const outsideDir = path.join(tmp, "outside", "dir");
const dataHome = path.join(tmp, "data");
const victim = path.join(outsideDir, "victim.md");

let drv, s;
let dialogText = "";

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;

before(async () => {
  fs.mkdirSync(outsideDir, { recursive: true });
  fs.writeFileSync(victim, "please-keep-me\n");
  fs.writeFileSync(path.join(outsideDir, "keep.md"), "keep\n");
  fs.mkdirSync(vault, { recursive: true });
  fs.writeFileSync(path.join(vault, "Welcome.md"), "# Welcome\n");
  fs.symlinkSync(outsideDir, path.join(vault, "linked"));
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: dataHome,
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 20000 });

  // Real UI flow: expand the linked folder, select the note, Delete, confirm.
  await s.click(await s.findWait(rowSel("linked")));
  await s.click(await s.findWait(rowSel("linked/victim.md")));
  await sleep(200);
  await s.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
  await s.keys(Key.delete);
  const ok = await s.findWait("[data-testid=dialog-ok]");
  dialogText = await s.exec(`return document.querySelector('[data-testid=dialog-ok]').closest('div').parentElement.innerText`);
  await s.click(ok);
  await s.waitFor(`return !document.querySelector('${rowSel("linked/victim.md")}')`);
  await sleep(300);
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test(
  "not a defect (by design): deleting a note shown under a symlinked folder leaves the outside file in place",
  { todo: "not a defect (by design): the outside file leaves its location (it goes to the OS trash, see next test)" },
  () => {
    console.log(`dialog: ${JSON.stringify(dialogText)}`);
    assert.ok(fs.existsSync(victim), "the outside file left its original location");
  },
);

test("not a defect (by design), impact: the confirmed delete went to the OS trash and is restorable", () => {
  const trashed = path.join(dataHome, "Trash", "files", "victim.md");
  const infoFile = path.join(dataHome, "Trash", "info", "victim.md.trashinfo");
  const info = fs.existsSync(infoFile) ? fs.readFileSync(infoFile, "utf8") : "(no trashinfo)";
  console.log(`dialog: ${JSON.stringify(dialogText)}\ntrashed: ${fs.existsSync(trashed)}\n${info}`);
  assert.match(dialogText, /trash/i, "the confirmation says the file goes to the trash");
  assert.equal(fs.readFileSync(trashed, "utf8"), "please-keep-me\n");
  assert.ok(info.includes(`Path=${fs.realpathSync(outsideDir)}/victim.md`), info);
  assert.equal(fs.existsSync(path.join(vault, ".trash")), false, "the vault-trash fallback was not used");
  assert.ok(fs.existsSync(path.join(outsideDir, "keep.md")), "siblings untouched");
});
