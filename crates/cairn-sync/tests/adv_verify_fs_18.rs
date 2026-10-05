//! Reproduction of the impact of FINDING-138 on sync: a file
//! whose on-disk name is a single backslash (legal on Linux) is listed by
//! StdFs::list_into under the empty path "" (vpath::normalize splits on '\\'),
//! i.e. as the vault root itself with kind File. The sync scan then tries to
//! read_file("") (the vault root folder) and the error is not NotFound.
//!
//! A file named `back\slash.md` is listed as `back/slash.md`, which cannot be
//! read (NotFound), so sync skips it silently and never uploads it.
//!
//! Run:
//!   cargo test -p cairn-sync --test adv_verify_fs_18 -- --include-ignored --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

#[test]
fn fs18_file_named_backslash_breaks_sync() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("ok.md", "ok\n")]);
    a.sync();
    std::fs::write(a.root.join("\\"), "stray file\n").unwrap();
    let r = a.try_sync();
    println!("sync with a file named '\\\\': {:?}", r.as_ref().map(|r| r.pushed).map_err(|e| e.to_string()));
    let entries: Vec<String> = a.vault.entries().into_iter().map(|e| e.path).collect();
    println!("index entries: {entries:?}");
    // Edits to other notes must still sync.
    a.write("ok.md", "ok edited\n");
    let r2 = a.try_sync();
    println!("next sync: {:?}", r2.as_ref().map(|r| r.pushed).map_err(|e| e.to_string()));
    assert!(r.is_ok() && r2.is_ok(), "sync fails while the vault holds a file named '\\\\'");
    // Sync no longer stops at it (FINDING-017), and the vault root is not
    // indexed as a file (FINDING-138).
    assert!(!entries.iter().any(|p| p.is_empty()), "the vault root is indexed as a file: {entries:?}");
}

#[test]
#[ignore = "FINDING-011 won't fix (by design): a note named 'back\\slash.md' is skipped and listed in the files not synced"]
fn fs18_backslash_note_is_never_synced() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("ok.md", "ok\n")]);
    a.sync();
    std::fs::write(a.root.join("back\\slash.md"), "note with backslash\n").unwrap();
    let r = a.try_sync().map(|r| r.pushed).map_err(|e| e.to_string());
    let mut b = Device::new(&srv, "phone", &[]);
    let rb = b.try_sync().map(|r| r.pushed).map_err(|e| e.to_string());
    println!("laptop sync: {r:?}; phone sync: {rb:?}; phone files: {:?}", b.files());
    assert!(b.files().iter().any(|(_, c)| c == "note with backslash\n"), "the note never reached the other device");
}
