// Reproduction for FINDING-011: a note and a folder whose names are NFD on
// disk, as macOS / Mac-made zips / rsync from a Mac write them. Run through
// the headless runner:
//   scripts/e2e-headless.sh e2e/adv_verify_fs_02.test.mjs
// Everything (including screenshots) lives in a temp dir.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-fs02-"));
const vault = path.join(tmp, "vault");

const NFD = "café.md";
const NFC = "café.md";
const NFD_DIR = "Résumé";
const NFC_DIR = "Résumé";

fs.mkdirSync(path.join(vault, NFD_DIR), { recursive: true });
fs.writeFileSync(path.join(vault, "Start.md"), "# Start\n");
fs.writeFileSync(path.join(vault, NFD), "hello from a mac\n");
fs.writeFileSync(path.join(vault, NFD_DIR, "inside.md"), "inside the folder\n");

// Unicode twins (the same name in NFC and NFD): the second one is shown
// under a twin name (by design).
const TWIN_NFC = "No\u00ebl.md";
const TWIN_NFD = "Noe\u0308l.md";
const TWIN = "No\u00ebl (Unicode twin).md";
fs.writeFileSync(path.join(vault, TWIN_NFC), "nfc twin\n");
fs.writeFileSync(path.join(vault, TWIN_NFD), "nfd twin\n");

let drv, s;
let initialRows = [];

before(async () => {
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 2`, { timeout: 15000 });
  await sleep(1500);
  initialRows = await s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path + ':' + r.dataset.kind)`);
});

after(async () => {
  if (s) fs.writeFileSync(path.join(tmp, "final.png"), await s.screenshot());
  await s?.close();
  drv?.proc.kill();
  await sleep(1000);
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 3, retryDelay: 200 });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;

test("FINDING-011: an NFD-named note opens in the editor", async () => {
  await s.waitFor(`return !!document.querySelector('${rowSel(NFC)}')`);
  await s.click(await s.find(rowSel(NFC)));
  await sleep(1000);
  const text = await s.exec(`return document.querySelector('.cm-editor')?.__cairnView?.state.doc.toString() ?? null`);
  const body = await s.exec(`return document.body.innerText.slice(0, 3000)`);
  assert.equal(text, "hello from a mac\n", `editor text ${JSON.stringify(text)}; page says: ${JSON.stringify(body.match(/not found[^\n]*/i)?.[0] ?? "")}`);
});

test("FINDING-011: the note inside an NFD-named folder is listed", async () => {
  const before = await s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path + ':' + r.dataset.kind + ':' + r.getAttribute('aria-expanded'))`);
  const folder = await s.exec(`return !!document.querySelector('${rowSel(NFC_DIR)}')`);
  // expand the folder if it is collapsed
  if (folder) {
    await s.click(await s.find(rowSel(NFC_DIR)));
    await sleep(500);
  }
  const rows = await s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path)`);
  assert.ok(rows.includes(`${NFC_DIR}/inside.md`), `tree rows at start: ${JSON.stringify(initialRows)}; before expanding: ${JSON.stringify(before)}; after: ${JSON.stringify(rows)}`);
});

const names = () => fs.readdirSync(vault).filter((n) => !n.startsWith(".")).sort();
const read = (name) => fs.readFileSync(path.join(vault, name), "utf8");

async function until(fn, message, timeout = 8000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (fn()) return;
    await sleep(100);
  }
  assert.fail(message);
}

async function openAndExpect(p, text) {
  await s.waitFor(`return !!document.querySelector('${rowSel(p)}')`, { timeout: 10000, message: `${p} is listed` });
  await s.click(await s.find(rowSel(p)));
  const doc = `document.querySelector('.cm-editor')?.__cairnView?.state.doc.toString()`;
  await s.waitFor(`return ${doc} === ${JSON.stringify(text)}`, { timeout: 10000, message: `${p} opens with its text` });
}

async function typeAtEnd(text) {
  await s.exec(`
    const v = document.querySelector('.cm-editor').__cairnView;
    v.focus();
    v.dispatch({ selection: { anchor: v.state.doc.length } });
  `);
  await s.keys(text);
}

test("FINDING-011: Unicode twins both show, and the twin saves to its own file", async () => {
  const before = names();
  await openAndExpect(TWIN_NFC, "nfc twin\n");
  await openAndExpect(TWIN, "nfd twin\n");
  await typeAtEnd("edited");
  await until(() => read(TWIN_NFD) === "nfd twin\nedited", `the NFD file has ${JSON.stringify(read(TWIN_NFD))}`);
  assert.equal(read(TWIN_NFC), "nfc twin\n");
  assert.deepEqual(names(), before);
});
