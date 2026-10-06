// Settings are written to .cairn/settings.json 300 ms after the last change
// (settings.svelte.ts update(), debounced) and flushed when the window closes,
// so closing inside that window keeps the change. Three app launches: a control
// that waits before closing, one that closes right away, and one that reloads.
//
// Run:  scripts/e2e-headless.sh e2e/adv_a11y_settings_close.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { AxApp, sleep } from "./adv_a11y_lib.mjs";

async function changeThemeThenClose(waitMs) {
  const app = new AxApp("cairn-ax-close-");
  let saved;
  try {
    await app.start();
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=theme-select]')`);
    await app.exec(`const s = document.querySelector('[data-testid=theme-select]'); s.value = 'dark'; s.dispatchEvent(new Event('change', { bubbles: true })); return 1`);
    if (waitMs) await sleep(waitMs);
    // Same as closing the window from the title bar: a close request the app
    // sees first. (WebDriver's Close Window drops the web view without one.)
    await app.exec(`window.__TAURI_INTERNALS__.invoke("plugin:window|close", { label: "main" }); return 1`).catch(() => {});
    await sleep(1500);
    assert.ok(await app.exec("return 1").then(() => false, () => true), "the window closed");
    saved = app.exists(".cairn/settings.json") ? JSON.parse(app.read(".cairn/settings.json")).theme : null;
  } finally {
    await app.stop();
  }
  return saved;
}

test("held up (control): a theme change is saved when the window is closed 800 ms later", async () => {
  assert.equal(await changeThemeThenClose(800), "dark");
});

test("FINDING-223: a settings change made less than 300 ms before the window closes is saved (the debounced save is flushed on close)", async () => {
  const saved = await changeThemeThenClose(0);
  assert.equal(saved, "dark", `Settings > Theme = Dark, window closed at once: settings.json theme is ${JSON.stringify(saved)}`);
});

test("a settings change made right before the page reloads is saved", async () => {
  const app = new AxApp("cairn-ax-reload-");
  try {
    await app.start();
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=theme-select]')`);
    await app.exec(`const s = document.querySelector('[data-testid=theme-select]'); s.value = 'dark'; s.dispatchEvent(new Event('change', { bubbles: true })); return 1`);
    await app.s.cmd("POST", "/refresh", {});
    await sleep(1500);
    const saved = app.exists(".cairn/settings.json") ? JSON.parse(app.read(".cairn/settings.json")).theme : null;
    assert.equal(saved, "dark", `Settings > Theme = Dark, page reloaded at once: settings.json theme is ${JSON.stringify(saved)}`);
  } finally {
    await app.stop();
  }
});
