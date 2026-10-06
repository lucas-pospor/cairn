// Accessibility audit, part 2: screen-reader semantics (roles, accessible
// names, states, live regions) and colour contrast in the light and dark
// themes, measured in the real desktop app.
//
// Run:  scripts/e2e-headless.sh e2e/adv_a11y_semantics.test.mjs
// One:  scripts/e2e-headless.sh --test-name-pattern 'FINDING-203' e2e/adv_a11y_semantics.test.mjs
//
// Accessible names are computed by an approximation of the accname algorithm
// in adv_a11y_lib.mjs (aria-labelledby, aria-label, <label>, content for roles
// that take their name from content, title, then placeholder as a last
// resort). Contrast is computed from getComputedStyle after alpha compositing
// against the backgrounds of all ancestors.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { AxApp, K, SERVER, EVIDENCE, eventually, sleep, decodePng, regionContrast } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-sem-");

before(async () => {
  app.write(
    ".cairn/plugins/perm.js",
    "// @name Perm plugin\n// @description Asks for a permission.\n// @permissions read\ncairn.commands.register('x', 'Perm plugin command', async () => {});\n",
  );
  app.write(".cairn/snippets/one.css", ":root {}\n");
  await app.start();
});

after(async () => {
  await app.stop("semantics-final.png");
});

const log = (label, v) => console.log(`${label}:\n${typeof v === "string" ? v : JSON.stringify(v, null, 1)}`);

/** Interactive elements with no accessible name, and those named only by a placeholder. */
async function unnamed(rootCss = "body") {
  return app.exec(
    `const root = document.querySelector(arguments[0]);
     const all = __ax.audit(root);
     return { none: all.filter(a => !a.name).map(a => a.el), placeholderOnly: all.filter(a => a.name.startsWith('(placeholder)')).map(a => a.el + ' ' + a.name) };`,
    rootCss,
  );
}

async function openSettings(section) {
  await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await app.exec(`document.querySelector('[data-testid=settings-${section}]').click(); return 1`);
  await sleep(250);
}

// ---------------------------------------------------------------------------
// Held up.

test("semantics: dialogs from DialogHost are role=dialog + aria-modal + labelled; toasts sit in a polite live region; icons are aria-hidden", async () => {
  await app.reset();
  await app.palette("create new folder");
  await app.s.waitFor(`return !!document.querySelector('[role=dialog]')`);
  const d = await app.exec(`const d = document.querySelector('[role=dialog]'); return { modal: d.getAttribute('aria-modal'), label: d.getAttribute('aria-label') }`);
  assert.deepEqual(d, { modal: "true", label: "New folder" });
  await app.keys(K.esc);
  const t = await app.exec(`const t = document.querySelector('.toasts'); return t.getAttribute('aria-live')`);
  assert.equal(t, "polite");
  const visibleSvgs = await app.exec(`return [...document.querySelectorAll('svg')].filter(s => s.closest('button') && s.getAttribute('aria-hidden') !== 'true').length`);
  assert.equal(visibleSvgs, 0);
  // The view-mode buttons expose their state.
  await app.openNote("welcome", "Welcome.md");
  const modes = await app.exec(`return [...document.querySelectorAll('[data-testid^=mode-]')].map(b => b.getAttribute('aria-pressed'))`);
  assert.deepEqual(modes, ["true", "false", "false", "false"]);
});

