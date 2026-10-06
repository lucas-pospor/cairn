// Regression test for FINDING-041 (relative Markdown links in Live Preview
// table / embed widgets were followed by the webview).
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ed_23.test.mjs
//
// The defect had two symptoms:
//   1. the link did not open the note and the page reloaded (any timing);
//   2. typed text was lost, which only happened when the click came inside
//      the 600 ms autosave debounce. Here the click comes 1.5 s after typing,
//      so this test checks the first: the link opens the note, no reload.

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

test(
  "FINDING-041: relative link in a Live Preview table, clicked after autosave, opens the note without reloading the app",
  async () => {
    const css = '.cm-lp-table a[href="Other.md"]';
    let r;
    await withApp(
      { "T.md": "top\n\n| Col | Link |\n|---|---|\n| a | [other note](Other.md) |\n\nend\n", "Other.md": "# Other\n" },
      async (app, env) => {
        await app.open("T.md");
        await app.fakeFocus();
        await app.focusEnd();
        await app.keys("typed");
        await sleep(1500); // well past the 600 ms autosave debounce
        const savedBefore = env.vault.read("T.md").includes("typed");
        await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(css)})`, { message: css, timeout: 8000 });
        const c = await app.center(css);
        await app.clickAt(c.x, c.y);
        await sleep(1500);
        const after = await where(app);
        const tab = after.workspace ? await app.activeTab() : null;
        r = { savedBefore, after, tab, savedAfter: env.vault.read("T.md").includes("typed") };
      },
      { shot: "verify-ED-23-table-after-autosave" },
    );
    console.log(`table link after autosave: ${JSON.stringify(r)}`);
    assert.equal(r.after.adv, true, `page reloaded (window at ${r.after.href})`);
    assert.equal(r.tab, "Other.md", `active tab is ${r.tab}`);
  },
);
