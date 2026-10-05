//! FINDING-063: two devices that each upload a note at the same path with
//! the same content, without having seen the other's, make two server files
//! of one note. A device that sees both keeps the one uploaded later and
//! deletes ("retires") the earlier one on the server, checked against its
//! head, so that nothing is deleted that was changed meanwhile. Devices that
//! have the retired copy unchanged keep their file under the kept id; no
//! file is moved to the trash or written for it.
//!
//! Run:
//!   cargo test -p cairn-sync --test identical_copies

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;

use common::*;
use parking_lot::Mutex;

const SAME: &str = "identical\n";

/// The phone uploads Same.md first, then the laptop its own with the same
/// content, without having seen the phone's. The tablet, connected before,
/// has pulled the phone's copy; the laptop has not pulled anything yet.
fn race(srv: &Server) -> (Device, Device, Device) {
    let mut a = Device::new(srv, "laptop", &[("Same.md", SAME)]);
    let b = Arc::new(Mutex::new(Device::new(srv, "phone", &[("Same.md", SAME)])));
    let c = Arc::new(Mutex::new(Device::new(srv, "tablet", &[])));
    let (b2, c2) = (b.clone(), c.clone());
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        b2.lock().sync();
        c2.lock().sync();
    }));
    a.sync();
    let b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    let c = Arc::try_unwrap(c).ok().unwrap().into_inner();
    assert_eq!(c.files(), [("Same.md".to_string(), SAME.to_string())]);
    (a, b, c)
}

/// The live file id at `path` on a device.
fn id_at(d: &Device, path: &str) -> String {
    let ids: Vec<&String> = d.engine.state().files.iter().filter(|(_, t)| !t.deleted && t.path == path).map(|(f, _)| f).collect();
    assert_eq!(ids.len(), 1, "on {}: {:?}", d.name, d.engine.state().files);
    ids[0].clone()
}

/// Delete revisions on the server, by file id.
fn deletes(srv: &Server) -> Vec<String> {
    let db = rusqlite::Connection::open(&srv.db_path).unwrap();
    let mut q = db.prepare("SELECT file_id FROM revisions WHERE deleted = 1 ORDER BY seq").unwrap();
    q.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
}

fn assert_no_trash(devs: &[&Device], when: &str) {
    for d in devs {
        assert_eq!(d.trash_text(), "", "{when}: {} moved something to its trash", d.name);
    }
}

/// Whichever device sees both copies first deletes the phone's (uploaded
/// first) and keeps the laptop's. The phone and the tablet keep their file,
/// untouched, under the laptop's id.
#[test]
fn identical_copies_become_one_file_and_nothing_goes_to_the_trash() {
    for first in ["laptop", "phone", "tablet"] {
        let srv = server();
        let (mut a, mut b, mut c) = race(&srv);
        let (laptops, phones) = (id_at(&a, "Same.md"), id_at(&b, "Same.md"));
        let before = [mtime(&b.abs("Same.md")), mtime(&c.abs("Same.md"))];
        let r = match first {
            "laptop" => a.sync(),
            "phone" => b.sync(),
            _ => c.sync(),
        };
        assert!(r.conflicts.is_empty() && r.skipped.is_empty(), "{first} first: {r:?}");
        assert_eq!(deletes(&srv), [phones.as_str()], "{first} first");
        converge(&mut [&mut a, &mut b, &mut c]);
        for d in [&a, &b, &c] {
            assert_eq!(d.files(), [("Same.md".to_string(), SAME.to_string())], "{first} first, on {}", d.name);
            assert_eq!(id_at(d, "Same.md"), laptops, "{first} first, on {}", d.name);
        }
        assert_no_trash(&[&a, &b, &c], &format!("{first} first"));
        // not written either
        assert_eq!([mtime(&b.abs("Same.md")), mtime(&c.abs("Same.md"))], before, "{first} first");
        assert_eq!(deletes(&srv), [phones], "{first} first");
        // the kept file's history, on every device
        for d in [&a, &b, &c] {
            let h = d.engine.history("Same.md").unwrap();
            assert_eq!(h.len(), 1, "{first} first, on {}: {h:?}", d.name);
            assert_eq!(h[0].device, "laptop");
        }
    }
}

