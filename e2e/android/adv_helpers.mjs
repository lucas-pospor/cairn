// Helpers for the adversarial Android tests (adv_*.test.mjs).
// Built on cdp.mjs (adb + WebView DevTools protocol). Every test file that
// uses these clears the app's data first; nothing outside the emulator and
// temp folders is touched.

import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { adb, connect, screenshot, webViewTop } from "./cdp.mjs";

export { adb, connect, screenshot, webViewTop, sleep };

export const ROOT = path.resolve(import.meta.dirname, "../..");
export const APK = path.join(ROOT, "app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk");
export const SERVER = path.join(ROOT, "target/debug/cairn-server");
export const SYNC_DIR = path.join(ROOT, "target/debug/examples/sync_dir");
export const SHOTS = path.join(ROOT, "e2e/.tmp/AN");
export const PKG = "app.cairn.notes";
export const APP_DATA = `/data/user/0/${PKG}`;

fs.mkdirSync(SHOTS, { recursive: true });

export const q = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;
/** Shell command on the device as the app (sees its private files). */
export const runAs = (cmd) => execFileSync("adb", ["shell", `run-as ${PKG} sh -c ${q(cmd)}`], { encoding: "utf8" });
/** Shell command on the device. */
export const devSh = (cmd) => execFileSync("adb", ["shell", cmd], { encoding: "utf8" });
export const appPid = () => {
  try {
    return devSh(`pidof ${PKG}`).trim();
  } catch {
    return "";
  }
};

export async function eventually(fn, { timeout = 15000, message = "condition", interval = 250 } = {}) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e.message;
    }
    await sleep(interval);
  }
  throw new Error(`timed out: ${message} (last: ${typeof last === "string" ? last.slice(0, 400) : JSON.stringify(last)})`);
}

