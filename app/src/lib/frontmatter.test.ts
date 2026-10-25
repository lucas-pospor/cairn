// Properties as text (frontmatter.ts): top-level keys, and templates whose
// properties go into the note's.
//
// Run: cd app && npx vitest run src/lib/frontmatter.test.ts

import { describe, expect, it } from "vitest";
import { propertyBlocks, templateEdit, type TextEdit } from "./frontmatter";

/** `doc` after `edit`, with `|` at the cursor. */
function apply(doc: string, edit: TextEdit): string {
  let out = doc;
  for (const c of [...edit.changes].reverse()) out = out.slice(0, c.from) + c.insert + out.slice(c.to);
  return out.slice(0, edit.cursor) + "|" + out.slice(edit.cursor);
}

/** Insert `text` at the `|` in `doc`. */
function insert(doc: string, text: string): string {
  const at = doc.indexOf("|");
  const d = doc.slice(0, at) + doc.slice(at + 1);
  return apply(d, templateEdit(d, at, at, text));
}

describe("propertyBlocks", () => {
  it("splits YAML at its top-level keys, nested lines and comments included", () => {
    const yaml = "# about\ntitle: Hi\ntags:\n  - a\n- b\nnested:\n  x: 1\n\n# end\n\"quoted key\": v\n'it''s': w\nurl: http://x:80\n";
    expect(propertyBlocks(yaml)).toEqual([
      { key: "title", text: "# about\ntitle: Hi" },
      { key: "tags", text: "tags:\n  - a\n- b" },
      { key: "nested", text: "nested:\n  x: 1\n\n# end" },
      { key: "quoted key", text: '"quoted key": v' },
      { key: "it's", text: "'it''s': w" },
      { key: "url", text: "url: http://x:80" },
    ]);
  });

  it("gives no blocks for empty YAML, and null for YAML that is not a mapping", () => {
    expect(propertyBlocks("")).toEqual([]);
    expect(propertyBlocks("\n# only a comment\n")).toEqual([]);
    expect(propertyBlocks("- a\n- b")).toBeNull();
    expect(propertyBlocks("  indented: 1")).toBeNull();
    expect(propertyBlocks("{a: 1}")).toBeNull();
    expect(propertyBlocks("just text")).toBeNull();
  });
});

describe("templateEdit", () => {
  it("inserts a template without properties at the cursor, as it is", () => {
    expect(insert("Hello |world", "# T\n")).toBe("Hello # T\n|world");
  });

  it("adds the template's properties to a note that has none, at the top", () => {
    expect(insert("Intro\n|", "---\ntags: [daily]\n---\n# Log\n")).toBe("---\ntags: [daily]\n---\nIntro\n# Log\n|");
    // A cursor at the top: one insert, properties then body.
    expect(insert("|Rest", "---\nk: v\n---\nBody ")).toBe("---\nk: v\n---\nBody |Rest");
    // A template that is only properties, without a line break at the end.
    expect(insert("Text|", "---\nk: v\n---")).toBe("---\nk: v\n---\nText|");
    // After a BOM, which stays first.
    expect(insert("﻿Text|", "---\nk: v\n---\n!")).toBe("﻿---\nk: v\n---\nText!|");
  });

  it("adds only the keys a note lacks, at the end of its properties", () => {
    const note = "---\ntitle: Mine\n# kept\n---\nBody |\n";
    const tpl = "---\ntitle: Theirs\ntags:\n  - meeting\nstatus: open\n---\n## Notes\n";
    expect(insert(note, tpl)).toBe("---\ntitle: Mine\n# kept\ntags:\n  - meeting\nstatus: open\n---\nBody ## Notes\n|\n");
  });

  it("leaves the note's properties alone when it has every key", () => {
    expect(insert("---\nk: 1\n---\nA|", "---\nk: 2\n---\nB")).toBe("---\nk: 1\n---\nAB|");
  });

  it("fills an empty block, and keeps Windows line breaks", () => {
    expect(insert("---\n---\nA|", "---\nk: v\n---\nB")).toBe("---\nk: v\n---\nAB|");
    expect(insert("---\r\na: 1\r\n---\r\nA|", "---\nb: 2\nc: 3\n---\nB")).toBe("---\r\na: 1\r\nb: 2\r\nc: 3\r\n---\r\nAB|");
  });

  it("puts the body below the properties when the cursor is in them", () => {
    expect(insert("---\nk|: 1\n---\nA", "---\nn: 2\n---\nB")).toBe("---\nk: 1\nn: 2\n---\nB|A");
    expect(insert("|Rest", "---\n---\nBody")).toBe("Body|Rest");
  });

  it("replaces the selection", () => {
    const doc = "---\na: 1\n---\nold text";
    const from = doc.indexOf("old");
    expect(apply(doc, templateEdit(doc, from, doc.length, "---\nb: 2\n---\nnew"))).toBe("---\na: 1\nb: 2\n---\nnew|");
  });

  it("inserts the template as it is when either block is not a mapping", () => {
    expect(insert("A|", "---\n- a\n---\nB")).toBe("A---\n- a\n---\nB|");
    expect(insert("---\n- x\n---\nA|", "---\nk: v\n---\nB")).toBe("---\n- x\n---\nA---\nk: v\n---\nB|");
  });
});
