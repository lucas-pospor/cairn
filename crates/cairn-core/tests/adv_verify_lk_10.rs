// Reproduction for FINDING-184 (core side): the text the autocomplete
// inserts (links.ts LinkIndex.linkText) must resolve back to the picked file
// in Index::resolve.
//
// linkText tries the name, then the path, each without and then with the
// extension, and takes the first text that [[text]] reads back as the picked
// file; when none does, the autocomplete leaves the file out. Taking the stem
// (or full name for attachments) when the key is unique, otherwise the full
// path without .md/.markdown, is not enough. These tests check the core side.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_10 -- --include-ignored --nocapture

use cairn_core::fs::{EntryKind, FileStat};
use cairn_core::index::{hash_bytes, Index};
use cairn_core::parse;
use cairn_core::path as vpath;

fn index(files: &[&str]) -> Index {
    let mut idx = Index::default();
    for p in files {
        let st = FileStat { path: (*p).into(), kind: EntryKind::File, size: 0, mtime: 0 };
        if vpath::is_markdown(p) {
            idx.put_note(st, String::new(), hash_bytes(b""));
        } else {
            idx.put_entry(st, None);
        }
    }
    idx
}

/// Resolve the text as a wikilink would be parsed: [[text]].
fn via_wikilink(idx: &Index, text: &str) -> Option<String> {
    let (t, _, _) = parse::split_wikilink_inner(text);
    idx.resolve(&t, "src.md")
}

/// Control: ordinary names round-trip.
#[test]
fn control_ordinary_names() {
    let idx = index(&["a/Note.md", "b/Note.md", "Other.md", "img/pic.png"]);
    assert_eq!(via_wikilink(&idx, "a/Note").as_deref(), Some("a/Note.md"));
    assert_eq!(via_wikilink(&idx, "Other").as_deref(), Some("Other.md"));
    assert_eq!(via_wikilink(&idx, "pic.png").as_deref(), Some("img/pic.png"));
}

/// x.md.md: [[x.md]] resolves to nothing, so linkText gives [[x.md.md]].
#[test]
fn finding_double_extension() {
    let idx = index(&["x.md.md"]);
    assert_eq!(via_wikilink(&idx, "x.md").as_deref(), None);
    assert_eq!(via_wikilink(&idx, "x.md.md").as_deref(), Some("x.md.md"));
}

/// Case-variant duplicates a/Note.md + a/note.md: resolution is
/// case-insensitive, so every text reaches a/Note.md. linkText gives
/// [[a/Note]] for that one and no text for a/note.md.
#[test]
fn finding_case_variants() {
    let idx = index(&["a/Note.md", "a/note.md"]);
    for t in ["note", "note.md", "a/note", "a/note.md", "a/Note"] {
        assert_eq!(via_wikilink(&idx, t).as_deref(), Some("a/Note.md"), "[[{t}]]");
    }
}

/// A note named 'C# notes.md' (created outside Cairn): [[C# notes]] is
/// target C with heading "notes", so linkText gives no text for it.
#[test]
fn finding_metacharacters() {
    let idx = index(&["C# notes.md", "C.md"]);
    assert_eq!(via_wikilink(&idx, "C# notes").as_deref(), Some("C.md"));
    assert_eq!(via_wikilink(&idx, "C# notes.md").as_deref(), Some("C.md"));
}
