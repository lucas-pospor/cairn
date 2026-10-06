// Image tabs on the desktop. Images open in a tab of their own from the file
// tree, the quick switcher, search, links and embedded images; the tab
// follows the file on disk (an outside change, a rename or move, a delete);
// an SVG shows only as an image, so its scripts never run; "Open in default
// app" still hands the file to the system opener, and other attachments
// still open there. The app runs with fake openers on PATH (as in
// adv_xss_ui.test.mjs), so nothing opens on the desktop.
//
// Run:  scripts/e2e-headless.sh e2e/image_tabs.test.mjs
//   one: scripts/e2e-headless.sh --test-name-pattern 'SVG' e2e/image_tabs.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { launch, freshEnv, eventually, sleep, Key } from "./adv_editor_lib.mjs";

const SHOTS = path.join(import.meta.dirname, ".tmp", "IMG");
fs.mkdirSync(SHOTS, { recursive: true });

// WebDriver keys that Key lacks.
const K = { right: "", down: "", end: "" };

/** A PNG of w x h pixels: a gradient, transparent on the right half when `alpha`. */
function png(w, h, alpha = false) {
  const px = alpha ? 4 : 3;
  const raw = Buffer.alloc((w * px + 1) * h);
  for (let y = 0; y < h; y++) {
    const row = y * (w * px + 1);
    for (let x = 0; x < w; x++) {
      const p = row + 1 + x * px;
      raw[p] = (x * 255) / w;
      raw[p + 1] = (y * 255) / h;
      raw[p + 2] = 160;
      if (alpha) raw[p + 3] = x < w / 2 ? 255 : 0;
    }
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(zlib.crc32(body));
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = alpha ? 6 : 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", zlib.deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

// Small files of each type the web view shows (made with Pillow and ImageMagick), and their widths.
const B64 = {
  "jpg": "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCAAIAAwDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD0DxN8ZvD2qeG9V0+3s9WWa7tJYEZ4owoZkKgnDnjJrwCiivCq1ZVXeR+qYLL6OBi40dmf/9k=",
  "gif": "R0lGODlhCgAHAIEAAP///x6gHgAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQAFAAAACwAAAAACgAHAAAIGQABCBwYoGCAgQQNIhRo8ODChhAjSpzYMCAAIfkEARQAAgAsAAAAAAoABwCB////Hh6gAAAAAAAACBkABQgcGKBggIEEDSIUaPDgwoYQI0qc2DAgADs=",
  "webp": "UklGRlYAAABXRUJQVlA4IEoAAAAQAgCdASoJAAYAAUAmJbACdLoAAyvScCsAAP7/Ez+T//EHv/y5P/1fV/9YKP3/1emf2B/eSLnfsG//vGf/3Gf/W7lNtqqyWgAAAA==",
  "avif": "AAAAHGZ0eXBhdmlmAAAAAG1pZjFhdmlmbWlhZgAAANZtZXRhAAAAAAAAACFoZGxyAAAAAAAAAABwaWN0AAAAAAAAAAAAAAAAAAAAACJpbG9jAAAAAERAAAEAAQAAAAAA+gABAAAAAAAAACkAAAAjaWluZgAAAAAAAQAAABVpbmZlAgAAAAABAABhdjAxAAAAAA5waXRtAAAAAAABAAAAVmlwcnAAAAA4aXBjbwAAAAxhdjFDgUBsAAAAABRpc3BlAAAAAAAAAA4AAAAKAAAAEHBpeGkAAAAAAwwMDAAAABZpcG1hAAAAAAAAAAEAAQOBAgMAAAAxbWRhdBIACglYDPZa0BDQbhAyGheHh4YhhhhhhlAAAAA61gTIxLk0hARy+ENA",
  "bmp": "Qk2uAAAAAAAAADYAAAAoAAAABwAAAAUAAAABABgAAAAAAHgAAADEDgAAxA4AAAAAAAAAAAAAHsjIHsjIHsjIHsjIHsjIHsjIHsjIAAAAHsjIHsjIHsjIHsjIHsjIHsjIHsjIAAAA////////////////HsjIHsjIHsjIAAAA////////////////HsjIHsjIHsjIAAAA////////////////HsjIHsjIHsjIAAAA",
  "ico": "AAABAAEAEBAAAAAAIABgAAAAFgAAAIlQTkcNChoKAAAADUlIRFIAAAAQAAAAEAgCAAAAkJFoNgAAACdJREFUeJxj/P//PwMOcFL+JKYgEy7VuMCoBppoYMEaO9S0YURqAACWTAaBNaAAmAAAAABJRU5ErkJggg==",
};
const WIDTHS = { png: 20, jpg: 12, gif: 10, webp: 9, avif: 14, bmp: 7, ico: 16, svg: 40 };

// An SVG whose scripts and handlers would set window.__pwned if they ran in the page.
const EVIL_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30" onload="top.__pwned='onload'">
<script>top.__pwned='script'</script>
<rect width="40" height="30" fill="#2a7"/><circle cx="20" cy="15" r="9" fill="#fff"/>
<foreignObject width="40" height="30"><img xmlns="http://www.w3.org/1999/xhtml" src="x" onerror="top.__pwned='foreign'"/></foreignObject>
</svg>`;
const VIEWBOX_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><rect width="100" height="50" fill="#36c"/></svg>`;

const files = {
  "Notes.md": "# Notes\n\nSee [[photo.png]] and [the chart](media/chart.webp).\n\nEmbedded:\n\n![[photo.png]]\n\nInline ![[icon.ico]] here.\n",
  "Other.md": "other note\n",
  "Linked.md": "# Linked\n\n| a |\n| - |\n| [![p](media/photo.png)](Other.md) |\n\nAnd [![p](media/photo.png)](Other.md) here.\n\nend\n",
  "media/photo.png": png(20, 14),
  "media/chart.webp": Buffer.from(B64.webp, "base64"),
  "media/big.png": png(2400, 1600),
  "media/clear.png": png(300, 200, true),
  "media/drawing.svg": EVIL_SVG,
  "media/wide.svg": VIEWBOX_SVG,
  "media/broken.png": "this is not a PNG",
  "media/report.pdf": "%PDF-1.4\n%%EOF\n",
  "media/swap.png": png(32, 32),
  "media/del.png": png(10, 10),
  "moved/keep.md": "a folder to move into\n",
  "icon.ico": Buffer.from(B64.ico, "base64"),
};
for (const ext of ["jpg", "gif", "avif", "bmp"]) files[`formats/sample.${ext}`] = Buffer.from(B64[ext], "base64");
files["formats/sample.png"] = png(20, 14);
files["formats/sample.webp"] = Buffer.from(B64.webp, "base64");
files["formats/sample.ico"] = Buffer.from(B64.ico, "base64");
files["formats/sample.svg"] = EVIL_SVG;

let env;
let app;
let canary;

before(async () => {
  env = freshEnv(files);
  // Fake system openers: they only record what they were asked to open.
  const shimDir = path.join(env.tmp, "bin");
  canary = path.join(env.tmp, "opened.txt");
  fs.mkdirSync(shimDir, { recursive: true });
  for (const n of ["xdg-open", "gio", "gnome-open", "kde-open", "wslview"]) {
    fs.writeFileSync(path.join(shimDir, n), `#!/bin/sh\nprintf '%s %s\\n' "${n}" "$*" >> ${JSON.stringify(canary)}\nexit 0\n`);
    fs.chmodSync(path.join(shimDir, n), 0o755);
  }
  app = await launch({
    vault: env.vault.root,
    xdg: env.xdg,
    waitRows: 4,
    env: { PATH: `${shimDir}:${process.env.PATH}`, BROWSER: path.join(shimDir, "xdg-open") },
  });
  await app.fakeFocus();
});

after(async () => {
  await app?.shot("image-final").catch(() => {});
  await app?.stop();
  await env?.cleanup();
});

const opened = () => (fs.existsSync(canary) ? fs.readFileSync(canary, "utf8") : "");
const clearOpened = () => fs.rmSync(canary, { force: true });

async function shot(name) {
  fs.writeFileSync(path.join(SHOTS, `${name}.png`), await app.s.screenshot());
}

/** The info line's file size for `rel`, as the viewer rounds it. */
function sizeText(rel) {
  const n = fs.statSync(env.vault.p(rel)).size;
  return n < 1024 ? `${n} bytes` : n < 1024 * 1024 ? `${Math.round(n / 1024)} KB` : `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** The image tab's state as the page shows it. */
function view() {
  return app.exec(`
    const v = document.querySelector('[data-testid=image-view]');
    const i = v?.querySelector('img');
    const tab = document.querySelector('[data-testid=tab][aria-selected=true]');
    return v && {
      tab: tab?.dataset.path, label: tab?.querySelector('.label')?.textContent,
      src: decodeURIComponent(i.getAttribute('src')), alt: i.alt, complete: i.complete, natural: i.naturalWidth,
      width: i.getBoundingClientRect().width, height: i.getBoundingClientRect().height,
      failed: document.querySelector('[data-testid=image-failed]')?.textContent.trim() ?? null,
      info: document.querySelector('[data-testid=image-info]')?.textContent.replace(/\\s+/g, ' ').trim(),
      actual: document.querySelector('[data-testid=image-actual-size]')?.getAttribute('aria-pressed'),
      focus: document.activeElement?.dataset.testid ?? document.activeElement?.tagName,
      editorShown: getComputedStyle(document.querySelector('[data-testid=editor]')).display !== 'none',
    };`);
}

/**
 * Wait until `p` is the active tab and its image has loaded, `w` pixels wide.
 * The width is read from the tab's info line: WebKit gives an SVG the size
 * it is laid out at as its naturalWidth.
 */
async function waitShown(p, w) {
  await eventually(async () => (await app.activeTab()) === p, { message: `tab ${p} active` });
  let last;
  try {
    return await eventually(
      async () => {
        const v = (last = await view());
        return v && v.src.includes(`/${p}?v=`) && v.complete && v.natural > 0 && v.info.startsWith(`${w} × `) ? v : null;
      },
      { message: `${p} shown at ${w}px`, timeout: 8000 },
    );
  } catch (e) {
    e.message += ` ${JSON.stringify(last)}`;
    throw e;
  }
}

async function treeClick(p, opts = {}) {
  const dir = p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : null;
  await app.exec(`if (!document.querySelector('[data-testid=file-tree]')) document.querySelector('[data-testid=tab-files]').click(); return 1`);
  if (dir) {
    const open = await app.exec(`return !!document.querySelector('[data-testid=tree-row][data-path="${p}"]')`);
    if (!open) await app.click(`[data-testid=tree-row][data-path="${dir}"]`);
  }
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="${p}"]')`, { message: `row ${p}` });
  if (opts.ctrl) {
    await app.exec(`document.querySelector('[data-testid=tree-row][data-path="${p}"]').dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true })); return 1`);
  } else {
    await app.click(`[data-testid=tree-row][data-path="${p}"]`);
  }
}

async function closeAll() {
  await app.exec(`return 1`);
  for (let i = 0; i < 30; i++) {
    const n = await app.exec(`return document.querySelectorAll('[data-testid=tab]').length`);
    if (!n) return;
    await app.exec(`document.querySelector('[data-testid=tab] .close').click(); return 1`);
    await sleep(80);
  }
}

test("an image clicked in the file tree opens in an image tab, not in another app", async () => {
  clearOpened();
  await treeClick("media/photo.png");
  const v = await waitShown("media/photo.png", 20);
  assert.equal(v.label, "photo.png");
  assert.equal(v.alt, "photo.png");
  assert.equal(v.editorShown, false, "the note editor is hidden");
  assert.equal(v.failed, null);
  assert.match(v.info, /^20 × 14 px · 100% · \d+ bytes$/);
  // The viewer takes the focus, as the editor does for a note.
  await eventually(async () => (await view()).focus === "image-stage", { message: "focus on the image" });
  assert.equal(await app.exec(`return document.activeElement.getAttribute('role') + ' ' + document.activeElement.getAttribute('aria-label')`), "region photo.png");
  await sleep(500);
  assert.deepEqual(await app.invokes("open_externally"), []);
  assert.equal(opened(), "");
  await shot("tree-photo");
});

test("each image type shows in the tab (png, jpg, gif, webp, avif, svg, bmp, ico)", async () => {
  const got = {};
  for (const ext of Object.keys(WIDTHS)) {
    const p = `formats/sample.${ext}`;
    await treeClick(p);
    try {
      await waitShown(p, WIDTHS[ext]);
      got[ext] = WIDTHS[ext];
    } catch (e) {
      got[ext] = e.message;
    }
  }
  assert.deepEqual(got, WIDTHS);
  // Each one replaced the image tab before it.
  assert.equal((await app.tabs()).filter((p) => p.startsWith("formats/")).length, 1);
  assert.deepEqual(await app.invokes("open_externally"), []);
});

test("an SVG shows as an image: its scripts and handlers never run, and it is never inlined", async () => {
  await closeAll();
  await treeClick("media/drawing.svg");
  const v = await waitShown("media/drawing.svg", 40);
  await sleep(800);
  const page = await app.exec(`return {
    pwned: window.__pwned ?? null,
    inlineSvg: document.querySelectorAll('[data-testid=image-view] svg, [data-testid=image-view] object, [data-testid=image-view] iframe, [data-testid=image-view] embed').length,
    tag: document.querySelector('[data-testid=image-view] img').tagName,
  }`);
  assert.deepEqual(page, { pwned: null, inlineSvg: 0, tag: "IMG" });
  assert.equal(v.failed, null);
  // An SVG with only a viewBox: WebKit sizes it by the viewBox.
  await treeClick("media/wide.svg");
  const wide = await waitShown("media/wide.svg", 100);
  assert.match(wide.info, /^100 × 50 px · 100%/);
  await shot("svg");
  assert.deepEqual(await app.errors(), []);
});

test("an image opens from the quick switcher", async () => {
  await closeAll();
  await app.open("Other.md");
  await app.chord(Key.ctrl, "o");
  const input = await app.s.findWait("[data-testid=switcher-input]");
  // Before anything is typed it lists recent notes only.
  const recent = await app.s.waitFor(`const l = [...document.querySelectorAll('[data-testid=switcher-item] .name')].map(e => e.textContent); return l.length && l`);
  assert.deepEqual([...recent].sort((a, b) => a.localeCompare(b)), ["keep", "Linked", "Notes", "Other"]);
  await app.s.type(input, "chart");
  await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=switcher-item]')].some(e => e.textContent.includes('chart.webp'))`);
  await shot("switcher");
  await app.keys(Key.enter);
  await waitShown("media/chart.webp", 9);
  // The note stays open in its own tab.
  assert.deepEqual(await app.tabs(), ["Other.md", "media/chart.webp"]);
});

