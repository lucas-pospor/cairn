// The Templates core plugin (app/src/lib/corePlugins/templates.ts).
//
// Run: cd app && npx vitest run src/lib/corePlugins/templates.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../backend", () => ({ backend: {} }));

const { settings } = await import("../settings.svelte");
const { backend } = await import("../backend");
const { pluginOn, setOption } = await import("./core");
const { fillTemplate, templates } = await import("./templates");
type CoreHost = import("./core").CoreHost;

// Monday 5 October 2026, 09:04.
const NOW = new Date(2026, 9, 5, 9, 4);
const values = { title: "Standup", now: NOW, dateFormat: "YYYY-MM-DD", timeFormat: "HH:mm" };

describe("fillTemplate", () => {
  it("fills in the title, date and time", () => {
    expect(fillTemplate("# {{title}}\n{{date}} {{time}}\n", values)).toBe("# Standup\n2026-10-05 09:04\n");
  });

  it("takes a format in the template", () => {
    expect(fillTemplate("{{date:dddd, MMMM Do YYYY}} at {{time:h:mm A}}", values)).toBe("Monday, October 5th 2026 at 9:04 AM");
    expect(fillTemplate("{{date:YYYY}}-{{date:[W]WW}}", values)).toBe("2026-W41");
  });

  it("uses the formats from Settings, or the defaults when they are empty", () => {
    expect(fillTemplate("{{date}} {{time}}", { ...values, dateFormat: "DD.MM.YYYY", timeFormat: "HH.mm" })).toBe("05.10.2026 09.04");
    expect(fillTemplate("{{date}} {{time}} {{date:}}", { ...values, dateFormat: "", timeFormat: "" })).toBe("2026-10-05 09:04 2026-10-05");
  });

  it("ignores case and spaces in the braces, and every occurrence counts", () => {
    expect(fillTemplate("{{TITLE}} {{ Date }} {{Time }} {{title}}", values)).toBe("Standup 2026-10-05 09:04 Standup");
  });

  it("leaves other text in braces alone", () => {
    const text = "{{name}} {{title:x}} {title} {{ }} {{date x}} ``{{code}}``";
    expect(fillTemplate(text, values)).toBe(text);
  });

  it("keeps a title with dollar signs and braces as it is", () => {
    expect(fillTemplate("# {{title}}", { ...values, title: "Costs $& {{date}}" })).toBe("# Costs $& {{date}}");
  });
});

/** A host with a vault of `files` and a note open for editing. */
function fakeHost(files: string[], opts: { open?: string | null; choose?: string | null; text?: Record<string, string> } = {}) {
  const inserted: [string, string][] = [];
  const toasts: string[] = [];
  const choices: { value: string; label: string }[][] = [];
  const open = opts.open === undefined ? "Notes/Standup.md" : opts.open;
  const host: CoreHost = {
    files: () => files,
    folders: () => [...new Set(files.flatMap((f) => f.split("/").slice(0, -1).map((_, i, a) => a.slice(0, i + 1).join("/"))))],
    activeNote: () => open,
    canInsert: () => open !== null,
    insert: (path, text) => (inserted.push([path, text]), true),
    readNote: async (p) => {
      const t = opts.text?.[p];
      if (t === undefined) throw { kind: "notFound", detail: p };
      return t;
    },
    choose: async (_title, options) => {
      choices.push(options);
      return opts.choose === undefined ? (options[0]?.value ?? null) : opts.choose;
    },
    createNote: async () => {
      throw new Error("Templates creates no notes");
    },
    openNote: async () => {},
    toast: (m) => void toasts.push(m),
    now: () => NOW,
  };
  return { host, inserted, toasts, choices };
}

const insert = (host: CoreHost) => templates.commands[0].run(host);

beforeEach(() => {
  vi.stubGlobal("document", {
    documentElement: { style: { setProperty() {}, removeProperty() {} }, removeAttribute() {}, dataset: {} },
    querySelectorAll: () => [],
  });
  Object.assign(backend, { readConfig: async () => null, listConfig: async () => [], writeConfig: async () => {} });
});

afterEach(() => {
  settings.reset();
  vi.unstubAllGlobals();
});

