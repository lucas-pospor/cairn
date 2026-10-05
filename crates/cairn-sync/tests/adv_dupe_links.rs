//! Reproduction for FINDING-224 (related to FINDING-012): a symlink to a
//! folder that does not loop back (`alias -> notes` inside the vault, or two
//! links to one folder outside it) is listed under both names, as linked
//! folders keep showing by design, and sync uploads every copy.
//! The other device receives the copies as separate real files. When its
//! user deletes the duplicate under the link's name, this device applies the
//! delete through the link and moves the real note to the trash (that sync
//! then fails removing the emptied link: "alias: Not a directory"). The next
//! sync finds the note gone and uploads its delete too, so the note is lost
//! on every device.
//!
//! Expected: the real note stays on both devices, and only one copy of each
//! real file is synced.
//!
//!   cargo test -p cairn-sync --test adv_dupe_links -- --ignored --nocapture

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::path::Path;

use common::*;

const NOTE: &str = "the real note\n";

/// Makes the folder link `at -> target`. Returns false, and the test is
/// skipped, where links cannot be made (no symlinks on this platform, or
/// Windows without the privilege).
fn link_folder(target: &Path, at: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, at);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(target, at);
    #[cfg(not(any(unix, windows)))]
    let made: std::io::Result<()> = Err(std::io::ErrorKind::Unsupported.into());
    match made {
        Ok(()) => true,
        Err(e) => {
            eprintln!("skipped: cannot make a folder link here: {e}");
            false
        }
    }
}

fn copies_of_note(d: &Device) -> Vec<String> {
    d.files().into_iter().filter(|f| f.1 == NOTE).map(|f| f.0).collect()
}

/// Syncs the phone, then the laptop, three times (long enough for a delete
/// to travel back), and returns the errors instead of failing on them.
fn settle(phone: &mut Device, laptop: &mut Device) -> Vec<String> {
    let mut errors = Vec::new();
    for _ in 0..3 {
        for d in [&mut *phone, &mut *laptop] {
            if let Err(e) = d.sync() {
                errors.push(format!("{}: {e}", d.name));
            }
        }
    }
    println!("sync errors: {errors:?}");
    errors
}

#[test]
#[ignore = "FINDING-224: a linked duplicate folder deleted on another device trashes the real note through the link"]
fn deleting_the_link_copy_on_another_device_keeps_the_real_note() {
    let srv = server();
    // inbox.md keeps the vault from looking emptied (the FINDING-006 guard).
    let mut laptop = Device::new(&srv, "laptop", &[("notes/x.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    // A second name for a folder of the vault: not a loop, so it is listed.
    if !link_folder(Path::new("notes"), &laptop.root.join("alias")) {
        return;
    }
    laptop.restart();
    laptop.sync_ok();

    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    println!("phone received {:?}", phone.paths());
    // The phone's user sees the note twice and deletes the copy under the
    // link's name (with one copy per real file synced, there is none).
    if phone.read("alias/x.md").is_some() && phone.read("notes/x.md").is_some() {
        phone.rm("alias/x.md");
    }
    let errors = settle(&mut phone, &mut laptop);

    assert_eq!(
        (laptop.read("notes/x.md").as_deref(), phone.read("notes/x.md").as_deref()),
        (Some(NOTE), Some(NOTE)),
        "the real note notes/x.md is gone (laptop, phone): laptop trash {:?}, phone trash {:?}",
        laptop.trash(),
        phone.trash()
    );
    assert_eq!(phone.paths(), vec!["inbox.md".to_string(), "notes/x.md".to_string()], "one copy per real file");
    assert!(errors.is_empty(), "sync errors: {errors:?}");
}

#[test]
#[ignore = "FINDING-224: a linked duplicate folder deleted on another device trashes the real note through the link"]
fn deleting_one_of_two_links_to_an_outside_folder_on_another_device_keeps_the_real_note() {
    let srv = server();
    let outside = tempfile::tempdir().unwrap();
    let shared = outside.path().join("shared");
    fs::create_dir(&shared).unwrap();
    fs::write(shared.join("x.md"), NOTE).unwrap();
    let mut laptop = Device::new(&srv, "laptop", &[("inbox.md", "inbox\n")]);
    // One folder outside the vault, linked in under two names.
    if !link_folder(&shared, &laptop.root.join("projects")) || !link_folder(&shared, &laptop.root.join("work")) {
        return;
    }
    laptop.restart();
    laptop.sync_ok();

    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    println!("phone received {:?}", phone.paths());
    // The phone's user keeps one copy and deletes the other.
    let copies = copies_of_note(&phone);
    if copies.len() > 1 {
        phone.rm(copies.last().unwrap());
    }
    let errors = settle(&mut phone, &mut laptop);

    assert_eq!(
        (fs::read_to_string(shared.join("x.md")).ok().as_deref(), copies_of_note(&phone).len()),
        (Some(NOTE), 1),
        "(the real file outside the vault, copies on the phone): laptop trash {:?}, phone files {:?}, phone trash {:?}",
        laptop.trash(),
        phone.paths(),
        phone.trash()
    );
    assert!(errors.is_empty(), "sync errors: {errors:?}");
}
