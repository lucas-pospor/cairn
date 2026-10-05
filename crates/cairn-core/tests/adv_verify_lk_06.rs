// Reproduction for FINDING-181: an image wrapped in a Markdown link to a
// note. The link to the note must stay in the index. With a single pending
// `md_link` in parse.rs, Start(Image) would overwrite the outer Start(Link),
// End(Image) would consume it and End(Link) would find nothing, so the link
// to the note would be dropped.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_06 -- --include-ignored --nocapture

use std::fs;
use std::sync::Arc;

use cairn_core::parse;
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

/// Control: a link with plain text is indexed.
#[test]
fn control_text_link() {
    let (_d, v) = vault(&[("Note.md", "x"), ("src.md", "[badge](Note.md)")]);
    assert_eq!(v.backlinks("Note.md").len(), 1);
}

/// Observation: the side effect on skip ranges: with the outer link lost, a
/// wikilink in the same link text would be indexed (it is skipped for text
/// links).
#[test]
fn observe_parse() {
    let t = |s: &str| parse::parse(s).links.iter().map(|l| l.target.clone()).collect::<Vec<_>>();
    println!("{:?}", t("[![b](img.png)](Note.md)"));
    println!("{:?}", t("[![b](img.png) [[W]]](Note.md)"));
    println!("{:?}", t("[b [[W]]](Note.md)"));
}

#[test]
fn finding_image_link_backlinks() {
    let (_d, v) = vault(&[("Note.md", "x"), ("img.png", "png"), ("src.md", "[![badge](img.png)](Note.md)")]);
    assert_eq!(v.backlinks("Note.md").len(), 1, "backlinks of Note.md");
}
