// Plugin resource abuse in the real app.
//
//   systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0 --quiet \
//     scripts/e2e-headless.sh e2e/adv_plugins_abuse.test.mjs
//   PA_SLOW=1 ...   adds the 30-35 s watchdog tests
//   PA_OOM=1  ...   adds the out-of-memory test (ALWAYS run inside the
//                   memory-capped scope above, or it can exhaust the machine)
//
// Every test starts its own app session on its own throwaway vault, so a hung
// or crashed page cannot break the next test. Measurements (event-loop lag,
// frame gaps, web-process CPU and RSS, wall-clock times) are written to
// e2e/.tmp/PA/measurements.json. The test marked { todo: "FINDING-161: ..." }
// reproduces a known limit (docs/PLAN.md section 9): it fails without failing
// the run.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";
import { approvePlugins } from "./plugin_approvals.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const SLOW = !!process.env.PA_SLOW;
const OOM = !!process.env.PA_OOM;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-pa-"));
const evidence = path.join(import.meta.dirname, ".tmp/PA");
const MEAS = path.join(evidence, "measurements.json");
let drv;
let vaultCount = 0;

function record(key, value) {
  let all = {};
  try {
    all = JSON.parse(fs.readFileSync(MEAS, "utf8"));
  } catch {}
  all[key] = { ...value, at: new Date().toISOString() };
  fs.writeFileSync(MEAS, JSON.stringify(all, null, 2));
  console.log(`[PA] ${key}: ${JSON.stringify(value)}`);
}

function makeVault(name, files) {
  const dir = path.join(tmp, `${++vaultCount}-${name}`);
  for (const [rel, content] of Object.entries(files)) {
    const p = path.join(dir, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, content);
  }
  return { dir, path: (rel) => path.join(dir, rel) };
}

// ---------- processes ----------

function procTable() {
  const out = execFileSync("ps", ["-e", "-o", "pid=,ppid=,comm="], { encoding: "utf8" });
  return out
    .trim()
    .split("\n")
    .map((l) => l.trim().split(/\s+/))
    .map(([pid, ppid, comm]) => ({ pid: +pid, ppid: +ppid, comm }));
}

function descendants(root) {
  const t = procTable();
  const out = [];
  const walk = (p) => {
    for (const c of t.filter((x) => x.ppid === p)) {
      out.push(c);
      walk(c.pid);
    }
  };
  walk(root);
  return out;
}

const webProcs = () => descendants(drv.proc.pid).filter((p) => p.comm.startsWith("WebKitWebProces"));
const appProcs = () => descendants(drv.proc.pid).filter((p) => p.comm === "cairn");

function cpuTicks(pid) {
  try {
    const f = fs.readFileSync(`/proc/${pid}/stat`, "utf8").split(") ")[1].split(" ");
    return +f[11] + +f[12]; // utime + stime (fields 14, 15)
  } catch {
    return null;
  }
}

function rssMb(pid) {
  try {
    const m = /VmRSS:\s+(\d+)/.exec(fs.readFileSync(`/proc/${pid}/status`, "utf8"));
    return m ? Math.round(+m[1] / 1024) : null;
  } catch {
    return null;
  }
}

/** Percent of one core used by the web process over `ms`. */
async function webCpu(ms = 3000) {
  const pids = webProcs().map((p) => p.pid);
  const a = pids.map(cpuTicks);
  await sleep(ms);
  const b = pids.map(cpuTicks);
  let ticks = 0;
  pids.forEach((_, i) => (ticks += (b[i] ?? 0) - (a[i] ?? 0)));
  return Math.round((ticks / 100 / (ms / 1000)) * 100);
}

const webRss = () => webProcs().reduce((s, p) => s + (rssMb(p.pid) ?? 0), 0);

// ---------- WebDriver with timeouts ----------

async function execT(s, script, ms = 5000) {
  const t0 = performance.now();
  try {
    const r = await fetch(`${s.base}/session/${s.id}/execute/sync`, {
      method: "POST",
      body: JSON.stringify({ script, args: [] }),
      headers: { "content-type": "application/json" },
      signal: AbortSignal.timeout(ms),
    });
    const j = await r.json().catch(() => ({}));
    return { ok: r.ok, value: j.value, ms: Math.round(performance.now() - t0) };
  } catch (e) {
    return { ok: false, error: String(e), ms: Math.round(performance.now() - t0) };
  }
}

async function withTimeout(p, ms, what) {
  let t;
  return Promise.race([p, new Promise((_, rej) => (t = setTimeout(() => rej(new Error(`timeout: ${what}`)), ms)))]).finally(() => clearTimeout(t));
}

const LAG_MONITOR = `
  if (!window.__lag) {
    window.__lag = { max: 0, maxRaf: 0, ticks: 0, frames: 0 };
    let last = performance.now();
    const tick = () => { const n = performance.now(); const d = n - last - 10; if (d > __lag.max) __lag.max = d; last = n; __lag.ticks++; setTimeout(tick, 10); };
    setTimeout(tick, 10);
    let lr = performance.now();
    const raf = (n) => { const d = n - lr; if (d > __lag.maxRaf) __lag.maxRaf = d; lr = n; __lag.frames++; requestAnimationFrame(raf); };
    requestAnimationFrame(raf);
  }
`;
const resetLag = (s) => execT(s, `window.__lag.max = 0; window.__lag.maxRaf = 0; window.__lag.ticks = 0; window.__lag.frames = 0; return true`);
const readLag = async (s, ms = 60000) => {
  const r = await execT(s, `return { max: Math.round(__lag.max), maxRaf: Math.round(__lag.maxRaf), ticks: __lag.ticks, frames: __lag.frames }`, ms);
  return r.ok ? { ...r.value, readMs: r.ms } : { error: r.error, readMs: r.ms };
};

