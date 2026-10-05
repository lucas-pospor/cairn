// Performance measurements on large generated vaults (adversarial tests).
// Skipped unless CAIRN_PERF=1, so the normal e2e run ignores it.
//
//   node scripts/gen-vault.mjs target/tmp/perf-v10000 10000
//   node scripts/gen-vault.mjs target/tmp/perf-v50000 50000
//   node scripts/adv-perf-gen.mjs bignotes target/tmp/perf-bignotes
//   node scripts/adv-perf-gen.mjs flat target/tmp/perf-flat 20000
//   CAIRN_PERF=1 scripts/e2e-headless.sh e2e/adv_perf.test.mjs
//
// Env: CAIRN_PERF_APP (default target/release/cairn), CAIRN_PERF_PARTS
// (comma list of startup,inapp,typing,tree; default all), CAIRN_PERF_VAULTS
// (comma list, default the two generated vaults), CAIRN_PERF_OUT (folder for
// the JSON results, default target/tmp/perf-results).
// Sync is measured separately with scripts/adv-perf-sync.py.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { startDriver, createSession, sleep } from "./adv_editor_lib.mjs";
import {
  stats,
  memTree,
  pidsWithArg,
  evict,
  rawStart,
  FRAMES_START,
  FRAMES_STOP,
  KEYLAT_START,
  KEYLAT_STOP,
} from "../scripts/adv-perf-lib.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const ON = process.env.CAIRN_PERF === "1";
const APP = path.resolve(ROOT, process.env.CAIRN_PERF_APP ?? "target/release/cairn");
const PARTS = (process.env.CAIRN_PERF_PARTS ?? "startup,inapp,typing,tree").split(",");
const VAULTS = (process.env.CAIRN_PERF_VAULTS ?? "target/tmp/perf-v10000,target/tmp/perf-v50000").split(",").map((v) => path.resolve(ROOT, v));
const BIGNOTES = path.resolve(ROOT, "target/tmp/perf-bignotes");
const FLAT = path.resolve(ROOT, "target/tmp/perf-flat");
const OUT = path.resolve(ROOT, process.env.CAIRN_PERF_OUT ?? "target/tmp/perf-results");
const TMP = path.resolve(ROOT, "target/tmp/perf-xdg");
const appLabel = `${path.relative(ROOT, APP)}`;

function save(name, data) {
  fs.mkdirSync(OUT, { recursive: true });
  const f = path.join(OUT, `${name}.json`);
  fs.writeFileSync(f, JSON.stringify({ app: appLabel, at: new Date().toISOString(), ...data }, null, 1));
  console.log(`# ${name}: ${JSON.stringify(data).slice(0, 4000)}`);
}

let xdgN = 0;
function freshXdg() {
  const d = path.join(TMP, `x${process.pid}-${xdgN++}`);
  fs.rmSync(d, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  fs.mkdirSync(d, { recursive: true });
  return d;
}

/** Start tauri-driver + the app on `vault`; returns { s, pid, stop }. */
async function launch(vault, waitTimeout = 120000) {
  const xdg = freshXdg();
  const drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(xdg, "config"),
    XDG_DATA_HOME: path.join(xdg, "data"),
    XDG_CACHE_HOME: path.join(xdg, "cache"),
  });
  let s;
  try {
    s = await createSession(drv.port, APP, [vault], 60000);
    await s.cmd("POST", "/timeouts", { script: 180000 });
    await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: waitTimeout });
  } catch (e) {
    await s?.close();
    drv.proc.kill();
    throw new Error(`${e.message}\n--- driver log ---\n${drv.log().slice(-3000)}`);
  }
  const pid = pidsWithArg(vault).find((p) => {
    try {
      return fs.readFileSync(`/proc/${p}/comm`, "utf8").trim() === path.basename(APP).slice(0, 15);
    } catch {
      return false;
    }
  });
  const stop = async () => {
    try {
      await Promise.race([s.close(), sleep(8000)]);
    } catch {}
    if (drv.proc.exitCode == null) {
      const done = new Promise((r) => drv.proc.once("exit", r));
      drv.proc.kill();
      await Promise.race([done, sleep(3000)]);
    }
    for (const p of pidsWithArg(vault)) {
      try {
        process.kill(p, "SIGKILL");
      } catch {}
    }
    await sleep(500);
    fs.rmSync(xdg, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  };
  return { s, pid, stop };
}

