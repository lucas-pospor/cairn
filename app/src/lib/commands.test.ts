// Key combos and the command registry (app/src/lib/commands.ts).
//
// Run: cd app && npx vitest run src/lib/commands.test.ts

import { describe, expect, it } from "vitest";
import { CommandRegistry, comboFromEvent, hotkeyProblem } from "./commands";

const ev = (key: string, code: string, mods: { ctrl?: boolean; alt?: boolean; shift?: boolean } = {}) => ({
  key,
  code,
  ctrlKey: !!mods.ctrl,
  metaKey: false,
  altKey: !!mods.alt,
  shiftKey: !!mods.shift,
});

describe("comboFromEvent", () => {
  it("names letters by the character typed, whatever the layout (FINDING-040)", () => {
    // AZERTY: the key labelled Z sits where QWERTY has W.
    expect(comboFromEvent(ev("z", "KeyW", { ctrl: true }))).toBe("Mod+Z");
    // Dvorak: the key labelled C sits where QWERTY has I.
    expect(comboFromEvent(ev("c", "KeyI", { ctrl: true }))).toBe("Mod+C");
    expect(comboFromEvent(ev("F", "KeyF", { ctrl: true, shift: true }))).toBe("Mod+Shift+F");
    // AZERTY's top row types digits with Shift.
    expect(comboFromEvent(ev("1", "Digit1", { ctrl: true, shift: true }))).toBe("Mod+Shift+1");
    // AZERTY: Alt with a letter keeps the letter typed.
    expect(comboFromEvent(ev("z", "KeyW", { alt: true }))).toBe("Alt+Z");
  });

  it("names punctuation by the character typed, also where QWERTY has a letter (FINDING-040)", () => {
    // Dvorak: the comma key sits where QWERTY has W, the full stop where it has E.
    expect(comboFromEvent(ev(",", "KeyW", { ctrl: true }))).toBe("Mod+,");
    expect(comboFromEvent(ev(".", "KeyE", { ctrl: true }))).toBe("Mod+.");
    expect(comboFromEvent(ev("<", "KeyW", { ctrl: true, shift: true }))).toBe("Mod+Shift+<");
    // AZERTY: the comma key sits where QWERTY has M.
    expect(comboFromEvent(ev(",", "KeyM", { ctrl: true }))).toBe("Mod+,");
  });

  it("falls back to the physical key when the character is not a Latin letter or digit", () => {
    // Russian layout: Ctrl+Я is Ctrl+Z.
    expect(comboFromEvent(ev("я", "KeyZ", { ctrl: true }))).toBe("Mod+Z");
    // Alt or Shift making a symbol.
    expect(comboFromEvent(ev("©", "KeyG", { alt: true }))).toBe("Alt+G");
    // AltGr (Ctrl+Alt) on a German keyboard types @ on the Q key.
    expect(comboFromEvent(ev("@", "KeyQ", { ctrl: true, alt: true }))).toBe("Mod+Alt+Q");
    expect(comboFromEvent(ev("Dead", "KeyU", { alt: true }))).toBe("Alt+U");
    expect(comboFromEvent(ev("Unidentified", "KeyZ", { ctrl: true }))).toBe("Mod+Z");
    expect(comboFromEvent(ev("!", "Digit1", { ctrl: true, shift: true }))).toBe("Mod+Shift+1");
    // AZERTY's top row without Shift.
    expect(comboFromEvent(ev("&", "Digit1", { ctrl: true }))).toBe("Mod+1");
  });

  it("keeps named keys and punctuation", () => {
    expect(comboFromEvent(ev(",", "Comma", { ctrl: true }))).toBe("Mod+,");
    expect(comboFromEvent(ev("Tab", "Tab", { ctrl: true, shift: true }))).toBe("Mod+Shift+Tab");
    expect(comboFromEvent(ev("ArrowUp", "ArrowUp", { alt: true }))).toBe("Alt+Up");
    expect(comboFromEvent(ev("Control", "ControlLeft", { ctrl: true }))).toBeNull();
  });
});

describe("CommandRegistry", () => {
  it("lists every command for the hotkey settings, also those that cannot run now (FINDING-097, FINDING-123)", () => {
    const r = new CommandRegistry();
    r.register([
      { id: "a", name: "A", run: () => {} },
      { id: "b", name: "B", run: () => {}, available: () => false },
    ]);
    expect(r.all().map((c) => c.id)).toEqual(["a"]);
    expect(r.registered().map((c) => c.id)).toEqual(["a", "b"]);
  });

  it("gives a command's keys from given overrides, for hints that follow the settings (FINDING-211)", () => {
    const r = new CommandRegistry();
    r.register([{ id: "note:new", name: "New", run: () => {}, defaultKeys: ["Mod+N"] }]);
    r.setOverrides({ "note:new": ["Mod+M"] });
    expect(r.keysFor("note:new")).toEqual(["Mod+M"]);
    expect(r.keysFor("note:new", { "note:new": ["Mod+Alt+J"] })).toEqual(["Mod+Alt+J"]);
    expect(r.keysFor("note:new", {})).toEqual(["Mod+N"]);
  });
});

describe("hotkeyProblem", () => {
  it("refuses keys needed for typing and the editor's own keys (FINDING-096, FINDING-214)", () => {
    for (const c of ["G", "Shift+G", "Q", "Enter", "Backspace", "Space", "Tab", "Shift+Tab", "Up", "Delete", "Shift++"]) {
      expect(hotkeyProblem(c), c).toMatch(/cannot be a hotkey\. Add Ctrl or Alt\.$/);
    }
    expect(hotkeyProblem("Mod+Z")).toBe("Ctrl+Z is needed for Undo. Pick another key.");
    for (const c of ["Mod+Shift+Z", "Mod+Y", "Mod+A", "Mod+C", "Mod+X", "Mod+V"]) expect(hotkeyProblem(c), c).toMatch(/is needed for/);
  });

  it("accepts combos with Ctrl, Alt or Cmd, and function keys", () => {
    for (const c of ["Mod+G", "Mod+Shift+F", "Alt+G", "Mod+Alt+J", "Mod+Tab", "Mod++", "F5", "Shift+F12", "Mod+Enter"]) {
      expect(hotkeyProblem(c), c).toBeNull();
    }
  });
});