/// The laptop keeps editing the kept copy after the retire, before the
/// others sync: they take the edit into their file, as for any edit, with
/// nothing in the trash.
#[test]
fn kept_copy_edited_after_the_retire_reaches_the_other_copies() {
    let srv = server();
    let (mut a, mut b, mut c) = race(&srv);
    a.sync();
    a.write("Same.md", "identical\nmore\n");
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    for d in [&a, &b, &c] {
        assert_eq!(d.files(), [("Same.md".to_string(), "identical\nmore\n".to_string())], "on {}", d.name);
    }
    assert_no_trash(&[&a, &b, &c], "after the edit");
    assert_eq!(deletes(&srv).len(), 1);
}

/// The laptop renames the kept copy after the retire, before the others
/// sync: they rename their file, with nothing in the trash.
#[test]
fn kept_copy_renamed_after_the_retire_renames_the_other_copies() {
    let srv = server();
    let (mut a, mut b, mut c) = race(&srv);
    a.sync();
    a.mv("Same.md", "Renamed.md");
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    for d in [&a, &b, &c] {
        assert_eq!(d.files(), [("Renamed.md".to_string(), SAME.to_string())], "on {}", d.name);
    }
    assert_no_trash(&[&a, &b, &c], "after the rename");
    assert_eq!(deletes(&srv).len(), 1);
}

/// The tablet edits its copy (the phone's, uploaded first) and uploads the
/// edit while the laptop is retiring that copy: before the laptop looks at
/// the files' history, or between that and its delete, which the server
/// then refuses. The edit is kept, as an ordinary conflict copy of the
/// laptop's file; nothing is deleted.
#[test]
fn copy_edited_during_the_retire_is_kept_as_a_conflict_copy() {
    for when in ["after_changes", "before_put"] {
        let srv = server();
        let (mut a, mut b, c) = race(&srv);
        c.write("Same.md", "identical\nedited on tablet\n");
        let c = Arc::new(Mutex::new(c));
        let c2 = c.clone();
        let hook: Hook = Box::new(move || {
            let r = c2.lock().sync();
            assert_eq!(r.pushed, 1, "{r:?}");
        });
        match when {
            "after_changes" => *a.hooks.after_changes.lock() = Some(hook),
            _ => *a.hooks.before_put.lock() = Some(hook),
        }
        let r = a.sync();
        let mut c = Arc::try_unwrap(c).ok().unwrap().into_inner();
        if when == "before_put" {
            assert_eq!(a.hooks.refused_deletes.load(SeqCst), 1, "the delete was not refused: {r:?}");
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        for d in [&a, &b, &c] {
            assert_eq!(d.read("Same.md").as_deref(), Some(SAME), "{when}, on {}: {:?}", d.name, d.files());
            let copies = d.conflict_copies();
            assert_eq!(copies.len(), 1, "{when}, on {}: {:?}", d.name, d.files());
            assert_eq!(d.read(&copies[0]).as_deref(), Some("identical\nedited on tablet\n"), "{when}, on {}", d.name);
            assert_eq!(d.files().len(), 2, "{when}, on {}: {:?}", d.name, d.files());
        }
        assert!(deletes(&srv).is_empty(), "{when}: {:?}", deletes(&srv));
        assert_no_trash(&[&a, &b, &c], when);
    }
}

/// The tablet pulls before the laptop uploads its copy, edits the phone's
/// copy, and uploads the edit right before the laptop's delete of that copy,
/// which the server refuses. The edit is kept, as an ordinary conflict copy
/// of the laptop's file; nothing is deleted.
#[test]
fn edit_uploaded_just_before_the_delete_is_kept_as_a_conflict_copy() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Same.md", SAME)]);
    let b = Arc::new(Mutex::new(Device::new(&srv, "phone", &[("Same.md", SAME)])));
    let mut c = Device::new(&srv, "tablet", &[]);
    let (fetched_tx, fetched_rx) = std::sync::mpsc::channel::<()>();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let tablet = Arc::new(Mutex::new(None));
    let (b2, tablet2) = (b.clone(), tablet.clone());
    // Before the laptop's upload: the phone uploads its copy, the tablet
    // pulls it, edits it, and starts a sync that fetches the changes now
    // but uploads the edit only when told to.
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        b2.lock().sync();
        c.sync();
        c.write("Same.md", "identical\nedited on tablet\n");
        *c.hooks.after_changes.lock() = Some(Box::new(move || fetched_tx.send(()).unwrap()));
        *c.hooks.before_put.lock() = Some(Box::new(move || go_rx.recv().unwrap()));
        *tablet2.lock() = Some(std::thread::spawn(move || {
            c.sync();
            c
        }));
        fetched_rx.recv().unwrap();
    }));
    a.sync();
    // The laptop retires the phone's copy; the tablet's edit gets in first.
    let done = Arc::new(Mutex::new(None));
    let (tablet3, done2) = (tablet.clone(), done.clone());
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        go_tx.send(()).unwrap();
        *done2.lock() = Some(tablet3.lock().take().unwrap().join().unwrap());
    }));
    let r = a.sync();
    assert_eq!(a.hooks.refused_deletes.load(SeqCst), 1, "the delete was not refused: {r:?}");
    let mut c: Device = done.lock().take().unwrap();
    let mut b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    converge(&mut [&mut a, &mut b, &mut c]);
    for d in [&a, &b, &c] {
        assert_eq!(d.read("Same.md").as_deref(), Some(SAME), "on {}: {:?}", d.name, d.files());
        let copies = d.conflict_copies();
        assert_eq!(copies.len(), 1, "on {}: {:?}", d.name, d.files());
        assert_eq!(d.read(&copies[0]).as_deref(), Some("identical\nedited on tablet\n"), "on {}", d.name);
        assert_eq!(d.files().len(), 2, "on {}: {:?}", d.name, d.files());
    }
    assert!(deletes(&srv).is_empty(), "{:?}", deletes(&srv));
    assert_no_trash(&[&a, &b, &c], "after the edit");
}

