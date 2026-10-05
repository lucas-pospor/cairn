// Adversarial tests for the plugin host (app/src/lib/plugins.ts).
//
// The host is driven with a fake Worker and a mocked backend, so these run in
// plain Node without a DOM. Tests named "FINDING-NNN" are regression tests:
// each reproduces a defect and asserts the correct behaviour.
//
// Run: cd app && npx vitest run src/lib/adv_plugins.test.ts

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// ---- mocked backend: an in-memory vault + .cairn config folder ----
const files = new Map<string, string>(); // vault-relative note path -> content
const config = new Map<string, string>(); // path under .cairn/ -> content
let readConfigDelay = 0;
// Approvals on this device. Unless a test sets `recordedApprovals`, every plugin
// file counts as turned on (approveAll); consent tests approve with host.approve().
const approvals = new Map<string, PluginApproval>();
let recordedApprovals = false;

vi.mock("./backend", () => {
  const notFound = (p: string) => ({ kind: "notFound", detail: p });
  const backend = {
    listConfig: async (dir: string) =>
      [...config.keys()].filter((k) => k.startsWith(dir + "/") && !k.slice(dir.length + 1).includes("/")).map((k) => k.slice(dir.length + 1)),
    readConfig: async (name: string) => {
      if (readConfigDelay) await new Promise((r) => setTimeout(r, readConfigDelay));
      return config.get(name) ?? null;
    },
    pluginApprovals: async () => (recordedApprovals ? Object.fromEntries(approvals) : approveAll(config)),
    setPluginApproval: async (file: string, approval: PluginApproval | null) => {
      if (approval) approvals.set(file, approval);
      else approvals.delete(file);
    },
    readNote: async (path: string) => {
      if (!files.has(path)) throw notFound(path);
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
  };
  return { backend };
});

import { PluginHost, isApproved, parseManifest, type PluginApproval } from "./plugins";
import { approveAll } from "./plugins.testutil";
import { commands } from "./commands";

// ---- fake Worker ----
class FakeWorker {
  static all: FakeWorker[] = [];
  sent: unknown[] = [];
  terminated = false;
  /** Stuck in a busy loop: does not answer the host's pings. */
  busy = false;
  onmessage: ((e: { data: unknown }) => unknown) | null = null;
  onerror: ((e: { message: string }) => unknown) | null = null;
  constructor(
    public url: string,
    public opts: unknown,
  ) {
    FakeWorker.all.push(this);
  }
  /** Set by runRealWorker(): the host's messages go to the real bootstrap instead. */
  deliver: ((m: unknown) => void) | null = null;
  postMessage(m: unknown) {
    if (this.deliver) return this.deliver(m);
    // The host checks that the worker is not stuck; the real bootstrap answers each ping.
    if ((m as { type: string }).type === "ping") {
      if (!this.busy) void this.emit({ type: "pong" });
      return;
    }
    this.sent.push(m);
  }
  terminate() {
    this.terminated = true;
  }
  /** Simulate the worker posting a message to the host. */
  async emit(data: unknown) {
    await this.onmessage?.({ data });
  }
}

const toasts: { message: string; kind?: string }[] = [];
function makeHost() {
  return new PluginHost({
    notePaths: () => [...files.keys()],
    activePath: () => "a.md",
    getSelection: () => "sel",
    replaceSelection: () => true,
    toast: (message, kind) => toasts.push({ message, kind }),
  });
}

const flush = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

beforeEach(() => {
  files.clear();
  config.clear();
  toasts.length = 0;
  readConfigDelay = 0;
  approvals.clear();
  recordedApprovals = false;
  FakeWorker.all = [];
  (globalThis as unknown as { Worker: unknown }).Worker = FakeWorker;
  commands.unregisterPrefix("plugin:");
});

afterEach(() => {
  vi.useRealTimers();
});

const pluginCommands = () => commands.all().filter((c) => c.id.startsWith("plugin:"));

/**
 * Start a plugin with the host's real worker code (the bootstrap, then the plugin's file),
 * run in this thread with `self` and `postMessage` faked. Messages go both ways as in a
 * browser: copied, and handled only once the code running now is done, so a plugin in a
 * loop gets no answers until its loop ends. Returns what the worker posted to the host.
 */
async function runRealWorker(start: () => Promise<unknown>) {
  const blobs: Blob[] = [];
  const createObjectURL = URL.createObjectURL.bind(URL);
  const spy = vi.spyOn(URL, "createObjectURL").mockImplementation((b) => (blobs.push(b as Blob), createObjectURL(b as Blob)));
  await start();
  spy.mockRestore();
  const w = FakeWorker.all.at(-1)!;
  const scope: { onmessage?: (e: { data: unknown }) => unknown } = {};
  const posted: { type: string; id?: number; method?: string; args?: string[]; [k: string]: unknown }[] = [];
  const post = (m: unknown) => {
    const data = structuredClone(m) as (typeof posted)[number];
    posted.push(data);
    queueMicrotask(() => void w.onmessage?.({ data }));
  };
  w.deliver = (m) => {
    const data = structuredClone(m);
    queueMicrotask(() => void scope.onmessage?.({ data }));
  };
  new Function("self", "postMessage", await blobs.at(-1)!.text())(scope, post);
  return { w, posted };
}
/** Let queued messages, and the ones they cause, be handled. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
/** Run ids of the commands the host asked a worker to run, oldest first (the worker sends each back with "command-done"). */
const runIds = (w: FakeWorker) => w.sent.filter((m) => (m as { type: string }).type === "run-command").map((m) => (m as { run: number }).run);

describe("plugin host: lifecycle", () => {
  it("disabling a plugin terminates its worker and removes its commands", async () => {
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "x", name: "X" });
    expect(pluginCommands().map((c) => c.id)).toEqual(["plugin:a.js:x"]);
    await host.sync([]);
    expect(w.terminated).toBe(true);
    expect(pluginCommands()).toEqual([]);
  });

  it("a command that never finishes stops its plugin after 30 s", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x");
    vi.advanceTimersByTime(29_000);
    expect(w.terminated).toBe(false);
    vi.advanceTimersByTime(1_500);
    expect(w.terminated).toBe(true);
    expect(toasts.some((t) => /took too long/.test(t.message))).toBe(true);
    expect(pluginCommands()).toEqual([]);
  });

  it("a command that finishes in time does not stop its plugin", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x");
    await w.emit({ type: "command-done", id: "x", run: runIds(w)[0] });
    vi.advanceTimersByTime(60_000);
    expect(w.terminated).toBe(false);
  });

  it("FINDING-069: running a plugin command twice before the first run finishes kills the plugin 30 s later", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "x", name: "X" });
    // The user presses the command's hotkey twice; both runs finish quickly.
    commands.run("plugin:a.js:x");
    commands.run("plugin:a.js:x");
    await w.emit({ type: "command-done", id: "x", run: runIds(w)[0] });
    await w.emit({ type: "command-done", id: "x", run: runIds(w)[1] });
    vi.advanceTimersByTime(31_000);
    // Expected: both runs finished, so the plugin keeps running.
    expect(toasts.filter((t) => /took too long/.test(t.message))).toEqual([]);
    expect(w.terminated).toBe(false);
  });

  it("FINDING-070: a stale command timer from a stopped instance kills the re-enabled plugin", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w1 = FakeWorker.all[0];
    await w1.emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x"); // a slow command is running...
    await host.sync([]); // ...the user disables the plugin
    await host.sync(["a.js"]); // ...and enables it again
    const w2 = FakeWorker.all[1];
    await w2.emit({ type: "register-command", id: "x", name: "X" });
    vi.advanceTimersByTime(31_000);
    // Expected: the new instance never ran a command, so nothing stops it.
    expect(w2.terminated).toBe(false);
    expect(pluginCommands().map((c) => c.id)).toEqual(["plugin:a.js:x"]);
  });

  it("FINDING-069: of two overlapping runs of one command, one that never finishes still stops the plugin", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x");
    vi.advanceTimersByTime(1_000);
    commands.run("plugin:a.js:x");
    await w.emit({ type: "command-done", id: "x", run: runIds(w)[0] }); // only the first of the two runs finishes
    vi.advanceTimersByTime(31_000);
    expect(w.terminated).toBe(true);
  });

  it("FINDING-069: a run that never finishes stops the plugin 30 s after it started, even if later runs of the command finish", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x"); // t = 0: this run never finishes
    for (let i = 1; i <= 3; i++) {
      vi.advanceTimersByTime(9_000);
      commands.run("plugin:a.js:x"); // t = 9, 18, 27 s: quick runs
      await w.emit({ type: "command-done", id: "x", run: runIds(w)[i] });
    }
    expect(w.terminated).toBe(false);
    vi.advanceTimersByTime(3_500); // t = 30.5 s
    expect(w.terminated).toBe(true);
  });

  it("FINDING-070: a palette entry kept from a stopped instance neither runs nor stops the new instance", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w1 = FakeWorker.all[0];
    await w1.emit({ type: "register-command", id: "x", name: "X" });
    const stale = commands.get("plugin:a.js:x")!; // e.g. the palette was open while the plugin was reloaded
    host.stopAll();
    await host.sync(["a.js"]);
    const w2 = FakeWorker.all[1];
    await w1.emit({ type: "register-command", id: "late", name: "Late" }); // already on its way when w1 was stopped
    stale.run();
    vi.advanceTimersByTime(31_000);
    expect(w1.sent).toEqual([]);
    expect(w2.terminated).toBe(false);
    expect(pluginCommands()).toEqual([]);
  });

  it("FINDING-068: two overlapping sync() calls start the plugin twice and leave an orphan worker that disabling cannot stop", async () => {
    readConfigDelay = 5;
    config.set("plugins/a.js", "// @name A\n// @permissions write\n");
    const host = makeHost();
    // e.g. "Reload plugins" clicked twice, or enable + reload in quick succession
    const p1 = host.sync(["a.js"]);
    const p2 = host.sync(["a.js"]);
    await Promise.all([p1, p2]);
    await host.sync([]); // the user disables the plugin
    const alive = FakeWorker.all.filter((w) => !w.terminated);
    expect(FakeWorker.all.length).toBe(1);
    expect(alive).toEqual([]);
  });

  it("FINDING-068: stopAll() while a start is in flight (vault switch, Reload) leaves the plugin running", async () => {
    readConfigDelay = 5;
    config.set("plugins/a.js", "// @name A\n// @permissions read write\n");
    const host = makeHost();
    const p = host.sync(["a.js"]);
    host.stopAll(); // closeVault() / openVault() / reloadPlugins() call this
    await p;
    const alive = FakeWorker.all.filter((w) => !w.terminated);
    expect(alive).toEqual([]);
  });

  it("FINDING-068: a sync() overtaken by stopAll() or a newer sync() starts none of its remaining plugins", async () => {
    readConfigDelay = 5;
    config.set("plugins/a.js", "// @name A\n");
    config.set("plugins/b.js", "// @name B\n");
    const host = makeHost();
    const names = () => FakeWorker.all.filter((w) => !w.terminated).map((w) => (w.opts as { name: string }).name);
    // Vault closed (or switched) while a.js was being read: b.js must not start afterwards.
    const p1 = host.sync(["a.js", "b.js"]);
    host.stopAll();
    await p1;
    expect(names()).toEqual([]);
    // b.js disabled again while a.js was being read.
    const p2 = host.sync(["a.js", "b.js"]);
    const p3 = host.sync(["a.js"]);
    await Promise.all([p2, p3]);
    expect(names()).toEqual(["cairn-plugin-a.js"]);
  });

  it("FINDING-153: a plugin stopped for taking too long is silently restarted when any other plugin is toggled", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    config.set("plugins/b.js", "// @name B\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x");
    vi.advanceTimersByTime(31_000);
    expect(w.terminated).toBe(true);
    // Settings still list a.js as enabled; the user now enables b.js.
    await host.sync(["a.js", "b.js"]);
    const aWorkers = FakeWorker.all.filter((x) => x.opts && (x.opts as { name: string }).name === "cairn-plugin-a.js");
    expect(aWorkers.length).toBe(1);
  });

  it("FINDING-153: a plugin stopped for taking too long is turned off, and starts again once turned back on", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    let enabled = ["a.js"];
    const host = new PluginHost({
      notePaths: () => [],
      activePath: () => null,
      getSelection: () => null,
      replaceSelection: () => false,
      toast: (message, kind) => toasts.push({ message, kind }),
      disable: (file) => (enabled = enabled.filter((f) => f !== file)),
    });
    const live = () => FakeWorker.all.filter((w) => !w.terminated).length;
    await host.sync(enabled);
    await FakeWorker.all[0].emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x");
    vi.advanceTimersByTime(31_000);
    expect(enabled).toEqual([]); // the settings no longer list it, so its checkbox is off
    expect(live()).toBe(0);
    // The user ticks it again right away, with no other sync() in between (as SettingsModal's togglePlugin does).
    enabled = [...enabled, "a.js"];
    host.turnedOn("a.js");
    await host.sync(enabled);
    expect(live()).toBe(1);
  });

  it("FINDING-153: a sync() already under way when a plugin is stopped for taking too long does not start it again", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    config.set("plugins/b.js", "// @name B\n");
    let enabled = ["a.js"];
    const host = new PluginHost({
      notePaths: () => [],
      activePath: () => null,
      getSelection: () => null,
      replaceSelection: () => false,
      toast: (message, kind) => toasts.push({ message, kind }),
      disable: (file) => (enabled = enabled.filter((f) => f !== file)),
    });
    const names = () => FakeWorker.all.filter((w) => !w.terminated).map((w) => (w.opts as { name: string }).name);
    await host.sync(enabled);
    await FakeWorker.all[0].emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x");
    vi.advanceTimersByTime(29_990);
    readConfigDelay = 50;
    // b.js is still being read when a.js is stopped; the list of this sync() still has a.js after it.
    const p = host.sync(["b.js", "a.js"]);
    await vi.advanceTimersByTimeAsync(500);
    await p;
    expect(enabled).toEqual([]);
    expect(names()).toEqual(["cairn-plugin-b.js"]);
  });

  it("FINDING-153: after a vault switch, a plugin file of the same name starts even if the old one was stopped for taking too long", async () => {
    vi.useFakeTimers();
    config.set("plugins/a.js", "// @name A\n");
    const host = makeHost();
    await host.sync(["a.js"]);
    await FakeWorker.all[0].emit({ type: "register-command", id: "x", name: "X" });
    commands.run("plugin:a.js:x");
    vi.advanceTimersByTime(31_000);
    host.stopAll(); // openVault() of another vault that also enables an a.js
    await host.sync(["a.js"]);
    expect(FakeWorker.all.filter((w) => !w.terminated).length).toBe(1);
  });
});

