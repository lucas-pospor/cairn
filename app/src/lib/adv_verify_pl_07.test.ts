// Reproduction for FINDING-070 (a command timer left over from a stopped
// plugin instance must not stop the next instance of the same plugin file).
//
// If the 30 s "took too long" timers lived in a Map local to one start() call,
// stop() did not clear them, and the timer callback called this.stop(file) by
// file name, then whatever instance runs under that file name 30 s after the
// first run would be terminated, even if it never ran anything.
//
// Every test below is a regression test that asserts the correct behaviour.
//
// Run: cd app && npx vitest run src/lib/adv_verify_pl_07.test.ts

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const config = new Map<string, string>();

vi.mock("./backend", () => ({
  backend: {
    listConfig: async (dir: string) =>
      [...config.keys()].filter((k) => k.startsWith(dir + "/") && !k.slice(dir.length + 1).includes("/")).map((k) => k.slice(dir.length + 1)),
    // Every plugin file counts as turned on on this device.
    pluginApprovals: () => approveAll(config),
    readConfig: async (name: string) => config.get(name) ?? null,
    readNote: async () => {
      throw { kind: "notFound" };
    },
    writeNote: async () => ({}),
    createNote: async () => ({}),
  },
}));

import { PluginHost } from "./plugins";
import { approveAll } from "./plugins.testutil";
import { commands } from "./commands";

class FakeWorker {
  static all: FakeWorker[] = [];
  terminated = false;
  onmessage: ((e: { data: unknown }) => unknown) | null = null;
  onerror: unknown = null;
  constructor(
    public url: string,
    public opts: unknown,
  ) {
    FakeWorker.all.push(this);
  }
  postMessage(m: { type: string }) {
    // The real bootstrap answers the host's pings (the host stops a plugin that does not).
    if (m.type === "ping") void this.emit({ type: "pong" });
  }
  terminate() {
    this.terminated = true;
  }
  async emit(data: unknown) {
    await this.onmessage?.({ data });
  }
}

const toasts: string[] = [];
const makeHost = () =>
  new PluginHost({
    notePaths: () => [],
    activePath: () => null,
    getSelection: () => "",
    replaceSelection: () => true,
    toast: (m) => toasts.push(m),
  });
const pluginCmds = () =>
  commands
    .all()
    .filter((c) => c.id.startsWith("plugin:"))
    .map((c) => c.id);

/** Start a.js, register one command, run it (it never answers). */
async function startAndHang(host: PluginHost) {
  await host.sync(["a.js"]);
  const w = FakeWorker.all.at(-1)!;
  await w.emit({ type: "register-command", id: "slow", name: "Slow" });
  commands.run("plugin:a.js:slow");
  return w;
}

beforeEach(() => {
  config.clear();
  toasts.length = 0;
  FakeWorker.all = [];
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
  commands.unregisterPrefix("plugin:");
  vi.useFakeTimers();
  config.set("plugins/a.js", "// @name A\n");
});

afterEach(() => vi.useRealTimers());

describe("FINDING-070", () => {
  it("FINDING-070: 'Reload plugins' after a hung command does not help: the fresh instance is stopped 30 s after the old run", async () => {
    const host = makeHost();
    const w1 = await startAndHang(host);
    vi.advanceTimersByTime(10_000); // the user waits a bit, then presses Reload plugins
    host.stopAll(); // SettingsModal.reloadPlugins(): stopAll() + sync()
    await host.sync(["a.js"]);
    const w2 = FakeWorker.all[1];
    await w2.emit({ type: "register-command", id: "other", name: "Other" });
    expect(w1.terminated).toBe(true);
    vi.advanceTimersByTime(21_000); // 31 s after the original run
    // The only toast is the one that said, 2 s in, that the old run was still going.
    expect({ terminated: w2.terminated, commands: pluginCmds(), toasts }).toEqual({
      terminated: false,
      commands: ["plugin:a.js:other"],
      toasts: ["Plugin A: Slow is still running."],
    });
  });

  it("FINDING-070: disabling and re-enabling a plugin within 30 s of a slow run gets the new instance stopped", async () => {
    const host = makeHost();
    await startAndHang(host);
    await host.sync([]);
    await host.sync(["a.js"]);
    const w2 = FakeWorker.all[1];
    await w2.emit({ type: "register-command", id: "slow", name: "Slow" });
    vi.advanceTimersByTime(31_000);
    expect({ terminated: w2.terminated, commands: pluginCmds() }).toEqual({ terminated: false, commands: ["plugin:a.js:slow"] });
  });

  it("FINDING-070: switching vaults stops the other vault's plugin that has the same file name", async () => {
    const host = makeHost();
    await startAndHang(host); // vault 1: a.js has a slow command running
    // openVault() for vault 2: stopAll(), then sync() with vault 2's plugins.
    config.set("plugins/a.js", "// @name Something else\n");
    host.stopAll();
    await host.sync(["a.js"]);
    const w2 = FakeWorker.all[1];
    vi.advanceTimersByTime(31_000);
    expect(w2.terminated).toBe(false);
  });

  it("FINDING-070 (side effect): a plugin disabled during a slow run still gets a 'took too long and was stopped' error 30 s later", async () => {
    const host = makeHost();
    await startAndHang(host);
    await host.sync([]); // disabled; nothing of a.js is running any more
    vi.advanceTimersByTime(31_000);
    expect(toasts).toEqual([]);
  });
});
