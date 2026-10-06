//! Regression tests for FINDING-049 (one unreadable subfolder failed the
//! whole scan). A sync script built on the `sync_dir` example failed in
//! `Vault::open`, so it did not show the sync engine itself failing.
//! The first test opens the vault and syncs first, then makes a subfolder
//! unreadable and checks that the engine's next sync still pushes a new note
//! and does not tell the other device to delete the unreadable folder's notes.
//!
//!   cargo test -p cairn-sync --test adv_verify_fs_03 -- --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;
#[path = "adv_sync_robust_common.rs"]
mod robust;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::*;

fn chmod(p: &Path, mode: u32) {
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn fs03_unreadable_subfolder_does_not_stop_sync_engine() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("n.md", "n\n"), ("private/x.md", "x\n")]);
    laptop.sync();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync();
    assert_eq!(phone.read("private/x.md").as_deref(), Some("x\n"));

    chmod(&laptop.abs("private"), 0o000);
    laptop.write("new.md", "made after the folder became unreadable\n");
    let r = laptop.try_sync();
    chmod(&laptop.abs("private"), 0o755);
    println!("laptop sync with unreadable subfolder: {:?}", r.as_ref().map(|r| r.pushed).map_err(|e| e.to_string()));
    let pr = phone.try_sync();
    println!("phone files after: {:?}", phone.paths());
    assert!(r.is_ok(), "laptop sync failed: {}", r.err().unwrap());
    assert!(pr.is_ok());
    assert_eq!(phone.read("new.md").as_deref(), Some("made after the folder became unreadable\n"));
    // Skipping must not be mistaken for deleting.
    assert_eq!(phone.read("private/x.md").as_deref(), Some("x\n"));
}

/// After a restart the index has nothing inside the unreadable folder, so
/// the sync must not take the files it had synced from there for deleted.
#[test]
fn fs03_unreadable_subfolder_after_restart_is_not_deleted_elsewhere() {
    let srv = robust::server();
    let mut laptop = robust::Device::new(&srv, "laptop", &[("n.md", "n\n"), ("private/x.md", "x\n")]);
    laptop.sync_ok();
    let mut phone = robust::Device::new(&srv, "phone", &[]);
    phone.sync_ok();

    chmod(&laptop.root.join("private"), 0o000);
    laptop.restart();
    laptop.write("new.md", "made after the folder became unreadable\n");
    let r = laptop.sync();
    chmod(&laptop.root.join("private"), 0o755);
    assert!(r.is_ok(), "laptop sync failed: {}", r.err().unwrap());
    phone.sync_ok();
    assert_eq!(phone.read("new.md").as_deref(), Some("made after the folder became unreadable\n"));
    assert_eq!(phone.read("private/x.md").as_deref(), Some("x\n"));
    // Readable again: the folder's files are found where they were.
    laptop.sync_ok();
    phone.sync_ok();
    assert_eq!(laptop.read("private/x.md").as_deref(), Some("x\n"));
    assert_eq!(phone.read("private/x.md").as_deref(), Some("x\n"));
}

/// Files the sync has not uploaded yet (new, or changed since the last
/// sync) cannot be read once their folder becomes unreadable. The sync
/// leaves them for later instead of failing.
#[test]
fn fs03_unsynced_files_in_a_folder_that_became_unreadable_do_not_stop_sync() {
    let srv = robust::server();
    let mut laptop = robust::Device::new(&srv, "laptop", &[("n.md", "n\n"), ("private/x.md", "x\n")]);
    laptop.sync_ok();
    let mut phone = robust::Device::new(&srv, "phone", &[]);
    phone.sync_ok();

    laptop.write("private/new.md", "new\n");
    laptop.write("private/x.md", "x edited\n");
    laptop.vault.rescan().unwrap(); // the watcher saw both
    chmod(&laptop.root.join("private"), 0o000);
    laptop.write("other.md", "other\n");
    let r = laptop.sync();
    chmod(&laptop.root.join("private"), 0o755);
    assert!(r.is_ok(), "laptop sync failed: {}", r.err().unwrap());
    phone.sync_ok();
    assert_eq!(phone.read("other.md").as_deref(), Some("other\n"));
    assert_eq!(phone.read("private/x.md").as_deref(), Some("x\n"));
    // Readable again: both changes are uploaded.
    laptop.sync_ok();
    phone.sync_ok();
    assert_eq!(phone.read("private/new.md").as_deref(), Some("new\n"));
    assert_eq!(phone.read("private/x.md").as_deref(), Some("x edited\n"));
}

