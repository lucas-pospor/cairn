// Reproduction for FINDING-003 in the real desktop app: F2-renaming a.md to
// "A" when a different A.md exists replaces A.md.
// Also checks where the overwritten note went: the vault's .trash folder and
// the XDG system trash ($XDG_DATA_HOME/Trash, which the desktop app uses for
// deletes) are both searched, as is the rest of the temp directory.
//   scripts/e2e-headless.sh e2e/adv_verify_fs_01.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-fs01-"));
const vault = path.join(tmp, "vault");
const dataHome = path.join(tmp, "data");
const PRECIOUS = "UPPER CASE NOTE, precious";

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}

function holders(dir, needle, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) holders(p, needle, out);
    else if (e.isFile()) {
      try {
        if (fs.readFileSync(p, "utf8").includes(needle)) out.push(path.relative(tmp, p));
      } catch {}
    }
  }
  return out;
}

write("Start.md", "# Start\n");
write("a.md", "lower case note\n");
write("A.md", `${PRECIOUS}\n`);

let drv, s;

before(async () => {
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: dataHome,
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 3`, { timeout: 15000 });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  await sleep(1000);
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 3, retryDelay: 200 });
});

test(
  "FINDING-003: F2 rename a.md -> A keeps the other A.md (or at least trashes it)",
  async () => {
    await s.click(await s.find(`[data-testid=tree-row][data-path="a.md"]`));
    await sleep(100);
    await s.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
    await s.keys(Key.f2);
    const input = await s.find("[data-testid=rename-input]");
    await s.exec(`document.querySelector('[data-testid=rename-input]').select()`);
    await s.type(input, "A");
    await s.keys(Key.enter);
    await sleep(1500);
    const upper = fs.readFileSync(path.join(vault, "A.md"), "utf8");
    const rows = await s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map((r) => r.dataset.path)`);
    const toasts = await s.exec(`return [...document.querySelectorAll('[role=alert], [role=status], .toast')].map((t) => t.innerText).join(' | ')`);
    const where = holders(tmp, PRECIOUS);
    assert.equal(
      upper,
      `${PRECIOUS}\n`,
      `A.md now ${JSON.stringify(upper)}; a.md exists: ${fs.existsSync(path.join(vault, "a.md"))}; ` +
        `tree rows: ${JSON.stringify(rows)}; toasts: ${JSON.stringify(toasts)}; ` +
        `vault .trash exists: ${fs.existsSync(path.join(vault, ".trash"))}; ` +
        `XDG Trash exists: ${fs.existsSync(path.join(dataHome, "Trash"))}; ` +
        `files anywhere under the temp dir holding the old A.md text: ${JSON.stringify(where)}`,
    );
  },
);
