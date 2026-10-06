//! FINDING-224 (related to FINDING-012): a symlink to a folder that does
//! not loop back (`alias -> notes` inside the vault, or two links to one
//! folder outside it) is listed under both names, as linked folders keep
//! showing by design. Sync used to upload every copy, and the other devices
//! got them as separate files. When a user there deleted the duplicate
//! under the link's name, this device applied the delete through the link
//! and moved the real note to the trash (that sync then failed removing
//! the emptied link: "alias: Not a directory"). The next sync found the
//! note gone and uploaded its delete too, so the note was lost on every
//! device.
//!
//! Now each file syncs under one name: the one a synced file has, else the
//! one with no link on the way, then the one with the fewest folders, then
//! the first in byte order. The copies that older versions synced under
//! the other names stay on the other devices: nothing is sent for them
//! again but their delete when the note is deleted here, a delete of one
//! is only recorded, an edit or rename of one waits and is listed, and
//! when the other copy is deleted elsewhere, the duplicate syncs on. No
//! remote change is applied through a name that is not synced, and an
//! emptied folder link is never removed.
//!
//!   cargo test -p cairn-sync --test adv_dupe_links -- --nocapture

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::path::Path;

use cairn_sync::engine::Skipped;
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

fn is_link(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

/// Copies the folder or file `from` to `to`.
fn copy_tree(from: &Path, to: &Path) {
    if from.is_dir() {
        fs::create_dir(to).unwrap();
        for e in fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            copy_tree(&e.path(), &to.join(e.file_name()));
        }
    } else {
        fs::copy(from, to).unwrap();
    }
}

/// Replaces the link at `link` with a copy of the folder or file it leads
/// to.
fn replace_with_copy(link: &Path) {
    let target = fs::canonicalize(link).unwrap();
    fs::remove_file(link).or_else(|_| fs::remove_dir(link)).unwrap();
    copy_tree(&target, link);
}

fn copies_of_note(d: &Device) -> Vec<String> {
    d.files().into_iter().filter(|f| f.1 == NOTE).map(|f| f.0).collect()
}

/// The laptop's vault has notes/x.md and a second name for that folder,
/// `alias -> notes` (not a loop, so it is listed), before its first sync;
/// a phone syncs after it. None where links cannot be made.
fn linked_folder(srv: &Server) -> Option<(Device, Device)> {
    // inbox.md keeps the vault from looking emptied (the FINDING-006 guard).
    let mut laptop = Device::new(srv, "laptop", &[("notes/x.md", NOTE), ("inbox.md", "inbox\n")]);
    if !link_folder(Path::new("notes"), &laptop.root.join("alias")) {
        return None;
    }
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(srv, "phone", &[]);
    phone.sync_ok();
    Some((laptop, phone))
}

