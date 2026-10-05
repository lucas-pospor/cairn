// Reproduction for FINDING-180 (core side): Index::resolve and links.ts
// LinkIndex.resolve must break ties the same way, or the editor's image
// embeds could pick a different file than the core (click, backlinks). UTF-8
// byte length and byte order on one side and c.length (UTF-16 units) and `<`
// (UTF-16 order) on the other disagree. PLAN says "shortest path, then
// alphabetical": both count characters and compare code points.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_05 -- --include-ignored --nocapture

use cairn_core::fs::{EntryKind, FileStat};
use cairn_core::index::Index;

fn index(files: &[&str]) -> Index {
    let mut idx = Index::default();
    for p in files {
        idx.put_entry(FileStat { path: (*p).into(), kind: EntryKind::File, size: 0, mtime: 0 }, None);
    }
    idx
}

/// The core's choices, which the TS side (adv_verify_lk_05.test.ts) must
/// match: the shortest path in characters, then code point order.
#[test]
fn observe_core_tie_breaks() {
    // 14 bytes vs 12 bytes; 11 vs 12 characters.
    let idx = index(&["ééé/pic.png", "abcd/pic.png"]);
    assert_eq!(idx.resolve("pic.png", "x.md").as_deref(), Some("ééé/pic.png"));
    // 11 characters (14 UTF-16 units) vs 12.
    let idx = index(&["😀😀😀/pic.png", "abcd/pic.png"]);
    assert_eq!(idx.resolve("pic.png", "x.md").as_deref(), Some("😀😀😀/pic.png"));
    // 10 characters each; code point order puts U+FF71 before U+1F600
    // (UTF-16 order would not: 0xD83D < 0xFF71).
    let idx = index(&["😀ｱ/pic.png", "ｱ😀/pic.png"]);
    assert_eq!(idx.resolve("pic.png", "x.md").as_deref(), Some("ｱ😀/pic.png"));
}
