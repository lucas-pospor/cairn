// Helpers for the adversarial editor/UI e2e tests (adv_editor*.test.mjs,
// adv_ui*.test.mjs). Each test gets its own temp vault, its own temp XDG
// dirs and its own app process, so a failing test cannot break later ones.
//
// Run through scripts/e2e-headless.sh (private headless display and ports).

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { spawn } from "node:child_process";
import net from "node:net";
import { Session, Key, releaseOnExit } from "./webdriver.mjs";

export { Key, sleep };
export const ROOT = path.resolve(import.meta.dirname, "..");
export const APP = path.join(ROOT, "target/debug/cairn");
export const EVIDENCE = path.join(import.meta.dirname, ".tmp", "ED");

export function mkTmp(prefix = "cairn-adv-ed-") {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix));
}

export class Dir {
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
  exists(rel) {
    return fs.existsSync(this.p(rel));
  }
  rm(rel) {
    fs.rmSync(this.p(rel), { recursive: true, force: true });
  }
  rename(a, b) {
    fs.renameSync(this.p(a), this.p(b));
  }
  /** All files below the root (relative paths), dot-folders included. */
  list() {
    return fs.readdirSync(this.root, { recursive: true }).map(String).sort();
  }
}

export async function eventually(fn, { timeout = 5000, message = "condition" } = {}) {
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
    await sleep(80);
  }
  throw new Error(`timed out: ${message}${err ? ` (${err.message})` : ""}${last !== undefined ? ` last=${JSON.stringify(last)}` : ""}`);
}

/** True if something accepts TCP connections on 127.0.0.1:port. */
function portBusy(port) {
  return new Promise((resolve) => {
    const sock = net.connect({ host: "127.0.0.1", port });
    const done = (v) => {
      sock.destroy();
      resolve(v);
    };
    sock.setTimeout(400, () => done(false));
    sock.once("connect", () => done(true));
    sock.once("error", () => done(false));
  });
}

/**
 * Like startDriver in webdriver.mjs, but always passes --native-driver so
 * the driver can run with a restricted PATH (env.PATH).
 */