/// As an older version left a vault with `alias -> notes`: both names of
/// x.md synced, as two files, and the phone has both. Made with a real
/// alias folder that is then replaced by the link. The laptop then syncs
/// once more with this version; its report is returned.
fn legacy_linked_folder(srv: &Server) -> Option<(Device, Device, cairn_sync::engine::SyncReport)> {
    let mut laptop = Device::new(srv, "laptop", &[("notes/x.md", NOTE), ("alias/x.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(srv, "phone", &[]);
    phone.sync_ok();
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md", "notes/x.md"]);
    fs::remove_dir_all(laptop.root.join("alias")).unwrap();
    if !link_folder(Path::new("notes"), &laptop.root.join("alias")) {
        return None;
    }
    laptop.restart();
    let upgrade = laptop.sync_ok();
    Some((laptop, phone, upgrade))
}

/// One folder outside the laptop's vault (`shared`, with x.md), linked in
/// under two names, `projects` and `work`. With `legacy`, both names were
/// synced first, as two files (made with real folders then replaced by the
/// links). A phone has synced. Returns the folder that holds `shared` too.
fn two_links_out(srv: &Server, legacy: bool) -> Option<(Device, Device, tempfile::TempDir)> {
    let outside = tempfile::tempdir().unwrap();
    let shared = outside.path().join("shared");
    fs::create_dir(&shared).unwrap();
    fs::write(shared.join("x.md"), NOTE).unwrap();
    let files: &[(&str, &str)] = if legacy { &[("projects/x.md", NOTE), ("work/x.md", NOTE), ("inbox.md", "inbox\n")] } else { &[("inbox.md", "inbox\n")] };
    let mut laptop = Device::new(srv, "laptop", files);
    let mut phone = Device::new(srv, "phone", &[]);
    if legacy {
        laptop.sync_ok();
        phone.sync_ok();
        fs::remove_dir_all(laptop.root.join("projects")).unwrap();
        fs::remove_dir_all(laptop.root.join("work")).unwrap();
    }
    if !link_folder(&shared, &laptop.root.join("projects")) || !link_folder(&shared, &laptop.root.join("work")) {
        return None;
    }
    laptop.restart();
    laptop.sync_ok();
    phone.sync_ok();
    Some((laptop, phone, outside))
}

/// Errors, what the laptop's last sync listed as not synced, and how many
/// changes the laptop uploaded.
struct Settled {
    errors: Vec<String>,
    skipped: Vec<Skipped>,
    pushed: usize,
}

impl Settled {
    /// Whether the laptop lists `path` as not synced with a reason that
    /// says `words`.
    fn listed(&self, path: &str, words: &str) -> bool {
        self.skipped.iter().any(|s| s.path == path && s.reason.contains(words))
    }

    fn clean(&self) -> bool {
        self.errors.is_empty() && self.skipped.is_empty()
    }
}

/// Syncs the phone, then the laptop, three times (long enough for a delete
/// to travel back), and returns the errors instead of failing on them.
fn settle(phone: &mut Device, laptop: &mut Device) -> Settled {
    let mut s = Settled { errors: Vec::new(), skipped: Vec::new(), pushed: 0 };
    for _ in 0..3 {
        if let Err(e) = phone.sync() {
            s.errors.push(format!("phone: {e}"));
        }
        match laptop.sync() {
            Ok(r) => {
                s.pushed += r.pushed;
                s.skipped = r.skipped;
            }
            Err(e) => s.errors.push(format!("laptop: {e}")),
        }
    }
    println!("sync errors: {:?}\nlaptop not synced: {:?}", s.errors, s.skipped);
    s
}

// ---------------------------------------------------------------- one copy per real file

#[test]
fn a_folder_reached_under_two_names_syncs_one_copy() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"], "one copy per real file, under the real folder's path");
    // Edits go both ways, made through either name here.
    laptop.write("alias/x.md", "edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.read("notes/x.md").as_deref(), Some("edited on the laptop\n"));
    phone.write("notes/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"]);
    // The history of either name is the note's.
    let (a, n) = (laptop.engine().history("alias/x.md").unwrap(), laptop.engine().history("notes/x.md").unwrap());
    assert_eq!(a.iter().map(|h| h.seq).collect::<Vec<_>>(), n.iter().map(|h| h.seq).collect::<Vec<_>>());
}

#[test]
fn two_links_to_an_outside_folder_sync_one_copy() {
    let srv = server();
    let Some((mut laptop, mut phone, outside)) = two_links_out(&srv, false) else { return };
    assert_eq!(phone.paths(), ["inbox.md", "projects/x.md"], "one copy per real file, the first name in byte order");
    phone.write("projects/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(fs::read_to_string(outside.path().join("shared/x.md")).unwrap(), "edited on the phone\n");
}

/// A link added to a folder that is synced already uploads nothing.
#[test]
fn a_link_added_after_a_sync_adds_no_copy() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("notes/x.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    if !link_folder(Path::new("notes"), &laptop.root.join("alias")) {
        return;
    }
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(s.pushed, 0);
    assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"]);
}

/// A note and a symlink to it sync once, under the note's own name.
#[cfg(unix)]
#[test]
fn a_symlinked_note_syncs_once() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("x.md", NOTE), ("inbox.md", "inbox\n")]);
    std::os::unix::fs::symlink("x.md", laptop.root.join("link.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    assert_eq!(phone.paths(), ["inbox.md", "x.md"]);
}

/// The phone, not having seen the laptop's note, uploads a note of its own
/// at a name that is, on the laptop, the other name of that note. Written
/// there, it would replace the note: it waits, listed, until it is renamed
/// on the phone.
#[test]
fn a_note_uploaded_elsewhere_at_the_link_name_waits() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("notes/x.md", NOTE), ("inbox.md", "inbox\n")]);
    if !link_folder(Path::new("notes"), &laptop.root.join("alias")) {
        return;
    }
    laptop.restart();
    let laptop = std::sync::Arc::new(parking_lot::Mutex::new(laptop));
    let mut phone = Device::new(&srv, "phone", &[("alias/x.md", "the phone's note\n")]);
    // The laptop's whole first sync happens between the phone's pull and push.
    let t = std::sync::Arc::new(FaultTransport::passthrough(http(&phone.url)));
    let l2 = laptop.clone();
    let mut puts = 0;
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "put" {
            puts += 1;
            if puts == 1 {
                l2.lock().sync_ok();
            }
        }
    }));
    phone.set_transport(Box::new(SharedTransport(t.clone())));
    phone.sync_ok();
    *t.before.lock() = None;
    let mut laptop = std::sync::Arc::try_unwrap(laptop).ok().unwrap().into_inner();
    let s = settle(&mut phone, &mut laptop);

    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.listed("alias/x.md", "another name of notes/x.md"), "not listed: {:?}", s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md", "notes/x.md"]);
    // Renamed there, it comes as a note of its own.
    phone.mv("alias/x.md", "other/x.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("other/x.md").as_deref(), Some("the phone's note\n"));
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
}

// ---------------------------------------------------------------- copies that older versions synced

/// The upgraded laptop sends nothing for the copy under the link's name;
/// it stays on the phone, no longer updated.
#[test]
fn an_upgrade_sends_nothing_for_a_copy_synced_before() {
    let srv = server();
    let Some((mut laptop, mut phone, upgrade)) = legacy_linked_folder(&srv) else { return };
    assert_eq!((upgrade.pushed, upgrade.skipped.clone()), (0, vec![]), "{upgrade:?}");
    laptop.write("notes/x.md", "edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(s.pushed, 1);
    assert_eq!(phone.read("notes/x.md").as_deref(), Some("edited on the laptop\n"));
    assert_eq!(phone.read("alias/x.md").as_deref(), Some(NOTE));
}

#[test]
fn deleting_the_copy_under_the_link_name_on_another_device_keeps_the_note() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.rm("alias/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!((laptop.read("notes/x.md").as_deref(), phone.read("notes/x.md").as_deref()), (Some(NOTE), Some(NOTE)));
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
    assert!(is_link(&laptop.root.join("alias")), "the link is gone");
    assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"]);
}

/// The phone keeps the copy under the link's name and deletes the other:
/// the laptop's note syncs on as that copy, and nothing is trashed.
#[test]
fn deleting_the_copy_under_the_real_name_on_another_device_keeps_the_note() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md"]);
    laptop.write("notes/x.md", "edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.read("alias/x.md").as_deref(), Some("edited on the laptop\n"));
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md"]);
}

/// An edit of the older copy is not written into the note: it waits,
/// listed, until the user copies it into the note on the phone and deletes
/// the copy there, as the reason says.
#[test]
fn editing_the_copy_under_the_link_name_on_another_device_waits() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.write("alias/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.listed("alias/x.md", "an older copy of notes/x.md"), "not listed: {:?}", s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(phone.read("notes/x.md").as_deref(), Some(NOTE));
    phone.write("notes/x.md", "edited on the phone\n");
    phone.rm("alias/x.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"]);
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
}