test("semantics: Live Preview checkboxes are named ('To do' / 'Done')", async () => {
  await app.reset();
  app.write("Tasks.md", "# Tasks\n\n- [ ] open one\n- [x] closed one\n\nend\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Tasks.md"]')`);
  await app.openNote("tasks", "Tasks.md");
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length } }); return 1`);
  await app.s.waitFor(`return document.querySelectorAll('.cm-lp-task').length === 2`);
  const names = await app.exec(`return [...document.querySelectorAll('.cm-lp-task')].map(b => __ax.name(b))`);
  assert.deepEqual(names, ["To do", "Done"]);
});

// ---------------------------------------------------------------------------
// Findings: names and roles.

test("FINDING-114: every form control and composite widget has an accessible name", async () => {
  await app.reset();
  const report = {};
  // Main workspace with a note open, then the Search and Tags panels.
  await app.openNote("welcome", "Welcome.md");
  report.workspace = await unnamed();
  await app.exec(`document.querySelector('[data-testid=tab-search]').click(); return 1`);
  await sleep(150);
  report.searchPanel = await unnamed("aside.left");
  await app.exec(`document.querySelector('[data-testid=tab-tags]').click(); return 1`);
  await sleep(150);
  report.tagsPanel = await unnamed("aside.left");
  await app.exec(`document.querySelector('[data-testid=tab-files]').click(); return 1`);
  // Inline rename box.
  await app.palette("rename current");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=rename-input]')`);
  report.renameBox = await app.exec(`const i = document.querySelector('[data-testid=rename-input]'); return __ax.desc(i) + ' name=' + JSON.stringify(__ax.name(i))`);
  await app.keys(K.esc);
  // Prompt dialog.
  await app.palette("create new folder");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=dialog-input]')`);
  report.promptInput = await app.exec(`const i = document.querySelector('[data-testid=dialog-input]'); return __ax.desc(i) + ' name=' + JSON.stringify(__ax.name(i))`);
  await app.keys(K.esc);
  // Quick switcher and palette.
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return !!document.querySelector('.switcher')`);
  report.switcher = await unnamed(".switcher");
  await app.keys(K.esc);
  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return !!document.querySelector('.palette')`);
  report.palette = await unnamed(".palette");
  await app.keys(K.esc);
  // Every Settings section.
  for (const sec of ["appearance", "editor", "files", "sync", "plugins", "hotkeys"]) {
    await openSettings(sec);
    report[`settings/${sec}`] = await unnamed("[data-testid=settings]");
  }
  await app.keys(K.esc);
  // Graph view.
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view]')`);
  await sleep(300);
  report.graph = await unnamed("[data-testid=graph-view]");
  log("unnamed controls per view", report);

  const none = Object.entries(report).flatMap(([view, r]) =>
    typeof r === "string" ? (/name=""/.test(r) ? [`${view}: ${r}`] : []) : r.none.map((e) => `${view}: ${e}`),
  );
  assert.deepEqual(none, []);
});

test("FINDING-203: icon buttons repeated per row have distinct names (Close, Add hotkey, Remove)", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("ideas");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Ideas')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.tabs()).length === 2);
  const tabClose = await app.exec(`return [...document.querySelectorAll('[data-testid=tab] button.close')].map(b => __ax.name(b))`);
  await openSettings("hotkeys");
  const hk = await app.exec(`
    const names = [...document.querySelectorAll('[data-testid=hotkey-row] button')].map(b => __ax.name(b));
    const count = {}; for (const n of names) count[n] = (count[n] || 0) + 1;
    return count;`);
  await app.keys(K.esc);
  log("tab close button names", tabClose);
  log("hotkey row button names (name: count)", hk);
  const dup = Object.entries(hk).filter(([, c]) => c > 1).map(([n, c]) => `${c} hotkey buttons named "${n}"`);
  if (new Set(tabClose).size < tabClose.length) dup.push(`${tabClose.length} tab close buttons all named ${JSON.stringify(tabClose[0])}`);
  assert.deepEqual(dup, []);
});

