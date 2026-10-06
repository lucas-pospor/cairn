// Accessibility audit, part 3: wording and feedback. Triggers the error paths
// a user can hit (bad vault paths, permission errors, sync failures) and
// checks what text the app shows, plus a few messages that used to mislead.
//
// Run:  scripts/e2e-headless.sh e2e/adv_a11y_wording.test.mjs
// One:  scripts/e2e-headless.sh --test-name-pattern 'FINDING-119' e2e/adv_a11y_wording.test.mjs
//
// Tests run in order: the ones that need the vault open come first, the
// Welcome-screen tests (which close the vault) come last and reopen it.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { AxApp, K, SERVER, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-txt-");
// A stand-in for xdg-open that writes down what the app asks it to open, so
// opening an attachment starts no real viewer (on the user's desktop, through
// D-Bus, even from the headless display).
const launched = path.join(app.tmp, "launched.txt");
const launcher = path.join(app.tmp, "bin", "xdg-open");
// Strings that come straight from Rust's io::Error, the HTTP client or a raw
// server response body.
const RAW = /\(os error \d+\)|error sending request|builder error|io error:|\bio: |\bhttp: |invalid format|HTTP \d{3}: \{|tcp connect|relative URL|\bdns\b/i;

before(async () => {
  app.write("Locked/Locked note.md", "locked\n");
  fs.mkdirSync(path.dirname(launcher), { recursive: true });
  fs.writeFileSync(launcher, `#!/bin/sh\nprintf '%s\\n' "$1" >> ${JSON.stringify(launched)}\nexit 0\n`);
  fs.chmodSync(launcher, 0o755);
  await app.start({ PATH: `${path.dirname(launcher)}:${process.env.PATH}` });
});

after(async () => {
  try {
    fs.chmodSync(app.p("Locked"), 0o755);
  } catch {}
  for (const d of ["no-access"]) {
    try {
      fs.chmodSync(path.join(app.tmp, d), 0o755);
    } catch {}
  }
  await app.stop("wording-final.png");
});

const log = (label, v) => console.log(`${label}:\n${typeof v === "string" ? v : JSON.stringify(v, null, 1)}`);

/** Wait for a toast and return its text (toasts disappear after 3.5 s / 7 s). */
async function nextToast(match = "") {
  return app.s.waitFor(
    `const t = [...document.querySelectorAll('.toast')].map(t => t.textContent.trim()).filter(t => t.includes(${JSON.stringify(match)})); return t.length ? t[t.length - 1] : null`,
    { timeout: 8000, message: `a toast containing ${JSON.stringify(match)}` },
  );
}

async function freePort() {
  return new Promise((resolve) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
}

/** Fill and submit the sync form (setup only; the error text is under test). */
async function trySync(server, token) {
  return app.s.execAsync(
    `const done = arguments[arguments.length - 1];
     (async () => {
       if (!document.querySelector('[data-testid=settings]')) document.querySelector('[data-testid=open-settings]').click();
       await new Promise(r => setTimeout(r, 200));
       document.querySelector('[data-testid=settings-sync]').click();
       await new Promise(r => setTimeout(r, 300));
       const set = (id, v) => { const i = document.querySelector('[data-testid=' + id + ']'); i.value = v; i.dispatchEvent(new Event('input', { bubbles: true })); };
       set('sync-server', arguments[0]); set('sync-token', arguments[1]); set('sync-pass', 'passphrase123'); set('sync-pass2', 'passphrase123');
       const form = document.querySelector('.sync-form');
       if (!form.checkValidity()) return 'invalid form: ' + [...form.querySelectorAll('input')].filter(i => !i.checkValidity()).map(i => i.dataset.testid + ' ' + i.validationMessage).join('; ');
       document.querySelector('[data-testid=sync-connect]').click();
       for (let i = 0; i < 300; i++) {
         await new Promise(r => setTimeout(r, 100));
         const e = document.querySelector('[data-testid=sync-error]');
         if (e) return 'error: ' + e.textContent;
         if (document.querySelector('[data-testid=sync-state]')) return 'connected';
       }
       return 'timeout';
     })().then(done, e => done('ERR ' + e));`,
    server,
    token,
  );
}

/** Welcome screen: type a path into "…or type a folder path" and press Open. */
async function openTyped(p) {
  await app.exec(
    `const i = document.querySelector('[data-testid=vault-path]'); i.value = arguments[0]; i.dispatchEvent(new Event('input', { bubbles: true })); document.querySelector('[data-testid=vault-open]').click(); return 1`,
    p,
  );
}

async function toWelcome() {
  await app.palette("switch vault");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=vault-path]')`, { message: "welcome screen" });
}

// ---------------------------------------------------------------------------

test("FINDING-119: clicking an attachment in the tree hands it to the system's default app (no 'is not supported yet' message); an image opens in a tab of its own, and its menu still offers the default app", async () => {
  await app.reset();
  fs.rmSync(launched, { force: true });
  fs.writeFileSync(app.p("doc.pdf"), "%PDF-1.4\n%%EOF\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="doc.pdf"]')`, { timeout: 8000, message: "doc.pdf listed" });
  const launches = () => (fs.existsSync(launched) ? fs.readFileSync(launched, "utf8").trim().split("\n").filter(Boolean) : []);
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="doc.pdf"]').click(); return 1`);
  const opened = await eventually(() => launches()[0], { timeout: 3000, message: "doc.pdf handed to xdg-open" }).catch(() => "");
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="pic.png"]').click(); return 1`);
  const image = await eventually(
    () => app.exec(`const i = document.querySelector('[data-testid=image-view] img'); return i?.complete && i.naturalWidth === 32 && document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path`),
    { timeout: 5000, message: "pic.png shown in a tab" },
  ).catch(() => null);
  const msg = await app.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent.trim()).filter(t => /pic\.png|doc\.pdf/.test(t)).join(' | ')`);
  const menu = await app.exec(`
    const r = document.querySelector('[data-testid=tree-row][data-path="pic.png"]'); const b = r.getBoundingClientRect();
    r.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: b.x + 20, clientY: b.y + 5 }));
    return [...document.querySelectorAll('[role=menuitem]')].map(m => m.textContent.trim());`);
  await app.exec(`[...document.querySelectorAll('[role=menuitem]')].find(m => m.textContent.trim() === 'Open in default app')?.click(); return 1`);
  const fromMenu = await eventually(() => launches()[1], { timeout: 3000, message: "pic.png handed to xdg-open from its menu" }).catch(() => "");
  log("toast", msg);
  log("opened externally", launches());
  log("context menu for pic.png", menu);
  assert.doesNotMatch(msg, /not supported yet/, "README (v2): 'Attachments in the tree open in the system's default app.' app.openAttachment exists and is used for links");
  assert.ok(opened && fs.realpathSync(opened) === fs.realpathSync(app.p("doc.pdf")), "clicking the attachment did not hand it to the system's default app");
  assert.equal(image, "pic.png", "clicking the image did not show it in a tab");
  assert.deepEqual(menu.slice(0, 2), ["Open in new tab", "Open in default app"]);
  assert.ok(fromMenu && fs.realpathSync(fromMenu) === fs.realpathSync(app.p("pic.png")), "Open in default app did not hand the image to the system's default app");
  fs.rmSync(app.p("doc.pdf"));
});

test("FINDING-120: a failed save shows a plain message, not the raw OS error ('Permission denied (os error 13)')", async () => {
  await app.reset();
  await app.openNote("locked note", "Locked/Locked note.md");
  fs.chmodSync(app.p("Locked"), 0o555);
  let msg;
  try {
    await app.keys("x");
    msg = await nextToast("Could not save");
  } finally {
    fs.chmodSync(app.p("Locked"), 0o755);
  }
  log("save failure toast", msg);
  await app.shot("AX-28-save-error.png");
  assert.doesNotMatch(msg, RAW);
});

test("FINDING-120: sync setup errors are plain messages, not raw HTTP-client strings", async () => {
  await app.reset();
  const out = {};
  const dead = await freePort();
  out.unreachable = await trySync(`http://127.0.0.1:${dead}`, "whatever-token");
  out.noScheme = await trySync(`127.0.0.1:${dead}`, "whatever-token");
  const port = await freePort();
  const server = spawn(SERVER, [], {
    env: { ...process.env, CAIRN_TOKENS: "right-token-0123456789", CAIRN_DATA: path.join(app.tmp, "server-txt"), CAIRN_ADDR: `127.0.0.1:${port}` },
    stdio: "ignore",
  });
  try {
    await eventually(async () => (await fetch(`http://127.0.0.1:${port}/health`)).ok, { message: "server up" });
    out.wrongToken = await trySync(`http://127.0.0.1:${port}`, "wrong-token");
    out.notCairn = await trySync(`http://127.0.0.1:${port}/nothing-here`, "right-token-0123456789");
  } finally {
    server.kill();
  }
  await app.shot("AX-28-sync-error.png");
  await app.keys(K.esc);
  log("sync setup errors", out);
  const raw = Object.entries(out).filter(([, v]) => RAW.test(v)).map(([k, v]) => `${k}: ${v}`);
  assert.deepEqual(raw, []);
});

