// Adversarial tests: app lifecycle (process death, Back, rotation, memory
// trim, configuration changes, renderer crash) and touch usability (keyboard,
// toolbar, long-press menu, dark mode, large fonts, sync setup) with a vault
// in the app's own storage.
//
//   . scripts/android-env.sh
//   node --test --test-concurrency=1 e2e/android/adv_lifecycle.test.mjs
//
// Needs one emulator/device in `adb devices` (the debug APK is installed if
// missing). Clears the app's data. Device settings it changes (rotation, font
// scale, night mode) are put back at the end. Screenshots go to e2e/.tmp/AN/.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import {
  Device, adb, devSh, sleep, eventually, appPid, runAs, key, APK, APP_DATA, typeOnDevice, foregroundPackage, webViewTop,
  newNote, deleteEntry, writeAppFile, readAppFile, rescan, screenPixels, luma, readSettings, restoreSettings, kill9, PKG,
  cdpRaw, captureErrors, pageErrors, revealRow, fileSize, q,
} from "./adv_helpers.mjs";

const d = new Device();
const VAULT = `${APP_DATA}/vaults/Life`;
let settings;

before(async () => {
  settings = readSettings();
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  devSh("settings put system accelerometer_rotation 0");
  devSh("settings put system user_rotation 0");
  devSh("settings put system font_scale 1.0");
  devSh("cmd uimode night no");
  await d.fresh();
  await d.createAppVault("Life");
});

