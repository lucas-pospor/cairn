//! FINDING-224 (related to FINDING-012): a symlink to a folder that does
//! not loop back (`alias -> notes` inside the vault, or two links to one
//! folder outside it) is listed under both names, as linked folders keep
//! showing by design, and sync uploads every copy. The other devices
//! receive the copies as separate real files. When a user there deleted
//! the duplicate under the link's name, this device applied the delete
//! through the link and moved the real note to the trash (that sync then
//! failed removing the emptied link: "alias: Not a directory"). The next
//! sync found the note gone and uploaded its delete too, so the note was
//! lost on every device.
//!
//! Now no remote delete, rename or write is applied through a path whose
//! file is also synced under another name here: the change waits and is
//! listed under "Files not synced", and the real note stays on every
//! device. Replacing the link with a copy of what it leads to lets the
//! change apply to its own copy. An emptied folder link is never removed.
//! Still to do (ignored below): sync one copy per real file.
//!
//!   cargo test -p cairn-sync --test adv_dupe_links -- --include-ignored --nocapture

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
/// to, as the reason listed for a held change advises.
fn replace_with_copy(link: &Path) {
    let target = fs::canonicalize(link).unwrap();
    fs::remove_file(link).or_else(|_| fs::remove_dir(link)).unwrap();
    copy_tree(&target, link);
}

fn copies_of_note(d: &Device) -> Vec<String> {
    d.files().into_iter().filter(|f| f.1 == NOTE).map(|f| f.0).collect()
}

/// The laptop's vault has notes/x.md and a second name for that folder,
/// `alias -> notes` (not a loop, so it is listed). Both are synced, and a
/// phone has synced after the laptop. None where links cannot be made.
fn linked_folder(srv: &Server) -> Option<(Device, Device)> {
    // inbox.md keeps the vault from looking emptied (the FINDING-006 guard).
    let mut laptop = Device::new(srv, "laptop", &[("notes/x.md", NOTE), ("inbox.md", "inbox\n")]);
    laptop.sync_ok();
    if !link_folder(Path::new("notes"), &laptop.root.join("alias")) {
        return None;
    }
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(srv, "phone", &[]);
    phone.sync_ok();
    println!("phone received {:?}", phone.paths());
    Some((laptop, phone))
}

/// One folder outside the laptop's vault (`shared`, with x.md), linked in
/// under two names, `projects` and `work`. Synced, and a phone has synced
/// after the laptop. Returns the folder that holds `shared` too.
fn two_links_out(srv: &Server) -> Option<(Device, Device, tempfile::TempDir)> {
    let outside = tempfile::tempdir().unwrap();
    let shared = outside.path().join("shared");
    fs::create_dir(&shared).unwrap();
    fs::write(shared.join("x.md"), NOTE).unwrap();
    let mut laptop = Device::new(srv, "laptop", &[("inbox.md", "inbox\n")]);
    if !link_folder(&shared, &laptop.root.join("projects")) || !link_folder(&shared, &laptop.root.join("work")) {
        return None;
    }
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(srv, "phone", &[]);
    phone.sync_ok();
    println!("phone received {:?}", phone.paths());
    Some((laptop, phone, outside))
}

/// Errors, and what the laptop's last sync listed as not synced.
struct Settled {
    errors: Vec<String>,
    skipped: Vec<Skipped>,
}

impl Settled {
    /// Whether the laptop lists `path` as not synced because its file is
    /// also `other` there.
    fn held(&self, path: &str, other: &str) -> bool {
        self.skipped.iter().any(|s| s.path == path && s.reason.contains(&format!("is one file with {other} on this device")))
    }
}

/// Syncs the phone, then the laptop, three times (long enough for a delete
/// to travel back), and returns the errors instead of failing on them.
fn settle(phone: &mut Device, laptop: &mut Device) -> Settled {
    let mut s = Settled { errors: Vec::new(), skipped: Vec::new() };
    for _ in 0..3 {
        if let Err(e) = phone.sync() {
            s.errors.push(format!("phone: {e}"));
        }
        match laptop.sync() {
            Ok(r) => s.skipped = r.skipped,
            Err(e) => s.errors.push(format!("laptop: {e}")),
        }
    }
    println!("sync errors: {:?}\nlaptop not synced: {:?}", s.errors, s.skipped);
    s
}

