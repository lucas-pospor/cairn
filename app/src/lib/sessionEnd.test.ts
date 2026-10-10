// On Windows the page keeps the backend up to date with what it has not
// saved, so that the backend can save it, or name it, when Windows ends the
// session and the page cannot answer (held.ts; app/src-tauri/src/held.rs and
// session_end.rs). These tests run the app with the Windows gate on and a
// fake backend that records the requests.
//
// Run: cd app && npx vitest run src/lib/sessionEnd.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import type { HeldRequest } from "./backend";

const READ_ONLY = { kind: "io", detail: '"A.md" is read-only.' };
const CONFLICT = { kind: "conflict", detail: "A.md" };

/** The requests to session_hold, as sent. */
let holds: HeldRequest[] = [];
/** Tabs open when each request was sent. */
let tabsAtHold: number[] = [];
let holdReply = { floor: 0, applied: true };
let holdFails = 0;
/** What writeNote fails with, by call; null when it works. */
let writeFails: (object | null)[] = [];
let writes: { path: string; content: string; baseHash: string | null; edit?: number }[] = [];
/** Runs during a write, before it answers. */
let duringWrite: (() => void) | null = null;
let moved: string | null = null;
let disk = { content: "on disk\n", hash: "B0" };
let configFails: object | null = null;
let deleteFails: object | null = null;
/** Resolves the next session_hold when set. */
let holdGate: Promise<void> | null = null;

vi.mock("./platform", async (original) => ({ ...(await original<typeof import("./platform")>()), isWindows: true }));

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `vault://localhost/${p}`,
  backend: {
    sessionHold: async (hold: HeldRequest) => {
      holds.push(JSON.parse(JSON.stringify(hold)));
      tabsAtHold.push(app.tabs.length);
      if (holdGate) await holdGate;
      if (holdFails > 0) {
        holdFails--;
        throw new Error("no");
      }
      return holdReply;
    },
    sessionMoved: async () => moved,
    writeNote: async (path: string, content: string, baseHash: string | null, _inVault: boolean, edit?: number) => {
      writes.push({ path, content, baseHash, edit });
      duringWrite?.();
      duringWrite = null;
      const fail = writeFails.shift();
      if (fail) throw fail;
      disk = { content, hash: `H${writes.length}` };
      return { hash: disk.hash, entry: { path, kind: "file", size: content.length, mtime: 0 }, changes: [] };
    },
    readNote: async () => disk,
    deleteEntry: async () => {
      if (deleteFails) throw deleteFails;
      return [];
    },
    mergeText: async () => null,
    rescan: async () => [],
    writeConfig: async () => {
      if (configFails) throw configFails;
    },
  },
}));

import { app, Tab } from "./app.svelte";
import { settings } from "./settings.svelte";
import { TEXT_LIMIT } from "./held";

/** A note open in a tab, saved, as loadTab leaves it. */
function openNote(path = "A.md", doc = "on disk\n", base = "B0"): Tab {
  const tab = new Tab(path);
  tab.loading = false;
  tab.baseHash = base;
  tab.baseText = doc;
  tab.editorState = EditorState.create({ doc });
  tab.edit = 1;
  app.tabs = [...app.tabs, tab];
  app.activeId = tab.id;
  return tab;
}

/** Type `text` at the end of `tab`'s note, as the editor reports it. */
function type(tab: Tab, text: string) {
  app.viewTab = tab;
  const st = tab.editorState!;
  tab.editorState = st.update({ changes: { from: st.doc.length, insert: text } }).state;
  app["onEdit"]();
}

/** Let queued requests go out and come back. */
async function settle() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await app["heldQueue"];
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

/** The notes of the last request. */
const lastNotes = () => holds.at(-1)?.notes ?? [];

beforeEach(() => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  holds = [];
  tabsAtHold = [];
  holdReply = { floor: 0, applied: true };
  holdFails = 0;
  writeFails = [];
  writes = [];
  duringWrite = null;
  moved = null;
  disk = { content: "on disk\n", hash: "B0" };
  configFails = null;
  deleteFails = null;
  holdGate = null;
  app.vault = { root: "/vault", name: "vault", entries: [] };
  app.entries = [];
  app.tabs = [];
  app.activeId = null;
  app.viewTab = null;
  app.dialog = null;
  app.toasts = [];
  app["closeConfirmed"] = false;
  app["settingsDiscarded"] = false;
  app["refusal"] = null;
  // As after the page's reset.
  app["heldSent"] = new Map();
  app["heldSettings"] = false;
});

afterEach(async () => {
  // No autosave of these tabs runs into the next test.
  for (const t of app.tabs) clearTimeout(t.saveTimer);
  clearTimeout(app["heldTimer"]);
  vi.useRealTimers();
  await settle();
  settings["dirty"] = false;
  settings["failed"] = false;
  vi.unstubAllGlobals();
});