#[test]
fn renaming_the_copy_under_the_link_name_on_another_device_waits() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.mv("alias/x.md", "alias/y.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.listed("alias/y.md", "(alias/x.md here), an older copy of notes/x.md"), "not listed: {:?}", s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.read("notes/y.md"), None);
    phone.rm("alias/y.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
}

/// The note's own edits from another device are applied, though its older
/// copy is the same file here.
#[test]
fn editing_the_note_on_another_device_applies_while_an_older_copy_exists() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.write("notes/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(phone.read("alias/x.md").as_deref(), Some(NOTE));
    assert_eq!(s.pushed, 0);
}

/// Deleting the note here deletes it everywhere, its older copy too.
#[test]
fn deleting_the_note_here_deletes_its_older_copy_too() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert_eq!(phone.trash().len(), 2, "{:?}", phone.trash());
}

/// Removing the link sends nothing: the phone keeps its older copy, and a
/// change of it still waits.
#[test]
fn removing_the_link_sends_nothing() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    fs::remove_file(laptop.root.join("alias")).or_else(|_| fs::remove_dir(laptop.root.join("alias"))).unwrap();
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(s.pushed, 0);
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md", "notes/x.md"]);
    phone.write("alias/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.listed("alias/x.md", "which this device does not keep"), "not listed: {:?}", s.skipped);
    assert!(!laptop.root.join("alias").exists());
}

/// Without the link, the note here has one name. When the phone deletes
/// that copy and keeps the older one, the note here goes to the trash as
/// on any device, and the older copy comes down as a file of its own.
#[test]
fn deleting_the_note_elsewhere_after_the_link_was_removed_brings_down_the_older_copy() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    fs::remove_file(laptop.root.join("alias")).or_else(|_| fs::remove_dir(laptop.root.join("alias"))).unwrap();
    laptop.sync_ok();
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.files(), phone.files());
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md"]);
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.trash(), [("x.md".to_string(), NOTE.to_string())]);
}

/// With the link replaced by a copy, the older copy is a file of its own
/// again: it syncs, and the change that waited applies to it.
#[test]
fn replacing_the_link_with_a_copy_syncs_the_older_copy_again() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.write("alias/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.listed("alias/x.md", "an older copy of notes/x.md"), "not listed: {:?}", s.skipped);
    replace_with_copy(&laptop.root.join("alias"));
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.files(), phone.files());
}

/// A note made on the phone in its own copy of the linked folder syncs
/// under that name: the laptop writes it through the link and uploads no
/// second copy.
#[test]
fn a_note_made_elsewhere_in_the_folder_under_the_link_name_syncs_once() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.write("alias/new.md", "new\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(s.pushed, 0);
    assert_eq!(laptop.read("notes/new.md").as_deref(), Some("new\n"));
    assert_eq!(phone.paths(), ["alias/new.md", "alias/x.md", "inbox.md", "notes/x.md"]);
    laptop.write("notes/new.md", "new, edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.read("alias/new.md").as_deref(), Some("new, edited on the laptop\n"));
    assert_eq!(phone.paths(), ["alias/new.md", "alias/x.md", "inbox.md", "notes/x.md"]);
}

/// Two links to one folder outside the vault, both synced before. Whichever
/// copy the phone deletes, the file outside stays, and nothing is trashed.
#[test]
fn deleting_either_copy_of_two_links_out_on_another_device_keeps_the_note() {
    for (deleted, kept) in [("work/x.md", "projects/x.md"), ("projects/x.md", "work/x.md")] {
        let srv = server();
        let Some((mut laptop, mut phone, outside)) = two_links_out(&srv, true) else { return };
        let shared = outside.path().join("shared/x.md");
        assert_eq!(phone.paths(), ["inbox.md", "projects/x.md", "work/x.md"]);
        phone.rm(deleted);
        let s = settle(&mut phone, &mut laptop);

        assert!(s.clean(), "deleting {deleted}: {:?} {:?}", s.errors, s.skipped);
        assert_eq!(fs::read_to_string(&shared).unwrap(), NOTE, "deleting {deleted}");
        assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]), "deleting {deleted}");
        assert!(is_link(&laptop.root.join("projects")) && is_link(&laptop.root.join("work")));
        assert_eq!(phone.paths(), ["inbox.md", kept]);
        fs::write(&shared, "edited on the laptop\n").unwrap();
        let s = settle(&mut phone, &mut laptop);
        assert!(s.clean(), "deleting {deleted}: {:?} {:?}", s.errors, s.skipped);
        assert_eq!(phone.read(kept).as_deref(), Some("edited on the laptop\n"), "deleting {deleted}");
    }
}

