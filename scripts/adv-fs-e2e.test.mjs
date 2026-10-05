// Adversarial desktop e2e tests: file-level problems as the user sees them in
// the real app. Run through the headless runner:
//   scripts/e2e-headless.sh scripts/adv-fs-e2e.test.mjs
// Tests named "FINDING-NNN ..." are regression tests for a defect.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "../e2e/webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-advfs-"));
const vault = path.join(tmp, "vault");
const shots = path.resolve(import.meta.dirname, "../e2e/.tmp/FS");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
const read = (rel) => fs.readFileSync(path.join(vault, rel), "utf8");

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

const NFD = "café.md"; // as written by macOS
const NFC = "café.md";

write("Start.md", "# Start\n");
write("crlf.md", "# Windows note\r\n\r\nline one\r\nline two\r\n");
write("a.md", "lower case note\n");
write("A.md", "UPPER CASE NOTE, precious\n");
write(NFD, "hello from a mac\n");

let drv, s;

before(async () => {
  fs.mkdirSync(shots, { recursive: true });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 4`, { timeout: 15000 });
});

after(async () => {
  if (s) fs.writeFileSync(path.join(shots, "adv-fs-final.png"), await s.screenshot());
  await s?.close();
  drv?.proc.kill();
  await sleep(1000); // the app may still flush its cache/data dirs while exiting
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 3, retryDelay: 200 });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const activeTab = () => s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);

async function openFromTree(p) {
  await s.click(await s.find(rowSel(p)));
  await eventually(async () => (await activeTab()) === p, { message: `tab ${p} active` });
}

test("FINDING-045: editing a CRLF note keeps its line endings", async () => {
  await openFromTree("crlf.md");
  await s.waitFor(`return !!document.querySelector('.cm-editor')?.__cairnView`);
  await s.click(await s.find("[data-testid=mode-source]"));
  await s.exec(`
    const v = document.querySelector('.cm-editor').__cairnView;
    v.focus();
    v.dispatch({ selection: { anchor: v.state.doc.length } });
  `);
  await s.keys("line three");
  await eventually(() => read("crlf.md").includes("line three"), { message: "file saved" });
  const bytes = read("crlf.md");
  fs.writeFileSync(path.join(shots, "crlf-after-save.txt"), JSON.stringify(bytes));
  const bareLf = (bytes.match(/(?<!\r)\n/g) ?? []).length;
  assert.equal(bareLf, 0, `file now has ${bareLf} bare LF line endings: ${JSON.stringify(bytes)}`);
});

test("FINDING-011: a note with an NFD file name (made on a Mac) opens", async () => {
  await s.waitFor(`return !!document.querySelector('${rowSel(NFC)}')`);
  await s.click(await s.find(rowSel(NFC)));
  await sleep(800);
  fs.writeFileSync(path.join(shots, "nfd-open.png"), await s.screenshot());
  const text = await s.exec(`return document.querySelector('.cm-editor')?.__cairnView?.state.doc.toString() ?? null`);
  const body = await s.exec(`return document.querySelector('main, body').innerText.slice(0, 2000)`);
  assert.equal(text, "hello from a mac\n", `editor text ${JSON.stringify(text)}; page says: ${JSON.stringify(body.match(/not found[^\n]*/i)?.[0] ?? "")}`);
});

test("FINDING-003: inline rename a.md -> A refuses to overwrite the existing A.md", async () => {
  await s.click(await s.find(rowSel("a.md")));
  await sleep(100);
  await s.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
  await s.keys(Key.f2);
  const input = await eventually(() => s.find("[data-testid=rename-input]"), { message: "rename input" });
  await s.exec(`document.querySelector('[data-testid=rename-input]').select()`);
  await s.type(input, "A");
  await s.keys(Key.enter);
  await sleep(1000);
  fs.writeFileSync(path.join(shots, "case-rename.png"), await s.screenshot());
  const upper = read("A.md");
  const lowerExists = fs.existsSync(path.join(vault, "a.md"));
  assert.equal(upper, "UPPER CASE NOTE, precious\n", `A.md now contains ${JSON.stringify(upper)}; a.md exists: ${lowerExists}`);
});
