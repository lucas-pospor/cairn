// Regression tests for FINDING-087: with a query word that expands to at most
// 256 dictionary terms taken in alphabetical order, notes whose matching word
// sorts later would be silently missing.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_15 -- --include-ignored --nocapture
//
// Scenario: a Zettelkasten-style vault whose notes are named with timestamp
// IDs ("202401010900 Title.md"). The ID is indexed as one token, so a vault
// with a few hundred notes from one year has hundreds of distinct terms
// starting with "2024". Goes through the real Vault on a temp folder and the
// same limit (100) the search panel passes.
//
// Tests named `control_*` are controls; the others check the fix.

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn zettel_vault(n: usize) -> (tempfile::TempDir, Vault, String) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    // Spread n notes over the year: one per day-ish, IDs increase with time.
    let mut last = String::new();
    for i in 0..n {
        let month = 1 + (i * 12 / n);
        let day = 1 + (i % 28);
        let id = format!("2024{month:02}{day:02}{:04}", 900 + i);
        let name = format!("{id} Log.md");
        let body = if i == n - 1 { "Trip notes: saw a kiwi in the park.\n".to_string() } else { "Daily log entry.\n".to_string() };
        fs::write(root.join(&name), body).unwrap();
        last = name;
    }
    let v = Vault::open(Arc::new(StdFs::new(&root, TrashMode::Vault).unwrap())).unwrap();
    (d, v, last)
}

#[test]
fn control_small_vault_finds_late_note() {
    let (_d, v, last) = zettel_vault(200);
    let hits: Vec<String> = v.search("2024 kiwi", 100).into_iter().map(|h| h.path).collect();
    assert_eq!(hits, vec![last]);
    assert_eq!(v.search("2024", 1000).len(), 200);
}

#[test]
fn control_rare_word_alone_finds_note() {
    let (_d, v, last) = zettel_vault(400);
    let hits: Vec<String> = v.search("kiwi", 100).into_iter().map(|h| h.path).collect();
    assert_eq!(hits, vec![last]);
}

#[test]
fn finding_and_query_finds_note_past_expansion_cap() {
    let (_d, v, last) = zettel_vault(400);
    let hits: Vec<String> = v.search("2024 kiwi", 100).into_iter().map(|h| h.path).collect();
    eprintln!("'2024 kiwi' -> {hits:?} (expected [{last:?}])");
    assert_eq!(hits, vec![last], "the only 2024 note mentioning kiwi must be found");
}

#[test]
fn finding_prefix_query_keeps_later_months() {
    let (_d, v, _last) = zettel_vault(400);
    let all = v.search("2024", 1000);
    let december = all.iter().filter(|h| h.path.starts_with("202412")).count();
    eprintln!("'2024' -> {} of 400 notes, {} from December", all.len(), december);
    assert_eq!(all.len(), 400, "every note whose ID starts with 2024 must match '2024'");
}
