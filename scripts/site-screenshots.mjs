// Takes the desktop screenshots in docs/images, which the README, the manual
// and the website use, from the demo notebook in scripts/demo-notebook.mjs.
// Run it on the headless display after building the app
// (cd app && npm run e2e:build):
//
//   scripts/e2e-headless.sh scripts/site-screenshots.mjs
//
// It writes editor.png, search.png and appearance.png in Limestone, and
// editor-dark.png and graph.png in Slate. Set SHOTS_DIR to write them
// somewhere else.

import { test } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session, Key } from "../e2e/webdriver.mjs";
import { pickThemeMode } from "../e2e/theme_mode.mjs";
import { NOTES } from "./demo-notebook.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.CAIRN_BIN ?? path.join(ROOT, "target/debug/cairn");
const OUT = path.resolve(process.env.SHOTS_DIR ?? path.join(ROOT, "docs/images"));

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-shots-"));
const notebook = path.join(tmp, "Notes");
for (const [file, text] of Object.entries(NOTES)) {
  fs.mkdirSync(path.dirname(path.join(notebook, file)), { recursive: true });
  fs.writeFileSync(path.join(notebook, file), text);
}
fs.mkdirSync(OUT, { recursive: true });

const row = (p) => `[data-testid=tree-row][data-path="${p}"]`;

/** Starts the app on the demo notebook with a theme, runs `fn`, and closes it. */
async function withApp(settings, fn) {
  fs.mkdirSync(path.join(notebook, ".cairn"), { recursive: true });
  fs.writeFileSync(path.join(notebook, ".cairn/settings.json"), JSON.stringify(settings, null, 2));
  const drv = await startDriver();
  let s;
  try {
    s = await Session.create(drv.port, APP, [notebook]);
    await s.waitFor(`return !!document.querySelector('${row("Garden")}')`, { timeout: 20000 });
    await fn(s);
  } finally {
    await s?.close();
    drv.proc.kill();
  }
}

async function openGardenPlan(s) {
  // The notebook keeps which folders are open, so open only the closed ones.
  for (const folder of ["Garden", "Kitchen"]) {
    await s.exec(`const r = document.querySelector('${row(folder)}'); if (r.getAttribute('aria-expanded') !== 'true') r.click()`);
    await sleep(200);
  }
  await s.waitFor(`return !!document.querySelector('${row("Garden/Garden plan.md")}')`);
  await s.exec(`document.querySelector('${row("Garden/Garden plan.md")}').click()`);
  await s.waitFor(`return document.querySelectorAll('[data-testid=backlink-source]').length >= 5`, { timeout: 10000 });
  // Nothing focused in the editor, so Live Preview shows every line rendered.
  await s.exec(`document.activeElement?.blur()`);
  await sleep(800);
}

async function shot(s, name) {
  fs.writeFileSync(path.join(OUT, name), await s.screenshot());
  console.log(`wrote ${path.join(OUT, name)}`);
}

test("screenshots in Limestone", { timeout: 120000 }, async () => {
  await withApp({ theme: "light", lightTheme: "limestone", darkTheme: "slate" }, async (s) => {
    await openGardenPlan(s);
    await shot(s, "editor.png");

    await s.click(await s.find("[data-testid=tab-search]"));
    await s.type(await s.findWait("[data-testid=search-input]"), "tomato");
    await s.waitFor(`return document.querySelectorAll('[data-testid=search-hit]').length >= 3`, { timeout: 10000 });
    await sleep(500);
    await shot(s, "search.png");

    await s.exec(`document.querySelector('[data-testid=open-settings]').click()`);
    await s.waitFor(`return !!document.querySelector('[data-testid=settings-appearance]')`);
    await s.exec(`document.querySelector('[data-testid=settings-appearance]').click()`);
    await s.findWait("[data-testid=theme-select]");
    await s.exec(pickThemeMode("system"));
    await sleep(500);
    await shot(s, "appearance.png");
  });
});

test("screenshots in Slate", { timeout: 120000 }, async () => {
  await withApp({ theme: "dark", lightTheme: "limestone", darkTheme: "slate" }, async (s) => {
    await openGardenPlan(s);
    await shot(s, "editor-dark.png");

    await s.exec(`document.querySelector('[data-testid=open-graph]').click()`);
    await s.waitFor(`return !!document.querySelector('[data-testid=graph-view] canvas')`, { timeout: 15000 });
    await sleep(3000);
    // Enter in the find box zooms to the note and highlights its links.
    await s.type(await s.find('[data-testid=graph-view] input[placeholder="Find note in graph"]'), "Garden plan" + Key.enter);
    await sleep(1000);
    // That zoom is close for a small notebook: two wheel steps out, over the note.
    for (let i = 0; i < 2; i++) {
      await s.exec(`const c = document.querySelector('[data-testid=graph-view] .sigma-mouse'), r = c.getBoundingClientRect();
        c.dispatchEvent(new WheelEvent('wheel', { deltaY: 120, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, bubbles: true, cancelable: true }))`);
      await sleep(700);
    }
    await sleep(1000);
    await shot(s, "graph.png");
  });
});

test.after(() => fs.rmSync(tmp, { recursive: true, force: true }));