after(async () => {
  restoreSettings(settings);
  await d.shot("adv-lifecycle-final.png").catch(() => {});
  d.close();
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

/** App in front with the Life vault open and `name` (a fresh note with `content`) in the editor. */
async function note(name, content) {
  await d.launch(); // also brings the app back to the front
  if (await d.isWelcome()) await d.openRecent("Life");
  await d.eval(`document.querySelector('[data-testid=settings] .close')?.click(); document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`).catch(() => {});
  writeAppFile(`${VAULT}/${name}`, content);
  await rescan(d);
  await d.openNote(name, { contains: content.slice(0, 20) });
}

const disk = (name) => readAppFile(`${VAULT}/${name}`);

async function openDrawer() {
  if (await d.eval(`document.querySelector('aside.left').classList.contains('hidden')`)) await d.click("[data-testid=mobile-files]");
  await d.waitFor(`!document.querySelector('aside.left').classList.contains('hidden')`);
  await sleep(300);
}

// ---------------------------------------------------------------- process death

test("edits survive kill -9 once the autosave debounce has passed; Home flushes at once", async () => {
  await note("Kill.md", "kill base\n");
  await d.append("after debounce\n");
  await sleep(1500);
  kill9();
  await sleep(500);
  assert.match(disk("Kill.md"), /after debounce/);
  await d.launch();
  await d.openNote("Kill.md", { contains: "after debounce" });
  await d.append("before home\n");
  key("KEYCODE_HOME");
  kill9();
  await sleep(500);
  assert.match(disk("Kill.md"), /before home/, "Home must flush the pending edit before the process can be killed");
});

test("Back with a pending edit saves it and the vault opens again on the next start", async () => {
  await note("Back.md", "back base\n");
  await d.append("typed then back\n");
  key("KEYCODE_BACK");
  await eventually(() => foregroundPackage() !== PKG, { message: "app left" });
  await sleep(1500);
  assert.match(disk("Back.md"), /typed then back/);
  await d.launch();
  await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 20000);
});

// Edits younger than the 600 ms autosave debounce may be lost to a SIGKILL in
// the foreground (by design); anything older, and anything typed before the
// app went to the background, must be on disk, and the file must never be damaged.
test("process death: SIGKILL inside the debounce loses only the last edit and never damages the file", async () => {
  await note("Death1.md", "death base\n");
  await d.append("older edit\n");
  await sleep(1500);
  await d.append("newest edit");
  kill9();
  await sleep(800);
  const text = disk("Death1.md");
  console.log("after SIGKILL 0 ms after typing:", JSON.stringify(text));
  assert.ok(text === "death base\nolder edit\n" || text === "death base\nolder edit\nnewest edit", JSON.stringify(text));
});

test("process death: background then am kill / force-stop / SIGKILL keeps the edit", async () => {
  for (const [name, how] of [["Death2.md", "am kill"], ["Death3.md", "force-stop"], ["Death4.md", "kill -9"]]) {
    await note(name, "base\n");
    await d.append(`typed before ${how}`);
    key("KEYCODE_HOME");
    await sleep(500);
    if (how === "am kill") devSh(`am kill ${PKG}`);
    else if (how === "force-stop") devSh(`am force-stop ${PKG}`);
    else kill9();
    await sleep(800);
    console.log(`${how}: process ${appPid() ? "alive" : "gone"}, disk ${JSON.stringify(disk(name))}`);
    assert.equal(disk(name), `base\ntyped before ${how}`, how);
  }
});

test("memory trim (RUNNING_CRITICAL in front, COMPLETE in the background) loses nothing", async () => {
  await note("Trim.md", "trim base\n");
  const pid = appPid();
  await d.append("before critical\n");
  devSh(`am send-trim-memory ${PKG} RUNNING_CRITICAL`);
  await sleep(1500);
  await d.append("before complete\n");
  key("KEYCODE_HOME");
  await sleep(800);
  devSh(`am send-trim-memory ${PKG} COMPLETE`);
  await sleep(1500);
  assert.equal(appPid(), pid);
  assert.equal(disk("Trim.md"), "trim base\nbefore critical\nbefore complete\n");
  await d.launch();
  await d.openNote("Trim.md", { contains: "before complete" });
  assert.equal(await d.doc(), "trim base\nbefore critical\nbefore complete\n");
});

test("app storage: Back right after editing a 12 MB note never truncates the file (atomic writes)", async () => {
  const line = "The quick brown fox jumps over the lazy dog, line padding text here.\n";
  const big = "# Big\n" + line.repeat(Math.ceil((12 * 1024 * 1024) / line.length)) + "END\n";
  await d.launch();
  if (await d.isWelcome()) await d.openRecent("Life");
  devSh("rm -f /data/local/tmp/adv-big.md");
  const local = `${process.env.TMPDIR ?? "/tmp"}/adv-an-big-${process.pid}.md`;
  (await import("node:fs")).writeFileSync(local, big);
  adb("push", local, "/data/local/tmp/adv-big.md");
  (await import("node:fs")).rmSync(local);
  devSh("chmod 644 /data/local/tmp/adv-big.md");
  runAs(`cp /data/local/tmp/adv-big.md ${VAULT}/BigApp.md`);
  devSh("rm -f /data/local/tmp/adv-big.md");
  await rescan(d);
  await d.openNote("BigApp.md");
  await eventually(async () => (await d.eval(`document.querySelector('.cm-editor').__cairnView.state.doc.length`)) === big.length, { timeout: 60000, message: "big note loaded" });
  await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: 0, insert: "MARK\\n" }, userEvent: 'input.type' }); })()`);
  key("KEYCODE_BACK");
  await eventually(() => !appPid(), { timeout: 15000, message: "process exited after Back" }).catch(() => {});
  const size = fileSize(runAs, `${VAULT}/BigApp.md`);
  console.log(`app storage after Back: ${size} bytes (before ${big.length}, with the edit ${big.length + 5}); edit ${size === big.length + 5 ? "saved" : "LOST"}`);
  assert.ok(size === big.length || size === big.length + 5, `the note must be either the old or the new version, got ${size} bytes`);
  runAs(`rm -f ${VAULT}/BigApp.md`);
});

// A SIGKILL between StdFs::write's temp-file write and its rename leaves the
// temp file behind. Sweeping kills across a 16 MB save can produce one
// (".TmpNote.md.cairn-tmp-<pid>", 16 MB, still there after a restart and a
// rescan), but hitting the window takes luck, so this test plants the same leftover a kill produces (the name uses
// the dead process's pid) and checks whether Cairn ever cleans it up.
test("app storage: a kill during a save leaves no stray copy of the note behind", async () => {
  await note("Stray.md", "stray base\n");
  const deadPid = kill9(); // the process dies; pretend it was in the middle of saving Stray.md
  await eventually(() => !appPid(), { timeout: 10000, message: "process killed" });
  const stray = `.Stray.md.cairn-tmp-${deadPid}`;
  writeAppFile(`${VAULT}/${stray}`, "stray base\nhalf-saved private text\n");
  await d.launch();
  if (await d.isWelcome()) await d.openRecent("Life");
  await rescan(d);
  await d.openNote("Stray.md", { contains: "stray base" });
  await d.append("saved after the restart\n");
  await eventually(() => disk("Stray.md") === "stray base\nsaved after the restart\n", { message: "edit saved" });
  key("KEYCODE_HOME");
  await sleep(800);
  await d.launch();
  await rescan(d);
  const left = runAs(`ls -a ${VAULT} | grep cairn-tmp || true`).trim().split("\n").filter(Boolean);
  console.log("temp files left in the vault after a restart, a rescan and a save of the same note:", JSON.stringify(left));
  runAs(`rm -f ${VAULT}/.*.cairn-tmp-*`);
  assert.deepEqual(left, [], "stale temp copies of notes are removed");
});

test("after a restart the note that was open last comes back", async () => {
  await note("First.md", "first\n");
  await sleep(16000); // the periodic session save runs every 15 s
  writeAppFile(`${VAULT}/Second.md`, "second\n");
  await rescan(d);
  await d.openNote("Second.md", { contains: "second" });
  await sleep(2000);
  const saved = await d.eval(`Object.entries(localStorage).filter(([k]) => k.startsWith('cairn.session')).map(([, v]) => JSON.parse(v).active)`);
  key("KEYCODE_HOME");
  await sleep(800);
  kill9(); // the low-memory killer
  await sleep(500);
  await d.launch();
  await sleep(3000);
  const title = await d.eval(`document.querySelector('.mobile-title')?.textContent ?? null`);
  console.log("session.active before the kill:", saved, "title after restart:", title);
  assert.equal(title, "Second");
});

test("a crash of the WebView renderer process does not take the whole app down", async () => {
  await note("Renderer.md", "renderer\n");
  const pid = appPid();
  devSh("logcat -c");
  const res = await cdpRaw("Page.crash");
  await sleep(3000);
  const now = appPid();
  const fatal = devSh(`logcat -d | grep -E "crash wasn't handled|has died" | grep -v sandboxed || true`).trim();
  console.log("Page.crash:", JSON.stringify(res).slice(0, 120), `pid before ${pid}, after ${now || "(gone)"}`);
  console.log(fatal);
  await d.shot("adv-renderer-crash.png");
  if (!now) await d.launch();
  assert.equal(now, pid, "the app process should survive and reload its web view");
  // A new web view loads the page and reopens the vault and the note.
  await d.attach();
  await d.waitFor(`document.querySelector('.mobile-title')?.textContent === 'Renderer'`, 20000);
});

// ---------------------------------------------------------------- Back and configuration changes

test("Back closes an open drawer before leaving the app", async () => {
  await note("Drawer.md", "drawer\n");
  await openDrawer();
  const pid = appPid();
  key("KEYCODE_BACK");
  await sleep(2000);
  const fg = foregroundPackage();
  const alive = appPid();
  console.log("after Back with the drawer open: foreground", fg, "pid", alive || "(process gone)", "was", pid);
  await d.shot("adv-back-drawer.png");
  if (!alive) await d.launch();
  assert.equal(fg, PKG, "Back should close the drawer, not leave the app");
});

test("Back closes the settings sheet, the quick switcher and a dialog before leaving the app", async () => {
  const results = {};
  for (const [name, open] of [
    ["settings sheet", () => d.click("[data-testid=open-settings]")],
    ["quick switcher", () => d.click(".mobile-bar [title='Find note']")],
    ["new note dialog", () => d.click(".mobile-bar [title='New note']")],
  ]) {
    await note("Overlay.md", "overlay\n");
    await open();
    await sleep(800);
    key("KEYCODE_BACK");
    await sleep(2000);
    results[name] = { foreground: foregroundPackage(), alive: !!appPid() };
  }
  console.log(JSON.stringify(results));
  await d.launch();
  for (const [name, r] of Object.entries(results)) assert.ok(r.alive && r.foreground === PKG, `Back with the ${name} open left the app: ${JSON.stringify(r)}`);
});

test("rotating mid-edit keeps the text, the cursor, the open tabs and the process", async () => {
  await note("Rotate.md", "rotate base\n");
  await d.tap(".cm-content");
  await sleep(1200);
  await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length } }); })()`);
  const pid = appPid();
  typeOnDevice("before rotation");
  devSh("settings put system user_rotation 1");
  await sleep(2500);
  typeOnDevice(" after");
  await sleep(1000);
  devSh("settings put system user_rotation 0");
  await sleep(2500);
  assert.equal(appPid(), pid, "no restart");
  assert.equal(await d.doc(), "rotate base\nbefore rotation after");
  assert.equal(await d.eval(`document.querySelector('.cm-editor').__cairnView.state.selection.main.head`), "rotate base\nbefore rotation after".length);
  await eventually(() => disk("Rotate.md") === "rotate base\nbefore rotation after", { message: "saved" });
  key("KEYCODE_ESCAPE");
});

