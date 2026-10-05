// Adversarial tests for link parsing and resolution.
//
// Run: cargo test -p cairn-core --test adv_links
//
// Tests named finding_* are regression tests for bugs that have been fixed.
// Tests named held_* are scenarios that held up.

use std::fs;
use std::sync::Arc;

use cairn_core::fs::{EntryKind, FileStat};
use cairn_core::index::{hash_bytes, Index};
use cairn_core::parse::{self, LinkKind};
use cairn_core::path as vpath;
use cairn_core::{StdFs, TrashMode, Vault};

fn st(path: &str) -> FileStat {
    FileStat { path: path.into(), kind: EntryKind::File, size: 0, mtime: 0 }
}

/// Index with the given files; Markdown files get the given content.
fn index(files: &[(&str, &str)]) -> Index {
    let mut idx = Index::default();
    for (p, c) in files {
        if vpath::is_markdown(p) {
            idx.put_note(st(p), c.to_string(), hash_bytes(c.as_bytes()));
        } else {
            idx.put_entry(st(p), None);
        }
    }
    idx
}

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

fn resolved_outgoing(v: &Vault, path: &str) -> Vec<(String, Option<String>)> {
    v.outgoing(path).into_iter().map(|o| (o.target, o.resolved)).collect()
}

fn targets(p: &parse::ParsedNote) -> Vec<String> {
    p.links.iter().map(|l| l.target.clone()).collect()
}

// ---------------------------------------------------------------------------
// Resolution rules that held up
// ---------------------------------------------------------------------------

#[test]
fn held_rules_exact_relative_basename_order() {
    let idx = index(&[
        ("Note.md", ""),
        ("a/Note.md", ""),
        ("a/b/Note.md", ""),
        ("z/Other.md", ""),
        ("y/Other.md", ""),
        ("q/sub/Leaf.md", ""),
        ("r/sub/Leaf.md", ""),
        ("img/Pic.PNG", ""),
    ]);
    // exact vault path wins over same-folder basename
    assert_eq!(idx.resolve("Note", "a/b/x.md").as_deref(), Some("Note.md"));
    assert_eq!(idx.resolve("a/Note", "a/b/x.md").as_deref(), Some("a/Note.md"));
    // relative to the source folder (descendant form)
    assert_eq!(idx.resolve("b/Note", "a/x.md").as_deref(), Some("a/b/Note.md"));
    // same length: alphabetical
    assert_eq!(idx.resolve("Other", "Note.md").as_deref(), Some("y/Other.md"));
    // partial path, alphabetical between equal lengths
    assert_eq!(idx.resolve("sub/Leaf", "Note.md").as_deref(), Some("q/sub/Leaf.md"));
    // relative beats alphabetical; same folder beats alphabetical
    assert_eq!(idx.resolve("sub/Leaf", "r/x.md").as_deref(), Some("r/sub/Leaf.md"));
    assert_eq!(idx.resolve("sub/Leaf", "r/sub/x.md").as_deref(), Some("r/sub/Leaf.md"));
    // case-insensitive everywhere, attachments by full name, leading slash
    assert_eq!(idx.resolve("NOTE.MD", "z/q.md").as_deref(), Some("Note.md"));
    assert_eq!(idx.resolve("pic.png", "z/q.md").as_deref(), Some("img/Pic.PNG"));
    assert_eq!(idx.resolve("/a/note", "z/q.md").as_deref(), Some("a/Note.md"));
    assert_eq!(idx.resolve("  Note  ", "z/q.md").as_deref(), Some("Note.md"));
    // a .png target never resolves to a note with the same stem
    assert_eq!(idx.resolve("Note.png", "z/q.md"), None);
}

#[test]
fn held_markdown_links_resolve_relative_first() {
    let (_d, v) = vault(&[
        ("Note.md", ""),
        ("a/Note.md", ""),
        ("a/src.md", "[rel](Note.md) [up](../Note.md) [dot](./Note.md) [enc](My%20Note.md) [ang](<My Note.md>) [t](Note.md \"title\")"),
        ("a/My Note.md", ""),
    ]);
    let out = resolved_outgoing(&v, "a/src.md");
    assert_eq!(
        out,
        vec![
            ("Note.md".into(), Some("a/Note.md".into())),
            ("../Note.md".into(), Some("Note.md".into())),
            ("./Note.md".into(), Some("a/Note.md".into())),
            ("My Note.md".into(), Some("a/My Note.md".into())),
            ("My Note.md".into(), Some("a/My Note.md".into())),
            ("Note.md".into(), Some("a/Note.md".into())),
        ]
    );
}