test("an image opens from a wikilink and from a Markdown link", async () => {
  await closeAll();
  await app.open("Notes.md");
  await app.setMode("live");
  await app.focusEnd();
  await app.s.waitFor(`return !!document.querySelector('.cm-lp-wikilink')`);
  await app.click(".cm-lp-wikilink");
  await waitShown("media/photo.png", 20);
  assert.deepEqual(await app.tabs(), ["Notes.md", "media/photo.png"]);
  await app.click(`[data-testid=tab][data-path="Notes.md"]`);
  await app.focusEnd();
  await app.s.waitFor(`return !!document.querySelector('.cm-lp-link[data-url="media/chart.webp"]')`);
  await app.click(`.cm-lp-link[data-url="media/chart.webp"]`);
  // An image never replaces the note: it gets a tab of its own (an image
  // replaces only the image tab that is open).
  await waitShown("media/chart.webp", 9);
  assert.deepEqual(await app.tabs(), ["Notes.md", "media/photo.png", "media/chart.webp"]);
  // From the keyboard: Alt+Enter follows the link at the cursor.
  await app.click(`[data-testid=tab][data-path="Notes.md"]`);
  await app.setMode("source");
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; const i = v.state.doc.toString().indexOf('media/chart'); v.focus(); v.dispatch({ selection: { anchor: i + 2 } }); return 1`);
  await app.chord(Key.alt, Key.enter);
  await waitShown("media/chart.webp", 9);
  await app.click(`[data-testid=tab][data-path="Notes.md"]`);
  await app.setMode("live");
});

test("an image opens from the search panel, under the notes that match", async () => {
  await closeAll();
  await app.click("[data-testid=tab-search]");
  const input = await app.s.findWait("[data-testid=search-input]");
  await app.s.type(input, "photo");
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=search-image]').length > 0`);
  const listed = await app.exec(`return {
    notes: [...document.querySelectorAll('[data-testid=search-hit]')].map(e => e.textContent.trim()),
    images: [...document.querySelectorAll('[data-testid=search-image]')].map(e => e.textContent.replace(/\\s+/g, ' ').trim()),
    meta: document.querySelector('.search [role=status]').textContent.replace(/\\s+/g, ' ').trim(),
  }`);
  assert.deepEqual(listed.images, ["photo.png media"]);
  assert.deepEqual(listed.notes, ["Linked", "Notes"], "the notes that mention it are still found");
  assert.match(listed.meta, /^2 results · 1 image · \d+ ms$/);
  await shot("search");
  await app.click("[data-testid=search-image]");
  await waitShown("media/photo.png", 20);
  // A tag search lists no image; without one the line reads as before.
  const search = async (q) => {
    await app.exec(`const i = document.querySelector('[data-testid=search-input]'); i.value = ''; i.dispatchEvent(new Event('input')); return 1`);
    await app.s.type(input, q);
    await sleep(600);
    return app.exec(`return { images: document.querySelectorAll('[data-testid=search-image]').length, meta: document.querySelector('.search [role=status]').textContent.replace(/\\s+/g, ' ').trim() }`);
  };
  assert.equal((await search("photo #x")).images, 0);
  const other = await search("other");
  assert.equal(other.images, 0);
  assert.match(other.meta, /^2 results · \d+ ms$/);
  await app.click("[data-testid=tab-files]");
});

