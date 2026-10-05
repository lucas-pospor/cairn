// Reproduction for FINDING-153: a plugin that the host stopped because a
// command took too long stays enabled in settings, and the next toggle of any
// other plugin (enable or disable) must not silently start it again.
//
// The Settings modal's togglePlugin() is mirrored here: it builds the new list
// from settings.plugins (which still contains the stopped plugin) and calls
// host.sync(list). The backend and Worker are faked so this runs in Node.
//
// Run: cd app && npx vitest run src/lib/adv_verify_pl_10.test.ts

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

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

class FakeWorker {
  static all: FakeWorker[] = [];
  terminated = false;
  onmessage: ((e: { data: unknown }) => unknown) | null = null;
  onerror: ((e: { message: string }) => unknown) | null = null;
  constructor(
    public url: string,
    public opts: { name: string },
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
const settings = { plugins: [] as string[] };

// Same logic as SettingsModal.svelte togglePlugin() (no permissions, so no confirm dialog).
async function togglePlugin(host: PluginHost, file: string, on: boolean) {
  const list = on ? [...settings.plugins, file] : settings.plugins.filter((f) => f !== file);
  settings.plugins = list;
  if (on) host.turnedOn(file);
  await host.sync(list);
}

const liveWorkersFor = (file: string) => FakeWorker.all.filter((w) => w.opts.name === `cairn-plugin-${file}` && !w.terminated);

beforeEach(() => {
  config.clear();
  toasts.length = 0;
  settings.plugins = [];
  FakeWorker.all = [];
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
  commands.unregisterPrefix("plugin:");
});

afterEach(() => vi.useRealTimers());

describe("FINDING-153", () => {
  for (const [label, other, on] of [
    ["enabling", "b.js", true],
    ["disabling", "c.js", false],
  ] as const) {
    it(`FINDING-153: ${label} another plugin restarts a plugin that was stopped for taking too long`, async () => {
      vi.useFakeTimers();
      config.set("plugins/a.js", "// @name A\n");
      config.set("plugins/b.js", "// @name B\n");
      config.set("plugins/c.js", "// @name C\n");
      const host = new PluginHost({
        notePaths: () => [],
        activePath: () => null,
        getSelection: () => null,
        replaceSelection: () => false,
        toast: (m) => toasts.push(m),
      });
      await togglePlugin(host, "a.js", true);
      await togglePlugin(host, "c.js", true);
      const a1 = liveWorkersFor("a.js")[0];
      await a1.emit({ type: "register-command", id: "x", name: "X" });
      commands.run("plugin:a.js:x");
      vi.advanceTimersByTime(31_000);

      // The host stopped A and said so, but the setting (and so the checkbox) still says enabled.
      expect(a1.terminated).toBe(true);
      expect(toasts.some((t) => /took too long and was stopped/.test(t))).toBe(true);
      expect(settings.plugins).toContain("a.js");
      expect(liveWorkersFor("a.js")).toEqual([]);
      const toastsBefore = toasts.length;

      // The user toggles a different plugin.
      await togglePlugin(host, other, on);

      // Expected: A stays stopped (or is shown as stopped/disabled). Actual: a new live A worker, no toast.
      expect(toasts.length).toBe(toastsBefore); // no message tells the user A came back
      expect(liveWorkersFor("a.js")).toEqual([]);
    });
  }
});