export async function startDriver(port = 4444, env = {}, cwd = undefined) {
  const bin = process.env.TAURI_DRIVER ?? path.join(os.homedir(), ".cargo/bin/tauri-driver");
  if (process.env.CAIRN_WD_PORT) port = Number(process.env.CAIRN_WD_PORT);
  let nativePort = process.env.CAIRN_WD_NATIVE_PORT ?? null;
  for (let i = 0; ; i++) {
    // A driver from an earlier (timed out) test may still hold the port.
    if (!(await portBusy(port)) && !(nativePort && (await portBusy(Number(nativePort))))) break;
    if (i > 10) {
      // A leaked driver from another run holds this slot's ports: fall back
      // to a free random pair instead of failing.
      for (let k = 0; k < 50; k++) {
        const p = 30000 + 2 * Math.floor(Math.random() * 10000);
        if (!(await portBusy(p)) && !(await portBusy(p + 1))) {
          port = p;
          nativePort = String(p + 1);
          break;
        }
      }
      break;
    }
    await sleep(100);
  }
  const args = ["--port", String(port), "--native-driver", process.env.WEBKIT_WEBDRIVER ?? "/usr/bin/WebKitWebDriver"];
  if (nativePort) args.push("--native-port", nativePort);
  const proc = spawn(bin, args, { stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, ...env }, cwd });
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  releaseOnExit(proc);
  for (let i = 0; i < 100; i++) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}/status`);
      if (r.ok) return { proc, port, log: () => log };
    } catch {}
    await sleep(100);
  }
  proc.kill();
  throw new Error("tauri-driver did not start:\n" + log);
}

/** Session.create with a timeout (a stuck WebKitWebDriver must not hang the run). */
export async function createSession(port, application, args = [], timeoutMs = 30000) {
  const base = `http://127.0.0.1:${port}`;
  const caps = { capabilities: { alwaysMatch: { "tauri:options": { application, args } } } };
  const r = await fetch(`${base}/session`, {
    method: "POST",
    body: JSON.stringify(caps),
    headers: { "content-type": "application/json" },
    signal: AbortSignal.timeout(timeoutMs),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(JSON.stringify(j));
  return new Session(base, j.value.sessionId);
}

/** Script run in the page right after start: collect errors, wrap invoke. */
const INSTRUMENT = `
  if (!window.__adv) {
    window.__adv = { errs: [], invokes: [], blockOpen: true };
    const push = (k, a) => window.__adv.errs.push(k + ': ' + a.map(x => (x && x.stack) ? String(x.stack).slice(0, 400) : String(x)).join(' '));
    const ce = console.error.bind(console);
    console.error = (...a) => { push('console.error', a); ce(...a); };
    const cw = console.warn.bind(console);
    console.warn = (...a) => { push('console.warn', a); cw(...a); };
    window.addEventListener('error', (e) => push('error', [e.message]));
    window.addEventListener('unhandledrejection', (e) => push('rejection', [e.reason && (e.reason.message || JSON.stringify(e.reason))]));
    // Record IPC calls (Tauri sends them with fetch to ipc://localhost/<cmd>)
    // and never let a test launch a real browser: open_url is answered here
    // unless window.__adv.blockOpen is set to false.
    const of = window.fetch.bind(window);
    window.fetch = function (input, init) {
      const url = typeof input === 'string' ? input : (input && input.url) || '';
      const pre = ['ipc://localhost/', 'http://ipc.localhost/'].find(p => url.startsWith(p));
      const m = pre ? [url, url.slice(pre.length)] : null;
      if (m) {
        const cmd = decodeURIComponent(m[1]);
        const body = init && typeof init.body === 'string' ? init.body : null;
        window.__adv.invokes.push({ cmd, body: body && body.length > 400 ? body.slice(0, 400) : body });
        if (cmd === 'plugin:opener|open_url' && window.__adv.blockOpen) {
          return Promise.resolve(new Response(JSON.stringify('blocked by e2e test'), { headers: { 'Tauri-Response': 'error', 'content-type': 'application/json' } }));
        }
      }
      return of(input, init);
    };
  }
  return true;
`;

/**
 * Start tauri-driver and the app on `vault`. `xdg` is a folder for the
 * XDG config/data/cache dirs (reuse it to keep localStorage and the recent
 * list between two launches). `args` defaults to [vault].
 */
export async function launch({ vault, xdg, args, waitRows = 1, env = {}, cwd } = {}) {
  const drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(xdg, "config"),
    XDG_DATA_HOME: path.join(xdg, "data"),
    XDG_CACHE_HOME: path.join(xdg, "cache"),
    ...env,
  }, cwd);
  let s;
  try {
    s = await createSession(drv.port, APP, args ?? (vault ? [vault] : []));
    if (vault && waitRows > 0) {
      await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= ${waitRows}`, { timeout: 20000 });
    } else {
      await s.waitFor(`return !!document.querySelector('.welcome, .workspace')`, { timeout: 20000 });
    }
    await s.exec(INSTRUMENT);
  } catch (e) {
    await s?.close();
    drv.proc.kill();
    throw new Error(`${e.message}\n--- driver log ---\n${drv.log().slice(-3000)}`);
  }
  const app = new AppDriver(s, drv);
  return app;
}

export class AppDriver {
  constructor(s, drv) {
    this.s = s;
    this.drv = drv;
  }
  exec(script, ...args) {
    return this.s.exec(script, ...args);
  }
  async stop() {
    // Session.close gives up when a wedged WebKitWebDriver does not answer.
    await this.s.close();
    const p = this.drv.proc;
    if (p.exitCode == null) {
      const done = new Promise((r) => p.once("exit", r));
      p.kill();
      await Promise.race([done, sleep(3000)]);
    }
  }
  async instrument() {
    return this.s.exec(INSTRUMENT);
  }
  async shot(name) {
    try {
      fs.mkdirSync(EVIDENCE, { recursive: true });
      fs.writeFileSync(path.join(EVIDENCE, name + ".png"), await this.s.screenshot());
    } catch {}
  }
  errors() {
    return this.s.exec(`return window.__adv ? window.__adv.errs : []`);
  }
  invokes(cmd) {
    return this.s.exec(`return (window.__adv ? window.__adv.invokes : []).filter(i => !arguments[0] || i.cmd === arguments[0])`, cmd ?? null);
  }
  /** Call a Tauri command directly (bypassing the UI). */
  invoke(cmd, args = {}) {
    return this.s.execAsync(
      `const done = arguments[arguments.length - 1];
       window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then(v => done({ ok: v }), e => done({ err: e }));`,
      cmd,
      args,
    );
  }
  text() {
    return this.s.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
  }
  sel() {
    return this.s.exec(`const s = document.querySelector('.cm-editor').__cairnView.state.selection.main; return { anchor: s.anchor, head: s.head, from: s.from, to: s.to }`);
  }
  async setSel(anchor, head = anchor, focus = true) {
    await this.s.exec(
      `const v = document.querySelector('.cm-editor').__cairnView; if (arguments[2]) v.focus(); v.dispatch({ selection: { anchor: arguments[0], head: arguments[1] } });`,
      anchor,
      head,
      focus,
    );
  }
  async focusEnd() {
    await this.s.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } });`);
  }
  async blur() {
    await this.s.exec(`document.activeElement && document.activeElement.blur(); document.querySelector('[data-testid=file-tree]')?.focus();`);
  }
  activeTab() {
    return this.s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
  }
  tabs() {
    return this.s.exec(`return [...document.querySelectorAll('[data-testid=tab]')].map(e => e.dataset.path)`);
  }
  toasts() {
    return this.s.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent.trim())`);
  }
  contentText() {
    return this.s.exec(`return document.querySelector('.cm-content').textContent`);
  }
  keys(...k) {
    return this.s.keys(...k);
  }
  async click(css) {
    return this.s.click(await this.s.findWait(css));
  }
  async openFromTree(p) {
    const rowSel = `[data-testid=tree-row][data-path="${p}"]`;
    // expand parent folders
    const parts = p.split("/");
    for (let i = 1; i < parts.length; i++) {
      const dir = parts.slice(0, i).join("/");
      const open = await this.s.exec(`return !!document.querySelector('[data-testid=tree-row][data-path="${parts.slice(0, i + 1).join("/")}"]')`);
      if (!open) await this.s.click(await this.s.findWait(`[data-testid=tree-row][data-path="${dir}"]`));
    }
    await this.s.click(await this.s.findWait(rowSel));
    await eventually(async () => (await this.activeTab()) === p, { message: `tab ${p} active` });
    await this.s.waitFor(`const t = document.querySelector('[data-testid=tab][aria-selected=true]'); const v = document.querySelector('.cm-editor')?.__cairnView; return !!v`);
    await sleep(150);
  }
  async setMode(mode) {
    await this.s.click(await this.s.findWait(`[data-testid=mode-${mode}]`));
    await sleep(100);
  }
  /** Bounding box of the first element matching css (page coordinates). */
  rect(css, index = 0) {
    return this.s.exec(
      `const el = document.querySelectorAll(arguments[0])[arguments[1]]; if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height };`,
      css,
      index,
    );
  }
  /** Real pointer click at viewport coordinates (through WebDriver actions). */
  async clickAt(x, y, { button = 0 } = {}) {
    await this.s.pointer([
      { type: "pointerMove", origin: "viewport", x: Math.round(x), y: Math.round(y) },
      { type: "pointerDown", button },
      { type: "pointerUp", button },
    ]);
  }
  saveState() {
    return this.s.exec(`return document.querySelector('[data-testid=save-state]')?.textContent.trim() ?? null`);
  }
  /**
   * The headless test display never gives the window OS focus, so
   * document.hasFocus() is false and CodeMirror's view.hasFocus with it; Live
   * Preview would then render the cursor line as if the editor were blurred.
   * Pretend the window is focused (what a user at the screen has).
   */
  async fakeFocus() {
    await this.s.exec(`document.hasFocus = () => true; return true`);
  }
  /** Open a note by clicking its row in the tree (JS click, no expansion). */
  async open(p) {
    // The left sidebar may show search or tags: switch it to the file tree.
    await this.s.exec(`if (!document.querySelector('[data-testid=file-tree]')) document.querySelector('[data-testid=tab-files]')?.click(); return true`);
    await this.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="' + ${JSON.stringify(p)} + '"]')`, { message: `tree row ${p}` });
    await this.s.exec(`document.querySelector('[data-testid=tree-row][data-path="' + arguments[0] + '"]').click(); return true`, p);
    await eventually(async () => (await this.activeTab()) === p, { message: `tab ${p} active` });
    await this.s.waitFor(`const v = document.querySelector('.cm-editor')?.__cairnView; return !!v && document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === ${JSON.stringify(p)}`);
    await sleep(250);
  }
  /** Ctrl (or another modifier) + key through real WebDriver key actions. */
  chord(...k) {
    return this.s.keys({ chord: k });
  }
  /** Center of the n-th element matching css, or null. */
  async center(css, index = 0) {
    const r = await this.rect(css, index);
    return r ? { x: r.x + r.w / 2, y: r.y + r.h / 2 } : null;
  }
  /** Number of the line holding the main cursor, and its text. */
  cursorLine() {
    return this.s.exec(`const v = document.querySelector('.cm-editor').__cairnView; const l = v.state.doc.lineAt(v.state.selection.main.head); return { n: l.number, text: l.text, head: v.state.selection.main.head }`);
  }
  /** Dispatch a synthetic keydown (for keyboard layouts WebDriver cannot select). */
  keydownInEditor(init) {
    return this.s.exec(
      `const v = document.querySelector('.cm-editor').__cairnView; v.focus();
       const ev = new KeyboardEvent('keydown', Object.assign({ bubbles: true, cancelable: true }, arguments[0]));
       v.contentDOM.dispatchEvent(ev); return ev.defaultPrevented;`,
      init,
    );
  }
}