test("an embedded image opens on a click in the reading view and on Ctrl+click in Live Preview; a plain click there still edits", async () => {
  await closeAll();
  await app.open("Notes.md");
  // Reading view: a click opens it.
  await app.setMode("preview");
  await app.s.waitFor(`return document.querySelector('[data-testid=preview] img[data-path="media/photo.png"]')?.naturalWidth === 20`);
  await app.click(`[data-testid=preview] img[data-path="media/photo.png"]`);
  await waitShown("media/photo.png", 20);
  await app.click(`[data-testid=tab][data-path="Notes.md"]`);
  // Live Preview: a plain click puts the cursor on the embed's line.
  await app.setMode("live");
  await app.focusEnd();
  await app.s.waitFor(`return document.querySelector('.cm-lp-image-block img')?.naturalWidth === 20`);
  await closeTabs((p) => p !== "Notes.md");
  const r = await app.rect(".cm-lp-image-block img");
  await app.clickAt(r.x + r.w / 2, r.y + r.h / 2);
  await sleep(400);
  assert.deepEqual(await app.tabs(), ["Notes.md"]);
  assert.equal((await app.cursorLine()).text, "![[photo.png]]");
  // Ctrl+click opens it.
  await app.focusEnd();
  await app.s.waitFor(`return document.querySelector('.cm-lp-image-block img')?.naturalWidth === 20`);
  const r2 = await app.rect(".cm-lp-image-block img");
  // Ctrl held down through a real click (both input sources, tick by tick).
  await app.s.cmd("POST", "/actions", {
    actions: [
      { type: "key", id: "kb", actions: [{ type: "keyDown", value: Key.ctrl }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "keyUp", value: Key.ctrl }] },
      {
        type: "pointer",
        id: "mouse",
        parameters: { pointerType: "mouse" },
        actions: [
          { type: "pointerMove", origin: "viewport", x: Math.round(r2.x + r2.w / 2), y: Math.round(r2.y + r2.h / 2) },
          { type: "pointerDown", button: 0 },
          { type: "pointerUp", button: 0 },
          { type: "pause", duration: 0 },
        ],
      },
    ],
  });
  await app.s.cmd("DELETE", "/actions");
  await waitShown("media/photo.png", 20);
  assert.equal(await app.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`), files["Notes.md"], "the note is unchanged");
  // An inline embed opens on a middle click.
  await app.click(`[data-testid=tab][data-path="Notes.md"]`);
  await app.focusEnd();
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: 0 } }); return 1`);
  await app.s.waitFor(`return document.querySelector('.cm-lp-image img[data-path="icon.ico"]')?.naturalWidth === 16`);
  const r3 = await app.rect(`.cm-lp-image img[data-path="icon.ico"]`);
  await app.clickAt(r3.x + r3.w / 2, r3.y + r3.h / 2, { button: 1 });
  await waitShown("icon.ico", 16);
});

