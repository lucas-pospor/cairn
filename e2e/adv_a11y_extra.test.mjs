// Accessibility audit, part 4: hotkey settings, link following, focus after
// in-panel actions, widget structure, toasts, custom accent colours, extra
// contrast states and sync status, plus the held-up checks that go with them.
//
// Run:  scripts/e2e-headless.sh e2e/adv_a11y_extra.test.mjs
// One:  scripts/e2e-headless.sh --test-name-pattern 'FINDING-214' e2e/adv_a11y_extra.test.mjs
//
// Same harness as the other adv_a11y files (adv_a11y_lib.mjs): one app on a
// throwaway vault with temp XDG dirs; every test starts with app.reset().
// The two tests that need a sync server run last and turn sync off again.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { AxApp, K, SERVER, EVIDENCE, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-extra-");
const ALT = "";

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop("extra-final.png");
});

const log = (label, v) => console.log(`${label}:\n${typeof v === "string" ? v : JSON.stringify(v, null, 1)}`);

async function openSettings(section) {
  await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await app.exec(`document.querySelector('[data-testid=settings-${section}]').click(); return 1`);
  await sleep(250);
}

async function closeSettings() {
  await app.exec(`document.querySelector('[data-testid=settings] .close')?.click(); return 1`);
  await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
}

/** Settings are saved 300 ms after a change; wait for the file so a page reload cannot drop the write. */
async function settingsSaved(pred, message) {
  await eventually(() => {
    try {
      return pred(JSON.parse(app.read(".cairn/settings.json")));
    } catch {
      return false;
    }
  }, { message });
}

async function restoreHotkeyDefaults() {
  if (!(await app.exec(`return !!document.querySelector('[data-testid=settings]')`))) await openSettings("hotkeys");
  else await app.exec(`document.querySelector('[data-testid=settings-hotkeys]').click(); return 1`);
  await sleep(200);
  await app.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Restore all defaults')?.click(); return 1`);
  await sleep(200);
  await closeSettings();
  if (app.exists(".cairn/settings.json")) await settingsSaved((j) => Object.keys(j.hotkeys ?? {}).length === 0, "default hotkeys saved");
}

async function addFile(rel, content) {
  app.write(rel, content);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path=${JSON.stringify(rel)}]')`, { message: `${rel} in the tree` });
}

async function freePort() {
  return new Promise((resolve) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
}

/** Start a cairn-server on a free port; returns { url, proc }. */
async function startServer(label) {
  const port = await freePort();
  const url = `http://127.0.0.1:${port}`;
  const proc = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: "ax-token-0123456789", CAIRN_DATA: path.join(app.tmp, `server-${label}`), CAIRN_ADDR: `127.0.0.1:${port}` },
    stdio: "ignore",
  });
  await eventually(async () => (await fetch(`${url}/health`)).ok, { message: "server up" });
  return { url, proc };
}

/** Fill and submit the sync form (setup is not what is under test). */
async function connectSync(url) {
  return app.s.execAsync(
    `const done = arguments[arguments.length - 1];
     (async () => {
       if (!document.querySelector('[data-testid=settings]')) document.querySelector('[data-testid=open-settings]').click();
       await new Promise(r => setTimeout(r, 200));
       document.querySelector('[data-testid=settings-sync]').click();
       await new Promise(r => setTimeout(r, 300));
       const set = (id, v) => { const i = document.querySelector('[data-testid=' + id + ']'); i.value = v; i.dispatchEvent(new Event('input', { bubbles: true })); };
       set('sync-server', arguments[0]); set('sync-token', 'ax-token-0123456789'); set('sync-pass', 'passphrase123'); set('sync-pass2', 'passphrase123');
       document.querySelector('[data-testid=sync-connect]').click();
       for (let i = 0; i < 150; i++) {
         await new Promise(r => setTimeout(r, 100));
         document.querySelector('[data-testid=dialog-ok]')?.click(); // a new vault: "Create it?"
         const e = document.querySelector('[data-testid=sync-error]');
         if (e) return 'error: ' + e.textContent;
         if (document.querySelector('[data-testid=sync-state]')) return 'connected';
       }
       return 'timeout';
     })().then(done, e => done('ERR ' + e));`,
    url,
  );
}

const disconnectSync = () => app.exec(`return window.__TAURI_INTERNALS__.invoke('sync_disconnect').then(() => 1, () => 0)`).catch(() => {});

// ---------------------------------------------------------------------------
// Held up.