test("FINDING-204: the selected panel / section exposes its state (aria-pressed, aria-selected or aria-current), not only its colour", async () => {
  await app.reset();
  await openSettings("editor");
  const r = await app.exec(`
    const st = (b) => ({ name: __ax.name(b), visualOn: b.classList.contains('on'), pressed: b.getAttribute('aria-pressed'), selected: b.getAttribute('aria-selected'), current: b.getAttribute('aria-current'), role: b.getAttribute('role') });
    return {
      left: ['tab-files', 'tab-search', 'tab-tags'].map(id => st(document.querySelector('[data-testid=' + id + ']'))),
      right: ['right-links', 'right-outline', 'right-properties'].map(id => st(document.querySelector('[data-testid=' + id + ']'))),
      settingsNav: [...document.querySelectorAll('[data-testid=settings] nav button')].map(st),
    };`);
  await app.keys(K.esc);
  log("panel switch buttons", r);
  const bad = [...r.left, ...r.right, ...r.settingsNav].filter((b) => b.visualOn && !b.pressed && !b.selected && !b.current).map((b) => b.name);
  assert.deepEqual(bad, [], `buttons that look selected but expose no state: ${bad.join(", ")}`);
});

test("FINDING-115: file tree semantics: a named tree, nested treeitems with a level, aria-selected on the row selected for F2/Delete", async () => {
  await app.reset();
  // Expand a folder with the mouse-equivalent click so there is a nested item.
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="Projects"]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Projects/Garden plan.md"]')`);
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="Ideas.md"]').click(); return 1`);
  await eventually(async () => (await app.activeTab()) === "Ideas.md");
  // Select (not open) another row: right click selects it.
  await app.exec(`const r = document.querySelector('[data-testid=tree-row][data-path="Welcome.md"]'); const b = r.getBoundingClientRect(); r.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: b.x + 20, clientY: b.y + 5 })); return 1`);
  await app.keys(K.esc);
  const r = await app.exec(`
    const t = document.querySelector('[role=tree]');
    const nested = document.querySelector('[data-testid=tree-row][data-path="Projects/Garden plan.md"]');
    return {
      treeName: __ax.name(t),
      nestedLevel: nested.getAttribute('aria-level'),
      nestedParentRole: nested.parentElement.getAttribute('role'),
      selectedRow: document.querySelector('.row.selected')?.dataset.path ?? null,
      ariaSelected: [...document.querySelectorAll('[role=treeitem][aria-selected=true]')].map(e => e.dataset.path),
    };`);
  log("tree semantics", r);
  const problems = [];
  if (!r.treeName) problems.push("role=tree has no accessible name");
  if (!r.nestedLevel && r.nestedParentRole !== "group") problems.push("nested treeitem has no aria-level and is not inside role=group: depth only shown by indentation");
  if (r.selectedRow && !r.ariaSelected.includes(r.selectedRow)) problems.push(`row selected for F2/Delete (${r.selectedRow}) is not aria-selected; aria-selected marks the open note ${JSON.stringify(r.ariaSelected)}`);
  assert.deepEqual(problems, []);
});

test("FINDING-116: quick switcher and palette expose the highlighted result (aria-activedescendant on the input)", async () => {
  await app.reset();
  const out = {};
  for (const [hot, inputId] of [["o", "switcher-input"], ["p", "palette-input"]]) {
    await app.chord(K.ctrl, hot);
    await app.s.waitFor(`return document.activeElement?.dataset.testid === '${inputId}'`);
    await app.keys(K.down, K.down);
    await sleep(100);
    out[inputId] = await app.exec(`
      const i = document.activeElement;
      const ad = i.getAttribute('aria-activedescendant');
      const sel = document.querySelector('[role=option][aria-selected=true]');
      return { focus: __ax.desc(i), role: i.getAttribute('role'), activedescendant: ad, controls: i.getAttribute('aria-controls'), highlighted: sel ? __ax.name(sel) : null, highlightedId: sel?.id || null, listboxName: __ax.name(document.querySelector('[role=listbox]')) };`);
    await app.keys(K.esc);
    await sleep(100);
  }
  log("switcher / palette", out);
  const bad = Object.entries(out).filter(([, v]) => !v.activedescendant || v.activedescendant !== v.highlightedId).map(([k, v]) => `${k}: focus stays on the input (role=${v.role}, aria-activedescendant=${v.activedescendant}) while "${v.highlighted}" is highlighted`);
  assert.deepEqual(bad, []);
});

