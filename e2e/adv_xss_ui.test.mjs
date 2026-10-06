// Adversarial injection / XSS tests for the UI surfaces other than the file
// tree and the Markdown reading view.
//
// A hostile vault is created OUTSIDE Cairn: file and folder names, headings,
// frontmatter keys/values, tags, backlink context lines, CSS snippet names and
// a plugin command name all carry HTML. Each test drives one surface of the
// real app and then probes the whole document:
//   - window.__pwned (set by the payloads' onerror handlers, if they ran);
//   - any element with an onerror attribute anywhere (DOMPurify strips those,
//     so one in the DOM means raw markup was injected);
//   - any .xssm marker element outside the sanitized .md-render containers;
//   - that the surface shows the payload as literal text.
//
// Run:  scripts/e2e-headless.sh e2e/adv_xss_ui.test.mjs
//   one: scripts/e2e-headless.sh --test-name-pattern 'search' e2e/adv_xss_ui.test.mjs
//
// Behaviour that is by design but kept documented asserts the SAFE expectation
// and is marked { todo: "not a defect (by design): ..." }; each such test
// launches its own app so a failure cannot break later tests.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import http from "node:http";
import path from "node:path";
import { launch, freshEnv, eventually, sleep, Key } from "./adv_editor_lib.mjs";
import { approvePlugins } from "./plugin_approvals.mjs";

const EVID = path.join(import.meta.dirname, ".tmp", "XS");
fs.mkdirSync(EVID, { recursive: true });

const P = (n) => `<img src=x class=xssm onerror=window.__pwned=${n}>`;
const TARGET = `Target ${P(2)}.md`;
const QUOTE = `Quote "><b class=xssm>q.md`;
const FOLDER = `Folder ${P(1)}`;
const SNIPPET = `snip ${P(13)}.css`;

function evidence(name, data) {
  fs.writeFileSync(path.join(EVID, name), typeof data === "string" ? data : JSON.stringify(data, null, 2));
}

// TCP sink recording connection attempts (CSS / image beacons, https or http).
function beaconServer() {
  const hits = [];
  const srv = net.createServer((sock) => {
    hits.push(Date.now());
    sock.on("error", () => {});
    setTimeout(() => sock.destroy(), 200);
  });
  return new Promise((r) => srv.listen(0, "127.0.0.1", () => r({ srv, port: srv.address().port, hits })));
}

const PROBE = `
  const outside = [...document.querySelectorAll('.xssm')].filter((e) => !e.closest('.md-render'));
  return {
    pwned: window.__pwned ?? null,
    onerror: [...document.querySelectorAll('[onerror]')].map((e) => e.outerHTML.slice(0, 160)),
    marks: outside.map((e) => (e.parentElement ? e.parentElement.outerHTML : e.outerHTML).slice(0, 200)),
    title: document.title,
  };`;

async function probe(app) {
  return app.exec(PROBE);
}
function assertClean(r, where) {
  assert.equal(r.pwned, null, `${where}: a payload ran (window.__pwned=${r.pwned})`);
  assert.deepEqual(r.onerror, [], `${where}: raw markup with onerror injected: ${JSON.stringify(r.onerror)}`);
  assert.deepEqual(r.marks, [], `${where}: marker element injected outside .md-render: ${JSON.stringify(r.marks)}`);
}

async function escapeAll(app) {
  for (let i = 0; i < 3; i++) {
    await app.keys(Key.escape);
    await sleep(60);
  }
}

async function runCommand(app, name) {
  await escapeAll(app);
  await app.exec(`document.activeElement && document.activeElement.blur(); return true`);
  await app.chord(Key.ctrl, "p");
  const input = await app.s.findWait("[data-testid=palette-input]");
  await app.s.type(input, name);
  await app.s.waitFor(
    `return [...document.querySelectorAll('[data-testid=palette-item]')].some((e) => e.textContent.includes(${JSON.stringify(name)}))`,
    { message: `palette shows ${name}` },
  );
  await app.keys(Key.enter);
  await sleep(250);
}

