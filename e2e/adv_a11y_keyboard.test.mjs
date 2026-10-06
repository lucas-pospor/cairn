// Accessibility audit, part 1: keyboard-only use and focus management in the
// real desktop app. Tests named "FINDING-nnn: ..." check the behaviour for
// that finding.
//
// Run:  scripts/e2e-headless.sh e2e/adv_a11y_keyboard.test.mjs
// One:  scripts/e2e-headless.sh --test-name-pattern 'FINDING-104' e2e/adv_a11y_keyboard.test.mjs
//
// Notes on the harness:
// - Keys are sent with the WebDriver actions API, so they are real key events
//   in the web view. WebKitWebDriver types upper-case letters as lower case,
//   so typed names are lower case.
// - Every test starts with app.reset() (empty session + page reload).

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { AxApp, K, SERVER, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-kb-");

before(async () => {
  app.write(
    ".cairn/plugins/perm.js",
    "// @name Perm plugin\n// @description Asks for a permission.\n// @permissions read\ncairn.commands.register('x', 'Perm plugin command', async () => {});\n",
  );
  await app.start();
});

after(async () => {
  await app.stop("keyboard-final.png");
});

/** Press Tab until `pred` (page script returning a boolean) holds; returns the descriptions visited. */
async function tabUntil(pred, max = 40, back = false) {
  const seen = [];
  for (let i = 0; i < max; i++) {
    if (back) await app.chord(K.shift, K.tab);
    else await app.keys(K.tab);
    seen.push(await app.focus());
    if (await app.exec(pred)) return seen;
  }
  throw new Error(`Tab never reached the target; visited:\n${seen.join("\n")}`);
}

function findFile(dir, name) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true, recursive: true })) {
    if (e.isFile() && e.name === name) return path.join(e.parentPath ?? e.path, e.name);
  }
  return null;
}

async function rightClick(css) {
  const r = await app.rectOf(css);
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(r.x + 24), y: Math.round(r.y + r.h / 2), duration: 0 },
    { type: "pointerDown", button: 2 },
    { type: "pointerUp", button: 2 },
  ]);
}

// ---------------------------------------------------------------------------
// Flows that work with the keyboard only (held up).

test("keyboard: Ctrl+N creates a note, the rename box has focus, Enter renames it and focus returns to the editor", async () => {
  await app.reset();
  await app.chord(K.ctrl, "n");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`, { message: "rename box focused" });
  await app.keys("kbnew", K.enter);
  await eventually(() => app.exists("kbnew.md") && !app.exists("Untitled.md"), { message: "kbnew.md on disk" });
  await eventually(async () => (await app.activeTab()) === "kbnew.md", { message: "renamed tab active" });
  await eventually(() => app.focusInEditor(), { message: "focus back in the editor" });
});

// Ctrl+Shift+Tab is not checked: WebKitWebDriver sends Shift+Tab with
// KeyboardEvent.key "Unidentified" (code "Tab"), so the hotkey cannot be
// exercised through WebDriver. Ctrl+PageUp is the other default for it.
test("keyboard: Ctrl+O opens notes, Ctrl+Enter opens a new tab, Ctrl+Tab / Ctrl+PageUp switch tabs, Ctrl+W closes, focus follows into the editor", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("garden");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Garden')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.tabs()).length === 2 && (await app.activeTab()) === "Projects/Garden plan.md", { message: "second tab" });
  await app.chord(K.ctrl, K.tab);
  await eventually(async () => (await app.activeTab()) === "Ideas.md", { message: "Ctrl+Tab" });
  await app.chord(K.ctrl, "\ue00e"); // PageUp
  await eventually(async () => (await app.activeTab()) === "Projects/Garden plan.md", { message: "Ctrl+PageUp" });
  await eventually(() => app.focusInEditor(), { message: "editor focused after switching" });
  await app.chord(K.ctrl, "w");
  await eventually(async () => JSON.stringify(await app.tabs()) === '["Ideas.md"]', { message: "Ctrl+W closed the tab" });
  await eventually(() => app.focusInEditor(), { message: "editor focused after closing" });
});

test("keyboard: palette 'Delete current note' + Enter on the confirm moves the note to the trash and focuses the next tab", async () => {
  await app.reset();
  app.write("Delete me.md", "to be deleted\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Delete me.md"]')`);
  await app.openNote("ideas", "Ideas.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("delete me");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Delete me')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.activeTab()) === "Delete me.md");
  await app.palette("delete current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`, { message: "confirm focused" });
  await app.keys(K.enter);
  await eventually(() => !app.exists("Delete me.md"), { message: "file gone from the vault" });
  assert.ok(findFile(app.tmp, "Delete me.md"), "file is in a trash folder");
  await eventually(async () => (await app.activeTab()) === "Ideas.md" && (await app.focusInEditor()), { message: "next tab active and focused" });
});

test("keyboard: palette 'Rename current note' renames with the keyboard only", async () => {
  await app.reset();
  app.write("Rename me.md", "rename body\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Rename me.md"]')`);
  await app.openNote("rename me", "Rename me.md");
  await app.palette("rename current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`, { message: "rename box focused" });
  await app.keys("renamed by keys", K.enter);
  await eventually(() => app.exists("renamed by keys.md") && !app.exists("Rename me.md"), { message: "renamed on disk" });
  assert.equal(app.read("renamed by keys.md"), "rename body\n");
  await eventually(() => app.focusInEditor(), { message: "focus back in the editor" });
});

test("keyboard: Ctrl+E switches to reading view; its links are reachable with Tab and open with Enter", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  await app.chord(K.ctrl, "e");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] a.internal-link')`, { message: "reading view rendered" });
  await tabUntil(`return document.activeElement?.matches('[data-testid=preview] a.internal-link')`, 30);
  assert.match(await app.focus(), /"Ideas"/);
  await app.keys(K.enter);
  await eventually(async () => (await app.activeTab()) === "Ideas.md", { message: "link opened with Enter" });
});

test("keyboard: Tab reaches the right panel tabs, view-mode buttons and backlinks; Enter on a backlink opens it", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  // Leave the editor with CodeMirror's Escape-then-Tab, then keep tabbing.
  await app.keys(K.esc);
  const seen = await tabUntil(`return document.activeElement?.dataset.testid === 'backlink-source'`, 40);
  const all = seen.join("\n");
  for (const want of ["right-links", "right-outline", "right-properties"]) assert.ok(all.includes(want), `${want} reached:\n${all}`);
  // The view-mode buttons sit before the editor in the Tab order.
  const back = await tabUntil(`return document.activeElement?.dataset.testid === 'mode-preview'`, 40, true);
  assert.ok(back.length > 0);
  await tabUntil(`return document.activeElement?.dataset.testid === 'backlink-source'`, 40);
  await app.keys(K.enter);
  await eventually(async () => (await app.activeTab()) === "Ideas.md", { message: "backlink opened" });
});

test("keyboard: Escape closes the switcher, palette, settings, confirm dialog and context menu while focus is where the app put it", async () => {
  await app.reset();
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=switcher-input]')`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[data-testid=switcher-input]')`, { message: "switcher closed" });

  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=palette-input]')`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[data-testid=palette-input]')`, { message: "palette closed" });

  await app.chord(K.ctrl, ",");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`, { message: "settings closed" });

  await app.openNote("ideas", "Ideas.md");
  await app.palette("delete current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[role=dialog]')`, { message: "confirm closed" });
  assert.ok(app.exists("Ideas.md"), "Escape cancelled the delete");

  await rightClick(`[data-testid=tree-row][data-path="Welcome.md"]`);
  await app.s.waitFor(`return !!document.querySelector('[role=menu]')`, { message: "context menu open" });
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[role=menu]')`, { message: "context menu closed" });
});

test("keyboard: Escape then Tab leaves the editor without changing the note (CodeMirror escape hatch for indentWithTab)", async () => {
  await app.reset();
  const before = app.read("Projects/Garden plan.md");
  await app.openNote("garden", "Projects/Garden plan.md");
  await app.keys(K.esc, K.tab);
  await eventually(async () => !(await app.focusInEditor()), { message: "focus left the editor" });
  await sleep(900);
  assert.equal(app.read("Projects/Garden plan.md"), before);
});

