// settings.json written by a version with core plugins or a choice of light and
// dark themes, read and saved again by this version's settings code: the
// "corePlugins", "lightTheme" and "darkTheme" keys and the hotkeys of core plugin
// commands must come through unchanged. This test uses only load(), update() and
// flush(), so it runs against older versions of settings.svelte.ts too (1.0.0 and
// 1.1.0 keep keys they do not know, and replace only a "theme" they do not know).
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

async function loadAndChange(file: unknown, patch: Parameters<typeof settings.update>[0]) {
  Object.assign(backend, {
    readConfig: async () => JSON.stringify(file, null, 2),
    listConfig: async () => [],
    writeConfig: async (_name: string, content: string) => void written.push(content),
  });
  await settings.load();
  settings.update(patch);
  await settings.flush();
  expect(written).toHaveLength(1);
  return JSON.parse(written[0]);
}

const loadAndChangeTheme = (file: unknown) => loadAndChange(file, { theme: "dark" });

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

describe("settings.json with a light and a dark theme chosen", () => {
  const THEMES = { lightTheme: "marble", darkTheme: "graphite" };

  it("keeps lightTheme and darkTheme when the theme is switched", async () => {
    const saved = await loadAndChangeTheme({ theme: "system", ...THEMES });
    expect(saved).toMatchObject({ theme: "dark", ...THEMES });
  });

  it("keeps them when another setting changes", async () => {
    for (const theme of ["system", "light", "dark"]) {
      written = [];
      const saved = await loadAndChange({ theme, ...THEMES, fontSize: 15 }, { fontSize: 18 });
      expect(saved).toMatchObject({ theme, ...THEMES, fontSize: 18 });
    }
  });

  it("keeps theme ids of later versions and values of the wrong type", async () => {
    for (const v of ["some-later-theme", 5, null, ["marble"], { id: "graphite" }]) {
      written = [];
      const saved = await loadAndChange({ lightTheme: v, darkTheme: v }, { fontSize: 18 });
      expect(saved.lightTheme).toEqual(v);
      expect(saved.darkTheme).toEqual(v);
    }
  });

  it("would lose a theme id written into \"theme\" itself, which is why it has keys of its own", async () => {
    const saved = await loadAndChange({ theme: "graphite" }, { fontSize: 18 });
    expect(saved.theme).toBe("system");
  });
});
