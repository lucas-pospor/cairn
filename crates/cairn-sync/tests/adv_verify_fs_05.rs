//! Regression tests for FINDING-012 (symlink loops were followed by
//! `StdFs::list_into`, crates/cairn-core/src/fs.rs, with no visited-directory
//! check; a depth > 64 guard never fired because the kernel stops at 40
//! symlinks first, and that ELOOP on `fs::metadata` was silently skipped).
//!
//! Following the links, one `loop -> .` link turned one note into 41 notes
//! that were uploaded and arrived on the other device as 41 real files. When
//! the user on the other device cleaned up the 40 duplicate files, those
//! deletes were pulled by the first device and applied to `loop/note.md`,
//! `loop/loop/note.md`, ... which all resolve through the link to the one
//! real `note.md`, so the only copy of the note was moved to the trash, and
//! that delete then propagated back. Now the link is not listed, so only the
//! real note is synced.
//!
//!   cargo test -p cairn-sync --test adv_verify_fs_05 -- --nocapture

// Symlinks need Developer Mode or admin rights on Windows.
#![cfg(unix)]

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::os::unix::fs::symlink;

use common::*;

#[test]
fn fs05_loop_link_syncs_no_duplicates_and_keeps_real_note() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("note.md", "my only note\n")]);
    laptop.sync_ok();
    // The user adds a link to the vault root (or to any ancestor folder).
    symlink(".", laptop.root.join("loop")).unwrap();
    laptop.restart(); // fresh open of the vault, as on the next app start
    laptop.sync_ok();

    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    let got = phone.paths();
    println!("phone received {got:?}");
    assert_eq!(got, vec!["note.md".to_string()], "the loop produced duplicates");

    // The user removes the link again.
    std::fs::remove_file(laptop.root.join("loop")).unwrap();
    laptop.sync_ok();
    phone.sync_ok();
    assert_eq!(laptop.read("note.md").as_deref(), Some("my only note\n"));
    assert_eq!(phone.read("note.md").as_deref(), Some("my only note\n"));
    assert!(laptop.trash().is_empty() && phone.trash().is_empty());
}

/// Copies below a loop that a client following the link already synced:
/// once the first device no longer lists them, they are deleted on the other
/// devices (moved to the trash there), and the real note stays everywhere.
#[test]
fn fs05_duplicates_synced_before_the_fix_are_removed_and_the_note_stays() {
    let srv = server();
    // A real folder stands in for what such a client listed through the link.
    let mut laptop = Device::new(&srv, "laptop", &[("note.md", "my only note\n"), ("loop/note.md", "my only note\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    assert_eq!(phone.paths(), vec!["loop/note.md".to_string(), "note.md".to_string()]);

    std::fs::remove_dir_all(laptop.root.join("loop")).unwrap();
    symlink(".", laptop.root.join("loop")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    phone.sync_ok();
    laptop.sync_ok();

    assert_eq!(phone.paths(), vec!["note.md".to_string()]);
    assert_eq!(phone.trash(), vec![("note.md".to_string(), "my only note\n".to_string())]);
    assert_eq!(laptop.read("note.md").as_deref(), Some("my only note\n"));
    assert_eq!(phone.read("note.md").as_deref(), Some("my only note\n"));
    assert!(laptop.trash().is_empty(), "laptop trash: {:?}", laptop.trash());
}

/// The listing skips a loop, but a path through it still leads to a folder
/// on disk. A pulled file at such a path (another device has a real folder
/// with the loop's name) is not written through the loop over the note
/// there: it waits until the path is free.
#[test]
fn fs05_a_pulled_file_under_a_loop_is_not_written_through_it() {
    let srv = server();
    let mut phone = Device::new(&srv, "phone", &[("loop/todo.md", "phone's todo\n")]);
    phone.sync_ok();
    let mut laptop = Device::new(&srv, "laptop", &[("todo.md", "laptop's todo\n")]);
    symlink(".", laptop.root.join("loop")).unwrap();
    let _ = laptop.sync();
    let _ = laptop.sync();
    assert_eq!(laptop.read("todo.md").as_deref(), Some("laptop's todo\n"));
    assert!(laptop.trash().is_empty(), "laptop trash: {:?}", laptop.trash());
    phone.sync_ok();
    assert_eq!(phone.read("loop/todo.md").as_deref(), Some("phone's todo\n"));
    // Without the loop, the file arrives where it belongs.
    std::fs::remove_file(laptop.root.join("loop")).unwrap();
    laptop.sync_ok();
    phone.sync_ok();
    for (name, d) in [("laptop", &laptop), ("phone", &phone)] {
        assert_eq!(d.read("todo.md").as_deref(), Some("laptop's todo\n"), "{name}: {:?}", d.files());
        assert_eq!(d.read("loop/todo.md").as_deref(), Some("phone's todo\n"), "{name}: {:?}", d.files());
    }
}

/// A client following the link synced the notes below a loop as real files.
/// Later the other device edits such a copy while this device edits the
/// note itself: the copy's edit is not written through the loop over it.
#[test]
fn fs05_an_edit_to_an_old_loop_copy_does_not_overwrite_the_note() {
    let srv = server();
    let body = "line1\nline2\nline3\nline4\nline5\n";
    let mut laptop = Device::new(&srv, "laptop", &[("plan.md", body), ("loop/plan.md", body)]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    // What such a client listed through `loop -> .`.
    std::fs::remove_dir_all(laptop.root.join("loop")).unwrap();
    symlink(".", laptop.root.join("loop")).unwrap();
    laptop.restart();
    laptop.write("plan.md", "LAPTOP EDIT\nline2\nline3\nline4\nline5\n");
    phone.write("loop/plan.md", "line1\nline2\nline3\nline4\nPHONE EDIT\n");
    phone.sync_ok();
    let _ = laptop.sync();
    phone.sync_ok();
    let _ = laptop.sync();
    assert_eq!(laptop.read("plan.md").as_deref(), Some("LAPTOP EDIT\nline2\nline3\nline4\nline5\n"));
    assert!(laptop.trash().is_empty(), "laptop trash: {:?}", laptop.trash());
    assert!(phone.all_text().contains("PHONE EDIT"), "phone: {:?}", phone.files());
}
