// Helpers for the performance tests (e2e/adv_perf.test.mjs).
// Process/RSS sampling, statistics, and the in-page measuring snippets.

import fs from "node:fs";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

export function stats(xs) {
  const a = xs.filter((x) => Number.isFinite(x)).sort((p, q) => p - q);
  if (!a.length) return { n: 0 };
  const q = (p) => a[Math.min(a.length - 1, Math.max(0, Math.ceil(p * a.length) - 1))];
  const r = (x) => Math.round(x * 10) / 10;
  return { n: a.length, min: r(a[0]), median: r(q(0.5)), p95: r(q(0.95)), max: r(a[a.length - 1]), mean: r(a.reduce((s, x) => s + x, 0) / a.length) };
}

/** Children map from /proc (pid -> [child pids]). */
function childrenMap() {
  const m = new Map();
  for (const d of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(d)) continue;
    try {
      const st = fs.readFileSync(`/proc/${d}/stat`, "utf8");
      const ppid = Number(st.slice(st.lastIndexOf(")") + 2).split(" ")[1]);
      if (!m.has(ppid)) m.set(ppid, []);
      m.get(ppid).push(Number(d));
    } catch {}
  }
  return m;
}

export function descendants(pid) {
  const m = childrenMap();
  const out = [];
  const stack = [pid];
  while (stack.length) {
    const p = stack.pop();
    for (const c of m.get(p) ?? []) {
      out.push(c);
      stack.push(c);
    }
  }
  return out;
}

function procInfo(pid) {
  try {
    const comm = fs.readFileSync(`/proc/${pid}/comm`, "utf8").trim();
    const status = fs.readFileSync(`/proc/${pid}/status`, "utf8");
    const kb = (k) => Number((status.match(new RegExp(`^${k}:\\s+(\\d+)`, "m")) ?? [])[1] ?? 0);
    let pss = 0;
    try {
      const sr = fs.readFileSync(`/proc/${pid}/smaps_rollup`, "utf8");
      pss = Number((sr.match(/^Pss:\s+(\d+)/m) ?? [])[1] ?? 0);
    } catch {}
    return { pid, comm, rssMB: Math.round(kb("VmRSS") / 102.4) / 10, hwmMB: Math.round(kb("VmHWM") / 102.4) / 10, pssMB: Math.round(pss / 102.4) / 10 };
  } catch {
    return null;
  }
}

/** RSS/PSS of a process and its descendants that use memory (WebKit helpers). */
export function memTree(pid) {
  const list = [pid, ...descendants(pid)].map(procInfo).filter((x) => x && x.rssMB > 0);
  const sum = (k) => Math.round(list.reduce((s, x) => s + x[k], 0) * 10) / 10;
  return { procs: list, totalRssMB: sum("rssMB"), totalPssMB: sum("pssMB") };
}

/** PIDs of processes whose argv contains `needle` exactly as one argument. */
export function pidsWithArg(needle) {
  const out = [];
  for (const d of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(d) || Number(d) === process.pid) continue;
    try {
      const argv = fs.readFileSync(`/proc/${d}/cmdline`, "utf8").split("\0");
      if (argv.includes(needle)) out.push(Number(d));
    } catch {}
  }
  return out;
}

export function evict(dir) {
  return execFileSync("python3", [new URL("./evict-cache.py", import.meta.url).pathname, dir], { encoding: "utf8" }).trim();
}

/**
 * Start the app binary directly (no WebDriver) on `vault`, wait for the
 * "UI ready N ms after process start" log line, sample memory after
 * `settleMs`, then kill it and its helpers.
 */
