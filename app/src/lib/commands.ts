// Command registry, key handling and hotkey customization.
//
// Key combos are strings like "Mod+Shift+F": modifiers in the order
// Mod, Ctrl, Alt, Shift, then the key. "Mod" is Cmd on macOS and Ctrl
// elsewhere; "Ctrl" is only used on macOS for the real Control key.

export interface Command {
  id: string;
  name: string;
  run: () => void | Promise<void>;
  defaultKeys?: string[];
  /** Hidden from the palette when false. */
  available?: () => boolean;
}

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

const KEY_NAMES: Record<string, string> = {
  " ": "Space",
  arrowup: "Up",
  arrowdown: "Down",
  arrowleft: "Left",
  arrowright: "Right",
  escape: "Esc",
};

/** Normalize a keyboard event to a combo string, or null for bare modifiers. */
export function comboFromEvent(e: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">): string | null {
  const k = e.key.toLowerCase();
  if (["control", "meta", "alt", "shift", "os", "altgraph"].includes(k)) return null;
  const parts: string[] = [];
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (mod) parts.push("Mod");
  if (isMac && e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  // Keys go by the character typed, as in CodeMirror, so Ctrl+Z is the key
  // labelled Z and Ctrl+, the key labelled comma on AZERTY or Dvorak too.
  // The physical key names a letter key only when it types no ASCII character
  // (a non-Latin layout, a dead key) or with Alt, whose symbols vary by layout;
  // and a digit key whenever it types no digit (AZERTY's top row).
  let key = e.key;
  if (!/^[a-z0-9]$/i.test(key)) {
    if (/^Key[A-Z]$/.test(e.code)) {
      if (e.altKey || !/^[!-~]$/.test(key)) key = e.code.slice(3);
    } else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  }
  key = KEY_NAMES[key.toLowerCase()] ?? (key.length === 1 ? key.toUpperCase() : key);
  parts.push(key);
  return parts.join("+");
}

// Combos the editor needs for itself, so they cannot be hotkeys.
const EDITOR_KEYS: Record<string, string> = {
  "Mod+Z": "Undo",
  "Mod+Shift+Z": "Redo",
  "Mod+Y": "Redo",
  "Mod+A": "Select all",
  "Mod+C": "Copy",
  "Mod+X": "Cut",
  "Mod+V": "Paste",
};

/**
 * Why a combo cannot be a hotkey, as a short message, or null when it can.
 * A key without Ctrl, Alt or Cmd (except F1, F2, …) is for typing or moving
 * around, and the editor keeps undo and the clipboard keys.
 */
export function hotkeyProblem(combo: string): string | null {
  const mods = combo.match(/^(?:(?:Mod|Ctrl|Alt|Shift)\+)*/)![0];
  const key = combo.slice(mods.length);
  if (!/\b(?:Mod|Ctrl|Alt)\+/.test(mods) && !/^F\d{1,2}$/.test(key)) {
    return `${displayCombo(combo)} cannot be a hotkey. Add ${isMac ? "⌘, ⌃ or ⌥" : "Ctrl or Alt"}.`;
  }
  const use = EDITOR_KEYS[combo];
  return use ? `${displayCombo(combo)} is needed for ${use}. Pick another key.` : null;
}

/** Pretty form for display ("Mod+O" -> "Ctrl+O" or "⌘O"). */
export function displayCombo(combo: string): string {
  if (!isMac) return combo.replace(/\bMod\b/g, "Ctrl");
  return combo
    .split("+")
    .map((p) => ({ Mod: "⌘", Ctrl: "⌃", Alt: "⌥", Shift: "⇧" })[p] ?? p)
    .join("");
}

export class CommandRegistry {
  private commands = new Map<string, Command>();
  private overrides: Record<string, string[]> = {};

  register(cmds: Command[]) {
    for (const c of cmds) this.commands.set(c.id, c);
  }

  unregister(ids: Iterable<string>) {
    for (const id of ids) this.commands.delete(id);
  }

  /** Remove every command whose id starts with `prefix` (the tests use it to clear all plugin commands). */
  unregisterPrefix(prefix: string) {
    for (const id of [...this.commands.keys()]) if (id.startsWith(prefix)) this.commands.delete(id);
  }

  /** The commands that can run now (the palette's list). */
  all(): Command[] {
    return this.registered().filter((c) => c.available?.() ?? true);
  }

  /** Every command, whether it can run now or not (the hotkey list). */
  registered(): Command[] {
    return [...this.commands.values()];
  }

  get(id: string) {
    return this.commands.get(id);
  }

  setOverrides(o: Record<string, string[]>) {
    this.overrides = o;
  }

  /** A command's keys: its own in `overrides` (by default those set last), else its default keys. */
  keysFor(id: string, overrides = this.overrides): string[] {
    return overrides[id] ?? this.commands.get(id)?.defaultKeys ?? [];
  }

  /** Command bound to a combo, honoring overrides. */
  lookup(combo: string): Command | null {
    for (const c of this.commands.values()) {
      if (this.keysFor(c.id).includes(combo) && (c.available?.() ?? true)) return c;
    }
    return null;
  }

  /** Other commands already using a combo (for conflict warnings). */
  conflicts(combo: string, exceptId: string): Command[] {
    return [...this.commands.values()].filter((c) => c.id !== exceptId && this.keysFor(c.id).includes(combo));
  }

  run(id: string) {
    const c = this.commands.get(id);
    if (c) void c.run();
  }
}

export const commands = new CommandRegistry();