test("changing the font size (activity recreated) keeps an app-storage vault open and the last edit", async () => {
  await note("FontA.md", "font base\n");
  await d.append("edit before font change");
  devSh("settings put system font_scale 1.3");
  try {
    await sleep(4000);
    await d.attach();
    await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 20000);
    assert.match(disk("FontA.md"), /edit before font change/);
  } finally {
    devSh("settings put system font_scale 1.0");
    await sleep(3000);
    await d.attach().catch(() => {});
  }
});

test("app storage: deleting two notes with the same name keeps both in .trash", async () => {
  await note("Seed.md", "seed\n");
  for (let i = 0; i < 2; i++) {
    await newNote(d, "Twice");
    await d.append(`twice ${i}\n`);
    await eventually(() => disk("Twice.md").includes(`twice ${i}`), { message: "saved" });
    await deleteEntry(d, "Twice.md");
    await eventually(() => !runAs(`ls ${VAULT}`).includes("Twice.md"), { message: `delete ${i}` });
  }
  const trash = runAs(`ls ${VAULT}/.trash`).trim().split("\n").filter((f) => f.startsWith("Twice"));
  assert.equal(trash.length, 2, trash.join(", "));
});

// ---------------------------------------------------------------- keyboard and layout

test("landscape with the keyboard up still shows the line being typed", async () => {
  await note("Land.md", "landscape line\n");
  await d.tap(".cm-content");
  await sleep(1200);
  devSh("settings put system user_rotation 1");
  try {
    await sleep(3000);
    const r = await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; const e = v.dom.getBoundingClientRect(); const c = v.coordsAtPos(v.state.selection.main.head); const tb = document.querySelector('.toolbar[role=toolbar]')?.getBoundingClientRect(); return { editorH: Math.round(e.height), caretTop: c && Math.round(c.top), caretBottom: c && Math.round(c.bottom), toolbarTop: tb ? Math.round(tb.top) : null, viewportH: innerHeight, keyboard: document.activeElement === v.contentDOM }; })()`);
    console.log("landscape + keyboard:", JSON.stringify(r));
    await d.shot("adv-landscape-keyboard.png");
    assert.ok(r.keyboard, "editor focused");
    assert.ok(r.editorH >= 40, `editor area should be at least two lines tall, is ${r.editorH}px`);
    assert.ok(r.caretBottom <= r.viewportH, `caret (bottom ${r.caretBottom}px) should be inside the ${r.viewportH}px viewport`);
    assert.ok(r.toolbarTop !== null && r.caretBottom <= r.toolbarTop, `caret (bottom ${r.caretBottom}px) should be above the formatting toolbar (top ${r.toolbarTop}px)`);
  } finally {
    devSh("settings put system user_rotation 0");
    key("KEYCODE_ESCAPE");
    await sleep(2000);
  }
});

test("landscape with the keyboard up keeps the cursor line in view when it was low on the screen", async () => {
  let body = "";
  for (let i = 1; i <= 40; i++) body += `Line ${i} of a long note\n`;
  await note("LandLong.md", body);
  await d.tap(".cm-content .cm-line:nth-child(12)");
  await sleep(1200);
  const line = await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; return v.state.doc.lineAt(v.state.selection.main.head).number; })()`);
  devSh("settings put system user_rotation 1");
  try {
    await sleep(3000);
    const r = await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; const e = v.scrollDOM.getBoundingClientRect(); const c = v.coordsAtPos(v.state.selection.main.head); return { editorTop: Math.round(e.top), editorBottom: Math.round(e.bottom), caretTop: c && Math.round(c.top), caretBottom: c && Math.round(c.bottom), focused: v.hasFocus }; })()`);
    console.log("cursor on line", line, JSON.stringify(r));
    await d.shot("adv-landscape-keyboard-long.png");
    assert.ok(line > 1, `the tap put the cursor on line ${line}`);
    assert.ok(r.focused, "editor focused");
    assert.ok(r.caretTop >= r.editorTop && r.caretBottom <= r.editorBottom, `cursor line (${r.caretTop}-${r.caretBottom}px) should be inside the editor (${r.editorTop}-${r.editorBottom}px)`);
  } finally {
    devSh("settings put system user_rotation 0");
    key("KEYCODE_ESCAPE");
    await sleep(2000);
  }
});

test("portrait: typing at the end of a long note keeps the caret above the toolbar and keyboard", async () => {
  let body = "# Long\n";
  for (let i = 1; i <= 200; i++) body += `Line ${i} of a long note\n`;
  await note("LongP.md", body);
  await d.tap(".cm-content");
  await sleep(1200);
  await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length }, scrollIntoView: true }); })()`);
  await sleep(500);
  typeOnDevice("typed at the very end");
  key("KEYCODE_ENTER");
  typeOnDevice("and one more");
  await sleep(1200);
  const r = await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; const c = v.coordsAtPos(v.state.selection.main.head); const tb = document.querySelector('.toolbar[role=toolbar]').getBoundingClientRect(); return { caretBottom: c.bottom, toolbarTop: tb.top, viewportH: innerHeight }; })()`);
  console.log(JSON.stringify(r));
  await d.shot("adv-portrait-keyboard.png");
  assert.ok(r.caretBottom <= r.toolbarTop, `caret bottom ${r.caretBottom} must be above the toolbar top ${r.toolbarTop}`);
  key("KEYCODE_ESCAPE");
});

test("system font size 200%: the phone layout still fits the screen and the main controls stay usable", async () => {
  await note("Huge.md", "# Huge text\nsome body\n");
  devSh("settings put system font_scale 2.0");
  try {
    await sleep(4000);
    await d.attach();
    await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 20000);
    await sleep(1500);
    const r = await d.eval(`(() => {
      const se = document.scrollingElement;
      const bar = [...document.querySelectorAll('.mobile-bar button')].map(b => { const r = b.getBoundingClientRect(); return { t: b.title, w: Math.round(r.width), right: Math.round(r.right) }; });
      return { pageOverflowX: se.scrollWidth > innerWidth + 1, innerWidth, bar, statusVisible: (() => { const s = document.querySelector('[data-testid=save-state]'); if (!s) return null; const r = s.getBoundingClientRect(); return r.right <= innerWidth && r.width > 0; })() };
    })()`);
    console.log("font scale 2.0:", JSON.stringify(r));
    await d.shot("adv-fontscale2-editor.png");
    await d.click("[data-testid=open-settings]");
    await sleep(800);
    await d.shot("adv-fontscale2-settings.png");
    await d.eval(`document.querySelector('[data-testid=settings] .close')?.click()`);
    assert.equal(r.pageOverflowX, false, "no horizontal page scrolling");
    for (const b of r.bar) assert.ok(b.w >= 36 && b.right <= r.innerWidth, `mobile bar button ${b.t} visible and tappable: ${JSON.stringify(b)}`);
  } finally {
    devSh("settings put system font_scale 1.0");
    await sleep(3000);
    await d.attach().catch(() => {});
  }
});

// ---------------------------------------------------------------- formatting toolbar

async function toolbar(text, from, to, title) {
  await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: ${JSON.stringify(text)} }, selection: { anchor: ${from}, head: ${to} } }); v.focus(); })()`);
  await sleep(100);
  await d.eval(`document.querySelector('.toolbar[role=toolbar] button[title=${JSON.stringify(title)}]').click()`);
  await sleep(150);
  return d.doc();
}

