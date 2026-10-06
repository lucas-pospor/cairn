// Reproduction for FINDING-020 (write through a symlinked folder lands
// outside the vault). The other reproduction (adv_security.test.mjs) calls
// write_note straight over IPC, which needs script execution in the page.
// This file checks whether a realistic attacker without that can reach the
// same write:
//
//   A vault received from someone else (git clone, archive, Syncthing share)
//   carries a RELATIVE folder symlink `shared -> ../victim` plus a plugin that
//   .cairn/settings.json enables. Such a plugin only runs once the user
//   turned it on on this device (FINDING-005), so the test approves
//   it first (the user trusted the helper); opening the vault does the rest.
//
// The vault also links two "notes" to its own files: notes/setup.md ->
// ../.git/config and notes/data.md -> ../data.json. A save follows a
// symlinked note, so a plugin writing them would rewrite git's config
// (core.fsmonitor runs a command on the next `git status`) or a data file.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_as_04.test.mjs
//
// The chain tests check that the plugin is refused (FINDING-020).
// The control tests check the other limits on what a plugin can write.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";
import { approvePlugins } from "./plugin_approvals.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-as04-"));
const victim = path.join(tmp, "victim"); // the user's real notes, NOT the opened vault
const vault = path.join(tmp, "shared-vault"); // the vault the user opens
const ORIGINAL = "# Important\nmy real notes, outside the opened vault\n";
const GIT_CONFIG = "[core]\n\trepositoryformatversion = 0\n";
const DATA = '{"budget": 1200}\n';

const PLUGIN = `// @name Helper
// @description Harmless-looking helper shipped with a shared vault.
// @permissions read write
const out = [];
const paths = await cairn.notes.list();
out.push("listed:" + paths.filter((p) => p.startsWith("shared/")).join(","));
for (const p of paths) {
  if (p.startsWith("shared/") && p.endsWith(".md")) {
    try { await cairn.notes.write(p, "CLOBBERED-BY-VAULT-PLUGIN\\n"); out.push("overwrote:" + p); }
    catch (e) { out.push("overwrite-failed:" + p + ":" + e.message); }
  }
}
try { await cairn.notes.write("shared/Planted.md", "planted by a vault plugin\\n"); out.push("planted"); }
catch (e) { out.push("plant-failed:" + e.message); }
for (const p of ["notes/setup.md", "notes/data.md"]) {
  try { await cairn.notes.read(p); out.push("read:" + p); }
  catch (e) { out.push("read-failed:" + p + ":" + e.message); }
  try { await cairn.notes.write(p, "[core]\\n\\tfsmonitor = touch PWNED\\n"); out.push("overwrote:" + p); }
  catch (e) { out.push("overwrite-failed:" + p + ":" + e.message); }
}
try { await cairn.notes.write("../victim/DotDot.md", "x"); out.push("dotdot-ok"); }
catch (e) { out.push("dotdot-rejected"); }
try { await cairn.notes.write("shared/evil.desktop", "x"); out.push("desktop-ok"); }
catch (e) { out.push("desktop-rejected"); }
await cairn.notes.write("probe-result.md", JSON.stringify(out));
`;

let drv, s;

function put(root, rel, content) {
  const p = path.join(root, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}

async function eventually(fn, timeout = 8000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      if (await fn()) return true;
    } catch {}
    await sleep(100);
  }
  return false;
}

before(async () => {
  put(victim, "Important.md", ORIGINAL);
  put(vault, "Readme.md", "# Shared vault\n");
  put(vault, ".cairn/settings.json", JSON.stringify({ plugins: ["helper.js"] }));
  put(vault, ".cairn/plugins/helper.js", PLUGIN);
  approvePlugins(path.join(tmp, "config"), vault, ["helper.js"]);
  // Relative target, as git or tar would recreate it from the shared copy.
  fs.symlinkSync("../victim", path.join(vault, "shared"));
  put(vault, ".git/config", GIT_CONFIG);
  put(vault, "data.json", DATA);
  fs.mkdirSync(path.join(vault, "notes"));
  fs.symlinkSync("../.git/config", path.join(vault, "notes/setup.md"));
  fs.symlinkSync("../data.json", path.join(vault, "notes/data.md"));

  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 20000 });
  await eventually(() => fs.existsSync(path.join(vault, "probe-result.md")), 10000);
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

const probe = () => {
  try {
    return JSON.parse(fs.readFileSync(path.join(vault, "probe-result.md"), "utf8"));
  } catch {
    return null;
  }
};

test(
  "FINDING-020 chain: opening a shared vault does not let its plugin overwrite notes outside the vault through a planted symlink",
  () => {
    const p = probe();
    console.log(`plugin probe: ${JSON.stringify(p)}`);
    assert.ok(p, "plugin ran and wrote its probe");
    const now = fs.readFileSync(path.join(victim, "Important.md"), "utf8");
    const planted = fs.existsSync(path.join(victim, "Planted.md"));
    console.log(`victim/Important.md now: ${JSON.stringify(now)}; victim/Planted.md exists: ${planted}`);
    assert.equal(now, ORIGINAL, "a note outside the opened vault was overwritten");
    assert.ok(!planted, "a new note was created outside the opened vault");
  },
);

test("FINDING-020 chain: its plugin cannot read or rewrite .git/config or a data file through a note linked to them", () => {
  const p = probe();
  assert.ok(p, "plugin ran and wrote its probe");
  for (const n of ["notes/setup.md", "notes/data.md"]) {
    assert.ok(p.some((x) => x.startsWith(`read-failed:${n}:`) && x.includes("leads outside the notebook")), JSON.stringify(p));
    assert.ok(p.some((x) => x.startsWith(`overwrite-failed:${n}:`) && x.includes("leads outside the notebook")), JSON.stringify(p));
  }
  assert.equal(fs.readFileSync(path.join(vault, ".git/config"), "utf8"), GIT_CONFIG);
  assert.equal(fs.readFileSync(path.join(vault, "data.json"), "utf8"), DATA);
  assert.ok(fs.lstatSync(path.join(vault, "notes/setup.md")).isSymbolicLink());
});

test("control: without a symlink the plugin cannot leave the vault ('..' is rejected)", () => {
  const p = probe();
  assert.ok(p, "plugin ran and wrote its probe");
  assert.ok(p.includes("dotdot-rejected"), JSON.stringify(p));
  assert.ok(!fs.existsSync(path.join(victim, "DotDot.md")));
});

test("control: the escape is limited to Markdown files (plugins cannot write other types)", () => {
  const p = probe();
  assert.ok(p, "plugin ran and wrote its probe");
  assert.ok(p.includes("desktop-rejected"), JSON.stringify(p));
  assert.ok(!fs.existsSync(path.join(victim, "evil.desktop")));
});
