// Note paths and note creation for core plugins (app/src/lib/corePlugins/files.ts).
//
// Run: cd app && npx vitest run src/lib/corePlugins/files.test.ts

import { describe, expect, it, vi } from "vitest";
import { cleanFolder, createOrOpen, noteOption, notePath, notePathProblem, notesIn } from "./files";
import type { CoreHost } from "./core";

describe("paths", () => {
  it("cleans a folder option", () => {
    expect(cleanFolder("")).toBe("");
    expect(cleanFolder(" /Journal/ ")).toBe("Journal");
    expect(cleanFolder("Work//Daily\\Notes/")).toBe("Work/Daily/Notes");
    // NFD typed on a Mac becomes NFC, as vault paths are.
    expect(cleanFolder("Café")).toBe("Café");
  });

  it("puts a note name in a folder, with .md", () => {
    expect(notePath("", "2026-10-05")).toBe("2026-10-05.md");
    expect(notePath("/Journal/", "2026/10/2026-10-05")).toBe("Journal/2026/10/2026-10-05.md");
    expect(notePath("Journal", "Notes.md")).toBe("Journal/Notes.md");
  });

  it("reads a note option with or without .md", () => {
    expect(noteOption("")).toBe("");
    expect(noteOption(" Templates/Daily ")).toBe("Templates/Daily.md");
    expect(noteOption("Templates/Daily.md")).toBe("Templates/Daily.md");
    expect(noteOption("Templates/Daily.markdown")).toBe("Templates/Daily.markdown");
  });

  it("lists the notes in a folder and its subfolders", () => {
    const files = ["A.md", "T/b.md", "T/a.md", "T/x/c.md", "T/pic.png", "T/.hidden.md", "TT/d.md"];
    expect(notesIn("T", files)).toEqual(["T/a.md", "T/b.md", "T/x/c.md"]);
    expect(notesIn("", files)).toEqual(["A.md", "T/a.md", "T/b.md", "T/x/c.md", "TT/d.md"]);
  });
});

describe("notePathProblem", () => {
  it("accepts names the core accepts, in any script", () => {
    for (const p of ["2026-10-05.md", "Journal/2026/10/05.md", "Monday, October 5th 2026.md", "日記 2026年10月5日.md", "Ημερολόγιο.md", "📓 notes.md", "a.b.md"]) {
      expect(notePathProblem(p), p).toBeNull();
    }
  });

  it("says why the core would refuse a name", () => {
    expect(notePathProblem("14:30.md")).toBe('"14:30.md" contains :, which names cannot contain.');
    for (const c of ["\\", "*", "?", '"', "<", ">", "|", "[", "]", "#", "^"]) expect(notePathProblem(`a${c}b.md`), c).toMatch(/which names cannot contain/);
    expect(notePathProblem(".hidden.md")).toBe('".hidden.md" starts with a dot, which would hide it.');
    expect(notePathProblem("Journal/.x/a.md")).toBe('".x" starts with a dot, which would hide it.');
    expect(notePathProblem("a//b.md")).toBe("A folder name in it is empty.");
    expect(notePathProblem(" a.md")).toBe('" a.md" starts or ends with a space.');
    expect(notePathProblem("Journal /a.md")).toBe('"Journal " starts or ends with a space.');
    expect(notePathProblem("dots./a.md")).toBe('"dots." ends with a dot.');
    expect(notePathProblem("a\tb.md")).toBe('"a\tb.md" contains a control character.');
  });

  it("refuses a name longer than 255 bytes", () => {
    expect(notePathProblem(`${"a".repeat(252)}.md`)).toBeNull();
    expect(notePathProblem(`${"a".repeat(253)}.md`)).toMatch(/is too long for a file name/);
    // Two bytes each in UTF-8.
    expect(notePathProblem(`${"é".repeat(126)}.md`)).toBeNull();
    expect(notePathProblem(`${"é".repeat(127)}.md`)).toMatch(/is too long/);
  });
});

describe("createOrOpen", () => {
  function host(createNote: CoreHost["createNote"]) {
    const opened: [string, boolean | undefined][] = [];
    const toasts: string[] = [];
    const h = {
      createNote: vi.fn(createNote),
      openNote: async (p: string, newTab?: boolean) => void opened.push([p, newTab]),
      toast: (m: string) => void toasts.push(m),
    } as unknown as CoreHost;
    return { h, opened, toasts };
  }

  it("creates the note and opens it", async () => {
    const { h, opened, toasts } = host(async () => {});
    await createOrOpen(h, "2026-10-05.md", "text", true);
    expect(h.createNote).toHaveBeenCalledWith("2026-10-05.md", "text");
    expect(opened).toEqual([["2026-10-05.md", true]]);
    expect(toasts).toEqual([]);
  });

  it("opens the note that is already there instead of writing over it", async () => {
    const { h, opened, toasts } = host(async (p) => {
      throw { kind: "alreadyExists", detail: p };
    });
    await createOrOpen(h, "2026-10-05.md", "text");
    expect(opened).toEqual([["2026-10-05.md", false]]);
    expect(toasts).toEqual([]);
  });

  it("opens a note whose name differs only in case", async () => {
    const { h, opened } = host(async () => {
      throw { kind: "alreadyExists", detail: "Journal/2026-10-05.MD" };
    });
    await createOrOpen(h, "Journal/2026-10-05.md", "");
    expect(opened).toEqual([["Journal/2026-10-05.MD", false]]);
  });

  it("says so, and opens nothing, when a folder is in the way or the write fails", async () => {
    const folder = host(async () => {
      throw { kind: "alreadyExists", detail: "journal" };
    });
    await createOrOpen(folder.h, "Journal/2026-10-05.md", "");
    expect(folder.opened).toEqual([]);
    expect(folder.toasts).toEqual(['Could not create Journal/2026-10-05.md: Something named "journal" already exists.']);

    const io = host(async () => {
      throw { kind: "io", detail: "No space left on device" };
    });
    await createOrOpen(io.h, "a.md", "");
    expect(io.opened).toEqual([]);
    expect(io.toasts).toEqual(["Could not create a.md: No space left on device"]);
  });
});