/** Run `action` (JS) in the page, then resolve at the end of the first frame where `cond` holds. */
async function timed(s, action, cond = "true", timeoutMs = 60000) {
  return s.execAsync(
    `const done = arguments[arguments.length - 1];
     const t0 = performance.now();
     try { ${action} } catch (e) { return done('action error: ' + e); }
     const check = () => {
       let ok = false; try { ok = !!(${cond}); } catch (e) {}
       if (ok) { const ch = new MessageChannel(); ch.port1.onmessage = () => done(performance.now() - t0); ch.port2.postMessage(0); return; }
       if (performance.now() - t0 > ${timeoutMs}) return done(-1);
       requestAnimationFrame(check);
     };
     requestAnimationFrame(check);`,
  );
}

function setInput(testid, value) {
  return `{ const el = document.querySelector('[data-testid=${testid}]'); el.focus(); el.value = ${JSON.stringify(value)}; el.dispatchEvent(new Event('input', { bubbles: true })); }`;
}

const r1 = (x) => Math.round(x * 10) / 10;

// ---------------------------------------------------------------- 1 + 7: start-up and RSS

test("perf: start-up (process start to UI ready) and RSS after opening", { skip: !ON || !PARTS.includes("startup") }, async () => {
  const out = {};
  for (const vault of [...VAULTS, FLAT]) {
    const name = path.basename(vault);
    const warm = [];
    const spawnToLog = [];
    const mems = [];
    const first = await rawStart(APP, vault, freshXdg()); // warms the page cache, discarded
    assert.ok(first.uiReadyMs != null, `app did not report UI ready:\n${first.logTail}`);
    for (let i = 0; i < 5; i++) {
      const r = await rawStart(APP, vault, freshXdg());
      warm.push(r.uiReadyMs);
      spawnToLog.push(r.spawnToLogMs);
      mems.push(r.mem);
    }
    const cold = [];
    for (let i = 0; i < 3; i++) {
      evict(vault);
      evict(APP);
      const r = await rawStart(APP, vault, freshXdg());
      cold.push(r.uiReadyMs);
    }
    out[name] = {
      warm: { runs: warm, ...stats(warm) },
      warmSpawnToLog: stats(spawnToLog),
      cold: { runs: cold, ...stats(cold) },
      rssAfterOpen3s: {
        totalRssMB: stats(mems.map((m) => m.totalRssMB)),
        totalPssMB: stats(mems.map((m) => m.totalPssMB)),
        lastProcs: mems.at(-1).procs,
      },
    };
    save(`startup-${name}`, out[name]);
  }
});

// ---------------------------------------------------------------- 2, 3, 7: search, switcher, graph, RSS

const QUERIES = ["garden", "river stone", "harv", "g", "tag:garden", "path:projects garden", '"river stone"', "zzzz"];

