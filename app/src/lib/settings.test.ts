// Reading and saving settings.json (app/src/lib/settings.svelte.ts).
//
// Run: cd app && npx vitest run src/lib/settings.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./backend", () => ({ backend: {} }));

const { validHotkeys, settings } = await import("./settings.svelte");
const { backend } = await import("./backend");

describe("validHotkeys", () => {
  it("keeps only lists of key combos from a hand-edited settings.json (FINDING-194)", () => {
    expect(validHotkeys({ "editor:bold": ["Mod+J"], "app:graph": [] })).toEqual({ "editor:bold": ["Mod+J"], "app:graph": [] });
    // A string would be matched as a substring ("Mod+J" contains "J", "M", "O", "D"); a number has no includes().
    expect(validHotkeys({ "editor:bold": "Mod+J", "editor:italic": 5, "editor:code": ["Mod+1", 2], "note:new": null })).toEqual({});
    for (const v of [null, undefined, "x", 5, ["Mod+J"]]) expect(validHotkeys(v)).toEqual({});
  });

  it("names the hotkeys it ignores in a console warning", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    validHotkeys({ "editor:bold": "Mod+J", "editor:italic": ["Mod+U"], "note:new": null });
    expect(warn).toHaveBeenCalledOnce();
    expect(warn.mock.calls[0][0]).toBe("settings.json: ignoring hotkeys that are not lists of keys: editor:bold, note:new");
    warn.mockClear();
    validHotkeys({ "editor:italic": ["Mod+U"] });
    validHotkeys(undefined);
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});

describe("saving", () => {
  it("reports a change that could not be saved, so the app can say so (FINDING-013, config folders)", async () => {
    // Just enough of a page for apply().
    vi.stubGlobal("document", {
      documentElement: { style: { setProperty() {}, removeProperty() {} }, removeAttribute() {}, dataset: {} },
      querySelectorAll: () => [],
    });
    vi.useFakeTimers();
    const refused = { kind: "io", detail: 'The ".cairn" folder leads outside the vault.' };
    const writeConfig = vi.fn(async () => {
      throw refused;
    });
    Object.assign(backend, { readConfig: async () => '{"theme":"dark"}', listConfig: async () => [], writeConfig });
    const reported: unknown[] = [];
    settings.onSaveError = (e) => reported.push(e);
    try {
      await settings.load();
      expect(settings.value.theme).toBe("dark");
      settings.update({ theme: "light" });
      await vi.advanceTimersByTimeAsync(300);
      expect(writeConfig).toHaveBeenCalledOnce();
      expect(reported).toEqual([refused]);
    } finally {
      settings.reset();
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });
});

describe("light and dark themes", () => {
  let root: { dataset: Record<string, string> };
  let written: string[];

  beforeEach(() => {
    root = { dataset: {} };
    vi.stubGlobal("document", {
      documentElement: { style: { setProperty() {}, removeProperty() {} }, removeAttribute() {}, dataset: root.dataset },
      querySelectorAll: () => [],
    });
    written = [];
  });

  afterEach(() => {
    settings.reset();
    vi.unstubAllGlobals();
  });

  async function load(file: unknown) {
    Object.assign(backend, {
      readConfig: async () => JSON.stringify(file),
      listConfig: async () => [],
      writeConfig: async (_name: string, content: string) => void written.push(content),
    });
    await settings.load();
  }

  async function saved() {
    await settings.flush();
    return JSON.parse(written.at(-1)!);
  }

  it("applies the saved themes, and saves a new choice", async () => {
    await load({ theme: "system", lightTheme: "marble", darkTheme: "graphite" });
    expect(root.dataset).toMatchObject({ lightTheme: "marble", darkTheme: "graphite" });
    settings.update({ lightTheme: "limestone" });
    expect(root.dataset).toMatchObject({ lightTheme: "limestone", darkTheme: "graphite" });
    expect(await saved()).toMatchObject({ theme: "system", lightTheme: "limestone", darkTheme: "graphite" });
  });

  it("uses Limestone and Slate when none is chosen, and writes no theme keys until one is", async () => {
    await load({ fontSize: 15 });
    expect(root.dataset).toMatchObject({ lightTheme: "limestone", darkTheme: "slate" });
    settings.update({ fontSize: 17 });
    const file = await saved();
    expect("lightTheme" in file || "darkTheme" in file).toBe(false);
    settings.update({ darkTheme: "graphite" });
    const chosen = await saved();
    expect(chosen).toMatchObject({ darkTheme: "graphite" });
    expect("lightTheme" in chosen).toBe(false);
  });

  it("shows the default for an id it does not know or a wrong value, and keeps that value in the file", async () => {
    // A theme of a later version, a typo, a dark theme as the light one, and values of the wrong type.
    for (const bad of ["sandstone", "Marble", "slate", 5, null, true, ["marble"], { id: "marble" }]) {
      written = [];
      await load({ lightTheme: bad, darkTheme: bad === "slate" ? "limestone" : bad });
      expect(root.dataset).toMatchObject({ lightTheme: "limestone", darkTheme: "slate" });
      settings.update({ fontSize: 18 });
      const file = await saved();
      expect(file.lightTheme).toEqual(bad);
      expect(file.darkTheme).toEqual(bad === "slate" ? "limestone" : bad);
      settings.reset();
    }
  });
});