const SPY = `
  if (!window.__plSpy) {
    window.__plSpy = true;
    window.__plMsgs = [];
    window.__plMsgCount = 0;
    window.__plWorkers = [];
    const W = window.Worker;
    window.Worker = function (url, opts) {
      const w = new W(url, opts);
      w.__name = opts && opts.name;
      const term = w.terminate.bind(w);
      w.terminate = () => { w.__terminated = true; term(); };
      w.addEventListener("message", (e) => { window.__plMsgCount++; if (e.data && e.data.type === "probe") window.__plMsgs.push({ from: w.__name, data: e.data }); });
      window.__plWorkers.push(w);
      return w;
    };
    window.Worker.prototype = W.prototype;
  }
`;

const probes = async (s, k) => {
  const r = await execT(s, `return window.__plMsgs.filter((m) => m.data.k === ${JSON.stringify(k)}).map((m) => m.data)`, 30000);
  return r.ok ? r.value : [];
};
async function waitProbe(s, k, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const p = await probes(s, k);
    if (p.length) return p[0];
    await sleep(100);
  }
  throw new Error(`timed out waiting for probe ${k}`);
}
const liveWorkers = async (s, file) => (await execT(s, `return window.__plWorkers.filter((w) => w.__name === "cairn-plugin-${file}" && !w.__terminated).length`)).value;
const toastTexts = async (s) => (await execT(s, `return [...document.querySelectorAll('.toast')].map((t) => t.textContent)`, 20000)).value ?? [];

async function escapeAll(s) {
  for (let i = 0; i < 3; i++) {
    await s.keys(Key.escape);
    await sleep(60);
  }
}

async function openPluginSettings(s) {
  await escapeAll(s);
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
}

async function enablePlugin(s, file, { expectDialog = false } = {}) {
  await openPluginSettings(s);
  await s.click(await s.findWait(`[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`));
  if (expectDialog) await s.click(await s.findWait("[data-testid=dialog-ok]"));
  await sleep(300);
}

async function runCommand(s, name) {
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), name);
  await sleep(150);
  await s.keys(Key.enter);
}

async function shot(s, name) {
  try {
    fs.writeFileSync(path.join(evidence, name), await withTimeout(s.screenshot(), 10000, "screenshot"));
  } catch {}
}

/** One app session on a fresh vault; always cleaned up, even when the page hangs or dies. */
async function withApp(name, files, fn) {
  const v = makeVault(name, { "Home.md": "# Home\n\nsome text\n", ...files });
  // The plugins its settings.json lists were turned on on this device.
  approvePlugins(path.join(tmp, "config"), v.dir, JSON.parse(files[".cairn/settings.json"] ?? "{}").plugins ?? []);
  const s = await Session.create(drv.port, APP, [v.dir]);
  const pids = [];
  try {
    await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 20000 });
    for (const p of [...appProcs(), ...webProcs()]) pids.push(p.pid);
    await execT(s, SPY + LAG_MONITOR + "return true");
    return await fn(s, v);
  } finally {
    await execT(s, `(window.__plWorkers || []).forEach((w) => w.terminate()); return true`, 3000);
    await withTimeout(s.close(), 15000, "session close").catch(() => {});
    await sleep(300);
    for (const pid of pids) {
      try {
        process.kill(pid, 0);
        process.kill(pid, "SIGKILL");
        console.log(`[PA] killed leftover pid ${pid}`);
      } catch {}
    }
  }
}

before(async () => {
  fs.mkdirSync(evidence, { recursive: true });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
});

