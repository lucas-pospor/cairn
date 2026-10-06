// The Unique note creator core plugin in the real app: off by default, turned on
// and set up in Settings, and used from the command palette.
//
//   scripts/e2e-headless.sh e2e/core_unique_note.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, eventually } from "./core_lib.mjs";

const pad = (n) => String(n).padStart(2, "0");
const day = (d = new Date()) => `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}`;

let app;

before(async () => {
  app = await CoreApp.start("core-unique", {
    "Welcome.md": "# Welcome\n",
    "Templates/Card.md": "# {{title}}\n\nSource:\n",
    [`Zettel/${day()}.md`]: "Already here.\n",
  });
});

after(async () => {
  await app?.stop();
});

const row = "[data-testid=core-plugin-row][data-id=unique-note]";
const option = (key) => `[data-testid=core-plugin-option][data-id=unique-note][data-key=${key}]`;

async function setOption(key, value) {
  await app.exec(
    `const i = document.querySelector(arguments[0] + ' input'); i.value = arguments[1]; i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new Event('change', { bubbles: true }));`,
    option(key),
    value,
  );
}

const tabs = () => app.exec(`return [...document.querySelectorAll('[data-testid=tab]')].map((t) => t.dataset.path)`);

test("Unique note creator is off by default: no options and no command", async () => {
  await app.openSettings("core-plugins");
  assert.equal(await app.exec(`return document.querySelector('${row} [data-testid=core-plugin-toggle]').checked`), false);
  assert.equal(await app.exec(`return document.querySelectorAll('[data-testid=core-plugin-option][data-id=unique-note]').length`), 0);
  await app.closeSettings();
  assert.deepEqual(await app.paletteNames("Create unique note"), []);
});

test("turned on, it shows its options with an example name", async () => {
  await app.openSettings("core-plugins");
  await app.click(`${row} [data-testid=core-plugin-toggle]`);
  await app.waitFor(`return document.querySelectorAll('[data-testid=core-plugin-option][data-id=unique-note]').length === 3`);
  const fields = await app.exec(`return [...document.querySelectorAll('[data-testid=core-plugin-option][data-id=unique-note] input')].map((i) => i.value)`);
  assert.deepEqual(fields, ["", "YYYYMMDDHHmm", ""]);
  const example = await app.exec(`return document.querySelector('${option("format")} [data-testid=core-plugin-example]').textContent`);
  assert.match(example, new RegExp(`^A note made now: ${day()}\\d{4}\\.md$`));
  await eventually(() => app.settingsFile().corePlugins?.["unique-note"]?.on === true, { message: "switch saved" });
  await app.closeSettings();
});

test("each new note gets a name of its own, from the template, in a new tab; the note already there is left alone", async () => {
  await app.openSettings("core-plugins");
  await setOption("folder", "Zettel");
  // One name a day, so that the names clash.
  await setOption("format", "YYYYMMDD");
  await setOption("template", "Templates/Card");
  await app.closeSettings();
  await app.openNote("Welcome.md");
  const d = day();
  for (const n of [1, 2]) {
    await app.runCommand("Unique note creator: Create unique note");
    await eventually(async () => (await app.activeTab()) === `Zettel/${d} ${n}.md`, { message: `note ${n} open` });
  }
  assert.deepEqual(await tabs(), ["Welcome.md", `Zettel/${d} 1.md`, `Zettel/${d} 2.md`]);
  assert.equal(app.read(`Zettel/${d} 1.md`), `# ${d} 1\n\nSource:\n`);
  assert.equal(app.read(`Zettel/${d} 2.md`), `# ${d} 2\n\nSource:\n`);
  assert.equal(app.read(`Zettel/${d}.md`), "Already here.\n");
  assert.deepEqual(app.files("Zettel"), [`Zettel/${d} 1.md`, `Zettel/${d} 2.md`, `Zettel/${d}.md`]);
});

test("turned off again, its command goes and its options are kept", async () => {
  await app.openSettings("core-plugins");
  await app.click(`${row} [data-testid=core-plugin-toggle]`);
  await eventually(() => app.settingsFile().corePlugins["unique-note"].on === false, { message: "switch saved" });
  assert.deepEqual(app.settingsFile().corePlugins["unique-note"], { on: false, folder: "Zettel", format: "YYYYMMDD", template: "Templates/Card" });
  await app.closeSettings();
  assert.deepEqual(await app.paletteNames("Create unique note"), []);
});