/** Open a note by exact data-path (works for names with quotes). */
async function openPath(app, p) {
  await app.exec(`if (!document.querySelector('[data-testid=file-tree]')) document.querySelector('[data-testid=tab-files]')?.click(); return true`);
  await app.s.waitFor(
    `return [...document.querySelectorAll('[data-testid=tree-row]')].some((e) => e.dataset.path === arguments[0])`.replace("arguments[0]", JSON.stringify(p)),
    { message: `tree row ${p}` },
  );
  await app.exec(`[...document.querySelectorAll('[data-testid=tree-row]')].find((e) => e.dataset.path === arguments[0]).click(); return true`, p);
  await eventually(async () => (await app.activeTab()) === p, { message: `tab ${p} active` });
  await sleep(300);
}

// ---------------------------------------------------------------------------
// Shared hostile vault, one app instance for the "held up" surface tests.
// ---------------------------------------------------------------------------
let env, app, beacon;

const TARGET_BODY = `---
title: "${P(3)}"
tags: ["${'<b class=xssm>t'}", "evil\\"><i class=xssm>"]
"<i class=xssm>key": "v ${P(6)}"
aliases: ['"><b class=xssm>alias']
---
# Head ${P(4)}
## Second "><b class=xssm>h
needle ${P(5)} needle
`;

before(async () => {
  beacon = await beaconServer();
  env = freshEnv({
    "Welcome.md": "# Welcome\n\nhello\n",
    [TARGET]: TARGET_BODY,
    [QUOTE]: "quote note\n",
    [`${FOLDER}/Inner ${P(7)}.md`]: `# inner\n\nsee [[Target ${P(2)}]] context ${P(8)} here\n`,
    "Linker.md": `links: [[Target ${P(2)}]] then ${P(9)} and [[Missing ${P(10)}]]\n`,
    [`.cairn/snippets/${SNIPPET}`]: ":root { }\n",
    ".cairn/plugins/evil.js": `// @name Evil ${P(14)}
// @description Desc "><b class=xssm>d
cairn.commands.register("go", 'Cmd "><b class=xssm>c ${P(15)}', async () => {});
`,
    ".cairn/settings.json": JSON.stringify({ plugins: ["evil.js"], snippets: [] }),
  });
  approvePlugins(path.join(env.xdg, "config"), env.vault.root, ["evil.js"]); // turned on on this device
  app = await launch({ vault: env.vault.root, xdg: env.xdg, waitRows: 3 });
  await app.fakeFocus();
});

after(async () => {
  try {
    if (app) fs.writeFileSync(path.join(EVID, "final.png"), await app.s.screenshot());
  } catch {}
  await app?.stop();
  await env?.cleanup();
  beacon?.srv.close();
});

test("tab title and tab tooltip show a hostile note name as text (held up)", async () => {
  const tabInfo = () =>
    app.exec(
      `return [...document.querySelectorAll('[data-testid=tab]')].map((t) => ({ label: t.querySelector('.label').textContent, title: t.getAttribute('title'), attrs: t.getAttributeNames() }))`,
    );
  await openPath(app, TARGET);
  const t1 = await tabInfo();
  assertClean(await probe(app), "tab bar (img name)");
  await openPath(app, QUOTE);
  const t2 = await tabInfo();
  evidence("tabs.json", { t1, t2 });
  assert.ok(t1.some((t) => t.label.includes("<img src=x class=xssm")), JSON.stringify(t1));
  assert.ok(t2.some((t) => t.title === QUOTE), `quote in name must stay inside the title attribute: ${JSON.stringify(t2)}`);
  assert.ok(t2.every((t) => t.attrs.length === 7), `no extra attributes: ${JSON.stringify(t2)}`);
  assertClean(await probe(app), "tab bar (quote name)");
});