test("toolbar: undo, redo, bold, code, link, checkbox, heading and indent do what they say", async () => {
  await note("Tools.md", "tools\n");
  assert.equal(await toolbar("make word bold", 5, 9, "Bold"), "make **word** bold");
  assert.equal(await d.eval(`(document.querySelector('.toolbar[role=toolbar] button[title=Undo]').click(), document.querySelector('.cm-editor').__cairnView.state.doc.toString())`), "make word bold");
  assert.equal(await d.eval(`(document.querySelector('.toolbar[role=toolbar] button[title=Redo]').click(), document.querySelector('.cm-editor').__cairnView.state.doc.toString())`), "make **word** bold");
  assert.equal(await toolbar("**word**", 2, 6, "Bold"), "word");
  assert.equal(await toolbar("*word*", 1, 5, "Bold"), "***word***");
  assert.equal(await toolbar("x = 1", 0, 5, "Code"), "`x = 1`");
  assert.equal(await toolbar("Other note", 0, 10, "Link"), "[[Other note]]");
  assert.equal(await toolbar("buy milk", 3, 3, "Checkbox"), "- [ ] buy milk");
  assert.equal(await toolbar("- [ ] buy milk", 3, 3, "Checkbox"), "- [x] buy milk");
  assert.equal(await toolbar("plain", 5, 5, "Heading"), "# plain");
  assert.equal(await toolbar("# plain", 7, 7, "Heading"), "## plain");
  assert.equal(await toolbar("- item", 6, 6, "Indent"), "  - item");
  assert.equal(await toolbar("  - item", 8, 8, "Outdent"), "- item");
});

