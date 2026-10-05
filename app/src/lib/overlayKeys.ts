// Which keys the quick switcher and command palette keep for themselves while
// they are open, and which still run the global command bound to them.

type Keys = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

/**
 * The quick switcher's Ctrl+N / Ctrl+P: 1 for the next result, -1 for the
 * previous one, 0 for any other key. Control on every platform, as in Emacs,
 * also on macOS where Cmd is the command key.
 */
export function switcherStep(e: Keys): 1 | -1 | 0 {
  if (!e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return 0;
  // Lower case: Caps Lock makes the key "N".
  const k = e.key.toLowerCase();
  return k === "n" ? 1 : k === "p" ? -1 : 0;
}

// Commands that open an overlay in place of the switcher or palette.
const OVERLAY_COMMANDS = ["app:quick-switcher", "app:command-palette", "app:settings"];

/**
 * Whether the global hotkey handler leaves a key to the open quick switcher or
 * command palette instead of running the command bound to it. They are modal:
 * only a command that opens another overlay in their place runs. The switcher
 * keeps its Ctrl+N / Ctrl+P even so, where Ctrl+P opens the palette (Linux,
 * Windows); on macOS that is Cmd+P, which still swaps them.
 */
export function overlayKeepsKey(open: "switcher" | "palette" | null, commandId: string, e: Keys): boolean {
  if (!open) return false;
  if (open === "switcher" && switcherStep(e)) return true;
  return !OVERLAY_COMMANDS.includes(commandId);
}
