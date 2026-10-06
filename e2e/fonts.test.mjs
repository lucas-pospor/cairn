// A font file for note text, in the real app: Settings > Appearance > Font
// file saves the picked file in .cairn/fonts/ and the note text uses it, also
// after a restart; another file replaces it and Remove takes it away, each
// moving the old file to the trash; files that are not a usable font are
// refused with a toast and nothing saved; a missing or broken font file falls
// back to the Text font with a toast; a textFont value of a later version stays
// in the file; and the controls fit a phone-sized window.
//
//   scripts/e2e-headless.sh e2e/fonts.test.mjs
//
// The fonts come from the system (Liberation Mono, a monospaced TTF), and a
// WOFF2 copy is made with woff2_compress; a test that needs one skips without it.

import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { execFileSync } from "node:child_process";
import { launch, freshEnv, eventually, sleep } from "./adv_editor_lib.mjs";

const TTF = "/usr/share/fonts/liberation/LiberationMono-Regular.ttf";
const SETTINGS = ".cairn/settings.json";
const SERIF = 'Charter, "Iowan Old Style", "Source Serif 4", Georgia, "Noto Serif", serif';
const NOTES = { "Lines.md": "iiiiiiiiiiiiiiii\n\nMMMMMMMMMMMMMMMM\n" };

const envs = [];
function env(settings) {
  const e = freshEnv({ ...NOTES, ...(settings ? { [SETTINGS]: JSON.stringify(settings, null, 2) } : {}) });
  envs.push(e);
  return e;
}
after(async () => {
  for (const e of envs) await e.cleanup();
});

const haveTtf = fs.existsSync(TTF);
/** A WOFF2 copy of the TTF, made once, or null without woff2_compress. */
const woff2 = (() => {
  if (!haveTtf) return null;
  try {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-fonts-"));
    fs.copyFileSync(TTF, path.join(dir, "Mono.ttf"));
    execFileSync("woff2_compress", [path.join(dir, "Mono.ttf")], { stdio: "ignore" });
    const bytes = fs.readFileSync(path.join(dir, "Mono.woff2"));
    fs.rmSync(dir, { recursive: true, force: true });
    return bytes;
  } catch {
    return null;
  }
})();

const saved = (e) => JSON.parse(e.vault.read(SETTINGS));
const fontsIn = (e) => (e.vault.exists(".cairn/fonts") ? fs.readdirSync(e.vault.p(".cairn/fonts")).sort() : []);

async function openAppearance(app) {
  if (!(await app.exec(`return !!document.querySelector('[data-testid=settings]')`))) await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings-appearance]')`);
  await app.exec(`document.querySelector('[data-testid=settings-appearance]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=font-choose]')`);
}

async function closeSettings(app) {
  await app.exec(`document.querySelector('[data-testid=settings] .close').click(); return 1`);
  await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
}

/** Pick `bytes` as `name` in the Font file's file input, as the system's file picker does. */
const pickFont = (app, name, bytes) =>
  app.exec(
    `const b = Uint8Array.from(atob(arguments[0]), (c) => c.charCodeAt(0));
     const dt = new DataTransfer(); dt.items.add(new File([b], arguments[1]));
     const i = document.querySelector('[data-testid=font-input]'); i.files = dt.files;
     i.dispatchEvent(new Event('change', { bubbles: true })); return 1`,
    Buffer.from(bytes).toString("base64"),
    name,
  );

/** The font of the note text: --font-text as set on <html> (the computed value has var() filled in), the faces Cairn added, the state line, and how wide the two lines are. */
const look = (app) =>
  app.exec(
    `const faces = [...document.fonts].filter((f) => f.family.startsWith('cairn-text-font-')).map((f) => [f.family, f.status]);
     const lines = [...document.querySelectorAll('.cm-line')].filter((l) => l.textContent.trim()).map((l) => { const r = document.createRange(); r.selectNodeContents(l); return Math.round(r.getBoundingClientRect().width); });
     return { fontText: document.documentElement.style.getPropertyValue('--font-text').trim(), faces, state: document.querySelector('[data-testid=font-file-state]')?.textContent ?? null,
       scroller: getComputedStyle(document.querySelector('.cm-scroller')).fontFamily, widths: lines };`,
  );

