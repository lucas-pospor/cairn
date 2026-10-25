//! Adversarial sync-semantics tests: three devices, renames, folders,
//! deletes, case, clock skew, conflict copies and merges, compared with the
//! conflict rules in docs/PLAN.md section 3.
//!
//! Tests marked `#[ignore = "FINDING-nnn: ..."]` reproduce open findings
//! or behaviour that is not changed by design, and fail. Run one with
//!   cargo test -p cairn-sync --test adv_sync_semantics -- --ignored --exact <name>
//! Run the passing ones with
//!   cargo test -p cairn-sync --test adv_sync_semantics

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::fs;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cairn_core::{Change, EntryKind};
use cairn_sync::engine::{SyncEngine, Tracked};
use common::*;
use parking_lot::Mutex;

fn trio(srv: &Server, files: &[(&str, &str)]) -> (Device, Device, Device) {
    let mut a = Device::new(srv, "laptop", files);
    a.sync();
    let mut b = Device::new(srv, "phone", &[]);
    b.sync();
    let mut c = Device::new(srv, "tablet", &[]);
    c.sync();
    (a, b, c)
}

// ===================================================================== three devices

#[test]
fn three_devices_interleaved_edits_to_different_lines_merge() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("n.md", "l1\nl2\nl3\nl4\nl5\nl6\nl7\n")]);
    a.write("n.md", "l1 A\nl2\nl3\nl4\nl5\nl6\nl7\n");
    b.write("n.md", "l1\nl2\nl3\nl4 B\nl5\nl6\nl7\n");
    c.write("n.md", "l1\nl2\nl3\nl4\nl5\nl6\nl7 C\n");
    // interleaved: B first, then A, then C, then everyone again
    b.sync();
    a.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let want = "l1 A\nl2\nl3\nl4 B\nl5\nl6\nl7 C\n";
    for d in [&a, &b, &c] {
        assert_eq!(d.read("n.md").as_deref(), Some(want), "on {}", d.name);
        assert!(d.conflict_copies().is_empty(), "{:?}", d.paths());
    }
}

#[test]
fn three_devices_edit_the_same_line_keep_all_three_versions() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("n.md", "same\n")]);
    a.write("n.md", "from laptop\n");
    b.write("n.md", "from phone\n");
    c.write("n.md", "from tablet\n");
    a.sync();
    b.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let all: String = a.files().into_iter().map(|f| f.1).collect();
    for w in ["from laptop", "from phone", "from tablet"] {
        assert!(all.contains(w), "{w} lost: {:?}", a.files());
    }
    assert_eq!(a.files().len(), 3, "{:?}", a.paths());
    // more syncs make no more copies
    for _ in 0..3 {
        for d in [&mut a, &mut b, &mut c] {
            let r = d.sync();
            assert!(r.conflicts.is_empty() && r.pushed == 0, "{} keeps changing: {r:?}", d.name);
        }
    }
}

#[test]
fn three_devices_create_the_same_path_with_different_content() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop\n")]);
    let mut b = Device::new(&srv, "phone", &[("Today.md", "phone\n")]);
    let mut c = Device::new(&srv, "tablet", &[("Today.md", "tablet\n")]);
    a.sync();
    b.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let files = a.files();
    assert_eq!(files.len(), 3, "{files:?}");
    for w in ["laptop\n", "phone\n", "tablet\n"] {
        assert!(files.iter().any(|f| f.1 == w), "{w:?} lost: {files:?}");
    }
}

/// PLAN: "the file uploaded first is renamed to a conflict copy and the
/// second keeps the name". With three devices the last one keeps the name,
/// also when the laptop pulls (a) the rename of its own Today.md to a
/// conflict name and (b) the tablet's new Today.md in the same pull: the
/// scan from before the pull still shows Today.md as taken, but the
/// tablet's note must not be renamed too.
#[test]
fn three_devices_create_the_same_path_last_one_keeps_the_name() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop\n")]);
    let mut b = Device::new(&srv, "phone", &[("Today.md", "phone\n")]);
    let mut c = Device::new(&srv, "tablet", &[("Today.md", "tablet\n")]);
    a.sync();
    b.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.read("Today.md").as_deref(), Some("tablet\n"), "nobody has Today.md: {:?}", a.paths());
}

/// Rename a note and later create a new note with the old name (daily
/// notes, "Untitled.md", templates). A device that receives both changes in
/// one pull keeps the NEW note under the old name, which is free by then,
/// with no conflict copy.
#[test]
fn rename_then_new_note_with_the_old_name_received_in_one_pull() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Untitled.md", "first note\n")]);
    a.mv("Untitled.md", "Meeting notes.md");
    a.sync();
    a.write("Untitled.md", "second note\n");
    a.sync();
    let r = b.sync(); // the phone was offline for both
    converge(&mut [&mut a, &mut b, &mut c]);
    assert!(r.conflicts.is_empty(), "phone made conflict copies: {:?}", r.conflicts);
    assert_eq!(
        a.files(),
        vec![("Meeting notes.md".to_string(), "first note\n".to_string()), ("Untitled.md".to_string(), "second note\n".to_string())]
    );
}

#[test]
fn three_devices_all_edit_one_note_before_any_sync_offline_for_long() {
    // Each device edits its own section many times; only then they sync.
    let srv = server();
    let base: String = (0..30).map(|i| format!("line {i}\n")).collect();
    let (mut a, mut b, mut c) = trio(&srv, &[("long.md", &base)]);
    let mut la: Vec<String> = base.lines().map(String::from).collect();
    let mut lb = la.clone();
    let mut lc = la.clone();
    for i in 0..5 {
        la[i] = format!("line {i} edited by laptop");
        lb[12 + i] = format!("line {} edited by phone", 12 + i);
        lc[24 + i] = format!("line {} edited by tablet", 24 + i);
    }
    a.write("long.md", &(la.join("\n") + "\n"));
    b.write("long.md", &(lb.join("\n") + "\n"));
    c.write("long.md", &(lc.join("\n") + "\n"));
    c.sync();
    b.sync();
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let got = a.read("long.md").unwrap();
    for i in 0..5 {
        assert!(got.contains(&format!("line {i} edited by laptop")));
        assert!(got.contains(&format!("line {} edited by phone", 12 + i)));
        assert!(got.contains(&format!("line {} edited by tablet", 24 + i)));
    }
    assert_eq!(got.lines().count(), 30, "{got}");
}

// ===================================================================== renames

#[test]
fn rename_into_a_path_the_other_device_just_created() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "original a\n")]);
    a.mv("a.md", "b.md");
    b.write("b.md", "new b from phone\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let files = a.files();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files.iter().any(|f| f.1 == "original a\n"));
    assert!(files.iter().any(|f| f.1 == "new b from phone\n"));
}

#[test]
fn rename_into_a_path_the_other_device_just_created_other_order() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "original a\n")]);
    a.mv("a.md", "b.md");
    b.write("b.md", "new b from phone\n");
    b.sync();
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let files = a.files();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files.iter().any(|f| f.1 == "original a\n"));
    assert!(files.iter().any(|f| f.1 == "new b from phone\n"));
}

#[test]
fn both_rename_to_different_names_first_synced_name_wins() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "x\n")]);
    a.mv("a.md", "from-laptop.md");
    b.mv("a.md", "from-phone.md");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.files(), vec![("from-laptop.md".to_string(), "x\n".to_string())]);
}

#[test]
fn both_rename_to_the_same_name() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "x\n")]);
    a.mv("a.md", "dir/same.md");
    b.mv("a.md", "dir/same.md");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.files(), vec![("dir/same.md".to_string(), "x\n".to_string())]);
}

#[test]
fn rename_on_one_device_edits_on_two_others() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("draft.md", "1\n2\n3\n4\n5\n")]);
    a.mv("draft.md", "final/report.md");
    b.write("draft.md", "1 phone\n2\n3\n4\n5\n");
    c.write("draft.md", "1\n2\n3\n4\n5 tablet\n");
    b.sync();
    a.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.files(), vec![("final/report.md".to_string(), "1 phone\n2\n3\n4\n5 tablet\n".to_string())]);
}

#[test]
fn swap_two_files_by_renames_while_other_device_edits_one() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "alpha 1\nalpha 2\n"), ("b.md", "beta 1\nbeta 2\n")]);
    a.mv("a.md", "tmp.md");
    a.mv("b.md", "a.md");
    a.mv("tmp.md", "b.md");
    b.write("a.md", "alpha 1\nalpha 2\nalpha 3 phone\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let all: String = a.files().into_iter().map(|f| f.1).collect();
    for w in ["alpha 1", "alpha 3 phone", "beta 1", "beta 2"] {
        assert!(all.contains(w), "{w} lost: {:?}", a.files());
    }
}

/// A swap done in three steps with a sync after each (a -> tmp, b -> a,
/// tmp -> b), received by a device that was offline for all three: "b -> a"
/// comes while a.md is still there, and "a -> b" while b.md is. Giving b's
/// note a conflict name there would upload it to everyone.
#[test]
fn swap_renames_received_in_one_pull_need_no_conflict_copy() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "alpha\n"), ("b.md", "beta\n")]);
    a.mv("a.md", "tmp.md");
    a.sync();
    a.mv("b.md", "a.md");
    a.sync();
    a.mv("tmp.md", "b.md");
    a.sync();
    let r = b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert!(r.conflicts.is_empty(), "phone made {:?}", r.conflicts);
    assert_eq!(a.files(), vec![("a.md".to_string(), "beta\n".to_string()), ("b.md".to_string(), "alpha\n".to_string())]);
}

#[test]
fn rename_vs_delete_rename_synced_first() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "keep me?\n"), ("anchor.md", "untouched\n")]);
    a.mv("a.md", "renamed.md");
    b.rm("a.md");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let in_tree = a.files().iter().any(|f| f.1 == "keep me?\n");
    let in_trash = a.trash_text().contains("keep me?") || b.trash_text().contains("keep me?");
    assert!(in_tree || in_trash, "content gone");
    eprintln!("rename first: tree {:?}, kept in tree: {in_tree}", a.paths());
}

#[test]
fn rename_vs_delete_delete_synced_first() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "keep me?\n"), ("anchor.md", "untouched\n")]);
    a.mv("a.md", "renamed.md");
    b.rm("a.md");
    b.sync();
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let in_tree = a.files().iter().any(|f| f.1 == "keep me?\n");
    let in_trash = a.trash_text().contains("keep me?") || b.trash_text().contains("keep me?");
    assert!(in_tree || in_trash, "content gone");
    eprintln!("delete first: tree {:?}, kept in tree: {in_tree}", a.paths());
}

/// The outcome of "rename on one device, delete on another" should not
/// depend on which device happens to sync first.
#[test]
fn rename_vs_delete_is_order_independent() {
    let mut outcomes = Vec::new();
    for rename_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "keep me?\n"), ("anchor.md", "untouched\n")]);
        a.mv("a.md", "renamed.md");
        b.rm("a.md");
        if rename_first {
            a.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        outcomes.push(a.paths());
    }
    assert_eq!(outcomes[0], outcomes[1], "rename synced first -> {:?}; delete synced first -> {:?}", outcomes[0], outcomes[1]);
}

/// A rename followed by an edit before the next sync (the app syncs 4 s
/// after the last keystroke, so "rename, then keep typing" is the normal
/// case) loses the file identity: the other devices see a delete plus a new
/// file. A concurrent edit on another device then resurrects the old name
/// instead of merging into the renamed note.
#[test]
#[ignore = "FINDING-062: won't fix (by design) for renames made outside Cairn: a rename + edit there is a delete + create; see rename_in_app_then_edit_merges_with_edit_on_another"]
fn rename_then_edit_on_one_device_merges_with_edit_on_another() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("draft.md", "1\n2\n3\n4\n5\n")]);
    a.mv("draft.md", "final.md");
    a.write("final.md", "1 laptop\n2\n3\n4\n5\n");
    b.write("draft.md", "1\n2\n3\n4\n5 phone\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(
        a.files(),
        vec![("final.md".to_string(), "1 laptop\n2\n3\n4\n5 phone\n".to_string())],
        "expected one merged, renamed note"
    );
}

#[test]
#[ignore = "FINDING-062: won't fix (by design) for renames made outside Cairn: a rename + edit there cuts the version history; see rename_in_app_then_edit_keeps_version_history"]
fn rename_then_edit_keeps_version_history() {
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[("draft.md", "v1\n")]);
    a.write("draft.md", "v2\n");
    a.sync();
    a.mv("draft.md", "final.md");
    a.write("final.md", "v3\n");
    a.sync();
    b.sync();
    let h = b.engine.history("final.md").unwrap();
    assert!(h.len() >= 3, "history of final.md has {} entries (v1, v2 are only reachable from the deleted draft.md)", h.len());
}

#[test]
fn both_rename_to_same_name_and_one_also_edits() {
    // Device order matters in the engine; check both.
    for laptop_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "1\n2\n")]);
        a.mv("a.md", "b.md");
        b.mv("a.md", "b.md");
        b.write("b.md", "1\n2\n3 phone\n");
        if laptop_first {
            a.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        let all: String = a.files().into_iter().map(|f| f.1).collect();
        assert!(all.contains("3 phone"), "edit lost (laptop_first={laptop_first}): {:?}", a.files());
        eprintln!("laptop_first={laptop_first}: {:?}", a.files());
    }
}

#[test]
#[ignore = "FINDING-062: won't fix (by design) for renames made outside Cairn: rename + edit vs the same rename elsewhere leaves two files; see both_rename_in_app_to_same_name_and_one_also_edits_gives_one_file"]
fn both_rename_to_same_name_and_one_also_edits_gives_one_file() {
    for laptop_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "1\n2\n")]);
        a.mv("a.md", "b.md");
        b.mv("a.md", "b.md");
        b.write("b.md", "1\n2\n3 phone\n");
        if laptop_first {
            a.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        assert_eq!(a.files(), vec![("b.md".to_string(), "1\n2\n3 phone\n".to_string())], "laptop_first={laptop_first}");
    }
}

// FINDING-062: a rename made in the app is recorded for the next sync
// (`cairn_sync::engine::rename`, what the rename command calls), so a
// rename and an edit before that sync are one change of the same file.

#[test]
fn rename_in_app_then_edit_merges_with_edit_on_another() {
    for laptop_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("draft.md", "1\n2\n3\n4\n5\n")]);
        a.app_mv("draft.md", "final.md");
        a.write("final.md", "1 laptop\n2\n3\n4\n5\n");
        b.write("draft.md", "1\n2\n3\n4\n5 phone\n");
        if laptop_first {
            a.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        assert_eq!(a.files(), vec![("final.md".to_string(), "1 laptop\n2\n3\n4\n5 phone\n".to_string())], "laptop_first={laptop_first}");
    }
}

#[test]
fn rename_in_app_then_edit_keeps_version_history() {
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[("draft.md", "v1\n")]);
    a.write("draft.md", "v2\n");
    a.sync();
    b.sync();
    a.app_mv("draft.md", "final.md");
    a.write("final.md", "v3\n");
    a.sync();
    b.sync();
    assert_eq!(b.files(), vec![("final.md".to_string(), "v3\n".to_string())]);
    assert_eq!(b.engine.history("final.md").unwrap().len(), 3);
    assert_eq!(b.trash_text(), "", "the note went to the trash on the other device");
}

#[test]
fn both_rename_in_app_to_same_name_and_one_also_edits_gives_one_file() {
    for laptop_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "1\n2\n")]);
        a.app_mv("a.md", "b.md");
        b.app_mv("a.md", "b.md");
        b.write("b.md", "1\n2\n3 phone\n");
        if laptop_first {
            a.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        assert_eq!(a.files(), vec![("b.md".to_string(), "1\n2\n3 phone\n".to_string())], "laptop_first={laptop_first}");
    }
}

/// A folder renamed in the app: the notes in it keep their ids, also the
/// one edited before the sync, and an edit made elsewhere meanwhile merges.
#[test]
fn folder_renamed_in_app_with_an_edited_note_merges_with_edit_on_another() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Inbox/x.md", "1\n2\n3\n"), ("Inbox/sub/y.md", "y\n"), ("anchor.md", "untouched\n")]);
    a.app_mv("Inbox", "Archive");
    a.write("Archive/x.md", "1 laptop\n2\n3\n");
    b.write("Inbox/x.md", "1\n2\n3 phone\n");
    b.sync();
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(
        a.files(),
        vec![
            ("Archive/sub/y.md".to_string(), "y\n".to_string()),
            ("Archive/x.md".to_string(), "1 laptop\n2\n3 phone\n".to_string()),
            ("anchor.md".to_string(), "untouched\n".to_string()),
        ]
    );
    assert_eq!(c.engine.history("Archive/x.md").unwrap().len(), 3, "upload, phone edit, laptop rename with both edits");
}

/// Renamed in the app, then a new note under the old name, both edited
/// before the sync: the renamed note keeps its id, the new one is new.
#[test]
fn rename_in_app_then_new_note_under_the_old_name() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("draft.md", "1\n2\n3\n")]);
    a.app_mv("draft.md", "final.md");
    a.write("final.md", "1 laptop\n2\n3\n");
    a.write("draft.md", "a new draft\n");
    b.write("draft.md", "1\n2\n3 phone\n");
    b.sync();
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let files = a.files();
    assert!(files.contains(&("final.md".to_string(), "1 laptop\n2\n3 phone\n".to_string())), "{files:?}");
    assert!(files.iter().any(|f| f.1 == "a new draft\n"), "{files:?}");
    assert_eq!(files.len(), 2, "{files:?}");
}

/// Renamed in the app twice before a sync, back to its first name the
/// second time: nothing is uploaded for it.
#[test]
fn rename_in_app_and_back_uploads_nothing() {
    let srv = server();
    let (mut a, _b, _c) = trio(&srv, &[("draft.md", "1\n")]);
    a.app_mv("draft.md", "final.md");
    a.app_mv("final.md", "draft.md");
    assert_eq!(a.sync().pushed, 0);
}

/// A rename in the app against a delete on another device ends as a rename
/// outside the app does, whichever device syncs first.
#[test]
fn rename_in_app_vs_delete_ends_as_a_rename_outside_the_app() {
    for laptop_first in [true, false] {
        let mut outcomes = Vec::new();
        for in_app in [true, false] {
            let srv = server();
            let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "keep me?\n"), ("anchor.md", "untouched\n")]);
            if in_app {
                a.app_mv("a.md", "renamed.md");
            } else {
                a.mv("a.md", "renamed.md");
            }
            b.rm("a.md");
            if laptop_first {
                a.sync();
                b.sync();
            } else {
                b.sync();
                a.sync();
            }
            converge(&mut [&mut a, &mut b, &mut c]);
            outcomes.push(a.files());
        }
        assert_eq!(outcomes[0], outcomes[1], "laptop_first={laptop_first}: in the app vs outside");
    }
}

/// A note under a name the sync chose here, a conflict copy name whose
/// rename has not gone up yet, which the user renames back to the note's
/// name on the server, outside the app, while another device deletes the
/// note: the rename wins, as any rename does, and the note goes up again,
/// so the other device gets it back. It is not taken for one of files that
/// swapped names (see `settle_renames`), which would leave it here only.
#[test]
fn a_note_renamed_back_to_its_server_name_while_another_device_deletes_it_comes_back() {
    let srv = server();
    let mut b = Device::new(&srv, "b", &[("anchor.md", "untouched\n"), ("x.md", "from b\n")]);
    assert_eq!(traffic(&mut b), (0, 2));
    let mut a = Device::new(&srv, "a", &[("x.md", "from a\n")]);
    // a's pull puts b's note under a conflict copy name. Before a uploads
    // anything, its user deletes a's own note and gives the copy its name,
    // and b deletes its note.
    let (root, b) = (a.root.clone(), Arc::new(Mutex::new(b)));
    let other = b.clone();
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        let names: Vec<String> = fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        let copy = names.iter().find(|n| n.contains("(conflict ")).expect("a conflict copy");
        fs::remove_file(root.join("x.md")).unwrap();
        fs::rename(root.join(copy), root.join("x.md")).unwrap();
        let mut b = other.lock();
        b.rm("x.md");
        assert_eq!(traffic(&mut b), (0, 1));
    }));
    let r = a.sync();
    assert_eq!((r.pulled, r.pushed), (3, 1), "a takes b's delete, and its rename wins");
    let mut b = Arc::try_unwrap(b).ok().expect("the hook ran").into_inner();
    assert_eq!(traffic(&mut b), (1, 0));
    assert_quiet(&mut [&mut a, &mut b]);
    for d in [&a, &b] {
        assert_eq!(d.files(), pairs(&[("anchor.md", "untouched\n"), ("x.md", "from b\n")]), "{}", d.name);
    }
}

/// The user deletes a note and gives another one its name in the app, then
/// edits it: the renamed note keeps its id and history, even when the
/// deleted one has newer changes.
#[test]
fn delete_then_rename_in_app_onto_its_name_keeps_the_renamed_note() {
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[("a.md", "new content\n"), ("b.md", "old content\n")]);
    a.write("b.md", "old content v2\n");
    a.sync();
    b.sync();
    a.vault.delete("b.md").unwrap();
    a.app_mv("a.md", "b.md");
    a.write("b.md", "new content v3\n");
    a.sync();
    b.sync();
    assert_eq!(b.files(), vec![("b.md".to_string(), "new content v3\n".to_string())]);
    let h = b.engine.history("b.md").unwrap();
    let first = b.engine.revision_content(h.last().unwrap().seq).unwrap();
    assert_eq!((h.len(), first.path.as_str()), (2, "a.md"), "the history of a.md goes on under b.md: {h:?}");
}

/// The user renames a note in the app while a sync's pull brings an edit of
/// it, then keeps typing: the edit waits for the next sync, which merges it
/// into the renamed note.
#[test]
fn rename_in_app_during_a_sync_then_edit() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("draft.md", "1\n2\n3\n4\n5\n"), ("anchor.md", "untouched\n")]);
    b.write("draft.md", "1\n2\n3\n4\n5 phone\n");
    b.sync();
    let (vault, dir) = (a.vault.clone(), a.state_dir.clone());
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        cairn_sync::engine::rename(&vault, &dir, "draft.md", "final.md").unwrap();
    }));
    a.sync();
    a.write("final.md", &format!("1 laptop\n{}", a.read("final.md").unwrap().split_once('\n').unwrap().1));
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(
        c.files(),
        vec![
            ("anchor.md".to_string(), "untouched\n".to_string()),
            ("final.md".to_string(), "1 laptop\n2\n3\n4\n5 phone\n".to_string())
        ]
    );
}

