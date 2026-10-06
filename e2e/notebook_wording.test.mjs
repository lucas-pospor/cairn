// The Welcome screen, the status bar and every section of Settings call the
// folder of notes a notebook, and nothing a user can read there says vault.
// app/src/lib/wording.test.ts checks the strings in the source; this checks
// what the app shows.
//
// Run:  scripts/e2e-headless.sh e2e/notebook_wording.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

const APP = path.resolve(import.meta.dirname, "../target/debug/cairn");
const OLD_WORD = /\bvaults?\b/i;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-notebook-wording-"));
const notes = path.join(tmp, "Notes");
let drv;
let s;

/** The text a user can read in `selector`: what it shows, and its titles, accessible names, placeholders and alt texts. */
function readable(selector) {
  return s.exec(
    `const root = document.querySelector(arguments[0]);
     const names = [root, ...root.querySelectorAll('*')].flatMap((e) => ['title', 'aria-label', 'placeholder', 'alt'].map((a) => e.getAttribute(a)).filter(Boolean));
     return [root.innerText, ...names].join('\\n');`,
    selector,
  );
}

before(async () => {
  fs.mkdirSync(notes);
  fs.writeFileSync(path.join(notes, "Hello.md"), "# Hello\n");
  drv = await startDriver(4444, {
    XDG_CONFIG_HOME: path.join(tmp, "config"),
    XDG_DATA_HOME: path.join(tmp, "data"),
    XDG_CACHE_HOME: path.join(tmp, "cache"),
    CAIRN_VAULT: "",
  });
  // No notebook on the command line, in CAIRN_VAULT or in the recent list: the Welcome screen.
  s = await Session.create(drv.port, APP, []);
  await s.waitFor(`return !!document.querySelector('[data-testid=vault-path]')`, { timeout: 15000, message: "the Welcome screen" });
});

after(async () => {
  await s?.close();
  drv?.proc.kill();
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
});

test("the Welcome screen says notebook", async () => {
  const buttons = await s.exec(`return [...document.querySelectorAll('main.welcome button')].map((b) => b.textContent.trim())`);
  assert.ok(buttons.includes("Open folder as notebook"), JSON.stringify(buttons));
  assert.ok(buttons.includes("Create new notebook"), JSON.stringify(buttons));
  const text = await readable("main.welcome");
  assert.doesNotMatch(text, OLD_WORD);
});

test("the status bar and every section of Settings say notebook", async () => {
  await s.exec(
    `const i = document.querySelector('[data-testid=vault-path]');
     i.value = arguments[0];
     i.dispatchEvent(new Event('input', { bubbles: true }));
     document.querySelector('[data-testid=vault-open]').click();`,
    notes,
  );
  await s.waitFor(`return !!document.querySelector('[data-testid=tree-row]')`, { timeout: 15000, message: "the notebook open" });

  const status = await s.exec(`const b = document.querySelector('footer.status button.vault'); return { title: b.title, name: b.getAttribute('aria-label') }`);
  assert.deepEqual(status, { title: "Switch notebook", name: "Switch notebook (current: Notes)" });
  assert.doesNotMatch(await readable("footer.status"), OLD_WORD);

  await s.exec(`document.querySelector('[data-testid=open-settings]').click()`);
  await s.waitFor(`return !!document.querySelector('[data-testid=settings]')`, { message: "Settings open" });
  const shown = {};
  for (const id of ["appearance", "editor", "files", "sync", "core-plugins", "plugins", "hotkeys"]) {
    await s.exec(`document.querySelector('[data-testid=settings-${id}]').click()`);
    await s.waitFor(`return document.querySelector('[data-testid=settings-${id}]').getAttribute('aria-current') === 'page'`, { message: `Settings, then ${id}` });
    await sleep(100);
    shown[id] = await readable("[data-testid=settings]");
    assert.doesNotMatch(shown[id], OLD_WORD, `Settings, then ${id}`);
  }
  const has = (id, text) => assert.ok(shown[id].includes(text), `Settings, then ${id}, does not say ${JSON.stringify(text)}:\n${shown[id]}`);
  has("appearance", "Stored in .cairn/settings.json inside this notebook.");
  has("files", "Where pasted and dropped files are saved, relative to the notebook folder.");
  has("sync", "Sync keeps this notebook in step with your other devices");
  has("sync", "Notebook name on the server");
  has("sync", "Use the same notebook name and passphrase on every device.");
  has("core-plugins", "their settings are saved in this notebook.");
  has("core-plugins", "Where new daily notes go. Empty means the notebook folder.");
  has("core-plugins", "Notebook folder");
  has("plugins", "Plugins are JavaScript files in .cairn/plugins/ in this notebook.");
  has("hotkeys", "Switch notebook");
});
