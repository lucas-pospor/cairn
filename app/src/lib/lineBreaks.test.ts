// A note keeps its own line breaks when it is edited and saved (FINDING-045).
import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { lineBreaksOf, textWithLineBreaks } from "./editor/lineBreaks";
import { diskChange } from "./editor/setup";

/** A note's editor state as loadTab makes it. */
function load(text: string) {
  return EditorState.create({ doc: text, extensions: [lineBreaksOf(text)] });
}

/** Replace from..to with `insert`, as typing or pasting does. */
function edit(st: EditorState, from: number, to: number, insert: string) {
  return st.update({ changes: { from, to, insert } }).state;
}

describe("textWithLineBreaks", () => {
  it("gives back the loaded text unchanged", () => {
    for (const text of ["", "a\nb\n", "a\r\nb\r\n", "a\rb\r", "a\r\nb\nc\rd\r\n", "a\rb\nc\n", "\r\n\r\n", "no break"]) {
      expect(textWithLineBreaks(load(text))).toBe(text);
    }
  });

  it("gives new lines the note's usual line break", () => {
    let st = load("one\r\ntwo\r\n");
    st = edit(st, 3, 3, "\nnew"); // Enter at the end of "one", then text
    st = edit(st, st.doc.length, st.doc.length, "x\r\ny\nz"); // pasted text
    expect(textWithLineBreaks(st)).toBe("one\r\nnew\r\ntwo\r\nx\r\ny\r\nz");
    expect(textWithLineBreaks(edit(load("a\rb"), 1, 1, "\n"))).toBe("a\r\rb");
    expect(textWithLineBreaks(edit(load("a\nb"), 1, 1, "\r\n"))).toBe("a\n\nb");
  });

  it("keeps an odd line break at the end of its line in a mixed note", () => {
    // Usual break CRLF; the line "b" ends in LF.
    let st = load("a\r\nb\nc\r\n");
    st = edit(st, 3, 3, " typed"); // at the end of "b"
    expect(textWithLineBreaks(st)).toBe("a\r\nb typed\nc\r\n");
    st = edit(st, 2, 9, "B"); // replace the text of that line
    expect(textWithLineBreaks(st)).toBe("a\r\nB\nc\r\n");
    st = edit(st, 3, 3, "\nnew"); // Enter at its end: the odd break ends the new line
    expect(textWithLineBreaks(st)).toBe("a\r\nB\r\nnew\nc\r\n");
  });

  it("drops an odd line break that is deleted", () => {
    let st = load("a\r\nb\nc\r\n");
    st = edit(st, 3, 4, ""); // join "b" and "c"
    expect(textWithLineBreaks(st)).toBe("a\r\nbc\r\n");
    st = edit(st, 3, 3, "\n"); // split them again: a new line gets the usual break
    expect(textWithLineBreaks(st)).toBe("a\r\nb\r\nc\r\n");
  });

  it("follows the line breaks of a note reloaded from disk", () => {
    let st = load("a\r\nb\nc\r\n");
    st = st.update(diskChange(st, "a\nb\nc\nd\n")).state;
    expect(textWithLineBreaks(st)).toBe("a\nb\nc\nd\n");
    st = st.update(diskChange(st, "a\r\nb\rc\r\n")).state;
    expect(textWithLineBreaks(st)).toBe("a\r\nb\rc\r\n");
    st = edit(st, st.doc.length, st.doc.length, "d\n");
    expect(textWithLineBreaks(st)).toBe("a\r\nb\rc\r\nd\r\n");
  });

  it("joins with LF in a state made without the note's line breaks", () => {
    expect(textWithLineBreaks(EditorState.create({ doc: "a\r\nb" }))).toBe("a\nb");
  });
});