/** Type with the device keyboard (adb input text needs %s for spaces). */
export function typeOnDevice(text) {
  adb("shell", "input", "text", text.replace(/ /g, "%s").replace(/(['"()&;<>|*\\#[\]!$`~?{}])/g, "\\$1"));
}
export const key = (k) => adb("shell", "input", "keyevent", k);

export class Device {
  page = null;

  async attach(timeout = 20000) {
    this.page?.close();
    this.page = await connect();
    await this.page.waitFor("document.readyState === 'complete' && !!document.querySelector('#app *')", timeout);
    // Start-up reopens the last vault asynchronously: wait for the welcome screen or the workspace.
    await this.page.waitFor("!!document.querySelector('.welcome, .workspace')", timeout);
    return this.page;
  }

  async launch() {
    adb("shell", "am", "start", "-n", `${PKG}/.MainActivity`);
    return this.attach();
  }

  /** Install (if needed), wipe app data and start fresh. */
  async fresh() {
    adb("shell", "am", "force-stop", PKG);
    adb("shell", "pm", "clear", PKG);
    adb("forward", "--remove-all");
    return this.launch();
  }

  eval(expr) {
    return this.page.eval(expr);
  }
  waitFor(expr, timeout) {
    return this.page.waitFor(expr, timeout);
  }

  async setValue(selector, value) {
    await this.eval(
      `(() => { const el = document.querySelector(${JSON.stringify(selector)}); el.value = ${JSON.stringify(value)}; el.dispatchEvent(new Event('input', { bubbles: true })); })()`,
    );
  }

  click(selector) {
    return this.eval(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) throw new Error('no ' + ${JSON.stringify(selector)}); e.click(); return true; })()`);
  }

  clickText(text, sel = "button") {
    return this.eval(
      `(() => { const b = [...document.querySelectorAll(${JSON.stringify(sel)})].find(b => b.textContent.includes(${JSON.stringify(text)})); if (!b) throw new Error('no button ' + ${JSON.stringify(text)}); b.click(); return true; })()`,
    );
  }

  /** Real tap at an element's centre (CSS px -> device px). */
  async tap(selector) {
    const r = await this.rect(selector);
    if (!r) throw new Error(`no element ${selector}`);
    const top = webViewTop();
    adb("shell", "input", "tap", String(Math.round(r.cx * r.dpr)), String(Math.round(r.cy * r.dpr + top)));
  }

  rect(selector) {
    return this.eval(
      `(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) return null; const b = e.getBoundingClientRect(); return { x: b.left, y: b.top, w: b.width, h: b.height, cx: b.left + b.width / 2, cy: b.top + b.height / 2, dpr: devicePixelRatio, ih: innerHeight, iw: innerWidth }; })()`,
    );
  }

  /** Create a vault in app storage through the UI and return its folder. */
  async createAppVault(name) {
    await this.waitFor(`!!document.querySelector('[data-testid=create-on-device]')`);
    await this.click("[data-testid=create-on-device]");
    await this.waitFor(`!!document.querySelector('[data-testid=dialog-input]')`);
    await this.setValue("[data-testid=dialog-input]", name);
    await this.click("[data-testid=dialog-ok]");
    await this.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 20000);
    return `${APP_DATA}/vaults/${name}`;
  }

  /** From the welcome screen, pick a shared folder (under Documents) with the system picker. */
  async pickSafFolder(folderName) {
    await this.waitFor(`[...document.querySelectorAll('button')].some(b => b.textContent.includes('Open a folder from storage'))`);
    await this.clickText("Open a folder from storage");
    await sleep(2000);
    try {
      await uiClick(new RegExp(`^${folderName}$`), 4000);
    } catch {
      try {
        await uiClick(/^Documents$/, 6000);
      } catch {
        // Picker opened somewhere else: open the drawer and go to the device root.
        await uiClickDesc(/Show roots/i, 4000);
        await sleep(800);
        await uiClick(/^sdk_gphone|^Android SDK|^Internal storage|^SDK/i, 4000);
        await sleep(800);
        await uiClick(/^Documents$/);
      }
      await sleep(800);
      await uiClick(new RegExp(`^${folderName}$`));
    }
    await sleep(800);
    await uiClick(/USE THIS FOLDER|Use this folder/i);
    await sleep(800);
    await uiClick(/^ALLOW$|^Allow$/i);
    await this.attach();
    await this.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 60000);
  }

  /** Open a vault from the welcome screen's Recent list (works for SAF URIs). */
  async openRecent(labelPart) {
    await this.waitFor(`[...document.querySelectorAll('.recent-open')].some(b => b.textContent.includes(${JSON.stringify(labelPart)}))`);
    await this.eval(`[...document.querySelectorAll('.recent-open')].find(b => b.textContent.includes(${JSON.stringify(labelPart)})).click()`);
    await this.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 60000);
  }

  async toWelcome() {
    await this.click(".status .vault");
    await this.waitFor(`!!document.querySelector('.welcome')`);
  }

  /** Open a note by clicking its row in the (possibly hidden) file tree. */
  async openNote(p, { contains = null } = {}) {
    const sel = `[data-testid=tree-row][data-path=${JSON.stringify(JSON.stringify(p)).slice(1, -1)}]`;
    await this.waitFor(`!!document.querySelector('${sel}')`, 3000).catch(() => false);
    await revealRow(this, p);
    const ok = await this.eval(`(() => { const r = document.querySelector('${sel}'); if (!r) return false; r.click(); return true; })()`);
    if (!ok) throw new Error(`no tree row for ${p}`);
    await this.waitFor(`document.querySelector('.mobile-title')?.textContent.trim() === ${JSON.stringify(path.basename(p).replace(/\.md$/, ""))} && !!document.querySelector('.cm-editor')?.__cairnView`);
    if (contains != null) await this.waitFor(`document.querySelector('.cm-editor').__cairnView.state.doc.toString().includes(${JSON.stringify(contains)})`, 30000);
    await sleep(500);
  }

  /** Error text shown instead of the editor (e.g. "Not found: …"), or null. */
  tabError() {
    return this.eval(`document.querySelector('.pane .empty .error')?.textContent ?? null`);
  }

  isWelcome() {
    return this.eval(`!!document.querySelector('.welcome')`);
  }

  doc() {
    return this.eval(`document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
  }

  /** Edit through CodeMirror (same path as typing: triggers autosave). */
  append(text) {
    return this.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: v.state.doc.length, insert: ${JSON.stringify(text)} }, userEvent: 'input.type' }); return v.state.doc.length; })()`);
  }

  replaceAll(text) {
    return this.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: ${JSON.stringify(text)} }, userEvent: 'input.type' }); return v.state.doc.length; })()`);
  }

  saveState() {
    return this.eval(`document.querySelector('[data-testid=save-state]')?.textContent.trim() ?? null`);
  }

  toasts() {
    return this.eval(`[...document.querySelectorAll('.toast, [class*=toast]')].map(t => t.textContent.trim()).filter(Boolean)`);
  }

  async shot(name) {
    await screenshot(path.join(SHOTS, name));
  }

  close() {
    this.page?.close();
    this.page = null;
  }
}

