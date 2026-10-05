// Reproduction for FINDING-021 (plugin consent must not be bound to the file
// name only). Consent is an approval of the file's hash and permissions on
// this device (the mocked pluginApprovals below), so a changed file does not
// start at all.
//
// Run: cd app && npx vitest run src/lib/adv_verify_pl_02.test.ts

import { describe, it, expect, vi, beforeEach } from "vitest";

const files = new Map<string, string>();
const config = new Map<string, string>();
const approvals = new Map<string, PluginApproval>(); // what this device approved

vi.mock("./backend", () => ({
  backend: {
    listConfig: async (dir: string) =>
      [...config.keys()].filter((k) => k.startsWith(dir + "/") && !k.slice(dir.length + 1).includes("/")).map((k) => k.slice(dir.length + 1)),
    readConfig: async (name: string) => config.get(name) ?? null,
    pluginApprovals: async () => Object.fromEntries(approvals),
    setPluginApproval: async (file: string, approval: PluginApproval | null) => {
      if (approval) approvals.set(file, approval);
      else approvals.delete(file);
    },
    readNote: async (path: string) => {
      if (!files.has(path)) throw { kind: "notFound", detail: path };
      return { content: files.get(path)!, hash: "h" };
    },
    writeNote: async (path: string, content: string) => {
      files.set(path, content);
      return { entry: { path }, hash: "h", changes: [] };
    },
    createNote: async (path: string, content: string) => {
      files.set(path, content);
      return { entry: { path }, hash: "h", changes: [] };
    },
  },
}));

import { PluginHost, type PluginApproval } from "./plugins";

class FakeWorker {
  static all: FakeWorker[] = [];
  sent: { type: string; id?: number; value?: unknown; error?: string }[] = [];
  onmessage: ((e: { data: unknown }) => unknown) | null = null;
  onerror: unknown = null;
  constructor(
    public url: string,
    public opts: unknown,
  ) {
    FakeWorker.all.push(this);
  }
  postMessage(m: never) {
    // The real bootstrap answers the host's pings (the host stops a plugin that does not).
    if ((m as { type: string }).type === "ping") return void this.emit({ type: "pong" });
    this.sent.push(m);
  }
  terminate() {}
  async emit(data: unknown) {
    await this.onmessage?.({ data });
  }
}

const host = () =>
  new PluginHost({ notePaths: () => [...files.keys()], activePath: () => null, getSelection: () => "", replaceSelection: () => true, toast: () => {} });

beforeEach(() => {
  files.clear();
  config.clear();
  approvals.clear();
  FakeWorker.all = [];
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
});

describe("FINDING-021", () => {
  it("FINDING-021: a plugin approved for 'editor' only gets read+write after its file changes and plugins are reloaded", async () => {
    config.set("plugins/wordcount.js", "// @name Word count\n// @permissions editor\n");
    files.set("Secret.md", "bank pin 0000\n");
    const h = host();
    const [shown] = await h.available();
    expect(shown.permissions).toEqual(["editor"]); // what the consent dialog showed
    await h.approve(shown); // the user clicked Enable
    await h.sync(["wordcount.js"]);
    await FakeWorker.all[0].emit({ type: "call", id: 1, method: "notes.read", args: ["Secret.md"] });
    expect(FakeWorker.all[0].sent[0].error).toMatch(/permission/); // refused before the change, as it should be
    // The file is replaced on disk, then "Reload plugins" (or app restart / vault reopen).
    config.set("plugins/wordcount.js", "// @name Word count\n// @permissions editor read write\n");
    h.stopAll();
    // Expected: neither permission was ever approved, so the changed file does not run at all.
    expect(await h.sync(["wordcount.js"])).toEqual(["wordcount.js"]);
    expect(FakeWorker.all.length).toBe(1); // not a second worker that reads Secret.md into Leak.md
    expect(files.has("Leak.md")).toBe(false);
  });

  it("FINDING-021: the permissions shown in the consent dialog are not the ones the started plugin gets (file changes between dialog and start)", async () => {
    config.set("plugins/p.js", "// @name P\n// @permissions editor\n");
    files.set("Secret.md", "s3cret");
    const h = host();
    const [shown] = await h.available(); // SettingsModal parses this when the Plugins screen opens
    expect(shown.permissions).toEqual(["editor"]);
    config.set("plugins/p.js", "// @name P\n// @permissions read\n"); // changes while the dialog is open
    await h.approve(shown); // user clicked Enable on the "editor" dialog
    expect(await h.sync(["p.js"])).toEqual(["p.js"]);
    expect(FakeWorker.all).toEqual([]); // not started with "read", which would read Secret.md
  });
});
