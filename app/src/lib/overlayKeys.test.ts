// Keys the quick switcher and command palette keep while they are open
// (app/src/lib/overlayKeys.ts, used by Workspace's global hotkey handler).
//
// Run: cd app && npx vitest run src/lib/overlayKeys.test.ts

import { describe, expect, it } from "vitest";
import { overlayKeepsKey, switcherStep } from "./overlayKeys";

const key = (k: string, mods: { ctrl?: boolean; meta?: boolean; alt?: boolean; shift?: boolean } = {}) => ({
  key: k,
  ctrlKey: !!mods.ctrl,
  metaKey: !!mods.meta,
  altKey: !!mods.alt,
  shiftKey: !!mods.shift,
});

describe("switcherStep", () => {
  it("Ctrl+N and Ctrl+P move to the next and previous result", () => {
    expect(switcherStep(key("n", { ctrl: true }))).toBe(1);
    expect(switcherStep(key("p", { ctrl: true }))).toBe(-1);
    // Caps Lock on.
    expect(switcherStep(key("N", { ctrl: true }))).toBe(1);
    expect(switcherStep(key("P", { ctrl: true }))).toBe(-1);
  });

  it("is Control on macOS too: Cmd+N and Cmd+P are not steps", () => {
    expect(switcherStep(key("n", { meta: true }))).toBe(0);
    expect(switcherStep(key("p", { meta: true }))).toBe(0);
    expect(switcherStep(key("p", { ctrl: true, meta: true }))).toBe(0);
  });

  it("other keys and other modifiers are not steps", () => {
    expect(switcherStep(key("p"))).toBe(0);
    expect(switcherStep(key("o", { ctrl: true }))).toBe(0);
    expect(switcherStep(key("P", { ctrl: true, shift: true }))).toBe(0);
    expect(switcherStep(key("p", { ctrl: true, alt: true }))).toBe(0);
  });
});

describe("overlayKeepsKey", () => {
  it("keeps nothing while no overlay is open: Ctrl+P from the editor opens the palette (FINDING-106)", () => {
    expect(overlayKeepsKey(null, "app:command-palette", key("p", { ctrl: true }))).toBe(false);
    expect(overlayKeepsKey(null, "note:new", key("n", { ctrl: true }))).toBe(false);
  });

  it("the quick switcher keeps Ctrl+P (previous result) where it is the palette's key, on Linux and Windows (FINDING-208)", () => {
    expect(overlayKeepsKey("switcher", "app:command-palette", key("p", { ctrl: true }))).toBe(true);
  });

  it("the quick switcher keeps Ctrl+N (next result) instead of creating a note (FINDING-208)", () => {
    expect(overlayKeepsKey("switcher", "note:new", key("n", { ctrl: true }))).toBe(true);
  });

  it("the quick switcher keeps Ctrl+N / Ctrl+P whatever they are bound to", () => {
    expect(overlayKeepsKey("switcher", "app:settings", key("p", { ctrl: true }))).toBe(true);
    expect(overlayKeepsKey("switcher", "app:quick-switcher", key("n", { ctrl: true }))).toBe(true);
  });

  it("on macOS Cmd+P still swaps the switcher for the palette (FINDING-106)", () => {
    expect(overlayKeepsKey("switcher", "app:command-palette", key("p", { meta: true }))).toBe(false);
  });

  it("a rebound palette key still swaps the switcher for the palette", () => {
    expect(overlayKeepsKey("switcher", "app:command-palette", key("P", { ctrl: true, shift: true }))).toBe(false);
  });

  it("the other overlay commands still swap one overlay for another (FINDING-106)", () => {
    expect(overlayKeepsKey("switcher", "app:settings", key(",", { ctrl: true }))).toBe(false);
    expect(overlayKeepsKey("switcher", "app:quick-switcher", key("o", { ctrl: true }))).toBe(false);
    expect(overlayKeepsKey("palette", "app:quick-switcher", key("o", { ctrl: true }))).toBe(false);
    expect(overlayKeepsKey("palette", "app:settings", key(",", { ctrl: true }))).toBe(false);
  });

  it("the palette has no Ctrl+N / Ctrl+P of its own: Ctrl+P there is still the palette command", () => {
    expect(overlayKeepsKey("palette", "app:command-palette", key("p", { ctrl: true }))).toBe(false);
    expect(overlayKeepsKey("palette", "note:new", key("n", { ctrl: true }))).toBe(true);
  });

  it("the switcher and palette keep every other command, so nothing runs behind them", () => {
    expect(overlayKeepsKey("switcher", "app:graph", key("g", { ctrl: true }))).toBe(true);
    expect(overlayKeepsKey("palette", "app:graph", key("g", { ctrl: true }))).toBe(true);
    expect(overlayKeepsKey("palette", "editor:bold", key("b", { ctrl: true }))).toBe(true);
  });
});
