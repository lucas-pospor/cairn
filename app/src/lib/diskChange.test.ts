// diskChange(): how a note reloaded from disk is applied to its editor state
// (FINDING-037: not an undo step; FINDING-100: the cursor stays at its text).
import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import { history, undo, redo } from "@codemirror/commands";
import { diskChange, fromDisk } from "./editor/setup";

function stateWith(doc: string, cursor: number) {
  return EditorState.create({ doc, selection: { anchor: cursor }, extensions: [history()] });
}

/** Type `text` at the cursor as a user edit (an undo step). */
function type(st: EditorState, text: string) {
  return st.update(st.replaceSelection(text), { userEvent: "input.type" }).state;
}

function runCmd(st: EditorState, cmd: typeof undo) {
  let out = st;
  cmd({ state: st, dispatch: (tr) => (out = tr.state) });
  return out;
}

describe("diskChange", () => {
  it("replaces only the part that differs", () => {
    const st = stateWith("first line\nsecond line\nthird line\n", 0);
    const tr = st.update(diskChange(st, "first line\nsecond LINE\nthird line\n"));
    const parts: [number, number, string][] = [];
    tr.changes.iterChanges((fromA, toA, _fromB, _toB, ins) => parts.push([fromA, toA, ins.toString()]));
    expect(parts).toEqual([[18, 22, "LINE"]]);
    expect(tr.annotation(fromDisk)).toBe(true);
  });

  it("keeps the cursor at the same text when lines are added above it", () => {
    const doc = "first line\nsecond line\nthird line\n";
    const st = stateWith(doc, doc.length - 1); // end of "third line"
    const after = st.update(diskChange(st, "NEW TOP LINE\n" + doc)).state;
    expect(type(after, "!").doc.toString()).toBe("NEW TOP LINE\nfirst line\nsecond line\nthird line!\n");
  });

  it("is not an undo step: Ctrl+Z undoes the user's own edit and keeps the external change", () => {
    let st = type(stateWith("line one\n", 8), " mine");
    st = st.update(diskChange(st, "line one mine\nEXTERNAL LINE\n")).state;
    st = runCmd(st, undo);
    expect(st.doc.toString()).toBe("line one\nEXTERNAL LINE\n");
    st = runCmd(st, redo);
    expect(st.doc.toString()).toBe("line one mine\nEXTERNAL LINE\n");
  });

  it("a user edit the external change replaced is gone from the history", () => {
    let st = type(stateWith("a b c\n", 3), "X");
    st = st.update(diskChange(st, "a Y c\n")).state;
    expect(runCmd(st, undo).doc.toString()).toBe("a Y c\n");
  });

  it("undoable (Load disk version) is one undo step of its own that brings the discarded edit back", () => {
    let st = type(stateWith("truncate me\n", 12), "MINE");
    st = st.update(diskChange(st, "", true)).state;
    expect(st.doc.toString()).toBe("");
    st = runCmd(st, undo);
    expect(st.doc.toString()).toBe("truncate me\nMINE");
  });

  it("does not split a surrogate pair", () => {
    const st = stateWith("x 😀 y\n", 0);
    const tr = st.update(diskChange(st, "x 😁 y\n"));
    const parts: string[] = [];
    tr.changes.iterChanges((fromA, toA, _fromB, _toB, ins) => parts.push(st.sliceDoc(fromA, toA) + "→" + ins.toString()));
    expect(parts).toEqual(["😀→😁"]);
  });

  it("handles growing, shrinking, emptying and CRLF text", () => {
    for (const [from, to] of [
      ["", "abc\n"],
      ["abc\n", ""],
      ["aaa", "aaaa"],
      ["aaaa", "aa"],
      ["a\nb\n", "a\r\nb\r\nc\r\n"],
    ]) {
      const st = EditorState.create({ doc: from, selection: EditorSelection.single(from.length) });
      expect(st.update(diskChange(st, to)).state.doc.toString()).toBe(to.replace(/\r\n/g, "\n"));
    }
  });

  // FINDING-074: another device's edit merged into the note while the user types.
  it("changes above and below the cursor are separate changes: the cursor and the user's undo step stay", () => {
    const doc = "one\ntwo\nthree\nfour\nfive\n";
    let st = type(stateWith(doc, 13), " mine"); // end of "three"
    const tr = st.update(diskChange(st, "ONE\ntwo\nthree mine\nfour\nFIVE\n"));
    const parts: [number, number, string][] = [];
    tr.changes.iterChanges((fromA, toA, _fromB, _toB, ins) => parts.push([fromA, toA, ins.toString()]));
    expect(parts).toEqual([
      [0, 3, "ONE"],
      [24, 28, "FIVE"],
    ]);
    st = tr.state;
    expect(type(st, "!").doc.toString()).toBe("ONE\ntwo\nthree mine!\nfour\nFIVE\n");
    expect(runCmd(st, undo).doc.toString()).toBe("ONE\ntwo\nthree\nfour\nFIVE\n");
  });

  it("gives the disk text for lines added, removed and changed anywhere, and for many changes", () => {
    let seed = 7;
    const rnd = (n: number) => ((seed = (seed * 1103515245 + 12345) % 2147483648), seed % n);
    const words = ["a", "b", "c", "", "dd", "😀", "a b"];
    const text = (n: number) => Array.from({ length: n }, () => words[rnd(words.length)]).join("\n") + (rnd(2) ? "\n" : "");
    const cases: [string, string][] = [];
    for (let i = 0; i < 300; i++) cases.push([text(rnd(12)), text(rnd(12))]);
    const many = Array.from({ length: 500 }, (_, i) => `line ${i}`);
    cases.push([many.join("\n"), many.map((l, i) => (i % 2 ? l : l.toUpperCase())).join("\n")]);
    for (const [from, to] of cases) {
      const st = stateWith(from, 0);
      expect(st.update(diskChange(st, to)).state.doc.toString()).toBe(to);
    }
  });
});