after(async () => {
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

const settings = (plugins = []) => JSON.stringify({ plugins });

// ---------------------------------------------------------------------------
// Busy loop at load time
// ---------------------------------------------------------------------------

test("a plugin stuck in a busy loop at load: the app opens, stays responsive, and the plugin can be switched off", async () => {
  await withApp("busy-load", {
    ".cairn/settings.json": settings(["busy.js"]),
    ".cairn/plugins/busy.js": `// @name Busy\nwhile (true) {}\n`,
  }, async (s) => {
    await sleep(1000);
    await resetLag(s);
    const cpuOn = await webCpu(3000);
    const lag = await readLag(s);
    const t0 = performance.now();
    await openPluginSettings(s);
    const settingsMs = Math.round(performance.now() - t0);
    await s.click(await s.findWait(`[data-testid=plugin-row][data-file="busy.js"] [data-testid=plugin-toggle]`));
    await sleep(500);
    const cpuOff = await webCpu(3000);
    record("busy-load", { cpuOnPct: cpuOn, cpuOffPct: cpuOff, lagWhileBusy: lag, openSettingsMs: settingsMs });
    assert.ok(lag.max < 250, `event loop lag ${lag.max} ms`);
    assert.ok(cpuOn >= 80, `the busy worker should burn a core (measured ${cpuOn}%)`);
    assert.ok(cpuOff < 40, `CPU after disabling: ${cpuOff}%`);
  });
});

test("a plugin stuck in a busy loop at load is stopped, or reported to the user, within 35 s", { skip: SLOW ? false : "slow (40 s): set PA_SLOW=1", timeout: 90000 }, async () => {
  await withApp("busy-load-watchdog", {
    ".cairn/settings.json": settings(["spin.js"]),
    ".cairn/plugins/spin.js": `// @name Spinner\n// @description Looks harmless.\nconst words = [];\nfor (;;) { words.push(Math.random()); if (words.length > 1000) words.length = 0; }\n`,
  }, async (s) => {
    await sleep(35000);
    const cpu = await webCpu(3000);
    const toasts = await toastTexts(s);
    await shot(s, "PA-01-after-35s.png");
    record("busy-load-35s", { cpuPctAfter35s: cpu, toasts });
    assert.ok(cpu < 40 || toasts.some((t) => /Spinner/.test(t)), `after 35 s the plugin still uses ${cpu}% CPU and nothing was shown (toasts: ${JSON.stringify(toasts)})`);
  });
});

// ---------------------------------------------------------------------------
// Busy loop inside a command (30 s limit)
// ---------------------------------------------------------------------------

test("a command stuck in a busy loop is terminated after about 30 s: CPU drops, commands go, the user is told", { skip: SLOW ? false : "slow (40 s): set PA_SLOW=1", timeout: 90000 }, async () => {
  await withApp("busy-cmd", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/loop.js": `// @name Looper\ncairn.commands.register("spin", "Spin", () => { for (;;) {} });\ncairn.commands.register("ping", "Ping", () => postMessage({ type: "probe", k: "ping" }));\n`,
  }, async (s) => {
    await enablePlugin(s, "loop.js");
    await escapeAll(s);
    await runCommand(s, "Looper: Spin");
    const t0 = Date.now();
    await sleep(1500);
    await resetLag(s);
    const cpuDuring = await webCpu(3000);
    const lag = await readLag(s);
    // Another command of the same plugin while it spins: never runs.
    await runCommand(s, "Looper: Ping");
    await sleep(1000);
    const toastsDuring = await toastTexts(s);
    await shot(s, "PA-busy-cmd-during.png");
    await s.waitFor(`return [...document.querySelectorAll('.toast')].some((t) => t.textContent.includes('took too long'))`, { timeout: 40000 });
    const stoppedAfter = Date.now() - t0;
    const toastsAfter = await toastTexts(s);
    await sleep(500);
    const cpuAfter = await webCpu(3000);
    const live = await liveWorkers(s, "loop.js");
    const pings = (await probes(s, "ping")).length;
    record("busy-cmd", { cpuDuring, lagDuring: lag, toastsDuring, stoppedAfterMs: stoppedAfter, toastsAfter, cpuAfter, liveWorkers: live, pings });
    assert.ok(cpuDuring >= 80, `${cpuDuring}%`);
    assert.ok(lag.max < 250, `lag ${lag.max}`);
    assert.ok(stoppedAfter > 25000 && stoppedAfter < 36000, `${stoppedAfter}`);
    assert.ok(cpuAfter < 40, `CPU after stop ${cpuAfter}%`);
    assert.equal(live, 0);
  });
});

test("while a plugin command runs, the user can see that it is still working", { skip: SLOW ? false : "slow (10 s, part of the slow set): set PA_SLOW=1", timeout: 60000 }, async () => {
  await withApp("hang-ui", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/hang.js": `// @name Hanger\ncairn.commands.register("hang", "Hang", () => new Promise(() => {}));\n`,
  }, async (s) => {
    await enablePlugin(s, "hang.js");
    await escapeAll(s);
    await runCommand(s, "Hanger: Hang");
    // 10 s: long after an ordinary toast (3.5 s) shown at 2 s would have gone.
    await sleep(10000);
    const ui = (await execT(s, `return { toasts: [...document.querySelectorAll('.toast')].map((t) => t.textContent), busy: document.querySelectorAll('[aria-busy=true], .spinner, progress').length, body: document.body.innerText.includes('Hanger') }`)).value;
    await shot(s, "PA-02-hang-10s.png");
    record("hang-ui-10s", ui);
    assert.ok(ui.busy > 0 || ui.toasts.some((t) => /Hanger/.test(t)) || ui.body, `nothing tells the user the command is running: ${JSON.stringify(ui)}`);
  });
});

// ---------------------------------------------------------------------------
// Toast spam
// ---------------------------------------------------------------------------

test("1,000 toasts from one plugin: measured cost, and toasts are capped", async () => {
  await withApp("toast-1000", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/spam.js": `// @name Spam\ncairn.commands.register("spam", "Spam", async () => { const t0 = Date.now(); await Promise.all(Array.from({ length: 1000 }, (_, i) => cairn.ui.toast("spam message number " + i))); postMessage({ type: "probe", k: "spammed", v: Date.now() - t0 }); });\n`,
  }, async (s) => {
    await enablePlugin(s, "spam.js");
    await escapeAll(s);
    const rss0 = webRss();
    await resetLag(s);
    const t0 = performance.now();
    await runCommand(s, "Spam: Spam");
    const p = await waitProbe(s, "spammed", 30000);
    // At most a few are on screen at once (FINDING-157): measure while they are showing.
    await s.waitFor(`return document.querySelectorAll('.toast').length >= 1`, { timeout: 30000 });
    const renderMs = Math.round(performance.now() - t0);
    const lag = await readLag(s);
    const geo = (await execT(s, `
      const ts = [...document.querySelectorAll('.toast')];
      const col = document.querySelector('.toasts').getBoundingClientRect();
      const hit = (x, y) => { const e = document.elementFromPoint(x, y); return e ? (e.closest('.toast') ? 'toast' : e.tagName + '.' + e.className) : null; };
      return { count: ts.length, column: { top: Math.round(col.top), left: Math.round(col.left), bottom: Math.round(col.bottom), width: Math.round(col.width) }, innerHeight, hitTopRight: hit(innerWidth - 100, 60), hitMidRight: hit(innerWidth - 100, innerHeight / 2) };
    `)).value;
    const rss1 = webRss();
    await shot(s, "PA-03-1000-toasts.png");
    // Can the user still open Settings (the gear is outside the column) while they are up?
    const t1 = performance.now();
    await escapeAll(s);
    await s.keys({ chord: [Key.ctrl, ","] });
    await s.findWait("[data-testid=settings-plugins]", 10000);
    const settingsMs = Math.round(performance.now() - t1);
    const toggleHit = (await execT(s, `const b = document.querySelector('[data-testid=settings-plugins]').getBoundingClientRect(); const c = document.querySelector('.modal, [role=dialog]'); const r = c ? c.getBoundingClientRect() : null; const e = r && document.elementFromPoint(r.right - 40, r.top + 120); return { dialogRight: r && Math.round(r.right), coveredAtDialogRight: !!(e && e.closest('.toast')) };`)).value;
    await sleep(4000);
    const left = (await execT(s, `return document.querySelectorAll('.toast').length`)).value;
    record("toast-1000", { hostAckMs: p.v, renderMs, lag, geo, rssBeforeMb: rss0, rssAfterMb: rss1, openSettingsMs: settingsMs, toggleHit, toastsLeftAfter4s: left });
    // At most 5, in a column no higher than half the window (plus its padding).
    assert.ok(geo.count >= 1 && geo.count <= 5 && geo.column.top >= geo.innerHeight / 2 - 60, `${geo.count} toasts on screen at once; column top at ${geo.column.top}px (window height ${geo.innerHeight})`);
  });
});

