// Adversarial tests for UI features around the editor in the real desktop
// app: hotkeys, settings, session restore, vault switching and the side
// panels (search, quick switcher, tags, graph, backlinks) after changes.
//
// Run:  scripts/e2e-headless.sh e2e/adv_ui.test.mjs
//
// Every test starts its own app on its own temp vault (see adv_editor_lib.mjs).

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { withApp, launch, freshEnv, Dir, eventually, sleep, Key } from "./adv_editor_lib.mjs";
import { approvePlugins } from "./plugin_approvals.mjs";

const SETTINGS = ".cairn/settings.json";

async function openSettings(app, section) {
  await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  if (section) {
    await app.exec(`document.querySelector('[data-testid=settings-${section}]').click(); return 1`);
    await sleep(150);
  }
}

async function closeSettings(app) {
  await app.exec(`document.querySelector('[data-testid=settings] .close').click(); return 1`);
  await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
}

const hotkeyRows = (app) => app.exec(`return [...document.querySelectorAll('[data-testid=hotkey-row]')].map(r => r.dataset.command)`);

/** Run a command from the palette by (part of) its name. */
async function palette(app, name) {
  await app.exec(`document.querySelector('[title="Command palette"]').click(); return 1`);
  const input = await app.s.findWait("[data-testid=palette-input]");
  await app.s.type(input, name);
  await sleep(150);
  await app.keys(Key.enter);
  await sleep(300);
}

/** "Switch notebook" then open `dir` from the typed-path box on the Welcome screen. */
async function switchVault(app, dir, waitRows = 1) {
  await palette(app, "Switch notebook");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=vault-path]')`, { message: "welcome screen" });
  await app.exec(
    `const i = document.querySelector('[data-testid=vault-path]'); i.value = arguments[0]; i.dispatchEvent(new Event('input', { bubbles: true })); document.querySelector('[data-testid=vault-open]').click(); return 1`,
    dir,
  );
  await app.s.waitFor(`return !!document.querySelector('.workspace')`, { timeout: 15000, message: "workspace of the new vault" });
  await app.exec(`if (!document.querySelector('[data-testid=file-tree]')) document.querySelector('[data-testid=tab-files]')?.click(); return 1`);
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= ${waitRows}`, { timeout: 15000, message: "new vault tree" });
  await app.instrument();
  await sleep(300);
}

// ---------------------------------------------------------------- findings

test(
  "FINDING-040: hotkeys follow the typed letter, not the physical key: AZERTY Ctrl+Z does not close the tab, Dvorak Ctrl+C does not toggle italic",
  async () => {
    await withApp({ "A.md": "alpha\n", "B.md": "bravo line\n" }, async (app) => {
      await app.open("A.md");
      await app.exec(`document.querySelector('[data-testid=tree-row][data-path="B.md"]').dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true })); return 1`);
      await eventually(async () => (await app.tabs()).length === 2 && (await app.activeTab()) === "B.md", { message: "two tabs, B active" });
      await app.s.waitFor(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString() === 'bravo line\\n'`);
      await app.focusEnd();
      await app.keys("xyz");
      assert.equal(await app.text(), "bravo line\nxyz");
      // French AZERTY: the key labelled Z sits where QWERTY has W, so Ctrl+Z
      // arrives as key "z", code "KeyW". The user means Undo.
      await app.keydownInEditor({ key: "z", code: "KeyW", keyCode: 90, ctrlKey: true });
      await sleep(400);
      const afterAzerty = { tabs: await app.tabs(), active: await app.activeTab() };
      // US Dvorak: the key labelled C sits where QWERTY has I: key "c", code "KeyI". The user means Copy.
      if (afterAzerty.tabs.includes("B.md")) await app.open("B.md");
      else await app.open("A.md");
      const before = await app.text();
      await app.setSel(0, 5);
      const dv = await app.keydownInEditor({ key: "c", code: "KeyI", keyCode: 67, ctrlKey: true });
      await sleep(300);
      const afterDvorak = await app.text();
      const problems = [];
      if (afterAzerty.tabs.length !== 2) problems.push(`AZERTY Ctrl+Z (undo) closed the tab: tabs now ${JSON.stringify(afterAzerty.tabs)}`);
      if (afterDvorak !== before) problems.push(`Dvorak Ctrl+C (copy) changed the note from ${JSON.stringify(before)} to ${JSON.stringify(afterDvorak)} (copy prevented: ${dv})`);
      assert.deepEqual(problems, []);
    }, { shot: "ED-09-layouts" });
  },
);

