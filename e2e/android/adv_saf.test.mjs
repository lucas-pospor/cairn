// Adversarial tests: vaults in a shared folder picked with the system picker
// (Storage Access Framework). Data loss and file-name handling.
//
//   . scripts/android-env.sh
//   node --test --test-concurrency=1 e2e/android/adv_saf.test.mjs
//
// Needs one emulator/device in `adb devices` (the debug APK is installed if
// missing). Shared storage on the emulator is case-insensitive (FUSE over
// casefolded ext4), like most phones. Clears the app's data and uses
// /sdcard/Documents/AdvSafT on the device.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import {
  Device, adb, devSh, sleep, eventually, appPid, runAs, key, APK, ls,
  deleteEntry, renameEntry, newNote, treeMenu, treePaths, reopenSaf, pushFile, readSettings, restoreSettings, kill9, fileSize, PKG,
} from "./adv_helpers.mjs";

const LABEL = "AdvSafT";
const F = `/sdcard/Documents/${LABEL}`;
const d = new Device();
let settings;

before(async () => {
  settings = readSettings();
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  devSh(`mkdir -p ${F}`);
  await d.fresh();
  devSh(`cd ${F} && rm -rf ./* ./.trash ./.cairn; printf 'seed' > ${F}/Seed.md`);
  await d.pickSafFolder(LABEL);
});

