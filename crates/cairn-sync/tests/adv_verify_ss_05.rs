//! Regression tests for FINDING-057 (file vs folder with the same name on
//! two devices used to stop sync on the receiving device). The first two
//! tests check that other files keep syncing; the characterisation tests
//! show how the clash itself is cleared.
//!
//!   cargo test -p cairn-sync --test adv_verify_ss_05

#[path = "adv_sync_semantics_common.rs"]
mod common;

use common::*;

/// A file "Projects" arrives on a device that has a folder "Projects/".
/// Later syncs must keep working: later remote edits arrive and local work
/// is pushed. Nothing is lost on disk.
#[test]
fn incoming_file_onto_local_folder_keeps_other_files_syncing() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("Projects", "a file called Projects\n"), ("shared.md", "v1\n")]);
    laptop.sync();
    let mut phone = Device::new(&srv, "phone", &[("Projects/plan.md", "plan\n")]);
    let mut errs = Vec::new();
    for i in 0..3 {
        if let Err(e) = phone.try_sync() {
            errs.push(format!("phone sync {i}: {e}"));
        }
    }
    // later remote change, sorted after the bad head
    laptop.write("shared.md", "v2 from laptop\n");
    laptop.sync();
    phone.write("from_phone.md", "phone work\n");
    if let Err(e) = phone.try_sync() {
        errs.push(format!("phone sync after more work: {e}"));
    }
    laptop.sync();
    // no data loss either way
    assert_eq!(phone.read("Projects/plan.md").as_deref(), Some("plan\n"));
    assert_eq!(laptop.read("Projects").as_deref(), Some("a file called Projects\n"));
    let phone_shared = phone.read("shared.md");
    let laptop_got = laptop.exists("from_phone.md");
    assert!(
        errs.is_empty() && phone_shared.as_deref() == Some("v2 from laptop\n") && laptop_got,
        "errors: {errs:#?}\nphone shared.md = {phone_shared:?}, laptop got from_phone.md = {laptop_got}\nphone has {:?}\nlaptop has {:?}",
        phone.paths(),
        laptop.paths()
    );
}

/// The reverse order: "Projects/plan.md" arrives on a device that has a
/// plain file "Projects". The parent folder cannot be created.
#[test]
fn incoming_folder_onto_local_file_keeps_other_files_syncing() {
    let srv = server();
    let mut phone = Device::new(&srv, "phone", &[("Projects/plan.md", "plan\n")]);
    phone.sync();
    let mut laptop = Device::new(&srv, "laptop", &[("Projects", "a file called Projects\n")]);
    let mut errs = Vec::new();
    for i in 0..3 {
        if let Err(e) = laptop.try_sync() {
            errs.push(format!("laptop sync {i}: {e}"));
        }
    }
    laptop.write("from_laptop.md", "laptop work\n");
    if let Err(e) = laptop.try_sync() {
        errs.push(format!("laptop sync after more work: {e}"));
    }
    phone.sync();
    let got = phone.exists("from_laptop.md");
    assert!(errs.is_empty() && got, "errors: {errs:#?}\nphone got from_laptop.md = {got}; phone has {:?}, laptop has {:?}", phone.paths(), laptop.paths());
}

/// Characterisation: the clashing file is reported and stays pending. Once
/// either side renames its clashing entry, the pending change applies on
/// the next sync.
#[test]
fn stall_clears_when_the_clashing_file_is_renamed_on_the_other_device() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("Projects", "a file called Projects\n")]);
    laptop.sync();
    let mut phone = Device::new(&srv, "phone", &[("Projects/plan.md", "plan\n"), ("other.md", "o\n")]);
    let skipped: Vec<String> = phone.sync().skipped.into_iter().map(|s| s.path).collect();
    assert_eq!(skipped, ["Projects"], "expected the clash to be reported");
    laptop.mv("Projects", "Projects.txt");
    laptop.sync();
    let r = phone.try_sync();
    assert!(r.as_ref().is_ok_and(|r| r.skipped.is_empty()), "phone still stuck after the laptop renamed its file: {:?}", r.map_err(|e| e.to_string()));
    laptop.sync();
    assert_eq!(phone.read("Projects.txt").as_deref(), Some("a file called Projects\n"));
    assert_eq!(laptop.read("Projects/plan.md").as_deref(), Some("plan\n"));
    assert_eq!(laptop.read("other.md").as_deref(), Some("o\n"));
}

/// Characterisation: the device with the pending change can also clear it
/// by renaming its own folder.
#[test]
fn stall_clears_when_the_stuck_device_renames_its_folder() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("Projects", "a file called Projects\n")]);
    laptop.sync();
    let mut phone = Device::new(&srv, "phone", &[("Projects/plan.md", "plan\n")]);
    let skipped: Vec<String> = phone.sync().skipped.into_iter().map(|s| s.path).collect();
    assert_eq!(skipped, ["Projects"], "expected the clash to be reported");
    phone.mv("Projects/plan.md", "Projects2/plan.md");
    std::fs::remove_dir(phone.abs("Projects")).unwrap();
    let r = phone.try_sync();
    assert!(r.as_ref().is_ok_and(|r| r.skipped.is_empty()), "phone still stuck after renaming its folder: {:?}", r.map_err(|e| e.to_string()));
    laptop.sync();
    assert_eq!(laptop.read("Projects2/plan.md").as_deref(), Some("plan\n"));
    assert_eq!(phone.read("Projects").as_deref(), Some("a file called Projects\n"));
}
