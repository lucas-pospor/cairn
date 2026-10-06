// The Daily notes core plugin (app/src/lib/corePlugins/dailyNotes.ts).
//
// Run: cd app && npx vitest run src/lib/corePlugins/dailyNotes.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../backend", () => ({ backend: {} }));

const { settings } = await import("../settings.svelte");
const { backend } = await import("../backend");
const { pluginOn, setOption } = await import("./core");
const { dailyNotes, dailyPath } = await import("./dailyNotes");
const { templates } = await import("./templates");
type CoreHost = import("./core").CoreHost;

// Monday 5 October 2026, 07:30.
const NOW = new Date(2026, 9, 5, 7, 30);

/** A host with a vault of `files` (path -> text). createNote fails as the core does when a file is there. */
function fakeHost(files: Record<string, string>) {
  const created: [string, string][] = [];
  const opened: string[] = [];
  const toasts: string[] = [];
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
      const twin = Object.keys(files).find((f) => f.toLowerCase() === p.toLowerCase());
      if (twin) throw { kind: "alreadyExists", detail: twin };
      files[p] = content;
      created.push([p, content]);
    },
    openNote: async (p) => void opened.push(p),
    uniquePath: async () => {
      throw new Error("Daily notes asks for no other name");
    },
    toast: (m) => void toasts.push(m),
    now: () => NOW,
    random: () => 0,
  };
  return { host, files, created, opened, toasts };
}

const openToday = (host: CoreHost) => dailyNotes.commands[0].run(host);

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

describe("dailyPath", () => {
  it("names the note by the date, in the folder", () => {
    expect(dailyPath(NOW, "", "")).toBe("2026-10-05.md");
    expect(dailyPath(NOW, "", "YYYY-MM-DD")).toBe("2026-10-05.md");
    expect(dailyPath(NOW, " Journal/ ", "dddd, MMMM Do YYYY")).toBe("Journal/Monday, October 5th 2026.md");
    expect(dailyPath(NOW, "Journal", "YYYY/MM/YYYY-MM-DD")).toBe("Journal/2026/10/2026-10-05.md");
  });
});

describe("Open today's note", () => {
  it("is on by default, and makes YYYY-MM-DD.md in the vault root", async () => {
    expect(pluginOn(dailyNotes)).toBe(true);
    const f = fakeHost({});
    await openToday(f.host);
    expect(f.created).toEqual([["2026-10-05.md", ""]]);
    expect(f.opened).toEqual(["2026-10-05.md"]);
    expect(f.toasts).toEqual([]);
  });

  it("opens today's note when it is there, without writing to it", async () => {
    const f = fakeHost({ "2026-10-05.md": "Already written.\n" });
    await openToday(f.host);
    expect(f.created).toEqual([]);
    expect(f.opened).toEqual(["2026-10-05.md"]);
    expect(f.files["2026-10-05.md"]).toBe("Already written.\n");
  });

  it("opens a note that appeared since the file list was read, or whose name differs only in case", async () => {
    const f = fakeHost({});
    f.host.files = () => [];
    f.files["2026-10-05.MD"] = "From another device.\n";
    await openToday(f.host);
    expect(f.created).toEqual([]);
    expect(f.opened).toEqual(["2026-10-05.MD"]);
    expect(f.files["2026-10-05.MD"]).toBe("From another device.\n");
  });

  it("uses the folder, format and template set in Settings", async () => {
    setOption(dailyNotes, "folder", "Journal");
    setOption(dailyNotes, "format", "YYYY/MM/YYYY-MM-DD");
    setOption(dailyNotes, "template", "Templates/Day");
    const f = fakeHost({ "Templates/Day.md": "# {{title}}\n{{date:dddd}} at {{time}}\n{{date}}\n" });
    await openToday(f.host);
    expect(f.created).toEqual([["Journal/2026/10/2026-10-05.md", "# 2026-10-05\nMonday at 07:30\n2026-10-05\n"]]);
    expect(f.opened).toEqual(["Journal/2026/10/2026-10-05.md"]);
  });

  it("fills {{date}} and {{time}} with the formats of the Templates plugin", async () => {
    setOption(dailyNotes, "template", "Day.md");
    setOption(templates, "dateFormat", "D MMMM YYYY");
    setOption(templates, "timeFormat", "h:mm A");
    const f = fakeHost({ "Day.md": "{{date}}, {{time}}" });
    await openToday(f.host);
    expect(f.created).toEqual([["2026-10-05.md", "5 October 2026, 7:30 AM"]]);
  });

  it("creates nothing when the template cannot be read", async () => {
    setOption(dailyNotes, "template", "Templates/Gone");
    const f = fakeHost({});
    await openToday(f.host);
    expect(f.created).toEqual([]);
    expect(f.opened).toEqual([]);
    expect(f.toasts).toEqual(["Could not read the daily note template Templates/Gone.md: Not found: Templates/Gone.md"]);
  });

  it("creates nothing when the format makes a name that cannot be a file name", async () => {
    setOption(dailyNotes, "format", "YYYY-MM-DD HH:mm");
    const f = fakeHost({});
    await openToday(f.host);
    expect(f.created).toEqual([]);
    expect(f.toasts).toEqual([
      'Today\'s note cannot be called 2026-10-05 07:30.md: "2026-10-05 07:30.md" contains :, which names cannot contain. Change it in Settings > Core plugins.',
    ]);
  });
});

describe("Settings examples", () => {
  const [folder, format, template] = dailyNotes.options;
  const { host } = fakeHost({ "Templates/Day.md": "" });
  const get = (values: Record<string, string>) => (key: string) => values[key] ?? "";

  it("shows where today's note goes, with the folder as it is in Settings", () => {
    expect(format.check?.("YYYY-MM-DD", host, get({}))).toEqual({ example: "Today's note: 2026-10-05.md" });
    expect(format.check?.("", host, get({ folder: "Journal" }))).toEqual({ example: "Today's note: Journal/2026-10-05.md" });
    expect(format.check?.("gggg/[W]ww/ddd", host, get({ folder: "Log" }))).toEqual({ example: "Today's note: Log/2026/W41/Mon.md" });
  });

  it("says why a name is not valid", () => {
    expect(format.check?.("HH:mm", host, get({}))).toEqual({ problem: 'Not a valid name: "07:30.md" contains :, which names cannot contain.' });
    expect(format.check?.("[.]YYYY", host, get({}))?.problem).toMatch(/starts with a dot/);
    expect(folder.check?.(".journal", host, get({}))?.problem).toMatch(/starts with a dot/);
    expect(folder.check?.("", host, get({}))).toEqual({});
    expect(folder.check?.("Journal", host, get({}))).toEqual({});
  });

  it("says when the template is not there", () => {
    expect(template.check?.("", host, get({}))).toEqual({});
    expect(template.check?.("Templates/Day", host, get({}))).toEqual({});
    expect(template.check?.("Templates/Night", host, get({}))).toEqual({ problem: "There is no note at Templates/Night.md." });
  });
});