describe("what the page holds", () => {
  it("sends a note's text at its first edit, then after typing stops", async () => {
    vi.useFakeTimers();
    const tab = openNote();
    type(tab, "x");
    await settle();
    expect(holds).toHaveLength(1);
    expect(holds[0]).toMatchObject({ root: "/vault", settingsFailed: false });
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: null, text: "on disk\nx" }]);
    type(tab, "y");
    await vi.advanceTimersByTimeAsync(150);
    type(tab, "z");
    await vi.advanceTimersByTimeAsync(150);
    expect(holds).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(60);
    await settle();
    expect(holds).toHaveLength(2);
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: null, text: "on disk\nxyz" }]);
    expect(holds[1].seq).toBeGreaterThan(holds[0].seq);
  });

  it("sends the text at least once a second while typing goes on", async () => {
    vi.useFakeTimers();
    const tab = openNote();
    type(tab, "a");
    await settle();
    for (let i = 0; i < 12; i++) {
      await vi.advanceTimersByTimeAsync(100);
      type(tab, "b");
    }
    await settle();
    // The first edit, then one at about 1 s.
    expect(holds).toHaveLength(2);
    expect((lastNotes()[0] as { text: string }).text).toMatch(/^on disk\nab{9,10}$/);
  });

  it("lets a note go once a save wrote it, and saves send their edit number", async () => {
    const tab = openNote();
    type(tab, "x");
    await settle();
    await app.save(tab);
    await settle();
    expect(writes).toEqual([{ path: "A.md", content: "on disk\nx", baseHash: "B0", edit: tab.edit }]);
    expect(lastNotes()).toEqual([{ path: "A.md", release: true }]);
  });

  it("holds a note whose save failed with its text, and one in conflict without it", async () => {
    const tab = openNote();
    type(tab, "x");
    await settle();
    writeFails = [READ_ONLY];
    await app.save(tab);
    await settle();
    // The backend has the text of this edit already.
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: "failed" }]);
    type(tab, "z");
    await app["heldSend"]();
    await settle();
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: "failed", text: "on disk\nxz" }]);
    // The file changed on disk and the changes cannot be merged.
    writeFails = [CONFLICT];
    disk = { content: "theirs\n", hash: "B1" };
    type(tab, "y");
    await app.save(tab);
    await settle();
    expect(tab.conflict).toBe("changed");
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: "conflict" }]);
  });

  it("holds two tabs with edits for one file as a conflict", async () => {
    const a = openNote();
    const b = openNote("A.md", "on disk\n", "B0");
    type(a, "x");
    type(b, "y");
    await settle();
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: b.edit, problem: "conflict" }]);
  });

  it("holds a note too large, or not well formed, without its text", async () => {
    const big = openNote("Big.md", "x".repeat(TEXT_LIMIT));
    type(big, "y");
    const odd = openNote("Odd.md", "a\uD800b");
    type(odd, "c");
    await settle();
    const notes = holds.flatMap((h) => h.notes);
    expect(notes).toContainEqual({ path: "Big.md", base: "B0", edit: big.edit, problem: null, text: null });
    expect(notes).toContainEqual({ path: "Odd.md", base: "B0", edit: odd.edit, problem: null, text: null });
  });

  it("lets edits the user discards go before the tab closes", async () => {
    const tab = openNote();
    type(tab, "x");
    writeFails = [READ_ONLY, READ_ONLY];
    await app.save(tab);
    await settle();
    const closing = app.closeTab(tab);
    await vi.waitFor(() => expect(app.dialog?.kind).toBe("confirm"));
    const d = app.dialog;
    if (d?.kind === "confirm") d.resolve(true);
    app.dialog = null;
    await closing;
    await settle();
    const release = holds.findIndex((h) => h.notes.some((n) => "release" in n));
    expect(release).toBeGreaterThan(-1);
    // Sent while the tab was still open.
    expect(tabsAtHold[release]).toBe(1);
    expect(app.tabs).toEqual([]);
  });

  it("holds discarded edits again when the delete they were discarded for fails", async () => {
    app.entries = [{ path: "A.md", kind: "file", size: 1, mtime: 0 }];
    const tab = openNote();
    type(tab, "x");
    writeFails = [READ_ONLY, READ_ONLY];
    await app.save(tab);
    await settle();
    deleteFails = { kind: "io", detail: "denied" };
    const removing = app.remove("A.md");
    // "Delete", then "Discard".
    for (let i = 0; i < 2; i++) {
      await vi.waitFor(() => expect(app.dialog?.kind).toBe("confirm"));
      const d = app.dialog;
      if (d?.kind === "confirm") d.resolve(true);
      app.dialog = null;
    }
    await removing;
    await settle();
    expect(app.tabs).toEqual([tab]);
    expect(tab.discarded).toBe(false);
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: "failed", text: "on disk\nx" }]);
  });

  it("does not take a request sent before a reset for what the backend holds after it", async () => {
    const tab = openNote();
    let open!: () => void;
    holdGate = new Promise((r) => (open = r));
    type(tab, "x");
    for (let i = 0; i < 10; i++) await Promise.resolve();
    // The request is on its way when the notebook closes.
    app["heldReset"]();
    holdGate = null;
    open();
    await settle();
    expect(holds.at(-1)).toMatchObject({ reset: true });
  });

  it("starts over at a reset, and keeps its numbers above the backend's floor", async () => {
    holdReply = { floor: 9e15, applied: true };
    const tab = openNote();
    type(tab, "x");
    await settle();
    app["heldReset"]();
    await settle();
    expect(holds.at(-1)).toMatchObject({ reset: true, root: "/vault" });
    // Everything again after a reset.
    expect(lastNotes()).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: null, text: "on disk\nx" }]);
    type(tab, "y");
    expect(tab.edit).toBeGreaterThan(9e15);
    app.heldSync();
    await settle();
    expect(holds.at(-1)!.seq).toBeGreaterThan(9e15);
  });

  it("sends everything again after a request failed", async () => {
    vi.useFakeTimers();
    holdFails = 1;
    const tab = openNote();
    type(tab, "x");
    // No autosave in between.
    clearTimeout(tab.saveTimer);
    await settle();
    expect(holds).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1000);
    await settle();
    expect(holds).toHaveLength(2);
    expect(holds[1]).toMatchObject({ reset: true });
    expect(holds[1].notes).toEqual([{ path: "A.md", base: "B0", edit: tab.edit, problem: null, text: "on disk\nx" }]);
  });

  it("does not count a request the backend did not take", async () => {
    holdReply = { floor: 0, applied: false };
    const tab = openNote();
    type(tab, "x");
    await settle();
    holdReply = { floor: 0, applied: true };
    app.heldSync();
    await settle();
    expect(holds).toHaveLength(2);
    expect(holds[1].notes).toEqual(holds[0].notes);
  });

  it("holds the settings while their write fails, until a write works or they are discarded", async () => {
    settings.onWriteResult = () => app.heldSync();
    configFails = { kind: "io", detail: "disk full" };
    settings["dirty"] = true;
    await settings.flush().catch(() => {});
    await settle();
    expect(holds.at(-1)).toMatchObject({ settingsFailed: true });
    configFails = null;
    await settings.flush();
    await settle();
    expect(holds.at(-1)).toMatchObject({ settingsFailed: false });
    // A failed write the user discarded with the notebook is let go too.
    configFails = { kind: "io", detail: "disk full" };
    settings["dirty"] = true;
    await settings.flush().catch(() => {});
    await settle();
    expect(holds.at(-1)).toMatchObject({ settingsFailed: true });
    app["settingsDiscarded"] = true;
    app.heldSync();
    await settle();
    expect(holds.at(-1)).toMatchObject({ settingsFailed: false });
    settings.onWriteResult = () => {};
  });

  it("lets everything go once the window may close", async () => {
    const tab = openNote();
    type(tab, "x");
    await settle();
    app["closeConfirmed"] = true;
    app.heldSync();
    await settle();
    expect(lastNotes()).toEqual([{ path: "A.md", release: true }]);
  });

  it("sends nothing for image tabs or notes still loading", async () => {
    const img = new Tab("pic.png", "image");
    const loading = new Tab("B.md");
    loading.dirty = true;
    app.tabs = [img, loading];
    app.heldSync();
    await settle();
    expect(holds).toEqual([]);
  });
});

