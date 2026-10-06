// Adversarial tests for full-text search.
//
//   cargo test -p cairn-core --test adv_search
//   cargo test -p cairn-core --test adv_search finding_     (FINDING tests only)

use std::fs;
use std::sync::Arc;
use std::time::Instant;

use cairn_core::fs::{EntryKind, FileStat};
use cairn_core::index::{hash_bytes, Index, SearchHit};
use cairn_core::search::{highlight_regex, snippets, Query};
use cairn_core::{StdFs, TrashMode, Vault};

fn st(path: &str) -> FileStat {
    FileStat { path: path.into(), kind: EntryKind::File, size: 0, mtime: 0 }
}

fn index(files: &[(&str, &str)]) -> Index {
    let mut idx = Index::default();
    for (p, c) in files {
        idx.put_note(st(p), c.to_string(), hash_bytes(c.as_bytes()));
    }
    idx
}

fn paths(h: Vec<SearchHit>) -> Vec<String> {
    h.into_iter().map(|h| h.path).collect()
}

fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

// ---------------------------------------------------------------------------
// Held up
// ---------------------------------------------------------------------------

#[test]
fn held_filters_tags_paths_case_and_nesting() {
    let idx = index(&[
        ("Projects/Garden.md", "---\ntags: [Plants/Herbs]\n---\nbasil"),
        ("projects/b.md", "#plants basil"),
        ("Other/c.md", "#PlantsX basil #home/Kitchen/Sink"),
    ]);
    assert_eq!(paths(idx.search("tag:PLANTS", 10)), vec!["Projects/Garden.md", "projects/b.md"]);
    assert_eq!(paths(idx.search("tag:#plants/herbs", 10)), vec!["Projects/Garden.md"]);
    assert_eq!(paths(idx.search("#home/kitchen", 10)), vec!["Other/c.md"]);
    assert!(idx.search("#home/kit", 10).is_empty(), "tag prefixes inside a segment do not match");
    assert_eq!(paths(idx.search("basil PATH:projects/", 10)).len(), 2, "path filter is case-insensitive");
    assert_eq!(paths(idx.search("file:other basil", 10)), vec!["Other/c.md"]);
}

#[test]
fn held_empty_whitespace_and_punctuation_queries_return_nothing_without_panicking() {
    let idx = index(&[("a.md", "some text, (with) punctuation!")]);
    for q in ["", " ", "\t\n", "!!!", "#", "tag:", "path:", "\"\"", "\"", "\"   \"", "(", "*", "\\", "tag:#", "%%%"] {
        assert!(idx.search(q, 10).is_empty(), "query {q:?}");
    }
}

#[test]
fn held_prefix_phrase_and_ranking() {
    let idx = index(&[
        ("Garden.md", "A short note."),
        ("long.md", &format!("{} garden {}", "filler ".repeat(200), "filler ".repeat(200))),
        ("prog.md", "Programming in Rust; the quick brown fox."),
    ]);
    assert_eq!(paths(idx.search("garden", 10))[0], "Garden.md", "title match first");
    assert_eq!(paths(idx.search("prog", 10)), vec!["prog.md"]);
    assert_eq!(paths(idx.search("\"QUICK brown\"", 10)), vec!["prog.md"]);
    assert!(idx.search("\"brown quick\"", 10).is_empty());
    // single document: BM25 idf stays positive
    let one = index(&[("only.md", "lonely word")]);
    let h = one.search("lonely", 10);
    assert!(h[0].score > 0.0);
}

