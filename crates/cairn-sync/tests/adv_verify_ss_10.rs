//! Regression tests for FINDING-061:
//! `apply_remote`'s new-file branch decides whether the remote path is
//! taken. If it used `local`, the scan taken at the start of the round,
//! instead of the current vault, then when an earlier head in the SAME pull
//! already moved the file away from that path, the new file would still get
//! a conflict name, and the push would upload that rename to every device
//! (through `Tracked::server_path`).
//!
//! 1. Control: the very same remote changes received in two separate pulls
//!    give no conflict. With the defect, only "both heads in one pull"
//!    differed, which isolated the stale round-start scan as the cause.
//! 2. Variant: a folder rename followed by a new note in a new folder
//!    with the old name ("Inbox/todo.md" -> "Done/todo.md", then a new
//!    "Inbox/todo.md"), received in one pull.
//!
//!   cargo test -p cairn-sync --test adv_verify_ss_10 -- --include-ignored

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

fn pair(srv: &Server, files: &[(&str, &str)]) -> (Device, Device) {
    let mut a = Device::new(srv, "laptop", files);
    a.sync();
    let mut b = Device::new(srv, "phone", &[]);
    b.sync();
    (a, b)
}

#[test]
fn control_rename_and_new_note_in_separate_pulls_has_no_conflict() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("Untitled.md", "first note\n")]);
    a.mv("Untitled.md", "Meeting notes.md");
    a.sync();
    let r1 = b.sync(); // the phone sees the rename on its own
    a.write("Untitled.md", "second note\n");
    a.sync();
    let r2 = b.sync(); // and the new note in a later pull
    converge(&mut [&mut a, &mut b]);
    assert!(r1.conflicts.is_empty() && r2.conflicts.is_empty(), "{:?} {:?}", r1.conflicts, r2.conflicts);
    assert_eq!(
        b.files(),
        vec![("Meeting notes.md".to_string(), "first note\n".to_string()), ("Untitled.md".to_string(), "second note\n".to_string())]
    );
}

#[test]
fn folder_rename_then_new_note_with_the_old_path_received_in_one_pull() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("Inbox/todo.md", "old list\n")]);
    a.mv("Inbox", "Done");
    a.sync();
    a.write("Inbox/todo.md", "new list\n");
    a.sync();
    let r = b.sync(); // the phone was offline for both
    converge(&mut [&mut a, &mut b]);
    assert_eq!(
        a.files(),
        vec![("Done/todo.md".to_string(), "old list\n".to_string()), ("Inbox/todo.md".to_string(), "new list\n".to_string())],
        "the laptop's own new note was renamed by the phone's upload (phone conflicts: {:?})",
        r.conflicts
    );
}