async function closeTabs(which) {
  for (const p of (await app.tabs()).filter(which)) {
    await app.exec(`document.querySelector('[data-testid=tab][data-path="${p}"] .close').click(); return 1`);
    await sleep(100);
  }
}

test("an image inside a link belongs to the link: a click follows the link in the reading view and Ctrl+click opens it in Live Preview", async () => {
  await closeAll();
  await app.open("Linked.md");
  await app.setMode("preview");
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=preview] a img[data-path="media/photo.png"]').length === 2`);
  await app.click(`[data-testid=preview] p a img[data-path="media/photo.png"]`);
  await eventually(async () => (await app.activeTab()) === "Other.md", { message: "link followed from the reading view" });
  assert.deepEqual(await app.tabs(), ["Other.md"]);
  await closeAll();
  await app.open("Linked.md");
  await app.setMode("live");
  await app.focusEnd();
  await app.s.waitFor(`return document.querySelector('.cm-lp-table a img[data-path="media/photo.png"]')?.naturalWidth === 20`);
  const r = await app.rect(`.cm-lp-table a img[data-path="media/photo.png"]`);
  await app.s.cmd("POST", "/actions", {
    actions: [
      { type: "key", id: "kb", actions: [{ type: "keyDown", value: Key.ctrl }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "keyUp", value: Key.ctrl }] },
      {
        type: "pointer",
        id: "mouse",
        parameters: { pointerType: "mouse" },
        actions: [
          { type: "pointerMove", origin: "viewport", x: Math.round(r.x + r.w / 2), y: Math.round(r.y + r.h / 2) },
          { type: "pointerDown", button: 0 },
          { type: "pointerUp", button: 0 },
          { type: "pause", duration: 0 },
        ],
      },
    ],
  });
  await app.s.cmd("DELETE", "/actions");
  await eventually(async () => (await app.tabs()).includes("Other.md"), { message: "link opened in a new tab from Live Preview" });
  assert.deepEqual(await app.tabs(), ["Linked.md", "Other.md"]);
});

