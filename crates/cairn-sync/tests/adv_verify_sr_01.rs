//! Reproduction for FINDING-143 with a real process and a real
//! SIGKILL (no simulated panic, no hooked file system).
//!
//! Device A's sync runs as the prebuilt `target/debug/examples/sync_dir`
//! binary under gdb. gdb stops it at the first call of
//! `SyncEngine::save_state`, which in `round()` is the call right after the
//! pull loop: every pulled vault change is on disk (vault
//! writes are fsynced), state.json is not yet updated. gdb then kills the
//! process with SIGKILL, the same as the OS killing the app at that moment.
//! A then syncs normally (new sync_dir processes), B is an in-process device.
//!
//! Needs gdb and target/debug/examples/sync_dir, which `cargo test --test
//! adv_verify_sr_01` does not rebuild. Run:
//!   cargo build -p cairn-sync --example sync_dir
//!   cargo test -p cairn-sync --test adv_verify_sr_01

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;

fn sync_dir_bin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/examples/sync_dir")
}

fn args(d: &Device) -> Vec<String> {
    vec![
        d.root.to_string_lossy().to_string(),
        d.state_dir.to_string_lossy().to_string(),
        d.url.clone(),
        TOKEN.to_string(),
        VAULT_ID.to_string(),
        d.name.clone(),
        PASS.to_string(),
    ]
}

/// One complete sync of `d` in a separate sync_dir process.
fn proc_sync(d: &Device) -> String {
    let out = Command::new(sync_dir_bin()).args(args(d)).output().expect("run sync_dir");
    assert!(out.status.success(), "sync_dir {} failed: {}", d.name, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Run a sync of `d` in sync_dir under gdb and SIGKILL it at the first
/// save_state() call (right after the pull loop, before state.json is saved).
fn proc_sync_killed_after_pull(d: &Device) {
    let state_before = std::fs::read_to_string(d.state_file()).unwrap();
    let out = Command::new("gdb")
        .env("DEBUGINFOD_URLS", "")
        .args([
            "-nx",
            "-batch",
            "-iex",
            "set debuginfod enabled off",
            "-ex",
            "set pagination off",
            "-ex",
            "set confirm off",
            "-ex",
            "break cairn_sync::engine::SyncEngine::save_state",
            "-ex",
            "run",
            "-ex",
            "kill",
            "--args",
        ])
        .arg(sync_dir_bin())
        .args(args(d))
        .output()
        .expect("run gdb");
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(so.contains("Breakpoint 1, "), "gdb did not stop at save_state:\n{so}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(so.contains("[Inferior 1 (process") && so.contains("killed]"), "gdb did not kill the process:\n{so}");
    // the process died before state.json was rewritten
    assert_eq!(std::fs::read_to_string(d.state_file()).unwrap(), state_before, "state.json changed before the kill");
}

fn converge_proc(a: &Device, b: &mut Device) {
    for _ in 0..3 {
        proc_sync(a);
        b.sync_ok();
    }
}

#[test]
fn sigkill_after_pulled_rename_and_edit_keeps_the_note_name() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Inbox/Idea.md", "idea 1\nidea 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.mv("Inbox/Idea.md", "Projects/Idea.md");
    b.sync_ok();
    b.write("Projects/Idea.md", "idea 1\nidea 2\nidea 3 phone\n");
    b.sync_ok();

    proc_sync_killed_after_pull(&a);
    // the pull's changes are on disk: renamed and rewritten
    assert_eq!(a.files(), vec![("Projects/Idea.md".to_string(), "idea 1\nidea 2\nidea 3 phone\n".to_string())]);

    // A's first sync after the restart: the tracked path is missing and the
    // new path holds other content. Read as Deleted + Created, that would give
    // a conflict copy for the pulled head plus a new file id for
    // Projects/Idea.md (the FINDING-143 duplicate). B would then give that new
    // id a second conflict name because of its stale pre-pull snapshot
    // (FINDING-061), so the name would be lost.
    eprintln!("A after the first post-kill sync: {}", proc_sync(&a));
    eprintln!("A files: {:?}", a.paths());
    b.sync_ok();
    eprintln!("B files after its sync: {:?}", b.paths());

    converge_proc(&a, &mut b);
    assert_eq!(a.files(), b.files(), "devices did not converge");
    assert_eq!(
        a.files(),
        vec![("Projects/Idea.md".to_string(), "idea 1\nidea 2\nidea 3 phone\n".to_string())],
        "after one SIGKILL the note is no longer where the user put it"
    );
}

#[test]
fn sigkill_after_pulled_edit_into_locally_renamed_note_does_not_duplicate() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Draft.md", "line 1\nline 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("Draft.md", "line 1\nline 2 phone\n");
    b.sync_ok();
    a.mv("Draft.md", "Final.md");

    proc_sync_killed_after_pull(&a);
    assert_eq!(a.files(), vec![("Final.md".to_string(), "line 1\nline 2 phone\n".to_string())]);

    converge_proc(&a, &mut b);
    assert_eq!(a.files(), b.files(), "devices did not converge");
    assert_eq!(a.files(), vec![("Final.md".to_string(), "line 1\nline 2 phone\n".to_string())]);
}

/// Case (3) of FINDING-143 does NOT reproduce with a kill at the end of the
/// pull: apply_remote already saved the remote content as the merge base
/// (save_base, a plain fs::write outside state.json), so after the
/// restart merge(base = theirs, ours, theirs) is clean and no second copy is
/// made. The duplicate conflict copy needs the kill to land between the copy
/// write and save_base of that same head, a
/// window of microseconds (what the in-process PanicAfter hook hits).
#[test]
fn sigkill_at_end_of_pull_after_conflict_copy_does_not_duplicate_it() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "same\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("n.md", "phone\n");
    b.sync_ok();
    a.write("n.md", "laptop\n");

    proc_sync_killed_after_pull(&a);
    assert_eq!(conflict_copies(&a.files()).len(), 1, "{:?}", a.files());

    converge_proc(&a, &mut b);
    assert_eq!(a.files(), b.files(), "devices did not converge");
    let files = a.files();
    assert_eq!(conflict_copies(&files).len(), 1, "{files:?}");
}

/// Control: the same kill point with only a remote edit (no rename) is
/// harmless, so the failures above are not an artifact of the kill method.
#[test]
fn control_sigkill_after_pulled_plain_edit_is_harmless() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Inbox/Idea.md", "idea 1\nidea 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("Inbox/Idea.md", "idea 1\nidea 2\nidea 3 phone\n");
    b.sync_ok();

    proc_sync_killed_after_pull(&a);
    converge_proc(&a, &mut b);
    assert_eq!(a.files(), b.files(), "devices did not converge");
    assert_eq!(a.files(), vec![("Inbox/Idea.md".to_string(), "idea 1\nidea 2\nidea 3 phone\n".to_string())]);
}
