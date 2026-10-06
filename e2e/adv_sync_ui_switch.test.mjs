// Switching vaults while a sync is running, through the
// real app. SyncManager::stop() only sets a flag, so the old vault's sync
// finishes in the background after the user switched. Its engine holds its
// own Arc<Vault>, so files land in the right folder, and the stopped manager
// sends no more events to the one window, which now shows the other vault.
//
// Run: scripts/e2e-headless.sh e2e/adv_sync_ui_switch.test.mjs   (about 1 min)

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, evidence, read, write, sleep, path, fs } from "./adv_sync_ui_lib.mjs";

const env = new Env("switch");
const A = env.dir("laptop");
const B = env.dir("work");
const P = env.dir("phone");
const PS = path.join(env.tmp, "phone-state");

before(async () => {
  write(path.join(A, "Plan.md"), "plan from the synced vault\n");
  write(path.join(A, "Todo.md"), "todo from the synced vault\n");
  write(path.join(A, "Other.md"), "other\n");
  write(path.join(B, "Plan.md"), "WORK PLAN (unsynced vault)\n");
  write(path.join(B, "Todo.md"), "WORK TODO (unsynced vault)\n");
  write(path.join(B, "Zeta.md"), "zeta\n");
  await env.startServer();
  await env.startProxy();
  await env.startApp(A);
  await env.connect({ server: env.purl });
  await sleep(2500); // let the manager's first scheduled sync pass
  const r = env.syncB(P, PS);
  assert.ok(!r.error, JSON.stringify(r));
  assert.equal(read(path.join(P, "Plan.md")), "plan from the synced vault\n");
});

after(async () => {
  await env.stop();
});