test("a plugin that toasts continuously can still be switched off with the mouse", async () => {
  await withApp("toast-stream", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/stream.js": `// @name Stream\nlet n = 0; setInterval(() => cairn.ui.toast("Sync helper is still checking your notes for changes, please wait (" + (++n) + ")"), 20);\n`,
  }, async (s, v) => {
    await enablePlugin(s, "stream.js");
    await sleep(3000);
    const sel = `[data-testid=plugin-row][data-file="stream.js"] [data-testid=plugin-toggle]`;
    const geo = (await execT(s, `
      const t = document.querySelector('${sel}').getBoundingClientRect();
      const x = t.left + t.width / 2, y = t.top + t.height / 2;
      const e = document.elementFromPoint(x, y);
      const col = document.querySelector('.toasts').getBoundingClientRect();
      const stack = document.elementsFromPoint(x, y).slice(0, 4).map((el) => el.tagName + '.' + String(el.className).slice(0, 30));
      // A click on a toast while Settings is open goes to what is underneath.
      const r0 = document.querySelector('.toast')?.getBoundingClientRect();
      const under = r0 && document.elementFromPoint(r0.left + r0.width / 2, r0.top + r0.height / 2);
      const throughToast = r0 ? !(under && under.closest('.toast')) : null;
      return { column: { top: Math.round(col.top), left: Math.round(col.left), right: Math.round(col.right) }, stack, toggle: { x: Math.round(x), y: Math.round(y) }, topElement: e ? (e.closest('.toast') ? 'toast: ' + e.textContent : e.outerHTML.slice(0, 80)) : null, toasts: document.querySelectorAll('.toast').length, checked: document.querySelector('${sel}').checked, throughToast };
    `)).value;
    await shot(s, "PA-04-toasts-over-settings.png");
    const clickError = null;
    // A real mouse click at the switch (pointer actions do not check what is on top).
    await s.pointer([
      { type: "pointerMove", x: geo.toggle.x, y: geo.toggle.y, duration: 0 },
      { type: "pointerDown", button: 0 },
      { type: "pointerUp", button: 0 },
    ]);
    await sleep(500);
    const after = (await execT(s, `return document.querySelector('${sel}').checked`)).value;
    const settingsFile = JSON.parse(fs.readFileSync(v.path(".cairn/settings.json"), "utf8"));
    // Keyboard is the escape hatch: the switch still has focus from enabling it.
    const focused = (await execT(s, `return document.activeElement === document.querySelector('${sel}')`)).value;
    record("toast-stream", { geo, webdriverClickError: clickError, checkedAfterMouseClick: after, settingsPluginsAfterClick: settingsFile.plugins, switchHasFocus: focused });
    assert.equal(after, false, `the switch is still on after clicking it; on top of it: ${geo.topElement}; WebDriver: ${clickError}`);
    assert.equal(geo.throughToast, true, "a click on a toast over the Settings dialog does not reach the dialog");
  });
});

// ---------------------------------------------------------------------------
// 10,000 commands
// ---------------------------------------------------------------------------

async function paletteTiming(s, query) {
  await escapeAll(s);
  await resetLag(s);
  const t0 = performance.now();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.waitFor(`return !!document.querySelector('[data-testid=palette-input]') && document.querySelectorAll('[data-testid=palette-item]').length > 0`, { timeout: 60000 });
  const openMs = Math.round(performance.now() - t0);
  const items = (await execT(s, `return document.querySelectorAll('[data-testid=palette-item]').length`, 60000)).value;
  let filterMs = null;
  if (query) {
    const t1 = performance.now();
    await s.type(await s.find("[data-testid=palette-input]"), query);
    await s.waitFor(`const i = document.querySelector('[data-testid=palette-item]'); return i && i.textContent.includes(${JSON.stringify(query)})`, { timeout: 60000 });
    filterMs = Math.round(performance.now() - t1);
  }
  const t2 = performance.now();
  await s.keys(Key.escape);
  await s.waitFor(`return !document.querySelector('[data-testid=palette-input]')`, { timeout: 60000 });
  const closeMs = Math.round(performance.now() - t2);
  const lag = await readLag(s);
  return { openMs, items, filterMs, closeMs, lagMax: lag.max };
}

