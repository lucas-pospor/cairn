//! Regression tests for FINDING-062:
//! a rename followed by an edit before the next sync must be uploaded as a
//! rename of the same file, not as a delete plus a new file.
//!
//! The FINDING-062 tests in adv_sync_semantics.rs rename with `fs::rename`
//! (an external rename).
//! This file drives the same scenario through the app's own code path:
//! `cairn_sync::engine::rename` (what the `rename_entry` command calls: it
//! runs `Vault::rename`, which reports `Change::Renamed`, and records the
//! rename for the next sync) followed by `Vault::write_file` (what autosave
//! calls). The app debounces sync by 4 s after the last local change
//! (app/src-tauri/src/sync.rs DEBOUNCE), so "rename, then keep typing" ends
//! up in one sync.
//!
//! Run:
//!   cargo test -p cairn-sync --test adv_verify_ss_11 -- --include-ignored

#[path = "adv_sync_semantics_common.rs"]
mod common;

use cairn_core::Change;
use common::*;

fn pair(srv: &Server, files: &[(&str, &str)]) -> (Device, Device) {
    let mut a = Device::new(srv, "laptop", files);
    a.sync();
    let mut b = Device::new(srv, "phone", &[]);
    b.sync();
    (a, b)
}

/// Control: a rename through the vault API with no edit is linked to the
/// file id and merges with a concurrent edit elsewhere (PLAN section 3).
#[test]
fn ui_rename_alone_merges_with_remote_edit() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("draft.md", "1\n2\n3\n4\n5\n")]);
    a.vault.rename("draft.md", "final.md").unwrap();
    b.vault.write_file("draft.md", b"1\n2\n3\n4\n5 phone\n", None).unwrap();
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b]);
    assert_eq!(a.files(), vec![("final.md".to_string(), "1\n2\n3\n4\n5 phone\n".to_string())]);
}

/// The app's own rename (Vault::rename reports Change::Renamed) followed by
/// one autosave before the debounced sync: the rename must not be lost, and
/// the remote edit must not revive the old name next to the renamed copy.
#[test]
fn ui_rename_then_autosave_merges_with_remote_edit() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("draft.md", "1\n2\n3\n4\n5\n")]);
    let changes = cairn_sync::engine::rename(&a.vault, &a.state_dir, "draft.md", "final.md").unwrap();
    assert!(
        changes.iter().any(|c| matches!(c, Change::Renamed { from, .. } if from == "draft.md")),
        "the vault itself knows this was a rename: {changes:?}"
    );
    // one keystroke's worth of autosave
    a.vault.write_file("final.md", b"1 \n2\n3\n4\n5\n", None).unwrap();
    b.vault.write_file("draft.md", b"1\n2\n3\n4\n5 phone\n", None).unwrap();
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b]);
    assert_eq!(
        a.files(),
        vec![("final.md".to_string(), "1 \n2\n3\n4\n5 phone\n".to_string())],
        "PLAN section 3: 'Rename on one side, edit on the other: both are applied, because the file_id links them'"
    );
}

/// Without a concurrent edit the files end up right either way, but the
/// other device must not get the old note in its trash, and the server-side
/// history of the renamed note must not restart at one revision.
#[test]
fn ui_rename_then_autosave_keeps_identity() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("draft.md", "v1\n")]);
    a.vault.write_file("draft.md", b"v2\n", None).unwrap();
    a.sync();
    b.sync();
    cairn_sync::engine::rename(&a.vault, &a.state_dir, "draft.md", "final.md").unwrap();
    a.vault.write_file("final.md", b"v2\nv3\n", None).unwrap();
    a.sync();
    b.sync();
    assert_eq!(b.files(), vec![("final.md".to_string(), "v2\nv3\n".to_string())]);
    let trash = b.trash_text();
    let h = b.engine.history("final.md").unwrap();
    assert!(
        h.len() >= 3 && !trash.contains("v2"),
        "history of final.md has {} entries; phone trash contains: {trash:?}",
        h.len()
    );
}
