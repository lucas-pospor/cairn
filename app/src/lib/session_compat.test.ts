// Image tabs and the saved session (localStorage "cairn.session:<vault>").
// Image tabs are not saved: Cairn 1.0.0 and 1.1.0 open every saved tab that
// is not the graph as a note, so an image there would come back as a note
// tab that cannot load. The first test checks what this version saves while
// image tabs are open; the "older versions" tests restore exactly that and
// use only what 1.0.0 already has (openVault, tabs, active, the session key),
// so they also run against the app.svelte.ts of v1.0.0 and v1.1.0:
//
//   git worktree add --detach ../cairn-v1.1.0 v1.1.0
//   ln -s "$PWD/app/node_modules" ../cairn-v1.1.0/app/node_modules
//   cp app/src/lib/session_compat.test.ts ../cairn-v1.1.0/app/src/lib/
//   (cd ../cairn-v1.1.0/app && npx vitest run src/lib/session_compat.test.ts -t "older versions")
//
// Run: cd app && npx vitest run src/lib/session_compat.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FileStat } from "./types";

const reads: string[] = [];

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `vault://localhost/${p}`,
  backend: {
    openVault: async (root: string) => ({ root, name: "vault", entries: ENTRIES }),
    readConfig: async () => null,
    listConfig: async () => [],
    writeConfig: async () => {},
    pluginApprovals: async () => ({}),
    syncStatus: async () => null,
    recentVaults: async () => [],
    readNote: async (path: string) => {
      reads.push(path);
      return { content: `# ${path}\n`, hash: `h ${path}` };
    },
    openExternally: async () => {},
  },
}));

import { app } from "./app.svelte";

const file = (path: string): FileStat => ({ path, kind: "file", size: 10, mtime: 1 });
const ENTRIES: FileStat[] = [file("A.md"), file("B.md"), { path: "img", kind: "dir", size: 0, mtime: 1 }, file("img/pic.png"), file("img/logo.svg")];
const KEY = "cairn.session:/vault";

// What this version saves with notes A and B (in source mode), the graph and
// two image tabs open, in this order: A, pic.png, graph, B, logo.svg.
const IMAGE_ACTIVE = {
  tabs: [
    { path: "A.md", mode: "live", kind: "note" },
    { path: "", mode: "live", kind: "graph" },
    { path: "B.md", mode: "source", kind: "note" },
  ],
  active: null,
  expanded: ["img"],
};
const NOTE_ACTIVE = { ...IMAGE_ACTIVE, active: "B.md" };

const store = new Map<string, string>();

beforeEach(() => {
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  });
  // Just enough of a page for the settings to apply.
  vi.stubGlobal("document", {
    documentElement: { style: { setProperty() {}, removeProperty() {} }, removeAttribute() {}, dataset: {} },
    querySelectorAll: () => [],
    querySelector: () => null,
  });
  store.clear();
  reads.length = 0;
  app.toasts = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function openVault() {
  await app.openVault("/vault");
  expect(app.toasts.map((t) => t.message)).toEqual([]);
}

describe("a session saved while image tabs are open", () => {
  it("leaves the image tabs out and keeps the format of 1.0.0", async () => {
    await openVault();
    await app.openNote("A.md");
    await app.openNote("img/pic.png");
    app.openGraph();
    await app.openNote("B.md", { newTab: true });
    app.setMode("source");
    await app.openNote("img/logo.svg");
    app.expanded.add("img");
    expect(app.tabs.map((t) => t.kind)).toEqual(["note", "image", "graph", "note", "image"]);
    app.saveSession();
    expect(JSON.parse(store.get(KEY)!)).toEqual(IMAGE_ACTIVE);
    await app.openNote("B.md");
    app.saveSession();
    expect(JSON.parse(store.get(KEY)!)).toEqual(NOTE_ACTIVE);
    expect(store.get(KEY)).not.toMatch(/png|svg|image/);
  });
});

describe("image URLs", () => {
  it("change when a vault opens: the same path in another vault is another file", async () => {
    await openVault();
    const first = app.imageSrc("img/pic.png");
    await app.openVault("/other");
    expect(app.imageSrc("img/pic.png")).not.toBe(first);
  });
});

describe("older versions restoring that session", () => {
  async function restore(session: object) {
    store.set(KEY, JSON.stringify(session));
    await openVault();
    // Every note tab loads.
    await vi.waitFor(() => expect(app.tabs.every((t) => !t.loading)).toBe(true));
    return app.tabs.map((t) => ({ kind: t.kind, path: t.path, mode: t.mode, error: t.error }));
  }

  it("restore every other tab, in order and with its mode, and open the first one", async () => {
    expect(await restore(IMAGE_ACTIVE)).toEqual([
      { kind: "note", path: "A.md", mode: "live", error: null },
      { kind: "graph", path: "", mode: "live", error: null },
      { kind: "note", path: "B.md", mode: "source", error: null },
    ]);
    expect(app.active?.path).toBe("A.md");
    expect([...app.expanded]).toEqual(["img"]);
    expect(reads.sort()).toEqual(["A.md", "B.md"]);
  });

  it("open the note that was open when a note was active", async () => {
    expect((await restore(NOTE_ACTIVE)).map((t) => t.path)).toEqual(["A.md", "", "B.md"]);
    expect(app.active?.path).toBe("B.md");
    expect(reads.sort()).toEqual(["A.md", "B.md"]);
  });
});