/// A rename in the app while a sync's pull deletes the note and moves
/// another one to its old name: the rename is not taken for the other note,
/// which keeps its new name and its id, and the renamed one is kept, as for
/// a rename the scan finds against a remote delete.
#[test]
fn rename_in_app_during_a_sync_that_brings_another_note_under_the_old_name() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("draft.md", "draft\n"), ("other.md", "other\n")]);
    b.rm("draft.md");
    b.sync();
    b.mv("other.md", "draft.md");
    b.sync();
    let (vault, dir) = (a.vault.clone(), a.state_dir.clone());
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        cairn_sync::engine::rename(&vault, &dir, "draft.md", "final.md").unwrap();
    }));
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(c.files(), vec![("draft.md".to_string(), "other\n".to_string()), ("final.md".to_string(), "draft\n".to_string())]);
    assert_eq!(c.engine.history("draft.md").unwrap().len(), 2, "other.md's history goes on");
    assert_eq!(c.engine.history("final.md").unwrap().len(), 1, "the renamed note is kept as a new file");
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect()
}

/// The path a file's first revision had, as `dev` sees the history of the
/// file at `path`.
fn first_path(dev: &Device, path: &str) -> String {
    let h = dev.engine.history(path).unwrap();
    dev.engine.revision_content(h.last().unwrap().seq).unwrap().path
}

/// Sync `dev`, which uploads two changes, with `other` syncing between the
/// two uploads. Returns the conflict copies `other` made then, and `other`.
fn sync_with_a_sync_between_uploads(dev: &mut Device, other: Device) -> (Vec<String>, Device) {
    let other = Arc::new(Mutex::new(other));
    let made = Arc::new(Mutex::new(Vec::new()));
    let (hooks, o, m) = (dev.hooks.clone(), other.clone(), made.clone());
    *dev.hooks.before_put.lock() = Some(Box::new(move || {
        *hooks.before_put.lock() = Some(Box::new(move || m.lock().extend(o.lock().sync().conflicts)));
    }));
    assert_eq!(dev.sync().pushed, 2);
    let made = made.lock().clone();
    (made, Arc::try_unwrap(other).ok().expect("the hook ran").into_inner())
}

/// Two renames in the app before a sync, the first freeing the name the
/// second takes (a.md -> z.md, then b.md -> a.md; the uploads by path would
/// put the second first): every device ends with both names, and each note
/// keeps its id.
#[test]
fn renames_in_app_that_free_and_take_a_name_keep_both_names() {
    for edit in [false, true] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "old a\n"), ("b.md", "new b\n")]);
        a.app_mv("a.md", "z.md");
        a.app_mv("b.md", "a.md");
        let new = if edit { "new b\nmore\n" } else { "new b\n" };
        a.write("a.md", new);
        a.sync();
        let r = b.sync();
        assert!(r.conflicts.is_empty(), "edit={edit}: phone made {:?}", r.conflicts);
        converge(&mut [&mut a, &mut b, &mut c]);
        for d in [&a, &b, &c] {
            assert_eq!(d.files(), pairs(&[("a.md", new), ("z.md", "old a\n")]), "edit={edit}: {}", d.name);
        }
        assert_eq!((first_path(&c, "a.md"), first_path(&c, "z.md")), ("b.md".into(), "a.md".into()), "edit={edit}");
    }
}

/// As above, with the phone syncing between the laptop's two uploads: the
/// one that frees a.md must go up first.
#[test]
fn renames_in_app_that_free_and_take_a_name_seen_between_uploads() {
    let srv = server();
    let (mut a, b, mut c) = trio(&srv, &[("a.md", "old a\n"), ("b.md", "new b\n")]);
    a.app_mv("a.md", "z.md");
    a.app_mv("b.md", "a.md");
    let (made, mut b) = sync_with_a_sync_between_uploads(&mut a, b);
    assert!(made.is_empty(), "phone made {made:?}");
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(c.files(), pairs(&[("a.md", "new b\n"), ("z.md", "old a\n")]));
}

/// As above, received by a device that missed that sync and the next one,
/// which changed z.md again: the feed then has the change of z.md after the
/// rename onto a.md, and the device must still move z.md out of the way
/// first.
#[test]
fn renames_in_app_that_free_and_take_a_name_received_late() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Plan.md", "old plan\n"), ("Plan v2.md", "new plan\n"), ("anchor.md", "x\n")]);
    a.app_mv("Plan.md", "Plan_old.md");
    a.app_mv("Plan v2.md", "Plan.md");
    a.sync();
    a.write("Plan_old.md", "old plan\nkept for reference\n");
    a.sync();
    let r = b.sync();
    assert!(r.conflicts.is_empty(), "phone made {:?}", r.conflicts);
    converge(&mut [&mut a, &mut b, &mut c]);
    let want = pairs(&[("Plan.md", "new plan\n"), ("Plan_old.md", "old plan\nkept for reference\n"), ("anchor.md", "x\n")]);
    assert_eq!((a.files(), b.files()), (want.clone(), want));
}

/// Two names swapped in the app through a temporary name, before a sync:
/// the notes go up as edits of each other, as when renamed outside the app
/// (as two renames, a device that syncs between the uploads could not apply
/// them). Every device ends with the right content under each name and
/// nothing in the trash.
#[test]
fn swap_in_app_ends_with_the_right_names() {
    for edit in [false, true] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "alpha\n"), ("b.md", "beta\n")]);
        a.app_mv("a.md", "tmp.md");
        a.app_mv("b.md", "a.md");
        a.app_mv("tmp.md", "b.md");
        let (na, nb) = if edit { ("beta\nmore\n", "alpha\nmore\n") } else { ("beta\n", "alpha\n") };
        a.write("a.md", na);
        a.write("b.md", nb);
        a.sync();
        let r = b.sync();
        assert!(r.conflicts.is_empty(), "edit={edit}: phone made {:?}", r.conflicts);
        converge(&mut [&mut a, &mut b, &mut c]);
        for d in [&a, &b, &c] {
            assert_eq!(d.files(), pairs(&[("a.md", na), ("b.md", nb)]), "edit={edit}: {}", d.name);
            assert_eq!(d.trash_text(), "", "edit={edit}: {}", d.name);
        }
    }
}

/// As above, with the phone syncing between the laptop's two uploads: as
/// renames, the first would want a name the other note still has there.
#[test]
fn swap_in_app_seen_between_uploads() {
    let srv = server();
    let (mut a, b, mut c) = trio(&srv, &[("a.md", "alpha\n"), ("b.md", "beta\n")]);
    a.app_mv("a.md", "tmp.md");
    a.app_mv("b.md", "a.md");
    a.app_mv("tmp.md", "b.md");
    let (made, mut b) = sync_with_a_sync_between_uploads(&mut a, b);
    assert!(made.is_empty(), "phone made {made:?}");
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(c.files(), pairs(&[("a.md", "beta\n"), ("b.md", "alpha\n")]));
}

/// A swap in the app with a third note moved onto a freed name in the
/// same sync: the swap goes up as edits, the third note as a rename.
#[test]
fn swap_in_app_next_to_a_rename_onto_a_freed_name() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "alpha\n"), ("b.md", "beta\n"), ("c.md", "gamma\n")]);
    a.app_mv("a.md", "tmp.md");
    a.app_mv("b.md", "a.md");
    a.app_mv("tmp.md", "b.md");
    a.app_mv("c.md", "d.md");
    a.write("d.md", "gamma\nmore\n");
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(c.files(), pairs(&[("a.md", "beta\n"), ("b.md", "alpha\n"), ("d.md", "gamma\nmore\n")]));
    assert_eq!(first_path(&c, "d.md"), "c.md");
}

/// Renamed in the app, then renamed back outside the app and edited before
/// the next sync: an edit of the note, which merges with an edit made
/// elsewhere, as if it had never been renamed.
#[test]
fn rename_in_app_undone_outside_the_app_is_an_edit() {
    for edit in [false, true] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("draft.md", "1\n2\n3\n"), ("anchor.md", "x\n")]);
        a.app_mv("draft.md", "final.md");
        a.mv("final.md", "draft.md");
        if edit {
            a.write("draft.md", "1 laptop\n2\n3\n");
        }
        b.write("draft.md", "1\n2\n3 phone\n");
        b.sync();
        let r = a.sync();
        assert_eq!(r.pushed, usize::from(edit), "edit={edit}");
        converge(&mut [&mut a, &mut b, &mut c]);
        let want = if edit { "1 laptop\n2\n3 phone\n" } else { "1\n2\n3 phone\n" };
        assert_eq!(c.files(), pairs(&[("anchor.md", "x\n"), ("draft.md", want)]), "edit={edit}");
        assert_eq!(c.trash_text(), "", "edit={edit}");
    }
}

/// Renames in the app on the laptop, among a few names, so that many take a
/// name another one freed (chains, and rings such as swaps), with edits and
/// syncs at random points, while the other devices sync at random points.
/// Every device must end with the laptop's notes, with no conflict copy
/// made. With the other devices editing too, the devices must converge and
/// no line may be lost (a ring goes up as edits, and an edit elsewhere of a
/// note in it can then give a conflict copy). Seeds: CAIRN_FUZZ_SEEDS
/// (default 30).
#[test]
fn renames_in_app_reach_lagging_devices_under_their_names() {
    let seeds: u64 = std::env::var("CAIRN_FUZZ_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    let names: Vec<String> = (0..6).map(|i| format!("n{i}.md")).chain(["tmp.md".into(), "d/n0.md".into()]).collect();
    for edits in [false, true] {
        for seed in 0..seeds {
            let srv = server();
            // Each device edits its own part of a note, so edits merge.
            let init: Vec<(String, String)> = (0..4).map(|i| (format!("n{i}.md"), format!("id{i}\n--1\n--2\n"))).collect();
            let (mut a, mut b, mut c) = trio(&srv, &init.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect::<Vec<_>>());
            let mut rng = Rng::new(seed + 7);
            let (mut conflicts, mut lines) = (Vec::new(), Vec::new());
            for step in 0..40 {
                match rng.below(6) {
                    0..=2 => {
                        let here = a.paths();
                        let from = rng.pick(&here).clone();
                        let free: Vec<&String> = names.iter().filter(|n| !here.contains(n)).collect();
                        a.app_mv(&from, rng.pick(&free));
                    }
                    3 => {
                        let here = a.paths();
                        let p = rng.pick(&here).clone();
                        a.write(&p, &format!("{}laptop{step}\n", a.read(&p).unwrap()));
                        lines.push(format!("laptop{step}"));
                    }
                    4 => conflicts.extend(a.sync().conflicts),
                    _ => {
                        let d = if rng.chance(50) { &mut b } else { &mut c };
                        if edits {
                            let here = d.paths();
                            let p = rng.pick(&here).clone();
                            let t = d.read(&p).unwrap();
                            let mark = if d.name == "phone" { format!("{}\n", t.lines().next().unwrap()) } else { "--1\n".into() };
                            let (head, rest) = t.split_once(&mark).unwrap();
                            d.write(&p, &format!("{head}{mark}{}{step}\n{rest}", d.name));
                            lines.push(format!("{}{step}", d.name));
                        }
                        conflicts.extend(d.sync().conflicts);
                    }
                }
            }
            conflicts.extend(a.sync().conflicts);
            let laptop = a.files();
            converge(&mut [&mut a, &mut b, &mut c]);
            let ctx = format!("edits={edits} seed={seed}");
            assert_eq!(b.files(), a.files(), "{ctx}");
            assert_eq!(c.files(), a.files(), "{ctx}");
            let all = a.all_text();
            for l in &lines {
                assert!(all.contains(&format!("{l}\n")), "{ctx}: {l} lost: {:?}", a.files());
            }
            if !edits {
                assert!(conflicts.is_empty(), "{ctx}: conflict copies {conflicts:?}");
                assert_eq!(a.files(), laptop, "{ctx}");
            }
        }
    }
}

/// Replace a note by moving another one onto its name (delete b.md, then
/// rename a.md to b.md). The engine sees this as an edit of b.md plus a
/// delete of a.md, which arrives correctly everywhere (checked 12 times).
#[test]
fn replace_by_rename_arrives_under_the_right_name() {
    let mut bad = Vec::new();
    for i in 0..12 {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "new content\n"), ("b.md", "old content\n")]);
        a.rm("b.md");
        a.mv("a.md", "b.md");
        a.sync();
        b.sync();
        c.sync();
        if b.paths() != vec!["b.md".to_string()] {
            bad.push((i, b.paths()));
        }
    }
    assert!(bad.is_empty(), "runs where the phone did not end with just b.md: {bad:?}");
}

#[test]
fn rename_rename_edit_on_three_devices() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "1\n2\n3\n")]);
    a.mv("a.md", "laptop-name.md");
    b.mv("a.md", "phone-name.md");
    c.write("a.md", "1\n2\n3\n4 tablet\n");
    c.sync();
    b.sync();
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.files(), vec![("phone-name.md".to_string(), "1\n2\n3\n4 tablet\n".to_string())]);
}

/// Delete on one device, rename on a second, edit on a third. Nothing is
/// lost, and the renaming device must not re-upload the note as a NEW file
/// (see FINDING-148), or the tablet's edit would revive the old name and
/// the renamed copy would stay stale: two diverging copies of one note.
#[test]
fn delete_rename_edit_on_three_devices() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "1\n2\n3\n"), ("anchor.md", "untouched\n")]);
    a.rm("a.md");
    b.mv("a.md", "moved/a.md");
    c.write("a.md", "1\n2\n3\n4 tablet\n");
    a.sync();
    b.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let all: String = a.files().into_iter().map(|f| f.1).collect();
    assert!(all.contains("4 tablet"), "edit lost: {:?}", a.files());
    assert_eq!(a.files().len(), 2, "{:?}", a.files()); // the note and anchor.md
}

/// The file id every device tracks at `path`, which must be the same on all.
fn file_id_at(devs: &[&Device], path: &str) -> String {
    let ids: Vec<Option<String>> = devs
        .iter()
        .map(|d| d.engine.state().files.iter().find(|(_, t)| !t.deleted && t.path == path).map(|(f, _)| f.clone()))
        .collect();
    assert!(ids.iter().all(|i| i.is_some() && *i == ids[0]), "file ids at {path}: {ids:?}");
    ids[0].clone().unwrap()
}

/// File ids the server has a live head for.
fn live_server_files(srv: &Server) -> Vec<String> {
    use cairn_sync::transport::Transport;
    let r = cairn_sync::transport::HttpTransport::new(&srv.url, TOKEN).changes("notes", 0, 500).unwrap();
    assert!(!r.more);
    let mut ids: Vec<String> = r.heads.into_iter().filter(|h| !h.deleted).map(|h| h.file_id).collect();
    ids.sort();
    ids
}

/// FINDING-148, by design: rename vs delete keeps the renamed file
/// whichever device syncs first, under its old file id, so its version
/// history goes on and no second file is uploaded.
#[test]
fn rename_vs_delete_keeps_the_renamed_file_and_its_id_in_both_orders() {
    for rename_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "keep me?\n"), ("anchor.md", "untouched\n")]);
        let fid = file_id_at(&[&a, &b, &c], "a.md");
        let anchor = file_id_at(&[&a, &b, &c], "anchor.md");
        let first = a.engine.history("a.md").unwrap().last().unwrap().seq;
        a.mv("a.md", "renamed.md");
        b.rm("a.md");
        if rename_first {
            a.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        let want = vec![("anchor.md".to_string(), "untouched\n".to_string()), ("renamed.md".to_string(), "keep me?\n".to_string())];
        assert_eq!(a.files(), want, "rename_first={rename_first}");
        assert!(!a.trash_text().contains("keep me?"), "the renaming device trashed it (rename_first={rename_first})");
        assert_eq!(file_id_at(&[&a, &b, &c], "renamed.md"), fid, "rename_first={rename_first}");
        let mut server_ids = vec![fid.clone(), anchor];
        server_ids.sort();
        assert_eq!(live_server_files(&srv), server_ids, "rename_first={rename_first}");
        for d in [&a, &b, &c] {
            let h = d.engine.history("renamed.md").unwrap();
            assert_eq!(h.last().map(|e| e.seq), Some(first), "history on {} (rename_first={rename_first}): {h:?}", d.name);
            assert!(!h[0].deleted, "{h:?}");
        }
        // nothing left to do
        for d in [&mut a, &mut b, &mut c] {
            let r = d.sync();
            assert!(r.pushed == 0 && r.pulled == 0 && r.conflicts.is_empty(), "{} keeps changing: {r:?}", d.name);
        }
    }
}

/// Delete on one device, rename on a second, edit on a third, synced in
/// every order: the rename and the edit both win over the delete, and the
/// note stays one file with one history.
#[test]
fn delete_rename_edit_in_any_order_gives_one_renamed_note() {
    const ORDERS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    for order in ORDERS {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "1\n2\n3\n"), ("anchor.md", "untouched\n")]);
        let fid = file_id_at(&[&a, &b, &c], "a.md");
        let first = a.engine.history("a.md").unwrap().last().unwrap().seq;
        a.rm("a.md");
        b.mv("a.md", "moved/a.md");
        c.write("a.md", "1\n2\n3\n4 tablet\n");
        for i in order {
            [&mut a, &mut b, &mut c][i].sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        let want = vec![("anchor.md".to_string(), "untouched\n".to_string()), ("moved/a.md".to_string(), "1\n2\n3\n4 tablet\n".to_string())];
        assert_eq!(a.files(), want, "order {order:?}");
        assert_eq!(file_id_at(&[&a, &b, &c], "moved/a.md"), fid, "order {order:?}");
        assert_eq!(live_server_files(&srv).len(), 2, "order {order:?}");
        let h = c.engine.history("moved/a.md").unwrap();
        assert_eq!(h.last().map(|e| e.seq), Some(first), "order {order:?}: {h:?}");
    }
}

// ===================================================================== folders

#[test]
fn folder_rename_with_edit_inside_on_other_device() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Inbox/a.md", "1\n2\n"), ("Inbox/sub/b.md", "b\n")]);
    a.mv("Inbox", "Archive/2026");
    b.write("Inbox/a.md", "1\n2\n3 phone\n");
    c.write("Inbox/sub/b.md", "b\nb tablet\n");
    a.sync();
    b.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(
        a.files(),
        vec![
            ("Archive/2026/a.md".to_string(), "1\n2\n3 phone\n".to_string()),
            ("Archive/2026/sub/b.md".to_string(), "b\nb tablet\n".to_string())
        ]
    );
    assert!(!b.abs("Inbox").exists() && !c.abs("Inbox").exists());
}

#[test]
fn folder_rename_with_new_file_inside_on_other_device() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Inbox/a.md", "a\n")]);
    a.mv("Inbox", "Archive");
    b.write("Inbox/new.md", "new\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    // File-based sync: the new file stays in the old folder name.
    assert_eq!(
        a.files(),
        vec![("Archive/a.md".to_string(), "a\n".to_string()), ("Inbox/new.md".to_string(), "new\n".to_string())]
    );
}

/// Sync sees a folder rename as a rename of each file in it, and a rename
/// wins over a delete whichever device syncs first (FINDING-148): the file
/// deleted inside the folder is kept in the renamed folder.
#[test]
fn folder_rename_with_delete_inside_on_other_device() {
    for rename_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("Inbox/a.md", "a\n"), ("Inbox/b.md", "b\n")]);
        a.mv("Inbox", "Archive");
        b.rm("Inbox/a.md");
        if rename_first {
            a.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        let want = vec![("Archive/a.md".to_string(), "a\n".to_string()), ("Archive/b.md".to_string(), "b\n".to_string())];
        assert_eq!(a.files(), want, "rename_first={rename_first}");
        assert!(!b.abs("Inbox").exists(), "rename_first={rename_first}");
    }
}

#[test]
fn folder_rename_with_file_moved_out_on_other_device() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Inbox/a.md", "a\n"), ("Inbox/b.md", "b\n")]);
    a.mv("Inbox", "Archive");
    b.mv("Inbox/a.md", "Top/a.md");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let paths = a.paths();
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.contains(&"Archive/b.md".to_string()));
}

#[test]
fn folder_rename_with_edit_inside_on_same_device() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Inbox/a.md", "1\n2\n"), ("Inbox/b.md", "b\n")]);
    a.mv("Inbox", "Archive");
    a.write("Archive/a.md", "1\n2\n3 laptop\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(
        a.files(),
        vec![("Archive/a.md".to_string(), "1\n2\n3 laptop\n".to_string()), ("Archive/b.md".to_string(), "b\n".to_string())]
    );
}

#[test]
fn nested_folder_renames_on_two_devices() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("P/Q/R/n.md", "n\n"), ("P/Q/m.md", "m\n")]);
    a.mv("P/Q", "P/Q2");
    b.mv("P", "P0");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.files().len(), 2, "{:?}", a.paths());
}

#[test]
fn same_folder_renamed_to_different_names_on_two_devices() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Inbox/a.md", "a\n"), ("Inbox/b.md", "b\n"), ("Inbox/sub/c.md", "c\n")]);
    a.mv("Inbox", "Work");
    b.mv("Inbox", "Personal");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.paths(), vec!["Work/a.md".to_string(), "Work/b.md".to_string(), "Work/sub/c.md".to_string()]);
    assert!(!b.abs("Personal").exists(), "empty folder left behind on the phone");
}

// ===================================================================== deletes

#[test]
fn delete_then_recreate_same_path_without_sync_between() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("n.md", "old\n")]);
    a.rm("n.md");
    a.write("n.md", "brand new\n");
    a.sync();
    b.sync();
    c.sync();
    assert_eq!(b.read("n.md").as_deref(), Some("brand new\n"));
    assert_eq!(c.read("n.md").as_deref(), Some("brand new\n"));
}

#[test]
fn delete_then_recreate_same_path_with_sync_between() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("n.md", "old\n"), ("anchor.md", "untouched\n")]);
    a.rm("n.md");
    a.sync();
    b.sync();
    assert_eq!(b.read("n.md"), None);
    a.write("n.md", "brand new\n");
    a.sync();
    b.sync();
    c.sync(); // c never saw the delete
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.files(), vec![("anchor.md".to_string(), "untouched\n".to_string()), ("n.md".to_string(), "brand new\n".to_string())]);
}

#[test]
fn delete_vs_edit_edit_wins_with_three_devices() {
    for deleter_first in [true, false] {
        let srv = server();
        let (mut a, mut b, mut c) = trio(&srv, &[("x.md", "v1\n"), ("anchor.md", "untouched\n")]);
        a.rm("x.md");
        b.write("x.md", "v1\nv2 phone\n");
        if deleter_first {
            a.sync();
            c.sync();
            b.sync();
        } else {
            b.sync();
            a.sync();
        }
        converge(&mut [&mut a, &mut b, &mut c]);
        assert_eq!(
            a.files(),
            vec![("anchor.md".to_string(), "untouched\n".to_string()), ("x.md".to_string(), "v1\nv2 phone\n".to_string())],
            "deleter_first={deleter_first}"
        );
    }
}

#[test]
fn delete_of_one_of_two_identical_files() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("a.md", "same\n"), ("b.md", "same\n"), ("c.md", "")]);
    a.rm("a.md");
    a.sync();
    b.sync();
    c.sync();
    assert_eq!(b.paths(), vec!["b.md".to_string(), "c.md".to_string()]);
    // and the other way, with a rename of the twin in the same sync
    b.write("d.md", "same\n");
    b.sync();
    a.sync();
    a.rm("b.md");
    a.mv("d.md", "e.md");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.paths(), vec!["c.md".to_string(), "e.md".to_string()]);
}

