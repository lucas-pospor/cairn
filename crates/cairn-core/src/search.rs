//! In-memory inverted index for full-text search.
//!
//! Terms are runs of letters and digits (with the combining marks after
//! them), plus each emoji on its own, all folded (see [`fold`]) so that NFC
//! and NFD spellings match. Chinese and Japanese, written without spaces,
//! are indexed as pairs of characters (see `cjk_tokens`). Every query word
//! is matched as a prefix ("prog" finds "programming"), all words must match
//! (AND), and quoted phrases are checked against the text and the file
//! name. Scoring is BM25 with a boost for query words that start a word of
//! the note's file name. Updates are per document.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::sync::Arc;

use serde::Serialize;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

const K1: f32 = 1.2;
const B: f32 = 0.75;

#[derive(Default)]
pub struct SearchIndex {
    ids: HashMap<String, u32>,
    docs: Vec<Option<Doc>>,
    free: Vec<u32>,
    /// Terms that occur in at least one document. A term is dropped as soon
    /// as no document has it, so words from old versions of a note (every
    /// autosave indexes the partial word being typed) do not pile up.
    dict: BTreeMap<Arc<str>, u32>,
    /// The same terms for exact lookups while indexing, which is much
    /// faster than the tree when there are many terms (each pair of
    /// characters in Chinese or Japanese text is one).
    lookup: HashMap<Arc<str>, u32>,
    /// term id -> the term and where it occurs
    terms: Vec<Term>,
    /// ids of dropped terms, reused for new ones
    free_terms: Vec<u32>,
    total_len: u64,
    live: u32,
}

struct Term {
    word: Arc<str>,
    /// sorted (doc id, term frequency)
    postings: Vec<(u32, u32)>,
}

struct Doc {
    path: String,
    /// folded file stem, for the title boost
    title: String,
    len: u32,
    terms: Vec<u32>,
}

/// Text in the form search compares it: NFC without variation selectors,
/// lowercased, and with the dot that lowercasing "İ" leaves (U+0307)
/// dropped, so "istanbul" finds "İstanbul". Note text, queries, tags and
/// paths all go through this.
pub fn fold(text: &str) -> String {
    fold_nfc(&nfc(text))
}

/// Variation selectors only pick a glyph (U+FE0F after "❤" asks for the
/// emoji style), so "❤" and "❤\u{fe0f}" are the same text to search.
fn is_variation_selector(c: char) -> bool {
    matches!(c as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

/// Enclosing marks such as the keycap U+20E3 in "1️⃣" draw around what came
/// before; they do not continue a word.
fn is_enclosing_mark(c: char) -> bool {
    matches!(c as u32, 0x0488..=0x0489 | 0x1ABE | 0x20DD..=0x20E0 | 0x20E2..=0x20E4 | 0xA670..=0xA672)
}

/// NFC, without variation selectors.
fn nfc(text: &str) -> Cow<'_, str> {
    if text.is_ascii() || (unicode_normalization::is_nfc(text) && !text.contains(is_variation_selector)) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.chars().filter(|&c| !is_variation_selector(c)).nfc().collect())
    }
}

fn fold_nfc(text: &str) -> String {
    let lower = text.to_lowercase();
    if lower.contains('\u{307}') {
        lower.replace("i\u{307}", "i")
    } else {
        lower
    }
}

/// Emoji and similar pictographs. They are not letters, so each one is
/// indexed as a word of its own.
fn is_pictograph(c: char) -> bool {
    matches!(c as u32,
        0x2300..=0x23FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF
        // emoji outside those blocks: © ® ‼ ⁉ ™ ↔-↙ ↩ ↪ ▪ ▫ ▶ ◀ ◻-◾ ⤴ ⤵ 〰 〽 ㊗ ㊙
        | 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2194..=0x2199 | 0x21A9..=0x21AA
        | 0x25AA..=0x25AB | 0x25B6 | 0x25C0 | 0x25FB..=0x25FE | 0x2934..=0x2935
        | 0x3030 | 0x303D | 0x3297 | 0x3299)
}

