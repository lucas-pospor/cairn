// Sync setup and connection errors as the user sees them in
// the real app (Settings > Sync and the status bar). A second device
// (sync_dir) creates the server vault "e2e" first, as a phone would.
//
// Run: scripts/e2e-headless.sh e2e/adv_sync_ui_setup.test.mjs   (about 2.5 min: one case waits for the 60 s timeout)

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, evidence, read, write, sleep, path, fs, Key, PASS, TOKEN } from "./adv_sync_ui_lib.mjs";

const env = new Env("setup");
const A = env.dir("laptop");
const P = env.dir("phone");
const PS = path.join(env.tmp, "phone-state");
const results = {};

before(async () => {
  write(path.join(A, "Laptop.md"), "from the laptop\n");
  write(path.join(P, "From phone.md"), "from the phone\n");
  await env.startServer();
  await env.startProxy();
  const r = env.syncB(P, PS);
  assert.ok(!r.error, JSON.stringify(r));
  await env.startApp(A);
  await env.openSyncSettings();
});

after(async () => {
  evidence("SU-setup.json", JSON.stringify(results, null, 2));
  await env.stop();
});

/** Fill the form, press Connect, wait for an error or success; returns what the user sees. */
async function attempt(name, opts, timeout = 90000) {
  const s = env.s;
  const t0 = Date.now();
  await env.fillSetup(opts);
  await s.waitFor(
    `return !!document.querySelector('[data-testid=sync-error]') || !!document.querySelector('[data-testid=sync-state]')`,
    { timeout },
  );
  const out = {
    ms: Date.now() - t0,
    error: await s.exec(`return document.querySelector('[data-testid=sync-error]')?.textContent.trim() ?? null`),
    connected: await s.exec(`return !!document.querySelector('[data-testid=sync-state]')`),
  };
  results[name] = out;
  console.log(name, JSON.stringify(out));
  return out;
}

test("wrong passphrase for an existing server vault: clear message, nothing configured (held up)", async () => {
  const r = await attempt("wrong passphrase", { server: env.surl, pass: "not the right passphrase" });
  assert.equal(r.connected, false);
  assert.match(r.error, /wrong passphrase/i);
});

test("wrong token: clear message (held up)", async () => {
  const r = await attempt("wrong token", { server: env.surl, token: "nope-nope-nope-nope" });
  assert.equal(r.connected, false);
  assert.match(r.error, /rejected the token/i);
});

test("nothing listening at the URL: 'cannot reach the server' quickly (held up)", async () => {
  const r = await attempt("unreachable", { server: `http://127.0.0.1:${env.sport + 3999}` });
  assert.equal(r.connected, false);
  assert.match(r.error, /cannot reach the server/i);
  assert.ok(r.ms < 10000, `took ${r.ms} ms`);
});

