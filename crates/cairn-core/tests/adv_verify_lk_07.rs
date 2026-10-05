// Reproduction for FINDING-182: `\[[x]]` (CommonMark escape) must not be a
// wikilink for the core (WIKILINK_RE runs over raw text, parse.rs).
//
// Run: cargo test -p cairn-core --test adv_verify_lk_07 -- --include-ignored --nocapture

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

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

/// Control: a code span is the documented way to show literal brackets.
#[test]
fn control_code_span_is_not_a_link() {
    let (_d, v) = vault(&[("Not a link.md", "x"), ("src.md", "Write `[[Not a link]]` to show brackets.")]);
    assert!(v.backlinks("Not a link.md").is_empty());
}

#[test]
fn finding_escaped_wikilink_backlink() {
    let (_d, v) = vault(&[("Not a link.md", "x"), ("src.md", "Write \\[[Not a link]] to show brackets.")]);
    assert!(v.backlinks("Not a link.md").is_empty(), "{:?}", v.backlinks("Not a link.md"));
}