test("keyboard: Outline panel is reachable with Tab and Enter on a heading moves the cursor there; palette toggles source / split / sidebars", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await app.keys(K.esc);
  await tabUntil(`return document.activeElement?.dataset.testid === 'right-outline'`, 40);
  await app.keys(K.enter);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=outline] .heading')`, { message: "outline shown" });
  await tabUntil(`return document.activeElement?.matches('[data-testid=outline] .heading') && document.activeElement.textContent.includes('Sub heading')`, 10);
  await app.keys(K.enter);
  await eventually(() => app.exec(`const v = document.querySelector('.cm-editor').__cairnView; return v.state.doc.lineAt(v.state.selection.main.head).text === '## Sub heading' && !!document.activeElement?.closest('.cm-content')`), { message: "cursor on the heading, editor focused" });
  await app.palette("toggle live preview");
  await eventually(async () => (await app.exec(`return document.querySelector('[data-testid=mode-source]').getAttribute('aria-pressed')`)) === "true", { message: "source mode" });
  await app.palette("side by side");
  await eventually(async () => (await app.exec(`return document.querySelector('[data-testid=mode-split]').getAttribute('aria-pressed')`)) === "true", { message: "split mode" });
  await app.palette("toggle left sidebar");
  await eventually(() => app.exec(`return document.querySelector('aside.left').classList.contains('hidden')`), { message: "left sidebar hidden" });
  await app.palette("toggle left sidebar");
  await eventually(() => app.exec(`return !document.querySelector('aside.left').classList.contains('hidden')`), { message: "left sidebar back" });
  await app.palette("toggle live preview");
});

// ---------------------------------------------------------------------------
// Findings.

test("FINDING-102: the file tree works from the keyboard (ArrowDown moves, ArrowRight expands, F2 starts a rename, Enter opens the note)", async () => {
  await app.reset();
  // The tree is a Tab stop; get there with Tab like a keyboard user would.
  await tabUntil(`return document.activeElement?.dataset.testid === 'file-tree'`, 20);
  const state = () =>
    app.exec(`
      const t = document.querySelector('[data-testid=file-tree]');
      const a = document.activeElement;
      return {
        focusedItem: a?.getAttribute('role') === 'treeitem' ? a.dataset.path : null,
        activedescendant: t.getAttribute('aria-activedescendant'),
        cursor: document.getElementById(t.getAttribute('aria-activedescendant'))?.dataset.path ?? null,
        selected: document.querySelector('.row.selected')?.dataset.path ?? null,
        expanded: [...document.querySelectorAll('[role=treeitem][aria-expanded=true]')].map(r => r.dataset.path),
        tabs: [...document.querySelectorAll('[data-testid=tab]')].map(t => t.dataset.path),
      };`);
  const problems = [];
  const s0 = await state();
  await app.keys(K.down);
  const s1 = await state();
  if (!s1.focusedItem && !s1.activedescendant && !s1.selected) problems.push(`ArrowDown in the focused tree selects nothing: ${JSON.stringify(s1)}`);
  else if (s1.cursor === s0.cursor && s1.focusedItem === s0.focusedItem) problems.push(`ArrowDown does not move to the next row: ${JSON.stringify(s1)}`);
  await app.keys(K.right);
  const s2 = await state();
  if (s2.expanded.length === 0) problems.push(`ArrowRight does not expand the folder: ${JSON.stringify(s2)}`);
  // Down into the folder just opened. F2 comes before Enter: Enter opens the
  // note and, like a click, moves focus to the editor.
  await app.keys(K.down, K.f2);
  await sleep(200);
  if (!(await app.exec(`return !!document.querySelector('[data-testid=rename-input]')`))) problems.push("F2 in the focused tree starts no rename (it only works on a row selected with the mouse)");
  await app.keys(K.esc);
  await sleep(200);
  await app.keys(K.enter);
  await sleep(400);
  const s3 = await state();
  if (s3.tabs.length === 0) problems.push(`Down + Enter opens no note: ${JSON.stringify(s3)}`);
  else if (!(await app.focusInEditor())) problems.push(`Enter opened ${s3.tabs} but focus is not in the editor: ${await app.focus()}`);
  await app.shot("AX-01-tree-keyboard.png");
  assert.deepEqual(problems, []);
});

test("FINDING-103: there is a keyboard path to move a note and to rename or delete a folder", async () => {
  await app.reset();
  const problems = [];
  app.write("Journal/Upper.md", "upper\n");
  // Folders: the row menu, F2 and Delete on the focused row of the tree.
  // The Menu key / Shift+F10 fires "contextmenu" at the focused element.
  // WebDriver cannot send those keys to WebKitGTK (tried: no event), so the
  // event a keyboard would produce is dispatched at the focused tree.
  await tabUntil(`return document.activeElement?.dataset.testid === 'file-tree'`, 20);
  await app.keys(""); // Home: the first row, the Journal folder
  const cursor = () => app.exec(`const t = document.activeElement; return document.getElementById(t.getAttribute('aria-activedescendant'))?.dataset.path ?? null`);
  const first = await cursor();
  if (first !== "Journal") problems.push(`Home in the focused tree does not go to the first row, the Journal folder (cursor: ${first})`);
  await app.exec(`const t = document.activeElement; const r = t.getBoundingClientRect(); t.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.x + 10, clientY: r.y + 10 })); return 1`);
  const items = await app.exec(`return [...document.querySelectorAll('[role=menuitem]')].map(b => b.textContent)`);
  if (!["Move to…", "Rename…", "Delete"].every((i) => items.includes(i))) problems.push(`keyboard context menu on the tree offers only ${JSON.stringify(items)} (no Move/Rename/Delete: it targets the tree, not a row)`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[role=menu]')`, { message: "menu closed" });
  await app.keys(K.f2);
  await sleep(200);
  const renaming = await app.exec(`return document.querySelector('[data-testid=rename-input]')?.closest('[data-testid=tree-row]').dataset.path ?? null`);
  if (renaming !== "Journal") problems.push(`F2 on the focused folder does not rename it (renaming: ${renaming})`);
  await app.keys(K.esc);
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'file-tree'`, { message: "focus back on the tree" });
  await app.keys(K.del);
  await sleep(200);
  const confirm = await app.exec(`return document.querySelector('#dialog-message')?.textContent ?? null`);
  if (!/folder "Journal"/.test(confirm ?? "")) problems.push(`Delete on the focused folder asks nothing about it (confirm: ${confirm})`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[role=dialog]')`, { message: "confirm closed" });
  assert.ok(app.exists("Journal/2026-10-03.md"), "Escape cancelled the delete");
  // Notes: "Move current note to…" in the palette, then the folder chooser.
  await app.openNote("upper", "Journal/Upper.md");
  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=palette-item]').length > 10`);
  const cmds = await app.exec(`return [...document.querySelectorAll('[data-testid=palette-item]')].map(b => b.firstElementChild.textContent)`);
  await app.keys(K.esc);
  if (!cmds.some((c) => /move/i.test(c))) problems.push(`the command palette has no move command: ${JSON.stringify(cmds)}`);
  else {
    await app.palette("move current");
    await app.s.waitFor(`return document.activeElement?.matches('[role=dialog] .choice')`, { message: "folder chooser focused" });
    assert.equal(await app.exec(`return document.activeElement.textContent`), "Vault root");
    await app.keys(K.enter);
    await eventually(() => app.exists("Upper.md") && !app.exists("Journal/Upper.md"), { message: "note moved to the vault root" }).catch(() =>
      problems.push("Move current note to… + Enter on 'Vault root' did not move the note"),
    );
  }
  assert.deepEqual(problems, []);
});

