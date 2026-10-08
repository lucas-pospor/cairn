// Image tabs on Android: images open in a tab like on the desktop; other
// attachments keep the toast, since Android cannot hand files to other apps
// yet.
//
// Run: cd app && npx vitest run src/lib/imageTabs_android.test.ts

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileStat } from "./types";

const calls: string[] = [];

vi.mock("./platform", () => ({
  isAndroid: true,
  isMobile: true,
  narrowQuery: () => null,
  vaultLabel: (root: string) => root,
}));

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `http://vault.localhost/${p}`,
  backend: {
    readNote: async (path: string) => {
      calls.push(`readNote ${path}`);
      return { content: "", hash: "h" };
    },
    openExternally: async (path: string) => void calls.push(`openExternally ${path}`),
    resolveLink: async (target: string) => target,
  },
}));

import { app } from "./app.svelte";

const file = (path: string): FileStat => ({ path, kind: "file", size: 10, mtime: 1 });

beforeEach(() => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  calls.length = 0;
  app.tabs = [];
  app.activeId = null;
  app.vault = { root: "content://tree/Notes", name: "Notes", entries: [] };
  app.entries = [file("pic.png"), file("doc.pdf"), file("A.md")];
  app.toasts = [];
});

describe("opening files on Android", () => {
  it("opens an image in an image tab, without a toast", async () => {
    const focus = app.imageFocus;
    await app.openNote("pic.png");
    await app.openLink("pic.png", null);
    app.openEmbeddedImage("pic.png");
    expect(app.tabs.map((t) => `${t.kind} ${t.path}`)).toEqual(["image pic.png"]);
    // Each of the three showed it.
    expect(app.imageFocus).toBe(focus + 3);
    expect(app.toasts).toEqual([]);
    expect(calls).toEqual([]);
  });

  it("closes the drawers on a small screen to show the image", async () => {
    app.narrow = true;
    app.leftOpen = app.rightOpen = true;
    try {
      await app.openNote("pic.png");
      expect([app.leftOpen, app.rightOpen]).toEqual([false, false]);
    } finally {
      app.narrow = false;
    }
  });

  it("keeps the toast for other attachments and never calls the default app", async () => {
    const toast = "doc.pdf is an attachment. On Android, Cairn cannot open attachments in other apps yet.";
    await app.openNote("doc.pdf");
    expect(app.toasts.map((t) => t.message)).toEqual([toast]);
    app.toasts = [];
    await app.openLink("doc.pdf", null);
    expect(app.toasts.map((t) => t.message)).toEqual([toast]);
    expect(app.tabs).toEqual([]);
    expect(calls).toEqual([]);
  });

  it("opens notes as before", async () => {
    await app.openNote("A.md");
    expect(app.tabs.map((t) => `${t.kind} ${t.path}`)).toEqual(["note A.md"]);
    expect(calls).toEqual(["readNote A.md"]);
  });
});