/// The link was renamed outside the app since the last sync, so its copy
/// is found under the new name: it still keeps the note when the phone
/// deletes the other copy.
#[test]
fn deleting_the_real_name_copy_after_the_link_was_renamed_keeps_the_note() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.mv("alias", "links");
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert!(is_link(&laptop.root.join("links")));
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
    assert_eq!(phone.paths(), ["inbox.md", "links/x.md"]);
}

/// A note renamed in the app takes the phone's edit, and its older copy
/// stays as it was.
#[test]
fn a_note_renamed_here_takes_a_remote_edit_while_an_older_copy_exists() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.app_mv("notes/x.md", "notes/y.md");
    phone.write("notes/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/y.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(phone.read("notes/y.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md", "notes/y.md"]);
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
}

/// A note and a symlink to it, both synced before: deleting the note on
/// the phone keeps it here, where it syncs on as the other copy.
#[cfg(unix)]
#[test]
fn deleting_a_linked_note_on_another_device_keeps_it_here() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("x.md", NOTE), ("link.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    fs::remove_file(laptop.root.join("link.md")).unwrap();
    std::os::unix::fs::symlink("x.md", laptop.root.join("link.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    phone.rm("x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("x.md").as_deref(), Some(NOTE));
    assert!(is_link(&laptop.root.join("link.md")));
    assert_eq!(phone.paths(), ["inbox.md", "link.md"]);
    assert_eq!(laptop.trash(), vec![]);
}

/// The link is in a folder that the laptop cannot read any more. Its copy
/// is unknown, not gone: when the phone deletes the other copy, the delete
/// waits, listed, until the folder can be read; then the note stays, and
/// syncs on as the copy the phone kept. Also after a restart, and when
/// this version first syncs while the folder cannot be read.
#[cfg(unix)]
#[test]
fn deleting_the_real_name_copy_keeps_the_note_while_the_link_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    for (restart, upgrade_locked) in [(false, false), (true, false), (true, true)] {
        let case = format!("restart {restart}, upgrade while locked {upgrade_locked}");
        let srv = server();
        let mut laptop = Device::new(&srv, "laptop", &[("notes/x.md", NOTE), ("private/link/x.md", NOTE), ("inbox.md", "inbox\n")]);
        laptop.sync_ok();
        let mut phone = Device::new(&srv, "phone", &[]);
        phone.sync_ok();
        assert_eq!(phone.paths(), ["inbox.md", "notes/x.md", "private/link/x.md"]);
        fs::remove_dir_all(laptop.root.join("private/link")).unwrap();
        std::os::unix::fs::symlink("../notes", laptop.root.join("private/link")).unwrap();
        laptop.restart();
        if !upgrade_locked {
            laptop.sync_ok();
        }
        phone.rm("notes/x.md");
        phone.sync_ok();
        let private = laptop.root.join("private");
        fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&private).is_ok() {
            fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
            return; // running as root
        }
        let s = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if restart {
                laptop.restart();
            }
            settle(&mut phone, &mut laptop)
        }));
        fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
        let s = s.unwrap_or_else(|e| std::panic::resume_unwind(e));
        let note = NOTE;

        assert_eq!(laptop.read("notes/x.md").as_deref(), Some(note), "{case}");
        assert_eq!(laptop.trash(), vec![], "{case}");
        assert!(s.errors.is_empty(), "{case}: sync errors: {:?}", s.errors);
        assert!(s.listed("notes/x.md", "may be one file with private/link/x.md"), "{case}: not listed: {:?}", s.skipped);
        // Readable again, it is the link it was.
        let s = settle(&mut phone, &mut laptop);
        assert!(s.clean(), "{case}: {:?} {:?}", s.errors, s.skipped);
        assert_eq!(laptop.read("notes/x.md").as_deref(), Some(note), "{case}");
        assert_eq!(phone.paths(), ["inbox.md", "private/link/x.md"], "{case}");
        assert_eq!(phone.read("private/link/x.md").as_deref(), Some(note), "{case}");
    }
}

/// The phone deletes both copies after the note changed (the older copy's
/// last version is older than the note here): the note goes to the trash
/// here, as in 1.1, and does not come back. Also when the phone deletes
/// them in two syncs.
#[test]
fn deleting_both_copies_on_another_device_deletes_the_note() {
    for two_links in [true, false] {
        let srv = server();
        let (mut laptop, mut phone, outside, names) = if two_links {
            let Some((l, p, o)) = two_links_out(&srv, true) else { return };
            (l, p, Some(o), ["projects/x.md", "work/x.md"])
        } else {
            let Some((l, p, _)) = legacy_linked_folder(&srv) else { return };
            (l, p, None, ["notes/x.md", "alias/x.md"])
        };
        laptop.write(if two_links { "projects/x.md" } else { "notes/x.md" }, "edited on the laptop\n");
        let s = settle(&mut phone, &mut laptop);
        assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
        for name in names {
            phone.rm(name);
            if !two_links {
                phone.sync_ok();
            }
        }
        let s = settle(&mut phone, &mut laptop);

        assert!(s.clean(), "two links {two_links}: {:?} {:?}", s.errors, s.skipped);
        assert_eq!(phone.paths(), ["inbox.md"], "two links {two_links}");
        // (The emptied notes folder is removed, so the alias link dangles.)
        assert_eq!(laptop.files().into_iter().filter(|f| f.0.ends_with("/x.md")).count(), 0, "two links {two_links}: {:?}", laptop.files());
        assert_eq!(laptop.trash(), [("x.md".to_string(), "edited on the laptop\n".to_string())], "two links {two_links}");
        if let Some(o) = outside {
            assert!(!o.path().join("shared/x.md").exists());
        }
    }
}

