//! Regression tests for FINDING-014: a "push this rename" intent lived only
//! in memory for one round, while apply_remote had already recorded the
//! LOCAL path as if it were the server's path in state.json. When the round
//! did not get to push it, the rename was never uploaded.
//!
//! 1. A version of adv_sync_semantics.rs
//!    `user_rename_survives_a_restarted_round`: the phone's last step must
//!    not write "a.md", which only exists there if the rename was lost;
//!    with a correct engine the phone has "b.md", so writing "a.md" would
//!    create a new note. Here the phone edits the note under whatever name
//!    it currently has.
//! 2. A variant with no faults at all: the 409 comes from another device
//!    editing the SAME renamed note during the laptop's sync. With the
//!    defect, the next round saw the note as Unchanged and applied the
//!    remote head's path, so the laptop user's rename was undone inside
//!    that one sync.
//!
//!   cargo test -p cairn-sync --test adv_verify_ss_09

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::sync::Arc;

use common::*;
use parking_lot::Mutex;

#[test]
fn user_rename_survives_a_restarted_round_corrected() {
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
    // the tablet uploads 0.md between the laptop's pull and its first put
    let c2 = c.clone();
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        let mut c = c2.lock();
        c.write("0.md", "tablet\nzero\n");
        c.sync();
    }));
    let r = a.sync();
    let mut c = Arc::try_unwrap(c).ok().unwrap().into_inner();
    let conv = try_converge(&mut [&mut a, &mut b, &mut c]);
    let phone_name = b.files().into_iter().find(|f| f.1 == "1\n2\n3 phone\n").map(|f| f.0);
    eprintln!(
        "laptop sync rounds {} (409s {}); converge: {conv:?}\nlaptop {:?}\nphone {:?}",
        r.rounds,
        a.hooks.conflicts.load(std::sync::atomic::Ordering::SeqCst),
        a.paths(),
        b.paths()
    );
    // the phone edits the note under the name it has for it
    let phone_name = phone_name.expect("phone lost the note");
    b.write(&phone_name, "1\n2\n3 phone\n4 phone\n");
    b.sync();
    a.sync();
    eprintln!("after the phone edits {phone_name:?}: laptop {:?}", a.paths());
    assert_eq!(
        a.read("b.md").as_deref(),
        Some("1\n2\n3 phone\n4 phone\n"),
        "laptop's rename a.md -> b.md was lost: laptop has {:?}",
        a.paths()
    );
    assert!(!a.exists("a.md"), "laptop has a.md again: {:?}", a.paths());
    conv.unwrap();
}

#[test]
fn user_rename_survives_a_409_on_the_same_note() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "1\n2\n3\n")]);
    a.sync();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync();
    let c = Arc::new(Mutex::new(Device::new(&srv, "tablet", &[])));
    c.lock().sync();
    // laptop renames, phone edits line 3 (both offline)
    a.mv("a.md", "b.md");
    b.write("a.md", "1\n2\n3 phone\n");
    b.sync();
    // during the laptop's sync, the tablet (which has the phone's edit)
    // edits line 1 of the same note and uploads first
    let c2 = c.clone();
    *a.hooks.before_put.lock() = Some(Box::new(move || {
        let mut c = c2.lock();
        c.sync();
        c.write("a.md", "1 tablet\n2\n3 phone\n");
        c.sync();
    }));
    let r = a.sync();
    eprintln!(
        "laptop sync rounds {} (409s {}), laptop now {:?}",
        r.rounds,
        a.hooks.conflicts.load(std::sync::atomic::Ordering::SeqCst),
        a.files()
    );
    let mut c = Arc::try_unwrap(c).ok().unwrap().into_inner();
    converge(&mut [&mut a, &mut b, &mut c]);
    eprintln!("after converge: {:?}", a.files());
    // PLAN rule: rename on one side, edits on the others: all apply
    assert_eq!(a.files(), vec![("b.md".to_string(), "1 tablet\n2\n3 phone\n".to_string())], "the laptop user's rename was undone");
}