for (const vault of VAULTS) {
  const name = path.basename(vault);
  test(`perf: search, quick switcher, graph on ${name}`, { skip: !ON || !PARTS.includes("inapp"), timeout: 900000 }, async () => {
    const tLaunch = performance.now();
    const app = await launch(vault);
    const s = app.s;
    const res = { launchToTreeMs: Math.round(performance.now() - tLaunch) };
    try {
      res.startupMs = await s.waitFor(`return window.__cairnStartupMs`, { timeout: 30000 });
      await sleep(1500);
      res.memAfterOpen = memTree(app.pid);

      // -- backend search latency, invoked from the page (includes IPC)
      res.searchBackend = await s.execAsync(
        `const done = arguments[arguments.length - 1];
         (async () => {
           const out = {};
           for (const q of arguments[0]) {
             const ts = []; let hits = 0;
             for (let i = 0; i < 11; i++) {
               const t = performance.now();
               const r = await window.__TAURI_INTERNALS__.invoke('search', { query: q, limit: 200 });
               const dt = performance.now() - t;
               if (i > 0) ts.push(dt);
               hits = r.length;
             }
             out[q] = { hits, ts };
           }
           done(out);
         })().catch(e => done({ err: String(e && (e.message || e)) }));`,
        QUERIES,
      );
      for (const q of Object.keys(res.searchBackend)) {
        const v = res.searchBackend[q];
        if (v.ts) res.searchBackend[q] = { hits: v.hits, ...stats(v.ts) };
      }

      // -- search panel: from setting the query to the frame that shows results (includes the 120 ms debounce)
      await s.exec(`document.querySelector('[data-testid=tab-search]').click(); return true`);
      await s.waitFor(`return !!document.querySelector('[data-testid=search-input]')`);
      res.searchUi = {};
      const noMatchFlash = {};
      res.searchUiNoMatchesShownWhileWaiting = noMatchFlash;
      for (const q of QUERIES) {
        const ts = [];
        for (let i = 0; i < 4; i++) {
          await timed(s, setInput("search-input", ""), `document.querySelectorAll('[data-testid=search-hit]').length === 0`, 5000);
          await sleep(200);
          // Results are shown once the panel's search call has resolved and the
          // frame after it is drawn (the panel shows "No matches." while it waits
          // for the 120 ms debounce, so the DOM alone cannot tell).
          const t = await timed(
            s,
            `window.__pfSearchDone = false;
             if (!window.fetch.__pf) {
               // Tauri IPC goes through fetch('ipc://localhost/<cmd>'); mark when the search call has answered.
               const of = window.fetch;
               const w = function (input, init) {
                 const url = typeof input === 'string' ? input : (input && input.url) || '';
                 const p = of.call(this, input, init);
                 if (url.endsWith('/search')) p.then(() => { window.__pfSearchDone = true; }, () => { window.__pfSearchDone = true; });
                 return p;
               };
               w.__pf = true; window.fetch = w;
             }
             ${setInput("search-input", q)}`,
            `window.__pfSearchDone && (document.querySelectorAll('[data-testid=search-hit]').length > 0 || document.querySelector('.results .none'))`,
            5000,
          );
          if (i > 0) ts.push(t);
        }
        const shown = await s.exec(`return document.querySelector('.search .meta')?.textContent.trim()`);
        // Does the panel claim "No matches." during the debounce for a query that has hits?
        await timed(s, setInput("search-input", ""), "true");
        await sleep(200);
        noMatchFlash[q] = await s.execAsync(
          `const done = arguments[arguments.length - 1];
           ${setInput("search-input", q)}
           requestAnimationFrame(() => requestAnimationFrame(() => done(document.querySelector('.results .none')?.textContent.trim() ?? null)));`,
        );
        res.searchUi[q] = { shown, ...stats(ts) };
      }
      await timed(s, setInput("search-input", ""), "true");

      // -- quick switcher: open (Ctrl+O) and filter
      const sw = { open: [], filter: [] };
      for (let i = 0; i < 4; i++) {
        const t = await timed(
          s,
          `window.dispatchEvent(new KeyboardEvent('keydown', { key: 'o', code: 'KeyO', ctrlKey: true, bubbles: true, cancelable: true }));`,
          `document.querySelectorAll('[data-testid=switcher-item]').length > 0`,
          10000,
        );
        sw.open.push(t);
        let q = "";
        for (const ch of "river stone 42") {
          q += ch;
          sw.filter.push(await timed(s, setInput("switcher-input", q), "true"));
        }
        await s.exec(`document.querySelector('[data-testid=switcher-input]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })); return true`);
        await s.waitFor(`return !document.querySelector('[data-testid=switcher-input]')`);
        await sleep(200);
      }
      res.switcher = { open: { runs: sw.open.map(r1), ...stats(sw.open.slice(1)) }, filterPerKeystroke: stats(sw.filter) };

      // -- graph backend
      res.graphBackend = await s.execAsync(
        `const done = arguments[arguments.length - 1];
         (async () => { const ts = []; let n = 0, e = 0;
           for (let i = 0; i < 6; i++) { const t = performance.now(); const g = await window.__TAURI_INTERNALS__.invoke('graph', { includeUnresolved: false }); const dt = performance.now() - t; if (i) ts.push(dt); n = g.nodes.length; e = g.edges.length; }
           done({ nodes: n, edges: e, ts }); })().catch(err => done({ err: String(err) }));`,
      );
      if (res.graphBackend.ts) res.graphBackend = { nodes: res.graphBackend.nodes, edges: res.graphBackend.edges, ...stats(res.graphBackend.ts) };

      // -- graph view: open, first frame with data, frames during layout
      await s.exec(FRAMES_START);
      const tOpen = await timed(s, `document.querySelector('[data-testid=open-graph]').click();`, `document.querySelector('[data-testid=graph-view] canvas')`, 30000);
      const tData = await timed(s, "", `(() => { const t = document.querySelector('[data-testid=graph-stats]')?.textContent ?? ''; return t && !/Loading/.test(t); })()`, 60000);
      const tLayoutEnd = await timed(s, "", `!/laying out/.test(document.querySelector('[data-testid=graph-stats]')?.textContent ?? 'laying out')`, 60000);
      const layoutFrames = await s.exec(FRAMES_STOP);
      res.graphView = {
        clickToCanvasMs: r1(tOpen),
        thenToDataShownMs: r1(tData),
        clickToDataShownMs: r1(tOpen + tData),
        thenToLayoutEndMs: r1(tLayoutEnd),
        stats: await s.exec(`return document.querySelector('[data-testid=graph-stats]').textContent`),
        frameIntervalsDuringOpenAndLayout: { ...stats(layoutFrames), over50: layoutFrames.filter((x) => x > 50).length, over100: layoutFrames.filter((x) => x > 100).length, top5: [...layoutFrames].sort((a, b) => b - a).slice(0, 5).map(r1) },
      };
      res.memWithGraph = memTree(app.pid);

      // -- hover: find nodes under synthetic mouse moves, then time enter/leave
      res.graphHover = await s.execAsync(
        `const done = arguments[arguments.length - 1];
         const c = document.querySelector('[data-testid=graph-view] .canvas');
         const canv = [...c.querySelectorAll('canvas')];
         const mouse = c.querySelector('canvas.sigma-mouse') || canv[canv.length - 1];
         const r = c.getBoundingClientRect();
         const move = (x, y) => mouse.dispatchEvent(new MouseEvent('mousemove', { clientX: x, clientY: y, bubbles: true, cancelable: true }));
         const frame = () => new Promise(res => requestAnimationFrame(() => { const ch = new MessageChannel(); ch.port1.onmessage = () => res(); ch.port2.postMessage(0); }));
         (async () => {
           const hitPts = []; const empty = [];
           for (let gy = 0.15; gy < 0.95 && hitPts.length < 12; gy += 0.035) {
             for (let gx = 0.05; gx < 0.95 && hitPts.length < 12; gx += 0.035) {
               const x = r.left + r.width * gx, y = r.top + r.height * gy;
               move(x, y);
               if (c.style.cursor === 'pointer') { hitPts.push([x, y]); await frame(); }
               else if (empty.length < 3 && gy > 0.8) empty.push([x, y]);
             }
           }
           if (!empty.length) empty.push([r.left + 5, r.bottom - 5]);
           move(...empty[0]); await frame(); await frame();
           const enter = [], enterSync = [], leave = [], frames = [];
           let rec = true; let last = performance.now();
           const loop = () => { if (!rec) return; const n = performance.now(); frames.push(n - last); last = n; requestAnimationFrame(loop); };
           requestAnimationFrame(loop);
           for (const p of hitPts.slice(0, 10)) {
             let t0 = performance.now(); move(...p); enterSync.push(performance.now() - t0);
             const okEnter = c.style.cursor === 'pointer';
             await frame(); enter.push(okEnter ? performance.now() - t0 : NaN);
             await frame();
             t0 = performance.now(); move(...empty[0]); await frame(); leave.push(performance.now() - t0);
             await frame();
           }
           rec = false;
           done({ found: hitPts.length, enter, enterSync, leave, frames: frames.slice(1), hit: hitPts[0] || null });
         })().catch(e => done({ err: String(e) }));`,
      );
      const h = res.graphHover;
      if (h.enter) {
        res.graphHover = {
          found: h.found,
          enterToFrameEndMs: stats(h.enter),
          enterHandlerSyncMs: stats(h.enterSync),
          leaveToFrameEndMs: stats(h.leave),
          frameIntervalsDuringHover: stats(h.frames),
          hit: h.hit,
        };
      }
      // -- click a node: real pointer click, mousedown to the opened note's frame
      if (h.hit) {
        const clicks = [];
        for (let i = 0; i < 3; i++) {
          if (i > 0) {
            await s.exec(`document.querySelector('[data-testid=open-graph]').click(); return true`);
            await sleep(500);
            await s.waitFor(`return !/laying out|Loading/.test(document.querySelector('[data-testid=graph-stats]')?.textContent ?? 'Loading')`, { timeout: 60000 });
            await sleep(300);
          }
          // re-find a hoverable point (positions can change after a new layout)
          const pt = await s.exec(
            `const c = document.querySelector('[data-testid=graph-view] .canvas'); const canv = [...c.querySelectorAll('canvas')]; const mouse = c.querySelector('canvas.sigma-mouse') || canv[canv.length - 1];
             const r = c.getBoundingClientRect();
             for (let gy = 0.2; gy < 0.9; gy += 0.03) for (let gx = 0.1; gx < 0.9; gx += 0.03) { const x = r.left + r.width * gx, y = r.top + r.height * gy; mouse.dispatchEvent(new MouseEvent('mousemove', { clientX: x, clientY: y, bubbles: true })); if (c.style.cursor === 'pointer') return [Math.round(x), Math.round(y)]; }
             return null;`,
          );
          if (!pt) break;
          await s.exec(`window.__pfClick = {}; window.addEventListener('mousedown', (e) => { window.__pfClick.t0 = e.timeStamp; }, { once: true, capture: true }); return true`);
          await s.pointer([
            { type: "pointerMove", origin: "viewport", x: pt[0], y: pt[1] },
            { type: "pause", duration: 50 },
            { type: "pointerDown", button: 0 },
            { type: "pointerUp", button: 0 },
          ]);
          const t = await s.execAsync(
            `const done = arguments[arguments.length - 1];
             const check = () => {
               const tab = document.querySelector('[data-testid=tab][aria-selected=true]');
               if (tab && (tab.dataset.path || '').endsWith('.md') && document.querySelector('.cm-editor .cm-line')) {
                 const ch = new MessageChannel(); ch.port1.onmessage = () => done(performance.now() - window.__pfClick.t0); ch.port2.postMessage(0); return;
               }
               if (performance.now() - (window.__pfClick.t0 ?? performance.now()) > 20000) return done(-1);
               requestAnimationFrame(check);
             };
             requestAnimationFrame(check);`,
          );
          clicks.push(t);
        }
        res.graphClickToNoteShownMs = { runs: clicks.map(r1), ...stats(clicks) };
      }
      res.memEnd = memTree(app.pid);
    } finally {
      save(`inapp-${name}`, res);
      await app.stop();
    }
  });
}

