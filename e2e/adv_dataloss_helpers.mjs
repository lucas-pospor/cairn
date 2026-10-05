// Shared harness for the adversarial data-loss e2e tests (adv_dataloss*.test.mjs).
// Drives the real debug app (target/debug/cairn) through tauri-driver.
// Run through scripts/e2e-headless.sh so each run has its own display/ports.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

export { Key, sleep };
export const ROOT = path.resolve(import.meta.dirname, "..");
export const APP = path.join(ROOT, "target/debug/cairn");
export const EVIDENCE = path.join(ROOT, "e2e/.tmp/DL");

export async function eventually(fn, { timeout = 5000, message = "condition", interval = 50 } = {}) {
  const end = Date.now() + timeout;
  let err;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      err = e;
    }
    await sleep(interval);
  }
  throw new Error(`timed out: ${message}${err ? ` (${err.message})` : ""}${last !== undefined ? ` (last: ${JSON.stringify(last)})` : ""}`);
}

/** Make everything under `p` writable again (for cleanup after chmod tests). */
function makeWritable(p) {
  try {
    const st = fs.lstatSync(p);
    if (st.isSymbolicLink()) return;
    fs.chmodSync(p, st.isDirectory() ? 0o755 : 0o644);
    if (st.isDirectory()) for (const n of fs.readdirSync(p)) makeWritable(path.join(p, n));
  } catch {}
}

export class Vault {
  constructor(root) {
    this.root = root;
    fs.mkdirSync(root, { recursive: true });
  }
  p(rel) {
    return path.join(this.root, rel);
  }
  write(rel, content) {
    fs.mkdirSync(path.dirname(this.p(rel)), { recursive: true });
    fs.writeFileSync(this.p(rel), content);
  }
  read(rel) {
    return fs.readFileSync(this.p(rel), "utf8");
  }
  bytes(rel) {
    return fs.readFileSync(this.p(rel));
  }
  exists(rel) {
    return fs.existsSync(this.p(rel));
  }
  rm(rel) {
    fs.rmSync(this.p(rel), { recursive: true, force: true });
  }
  mv(a, b) {
    fs.renameSync(this.p(a), this.p(b));
  }
  /** Every non-hidden path under the vault (files and folders), sorted. */
  listDisk() {
    const out = [];
    const walk = (dir, rel) => {
      for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
        if (ent.name.startsWith(".")) continue;
        const r = rel ? `${rel}/${ent.name}` : ent.name;
        out.push(r);
        if (ent.isDirectory()) walk(path.join(dir, ent.name), r);
      }
    };
    walk(this.root, "");
    return out.sort();
  }
}

/** One tauri-driver (and one temp root with private XDG dirs) per test file. */
export class Env {
  static async create(tag) {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), `cairn-dl-${tag}-`));
    const drv = await startDriver(4444, {
      XDG_CONFIG_HOME: path.join(tmp, "config"),
      XDG_DATA_HOME: path.join(tmp, "data"),
      XDG_CACHE_HOME: path.join(tmp, "cache"),
      RUST_LOG: process.env.RUST_LOG ?? "info",
    });
    fs.mkdirSync(EVIDENCE, { recursive: true });
    return new Env(tag, tmp, drv);
  }
  constructor(tag, tmp, drv) {
    this.tag = tag;
    this.tmp = tmp;
    this.drv = drv;
    this.apps = [];
    this.n = 0;
  }
  vault(name, files = {}) {
    const v = new Vault(path.join(this.tmp, `${name}-${++this.n}`));
    for (const [rel, c] of Object.entries(files)) v.write(rel, c);
    return v;
  }
  async launch(vault, { args } = {}) {
    const s = await Session.create(this.drv.port, APP, args ?? [vault.root]);
    const a = new AppDriver(s, vault, this);
    this.apps.push(a);
    await a.waitReady();
    return a;
  }
  /** Driver log (includes the app's stderr / env_logger output). */
  log() {
    return this.drv.log();
  }
  async dispose() {
    for (const a of this.apps) await a.close();
    this.drv?.proc.kill();
    // The app keeps writing its GPU shader cache for a moment after the
    // session ends, so remove the temp root until it stays gone.
    await sleep(800);
    for (let i = 0; i < 12; i++) {
      makeWritable(this.tmp);
      try {
        fs.rmSync(this.tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
      } catch {}
      await sleep(400);
      if (!fs.existsSync(this.tmp)) return;
    }
    console.warn(`could not remove ${this.tmp}`);
  }
}

const rowSel = (p) => `[data-testid=tree-row][data-path="${p.replace(/"/g, '\\"')}"]`;

