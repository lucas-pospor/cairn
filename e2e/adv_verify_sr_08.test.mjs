// FINDING-056 (app level): reopening the same vault while its
// sync is running starts a second SyncEngine on the same state folder.
// open_vault calls old.stop(), which only sets a flag; the old sync keeps
// running while the new SyncManager loads the (half-written) state and starts
// its own first sync 2 s later. Both upload the same new notes under
// different file ids, so every other device gets a conflict copy of each.
//
// Flow, all through the real UI: connect sync, drop many new notes into the
// vault (the watcher starts a sync), and while the status bar says "syncing"
// click "Switch vault" and then the same vault under "Recent".
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_sr_08.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = path.join(ROOT, "target/debug/cairn");
const SERVER = path.join(ROOT, "target/debug/cairn-server");
const SYNC_DIR = path.join(ROOT, "target/debug/examples/sync_dir");
const TOKEN = "e2e-token-0123456789abcdef";
const PASS = "e2e passphrase for sync";
const PORT = 18000 + Math.floor(Math.random() * 1000);
const URL = `http://127.0.0.1:${PORT}`;
const N = Number(process.env.SR08_NOTES ?? 2500);

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-sr08-"));
const vaultA = path.join(tmp, "laptop");
const vaultB = path.join(tmp, "phone");
const stateB = path.join(tmp, "phone-state");

let server, drv, s;

function writeA(rel, c) {
  fs.mkdirSync(path.dirname(path.join(vaultA, rel)), { recursive: true });
  fs.writeFileSync(path.join(vaultA, rel), c);
}

function syncB() {
  return JSON.parse(execFileSync(SYNC_DIR, [vaultB, stateB, URL, TOKEN, "e2e", "phone", PASS], { encoding: "utf8" }));
}

function walk(dir, base = dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name.startsWith(".")) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...walk(p, base));
    else out.push(path.relative(base, p));
  }
  return out;
}

/** Number of distinct file ids the server holds for the vault. */
async function serverFileCount() {
  let since = 0;
  const ids = new Set();
  for (;;) {
    const r = await fetch(`${URL}/v1/vaults/e2e/changes?since=${since}&limit=2000`, { headers: { authorization: `Bearer ${TOKEN}` } });
    const j = await r.json();
    for (const h of j.heads) ids.add(h.file_id);
    since = j.cursor;
    if (!j.more) break;
  }
  return ids.size;
}

async function eventually(fn, { timeout = 10000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e;
    }
    await sleep(100);
  }
  throw new Error(`timed out: ${message} (last: ${last})`);
}

async function setInput(testid, value) {
  await s.exec(
    `const el = document.querySelector('[data-testid=${testid}]'); el.value = arguments[0]; el.dispatchEvent(new Event('input', { bubbles: true }));`,
    value,
  );
}

before(async () => {
  fs.mkdirSync(vaultA, { recursive: true });
  writeA("Seed.md", "seed\n");
  server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: TOKEN, CAIRN_DATA: path.join(tmp, "server"), CAIRN_ADDR: `127.0.0.1:${PORT}` },
    stdio: "ignore",
  });
  await eventually(async () => (await fetch(`${URL}/health`)).ok, { message: "server up" });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vaultA]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  server?.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test(
  "reopening the vault from Recent while a sync runs does not upload every new note twice",
  async () => {
    // connect sync from settings
    await s.click(await s.find("[data-testid=open-settings]"));
    await s.click(await s.findWait("[data-testid=settings-sync]"));
    await s.findWait("[data-testid=sync-server]");
    await setInput("sync-server", URL);
    await setInput("sync-token", TOKEN);
    await setInput("sync-vault", "e2e");
    await setInput("sync-device", "laptop");
    await setInput("sync-pass", PASS);
    await setInput("sync-pass2", PASS);
    await s.click(await s.find("[data-testid=sync-connect]"));
    await s.click(await s.findWait("[data-testid=dialog-ok]")); // a new vault: "Create it?"
    await s.waitFor(`return document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle'`, { timeout: 30000 });
    await s.exec(`document.querySelector('[data-testid=settings] .close').click()`);
    await sleep(2500); // let the manager's own first scheduled sync pass

    // many new notes arrive (copied in, or written by another tool)
    for (let i = 0; i < N; i++) writeA(`bulk/${String(i).padStart(5, "0")}.md`, `note ${i}\n`);
    // the watcher pokes sync; wait until it is uploading
    await s.waitFor(`return document.querySelector('[data-testid=sync-indicator]')?.classList.contains('syncing')`, { timeout: 30000 });
    const uploadedWhenReopened = await serverFileCount();

    // "Switch vault", then the same vault from the Recent list
    await s.exec(`document.querySelector('footer.status button.vault').click()`);
    await s.waitFor(`return !!document.querySelector('button.recent-open[title="${vaultA}"]')`, { timeout: 10000 });
    await s.exec(`document.querySelector('button.recent-open[title="${vaultA}"]').click()`);
    await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });

    // wait until the server stops receiving uploads
    async function settle() {
      let prev = -1;
      let stable = 0;
      const end = Date.now() + 180000;
      while (Date.now() < end && stable < 6) {
        await sleep(1000);
        const n = await serverFileCount();
        stable = n === prev ? stable + 1 : 0;
        prev = n;
      }
      return prev;
    }
    const afterRace = await settle();
    const errors = (drv.log().match(/sync failed: [^\n]*/g) ?? []).slice(0, 3);
    // let the (new) manager finish: "Sync now" in the status bar, twice
    for (let k = 0; k < 2; k++) {
      await s.exec(`document.querySelector('[data-testid=sync-indicator]').click()`);
      await sleep(500);
      await s.waitFor(`return !document.querySelector('[data-testid=sync-indicator]')?.classList.contains('syncing')`, { timeout: 120000 });
    }
    const onServer = await settle();
    const copiesA = walk(vaultA).filter((f) => f.includes("(conflict"));

    syncB();
    syncB();
    const filesB = walk(vaultB);
    const copiesB = filesB.filter((f) => f.includes("(conflict"));
    const log = drv
      .log()
      .split("\n")
      .filter((l) => /sync/.test(l))
      .slice(-8)
      .join("\n");
    assert.ok(
      onServer === N + 1 && copiesB.length === 0,
      `notes: ${N + 1}; uploaded when the vault was reopened: ${uploadedWhenReopened}; file ids on the server after the race: ${afterRace}, ` +
        `after two more syncs: ${onServer}; conflict copies in the app's vault: ${copiesA.length}; ` +
        `second device has ${filesB.length} files, ${copiesB.length} conflict copies (e.g. ${copiesB.slice(0, 2).join(", ")}); ` +
        `sync errors during the race: ${JSON.stringify(errors)}\n${log}`,
    );
  },
);