/// The note is deleted here while the phone has deleted its copy under the
/// real name already: the phone's older copy goes too, and the note does
/// not come back here.
#[test]
fn deleting_the_note_here_after_another_device_deleted_its_copy_deletes_the_older_copy() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.rm("notes/x.md");
    phone.rm("notes/x.md");
    phone.sync_ok();
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert_eq!(laptop.paths(), ["inbox.md"]);
}

/// The note is deleted here, and the sync stops partway through its
/// uploads (the older copy's delete goes first). The next sync sends the
/// rest, and the note does not come back.
#[test]
fn deletes_cut_off_partway_are_sent_on_the_next_sync() {
    let srv = server();
    let Some((mut laptop, mut phone, outside)) = two_links_out(&srv, true) else { return };
    fs::remove_file(outside.path().join("shared/x.md")).unwrap();
    let mut puts = 0;
    let t = FaultTransport::new(
        http(&laptop.url),
        Box::new(move |op, _| {
            if op == "put" {
                puts += 1;
                if puts == 2 {
                    return Fault::ErrBefore;
                }
            }
            Fault::None
        }),
    );
    laptop.set_transport(Box::new(t));
    assert!(laptop.sync().is_err(), "the second upload did not fail");
    laptop.set_transport(http(&laptop.url));
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert!(!outside.path().join("shared/x.md").exists(), "the note came back");
}

/// Another device edited the older copy, and the note is then deleted
/// here: the edit wins, as an edit of any file deleted here does.
#[test]
fn an_edit_of_the_older_copy_wins_over_deleting_the_note_here() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    phone.write("alias/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.listed("alias/x.md", "an older copy of notes/x.md"), "not listed: {:?}", s.skipped);
    laptop.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md"]);
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(phone.read("alias/x.md").as_deref(), Some("edited on the phone\n"));
}

/// The link was removed here, the phone edited its older copy, then
/// deleted the note: the edited copy comes down as a file of its own.
#[test]
fn an_edited_older_copy_comes_down_when_the_note_is_deleted_after_the_link_was_removed() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    fs::remove_file(laptop.root.join("alias")).or_else(|_| fs::remove_dir(laptop.root.join("alias"))).unwrap();
    phone.write("alias/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.listed("alias/x.md", "which this device does not keep"), "not listed: {:?}", s.skipped);
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.files(), phone.files());
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some("edited on the phone\n"));
}

/// The phone tidies up: it deletes the older copy and gives the note the
/// copy's name. Here that name is the same file already: nothing moves,
/// and no conflict copy is made.
#[test]
fn renaming_the_note_to_its_other_name_elsewhere_moves_nothing_here() {
    for two_links in [false, true] {
        let srv = server();
        let (mut laptop, mut phone, outside, (from, to)) = if two_links {
            let Some((l, p, o)) = two_links_out(&srv, true) else { return };
            (l, p, Some(o), ("projects/x.md", "work/x.md"))
        } else {
            let Some((l, p, _)) = legacy_linked_folder(&srv) else { return };
            (l, p, None, ("notes/x.md", "alias/x.md"))
        };
        phone.rm(to);
        phone.sync_ok();
        phone.mv(from, to);
        let s = settle(&mut phone, &mut laptop);

        assert!(s.clean(), "two links {two_links}: {:?} {:?}", s.errors, s.skipped);
        assert_eq!(phone.paths(), ["inbox.md", to].iter().copied().collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>());
        assert_eq!(laptop.read(to).as_deref(), Some(NOTE));
        assert_eq!(laptop.read(from).as_deref(), Some(NOTE));
        assert!(laptop.paths().iter().all(|p| !p.contains("conflict")), "{:?}", laptop.paths());
        assert!(phone.paths().iter().all(|p| !p.contains("conflict")), "{:?}", phone.paths());
        if let Some(o) = &outside {
            assert_eq!(fs::read_to_string(o.path().join("shared/x.md")).unwrap(), NOTE);
        }
        // It syncs under that name from now on.
        laptop.write(from, "edited on the laptop\n");
        let s = settle(&mut phone, &mut laptop);
        assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
        assert_eq!(phone.read(to).as_deref(), Some("edited on the laptop\n"));
    }
}

/// The note changed since the older copy was synced, then the link was
/// renamed here: the scan cannot tell the copy by its content, but it is
/// still found under the new name and keeps the note when the phone
/// deletes the other copy.
#[test]
fn an_older_copy_is_found_under_a_renamed_link_after_the_note_changed() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.write("notes/x.md", "edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    laptop.mv("alias", "links");
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the laptop\n"));
    assert!(is_link(&laptop.root.join("links")));
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
    assert_eq!(phone.paths(), ["inbox.md", "links/x.md"]);
    assert_eq!(phone.read("links/x.md").as_deref(), Some("edited on the laptop\n"));
}

