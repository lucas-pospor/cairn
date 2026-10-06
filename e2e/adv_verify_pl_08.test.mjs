// Regression test for FINDING-071 (notes.write overwrote existing
// Markdown files inside hidden folders), against the real app.
//
//   scripts/e2e-headless.sh e2e/adv_verify_pl_08.test.mjs
//
// A plugin with only the "write" permission ("create and change notes")
// targets a deleted note's recovery copy in the vault trash (.trash/), a
// Markdown file in .cairn/ and one in .github/. Creating a new file in a hidden
// folder is refused (control). The test checks the files on disk, and also
// that the write does not pull the hidden folder into the file tree (with the
// defect the backend emitted Created changes for the hidden entries, see
// FINDING-135).

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-pl08-"));
let drv, s;
let n = 0;

function makeVault(name, files) {
  const dir = path.join(tmp, `${++n}-${name}`);
  const v = {
    dir,
    write(rel, c) {
      const p = path.join(dir, rel);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, c);
    },
    read: (rel) => fs.readFileSync(path.join(dir, rel), "utf8"),
    exists: (rel) => fs.existsSync(path.join(dir, rel)),
  };
  fs.mkdirSync(dir, { recursive: true });
  for (const [k, c] of Object.entries(files)) v.write(k, c);
  return v;
}

async function eventually(fn, { timeout = 8000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      const r = await fn();
      if (r) return r;
    } catch {}
    await sleep(100);
  }
  throw new Error(`timed out: ${message}`);
}

async function escapeAll() {
  for (let i = 0; i < 3; i++) {
    await s.keys(Key.escape);
    await sleep(60);
  }
}

async function switchTo(v) {
  await escapeAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Switch vault");
  await sleep(150);
  await s.keys(Key.enter);
  const input = await s.findWait("[data-testid=vault-path]", 8000);
  await s.type(input, v.dir);
  await s.click(await s.find("[data-testid=vault-open]"));
  await s.waitFor(`return !document.querySelector('[data-testid=vault-path]') && document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
}

async function enablePlugin(file) {
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
  await s.findWait(`[data-testid=plugin-row][data-file="${file}"]`);
  await s.click(await s.find(`[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`));
  await s.click(await s.findWait("[data-testid=dialog-ok]"));
  await sleep(300);
}

before(async () => {
  const home = makeVault("home", { "Home.md": "# Home\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }) });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [home.dir]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("a write-only plugin cannot overwrite trashed notes or other hidden Markdown files", async () => {
  const v = makeVault("hidden-write", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/notes.md": "config-folder note\n",
    ".github/README.md": "repo readme\n",
    ".trash/Deleted plan.md": "the only remaining copy of a deleted note\n",
    ".cairn/plugins/hw.js": `// @name HW
// @permissions write
(async () => {
  const r = {};
  for (const p of [".trash/Deleted plan.md", ".cairn/notes.md", ".github/README.md", ".trash/brand-new.md"]) {
    try { await cairn.notes.write(p, "overwritten by plugin\\n"); r[p] = "ok"; } catch (e) { r[p] = "ERR " + e.message; }
  }
  self.__r = r;
  await cairn.ui.toast("HWDONE " + JSON.stringify(r));
})();
`,
  });
  await switchTo(v);
  await enablePlugin("hw.js");
  const toast = await eventually(
    () => s.exec(`return [...document.querySelectorAll('.toast')].map((t) => t.textContent).find((t) => t.includes('HWDONE')) ?? null`),
    { message: "plugin result toast" },
  );
  await sleep(500);
  const hiddenRows = await s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map((r) => r.dataset.path).filter((p) => p && p.split('/').some((c) => c.startsWith('.')))`);
  await escapeAll();
  const disk = {
    trash: v.read(".trash/Deleted plan.md"),
    cairn: v.read(".cairn/notes.md"),
    github: v.read(".github/README.md"),
    newHidden: v.exists(".trash/brand-new.md"),
  };
  const detail = JSON.stringify({ toast, disk, hiddenRows });
  // Control: creating a new hidden file is refused, so the policy exists.
  assert.equal(disk.newHidden, false, detail);
  assert.match(toast, /brand-new\.md":"ERR/, detail);
  // The finding: existing hidden Markdown files must not be replaced.
  assert.equal(disk.trash, "the only remaining copy of a deleted note\n", detail);
  assert.equal(disk.cairn, "config-folder note\n", detail);
  assert.equal(disk.github, "repo readme\n", detail);
  assert.deepEqual(hiddenRows, [], detail);
});
