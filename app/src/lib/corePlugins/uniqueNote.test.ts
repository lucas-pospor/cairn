// The Unique note creator core plugin (app/src/lib/corePlugins/uniqueNote.ts).
//
// Run: cd app && npx vitest run src/lib/corePlugins/uniqueNote.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../backend", () => ({ backend: {} }));

const { settings } = await import("../settings.svelte");
const { backend } = await import("../backend");
const { pluginOn, setOption } = await import("./core");
const { uniqueNote, uniqueName } = await import("./uniqueNote");
type CoreHost = import("./core").CoreHost;

// Monday 5 October 2026, 14:32:07.
const NOW = new Date(2026, 9, 5, 14, 32, 7);

/**
 * A host with a vault of `files` (path -> text). uniquePath and createNote behave as
 * the core's: the first free "base", "base 1", ...; never a create over a file.
 */
function fakeHost(files: Record<string, string>) {
  const created: [string, string][] = [];
  const opened: [string, boolean | undefined][] = [];
  const toasts: string[] = [];
  const taken = (p: string) => Object.keys(files).some((f) => f.toLowerCase() === p.toLowerCase());
  const host: CoreHost = {
    files: () => Object.keys(files),
    folders: () => [],
    activeNote: () => null,
    canInsert: () => false,
    insert: () => false,
    readNote: async (p) => {
      if (!(p in files)) throw { kind: "notFound", detail: p };
      return files[p];
    },
    choose: async () => null,
    createNote: async (p, content) => {
      if (taken(p)) throw { kind: "alreadyExists", detail: p };
      files[p] = content;
      created.push([p, content]);
    },
    openNote: async (p, newTab) => void opened.push([p, newTab]),
    uniquePath: async (dir, base) => {
      for (let n = 0; ; n++) {
        const p = `${dir ? `${dir}/` : ""}${base}${n ? ` ${n}` : ""}.md`;
        if (!taken(p)) return p;
      }
    },
    toast: (m) => void toasts.push(m),
    now: () => NOW,
  };
  return { host, files, created, opened, toasts };
}

const create = (host: CoreHost) => uniqueNote.commands[0].run(host);

beforeEach(async () => {
  vi.stubGlobal("document", {
    documentElement: { style: { setProperty() {}, removeProperty() {} }, removeAttribute() {}, dataset: {} },
    querySelectorAll: () => [],
  });
  Object.assign(backend, { readConfig: async () => null, listConfig: async () => [], writeConfig: async () => {} });
  await settings.load();
});

afterEach(() => {
  settings.reset();
  vi.unstubAllGlobals();
});

describe("Create unique note", () => {
  it("is off by default, and names the note YYYYMMDDHHmm in the vault root", async () => {
    expect(pluginOn(uniqueNote)).toBe(false);
    expect(uniqueName(NOW, "", "")).toBe("202610051432.md");
    const f = fakeHost({});
    await create(f.host);
    expect(f.created).toEqual([["202610051432.md", ""]]);
    // In a new tab, as a new note.
    expect(f.opened).toEqual([["202610051432.md", true]]);
  });

  it("gives a taken name a number, and never writes over a note", async () => {
    const f = fakeHost({ "202610051432.md": "First.", "202610051432 1.md": "Second." });
    await create(f.host);
    await create(f.host);
    expect(f.created.map(([p]) => p)).toEqual(["202610051432 2.md", "202610051432 3.md"]);
    expect(f.files["202610051432.md"]).toBe("First.");
    expect(f.files["202610051432 1.md"]).toBe("Second.");
  });

  it("takes the next name when a note appears at the free one before it is created", async () => {
    const f = fakeHost({});
    const uniquePath = f.host.uniquePath;
    let first = true;
    f.host.uniquePath = async (dir, base) => {
      const p = await uniquePath(dir, base);
      // Another device's note arrives right after the name was chosen.
      if (first) f.files[p] = "From sync.";
      first = false;
      return p;
    };
    await create(f.host);
    expect(f.files["202610051432.md"]).toBe("From sync.");
    expect(f.created.map(([p]) => p)).toEqual(["202610051432 1.md"]);
    expect(f.opened).toEqual([["202610051432 1.md", true]]);
  });

  it("uses the folder, format and template set in Settings", async () => {
    setOption(uniqueNote, "folder", "Zettel");
    setOption(uniqueNote, "format", "YYYYMMDDHHmmss");
    setOption(uniqueNote, "template", "Templates/Card");
    const f = fakeHost({ "Templates/Card.md": "# {{title}}\n\nCreated {{date}} {{time}}\n" });
    await create(f.host);
    expect(f.created).toEqual([["Zettel/20261005143207.md", "# 20261005143207\n\nCreated 2026-10-05 14:32\n"]]);
  });

  it("fills {{title}} with the name the note got", async () => {
    setOption(uniqueNote, "template", "T");
    const f = fakeHost({ "T.md": "{{title}}", "202610051432.md": "" });
    await create(f.host);
    expect(f.created).toEqual([["202610051432 1.md", "202610051432 1"]]);
  });

  it("creates nothing when the template cannot be read or the name is not valid", async () => {
    setOption(uniqueNote, "template", "Gone");
    const missing = fakeHost({});
    await create(missing.host);
    expect(missing.created).toEqual([]);
    expect(missing.toasts).toEqual(["Could not read the template Gone.md: Not found: Gone.md"]);

    setOption(uniqueNote, "template", "");
    setOption(uniqueNote, "format", "HH:mm");
    const invalid = fakeHost({});
    await create(invalid.host);
    expect(invalid.created).toEqual([]);
    expect(invalid.toasts).toEqual(['A new note cannot be called 14:32.md: "14:32.md" contains :, which names cannot contain. Change it in Settings > Core plugins.']);
  });

  it("reports a failure other than a taken name, and opens nothing", async () => {
    const f = fakeHost({});
    f.host.createNote = async () => {
      throw { kind: "alreadyExists", detail: "zettel" };
    };
    setOption(uniqueNote, "folder", "Zettel");
    await create(f.host);
    expect(f.opened).toEqual([]);
    expect(f.toasts).toEqual(['Could not create Zettel/202610051432.md: Something named "zettel" already exists.']);
  });
});

describe("Settings examples", () => {
  const [, format] = uniqueNote.options;
  const { host } = fakeHost({});

  it("shows the name a note made now would get, or why it is not valid", () => {
    expect(format.check?.("", host, () => "")).toEqual({ example: "A note made now: 202610051432.md" });
    expect(format.check?.("YYYY-MM-DD-HHmmss", host, (k) => (k === "folder" ? "Zettel" : ""))).toEqual({ example: "A note made now: Zettel/2026-10-05-143207.md" });
    expect(format.check?.("HH:mm", host, () => "")?.problem).toMatch(/^Not a valid name: .*contains :/);
  });
});
