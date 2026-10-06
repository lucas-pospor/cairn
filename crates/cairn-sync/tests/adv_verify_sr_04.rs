//! Regression tests for FINDING-015 (writes made
//! while a sync round is waiting for the changes feed).
//!
//! The engine classifies the vault once, before the pull
//! (`SyncEngine::round` -> `scan` + `classify`), and `apply_remote` used to
//! perform unconditional destructive operations based on that snapshot and
//! on the in-memory index:
//! * new remote file: `self.exists()` (index only) + `write(.., None)`
//! * remote delete of an "Unchanged" file: `Vault::delete` without a hash check
//! * remote edit of a locally "Deleted" file: `self.exists()` + `write(.., None)`
//!
//! Beyond an external editor's write after the scan, these tests add three
//! cases:
//! 1. the remote-delete case also hit a save made through the app's own
//!    path (`Vault::write_note` with the correct base hash, which is what the
//!    `write_note` Tauri command does), so it was not limited to external
//!    editors;
//! 2. the "remotely edited, locally deleted" restore branch
//!    in `apply_remote` overwrote a note the user re-created meanwhile;
//! 3. once the index knows about the external file (the app's
//!    file watcher calls `rescan_paths` after a 250 ms debounce), the
//!    incoming file gets a conflict name, so the overwrite window was the time
//!    between the external write and the watcher's rescan.
//!
//!   cargo test -p cairn-sync --test adv_verify_sr_04

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::sync::Arc;

use cairn_core::index::{hash_bytes, hash_hex};
use common::*;

fn synced_pair(files: &[(&str, &str)]) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", files);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    (srv, a, b)
}

fn plain(d: &mut Device) {
    let url = d.url.clone();
    d.set_transport(http(&url));
}

/// Run `f(vault, root)` on A while its sync waits for the changes feed
/// (after the scan, before anything is applied).
fn during_pull(d: &mut Device, f: impl FnOnce(&Arc<cairn_core::Vault>, &std::path::Path) + Send + 'static) {
    let root = d.root.clone();
    let vault = d.vault.clone();
    let t = Arc::new(FaultTransport::passthrough(http(&d.url)));
    let mut f = Some(f);
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "changes" {
            if let Some(f) = f.take() {
                f(&vault, &root);
            }
        }
    }));
    d.set_transport(Box::new(SharedTransport(t)));
}

#[test]
fn app_save_during_pull_of_remote_delete_is_kept() {
    let (_srv, mut a, mut b) = synced_pair(&[("todo.md", "buy milk\n"), ("anchor.md", "untouched\n")]);
    b.rm("todo.md");
    b.sync_ok();
    during_pull(&mut a, |vault, _| {
        let base = hash_hex(&hash_bytes(b"buy milk\n"));
        vault.write_note("todo.md", "buy milk\nand call mom\n", Some(&base)).unwrap();
    });
    a.sync_ok();
    plain(&mut a);
    converge(&mut a, &mut b);
    assert_eq!(
        a.read("todo.md").as_deref(),
        Some("buy milk\nand call mom\n"),
        "A files: {:?}; A trash: {:?}; B files: {:?}",
        a.files(),
        a.trash(),
        b.files()
    );
}

#[test]
fn recreated_note_during_pull_of_remote_edit_is_not_overwritten() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n"), ("anchor.md", "untouched\n")]);
    a.rm("n.md"); // A deletes it, not synced yet
    b.write("n.md", "v1\nphone edit\n");
    b.sync_ok();
    during_pull(&mut a, |_, root| {
        std::thread::sleep(std::time::Duration::from_millis(3));
        std::fs::write(root.join("n.md"), "laptop: re-created, only copy\n").unwrap();
    });
    a.sync_ok();
    plain(&mut a);
    converge(&mut a, &mut b);
    let all = format!("{}{}", a.all_text(), b.all_text());
    assert!(all.contains("laptop: re-created, only copy"), "A files: {:?}; A trash: {:?}", a.files(), a.trash());
}

#[test]
fn external_create_seen_by_the_watcher_gets_a_conflict_copy() {
    // Same as FINDING-015 case 1, but the file watcher has already
    // rescanned the path (what the app does 250 ms after the write).
    let (_srv, mut a, mut b) = synced_pair(&[("x.md", "x\n")]);
    b.write("Meeting.md", "phone agenda\n");
    b.sync_ok();
    during_pull(&mut a, |vault, root| {
        std::fs::write(root.join("Meeting.md"), "laptop minutes\n").unwrap();
        vault.rescan_paths(&["Meeting.md".to_string()]).unwrap();
    });
    a.sync_ok();
    plain(&mut a);
    converge(&mut a, &mut b);
    // Both texts survive (the names may both end up as conflict copies once
    // the phone applies the laptop's rename; that is a naming quirk, not loss).
    let all = a.all_text();
    assert!(all.contains("laptop minutes") && all.contains("phone agenda"), "{:?}", a.files());
}
