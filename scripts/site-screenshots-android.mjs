// Takes the Android screenshots in docs/images, android-editor.png and
// android-files.png, from the demo notebook in scripts/demo-notebook.mjs, in
// the light theme. Run it on a fresh emulator with the debug APK
// (cd app && npx tauri android build --debug --apk --target x86_64):
//
//   scripts/adv-android-run-all.sh scripts/site-screenshots-android.mjs
//
// It clears the app's data first. Set SHOTS_DIR to write the files somewhere
// else.

import { test } from "node:test";
import path from "node:path";
import { Device, writeAppFile, rescan, screenshot, sleep } from "../e2e/android/adv_helpers.mjs";
import { NOTES } from "./demo-notebook.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.resolve(process.env.SHOTS_DIR ?? path.join(ROOT, "docs/images"));
const row = (p) => `[data-testid=tree-row][data-path="${p}"]`;

const d = new Device();
// The DevTools connection to the web view would keep node running.
test.after(() => d.close());

test("Android screenshots", { timeout: 180000 }, async () => {
  await d.fresh();
  const folder = await d.createAppVault("Notes");
  for (const [file, text] of Object.entries(NOTES)) writeAppFile(`${folder}/${file}`, text);
  await rescan(d);

  // The note, from the Files drawer.
  await d.click("[data-testid=mobile-files]");
  await d.waitFor(`!!document.querySelector('${row("Garden")}')`, 15000);
  await d.click(row("Garden"));
  await d.waitFor(`!!document.querySelector('${row("Garden/Tomatoes.md")}')`);
  await d.click(row("Garden/Tomatoes.md"));
  await d.waitFor(`!!document.querySelector('[data-testid=tab][aria-selected=true][data-path="Garden/Tomatoes.md"]')`, 15000);
  // No focus in the editor, so no keyboard and every line rendered.
  await d.eval(`document.activeElement?.blur()`);
  await sleep(1500);
  await screenshot(path.join(OUT, "android-editor.png"));

  // The Files drawer, with two folders open.
  await d.click("[data-testid=mobile-files]");
  await d.waitFor(`!!document.querySelector('${row("Kitchen")}')`);
  if ((await d.eval(`document.querySelector('${row("Kitchen")}').getAttribute('aria-expanded')`)) !== "true") await d.click(row("Kitchen"));
  await d.eval(`document.activeElement?.blur()`);
  await sleep(1500);
  await screenshot(path.join(OUT, "android-files.png"));
});