test("held up: Escape in the inline rename box cancels (the blur that follows does not commit the typed name)", async () => {
  await app.reset();
  await addFile("Keep name.md", "keep\n");
  await app.openNote("keep name", "Keep name.md");
  await app.palette("rename current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`);
  await app.keys("changed name", K.esc);
  await sleep(800);
  assert.ok(app.exists("Keep name.md"), "original name kept");
  assert.ok(!app.exists("changed name.md"), "typed name not applied");
});

test("held up: with a tree row selected, the Delete key in the editor deletes text and never reaches the tree's delete handler", async () => {
  await app.reset();
  await addFile("Del key.md", "abc\n");
  // Select the row with the mouse (sets the tree's selection), then work elsewhere.
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="Del key.md"]').click(); return 1`);
  await eventually(async () => (await app.activeTab()) === "Del key.md");
  await eventually(() => app.focusInEditor(), { message: "editor focused" });
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: 0 } }); return 1`);
  await app.keys(K.del);
  await sleep(300);
  assert.equal(await app.exec(`return !!document.querySelector('[role=dialog]')`), false, "no delete confirm from the editor");
  await eventually(() => app.read("Del key.md") === "bc\n", { message: "Delete edited the text" });
});

test("held up: typing after Ctrl+E (reading view) does not edit the hidden note", async () => {
  await app.reset();
  await addFile("Reading typing.md", "# Reading\n\nbody\n");
  await app.openNote("reading typing", "Reading typing.md");
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] h1')`);
  await sleep(200);
  await app.keys("qq");
  await sleep(1200);
  assert.equal(app.read("Reading typing.md"), "# Reading\n\nbody\n");
  await app.chord(K.ctrl, "e");
});

// ---------------------------------------------------------------------------
// Findings.

test("FINDING-123: with no note open and in reading view, Settings > Hotkeys still lists every command it lists with a note open", async () => {
  await app.reset();
  const listed = async () => {
    await openSettings("hotkeys");
    const ids = await app.exec(`return [...document.querySelectorAll('[data-testid=hotkey-row]')].map(r => r.dataset.command)`);
    await closeSettings();
    return ids;
  };
  const noNote = await listed();
  await app.openNote("welcome", "Welcome.md");
  const withNote = await listed();
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview]')`);
  const reading = await listed();
  await app.chord(K.ctrl, "e");
  const missingNoNote = withNote.filter((id) => !noNote.includes(id));
  const missingReading = withNote.filter((id) => !reading.includes(id));
  log("hotkey rows", { noNote: noNote.length, withNote: withNote.length, reading: reading.length, missingNoNote, missingReading });
  assert.deepEqual(
    { missingNoNote, missingReading },
    { missingNoNote: [], missingReading: [] },
    `Settings > Hotkeys lists ${noNote.length} commands with no note open, ${withNote.length} with a note open, ${reading.length} in reading view`,
  );
});

test("FINDING-214: the hotkey recorder does not bind bare keys: Tab or Q pressed while recording binds nothing, so Tab still moves focus and q can still be typed", async () => {
  await app.reset();
  await addFile("Bare keys.md", "start\n");
  const problems = [];
  const combos = (id) => app.exec(`return [...document.querySelectorAll('[data-command="${id}"] .combo')].map(c => c.firstChild.textContent.trim())`);
  try {
    await openSettings("hotkeys");
    // A keyboard user activates "+" and then presses Tab to move on.
    await app.exec(`document.querySelector('[data-command="app:toggle-right"] [data-testid=hotkey-add]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-command="app:toggle-right"] .recording')`);
    await app.keys(K.tab);
    await sleep(200);
    const right = await combos("app:toggle-right");
    if (right.includes("Tab")) problems.push(`pressing Tab while recording bound bare "Tab" to "Toggle right sidebar" (no warning): ${JSON.stringify(right)}`);
    await app.exec(`document.querySelector('[data-command="app:toggle-left"] [data-testid=hotkey-add]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-command="app:toggle-left"] .recording')`);
    await app.keys("q");
    await sleep(200);
    const left = await combos("app:toggle-left");
    if (left.includes("Q")) problems.push(`pressing q while recording bound bare "Q" to "Toggle left sidebar": ${JSON.stringify(left)}`);
    const saved = await eventually(() => {
      try {
        return JSON.parse(app.read(".cairn/settings.json")).hotkeys;
      } catch {
        return null;
      }
    }, { message: "settings.json written" }).catch(() => null);
    await app.keys(K.esc);
    await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
    // Tab in the workspace.
    await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
    const rightBefore = await app.exec(`return !document.querySelector('aside.right').classList.contains('hidden')`);
    await app.keys(K.tab);
    await sleep(200);
    const st = await app.exec(`return { focus: __ax.desc(document.activeElement), right: !document.querySelector('aside.right').classList.contains('hidden') }`);
    if (/file-tree/.test(st.focus)) problems.push(`Tab from the file tree no longer moves focus (still on ${st.focus}); it toggled the right sidebar ${rightBefore} -> ${st.right}`);
    // Typing q in a note.
    await app.openNote("bare keys", "Bare keys.md");
    await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length } }); v.focus(); return 1`);
    await app.keys("aqa");
    await sleep(1200);
    const disk = app.read("Bare keys.md");
    if (!disk.includes("aqa")) problems.push(`typing "aqa" in the note saved ${JSON.stringify(disk)} (the q ran "Toggle left sidebar")`);
    log("saved hotkeys in .cairn/settings.json", saved);
    await app.shot("AX-36-bare-keys.png");
  } finally {
    await app.keys(K.esc).catch(() => {});
    await restoreHotkeyDefaults();
  }
  assert.deepEqual(problems, []);
});

test("FINDING-124: a link can be followed from the editor with the keyboard: Alt+Enter or Ctrl+Enter opens it and the palette has a 'follow link' command", async () => {
  await app.reset();
  await addFile("Linker.md", "Go to [[Ideas]] now.\n\nsecond line\n");
  const problems = [];
  const tries = {};
  for (const [label, chord] of [
    ["Alt+Enter", [ALT, K.enter]],
    ["Ctrl+Enter", [K.ctrl, K.enter]],
  ]) {
    if ((await app.activeTab()) !== "Linker.md") await app.openNote("linker", "Linker.md");
    await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; const i = v.state.doc.toString().indexOf('Ideas') + 2; v.dispatch({ selection: { anchor: i } }); v.focus(); return 1`);
    await app.chord(...chord);
    await sleep(600);
    tries[label] = await app.activeTab();
  }
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=palette-item]').length > 5`);
  const linkCmds = await app.exec(`return [...document.querySelectorAll('[data-testid=palette-item]')].map(b => b.firstElementChild.textContent).filter(n => /link/i.test(n))`);
  await app.keys(K.esc);
  log("active tab after each key with the cursor inside [[Ideas]]", tries);
  log("palette commands mentioning links", linkCmds);
  if (!Object.values(tries).includes("Ideas.md")) problems.push(`with the cursor inside [[Ideas]], ${Object.keys(tries).join(" and ")} leave ${JSON.stringify(tries)} active`);
  if (!linkCmds.some((n) => /follow|open/i.test(n))) problems.push(`palette commands about links: ${JSON.stringify(linkCmds)} (no "follow/open link under cursor")`);
  assert.deepEqual(problems, []);
});

