// End-to-end sync: the real app, a real cairn-server, and a second device
// driven by the `sync_dir` example.
// Build first:  cargo build -p cairn-server --example sync_dir -p cairn-sync
//               cd app && npx tauri build --debug --no-bundle

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = path.join(ROOT, "target/debug/cairn");
const SERVER = path.join(ROOT, "target/debug/cairn-server");
const SYNC_DIR = path.join(ROOT, "target/debug/examples/sync_dir");
const TOKEN = "e2e-token-0123456789abcdef";
const PASS = "e2e passphrase for sync";
const PORT = 18000 + Math.floor(Math.random() * 1000);
const URL = `http://127.0.0.1:${PORT}`;

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-sync-e2e-"));
const vaultA = path.join(tmp, "laptop");
const vaultB = path.join(tmp, "phone");
const stateB = path.join(tmp, "phone-state");

let server, drv, s;

function writeA(rel, c) {
  fs.mkdirSync(path.dirname(path.join(vaultA, rel)), { recursive: true });
  fs.writeFileSync(path.join(vaultA, rel), c);
}

function syncB() {
  const out = execFileSync(SYNC_DIR, [vaultB, stateB, URL, TOKEN, "e2e", "phone", PASS], { encoding: "utf8" });
  return JSON.parse(out);
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

before(async () => {
  fs.mkdirSync(vaultA, { recursive: true });
  writeA("Shared.md", "line one\nline two\nline three\n");
  writeA("Laptop only.md", "from the laptop\n");
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
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 2`, { timeout: 15000 });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  server?.kill();
  fs.rmSync(tmp, { recursive: true, force: true });
});

async function setInput(testid, value) {
  await s.exec(
    `const el = document.querySelector('[data-testid=${testid}]'); el.value = arguments[0]; el.dispatchEvent(new Event('input', { bubbles: true }));`,
    value,
  );
}

test("connect the app to a server from settings", async () => {
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
  // The server has no vault "e2e" yet: setup asks before creating it.
  const ok = await s.findWait("[data-testid=dialog-ok]");
  assert.equal(
    await s.exec(`return document.querySelector('[data-testid=dialog-ok]').closest('[role=dialog]').querySelector('p').textContent.trim()`),
    "There's no vault called e2e on this server. Create it?",
  );
  await s.click(ok);
  await s.waitFor(`return document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle'`, { timeout: 30000 }).catch(async (e) => {
    throw new Error(e.message + " / " + (await s.exec(`return document.querySelector('[data-testid=sync-error]')?.textContent`)));
  });
  await s.exec(`document.querySelector('[data-testid=settings] .close').click()`);
  await s.waitFor(`return document.querySelector('[data-testid=sync-indicator]')?.textContent.includes('Synced')`);
});

test("a second device receives the notes", async () => {
  const r = syncB();
  assert.equal(r.pulled, 2);
  assert.equal(fs.readFileSync(path.join(vaultB, "Shared.md"), "utf8"), "line one\nline two\nline three\n");
});

test("notes from the second device appear in the app after Sync now", async () => {
  fs.writeFileSync(path.join(vaultB, "From phone.md"), "written on the phone\n");
  syncB();
  await s.click(await s.find("[data-testid=sync-indicator]"));
  await s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="From phone.md"]')`, { timeout: 15000 });
  assert.equal(fs.readFileSync(path.join(vaultA, "From phone.md"), "utf8"), "written on the phone\n");
});