test("an image is fitted to the tab, and shown at its own size on demand", async () => {
  await closeAll();
  await treeClick("media/big.png");
  let v = await waitShown("media/big.png", 2400);
  assert.ok(v.width < 1400 && v.width > 200, `fitted width ${v.width}`);
  assert.ok(Math.abs(v.width / v.height - 1.5) < 0.02, `keeps its proportions: ${v.width} x ${v.height}`);
  assert.equal(v.info.replace(/ · \d+% · /, " · N% · "), `2400 × 1600 px · N% · ${sizeText("media/big.png")}`);
  assert.match(v.info, / · \d{1,2}% · /);
  assert.equal(v.actual, "false");
  await shot("fit");
  await app.click("[data-testid=image-actual-size]");
  v = await eventually(async () => {
    const w = await view();
    return w.width === 2400 ? w : null;
  }, { message: "actual size" });
  assert.equal(v.actual, "true");
  assert.match(v.info, /^2400 × 1600 px · 100%/);
  const scroll = await app.exec(`const s = document.querySelector('[data-testid=image-stage]'); return { w: s.scrollWidth > s.clientWidth, h: s.scrollHeight > s.clientHeight, left: s.scrollLeft, top: s.scrollTop }`);
  assert.ok(scroll.w && scroll.h, "the stage scrolls");
  // The middle of the image stays in view.
  assert.ok(scroll.left > 500 && scroll.top > 300, JSON.stringify(scroll));
  await shot("actual-size");
  // The keys scroll it once the viewer has the focus.
  await app.exec(`document.querySelector('[data-testid=image-stage]').focus(); return 1`);
  const before = await app.exec(`const s = document.querySelector('[data-testid=image-stage]'); return [s.scrollLeft, s.scrollTop]`);
  await app.keys(K.down, K.down, K.right, K.right);
  await eventually(async () => {
    const now = await app.exec(`const s = document.querySelector('[data-testid=image-stage]'); return [s.scrollLeft, s.scrollTop]`);
    return now[0] > before[0] && now[1] > before[1];
  }, { message: "arrow keys scroll" });
  // A click on the image fits it again.
  await app.click("[data-testid=image]");
  await eventually(async () => (await view()).width < 1400, { message: "fitted again" });
  // A small image is already at its own size.
  await treeClick("media/photo.png");
  await waitShown("media/photo.png", 20);
  assert.equal(await app.exec(`return document.querySelector('[data-testid=image-actual-size]').disabled`), true);
  assert.equal((await view()).width, 20);
});