test(
  "FINDING-040: punctuation hotkeys follow the typed character: Ctrl+, opens Settings on Dvorak (without closing the tab) and on AZERTY",
  async () => {
    await withApp({ "A.md": "alpha\n" }, async (app) => {
      await app.open("A.md");
      const settingsOpen = () => app.exec(`return !!document.querySelector('[data-testid=settings]')`);
      const problems = [];
      // US Dvorak: the comma key sits where QWERTY has W: key ",", code "KeyW".
      await app.keydownInEditor({ key: ",", code: "KeyW", keyCode: 188, ctrlKey: true });
      await sleep(400);
      const dvorak = { tabs: await app.tabs(), settings: await settingsOpen() };
      if (dvorak.tabs.length !== 1) problems.push(`Dvorak Ctrl+, closed the tab: tabs now ${JSON.stringify(dvorak.tabs)}`);
      if (!dvorak.settings) problems.push("Dvorak Ctrl+, did not open Settings");
      if (dvorak.settings) await closeSettings(app);
      if (dvorak.tabs.length !== 1) await app.open("A.md");
      // French AZERTY: the comma key sits where QWERTY has M: key ",", code "KeyM".
      await app.keydownInEditor({ key: ",", code: "KeyM", keyCode: 188, ctrlKey: true });
      await sleep(400);
      if (!(await settingsOpen())) problems.push("AZERTY Ctrl+, did not open Settings");
      assert.deepEqual(problems, []);
    });
  },
);

test(
  "FINDING-096: the hotkey recorder shows a warning when asked to bind a plain letter and Ctrl+Z; afterwards the letter can still be typed and Ctrl+Z toggles no checkbox",
  async () => {
    await withApp({ "N.md": "start\n" }, async (app, env) => {
      await app.open("N.md");
      await openSettings(app, "hotkeys");
      // Bind "Open graph view" to the plain G key.
      await app.exec(`document.querySelector('[data-command="app:graph"] [data-testid=hotkey-add]').click(); return 1`);
      await sleep(150);
      await app.keys("g");
      await sleep(200);
      // Bind "Toggle checkbox" to Ctrl+Z.
      await app.exec(`document.querySelector('[data-command="editor:task"] [data-testid=hotkey-add]').click(); return 1`);
      await sleep(150);
      await app.chord(Key.ctrl, "z");
      await sleep(400);
      const toasts = await app.toasts();
      await closeSettings(app);
      // Settings are written 300 ms after a change (if the recorder took the keys).
      await sleep(800);
      const saved = env.vault.exists(SETTINGS) ? JSON.parse(env.vault.read(SETTINGS)).hotkeys : {};
      // Type a word with a g, then try to undo it.
      await app.fakeFocus();
      await app.focusEnd();
      await app.keys("dog");
      await sleep(400);
      const afterTyping = { text: await app.text(), tabs: await app.tabs() };
      if (afterTyping.tabs.length > 1 || (await app.activeTab()) !== "N.md") await app.open("N.md");
      await app.focusEnd();
      await app.chord(Key.ctrl, "z");
      await sleep(300);
      const afterUndo = await app.text();
      console.log(`saved hotkeys: ${JSON.stringify(saved)}; toasts while recording: ${JSON.stringify(toasts)}`);
      const problems = [];
      if (!afterTyping.text.includes("dog")) problems.push(`typing "dog" produced ${JSON.stringify(afterTyping.text)} and tabs ${JSON.stringify(afterTyping.tabs)} (the g opened the graph)`);
      if (/\[ \]/.test(afterUndo)) problems.push(`Ctrl+Z toggled a checkbox instead of undoing: ${JSON.stringify(afterUndo)}`);
      if (!toasts.some((t) => /G|Ctrl\+Z/.test(t))) problems.push("no warning was shown when binding keys that are needed for typing / undo");
      assert.deepEqual(problems, []);
    }, { shot: "ED-10-plain-key-hotkey" });
  },
);

