// Regression tests for FINDING-066 (open_externally handed any vault file,
// including .desktop launchers, scripts and Windows executables, to the
// system opener with no type check or confirmation).
//
// The other reproduction (e2e/adv_security.test.mjs) only invokes the IPC
// command directly and treats ANY launcher call as a failure, which would also
// flag opening a PDF (documented, intended behavior). This file instead:
//   - checks the intended behavior still works (a .pdf link opens externally),
//   - drives the realistic user path: a normal-looking aliased wikilink /
//     Markdown link in a note ("Quarterly report (PDF)") whose target is
//     report.pdf.desktop / invoice.pdf.bat, clicked in Live Preview and in the
//     reading view, and a click on the file in the tree.
// A fake xdg-open on PATH records what would be launched (it never runs it).
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_as_06.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-vas06-"));
const vault = path.join(tmp, "vault");
const shimDir = path.join(tmp, "bin");
const canary = path.join(tmp, "launched.txt");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
const launched = () => (fs.existsSync(canary) ? fs.readFileSync(canary, "utf8") : "");
const clearLaunched = () => { try { fs.rmSync(canary); } catch {} };

async function eventually(fn, { timeout = 5000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await fn()) return true;
    await sleep(80);
  }
  throw new Error(`timed out: ${message}`);
}

let drv, s;

before(async () => {
  fs.mkdirSync(shimDir, { recursive: true });
  const shim = path.join(shimDir, "xdg-open");
  fs.writeFileSync(shim, `#!/bin/sh\nprintf '%s\\n' "$1" >> ${JSON.stringify(canary)}\nexit 0\n`);
  fs.chmodSync(shim, 0o755);

  write("Welcome.md", "# Welcome\n");
  write(
    "Report.md",
    "# Q3\n\nSee [[report.pdf.desktop|Quarterly report (PDF)]] and the [[doc.pdf|real PDF]].\n\n" +
      "Also [the invoice](invoice.pdf.bat).\n\nlast line\n",
  );
  // Files as they would arrive by sync / unzip (no executable bit).
  write("report.pdf.desktop", "[Desktop Entry]\nType=Application\nName=r\nExec=sh -c 'touch /tmp/cairn-should-not-run'\n");
  write("invoice.pdf.bat", "@echo off\r\ncalc.exe\r\n");
  write("doc.pdf", "%PDF-1.4\n%%EOF\n");

  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
    PATH: `${shimDir}:${process.env.PATH}`,
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 4`, { timeout: 20000 });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

const rowSel = (p) => `[data-testid=tree-row][data-path="${p}"]`;
const activeTab = () => s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);

async function openReportLive() {
  await s.click(await s.find(rowSel("Report.md")));
  await eventually(async () => (await activeTab()) === "Report.md", { message: "Report.md active" });
  await s.click(await s.find("[data-testid=mode-live]"));
  await s.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } });`);
  await s.waitFor(`return [...document.querySelectorAll('.cm-lp-wikilink')].some(e => e.textContent === 'Quarterly report (PDF)')`, { timeout: 8000 });
}

async function mousedownOn(selector, text) {
  return s.exec(
    `const el = [...document.querySelectorAll(arguments[0])].find(e => e.textContent === arguments[1]);
     if (!el) return false;
     const r = el.getBoundingClientRect();
     el.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, clientX: r.left + 5, clientY: r.top + r.height / 2, button: 0 }));
     return true;`,
    selector,
    text,
  );
}

test("baseline (intended): clicking a link to a PDF opens it with the system app", async () => {
  await openReportLive();
  clearLaunched();
  assert.ok(await mousedownOn(".cm-lp-wikilink", "real PDF"), "PDF link present");
  await eventually(() => launched().includes("doc.pdf"), { message: "launcher called for doc.pdf" });
});

test(
  "FINDING-066: clicking an innocent-looking link in Live Preview does not launch a .desktop file",
  async () => {
    await openReportLive();
    clearLaunched();
    assert.ok(await mousedownOn(".cm-lp-wikilink", "Quarterly report (PDF)"), "aliased link present");
    await sleep(1200);
    const got = launched();
    console.log("launcher saw (live preview .desktop):", JSON.stringify(got));
    assert.ok(!got.includes("report.pdf.desktop"), `a .desktop launcher was handed to the system opener: ${got.trim()}`);
  },
);

test(
  "FINDING-066: clicking a Markdown link to a .bat in the reading view does not launch it",
  async () => {
    await s.click(await s.find(rowSel("Report.md")));
    await eventually(async () => (await activeTab()) === "Report.md", { message: "Report.md active" });
    await s.click(await s.find("[data-testid=mode-preview]"));
    await s.waitFor(`return [...document.querySelectorAll('[data-testid=preview] a')].some(a => a.textContent === 'the invoice')`, { timeout: 8000 });
    clearLaunched();
    await s.exec(`[...document.querySelectorAll('[data-testid=preview] a')].find(a => a.textContent === 'the invoice').click()`);
    await sleep(1200);
    const got = launched();
    console.log("launcher saw (reading view .bat):", JSON.stringify(got));
    await s.click(await s.find("[data-testid=mode-live]"));
    assert.ok(!got.includes("invoice.pdf.bat"), `a .bat file was handed to the system opener: ${got.trim()}`);
  },
);

// Left-click on an attachment row (activate -> openAttachment) and middle-click
// (auxclick button 1 -> openNote(newTab) -> openAttachment) both reach
// open_externally, which refuses a .desktop file: the user gets a toast and
// the launcher is never called.
test("tree left-click on a .desktop attachment does not launch it (held up)", async () => {
  clearLaunched();
  await s.click(await s.find(rowSel("report.pdf.desktop")));
  await sleep(1200);
  const got = launched();
  console.log("launcher saw (tree left-click .desktop):", JSON.stringify(got));
  assert.ok(!got.includes("report.pdf.desktop"), `tree left-click handed a .desktop file to the system opener: ${got.trim()}`);
});

test(
  "FINDING-066: middle-clicking a .desktop file in the tree does not launch it without confirmation",
  async () => {
    clearLaunched();
    await s.exec(
      `const el = document.querySelector(arguments[0]);
       el.dispatchEvent(new MouseEvent('auxclick', { bubbles: true, cancelable: true, button: 1 }));`,
      rowSel("report.pdf.desktop"),
    );
    await sleep(1200);
    const got = launched();
    console.log("launcher saw (tree middle-click .desktop):", JSON.stringify(got));
    assert.ok(!got.includes("report.pdf.desktop"), `tree middle-click handed a .desktop file to the system opener: ${got.trim()}`);
    // The user is told why, and how to open it anyway.
    const toast = await s.exec(`return [...document.querySelectorAll('.toast')].map(t => t.textContent).find(t => t.includes('report.pdf.desktop')) ?? null`);
    assert.match(toast ?? "", /Could not open report\.pdf\.desktop: .*Reveal in file manager/, `toast: ${toast}`);
  },
);