/// The engine classifies local files once, before the pull. If the user
/// saves a note after that scan and the server has a deletion for it, the
/// fresh edit must still win ("edit vs delete: the edit wins"): it stays in
/// place and is uploaded, instead of going to the trash.
#[test]
fn local_edit_during_sync_beats_remote_delete() {
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[("n.md", "v1\n"), ("anchor.md", "untouched\n")]);
    b.rm("n.md");
    b.sync();
    // The user types in n.md on the laptop while its sync is talking to
    // the server (after the scan, before the pull is applied).
    let root = a.root.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        std::thread::sleep(Duration::from_millis(3));
        fs::write(root.join("n.md"), "v1\nimportant edit typed during sync\n").unwrap();
    }));
    a.sync();
    a.sync();
    b.sync();
    assert_eq!(
        a.read("n.md").as_deref(),
        Some("v1\nimportant edit typed during sync\n"),
        "laptop tree: {:?}; trash: {:?}",
        a.paths(),
        a.trash_text()
    );
    assert_eq!(b.read("n.md").as_deref(), Some("v1\nimportant edit typed during sync\n"));
}

/// Same window, other direction: a remote NEW file arrives for a path that
/// was free at scan time. A file the user creates at that path in the
/// meantime with another program (not through Cairn, so the index does not
/// know it yet) must not be overwritten: its text is kept on the device.
#[test]
fn file_created_outside_cairn_during_sync_is_not_overwritten() {
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[]);
    b.write("Inbox.md", "phone inbox\n");
    b.sync();
    let root = a.root.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        fs::write(root.join("Inbox.md"), "typed in vim on the laptop\n").unwrap();
    }));
    a.sync();
    let all = a.all_text();
    eprintln!("laptop files: {:?}", a.files());
    assert!(all.contains("typed in vim on the laptop"), "laptop's own Inbox.md is gone: {:?} trash {:?}", a.files(), a.trash_text());
}

#[test]
fn recreate_after_delete_on_two_devices() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("n.md", "old\n"), ("anchor.md", "untouched\n")]);
    a.rm("n.md");
    a.sync();
    b.sync();
    c.sync();
    a.write("n.md", "laptop new\n");
    b.write("n.md", "phone new\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let files = a.files();
    assert!(files.iter().any(|f| f.1 == "laptop new\n"), "{files:?}");
    assert!(files.iter().any(|f| f.1 == "phone new\n"), "{files:?}");
}

/// `SyncEngine::history(path)` must take the live file whose tracked path
/// is `path`, not a deleted one. After a note is deleted and another note
/// is later renamed to the same name, the version history of the new note
/// shows the new note's revisions, not the deleted note's.
#[test]
fn history_belongs_to_the_current_note_at_that_path() {
    let mut wrong = Vec::new();
    for attempt in 0..10 {
        let srv = server();
        let mut a = Device::new(&srv, "laptop", &[("n.md", "deleted note v1\n"), ("anchor.md", "untouched\n")]);
        a.sync();
        a.write("n.md", "deleted note v2\n");
        a.sync();
        a.rm("n.md");
        a.sync();
        a.write("other.md", "current note\n");
        a.sync();
        a.mv("other.md", "n.md");
        a.sync();
        let h = a.engine.history("n.md").unwrap();
        let newest = a.engine.revision_content(h[0].seq).unwrap();
        let text = String::from_utf8_lossy(&newest.data).into_owned();
        if text != "current note\n" {
            wrong.push(format!("attempt {attempt}: newest history entry of n.md is {text:?} ({} entries)", h.len()));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

// ===================================================================== hidden folders

#[test]
fn move_into_hidden_folder_is_a_delete_elsewhere_and_back_out_revives() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Ideas.md", "idea\n"), ("anchor.md", "untouched\n")]);
    a.mv("Ideas.md", ".archive/Ideas.md");
    a.sync();
    b.sync();
    c.sync();
    // The other devices lose it from the tree; it is in their trash.
    assert_eq!(b.read("Ideas.md"), None);
    assert!(b.trash_text().contains("idea"));
    // Moving it back out brings it back everywhere.
    a.mv(".archive/Ideas.md", "Ideas.md");
    a.sync();
    b.sync();
    c.sync();
    assert_eq!(b.read("Ideas.md").as_deref(), Some("idea\n"));
    assert_eq!(c.read("Ideas.md").as_deref(), Some("idea\n"));
}

#[test]
fn move_into_hidden_folder_vs_edit_elsewhere() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Ideas.md", "idea\n"), ("anchor.md", "untouched\n")]);
    a.mv("Ideas.md", ".archive/Ideas.md");
    b.write("Ideas.md", "idea\nmore from phone\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.read("Ideas.md").as_deref(), Some("idea\nmore from phone\n"));
}

// ===================================================================== case

#[test]
fn case_only_rename_propagates() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Note.md", "x\n")]);
    a.mv("Note.md", "note.md");
    a.sync();
    b.sync();
    c.sync();
    assert_eq!(b.paths(), vec!["note.md".to_string()]);
    assert_eq!(c.paths(), vec!["note.md".to_string()]);
}

#[test]
fn case_only_rename_vs_edit() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("Note.md", "x\n")]);
    a.mv("Note.md", "note.md");
    b.write("Note.md", "x\ny\n");
    b.sync();
    a.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.files(), vec![("note.md".to_string(), "x\ny\n".to_string())]);
}

#[test]
fn case_only_rename_received_on_case_insensitive_device() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[("Note.md", "x\n")]);
    a.sync();
    let mut m = Device::with_fs(&srv, "mac", &[], CaseInsensitiveFs::new);
    m.sync();
    a.mv("Note.md", "note.md");
    a.sync();
    m.sync();
    assert_eq!(m.paths(), vec!["note.md".to_string()]);
    converge(&mut [&mut a, &mut m]);
}

/// Linux (and Android app storage) allow `Note.md` and `note.md` side by
/// side. On a case-insensitive device (macOS, Windows) the second download
/// must not overwrite the first (nor be uploaded back as an "edit"): a
/// name that differs only in case counts as taken, so that file gets a
/// conflict copy name, as PLAN section 6 says ("sync gives a pulled case
/// twin a conflict copy name on macOS and Windows"). Both notes survive on
/// both devices.
#[cfg(target_os = "linux")] // Case twins need a case-sensitive file system.
#[test]
fn two_files_differing_only_in_case_survive_a_case_insensitive_device() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[("Note.md", "upper case note\n"), ("note.md", "lower case note\n")]);
    a.sync();
    let mut m = Device::with_fs(&srv, "mac", &[], CaseInsensitiveFs::new);
    let r = m.sync();
    eprintln!("mac first sync: {r:?}");
    for _ in 0..2 {
        let _ = a.try_sync();
        let _ = m.try_sync();
    }
    let linux_text = a.files().into_iter().map(|f| f.1).collect::<String>();
    let mac_text = m.files().into_iter().map(|f| f.1).collect::<String>();
    eprintln!("linux files: {:?}\nlinux trash: {:?}\nmac files: {:?}", a.files(), a.trash_text(), m.files());
    for w in ["upper case note", "lower case note"] {
        assert!(mac_text.contains(w), "{w} missing on the mac: {:?}", m.files());
        assert!(linux_text.contains(w), "{w} missing on linux: {:?}", a.files());
    }
}

/// Worse variant: the case-insensitive device has its own, never synced
/// `Note.md`; a remote `note.md` must not be written into it. That text was
/// never uploaded, so it would be on no device and in no server history.
#[test]
fn remote_file_differing_in_case_does_not_overwrite_local_unsynced_note() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[("note.md", "from linux\n")]);
    a.sync();
    let mut m = Device::with_fs(&srv, "mac", &[("Note.md", "only on the mac, never synced\n")], CaseInsensitiveFs::new);
    let r = m.try_sync();
    let _ = a.try_sync();
    let everywhere = format!("{}{}", m.all_text(), a.all_text());
    eprintln!("mac sync: {:?}; mac files {:?}", r.map(|r| r.conflicts).map_err(|e| e.to_string()), m.files());
    assert!(everywhere.contains("only on the mac, never synced"), "the mac's own note is gone: mac {:?}, linux {:?}", m.files(), a.files());
}

/// Sync `a` then `b` for `rounds` rounds and return what each sync pulled
/// and pushed: `(a.pulled, a.pushed, b.pulled, b.pushed)` per round.
fn sync_rounds(a: &mut Device, b: &mut Device, rounds: usize) -> Vec<(usize, usize, usize, usize)> {
    (0..rounds)
        .map(|_| {
            let ra = a.sync();
            let rb = b.sync();
            (ra.pulled, ra.pushed, rb.pulled, rb.pushed)
        })
        .collect()
}

