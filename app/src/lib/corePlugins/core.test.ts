// Core plugin switches and options in settings.json, and their commands
// (app/src/lib/corePlugins/core.ts).
//
// Run: cd app && npx vitest run src/lib/corePlugins/core.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../backend", () => ({ backend: {} }));

const { settings } = await import("../settings.svelte");
const { backend } = await import("../backend");
const { CommandRegistry } = await import("../commands");
const { coreCommands, option, pluginOn, setOption, setPluginOn } = await import("./core");
type CorePlugin = import("./core").CorePlugin;
type CoreHost = import("./core").CoreHost;

const onByDefault: CorePlugin = {
  id: "alpha",
  name: "Alpha",
  description: "On unless turned off.",
  defaultOn: true,
  defaults: { folder: "Alpha folder", format: "YYYY" },
  options: [],
  commands: [{ id: "go", name: "Go", run: vi.fn() }],
};
const offByDefault: CorePlugin = {
  id: "beta",
  name: "Beta",
  description: "Off unless turned on.",
  defaultOn: false,
  defaults: {},
  options: [],
  commands: [
    { id: "go", name: "Go", run: vi.fn() },
    { id: "edit", name: "Edit", run: vi.fn(), available: () => false },
  ],
};
const host = {} as CoreHost;

let written: string[] = [];

async function loadFile(file: unknown) {
  Object.assign(backend, {
    readConfig: async () => (file === undefined ? null : JSON.stringify(file)),
    listConfig: async () => [],
    writeConfig: async (_name: string, content: string) => void written.push(content),
  });
  await settings.load();
}

/** What the next save writes. */
async function saved() {
  await settings.flush();
  return JSON.parse(written.at(-1)!);
}

beforeEach(() => {
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

describe("switches and options", () => {
  it("uses the defaults when settings.json has nothing for a plugin", async () => {
    await loadFile({});
    expect(pluginOn(onByDefault)).toBe(true);
    expect(pluginOn(offByDefault)).toBe(false);
    expect(option(onByDefault, "folder")).toBe("Alpha folder");
    expect(option(onByDefault, "unknown")).toBe("");
  });

  it("uses what the user stored, an empty text too", async () => {
    await loadFile({ corePlugins: { alpha: { on: false, folder: "" }, beta: { on: true } } });
    expect(pluginOn(onByDefault)).toBe(false);
    expect(pluginOn(offByDefault)).toBe(true);
    expect(option(onByDefault, "folder")).toBe("");
    expect(option(onByDefault, "format")).toBe("YYYY");
  });

  it("reads a value of the wrong type as the default", async () => {
    await loadFile({ corePlugins: { alpha: { on: "no", folder: 7, format: null }, beta: true } });
    expect(pluginOn(onByDefault)).toBe(true);
    expect(pluginOn(offByDefault)).toBe(false);
    expect(option(onByDefault, "folder")).toBe("Alpha folder");
    expect(option(onByDefault, "format")).toBe("YYYY");
    for (const bad of ["x", 1, [], null]) {
      await loadFile({ corePlugins: bad });
      expect(pluginOn(onByDefault)).toBe(true);
      expect(option(onByDefault, "folder")).toBe("Alpha folder");
    }
  });

  it("writes only what the user changed", async () => {
    await loadFile({ theme: "dark" });
    setPluginOn(offByDefault, true);
    expect((await saved()).corePlugins).toEqual({ beta: { on: true } });
    setOption(onByDefault, "folder", "Journal");
    expect((await saved()).corePlugins).toEqual({ beta: { on: true }, alpha: { folder: "Journal" } });
    setPluginOn(onByDefault, true);
    expect((await saved()).corePlugins).toEqual({ beta: { on: true }, alpha: { folder: "Journal", on: true } });
  });

  it("keeps plugins and options it does not know, and wrong values of other settings, when it stores one", async () => {
    const later = { on: true, level: 3, list: [1, { a: null }] };
    await loadFile({ corePlugins: { later, alpha: { format: 5, extra: "kept" }, beta: "on" } });
    setOption(onByDefault, "folder", "Notes");
    expect((await saved()).corePlugins).toEqual({ later, alpha: { format: 5, extra: "kept", folder: "Notes" }, beta: "on" });
    // A wrong value of the setting itself goes when the user changes that setting.
    setOption(onByDefault, "format", "YY");
    expect((await saved()).corePlugins.alpha).toEqual({ format: "YY", extra: "kept", folder: "Notes" });
    setPluginOn(offByDefault, true);
    expect((await saved()).corePlugins.beta).toEqual({ on: true });
  });

  it("replaces a corePlugins value that is not an object only when the user changes a core plugin", async () => {
    await loadFile({ corePlugins: ["x"] });
    settings.update({ theme: "light" });
    expect((await saved()).corePlugins).toEqual(["x"]);
    setPluginOn(onByDefault, false);
    expect((await saved()).corePlugins).toEqual({ alpha: { on: false } });
  });

  it("goes back to the defaults when the vault is closed", async () => {
    await loadFile({ corePlugins: { alpha: { on: false } } });
    settings.reset();
    expect(pluginOn(onByDefault)).toBe(true);
  });
});

describe("commands", () => {
  function registry() {
    const r = new CommandRegistry();
    r.register([{ id: "editor:bold", name: "Toggle bold", run: () => {} }, ...coreCommands([onByDefault, offByDefault], host)]);
    return r;
  }

  it("names each command after its plugin and gives it the plugin's id", async () => {
    await loadFile({});
    expect(registry().registered().map((c) => [c.id, c.name])).toEqual([
      ["editor:bold", "Toggle bold"],
      ["alpha:go", "Alpha: Go"],
      ["beta:go", "Beta: Go"],
      ["beta:edit", "Beta: Edit"],
    ]);
  });

  it("lists, binds and runs a plugin's commands only while it is on", async () => {
    await loadFile({ hotkeys: { "beta:go": ["Mod+Alt+B"] } });
    const r = registry();
    r.setOverrides(settings.value.hotkeys);
    expect(r.all().map((c) => c.id)).toEqual(["editor:bold", "alpha:go"]);
    expect(r.bindable().map((c) => c.id)).toEqual(["editor:bold", "alpha:go"]);
    expect(r.lookup("Mod+Alt+B")).toBeNull();

    setPluginOn(offByDefault, true);
    // "beta:edit" cannot run now, but can have a hotkey.
    expect(r.all().map((c) => c.id)).toEqual(["editor:bold", "alpha:go", "beta:go"]);
    expect(r.bindable().map((c) => c.id)).toEqual(["editor:bold", "alpha:go", "beta:go", "beta:edit"]);
    r.lookup("Mod+Alt+B")!.run();
    expect(offByDefault.commands[0].run).toHaveBeenCalledWith(host);

    // Turned off again: the binding stays saved.
    setPluginOn(offByDefault, false);
    expect(r.lookup("Mod+Alt+B")).toBeNull();
    expect((await saved()).hotkeys).toEqual({ "beta:go": ["Mod+Alt+B"] });
  });

  it("counts the keys of a plugin that is off as taken, so a combo never runs two commands", async () => {
    await loadFile({ hotkeys: { "beta:go": ["Mod+Alt+B"] } });
    const r = registry();
    r.setOverrides(settings.value.hotkeys);
    expect(r.conflicts("Mod+Alt+B", "editor:bold").map((c) => c.id)).toEqual(["beta:go"]);
  });
});
