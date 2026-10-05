// A note's line breaks as they are on disk. CodeMirror splits a document on
// \r\n, \r and \n alike and keeps only the lines, so its text always comes
// back joined with \n. To write a note back byte for byte (FINDING-045), its
// editor state remembers the note's usual line break, used for new lines too,
// and which lines end in a different one.

import {
  MapMode,
  RangeSet,
  RangeValue,
  StateEffect,
  StateField,
  type EditorState,
  type Extension,
  type Range,
} from "@codemirror/state";

/** Marks the end of a line whose break is not the note's usual one. */
class OddBreak extends RangeValue {
  // Stays at the end of its line when text is typed there, and goes away
  // with the line break it stands for.
  startSide = 1;
  endSide = 1;
  mapMode = MapMode.TrackAfter;
  constructor(readonly text: string) {
    super();
  }
  eq(other: RangeValue) {
    return other instanceof OddBreak && other.text === this.text;
  }
}

interface LineBreaks {
  /** The note's usual line break. */
  sep: string;
  odd: RangeSet<OddBreak>;
}

const LF: LineBreaks = { sep: "\n", odd: RangeSet.empty };

function scan(text: string): LineBreaks {
  if (!text.includes("\r")) return LF;
  const count: Record<string, number> = { "\n": 0, "\r\n": 0, "\r": 0 };
  for (const m of text.matchAll(/\r\n?|\n/g)) count[m[0]]++;
  const sep = ["\r\n", "\r"].reduce((a, b) => (count[b] > count[a] ? b : a), "\n");
  const odd: Range<OddBreak>[] = [];
  // A line break is one character in the document, whatever its length on disk.
  let shift = 0;
  for (const m of text.matchAll(/\r\n?|\n/g)) {
    if (m[0] !== sep) odd.push(new OddBreak(m[0]).range(m.index - shift));
    shift += m[0].length - 1;
  }
  return { sep, odd: RangeSet.of(odd) };
}

const setLineBreaks = StateEffect.define<LineBreaks>();

const lineBreaks = StateField.define<LineBreaks>({
  create: () => LF,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setLineBreaks)) return e.value;
    if (!tr.docChanged || !value.odd.size) return value;
    return { sep: value.sep, odd: value.odd.map(tr.changes) };
  },
});

/** Extension for the editor state of a note read from disk as `text`. */
export function lineBreaksOf(text: string): Extension {
  return lineBreaks.init(() => scan(text));
}

/** Effect for a transaction that turns the document into `text` from disk. */
export function resetLineBreaks(text: string): StateEffect<LineBreaks> {
  return setLineBreaks.of(scan(text));
}

/** The document's text with the note's line breaks, as it is saved. */
export function textWithLineBreaks(state: EditorState): string {
  const { sep, odd } = state.field(lineBreaks, false) ?? LF;
  const doc = state.doc;
  if (!odd.size) return doc.sliceString(0, doc.length, sep);
  const parts: string[] = [];
  let pos = 0;
  for (const c = odd.iter(); c.value; c.next()) {
    if (c.from < pos || c.from >= doc.length || doc.lineAt(c.from).to !== c.from) continue;
    parts.push(doc.sliceString(pos, c.from, sep), c.value.text);
    pos = c.from + 1;
  }
  parts.push(doc.sliceString(pos, doc.length, sep));
  return parts.join("");
}
