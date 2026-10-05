// Plugins: single JavaScript files in `<vault>/.cairn/plugins/`.
//
// Each enabled plugin runs in its own Web Worker. A worker has no DOM and no
// access to the page's Tauri bridge (the IPC key lives in the page, not the
// worker), and the app's Content Security Policy blocks network requests.
// The only way out is the `cairn` object below, whose calls are checked
// against the permissions the plugin declares in its header:
//
//   // @name Word count
//   // @description Shows how many words the selection has.
//   // @permissions editor
//
// Permissions: "read" (list and read notes), "write" (create or change
// notes), "editor" (read and replace the selection in the open note).
// Commands and toasts need no permission. "Notes" are the Markdown files
// notes.list returns: never anything in .cairn/, .git/, .trash/ or other
// hidden files and folders, and never a file reached through a symlink that
// leads out of the vault.
//
// The vault's settings.json lists the plugins that are on, but a vault can
// come from anyone, so that list alone starts nothing. A plugin runs only
// after the user turned it on in Settings > Plugins on this device: that
// records an approval (a hash of the plugin file and the permissions the user
// saw) in the app's own config folder, by vault. A listed plugin whose file
// has no matching approval stays off until the user turns it on again. Only
// files Settings > Plugins lists (top-level *.js in .cairn/plugins) can run.
//
// Limits: the host stops a plugin, and turns it off, when a command runs longer
// than 30 s, when it answers none of the host's pings for 30 s (a busy loop), or
// when it sends more than 20,000 messages in a second. A plugin can have at most
// 200 commands, and the text it shows is cut to 500 characters (names to 100).
// At most 16 of its calls are on their way to the app at a time; the rest wait in
// the plugin. A call made while 10,000 are waiting throws, and those fail with it.

import { backend } from "./backend";
import { commands } from "./commands";
import { isHidden, isMarkdown } from "./paths";
import { sha256Hex } from "./sha256";
import { errorMessage } from "./types";

export type Permission = "read" | "write" | "editor";

export interface PluginManifest {
  file: string;
  name: string;
  description: string;
  permissions: Permission[];
}

/** What the user approved on this device when turning a plugin on. */
export interface PluginApproval {
  /** sourceHash() of the plugin file the user saw. */
  hash: string;
  permissions: Permission[];
}

/** A plugin file as Settings > Plugins shows it. */
export interface PluginInfo extends PluginManifest {
  hash: string;
  /** Turned on on this device for exactly this file. */
  approved: boolean;
}

export interface PluginHostApi {
  notePaths(): string[];
  activePath(): string | null;
  getSelection(): string | null;
  replaceSelection(text: string): boolean;
  toast(message: string, kind?: "info" | "error"): void;
  /** Show `message` until the returned function is called (a command still running). */
  notice?(message: string): () => void;
  /** Turn a plugin off in the settings (it was stopped for taking too long). */
  disable?(file: string): void;
}

const CALL_TIMEOUT_MS = 10000;
/** A command still running after this long is shown as running. */
const SLOW_MS = 2000;
const KNOWN: Permission[] = ["read", "write", "editor"];
// Limits for text a plugin controls: what the host shows (toasts, the plugin's
// name and description, command names; longer text is cut) and command ids (a longer
// one is refused).
const MAX_TEXT = 500;
const MAX_NAME = 100;
const MAX_ID = 200;
/** Commands one plugin can have; more would make the palette slow to open. */
const MAX_COMMANDS = 200;
/** Messages a plugin can send in one second. A plugin posting in a loop fills the page's memory. */
const MAX_MESSAGES_PER_S = 20000;
/** How often the host pings each plugin; 3 pings in a row unanswered stop it. */
const PING_MS = CALL_TIMEOUT_MS;
// A plugin calling the API in a loop can send calls much faster than the app answers them,
// and every message on its way takes memory in the page. So the bootstrap sends at most
// MAX_IN_FLIGHT calls (with MAX_IN_FLIGHT_CHARS characters of arguments) at a time and keeps
// the rest. A call made while MAX_WAITING are kept throws, and the kept ones fail with it.
const MAX_IN_FLIGHT = 16;
const MAX_IN_FLIGHT_CHARS = 16 << 20;
const MAX_WAITING = 10000;

/** `s` as a string of at most `max` characters, "…" marking a cut. */
const clip = (s: unknown, max = MAX_TEXT) => {
  const t = String(s);
  return t.length > max ? t.slice(0, max - 1) + "…" : t;
};

