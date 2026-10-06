// Regression tests for FINDING-088: Chinese/Japanese words in the middle of a
// sentence must be found by full-text search, through the same Vault::search
// path the app's search panel uses (commands.rs `search`).
//
// Run: cargo test -p cairn-core --test adv_verify_lk_17 -- --include-ignored --nocapture
//
// Tests named `control_*` are controls; the other one checks the fix.

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    for (p, c) in files {
        let abs = root.join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(&root, TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

fn fixture() -> (tempfile::TempDir, Vault) {
    vault(&[
        ("ja.md", "今日は東京で会議があります。"),
        ("zh.md", "我们明天去北京开会。"),
        ("en.md", "Meeting in Tokyo today."),
    ])
}

fn hits(v: &Vault, q: &str) -> Vec<String> {
    v.search(q, 10).into_iter().map(|h| h.path).collect()
}

/// Controls: Latin text works, and the first characters of a CJK run are
/// found (as they were when a whole CJK run was one token).
#[test]
fn control_latin_and_cjk_prefix() {
    let (_d, v) = fixture();
    assert_eq!(hits(&v, "tokyo"), vec!["en.md"]);
    assert_eq!(hits(&v, "今日"), vec!["ja.md"]);
    assert_eq!(hits(&v, "我们"), vec!["zh.md"]);
    // The whole clause is one token, so even this is found.
    assert_eq!(hits(&v, "今日は東京で会議があります"), vec!["ja.md"]);
}

/// Words in the middle of a clause are found, plain or quoted.
#[test]
fn cjk_mid_sentence_words() {
    let (_d, v) = fixture();
    let mut failures = Vec::new();
    for (q, want) in [
        ("東京", "ja.md"),
        ("会議", "ja.md"),
        ("\"東京\"", "ja.md"),
        ("北京", "zh.md"),
        ("开会", "zh.md"),
        ("\"北京\"", "zh.md"),
    ] {
        let got = hits(&v, q);
        println!("{q:>8} -> {got:?}");
        if got != vec![want.to_string()] {
            failures.push(format!("{q} -> {got:?} (want [{want}])"));
        }
    }
    assert!(failures.is_empty(), "not found: {failures:#?}");
}