test(
  "FINDING-097: Settings > Hotkeys lists the editing commands also with no note open and in reading view",
  async () => {
    await withApp({ "N.md": "text\n" }, async (app) => {
      // No note open yet (fresh session).
      assert.deepEqual(await app.tabs(), []);
      await openSettings(app, "hotkeys");
      const noTab = await hotkeyRows(app);
      await closeSettings(app);
      // Reading view of a note.
      await app.open("N.md");
      await app.setMode("preview");
      await openSettings(app, "hotkeys");
      const reading = await hotkeyRows(app);
      await closeSettings(app);
      // Control: in Live Preview the editing commands are listed.
      await app.setMode("live");
      await openSettings(app, "hotkeys");
      const live = await hotkeyRows(app);
      await closeSettings(app);
      const editing = ["editor:bold", "editor:italic", "editor:code", "editor:task", "editor:link", "note:save"];
      assert.deepEqual(editing.filter((c) => !live.includes(c)), [], "control: all editing commands listed in Live Preview");
      assert.deepEqual(
        { noTab: editing.filter((c) => !noTab.includes(c)), reading: editing.filter((c) => !reading.includes(c)) },
        { noTab: [], reading: [] },
        "editing commands missing from Settings > Hotkeys",
      );
    }, { shot: "ED-11-hotkey-list" });
  },
);

test(
  "FINDING-193: a settings.json that is valid JSON but not an object (null) opens the vault without an error, restores the session and drops the previous vault's hotkeys",
  async () => {
    const env = freshEnv({ "A.md": "alpha\n", [SETTINGS]: JSON.stringify({ hotkeys: { "app:graph": ["Mod+J"] } }) });
    const other = new Dir(path.join(env.tmp, "other"));
    other.write("B.md", "bravo\n");
    other.write("C.md", "charlie\n");
    other.write(SETTINGS, "null\n");
    let app;
    try {
      // First visit of the other vault with a working settings file: leave a tab open there.
      fs.writeFileSync(other.p(SETTINGS), "{}\n");
      app = await launch({ vault: other.root, xdg: env.xdg });
      await app.open("C.md");
      await sleep(1500); // session is written on tab activation
      await app.stop();
      fs.writeFileSync(other.p(SETTINGS), "null\n");
      // Now start in vault A (Ctrl+J opens the graph there), then switch to the other vault.
      app = await launch({ vault: env.vault.root, xdg: env.xdg });
      await switchVault(app, other.root, 2);
      await sleep(500);
      const toasts = await app.toasts();
      const tabs = await app.tabs();
      await app.exec(`document.querySelector('[data-testid=tree-row][data-path="B.md"]').click(); return 1`);
      await eventually(async () => (await app.activeTab()) === "B.md", { message: "B open" });
      await app.focusEnd();
      await app.chord(Key.ctrl, "j");
      await sleep(400);
      const graph = await app.exec(`return !!document.querySelector('[data-testid=graph-view]')`);
      const problems = [];
      const bad = toasts.find((t) => /Could not open notebook/.test(t));
      if (bad) problems.push(`error toast although the vault opened: ${JSON.stringify(bad)}`);
      if (!tabs.includes("C.md")) problems.push(`the session (tab C.md) was not restored: tabs ${JSON.stringify(tabs)}`);
      if (graph) problems.push("Ctrl+J (a custom hotkey of the previous vault) still opens the graph in this vault");
      assert.deepEqual(problems, []);
    } catch (e) {
      if (app) await app.shot("ED-12-null-settings");
      throw e;
    } finally {
      await app?.stop();
      await env.cleanup();
    }
  },
);

test(
  "FINDING-194: a hand-edited hotkey given as a string instead of a list does not hijack plain typing",
  async () => {
    await withApp({ "N.md": "x\n", [SETTINGS]: JSON.stringify({ hotkeys: { "editor:bold": "Mod+J" } }) }, async (app) => {
      await app.open("N.md");
      await app.fakeFocus();
      await app.focusEnd();
      await app.keys("jam do");
      await sleep(300);
      const t = await app.text();
      assert.equal(t, "x\njam do", `typing "jam do" produced ${JSON.stringify(t)}`);
    }, { shot: "ED-13-string-hotkey" });
  },
);

test(
  "FINDING-194: a hand-edited hotkey given as a number does not break the other hotkeys (Ctrl+I still toggles italic)",
  async () => {
    await withApp({ "N.md": "word\n", [SETTINGS]: JSON.stringify({ hotkeys: { "editor:bold": 5 } }) }, async (app) => {
      await app.open("N.md");
      await app.fakeFocus();
      await app.setSel(0, 4);
      await app.chord(Key.ctrl, "i");
      await sleep(300);
      const t = await app.text();
      const errs = await app.errors();
      assert.deepEqual({ text: t, errors: errs }, { text: "*word*\n", errors: [] });
    }, { shot: "ED-13-number-hotkey" });
  },
);