test("FINDING-211: empty-state and tooltip hints stop saying Ctrl+N once 'Create new note' is rebound", async () => {
  await app.reset();
  // Rebind "Create new note" with the Settings recorder: + then Ctrl+Alt+J, then remove Ctrl+N.
  await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await app.exec(`document.querySelector('[data-testid=settings-hotkeys]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-command="note:new"] [data-testid=hotkey-add]')`);
  let texts;
  try {
    await app.exec(`document.querySelector('[data-command="note:new"] [data-testid=hotkey-add]').click(); return 1`);
    await app.chord(K.ctrl, "", "j"); // Ctrl+Alt+J
    await app.s.waitFor(`return document.querySelector('[data-command="note:new"]').textContent.includes('Alt+J')`, { message: "new combo recorded" });
    await app.exec(`[...document.querySelectorAll('[data-command="note:new"] .combo')].find(c => c.textContent.startsWith('Ctrl+N')).querySelector('button').click(); return 1`);
    await app.s.waitFor(`return !document.querySelector('[data-command="note:new"]').textContent.includes('Ctrl+N')`);
    await app.keys(K.esc);
    texts = await app.exec(`return { empty: document.querySelector('.pane .empty')?.textContent.replace(/\\s+/g, ' ').trim(), newTabButton: document.querySelector('.tabbar .new')?.title }`);
  } finally {
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
    await app.exec(`document.querySelector('[data-testid=settings-hotkeys]').click(); return 1`);
    await sleep(200);
    await app.exec(`[...document.querySelectorAll('[data-testid=settings] button')].find(b => b.textContent.trim() === 'Restore all defaults').click(); return 1`);
    await app.keys(K.esc);
    // Settings are written 300 ms after a change; the next test reloads the page.
    await eventually(() => {
      try {
        return Object.keys(JSON.parse(app.read(".cairn/settings.json")).hotkeys ?? {}).length === 0;
      } catch {
        return false;
      }
    }, { message: "default hotkeys saved" });
  }
  log("hints after rebinding 'Create new note' to Ctrl+Alt+J", texts);
  const bad = [];
  if (/Ctrl\+N/.test(texts.empty ?? "")) bad.push(`empty editor still says: "${texts.empty}"`);
  if (/Ctrl\+N/.test(texts.newTabButton ?? "")) bad.push(`tab bar button tooltip still says: "${texts.newTabButton}"`);
  assert.deepEqual(bad, []);
});

