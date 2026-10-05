// Wikilink autocomplete inserts text that links back to the picked file
// (FINDING-184): [[x.md.md]] for a note named x.md.md, and no entry for a
// file no wikilink can name (C# notes.md, created outside Cairn).
// Run: scripts/e2e-headless.sh e2e/link_autocomplete.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-link-ac-"));
const vault = path.join(tmp, "vault");

function write(rel, content) {
  const p = path.join(vault, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}

write("Start.md", "# Start\n");
write("Dbl/x.md.md", "# Double extension\n");
write("Hash/C# notes.md", "# C sharp\n");
write("Hash/Cello.md", "# Cello\n");

let drv, s;

before(async () => {
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
  });
  s = await Session.create(drv.port, APP, [vault]);
  await s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 3`, { timeout: 15000 });
  await s.click(await s.findWait(`[data-testid=tree-row][data-path="Start.md"]`));
  await s.waitFor(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path === "Start.md"`);
});

after(async () => {
  await s?.close().catch(() => {});
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

const editorText = () => s.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
const options = () => s.exec(`return [...document.querySelectorAll('.cm-tooltip-autocomplete li')].map(li => li.textContent)`);

async function typeAtEnd(text) {
  await s.exec(`
    const v = document.querySelector('.cm-editor').__cairnView;
    v.focus();
    v.dispatch({ selection: { anchor: v.state.doc.length } });
  `);
  await s.keys(Key.enter, text);
  await s.waitFor(`return !!document.querySelector('.cm-tooltip-autocomplete li')`, { message: "completion list" });
}

test("a note named x.md.md is linked as [[x.md.md]], which resolves back to it", async () => {
  await typeAtEnd("[[x.md");
  const first = (await options())[0];
  assert.match(first, /^x\.md/, JSON.stringify(await options()));
  await sleep(150); // CodeMirror ignores Enter right after the popup opens
  await s.keys(Key.enter);
  await s.waitFor(`return document.querySelector('.cm-editor').__cairnView.state.doc.toString().includes("[[x.md.md]]")`, {
    message: "completion applied",
  });
  const marks = await s.exec(`return [...document.querySelectorAll('.cm-wikilink')].map(e => e.className + ' ' + e.textContent)`);
  const why = `marks ${JSON.stringify(marks)}; text ${JSON.stringify(await editorText())}`;
  assert.ok(marks.some((m) => m.includes("x.md.md")), why);
  assert.ok(!marks.some((m) => m.includes("unresolved")), why);
});

test("a file whose name has a # is not offered (no wikilink can name it)", async () => {
  await typeAtEnd("[[C");
  const opts = await options();
  assert.ok(opts.some((o) => o.startsWith("Cello")), JSON.stringify(opts));
  assert.ok(!opts.some((o) => o.startsWith("C# notes")), JSON.stringify(opts));
  await s.keys(Key.escape);
});
