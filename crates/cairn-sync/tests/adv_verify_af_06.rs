//! Regression tests for FINDING-084: one device replaces a folder with a file
//! of the same name (no concurrent clash). This was a cairn-sync defect, not
//! only an Android SAF one: a receiving desktop device that wrote the file
//! before the folder was gone failed every later sync ("Old: Is a
//! directory"). The pull now applies deletions first, so the folder is gone
//! before the file is written, and the file arrives in the same sync.
//!
//!   cargo test -p cairn-sync --test adv_verify_af_06

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::sync::Arc;

use common::*;
use parking_lot::Mutex;

#[test]
fn folder_replaced_by_file_on_one_device_arrives_on_the_other() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("Old/x.md", "x\n"), ("Seed.md", "s\n")]);
    laptop.sync();
    let mut desk2 = Device::new(&srv, "desk2", &[]);
    desk2.sync();
    assert_eq!(desk2.read("Old/x.md").as_deref(), Some("x\n"));

    // The laptop deletes the folder and creates a file with the same name.
    std::fs::remove_dir_all(laptop.abs("Old")).unwrap();
    laptop.write("Old", "now a file\n");
    laptop.sync();

    let mut errs = Vec::new();
    for i in 0..2 {
        if let Err(e) = desk2.try_sync() {
            errs.push(format!("desk2 sync {i}: {e}"));
        }
    }
    // An unrelated later change must still arrive.
    laptop.write("later.md", "later\n");
    laptop.sync();
    if let Err(e) = desk2.try_sync() {
        errs.push(format!("desk2 sync after a later change: {e}"));
    }
    let got_file = desk2.read("Old");
    let got_later = desk2.exists("later.md");
    assert!(
        errs.is_empty() && got_file.as_deref() == Some("now a file\n") && got_later,
        "errors: {errs:#?}\ndesk2 Old = {got_file:?}, later.md = {got_later}\ndesk2 has {:?}",
        desk2.paths()
    );
}

/// The remote batch for that change is "write Old" followed by "delete
/// Old/x.md". The delete (and the removal of the emptied folder) must be
/// applied first, so the file arrives in the same sync instead of waiting
/// as a pending change.
#[test]
fn folder_replaced_by_file_arrives_in_one_sync() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("Old/x.md", "x\n"), ("Seed.md", "s\n")]);
    laptop.sync();
    let mut desk2 = Device::new(&srv, "desk2", &[]);
    desk2.sync();

    std::fs::remove_dir_all(laptop.abs("Old")).unwrap();
    laptop.write("Old", "now a file\n");
    laptop.sync();

    let r = desk2.sync();
    assert!(r.skipped.is_empty(), "left out: {:?}", r.skipped);
    assert_eq!(desk2.read("Old").as_deref(), Some("now a file\n"), "desk2 has {:?}", desk2.paths());
    assert!(desk2.trash_text().contains("x\n"), "Old/x.md must go to the trash");
    assert_eq!(desk2.paths(), ["Old", "Seed.md"]);
}

/// Applying deletions first must not put an old head of a file after its
/// newer one. A file is listed twice when it changes while the changes
/// feed is being paged (500 heads a page): here A.md is on the first page,
/// and is deleted before the second page is fetched.
#[test]
fn file_deleted_while_the_feed_is_paged_stays_deleted() {
    let srv = server();
    let mut files: Vec<(String, String)> = (0..510).map(|i| (format!("f{i:03}.md"), format!("{i}\n"))).collect();
    files.push(("A.md".into(), "a\n".into()));
    let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let laptop = Arc::new(Mutex::new(Device::new(&srv, "laptop", &files)));
    laptop.lock().sync();
    let mut desk2 = Device::new(&srv, "desk2", &[]);
    let l = laptop.clone();
    *desk2.hooks.after_changes.lock() = Some(Box::new(move || {
        let mut l = l.lock();
        l.rm("A.md");
        l.sync();
    }));
    let r = desk2.sync();
    assert!(r.skipped.is_empty(), "left out: {:?}", r.skipped);
    assert!(!desk2.exists("A.md"), "A.md came back on desk2");
    assert_eq!(desk2.paths().len(), 510);
    let r = desk2.sync();
    assert_eq!((r.pulled, r.pushed), (0, 0));
    assert_eq!(laptop.lock().paths(), desk2.paths());
}
