// Core plugins: optional features that are part of the app. Each has a switch in
// Settings > Core plugins and, for some, options of their own.
//
// They are app code, so unlike the plugins in .cairn/plugins/ they need no approval
// on each device: a vault's settings can only switch them on or off and set their
// options. None of them acts when a vault opens; they act only through their
// commands (and the buttons that run those). A note they make is created only where
// there is no file, and text they add goes into the editor like typing, so autosave
// writes it.
//
// The vault's settings.json holds, under "corePlugins", what the user changed, by
// plugin id:
//
//   "corePlugins": { "daily-notes": { "on": true, "folder": "Journal" } }
//
// A missing plugin or option means its default, so a later version can change a
// default. The object is kept as read: plugin ids and options this version does not
// know, and values of the wrong type (which read as the default), stay in the file
// until the user changes that setting.

import type { Command } from "../commands";
import { settings } from "../settings.svelte";

/** What core plugins can see and do in the app (app.svelte.ts provides it). */
export interface CoreHost {
  /** Every file in the vault, by vault path (no folders, nothing hidden). */
  files(): string[];
  /** Every folder in the vault. */
  folders(): string[];
  /** The note in the active tab (null: none, or the graph). */
  activeNote(): string | null;
  /** Whether the active tab is a note open for editing (not in reading view). */
  canInsert(): boolean;
  /**
   * Put `text` in place of the selection, as one edit that one undo takes back, if `path`
   * is still the note open for editing. False when it is not.
   */
  insert(path: string, text: string): boolean;
  /** The text of a note. */
  readNote(path: string): Promise<string>;
  /** Let the user pick one of `options`; null if they cancel. */
  choose(title: string, options: { value: string; label: string }[]): Promise<string | null>;
  /**
   * Create a note (and the folders on the way). Never over anything: if a file or folder
   * is at `path`, or one whose name differs only in case, it fails with a CoreError
   * "alreadyExists" whose detail is the path that is there.
   */
  createNote(path: string, content: string): Promise<void>;
  openNote(path: string, newTab?: boolean): Promise<void>;
  toast(message: string, kind?: "info" | "error"): void;
  now(): Date;
}

export interface CoreOption {
  key: string;
  label: string;
  description: string;
  /** Shown in the field while it is empty. */
  placeholder?: string;
  /**
   * An example of what `value` gives, or why it cannot work; Settings shows it under the
   * field. `get` gives the plugin's other options as they are in Settings at the moment.
   */
  check?(value: string, host: CoreHost, get: (key: string) => string): { example?: string; problem?: string };
}

export interface CoreCommand {
  /** Unique within its plugin; the command's id is `<plugin id>:<id>`. */
  id: string;
  name: string;
  run(host: CoreHost): void | Promise<void>;
  /** Hidden from the palette while false, as Command.available. */
  available?(host: CoreHost): boolean;
}

export interface CorePlugin {
  id: string;
  name: string;
  description: string;
  defaultOn: boolean;
  /** Option defaults, by key. Every option is text. */
  defaults: Record<string, string>;
  /** Shown under the plugin's row in Settings while it is on. */
  options: CoreOption[];
  commands: CoreCommand[];
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);

/** What settings.json holds for one plugin, if that is an object. */
function stored(id: string): Obj | null {
  const all = settings.value.corePlugins;
  const entry = isObj(all) ? all[id] : undefined;
  return isObj(entry) ? entry : null;
}

/** Whether a plugin is on: the user's choice, else its default. */
export function pluginOn(p: CorePlugin): boolean {
  const on = stored(p.id)?.on;
  return typeof on === "boolean" ? on : p.defaultOn;
}

/** One of a plugin's options: the stored text, else its default. */
export function option(p: CorePlugin, key: string): string {
  const v = stored(p.id)?.[key];
  return typeof v === "string" ? v : (p.defaults[key] ?? "");
}

/** Store one value of a plugin. Everything else under "corePlugins" stays as it is. */
function store(p: CorePlugin, key: string, value: unknown) {
  const all = settings.value.corePlugins;
  settings.update({ corePlugins: { ...(isObj(all) ? all : {}), [p.id]: { ...(stored(p.id) ?? {}), [key]: value } } });
}

export const setPluginOn = (p: CorePlugin, on: boolean) => store(p, "on", on);
export const setOption = (p: CorePlugin, key: string, value: string) => store(p, key, value);

/** The commands of `plugins`, for the registry. Each one runs, and is listed, only while its plugin is on. */
export function coreCommands(plugins: CorePlugin[], host: CoreHost): Command[] {
  return plugins.flatMap((p) =>
    p.commands.map((c) => ({
      id: `${p.id}:${c.id}`,
      name: `${p.name}: ${c.name}`,
      run: () => c.run(host),
      available: c.available && (() => c.available!(host)),
      enabled: () => pluginOn(p),
    })),
  );
}