test("FINDING-212: renaming a note to a name with '/' does not report a misleading 'Not found: …'", async () => {
  await app.reset();
  app.write("Slash me.md", "slash\n");
  await app.s.waitFor(`return !!document.querySelector('[data-testid=tree-row][data-path="Slash me.md"]')`);
  await app.openNote("slash me", "Slash me.md");
  await app.palette("rename current");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'rename-input'`);
  await app.exec(`const i = document.activeElement; i.value = 'nofolder/new name'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
  await app.keys(K.enter);
  const msg = await nextToast("");
  log("toast after renaming to 'nofolder/new name'", msg);
  assert.ok(app.exists("Slash me.md"));
  assert.doesNotMatch(msg, /^Not found/, "the user typed a name; 'Not found: nofolder' does not say that '/' is not allowed in names (the invalid-name message lists '/')");
});

// ----- Welcome screen (closes the vault; reopens it at the end) -----

test("FINDING-114: Welcome screen: the folder-path box has a name beyond its placeholder and 'Remove from list' buttons say which vault", async () => {
  await app.reset();
  await toWelcome();
  const r = await app.exec(`return __ax.audit(document.querySelector('main.welcome')).map(a => a.el + ' => ' + JSON.stringify(a.name))`);
  log("Welcome screen controls and accessible names", r.join("\n"));
  const bad = r.filter((l) => /=> ""$|\(placeholder\)/.test(l));
  const removes = r.filter((l) => /"Remove from list"$/.test(l));
  if (removes.length) bad.push(`${removes.length} x "Remove from list" with no vault in the name`);
  assert.deepEqual(bad, []);
});

