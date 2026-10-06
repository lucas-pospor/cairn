// Core plugins in the real app: the Settings section, and the "corePlugins" key
// in .cairn/settings.json.
//
//   scripts/e2e-headless.sh e2e/core_plugins.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, eventually } from "./core_lib.mjs";

// What a later version may have written: a plugin this one does not know, values of
// the wrong type, and hotkeys of core plugin commands.
const CORE = {
  "some-later-plugin": { on: true, level: 3 },
  "random-note": "on",
  "unique-note": { on: "yes", folder: 7 },
};
const HOTKEYS = { "daily-notes:today": ["Mod+Alt+D"], "some-later-plugin:go": ["Mod+Alt+L"] };

let app;

before(async () => {
  app = await CoreApp.start("core-plugins", {
    "Welcome.md": "# Welcome\n",
    "Ideas.md": "Some ideas.\n",
    ".cairn/settings.json": JSON.stringify({ theme: "light", hotkeys: HOTKEYS, corePlugins: CORE }, null, 2),
    ".cairn/plugins/hello.js": "// @name Hello\n// @description Says hello.\ncairn.commands.register('hi', 'Say hi', () => cairn.ui.toast('hi'));\n",
  });
});

after(async () => {
  await app?.stop();
});

test("Settings has a Core plugins section just above Plugins", async () => {
  await app.openSettings("core-plugins");
  const nav = await app.exec(`return [...document.querySelectorAll('[data-testid=settings] nav button')].map(b => b.dataset.testid)`);
  assert.deepEqual(nav, [
    "settings-appearance",
    "settings-editor",
    "settings-files",
    "settings-sync",
    "settings-core-plugins",
    "settings-plugins",
    "settings-hotkeys",
  ]);
  assert.equal(await app.exec(`return document.querySelector('[data-testid=settings] section h3').textContent`), "Core plugins");
  const intro = await app.exec(`return document.querySelector('[data-testid=settings] section p.muted').textContent`);
  assert.match(intro, /need no approval/);
});

test("the Plugins section is as before", async () => {
  await app.openSettings("plugins");
  assert.equal(await app.exec(`return document.querySelector('[data-testid=settings] section h3').textContent`), "Plugins");
  const intro = await app.exec(`return document.querySelector('[data-testid=settings] section p.muted').textContent`);
  assert.match(intro, /Plugins are JavaScript files in .cairn\/plugins\/ in this vault/);
  await app.waitFor(`return document.querySelector('[data-testid=plugin-row]')?.dataset.file === 'hello.js'`);
  assert.equal(await app.exec(`return document.querySelector('[data-testid=plugin-row] [data-testid=plugin-file]').textContent`), "hello.js");
  assert.equal(await app.exec(`return document.querySelector('[data-testid=plugin-row] [data-testid=plugin-toggle]').checked`), false);
  assert.ok(await app.exec(`return !!document.querySelector('[data-testid=plugins-reload]')`));
});

test("saving another setting keeps corePlugins and the hotkeys of core plugin commands", async () => {
  await app.openSettings("appearance");
  await app.exec(`const s = document.querySelector('[data-testid=theme-select]'); s.value = 'dark'; s.dispatchEvent(new Event('change', { bubbles: true }));`);
  await eventually(() => app.settingsFile().theme === "dark", { message: "theme saved" });
  const saved = app.settingsFile();
  assert.deepEqual(saved.corePlugins, CORE);
  assert.deepEqual(saved.hotkeys, HOTKEYS);
  await app.closeSettings();
});
