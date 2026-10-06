// End-to-end tests against the real Cairn binary through tauri-driver.
// Build first:  cd app && npx tauri build --debug --no-bundle
// Run:          node --test e2e/

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";
import { pickThemeMode } from "./theme_mode.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-e2e-"));
const vault = path.join(tmp, "vault");
const shots = path.join(import.meta.dirname, ".tmp");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
const read = (rel) => fs.readFileSync(path.join(vault, rel), "utf8");
const exists = (rel) => fs.existsSync(path.join(vault, rel));

async function eventually(fn, { timeout = 5000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let err;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      err = e;
    }
    await sleep(80);
  }
  throw new Error(`timed out: ${message}${err ? ` (${err.message})` : ""}`);
}

write("Welcome.md", "# Welcome\n\nLinks: [[Ideas]], [[Projects/Garden plan|the garden]] and [[Not yet written]].\n");
write("Ideas.md", "---\ntags: [brainstorm]\n---\n# Ideas\n\nSee [[Welcome]].\n");
write("Projects/Garden plan.md", "# Garden plan\n\nTomatoes and basil. Back to [[Welcome]].\n");
write("Journal/2026-10-03.md", "Daily note about [[Ideas]].\n");
write("Archive/.keep", "");
write(".cairn/plugins/probe.js", `// @name Probe
// @description Tests the sandbox.
// @permissions editor
cairn.commands.register("stamp", "Insert stamp", async () => {
  const sel = await cairn.editor.getSelection();
  await cairn.editor.replaceSelection("[stamp:" + sel.length + "]");
  const r = [];
  r.push(typeof document === "undefined" ? "no-dom" : "dom");
  r.push(typeof self.__TAURI_INTERNALS__ === "undefined" ? "no-tauri" : "tauri");
  try { await fetch("https://example.com/"); r.push("net-open"); } catch { r.push("net-blocked"); }
  for (const url of ["ipc://localhost/read_note", "http://ipc.localhost/read_note"]) {
    try {
      const res = await fetch(url, { method: "POST", body: JSON.stringify({ path: "Welcome.md" }), headers: { "Content-Type": "application/json" } });
      const text = await res.text();
      r.push(text.includes("Welcome") ? "ipc-READ" : "ipc-" + res.status);
    } catch { r.push("ipc-blocked"); }
  }
  try { await cairn.notes.read("Welcome.md"); r.push("read-allowed"); } catch { r.push("read-denied"); }
  await cairn.ui.toast("probe:" + r.join(","));
});
`);
write("Tagged.md", "---\ntags: [project]\nowner: Sam\n---\n# Tagged\n\n## Section one\n\ntext #urgent\n\n## Section two\n");
write("Live.md", "---\ntags: [demo]\n---\n# Live\n\nSome **bold** text and [[Ideas|an alias]].\n\n- [ ] open task\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n![[pic.png]]\n\n![[Projects/Garden plan]]\n\nlast line\n");
fs.copyFileSync(path.resolve(import.meta.dirname, "../app/src-tauri/icons/32x32.png"), path.join(vault, "pic.png"));

let drv, s;

before(async () => {
  fs.mkdirSync(shots, { recursive: true });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 4`, { timeout: 15000 });
});

after(async () => {
  if (s) fs.writeFileSync(path.join(shots, "final.png"), await s.screenshot());
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const treePaths = () => s.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map(e => e.dataset.path)`);
const activeTab = () => s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
const editorText = () => s.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);

async function openFromTree(p) {
  await s.click(await s.find(rowSel(p)));
  await eventually(async () => (await activeTab()) === p, { message: `tab ${p} active` });
  await s.waitFor(`return document.querySelector('.cm-content')?.textContent.length >= 0`);
}

async function focusEditorEnd() {
  await s.exec(`
    const v = document.querySelector('.cm-editor').__cairnView;
    v.focus();
    v.dispatch({ selection: { anchor: v.state.doc.length } });
  `);
}

test("lists the vault in the file tree, folders first, hidden files skipped", async () => {
  assert.deepEqual(await treePaths(), ["Archive", "Journal", "Projects", "Ideas.md", "Live.md", "pic.png", "Tagged.md", "Welcome.md"]);
});

test("expands a folder and opens a note in a tab", async () => {
  await s.click(await s.find(rowSel("Projects")));
  await s.waitFor(`return !!document.querySelector('${rowSel("Projects/Garden plan.md")}')`);
  await openFromTree("Welcome.md");
  assert.match(await editorText(), /Links: \[\[Ideas\]\]/);
});