test("keyboard: Enter on the open image tab gives the viewer the focus; keys typed there never reach the hidden editor", async () => {
  await closeAll();
  await app.open("Other.md");
  await treeClick("media/photo.png");
  await waitShown("media/photo.png", 20);
  await app.exec(`document.querySelector('[data-testid=tab][aria-selected=true]').focus(); return 1`);
  await app.keys(Key.enter);
  await eventually(async () => (await view()).focus === "image-stage", { message: "Enter focuses the image" });
  await app.keys("xyz", Key.backspace);
  await sleep(300);
  await app.click(`[data-testid=tab][data-path="Other.md"]`);
  assert.equal(await app.text(), "other note\n");
  // Ctrl+W closes the image tab like any tab.
  await treeClick("media/photo.png");
  await waitShown("media/photo.png", 20);
  await app.chord(Key.ctrl, "w");
  await eventually(async () => !(await app.tabs()).includes("media/photo.png"), { message: "closed with Ctrl+W" });
});

test("the tab loads the image again when it changes outside Cairn", async () => {
  await closeAll();
  env.vault.write("media/swap.png", png(32, 32));
  await treeClick("media/swap.png");
  const before = await waitShown("media/swap.png", 32);
  env.vault.write("media/swap.png", png(64, 48));
  await waitShown("media/swap.png", 64);
  // The new size too, once the file list is read again.
  const v = await eventually(async () => {
    const w = await view();
    return w.info === `64 × 48 px · 100% · ${sizeText("media/swap.png")}` ? w : null;
  }, { message: "new size in the info line" });
  assert.notEqual(v.src, before.src);
});

