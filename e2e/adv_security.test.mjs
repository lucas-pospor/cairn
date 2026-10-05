// Adversarial security tests for Cairn's app layer:
//   vault:// protocol, Markdown/XSS sanitization, CSP, IPC exposure,
//   external-open / link handling.
//
// These drive the REAL app (WebKitGTK) through tauri-driver, like
// e2e/app.test.mjs, and render hostile notes through the app's own reading
// view (markdown-it -> DOMPurify -> {@html}). Build first:
//   cd app && npm run e2e:build
// Run:   scripts/e2e-headless.sh e2e/adv_security.test.mjs
//   one:  scripts/e2e-headless.sh --test-name-pattern 'phone home' e2e/adv_security.test.mjs
//
// Behaviour that is by design but kept documented is asserted as the SAFE
// expectation and marked { todo: "not a defect (by design): ..." } so its
// failure prints but does not fail the suite. Scenarios that held up are
// normal passing tests.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import net from "node:net";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const shots = path.join(import.meta.dirname, ".tmp", "AS");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-as-"));
const vault = path.join(tmp, "vault");
const outside = path.join(tmp, "outside"); // never inside the vault
const shimDir = path.join(tmp, "bin"); // fake xdg-open to observe external open
const canary = path.join(tmp, "xdg-open-canary.txt");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}

async function eventually(fn, { timeout = 8000, message = "condition" } = {}) {
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

// A TCP sink that records connection attempts (phone-home / beacon detection).
function beaconServer() {
  const hits = [];
  const srv = net.createServer((sock) => {
    hits.push({ at: Date.now() });
    sock.on("error", () => {});
    setTimeout(() => sock.destroy(), 200);
  });
  return new Promise((resolve) => {
    srv.listen(0, "127.0.0.1", () => resolve({ srv, port: srv.address().port, hits }));
  });
}

let drv, s, beacon;

before(async () => {
  fs.mkdirSync(shots, { recursive: true });
  fs.mkdirSync(outside, { recursive: true });

  // Secret that lives OUTSIDE the vault; used for traversal / symlink checks.
  fs.writeFileSync(path.join(outside, "secret.txt"), "TOP-SECRET-7f3a9c2e");
  fs.mkdirSync(path.join(outside, "dir"), { recursive: true });
  fs.writeFileSync(path.join(outside, "dir", "note.md"), "# outside note\nprivate-c0ffee\n");

  // A fake xdg-open that records being launched, so we can see open_externally
  // hand a vault file to the system launcher (which, on a real desktop, runs it).
  fs.mkdirSync(shimDir, { recursive: true });
  const shim = path.join(shimDir, "xdg-open");
  fs.writeFileSync(shim, `#!/bin/sh\nprintf '%s\\n' "$1" >> ${JSON.stringify(canary)}\nexit 0\n`);
  fs.chmodSync(shim, 0o755);

  // ---- vault contents -------------------------------------------------
  write("Welcome.md", "# Welcome\n\nHello.\n");

  // Symlinks pointing outside the vault.
  try {
    fs.symlinkSync(path.join(outside, "secret.txt"), path.join(vault, "leak.txt"));
    fs.symlinkSync(path.join(outside, "dir"), path.join(vault, "linked"));
  } catch (e) {
    console.warn("symlink setup failed:", e.message);
  }

  // A disguised executable "attachment" the user might click in the tree.
  write("report.pdf.desktop", "[Desktop Entry]\nType=Application\nName=r\nExec=/bin/true\n");
  const sh = path.join(vault, "payload.sh");
  fs.writeFileSync(sh, "#!/bin/sh\ntouch /tmp/cairn-should-not-run\n");
  fs.chmodSync(sh, 0o755);

  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
    PATH: `${shimDir}:${process.env.PATH}`,
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1`, { timeout: 20000 });
  beacon = await beaconServer();
});

after(async () => {
  try {
    if (s) fs.writeFileSync(path.join(shots, "final.png"), await s.screenshot());
  } catch {}
  await s?.close();
  drv?.proc.kill();
  beacon?.srv.close();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

// ------------------------- helpers ----------------------------------------

// Run an IPC invoke from the page; returns { ok } or { err }.
async function invoke(cmd, args) {
  const r = await s.execAsync(
    `const cb = arguments[arguments.length-1];
     window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {})
       .then(v => cb(JSON.stringify({ ok: v === undefined ? null : v })))
       .catch(e => cb(JSON.stringify({ err: String((e && (e.message || e.detail)) || e) })));`,
    cmd,
    args ?? {},
  );
  return JSON.parse(r);
}

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const activeTab = () => s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);

// Write a note, make the app see it, open it, and switch to reading view.
// Returns the rendered `.md-render` innerHTML and any window.__pwned value.
async function renderInReadingView(rel, md) {
  write(rel, md);
  await invoke("rescan", {});
  await s.waitFor(`return !!document.querySelector('${rowSel(rel)}')`, { timeout: 8000, message: `tree row ${rel}` });
  await s.exec(`window.__pwned = null;`);
  await s.click(await s.find(rowSel(rel)));
  await eventually(async () => (await activeTab()) === rel, { message: `tab ${rel} active` });
  const btn = await s.findWait("[data-testid=mode-preview]", 5000);
  await s.click(btn);
  await s.waitFor(`return !!document.querySelector('[data-testid=preview] .md-render')`, { timeout: 5000, message: "preview rendered" });
  // allow async embeds / image loads / event handlers to fire
  await sleep(600);
  return await s.exec(
    `const el = document.querySelector('[data-testid=preview] .md-render');
     return JSON.stringify({ html: el ? el.innerHTML : null, pwned: window.__pwned ?? null, title: document.title });`,
  ).then(JSON.parse);
}

// ---------------------------------------------------------------------------
// 1. CSP: a note with a remote image phones home when previewed.
// ---------------------------------------------------------------------------
test(
  "not a defect (by design): a note with a remote image does not phone home (CSP)",
  { todo: "not a defect (by design): remote images in notes contact external hosts on preview" },
  async () => {
    const port = beacon.port;
    const before = beacon.hits.length;
    const out = await renderInReadingView(
      "evil-beacon.md",
      `# hi\n\n<img src="https://127.0.0.1:${port}/beacon-raw.png">\n\n![m](https://127.0.0.1:${port}/beacon-md.png)\n`,
    );
    await sleep(2500);
    const after = beacon.hits.length;
    fs.writeFileSync(
      path.join(shots, "csp-beacon.txt"),
      `https img survived sanitization? ${/https:\/\/127\.0\.0\.1/.test(out.html || "")}\n` +
        `beacon connections observed: ${after - before}\nrendered html:\n${out.html}\n`,
    );
    assert.equal(after - before, 0, `preview opened ${after - before} connection(s) to a note-controlled remote host`);
  },
);

