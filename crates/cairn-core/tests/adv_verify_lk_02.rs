// Regression test for FINDING-036.
//
// The app follows every clicked link (reading view, Live Preview, Ctrl+click in
// source, and the outgoing-links panel) through the `resolve_link` command.
// The index (outgoing, backlinks, graph) resolves Markdown links with
// `Index::resolve_link`, which resolves them relative to the source note
// first. The command gets the link kind and calls `Vault::resolve_target`, so
// a click on a Markdown link follows the same rules, not the name-based
// wikilink rules of `Vault::resolve`.
//
// This test uses `../` and `./` Markdown links to uniquely named notes,
// because a mismatch did not need duplicate note names: the index resolves
// them, but `Vault::resolve` used to return None for them, so a click resolved
// that way fell through to "create note" and failed with "Invalid path".
//
// Run: cargo test -p cairn-core --test adv_verify_lk_02

use std::fs;
use std::sync::Arc;

use cairn_core::parse::LinkKind;
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

#[test]
fn relative_markdown_links_to_unique_notes_can_be_followed() {
    let (_d, v) = vault(&[
        ("Root.md", "# Root"),
        ("docs/Sibling.md", "# Sibling"),
        ("docs/guide/Deep.md", "# Deep"),
        (
            "docs/guide/src.md",
            "[up two](../../Root.md) [up one](../Sibling.md) [dot](./Deep.md) [plain](Deep.md)",
        ),
    ]);
    let index_view: Vec<(String, Option<String>)> =
        v.outgoing("docs/guide/src.md").into_iter().map(|o| (o.target, o.resolved)).collect();
    // The index resolves all four.
    assert_eq!(
        index_view.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
        vec![
            Some("Root.md".to_string()),
            Some("docs/Sibling.md".to_string()),
            Some("docs/guide/Deep.md".to_string()),
            Some("docs/guide/Deep.md".to_string()),
        ],
        "index view"
    );
    // What a click does: commands::resolve_link -> Vault::resolve_target.
    let click_view: Vec<(String, Option<String>)> = index_view
        .iter()
        .map(|(t, _)| (t.clone(), v.resolve_target(t, LinkKind::Markdown, "docs/guide/src.md")))
        .collect();
    let mismatches: Vec<_> = index_view
        .iter()
        .zip(&click_view)
        .filter(|(a, b)| a.1 != b.1)
        .map(|(a, b)| format!("{}: index {:?}, click {:?}", a.0, a.1, b.1))
        .collect();
    assert!(mismatches.is_empty(), "click resolution differs from the index:\n{}", mismatches.join("\n"));
}
