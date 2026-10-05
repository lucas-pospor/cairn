// Editing, conflicts and version history while the app's
// sync thread runs, with sync_dir as a second device ("phone").
//
// Run: scripts/e2e-headless.sh e2e/adv_sync_ui_edit.test.mjs   (about 2 min)

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, Key, eventually, evidence, read, write, sleep, path, fs } from "./adv_sync_ui_lib.mjs";

const env = new Env("edit");
const A = env.dir("laptop");
const P = env.dir("phone");
const PS = path.join(env.tmp, "phone-state");
const syncP = () => {
  const r = env.syncB(P, PS);
  if (r.error) throw new Error("phone sync failed: " + r.error);
  return r;
};

before(async () => {
  write(path.join(A, "Shared.md"), "line one\nline two\nline three\n");
  write(path.join(A, "Restore.md"), "version one\n");
  write(path.join(A, "Conf.md"), "original\n");
  write(path.join(A, "Laptop.md"), "laptop note\n");
  write(path.join(A, "Many.md"), "rev 0\n");
  write(path.join(A, "Gone.md"), "the old Gone text\n");
  write(path.join(A, "Old name.md"), "renamed note, first version\n");
  await env.startServer();
  await env.startProxy();
  await env.startApp(A);
  await env.connect({ server: env.purl });
  await sleep(2500);
  syncP();
  assert.equal(read(path.join(P, "Shared.md")), "line one\nline two\nline three\n");
});

after(async () => {
  await env.stop();
});

async function waitIdle(timeout = 30000) {
  await eventually(async () => (await env.invoke("sync_status")).ok?.state !== "syncing", { timeout, message: "sync idle" });
}

/** Sync the app through its own backend command and wait for it to finish. */
async function appSync() {
  const r = await env.invoke("sync_now");
  await sleep(200);
  return r.ok ?? r;
}

async function openHistory(p) {
  const s = env.s;
  await env.openNote(p);
  await s.exec(`document.querySelector('.cm-content')?.focus()`);
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "version history");
  await sleep(200);
  await s.keys(Key.enter);
  await s.findWait("[data-testid=history]", 10000);
  await s.waitFor(`return !document.querySelector('[data-testid=history] li.muted')`, { timeout: 30000 });
}

async function closeHistory() {
  await env.s.exec(`document.querySelector('[data-testid=history] button[title=Close]')?.click()`);
  await env.s.waitFor(`return !document.querySelector('[data-testid=history]')`);
}

async function allRevisionTexts(p) {
  const h = await env.invoke("sync_history", { path: p });
  const texts = [];
  for (const e of h.ok ?? []) {
    const r = await env.invoke("sync_revision", { seq: e.seq });
    texts.push(r.ok?.text ?? `ERR ${r.err}`);
  }
  return texts;
}

