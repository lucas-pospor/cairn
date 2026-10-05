// Reproduction for FINDING-072: the 500-character cap that the plugin host
// applies to ui.toast text (plugins.ts handle(), "ui.toast") must also apply to
// the other plugin-controlled strings the host shows: the error of a failed
// command, a worker error message, a registered command's name, and the
// plugin's @name (prefixed to every toast, including capped ones).
//
// Each path is checked on its own, so a fix for one does not hide the others.
// The last test runs the host's real worker bootstrap (the Blob the host builds)
// in a node:vm context wired to a fake Worker, to show the command-error path is
// reachable from ordinary plugin code (a command handler that throws), not only
// from a hand-crafted postMessage.
//
// The FINDING-072 tests are regression tests that assert the correct
// behaviour.
//
// Run: cd app && npx vitest run src/lib/adv_verify_pl_09.test.ts

import { describe, it, expect, vi, beforeEach } from "vitest";
import { resolveObjectURL } from "node:buffer";
import vm from "node:vm";

const config = new Map<string, string>();

vi.mock("./backend", () => ({
  backend: {
    listConfig: async (dir: string) =>
      [...config.keys()].filter((k) => k.startsWith(dir + "/")).map((k) => k.slice(dir.length + 1)),
    // Every plugin file counts as turned on on this device.
    pluginApprovals: () => approveAll(config),
    readConfig: async (name: string) => config.get(name) ?? null,
  },
}));

import { PluginHost } from "./plugins";
import { approveAll } from "./plugins.testutil";
import { commands } from "./commands";

const CAP = 500;
const finding = it;
const BIG = 100_000;

class FakeWorker {
  static all: FakeWorker[] = [];
  onmessage: ((e: { data: unknown }) => unknown) | null = null;
  onerror: ((e: { message: string }) => unknown) | null = null;
  /** Where postMessage from the host goes (set by the vm test). */
  inbox: ((m: unknown) => void) | null = null;
  constructor(
    public url: string,
    public opts: { name: string },
  ) {
    FakeWorker.all.push(this);
  }
  postMessage(m: unknown) {
    this.inbox?.(structuredClone(m));
  }
  terminate() {}
  async emit(data: unknown) {
    await this.onmessage?.({ data });
  }
}

const toasts: { message: string; kind?: string }[] = [];
const makeHost = () =>
  new PluginHost({
    notePaths: () => [],
    activePath: () => null,
    getSelection: () => null,
    replaceSelection: () => false,
    toast: (message, kind) => toasts.push({ message, kind }),
  });
const pluginCommands = () => commands.all().filter((c) => c.id.startsWith("plugin:"));
const ticks = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

beforeEach(() => {
  config.clear();
  toasts.length = 0;
  FakeWorker.all = [];
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
  commands.unregisterPrefix("plugin:");
});

describe("FINDING-072 baseline", () => {
  it("ui.toast text from a plugin with a short name is capped (the intended limit)", async () => {
    config.set("plugins/p.js", "// @name P\n");
    await makeHost().sync(["p.js"]);
    await FakeWorker.all[0].emit({ type: "call", id: 1, method: "ui.toast", args: ["x".repeat(BIG)] });
    expect(toasts).toHaveLength(1);
    expect(toasts[0].message.length).toBeLessThanOrEqual(CAP + 10);
  });
});

describe("FINDING-072: plugin-controlled text that skips the 500-char cap", () => {
  finding("FINDING-072a: a command-done error is shown uncapped", async () => {
    config.set("plugins/p.js", "// @name P\n");
    await makeHost().sync(["p.js"]);
    await FakeWorker.all[0].emit({ type: "command-done", id: "x", error: "y".repeat(BIG) });
    expect(toasts).toHaveLength(1);
    expect(toasts[0].message.length).toBeLessThanOrEqual(CAP + 10);
  });

  finding("FINDING-072b: a worker error message is shown uncapped", async () => {
    config.set("plugins/p.js", "// @name P\n");
    await makeHost().sync(["p.js"]);
    await FakeWorker.all[0].onerror?.({ message: "Uncaught Error: " + "z".repeat(BIG) });
    expect(toasts).toHaveLength(1);
    expect(toasts[0].message.length).toBeLessThanOrEqual(CAP + 30);
  });

  finding("FINDING-072c: a registered command name is stored uncapped", async () => {
    config.set("plugins/p.js", "// @name P\n");
    await makeHost().sync(["p.js"]);
    await FakeWorker.all[0].emit({ type: "register-command", id: "c", name: "n".repeat(BIG) });
    expect(pluginCommands()).toHaveLength(1);
    expect(pluginCommands()[0].name.length).toBeLessThanOrEqual(CAP + 10);
  });

  finding("FINDING-072d: a long @name makes even a capped ui.toast exceed the cap", async () => {
    config.set("plugins/p.js", `// @name ${"N".repeat(BIG)}\n`);
    await makeHost().sync(["p.js"]);
    await FakeWorker.all[0].emit({ type: "call", id: 1, method: "ui.toast", args: ["hi"] });
    expect(toasts).toHaveLength(1);
    expect(toasts[0].message.length).toBeLessThanOrEqual(CAP + 10);
  });

  finding("FINDING-072e: real bootstrap: a command handler that throws a long error gives an uncapped toast", async () => {
    // Capture the Blob the host builds, so the real BOOTSTRAP + plugin wrapper runs.
    const plugin = [
      "// @name Thrower",
      "cairn.commands.register('boom', 'Boom', () => { throw new Error('e'.repeat(100000)); });",
    ].join("\n");
    config.set("plugins/real.js", plugin);
    const host = makeHost();
    await host.sync(["real.js"]);
    const w = FakeWorker.all[0];
    const blob = resolveObjectURL(w.url);
    expect(blob).toBeDefined();
    const code = await blob!.text();

    // A minimal worker global: self, postMessage, timers.
    const sandbox: Record<string, unknown> = {
      setTimeout,
      clearTimeout,
      postMessage: (m: unknown) => void w.onmessage?.({ data: structuredClone(m) }),
    };
    sandbox.self = sandbox;
    vm.createContext(sandbox);
    w.inbox = (m) => void (sandbox.onmessage as (e: { data: unknown }) => unknown)?.({ data: m });
    vm.runInContext(code, sandbox);
    await ticks();

    expect(pluginCommands().map((c) => c.id)).toEqual(["plugin:real.js:boom"]);
    commands.run("plugin:real.js:boom");
    await ticks();
    host.stopAll(); // clears nothing pending in vm, but stops the 30 s timer from mattering

    const errs = toasts.filter((t) => t.kind === "error");
    expect(errs).toHaveLength(1);
    expect(errs[0].message.length).toBeLessThanOrEqual(CAP + 30);
  });
});
