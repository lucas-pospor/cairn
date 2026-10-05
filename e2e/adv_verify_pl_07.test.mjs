// Reproduction for FINDING-070 against the real app: a command timer left
// over from a stopped plugin instance stops the next instance of the same
// plugin file 30 s after the original run.
//
//   scripts/e2e-headless.sh e2e/adv_verify_pl_07.test.mjs      (about 90 s)
//
// Test 1: a plugin command hangs; the user presses "Reload plugins" in
// Settings > Plugins to recover. With the defect, the fresh instance works at
// first, then is terminated (and its commands removed) when the old run's
// timer fires.
// Test 2: a plugin command hangs; the user disables the plugin, then enables
// it again within 30 s.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-pl07-"));
const evidence = path.join(import.meta.dirname, ".tmp/PL07-verify");
let drv, s;
let n = 0;
const results = {};

function makeVault(name, files) {
  const dir = path.join(tmp, `${++n}-${name}`);
  for (const [rel, c] of Object.entries(files)) {
    const p = path.join(dir, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, c);
  }
  return { dir };
}

async function eventually(fn, { timeout = 6000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      const r = await fn();
      if (r) return r;
    } catch {}
    await sleep(100);
  }
  throw new Error(`timed out: ${message}`);
}

async function installSpy() {
  await s.exec(`
    if (!window.__plSpy) {
      window.__plSpy = true;
      window.__plMsgs = [];
      window.__plWorkers = [];
      window.__toastLog = [];
      const W = window.Worker;
      window.Worker = function (url, opts) {
        const w = new W(url, opts);
        w.__name = opts && opts.name;
        w.__createdAt = Date.now();
        const term = w.terminate.bind(w);
        w.terminate = () => { if (!w.__terminated) { w.__terminated = true; w.__terminatedAt = Date.now(); } term(); };
        w.addEventListener("message", (e) => window.__plMsgs.push({ from: w.__name, data: e.data, at: Date.now() }));
        window.__plWorkers.push(w);
        return w;
      };
      window.Worker.prototype = W.prototype;
      new MutationObserver((ms) => {
        for (const m of ms) for (const node of m.addedNodes)
          if (node.nodeType === 1 && node.classList.contains("toast")) window.__toastLog.push({ text: node.textContent, at: Date.now() });
      }).observe(document.body, { childList: true, subtree: true });
    }
  `);
}

const workers = (name) =>
  s.exec(`return window.__plWorkers.filter((w) => w.__name === ${JSON.stringify(name)}).map((w) => ({ createdAt: w.__createdAt, terminatedAt: w.__terminatedAt || null }))`);
const probes = (k) => s.exec(`return window.__plMsgs.filter((m) => m.data && m.data.type === "probe" && m.data.k === ${JSON.stringify(k)}).map((m) => m.data)`);
const toastLog = () => s.exec(`return window.__toastLog`);
const now = () => s.exec(`return Date.now()`);

async function escapeAll() {
  for (let i = 0; i < 3; i++) {
    await s.keys(Key.escape);
    await sleep(60);
  }
}

async function runCommand(name) {
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), name);
  await sleep(200);
  await s.keys(Key.enter);
  await sleep(150);
}

async function paletteItems(filter) {
  await escapeAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), filter);
  await sleep(250);
  const items = await s.exec(`return [...document.querySelectorAll('[data-testid=palette-item]')].map((e) => e.textContent.trim())`);
  await escapeAll();
  return items.filter((t) => t.includes(filter));
}