async function typingLatency(s) {
  await escapeAll(s);
  const row = await s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].find((r) => r.textContent.includes('Home'))`);
  await s.click(Object.values(row)[0]);
  await s.click(await s.findWait(".cm-content"));
  const t0 = performance.now();
  await s.keys("abcdefghij");
  await s.waitFor(`return document.querySelector('.cm-content').textContent.includes('abcdefghij')`, { timeout: 30000 });
  return Math.round(performance.now() - t0);
}

test("10,000 plugin commands: the command palette opens and filters quickly", async () => {
  await withApp("cmds-10k", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/many.js": `// @name Many\nfor (let i = 0; i < 10000; i++) cairn.commands.register("c" + i, "Command number " + i, () => {});\npostMessage({ type: "probe", k: "registered" });\n`,
  }, async (s) => {
    const base = await paletteTiming(s, "Toggle");
    const typeBase = await typingLatency(s);
    await resetLag(s);
    const t0 = performance.now();
    await enablePlugin(s, "many.js");
    await waitProbe(s, "registered", 30000);
    await sleep(1500);
    const regMs = Math.round(performance.now() - t0);
    const regLag = await readLag(s);
    const rssBefore = webRss();
    // A plugin gets at most 200 commands (FINDING-159), so filter for the last one it has.
    const many = await paletteTiming(s, "Command number 199");
    const many2 = await paletteTiming(s, null);
    const rssAfter = webRss();
    const typeMany = await typingLatency(s);
    await shot(s, "PA-05-palette.png");
    record("cmds-10k", { baseline: base, with10k: many, with10kSecondOpen: many2, registerMs: regMs, registerLag: regLag, typingBaselineMs: typeBase, typing10kMs: typeMany, rssBeforeMb: rssBefore, rssAfterPaletteMb: rssAfter });
    assert.ok(many.openMs < 1000 && many.lagMax < 500, `palette open ${many.openMs} ms, main thread blocked ${many.lagMax} ms, ${many.items} items rendered`);
  });
});

// ---------------------------------------------------------------------------
// Registering commands in a loop
// ---------------------------------------------------------------------------

test("a plugin that keeps re-registering commands (5,000/s) does not make the app unusable", async () => {
  await withApp("rereg-steady", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/rereg.js": `// @name Rereg\nlet n = 0; setInterval(() => { n++; for (let i = 0; i < 50; i++) cairn.commands.register("c" + i, "Again " + i + " v" + n, () => {}); }, 10);\n`,
  }, async (s) => {
    await enablePlugin(s, "rereg.js");
    await sleep(2000);
    await resetLag(s);
    const cpu = await webCpu(3000);
    const lag = await readLag(s);
    const pal = await paletteTiming(s, "Again 7");
    const typing = await typingLatency(s);
    record("rereg-steady", { cpuPct: cpu, lag, palette: pal, typingMs: typing });
    assert.ok(lag.max < 500, `lag ${lag.max}`);
    assert.ok(typing < 3000, `typing ${typing}`);
  });
});

test("a burst of 300,000 messages from a plugin (register loop) does not freeze the page", async () => {
  await withApp("rereg-burst", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/burst.js": `// @name Burst\ncairn.commands.register("go", "Go", () => { for (let i = 0; i < 300000; i++) cairn.commands.register("x" + (i % 100), "Burst " + i, () => {}); });\n`,
  }, async (s) => {
    await enablePlugin(s, "burst.js");
    await escapeAll(s);
    const rss0 = webRss();
    await resetLag(s);
    await runCommand(s, "Burst: Go");
    const t0 = performance.now();
    // Poll a trivial script: how long until the page answers again?
    const pings = [];
    let maxRss = rss0;
    for (let i = 0; i < 12; i++) {
      const r = await execT(s, `return window.__plMsgCount`, 30000);
      pings.push({ ms: r.ms, ok: r.ok, count: r.value });
      maxRss = Math.max(maxRss, webRss());
      if (r.ok && r.ms < 200 && i > 2) break;
      await sleep(500);
    }
    const settleMs = Math.round(performance.now() - t0);
    const lag = await readLag(s);
    const typing = await typingLatency(s);
    record("rereg-burst", { pings, settleMs, lag, typingAfterMs: typing, rssBeforeMb: rss0, maxRssMb: maxRss });
    assert.ok(lag.max < 1000, `main thread blocked for ${lag.max} ms at a stretch; pings ${JSON.stringify(pings.slice(0, 4))}`);
  });
});

test("a plugin re-registering commands in a tight loop for 10 s: the UI stays usable and memory stays bounded", async () => {
  await withApp("rereg-tight", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/tight.js": `// @name Tight\ncairn.commands.register("go", "Go", () => { const end = Date.now() + 10000; let n = 0; while (Date.now() < end) cairn.commands.register("x" + (n++ % 100), "Tight " + n, () => {}); postMessage({ type: "probe", k: "posted", v: n }); });\n`,
  }, async (s) => {
    await enablePlugin(s, "tight.js");
    await escapeAll(s);
    const rss0 = webRss();
    await resetLag(s);
    await runCommand(s, "Tight: Go");
    const t0 = performance.now();
    const samples = [];
    let aborted = false;
    while (performance.now() - t0 < 40000) {
      await sleep(500);
      const rss = webRss();
      const r = await execT(s, `return [window.__plMsgCount, window.__plMsgs.filter((m) => m.data.k === 'posted').map((m) => m.data.v)[0] ?? null]`, 3000);
      samples.push({ t: Math.round(performance.now() - t0), rss, pingMs: r.ms, ok: r.ok, handled: r.ok ? r.value[0] : null, posted: r.ok ? r.value[1] : null });
      if (rss > 3500) {
        aborted = true;
        console.log(`[PA] RSS ${rss} MB over the limit; terminating plugin workers`);
        await execT(s, `(window.__plWorkers || []).forEach((w) => w.terminate()); return true`, 30000);
        break;
      }
      const last = samples.at(-1);
      if (last.ok && last.posted != null && last.handled >= last.posted) break;
    }
    const lag = await readLag(s, 60000);
    const maxRss = Math.max(...samples.map((x) => x.rss));
    await sleep(5000);
    const rssAfter = webRss();
    record("rereg-tight", { rssBeforeMb: rss0, maxRssMb: maxRss, rss5sAfterStopMb: rssAfter, aborted, lag, samples });
    assert.ok(!aborted && maxRss - rss0 < 1000 && lag.max < 1000, `RSS ${rss0} -> ${maxRss} MB, longest main-thread block ${lag.max} ms, aborted: ${aborted}`);
  });
});