test(
  "FINDING-098: Ctrl+G in the editor's search panel finds the next match instead of opening the graph",
  async () => {
    await withApp({ "F.md": "foo one\nfoo two\nfoo three\n" }, async (app) => {
      await app.open("F.md");
      await app.fakeFocus();
      await app.setSel(0);
      await app.chord(Key.ctrl, "f");
      await app.s.waitFor(`return document.activeElement?.name === 'search'`, { message: "search panel focused" });
      await app.keys("foo", Key.enter);
      await sleep(150);
      const first = await app.sel();
      await app.chord(Key.ctrl, "g");
      await sleep(400);
      const graph = await app.exec(`return !!document.querySelector('[data-testid=graph-view]')`);
      const second = await app.sel();
      assert.deepEqual({ graph, moved: second.from > first.from }, { graph: false, moved: true }, `selection ${JSON.stringify(first)} -> ${JSON.stringify(second)}`);
    }, { shot: "ED-14-ctrl-g" });
  },
);

const STAMP = `// @name Stamp
// @description Replaces the selection.
// @permissions editor
cairn.commands.register("stamp", "Insert stamp", async () => {
  const p = await cairn.editor.activePath();
  const sel = await cairn.editor.getSelection();
  const ok = await cairn.editor.replaceSelection("[stamp]");
  await cairn.ui.toast("stamp active=" + p + " sel=" + JSON.stringify(sel) + " ok=" + ok);
});
`;

test(
  "FINDING-197: the Properties panel does not say 'No frontmatter. Add a --- block' for a note whose frontmatter is invalid YAML",
  async () => {
    await withApp({ "Y.md": "---\nkey: [unclosed\nother: value\n---\nbody\n" }, async (app) => {
      await app.open("Y.md");
      await app.exec(`document.querySelector('[data-testid=right-properties]').click(); return 1`);
      await sleep(800);
      const panel = await app.exec(`return document.querySelector('[data-testid=properties]').innerText`);
      assert.doesNotMatch(panel, /No frontmatter/, panel);
    }, { shot: "ED-19-invalid-props" });
  },
);

test(
  "FINDING-199: picking a note in the quick switcher that was deleted meanwhile keeps the current tab and opens no 'Not found' tab",
  async () => {
    await withApp({ "Work.md": "my work\n", "Gone.md": "gone\n" }, async (app, env) => {
      await app.open("Work.md");
      await app.focusEnd();
      await app.chord(Key.ctrl, "o");
      await app.s.type(await app.s.findWait("[data-testid=switcher-input]"), "Gone");
      await sleep(200);
      env.vault.rm("Gone.md"); // e.g. deleted by sync or another app while the switcher is open
      await sleep(1500);
      const items = await app.exec(`return [...document.querySelectorAll('.switcher .item .name')].map(e => e.textContent)`);
      await app.keys(Key.enter);
      await sleep(1000);
      const after = { tabs: await app.tabs(), pane: await app.exec(`return document.querySelector('.pane .empty')?.innerText.trim() ?? null`) };
      assert.deepEqual(after, { tabs: ["Work.md"], pane: null }, `switcher still listed ${JSON.stringify(items)} after the delete; choosing it gave ${JSON.stringify(after)}`);
    }, { shot: "ED-21-stale-switcher" });
  },
);

// ---------------------------------------------------------------- held up

test("after switching vaults, a plugin of the new vault cannot read or change the previous vault's note (held up)", async () => {
  const env = freshEnv({ "Same.md": "shared secret line\n" });
  const b = new Dir(path.join(env.tmp, "vaultB"));
  b.write("Same.md", "shared secret line\n"); // e.g. a copy of the vault
  b.write("Other.md", "other\n");
  b.write(".cairn/plugins/stamp.js", STAMP);
  b.write(SETTINGS, JSON.stringify({ plugins: ["stamp.js"] }));
  approvePlugins(path.join(env.xdg, "config"), b.root, ["stamp.js"]); // turned on on this device
  let app;
  try {
    app = await launch({ vault: env.vault.root, xdg: env.xdg });
    await app.open("Same.md");
    await app.setSel(7, 13); // "secret"
    await switchVault(app, b.root, 2);
    const tabs = await app.tabs();
    assert.deepEqual(tabs, [], "precondition: no tab open in vault B");
    await sleep(800); // plugin start
    await palette(app, "Insert stamp");
    await sleep(1500);
    const toasts = await app.toasts();
    const t = toasts.find((x) => x.includes("stamp active="));
    console.log(`plugin toast: ${JSON.stringify(t)} (replaceSelection reports ok=true although no note is open)`);
    assert.ok(t && !/secret/.test(t), JSON.stringify(toasts));
    await sleep(1000);
    assert.equal(b.read("Same.md"), "shared secret line\n");
    assert.equal(env.vault.read("Same.md"), "shared secret line\n");
  } catch (e) {
    if (app) await app.shot("ED-cross-vault");
    throw e;
  } finally {
    await app?.stop();
    await env.cleanup();
  }
});