export function uiDump() {
  for (let i = 0; i < 3; i++) {
    try {
      adb("shell", "uiautomator", "dump", "/sdcard/ui.xml");
      return adb("shell", "cat", "/sdcard/ui.xml");
    } catch {}
  }
  return "";
}

/** Tap the first UI node whose text matches (system UI such as the folder picker). */
export async function uiClick(re, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const xml = uiDump();
    const nodes = [...xml.matchAll(/<node [^>]*?text="([^"]*)"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g)];
    const hit = nodes.find((n) => re.test(n[1]));
    if (hit) {
      const [x1, y1, x2, y2] = hit.slice(2).map(Number);
      adb("shell", "input", "tap", String((x1 + x2) >> 1), String((y1 + y2) >> 1));
      return hit[1];
    }
    await sleep(700);
  }
  throw new Error(`ui: no element matching ${re}`);
}

export async function uiClickDesc(re, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const xml = uiDump();
    const nodes = [...xml.matchAll(/<node [^>]*?content-desc="([^"]*)"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g)];
    const hit = nodes.find((n) => re.test(n[1]));
    if (hit) {
      const [x1, y1, x2, y2] = hit.slice(2).map(Number);
      adb("shell", "input", "tap", String((x1 + x2) >> 1), String((y1 + y2) >> 1));
      return hit[1];
    }
    await sleep(700);
  }
  throw new Error(`ui: no element with description ${re}`);
}

/** The package of the activity in front ("" if none). */
export function foregroundPackage() {
  const out = devSh("dumpsys activity activities | grep -E 'topResumedActivity|mResumedActivity' | head -1");
  const m = out.match(/u0 ([\w.]+)\//);
  return m ? m[1] : "";
}

/** Start a sync server on the host, reachable from the emulator at 10.0.2.2. */
export function startServer(dataDir, token) {
  const port = 19500 + Math.floor(Math.random() * 400);
  const proc = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: token, CAIRN_DATA: dataDir, CAIRN_ADDR: `0.0.0.0:${port}` },
    stdio: "ignore",
  });
  return { proc, port };
}

export function syncDir(vault, state, port, token, vaultId, device, pass) {
  return JSON.parse(execFileSync(SYNC_DIR, [vault, state, `http://127.0.0.1:${port}`, token, vaultId, device, pass], { encoding: "utf8" }));
}

/** Device settings this suite may change, so they can be put back. */
export function readSettings() {
  return {
    accel: devSh("settings get system accelerometer_rotation").trim(),
    rot: devSh("settings get system user_rotation").trim(),
    font: devSh("settings get system font_scale").trim(),
    night: devSh("cmd uimode night").trim(),
    density: devSh("wm density").trim(),
  };
}