/// Han characters and kana: Chinese and Japanese are written without spaces
/// between words.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3005..=0x3007             // 々 〆 〇
        | 0x3041..=0x309F           // Hiragana
        | 0x30A1..=0x30FA           // Katakana (without ゠ and the middle dot ・)
        | 0x30FC..=0x30FF
        | 0x31F0..=0x31FF           // Katakana phonetic extensions
        | 0x3400..=0x4DBF           // CJK extension A
        | 0x4E00..=0x9FFF           // CJK unified ideographs
        | 0xF900..=0xFAFF           // CJK compatibility ideographs
        | 0xFF66..=0xFF9F           // halfwidth Katakana
        | 0x20000..=0x323AF)        // CJK extensions B to H
}

/// Folded word tokens of `text`, as the index stores them.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    each_token(text, false, |t| out.push(t));
    out
}

/// Calls `f` with each token of `text`. `query` selects how a run of CJK
/// characters is split (see [`cjk_tokens`]).
fn each_token(text: &str, query: bool, mut f: impl FnMut(String)) {
    let text = nfc(text);
    let mut word = None;
    let mut cjk = None;
    for (i, c) in text.char_indices() {
        if !c.is_ascii() && is_cjk(c) {
            if let Some(s) = word.take() {
                f(fold_nfc(&text[s..i]));
            }
            cjk.get_or_insert(i);
            continue;
        }
        if let Some(s) = cjk.take() {
            cjk_tokens(&text[s..i], query, &mut f);
        }
        // A combining mark (an NFD accent, a virama) belongs to the word
        // before it, but cannot start one.
        if c.is_alphanumeric() || (word.is_some() && !c.is_ascii() && is_combining_mark(c) && !is_enclosing_mark(c)) {
            word.get_or_insert(i);
            continue;
        }
        if let Some(s) = word.take() {
            f(fold_nfc(&text[s..i]));
        }
        if is_pictograph(c) {
            f(c.to_string());
        }
    }
    if let Some(s) = word {
        f(fold_nfc(&text[s..]));
    }
    if let Some(s) = cjk {
        cjk_tokens(&text[s..], query, &mut f);
    }
}

/// Tokens of a run of CJK characters: each pair of neighbouring characters
/// ("東京で" gives "東京", "京で"), so a word inside a sentence is found by
/// its pairs. In the index the run's last character is a token too, so that
/// a one-character query (a prefix of the pairs) finds every position. A
/// query of three or more characters is also checked as a phrase, see
/// [`Query::parse`].
fn cjk_tokens(run: &str, query: bool, f: &mut impl FnMut(String)) {
    let mut prev = None;
    for (i, c) in run.char_indices() {
        if let Some(p) = prev {
            f(run[p..i + c.len_utf8()].to_string());
        }
        prev = Some(i);
    }
    if let Some(p) = prev.filter(|&p| !query || p == 0) {
        f(run[p..].to_string());
    }
}

/// Term counts of one document, computed without touching the index.
pub struct PreparedDoc {
    counts: Vec<(String, u32)>,
    len: u32,
}

