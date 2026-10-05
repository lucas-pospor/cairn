// FINDING-146 (Turn off only): how long Settings > Sync >
// Turn off really takes while a sync is stuck on a server that never answers,
// with a long enough wait to see it finish, and a log of every hung changes
// request (to see whether the background sync loop gets the lock first and
// starts another hung sync before the disconnect runs).
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_sr_18_02.test.mjs   (about 3 min)
//
// See adv_verify_sr_18.test.mjs for the setup (a proxy that stops answering the changes feed).

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = path.join(ROOT, "target/debug/cairn");
const SERVER = path.join(ROOT, "target/debug/cairn-server");
const TOKEN = "e2e-token-0123456789abcdef";
const PASS = "e2e passphrase for sync";
const SPORT = 19000 + Math.floor(Math.random() * 500);
const PPORT = SPORT + 500;
const SURL = `http://127.0.0.1:${SPORT}`;
const PURL = `http://127.0.0.1:${PPORT}`;

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-sr18b-"));
const vault = path.join(tmp, "laptop");

let server, proxy, drv, s;
let hangChanges = false;
const held = new Set();
const log = [];

function startProxy() {
  return new Promise((resolve) => {
    proxy = http.createServer((req, res) => {
      if (hangChanges && req.url.includes("/changes")) {
        // accept and never answer
        log.push(`${Date.now()} HANG ${req.method} ${req.url}`);
        held.add(res);
        req.on("close", () => log.push(`${Date.now()} CLOSED by client ${req.url}`));
        req.resume();
        return;
      }
      log.push(`${Date.now()} pass ${req.method} ${req.url}`);
      const up = http.request(`${SURL}${req.url}`, { method: req.method, headers: req.headers }, (ur) => {
        res.writeHead(ur.statusCode, ur.headers);
        ur.pipe(res);
      });
      up.on("error", () => res.destroy());
      req.pipe(up);
    });
    proxy.listen(PPORT, "127.0.0.1", resolve);
  });
}

async function eventually(fn, { timeout = 10000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e;
    }
    await sleep(100);
  }
  throw new Error(`timed out: ${message} (last: ${last})`);
}

async function setInput(testid, value) {
  await s.exec(
    `const el = document.querySelector('[data-testid=${testid}]'); el.value = arguments[0]; el.dispatchEvent(new Event('input', { bubbles: true }));`,
    value,
  );
}

const indicator = () => s.exec(`return document.querySelector('[data-testid=sync-indicator]')?.textContent.trim()`);

async function startHungSync() {
  hangChanges = true;
  await s.exec(`document.querySelector('[data-testid=sync-indicator]').click()`);
  await s.waitFor(`return document.querySelector('[data-testid=sync-indicator]')?.classList.contains('syncing')`, { timeout: 10000 });
  // make sure the engine is really waiting on the hung request
  await eventually(() => log.some((l) => l.includes("HANG")), { message: "hung changes request" });
}

before(async () => {
  fs.mkdirSync(vault, { recursive: true });
  fs.writeFileSync(path.join(vault, "Note.md"), "first\n");
  server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: TOKEN, CAIRN_DATA: path.join(tmp, "server"), CAIRN_ADDR: `127.0.0.1:${SPORT}` },
    stdio: "ignore",
  });
  await eventually(async () => (await fetch(`${SURL}/health`)).ok, { message: "server up" });
  await startProxy();
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });

  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-sync]"));
  await s.findWait("[data-testid=sync-server]");
  await setInput("sync-server", PURL);
  await setInput("sync-token", TOKEN);
  await setInput("sync-vault", "e2e");
  await setInput("sync-device", "laptop");
  await setInput("sync-pass", PASS);
  await setInput("sync-pass2", PASS);
  await s.click(await s.find("[data-testid=sync-connect]"));
  await s.click(await s.findWait("[data-testid=dialog-ok]")); // a new vault: "Create it?"
  await s.waitFor(`return document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle'`, { timeout: 30000 });
  await s.exec(`document.querySelector('[data-testid=settings] .close').click()`);
  await sleep(2500); // let the manager's own first scheduled sync pass
  // a second revision so history has something to list
  fs.writeFileSync(path.join(vault, "Note.md"), "first\nsecond\n");
  await sleep(1500);
  await s.exec(`document.querySelector('[data-testid=sync-indicator]').click()`);
  await s.waitFor(`return document.querySelector('[data-testid=sync-indicator]')?.textContent.trim() === 'Synced'`, { timeout: 20000 });
});

after(async () => {
  hangChanges = false;
  for (const r of held) r.destroy();
  await s?.close();
  drv?.proc.kill();
  server?.kill();
  proxy?.close();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test(
  "Turn off responds promptly while a sync is stuck on a hung server",
  async () => {
    log.length = 0;
    await startHungSync();
    await s.exec(`document.querySelector('[data-testid=open-settings]').click()`);
    await s.click(await s.findWait("[data-testid=settings-sync]"));
    await s.findWait("[data-testid=sync-state]");
    await s.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Turn off').click()`);
    await s.click(await s.findWait("[data-testid=dialog-ok]"));
    const t1 = Date.now();
    const rel = (t) => ((t - t1) / 1000).toFixed(1) + "s";
    let backendOffAt = null;
    let uiOffAt = null;
    const samples = [];
    while (Date.now() - t1 < 150000) {
      const be = await s.exec(`return window.__TAURI_INTERNALS__.invoke('sync_status').then((st) => JSON.stringify({c: st.configured, s: st.state}), (e) => 'ERR ' + e)`);
      const ui = await s.exec(`return JSON.stringify({form: !!document.querySelector('[data-testid=sync-server]'), state: document.querySelector('[data-testid=sync-state]')?.textContent.trim() ?? null, ind: document.querySelector('[data-testid=sync-indicator]')?.textContent.trim() ?? null})`);
      const now = Date.now();
      if (!samples.length || samples[samples.length - 1].be !== be || samples[samples.length - 1].ui !== ui) samples.push({ t: rel(now), be, ui });
      if (backendOffAt === null && be.includes('"c":false')) backendOffAt = now;
      if (uiOffAt === null && ui.includes('"form":true')) uiOffAt = now;
      if (backendOffAt !== null && uiOffAt !== null) break;
      if (backendOffAt !== null && now - backendOffAt > 30000) break; // UI never caught up
      await sleep(1000);
    }
    const events = log.filter((l) => l.includes("HANG") || l.includes("CLOSED")).map((l) => `${rel(Number(l.split(" ")[0]))} ${l.split(" ").slice(1, 4).join(" ")}`);
    const msg =
      `backend off after ${backendOffAt ? rel(backendOffAt) : "never"}; UI shows setup form after ${uiOffAt ? rel(uiOffAt) : "never"}; ` +
      `proxy: ${JSON.stringify(events)}; state changes: ${JSON.stringify(samples)}`;
    console.log(msg);
    assert.ok(backendOffAt && uiOffAt && uiOffAt - t1 < 5000, msg);
  },
);
