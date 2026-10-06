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
  const proc = spawn(bin, args, { stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, ...env } });
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
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

  async cmd(method, p, body) {
    const r = await fetch(`${this.base}/session/${this.id}${p}`, {
      method,
      body: body === undefined ? undefined : JSON.stringify(body),
      headers: { "content-type": "application/json" },
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

  async screenshot() {
    return Buffer.from(await this.cmd("GET", "/screenshot"), "base64");
  }

  async close() {
    try {
      await this.cmd("DELETE", "");
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