test("FINDING-104: Settings takes focus; typing after Ctrl+, does not edit the note behind it and Escape gives focus back to the editor", async () => {
  await app.reset();
  app.write("Settings typing.md", "original text\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Settings typing.md"]')`);
  await app.openNote("settings typing", "Settings typing.md");
  await app.chord(K.ctrl, ",");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  const focusAfterOpen = await app.focus();
  await app.keys("zzq");
  await sleep(1500);
  await app.shot("AX-03-typing-behind-settings.png");
  const disk = app.read("Settings typing.md");
  const problems = [];
  if (!(await app.exec(`return !!document.activeElement?.closest('[data-testid=settings]')`))) problems.push(`focus after opening Settings: ${focusAfterOpen}`);
  if (disk !== "original text\n") problems.push(`keys typed while Settings was open were saved into the note: ${JSON.stringify(disk)}`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`, { message: "Escape closes Settings" });
  if (!(await app.focusInEditor())) problems.push(`focus after closing Settings with Escape: ${await app.focus()}`);
  assert.deepEqual(problems, []);
});

test("FINDING-104: with a note open, Tab after Ctrl+, reaches Settings and does not indent the note behind it", async () => {
  await app.reset();
  app.write("Settings tab.md", "first line\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Settings tab.md"]')`);
  await app.openNote("settings tab", "Settings tab.md");
  await app.chord(K.ctrl, ",");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  const visited = [];
  for (let i = 0; i < 3; i++) {
    await app.keys(K.tab);
    visited.push(await app.focus());
  }
  await sleep(1200);
  const disk = app.read("Settings tab.md");
  const reached = await app.exec(`return !!document.activeElement?.closest('[data-testid=settings]')`);
  await app.keys(K.esc);
  await sleep(150);
  const stillOpen = await app.exec(`return !!document.querySelector('[data-testid=settings]')`);
  await app.shot("AX-03-tab-into-settings.png");
  const problems = [];
  if (!reached) problems.push(`3 x Tab never reached a Settings control; focus: ${visited.join(" -> ")}`);
  if (disk !== "first line\n") problems.push(`Tab indented the note behind Settings and it was saved: ${JSON.stringify(disk)}`);
  // Only a problem while focus is still in the editor: inside the dialog, Escape is meant to close it.
  if (!reached && !stillOpen) problems.push("Escape (CodeMirror's 'leave the editor' key) closed Settings instead, so there is no keyboard path into it");
  assert.deepEqual(problems, []);
});

test("FINDING-105: global hotkeys do not act behind the Settings modal (no quick switcher under it, no new note behind it)", async () => {
  await app.reset();
  const problems = [];
  try {
    await app.openNote("ideas", "Ideas.md");
    await app.chord(K.ctrl, ",");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
    await app.chord(K.ctrl, "o");
    await sleep(300);
    const sw = await app.exec(`
      const i = document.querySelector('[data-testid=switcher-input]');
      if (!i) return null;
      const r = i.getBoundingClientRect();
      const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return { focused: document.activeElement === i, topmost: __ax.desc(top), visible: !!top?.closest('.switcher') };`);
    await app.shot("AX-04-switcher-under-settings.png");
    if (sw && !sw.visible) problems.push(`Ctrl+O opened the quick switcher underneath Settings (focused=${sw.focused}, element on top: ${sw.topmost})`);
    if (sw) {
      await app.keys("garden", K.enter);
      await sleep(500);
      const t = await app.activeTab();
      if (t !== "Ideas.md") problems.push(`typing + Enter in the invisible switcher replaced the note behind Settings with ${t}`);
    }
    // Escape closes Settings (and the switcher, if still there).
    await app.keys(K.esc);
    await sleep(200);
    await app.chord(K.ctrl, ",");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
    await app.chord(K.ctrl, "n");
    await sleep(900);
    if (app.exists("Untitled.md")) problems.push(`Ctrl+N with Settings open created Untitled.md behind the modal; focus: ${await app.focus()}`);
  } finally {
    fs.rmSync(app.p("Untitled.md"), { force: true });
  }
  assert.deepEqual(problems, []);
});

// Ctrl+P inside the switcher is its "previous result" key (FINDING-208; Linux,
// Windows), so no palette opens; two Escapes must still leave nothing.
test("FINDING-106: after Ctrl+P over the quick switcher, two Escapes leave no overlay open", async () => {
  await app.reset();
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.chord(K.ctrl, "p");
  await sleep(200);
  const both = await app.exec(`return !!document.querySelector('[data-testid=switcher-input]') && !!document.querySelector('[data-testid=palette-input]')`);
  await app.keys(K.esc);
  await sleep(150);
  await app.keys(K.esc);
  await sleep(150);
  const st = await app.exec(`return { switcher: !!document.querySelector('[data-testid=switcher-input]'), palette: !!document.querySelector('[data-testid=palette-input]'), focus: __ax.desc(document.activeElement) }`);
  await app.shot("AX-05-orphaned-switcher.png");
  assert.ok(!st.switcher && !st.palette, `two Escapes left overlays open (both were open at once: ${both}): ${JSON.stringify(st)}`);
});

test("FINDING-107: the confirm dialog traps focus: Tab stays inside it and Escape closes it", async () => {
  await app.reset();
  app.write("Trap target.md", "keep me\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Trap target.md"]')`);
  await app.openNote("trap target", "Trap target.md");
  await app.palette("delete current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`);
  const problems = [];
  await app.keys(K.tab);
  const f = await app.focus();
  if (!(await app.exec(`return !!document.activeElement?.closest('[role=dialog]')`))) problems.push(`Tab from the last dialog button moved focus behind the aria-modal dialog to: ${f}`);
  await app.keys(K.esc);
  await sleep(150);
  if (await app.exec(`return !!document.querySelector('[role=dialog]')`)) problems.push("Escape no longer closes the dialog once focus has left it");
  await app.shot("AX-06-dialog-focus-escaped.png");
  assert.ok(app.exists("Trap target.md"));
  assert.deepEqual(problems, []);
});

test("FINDING-107: the quick switcher traps focus: Tab stays inside it and Escape closes it", async () => {
  await app.reset();
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("ide");
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=switcher-item]').length === 1`);
  const visited = [];
  for (let i = 0; i < 4; i++) {
    await app.keys(K.tab);
    visited.push(await app.focus());
  }
  const problems = [];
  if (!(await app.exec(`return !!document.activeElement?.closest('.switcher')`))) problems.push(`Tab left the switcher: ${visited.join(" -> ")}`);
  await app.keys(K.esc);
  await sleep(150);
  if (await app.exec(`return !!document.querySelector('[data-testid=switcher-input]')`)) problems.push("Escape does not close the switcher once focus is on one of its results or behind it");
  assert.deepEqual(problems, []);
});

// WebKitGTK gives Shift+Tab the key "Unidentified" (code "Tab"), from WebDriver
// and from a real keyboard alike, so the trap has to recognise it by its code.
test("FINDING-107: Shift+Tab stays inside the confirm dialog, the quick switcher and Settings", async () => {
  await app.reset();
  app.write("Back tab.md", "keep me\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Back tab.md"]')`);
  await app.openNote("back tab", "Back tab.md");
  const problems = [];
  const backTab = async (label, inside) => {
    const seen = [];
    for (let i = 0; i < 4; i++) {
      await app.chord(K.shift, K.tab);
      seen.push(await app.focus());
      if (!(await app.exec(`return !!document.activeElement?.closest(arguments[0])`, inside))) {
        problems.push(`${label}: Shift+Tab left it: ${seen.join(" -> ")}`);
        break;
      }
    }
    await app.keys(K.esc);
    await sleep(150);
    if (await app.exec(`return !!document.querySelector(arguments[0])`, inside)) {
      problems.push(`${label}: Escape did not close it`);
      await app.keys(K.esc);
      await sleep(150);
    }
  };

  await app.palette("delete current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`);
  await backTab("confirm dialog", "[role=dialog]");

  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("back");
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=switcher-item]').length === 1`);
  await backTab("quick switcher", ".switcher");

  await app.chord(K.ctrl, ",");
  await app.s.waitFor(`return !!document.activeElement?.closest('[data-testid=settings]')`);
  await backTab("Settings", "[data-testid=settings]");

  assert.ok(app.exists("Back tab.md"));
  assert.deepEqual(problems, []);
});