test("session restore: open tabs and the active tab come back; a tab whose file was deleted or renamed while closed is dropped quietly (held up)", async () => {
  const env = freshEnv({ "A.md": "alpha\n", "B.md": "bravo\n", "C.md": "charlie\n", "D.md": "delta\n" });
  let app;
  try {
    app = await launch({ vault: env.vault.root, xdg: env.xdg });
    await app.open("A.md");
    for (const p of ["B.md", "C.md", "D.md"]) {
      await app.exec(`document.querySelector('[data-testid=tree-row][data-path="${p}"]').dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true })); return 1`);
      await eventually(async () => (await app.activeTab()) === p, { message: `${p} active` });
    }
    await app.open("C.md");
    await sleep(1500);
    await app.stop();
    env.vault.rm("B.md");
    env.vault.rename("D.md", "D2.md");
    app = await launch({ vault: env.vault.root, xdg: env.xdg });
    await eventually(async () => (await app.tabs()).length >= 2, { message: "tabs restored" });
    await sleep(500);
    assert.deepEqual(await app.tabs(), ["A.md", "C.md"]);
    assert.equal(await app.activeTab(), "C.md");
    await app.s.waitFor(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString() === 'charlie\\n'`);
    assert.deepEqual(await app.errors(), []);
    // A broken session entry in localStorage is ignored.
    await app.exec(`for (const k of Object.keys(localStorage)) if (k.startsWith('cairn.session:')) localStorage.setItem(k, '{"tabs": 5, "active": {}, "expanded": "x"}'); return 1`);
    await app.stop();
    app = await launch({ vault: env.vault.root, xdg: env.xdg });
    await sleep(800);
    assert.deepEqual(await app.tabs(), []);
  } catch (e) {
    if (app) await app.shot("ED-session");
    throw e;
  } finally {
    await app?.stop();
    await env.cleanup();
  }
});

test("recent vaults: a folder that no longer exists shows an error, stays listed until removed, and is not reopened at start (held up)", async () => {
  const env = freshEnv({ "A.md": "alpha\n" });
  const gone = new Dir(path.join(env.tmp, "gone"));
  gone.write("G.md", "g\n");
  let app;
  try {
    app = await launch({ vault: gone.root, xdg: env.xdg });
    await switchVault(app, env.vault.root);
    await app.stop();
    fs.rmSync(gone.root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    // Start without a vault argument: the most recent vault (A) opens.
    app = await launch({ vault: null, xdg: env.xdg, args: [] });
    await app.s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
    await palette(app, "Switch notebook");
    await app.s.waitFor(`return document.querySelectorAll('.recent-open').length === 2`);
    await app.instrument();
    await app.exec(`[...document.querySelectorAll('.recent-open')].find(b => b.title === arguments[0]).click(); return 1`, gone.root);
    await sleep(800);
    const toasts = await app.toasts();
    assert.ok(toasts.some((t) => /Could not open notebook/.test(t)), JSON.stringify(toasts));
    assert.ok(!fs.existsSync(gone.root), "clicking a missing recent vault must not create it");
    assert.equal(await app.exec(`return !!document.querySelector('[data-testid=vault-path]')`), true, "still on the Welcome screen");
    await app.exec(`[...document.querySelectorAll('.recent-open')].find(b => b.title === arguments[0]).parentElement.querySelector('.icon-btn').click(); return 1`, gone.root);
    await app.s.waitFor(`return document.querySelectorAll('.recent-open').length === 1`);
    // The only remaining entry opens the vault.
    await app.exec(`document.querySelector('.recent-open').click(); return 1`);
    await app.s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`);
    await app.stop();
    // Most recent vault removed while the app was closed: start shows the Welcome screen.
    fs.rmSync(env.vault.root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    app = await launch({ vault: null, xdg: env.xdg, args: [] });
    assert.equal(await app.exec(`return !!document.querySelector('[data-testid=vault-path]')`), true);
  } catch (e) {
    if (app) await app.shot("ED-recent");
    throw e;
  } finally {
    await app?.stop();
    await env.cleanup();
  }
});

test("switching vaults clears tabs, backlinks and the graph of the old vault; each vault keeps its own hotkeys (held up)", async () => {
  const env = freshEnv({ "A1.md": "see [[A2]]\n", "A2.md": "a two\n", [SETTINGS]: JSON.stringify({ hotkeys: { "app:graph": ["Mod+J"] } }) });
  const b = new Dir(path.join(env.tmp, "vaultB"));
  b.write("B1.md", "see [[B2]]\n");
  b.write("B2.md", "b two\n");
  let app;
  try {
    app = await launch({ vault: env.vault.root, xdg: env.xdg });
    await app.open("A2.md");
    await app.s.waitFor(`return document.querySelectorAll('[data-testid=backlink-source]').length === 1`);
    await app.exec(`document.querySelector('[data-testid=tab-search]').click(); return 1`);
    await app.s.type(await app.s.findWait("[data-testid=search-input]"), "two");
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=search-hit] .name')].map(e => e.textContent).join() === 'A2'`);
    await app.focusEnd();
    await app.chord(Key.ctrl, "j"); // custom binding of vault A
    await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view]')`, { message: "graph via Ctrl+J in A" });
    await switchVault(app, b.root, 2);
    assert.deepEqual(await app.tabs(), []);
    assert.equal(await app.exec(`return document.querySelectorAll('[data-testid=backlink-source]').length`), 0);
    // The search panel re-runs the query in the new vault.
    await app.exec(`document.querySelector('[data-testid=tab-search]').click(); return 1`);
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=search-hit] .name')].map(e => e.textContent).join() === 'B2'`, { message: "search shows vault B only" });
    await app.open("B2.md");
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=backlink-source]')].map(e => e.textContent.trim()).join() === 'B1'`);
    await app.focusEnd();
    await app.chord(Key.ctrl, "j");
    await sleep(300);
    assert.equal(await app.exec(`return !!document.querySelector('[data-testid=graph-view]')`), false, "vault B has default hotkeys");
    await app.chord(Key.ctrl, "g");
    await app.s.waitFor(`const g = document.querySelector('[data-testid=graph-view] .canvas')?.__graph; return g && g.order === 2`);
    const nodes = await app.exec(`return document.querySelector('[data-testid=graph-view] .canvas').__graph.nodes().sort()`);
    assert.deepEqual(nodes, ["B1.md", "B2.md"]);
    // Back to A: its binding is active again.
    await switchVault(app, env.vault.root, 2);
    await app.open("A1.md");
    await app.focusEnd();
    await app.chord(Key.ctrl, "j");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view]')`, { message: "graph via Ctrl+J back in A" });
    assert.deepEqual(await app.errors(), []);
  } catch (e) {
    if (app) await app.shot("ED-vault-switch");
    throw e;
  } finally {
    await app?.stop();
    await env.cleanup();
  }
});

