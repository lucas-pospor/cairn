// Image tabs on the desktop: which files open where from the tree, the
// quick switcher, search, links and embeds (all go through openNote,
// openLink or openEmbeddedImage), how the tab follows changes on disk, and
// that the session never stores one. The Android routes are in
// imageTabs_android.test.ts.
//
// Run: cd app && npx vitest run src/lib/imageTabs.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Change, FileStat } from "./types";

const calls: string[] = [];
const links = new Map<string, string>();
/** What list_entries returns (default: the entries as they are). */
let disk: FileStat[] | null = null;

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `vault://localhost/${p}`,
  backend: {
    readNote: async (path: string) => {
      calls.push(`readNote ${path}`);
      return { content: `# ${path}\n`, hash: `h ${path}` };
    },
    openExternally: async (path: string) => void calls.push(`openExternally ${path}`),
    resolveLink: async (target: string, _source: string, kind: string) => {
      calls.push(`resolveLink ${target} ${kind}`);
      return links.get(target) ?? null;
    },
    createNote: async (path: string) => void calls.push(`createNote ${path}`),
    listEntries: async () => disk ?? app.entries,
  },
}));

import { app, Tab } from "./app.svelte";

const file = (path: string): FileStat => ({ path, kind: "file", size: 10, mtime: 1 });
const ENTRIES: FileStat[] = [
  file("A.md"),
  file("B.md"),
  { path: "img", kind: "dir", size: 0, mtime: 1 },
  file("img/pic.png"),
  file("img/other.svg"),
  file("doc.pdf"),
];
const store = new Map<string, string>();
const kinds = () => app.tabs.map((t) => `${t.kind} ${t.path}`);
const changes = (cs: Change[]) => app["handleChanges"](cs);

beforeEach(() => {
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  });
  store.clear();
  calls.length = 0;
  links.clear();
  disk = null;
  app.tabs = [];
  app.activeId = null;
  app.expanded.clear();
  app.vault = { root: "/vault", name: "vault", entries: ENTRIES };
  app.entries = ENTRIES;
  app.toasts = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("opening files on the desktop", () => {
  it("opens an image in an image tab, without reading it as a note or opening another app", async () => {
    await app.openNote("img/pic.png");
    expect(kinds()).toEqual(["image img/pic.png"]);
    expect(app.active?.kind).toBe("image");
    expect(app.active?.title).toBe("pic.png");
    expect(app.active?.loading).toBe(false);
    expect(calls).toEqual([]);
  });

  it("opens every image type in an image tab", async () => {
    for (const ext of ["png", "jpg", "JPEG", "gif", "webp", "avif", "svg", "bmp", "ico"]) {
      await app.openNote(`x.${ext}`, { newTab: true });
    }
    expect(app.tabs.every((t) => t.kind === "image")).toBe(true);
    expect(app.tabs).toHaveLength(9);
    expect(calls).toEqual([]);
  });

  it("opens other attachments in the default app, as before", async () => {
    await app.openNote("doc.pdf");
    await app.openNote("doc.pdf", { newTab: true });
    expect(app.tabs).toEqual([]);
    expect(calls).toEqual(["openExternally doc.pdf", "openExternally doc.pdf"]);
  });

  it("opens notes in a note tab", async () => {
    await app.openNote("A.md");
    expect(kinds()).toEqual(["note A.md"]);
    expect(calls).toEqual(["readNote A.md"]);
  });

  it("follows a wikilink or a Markdown link to an image into an image tab", async () => {
    links.set("pic.png", "img/pic.png");
    links.set("img/other.svg", "img/other.svg");
    await app.openNote("A.md");
    await app.openLink("pic.png", null);
    expect(kinds()).toEqual(["note A.md", "image img/pic.png"]);
    await app.openLink("img/other.svg", null, false, "markdown");
    expect(kinds()).toEqual(["note A.md", "image img/other.svg"]);
    expect(calls).toEqual(["readNote A.md", "resolveLink pic.png wiki", "resolveLink img/other.svg markdown"]);
  });

  it("still opens a link to another attachment in the default app", async () => {
    links.set("doc.pdf", "doc.pdf");
    await app.openLink("doc.pdf", null);
    expect(calls).toEqual(["resolveLink doc.pdf wiki", "openExternally doc.pdf"]);
  });

  it("opens an embedded image only when it is an image of the vault", async () => {
    // The path comes from the rendered note, where raw HTML can set it.
    app.openEmbeddedImage("doc.pdf");
    app.openEmbeddedImage("missing.png");
    app.openEmbeddedImage("img");
    app.openEmbeddedImage("../outside.png");
    expect(app.tabs).toEqual([]);
    app.openEmbeddedImage("img/pic.png");
    expect(kinds()).toEqual(["image img/pic.png"]);
    expect(calls).toEqual([]);
  });
});