// The save state is not expected to be live: it flips between Saved and
// Unsaved on every keystroke and autosave, and a save conflict is announced by
// the role=alert conflict banner (adv_verify_ax_19). Nor is the sync button
// itself: it says "Syncing…" a few seconds after every edit. The status bar
// must carry a live region for the sync changes that matter;
// adv_verify_ax_19 checks that a sync error reaches it.
test("FINDING-205: the status bar has a live region for sync status", async () => {
  await app.reset();
  const port = 19000 + Math.floor(Math.random() * 900);
  const url = `http://127.0.0.1:${port}`;
  const server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: "ax-token-0123456789", CAIRN_DATA: path.join(app.tmp, "server-sem"), CAIRN_ADDR: `127.0.0.1:${port}` },
    stdio: "ignore",
  });
  let r;
  try {
    await eventually(async () => (await fetch(`${url}/health`)).ok, { message: "server up" });
    const res = await app.s.execAsync(
      `const done = arguments[arguments.length - 1];
       (async () => {
         document.querySelector('[data-testid=open-settings]').click();
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
    assert.equal(res, "connected");
    await app.keys(K.esc);
    await app.openNote("ideas", "Ideas.md");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=sync-indicator]')`);
    r = await app.exec(`
      const live = (el) => { for (let e = el; e; e = e.parentElement) { if (e.getAttribute('aria-live') && e.getAttribute('aria-live') !== 'off') return true; if (['status', 'alert', 'log'].includes(e.getAttribute('role'))) return true; } return false; };
      const sync = document.querySelector('[data-testid=sync-indicator]');
      const save = document.querySelector('[data-testid=save-state]');
      const region = sync.closest('footer').querySelector('[role=status], [role=alert], [aria-live=polite], [aria-live=assertive]');
      return { syncText: sync.textContent.trim(), syncLive: live(sync) || (!!region && !region.contains(save)), saveText: save.textContent.trim(), saveLive: live(save) };`);
  } finally {
    await app.exec(`return window.__TAURI_INTERNALS__.invoke('sync_disconnect').then(() => 1, () => 0)`).catch(() => {});
    server.kill();
  }
  log("status bar", r);
  const bad = [];
  if (!r.syncLive) bad.push(`sync status "${r.syncText}" (changes to Syncing… / Sync error) is not in a live region`);
  assert.deepEqual(bad, []);
});

test("FINDING-122: the Delete confirm exposes its message through aria-describedby", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await app.palette("delete current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`);
  const r = await app.exec(`
    const d = document.querySelector('[role=dialog]');
    const ids = (d.getAttribute('aria-describedby') || '').split(/\\s+/).filter(Boolean);
    return { name: __ax.name(d), describedby: ids.map(id => document.getElementById(id)?.textContent ?? null), message: d.querySelector('p')?.textContent, focused: __ax.desc(document.activeElement) };`);
  await app.keys(K.esc);
  log("delete confirm", r);
  assert.ok(app.exists("Ideas.md"));
  assert.ok(r.describedby.some((t) => t && t.includes("Ideas")), `a screen reader hears "${r.name} dialog, ${r.focused}" but not "${r.message}"`);
});

test("FINDING-213: workspace structure: a main landmark, labelled sidebars, headings outside the note and a document title that names the note", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  const r = await app.exec(`
    const named = (el) => __ax.name(el) || null;
    return {
      main: document.querySelectorAll('main, [role=main]').length,
      asides: [...document.querySelectorAll('aside')].map(a => ({ cls: a.className.replace(/svelte-\\S+/g, '').trim(), name: a.getAttribute('aria-label') || a.getAttribute('aria-labelledby') })),
      headingsOutsideNote: [...document.querySelectorAll('h1,h2,h3,h4,h5,h6,[role=heading]')].filter(h => !h.closest('.cm-editor, [data-testid=preview]')).map(h => h.textContent.trim()),
      sectionLabels: ['Files', 'Backlinks', 'Outgoing links'].map(t => { const e = [...document.querySelectorAll('span')].find(s => s.textContent.trim() === t); return e ? e.tagName.toLowerCase() + (e.closest('h1,h2,h3,h4,h5,h6,[role=heading]') ? ' (heading)' : ' (plain text)') : null; }),
      title: document.title,
    };`);
  log("workspace structure", r);
  const bad = [];
  if (!r.main) bad.push("no <main> / role=main around the editor");
  for (const a of r.asides) if (!a.name) bad.push(`<aside class="${a.cls}"> has no label (two unnamed 'complementary' landmarks)`);
  if (!r.headingsOutsideNote.length) bad.push(`no headings outside the note: panel titles are ${JSON.stringify(r.sectionLabels)}`);
  if (!/Ideas/.test(r.title)) bad.push(`document title is "${r.title}" with Ideas.md open`);
  assert.deepEqual(bad, []);
});

