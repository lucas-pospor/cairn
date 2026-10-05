// Shared harness for the adv_sync_ui_* tests (sync driven through the real
// desktop app). One real cairn-server, an optional HTTP proxy
// in front of it (to delay or hang the changes feed), the app under
// tauri-driver with private XDG dirs, and `sync_dir` as a second device.
// Patterned on e2e/sync.test.mjs and e2e/adv_verify_sr_18.test.mjs.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { spawn, execFileSync, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

export { Key, sleep };
export const ROOT = path.resolve(import.meta.dirname, "..");
export const APP = path.join(ROOT, "target/debug/cairn");
export const SERVER = path.join(ROOT, "target/debug/cairn-server");
export const SYNC_DIR = path.join(ROOT, "target/debug/examples/sync_dir");
export const TOKEN = "e2e-token-0123456789abcdef";
export const PASS = "e2e passphrase for sync";
export const EVIDENCE = path.join(ROOT, "e2e/.tmp/SU");

export async function eventually(fn, { timeout = 10000, message = "condition", every = 100 } = {}) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e;
    }
    await sleep(every);
  }
  throw new Error(`timed out: ${message} (last: ${last})`);
}

export function evidence(name, text) {
  fs.mkdirSync(EVIDENCE, { recursive: true });
  fs.writeFileSync(path.join(EVIDENCE, name), text);
}

/** A test environment: server (+ proxy), app session, helpers. */
export class Env {
  constructor(tag) {
    this.tmp = fs.mkdtempSync(path.join(os.tmpdir(), `cairn-su-${tag}-`));
    this.sport = 20000 + Math.floor(Math.random() * 2000);
    this.pport = this.sport + 2000;
    this.surl = `http://127.0.0.1:${this.sport}`;
    this.purl = `http://127.0.0.1:${this.pport}`;
    this.delayChanges = 0; // ms to hold each /changes request before forwarding
    this.hangChanges = false;
    this.proxyDown = false; // proxy answers every request by resetting the socket
    this.held = new Set();
    this.log = [];
  }

  dir(name) {
    const p = path.join(this.tmp, name);
    fs.mkdirSync(p, { recursive: true });
    return p;
  }

  async startServer() {
    this.server = spawn(SERVER, [], {
      env: { ...process.env, CAIRN_TOKENS: TOKEN, CAIRN_DATA: path.join(this.tmp, "server"), CAIRN_ADDR: `127.0.0.1:${this.sport}`, ...(this.serverEnv ?? {}) },
      stdio: "ignore",
    });
    await eventually(async () => (await fetch(`${this.surl}/health`)).ok, { message: "server up" });
  }

  stopServer() {
    this.server?.kill("SIGKILL");
    this.server = null;
  }

  startProxy() {
    return new Promise((resolve) => {
      this.proxy = http.createServer(async (req, res) => {
        if (this.proxyDown) {
          req.socket.destroy();
          return;
        }
        if (req.method === "POST" && this.postsLeft !== undefined) {
          if (this.postsLeft <= 0) {
            this.proxyDown = true; // the connection drops mid-sync
            req.socket.destroy();
            return;
          }
          this.postsLeft--;
        }
        if (this.hangAll) {
          this.log.push(`${Date.now()} HANG ${req.method} ${req.url}`);
          this.held.add(res);
          req.resume();
          return;
        }
        if (req.url.includes("/changes")) {
          if (this.hangChanges) {
            this.log.push(`${Date.now()} HANG ${req.method} ${req.url}`);
            this.held.add(res);
            req.resume();
            return;
          }
          if (this.delayChanges) {
            this.log.push(`${Date.now()} DELAY ${req.method} ${req.url}`);
            await sleep(this.delayChanges);
          }
        }
        this.log.push(`${Date.now()} pass ${req.method} ${req.url}`);
        const up = http.request(`${this.surl}${req.url}`, { method: req.method, headers: req.headers }, (ur) => {
          res.writeHead(ur.statusCode, ur.headers);
          ur.pipe(res);
        });
        up.on("error", () => res.destroy());
        req.pipe(up);
      });
      this.proxy.listen(this.pport, "127.0.0.1", resolve);
    });
  }

