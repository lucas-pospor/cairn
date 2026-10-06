// settings.json written by a version with core plugins, read and saved again by
// this version's settings code (which is also what 1.0.0 has): the "corePlugins"
// key and the hotkeys of core plugin commands must come through unchanged, even
// though nothing here reads them. This test uses only load(), update() and flush(),
// so it runs against older versions of settings.svelte.ts too.
//
// Run: cd app && npx vitest run src/lib/settings_compat.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./backend", () => ({ backend: {} }));

const { settings } = await import("./settings.svelte");
const { backend } = await import("./backend");

const CORE = {
  "daily-notes": { on: true, folder: "Journal", format: "YYYY/MM/YYYY-MM-DD", template: "Templates/Daily" },
  templates: { on: false },
  // A plugin of a later version, and values of the wrong type: kept as they are.
  "some-later-plugin": { on: true, level: 3, list: ["a", { b: null }] },
  "random-note": "on",
  "unique-note": { on: "yes", folder: 7 },
};
const HOTKEYS = { "daily-notes:today": ["Mod+Alt+D"], "templates:insert": [], "editor:bold": ["Mod+J"] };

let written: string[] = [];

beforeEach(() => {
  // Just enough of a page for apply().
  vi.stubGlobal("document", {
    documentElement: { style: { setProperty() {}, removeProperty() {} }, removeAttribute() {}, dataset: {} },
    querySelectorAll: () => [],
  });
  written = [];
});

afterEach(() => {
  settings.reset();
  vi.unstubAllGlobals();
});

async function loadAndChangeTheme(file: unknown) {
  Object.assign(backend, {
    readConfig: async () => JSON.stringify(file, null, 2),
    listConfig: async () => [],
    writeConfig: async (_name: string, content: string) => void written.push(content),
  });
  await settings.load();
  settings.update({ theme: "dark" });
  await settings.flush();
  expect(written).toHaveLength(1);
  return JSON.parse(written[0]);
}

describe("settings.json with core plugins", () => {
  it("keeps corePlugins and the hotkeys of core plugin commands through load, update and save", async () => {
    const saved = await loadAndChangeTheme({ theme: "light", plugins: ["word-count.js"], hotkeys: HOTKEYS, corePlugins: CORE });
    expect(saved.theme).toBe("dark");
    expect(saved.corePlugins).toEqual(CORE);
    expect(saved.hotkeys).toEqual(HOTKEYS);
    // Third-party plugins keep their own list.
    expect(saved.plugins).toEqual(["word-count.js"]);
  });

  it("keeps a corePlugins value of the wrong type as it is", async () => {
    for (const bad of ["on", 5, [1, 2], null, true]) {
      written = [];
      const saved = await loadAndChangeTheme({ corePlugins: bad });
      expect(saved.corePlugins).toEqual(bad);
    }
  });

  it("does not add corePlugins to a file that has none", async () => {
    const saved = await loadAndChangeTheme({ theme: "light" });
    expect("corePlugins" in saved).toBe(false);
  });
});
