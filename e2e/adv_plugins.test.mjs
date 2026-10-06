// Adversarial end-to-end tests for the plugin sandbox, run against the real app.
//
//   scripts/e2e-headless.sh e2e/adv_plugins.test.mjs
//   PL_SLOW=1 scripts/e2e-headless.sh e2e/adv_plugins.test.mjs     (adds the 30 s timer tests)
//
// Every test opens its own throwaway vault (via "Switch vault"), so a failing
// test cannot leave plugins running in the next one.
//
// To read what a plugin worker does, the page's Worker constructor is wrapped
// once at start (window.__plMsgs gets every message a worker posts,
// window.__plWorkers keeps every worker so cleanup can terminate orphans).

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const SLOW = !!process.env.PL_SLOW;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-adv-plugins-"));
const evidence = path.join(import.meta.dirname, ".tmp/PL");

let drv, s, server, PORT;
const hits = [];
let vaultCount = 0;

function makeVault(name, files) {
  const dir = path.join(tmp, `${++vaultCount}-${name}`);
  fs.mkdirSync(dir, { recursive: true });
  const v = {
    dir,
    write(rel, content) {
      const p = path.join(dir, rel);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, content);
    },
    read: (rel) => fs.readFileSync(path.join(dir, rel), "utf8"),
    exists: (rel) => fs.existsSync(path.join(dir, rel)),
  };
  for (const [rel, content] of Object.entries(files)) v.write(rel, content);
  return v;
}

async function eventually(fn, { timeout = 5000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let err;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      err = e;
    }
    await sleep(80);
  }
  throw new Error(`timed out: ${message}${err ? ` (${err.message})` : ""}`);
}

async function installSpy() {
  await s.exec(`
    if (!window.__plSpy) {
      window.__plSpy = true;
      window.__plMsgs = [];
      window.__plWorkers = [];
      const W = window.Worker;
      window.Worker = function (url, opts) {
        const w = new W(url, opts);
        w.__name = opts && opts.name;
        const term = w.terminate.bind(w);
        w.terminate = () => { w.__terminated = true; term(); };
        w.addEventListener("message", (e) => window.__plMsgs.push({ from: w.__name, data: e.data }));
        window.__plWorkers.push(w);
        return w;
      };
      window.Worker.prototype = W.prototype;
    }
  `);
}

/** Terminate every plugin worker the page ever created (orphans included). */
async function killAllPluginWorkers() {
  await s.exec(`(window.__plWorkers || []).forEach((w) => { if (String(w.__name).startsWith("cairn-plugin-")) w.terminate(); })`);
}

const probes = (k) => s.exec(`return window.__plMsgs.filter((m) => m.data && m.data.type === "probe" && (${JSON.stringify(k ?? null)} === null || m.data.k === ${JSON.stringify(k ?? null)})).map((m) => m.data)`);
const waitProbe = (k, timeout = 8000) => eventually(async () => (await probes(k))[0], { timeout, message: `probe ${k}` });
const toastTexts = () => s.exec(`return [...document.querySelectorAll('.toast')].map((t) => t.textContent)`);
const dialogOpen = () => s.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);

async function escapeAll() {
  for (let i = 0; i < 3; i++) {
    await s.keys(Key.escape);
    await sleep(60);
  }
}

async function runCommand(name) {
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), name);
  await sleep(150);
  await s.keys(Key.enter);
}

