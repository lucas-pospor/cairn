// Desktop e2e regression tests for FINDING-035 (wikilinks with ../
// or ./ never resolved). Uses the prebuilt target/debug/cairn.
//   scripts/e2e-headless.sh e2e/adv_verify_lk_01.test.mjs
// The first test only records what the app shows. Screenshots go to a temp
// dir printed at the start.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-lk01-"));
const vault = path.join(tmp, "vault");
const shots = path.join(tmp, "shots");
fs.mkdirSync(shots, { recursive: true });
console.log(`# screenshots in ${shots}`);

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
write("Top.md", "# Top\n");
write("a/Sibling.md", "# Sibling\n");
write("a/sub/Child.md", "# Child\n");
write("b/Target.md", "# Target\n");
write("a/src.md", "# Source\n\n[[../Top|wiki parent]]\n\n[[./Sibling|wiki dot]]\n\n[[../b/Target|wiki up and over]]\n\n[[sub/Child|wiki descendant]]\n\n![[../attachments/pic.png]]\n\n[[../Fresh|wiki missing]]\n");
fs.mkdirSync(path.join(vault, "attachments"), { recursive: true });
fs.copyFileSync(path.resolve(import.meta.dirname, "../app/src-tauri/icons/32x32.png"), path.join(vault, "attachments/pic.png"));

let drv, s;
const observed = {};

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
  console.log("# observed: " + JSON.stringify(observed, null, 1));
  await s?.close().catch(() => {});
  drv?.proc.kill();
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const activeTab = () => s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
const toasts = () => s.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent)`);

async function openNote(p) {
  const parts = p.split("/");
  for (let i = 1; i < parts.length; i++) {
    const dir = parts.slice(0, i).join("/");
    const shown = await s.exec(`return !!document.querySelector('${rowSel(p)}')`);
    if (!shown) await s.click(await s.findWait(rowSel(dir)));
  }
  await s.click(await s.findWait(rowSel(p)));
  await s.waitFor(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === ${JSON.stringify(p)}`);
}

test("record what the app shows for dot-segment wikilinks", async () => {
  await openNote("a/src.md");
  await s.click(await s.find("[data-testid=mode-live]"));
  await sleep(600);
  observed.live = await s.exec(`return [...document.querySelectorAll('.cm-lp-wikilink')].map(e => e.dataset.wiki + ' => ' + e.className)`);
  observed.liveImages = await s.exec(`return [...document.querySelectorAll('.cm-content img')].map(i => i.getAttribute('src'))`);
  await s.click(await s.find("[data-testid=mode-preview]"));
  await sleep(600);
  observed.preview = await s.exec(`return [...document.querySelectorAll('[data-testid=preview] a.internal-link')].map(a => a.dataset.href + ' => ' + a.className)`);
  observed.previewImages = await s.exec(`return [...document.querySelectorAll('[data-testid=preview] img')].map(i => i.getAttribute('src'))`);
  await s.click(await s.find("[data-testid=right-links]")).catch(() => {});
  await sleep(400);
  observed.outgoing = await s.exec(`return [...document.querySelectorAll('.outlink')].map(b => b.textContent.trim() + (b.classList.contains('unresolved') ? ' (unresolved)' : ''))`);
  fs.writeFileSync(path.join(shots, "src-preview.png"), await s.screenshot());
  // control: the descendant form is resolved
  assert.ok(observed.preview.some((x) => x.startsWith("sub/Child") && !x.includes("is-unresolved")), JSON.stringify(observed.preview));
});

test("Top.md backlinks list a/src.md ([[../Top]])", async () => {
  await openNote("Top.md");
  await s.click(await s.find("[data-testid=right-links]")).catch(() => {});
  await sleep(600);
  const bl = await s.exec(`return [...document.querySelectorAll('[data-testid=backlink-source]')].map(e => e.textContent.trim())`);
  const panel = await s.exec(`return document.querySelector('.pad.muted, p.muted.pad')?.textContent ?? null`);
  observed.topBacklinks = { bl, panel };
  fs.writeFileSync(path.join(shots, "top-backlinks.png"), await s.screenshot());
  assert.ok(bl.some((t) => t.includes("src")), `backlinks ${JSON.stringify(bl)}; panel says ${panel}`);
});

test("clicking [[../Top]] in reading view opens Top.md", async () => {
  await openNote("a/src.md");
  await s.click(await s.find("[data-testid=mode-preview]"));
  await s.waitFor(`return !!document.querySelector('[data-testid=preview] a.internal-link[data-href="../Top"]')`);
  await s.exec(`document.querySelector('[data-testid=preview] a.internal-link[data-href="../Top"]').click()`);
  await sleep(900);
  const tab = await activeTab();
  const t = await toasts();
  observed.clickParent = { tab, toasts: t, createdOutside: fs.existsSync(path.join(tmp, "Top.md")) };
  fs.writeFileSync(path.join(shots, "click-parent.png"), await s.screenshot());
  assert.ok(!fs.existsSync(path.join(tmp, "Top.md")), "a file was created outside the vault");
  assert.equal(tab, "Top.md", `active tab ${tab}; toasts ${JSON.stringify(t)}`);
});

test("clicking a missing [[../Fresh]] creates Fresh.md in the parent folder", async () => {
  await openNote("a/src.md");
  await s.click(await s.find("[data-testid=mode-preview]"));
  const sel = '[data-testid=preview] a.internal-link[data-href="../Fresh"]';
  await s.waitFor(`return !!document.querySelector(${JSON.stringify(sel)})`);
  const cls = await s.exec(`return document.querySelector(${JSON.stringify(sel)}).className`);
  assert.ok(cls.includes("is-unresolved"), cls);
  await s.exec(`document.querySelector(${JSON.stringify(sel)}).click()`);
  await s
    .waitFor(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === "Fresh.md"`)
    .catch(async (e) => assert.fail(`${e.message}; active tab ${await activeTab()}; toasts ${JSON.stringify(await toasts())}`));
  assert.ok(fs.existsSync(path.join(vault, "Fresh.md")));
  assert.ok(!fs.existsSync(path.join(tmp, "Fresh.md")), "a file was created outside the vault");
});