test("FINDING-114, FINDING-213: the editor's name and the window title follow the open note", async () => {
  await app.reset();
  const now = () =>
    app.exec(`return window.__TAURI_INTERNALS__.invoke('plugin:window|title', { label: 'main' }).then(
      (win) => ({ editor: document.querySelector('.cm-content').getAttribute('aria-label'), doc: document.title, win }),
      (e) => ({ error: String(e) }))`);
  await app.openNote("welcome", "Welcome.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("ideas");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Ideas')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.activeTab()) === "Ideas.md");
  await sleep(200);
  const seen = { ideas: await now() };
  // Ctrl+Tab from inside the editor: the view takes the other note's state.
  await app.s.waitFor(`return !!document.activeElement?.closest('.cm-content')`, { message: "editor focused" });
  await app.chord(K.ctrl, K.tab);
  await eventually(async () => (await app.activeTab()) === "Welcome.md");
  await sleep(200);
  seen.welcome = await now();
  log("editor name and titles", seen);
  assert.deepEqual(seen, {
    ideas: { editor: "Ideas", doc: "Ideas - Cairn", win: "Ideas - Cairn" },
    welcome: { editor: "Welcome", doc: "Welcome - Cairn", win: "Welcome - Cairn" },
  });
});

test("FINDING-107: quick switcher, palette and Settings are modal dialogs (aria-modal=true)", async () => {
  await app.reset();
  const out = {};
  for (const [label, open, css] of [
    ["quick switcher", async () => app.chord(K.ctrl, "o"), ".switcher"],
    ["command palette", async () => app.chord(K.ctrl, "p"), ".palette"],
    ["settings", async () => app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`), "[data-testid=settings]"],
  ]) {
    await open();
    await app.s.waitFor(`return !!document.querySelector('${css}')`);
    out[label] = await app.exec(`const d = document.querySelector(arguments[0]); return { role: d.getAttribute('role'), modal: d.getAttribute('aria-modal'), name: __ax.name(d) }`, css);
    await app.keys(K.esc);
    await sleep(150);
  }
  log("overlay dialog semantics", out);
  const bad = Object.entries(out).filter(([, v]) => v.modal !== "true").map(([k, v]) => `${k}: role=${v.role} aria-modal=${v.modal}`);
  assert.deepEqual(bad, []);
});

// ---------------------------------------------------------------------------
// Findings: contrast.

/** Audit text and icon contrast of everything visible, grouped by colour pair. */
async function contrastReport() {
  return app.exec(`
    const out = {};
    for (const c of __ax.contrast(document.body)) {
      if (c.pass) continue;
      const k = c.fg + ' on ' + c.bg + ' = ' + c.ratio + ':1 (needs ' + c.need + ')';
      (out[k] ||= new Set()).add(c.el.replace(/^(\\w+)\\./, '$1.') + ' "' + c.text + '"');
    }
    for (const c of __ax.icons(document.body)) {
      if (c.pass) continue;
      const k = 'icon ' + c.fg + ' on ' + c.bg + ' = ' + c.ratio + ':1 (needs 3)';
      (out[k] ||= new Set()).add(c.el + (c.name ? ' "' + c.name + '"' : ''));
    }
    return Object.fromEntries(Object.entries(out).map(([k, v]) => [k, [...v].slice(0, 6)]));`);
}

async function contrastScenes(theme) {
  await app.reset();
  await app.setTheme(theme);
  const scenes = {};
  // A code block with every syntax colour: keyword, comment, type (class name), function, number, string.
  const code = "```js\n// a comment\nclass Box { size = 42; grow(n) { return \"big\"; } }\n```\n\n";
  app.write("Contrast.md", "---\ntags: [prop]\n---\n# Contrast\n\nA #tag and [[Missing note]] and [[Ideas]] and `code` and [url](https://example.com).\n\n- [ ] task\n\n> quote\n\n" + code + "line\n".repeat(3));
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Contrast.md"]')`);
  await app.openNote("contrast", "Contrast.md");
  await app.s.waitFor(`return !!document.querySelector('.cm-content .tok-comment')`, { message: "the code block is highlighted" });
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="Projects"]').click(); return 1`);
  await sleep(400);
  scenes.workspace = await contrastReport();
  await app.shot(`AX-contrast-${theme}-workspace.png`);
  // Search results: the matched words are highlighted inside muted snippet text.
  await app.exec(`document.querySelector('[data-testid=tab-search]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=search-input]')`);
  await app.exec(`const i = document.querySelector('[data-testid=search-input]'); i.value = 'quote'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=search-results] mark.hit')`, { message: "search hits shown" });
  scenes.search = await contrastReport();
  await app.shot(`AX-contrast-${theme}-search.png`);
  await app.exec(`const i = document.querySelector('[data-testid=search-input]'); i.value = ''; i.dispatchEvent(new Event('input', { bubbles: true })); document.querySelector('[data-testid=tab-files]').click(); return 1`);
  await app.exec(`document.querySelector('[data-testid=right-properties]').click(); document.querySelector('[data-testid=tab-tags]').click(); return 1`);
  await sleep(300);
  scenes.tagsAndProperties = await contrastReport();
  await app.exec(`document.querySelector('[data-testid=right-links]').click(); document.querySelector('[data-testid=tab-files]').click(); return 1`);
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] a')`);
  await sleep(300);
  scenes.readingView = await contrastReport();
  await app.chord(K.ctrl, "e");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return !!document.querySelector('.switcher')`);
  await sleep(150);
  scenes.switcher = await contrastReport();
  await app.keys(K.esc);
  await app.palette("delete current");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=dialog-ok]')`);
  scenes.deleteConfirm = await contrastReport();
  await app.shot(`AX-contrast-${theme}-delete-confirm.png`);
  await app.keys(K.esc);
  await openSettings("hotkeys");
  scenes.settingsHotkeys = await contrastReport();
  await app.keys(K.esc);
  return scenes;
}

const summarize = (scenes) =>
  Object.entries(scenes).flatMap(([scene, r]) => Object.entries(r).map(([k, v]) => `[${scene}] ${k}: ${v.join(" | ")}`));

test("FINDING-117: light theme: muted, faint, tag, unresolved-link and placeholder text meet WCAG AA", async () => {
  const scenes = await contrastScenes("light");
  const lines = summarize(scenes);
  log("light theme contrast failures", lines.join("\n"));
  fs.writeFileSync(path.join(EVIDENCE, "contrast-light.txt"), lines.join("\n") + "\n");
  assert.deepEqual(lines, []);
});

test("FINDING-206: dark theme: text on the red Delete button and faint text meet WCAG AA", async () => {
  const scenes = await contrastScenes("dark");
  const lines = summarize(scenes);
  log("dark theme contrast failures", lines.join("\n"));
  fs.writeFileSync(path.join(EVIDENCE, "contrast-dark.txt"), lines.join("\n") + "\n");
  await app.setTheme("light");
  assert.deepEqual(lines, []);
});

/** In-page helper: contrast of the colour in `css` (a colour, or a box-shadow) against what is behind `el`. */
const EDGE = `const edge = (el, css) => { const m = css.match(/rgba?\\([^)]*\\)/); const bg = __ax.bgOf(el); return m ? Math.round(__ax.ratio(__ax.over(__ax.parse(m[0]), bg), bg) * 100) / 100 : 0; };`;

test("FINDING-207: non-text contrast: input borders and the selected-row outline reach 3:1, placeholders 4.5:1", async () => {
  await app.reset();
  app.write("Empty.md", "");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Empty.md"]')`);
  const out = {};
  await app.openNote("empty", "Empty.md");
  for (const theme of ["light", "dark"]) {
    await app.setTheme(theme);
    await app.s.waitFor(`return !!document.querySelector('.cm-placeholder')`, { message: "editor placeholder shown" });
    // A click selects a folder row (only the open note's row is "active").
    await app.exec(`document.querySelector('[data-testid=tree-row][data-path="Projects"]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Projects"].selected')`, { message: "folder row selected" });
    const selectedRowOutline = await app.exec(`${EDGE} const row = document.querySelector('[data-testid=tree-row][data-path="Projects"]'); return edge(row, getComputedStyle(row).boxShadow)`);
    await app.exec(`document.querySelector('[data-testid=tab-search]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=search-input]')`);
    await sleep(300);
    out[theme] = await app.exec(`
      ${EDGE}
      const v = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
      const r = (fg, bg) => __ax.pair(fg, bg).ratio;
      const input = document.querySelector('[data-testid=search-input]');
      const pe = document.querySelector('.cm-placeholder');
      const pbg = __ax.bgOf(pe), pfg = __ax.over(__ax.parse(getComputedStyle(pe).color), pbg);
      const ph = { fg: __ax.hex(pfg), bg: __ax.hex(pbg), ratio: Math.round(__ax.ratio(pfg, pbg) * 100) / 100, text: pe.textContent };
      return {
        dpr: devicePixelRatio,
        textInputBorder: edge(input, getComputedStyle(input).borderTopColor),
        selectedRowOutline: arguments[0],
        activeRowBackground: r(v('--bg-active'), v('--bg-side')),
        editorPlaceholder: ph ? ph.fg + ' on ' + ph.bg + ' = ' + ph.ratio : null,
        editorPlaceholderRatio: ph?.ratio ?? null,
      };`, selectedRowOutline);
    // The search box placeholder: getComputedStyle cannot read ::placeholder in
    // WebKit, so measure the pixels of a screenshot.
    const rect = await app.rectOf("[data-testid=search-input]");
    const png = await app.s.screenshot();
    fs.writeFileSync(path.join(EVIDENCE, `AX-22-placeholder-${theme}.png`), png);
    const img = decodePng(png);
    const k = out[theme].dpr;
    out[theme].searchPlaceholder = regionContrast(img, (rect.x + 9) * k, (rect.y + 6) * k, (rect.x + 120) * k, (rect.y + rect.h - 6) * k);
    await app.exec(`document.querySelector('[data-testid=tab-files]').click(); return 1`);
  }
  await app.setTheme("light");
  log("non-text and placeholder contrast", out);
  const bad = [];
  for (const [theme, o] of Object.entries(out)) {
    if (o.textInputBorder < 3) bad.push(`${theme}: text input border ${o.textInputBorder}:1 (needs 3)`);
    if (o.selectedRowOutline < 3) bad.push(`${theme}: outline of the tree row selected for F2/Delete ${o.selectedRowOutline}:1 (needs 3)`);
    if (o.editorPlaceholderRatio < 4.5) bad.push(`${theme}: editor placeholder "Start writing…" ${o.editorPlaceholder}:1`);
    if (o.searchPlaceholder.ratio < 4.5) bad.push(`${theme}: "Search notes" placeholder ${o.searchPlaceholder.fg} on ${o.searchPlaceholder.bg} = ${o.searchPlaceholder.ratio}:1 (screenshot)`);
  }
  assert.deepEqual(bad, []);
});
