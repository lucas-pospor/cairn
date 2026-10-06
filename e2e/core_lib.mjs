// Helpers for the core plugin end-to-end tests (e2e/core_*.test.mjs). Each test
// file starts the built app on a throwaway vault in the system temp folder.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

export { Key, sleep };

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const SHOTS = path.join(import.meta.dirname, ".tmp");

export async function eventually(fn, { timeout = 5000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let err;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      err = e;
    }
    await sleep(80);
  }
  throw new Error(`timed out: ${message}${err ? ` (${err.message})` : ""}`);
}

/** The app on a vault made of `files` (vault path -> text). */
export class CoreApp {
  static async start(name, files) {
    const app = new CoreApp();
    app.tmp = fs.mkdtempSync(path.join(os.tmpdir(), `cairn-${name}-`));
    app.name = name;
    app.vault = path.join(app.tmp, "vault");
    fs.mkdirSync(app.vault);
    for (const [rel, content] of Object.entries(files)) app.write(rel, content);
    app.drv = await startDriver(4444, {
      XDG_CONFIG_HOME: path.join(app.tmp, "config"),
      XDG_DATA_HOME: path.join(app.tmp, "data"),
      XDG_CACHE_HOME: path.join(app.tmp, "cache"),
    });
    app.s = await Session.create(app.drv.port, APP, [app.vault]);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row]')`, { timeout: 15000 });
    return app;
  }

  async stop() {
    if (this.s) {
      fs.mkdirSync(SHOTS, { recursive: true });
      fs.writeFileSync(path.join(SHOTS, `${this.name}-final.png`), await this.s.screenshot().catch(() => Buffer.alloc(0)));
    }
    await this.s?.close();
    this.drv?.proc.kill();
    // The app can still be saving the settings as it closes: try again for a moment.
    fs.rmSync(this.tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }

  write(rel, content) {
    const p = path.join(this.vault, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, content);
  }
  read = (rel) => fs.readFileSync(path.join(this.vault, rel), "utf8");
  exists = (rel) => fs.existsSync(path.join(this.vault, rel));
  /** The vault's files under `dir`, as vault paths, sorted (dot-folders left out). */
  files(dir = "") {
    const out = [];
    const walk = (rel) => {
      for (const e of fs.readdirSync(path.join(this.vault, rel), { withFileTypes: true })) {
        if (e.name.startsWith(".")) continue;
        const p = rel ? `${rel}/${e.name}` : e.name;
        if (e.isDirectory()) walk(p);
        else out.push(p);
      }
    };
    walk(dir);
    return out.sort();
  }
  settingsFile() {
    return JSON.parse(this.read(".cairn/settings.json"));
  }

  exec(script, ...args) {
    return this.s.exec(script, ...args);
  }
  waitFor(script, opts) {
    return this.s.waitFor(script, opts);
  }
  async click(css) {
    await this.s.click(await this.s.findWait(css));
  }

  async openSettings(section) {
    if (!(await this.exec(`return !!document.querySelector('[data-testid=settings]')`))) await this.click("[data-testid=open-settings]");
    await this.click(`[data-testid=settings-${section}]`);
    await this.waitFor(`return document.querySelector('[data-testid=settings-${section}]')?.getAttribute('aria-current') === 'page'`);
  }

  async closeSettings() {
    await this.click("[data-testid=settings] .close");
    await this.waitFor(`return !document.querySelector('[data-testid=settings]')`);
  }

  /** The editor's text. */
  doc() {
    return this.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
  }
  activeTab() {
    return this.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
  }
  async openNote(p) {
    await this.exec(`document.querySelector('[data-testid=tree-row][data-path="' + arguments[0] + '"]').click()`, p);
    await eventually(async () => (await this.activeTab()) === p, { message: `tab ${p} active` });
  }

  /** The names the command palette lists for `query`. */
  async paletteNames(query) {
    await this.s.keys({ chord: [Key.ctrl, "p"] });
    await this.s.type(await this.s.findWait("[data-testid=palette-input]"), query);
    await sleep(150);
    const names = await this.exec(`return [...document.querySelectorAll('[data-testid=palette-item]')].map(e => e.querySelector('span').textContent)`);
    await this.s.keys(Key.escape);
    await this.waitFor(`return !document.querySelector('[data-testid=palette-input]')`);
    return names;
  }

  /** Run the command named `name` from the palette. */
  async runCommand(name) {
    await this.s.keys({ chord: [Key.ctrl, "p"] });
    await this.s.type(await this.s.findWait("[data-testid=palette-input]"), name);
    await this.waitFor(`return document.querySelector('[data-testid=palette-item]')?.querySelector('span').textContent === ${JSON.stringify(name)}`);
    await this.s.keys(Key.enter);
    await this.waitFor(`return !document.querySelector('[data-testid=palette-input]')`);
  }

  toasts() {
    return this.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent.trim())`);
  }
}