// Control: a plain http:// host is NOT allowed by the CSP, proving the leak
// above is governed by the policy's `https:` image source.
test("http remote image host is blocked by CSP (control)", async () => {
  const port = beacon.port;
  const before = beacon.hits.length;
  await renderInReadingView("http-control.md", `<img src="http://127.0.0.1:${port}/http-control.png">\n`);
  await sleep(1500);
  const n = beacon.hits.length - before;
  fs.writeFileSync(path.join(shots, "csp-http-control.txt"), `http connections: ${n}`);
  assert.equal(n, 0, "plain http image unexpectedly connected (CSP should block non-localhost http)");
});

// ---------------------------------------------------------------------------
// 2. Symlink traversal: files outside the vault reachable through the vault
//    path API, defeating the "..-rejected => cannot escape" invariant.
// ---------------------------------------------------------------------------
// Not a defect, by design: the app follows the symlinks the user
// made; only plugins are kept inside the vault (e2e/adv_verify_as_02.test.mjs,
// e2e/adv_verify_as_04.test.mjs).
test(
  "not a defect (by design): a symlink in the vault cannot read a file outside the vault",
  { todo: "not a defect (by design): the app follows symlinks out of the vault (read_text_file/read_note/vault://); plugins cannot" },
  async () => {
    const r = await invoke("read_text_file", { path: "leak.txt" });
    fs.writeFileSync(path.join(shots, "symlink-read.txt"), JSON.stringify(r));
    assert.ok(!(r.ok && r.ok.includes("TOP-SECRET")), `read a file outside the vault via symlink: ${JSON.stringify(r)}`);
  },
);

