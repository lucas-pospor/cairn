// The Templates core plugin in the real app: turned on (by default), set up in
// Settings and used from the command palette.
//
//   scripts/e2e-headless.sh e2e/core_templates.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, Key, eventually } from "./core_lib.mjs";

const pad = (n) => String(n).padStart(2, "0");
const now = () => new Date();
const today = (d = now()) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

let app;

before(async () => {
  app = await CoreApp.start("core-templates", {
    "Notes/Standup.md": "Intro\n",
    "Templates/Meeting.md": "## {{title}} on {{date}} at {{time}}\n- Attendees:\n",
    "Templates/Work/Sign-off.md": "Signed {{date:YYYY}} ({{date}})\n",
  });
});

after(async () => {
  await app?.stop();
});

const option = (key) => `[data-testid=core-plugin-option][data-id=templates][data-key=${key}]`;

/** Type `value` into an option field and leave it, as a user does. */
async function setOption(key, value) {
  await app.exec(
    `const i = document.querySelector(arguments[0] + ' input'); i.value = arguments[1]; i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new Event('change', { bubbles: true }));`,
    option(key),
    value,
  );
}

const example = (key) => app.exec(`return document.querySelector(arguments[0] + ' [data-testid=core-plugin-example]')?.textContent ?? null`, option(key));

/** Run Insert template on the open note and pick `label`. */
async function insertTemplate(label) {
  await app.runCommand("Templates: Insert template");
  await app.waitFor(`return [...document.querySelectorAll('.choice')].some((b) => b.textContent === ${JSON.stringify(label)})`);
  const labels = await app.exec(`return [...document.querySelectorAll('.choice')].map((b) => b.textContent)`);
  await app.exec(`[...document.querySelectorAll('.choice')].find((b) => b.textContent === arguments[0]).click()`, label);
  await app.waitFor(`return !document.querySelector('.choice')`);
  return labels;
}

async function cursorAtEnd() {
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } });`);
}

test("Templates is on by default, with its folder and formats in Settings", async () => {
  await app.openSettings("core-plugins");
  assert.equal(await app.exec(`return document.querySelector('[data-testid=core-plugin-row][data-id=templates] [data-testid=core-plugin-toggle]').checked`), true);
  const values = await app.exec(`return [...document.querySelectorAll('[data-testid=core-plugin-option][data-id=templates] input')].map((i) => i.value)`);
  assert.deepEqual(values, ["Templates", "YYYY-MM-DD", "HH:mm"]);
  assert.equal(await example("folder"), "2 templates");
  assert.equal(await example("dateFormat"), `Today: ${today()}`);
  assert.match(await example("timeFormat"), /^Now: \d\d:\d\d$/);
  await app.closeSettings();
  // Nothing was written: these are the defaults.
  assert.equal(app.exists(".cairn/settings.json"), false);
});

test("Insert template puts the filled-in template at the cursor, and one undo takes it out", async () => {
  await app.openNote("Notes/Standup.md");
  await cursorAtEnd();
  assert.deepEqual(await insertTemplate("Meeting"), ["Meeting", "Work/Sign-off"]);
  const want = new RegExp(`^Intro\\n## Standup on ${today()} at \\d\\d:\\d\\d\\n- Attendees:\\n$`);
  await eventually(async () => want.test(await app.doc()), { message: "template inserted" });
  await eventually(() => want.test(app.read("Notes/Standup.md")), { message: "template saved" });
  await app.s.keys({ chord: [Key.ctrl, "z"] });
  await eventually(async () => (await app.doc()) === "Intro\n", { message: "one undo removes the template" });
  await eventually(() => app.read("Notes/Standup.md") === "Intro\n", { message: "undo saved" });
});

test("the date format set in Settings is used, and only it is saved", async () => {
  await app.openSettings("core-plugins");
  await setOption("dateFormat", "DD.MM.YYYY");
  const d = now();
  const dotted = `${pad(d.getDate())}.${pad(d.getMonth() + 1)}.${d.getFullYear()}`;
  await eventually(async () => (await example("dateFormat")) === `Today: ${dotted}`, { message: "example follows the format" });
  await eventually(() => app.exists(".cairn/settings.json") && app.settingsFile().corePlugins?.templates?.dateFormat === "DD.MM.YYYY", { message: "format saved" });
  assert.deepEqual(app.settingsFile().corePlugins, { templates: { dateFormat: "DD.MM.YYYY" } });
  await app.closeSettings();

  await app.openNote("Notes/Standup.md");
  await cursorAtEnd();
  await insertTemplate("Work/Sign-off");
  await eventually(async () => (await app.doc()) === `Intro\nSigned ${d.getFullYear()} (${dotted})\n`, { message: "custom format used" });
});

test("a template folder that does not exist: Settings and the command say so, and nothing is inserted", async () => {
  await app.openSettings("core-plugins");
  await setOption("folder", "Missing");
  await eventually(async () => (await example("folder")) === "There is no folder named Missing yet.", { message: "folder example" });
  await app.closeSettings();
  const before = await app.doc();
  await app.runCommand("Templates: Insert template");
  await eventually(async () => (await app.toasts()).some((t) => t.includes("There is no folder named Missing. Create it and add notes to use them as templates.")), {
    message: "toast",
  });
  assert.equal(await app.doc(), before);
  await app.openSettings("core-plugins");
  await setOption("folder", "Templates");
  await app.closeSettings();
});

test("in reading view the command is not in the palette", async () => {
  await app.openNote("Notes/Standup.md");
  assert.deepEqual(await app.paletteNames("Insert template"), ["Templates: Insert template"]);
  await app.s.keys({ chord: [Key.ctrl, "e"] });
  await app.waitFor(`return !!document.querySelector('[data-testid=preview]')`);
  assert.deepEqual(await app.paletteNames("Insert template"), []);
  await app.s.keys({ chord: [Key.ctrl, "e"] });
  await app.waitFor(`return !document.querySelector('[data-testid=preview]')`);
});
