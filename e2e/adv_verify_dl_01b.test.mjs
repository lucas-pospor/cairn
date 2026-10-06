// Regression test for FINDING-001.
// The cases in adv_verify_dl_01.test.mjs need the conflict banner (or a
// failed-save toast) to be on screen before the user navigates away. This
// variant does not: another program writes the note while the user is
// typing, and the user clicks another note before the 600 ms autosave fires,
// so openNote()'s own flush() hits the conflict. With the defect, the same
// tab was then reused (dirty/conflict cleared, editorState dropped): the edit
// was gone and the user never saw a toast; the conflict banner only flashed
// for a few milliseconds (between flush() setting tab.conflict and openNote()
// clearing it). The test checks that the edit is kept, or that a lasting
// warning is shown.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_01b.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl01b");
});
after(async () => {
  await env?.dispose();
});

test(
  "external write during typing, then a plain click on another note before autosave keeps my edit (or at least warns)",
  async () => {
    const v = env.vault("v", { "A.md": "alpha\n", "B.md": "bravo\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("A.md");
      await app.source();
      await sleep(150);
      // Record whether the conflict banner or any toast ever appears.
      await app.exec(`
        window.__seen = { banner: false, bannerOn: null, bannerMs: 0, toasts: [] };
        new MutationObserver(() => {
          const on = !!document.querySelector('[data-testid=conflict-banner]');
          const now = performance.now();
          if (on && window.__seen.bannerOn == null) { window.__seen.banner = true; window.__seen.bannerOn = now; }
          if (!on && window.__seen.bannerOn != null) { window.__seen.bannerMs += now - window.__seen.bannerOn; window.__seen.bannerOn = null; }
          for (const t of document.querySelectorAll('.toast')) {
            const s = t.textContent.trim();
            if (!window.__seen.toasts.includes(s)) window.__seen.toasts.push(s);
          }
        }).observe(document.body, { subtree: true, childList: true, characterData: true });
      `);
      const t0 = Date.now();
      await app.insertEnd("MY IMPORTANT EDIT");
      v.write("A.md", "THEIRS\n"); // e.g. a sync tool or another editor
      // Plain click on B right away (well inside the 600 ms autosave delay).
      await app.s.click(await app.s.find('[data-testid=tree-row][data-path="B.md"]'));
      const clickedAfter = Date.now() - t0;
      await eventually(async () => (await app.activeTab()) === "B.md", { message: "B active" });
      await sleep(1000);
      const tabsAfter = await app.tabs();
      const seen = await app.exec(`return window.__seen`);
      await app.openFromTree("A.md");
      const ed = await app.editorText();
      const where = [ed, ...v.listDisk().filter((p) => p.endsWith(".md")).map((p) => v.read(p))];
      assert.ok(clickedAfter < 600, `click came too late for this scenario (${clickedAfter} ms)`);
      assert.ok(
        // A banner that flashes for a few milliseconds while the tab is being
        // replaced is not a warning anyone can read or act on.
        where.some((t) => t.includes("MY IMPORTANT EDIT")) || seen.bannerMs > 500 || seen.bannerOn != null || seen.toasts.length > 0,
        `edit gone with no warning: clicked after ${clickedAfter} ms, tabs after click=${JSON.stringify(tabsAfter)}, ` +
          `banner ever shown=${seen.banner} for ${Math.round(seen.bannerMs)} ms, toasts=${JSON.stringify(seen.toasts)}, editor=${JSON.stringify(ed)}, disk A=${JSON.stringify(v.read("A.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);