/** A path plugins may read or write: a Markdown note outside hidden folders. */
const isNotePath = (p: string) => isMarkdown(p) && !isHidden(p);

export function parseManifest(file: string, source: string): PluginManifest {
  const header = (key: string) => new RegExp(`^\\s*//\\s*@${key}\\s+(.+)$`, "m").exec(source)?.[1].trim() ?? "";
  const perms = header("permissions")
    .split(/[\s,]+/)
    .filter((p): p is Permission => (KNOWN as string[]).includes(p));
  return {
    file,
    name: clip(header("name") || file.replace(/\.js$/, ""), MAX_NAME),
    description: clip(header("description")),
    permissions: perms,
  };
}

/** What an approval records of a plugin file: its SHA-256, in hex. */
export const sourceHash = (source: string) => sha256Hex(source);

/** The approval covers this exact file and every permission it asks for. */
export function isApproved(approval: PluginApproval | undefined, hash: string, permissions: Permission[]): boolean {
  return !!approval && approval.hash === hash && permissions.every((p) => approval.permissions.includes(p));
}

/** Which permission a host method needs (null = always allowed). */
export function permissionFor(method: string): Permission | null | undefined {
  switch (method) {
    case "notes.list":
    case "notes.read":
      return "read";
    case "notes.write":
      return "write";
    case "editor.getSelection":
    case "editor.replaceSelection":
    case "editor.activePath":
      return "editor";
    case "ui.toast":
      return null;
    default:
      return undefined; // unknown method
  }
}

// Runs inside the worker before the plugin's own code.
const BOOTSTRAP = `
"use strict";
// Calls sent and not answered yet, and calls waiting to be sent, by id (ids go up by one).
const __pending = new Map();
const __waiting = new Map();
let __seq = 0;
let __sent = 0;
let __chars = 0;
const __handlers = new Map();
const __clip = (s, max) => (s.length > max ? s.slice(0, max - 1) + "\\u2026" : s);
const __send = () => {
  while (__sent < __seq && __pending.size < ${MAX_IN_FLIGHT}) {
    const c = __waiting.get(__sent + 1);
    if (__pending.size && __chars + c.chars > ${MAX_IN_FLIGHT_CHARS}) break;
    __waiting.delete(++__sent);
    __pending.set(__sent, c);
    __chars += c.chars;
    postMessage({ type: "call", id: __sent, method: c.method, args: c.args });
    c.args = null;
  }
};
const __call = (method, ...args) => {
  if (__waiting.size >= ${MAX_WAITING}) {
    // A loop making calls ends here, and the calls it left waiting fail too instead of keeping
    // the app busy answering them afterwards (their failures are not reported again).
    const err = new Error("too many calls are waiting (${MAX_WAITING})");
    for (const c of __waiting.values()) {
      c.promise.catch(() => {});
      c.reject(err);
    }
    __waiting.clear();
    __sent = __seq;
    throw err;
  }
  const c = { method, args, chars: args.reduce((n, a) => n + a.length, 0) };
  c.promise = new Promise((resolve, reject) => Object.assign(c, { resolve, reject }));
  __waiting.set(++__seq, c);
  __send();
  return c.promise;
};
// An error at load is shown even when calls are waiting (its answer, id 0, is ignored).
const __report = (text) => postMessage({ type: "call", id: 0, method: "ui.toast", args: [__clip(text, ${MAX_TEXT})] });
self.onmessage = async (e) => {
  const m = e.data;
  if (m.type === "result") {
    const p = __pending.get(m.id);
    __pending.delete(m.id);
    if (p) {
      __chars -= p.chars;
      m.error ? p.reject(new Error(m.error)) : p.resolve(m.value);
      __send();
    }
  } else if (m.type === "ping") {
    postMessage({ type: "pong" });
  } else if (m.type === "run-command") {
    try {
      await __handlers.get(m.id)?.();
      postMessage({ type: "command-done", id: m.id, run: m.run });
    } catch (err) {
      postMessage({ type: "command-done", id: m.id, run: m.run, error: String(err && err.message || err).slice(0, 1000) });
    }
  }
};
const cairn = Object.freeze({
  commands: Object.freeze({
    register(id, name, handler) {
      id = String(id);
      if (id.length > ${MAX_ID}) throw new Error("a command id can have at most ${MAX_ID} characters");
      __handlers.set(id, handler);
      postMessage({ type: "register-command", id, name: __clip(String(name), ${MAX_NAME}) });
    },
  }),
  notes: Object.freeze({
    list: () => __call("notes.list"),
    read: (path) => __call("notes.read", String(path)),
    write: (path, content) => __call("notes.write", String(path), String(content)),
  }),
  editor: Object.freeze({
    activePath: () => __call("editor.activePath"),
    getSelection: () => __call("editor.getSelection"),
    replaceSelection: (text) => __call("editor.replaceSelection", String(text)),
  }),
  ui: Object.freeze({ toast: (message) => __call("ui.toast", __clip(String(message), ${MAX_TEXT})) }),
});
self.cairn = cairn;
`;