test("outline panel shows hostile headings as text (held up)", async () => {
  await openPath(app, TARGET);
  await app.exec(`document.querySelector('[data-testid=right-outline]').click(); return true`);
  const txt = await app.s.waitFor(`const o = document.querySelector('[data-testid=outline]'); return o && o.textContent.includes('Head') ? o.textContent : null`);
  evidence("outline.txt", txt);
  assert.ok(txt.includes("<img src=x class=xssm"), txt);
  // Inline HTML tags are dropped from heading text by the parser (the tag, not
  // its text): "Second \"><b class=xssm>h" is listed as "Second \">h".
  assert.ok(txt.includes('Second ">h'), txt);
  assertClean(await probe(app), "outline");
});

test("properties panel shows hostile frontmatter keys, values and tags as text (held up)", async () => {
  await openPath(app, TARGET);
  await app.exec(`document.querySelector('[data-testid=right-properties]').click(); return true`);
  const txt = await app.s.waitFor(`const o = document.querySelector('[data-testid=properties]'); return o && o.textContent.includes('title') ? o.textContent : null`);
  evidence("properties.txt", txt);
  assert.ok(txt.includes("<i class=xssm>key"), txt);
  assert.ok(txt.includes(`v <img src=x class=xssm`), txt);
  assertClean(await probe(app), "properties panel");
});

test("Live Preview properties box shows hostile frontmatter as text (held up)", async () => {
  await openPath(app, TARGET);
  await app.setMode("live");
  // Put the cursor at the end so the frontmatter is rendered as the box.
  await app.focusEnd();
  await app.blur();
  const html = await app.s.waitFor(`const b = document.querySelector('.cm-lp-props'); return b ? b.innerHTML : null`, { message: "LP properties box" });
  const txt = await app.exec(`return document.querySelector('.cm-lp-props').textContent`);
  evidence("lp-props.html", html);
  assert.ok(txt.includes("<i class=xssm>key"), txt);
  // The box is .md-render-classed, so check it explicitly.
  assert.ok(!(await app.exec(`return !!document.querySelector('.cm-lp-props .xssm, .cm-lp-props [onerror]')`)), html);
  assertClean(await probe(app), "LP properties box");
});

test("tags panel shows hostile frontmatter tags as text (held up)", async () => {
  await runCommand(app, "Show tags");
  const rows = await app.s.waitFor(
    `const r = [...document.querySelectorAll('[data-testid=tag-row]')].map((e) => e.textContent); return r.length ? r : null`,
    { message: "tag rows" },
  );
  evidence("tags.json", rows);
  assertClean(await probe(app), "tags panel");
  await runCommand(app, "Show files");
});

test("backlinks and outgoing-links panels show hostile names and context lines as text (held up)", async () => {
  await openPath(app, TARGET);
  await app.exec(`document.querySelector('[data-testid=right-links]').click(); return true`);
  const txt = await app.s.waitFor(
    `const b = document.querySelector('[data-testid=backlinks]'); return b && document.querySelectorAll('[data-testid=backlink-source]').length >= 2 ? b.textContent : null`,
    { timeout: 8000, message: "two backlink sources" },
  );
  evidence("backlinks.txt", txt);
  assert.ok(txt.includes(`context <img src=x class=xssm onerror=window.__pwned=8>`), txt);
  assert.ok(txt.includes(`Inner <img src=x class=xssm`), txt);
  assertClean(await probe(app), "backlinks");
  await openPath(app, "Linker.md");
  const out = await app.s.waitFor(`const b = document.querySelector('[data-testid=backlinks]'); return b && b.textContent.includes('Missing') ? b.textContent : null`, {
    message: "outgoing links",
  });
  evidence("outgoing.txt", out);
  assertClean(await probe(app), "outgoing links");
});

test("search results and snippet highlighting show HTML next to the hit as text (held up)", async () => {
  await runCommand(app, "Search in all notes");
  const input = await app.s.findWait("[data-testid=search-input]");
  await app.s.type(input, "needle");
  const res = await app.s.waitFor(
    `const r = document.querySelector('[data-testid=search-results]'); return r && r.querySelector('mark.hit') ? r.innerHTML : null`,
    { timeout: 8000, message: "search hits" },
  );
  const txt = await app.exec(`return document.querySelector('[data-testid=search-results]').textContent`);
  evidence("search.html", res);
  assert.ok(txt.includes("<img src=x class=xssm onerror=window.__pwned=5>"), txt);
  assertClean(await probe(app), "search results");
  // A query that is itself HTML.
  await app.exec(`const i = document.querySelector('[data-testid=search-input]'); i.value = ''; i.dispatchEvent(new Event('input', { bubbles: true })); return true`);
  await app.s.type(input, "xssm onerror");
  await sleep(600);
  assertClean(await probe(app), "search results (HTML-ish query)");
  await runCommand(app, "Show files");
});

