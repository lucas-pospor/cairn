// More adversarial Live Preview / UI tests in the real desktop app.
//
// Run:  scripts/e2e-headless.sh e2e/adv_editor_more.test.mjs
//
// Every test starts its own app on its own temp vault (see adv_editor_lib.mjs).

import { test } from "node:test";
import assert from "node:assert/strict";
import { withApp, eventually, sleep, Key } from "./adv_editor_lib.mjs";

/** Where the app window is after a click: still Cairn, or navigated away? */
async function whereAmI(app) {
  try {
    return await app.exec(`return { href: location.href, workspace: !!document.querySelector('.workspace'), title: document.title, adv: !!window.__adv }`);
  } catch (e) {
    return { error: String(e.message).slice(0, 200) };
  }
}

/** Click a link inside a Live Preview widget, report window location and open_url calls. */
async function clickLinkIn(app, css) {
  await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(css)})`, { message: css, timeout: 8000 });
  const before = await app.exec(`return location.href`);
  const c = await app.center(css);
  await app.clickAt(c.x, c.y);
  await sleep(1500);
  const after = await whereAmI(app);
  const opened = after.adv ? (await app.invokes("plugin:opener|open_url")).map((i) => i.body) : null;
  return { before, after, opened };
}

test(
  "FINDING-007: clicking an external link inside an embedded note in Live Preview does not navigate the app window away",
  async () => {
    await withApp(
      { "Host.md": "top line\n\n![[Inner]]\n\nend\n", "Inner.md": "Inner says: [example](https://example.com/from-embed)\n" },
      async (app, env) => {
        await app.open("Host.md");
        await app.fakeFocus();
        await app.focusEnd();
        await app.keys("typed"); // an edit still waiting for the 600 ms autosave
        const r = await clickLinkIn(app, '.cm-lp-embed .embed-body a[href^="https://"]');
        await sleep(1000);
        const disk = env.vault.read("Host.md");
        console.log(`external link in an embed: ${JSON.stringify(r)}; Host.md on disk: ${JSON.stringify(disk)}`);
        assert.ok(r.after.workspace, `the app window navigated away to ${r.after.href} (title ${JSON.stringify(r.after.title)}); it was at ${r.before}; pending edit on disk: ${disk.includes("typed")}`);
      },
      { shot: "ED-22-embed-external-link" },
    );
  },
);

test("the app window refuses to navigate away from the app, even when a script asks it to (website, vault:// file)", async () => {
  await withApp(
    { "N.md": "note\n", "att/d.svg": `<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>` },
    async (app) => {
      await app.open("N.md");
      const results = {};
      for (const url of ["https://example.com/forced", "vault://localhost/att/d.svg"]) {
        await app.exec(`location.href = arguments[0]; return true`, url);
        await sleep(1500);
        results[url] = await whereAmI(app);
      }
      // Navigation inside the app's own page is still allowed.
      await app.exec(`location.hash = "inside"; return true`);
      await sleep(300);
      const hash = await app.exec(`return location.hash`);
      console.log(`forced navigations: ${JSON.stringify(results)}; hash: ${hash}`);
      const moved = Object.entries(results).filter(([, w]) => !w.workspace || !w.adv);
      assert.deepEqual(moved, [], "the window left the app");
      assert.equal(hash, "#inside");
    },
    { shot: "nav-guard" },
  );
});

test(
  "FINDING-041: a relative Markdown link ([t](Other.md)) in a Live Preview table or embed opens the note instead of reloading the app window",
  async () => {
    const results = {};
    for (const [what, files, css] of [
      ["table", { "T.md": "top\n\n| Col | Link |\n|---|---|\n| a | [other note](Other.md) |\n\nend\n", "Other.md": "# Other\n" }, '.cm-lp-table a[href="Other.md"]'],
      ["embed", { "T.md": "top\n\n![[Inner]]\n\nend\n", "Inner.md": "see [other note](Other.md)\n", "Other.md": "# Other\n" }, '.cm-lp-embed .embed-body a[href="Other.md"]'],
    ]) {
      await withApp(
        files,
        async (app, env) => {
          await app.open("T.md");
          await app.fakeFocus();
          await app.focusEnd();
          // Leave an edit pending (autosave is 600 ms) to see whether it survives.
          await app.keys("typed");
          const r = await clickLinkIn(app, css);
          const tab = r.after.workspace ? await app.activeTab() : null;
          await sleep(1000);
          results[what] = { ...r, tab, disk: env.vault.read("T.md").includes("typed") ? "typed text saved" : "typed text LOST on disk" };
        },
        { shot: `ED-23-relative-link-${what}` },
      );
    }
    console.log(`relative links: ${JSON.stringify(results)}`);
    const bad = Object.entries(results)
      .filter(([, r]) => !r.after.workspace || r.tab !== "Other.md")
      .map(([what, r]) => `${what}: window ${r.after.href} (workspace shown: ${r.after.workspace}, page reloaded: ${!r.after.adv}), active tab ${r.tab}, ${r.disk}`);
    assert.deepEqual(bad, []);
  },
);

test(
  "FINDING-101: double-clicking a word on a line with hidden syntax selects that word",
  async () => {
    const src = "first line here\n**Bold** and [[Target|alias]] then target word\nlast line\n";
    await withApp({ "D.md": src, "Target.md": "x\n" }, async (app) => {
      await app.open("D.md");
      await app.fakeFocus();
      await app.setSel(0);
      await sleep(200);
      // Screen position of "target" while line 2 is rendered (syntax hidden).
      const c = await app.exec(`
        const v = document.querySelector('.cm-editor').__cairnView;
        const pos = v.state.doc.toString().indexOf('target word') + 3;
        const r = v.coordsAtPos(pos);
        return { x: r.left + 1, y: (r.top + r.bottom) / 2 };`);
      await app.s.pointer([
        { type: "pointerMove", origin: "viewport", x: Math.round(c.x), y: Math.round(c.y) },
        { type: "pointerDown", button: 0 },
        { type: "pointerUp", button: 0 },
        { type: "pointerDown", button: 0 },
        { type: "pointerUp", button: 0 },
      ]);
      await sleep(300);
      const s = await app.sel();
      const picked = src.slice(s.from, s.to);
      assert.equal(picked, "target", `double-click on "target" selected ${JSON.stringify(picked)} (${s.from}-${s.to})`);
    }, { shot: "ED-24-double-click" });
  },
);

test(
  "FINDING-200: an unknown defaultMode in .cairn/settings.json (e.g. \"graph\") still shows the note's editor",
  async () => {
  await withApp({ "N.md": "body text\n", ".cairn/settings.json": JSON.stringify({ defaultMode: "graph" }) }, async (app) => {
    await app.open("N.md");
    const st = await app.exec(`const e = document.querySelector('[data-testid=editor]'); return { visible: !!e && e.offsetParent !== null && e.getBoundingClientRect().height > 0, body: document.querySelector('.pane .body')?.className, text: document.querySelector('.pane')?.innerText.slice(0, 200) }`);
    console.log(`defaultMode "graph": ${JSON.stringify(st)}`);
    assert.ok(st.visible, `the note's editor is hidden: ${JSON.stringify(st)}`);
  }, { shot: "ED-25-defaultmode" });
  },
);

