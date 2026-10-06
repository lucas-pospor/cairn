//! Regression tests for FINDING-063: two devices that upload a new file at
//! the same path in overlapping syncs used to both rename the other's file
//! to a conflict copy, so nobody kept the name, and identical files were
//! not merged.
//!
//! The simplest case has the phone sync before the laptop's first
//! upload. This file shows the case a user is more likely to hit: the
//! same vault exists on two devices (copied over, or previously synced
//! with another tool) and sync is set up on the second device while the
//! first is still doing its initial upload. Every file the laptop had not
//! uploaded yet when the phone pulled could end up as two identical
//! conflict copies on both devices, with the original name gone (wiki
//! links to it would break).
//!
//! Run:
//!   cargo test -p cairn-sync --test adv_verify_ss_12 -- --include-ignored

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::sync::Arc;

use common::*;
use parking_lot::Mutex;

/// Install a `before_put` hook on `hooks` that runs `f` right before the
/// `(skip + 1)`-th upload from now.
fn before_nth_put(hooks: Arc<Hooks>, skip: usize, f: Hook) {
    let h2 = hooks.clone();
    *hooks.before_put.lock() = Some(Box::new(move || {
        if skip == 0 {
            f();
        } else {
            before_nth_put(h2, skip - 1, f);
        }
    }));
}

fn vault_files(n: usize) -> Vec<(String, String)> {
    (0..n).map(|i| (format!("note {i:02}.md"), format!("body of note {i}\n"))).collect()
}

/// Control: if the phone connects after the laptop's initial upload has
/// finished, identical files are adopted and nothing is duplicated.
#[test]
fn second_device_connecting_after_initial_upload_adopts_identical_files() {
    let srv = server();
    let files = vault_files(20);
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let mut a = Device::new(&srv, "laptop", &refs);
    a.sync();
    let mut b = Device::new(&srv, "phone", &refs);
    b.sync();
    converge(&mut [&mut a, &mut b]);
    assert_eq!(a.files(), files, "{:?}", a.paths());
}

/// The phone connects (and syncs) while the laptop is halfway through its
/// initial upload of the same 20 notes.
#[test]
fn second_device_connecting_during_initial_upload_keeps_names() {
    let srv = server();
    let files = vault_files(20);
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let mut a = Device::new(&srv, "laptop", &refs);
    let b = Arc::new(Mutex::new(Device::new(&srv, "phone", &refs)));
    let b2 = b.clone();
    // The phone's sync runs after the laptop has uploaded 10 of 20 notes.
    before_nth_put(
        a.hooks.clone(),
        10,
        Box::new(move || {
            b2.lock().sync();
        }),
    );
    a.sync();
    let mut b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    converge(&mut [&mut a, &mut b]);
    let got = a.files();
    let conflicts = a.conflict_copies();
    let missing: Vec<&String> = files.iter().map(|f| &f.0).filter(|p| !got.iter().any(|g| &g.0 == *p)).collect();
    eprintln!("{} files, {} conflict copies, original names missing: {:?}", got.len(), conflicts.len(), missing);
    assert!(missing.is_empty(), "original names gone: {missing:?}\nall files: {:?}", a.paths());
    assert!(conflicts.is_empty(), "identical notes duplicated as conflict copies: {conflicts:?}");
    assert_eq!(got, files);
}