test("quick switcher lists hostile names (and the create row echoes a hostile query) as text (held up)", async () => {
  await escapeAll(app);
  await app.exec(`document.activeElement && document.activeElement.blur(); return true`);
  await app.chord(Key.ctrl, "o");
  const input = await app.s.findWait("[data-testid=switcher-input]");
  await app.s.type(input, "xssm");
  const items = await app.s.waitFor(
    `const r = [...document.querySelectorAll('[data-testid=switcher-item]')].map((e) => e.textContent); return r.length ? r : null`,
    { message: "switcher items" },
  );
  evidence("switcher.json", items);
  assert.ok(items.some((t) => t.includes("<img src=x class=xssm")), JSON.stringify(items));
  assertClean(await probe(app), "quick switcher");
  await app.exec(`const i = document.querySelector('[data-testid=switcher-input]'); i.value = ''; i.dispatchEvent(new Event('input', { bubbles: true })); return true`);
  await app.s.type(input, `new "><img src=x class=xssm onerror=window.__pwned=40>`);
  await app.s.findWait("[data-testid=switcher-create]");
  assertClean(await probe(app), "quick switcher create row");
  await escapeAll(app);
});

test("wikilink autocomplete lists hostile names as text (held up)", async () => {
  await openPath(app, "Welcome.md");
  await app.setMode("source");
  await app.focusEnd();
  await app.keys("\n[[Targ");
  const opts = await app.s.waitFor(
    `const r = [...document.querySelectorAll('.cm-tooltip-autocomplete li')].map((e) => e.textContent); return r.length ? r : null`,
    { timeout: 6000, message: "autocomplete options" },
  );
  evidence("autocomplete.json", opts);
  assert.ok(opts.some((t) => t.includes("<img src=x class=xssm")), JSON.stringify(opts));
  assertClean(await probe(app), "wikilink autocomplete");
  await app.keys(Key.escape);
  // undo the typing
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: "# Welcome\\n\\nhello\\n" } }); return true`);
});

test("graph labels and graph search with hostile names inject nothing (held up)", async () => {
  await runCommand(app, "Open graph view");
  await app.s.waitFor(`const s = document.querySelector('[data-testid=graph-stats]'); return s && /notes/.test(s.textContent)`, { timeout: 10000 });
  const labels = await app.exec(
    `const g = document.querySelector('.graph-view .canvas').__graph; return g ? g.mapNodes((k, a) => a.label) : null`,
  );
  evidence("graph-labels.json", labels);
  assert.ok(labels && labels.some((l) => String(l).includes("<img src=x class=xssm")), JSON.stringify(labels));
  const q = await app.s.findWait(".graph-view input.text-input");
  await app.s.type(q, `zz ${P(41)}`);
  await app.keys(Key.enter);
  await app.s.waitFor(`return [...document.querySelectorAll('.toast')].some((t) => t.textContent.includes('No note named'))`, { message: "graph search toast" });
  assertClean(await probe(app), "graph view + toast");
  await runCommand(app, "Close tab");
});

test("delete confirmation for a hostile note shows the name as text (held up)", async () => {
  await openPath(app, TARGET);
  await runCommand(app, "Delete current note");
  const d = await app.s.waitFor(`const d = document.querySelector('.dialog'); return d ? d.textContent : null`, { message: "confirm dialog" });
  evidence("confirm-dialog.txt", d);
  assert.ok(d.includes("<img src=x class=xssm"), d);
  assertClean(await probe(app), "confirm dialog");
  await escapeAll(app);
  assert.ok(env.vault.exists(TARGET), "cancelling the dialog must not delete the note");
});

test("embed error messages echo a hostile target and heading as text (reading view and Live Preview) (held up)", async () => {
  env.vault.write("Embedder.md", `![[Target ${P(2)}#Nope "><b class=xssm>s ${P(43)}]]\n\n![[Gone ${P(44)}]]\n`);
  await app.invoke("rescan", {});
  await openPath(app, "Embedder.md");
  const check = async (where, root) => {
    const r = await app.s.waitFor(
      `const m = [...document.querySelectorAll('${root} .embed-missing')].map((e) => e.textContent); return m.length >= 2 ? { m, bad: document.querySelectorAll('${root} .embed .xssm, ${root} .embed [onerror]').length } : null`,
      { timeout: 8000, message: `${where} embed messages` },
    );
    evidence(`embeds-${where}.json`, r);
    assert.ok(r.m.some((t) => t.includes('Nope "><b class=xssm>s')), JSON.stringify(r));
    assert.ok(r.m.some((t) => t.includes("Gone <img src=x class=xssm")), JSON.stringify(r));
    assert.equal(r.bad, 0, `${where}: markup injected into an embed message`);
    assertClean(await probe(app), where);
  };
  await app.setMode("preview");
  await check("preview", "[data-testid=preview]");
  await app.setMode("live");
  await app.focusEnd();
  await app.keys("\n\n");
  await app.blur();
  await check("live", ".cm-content");
});