test("FINDING-106: Escape from an overlay that replaced another by hotkey gives focus back to the editor", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  const sel = { o: "[data-testid=switcher-input]", p: "[data-testid=palette-input]", ",": "[data-testid=settings]" };
  const results = [];
  // The third key is the overlay left open. Ctrl+P inside the switcher is its
  // "previous result" key (FINDING-208), so there the switcher stays; on
  // macOS Cmd+P would swap it for the palette.
  for (const [first, second, shown] of [["o", "p", "o"], ["o", ",", ","], ["p", ",", ","], ["p", "o", "o"]]) {
    await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
    await app.chord(K.ctrl, first);
    await app.s.waitFor(`return !!document.activeElement?.closest(${JSON.stringify(sel[first])})`, { message: `Ctrl+${first} focused its overlay` });
    await app.chord(K.ctrl, second);
    await sleep(200);
    await app.s.waitFor(`return !!document.activeElement?.closest(${JSON.stringify(sel[shown])})`, { message: `after Ctrl+${second} the overlay of Ctrl+${shown} has focus` });
    await app.keys(K.esc);
    await sleep(200);
    const open = await app.exec(`return Object.values(arguments[0]).filter((s) => document.querySelector(s))`, sel);
    if (open.length || !(await app.focusInEditor())) results.push(`Ctrl+${first}, Ctrl+${second}, Escape: open ${JSON.stringify(open)}, focus ${await app.focus()}`);
  }
  assert.deepEqual(results, []);
});

test("FINDING-109: closing an overlay gives focus back to the editor, not to <body>", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  const refocus = () => app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  const results = [];
  const record = async (label) => results.push({ label, editor: await app.focusInEditor(), focus: await app.focus() });

  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=switcher-input]')`);
  await app.keys(K.esc);
  await sleep(150);
  await record("quick switcher, Escape");

  await refocus();
  await app.chord(K.ctrl, "p");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=palette-input]')`);
  await app.keys(K.esc);
  await sleep(150);
  await record("command palette, Escape");

  await refocus();
  await app.chord(K.ctrl, ",");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await app.keys(K.esc);
  await sleep(150);
  await record("settings (Ctrl+,), Escape");

  await refocus();
  await app.palette("create new folder");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-input'`);
  await app.keys(K.esc);
  await sleep(150);
  await record("New folder prompt, Escape");

  await refocus();
  await app.palette("delete current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`);
  await app.keys(K.esc);
  await sleep(150);
  await record("Delete confirm, Escape");

  await refocus();
  await app.palette("rename current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`);
  await app.keys(K.esc);
  await sleep(150);
  await record("inline rename, Escape");

  await refocus();
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("welcome");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Welcome')`);
  await app.keys(K.enter);
  await sleep(300);
  await record("quick switcher, Enter on the note that is already open");

  const lost = results.filter((r) => !r.editor);
  assert.deepEqual(lost, [], `focus was in the editor before each overlay opened; after closing it went to:\n${lost.map((r) => `${r.label}: ${r.focus}`).join("\n")}`);
});

// Related to FINDING-109: the editor focuses itself one animation frame after
// a note opens. Frames are slowed down here so Ctrl+P lands first.
test("keyboard: the palette opened right after a note opens keeps focus when the editor's deferred focus fires", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await app.exec(`const raf = window.requestAnimationFrame.bind(window); window.__axRaf = raf; window.requestAnimationFrame = (cb) => setTimeout(() => raf(cb), 400); return 1`);
  let f;
  try {
    await app.chord(K.ctrl, "o");
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
    await app.keys("garden");
    await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Garden')`);
    await app.chord(K.ctrl, K.enter);
    await eventually(async () => (await app.activeTab()) === "Projects/Garden plan.md", { message: "second tab" });
    await app.chord(K.ctrl, "p");
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'palette-input'`, { message: "palette focused" });
    await sleep(900);
    f = await app.focus();
  } finally {
    await app.exec(`window.requestAnimationFrame = window.__axRaf; return 1`);
    await app.keys(K.esc);
  }
  assert.match(f, /palette-input/, "the editor took focus from the open palette");
});

test("FINDING-110: tab bar keyboard support: no invisible focused close button, ArrowRight moves to the next tab, Space activates a tab", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("garden");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Garden')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.tabs()).length === 2);
  const problems = [];
  // Tab from the left sidebar into the tab bar: inactive tab, its close button, active tab, ...
  await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
  await app.keys(K.tab);
  const t1 = await app.focus();
  await app.keys(K.tab);
  const closeInfo = await app.exec(`const b = document.activeElement; return { desc: __ax.desc(b), inInactiveTab: !!b.closest('.tab:not(.active)'), ...__ax.focusStyle(b) }`);
  if (closeInfo.inInactiveTab && closeInfo.opacity === 0) problems.push(`Tab lands on the inactive tab's close button, which stays invisible (opacity 0) while focused: ${closeInfo.desc}`);
  if (/"Close"$/.test(closeInfo.desc)) problems.push(`close button name does not say which tab: ${closeInfo.desc}`);
  // Arrow keys / Space on a role=tab element.
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Ideas.md"]').focus(); return 1`);
  await app.keys(K.right);
  const afterRight = await app.focus();
  if (!/data-testid=tab\].*Garden/.test(afterRight)) problems.push(`ArrowRight on a tab does not move to the next tab (focus: ${afterRight})`);
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Ideas.md"]').focus(); return 1`);
  await app.keys(K.space);
  await sleep(200);
  if ((await app.activeTab()) !== "Ideas.md") problems.push("Space on a focused tab does not activate it (only Enter does)");
  await app.shot("AX-09-tabbar.png");
  assert.deepEqual(problems, [], `first Tab stop in the tab bar: ${t1}`);
});

test("FINDING-110, FINDING-216: tab bar keys: the open tab is the one Tab stop, arrows wrap, Home/End, Delete closes the focused tab", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  for (const [query, p, name] of [["garden", "Projects/Garden plan.md", "Garden"], ["welcome", "Welcome.md", "Welcome"]]) {
    await app.chord(K.ctrl, "o");
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
    await app.keys(query);
    await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes(${JSON.stringify(name)})`);
    await app.chord(K.ctrl, K.enter);
    await eventually(async () => (await app.activeTab()) === p, { message: `${p} open` });
  }
  assert.deepEqual(await app.tabs(), ["Ideas.md", "Projects/Garden plan.md", "Welcome.md"]);
  const focused = () => app.exec(`return document.activeElement?.dataset.path ?? __ax.desc(document.activeElement)`);
  const problems = [];
  await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
  await app.keys(K.tab);
  const first = await focused();
  if (first !== "Welcome.md") problems.push(`Tab from the tree lands on ${first}, not on the open tab`);
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Welcome.md"]').focus(); return 1`);
  for (const [key, name, want] of [
    [K.right, "ArrowRight on the last tab", "Ideas.md"],
    [K.left, "ArrowLeft on the first tab", "Welcome.md"],
    [K.home, "Home", "Ideas.md"],
    [K.end, "End", "Welcome.md"],
    [K.left, "ArrowLeft", "Projects/Garden plan.md"],
  ]) {
    await app.keys(key);
    const f = await focused();
    if (f !== want) problems.push(`${name}: focus on ${f}, expected ${want}`);
  }
  if ((await app.activeTab()) !== "Welcome.md") problems.push("moving the focus along the tabs changed the open tab");
  // Tab leaves the tab list; Shift+Tab comes back to the open tab.
  await app.keys(K.tab);
  const out = await app.exec(`return { inTabs: !!document.activeElement?.closest('[role=tablist]'), desc: __ax.desc(document.activeElement) }`);
  if (out.inTabs) problems.push(`Tab from a tab stays in the tab list: ${out.desc}`);
  await app.chord(K.shift, K.tab);
  const back = await focused();
  if (back !== "Welcome.md") problems.push(`Shift+Tab back into the tab list lands on ${back}, not on the open tab`);
  // Delete closes the focused tab, not the note, and focus stays in the tab bar.
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Ideas.md"]').focus(); return 1`);
  await app.keys(K.del);
  await eventually(async () => !(await app.tabs()).includes("Ideas.md"), { message: "Ideas tab closed" });
  await sleep(300);
  const afterDel = await focused();
  if (afterDel !== "Projects/Garden plan.md") problems.push(`after Delete on a tab, focus on ${afterDel}`);
  if ((await app.activeTab()) !== "Welcome.md") problems.push("Delete on another tab changed the open tab");
  assert.ok(app.exists("Ideas.md"), "Delete on a tab deleted the note");
  assert.deepEqual(problems, []);
});