test(
  "server URL typed without http(s):// gets an understandable message",
  async () => {
    const r = await attempt("no scheme", { server: `127.0.0.1:${env.sport}` });
    assert.equal(r.connected, false);
    // The user typed a host and port; the message should say the URL needs https:// (or http://).
    assert.match(r.error ?? "", /https?:\/\//, `message: ${JSON.stringify(r.error)}`);
  },
);

test("URL of a web server that is not a Cairn server (wrong path prefix behind a reverse proxy)", async () => {
  const r = await attempt("wrong path", { server: `${env.surl}/notes` });
  assert.equal(r.connected, false);
  assert.ok(r.error && r.error.length > 0);
});

test(
  "a server that accepts the connection and never answers: the user can give up, and the app stays usable",
  async () => {
    const s = env.s;
    env.hangAll = true;
    env.log.length = 0;
    const t0 = Date.now();
    await env.fillSetup({ server: env.purl });
    await sleep(3000);
    const at3s = {
      button: await s.exec(`return document.querySelector('[data-testid=sync-connect]')?.textContent.trim()`),
      cancel: await s.exec(`return [...document.querySelectorAll('[data-testid=settings] button')].some(b => /cancel|stop/i.test(b.textContent))`),
    };
    // Close Settings and check the editor still works.
    await env.closeSettings();
    await env.openNote("Laptop.md");
    await s.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: v.state.doc.length, insert: 'typed while connecting\\n' } });`);
    await eventually(() => read(path.join(A, "Laptop.md"))?.includes("typed while connecting"), { timeout: 5000, message: "autosave during setup" });
    // Reopen Settings: what does it show?
    await env.openSyncSettings();
    const reopened = {
      button: await s.exec(`return document.querySelector('[data-testid=sync-connect]')?.textContent.trim() ?? null`),
      disabled: await s.exec(`return document.querySelector('[data-testid=sync-connect]')?.disabled ?? null`),
      serverField: await s.exec(`return document.querySelector('[data-testid=sync-server]')?.value ?? null`),
    };
    // Wait for the first attempt to fail by timeout.
    while (Date.now() - t0 < 66000) await sleep(1000);
    const statusAfterTimeout = await env.invoke("sync_status");
    env.hangAll = false;
    for (const r of env.held) r.destroy();
    env.held.clear();
    const after = {
      error: await s.exec(`return document.querySelector('[data-testid=sync-error]')?.textContent.trim() ?? null`),
      button: await s.exec(`return document.querySelector('[data-testid=sync-connect]')?.textContent.trim() ?? null`),
      requestsHeld: env.log.filter((l) => l.includes("HANG")).length,
      statusAfterTimeout,
    };
    const out = { at3s, reopened, after, ms: Date.now() - t0 };
    results["hung server"] = out;
    console.log("hung server", JSON.stringify(out, null, 2));
    const problems = [];
    if (!at3s.cancel) problems.push(`no Cancel while ${JSON.stringify(at3s.button)}`);
    if (reopened.button !== "Connecting…") problems.push(`after reopening Settings the attempt in progress is not shown (button ${JSON.stringify(reopened.button)}, server field ${JSON.stringify(reopened.serverField)})`);
    if (!after.error) problems.push("the timeout error of the first attempt is never shown (Settings was reopened)");
    assert.deepEqual(problems, []);
  },
);

test("Cancel gives up a setup that waits on a server that never answers; nothing is set up and the next Connect does not wait", async () => {
  const s = env.s;
  if (!(await s.exec(`return !!document.querySelector('[data-testid=sync-server]')`))) await env.openSyncSettings();
  env.hangAll = true;
  env.log.length = 0;
  await env.fillSetup({ server: env.purl });
  await eventually(() => env.log.some((l) => l.includes("HANG")), { message: "hung setup request" });
  const t0 = Date.now();
  await s.click(await s.findWait("[data-testid=sync-cancel]"));
  await s.waitFor(`return document.querySelector('[data-testid=sync-connect]')?.textContent.trim() === 'Connect and sync'`, { timeout: 5000 });
  const cancelled = {
    ms: Date.now() - t0,
    disabled: await s.exec(`return document.querySelector('[data-testid=sync-connect]').disabled`),
    error: await s.exec(`return document.querySelector('[data-testid=sync-error]')?.textContent.trim() ?? null`),
    configured: (await env.invoke("sync_status")).ok?.configured,
  };
  // The request is still held by the server; a new attempt must not queue behind it.
  const next = await attempt("after cancel", { server: env.surl, pass: "not the right passphrase" }, 10000);
  env.hangAll = false;
  for (const r of env.held) r.destroy();
  env.held.clear();
  results["cancel"] = { cancelled, next };
  console.log("cancel", JSON.stringify({ cancelled, next }));
  assert.deepEqual({ ...cancelled, ms: cancelled.ms < 2000 }, { ms: true, disabled: false, error: null, configured: false });
  assert.match(next.error ?? "", /wrong passphrase/i);
  assert.ok(next.ms < 10000, `took ${next.ms} ms`);
});

test(
  "a mistyped vault name does not silently create a second, empty vault on the server",
  async () => {
    const s = env.s;
    // Make sure the form is shown (a previous test may have connected).
    if (await s.exec(`return !!document.querySelector('[data-testid=sync-state]')`)) {
      await s.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Turn off').click()`);
      await s.click(await s.findWait("[data-testid=dialog-ok]"));
      await s.findWait("[data-testid=sync-server]", 70000);
    }
    const defaultVaultName = await s.exec(`return document.querySelector('[data-testid=sync-vault]')?.value`);
    // Setup asks before it creates a vault the server does not have; wait for
    // that question as well as for an error or a connection.
    await env.fillSetup({ server: env.surl, vaultId: "e2e-" });
    await s.waitFor(
      `return !!document.querySelector('[data-testid=dialog-ok]') || !!document.querySelector('[data-testid=sync-error]') || !!document.querySelector('[data-testid=sync-state]')`,
      { timeout: 90000 },
    );
    const question = await s.exec(`return document.querySelector('[data-testid=dialog-ok]')?.closest('[role=dialog]').querySelector('p')?.textContent.trim() ?? null`);
    // The user says no, to correct the name.
    if (question) {
      await s.exec(`const ok = document.querySelector('[data-testid=dialog-ok]'); [...ok.parentElement.querySelectorAll('button')].find((b) => b !== ok).click()`);
      await s.waitFor(`return !document.querySelector('[data-testid=dialog-ok]')`);
    }
    await sleep(1000);
    const r = {
      connected: await s.exec(`return !!document.querySelector('[data-testid=sync-state]')`),
      error: await s.exec(`return document.querySelector('[data-testid=sync-error]')?.textContent.trim() ?? null`),
      button: await s.exec(`return document.querySelector('[data-testid=sync-connect]')?.textContent.trim() ?? null`),
      vaultField: await s.exec(`return document.querySelector('[data-testid=sync-vault]')?.value ?? null`),
    };
    const settingsText = await s.exec(`return document.querySelector('[data-testid=settings]')?.innerText`);
    await env.closeSettings();
    const indicator = await env.indicator();
    const gotPhoneNote = fs.existsSync(path.join(A, "From phone.md"));
    const vaults = await (await fetch(`${env.surl}/v1/vaults/e2e-`, { headers: { Authorization: `Bearer ${TOKEN}` } })).status;
    const out = { defaultVaultName, question, after: r, indicator, gotPhoneNote, serverStatusForTypoVault: vaults, settingsText: settingsText?.slice(0, 400) };
    results["typo vault"] = out;
    console.log("typo vault", JSON.stringify(out, null, 2));
    // put things back for the next test
    await env.openSyncSettings();
    if (await s.exec(`return !!document.querySelector('[data-testid=sync-state]')`)) {
      await s.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Turn off').click()`);
      await s.click(await s.findWait("[data-testid=dialog-ok]"));
      await s.findWait("[data-testid=sync-server]", 70000);
    }
    assert.ok(question || !r.connected, `connected to a brand-new vault "e2e-" without asking (status bar: ${JSON.stringify(indicator)}, phone's note present: ${gotPhoneNote}; default vault name offered: ${JSON.stringify(defaultVaultName)})`);
    assert.equal(question, "There's no notebook called e2e- on this server. Create it?");
    // Saying no creates nothing and leaves the form as it was.
    assert.deepEqual({ ...r, vaults }, { connected: false, error: null, button: "Connect and sync", vaultField: "e2e-", vaults: 404 });
  },
);

