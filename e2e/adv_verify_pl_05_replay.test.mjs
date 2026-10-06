// Regression tests for FINDING-068 (overlapping plugin starts left an
// orphan worker). Self-contained: own driver session, own throwaway vaults.
//
//   scripts/e2e-headless.sh e2e/adv_verify_pl_05_replay.test.mjs
//
// Results (worker counts, click-to-worker latency, per-gap outcome) are written
// to e2e/.tmp/PL/verify_pl_05_replay.json.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";
import { approvePlugins } from "./plugin_approvals.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-pl05r-"));
const evidence = path.join(import.meta.dirname, ".tmp/PL");
const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";
const results = {};

let drv, s;
let n = 0;
function makeVault(name, files) {
  const dir = path.join(tmp, `${++n}-${name}`);
  for (const [rel, content] of Object.entries(files)) {
    const p = path.join(dir, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, content);
  }
  fs.mkdirSync(dir, { recursive: true });
  return { dir };
}

async function escapeAll() {
  for (let i = 0; i < 3; i++) {
    await s.keys(Key.escape);
    await sleep(60);
  }
}

async function runCommand(name) {
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), name);
  await sleep(150);
  await s.keys(Key.enter);
}

const killAll = () =>
  s.exec(`(window.__vw || []).forEach((r) => { if (String(r.name).startsWith("cairn-plugin-")) r.w.terminate(); })`);

