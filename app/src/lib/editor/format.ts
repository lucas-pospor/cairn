// Editing commands for hotkeys and the mobile toolbar (bold, italic, checklists, headings, links).

import { EditorSelection, type ChangeSpec, type EditorState } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

/** Number of "*" in a row from `pos` going left (dir -1) or right (dir 1), up to `stop`. */
function stars(state: EditorState, pos: number, dir: -1 | 1, stop: number): number {
  let n = 0;
  for (; pos !== stop; pos += dir) {
    const ch = dir < 0 ? state.sliceDoc(pos - 1, pos) : state.sliceDoc(pos, pos + 1);
    if (ch !== "*") break;
    n++;
  }
  return n;
}

/** Wrap each selection in `marker`, or unwrap if already wrapped. */
export function toggleWrap(view: EditorView, marker: string): boolean {
  const { state } = view;
  const m = marker.length;
  const tr = state.changeByRange((range) => {
    // A "*" next to the selection may be half of "**" (bold): the run of stars
    // on each edge, outside plus inside, holds an italic "*" only when it is odd.
    const mayUnwrap =
      marker !== "*" ||
      ((stars(state, range.from, -1, 0) + stars(state, range.from, 1, range.to)) % 2 === 1 &&
        (stars(state, range.to, -1, range.from) + stars(state, range.to, 1, state.doc.length)) % 2 === 1);
    const before = state.sliceDoc(range.from - m, range.from);
    const after = state.sliceDoc(range.to, range.to + m);
    if (mayUnwrap && before === marker && after === marker) {
      return {
        changes: [
          { from: range.from - m, to: range.from },
          { from: range.to, to: range.to + m },
        ],
        range: EditorSelection.range(range.anchor - m, range.head - m),
      };
    }
    const text = state.sliceDoc(range.from, range.to);
    if (mayUnwrap && text.length >= 2 * m && text.startsWith(marker) && text.endsWith(marker)) {
      return {
        changes: { from: range.from, to: range.to, insert: text.slice(m, -m) },
        range: EditorSelection.range(range.from, range.to - 2 * m),
      };
    }
    return {
      changes: [
        { from: range.from, insert: marker },
        { from: range.to, insert: marker },
      ],
      range: EditorSelection.range(range.from + m, range.to + m),
    };
  });
  view.dispatch(state.update(tr, { scrollIntoView: true, userEvent: "input.format" }));
  return true;
}

/** Cycle lines: text -> "- [ ] text" -> "- [x] text" -> "- [ ] text".
 *  In a quote the checkbox goes after the ">"; headings are left alone. */
export function toggleTask(view: EditorView): boolean {
  const { state } = view;
  const changes: ChangeSpec[] = [];
  const seen = new Set<number>();
  for (const r of state.selection.ranges) {
    for (let pos = r.from; pos <= r.to; ) {
      const line = state.doc.lineAt(pos);
      if (!seen.has(line.number)) {
        seen.add(line.number);
        // Indent and quote markers ("> > ") stay in front of the checkbox.
        const prefix = /^\s*(?:>\s*)*/.exec(line.text)![0];
        const rest = line.text.slice(prefix.length);
        const start = line.from + prefix.length;
        const task = /^((?:[-*+]|\d+[.)])\s+)\[( |x|X)\]/.exec(rest);
        const list = /^((?:[-*+]|\d+[.)])\s+)/.exec(rest);
        if (task) {
          const at = start + task[1].length + 1;
          changes.push({ from: at, to: at + 1, insert: task[2] === " " ? "x" : " " });
        } else if (list) {
          changes.push({ from: start + list[1].length, insert: "[ ] " });
        } else if (!/^#{1,6}(?:\s|$)/.test(rest)) {
          changes.push({ from: start, insert: prefix.endsWith(">") ? " - [ ] " : "- [ ] " });
        }
      }
      pos = line.to + 1;
    }
  }
  view.dispatch({ changes, userEvent: "input.format" });
  return true;
}

/** Toolbar heading button on the cursor line: plain -> H1 -> H2 -> H3 -> plain.
 *  H4 to H6 go to H1, so the line stays a heading. */
export function cycleHeading(view: EditorView): boolean {
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  const m = /^(#{1,6})\s/.exec(line.text);
  const level = m ? m[1].length : 0;
  const next = level === 3 ? "" : "#".repeat(level > 3 ? 1 : level + 1) + " ";
  view.dispatch({ changes: { from: line.from, to: line.from + (m ? m[0].length : 0), insert: next } });
  return true;
}

/** Insert `[[]]` (or wrap the selection) and leave the cursor inside. */
export function insertWikilink(view: EditorView): boolean {
  const tr = view.state.changeByRange((range) => {
    const text = view.state.sliceDoc(range.from, range.to);
    return {
      changes: { from: range.from, to: range.to, insert: `[[${text}]]` },
      range: EditorSelection.cursor(range.from + 2 + text.length),
    };
  });
  view.dispatch(view.state.update(tr, { userEvent: "input.format" }));
  return true;
}