after(async () => {
  restoreSettings(settings);
  await d.shot("adv-saf-final.png").catch(() => {});
  d.close();
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

/** Make sure the app runs and shows the SAF vault with exactly these files. */
async function fresh(files) {
  await d.launch(); // also brings the app back to the front
  if (!(await d.isWelcome())) await d.toWelcome();
  if (!(await d.eval(`[...document.querySelectorAll('.recent-open')].some(b => b.textContent.includes(${JSON.stringify(LABEL)}))`))) await d.pickSafFolder(LABEL);
  await reopenSaf(d, LABEL, F, files);
}

test("SAF: overwriting a long note with a short one leaves no stale bytes on disk", async () => {
  const long = "# Long\n" + "x".repeat(5000) + "\nEND-OF-LONG\n";
  await fresh({ "Long.md": long });
  await d.openNote("Long.md", { contains: "END-OF-LONG" });
  await d.replaceAll("short");
  await eventually(() => devSh(`cat ${F}/Long.md`) === "short", { message: "short text on disk" });
  assert.equal(devSh(`stat -c %s ${F}/Long.md`).trim(), "5");
});

test("SAF: a note changed by another app reloads when clean and shows the conflict banner when dirty", async () => {
  await fresh({ "Ext.md": "original" });
  await d.openNote("Ext.md", { contains: "original" });
  devSh(`printf 'changed outside 1' > ${F}/Ext.md`);
  // Android has no watcher: picked up by the 20 s rescan.
  await eventually(async () => (await d.doc()) === "changed outside 1", { timeout: 30000, message: "clean tab reloads" });
  devSh(`printf 'changed outside 2' > ${F}/Ext.md`);
  await d.append(" + my edit");
  await d.waitFor(`!!document.querySelector('[data-testid=conflict-banner]')`, 10000);
  assert.equal(devSh(`cat ${F}/Ext.md`), "changed outside 2", "the outside edit must not be overwritten");
  await d.click("[data-testid=conflict-mine]");
  await eventually(() => devSh(`cat ${F}/Ext.md`) === "changed outside 1 + my edit", { message: "keep mine writes the editor text" });
});

test("SAF: a clean note deleted by another app closes its tab", async () => {
  await fresh({ "Gone.md": "bye", "Stay.md": "stay" });
  await d.openNote("Gone.md", { contains: "bye" });
  devSh(`rm ${F}/Gone.md`);
  await d.waitFor(`document.querySelector('.mobile-title')?.textContent.trim() !== 'Gone'`, 30000);
  assert.deepEqual(ls(F).sort(), ["Stay.md"]);
});

test("SAF: deleting a note that was deleted before (same name already in .trash)", async () => {
  await fresh({ "Keep.md": "k" });
  for (let i = 0; i < 2; i++) {
    await newNote(d, "Untitled");
    await d.append(`version ${i}\n`);
    await eventually(() => devSh(`cat ${F}/Untitled.md 2>/dev/null || true`).includes(`version ${i}`), { message: `version ${i} saved` });
    await deleteEntry(d, "Untitled.md");
    await sleep(2500);
  }
  const toasts = await d.eval(`document.body.innerText.match(/Something named .*already exists\\./)?.[0] ?? null`);
  const root = ls(F);
  const trash = ls(`${F}/.trash`);
  console.log("root:", root, "trash:", trash, "toast:", toasts);
  assert.ok(!root.includes("Untitled.md"), `second delete must remove Untitled.md from the vault (root: ${root.join(", ")}; toast: ${toasts})`);
  assert.equal(trash.length, 2, `both versions must be in .trash (trash: ${trash.join(", ")})`);
});

test("SAF: renaming a note by changing only the case of its name", async () => {
  await fresh({ "Hello.md": "hello there" });
  await d.click("[data-testid=mobile-files]");
  await renameEntry(d, "Hello.md", "hello");
  await sleep(3000);
  const disk = ls(F);
  const err = await d.eval(`document.body.innerText.match(/Not found: [^\\n]*/)?.[0] ?? null`);
  console.log("disk:", disk, "tree:", await treePaths(d), "error:", err);
  assert.deepEqual(disk, ["hello.md"], `expected exactly hello.md on disk, got ${JSON.stringify(disk)} (error toast: ${err})`);
});

test("SAF: a note whose name is in decomposed Unicode (NFD, e.g. from macOS) can be opened", async () => {
  const nfd = "Café.md";
  await fresh({ [nfd]: "nfd content", "Thé.md": "nfc content" });
  await d.click("[data-testid=mobile-files]");
  // Vault paths are NFC, so the tree may list the note under its NFC name.
  const row = (await treePaths(d)).find((p) => p.normalize("NFD").startsWith("Cafe"));
  assert.ok(row, "NFD note listed");
  await d.eval(`document.querySelector('[data-testid=tree-row][data-path=${JSON.stringify(JSON.stringify(row)).slice(1, -1)}]').click()`);
  await sleep(2500);
  const err = await d.tabError();
  console.log("tab error:", err);
  assert.equal(err, null, `opening ${JSON.stringify(row)} failed: ${err}`);
  assert.equal(await d.doc(), "nfd content");
});

test("SAF: a note deleted by another app while it has unsaved edits", async () => {
  await fresh({ "Dirty.md": "dirty base" });
  await d.openNote("Dirty.md", { contains: "dirty base" });
  devSh(`rm ${F}/Dirty.md`);
  await d.append(" + unsaved");
  await d.waitFor(`!!document.querySelector('[data-testid=conflict-banner]')`, 10000);
  const banner = await d.eval(`document.querySelector('[data-testid=conflict-banner]').textContent`);
  console.log("banner:", banner);
  // The file list drops the note now, not at the next periodic rescan (20 s).
  const listed = await d
    .waitFor(`!document.querySelector('[data-testid=tree-row][data-path="Dirty.md"]')`, 3000)
    .then(() => false, () => true);
  // Keeping my version must bring the file back either way.
  await d.eval(`[...document.querySelectorAll('[data-testid=conflict-banner] button')].find(b => /Keep mine|Save my version/.test(b.textContent)).click()`);
  await eventually(() => devSh(`cat ${F}/Dirty.md 2>/dev/null || true`) === "dirty base + unsaved", { message: "file restored" });
  assert.match(banner, /deleted/, "the banner should say the note was deleted");
  assert.equal(listed, false, "the deleted note should leave the file list within 3 s");
});

test("SAF: the vault is open again after Android restarts the app", async () => {
  await fresh({ "Restart.md": "restart me" });
  await d.openNote("Restart.md", { contains: "restart me" });
  key("KEYCODE_HOME");
  await sleep(800);
  runAs(`kill -9 ${appPid()}`); // what the low-memory killer does to a background app
  await sleep(800);
  await d.launch();
  await sleep(4000);
  const welcome = await d.isWelcome();
  const title = await d.eval(`document.querySelector('.mobile-title')?.textContent ?? null`);
  console.log("after restart: welcome:", welcome, "title:", title);
  await d.shot("adv-saf-after-restart.png");
  assert.equal(welcome, false, "expected the SAF vault to be reopened, got the welcome screen");
  assert.equal(title, "Restart");
});

test("SAF: changing the system font size keeps the vault open", async () => {
  await fresh({ "Font.md": "font" });
  await d.openNote("Font.md", { contains: "font" });
  const before = devSh("settings get system font_scale").trim();
  const textSize = () => d.eval(`parseFloat(getComputedStyle(document.querySelector('.cm-content')).fontSize)`);
  const unscaled = (await textSize()) / (before === "null" ? 1 : parseFloat(before));
  devSh("settings put system font_scale 1.3");
  try {
    await sleep(4000);
    await d.attach();
    await sleep(1500);
    const welcome = await d.isWelcome();
    const title = await d.eval(`document.querySelector('.mobile-title')?.textContent ?? null`);
    console.log("after font change: welcome:", welcome, "title:", title);
    assert.equal(welcome, false, "expected the SAF vault to stay open after a font size change");
    assert.equal(title, "Font");
    // The app handles the change without being recreated, so it must pass the new scale to the web view.
    const size = await textSize();
    assert.ok(Math.abs(size - unscaled * 1.3) < 0.5, `editor text should follow the font size: ${unscaled}px at 100%, ${size}px at 130%`);
  } finally {
    devSh(`settings put system font_scale ${before === "null" ? "1.0" : before}`);
    await sleep(3000);
    await d.attach().catch(() => {});
  }
});

test("SAF: changing the display size keeps the vault open", async () => {
  await fresh({ "Dense.md": "dense" });
  await d.openNote("Dense.md", { contains: "dense" });
  const density = devSh("wm density");
  const override = density.match(/Override density: (\d+)/);
  const current = Number((override ?? density.match(/Physical density: (\d+)/))[1]);
  const width = await d.eval(`innerWidth`);
  devSh(`wm density ${Math.round((current * 4) / 3)}`); // a larger display size
  try {
    await sleep(4000);
    await d.attach();
    await sleep(1500);
    const welcome = await d.isWelcome();
    const r = await d.eval(`({ title: document.querySelector('.mobile-title')?.textContent ?? null, width: innerWidth })`);
    console.log("after display size change: welcome:", welcome, JSON.stringify(r), "width before:", width);
    assert.equal(welcome, false, "expected the SAF vault to stay open after a display size change");
    assert.equal(r.title, "Dense");
    assert.ok(r.width < width, "the layout should follow the larger display size");
  } finally {
    devSh(override ? `wm density ${override[1]}` : "wm density reset");
    await sleep(3000);
    await d.attach().catch(() => {});
  }
});

test("SAF: pressing Back right after typing in a small note saves the edit", async () => {
  await fresh({ "Tiny.md": "tiny base\n" });
  await d.openNote("Tiny.md", { contains: "tiny base" });
  await d.append("typed just before Back\n");
  key("KEYCODE_BACK");
  await eventually(() => !appPid(), { timeout: 15000, message: "process exited after Back" });
  const text = devSh(`cat ${F}/Tiny.md`);
  console.log("small SAF note after Back:", JSON.stringify(text));
  assert.equal(text, "tiny base\ntyped just before Back\n");
});

// FINDING-026: a write that opens the note with "rwt" empties it before the
// new bytes arrive, and Back ends the process mid-write, which would leave the
// note at 0 bytes. The note must hold its old or its new text.
test("SAF: pressing Back right after editing a large note keeps the note on disk", async (t) => {
  const line = "The quick brown fox jumps over the lazy dog, line padding text here.\n";
  const big = "# Big\n" + line.repeat(Math.ceil((12 * 1024 * 1024) / line.length)) + "END\n";
  await fresh({ "Small.md": "s" });
  await d.toWelcome();
  pushFile(`${F}/Big.md`, big);
  await d.openRecent(LABEL);
  await d.openNote("Big.md");
  await eventually(async () => (await d.eval(`document.querySelector('.cm-editor').__cairnView.state.doc.length`)) === big.length, { timeout: 60000, message: "big note loaded" });
  await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: 0, insert: "MARK\\n" }, userEvent: 'input.type' }); })()`);
  key("KEYCODE_BACK");
  await eventually(() => !appPid(), { timeout: 15000, message: "process exited after Back" });
  const size = Number(devSh(`stat -c %s ${F}/Big.md`).trim());
  console.log("on disk after Back:", size, "bytes; before:", big.length, "expected:", big.length + 5);
  // Never acceptable: the note shrinks below its old content.
  assert.ok(size >= big.length, `note truncated on disk: ${size} bytes left of ${big.length}`);
  const sha1 = (s) => crypto.createHash("sha1").update(s).digest("hex");
  const onDisk = devSh(`sha1sum ${F}/Big.md`).split(/\s/)[0];
  assert.ok([sha1(big), sha1("MARK\n" + big)].includes(onDisk), "the note on disk is either the old or the new text, not a mix");
  await t.test("the edit made just before Back is saved", () => {
    assert.equal(size, big.length + 5, "the edit made before Back should be saved");
  });
});

test("SAF: the shared folder is case-insensitive, so a new note 'hello' next to 'Hello.md' is refused or renamed visibly", async () => {
  await fresh({ "Hello.md": "existing hello" });
  await d.click(".mobile-bar [title='New note']");
  await d.waitFor(`!!document.querySelector('[data-testid=dialog-input]')`);
  await d.setValue("[data-testid=dialog-input]", "hello");
  await d.click("[data-testid=dialog-ok]");
  await sleep(3000);
  const disk = ls(F);
  const title = await d.eval(`document.querySelector('.mobile-title')?.textContent.trim() ?? null`);
  const err = await d.tabError();
  const toasts = await d.eval(`[...document.querySelectorAll('.toast')].map(t => t.textContent.trim())`);
  console.log("disk:", disk, "title:", title, "tab error:", err, "toasts:", toasts, "Hello.md:", JSON.stringify(devSh(`cat ${F}/Hello.md`)));
  assert.equal(devSh(`cat ${F}/Hello.md`), "existing hello", "the existing note is untouched");
  assert.ok(disk.every((f) => f.endsWith(".md")), `no stray non-note files: ${JSON.stringify(disk)}`);
  assert.equal(err, null, "whatever note was opened can be read");
});

test("SAF: opening another note while the conflict banner is up does not throw away the unsaved edits", async () => {
  await fresh({ "Mine.md": "mine base\n", "Other.md": "other note\n" });
  await d.openNote("Mine.md", { contains: "mine base" });
  devSh(`printf 'changed by another app\\n' > ${F}/Mine.md`);
  await d.append("my precious unsaved paragraph\n");
  await d.waitFor(`!!document.querySelector('[data-testid=conflict-banner]')`, 10000);
  // The user taps another note in the drawer (the phone layout reuses the one tab).
  await d.openNote("Other.md", { contains: "other note" });
  await sleep(1500);
  const disk = devSh(`cat ${F}/Mine.md`);
  const anyFile = devSh(`grep -rl 'my precious unsaved paragraph' ${F} 2>/dev/null || true`).trim();
  const tabs = await d.eval(`[...document.querySelectorAll('[data-testid=tab], .tab')].map(t => t.textContent.trim())`);
  // Coming back to Mine.md: are the edits still there?
  await d.openNote("Mine.md");
  await sleep(1000);
  const back = await d.doc();
  console.log("Mine.md on disk:", JSON.stringify(disk), "| files containing the edit:", JSON.stringify(anyFile), "| tabs:", JSON.stringify(tabs), "| Mine.md reopened:", JSON.stringify(back));
  assert.ok(anyFile || back.includes("my precious unsaved paragraph"), "the unsaved paragraph must survive somewhere (kept in a tab, saved, or the user asked)");
});