async function switchTo(v, { kill = true } = {}) {
  await escapeAll();
  if (kill) await killAll();
  await runCommand("Switch vault");
  await s.type(await s.findWait("[data-testid=vault-path]", 8000), v.dir);
  await s.click(await s.find("[data-testid=vault-open]"));
  await s.waitFor(`return !document.querySelector('[data-testid=vault-path]') && document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
}

async function openPluginSettings() {
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
  await s.findWait("[data-testid=plugins-reload]");
}

const toggle = (file) => s.find(`[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`);

// Workers created since index `from` that are not terminated.
const aliveSince = (from) => s.exec(`return window.__vw.slice(${from}).filter((r) => String(r.name).startsWith("cairn-plugin-") && !r.dead).length`);
const workerCount = () => s.exec(`return window.__vw.length`);

const snapshot = (dir) => {
  const d = path.join(dir, "ticks");
  if (!fs.existsSync(d)) return {};
  return Object.fromEntries(fs.readdirSync(d).map((f) => [f, fs.readFileSync(path.join(d, f), "utf8")]));
};

const dblClick = async (pause) => {
  const el = await s.find("[data-testid=plugins-reload]");
  await s.pointer([
    { type: "pointerMove", duration: 0, origin: { [ELEMENT]: el }, x: 0, y: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
    { type: "pause", duration: pause },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
};

before(async () => {
  fs.mkdirSync(evidence, { recursive: true });
  const home = makeVault("home", { "Home.md": "# Home\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }) });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [home.dir]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  // Record every worker: name, creation time, terminated flag, calls it made.
  await s.exec(`
    window.__vw = [];
    const W = window.Worker;
    window.Worker = function (url, opts) {
      const w = new W(url, opts);
      const r = { w, name: opts && opts.name, t: performance.now(), dead: false, calls: 0 };
      const term = w.terminate.bind(w);
      w.terminate = () => { r.dead = true; term(); };
      w.addEventListener("message", (e) => { if (e.data && e.data.type === "call") r.calls++; });
      window.__vw.push(r);
      return w;
    };
    window.Worker.prototype = W.prototype;
  `);
});

after(async () => {
  try {
    fs.writeFileSync(path.join(evidence, "verify_pl_05_replay.json"), JSON.stringify(results, null, 2));
  } catch {}
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("instrumented replay: Reload x2 then disable leaves zero live plugin workers, and nothing reaches the next vault", async () => {
  const ticker = `// @name Ticker
// @permissions write
const id = Math.random().toString(36).slice(2, 8);
let n = 0;
setInterval(() => { cairn.notes.write("ticks/" + id + ".md", String(++n)); }, 250);
`;
  const a = makeVault("orphan-a", { "Note.md": "x\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }), ".cairn/plugins/ticker.js": ticker });
  const b = makeVault("orphan-b", { "Other.md": "y\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }) });
  try {
    await switchTo(a);
    const base = await workerCount();
    await openPluginSettings();
    await s.click(await toggle("ticker.js"));
    await s.click(await s.findWait("[data-testid=dialog-ok]"));
    await sleep(500);
    await s.exec(`const b = document.querySelector('[data-testid=plugins-reload]'); b.click(); b.click();`);
    await sleep(800);
    const createdAfterReload = (await workerCount()) - base;
    const aliveAfterReload = await aliveSince(base);
    await s.click(await toggle("ticker.js")); // disable
    await sleep(600);
    const aliveAfterDisable = await aliveSince(base);
    const s1 = snapshot(a.dir);
    await sleep(1200);
    const s2 = snapshot(a.dir);
    const stillTicking = Object.keys(s2).filter((f) => s1[f] !== s2[f]);
    const settingsA = JSON.parse(fs.readFileSync(path.join(a.dir, ".cairn/settings.json"), "utf8"));
    await switchTo(b, { kill: false });
    await sleep(1500);
    const crossVault = snapshot(b.dir);
    const settingsB = JSON.parse(fs.readFileSync(path.join(b.dir, ".cairn/settings.json"), "utf8"));
    results.replay = { createdAfterReload, aliveAfterReload, aliveAfterDisable, stillTicking, crossVault, settingsAPlugins: settingsA.plugins, settingsBPlugins: settingsB.plugins, aliveAfterVaultSwitch: await aliveSince(base) };
    assert.equal(aliveAfterDisable, 0, JSON.stringify(results.replay));
    assert.deepEqual(Object.keys(crossVault), [], JSON.stringify(results.replay));
  } finally {
    await killAll();
  }
});

test("gap sweep: two Reload clicks dispatched G ms apart in the page (measurement)", async () => {
  const idle = `// @name Idle\nsetInterval(() => {}, 1000);\n`;
  const v = makeVault("sweep", { "Note.md": "x\n", ".cairn/settings.json": JSON.stringify({ plugins: ["idle.js"] }), ".cairn/plugins/idle.js": idle });
  approvePlugins(path.join(tmp, "config"), v.dir, ["idle.js"]); // turned on on this device
  await switchTo(v);
  await openPluginSettings();
  const sweep = [];
  for (const gap of [0, 1, 3, 10, 50]) {
    for (let rep = 0; rep < 3; rep++) {
      await killAll();
      await s.exec(`document.querySelector('[data-testid=plugins-reload]').click()`);
      await sleep(400);
      const mark = await workerCount();
      const t0 = await s.exec(`
        const b = document.querySelector('[data-testid=plugins-reload]');
        const t0 = performance.now();
        b.click();
        if (${gap} === 0) b.click(); else setTimeout(() => b.click(), ${gap});
        return t0;`);
      await sleep(gap + 600);
      const info = await s.exec(`return window.__vw.slice(${mark}).map((r) => ({ dt: Math.round((r.t - ${t0}) * 10) / 10, dead: r.dead }))`);
      sweep.push({ gap, rep, alive: info.filter((r) => !r.dead).length, created: info });
    }
  }
  await killAll();
  results.sweepSummary = Object.fromEntries([...new Set(sweep.map((x) => x.gap))].map((g) => [g, sweep.filter((x) => x.gap === g).map((x) => x.alive)]));
  results.clickToWorkerMs = sweep.map((x) => x.created[0]?.dt);
  assert.ok(sweep.every((x) => x.alive <= 1), JSON.stringify(results.sweepSummary));
});

test("real WebDriver double-click gesture on Reload, idle UI and while the UI thread is busy", async () => {
  const idle = `// @name Idle\nsetInterval(() => {}, 1000);\n`;
  const v = makeVault("dbl", { "Note.md": "x\n", ".cairn/settings.json": JSON.stringify({ plugins: ["idle.js"] }), ".cairn/plugins/idle.js": idle });
  approvePlugins(path.join(tmp, "config"), v.dir, ["idle.js"]); // turned on on this device
  await switchTo(v);
  await openPluginSettings();
  const out = [];
  for (const busy of [false, true]) {
    for (const pause of [30, 80, 150]) {
      for (let rep = 0; rep < 3; rep++) {
        await killAll();
        await s.exec(`document.querySelector('[data-testid=plugins-reload]').click()`);
        await sleep(400);
        const mark = await workerCount();
        // busy: the page's main thread is blocked for 400 ms starting now
        // (as during a long render or GC), so both clicks arrive while it is busy.
        if (busy) await s.exec(`setTimeout(() => { const e = performance.now() + 400; while (performance.now() < e); }, 0)`);
        await dblClick(pause);
        await sleep(900);
        out.push({ busy, pause, rep, alive: await aliveSince(mark), created: (await workerCount()) - mark });
      }
    }
  }
  await killAll();
  results.dblclick = out;
  assert.ok(out.every((x) => x.alive <= 1), JSON.stringify(out));
});