test("FINDING-109: focus is not dropped to <body> after in-panel keyboard actions (Enter on a tag in the Tags panel, Delete on an inactive tab and on the last tab)", async () => {
  await app.reset();
  const out = [];
  // Tags panel: Enter on a tag switches the sidebar to Search.
  await app.exec(`document.querySelector('[data-testid=tab-tags]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tag-row]')`);
  await app.exec(`document.querySelector('[data-testid=tag-row]').focus(); return 1`);
  await app.keys(K.enter);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=search-input]')`);
  await sleep(300);
  out.push({ action: "Enter on a tag in the Tags panel (switches to Search)", focus: await app.focus() });
  await app.exec(`document.querySelector('[data-testid=tab-files]').click(); return 1`);
  // Tab bar: close the inactive tab, then the last tab, with Delete on the focused tab
  // (the keyboard way to close a tab; the close mark is not a focus stop, FINDING-216).
  await app.openNote("ideas", "Ideas.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("garden");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Garden')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.tabs()).length === 2);
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Ideas.md"]').focus(); return 1`);
  await app.keys(K.del);
  await eventually(async () => (await app.tabs()).length === 1, { message: "inactive tab closed" });
  await sleep(300);
  out.push({ action: "Delete on an inactive tab", focus: await app.focus() });
  await app.exec(`document.querySelector('[data-testid=tab]').focus(); return 1`);
  await app.keys(K.del);
  await eventually(async () => (await app.tabs()).length === 0, { message: "last tab closed" });
  await sleep(300);
  out.push({ action: "Delete on the last tab", focus: await app.focus() });
  log("focus after each action", out);
  const lost = out.filter((o) => o.focus === "BODY");
  assert.deepEqual(lost, []);
});

test("FINDING-109: Settings > Hotkeys: adding or removing a hotkey from the keyboard does not drop focus to <body>", async () => {
  await app.reset();
  const out = [];
  try {
    await openSettings("hotkeys");
    await app.exec(`document.querySelector('[data-command="app:graph"] [data-testid=hotkey-add]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.querySelector('[data-command="app:graph"] .recording')`, { message: "recording" });
    out.push({ action: "Enter on '+' (recording starts)", focus: await app.focus() });
    await app.chord(K.ctrl, ALT, "y");
    await app.s.waitFor(`return document.querySelector('[data-command="app:graph"]').textContent.includes('Alt+Y')`, { message: "combo recorded" });
    out.push({ action: "Ctrl+Alt+Y recorded", focus: await app.focus() });
    await app.exec(`[...document.querySelectorAll('[data-command="app:graph"] .combo')].find(c => c.textContent.includes('Alt+Y')).querySelector('button').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !document.querySelector('[data-command="app:graph"]').textContent.includes('Alt+Y')`, { message: "combo removed" });
    out.push({ action: "Enter on the combo's remove button", focus: await app.focus() });
  } finally {
    await restoreHotkeyDefaults();
  }
  log("focus while editing hotkeys with the keyboard", out);
  assert.deepEqual(out.filter((o) => o.focus === "BODY"), []);
});

