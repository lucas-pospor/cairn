// Reproduction for FINDING-069, run against the real app, with a
// control in the same vault and the same 32 s window:
//   - once.js:  command "Work" (1.5 s) run once            -> must keep running (control, plain test)
//   - twice.js: command "Work" (1.5 s) run twice, 0.3 s apart -> must keep running (FINDING-069;
//               the defect stops it ~30 s later)
//
//   scripts/e2e-headless.sh e2e/adv_verify_pl_06.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-pl06-"));
let drv, s;
let result = null;

const PLUGIN = (name) => `// @name ${name}
cairn.commands.register("work", "Work", async () => { await new Promise((r) => setTimeout(r, 1500)); postMessage({ type: "probe", k: "done-${name}" }); });
`;

async function eventually(fn, timeout, message) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await fn()) return;
    await sleep(80);
  }
  throw new Error(`timed out: ${message}`);
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

async function enable(file) {
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
  await s.click(await s.findWait(`[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`));
  await sleep(300);
}

const alive = (file) => s.exec(`return window.__plWorkers.filter((w) => w.__name === "cairn-plugin-${file}" && !w.__terminated).length`);
const doneCount = (name) => s.exec(`return window.__plMsgs.filter((m) => m.data && m.data.type === "probe" && m.data.k === "done-${name}").length`);
const hasCommand = async (label) => {
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), label);
  await sleep(300);
  const text = await s.exec(`return document.body.innerText`);
  await escapeAll();
  return text.includes(label);
};

before(async () => {
  const dir = path.join(tmp, "vault");
  fs.mkdirSync(path.join(dir, ".cairn/plugins"), { recursive: true });
  fs.writeFileSync(path.join(dir, "Note.md"), "x\n");
  fs.writeFileSync(path.join(dir, ".cairn/settings.json"), JSON.stringify({ plugins: [] }));
  fs.writeFileSync(path.join(dir, ".cairn/plugins/once.js"), PLUGIN("Once"));
  fs.writeFileSync(path.join(dir, ".cairn/plugins/twice.js"), PLUGIN("Twice"));
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [dir]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  await s.exec(`
    window.__plMsgs = []; window.__plWorkers = [];
    const W = window.Worker;
    window.Worker = function (url, opts) {
      const w = new W(url, opts);
      w.__name = opts && opts.name;
      const term = w.terminate.bind(w);
      w.terminate = () => { w.__terminated = true; term(); };
      w.addEventListener("message", (e) => window.__plMsgs.push({ from: w.__name, data: e.data }));
      window.__plWorkers.push(w);
      return w;
    };
    window.Worker.prototype = W.prototype;
  `);
  await enable("once.js");
  await enable("twice.js");
  await escapeAll();
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("control: one run of a 1.5 s command keeps the plugin running past 30 s", { timeout: 80000 }, async () => {
  assert.equal(await alive("once.js"), 1, "once.js started");
  assert.equal(await alive("twice.js"), 1, "twice.js started");
  await runCommand("Once: Work");
  await sleep(200);
  await runCommand("Twice: Work");
  await sleep(300);
  await runCommand("Twice: Work");
  await eventually(async () => (await doneCount("Once")) >= 1 && (await doneCount("Twice")) >= 2, 8000, "all runs finished");
  await sleep(32000);
  result = {
    onceAlive: await alive("once.js"),
    twiceAlive: await alive("twice.js"),
    toasts: await s.exec(`return [...document.querySelectorAll('.toast')].map((t) => t.textContent)`),
    onceCmd: await hasCommand("Once: Work"),
    twiceCmd: await hasCommand("Twice: Work"),
  };
  console.log("PL-06 verify result:", JSON.stringify(result));
  assert.equal(result.onceAlive, 1, `control plugin was stopped: ${JSON.stringify(result)}`);
  assert.ok(result.onceCmd, "control command still listed");
});

test("FINDING-069: two overlapping runs of the same quick command do not stop the plugin", async () => {
  assert.ok(result, "control test ran");
  assert.equal(result.twiceAlive, 1, `plugin was stopped: ${JSON.stringify(result)}`);
  assert.ok(result.twiceCmd, "command still listed");
});
