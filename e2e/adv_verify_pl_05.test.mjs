// Measurements for FINDING-068 (overlapping plugin starts leave an
// orphan worker). Measures how wide the race window is in the real app, so the
// severity can be judged: how long does the `readConfig` await in
// PluginHost.start() take, and which gaps between two "Reload plugins" clicks
// still produce two workers for one plugin?
//
//   scripts/e2e-headless.sh e2e/adv_verify_pl_05.test.mjs
//
// Evidence goes to e2e/.tmp/PL05-verify/.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";
import { approvePlugins } from "./plugin_approvals.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-pl05-"));
const evidence = path.join(import.meta.dirname, ".tmp/PL05-verify");
const PLUGIN = "t.js";
const WNAME = `cairn-plugin-${PLUGIN}`;

let drv, s, vault;
const results = {};

function makeVault(name, files) {
  const dir = path.join(tmp, name);
  for (const [rel, content] of Object.entries(files)) {
    const p = path.join(dir, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, content);
  }
  return dir;
}

async function escapeAll() {
  for (let i = 0; i < 3; i++) {
    await s.keys(Key.escape);
    await sleep(60);
  }
}

async function installSpy() {
  await s.exec(`
    if (!window.__plSpy) {
      window.__plSpy = true;
      window.__plWorkers = [];
      const W = window.Worker;
      window.Worker = function (url, opts) {
        const w = new W(url, opts);
        w.__name = opts && opts.name;
        const term = w.terminate.bind(w);
        w.terminate = () => { w.__terminated = true; term(); };
        window.__plWorkers.push(w);
        return w;
      };
      window.Worker.prototype = W.prototype;
    }
  `);
}

const alive = () => s.exec(`return window.__plWorkers.filter((w) => w.__name === ${JSON.stringify(WNAME)} && !w.__terminated).length`);
const killAll = () => s.exec(`window.__plWorkers.forEach((w) => { if (String(w.__name).startsWith("cairn-plugin-")) w.terminate(); })`);

/** Back to exactly one running instance, tracked by the host. */
async function reset() {
  await killAll();
  await s.exec(`document.querySelector('[data-testid=plugins-reload]').click()`);
  await sleep(400);
  assert.equal(await alive(), 1, "reset left exactly one worker");
}

before(async () => {
  fs.mkdirSync(evidence, { recursive: true });
  vault = makeVault("v", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [PLUGIN] }),
    ".cairn/plugins/t.js": `// @name T\nsetInterval(() => {}, 1000);\n`,
  });
  approvePlugins(path.join(tmp, "config"), vault, [PLUGIN]); // turned on on this device
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  await installSpy();
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
  await s.findWait("[data-testid=plugins-reload]");
  await reset();
});

