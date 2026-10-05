//! Reproductions for FINDING-003 (a case-only rename onto
//! a *different* existing file replaces it on a case-sensitive file system).
//! These tests check where the overwritten note goes (trash? nowhere?), and
//! whether sync's server-side version history makes it recoverable.
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_ss_02 -- --ignored --nocapture
//!   cargo test -p cairn-sync --test adv_verify_ss_02 -- --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{CoreError, StdFs, TrashMode, Vault};
use common::*;

/// Every file under `root` (hidden ones included) whose bytes contain `needle`.
fn holders(root: &Path, needle: &str) -> Vec<String> {
    let mut v = Vec::new();
    walk(root, root, true, &mut v);
    v.into_iter()
        .filter(|(_, b)| String::from_utf8_lossy(b).contains(needle))
        .map(|(p, _)| p)
        .collect()
}

/// No sync: `Note.md -> note.md` must be refused for every trash mode, or
/// the old `note.md` would exist nowhere in the vault, not in `.trash` and
/// not in any other hidden folder.
#[test]
fn local_only_overwritten_note_goes_nowhere() {
    let mut failures = Vec::new();
    for mode in [TrashMode::Vault, TrashMode::System, TrashMode::Permanent] {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("Note.md"), "upper\n").unwrap();
        fs::write(d.path().join("note.md"), "lower PRECIOUS\n").unwrap();
        let root = d.path().canonicalize().unwrap();
        let v = Vault::open(Arc::new(StdFs::new(&root, mode).unwrap())).unwrap();
        let r = v.rename("Note.md", "note.md");
        let found = holders(&root, "PRECIOUS");
        let mut listing = Vec::new();
        walk(&root, &root, true, &mut listing);
        let names: Vec<_> = listing.iter().map(|(p, _)| p.clone()).collect();
        println!("{mode:?}: rename -> {r:?}; files holding PRECIOUS: {found:?}; all files: {names:?}");
        if r.is_ok() || found != vec!["note.md".to_string()] {
            failures.push(format!("{mode:?}: rename ok={}; PRECIOUS survives in {found:?}; vault now has {names:?}", r.is_ok()));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// With sync on and the note synced before. The rename is refused, so the
/// check is that nothing is pushed and both devices keep both notes.
#[test]
fn synced_case_twin_is_kept_on_both_devices() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("Note.md", "upper\n"), ("note.md", "lower PRECIOUS\n")]);
    laptop.sync();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync();
    assert_eq!(phone.read("note.md").as_deref(), Some("lower PRECIOUS\n"));
    assert_eq!(phone.read("Note.md").as_deref(), Some("upper\n"));

    let r = laptop.vault.rename("Note.md", "note.md");
    println!("rename -> {r:?}");
    assert!(matches!(r, Err(CoreError::AlreadyExists(_))), "rename returned {r:?}");
    let lr = laptop.sync();
    let pr = phone.sync();
    println!("laptop sync: pushed={} conflicts={:?}", lr.pushed, lr.conflicts);
    println!("phone sync: pulled={} conflicts={:?}", pr.pulled, pr.conflicts);
    assert_eq!(lr.pushed, 0);
    for d in [&laptop, &phone] {
        assert_eq!(d.read("note.md").as_deref(), Some("lower PRECIOUS\n"), "{:?}", d.files());
        assert_eq!(d.read("Note.md").as_deref(), Some("upper\n"), "{:?}", d.files());
    }
}

/// A file never synced (created after the last sync, or sync not set up) has
/// no server history, so with sync on it is still lost for good.
#[test]
fn unsynced_overwritten_note_is_unrecoverable_even_with_sync() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("Note.md", "upper\n")]);
    laptop.sync();
    laptop.write("note.md", "lower PRECIOUS fresh\n");
    let r = laptop.vault.rename("Note.md", "note.md");
    laptop.sync();
    let found = holders(&laptop.root, "PRECIOUS");
    let hist = laptop.engine.history("note.md").unwrap_or_default();
    let mut in_hist = false;
    for h in &hist {
        if let Ok(p) = laptop.engine.revision_content(h.seq) {
            in_hist |= String::from_utf8_lossy(&p.data).contains("PRECIOUS");
        }
    }
    println!("rename -> {r:?}; on disk: {found:?}; in server history: {in_hist}");
    assert!(r.is_err(), "rename succeeded; on disk: {found:?}; in server history: {in_hist}");
}