/// A second device with the same link connects to a server that has both
/// copies: it takes them as they are, uploads nothing, and records a
/// delete of the older copy.
#[test]
fn a_second_device_with_the_link_takes_both_copies_as_they_are() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    let mut desk = Device::new(&srv, "desk", &[("notes/x.md", NOTE), ("inbox.md", "inbox\n")]);
    if !link_folder(Path::new("notes"), &desk.root.join("alias")) {
        return;
    }
    desk.restart();
    let r = desk.sync_ok();
    assert_eq!((r.pushed, r.skipped.clone(), r.conflicts.clone()), (0, vec![], vec![]), "{r:?}");
    phone.rm("alias/x.md");
    for _ in 0..2 {
        phone.sync_ok();
        laptop.sync_ok();
        let r = desk.sync_ok();
        assert_eq!(r.skipped, vec![]);
    }
    for d in [&laptop, &desk] {
        assert_eq!(d.read("notes/x.md").as_deref(), Some(NOTE), "{}", d.name);
        assert_eq!(d.trash(), vec![], "{}", d.name);
    }
    assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"]);
}

/// A device that still runs 1.1 with the same link uploads both names
/// whenever the note changes, the older copy first, with the note's
/// content: the copy's version is recorded once the note's is in, not
/// listed.
#[test]
fn both_names_uploaded_again_with_the_same_content_are_recorded() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    // As such a device would: the same text under both names, in one sync.
    phone.write("notes/x.md", "edited on the phone\n");
    phone.write("alias/x.md", "edited on the phone\n");
    phone.sync_ok();
    let r = laptop.sync_ok();
    assert_eq!((r.skipped.clone(), r.pushed), (vec![], 0), "{r:?}");
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the phone\n"));
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
}

/// The note was moved out of the linked folder here, so its older copy's
/// name is gone. When the phone then deletes the moved note and keeps the
/// older copy, that copy comes down here as a file of its own: the phone's
/// user kept it, and nothing deletes it.
#[test]
fn an_older_copy_kept_elsewhere_stays_when_the_note_moved_away_from_the_link_is_deleted() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.app_mv("notes/x.md", "archive/x.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["alias/x.md", "archive/x.md", "inbox.md"]);
    phone.rm("archive/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md"]);
    assert_eq!(phone.trash(), vec![]);
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some(NOTE));
}

/// A note and a symlink to it, both synced before; the phone deleted the
/// note's own copy, so the copy under the link's name syncs on here. When
/// the phone deletes that one too, the note goes, not only the link.
#[cfg(unix)]
#[test]
fn deleting_the_last_copy_of_a_linked_note_deletes_the_note_not_only_the_link() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("x.md", NOTE), ("link.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    fs::remove_file(laptop.root.join("link.md")).unwrap();
    std::os::unix::fs::symlink("x.md", laptop.root.join("link.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    phone.rm("x.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    phone.rm("link.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert_eq!(laptop.read("x.md"), None);
    assert_eq!(laptop.files(), phone.files());
}

/// The phone deletes both copies after the note changed, and the laptop's
/// pull takes them in two batches: the copy that took the note's place in
/// the first is behind on the server, not edited, so its delete in the
/// second applies.
#[test]
fn deleting_both_copies_in_two_pull_batches_deletes_the_note() {
    let srv = server();
    let Some((mut laptop, mut phone, outside)) = two_links_out(&srv, true) else { return };
    fs::write(outside.path().join("shared/x.md"), "edited on the laptop\n").unwrap();
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    phone.rm("projects/x.md");
    phone.rm("work/x.md");
    phone.sync_ok();
    laptop.pull_in_batches(1, 1);
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert!(!outside.path().join("shared/x.md").exists(), "the note was kept");
}

/// A second device with the same link and an empty folder behind it
/// connects after the note changed: the older copy, uploaded first, is
/// not written through the link before the note, and nothing gets a
/// conflict copy name.
#[test]
fn a_new_device_with_the_link_writes_the_note_not_its_older_copy() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.write("notes/x.md", "edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    for batches in [false, true] {
        let mut desk = Device::new(&srv, "desk", &[("inbox.md", "inbox\n")]);
        fs::create_dir(desk.root.join("notes")).unwrap();
        if !link_folder(Path::new("notes"), &desk.root.join("alias")) {
            return;
        }
        desk.restart();
        if batches {
            desk.pull_in_batches(1, 1);
        }
        let r = desk.sync_ok();

        assert_eq!(r.conflicts, Vec::<String>::new(), "batches {batches}");
        assert_eq!(desk.read("notes/x.md").as_deref(), Some("edited on the laptop\n"), "batches {batches}");
        assert!(r.skipped.iter().any(|x| x.path == "alias/x.md" && x.reason.contains("another name of notes/x.md")), "batches {batches}: {:?}", r.skipped);
    }
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md", "notes/x.md"]);
}

/// The link device loses its sync state after the note changed: the
/// older copy waits, listed, and while it does, a delete of the note from
/// another device waits too, rather than replace the note with that copy.
#[test]
fn after_a_lost_state_a_delete_of_the_note_waits_for_its_older_copy() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.write("notes/x.md", "edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    for f in ["state.json", "state.journal"] {
        let _ = fs::remove_file(laptop.state_dir.join(f));
    }
    laptop.restart();
    let r = laptop.sync_ok();
    assert!(r.skipped.iter().any(|x| x.path == "alias/x.md"), "{:?}", r.skipped);
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.errors.is_empty(), "{:?}", s.errors);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the laptop\n"));
    assert!(s.listed("notes/x.md", "waits to sync at alias/x.md"), "{:?}", s.skipped);
}

