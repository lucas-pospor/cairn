// Reproduction for FINDING-154 (plugin command-id prefix collision).
//
// If plugin command ids were built as `plugin:${file}:${id}` with no escaping,
// and stopping a plugin removed every command whose id starts with
// `plugin:${file}:` (app/src/lib/plugins.ts stop() / register-command), a
// plugin file whose name starts with another plugin's name followed by ':'
// (legal on Linux and macOS) would share that plugin's id namespace.
//
// The tests assert the correct behaviour. Each logs the observed state before
// asserting.
//
// Run: cd app && npx vitest run src/lib/adv_verify_pl_11.test.ts

import { describe, it, expect, vi, beforeEach } from "vitest";

const config = new Map<string, string>();

vi.mock("./backend", () => {
  const backend = {
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
  };
  return { backend };
});

import { PluginHost } from "./plugins";
import { approveAll } from "./plugins.testutil";
import { commands } from "./commands";

class FakeWorker {
  static all: FakeWorker[] = [];
  sent: unknown[] = [];
  terminated = false;
  onmessage: ((e: { data: unknown }) => unknown) | null = null;
  onerror: ((e: { message: string }) => unknown) | null = null;
  constructor(
    public url: string,
    public opts: unknown,
  ) {
    FakeWorker.all.push(this);
  }
  postMessage(m: unknown) {
    // The real bootstrap answers the host's pings (the host stops a plugin that does not).
    if ((m as { type: string }).type === "ping") return void this.emit({ type: "pong" });
    this.sent.push(m);
  }
  terminate() {
    this.terminated = true;
  }
  async emit(data: unknown) {
    await this.onmessage?.({ data });
  }
}

const makeHost = () =>
  new PluginHost({
    notePaths: () => [],
    activePath: () => null,
    getSelection: () => null,
    replaceSelection: () => false,
    toast: () => {},
  });

const pluginCommands = () => commands.all().filter((c) => c.id.startsWith("plugin:"));

beforeEach(() => {
  config.clear();
  FakeWorker.all = [];
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
  commands.unregisterPrefix("plugin:");
});

describe("FINDING-154 verification", () => {
  it("FINDING-154: disabling 'a.js' keeps the command of the still-enabled plugin 'a.js:b.js'", async () => {
    config.set("plugins/a.js", "// @name A\n");
    config.set("plugins/a.js:b.js", "// @name AB\n");
    const host = makeHost();
    // available() really lists both files (the backend does not reject ':').
    expect((await host.available()).map((m) => m.file)).toEqual(["a.js", "a.js:b.js"]);
    await host.sync(["a.js", "a.js:b.js"]);
    const [wa, wab] = FakeWorker.all;
    await wa.emit({ type: "register-command", id: "x", name: "X" });
    await wab.emit({ type: "register-command", id: "y", name: "Y" });
    await host.sync(["a.js:b.js"]);
    const ids = pluginCommands().map((c) => c.id);
    console.log("FINDING-154 after disabling a.js, plugin commands =", JSON.stringify(ids), "a.js:b.js worker terminated =", wab.terminated);
    expect(wab.terminated).toBe(false); // the other plugin is still running...
    expect(ids).toEqual(["plugin:a.js:b.js:y"]); // ...and so is its command
  });

  it("FINDING-154: plugin 'a.js' registering id 'b.js:y' does not replace the command of plugin 'a.js:b.js'", async () => {
    config.set("plugins/a.js", "// @name A\n");
    config.set("plugins/a.js:b.js", "// @name AB\n");
    const host = makeHost();
    await host.sync(["a.js", "a.js:b.js"]);
    const [wa, wab] = FakeWorker.all;
    await wab.emit({ type: "register-command", id: "y", name: "Y" });
    await wa.emit({ type: "register-command", id: "b.js:y", name: "Y" });
    const cmd = commands.get("plugin:a.js:b.js:y");
    console.log("FINDING-154 palette entry for plugin:a.js:b.js:y is now named", JSON.stringify(cmd?.name));
    // Running the entry should reach plugin a.js:b.js, not plugin a.js.
    wa.sent.length = 0;
    wab.sent.length = 0;
    cmd?.run();
    console.log("FINDING-154 run-command delivered to a.js:", JSON.stringify(wa.sent), "to a.js:b.js:", JSON.stringify(wab.sent));
    expect(wab.sent).toEqual([{ type: "run-command", id: "y", run: expect.any(Number) }]);
    expect(pluginCommands().length).toBe(2);
  });
});
