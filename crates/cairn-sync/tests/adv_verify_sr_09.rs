//! Reproduction for FINDING-057 (one file that cannot be read
//! or written stops every sync on that device).
//!
//! Run: cargo test -p cairn-sync --test adv_verify_sr_09 -- --ignored --nocapture
//!
//! What these show beyond sync stopping:
//! - (a) the unreadable-file case aborts in `scan()` before the pull starts
//!   (last_seq does not move, nothing is pulled), so it is a separate code path
//!   from the FINDING-017 pull loop. The vault itself opens and indexes around the
//!   unreadable note; only sync stops. Fixing the permission recovers.
//! - (c) the file-vs-folder clash fails in both directions, because
//!   `exists()` only counts files, so no conflict name is chosen.

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::*;

fn pair(files: &[(&str, &str)]) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", files);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    (srv, a, b)
}

fn last_seq(d: &Device) -> u64 {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(d.state_file()).unwrap()).unwrap();
    v["last_seq"].as_u64().unwrap_or(0)
}

#[test]
fn unreadable_note_stops_pull_and_push_until_fixed() {
    let (_srv, mut a, mut b) = pair(&[("n.md", "v1\n")]);
    a.write("private.md", "secret\n");
    fs::set_permissions(a.root.join("private.md"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(a.root.join("private.md")).is_ok() {
        eprintln!("running as root; permission test not meaningful");
        return;
    }
    a.write("n.md", "v2 laptop\n");
    b.write("from_phone.md", "hello\n");
    b.sync_ok();

    let seq_before = last_seq(&a);
    let mut errs = Vec::new();
    for _ in 0..3 {
        if let Err(e) = a.sync() {
            errs.push(e.to_string());
        }
    }
    let seq_after = last_seq(&a);
    let pulled = a.read("from_phone.md").is_some();
    b.sync_ok();
    let pushed = b.read("n.md").as_deref() == Some("v2 laptop\n");
    // The vault itself is fine with the unreadable note (Vault::open/rescan skip it).
    let _ = a.vault.rescan();
    let in_index = a.vault.entries().iter().any(|e| e.path == "private.md");

    fs::set_permissions(a.root.join("private.md"), fs::Permissions::from_mode(0o644)).unwrap();
    let after_fix = a.sync().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string());
    b.sync_ok();
    let recovered = a.read("from_phone.md").is_some() && b.read("n.md").as_deref() == Some("v2 laptop\n");

    eprintln!(
        "3 syncs with an unreadable note: {} errors, first {:?}\n\
         last_seq {seq_before} -> {seq_after}; pulled from_phone.md: {pulled}; pushed n.md edit: {pushed}; \
         private.md still in the vault index: {in_index}\n\
         after chmod 644: {after_fix:?}; recovered: {recovered}",
        errs.len(),
        errs.first()
    );
    assert!(
        errs.is_empty() && pulled && pushed,
        "one unreadable note blocked every pull and push: {} of 3 syncs failed ({:?}); pulled {pulled}, pushed {pushed}",
        errs.len(),
        errs.first()
    );
}

#[test]
fn folder_first_then_file_also_blocks() {
    // Reverse of the usual order: the folder is on the server first and
    // the device that pulls it has a plain file with that name.
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Projects/plan.md", "the plan\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[("Projects", "a plain file\n"), ("other.md", "o\n")]);
    let mut errs = Vec::new();
    for _ in 0..2 {
        if let Err(e) = b.sync() {
            errs.push(e.to_string());
        }
    }
    let _ = a.sync();
    let all = format!("{}{}", a.all_text(), b.all_text());
    eprintln!("phone errors: {errs:?}\nlaptop: {:?}\nphone: {:?}", a.paths(), b.paths());
    assert!(
        errs.is_empty() && all.contains("a plain file") && all.contains("the plan") && a.read("other.md").is_some(),
        "phone: {errs:?}; laptop has {:?}, phone has {:?}",
        a.paths(),
        b.paths()
    );
}