/// After the user replaced the link on the laptop with a copy of what it
/// leads to, the held change applies to its own copy, and both devices
/// agree, with nothing left out.
fn resolve_by_copying(link: &Path, phone: &mut Device, laptop: &mut Device) {
    replace_with_copy(link);
    let s = settle(phone, laptop);
    assert!(s.errors.is_empty() && s.skipped.is_empty(), "after replacing the link: {:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.files(), phone.files(), "the devices disagree after the link was replaced");
}

#[test]
fn deleting_the_link_copy_on_another_device_keeps_the_real_note() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    // The phone's user sees the note twice and deletes the copy under the
    // link's name (with one copy per real file synced, there is none).
    let both = phone.read("alias/x.md").is_some() && phone.read("notes/x.md").is_some();
    if both {
        phone.rm("alias/x.md");
    }
    let s = settle(&mut phone, &mut laptop);

    assert_eq!(
        (laptop.read("notes/x.md").as_deref(), phone.read("notes/x.md").as_deref()),
        (Some(NOTE), Some(NOTE)),
        "the real note notes/x.md is gone (laptop, phone): laptop trash {:?}, phone trash {:?}",
        laptop.trash(),
        phone.trash()
    );
    assert_eq!(laptop.trash(), vec![], "the laptop moved something to the trash");
    assert!(is_link(&laptop.root.join("alias")), "the link is gone");
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    if both {
        assert!(s.held("alias/x.md", "notes/x.md"), "the held delete is not listed: {:?}", s.skipped);
        resolve_by_copying(&laptop.root.join("alias"), &mut phone, &mut laptop);
        assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"]);
    }
}

/// Two links to one folder outside the vault. Whichever link the user then
/// replaces with a copy, nothing is lost: replacing the one the phone's
/// delete came under keeps the folder outside as it is; replacing the other
/// one applies the delete through the remaining link (sync follows links),
/// and the note stays in the vault as the copy.
#[test]
fn deleting_one_of_two_links_to_an_outside_folder_on_another_device_keeps_the_real_note() {
    for replace_deleted in [true, false] {
        let srv = server();
        let Some((mut laptop, mut phone, outside)) = two_links_out(&srv) else { return };
        let shared = outside.path().join("shared");
        // The phone's user keeps one copy and deletes the other.
        let copies = copies_of_note(&phone);
        if copies.len() > 1 {
            phone.rm(copies.last().unwrap());
        }
        let s = settle(&mut phone, &mut laptop);

        assert_eq!(
            (fs::read_to_string(shared.join("x.md")).ok().as_deref(), copies_of_note(&phone).len()),
            (Some(NOTE), 1),
            "(the real file outside the vault, copies on the phone): laptop trash {:?}, phone files {:?}, phone trash {:?}",
            laptop.trash(),
            phone.paths(),
            phone.trash()
        );
        assert_eq!(laptop.trash(), vec![], "the laptop moved something to the trash");
        assert!(is_link(&laptop.root.join("projects")) && is_link(&laptop.root.join("work")), "a link is gone");
        assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
        if copies.len() < 2 {
            return;
        }
        let (kept, deleted) = (&copies[0], copies.last().unwrap());
        assert!(s.held(deleted, kept), "the held delete is not listed: {:?}", s.skipped);
        let link = if replace_deleted { deleted } else { kept };
        resolve_by_copying(&laptop.root.join(link.split('/').next().unwrap()), &mut phone, &mut laptop);
        for d in [&laptop, &phone] {
            assert_eq!(d.read(kept).as_deref(), Some(NOTE), "replacing {link}: {} has {:?}", d.name, d.files());
        }
        if replace_deleted {
            assert_eq!(fs::read_to_string(shared.join("x.md")).unwrap(), NOTE);
        }
    }
}

