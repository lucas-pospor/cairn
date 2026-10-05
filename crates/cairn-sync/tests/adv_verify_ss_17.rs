//! Reproduction for FINDING-149, showing the problem is wider
//! than a rename cycle (swap). The changes feed returns only the latest head
//! per file (cairn-server's `changes` handler), ordered by seq, and the
//! engine applies them one at a time. A remote rename whose target is still
//! occupied by a file that a LATER head in the same pull moves away must not
//! get a conflict name just because `self.exists(rpath)` is true, or that
//! name would be pushed to every device.
//!
//! No cycle is needed: "archive the old journal, rename the draft to
//! journal.md, then keep editing the archived journal" on one device, with
//! the other device offline for those three syncs, is enough.
//!
//!   cargo test -p cairn-sync --test adv_verify_ss_17

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

#[test]
fn rename_into_vacated_path_then_edit_vacating_file_needs_no_conflict_copy() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("journal.md", "old journal\n"), ("draft.md", "new journal\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    assert_eq!(b.paths(), vec!["draft.md".to_string(), "journal.md".to_string()]);

    a.mv("journal.md", "archive/journal-2026.md");
    a.sync();
    a.mv("draft.md", "journal.md");
    a.sync();
    a.write("archive/journal-2026.md", "old journal\nlast entry\n");
    a.sync();

    let r = b.sync();
    converge(&mut [&mut a, &mut b]);
    let want = vec![
        ("archive/journal-2026.md".to_string(), "old journal\nlast entry\n".to_string()),
        ("journal.md".to_string(), "new journal\n".to_string()),
    ];
    assert!(r.conflicts.is_empty(), "phone made conflict copies {:?}; laptop now has {:?}", r.conflicts, a.files());
    assert_eq!(a.files(), want);
}
