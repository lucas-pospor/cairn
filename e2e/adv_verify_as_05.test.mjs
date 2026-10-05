// Not a defect (by design), checked in the REAL app: delete_entry on a file
// inside a symlinked folder (linked -> <outside>/dir) moves that file into the
// OS trash (XDG_DATA_HOME/Trash, isolated to a temp dir here), and the
// .trashinfo records the original outside path. Nothing is permanently deleted,
// so the delete does not lose data.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_as_05.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-vas05-"));
const vault = path.join(tmp, "vault");
const outside = path.join(tmp, "outside");
const dataHome = path.join(tmp, "data");

let drv, s;

before(async () => {
  fs.mkdirSync(path.join(outside, "dir"), { recursive: true });
  fs.writeFileSync(path.join(outside, "dir", "victim.md"), "please-keep-me\n");
  fs.mkdirSync(vault, { recursive: true });
  fs.writeFileSync(path.join(vault, "Welcome.md"), "# Welcome\n");
  fs.symlinkSync(path.join(outside, "dir"), path.join(vault, "linked"));
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: dataHome,
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 20000 });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

async function invoke(cmd, args) {
  const r = await s.execAsync(
    `const cb = arguments[arguments.length-1];
     window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {})
       .then(v => cb(JSON.stringify({ ok: v === undefined ? null : v })))
       .catch(e => cb(JSON.stringify({ err: String((e && e.message) || e) })));`,
    cmd,
    args ?? {},
  );
  return JSON.parse(r);
}

test("not a defect (by design): deleting through a symlinked folder moves the file to the OS trash, restorable", async () => {
  const r = await invoke("delete_entry", { path: "linked/victim.md" });
  assert.ok(r.ok, `delete failed: ${JSON.stringify(r)}`);
  const orig = path.join(outside, "dir", "victim.md");
  assert.equal(fs.existsSync(orig), false, "file left its original location");
  const trashed = path.join(dataHome, "Trash", "files", "victim.md");
  const info = fs.readFileSync(path.join(dataHome, "Trash", "info", "victim.md.trashinfo"), "utf8");
  console.log(`trashed copy: ${fs.readFileSync(trashed, "utf8").trim()}\n${info}`);
  assert.equal(fs.readFileSync(trashed, "utf8"), "please-keep-me\n");
  assert.ok(info.includes(`Path=${fs.realpathSync(path.join(outside, "dir"))}/victim.md`), info);
  assert.equal(fs.existsSync(path.join(vault, ".trash", "victim.md")), false, "went to the OS trash, not the vault fallback");
});
