// Core plugins in the real app: the Settings section, and the "corePlugins" key
// in .cairn/settings.json.
//
//   scripts/e2e-headless.sh e2e/core_plugins.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, Key, eventually, sleep } from "./core_lib.mjs";

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
  assert.match(intro, /Plugins are JavaScript files in .cairn\/plugins\/ in this notebook/);
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

test("lists each core plugin with its switch", async () => {
  await app.openSettings("core-plugins");
  const rows = await app.exec(
    `return [...document.querySelectorAll('[data-testid=core-plugin-row]')].map((r) => [r.dataset.id, r.querySelector('b').textContent, r.querySelector('[data-testid=core-plugin-toggle]').checked])`,
  );
  assert.deepEqual(rows, [
    // Off by default: the wrong values in settings.json read as the default.
    ["daily-notes", "Daily notes", true],
    ["random-note", "Random note", false],
    ["templates", "Templates", true],
    ["unique-note", "Unique note creator", false],
  ]);
  await app.closeSettings();
});

test("a plugin turned off leaves the palette and the Hotkeys list, and keeps its hotkey", async () => {
  const row = `[data-testid=hotkey-row][data-command="templates:insert"]`;
  const toggle = `[data-testid=core-plugin-row][data-id=templates] [data-testid=core-plugin-toggle]`;
  const focusEditor = () => app.exec(`document.querySelector('.cm-editor').__cairnView.focus()`);
  await app.openNote("Welcome.md");

  // Bind a hotkey to Insert template.
  await app.openSettings("hotkeys");
  await app.waitFor(`return !!document.querySelector('${row}')`);
  await app.exec(`document.querySelector('${row} [data-testid=hotkey-add]').click()`);
  await app.s.keys({ chord: [Key.ctrl, Key.alt, "j"] });
  await eventually(() => JSON.stringify(app.settingsFile().hotkeys["templates:insert"]) === '["Mod+Alt+J"]', { message: "hotkey saved" });

  // Off: its options and its Hotkeys row go, without closing Settings.
  await app.openSettings("core-plugins");
  assert.ok(await app.exec(`return document.querySelectorAll('[data-testid=core-plugin-option][data-id=templates]').length > 0`));
  await app.click(toggle);
  await eventually(() => app.settingsFile().corePlugins.templates?.on === false, { message: "switch saved" });
  assert.deepEqual(app.settingsFile().corePlugins, { ...CORE, templates: { on: false } });
  assert.equal(await app.exec(`return document.querySelectorAll('[data-testid=core-plugin-option][data-id=templates]').length`), 0);
  await app.openSettings("hotkeys");
  assert.equal(await app.exec(`return !!document.querySelector('${row}')`), false);
  await app.closeSettings();
  assert.deepEqual(await app.paletteNames("Insert template"), []);

  // Its key does nothing now, and stays saved.
  await focusEditor();
  await app.s.keys({ chord: [Key.ctrl, Key.alt, "j"] });
  await sleep(400);
  assert.equal(await app.exec(`return document.querySelectorAll('.choice').length`), 0);
  assert.ok(!(await app.toasts()).some((t) => /template/i.test(t)));
  assert.deepEqual(app.settingsFile().hotkeys["templates:insert"], ["Mod+Alt+J"]);

  // On again: listed and bound again (this vault has no Templates folder, so it says so).
  await app.openSettings("core-plugins");
  await app.click(toggle);
  await eventually(() => app.settingsFile().corePlugins.templates?.on === true, { message: "switch saved" });
  await app.openSettings("hotkeys");
  await app.waitFor(`return document.querySelector('${row} .combo')?.textContent.includes('Ctrl+Alt+J')`);
  await app.closeSettings();
  assert.deepEqual(await app.paletteNames("Insert template"), ["Templates: Insert template"]);
  await focusEditor();
  await app.s.keys({ chord: [Key.ctrl, Key.alt, "j"] });
  await eventually(async () => (await app.toasts()).some((t) => t.includes("There is no folder named Templates.")), { message: "hotkey runs it" });
});
