// Reveal in file manager: a path the system's file manager cannot show (on
// Windows a share path it refuses, or one longer than about 260 characters)
// gives an error toast instead of nothing, from the file tree's menu and from
// the command palette alike.
//
// Run: cd app && npx vitest run src/lib/reveal.test.ts

import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreError } from "./types";

const revealed: string[] = [];
/** What revealInFileManager throws, if anything. */
let fail: CoreError | null = null;

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `vault://localhost/${p}`,
  backend: {
    readNote: async (path: string) => ({ content: `# ${path}\n`, hash: `h ${path}` }),
    resolveLink: async () => null,
    revealInFileManager: async (path: string) => {
      revealed.push(path);
      if (fail) throw fail;
    },
  },
}));

import { app } from "./app.svelte";
import { commands } from "./commands";

const TOO_LONG = "File Explorer cannot show this path. It may be too long, or have a name that File Explorer cannot handle.";

const toasts = () => app.toasts.map((t) => [t.kind, t.message]);

beforeEach(() => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  revealed.length = 0;
  fail = null;
  app.tabs = [];
  app.activeId = null;
  app.vault = { root: "/notes", name: "notes", entries: [] };
  app.entries = [{ path: "notes/a.md", kind: "file", size: 10, mtime: 1 }];
  app.toasts = [];
});

describe("reveal in file manager", () => {
  it("shows no toast when the file manager shows the file", async () => {
    await app.revealInFileManager("notes/a.md");
    expect(revealed).toEqual(["notes/a.md"]);
    expect(app.toasts).toEqual([]);
  });

  it("says why when the file manager cannot show it", async () => {
    fail = { kind: "io", detail: TOO_LONG };
    await app.revealInFileManager("notes/a.md");
    expect(toasts()).toEqual([["error", `Could not show a.md in the file manager: ${TOO_LONG}`]]);
  });

  it("says why from the command palette too", async () => {
    app["registerCommands"]();
    await app.openNote("notes/a.md");
    const reveal = commands.get("note:reveal")!;
    expect(reveal.available?.()).toBe(true);
    fail = { kind: "io", detail: TOO_LONG };
    reveal.run();
    await vi.waitFor(() => expect(app.toasts.length).toBe(1));
    expect(revealed).toEqual(["notes/a.md"]);
    expect(toasts()).toEqual([["error", `Could not show a.md in the file manager: ${TOO_LONG}`]]);
  });

  it("goes through the app from the file tree's menu", () => {
    // The menu action would otherwise call the backend directly, with no
    // toast when it fails.
    const tree = readFileSync(new URL("./components/FileTree.svelte", import.meta.url), "utf8");
    expect(tree).toContain("app.revealInFileManager(node.path)");
    expect(tree).not.toMatch(/backend\.revealInFileManager/);
  });
});