test("the tab follows a rename and a move made outside Cairn, and one made in Cairn", async () => {
  await closeAll();
  // A picture of its own size, so that the rename waits until the write is
  // seen: an outside rename of an attachment is paired by size and time, so
  // a write and a rename in one scan count as a delete and a create.
  env.vault.write("media/swap.png", png(66, 50));
  await treeClick("media/swap.png");
  await waitShown("media/swap.png", 66);
  env.vault.rename("media/swap.png", "media/renamed.png");
  await waitShown("media/renamed.png", 66);
  // Let the scan of that rename finish: a move whose two halves land in two
  // scans counts as a delete and a create, for a note tab too.
  await sleep(1000);
  // Into a folder that exists: a move into a folder made in the same
  // moment can show as a delete and a create, for note tabs too.
  env.vault.rename("media/renamed.png", "moved/renamed.png");
  await waitShown("moved/renamed.png", 66);
  assert.equal((await view()).label, "renamed.png");
  // Rename in the file tree.
  if (!(await app.exec(`return !!document.querySelector('[data-testid=tree-row][data-path="moved/renamed.png"]')`))) await app.click(`[data-testid=tree-row][data-path="moved"]`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="moved/renamed.png"]')`);
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="moved/renamed.png"]').dispatchEvent(new MouseEvent('dblclick', { bubbles: true })); return 1`);
  const input = await app.s.findWait("[data-testid=rename-input]");
  await app.exec(`const i = document.querySelector('[data-testid=rename-input]'); i.value = 'final.png'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
  await app.s.click(input);
  await app.keys(Key.enter);
  await waitShown("moved/final.png", 66);
  assert.ok(env.vault.exists("moved/final.png"));
  assert.deepEqual(await app.errors(), []);
});

test("the tab closes when the image is deleted, outside Cairn or in it", async () => {
  await closeAll();
  await app.open("Other.md");
  await treeClick("media/del.png");
  await waitShown("media/del.png", 10);
  env.vault.rm("media/del.png");
  await eventually(async () => !(await app.tabs()).includes("media/del.png"), { message: "tab closed after an outside delete" });
  assert.equal(await app.activeTab(), "Other.md");
  // Delete from the tree, with the image open.
  env.vault.write("media/gone.png", png(10, 10));
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="media/gone.png"]')`, { timeout: 8000 });
  await treeClick("media/gone.png");
  await waitShown("media/gone.png", 10);
  await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
  await app.keys(Key.delete);
  await app.click("[data-testid=dialog-ok]");
  await eventually(async () => !(await app.tabs()).includes("media/gone.png"), { message: "tab closed after a delete in Cairn" });
  assert.ok(!env.vault.exists("media/gone.png"));
});

test("a file that is not a valid image shows a clear message", async () => {
  await closeAll();
  await treeClick("media/broken.png");
  await eventually(async () => (await view())?.failed, { message: "failed state" });
  const v = await view();
  assert.match(v.failed, /^Cairn cannot show broken\.png\./);
  assert.equal(await app.exec(`return document.querySelector('[data-testid=image-failed]').getAttribute('role')`), "alert");
  await shot("broken-light");
});

