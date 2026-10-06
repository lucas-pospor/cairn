// The Daily notes core plugin in the real app: on by default, its button next to
// Graph view, its options in Settings, and never writing over a note.
//
//   scripts/e2e-headless.sh e2e/core_daily_notes.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, eventually, sleep } from "./core_lib.mjs";

const pad = (n) => String(n).padStart(2, "0");
const today = (d = new Date()) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

let app;

before(async () => {
  app = await CoreApp.start("core-daily", {
    "Welcome.md": "# Welcome\n",
    "Templates/Day.md": "# {{title}}\n\n{{date:dddd}}\n",
    // Today's note in Diary is already there.
    [`Diary/${today()}.md`]: "Written earlier.\n",
  });
});

after(async () => {
  await app?.stop();
});

const option = (key) => `[data-testid=core-plugin-option][data-id=daily-notes][data-key=${key}]`;

async function setOption(key, value) {
  await app.exec(
    `const i = document.querySelector(arguments[0] + ' input'); i.value = arguments[1]; i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new Event('change', { bubbles: true }));`,
    option(key),
    value,
  );
}

const note = (key, kind) =>
  app.exec(`return document.querySelector(arguments[0] + ' [data-testid=core-plugin-' + arguments[1] + ']')?.textContent ?? null`, option(key), kind);

async function clickToday() {
  await app.click("[data-testid=open-today]");
}

test("Daily notes is on by default: its button is next to Graph view and its options are in Settings", async () => {
  assert.equal(await app.exec(`return document.querySelector('[data-testid=open-today]')?.nextElementSibling?.dataset.testid`), "open-graph");
  assert.equal(await app.exec(`return document.querySelector('[data-testid=open-today]').title`), "Open today's note");
  await app.openSettings("core-plugins");
  assert.equal(await app.exec(`return document.querySelector('[data-testid=core-plugin-row][data-id=daily-notes] [data-testid=core-plugin-toggle]').checked`), true);
  const fields = await app.exec(`return [...document.querySelectorAll('[data-testid=core-plugin-option][data-id=daily-notes] input')].map((i) => [i.value, i.placeholder])`);
  assert.deepEqual(fields, [["", "Vault root"], ["YYYY-MM-DD", "YYYY-MM-DD"], ["", "None"]]);
  assert.equal(await note("format", "example"), `Today's note: ${today()}.md`);
  await app.closeSettings();
});

test("the button creates today's note in the vault root and opens it; again, it opens the same note", async () => {
  const path = `${today()}.md`;
  await clickToday();
  await eventually(async () => (await app.activeTab()) === path, { message: "today's note open" });
  assert.equal(app.read(path), "");
  // Write in it, then ask for today's note again: the same note, as it was.
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ changes: { from: 0, insert: 'Morning.' }, userEvent: 'input.type' });`);
  await eventually(() => app.read(path) === "Morning.", { message: "saved" });
  await app.openNote("Welcome.md");
  await clickToday();
  await eventually(async () => (await app.activeTab()) === path, { message: "today's note open again" });
  assert.equal(await app.doc(), "Morning.");
  assert.equal(app.read(path), "Morning.");
  assert.deepEqual(app.files().filter((f) => f.startsWith(today())), [path]);
});

test("with a folder, a format and a template set, today's note starts from the template", async () => {
  await app.openSettings("core-plugins");
  await setOption("folder", "Journal");
  await setOption("format", "YYYY/MM/YYYY-MM-DD");
  await setOption("template", "Templates/Day");
  const d = new Date();
  const path = `Journal/${d.getFullYear()}/${pad(d.getMonth() + 1)}/${today(d)}.md`;
  await eventually(async () => (await note("format", "example")) === `Today's note: ${path}`, { message: "example follows the folder and format" });
  assert.equal(await note("template", "problem"), null);
  await eventually(() => app.settingsFile().corePlugins?.["daily-notes"]?.template === "Templates/Day", { message: "saved" });
  assert.deepEqual(app.settingsFile().corePlugins, { "daily-notes": { folder: "Journal", format: "YYYY/MM/YYYY-MM-DD", template: "Templates/Day" } });
  await app.closeSettings();
  await clickToday();
  await eventually(async () => (await app.activeTab()) === path, { message: "note in subfolders open" });
  assert.equal(app.read(path), `# ${today(d)}\n\n${DAYS[d.getDay()]}\n`);
});

test("today's note that is already there opens as it is, even with a template set", async () => {
  await app.openSettings("core-plugins");
  await setOption("folder", "Diary");
  await setOption("format", "YYYY-MM-DD");
  await app.closeSettings();
  const path = `Diary/${today()}.md`;
  await clickToday();
  await eventually(async () => (await app.activeTab()) === path, { message: "existing note open" });
  assert.equal(await app.doc(), "Written earlier.\n");
  await sleep(800);
  assert.equal(app.read(path), "Written earlier.\n");
});

test("a format that cannot be a file name: Settings says why, and nothing is created", async () => {
  await app.openSettings("core-plugins");
  await setOption("format", "YYYY-MM-DD HH:mm");
  await eventually(async () => (await note("format", "problem"))?.includes("contains :, which names cannot contain."), { message: "problem shown" });
  await app.closeSettings();
  const before = app.files();
  await clickToday();
  await eventually(async () => (await app.toasts()).some((t) => t.includes("Change it in Settings > Core plugins.")), { message: "error toast" });
  assert.deepEqual(app.files(), before);
  await app.openSettings("core-plugins");
  await setOption("format", "YYYY-MM-DD");
  await app.closeSettings();
});

test("turned off, the button and the command go", async () => {
  await app.openSettings("core-plugins");
  await app.click("[data-testid=core-plugin-row][data-id=daily-notes] [data-testid=core-plugin-toggle]");
  await eventually(() => app.settingsFile().corePlugins["daily-notes"].on === false, { message: "switch saved" });
  await app.closeSettings();
  assert.equal(await app.exec(`return !!document.querySelector('[data-testid=open-today]')`), false);
  assert.deepEqual(await app.paletteNames("today's note"), []);
  await app.openSettings("core-plugins");
  await app.click("[data-testid=core-plugin-row][data-id=daily-notes] [data-testid=core-plugin-toggle]");
  await app.closeSettings();
  await app.waitFor(`return !!document.querySelector('[data-testid=open-today]')`);
  assert.deepEqual(await app.paletteNames("today's note"), ["Daily notes: Open today's note"]);
});