test("FINDING-205: search progress and result counts are announced (a live region around 'Searching…' / 'N results')", async () => {
  await app.reset();
  await app.chord(K.ctrl, K.shift, "f");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'search-input'`, { message: "search focused" });
  await app.keys("garden");
  await app.s.waitFor(`return /[1-9]\\d* results/.test(document.querySelector('.search .meta')?.textContent ?? '')`, { message: "result count shown" });
  const r = await app.exec(`
    const m = document.querySelector('.search .meta');
    let live = false;
    for (let e = m; e; e = e.parentElement) if ((e.getAttribute('aria-live') && e.getAttribute('aria-live') !== 'off') || ['status', 'alert', 'log'].includes(e.getAttribute('role'))) live = true;
    return { text: m.textContent.trim(), live };`);
  log("search meta", r);
  assert.ok(r.live, `"${r.text}" appears silently; focus stays in the search box`);
});

test("FINDING-222: the search panel does not say 'No matches.' or '0 results · 0 ms' while the search is pending, nor '1 results' for one hit", async () => {
  await app.reset();
  await app.chord(K.ctrl, K.shift, "f");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'search-input'`, { message: "search focused" });
  // Watch the panel from inside the page while typing a query that matches.
  await app.exec(`
    window.__axSeen = [];
    const rec = () => { const t = (document.querySelector('.search .meta')?.textContent ?? '') + ' | ' + (document.querySelector('[data-testid=search-results] .none')?.textContent ?? ''); if (window.__axSeen[window.__axSeen.length - 1] !== t) window.__axSeen.push(t); };
    window.__axObs = new MutationObserver(rec); window.__axObs.observe(document.querySelector('[data-testid=left-sidebar]'), { subtree: true, childList: true, characterData: true }); return 1`);
  await app.keys("tomatoes");
  // "tomatoes" has one hit: wait for "1 result" as well as "N results".
  await app.s.waitFor(`return /[1-9]\\d* results?\\b/.test(document.querySelector('.search .meta')?.textContent ?? '')`, { message: "result count shown" });
  const seen = await app.exec(`window.__axObs.disconnect(); return window.__axSeen`);
  log("search panel states while typing 'tomatoes' (a word in Projects/Garden plan.md)", seen);
  const wrong = seen.filter((t) => /No matches|^0 results|\b1 results\b/.test(t));
  assert.deepEqual(wrong, [], "the panel claimed there were no matches before the search had returned, or said '1 results'");
});

test("FINDING-125: the interface text can be enlarged: Ctrl+= / Ctrl++ zoom the whole interface and Ctrl+0 resets it", async () => {
  await app.reset();
  const sizes = () =>
    app.exec(`
      const fs = (css) => { const e = document.querySelector(css); return e ? parseFloat(getComputedStyle(e).fontSize) * (window.visualViewport?.scale ?? 1) : null; };
      return { zoom: Math.round(devicePixelRatio * 100) / 100, treeRow: fs('[data-testid=tree-row] .name'), filesHeader: fs('.header .title'), statusBar: fs('.status'), editor: fs('.cm-content'), treeRowHeight: document.querySelector('[data-testid=tree-row]').getBoundingClientRect().height };`);
  await app.openNote("welcome", "Welcome.md");
  const before = await sizes();
  await app.chord(K.ctrl, "=");
  await app.chord(K.ctrl, K.shift, "=");
  await app.chord(K.ctrl, "+");
  await sleep(300);
  const afterZoomKeys = await sizes();
  await app.chord(K.ctrl, "0");
  await sleep(300);
  const afterReset = await sizes();
  let afterSetting;
  try {
    await openSettings("appearance");
    await app.exec(`const r = document.querySelector('[data-testid=settings] input[type=range]'); r.value = 24; r.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
    await sleep(300);
    await closeSettings();
    afterSetting = await sizes();
  } finally {
    await openSettings("appearance");
    await app.exec(`const r = document.querySelector('[data-testid=settings] input[type=range]'); r.value = 16; r.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
    await sleep(200);
    await closeSettings();
    await settingsSaved((j) => j.fontSize === 16, "font size restored");
  }
  log("UI text sizes (px)", { before, afterZoomKeys, afterCtrl0: afterReset, afterFontSize24: afterSetting });
  const bad = [];
  const zoomed = afterZoomKeys.zoom > before.zoom || afterZoomKeys.treeRow > before.treeRow;
  if (!zoomed) bad.push(`Ctrl+= / Ctrl+Shift+= / Ctrl++ change nothing: ${JSON.stringify(before)}`);
  else if (afterReset.zoom !== before.zoom) bad.push(`Ctrl+0 leaves the zoom at ${afterReset.zoom}`);
  // One way to scale the whole interface is enough (WCAG 1.4.4): with the
  // zoom keys working, Font size may stay the note's text size.
  if (!zoomed && afterSetting.treeRow === before.treeRow && afterSetting.statusBar === before.statusBar) bad.push(`Font size 24 makes the note ${afterSetting.editor}px but leaves the tree at ${afterSetting.treeRow}px, the Files header at ${afterSetting.filesHeader}px and the status bar at ${afterSetting.statusBar}px`);
  assert.deepEqual(bad, []);
});

test("FINDING-221: pointer targets (tab close, hotkey remove, status-bar buttons) are at least 24x24 px", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  const main = await app.exec(`
    const out = [];
    // The tab's close mark counts too: a pointer-only target, hidden from
    // screen readers (Delete closes a focused tab).
    for (const el of document.querySelectorAll('button, [role=tab], [role=treeitem], input[type=checkbox], a[href], [data-testid=tab] .close')) {
      if ((__ax.hidden(el) && !el.matches('.close')) || el.closest('.cm-content, [data-testid=preview]')) continue;
      const r = el.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) continue;
      if (r.width < 24 || r.height < 24) out.push(__ax.desc(el) + ' ' + Math.round(r.width) + 'x' + Math.round(r.height));
    }
    return [...new Set(out)];`);
  await openSettings("hotkeys");
  const hk = await app.exec(`const b = document.querySelector('.combo button'); const r = b.getBoundingClientRect(); return r.width < 24 || r.height < 24 ? [__ax.desc(b) + ' ' + Math.round(r.width) + 'x' + Math.round(r.height)] : []`);
  await closeSettings();
  log("targets under 24x24", [...main, ...hk]);
  assert.deepEqual([...main, ...hk], []);
});

