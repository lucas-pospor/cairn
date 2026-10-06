// Core plugins on the phone: Settings > Core plugins with touch.
//
//   scripts/adv-android-run-all.sh e2e/android/core_plugins.test.mjs
//
// Needs one emulator or device in `adb devices` and the debug APK. Clears the
// app's data. Screenshots go to e2e/.tmp/AN/.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Device, adb, sleep, closeSettings, eventually, writeAppFile, readAppFile, rescan, runAs, APP_DATA } from "./adv_helpers.mjs";

const d = new Device();
const VAULT = `${APP_DATA}/vaults/Core`;

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

/**
 * Tap `selector` once it has stopped moving. The on-screen keyboard opens and closes
 * as the focus moves (into the editor, into a dialog), and the layout follows it.
 */
async function tap(selector) {
  let last = null;
  for (let i = 0; i < 40; i++) {
    const r = await d.rect(selector);
    if (r && last && r.cx === last.cx && r.cy === last.cy && r.ih === last.ih) break;
    last = r;
    await sleep(250);
  }
  await d.tap(selector);
}

async function openDrawer() {
  if (await d.eval(`document.querySelector('aside.left').classList.contains('hidden')`)) await tap("[data-testid=mobile-files]");
  await d.waitFor(`!document.querySelector('aside.left').classList.contains('hidden')`);
  await sleep(300);
}

/** Settings, opened with taps from the Files drawer, on `section`. */
async function openSettings(section) {
  await openDrawer();
  await tap("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=settings]')`);
  // On a phone the sections are a row that scrolls sideways.
  await d.eval(`document.querySelector('[data-testid=settings-${section}]').scrollIntoView({ inline: 'center', block: 'nearest' })`);
  await sleep(300);
  await tap(`[data-testid=settings-${section}]`);
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

const doc = () => d.eval(`document.querySelector('.cm-editor').__cairnView.state.doc.toString()`);
const settingsOnDevice = () => JSON.parse(readAppFile(`${VAULT}/.cairn/settings.json`) || "{}");

/** Tap the button of the open choice dialog whose text is `label`. */
async function tapChoice(label) {
  await d.waitFor(`[...document.querySelectorAll('.choice')].some((b) => b.textContent === ${JSON.stringify(label)})`);
  await d.eval(`[...document.querySelectorAll('.choice')].find((b) => b.textContent === ${JSON.stringify(label)}).dataset.pick = 'yes'`);
  await tap(`.choice[data-pick=yes]`);
}

test("the Templates button in the formatting toolbar inserts a template", async () => {
  writeAppFile(`${VAULT}/Templates/Greeting.md`, "Hello from {{title}}\n");
  writeAppFile(`${VAULT}/Visit.md`, "Start\n");
  await rescan(d);
  await d.openNote("Visit.md", { contains: "Start" });
  await d.eval(`(() => { const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length } }); })()`);
  await d.waitFor(`!!document.querySelector('[data-testid=toolbar-template]')`);
  await d.eval(`document.querySelector('[data-testid=toolbar-template]').scrollIntoView({ inline: 'center', block: 'nearest' })`);
  await sleep(300);
  await tap("[data-testid=toolbar-template]");
  await tapChoice("Greeting");
  await eventually(async () => (await doc()) === "Start\nHello from Visit\n", { message: "template inserted" });
  await eventually(() => readAppFile(`${VAULT}/Visit.md`) === "Start\nHello from Visit\n", { message: "template saved" });
  await d.shot("core-plugins-template.png");
});

test("turning Templates off with a tap hides its toolbar button and options", async () => {
  const toggle = "[data-testid=core-plugin-row][data-id=templates] [data-testid=core-plugin-toggle]";
  await openSettings("core-plugins");
  assert.equal(await d.eval(`document.querySelector('${toggle}').checked`), true);
  assert.ok(await d.eval(`document.querySelectorAll('[data-testid=core-plugin-option][data-id=templates]').length === 3`));
  await tap(toggle);
  await d.waitFor(`!document.querySelector('${toggle}').checked && !document.querySelector('[data-testid=core-plugin-option][data-id=templates]')`);
  await eventually(() => settingsOnDevice().corePlugins?.templates?.on === false, { message: "switch saved" });
  await closeSettings(d);
  await d.waitFor(`!document.querySelector('[data-testid=toolbar-template]') && !!document.querySelector('.toolbar')`);
  // And back on.
  await openSettings("core-plugins");
  await tap(toggle);
  await eventually(() => settingsOnDevice().corePlugins?.templates?.on === true, { message: "switch saved" });
  await closeSettings(d);
  await d.waitFor(`!!document.querySelector('[data-testid=toolbar-template]')`);
});

test("the Today button in the Files drawer creates today's note, then opens the same one", async () => {
  // The phone's date, which may differ from this computer's.
  const today = await d.eval(`(() => { const d = new Date(), p = (n) => String(n).padStart(2, "0"); return d.getFullYear() + "-" + p(d.getMonth() + 1) + "-" + p(d.getDate()); })()`);
  const file = `${VAULT}/${today}.md`;
  await openDrawer();
  await d.waitFor(`document.querySelector('[data-testid=open-today]')?.nextElementSibling?.dataset.testid === 'open-graph'`);
  await tap("[data-testid=open-today]");
  await d.waitFor(`document.querySelector('.mobile-title')?.textContent.trim() === ${JSON.stringify(today)}`);
  // The drawer closed over the note.
  await d.waitFor(`document.querySelector('aside.left').classList.contains('hidden')`);
  await eventually(() => runAs(`ls ${VAULT}`).split(/\s+/).includes(`${today}.md`), { message: "note on the phone" });
  await d.append("Phone entry.");
  await eventually(() => readAppFile(file) === "Phone entry.", { message: "saved" });

  await d.openNote("Visit.md", { contains: "Start" });
  await openDrawer();
  await tap("[data-testid=open-today]");
  await d.waitFor(`document.querySelector('.mobile-title')?.textContent.trim() === ${JSON.stringify(today)}`);
  assert.equal(await doc(), "Phone entry.");
  assert.equal(readAppFile(file), "Phone entry.");
  await d.shot("core-plugins-today.png");
});