#[test]
fn held_results_follow_renames_and_deletes() {
    let (_d, v) = vault(&[("a/Old.md", "zebra stripes"), ("b.md", "zebra crossing")]);
    v.rename("a/Old.md", "a/New name.md").unwrap();
    let mut z = paths(v.search("zebra", 10));
    z.sort();
    assert_eq!(z, vec!["a/New name.md", "b.md"]);
    assert_eq!(paths(v.search("new", 10)), vec!["a/New name.md"], "title follows the rename");
    assert!(v.search("old", 10).is_empty(), "no stale title postings");
    v.rename("a", "c").unwrap();
    assert_eq!(paths(v.search("stripes", 10)), vec!["c/New name.md"]);
    v.delete("b.md").unwrap();
    assert_eq!(paths(v.search("zebra", 10)), vec!["c/New name.md"]);
    v.write_note("c/New name.md", "now about horses", None).unwrap();
    assert!(v.search("zebra", 10).is_empty());
}

#[test]
fn held_unicode_case_folding_basics() {
    let idx = index(&[("u.md", "Über CAFÉ naïve Ελληνικά ÆSIR")]);
    for q in ["über", "ÜBER", "café", "Café", "naïve", "ελληνικά", "æsir", "üb"] {
        assert_eq!(idx.search(q, 10).len(), 1, "query {q:?}");
    }
}

#[test]
fn held_snippets_highlight_multibyte_correctly() {
    let idx = index(&[("m.md", &format!("{} ünïcode wörd {} end", "日本語 ".repeat(40), "é".repeat(300)))]);
    let h = idx.search("wörd", 10);
    let segs = &h[0].snippets[0].segments;
    let hits: Vec<&str> = segs.iter().filter(|s| s.hit).map(|s| s.text.as_str()).collect();
    assert_eq!(hits, vec!["wörd"]);
    assert_eq!(segs.first().unwrap().text, "…");
    assert_eq!(segs.last().unwrap().text, "…");
}

/// Random text and queries through search, highlight and snippets: no panic,
/// and every snippet is made of pieces of the line it points at.
#[test]
fn held_fuzz_search_and_snippets_no_panic() {
    let alphabet: Vec<char> = "aAbBéÉüß İıſ日本語😀👍🏽\u{301}\u{200d}#\"'[]()|^*.\\/-_ \n\t:tagpath0123456789".chars().collect();
    let mut r = Rng(0xdead_beef);
    for round in 0..300 {
        let len = r.below(400);
        let text: String = (0..len).map(|_| alphabet[r.below(alphabet.len())]).collect();
        let qlen = r.below(12);
        let q: String = (0..qlen).map(|_| alphabet[r.below(alphabet.len())]).collect();
        let idx = index(&[("f.md", &text), ("g.md", "filler words here")]);
        let hits = idx.search(&q, 10);
        for h in hits {
            for s in &h.snippets {
                let line = text.lines().nth(s.line as usize).unwrap_or_else(|| panic!("round {round}: bad line {}", s.line));
                let joined: String = s.segments.iter().filter(|x| x.text != "…").map(|x| x.text.as_str()).collect();
                assert!(line.contains(&joined), "round {round}: snippet {joined:?} not in line {line:?}");
            }
        }
        let parsed = Query::parse(&q);
        if let Some(re) = highlight_regex(&parsed) {
            let _ = snippets(&text, &re, 5);
        }
    }
}

