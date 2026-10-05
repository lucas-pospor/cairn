// Reproduction for FINDING-223 (debounced settings save, no flush).
//
// Variant: "Switch vault" (status bar) within 300 ms of a settings change.
// app.closeVault() calls settings.reset() (value = DEFAULT_SETTINGS) but does
// not clear the pending save timer and does not close the vault in the
// backend, so when the timer fires it writes the *defaults* into the vault
// that was just left: earlier, already-saved settings are overwritten too,
// not only the last change.
//
// Run:  scripts/e2e-headless.sh e2e/adv_verify_ax_47.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { AxApp, sleep, eventually } from "./adv_a11y_lib.mjs";

async function changeThenSwitchVault(waitMs) {
  const app = new AxApp("cairn-ax47v-");
  let saved;
  try {
    await app.start();
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=theme-select]')`);
    // A first change that is saved normally (font size 20).
    await app.exec(`const r = document.querySelector('[data-testid=settings] input[type=range]'); r.value = 20; r.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
    await eventually(() => {
      try {
        return JSON.parse(app.read(".cairn/settings.json")).fontSize === 20;
      } catch {
        return false;
      }
    }, { message: "font size 20 saved" });
    // Second change (theme dark), then Switch vault.
    await app.exec(
      `const s = document.querySelector('[data-testid=theme-select]'); s.value = 'dark'; s.dispatchEvent(new Event('change', { bubbles: true }));
       if (arguments[0]) return 1;
       document.querySelector('.status .vault').click(); return 1`,
      waitMs,
    );
    if (waitMs) {
      await sleep(waitMs);
      await app.exec(`document.querySelector('.status .vault').click(); return 1`);
    }
    await sleep(1500);
    saved = JSON.parse(app.read(".cairn/settings.json"));
  } finally {
    await app.stop();
  }
  return saved;
}

test("control: Switch vault 800 ms after a settings change keeps both changes", async () => {
  const s = await changeThenSwitchVault(800);
  assert.equal(s.fontSize, 20);
  assert.equal(s.theme, "dark");
});

test("FINDING-223: Switch vault right after a settings change overwrites the left vault's settings.json with defaults", async () => {
  const s = await changeThenSwitchVault(0);
  assert.deepEqual({ fontSize: s.fontSize, theme: s.theme }, { fontSize: 20, theme: "dark" }, `settings.json after Switch vault: ${JSON.stringify(s)}`);
});
