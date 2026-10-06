// Live Preview block decorations (tables, embeds, the properties box) are
// updated per edit, not rebuilt for the whole note (FINDING-043,
// FINDING-195). These tests check that the updated decorations always equal
// a full rebuild once the parse is complete, also when the editor's parse
// ran out of time on the way, and that an edit renders only the tables it
// touches.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EditorSelection, EditorState, type TransactionSpec } from "@codemirror/state";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { yamlFrontmatter } from "@codemirror/lang-yaml";
import { ensureSyntaxTree, syntaxTree, syntaxTreeAvailable } from "@codemirror/language";
import { _internal as lp, refreshEmbeds } from "./editor/livePreview";
import { LinkIndex } from "./links";

// No DOM here: count the table renders instead of sanitizing.
const sanitize = vi.hoisted(() => vi.fn((html: string) => html));
vi.mock("dompurify", () => ({ default: { sanitize } }));

const links = new LinkIndex(["Inner.md", "pic.png", "Note.md"].map((path) => ({ path, kind: "file" as const, size: 0, mtime: 0 })));
const hooks = {
  linkIndex: () => links,
  notePath: () => "Note.md",
  openLink: () => {},
  openUrl: () => {},
  openImage: () => {},
};

// The editor parses for a set time (20 ms) per update and leaves the rest to
// a background parse. A fake clock makes that deterministic: it stands still
// (the parse always gets as far as it is asked to), or after a set number of
// reads it moves on by a second per read (the parse runs out of time there).
let reads = 0;
let cutAfter = Infinity;
beforeEach(() => {
  vi.spyOn(Date, "now").mockImplementation(() => (++reads > cutAfter ? reads * 1000 : 0));
});
afterEach(() => {
  vi.restoreAllMocks();
});

/** Run `f` with a parse that runs out of time after `n` clock reads. */
function outOfTime<T>(n: number, f: () => T): T {
  reads = 0;
  cutAfter = n;
  try {
    return f();
  } finally {
    cutAfter = Infinity;
  }
}

/** Finish the parse, as the editor's background parse does. */
function finishParse(state: EditorState): EditorState {
  ensureSyntaxTree(state, state.doc.length, 1e9);
  return state.update({}).state;
}

/**
 * What a parse that ran out of time left: "outer" when the Markdown parse
 * had not started (only the frontmatter wrapper is there), "markdown" when it
 * stopped part way, "" when it is complete.
 */
function cutKind(state: EditorState): string {
  if (syntaxTreeAvailable(state, state.doc.length)) return "";
  const last = syntaxTree(state).topNode.lastChild;
  return last?.name === "Document" ? "markdown" : "outer";
}

function setup(doc: string, anchor = doc.length) {
  const field = lp.blockField(hooks);
  const state = EditorState.create({
    doc,
    selection: { anchor },
    extensions: [
      EditorState.allowMultipleSelections.of(true),
      yamlFrontmatter({ content: markdown({ base: markdownLanguage }) }),
      lp.embedVersion,
      field,
    ],
  });
  return { field, state };
}

/** The field's decorations equal a full rebuild of the same state (which renders every table). */
function expectFull(state: EditorState, field: ReturnType<typeof lp.blockField>, what: string) {
  const mine = state.field(field).decos;
  const full = lp.buildBlocks(state, hooks);
  const dump = (set: typeof mine) => {
    const out: string[] = [];
    set.between(0, state.doc.length, (from, to, d) => {
      out.push(`${from}-${to} ${(d.spec.widget as { constructor: { name: string } }).constructor.name}`);
    });
    return out.join(", ");
  };
  if (!lp.RangeSet.eq([mine], [full], 0, state.doc.length)) {
    expect(`${what}\n${dump(mine)}`).toBe(`${what}\n${dump(full)}`);
    throw new Error(`${what}: same ranges, different widgets`);
  }
}

/** No widget covers a line with a cursor, also before the parse is done. */
function expectNoneOnCursor(state: EditorState, field: ReturnType<typeof lp.blockField>, what: string) {
  for (const r of state.selection.ranges) {
    const a = state.doc.lineAt(r.from).from;
    const b = state.doc.lineAt(r.to).to;
    state.field(field).decos.between(a, b, (from, to) => {
      throw new Error(`${what}: a widget at ${from}-${to} covers the cursor line(s) ${a}-${b}`);
    });
  }
}

const TABLE = "| a | b |\n|---|---|\n| 1 | 2 |";

