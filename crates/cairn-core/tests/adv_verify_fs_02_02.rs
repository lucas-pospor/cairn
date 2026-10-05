//! Reproduction for FINDING-011, the parts adv_fs_names.rs does not reach:
//! deleting an NFD-named note, and creating a note inside a folder whose
//! name is NFD.
//!
//!   cargo test -p cairn-core --test adv_verify_fs_02_02 -- --ignored

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

const NFC: &str = "caf\u{e9}.md";
const NFD: &str = "cafe\u{301}.md";

fn open(p: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().escape_unicode().to_string())
        .filter(|n| !n.starts_with("\\u{2e}")) // hidden
        .collect();
    v.sort();
    v
}

#[test]
fn fs02_nfd_note_can_be_deleted() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(NFD), "hello from a mac").unwrap();
    let v = open(d.path());
    let r = v.delete(NFC);
    assert!(r.is_ok(), "delete: {r:?}; files on disk: {:?}", names(d.path()));
}

#[test]
fn fs02_new_note_in_nfd_folder_goes_into_that_folder() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("Re\u{301}sume\u{301}")).unwrap();
    let v = open(d.path());
    let folder: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    let r = v.create_note("R\u{e9}sum\u{e9}/new.md", "x");
    assert!(
        r.is_ok() && names(d.path()).len() == 1,
        "listed {folder:?}; create_note: {:?}; top-level names on disk: {:?}",
        r.map(|r| r.entry.path),
        names(d.path())
    );
}
