// Reproduction for FINDING-203: repeated icon buttons (tab Close, hotkey
// "+" / "×", Welcome "Remove from list") have identical names that do not
// say which tab / command / vault they act on.
//
// Unlike the other reproduction (adv_a11y_semantics.test.mjs), this one asks
// the browser engine itself for the accessible name (WebDriver "Get Computed
// Label", answered by WebKit's accessibility tree) when the driver supports
// it, and falls back to the accname approximation in adv_a11y_lib.mjs
// otherwise. It also covers the Welcome screen's recent-vault list with two
// entries.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_15.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { AxApp, K, eventually, sleep } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax-v15-");
const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop("verify-ax15-final.png");
});

const log = (label, v) => console.log(`${label}:\n${typeof v === "string" ? v : JSON.stringify(v, null, 1)}`);

let engineLabels = null; // null = unknown, true = supported, false = unsupported

/** Accessible names of all elements matching css: engine-computed when possible, approximation otherwise. */
async function names(css) {
  const approx = await app.exec(`return [...document.querySelectorAll(arguments[0])].map(b => __ax.name(b))`, css);
  let engine = null;
  if (engineLabels !== false) {
    try {
      const els = await app.s.cmd("POST", "/elements", { using: "css selector", value: css });
      engine = [];
      for (const e of els) engine.push(await app.s.cmd("GET", `/element/${e[ELEMENT]}/computedlabel`));
      engineLabels = true;
    } catch (e) {
      engineLabels = false;
      console.log(`computedlabel unsupported: ${e.message.slice(0, 200)}`);
      engine = null;
    }
  }
  return { engine, approx };
}

const counts = (arr) => {
  const c = {};
  for (const n of arr) c[n] = (c[n] || 0) + 1;
  return c;
};

test("FINDING-203: repeated per-row buttons have identical, context-free accessible names", async () => {
  await app.reset();
  await app.openNote("welcome", "Welcome.md");
  await app.chord(K.ctrl, "o");
  await app.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.keys("ideas");
  await app.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Ideas')`);
  await app.chord(K.ctrl, K.enter);
  await eventually(async () => (await app.tabs()).length === 2);

  const tabClose = await names("[data-testid=tab] button.close");
  const tabRole = await app.exec(`return [...document.querySelectorAll('[data-testid=tab] button.close')].map(b => ({ parentRole: b.parentElement.getAttribute('role'), title: b.title, ariaLabel: b.getAttribute('aria-label') }))`);

  await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings]')`);
  await app.exec(`document.querySelector('[data-testid=settings-hotkeys]').click(); return 1`);
  await sleep(300);
  const hkAdd = await names("[data-testid=hotkey-row] button[data-testid=hotkey-add]");
  const hkRemove = await names("[data-testid=hotkey-row] .combo button");
  const hkTitles = await app.exec(`return { add: document.querySelector('[data-testid=hotkey-add]')?.title, remove: document.querySelector('[data-testid=hotkey-row] .combo button')?.title }`);
  await app.keys(K.esc);
  await sleep(200);

  // Welcome screen with two recent vaults: open a second vault through the
  // typed-path field, then "Switch vault" again.
  const second = path.join(app.tmp, "second-vault");
  fs.mkdirSync(second, { recursive: true });
  fs.writeFileSync(path.join(second, "Note.md"), "# Note\n");
  await app.exec(`document.querySelector('.statusbar button.vault, button.vault[title="Switch vault"]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=vault-path]')`, { message: "welcome screen" });
  await app.exec(`
    const i = document.querySelector('[data-testid=vault-path]');
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    set.call(i, arguments[0]); i.dispatchEvent(new Event('input', { bubbles: true }));
    return 1`, second);
  await sleep(100);
  await app.exec(`document.querySelector('[data-testid=vault-open]').click(); return 1`);
  await app.s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 1 && !document.querySelector('[data-testid=vault-path]')`, { timeout: 15000, message: "second vault open" });
  await app.exec(`document.querySelector('button.vault[title="Switch vault"]').click(); return 1`);
  await app.s.waitFor(`return document.querySelectorAll('.recent li').length >= 2`, { timeout: 10000, message: "two recent vaults" });
  const recentRemove = await names(".recent li button.icon-btn");
  const recentOpen = await names(".recent li button.recent-open");

  const r = { engineLabels, tabClose, tabRole, hkAdd: counts(hkAdd.engine ?? hkAdd.approx), hkAddApprox: counts(hkAdd.approx), hkRemove: counts(hkRemove.engine ?? hkRemove.approx), hkRemoveApprox: counts(hkRemove.approx), hkTitles, recentRemove, recentOpen };
  log("result", r);

  const dup = [];
  const pick = (x) => x.engine ?? x.approx;
  for (const [label, x] of [["tab close", tabClose], ["hotkey add", hkAdd], ["hotkey remove", hkRemove], ["recent remove", recentRemove]]) {
    const n = pick(x);
    if (n.length > 1 && new Set(n).size < n.length) dup.push(`${n.length} ${label} buttons, distinct names: ${JSON.stringify([...new Set(n)])}`);
  }
  assert.deepEqual(dup, []);
});