test(
  "not a defect (by design): a symlinked directory is not indexed/listed into the vault",
  { todo: "not a defect (by design): a symlinked directory exposes an outside subtree in the tree/index" },
  async () => {
    const r = await invoke("list_entries");
    const paths = Array.isArray(r.ok) ? r.ok.map((e) => e.path) : [];
    fs.writeFileSync(path.join(shots, "symlink-list.txt"), paths.join("\n"));
    assert.ok(
      !paths.includes("linked/note.md"),
      `outside file surfaced in the vault index: ${paths.filter((p) => p.startsWith("linked")).join(", ")}`,
    );
  },
);

// ---------------------------------------------------------------------------
// 3. Symlinked directory => vault write/delete escapes the vault (data loss).
// ---------------------------------------------------------------------------
// Not a defect, by design (see section 2 above).
test(
  "not a defect (by design): writing through a symlinked folder cannot modify a file outside the vault",
  { todo: "not a defect (by design): write_note through a symlinked folder writes to the outside file; plugins cannot" },
  async () => {
    const r = await invoke("write_note", { path: "linked/note.md", content: "OVERWRITTEN-BY-CAIRN\n", baseHash: null });
    const p = path.join(outside, "dir", "note.md");
    const onDisk = fs.existsSync(p) ? fs.readFileSync(p, "utf8") : "(gone)";
    fs.writeFileSync(path.join(shots, "symlink-write.txt"), `invoke=${JSON.stringify(r)}\noutside file now: ${onDisk}`);
    assert.ok(!onDisk.includes("OVERWRITTEN-BY-CAIRN"), "a vault write escaped the vault and overwrote an outside file");
  },
);

test("FINDING-020: plugin calls (inVault) cannot read or write a file outside the vault through a symlink", async () => {
  const p = path.join(outside, "dir", "fenced.md");
  fs.writeFileSync(p, "outside-original\n");
  await invoke("rescan", {});
  const calls = [
    await invoke("read_note", { path: "linked/fenced.md", inVault: true }),
    await invoke("read_note", { path: "leak.txt", inVault: true }),
    await invoke("write_note", { path: "linked/fenced.md", content: "FENCED\n", baseHash: null, inVault: true }),
    await invoke("create_note", { path: "linked/new-fenced.md", content: "FENCED\n", inVault: true }),
  ];
  for (const r of calls) assert.ok("err" in r, `a plugin call reached outside the vault: ${JSON.stringify(r)}`);
  assert.equal(fs.readFileSync(p, "utf8"), "outside-original\n");
  assert.ok(!fs.existsSync(path.join(outside, "dir", "new-fenced.md")));
  // A note of the vault itself is read the same way.
  const own = await invoke("read_note", { path: "Welcome.md", inVault: true });
  assert.match(own.ok?.content ?? "", /Hello/, JSON.stringify(own));
});

test(
  "not a defect (by design): deleting through a symlinked folder cannot remove a file outside the vault",
  { todo: "not a defect (by design): delete_entry through a symlinked folder removes an outside file" },
  async () => {
    fs.writeFileSync(path.join(outside, "dir", "victim.md"), "please-keep-me\n");
    await invoke("rescan", {});
    const r = await invoke("delete_entry", { path: "linked/victim.md" });
    const stillThere = fs.existsSync(path.join(outside, "dir", "victim.md"));
    fs.writeFileSync(path.join(shots, "symlink-delete.txt"), `invoke=${JSON.stringify(r)}\noutside victim still present: ${stillThere}`);
    assert.ok(stillThere, "a vault delete removed a file that lives outside the vault");
  },
);

// ---------------------------------------------------------------------------
// 4. open_externally: any vault file handed to the system launcher with no
//    gate, so a disguised executable/.desktop/script runs on click.
// ---------------------------------------------------------------------------
test(
  "FINDING-066: open_externally refuses to launch an executable/.desktop vault file",
  async () => {
    try { fs.rmSync(canary); } catch {}
    const r1 = await invoke("open_externally", { path: "report.pdf.desktop" });
    const r2 = await invoke("open_externally", { path: "payload.sh" });
    await sleep(700);
    const launched = fs.existsSync(canary) ? fs.readFileSync(canary, "utf8") : "";
    fs.writeFileSync(path.join(shots, "open-externally.txt"), `desktop=${JSON.stringify(r1)}\nsh=${JSON.stringify(r2)}\nlauncher saw:\n${launched}`);
    assert.equal(launched.trim(), "", `open_externally launched the system opener on: ${launched.trim()}`);
  },
);