interface Running {
  manifest: PluginManifest;
  worker: Worker;
  url: string;
  /** Ids of the commands it registered. */
  commandIds: Set<string>;
  /** It tried to register more than MAX_COMMANDS (the user was told once). */
  tooManyCommands: boolean;
  /** Timers of command runs not finished yet ("still running", then "took too long"), by run id. */
  timers: Map<number, ReturnType<typeof setTimeout>>;
  /** Closes the "still running" notice of a run, by run id. */
  notices: Map<number, () => void>;
  /** Start of the current one-second window and the messages received in it. */
  windowStart: number;
  messages: number;
  /** Pings it has not answered yet, and the timer that sends them. */
  unanswered: number;
  /** It answered a ping at least once; it reported an error before that (a file that does not parse). */
  answered: boolean;
  startError: boolean;
  heartbeat?: ReturnType<typeof setInterval>;
}

export class PluginHost {
  private running = new Map<string, Running>();
  /** Plugins whose file is still being read, with a token for that start; stop() removes it to cancel the start. */
  private starting = new Map<string, object>();
  /** Bumped by every sync() and stopAll(), so that an older sync() stops starting plugins. */
  private generation = 0;
  /** Numbers each command run; the worker sends it back with "command-done". */
  private runs = 0;
  /**
   * Plugins the host stopped for misbehaving (a command taking too long, no answer to its pings,
   * too many messages). sync() skips them even if its list still has them
   * (a sync() already under way, or a host without api.disable), until the user turns them
   * on again (turnedOn()), a sync() is called without them, or stopAll().
   */
  private timedOut = new Set<string>();

  constructor(private api: PluginHostApi) {}

  /** The plugin files Settings > Plugins lists: the *.js files directly in .cairn/plugins. */
  private async files(): Promise<string[]> {
    try {
      return (await backend.listConfig("plugins")).filter((f) => f.endsWith(".js"));
    } catch {
      return [];
    }
  }

  /** Approvals on this device for the open vault (none if they cannot be read). */
  private async approvals(): Promise<Record<string, PluginApproval>> {
    try {
      return await backend.pluginApprovals();
    } catch (e) {
      console.warn("plugin approvals unreadable", e);
      return {};
    }
  }

  async available(): Promise<PluginInfo[]> {
    const files = await this.files();
    const approvals = files.length ? await this.approvals() : {};
    const out: PluginInfo[] = [];
    for (const f of files) {
      const src = (await backend.readConfig(`plugins/${f}`)) ?? "";
      const manifest = parseManifest(f, src);
      const hash = sourceHash(src);
      out.push({ ...manifest, hash, approved: isApproved(approvals[f], hash, manifest.permissions) });
    }
    return out;
  }

  /** The user turned a plugin on after seeing `p`: approve that file and those permissions on this device. */
  async approve(p: PluginInfo) {
    await backend.setPluginApproval(p.file, { hash: p.hash, permissions: p.permissions });
  }

  /** The user turned a plugin off: it needs approving again before it runs. */
  async revoke(file: string) {
    await backend.setPluginApproval(file, null);
  }

  /**
   * Start exactly the enabled plugins (stopping the others). Returns the enabled plugins
   * that did not start because they are not approved on this device.
   */
  async sync(enabled: string[]): Promise<string[]> {
    const generation = ++this.generation;
    for (const file of this.started()) if (!enabled.includes(file)) this.stop(file);
    for (const file of this.timedOut) if (!enabled.includes(file)) this.timedOut.delete(file);
    // Entries Settings > Plugins cannot list (subfolders, other file types) never start.
    const files = await this.files();
    const approvals = files.length ? await this.approvals() : {};
    const off: string[] = [];
    for (const file of enabled) {
      // A newer sync() or stopAll() (another toggle, Reload, vault switch) takes over.
      if (generation !== this.generation) return [];
      if (!files.includes(file) || this.timedOut.has(file)) continue;
      if (!(await this.start(file, approvals[file]))) off.push(file);
    }
    return generation === this.generation ? off : [];
  }