/// The phone keeps the copy under the link's name and deletes the one
/// under the real folder's name. Removing the link here would let that
/// delete through and lose the note; replacing it with a copy keeps it.
#[test]
fn deleting_the_real_name_copy_on_another_device_keeps_the_note() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    if phone.read("alias/x.md").is_none() {
        return;
    }
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(phone.read("alias/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.trash(), vec![]);
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.held("notes/x.md", "alias/x.md"), "the held delete is not listed: {:?}", s.skipped);
    resolve_by_copying(&laptop.root.join("alias"), &mut phone, &mut laptop);
    assert_eq!(laptop.paths(), ["alias/x.md", "inbox.md"]);
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some(NOTE));
}

/// The link was renamed outside the app since the last sync, so its copy
/// is tracked under the old name until the push: it still counts as the
/// other name of the real note.
#[test]
fn a_delete_of_the_real_note_waits_when_its_link_was_renamed_since_the_last_sync() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    if phone.read("alias/x.md").is_none() {
        return;
    }
    laptop.mv("alias", "links");
    phone.rm("notes/x.md");
    let s = settle(&mut phone, &mut laptop);

    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert!(is_link(&laptop.root.join("links")));
    assert_eq!(phone.read("links/x.md").as_deref(), Some(NOTE), "{:?}", phone.files());
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.held("notes/x.md", "links/x.md"), "the held delete is not listed: {:?}", s.skipped);
    resolve_by_copying(&laptop.root.join("links"), &mut phone, &mut laptop);
    assert_eq!(laptop.paths(), ["inbox.md", "links/x.md"]);
}

/// A note in the linked folder renamed in the app: its copy under the
/// link's name is found renamed too, and a remote edit of the note is not
/// written through it.
#[test]
fn an_edit_of_a_note_renamed_here_waits_while_its_link_copy_is_synced() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    if phone.read("alias/x.md").is_none() {
        return;
    }
    laptop.app_mv("notes/x.md", "notes/y.md");
    phone.write("notes/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert_eq!((laptop.read("notes/y.md").as_deref(), laptop.read("alias/y.md").as_deref()), (Some(NOTE), Some(NOTE)));
    assert_eq!(phone.read("notes/x.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(phone.read("alias/y.md").as_deref(), Some(NOTE), "{:?}", phone.files());
    assert_eq!((laptop.trash(), phone.trash()), (vec![], vec![]));
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.held("notes/x.md", "alias/y.md"), "the held edit is not listed: {:?}", s.skipped);
}

/// A link in a folder that the laptop cannot read any more is unknown, not
/// gone: its copy still counts as the other name of the real note, also
/// after a restart, when the app has not seen the link yet.
#[cfg(unix)]
#[test]
fn a_delete_waits_while_the_other_name_is_in_a_folder_that_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    for restart in [false, true] {
        let srv = server();
        let mut laptop = Device::new(&srv, "laptop", &[("notes/x.md", NOTE), ("inbox.md", "inbox\n")]);
        fs::create_dir(laptop.root.join("private")).unwrap();
        std::os::unix::fs::symlink("../notes", laptop.root.join("private/link")).unwrap();
        laptop.restart();
        laptop.sync_ok();
        let mut phone = Device::new(&srv, "phone", &[]);
        phone.sync_ok();
        assert_eq!(phone.paths(), ["inbox.md", "notes/x.md", "private/link/x.md"]);
        phone.rm("notes/x.md");
        phone.sync_ok();
        // The folder is locked after a sync that could read it.
        laptop.sync_ok();
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

        assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE), "restart {restart}");
        assert_eq!(laptop.trash(), vec![], "restart {restart}");
        assert!(s.errors.is_empty(), "restart {restart}: sync errors: {:?}", s.errors);
        let held = s.skipped.iter().any(|x| x.path == "notes/x.md" && x.reason.contains("one file with private/link/x.md on this device"));
        assert!(held, "restart {restart}: the held delete is not listed: {:?}", s.skipped);
        // Readable again, it is the link it was.
        let s = settle(&mut phone, &mut laptop);
        assert!(s.held("notes/x.md", "private/link/x.md"), "restart {restart}: {:?}", s.skipped);
        assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    }
}