export async function rawStart(app, vault, xdg, { settleMs = 3000, timeoutMs = 60000 } = {}) {
  fs.mkdirSync(xdg, { recursive: true });
  const env = {
    ...process.env,
    XDG_CONFIG_HOME: `${xdg}/config`,
    XDG_DATA_HOME: `${xdg}/data`,
    XDG_CACHE_HOME: `${xdg}/cache`,
    RUST_LOG: "info",
  };
  const t0 = performance.now();
  const p = spawn(app, [vault], { env, stdio: ["ignore", "pipe", "pipe"] });
  let log = "";
  let readyAt = null;
  let uiMs = null;
  const onData = (d) => {
    log += d;
    const m = log.match(/UI ready (\d+) ms after process start/);
    if (m && uiMs == null) {
      uiMs = Number(m[1]);
      readyAt = performance.now();
    }
  };
  p.stdout.on("data", onData);
  p.stderr.on("data", onData);
  const end = Date.now() + timeoutMs;
  while (uiMs == null && Date.now() < end && p.exitCode == null) await sleep(5);
  let mem = null;
  if (uiMs != null) {
    await sleep(settleMs);
    mem = memTree(p.pid);
  }
  const kids = descendants(p.pid);
  p.kill("SIGTERM");
  await Promise.race([new Promise((r) => p.once("exit", r)), sleep(3000)]);
  for (const k of [p.pid, ...kids]) {
    try {
      process.kill(k, "SIGKILL");
    } catch {}
  }
  await sleep(300);
  return { uiReadyMs: uiMs, spawnToLogMs: readyAt ? Math.round(readyAt - t0) : null, mem, logTail: uiMs == null ? log.slice(-1500) : undefined };
}

// ---- snippets run in the page --------------------------------------------

/** Start recording requestAnimationFrame timestamps in window.__pf.frames. */
export const FRAMES_START = `
  window.__pf = window.__pf || {};
  window.__pf.frames = [];
  window.__pf.rec = true;
  const loop = () => { if (!window.__pf.rec) return; window.__pf.frames.push(performance.now()); requestAnimationFrame(loop); };
  requestAnimationFrame(loop);
  return true;
`;
/** Stop recording; return frame intervals (ms). */
export const FRAMES_STOP = `
  window.__pf.rec = false;
  const f = window.__pf.frames; const d = [];
  for (let i = 1; i < f.length; i++) d.push(f[i] - f[i - 1]);
  return d;
`;

/**
 * Key latency recorder: for every keydown (capture phase on window) record
 * event time -> time when the frame after it has been produced (rAF, then a
 * message posted from inside the rAF callback, which runs after that frame's
 * style/layout/paint work).
 */
export const KEYLAT_START = `
  window.__pf = window.__pf || {};
  window.__pf.keys = [];
  if (!window.__pf.keyHook) {
    window.__pf.keyHook = true;
    window.addEventListener('keydown', (e) => {
      if (!window.__pf.keyOn) return;
      const t0 = e.timeStamp; const rec = { t0 };
      window.__pf.keys.push(rec);
      requestAnimationFrame(() => {
        rec.raf = performance.now();
        const ch = new MessageChannel();
        ch.port1.onmessage = () => { rec.after = performance.now(); };
        ch.port2.postMessage(0);
      });
    }, true);
  }
  window.__pf.keyOn = true;
  return true;
`;
export const KEYLAT_STOP = `
  window.__pf.keyOn = false;
  return window.__pf.keys.map(r => ({ toRaf: r.raf - r.t0, toFrame: (r.after ?? r.raf) - r.t0 }));
`;

/**
 * Time from now until `cond` (a JS expression) is true, checked on every
 * animation frame; the reported time is the end of the frame that showed it
 * (message posted from that rAF). Returns ms, or -1 on timeout.
 */
export function untilFrame(cond, timeoutMs = 60000) {
  return `
    const done = arguments[arguments.length - 1];
    const t0 = window.__pf_t0 ?? performance.now();
    window.__pf_t0 = undefined;
    const check = () => {
      let ok = false; try { ok = !!(${cond}); } catch (e) {}
      if (ok) {
        const ch = new MessageChannel();
        ch.port1.onmessage = () => done(performance.now() - t0);
        ch.port2.postMessage(0);
        return;
      }
      if (performance.now() - t0 > ${timeoutMs}) return done(-1);
      requestAnimationFrame(check);
    };
    requestAnimationFrame(check);
  `;
}