/// The laptop and the phone retire the phone's copy at once: the phone
/// while the laptop is between fetching the changes and deciding, or
/// between deciding and its delete. One delete gets through; the laptop's
/// is refused or not made, and it takes the phone's.
#[test]
fn two_devices_retiring_at_once_delete_the_copy_once() {
    for when in ["after_changes", "before_put"] {
        let srv = server();
        let (mut a, b, mut c) = race(&srv);
        let phones = id_at(&b, "Same.md");
        let b = Arc::new(Mutex::new(b));
        let b2 = b.clone();
        let hook: Hook = Box::new(move || {
            let r = b2.lock().sync();
            assert!(r.conflicts.is_empty(), "{r:?}");
        });
        match when {
            "after_changes" => *a.hooks.after_changes.lock() = Some(hook),
            _ => *a.hooks.before_put.lock() = Some(hook),
        }
        let r = a.sync();
        assert!(r.conflicts.is_empty() && r.skipped.is_empty() && r.pushed == 0, "{when}: {r:?}");
        let mut b = Arc::try_unwrap(b).ok().unwrap().into_inner();
        assert_eq!(deletes(&srv), [phones.as_str()], "{when}");
        converge(&mut [&mut a, &mut b, &mut c]);
        let laptops = id_at(&a, "Same.md");
        for d in [&a, &b, &c] {
            assert_eq!(d.files(), [("Same.md".to_string(), SAME.to_string())], "{when}, on {}", d.name);
            assert_eq!(id_at(d, "Same.md"), laptops, "{when}, on {}", d.name);
        }
        assert_eq!(deletes(&srv), [phones], "{when}");
        assert_no_trash(&[&a, &b, &c], when);
    }
}

/// The phone edits its copy before it syncs again: the edit wins over the
/// delete, and the phone's file is revived, next to the laptop's as an
/// ordinary conflict copy pair (the revived file keeps the name, as when an
/// edited file is deleted elsewhere and a new one takes its name).
#[test]
fn copy_edited_before_its_retire_arrives_is_kept_next_to_the_other() {
    let srv = server();
    let (mut a, mut b, mut c) = race(&srv);
    a.sync();
    b.write("Same.md", "identical\nedited on phone\n");
    converge(&mut [&mut a, &mut b, &mut c]);
    for d in [&a, &b, &c] {
        let copies = d.conflict_copies();
        assert_eq!(copies.len(), 1, "on {}: {:?}", d.name, d.files());
        let mut contents: Vec<String> = d.files().into_iter().map(|f| f.1).collect();
        contents.sort();
        assert_eq!(contents, [SAME, "identical\nedited on phone\n"], "on {}: {:?}", d.name, d.files());
    }
    assert_no_trash(&[&a, &b, &c], "after the edit");
}