test("toolbar: Italic on a bold word makes it bold italic", async () => {
  await note("Ital.md", "ital\n");
  const got = await toolbar("**word**", 2, 6, "Italic");
  console.log(JSON.stringify("**word**"), "-> Italic ->", JSON.stringify(got));
  assert.equal(got, "***word***");
});

test("toolbar: Heading on an H4 line does not delete the heading", async () => {
  await note("Head.md", "head\n");
  const got = await toolbar("#### Deep", 9, 9, "Heading");
  console.log(JSON.stringify("#### Deep"), "-> Heading ->", JSON.stringify(got));
  assert.match(got, /^#{1,6} Deep$/);
});

// ---------------------------------------------------------------- long-press menu

test("Move to… from the long-press menu (real touch) moves a note into a folder", async () => {
  await note("Mover.md", "moving\n");
  writeAppFile(`${VAULT}/Box/Inside.md`, "inside\n");
  await rescan(d);
  await captureErrors(d);
  await openDrawer();
  await revealRow(d, "Mover.md");
  const r = await d.rect('[data-testid=tree-row][data-path="Mover.md"]');
  const top = webViewTop();
  const x = Math.round((r.x + 40) * r.dpr);
  const y = Math.round(r.cy * r.dpr + top);
  adb("shell", "input", "swipe", String(x), String(y), String(x), String(y), "900"); // long press
  await d.waitFor(`[...document.querySelectorAll('[role=menuitem]')].some(b => b.textContent.trim() === 'Move to…')`);
  await d.eval(`[...document.querySelectorAll('[role=menuitem]')].find(b => b.textContent.trim() === 'Move to…').setAttribute('data-adv', 'move')`);
  await d.tap("[data-adv=move]");
  await d.waitFor(`[...document.querySelectorAll('.choice')].some(b => b.textContent.trim() === 'Box')`);
  await sleep(500);
  await d.eval(`[...document.querySelectorAll('.choice')].find(b => b.textContent.trim() === 'Box').setAttribute('data-adv', 'box')`);
  await d.shot("adv-move-to.png");
  await d.tap("[data-adv=box]");
  await sleep(2500);
  const errs = await pageErrors(d);
  const root = runAs(`ls ${VAULT}`).trim().split("\n");
  const box = runAs(`ls ${VAULT}/Box`).trim().split("\n");
  console.log("page errors:", errs, "root:", root, "Box:", box);
  assert.ok(box.includes("Mover.md") && !root.includes("Mover.md"), "Mover.md moved into Box on disk");
  assert.deepEqual(errs, [], "no script errors");
});

// ---------------------------------------------------------------- sync setup on a phone

test("sync setup: the vault name and server fields do not auto-capitalize or auto-correct", async () => {
  await note("Ime.md", "ime\n");
  if ((await d.eval(`window.__TAURI_INTERNALS__.invoke('sync_status')`)).configured) await d.eval(`window.__TAURI_INTERNALS__.invoke('sync_disconnect')`);
  await d.click("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=settings-sync]')`);
  await d.click("[data-testid=settings-sync]");
  await d.waitFor(`!!document.querySelector('[data-testid=sync-vault]')`);
  const got = {};
  for (const f of ["sync-server", "sync-vault"]) {
    await d.eval(`(() => { const i = document.querySelector('[data-testid=${f}]'); i.value = ''; i.dispatchEvent(new Event('input', { bubbles: true })); i.scrollIntoView({ block: 'center' }); })()`);
    await sleep(400);
    await d.tap(`[data-testid=${f}]`);
    await sleep(1500);
    // The editor info the web view handed to the keyboard for the focused field.
    const im = devSh("dumpsys input_method");
    const cur = im.slice(im.indexOf("mCurrentEditorInfo:"));
    const t = im.includes("mCurrentEditorInfo:") ? parseInt(cur.match(/inputType=0x([0-9a-f]+)/)?.[1] ?? "x", 16) : NaN;
    got[f] = { inputType: Number.isNaN(t) ? null : "0x" + t.toString(16), capSentences: !!(t & 0x4000), autoCorrect: !!(t & 0x8000), attrs: await d.eval(`(() => { const i = document.querySelector('[data-testid=${f}]'); return { type: i.type, autocapitalize: i.getAttribute('autocapitalize'), autocorrect: i.getAttribute('autocorrect'), spellcheck: i.getAttribute('spellcheck') }; })()`) };
    await d.shot(`adv-ime-${f}.png`);
  }
  key("KEYCODE_ESCAPE");
  await d.eval(`document.querySelector('[data-testid=settings] .close')?.click()`);
  console.log(JSON.stringify(got));
  for (const [f, g] of Object.entries(got)) {
    assert.ok(g.inputType, `keyboard state for ${f} read from dumpsys input_method`);
    assert.ok(!g.capSentences && !g.autoCorrect, `${f} must not request sentence capitalization or autocorrect: ${JSON.stringify(g)}`);
  }
});

// ---------------------------------------------------------------- dark mode

test("switching the system to dark mode while the app is open darkens the system bar areas too", async () => {
  await note("Dark.md", "dark\n");
  key("KEYCODE_ESCAPE");
  devSh("cmd uimode night yes");
  try {
    await sleep(3000);
    await d.attach();
    assert.equal(await d.eval(`matchMedia('(prefers-color-scheme: dark)').matches`), true, "page follows the system theme");
    await d.shot("adv-dark-live.png");
    const p = screenPixels();
    const statusBar = luma(p.px(Math.floor(p.w / 2), 20));
    const page = luma(p.px(Math.floor(p.w / 2), Math.floor(p.h / 2)));
    const navBar = luma(p.px(Math.floor(p.w / 2), p.h - 15));
    console.log("luma: status bar", statusBar, "page", page, "nav bar", navBar);
    assert.ok(page < 80, "page is dark");
    assert.ok(statusBar < 128 && navBar < 128, `system bar areas should be dark too (status ${statusBar}, nav ${navBar})`);
  } finally {
    devSh("cmd uimode night no");
    await sleep(2000);
  }
});