  async startApp(vault) {
    this.drv = await startDriver(4444, {
      XDG_CONFIG_HOME: path.join(this.tmp, "config"),
      XDG_DATA_HOME: path.join(this.tmp, "data"),
      XDG_CACHE_HOME: path.join(this.tmp, "cache"),
    });
    this.s = await Session.create(this.drv.port, APP, [vault]);
    await this.s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 20000 });
    return this.s;
  }

  async stop() {
    this.hangChanges = false;
    this.hangAll = false;
    for (const r of this.held) r.destroy();
    await this.s?.close();
    this.drv?.proc.kill();
    this.stopServer();
    this.proxy?.close();
    this.proxy?.closeAllConnections?.();
    fs.rmSync(this.tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    // the app can still write its data dir for a moment after the driver goes
    await sleep(1500);
    fs.rmSync(this.tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }

  /** Run sync_dir as the second device. Returns the JSON report, or {error}. */
  syncB(vault, state, { url = this.surl, device = "phone", vaultId = "e2e", pass = PASS } = {}) {
    const r = spawnSync(SYNC_DIR, [vault, state, url, TOKEN, vaultId, device, pass], { encoding: "utf8" });
    if (r.status !== 0) return { error: (r.stderr || "").trim(), status: r.status };
    return JSON.parse(r.stdout);
  }

  async setInput(testid, value) {
    await this.s.exec(
      `const el = document.querySelector('[data-testid=${testid}]'); el.value = arguments[0]; el.dispatchEvent(new Event('input', { bubbles: true }));`,
      value,
    );
  }

  async openSyncSettings() {
    const s = this.s;
    await s.exec(`document.querySelector('[data-testid=open-settings]').click()`);
    await s.click(await s.findWait("[data-testid=settings-sync]"));
    await s.waitFor(`return !!document.querySelector('[data-testid=sync-server]') || !!document.querySelector('[data-testid=sync-state]')`);
  }

  async closeSettings() {
    await this.s.exec(`document.querySelector('[data-testid=settings] .close')?.click()`);
    await this.s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
  }

  /** Fill the setup form and press Connect (does not wait for the result). */
  async fillSetup({ server, token = TOKEN, vaultId = "e2e", device = "laptop", pass = PASS, pass2 = pass }) {
    await this.setInput("sync-server", server);
    await this.setInput("sync-token", token);
    await this.setInput("sync-vault", vaultId);
    await this.setInput("sync-device", device);
    await this.setInput("sync-pass", pass);
    await this.setInput("sync-pass2", pass2);
    await this.s.click(await this.s.find("[data-testid=sync-connect]"));
  }

  /**
   * Connect through the settings UI and wait until it says idle. A vault the
   * server does not have yet is created (the setup question is answered yes).
   */
  async connect(opts) {
    await this.openSyncSettings();
    await this.fillSetup(opts);
    await this.s
      .waitFor(`document.querySelector('[data-testid=dialog-ok]')?.click(); return document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle'`, { timeout: 60000 })
      .catch(async (e) => {
        throw new Error(e.message + " / " + (await this.s.exec(`return document.querySelector('[data-testid=sync-error]')?.textContent`)));
      });
    await this.closeSettings();
  }

  indicator() {
    return this.s.exec(`return document.querySelector('[data-testid=sync-indicator]')?.textContent.trim() ?? null`);
  }

  indicatorTitle() {
    return this.s.exec(`return document.querySelector('[data-testid=sync-indicator]')?.title ?? null`);
  }

  /** Call a Tauri command directly from the page (for observing backend state). */
  invoke(cmd, args = {}) {
    return this.s.execAsync(
      `const done = arguments[arguments.length - 1];
       window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then((v) => done({ ok: v }), (e) => done({ err: String(e?.message ?? JSON.stringify(e)) }));`,
      cmd,
      args,
    );
  }

  editorText() {
    return this.s.exec(`return document.querySelector('.cm-editor')?.__cairnView?.state.doc.toString() ?? null`);
  }

  async openNote(p, { newTab = false } = {}) {
    await this.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path=${JSON.stringify(p)}]')`, { timeout: 10000 });
    await this.s.exec(
      `document.querySelector('[data-testid=tree-row][data-path=' + JSON.stringify(arguments[0]) + ']').dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: arguments[1] }))`,
      p,
      newTab,
    );
    await this.s.waitFor(`return document.querySelector('[data-testid=tab].active')?.dataset.path === ${JSON.stringify(p)} && !!document.querySelector('.cm-editor')?.__cairnView`, { timeout: 10000 });
    await sleep(200);
  }

  async syncNowFromUi({ timeout = 30000 } = {}) {
    await this.s.exec(`document.querySelector('[data-testid=sync-indicator]').click()`);
    await sleep(300);
    await this.s.waitFor(`return !document.querySelector('[data-testid=sync-indicator]')?.classList.contains('syncing')`, { timeout });
  }

  async shot(name) {
    try {
      fs.mkdirSync(EVIDENCE, { recursive: true });
      fs.writeFileSync(path.join(EVIDENCE, name), await this.s.screenshot());
    } catch {}
  }

  appLog() {
    return this.drv?.log() ?? "";
  }
}

export function read(p) {
  try {
    return fs.readFileSync(p, "utf8");
  } catch {
    return null;
  }
}

export function write(p, c) {
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, c);
}

export { fs, path, execFileSync };