describe("plugin host: consent and permissions", () => {
  it("FINDING-005: a plugin the vault's settings.json lists does not start until it is turned on on this device", async () => {
    recordedApprovals = true;
    config.set("plugins/helper.js", "// @name Helper\n// @permissions read write\n");
    const host = makeHost();
    // A vault from someone else: settings.json lists helper.js, this device never approved it.
    expect(await host.sync(["helper.js"])).toEqual(["helper.js"]);
    expect(FakeWorker.all).toEqual([]);
    const [listed] = await host.available();
    expect(listed).toEqual(expect.objectContaining({ file: "helper.js", permissions: ["read", "write"], approved: false }));
    // The user turns it on in Settings > Plugins (after the permission dialog).
    await host.approve(listed);
    expect((await host.available())[0].approved).toBe(true);
    expect(await host.sync(["helper.js"])).toEqual([]);
    expect(FakeWorker.all.length).toBe(1);
    // Turned off again: the approval is gone, so the next start needs a new one.
    await host.sync([]);
    await host.revoke("helper.js");
    expect(await host.sync(["helper.js"])).toEqual(["helper.js"]);
    expect(FakeWorker.all.length).toBe(1);
  });

  it("FINDING-021: permissions are re-read from the file at every start, so a changed file gets new permissions without consent", async () => {
    recordedApprovals = true;
    config.set("plugins/p.js", "// @name P\n// @permissions editor\n");
    const host = makeHost();
    const listed = await host.available();
    expect(listed[0].permissions).toEqual(["editor"]); // what the user consented to
    await host.approve(listed[0]);
    await host.sync(["p.js"]);
    expect(FakeWorker.all.length).toBe(1);
    host.stopAll();
    // The file changes on disk (git pull, file sync, another app, a malicious update).
    config.set("plugins/p.js", "// @name P\n// @permissions editor read write\n");
    // Reload plugins / app restart: the changed file does not start, and is shown as not approved,
    // so turning it on again shows the dialog with the new permissions.
    expect(await host.sync(["p.js"])).toEqual(["p.js"]);
    expect(FakeWorker.all.length).toBe(1);
    expect((await host.available())[0]).toEqual(expect.objectContaining({ permissions: ["editor", "read", "write"], approved: false }));
  });

  it("FINDING-021: an approval covers only the permissions the user saw", () => {
    const approval: PluginApproval = { hash: "h1", permissions: ["editor"] };
    expect(isApproved(approval, "h1", ["editor"])).toBe(true);
    expect(isApproved(approval, "h1", [])).toBe(true);
    expect(isApproved(approval, "h1", ["editor", "read"])).toBe(false);
    expect(isApproved(approval, "h2", ["editor"])).toBe(false);
    expect(isApproved(undefined, "h1", [])).toBe(false);
  });

  it("FINDING-067: plugins enabled in settings but not ending in .js or inside a subfolder run, yet are not listed in Settings > Plugins", async () => {
    config.set("plugins/visible.js", "// @name Visible\n");
    config.set("plugins/hidden/stealth.js", "// @name Stealth\n// @permissions read write\n");
    config.set("plugins/stealth2.txt", "// @name Stealth 2\n// @permissions read write\n");
    const host = makeHost();
    const listed = (await host.available()).map((m) => m.file);
    // Even with an approval on this device for each file, only listed files can start.
    await host.sync(["visible.js", "hidden/stealth.js", "stealth2.txt"]);
    const started = FakeWorker.all.map((w) => (w.opts as { name: string }).name.replace("cairn-plugin-", ""));
    // Every plugin that runs must be visible (and switchable off) in the list.
    for (const f of started) expect(listed).toContain(f);
    expect(started).toEqual(["visible.js"]);
  });

  it("raw messages for every gated method are checked on the host side", async () => {
    config.set("plugins/n.js", "// @name N\n");
    const host = makeHost();
    await host.sync(["n.js"]);
    const w = FakeWorker.all[0];
    files.set("a.md", "text");
    const methods = ["notes.list", "notes.read", "notes.write", "editor.activePath", "editor.getSelection", "editor.replaceSelection", "fs.read", "__proto__", "constructor", "toString", ""];
    for (const [i, method] of methods.entries()) await w.emit({ type: "call", id: i, method, args: ["a.md", "x"] });
    await flush();
    const results = w.sent.filter((m) => (m as { type: string }).type === "result") as { id: number; error?: string; value?: unknown }[];
    expect(results.length).toBe(methods.length);
    for (const r of results) expect(r.error, `method ${methods[r.id]}`).toBeTruthy();
    expect(files.get("a.md")).toBe("text");
  });

  it("method names that are not strings are refused", async () => {
    config.set("plugins/n.js", "// @name N\n// @permissions read write editor\n");
    const host = makeHost();
    await host.sync(["n.js"]);
    const w = FakeWorker.all[0];
    for (const method of [["notes.read"], { toString: "notes.read" }, 42, null]) await w.emit({ type: "call", id: 1, method, args: [] });
    await flush();
    const results = w.sent.filter((m) => (m as { type: string }).type === "result") as { error?: string }[];
    expect(results.length).toBe(4);
    for (const r of results) expect(r.error).toBeTruthy();
  });

  it("notes.write refuses non-Markdown targets before touching the backend", async () => {
    const host = makeHost();
    const m = parseManifest("w.js", "// @permissions write");
    for (const p of ["x.js", "x.html", "x.svg", "x.md.js", "x", ".cairn/plugins/evil.js", ".cairn/settings.json", ".cairn/snippets/x.css", "x.md/"]) {
      await expect(host.handle(m, "notes.write", [p, "boom"]), p).rejects.toThrow(/Markdown/);
    }
    expect(files.size).toBe(0);
  });

  it("notes.read and notes.write only reach Markdown notes outside hidden folders (FINDING-022, FINDING-071)", async () => {
    const host = makeHost();
    const m = parseManifest("rw.js", "// @permissions read write");
    const hidden: [string, string][] = [
      [".cairn/settings.json", "{}"],
      [".cairn/plugins/other.js", "other plugin's source"],
      [".git/config", "url = https://user:token@example.invalid/x.git"],
      [".trash/deleted.md", "a deleted note"],
      [".cairn/notes.md", "config-folder note"],
      ["a/.hidden/x.md", "in a dot-folder"],
      [".dot.md", "a dotfile"],
    ];
    for (const [p, c] of hidden) files.set(p, c);
    files.set("Note.md", "a note");
    for (const p of [...hidden.map(([p]) => p), ".trash\\deleted.md", "../x.md", "x.txt"]) {
      await expect(host.handle(m, "notes.read", [p]), p).rejects.toThrow(/Markdown notes/);
    }
    for (const p of [".trash/deleted.md", ".cairn/notes.md", "a/.hidden/x.md", ".dot.md", ".trash\\deleted.md", ".cairn/new.md"]) {
      await expect(host.handle(m, "notes.write", [p, "overwritten"]), p).rejects.toThrow(/Markdown notes/);
    }
    expect(Object.fromEntries(files)).toEqual(Object.fromEntries([...hidden, ["Note.md", "a note"]]));
    // Ordinary notes still work.
    await expect(host.handle(m, "notes.read", ["Note.md"])).resolves.toBe("a note");
    await host.handle(m, "notes.write", ["dir/New.md", "new"]);
    expect(files.get("dir/New.md")).toBe("new");
  });

  it("prototype-polluting payloads in messages do not reach Object.prototype", async () => {
    config.set("plugins/pp.js", "// @name PP\n// @permissions read write editor\n");
    const host = makeHost();
    await host.sync(["pp.js"]);
    const w = FakeWorker.all[0];
    const evil = JSON.parse('{"__proto__": {"polluted": "yes"}, "constructor": {"prototype": {"polluted2": "yes"}}}');
    await w.emit({ type: "register-command", id: "__proto__", name: evil });
    await w.emit({ type: "register-command", id: "constructor", name: "c" });
    await w.emit({ type: "call", id: "__proto__", method: "ui.toast", args: [evil, evil] });
    await w.emit({ type: "call", id: 2, method: "notes.write", args: ["__proto__.md", evil] });
    await w.emit(JSON.parse('{"type": "command-done", "id": "__proto__", "__proto__": {"polluted3": 1}}'));
    await w.emit(evil);
    await flush();
    const probe = {} as Record<string, unknown>;
    expect(probe.polluted).toBeUndefined();
    expect(probe.polluted2).toBeUndefined();
    expect(probe.polluted3).toBeUndefined();
    expect(pluginCommands().map((c) => c.id).sort()).toEqual(["plugin:pp.js:__proto__", "plugin:pp.js:constructor"]);
    expect(commands.keysFor("plugin:pp.js:__proto__")).toEqual([]);
  });

  it("unknown or malformed message types are ignored without throwing", async () => {
    config.set("plugins/m.js", "// @name M\n");
    const host = makeHost();
    await host.sync(["m.js"]);
    const w = FakeWorker.all[0];
    for (const data of [null, undefined, 42, "call", [], { type: "result", id: 1, value: "x" }, { type: "run-command", id: "x" }, { type: "bogus" }]) {
      // null/undefined make the async handler reject (an unhandled rejection in the
      // page console); nothing else happens.
      await w.emit(data).catch(() => {});
    }
    // The host still answers a normal call afterwards and nothing was registered.
    await w.emit({ type: "call", id: 99, method: "ui.toast", args: ["still here"] });
    expect(toasts.map((t) => t.message)).toEqual(["M: still here"]);
    expect(pluginCommands()).toEqual([]);
  });
});

