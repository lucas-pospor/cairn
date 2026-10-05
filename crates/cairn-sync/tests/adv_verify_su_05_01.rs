//! Reproduction for FINDING-024: SyncEngine::restore must not overwrite
//! local edits that were never uploaded, or the text would end up nowhere
//! (not on disk, not in the server history, not in the trash).
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_su_05_01 -- --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

#[test]
fn restore_keeps_unsynced_local_text_somewhere() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Restore.md", "version one\n")]);
    a.sync();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    a.write("Restore.md", "version two\n");
    a.sync();
    // Unsynced local edit (offline, or within the 4 s debounce).
    std::thread::sleep(std::time::Duration::from_millis(1100));
    a.write("Restore.md", "version two\nUNSYNCED WORK\n");
    let hist = a.engine.history("Restore.md").unwrap();
    let oldest = hist.last().unwrap().seq;
    // The engine refuses to replace text the server does not have yet ...
    assert!(a.engine.restore("Restore.md", oldest).is_err(), "restore replaced text that was never synced");
    assert_eq!(a.read("Restore.md").as_deref(), Some("version two\nUNSYNCED WORK\n"));
    // ... so the app syncs first (SyncManager::restore), then restores.
    a.sync();
    a.engine.restore("Restore.md", oldest).unwrap();
    a.sync();
    assert_eq!(a.read("Restore.md").as_deref(), Some("version one\n"));
    let mut revs = Vec::new();
    for h in a.engine.history("Restore.md").unwrap() {
        revs.push(String::from_utf8_lossy(&a.engine.revision_content(h.seq).unwrap().data).to_string());
    }
    let all = a.all_text();
    println!("disk={:?} revisions={revs:?} trash={:?}", a.read("Restore.md"), a.trash_text());
    assert!(all.contains("UNSYNCED WORK") || revs.iter().any(|r| r.contains("UNSYNCED WORK")),
        "unsynced text lost: disk={:?} revisions={revs:?}", a.read("Restore.md"));
}
