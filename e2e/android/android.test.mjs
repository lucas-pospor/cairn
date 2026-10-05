// Android end-to-end test on an emulator or device (debug build).
//   . scripts/android-env.sh
//   cd app && npx tauri android build --debug --apk --target x86_64
//   cargo build -p cairn-server -p cairn-sync --example sync_dir
//   node --test e2e/android/
// Requires one device in `adb devices`. The test clears the app's data.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { adb, connect, tapElement, screenshot, webViewTop } from "./cdp.mjs";

const ROOT = path.resolve(import.meta.dirname, "../..");
const APK = path.join(ROOT, "app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk");
const SERVER = path.join(ROOT, "target/debug/cairn-server");
const SYNC_DIR = path.join(ROOT, "target/debug/examples/sync_dir");
const SHOTS = path.join(ROOT, "e2e/.tmp");
const PKG = "app.cairn.notes";
const TOKEN = "android-e2e-token-0123456789";
const PASS = "android e2e passphrase";
const PORT = 19000 + Math.floor(Math.random() * 500);

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-android-e2e-"));
const desktopVault = path.join(tmp, "desktop");
const desktopState = path.join(tmp, "desktop-state");
let server;
let page;

const q = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;
/** Run a shell command on the device as the app (sees its private files). */
const runAs = (cmd) => execFileSync("adb", ["shell", `run-as ${PKG} sh -c ${q(cmd)}`], { encoding: "utf8" });
/** Run a shell command on the device. */
const devSh = (cmd) => execFileSync("adb", ["shell", cmd], { encoding: "utf8" });

function syncDesktop() {
  return JSON.parse(
    execFileSync(SYNC_DIR, [desktopVault, desktopState, `http://127.0.0.1:${PORT}`, TOKEN, "android-e2e", "desktop", PASS], { encoding: "utf8" }),
  );
}

async function launch() {
  adb("shell", "am", "start", "-n", `${PKG}/.MainActivity`);
  page?.close();
  page = await connect();
  await page.waitFor("document.readyState === 'complete' && !!document.querySelector('#app *')", 20000);
}

async function setValue(selector, value) {
  await page.eval(`(() => { const el = document.querySelector(${JSON.stringify(selector)}); el.value = ${JSON.stringify(value)}; el.dispatchEvent(new Event('input', { bubbles: true })); })()`);
}

/** Type with the device keyboard (adb input text needs %s for spaces). */
function typeOnDevice(text) {
  adb("shell", "input", "text", text.replace(/ /g, "%s").replace(/(['"()&;<>|*\\#[\]!$`~?{}])/g, "\\$1"));
}

async function eventually(fn, { timeout = 15000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e.message;
    }
    await sleep(250);
  }
  throw new Error(`timed out: ${message} (last: ${last})`);
}

before(async () => {
  fs.mkdirSync(SHOTS, { recursive: true });
  adb("install", "-r", APK);
  adb("shell", "pm", "clear", PKG);
  server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: TOKEN, CAIRN_DATA: path.join(tmp, "server"), CAIRN_ADDR: `0.0.0.0:${PORT}` },
    stdio: "ignore",
  });
  await launch();
});

after(async () => {
  await screenshot(path.join(SHOTS, "android-final.png")).catch(() => {});
  page?.close();
  server?.kill();
  fs.rmSync(tmp, { recursive: true, force: true });
  adb("forward", "--remove-all");
  // Open sockets to the device can keep Node alive; results are reported by now.
  setTimeout(() => process.exit(), 1500).unref();
});

let vaultDir = "";