#[test]
fn held_long_queries_are_fast_enough() {
    let mut files = Vec::new();
    for i in 0..300 {
        files.push((format!("n{i}.md"), format!("word{i} common text garden river {}", "x".repeat(i % 7))));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let idx = index(&refs);
    let long = "garden ".repeat(2000);
    let t = Instant::now();
    let _ = idx.search(&long, 100);
    let distinct: String = (0..2000).map(|i| format!("w{i} ")).collect();
    let _ = idx.search(&distinct, 100);
    let huge_phrase = format!("\"{}\"", "a ".repeat(5000));
    let _ = idx.search(&huge_phrase, 100);
    let el = t.elapsed();
    eprintln!("long queries took {el:?}");
    assert!(el.as_secs_f32() < 10.0, "long queries took {el:?}");
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

/// Each query word expands to every dictionary term it is a prefix of. With
/// the former cap of 256 terms (MAX_EXPANSION, in alphabetical order), notes
/// whose word sorted later were silently missing from the results.
#[test]
fn finding_prefix_expansion_drops_no_results() {
    let mut files = Vec::new();
    for i in 0..300 {
        files.push((format!("n{i:03}.md"), format!("pre{i:03}xyz something")));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let idx = index(&refs);
    let n = idx.search("pre", 1000).len();
    assert_eq!(n, 300, "query 'pre' must find every note containing a word starting with 'pre'");
    // AND with another word: the later notes cannot be found by a prefix at all
    assert_eq!(paths(idx.search("pre something", 1000)).len(), 300);
}

/// The search dictionary drops a term once no note has it. Words that existed
/// only in earlier versions of a note (every autosave while typing indexes
/// the partial words) used to stay as dead terms and use up the 256-term
/// prefix budget; after a long editing session, prefix search must still find
/// the notes a fresh open finds.
#[test]
fn finding_dead_terms_hide_no_results_incremental_vs_fresh() {
    let (d, v) = vault(&[("target.md", "the prezzz plan")]);
    v.create_note("draft.md", "").unwrap();
    // Simulate autosave while typing 81 words starting with "pre": each save
    // indexes the partial word typed so far.
    let words = [
        "prepare", "present", "preserve", "pressure", "prevent", "previous", "premium", "prefer", "predict", "prelude",
        "premise", "prepaid", "presence", "preset", "preside", "press", "prestige", "presume", "pretend", "pretext",
        "prevail", "preview", "prey", "precede", "precise", "precinct", "preclude", "predator", "preface", "prefix",
        "pregnant", "prehistoric", "prejudice", "preliminary", "premature", "premiere", "preoccupy", "prescribe", "preamble", "precaution",
        "precedent", "precious", "precipitate", "precision", "preconceive", "precursor", "predecessor", "predicament", "predominant",
        "preeminent", "preexisting", "prefabricate", "preferential", "pregame", "preheat", "prelaunch", "premeditate", "preorder",
        "prepackage", "preponderance", "preposition", "prerequisite", "prerogative", "presbyopia", "preschool", "prescient",
        "presentation", "preservative", "presidency", "presidential", "pressurize", "presumption", "pretentious", "preternatural",
        "prettiness", "pretzel", "prevalence", "prevaricate", "preventable", "preventive", "prewar",
    ];
    let mut text = String::new();
    for w in words {
        for i in 1..=w.len() {
            let mut t = text.clone();
            t.push_str(&w[..i]);
            v.write_note("draft.md", &t, None).unwrap();
        }
        text.push_str(w);
        text.push(' ');
    }
    // The user then deletes the draft text.
    v.write_note("draft.md", "", None).unwrap();
    let fresh = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    assert_eq!(paths(fresh.search("pre", 10)), vec!["target.md"], "fresh open finds it");
    assert_eq!(paths(v.search("pre", 10)), vec!["target.md"], "the long-running index must find it too");
}

/// Chinese/Japanese text without spaces is indexed as pairs of characters,
/// so a word in the middle of a sentence is found (as one huge token, only
/// its first characters could be searched).
#[test]
fn finding_cjk_search() {
    let idx = index(&[("ja.md", "今日は東京で会議があります。"), ("zh.md", "我们明天去北京开会。")]);
    assert_eq!(paths(idx.search("東京", 10)), vec!["ja.md"]);
    assert_eq!(paths(idx.search("会議", 10)), vec!["ja.md"]);
    assert_eq!(paths(idx.search("北京", 10)), vec!["zh.md"]);
}

/// `path:` takes a quoted value as one filter, not as a phrase, so a folder
/// with a space in its name can be used as a filter.
#[test]
fn finding_path_filter_with_spaces() {
    let idx = index(&[("My Projects/a.md", "garden"), ("My/b.md", "garden"), ("Projects/c.md", "garden")]);
    assert_eq!(paths(idx.search("garden path:\"my projects\"", 10)), vec!["My Projects/a.md"]);
}

/// Text and queries are normalized as well as lowercased: NFD text (macOS
/// file names pasted into notes, some input methods) matches an NFC
/// query, "İstanbul" lowercases to "i̇stanbul" (with U+0307) so "istanbul"
/// would miss it if the dot stayed, and an emoji-only query is searched.
#[test]
fn finding_search_unicode_normalization() {
    let idx = index(&[("nfd.md", "Cafe\u{301} cre\u{300}me"), ("tr.md", "İstanbul trip"), ("e.md", "party 🎉 time")]);
    assert_eq!(paths(idx.search("café", 10)), vec!["nfd.md"], "NFC query, NFD text");
    assert_eq!(paths(idx.search("istanbul", 10)), vec!["tr.md"], "dotted capital I");
    assert_eq!(paths(idx.search("🎉", 10)), vec!["e.md"], "emoji");
}

/// The same in the parser: the tag regex does not stop at the combining accent
/// of NFD text, so "#café" typed in NFD is the same tag as the NFC "#café"
/// (not the truncated tag "cafe").
#[test]
fn finding_nfd_tags_not_truncated() {
    let p = cairn_core::parse::parse("#cafe\u{301} and #caf\u{e9}");
    assert_eq!(p.tags, vec!["café"]);
}

/// Frontmatter list items become tags verbatim, spaces included
/// (`tags: [my tag]` gives the tag "my tag"), and the tag search the UI runs
/// when that tag is clicked quotes the value (`tag:"my tag"`), so it finds
/// the note. Inline tags cannot contain spaces at all.
#[test]
fn finding_frontmatter_tag_with_space() {
    // b.md has the words but not the tag: a quoted phrase would find it.
    let idx = index(&[("a.md", "---\ntags: [my tag, other]\n---\nbody"), ("b.md", "my tag in the text")]);
    let tags: Vec<String> = idx.tags().into_iter().map(|t| t.tag).collect();
    assert!(tags.contains(&"my tag".to_string()), "{tags:?}");
    // A value with spaces is quoted (by design): LinksPanel.svelte and
    // TagsPanel.svelte search `tag:"my tag"` for it.
    assert_eq!(paths(idx.search("tag:\"my tag\"", 10)), vec!["a.md"], "clicking the tag finds its note");
}

/// Ranking sanity in a realistic vault: a long note titled "Garden" against a
/// short note that mentions the word once. BM25 favours short documents, so
/// the title boost is weighted like a body match and the titled note wins (a
/// flat +3 per query word let the passing mention win).
#[test]
fn finding_title_match_beats_passing_mention() {
    let mut files: Vec<(String, String)> = Vec::new();
    let mut r = Rng(42);
    let vocab = ["river", "stone", "harvest", "lamp", "cloud", "paper", "window", "music", "bread", "train", "letter", "mountain"];
    for i in 0..500 {
        let n = 50 + r.below(300);
        let words: Vec<&str> = (0..n).map(|_| vocab[r.below(vocab.len())]).collect();
        files.push((format!("n{i}.md"), words.join(" ")));
    }
    for i in 0..8 {
        files.push((format!("g{i}.md"), format!("{} garden {}", "river ".repeat(100), "stone ".repeat(100))));
    }
    // the note about gardens: long, uses the word a few times
    files.push(("Garden.md".into(), format!("# Garden\n{} garden beds {} garden tools {}", "soil compost seeds ".repeat(600), "water ".repeat(200), "lamp ".repeat(200))));
    // a short note with one passing mention
    files.push(("Shopping.md".into(), "milk eggs bread, gloves for the garden".into()));
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let idx = index(&refs);
    let hits = idx.search("garden", 5);
    let top: Vec<(String, f32)> = hits.iter().map(|h| (h.path.clone(), h.score)).collect();
    eprintln!("ranking: {top:?}");
    assert_eq!(top[0].0, "Garden.md", "ranking: {top:?}");
}
