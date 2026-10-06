// Themes and a font file on the phone: the Theme list applies and saves a
// theme and shows the Light and Dark lists under System; a font file picked in
// Settings > Appearance is saved in the vault's .cairn/fonts/ and used for note
// text, in app storage and in a shared folder (written and read through the
// Storage Access Framework), also after the app starts again; and the
// system's file picker opens for any type of file and hands the font over.
//
//   scripts/adv-android-run-all.sh e2e/android/appearance.test.mjs
//
// Needs one emulator or device in `adb devices` and the debug APK, and
// /usr/share/fonts/liberation/LiberationMono-Regular.ttf on this computer.
// Clears the app's data and uses /sdcard/Documents/Looks and
// /sdcard/AFontPick. Screenshots go to e2e/.tmp/AN/.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import {
  Device, adb, devSh, runAs, sleep, eventually, closeSettings, writeAppFile, readAppFile, rescan, pushFile, fillSharedFolder,
  uiClick, uiClickDesc, captureErrors, pageErrors, toastTexts, APK, PKG, APP_DATA,
} from "./adv_helpers.mjs";
import { tapElement } from "./cdp.mjs";

const TTF = "/usr/share/fonts/liberation/LiberationMono-Regular.ttf";
const FONT = fs.existsSync(TTF) ? fs.readFileSync(TTF) : null;
const MD5 = FONT && crypto.createHash("md5").update(FONT).digest("hex");
const LINES = "iiiiiiiiiiiiiiii\n\nMMMMMMMMMMMMMMMM\n";
const LABEL = "Looks";
const F = `/sdcard/Documents/${LABEL}`;
const VAULT = `${APP_DATA}/vaults/Looks`;
const PICK = "/sdcard/AFontPick";
const d = new Device();

before(async () => {
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  await d.fresh();
});

