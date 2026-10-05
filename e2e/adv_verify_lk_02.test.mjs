// Reproduction for FINDING-036: clicking a Markdown link resolved
// the target by name only (backend resolve_link -> Index::resolve), while the
// backlinks/outgoing panels and the graph resolve Markdown links relative to
// the source note first (Index::resolve_link). A click must open what the
// panels show.
//
// Run (prebuilt target/debug/cairn, private headless display):
//   scripts/e2e-headless.sh e2e/adv_verify_lk_02.test.mjs
//
// Screenshots go to a temp dir that is printed at the end.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-lk02-"));
const vault = path.join(tmp, "vault");
const shots = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-lk02-shots-"));

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
function listVault(dir = vault, pre = "") {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name === ".cairn") continue;
    const rel = pre ? `${pre}/${e.name}` : e.name;
    if (e.isDirectory()) out.push(...listVault(path.join(dir, e.name), rel));
    else out.push(rel);
  }
  return out.sort();
}

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
  "# Source\n\nintro line\n\n[same folder](Note.md)\n\n[parent dir](../Note.md)\n\n[dot slash](./Note.md)\n\ntrailing line\n",
);
const initialFiles = listVault();

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
  try {
    if (s) fs.writeFileSync(path.join(shots, "final.png"), await s.screenshot());
  } catch {}
  await s?.close().catch(() => {});
  drv?.proc.kill();
  console.log(`# screenshots in ${shots}`);
  console.log(`# vault files at end: ${JSON.stringify(listVault())}`);
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const activeTab = () => s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
const toasts = () => s.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent.trim())`);
const clearToasts = () => s.exec(`document.querySelectorAll('.toast').forEach(t => t.remove())`);

async function openFromTree(p) {
  const parts = p.split("/");
  for (let i = 1; i < parts.length; i++) {
    const dir = parts.slice(0, i).join("/");
    const open = await s.exec(`return !!document.querySelector('${rowSel(p)}')`);
    if (!open) await s.click(await s.findWait(rowSel(dir)));
  }
  await s.click(await s.findWait(rowSel(p)));
  await eventually(async () => (await activeTab()) === p, { message: `tab ${p} active` });
}

async function observe(label) {
  await sleep(900);
  const tab = await activeTab();
  const t = await toasts();
  fs.writeFileSync(path.join(shots, `${label}.png`), await s.screenshot());
  console.log(`# ${label}: active tab ${tab}; toasts ${JSON.stringify(t)}`);
  return { tab, toasts: t };
}

// ---- control: what the index says ----

test("control: outgoing panel of a/src.md resolves the three links relative to a/", async () => {
  await openFromTree("a/src.md");
  await s.click(await s.find("[data-testid=right-links]")).catch(() => {});
  const titles = await eventually(
    async () => {
      const t = await s.exec(`return [...document.querySelectorAll('[data-testid=backlinks] button.outlink')].map(b => b.title)`);
      return t.length ? t : null;
    },
    { message: "outgoing links listed" },
  );
  console.log(`# outgoing titles: ${JSON.stringify(titles)}`);
  // uniqueOut collapses same-resolved entries: a/Note.md (same folder + ./) and Note.md (../)
  assert.deepEqual([...titles].sort(), ["Note.md", "a/Note.md"]);
});

test("control: backlinks panel of a/Note.md lists a/src.md", async () => {
  await openFromTree("a/Note.md");
  await s.click(await s.find("[data-testid=right-links]")).catch(() => {});
  await s.waitFor(`return [...document.querySelectorAll('[data-testid=backlink-source]')].some(e => e.textContent.includes('src'))`, {
    message: "a/src.md listed as a backlink of a/Note.md",
  });
});

// ---- reading view ----

async function previewClick(href, label) {
  await openFromTree("a/src.md");
  await s.click(await s.find("[data-testid=mode-preview]"));
  await s.waitFor(`return !!document.querySelector('[data-testid=preview] a[href=${JSON.stringify(href)}]')`);
  await clearToasts();
  await s.click(await s.find(`[data-testid=preview] a[href="${href}"]`));
  return observe(label);
}

test("reading view: click [same folder](Note.md) opens a/Note.md", async () => {
  const r = await previewClick("Note.md", "preview-same-folder");
  assert.equal(r.tab, "a/Note.md", JSON.stringify(r));
});

test("reading view: click [parent dir](../Note.md) opens Note.md", async () => {
  const r = await previewClick("../Note.md", "preview-parent");
  assert.equal(r.tab, "Note.md", JSON.stringify(r));
});