/// An edit of the copy under the link's name on another device is not
/// written into the real note here, where it would change notes/x.md too.
#[test]
fn editing_the_link_copy_on_another_device_leaves_the_real_note_alone() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    if phone.read("alias/x.md").is_none() {
        return; // one copy per real file: nothing to edit
    }
    phone.write("alias/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(phone.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(phone.read("alias/x.md").as_deref(), Some("edited on the phone\n"));
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.held("alias/x.md", "notes/x.md"), "the held edit is not listed: {:?}", s.skipped);
    // With a copy in place of the link, the held edit applies to that copy
    // only, the separate file it is on the phone.
    resolve_by_copying(&laptop.root.join("alias"), &mut phone, &mut laptop);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some("edited on the phone\n"));
}

/// A rename of the copy under the link's name on another device does not
/// move the real note here.
#[test]
fn renaming_the_link_copy_on_another_device_leaves_the_real_note_alone() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    if phone.read("alias/x.md").is_none() {
        return;
    }
    phone.mv("alias/x.md", "alias/y.md");
    let s = settle(&mut phone, &mut laptop);

    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.read("notes/y.md"), None);
    assert_eq!(phone.read("notes/x.md").as_deref(), Some(NOTE));
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(
        s.skipped.iter().any(|x| x.path == "alias/y.md" && x.reason.contains("(alias/x.md here)") && x.reason.contains("alias/x.md is one file with notes/x.md")),
        "the held rename is not listed: {:?}",
        s.skipped
    );
    resolve_by_copying(&laptop.root.join("alias"), &mut phone, &mut laptop);
    assert_eq!(laptop.paths(), ["alias/y.md", "inbox.md", "notes/x.md"]);
}

/// The real note's own remote edits wait too while its copy under the
/// link's name is synced here: writing them would change that copy.
#[test]
fn editing_the_real_note_on_another_device_waits_while_its_link_copy_is_synced() {
    let srv = server();
    let Some((mut laptop, mut phone)) = linked_folder(&srv) else { return };
    if phone.read("alias/x.md").is_none() {
        return;
    }
    phone.write("notes/x.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some(NOTE));
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.held("notes/x.md", "alias/x.md"), "the held edit is not listed: {:?}", s.skipped);
    // With a copy in place of the link, the edit applies to the real note
    // only.
    resolve_by_copying(&laptop.root.join("alias"), &mut phone, &mut laptop);
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some("edited on the phone\n"));
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some(NOTE));
}

/// The phone, not having seen the laptop's copy under the link's name,
/// uploads a note of its own at that path. The laptop's copy was uploaded
/// first, so the laptop would move it aside to a conflict copy name, which
/// would move the real note too: the phone's note gets that name instead.
#[test]
fn a_note_uploaded_elsewhere_at_the_link_copys_path_does_not_move_the_real_note() {
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
    let r = laptop.sync_ok();
    println!("laptop: {r:?}\n{:?}", laptop.files());

    assert_eq!(r.skipped, vec![], "the phone's note was held");
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.read("alias/x.md").as_deref(), Some(NOTE));
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    assert_eq!(laptop.read(&r.conflicts[0]).as_deref(), Some("the phone's note\n"));
    let s = settle(&mut phone, &mut laptop);
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    for d in [&laptop, &phone] {
        let texts: Vec<String> = d.files().into_iter().map(|f| f.1).collect();
        assert!(texts.contains(&NOTE.to_string()) && texts.contains(&"the phone's note\n".to_string()), "{}: {:?}", d.name, d.files());
    }
    assert_eq!(laptop.read("notes/x.md").as_deref(), Some(NOTE));
}