test("FINDING-111: the context menu works from the keyboard (focus on open, arrows, Home/End, Escape, Tab and Enter)", async () => {
  await app.reset();
  // Open it first: WebKitWebDriver ends a right click with a left click
  // (mouseup and click with button 0), which would open the note and send
  // focus to the editor. With the note open that click changes nothing.
  await app.openNote("welcome", "Welcome.md");
  await rightClick(`[data-testid=tree-row][data-path="Welcome.md"]`);
  await app.s.waitFor(`return !!document.querySelector('[role=menu]')`);
  const problems = [];
  const f1 = await app.focus();
  if (!/role=menuitem/.test(f1)) problems.push(`opening the menu does not focus its first item (focus: ${f1})`);
  await app.keys(K.down);
  const f2 = await app.focus();
  if (!/role=menuitem/.test(f2)) problems.push(`ArrowDown does not move through the menu items (focus: ${f2})`);
  // Down and Up wrap around; Home and End go to the first and last item.
  const labels = await app.exec(`return [...document.querySelectorAll('[role=menuitem]')].map(b => b.textContent.trim())`);
  const item = () => app.exec(`const a = document.activeElement; return a?.getAttribute('role') === 'menuitem' ? a.textContent.trim() : __ax.desc(a)`);
  if ((await item()) !== labels[1]) problems.push(`ArrowDown from the first item lands on ${await item()}, not ${labels[1]}`);
  for (const [name, key, want] of [
    ["End", K.end, labels.at(-1)],
    ["ArrowDown on the last item", K.down, labels[0]],
    ["ArrowUp on the first item", K.up, labels.at(-1)],
    ["Home", K.home, labels[0]],
  ]) {
    await app.keys(key);
    const f = await item();
    if (f !== want) problems.push(`${name}: focus on ${f}, expected ${want}`);
  }
  const stops = await app.exec(`return [...document.querySelectorAll('[role=menuitem]')].filter(b => b.tabIndex >= 0).map(b => b.textContent.trim())`);
  if (stops.length !== 1 || stops[0] !== labels[0]) problems.push(`menu items that are Tab stops: ${JSON.stringify(stops)} (expected only the focused one)`);
  // Escape closes the menu and focus goes back to where it was when the menu
  // opened: the tree, which the right mouse button focused.
  const onTree = () => app.exec(`return document.activeElement?.dataset.testid === 'file-tree'`);
  await app.keys(K.esc);
  await app.s.waitFor(`return !document.querySelector('[role=menu]')`, { message: "Escape closes the menu" });
  if (!(await onTree())) problems.push(`after Escape, focus on ${await app.focus()}, not back on the tree`);
  // The row menu from the keyboard (Menu key or Shift+F10: dispatched, see FINDING-103).
  await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
  const keyboardMenu = async () => {
    await app.exec(`const t = document.activeElement; const r = t.getBoundingClientRect(); t.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.x + 10, clientY: r.y + 10 })); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[role=menu]')`, { message: "menu open" });
    const f = await app.focus();
    if (!/role=menuitem/.test(f)) problems.push(`the menu opened from the keyboard does not focus its first item (focus: ${f})`);
  };
  // Tab closes it and gives focus back to the tree.
  await keyboardMenu();
  await app.keys(K.tab);
  await sleep(200);
  if (await app.exec(`return !!document.querySelector('[role=menu]')`)) problems.push("Tab does not close the menu");
  else if (!(await onTree())) problems.push(`after Tab closed the menu, focus on ${await app.focus()}, not on the tree`);
  // Enter runs the focused item: Rename… (just above Delete).
  if (await onTree()) {
    await keyboardMenu();
    await app.keys(K.end, K.up);
    const r = await item();
    if (r !== "Rename…") problems.push(`End then ArrowUp lands on ${r}, not Rename…`);
    await app.keys(K.enter);
    await app.s
      .waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`, { message: "rename box focused" })
      .catch(() => problems.push("Enter on Rename… does not start a rename"));
    await app.keys(K.esc);
    await app.s
      .waitFor(`return document.activeElement?.dataset.testid === 'file-tree'`, { message: "focus back on the tree" })
      .catch(async () => problems.push(`after Escape in the rename box, focus on ${await app.focus()}`));
  }
  // A hotkey overlay closes the menu; when it closes, focus goes back to the tree.
  if (await onTree()) {
    await keyboardMenu();
    await app.chord(K.ctrl, "o");
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`, { message: "switcher focused" });
    if (await app.exec(`return !!document.querySelector('[role=menu]')`)) problems.push("the menu stays open under the quick switcher");
    await app.keys(K.esc);
    await app.s.waitFor(`return !document.querySelector('[data-testid=switcher-input]')`, { message: "switcher closed" });
    if (!(await onTree())) problems.push(`after Escape in a quick switcher opened over the menu, focus on ${await app.focus()}, not on the tree`);
  }
  assert.ok(app.exists("Welcome.md"), "the menu keys changed the note");
  assert.deepEqual(problems, []);
});

test("FINDING-112: narrow layout drawers (window <= 760px): opening one moves focus into it and sets aria-expanded, Tab stays inside, Escape closes it", async () => {
  await app.reset();
  const problems = [];
  try {
    await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
    await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !document.querySelector('aside.left').classList.contains('hidden')`, { message: "drawer open" });
    const st = await app.exec(`return { focus: __ax.desc(document.activeElement), inDrawer: !!document.activeElement?.closest('aside.left'), expanded: document.querySelector('[data-testid=mobile-files]').getAttribute('aria-expanded') }`);
    if (!st.inDrawer) problems.push(`opening the Files drawer leaves focus on ${st.focus}`);
    if (st.expanded !== "true") problems.push(`drawer toggle has aria-expanded=${st.expanded}`);
    await app.keys(K.tab);
    const next = await app.exec(`return { focus: __ax.desc(document.activeElement), inDrawer: !!document.activeElement?.closest('aside.left') }`);
    if (!next.inDrawer) problems.push(`Tab moves to a control behind the drawer backdrop: ${next.focus}`);
    await app.shot("AX-11-narrow-drawer.png");
    await app.keys(K.esc);
    await sleep(200);
    if (!(await app.exec(`return document.querySelector('aside.left').classList.contains('hidden')`))) problems.push("Escape does not close the drawer");
  } finally {
    await app.s.cmd("POST", "/window/maximize", {});
    await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`).catch(() => {});
  }
  assert.deepEqual(problems, []);
});