// The switcher and palette inputs (outline:none, no border) are left out: they
// are auto-focused text fields whose blinking caret shows focus.
test("held up: every control in Settings, the switcher, the palette and the prompt dialog shows a focus indicator", async () => {
  await app.reset();
  const sweep = (rootCss) =>
    app.exec(
      `const out = [];
       for (const el of document.querySelector(arguments[0]).querySelectorAll('button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])')) {
         if (__ax.hidden(el)) continue;
         const r = el.getBoundingClientRect(); if (r.width < 1) continue;
         const f = __ax.focusStyle(el);
         if (!f.indicator || f.opacity === 0) out.push(__ax.desc(el) + (f.opacity === 0 ? ' (opacity 0)' : ' (no focus style)'));
       }
       return out;`,
      rootCss,
    );
  const out = {};
  for (const sec of ["appearance", "editor", "files", "sync", "plugins", "hotkeys"]) {
    await openSettings(sec);
    out[`settings/${sec}`] = await sweep("[data-testid=settings]");
  }
  await closeSettings();
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return !!document.querySelector('.switcher')`);
  out.switcher = await sweep(".switcher");
  await app.keys(K.esc);
  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return !!document.querySelector('.palette')`);
  out.palette = await sweep(".palette");
  await app.keys(K.esc);
  await app.palette("create new folder");
  await app.s.waitFor(`return !!document.querySelector('[role=dialog]')`);
  out.prompt = await sweep("[role=dialog]");
  await app.keys(K.esc);
  const caretOnly = [];
  for (const k of Object.keys(out)) {
    caretOnly.push(...out[k].filter((e) => /^input(\[role=combobox\])?\[data-testid=(switcher|palette)-input\]/.test(e)));
    out[k] = out[k].filter((e) => !/^input(\[role=combobox\])?\[data-testid=(switcher|palette)-input\]/.test(e));
    if (!out[k].length) delete out[k];
  }
  log("text inputs whose only focus indicator is the caret", caretOnly);
  assert.deepEqual(out, {});
});

test("FINDING-104: with no note open, the first Tab after Ctrl+, lands in Settings and Tab never leaves it", async () => {
  await app.reset();
  await app.chord(K.ctrl, ",");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  const start = await app.focus();
  const visited = [];
  for (let i = 0; i < 40; i++) {
    await app.keys(K.tab);
    visited.push(await app.focus());
    if (await app.exec(`return !!document.activeElement?.closest('[data-testid=settings]')`)) break;
  }
  // Settings is modal: Tab cycles through its controls and never leaves it.
  const left = [];
  for (let i = 0; i < 30; i++) {
    await app.keys(K.tab);
    if (!(await app.exec(`return !!document.activeElement?.closest('[data-testid=settings]')`))) left.push(await app.focus());
  }
  await app.keys(K.esc);
  log("Tab stops after Ctrl+, (no note open)", [start, ...visited]);
  assert.ok(visited.length <= 1, `focus started on ${start}; ${visited.length} Tab presses to reach the first Settings control, through: ${visited.slice(0, -1).join(" -> ")}`);
  assert.deepEqual(left, [], "Tab moved focus out of Settings");
});

test("FINDING-216: composite widgets hold only children with the right role (no buttons inside role=tab, only tabs in the tablist, only options in the switcher listbox) and the tabs are one Tab stop", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("garden");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Garden')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.tabs()).length === 2);
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("ide");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=switcher-create]')`);
  const r = await app.exec(`
    const tabs = [...document.querySelectorAll('[role=tab]')];
    return {
      tabsWithFocusableChild: tabs.filter(t => t.querySelector('button, a[href], [tabindex]:not([tabindex="-1"])')).map(t => __ax.desc(t)),
      tablistChildrenNotTab: [...document.querySelector('[role=tablist]').children].filter(c => c.getAttribute('role') !== 'tab').map(c => __ax.desc(c)),
      tabStops: tabs.filter(t => t.tabIndex >= 0).length,
      tabCount: tabs.length,
      listboxChildrenNotOption: [...document.querySelector('.switcher [role=listbox]').children].filter(c => c.getAttribute('role') !== 'option').map(c => __ax.desc(c)),
    };`);
  await app.keys(K.esc);
  log("widget structure", r);
  const bad = [];
  if (r.tabsWithFocusableChild.length) bad.push(`role=tab elements containing a focusable button: ${r.tabsWithFocusableChild.join(", ")}`);
  if (r.tablistChildrenNotTab.length) bad.push(`tablist children that are not tabs: ${r.tablistChildrenNotTab.join(", ")}`);
  if (r.tabStops > 1) bad.push(`${r.tabStops} of ${r.tabCount} tabs are separate Tab stops (no roving tabindex)`);
  if (r.listboxChildrenNotOption.length) bad.push(`listbox children that are not options: ${r.listboxChildrenNotOption.join(", ")}`);
  assert.deepEqual(bad, []);
});