test("edits in the app sync automatically, merged with the other device", async () => {
  // Let the background loop's first scheduled sync (2 s after opening) pass,
  // so this test exercises the sync that a local edit triggers.
  await sleep(2500);
  // Phone changes line one; laptop (the app) changes line three.
  fs.writeFileSync(path.join(vaultB, "Shared.md"), "line one (phone)\nline two\nline three\n");
  syncB();
  await s.click(await s.find('[data-testid=tree-row][data-path="Shared.md"]'));
  await s.waitFor(`return document.querySelector('.cm-editor')?.__cairnView?.state.doc.toString().startsWith('line one')`);
  await s.exec(`
    const v = document.querySelector('.cm-editor').__cairnView;
    const line = v.state.doc.line(3);
    v.dispatch({ changes: { from: line.to, insert: ' (laptop)' } });
  `);
  // Autosave, then the debounced background sync pulls the phone's change and merges.
  const want = "line one (phone)\nline two\nline three (laptop)\n";
  const t0 = Date.now();
  await eventually(() => fs.readFileSync(path.join(vaultA, "Shared.md"), "utf8") === want, { timeout: 20000, message: "merged in the app vault" });
  await eventually(async () => (await s.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`)) === want, {
    message: "editor shows the merged text",
  });
  if (process.env.CAIRN_E2E_DEBUG) console.log("merged after", Date.now() - t0, "ms\n", drv.log().split("\n").filter((l) => /sync|vault/.test(l)).slice(-12).join("\n"));
  syncB();
  assert.equal(fs.readFileSync(path.join(vaultB, "Shared.md"), "utf8"), want);
});

test("overlapping edits produce a conflict copy shown in the status bar", async () => {
  fs.writeFileSync(path.join(vaultB, "Laptop only.md"), "phone rewrote this\n");
  syncB();
  writeA("Laptop only.md", "laptop rewrote this\n");
  await s.click(await s.find("[data-testid=sync-indicator]"));
  await s.waitFor(`return /conflict/.test(document.querySelector('[data-testid=sync-indicator]')?.textContent)`, { timeout: 15000 });
  const files = fs.readdirSync(vaultA).filter((f) => f.startsWith("Laptop only"));
  assert.equal(files.length, 2, files.join(", "));
  const contents = files.map((f) => fs.readFileSync(path.join(vaultA, f), "utf8")).sort();
  assert.deepEqual(contents, ["laptop rewrote this\n", "phone rewrote this\n"]);
});

test("version history restores an older version", async () => {
  await s.click(await s.find('[data-testid=tree-row][data-path="Shared.md"]'));
  await s.keys({ chord: ["", "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "version history");
  await sleep(150);
  await s.keys("");
  await s.waitFor(`return document.querySelectorAll('[data-testid=history-entry]').length >= 3`, { timeout: 10000 });
  // pick the oldest version
  await s.exec(`[...document.querySelectorAll('[data-testid=history-entry]')].at(-1).click()`);
  await sleep(300);
  await s.click(await s.find("[data-testid=history-restore]"));
  await s.click(await s.findWait("[data-testid=dialog-ok]"));
  await eventually(() => fs.readFileSync(path.join(vaultA, "Shared.md"), "utf8") === "line one\nline two\nline three\n", {
    message: "restored on disk",
  });
});

test("a note renamed in the app and edited before the next sync merges with the other device's edit", async () => {
  fs.writeFileSync(path.join(vaultB, "Draft.md"), "one\ntwo\nthree\n");
  syncB();
  await s.click(await s.find("[data-testid=sync-indicator]"));
  await s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Draft.md"]')`, { timeout: 15000 });
  // The phone changes the last line meanwhile.
  fs.writeFileSync(path.join(vaultB, "Draft.md"), "one\ntwo\nthree (phone)\n");
  syncB();
  // The laptop renames the note and goes on typing before its next sync.
  await s.click(await s.find('[data-testid=tree-row][data-path="Draft.md"]'));
  await s.waitFor(`return document.querySelector('.cm-editor')?.__cairnView?.state.doc.toString() === 'one\\ntwo\\nthree\\n'`);
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "rename current");
  await sleep(150);
  await s.keys(Key.enter);
  await s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`);
  await s.exec(`document.querySelector('[data-testid=rename-input]').select()`);
  await s.type(await s.find("[data-testid=rename-input]"), "Plan");
  await s.keys(Key.enter);
  await eventually(() => fs.existsSync(path.join(vaultA, "Plan.md")), { message: "renamed on disk" });
  await s.exec(`document.querySelector('.cm-editor').__cairnView.dispatch({ changes: { from: 0, insert: 'zero\\n' } })`);
  const want = "zero\none\ntwo\nthree (phone)\n";
  await eventually(() => fs.readFileSync(path.join(vaultA, "Plan.md"), "utf8") === want, { timeout: 20000, message: "the phone's edit merged into the renamed note" });
  assert.equal(fs.existsSync(path.join(vaultA, "Draft.md")), false);
  syncB();
  assert.equal(fs.readFileSync(path.join(vaultB, "Plan.md"), "utf8"), want);
  assert.equal(fs.existsSync(path.join(vaultB, "Draft.md")), false);
});
