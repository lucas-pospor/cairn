// Link helpers used by the editor and preview. The Rust index is the
// authority on resolution; this module only gives fast answers for
// autocomplete text and for styling links as resolved or not.

import type { FileStat } from "./types";
import { isMarkdown, stem, fileName, resolveRelative } from "./paths";

export function linkKeyForFile(p: string): string {
  return (isMarkdown(p) ? stem(p) : fileName(p)).toLowerCase();
}

/** Trim as the core does (Rust's str::trim: Unicode White_Space). JS trim()
 *  also takes U+FEFF and leaves U+0085. */
export function trimLink(s: string): string {
  return s.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
}

/** Link text as the core reads it: '\' is a separator too, a leading '/'
 *  means the vault root, and the text is NFC like paths. */
function normalizeTarget(target: string): string {
  return trimLink(target).replace(/\\/g, "/").replace(/^\/+/, "").normalize("NFC");
}

/** Same key from link text: NFC like paths, without a note extension. */
export function linkKeyForTarget(target: string): string {
  return stripNoteExt(fileName(target.replace(/\/+$/, "")).normalize("NFC").toLowerCase());
}

/** Lowercase link text without a trailing `.md` or `.markdown`. */
function stripNoteExt(lower: string): string {
  return lower.replace(/\.(md|markdown)$/, "");
}

/** Order by code points, like Rust strings (`<` compares UTF-16 units). */
function compareCodePoints(a: string, b: string): number {
  for (let i = 0; i < a.length && i < b.length; ) {
    const x = a.codePointAt(i)!;
    const y = b.codePointAt(i)!;
    if (x !== y) return x - y;
    i += x > 0xffff ? 2 : 1;
  }
  return a.length - b.length;
}

export class LinkIndex {
  private byKey = new Map<string, string[]>();
  private paths = new Set<string>();

  constructor(entries: FileStat[]) {
    for (const e of entries) {
      if (e.kind !== "file") continue;
      this.paths.add(e.path);
      const k = linkKeyForFile(e.path);
      const list = this.byKey.get(k);
      if (list) list.push(e.path);
      else this.byKey.set(k, [e.path]);
    }
  }

  /**
   * Shortest text that links unambiguously to `path` from any note: the
   * name, else the path, each without and then with the extension. Null when
   * no [[text]] reads back as this file: a name with # | [ or ], or a file
   * the core takes for another one (a/note.md next to a/Note.md).
   */
  linkText(path: string): string | null {
    const md = isMarkdown(path);
    const texts = [md ? path.replace(/\.(md|markdown)$/i, "") : path, path];
    if ((this.byKey.get(linkKeyForFile(path)) ?? []).length <= 1) texts.unshift(md ? stem(path) : fileName(path), fileName(path));
    return texts.find((t) => !/[[\]\n]/.test(t) && splitWikilink(t).target === t && this.resolve(t) === path) ?? null;
  }

  /**
   * Resolve a link target to a path, following the same rules as the Rust
   * index (exact path, then relative to the source folder, then same folder,
   * then shortest path). Used where a synchronous answer is needed, such as
   * image sources.
   */
  resolve(target: string, source = ""): string | null {
    const t = normalizeTarget(target);
    if (!t) return null;
    if (t.endsWith("/")) return null; // a folder is not a link target
    const cands = this.byKey.get(linkKeyForTarget(t));
    if (!cands?.length) return null;
    const want = stripNoteExt(t.toLowerCase());
    const noExt = (c: string) => (/\.(md|markdown)$/i.test(c) ? c.replace(/\.(md|markdown)$/i, "") : c).toLowerCase();
    const srcDir = source.includes("/") ? source.slice(0, source.lastIndexOf("/")) : "";
    // Null when ../ climbs out of the vault. NFC, as in the core.
    const rel = resolveRelative(srcDir, want)?.normalize("NFC").toLowerCase() ?? null;
    // The folders a same-name match must end in: ../b/Note -> b/note.
    const tail = want
      .split("/")
      .filter((s) => s && s !== "." && s !== "..")
      .join("/");
    let best: [number, number, string] | null = null;
    for (const c of cands) {
      const n = noExt(c);
      const cDir = c.includes("/") ? c.slice(0, c.lastIndexOf("/")) : "";
      let rank: number;
      if (n === want) rank = 0;
      else if (want.includes("/") && n === rel) rank = 1;
      else if (tail.includes("/") && n !== tail && !n.endsWith("/" + tail)) continue;
      else if (cDir === srcDir) rank = 2;
      else rank = 3;
      // Length in characters, then code point order, as in the core.
      const cand: [number, number, string] = [rank, [...c].length, c];
      if (!best || cand[0] < best[0] || (cand[0] === best[0] && (cand[1] < best[1] || (cand[1] === best[1] && compareCodePoints(cand[2], best[2]) < 0)))) best = cand;
    }
    return best ? best[2] : null;
  }

  /** Whether a file with exactly this path (same case) is in the vault. */
  has(path: string): boolean {
    return this.paths.has(path);
  }

  /** Whether a link target in the note `source` resolves (for styling). */
  exists(target: string, source = ""): boolean {
    const t = normalizeTarget(target);
    if (!t) return true;
    return this.resolve(t, source) !== null;
  }
}

export interface WikiLinkParts {
  target: string;
  subpath: string | null;
  alias: string | null;
}

export function splitWikilink(inner: string): WikiLinkParts {
  let main = inner;
  let alias: string | null = null;
  const bar = inner.indexOf("|");
  if (bar >= 0) {
    main = inner.slice(0, bar).replace(/\\$/, "");
    alias = trimLink(inner.slice(bar + 1)) || null;
  }
  let subpath: string | null = null;
  const hash = main.indexOf("#");
  if (hash >= 0) {
    subpath = trimLink(main.slice(hash + 1)) || null;
    main = main.slice(0, hash);
  }
  return { target: trimLink(main), subpath, alias };
}

export const WIKILINK_RE = /(!?)\[\[([^[\]\n]+?)\]\]/g;

/** Find the wikilink covering `pos` in `line` (positions relative to line). */
export function wikilinkAt(line: string, pos: number): (WikiLinkParts & { from: number; to: number; embed: boolean }) | null {
  WIKILINK_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = WIKILINK_RE.exec(line))) {
    const from = m.index;
    const to = from + m[0].length;
    if (pos >= from && pos <= to) {
      return { ...splitWikilink(m[2]), from, to, embed: m[1] === "!" };
    }
  }
  return null;
}

/** If the cursor is inside an unfinished `[[`, the query typed so far. */
export function openWikilinkQuery(textBefore: string): { query: string; start: number } | null {
  const m = /\[\[([^[\]|#\n]*)$/.exec(textBefore);
  if (!m) return null;
  return { query: m[1], start: m.index + 2 };
}