test("FINDING-112: in a drawer, Escape closes the rename box or the row menu first, then the drawer; focus goes back to the toggle", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  const doc = `return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`;
  const before = await app.exec(doc);
  const drawerOpen = () => app.exec(`return !document.querySelector('aside.left').classList.contains('hidden')`);
  const inDrawer = () => app.exec(`return !!document.activeElement?.closest('aside.left')`);
  const problems = [];
  try {
    await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
    await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.activeElement?.closest('aside.left')`, { message: "focus in the drawer" });
    // A screen reader hears that a modal opened.
    const announced = await app.exec(`const a = document.querySelector('aside.left'); return [__ax.role(a), a.getAttribute('aria-modal'), __ax.name(a)].join(' ')`);
    if (announced !== "dialog true Left sidebar") problems.push(`the open drawer is "${announced}", not a modal dialog with a name`);
    // Keys typed in the drawer do not reach the note behind it.
    await app.keys("zz");
    await sleep(200);
    if ((await app.exec(doc)) !== before) problems.push("typing in the drawer changed the note behind it");
    // Shift+Tab from the first control wraps inside the drawer.
    await app.chord(K.shift, K.tab);
    if (!(await inDrawer())) problems.push(`Shift+Tab from the first control left the drawer: ${await app.focus()}`);
    // The rename box takes Escape; the drawer stays open.
    await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
    await app.keys(K.f2);
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`, { message: "rename box focused" });
    await app.keys(K.esc);
    await app.s.waitFor(`return !document.querySelector('[data-testid=rename-input]')`, { message: "rename cancelled" });
    if (!(await drawerOpen())) problems.push("Escape in the rename box also closed the drawer");
    // So does the row menu.
    await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
    await app.chord(K.shift, K.f10);
    await app.s.waitFor(`return !!document.activeElement?.closest('[role=menu]')`, { message: "row menu focused" });
    await app.keys(K.esc);
    await app.s.waitFor(`return !document.querySelector('[role=menu]')`, { message: "menu closed" });
    if (!(await drawerOpen())) problems.push("Escape in the row menu also closed the drawer");
    if (!(await inDrawer())) problems.push(`after Escape in the row menu, focus on ${await app.focus()}`);
    // Then Escape closes the drawer and focus goes back to its toggle.
    await app.keys(K.esc);
    await eventually(async () => !(await drawerOpen()), { message: "drawer closed" }).catch(() => problems.push("Escape does not close the drawer"));
    await eventually(() => app.exec(`return document.activeElement?.dataset.testid === 'mobile-files'`), { message: "focus on the toggle" }).catch(async () =>
      problems.push(`after the drawer closed, focus on ${await app.focus()}`),
    );
    // A drawer that is open when the window gets wide keeps the focus inside, as a sidebar.
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.activeElement?.closest('aside.left')`, { message: "focus in the drawer again" });
    await app.s.cmd("POST", "/window/maximize", {});
    await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`, { message: "wide layout" });
    await sleep(200);
    if (!(await inDrawer())) problems.push(`after widening, focus moved from the sidebar to ${await app.focus()}`);
  } finally {
    await app.s.cmd("POST", "/window/maximize", {});
    await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`).catch(() => {});
  }
  assert.ok(app.exists("Welcome.md"), "the drawer keys renamed the note");
  assert.deepEqual(problems, []);
});

test("FINDING-112: choosing an outline heading, a search result or the open note in a drawer puts focus in the editor at that line", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  const view = `document.querySelector('.cm-editor').__cairnView`;
  const problems = [];
  // After choosing `what` in a drawer: the drawers are closed, the editor has focus with the caret on `line` and takes keys.
  const check = async (what, line) => {
    await eventually(() => app.exec(`return [...document.querySelectorAll('aside')].every(a => a.classList.contains('hidden'))`), { message: "drawer closed" }).catch(() =>
      problems.push(`${what}: the drawer stays open`),
    );
    await eventually(() => app.focusInEditor(), { message: "editor focused" }).catch(async () => problems.push(`${what}: focus on ${await app.focus()}, not in the editor`));
    const at = await app.exec(`const v = ${view}; return v.state.doc.lineAt(v.state.selection.main.head).number`);
    if (at !== line) problems.push(`${what}: caret on line ${at}, not ${line}`);
    const text = await app.exec(`return ${view}.state.doc.line(${line}).text`);
    await app.keys("xy");
    await eventually(async () => (await app.exec(`return ${view}.state.doc.line(${line}).text`)) === "xy" + text, { message: "typed" }).catch(() =>
      problems.push(`${what}: keys typed next do not reach the note`),
    );
    await app.keys("", ""); // Backspace
    await app.chord(K.ctrl, K.home);
  };
  try {
    await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
    await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
    // A heading in the outline (right drawer).
    await app.exec(`document.querySelector('[aria-controls=right-sidebar][title="Links and outline"]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.activeElement?.closest('aside.right')`, { message: "focus in the right drawer" });
    await app.exec(`document.querySelector('[data-testid=right-outline]').focus(); return 1`);
    await app.keys(K.enter);
    const heading = `[...document.querySelectorAll('[data-testid=outline] .heading')].find(b => b.textContent === 'Sub heading')`;
    await app.s.waitFor(`return !!${heading}`, { message: "outline shown" });
    await app.exec(`${heading}.focus(); return 1`);
    await app.keys(K.enter);
    await check("outline heading", 8);
    // A search result in the note that is open (left drawer).
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.activeElement?.closest('aside.left')`, { message: "focus in the left drawer" });
    await app.exec(`document.querySelector('[data-testid=tab-search]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=search-input]')`, { message: "search panel" });
    await app.exec(`document.querySelector('[data-testid=search-input]').focus(); return 1`);
    await app.keys("sub heading");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=search-results] .snippet')`, { message: "search result" });
    await app.exec(`document.querySelector('[data-testid=search-results] .snippet').focus(); return 1`);
    await app.keys(K.enter);
    await check("search result", 8);
    // The open note in the file tree.
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.activeElement?.closest('aside.left')`, { message: "focus in the left drawer again" });
    await app.exec(`document.querySelector('[data-testid=tab-files]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=file-tree]')`, { message: "file tree" });
    await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
    await app.keys(K.enter);
    await check("the open note in the tree", 1);
  } finally {
    await app.s.cmd("POST", "/window/maximize", {});
    await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`).catch(() => {});
  }
  assert.deepEqual(problems, []);
});

test("FINDING-112: the rename box in a drawer gives focus back to the drawer, not to the editor under it", async () => {
  app.write("kbdrawer.md", "rename me\n");
  await app.reset({ minRows: 5 });
  // A note near the top: a tree in a closed drawer renders only its first rows.
  await app.openNote("ideas", "Ideas.md");
  const problems = [];
  const inDrawer = () => app.exec(`return !!document.activeElement?.closest('aside.left') && !document.querySelector('aside.left').classList.contains('hidden')`);
  try {
    await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
    await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.activeElement?.closest('aside.left')`, { message: "focus in the drawer" });
    // F2 on a row, a new name, Enter.
    await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
    const cursorPath = `return document.getElementById(document.querySelector('[data-testid=file-tree]').getAttribute('aria-activedescendant'))?.dataset.path`;
    await app.keys(K.home);
    for (let i = 0; i < 10 && (await app.exec(cursorPath)) !== "kbdrawer.md"; i++) await app.keys(K.down);
    await app.keys(K.f2);
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`, { message: "rename box focused" });
    await app.keys("kbdrawer2", K.enter);
    await eventually(() => app.exists("kbdrawer2.md"), { message: "renamed on disk" });
    await eventually(inDrawer, { message: "focus in the drawer" }).catch(async () => problems.push(`after Enter in the rename box, focus on ${await app.focus()}`));
    // The rename command opens the drawer with the box; Escape there.
    await app.keys(K.esc);
    await app.s.waitFor(`return document.querySelector('aside.left').classList.contains('hidden')`, { message: "drawer closed" });
    await app.palette("rename current");
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`, { message: "rename box focused" });
    await app.keys(K.esc);
    await app.s.waitFor(`return !document.querySelector('[data-testid=rename-input]')`, { message: "rename cancelled" });
    await eventually(inDrawer, { message: "focus in the drawer" }).catch(async () => problems.push(`after Escape in the rename box, focus on ${await app.focus()}`));
  } finally {
    await app.s.cmd("POST", "/window/maximize", {});
    await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`).catch(() => {});
    fs.rmSync(app.p("kbdrawer.md"), { force: true });
    fs.rmSync(app.p("kbdrawer2.md"), { force: true });
  }
  assert.ok(app.exists("Ideas.md"), "Escape renamed the note");
  assert.deepEqual(problems, []);
});

