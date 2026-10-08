// The page's reconcile of what the backend holds on Windows (held.ts).
//
// Run: cd app && npx vitest run src/lib/held.test.ts

import { describe, expect, it } from "vitest";
import { changes, wanted, wellFormed, type HeldTab, type Sent } from "./held";

const tab = (over: Partial<HeldTab> = {}): HeldTab => ({
  kind: "note",
  path: "A.md",
  dirty: true,
  conflict: null,
  saveFailed: false,
  baseHash: "B0",
  edit: 5,
  loaded: true,
  discarded: false,
  ...over,
});

describe("held", () => {
  it("wants every loaded note tab with edits that were not discarded", () => {
    const want = wanted([
      tab(),
      tab({ path: "Clean.md", dirty: false }),
      tab({ path: "Gone.md", discarded: true }),
      tab({ path: "Loading.md", loaded: false }),
      tab({ path: "pic.png", kind: "image" }),
      tab({ path: "Failed.md", saveFailed: true }),
      tab({ path: "Conflict.md", conflict: "deleted", saveFailed: true }),
    ]);
    expect([...want]).toEqual([
      ["A.md", { base: "B0", edit: 5, problem: null }],
      ["Failed.md", { base: "B0", edit: 5, problem: "failed" }],
      ["Conflict.md", { base: "B0", edit: 5, problem: "conflict" }],
    ]);
  });

  it("sends only what changed, text only when the backend lacks it", () => {
    const asked: string[] = [];
    const text = (p: string) => (asked.push(p), `text of ${p}`);
    const sent = new Map<string, Sent>([
      ["Same.md", { base: "B0", edit: 5, problem: null, text: true }],
      ["Rebased.md", { base: "B0", edit: 5, problem: null, text: true }],
      ["Big.md", { base: "B0", edit: 5, problem: null, text: false }],
      ["Fixed.md", { base: "B0", edit: 5, problem: "failed", text: false }],
      ["Closed.md", { base: "B0", edit: 5, problem: null, text: true }],
    ]);
    const want = new Map([
      ["Same.md", { base: "B0", edit: 5, problem: null }],
      ["Rebased.md", { base: "H1", edit: 5, problem: null }],
      ["Big.md", { base: "B0", edit: 5, problem: null }],
      ["Fixed.md", { base: "B0", edit: 5, problem: null }],
      ["New.md", { base: "B0", edit: 6, problem: null }],
    ] as const);
    const { notes, next } = changes(sent, want, text);
    expect(notes).toEqual([
      // The same edit with another base: the backend keeps its text.
      { path: "Rebased.md", base: "H1", edit: 5, problem: null },
      // Its problem went: the backend has no text for it.
      { path: "Fixed.md", base: "B0", edit: 5, problem: null, text: "text of Fixed.md" },
      { path: "New.md", base: "B0", edit: 6, problem: null, text: "text of New.md" },
      { path: "Closed.md", release: true },
    ]);
    expect(asked).toEqual(["Fixed.md", "New.md"]);
    expect(next.get("Rebased.md")).toEqual({ base: "H1", edit: 5, problem: null, text: true });
    expect(next.has("Closed.md")).toBe(false);
  });

  it("sends a problem without text, and null for text it cannot send", () => {
    const want = new Map([
      ["Failed.md", { base: "B0", edit: 5, problem: "failed" as const }],
      ["Big.md", { base: "B0", edit: 5, problem: null }],
    ]);
    const { notes, next } = changes(new Map(), want, () => null);
    expect(notes).toEqual([
      { path: "Failed.md", base: "B0", edit: 5, problem: "failed" },
      { path: "Big.md", base: "B0", edit: 5, problem: null, text: null },
    ]);
    expect(next.get("Big.md")?.text).toBe(false);
  });

  it("tells well-formed text from text with a lone surrogate", () => {
    expect(wellFormed("plain 😀 text")).toBe(true);
    expect(wellFormed("a\uD800b")).toBe(false);
    expect(wellFormed("a\uDC00")).toBe(false);
  });
});