// A plugin calling the API in a tight loop with calls the page is slower to answer than to
// receive (FINDING-162): fewer than 20,000 a second reach the page, so only the calls the
// plugin sends at once bound the memory. The plugin's command posts probe "ended" when its
// loop stops (after 10 s, or when a call throws).
async function apiFlood(s, key, cmd) {
  await escapeAll(s);
  const rss0 = webRss();
  await resetLag(s);
  await runCommand(s, cmd);
  const t0 = performance.now();
  const samples = [];
  let aborted = false;
  let endedAt = null;
  // Sample until 3 s after the loop ended, so the calls still waiting in the plugin are answered too.
  while (performance.now() - t0 < 30000 && (endedAt == null || performance.now() - endedAt < 3000)) {
    await sleep(500);
    const rss = webRss();
    const r = await execT(s, `return [window.__plMsgCount, window.__plMsgs.find((m) => m.data.k === 'ended')?.data.v ?? null]`, 3000);
    samples.push({ t: Math.round(performance.now() - t0), rss, pingMs: r.ms, ok: r.ok, handled: r.ok ? r.value[0] : null, calls: r.ok ? r.value[1] : null });
    if (rss - rss0 > 1500) {
      aborted = true;
      console.log(`[PA] RSS ${rss} MB, ${rss - rss0} MB more than at the start; terminating plugin workers`);
      await execT(s, `(window.__plWorkers || []).forEach((w) => w.terminate()); return true`, 30000);
      break;
    }
    if (endedAt == null && samples.at(-1).calls != null) endedAt = performance.now();
  }
  const lag = await readLag(s, 60000);
  const toasts = await toastTexts(s);
  const maxRss = Math.max(...samples.map((x) => x.rss));
  record(key, { rssBeforeMb: rss0, maxRssMb: maxRss, aborted, lag, toasts, samples });
  // Only the 16 calls already sent are answered: answering all 10,016 took 500-900 MB more.
  assert.ok(!aborted && maxRss - rss0 < 300 && lag.max < 1000, `RSS ${rss0} -> ${maxRss} MB, longest main-thread block ${lag.max} ms, aborted: ${aborted}`);
  assert.ok(toasts.some((t) => t.includes("too many calls are waiting")), JSON.stringify(toasts));
}

const loopFor10s = (call) =>
  `cairn.commands.register("go", "Go", () => { const big = "x".repeat(200000); const end = Date.now() + 10000; let n = 0; try { while (Date.now() < end) { ${call}; n++; } } finally { postMessage({ type: "probe", k: "ended", v: n }); } });\n`;

test("a plugin calling ui.toast with a 200 KB text in a tight loop for 10 s: memory stays bounded", async () => {
  await withApp("toast-flood", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/tf.js": `// @name Toaster\n${loopFor10s("cairn.ui.toast(big)")}`,
  }, async (s) => {
    await enablePlugin(s, "tf.js");
    await apiFlood(s, "toast-flood", "Toaster: Go");
  });
});

test("a plugin calling notes.list in a tight loop for 10 s in a vault of 3,000 notes: memory stays bounded", async () => {
  const notes = {};
  for (let i = 0; i < 3000; i++) notes[`folder ${i % 30}/note number ${i}.md`] = `# ${i}\n`;
  await withApp("list-flood", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/lf.js": `// @name Lister\n// @permissions read\n${loopFor10s("cairn.notes.list()")}`,
    ...notes,
  }, async (s) => {
    await enablePlugin(s, "lf.js", { expectDialog: true });
    await apiFlood(s, "list-flood", "Lister: Go");
  });
});

// ---------------------------------------------------------------------------
// Two plugins, same command id
// ---------------------------------------------------------------------------

test("two plugins registering the same command id each keep their own command", async () => {
  const src = (k) => `// @name Tools\ncairn.commands.register("go", "Go", () => postMessage({ type: "probe", k: "${k}" }));\n`;
  await withApp("same-id", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/a.js": src("ran-a"),
    ".cairn/plugins/b.js": src("ran-b"),
  }, async (s) => {
    await enablePlugin(s, "a.js");
    await enablePlugin(s, "b.js");
    await escapeAll(s);
    await s.keys({ chord: [Key.ctrl, "p"] });
    await s.type(await s.findWait("[data-testid=palette-input]"), "Tools: Go");
    await sleep(300);
    const items = (await execT(s, `return [...document.querySelectorAll('[data-testid=palette-item]')].map((e) => e.textContent.trim()).filter((t) => t.startsWith('Tools'))`)).value;
    await shot(s, "PA-same-id-palette.png");
    await s.keys(Key.enter);
    await sleep(300);
    await s.keys({ chord: [Key.ctrl, "p"] });
    await s.type(await s.findWait("[data-testid=palette-input]"), "Tools: Go");
    await sleep(300);
    await s.keys(Key.down, Key.enter);
    await sleep(800);
    const ranA = (await probes(s, "ran-a")).length;
    const ranB = (await probes(s, "ran-b")).length;
    record("same-id", { items, ranA, ranB });
    assert.equal(items.length, 2, JSON.stringify(items));
    assert.equal(ranA, 1);
    assert.equal(ranB, 1);
  });
});