test("live preview hides syntax and renders widgets", async () => {
  await openFromTree("Live.md");
  await s.click(await s.find("[data-testid=mode-live]"));
  // Move the cursor to the last line so the other lines render.
  await s.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } });`);
  await s.waitFor(`return !!document.querySelector('.cm-lp-props')`);
  const text = await s.exec(`return document.querySelector('.cm-content').textContent`);
  assert.ok(text.includes("Some bold text and an alias."), text);
  assert.ok(!text.includes("**"), "emphasis marks hidden");
  assert.ok(await s.exec(`return !!document.querySelector('.cm-lp-table table')`), "table widget");
  const src = await s.exec(`return document.querySelector('.cm-lp-image-block img')?.getAttribute('src')`);
  assert.equal(src, "vault://localhost/pic.png");
  assert.ok(await s.waitFor(`return document.querySelector('.cm-lp-embed .embed-body')?.textContent.includes('Tomatoes')`), "note embed");
  assert.ok(await s.waitFor(`return document.querySelector('.cm-lp-image-block img')?.naturalWidth === 32`), "image loaded through vault://");
});

test("live preview checkboxes toggle the file", async () => {
  await s.click(await s.find(".cm-lp-task"));
  await eventually(() => read("Live.md").includes("- [x] open task"), { message: "task checked on disk" });
});

test("live preview link click opens the note", async () => {
  await s.click(await s.find(".cm-lp-wikilink"));
  await eventually(async () => (await activeTab()) === "Ideas.md", { message: "Ideas opened" });
  await openFromTree("Live.md");
  await s.click(await s.find("[data-testid=mode-source]"));
  const raw = await s.exec(`return document.querySelector('.cm-content').textContent`);
  assert.ok(raw.includes("**bold**"), "source mode shows syntax");
  await s.click(await s.find("[data-testid=mode-live]"));
});

test("autosaves edits to disk", async () => {
  await openFromTree("Welcome.md");
  await s.click(await s.find("[data-testid=mode-source]"));
  await focusEditorEnd();
  await s.keys(Key.enter, "Added from the e2e test.");
  await eventually(() => read("Welcome.md").includes("Added from the e2e test."), { message: "file saved" });
  await s.waitFor(`return document.querySelector('[data-testid=save-state]').textContent.trim() === 'Saved'`);
});

test("backlinks panel lists linking notes", async () => {
  await s.waitFor(`return document.querySelectorAll('[data-testid=backlink-source]').length === 2`);
  const srcs = await s.exec(`return [...document.querySelectorAll('[data-testid=backlink-source]')].map(e => e.textContent.trim())`);
  assert.ok(srcs.some((t) => t.startsWith("Garden plan")));
  assert.ok(srcs.some((t) => t.startsWith("Ideas")));
});

test("wikilink autocomplete inserts a link", async () => {
  await focusEditorEnd();
  await s.keys(Key.enter, "See [[Gard");
  await s.waitFor(`return !!document.querySelector('.cm-tooltip-autocomplete li')`);
  const first = await s.exec(`return document.querySelector('.cm-tooltip-autocomplete li').textContent`);
  assert.match(first, /Garden plan/);
  await sleep(150); // CodeMirror ignores Enter right after the popup opens
  await s.keys(Key.enter, " done");
  await eventually(async () => (await editorText()).includes("See [[Garden plan]] done"), {
    message: "completion applied",
  }).catch(async (e) => {
    throw new Error(e.message + "\n" + JSON.stringify(await editorText()));
  });
  await eventually(() => read("Welcome.md").includes("See [[Garden plan]] done"), { message: "saved" });
});

test("ctrl+click on a wikilink opens the target note", async () => {
  await s.exec(`
    const el = [...document.querySelectorAll('.cm-wikilink')].find(e => e.textContent === '[[Ideas]]');
    const r = el.getBoundingClientRect();
    el.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, clientX: r.left + 10, clientY: r.top + r.height / 2, ctrlKey: true, button: 0 }));
  `);
  await eventually(async () => (await activeTab()) === "Ideas.md", { message: "Ideas opened" });
});

test("preview renders links; clicking a broken link creates the note", async () => {
  await openFromTree("Welcome.md");
  await s.click(await s.find("[data-testid=mode-preview]"));
  await s.waitFor(`return !!document.querySelector('[data-testid=preview] a.internal-link.is-unresolved')`);
  const resolved = await s.exec(`return [...document.querySelectorAll('[data-testid=preview] a.internal-link:not(.is-unresolved)')].map(a => a.textContent)`);
  assert.deepEqual(resolved.slice(0, 2), ["Ideas", "the garden"]);
  await s.click(await s.find("[data-testid=preview] a.is-unresolved"));
  await eventually(() => exists("Not yet written.md"), { message: "note created from link" });
  await eventually(async () => (await activeTab()) === "Not yet written.md", { message: "new note opened" });
  // back to source mode for later tests
  await openFromTree("Welcome.md");
  await s.click(await s.find("[data-testid=mode-source]"));
});

test("quick switcher finds and opens a note", async () => {
  await s.keys({ chord: [Key.ctrl, "o"] });
  const input = await s.find("[data-testid=switcher-input]");
  await s.type(input, "garden");
  await s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Garden plan')`);
  await s.keys(Key.enter);
  await eventually(async () => (await activeTab()) === "Projects/Garden plan.md", { message: "garden opened" });
});

