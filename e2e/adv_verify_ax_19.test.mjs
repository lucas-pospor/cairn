// Reproduction for FINDING-205 ("status changes are silent").
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ax_19.test.mjs
//
// The other reproduction (adv_a11y_semantics.test.mjs) checks that the static
// "Saved" / "Synced" spans sit in a live region. That over-reaches for the
// save state: "Saved"/"Unsaved" flips on every keystroke and autosave, so it
// should not be live, and a save conflict is already announced by the
// role=alert conflict banner (EditorPane.svelte). This file checks the
// transitions that matter instead:
//   1. held up: a save conflict is announced (role=alert banner appears);
//   2. FINDING-205: a sync failure is not announced anywhere: the status bar
//      text changes to "Sync error" but no live region, toast or alert says so.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { spawn } from "node:child_process";
import { AxApp, K, SERVER, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v19-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

const log = (label, v) => console.log(`${label}:\n${typeof v === "string" ? v : JSON.stringify(v, null, 1)}`);

/** Record text that appears inside live regions (what a screen reader would speak), from now on. */
async function startAnnouncementLog() {
  await app.exec(`
    window.__axSaid = [];
    const live = (el) => { for (let e = el; e && e.nodeType === 1; e = e.parentElement) { const l = e.getAttribute('aria-live'); if (l && l !== 'off') return true; if (['status', 'alert', 'log'].includes(e.getAttribute('role'))) return true; } return false; };
    const note = (n) => { const el = n.nodeType === 1 ? n : n.parentElement; if (el && live(el)) { const t = (n.textContent || '').trim(); if (t) window.__axSaid.push(t); } };
    window.__axSayObs = new MutationObserver((ms) => { for (const m of ms) { if (m.type === 'characterData') note(m.target); for (const n of m.addedNodes) note(n); } });
    window.__axSayObs.observe(document.body, { subtree: true, childList: true, characterData: true });
    return 1`);
}
const stopAnnouncementLog = () => app.exec(`window.__axSayObs.disconnect(); return window.__axSaid`);

test("held up: a save conflict is announced (the conflict banner is role=alert and is inserted when the conflict happens)", async () => {
  await app.reset();
  await app.openNote("ideas", "Ideas.md");
  await startAnnouncementLog();
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length } }); return 1`);
  await app.keys(" mine");
  app.write("Ideas.md", "Theirs, written elsewhere.\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=conflict-banner]')`, { timeout: 6000, message: "conflict banner" });
  await sleep(100);
  const said = await stopAnnouncementLog();
  const r = await app.exec(`const b = document.querySelector('[data-testid=conflict-banner]'); return { role: b.getAttribute('role'), saveText: document.querySelector('[data-testid=save-state]').textContent.trim() }`);
  log("save conflict", { ...r, said });
  assert.equal(r.role, "alert");
  assert.ok(said.some((t) => t.includes("changed on disk")), "the conflict banner text reached a live region");
  // Leave the note as it is on disk.
  await app.exec(`document.querySelector('[data-testid=conflict-theirs]').click(); return 1`);
  await app.s.waitFor(`return !document.querySelector('[data-testid=conflict-banner]')`);
});

test("FINDING-205: a sync failure is silent: the status bar says 'Sync error' but nothing is announced", async () => {
  await app.reset();
  const port = 19000 + Math.floor(Math.random() * 900);
  const url = `http://127.0.0.1:${port}`;
  const server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: "ax-token-0123456789", CAIRN_DATA: path.join(app.tmp, "server-v19"), CAIRN_ADDR: `127.0.0.1:${port}` },
    stdio: "ignore",
  });
  let r;
  try {
    await eventually(async () => (await fetch(`${url}/health`)).ok, { message: "server up" });
    const res = await app.s.execAsync(
      `const done = arguments[arguments.length - 1];
       (async () => {
         document.querySelector('[data-testid=open-settings]').click();
         await new Promise(r => setTimeout(r, 200));
         document.querySelector('[data-testid=settings-sync]').click();
         await new Promise(r => setTimeout(r, 300));
         const set = (id, v) => { const i = document.querySelector('[data-testid=' + id + ']'); i.value = v; i.dispatchEvent(new Event('input', { bubbles: true })); };
         set('sync-server', arguments[0]); set('sync-token', 'ax-token-0123456789'); set('sync-pass', 'passphrase123'); set('sync-pass2', 'passphrase123');
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
    await app.s.waitFor(`return /Synced/.test(document.querySelector('[data-testid=sync-indicator]')?.textContent ?? '')`, { timeout: 15000, message: "synced" });
    // The server goes away; the user asks for a sync from the keyboard.
    server.kill();
    await eventually(async () => {
      try {
        await fetch(`${url}/health`);
        return false;
      } catch {
        return true;
      }
    }, { message: "server down" });
    await startAnnouncementLog();
    await app.palette("sync now");
    await app.s.waitFor(`return /Sync error/.test(document.querySelector('[data-testid=sync-indicator]')?.textContent ?? '')`, { timeout: 20000, message: "status bar shows Sync error" });
    await sleep(500);
    const said = await stopAnnouncementLog();
    r = await app.exec(`
      const ind = document.querySelector('[data-testid=sync-indicator]');
      let live = false;
      for (let e = ind; e; e = e.parentElement) if ((e.getAttribute('aria-live') && e.getAttribute('aria-live') !== 'off') || ['status', 'alert', 'log'].includes(e.getAttribute('role'))) live = true;
      return { indicator: ind.textContent.trim(), title: ind.getAttribute('title'), indicatorLive: live, toasts: [...document.querySelectorAll('.toast')].map(t => t.textContent.trim()), focus: __ax.desc(document.activeElement) };`);
    r.said = said;
  } finally {
    server.kill();
    await app.exec(`return window.__TAURI_INTERNALS__.invoke('sync_disconnect').then(() => 1, () => 0)`).catch(() => {});
  }
  log("after a failed sync", r);
  assert.ok(r.said.some((t) => /sync/i.test(t)), `status bar shows "${r.indicator}" (title "${r.title}") but nothing was announced (live-region text seen: ${JSON.stringify(r.said)})`);
});
