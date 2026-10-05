//! Reproductions for FINDING-015 (a remote NEW file
//! overwrites a file created outside Cairn after the sync round's scan; a
//! remote delete trashes an edit saved after the scan).
//!
//! Questions checked: is the overwritten text recoverable anywhere (trash,
//! other device, server revision history)? How wide is the window in the
//! desktop app, where a file watcher rescans changed paths ~250 ms after an
//! external write (app/src-tauri/src/watcher.rs)? Does the delete case lose
//! anything, or only put the edit in the trash?
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_ss_03 -- --nocapture
//!   cargo test -p cairn-sync --test adv_verify_ss_03 -- --ignored --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::fs;

use common::*;

/// Every revision the server holds for every file id this device knows, decrypted.
fn server_history_text(d: &Device) -> String {
    let mut out = String::new();
    let paths: Vec<String> = d.engine.state().files.values().map(|t| t.path.clone()).collect();
    for p in paths {
        if let Ok(h) = d.engine.history(&p) {
            for e in h {
                if let Ok(rev) = d.engine.revision_content(e.seq) {
                    out.push_str(&String::from_utf8_lossy(&rev.data));
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// The external text was never uploaded, so it is not in the server's
/// version history either: if the race overwrote it, it would exist on no
/// disk, in no trash and in no server revision (that is the data loss).
#[test]
fn overwritten_external_file_is_recoverable_somewhere() {
    let srv = server();
    let (mut a, mut b) = (Device::new(&srv, "laptop", &[]), Device::new(&srv, "phone", &[]));
    a.sync();
    b.sync();
    b.write("Inbox.md", "phone inbox\n");
    b.sync();
    let root = a.root.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        fs::write(root.join("Inbox.md"), "UNIQUE laptop text typed in vim\n").unwrap();
    }));
    a.sync();
    converge(&mut [&mut a, &mut b]);
    let disk_a = a.all_text();
    let disk_b = b.all_text();
    let hist = server_history_text(&a);
    println!("laptop files {:?}\nphone files {:?}\nserver history contains it: {}", a.files(), b.files(), hist.contains("UNIQUE laptop text"));
    assert!(
        disk_a.contains("UNIQUE laptop text") || disk_b.contains("UNIQUE laptop text") || hist.contains("UNIQUE laptop text"),
        "the laptop's text is nowhere: not on either disk (trash included), not in server history"
    );
}

/// Mitigation in the desktop app: once the file watcher has rescanned the
/// externally created path (what `watcher.rs` does ~250 ms after the write),
/// the engine's `exists()` check sees it and the incoming note gets a
/// conflict name. So in the desktop app only an external
/// create that lands after the round's scan AND less than one watcher
/// debounce before the apply of that head is overwritten.
#[test]
fn watcher_rescan_before_apply_prevents_the_overwrite() {
    let srv = server();
    let (mut a, mut b) = (Device::new(&srv, "laptop", &[]), Device::new(&srv, "phone", &[]));
    a.sync();
    b.sync();
    b.write("Inbox.md", "phone inbox\n");
    b.sync();
    let root = a.root.clone();
    let vault = a.vault.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        fs::write(root.join("Inbox.md"), "laptop text typed in vim\n").unwrap();
        // the desktop watcher's callback, after its debounce
        vault.rescan_paths(&["Inbox.md".to_string()]).unwrap();
    }));
    a.sync();
    converge(&mut [&mut a, &mut b]);
    println!("laptop files {:?}", a.files());
    // Both texts survive as visible files on both devices. (Both may end
    // up as conflict copies and no plain Inbox.md remains: the phone's own
    // stale scan treats Inbox.md as taken when the laptop's new file
    // arrives. Cosmetic, not checked here.)
    for d in [&a, &b] {
        let text = d.non_trash_text();
        assert!(text.contains("laptop text typed in vim") && text.contains("phone inbox"), "{}: {:?}", d.name, d.files());
    }
}

/// FINDING-015 second case, impact: the edit saved after the scan is not
/// lost. It is not even moved to the trash (the app uses the OS trash on
/// desktop, the vault `.trash/` on Android); it stays in place (see
/// `in_app_edit_during_pull_beats_remote_delete`).
#[test]
fn edit_saved_during_pull_of_remote_delete_is_not_lost() {
    let srv = server();
    let (mut a, mut b) = (Device::new(&srv, "laptop", &[("todo.md", "buy milk\n"), ("anchor.md", "untouched\n")]), Device::new(&srv, "phone", &[]));
    a.sync();
    b.sync();
    b.rm("todo.md");
    b.sync();
    let vault = a.vault.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        // saved through Cairn's own UI path (the index knows about it)
        vault.write_file("todo.md", b"buy milk\nand call mom\n", None).unwrap();
    }));
    a.sync();
    converge(&mut [&mut a, &mut b]);
    println!("laptop files {:?}; trash {:?}", a.files(), a.trash_text());
    assert!(a.all_text().contains("and call mom"), "{:?}", a.files());
}

/// FINDING-015 second case, the rule: edit beats delete. The delete of a
/// file classified Unchanged must re-check the file's hash, so that an in-app
/// save (through Vault, which the index knows about) made after the scan is
/// not trashed.
#[test]
fn in_app_edit_during_pull_beats_remote_delete() {
    let srv = server();
    let (mut a, mut b) = (Device::new(&srv, "laptop", &[("todo.md", "buy milk\n"), ("anchor.md", "untouched\n")]), Device::new(&srv, "phone", &[]));
    a.sync();
    b.sync();
    b.rm("todo.md");
    b.sync();
    let vault = a.vault.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        vault.write_file("todo.md", b"buy milk\nand call mom\n", None).unwrap();
    }));
    a.sync();
    converge(&mut [&mut a, &mut b]);
    assert_eq!(a.read("todo.md").as_deref(), Some("buy milk\nand call mom\n"), "files {:?}; trash {:?}", a.files(), a.trash_text());
}