test("error toast that echoes a hostile link target shows it as text (held up)", async () => {
  // A broken link whose target cannot be created ('..' segment) makes the app
  // show "Could not create <target>: ..." with the note-controlled name.
  env.vault.write("BadLink.md", `[[../up ${P(42)}]]\n`);
  await app.invoke("rescan", {});
  await openPath(app, "BadLink.md");
  await app.setMode("preview");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=preview] .internal-link')`, { message: "preview link" });
  await app.exec(`document.querySelector('[data-testid=preview] .internal-link').click(); return true`);
  const toasts = await eventually(async () => {
    const t = await app.toasts();
    return t.length ? t : null;
  }, { message: "a toast" }).catch(() => []);
  evidence("error-toast.json", { toasts, files: env.vault.list() });
  assertClean(await probe(app), "error toast");
  await app.setMode("live");
});

test("Settings: CSS snippet names and plugin command names in the hotkey list are text (held up)", async () => {
  await runCommand(app, "Open settings");
  await app.s.findWait("[data-testid=settings]");
  // Appearance tab holds snippets; find it by text.
  await app.click("[data-testid=settings-appearance]");
  const txt = await app.s.waitFor(
    `const t = document.querySelector('[data-testid=settings]').textContent; return t.includes('snip ') ? t : null`,
    { message: "snippet listed" },
  );
  assert.ok(txt.includes(`snip <img src=x class=xssm onerror=window.__pwned=13>.css`), "snippet name shown literally");
  assertClean(await probe(app), "settings snippet list");
  await app.click("[data-testid=settings-plugins]");
  const pl = await app.s.waitFor(`const r = document.querySelector('[data-testid=plugin-row]'); return r ? r.textContent : null`, { message: "plugin row" });
  assert.ok(pl.includes('Desc "><b class=xssm>d'), pl);
  assertClean(await probe(app), "settings plugin list");
  await app.click("[data-testid=settings-hotkeys]");
  const hk = await app.s.waitFor(
    `const r = [...document.querySelectorAll('[data-testid=hotkey-row] .hk-name')].map((e) => e.textContent).filter((t) => t.includes('Cmd')); return r.length ? r : null`,
    { message: "plugin command in hotkey list" },
  );
  evidence("hotkeys.json", hk);
  assert.ok(hk[0].includes('"><b class=xssm>c'), JSON.stringify(hk));
  assertClean(await probe(app), "settings hotkey list");
  await escapeAll(app);
});