#[test]
fn held_heading_and_block_subpaths_and_aliases_split() {
    let p = parse::parse("[[A#H1|al]] [[B#^blk]] [[#Self]] [[C\\|x]] [[ D # E | F ]] ![[pic.png|200]]");
    let parts: Vec<_> = p
        .links
        .iter()
        .map(|l| (l.target.as_str(), l.subpath.as_deref(), l.display.as_deref(), l.embed))
        .collect();
    assert_eq!(
        parts,
        vec![
            ("A", Some("H1"), Some("al"), false),
            ("B", Some("^blk"), None, false),
            ("", Some("Self"), None, false),
            ("C", None, Some("x"), false),
            ("D", Some("E"), Some("F"), false),
            ("pic.png", None, Some("200"), true),
        ]
    );
}

#[test]
fn held_links_in_code_html_and_frontmatter_are_ignored() {
    let src = "---\nrel: \"[[FM]]\"\n---\n`[[Span]]`\n\n```\n[[Fence]]\n```\n\n~~~md\n[[Tilde]]\n~~~\n\n    [[Indented]]\n\n<div>\n[[Html]]\n</div>\n\n[[Real]]\n";
    let p = parse::parse(src);
    assert_eq!(targets(&p), vec!["Real"]);
}

#[test]
fn held_unresolved_link_becomes_resolved_after_create_and_back_after_delete() {
    let (_d, v) = vault(&[("a.md", "[[Later]] [[sub/Later]]")]);
    assert!(v.outgoing("a.md").iter().all(|o| o.resolved.is_none()));
    v.create_note("sub/Later.md", "").unwrap();
    assert!(v.outgoing("a.md").iter().all(|o| o.resolved.as_deref() == Some("sub/Later.md")));
    assert_eq!(v.backlinks("sub/Later.md")[0].items.len(), 2);
    v.delete("sub/Later.md").unwrap();
    assert!(v.outgoing("a.md").iter().all(|o| o.resolved.is_none()));
    assert!(v.backlinks("sub/Later.md").is_empty());
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

/// PLAN: "then a path relative to the linking note's folder". Obsidian writes
/// `[[../Folder/Note]]` when "New link format" is "Relative path to file".
/// Cairn only handles the descendant form (`[[sub/Note]]`); any `..` or `.`
/// segment makes the link unresolved (and clicking it fails to create the note
/// because `..`/`.` are rejected by path::normalize).
#[test]
fn finding_relative_wikilinks_with_dot_segments() {
    let (_d, v) = vault(&[
        ("Top.md", ""),
        ("a/Sibling.md", ""),
        ("b/Target.md", ""),
        ("a/src.md", "[[../Top]] [[./Sibling]] [[../b/Target]] [[../b/Target.md#H]]"),
    ]);
    let out = resolved_outgoing(&v, "a/src.md");
    assert_eq!(
        out,
        vec![
            ("../Top".into(), Some("Top.md".into())),
            ("./Sibling".into(), Some("a/Sibling.md".into())),
            ("../b/Target".into(), Some("b/Target.md".into())),
            ("../b/Target.md".into(), Some("b/Target.md".into())),
        ],
        "relative wikilinks"
    );
}

/// The UI opens every clicked link (wikilink or Markdown link) through the
/// `resolve_link` command, while backlinks/outgoing/graph use
/// `Index::resolve_link` (relative-first for Markdown links). If the command
/// used `Vault::resolve` (name-based rules), the same Markdown link would open
/// a different note than the one whose backlinks list it (FINDING-036). The
/// command gets the link kind and calls `Vault::resolve_target`.
#[test]
fn finding_markdown_link_click_differs_from_index() {
    let (_d, v) = vault(&[
        ("Note.md", "root note"),
        ("a/Note.md", "folder note"),
        ("a/src.md", "[same folder](Note.md) [up](../Note.md) [dot](./Note.md)"),
    ]);
    let idx_view: Vec<Option<String>> = v.outgoing("a/src.md").into_iter().map(|o| o.resolved).collect();
    assert_eq!(idx_view, vec![Some("a/Note.md".into()), Some("Note.md".into()), Some("a/Note.md".into())]);
    assert_eq!(v.backlinks("a/Note.md")[0].source, "a/src.md");
    // What the app does on click: app.openLink(target, .., "markdown") ->
    // backend.resolveLink -> commands::resolve_link -> Vault::resolve_target.
    let click_view: Vec<Option<String>> = ["Note.md", "../Note.md", "./Note.md"]
        .iter()
        .map(|t| v.resolve_target(t, LinkKind::Markdown, "a/src.md"))
        .collect();
    assert_eq!(click_view, idx_view, "click resolution must match the index (backlinks/outgoing)");
}

/// Markdown links to a `.markdown` note resolve (outgoing shows them), but the
/// backlinks lookup files them under the key "note.markdown" while the note's
/// key is "note", so the note's backlinks panel never lists them. Wikilinks
/// with the full name `[[Note.markdown]]` do not resolve at all.
#[test]
fn finding_dot_markdown_backlinks() {
    let (_d, v) = vault(&[("Doc.markdown", "# Doc"), ("src.md", "[d](Doc.markdown) [[Doc.markdown]]")]);
    let out = resolved_outgoing(&v, "src.md");
    assert_eq!(out[0].1.as_deref(), Some("Doc.markdown"), "md link resolves");
    let bl = v.backlinks("Doc.markdown");
    assert_eq!(bl.len(), 1, "backlinks of Doc.markdown: {bl:?}");
    assert_eq!(bl[0].items.len(), 2, "both links listed: {bl:?}");
    assert_eq!(out[1].1.as_deref(), Some("Doc.markdown"), "wikilink with full name resolves");
}

/// Note identity is NFC (PLAN 2.2) and paths from disk are NFC-normalized,
/// but link text is not normalized. A link typed or pasted in NFD (macOS
/// file names, some keyboards) never matches the NFC key of the note.
#[test]
fn finding_nfd_link_target() {
    let nfc_name = "Caf\u{e9}.md";
    let (_d, v) = vault(&[(nfc_name, "# Café"), ("src.md", "[[Cafe\u{301}]] [m](Cafe\u{301}.md)")]);
    let out = resolved_outgoing(&v, "src.md");
    let bl: usize = v.backlinks(nfc_name).iter().map(|b| b.items.len()).sum();
    let mut problems = Vec::new();
    if out[0].1.as_deref() != Some(nfc_name) {
        problems.push(format!("NFD wikilink unresolved: {:?}", out[0]));
    }
    if out[1].1.as_deref() != Some(nfc_name) {
        problems.push(format!("NFD md link unresolved: {:?}", out[1]));
    }
    if bl != 2 {
        problems.push(format!("backlinks of {nfc_name} list {bl} of 2 links (the md link resolves in outgoing but is filed under an NFD key)"));
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// "Shortest path" is measured in UTF-8 bytes in Rust, so a non-ASCII folder
/// counts double. The TS LinkIndex (UTF-16 units) picks the other file, so
/// previews and the core disagree (see the TS fixture test).
#[test]
fn finding_shortest_path_counts_bytes() {
    let idx = index(&[("ééé/Note.md", ""), ("abcd/Note.md", "")]);
    // "ééé/Note.md" has 11 characters, "abcd/Note.md" 12.
    assert_eq!(idx.resolve("Note", "x.md").as_deref(), Some("ééé/Note.md"));
}

/// `[![badge](img.png)](Note.md)`, an image that links to a note, is a common
/// pattern. The parser keeps only one pending Markdown link, so the inner
/// image overwrites the outer link and the link to Note.md is lost.
#[test]
fn finding_image_inside_link_drops_outer_link() {
    let p = parse::parse("[![badge](img.png)](Note.md)");
    let mut t = targets(&p);
    t.sort();
    assert_eq!(t, vec!["Note.md", "img.png"]);
}

/// A backslash-escaped wikilink is literal text for CommonMark and for the
/// preview (markdown-it), but the core's regex runs on raw text and still
/// counts it as a link (backlinks, graph, outgoing).
#[test]
fn finding_escaped_wikilink_counted() {
    let p = parse::parse("Write \\[[Not a link]] to show brackets.");
    assert!(p.links.is_empty(), "{:?}", p.links);
}

/// The closing `---` of frontmatter followed by trailing spaces is accepted by
/// the UI (markdown.ts and the Live Preview properties box allow `[ \t]*`), so
/// the core must accept it too. Otherwise it parses the YAML as body:
/// properties and frontmatter tags vanish, `key: value\n---` becomes a
/// heading, and links inside the YAML are indexed.
#[test]
fn finding_frontmatter_trailing_space_close() {
    let p = parse::parse("---\ntitle: Hello\ntags: [alpha]\nrelated: \"[[Other]]\"\n---  \n# Real\n");
    let h: Vec<_> = p.headings.iter().map(|h| h.text.as_str()).collect();
    let l: Vec<_> = p.links.iter().map(|l| l.target.as_str()).collect();
    assert!(p.frontmatter.is_some(), "frontmatter not recognized; tags={:?} headings={h:?} links={l:?}", p.tags);
    assert_eq!(p.tags, vec!["alpha"]);
    let h: Vec<_> = p.headings.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(h, vec!["Real"]);
}

/// The skip-range lookup uses partition_point on ranges that can nest (a
/// Markdown link range contains its code spans and inline HTML). For a
/// position after a nested range the binary search answers "not inside", so a
/// wikilink inside Markdown link text is indexed only when an inline code span
/// (or inline HTML) precedes it. (Tags are protected by a second check.)
#[test]
fn finding_nested_skip_ranges() {
    let plain = parse::parse("[see [[Wiki]]](x.md)");
    assert_eq!(targets(&plain), vec!["x.md"], "control: wikilinks in link text are skipped");
    let nested = parse::parse("[`code` see [[Wiki]]](x.md)");
    assert_eq!(targets(&nested), vec!["x.md"], "after a code span");
    let html = parse::parse("[<b>bold</b> [[Wiki]]](x.md)");
    assert_eq!(targets(&html), vec!["x.md"], "after inline HTML");
}

#[test]
fn held_markdown_link_kinds() {
    let p = parse::parse("[a](x.md) ![b](y.png) [[w]] ![[e]]");
    let k: Vec<_> = p.links.iter().map(|l| (l.kind, l.embed)).collect();
    assert_eq!(
        k,
        vec![(LinkKind::Markdown, false), (LinkKind::Markdown, true), (LinkKind::Wiki, false), (LinkKind::Wiki, true)]
    );
}

/// Unresolved graph nodes are keyed by basename only, so different missing
/// notes in different folders ("Projects/Todo" and "Home/Todo") become one
/// ghost node labelled with whichever link was seen first.
#[test]
fn finding_graph_unresolved_nodes_merge_by_basename() {
    let idx = index(&[("a.md", "[[Projects/Todo]]"), ("b.md", "[[Home/Todo]]")]);
    let g = idx.graph(true);
    let ghosts: Vec<&str> = g
        .nodes
        .iter()
        .filter(|n| n.kind == cairn_core::index::GraphNodeKind::Unresolved)
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(ghosts, vec!["Projects/Todo", "Home/Todo"]);
}

#[test]
fn held_graph_edges_follow_resolution_and_skip_attachments_and_self() {
    let idx = index(&[
        ("a.md", "[[b]] [[B]] [[a]] [[#x]] ![[pic.png]] [m](b.md) [[sub/c]]"),
        ("b.md", ""),
        ("sub/c.md", ""),
        ("pic.png", ""),
    ]);
    let g = idx.graph(false);
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["a.md", "b.md", "sub/c.md"]);
    assert_eq!(g.edges, vec![(0, 1), (0, 2)]);
    assert_eq!(g.nodes[0].degree, 2);
}

/// Outline text for a multi-line setext heading drops the line break
/// entirely (SoftBreak events are not turned into spaces): "Line one" +
/// "line two" becomes "Line oneline two".
#[test]
fn finding_multiline_setext_heading_text() {
    let p = parse::parse("Line one\nline two\n===\n");
    assert_eq!(p.headings[0].text, "Line one line two");
}