test("create a vault on the device with touch", async () => {
  await page.waitFor(`!!document.querySelector('[data-testid=create-on-device]')`);
  await tapElement(page, "[data-testid=create-on-device]");
  await page.waitFor(`document.activeElement?.dataset.testid === 'dialog-input'`);
  await page.eval(`document.querySelector('[data-testid=dialog-input]').select()`);
  typeOnDevice("Phone notes");
  adb("shell", "input", "keyevent", "KEYCODE_ENTER");
  await page.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`);
  vaultDir = await eventually(() => runAs("ls -d /data/user/0/app.cairn.notes/vaults/*").trim(), {
    message: "vault folder created",
  });
  assert.match(vaultDir, /Phone notes$/);
  await screenshot(path.join(SHOTS, "android-empty-vault.png"));
});

test("new note, rename and typing with the device keyboard", async () => {
  await tapElement(page, ".mobile-bar [title='New note']");
  await page.waitFor(`!!document.querySelector('[data-testid=dialog-input]')`);
  if (!(await page.eval(`document.activeElement?.dataset.testid === 'dialog-input'`))) await tapElement(page, "[data-testid=dialog-input]");
  await page.waitFor(`document.activeElement?.dataset.testid === 'dialog-input'`);
  await page.eval(`document.querySelector('[data-testid=dialog-input]').select()`);
  typeOnDevice("Groceries");
  adb("shell", "input", "keyevent", "KEYCODE_ENTER");
  await eventually(() => runAs(`ls ${q(vaultDir)}`).includes("Groceries.md"), { message: "created on disk" });
  await page.waitFor(`document.activeElement?.classList.contains('cm-content')`, 5000).catch(() => tapElement(page, ".cm-content"));
  typeOnDevice("# Shopping");
  adb("shell", "input", "keyevent", "KEYCODE_ENTER");
  typeOnDevice("- [ ] apples");
  adb("shell", "input", "keyevent", "KEYCODE_ENTER");
  typeOnDevice("see [[Recipes]] too");
  await eventually(() => runAs(`cat ${q(vaultDir + "/Groceries.md")}`).includes("see [[Recipes]] too"), { message: "autosaved on device" });
  const text = runAs(`cat ${q(vaultDir + "/Groceries.md")}`);
  assert.match(text, /# Shopping\n- \[ \] apples/);
});

test("live preview renders on the phone", async () => {
  // The checkbox line is not the cursor line, so it renders as a widget.
  await page.waitFor(`!!document.querySelector('.cm-lp-task')`);
  await page.waitFor(`!document.querySelector('.cm-content').textContent.includes('- [ ] apples')`);
  await screenshot(path.join(SHOTS, "android-live-preview.png"));
});

test("files drawer opens and long-press shows the move menu", async () => {
  await tapElement(page, "[data-testid=mobile-files]");
  await page.waitFor(`!!document.querySelector('[data-testid=tree-row][data-path="Groceries.md"]')`);
  await screenshot(path.join(SHOTS, "android-drawer.png"));
  const r = await page.eval(`(() => { const b = document.querySelector('[data-testid=tree-row][data-path="Groceries.md"]').getBoundingClientRect(); return { x: b.left + 40, y: b.top + b.height / 2, dpr: devicePixelRatio }; })()`);
  const top = webViewTop();
  const x = Math.round(r.x * r.dpr);
  const y = Math.round(r.y * r.dpr + top);
  adb("shell", "input", "swipe", String(x), String(y), String(x), String(y), "800");
  await page.waitFor(`[...document.querySelectorAll('[role=menuitem]')].some(b => b.textContent === 'Move to…')`);
  await screenshot(path.join(SHOTS, "android-longpress.png"));
  await page.eval(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))`);
});

test("files added while the app is in the background show up on resume", async () => {
  adb("shell", "input", "keyevent", "KEYCODE_HOME");
  await sleep(800);
  runAs(`printf 'made elsewhere' > ${q(vaultDir + "/Outside.md")}`);
  adb("shell", "am", "start", "-n", `${PKG}/.MainActivity`);
  await page.waitFor(`!!document.querySelector('[data-testid=tree-row][data-path="Outside.md"]')`, 15000);
});

test("sync with a desktop device through the server", async () => {
  await page.eval(`document.querySelector('[data-testid=open-settings]')?.click()`);
  await page.waitFor(`!!document.querySelector('[data-testid=settings-sync]')`);
  await page.eval(`document.querySelector('[data-testid=settings-sync]').click()`);
  await page.waitFor(`!!document.querySelector('[data-testid=sync-server]')`);
  // 10.0.2.2 is the host machine as seen from the emulator.
  await setValue("[data-testid=sync-server]", `http://10.0.2.2:${PORT}`);
  await setValue("[data-testid=sync-token]", TOKEN);
  await setValue("[data-testid=sync-vault]", "android-e2e");
  await setValue("[data-testid=sync-device]", "pixel");
  await setValue("[data-testid=sync-pass]", PASS);
  await setValue("[data-testid=sync-pass2]", PASS);
  await screenshot(path.join(SHOTS, "android-sync-form.png"));
  await page.eval(`document.querySelector('[data-testid=sync-connect]').click()`);
  // A new vault: the "Create it?" question is answered yes.
  await page
    .waitFor(`(document.querySelector('[data-testid=dialog-ok]')?.click(), document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle')`, 60000)
    .catch(async (e) => {
      throw new Error(e.message + " " + (await page.eval(`document.querySelector('[data-testid=sync-error]')?.textContent`)));
    });
  await screenshot(path.join(SHOTS, "android-sync-connected.png"));
  const r = syncDesktop();
  assert.ok(r.pulled >= 2, JSON.stringify(r));
  assert.match(fs.readFileSync(path.join(desktopVault, "Groceries.md"), "utf8"), /apples/);
  // and back: a note written on the desktop arrives on the phone
  fs.writeFileSync(path.join(desktopVault, "Recipes.md"), "# Recipes\napple pie\n");
  syncDesktop();
  await page.eval(`document.querySelector('[data-testid=sync-now]').click()`);
  await eventually(() => runAs(`ls ${q(vaultDir)}`).includes("Recipes.md"), { message: "desktop note on the phone" });
  await page.eval(`document.querySelector('[data-testid=settings] .close').click()`);
});