/// The tablet has the phone's copy (uploaded first) and pulls the laptop's.
/// Right after the tablet fetched the changes, the laptop syncs and retires
/// the phone's copy: the tablet takes its file for the laptop's, without a
/// delete of its own and with nothing in the trash.
#[test]
fn copy_retired_by_another_device_during_the_pull_is_kept_under_the_other_id() {
    let srv = server();
    let (a, mut b, mut c) = race(&srv);
    let phones = id_at(&b, "Same.md");
    let a = Arc::new(Mutex::new(a));
    let a2 = a.clone();
    *c.hooks.after_changes.lock() = Some(Box::new(move || {
        let r = a2.lock().sync();
        assert!(r.conflicts.is_empty() && r.skipped.is_empty(), "laptop: {r:?}");
    }));
    let r = c.sync();
    assert!(r.conflicts.is_empty() && r.skipped.is_empty() && r.pushed == 0, "{r:?}");
    assert_eq!(c.hooks.delete_puts.load(SeqCst), 0, "the tablet deleted a copy too");
    let mut a = Arc::try_unwrap(a).ok().unwrap().into_inner();
    assert_eq!(deletes(&srv), [phones.as_str()]);
    converge(&mut [&mut a, &mut b, &mut c]);
    let laptops = id_at(&a, "Same.md");
    for d in [&a, &b, &c] {
        assert_eq!(d.files(), [("Same.md".to_string(), SAME.to_string())], "on {}", d.name);
        assert_eq!(id_at(d, "Same.md"), laptops, "on {}", d.name);
    }
    assert_eq!(deletes(&srv), [phones]);
    assert_no_trash(&[&a, &b, &c], "after the retire");
}

/// The same when the tablet pulls in batches, and the laptop's copy and the
/// delete of the phone's are in different ones (as in a pull of more than
/// about 64 MB). A new round would read the batch with the laptop's copy
/// again, without the delete, so the tablet must not start one: otherwise
/// every sync of the tablet would stop with "the server kept changing".
#[test]
fn copy_retired_in_a_later_batch_of_the_pull_is_kept_under_the_other_id() {
    let srv = server();
    let (mut a, mut b, mut c) = race(&srv);
    let phones = id_at(&b, "Same.md");
    a.sync();
    assert_eq!(deletes(&srv), [phones.as_str()]);
    c.pull_one_by_one();
    let r = c.try_sync().map_err(|e| e.to_string()).unwrap();
    assert!(r.conflicts.is_empty() && r.skipped.is_empty() && r.pushed == 0, "{r:?}");
    assert_eq!(c.hooks.delete_puts.load(SeqCst), 0);
    converge(&mut [&mut a, &mut b, &mut c]);
    let laptops = id_at(&a, "Same.md");
    for d in [&a, &b, &c] {
        assert_eq!(d.files(), [("Same.md".to_string(), SAME.to_string())], "on {}", d.name);
        assert_eq!(id_at(d, "Same.md"), laptops, "on {}", d.name);
    }
    assert_eq!(deletes(&srv), [phones]);
    assert_no_trash(&[&a, &b, &c], "after the retire");
}

/// The laptop's copy waits as pending on the tablet (its history could not
/// be fetched), and the laptop deletes it before the tablet syncs again. The
/// tablet pulls in batches: the first has another change and the retry of
/// the pending copy, which has moved on, and the delete comes in the next.
/// The pending copy waits for it instead of starting the round again, which
/// would retry it with the same first batch for ever.
#[test]
fn pending_copy_that_moved_on_waits_for_its_newer_change() {
    let srv = server();
    let (mut a, mut b, mut c) = race(&srv);
    c.hooks.fail_history.store(1, SeqCst);
    let r = c.sync();
    assert_eq!(r.skipped.len(), 1, "the laptop's copy is not pending: {r:?}");
    // The laptop uploads a note (its retire cannot look at the history),
    // then deletes its copy of Same.md, which leaves the phone's.
    a.hooks.fail_history.store(1, SeqCst);
    a.write("x.md", "x\n");
    a.sync();
    a.rm("Same.md");
    a.sync();
    assert_eq!(deletes(&srv).len(), 1);
    c.pull_one_by_one();
    let r = c.try_sync().map_err(|e| e.to_string()).unwrap();
    assert!(r.conflicts.is_empty() && r.skipped.is_empty(), "{r:?}");
    converge(&mut [&mut a, &mut b, &mut c]);
    let phones = id_at(&b, "Same.md");
    for d in [&a, &b, &c] {
        let mut files = d.files();
        files.sort();
        assert_eq!(files, [("Same.md".to_string(), SAME.to_string()), ("x.md".to_string(), "x\n".to_string())], "on {}", d.name);
        assert_eq!(id_at(d, "Same.md"), phones, "on {}", d.name);
    }
    assert_no_trash(&[&b, &c], "after the delete");
}
