// Regression tests for FINDING-041 (relative Markdown links in Live Preview
// table / embed widgets were followed by the webview), see also
// adv_verify_ed_23.test.mjs.
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ed_23_02.test.mjs
//
// Covers both the broken-feature part and the data-loss part:
//   1. control: the same relative link as plain inline text in Live Preview
//      opens Other.md (no reload);
//   2. embed case, click 1.5 s after typing (autosave has run): the link opens
//      Other.md without reloading the page;
//   3. table case, click about 200 ms after typing (inside the 600 ms
//      autosave debounce): the pending text still reaches the disk.

import { test } from "node:test";
import assert from "node:assert/strict";
import { withApp, sleep } from "./adv_editor_lib.mjs";

async function where(app) {
  try {
    return await app.exec(`return { href: location.href, workspace: !!document.querySelector('.workspace'), adv: !!window.__adv }`);
  } catch (e) {
    return { error: String(e.message).slice(0, 200) };
  }
}

async function clickCss(app, css) {
  await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(css)})`, { message: css, timeout: 8000 });
  const c = await app.center(css);
  await app.clickAt(c.x, c.y);
}

test("FINDING-041 control: an inline relative link in Live Preview opens the note without reloading", async () => {
  let r;
  await withApp(
    { "T.md": "top\n\nsee [other note](Other.md) here\n\nend\n", "Other.md": "# Other\n" },
    async (app, env) => {
      await app.open("T.md");
      await app.fakeFocus();
      await app.focusEnd();
      await app.keys("typed");
      await sleep(1500);
      await clickCss(app, '.cm-lp-link[data-url="Other.md"]');
      await sleep(1500);
      const after = await where(app);
      r = { after, tab: after.workspace ? await app.activeTab() : null, saved: env.vault.read("T.md").includes("typed") };
    },
    { shot: "verify2-ED-23-inline-control" },
  );
  console.log(`inline control: ${JSON.stringify(r)}`);
  assert.equal(r.after.adv, true, `page reloaded (${r.after.href})`);
  assert.equal(r.tab, "Other.md");
});

test(
  "FINDING-041: relative link in a Live Preview embed, clicked after autosave, opens the note without reloading the app",
  async () => {
    let r;
    await withApp(
      { "T.md": "top\n\n![[Inner]]\n\nend\n", "Inner.md": "see [other note](Other.md)\n", "Other.md": "# Other\n" },
      async (app, env) => {
        await app.open("T.md");
        await app.fakeFocus();
        await app.focusEnd();
        await app.keys("typed");
        await sleep(1500);
        const savedBefore = env.vault.read("T.md").includes("typed");
        await clickCss(app, '.cm-lp-embed .embed-body a[href="Other.md"]');
        await sleep(1500);
        const after = await where(app);
        r = { savedBefore, after, tab: after.workspace ? await app.activeTab() : null, savedAfter: env.vault.read("T.md").includes("typed") };
      },
      { shot: "verify2-ED-23-embed-after-autosave" },
    );
    console.log(`embed link after autosave: ${JSON.stringify(r)}`);
    assert.equal(r.after.adv, true, `page reloaded (window at ${r.after.href}); typed text on disk: ${r.savedAfter}`);
    assert.equal(r.tab, "Other.md", `active tab is ${r.tab}`);
  },
);

test(
  "FINDING-041: relative link in a Live Preview table, clicked inside the autosave debounce, keeps the pending text",
  async () => {
    let r;
    await withApp(
      { "T.md": "top\n\n| Col | Link |\n|---|---|\n| a | [other note](Other.md) |\n\nend\n", "Other.md": "# Other\n" },
      async (app, env) => {
        await app.open("T.md");
        await app.fakeFocus();
        const css = '.cm-lp-table a[href="Other.md"]';
        await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(css)})`, { message: css, timeout: 8000 });
        await app.focusEnd();
        const c = await app.center(css);
        const t0 = Date.now();
        await app.keys("typed");
        await sleep(200);
        await app.clickAt(c.x, c.y);
        const clickMs = Date.now() - t0;
        await sleep(2000);
        const after = await where(app);
        r = { clickMs, after, tab: after.workspace ? await app.activeTab() : null, savedAfter: env.vault.read("T.md").includes("typed") };
      },
      { shot: "verify2-ED-23-table-in-debounce" },
    );
    console.log(`table link inside debounce: ${JSON.stringify(r)}`);
    assert.equal(r.savedAfter, true, `typed text lost on disk (click ${r.clickMs} ms after typing started; window at ${r.after.href})`);
  },
);
