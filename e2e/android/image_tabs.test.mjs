// Image tabs on Android: an image opens in a tab inside Cairn instead of the
// "cannot open attachments in other apps yet" toast, in a vault in the app's
// storage and in a shared folder (Storage Access Framework, read through
// SafFs). Other attachments keep the toast. The tab follows the file: it
// loads it again after a change made by another app and closes when the
// file is deleted (both noticed at the next rescan, as for notes).
//
//   . scripts/android-env.sh
//   node --test --test-concurrency=1 e2e/android/image_tabs.test.mjs
//   or: scripts/adv-android-run-all.sh e2e/android/image_tabs.test.mjs
//
// Needs one emulator/device in `adb devices` (the debug APK is installed if
// missing). Clears the app's data and uses /sdcard/Documents/ImgTabs.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  Device, adb, devSh, runAs, sleep, eventually, APK, PKG, ROOT, revealRow, rescan, writeAppFile, pushFile, fillSharedFolder,
  readSettings, restoreSettings, captureErrors, pageErrors, toastTexts,
} from "./adv_helpers.mjs";
import { tapElement } from "./cdp.mjs";

const LABEL = "ImgTabs";
const F = `/sdcard/Documents/${LABEL}`;
const PIC = fs.readFileSync(path.join(ROOT, "app/src-tauri/icons/32x32.png"));
const PIC64 = fs.readFileSync(path.join(ROOT, "app/src-tauri/icons/64x64.png"));
const SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30" onload="top.__pwned='onload'"><script>top.__pwned='script'</script><rect width="40" height="30" fill="#2a7"/></svg>`;
const NOTE = "# Note\n\nSee [[pic.png]].\n\n![[pic.png]]\n";
const TOAST = /cannot open attachments in other apps yet/;
const d = new Device();
let settings;

before(async () => {
  settings = readSettings();
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  await d.fresh();
});