/// Two case-insensitive devices (Windows, macOS) hold one folder under
/// spellings that differ only in case: `Drafts` on one, `DRAFTS` on the
/// other. Each disk keeps its own spelling of the folder, so the devices
/// must stop moving its files back and forth after a few syncs, with every
/// note and its text on both.
#[test]
fn folder_spelled_differently_on_two_case_insensitive_devices_settles() {
    let srv = server();
    let mut a = Device::with_fs(&srv, "win-a", &[("Drafts/a.md", "from a\n")], CaseInsensitiveFs::new);
    let mut b = Device::with_fs(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")], CaseInsensitiveFs::new);
    let counts = sync_rounds(&mut a, &mut b, 5);
    eprintln!("per round (a pulled, a pushed, b pulled, b pushed): {counts:?}");
    eprintln!("a: {:?}\nb: {:?}", a.files(), b.files());
    assert!(counts[2..].iter().all(|c| *c == (0, 0, 0, 0)), "sync never settled: {counts:?}");
    for d in [&a, &b] {
        let text: String = d.files().into_iter().map(|f| f.1).collect();
        assert!(text.contains("from a") && text.contains("from b"), "a note is missing on {}: {:?}", d.name, d.files());
        assert!(d.conflict_copies().is_empty(), "conflict copies on {}: {:?}", d.name, d.files());
    }
    // An edit made after that still reaches the other device.
    let edited = a.paths().into_iter().find(|p| p.ends_with("a.md")).unwrap();
    a.write(&edited, "from a, edited\n");
    sync_rounds(&mut a, &mut b, 1);
    let text: String = b.files().into_iter().map(|f| f.1).collect();
    assert!(text.contains("from a, edited"), "the edit did not reach b: {:?}", b.files());
    let counts = sync_rounds(&mut a, &mut b, 2);
    assert!(counts.iter().all(|c| *c == (0, 0, 0, 0)), "sync did not settle after the edit: {counts:?}");
}

/// The same after a case-only folder rename on one case-insensitive device
/// (`proj` to `Proj` in File Explorer) once both were in sync.
#[test]
fn case_only_folder_rename_between_case_insensitive_devices_settles() {
    let srv = server();
    let files: Vec<(String, String)> = (0..5).map(|i| (format!("proj/n{i}.md"), format!("note {i}\n"))).collect();
    let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let mut a = Device::with_fs(&srv, "win-a", &files, CaseInsensitiveFs::new);
    a.sync();
    let mut b = Device::with_fs(&srv, "win-b", &[], CaseInsensitiveFs::new);
    b.sync();
    assert_eq!(sync_rounds(&mut a, &mut b, 1), vec![(0, 0, 0, 0)]);
    a.mv("proj", "Proj");
    let counts = sync_rounds(&mut a, &mut b, 5);
    eprintln!("per round (a pulled, a pushed, b pulled, b pushed): {counts:?}");
    eprintln!("a: {:?}\nb: {:?}", a.paths(), b.paths());
    assert!(counts[2..].iter().all(|c| *c == (0, 0, 0, 0)), "sync never settled: {counts:?}");
    for d in [&a, &b] {
        assert_eq!(d.files().len(), 5, "notes missing on {}: {:?}", d.name, d.paths());
        assert!(d.conflict_copies().is_empty(), "conflict copies on {}: {:?}", d.name, d.paths());
    }
}

/// A device whose file system ignores case but keeps it (Windows, macOS).
fn ci(srv: &Server, name: &str, files: &[(&str, &str)]) -> Device {
    Device::with_fs(srv, name, files, CaseInsensitiveFs::new)
}

/// Sync `d` once and return what it pulled and pushed. Such a sync must
/// make no conflict copy and leave no file out.
fn traffic(d: &mut Device) -> (usize, usize) {
    let r = d.sync();
    assert!(r.conflicts.is_empty(), "{}: conflict copies {:?}", d.name, r.conflicts);
    assert!(r.skipped.is_empty(), "{}: files not synced {:?}", d.name, r.skipped);
    (r.pulled, r.pushed)
}

/// Sync the devices in turn, twice: none may pull or push anything.
fn assert_quiet(devs: &mut [&mut Device]) {
    for round in 0..2 {
        for d in devs.iter_mut() {
            assert_eq!(traffic(d), (0, 0), "{} is not quiet in round {round}", d.name);
        }
    }
}

/// The notes on `d` by file name, with their text, whatever the spelling of
/// their folder. Nothing may be in a conflict copy or in the trash.
fn notes(d: &Device) -> Vec<(String, String)> {
    assert!(d.conflict_copies().is_empty(), "{}: {:?}", d.name, d.paths());
    assert!(d.trash_text().is_empty(), "{}: trash {:?}", d.name, d.trash_text());
    let mut v: Vec<(String, String)> = d.files().into_iter().map(|(p, t)| (p.rsplit('/').next().unwrap().to_string(), t)).collect();
    v.sort();
    v
}

/// The server's spelling of the note named `name` that `d` keeps in its sync
/// state, if `d` spells the note's folder otherwise.
fn server_spelling(d: &Device, name: &str) -> Option<String> {
    let found: Vec<&Tracked> = d.engine.state().files.values().filter(|t| !t.deleted && t.path.rsplit('/').next() == Some(name)).collect();
    assert_eq!(found.len(), 1, "{}: {name} is tracked {} times", d.name, found.len());
    found[0].server_spelling.clone()
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// The folders at the top of `d`'s notebook.
fn top_folders(d: &Device) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(&d.root)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

/// After two such devices settle, an edit on either reaches the other once,
/// in its own spelling of the folder, and keeps the server's spelling: no
/// note moves on the server. A note edited on both, on lines apart, is
/// merged.
#[test]
fn edits_after_folder_spellings_settle_go_up_once_and_merge() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "1\n2\n3\n4\n5\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    // a edits its own note, then the note b made.
    a.write("Drafts/a.md", "1 a\n2\n3\n4\n5\n");
    assert_eq!(sync_rounds(&mut a, &mut b, 1), vec![(0, 1, 1, 0)]);
    assert_eq!(b.read("DRAFTS/a.md").as_deref(), Some("1 a\n2\n3\n4\n5\n"));
    a.write("Drafts/b.md", "from b, edited on a\n");
    assert_eq!(sync_rounds(&mut a, &mut b, 1), vec![(0, 1, 1, 0)]);
    assert_eq!(b.read("DRAFTS/b.md").as_deref(), Some("from b, edited on a\n"));
    // Both edit a.md before they sync.
    a.write("Drafts/a.md", "1 a\n2\n3\n4\n5 a\n");
    b.write("DRAFTS/a.md", "1 a\n2 b\n3\n4\n5\n");
    assert_eq!(sync_rounds(&mut a, &mut b, 2), vec![(0, 1, 1, 1), (1, 0, 0, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    let want = pairs(&[("a.md", "1 a\n2 b\n3\n4\n5 a\n"), ("b.md", "from b, edited on a\n")]);
    assert_eq!(notes(&a), want);
    assert_eq!(notes(&b), want);
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/b.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/b.md"]));
    // Each note is still where its maker put it on the server.
    assert_eq!(server_spelling(&a, "a.md"), None);
    assert_eq!(server_spelling(&a, "b.md").as_deref(), Some("DRAFTS/b.md"));
    assert_eq!(server_spelling(&b, "a.md").as_deref(), Some("Drafts/a.md"));
    assert_eq!(server_spelling(&b, "b.md"), None);
}

/// The same note made on both devices, each in its own spelling of the
/// folder, is one note: adopted, with no second upload and no rename.
#[test]
fn same_note_in_folders_spelled_differently_is_adopted_without_a_rename() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/same.md", "same\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/same.md", "same\n")]);
    assert_eq!(sync_rounds(&mut a, &mut b, 1), vec![(0, 1, 1, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Drafts/same.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/same.md"]));
    assert_eq!(server_spelling(&b, "same.md").as_deref(), Some("Drafts/same.md"));
}

/// A case-only rename of a file still reaches a device that spells its
/// folder otherwise, in that device's folder: only folders keep the
/// device's spelling.
#[test]
fn case_only_file_rename_reaches_a_device_that_spells_the_folder_otherwise() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/Beta.md", "beta\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/x.md", "x\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    a.mv("Drafts/Beta.md", "Drafts/beta.md");
    assert_eq!(sync_rounds(&mut a, &mut b, 1), vec![(0, 1, 1, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Drafts/beta.md", "Drafts/x.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/beta.md", "DRAFTS/x.md"]));
}

/// A folder renamed on one case-insensitive device while a device that
/// spells it otherwise edits a note in it: the note goes into the renamed
/// folder with the edit on both devices, and the old folder is not made
/// again under the other spelling (as it would be if the edit's path were
/// taken for a move made on the server).
#[test]
fn a_folder_rename_racing_an_edit_from_a_device_that_spells_the_folder_otherwise() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    a.mv("Drafts", "Notes");
    b.write("DRAFTS/b.md", "from b, edited\n");
    assert_eq!(traffic(&mut b), (0, 1));
    // a writes b's edit into the renamed folder and pushes the rename of both notes.
    assert_eq!(traffic(&mut a), (1, 2));
    assert_eq!(traffic(&mut b), (2, 0));
    assert_quiet(&mut [&mut a, &mut b]);
    for d in [&a, &b] {
        assert_eq!(d.paths(), strings(&["Notes/a.md", "Notes/b.md"]), "{}", d.name);
        assert_eq!(top_folders(d), strings(&["Notes"]), "{}", d.name);
        assert_eq!(notes(d), pairs(&[("a.md", "from a\n"), ("b.md", "from b, edited\n")]), "{}", d.name);
    }
}

/// A case-only rename of the folder made in the app goes up once, for the
/// notes the server spells otherwise too, and the device that spells it
/// otherwise takes it for the server's spelling.
#[test]
fn a_case_only_folder_rename_made_in_the_app_goes_up_once() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    a.app_mv("Drafts", "drafts");
    assert_eq!(sync_rounds(&mut a, &mut b, 1), vec![(0, 2, 2, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["drafts/a.md", "drafts/b.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/b.md"]));
    assert_eq!(server_spelling(&a, "b.md"), None);
    assert_eq!(server_spelling(&b, "a.md").as_deref(), Some("drafts/a.md"));
    assert_eq!(server_spelling(&b, "b.md").as_deref(), Some("drafts/b.md"));
}

/// A case-only rename of the folder made in the app goes up for the notes
/// that were in it then, not for a note that another device made there
/// before the rename and that this device pulls with the sync that pushes
/// the rename: that one keeps the server's spelling. So a delete of it made
/// on the other device meanwhile still applies, and the note does not come
/// back as if this device had renamed it.
#[test]
fn a_case_only_folder_rename_made_in_the_app_leaves_a_note_pulled_after_it_alone() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("DRAFTS/a.md", "from a\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    b.write("DRAFTS/n.md", "from b\n");
    assert_eq!(traffic(&mut b), (0, 1));
    a.app_mv("DRAFTS", "Drafts");
    b.rm("DRAFTS/n.md");
    // a pulls n.md into the folder as it spells it, and pushes the rename of
    // a.md only.
    assert_eq!(traffic(&mut a), (1, 1));
    assert_eq!(server_spelling(&a, "n.md").as_deref(), Some("DRAFTS/n.md"));
    // b pushes its delete of n.md, which a then applies.
    assert_eq!(traffic(&mut b), (1, 1));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Drafts/a.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md"]));
}

/// As above, after a note in the folder that this device deleted since the
/// rename comes back in the same pull, as another device edited it (edit
/// beats delete): it comes back in the folder as this device spells it,
/// and the notes pulled after it still go there with the server's spelling
/// kept. Writing it under the spelling of the folder from before the rename
/// had the index list the folder under that spelling too, so the notes
/// after it were tracked so and their names here went up as renames, which
/// beat a delete made meanwhile.
#[test]
fn a_note_back_from_a_delete_in_a_folder_renamed_by_case_in_the_app_leaves_the_notes_after_it_alone() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("DRAFTS/a.md", "1\n2\n3\n"), ("DRAFTS/keep.md", "keep\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    b.write("DRAFTS/a.md", "1\n2 b\n3\n");
    assert_eq!(traffic(&mut b), (0, 1));
    b.write("DRAFTS/n.md", "from b\n");
    assert_eq!(traffic(&mut b), (0, 1));
    a.app_mv("DRAFTS", "Drafts");
    a.rm("Drafts/a.md");
    b.rm("DRAFTS/n.md");
    // a gets a.md back with b's edit and n.md, and pushes the rename of
    // keep.md and a.md, which was in the folder at the rename; not n.md.
    assert_eq!(traffic(&mut a), (2, 2));
    assert_eq!(server_spelling(&a, "n.md").as_deref(), Some("DRAFTS/n.md"));
    assert_eq!(traffic(&mut b), (2, 1));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/keep.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/keep.md"]));
    for d in [&a, &b] {
        assert_eq!(d.read(&format!("{}/a.md", top_folders(d)[0])).as_deref(), Some("1\n2 b\n3\n"), "{}", d.name);
    }
}

/// A case-only rename of a folder inside one that a case-insensitive device
/// spells otherwise than the server (`WORK/Notes` here, `Work/Notes` on the
/// server): only the renamed folder's new spelling goes up, and the folder
/// above it keeps the server's, so that a device that tells case apart
/// renames one folder, not both.
#[cfg(target_os = "linux")] // The peer needs a file system that tells case apart.
#[test]
fn a_case_only_rename_of_a_folder_in_one_spelled_otherwise_goes_up_for_that_folder_only() {
    let srv = server();
    let mut l = Device::new(&srv, "linux", &[("Work/Notes/n.md", "n\n"), ("Work/top.md", "top\n")]);
    let mut a = ci(&srv, "win-a", &[("WORK/a.md", "a\n")]);
    assert_eq!(traffic(&mut l), (0, 2));
    assert_eq!(traffic(&mut a), (2, 1));
    assert_eq!(traffic(&mut l), (1, 0));
    assert_eq!(a.paths(), strings(&["WORK/Notes/n.md", "WORK/a.md", "WORK/top.md"]));
    a.mv("WORK/Notes", "WORK/NOTES");
    assert_eq!(traffic(&mut a), (0, 1));
    assert_eq!(traffic(&mut l), (1, 0));
    assert_quiet(&mut [&mut l, &mut a]);
    assert_eq!(l.paths(), strings(&["WORK/a.md", "Work/NOTES/n.md", "Work/top.md"]));
    assert_eq!(server_spelling(&a, "n.md").as_deref(), Some("Work/NOTES/n.md"));
}

/// Folders two levels deep, spelled differently at both levels (`Work/DRAFTS`
/// on one device, `WORK/Drafts` on the other): each device keeps its own
/// spelling of both, an edit made after that goes up under the server's
/// spelling, and the devices go quiet.
#[test]
fn nested_folders_spelled_differently_at_both_levels_settle() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Work/DRAFTS/a.md", "1\n2\n3\n")]);
    let mut b = ci(&srv, "win-b", &[("WORK/Drafts/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Work/DRAFTS/a.md", "Work/DRAFTS/b.md"]));
    assert_eq!(b.paths(), strings(&["WORK/Drafts/a.md", "WORK/Drafts/b.md"]));
    assert_eq!(server_spelling(&a, "a.md"), None);
    assert_eq!(server_spelling(&a, "b.md").as_deref(), Some("WORK/Drafts/b.md"));
    assert_eq!(server_spelling(&b, "a.md").as_deref(), Some("Work/DRAFTS/a.md"));
    assert_eq!(server_spelling(&b, "b.md"), None);
    // An edit on b of the note a made goes up under a's spelling: a takes it
    // as an edit, not as a move, and the server's spelling stays as it was.
    b.write("WORK/Drafts/a.md", "1\n2 b\n3\n");
    assert_eq!(sync_rounds(&mut b, &mut a, 1), vec![(0, 1, 1, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.read("Work/DRAFTS/a.md").as_deref(), Some("1\n2 b\n3\n"));
    assert_eq!(a.paths(), strings(&["Work/DRAFTS/a.md", "Work/DRAFTS/b.md"]));
    assert_eq!(server_spelling(&a, "a.md"), None);
    assert_eq!(server_spelling(&b, "a.md").as_deref(), Some("Work/DRAFTS/a.md"));
    let want = pairs(&[("a.md", "1\n2 b\n3\n"), ("b.md", "from b\n")]);
    assert_eq!(notes(&a), want);
    assert_eq!(notes(&b), want);
}

/// A note in a folder that is here only in its first level, under another
/// spelling (`WORK/New/Deep/c.md` from a device that spells `Work` as
/// `WORK`): the first level takes this device's spelling, and the folders
/// from the first one that is not here keep the spelling they came with.
#[test]
fn a_new_folder_inside_a_folder_spelled_otherwise_keeps_its_own_spelling() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Work/a.md", "from a\n")]);
    let mut b = ci(&srv, "win-b", &[("WORK/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    b.write("WORK/New/Deep/c.md", "from b, deeper\n");
    assert_eq!(sync_rounds(&mut b, &mut a, 1), vec![(0, 1, 1, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Work/New/Deep/c.md", "Work/a.md", "Work/b.md"]));
    assert_eq!(b.paths(), strings(&["WORK/New/Deep/c.md", "WORK/a.md", "WORK/b.md"]));
    assert_eq!(server_spelling(&a, "c.md").as_deref(), Some("WORK/New/Deep/c.md"));
    assert_eq!(server_spelling(&b, "c.md"), None);
    // Another note in the new folders now finds them through the index.
    b.write("WORK/New/Deep/d.md", "d\n");
    assert_eq!(sync_rounds(&mut b, &mut a, 1), vec![(0, 1, 1, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Work/New/Deep/c.md", "Work/New/Deep/d.md", "Work/a.md", "Work/b.md"]));
    assert_eq!(server_spelling(&a, "d.md").as_deref(), Some("WORK/New/Deep/d.md"));
    let want = pairs(&[("a.md", "from a\n"), ("b.md", "from b\n"), ("c.md", "from b, deeper\n"), ("d.md", "d\n")]);
    assert_eq!(notes(&a), want);
    assert_eq!(notes(&b), want);
}

/// A device whose file system tells case apart (Linux) syncs as before: it
/// keeps a folder for each spelling the server has, each note in the
/// spelling of the device that made it, and gets a case-only rename of the
/// folder made on a case-insensitive device once, as a rename.
#[cfg(target_os = "linux")] // The peer needs a file system that tells case apart.
#[test]
fn a_case_sensitive_peer_keeps_both_spellings_and_gets_a_folder_case_rename_once() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    let mut l = Device::new(&srv, "linux", &[]);
    for _ in 0..2 {
        a.sync();
        b.sync();
        l.sync();
    }
    assert_quiet(&mut [&mut a, &mut b, &mut l]);
    assert_eq!(l.paths(), strings(&["DRAFTS/b.md", "Drafts/a.md"]));
    a.mv("Drafts", "drafts");
    assert_eq!(traffic(&mut a), (0, 2), "the rename goes up once, for both notes");
    assert_eq!(traffic(&mut l), (2, 0));
    assert_eq!(traffic(&mut b), (2, 0));
    assert_quiet(&mut [&mut a, &mut b, &mut l]);
    assert_eq!(l.paths(), strings(&["drafts/a.md", "drafts/b.md"]));
    assert_eq!(a.paths(), strings(&["drafts/a.md", "drafts/b.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/b.md"]));
    let want = pairs(&[("a.md", "from a\n"), ("b.md", "from b\n")]);
    for d in [&a, &b, &l] {
        assert_eq!(notes(d), want, "{}", d.name);
    }
}

/// A case-only rename of a folder on a case-insensitive device is not lost
/// when the pull brings an edit to a note in it from a device that spells
/// the folder otherwise: the rename still goes up for that note, so the
/// case-sensitive peer ends with one folder. (Comparing the edit's path
/// with the folder's spelling here, instead of the server's, would take it
/// for a move made there and drop the rename of that note.)
#[cfg(target_os = "linux")] // The peer needs a file system that tells case apart.
#[test]
fn a_folder_case_rename_is_not_lost_to_an_edit_from_a_device_that_spells_the_folder_otherwise() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    let mut l = Device::new(&srv, "linux", &[]);
    for _ in 0..2 {
        a.sync();
        b.sync();
        l.sync();
    }
    assert_quiet(&mut [&mut a, &mut b, &mut l]);
    b.mv("DRAFTS", "drafts");
    a.write("Drafts/a.md", "from a, edited\n");
    assert_eq!(traffic(&mut a), (0, 1));
    // b writes a's edit into its renamed folder and pushes the rename of both notes.
    assert_eq!(traffic(&mut b), (1, 2));
    assert_eq!(traffic(&mut a), (2, 0));
    assert_eq!(traffic(&mut l), (2, 0));
    assert_quiet(&mut [&mut a, &mut b, &mut l]);
    assert_eq!(l.paths(), strings(&["drafts/a.md", "drafts/b.md"]), "the rename of a.md was dropped");
    assert_eq!(b.paths(), strings(&["drafts/a.md", "drafts/b.md"]));
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/b.md"]));
    let want = pairs(&[("a.md", "from a, edited\n"), ("b.md", "from b\n")]);
    for d in [&a, &b, &l] {
        assert_eq!(notes(d), want, "{}", d.name);
    }
}

/// Notes that a device which tells case apart (Linux) deletes in `Notes`
/// while a case-insensitive PC that spells the folder `notes` edits them:
/// the edits beat the deletes, and the notes come back in `Notes`, under
/// their ids. The PC holds each at the same path as a note of its name that
/// the desktop deleted in its folder `notes` before, so that its sync state
/// has two notes deleted at that path as it uploads the edit: the note
/// deleted last is the one that comes back. (Taking the first of the two
/// by id brought about half of the edits back in `notes`, under the ids of
/// the other notes.)
#[cfg(target_os = "linux")] // The desktop needs a file system that tells case apart.
#[test]
fn edits_that_beat_deletes_come_back_in_the_server_spelling_of_their_folder_where_other_notes_were_deleted_at_their_names() {
    const N: usize = 16;
    let srv = server();
    let old: Vec<(String, String)> = (0..N).map(|i| (format!("notes/n{i}.md"), format!("old {i}\n"))).collect();
    let mut files: Vec<(&str, &str)> = old.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    files.push(("notes/keep.md", "keep\n"));
    let mut l = Device::new(&srv, "linux", &files);
    let mut a = ci(&srv, "win-a", &[]);
    assert_eq!(traffic(&mut l), (0, N + 1));
    assert_eq!(traffic(&mut a), (N + 1, 0));
    // The desktop deletes each note in notes, and makes one of its name in Notes.
    for i in 0..N {
        l.rm(&format!("notes/n{i}.md"));
        l.write(&format!("Notes/n{i}.md"), &format!("new {i}\n"));
    }
    assert_eq!(traffic(&mut l), (0, 2 * N));
    assert_eq!(traffic(&mut a), (2 * N, 0));
    assert_eq!(server_spelling(&a, "n0.md").as_deref(), Some("Notes/n0.md"));
    let id_at = |d: &Device, path: &str| d.engine.state().files.iter().find(|(_, t)| !t.deleted && t.path == path).map(|(f, _)| f.clone());
    let ids: Vec<Option<String>> = (0..N).map(|i| id_at(&l, &format!("Notes/n{i}.md"))).collect();
    // The desktop deletes them, while the PC edits them.
    for i in 0..N {
        l.rm(&format!("Notes/n{i}.md"));
        a.write(&format!("notes/n{i}.md"), &format!("new {i}, edited on the pc\n"));
    }
    assert_eq!(traffic(&mut l), (0, N));
    assert_eq!(traffic(&mut a), (N, N));
    assert_eq!(traffic(&mut l), (N, 0));
    assert_quiet(&mut [&mut l, &mut a]);
    let mut want: Vec<(String, String)> = (0..N).map(|i| (format!("Notes/n{i}.md"), format!("new {i}, edited on the pc\n"))).collect();
    want.push(("notes/keep.md".into(), "keep\n".into()));
    want.sort();
    assert_eq!(l.files(), want);
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(id_at(&l, &format!("Notes/n{i}.md")), *id, "n{i}.md");
    }
    assert_eq!(live_server_files(&srv).len(), N + 1);
}

/// A folder that another program makes on a device whose file system tells
/// case apart (Linux), after the scan and before the pull applies a note in
/// it, is not taken for the indexed folder whose name differs only in case:
/// the note goes into the new folder, as the server spells it, and nothing
/// goes up for it. `same_mtime` gives the new folder the old one's mtime,
/// so that only the change stamp tells the two apart.
#[cfg(target_os = "linux")] // The device needs a file system that tells case apart.
fn a_folder_made_during_the_pull_is_not_taken_for_its_case_twin(same_mtime: bool) {
    let srv = server();
    let mut l = Device::new(&srv, "linux", &[("Drafts/a.md", "from l\n")]);
    let mut m = Device::new(&srv, "linux-2", &[]);
    assert_eq!(traffic(&mut l), (0, 1));
    assert_eq!(traffic(&mut m), (1, 0));
    m.write("DRAFTS/b.md", "from m\n");
    assert_eq!(traffic(&mut m), (0, 1));
    let root = l.root.clone();
    *l.hooks.after_changes.lock() = Some(Box::new(move || {
        fs::create_dir(root.join("DRAFTS")).unwrap();
        if same_mtime {
            let t = fs::metadata(root.join("Drafts")).unwrap().modified().unwrap();
            fs::File::open(root.join("DRAFTS")).unwrap().set_modified(t).unwrap();
        }
    }));
    assert_eq!(traffic(&mut l), (1, 0));
    assert_quiet(&mut [&mut l, &mut m]);
    assert_eq!(l.paths(), strings(&["DRAFTS/b.md", "Drafts/a.md"]));
    assert_eq!(m.paths(), strings(&["DRAFTS/b.md", "Drafts/a.md"]));
    assert_eq!(server_spelling(&l, "b.md"), None);
    let want = pairs(&[("a.md", "from l\n"), ("b.md", "from m\n")]);
    assert_eq!(notes(&l), want);
    assert_eq!(notes(&m), want);
}

#[cfg(target_os = "linux")] // The device needs a file system that tells case apart.
#[test]
fn a_folder_made_during_the_pull_with_its_own_mtime_is_not_taken_for_its_case_twin() {
    a_folder_made_during_the_pull_is_not_taken_for_its_case_twin(false);
}

#[cfg(target_os = "linux")] // The device needs a file system that tells case apart.
#[test]
fn a_folder_made_during_the_pull_with_the_same_mtime_is_not_taken_for_its_case_twin() {
    a_folder_made_during_the_pull_is_not_taken_for_its_case_twin(true);
}

/// Two case-insensitive devices that spell a folder differently, and a phone
/// on Android shared storage. An edit from either device keeps the server's
/// spelling of the folder, so it never changes, and the phone, which renames
/// its folder whenever it does and then uploads every other note in it
/// (FINDING-034), uploads nothing for the edits.
#[test]
fn edits_from_two_spellings_of_a_folder_cost_a_phone_on_shared_storage_no_uploads() {
    let srv = server();
    let files: Vec<(String, String)> = (0..4).map(|i| (format!("Drafts/n{i}.md"), format!("note {i}\n"))).collect();
    let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let mut a = ci(&srv, "win-a", &files);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = Device::with_fs(&srv, "phone", &[], SharedStorageFs::new);
    assert_eq!(traffic(&mut a), (0, 4));
    assert_eq!(traffic(&mut b), (4, 0));
    assert_eq!(traffic(&mut phone), (4, 0));
    // b's user renames the folder in File Explorer, only in case. The
    // rename goes up once; a keeps its spelling, and the phone takes b's.
    b.mv("Drafts", "DRAFTS");
    assert_eq!(traffic(&mut b), (0, 4));
    assert_eq!(traffic(&mut a), (4, 0));
    assert_eq!(traffic(&mut phone), (4, 0));
    let mut phone_pushed = 0;
    for round in 0..3 {
        // a edits a note, and the phone gets the edit; then b edits that
        // note and another one, and the phone gets those.
        assert_eq!(traffic(&mut a), (if round == 0 { 0 } else { 2 }, 0), "round {round}");
        a.write("Drafts/n0.md", &format!("n0, round {round}, a\n"));
        assert_eq!(traffic(&mut a), (0, 1), "round {round}");
        let (pulled, pushed) = traffic(&mut phone);
        assert_eq!(pulled, 1, "round {round}");
        phone_pushed += pushed;
        assert_eq!(traffic(&mut b), (1, 0), "round {round}");
        b.write("DRAFTS/n0.md", &format!("n0, round {round}, b\n"));
        b.write(&format!("DRAFTS/n{}.md", round + 1), &format!("round {round}, b\n"));
        assert_eq!(traffic(&mut b), (0, 2), "round {round}");
        let (pulled, pushed) = traffic(&mut phone);
        assert_eq!(pulled, 2, "round {round}");
        phone_pushed += pushed;
    }
    assert_eq!(phone_pushed, 0, "the phone uploaded notes for edits made elsewhere");
    assert_eq!(traffic(&mut a), (2, 0));
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    let want = pairs(&[("n0.md", "n0, round 2, b\n"), ("n1.md", "round 0, b\n"), ("n2.md", "round 1, b\n"), ("n3.md", "round 2, b\n")]);
    for d in [&a, &b, &phone] {
        assert_eq!(notes(d), want, "{}", d.name);
    }
    assert_eq!(top_folders(&a), strings(&["Drafts"]));
    assert_eq!(top_folders(&b), strings(&["DRAFTS"]));
    assert_eq!(top_folders(&phone), strings(&["DRAFTS"]));
}

/// A phone on Android shared storage (SafPlugin.kt, as `SharedStorageFs`).
fn shared_storage_phone(srv: &Server, files: &[(&str, &str)]) -> Device {
    Device::with_fs(srv, "phone", files, SharedStorageFs::new)
}

/// A new note goes up in the spelling of the device that made it. A phone
/// on Android shared storage, which holds one spelling of a folder and whose
/// folder has the other one, renames its folder to the note's spelling
/// first, on the phone only, and stores the note: its other note in the
/// folder keeps its spelling on the server, and nothing goes up. Then all
/// go quiet, and each PC keeps its own spelling.
#[test]
fn a_new_note_in_a_folder_that_a_phone_on_shared_storage_spells_otherwise_renames_the_folder_there() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/n.md", "n\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    for d in [&mut a, &mut b, &mut phone] {
        traffic(d);
    }
    b.mv("Drafts", "DRAFTS");
    for d in [&mut b, &mut a, &mut phone] {
        traffic(d);
    }
    assert_eq!(phone.paths(), strings(&["DRAFTS/n.md"]));
    a.write("Drafts/new.md", "new on a\n");
    assert_eq!(traffic(&mut a), (0, 1));
    assert_eq!(traffic(&mut b), (1, 0));
    assert_eq!(b.read("DRAFTS/new.md").as_deref(), Some("new on a\n"));
    assert_eq!(traffic(&mut phone), (1, 0), "the phone stores the note and uploads nothing for its folder");
    assert_eq!(phone.paths(), strings(&["Drafts/n.md", "Drafts/new.md"]));
    assert_eq!(server_spelling(&phone, "n.md").as_deref(), Some("DRAFTS/n.md"));
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    assert_eq!(a.paths(), strings(&["Drafts/n.md", "Drafts/new.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/n.md", "DRAFTS/new.md"]));
    let want = pairs(&[("n.md", "n\n"), ("new.md", "new on a\n")]);
    for d in [&a, &b, &phone] {
        assert_eq!(notes(d), want, "{}", d.name);
    }
}

/// A Windows PC and a phone on Android shared storage that each made a
/// folder before their first sync, spelled differently (`Drafts` on the PC,
/// `DRAFTS` on the phone): in either order of their first syncs, the phone
/// takes the PC's spelling, both have every note, and the two go quiet. The
/// server keeps each note in the spelling it first went up in: the phone's
/// note goes up in the PC's spelling when the phone syncs second, and keeps
/// its own when the phone syncs first, as the phone's rename of its folder
/// stays on the phone.
#[test]
fn a_pc_and_a_phone_on_shared_storage_that_spell_a_folder_differently_settle_on_the_pc_spelling() {
    for pc_first in [true, false] {
        let srv = server();
        let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n")]);
        let mut phone = shared_storage_phone(&srv, &[("DRAFTS/p.md", "from the phone\n")]);
        if pc_first {
            assert_eq!(traffic(&mut a), (0, 1));
            // The phone renames its folder, stores a.md and uploads p.md, new, in the PC's spelling.
            assert_eq!(traffic(&mut phone), (1, 1));
            assert_eq!(traffic(&mut a), (1, 0));
        } else {
            assert_eq!(traffic(&mut phone), (0, 1));
            assert_eq!(traffic(&mut a), (1, 1));
            // The phone renames its folder and stores a.md, and uploads nothing.
            assert_eq!(traffic(&mut phone), (1, 0));
        }
        assert_quiet(&mut [&mut a, &mut phone]);
        for d in [&a, &phone] {
            assert_eq!(d.paths(), strings(&["Drafts/a.md", "Drafts/p.md"]), "pc first {pc_first}: {}", d.name);
            assert_eq!(notes(d), pairs(&[("a.md", "from a\n"), ("p.md", "from the phone\n")]), "pc first {pc_first}: {}", d.name);
        }
        let p_on_server = (!pc_first).then(|| "DRAFTS/p.md".to_string());
        assert_eq!(server_spelling(&a, "p.md"), p_on_server, "pc first {pc_first}");
        assert_eq!(server_spelling(&phone, "p.md"), p_on_server, "pc first {pc_first}");
    }
}

/// Two Windows PCs that spell a folder differently, and a phone on Android
/// shared storage, which can hold only one of the spellings: a new note from
/// the PC whose spelling the phone does not have makes the phone rename its
/// folder, on the phone only. It uploads nothing for that, and the PCs get
/// nothing, so all go quiet after each new note, and nothing is lost. Edits
/// keep the server's spelling and cost the phone no upload either.
#[test]
fn a_phone_on_shared_storage_follows_new_notes_from_two_pcs_that_spell_a_folder_differently() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/n0.md", "n0\n"), ("Drafts/n1.md", "n1\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    assert_eq!(traffic(&mut a), (0, 2));
    assert_eq!(traffic(&mut b), (2, 0));
    assert_eq!(traffic(&mut phone), (2, 0));
    b.mv("Drafts", "DRAFTS");
    assert_eq!(traffic(&mut b), (0, 2));
    assert_eq!(traffic(&mut a), (2, 0));
    assert_eq!(traffic(&mut phone), (2, 0));
    assert_eq!(top_folders(&phone), strings(&["DRAFTS"]));
    let mut want = pairs(&[("n0.md", "n0\n"), ("n1.md", "n1\n")]);
    for (round, (pc, folder)) in [("a", "Drafts"), ("b", "DRAFTS"), ("a", "Drafts"), ("b", "DRAFTS")].into_iter().enumerate() {
        let (name, text) = (format!("new{round}.md"), format!("new {round}, made on {pc}\n"));
        let (maker, other) = if pc == "a" { (&mut a, &mut b) } else { (&mut b, &mut a) };
        maker.write(&format!("{folder}/{name}"), &text);
        assert_eq!(traffic(maker), (0, 1), "round {round}");
        assert_eq!(traffic(other), (1, 0), "round {round}");
        assert_eq!(traffic(&mut phone), (1, 0), "round {round}: the phone uploads nothing for its folder");
        assert_eq!(top_folders(&phone), strings(&[folder]), "round {round}: the phone takes the spelling of the new note");
        assert_quiet(&mut [&mut a, &mut b, &mut phone]);
        want.push((name, text));
        want.sort();
        for d in [&a, &b, &phone] {
            assert_eq!(notes(d), want, "round {round}: {}", d.name);
        }
        assert_eq!(top_folders(&a), strings(&["Drafts"]), "round {round}");
        assert_eq!(top_folders(&b), strings(&["DRAFTS"]), "round {round}");
    }
    // Every note keeps the spelling it went up in.
    for (name, on_server) in [("n0.md", "DRAFTS/n0.md"), ("new0.md", "Drafts/new0.md"), ("new1.md", "DRAFTS/new1.md"), ("new2.md", "Drafts/new2.md")] {
        let here = format!("DRAFTS/{name}");
        assert_eq!(server_spelling(&phone, name), (here != on_server).then(|| on_server.to_string()), "{name}");
    }
    a.write("Drafts/n0.md", "n0, edited on a\n");
    b.write("DRAFTS/n1.md", "n1, edited on b\n");
    assert_eq!(traffic(&mut a), (0, 1));
    assert_eq!(traffic(&mut b), (1, 1));
    assert_eq!(traffic(&mut phone), (2, 0), "edits cost the phone no upload");
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    assert_eq!(phone.read("DRAFTS/n0.md").as_deref(), Some("n0, edited on a\n"));
    assert_eq!(phone.read("DRAFTS/n1.md").as_deref(), Some("n1, edited on b\n"));
}

/// A note moved on a PC into a folder that a phone on Android shared storage
/// spells otherwise: the phone renames its folder to the PC's spelling, on
/// the phone only, and moves the note there, as it does for a new note.
#[test]
fn a_note_moved_into_a_folder_that_a_phone_on_shared_storage_spells_otherwise_renames_the_folder_there() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/n.md", "n\n"), ("Inbox/x.md", "x\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    for d in [&mut a, &mut b, &mut phone] {
        traffic(d);
    }
    b.mv("Drafts", "DRAFTS");
    for d in [&mut b, &mut a, &mut phone] {
        traffic(d);
    }
    assert_eq!(phone.paths(), strings(&["DRAFTS/n.md", "Inbox/x.md"]));
    a.mv("Inbox/x.md", "Drafts/x.md");
    assert_eq!(traffic(&mut a), (0, 1));
    assert_eq!(traffic(&mut b), (1, 0));
    assert_eq!(traffic(&mut phone), (1, 0), "the phone moves x.md and uploads nothing for its folder");
    assert_eq!(phone.paths(), strings(&["Drafts/n.md", "Drafts/x.md"]));
    assert_eq!(top_folders(&phone), strings(&["Drafts"]));
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    assert_eq!(b.paths(), strings(&["DRAFTS/n.md", "DRAFTS/x.md"]));
    let want = pairs(&[("n.md", "n\n"), ("x.md", "x\n")]);
    for d in [&a, &b, &phone] {
        assert_eq!(notes(d), want, "{}", d.name);
    }
}

/// A note deleted on a PC while a phone on Android shared storage renames
/// its folder to another spelling in its pull: nothing goes up for the
/// rename, so the delete applies on the phone in its next sync, and the
/// note does not come back.
#[test]
fn a_delete_made_while_a_phone_on_shared_storage_renames_its_folder_still_applies() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/n0.md", "n0\n"), ("Drafts/n1.md", "n1\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    for d in [&mut a, &mut b, &mut phone] {
        traffic(d);
    }
    b.mv("Drafts", "DRAFTS");
    for d in [&mut b, &mut a, &mut phone] {
        traffic(d);
    }
    a.write("Drafts/new.md", "new on a\n");
    assert_eq!(traffic(&mut a), (0, 1));
    b.rm("DRAFTS/n1.md");
    let b = Arc::new(Mutex::new(b));
    let other = b.clone();
    // b deletes n1.md once the phone has read the changes to pull.
    *phone.hooks.after_changes.lock() = Some(Box::new(move || assert_eq!(traffic(&mut other.lock()), (1, 1))));
    assert_eq!(traffic(&mut phone), (1, 0));
    let mut b = Arc::try_unwrap(b).ok().expect("the hook ran").into_inner();
    assert_eq!(phone.paths(), strings(&["Drafts/n0.md", "Drafts/n1.md", "Drafts/new.md"]));
    assert_eq!(traffic(&mut phone), (1, 0));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    assert_eq!(phone.paths(), strings(&["Drafts/n0.md", "Drafts/new.md"]));
    assert_eq!(a.paths(), strings(&["Drafts/n0.md", "Drafts/new.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/n0.md", "DRAFTS/new.md"]));
}

/// A note edited on a phone on Android shared storage and deleted on a PC,
/// in the pull that has the phone's folder take another spelling: the edit
/// beats the delete, and the note comes back on every device, with its id,
/// so its history goes on, and in the server's spelling of its folder.
#[test]
fn an_edit_on_a_phone_on_shared_storage_beats_a_delete_and_keeps_its_id_when_the_folder_takes_another_spelling() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/m.md", "m\n"), ("Drafts/n.md", "n\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    for d in [&mut a, &mut b, &mut phone] {
        traffic(d);
    }
    b.mv("Drafts", "DRAFTS");
    for d in [&mut b, &mut a, &mut phone] {
        traffic(d);
    }
    let id_of = |d: &Device, name: &str| -> Vec<String> {
        d.engine.state().files.iter().filter(|(_, t)| !t.deleted && t.path.ends_with(name)).map(|(f, _)| f.clone()).collect()
    };
    let id = id_of(&phone, "/n.md");
    phone.write("DRAFTS/n.md", "n, edited on the phone\n");
    b.rm("DRAFTS/n.md");
    assert_eq!(traffic(&mut b), (0, 1));
    a.write("Drafts/new.md", "new on a\n");
    assert_eq!(traffic(&mut a), (1, 1));
    assert_eq!(traffic(&mut phone), (2, 1), "the phone uploads n.md again");
    assert_eq!(traffic(&mut a), (1, 0));
    assert_eq!(traffic(&mut b), (2, 0));
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    assert_eq!(phone.paths(), strings(&["Drafts/m.md", "Drafts/n.md", "Drafts/new.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/m.md", "DRAFTS/n.md", "DRAFTS/new.md"]));
    assert_eq!(server_spelling(&a, "n.md").as_deref(), Some("DRAFTS/n.md"));
    for d in [&a, &b, &phone] {
        assert_eq!(id_of(d, "/n.md"), id, "{}", d.name);
        assert_eq!(d.files().into_iter().find(|(p, _)| p.ends_with("/n.md")).map(|f| f.1).as_deref(), Some("n, edited on the phone\n"), "{}", d.name);
    }
}

/// A phone on Android shared storage whose user renamed a folder by case,
/// outside the app or in it, while a PC deleted a note in it and made
/// another one there: the new note does not have the folder take the PC's
/// spelling again, which would undo the user's rename. It goes into the
/// folder as the phone spells it, and its rename to that goes up with the
/// user's. The rename wins over the delete, as any rename does, so the PC
/// gets the note back, under the phone's spelling on the server.
#[test]
fn a_case_only_folder_rename_on_a_phone_on_shared_storage_is_kept_when_a_new_note_comes_in_the_old_spelling() {
    for in_app in [false, true] {
        let srv = server();
        let mut a = ci(&srv, "win-a", &[("Drafts/m.md", "m\n"), ("Drafts/n.md", "n\n")]);
        let mut phone = shared_storage_phone(&srv, &[]);
        assert_eq!(traffic(&mut a), (0, 2));
        assert_eq!(traffic(&mut phone), (2, 0));
        if in_app {
            phone.app_mv("Drafts", "DRAFTS");
        } else {
            phone.mv("Drafts", "DRAFTS");
        }
        a.rm("Drafts/n.md");
        a.write("Drafts/new.md", "new on a\n");
        assert_eq!(traffic(&mut a), (0, 2));
        assert_eq!(traffic(&mut phone), (2, 3), "in app {in_app}: the phone uploads three renames, n.md's on top of its delete");
        assert_eq!(phone.paths(), strings(&["DRAFTS/m.md", "DRAFTS/n.md", "DRAFTS/new.md"]), "in app {in_app}");
        assert_eq!(traffic(&mut a), (3, 0), "in app {in_app}");
        assert_quiet(&mut [&mut a, &mut phone]);
        assert_eq!(a.paths(), strings(&["Drafts/m.md", "Drafts/n.md", "Drafts/new.md"]), "in app {in_app}");
        let want = pairs(&[("m.md", "m\n"), ("n.md", "n\n"), ("new.md", "new on a\n")]);
        for d in [&a, &phone] {
            assert_eq!(notes(d), want, "in app {in_app}: {}", d.name);
        }
        for name in ["m.md", "n.md", "new.md"] {
            assert_eq!(server_spelling(&a, name), Some(format!("DRAFTS/{name}")), "in app {in_app}");
        }
    }
}

/// As above, a level deeper, where the phone spells the folder inside the
/// one the user renamed otherwise than the server does only because it took
/// another device's spelling, on the phone only: the new note goes up with
/// the user's spelling of the folder they renamed and the server's spelling
/// of the folder inside it. Going up with the phone's spelling of both, it
/// moved the note into a folder of its own on a device that tells case
/// apart.
#[test]
fn a_case_only_folder_rename_on_a_phone_on_shared_storage_goes_up_for_that_folder_only_when_a_new_note_comes_in() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Work/NOTES/x.md", "x\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    assert_eq!(traffic(&mut a), (0, 1));
    // b makes the folder inside in another spelling before it has x.md.
    b.write("Work/notes/y.md", "y\n");
    assert_eq!(traffic(&mut b), (1, 1));
    // The phone's folder inside takes b's spelling, on the phone only.
    assert_eq!(traffic(&mut phone), (2, 0));
    assert_eq!(phone.paths(), strings(&["Work/notes/x.md", "Work/notes/y.md"]));
    phone.app_mv("Work", "work");
    a.write("Work/NOTES/z.md", "z\n");
    assert_eq!(traffic(&mut a), (1, 1));
    traffic(&mut phone);
    assert_eq!(phone.paths(), strings(&["work/notes/x.md", "work/notes/y.md", "work/notes/z.md"]));
    traffic(&mut a);
    traffic(&mut b);
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    let mut linux = Device::new(&srv, "linux", &[]);
    traffic(&mut linux);
    assert_eq!(linux.paths(), strings(&["work/NOTES/x.md", "work/NOTES/z.md", "work/notes/y.md"]));
}

/// The same when, in that pull, moves from a PC that renamed the folder by
/// case rename the phone's folder first (FINDING-034), and a new note from
/// another PC comes in a third spelling: every spelling of the folder is the
/// one folder on the phone. It keeps the spelling that the moves gave it,
/// as the user's rename of the note kept against the delete has not gone up
/// yet: that note goes up in it, and so does the new note's rename. Both
/// ways round, as the phone looks for the folder by its spelling on disk.
#[test]
fn a_note_kept_against_a_delete_goes_up_when_moves_and_a_new_note_come_into_its_folder_on_a_phone_on_shared_storage() {
    for (mine, theirs) in [("drafts", "DRAFTS"), ("DRAFTS", "drafts")] {
        let srv = server();
        let mut a = ci(&srv, "win-a", &[("Drafts/n.md", "n\n"), ("Drafts/w.md", "w\n"), ("Drafts/x.md", "x\n")]);
        let mut b = ci(&srv, "win-b", &[]);
        let mut phone = shared_storage_phone(&srv, &[]);
        for d in [&mut a, &mut b, &mut phone] {
            traffic(d);
        }
        phone.mv("Drafts", mine);
        a.rm("Drafts/n.md");
        assert_eq!(traffic(&mut a), (0, 1));
        assert_eq!(traffic(&mut b), (1, 0));
        b.mv("Drafts", theirs);
        assert_eq!(traffic(&mut b), (0, 2));
        a.write("Drafts/new.md", "new on a\n");
        assert_eq!(traffic(&mut a), (2, 1));
        assert_eq!(traffic(&mut phone), (4, 2), "{mine}: the phone uploads n.md again and the rename of new.md");
        assert_eq!(top_folders(&phone), strings(&[theirs]), "{mine}");
        assert_eq!(traffic(&mut a), (2, 0), "{mine}");
        assert_eq!(traffic(&mut b), (2, 0), "{mine}");
        assert_quiet(&mut [&mut a, &mut b, &mut phone]);
        // b moved its copy of n.md to the trash for the delete; it has it back.
        let want = pairs(&[("n.md", "n\n"), ("new.md", "new on a\n"), ("w.md", "w\n"), ("x.md", "x\n")]);
        for d in [&a, &b, &phone] {
            let mut live: Vec<(String, String)> = d.files().into_iter().map(|(p, t)| (p.rsplit('/').next().unwrap().to_string(), t)).collect();
            live.sort();
            assert_eq!(live, want, "{mine}: {}", d.name);
        }
        assert_eq!(a.paths(), strings(&["Drafts/n.md", "Drafts/new.md", "Drafts/w.md", "Drafts/x.md"]), "{mine}");
        let there: Vec<String> = ["n.md", "new.md", "w.md", "x.md"].iter().map(|n| format!("{theirs}/{n}")).collect();
        assert_eq!(b.paths(), there, "{mine}");
        assert_eq!(phone.paths(), there, "{mine}");
    }
}

/// Edits made on a phone on Android shared storage before its folder takes
/// another spelling go up with the renames: one to a note that a PC edited
/// too, merged in the same pull, and one to a note that only the phone
/// edited. The note that brings the other spelling comes first on the
/// server: the phone takes it after the rest of the pull, also when it
/// pulls one record per batch, where it waits for the last one.
#[test]
fn edits_made_on_a_phone_on_shared_storage_before_its_folder_takes_another_spelling_go_up() {
    for one_by_one in [false, true] {
        let srv = server();
        let mut a = ci(&srv, "win-a", &[("Drafts/n0.md", "1\n2\n3\n"), ("Drafts/n1.md", "n1\n")]);
        let mut b = ci(&srv, "win-b", &[]);
        let mut phone = shared_storage_phone(&srv, &[]);
        if one_by_one {
            phone.pull_one_by_one();
        }
        for d in [&mut a, &mut b, &mut phone] {
            traffic(d);
        }
        b.mv("Drafts", "DRAFTS");
        for d in [&mut b, &mut a, &mut phone] {
            traffic(d);
        }
        phone.write("DRAFTS/n0.md", "1\n2\n3 phone\n");
        phone.write("DRAFTS/n1.md", "n1, edited on the phone\n");
        a.write("Drafts/n0.md", "1 a\n2\n3\n");
        a.write("Drafts/a.md", "new on a\n");
        assert_eq!(traffic(&mut a), (0, 2));
        assert_eq!(traffic(&mut phone), (2, 2), "one by one {one_by_one}");
        assert_eq!(phone.paths(), strings(&["Drafts/a.md", "Drafts/n0.md", "Drafts/n1.md"]), "one by one {one_by_one}");
        assert_eq!(traffic(&mut a), (2, 0), "one by one {one_by_one}");
        assert_eq!(traffic(&mut b), (3, 0), "one by one {one_by_one}");
        assert_quiet(&mut [&mut a, &mut b, &mut phone]);
        let want = pairs(&[("a.md", "new on a\n"), ("n0.md", "1 a\n2\n3 phone\n"), ("n1.md", "n1, edited on the phone\n")]);
        for d in [&a, &b, &phone] {
            assert_eq!(notes(d), want, "one by one {one_by_one}: {}", d.name);
        }
    }
}

/// Folders two levels deep that a phone on Android shared storage spells
/// otherwise at both levels (`WORK/DRAFTS` against a PC's `Work/Drafts`):
/// in either order of the first syncs, the phone renames both, top level
/// first, on the phone only, and its note in the top folder goes along.
#[test]
fn nested_folders_that_a_phone_on_shared_storage_spells_otherwise_take_the_pc_spelling() {
    for pc_first in [true, false] {
        let srv = server();
        let mut a = ci(&srv, "win-a", &[("Work/Drafts/a.md", "from a\n")]);
        let mut phone = shared_storage_phone(&srv, &[("WORK/DRAFTS/p.md", "p\n"), ("WORK/top.md", "top\n")]);
        if pc_first {
            assert_eq!(traffic(&mut a), (0, 1));
            assert_eq!(traffic(&mut phone), (1, 2));
            assert_eq!(traffic(&mut a), (2, 0));
        } else {
            assert_eq!(traffic(&mut phone), (0, 2));
            assert_eq!(traffic(&mut a), (2, 1));
            assert_eq!(traffic(&mut phone), (1, 0));
        }
        assert_quiet(&mut [&mut a, &mut phone]);
        for d in [&a, &phone] {
            assert_eq!(d.paths(), strings(&["Work/Drafts/a.md", "Work/Drafts/p.md", "Work/top.md"]), "pc first {pc_first}: {}", d.name);
            assert_eq!(notes(d), pairs(&[("a.md", "from a\n"), ("p.md", "p\n"), ("top.md", "top\n")]), "pc first {pc_first}: {}", d.name);
        }
        let (p, top) = if pc_first { (None, None) } else { (Some("WORK/DRAFTS/p.md".to_string()), Some("WORK/top.md".to_string())) };
        assert_eq!((server_spelling(&a, "p.md"), server_spelling(&a, "top.md")), (p.clone(), top.clone()), "pc first {pc_first}");
        assert_eq!((server_spelling(&phone, "p.md"), server_spelling(&phone, "top.md")), (p, top), "pc first {pc_first}");
    }
}

/// A note that a phone on Android shared storage cannot hold next to one of
/// its own, as their names differ only in case, keeps FINDING-172's
/// behaviour when the phone spells their folder otherwise too: the phone
/// does not rename the folder for it, nor for another new note that the
/// server has in that spelling, lists both and uploads nothing for them.
/// Once the user renames the phone's note, the folder takes the other
/// spelling and both notes come in.
#[test]
fn a_case_twin_keeps_the_name_of_a_folder_that_a_phone_on_shared_storage_spells_otherwise() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "a\n"), ("Drafts/note.md", "from a\n")]);
    let mut phone = shared_storage_phone(&srv, &[("DRAFTS/Note.md", "from the phone\n")]);
    assert_eq!(traffic(&mut a), (0, 2));
    for round in 0..2 {
        let r = phone.sync();
        // In the first round the phone uploads its own note.
        assert_eq!((r.pulled, r.pushed), (0, 1 - round), "round {round}");
        assert!(r.conflicts.is_empty(), "round {round}: {:?}", r.conflicts);
        let mut listed: Vec<&str> = r.skipped.iter().map(|s| s.path.as_str()).collect();
        listed.sort();
        assert_eq!(listed, ["Drafts/a.md", "Drafts/note.md"], "round {round}: {:?}", r.skipped);
        assert!(r.skipped.iter().any(|s| s.path == "Drafts/note.md" && s.reason.contains("differ only in case")), "round {round}: {:?}", r.skipped);
        assert!(r.skipped.iter().any(|s| s.path == "Drafts/a.md" && s.reason.contains("Drafts/note.md")), "round {round}: {:?}", r.skipped);
        assert_eq!(phone.paths(), strings(&["DRAFTS/Note.md"]), "round {round}");
    }
    phone.mv("DRAFTS/Note.md", "DRAFTS/Phone note.md");
    assert_eq!(traffic(&mut phone), (2, 1));
    assert_eq!(phone.paths(), strings(&["Drafts/Phone note.md", "Drafts/a.md", "Drafts/note.md"]));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut phone]);
    let want = pairs(&[("Phone note.md", "from the phone\n"), ("a.md", "a\n"), ("note.md", "from a\n")]);
    assert_eq!(notes(&a), want);
    assert_eq!(notes(&phone), want);
}

/// A device whose file system tells case apart can have two folders whose
/// names differ only in case, here each with a note `same.md`. A phone on
/// Android shared storage holds one of the folders. The other one holds a
/// case twin of a note in it, so the phone does not take that folder for
/// its own and rename it, which would merge the two on the desktop: it lists
/// the notes of the other folder, and the desktop keeps both folders as they
/// are (as e2e/android/adv_saf_paths.test.mjs checks on a phone). So also
/// when the phone pulls one record per batch, as a large notebook comes in
/// batches: the twin comes after another note of its folder.
#[cfg(target_os = "linux")] // The desktop needs a file system that tells case apart.
#[test]
fn folders_that_a_case_sensitive_device_keeps_apart_are_not_merged_by_a_phone_on_shared_storage() {
    for one_by_one in [false, true] {
        let srv = server();
        let files = [("Seed.md", "seed\n"), ("Notes/a.md", "A\n"), ("Notes/same.md", "upper\n"), ("notes/b.md", "B\n"), ("notes/same.md", "lower\n")];
        let mut l = Device::new(&srv, "linux", &files);
        let mut phone = shared_storage_phone(&srv, &[]);
        if one_by_one {
            phone.pull_one_by_one();
        }
        assert_eq!(traffic(&mut l), (0, 5));
        for round in 0..2 {
            let at = format!("one by one {one_by_one}, round {round}");
            let r = phone.sync();
            assert_eq!((r.pulled, r.pushed), (if round == 0 { 3 } else { 0 }, 0), "{at}");
            assert!(r.conflicts.is_empty(), "{at}: {:?}", r.conflicts);
            let mut listed: Vec<&str> = r.skipped.iter().map(|s| s.path.as_str()).collect();
            listed.sort();
            assert_eq!(listed, ["notes/b.md", "notes/same.md"], "{at}: {:?}", r.skipped);
            assert!(r.skipped.iter().any(|s| s.path == "notes/b.md" && s.reason.contains("notes/same.md")), "{at}: {:?}", r.skipped);
            assert_eq!(phone.paths(), strings(&["Notes/a.md", "Notes/same.md", "Seed.md"]), "{at}");
            assert_eq!(phone.read("Notes/same.md").as_deref(), Some("upper\n"), "{at}");
            assert_eq!(traffic(&mut l), (0, 0), "{at}");
        }
        let mut want: Vec<(String, String)> = files.iter().map(|(p, c)| (p.to_string(), c.to_string())).collect();
        want.sort();
        assert_eq!(l.files(), want, "one by one {one_by_one}");
    }
}

/// The same when the desktop made the notes of the other folder in one sync
/// and edited the note whose name the twin has in a later one, before the
/// phone's next sync: in the phone's pull the twin waits for that edit, as
/// it wants the edited note's name, and still keeps the phone from merging
/// the folders.
#[cfg(target_os = "linux")] // The desktop needs a file system that tells case apart.
#[test]
fn a_case_twin_that_waits_in_the_pull_still_keeps_folders_apart_on_a_phone_on_shared_storage() {
    let srv = server();
    let mut l = Device::new(&srv, "linux", &[("Seed.md", "seed\n"), ("Notes/a.md", "A\n"), ("Notes/same.md", "upper\n")]);
    let mut phone = shared_storage_phone(&srv, &[]);
    assert_eq!(traffic(&mut l), (0, 3));
    assert_eq!(traffic(&mut phone), (3, 0));
    l.write("notes/b.md", "B\n");
    l.write("notes/same.md", "lower\n");
    assert_eq!(traffic(&mut l), (0, 2));
    l.write("Notes/same.md", "upper, edited\n");
    assert_eq!(traffic(&mut l), (0, 1));
    for round in 0..2 {
        let r = phone.sync();
        assert_eq!((r.pulled, r.pushed), (if round == 0 { 1 } else { 0 }, 0), "round {round}");
        assert!(r.conflicts.is_empty(), "round {round}: {:?}", r.conflicts);
        let mut listed: Vec<&str> = r.skipped.iter().map(|s| s.path.as_str()).collect();
        listed.sort();
        assert_eq!(listed, ["notes/b.md", "notes/same.md"], "round {round}: {:?}", r.skipped);
        assert!(r.skipped.iter().any(|s| s.path == "notes/b.md" && s.reason.contains("notes/same.md")), "round {round}: {:?}", r.skipped);
        assert_eq!(phone.paths(), strings(&["Notes/a.md", "Notes/same.md", "Seed.md"]), "round {round}");
        assert_eq!(phone.read("Notes/same.md").as_deref(), Some("upper, edited\n"), "round {round}");
        assert_eq!(traffic(&mut l), (0, 0), "round {round}");
    }
    let want = [("Notes/a.md", "A\n"), ("Notes/same.md", "upper, edited\n"), ("Seed.md", "seed\n"), ("notes/b.md", "B\n"), ("notes/same.md", "lower\n")];
    assert_eq!(l.files(), want.map(|(p, c)| (p.to_string(), c.to_string())));
}

/// A device whose file system tells case apart (Linux) can have two
/// folders whose names differ only in case and no note name in common,
/// here `Notes/a.md` and `notes/b.md`. A phone on Android shared storage
/// holds them as one folder, which takes the spelling of the note it stores
/// last, on the phone only: nothing goes up for it, so the desktop keeps its
/// folders as they are, also when it makes another note in the first one
/// and the phone's folder takes that spelling again. An edit on the phone
/// goes up where the desktop has the note.
#[cfg(target_os = "linux")] // The desktop needs a file system that tells case apart.
#[test]
fn folders_that_a_case_sensitive_device_keeps_apart_without_a_case_twin_stay_apart_with_a_phone_on_shared_storage() {
    let srv = server();
    let files = [("Notes/a.md", "A\n"), ("notes/b.md", "B\n")];
    let mut l = Device::new(&srv, "linux", &files);
    let mut phone = shared_storage_phone(&srv, &[]);
    assert_eq!(traffic(&mut l), (0, 2));
    assert_eq!(traffic(&mut phone), (2, 0));
    assert_eq!(top_folders(&phone), strings(&["notes"]));
    assert_eq!(traffic(&mut l), (0, 0));
    assert_eq!(l.files(), pairs(&files));
    l.write("Notes/c.md", "C\n");
    assert_eq!(traffic(&mut l), (0, 1));
    assert_eq!(traffic(&mut phone), (1, 0));
    assert_eq!(top_folders(&phone), strings(&["Notes"]));
    assert_quiet(&mut [&mut l, &mut phone]);
    assert_eq!(l.files(), pairs(&[("Notes/a.md", "A\n"), ("Notes/c.md", "C\n"), ("notes/b.md", "B\n")]));
    assert_eq!(phone.paths(), strings(&["Notes/a.md", "Notes/b.md", "Notes/c.md"]));
    phone.write("Notes/b.md", "B, edited on the phone\n");
    assert_eq!(traffic(&mut phone), (0, 1));
    assert_eq!(traffic(&mut l), (1, 0));
    assert_quiet(&mut [&mut l, &mut phone]);
    assert_eq!(l.files(), pairs(&[("Notes/a.md", "A\n"), ("Notes/c.md", "C\n"), ("notes/b.md", "B, edited on the phone\n")]));
}

/// Notes that wait on a phone on Android shared storage, listed, for a
/// case twin in their folder (a device that tells case apart has two
/// folders whose names differ only in case, with a note of the same name in
/// each) have their records read and decrypted once in each sync, as they
/// are tried again: why they wait on is known from their paths.
#[cfg(target_os = "linux")] // The desktop needs a file system that tells case apart.
#[test]
fn notes_that_wait_for_a_case_twin_on_a_phone_on_shared_storage_are_read_once_per_sync() {
    let srv = server();
    let mut files: Vec<(String, String)> = vec![("Notes/a.md".into(), "A\n".into()), ("Notes/same.md".into(), "upper\n".into()), ("notes/same.md".into(), "lower\n".into())];
    files.extend((0..6).map(|i| (format!("notes/b{i}.md"), format!("b{i}\n"))));
    let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let mut l = Device::new(&srv, "linux", &files);
    let mut phone = shared_storage_phone(&srv, &[]);
    assert_eq!(traffic(&mut l), (0, 9));
    for round in 0..3 {
        let before = phone.engine.records_opened();
        let r = phone.sync();
        let opened = phone.engine.records_opened() - before;
        assert_eq!((r.pulled, r.pushed, r.skipped.len()), (if round == 0 { 2 } else { 0 }, 0, 7), "round {round}: {:?}", r.skipped);
        // All nine come in once in the first sync, then the seven that wait.
        assert_eq!(opened, if round == 0 { 9 } else { 7 }, "round {round}");
        assert_eq!(phone.paths(), strings(&["Notes/a.md", "Notes/same.md"]), "round {round}");
    }
    assert_eq!(traffic(&mut l), (0, 0));
}

/// A case-only rename of a folder on a phone on Android shared storage, in
/// the app or outside it, while a PC renames a note in that folder and
/// deletes another one. The PC's rename keeps the server's spelling of the
/// folder, so the phone moves the note within the folder as the user spells
/// it, rather than have the move rename the folder back (FINDING-034), and
/// the note's rename to that spelling goes up with the user's. The user's
/// rename of the deleted note wins over the delete, as any rename does, so
/// the PC gets it back.
#[test]
fn a_case_only_folder_rename_on_a_phone_on_shared_storage_is_kept_when_a_pc_renames_a_note_in_it() {
    for in_app in [false, true] {
        let srv = server();
        let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "a\n"), ("Drafts/c.md", "c\n"), ("Drafts/d.md", "d\n")]);
        let mut phone = shared_storage_phone(&srv, &[]);
        assert_eq!(traffic(&mut a), (0, 3));
        assert_eq!(traffic(&mut phone), (3, 0));
        if in_app {
            phone.app_mv("Drafts", "DRAFTS");
        } else {
            phone.mv("Drafts", "DRAFTS");
        }
        a.mv("Drafts/a.md", "Drafts/b.md");
        a.rm("Drafts/c.md");
        assert_eq!(traffic(&mut a), (0, 2));
        assert_eq!(traffic(&mut phone), (2, 3), "in app {in_app}: the phone uploads its renames of b.md, c.md and d.md");
        assert_eq!(top_folders(&phone), strings(&["DRAFTS"]), "in app {in_app}");
        assert_eq!(phone.paths(), strings(&["DRAFTS/b.md", "DRAFTS/c.md", "DRAFTS/d.md"]), "in app {in_app}");
        assert_eq!(traffic(&mut a), (3, 0), "in app {in_app}");
        assert_quiet(&mut [&mut a, &mut phone]);
        assert_eq!(a.paths(), strings(&["Drafts/b.md", "Drafts/c.md", "Drafts/d.md"]), "in app {in_app}");
        let want = pairs(&[("b.md", "a\n"), ("c.md", "c\n"), ("d.md", "d\n")]);
        for d in [&a, &phone] {
            assert_eq!(notes(d), want, "in app {in_app}: {}", d.name);
        }
        for name in ["b.md", "c.md", "d.md"] {
            assert_eq!(server_spelling(&a, name), Some(format!("DRAFTS/{name}")), "in app {in_app}");
        }
    }
}