test("hotkey conflicts: binding Ctrl+B to italic removes it from bold, says so, and survives a restart (held up)", async () => {
  const env = freshEnv({ "N.md": "word\n" });
  let app;
  try {
    app = await launch({ vault: env.vault.root, xdg: env.xdg });
    await app.open("N.md");
    await openSettings(app, "hotkeys");
    await app.exec(`document.querySelector('[data-command="editor:italic"] [data-testid=hotkey-add]').click(); return 1`);
    await sleep(150);
    await app.chord(Key.ctrl, "b");
    await sleep(300);
    const toasts = await app.toasts();
    assert.ok(toasts.some((t) => /Ctrl\+B/.test(t) && /Toggle bold/.test(t)), JSON.stringify(toasts));
    await closeSettings(app);
    await eventually(() => env.vault.exists(SETTINGS) && JSON.parse(env.vault.read(SETTINGS)).hotkeys?.["editor:bold"]?.length === 0, { message: "saved" });
    await app.stop();
    app = await launch({ vault: env.vault.root, xdg: env.xdg });
    await app.open("N.md");
    await app.fakeFocus();
    await app.setSel(0, 4);
    await app.chord(Key.ctrl, "b");
    await sleep(200);
    assert.equal(await app.text(), "*word*\n");
  } catch (e) {
    if (app) await app.shot("ED-hotkey-conflict");
    throw e;
  } finally {
    await app?.stop();
    await env.cleanup();
  }
});