describe("plugin manifest parsing", () => {
  it("only the first @permissions line counts, and the consent text uses the same parse as enforcement", () => {
    const m = parseManifest("x.js", "// @permissions editor\n// @permissions read write\n");
    expect(m.permissions).toEqual(["editor"]);
  });

  it("uppercase or unknown permission names grant nothing", () => {
    expect(parseManifest("x.js", "// @permissions READ Write all *").permissions).toEqual([]);
    expect(parseManifest("x.js", "// @permissions: read").permissions).toEqual([]);
  });

  it("a @permissions line with nothing after it picks up the next line of code as permissions", () => {
    // Documenting behavior: \s+ in the header regex crosses the newline. The settings
    // screen shows the same parse, so this is not a silent escalation, but a header
    // left empty grants whatever words the following line happens to contain.
    const m = parseManifest("x.js", "// @permissions\nconst read = 1, write = 2;\n");
    expect(m.permissions).toEqual(["read", "write"]);
  });
});

describe("plugin host: messages that bypass limits", () => {
  it("FINDING-072: error toasts from a plugin are not capped like ui.toast (500 chars)", async () => {
    config.set("plugins/big.js", "// @name Big\n");
    const host = makeHost();
    await host.sync(["big.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "call", id: 1, method: "ui.toast", args: ["x".repeat(100_000)] });
    await w.emit({ type: "command-done", id: "nope", error: "y".repeat(100_000) });
    await w.onerror?.({ message: "z".repeat(100_000) });
    await w.emit({ type: "register-command", id: "c", name: "n".repeat(100_000) });
    const longest = Math.max(...toasts.map((t) => t.message.length), ...pluginCommands().map((c) => c.name.length));
    expect(longest).toBeLessThan(1000);
  });

  it("FINDING-154: disabling plugin 'a.js' also removes the commands of plugin 'a.js:b.js' (prefix collision)", async () => {
    config.set("plugins/a.js", "// @name A\n");
    config.set("plugins/a.js:b.js", "// @name AB\n");
    const host = makeHost();
    await host.sync(["a.js", "a.js:b.js"]);
    const [wa, wab] = FakeWorker.all;
    await wa.emit({ type: "register-command", id: "x", name: "X" });
    await wab.emit({ type: "register-command", id: "y", name: "Y" });
    await host.sync(["a.js:b.js"]);
    expect(pluginCommands().map((c) => c.id)).toEqual(["plugin:a.js:b.js:y"]);
  });

  it("FINDING-160: two running plugins with the same @name have palette entries and toasts that tell them apart", async () => {
    config.set("plugins/a.js", "// @name Tools\n");
    config.set("plugins/evil.js", "// @name Tools\n");
    const host = makeHost();
    await host.sync(["a.js", "evil.js"]);
    const [wa, we] = FakeWorker.all;
    await wa.emit({ type: "register-command", id: "go", name: "Go" });
    await we.emit({ type: "register-command", id: "go", name: "Go" });
    await wa.emit({ type: "call", id: 1, method: "ui.toast", args: ["hi"] });
    await we.emit({ type: "call", id: 1, method: "ui.toast", args: ["hi"] });
    expect(pluginCommands().map((c) => c.name).sort()).toEqual(["Tools (a.js): Go", "Tools (evil.js): Go"]);
    expect(toasts.map((t) => t.message)).toEqual(["Tools (a.js): hi", "Tools (evil.js): hi"]);
    // Once the other one is off, the name alone is enough again.
    await host.sync(["a.js"]);
    expect(pluginCommands().map((c) => c.name)).toEqual(["Tools: Go"]);
  });

  it("FINDING-159: a plugin registering 10,000 commands puts at most 200 into the palette", async () => {
    config.set("plugins/many.js", "// @name Many\n");
    const host = makeHost();
    await host.sync(["many.js"]);
    const w = FakeWorker.all[0];
    for (let i = 0; i < 10_000; i++) await w.emit({ type: "register-command", id: `c${i}`, name: `Command number ${i}` });
    expect(pluginCommands()).toHaveLength(200);
    // Registering a command it already has again still replaces it.
    await w.emit({ type: "register-command", id: "c7", name: "Renamed" });
    expect(pluginCommands().find((c) => c.id === "plugin:many.js:c7")?.name).toBe("Many: Renamed");
    expect(pluginCommands()).toHaveLength(200);
    expect(toasts.map((t) => t.message)).toEqual(["Plugin Many has more than 200 commands. Only the first 200 are in the palette."]);
  });

  it("FINDING-162: a plugin flooding the page with messages is stopped and turned off", async () => {
    config.set("plugins/flood.js", "// @name Flood\n");
    let enabled = ["flood.js"];
    const host = new PluginHost({
      notePaths: () => [],
      activePath: () => null,
      getSelection: () => null,
      replaceSelection: () => false,
      toast: (message, kind) => toasts.push({ message, kind }),
      disable: (file) => (enabled = enabled.filter((f) => f !== file)),
    });
    await host.sync(enabled);
    const w = FakeWorker.all[0];
    // A tight loop re-registering commands: far more messages in one second than any plugin needs.
    for (let n = 0; n < 30_000; n++) await w.emit({ type: "register-command", id: `x${n % 100}`, name: `Tight ${n}` });
    expect(w.terminated).toBe(true);
    expect(pluginCommands()).toEqual([]);
    expect(enabled).toEqual([]);
    expect(toasts.map((t) => t.message)).toEqual(["Plugin Flood sent too many messages and was stopped."]);
  });

  it("FINDING-162: a plugin calling ui.toast with a long text in a tight loop sends 16 calls at a time, cut to 500 characters, and its loop stops with an error", async () => {
    // Each call is sent at once without the limit, so 20,000 of them would be on their way.
    config.set("plugins/tf.js", `// @name Toaster\ncairn.commands.register("go", "Go", () => { const big = "x".repeat(2000); for (let i = 0; i < 20000; i++) cairn.ui.toast(big); });\n`);
    const host = makeHost();
    const { posted } = await runRealWorker(() => host.sync(["tf.js"]));
    await settle();
    commands.run("plugin:tf.js:go");
    // The loop runs when the run-command message is handled; nothing is answered until it ends.
    await Promise.resolve();
    const calls = () => posted.filter((m) => m.type === "call");
    expect(calls()).toHaveLength(16);
    expect(Math.max(...calls().map((m) => m.args![0].length))).toBe(500);
    // 16 sent, 10,000 waiting: the next call threw and ended the command with an error.
    expect(posted.find((m) => m.type === "command-done")?.error).toBe("too many calls are waiting (10000)");
    await settle();
    // The waiting calls failed with it: only the 16 already sent are answered.
    expect(calls()).toHaveLength(16);
    expect(toasts.filter((t) => t.kind !== "error")).toHaveLength(16);
    expect(toasts.filter((t) => t.kind === "error").map((t) => t.message)).toEqual(["Plugin Toaster: too many calls are waiting (10000)"]);
  });

  it("FINDING-162: when a call throws because 10,000 are waiting, the waiting ones fail with the same error", async () => {
    config.set(
      "plugins/held.js",
      `// @name Held\ncairn.commands.register("go", "Go", async () => { const ps = []; try { for (;;) ps.push(cairn.ui.toast("t")); } catch (e) { const r = await Promise.allSettled(ps); postMessage({ type: "probe", thrown: e.message, made: ps.length, done: r.filter((x) => x.status === "fulfilled").length, failed: [...new Set(r.filter((x) => x.status === "rejected").map((x) => x.reason.message))] }); } });\n`,
    );
    const host = makeHost();
    const { posted } = await runRealWorker(() => host.sync(["held.js"]));
    await settle();
    commands.run("plugin:held.js:go");
    await vi.waitFor(() => expect(posted.find((m) => m.type === "probe")).toBeDefined());
    expect(posted.find((m) => m.type === "probe")).toMatchObject({ thrown: "too many calls are waiting (10000)", made: 10_016, done: 16, failed: ["too many calls are waiting (10000)"] });
    expect(toasts).toHaveLength(16);
  });

  it("FINDING-162: reading 5,000 notes at once through the real bootstrap gets every note, with at most 16 calls on their way", async () => {
    config.set(
      "plugins/bulk.js",
      `// @name Bulk\n// @permissions read\ncairn.commands.register("go", "Go", async () => { const paths = await cairn.notes.list(); const all = await Promise.all(paths.map((p) => cairn.notes.read(p))); postMessage({ type: "probe", n: all.length, ok: all.every((c, i) => c === "text of " + paths[i]) }); });\n`,
    );
    for (let i = 0; i < 5000; i++) files.set(`n${i}.md`, `text of n${i}.md`);
    const host = makeHost();
    const { w, posted } = await runRealWorker(() => host.sync(["bulk.js"]));
    // Count the calls on their way: sent by the worker, not yet answered by the host.
    let onTheirWay = 0;
    let most = 0;
    const deliver = w.deliver!;
    w.deliver = (m) => {
      if ((m as { type: string }).type === "result") onTheirWay--;
      deliver(m);
    };
    const onmessage = w.onmessage!;
    w.onmessage = (e) => {
      if ((e.data as { type: string }).type === "call") most = Math.max(most, ++onTheirWay);
      return onmessage(e);
    };
    await settle();
    commands.run("plugin:bulk.js:go");
    await vi.waitFor(() => expect(posted.find((m) => m.type === "probe")).toBeDefined());
    expect(posted.find((m) => m.type === "probe")).toMatchObject({ n: 5000, ok: true });
    expect(most).toBe(16);
    expect(w.terminated).toBe(false);
    expect(toasts).toEqual([]);
  });

  it("FINDING-162: of two calls with 9 million characters each, the second is sent once the first is answered", async () => {
    config.set("plugins/wr.js", `// @name Writer\n// @permissions write\ncairn.commands.register("go", "Go", () => { const big = "y".repeat(9 << 20); cairn.notes.write("a.md", big); cairn.notes.write("b.md", big); });\n`);
    const host = makeHost();
    const { posted } = await runRealWorker(() => host.sync(["wr.js"]));
    await settle();
    commands.run("plugin:wr.js:go");
    await Promise.resolve();
    expect(posted.filter((m) => m.type === "call").map((m) => m.args![0])).toEqual(["a.md"]);
    await settle();
    expect(posted.filter((m) => m.type === "call").map((m) => m.args![0])).toEqual(["a.md", "b.md"]);
    expect(files.get("b.md")).toHaveLength(9 << 20);
  });

  it("FINDING-072: a command id longer than 200 characters is refused, not cut (two such ids would become one)", async () => {
    const long = "i".repeat(200);
    config.set("plugins/ids.js", `// @name Ids\ncairn.commands.register("${long}", "Ok", () => {});\ncairn.commands.register("${long}x", "Too long", () => {});\n`);
    const host = makeHost();
    await runRealWorker(() => host.sync(["ids.js"]));
    await settle();
    expect(pluginCommands().map((c) => c.name)).toEqual(["Ids: Ok"]);
    expect(toasts.map((t) => t.message)).toEqual(["Ids: Plugin failed to start: Error: a command id can have at most 200 characters"]);
    // Sent to the host directly, ids that share their first 200 characters are refused as well.
    await FakeWorker.all[0].emit({ type: "register-command", id: `${long}a`, name: "A" });
    await FakeWorker.all[0].emit({ type: "register-command", id: `${long}b`, name: "B" });
    expect(pluginCommands().map((c) => c.name)).toEqual(["Ids: Ok"]);
  });

  it("FINDING-155: a plugin stuck in a busy loop since it loaded is stopped and turned off after 30 s", async () => {
    vi.useFakeTimers();
    config.set("plugins/spin.js", "// @name Spinner\n");
    config.set("plugins/ok.js", "// @name Fine\n");
    let enabled = ["spin.js", "ok.js"];
    const host = new PluginHost({
      notePaths: () => [],
      activePath: () => null,
      getSelection: () => null,
      replaceSelection: () => false,
      toast: (message, kind) => toasts.push({ message, kind }),
      disable: (file) => (enabled = enabled.filter((f) => f !== file)),
    });
    const spin = new Proxy(FakeWorker, {
      construct: (W, args: [string, { name: string }]) => Object.assign(new W(...args), { busy: args[1].name.endsWith("spin.js") }),
    });
    (globalThis as unknown as { Worker: unknown }).Worker = spin;
    await host.sync(enabled);
    const [ws, wo] = FakeWorker.all;
    vi.advanceTimersByTime(29_000);
    expect(ws.terminated).toBe(false);
    vi.advanceTimersByTime(1_500);
    expect(ws.terminated).toBe(true);
    expect(enabled).toEqual(["ok.js"]);
    expect(toasts.map((t) => t.message)).toEqual(["Plugin Spinner is not responding and was stopped."]);
    // A plugin that answers keeps running, however long it runs.
    vi.advanceTimersByTime(10 * 60_000);
    expect(wo.terminated).toBe(false);
  });

  it("FINDING-155: a plugin file that does not parse is stopped after 30 s as one that could not start", async () => {
    vi.useFakeTimers();
    config.set("plugins/broken.js", "// @name Broken\n");
    const host = makeHost();
    (globalThis as unknown as { Worker: unknown }).Worker = new Proxy(FakeWorker, {
      construct: (W, args: [string, { name: string }]) => Object.assign(new W(...args), { busy: true }),
    });
    await host.sync(["broken.js"]);
    const w = FakeWorker.all[0];
    // What the browser reports for a script that does not parse; the bootstrap never runs.
    w.onerror?.({ message: "SyntaxError: Unexpected token '}'" });
    vi.advanceTimersByTime(30_500);
    expect(w.terminated).toBe(true);
    expect(toasts.map((t) => t.message)).toEqual(["Plugin Broken: SyntaxError: Unexpected token '}'", "Plugin Broken could not start and was stopped."]);
  });

  it("FINDING-156: a command still running after 2 s is shown as running; a quicker one is not", async () => {
    vi.useFakeTimers();
    config.set("plugins/hang.js", "// @name Hanger\n");
    const host = makeHost();
    await host.sync(["hang.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "quick", name: "Quick" });
    await w.emit({ type: "register-command", id: "hang", name: "Hang" });
    commands.run("plugin:hang.js:quick");
    vi.advanceTimersByTime(1_000);
    await w.emit({ type: "command-done", id: "quick", run: runIds(w)[0] });
    commands.run("plugin:hang.js:hang");
    vi.advanceTimersByTime(1_900);
    expect(toasts).toEqual([]);
    vi.advanceTimersByTime(200);
    expect(toasts.map((t) => t.message)).toEqual(["Plugin Hanger: Hang is still running."]);
    // The 30 s limit still counts from when the command started.
    vi.advanceTimersByTime(27_800);
    expect(w.terminated).toBe(false);
    vi.advanceTimersByTime(200);
    expect(w.terminated).toBe(true);
  });

  it("FINDING-156: the still-running notice stays up until the command finishes, or until the plugin is stopped", async () => {
    vi.useFakeTimers();
    config.set("plugins/hang.js", "// @name Hanger\n");
    const shown = new Set<string>();
    const host = new PluginHost({
      notePaths: () => [],
      activePath: () => null,
      getSelection: () => null,
      replaceSelection: () => false,
      toast: (message, kind) => toasts.push({ message, kind }),
      notice: (message) => (shown.add(message), () => shown.delete(message)),
    });
    await host.sync(["hang.js"]);
    const w = FakeWorker.all[0];
    await w.emit({ type: "register-command", id: "slow", name: "Slow" });
    await w.emit({ type: "register-command", id: "hang", name: "Hang" });
    commands.run("plugin:hang.js:slow");
    commands.run("plugin:hang.js:hang");
    vi.advanceTimersByTime(10_000);
    expect([...shown]).toEqual(["Plugin Hanger: Slow is still running.", "Plugin Hanger: Hang is still running."]);
    await w.emit({ type: "command-done", id: "slow", run: runIds(w)[0] });
    expect([...shown]).toEqual(["Plugin Hanger: Hang is still running."]);
    vi.advanceTimersByTime(20_000);
    expect(w.terminated).toBe(true);
    expect([...shown]).toEqual([]);
    expect(toasts.map((t) => t.message)).toEqual(["Plugin Hanger took too long and was stopped."]);
  });

  it("a plugin sending a few thousand messages at once (one call per note) keeps running", async () => {
    config.set("plugins/bulk.js", "// @name Bulk\n// @permissions read\n");
    for (let i = 0; i < 5000; i++) files.set(`n${i}.md`, "x");
    const host = makeHost();
    await host.sync(["bulk.js"]);
    const w = FakeWorker.all[0];
    await Promise.all([...files.keys()].map((p, i) => w.emit({ type: "call", id: i, method: "notes.read", args: [p] })));
    expect(w.terminated).toBe(false);
    expect(w.sent).toHaveLength(5000);
  });
});