  /** The user turned a plugin on: the next sync() starts it even if it was stopped for taking too long. */
  turnedOn(file: string) {
    this.timedOut.delete(file);
  }

  stopAll() {
    this.generation++;
    this.timedOut.clear();
    for (const file of this.started()) this.stop(file);
  }

  /** Plugins running or starting. */
  private started() {
    return [...new Set([...this.running.keys(), ...this.starting.keys()])];
  }

  stop(file: string) {
    this.starting.delete(file);
    const r = this.running.get(file);
    if (!r) return;
    for (const t of r.timers.values()) clearTimeout(t);
    for (const close of r.notices.values()) close();
    clearInterval(r.heartbeat);
    r.worker.terminate();
    URL.revokeObjectURL(r.url);
    commands.unregister(r.commandIds);
    this.running.delete(file);
  }

  /** Start one plugin if `approval` covers its file. Returns false if it does not. */
  private async start(file: string, approval: PluginApproval | undefined): Promise<boolean> {
    // Already running, or started a moment ago and its file is still being read.
    if (this.running.has(file) || this.starting.has(file)) return true;
    const token = {};
    this.starting.set(file, token);
    let source: string | null;
    try {
      source = await backend.readConfig(`plugins/${file}`);
    } finally {
      // stop() meanwhile (disabled, Reload, vault switch) cancels this start.
      if (this.starting.get(file) === token) this.starting.delete(file);
      else source = null;
    }
    if (source == null) return true;
    const manifest = parseManifest(file, source);
    // The code that runs is the code that was hashed, so a file changed after approval never runs.
    if (!isApproved(approval, sourceHash(source), manifest.permissions)) return false;
    // The plugin's code runs in its own function scope after the bootstrap.
    const blob = new Blob([BOOTSTRAP, "\n;(async () => {\n", source, "\n})().catch((e) => __report('Plugin failed to start: ' + e));\n"], {
      type: "text/javascript",
    });
    const url = URL.createObjectURL(blob);
    let worker: Worker;
    try {
      worker = new Worker(url, { name: `cairn-plugin-${file}` });
    } catch (e) {
      URL.revokeObjectURL(url);
      this.api.toast(`Plugin ${manifest.name} could not start: ${errorMessage(e)}`, "error");
      return true;
    }
    const running: Running = { manifest, worker, url, commandIds: new Set(), tooManyCommands: false, timers: new Map(), notices: new Map(), windowStart: 0, messages: 0, unanswered: 0, answered: false, startError: false };
    this.running.set(file, running);
    const current = () => this.running.get(file) === running;
    // Its name in the palette and in toasts, with the file name while another running plugin has the same name.
    const label = () =>
      [...this.running.values()].some((r) => r !== running && r.manifest.name === manifest.name) ? `${manifest.name} (${file})` : manifest.name;
    // Stop it for misbehaving. It stays off (and is shown as off) until the user turns it on again.
    const kill = (message: string) => {
      this.api.toast(message, "error");
      this.stop(file);
      this.timedOut.add(file);
      this.api.disable?.(file);
    };
    worker.onerror = (e) => {
      if (!running.answered) running.startError = true;
      this.api.toast(`Plugin ${label()}: ${clip(e.message)}`, "error");
    };
    worker.onmessage = async (e: MessageEvent) => {
      // A message that was already on its way when this instance was stopped.
      if (!current()) return;
      const now = Date.now();
      if (now - running.windowStart >= 1000) {
        running.windowStart = now;
        running.messages = 0;
      }
      if (++running.messages > MAX_MESSAGES_PER_S) return kill(`Plugin ${label()} sent too many messages and was stopped.`);
      const m = e.data as { type: string; id: string | number; run?: number; name?: string; method?: string; args?: unknown[]; error?: string };
      if (m.type === "register-command") {
        // A longer id is refused (the bootstrap throws), not cut, so two ids cannot become one.
        const id = String(m.id);
        if (id.length > MAX_ID) return;
        // The command's own part has its ":" escaped, so plugin "a.js" registering "b.js:y"
        // does not get the same id as plugin "a.js:b.js" registering "y".
        const cmdId = `plugin:${file}:${id.replaceAll("%", "%25").replaceAll(":", "%3A")}`;
        // Past MAX_COMMANDS only commands it already has can be registered again.
        if (!running.commandIds.has(cmdId) && running.commandIds.size >= MAX_COMMANDS) {
          if (!running.tooManyCommands) {
            running.tooManyCommands = true;
            this.api.toast(`Plugin ${label()} has more than ${MAX_COMMANDS} commands. Only the first ${MAX_COMMANDS} are in the palette.`, "error");
          }
          return;
        }
        running.commandIds.add(cmdId);
        const name = clip(m.name, MAX_NAME);
        commands.register([
          {
            id: cmdId,
            get name() {
              return `${label()}: ${name}`;
            },
            run: () => {
              // A palette entry still showing a stopped instance does nothing.
              if (!current()) return;
              const run = ++this.runs;
              worker.postMessage({ type: "run-command", id: m.id, run });
              // A command that never finishes stops the plugin. Each run has its own
              // timer (the same command can run again before the first run is done).
              // After SLOW_MS the user is told it is still running, until it finishes or is stopped.
              const slow = setTimeout(() => {
                const text = `Plugin ${label()}: ${name} is still running.`;
                if (this.api.notice) running.notices.set(run, this.api.notice(text));
                else this.api.toast(text);
                const limit = setTimeout(() => kill(`Plugin ${label()} took too long and was stopped.`), CALL_TIMEOUT_MS * 3 - SLOW_MS);
                running.timers.set(run, limit);
              }, SLOW_MS);
              running.timers.set(run, slow);
            },
          },
        ]);
      } else if (m.type === "pong") {
        running.unanswered = 0;
        running.answered = true;
      } else if (m.type === "command-done") {
        clearTimeout(running.timers.get(Number(m.run)));
        running.timers.delete(Number(m.run));
        running.notices.get(Number(m.run))?.();
        running.notices.delete(Number(m.run));
        if (m.error) this.api.toast(`Plugin ${label()}: ${clip(m.error)}`, "error");
      } else if (m.type === "call") {
        try {
          const value = await this.handle({ ...manifest, name: label() }, m.method ?? "", m.args ?? []);
          worker.postMessage({ type: "result", id: m.id, value });
        } catch (err) {
          worker.postMessage({ type: "result", id: m.id, error: errorMessage(err) });
        }
      }
    };
    // A plugin that never gets back to its event loop (a busy loop when it loads, or in a
    // timer; also a file that does not parse) cannot answer pings: it is stopped once 3 in
    // a row, over 30 s, go unanswered. Counting pings rather than time means a suspended
    // computer does not count. A command stuck in a loop hits its own 30 s limit first.
    const ping = () => {
      if (running.unanswered >= 3) {
        return kill(running.startError ? `Plugin ${label()} could not start and was stopped.` : `Plugin ${label()} is not responding and was stopped.`);
      }
      running.unanswered++;
      worker.postMessage({ type: "ping" });
    };
    ping();
    running.heartbeat = setInterval(ping, PING_MS);
    return true;
  }

