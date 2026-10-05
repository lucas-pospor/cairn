// Reproduction for FINDING-118 (the "Move to…" chooser takes no
// focus and ignores Escape), from the state a real user is usually in: a note
// open in the editor, then right-click a tree row > Move to….
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ax_23.test.mjs
//
// Records: where focus lands, whether Escape closes the chooser, how many Tab
// presses it takes to reach the first choice, and whether letters typed while
// the modal chooser is open reach the note behind it.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { AxApp, K, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v23-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

test("FINDING-118: Move-to chooser opened with a note in the editor: focus, Escape, Tab reachability", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  const before = app.read("Ideas.md");
  const r0 = await app.rectOf(`[data-testid=tree-row][data-path="Welcome.md"]`);
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(r0.x + 24), y: Math.round(r0.y + r0.h / 2), duration: 0 },
    { type: "pointerDown", button: 2 },
    { type: "pointerUp", button: 2 },
  ]);
  await app.s.waitFor(`return !!document.querySelector('[role=menu]')`);
  const r = await app.exec(`const b = [...document.querySelectorAll('[role=menuitem]')].find(b => b.textContent.startsWith('Move to')).getBoundingClientRect(); return { x: b.x + 20, y: b.y + b.height / 2 }`);
  await app.s.pointer([
    { type: "pointerMove", x: Math.round(r.x), y: Math.round(r.y), duration: 0 },
    { type: "pointerDown", button: 0 },
    { type: "pointerUp", button: 0 },
  ]);
  await app.s.waitFor(`return !!document.querySelector('[role=dialog] .choice')`, { message: "chooser open" });
  const st = { focusOnOpen: await app.focus(), focusInsideOnOpen: await app.exec(`return !!document.activeElement?.closest('[role=dialog]')`) };
  await app.keys(K.esc);
  await sleep(200);
  st.openAfterEscape = await app.exec(`return !!document.querySelector('[role=dialog] .choice')`);
  await app.keys("q");
  await sleep(1500);
  st.noteChangedByTyping = app.read("Ideas.md") !== before;
  // Tab reachability only means something while the chooser is still open.
  let tabs = 0;
  for (; st.openAfterEscape && tabs < 40; tabs++) {
    if (await app.exec(`return !!document.activeElement?.closest('[role=dialog]')`)) break;
    await app.keys(K.tab);
  }
  st.tabsToReachDialog = tabs;
  st.firstDialogFocus = await app.focus();
  if (st.openAfterEscape && (await app.exec(`return !!document.activeElement?.closest('[role=dialog]')`))) {
    await app.keys(K.esc);
    await sleep(200);
    st.escapeWorksOnceFocusedInside = !(await app.exec(`return !!document.querySelector('[role=dialog] .choice')`));
  }
  await app.exec(`[...document.querySelectorAll('[role=dialog] button')].find(b => b.textContent.trim() === 'Cancel')?.click(); return 1`);
  console.log(JSON.stringify(st, null, 1));
  if (st.noteChangedByTyping) app.write("Ideas.md", before);
  assert.ok(app.exists("Welcome.md"));
  assert.ok(st.focusInsideOnOpen && !st.openAfterEscape, JSON.stringify(st));
});
