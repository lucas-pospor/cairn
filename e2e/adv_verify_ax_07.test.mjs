// Regression test for FINDING-108 (the version history modal neither took
// focus nor closed with Escape).
//
// Beyond the other reproduction (adv_a11y_keyboard.test.mjs), this one:
// - logs every keydown at window level (capture), so we know the Escape key
//   really reached the page and where it was targeted;
// - checks that the dialog takes focus when opened, is aria-modal, and closes
//   on the first Escape;
// - if it stays open, runs a control: Tab until focus is inside the dialog,
//   counting the presses, then Escape again.
//
// With the defect, focus stayed on BODY, aria-modal was null, the Escape
// keydown reached the page with target BODY and the dialog stayed open; ONE
// Tab landed on the dialog's Close button (WebKit's focus-navigation starting
// point is where the removed palette was, right before the dialog), and
// Escape then closed it. So the impact was "Escape needs a Tab first / nothing
// announced to a screen reader", not "the dialog cannot be closed from the
// keyboard".
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_07.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { spawn } from "node:child_process";
import { AxApp, K, SERVER, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v07-");
let server;
let url;

before(async () => {
  await app.start();
  const port = 19000 + Math.floor(Math.random() * 900);
  url = `http://127.0.0.1:${port}`;
  server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: "axv07-token-0123456789", CAIRN_DATA: path.join(app.tmp, "server"), CAIRN_ADDR: `127.0.0.1:${port}` },
    stdio: "ignore",
  });
  await eventually(async () => (await fetch(`${url}/health`)).ok, { message: "server up" });
});

after(async () => {
  await app.exec(`return window.__TAURI_INTERNALS__.invoke('sync_disconnect').then(() => 1, () => 0)`).catch(() => {});
  server?.kill();
  await app.stop();
});

async function connectSync() {
  const res = await app.s.execAsync(
    `const done = arguments[arguments.length - 1];
     (async () => {
       document.querySelector('[data-testid=open-settings]').click();
       await new Promise(r => setTimeout(r, 200));
       document.querySelector('[data-testid=settings-sync]').click();
       await new Promise(r => setTimeout(r, 300));
       const set = (id, v) => { const i = document.querySelector('[data-testid=' + id + ']'); i.value = v; i.dispatchEvent(new Event('input', { bubbles: true })); };
       set('sync-server', arguments[0]); set('sync-token', 'axv07-token-0123456789'); set('sync-pass', 'passphrase123'); set('sync-pass2', 'passphrase123');
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
}

const historyOpen = () => app.exec(`return !!document.querySelector('[data-testid=history]')`);
const focusInside = () => app.exec(`return !!document.activeElement?.closest('[data-testid=history]')`);

async function openHistory() {
  await app.palette("version history");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=history]')`, { message: "history open" });
  await sleep(300);
  await app.exec(`window.__keylog = []; addEventListener('keydown', e => window.__keylog.push({ key: e.key, target: __ax.desc(e.target) }), true); return 1`);
}

test(
  "FINDING-108: Version history takes focus, is aria-modal and closes with Escape",
  async () => {
    await app.reset();
    await connectSync();
    await app.openNote("ideas", "Ideas.md");
    const facts = {};

    // 1. As the user gets it: open from the palette, press Escape.
    await openHistory();
    facts.focusAfterOpen = await app.focus();
    facts.focusInsideAfterOpen = await focusInside();
    facts.ariaModal = await app.exec(`return document.querySelector('[data-testid=history]').getAttribute('aria-modal')`);
    await app.keys(K.esc);
    await sleep(300);
    facts.escapeLog = await app.exec(`return window.__keylog.filter(k => k.key === 'Escape')`);
    facts.openAfterEscape = await historyOpen();

    // 2. Control: keyboard-only route into the dialog (Tab until focus is inside), then Escape.
    let tabs = 0;
    if (facts.openAfterEscape) {
      while (tabs < 120 && !(await focusInside())) {
        await app.keys(K.tab);
        tabs++;
      }
      facts.tabsToReachDialog = (await focusInside()) ? tabs : `not reached in ${tabs}`;
      facts.focusWhenInside = await app.focus();
      await app.keys(K.esc);
      await sleep(300);
      facts.openAfterEscapeFromInside = await historyOpen();
    }
    await app.shot("AX-07-verify.png");
    console.log("AX-07 facts:", JSON.stringify(facts, null, 2));

    // The key really reached the page (so the harness is not the problem).
    assert.ok(facts.escapeLog.length >= 1, "Escape keydown reached the page");
    // Control (run only if the first Escape left it open): once focus is inside, Escape closes it.
    if (typeof facts.tabsToReachDialog === "number") assert.equal(facts.openAfterEscapeFromInside, false, "Escape closes once focus is inside");

    const problems = [];
    if (!facts.focusInsideAfterOpen) problems.push(`focus after opening: ${facts.focusAfterOpen}`);
    if (facts.ariaModal !== "true") problems.push(`aria-modal=${facts.ariaModal}`);
    if (facts.openAfterEscape) problems.push(`Escape (target ${facts.escapeLog.map((k) => k.target).join(",")}) left Version history open; ${facts.tabsToReachDialog} Tab presses needed to get focus into the dialog`);
    assert.deepEqual(problems, []);
  },
);