/// A folder whose names can be read but not opened (mode 644, as after a
/// `chmod -R 644`) is unreadable too: its files are not deleted elsewhere.
#[test]
fn fs03_folder_without_search_permission_is_not_deleted_elsewhere() {
    let srv = robust::server();
    let mut laptop = robust::Device::new(&srv, "laptop", &[("n.md", "n\n"), ("private/x.md", "x\n")]);
    laptop.sync_ok();
    let mut phone = robust::Device::new(&srv, "phone", &[]);
    phone.sync_ok();

    chmod(&laptop.root.join("private"), 0o644);
    let r = laptop.sync();
    chmod(&laptop.root.join("private"), 0o755);
    assert!(r.is_ok(), "laptop sync failed: {}", r.err().unwrap());
    phone.sync_ok();
    assert_eq!(phone.read("private/x.md").as_deref(), Some("x\n"));
    assert!(phone.trash().is_empty(), "phone trash: {:?}", phone.trash());
}

/// The same for a mode-644 folder that holds only symlinks: each link
/// looks broken, so the folder looks empty, and its files must still not be
/// deleted on the other devices.
#[test]
fn fs03_folder_of_links_without_search_permission_is_not_deleted_elsewhere() {
    let srv = robust::server();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("o.md"), "outside\n").unwrap();
    let mut laptop = robust::Device::new(&srv, "laptop", &[("n.md", "n\n")]);
    fs::create_dir(laptop.root.join("refs")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("o.md"), laptop.root.join("refs/o.md")).unwrap();
    laptop.sync_ok();
    let mut phone = robust::Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    assert_eq!(phone.read("refs/o.md").as_deref(), Some("outside\n"));

    chmod(&laptop.root.join("refs"), 0o644);
    let r = laptop.sync();
    chmod(&laptop.root.join("refs"), 0o755);
    assert!(r.is_ok(), "laptop sync failed: {}", r.err().unwrap());
    phone.sync_ok();
    assert_eq!(phone.read("refs/o.md").as_deref(), Some("outside\n"));
    assert!(phone.trash().is_empty(), "phone trash: {:?}", phone.trash());
}

/// A symlink whose target this device may not reach (in a folder without
/// permission) looked like a broken link, so its file was deleted on the
/// other devices. It is unknown instead, before and after a restart.
#[test]
fn fs03_link_that_cannot_be_followed_is_not_deleted_elsewhere() {
    let srv = robust::server();
    let outside = tempfile::tempdir().unwrap();
    let locked = outside.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("o.md"), "outside\n").unwrap();
    let mut laptop = robust::Device::new(&srv, "laptop", &[("n.md", "n\n")]);
    fs::create_dir(laptop.root.join("refs")).unwrap();
    std::os::unix::fs::symlink(locked.join("o.md"), laptop.root.join("refs/o.md")).unwrap();
    std::os::unix::fs::symlink(locked.join("o.md"), laptop.root.join("top.md")).unwrap();
    laptop.sync_ok();
    let mut phone = robust::Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    assert_eq!(phone.read("refs/o.md").as_deref(), Some("outside\n"));

    chmod(&locked, 0o000);
    let r = laptop.sync();
    laptop.restart();
    let after_restart = laptop.sync();
    chmod(&locked, 0o755);
    assert!(r.is_ok(), "laptop sync failed: {}", r.err().unwrap());
    assert!(after_restart.is_ok(), "laptop sync after a restart failed: {}", after_restart.err().unwrap());
    phone.sync_ok();
    assert_eq!(phone.read("refs/o.md").as_deref(), Some("outside\n"));
    assert_eq!(phone.read("top.md").as_deref(), Some("outside\n"));
    assert!(phone.trash().is_empty(), "phone trash: {:?}", phone.trash());
}
