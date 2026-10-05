//! Reproduction for FINDING-135: Vault::write_note must refuse paths inside
//! hidden folders, like create_note, create_file, create_folder and
//! write_file. Without that check it would overwrite the hidden file and put
//! the hidden folder and file into the index.
//!
//! The write goes through the same calls the plugin bridge makes for
//! `notes.write` on an existing file (plugins.ts: readNote, then writeNote
//! with the hash just read), so the base-hash check passes.
//!
//! Same core root cause as FINDING-071 (which is how it reaches plugins).
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_fs_14 -- --include-ignored --nocapture

use std::fs;
use std::sync::Arc;

use cairn_core::index::{hash_bytes, hash_hex};
use cairn_core::{Change, CoreError, EntryKind, StdFs, TrashMode, Vault};

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

/// Control: every other write entry point refuses hidden paths.
#[test]
fn other_entry_points_refuse_hidden_paths() {
    let (_d, v) = setup(&[(".trash/old.md", "trashed"), ("n.md", "n")]);
    assert!(matches!(v.create_note(".trash/new.md", "x"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.create_file(".trash/new.png", b"x"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.create_folder(".trash/sub"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.write_file(".trash/old.md", b"x", None), Err(CoreError::InvalidPath(_))));
    // Hidden entries are not indexed on open.
    assert!(v.entries().iter().all(|e| !e.path.starts_with('.')));
}

#[test]
fn fs14_plugin_style_write_into_hidden_folder() {
    let (d, v) = setup(&[(".trash/old.md", "trashed unique-zebra"), ("n.md", "n")]);
    // What plugins.ts notes.write does for an existing file. read_note
    // refuses hidden paths (FINDING-022), so the base hash comes from disk.
    let base = hash_hex(&hash_bytes(&fs::read(d.path().join(".trash/old.md")).unwrap()));
    let w = v.write_note(".trash/old.md", "overwritten quokka", Some(&base));
    let disk = fs::read_to_string(d.path().join(".trash/old.md")).unwrap();
    let hidden_in_index: Vec<String> = v.entries().into_iter().map(|e| e.path).filter(|p| p.starts_with('.')).collect();
    let hits: Vec<String> = v.search("quokka", 10).into_iter().map(|h| h.path).collect();
    let rescan = v.rescan().unwrap();
    println!("write_note: {:?}", w.as_ref().map(|r| &r.changes));
    println!("disk now: {disk:?}");
    println!("hidden entries in index: {hidden_in_index:?}");
    println!("search 'quokka': {hits:?}");
    println!("next rescan: {rescan:?}");
    let mut problems = Vec::new();
    if w.is_ok() {
        problems.push("write_note accepted a hidden path".to_string());
    }
    if disk != "trashed unique-zebra" {
        problems.push(format!("hidden file overwritten: {disk:?}"));
    }
    if !hidden_in_index.is_empty() {
        problems.push(format!("hidden entries indexed: {hidden_in_index:?}"));
    }
    if !hits.is_empty() {
        problems.push(format!("search returns hidden file: {hits:?}"));
    }
    if rescan.iter().any(|c| matches!(c, Change::Deleted { path, kind: EntryKind::Dir } if path == ".trash")) {
        problems.push("rescan reports a phantom 'Deleted .trash' although nothing was deleted".to_string());
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
