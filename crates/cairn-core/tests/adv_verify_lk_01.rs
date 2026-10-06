// Regression tests for FINDING-035: wikilinks with `../` or `./` segments
// must resolve (outgoing, backlinks, graph, click, image embeds).
//
// Run: cargo test -p cairn-core --test adv_verify_lk_01 -- --include-ignored --nocapture
//
// Tests named `control_*` are controls; the others check the fix.

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn vault(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Vault) {
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
        ("Top.md", b"# Top"),
        ("a/Sibling.md", b"# Sibling"),
        ("a/sub/Child.md", b"# Child"),
        ("b/Target.md", b"# Target"),
        ("attachments/pic.png", b"\x89PNG fake"),
        (
            "a/src.md",
            b"[[../Top]]\n[[./Sibling]]\n[[../b/Target|alias]]\n![[../attachments/pic.png]]\n\
              [[sub/Child]]\n[md up](../Top.md)\n",
        ),
    ])
}

fn graph_edges(v: &Vault) -> Vec<(String, String)> {
    let g = v.graph(false);
    let mut e: Vec<(String, String)> =
        g.edges.iter().map(|(a, b)| (g.nodes[*a as usize].id.clone(), g.nodes[*b as usize].id.clone())).collect();
    e.sort();
    e
}

/// Controls: the descendant form of a relative wikilink and the Markdown link
/// with `..` both resolve, so the index itself can handle relative paths.
#[test]
fn control_descendant_wikilink_and_md_parent_link_resolve() {
    let (_d, v) = fixture();
    let out: Vec<(String, Option<String>)> = v.outgoing("a/src.md").into_iter().map(|o| (o.target, o.resolved)).collect();
    eprintln!("outgoing: {out:?}");
    assert!(out.contains(&("sub/Child".into(), Some("a/sub/Child.md".into()))));
    assert!(out.contains(&("../Top.md".into(), Some("Top.md".into()))));
    // Plain-name links work as well.
    assert_eq!(v.resolve("Top", "a/src.md").as_deref(), Some("Top.md"));
}

#[test]
fn dot_segment_wikilinks_resolve_in_outgoing() {
    let (_d, v) = fixture();
    let out: Vec<(String, Option<String>)> = v.outgoing("a/src.md").into_iter().map(|o| (o.target, o.resolved)).collect();
    eprintln!("outgoing: {out:?}");
    for (t, want) in [
        ("../Top", "Top.md"),
        ("./Sibling", "a/Sibling.md"),
        ("../b/Target", "b/Target.md"),
        ("../attachments/pic.png", "attachments/pic.png"),
    ] {
        let got = out.iter().find(|(x, _)| x == t).map(|(_, r)| r.clone());
        assert_eq!(got, Some(Some(want.to_string())), "outgoing for [[{t}]]: {out:?}");
    }
}

#[test]
fn dot_segment_wikilinks_show_in_backlinks() {
    let (_d, v) = fixture();
    let sib = v.backlinks("a/Sibling.md");
    let tgt = v.backlinks("b/Target.md");
    let top = v.backlinks("Top.md");
    eprintln!("backlinks Sibling: {sib:?}\nbacklinks Target: {tgt:?}\nbacklinks Top: {top:?}");
    assert_eq!(sib.len(), 1, "a/Sibling.md backlinks: {sib:?}");
    assert_eq!(tgt.len(), 1, "b/Target.md backlinks: {tgt:?}");
    // Top.md is linked by [[../Top]] and by [md up](../Top.md): two items.
    assert_eq!(top.first().map(|b| b.items.len()), Some(2), "Top.md backlinks: {top:?}");
}

#[test]
fn dot_segment_wikilinks_show_in_graph() {
    let (_d, v) = fixture();
    let e = graph_edges(&v);
    eprintln!("graph edges: {e:?}");
    assert!(e.contains(&("a/src.md".into(), "a/Sibling.md".into())), "edges {e:?}");
    assert!(e.contains(&("a/src.md".into(), "b/Target.md".into())), "edges {e:?}");
}

/// What a click does: commands::resolve_link -> Vault::resolve; on None the
/// app calls create_note("<target>.md"), which rejects dot segments.
#[test]
fn dot_segment_wikilink_click_path() {
    let (d, v) = fixture();
    let click = v.resolve("../Top", "a/src.md");
    let create = v.create_note("../Top.md", "").map(|_| ()).map_err(|e| e.to_string());
    eprintln!("Vault::resolve(\"../Top\", \"a/src.md\") = {click:?}; create_note fallback = {create:?}");
    // Safety: nothing was created outside or inside the vault by the fallback.
    assert!(!d.path().join("Top.md").exists(), "fallback escaped the vault");
    assert_eq!(click.as_deref(), Some("Top.md"));
}