/// A copy of the note made after the link was removed has the older copy's
/// content: it is a new note, not the older copy found renamed.
#[test]
fn a_copy_made_after_the_link_was_removed_syncs_as_a_new_note() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    fs::remove_file(laptop.root.join("alias")).or_else(|_| fs::remove_dir(laptop.root.join("alias"))).unwrap();
    laptop.write("notes/x copy.md", NOTE);
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.read("notes/x copy.md").as_deref(), Some(NOTE));
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md", "notes/x copy.md", "notes/x.md"]);
}

/// The link was renamed outside the app; the phone then deletes the
/// note's own copy and edits the older one. The older copy, found under
/// the new name, keeps the note, takes the edit, and is renamed on the
/// phone to the name it has here.
#[test]
fn an_older_copy_under_a_renamed_link_keeps_the_note_and_takes_its_edit() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.mv("alias", "links");
    phone.rm("notes/x.md");
    phone.write("alias/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the phone\n"));
    assert!(!laptop.root.join("alias").exists(), "a real alias folder was made");
    assert_eq!(phone.paths(), ["inbox.md", "links/x.md"]);
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
}

/// The same, with the note changed before and both copies deleted on the
/// phone (the note's own first), taken by the laptop in two pull batches:
/// the older copy takes the note's place in the first, and its delete in
/// the second applies, though its last version is older and it was found
/// under a new name.
#[test]
fn deleting_both_copies_after_the_link_was_renamed_deletes_the_note() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    laptop.write("notes/x.md", "edited on the laptop\n");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    laptop.mv("alias", "links");
    phone.rm("notes/x.md");
    phone.sync_ok();
    phone.rm("alias/x.md");
    phone.sync_ok();
    laptop.pull_in_batches(1, 1);
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert_eq!(laptop.read("notes/x.md"), None);
}

/// The note is deleted here while the folder that holds its older copy's
/// link cannot be read: once it can, the older copy's delete follows, and
/// the note does not come back.
#[cfg(unix)]
#[test]
fn deleting_the_note_while_the_link_cannot_be_read_deletes_the_older_copy_later() {
    use std::os::unix::fs::PermissionsExt;
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("notes/x.md", NOTE), ("private/link/x.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    fs::remove_dir_all(laptop.root.join("private/link")).unwrap();
    std::os::unix::fs::symlink("../notes", laptop.root.join("private/link")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    let private = laptop.root.join("private");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&private).is_ok() {
        fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
        return; // running as root
    }
    laptop.rm("notes/x.md");
    let s = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| settle(&mut phone, &mut laptop)));
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    let s = s.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(s.errors.is_empty(), "{:?}", s.errors);
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert_eq!(laptop.read("notes/x.md"), None);
}

/// A device on 1.1 with the same link renames the note: it uploads the
/// rename of both names. The older copy's rename, to the name it has here
/// now, is recorded, not listed.
#[test]
fn both_names_renamed_elsewhere_by_an_older_link_device_are_recorded() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    // As such a device would: both names renamed in one sync.
    phone.mv("notes/x.md", "notes/y.md");
    phone.mv("alias/x.md", "alias/y.md");
    phone.sync_ok();
    let r = laptop.sync_ok();
    assert_eq!(r.skipped, vec![], "{r:?}");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/y.md").as_deref(), Some(NOTE));
    assert_eq!(s.pushed, 0);
}

/// The link was removed here before the note was deleted here: the copy
/// on the phone is no copy that a link here makes any more, so it stays,
/// and comes down here as a file of its own.
#[test]
fn deleting_the_note_after_the_link_was_removed_keeps_the_older_copy() {
    let srv = server();
    let Some((mut laptop, mut phone, _)) = legacy_linked_folder(&srv) else { return };
    fs::remove_file(laptop.root.join("alias")).or_else(|_| fs::remove_dir(laptop.root.join("alias"))).unwrap();
    laptop.sync_ok();
    laptop.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["alias/x.md", "inbox.md"]);
    assert_eq!(laptop.files(), phone.files());
}

/// A promoted symlink to a note, moved on the phone into another folder:
/// moving the link here would break its relative target, so the move
/// waits, listed, and nothing changes here.
#[cfg(unix)]
#[test]
fn moving_a_synced_relative_link_into_another_folder_waits() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("x.md", NOTE), ("link.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    fs::remove_file(laptop.root.join("link.md")).unwrap();
    std::os::unix::fs::symlink("x.md", laptop.root.join("link.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    phone.rm("x.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    phone.mv("link.md", "sub/link.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.errors.is_empty(), "{:?}", s.errors);
    assert!(s.listed("sub/link.md", "relative path"), "{:?}", s.skipped);
    assert!(is_link(&laptop.root.join("link.md")));
    assert_eq!(laptop.read("link.md").as_deref(), Some(NOTE));
}

/// This version first syncs while the folder that holds the link cannot
/// be read, and the note is edited and synced then: the copy there no
/// longer has the note's content, but has its name, so a delete of the
/// note's own copy from another device still waits until it can be read.
#[cfg(unix)]
#[test]
fn a_delete_waits_for_a_copy_with_the_note_s_name_in_a_folder_that_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("notes/x.md", NOTE), ("private/link/x.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    fs::remove_dir_all(laptop.root.join("private/link")).unwrap();
    std::os::unix::fs::symlink("../notes", laptop.root.join("private/link")).unwrap();
    let private = laptop.root.join("private");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&private).is_ok() {
        fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
        return; // running as root
    }
    let s = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        laptop.restart();
        laptop.write("notes/x.md", "edited on the laptop\n");
        let first = settle(&mut phone, &mut laptop);
        phone.rm("notes/x.md");
        (first, settle(&mut phone, &mut laptop))
    }));
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    let (first, s) = s.unwrap_or_else(|e| std::panic::resume_unwind(e));

    assert!(first.errors.is_empty() && s.errors.is_empty(), "{:?} {:?}", first.errors, s.errors);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the laptop\n"));
    assert!(s.listed("notes/x.md", "may be one file with private/link/x.md"), "{:?}", s.skipped);
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the laptop\n"));
    assert_eq!(phone.read("private/link/x.md").as_deref(), Some("edited on the laptop\n"));
}

