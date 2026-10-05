// Reproduction for FINDING-192 (flat substring title boost).
//
//   cargo test -p cairn-core --test adv_verify_lk_23 -- --ignored --nocapture

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

/// A boost that used `title.contains(word)` would give the query "note" the
/// same +3 for "Footnotes.md" as for "Note.md", although "footnotes" does not
/// even start with "note" (body matching is prefix-only).
#[test]
fn verify_title_boost_substring() {
    let body = "a note about things";
    let idx = index(&[("Footnotes.md", body), ("Other.md", body)]);
    let hits: Vec<(String, f32)> = idx.search("note", 10).into_iter().map(|h| (h.path, h.score)).collect();
    eprintln!("hits: {hits:?}");
    assert!((hits[0].1 - hits[1].1).abs() < 0.01, "a non-prefix title substring should not be boosted: {hits:?}");
}

/// How much longer than average the titled note must be before a one-word
/// passing mention outranks it: small vault, varying the length of Garden.md.
#[test]
fn verify_title_vs_short_note_threshold() {
    let mut files: Vec<(String, String)> = Vec::new();
    for i in 0..200 {
        files.push((format!("n{i}.md"), "river stone lamp ".repeat(70))); // ~210 words each
    }
    for i in 0..8 {
        files.push((format!("g{i}.md"), format!("{} garden", "river ".repeat(200))));
    }
    files.push(("Shopping.md".into(), "milk eggs bread, gloves for the garden".into()));
    for words in [200usize, 600, 1200, 2400, 4800] {
        let mut f = files.clone();
        f.push(("Garden.md".into(), format!("# Garden\n{} garden beds garden tools", "soil ".repeat(words))));
        let refs: Vec<(&str, &str)> = f.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let idx = index(&refs);
        let top: Vec<(String, f32)> = idx.search("garden", 3).into_iter().map(|h| (h.path, h.score)).collect();
        eprintln!("Garden.md ~{words} words: {top:?}");
    }
}