test("quick switcher creates a note with shift+enter", async () => {
  await s.keys({ chord: [Key.ctrl, "o"] });
  await s.type(await s.find("[data-testid=switcher-input]"), "Fresh idea");
  await s.keys({ chord: [Key.shift, Key.enter] });
  await eventually(() => exists("Fresh idea.md"), { message: "created via switcher" });
});

test("full-text search shows snippets", async () => {
  await s.click(await s.find("[data-testid=tab-search]"));
  try {
    await s.type(await s.find("[data-testid=search-input]"), "basil");
    await s.waitFor(`return document.querySelectorAll('[data-testid=search-hit]').length === 1`);
    const hit = await s.exec(`return document.querySelector('[data-testid=search-results] mark.hit')?.textContent`);
    assert.equal(hit, "basil");
  } finally {
    await s.click(await s.find("[data-testid=tab-files]"));
  }
});

test("new note, then inline rename", async () => {
  await s.click(await s.find("[data-testid=new-note]"));
  const input = await eventually(() => s.find("[data-testid=rename-input]"), { message: "rename input" });
  await eventually(() => exists("Untitled.md"), { message: "untitled created" });
  await s.exec(`document.querySelector('[data-testid=rename-input]').select()`);
  await s.type(input, "Renamed note");
  await s.keys(Key.enter);
  await eventually(() => exists("Renamed note.md") && !exists("Untitled.md"), { message: "renamed on disk" });
  await eventually(async () => (await activeTab()) === "Renamed note.md", { message: "tab follows rename" });
});

test("new folder via dialog", async () => {
  await s.click(await s.find("[data-testid=new-folder]"));
  await s.type(await s.find("[data-testid=dialog-input]"), "Inbox");
  await s.click(await s.findWait("[data-testid=dialog-ok]"));
  await eventually(() => fs.statSync(path.join(vault, "Inbox")).isDirectory(), { message: "folder created" });
});