after(async () => {
  try {
    fs.writeFileSync(path.join(evidence, "results.json"), JSON.stringify(results, null, 2));
  } catch {}
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("measure: read_config IPC round trip (the await inside PluginHost.start)", async () => {
  const r = await s.execAsync(`
    const done = arguments[arguments.length - 1];
    (async () => {
      const inv = window.__TAURI_INTERNALS__.invoke;
      const xs = [];
      for (let i = 0; i < 60; i++) {
        const t = performance.now();
        await inv("read_config", { name: "plugins/t.js" });
        xs.push(performance.now() - t);
      }
      xs.sort((a, b) => a - b);
      done({ min: xs[0], median: xs[30], p95: xs[57], max: xs[59] });
    })().catch((e) => done({ error: String(e) }));
  `);
  results.readConfigMs = r;
  console.log("read_config round trip (ms):", JSON.stringify(r));
  assert.ok(!r.error, r.error);
});

test("programmatic double click: which gaps between the two clicks orphan a worker", async () => {
  const sweep = {};
  for (const gap of ["sync", 0, 1, 2, 4, 8, 16, 32, 64, 100]) {
    await reset();
    if (gap === "sync") await s.exec(`const b = document.querySelector('[data-testid=plugins-reload]'); b.click(); b.click();`);
    else await s.exec(`const b = document.querySelector('[data-testid=plugins-reload]'); b.click(); setTimeout(() => b.click(), ${gap});`);
    await sleep(600);
    sweep[gap] = await alive();
  }
  await reset();
  results.programmaticSweep = sweep;
  console.log("live workers after two Reload clicks, by gap in ms:", JSON.stringify(sweep));
  assert.ok(Object.values(sweep).every((n) => n === 1), JSON.stringify(sweep));
});

async function pointerDouble(gap) {
  await s.exec(`document.querySelector('[data-testid=plugins-reload]').scrollIntoView({ block: "center" })`);
  const el = await s.find("[data-testid=plugins-reload]");
  await s.pointer([
    { type: "pointerMove", origin: { "element-6066-11e4-a52e-4f735466cecf": el }, x: 0, y: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
    { type: "pause", duration: gap },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
}

test("real pointer double click on Reload at human speed (30, 80, 150 ms) leaves one worker", async () => {
  const out = {};
  for (const gap of [30, 80, 150]) {
    await reset();
    await pointerDouble(gap);
    await sleep(600);
    out[gap] = await alive();
  }
  await reset();
  results.pointerDoubleClickHuman = out;
  console.log("live workers after a real double click, by gap in ms:", JSON.stringify(out));
  assert.ok(Object.values(out).every((n) => n === 1), JSON.stringify(out));
});

test("real pointer double click with no pause between the clicks (faster than a person can click)", async () => {
  await reset();
  await pointerDouble(0);
  await sleep(600);
  const n = await alive();
  results.pointerDoubleClickNoGap = n;
  console.log("live workers after a no-gap double click:", n);
  await reset();
  assert.equal(n, 1);
});

test("ten enabled 200 KB plugins: how wide the window gets", async () => {
  const big = "// padding\n" + ("/* " + "x".repeat(1000) + " */\n").repeat(200);
  const names = Array.from({ length: 10 }, (_, i) => `p${i}.js`);
  const files = { "Note.md": "x\n", ".cairn/settings.json": JSON.stringify({ plugins: names }) };
  for (const n of names) files[`.cairn/plugins/${n}`] = `// @name ${n}\nsetInterval(() => {}, 1000);\n${big}`;
  const dir = makeVault("many", files);
  approvePlugins(path.join(tmp, "config"), dir, names);
  await escapeAll();
  await killAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Switch vault");
  await sleep(150);
  await s.keys(Key.enter);
  await s.type(await s.findWait("[data-testid=vault-path]", 8000), dir);
  await s.click(await s.find("[data-testid=vault-open]"));
  await s.waitFor(`return !document.querySelector('[data-testid=vault-path]') && document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
  await s.findWait("[data-testid=plugins-reload]");
  const live = () => s.exec(`return window.__plWorkers.filter((w) => /^cairn-plugin-p\\d\\.js$/.test(w.__name) && !w.__terminated).length`);
  const one = async () => {
    await killAll();
    await s.exec(`document.querySelector('[data-testid=plugins-reload]').click()`);
    await sleep(800);
    return live();
  };
  // How long one full Reload (stopAll + sync over all ten) takes.
  const t = await s.execAsync(`
    const done = arguments[arguments.length - 1];
    const inv = window.__TAURI_INTERNALS__.invoke;
    (async () => { const t0 = performance.now(); for (let i = 0; i < 10; i++) await inv("read_config", { name: "plugins/p" + i + ".js" }); done(performance.now() - t0); })();
  `);
  results.tenBigReadConfigMs = t;
  const sweep = {};
  for (const gap of [0, 2, 4, 8, 16, 32, 64]) {
    assert.equal(await one(), 10, "reset to ten workers");
    await s.exec(`const b = document.querySelector('[data-testid=plugins-reload]'); b.click(); setTimeout(() => b.click(), ${gap});`);
    await sleep(1200);
    sweep[gap] = (await live()) - 10;
  }
  results.tenBigExtraWorkersByGap = sweep;
  for (const gap of [30, 80, 150]) {
    assert.equal(await one(), 10, "reset to ten workers");
    await pointerDouble(gap);
    await sleep(1200);
    sweep[`pointer${gap}`] = (await live()) - 10;
  }
  await killAll();
  console.log("ten plugins: sequential read_config of all ten (ms):", t, "extra (orphan) workers by gap:", JSON.stringify(sweep));
  assert.ok(Object.values(sweep).every((n) => n === 0), JSON.stringify(sweep));
});

test("one 8 MB plugin (the size of a large bundled plugin): a human-speed double click on Reload", async () => {
  const big = ("/* " + "x".repeat(1000) + " */\n").repeat(8000);
  const dir = makeVault("huge", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: ["h.js"] }),
    ".cairn/plugins/h.js": `// @name H\nsetInterval(() => {}, 1000);\n${big}`,
  });
  approvePlugins(path.join(tmp, "config"), dir, ["h.js"]);
  await escapeAll();
  await killAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Switch vault");
  await sleep(150);
  await s.keys(Key.enter);
  await s.type(await s.findWait("[data-testid=vault-path]", 8000), dir);
  await s.click(await s.find("[data-testid=vault-open]"));
  await s.waitFor(`return !document.querySelector('[data-testid=vault-path]') && document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  await sleep(1500);
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
  await s.findWait("[data-testid=plugins-reload]");
  const live = () => s.exec(`return window.__plWorkers.filter((w) => w.__name === "cairn-plugin-h.js" && !w.__terminated).length`);
  const t = await s.execAsync(`
    const done = arguments[arguments.length - 1];
    const inv = window.__TAURI_INTERNALS__.invoke;
    (async () => { const xs = []; for (let i = 0; i < 5; i++) { const t0 = performance.now(); await inv("read_config", { name: "plugins/h.js" }); xs.push(performance.now() - t0); } done(xs); })();
  `);
  const out = { readConfigMs: t };
  for (const gap of [80, 150, 250]) {
    await killAll();
    await s.exec(`document.querySelector('[data-testid=plugins-reload]').click()`);
    await sleep(2500);
    assert.equal(await live(), 1, "reset to one worker");
    await pointerDouble(gap);
    await sleep(2500);
    out[`pointer${gap}`] = await live();
  }
  await killAll();
  results.hugePlugin = out;
  console.log("8 MB plugin:", JSON.stringify(out));
  assert.ok([80, 150, 250].every((g) => out[`pointer${g}`] === 1), JSON.stringify(out));
});