// ---------------------------------------------------------------------------
// Welcome screen: recent-vault list with a vault folder whose name is HTML.
// ---------------------------------------------------------------------------
test("Welcome recent-vault list shows a hostile vault folder name as text (held up)", async () => {
  const e2 = freshEnv({});
  const dir = path.join(e2.tmp, `vault "><b class=xssm>x ${P(30)}`);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "A.md"), "# a\n");
  let a2;
  try {
    a2 = await launch({ vault: dir, xdg: e2.xdg, waitRows: 1 });
    await runCommand(a2, "Switch notebook");
    const rec = await a2.s.waitFor(
      `const r = [...document.querySelectorAll('.recent-open')].map((e) => ({ text: e.textContent, title: e.getAttribute('title') })); return r.length ? r : null`,
      { timeout: 8000, message: "recent list" },
    );
    evidence("welcome-recent.json", rec);
    assert.ok(rec.some((r) => r.title === dir), JSON.stringify(rec));
    assert.ok(rec.some((r) => r.text.includes("<img src=x class=xssm")), JSON.stringify(rec));
    assertClean(await probe(a2), "welcome recent list");
  } finally {
    await a2?.stop();
    await e2.cleanup();
  }
});

// ---------------------------------------------------------------------------
// A sync server that answers with an HTML error body: the message is echoed
// in Settings > Sync. Must be text.
// ---------------------------------------------------------------------------
test("a sync server's HTML error message is shown as text in Settings (held up)", async () => {
  const body = `<img src=x class=xssm onerror=window.__pwned=50><b class=xssm>server says no`;
  const srv = http.createServer((req, res) => {
    req.resume();
    res.writeHead(500, { "content-type": "text/html" });
    res.end(body);
  });
  await new Promise((r) => srv.listen(0, "127.0.0.1", r));
  const port = srv.address().port;
  try {
    await runCommand(app, "Open settings");
    await app.click("[data-testid=settings-sync]");
    const fill = async (id, v) => {
      await app.s.findWait(`[data-testid=${id}]`);
      await app.exec(`const i = document.querySelector('[data-testid=' + arguments[0] + ']'); i.value = arguments[1]; i.dispatchEvent(new Event('input', { bubbles: true })); return true`, id, v);
    };
    await fill("sync-server", `http://127.0.0.1:${port}`);
    await fill("sync-token", "t");
    await fill("sync-vault", "v");
    await fill("sync-device", `dev ${P(51)}`);
    await fill("sync-pass", "correct horse battery");
    await fill("sync-pass2", "correct horse battery");
    await app.click("[data-testid=sync-connect]");
    const err = await app.s.waitFor(`const e = document.querySelector('[data-testid=sync-error]'); return e ? e.textContent : null`, {
      timeout: 15000,
      message: "sync error",
    });
    evidence("sync-error.txt", err);
    assertClean(await probe(app), "sync error message");
  } finally {
    srv.close();
    await escapeAll(app);
  }
});