export class AppDriver {
  constructor(s, vault, env) {
    this.s = s;
    this.vault = vault;
    this.env = env;
    this.closed = false;
  }
  exec(script, ...args) {
    return this.s.exec(script, ...args);
  }
  async waitReady() {
    await this.s.waitFor(
      `return !!document.querySelector('[data-testid=file-tree]') || !!document.querySelector('[data-testid=vault-path]')`,
      { timeout: 20000 },
    );
  }
  /**
   * Close the window the way the title-bar X does: a close request the app
   * sees first (and may answer by saving, or by asking). WebDriver's Close
   * Window is a different path: WebKit drops the web view at once, as for a
   * page script calling window.close(), so no app gets a chance to save.
   */
  closeWindow() {
    return this.exec(`window.__TAURI_INTERNALS__.invoke("plugin:window|close", { label: "main" }); return 1`);
  }
  /** Call a backend command directly (like backend.ts does). */
  invoke(cmd, args = {}) {
    return this.s.execAsync(
      `const [cmd, args, done] = arguments;
       window.__TAURI_INTERNALS__.invoke(cmd, args).then(v => done({ ok: v }), e => done({ err: e }));`,
      cmd,
      args,
    );
  }
  activeTab() {
    return this.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
  }
  tabs() {
    return this.exec(
      `return [...document.querySelectorAll('[data-testid=tab]')].map(t => ({ path: t.dataset.path, dirty: !!t.querySelector('.dot'), active: t.getAttribute('aria-selected') === 'true' }))`,
    );
  }
  editorText() {
    return this.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
  }
  banner() {
    return this.exec(`return document.querySelector('[data-testid=conflict-banner]')?.textContent.trim() ?? null`);
  }
  saveState() {
    return this.exec(`return document.querySelector('[data-testid=save-state]')?.textContent.trim() ?? null`);
  }
  toasts() {
    return this.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent.trim())`);
  }
  async expandTo(rel) {
    const parts = rel.split("/");
    for (let i = 1; i < parts.length; i++) {
      const dir = parts.slice(0, i).join("/");
      const child = parts.slice(0, i + 1).join("/");
      await this.s.waitFor(`return !!document.querySelector('${rowSel(dir)}')`, { timeout: 5000, message: `row ${dir}` });
      const has = await this.exec(`return !!document.querySelector('${rowSel(child)}')`);
      if (!has) {
        await this.s.click(await this.s.find(rowSel(dir)));
        await this.s.waitFor(`return !!document.querySelector('${rowSel(child)}')`, { timeout: 5000, message: `row ${child}` });
      }
    }
  }
  /** Click a note in the tree (replaces the active tab, like a user's plain click). */
  async openFromTree(rel, { newTab = false } = {}) {
    await this.expandTo(rel);
    await this.s.waitFor(`return !!document.querySelector('${rowSel(rel)}')`, { timeout: 5000, message: `row ${rel}` });
    if (newTab) {
      await this.exec(`document.querySelector(arguments[0]).dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, ctrlKey: true, button: 0 }))`, rowSel(rel));
    } else {
      await this.s.click(await this.s.find(rowSel(rel)));
    }
    await eventually(async () => (await this.activeTab()) === rel, { message: `tab ${rel} active` });
    await this.waitLoaded(rel);
  }
  async waitLoaded(rel) {
    // The editor shows the tab once its document is loaded.
    await this.s.waitFor(
      `const t = document.querySelector('[data-testid=tab][aria-selected=true]'); return t && t.dataset.path === ${JSON.stringify(rel)} && !!document.querySelector('.cm-editor')`,
      { timeout: 10000 },
    );
    await sleep(120);
  }
  async source() {
    await this.s.click(await this.s.find("[data-testid=mode-source]"));
  }
  async focusEnd() {
    await this.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } });`);
  }
  async focusAt(pos) {
    await this.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: arguments[0] } });`, pos);
  }
  /** Type real key events at the end of the document. */
  async typeEnd(text) {
    await this.focusEnd();
    await this.s.keys(text);
  }
  /** Insert text as a user edit (no key events; fast, triggers autosave). */
  async insertEnd(text) {
    await this.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: v.state.doc.length, insert: arguments[0] }, userEvent: 'input.type' });`, text);
  }
  async insertAt(pos, text) {
    await this.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: arguments[0], insert: arguments[1] }, userEvent: 'input.type' });`, pos, text);
  }
  async waitSaved(timeout = 5000) {
    await eventually(async () => (await this.saveState()) === "Saved", { timeout, message: "status Saved" });
  }
  async waitBanner(timeout = 6000) {
    return eventually(() => this.banner(), { timeout, message: "conflict banner" });
  }
  async clickTestId(id) {
    await this.s.click(await this.s.find(`[data-testid=${id}]`));
  }
  async clickButtonText(text) {
    const ok = await this.exec(
      `const b = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === arguments[0]); if (!b) return false; b.click(); return true;`,
      text,
    );
    if (!ok) throw new Error(`no button "${text}"`);
  }
  async closeActiveTab() {
    await this.s.click(await this.s.find("[data-testid=tab][aria-selected=true] .close"));
  }
  /** Close every tab (discarding anything unsaved). Used for cleanup between tests. */
  async closeAllTabs() {
    for (let i = 0; i < 30; i++) {
      const n = await this.exec(`const b = document.querySelector('[data-testid=tab] .close'); if (b) b.click(); return document.querySelectorAll('[data-testid=tab]').length;`);
      if (!n) return;
      await sleep(60);
    }
  }
  async shot(name) {
    try {
      fs.writeFileSync(path.join(EVIDENCE, `${name}.png`), await this.s.screenshot());
    } catch {}
  }
  async close() {
    if (this.closed) return;
    this.closed = true;
    await this.s.close();
  }
}
