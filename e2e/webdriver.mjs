// Minimal W3C WebDriver client for tauri-driver (no dependencies).

import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import os from "node:os";
import path from "node:path";

const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";

export async function startDriver(port = 4444, env = {}) {
  const bin = process.env.TAURI_DRIVER ?? path.join(os.homedir(), ".cargo/bin/tauri-driver");
  // CAIRN_WD_PORT / CAIRN_WD_NATIVE_PORT let several e2e runs share a machine
  // (scripts/e2e-headless.sh sets them, one pair per run).
  if (process.env.CAIRN_WD_PORT) port = Number(process.env.CAIRN_WD_PORT);
  const args = ["--port", String(port)];
  if (process.env.CAIRN_WD_NATIVE_PORT) args.push("--native-port", process.env.CAIRN_WD_NATIVE_PORT);
  const proc = spawn(bin, args, { stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, ...noRealNotebook(), ...env } });
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

/**
 * The app's environment default for CAIRN_VAULT. On Windows, msedgedriver
 * hands the app its notebook argument as a lowercased switch
 * (`--c:\users\...`), which the app skips as an option, so without
 * CAIRN_VAULT it opens the most recent notebook of the user's own config.
 * There an empty CAIRN_VAULT opens the Welcome screen instead, and a test
 * that does not pass its notebook in CAIRN_VAULT fails without touching a
 * real one. Elsewhere the argument arrives as given, and some tests rely on
 * the recent list, so nothing is set.
 *
 * This keeps the tests out of the user's notebooks, not out of the user's
 * Cairn config. The XDG_* variables the tests set do nothing on Windows: the
 * app still writes the real %APPDATA%\app.cairn.notes (the recent list,
 * plugin approvals, sync state). After a run the first recent notebook is a
 * temporary folder that is gone, so the next normal launch opens the Welcome
 * screen.
 */
export function noRealNotebook() {
  return process.platform === "win32" ? { CAIRN_VAULT: "" } : {};
}

/**
 * WebKitWebDriver and the app it starts write to tauri-driver's stderr pipe.
 * If one of them outlives tauri-driver (a session that could not be closed),
 * node would wait for that pipe after the last test, forever: once
 * tauri-driver is gone, its pipes no longer keep the run alive.
 */
export function releaseOnExit(proc) {
  proc.once("exit", () => {
    proc.stdout?.unref();
    proc.stderr?.unref();
  });
}

export class Session {
  constructor(base, id) {
    this.base = base;
    this.id = id;
  }

  static async create(port, application, args = [], env = {}) {
    const base = `http://127.0.0.1:${port}`;
    const caps = { capabilities: { alwaysMatch: { "tauri:options": { application, args, env } } } };
    const r = await fetch(`${base}/session`, { method: "POST", body: JSON.stringify(caps), headers: { "content-type": "application/json" } });
    const j = await r.json();
    if (!r.ok) throw new Error(JSON.stringify(j));
    return new Session(base, j.value.sessionId);
  }

  /** `timeout` (ms): give up on a command that gets no answer (a wedged WebKitWebDriver). */
  async cmd(method, p, body, timeout) {
    const r = await fetch(`${this.base}/session/${this.id}${p}`, {
      method,
      body: body === undefined ? undefined : JSON.stringify(body),
      headers: { "content-type": "application/json" },
      signal: timeout ? AbortSignal.timeout(timeout) : undefined,
    });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(`${method} ${p}: ${JSON.stringify(j.value ?? j)}`);
    return j.value;
  }

  async exec(script, ...args) {
    return this.cmd("POST", "/execute/sync", { script, args });
  }

  async execAsync(script, ...args) {
    return this.cmd("POST", "/execute/async", { script, args });
  }

  async find(css) {
    const v = await this.cmd("POST", "/element", { using: "css selector", value: css });
    return v[ELEMENT];
  }

  /** Like find, but waits up to `timeout` ms for the element to appear. */
  async findWait(css, timeout = 5000) {
    const end = Date.now() + timeout;
    for (;;) {
      try {
        return await this.find(css);
      } catch (e) {
        if (Date.now() > end) throw e;
        await sleep(50);
      }
    }
  }

  async findAll(css) {
    const v = await this.cmd("POST", "/elements", { using: "css selector", value: css });
    return v.map((e) => e[ELEMENT]);
  }

  /** Wait until `fn` (evaluated in the page) returns truthy; returns its value. */
  async waitFor(script, { timeout = 5000, message = script } = {}) {
    const end = Date.now() + timeout;
    let last;
    while (Date.now() < end) {
      try {
        last = await this.exec(script);
        if (last) return last;
      } catch (e) {
        last = e;
      }
      await sleep(80);
    }
    throw new Error(`timed out waiting for: ${message} (last: ${last})`);
  }

  async click(el) {
    return this.cmd("POST", `/element/${el}/click`, {});
  }

  async type(el, text) {
    return this.cmd("POST", `/element/${el}/value`, { text });
  }

  async text(el) {
    return this.cmd("GET", `/element/${el}/text`);
  }

  /** Send keys to whatever has focus, using the actions API. */
  async keys(...seq) {
    const actions = [];
    for (const k of seq) {
      if (typeof k === "string") {
        for (const ch of k) actions.push({ type: "keyDown", value: ch }, { type: "keyUp", value: ch });
      } else {
        // { chord: [mod..., key] }
        for (const c of k.chord) actions.push({ type: "keyDown", value: c });
        for (const c of [...k.chord].reverse()) actions.push({ type: "keyUp", value: c });
      }
    }
    await this.cmd("POST", "/actions", { actions: [{ type: "key", id: "kb", actions }] });
    await this.cmd("DELETE", "/actions");
  }

  async pointer(actions) {
    await this.cmd("POST", "/actions", {
      actions: [{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions }],
    });
    await this.cmd("DELETE", "/actions");
  }

  // Screenshots and close run in teardown (after hooks, failure shots): a
  // wedged WebKitWebDriver must not hang the run there, so both give up.

  async screenshot(timeout = 15000) {
    return Buffer.from(await this.cmd("GET", "/screenshot", undefined, timeout), "base64");
  }

  async close(timeout = 15000) {
    try {
      await this.cmd("DELETE", "", undefined, timeout);
    } catch {}
  }
}

export const Key = {
  ctrl: "",
  alt: "",
  shift: "",
  enter: "",
  escape: "",
  backspace: "",
  down: "",
  up: "",
  tab: "",
  f2: "",
  delete: "",
};
