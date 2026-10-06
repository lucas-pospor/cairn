// The Random note core plugin in the real app: off by default, turned on in
// Settings and used from the command palette.
//
//   scripts/e2e-headless.sh e2e/core_random_note.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, eventually } from "./core_lib.mjs";

const NOTES = { "A.md": "a\n", "B.md": "b\n", "C.md": "c\n", "Sub/D.md": "d\n" };
let app;

before(async () => {
  app = await CoreApp.start("core-random", { ...NOTES, "pic.png": "not a note" });
});

after(async () => {
  await app?.stop();
});

const toggle = "[data-testid=core-plugin-row][data-id=random-note] [data-testid=core-plugin-toggle]";

test("Random note is off by default and has no command", async () => {
  await app.openSettings("core-plugins");
  assert.equal(await app.exec(`return document.querySelector('${toggle}').checked`), false);
  await app.closeSettings();
  assert.deepEqual(await app.paletteNames("random note"), []);
});

test("turned on, it opens a note other than the open one each time, and writes nothing", async () => {
  await app.openSettings("core-plugins");
  await app.click(toggle);
  await eventually(() => app.settingsFile().corePlugins?.["random-note"]?.on === true, { message: "switch saved" });
  // No options to show.
  assert.equal(await app.exec(`return document.querySelectorAll('[data-testid=core-plugin-option][data-id=random-note]').length`), 0);
  await app.closeSettings();

  await app.openNote("A.md");
  const seen = new Set();
  let current = "A.md";
  for (let i = 0; i < 8; i++) {
    await app.runCommand("Random note: Open random note");
    const next = await eventually(async () => {
      const t = await app.activeTab();
      return t !== current && t;
    }, { message: `a note other than ${current}` });
    assert.ok(next in NOTES, next);
    seen.add(next);
    current = next;
  }
  assert.ok(seen.size >= 2, [...seen].join());
  for (const [p, text] of Object.entries(NOTES)) assert.equal(app.read(p), text);
});

test("turned off, its command goes", async () => {
  await app.openSettings("core-plugins");
  await app.click(toggle);
  await eventually(() => app.settingsFile().corePlugins["random-note"].on === false, { message: "switch saved" });
  await app.closeSettings();
  assert.deepEqual(await app.paletteNames("random note"), []);
});