impl PreparedDoc {
    pub fn new(title: &str, body: &str) -> PreparedDoc {
        let mut tf: HashMap<String, u32> = HashMap::new();
        let mut len = 0u32;
        for text in [title, body] {
            each_token(text, false, |word| {
                len += 1;
                *tf.entry(word).or_default() += 1;
            });
        }
        PreparedDoc { counts: tf.into_iter().collect(), len }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub words: Vec<String>,
    pub phrases: Vec<String>,
    /// `tag:x` or `#x` filters (lowercase, without `#`).
    pub tags: Vec<String>,
    /// `path:x` / `file:x` filters (lowercase substrings of the path).
    pub paths: Vec<String>,
}

impl Query {
    pub fn parse(q: &str) -> Query {
        let mut out = Query::default();
        let mut rest = q;
        while let Some(i) = rest.find('"') {
            let (before, after) = (&rest[..i], &rest[i + 1..]);
            match after.find('"') {
                Some(j) => {
                    let quoted = after[..j].trim();
                    // `path:"My Folder"` and `tag:"two words"`: a quoted
                    // filter value may contain spaces.
                    let head = before.trim_end_matches(|c: char| !c.is_whitespace());
                    let last = &before[head.len()..];
                    if Self::FILTERS.iter().any(|f| last.eq_ignore_ascii_case(f)) {
                        out.add_plain(head);
                        out.add_filter(&fold(&format!("{last}{quoted}")));
                    } else {
                        out.add_plain(before);
                        let phrase = fold(quoted);
                        if !phrase.is_empty() {
                            each_token(&phrase, true, |t| out.words.push(t));
                            out.phrases.push(phrase);
                        }
                    }
                    rest = &after[j + 1..];
                }
                None => {
                    out.add_plain(before);
                    rest = after;
                }
            }
        }
        out.add_plain(rest);
        out.words.dedup();
        out
    }

    const FILTERS: [&str; 3] = ["tag:", "path:", "file:"];

    /// Add a folded `tag:x`, `path:x` or `file:x`; false if `lower` is none
    /// of these.
    fn add_filter(&mut self, lower: &str) -> bool {
        if let Some(t) = lower.strip_prefix("tag:") {
            let t = t.trim_start_matches('#');
            if !t.is_empty() {
                self.tags.push(t.to_string());
            }
        } else if let Some(p) = lower.strip_prefix("path:").or_else(|| lower.strip_prefix("file:")) {
            if !p.is_empty() {
                self.paths.push(p.to_string());
            }
        } else {
            return false;
        }
        true
    }