test("drag and drop moves a note into a folder", async () => {
  await s.waitFor(`return !!document.querySelector('${rowSel("Inbox")}')`);
  const from = await s.find(rowSel("Fresh idea.md"));
  const to = await s.find(rowSel("Inbox"));
  await s.pointer([
    { type: "pointerMove", origin: { [ "element-6066-11e4-a52e-4f735466cecf" ]: from }, x: 0, y: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerMove", origin: "pointer", x: 0, y: 12, duration: 100 },
    { type: "pointerMove", origin: { [ "element-6066-11e4-a52e-4f735466cecf" ]: to }, x: 0, y: 0, duration: 200 },
    { type: "pointerUp", button: 0 },
  ]);
  await eventually(() => exists("Inbox/Fresh idea.md") && !exists("Fresh idea.md"), { message: "moved on disk" });
});

test("delete asks for confirmation and removes the file", async () => {
  await s.click(await s.find(rowSel("Renamed note.md")));
  await sleep(100);
  await s.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
  await s.keys(Key.delete);
  await s.click(await s.findWait("[data-testid=dialog-ok]"));
  await eventually(() => !exists("Renamed note.md"), { message: "deleted" });
  await s.waitFor(`return !document.querySelector('${rowSel("Renamed note.md")}')`);
});

test("reflects files created, changed and deleted outside the app", async () => {
  write("External.md", "made outside");
  await s.waitFor(`return !!document.querySelector('${rowSel("External.md")}')`, { timeout: 5000 });
  await openFromTree("External.md");
  write("External.md", "changed outside too");
  await eventually(async () => (await editorText()) === "changed outside too", { message: "editor reloaded" });
  fs.rmSync(path.join(vault, "External.md"));
  await s.waitFor(`return !document.querySelector('${rowSel("External.md")}')`);
  await eventually(async () => (await activeTab()) !== "External.md", { message: "tab closed" });
});

test("external rename keeps the tab open on the new path", async () => {
  await openFromTree("Ideas.md");
  fs.renameSync(path.join(vault, "Ideas.md"), path.join(vault, "Ideas v2.md"));
  await eventually(async () => (await activeTab()) === "Ideas v2.md", { message: "tab followed rename" });
});

test("conflicting edits show a banner instead of overwriting", async () => {
  await s.click(await s.find(rowSel("Journal")));
  await s.waitFor(`return !!document.querySelector('${rowSel("Journal/2026-10-03.md")}')`);
  await openFromTree("Journal/2026-10-03.md");
  await focusEditorEnd();
  await s.keys(" mine");
  write("Journal/2026-10-03.md", "Theirs, written elsewhere.\n");
  await s.waitFor(`return !!document.querySelector('[data-testid=conflict-banner]')`, { timeout: 6000 });
  assert.equal(read("Journal/2026-10-03.md"), "Theirs, written elsewhere.\n");
  await s.click(await s.find("[data-testid=conflict-mine]"));
  await eventually(() => read("Journal/2026-10-03.md").includes(" mine"), { message: "kept mine" });
});

test("command palette runs commands", async () => {
  await openFromTree("Tagged.md");
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.find("[data-testid=palette-input]"), "reading view");
  await s.waitFor(`return document.querySelector('[data-testid=palette-item]')?.textContent.includes('Toggle reading view')`);
  await s.keys(Key.enter);
  await s.waitFor(`return !!document.querySelector('[data-testid=preview] h1')`);
  await s.keys({ chord: [Key.ctrl, "e"] });
  await s.waitFor(`return !document.querySelector('[data-testid=preview]')`);
});

test("formatting hotkeys edit the note", async () => {
  await focusEditorEnd();
  await s.keys("make me bold");
  await s.exec(`const v = document.querySelector('.cm-editor').__cairnView; const end = v.state.doc.length; v.dispatch({ selection: { anchor: end - 4, head: end } });`);
  await s.keys({ chord: [Key.ctrl, "b"] });
  await eventually(() => read("Tagged.md").includes("make me **bold**"), { message: "bold saved" }).catch(async (e) => {
    throw new Error(`${e.message}\nfile: ${JSON.stringify(read("Tagged.md"))}\neditor: ${JSON.stringify(await editorText())}\nfocus: ${await s.exec("return document.activeElement?.className")}`);
  });
  await s.keys({ chord: [Key.ctrl, "l"] });
  await eventually(() => read("Tagged.md").includes("- [ ] make me **bold**"), { message: "task toggled" });
});

test("hotkeys can be rebound and are saved in the vault", async () => {
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.find("[data-testid=settings-hotkeys]"));
  await s.click(await s.find('[data-command="app:graph"] [data-testid=hotkey-add]'));
  await s.keys({ chord: [Key.ctrl, Key.shift, "y"] });
  await eventually(() => {
    const cfg = JSON.parse(read(".cairn/settings.json"));
    return cfg.hotkeys?.["app:graph"]?.includes("Mod+Shift+Y");
  }, { message: "hotkey saved" });
  await s.keys(Key.escape);
  await s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
  await s.keys({ chord: [Key.ctrl, Key.shift, "y"] });
  await s.waitFor(`return !!document.querySelector('[data-testid=graph-view]')`);
});

test("graph view shows notes and links", async () => {
  await s.waitFor(`return /notes/.test(document.querySelector('[data-testid=graph-stats]')?.textContent)`, { timeout: 8000 });
  const order = await s.exec(`return document.querySelector('[data-testid=graph-view] .canvas').__graph.order`);
  const notes = fs.readdirSync(vault, { recursive: true }).filter((f) => String(f).endsWith(".md") && !String(f).startsWith(".")).length;
  assert.equal(order, notes);
  await s.click(await s.find("[data-testid=tab][aria-selected=true] .close"));
});