test("invalid JSON in .cairn/settings.json: the vault opens with defaults, no error, hotkeys work (held up)", async () => {
  await withApp({ "N.md": "word\n", [SETTINGS]: '{"theme": "dark", "hotkeys": {' }, async (app) => {
    await app.open("N.md");
    await app.fakeFocus();
    await app.setSel(0, 4);
    await app.chord(Key.ctrl, "b");
    await sleep(200);
    assert.equal(await app.text(), "**word**\n");
    assert.deepEqual((await app.toasts()).filter((t) => /Could not/.test(t)), []);
  });
});

test("side panels after external and in-app changes: search, switcher, tags, graph and backlinks drop deleted/renamed notes (held up)", async () => {
  await withApp(
    {
      "Hub.md": "# Hub\n",
      "Alpha.md": "zebra words #oldtag and [[Hub]]\n",
      "Beta.md": "zebra again\n",
    },
    async (app, env) => {
      await app.open("Hub.md");
      await app.s.waitFor(`return document.querySelectorAll('[data-testid=backlink-source]').length === 1`);
      // Search panel showing two hits.
      await app.exec(`document.querySelector('[data-testid=tab-search]').click(); return 1`);
      const input = await app.s.findWait("[data-testid=search-input]");
      await app.s.type(input, "zebra");
      await app.s.waitFor(`return document.querySelectorAll('[data-testid=search-hit]').length === 2`);
      // External delete of one, external rename of the other (with a new tag).
      env.vault.rm("Beta.md");
      env.vault.rename("Alpha.md", "Gamma.md");
      env.vault.write("Gamma.md", "zebra words #newtag and [[Hub]]\n");
      await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=search-hit] .name')].map(e => e.textContent).join() === 'Gamma'`, { timeout: 6000, message: "search updated" });
      await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=backlink-source]')].map(e => e.textContent.trim()).join() === 'Gamma'`, { message: "backlinks updated" });
      await app.exec(`document.querySelector('[data-testid=tab-tags]').click(); return 1`);
      await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=tag-row] .name')].map(e => e.textContent).join() === '#newtag'`, { message: "tags updated" });
      // Quick switcher lists the current names only.
      await app.focusEnd();
      await app.chord(Key.ctrl, "o");
      await app.s.waitFor(`return !!document.querySelector('[data-testid=switcher-input]')`);
      const names = await app.exec(`return [...document.querySelectorAll('.switcher .item .name')].map(e => e.textContent.trim())`);
      await app.keys(Key.escape);
      assert.ok(names.some((n) => n.includes("Gamma")) && !names.some((n) => /Alpha|Beta/.test(n)), JSON.stringify(names));
      // Graph: nodes follow.
      await app.chord(Key.ctrl, "g");
      await app.s.waitFor(`const g = document.querySelector('[data-testid=graph-view] .canvas')?.__graph; return g && g.order === 2`);
      assert.deepEqual(await app.exec(`return document.querySelector('[data-testid=graph-view] .canvas').__graph.nodes().sort()`), ["Gamma.md", "Hub.md"]);
      env.vault.rename("Gamma.md", "Delta.md");
      await app.s.waitFor(`return document.querySelector('[data-testid=graph-view] .canvas').__graph.nodes().sort().join() === 'Delta.md,Hub.md'`, { message: "graph after rename" });
      // In-app rename of the open note (palette > Rename current note).
      await app.open("Delta.md");
      await palette(app, "Rename current note");
      const ren = await app.s.findWait("[data-testid=rename-input]");
      await app.exec(`const i = document.querySelector('[data-testid=rename-input]'); i.value = 'Epsilon'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
      await app.s.click(ren);
      await app.keys(Key.enter);
      await eventually(() => env.vault.exists("Epsilon.md") && !env.vault.exists("Delta.md"), { message: "renamed on disk" });
      await eventually(async () => (await app.tabs()).includes("Epsilon.md"), { message: "tab follows" });
      await app.exec(`document.querySelector('[data-testid=tab-search]').click(); return 1`);
      await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=search-hit] .name')].map(e => e.textContent).join() === 'Epsilon'`, { timeout: 6000, message: "search after in-app rename" });
      await app.chord(Key.ctrl, "g");
      await app.s.waitFor(`return document.querySelector('[data-testid=graph-view] .canvas')?.__graph?.nodes().sort().join() === 'Epsilon.md,Hub.md'`, { message: "graph after in-app rename" });
      await app.open("Hub.md");
      await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=backlink-source]')].map(e => e.textContent.trim()).join() === 'Epsilon'`, { message: "backlinks after in-app rename" });
      assert.deepEqual(await app.errors(), []);
    },
    { shot: "ED-panels" },
  );
});

