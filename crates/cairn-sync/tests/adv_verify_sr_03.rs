//! Reproduction for FINDING-054 (lost, damaged, stale or reset
//! sync state undoes deletions and renames).
//!
//! Run: cargo test -p cairn-sync --test adv_verify_sr_03 -- --ignored --nocapture
//!
//! These split lost, damaged or stale state into the parts a client could
//! get right without its state, and leave out the part it cannot: a note the
//! user deleted locally while the state was lost is indistinguishable from a
//! note this device never had, so it coming back is inherent.
//! - (a) "Turn off sync" then turning it on again (the app's
//!   Settings > Sync flow; also what happens after moving the vault folder,
//!   since the state folder is keyed by the vault path) undoes a delete made
//!   on another device in the meantime, although the local copy is
//!   byte-identical to the last live revision. It comes back on every device.
//! - (b) the same with a damaged state.json (no warning, the engine loads).
//! - (c) a local rename made before the state was lost duplicates the note,
//!   although the new file's content equals the server head of the old path
//!   and the old path is absent locally.
//! - (d) a stale state.json makes this device write a conflict copy whose
//!   content is identical to the file already at that path (before any other
//!   device is involved; FINDING-061 then turns the original into a second
//!   conflict copy on the other device).

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;

use common::*;

fn pair(files: &[(&str, &str)]) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", files);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    (srv, a, b)
}

fn reconnect(a: &mut Device) {
    a.engine = None;
    cairn_sync::engine::SyncEngine::disconnect(&a.state_dir).unwrap();
    a.engine = Some(
        cairn_sync::engine::SyncEngine::connect_with(a.vault.clone(), &a.state_dir, settings(&a.url, "laptop"), PASS, http(&a.url), FAST_KDF)
            .unwrap(),
    );
}

#[test]
#[ignore = "FINDING-054 (deferred: needs an automatic delete, which sync does not make by design): turning sync off and on again undoes deletes made on other devices (unchanged local copy is re-uploaded)"]
fn a_turn_off_and_on_undoes_remote_delete_of_unchanged_note() {
    let (_srv, mut a, mut b) = pair(&[("keep.md", "k\n"), ("old.md", "obsolete\n")]);
    b.rm("old.md");
    b.sync_ok();
    assert_eq!(b.paths(), vec!["keep.md"]);
    // laptop: sync turned off and on again (no local change at all)
    assert_eq!(a.read("old.md").as_deref(), Some("obsolete\n"), "laptop copy is the last live revision");
    reconnect(&mut a);
    let r = a.sync_ok();
    eprintln!("laptop after reconnect: pulled={} pushed={} conflicts={:?}", r.pulled, r.pushed, r.conflicts);
    let rb = b.sync_ok();
    eprintln!("phone: pulled={} paths={:?}", rb.pulled, b.paths());
    assert_eq!(r.pushed, 0, "an unchanged note was re-uploaded over a tombstone");
    assert_eq!(b.paths(), vec!["keep.md"], "the note deleted on the phone is back on the phone");
}

#[test]
#[ignore = "FINDING-054 (deferred: needs an automatic delete, which sync does not make by design): a damaged state.json is reset (and logged) and then undoes deletes made on other devices"]
fn b_damaged_state_undoes_remote_delete_of_unchanged_note() {
    let (_srv, mut a, mut b) = pair(&[("keep.md", "k\n"), ("old.md", "obsolete\n")]);
    b.rm("old.md");
    b.sync_ok();
    fs::write(a.state_file(), b"{\"last_seq\": 4, \"fi").unwrap();
    a.restart(); // loads without any error
    assert!(a.engine().state().files.is_empty(), "damaged state was replaced by an empty one");
    a.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(b.paths(), vec!["keep.md"], "the note deleted on the phone is back on the phone");
}

#[test]
fn c_lost_state_duplicates_local_rename() {
    let (_srv, mut a, mut b) = pair(&[("keep.md", "k\n"), ("old_name.md", "o\n")]);
    a.mv("old_name.md", "new_name.md");
    fs::remove_file(a.state_file()).unwrap();
    a.restart();
    a.sync_ok();
    converge(&mut a, &mut b);
    eprintln!("phone: {:?}", b.files());
    assert_eq!(b.paths(), vec!["keep.md", "new_name.md"], "rename turned into a copy");
}

#[test]
fn d_stale_state_writes_identical_conflict_copy_locally() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "one\n"), ("r.md", "rename me\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    let old_state = fs::read(a.state_file()).unwrap();
    // rename and edit in two syncs, so the file id is kept (in one sync it is
    // uploaded as delete + create, FINDING-062, and the stale state is
    // harmless)
    b.mv("r.md", "sub/r.md");
    b.sync_ok();
    b.write("sub/r.md", "rename me\nand edit\n");
    b.sync_ok();
    a.sync_ok();
    let want = a.files();
    assert_eq!(want, vec![("n.md".to_string(), "one\n".to_string()), ("sub/r.md".to_string(), "rename me\nand edit\n".to_string())]);
    fs::write(a.state_file(), old_state).unwrap();
    a.restart();
    let r = a.sync_ok();
    eprintln!("laptop with stale state: conflicts={:?} files={:?}", r.conflicts, a.files());
    assert!(r.conflicts.is_empty(), "conflict copy of identical content: {:?}", a.files());
    assert_eq!(a.files(), want);
}
