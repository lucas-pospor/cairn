// Regression tests for FINDING-007: an external link inside a note embed in
// Live Preview must not navigate the app's own webview to that site.
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ed_22.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { withApp, sleep } from "./adv_editor_lib.mjs";

const FILES = {
  "Host.md": "top line\n\n![[Inner]]\n\nend\n",
  "Inner.md": "Inner says: [example](https://example.com/from-embed)\n",
};
const LINK = '.cm-lp-embed .embed-body a[href^="https://"]';

async function whereAmI(app) {
  try {
    return await app.exec(`return { href: location.href, workspace: !!document.querySelector('.workspace'), title: document.title, histLen: history.length }`);
  } catch (e) {
    return { error: String(e.message).slice(0, 200) };
  }
}

async function realClick(app, css) {
  await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(css)})`, { message: css, timeout: 8000 });
  const c = await app.center(css);
  await app.clickAt(c.x, c.y);
}

test(
  "FINDING-007 (diagnostic): the click event after the handled mousedown reaches window with defaultPrevented=true",
  async () => {
    await withApp(FILES, async (app) => {
      await app.open("Host.md");
      await app.fakeFocus();
      await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(LINK)})`, { message: LINK, timeout: 8000 });
      // Bubble-phase listener on window runs after every app handler. It records
      // whether anyone cancelled the click, then cancels it itself so the page
      // survives long enough to read the record.
      await app.exec(`
        window.__ed22 = [];
        window.addEventListener('click', (e) => {
          const a = e.target.closest ? e.target.closest('a') : null;
          window.__ed22.push({
            href: a ? a.getAttribute('href') : null,
            defaultPrevented: e.defaultPrevented,
            openUrlCalls: window.__adv.invokes.filter(i => i.cmd === 'plugin:opener|open_url').map(i => i.body),
          });
          e.preventDefault();
        });
        return true;`);
      await realClick(app, LINK);
      await sleep(800);
      const rec = await app.exec(`return window.__ed22`);
      const where = await whereAmI(app);
      // open_url is requested after a dynamic import, so look again a bit later.
      const openedLater = (await app.invokes("plugin:opener|open_url")).map((i) => i.body);
      const toasts = await app.toasts();
      console.log(`diagnostic: click record ${JSON.stringify(rec)}; window ${JSON.stringify(where)}; open_url calls 800 ms later: ${JSON.stringify(openedLater)}; toasts: ${JSON.stringify(toasts)}`);
      assert.equal(rec.length, 1, "one click reached window");
      assert.equal(rec[0].defaultPrevented, true, `nobody cancelled the click on ${rec[0].href}; open_url requested too (system browser): ${JSON.stringify(openedLater)}`);
    });
  },
);

test(
  "FINDING-007: type, then click the embed's external link right away: the window stays on the app",
  async () => {
    await withApp(
      FILES,
      async (app, env) => {
        await app.open("Host.md");
        await app.fakeFocus();
        await app.focusEnd();
        await app.keys("typed");
        const inEditor = (await app.text()).includes("typed");
        const stateBefore = await app.saveState();
        const t0 = Date.now();
        await realClick(app, LINK);
        const clickMs = Date.now() - t0;
        await sleep(1500);
        const after = await whereAmI(app);
        await sleep(1000);
        const disk = env.vault.read("Host.md");
        // Can the user get back with the usual browser keys?
        await app.chord("", ""); // Alt+Left
        await sleep(1000);
        const afterAltLeft = await whereAmI(app);
        console.log(
          `type+click: typed text in editor before click: ${inEditor}; save state before click: ${JSON.stringify(stateBefore)}; click took ${clickMs} ms; ` +
            `window after: ${JSON.stringify(after)}; Host.md on disk: ${JSON.stringify(disk)}; after Alt+Left: ${JSON.stringify(afterAltLeft)}`,
        );
        assert.ok(inEditor, "precondition: typed text reached the editor");
        assert.ok(after.workspace, `window navigated to ${after.href} (${after.title}); typed text on disk: ${disk.includes("typed")}`);
      },
      { shot: "verify-ED-22-type-click" },
    );
  },
);

test(
  "FINDING-007: after autosave completed, clicking the embed's external link does not replace the app UI with the site",
  async () => {
    await withApp(FILES, async (app, env) => {
      await app.open("Host.md");
      await app.fakeFocus();
      await app.focusEnd();
      await app.keys("saved");
      await sleep(1500);
      const diskBefore = env.vault.read("Host.md");
      await realClick(app, LINK);
      await sleep(1500);
      const after = await whereAmI(app);
      console.log(`after autosave: disk before click ${JSON.stringify(diskBefore)}; window after: ${JSON.stringify(after)}`);
      assert.ok(diskBefore.includes("saved"), "precondition: autosave wrote the edit");
      assert.ok(after.workspace, `window navigated to ${after.href} (${after.title})`);
    });
  },
);
