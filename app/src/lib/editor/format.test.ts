// Formatting commands (hotkeys and the mobile toolbar) on plain editor state.
//
// Run: cd app && npx vitest run src/lib/editor/format.test.ts
import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState, Transaction, type TransactionSpec } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { cycleHeading, toggleTask, toggleWrap } from "./format";

/** Just enough of an EditorView for the commands: state and dispatch. */
function run(doc: string, from: number, to: number, cmd: (v: EditorView) => boolean): { doc: string; sel: string } {
  let state = EditorState.create({ doc, selection: EditorSelection.range(from, to), extensions: [EditorState.allowMultipleSelections.of(true)] });
  const view = {
    get state() {
      return state;
    },
    dispatch(tr: Transaction | TransactionSpec) {
      state = (tr instanceof Transaction ? tr : state.update(tr)).state;
    },
  } as unknown as EditorView;
  cmd(view);
  const r = state.selection.main;
  return { doc: state.doc.toString(), sel: state.sliceDoc(r.from, r.to) };
}

const italic = (v: EditorView) => toggleWrap(v, "*");
const bold = (v: EditorView) => toggleWrap(v, "**");

describe("toggleWrap", () => {
  it("wraps and unwraps plain text", () => {
    expect(run("a word b", 2, 6, italic)).toEqual({ doc: "a *word* b", sel: "word" });
    expect(run("a *word* b", 3, 7, italic)).toEqual({ doc: "a word b", sel: "word" });
    expect(run("a word b", 2, 6, bold)).toEqual({ doc: "a **word** b", sel: "word" });
    expect(run("a **word** b", 4, 8, bold)).toEqual({ doc: "a word b", sel: "word" });
    expect(run("a *word* b", 2, 8, italic)).toEqual({ doc: "a word b", sel: "word" });
    // Pressing it twice with no selection leaves nothing behind.
    expect(run("a  b", 2, 2, italic).doc).toBe("a ** b");
    expect(run("a ** b", 3, 3, italic).doc).toBe("a  b");
  });

  // FINDING-095 (Ctrl+I) and FINDING-079 (toolbar Italic).
  it("italic on a bold word makes it bold italic", () => {
    expect(run("a **bold** b", 4, 8, italic)).toEqual({ doc: "a ***bold*** b", sel: "bold" });
    expect(run("**word**", 2, 6, italic)).toEqual({ doc: "***word***", sel: "word" });
    // The same with the markers selected too.
    expect(run("a **bold** b", 2, 10, italic).doc).toBe("a ***bold*** b");
    expect(run("a **bold** b", 3, 9, italic).doc).toBe("a ***bold*** b");
  });

  it("italic on a bold italic word leaves it bold", () => {
    expect(run("a ***bold*** b", 5, 9, italic)).toEqual({ doc: "a **bold** b", sel: "bold" });
    expect(run("a ***bold*** b", 2, 12, italic).doc).toBe("a **bold** b");
  });

  it("bold on an italic or bold italic word adds or removes only the bold", () => {
    expect(run("*word*", 1, 5, bold)).toEqual({ doc: "***word***", sel: "word" });
    expect(run("a ***bold*** b", 5, 9, bold)).toEqual({ doc: "a *bold* b", sel: "bold" });
  });
});

describe("toggleTask", () => {
  it("adds, checks and unchecks a checkbox", () => {
    expect(run("buy milk", 3, 3, toggleTask).doc).toBe("- [ ] buy milk");
    expect(run("  buy milk", 3, 3, toggleTask).doc).toBe("  - [ ] buy milk");
    expect(run("- buy milk", 3, 3, toggleTask).doc).toBe("- [ ] buy milk");
    expect(run("1. buy milk", 3, 3, toggleTask).doc).toBe("1. [ ] buy milk");
    expect(run("- [ ] buy milk", 3, 3, toggleTask).doc).toBe("- [x] buy milk");
    expect(run("- [x] buy milk", 3, 3, toggleTask).doc).toBe("- [ ] buy milk");
    expect(run("  - sub", 5, 5, toggleTask).doc).toBe("  - [ ] sub");
    expect(run("  - [ ] sub", 5, 5, toggleTask).doc).toBe("  - [x] sub");
    expect(run("", 0, 0, toggleTask).doc).toBe("- [ ] ");
  });

  // FINDING-196
  it("puts the checkbox inside a quote", () => {
    expect(run("> quoted line", 3, 3, toggleTask).doc).toBe("> - [ ] quoted line");
    expect(run("> > nested", 5, 5, toggleTask).doc).toBe("> > - [ ] nested");
    expect(run(">tight", 3, 3, toggleTask).doc).toBe("> - [ ] tight");
    expect(run("> - item", 5, 5, toggleTask).doc).toBe("> - [ ] item");
    expect(run("> - [ ] item", 5, 5, toggleTask).doc).toBe("> - [x] item");
    expect(run("> - [x] item", 5, 5, toggleTask).doc).toBe("> - [ ] item");
  });

  it("leaves heading lines alone", () => {
    expect(run("# Heading", 3, 3, toggleTask).doc).toBe("# Heading");
    expect(run("> ## Quoted heading", 5, 5, toggleTask).doc).toBe("> ## Quoted heading");
    // A tag is not a heading.
    expect(run("#todo call", 3, 3, toggleTask).doc).toBe("- [ ] #todo call");
    const src = "> quoted line\n# Heading\nplain\n";
    expect(run(src, 0, src.length - 1, toggleTask).doc).toBe("> - [ ] quoted line\n# Heading\n- [ ] plain\n");
  });
});

describe("cycleHeading", () => {
  const head = (doc: string) => run(doc, doc.length, doc.length, cycleHeading).doc;

  it("cycles plain, H1, H2, H3, plain", () => {
    expect(head("plain")).toBe("# plain");
    expect(head("# plain")).toBe("## plain");
    expect(head("## plain")).toBe("### plain");
    expect(head("### plain")).toBe("plain");
  });

  // FINDING-173
  it("keeps an H4 to H6 line a heading", () => {
    expect(head("#### Deep")).toBe("# Deep");
    expect(head("##### Deep")).toBe("# Deep");
    expect(head("###### Deep")).toBe("# Deep");
  });
});