  /** Answer one API call from a plugin, enforcing its permissions. */
  async handle(manifest: PluginManifest, method: string, args: unknown[]): Promise<unknown> {
    const need = permissionFor(method);
    if (need === undefined) throw new Error(`unknown API ${method}`);
    if (need && !manifest.permissions.includes(need)) {
      throw new Error(`${manifest.name} does not have the "${need}" permission (needed for ${method})`);
    }
    const str = (i: number) => String(args[i] ?? "");
    switch (method) {
      case "ui.toast":
        this.api.toast(`${manifest.name}: ${clip(str(0))}`);
        return null;
      case "notes.list":
        return this.api.notePaths();
      case "notes.read": {
        const path = str(0);
        if (!isNotePath(path)) throw new Error("plugins can only read Markdown notes, not hidden files");
        return (await backend.readNote(path, true)).content;
      }
      case "notes.write": {
        const path = str(0);
        if (!isNotePath(path)) throw new Error("plugins can only write Markdown notes, not hidden files");
        try {
          const cur = await backend.readNote(path, true);
          await backend.writeNote(path, str(1), cur.hash, true);
        } catch (e) {
          if ((e as { kind?: string })?.kind !== "notFound") throw e;
          await backend.createNote(path, str(1), true);
        }
        return null;
      }
      case "editor.activePath":
        return this.api.activePath();
      case "editor.getSelection":
        return this.api.getSelection();
      case "editor.replaceSelection":
        return this.api.replaceSelection(str(0));
    }
    return null;
  }
}
