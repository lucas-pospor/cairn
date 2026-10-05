// Reproduction for FINDING-178: links to `.markdown` notes.
//
// path.rs link_key_for_file keys `Doc.markdown` by its stem ("doc"), so
// link_key_for_target must strip ".markdown" as well as ".md". Otherwise the
// link text "Doc.markdown" is keyed "doc.markdown", Index::resolve looks
// candidates up by that key (miss), and backlinks() looks link_sources up by
// the file key (miss).
//
// Run: cargo test -p cairn-core --test adv_verify_lk_03 -- --include-ignored --nocapture

use std::fs;
use std::sync::Arc;

use cairn_core::path as vpath;
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

/// Control: `.markdown` is a note everywhere else, and [[Doc]] (no extension)
/// resolves and shows in backlinks.
#[test]
fn control_stem_wikilink_works() {
    assert!(vpath::is_markdown("Doc.markdown"));
    let (_d, v) = vault(&[("Doc.markdown", "# Doc"), ("src.md", "[[Doc]]")]);
    assert_eq!(v.resolve("Doc", "src.md").as_deref(), Some("Doc.markdown"));
    let n: usize = v.backlinks("Doc.markdown").iter().map(|b| b.items.len()).sum();
    assert_eq!(n, 1);
}

/// Control: the same links to a `.md` note are fine.
#[test]
fn control_dot_md_full_name_works() {
    let (_d, v) = vault(&[("Doc.md", "# Doc"), ("src.md", "[d](Doc.md) [[Doc.md]]")]);
    let n: usize = v.backlinks("Doc.md").iter().map(|b| b.items.len()).sum();
    assert_eq!(n, 2);
}

/// Observation: the file and the link text get the same key ("doc"). With
/// different keys ("doc" for the file, "doc.markdown" for the link text) the
/// Markdown link would resolve in outgoing and in the graph, but backlinks
/// would be empty.
#[test]
fn observe_keys_and_graph() {
    assert_eq!(vpath::link_key_for_file("Doc.markdown"), "doc");
    assert_eq!(vpath::link_key_for_target("Doc.markdown"), "doc");
    let (_d, v) = vault(&[("Doc.markdown", "# Doc"), ("src.md", "[d](Doc.markdown) [[Doc.markdown]]")]);
    let out: Vec<_> = v.outgoing("src.md").into_iter().map(|o| (o.target, o.resolved)).collect();
    println!("outgoing: {out:?}");
    println!("backlinks: {:?}", v.backlinks("Doc.markdown"));
    let g = v.graph(false);
    println!("graph nodes {:?} edges {:?}", g.nodes.iter().map(|n| &n.id).collect::<Vec<_>>(), g.edges);
    assert_eq!(out[0].1.as_deref(), Some("Doc.markdown"));
    assert_eq!(out[1].1.as_deref(), Some("Doc.markdown"));
    assert_eq!(g.edges.len(), 1, "graph has one edge for both links");
    assert!(!v.backlinks("Doc.markdown").is_empty());
}

#[test]
fn finding_dot_markdown_full_name() {
    let (_d, v) = vault(&[("Doc.markdown", "# Doc"), ("src.md", "[d](Doc.markdown) [[Doc.markdown]]")]);
    assert_eq!(v.resolve("Doc.markdown", "src.md").as_deref(), Some("Doc.markdown"));
    let n: usize = v.backlinks("Doc.markdown").iter().map(|b| b.items.len()).sum();
    assert_eq!(n, 2);
}
