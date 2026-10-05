//! Reproduction for FINDING-137: a folder replaced (outside Cairn) by a
//! different folder whose files have the same relative names, sizes and
//! millisecond mtimes looks like a folder rename. The moved children must
//! still be re-read: otherwise the index keeps the old text, and a save with
//! a fresh disk hash could turn into a Conflict.
//!
//! The control test shows the same replacement with a different mtime is
//! handled correctly (rename + modification, fresh index, save works), so the
//! case needs equal size AND equal millisecond mtime for every file.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_fs_17 -- --include-ignored --nocapture

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use cairn_core::{CoreError, StdFs, TrashMode, Vault};

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

fn replace_folder(d: &tempfile::TempDir, same_mtime: bool) {
    let m = fs::metadata(d.path().join("old/x.md")).unwrap().modified().unwrap();
    fs::remove_dir_all(d.path().join("old")).unwrap();
    fs::create_dir_all(d.path().join("new")).unwrap();
    fs::write(d.path().join("new/x.md"), "bbbb [[t2]]").unwrap();
    let t = if same_mtime { m } else { m - Duration::from_secs(3600) };
    fs::OpenOptions::new().write(true).open(d.path().join("new/x.md")).unwrap().set_modified(t).unwrap();
}

#[test]
fn control_replaced_folder_with_different_mtime_is_reindexed() {
    let (d, v) = setup(&[("t1.md", ""), ("old/x.md", "aaaa [[t1]]")]);
    replace_folder(&d, false);
    let c = v.rescan().unwrap();
    assert_eq!(v.index().note("new/x.md").map(|n| n.content.clone()).as_deref(), Some("bbbb [[t2]]"), "{c:?}");
    let disk = v.read_note("new/x.md").unwrap();
    v.write_note("new/x.md", "edited", Some(&disk.hash)).unwrap();
}

#[test]
fn fs17_stale_index_and_persistent_conflict() {
    let (d, v) = setup(&[("t1.md", ""), ("old/x.md", "aaaa [[t1]]")]);
    replace_folder(&d, true);
    let c = v.rescan().unwrap();
    println!("rescan: {c:?}");
    let indexed = v.index().note("new/x.md").map(|n| n.content.clone());
    let backlinks_t1: Vec<String> = v.backlinks("t1.md").into_iter().map(|b| b.source).collect();
    let hits_aaaa: Vec<String> = v.search("aaaa", 10).into_iter().map(|h| h.path).collect();
    println!("indexed text: {indexed:?}; backlinks of t1.md: {backlinks_t1:?}; search aaaa: {hits_aaaa:?}");
    // The user opens the note (fresh hash from disk) and saves: twice.
    let mut saves = Vec::new();
    for _ in 0..2 {
        let disk = v.read_note("new/x.md").unwrap();
        saves.push(v.write_note("new/x.md", "bbbb [[t2]] edited", Some(&disk.hash)).map(|_| ()));
    }
    println!("saves with a fresh disk hash: {saves:?}");
    // A second rescan must keep it that way.
    let c2 = v.rescan().unwrap();
    println!("second rescan: {c2:?}");
    let mut problems = Vec::new();
    if indexed.as_deref() != Some("bbbb [[t2]]") {
        problems.push(format!("index holds deleted folder's text: {indexed:?}"));
    }
    if saves.iter().any(|s| matches!(s, Err(CoreError::Conflict(_)))) {
        problems.push(format!("save with fresh hash conflicts: {saves:?}"));
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
