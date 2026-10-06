// Words and characters of a text, for the status bar.
//
// Words: a run of letters, marks and digits is one word, with ' ’ and - inside it
// ("don't", "well-known"). Chinese and Japanese are written without spaces, so each
// Han, Hiragana or Katakana character counts as one word, as most word processors
// count them. Korean is written with spaces and counts like English.
//
// Thai, Lao, Khmer and Myanmar are written without spaces too, but their words are
// longer than one character: Intl.Segmenter finds them with the system's dictionary
// (ICU). That data is not the same in the desktop's web view and Android's, so the
// counts for these scripts can differ a little between the two.
//
// Characters: what a reader sees as one character (a letter with its accents, an
// emoji with its skin tone), spaces included, line breaks not.

const CJK = "\\p{Script=Han}\\p{Script=Hiragana}\\p{Script=Katakana}\\u30FC\\uFF70";
const NO_SPACES = "\\p{Script=Thai}\\p{Script=Lao}\\p{Script=Khmer}\\p{Script=Myanmar}";
const OTHER = `(?![${CJK}${NO_SPACES}])`;
const WORD = new RegExp(`[${CJK}]|[${NO_SPACES}][${NO_SPACES}\\p{M}]*|${OTHER}[\\p{L}\\p{N}](?:${OTHER}[\\p{L}\\p{M}\\p{N}'’-])*`, "gu");
const NO_SPACES_RUN = new RegExp(`^[${NO_SPACES}]`, "u");
const LINE_BREAK = /\r\n|[\n\r\u2028\u2029]/g;

export type Segmenters = { word: Intl.Segmenter; grapheme: Intl.Segmenter } | null;

let cached: Segmenters | undefined;

/** The system's segmenters; null where there is no Intl.Segmenter. */
function systemSegmenters(): Segmenters {
  if (cached === undefined) {
    cached =
      typeof Intl.Segmenter === "function"
        ? { word: new Intl.Segmenter(undefined, { granularity: "word" }), grapheme: new Intl.Segmenter(undefined, { granularity: "grapheme" }) }
        : null;
  }
  return cached;
}

export interface Counts {
  words: number;
  characters: number;
}

/** The words and characters of `text`. Without segmenters, a Thai (Lao...) run counts as one word and each code point as a character. */
export function countText(text: string, seg: Segmenters = systemSegmenters()): Counts {
  let words = 0;
  for (const [w] of text.matchAll(WORD)) {
    if (!NO_SPACES_RUN.test(w)) words++;
    else if (!seg) words += /[\p{L}\p{N}]/u.test(w) ? 1 : 0;
    else for (const s of seg.word.segment(w)) if (s.isWordLike) words++;
  }
  const t = text.replace(LINE_BREAK, "");
  let characters: number;
  // Latin text: one code unit is one character (nothing combines), so no need to segment.
  if (/^[\u0000-˿]*$/.test(t)) characters = t.length;
  else if (!seg) characters = Array.from(t).length;
  else {
    characters = 0;
    for (const _ of seg.grapheme.segment(t)) characters++;
  }
  return { words, characters };
}
