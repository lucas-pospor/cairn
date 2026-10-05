// Renames in the app refuse a name that differs only in case from another
// entry next to it, as creating one does; a case-only rename of the entry
// itself still works.
//   scripts/e2e-headless.sh e2e/rename_case_twin.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-case-rename-"));
const vault = path.join(tmp, "vault");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
const names = () => fs.readdirSync(vault).filter((n) => !n.startsWith(".")).sort();

async function eventually(fn, { timeout = 5000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let err;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      err = e;
    }
    await sleep(80);
  }
  throw new Error(`timed out: ${message}${err ? ` (${err.message})` : ""}`);
}

write("Note.md", "the real note\n");
write("Untitled.md", "fresh\n");

let drv, s;

before(async () => {
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 2`, { timeout: 15000 });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  await sleep(1000);
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 3, retryDelay: 200 });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const toasts = () => s.exec(`return [...document.querySelectorAll('.toast')].map((t) => t.textContent).join(' | ')`);

async function f2Rename(from, name) {
  await s.click(await s.find(rowSel(from)));
  await sleep(100);
  await s.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
  await s.keys(Key.f2);
  const input = await eventually(() => s.find("[data-testid=rename-input]"), { message: "rename input" });
  await s.exec(`document.querySelector('[data-testid=rename-input]').select()`);
  await s.type(input, name);
  await s.keys(Key.enter);
}

test("F2 to a name that differs only in case from another note is refused", async () => {
  await f2Rename("Untitled.md", "note");
  const shown = await eventually(async () => (await toasts()).includes("already exists") && (await toasts()), { message: "error toast" });
  assert.ok(shown.includes(`Something named "Note.md" already exists.`), shown);
  assert.deepEqual(names(), ["Note.md", "Untitled.md"]);
  assert.equal(fs.readFileSync(path.join(vault, "Note.md"), "utf8"), "the real note\n");
});

test("F2 to another case of the note's own name still renames it", async () => {
  await f2Rename("Note.md", "note");
  await eventually(() => names().includes("note.md"), { message: "renamed on disk" });
  assert.deepEqual(names(), ["Untitled.md", "note.md"]);
  assert.equal(fs.readFileSync(path.join(vault, "note.md"), "utf8"), "the real note\n");
});