test("FINDING-217: 'Sync now' with sync not set up opens Settings on the Sync page", async () => {
  await app.reset();
  await app.palette("sync now");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`, { message: "settings opened" });
  const st = await app.exec(`return { heading: document.querySelector('[data-testid=settings] section h3')?.textContent, toasts: [...document.querySelectorAll('.toast')].map(t => t.textContent) }`);
  await app.shot("AX-40-sync-now-unconfigured.png");
  await closeSettings();
  log("after 'Sync now' with no sync configured", st);
  assert.equal(st.heading, "Sync", `Settings opened on "${st.heading}" with toasts ${JSON.stringify(st.toasts)}`);
});

test("FINDING-218: an error toast stays while hovered and has a close button that removes it", async () => {
  await app.reset();
  await addFile("Toast me.md", "toast\n");
  await app.openNote("toast me", "Toast me.md");
  await app.palette("rename current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`);
  await app.exec(`const i = document.activeElement; i.value = 'nofolder/new name'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
  await app.keys(K.enter);
  await app.s.waitFor(`return !!document.querySelector('.toast.error')`, { message: "error toast" });
  const t0 = Date.now();
  const info = await app.exec(`const t = document.querySelector('.toast.error'); const r = t.getBoundingClientRect(); return { text: t.textContent, buttons: t.querySelectorAll('button').length, x: r.x + r.width / 2, y: r.y + r.height / 2 }`);
  // Keep the pointer on the toast, as someone reading it would.
  await app.s.pointer([{ type: "pointerMove", x: Math.round(info.x), y: Math.round(info.y), duration: 0 }]);
  // A toast that stays is the expected outcome, so a timeout here is not a failure.
  const removed = await app.s.waitFor(`return !document.querySelector('.toast.error')`, { timeout: 15000, message: "toast removed" }).then(() => true, () => false);
  const shownMs = Date.now() - t0;
  // Its close button takes it away.
  if (!removed) await app.exec(`document.querySelector('.toast.error button')?.click(); return 1`);
  const closed = removed || (await app.exec(`return !document.querySelector('.toast.error')`));
  log("error toast", { ...info, shownMs, removed, closed });
  const bad = [];
  if (info.buttons === 0) bad.push(`the toast "${info.text}" has no close/keep button`);
  if (removed) bad.push(`it disappeared after ${Math.round(shownMs / 100) / 10} s although the pointer was resting on it`);
  else if (!closed) bad.push("its close button did not remove it");
  assert.deepEqual(bad, []);
});

test("FINDING-219: with a custom accent colour, links, tag chips and button text still meet AA contrast", async () => {
  await app.reset();
  const out = {};
  const setTheme = (v) => app.exec(`const s = document.querySelector('[data-testid=theme-select]'); s.value = arguments[0]; s.dispatchEvent(new Event('change', { bubbles: true })); return 1`, v);
  try {
    await openSettings("appearance");
    // Settings > Theme = Light (applying settings re-applies the theme, so data-theme alone would not stick).
    await setTheme("light");
    for (const accent of ["#3b82f6", "#e8a33d"]) {
      await app.exec(`const i = document.querySelector('[data-testid=settings] input[type=color]'); i.value = arguments[0]; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`, accent);
      await sleep(200);
      out[accent] = await app.exec(`return { primaryButtonText: __ax.pair('var(--accent-text)', 'var(--accent)'), linkOnPage: __ax.pair('var(--link)', 'var(--bg)'), tagChip: __ax.pair('var(--accent)', 'var(--accent-soft)') }`);
    }
  } finally {
    await app.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Default')?.click(); return 1`).catch(() => {});
    await setTheme("system").catch(() => {});
    await closeSettings().catch(() => {});
    await settingsSaved((j) => j.theme === "system" && !j.accent, "theme and accent restored");
  }
  log("contrast with a custom accent (light theme)", out);
  const bad = [];
  for (const [accent, r] of Object.entries(out)) for (const [k, v] of Object.entries(r)) if (v.ratio < 4.5) bad.push(`${accent}: ${k} ${v.fg} on ${v.bg} = ${v.ratio}:1`);
  assert.deepEqual(bad, []);
});

test("FINDING-220: the status-bar vault button is named after its action (switch or close the vault)", async () => {
  await app.reset();
  const r = await app.exec(`const b = document.querySelector('.status .vault'); return { name: __ax.name(b), title: b.title }`);
  log("status bar vault button", r);
  assert.match(r.name, /switch|close/i, `a screen reader announces "${r.name}, button"; the action ("${r.title}") is only the tooltip, and pressing it closes the vault without asking`);
});