    fn add_plain(&mut self, text: &str) {
        for piece in text.split_whitespace() {
            let lower = fold(piece);
            if self.add_filter(&lower) {
                continue;
            }
            if lower.len() > 1 && lower.starts_with('#') {
                self.tags.push(lower[1..].to_string());
            } else {
                each_token(&lower, true, |t| self.words.push(t));
                // A longer CJK run is searched by its pairs, which can occur
                // apart, so it must also appear as written.
                for run in lower.split(|c: char| !is_cjk(c)) {
                    if run.chars().nth(2).is_some() {
                        self.phrases.push(run.to_string());
                    }
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.tags.is_empty() && self.paths.is_empty()
    }

    /// Whether a note passes the tag and path filters.
    pub fn filters_match(&self, path: &str, tags: &[String]) -> bool {
        let lp = fold(path);
        self.paths.iter().all(|p| lp.contains(p.as_str()))
            && self
                .tags
                .iter()
                .all(|t| tags.iter().any(|x| x == t || (x.starts_with(t.as_str()) && x.as_bytes().get(t.len()) == Some(&b'/'))))
    }
}

impl SearchIndex {
    pub fn len(&self) -> usize {
        self.live as usize
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    pub fn remove(&mut self, path: &str) {
        let Some(id) = self.ids.remove(path) else { return };
        if let Some(doc) = self.docs[id as usize].take() {
            for t in doc.terms {
                let term = &mut self.terms[t as usize];
                if let Ok(i) = term.postings.binary_search_by_key(&id, |&(d, _)| d) {
                    term.postings.remove(i);
                }
                if term.postings.is_empty() {
                    self.dict.remove(&term.word);
                    self.lookup.remove(&term.word);
                    term.postings = Vec::new();
                    self.free_terms.push(t);
                }
            }
            self.total_len -= doc.len as u64;
            self.live -= 1;
        }
        self.free.push(id);
    }

    /// Add or replace a document.
    pub fn upsert(&mut self, path: &str, title: &str, body: &str) {
        let prepared = PreparedDoc::new(title, body);
        self.upsert_prepared(path, title, prepared);
    }

    /// Add or replace a document whose tokens were counted beforehand
    /// (possibly on another thread).
    pub fn upsert_prepared(&mut self, path: &str, title: &str, prepared: PreparedDoc) {
        self.remove(path);
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                self.docs.push(None);
                (self.docs.len() - 1) as u32
            }
        };
        let mut terms = Vec::with_capacity(prepared.counts.len());
        for (word, n) in prepared.counts {
            let tid = match self.lookup.get(word.as_str()) {
                Some(&tid) => tid,
                None => self.add_term(word),
            };
            let p = &mut self.terms[tid as usize].postings;
            // Ids are mostly appended in increasing order, so this is usually a push.
            if p.last().is_none_or(|&(d, _)| d < id) {
                p.push((id, n));
            } else {
                let pos = p.partition_point(|&(d, _)| d < id);
                p.insert(pos, (id, n));
            }
            terms.push(tid);
        }
        self.ids.insert(path.to_string(), id);
        self.docs[id as usize] = Some(Doc {
            path: path.to_string(),
            title: fold(title),
            len: prepared.len,
            terms,
        });
        self.total_len += prepared.len as u64;
        self.live += 1;
    }

    fn add_term(&mut self, word: String) -> u32 {
        let word: Arc<str> = word.into();
        let term = Term { word: word.clone(), postings: Vec::new() };
        let tid = match self.free_terms.pop() {
            Some(tid) => {
                self.terms[tid as usize] = term;
                tid
            }
            None => {
                self.terms.push(term);
                (self.terms.len() - 1) as u32
            }
        };
        self.dict.insert(word.clone(), tid);
        self.lookup.insert(word, tid);
        tid
    }

    /// Score candidate documents. Phrase checks happen in the caller, which
    /// has the text. Returns (path, score), best first.
    pub fn search(&self, q: &Query) -> Vec<(String, f32)> {
        if q.words.is_empty() || self.live == 0 {
            return Vec::new();
        }
        let n = self.live as f32;
        let avg_len = (self.total_len as f32 / n).max(1.0);
        let mut acc: Option<HashMap<u32, f32>> = None;
        for word in &q.words {
            let mut scores: HashMap<u32, f32> = HashMap::new();
            for (term, &tid) in self.dict.range::<str, _>((Bound::Included(word.as_str()), Bound::Unbounded)) {
                if !term.starts_with(word.as_str()) {
                    break;
                }
                let post = &self.terms[tid as usize].postings;
                let df = post.len() as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                // Exact matches count more than prefix expansions.
                let exact = if &**term == word.as_str() { 1.0 } else { 0.6 };
                for &(doc, tf) in post {
                    let dl = self.docs[doc as usize].as_ref().map_or(1, |d| d.len) as f32;
                    let tf = tf as f32;
                    let s = idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / avg_len));
                    let e = scores.entry(doc).or_default();
                    *e = e.max(s * exact);
                }
            }
            acc = Some(match acc {
                None => scores,
                Some(prev) => prev
                    .into_iter()
                    .filter_map(|(d, s)| scores.get(&d).map(|s2| (d, s + s2)))
                    .collect(),
            });
            if acc.as_ref().is_some_and(|a| a.is_empty()) {
                return Vec::new();
            }
        }
        let mut out: Vec<(String, f32)> = acc
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(d, mut s)| {
                let doc = self.docs[d as usize].as_ref()?;
                s += self.title_boost(&doc.title, &q.words, n);
                Some((doc.path.clone(), s))
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// Boost for query words that start a word of the file name, weighted
    /// like a body match (idf, exact over prefix) at about the most one
    /// word of body text can score: the name says what the note is about,
    /// so it outranks a passing mention in a short note.
    fn title_boost(&self, title: &str, words: &[String], n: f32) -> f32 {
        let names = tokenize(title);
        let boost = |w: &str| {
            let best = names.iter().filter(|t| t.starts_with(w)).map(|t| {
                let df = self.lookup.get(t.as_str()).map_or(1, |&id| self.terms[id as usize].postings.len()) as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                idf * if t == w { 1.0 } else { 0.6 }
            });
            best.fold(0.0, f32::max) * (K1 + 1.0)
        };
        words.iter().map(|w| boost(w)).sum()
    }

    /// Rename a document without re-tokenizing the body.
    pub fn rename(&mut self, from: &str, to: &str, new_title: &str) {
        if let Some(id) = self.ids.remove(from) {
            if let Some(doc) = self.docs[id as usize].as_mut() {
                doc.path = to.to_string();
                doc.title = fold(new_title);
            }
            self.ids.insert(to.to_string(), id);
        }
    }
}

/// A piece of a snippet; `hit` marks matched text.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub hit: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Snippet {
    /// 0-based line of the first match in this snippet.
    pub line: u32,
    pub segments: Vec<Segment>,
}

/// Variation selectors, as a regex class (see [`is_variation_selector`]).
const VS_CLASS: &str = r"[\x{FE00}-\x{FE0F}\x{E0100}-\x{E01EF}]";

/// Build a case-insensitive regex matching any query word at a word start,
/// or any phrase.
pub fn highlight_regex(q: &Query) -> Option<regex::Regex> {
    // (length of the searched text, pattern)
    let mut alts: Vec<(usize, String)> = q.phrases.iter().map(|p| (p.len(), text_pattern(p))).collect();
    alts.extend(q.tags.iter().map(|t| (t.len() + 1, format!(r"#{}\b", text_pattern(t)))));
    alts.extend(q.words.iter().map(|w| {
        let pattern = if w.starts_with(|c: char| c.is_alphanumeric() && !is_cjk(c)) {
            // The regex counts a variation selector or the keycap mark as a
            // word character, so "Warning" in "⚠️Warning" or "1️⃣Step" has
            // no \b before it. `snippets` leaves those marks out of the hit.
            format!(r"(?:\b|[\x{{FE00}}-\x{{FE0F}}\x{{20E3}}]+){}\w*", text_pattern(w))
        } else {
            text_pattern(w)
        };
        (w.len(), pattern)
    }));
    if alts.is_empty() {
        return None;
    }
    // The first alternative that matches wins, so a phrase goes before the
    // words in it.
    alts.sort_by_key(|a| std::cmp::Reverse(a.0));
    let alts: Vec<String> = alts.into_iter().map(|a| a.1).collect();
    regex::RegexBuilder::new(&alts.join("|"))
        .case_insensitive(true)
        .build()
        .ok()
}

/// A regex for folded text `s` that also matches it in the note as written:
/// decomposed (NFD) letters, "İ" for "i", and a variation selector after an
/// emoji or a Han character. Snippets are cut from the note text, which is
/// not normalized.
fn text_pattern(s: &str) -> String {
    let mut out = String::new();
    let mut buf = [0u8; 4];
    for c in s.chars() {
        let one = regex::escape(c.encode_utf8(&mut buf));
        let nfd: String = std::iter::once(c).nfd().collect();
        if c == 'i' {
            out.push_str("[iİ]");
        } else if nfd.chars().count() > 1 {
            out.push_str(&format!("(?:{one}|{})", regex::escape(&nfd)));
        } else {
            out.push_str(&one);
        }
        if is_pictograph(c) || is_cjk(c) {
            out.push_str(VS_CLASS);
            out.push('?');
        }
    }
    out
}

/// Up to `max` snippets around matches in `text`, one per line.
pub fn snippets(text: &str, re: &regex::Regex, max: usize) -> Vec<Snippet> {
    let mut out = Vec::new();
    let mut last_line: Option<u32> = None;
    for (line_no, line) in text.lines().enumerate() {
        if out.len() >= max {
            break;
        }
        let hits: Vec<(usize, usize)> = re
            .find_iter(line)
            .map(|m| {
                // a variation selector or keycap stays with the emoji before it
                let rest = m.as_str().trim_start_matches(|c| is_variation_selector(c) || is_enclosing_mark(c));
                (m.end() - rest.len(), m.end())
            })
            .collect();
        if hits.is_empty() || last_line == Some(line_no as u32) {
            continue;
        }
        last_line = Some(line_no as u32);
        // Window of ~60 chars before the first hit and ~140 after it.
        let first = hits[0].0;
        let mut start = floor_char(line, first.saturating_sub(60));
        if start > 0 {
            // start at a word boundary
            if let Some(i) = line[start..first].find(' ') {
                start += i + 1;
            }
        }
        let end = floor_char(line, (first + 140).min(line.len()));
        let mut segs = Vec::new();
        if start > 0 {
            segs.push(Segment { text: "…".into(), hit: false });
        }
        let mut pos = start;
        for (s, e) in hits {
            if s < start || s >= end {
                continue;
            }
            let e = floor_char(line, e.min(end));
            if s > pos {
                segs.push(Segment { text: line[pos..s].to_string(), hit: false });
            }
            segs.push(Segment { text: line[s..e].to_string(), hit: true });
            pos = e;
        }
        if pos < end {
            segs.push(Segment { text: line[pos..end].to_string(), hit: false });
        }
        if end < line.len() {
            segs.push(Segment { text: "…".into(), hit: false });
        }
        out.push(Snippet { line: line_no as u32, segments: segs });
    }
    out
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> SearchIndex {
        let mut s = SearchIndex::default();
        s.upsert("rust.md", "Rust", "Rust is a systems programming language.");
        s.upsert("py.md", "Python", "Python is a programming language for scripting.");
        s.upsert("cook.md", "Cooking", "Recipes: bread, soup. Nothing about programs.");
        s
    }

    fn paths(r: Vec<(String, f32)>) -> Vec<String> {
        r.into_iter().map(|(p, _)| p).collect()
    }

    #[test]
    fn tokenizes_unicode() {
        let t = tokenize("Héllo, WORLD! naïve_x 日本語");
        assert_eq!(t, vec!["héllo", "world", "naïve", "x", "日本", "本語", "語"]);
    }

    #[test]
    fn tokens_are_normalized() {
        // NFD accents, a capital dotted I, a virama (a combining mark that is
        // not alphabetic) and emoji
        let t = tokenize("Cafe\u{301} İstanbul नोट्स party🎉time 👍🏽");
        assert_eq!(t, vec!["café", "istanbul", "नोट्स", "party", "🎉", "time", "👍", "🏽"]);
        assert_eq!(fold("CAFE\u{301} İ"), "café i");
        let q = Query::parse("Cafe\u{301} \"İstanbul trip\" #Cafe\u{301}");
        assert_eq!(q.words, vec!["café", "istanbul", "trip"]);
        assert_eq!(q.phrases, vec!["istanbul trip"]);
        assert_eq!(q.tags, vec!["café"]);
    }

    #[test]
    fn quoted_filter_values() {
        let q = Query::parse("garden Path:\"My Projects\"\u{3000}tag:\"#two Words\" \"a phrase\" x:\"y z\"");
        assert_eq!(q.paths, vec!["my projects"]);
        assert_eq!(q.tags, vec!["two words"]);
        assert_eq!(q.phrases, vec!["a phrase", "y z"]);
        assert_eq!(q.words, vec!["garden", "a", "phrase", "x", "y", "z"]);
    }

    #[test]
    fn marks_after_emoji_do_not_join_words() {
        // variation selectors (U+FE0F, U+E0100) and the keycap mark U+20E3
        // are not part of a word, and a mark cannot start one
        let t = tokenize("⚠\u{fe0f}Warning ❤\u{fe0f} love I ❤\u{fe0f}Rust 1\u{fe0f}\u{20e3}Step \u{301}x 葛\u{e0100}城");
        assert_eq!(t, vec!["⚠", "warning", "❤", "love", "i", "❤", "rust", "1", "step", "x", "葛城", "城"]);
        assert_eq!(fold("❤\u{fe0f}"), "❤");
        let mut s = SearchIndex::default();
        s.upsert("warn.md", "Warn", "⚠\u{fe0f}Warning: do not run this");
        s.upsert("heart.md", "Heart", "I ❤\u{fe0f}Rust");
        s.upsert("bare.md", "Bare", "a bare ❤");
        for (q, want) in [("warn", vec!["warn.md"]), ("rust", vec!["heart.md"]), ("❤\u{fe0f}", vec!["bare.md", "heart.md"]), ("\u{fe0f}", vec![])] {
            let mut got = paths(s.search(&Query::parse(q)));
            got.sort();
            assert_eq!(got, want, "{q}");
        }
        // ▶ ‼ ™ and ㊗ are emoji outside the main blocks
        assert_eq!(tokenize("▶play‼ Cairn™ ㊗"), vec!["▶", "play", "‼", "cairn", "™", "㊗"]);
    }

    #[test]
    fn cjk_runs_are_split_into_pairs() {
        assert_eq!(tokenize("東京で会議。Tokyo会議"), vec!["東京", "京で", "で会", "会議", "議", "tokyo", "会議", "議"]);
        assert_eq!(tokenize("ジョン・スミス"), vec!["ジョ", "ョン", "ン", "スミ", "ミス", "ス"]);
        let q = Query::parse("東京 京 会議があります");
        assert_eq!(q.words, vec!["東京", "京", "会議", "議が", "があ", "あり", "りま", "ます"]);
        assert_eq!(q.phrases, vec!["会議があります"]);
        let mut s = SearchIndex::default();
        s.upsert("ja.md", "Ja", "今日は東京で会議があります。北京");
        s.upsert("zh.md", "Zh", "我们明天去北京开会。");
        for (q, want) in [("東京", vec!["ja.md"]), ("京", vec!["ja.md", "zh.md"]), ("会議", vec!["ja.md"]), ("开会", vec!["zh.md"]), ("北京", vec!["ja.md", "zh.md"])] {
            let mut got = paths(s.search(&Query::parse(q)));
            got.sort();
            assert_eq!(got, want, "{q}");
        }
        let re = highlight_regex(&Query::parse("東京")).unwrap();
        assert_eq!(re.find("今日は東京で").map(|m| m.as_str()), Some("東京"));
    }

    #[test]
    fn highlight_matches_text_as_written() {
        let q = Query::parse("café istanbul 🎉");
        let re = highlight_regex(&q).unwrap();
        let text = "CAFE\u{301} in İstanbul 🎉";
        let hits: Vec<&str> = re.find_iter(text).map(|m| m.as_str()).collect();
        assert_eq!(hits, vec!["CAFE\u{301}", "İstanbul", "🎉"]);
        let q = Query::parse("warn ❤ step 葛城");
        let re = highlight_regex(&q).unwrap();
        let text = "⚠\u{fe0f}Warning ❤\u{fe0f} 1\u{fe0f}\u{20e3}Step 葛\u{e0100}城";
        let hits: Vec<String> = snippets(text, &re, 1)[0].segments.iter().filter(|s| s.hit).map(|s| s.text.clone()).collect();
        assert_eq!(hits, vec!["Warning", "❤\u{fe0f}", "Step", "葛\u{e0100}城"]);
    }

    #[test]
    fn and_prefix_and_ranking() {
        let s = idx();
        let r = paths(s.search(&Query::parse("program")));
        assert_eq!(r.len(), 3); // programming x2, programs x1
        let r = paths(s.search(&Query::parse("programming language")));
        assert_eq!(r.len(), 2);
        assert!(!r.contains(&"cook.md".to_string()));
        let r = paths(s.search(&Query::parse("python")));
        assert_eq!(r, vec!["py.md"]);
        assert!(s.search(&Query::parse("nonexistent")).is_empty());
    }

    #[test]
    fn title_boost() {
        let mut s = SearchIndex::default();
        s.upsert("a.md", "Other", "garden garden garden");
        s.upsert("garden.md", "Garden", "one mention");
        let r = paths(s.search(&Query::parse("garden")));
        assert_eq!(r[0], "garden.md");
    }

    #[test]
    fn update_and_remove() {
        let mut s = idx();
        s.upsert("rust.md", "Rust", "Now about gardening.");
        assert_eq!(paths(s.search(&Query::parse("garden"))), vec!["rust.md"]);
        assert_eq!(paths(s.search(&Query::parse("systems"))), Vec::<String>::new());
        s.remove("rust.md");
        assert_eq!(s.len(), 2);
        assert!(s.search(&Query::parse("garden")).is_empty());
        s.upsert("new.md", "New", "garden again");
        assert_eq!(paths(s.search(&Query::parse("garden"))), vec!["new.md"]);
        s.rename("new.md", "renamed.md", "Renamed");
        assert_eq!(paths(s.search(&Query::parse("garden"))), vec!["renamed.md"]);
    }

    #[test]
    fn terms_of_old_versions_are_dropped() {
        let mut s = SearchIndex::default();
        s.upsert("a.md", "A", "shared");
        for t in ["p", "pr", "pre", "pres", "press"] {
            s.upsert("b.md", "B", &format!("shared {t}"));
        }
        // Only a, b, shared and press are left; the dropped ids are reused.
        assert_eq!(s.dict.len(), 4);
        assert!(s.terms.len() <= 5, "{} term slots", s.terms.len());
        assert_eq!(paths(s.search(&Query::parse("pre"))), vec!["b.md"]);
        s.remove("b.md");
        assert_eq!(s.dict.keys().map(|k| &**k).collect::<Vec<_>>(), vec!["a", "shared"]);
        assert_eq!(paths(s.search(&Query::parse("shared"))), vec!["a.md"]);
    }

    #[test]
    fn query_parsing() {
        let q = Query::parse(r#"foo "Bar Baz" qux"#);
        assert_eq!(q.words, vec!["foo", "bar", "baz", "qux"]);
        assert_eq!(q.phrases, vec!["bar baz"]);
        let q = Query::parse("garden tag:Plants #home/kitchen path:Projects/");
        assert_eq!(q.words, vec!["garden"]);
        assert_eq!(q.tags, vec!["plants", "home/kitchen"]);
        assert_eq!(q.paths, vec!["projects/"]);
        assert!(q.filters_match("Projects/a.md", &["plants".into(), "home/kitchen/sink".into()]));
        assert!(!q.filters_match("Projects/a.md", &["plants".into(), "home/kitchenette".into()]));
        assert!(!q.filters_match("Other/a.md", &["plants".into(), "home/kitchen".into()]));
        let q = Query::parse(r#"unterminated "quote"#);
        assert_eq!(q.words, vec!["unterminated", "quote"]);
    }

    #[test]
    fn snippet_highlighting() {
        let q = Query::parse("lang");
        let re = highlight_regex(&q).unwrap();
        let text = "first line\nRust is a systems programming Language.\nslang is not a match";
        let s = snippets(text, &re, 5);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].line, 1);
        let hits: Vec<_> = s[0].segments.iter().filter(|x| x.hit).map(|x| x.text.as_str()).collect();
        assert_eq!(hits, vec!["Language"]);
    }

    #[test]
    fn snippet_multibyte_safe() {
        let q = Query::parse("ü");
        let re = highlight_regex(&q).unwrap();
        let line = format!("{}über", "é".repeat(100));
        let s = snippets(&line, &re, 1);
        // regex \b before ü fails after é (both word chars), so no hit is fine;
        // the point is no panic on char boundaries.
        assert!(s.len() <= 1);
        let line = format!("{} über {}", "é".repeat(100), "ö".repeat(200));
        let s = snippets(&line, &re, 1);
        assert_eq!(s.len(), 1);
    }
}