test("FINDING-112: widening the window while a drawer has focus, when that sidebar was closed in the wide layout, puts focus in the editor", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  await app.palette("toggle left sidebar");
  await app.s.waitFor(`return document.querySelector('aside.left').classList.contains('hidden')`, { message: "left sidebar closed" });
  try {
    await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
    await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
    await app.exec(`document.querySelector('[data-testid=mobile-files]').focus(); return 1`);
    await app.keys(K.enter);
    await app.s.waitFor(`return !!document.activeElement?.closest('aside.left')`, { message: "focus in the drawer" });
    await app.s.cmd("POST", "/window/maximize", {});
    await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`, { message: "wide layout" });
    await eventually(() => app.focusInEditor(), { message: "editor focused" }).catch(() => {});
    assert.ok(await app.focusInEditor(), `focus on ${await app.focus()}`);
  } finally {
    await app.s.cmd("POST", "/window/maximize", {});
    await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`).catch(() => {});
  }
});

test("FINDING-113: the focused file tree shows a focus indicator", async () => {
  await app.reset();
  const r = await app.exec(`const t = document.querySelector('[data-testid=file-tree]'); return { tabindex: t.tabIndex, ...__ax.focusStyle(t), rowsFocusable: [...t.querySelectorAll('[role=treeitem]')].some(r => r.tabIndex >= 0) }`);
  assert.ok(r.indicator || r.rowsFocusable, `tree is a Tab stop (tabindex ${r.tabindex}) with outline:none and no :focus style; rows are never focusable: ${JSON.stringify(r)}`);
});

test("FINDING-202: after the window was narrow, both sidebars come back when it is widened again", async () => {
  await app.reset();
  try {
    await app.s.cmd("POST", "/window/rect", { width: 700, height: 700 });
    await app.s.waitFor(`return !!document.querySelector('.workspace.narrow')`, { message: "narrow layout" });
  } finally {
    await app.s.cmd("POST", "/window/maximize", {});
  }
  await app.s.waitFor(`return !document.querySelector('.workspace.narrow')`, { message: "wide layout again" });
  await sleep(200);
  const st = await app.exec(`return { left: !document.querySelector('aside.left').classList.contains('hidden'), right: !document.querySelector('aside.right').classList.contains('hidden') }`);
  await app.shot("AX-13-sidebars-after-resize.png");
  assert.deepEqual(st, { left: true, right: true });
});


test("FINDING-118: the 'Move to…' folder chooser takes focus and closes with Escape", async () => {
  await app.reset();
  await rightClick(`[data-testid=tree-row][data-path="Welcome.md"]`);
  await app.s.waitFor(`return !!document.querySelector('[role=menu]')`);
  const r = await app.exec(`const b = [...document.querySelectorAll('[role=menuitem]')].find(b => b.textContent.startsWith('Move to')).getBoundingClientRect(); return { x: b.x + 20, y: b.y + b.height / 2 }`);
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(r.x), y: Math.round(r.y), duration: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
  await app.s.waitFor(`return !!document.querySelector('[role=dialog] .choice')`, { message: "chooser open" });
  const problems = [];
  try {
    const f = await app.focus();
    if (!(await app.exec(`return !!document.activeElement?.closest('[role=dialog]')`))) problems.push(`focus after the chooser opened: ${f}`);
    await app.keys(K.esc);
    await sleep(200);
    if (await app.exec(`return !!document.querySelector('[role=dialog] .choice')`)) problems.push("Escape does not close the chooser");
    await app.shot("AX-23-move-chooser.png");
  } finally {
    await app.exec(`[...document.querySelectorAll('[role=dialog] button')].find(b => b.textContent.trim() === 'Cancel')?.click(); return 1`);
  }
  assert.ok(app.exists("Welcome.md"));
  assert.deepEqual(problems, []);
});

// The same chooser on the desktop (FINDING-030 was found on Android with a
// long press; the right-click menu opens the same dialog).
test("FINDING-030: picking a folder in the 'Move to…' chooser moves the note", async () => {
  await app.reset();
  app.write("Chosen.md", "chosen\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Chosen.md"]')`);
  await app.exec(`window.__errors = []; addEventListener('error', e => window.__errors.push(e.message)); return 1`);
  await rightClick(`[data-testid=tree-row][data-path="Chosen.md"]`);
  await app.s.waitFor(`return !!document.querySelector('[role=menu]')`);
  await app.exec(`[...document.querySelectorAll('[role=menuitem]')].find(b => b.textContent === 'Move to…').click(); return 1`);
  await app.s.waitFor(`return [...document.querySelectorAll('[role=dialog] .choice')].some(b => b.textContent === 'Projects')`, { message: "chooser lists Projects" });
  await app.exec(`[...document.querySelectorAll('[role=dialog] .choice')].find(b => b.textContent === 'Projects').click(); return 1`);
  await eventually(() => app.exists("Projects/Chosen.md") && !app.exists("Chosen.md"), { message: "Chosen.md moved into Projects" }).catch(async (e) => {
    throw new Error(`${e.message}; page errors: ${JSON.stringify(await app.exec(`return window.__errors`))}`);
  });
  assert.equal(app.read("Projects/Chosen.md"), "chosen\n");
});

test("FINDING-208: Ctrl+N inside the quick switcher (its 'next result' key) moves to the next result and creates no note", async () => {
  await app.reset();
  fs.rmSync(app.p("Untitled.md"), { force: true });
  let st;
  try {
    await app.chord(K.ctrl, "o");
    await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
    await app.keys("e");
    await app.s.waitFor(`return document.querySelectorAll('[data-testid=switcher-item]').length >= 2`);
    const before = await app.exec(`return document.querySelector('[role=option][aria-selected=true]')?.dataset.i`);
    await app.chord(K.ctrl, "n");
    await sleep(900);
    st = await app.exec(`return { highlighted: document.querySelector('[role=option][aria-selected=true]')?.dataset.i ?? null, switcherOpen: !!document.querySelector('.switcher'), renaming: !!document.querySelector('[data-testid=rename-input]'), focus: __ax.desc(document.activeElement) }`);
    st.before = before;
    st.untitledOnDisk = app.exists("Untitled.md");
    await app.shot("AX-24-ctrl-n-in-switcher.png");
  } finally {
    await app.keys(K.esc).catch(() => {});
    fs.rmSync(app.p("Untitled.md"), { force: true });
  }
  assert.ok(!st.untitledOnDisk && st.highlighted === "1", `Ctrl+N in the switcher should move to result 1: ${JSON.stringify(st)}`);
});

test("FINDING-209: Escape in a confirm opened from Settings leaves Settings open; Escape while a CSS snippet is being edited closes the snippet editor or Settings", async () => {
  await app.reset();
  const problems = [];
  await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await app.exec(`document.querySelector('[data-testid=settings-plugins]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=plugin-toggle]')`, { message: "plugin listed" });
  await app.exec(`document.querySelector('[data-testid=plugin-toggle]').click(); return 1`);
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`, { message: "permission confirm focused" });
  await app.keys(K.esc);
  await sleep(250);
  const st = await app.exec(`return { confirm: !!document.querySelector('[role=dialog][aria-modal=true]'), settings: !!document.querySelector('[data-testid=settings]') }`);
  if (!st.confirm && !st.settings) problems.push("Escape on the 'Enable Perm plugin?' confirm closed the confirm AND the whole Settings window");
  // Snippet editor.
  if (!st.settings) {
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  }
  await app.exec(`document.querySelector('[data-testid=settings-appearance]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=snippet-new]')`);
  await app.exec(`document.querySelector('[data-testid=snippet-new]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=snippet-css]')`);
  await app.exec(`document.querySelector('[data-testid=snippet-css]').focus(); return 1`);
  await app.keys(K.esc);
  await sleep(250);
  const st2 = await app.exec(`return { editor: !!document.querySelector('[data-testid=snippet-css]'), settings: !!document.querySelector('[data-testid=settings]') }`);
  if (st2.editor && st2.settings) problems.push("Escape while editing a CSS snippet does nothing (neither cancels the snippet editor nor closes Settings)");
  await app.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Cancel')?.click(); return 1`).catch(() => {});
  await app.keys(K.esc);
  assert.deepEqual(problems, []);
});