describe("which tab an image opens in", () => {
  it("never replaces a note tab: a new tab opens next to it", async () => {
    await app.openNote("A.md");
    await app.openNote("img/pic.png");
    expect(kinds()).toEqual(["note A.md", "image img/pic.png"]);
  });

  it("replaces the active image tab unless asked for a new tab", async () => {
    await app.openNote("img/pic.png");
    const first = app.active;
    first!.actualSize = true;
    await app.openNote("img/other.svg");
    expect(kinds()).toEqual(["image img/other.svg"]);
    expect(app.active).toBe(first);
    // Fitted again for the new image.
    expect(app.active?.actualSize).toBe(false);
    await app.openNote("img/pic.png", { newTab: true });
    expect(kinds()).toEqual(["image img/other.svg", "image img/pic.png"]);
  });

  it("goes back to the tab that already shows the image", async () => {
    await app.openNote("img/pic.png");
    const tab = app.active;
    await app.openNote("A.md");
    await app.openNote("img/pic.png", { newTab: true });
    expect(kinds()).toEqual(["image img/pic.png", "note A.md"]);
    expect(app.active).toBe(tab);
  });

  it("does not let a note replace the active image tab", async () => {
    await app.openNote("img/pic.png");
    await app.openNote("A.md");
    expect(kinds()).toEqual(["image img/pic.png", "note A.md"]);
  });

  it("asks the image view for the focus each time", async () => {
    const before = app.imageFocus;
    await app.openNote("img/pic.png");
    await app.openNote("img/pic.png");
    expect(app.imageFocus).toBe(before + 2);
  });
});

describe("an image tab and changes on disk", () => {
  it("loads the file from a new URL after each change, without reading it as a note", async () => {
    await app.openNote("img/pic.png");
    const urls = [app.imageSrc("img/pic.png")];
    expect(urls[0]).toMatch(/^vault:\/\/localhost\/img\/pic\.png\?v=\d+\.0$/);
    changes([{ type: "modified", entry: file("img/pic.png") }]);
    // Replaced by another file moved over it, or created again.
    urls.push(app.imageSrc("img/pic.png"));
    changes([{ type: "renamed", from: "tmp.png", entry: file("img/pic.png") }]);
    urls.push(app.imageSrc("img/pic.png"));
    changes([{ type: "created", entry: file("img/pic.png") }]);
    urls.push(app.imageSrc("img/pic.png"));
    expect(new Set(urls).size).toBe(4);
    // Another file changing does not count.
    const other = app.imageSrc("img/other.svg");
    changes([{ type: "modified", entry: file("img/pic.png") }]);
    expect(app.imageSrc("img/other.svg")).toBe(other);
    expect(calls).toEqual([]);
  });

  it("uses a new URL for a file that changed while no tab showed it (the web view keeps the old picture by URL)", async () => {
    await app.openNote("img/pic.png");
    const shown = app.imageSrc("img/pic.png");
    await app.closeTab(app.active!);
    changes([{ type: "modified", entry: file("img/pic.png") }]);
    await app.openNote("img/pic.png");
    expect(app.imageSrc("img/pic.png")).not.toBe(shown);
  });

  it("uses a new URL after a folder move, also for a path that was shown before (no per-file change is reported)", async () => {
    await app.openNote("img/pic.png");
    const urls = [app.imageSrc("img/pic.png")];
    changes([{ type: "renamed", from: "img", entry: { path: "media", kind: "dir", size: 0, mtime: 1 } }]);
    urls.push(app.imageSrc("media/pic.png"));
    changes([{ type: "modified", entry: file("media/pic.png") }]);
    urls.push(app.imageSrc("media/pic.png"));
    changes([{ type: "renamed", from: "media", entry: { path: "img", kind: "dir", size: 0, mtime: 1 } }]);
    urls.push(app.imageSrc("img/pic.png"));
    // A folder created over a path that was shown before (restored, or synced).
    changes([{ type: "deleted", path: "img", kind: "dir" }]);
    changes([{ type: "created", entry: { path: "img", kind: "dir", size: 0, mtime: 1 } }]);
    urls.push(app.imageSrc("img/pic.png"));
    expect(new Set(urls).size).toBe(urls.length);
  });

  it("closes when the file is renamed to a name that is not an image's", async () => {
    await app.openNote("A.md");
    await app.openNote("img/pic.png");
    changes([{ type: "renamed", from: "img/pic.png", entry: file("img/pic.txt") }]);
    expect(kinds()).toEqual(["note A.md"]);
  });

  it("shows the new size of a changed image: the file list is read again", async () => {
    await app.openNote("img/pic.png");
    disk = ENTRIES.map((e) => (e.path === "img/pic.png" ? { ...e, size: 999999 } : e));
    changes([{ type: "modified", entry: { ...file("img/pic.png"), size: 999999 } }]);
    await vi.waitFor(() => expect(app.entries.find((e) => e.path === "img/pic.png")?.size).toBe(999999));
  });

  it("follows a rename and a move of the file and of its folder", async () => {
    await app.openNote("img/pic.png");
    const tab = app.active!;
    changes([{ type: "renamed", from: "img/pic.png", entry: file("img/photo.png") }]);
    expect(tab.path).toBe("img/photo.png");
    expect(tab.title).toBe("photo.png");
    changes([{ type: "renamed", from: "img", entry: { path: "media/img", kind: "dir", size: 0, mtime: 1 } }]);
    expect(tab.path).toBe("media/img/photo.png");
    expect(app.active).toBe(tab);
  });

  it("closes when the file or its folder is deleted, as a note tab does", async () => {
    await app.openNote("A.md");
    await app.openNote("img/pic.png");
    await app.openNote("img/other.svg", { newTab: true });
    changes([{ type: "deleted", path: "img/pic.png", kind: "file" }]);
    expect(kinds()).toEqual(["note A.md", "image img/other.svg"]);
    changes([{ type: "deleted", path: "img", kind: "dir" }]);
    expect(kinds()).toEqual(["note A.md"]);
    expect(app.active?.path).toBe("A.md");
  });

  it("leaves a note tab's reload as it was", async () => {
    await app.openNote("A.md");
    calls.length = 0;
    changes([{ type: "modified", entry: file("A.md") }]);
    await vi.waitFor(() => expect(calls).toEqual(["readNote A.md"]));
  });
});

