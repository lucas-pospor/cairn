import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { linkAtCursor } from "./setup";

const at = (doc: string, cursor: number) => linkAtCursor(EditorState.create({ doc, selection: { anchor: cursor } }));

describe("linkAtCursor (Follow link under cursor, FINDING-124)", () => {
  const doc = "Go to [[Ideas#Plan]] or [site](https://example.com) now.\n[[Next]]";

  it("finds the wikilink or URL the cursor is in", () => {
    expect(at(doc, doc.indexOf("Ideas") + 2)).toEqual({ kind: "wiki", target: "Ideas", subpath: "Plan" });
    expect(at(doc, doc.indexOf("site"))).toEqual({ kind: "url", url: "https://example.com" });
  });

  it("counts the cursor just after a link, as after typing its ]]", () => {
    expect(at(doc, doc.indexOf("]]") + 2)).toEqual({ kind: "wiki", target: "Ideas", subpath: "Plan" });
  });

  it("finds nothing away from links or on the next line", () => {
    expect(at(doc, 1)).toBeNull();
    expect(at(doc, doc.indexOf("[["))).toBeNull();
    // The start of a line does not look back at the end of the previous one.
    expect(at("[[A]]\nplain", "[[A]]\n".length)).toBeNull();
  });
});
