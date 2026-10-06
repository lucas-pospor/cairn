// Core plugins on the phone: Settings > Core plugins with touch.
//
//   scripts/adv-android-run-all.sh e2e/android/core_plugins.test.mjs
//
// Needs one emulator or device in `adb devices` and the debug APK. Clears the
// app's data. Screenshots go to e2e/.tmp/AN/.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Device, adb, sleep, closeSettings } from "./adv_helpers.mjs";

const d = new Device();

before(async () => {
  await d.fresh();
  await d.createAppVault("Core");
});

after(async () => {
  await d.shot("core-plugins-final.png").catch(() => {});
  d.close();
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

async function openDrawer() {
  if (await d.eval(`document.querySelector('aside.left').classList.contains('hidden')`)) await d.tap("[data-testid=mobile-files]");
  await d.waitFor(`!document.querySelector('aside.left').classList.contains('hidden')`);
  await sleep(300);
}

/** Settings, opened with taps from the Files drawer, on `section`. */
async function openSettings(section) {
  await openDrawer();
  await d.tap("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=settings]')`);
  // On a phone the sections are a row that scrolls sideways.
  await d.eval(`document.querySelector('[data-testid=settings-${section}]').scrollIntoView({ inline: 'center', block: 'nearest' })`);
  await sleep(300);
  await d.tap(`[data-testid=settings-${section}]`);
  await d.waitFor(`document.querySelector('[data-testid=settings-${section}]').getAttribute('aria-current') === 'page'`);
}

test("Settings has a Core plugins section on the phone, just before Plugins", async () => {
  await openSettings("core-plugins");
  const nav = await d.eval(`[...document.querySelectorAll('[data-testid=settings] nav button')].map(b => b.dataset.testid)`);
  assert.equal(nav.indexOf("settings-core-plugins"), nav.indexOf("settings-plugins") - 1, nav.join(" "));
  assert.equal(await d.eval(`document.querySelector('[data-testid=settings] section h3').textContent`), "Core plugins");
  // Nothing in the section sticks out sideways.
  assert.ok(await d.eval(`(() => { const s = document.querySelector('[data-testid=settings] section'); return s.scrollWidth <= s.clientWidth + 1; })()`));
  await d.shot("core-plugins-settings.png");
  await closeSettings(d);
});