/// A case-only rename of a folder on a phone on Android shared storage,
/// not uploaded yet, while the pull has a folder inside it take another
/// spelling, on the phone only, to follow a note from another device: the
/// user's rename still goes up, for that folder only.
#[test]
fn a_case_only_folder_rename_on_a_phone_on_shared_storage_goes_up_when_a_folder_in_it_takes_another_spelling() {
    for in_app in [false, true] {
        let srv = server();
        let mut a = ci(&srv, "win-a", &[("work/Notes/n.md", "n\n")]);
        let mut phone = shared_storage_phone(&srv, &[]);
        assert_eq!(traffic(&mut a), (0, 1));
        assert_eq!(traffic(&mut phone), (1, 0));
        let mut b = ci(&srv, "win-b", &[("WORK/NOTES/b.md", "b\n")]);
        assert_eq!(traffic(&mut b), (1, 1));
        if in_app {
            phone.app_mv("work", "WORK");
        } else {
            phone.mv("work", "WORK");
        }
        assert_eq!(traffic(&mut phone), (1, 1), "in app {in_app}: the phone uploads its rename of n.md");
        assert_eq!(phone.paths(), strings(&["WORK/NOTES/b.md", "WORK/NOTES/n.md"]), "in app {in_app}");
        assert_eq!(traffic(&mut a), (2, 0), "in app {in_app}");
        assert_eq!(traffic(&mut b), (1, 0), "in app {in_app}");
        assert_quiet(&mut [&mut a, &mut b, &mut phone]);
        assert_eq!(a.paths(), strings(&["work/Notes/b.md", "work/Notes/n.md"]), "in app {in_app}");
        assert_eq!(server_spelling(&a, "n.md").as_deref(), Some("WORK/Notes/n.md"), "in app {in_app}");
    }
}

