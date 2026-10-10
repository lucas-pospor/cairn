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

  it("sends a conflict without text, a failed save with it, and null for text it cannot send", () => {
    const want = new Map([
      ["Conflict.md", { base: "B0", edit: 5, problem: "conflict" as const }],
      ["Failed.md", { base: "B0", edit: 5, problem: "failed" as const }],
      ["Big.md", { base: "B0", edit: 5, problem: null }],
      ["BigFailed.md", { base: "B0", edit: 5, problem: "failed" as const }],
    ]);
    const text = (p: string) => (p.startsWith("Big") ? null : `text of ${p}`);
    const { notes, next } = changes(new Map(), want, text);
    expect(notes).toEqual([
      { path: "Conflict.md", base: "B0", edit: 5, problem: "conflict" },
      { path: "Failed.md", base: "B0", edit: 5, problem: "failed", text: "text of Failed.md" },
      { path: "Big.md", base: "B0", edit: 5, problem: null, text: null },
      { path: "BigFailed.md", base: "B0", edit: 5, problem: "failed", text: null },
    ]);
    expect(next.get("Conflict.md")?.text).toBe(false);
    expect(next.get("Failed.md")?.text).toBe(true);
    expect(next.get("Big.md")?.text).toBe(false);
  });

  it("sends the text again only when the backend dropped it for a conflict", () => {
    const asked: string[] = [];
    const text = (p: string) => (asked.push(p), `text of ${p}`);
    const sent = new Map<string, Sent>([
      ["Failing.md", { base: "B0", edit: 5, problem: null, text: true }],
      ["Recovered.md", { base: "B0", edit: 5, problem: "failed", text: true }],
      ["Clash.md", { base: "B0", edit: 5, problem: null, text: true }],
      ["Settled.md", { base: "B0", edit: 5, problem: "conflict", text: false }],
    ]);
    const want = new Map([
      ["Failing.md", { base: "B0", edit: 5, problem: "failed" }],
      ["Recovered.md", { base: "B0", edit: 5, problem: null }],
      ["Clash.md", { base: "B0", edit: 5, problem: "conflict" }],
      ["Settled.md", { base: "B0", edit: 5, problem: null }],
    ] as const);
    const { notes, next } = changes(sent, want, text);
    expect(notes).toEqual([
      // The backend keeps its text for the same edit.
      { path: "Failing.md", base: "B0", edit: 5, problem: "failed" },
      { path: "Recovered.md", base: "B0", edit: 5, problem: null },
      { path: "Clash.md", base: "B0", edit: 5, problem: "conflict" },
      { path: "Settled.md", base: "B0", edit: 5, problem: null, text: "text of Settled.md" },
    ]);
    expect(asked).toEqual(["Settled.md"]);
    expect(next.get("Failing.md")?.text).toBe(true);
    expect(next.get("Clash.md")?.text).toBe(false);
  });

  it("tells well-formed text from text with a lone surrogate", () => {
    expect(wellFormed("plain 😀 text")).toBe(true);
    expect(wellFormed("a\uD800b")).toBe(false);
    expect(wellFormed("a\uDC00")).toBe(false);
  });
});