async function switchTo(v) {
  await escapeAll();
  await runCommand("Switch vault");
  const input = await s.findWait("[data-testid=vault-path]", 8000);
  await s.type(input, v.dir);
  await s.click(await s.find("[data-testid=vault-open]"));
  await s.waitFor(`return !document.querySelector('[data-testid=vault-path]') && document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
}

async function openPluginSettings() {
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
}

async function togglePlugin(file) {
  await openPluginSettings();
  await s.click(await s.findWait(`[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`));
  await sleep(400);
}

async function shot(name) {
  try {
    fs.writeFileSync(path.join(evidence, name), await s.screenshot());
  } catch {}
}

const plugin = (name) => `// @name ${name}
cairn.commands.register("hang", "Hang", () => new Promise(() => {}));
cairn.commands.register("ping", "Ping", async () => { postMessage({ type: "probe", k: ${JSON.stringify(name)}, v: Date.now() }); });
`;

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
  await installSpy();
});

after(async () => {
  try {
    fs.writeFileSync(path.join(evidence, "results.json"), JSON.stringify(results, null, 2));
  } catch {}
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("'Reload plugins' after a hung command leaves the reloaded plugin running", { timeout: 90000 }, async () => {
  const v = makeVault("reload", { "Note.md": "x\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }), ".cairn/plugins/stuck.js": plugin("Stuck") });
  await switchTo(v);
  await togglePlugin("stuck.js"); // enable (no permissions, so no dialog)
  await escapeAll();
  await runCommand("Stuck: Hang");
  const t0 = await now();
  await sleep(3000);
  // The user notices the command is stuck and reloads plugins.
  await openPluginSettings();
  await s.click(await s.findWait("[data-testid=plugins-reload]"));
  await sleep(500);
  await escapeAll();
  await runCommand("Stuck: Ping");
  await eventually(async () => (await probes("Stuck")).length >= 1, { message: "reloaded instance answers Ping" });
  const before = await workers("cairn-plugin-stuck.js");
  // Wait until well past 30 s after the original run.
  const left = t0 + 33000 - (await now());
  if (left > 0) await sleep(left);
  const after = await workers("cairn-plugin-stuck.js");
  const items = await paletteItems("Stuck:");
  const toasts = await toastLog();
  await shot("PL-07-reload.png");
  results.reload = { t0, before, after, items, toasts };
  console.log("reload scenario:", JSON.stringify(results.reload));
  assert.equal(before.length, 2, "one worker before Reload, one after");
  assert.ok(before[0].terminatedAt && !before[1].terminatedAt, "Reload stopped the old worker and left the new one running");
  assert.equal(after[1].terminatedAt, null, `reloaded instance was terminated ${after[1].terminatedAt - t0} ms after the original run; toasts: ${JSON.stringify(toasts)}`);
  assert.deepEqual(items.sort(), ["Stuck: Hang", "Stuck: Ping"]);
});

test("disabling and re-enabling a plugin within 30 s of a hung run leaves the new instance running", { timeout: 90000 }, async () => {
  const v = makeVault("toggle", { "Note.md": "x\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }), ".cairn/plugins/toggle.js": plugin("Toggle") });
  await switchTo(v);
  await s.exec(`window.__toastLog.length = 0`);
  await togglePlugin("toggle.js"); // enable
  await escapeAll();
  await runCommand("Toggle: Hang");
  const t0 = await now();
  await sleep(2000);
  await togglePlugin("toggle.js"); // disable
  await togglePlugin("toggle.js"); // enable again
  await escapeAll();
  await runCommand("Toggle: Ping");
  await eventually(async () => (await probes("Toggle")).length >= 1, { message: "re-enabled instance answers Ping" });
  const left = t0 + 33000 - (await now());
  if (left > 0) await sleep(left);
  const after = await workers("cairn-plugin-toggle.js");
  const items = await paletteItems("Toggle:");
  const toasts = await toastLog();
  await shot("PL-07-toggle.png");
  results.toggle = { t0, after, items, toasts };
  console.log("toggle scenario:", JSON.stringify(results.toggle));
  assert.equal(after.length, 2);
  assert.equal(after[1].terminatedAt, null, `re-enabled instance was terminated ${after[1].terminatedAt - t0} ms after the original run; toasts: ${JSON.stringify(toasts)}`);
  assert.deepEqual(items.sort(), ["Toggle: Hang", "Toggle: Ping"]);
});