test("open a folder from shared storage through the system picker", async () => {
  devSh("rm -rf /sdcard/Documents/SafVault; mkdir -p /sdcard/Documents/SafVault/Sub");
  devSh(`printf ${q("# From SAF\nhello [[Other]]")} > /sdcard/Documents/SafVault/Start.md`);
  devSh(`printf ${q("other note")} > /sdcard/Documents/SafVault/Sub/Other.md`);
  // Back to the welcome screen and open the picker.
  await page.eval(`document.querySelector('.status .vault').click()`);
  await page.waitFor(`[...document.querySelectorAll('button')].some(b => b.textContent.includes('Open a folder from storage'))`);
  await page.eval(`[...document.querySelectorAll('button')].find(b => b.textContent.includes('Open a folder from storage')).click()`);
  // Drive the system DocumentsUI picker.
  const clickText = async (re, timeout = 15000) => {
    const end = Date.now() + timeout;
    while (Date.now() < end) {
      adb("shell", "uiautomator", "dump", "/sdcard/ui.xml");
      const xml = adb("shell", "cat", "/sdcard/ui.xml");
      const nodes = [...xml.matchAll(/<node [^>]*?text="([^"]*)"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g)];
      const hit = nodes.find((n) => re.test(n[1]));
      if (hit) {
        const [x1, y1, x2, y2] = hit.slice(2).map(Number);
        adb("shell", "input", "tap", String((x1 + x2) >> 1), String((y1 + y2) >> 1));
        return hit[1];
      }
      await sleep(700);
    }
    throw new Error(`picker: no element matching ${re}`);
  };
  await sleep(2000);
  await screenshot(path.join(SHOTS, "android-picker.png"));
  // The picker may open in Recent; navigate: Documents -> SafVault -> Use this folder -> Allow
  try {
    await clickText(/^SafVault$/, 4000);
  } catch {
    await clickText(/^Documents$/);
    await sleep(800);
    await clickText(/^SafVault$/);
  }
  await sleep(800);
  await clickText(/USE THIS FOLDER|Use this folder/i);
  await sleep(800);
  await clickText(/^ALLOW$|^Allow$/i);
  page.close();
  page = await connect();
  await page.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 20000);
  await page.eval(`document.querySelector('[data-testid=mobile-files]').click()`);
  await page.waitFor(`!!document.querySelector('[data-testid=tree-row][data-path="Start.md"]')`, 15000);
  await page.waitFor(`!!document.querySelector('[data-testid=tree-row][data-path="Sub"]')`);
  await sleep(1500);
  await screenshot(path.join(SHOTS, "android-saf-tree.png"));
  // Open, edit, and check the file on shared storage.
  await page.eval(`document.querySelector('[data-testid=tree-row][data-path="Start.md"]').click()`);
  await page.waitFor(`document.querySelector('.cm-editor')?.__cairnView?.state.doc.toString().includes('From SAF')`);
  await page.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: v.state.doc.length, insert: '\\nedited on the phone' } }); })()`);
  await eventually(() => devSh("cat /sdcard/Documents/SafVault/Start.md").includes("edited on the phone"), { message: "SAF write" });
  // Backlinks work across the SAF tree.
  await page.eval(`document.querySelector('[data-testid=mobile-files]').click()`);
  await page.eval(`document.querySelector('[data-testid=tree-row][data-path="Sub"]').click()`);
  await page.waitFor(`!!document.querySelector('[data-testid=tree-row][data-path="Sub/Other.md"]')`);
  // Create a note in the SAF folder.
  await page.eval(`document.querySelector('.mobile-bar [title="New note"]').click()`);
  await page.waitFor(`!!document.querySelector('[data-testid=dialog-input]')`);
  await setValue("[data-testid=dialog-input]", "Made on phone");
  await page.eval(`document.querySelector('[data-testid=dialog-ok]').click()`);
  await eventually(() => devSh("ls /sdcard/Documents/SafVault/").includes("Made on phone.md"), { message: "SAF create" });
});