/** Contrast failures of everything visible, grouped by colour pair. */
async function contrastFailures() {
  return app.exec(`
    const out = {};
    for (const c of __ax.contrast(document.body)) {
      if (c.pass) continue;
      const k = c.fg + ' on ' + c.bg + ' = ' + c.ratio + ':1 (needs ' + c.need + ')';
      (out[k] ||= new Set()).add(c.el + ' "' + c.text + '"');
    }
    return Object.fromEntries(Object.entries(out).map(([k, v]) => [k, [...v].slice(0, 5)]));`);
}

async function stateScenes(theme, name) {
  await app.reset();
  await app.setTheme(theme, name);
  const scenes = {};
  // Selected palette row (shortcut hint on the highlight) and the switcher's
  // "Create note" row when nothing matches (accent text on the highlight).
  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'palette-input'`);
  scenes.paletteSelected = await app.exec(`const r = document.querySelector('[data-testid=palette-item].sel kbd'); return r ? __ax.contrast(r).map(c => c.fg + ' on ' + c.bg + ' = ' + c.ratio + ' "' + c.text + '"') : null`);
  await app.keys(K.esc);
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("qqxx");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=switcher-create].sel')`);
  scenes.switcherCreate = await app.exec(`return __ax.contrast(document.querySelector('[data-testid=switcher-create]')).filter(c => !c.pass).map(c => c.fg + ' on ' + c.bg + ' = ' + c.ratio + ' "' + c.text + '"')`);
  scenes.switcherHints = await app.exec(`return __ax.contrast(document.querySelector('.switcher .hints')).filter(c => !c.pass).map(c => c.fg + ' on ' + c.bg + ' = ' + c.ratio + ' "' + c.text + '"').slice(0, 3)`);
  await app.keys(K.esc);
  // Conflict banner, plus an error toast.
  const label = name ?? theme;
  await addFile(`Conflict ${label}.md`, "mine or theirs\n");
  await app.openNote(`conflict ${label}`, `Conflict ${label}.md`);
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length } }); v.focus(); return 1`);
  await app.keys(" mine");
  app.write(`Conflict ${label}.md`, "theirs\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=conflict-banner]')`, { timeout: 6000, message: "conflict banner" });
  scenes.conflictBanner = await app.exec(`return __ax.contrast(document.querySelector('[data-testid=conflict-banner]')).map(c => c.fg + ' on ' + c.bg + ' = ' + c.ratio + (c.pass ? '' : ' FAIL') + ' "' + c.text + '"')`);
  scenes.saveStateConflict = await app.exec(`return __ax.contrast(document.querySelector('[data-testid=save-state]')).map(c => c.fg + ' on ' + c.bg + ' = ' + c.ratio + (c.pass ? '' : ' FAIL') + ' "' + c.text + '"')`);
  await app.shot(`AX-contrast-${label}-conflict.png`);
  await app.exec(`document.querySelector('[data-testid=conflict-theirs]').click(); return 1`);
  // Context menu with the red Delete item.
  await app.exec(`const r = document.querySelector('[data-testid=tree-row][data-path="Welcome.md"]'); const b = r.getBoundingClientRect(); r.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: b.x + 20, clientY: b.y + 5 })); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[role=menu]')`);
  scenes.contextMenu = await app.exec(`return __ax.contrast(document.querySelector('[role=menu]')).filter(c => !c.pass).map(c => c.fg + ' on ' + c.bg + ' = ' + c.ratio + ' "' + c.text + '"')`);
  await app.keys(K.esc);
  return scenes;
}

test("FINDING-117: light theme, more states: highlighted palette shortcut, switcher 'Create note' row and hint bar, conflict banner and context menu meet AA", async () => {
  const scenes = await stateScenes("light");
  log("light theme, extra states", scenes);
  const bad = Object.entries(scenes).flatMap(([k, v]) => (v ?? []).filter((l) => /= (\d+(\.\d+)?)/.test(l) && Number(l.match(/= (\d+(\.\d+)?)/)[1]) < 4.5).map((l) => `${k}: ${l}`));
  assert.deepEqual(bad, []);
});

test("FINDING-206: dark theme, more states: highlighted palette shortcut, switcher hints, conflict banner and context menu meet AA", async () => {
  const scenes = await stateScenes("dark");
  await app.setTheme("light");
  log("dark theme, extra states", scenes);
  const bad = Object.entries(scenes).flatMap(([k, v]) => (v ?? []).filter((l) => /= (\d+(\.\d+)?)/.test(l) && Number(l.match(/= (\d+(\.\d+)?)/)[1]) < 4.5).map((l) => `${k}: ${l}`));
  assert.deepEqual(bad, []);
});

for (const [theme, name] of [["light", "marble"], ["dark", "graphite"]]) {
  test(`${name} theme, more states: highlighted palette shortcut, switcher hints, conflict banner and context menu meet AA`, async () => {
    const scenes = await stateScenes(theme, name);
    await app.setTheme("light");
    log(`${name} theme, extra states`, scenes);
    const bad = Object.entries(scenes).flatMap(([k, v]) => (v ?? []).filter((l) => /= (\d+(\.\d+)?)/.test(l) && Number(l.match(/= (\d+(\.\d+)?)/)[1]) < 4.5).map((l) => `${k}: ${l}`));
    assert.deepEqual(bad, []);
  });
}

