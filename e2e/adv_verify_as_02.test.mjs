// Reproduction for FINDING-020 ("a symlink in the vault reads a
// file outside the vault"), against the real app.
//
//   scripts/e2e-headless.sh e2e/adv_verify_as_02.test.mjs
//
// Layout (a temp dir stands in for the user's home):
//   home/.ssh/id_ed25519          a private file outside every vault
//   home/Notes/received/          a vault from someone else (git clone/archive):
//       assets -> ../../.ssh      planted relative symlink (works wherever cloned
//                                 two levels under home)
//       .cairn/plugins/stats.js   a plugin that declares only `read`
//       .cairn/settings.json      plugins: []  (NOT auto-enabled, so this does
//                                 not depend on FINDING-005)
//
// Test 1 passes and is evidence against the finding's main vector ("any script
// in the web view can read files"): the IPC needs no symlink at all, because
// open_vault accepts any folder and read_text_file then reads any file in it.
// Test 2 passes and shows that simply opening the received vault (no plugin)
// only shows the linked file to the local user; nothing is sent anywhere.
// Test 3 is the case where following symlinks breaks a documented boundary:
// README says a plugin has "no access to ... the file system" and `read`
// means "read notes", yet a plugin the user enabled with only `read` could
// read a file outside the vault through the planted symlink. The app refuses
// that read, and the test passes.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-as02-"));
const home = path.join(tmp, "home");
const sshDir = path.join(home, ".ssh");
const received = path.join(home, "Notes", "received");
const KEY = "-----BEGIN OPENSSH PRIVATE KEY-----\nFAKE-KEY-9d2c41\n-----END OPENSSH PRIVATE KEY-----\n";

const PLUGIN = `// @name Word stats
// @description Counts words in your notes.
// @permissions read
(async () => {
  let got;
  try { got = await cairn.notes.read("assets/id_ed25519"); } catch (e) { got = "ERR " + (e && e.message); }
  await cairn.ui.toast("PROBE " + String(got).replace(/\\n/g, " ").slice(0, 120));
})();
`;

function put(p, c) {
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, c);
}

let drv, s;

async function invoke(cmd, args) {
  const r = await s.execAsync(
    `const cb = arguments[arguments.length-1];
     window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {})
       .then(v => cb(JSON.stringify({ ok: v === undefined ? null : v })))
       .catch(e => cb(JSON.stringify({ err: String((e && e.message) || e) })));`,
    cmd,
    args ?? {},
  );
  return JSON.parse(r);
}

async function eventually(fn, { timeout = 8000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch {}
    await sleep(100);
  }
  throw new Error(`timed out: ${message}`);
}

const toastTexts = () => s.exec(`return [...document.querySelectorAll('.toast')].map((t) => t.textContent)`);

before(async () => {
  put(path.join(sshDir, "id_ed25519"), KEY);
  put(path.join(received, "Readme.md"), "# Starter vault\n\nWelcome.\n");
  put(path.join(received, ".cairn/settings.json"), JSON.stringify({ plugins: [] }));
  put(path.join(received, ".cairn/plugins/stats.js"), PLUGIN);
  fs.symlinkSync("../../.ssh", path.join(received, "assets"));

  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [received]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 30000 });
});

after(async () => {
  await s?.close().catch(() => {});
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("FINDING-020: opening the received vault shows the linked file only to the local user", async () => {
  const entries = (await invoke("list_entries")).ok.map((e) => e.path);
  assert.ok(entries.includes("assets/id_ed25519"), `the planted link is listed: ${entries.join(", ")}`);
  const r = await invoke("read_text_file", { path: "assets/id_ed25519" });
  assert.equal(r.ok, KEY, "the app's own read follows the link (the finding's observation)");
});

test("FINDING-020: a web-view script needs no symlink, open_vault + read_text_file reads the same file", async () => {
  const before = (await invoke("startup_vault")).ok;
  const o = await invoke("open_vault", { path: sshDir });
  assert.ok(o.ok, JSON.stringify(o));
  const r = await invoke("read_text_file", { path: "id_ed25519" });
  assert.equal(r.ok, KEY, "IPC reads any file by opening its folder as a vault");
  const back = await invoke("open_vault", { path: received });
  assert.ok(back.ok, JSON.stringify(back));
  void before;
});

test(
  "FINDING-020: a plugin enabled with only `read` cannot read a file outside the vault through a planted symlink",
  async () => {
    // Reload the window so the UI is back on the received vault.
    await s.close();
    s = await Session.create(drv.port, APP, [received]);
    await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 30000 });
    for (let i = 0; i < 3; i++) await s.keys(Key.escape);
    await s.click(await s.find("[data-testid=open-settings]"));
    await s.click(await s.findWait("[data-testid=settings-plugins]"));
    await s.findWait(`[data-testid=plugin-row][data-file="stats.js"]`);
    await s.click(await s.find(`[data-testid=plugin-row][data-file="stats.js"] [data-testid=plugin-toggle]`));
    const ok = await s.findWait("[data-testid=dialog-ok]");
    const consent = await s.exec(`return document.querySelector('[data-testid=dialog-ok]').parentElement.parentElement.textContent`);
    await s.click(ok);
    const probe = await eventually(async () => (await toastTexts()).find((t) => t.includes("PROBE")), { message: "plugin toast" });
    console.log("consent dialog text:", JSON.stringify(consent));
    console.log("plugin toast:", JSON.stringify(probe));
    assert.ok(!probe.includes("FAKE-KEY-9d2c41"), `a read-only plugin read ~/.ssh/id_ed25519 through the planted link: ${probe}`);
  },
);