test(
  "restoring a version keeps the current text in the history, even when it was not synced yet",
  async () => {
    const s = env.s;
    // A second synced version, so there is something older to restore.
    write(path.join(A, "Restore.md"), "version two\n");
    await sleep(800);
    await appSync();
    await waitIdle();
    // Laptop goes offline for a while and keeps writing.
    env.proxyDown = true;
    await env.openNote("Restore.md");
    await s.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: v.state.doc.length, insert: 'OFFLINE WORK: an hour of writing\\n' } });`);
    await eventually(() => read(path.join(A, "Restore.md"))?.includes("OFFLINE WORK"), { timeout: 5000, message: "autosaved" });
    await sleep(6000); // the poked sync fails (offline)
    const offlineIndicator = await env.indicator();
    // Back online; before the next scheduled sync the user looks at the history.
    env.proxyDown = false;
    await openHistory("Restore.md");
    const entries = await s.exec(`return [...document.querySelectorAll('[data-testid=history-entry]')].map(e => e.textContent.trim())`);
    const dialogText = [];
    await s.exec(`[...document.querySelectorAll('[data-testid=history-entry]')].at(-1).click()`);
    await sleep(400);
    await s.exec(`document.querySelector('[data-testid=history-restore]').click()`);
    await s.findWait("[data-testid=dialog-ok]");
    dialogText.push(await s.exec(`return [...document.querySelectorAll('[role=dialog]')].map(d => d.textContent.trim()).join(' | ')`));
    await s.click(await s.find("[data-testid=dialog-ok]"));
    await eventually(() => read(path.join(A, "Restore.md")) === "version one\n", { message: "restored on disk" });
    await sleep(6000); // the restore pokes a sync
    await waitIdle();
    const revs = await allRevisionTexts("Restore.md");
    const onDisk = read(path.join(A, "Restore.md"));
    const report = { offlineIndicator, entries, dialogText, onDisk, serverRevisions: revs };
    evidence("SU-05.json", JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    assert.ok(revs.some((t) => t.includes("OFFLINE WORK")) || onDisk.includes("OFFLINE WORK"), "the offline text is in neither the note nor its history");
  },
);

test(
  "typing in a note while the sync pulls another device's edit to a different line of it: both edits survive",
  async () => {
    const s = env.s;
    await env.openNote("Shared.md");
    // Phone edits line one.
    write(path.join(P, "Shared.md"), "line one (phone)\nline two\nline three\n");
    syncP();
    // The user types at the end of line three for ~4 s (no pause of 600 ms, so
    // nothing is autosaved), while the background sync runs.
    let fired = false;
    for (let i = 0; i < 26; i++) {
      await s.exec(`const v = document.querySelector('.cm-editor').__cairnView; const l = v.state.doc.line(3); v.dispatch({ changes: { from: l.to, insert: arguments[0] } });`, "abcdefghijklmnopqrstuvwxyz"[i]);
      if (i === 4 && !fired) {
        fired = true;
        await s.exec(`window.__TAURI_INTERNALS__.invoke('sync_now')`);
      }
      await sleep(150);
    }
    await eventually(() => read(path.join(A, "Shared.md"))?.startsWith("line one (phone)"), { timeout: 15000, message: "phone edit pulled onto disk" });
    await sleep(1500); // autosave after typing stops
    const banner = await s.exec(`return document.querySelector('[data-testid=conflict-banner]')?.textContent.trim() ?? null`);
    const editor = await env.editorText();
    const disk1 = read(path.join(A, "Shared.md"));
    let afterKeepMine = null;
    if (banner) {
      await s.exec(`document.querySelector('[data-testid=conflict-mine]').click()`);
      await sleep(1000);
      afterKeepMine = read(path.join(A, "Shared.md"));
      await appSync();
      await waitIdle();
      syncP();
    }
    const phone = read(path.join(P, "Shared.md"));
    const report = { banner, editor, diskWhenBannerShown: disk1, afterKeepMine, phoneAfterSync: phone };
    evidence("SU-06.json", JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    const final = afterKeepMine ?? read(path.join(A, "Shared.md"));
    assert.ok(final.includes("line one (phone)") && final.includes("line threeabcdefghijklmnopqrstuvwxyz"), `final text ${JSON.stringify(final)}; banner ${JSON.stringify(banner)}`);
  },
);

test(
  "the conflict count and list follow the conflict copies: deleting the copy clears it",
  async () => {
    const s = env.s;
    // Close any banner state from earlier tests.
    write(path.join(P, "Conf.md"), "phone rewrote this\n");
    syncP();
    write(path.join(A, "Conf.md"), "laptop rewrote this\n");
    await sleep(800);
    await appSync();
    await waitIdle();
    const st = (await env.invoke("sync_status")).ok;
    const copy = st.conflicts.find((c) => c.startsWith("Conf"));
    assert.ok(copy, `no conflict copy reported: ${JSON.stringify(st)}`);
    const before = await env.indicator();
    // Resolve: the user deletes the conflict copy (same command as the tree's Delete).
    const del = await env.invoke("delete_entry", { path: copy });
    assert.ok(!del.err, JSON.stringify(del));
    await sleep(800);
    await appSync();
    await waitIdle();
    await sleep(500);
    const afterIndicator = await env.indicator();
    await env.openSyncSettings();
    const listed = await s.exec(`return [...document.querySelectorAll('[data-testid=settings] .linkish')].map(b => b.textContent.trim())`);
    // Click the stale entry: what happens?
    await s.exec(`[...document.querySelectorAll('[data-testid=settings] .linkish')].find(b => b.textContent.trim() === arguments[0])?.click()`, copy);
    await sleep(1000);
    const tabState = await s.exec(`return { tabs: [...document.querySelectorAll('[data-testid=tab]')].map(t => t.dataset.path), pane: document.querySelector('[data-testid=editor]')?.parentElement?.innerText.slice(0, 200) }`);
    await env.shot("SU-07-stale-conflict.png");
    const recreated = fs.existsSync(path.join(A, copy));
    const report = { copy, before, afterIndicator, listedInSettings: listed, tabState, recreatedOnDisk: recreated };
    evidence("SU-07.json", JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    await s.exec(`[...document.querySelectorAll('[data-testid=tab]')].filter(t => t.dataset.path === arguments[0]).forEach(t => t.querySelector('button')?.click())`, copy);
    // With the copy gone from the list nothing was clicked, so Settings is
    // still open; close it so the tests after this one start without it.
    await env.closeSettings();
    assert.ok(!/conflict/.test(afterIndicator) && !listed.includes(copy), `after deleting ${copy}: status bar ${JSON.stringify(afterIndicator)}, Settings lists ${JSON.stringify(listed)}`);
  },
);

test("version history of a note with 150 revisions lists them all promptly (held up)", async () => {
  for (let i = 1; i <= 150; i++) {
    write(path.join(P, "Many.md"), `rev ${i}\n`);
    syncP();
  }
  await appSync();
  await waitIdle();
  await eventually(() => read(path.join(A, "Many.md")) === "rev 150\n", { message: "pulled" });
  const t0 = Date.now();
  await openHistory("Many.md");
  await env.s.waitFor(`return document.querySelectorAll('[data-testid=history-entry]').length >= 151 && document.querySelector('[data-testid=history] .preview')?.textContent.includes('rev 150')`, { timeout: 15000 });
  const ms = Date.now() - t0;
  const n = await env.s.exec(`return document.querySelectorAll('[data-testid=history-entry]').length`);
  await closeHistory();
  console.log(`history: ${n} entries in ${ms} ms`);
  evidence("SU-history-many.txt", `history: ${n} entries in ${ms} ms\n`);
  assert.ok(n >= 151);
  assert.ok(ms < 8000, `${ms} ms`);
});

test(
  "version history of a note renamed since the last sync",
  async () => {
    const s = env.s;
    const r = await env.invoke("rename_entry", { from: "Old name.md", to: "New name.md" });
    assert.ok(!r.err, JSON.stringify(r));
    await sleep(300);
    await openHistory("New name.md");
    const immediately = await s.exec(`return document.querySelector('[data-testid=history]')?.innerText.slice(0, 300)`);
    await closeHistory();
    await sleep(5000); // debounced sync uploads the rename
    await waitIdle();
    await openHistory("New name.md");
    const later = await s.exec(`return { n: document.querySelectorAll('[data-testid=history-entry]').length, text: document.querySelector('[data-testid=history]')?.innerText.slice(0, 300) }`);
    await closeHistory();
    const report = { immediately, later };
    evidence("SU-08.json", JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    assert.ok(!/has not been synced yet/.test(immediately), `history right after the rename: ${JSON.stringify(immediately)}`);
  },
);

test(
  "restoring the 'deleted' entry of a deleted-and-recreated note",
  async () => {
    const s = env.s;
    let r = await env.invoke("delete_entry", { path: "Gone.md" });
    assert.ok(!r.err, JSON.stringify(r));
    await sleep(500);
    await appSync();
    await waitIdle();
    r = await env.invoke("create_note", { path: "Gone.md", content: "a brand new Gone note\n" });
    assert.ok(!r.err, JSON.stringify(r));
    await sleep(500);
    await appSync();
    await waitIdle();
    await openHistory("Gone.md");
    const entries = await s.exec(`return [...document.querySelectorAll('[data-testid=history-entry]')].map(e => e.textContent.trim())`);
    const idx = entries.findIndex((e) => /deleted/.test(e));
    assert.ok(idx > 0, `no deleted entry: ${JSON.stringify(entries)}`);
    await s.exec(`document.querySelectorAll('[data-testid=history-entry]')[arguments[0]].click()`, idx);
    await sleep(500);
    const preview = await s.exec(`return document.querySelector('[data-testid=history] .preview')?.textContent`);
    const restoreEnabled = await s.exec(`return !document.querySelector('[data-testid=history-restore]').disabled`);
    let disk = read(path.join(A, "Gone.md"));
    if (restoreEnabled) {
      await s.exec(`document.querySelector('[data-testid=history-restore]').click()`);
      await s.click(await s.findWait("[data-testid=dialog-ok]"));
      await sleep(1000);
      disk = read(path.join(A, "Gone.md"));
    } else await closeHistory();
    const report = { entries, preview, restoreEnabled, diskAfter: disk };
    evidence("SU-09.json", JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    assert.ok(!restoreEnabled || disk !== "", `restoring the deleted entry left Gone.md as ${JSON.stringify(disk)}`);
  },
);

test("conflict copies are still listed after the app restarts", async () => {
  write(path.join(P, "Conf.md"), "phone, once more\n");
  syncP();
  write(path.join(A, "Conf.md"), "laptop, once more\n");
  await sleep(800);
  await appSync();
  await waitIdle();
  const copy = (await env.invoke("sync_status")).ok.conflicts.find((c) => c.startsWith("Conf"));
  assert.ok(copy, "no conflict copy reported");
  // Quit and start the app again on the same vault and app data.
  await env.s.close();
  const exited = new Promise((r) => (env.drv.proc.exitCode === null ? env.drv.proc.once("exit", r) : r()));
  env.drv.proc.kill();
  await exited;
  await env.startApp(A);
  const listed = (await env.invoke("sync_status")).ok.conflicts;
  await env.openSyncSettings();
  const shown = await env.s.exec(`return [...document.querySelectorAll('[data-testid=settings] .linkish')].map(b => b.textContent.trim())`);
  await env.closeSettings();
  assert.deepEqual({ listed, shown }, { listed: [copy], shown: [copy] });
});