/// A note that a device that tells case apart moves into a third spelling
/// of a folder (`Notes/a.md` to `NOTES/a.md`, a folder of its own there):
/// a phone on Android shared storage, which holds all spellings as one
/// folder, renames that folder to the new spelling on the phone only, as
/// the move would (FINDING-034). The other notes in it keep their paths on
/// the server, so the desktop keeps them where they are.
#[cfg(target_os = "linux")] // The desktop needs a file system that tells case apart.
#[test]
fn a_note_moved_into_a_third_spelling_of_a_folder_renames_it_on_a_phone_on_shared_storage_only() {
    let srv = server();
    let mut l = Device::new(&srv, "linux", &[("Notes/a.md", "A\n"), ("Notes/c.md", "C\n")]);
    let mut phone = shared_storage_phone(&srv, &[]);
    assert_eq!(traffic(&mut l), (0, 2));
    assert_eq!(traffic(&mut phone), (2, 0));
    l.mv("Notes/a.md", "NOTES/a.md");
    assert_eq!(traffic(&mut l), (0, 1));
    assert_eq!(traffic(&mut phone), (1, 0), "the phone uploads nothing for its folder");
    assert_eq!(top_folders(&phone), strings(&["NOTES"]));
    assert_eq!(phone.paths(), strings(&["NOTES/a.md", "NOTES/c.md"]));
    assert_quiet(&mut [&mut l, &mut phone]);
    assert_eq!(l.files(), pairs(&[("NOTES/a.md", "A\n"), ("Notes/c.md", "C\n")]));
}

/// A folder that a phone on Android shared storage cannot rename (here one
/// in a folder it may not write in) leaves the phone as it was: the folder
/// renamed before it gets its name back, the new note waits, listed, and
/// nothing goes up. Once the folder can be renamed, the note comes in.
#[cfg(unix)] // Made read-only with Unix mode bits.
#[test]
fn a_folder_that_a_phone_on_shared_storage_cannot_rename_keeps_its_name_and_the_note_waits() {
    use std::os::unix::fs::PermissionsExt;
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Work/Drafts/a.md", "from a\n")]);
    let mut phone = shared_storage_phone(&srv, &[("WORK/DRAFTS/p.md", "p\n")]);
    assert_eq!(traffic(&mut phone), (0, 1));
    assert_eq!(traffic(&mut a), (1, 1));
    // WORK itself can be renamed, but not DRAFTS in it.
    let locked = phone.root.join("WORK");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::write(locked.join("probe"), "").is_ok() {
        fs::remove_file(locked.join("probe")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("running as root; permission test not meaningful");
        return;
    }
    // The folder is made writable again before any assertion can stop the
    // test, so that its temporary folder goes away.
    let rounds = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (0..2)
            .map(|_| {
                let r = phone.sync();
                let listed: Vec<String> = r.skipped.iter().map(|s| s.path.clone()).collect();
                (r.pulled, r.pushed, listed, phone.paths(), top_folders(&phone))
            })
            .collect::<Vec<_>>()
    }));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let rounds = rounds.unwrap_or_else(|e| std::panic::resume_unwind(e));
    for (round, r) in rounds.into_iter().enumerate() {
        assert_eq!(r, (0, 0, strings(&["Work/Drafts/a.md"]), strings(&["WORK/DRAFTS/p.md"]), strings(&["WORK"])), "round {round}");
    }
    assert_eq!(traffic(&mut phone), (1, 0));
    assert_quiet(&mut [&mut a, &mut phone]);
    assert_eq!(phone.paths(), strings(&["Work/Drafts/a.md", "Work/Drafts/p.md"]));
    assert_eq!(notes(&phone), notes(&a));
}

/// A phone on Android shared storage whose renames fail as `probe` says,
/// and which counts its listings there.
fn probed_phone(srv: &Server, files: &[(&str, &str)], probe: &Arc<StorageProbe>) -> Device {
    let probe = probe.clone();
    Device::with_fs(srv, "phone", files, move |root| SharedStorageFs::probed(root, probe))
}

/// The path, server path and `renamed` of the live note named `name` in the
/// sync state saved in `state_dir`.
fn saved(state_dir: &std::path::Path, name: &str) -> (String, Option<String>, bool) {
    let state: serde_json::Value = serde_json::from_slice(&fs::read(state_dir.join("state.json")).unwrap()).unwrap();
    let found: Vec<&serde_json::Value> = state["files"]
        .as_object()
        .unwrap()
        .values()
        .filter(|t| t["deleted"] == false && t["path"].as_str().unwrap().rsplit('/').next() == Some(name))
        .collect();
    assert_eq!(found.len(), 1, "{name} in {state}");
    let t = found[0];
    (t["path"].as_str().unwrap().to_string(), t["server_path"].as_str().map(str::to_string), t["renamed"].as_bool().unwrap_or(false))
}

/// A folder that a phone on Android shared storage fails to rename is not
/// tried again in that sync: the other notes that would have it take the
/// spelling wait too, listed with the reason, rather than each trying the
/// rename again. The next sync tries once more.
#[test]
fn a_folder_that_a_phone_on_shared_storage_fails_to_rename_is_tried_once_per_sync() {
    let srv = server();
    let faults = Arc::new(StorageProbe::default());
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "a\n"), ("Drafts/b.md", "b\n")]);
    let mut phone = probed_phone(&srv, &[("DRAFTS/p.md", "p\n")], &faults);
    assert_eq!(traffic(&mut phone), (0, 1));
    assert_eq!(traffic(&mut a), (1, 2));
    // The first try fails; a second one would not.
    faults.add("Drafts", RenameFault::Fail);
    let r = phone.sync();
    assert_eq!((r.pulled, r.pushed), (0, 0));
    let mut listed: Vec<&str> = r.skipped.iter().map(|s| s.path.as_str()).collect();
    listed.sort();
    assert_eq!(listed, ["Drafts/a.md", "Drafts/b.md"], "{:?}", r.skipped);
    assert!(r.skipped.iter().all(|s| s.reason.contains("Renaming DRAFTS to Drafts failed")), "{:?}", r.skipped);
    assert_eq!(faults.left(), 0);
    assert_eq!(phone.paths(), strings(&["DRAFTS/p.md"]));
    assert_eq!(traffic(&mut phone), (2, 0));
    assert_eq!(phone.paths(), strings(&["Drafts/a.md", "Drafts/b.md", "Drafts/p.md"]));
    assert_quiet(&mut [&mut a, &mut phone]);
}

/// Android shared storage renames a folder by case through a free name
/// ("Drafts (1)", see SafPlugin.kt). A rename that stops there, which the
/// storage fails to undo, is undone by the sync: the folder gets its name
/// back, nothing goes up, and the note waits, listed, until the next sync.
#[test]
fn a_folder_rename_that_stops_halfway_on_a_phone_on_shared_storage_is_undone() {
    let srv = server();
    let faults = Arc::new(StorageProbe::default());
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "a\n")]);
    let mut phone = probed_phone(&srv, &[("DRAFTS/p.md", "p\n")], &faults);
    assert_eq!(traffic(&mut phone), (0, 1));
    assert_eq!(traffic(&mut a), (1, 1));
    faults.add("Drafts", RenameFault::StopHalfway);
    let r = phone.sync();
    assert_eq!((r.pulled, r.pushed), (0, 0));
    assert_eq!(r.skipped.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["Drafts/a.md"], "{:?}", r.skipped);
    assert_eq!(faults.left(), 0);
    assert_eq!(top_folders(&phone), strings(&["DRAFTS"]));
    assert_eq!(phone.paths(), strings(&["DRAFTS/p.md"]));
    assert_eq!(saved(&phone.state_dir, "p.md"), ("DRAFTS/p.md".into(), None, false));
    assert_eq!(traffic(&mut phone), (1, 0));
    assert_quiet(&mut [&mut a, &mut phone]);
    assert_eq!(phone.paths(), strings(&["Drafts/a.md", "Drafts/p.md"]));
}

/// When the folder cannot get its name back either, its notes are tracked
/// where they are, under the free name, but nothing goes up for it: the PC
/// keeps its paths, and an edit made meanwhile goes up in the server's
/// spelling. The next sync gives the folder its name on the server back
/// before anything else, then has it take the PC's spelling as the note
/// that waited comes in, all on the phone only.
#[test]
fn a_folder_left_halfway_renamed_on_a_phone_on_shared_storage_never_goes_up_and_gets_its_name_back() {
    let srv = server();
    let faults = Arc::new(StorageProbe::default());
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "a\n")]);
    let mut phone = probed_phone(&srv, &[("DRAFTS/p.md", "p\n"), ("DRAFTS/q.md", "q\n")], &faults);
    assert_eq!(traffic(&mut phone), (0, 2));
    assert_eq!(traffic(&mut a), (2, 1));
    faults.add("Drafts", RenameFault::StopHalfway);
    faults.add("DRAFTS", RenameFault::Fail);
    let r = phone.sync();
    assert_eq!((r.pulled, r.pushed), (0, 0));
    assert_eq!(r.skipped.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["Drafts/a.md"], "{:?}", r.skipped);
    assert_eq!(faults.left(), 0);
    assert_eq!(phone.paths(), strings(&["Drafts (1)/p.md", "Drafts (1)/q.md"]));
    assert_eq!(saved(&phone.state_dir, "p.md"), ("Drafts (1)/p.md".into(), None, false));
    assert_eq!(server_spelling(&phone, "p.md").as_deref(), Some("DRAFTS/p.md"));
    assert_eq!(traffic(&mut a), (0, 0));
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/p.md", "Drafts/q.md"]));
    // An edit made under the free name goes up under the server's name.
    phone.write("Drafts (1)/q.md", "q, edited on the phone\n");
    let r = phone.sync();
    assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "{r:?}");
    assert_eq!((r.pulled, r.pushed), (1, 1));
    assert_eq!(phone.paths(), strings(&["Drafts/a.md", "Drafts/p.md", "Drafts/q.md"]));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut phone]);
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/p.md", "Drafts/q.md"]));
    assert_eq!(a.read("Drafts/q.md").as_deref(), Some("q, edited on the phone\n"));
    for name in ["p.md", "q.md"] {
        assert_eq!(server_spelling(&a, name), Some(format!("DRAFTS/{name}")), "{name}");
    }
    assert!(phone.trash_text().is_empty() && a.trash_text().is_empty());
}

/// The same when a folder further down cannot be renamed, and the folder
/// above it, renamed already, cannot get its name back: the notes in it
/// are tracked where they are, nothing goes up for them, a delete from
/// another device still applies to them, and the next sync renames the
/// folder further down once it can.
#[test]
fn a_folder_that_cannot_get_its_name_back_on_a_phone_on_shared_storage_is_tracked_where_it_is() {
    let srv = server();
    let faults = Arc::new(StorageProbe::default());
    let mut a = ci(&srv, "win-a", &[("Work/Drafts/a.md", "a\n")]);
    let mut phone = probed_phone(&srv, &[("WORK/DRAFTS/p.md", "p\n"), ("WORK/top.md", "top\n")], &faults);
    assert_eq!(traffic(&mut phone), (0, 2));
    assert_eq!(traffic(&mut a), (2, 1));
    // WORK takes the spelling Work, Work/DRAFTS cannot, and Work cannot get
    // its name back.
    faults.add("Work/Drafts", RenameFault::Fail);
    faults.add("WORK", RenameFault::Fail);
    faults.add("WORK", RenameFault::Fail);
    let r = phone.sync();
    assert_eq!((r.pulled, r.pushed), (0, 0));
    assert_eq!(r.skipped.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["Work/Drafts/a.md"], "{:?}", r.skipped);
    assert_eq!(faults.left(), 0);
    assert_eq!(phone.paths(), strings(&["Work/DRAFTS/p.md", "Work/top.md"]));
    assert_eq!(saved(&phone.state_dir, "top.md"), ("Work/top.md".into(), None, false));
    assert_eq!(saved(&phone.state_dir, "p.md"), ("Work/DRAFTS/p.md".into(), None, false));
    assert_eq!(server_spelling(&phone, "top.md").as_deref(), Some("WORK/top.md"));
    assert_eq!(traffic(&mut a), (0, 0));
    a.rm("Work/top.md");
    assert_eq!(traffic(&mut a), (0, 1));
    assert_eq!(traffic(&mut phone), (2, 0));
    assert_eq!(phone.paths(), strings(&["Work/Drafts/a.md", "Work/Drafts/p.md"]));
    assert!(phone.trash_text().contains("top\n"), "{:?}", phone.trash_text());
    assert_quiet(&mut [&mut a, &mut phone]);
    assert_eq!(a.paths(), strings(&["Work/Drafts/a.md", "Work/Drafts/p.md"]));
}

/// New notes from two PCs that spell a folder differently, in one pull of a
/// phone on Android shared storage that spells it in a third way: the
/// folder takes a spelling once, that of the first of them on the server,
/// on the phone only, and the note in the other spelling goes into it as
/// the phone spells it then. Nothing goes up, and all go quiet.
#[test]
fn notes_in_two_spellings_of_a_folder_in_one_pull_rename_it_once_on_a_phone_on_shared_storage() {
    let srv = server();
    let mut phone = shared_storage_phone(&srv, &[("drafts/p.md", "p\n")]);
    let mut a = ci(&srv, "win-a", &[("Drafts/a1.md", "a1\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b1.md", "b1\n")]);
    assert_eq!(traffic(&mut phone), (0, 1));
    assert_eq!(traffic(&mut a), (1, 1));
    assert_eq!(traffic(&mut b), (2, 1));
    let r = phone.sync();
    assert!(r.conflicts.is_empty() && r.skipped.is_empty(), "{r:?}");
    assert_eq!((r.pulled, r.pushed), (2, 0), "the phone uploads nothing for its folder");
    let renamed: Vec<&Change> = r.changes.iter().filter(|c| matches!(c, Change::Renamed { entry, .. } if entry.kind == EntryKind::Dir)).collect();
    assert_eq!(renamed.len(), 1, "{renamed:?}");
    assert_eq!(phone.paths(), strings(&["Drafts/a1.md", "Drafts/b1.md", "Drafts/p.md"]));
    assert_eq!(server_spelling(&phone, "b1.md").as_deref(), Some("DRAFTS/b1.md"));
    assert_eq!(server_spelling(&phone, "p.md").as_deref(), Some("drafts/p.md"));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    assert_eq!(a.paths(), strings(&["Drafts/a1.md", "Drafts/b1.md", "Drafts/p.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a1.md", "DRAFTS/b1.md", "DRAFTS/p.md"]));
}