test("two plugins with the same name can be told apart in the command palette", async () => {
  const src = (k) => `// @name Tools\ncairn.commands.register("go", "Go", () => postMessage({ type: "probe", k: "${k}" }));\n`;
  await withApp("same-name", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/a.js": src("ran-a"),
    ".cairn/plugins/evil.js": src("ran-evil"),
  }, async (s) => {
    await enablePlugin(s, "a.js");
    await enablePlugin(s, "evil.js");
    await escapeAll(s);
    await s.keys({ chord: [Key.ctrl, "p"] });
    await s.type(await s.findWait("[data-testid=palette-input]"), "Tools: Go");
    await sleep(300);
    const items = (await execT(s, `return [...document.querySelectorAll('[data-testid=palette-item]')].map((e) => e.textContent.trim()).filter((t) => t.startsWith('Tools'))`)).value;
    await escapeAll(s);
    // Settings > Plugins shows each plugin's file name next to its name, so the entries can be matched to a switch.
    await openPluginSettings(s);
    await s.findWait(`[data-testid=plugin-row][data-file="evil.js"] [data-testid=plugin-file]`);
    const rows = (await execT(s, `return [...document.querySelectorAll('[data-testid=plugin-row]')].map((r) => r.querySelector('b').textContent + ' ' + r.querySelector('[data-testid=plugin-file]').textContent)`)).value;
    await escapeAll(s);
    record("same-name", { items, rows });
    assert.equal(new Set(items).size, items.length, `identical palette entries: ${JSON.stringify(items)}`);
    assert.deepEqual(rows, ["Tools a.js", "Tools evil.js"]);
  });
});

// ---------------------------------------------------------------------------
// Very large messages to the host
// ---------------------------------------------------------------------------

const MB = 1 << 20;

test("a 32 MB toast message: the page stays responsive and shows at most 500 characters", async () => {
  await withApp("big-toast", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/big.js": `// @name Big\ncairn.commands.register("t", "Toast", async () => { const t0 = Date.now(); await cairn.ui.toast("x".repeat(${32 * MB})); postMessage({ type: "probe", k: "done", v: Date.now() - t0 }); });\n`,
  }, async (s) => {
    await enablePlugin(s, "big.js");
    await escapeAll(s);
    await resetLag(s);
    await runCommand(s, "Big: Toast");
    const p = await waitProbe(s, "done", 60000);
    const lag = await readLag(s);
    const len = (await execT(s, `return Math.max(0, ...[...document.querySelectorAll('.toast')].map((t) => t.textContent.length))`)).value;
    record("big-toast", { roundTripMs: p.v, lag, longestToastChars: len, rssMb: webRss() });
    assert.ok(len <= 520, `${len}`);
    assert.ok(lag.max < 1000, `lag ${lag.max}`);
  });
});

test("a 32 MB error from a plugin command is shown capped and does not hang the page", async () => {
  await withApp("big-error", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/err.js": `// @name Err\ncairn.commands.register("e", "Throw", () => { throw new Error("E".repeat(${32 * MB})); });\n`,
  }, async (s) => {
    await enablePlugin(s, "err.js");
    await escapeAll(s);
    await resetLag(s);
    const t0 = performance.now();
    await runCommand(s, "Err: Throw");
    await s.waitFor(`return [...document.querySelectorAll('.toast')].some((t) => t.textContent.startsWith('Plugin Err'))`, { timeout: 60000 });
    const shownMs = Math.round(performance.now() - t0);
    const lag = await readLag(s);
    const len = (await execT(s, `return Math.max(0, ...[...document.querySelectorAll('.toast')].map((t) => t.textContent.length))`, 30000)).value;
    await shot(s, "PA-07-big-error.png");
    record("big-error", { shownMs, lag, longestToastChars: len, rssMb: webRss() });
    assert.ok(len <= 1000 && lag.max < 1000, `toast of ${len} chars; main thread blocked ${lag.max} ms`);
  });
});

test("replaceSelection with 32 MB: the editor survives, the note is saved, the page does not hang", async () => {
  await withApp("big-replace", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/rep.js": `// @name Rep\n// @permissions editor\ncairn.commands.register("r", "Replace", async () => { const t0 = Date.now(); const ok = await cairn.editor.replaceSelection(("lorem ipsum dolor sit amet ".repeat(40) + "\\n").repeat(${Math.ceil((32 * MB) / 1081)})); postMessage({ type: "probe", k: "done", v: { ms: Date.now() - t0, ok } }); });\n`,
  }, async (s, v) => {
    await enablePlugin(s, "rep.js", { expectDialog: true });
    await escapeAll(s);
    await s.click(await s.find("[data-testid=tree-row]"));
    await s.click(await s.findWait(".cm-content"));
    await resetLag(s);
    const t0 = performance.now();
    await runCommand(s, "Rep: Replace");
    let p = null;
    try {
      p = await waitProbe(s, "done", 120000);
    } catch {}
    const doneMs = Math.round(performance.now() - t0);
    const lag = await readLag(s, 120000);
    let size = 0;
    for (let i = 0; i < 60; i++) {
      size = fs.statSync(v.path("Home.md")).size;
      if (size > 30 * MB) break;
      await sleep(1000);
    }
    const typing = await typingLatency(s).catch((e) => `ERR ${e.message.slice(0, 100)}`);
    await shot(s, "PA-big-replace.png");
    record("big-replace", { probe: p && p.v, doneMs, lag, savedBytes: size, typingAfterMs: typing, rssMb: webRss() });
    assert.ok(p && lag.max < 2000, `main thread blocked ${lag.max} ms; probe ${JSON.stringify(p && p.v)}`);
  });
});