/** PIDs of processes whose command line mentions `needle` (read from /proc). */
export function pidsWith(needle) {
  const out = [];
  for (const d of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(d) || Number(d) === process.pid) continue;
    try {
      const cmd = fs.readFileSync(`/proc/${d}/cmdline`, "utf8");
      if (cmd.includes(needle)) out.push(Number(d));
    } catch {}
  }
  return out;
}

/** Wait for app processes using `tmp` to exit (kill them by PID), then remove it. */
export async function cleanupTmp(tmp) {
  for (let i = 0; i < 40 && pidsWith(tmp).length; i++) await sleep(100);
  for (const pid of pidsWith(tmp)) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {}
  }
  await sleep(100);
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}

/** Create a fresh temp root with a vault folder and an xdg folder. */
export function freshEnv(files = {}) {
  const tmp = mkTmp();
  const vault = new Dir(path.join(tmp, "vault"));
  for (const [k, v] of Object.entries(files)) vault.write(k, v);
  const xdg = path.join(tmp, "xdg");
  fs.mkdirSync(xdg, { recursive: true });
  return { tmp, vault, xdg, cleanup: () => cleanupTmp(tmp) };
}

/**
 * Run `fn(app, env)` with a fresh vault and app; always cleans up.
 * Saves a screenshot named `shot` on failure.
 */
export async function withApp(files, fn, { shot, launchOpts = {}, keep = false } = {}) {
  const env = freshEnv(files);
  let app;
  try {
    app = await launch({ vault: env.vault.root, xdg: env.xdg, ...launchOpts });
    return await fn(app, env);
  } catch (e) {
    if (app && shot) await app.shot(shot);
    if (app) {
      try {
        const errs = await app.errors();
        if (errs.length) e.message += `\n--- page errors ---\n${errs.join("\n").slice(0, 3000)}`;
      } catch {}
    }
    throw e;
  } finally {
    await app?.stop();
    if (!keep) await env.cleanup();
  }
}