after(async () => {
  restoreSettings(settings);
  await d.shot("image-tabs-final.png").catch(() => {});
  d.close();
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

const rowSel = (p) => `[data-testid=tree-row][data-path=${JSON.stringify(p)}]`;

/** A real tap on an element, scrolled into view and once it stops moving (the drawer slides in). */
async function tap(sel) {
  await d.eval(`document.querySelector(${JSON.stringify(sel)})?.scrollIntoView({ block: 'center' })`);
  await tapElement(d.page, sel);
}

/** Open a file from the file tree drawer with a real tap (the drawer and its folder opened first). */
async function tapInTree(p) {
  // A row can be in the page while the drawer is closed: open it first.
  if (await d.eval(`document.querySelector('aside.left').classList.contains('hidden')`)) {
    await d.click("[data-testid=mobile-files]");
    await d.waitFor(`!document.querySelector('aside.left').classList.contains('hidden')`);
  }
  const dir = p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : null;
  if (dir) {
    if (!(await revealRow(d, dir))) throw new Error(`no tree row for ${dir}`);
    if ((await d.eval(`document.querySelector(${JSON.stringify(rowSel(dir))}).getAttribute('aria-expanded')`)) !== "true") await d.click(rowSel(dir));
  }
  if (!(await revealRow(d, p))) throw new Error(`no tree row for ${p}`);
  await tap(rowSel(p));
}

/** The image tab's state, or null when no image tab is shown. */
function view() {
  return d.eval(`(() => {
    const v = document.querySelector('[data-testid=image-view]');
    if (!v) return null;
    const i = v.querySelector('img');
    return {
      title: document.querySelector('.mobile-title')?.textContent.trim(),
      tab: document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path,
      src: decodeURIComponent(i.getAttribute('src')), complete: i.complete, natural: i.naturalWidth,
      info: document.querySelector('[data-testid=image-info]')?.textContent.trim(),
      failed: document.querySelector('[data-testid=image-failed]')?.textContent.trim() ?? null,
      defaultApp: !!document.querySelector('[data-testid=image-open-default]'),
      drawerOpen: !document.querySelector('aside.left').classList.contains('hidden'),
    };
  })()`);
}

/** Wait until `p` shows in the image tab, `w` pixels wide (read from the info line: WebKit and SVG). */
async function shown(p, w) {
  let last;
  try {
    return await eventually(
      async () => {
        const v = (last = await view());
        return v && v.tab === p && v.src.startsWith(`http://vault.localhost/${p}?v=`) && v.complete && v.natural > 0 && v.info?.startsWith(`${w} × `) ? v : null;
      },
      { message: `${p} shown at ${w}px`, timeout: 20000 },
    );
  } catch (e) {
    e.message += ` ${JSON.stringify(last)}`;
    throw e;
  }
}

async function checkVault(name, { write, remove }) {
  await captureErrors(d);
  await tapInTree("media/pic.png");
  const v = await shown("media/pic.png", 32);
  assert.equal(v.title, "pic.png");
  assert.equal(v.failed, null);
  assert.equal(v.defaultApp, false, "no 'Open in default app' on Android");
  assert.equal(v.drawerOpen, false, "the drawer closed to show the image");
  assert.deepEqual((await toastTexts(d)).filter((t) => TOAST.test(t)), []);
  await d.shot(`image-tabs-${name}-png.png`);

  // An SVG shows as an image; its scripts never run.
  await tapInTree("media/drawing.svg");
  await shown("media/drawing.svg", 40);
  assert.equal(await d.eval(`window.__pwned ?? null`), null);

  // Other attachments keep the toast and open nothing.
  await tapInTree("doc.pdf");
  await eventually(async () => (await toastTexts(d)).some((t) => TOAST.test(t) && t.includes("doc.pdf")), { message: "toast for doc.pdf" });
  assert.equal((await view())?.tab, "media/drawing.svg", "the image tab stays");

  // From a link and from the embedded image in the reading view.
  await tapInTree("Note.md");
  await d.waitFor(`document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === 'Note.md'`);
  await d.click("[data-testid=mode-preview]");
  await d.waitFor(`document.querySelector('[data-testid=preview] img[data-path="media/pic.png"]')?.naturalWidth === 32`, 20000);
  await tap(`[data-testid=preview] img[data-path="media/pic.png"]`);
  await shown("media/pic.png", 32);
  await d.eval(`document.querySelector('[data-testid=tab][data-path="Note.md"]').click()`);
  await d.waitFor(`!!document.querySelector('[data-testid=preview] a.internal-link')`);
  await tap(`[data-testid=preview] a.internal-link`);
  await shown("media/pic.png", 32);
  await d.shot(`image-tabs-${name}-from-link.png`);

  // Changed by another app: loaded again at the next rescan.
  write("media/pic.png", PIC64);
  await rescan(d);
  await shown("media/pic.png", 64);

  // Deleted by another app: the tab closes, the note stays.
  remove("media/pic.png");
  await rescan(d);
  await eventually(async () => !(await d.eval(`[...document.querySelectorAll('[data-testid=tab]')].map(t => t.dataset.path)`)).includes("media/pic.png"), {
    message: "image tab closed",
  });
  assert.equal(await d.eval(`document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path`), "Note.md");
  assert.deepEqual(await pageErrors(d), []);
}

test("app storage: images open in an image tab; other attachments keep the toast", async () => {
  const vault = await d.createAppVault("ImgTabs");
  const files = { "media/pic.png": PIC, "media/drawing.svg": SVG, "doc.pdf": "%PDF-1.4\n%%EOF\n", "Note.md": NOTE };
  for (const [p, c] of Object.entries(files)) writeAppFile(`${vault}/${p}`, c);
  await rescan(d);
  await checkVault("app", {
    write: (p, c) => writeAppFile(`${vault}/${p}`, c),
    remove: (p) => runAs(`rm ${vault}/${p}`),
  });
});

test("shared folder: images open in an image tab, read through the Storage Access Framework", async () => {
  fillSharedFolder(F, { "media/pic.png": PIC, "media/drawing.svg": SVG, "doc.pdf": "%PDF-1.4\n%%EOF\n", "Note.md": NOTE });
  await d.toWelcome();
  await d.pickSafFolder(LABEL);
  await checkVault("saf", {
    write: (p, c) => pushFile(`${F}/${p}`, c),
    remove: (p) => devSh(`rm ${F}/${p}`),
  });
  devSh(`rm -rf ${F}`);
});
