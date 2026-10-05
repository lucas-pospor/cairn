//! Reproduction of the sync consequence described by
//! FINDING-136: content-only rename matching moves the file id of a
//! deleted note onto an unrelated new note with the same content, so a
//! concurrent remote edit of the deleted note lands in the unrelated note
//! instead of following "edit beats delete" (PLAN section 3).
//!
//! The matching that matters for sync is the sync engine's own
//! (crates/cairn-sync/src/engine.rs classify(): "A missing file whose exact
//! content appeared elsewhere was renamed"), not Vault::apply_diff.
//!
//! Run:
//!   cargo test -p cairn-sync --test adv_verify_fs_16 -- --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use cairn_core::Change;
use common::*;

const MEETING: &str = "Meeting 2026-09-01.md";
const AGENDA: &str = "Agenda\n- budget\n";

fn pair(srv: &Server, files: &[(&str, &str)]) -> (Device, Device) {
    let mut a = Device::new(srv, "laptop", files);
    a.sync();
    let mut b = Device::new(srv, "phone", &[]);
    b.sync();
    (a, b)
}

/// Control: delete on the laptop vs edit on the phone, nothing else
/// created. Edit beats delete: the meeting note comes back with the agenda.
#[test]
fn control_edit_beats_delete() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[(MEETING, ""), ("work/keep.md", "k")]);
    b.write(MEETING, AGENDA);
    b.sync();
    a.rm(MEETING);
    a.sync();
    converge(&mut [&mut a, &mut b]);
    assert_eq!(a.read(MEETING).as_deref(), Some(AGENDA));
}

fn outcome(d: &Device) -> String {
    format!("{} files: {:?}", d.name, d.files())
}

#[test]
fn fs16_remote_edit_follows_unrelated_identical_note() {
    let mut problems = Vec::new();
    for phone_first in [true, false] {
        let srv = server();
        let (mut a, mut b) = pair(&srv, &[(MEETING, ""), ("work/keep.md", "k")]);
        // Laptop: deletes the empty meeting note, separately creates an empty todo.
        // Phone: types the agenda into the meeting note.
        b.write(MEETING, AGENDA);
        a.rm(MEETING);
        a.write("work/Todo.md", "");
        if phone_first {
            b.sync();
            a.sync();
        } else {
            a.sync();
            b.sync();
        }
        converge(&mut [&mut a, &mut b]);
        println!("phone_first={phone_first}: {} | {}", outcome(&a), outcome(&b));
        if a.read(MEETING).as_deref() != Some(AGENDA) {
            problems.push(format!("phone_first={phone_first}: meeting note not restored with the edit; Todo.md = {:?}", a.read("work/Todo.md")));
        }
        if a.read("work/Todo.md").as_deref() != Some("") {
            problems.push(format!("phone_first={phone_first}: work/Todo.md holds {:?}", a.read("work/Todo.md")));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The file id tracked at `p`, if any.
fn id(d: &Device, p: &str) -> Option<String> {
    d.engine.state().files.iter().find(|(_, t)| !t.deleted && t.path == p).map(|(f, _)| f.clone())
}

/// Two notes made from one template, moved outside Cairn at once into
/// different folders: each keeps its own file id, here and on the phone.
#[test]
fn identical_notes_renamed_together_keep_their_file_ids() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[("a.md", "template"), ("b.md", "template")]);
    let (ia, ib) = (id(&a, "a.md").unwrap(), id(&a, "b.md").unwrap());
    // Matching on content alone gave the first new path (in path order) to
    // the first file id: send the note with the smaller id to the later
    // folder, so that matching swaps them.
    let (to_a, to_b) = if ia < ib { ("y/a.md", "x/b.md") } else { ("x/a.md", "y/b.md") };
    a.mv("a.md", to_a);
    a.mv("b.md", to_b);
    a.sync();
    b.sync();
    for d in [&a, &b] {
        assert_eq!((id(d, to_a), id(d, to_b)), (Some(ia.clone()), Some(ib.clone())), "{}", outcome(d));
    }
}

/// An empty note says nothing about where it went: renamed outside Cairn,
/// it syncs as a delete and a create. The phone trashes the old name.
#[test]
fn empty_note_renamed_outside_cairn_syncs_as_delete_and_create() {
    let srv = server();
    let (mut a, mut b) = pair(&srv, &[(MEETING, ""), ("work/keep.md", "k")]);
    let old = id(&a, MEETING).unwrap();
    a.mv(MEETING, "work/Todo.md");
    a.sync();
    let new = id(&a, "work/Todo.md").expect("the new name is tracked");
    assert_ne!(new, old, "the empty note kept its file id under the new name");
    assert!(a.engine.state().files[&old].deleted, "the old file id is not deleted");
    let rep = b.sync();
    let renamed = rep.changes.iter().any(|c| matches!(c, Change::Renamed { .. }));
    assert!(!renamed, "the phone renamed the note: {:?}", rep.changes);
    assert_eq!(b.files(), [("work/Todo.md".to_string(), String::new()), ("work/keep.md".into(), "k".into())]);
    assert_eq!(id(&b, "work/Todo.md"), Some(new));
    let mut trash = Vec::new();
    walk(&b.abs(".trash"), &b.abs(".trash"), true, &mut trash);
    assert!(trash.iter().any(|(p, _)| p.starts_with("Meeting 2026-09-01")), "the old name is not in the phone's trash: {trash:?}");
}
