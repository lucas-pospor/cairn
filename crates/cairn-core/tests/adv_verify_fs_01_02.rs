//! Follow-up for FINDING-003 (likelihood and scope). Folders that differ
//! only in case can still appear on disk (another app, a case-sensitive sync
//! peer), and merging those twin folders by moving notes must not destroy a
//! note that exists in both. The controls show where the damage would stop:
//! folders, and plain renames, are safe.
//!
//!   cargo test -p cairn-core --test adv_verify_fs_01_02 -- --include-ignored --nocapture

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{CoreError, StdFs, TrashMode, Vault};

fn vault_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

fn holders(dir: &Path, root: &Path, needle: &str, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            holders(&p, root, needle, out);
        } else if fs::read_to_string(&p).is_ok_and(|s| s.contains(needle)) {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
        }
    }
}

/// Step 1: Cairn refuses to create `projects/` next to `Projects/` (what
/// the quick switcher does for "projects/todo", FINDING-053), so the twin is
/// made on disk, as another app or a case-sensitive sync peer can still do.
/// Step 2: the user merges the twins by dragging `projects/todo.md` into
/// `Projects/`. That must be refused like any other move onto an existing
/// note; otherwise it would replace `Projects/todo.md`, and the old text
/// would be nowhere in the vault (not in `.trash`).
#[test]
fn twin_folder_merge_move_is_refused_and_keeps_note() {
    let (d, v) = vault_with(&[("Projects/todo.md", "old list PRECIOUS\n")]);
    let r = v.create_note("projects/todo.md", "new list\n");
    assert!(matches!(r, Err(CoreError::AlreadyExists(_))), "create_note returned {:?}", r.map(|w| w.entry.path));
    fs::create_dir(d.path().join("projects")).unwrap();
    fs::write(d.path().join("projects/todo.md"), "new list\n").unwrap();
    v.rescan().unwrap();
    assert!(d.path().join("projects").is_dir() && d.path().join("Projects").is_dir());

    let r = v.rename("projects/todo.md", "Projects/todo.md");
    let mut found = Vec::new();
    holders(d.path(), d.path(), "PRECIOUS", &mut found);
    println!("rename -> {r:?}; files holding PRECIOUS: {found:?}; .trash exists: {}", d.path().join(".trash").exists());
    assert!(
        matches!(r, Err(CoreError::AlreadyExists(_))),
        "rename returned {r:?}; files holding PRECIOUS: {found:?}; .trash exists: {}",
        d.path().join(".trash").exists()
    );
    assert_eq!(found, vec!["Projects/todo.md".to_string()]);
}

/// Control: a case-only rename of a folder onto a non-empty twin folder
/// fails, so whole folders are not lost.
#[test]
fn control_case_only_folder_rename_onto_nonempty_twin_fails_safely() {
    let (d, v) = vault_with(&[("Projects/a.md", "A\n"), ("projects/b.md", "B\n")]);
    let r = v.rename("Projects", "projects");
    println!("folder rename -> {r:?}");
    assert!(r.is_err());
    assert_eq!(fs::read_to_string(d.path().join("Projects/a.md")).unwrap(), "A\n");
    assert_eq!(fs::read_to_string(d.path().join("projects/b.md")).unwrap(), "B\n");
}

/// Control: if the rename overwrote `A.md`, the index would match the disk
/// (one `A.md` holding the moved text), so nothing in the UI would hint that
/// a note was lost, and search would not find the old text either. The
/// rename is refused now, so these checks only run if that regresses.
#[test]
fn control_index_after_overwrite_shows_no_trace_of_the_lost_note() {
    let (_d, v) = vault_with(&[("a.md", "lower\n"), ("A.md", "UPPER precious\n")]);
    let _ = v.rename("a.md", "A.md");
    let paths: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    let hits = v.search("precious", 10);
    println!("entries after rename: {paths:?}; search 'precious': {} hits", hits.len());
    if fs::read_to_string(_d.path().join("A.md")).unwrap() == "lower\n" {
        assert_eq!(paths, vec!["A.md".to_string()]);
        assert!(hits.is_empty());
    }
}