export function restoreSettings(s) {
  try {
    devSh(`settings put system accelerometer_rotation ${s.accel === "null" ? 1 : s.accel}`);
    devSh(`settings put system user_rotation ${s.rot === "null" ? 0 : s.rot}`);
    devSh(`settings put system font_scale ${s.font === "null" ? 1.0 : s.font}`);
    devSh(`cmd uimode night ${/yes/.test(s.night) ? "yes" : "no"}`);
    if (/Override density/.test(s.density)) {
      const m = s.density.match(/Override density: (\d+)/);
      devSh(`wm density ${m[1]}`);
    } else devSh("wm density reset");
  } catch (e) {
    console.error("could not restore device settings", e.message);
  }
}

/** Open a tree row's context menu (what a long press does) and pick an item. */
export async function treeMenu(d, p, label) {
  const sel = `[data-testid=tree-row][data-path=${JSON.stringify(JSON.stringify(p)).slice(1, -1)}]`;
  await d.waitFor(`!!document.querySelector('${sel}')`, 3000).catch(() => false);
  await revealRow(d, p);
  await d.waitFor(`!!document.querySelector('${sel}')`, 20000);
  await d.eval(`(() => { const r = document.querySelector('${sel}'); const b = r.getBoundingClientRect(); r.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: b.left + 20, clientY: b.top + 5 })); })()`);
  await d.waitFor(`[...document.querySelectorAll('[role=menuitem]')].some(b => b.textContent.trim() === ${JSON.stringify(label)})`);
  await d.eval(`[...document.querySelectorAll('[role=menuitem]')].find(b => b.textContent.trim() === ${JSON.stringify(label)}).click()`);
}

/** Delete a tree entry through the menu and the confirmation dialog. */
export async function deleteEntry(d, p) {
  await treeMenu(d, p, "Delete");
  await d.waitFor(`!!document.querySelector('[data-testid=dialog-ok]')`);
  await d.click("[data-testid=dialog-ok]");
}

/** Rename a tree entry inline (as the menu's Rename… does). */
export async function renameEntry(d, p, newName) {
  await treeMenu(d, p, "Rename…");
  await d.waitFor(`!!document.querySelector('[data-testid=rename-input]')`);
  await d.eval(`(() => { const i = document.querySelector('[data-testid=rename-input]'); i.value = ${JSON.stringify(newName)}; i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); })()`);
}

/** Create a note through the mobile bar's New note button. */
export async function newNote(d, name) {
  await d.click(".mobile-bar [title='New note']");
  await d.waitFor(`!!document.querySelector('[data-testid=dialog-input]')`);
  await d.setValue("[data-testid=dialog-input]", name);
  await d.click("[data-testid=dialog-ok]");
  await d.waitFor(`document.querySelector('.mobile-title')?.textContent.trim() === ${JSON.stringify(name)}`, 20000);
  await sleep(700);
}

export function treePaths(d) {
  return d.eval(`[...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path)`);
}

/** Fill Settings > Sync and press Connect (does not wait for the result). */
export async function fillSyncForm(d, { port, token, vaultId, device, pass }) {
  await d.click("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=settings-sync]')`);
  await d.click("[data-testid=settings-sync]");
  await d.waitFor(`!!document.querySelector('[data-testid=sync-server]')`);
  await d.setValue("[data-testid=sync-server]", `http://10.0.2.2:${port}`);
  await d.setValue("[data-testid=sync-token]", token);
  await d.setValue("[data-testid=sync-vault]", vaultId);
  await d.setValue("[data-testid=sync-device]", device);
  await d.setValue("[data-testid=sync-pass]", pass);
  await d.setValue("[data-testid=sync-pass2]", pass);
  await d.click("[data-testid=sync-connect]");
}

/**
 * Connect the open vault to a sync server through Settings > Sync. A vault
 * the server does not have yet is created (the setup question is answered yes).
 */
export async function setupSync(d, opts) {
  await fillSyncForm(d, opts);
  await d.waitFor(`(document.querySelector('[data-testid=dialog-ok]')?.click(), document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle')`, 60000).catch(async (e) => {
    throw new Error(e.message + " " + (await d.eval(`document.querySelector('[data-testid=sync-error]')?.textContent`)));
  });
}

/** Run "Sync now" in the open settings sheet and wait until it is done; returns the state text. */
export async function syncNowInSettings(d) {
  await d.click("[data-testid=sync-now]");
  await sleep(300);
  await d.waitFor(`document.querySelector('[data-testid=sync-state]')?.textContent.trim() !== 'syncing'`, 60000);
  return d.eval(`document.querySelector('[data-testid=sync-state]')?.textContent.trim() + ' | ' + (document.querySelector('[data-testid=settings] .err')?.textContent ?? '')`);
}

export async function closeSettings(d) {
  await d.eval(`document.querySelector('[data-testid=settings] .close')?.click()`);
}

/** Send one raw DevTools command to the app's page (e.g. "Page.crash"). */
export async function cdpRaw(method, params = {}, port = 9223) {
  const pid = appPid();
  if (!pid) throw new Error("app not running");
  adb("forward", `tcp:${port}`, `localabstract:webview_devtools_remote_${pid}`);
  const pages = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).filter((p) => p.type === "page");
  const ws = new WebSocket(pages[0].webSocketDebuggerUrl);
  await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)));
  const res = await new Promise((resolve) => {
    ws.onmessage = (m) => resolve(JSON.parse(m.data));
    ws.onclose = () => resolve({ closed: true });
    ws.send(JSON.stringify({ id: 1, method, params }));
    setTimeout(() => resolve({ timeout: true }), 3000);
  });
  try {
    ws.close();
  } catch {}
  return res;
}

