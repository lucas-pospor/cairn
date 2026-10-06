// FINDING-146 (app level): a running sync could not be cancelled. With the
// defect, SyncManager::disconnect() and with_engine() (version history) waited
// on `running`, which sync_now() held for the whole engine.sync(); the engine
// had no cancellation, and HttpTransport's timeout is 60 s per request. The
// test checks that both actions below respond within 5 s.
//
// Flow, through the real UI: connect sync through a small proxy in front of a
// real cairn-server. Then make the proxy stop answering the changes feed (a
// server or network that hangs), click the sync indicator, and while the
// status bar says "Syncing…":
//   1. open Version history for a note (the history endpoint itself still
//      answers immediately through the proxy) and time it;
//   2. trigger another hung sync, press Settings > Sync > Turn off > OK and
//      time how long until the app shows sync as off.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_sr_18.test.mjs   (about 2.5 min)

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

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-sr18-"));
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
  "version history and Turn off respond while a sync is stuck on a hung server",
  async () => {
    // ---- 1. version history during a hung sync
    await s.click(await s.find('[data-testid=tree-row][data-path="Note.md"]'));
    await s.findWait(".cm-content");
    await startHungSync();
    const t0 = Date.now();
    await s.keys({ chord: [Key.ctrl, "p"] });
    await s.type(await s.findWait("[data-testid=palette-input]"), "version history");
    await sleep(150);
    await s.keys(Key.enter);
    await s.waitFor(`return document.querySelectorAll('[data-testid=history-entry]').length >= 1`, { timeout: 90000 });
    const historyWait = Date.now() - t0;
    const indicatorAfterHistory = await indicator();
    console.log(`phase 1: version history opened after ${(historyWait / 1000).toFixed(1)} s (indicator then: ${JSON.stringify(indicatorAfterHistory)})`);
    await s.exec(`document.querySelector('[data-testid=history] button[title=Close]')?.click()`);
    await s.waitFor(`return !document.querySelector('[data-testid=history]')`, { timeout: 5000 });
    // history did not wait for the hung sync; release it and let it end
    hangChanges = false;
    for (const r of held) r.destroy();
    held.clear();
    await s.waitFor(`return !document.querySelector('[data-testid=sync-indicator]')?.classList.contains('syncing')`, { timeout: 90000 });

    // ---- 2. Turn off during a hung sync
    log.length = 0;
    await startHungSync();
    await s.exec(`document.querySelector('[data-testid=open-settings]').click()`);
    await s.click(await s.findWait("[data-testid=settings-sync]"));
    await s.findWait("[data-testid=sync-state]");
    await s.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Turn off').click()`);
    await s.click(await s.findWait("[data-testid=dialog-ok]"));
    const t1 = Date.now();
    await s.waitFor(`return !!document.querySelector('[data-testid=sync-server]')`, { timeout: 90000 });
    const disconnectWait = Date.now() - t1;
    const stillConnectedAt5s = disconnectWait > 5000;

    const msg =
      `version history opened after ${(historyWait / 1000).toFixed(1)} s (indicator then: ${JSON.stringify(indicatorAfterHistory)}); ` +
      `Turn off took ${(disconnectWait / 1000).toFixed(1)} s (still shown as connected 5 s after OK: ${stillConnectedAt5s})`;
    console.log(msg);
    assert.ok(historyWait < 5000 && disconnectWait < 5000, msg);
  },
);
