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
    const refused = { kind: "io", detail: 'The ".cairn" folder leads outside the notebook.' };
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

describe("the font file (textFont)", () => {
  /** Inline styles set on <html>, faces in document.fonts, and what the store reported. */
  let style: Record<string, string>;
  let faces: Set<FakeFace>;
  let written: string[];
  let reads: string[];
  let errors: [string, string][];
  let changes: number;
  /** The files in .cairn/; a function delays a read until it is called. */
  let files: Record<string, Uint8Array | (() => Promise<Uint8Array>)>;

  const font = (head: string) => {
    const b = new Uint8Array(64);
    b.set([...head].map((c) => c.charCodeAt(0)));
    return b;
  };
  const WOFF2 = font("wOF2");
  const SERIF = 'Charter, "Iowan Old Style", "Source Serif 4", Georgia, "Noto Serif", serif';

  /** Called to finish loading a font whose fifth byte is 1 (a slow one). */
  let finishSlowFace: (() => void) | null;

  /** A FontFace that loads bytes starting with wOF2 and rejects others ("broken" ones too). */
  class FakeFace {
    constructor(
      readonly family: string,
      readonly data: ArrayBuffer,
    ) {}
    load() {
      const bytes = new Uint8Array(this.data);
      if (String.fromCharCode(...bytes.subarray(0, 4)) !== "wOF2") return Promise.reject(new Error("The font data is not valid."));
      if (bytes[4] === 1) return new Promise<this>((resolve) => (finishSlowFace = () => resolve(this)));
      return Promise.resolve(this);
    }
  }

  beforeEach(() => {
    style = {};
    faces = new Set();
    written = [];
    reads = [];
    errors = [];
    changes = 0;
    files = {};
    finishSlowFace = null;
    vi.stubGlobal("FontFace", FakeFace);
    vi.stubGlobal("document", {
      documentElement: { style: { setProperty: (k: string, v: string) => void (style[k] = v), removeProperty() {} }, removeAttribute() {}, dataset: {} },
      querySelectorAll: () => [],
      fonts: { add: (f: FakeFace) => void faces.add(f), delete: (f: FakeFace) => faces.delete(f) },
    });
    vi.spyOn(console, "warn").mockImplementation(() => {});
    settings.onFontError = (name, message) => void errors.push([name, message]);
    settings.onFontChange = () => void changes++;
  });

  afterEach(() => {
    settings.reset();
    settings.onFontError = () => {};
    settings.onFontChange = () => {};
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function load(file: unknown) {
    Object.assign(backend, {
      readConfig: async () => JSON.stringify(file),
      listConfig: async () => [],
      writeConfig: async (_name: string, content: string) => void written.push(content),
      readConfigBytes: async (name: string, max: number) => {
        reads.push(`${name} ${max}`);
        const f = files[name];
        if (!f) throw { kind: "notFound", detail: name };
        const bytes = typeof f === "function" ? await f() : f;
        return bytes.slice().buffer;
      },
    });
    await settings.load();
  }

  /** Wait until the font file is no longer loading. */
  const settled = () => vi.waitFor(() => expect(settings.font?.state).not.toBe("loading"));

  async function saved() {
    await settings.flush();
    return JSON.parse(written.at(-1)!);
  }

  it("uses the font file once it is loaded, with the Text font behind it", async () => {
    files["fonts/Inter.woff2"] = WOFF2;
    await load({ fontFamily: "serif", textFont: "Inter.woff2" });
    await settled();
    expect(settings.font).toEqual({ name: "Inter.woff2", state: "loaded" });
    expect(reads).toEqual([`fonts/Inter.woff2 ${20 * 1024 * 1024}`]);
    const [face] = faces;
    expect(style["--font-text"]).toBe(`"${face.family}", ${SERIF}`);
    expect(changes).toBe(1);
    expect(errors).toEqual([]);
  });

  it("uses the Text font, and says why, when the file is missing, is not a font or does not load", async () => {
    files["fonts/Fake.ttf"] = font("%PDF");
    files["fonts/Broken.woff"] = font("wOFF");
    const cases: [string, string][] = [
      ["Missing.woff2", "It is not in .cairn/fonts/."],
      ["Fake.ttf", "This is not a woff2, woff, ttf or otf font file."],
      ["Broken.woff", "The web view cannot read the font in this file."],
      ["Inter.ttc", "Cairn takes woff2, woff, ttf and otf font files."],
      ["../Inter.ttf", "A font file name cannot contain slashes or start with a dot."],
    ];
    for (const [name, why] of cases) {
      errors = [];
      await load({ fontFamily: "mono", textFont: name });
      await settled();
      expect(settings.font, name).toEqual({ name, state: "failed", error: why });
      expect(errors, name).toEqual([[name, why]]);
      expect(style["--font-text"], name).toBe("var(--font-mono)");
      expect(faces.size, name).toBe(0);
    }
    // A name that is not a font file is never read.
    expect(reads.map((r) => r.split(" ")[0])).toEqual(["fonts/Missing.woff2", "fonts/Fake.ttf", "fonts/Broken.woff"]);
  });

  it("is written only once chosen, and a value it cannot use stays in the file", async () => {
    await load({ fontSize: 15 });
    settings.update({ fontSize: 17 });
    expect("textFont" in (await saved())).toBe(false);
    expect(settings.font).toBeNull();
    files["fonts/A.woff2"] = WOFF2;
    settings.update({ textFont: "A.woff2" });
    expect(await saved()).toMatchObject({ textFont: "A.woff2", fontSize: 17 });
    for (const bad of [5, null, true, ["A.woff2"], { file: "A.woff2" }]) {
      written = [];
      await load({ textFont: bad });
      expect(settings.font, String(bad)).toBeNull();
      expect(style["--font-text"]).toBe("var(--font-ui)");
      settings.update({ fontSize: 18 });
      expect((await saved()).textFont).toEqual(bad);
    }
    expect(reads.map((r) => r.split(" ")[0])).toEqual(["fonts/A.woff2"]);
  });

  it("loads only when the setting changes or it is asked to, and lets the font go when the setting, the vault or the app leaves it", async () => {
    files["fonts/A.woff2"] = WOFF2;
    files["fonts/B.woff2"] = WOFF2;
    await load({ textFont: "A.woff2" });
    await settled();
    const [a] = faces;
    settings.update({ fontSize: 20, fontFamily: "serif" });
    expect(reads).toHaveLength(1);
    expect(style["--font-text"]).toBe(`"${a.family}", ${SERIF}`);
    // The same name again, after the file was replaced.
    await settings.reloadFont();
    expect(reads).toHaveLength(2);
    expect([...faces].map((f) => f === a)).toEqual([false]);
    // Another file.
    settings.update({ textFont: "B.woff2" });
    await vi.waitFor(() => expect(reads).toHaveLength(3));
    await settled();
    expect(faces.size).toBe(1);
    // No font file: the Text font, and the editor measures again.
    const before = changes;
    settings.update({ textFont: undefined });
    expect(faces.size).toBe(0);
    expect(style["--font-text"]).toBe(SERIF);
    expect(changes).toBe(before + 1);
    expect("textFont" in (await saved())).toBe(false);
    // Another vault with a file of the same name loads its own; leaving the vault lets it go.
    await load({ textFont: "A.woff2" });
    await settled();
    expect(reads).toHaveLength(4);
    settings.reset();
    expect(faces.size).toBe(0);
    expect(settings.font).toBeNull();
    expect(style["--font-text"]).toBe("var(--font-ui)");
  });

  it("drops a font that finishes loading after the setting changed", async () => {
    const slow = font("wOF2");
    slow[4] = 1;
    files["fonts/Slow.woff2"] = slow;
    files["fonts/Fast.woff2"] = WOFF2;
    await load({ textFont: "Slow.woff2" });
    await vi.waitFor(() => expect(finishSlowFace).toBeTypeOf("function"));
    settings.update({ textFont: "Fast.woff2" });
    await settled();
    finishSlowFace!();
    await new Promise((r) => setTimeout(r, 0));
    expect(settings.font).toEqual({ name: "Fast.woff2", state: "loaded" });
    expect(faces.size).toBe(1);
    expect([...faces][0].data.byteLength).toBe(WOFF2.length);
    expect(new Uint8Array([...faces][0].data)[4]).toBe(0);
  });

  it("drops a read that finishes after the setting changed", async () => {
    let finish!: () => void;
    files["fonts/Slow.woff2"] = () => new Promise((resolve) => (finish = () => resolve(WOFF2)));
    files["fonts/Fast.woff2"] = WOFF2;
    await load({ textFont: "Slow.woff2" });
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    settings.update({ textFont: "Fast.woff2" });
    await settled();
    finish();
    await new Promise((r) => setTimeout(r, 0));
    expect(settings.font).toEqual({ name: "Fast.woff2", state: "loaded" });
    expect(faces.size).toBe(1);
    expect(style["--font-text"]).toBe(`"${[...faces][0].family}", var(--font-ui)`);
  });
});