/// A note and a symlink to it, both synced before. The phone deleted the
/// note's own copy (the link's copy syncs on here), then brought it back,
/// then deletes both: the note goes, under both names, and stays gone.
#[cfg(unix)]
#[test]
fn deleting_both_names_of_a_linked_note_after_one_came_back_deletes_the_note() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("x.md", NOTE), ("link.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    fs::remove_file(laptop.root.join("link.md")).unwrap();
    std::os::unix::fs::symlink("x.md", laptop.root.join("link.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    phone.rm("x.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    phone.write("x.md", NOTE);
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    phone.rm("link.md");
    phone.rm("x.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert_eq!(laptop.read("x.md"), None);
}

/// Two symlinks to one note outside the vault, both synced before: when
/// the phone deletes both, both links go here, and the note does not come
/// back.
#[cfg(unix)]
#[test]
fn deleting_both_links_to_an_outside_note_on_another_device_removes_both() {
    let srv = server();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("n.md"), NOTE).unwrap();
    let mut laptop = Device::new(&srv, "laptop", &[("a.md", NOTE), ("b.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    for name in ["a.md", "b.md"] {
        fs::remove_file(laptop.root.join(name)).unwrap();
        std::os::unix::fs::symlink(outside.path().join("n.md"), laptop.root.join(name)).unwrap();
    }
    laptop.restart();
    laptop.sync_ok();
    phone.rm("a.md");
    phone.rm("b.md");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(phone.paths(), ["inbox.md"]);
    assert_eq!(laptop.paths(), ["inbox.md"]);
}

// ---------------------------------------------------------------- links in general

/// A remote delete through a folder linked in once is applied, as before
/// (sync follows links), and the emptied link stays: removing it as a
/// folder failed with "Not a directory" and held the delete.
#[test]
fn a_delete_through_a_folder_linked_in_once_keeps_the_link() {
    let srv = server();
    let outside = tempfile::tempdir().unwrap();
    let shared = outside.path().join("shared");
    fs::create_dir(&shared).unwrap();
    fs::write(shared.join("x.md"), NOTE).unwrap();
    let mut laptop = Device::new(&srv, "laptop", &[("inbox.md", "inbox\n")]);
    if !link_folder(&shared, &laptop.root.join("projects")) {
        return;
    }
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    phone.rm("projects/x.md");
    phone.sync_ok();
    let r = laptop.sync_ok();

    assert_eq!(r.skipped, vec![], "the delete was held");
    assert!(!shared.join("x.md").exists());
    assert_eq!(laptop.trash(), [("x.md".to_string(), NOTE.to_string())]);
    assert!(is_link(&laptop.root.join("projects")) && shared.is_dir(), "the link or its folder is gone");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.files(), phone.files());
    assert_eq!(copies_of_note(&phone), Vec::<String>::new());
}

/// Two hard links are two files, not one under two names: a change through
/// one name never deletes or moves the other, and a write keeps both, so
/// nothing waits. An edit of one on another device reaches both here, and
/// from here the other one's copy elsewhere.
#[cfg(unix)]
#[test]
fn hard_links_sync_as_two_notes() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("a.md", NOTE), ("inbox.md", "inbox\n")]);
    fs::hard_link(laptop.root.join("a.md"), laptop.root.join("b.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    assert_eq!(phone.paths(), ["a.md", "b.md", "inbox.md"]);
    phone.write("b.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    for d in [&laptop, &phone] {
        assert_eq!((d.read("a.md").as_deref(), d.read("b.md").as_deref()), (Some("edited on the phone\n"), Some("edited on the phone\n")), "{}", d.name);
    }
    phone.rm("a.md");
    let s = settle(&mut phone, &mut laptop);
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.read("b.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(laptop.files(), phone.files());
}

/// A remote delete is applied even when the folder it empties cannot be
/// removed (here: no write permission on the folder above it). The folder
/// stays; the delete is not held, nor listed, sync after sync.
#[cfg(unix)]
#[test]
fn a_delete_whose_emptied_folder_cannot_be_removed_is_applied() {
    use std::os::unix::fs::PermissionsExt;
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("locked/sub/x.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    phone.rm("locked/sub/x.md");
    phone.sync_ok();
    let locked = laptop.root.join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::write(locked.join("probe"), "").is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        return; // running as root
    }
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (laptop.sync(), settle(&mut phone, &mut laptop))));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let (first, s) = run.unwrap_or_else(|e| std::panic::resume_unwind(e));

    let first = first.unwrap();
    assert_eq!(first.skipped, vec![], "the delete was held");
    assert_eq!(laptop.trash(), [("x.md".to_string(), NOTE.to_string())]);
    assert!(laptop.root.join("locked/sub").is_dir());
    assert!(s.clean(), "{:?} {:?}", s.errors, s.skipped);
}