describe("Insert template", () => {
  const vault = ["Notes/Standup.md", "Templates/Meeting.md", "Templates/Work/Daily.md", "Templates/logo.png", "Templates/.hidden.md", "Other.md"];

  it("is on by default, with its templates in Templates", async () => {
    await settings.load();
    expect(pluginOn(templates)).toBe(true);
    const { host, choices } = fakeHost(vault, { choose: null });
    await insert(host);
    expect(choices).toEqual([[{ value: "Templates/Meeting.md", label: "Meeting" }, { value: "Templates/Work/Daily.md", label: "Work/Daily" }]]);
  });

  it("inserts the chosen template, filled in, into the note being edited", async () => {
    const { host, inserted, toasts } = fakeHost(vault, {
      choose: "Templates/Work/Daily.md",
      text: { "Templates/Work/Daily.md": "---\ntags: [daily]\n---\n# {{title}}, {{date}}\n" },
    });
    await insert(host);
    expect(inserted).toEqual([["Notes/Standup.md", "---\ntags: [daily]\n---\n# Standup, 2026-10-05\n"]]);
    expect(toasts).toEqual([]);
  });

  it("uses the folder and formats set in Settings", async () => {
    await settings.load();
    setOption(templates, "folder", " /Snippets/ ");
    setOption(templates, "dateFormat", "D MMMM");
    setOption(templates, "timeFormat", "h A");
    const { host, inserted } = fakeHost(["Notes/Standup.md", "Snippets/Sign-off.md", "Templates/Meeting.md"], {
      text: { "Snippets/Sign-off.md": "{{date}}, {{time}}" },
    });
    await insert(host);
    expect(inserted).toEqual([["Notes/Standup.md", "5 October, 9 AM"]]);
  });

  it("does nothing when the user cancels", async () => {
    const { host, inserted, toasts } = fakeHost(vault, { choose: null });
    await insert(host);
    expect(inserted).toEqual([]);
    expect(toasts).toEqual([]);
  });

  it("says what is missing instead of inserting", async () => {
    await settings.load();
    const noFolder = fakeHost(["Notes/Standup.md"]);
    await insert(noFolder.host);
    expect(noFolder.toasts).toEqual(["There is no folder named Templates. Create it and add notes to use them as templates."]);

    const empty = fakeHost(["Notes/Standup.md", "Templates/logo.png"]);
    await insert(empty.host);
    expect(empty.toasts).toEqual(["There are no notes in Templates. Add one to use it as a template."]);

    setOption(templates, "folder", "");
    const unset = fakeHost(vault);
    await insert(unset.host);
    expect(unset.toasts).toEqual(["Choose a template folder in Settings > Core plugins."]);

    const closed = fakeHost(vault, { open: null });
    await insert(closed.host);
    expect(closed.toasts).toEqual(["Open a note to insert a template."]);
    for (const h of [noFolder, empty, unset, closed]) expect(h.inserted).toEqual([]);
  });

  it("reports a template it cannot read", async () => {
    const { host, inserted, toasts } = fakeHost(vault, { choose: "Templates/Meeting.md", text: {} });
    await insert(host);
    expect(inserted).toEqual([]);
    expect(toasts).toEqual(["Could not read Meeting: Not found: Templates/Meeting.md"]);
  });

  it("says so when the note was closed while the user chose", async () => {
    const f = fakeHost(vault, { text: { "Templates/Meeting.md": "x" } });
    f.host.insert = () => false;
    await insert(f.host);
    expect(f.toasts).toEqual(["Standup is no longer open for editing. The template was not inserted."]);
  });

  it("can only run while a note is open for editing", () => {
    expect(templates.commands[0].available?.(fakeHost(vault).host)).toBe(true);
    expect(templates.commands[0].available?.(fakeHost(vault, { open: null }).host)).toBe(false);
  });
});

describe("Settings examples", () => {
  const [folder, date, time] = templates.options;
  const { host } = fakeHost(["Templates/A.md", "Templates/B/C.md", "Notes/x.md"]);
  const get = () => "";

  it("counts the templates in the folder, or says what is wrong with it", () => {
    expect(folder.check?.("Templates", host, get)).toEqual({ example: "2 templates" });
    expect(folder.check?.("Templates/B", host, get)).toEqual({ example: "1 template" });
    expect(folder.check?.("Notes/", host, get)).toEqual({ example: "1 template" });
    expect(folder.check?.("Missing", host, get)).toEqual({ example: "There is no folder named Missing yet." });
    expect(folder.check?.("", host, get)).toEqual({ problem: "Choose a folder to use templates." });
    expect(folder.check?.(".cairn/templates", host, get)?.problem).toMatch(/start with a dot/);
  });

  it("shows today's date and the time in the chosen formats", () => {
    expect(date.check?.("dddd D MMMM", host, get)).toEqual({ example: "Today: Monday 5 October" });
    expect(date.check?.("", host, get)).toEqual({ example: "Today: 2026-10-05" });
    expect(time.check?.("h:mm a", host, get)).toEqual({ example: "Now: 9:04 am" });
  });
});