test("settings: theme and CSS snippets apply", async () => {
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.find("[data-testid=settings-appearance]"));
  const sel = await s.find("[data-testid=theme-select]");
  await s.exec(pickThemeMode("light"));
  await s.waitFor(`return document.documentElement.dataset.theme === 'light'`);
  void sel;
  // Below the theme rows, the button can be cut off at the bottom of Settings,
  // and WebDriver does not scroll a partly hidden element into view.
  await s.exec(`document.querySelector('[data-testid=snippet-new]').scrollIntoView({ block: 'center' }); return 1`);
  await s.click(await s.find("[data-testid=snippet-new]"));
  await s.exec(`const t = document.querySelector('[data-testid=snippet-css]'); t.value = ':root { --accent: rgb(200, 10, 10); }'; t.dispatchEvent(new Event('input', { bubbles: true }));`);
  await s.click(await s.find("[data-testid=snippet-save]"));
  await eventually(() => exists(".cairn/snippets/custom.css"), { message: "snippet written" });
  await s.waitFor(`return getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() === 'rgb(200, 10, 10)'`);
  await s.keys(Key.escape);
});

test("tags panel and tag search", async () => {
  await s.click(await s.find("[data-testid=tab-tags]"));
  await s.waitFor(`return [...document.querySelectorAll('[data-testid=tag-row]')].some(e => e.textContent.includes('#urgent'))`);
  await s.exec(`[...document.querySelectorAll('[data-testid=tag-row]')].find(e => e.textContent.includes('#project')).click()`);
  await s.waitFor(`return [...document.querySelectorAll('[data-testid=search-hit]')].map(e => e.textContent.trim()).join() === 'Tagged'`);
  await s.click(await s.find("[data-testid=tab-files]"));
});

test("outline and properties panels", async () => {
  await openFromTree("Tagged.md");
  await s.click(await s.find("[data-testid=right-outline]"));
  await s.waitFor(`return document.querySelector('[data-testid=outline]')?.textContent.includes('Section two')`);
  await s.click(await s.find("[data-testid=right-properties]"));
  await s.waitFor(`return document.querySelector('[data-testid=properties]')?.textContent.includes('Sam')`);
  await s.click(await s.find("[data-testid=right-links]"));
});

test("dropping a file saves it as an attachment and links it", async () => {
  await focusEditorEnd();
  await s.exec(`
    const bytes = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=='), c => c.charCodeAt(0));
    const file = new File([bytes], 'dot.png', { type: 'image/png' });
    const dt = new DataTransfer();
    dt.items.add(file);
    const target = document.querySelector('.cm-content');
    const r = target.getBoundingClientRect();
    target.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true, clientX: r.left + 5, clientY: r.bottom - 5 }));
  `);
  await eventually(() => exists("attachments/dot.png"), { message: "attachment saved" });
  await eventually(() => read("Tagged.md").includes("![[dot.png]]"), { message: "link inserted" });
});

test("plugins run sandboxed and only with the permissions they declare", async () => {
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
  await s.findWait('[data-testid=plugin-row][data-file="probe.js"]');
  await s.click(await s.find('[data-testid=plugin-row][data-file="probe.js"] [data-testid=plugin-toggle]'));
  await s.click(await s.findWait("[data-testid=dialog-ok]"));
  await eventually(() => JSON.parse(read(".cairn/settings.json")).plugins?.includes("probe.js"), { message: "plugin enabled" });
  await s.keys(Key.escape);
  await openFromTree("Welcome.md");
  await s.click(await s.find("[data-testid=mode-source]"));
  await s.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: 0, head: 9 } });`);
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Probe: Insert stamp");
  await sleep(200);
  await s.keys(Key.enter);
  const toast = await s.waitFor(`return [...document.querySelectorAll('.toast')].map(t => t.textContent).find(t => t.includes('probe:'))`, { timeout: 15000 });
  assert.ok(toast.includes("no-dom"), toast);
  assert.ok(toast.includes("no-tauri"), toast);
  assert.ok(toast.includes("net-blocked"), toast);
  assert.ok(!toast.includes("ipc-READ"), toast);
  assert.ok(toast.includes("read-denied"), toast);
  await eventually(() => read("Welcome.md").startsWith("[stamp:9]"), { message: "stamp inserted and saved" });
});