async function openNote(app, p) {
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="' + arguments[0] + '"]').click(); return 1`, p);
  await eventually(async () => (await app.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path`)) === p, { message: `${p} open` });
  await app.s.waitFor(`return document.querySelectorAll('.cm-line').length >= 3`);
}

/** The note text is in the monospaced font file: both lines are as wide, and --font-text names its face before the Text font. */
function assertMono(l, fallback, what) {
  assert.equal(l.faces.length, 1, `${what}: one face: ${JSON.stringify(l.faces)}`);
  const [[family, status]] = l.faces;
  assert.equal(status, "loaded", what);
  assert.equal(l.fontText, `"${family}", ${fallback}`, what);
  assert.ok(Math.abs(l.widths[0] - l.widths[1]) <= 1, `${what}: lines of i and M as wide: ${l.widths}`);
}

test("a font file picked in Settings is saved in .cairn/fonts/, used for note text, and used again after a restart", { skip: !haveTtf && `no ${TTF}` }, async () => {
  const e = env({ fontFamily: "serif" });
  let app = await launch({ vault: e.vault.root, xdg: e.xdg });
  try {
    await openNote(app, "Lines.md");
    const before = await look(app);
    assert.equal(before.fontText, SERIF);
    assert.ok(before.widths[1] > before.widths[0] * 1.5, `serif: M is wider than i: ${before.widths}`);
    await openAppearance(app);
    assert.equal(await app.exec(`return document.querySelector('[data-testid=font-choose]').getAttribute('aria-label')`), "Choose a font file");
    assert.equal(await app.exec(`return document.querySelector('[data-testid=font-input]').accept`), ".woff2,.woff,.ttf,.otf");
    await pickFont(app, "LiberationMono-Regular.ttf", fs.readFileSync(TTF));
    await eventually(() => e.vault.exists(SETTINGS) && saved(e).textFont === "LiberationMono-Regular.ttf", { message: "textFont saved" });
    assert.deepEqual(fs.readFileSync(e.vault.p(".cairn/fonts/LiberationMono-Regular.ttf")), fs.readFileSync(TTF));
    assert.equal(saved(e).fontFamily, "serif", "the Text font stays");
    await eventually(async () => (await look(app)).state === "LiberationMono-Regular.ttf is in use.", { message: "in use" });
    assert.equal(await app.exec(`return document.querySelector('[data-testid=font-choose]').getAttribute('aria-label')`), "Change the font file");
    await closeSettings(app);
    assertMono(await look(app), SERIF, "after picking it");
    assert.deepEqual(await app.toasts(), []);
    await app.stop();
    app = await launch({ vault: e.vault.root, xdg: e.xdg });
    await openNote(app, "Lines.md");
    await eventually(async () => (await look(app)).faces.length === 1 && (await look(app)).faces[0][1] === "loaded", { message: "loaded after a restart" });
    assertMono(await look(app), SERIF, "after a restart");
    // Leaving the vault lets the font go.
    await app.exec(`document.querySelector('.status .vault').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('.welcome')`, { message: "welcome screen" });
    const left = await app.exec(`return { faces: [...document.fonts].filter((f) => f.family.startsWith('cairn-text-font-')).length, fontText: document.documentElement.style.getPropertyValue('--font-text').trim() }`);
    assert.deepEqual(left, { faces: 0, fontText: "var(--font-ui)" });
  } finally {
    await app.stop();
  }
});

test("another font file replaces it and Remove takes it away, each moving the old file to the trash", { skip: !woff2 && "no woff2_compress or no TTF" }, async () => {
  const e = env({ fontFamily: "serif" });
  const app = await launch({ vault: e.vault.root, xdg: e.xdg });
  const trash = (name) => fs.existsSync(path.join(e.xdg, "data", "Trash", "files", name));
  try {
    await openNote(app, "Lines.md");
    await openAppearance(app);
    await pickFont(app, "First.ttf", fs.readFileSync(TTF));
    await eventually(async () => (await look(app)).state === "First.ttf is in use.", { message: "First in use" });
    await pickFont(app, "Second.woff2", woff2);
    await eventually(async () => (await look(app)).state === "Second.woff2 is in use.", { message: "Second in use" });
    await eventually(() => fontsIn(e).join() === "Second.woff2", { message: "First moved out" });
    assert.ok(trash("First.ttf"), "First.ttf is in the trash");
    await eventually(() => saved(e).textFont === "Second.woff2", { message: "Second saved" });
    assertMono(await look(app), SERIF, "the woff2 file");
    // The same name again: saved over and loaded again.
    const [[family]] = (await look(app)).faces;
    await pickFont(app, "Second.woff2", woff2);
    await eventually(async () => (await look(app)).faces.length === 1 && (await look(app)).faces[0][0] !== family, { message: "loaded again" });
    assert.deepEqual(fontsIn(e), ["Second.woff2"]);
    // A name that differs only in case is the same file on macOS, Windows and Android
    // shared folders: written over under the name in use, nothing trashed.
    const [[again]] = (await look(app)).faces;
    await pickFont(app, "SECOND.WOFF2", woff2);
    await eventually(async () => (await look(app)).faces.length === 1 && (await look(app)).faces[0][0] !== again, { message: "loaded again, case" });
    await sleep(600);
    assert.deepEqual(fontsIn(e), ["Second.woff2"]);
    assert.equal(saved(e).textFont, "Second.woff2");
    assert.ok(!trash("SECOND.WOFF2") && !trash("Second.woff2"), "nothing in the trash");
    // Remove.
    await app.exec(`document.querySelector('[data-testid=font-remove]').click(); return 1`);
    await eventually(() => !("textFont" in saved(e)), { message: "textFont removed" });
    assert.deepEqual(fontsIn(e), []);
    assert.ok(trash("Second.woff2"), "Second.woff2 is in the trash");
    const l = await look(app);
    assert.deepEqual({ fontText: l.fontText, faces: l.faces, state: l.state }, { fontText: SERIF, faces: [], state: null });
    assert.ok((await app.toasts()).includes("Second.woff2 moved to the trash. Notes use the Text font."));
    assert.equal(await app.exec(`return document.activeElement?.dataset.testid ?? null`), "font-choose", "focus stays in the row");
    assert.equal(await app.exec(`return !!document.querySelector('[data-testid=font-remove]')`), false);
  } finally {
    await app.stop();
  }
});

test("files that are not a font Cairn can use are refused with a toast, and nothing is saved", { skip: !haveTtf && `no ${TTF}` }, async () => {
  const e = env({ fontFamily: "serif" });
  const app = await launch({ vault: e.vault.root, xdg: e.xdg });
  try {
    await openAppearance(app);
    const ttf = fs.readFileSync(TTF);
    const cases = [
      ["Fonts.ttc", ttf, "Cannot use Fonts.ttc: Cairn takes woff2, woff, ttf and otf font files."],
      ["Notes.pdf", Buffer.from("%PDF-1.7 not a font at all"), "Cannot use Notes.pdf: Cairn takes woff2, woff, ttf and otf font files."],
      ["Fake.ttf", Buffer.from("%PDF-1.7 not a font at all"), "Cannot use Fake.ttf: This is not a woff2, woff, ttf or otf font file."],
      ["Collection.otf", Buffer.concat([Buffer.from("ttcf"), ttf.subarray(4)]), "Cannot use Collection.otf: Font collections (ttc) are not supported. Choose a woff2, woff, ttf or otf file."],
      ["Broken.woff2", Buffer.concat([Buffer.from("wOF2"), Buffer.alloc(2000, 7)]), "Cannot use Broken.woff2: The web view cannot read the font in this file."],
    ];
    for (const [name, bytes, toast] of cases) {
      await pickFont(app, name, bytes);
      await eventually(async () => (await app.toasts()).some((t) => t.startsWith(toast)), { message: `${name}: toast "${toast}"` });
    }
    // Over 20 MB: refused by its size, before it is read.
    await app.exec(
      `const big = new Uint8Array(20 * 1024 * 1024 + 1); big.set([119, 79, 70, 50]);
       const dt = new DataTransfer(); dt.items.add(new File([big], 'Huge.woff2'));
       const i = document.querySelector('[data-testid=font-input]'); i.files = dt.files; i.dispatchEvent(new Event('change', { bubbles: true })); return 1`,
    );
    await eventually(async () => (await app.toasts()).includes("Cannot use Huge.woff2: The font file is 21 MB; Cairn takes font files up to 20 MB."), { message: "size toast" });
    await sleep(600);
    assert.deepEqual(fontsIn(e), []);
    assert.deepEqual(await app.invokes("write_config_bytes"), []);
    assert.equal(saved(e).textFont, undefined);
    assert.equal(await app.exec(`return document.querySelector('[data-testid=font-file-state]')`), null);
  } finally {
    await app.stop();
  }
});

test("a font file that is missing or broken falls back to the Text font with a toast; a value of a later version stays in the file", async () => {
  // Missing (Cairn sync does not copy .cairn/, so another device may lack it), and broken.
  for (const [name, bytes, why] of [
    ["Gone.woff2", null, "It is not in .cairn/fonts/."],
    ["Bad.ttf", Buffer.from("not a font, renamed"), "This is not a woff2, woff, ttf or otf font file."],
  ]) {
    const e = env({ fontFamily: "mono", textFont: name });
    if (bytes) e.vault.write(`.cairn/fonts/${name}`, bytes);
    const app = await launch({ vault: e.vault.root, xdg: e.xdg });
    try {
      const toast = `Could not use the font file ${name}: ${why} Notes use the Text font, Monospace, instead.`;
      await eventually(async () => (await app.toasts()).includes(toast), { message: `${name}: toast` });
      await openNote(app, "Lines.md");
      const l = await look(app);
      assert.deepEqual({ fontText: l.fontText, faces: l.faces }, { fontText: "var(--font-mono)", faces: [] }, name);
      await openAppearance(app);
      assert.equal((await look(app)).state, `${name} cannot be used: ${why} Notes use the Text font.`);
    } finally {
      await app.stop();
    }
  }
  // Remove with the file missing: the setting goes, and nothing is said to be in the trash.
  {
    const e = env({ fontFamily: "mono", textFont: "Gone.woff2" });
    const app = await launch({ vault: e.vault.root, xdg: e.xdg });
    try {
      await openAppearance(app);
      await app.exec(`document.querySelector('[data-testid=font-remove]').click(); return 1`);
      await eventually(() => !("textFont" in saved(e)), { message: "textFont removed" });
      await eventually(async () => (await app.toasts()).includes("Notes use the Text font."), { message: "toast" });
      assert.deepEqual((await app.toasts()).filter((t) => /trash/.test(t)), []);
    } finally {
      await app.stop();
    }
  }
  // A later version's value: no toast and no load (the state line says so: no font file is being
  // loaded or used), and it stays in the file when another setting is saved, with nothing read then.
  const later = { file: "Inter.woff2", weights: [400, 700] };
  const e = env({ textFont: later, fontSize: 15 });
  const app = await launch({ vault: e.vault.root, xdg: e.xdg });
  try {
    await openAppearance(app);
    assert.equal((await look(app)).state, "The font file setting holds no file name; notes use the Text font.");
    await app.exec(`const r = document.querySelector('[data-testid=settings] input[type=range]'); r.value = 18; r.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
    await eventually(() => saved(e).fontSize === 18, { message: "font size saved" });
    assert.deepEqual(saved(e).textFont, later);
    assert.deepEqual(await app.invokes("read_config_bytes"), []);
    assert.deepEqual(await app.toasts(), []);
  } finally {
    await app.stop();
  }
});

