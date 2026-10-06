// Regression tests for FINDING-069 (a plugin command timeout timer keyed by
// command id would leak a timer when runs of the same command overlap).
//
// The controls show the timeout is otherwise fine: one run, two
// non-overlapping runs, and two different commands overlapping do not stop
// the plugin, and a command that never finishes still does. The FINDING-069
// case covers overlapping runs of the same command.
//
// Run: cd app && npx vitest run src/lib/adv_verify_pl_06.test.ts

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

vi.mock("./backend", () => {
  const backend = {
    listConfig: async () => ["p.js"],
    // Every plugin file counts as turned on on this device.
    pluginApprovals: () => approveAll(new Map([["plugins/p.js", "// @name P\n"]])),
    readConfig: async (name: string) => (name === "plugins/p.js" ? "// @name P\n" : null),
    readNote: async () => ({ content: "", hash: "h" }),
    writeNote: async () => ({}),
    createNote: async () => ({}),
  };
  return { backend };
});

import { PluginHost } from "./plugins";
import { approveAll } from "./plugins.testutil";
import { commands } from "./commands";

class FakeWorker {
  static all: FakeWorker[] = [];
  sent: { type: string; id: string; run?: number }[] = [];
  terminated = false;
  onmessage: ((e: { data: unknown }) => unknown) | null = null;
  onerror: ((e: { message: string }) => unknown) | null = null;
  constructor() {
    FakeWorker.all.push(this);
  }
  postMessage(m: { type: string; id: string; run?: number }) {
    // The real bootstrap answers the host's pings (the host stops a plugin that does not).
    if (m.type === "ping") return void this.emit({ type: "pong" });
    this.sent.push(m);
  }
  terminate() {
    this.terminated = true;
  }
  async emit(data: unknown) {
    await this.onmessage?.({ data });
  }
}

const toasts: string[] = [];
// The run ids the host gave its run-command messages, oldest first; the real worker sends each back with command-done.
const runIds = (w: FakeWorker) => w.sent.filter((m) => m.type === "run-command").map((m) => m.run);
const stoppedToasts = () => toasts.filter((t) => /took too long/.test(t));

async function setup(ids: string[]) {
  const host = new PluginHost({
    notePaths: () => [],
    activePath: () => null,
    getSelection: () => "",
    replaceSelection: () => true,
    toast: (m: string) => toasts.push(m),
  } as never);
  await host.sync(["p.js"]);
  const w = FakeWorker.all[0];
  for (const id of ids) await w.emit({ type: "register-command", id, name: id.toUpperCase() });
  return { host, w };
}

beforeEach(() => {
  vi.useFakeTimers();
  toasts.length = 0;
  FakeWorker.all = [];
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
  commands.unregisterPrefix("plugin:");
});

afterEach(() => {
  commands.unregisterPrefix("plugin:");
  vi.useRealTimers();
});

describe("FINDING-069 controls", () => {
  it("one quick run does not stop the plugin", async () => {
    const { w } = await setup(["work"]);
    commands.run("plugin:p.js:work");
    vi.advanceTimersByTime(1500);
    await w.emit({ type: "command-done", id: "work", run: runIds(w)[0] });
    vi.advanceTimersByTime(60_000);
    expect(stoppedToasts()).toEqual([]);
    expect(w.terminated).toBe(false);
  });

  it("two runs of the same command that do not overlap do not stop the plugin", async () => {
    const { w } = await setup(["work"]);
    commands.run("plugin:p.js:work");
    vi.advanceTimersByTime(1500);
    await w.emit({ type: "command-done", id: "work", run: runIds(w)[0] });
    commands.run("plugin:p.js:work");
    vi.advanceTimersByTime(1500);
    await w.emit({ type: "command-done", id: "work", run: runIds(w)[1] });
    vi.advanceTimersByTime(60_000);
    expect(stoppedToasts()).toEqual([]);
    expect(w.terminated).toBe(false);
  });

  it("two different commands overlapping do not stop the plugin", async () => {
    const { w } = await setup(["a", "b"]);
    commands.run("plugin:p.js:a");
    vi.advanceTimersByTime(300);
    commands.run("plugin:p.js:b");
    vi.advanceTimersByTime(1200);
    await w.emit({ type: "command-done", id: "a", run: runIds(w)[0] });
    vi.advanceTimersByTime(300);
    await w.emit({ type: "command-done", id: "b", run: runIds(w)[1] });
    vi.advanceTimersByTime(60_000);
    expect(stoppedToasts()).toEqual([]);
    expect(w.terminated).toBe(false);
  });

  it("a command that never finishes still stops the plugin after 30 s", async () => {
    const { w } = await setup(["hang"]);
    commands.run("plugin:p.js:hang");
    vi.advanceTimersByTime(29_000);
    expect(w.terminated).toBe(false);
    vi.advanceTimersByTime(2_000);
    expect(w.terminated).toBe(true);
    expect(stoppedToasts()).toEqual(["Plugin P took too long and was stopped."]);
  });
});

describe("FINDING-069 regression", () => {
  it("FINDING-069: two overlapping runs of the same command, both finishing in 1.8 s, do not stop the plugin", async () => {
    const { w } = await setup(["work"]);
    commands.run("plugin:p.js:work"); // t = 0
    vi.advanceTimersByTime(300);
    commands.run("plugin:p.js:work"); // t = 0.3 s, first run still going
    vi.advanceTimersByTime(1200);
    await w.emit({ type: "command-done", id: "work", run: runIds(w)[0] }); // t = 1.5 s, first run done
    vi.advanceTimersByTime(300);
    await w.emit({ type: "command-done", id: "work", run: runIds(w)[1] }); // t = 1.8 s, second run done
    vi.advanceTimersByTime(29_000); // t = 30.8 s
    expect(stoppedToasts()).toEqual([]);
    expect(w.terminated).toBe(false);
    expect(commands.all().some((c) => c.id === "plugin:p.js:work")).toBe(true);
  });
});