test("states are readable in both themes", async () => {
  await closeAll();
  const contrast = `
    const parse = (c) => { const m = c.match(/[\\d.]+/g).map(Number); return { r: m[0], g: m[1], b: m[2], a: m[3] ?? 1 }; };
    const lum = ({ r, g, b }) => [r, g, b].map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
    const bgOf = (el) => { for (; el; el = el.parentElement) { const c = parse(getComputedStyle(el).backgroundColor); if (c.a > 0) return c; } return { r: 255, g: 255, b: 255 }; };
    const ratio = (el) => { const a = lum(parse(getComputedStyle(el).color)), b = lum(bgOf(el)); return Math.round(((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)) * 100) / 100; };
    return [...document.querySelectorAll(arguments[0])].map((el) => ({ text: el.textContent.trim().slice(0, 40), ratio: ratio(el) }));`;
  const out = {};
  for (const theme of ["light", "dark"]) {
    await app.exec(`document.documentElement.dataset.theme = arguments[0]; return 1`, theme);
    await treeClick("media/broken.png");
    await eventually(async () => (await view())?.failed, { message: "failed state" });
    await sleep(200);
    out[`${theme} failed`] = await app.exec(contrast, "[data-testid=image-failed] p");
    await shot(`broken-${theme}`);
    await treeClick("media/clear.png");
    await waitShown("media/clear.png", 300);
    // (A disabled button needs no contrast.)
    out[`${theme} viewer`] = await app.exec(contrast, "[data-testid=image-info], [data-testid=image-view] .btn:not(:disabled)");
    await shot(`transparent-${theme}`);
  }
  await app.exec(`delete document.documentElement.dataset.theme; return 1`);
  const low = Object.entries(out).flatMap(([k, list]) => list.filter((x) => x.ratio < 4.5).map((x) => `${k}: ${x.text} ${x.ratio}`));
  assert.deepEqual(low, [], JSON.stringify(out));
});

test('"Open in default app" still hands the file to the system opener, as do other attachments', async () => {
  await closeAll();
  clearOpened();
  await treeClick("media/photo.png");
  await waitShown("media/photo.png", 20);
  await app.click("[data-testid=image-open-default]");
  await eventually(() => opened().includes("photo.png"), { message: "opener called from the image tab" });
  assert.match(opened(), new RegExp(`${path.join(env.vault.root, "media", "photo.png").replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")}`));
  // The file tree menu: "Open in new tab" and "Open in default app" for an image.
  clearOpened();
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="media/drawing.svg"]').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 100, clientY: 200 })); return 1`);
  const items = await app.exec(`return [...document.querySelectorAll('[role=menuitem]')].map(e => e.textContent.trim())`);
  assert.deepEqual(items.slice(0, 2), ["Open in new tab", "Open in default app"]);
  await app.exec(`[...document.querySelectorAll('[role=menuitem]')].find(e => e.textContent.trim() === 'Open in default app').click(); return 1`);
  await eventually(() => opened().includes("drawing.svg"), { message: "opener called from the tree menu" });
  // A PDF still opens in the default app on a click, with no tab.
  clearOpened();
  const tabs = await app.tabs();
  await treeClick("media/report.pdf");
  await eventually(() => opened().includes("report.pdf"), { message: "PDF opened in the default app" });
  assert.deepEqual(await app.tabs(), tabs);
  assert.deepEqual(await app.toasts(), []);
});

test("image tabs are not restored with the session; the other tabs are", async () => {
  await closeAll();
  await app.open("Notes.md");
  await treeClick("Other.md", { ctrl: true });
  await eventually(async () => (await app.activeTab()) === "Other.md", { message: "Other.md active" });
  await treeClick("media/photo.png");
  await waitShown("media/photo.png", 20);
  await sleep(500);
  const stored = await app.exec(`return Object.entries(localStorage).filter(([k]) => k.startsWith('cairn.session:')).map(([, v]) => v)`);
  assert.equal(stored.length, 1);
  assert.deepEqual(JSON.parse(stored[0]).tabs.map((t) => t.path), ["Notes.md", "Other.md"]);
  assert.ok(!stored[0].includes("photo"), stored[0]);
  await app.stop();
  app = await launch({ vault: env.vault.root, xdg: env.xdg, waitRows: 4, env: { PATH: `${path.join(env.tmp, "bin")}:${process.env.PATH}` } });
  await eventually(async () => (await app.tabs()).length === 2, { message: "tabs restored" });
  await sleep(500);
  assert.deepEqual(await app.tabs(), ["Notes.md", "Other.md"]);
  assert.equal(await app.activeTab(), "Notes.md");
  assert.deepEqual(await app.errors(), []);
});