test(
  "a sync of the vault you just left does not touch the tabs, tree or status bar of the vault you switched to",
  async () => {
    const s = env.s;
    // Phone renames Plan.md and deletes Todo.md.
    fs.renameSync(path.join(P, "Plan.md"), path.join(P, "Plan renamed.md"));
    fs.rmSync(path.join(P, "Todo.md"));
    const r = env.syncB(P, PS);
    assert.ok(!r.error, JSON.stringify(r));

    // Laptop: start a sync whose changes request is slow (8 s), then switch vaults.
    env.delayChanges = 8000;
    env.log.length = 0;
    await s.exec(`document.querySelector('[data-testid=sync-indicator]').click()`);
    await eventually(() => env.log.some((l) => l.includes("DELAY")), { message: "slow changes request" });
    env.delayChanges = 0;
    await s.exec(`document.querySelector('footer.status .vault').click()`);
    await s.findWait("[data-testid=vault-path]");
    await env.setInput("vault-path", B);
    await s.exec(`document.querySelector('[data-testid=vault-open]').click()`);
    await env.openNote("Todo.md");
    await env.openNote("Plan.md", { newTab: true });
    const before = await s.exec(`return { tabs: [...document.querySelectorAll('[data-testid=tab]')].map(t => t.dataset.path), indicator: document.querySelector('[data-testid=sync-indicator]')?.textContent.trim() ?? null }`);

    // Wait for the old vault's sync to finish (its pull applies to A on disk).
    await eventually(() => fs.existsSync(path.join(A, "Plan renamed.md")) && !fs.existsSync(path.join(A, "Todo.md")), { timeout: 30000, message: "old vault's sync applied on disk" });
    await sleep(1500);
    const afterSync = await s.exec(`return {
      tabs: [...document.querySelectorAll('[data-testid=tab]')].map(t => t.dataset.path),
      tree: [...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path),
      indicator: document.querySelector('[data-testid=sync-indicator]')?.textContent.trim() ?? null,
      title: document.querySelector('[data-testid=sync-indicator]')?.title ?? null,
    }`);
    await env.shot("SU-01-after-old-sync.png");

    // Type in the "Plan" tab of the work vault and see where it is saved.
    await s.exec(`
      const v = document.querySelector('.cm-editor').__cairnView;
      v.dispatch({ changes: { from: v.state.doc.length, insert: 'typed in the work vault\\n' } });`);
    await sleep(2000);
    const bFiles = fs.readdirSync(B).filter((f) => !f.startsWith(".")).sort();
    const bPlan = read(path.join(B, "Plan.md"));
    const bRenamed = read(path.join(B, "Plan renamed.md"));
    const toasts = await s.exec(`return [...document.querySelectorAll('.toast, [role=status], [role=alert]')].map(t => t.textContent.trim()).filter(Boolean)`);

    // Settings > Sync for the work vault
    await env.openSyncSettings();
    const settingsText = await s.exec(`return document.querySelector('[data-testid=settings]')?.innerText.slice(0, 600)`);
    await env.closeSettings();
    const backend = await env.invoke("sync_status");

    // The banner offers "Save my version": what does it do?
    const banner = await s.exec(`return document.querySelector('[data-testid=conflict-banner]')?.textContent.trim() ?? null`);
    let afterSaveMine = null;
    if (banner) {
      await s.exec(`[...document.querySelectorAll('[data-testid=conflict-banner] button')].find(b => /Save my version/.test(b.textContent))?.click()`);
      await sleep(1500);
      afterSaveMine = {
        files: fs.readdirSync(B).filter((f) => !f.startsWith(".")).sort(),
        planRenamed: read(path.join(B, "Plan renamed.md")),
        plan: read(path.join(B, "Plan.md")),
      };
    }
    const report = { before, afterSync, bFiles, bPlan, bRenamed, toasts, banner, afterSaveMine, settingsText, backendStatusOfWorkVault: backend };
    evidence("SU-01.json", JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));

    const problems = [];
    if (JSON.stringify(afterSync.tabs) !== JSON.stringify(["Todo.md", "Plan.md"])) problems.push(`work vault tabs changed by the other vault's sync: ${JSON.stringify(afterSync.tabs)}`);
    if (afterSync.indicator) problems.push(`status bar of the unsynced work vault shows ${JSON.stringify(afterSync.indicator)}`);
    if (/Connected/.test(settingsText ?? "")) problems.push("Settings > Sync of the unsynced work vault says Connected");
    if (bRenamed !== null) problems.push(`typing in the work vault's Plan tab created "Plan renamed.md" there: ${JSON.stringify(bRenamed)}`);
    if (!bPlan?.includes("typed in the work vault")) problems.push(`work vault Plan.md did not get the typed text: ${JSON.stringify(bPlan)}`);
    assert.deepEqual(problems, []);
  },
);

test("switching back to the vault that was syncing: its state was saved for it, nothing duplicated (held up)", async () => {
  const s = env.s;
  await s.exec(`document.querySelector('footer.status .vault').click()`);
  await s.findWait("[data-testid=vault-path]");
  await env.setInput("vault-path", A);
  await s.exec(`document.querySelector('[data-testid=vault-open]').click()`);
  await s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Plan renamed.md"]')`, { timeout: 15000 });
  await sleep(3000); // first scheduled sync of the reopened vault
  await env.syncNowFromUi();
  const status = (await env.invoke("sync_status")).ok;
  const aFiles = fs.readdirSync(A).filter((f) => !f.startsWith(".")).sort();
  const r = env.syncB(P, PS);
  const pFiles = fs.readdirSync(P).filter((f) => !f.startsWith(".")).sort();
  console.log(JSON.stringify({ status, aFiles, pFiles, phone: r }));
  assert.equal(status.state, "idle", JSON.stringify(status));
  assert.deepEqual(status.conflicts, []);
  assert.deepEqual(aFiles, ["Other.md", "Plan renamed.md"]);
  assert.deepEqual(pFiles, aFiles);
  assert.equal(r.pulled, 0, JSON.stringify(r));
});