test("the Font file controls fit a phone-sized window", { skip: !haveTtf && `no ${TTF}` }, async () => {
  const e = env({});
  const app = await launch({ vault: e.vault.root, xdg: e.xdg });
  try {
    await openAppearance(app);
    // A name with no spaces or hyphens to break at, as variable fonts are often named.
    await pickFont(app, "RobotoFlex[GRAD,XOPQ,XTRA,YOPQ,YTAS,YTDE,YTFI,YTLC,YTUC,opsz,slnt,wdth,wght].ttf", fs.readFileSync(TTF));
    await eventually(async () => (await look(app)).state?.endsWith("is in use."), { message: "in use" });
    await app.s.cmd("POST", "/window/rect", { width: 480, height: 820 });
    const out = {};
    for (const zoom of [1.28, 1.5]) {
      await app.exec(`window.__TAURI_INTERNALS__.invoke('plugin:webview|set_webview_zoom', { label: 'main', value: arguments[0] }); return 1`, zoom);
      await eventually(() => app.exec(`return innerWidth <= (arguments[0] > 1.4 ? 320 : 400)`, zoom), { message: `zoom ${zoom}` });
      await app.exec(`document.querySelector('[data-testid=font-choose]').scrollIntoView({ block: 'center' }); return 1`);
      await sleep(200);
      out[zoom] = await app.exec(
        `const sec = document.querySelector('[data-testid=settings] section'), w = sec.getBoundingClientRect().right;
         const outside = [...document.querySelectorAll('[data-testid=font-choose], [data-testid=font-remove], [data-testid=font-file-state]')]
           .filter((el) => { const b = el.getBoundingClientRect(); return b.left < 0 || b.right > w + 0.5; }).map((el) => el.dataset.testid);
         return { innerWidth, overflow: sec.scrollWidth > sec.clientWidth + 1, outside };`,
      );
      await app.shot(`fonts-settings-${Math.round(out[zoom].innerWidth)}`);
    }
    for (const [zoom, o] of Object.entries(out)) assert.deepEqual({ overflow: o.overflow, outside: o.outside }, { overflow: false, outside: [] }, `zoom ${zoom} at ${o.innerWidth}px`);
  } finally {
    await app.stop();
  }
});