/** Write a file on the device's shared storage (any name or content) through adb push. */
export function pushFile(devicePath, content) {
  const dir = fs.mkdtempSync(path.join(SHOTS, ".push-"));
  const local = path.join(dir, "f");
  fs.writeFileSync(local, content);
  try {
    execFileSync("adb", ["push", local, devicePath], { stdio: "ignore" });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

/** Empty a shared-storage folder (keeping the folder itself, so a granted tree URI stays valid) and fill it. */
export function fillSharedFolder(folder, files) {
  devSh(`mkdir -p ${q(folder)} && cd ${q(folder)} && rm -rf ./* ./.trash ./.cairn`);
  for (const [name, content] of Object.entries(files)) {
    if (name.includes("/")) devSh(`mkdir -p ${q(folder + "/" + path.dirname(name))}`);
    pushFile(`${folder}/${name}`, content);
  }
}

/** Reopen an SAF vault from the welcome screen after its folder was refilled outside the app. */
export async function reopenSaf(d, label, folder, files) {
  if (!(await d.isWelcome())) await d.toWelcome();
  fillSharedFolder(folder, files);
  await d.openRecent(label);
}

export function ls(folder) {
  const out = devSh(`ls -a ${q(folder)} 2>/dev/null || true`).trim();
  return out ? out.split("\n").filter((f) => f !== "." && f !== "..") : [];
}

/** Write a file inside the app's private storage (app-storage vaults). */
export function writeAppFile(file, content) {
  const b64 = Buffer.from(content).toString("base64");
  runAs(`mkdir -p ${q(path.posix.dirname(file))} && echo ${b64} | base64 -d > ${q(file)}`);
}

export function readAppFile(file) {
  return runAs(`cat ${q(file)} 2>/dev/null || true`);
}

/** Ask the app to rescan the vault now (what the 20 s timer and app resume do). */
export async function rescan(d) {
  await d.eval(`window.__TAURI_INTERNALS__.invoke('rescan')`);
  await sleep(400);
}

/** Raw screenshot pixels: { w, h, px(x, y) -> [r, g, b] }. */
export function screenPixels() {
  const raw = execFileSync("adb", ["exec-out", "screencap"], { maxBuffer: 64 << 20 });
  const w = raw.readUInt32LE(0);
  const h = raw.readUInt32LE(4);
  const header = raw.length - w * h * 4 === 16 ? 16 : 12;
  return {
    w,
    h,
    px: (x, y) => {
      const o = header + (y * w + x) * 4;
      return [raw[o], raw[o + 1], raw[o + 2]];
    },
  };
}
export const luma = ([r, g, b]) => Math.round(0.299 * r + 0.587 * g + 0.114 * b);

/** Make a tree row exist: the tree only renders rows in view, so open the drawer and scroll to it. */
export async function revealRow(d, p) {
  const sel = `[data-testid=tree-row][data-path=${JSON.stringify(JSON.stringify(p)).slice(1, -1)}]`;
  for (let i = 0; i < 40; i++) {
    if (await d.eval(`!!document.querySelector('${sel}')`)) return true;
    if (i === 0) {
      if (await d.eval(`document.querySelector('aside.left').classList.contains('hidden')`)) await d.click("[data-testid=mobile-files]");
      await d.eval(`document.querySelector('[data-testid=tab-files]')?.click()`);
      await d.eval(`(() => { const t = document.querySelector('[data-testid=file-tree]'); t.scrollTop = 0; t.dispatchEvent(new Event('scroll')); })()`);
    } else {
      await d.eval(`(() => { const t = document.querySelector('[data-testid=file-tree]'); t.scrollTop += Math.max(200, t.clientHeight - 80); t.dispatchEvent(new Event('scroll')); })()`);
    }
    await sleep(150);
  }
  return false;
}

/** Size in bytes of a file on the device (-1 if missing). `sh` is devSh or runAs. */
export function fileSize(sh, file) {
  const out = sh(`stat -c %s ${q(file)} 2>/dev/null || echo -1`).trim();
  return Number(out);
}

/** Last `n` bytes of a file on the device (safe for big files). */
export function fileTail(sh, file, n = 64) {
  return sh(`tail -c ${n} ${q(file)} 2>/dev/null || true`);
}

/** Collect page errors and unhandled rejections in window.__advErrs. */
export function captureErrors(d) {
  return d.eval(
    `(() => { if (window.__advErrs) { window.__advErrs.length = 0; return; } window.__advErrs = []; window.addEventListener('error', (e) => window.__advErrs.push('error: ' + e.message)); window.addEventListener('unhandledrejection', (e) => window.__advErrs.push('rejection: ' + String(e.reason?.message ?? e.reason))); })()`,
  );
}

export const pageErrors = (d) => d.eval(`window.__advErrs ?? []`);

/** Error/info toasts currently shown. */
export const toastTexts = (d) => d.eval(`[...document.querySelectorAll('.toast')].map(t => t.textContent.trim())`);

/** App running, attached, and showing the app-storage vault `name` (created if needed). */
export async function ensureAppVault(d, name) {
  await d.launch(); // also brings a backgrounded app to the front
  if (!(await d.isWelcome())) {
    const cur = await d.eval(`document.querySelector('.status .vault')?.textContent.trim() ?? ''`);
    if (cur === name) return `${APP_DATA}/vaults/${name}`;
    await d.toWelcome();
  }
  const has = await d.eval(`[...document.querySelectorAll('.recent-open')].some(b => b.textContent.includes(${JSON.stringify(name)}))`);
  if (has) {
    await d.openRecent(name);
    return `${APP_DATA}/vaults/${name}`;
  }
  return d.createAppVault(name);
}

/** App running, attached, showing the SAF folder /sdcard/Documents/<label> (picked if not in Recent). */
export async function ensureSafVault(d, label) {
  await d.launch(); // also brings a backgrounded app to the front
  if (!(await d.isWelcome())) await d.toWelcome();
  const has = await d.eval(`[...document.querySelectorAll('.recent-open')].some(b => b.textContent.includes(${JSON.stringify(label)}))`);
  if (has) await d.openRecent(label);
  else await d.pickSafFolder(label);
}

/** `am kill` (what Android does to cached background apps) until the process is gone; returns true if it went. */
export async function amKill(timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    devSh(`am kill ${PKG}`);
    await sleep(300);
    if (!appPid()) return true;
  }
  return false;
}

/** Kill the app process with SIGKILL right away (one adb round trip). */
export function kill9() {
  const pid = appPid();
  if (pid) runAs(`kill -9 ${pid}`);
  return pid;
}
