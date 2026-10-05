// Reproduction for FINDING-089 (tag regex and tokenizer vs combining
// marks). Not only NFD text is affected: scripts whose ordinary NFC spelling
// uses combining vowel signs (Devanagari, Thai, Bengali, ...) would lose
// everything after the first consonant in a tag if TAG_RE in parse.rs (and
// the matching regexes in markdown.ts and livePreview.ts) had no \p{M}.
//
//   cargo test -p cairn-core --test adv_verify_lk_19 -- --ignored --nocapture

use cairn_core::fs::{EntryKind, FileStat};
use cairn_core::index::{hash_bytes, Index};

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

/// "#हिंदी" (Hindi, NFC) must not be truncated to the tag "ह", nor "#ที่นี่" (Thai) to "ท".
#[test]
fn verify_indic_thai_tags_truncated() {
    let p = cairn_core::parse::parse("notes #हिंदी and #ที่นี่ and #বাংলা");
    eprintln!("tags: {:?}", p.tags);
    assert_eq!(p.tags, vec!["हिंदी", "ที่นี่", "বাংলা"]);
}

/// Control: full-text search does find these words (vowel signs are
/// Alphabetic, so the tokenizer keeps them), so only tags are at risk for
/// NFC text in these scripts.
#[test]
fn verify_indic_search_control() {
    let idx = index(&[("hi.md", "मेरी हिंदी नोट्स"), ("th.md", "ภาษาไทย ที่นี่")]);
    let hi: Vec<String> = idx.search("हिंदी", 10).into_iter().map(|h| h.path).collect();
    let th: Vec<String> = idx.search("ที่นี่", 10).into_iter().map(|h| h.path).collect();
    eprintln!("hi: {hi:?} th: {th:?}");
    assert_eq!(hi, vec!["hi.md"]);
    assert_eq!(th, vec!["th.md"]);
}

/// Truncated tags of different words would collide: "#हिंदी" and "#हाथ" would
/// both become the tag "ह", so `tag:ह` would list unrelated notes and the
/// tags panel would show a single one-letter tag.
#[test]
fn verify_indic_tags_collide() {
    let idx = index(&[("a.md", "#हिंदी"), ("b.md", "#हाथ")]);
    let tags: Vec<String> = idx.tags().into_iter().map(|t| t.tag).collect();
    eprintln!("tags: {tags:?}");
    assert_eq!(tags.len(), 2, "{tags:?}");
}