after(async () => {
  await d.shot("appearance-final.png").catch(() => {});
  d.close();
  devSh(`rm -rf ${F} ${PICK}`);
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

async function tap(selector) {
  await d.eval(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({ block: 'center', inline: 'center' })`);
  await tapElement(d.page, selector);
}

/** Settings > Appearance, opened with taps from the Files drawer. */
async function openAppearance() {
  if (await d.eval(`!!document.querySelector('[data-testid=settings]')`)) return;
  if (await d.eval(`document.querySelector('aside.left').classList.contains('hidden')`)) await tap("[data-testid=mobile-files]");
  await d.waitFor(`!document.querySelector('aside.left').classList.contains('hidden')`);
  await sleep(300);
  await tap("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=font-choose]')`);
}

/** Pick an entry of a list in Settings (Android shows its own dialog for a select, so set it as that does). */
const pickIn = (testid, value) =>
  d.eval(`(() => { const s = document.querySelector('[data-testid=${testid}]'); s.value = ${JSON.stringify(value)}; s.dispatchEvent(new Event('change', { bubbles: true })); return s.value; })()`);

/** Hand `bytes` to the Font file input as `name`, as the system's file picker does. */
const pickFont = (name, bytes) =>
  d.eval(`(() => { const b = Uint8Array.from(atob(${JSON.stringify(Buffer.from(bytes).toString("base64"))}), (c) => c.charCodeAt(0));
    const dt = new DataTransfer(); dt.items.add(new File([b], ${JSON.stringify(name)}));
    const i = document.querySelector('[data-testid=font-input]'); i.files = dt.files; i.dispatchEvent(new Event('change', { bubbles: true })); return true; })()`);

/** The note text's font: --font-text as set on <html> (the computed value has var() filled in), Cairn's faces, the state line, and how wide the lines of i and M are. */
const look = () =>
  d.eval(`(() => {
    const faces = [...document.fonts].filter((f) => f.family.startsWith('cairn-text-font-')).map((f) => [f.family, f.status]);
    const widths = [...document.querySelectorAll('.cm-line')].filter((l) => l.textContent.trim()).map((l) => { const r = document.createRange(); r.selectNodeContents(l); return Math.round(r.getBoundingClientRect().width); });
    return { fontText: document.documentElement.style.getPropertyValue('--font-text').trim(), faces, widths,
      state: document.querySelector('[data-testid=font-file-state]')?.textContent ?? null };
  })()`);

/** The note text is in the font file: one face, loaded, named first in --font-text, and the two lines are as wide. */
async function usesFont(what) {
  let l;
  await eventually(async () => (l = await look()).faces.length === 1 && l.faces[0][1] === "loaded" && l.widths.length === 2, { message: `${what}: font loaded`, timeout: 20000 }).catch((e) => {
    throw new Error(`${e.message} ${JSON.stringify(l)}`);
  });
  assert.equal(l.fontText, `"${l.faces[0][0]}", var(--font-ui)`, what);
  assert.ok(Math.abs(l.widths[0] - l.widths[1]) <= 1, `${what}: lines of i and M as wide: ${l.widths}`);
}

const md5Of = (sh, file) => sh(`md5sum ${file} 2>/dev/null || true`).trim().split(/\s+/)[0];

test("the Theme list: a theme is applied and saved, System shows the Light and Dark lists, and nothing sticks out", async () => {
  await d.createAppVault("Looks");
  writeAppFile(`${VAULT}/Lines.md`, LINES);
  await rescan(d);
  await captureErrors(d);
  await openAppearance();
  const list = await d.eval(`[...document.querySelector('[data-testid=theme-select]').options].map((o) => o.value)`);
  assert.deepEqual(list, ["system", "limestone", "marble", "high-contrast-light", "slate", "graphite", "high-contrast-dark"]);
  await pickIn("theme-select", "high-contrast-dark");
  await eventually(async () => (await d.eval(`getComputedStyle(document.documentElement).getPropertyValue('--bg').trim()`)).replace(/^#000$/, "#000000") === "#000000", { message: "High contrast dark applied" });
  await eventually(() => /"darkTheme": "high-contrast-dark"/.test(readAppFile(`${VAULT}/.cairn/settings.json`)), { message: "saved" });
  assert.match(readAppFile(`${VAULT}/.cairn/settings.json`), /"theme": "dark"/);
  await d.shot("appearance-high-contrast-dark-settings.png");
  await pickIn("theme-select", "system");
  await d.waitFor(`!!document.querySelector('[data-testid=theme-light-select]') && !!document.querySelector('[data-testid=theme-dark-select]')`);
  assert.deepEqual(await d.eval(`[document.querySelector('[data-testid=theme-light-select]').value, document.querySelector('[data-testid=theme-dark-select]').value]`), ["limestone", "high-contrast-dark"]);
  assert.ok(await d.eval(`(() => { const s = document.querySelector('[data-testid=settings] section'); return s.scrollWidth <= s.clientWidth + 1; })()`), "no sideways scroll");
  await d.shot("appearance-system-settings.png");
  assert.deepEqual(await pageErrors(d), []);
  await closeSettings(d);
});

test("app storage: a font file is saved in .cairn/fonts/ and used for note text, also after a restart", { skip: !FONT && `no ${TTF}` }, async () => {
  await d.openNote("Lines.md");
  await openAppearance();
  // No type filter on Android: storage apps label fonts in many ways.
  assert.equal(await d.eval(`document.querySelector('[data-testid=font-input]').accept`), "");
  await pickFont("Mono.ttf", FONT);
  await eventually(async () => (await look()).state === "Mono.ttf is in use.", { message: "in use", timeout: 20000 });
  assert.equal(md5Of(runAs, `${VAULT}/.cairn/fonts/Mono.ttf`), MD5, "the file in app storage");
  await eventually(() => /"textFont": "Mono.ttf"/.test(readAppFile(`${VAULT}/.cairn/settings.json`)), { message: "textFont saved" });
  await closeSettings(d);
  await usesFont("app storage");
  await d.shot("appearance-font-app.png");
  // Start again: the vault reopens and the font is read from it.
  adb("shell", "am", "force-stop", PKG);
  await d.launch();
  await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 30000);
  await d.openNote("Lines.md");
  await usesFont("app storage, after a restart");
  assert.deepEqual((await toastTexts(d)).filter((t) => /font/i.test(t)), []);
});

test("shared folder: the font file is written and read through the Storage Access Framework", { skip: !FONT && `no ${TTF}` }, async () => {
  fillSharedFolder(F, { "Lines.md": LINES });
  await d.toWelcome();
  await d.pickSafFolder(LABEL);
  await captureErrors(d);
  await d.openNote("Lines.md");
  await openAppearance();
  await pickFont("Mono.ttf", FONT);
  await eventually(async () => (await look()).state === "Mono.ttf is in use.", { message: "in use", timeout: 30000 });
  assert.equal(md5Of(devSh, `${F}/.cairn/fonts/Mono.ttf`), MD5, "the file in the shared folder");
  await closeSettings(d);
  await usesFont("shared folder");
  // Replaced by another file: the old one goes to the vault's .trash.
  await openAppearance();
  await pickFont("Mono2.ttf", FONT);
  await eventually(async () => (await look()).state === "Mono2.ttf is in use.", { message: "Mono2 in use", timeout: 30000 });
  await eventually(() => devSh(`ls ${F}/.cairn/fonts`).trim() === "Mono2.ttf", { message: "Mono.ttf moved out" });
  assert.match(devSh(`ls ${F}/.trash`), /Mono\.ttf/);
  assert.deepEqual(await pageErrors(d), [], "while writing, replacing and trashing the font");
  // Saved 300 ms after the change: wait for it before the app is stopped.
  await eventually(() => /"textFont": "Mono2\.ttf"/.test(devSh(`cat ${F}/.cairn/settings.json 2>/dev/null || true`)), { message: "Mono2.ttf saved" });
  await closeSettings(d);
  // Start again: the settings name the file, which is read by name (a shared folder's .cairn is not listed).
  adb("shell", "am", "force-stop", PKG);
  await d.launch();
  await captureErrors(d);
  await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 60000);
  await d.openNote("Lines.md");
  await usesFont("shared folder, after a restart");
  assert.deepEqual(await pageErrors(d), []);
});

test("the system's file picker opens for any type of file and hands the picked font over", { skip: !FONT && `no ${TTF}` }, async () => {
  // At the top of the device's storage, under a name that no other page of the picker shows.
  devSh(`mkdir -p ${PICK}`);
  pushFile(`${PICK}/PickedMono.ttf`, FONT);
  await openAppearance();
  await tap("[data-testid=font-choose]");
  await sleep(2500);
  await d.shot("appearance-picker-open.png");
  // The picker opens on Recent, which lists only files the media store knows: go to the
  // device's storage through the side drawer, again if a tap lands while it still moves.
  for (let i = 0; ; i++) {
    try {
      await uiClick(/^AFontPick$/, 3000);
      break;
    } catch (e) {
      if (i === 3) throw e;
    }
    await uiClickDesc(/Show roots/i, 4000);
    await sleep(1500);
    await uiClick(/^sdk_gphone|^Android SDK|^Internal storage|^SDK/i, 4000);
    await sleep(1500);
  }
  await sleep(1000);
  await uiClick(/^PickedMono\.ttf$/);
  await d.attach();
  await eventually(async () => (await look()).state === "PickedMono.ttf is in use.", { message: "picked font in use", timeout: 30000 });
  assert.equal(md5Of(devSh, `${F}/.cairn/fonts/PickedMono.ttf`), MD5);
});
