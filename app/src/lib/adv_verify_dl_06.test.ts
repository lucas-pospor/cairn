// Reproduction for FINDING-045. An editor state built with
//   EditorState.create({ doc: n.content, selection, extensions: this.extensions })
// and no EditorState.lineSeparator splits the text on \r\n, \r and \n and
// joins it with \n, so doc.toString() (what save() would write) has lost every
// CR before the user types anything. loadTab in app.svelte.ts adds
// lineBreaksOf(n.content) and docOf() is textWithLineBreaks().
import { describe, it, expect } from "vitest";
import { EditorState } from "@codemirror/state";
import { lineBreaksOf, textWithLineBreaks } from "./editor/lineBreaks";

// CodeMirror alone: no lineBreaksOf, doc.toString() + an edit at the end.
function loadEditSave(content: string, insert = "X", extensions: any[] = []): string {
  const st = EditorState.create({ doc: content, selection: { anchor: 0 }, extensions });
  const after = st.update({ changes: { from: st.doc.length, insert } }).state;
  return after.doc.toString();
}

// Same shape as loadTab + docOf + an edit at the end.
function cairnLoadEditSave(content: string, insert = "X"): string {
  const st = EditorState.create({ doc: content, selection: { anchor: 0 }, extensions: [lineBreaksOf(content)] });
  const after = st.update({ changes: { from: st.doc.length, insert } }).state;
  return textWithLineBreaks(after);
}

describe("FINDING-045: CodeMirror state without lineSeparator", () => {
  it("the loaded document already has no CR in it (before any edit)", () => {
    const st = EditorState.create({ doc: "a\r\nb\r\n" });
    expect(st.doc.toString()).toBe("a\nb\n");
    expect(st.doc.lines).toBe(3);
  });

  it("FINDING-045: CRLF lines the user did not touch keep CRLF after an edit", () => {
    expect(cairnLoadEditSave("line one\r\nline two\r\nline three\r\n")).toBe("line one\r\nline two\r\nline three\r\nX");
  });

  it("FINDING-045: lone-CR (classic Mac) line endings survive an edit", () => {
    expect(cairnLoadEditSave("one\rtwo\r")).toBe("one\rtwo\rX");
  });

  it("FINDING-045: mixed line endings survive an edit", () => {
    expect(cairnLoadEditSave("a\r\nb\nc\r")).toBe("a\r\nb\nc\rX");
  });

  it("CodeMirror alone (no lineSeparator): every break becomes LF", () => {
    expect(loadEditSave("line one\r\nline two\r\nline three\r\n")).toBe("line one\nline two\nline three\nX");
    expect(loadEditSave("one\rtwo\r")).toBe("one\ntwo\nX");
    expect(loadEditSave("a\r\nb\nc\r")).toBe("a\nb\nc\nX");
  });

  it("setting lineSeparator alone is NOT enough: doc.toString() (docOf) still joins with LF", () => {
    const sep = EditorState.lineSeparator.of("\r\n");
    // Text.toString() is sliceString(0) with the default "\n" join.
    expect(loadEditSave("line one\r\nline two\r\nline three\r\n", "X", [sep])).toBe("line one\nline two\nline three\nX");
  });

  it("a working fix: lineSeparator facet AND docOf via state.sliceDoc() (joins with state.lineBreak)", () => {
    const sep = EditorState.lineSeparator.of("\r\n");
    const st = EditorState.create({ doc: "line one\r\nline two\r\nline three\r\n", extensions: [sep] });
    const after = st.update({ changes: { from: st.doc.length, insert: "X" } }).state;
    expect(after.sliceDoc()).toBe("line one\r\nline two\r\nline three\r\nX");
    // A new line typed by the user gets the file's separator too.
    expect(after.lineBreak).toBe("\r\n");
  });
});