test("FINDING-066: open_externally still opens documents and media, and refuses other types and executable text", async () => {
  const dir = path.join(vault, "open");
  fs.mkdirSync(dir, { recursive: true });
  const put = (name, content, mode = 0o644) => {
    fs.writeFileSync(path.join(dir, name), content);
    fs.chmodSync(path.join(dir, name), mode);
  };
  put("doc.pdf", "%PDF-1.4\n%%EOF\n");
  put("Photo.JPG", "not really a jpeg\n");
  put("exfat.pdf", "%PDF-1.4\n%%EOF\n", 0o755); // every file on an exFAT or NTFS drive has the x bit
  put("notes.txt", "hello\n");
  put("run.txt", "#!/bin/sh\ntouch /tmp/cairn-should-not-run\n", 0o755);
  put("README", "no extension\n");
  put("tool.py", "print(1)\n");
  fs.symlinkSync(path.join(vault, "payload.sh"), path.join(dir, "cute.png"));
  // Data files open too (as text: the executable bit refuses them) ...
  put("data.json", "{}\n");
  put("config.yaml", "a: 1\n");
  put("config.YML", "a: 1\n");
  put("meeting.ics", "BEGIN:VCALENDAR\nEND:VCALENDAR\n");
  put("contact.vcf", "BEGIN:VCARD\nEND:VCARD\n");
  put("run.json", "#!/bin/sh\ntouch /tmp/cairn-should-not-run\n", 0o755);
  // ... but web pages, XML and Office files with macros do not: they run scripts or macros in their app.
  for (const name of ["page.html", "page.HTM", "feed.xml", "macro.docm", "macro.xlsm", "macro.pptm"]) put(name, "<x/>\n");
  const opens = ["doc.pdf", "Photo.JPG", "exfat.pdf", "notes.txt", "data.json", "config.yaml", "config.YML", "meeting.ics", "contact.vcf"];
  const refused = ["run.txt", "README", "tool.py", "cute.png", "run.json", "page.html", "page.HTM", "feed.xml", "macro.docm", "macro.xlsm", "macro.pptm"];

  try { fs.rmSync(canary); } catch {}
  const results = {};
  for (const name of [...opens, ...refused]) results[name] = await invoke("open_externally", { path: `open/${name}` });
  const seen = () => (fs.existsSync(canary) ? fs.readFileSync(canary, "utf8") : "");
  await eventually(() => opens.every((n) => seen().includes(`/open/${n}\n`)), { message: `launcher saw ${opens.join(", ")}` });
  await sleep(500);
  const launched = seen();
  fs.writeFileSync(path.join(shots, "open-externally-types.txt"), `${JSON.stringify(results, null, 2)}\nlauncher saw:\n${launched}`);
  for (const n of opens) assert.deepEqual(results[n], { ok: null }, `${n} should open`);
  for (const n of refused) {
    assert.match(results[n].err ?? "", /Reveal in file manager/, `${n} should be refused with a plain message: ${JSON.stringify(results[n])}`);
    assert.ok(!launched.includes(`/open/${n}\n`), `${n} was handed to the system opener`);
  }
});

// ---------------------------------------------------------------------------
// 5. XSS battery through the REAL sanitizer in the WebKit DOM. These are
//    scenarios we expect to HOLD UP (no execution).
// ---------------------------------------------------------------------------
const XSS_NOTES = {
  "raw script tag": `<script>window.__pwned='script'</script>`,
  "img onerror": `<img src=x onerror="window.__pwned='imgerr'">`,
  "svg onload": `<svg onload="window.__pwned='svg'"></svg>`,
  "a javascript href": `[click](javascript:window.__pwned='jshref')`,
  "details ontoggle": `<details open ontoggle="window.__pwned='details'"></details>`,
  "math mxss": `<math><mtext><table><mglyph><style><!--</style><img src onerror=window.__pwned='math'>`,
  "iframe srcdoc": `<iframe srcdoc="&lt;script&gt;window.__pwned='iframe'&lt;/script&gt;"></iframe>`,
  "form action js": `<form action="javascript:window.__pwned='form'"><button>x</button></form>`,
  "object data html": `<object data="data:text/html,<script>window.__pwned='obj'</script>"></object>`,
  "svg xlink js": `<svg><a xlink:href="javascript:window.__pwned='xlink'"><text>x</text></a></svg>`,
};

