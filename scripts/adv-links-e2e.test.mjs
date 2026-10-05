// Desktop e2e reproductions for link findings.
// Run (prebuilt target/debug/cairn, private headless display):
//   scripts/e2e-headless.sh scripts/adv-links-e2e.test.mjs
// Tests named "FINDING-NNN ..." are regression tests for a defect. Evidence
// (screenshots, logs) goes to e2e/.tmp/LK/.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "../e2e/webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-adv-lk-"));
const vault = path.join(tmp, "vault");
const shots = path.resolve(import.meta.dirname, "../e2e/.tmp/LK");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
const exists = (rel) => fs.existsSync(path.join(vault, rel));

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

write("Note.md", "# Root note\n\nI am the note at the vault root.\n");
write("a/Note.md", "# Folder note\n\nI am a/Note.md.\n");
write(
  "a/src.md",
  "# Source\n\n[same folder md link](Note.md)\n\n[parent md link](../Note.md)\n\n[[../Note|wiki parent]]\n\n[[./Note|wiki dot]]\n\n![case image](img/Pic.png)\n\n![control image](img/pic.png)\n",
);
fs.mkdirSync(path.join(vault, "a/img"), { recursive: true });
fs.copyFileSync(path.resolve(import.meta.dirname, "../app/src-tauri/icons/32x32.png"), path.join(vault, "a/img/pic.png"));

let drv, s;

before(async () => {
  fs.mkdirSync(shots, { recursive: true });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 2`, { timeout: 15000 });
});

after(async () => {
  try {
    if (s) fs.writeFileSync(path.join(shots, "final.png"), await s.screenshot());
  } catch {}
  await s?.close().catch(() => {});
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const activeTab = () => s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
const toasts = () => s.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent)`);

async function openNote(p) {
  // expand parent folders, then click the row
  const parts = p.split("/");
  for (let i = 1; i < parts.length; i++) {
    const dir = parts.slice(0, i).join("/");
    const open = await s.exec(`return !!document.querySelector('${rowSel(p)}')`);
    if (!open) await s.click(await s.findWait(rowSel(dir)));
  }
  await s.click(await s.findWait(rowSel(p)));
  await eventually(async () => (await activeTab()) === p, { message: `tab ${p} active` });
}

async function openSourceInPreview() {
  await openNote("a/src.md");
  await s.click(await s.find("[data-testid=mode-preview]"));
  await s.waitFor(`return !!document.querySelector('[data-testid=preview] a[href="Note.md"]')`);
}

async function clickPreview(css) {
  await s.exec(`document.querySelector(${JSON.stringify(`[data-testid=preview] ${css}`)}).click()`);
}

test("core backlinks: a/Note.md lists a/src.md (the same-folder Markdown link)", async () => {
  await openNote("a/Note.md");
  await s.click(await s.find("[data-testid=right-links]")).catch(() => {});
  await s.waitFor(`return [...document.querySelectorAll('[data-testid=backlink-source]')].some(e => e.textContent.includes('src'))`, {
    message: "a/src.md listed as a backlink of a/Note.md",
  });
});

test("clicking [same folder md link](Note.md) opens a/Note.md", async () => {
  await openSourceInPreview();
  await clickPreview('a[href="Note.md"]');
  await sleep(800);
  const tab = await activeTab();
  fs.writeFileSync(path.join(shots, "lk02-same-folder-click.png"), await s.screenshot());
  assert.equal(tab, "a/Note.md", `opened ${tab}; the backlinks panel says this link points to a/Note.md`);
});

test("clicking [parent md link](../Note.md) opens Note.md", async () => {
  await openSourceInPreview();
  await clickPreview('a[href="../Note.md"]');
  await sleep(800);
  const tab = await activeTab();
  const t = await toasts();
  fs.writeFileSync(path.join(shots, "lk02-parent-click.png"), await s.screenshot());
  assert.equal(tab, "Note.md", `active tab ${tab}, toasts ${JSON.stringify(t)}`);
});

test("[[../Note]] is resolved (not styled unresolved) and opens Note.md", async () => {
  await openSourceInPreview();
  const cls = await s.exec(`return [...document.querySelectorAll('[data-testid=preview] a.internal-link')].map(a => a.textContent + ':' + a.className)`);
  await clickPreview('a.internal-link[data-href="../Note"]');
  await sleep(800);
  const tab = await activeTab();
  const t = await toasts();
  fs.writeFileSync(path.join(shots, "lk01-wiki-parent-click.png"), await s.screenshot());
  assert.equal(tab, "Note.md", `classes ${JSON.stringify(cls)}; active tab ${tab}; toasts ${JSON.stringify(t)}`);
  assert.ok(!exists("../Note.md") && !exists("a/../Note.md.md"));
});

test("[[./Note]] opens a/Note.md", async () => {
  await openSourceInPreview();
  await clickPreview('a.internal-link[data-href="./Note"]');
  await sleep(800);
  const tab = await activeTab();
  const t = await toasts();
  assert.equal(tab, "a/Note.md", `active tab ${tab}; toasts ${JSON.stringify(t)}`);
});

test("a Markdown image whose path differs only in case renders (the core resolves it)", async () => {
  await openSourceInPreview();
  const sizes = await eventually(
    () =>
      s.exec(`const imgs = [...document.querySelectorAll('[data-testid=preview] img')];
        if (imgs.length < 2 || imgs.some(i => !i.complete)) return null;
        return imgs.map(i => ({ alt: i.alt, src: i.getAttribute('src'), w: i.naturalWidth }));`),
    { message: "images settled" },
  );
  fs.writeFileSync(path.join(shots, "lk12-images.png"), await s.screenshot());
  const control = sizes.find((x) => x.alt === "control image");
  const cased = sizes.find((x) => x.alt === "case image");
  assert.equal(control.w, 32, JSON.stringify(sizes));
  assert.equal(cased.w, 32, `case-variant image is broken: ${JSON.stringify(sizes)}`);
});