// ---------------------------------------------------------------------------
// Sync surfaces: conflict-copy list and version-history modal, with a note
// name and a remote device name carrying HTML (real cairn-server + sync_dir).
// ---------------------------------------------------------------------------
test("conflict-copy list and version history show hostile note and device names as text (held up)", async () => {
  const ROOT = path.resolve(import.meta.dirname, "..");
  const { spawn, execFileSync } = await import("node:child_process");
  const TOKEN = "xs-token-0123456789abcdef";
  const PASS = "xs passphrase for sync";
  const NOTE = `H ${P(60)}.md`;
  const DEV = `phone "><b class=xssm>p ${P(61)}`;
  const e5 = freshEnv({ [NOTE]: "one\n" });
  const stateB = path.join(e5.tmp, "phone-state");
  const vaultB = path.join(e5.tmp, "phone");
  fs.mkdirSync(vaultB, { recursive: true });
  const port = 19000 + Math.floor(Math.random() * 900);
  const url = `http://127.0.0.1:${port}`;
  const server = spawn(path.join(ROOT, "target/debug/cairn-server"), [], {
    env: { ...process.env, CAIRN_TOKENS: TOKEN, CAIRN_DATA: path.join(e5.tmp, "server"), CAIRN_ADDR: `127.0.0.1:${port}` },
    stdio: "ignore",
  });
  const syncB = () => JSON.parse(execFileSync(path.join(ROOT, "target/debug/examples/sync_dir"), [vaultB, stateB, url, TOKEN, "xs", DEV, PASS], { encoding: "utf8" }));
  let a5;
  try {
    await eventually(async () => (await fetch(`${url}/health`)).ok, { message: "server up" });
    a5 = await launch({ vault: e5.vault.root, xdg: e5.xdg, waitRows: 1 });
    const set = (id, v) =>
      a5.exec(`const i = document.querySelector('[data-testid=' + arguments[0] + ']'); i.value = arguments[1]; i.dispatchEvent(new Event('input', { bubbles: true })); return true`, id, v);
    await a5.click("[data-testid=open-settings]");
    await a5.click("[data-testid=settings-sync]");
    await a5.s.findWait("[data-testid=sync-server]");
    await set("sync-server", url);
    await set("sync-token", TOKEN);
    await set("sync-vault", "xs");
    await set("sync-device", "laptop");
    await set("sync-pass", PASS);
    await set("sync-pass2", PASS);
    await a5.click("[data-testid=sync-connect]");
    await a5.click("[data-testid=dialog-ok]"); // a new vault: "Create it?"
    await a5.s.waitFor(`return document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle'`, { timeout: 30000 });
    await escapeAll(a5);
    syncB();
    fs.writeFileSync(path.join(vaultB, NOTE), "phone\n");
    syncB();
    e5.vault.write(NOTE, "laptop\n");
    await sleep(300);
    await a5.click("[data-testid=sync-indicator]");
    await eventually(() => e5.vault.list().some((f) => f.includes("(conflict")), { timeout: 20000, message: "conflict copy" }).catch((e) => {
      throw new Error(`${e.message}; app vault: ${JSON.stringify(e5.vault.list())}; note: ${e5.vault.read(NOTE)}; phone: ${JSON.stringify(fs.readdirSync(vaultB))}`);
    });
    await a5.click("[data-testid=open-settings]");
    await a5.click("[data-testid=settings-sync]");
    const conflicts = await a5.s.waitFor(
      `const r = [...document.querySelectorAll('[data-testid=settings] .linkish')].map((e) => e.textContent); return r.length ? r : null`,
      { timeout: 10000, message: "conflict list" },
    );
    assert.ok(conflicts.some((c) => c.includes("<img src=x class=xssm")), JSON.stringify(conflicts));
    assertClean(await probe(a5), "conflict-copy list");
    await escapeAll(a5);
    await a5.open(NOTE);
    await runCommand(a5, "version history");
    const hist = await a5.s.waitFor(
      `const h = document.querySelector('[data-testid=history]'); return h && document.querySelectorAll('[data-testid=history-entry]').length >= 2 ? h.textContent : null`,
      { timeout: 10000, message: "history entries" },
    );
    evidence("sync-surfaces.json", { conflicts, hist, files: e5.vault.list() });
    assert.ok(hist.includes('phone "><b class=xssm>p'), hist);
    assertClean(await probe(a5), "version history modal");
  } finally {
    await a5?.stop();
    server.kill();
    await e5.cleanup();
  }
});

