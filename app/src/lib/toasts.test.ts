// Toasts: the same timed message again while it is up shows once, such as
// "Could not save" from a save that keeps failing; toasts that stay until
// closed are never merged.
//
// Run: cd app && npx vitest run src/lib/toasts.test.ts

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `vault://localhost/${p}`,
  backend: {
    writeNote: async () => {
      throw { kind: "io", detail: '"A.md" is read-only.' };
    },
  },
}));

import { app, Tab } from "./app.svelte";

const SAVE_FAILED = 'Could not save A: "A.md" is read-only.';
const messages = () => app.toasts.map((t) => t.message);

/** An open note whose saves fail. */
function readOnlyNote(): Tab {
  const tab = new Tab("A.md");
  tab.loading = false;
  tab.baseHash = "disk";
  tab.editorState = EditorState.create({ doc: "mine" });
  tab.dirty = true;
  app.tabs = [tab];
  app.activeId = tab.id;
  return tab;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  app.vault = { root: "/vault", name: "vault", entries: [] };
  app.tabs = [];
  app.activeId = null;
  app.toasts = [];
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("toasts", () => {
  it("show a save that keeps failing once, for as long as it keeps failing", async () => {
    const tab = readOnlyNote();
    await app.save(tab);
    await app.save(tab);
    expect(messages()).toEqual([SAVE_FAILED]);
    // A failure 6 s later keeps the toast up 7 s from then.
    await vi.advanceTimersByTimeAsync(6000);
    await app.save(tab);
    await vi.advanceTimersByTimeAsync(6000);
    expect(messages()).toEqual([SAVE_FAILED]);
    await vi.advanceTimersByTimeAsync(1000);
    expect(messages()).toEqual([]);
  });

  it("show other messages, and the same text of another kind, side by side", () => {
    app.toast("One", "error");
    app.toast("Two", "error");
    app.toast("One", "info");
    expect(app.toasts.map((t) => `${t.kind}: ${t.message}`)).toEqual(["error: One", "error: Two", "info: One"]);
  });

  it("never merge toasts that stay until they are closed", () => {
    const first = app.toast("Running…", "info", 0);
    const second = app.toast("Running…", "info", 0);
    expect(messages()).toEqual(["Running…", "Running…"]);
    first();
    expect(messages()).toEqual(["Running…"]);
    second();
    expect(messages()).toEqual([]);
  });

  it("leave a toast held under the pointer held when the same message comes again", async () => {
    app.toast("Held", "error");
    const id = app.toasts[0].id;
    app.holdToast(id);
    app.toast("Held", "error");
    await vi.advanceTimersByTimeAsync(20000);
    expect(messages()).toEqual(["Held"]);
    app.releaseToast(id);
    await vi.advanceTimersByTimeAsync(7000);
    expect(messages()).toEqual([]);
  });

  it("give the caller of a merged toast a way to close it", () => {
    app.toast("Again", "info");
    const close = app.toast("Again", "info");
    close();
    expect(messages()).toEqual([]);
  });
});