test("notes.write with 32 MB: the note is written and the page does not hang", async () => {
  await withApp("big-write", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/wr.js": `// @name Wr\n// @permissions write\ncairn.commands.register("w", "Write", async () => { const t0 = Date.now(); let err = null; try { await cairn.notes.write("Big.md", ("lorem ipsum dolor sit amet ".repeat(40) + "\\n").repeat(${Math.ceil((32 * MB) / 1081)})); } catch (e) { err = String(e.message).slice(0, 200); } postMessage({ type: "probe", k: "done", v: { ms: Date.now() - t0, err } }); });\n`,
  }, async (s, v) => {
    await enablePlugin(s, "wr.js", { expectDialog: true });
    await escapeAll(s);
    await resetLag(s);
    await runCommand(s, "Wr: Write");
    const p = await waitProbe(s, "done", 120000);
    const lag = await readLag(s);
    const size = fs.existsSync(v.path("Big.md")) ? fs.statSync(v.path("Big.md")).size : -1;
    record("big-write", { probe: p.v, lag, bytesOnDisk: size, rssMb: webRss() });
    const typing = await typingLatency(s);
    record("big-write-typing", { typingAfterMs: typing });
    assert.equal(p.v.err, null);
    assert.ok(size >= 32 * MB, `${size}`);
    assert.ok(lag.max < 2000, `lag ${lag.max}`);
  });
});

// ---------------------------------------------------------------------------
// Recursion and memory inside the worker
// ---------------------------------------------------------------------------

test("deep recursion in a plugin (load and command) is reported and does not affect the app", async () => {
  await withApp("recursion", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/rec.js": `// @name Rec\nconst f = (n) => f(n + 1) + 1;\ncairn.commands.register("r", "Recurse", () => f(0));\ncairn.commands.register("p", "Ping", () => postMessage({ type: "probe", k: "ping" }));\nf(0);\n`,
  }, async (s) => {
    await enablePlugin(s, "rec.js");
    await sleep(1000);
    const loadToasts = await toastTexts(s);
    await escapeAll(s);
    await runCommand(s, "Rec: Recurse");
    await sleep(1500);
    const cmdToasts = await toastTexts(s);
    record("recursion", { loadToasts, cmdToasts });
    // The load-time throw happens before register() calls are reached? No: f(0) is last, after the registers.
    assert.ok(loadToasts.some((t) => /Rec|RangeError|stack|recursion/i.test(t)), JSON.stringify(loadToasts));
    assert.ok(cmdToasts.some((t) => /Rec:.*(RangeError|stack|recursion)/i.test(t) || /Plugin Rec/.test(t)), JSON.stringify(cmdToasts));
    await runCommand(s, "Rec: Ping");
    await waitProbe(s, "ping", 5000);
  });
});

test("a plugin that allocates memory without limit is stopped before it takes the app down", { todo: "FINDING-161: there is no memory limit for plugin workers; a plugin allocating in a loop grows the web process until the OOM killer kills it and the window goes blank", skip: OOM ? false : "set PA_OOM=1 and run inside a memory-capped systemd scope", timeout: 240000 }, async () => {
  await withApp("oom", {
    ".cairn/settings.json": settings(),
    ".cairn/plugins/hog.js": `// @name Hog\ncairn.commands.register("go", "Go", async () => { const keep = []; for (let i = 0; i < 200; i++) { const a = new Uint8Array(64 * 1024 * 1024); a.fill(i + 1); keep.push(a); postMessage({ type: "probe", k: "mb", v: (i + 1) * 64 }); await new Promise((r) => setTimeout(r, 50)); } postMessage({ type: "probe", k: "survived" }); });\n`,
  }, async (s) => {
    await enablePlugin(s, "hog.js");
    await escapeAll(s);
    const web0 = webProcs().map((p) => p.pid);
    const rssLog = [];
    await runCommand(s, "Hog: Go");
    let alive = true;
    for (let i = 0; i < 120; i++) {
      await sleep(1000);
      const rss = webRss();
      rssLog.push(rss);
      alive = web0.every((pid) => {
        try {
          process.kill(pid, 0);
          return true;
        } catch {
          return false;
        }
      });
      if (!alive) break;
      const r = await execT(s, `return window.__plMsgs.filter((m) => m.data.k === 'survived' || m.data.k === 'mb').map((m) => m.data.v || m.data.k).slice(-1)[0]`, 5000);
      if (r.ok && r.value === "survived") break;
    }
    const page = await execT(s, `return document.querySelectorAll('[data-testid=tree-row]').length`, 10000);
    const toasts = alive ? await toastTexts(s) : [];
    await shot(s, "PA-08-oom.png");
    record("oom", { webProcessAlive: alive, pageAnswers: page, rssLogMb: rssLog, toasts });
    assert.ok(alive && page.ok, `web process alive: ${alive}; page answers: ${JSON.stringify(page)}; peak RSS ${Math.max(...rssLog)} MB`);
  });
});