describe("the end of the session", () => {
  it("answers the backend's question at once with every note, whatever its size", () => {
    const tab = openNote();
    type(tab, "x");
    const big = openNote("Big.md", "x".repeat(TEXT_LIMIT));
    type(big, "y");
    holds = [];
    app.answerSessionEnd(4);
    // Sent before anything is awaited.
    expect(holds).toHaveLength(1);
    expect(holds[0]).toMatchObject({ round: 4 });
    expect(holds[0].notes).toContainEqual({ path: "A.md", base: "B0", edit: tab.edit, problem: null, text: "on disk\nx" });
    const sentBig = holds[0].notes.find((n) => n.path === "Big.md") as { text: string };
    expect(sentBig.text).toHaveLength(TEXT_LIMIT + 1);
  });

  it("answers with the text of a note whose save failed, whatever its size, and none for a conflict", async () => {
    const big = openNote("Big.md", "x".repeat(TEXT_LIMIT));
    type(big, "y");
    writeFails = [READ_ONLY];
    await app.save(big);
    const clash = openNote("C.md");
    type(clash, "c");
    writeFails = [CONFLICT];
    disk = { content: "theirs\n", hash: "B1" };
    await app.save(clash);
    await settle();
    expect([big.saveFailed, clash.conflict]).toEqual([true, "changed"]);
    holds = [];
    app.answerSessionEnd(5);
    const sentBig = holds[0].notes.find((n) => n.path === "Big.md") as { problem: string; text: string };
    expect(sentBig.problem).toBe("failed");
    expect(sentBig.text).toHaveLength(TEXT_LIMIT + 1);
    expect(holds[0].notes).toContainEqual({ path: "C.md", base: "B0", edit: clash.edit, problem: "conflict" });
  });

  it("saves a note the backend wrote again over the new file, once the session goes on", async () => {
    const tab = openNote();
    type(tab, "x");
    await settle();
    type(tab, "y");
    app.sessionEndNews({ written: [{ path: "A.md", from: "B0", to: "H9", edit: tab.edit - 1 }], refused: [] });
    await settle();
    expect(writes).toEqual([{ path: "A.md", content: "on disk\nxy", baseHash: "H9", edit: tab.edit }]);
    expect(tab.dirty).toBe(false);
  });

  it("leaves a tab alone when the backend wrote another file than its own", async () => {
    const tab = openNote();
    type(tab, "x");
    app.sessionEndNews({ written: [{ path: "A.md", from: "Bx", to: "H9", edit: 1 }], refused: [] });
    await settle();
    expect(tab.baseHash).toBe("B0");
    expect(writes).toEqual([]);
  });

  it("saves again over the file the backend wrote when a save finds it changed", async () => {
    const tab = openNote();
    type(tab, "x");
    writeFails = [CONFLICT];
    moved = "H9";
    await app.save(tab);
    await settle();
    expect(writes.map((w) => w.baseHash)).toEqual(["B0", "H9"]);
    expect(tab.conflict).toBe(null);
    expect(tab.dirty).toBe(false);
  });

  it("saves again, without a merge, when the news came during the save", async () => {
    const tab = openNote();
    type(tab, "x");
    writeFails = [CONFLICT];
    duringWrite = () => app.sessionEndNews({ written: [{ path: "A.md", from: "B0", to: "H9", edit: tab.edit }], refused: [] });
    await app.save(tab);
    await settle();
    expect(writes.map((w) => w.baseHash).slice(0, 2)).toEqual(["B0", "H9"]);
    expect(tab.conflict).toBe(null);
  });

  it("after a no, shows a notice that names what is not saved until it is", async () => {
    const tab = openNote();
    openNote("B.md");
    type(tab, "x");
    writeFails = [READ_ONLY];
    await app.save(tab);
    await settle();
    app.sessionEndNews({ written: [], refused: [{ path: "A.md", name: '"A"' }] });
    await settle();
    expect(app.active).toBe(tab);
    expect(app.toasts.map((t) => t.message)).toContain(
      'Windows was about to sign out or shut down, but changes to "A" are not saved. Save or discard them, then try again.',
    );
    await app.save(tab);
    await settle();
    expect(tab.dirty).toBe(false);
    expect(app.toasts.map((t) => t.message).filter((m) => m.startsWith("Windows was"))).toEqual([]);
  });

  it("keeps the notice while a note it names is renamed", async () => {
    const tab = openNote();
    type(tab, "x");
    writeFails = [READ_ONLY];
    await app.save(tab);
    await settle();
    app.sessionEndNews({ written: [], refused: [{ path: "A.md", name: '"A"' }] });
    app["handleChanges"]([{ type: "renamed", from: "A.md", entry: { path: "B.md", kind: "file", size: 1, mtime: 0 } }]);
    await settle();
    expect(tab.path).toBe("B.md");
    expect(app.toasts.map((t) => t.message).filter((m) => m.startsWith("Windows was"))).toHaveLength(1);
  });

  it("goes back to the merge after a few saves over files the backend wrote", async () => {
    const tab = openNote();
    type(tab, "x");
    writeFails = [CONFLICT, CONFLICT, CONFLICT, CONFLICT, CONFLICT];
    moved = "H9";
    disk = { content: "theirs\n", hash: "B1" };
    await app.save(tab);
    await settle();
    await vi.waitFor(() => expect(tab.conflict).toBe("changed"));
    expect(writes.length).toBeLessThanOrEqual(4);
  });

  it("keeps the notice past five other toasts", () => {
    app.sessionEndNews({ written: [], refused: [{ path: null, name: "the settings" }] });
    for (let i = 0; i < 6; i++) app.toast(`toast ${i}`);
    expect(app.toasts.map((t) => t.message)[0]).toMatch(/^Windows was about to sign out/);
    expect(app.toasts).toHaveLength(5);
  });
});