// ---------------------------------------------------------------- 4: typing in long notes

async function typeRun(s, note, mode, nChars) {
  await s.exec(`document.querySelector('[data-testid=mode-${mode}]').click(); return true`);
  await sleep(400);
  const lines = await s.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.lines`);
  const target = Math.floor(lines / 2) + (mode === "source" ? 37 : 0);
  await s.exec(
    `document.hasFocus = () => true;
     const v = document.querySelector('.cm-editor').__cairnView; v.focus();
     const l = v.state.doc.line(arguments[0]);
     v.dispatch({ selection: { anchor: l.to }, scrollIntoView: true }); return true`,
    target,
  );
  await sleep(800);
  const len0 = await s.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.length`);
  const text = "the quick brown fox jumps over the lazy dog and keeps typing ".repeat(5).slice(0, nChars);
  await s.exec(KEYLAT_START);
  await s.exec(FRAMES_START);
  const t0 = performance.now();
  for (const ch of text) await s.keys(ch);
  const wall = performance.now() - t0;
  await sleep(500);
  const frames = await s.exec(FRAMES_STOP);
  const keys = await s.exec(KEYLAT_STOP);
  const len1 = await s.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.length`);
  return {
    note,
    mode,
    lines,
    atLine: target,
    chars: text.length,
    inserted: len1 - len0,
    msPerCharWall: r1(wall / text.length),
    keyToFrameEnd: stats(keys.map((k) => k.toFrame)),
    keyToRaf: stats(keys.map((k) => k.toRaf)),
    keysOver50: keys.filter((k) => k.toFrame > 50).length,
    frameIntervals: { ...stats(frames), over50: frames.filter((x) => x > 50).length, top5: [...frames].sort((a, b) => b - a).slice(0, 5).map(r1) },
  };
}

test("perf: typing latency in a 3,000-line and a 20,000-line note", { skip: !ON || !PARTS.includes("typing"), timeout: 900000 }, async () => {
  const app = await launch(BIGNOTES);
  const s = app.s;
  const res = { runs: [] };
  try {
    for (const note of ["long-3000.md", "long-20000.md"]) {
      // open time: click in the tree until the editor shows the note's lines
      const tOpen = await timed(
        s,
        `document.querySelector('[data-testid=tree-row][data-path="${note}"]').click();`,
        `document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === ${JSON.stringify(note)} && document.querySelector('.cm-editor')?.__cairnView?.state.doc.lines > 1000 && document.querySelector('.cm-line')`,
        30000,
      );
      res[`open-${note}`] = r1(tOpen);
      await sleep(1000);
      for (const mode of ["live", "source", "live"]) res.runs.push(await typeRun(s, note, mode, 200));
      // scroll the whole note in Live Preview
      await s.exec(`document.querySelector('[data-testid=mode-live]').click(); return true`);
      await sleep(400);
      await s.exec(FRAMES_START);
      await s.execAsync(
        `const done = arguments[arguments.length - 1];
         const sc = document.querySelector('.cm-scroller'); sc.scrollTop = 0; let n = 0;
         const step = () => { sc.scrollTop += 400; if (++n < 300 && sc.scrollTop + sc.clientHeight < sc.scrollHeight) requestAnimationFrame(step); else done(n); };
         requestAnimationFrame(step);`,
      );
      const fr = await s.exec(FRAMES_STOP);
      res[`scrollLive-${note}`] = { ...stats(fr), over50: fr.filter((x) => x > 50).length };
    }
    res.mem = memTree(app.pid);
  } finally {
    save("typing", res);
    await app.stop();
  }
});

// ---------------------------------------------------------------- 5: one folder with 20,000 files

test("perf: file tree with one folder of 20,000 notes, quick switcher", { skip: !ON || !PARTS.includes("tree"), timeout: 900000 }, async () => {
  const app = await launch(FLAT);
  const s = app.s;
  const res = {};
  try {
    await sleep(1000);
    res.startupMs = await s.exec(`return window.__cairnStartupMs ?? null`);
    const expand = [];
    const collapse = [];
    for (let i = 0; i < 4; i++) {
      expand.push(
        await timed(s, `document.querySelector('[data-testid=tree-row][data-path="Inbox"]').click();`, `document.querySelectorAll('[data-testid=tree-row]').length > 20`, 30000),
      );
      await sleep(500);
      if (i < 3) {
        collapse.push(await timed(s, `document.querySelector('[data-testid=tree-row][data-path="Inbox"]').click();`, `document.querySelectorAll('[data-testid=tree-row]').length < 20`, 30000));
        await sleep(500);
      }
    }
    res.expandInboxMs = { runs: expand.map(r1), ...stats(expand) };
    res.collapseInboxMs = { runs: collapse.map(r1), ...stats(collapse) };
    res.renderedRows = await s.exec(`return document.querySelectorAll('[data-testid=tree-row]').length`);
    // scroll through the folder, 120 px per frame
    await s.exec(FRAMES_START);
    const steps = await s.execAsync(
      `const done = arguments[arguments.length - 1];
       const sc = document.querySelector('[data-testid=file-tree]'); sc.scrollTop = 0; let n = 0;
       const step = () => { sc.scrollTop += 120; if (++n < 600) requestAnimationFrame(step); else done({ n, top: sc.scrollTop, h: sc.scrollHeight }); };
       requestAnimationFrame(step);`,
    );
    const fr = await s.exec(FRAMES_STOP);
    res.scroll = { ...steps, frameIntervals: { ...stats(fr), over50: fr.filter((x) => x > 50).length, top5: [...fr].sort((a, b) => b - a).slice(0, 5).map(r1) } };
    // jump to the end and back
    res.jumpToEndMs = r1(await timed(s, `const sc = document.querySelector('[data-testid=file-tree]'); sc.scrollTop = sc.scrollHeight;`, "true"));
    res.jumpToTopMs = r1(await timed(s, `document.querySelector('[data-testid=file-tree]').scrollTop = 0;`, "true"));
    // open a note in the big folder (tree click)
    const row = await s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path).find(p => p && p.startsWith('Inbox/'))`);
    res.openNoteFromTreeMs = r1(
      await timed(s, `document.querySelector('[data-testid=tree-row][data-path="${row}"]').click();`, `document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === ${JSON.stringify(row)} && document.querySelector('.cm-line')`, 20000),
    );
    // new note in the expanded tree (tree refresh cost with 20k rows)
    // quick switcher
    const sw = { open: [], filter: [] };
    for (let i = 0; i < 4; i++) {
      sw.open.push(
        await timed(
          s,
          `window.dispatchEvent(new KeyboardEvent('keydown', { key: 'o', code: 'KeyO', ctrlKey: true, bubbles: true, cancelable: true }));`,
          `document.querySelectorAll('[data-testid=switcher-item]').length > 0`,
          10000,
        ),
      );
      let q = "";
      for (const ch of "garden 1234") {
        q += ch;
        sw.filter.push(await timed(s, setInput("switcher-input", q), "true"));
      }
      await s.exec(`document.querySelector('[data-testid=switcher-input]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })); return true`);
      await s.waitFor(`return !document.querySelector('[data-testid=switcher-input]')`);
      await sleep(200);
    }
    res.switcher = { open: { runs: sw.open.map(r1), ...stats(sw.open.slice(1)) }, filterPerKeystroke: stats(sw.filter) };
    res.mem = memTree(app.pid);
  } finally {
    save("tree-flat", res);
    await app.stop();
  }
});