test(
  "FINDING-201: Shift+click on a rendered link extends the selection instead of opening the link",
  async () => {
    const src = "first line\nsee [[Target]] here\nlast line\n";
    await withApp({ "S.md": src, "Target.md": "# Target\n" }, async (app) => {
      await app.open("S.md");
      await app.fakeFocus();
      await app.setSel(2); // inside "first line"
      await sleep(200);
      const c = await app.center(".cm-lp-wikilink");
      await app.s.cmd("POST", "/actions", {
        actions: [
          { type: "key", id: "kb", actions: [{ type: "keyDown", value: Key.shift }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "keyUp", value: Key.shift }] },
          { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", x: Math.round(c.x), y: Math.round(c.y) }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }, { type: "pause", duration: 0 }] },
        ],
      });
      await app.s.cmd("DELETE", "/actions");
      await sleep(600);
      const after = { tabs: await app.tabs(), active: await app.activeTab() };
      const s = after.active === "S.md" ? await app.sel() : null;
      assert.deepEqual({ active: after.active, extended: !!s && s.to > 11 && s.from === 2 }, { active: "S.md", extended: true }, `after Shift+click: ${JSON.stringify(after)}, selection ${JSON.stringify(s)}`);
    }, { shot: "ED-26-shift-click" });
  },
);

test("outline, properties, tags and backlinks follow in-app edits after autosave; an outline click lands on the heading in a note with frontmatter (held up)", async () => {
  const src = "---\ntitle: First\n---\nintro\n\n## Alpha\n\ntext\n";
  await withApp({ "N.md": src, "Other.md": "nothing yet\n" }, async (app, env) => {
    await app.open("N.md");
    await app.exec(`document.querySelector('[data-testid=right-outline]').click(); return 1`);
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=outline] .heading')].map(e => e.textContent).join() === 'Alpha'`, { message: "outline" });
    // Type a new heading at the end and change the title in the frontmatter.
    await app.fakeFocus();
    await app.focusEnd();
    await app.keys("\n## Beta #fresh\n");
    await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; const i = v.state.doc.toString().indexOf('First'); v.dispatch({ changes: { from: i, to: i + 5, insert: 'Second' } }); return 1`);
    await eventually(() => env.vault.read("N.md").includes("## Beta #fresh") && env.vault.read("N.md").includes("title: Second"), { message: "autosaved" });
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=outline] .heading')].map(e => e.textContent).join() === 'Alpha,Beta #fresh'`, { timeout: 6000, message: "outline after edit" });
    // Click "Alpha" in the outline: cursor goes to its line.
    await app.setSel(0);
    await app.exec(`[...document.querySelectorAll('[data-testid=outline] .heading')].find(e => e.textContent === 'Alpha').click(); return 1`);
    await sleep(400);
    assert.equal((await app.cursorLine()).text, "## Alpha");
    await app.exec(`document.querySelector('[data-testid=right-properties]').click(); return 1`);
    await app.s.waitFor(`return /title\\s*Second/.test(document.querySelector('[data-testid=properties]').innerText)`, { timeout: 6000, message: "properties after edit" });
    await app.s.waitFor(`return /#fresh/.test(document.querySelector('[data-testid=properties]').innerText)`, { message: "note tags after edit" });
    await app.exec(`document.querySelector('[data-testid=tab-tags]').click(); return 1`);
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=tag-row] .name')].map(e => e.textContent).includes('#fresh')`, { message: "tags panel" });
    // A link typed in another note shows up in this note's backlinks.
    await app.open("Other.md");
    await app.fakeFocus();
    await app.focusEnd();
    await app.keys("link to [[N]] ");
    await eventually(() => env.vault.read("Other.md").includes("[[N]]"), { message: "Other saved" });
    await app.open("N.md");
    await app.exec(`document.querySelector('[data-testid=right-links]').click(); return 1`);
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=backlink-source]')].map(e => e.textContent.trim()).join() === 'Other'`, { timeout: 6000, message: "backlinks" });
    assert.deepEqual(await app.errors(), []);
  }, { shot: "ED-panels-inapp" });
});
