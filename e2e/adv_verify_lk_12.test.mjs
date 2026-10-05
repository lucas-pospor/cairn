// Reproduction for FINDING-086: Markdown images whose relative path differs
// from the real file only in case, or matches it only by suffix, must render
// in Reading view and in Live Preview, since the link resolver finds the
// real file.
// Run: scripts/e2e-headless.sh e2e/adv_verify_lk_12.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-lk12-"));
const vault = path.join(tmp, "vault");
const PNG = path.resolve(import.meta.dirname, "../app/src-tauri/icons/32x32.png");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
function png(rel) {
  fs.mkdirSync(path.dirname(path.join(vault, rel)), { recursive: true });
  fs.copyFileSync(PNG, path.join(vault, rel));
}

// Case-only mismatch (typical for vaults made on macOS/Windows).
png("a/img/pic.png");
write("a/src.md", "# Case\n\nintro\n\n![case image](img/Pic.png)\n\ntext\n\n![control image](img/pic.png)\n\nend\n");
// Suffix-only match: the core and LinkIndex.resolve give z/a/pic.png.
png("z/a/pic.png");
write("n.md", "# Suffix\n\nintro\n\n![suffix image](a/pic.png)\n\ntext\n\n![control image](z/a/pic.png)\n\nend\n");

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
  await s?.close().catch(() => {});
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;

async function openNote(p) {
  const parts = p.split("/");
  for (let i = 1; i < parts.length; i++) {
    const dir = parts.slice(0, i).join("/");
    const open = await s.exec(`return !!document.querySelector('${rowSel(p)}')`);
    if (!open) await s.click(await s.findWait(rowSel(dir)));
  }
  await s.click(await s.findWait(rowSel(p)));
  await s.waitFor(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === ${JSON.stringify(p)}`);
}

async function readingImages(note) {
  await openNote(note);
  await s.click(await s.find("[data-testid=mode-preview]"));
  return s.waitFor(
    `const imgs = [...document.querySelectorAll('[data-testid=preview] img')];
     if (imgs.length < 2 || imgs.some(i => !i.complete)) return null;
     return imgs.map(i => ({ alt: i.alt, src: i.getAttribute('src'), w: i.naturalWidth }));`,
    { message: "reading view images settled" },
  );
}

async function liveImages(note) {
  await openNote(note);
  await s.click(await s.find("[data-testid=mode-live]"));
  await sleep(1500); // let images load or error
  return s.exec(`return [...document.querySelectorAll('.cm-lp-image-block, .cm-lp-image')].map(w => {
    const i = w.querySelector('img');
    return { broken: w.classList.contains('is-broken'), text: w.textContent, src: i?.getAttribute('src') ?? null, w: i?.naturalWidth ?? 0 };
  })`);
}

test("Reading view: case-variant Markdown image renders", async () => {
  const imgs = await readingImages("a/src.md");
  const ctl = imgs.find((x) => x.alt === "control image");
  const cased = imgs.find((x) => x.alt === "case image");
  assert.equal(ctl.w, 32, JSON.stringify(imgs));
  assert.equal(cased.w, 32, `broken: ${JSON.stringify(imgs)}`);
});

test("Reading view: suffix-matched Markdown image renders", async () => {
  const imgs = await readingImages("n.md");
  const ctl = imgs.find((x) => x.alt === "control image");
  const suf = imgs.find((x) => x.alt === "suffix image");
  assert.equal(ctl.w, 32, JSON.stringify(imgs));
  assert.equal(suf.w, 32, `broken: ${JSON.stringify(imgs)}`);
});

test("Live Preview: case-variant Markdown image renders", async () => {
  const w = await liveImages("a/src.md");
  assert.equal(w.length, 2, JSON.stringify(w));
  assert.ok(w.some((x) => !x.broken && x.w === 32), `control must load: ${JSON.stringify(w)}`);
  assert.ok(w.every((x) => !x.broken), `broken widget: ${JSON.stringify(w)}`);
});

test("Live Preview: suffix-matched Markdown image renders", async () => {
  const w = await liveImages("n.md");
  assert.equal(w.length, 2, JSON.stringify(w));
  assert.ok(w.some((x) => !x.broken && x.w === 32), `control must load: ${JSON.stringify(w)}`);
  assert.ok(w.every((x) => !x.broken), `broken widget: ${JSON.stringify(w)}`);
});
