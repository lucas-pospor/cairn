// Reproduction for FINDING-185 (core side): a target with a leading
// backslash must resolve like its forward-slash twin. Index::resolve has to
// turn '\' into '/' before it trims a leading '/'.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_11 -- --include-ignored --nocapture

use cairn_core::fs::{EntryKind, FileStat};
use cairn_core::index::{hash_bytes, Index};

fn index(files: &[&str]) -> Index {
    let mut idx = Index::default();
    for p in files {
        let st = FileStat { path: (*p).into(), kind: EntryKind::File, size: 0, mtime: 0 };
        idx.put_note(st, String::new(), hash_bytes(b""));
    }
    idx
}

/// Control: inner backslashes are accepted by the core, and a leading '/'.
#[test]
fn control_inner_backslash_and_leading_slash() {
    let idx = index(&["sub/Note.md"]);
    assert_eq!(idx.resolve("sub\\Note", "x.md").as_deref(), Some("sub/Note.md"));
    assert_eq!(idx.resolve("/sub/Note", "x.md").as_deref(), Some("sub/Note.md"));
}

#[test]
fn finding_leading_backslash() {
    let idx = index(&["sub/Note.md"]);
    assert_eq!(idx.resolve("\\sub\\Note", "x.md").as_deref(), Some("sub/Note.md"));
}