/// Notes that a phone on Android shared storage edited, or renamed, in a
/// folder that takes another spelling in a pull, which the same pull moves
/// into another folder that it renames too: the phone finds them under the
/// folder's new spelling, moves them, keeps its edit merged with the other
/// device's, and gives way to the other device's name.
#[test]
fn notes_changed_on_a_phone_on_shared_storage_move_on_after_their_folder_takes_another_spelling() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/x.md", "1\n2\n3\n"), ("Drafts/y.md", "y1\ny2\ny3\n"), ("Inbox/i.md", "i\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    for d in [&mut a, &mut b, &mut phone] {
        traffic(d);
    }
    b.mv("Drafts", "DRAFTS");
    b.mv("Inbox", "INBOX");
    for d in [&mut b, &mut a, &mut phone] {
        traffic(d);
    }
    assert_eq!(phone.paths(), strings(&["DRAFTS/x.md", "DRAFTS/y.md", "INBOX/i.md"]));
    phone.write("DRAFTS/x.md", "1\n2\n3 phone\n");
    phone.mv("DRAFTS/y.md", "DRAFTS/z.md");
    a.write("Drafts/new.md", "new on a\n");
    a.app_mv("Drafts/x.md", "Inbox/x.md");
    a.write("Inbox/x.md", "1 a\n2\n3\n");
    a.app_mv("Drafts/y.md", "Inbox/y.md");
    a.write("Inbox/y.md", "y1 a\ny2\ny3\n");
    assert_eq!(traffic(&mut a), (0, 3));
    assert_eq!(traffic(&mut phone), (3, 1), "the phone uploads the merged x.md");
    assert_eq!(phone.paths(), strings(&["Drafts/new.md", "Inbox/i.md", "Inbox/x.md", "Inbox/y.md"]));
    assert_eq!(phone.read("Inbox/x.md").as_deref(), Some("1 a\n2\n3 phone\n"));
    assert_eq!(phone.read("Inbox/y.md").as_deref(), Some("y1 a\ny2\ny3\n"));
    assert_eq!(traffic(&mut a), (1, 0));
    traffic(&mut b);
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
    let want = pairs(&[("i.md", "i\n"), ("new.md", "new on a\n"), ("x.md", "1 a\n2\n3 phone\n"), ("y.md", "y1 a\ny2\ny3\n")]);
    for d in [&a, &b, &phone] {
        assert_eq!(notes(d), want, "{}", d.name);
    }
}

/// A phone on Android shared storage whose folders take the spellings that
/// another device gives them lists the notebook for the scans of its sync
/// only, as any sync does: the files in those folders are tracked under
/// their new names as they are renamed, not found by listing it again. So
/// also when it pulls one record per batch, where the new notes that have
/// the folders take those spellings wait for every other change of the
/// pull: nothing was changed for them before.
#[test]
fn folders_taking_other_spellings_on_a_phone_on_shared_storage_do_not_list_the_notebook_again() {
    for one_by_one in [false, true] {
        let srv = server();
        let probe = Arc::new(StorageProbe::default());
        let mut phone = probed_phone(&srv, &[("DRAFTS/p.md", "p\n"), ("INBOX/q.md", "q\n")], &probe);
        let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "a\n"), ("Inbox/i.md", "i\n")]);
        assert_eq!(traffic(&mut phone), (0, 2));
        assert_eq!(traffic(&mut a), (2, 2));
        if one_by_one {
            phone.pull_one_by_one();
        }
        probe.listings.store(0, Ordering::SeqCst);
        assert_eq!(traffic(&mut phone), (2, 0), "one by one {one_by_one}");
        assert_eq!(phone.paths(), strings(&["Drafts/a.md", "Drafts/p.md", "Inbox/i.md", "Inbox/q.md"]), "one by one {one_by_one}");
        assert_eq!(probe.listings.load(Ordering::SeqCst), 2, "one by one {one_by_one}: one listing for the pull and one for the push");
        assert_quiet(&mut [&mut a, &mut phone]);
    }
}

/// A note moved into a folder that a phone on Android shared storage spells
/// otherwise, and a new note at the name it had, which the phone pulls one
/// record per batch: the moved note waits for every other change of the
/// pull, as it has the folder take another spelling, and the new note,
/// which wants its name, waits for it too, rather than take a conflict copy
/// name.
#[test]
fn a_note_that_wants_the_name_of_one_moving_into_a_folder_spelled_otherwise_waits_for_it_on_a_phone_on_shared_storage() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/n.md", "n\n"), ("Inbox/x.md", "x\n"), ("zz.md", "z\n")]);
    let mut b = ci(&srv, "win-b", &[]);
    let mut phone = shared_storage_phone(&srv, &[]);
    for d in [&mut a, &mut b, &mut phone] {
        traffic(d);
    }
    b.mv("Drafts", "DRAFTS");
    for d in [&mut b, &mut a, &mut phone] {
        traffic(d);
    }
    assert_eq!(phone.paths(), strings(&["DRAFTS/n.md", "Inbox/x.md", "zz.md"]));
    phone.pull_one_by_one();
    a.app_mv("Inbox/x.md", "Drafts/x.md");
    a.write("Inbox/x.md", "a new x\n");
    a.write("zz.md", "z, edited\n");
    assert_eq!(traffic(&mut a), (0, 3));
    assert_eq!(traffic(&mut phone), (3, 0), "the phone uploads nothing for its folder");
    assert_eq!(phone.paths(), strings(&["Drafts/n.md", "Drafts/x.md", "Inbox/x.md", "zz.md"]));
    assert_eq!(phone.read("Inbox/x.md").as_deref(), Some("a new x\n"));
    assert_eq!(phone.read("Drafts/x.md").as_deref(), Some("x\n"));
    traffic(&mut a);
    traffic(&mut b);
    assert_quiet(&mut [&mut a, &mut b, &mut phone]);
}

/// A sync that fails after a phone on Android shared storage renamed its
/// folder (here as it uploads a new note) still hands the folder's rename
/// to the app (`SyncEngine::take_changes`), so that notes open in that
/// folder follow it, as after a sync that did not fail.
#[test]
fn a_failed_sync_hands_the_folder_rename_of_a_phone_on_shared_storage_to_the_app() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "a\n")]);
    let mut phone = shared_storage_phone(&srv, &[("DRAFTS/p.md", "p\n")]);
    assert_eq!(traffic(&mut phone), (0, 1));
    assert_eq!(traffic(&mut a), (1, 1));
    phone.write("DRAFTS/q.md", "q\n");
    phone.hooks.fail_puts.store(1, Ordering::SeqCst);
    assert!(phone.try_sync().is_err());
    let changes = phone.engine.take_changes();
    assert!(changes.iter().any(|c| matches!(c, Change::Renamed { from, entry } if from == "DRAFTS" && entry.path == "Drafts")), "{changes:?}");
    assert!(phone.engine.take_changes().is_empty());
    assert_eq!(traffic(&mut phone), (0, 1));
    assert!(phone.engine.take_changes().is_empty());
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut phone]);
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/p.md", "Drafts/q.md"]));
}

/// Put `d`'s sync state back as an older version left it: a file whose
/// folder the server spells otherwise is tracked under the server's path,
/// which the disk here does not have (that version recorded the path it
/// pulled, and a sync that stopped before its push left it so).
fn as_an_older_version_left_it(d: &Device) {
    let path = d.state_dir.join("state.json");
    let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for t in state["files"].as_object_mut().unwrap().values_mut() {
        let t = t.as_object_mut().unwrap();
        if let Some(spelled) = t.remove("server_spelling") {
            t.insert("path".into(), spelled);
            t.remove("seen");
        }
    }
    fs::write(&path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
}

/// Older versions moved such a folder's notes back and forth on every
/// sync, and one that stopped between its pull and its push left the other
/// device's spelling in the sync state. Once both devices have this
/// version, they settle in two exchanges: each pushes its own spelling of
/// such a note once more, and the other takes it for the server's.
#[test]
fn a_sync_state_with_the_other_spelling_from_an_older_version_settles() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n"), ("Drafts/c.md", "c\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    as_an_older_version_left_it(&a);
    as_an_older_version_left_it(&b);
    assert_eq!(sync_rounds(&mut a, &mut b, 2), vec![(0, 1, 1, 2), (2, 0, 0, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    let want = pairs(&[("a.md", "from a\n"), ("b.md", "from b\n"), ("c.md", "c\n")]);
    assert_eq!(notes(&a), want);
    assert_eq!(notes(&b), want);
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/b.md", "Drafts/c.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/b.md", "DRAFTS/c.md"]));
}

/// The server's spelling is kept in the sync state on disk: an engine
/// loaded from it again still uploads an edit in that spelling, so the
/// note does not move on the server.
#[test]
fn the_server_spelling_of_a_folder_is_kept_in_the_sync_state() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    let saved = fs::read_to_string(b.state_dir.join("state.json")).unwrap();
    assert!(saved.contains(r#""server_spelling": "Drafts/a.md""#), "{saved}");
    b.engine = SyncEngine::load(b.vault.clone(), &b.state_dir).unwrap().unwrap();
    b.write("DRAFTS/a.md", "from a, edited on b\n");
    assert_eq!(traffic(&mut b), (0, 1));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.read("Drafts/a.md").as_deref(), Some("from a, edited on b\n"));
    // a's spelling went up, which a has, so a keeps no other.
    assert_eq!(server_spelling(&a, "a.md"), None);
    assert_eq!(server_spelling(&b, "a.md").as_deref(), Some("Drafts/a.md"));
}

/// A device that spells the folder otherwise and loses its sync state takes
/// its notes for the server's again, with no rename, and a note it renamed
/// meanwhile goes up as that rename, in the server's spelling of the folder.
#[test]
fn a_lost_sync_state_is_rebuilt_without_renaming_a_folder_spelled_otherwise() {
    let srv = server();
    let mut a = ci(&srv, "win-a", &[("Drafts/a.md", "from a\n"), ("Drafts/c.md", "c\n")]);
    let mut b = ci(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    sync_rounds(&mut a, &mut b, 2);
    assert_quiet(&mut [&mut a, &mut b]);
    b.mv("DRAFTS/c.md", "DRAFTS/c2.md");
    fs::remove_file(b.state_dir.join("state.json")).unwrap();
    assert_eq!(traffic(&mut b), (2, 1));
    assert_eq!(traffic(&mut a), (1, 0));
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/b.md", "Drafts/c2.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/b.md", "DRAFTS/c2.md"]));
    assert_eq!(server_spelling(&a, "c2.md"), None);
    assert_eq!(server_spelling(&b, "c2.md").as_deref(), Some("Drafts/c2.md"));
}

/// The same on a real file system that ignores case, with `StdFs`, which on
/// NTFS keeps a folder's spelling when a note is moved into it under
/// another spelling, as `CaseInsensitiveFs` does.
#[cfg_attr(not(windows), ignore = "needs a file system that ignores case (NTFS); CaseInsensitiveFs covers it elsewhere")]
#[test]
fn folder_spelled_differently_on_two_real_case_insensitive_file_systems_settles() {
    let srv = server();
    let mut a = Device::new(&srv, "win-a", &[("Drafts/a.md", "from a\n")]);
    let mut b = Device::new(&srv, "win-b", &[("DRAFTS/b.md", "from b\n")]);
    assert_eq!(sync_rounds(&mut a, &mut b, 2), vec![(0, 1, 1, 1), (1, 0, 0, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    let want = pairs(&[("a.md", "from a\n"), ("b.md", "from b\n")]);
    assert_eq!(notes(&a), want);
    assert_eq!(notes(&b), want);
    assert_eq!(a.paths(), strings(&["Drafts/a.md", "Drafts/b.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/b.md"]));
    // A case-only rename of the folder goes up once, and b keeps its
    // spelling. It goes through another name: a Linux folder with case
    // folding keeps the old spelling on a case-only rename.
    a.mv("Drafts", "renamed");
    a.mv("renamed", "drafts");
    assert_eq!(sync_rounds(&mut a, &mut b, 1), vec![(0, 2, 2, 0)]);
    assert_quiet(&mut [&mut a, &mut b]);
    assert_eq!(a.paths(), strings(&["drafts/a.md", "drafts/b.md"]));
    assert_eq!(b.paths(), strings(&["DRAFTS/a.md", "DRAFTS/b.md"]));
}

/// `Vault::rename` refuses a case-only rename when the target is a
/// different file (Linux): `Note.md` -> `note.md` must not replace the
/// existing `note.md`, which would leave no trace (no trash).
#[cfg(target_os = "linux")] // Case twins need a case-sensitive file system.
#[test]
fn case_only_rename_onto_another_existing_file_is_refused() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Note.md"), "upper\n").unwrap();
    fs::write(d.path().join("note.md"), "lower\n").unwrap();
    let vault = cairn_core::Vault::open(std_fs(&d.path().canonicalize().unwrap())).unwrap();
    let r = vault.rename("Note.md", "note.md");
    let lower = fs::read_to_string(d.path().join("note.md")).unwrap_or_default();
    assert!(r.is_err(), "rename succeeded; note.md now contains {lower:?}");
    assert_eq!(lower, "lower\n");
}

// ===================================================================== unicode names

/// A file whose name is stored in NFD on disk (as macOS HFS+ writes them, and
/// as rsync/unzip copy them to Linux) is listed under its NFC name, which
/// does not exist on disk; reading it must still find the file, or sync
/// would silently skip it.
#[test]
fn nfd_file_name_is_synced() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[]);
    let nfd = "Cafe\u{301} notes.md";
    a.write(nfd, "bonjour\n");
    let r = a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    assert_eq!(r.pushed, 1, "the laptop pushed nothing for {nfd:?}; vault lists {:?}", a.vault.entries().iter().map(|e| e.path.clone()).collect::<Vec<_>>());
    assert_eq!(b.files().len(), 1);
}

/// Same silent skip for a Linux file name containing a backslash (legal on
/// Linux and Android; `normalize` turns it into a folder separator).
#[test]
#[ignore = "FINDING-011 won't fix (by design): a note whose name has a backslash is skipped and listed in the files not synced"]
fn backslash_file_name_is_synced() {
    let srv = server();
    let mut a = Device::new(&srv, "linux", &[]);
    a.write("draft\\v2.md", "backslash\n");
    let r = a.sync();
    assert_eq!(r.pushed, 1, "pushed nothing; vault lists {:?}", a.vault.entries().iter().map(|e| e.path.clone()).collect::<Vec<_>>());
}

/// PLAN section 3: a blob is `{path, content, mtime}`. The engine uploads
/// the mtime but never applies it, so every downloaded or merged file gets
/// the time of the sync. On a newly synced device the quick switcher's
/// "recent first" order (sorted by mtime) is just the download order.
#[test]
#[ignore = "FINDING-150: modification times are uploaded but never applied; quick switcher recency is wrong on other devices"]
fn modification_times_are_preserved_across_devices() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("old.md", "written last year\n"), ("new.md", "written today\n")]);
    let last_year = SystemTime::now() - Duration::from_secs(365 * 86_400);
    set_mtime(&a.abs("old.md"), last_year);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    let got = mtime(&b.abs("old.md"));
    let diff = got.duration_since(last_year).unwrap_or_default();
    assert!(diff < Duration::from_secs(2), "old.md on the phone has mtime {:?} days after the laptop's", diff.as_secs() / 86_400);
}

#[test]
fn device_name_in_nfd_does_not_cause_churn() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "base\n")]);
    a.sync();
    let mut b = Device::new(&srv, "Zoe\u{308}'s phone", &[]);
    b.sync();
    a.write("n.md", "laptop\n");
    b.write("n.md", "phone\n");
    a.sync();
    let r = b.sync();
    assert_eq!(r.conflicts.len(), 1);
    converge(&mut [&mut a, &mut b]);
    let r2 = b.sync();
    assert_eq!((r2.pushed, r2.pulled), (0, 0));
}

// ===================================================================== conflict copies

#[test]
fn conflict_copy_name_never_overwrites_an_existing_file() {
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[("n.md", "base\n")]);
    // Pre-create files with the exact conflict names for this minute and
    // the next one, on the receiving device.
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let mut squatters = Vec::new();
    for t in [now, now + 60] {
        let name = format!("n (conflict {} phone).md", stamp(t));
        b.write(&name, &format!("squatter {t}\n"));
        squatters.push(name);
    }
    b.sync();
    a.sync();
    a.write("n.md", "laptop\n");
    b.write("n.md", "phone\n");
    a.sync();
    let r = b.sync();
    assert_eq!(r.conflicts.len(), 1);
    for (i, s) in squatters.iter().enumerate() {
        assert!(b.read(s).unwrap().starts_with("squatter"), "squatter {i} overwritten");
    }
    let all: String = b.files().into_iter().map(|f| f.1).collect();
    assert!(all.contains("laptop") && all.contains("phone"));
}

#[test]
fn conflict_on_a_conflict_copy() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("n.md", "base\n")]);
    a.write("n.md", "laptop\n");
    b.write("n.md", "phone\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let copy = a.conflict_copies().pop().expect("a conflict copy");
    a.write(&copy, "copy edited on laptop\n");
    c.write(&copy, "copy edited on tablet\n");
    a.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let all: String = a.files().into_iter().map(|f| f.1).collect();
    for w in ["copy edited on laptop", "copy edited on tablet", "phone", "laptop"] {
        assert!(all.contains(w), "{w} lost: {:?}", a.files());
    }
}

/// Two devices that sync at the same time and both upload a new note at the
/// same path (both pulls happen before either push, so neither sees the
/// other's file) must still end with a file at that path, not each rename
/// the other's file to a conflict name and then receive the other's rename.
/// PLAN: "the file uploaded first is renamed to a conflict copy and the
/// second keeps the name"; identical contents merge into one file.
#[test]
fn concurrent_sync_creating_same_path_keeps_the_name() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop\n"), ("Same.md", "identical\n")]);
    let b = Arc::new(Mutex::new(Device::new(&srv, "phone", &[("Today.md", "phone\n"), ("Same.md", "identical\n")])));
    // The phone's whole sync happens between the laptop's pull and push.
    let b2 = b.clone();
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        b2.lock().sync();
    }));
    a.sync();
    let mut b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    converge(&mut [&mut a, &mut b]);
    let paths = a.paths();
    eprintln!("{:?}", a.files());
    assert!(paths.contains(&"Today.md".to_string()), "nobody kept Today.md: {paths:?}");
    assert_eq!(a.files().iter().filter(|f| f.1 == "identical\n").count(), 1, "identical files not merged: {paths:?}");
    assert!(paths.contains(&"Same.md".to_string()), "nobody kept Same.md: {paths:?}");
}

/// The race above with different contents only: the phone uploads its
/// Today.md first, and the laptop its own right after, without having seen
/// the phone's.
fn race_to_create_today(srv: &Server) -> (Device, Device) {
    let mut a = Device::new(srv, "laptop", &[("Today.md", "laptop\n")]);
    let b = Arc::new(Mutex::new(Device::new(srv, "phone", &[("Today.md", "phone\n")])));
    let b2 = b.clone();
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        b2.lock().sync();
    }));
    a.sync();
    (a, Arc::try_unwrap(b).ok().unwrap().into_inner())
}

/// The phone's file, uploaded first, is the conflict copy everywhere.
fn assert_laptop_keeps_today(devs: &[&Device], when: &str) {
    for d in devs {
        assert_eq!(d.read("Today.md").as_deref(), Some("laptop\n"), "{when}, on {}: {:?}", d.name, d.files());
        let copies = d.conflict_copies();
        assert_eq!(copies.len(), 1, "{when}, on {}: {:?}", d.name, d.files());
        assert_eq!(d.read(&copies[0]).as_deref(), Some("phone\n"), "{when}, on {}", d.name);
        assert_eq!(d.files().len(), 2, "{when}, on {}: {:?}", d.name, d.files());
    }
}

/// FINDING-063 with different contents. PLAN: "the file uploaded first is
/// renamed to a conflict copy and the second keeps the name". Every device
/// picks the same one, whichever syncs first, including a third device that
/// pulls both files before either is renamed.
#[test]
fn concurrent_sync_creating_same_path_with_different_content_keeps_the_later_one_under_the_name() {
    for first in ["laptop", "phone", "tablet"] {
        let srv = server();
        let (mut a, mut b) = race_to_create_today(&srv);
        let mut c = Device::new(&srv, "tablet", &[]);
        let r = match first {
            "laptop" => a.sync(),
            "phone" => b.sync(),
            _ => c.sync(),
        };
        assert_eq!(r.conflicts.len(), 1, "{first} first: {r:?}");
        converge(&mut [&mut a, &mut b, &mut c]);
        assert_laptop_keeps_today(&[&a, &b, &c], &format!("{first} syncing first"));
    }
}

/// Both devices decide at once: the phone syncs while the laptop is between
/// fetching the changes and applying them. Both give the phone's file a
/// conflict copy name; the second rename to reach the server is refused, and
/// that device takes the other's name.
#[test]
fn concurrent_creators_deciding_at_once_agree_on_the_name() {
    let srv = server();
    let (mut a, b) = race_to_create_today(&srv);
    let b = Arc::new(Mutex::new(b));
    let b2 = b.clone();
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        b2.lock().sync();
    }));
    a.sync();
    let mut b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    converge(&mut [&mut a, &mut b]);
    assert_laptop_keeps_today(&[&a, &b], "deciding at once");
}

/// The phone's file moves out of the way of the laptop's, and the same pull
/// brings an edit of it made on a third device that had not seen the
/// laptop's file: the edit lands in the moved file, with no extra copy (an
/// attachment cannot be merged, so a wrong merge would add one).
#[test]
fn file_moved_for_a_concurrent_one_takes_its_own_edit_from_the_same_pull() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.bin", "laptop\n")]);
    let b = Arc::new(Mutex::new(Device::new(&srv, "phone", &[("Today.bin", "phone\n")])));
    let c = Device::new(&srv, "tablet", &[]);
    let (fetched_tx, fetched_rx) = std::sync::mpsc::channel::<()>();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let tablet = Arc::new(Mutex::new(None));
    let (b2, tablet2) = (b.clone(), tablet.clone());
    // The laptop has fetched the (empty) changes: the phone uploads its
    // file, the tablet gets it and edits it, and fetches again before the
    // laptop's upload but uploads the edit only after it.
    *a.hooks.after_changes.lock() = Some(Box::new(move || {
        b2.lock().sync();
        let mut c = c;
        c.sync();
        c.write("Today.bin", "phone, edited on tablet\n");
        *c.hooks.after_changes.lock() = Some(Box::new(move || fetched_tx.send(()).unwrap()));
        *c.hooks.before_put.lock() = Some(Box::new(move || go_rx.recv().unwrap()));
        *tablet2.lock() = Some(std::thread::spawn(move || {
            c.sync();
            c
        }));
        fetched_rx.recv().unwrap();
    }));
    a.sync();
    go_tx.send(()).unwrap();
    let mut c = tablet.lock().take().unwrap().join().unwrap();
    let mut b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    let r = b.sync();
    assert!(r.skipped.is_empty() && r.conflicts.len() == 1, "{r:?}");
    converge(&mut [&mut a, &mut b, &mut c]);
    let copies = a.conflict_copies();
    assert_eq!(copies.len(), 1, "{:?}", a.files());
    assert_eq!(a.read("Today.bin").as_deref(), Some("laptop\n"));
    assert_eq!(a.read(&copies[0]).as_deref(), Some("phone, edited on tablet\n"));
    assert_eq!(a.files().len(), 2, "{:?}", a.files());
}

