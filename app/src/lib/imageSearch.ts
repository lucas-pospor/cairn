// Images for the search panel. Full-text search (the core's Query) covers
// notes only; this finds images by their path with the same query syntax:
// every word and every path:/file: filter must be in the path, folded and
// split into words as the core does. Tags (tag:x, #x) and quoted phrases are
// about a note's text, so a query with one finds no image.

import { isHidden, isImage } from "./paths";

/** As the core's search::fold: NFC without variation selectors, lowercased, without the dot lowercasing "İ" leaves. */
export function fold(text: string): string {
  return text
    .replace(/[︀-️]|\uDB40[\uDD00-\uDDEF]/g, "")
    .normalize("NFC")
    .toLowerCase()
    .replaceAll("i̇", "i");
}

// The core's is_cjk: Han and kana, written without spaces between words.
const CJK = /[々-〇ぁ-ゟァ-ヺー-ヿㇰ-ㇿ㐀-䶿一-鿿豈-﫿ｦ-ﾟ\u{20000}-\u{323AF}]/u;
// Rust's char::is_alphanumeric.
const ALNUM = /[\p{Alphabetic}\p{N}]/u;
// A combining mark continues a word; an enclosing one (the keycap in "1️⃣") does not.
const CONTINUES = /(?!\p{Me})\p{M}/u;

/** The core's is_pictograph: each one is a word of its own. */
function isPictograph(c: number): boolean {
  return (
    (c >= 0x2300 && c <= 0x23ff) || (c >= 0x2600 && c <= 0x27bf) || (c >= 0x2b00 && c <= 0x2bff) || (c >= 0x1f000 && c <= 0x1faff) ||
    [0xa9, 0xae, 0x203c, 0x2049, 0x2122, 0x25b6, 0x25c0, 0x3030, 0x303d, 0x3297, 0x3299].includes(c) ||
    (c >= 0x2194 && c <= 0x2199) || (c >= 0x21a9 && c <= 0x21aa) || (c >= 0x25aa && c <= 0x25ab) || (c >= 0x25fb && c <= 0x25fe) || (c >= 0x2934 && c <= 0x2935)
  );
}

/**
 * The words of folded `text`, as the core's each_token splits a query. A run
 * of CJK characters stays whole: the core searches it by its pairs and, from
 * three characters, as a phrase, which for a path is the same as the run.
 */
export function words(text: string): string[] {
  const out: string[] = [];
  let word = "";
  let cjk = "";
  for (const c of text) {
    if (c > "\x7f" && CJK.test(c)) {
      if (word) out.push(word);
      word = "";
      cjk += c;
      continue;
    }
    if (cjk) out.push(cjk);
    cjk = "";
    if (ALNUM.test(c) || (word && CONTINUES.test(c))) {
      word += c;
      continue;
    }
    if (word) out.push(word);
    word = "";
    if (isPictograph(c.codePointAt(0)!)) out.push(c);
  }
  if (word) out.push(word);
  if (cjk) out.push(cjk);
  return out;
}

const FILTERS = ["tag:", "path:", "file:"];

interface ImageQuery {
  words: string[];
  paths: string[];
  /** A tag filter or a quoted phrase: matches no image. */
  noteOnly: boolean;
}

function addFilter(q: ImageQuery, lower: string): boolean {
  if (lower.startsWith("tag:")) {
    if (lower.slice(4).replace(/^#+/, "")) q.noteOnly = true;
  } else if (lower.startsWith("path:") || lower.startsWith("file:")) {
    if (lower.length > 5) q.paths.push(lower.slice(5));
  } else return false;
  return true;
}

function addPlain(q: ImageQuery, text: string) {
  for (const piece of text.split(/\s+/)) {
    if (!piece) continue;
    const lower = fold(piece);
    if (addFilter(q, lower)) continue;
    if (lower.length > 1 && lower.startsWith("#")) q.noteOnly = true;
    else q.words.push(...words(lower));
  }
}

/** The query as the core's Query::parse reads it, for paths. */
export function parseImageQuery(text: string): ImageQuery {
  const q: ImageQuery = { words: [], paths: [], noteOnly: false };
  let rest = text;
  for (let i = rest.indexOf('"'); i >= 0; i = rest.indexOf('"')) {
    const before = rest.slice(0, i);
    const after = rest.slice(i + 1);
    const j = after.indexOf('"');
    if (j < 0) {
      addPlain(q, before);
      rest = after;
      continue;
    }
    const quoted = after.slice(0, j).trim();
    // path:"My Folder": a quoted filter value may contain spaces.
    let k = before.length;
    while (k > 0 && !/\s/.test(before[k - 1])) k--;
    const last = before.slice(k);
    if (FILTERS.includes(last.toLowerCase())) {
      addPlain(q, before.slice(0, k));
      addFilter(q, fold(last + quoted));
    } else {
      addPlain(q, before);
      if (fold(quoted)) q.noteOnly = true;
    }
    rest = after.slice(j + 1);
  }
  addPlain(q, rest);
  return q;
}

/** Image paths (sorted) whose path contains every word and path filter of `query`; at most `limit`. */
export function matchImages(paths: string[], query: string, limit = 100): string[] {
  const q = parseImageQuery(query);
  if (q.noteOnly || (!q.words.length && !q.paths.length)) return [];
  const out: string[] = [];
  for (const p of paths) {
    if (!isImage(p) || isHidden(p)) continue;
    const lp = fold(p);
    if (q.paths.every((f) => lp.includes(f)) && q.words.every((w) => lp.includes(w))) out.push(p);
  }
  out.sort((a, b) => a.localeCompare(b));
  return out.slice(0, limit);
}
