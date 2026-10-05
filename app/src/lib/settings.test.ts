// Reading and saving settings.json (app/src/lib/settings.svelte.ts).
//
// Run: cd app && npx vitest run src/lib/settings.test.ts

import { describe, expect, it, vi } from "vitest";

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