// ---------------------------------------------------------------------------
// External links: the opener plugin may open http, https and mailto URLs only.
// ---------------------------------------------------------------------------
test(
  "FINDING-023: clicking an https link asks the system to open it (opener scope allows https)",
  async () => {
    const e3 = freshEnv({ "Links.md": "[site](https://example.com/page)\n" });
    const shimDir = path.join(e3.tmp, "bin");
    const canary = path.join(e3.tmp, "opened.txt");
    fs.mkdirSync(shimDir, { recursive: true });
    for (const n of ["xdg-open", "gio", "gnome-open", "kde-open", "wslview"]) {
      fs.writeFileSync(path.join(shimDir, n), `#!/bin/sh\nprintf '%s %s\\n' "${n}" "$*" >> ${JSON.stringify(canary)}\nexit 0\n`);
      fs.chmodSync(path.join(shimDir, n), 0o755);
    }
    let a3;
    try {
      a3 = await launch({ vault: e3.vault.root, xdg: e3.xdg, waitRows: 1, env: { PATH: `${shimDir}:${process.env.PATH}`, BROWSER: path.join(shimDir, "xdg-open") } });
      await a3.exec(`window.__adv.blockOpen = false; return true`);
      // 0. only web and mail URLs may be handed to the system
      const others = {};
      for (const url of ["file:///etc/hostname", "javascript:alert(1)", "vault://localhost/Links.md"]) {
        others[url] = await a3.invoke("plugin:opener|open_url", { url });
      }
      const allowedOthers = Object.entries(others).filter(([, r]) => !r.err).map(([u]) => u);
      // 1. the call the app makes, straight through IPC
      const direct = await a3.invoke("plugin:opener|open_url", { url: "https://example.com/direct" });
      // 2. the real UI path: click the link in the reading view
      await a3.open("Links.md");
      await a3.setMode("preview");
      await a3.s.waitFor(`return !!document.querySelector('[data-testid=preview] a[href^="https"]')`);
      await a3.exec(`document.querySelector('[data-testid=preview] a[href^="https"]').click(); return true`);
      await sleep(1500);
      const toasts = await a3.toasts();
      const opened = fs.existsSync(canary) ? fs.readFileSync(canary, "utf8") : "";
      evidence("XS-01-opener.json", { others, direct, toasts, opened });
      assert.deepEqual(allowedOthers, [], `open_url accepted non-web URLs: ${opened}`);
      assert.ok(!direct.err, `open_url refused: ${JSON.stringify(direct)}`);
      assert.deepEqual(toasts.filter((t) => /Could not open/.test(t)), [], `clicking the link showed: ${JSON.stringify(toasts)}`);
      assert.match(opened, /example\.com/, "the system opener was never launched");
    } finally {
      await a3?.stop();
      await e3.cleanup();
    }
  },
);

// ---------------------------------------------------------------------------
// CSS injection from a vault's own .cairn/settings.json: pre-enabled snippets
// are applied on open with no consent, so a vault someone sends you makes the
// app fetch remote URLs on open (no click). The accent value is also applied
// unvalidated (a url() is accepted as --accent) but no fetch was observed from
// it; it is measured separately (accentConnections) as the control.
// ---------------------------------------------------------------------------
test(
  "not a defect (by design): opening a vault does not fetch remote URLs from CSS snippets that its own settings.json pre-enables",
  { todo: "not a defect (by design): a vault's settings.json pre-enables CSS snippets without consent; their url() phones home on open" },
  async () => {
    const b = await beaconServer(); // snippet URL
    const ba = await beaconServer(); // accent URL
    const e4 = freshEnv({
      "A.md": "# a\n\nhello\n",
      ".cairn/snippets/look.css": `body { background-image: url(https://127.0.0.1:${b.port}/snippet.png); }\n`,
      ".cairn/settings.json": JSON.stringify({
        accent: `url(https://127.0.0.1:${ba.port}/accent.png)`,
        snippets: ["look.css"],
      }),
    });
    let a4;
    try {
      a4 = await launch({ vault: e4.vault.root, xdg: e4.xdg, waitRows: 1 });
      await sleep(3000);
      const css = await a4.exec(
        `return { accent: document.documentElement.style.getPropertyValue('--accent'), snippets: [...document.querySelectorAll('style[data-cairn-snippet]')].map((s) => s.dataset.cairnSnippet), bodyBg: getComputedStyle(document.body).backgroundImage }`,
      );
      evidence("XS-02-css.json", { css, snippetConnections: b.hits.length, accentConnections: ba.hits.length });
      assert.equal(b.hits.length + ba.hits.length, 0, `vault open made ${b.hits.length} connection(s) to the snippet URL and ${ba.hits.length} to the accent URL from settings.json: ${JSON.stringify(css)}`);
    } finally {
      await a4?.stop();
      await e4.cleanup();
      b.srv.close();
      ba.srv.close();
    }
  },
);
