// Unit reproduction for FINDING-068 (plugins.ts start() race).
// Fake Worker + mocked backend, plain Node. The FINDING-068 tests are
// regression tests that assert the correct behaviour.
//
// Run: cd app && npx vitest run src/lib/adv_verify_pl_05_host.test.ts

import { describe, it, expect, vi, beforeEach } from "vitest";

let readConfigDelay = 0;
const config = new Map<string, string>();
vi.mock("./backend", () => ({
  backend: {
    listConfig: async () => [...config.keys()].map((k) => k.replace(/^plugins\//, "")),
    // Every plugin file counts as turned on on this device.
    pluginApprovals: () => approveAll(config),
    readConfig: async (name: string) => {
      if (readConfigDelay) await new Promise((r) => setTimeout(r, readConfigDelay));
      return config.get(name) ?? null;
    },
    readNote: async () => ({ content: "", hash: "h" }),
    writeNote: async () => ({}),
    createNote: async () => ({}),
  },
}));

import { PluginHost } from "./plugins";
import { approveAll } from "./plugins.testutil";

class FakeWorker {
  static all: FakeWorker[] = [];
  terminated = false;
  onmessage: unknown = null;
  onerror: unknown = null;
  constructor() {
    FakeWorker.all.push(this);
  }
  postMessage() {}
  terminate() {
    this.terminated = true;
  }
}

const host = () =>
  new PluginHost({ notePaths: () => [], activePath: () => null, getSelection: () => null, replaceSelection: () => false, toast: () => {} });

beforeEach(() => {
  FakeWorker.all = [];
  readConfigDelay = 1; // one IPC round trip
  config.clear();
  config.set("plugins/a.js", "// @name A\n// @permissions write\n");
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
});

describe("FINDING-068", () => {
  it("FINDING-068: Reload twice in the same tick (stopAll+sync, stopAll+sync) leaves a worker that neither sync([]) nor stopAll() can reach", async () => {
    const h = host();
    await h.sync(["a.js"]); // enabled
    // reloadPlugins() x2, as SettingsModal.svelte does
    h.stopAll();
    const p1 = h.sync(["a.js"]);
    h.stopAll();
    const p2 = h.sync(["a.js"]);
    await Promise.all([p1, p2]);
    await h.sync([]); // disable
    h.stopAll(); // closeVault / openVault of the next vault
    expect(FakeWorker.all.filter((w) => !w.terminated)).toHaveLength(0);
  });

  it("control: a second Reload after the first start finished (one readConfig later) is handled correctly", async () => {
    const h = host();
    await h.sync(["a.js"]);
    h.stopAll();
    await h.sync(["a.js"]);
    h.stopAll();
    await h.sync(["a.js"]);
    await h.sync([]);
    expect(FakeWorker.all.filter((w) => !w.terminated)).toHaveLength(0);
  });

  it("stopAll() during an in-flight start cancels that start: no worker is created", async () => {
    const h = host();
    const p = h.sync(["a.js"]);
    h.stopAll();
    await p;
    expect(FakeWorker.all).toHaveLength(0);
    // A later sync() starts it normally.
    await h.sync(["a.js"]);
    expect(FakeWorker.all.filter((w) => !w.terminated)).toHaveLength(1);
  });
});
