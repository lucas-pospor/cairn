// A page reload (the Reload item of WebKitGTK's right-click menu, a reload
// key an old WebView2 runtime still takes, a dev server) asks first while a
// note holds edits that could not be saved (a failed save or a conflict),
// or a change of the settings could not be written.
// Edits still waiting for autosave do not count, as the page going hidden
// saves them, and closing the window, which asks on its own, does not ask
// twice. App.svelte hands its beforeunload event to app.beforeUnload;
// e2e/adv_dataloss_tabs.test.mjs checks that wiring in the app.
//
// Run: cd app && npx vitest run src/lib/beforeUnload.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";

const READ_ONLY = { kind: "io", detail: '"A.md" is read-only.' };
const CONFLICT = { kind: "conflict", detail: "A.md" };
/** What writeNote fails with; null when it works. */
let failWith: object | null = null;
/** What writing settings.json fails with; null when it works. */
let configFailsWith: object | null = null;
let disk = "";

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `vault://localhost/${p}`,
  backend: {
    writeNote: async (_path: string, content: string) => {
      if (failWith) throw failWith;
      disk = content;
      return { hash: "saved" };
    },
    readNote: async () => ({ content: disk, hash: "disk" }),
    writeConfig: async () => {
      if (configFailsWith) throw configFailsWith;
    },
  },
}));

import { app, Tab } from "./app.svelte";
import { settings } from "./settings.svelte";

const store = new Map<string, string>();

/** Hand a beforeunload event to the app as App.svelte does; true when the web view would ask. */
function asks(): boolean {
  const e = {
    prevented: false,
    returnValue: undefined as unknown,
    preventDefault() {
      this.prevented = true;
    },
  };
  app.beforeUnload(e);
  // Both ways of asking, or neither: older engines look at returnValue only.
  expect(Boolean(e.returnValue)).toBe(e.prevented);
  return e.prevented;
}

/** An open note with an edit that waits for autosave. */
function editedNote(): Tab {
  const tab = new Tab("A.md");
  tab.loading = false;
  tab.baseHash = "disk";
  tab.editorState = EditorState.create({ doc: "on disk\nmine" });
  tab.dirty = true;
  app.tabs = [tab];
  app.activeId = tab.id;
  return tab;
}

/** Close the window and answer "Unsaved changes" with Discard (true) or Cancel. */
async function closeWindow(discard: boolean) {
  const closing = app.beforeClose();
  await vi.waitFor(() => expect(app.dialog?.kind).toBe("confirm"));
  const d = app.dialog;
  if (d?.kind === "confirm") d.resolve(discard);
  app.dialog = null;
  return closing;
}

beforeEach(() => {
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  });
  store.clear();
  failWith = null;
  configFailsWith = null;
  disk = "on disk\n";
  app.vault = { root: "/vault", name: "vault", entries: [] };
  app.entries = [];
  app.tabs = [];
  app.activeId = null;
  app.dialog = null;
  app.toasts = [];
  app["closeConfirmed"] = false;
});

afterEach(() => {
  settings["dirty"] = false;
  settings["failed"] = false;
  vi.unstubAllGlobals();
});

describe("reloading the page", () => {
  it("does not ask for edits that wait for autosave, and keeps the session", () => {
    editedNote();
    expect(asks()).toBe(false);
    expect(JSON.parse(store.get("cairn.session:/vault") ?? "null")).toMatchObject({ tabs: [{ path: "A.md", kind: "note" }], active: "A.md" });
  });

  it("asks while a save failed, and no longer once a save works", async () => {
    const tab = editedNote();
    failWith = READ_ONLY;
    await app.save(tab);
    expect(tab.saveFailed).toBe(true);
    expect(app.toasts.map((t) => t.message)).toEqual(['Could not save A: "A.md" is read-only.']);
    expect(asks()).toBe(true);
    failWith = null;
    await app.save(tab);
    expect(tab.saveFailed).toBe(false);
    expect(tab.dirty).toBe(false);
    expect(disk).toBe("on disk\nmine");
    expect(asks()).toBe(false);
  });

  it("asks while a note is in conflict with the disk", async () => {
    const tab = editedNote();
    failWith = CONFLICT;
    await app.save(tab);
    expect(tab.conflict).toBe("changed");
    expect(tab.saveFailed).toBe(false);
    expect(asks()).toBe(true);
  });

  it("does not ask once the user let the window close with Discard", async () => {
    const tab = editedNote();
    failWith = READ_ONLY;
    await app.save(tab);
    expect(await closeWindow(true)).toBe(true);
    expect(tab.dirty).toBe(true);
    expect(asks()).toBe(false);
  });

  it("still asks when the user kept the window open with Cancel", async () => {
    const tab = editedNote();
    failWith = READ_ONLY;
    await app.save(tab);
    expect(await closeWindow(false)).toBe(false);
    expect(asks()).toBe(true);
  });

  it("asks while a change of the settings could not be written, and no longer once a write works", async () => {
    // A change that waits for its debounce does not count: the page going hidden writes it.
    settings["dirty"] = true;
    expect(asks()).toBe(false);
    configFailsWith = { kind: "io", detail: "There is not enough space on the disk. (os error 112)" };
    await expect(settings.flush()).rejects.toBeTruthy();
    expect(settings.saveFailed).toBe(true);
    expect(asks()).toBe(true);
    configFailsWith = null;
    await settings.flush();
    expect(settings.saveFailed).toBe(false);
    expect(asks()).toBe(false);
  });

  it("does not ask at the next edit after a failed save, a conflict and Load disk version", async () => {
    const tab = editedNote();
    failWith = READ_ONLY;
    await app.save(tab);
    failWith = CONFLICT;
    await app.save(tab);
    expect(tab.conflict).toBe("changed");
    expect(asks()).toBe(true);
    await app.loadTheirs(tab);
    expect(tab.dirty).toBe(false);
    expect(tab.conflict).toBeNull();
    expect(app.docOf(tab)).toBe("on disk\n");
    // A keystroke sets dirty and waits for autosave.
    tab.dirty = true;
    expect(asks()).toBe(false);
  });
});
