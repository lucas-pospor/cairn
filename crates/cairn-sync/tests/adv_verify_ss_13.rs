//! Reproduction for the sync half of FINDING-011: file names that
//! `vpath::normalize` changes (NFD Unicode, a backslash) are listed under a
//! vault path that does not exist on disk, so the sync scan's `read_file`
//! gets NotFound and can skip the file silently (`SyncEngine::scan`). For a
//! file that was already synced, the skip makes the engine think the note
//! was deleted, and the other device moves it to its trash. NFD names are
//! handled; names with a backslash are skipped by design (the ignored tests).
//!
//!   cargo test -p cairn-sync --test adv_verify_ss_13 -- --ignored

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

const NFC: &str = "caf\u{e9}.md";
const NFD: &str = "cafe\u{301}.md";

fn listed(d: &Device) -> Vec<String> {
    d.vault.entries().into_iter().map(|e| e.path).collect()
}

#[test]
fn ss13_new_nfd_note_is_pushed_or_reported() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[("plain.md", "plain\n")]);
    a.write(NFD, "hello from a mac\n");
    let r = a.try_sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    // Either the note reaches device B, or the sync tells the user.
    let ok = r.as_ref().map(|r| r.pushed).unwrap_or(0);
    assert!(
        r.is_err() || b.paths().len() == 2,
        "sync result Ok(pushed={ok}); device A lists {:?}; device B has {:?}",
        listed(&a),
        b.paths()
    );
}

#[test]
#[ignore = "FINDING-011 won't fix (by design): a new note whose name has a backslash is skipped and listed in the files not synced"]
fn ss13_new_backslash_note_is_pushed_or_reported() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[("plain.md", "plain\n")]);
    a.write("draft\\v2.md", "backslash\n");
    let r = a.try_sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    let ok = r.as_ref().map(|r| r.pushed).unwrap_or(0);
    assert!(
        r.is_err() || b.paths().len() == 2,
        "sync result Ok(pushed={ok}); device A lists {:?}; device B has {:?}",
        listed(&a),
        b.paths()
    );
}

#[test]
fn fs02_synced_note_replaced_by_nfd_copy_is_trashed_elsewhere() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[(NFC, "v1\n"), ("anchor.md", "untouched\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    assert_eq!(b.read(NFC).as_deref(), Some("v1\n"));
    // e.g. the folder is restored from a Mac backup / rsynced from a Mac
    a.rm(NFC);
    a.write(NFD, "v1 plus an edit\n");
    let ra = a.sync();
    let rb = b.sync();
    assert_eq!(
        b.read(NFC).as_deref(),
        Some("v1 plus an edit\n"),
        "device A sync: pushed {} (lists {:?}); device B sync: pulled {}; B has {:?}; B trash: {:?}",
        ra.pushed,
        listed(&a),
        rb.pulled,
        b.paths(),
        b.trash_text()
    );
}

#[test]
#[ignore = "FINDING-011 won't fix (by design): a note renamed to a name with a backslash is skipped here, so it counts as deleted elsewhere"]
fn ss13_synced_note_renamed_to_backslash_name_is_trashed_elsewhere() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[("v2.md", "draft two\n"), ("anchor.md", "untouched\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    assert_eq!(b.read("v2.md").as_deref(), Some("draft two\n"));
    a.mv("v2.md", "draft\\v2.md");
    let ra = a.sync();
    let rb = b.sync();
    assert!(
        b.non_trash_text().contains("draft two"),
        "device A sync: pushed {} (lists {:?}); device B sync: pulled {}; B has {:?}; B trash: {:?}",
        ra.pushed,
        listed(&a),
        rb.pulled,
        b.paths(),
        b.trash_text()
    );
}

/// Unicode twins (by design): both notes sync, the NFD one
/// under its twin name. Edits from the other device land in the files as
/// they are named on disk here, and nothing new is made next to them.
// Twins cannot exist on macOS.
#[cfg(not(target_os = "macos"))]
#[test]
fn fs02_twins_sync_in_place() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[]);
    a.write(NFC, "nfc\n");
    a.write(NFD, "nfd\n");
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    let twin = "caf\u{e9} (Unicode twin).md";
    let want = [(twin, "nfd\n"), (NFC, "nfc\n")];
    assert_eq!(b.files(), want.map(|(p, c)| (p.to_string(), c.to_string())));
    b.write(twin, "nfd, edited on the phone\n");
    b.sync();
    a.sync();
    assert_eq!(a.read(NFD).as_deref(), Some("nfd, edited on the phone\n"));
    assert_eq!(a.read(NFC).as_deref(), Some("nfc\n"));
    assert_eq!(a.paths().len(), 2, "{:?}", a.paths());
}