let xssIdx = 0;
for (const [name, payload] of Object.entries(XSS_NOTES)) {
  test(`XSS held up: ${name}`, async () => {
    const out = await renderInReadingView(`xss-${xssIdx++}.md`, `# payload\n\n${payload}\n`);
    fs.writeFileSync(path.join(shots, `xss-${name.replace(/\W+/g, "_")}.txt`), `payload: ${payload}\npwned: ${out.pwned}\nhtml: ${out.html}`);
    assert.equal(out.pwned, null, `payload executed (window.__pwned=${out.pwned})`);
    // Look for dangerous markup in attribute position only (literal text like
    // "[click](javascript:...)" that was NOT turned into a link is harmless).
    const html = out.html || "";
    assert.ok(!/<script\b/i.test(html), `a <script> survived sanitization: ${html}`);
    assert.ok(!/\s(on[a-z]+)\s*=/i.test(html), `an event-handler attribute survived sanitization: ${html}`);
    assert.ok(!/(href|src|data|action|xlink:href)\s*=\s*["']?\s*(javascript|data):/i.test(html), `a dangerous URL attribute survived sanitization: ${html}`);
  });
}

// ---------------------------------------------------------------------------
// 5b. Hostile file / folder NAMES created OUTSIDE Cairn (its own validate_name
//     forbids < > etc., but a file manager / sync / git checkout does not).
//     The tree, tabs and toasts must render them as inert text. (Held up.)
// ---------------------------------------------------------------------------
test("hostile file/folder names render as text in the tree (held up)", async () => {
  const noteName = `pwn<img src=x onerror=window.__pwned='name'>.md`;
  const folderName = `d<svg onload=window.__pwned='folder'>`;
  fs.writeFileSync(path.join(vault, noteName), "# x\n");
  fs.mkdirSync(path.join(vault, folderName), { recursive: true });
  fs.writeFileSync(path.join(vault, folderName, "inner.md"), "# y\n");
  await s.exec(`window.__pwned = null;`);
  await invoke("rescan", {});
  await s.waitFor(
    `return [...document.querySelectorAll('[data-testid=tree-row]')].some(e => (e.dataset.path||'').includes('pwn<img'))`,
    { timeout: 8000, message: "hostile-named row present" },
  );
  await sleep(400);
  const res = await s.exec(
    `const rows = [...document.querySelectorAll('[data-testid=tree-row]')];
     const row = rows.find(e => (e.dataset.path||'').includes('pwn<img'));
     return JSON.stringify({ pwned: window.__pwned ?? null, hasImg: !!document.querySelector('[data-testid=file-tree] img[src="x"]'), rowHtml: row ? row.innerHTML.slice(0, 300) : null });`,
  ).then(JSON.parse);
  fs.writeFileSync(path.join(shots, "hostile-names.txt"), JSON.stringify(res, null, 1));
  assert.equal(res.pwned, null, `a hostile file/folder name executed (window.__pwned=${res.pwned})`);
  assert.equal(res.hasImg, false, "a hostile name injected a live <img> into the tree");
});

// ---------------------------------------------------------------------------
// 6. vault:// path traversal with .. / encodings (expected to hold up — the
//    protocol routes through path::normalize which rejects dot components).
// ---------------------------------------------------------------------------
const TRAVERSAL = [
  "../../../../etc/passwd",
  "..%2f..%2fetc%2fpasswd",
  "%2e%2e%2f%2e%2e%2fetc%2fpasswd",
  "..\\..\\etc\\passwd",
  ".cairn/settings.json",
];
for (const p of TRAVERSAL) {
  test(`vault path traversal blocked: ${p}`, async () => {
    const r = await invoke("read_text_file", { path: p });
    fs.writeFileSync(path.join(shots, `traversal-${p.replace(/\W+/g, "_")}.txt`), JSON.stringify(r));
    assert.ok(!(r.ok && /root:|TOP-SECRET|server|token/.test(r.ok)), `traversal leaked: ${JSON.stringify(r)}`);
  });
}
