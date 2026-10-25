// Properties (YAML frontmatter) as text: where a note's block is, which
// top-level keys it has, and how a template's properties go into a note.
// Edits keep the text as written: the YAML is not parsed and written back,
// so comments, quotes and the order of keys stay.

import { FRONTMATTER_RE } from "./markdown";

/** A top-level key of a YAML mapping and its lines (the value, nested lines, comments after it). */
export interface PropertyBlock {
  key: string;
  text: string;
}

// A key at the start of a line: quoted, or plain text up to `: ` or a `:` at
// the end. A line starting with a YAML indicator (`-`, `[`, `{`, `?`, `&`,
// `*`, `!`, `|`, `>`, `%`, `@`, a backtick) or `#` is not a key.
const KEY_RE = /^(?:"((?:[^"\\]|\\.)*)"|'((?:[^']|'')*)'|([^\s#\-[\]{}?&*!|>%@`"',][^#]*?))[ \t]*:(?:[ \t]|$)/;

/**
 * The top-level keys of the YAML in a frontmatter block, each with its lines,
 * or null when the text does not read as a mapping (a list, or a value that
 * starts indented). Empty YAML gives no blocks. Blank and comment lines before
 * the first key go with it.
 */
export function propertyBlocks(yaml: string): PropertyBlock[] | null {
  const lines = yaml.split(/\r?\n/);
  const blocks: PropertyBlock[] = [];
  const lead: string[] = [];
  for (const line of lines) {
    const m = KEY_RE.exec(line);
    if (m) {
      const key = m[1] !== undefined ? m[1].replace(/\\(.)/g, "$1") : m[2] !== undefined ? m[2].replace(/''/g, "'") : m[3];
      blocks.push({ key, text: [...lead.splice(0), line].join("\n") });
    } else if (blocks.length) {
      blocks[blocks.length - 1].text += "\n" + line;
    } else if (/^\s*(#.*)?$/.test(line)) {
      lead.push(line);
    } else {
      return null; // a list, a value or an indented line before any key
    }
  }
  // Blank lines at the end belong to no key.
  for (const b of blocks) b.text = b.text.replace(/(\n[ \t]*)+$/, "");
  return blocks;
}

export interface TextEdit {
  /** In positions of the document before the edit, in order, not overlapping. */
  changes: { from: number; to: number; insert: string }[];
  /** Where the cursor goes, in the document after the edit. */
  cursor: number;
}

/**
 * Put a template's `text` in place of `from`..`to` of `doc`. When the template
 * starts with properties, they go into the note's properties instead of the
 * body, as in Obsidian: a key the note lacks is added at the end of its block,
 * and a key it has keeps the note's value. A note without properties gets the
 * template's block at the top. When either block is not a plain mapping,
 * the template goes in as it is.
 */
export function templateEdit(doc: string, from: number, to: number, text: string): TextEdit {
  const asIs: TextEdit = { changes: [{ from, to, insert: text }], cursor: from + text.length };
  const t = FRONTMATTER_RE.exec(text);
  if (!t) return asIs;
  const body = text.slice(t[0].length);
  const added = propertyBlocks(t[1] ?? "");
  if (!added) return asIs;
  const n = FRONTMATTER_RE.exec(doc);
  const bom = doc.startsWith("﻿") ? 1 : 0;

  let props: { at: number; insert: string } | null = null;
  if (n) {
    const have = propertyBlocks(n[1] ?? "");
    if (!have) return asIs;
    const keys = new Set(have.map((b) => b.key));
    const missing = added.filter((b) => !keys.has(b.key));
    if (missing.length) {
      const eol = /^﻿?---\r\n/.test(doc) ? "\r\n" : "\n";
      const lines = missing.map((b) => b.text).join("\n").replace(/\r?\n/g, eol);
      // After the last line of YAML, or after the opening `---` of an empty block.
      const open = bom + 3 + eol.length;
      const at = n[1] === undefined ? open : open + n[1].length;
      props = n[1] === undefined ? { at, insert: lines + eol } : { at, insert: eol + lines };
    }
    // A cursor in the properties: the body goes in below them.
    if (from < n[0].length) from = to = n[0].length;
  } else if (added.length) {
    // The template's block as written, ending in a line break.
    const block = t[0].replace(/^﻿/, "");
    props = { at: bom, insert: /\n$/.test(block) ? block : block + "\n" };
    if (from < bom) from = to = bom;
  }

  if (!props) return { changes: [{ from, to, insert: body }], cursor: from + body.length };
  if (props.at === from) {
    const insert = props.insert + body;
    return { changes: [{ from, to, insert }], cursor: from + insert.length };
  }
  const bodyChange = { from, to, insert: body };
  const propChange = { from: props.at, to: props.at, insert: props.insert };
  // The properties are always above the body.
  return { changes: [propChange, bodyChange], cursor: from + props.insert.length + body.length };
}