test("graph hover: the hovered note, its links and its neighbours stand out, the rest is dimmed until the pointer leaves (FINDING-042)", async () => {
  await withApp({ "Hub.md": "# Hub [[Leaf]]\n", "Leaf.md": "leaf\n", "Other.md": "[[Far]]\n", "Far.md": "far\n" }, async (app, env) => {
    await app.open("Hub.md");
    await app.chord(Key.ctrl, "g");
    await app.s.waitFor(`const c = document.querySelector('[data-testid=graph-view] .canvas'); return !!c?.__sigma && c.__graph.order === 4`);
    const look = () =>
      app.exec(`const r = document.querySelector('[data-testid=graph-view] .canvas').__sigma;
        const css = (v) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
        const n = (k) => { const d = r.getNodeDisplayData(k); return [d.color, d.label, !!d.forceLabel]; };
        const e = (k) => { const d = r.getEdgeDisplayData(k); return [d.color, !!d.hidden]; };
        return { accent: css('--accent'), dim: css('--bg-active'), note: css('--text-muted'), edge: css('--border'),
          hub: n('Hub.md'), leaf: n('Leaf.md'), other: n('Other.md'), near: e('Hub.md->Leaf.md'), far: e('Other.md->Far.md') };`);
    const hover = (ev) => app.exec(`document.querySelector('[data-testid=graph-view] .canvas').__sigma.emit(arguments[0], { node: 'Hub.md' }); return 1`, ev);
    const before = await look();
    await hover("enterNode");
    const on = await look();
    const { accent, dim, note, edge } = on;
    assert.deepEqual(
      { hub: on.hub, leaf: on.leaf, other: on.other, near: on.near, far: on.far },
      { hub: [accent, "Hub", true], leaf: [note, "Leaf", true], other: [dim, "", false], near: [accent, false], far: [edge, true] },
    );
    // A note that links to the hovered one while it is hovered: the graph
    // reloads, and the new neighbour and link stand out too.
    env.vault.write("New.md", "[[Hub]]\n");
    await app.s.waitFor(`return document.querySelector('[data-testid=graph-view] .canvas').__graph.order === 5`, { timeout: 10000 });
    const added = await app.exec(`const r = document.querySelector('[data-testid=graph-view] .canvas').__sigma;
      const d = r.getNodeDisplayData('New.md'); const e = r.getEdgeDisplayData('New.md->Hub.md');
      return { node: [d.color, d.label, !!d.forceLabel], edge: [e.color, !!e.hidden] };`);
    assert.deepEqual(added, { node: [note, "New", true], edge: [accent, false] });
    // A new accent colour while hovering: the highlight follows it.
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
    await app.exec(`document.querySelector('[data-testid=settings-appearance]').click(); return 1`);
    await sleep(250);
    await app.exec(`const i = document.querySelector('[data-testid=settings] input[type=color]'); i.value = '#3b82f6'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
    await app.exec(`document.querySelector('[data-testid=settings] .close')?.click(); return 1`);
    // The page's --accent is the chosen colour adjusted for contrast
    // (FINDING-219), so the highlight is compared with that.
    await eventually(
      async () => {
        const l = await look();
        return l.accent !== accent && l.hub[0] === l.accent;
      },
      { message: "hovered node in the new accent colour" },
    );
    await hover("leaveNode");
    const off = await look();
    assert.deepEqual(
      { hub: off.hub, leaf: off.leaf, other: off.other, near: off.near, far: off.far },
      { hub: before.hub, leaf: before.leaf, other: [note, "Other", false], near: [edge, false], far: [edge, false] },
    );
    assert.deepEqual(await app.errors(), []);
  }, { shot: "ED-graph-hover" });
});