test("FINDING-120: opening a file or an unreadable folder as a vault shows a clear error, not a misleading or raw one", async () => {
  await app.reset().catch(() => {});
  if (!(await app.exec(`return !!document.querySelector('[data-testid=vault-path]')`))) await toWelcome();
  const file = path.join(app.tmp, "a-file.txt");
  fs.writeFileSync(file, "not a folder\n");
  const locked = path.join(app.tmp, "no-access");
  fs.mkdirSync(locked, { recursive: true });
  fs.chmodSync(locked, 0o000);
  const out = {};
  try {
    await openTyped(file);
    out.file = await nextToast("Could not open vault");
    await app.exec(`for (const t of document.querySelectorAll('.toast')) t.remove(); return 1`).catch(() => {});
    await sleep(300);
    await openTyped(locked);
    out.noAccess = await nextToast("Could not open vault");
  } finally {
    fs.chmodSync(locked, 0o755);
  }
  await app.shot("AX-28-open-vault-error.png");
  log("open vault errors", out);
  const bad = [];
  if (/already exists/.test(out.file)) bad.push(`a file path reports "${out.file}" instead of "not a folder"`);
  if (RAW.test(out.noAccess)) bad.push(`unreadable folder: "${out.noAccess}"`);
  assert.deepEqual(bad, []);
});

test("FINDING-121: 'Open' on the Welcome screen asks before creating a new vault for a mistyped path", async () => {
  if (!(await app.exec(`return !!document.querySelector('[data-testid=vault-path]')`))) await toWelcome();
  const typo = path.join(app.tmp, "vualt");
  assert.ok(!fs.existsSync(typo));
  await openTyped(typo);
  let created = false;
  try {
    await app.s.waitFor(`return !!document.querySelector('[data-testid=file-tree]')`, { timeout: 8000, message: "workspace opened" }).catch(() => {});
    created = fs.existsSync(typo);
    const st = await app.exec(`return { workspace: !!document.querySelector('[data-testid=file-tree]'), empty: document.querySelector('.tree .empty')?.textContent ?? null, dialog: !!document.querySelector('[role=dialog]') }`);
    await app.shot("AX-29-typo-vault.png");
    log("after opening a mistyped path", { created, ...st });
    assert.ok(!created || st.dialog, `the folder ${typo} was created and opened as an empty vault without any confirmation: ${JSON.stringify(st)}`);
    // Saying yes makes the folder and opens it.
    if (st.dialog) {
      await app.exec(`document.querySelector('[data-testid=dialog-ok]').click(); return 1`);
      await app.s.waitFor(`return !!document.querySelector('[data-testid=file-tree]')`, { timeout: 8000, message: "new vault opened after saying yes" });
      assert.ok(fs.existsSync(typo), `${typo} was not created after saying yes`);
    }
  } finally {
    // Back to the real vault for anything that runs after (answering "no" to a question first).
    if (await app.exec(`return !!document.querySelector('[role=dialog]')`)) await app.keys(K.esc);
    if (await app.exec(`return !!document.querySelector('[data-testid=file-tree]')`)) await toWelcome();
    await openTyped(app.vault);
    await app.s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 4`, { timeout: 10000 }).catch(() => {});
  }
});