// A fence that starts on the first screen (about 3000 characters) and ends
// after it, with table rows and embeds in it.
const FENCE_NOTE =
  Array.from({ length: 50 }, (_, i) => `Paragraph ${i} with words.`).join("\n\n") +
  "\n\n```\n" +
  Array.from({ length: 100 }, () => TABLE + "\n![[Inner]]").join("\n") +
  "\n```\n\n" +
  Array.from({ length: 40 }, (_, i) => `Tail ${i}.\n\n${TABLE}`).join("\n\n") +
  "\n";

/** The Markdown blocks that start in from..to, as "Name from-to". */
function blocksIn(state: EditorState, from: number, to: number): string[] {
  const out: string[] = [];
  for (let b = syntaxTree(state).topNode.lastChild!.firstChild; b; b = b.nextSibling) {
    if (b.from >= from && b.from < to) out.push(`${b.name} ${b.from}-${b.to}`);
  }
  return out;
}

describe("Live Preview block decorations", () => {
  it("typing in a paragraph renders no table again", () => {
    const doc = "# Tables\n\n" + Array.from({ length: 300 }, (_, i) => `Paragraph ${i}.\n\n${TABLE}\n`).join("\n");
    sanitize.mockClear();
    let { field, state } = setup(doc, 5);
    // The first parse stops early (the tables it covers are rendered); the
    // rest comes later, as the editor's background parse does it.
    expect(sanitize.mock.calls.length).toBeGreaterThan(50);
    expect(sanitize.mock.calls.length).toBeLessThan(300);
    ensureSyntaxTree(state, state.doc.length, 10000);
    state = state.update({}).state;
    expect(sanitize).toHaveBeenCalledTimes(300);
    expectFull(state, field, "after the parse finished");
    sanitize.mockClear();
    for (let k = 0; k < 10; k++) {
      state = state.update({ changes: { from: 5 + k, insert: "x" }, selection: { anchor: 6 + k } }).state;
    }
    expect(sanitize).not.toHaveBeenCalled();
    expectFull(state, field, "after typing at the top");
    sanitize.mockClear();
    // Typing in one table (the cursor is in it, so it shows as text), then
    // leaving it renders that table only.
    const at = state.doc.line(state.doc.lines - 3).from + 2;
    state = state.update({ selection: { anchor: at } }).state;
    state = state.update({ changes: { from: at, insert: "y" }, selection: { anchor: at + 1 } }).state;
    expect(sanitize).not.toHaveBeenCalled();
    state = state.update({ selection: { anchor: 0 } }).state;
    expect(sanitize).toHaveBeenCalledTimes(1);
    expectFull(state, field, "after leaving the table");
  });

  it("follows changes to the block structure away from the edit", () => {
    const doc = "intro\n\n" + TABLE + "\n\n![[Inner]]\n\n> " + TABLE.replace(/\n/g, "\n> ") + "\n\n- item\n\n  " + TABLE.replace(/\n/g, "\n  ") + "\n\nend\n";
    let { field, state } = setup(doc, doc.length);
    const steps: [string, TransactionSpec][] = [
      // An open fence turns everything below into code.
      ["open a fence", { changes: { from: 0, insert: "```\n" } }],
      ["close it", { changes: { from: 4, insert: "```\n" } }],
      ["remove both", { changes: { from: 0, to: 8 } }],
      // A delimiter row under a line makes it a table header.
      ["make a table", { changes: { from: 0, insert: "| x | y |\n|---|---|\n" } }],
      ["break it", { changes: { from: 10, to: 11, insert: "a" } }],
      // Frontmatter that closes further down.
      ["frontmatter opens", { changes: { from: 0, insert: "---\ntitle: x\n" } }],
      ["frontmatter closes", { changes: { from: 13, insert: "---\n" } }],
      ["cursor into it", { selection: { anchor: 5 } }],
      ["cursor out", { selection: { anchor: doc.length } }],
      ["frontmatter opens no more", { changes: { from: 0, to: 1 } }],
      // Setext underline and a blank line that splits a table off.
      ["setext", { changes: { from: 0, insert: "Title\n===\n" } }],
      ["join lines", { changes: { from: 5, to: 6 } }],
    ];
    for (const [what, spec] of steps) {
      state = state.update(spec).state;
      expectFull(state, field, what);
    }
  });

  it("drops the widget of a block that is deleted whole", () => {
    const doc = "intro\n\n![[Inner]]\n\n" + TABLE + "\n\nend\n";
    let { field, state } = setup(doc, 0);
    const embed = doc.indexOf("![[Inner]]");
    state = state.update({ changes: { from: embed, to: embed + "![[Inner]]".length } }).state;
    expectFull(state, field, "embed line deleted");
    const table = state.doc.toString().indexOf("| a |");
    state = state.update({ changes: { from: table, to: table + TABLE.length } }).state;
    expectFull(state, field, "table deleted");
  });

  it("fills embeds again after a change in the vault", () => {
    let { field, state } = setup("![[Inner]]\n\ntext\n");
    const before = state.field(field).decos;
    state = state.update({ effects: refreshEmbeds.of(null) }).state;
    expect(state.field(field).decos).not.toBe(before);
    expectFull(state, field, "after refreshEmbeds");
  });

  it("matches a full rebuild after random edits", () => {
    let seed = 12345;
    const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
    const pick = <T>(a: readonly T[]) => a[Math.floor(rand() * a.length)];
    const LINES = [
      "para text with words", "", "", "| a | b |", "|---|---|", "| 1 | 2 |", "row without pipes", "```", "```js", "~~~", "> quote",
      "> | q | r |", "> |---|---|", "- item", "  | l | m |", "  |---|---|", "1. one", "![[Inner]]", "![[pic.png]]",
      "![alt](pic.png)", "  ![[Inner]]", "---", "title: x", "===", "# Head", "    indented code", "<div>", "</div>",
    ];
    const BITS = ["|", "-", "`", ">", "!", "[[", "]]", "\n", " ", "#", "=", "x", "---\n", "| c |\n", "```\n", "\n\n", "![[Inner]]\n", "  "];
    for (let round = 0; round < 40; round++) {
      // Some notes start with frontmatter; some are too long for the first
      // parse, which then goes on in steps (as the editor's background parse).
      const big = round % 5 === 4;
      const lines = Array.from({ length: big ? 600 : 10 + Math.floor(rand() * 40) }, () => pick(LINES));
      if (round % 3 === 0) lines.unshift("---", "title: x", "tags: [a]", "---");
      const doc = lines.join("\n");
      const anchor = Math.floor(rand() * doc.length);
      const cut = round % 4 === 1 ? Math.floor(rand() * 40) : Infinity;
      let { field, state } = outOfTime(cut, () => setup(doc, anchor));
      const check = (what: string) => {
        expectNoneOnCursor(state, field, what);
        if (syntaxTreeAvailable(state, state.doc.length)) expectFull(state, field, what);
      };
      check(`round ${round} start`);
      for (let step = 0; step < 40; step++) {
        const len = state.doc.length;
        const at = Math.floor(rand() * (len + 1));
        const r = rand();
        let spec: TransactionSpec;
        if (r < 0.45) spec = { changes: { from: at, insert: pick(BITS) }, selection: { anchor: at + 1 } };
        else if (r < 0.65) spec = { changes: { from: at, to: Math.min(len, at + 1 + Math.floor(rand() * 12)) } };
        else if (r < 0.75) spec = { changes: { from: at, insert: pick(LINES) + "\n" } };
        else if (r < 0.85) spec = { selection: { anchor: at } };
        else if (r < 0.92) {
          const b = Math.floor(rand() * (len + 1));
          spec = {
            selection: EditorSelection.create([EditorSelection.range(Math.min(at, b), Math.max(at, b)), EditorSelection.cursor(len)], 0),
          };
        } else {
          ensureSyntaxTree(state, Math.min(len, syntaxTree(state).length + 2000), 1000);
          spec = {};
        }
        // Now and then the parse runs out of time in this update.
        const cut = rand() < 0.2 ? Math.floor(rand() * 60) : Infinity;
        state = outOfTime(cut, () => state.update(spec).state);
        check(`round ${round} step ${step}: ${JSON.stringify(spec)} cut ${cut}\n${JSON.stringify(state.doc.toString())}`);
      }
      state = finishParse(state);
      check(`round ${round} end`);
    }
  });

  it("follows the parser when it keeps a placeholder for a fence", () => {
    // After a parse ran out of time, the next one stops at the first screen
    // and leaves a zero-length placeholder where it would have reused the
    // fence that reaches past that point. Later parses keep it and read the
    // fence's lines as a table (@lezer/markdown 1.7.2). The widgets follow
    // what the parser says, as a full rebuild would.
    const full = setup(FENCE_NOTE, 0);
    const field = full.field;
    let state = finishParse(full.state);
    expectFull(state, field, "start");
    state = outOfTime(1, () => state.update({ changes: { from: 10, insert: "x" } }).state);
    state = state.update({ changes: { from: 20, insert: "y" } }).state;
    const fence = state.doc.toString().indexOf("```");
    const md = syntaxTree(state).topNode.lastChild!;
    expect(md.childAfter(fence - 1)?.to, "a placeholder where the fence starts").toBe(fence);
    state = finishParse(state);
    expectFull(state, field, "parse finished");
    state = finishParse(state.update({ changes: { from: 1200, insert: "\n\n" } }).state);
    expectFull(state, field, "after an edit above it");
    state = finishParse(state.update({ changes: { from: fence - 2, to: fence + 6 } }).state);
    expectFull(state, field, "after an edit at the fence");
  });

  // FINDING-225: @lezer/markdown keeps a placeholder for a fence that runs
  // past the end of the range it parses (in FragmentCursor.takeNodes, 1.7.2),
  // and later parses read the fence's lines as a table and the text after the
  // closing fence as code. The test above checks that Live Preview follows
  // the parser; this one checks what the note should look like, so it fails
  // until the parser is fixed upstream. With a fixed parser it passes (make
  // it a plain test then) and the test above finds no placeholder.
  it.fails("keeps a fence as code after a cut parse and one more edit", () => {
    const { field, state: opened } = setup(FENCE_NOTE, 0);
    let state = finishParse(opened);
    state = outOfTime(1, () => state.update({ changes: { from: 10, insert: "x" } }).state);
    state = state.update({ changes: { from: 20, insert: "y" } }).state;
    const text = state.doc.toString();
    const fence = text.indexOf("```");
    const end = text.indexOf("```", fence + 3) + 3;
    // Right after the edits: the block at the fence is the fence, not an
    // empty placeholder.
    const at = syntaxTree(state).topNode.lastChild!.childAfter(fence - 1);
    expect(`${at?.name} at ${at?.from}`, "the block at the fence after the edits").toBe(`FencedCode at ${fence}`);
    // Once the parse is done: one code block from fence to fence, and no
    // Live Preview widget on its lines.
    state = finishParse(state);
    expect(blocksIn(state, fence, end), "the blocks in the fence").toEqual([`FencedCode ${fence}-${end}`]);
    const widgets: string[] = [];
    state.field(field).decos.between(fence, end, (from, to, d) => {
      widgets.push(`${from}-${to} ${(d.spec.widget as { constructor: { name: string } }).constructor.name}`);
    });
    expect(widgets, "Live Preview widgets in the fence").toEqual([]);
  });

  it("loses no table when the parse runs out of time", { timeout: 60000 }, () => {
    const doc =
      "---\ntitle: x\n---\n" +
      Array.from({ length: 200 }, (_, i) => `Paragraph ${i}.\n\n${TABLE}\n\n![[Inner]]\n`).join("\n") +
      "\n```\n![[Inner]]\n```\n";
    const kinds = new Set<string>();
    // When the note opens (the first parse goes as far as the first screen,
    // about 3000 characters, in some 600 clock reads).
    for (let n = 0; n < 650; n += 9) {
      let { field, state } = outOfTime(n, () => setup(doc, 0));
      kinds.add(`open ${cutKind(state)}`);
      state = finishParse(state);
      expectFull(state, field, `open, out of time after ${n} reads`);
    }
    // When typing, in the frontmatter, the middle and at the end, and after
    // more typing before the parse is done (a parse after an edit takes some
    // 150 clock reads).
    const full = setup(doc, 0);
    const done = finishParse(full.state);
    let renders = 0;
    for (const at of [10, Math.floor(doc.length / 2), doc.length - 20]) {
      for (let n = 0; n < 200; n += 4) {
        sanitize.mockClear();
        let state = outOfTime(n, () => done.update({ changes: { from: at, insert: "x\n" }, selection: { anchor: at + 2 } }).state);
        kinds.add(`edit ${cutKind(state)}`);
        state = outOfTime(n, () => state.update({ changes: { from: at + 2, insert: "| q |\n|---|\n" } }).state);
        state = outOfTime(n >> 1, () => state.update({ selection: { anchor: 0 } }).state);
        expectNoneOnCursor(state, full.field, `edit at ${at}, out of time after ${n} reads`);
        state = finishParse(state);
        renders = Math.max(renders, sanitize.mock.calls.length);
        expectFull(state, full.field, `edit at ${at}, out of time after ${n} reads`);
        // A fence that turns the rest of the note into code (up to the
        // fence at its end); the next parse then only goes as far as the
        // first screen.
        state = outOfTime(n, () => done.update({ changes: { from: at, insert: "```\n" }, selection: { anchor: at + 4 } }).state);
        state = state.update({ changes: { from: at + 4, insert: "x" } }).state;
        state = outOfTime(n >> 1, () => state.update({ selection: { anchor: 0 } }).state);
        expectNoneOnCursor(state, full.field, `fence at ${at}, out of time after ${n} reads`);
        state = finishParse(state);
        expectFull(state, full.field, `fence at ${at}, out of time after ${n} reads`);
      }
    }
    // Both kinds of cut happened: before the Markdown parse and inside it.
    expect([...kinds].sort()).toEqual(["edit ", "edit markdown", "edit outer", "open markdown", "open outer"]);
    // Only the table made by the edit is rendered, not the 200 after it.
    expect(renders).toBeLessThanOrEqual(2);
  });
});
