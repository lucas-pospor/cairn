// Regression tests for FINDING-090: a Vault::rename that changes whether a
// path is a note (todo.txt -> todo.md, Note.md -> Note.txt) must not carry
// the old index state over (index.rs rename_tree). A stale index would not
// be fixed by a full rescan or by the watcher's rescan_paths either.
//
// Run: cargo test -p cairn-core --test adv_verify_lk_20 -- --include-ignored --nocapture
//
// The first two tests are controls; the others check the fix.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn open(root: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(root, TrashMode::Vault).unwrap())).unwrap()
}

fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf, Vault) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    for (p, c) in files {
        fs::write(root.join(p), c).unwrap();
    }
    let v = open(&root);
    (d, root, v)
}

const TODO: &str = "buy zucchini\nsee [[Target]] #errand\n";

fn indexed_as_note(v: &Vault, path: &str) -> (usize, usize, bool, bool, bool) {
    let hits = v.search("zucchini", 10).iter().filter(|h| h.path == path).count();
    let bl = v.backlinks("Target.md").iter().filter(|b| b.source == path).count();
    let tag = v.tags().iter().any(|t| t.tag == "errand");
    let node = v.graph(false).nodes.iter().any(|n| n.id == path);
    let info = v.note_info(path).is_some();
    (hits, bl, tag, node, info)
}

/// Control: the same rename done outside the app (file manager) and picked
/// up by a rescan is indexed correctly (apply_diff never pairs a .txt with
/// a .md as a rename, so it becomes delete + create and the note is parsed).
#[test]
fn external_rename_txt_to_md_is_indexed() {
    let (_d, root, v) = vault(&[("todo.txt", TODO), ("Target.md", "")]);
    fs::rename(root.join("todo.txt"), root.join("todo.md")).unwrap();
    v.rescan_paths(&["todo.txt".into(), "todo.md".into()]).unwrap();
    assert_eq!(indexed_as_note(&v, "todo.md"), (1, 1, true, true, true));
}

/// Control: a fresh open of the folder after the in-app rename sees the note.
#[test]
fn reopen_after_rename_txt_to_md_is_indexed() {
    let (_d, root, v) = vault(&[("todo.txt", TODO), ("Target.md", "")]);
    v.rename("todo.txt", "todo.md").unwrap();
    assert_eq!(indexed_as_note(&open(&root), "todo.md"), (1, 1, true, true, true));
}

/// The in-app rename (file tree rename, or a remote rename applied by sync)
/// must index todo.md as a note right away, and the watcher's rescan_paths
/// for both paths and a full rescan (what sync runs first) keep it that way.
#[test]
fn in_app_rename_txt_to_md_is_indexed() {
    let (_d, _root, v) = vault(&[("todo.txt", TODO), ("Target.md", "")]);
    v.rename("todo.txt", "todo.md").unwrap();
    let after_rename = indexed_as_note(&v, "todo.md");
    v.rescan_paths(&["todo.txt".into(), "todo.md".into()]).unwrap();
    let after_watch = indexed_as_note(&v, "todo.md");
    v.rescan().unwrap();
    let after_rescan = indexed_as_note(&v, "todo.md");
    eprintln!("(search, backlinks, tag, graph node, note_info): rename {after_rename:?} watcher {after_watch:?} rescan {after_rescan:?}");
    assert_eq!(after_rename, (1, 1, true, true, true), "right after rename");
    assert_eq!(after_rescan, (1, 1, true, true, true), "after rescan");
}

/// Reverse: Note.md -> Note.txt through Vault::rename (sync or API; the file
/// tree appends .md, so it cannot do this) must not stay a note in the index.
#[test]
fn in_app_rename_md_to_txt_is_not_a_note() {
    let (_d, _root, v) = vault(&[("Note.md", "secret zucchini [[Target]] #errand"), ("Target.md", "")]);
    v.rename("Note.md", "Note.txt").unwrap();
    v.rescan().unwrap();
    let st = indexed_as_note(&v, "Note.txt");
    eprintln!("(search, backlinks, tag, graph node, note_info) for Note.txt: {st:?}");
    assert_eq!(st, (0, 0, false, false, false));
}

/// Editing the renamed note through the app's save path leaves it indexed as
/// a note.
#[test]
fn editing_after_rename_keeps_note_indexed() {
    let (_d, _root, v) = vault(&[("todo.txt", TODO), ("Target.md", "")]);
    v.rename("todo.txt", "todo.md").unwrap();
    let n = v.read_note("todo.md").unwrap();
    v.write_note("todo.md", &format!("{}x\n", n.content), Some(&n.hash)).unwrap();
    assert_eq!(indexed_as_note(&v, "todo.md"), (1, 1, true, true, true));
}
