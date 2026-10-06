// Regression tests for FINDING-102 (the file tree had no keyboard navigation).
// Unlike the other reproduction (adv_a11y_keyboard.test.mjs), this one also
// proves that the keys
// actually reach the focused tree (a keydown listener on the tree logs them,
// focus is checked after every key) and that they focus or select a row,
// expand a folder, open a note and start a rename. A second test starts from
// a row clicked with the mouse: the arrows move on from it and F2 renames the
// row they reach.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_01.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v01-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

const state = () =>
  app.exec(`
    const t = document.querySelector('[data-testid=file-tree]');
    const a = document.activeElement;
    return {
      treeFocused: !!a && t.contains(a),
      focusedRow: a?.getAttribute('role') === 'treeitem' ? a.dataset.path : null,
      activedescendant: t.getAttribute('aria-activedescendant'),
      selected: document.querySelector('.row.selected, .row.active')?.dataset.path ?? null,
      expanded: [...document.querySelectorAll('[role=treeitem][aria-expanded=true]')].map(r => r.dataset.path),
      tabs: [...document.querySelectorAll('[data-testid=tab]')].map(t => t.dataset.path),
      renaming: !!document.querySelector('[data-testid=rename-input]'),
      rowTabindex: [...new Set([...document.querySelectorAll('[role=treeitem]')].map(r => r.getAttribute('tabindex')))],
      keysSeen: window.__v01keys ?? [],
    };`);

test("FINDING-102: keys reach the focused tree and focus or select a row, expand a folder, open a note and start a rename", async () => {
  await app.reset();
  const rows = await app.exec(`return [...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path)`);
  console.log("rows:", JSON.stringify(rows));
  // Get to the tree with Tab only.
  let reached = false;
  for (let i = 0; i < 25 && !reached; i++) {
    await app.keys(K.tab);
    reached = await app.exec(`return document.activeElement?.dataset.testid === 'file-tree'`);
  }
  assert.ok(reached, "Tab reaches the tree");
  await app.exec(`window.__v01keys = []; document.querySelector('[data-testid=file-tree]').addEventListener('keydown', e => window.__v01keys.push(e.key), true); return 1`);
  const log = [];
  // F2 (and Escape out of the rename box) before Enter: Enter opens the note
  // and, like a click, moves focus to the editor.
  for (const [label, key] of [["Down", K.down], ["Right", K.right], ["Down", K.down], ["Down", K.down], ["F2", K.f2], ["Escape", K.esc], ["Enter", K.enter]]) {
    await app.keys(key);
    await sleep(250);
    log.push([label, await state()]);
  }
  for (const [label, s] of log) console.log(label, JSON.stringify(s));
  const last = log.at(-1)[1];
  // Keys were delivered to the tree and focus stayed in it (the rename box
  // is inside the tree) until Enter opened the note.
  assert.ok(last.keysSeen.length >= 7, `keys delivered to the tree: ${JSON.stringify(last.keysSeen)}`);
  assert.ok(log.slice(0, -1).every(([, s]) => s.treeFocused), "focus stayed in the tree");
  const problems = [];
  if (log.every(([, s]) => !s.focusedRow && !s.activedescendant && !s.selected)) problems.push("no key focused or selected any row");
  if (log.every(([, s]) => s.expanded.length === 0)) problems.push("Right never expanded a folder");
  if (last.tabs.length === 0) problems.push("Enter opened no note");
  if (!log.find(([label]) => label === "F2")[1].renaming) problems.push("F2 started no rename");
  console.log("row tabindex values:", JSON.stringify(last.rowTabindex));
  assert.deepEqual(problems, []);
});

test("control: after a mouse click on a row, the arrows move on from that row and F2 renames the row they reach", async () => {
  await app.reset();
  const r = await app.rectOf('[data-testid=tree-row][data-path="Welcome.md"]');
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(r.x + 30), y: Math.round(r.y + r.h / 2), duration: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
  await sleep(400);
  const afterClick = await state();
  console.log("after click:", JSON.stringify(afterClick));
  const above = await app.exec(`const rs = [...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path); return rs[rs.indexOf('Welcome.md') - 1]`);
  await app.exec(`document.querySelector('[data-testid=file-tree]').focus(); return 1`);
  await app.keys(K.up);
  await sleep(250);
  const afterUp = await state();
  console.log("after ArrowUp:", JSON.stringify(afterUp));
  const selectedRow = () => app.exec(`return document.querySelector('[role=treeitem][aria-selected=true]')?.dataset.path ?? null`);
  assert.equal(await selectedRow(), above, "ArrowUp moved the selection to the row above the clicked one");
  await app.keys(K.f2);
  await sleep(300);
  const afterF2 = await state();
  console.log("after F2:", JSON.stringify(afterF2));
  assert.equal(await app.exec(`return document.querySelector('[data-testid=rename-input]')?.closest('[data-testid=tree-row]').dataset.path ?? null`), above, "F2 renames the row ArrowUp reached");
  await app.keys(K.esc);
});