/// A remote delete through a folder linked in once is applied, as before
/// (sync follows the user's links), and the emptied link stays: removing it
/// as a folder failed with "Not a directory" and held the delete.
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
    assert!(s.errors.is_empty() && s.skipped.is_empty(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.files(), phone.files());
}

/// A note and a symlink to it are one file under two names too: a delete
/// of the note on another device is held, and the link stays.
#[cfg(unix)]
#[test]
fn deleting_a_linked_note_on_another_device_keeps_it_here() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("x.md", NOTE), ("inbox.md", "inbox\n")]);
    std::os::unix::fs::symlink("x.md", laptop.root.join("link.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    assert_eq!(phone.paths(), ["inbox.md", "link.md", "x.md"]);
    phone.rm("x.md");
    let s = settle(&mut phone, &mut laptop);

    assert_eq!(laptop.read("x.md").as_deref(), Some(NOTE));
    assert!(is_link(&laptop.root.join("link.md")));
    assert_eq!(phone.read("link.md").as_deref(), Some(NOTE));
    assert_eq!(laptop.trash(), vec![]);
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.held("x.md", "link.md"), "the held delete is not listed: {:?}", s.skipped);
    resolve_by_copying(&laptop.root.join("link.md"), &mut phone, &mut laptop);
    assert_eq!(laptop.paths(), ["inbox.md", "link.md"]);
    assert_eq!(laptop.read("link.md").as_deref(), Some(NOTE));
}

/// Two hard links are one file under two names as well: a remote write
/// through one would change the other.
#[cfg(unix)]
#[test]
fn editing_one_of_two_hard_links_on_another_device_is_held() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("a.md", NOTE), ("inbox.md", "inbox\n")]);
    fs::hard_link(laptop.root.join("a.md"), laptop.root.join("b.md")).unwrap();
    laptop.restart();
    laptop.sync_ok();
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.sync_ok();
    phone.write("b.md", "edited on the phone\n");
    let s = settle(&mut phone, &mut laptop);

    assert_eq!((laptop.read("a.md").as_deref(), laptop.read("b.md").as_deref()), (Some(NOTE), Some(NOTE)));
    assert_eq!(phone.read("a.md").as_deref(), Some(NOTE));
    assert!(s.errors.is_empty(), "sync errors: {:?}", s.errors);
    assert!(s.held("b.md", "a.md"), "the held edit is not listed: {:?}", s.skipped);
    // b.md becomes a file of its own, and the edit applies to it only.
    let b = laptop.root.join("b.md");
    fs::remove_file(&b).unwrap();
    fs::write(&b, NOTE).unwrap();
    let s = settle(&mut phone, &mut laptop);
    assert!(s.errors.is_empty() && s.skipped.is_empty(), "{:?} {:?}", s.errors, s.skipped);
    assert_eq!(laptop.files(), phone.files());
    assert_eq!((laptop.read("a.md").as_deref(), laptop.read("b.md").as_deref()), (Some(NOTE), Some("edited on the phone\n")));
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
    assert!(s.errors.is_empty() && s.skipped.is_empty(), "{:?} {:?}", s.errors, s.skipped);
}

// ---------------------------------------------------------------- one copy per real file

#[test]
#[ignore = "FINDING-224: a folder reached under two names syncs one copy per name"]
fn a_folder_reached_under_two_names_syncs_one_copy() {
    let srv = server();
    let Some((_laptop, phone)) = linked_folder(&srv) else { return };
    assert_eq!(phone.paths(), ["inbox.md", "notes/x.md"], "one copy per real file, under the real folder's path");
}

#[test]
#[ignore = "FINDING-224: a folder reached under two names syncs one copy per name"]
fn two_links_to_an_outside_folder_sync_one_copy() {
    let srv = server();
    let Some((_laptop, phone, _outside)) = two_links_out(&srv) else { return };
    assert_eq!(copies_of_note(&phone).len(), 1, "one copy per real file: {:?}", phone.paths());
}