test("reading view: click [dot slash](./Note.md) opens a/Note.md", async () => {
  const r = await previewClick("./Note.md", "preview-dot");
  assert.equal(r.tab, "a/Note.md", JSON.stringify(r));
});

// ---- Live Preview ----

async function liveClick(url, label) {
  await openFromTree("a/src.md");
  await s.click(await s.find("[data-testid=mode-live]"));
  await s.waitFor(`return !!document.querySelector('.cm-lp-link[data-url=${JSON.stringify(url)}]')`, {
    message: `live link ${url} rendered`,
  });
  await clearToasts();
  await s.exec(
    `const el = document.querySelector('.cm-lp-link[data-url=' + JSON.stringify(arguments[0]) + ']');
     const r = el.getBoundingClientRect();
     el.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, clientX: r.left + 3, clientY: r.top + r.height / 2, button: 0 }));`,
    url,
  );
  return observe(label);
}

test("Live Preview: click [same folder](Note.md) opens a/Note.md", async () => {
  const r = await liveClick("Note.md", "live-same-folder");
  assert.equal(r.tab, "a/Note.md", JSON.stringify(r));
});

test("Live Preview: click [parent dir](../Note.md) opens Note.md", async () => {
  const r = await liveClick("../Note.md", "live-parent");
  assert.equal(r.tab, "Note.md", JSON.stringify(r));
});

// ---- Source mode, Ctrl+click ----

async function sourceCtrlClick(needle, label) {
  await openFromTree("a/src.md");
  await s.click(await s.find("[data-testid=mode-source]"));
  await s.waitFor(`return [...document.querySelectorAll('.cm-line')].some(l => l.textContent.includes(${JSON.stringify(needle)}))`);
  await clearToasts();
  const ok = await s.exec(
    `const needle = arguments[0];
     const line = [...document.querySelectorAll('.cm-line')].find(l => l.textContent.includes(needle));
     const want = line.textContent.indexOf(needle) + needle.length - 3; // inside the (url) part
     const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
     let off = 0, node;
     while ((node = walker.nextNode())) {
       if (off + node.data.length > want) break;
       off += node.data.length;
     }
     if (!node) return false;
     const range = document.createRange();
     range.setStart(node, want - off);
     range.setEnd(node, want - off + 1);
     const r = range.getBoundingClientRect();
     const x = r.left + r.width / 2, y = r.top + r.height / 2;
     const target = document.elementFromPoint(x, y);
     target.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, clientX: x, clientY: y, ctrlKey: true, button: 0 }));
     return true;`,
    needle,
  );
  assert.ok(ok, "found link text in source");
  return observe(label);
}

test("Source mode: Ctrl+click [same folder](Note.md) opens a/Note.md", async () => {
  const r = await sourceCtrlClick("[same folder](Note.md)", "source-same-folder");
  assert.equal(r.tab, "a/Note.md", JSON.stringify(r));
});

test("Source mode: Ctrl+click [parent dir](../Note.md) opens Note.md", async () => {
  const r = await sourceCtrlClick("[parent dir](../Note.md)", "source-parent");
  assert.equal(r.tab, "Note.md", JSON.stringify(r));
});

// ---- Outgoing links panel (same openLink path) ----

async function outgoingClick(title, label) {
  await openFromTree("a/src.md");
  await s.click(await s.find("[data-testid=right-links]")).catch(() => {});
  await s.waitFor(`return [...document.querySelectorAll('[data-testid=backlinks] button.outlink')].some(b => b.title === ${JSON.stringify(title)})`);
  await clearToasts();
  await s.exec(
    `[...document.querySelectorAll('[data-testid=backlinks] button.outlink')].find(b => b.title === arguments[0]).click()`,
    title,
  );
  return observe(label);
}

test("Outgoing panel: clicking the entry titled a/Note.md opens a/Note.md", async () => {
  const r = await outgoingClick("a/Note.md", "outgoing-a-note");
  assert.equal(r.tab, "a/Note.md", JSON.stringify(r));
});

test("Outgoing panel: clicking the entry titled Note.md (from ../Note.md) opens Note.md", async () => {
  const r = await outgoingClick("Note.md", "outgoing-root-note");
  assert.equal(r.tab, "Note.md", JSON.stringify(r));
});

test("no stray files were created by the clicks", async () => {
  assert.deepEqual(listVault(), initialFiles);
});