/// When a remote file is given a conflict name locally, the rename is
/// uploaded later in the same round ("force push"). If that intent lived
/// only in memory, a restarted round (409 from another device's upload, or
/// a file changed on disk) would forget it, and the devices would disagree
/// about the file's name until it changes again.
#[test]
fn conflict_rename_survives_a_restarted_round() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("A.md", "1\n2\n3\n")]);
    a.sync();
    let b = Arc::new(Mutex::new(Device::new(&srv, "phone", &[("P.md", "phone P\n")])));
    // phone: knows A.md, has an unsynced P.md
    {
        let mut bl = b.lock();
        // pull A.md only: temporarily hide P.md
        bl.mv("P.md", ".hide/P.md");
        bl.sync();
        bl.mv(".hide/P.md", "P.md");
    }
    // laptop creates its own P.md and uploads it
    a.write("P.md", "laptop P\n");
    a.sync();
    // phone edits A.md; while the phone syncs, the laptop edits A.md too and
    // uploads first, so the phone's first upload (A.md sorts first) gets 409.
    b.lock().write("A.md", "1 phone\n2\n3\n");
    let a_root = a.root.clone();
    let a_state = a.state_dir.clone();
    let url = srv.url.clone();
    *b.lock().hooks.before_put.lock() = Some(Box::new(move || {
        // a second engine instance on the laptop's vault and state
        fs::write(a_root.join("A.md"), "1\n2\n3 laptop\n").unwrap();
        let vault = Arc::new(cairn_core::Vault::open(std_fs(&a_root)).unwrap());
        let mut e = cairn_sync::engine::SyncEngine::load_with(
            vault,
            &a_state,
            cairn_sync::engine::SyncSettings { server: url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: "laptop".into() },
            Box::new(cairn_sync::transport::HttpTransport::new(&url, TOKEN)),
        )
        .unwrap();
        e.sync().unwrap();
    }));
    let r = b.lock().sync();
    eprintln!("phone sync: {r:?}, 409s: {}", b.lock().hooks.conflicts.load(std::sync::atomic::Ordering::SeqCst));
    // reload the laptop engine (its state changed under it)
    a.engine = cairn_sync::engine::SyncEngine::load(a.vault.clone(), &a.state_dir).unwrap().unwrap();
    let mut b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    let res = try_converge(&mut [&mut a, &mut b]);
    eprintln!("laptop {:?}\nphone {:?}", a.files(), b.files());
    res.unwrap();
}

/// The same when the round restarts because the user saved a note while
/// the sync was applying a remote edit to it ("file changed on disk").
#[test]
fn conflict_rename_survives_a_changed_on_disk_retry() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("N.md", "1\n2\n3\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    // laptop: a new P.md, then an edit of N.md (uploaded in that order)
    a.write("P.md", "laptop P\n");
    a.sync();
    a.write("N.md", "1 laptop\n2\n3\n");
    a.sync();
    // phone: its own unsynced P.md; the user saves N.md while the phone syncs
    b.write("P.md", "phone P\n");
    let n = b.abs("N.md");
    *b.hooks.after_changes.lock() = Some(Box::new(move || {
        std::thread::sleep(Duration::from_millis(3));
        fs::write(&n, "1\n2\n3 phone\n").unwrap();
    }));
    let r = b.sync();
    eprintln!("phone sync: rounds {} conflicts {:?}", r.rounds, r.conflicts);
    let res = try_converge(&mut [&mut a, &mut b]);
    eprintln!("laptop {:?}\nphone {:?}", a.paths(), b.paths());
    res.unwrap();
}

/// The same intent carries the user's own renames when a remote edit of the
/// renamed file arrives in the same sync ("rename on one side, edit on the
/// other"). If that round restarts (here: a 409 because a third device
/// uploaded another file a moment earlier), the rename must still be
/// uploaded; otherwise the laptop has b.md, everyone else keeps a.md, and
/// the next remote edit of the note renames it back to a.md on the laptop.
#[test]
fn user_rename_survives_a_restarted_round() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("0.md", "zero\n"), ("a.md", "1\n2\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    let c = Arc::new(Mutex::new(Device::new(&srv, "tablet", &[])));
    c.lock().sync();
    a.mv("a.md", "b.md");
    a.write("0.md", "zero\nlaptop\n");
    b.write("a.md", "1\n2\n3 phone\n");
    b.sync();
    let c2 = c.clone();
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        let mut c = c2.lock();
        c.write("0.md", "tablet\nzero\n");
        c.sync();
    }));
    let r = a.sync();
    eprintln!("laptop sync: rounds {}, 409s {}", r.rounds, a.hooks.conflicts.load(std::sync::atomic::Ordering::SeqCst));
    let mut c = Arc::try_unwrap(c).ok().unwrap().into_inner();
    let res = try_converge(&mut [&mut a, &mut b, &mut c]);
    eprintln!("after converge: laptop {:?}\nphone {:?}", a.paths(), b.paths());
    res.unwrap();
    // the phone edits the note again, under the name it has there (b.md
    // once the rename is uploaded): the laptop's rename must stay
    let name = b.files().into_iter().find(|f| f.1 == "1\n2\n3 phone\n").map(|f| f.0).expect("phone lost the note");
    b.write(&name, "1\n2\n3 phone\n4 phone\n");
    b.sync();
    a.sync();
    eprintln!("after the phone edits {name:?}: laptop {:?}", a.paths());
    assert_eq!(a.read("b.md").as_deref(), Some("1\n2\n3 phone\n4 phone\n"));
    assert!(!a.exists("a.md"), "the laptop has a.md again: {:?}", a.paths());
}

/// Conflict copy names append about 35 bytes to the file name. A note
/// whose name is already long (CJK titles are 3 bytes per character) would
/// get a conflict copy name over the 255-byte limit, and writing it would
/// fail: the conflict would stay pending, so the phone's edit of that note
/// would never reach the laptop. The stem is shortened instead.
#[test]
fn conflict_on_a_long_file_name_does_not_stop_sync() {
    let srv = server();
    // 70 CJK characters: 213 bytes, well under the 255-byte limit
    let title: String = "日本語のとても長いノートのタイトル".repeat(5).chars().take(70).collect();
    let name = format!("{title}.md");
    assert_eq!(name.len(), 213);
    let (mut a, mut b, _c) = trio(&srv, &[(&name, "base\n")]);
    a.write(&name, "laptop\n");
    b.write(&name, "phone\n");
    a.sync();
    let r = b.try_sync();
    eprintln!("phone sync: {:?}", r.as_ref().map(|r| &r.conflicts).map_err(|e| e.to_string()));
    // Unrelated work on the phone must still reach the laptop.
    b.write("other.md", "unrelated\n");
    let r2 = b.try_sync();
    a.sync();
    assert!(r.is_ok() && r2.is_ok(), "phone sync errors: {:?} / {:?}", r.err().map(|e| e.to_string()), r2.err().map(|e| e.to_string()));
    assert_eq!(a.read("other.md").as_deref(), Some("unrelated\n"));
    // The conflict itself must be resolved: the phone keeps its version and
    // the laptop's becomes a conflict copy, on both devices.
    let r = r.unwrap();
    assert_eq!(r.conflicts.len(), 1, "phone left out: {:?}", r.skipped);
    assert_eq!(a.read(&name).as_deref(), Some("phone\n"));
}

/// `StdFs::write` writes a temp file first. Its name must fit wherever the
/// target's does (one derived from the target, `.<name>.cairn-tmp-<pid>`,
/// was longer than the name), so that a file whose name is 237..=255 bytes
/// (legal on every file system; 79-85 CJK characters) can be downloaded and
/// the device keeps syncing.
#[test]
fn file_with_a_long_but_legal_name_syncs() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[]);
    let title: String = "日本語のとても長いノートのタイトル".repeat(5).chars().take(80).collect();
    let name = format!("{title}.md");
    assert_eq!(name.len(), 243);
    a.write(&name, "long name\n");
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    let r1 = b.try_sync();
    a.write("later.md", "later\n");
    a.sync();
    let r2 = b.try_sync();
    assert!(r1.is_ok() && r2.is_ok(), "phone: {:?} / {:?}", r1.err().map(|e| e.to_string()), r2.err().map(|e| e.to_string()));
    assert!(b.exists(&name) && b.exists("later.md"));
}

/// A remote file at a path where the receiving device has a folder (or a
/// remote file inside a folder whose name is a local file) cannot be
/// written. If that error aborted the pull before the cursor moves, every
/// later sync would fail at the same file and nothing else would sync.
#[test]
fn file_and_folder_with_the_same_name_do_not_stop_sync() {
    let mut errs: Vec<String> = Vec::new();
    for file_first in [true, false] {
        let srv = server();
        let mut a = Device::new(&srv, "laptop", &[("Projects", "a file called Projects\n")]);
        let mut b = Device::new(&srv, "phone", &[("Projects/plan.md", "plan\n")]);
        let (first, second) = if file_first { (&mut a, &mut b) } else { (&mut b, &mut a) };
        first.sync();
        let r1 = second.try_sync();
        // unrelated work must keep syncing
        second.write("unrelated.md", "x\n");
        let r2 = second.try_sync();
        first.sync();
        for (i, r) in [r1, r2].iter().enumerate() {
            if let Err(e) = r {
                errs.push(format!("file_first={file_first}: {} sync {}: {e}", second.name, i + 1));
            }
        }
        if !first.exists("unrelated.md") {
            errs.push(format!("file_first={file_first}: unrelated.md never reached {}", first.name));
        }
    }
    assert!(errs.is_empty(), "{errs:#?}");
}

/// Same failure mode with names that are legal on Linux but not on Windows
/// or Android shared storage (`?`, `:`, `"` ...): one such file must not
/// block the whole vault on the receiving device. With the device name put
/// into conflict copy names with only `/ \ :` replaced, a device called
/// "Sam's Pixel?" would produce such names by itself; the name is
/// sanitized, and the conflict copy arrives everywhere.
#[test]
fn name_refused_by_receiving_fs_does_not_stop_sync() {
    let srv = server();
    let mut a = Device::new(&srv, "Sam's Pixel?", &[("n.md", "base\n")]);
    a.sync();
    let mut w = Device::with_fs(&srv, "windows", &[], FatNamesFs::new);
    w.sync();
    let mut b = Device::new(&srv, "laptop", &[]);
    b.sync();
    // a conflict on the "Sam's Pixel?" device creates "n (conflict ... Sam's Pixel?).md"
    b.write("n.md", "laptop\n");
    a.write("n.md", "pixel\n");
    b.sync();
    let r = a.sync();
    assert_eq!(r.conflicts.len(), 1);
    eprintln!("conflict copy: {:?}", r.conflicts);
    let rw = w.try_sync();
    a.write("later.md", "later\n");
    a.sync();
    let rw2 = w.try_sync();
    assert!(rw.is_ok() && rw2.is_ok(), "windows device: {:?} / {:?}", rw.err().map(|e| e.to_string()), rw2.err().map(|e| e.to_string()));
    assert!(w.exists("later.md"));
    let rw2 = rw2.unwrap();
    assert!(rw2.skipped.is_empty(), "windows device left out: {:?}", rw2.skipped);
}

/// A note made outside Cairn with a name the receiving file system refuses
/// ("Why?.md" on Linux, received on Windows) is reported there on every
/// sync and waits; the other files keep syncing in both directions.
#[cfg(not(windows))] // The laptop plays Linux. Windows cannot hold '?' in a name.
#[test]
fn name_refused_here_is_reported_and_the_rest_syncs() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Why?.md", "why\n"), ("n.md", "n\n")]);
    a.sync();
    let mut w = Device::with_fs(&srv, "windows", &[], FatNamesFs::new);
    for _ in 0..2 {
        let r = w.sync();
        assert_eq!(r.skipped.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["Why?.md"]);
    }
    assert_eq!(w.paths(), ["n.md"]);
    w.write("w.md", "from windows\n");
    w.sync();
    a.write("n.md", "n2\n");
    a.sync();
    w.sync();
    assert_eq!(w.read("n.md").as_deref(), Some("n2\n"));
    assert_eq!(a.read("w.md").as_deref(), Some("from windows\n"));
    assert_eq!(a.read("Why?.md").as_deref(), Some("why\n"));
}

fn stamp(secs: i64) -> String {
    // same algorithm as the engine's conflict names
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}", rem / 3600, (rem % 3600) / 60)
}

// ===================================================================== merges

fn merge_case(base: &str, ours: &str, theirs: &str) -> (Vec<(String, String)>, Vec<String>) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", base)]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.write("n.md", theirs);
    b.write("n.md", ours);
    a.sync();
    let r = b.sync();
    converge(&mut [&mut a, &mut b]);
    (a.files(), r.conflicts)
}

#[test]
fn merge_both_append_different_lines_at_eof() {
    let (files, conflicts) = merge_case("a\nb\n", "a\nb\nphone\n", "a\nb\nlaptop\n");
    let all: String = files.iter().map(|f| f.1.clone()).collect();
    assert!(all.contains("phone") && all.contains("laptop"), "{files:?}");
    eprintln!("EOF appends: conflicts={conflicts:?} files={files:?}");
}

#[test]
fn merge_both_insert_the_same_line_at_the_same_spot() {
    let (files, conflicts) = merge_case("a\nb\nc\n", "a\nb\nNEW\nc\n", "a\nb\nNEW\nc\n");
    assert!(conflicts.is_empty());
    assert_eq!(files, vec![("n.md".to_string(), "a\nb\nNEW\nc\n".to_string())]);
}

#[test]
fn merge_identical_change_plus_separate_changes() {
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1 o\n2\n3\n4\n5 same\n6\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\n5 same\n6\n7\n8\n9 t\n";
    let (files, conflicts) = merge_case(base, ours, theirs);
    assert!(conflicts.is_empty(), "{conflicts:?}");
    assert_eq!(files[0].1, "1 o\n2\n3\n4\n5 same\n6\n7\n8\n9 t\n");
}

#[test]
fn merge_without_trailing_newline() {
    let (files, conflicts) = merge_case("a\nb\nc", "a phone\nb\nc", "a\nb\nc laptop");
    assert!(conflicts.is_empty(), "{conflicts:?} {files:?}");
    assert_eq!(files, vec![("n.md".to_string(), "a phone\nb\nc laptop".to_string())]);
}

#[test]
fn merge_crlf_note_edited_on_both_sides() {
    let base = "a\r\nb\r\nc\r\nd\r\ne\r\n";
    let (files, conflicts) = merge_case(base, "a phone\r\nb\r\nc\r\nd\r\ne\r\n", "a\r\nb\r\nc\r\nd\r\ne laptop\r\n");
    assert!(conflicts.is_empty(), "{conflicts:?}");
    assert_eq!(files[0].1, "a phone\r\nb\r\nc\r\nd\r\ne laptop\r\n");
}

#[test]
fn merge_crlf_conversion_vs_edit_keeps_both() {
    let (files, _c) = merge_case("a\nb\nc\n", "a\r\nb\r\nc\r\n", "a\nb laptop\nc\n");
    let all: String = files.iter().map(|f| f.1.clone()).collect();
    assert!(all.contains("b laptop"), "{files:?}");
}

#[test]
fn merge_unicode_lines() {
    let base = "αβγ\n日本語\n🙂 emoji\nZ\u{301}algo\n";
    let (files, conflicts) = merge_case(base, "αβγ δ\n日本語\n🙂 emoji\nZ\u{301}algo\n", "αβγ\n日本語\n🙂 emoji 🎉\nZ\u{301}algo\n");
    assert!(conflicts.is_empty());
    assert_eq!(files[0].1, "αβγ δ\n日本語\n🙂 emoji 🎉\nZ\u{301}algo\n");
}

#[test]
fn merge_md_with_invalid_utf8_becomes_conflict_copy() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[]);
    a.write_bytes("latin1.md", b"caf\xe9\nline\n");
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    a.write_bytes("latin1.md", b"caf\xe9 laptop\nline\n");
    b.write_bytes("latin1.md", b"caf\xe9\nline phone\n");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b]);
    let all: Vec<Vec<u8>> = a.files_bytes().into_iter().map(|f| f.1).collect();
    assert!(all.contains(&b"caf\xe9 laptop\nline\n".to_vec()));
    assert!(all.contains(&b"caf\xe9\nline phone\n".to_vec()));
}

#[test]
fn merge_txt_and_uppercase_md_extension() {
    for name in ["notes.txt", "README.MD", "x.markdown"] {
        let srv = server();
        let mut a = Device::new(&srv, "laptop", &[(name, "1\n2\n3\n")]);
        a.sync();
        let mut b = Device::new(&srv, "phone", &[]);
        b.sync();
        a.write(name, "1 a\n2\n3\n");
        b.write(name, "1\n2\n3 b\n");
        a.sync();
        let r = b.sync();
        converge(&mut [&mut a, &mut b]);
        assert!(r.conflicts.is_empty(), "{name}: {r:?}");
        assert_eq!(a.read(name).as_deref(), Some("1 a\n2\n3 b\n"), "{name}");
    }
}

#[test]
fn merge_large_note_with_many_changes_finishes_quickly() {
    let base: String = (0..20_000).map(|i| format!("line {i} lorem ipsum dolor sit amet\n")).collect();
    let mut ours: Vec<String> = base.lines().map(String::from).collect();
    let mut theirs = ours.clone();
    for i in (0..20_000).step_by(10) {
        ours[i] = format!("ours {i}");
        theirs[i + 5] = format!("theirs {}", i + 5);
    }
    let ours = ours.join("\n") + "\n";
    let theirs = theirs.join("\n") + "\n";
    let t = std::time::Instant::now();
    let (files, conflicts) = merge_case(&base, &ours, &theirs);
    let el = t.elapsed();
    assert!(conflicts.is_empty());
    let got = &files[0].1;
    assert!(got.contains("ours 19990") && got.contains("theirs 19995"));
    assert!(el < Duration::from_secs(20), "took {el:?}");
    eprintln!("large merge took {el:?}");
}

#[test]
fn merge_large_note_rewritten_on_both_sides_finishes_quickly() {
    let base: String = (0..5_000).map(|i| format!("base line {i}\n")).collect();
    let ours: String = (0..5_000).map(|i| format!("ours line {i}\n")).collect();
    let theirs: String = (0..5_000).map(|i| format!("theirs line {i}\n")).collect();
    let t = std::time::Instant::now();
    let (files, conflicts) = merge_case(&base, &ours, &theirs);
    let el = t.elapsed();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(files.len(), 2);
    assert!(el < Duration::from_secs(20), "took {el:?}");
    eprintln!("rewrite merge took {el:?}");
}

#[test]
fn binary_edit_vs_edit_and_empty_files() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("e1.md", ""), ("e2.md", ""), ("img.png", "\u{89}PNG v1")]);
    a.write_bytes("img.png", b"\x89PNG\xff laptop");
    b.write_bytes("img.png", b"\x89PNG\xfe phone");
    a.write("e1.md", "now has text\n");
    b.rm("e2.md");
    a.sync();
    b.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    let bytes: Vec<Vec<u8>> = a.files_bytes().into_iter().map(|f| f.1).collect();
    assert!(bytes.contains(&b"\x89PNG\xff laptop".to_vec()));
    assert!(bytes.contains(&b"\x89PNG\xfe phone".to_vec()));
    assert_eq!(a.read("e1.md").as_deref(), Some("now has text\n"));
    assert!(!a.exists("e2.md"));
}

// ===================================================================== clock skew / mtimes

#[test]
fn far_past_and_future_mtimes_do_not_change_outcomes() {
    let srv = server();
    let (mut a, mut b, mut c) = trio(&srv, &[("past.md", "1\n2\n3\n"), ("future.md", "1\n2\n3\n"), ("pic.png", "v1")]);
    let past = UNIX_EPOCH + Duration::from_secs(86_400 * 400); // 1971
    let future = UNIX_EPOCH + Duration::from_secs(13_000_000_000); // year 2381
    // laptop: content edits with skewed mtimes
    a.write("past.md", "1 laptop\n2\n3\n");
    set_mtime(&a.abs("past.md"), past);
    a.write("future.md", "1\n2\n3 laptop\n");
    set_mtime(&a.abs("future.md"), future);
    // phone: only mtime changes (touch), plus an edit
    set_mtime(&b.abs("past.md"), future);
    set_mtime(&b.abs("pic.png"), past);
    b.write("future.md", "1 phone\n2\n3\n");
    set_mtime(&b.abs("future.md"), past);
    let rb = b.sync();
    assert_eq!(rb.pushed, 1, "touching files must not upload them: {rb:?}");
    a.sync();
    c.sync();
    converge(&mut [&mut a, &mut b, &mut c]);
    assert_eq!(a.read("past.md").as_deref(), Some("1 laptop\n2\n3\n"));
    assert_eq!(a.read("future.md").as_deref(), Some("1 phone\n2\n3 laptop\n"));
    assert!(a.conflict_copies().is_empty());
    // touching again (mtime jumps backwards) changes nothing
    set_mtime(&c.abs("future.md"), past);
    let r = c.sync();
    assert_eq!((r.pushed, r.conflicts.len()), (0, 0));
}

/// The engine skips hashing a file when size and mtime equal what it
/// recorded. An edit that keeps the size and leaves the same mtime (a
/// typo fix saved by a tool that preserves timestamps, a coarse-mtime file
/// system such as FAT's 2 s, or a clock that does not move) must still be
/// uploaded. Otherwise a later remote edit of that file hits the "changed
/// on disk" check on every round, so every sync of the device fails.
#[test]
fn same_size_edit_with_unchanged_mtime_is_synced() {
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[("n.md", "teh cat\nline 2\n")]);
    a.sync();
    let p = a.abs("n.md");
    let t = mtime(&p);
    fs::write(&p, "the cat\nline 2\n").unwrap(); // same size
    set_mtime(&p, t);
    let r = a.sync();
    b.sync();
    let pushed = r.pushed;
    let phone_had = b.read("n.md");
    // now the phone edits the same note
    b.write("n.md", "teh cat\nline 2\nline 3 phone\n");
    b.sync();
    let r2 = a.try_sync();
    let r3 = a.try_sync();
    eprintln!(
        "pushed={pushed}; phone had {phone_had:?}; laptop syncs after the phone's edit: {:?} / {:?}",
        r2.as_ref().map(|r| r.pulled).map_err(|e| e.to_string()),
        r3.as_ref().map(|r| r.pulled).map_err(|e| e.to_string())
    );
    assert_eq!(pushed, 1, "the typo fix was not uploaded; phone has {phone_had:?}");
    assert!(r2.is_ok() && r3.is_ok(), "laptop sync now fails");
}

#[test]
fn conflict_copy_names_use_the_local_clock_only_for_the_name() {
    // The winner of a conflict never depends on clocks: only on which
    // device reached the server first, even if the earlier device's files
    // carry far-future mtimes.
    let srv = server();
    let (mut a, mut b, _c) = trio(&srv, &[("n.md", "base\n")]);
    a.write("n.md", "laptop\n");
    set_mtime(&a.abs("n.md"), UNIX_EPOCH + Duration::from_secs(13_000_000_000));
    b.write("n.md", "phone\n");
    set_mtime(&b.abs("n.md"), UNIX_EPOCH + Duration::from_secs(1));
    b.sync();
    let r = a.sync();
    assert_eq!(r.conflicts.len(), 1);
    // the device that synced second keeps its own text in place
    assert_eq!(a.read("n.md").as_deref(), Some("laptop\n"));
    assert_eq!(a.read(&r.conflicts[0]).as_deref(), Some("phone\n"));
    assert!(r.conflicts[0].contains("laptop"), "conflict copy is named after the device that made it: {:?}", r.conflicts);
}
