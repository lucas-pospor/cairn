// The Random note core plugin (app/src/lib/corePlugins/randomNote.ts).
//
// Run: cd app && npx vitest run src/lib/corePlugins/randomNote.test.ts

import { describe, expect, it, vi } from "vitest";

vi.mock("../backend", () => ({ backend: {} }));

const { pluginOn } = await import("./core");
const { randomNote } = await import("./randomNote");
type CoreHost = import("./core").CoreHost;

function fakeHost(files: string[], open: string | null, random: number) {
  const opened: string[] = [];
  const toasts: string[] = [];
  const host = {
    files: () => files,
    activeNote: () => open,
    openNote: async (p: string) => void opened.push(p),
    toast: (m: string) => void toasts.push(m),
    random: () => random,
  } as unknown as CoreHost;
  return { host, opened, toasts };
}

const openRandom = (host: CoreHost) => randomNote.commands[0].run(host);

describe("Open random note", () => {
  it("is off by default and has no options", () => {
    expect(pluginOn(randomNote)).toBe(false);
    expect(randomNote.options).toEqual([]);
  });

  it("picks among the notes, from first to last", async () => {
    const files = ["b.md", "a.md", "c/d.md", "pic.png"];
    const picks = [];
    for (const r of [0, 0.34, 0.67, 0.999999]) {
      const f = fakeHost(files, null, r);
      await openRandom(f.host);
      picks.push(...f.opened);
    }
    // Sorted by path; attachments are not notes.
    expect(picks).toEqual(["a.md", "b.md", "c/d.md", "c/d.md"]);
  });

  it("does not pick the note that is open, unless it is the only one", async () => {
    for (const r of [0, 0.5, 0.99]) {
      const f = fakeHost(["a.md", "b.md"], "a.md", r);
      await openRandom(f.host);
      expect(f.opened).toEqual(["b.md"]);
    }
    const only = fakeHost(["a.md"], "a.md", 0.3);
    await openRandom(only.host);
    expect(only.opened).toEqual(["a.md"]);
  });

  it("says so when the vault has no notes", async () => {
    const f = fakeHost(["pic.png"], null, 0.5);
    await openRandom(f.host);
    expect(f.opened).toEqual([]);
    expect(f.toasts).toEqual(["There are no notes in this vault."]);
  });
});