async function switchTo(v) {
  await escapeAll();
  await killAllPluginWorkers();
  await runCommand("Switch vault");
  const input = await s.findWait("[data-testid=vault-path]", 8000);
  await s.type(input, v.dir);
  await s.click(await s.find("[data-testid=vault-open]"));
  await s.waitFor(`return !document.querySelector('[data-testid=vault-path]') && document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  await s.exec(`window.__plMsgs.length = 0`);
}

async function openPluginSettings() {
  await escapeAll();
  await s.click(await s.find("[data-testid=open-settings]"));
  await s.click(await s.findWait("[data-testid=settings-plugins]"));
}

async function pluginRows() {
  return s.exec(`return [...document.querySelectorAll('[data-testid=plugin-row]')].map((r) => r.dataset.file)`);
}

/** Enable a plugin from Settings > Plugins; accepts the consent dialog if one appears. */
async function enablePlugin(file, { expectDialog }) {
  await openPluginSettings();
  await s.findWait(`[data-testid=plugin-row][data-file="${file}"]`);
  await s.click(await s.find(`[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`));
  if (expectDialog) await s.click(await s.findWait("[data-testid=dialog-ok]"));
  await sleep(300);
}

async function disablePlugin(file) {
  await openPluginSettings();
  await s.click(await s.findWait(`[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`));
  await sleep(300);
}

async function shot(name) {
  try {
    fs.writeFileSync(path.join(evidence, name), await s.screenshot());
  } catch {}
}

before(async () => {
  fs.mkdirSync(evidence, { recursive: true });
  server = http.createServer((req, res) => {
    hits.push(`${req.method} ${req.url}`);
    res.setHeader("access-control-allow-origin", "*");
    res.setHeader("content-type", req.url.endsWith(".js") ? "text/javascript" : "text/plain");
    res.end(req.url.endsWith(".js") ? "self.__imported = 'yes'; postMessage('loaded');" : "hello from the network");
  });
  server.on("upgrade", (req, sock) => {
    hits.push(`UPGRADE ${req.url}`);
    sock.destroy();
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  PORT = server.address().port;
  const home = makeVault("home", { "Home.md": "# Home\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }) });
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [home.dir]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 15000 });
  await installSpy();
});

after(async () => {
  if (s) await shot("final.png");
  await s?.close();
  drv?.proc.kill();
  server?.close();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

// ---------------------------------------------------------------------------
// The worker sandbox itself (held up)
// ---------------------------------------------------------------------------

test("a plugin worker cannot reach the network, vault://, the Tauri bridge or the IPC endpoint", async () => {
  const v = makeVault("sandbox", {
    "Secret.md": "top secret note text\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/escape.js": `// @name Escape
const out = (k, v) => postMessage({ type: "probe", k, v: String(v) });
const P = ${PORT};
const base = "http://127.0.0.1:" + P;
const t = async (k, f) => { try { out(k, await Promise.race([f(), new Promise((_, rej) => setTimeout(() => rej(new Error("timeout")), 3000))])); } catch (e) { out(k, "ERR " + (e && e.message || e)); } };
const sync = (k, f) => { try { out(k, f()); } catch (e) { out(k, "ERR " + (e && e.message || e)); } };
sync("tauri", () => typeof self.__TAURI_INTERNALS__ + "," + typeof self.__TAURI__ + "," + typeof self.webkit + "," + typeof document);
sync("eval", () => eval("'evaluated'"));
sync("importScripts-http", () => { importScripts(base + "/imp.js"); return self.__imported; });
sync("importScripts-data", () => { importScripts("data:text/javascript,self.__d=1"); return self.__d; });
sync("importScripts-blob", () => { importScripts(URL.createObjectURL(new Blob(["self.__b=1"], { type: "text/javascript" }))); return self.__b; });
sync("importScripts-vault", () => { importScripts("vault://localhost/Secret.md"); return "loaded"; });
const tasks = [
  t("fetch", async () => (await fetch(base + "/fetch")).status),
  t("fetch-nocors", async () => (await fetch(base + "/nocors", { mode: "no-cors" })).type),
  t("xhr", () => new Promise((res, rej) => { const x = new XMLHttpRequest(); x.open("GET", base + "/xhr"); x.onload = () => res(x.status); x.onerror = () => rej(new Error("xhr error")); x.send(); })),
  t("websocket", () => new Promise((res, rej) => { const w = new WebSocket("ws://127.0.0.1:" + P + "/ws"); w.onopen = () => res("open"); w.onerror = () => rej(new Error("ws error")); })),
  t("eventsource", () => new Promise((res, rej) => { const w = new EventSource(base + "/es"); w.onopen = () => res("open"); w.onerror = () => rej(new Error("es error")); })),
  t("import-http", async () => { await import(base + "/dyn.js"); return "imported"; }),
  t("import-blob", async () => { await import(URL.createObjectURL(new Blob(["export default 1"], { type: "text/javascript" }))); return "imported"; }),
  t("vault-fetch", async () => await (await fetch("vault://localhost/Secret.md")).text()),
  t("vault-http", async () => await (await fetch("http://vault.localhost/Secret.md")).text()),
  t("vault-xhr", () => new Promise((res, rej) => { const x = new XMLHttpRequest(); x.open("GET", "vault://localhost/Secret.md"); x.onload = () => res(x.responseText); x.onerror = () => rej(new Error("xhr error")); x.send(); })),
  t("ipc", async () => { const r = await fetch("ipc://localhost/read_note", { method: "POST", body: JSON.stringify({ path: "Secret.md" }), headers: { "Content-Type": "application/json" } }); return r.status + " " + (await r.text()); }),
  t("nested-blob", () => new Promise((res, rej) => {
    const code = "(async () => { const r = [typeof __TAURI_INTERNALS__]; try { await fetch('" + base + "/nested'); r.push('net'); } catch { r.push('no-net'); } try { await (await fetch('vault://localhost/Secret.md')).text(); r.push('vault'); } catch { r.push('no-vault'); } postMessage(r.join(',')); })();";
    const w = new Worker(URL.createObjectURL(new Blob([code], { type: "text/javascript" })));
    w.onmessage = (e) => res(e.data); w.onerror = () => rej(new Error("nested worker error"));
  })),
  t("nested-http", () => new Promise((res, rej) => { const w = new Worker(base + "/w.js"); w.onmessage = (e) => res(e.data); w.onerror = () => rej(new Error("nested http worker error")); })),
];
Promise.all(tasks).then(() => out("done", 1));
`,
  });
  await switchTo(v);
  hits.length = 0;
  await enablePlugin("escape.js", { expectDialog: false });
  await waitProbe("done", 15000);
  const r = Object.fromEntries((await probes()).map((p) => [p.k, p.v]));
  fs.writeFileSync(path.join(evidence, "sandbox-probes.json"), JSON.stringify({ probes: r, serverHits: hits }, null, 2));
  assert.deepEqual(hits, [], "no request reached the local HTTP server");
  assert.equal(r.tauri, "undefined,undefined,undefined,undefined");
  assert.match(r.eval, /^ERR/);
  for (const k of ["importScripts-http", "importScripts-data", "importScripts-blob", "importScripts-vault", "fetch", "fetch-nocors", "xhr", "websocket", "eventsource", "import-http", "import-blob", "vault-fetch", "vault-http", "vault-xhr", "nested-http"]) {
    assert.match(r[k], /^ERR/, `${k}: ${r[k]}`);
  }
  assert.equal(r["nested-blob"], "undefined,no-net,no-vault");
  // The IPC endpoint is reachable by fetch (CSP allows ipc:), but without the
  // page's invoke key Tauri refuses the call.
  assert.ok(!r.ipc.includes("top secret"), r.ipc);
  await escapeAll();
});

test("raw host-protocol messages are permission-checked on the host for every method", async () => {
  const v = makeVault("raw", {
    "Secret.md": "secret body\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/raw.js": `// @name Raw
const replies = [];
self.addEventListener("message", (e) => { if (e.data && e.data.type === "result") replies.push(e.data); });
const calls = [
  ["notes.list", []], ["notes.read", ["Secret.md"]], ["notes.write", ["Secret.md", "overwritten"]],
  ["notes.write", ["New.md", "created"]], ["editor.activePath", []], ["editor.getSelection", []],
  ["editor.replaceSelection", ["x"]], ["fs.read", ["Secret.md"]], ["__proto__", []], ["constructor", []],
];
calls.forEach(([method, args], i) => postMessage({ type: "call", id: 1000 + i, method, args }));
// Forged replies addressed to ids the host never issued, and junk types.
postMessage({ type: "result", id: 1, value: "forged" });
postMessage({ type: "command-done", id: "nope", error: "x" });
postMessage(JSON.parse('{"type":"call","id":2000,"method":"notes.read","args":["Secret.md"],"__proto__":{"permissions":["read"]}}'));
setTimeout(() => postMessage({ type: "probe", k: "replies", v: JSON.stringify(replies) }), 1500);
`,
  });
  await switchTo(v);
  await enablePlugin("raw.js", { expectDialog: false });
  const p = await waitProbe("replies");
  const replies = JSON.parse(p.v);
  const byId = Object.fromEntries(replies.map((r) => [r.id, r]));
  for (let i = 0; i < 10; i++) assert.ok(byId[1000 + i]?.error, `call ${i} refused: ${JSON.stringify(byId[1000 + i])}`);
  assert.ok(byId[2000]?.error, "a __proto__ payload does not grant permissions");
  assert.ok(!p.v.includes("secret body"), p.v);
  assert.equal(v.read("Secret.md"), "secret body\n");
  assert.equal(v.exists("New.md"), false);
  assert.equal(await s.exec(`return ({}).permissions === undefined`), true);
  await escapeAll();
});

test("HTML in plugin names, command names and toasts is shown as text, not markup", async () => {
  const v = makeVault("html", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/html.js": `// @name <img src=x onerror="window.__plXss=1">Evil
// @description <b id="pl-desc">desc</b>
cairn.commands.register("go", '<b id="pl-pwn">Bold</b> go', async () => {
  await cairn.ui.toast('<img id="pl-pwn2" src=x onerror="window.__plXss=2">toast');
});
`,
  });
  await switchTo(v);
  await openPluginSettings();
  await s.findWait('[data-testid=plugin-row][data-file="html.js"]');
  const rowText = await s.exec(`return document.querySelector('[data-testid=plugin-row][data-file="html.js"]').textContent`);
  assert.ok(rowText.includes("<img src=x"), rowText);
  await s.click(await s.find('[data-testid=plugin-row][data-file="html.js"] [data-testid=plugin-toggle]'));
  await sleep(500);
  await escapeAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Bold");
  const item = await s
    .waitFor(`return [...document.querySelectorAll('[data-testid=palette-item]')].map((e) => e.textContent).find((t) => t.includes('Bold'))`)
    .catch(async (e) => {
      const all = await s.exec(`return [...document.querySelectorAll('[data-testid=palette-item]')].map((e) => e.textContent)`);
      const live = await s.exec(`return window.__plWorkers.filter((w) => w.__name === "cairn-plugin-html.js").map((w) => !!w.__terminated)`);
      const msgs = await s.exec(`return window.__plMsgs.filter((m) => m.from === "cairn-plugin-html.js").map((m) => m.data)`);
      throw new Error(`${e.message}; palette: ${JSON.stringify(all)}; workers: ${JSON.stringify(live)}; msgs: ${JSON.stringify(msgs)}`);
    });
  assert.ok(item.includes('<b id="pl-pwn">'), item);
  await s.keys(Key.enter);
  await s.waitFor(`return [...document.querySelectorAll('.toast')].some((t) => t.textContent.includes('toast'))`, { timeout: 8000 });
  const dom = await s.exec(`return { xss: window.__plXss ?? null, pwn: !!document.getElementById('pl-pwn'), pwn2: !!document.getElementById('pl-pwn2'), desc: !!document.getElementById('pl-desc') }`);
  assert.deepEqual(dom, { xss: null, pwn: false, pwn2: false, desc: false });
});

test("a plugin command id equal to a built-in id does not replace the built-in or take its hotkey", async () => {
  const v = makeVault("collide", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/collide.js": `// @name Collide
for (const id of ["app:settings", "app:command-palette", "../app:settings", "__proto__"]) {
  cairn.commands.register(id, "Open settings", async () => { postMessage({ type: "probe", k: "hijacked", v: id }); });
}
`,
  });
  await switchTo(v);
  await enablePlugin("collide.js", { expectDialog: false });
  await escapeAll();
  await s.keys({ chord: [Key.ctrl, ","] });
  await s.findWait("[data-testid=settings-plugins]");
  await escapeAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Open settings");
  const names = await s.waitFor(`const n = [...document.querySelectorAll('[data-testid=palette-item]')].map((e) => e.textContent.trim()); return n.length ? n : null`);
  assert.ok(names.some((n) => n.startsWith("Open settings")), JSON.stringify(names));
  assert.ok(names.filter((n) => n.startsWith("Collide: Open settings")).length >= 1, JSON.stringify(names));
  await escapeAll();
  assert.deepEqual(await probes("hijacked"), [], "the built-in hotkey ran the built-in, not the plugin");
});

test("disabling a plugin terminates its worker and removes its commands; enabling it again works", async () => {
  const v = makeVault("toggle", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/pinger.js": `// @name Pinger
cairn.commands.register("ping", "Ping", async () => { postMessage({ type: "probe", k: "ping", v: Date.now() }); });
`,
  });
  await switchTo(v);
  await enablePlugin("pinger.js", { expectDialog: false });
  await escapeAll();
  await runCommand("Pinger: Ping");
  await waitProbe("ping");
  await disablePlugin("pinger.js");
  const live = await s.exec(`return window.__plWorkers.filter((w) => w.__name === "cairn-plugin-pinger.js" && !w.__terminated).length`);
  assert.equal(live, 0, "worker terminated");
  await escapeAll();
  await s.keys({ chord: [Key.ctrl, "p"] });
  await s.type(await s.findWait("[data-testid=palette-input]"), "Pinger");
  await sleep(200);
  const items = await s.exec(`return [...document.querySelectorAll('[data-testid=palette-item]')].map((e) => e.textContent).filter((t) => t.includes('Pinger'))`);
  assert.deepEqual(items, []);
  await escapeAll();
  await s.exec(`window.__plMsgs.length = 0`);
  await enablePlugin("pinger.js", { expectDialog: false });
  await escapeAll();
  await runCommand("Pinger: Ping");
  await waitProbe("ping");
});

test("notes.write cannot leave the vault or create non-Markdown or config files", async () => {
  const v = makeVault("write-scope", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/writer.js": `// @name Writer
// @permissions write
const targets = ["x.js", "x.html", "x.svg", "../escape.md", "../../escape2.md", "/tmp/cairn-pl-abs.md", "sub\\\\back.md",
  ".cairn/plugins/evil.js", ".cairn/settings.json", ".cairn/snippets/x.css", ".cairn/new.md", ".git/new.md", "x.md.js"];
(async () => {
  const r = {};
  for (const p of targets) { try { await cairn.notes.write(p, "planted by plugin"); r[p] = "ok"; } catch (e) { r[p] = "ERR " + e.message; } }
  postMessage({ type: "probe", k: "write", v: JSON.stringify(r) });
})();
`,
  });
  const settingsBefore = v.read(".cairn/settings.json");
  await switchTo(v);
  await enablePlugin("writer.js", { expectDialog: true });
  const r = JSON.parse((await waitProbe("write")).v);
  fs.writeFileSync(path.join(evidence, "write-scope.json"), JSON.stringify(r, null, 2));
  for (const p of ["x.js", "x.html", "x.svg", "../escape.md", "../../escape2.md", ".cairn/plugins/evil.js", ".cairn/settings.json", ".cairn/snippets/x.css", ".cairn/new.md", ".git/new.md", "x.md.js"]) {
    assert.match(r[p], /^ERR/, `${p}: ${r[p]}`);
  }
  assert.equal(fs.existsSync(path.join(path.dirname(v.dir), "escape.md")), false);
  assert.equal(fs.existsSync(path.join(tmp, "..", "escape2.md")), false);
  assert.equal(fs.existsSync("/tmp/cairn-pl-abs.md"), false, "absolute paths are taken as vault-relative");
  assert.equal(v.exists("tmp/cairn-pl-abs.md"), r["/tmp/cairn-pl-abs.md"] === "ok");
  assert.equal(v.exists(".cairn/plugins/evil.js"), false);
  assert.equal(v.exists(".cairn/snippets/x.css"), false);
  assert.equal(v.read(".cairn/settings.json").includes("planted"), false);
  assert.ok(!settingsBefore.includes("planted"));
});

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

test("opening a vault whose settings.json enables a plugin asks before running it", async () => {
  // A vault received from someone else (git clone, shared folder, archive).
  const v = makeVault("hostile", {
    "Diary.md": "my private diary\n",
    ".cairn/settings.json": JSON.stringify({ plugins: ["helper.js"] }),
    ".cairn/plugins/helper.js": `// @name Helper
// @permissions read write
(async () => {
  const text = await cairn.notes.read("Diary.md");
  await cairn.notes.write("Diary.md", "changed by an auto-started plugin\\n");
  await cairn.notes.write("Copied.md", text);
  postMessage({ type: "probe", k: "ran", v: 1 });
})();
`,
  });
  await switchTo(v);
  // Not approved on this device: it stays off, and a toast (no dialog) says so.
  const toast = await eventually(async () => (await toastTexts()).find((t) => t.includes("Settings > Plugins")), { timeout: 3000, message: "toast about the plugin that is off" }).catch(() => null);
  await sleep(2500);
  const sawDialog = await dialogOpen();
  await shot("PL-01-hostile-vault.png");
  const diary = v.read("Diary.md");
  const ran = (await probes("ran")).length > 0;
  assert.ok(sawDialog || !ran, `plugin ran without a prompt (dialog shown: ${sawDialog}); Diary.md is now ${JSON.stringify(diary)}`);
  assert.equal(diary, "my private diary\n");
  assert.equal(toast, "1 plugin in this vault is off until you turn it on in Settings > Plugins.");
  // Settings > Plugins shows it as off and not approved; turning it on asks first, then it runs.
  await openPluginSettings();
  const row = `[data-testid=plugin-row][data-file="helper.js"]`;
  await s.findWait(row);
  assert.equal(await s.exec(`return document.querySelector('${row} [data-testid=plugin-toggle]').checked`), false);
  assert.ok(await s.exec(`return !!document.querySelector('${row} [data-testid=plugin-not-approved]')`), "row says it is not approved here");
  await enablePlugin("helper.js", { expectDialog: true });
  await waitProbe("ran");
  await escapeAll();
  await killAllPluginWorkers();
});

test("every plugin that runs is listed in Settings > Plugins (and can be switched off)", async () => {
  const v = makeVault("stealth", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: ["lib/stealth.js", "stealth.txt"] }),
    ".cairn/plugins/lib/stealth.js": `// @name Stealth
// @permissions write
cairn.notes.write("stealth-ran.md", "a plugin no one can see\\n");
`,
    ".cairn/plugins/stealth.txt": `// @name Stealth two
// @permissions write
cairn.notes.write("stealth2-ran.md", "also invisible\\n");
`,
  });
  await switchTo(v);
  await eventually(() => v.exists("stealth-ran.md") || v.exists("stealth2-ran.md"), { timeout: 5000, message: "stealth plugins ran" }).catch(() => {});
  const ran = ["lib/stealth.js", "stealth.txt"].filter((f, i) => v.exists(i === 0 ? "stealth-ran.md" : "stealth2-ran.md"));
  await openPluginSettings();
  await sleep(500);
  const rows = await pluginRows();
  await shot("PL-03-plugins-list.png");
  await escapeAll();
  await killAllPluginWorkers();
  for (const f of ran) assert.ok(rows.includes(f), `${f} ran but the Plugins list shows only ${JSON.stringify(rows)}`);
  assert.deepEqual(ran, [], "only files the Plugins list can show may run");
});

test("a plugin whose file changes to ask for more permissions is not granted them without asking", async () => {
  const v = makeVault("toctou", {
    "Secret.md": "bank pin 0000\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/wordcount.js": `// @name Word count
// @permissions editor
cairn.commands.register("count", "Count", async () => { await cairn.ui.toast((await cairn.editor.getSelection()).split(/\\s+/).length + " words"); });
`,
  });
  await switchTo(v);
  await enablePlugin("wordcount.js", { expectDialog: true }); // the user agreed to "editor" only
  await eventually(() => JSON.parse(v.read(".cairn/settings.json")).plugins?.includes("wordcount.js"), { message: "enabled" });
  // Later the file is replaced (an update, a git pull, a file-sync tool, another app).
  v.write(".cairn/plugins/wordcount.js", `// @name Word count
// @permissions editor read write
(async () => {
  const text = await cairn.notes.read("Secret.md");
  await cairn.notes.write("Leak.md", text);
})();
`);
  await openPluginSettings();
  await s.click(await s.findWait("[data-testid=plugins-reload]"));
  await sleep(2000);
  const sawDialog = await dialogOpen();
  const leaked = v.exists("Leak.md") ? v.read("Leak.md") : null;
  assert.ok(sawDialog || leaked === null, `no new consent prompt (dialog: ${sawDialog}), and the plugin read Secret.md into Leak.md: ${JSON.stringify(leaked)}`);
  // The changed file is shown as off and not approved; turning it on asks again, with the new permissions.
  const row = `[data-testid=plugin-row][data-file="wordcount.js"]`;
  assert.equal(await s.exec(`return document.querySelector('${row} [data-testid=plugin-toggle]').checked`), false);
  assert.ok(await s.exec(`return !!document.querySelector('${row} [data-testid=plugin-not-approved]')`), "row says it is not approved here");
  await s.click(await s.find(`${row} [data-testid=plugin-toggle]`));
  await s.findWait("[data-testid=dialog-ok]");
  const asked = await s.exec(`return document.querySelector('[data-testid=dialog-ok]').parentElement.parentElement.textContent`);
  await escapeAll();
  await killAllPluginWorkers();
  assert.match(asked, /read all notes, create and change notes/);
  assert.equal(v.exists("Leak.md"), false);
});

test("the read permission only reads notes, not config, other plugins, git or trash files", async () => {
  const v = makeVault("read-scope", {
    "Note.md": "a note\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/reader.js": `// @name Reader
// @permissions read
(async () => {
  const r = {};
  for (const p of [".cairn/settings.json", ".cairn/plugins/other.js", ".git/config", ".trash/deleted.md", "Note.md"]) {
    try { r[p] = (await cairn.notes.read(p)).slice(0, 80); } catch (e) { r[p] = "ERR " + e.message; }
  }
  postMessage({ type: "probe", k: "read", v: JSON.stringify(r) });
})();
`,
    ".cairn/plugins/other.js": "// @name Other\n// other plugin's source: SOURCE-MARKER\n",
    ".git/config": '[remote "origin"]\n\turl = https://user:FAKE-TOKEN-123@example.invalid/notes.git\n',
    ".trash/deleted.md": "a note the user deleted\n",
  });
  await switchTo(v);
  await enablePlugin("reader.js", { expectDialog: true });
  const r = JSON.parse((await waitProbe("read")).v);
  fs.writeFileSync(path.join(evidence, "read-scope.json"), JSON.stringify(r, null, 2));
  await escapeAll();
  await killAllPluginWorkers();
  assert.equal(r["Note.md"], "a note\n");
  for (const p of [".cairn/settings.json", ".cairn/plugins/other.js", ".git/config", ".trash/deleted.md"]) assert.match(r[p], /^ERR/, `${p} was readable: ${r[p]}`);
});

test("notes.write cannot overwrite existing Markdown files inside hidden folders", async () => {
  const v = makeVault("hidden-write", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/notes.md": "config-folder note\n",
    ".trash/deleted.md": "a note in the trash\n",
    ".cairn/plugins/hw.js": `// @name HW
// @permissions write
(async () => {
  const r = {};
  for (const p of [".cairn/notes.md", ".trash/deleted.md"]) { try { await cairn.notes.write(p, "overwritten by plugin\\n"); r[p] = "ok"; } catch (e) { r[p] = "ERR " + e.message; } }
  postMessage({ type: "probe", k: "hw", v: JSON.stringify(r) });
})();
`,
  });
  await switchTo(v);
  await enablePlugin("hw.js", { expectDialog: true });
  const r = JSON.parse((await waitProbe("hw")).v);
  await escapeAll();
  await killAllPluginWorkers();
  assert.equal(v.read(".cairn/notes.md"), "config-folder note\n", JSON.stringify(r));
  assert.equal(v.read(".trash/deleted.md"), "a note in the trash\n", JSON.stringify(r));
});

test("a disabled plugin stops running even after Reload is clicked twice quickly", async () => {
  const ticker = `// @name Ticker
// @permissions write
const id = Math.random().toString(36).slice(2, 8);
let n = 0;
setInterval(() => { cairn.notes.write("ticks/" + id + ".md", String(++n)); }, 250);
`;
  const v = makeVault("orphan", { "Note.md": "x\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }), ".cairn/plugins/ticker.js": ticker });
  const next = makeVault("orphan-next", { "Other.md": "another vault\n", ".cairn/settings.json": JSON.stringify({ plugins: [] }) });
  const snapshot = (vault) => {
    const d = path.join(vault.dir, "ticks");
    if (!fs.existsSync(d)) return {};
    return Object.fromEntries(fs.readdirSync(d).map((f) => [f, fs.readFileSync(path.join(d, f), "utf8")]));
  };
  try {
    await switchTo(v);
    await enablePlugin("ticker.js", { expectDialog: true });
    await s.findWait("[data-testid=plugins-reload]");
    await s.exec(`const b = document.querySelector('[data-testid=plugins-reload]'); b.click(); b.click();`);
    await sleep(1500);
    await s.click(await s.find('[data-testid=plugin-row][data-file="ticker.js"] [data-testid=plugin-toggle]')); // disable
    await sleep(800);
    const a = snapshot(v);
    await sleep(1500);
    const b = snapshot(v);
    const stillTicking = Object.keys(b).filter((f) => a[f] !== b[f]);
    // Switch to a different vault: nothing from the old vault may write into it.
    await escapeAll();
    await runCommand("Switch vault");
    await s.type(await s.findWait("[data-testid=vault-path]", 8000), next.dir);
    await s.click(await s.find("[data-testid=vault-open]"));
    await sleep(2000);
    const crossVault = Object.keys(snapshot(next));
    fs.writeFileSync(path.join(evidence, "orphan.json"), JSON.stringify({ afterDisableA: a, afterDisableB: b, stillTicking, crossVault }, null, 2));
    assert.deepEqual(stillTicking, [], "plugin workers still writing after disable");
    assert.deepEqual(crossVault, [], "a plugin from the previous vault wrote into the newly opened vault");
  } finally {
    await killAllPluginWorkers();
  }
});

// ---------------------------------------------------------------------------
// 30 s command timeout (slow; PL_SLOW=1)
// ---------------------------------------------------------------------------

test("a command that never finishes stops its plugin after about 30 s and removes its commands", { skip: SLOW ? false : "slow (35 s): set PL_SLOW=1", timeout: 70000 }, async () => {
  const v = makeVault("hang", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/hang.js": `// @name Hang
cairn.commands.register("hang", "Hang", () => new Promise(() => {}));
`,
    ".cairn/plugins/stuck.js": `// @name Stuck
cairn.commands.register("stuck", "Stuck", () => new Promise(() => {}));
`,
  });
  const alive = (file) => s.exec(`return window.__plWorkers.filter((w) => w.__name === "cairn-plugin-${file}" && !w.__terminated).length`);
  const toggle = (file) => `[data-testid=plugin-row][data-file="${file}"] [data-testid=plugin-toggle]`;
  await switchTo(v);
  await enablePlugin("hang.js", { expectDialog: false });
  await enablePlugin("stuck.js", { expectDialog: false });
  await escapeAll();
  await runCommand("Hang: Hang");
  const t0 = Date.now();
  await runCommand("Stuck: Stuck");
  await s.waitFor(`return [...document.querySelectorAll('.toast')].some((t) => t.textContent.includes('Hang took too long'))`, { timeout: 40000 });
  const elapsed = Date.now() - t0;
  assert.ok(elapsed > 25000, `stopped after ${elapsed} ms`);
  assert.equal(await alive("hang.js"), 0);
  await eventually(async () => (await alive("stuck.js")) === 0, { timeout: 10000, message: "stuck.js stopped" });
  // FINDING-153: both are shown as off, and settings.json no longer enables them.
  await openPluginSettings();
  await s.findWait(toggle("hang.js"));
  assert.equal(await s.exec(`return document.querySelector('${toggle("hang.js")}').checked`), false);
  assert.equal(await s.exec(`return document.querySelector('${toggle("stuck.js")}').checked`), false);
  await eventually(() => JSON.parse(v.read(".cairn/settings.json")).plugins.length === 0, { message: "settings.json no longer enables hang.js and stuck.js" });
  // Turned on again by the user right away (no Reload in between), hang.js starts.
  await s.click(await s.find(toggle("hang.js")));
  await eventually(async () => (await alive("hang.js")) === 1, { message: "hang.js started again" });
  // Reload starts what the settings enable: hang.js again, but not stuck.js.
  await s.click(await s.find("[data-testid=plugins-reload]"));
  await sleep(500);
  assert.equal(await alive("hang.js"), 1);
  assert.equal(await alive("stuck.js"), 0);
});

test("running a quick command twice in a row does not get the plugin stopped", { skip: SLOW ? false : "slow (40 s): set PL_SLOW=1", timeout: 70000 }, async () => {
  const v = makeVault("twice", {
    "Note.md": "x\n",
    ".cairn/settings.json": JSON.stringify({ plugins: [] }),
    ".cairn/plugins/twice.js": `// @name Twice
cairn.commands.register("work", "Work", async () => { await new Promise((r) => setTimeout(r, 1500)); postMessage({ type: "probe", k: "done", v: Date.now() }); });
`,
  });
  await switchTo(v);
  await enablePlugin("twice.js", { expectDialog: false });
  await escapeAll();
  await runCommand("Twice: Work");
  await sleep(300);
  await runCommand("Twice: Work");
  await eventually(async () => (await probes("done")).length >= 2, { timeout: 8000, message: "both runs finished" });
  await sleep(32000);
  const toasts = await toastTexts();
  const alive = await s.exec(`return window.__plWorkers.filter((w) => w.__name === "cairn-plugin-twice.js" && !w.__terminated).length`);
  await shot("PL-06-stopped.png");
  assert.equal(alive, 1, `plugin was stopped; toasts: ${JSON.stringify(toasts)}`);
});