// ---------------------------------------------------------------------------
// Need a sync server (last).

// Version history opened from the palette leaves no live selection in the
// editor, so typed keys go nowhere. (Settings opened with Ctrl+, and the
// graph view used to let them through: FINDING-104, FINDING-008.)
test("held up: typing while Version history (opened from the palette) is shown does not edit the note behind it", async () => {
  await app.reset();
  const { url, proc } = await startServer("history");
  let r;
  try {
    assert.equal(await connectSync(url), "connected");
    await closeSettings();
    await addFile("History typing.md", "# History\n\nbody\n");
    await app.openNote("history typing", "History typing.md");
    await app.palette("version history");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=history]')`, { message: "history open" });
    await sleep(300);
    const focus = await app.focus();
    await app.keys("zz");
    await sleep(1300);
    r = { focus, disk: app.read("History typing.md") };
    await app.shot("AX-31-typing-behind-history.png");
    await app.exec(`document.querySelector('[data-testid=history] header .icon-btn').click(); return 1`);
  } finally {
    await disconnectSync();
    proc.kill();
  }
  log("typing with Version history open", r);
  assert.equal(r.disk, "# History\n\nbody\n", `focus was on ${r.focus}; "zz" typed with the modal open was saved into the note`);
});

// Version history opened while the cursor is in the note (here from the file
// tree's context menu, clicked from script so focus stays in the editor, as
// with a hotkey bound to the command).
test("FINDING-008, FINDING-105: with Version history open over the editor, typing does not edit the note and hotkeys do not run", async () => {
  await app.reset();
  const { url, proc } = await startServer("history-keys");
  const problems = [];
  try {
    assert.equal(await connectSync(url), "connected");
    await closeSettings();
    await addFile("History keys.md", "# History\n\nbody\n");
    await app.openNote("history keys", "History keys.md");
    await app.exec(`
      const r = document.querySelector('[data-testid=tree-row][data-path="History keys.md"]');
      const b = r.getBoundingClientRect();
      r.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: b.x + 20, clientY: b.y + 5 }));
      return 1`);
    await app.s.waitFor(`return !!document.querySelector('[role=menu]')`);
    await app.exec(`[...document.querySelectorAll('[role=menuitem]')].find(b => b.textContent.trim() === 'Version history').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=history]')`, { message: "history open" });
    await sleep(300);
    const focus = await app.focus();
    await app.keys("zz", "\uE003"); // Backspace
    await app.chord(K.ctrl, "o");
    await app.chord(K.ctrl, "w");
    await sleep(1300);
    const disk = app.read("History keys.md");
    if (disk !== "# History\n\nbody\n") problems.push(`focus on ${focus}; keys typed with Version history open were saved into the note: ${JSON.stringify(disk)}`);
    if (await app.exec(`return !!document.querySelector('[data-testid=switcher-input]')`)) problems.push("Ctrl+O opened the quick switcher behind Version history");
    const tabs = await app.tabs();
    if (!tabs.includes("History keys.md")) problems.push(`Ctrl+W closed the note behind Version history: ${JSON.stringify(tabs)}`);
    await app.shot("AX-history-keys.png");
    await app.exec(`document.querySelector('[data-testid=history] header .icon-btn')?.click(); return 1`);
  } finally {
    await disconnectSync();
    proc.kill();
  }
  assert.deepEqual(problems, []);
});

test("FINDING-215: a failed sync shows its reason in the page and in plain words, not only in a hover tooltip", async () => {
  await app.reset();
  const { url, proc } = await startServer("err");
  let r;
  try {
    assert.equal(await connectSync(url), "connected");
    await closeSettings();
    proc.kill();
    await eventually(async () => {
      try {
        await fetch(`${url}/health`);
        return false;
      } catch {
        return true;
      }
    }, { message: "server stopped" });
    await app.exec(`document.querySelector('[data-testid=sync-indicator]').click(); return 1`);
    await app.s.waitFor(`return document.querySelector('[data-testid=sync-indicator]')?.textContent.includes('Sync error')`, { timeout: 20000, message: "sync error shown" });
    r = await app.exec(`
      const b = document.querySelector('[data-testid=sync-indicator]');
      const reason = b.title;
      return { visible: b.textContent.trim(), name: __ax.name(b), tooltip: reason, reasonVisibleInPage: document.body.innerText.includes(reason), toasts: [...document.querySelectorAll('.toast')].map(t => t.textContent) };`);
    await app.shot("AX-38-sync-error.png");
  } finally {
    await disconnectSync();
    proc.kill();
  }
  log("status bar after a failed sync", r);
  const bad = [];
  if (!r.reasonVisibleInPage) bad.push(`the status bar says "${r.visible}"; the reason "${r.tooltip}" is only in the title tooltip (no toast: ${JSON.stringify(r.toasts)})`);
  if (/\(os error \d+\)|\bio: |\bhttp: /.test(r.tooltip)) bad.push(`and the reason is a raw client error: "${r.tooltip}"`);
  assert.deepEqual(bad, []);
});