describe("the session", () => {
  const saved = () => JSON.parse(store.get("cairn.session:/vault")!);

  it("keeps notes and the graph but no image tab, in the format of 1.0.0", async () => {
    await app.openNote("A.md");
    await app.openNote("img/pic.png");
    app.openGraph();
    await app.openNote("B.md", { newTab: true });
    app.expanded.add("img");
    app.saveSession();
    expect(saved()).toEqual({
      tabs: [
        { path: "A.md", mode: "live", kind: "note" },
        { path: "", mode: "live", kind: "graph" },
        { path: "B.md", mode: "live", kind: "note" },
      ],
      active: "B.md",
      expanded: ["img"],
    });
  });

  it("names no tab as active while an image tab is (the first tab opens next time)", async () => {
    await app.openNote("A.md");
    await app.openNote("img/pic.png");
    app.saveSession();
    expect(saved()).toEqual({ tabs: [{ path: "A.md", mode: "live", kind: "note" }], active: null, expanded: [] });
    expect(store.get("cairn.session:/vault")).not.toContain("pic.png");
  });

  it("restores no tab for an image, and no tab of a kind it does not know", async () => {
    store.set(
      "cairn.session:/vault",
      JSON.stringify({
        tabs: [
          // A note tab whose note was renamed to an image name, saved by any version.
          { path: "img/pic.png", mode: "live", kind: "note" },
          { path: "A.md", mode: "source" },
          { path: "img/other.svg", mode: "live", kind: "image" },
          { path: "doc.pdf", mode: "live", kind: "pdf" },
          { path: "B.md", mode: "live", kind: "note" },
        ],
        active: "img/pic.png",
        expanded: [],
      }),
    );
    app["restoreSession"]();
    expect(kinds()).toEqual(["note A.md", "note B.md"]);
    expect(app.active?.path).toBe("A.md");
    await vi.waitFor(() => expect(calls.sort()).toEqual(["readNote A.md", "readNote B.md"]));
  });
});

describe("Tab", () => {
  it("an image tab is never loading, dirty or in conflict", () => {
    const t = new Tab("img/pic.png", "image");
    expect([t.loading, t.dirty, t.conflict, t.actualSize]).toEqual([false, false, null, false]);
  });
});