test("Escape on the create-vault question closes only the question: Settings stays open with focus in it, nothing is created", async () => {
  const s = env.s;
  if (!(await s.exec(`return !!document.querySelector('[data-testid=sync-server]')`))) await env.openSyncSettings();
  await env.fillSetup({ server: env.surl, vaultId: "e2e-esc" });
  // The question is a dialog over Settings (modal.ts: one more layer) and takes focus.
  await s.waitFor(`return document.activeElement?.dataset.testid === 'dialog-ok'`, { timeout: 30000, message: "the create-vault question has focus" });
  const question = await s.exec(`return document.querySelector('[data-testid=dialog-ok]').closest('[role=dialog]').querySelector('p')?.textContent.trim() ?? null`);
  // A real key press, from where the app put focus.
  await s.keys(Key.escape);
  await s.waitFor(`return !document.querySelector('[data-testid=dialog-ok]')`, { message: "Escape closed the question" });
  // Escape answers no: the setup goes back to the form. Then focus is handed back.
  await s
    .waitFor(`return document.querySelector('[data-testid=sync-connect]')?.textContent.trim() === 'Connect and sync'`, { timeout: 5000 })
    .catch(() => {});
  await sleep(500);
  const after = {
    settingsOpen: await s.exec(`return !!document.querySelector('[data-testid=settings]')`),
    section: await s.exec(`return document.querySelector('[data-testid=settings] nav button.on')?.textContent.trim() ?? null`),
    focusInSettings: await s.exec(`return !!document.activeElement?.closest('[data-testid=settings]')`),
    focus: await s.exec(
      `const a = document.activeElement; return a ? a.tagName.toLowerCase() + (a.dataset.testid ? '[' + a.dataset.testid + ']' : '') + ' ' + JSON.stringify((a.textContent ?? '').trim().slice(0, 40)) : null`,
    ),
    connected: await s.exec(`return !!document.querySelector('[data-testid=sync-state]')`),
    error: await s.exec(`return document.querySelector('[data-testid=sync-error]')?.textContent.trim() ?? null`),
    button: await s.exec(`return document.querySelector('[data-testid=sync-connect]')?.textContent.trim() ?? null`),
    vaultField: await s.exec(`return document.querySelector('[data-testid=sync-vault]')?.value ?? null`),
  };
  const configured = (await env.invoke("sync_status")).ok?.configured;
  const vaults = (await fetch(`${env.surl}/v1/vaults/e2e-esc`, { headers: { Authorization: `Bearer ${TOKEN}` } })).status;
  const out = { question, after, configured, serverStatusForNewVault: vaults };
  results["escape on the question"] = out;
  console.log("escape on the question", JSON.stringify(out, null, 2));
  assert.equal(question, "There's no notebook called e2e-esc on this server. Create it?");
  assert.equal(after.settingsOpen, true, "Escape closed Settings as well as the question");
  assert.equal(after.section, "Sync");
  assert.ok(after.focusInSettings, `after Escape focus is on ${after.focus}, not in Settings`);
  // Nothing set up here or on the server; the form is as it was.
  assert.deepEqual(
    { connected: after.connected, error: after.error, button: after.button, vaultField: after.vaultField, configured, vaults },
    { connected: false, error: null, button: "Connect and sync", vaultField: "e2e-esc", configured: false, vaults: 404 },
  );
});

test("server goes away and comes back: status bar says so, then recovers (held up)", async () => {
  const s = env.s;
  if (!(await s.exec(`return !!document.querySelector('[data-testid=sync-server]')`))) await env.openSyncSettings();
  const r = await attempt("good setup", { server: env.surl });
  assert.equal(r.connected, true, JSON.stringify(r));
  await s.waitFor(`return document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle'`, { timeout: 30000 });
  await env.closeSettings();
  assert.ok(fs.existsSync(path.join(A, "From phone.md")));
  env.stopServer();
  await env.syncNowFromUi();
  const down = { indicator: await env.indicator(), title: await env.indicatorTitle() };
  await env.startServer();
  await env.syncNowFromUi();
  const up = { indicator: await env.indicator(), title: await env.indicatorTitle() };
  results["server down/up"] = { down, up };
  console.log("server down/up", JSON.stringify({ down, up }));
  assert.equal(down.indicator, "Sync error");
  assert.match(down.title, /cannot reach the server/);
  assert.match(up.indicator, /^Synced/);
});