test("FINDING-210: graph view from the keyboard: typing after Ctrl+G leaves the note alone, the canvas is focusable and a found node opens with Enter", async () => {
  await app.reset();
  const before = app.read("Ideas.md");
  await app.openNote("ideas", "Ideas.md");
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`, { message: "graph shown" });
  // Typing right after Ctrl+G must not reach the hidden editor.
  await app.keys("zz");
  await sleep(900);
  const problems = [];
  if (app.read("Ideas.md") !== before) problems.push(`keys typed after Ctrl+G were written into the hidden note: ${JSON.stringify(app.read("Ideas.md"))}`);
  const seen = await tabUntil(`return document.activeElement?.matches('[data-testid=graph-view] input.text-input')`, 40);
  // Counted while the graph is shown: opening a note leaves it.
  const focusableInCanvas = await app.exec(`return [...document.querySelectorAll('[data-testid=graph-view] .canvas *')].filter(e => e.tabIndex >= 0).length`);
  await app.keys("garden", K.enter);
  await sleep(700);
  await app.keys(K.enter);
  await sleep(500);
  const st = { focusableInCanvas, active: await app.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.textContent.trim()`) };
  await app.shot("AX-26-graph-keyboard.png");
  if (st.focusableInCanvas === 0) problems.push(`graph nodes are canvas pixels: nothing in the canvas is focusable (Tab path to the find box: ${seen.length} stops)`);
  if (!/Garden/.test(st.active ?? "")) problems.push(`find "garden" + Enter + Enter only moves the camera; no keyboard action opens the note (active tab: ${st.active})`);
  assert.deepEqual(problems, []);
});

test("FINDING-210: on the focused graph, the arrow keys pick a note (announced) and Enter opens it", async () => {
  await app.reset();
  await app.chord(K.ctrl, "g");
  await app.s.waitFor(`return document.querySelector('[data-testid=graph-view] .canvas')?.__graph?.order > 0`, { message: "graph loaded" });
  // Notes in name order (earlier tests in this file add some).
  const names = await app.exec(`return document.querySelector('[data-testid=graph-view] .canvas').__graph.mapNodes((k, a) => a.label).sort((a, b) => a.localeCompare(b))`);
  await tabUntil(`return document.activeElement?.matches('[data-testid=graph-view] canvas')`, 40);
  const picked = [];
  for (let i = 0; i < 30 && picked[picked.length - 1] !== "Ideas"; i++) {
    await app.keys(K.right);
    picked.push(await app.exec(`return document.querySelector('[data-testid=graph-view] [aria-live]')?.textContent.trim()`));
  }
  console.log(`notes picked with the Right arrow: ${JSON.stringify(picked)}`);
  assert.deepEqual(picked, names.slice(0, names.indexOf("Ideas") + 1));
  await app.keys(K.enter);
  await eventually(async () => (await app.activeTab()) === "Ideas.md", { message: "Ideas opened from the graph" });
});

test("FINDING-113: focus-indicator sweep: every focusable element shows a change when focused", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys(K.esc);
  const r = await app.exec(`
    const out = [];
    for (const el of document.querySelectorAll('button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])')) {
      if (__ax.hidden(el) || el.closest('.cm-content')) continue;
      const r = el.getBoundingClientRect(); if (r.width < 1) continue;
      const f = __ax.focusStyle(el);
      if (!f.indicator || f.opacity === 0) out.push(__ax.desc(el) + (f.opacity === 0 ? ' (opacity 0)' : ' (no focus style)'));
    }
    return out;`);
  console.log("focusable elements with no visible focus change:\n" + r.join("\n"));
  assert.deepEqual(r, []);
});


test("FINDING-008: typing while reading view, the graph view or Settings is shown does not edit the hidden note (also with focus on a button)", async () => {
  await app.reset();
  const problems = [];
  for (const [label, key] of [["reading view (Ctrl+E)", "e"], ["graph view (Ctrl+G)", "g"]]) {
    const file = `Hidden ${key}.md`;
    app.write(file, "# Hidden\n\nbody text\n");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="${file}"]')`);
    await app.openNote(`hidden ${key}`, file);
    await app.chord(K.ctrl, key);
    await app.s.waitFor(key === "e" ? `return !!document.querySelector('[data-testid=preview] h1')` : `return !!document.querySelector('[data-testid=graph-view] canvas')`, { message: label });
    await sleep(200);
    const st = await app.exec(`const a = document.activeElement; return { focus: __ax.desc(a), editorVisible: !!a?.closest('.cm-editor') && a.getBoundingClientRect().width > 0 }`);
    await app.keys("qq");
    await sleep(1200);
    const disk = app.read(file);
    if (disk !== "# Hidden\n\nbody text\n") problems.push(`${label}: focus stayed on ${st.focus} (visible: ${st.editorVisible}); typing "qq" saved ${JSON.stringify(disk)}`);
    await app.shot(`AX-31-hidden-editor-${key}.png`);
    if (key === "e") await app.chord(K.ctrl, "e");
  }
  // Same with the mouse: click the graph button in the sidebar, then type.
  const file = "Hidden m.md";
  app.write(file, "# Hidden\n\nbody text\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="${file}"]')`);
  await app.openNote("hidden m", file);
  const r = await app.rectOf("[data-testid=open-graph]");
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(r.x + r.w / 2), y: Math.round(r.y + r.h / 2), duration: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`);
  await sleep(200);
  const focusM = await app.focus();
  await app.keys("qq");
  await sleep(1200);
  const diskM = app.read(file);
  if (diskM !== "# Hidden\n\nbody text\n") problems.push(`graph view (mouse click on the Graph button): focus on ${focusM}; typing "qq" saved ${JSON.stringify(diskM)}`);
  // And behind Settings opened with the mouse (gear button), focus on the gear.
  await app.exec(`document.querySelector('[data-testid=tab][data-path="Hidden m.md"]').click(); return 1`);
  await eventually(async () => (await app.activeTab()) === "Hidden m.md");
  app.write(file, "# Hidden\n\nbody text\n");
  await sleep(800);
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
  const g = await app.rectOf("[data-testid=open-settings]");
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(g.x + g.w / 2), y: Math.round(g.y + g.h / 2), duration: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  const focusS = await app.focus();
  await app.keys("qq");
  await sleep(1200);
  const diskS = app.read(file);
  await app.shot("AX-31-typing-behind-settings-mouse.png");
  await app.keys(K.esc);
  if (diskS !== "# Hidden\n\nbody text\n") problems.push(`Settings opened with the mouse: focus on ${focusS}; typing "qq" saved ${JSON.stringify(diskS)}`);
  assert.deepEqual(problems, []);
});

// Last: needs a sync server and leaves sync configured until it disconnects.
test("FINDING-108: the version history modal takes focus, is aria-modal and closes with Escape", async () => {
  await app.reset();
  const port = 19000 + Math.floor(Math.random() * 900);
  const url = `http://127.0.0.1:${port}`;
  const server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: "ax-token-0123456789", CAIRN_DATA: path.join(app.tmp, "server"), CAIRN_ADDR: `127.0.0.1:${port}` },
    stdio: "ignore",
  });
  const problems = [];
  try {
    await eventually(async () => (await fetch(`${url}/health`)).ok, { message: "server up" });
    // Setup is not what is being tested: fill the sync form directly.
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
    await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
    await app.openNote("ideas", "Ideas.md");
    await app.palette("version history");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=history]')`, { message: "history open" });
    const st = await app.exec(`const d = document.querySelector('[data-testid=history]'); return { focus: __ax.desc(document.activeElement), inside: !!document.activeElement?.closest('[data-testid=history]'), modal: d.getAttribute('aria-modal') }`);
    if (!st.inside) problems.push(`focus after opening Version history: ${st.focus}`);
    if (st.modal !== "true") problems.push(`history dialog has aria-modal=${st.modal}`);
    await app.keys(K.esc);
    await sleep(250);
    if (await app.exec(`return !!document.querySelector('[data-testid=history]')`)) problems.push("Escape does not close Version history (its key handler is on the backdrop, which never has focus)");
    await app.shot("AX-07-history-escape.png");
  } finally {
    await app.exec(`return window.__TAURI_INTERNALS__.invoke('sync_disconnect').then(() => 1, () => 0)`).catch(() => {});
    server.kill();
  }
  assert.deepEqual(problems, []);
});
