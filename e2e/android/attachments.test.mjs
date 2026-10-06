// Pasting or dropping a file into a note on Android saves it as an
// attachment and links it, in a notebook in the app's storage and in a shared
// folder (Storage Access Framework, written through SafFs). Tauri cannot send
// raw bytes to a command on Android, so the bytes travel as base64: the test
// checks them byte for byte, with every byte value and a name that needs
// percent-encoding.
//
//   . scripts/android-env.sh
//   node --test --test-concurrency=1 e2e/android/attachments.test.mjs
//   or: scripts/adv-android-run-all.sh e2e/android/attachments.test.mjs
//
// Needs one emulator/device in `adb devices` (the debug APK is installed if
// missing). Clears the app's data and uses /sdcard/Documents/Attach.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import {
  Device, adb, devSh, runAs, q, eventually, APK, PKG, ROOT, rescan, writeAppFile, fillSharedFolder,
  readSettings, restoreSettings, captureErrors, pageErrors,
} from "./adv_helpers.mjs";

const LABEL = "Attach";
const F = `/sdcard/Documents/${LABEL}`;
const NOTE = "# Note\n\nFiles:\n";
const PNG = fs.readFileSync(path.join(ROOT, "app/src-tauri/icons/32x32.png"));
// 300 KB with every byte value, so nothing is lost or changed on the way.
const BIN = Buffer.alloc(300 * 1024).map((_, i) => (i * 31 + (i >> 10)) & 255);
// "#" is not allowed in a link and becomes "-"; "é" and the space are percent-encoded in the header.
const BIN_NAME = "Résumé #1.pdf";
const BIN_SAVED = "Résumé -1.pdf";
const sha = (b) => crypto.createHash("sha256").update(b).digest("hex");
const d = new Device();
let settings;

before(async () => {
  settings = readSettings();
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  await d.fresh();
});

after(async () => {
  restoreSettings(settings);
  await d.shot("attachments-final.png").catch(() => {});
  d.close();
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

/** Put the cursor at the end of the open note. */
function focusEnd() {
  return d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } }); })()`);
}

/** Page code that builds a DataTransfer holding one file. */
const transfer = (name, type, bytes) => `(() => {
  const bytes = Uint8Array.from(atob(${JSON.stringify(Buffer.from(bytes).toString("base64"))}), (c) => c.charCodeAt(0));
  const dt = new DataTransfer();
  dt.items.add(new File([bytes], ${JSON.stringify(name)}, { type: ${JSON.stringify(type)} }));
  return dt;
})()`;

/** Paste a file into the editor, as the clipboard does; true if the editor took it. */
function paste(name, type, bytes) {
  return d.eval(`(() => {
    const e = new ClipboardEvent('paste', { clipboardData: ${transfer(name, type, bytes)}, bubbles: true, cancelable: true });
    document.querySelector('.cm-content').dispatchEvent(e);
    return e.defaultPrevented;
  })()`);
}

/** Drop a file at the end of the editor's text; true if the editor took it. */
function drop(name, type, bytes) {
  return d.eval(`(() => {
    const target = document.querySelector('.cm-content');
    const r = target.getBoundingClientRect();
    const e = new DragEvent('drop', { dataTransfer: ${transfer(name, type, bytes)}, bubbles: true, cancelable: true, clientX: r.left + 5, clientY: r.bottom - 5 });
    target.dispatchEvent(e);
    return e.defaultPrevented;
  })()`);
}

/** Record every toast's text as it shows (an error toast goes away after 7 s). */
function recordToasts() {
  return d.eval(`(() => {
    window.__toastObserver?.disconnect();
    const log = (window.__toastLog = []);
    window.__toastObserver = new MutationObserver(() => {
      for (const t of document.querySelectorAll('.toast')) {
        const s = t.textContent.trim();
        if (s && !log.includes(s)) log.push(s);
      }
    });
    window.__toastObserver.observe(document.body, { childList: true, subtree: true, characterData: true });
  })()`);
}
const toastLog = () => d.eval(`window.__toastLog ?? []`);

/** Wait for `cond` and return its value; a "Could not save" toast fails at once, with its text. */
async function waitSaved(cond, message) {
  let value, error;
  try {
    await eventually(
      async () => {
        error = (await toastLog()).find((t) => /Could not save/.test(t));
        return error || (value = cond());
      },
      { message, timeout: 20000 },
    );
  } catch (e) {
    e.message += ` toasts=${JSON.stringify(await toastLog())}`;
    throw e;
  }
  if (error) assert.fail(`${message}: ${error}`);
  return value;
}

/**
 * Paste and drop into Note.md. `sh` runs a shell command that can read the
 * vault's files (runAs or devSh); `root` is the vault's folder.
 */
async function checkVault(name, sh, root) {
  const list = () => sh(`ls ${q(`${root}/attachments`)} 2>/dev/null || true`).split("\n").map((s) => s.trim()).filter(Boolean);
  const hash = (f) => sh(`sha256sum ${q(`${root}/attachments/${f}`)} 2>/dev/null || true`).split(" ")[0];
  const note = () => sh(`cat ${q(`${root}/Note.md`)}`);

  await captureErrors(d);
  await recordToasts();
  await d.openNote("Note.md", { contains: "Files:" });

  // A pasted image named image.png (what the clipboard gives) gets a dated name.
  await focusEnd();
  assert.equal(await paste("image.png", "image/png", PNG), true, "the editor took the paste");
  const pasted = await waitSaved(() => list().find((f) => /^Pasted image \d{14}\.png$/.test(f)), "pasted image saved");
  assert.equal(hash(pasted), sha(PNG), "pasted bytes");
  await waitSaved(() => note().includes(`![[${pasted}]]`), "pasted image linked in the note");

  // A dropped file keeps its name; a second drop of the same name gets a free one.
  await focusEnd();
  assert.equal(await drop(BIN_NAME, "application/pdf", BIN), true, "the editor took the drop");
  await waitSaved(() => list().includes(BIN_SAVED), `${BIN_SAVED} saved`);
  assert.equal(hash(BIN_SAVED), sha(BIN), "dropped bytes");
  await waitSaved(() => note().includes(`![[${BIN_SAVED}]]`), "dropped file linked in the note");
  await focusEnd();
  assert.equal(await drop(BIN_NAME, "application/pdf", BIN), true);
  const second = BIN_SAVED.replace(/\.pdf$/, " 1.pdf");
  await waitSaved(() => list().includes(second), `${second} saved`);
  assert.equal(hash(second), sha(BIN));
  await waitSaved(() => note().includes(`![[${second}]]`), "second drop linked in the note");

  assert.deepEqual(list().sort(), [pasted, BIN_SAVED, second].sort());
  assert.deepEqual((await toastLog()).filter((t) => /Could not save/.test(t)), []);
  assert.deepEqual(await pageErrors(d), []);
  await d.shot(`attachments-${name}.png`);
}

test("app storage: a pasted or dropped file is saved as an attachment and linked", async () => {
  const vault = await d.createAppVault(LABEL);
  writeAppFile(`${vault}/Note.md`, NOTE);
  await rescan(d);
  await checkVault("app", runAs, vault);
});

test("shared folder: a pasted or dropped file is saved through the Storage Access Framework and linked", async () => {
  fillSharedFolder(F, { "Note.md": NOTE });
  await d.toWelcome();
  await d.pickSafFolder(LABEL);
  await checkVault("saf", devSh, F);
  devSh(`rm -rf ${F}`);
});
