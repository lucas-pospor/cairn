//! Adversarial sync robustness: crashes at every step of a sync, lost
//! responses, damaged or lost local state, concurrent edits while a sync
//! writes, and two engines on one state folder.
//!
//! Run: cargo test -p cairn-sync --test adv_sync_robust
//! Failing reproductions are #[ignore = "FINDING-nnn: ..."]; run one with
//!   cargo test -p cairn-sync --test adv_sync_robust -- --ignored --exact <name>

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::*;

// ------------------------------------------------------------------ scenario

/// A synced vault, then concurrent offline changes on both devices that
/// exercise every branch of apply_remote. B has synced; A has not.
struct Scn {
    _srv: Server,
    a: Device,
    b: Device,
    /// include "A renames, B edits"
    rel: bool,
}

const BASE: &[(&str, &str)] = &[
    ("keep.md", "keep\n"),
    ("edit_remote.md", "er 1\ner 2\n"),
    ("rename_remote.md", "rr\n"),
    ("renedit_remote.md", "re 1\nre 2\n"),
    ("delete_remote.md", "dr\n"),
    ("merge.md", "m 1\nm 2\nm 3\nm 4\nm 5\n"),
    ("conflict.md", "c 1\nc 2\nc 3\n"),
    ("local_edit.md", "le\n"),
    ("local_rename.md", "lr\n"),
    ("local_delete.md", "ld\n"),
    ("att.png", "PNG v1"),
    ("ren_then_edit.md", "rte 1\nrte 2\n"),
    ("remote_ren_local_edit.md", "rrle 1\nrrle 2\n"),
    ("remote_edit_local_ren.md", "rel 1\nrel 2\n"),
];

fn setup() -> Scn {
    setup_opts(false)
}

fn setup_opts(rel: bool) -> Scn {
    let srv = server();
    let base: Vec<(&str, &str)> = BASE.iter().copied().filter(|f| rel || f.0 != "remote_edit_local_ren.md").collect();
    let mut a = Device::new(&srv, "laptop", &base);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    // remote changes (B)
    b.write("edit_remote.md", "er 1\ner 2 phone\n");
    b.mv("rename_remote.md", "sub/renamed.md");
    b.mv("renedit_remote.md", "sub/renedit.md");
    b.write("sub/renedit.md", "re 1\nre 2 phone\n");
    b.rm("delete_remote.md");
    b.write("merge.md", "m 1 phone\nm 2\nm 3\nm 4\nm 5\n");
    b.write("conflict.md", "c 1\nc 2 phone\nc 3\n");
    b.write("new_remote.md", "nr phone\n");
    b.write("att.png", "PNG v2 phone");
    b.mv("remote_ren_local_edit.md", "sub/rrle.md");
    if rel {
        b.write("remote_edit_local_ren.md", "rel 1\nrel 2 phone\n");
    }
    // a rename and an edit of the same file id in two syncs: A gets one head
    // with a new path and new content
    b.mv("ren_then_edit.md", "sub/ren_then_edit.md");
    b.sync_ok();
    b.write("sub/ren_then_edit.md", "rte 1\nrte 2 phone\n");
    b.sync_ok();
    // local changes (A)
    a.write("merge.md", "m 1\nm 2\nm 3\nm 4\nm 5 laptop\n");
    a.write("conflict.md", "c 1\nc 2 laptop\nc 3\n");
    a.write("local_edit.md", "le laptop\n");
    a.mv("local_rename.md", "moved/local_renamed.md");
    a.rm("local_delete.md");
    a.write("new_local.md", "nl laptop\n");
    a.write("remote_ren_local_edit.md", "rrle 1 laptop\nrrle 2\n");
    if rel {
        a.mv("remote_edit_local_ren.md", "moved/rel.md");
    }
    Scn { _srv: srv, a, b, rel }
}

const CONFLICT_LAPTOP: &str = "c 1\nc 2 laptop\nc 3\n";
const CONFLICT_PHONE: &str = "c 1\nc 2 phone\nc 3\n";

/// Converge and check the outcome is exactly what a crash-free sync gives:
/// every edit, rename, delete and create applied once, one conflict copy.
fn check(s: &mut Scn) -> Result<(), String> {
    for _ in 0..3 {
        s.a.sync().map_err(|e| format!("A sync: {e}"))?;
        s.b.sync().map_err(|e| format!("B sync: {e}"))?;
    }
    let fa = s.a.files();
    let fb = s.b.files();
    if fa != fb {
        let only_a: Vec<&(String, String)> = fa.iter().filter(|f| !fb.contains(f)).collect();
        let only_b: Vec<&(String, String)> = fb.iter().filter(|f| !fa.contains(f)).collect();
        return Err(format!("not converged: only on A {only_a:?}, only on B {only_b:?}"));
    }
    let want_all: &[(&str, &str)] = &[
        ("att.png", "PNG v2 phone"),
        ("edit_remote.md", "er 1\ner 2 phone\n"),
        ("keep.md", "keep\n"),
        ("local_edit.md", "le laptop\n"),
        ("merge.md", "m 1 phone\nm 2\nm 3\nm 4\nm 5 laptop\n"),
        ("moved/local_renamed.md", "lr\n"),
        ("new_local.md", "nl laptop\n"),
        ("new_remote.md", "nr phone\n"),
        ("sub/renamed.md", "rr\n"),
        ("sub/renedit.md", "re 1\nre 2 phone\n"),
        ("sub/ren_then_edit.md", "rte 1\nrte 2 phone\n"),
        ("sub/rrle.md", "rrle 1 laptop\nrrle 2\n"),
        ("moved/rel.md", "rel 1\nrel 2 phone\n"),
    ];
    let want: Vec<(&str, &str)> = want_all.iter().copied().filter(|w| s.rel || w.0 != "moved/rel.md").collect();
    let want = &want[..];
    let mut problems = Vec::new();
    for (p, c) in want {
        match fa.iter().find(|f| f.0 == *p) {
            Some(f) if f.1 == *c => {}
            Some(f) => problems.push(format!("{p} has {:?}, want {:?}", f.1, c)),
            None => problems.push(format!("{p} missing")),
        }
    }
    let conflict = fa.iter().find(|f| f.0 == "conflict.md").map(|f| f.1.clone());
    let copies = conflict_copies(&fa);
    match conflict.as_deref() {
        Some(CONFLICT_LAPTOP) | Some(CONFLICT_PHONE) => {}
        other => problems.push(format!("conflict.md is {other:?}")),
    }
    if copies.len() != 1 {
        problems.push(format!("expected exactly 1 conflict copy, got {copies:?}"));
    } else {
        let other = fa.iter().find(|f| f.0 == copies[0]).unwrap().1.clone();
        let mut both = vec![conflict.clone().unwrap_or_default(), other];
        both.sort();
        if both != vec![CONFLICT_LAPTOP.to_string(), CONFLICT_PHONE.to_string()] {
            problems.push(format!("conflict pair wrong: {both:?}"));
        }
    }
    if fa.len() != want.len() + 2 {
        let extra: Vec<&String> =
            fa.iter().map(|f| &f.0).filter(|p| !want.iter().any(|w| w.0 == p.as_str()) && *p != "conflict.md" && !copies.contains(p)).collect();
        problems.push(format!("{} files instead of {}; unexpected: {extra:?}; copies {copies:?}", fa.len(), want.len() + 2));
    }
    for gone in [
        "rename_remote.md",
        "renedit_remote.md",
        "delete_remote.md",
        "local_rename.md",
        "local_delete.md",
        "ren_then_edit.md",
        "remote_ren_local_edit.md",
        "remote_edit_local_ren.md",
    ] {
        if fa.iter().any(|f| f.0 == gone) {
            problems.push(format!("{gone} came back"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

#[test]
fn baseline_scenario_without_faults_is_clean() {
    let mut s = setup();
    check(&mut s).unwrap();
    let mut s = setup_opts(true);
    check(&mut s).unwrap();
}

// ------------------------------------------------------------------ crash sweeps

/// Crash A's sync right before / right after its k-th vault mutation (each
/// write, rename or delete the engine makes), restart from disk, sync on.
/// `rel` adds the local rename against a remote edit.
fn fs_crash_sweep(after: bool, rel: bool) -> Vec<String> {
    quiet_simulated_crashes();
    let mut failures = Vec::new();
    for k in 0.. {
        let mut s = setup_opts(rel);
        let hit = Arc::new(parking_lot::Mutex::new(None::<String>));
        let hit2 = hit.clone();
        s.a.hookfs.mutations.store(0, Ordering::SeqCst);
        *s.a.hookfs.decide.lock() = Some(Box::new(move |op, path, n| {
            if n == k {
                *hit2.lock() = Some(format!("{op:?} {path}"));
                if after {
                    FsAction::PanicAfter
                } else {
                    FsAction::PanicBefore
                }
            } else {
                FsAction::Pass
            }
        }));
        let r = s.a.sync_may_crash();
        let what = hit.lock().clone();
        if std::env::var("SR_VERBOSE").is_ok() {
            eprintln!("fs sweep after={after} k={k}: {what:?} crashed={}", r.is_none());
        }
        if r.is_some() && what.is_none() {
            break; // k is past the last mutation: done
        }
        if r.is_none() {
            s.a.restart();
        }
        if let Err(e) = check(&mut s) {
            failures.push(format!("crash {} mutation #{k} ({}): {e}", if after { "after" } else { "before" }, what.unwrap_or_default()));
        }
    }
    failures
}

#[test]
fn crash_before_each_vault_mutation_then_restart() {
    let f = fs_crash_sweep(false, false);
    assert!(f.is_empty(), "{} failing crash points:\n{}", f.len(), f.join("\n"));
}

#[test]
fn crash_after_each_vault_mutation_then_restart() {
    let f = fs_crash_sweep(true, false);
    assert!(f.is_empty(), "{} failing crash points:\n{}", f.len(), f.join("\n"));
}

#[test]
fn crash_at_each_vault_mutation_with_local_rename_vs_remote_edit() {
    let mut f = fs_crash_sweep(false, true);
    f.extend(fs_crash_sweep(true, true));
    assert!(f.is_empty(), "{} failing crash points:\n{}", f.len(), f.join("\n"));
}

/// Crash after the server processed A's k-th request (for a put: the server
/// stored the revision but A never recorded it), restart, sync on.
fn transport_sweep(fault: Fault, restart: bool) -> Vec<String> {
    quiet_simulated_crashes();
    let mut failures = Vec::new();
    for k in 0.. {
        let mut s = setup();
        let hit = Arc::new(parking_lot::Mutex::new(None::<String>));
        let hit2 = hit.clone();
        let t = FaultTransport::new(
            http(&s.a.url),
            Box::new(move |op, n| {
                if n == k {
                    *hit2.lock() = Some(op.to_string());
                    fault
                } else {
                    Fault::None
                }
            }),
        );
        s.a.set_transport(Box::new(t));
        let r = s.a.sync_may_crash();
        let what = hit.lock().clone();
        if std::env::var("SR_VERBOSE").is_ok() {
            eprintln!("transport sweep {fault:?} k={k}: {what:?} result={:?}", r.as_ref().map(|r| r.as_ref().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string())));
        }
        if what.is_none() {
            break;
        }
        if r.is_none() || restart {
            s.a.restart();
        } else {
            // keep the same engine, plain transport
            let e = s.a.engine.take();
            drop(e);
            s.a.set_transport(http(&s.a.url));
        }
        if let Err(e) = check(&mut s) {
            failures.push(format!("{fault:?} at call #{k} ({}): {e}", what.unwrap_or_default()));
        }
    }
    failures
}

#[test]
fn crash_after_server_accepted_each_request_then_restart() {
    let f = transport_sweep(Fault::PanicAfter, true);
    assert!(f.is_empty(), "{} failing crash points:\n{}", f.len(), f.join("\n"));
}

#[test]
fn crash_before_each_request_then_restart() {
    let f = transport_sweep(Fault::PanicBefore, true);
    assert!(f.is_empty(), "{} failing crash points:\n{}", f.len(), f.join("\n"));
}

#[test]
fn lost_response_on_each_request_then_retry() {
    let f = transport_sweep(Fault::ErrAfter, false);
    assert!(f.is_empty(), "{} failing points:\n{}", f.len(), f.join("\n"));
}

#[test]
fn network_error_on_each_request_then_retry() {
    let f = transport_sweep(Fault::ErrBefore, false);
    assert!(f.is_empty(), "{} failing points:\n{}", f.len(), f.join("\n"));
}

// ------------------------------------------------------------------ focused reproductions

/// Fail every put of A's next sync (the pull phase succeeds).
fn fail_puts(d: &mut Device) {
    let t = FaultTransport::new(http(&d.url), Box::new(|op, _| if op == "put" { Fault::ErrBefore } else { Fault::None }));
    d.set_transport(Box::new(t));
}

fn plain(d: &mut Device) {
    let url = d.url.clone();
    d.set_transport(http(&url));
}

#[test]
fn interrupted_sync_after_local_rename_vs_remote_edit_converges() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Draft.md", "line 1\nline 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("Draft.md", "line 1\nline 2 phone\n");
    b.sync_ok();
    a.mv("Draft.md", "Projects/Final.md");
    // A pulls B's edit into Projects/Final.md, then the network drops
    fail_puts(&mut a);
    assert!(a.sync().is_err());
    plain(&mut a);
    for _ in 0..3 {
        a.sync_ok();
        b.sync_ok();
    }
    assert_eq!(a.paths(), vec!["Projects/Final.md"]);
    assert_eq!(b.paths(), vec!["Projects/Final.md"], "B never learns about A's rename (A: {:?})", a.paths());
}

#[test]
fn interrupted_sync_after_same_path_created_twice_converges() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop notes\n")]);
    let mut b = Device::new(&srv, "phone", &[("Today.md", "phone notes\n")]);
    a.sync_ok();
    // B pulls A's Today.md, gives it a conflict name, and must tell the
    // server about that rename; the push never happens
    fail_puts(&mut b);
    assert!(b.sync().is_err());
    plain(&mut b);
    for _ in 0..3 {
        b.sync_ok();
        a.sync_ok();
    }
    assert_eq!(a.files(), b.files(), "devices disagree on names forever");
}

#[test]
fn crash_sweep_with_local_rename_vs_remote_edit() {
    // the full scenario including the rename/edit pair: after every
    // interruption in the push phase, A and B converge on the same files
    let mut failures = Vec::new();
    for fault in [Fault::ErrBefore, Fault::ErrAfter, Fault::PanicBefore, Fault::PanicAfter] {
        for k in 1..4 {
            let mut s = setup_opts(true);
            let t = FaultTransport::new(http(&s.a.url), Box::new(move |_, n| if n == k { fault } else { Fault::None }));
            s.a.set_transport(Box::new(t));
            quiet_simulated_crashes();
            if s.a.sync_may_crash().is_none() {
                s.a.restart();
            } else {
                plain(&mut s.a);
            }
            if let Err(e) = check(&mut s) {
                failures.push(format!("{fault:?} at call #{k}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Crash A at the first vault mutation matching `pred`, right after it hit
/// the disk; restart A from disk.
fn crash_after_first(d: &mut Device, pred: impl Fn(FsOp, &str) -> bool + Send + 'static) {
    quiet_simulated_crashes();
    let mut armed = true;
    *d.hookfs.decide.lock() = Some(Box::new(move |op, path, _| {
        if armed && pred(op, path) {
            armed = false;
            FsAction::PanicAfter
        } else {
            FsAction::Pass
        }
    }));
    assert!(d.sync_may_crash().is_none(), "expected a simulated crash");
    d.restart();
}

#[test]
fn crash_after_pulled_rename_and_edit_keeps_the_note_name() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Inbox/Idea.md", "idea 1\nidea 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.mv("Inbox/Idea.md", "Projects/Idea.md");
    b.sync_ok();
    b.write("Projects/Idea.md", "idea 1\nidea 2\nidea 3 phone\n");
    b.sync_ok();
    // A pulls one head (new path + new content): rename, then write; the
    // app is killed right after the write, before state.json is saved
    crash_after_first(&mut a, |op, p| op == FsOp::Write && p == "Projects/Idea.md");
    converge(&mut a, &mut b);
    assert_eq!(
        a.files(),
        vec![("Projects/Idea.md".to_string(), "idea 1\nidea 2\nidea 3 phone\n".to_string())],
        "after one crash the note is no longer where the user put it"
    );
}

#[test]
fn crash_after_pulled_edit_into_locally_renamed_note_does_not_duplicate() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Draft.md", "line 1\nline 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("Draft.md", "line 1\nline 2 phone\n");
    b.sync_ok();
    a.mv("Draft.md", "Final.md");
    crash_after_first(&mut a, |op, p| op == FsOp::Write && p == "Final.md");
    converge(&mut a, &mut b);
    assert_eq!(a.files(), vec![("Final.md".to_string(), "line 1\nline 2 phone\n".to_string())]);
}

#[test]
fn crash_after_conflict_copy_does_not_duplicate_it() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "same\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("n.md", "phone\n");
    b.sync_ok();
    a.write("n.md", "laptop\n");
    crash_after_first(&mut a, |op, p| op == FsOp::Write && p.contains("(conflict "));
    converge(&mut a, &mut b);
    let files = a.files();
    assert_eq!(conflict_copies(&files).len(), 1, "{files:?}");
}

#[test]
fn crash_after_conflict_name_for_a_new_remote_file_does_not_duplicate_it() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop notes\n")]);
    let mut b = Device::new(&srv, "phone", &[("Today.md", "phone notes\n")]);
    b.sync_ok();
    // A writes the phone's Today.md under a conflict name; the app is
    // killed right after
    crash_after_first(&mut a, |op, p| op == FsOp::Write && p.contains("(conflict "));
    converge(&mut a, &mut b);
    // each note once (which of them keeps the name is FINDING-061)
    let files = a.files();
    let mut contents: Vec<&str> = files.iter().map(|f| f.1.as_str()).collect();
    contents.sort();
    assert_eq!(contents, ["laptop notes\n", "phone notes\n"], "{files:?}");
}

/// FINDING-063: the phone uploads its Today.md first, and the laptop its
/// own right after, without having seen the phone's. `fault` decides the
/// laptop's later calls.
fn race_to_create_today(fault: impl FnMut(&str, usize) -> Fault + Send + 'static) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop\n")]);
    let b = Arc::new(parking_lot::Mutex::new(Device::new(&srv, "phone", &[("Today.md", "phone\n")])));
    let t = Arc::new(FaultTransport::new(http(&a.url), Box::new(fault)));
    let b2 = b.clone();
    let mut puts = 0;
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "put" {
            puts += 1;
            if puts == 1 {
                b2.lock().sync_ok();
            }
        }
    }));
    a.set_transport(Box::new(SharedTransport(t.clone())));
    a.sync_ok();
    *t.before.lock() = None;
    let b = Arc::try_unwrap(b).ok().unwrap().into_inner();
    (srv, a, b)
}

#[test]
fn crash_after_moving_a_file_out_of_the_way_of_a_concurrent_one_keeps_its_edit() {
    let (_srv, mut a, mut b) = race_to_create_today(|_, _| Fault::None);
    plain(&mut a);
    // The phone's note, uploaded first and edited since, moves to a conflict
    // name for the laptop's, which is written in its place; the app is
    // killed right after that write
    b.write("Today.md", "phone\nmore\n");
    crash_after_first(&mut b, |op, p| op == FsOp::Write && p == "Today.md");
    converge(&mut a, &mut b);
    let files = a.files();
    assert_eq!(a.read("Today.md").as_deref(), Some("laptop\n"), "{files:?}");
    let copies = conflict_copies(&files);
    assert_eq!(copies.len(), 1, "{files:?}");
    assert_eq!(a.read(&copies[0]).as_deref(), Some("phone\nmore\n"));
    assert_eq!(files.len(), 2, "{files:?}");
    // still the same file, with its first version, not a new one
    let history = a.engine().history(&copies[0]).unwrap();
    assert!(history.len() >= 2, "{history:?}");
}

#[test]
fn concurrent_file_waits_when_the_upload_order_cannot_be_fetched() {
    // the laptop's next sync cannot fetch the files' history: the phone's
    // note waits for the sync after
    let (_srv, mut a, mut b) = race_to_create_today(|op, _| if op == "history" { Fault::ErrBefore } else { Fault::None });
    let r = a.sync_ok();
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert_eq!(a.files(), [("Today.md".to_string(), "laptop\n".to_string())]);
    plain(&mut a);
    converge(&mut a, &mut b);
    let files = a.files();
    assert_eq!(a.read("Today.md").as_deref(), Some("laptop\n"), "{files:?}");
    let copies = conflict_copies(&files);
    assert_eq!(copies.len(), 1, "{files:?}");
    assert_eq!(a.read(&copies[0]).as_deref(), Some("phone\n"));
}

#[test]
fn crash_after_pulled_rename_onto_a_taken_name_and_edit_does_not_duplicate() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("x.md", "x 1\nx 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.mv("x.md", "y.md");
    b.write("y.md", "x 1\nx 2 phone\n");
    b.sync_ok();
    // A has its own y.md: the phone's note is moved to a conflict name and
    // then gets the phone's edit; the app is killed right after the write
    a.write("y.md", "laptop y\n");
    crash_after_first(&mut a, |op, p| op == FsOp::Write && p.contains("(conflict "));
    converge(&mut a, &mut b);
    // each note once (which of them keeps the name is FINDING-061)
    let files = a.files();
    let mut contents: Vec<&str> = files.iter().map(|f| f.1.as_str()).collect();
    contents.sort();
    assert_eq!(contents, ["laptop y\n", "x 1\nx 2 phone\n"], "{files:?}");
}

#[test]
fn crash_after_pulled_rename_and_edit_keeps_a_conflict_copy_with_the_old_content() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "a\nkeep me\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    // the phone's new q.md has n.md's content; n.md is renamed, then edited
    b.write("q.md", "a\nkeep me\n");
    b.sync_ok();
    b.mv("n.md", "m.md");
    b.sync_ok();
    b.write("m.md", "a\nphone\n");
    b.sync_ok();
    // A puts the phone's q.md under a conflict name (its own q.md is there),
    // then moves n.md to m.md and writes the edit; the app is killed right
    // after. The next scan takes the conflict copy for n.md renamed here.
    a.write("q.md", "laptop q\n");
    crash_after_first(&mut a, |op, p| op == FsOp::Write && p == "m.md");
    converge(&mut a, &mut b);
    for d in [&a, &b] {
        let text = d.all_text();
        for want in ["a\nkeep me\n", "a\nphone\n", "laptop q\n"] {
            assert!(text.contains(want), "{}: {want:?} lost: {:?}, trash {:?}", d.name, d.files(), d.trash());
        }
    }
}

#[test]
fn remote_delete_of_a_note_waiting_to_upload_its_conflict_name_is_applied() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop notes\n"), ("anchor.md", "anchor\n")]);
    let mut b = Device::new(&srv, "phone", &[("Today.md", "phone notes\n")]);
    a.sync_ok();
    // B puts A's Today.md under a conflict name; that name is not uploaded
    fail_puts(&mut b);
    assert!(b.sync().is_err());
    plain(&mut b);
    a.rm("Today.md");
    a.sync_ok();
    for _ in 0..3 {
        b.sync_ok();
        a.sync_ok();
    }
    assert_eq!(a.files(), b.files());
    let files = a.files();
    assert!(!files.iter().any(|f| f.1 == "laptop notes\n"), "the note deleted on the laptop came back: {files:?}");
    assert!(b.trash().iter().any(|f| f.1 == "laptop notes\n"), "not in the phone's trash: {:?}", b.trash());
    assert!(files.iter().any(|f| f.1 == "phone notes\n"), "{files:?}");
}

/// FINDING-148: a note renamed here and deleted on another device is kept
/// under its file id, and the rename is uploaded on top of the delete. When
/// that upload fails, the next sync after a restart still makes it.
#[test]
fn rename_kept_against_a_remote_delete_is_uploaded_after_an_interrupted_sync() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "keep me\n"), ("anchor.md", "anchor\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    let id = |d: &mut Device, p: &str| d.engine().state().files.iter().find(|(_, t)| !t.deleted && t.path == p).map(|(f, _)| f.clone());
    let fid = id(&mut a, "a.md").unwrap();
    a.mv("a.md", "renamed.md");
    b.rm("a.md");
    b.sync_ok();
    fail_puts(&mut a);
    assert!(a.sync().is_err());
    a.restart();
    for _ in 0..3 {
        a.sync_ok();
        b.sync_ok();
    }
    let want = vec![("anchor.md".to_string(), "anchor\n".to_string()), ("renamed.md".to_string(), "keep me\n".to_string())];
    assert_eq!(a.files(), want);
    assert_eq!(b.files(), want);
    assert_eq!(id(&mut a, "renamed.md").as_ref(), Some(&fid));
    assert_eq!(id(&mut b, "renamed.md").as_ref(), Some(&fid));
    // the original upload, the delete and the rename, on one file
    let h = b.engine().history("renamed.md").unwrap();
    assert_eq!(h.len(), 3, "{h:?}");
    assert!(h[1].deleted && !h[0].deleted, "{h:?}");
}

/// The same with a third device: before the laptop's upload of the kept
/// rename gets through, the tablet's edit brings the note back (an edit
/// beats a delete) and the phone deletes it again. The rename still wins:
/// the second delete is not applied to the renamed note, which the sync
/// would do to a name it chose itself (a conflict copy name).
#[test]
fn rename_kept_against_a_remote_delete_survives_a_second_delete() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "keep me\n"), ("anchor.md", "anchor\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    let mut c = Device::new(&srv, "tablet", &[]);
    c.sync_ok();
    a.mv("a.md", "renamed.md");
    b.rm("a.md");
    b.sync_ok();
    fail_puts(&mut a);
    assert!(a.sync().is_err());
    a.restart();
    c.write("a.md", "keep me\nedited on the tablet\n");
    c.sync_ok();
    b.sync_ok();
    assert_eq!(b.read("a.md").as_deref(), Some("keep me\nedited on the tablet\n"));
    b.rm("a.md");
    b.sync_ok();
    a.sync_ok();
    assert!(a.trash().is_empty(), "the renamed note went to the trash: {:?}", a.trash());
    for _ in 0..2 {
        b.sync_ok();
        c.sync_ok();
        a.sync_ok();
    }
    let want = vec![("anchor.md".to_string(), "anchor\n".to_string()), ("renamed.md".to_string(), "keep me\n".to_string())];
    for d in [&a, &b, &c] {
        assert_eq!(d.files(), want, "on {}", d.name);
    }
}

/// A rename made here whose upload fails after an edit from another device
/// was written into the renamed note: the rename still waits for upload, as
/// the user's. When the note is then deleted on that device and the upload
/// fails again, the rename still wins (FINDING-148): the renamed note keeps
/// the edit and its file id, and is not trashed as a name the sync chose
/// would be. Renamed outside the app (the scan finds it), in the app, and in
/// the app with the same edit made here too.
#[test]
fn rename_merged_with_a_remote_edit_survives_a_later_delete() {
    for case in ["scan", "app", "app, same edit"] {
        let srv = server();
        let mut a = Device::new(&srv, "laptop", &[("a.md", "keep me\n"), ("anchor.md", "anchor\n")]);
        a.sync_ok();
        let mut b = Device::new(&srv, "phone", &[]);
        b.sync_ok();
        let id = |d: &mut Device, p: &str| d.engine().state().files.iter().find(|(_, t)| !t.deleted && t.path == p).map(|(f, _)| f.clone());
        let fid = id(&mut a, "a.md").unwrap();
        if case == "scan" {
            a.mv("a.md", "renamed.md");
        } else {
            a.app_mv("a.md", "renamed.md");
        }
        if case == "app, same edit" {
            a.write("renamed.md", "keep me\nedited on the phone\n");
        }
        b.write("a.md", "keep me\nedited on the phone\n");
        b.sync_ok();
        fail_puts(&mut a);
        assert!(a.sync().is_err());
        assert_eq!(a.read("renamed.md").as_deref(), Some("keep me\nedited on the phone\n"), "{case}");
        b.rm("a.md");
        b.sync_ok();
        let r = a.sync();
        assert!(a.trash().is_empty(), "{case}: the renamed note went to the trash: {:?}", a.trash());
        assert!(r.is_err(), "{case}: nothing was left to upload");
        a.restart();
        for _ in 0..2 {
            a.sync_ok();
            b.sync_ok();
        }
        let want = vec![
            ("anchor.md".to_string(), "anchor\n".to_string()),
            ("renamed.md".to_string(), "keep me\nedited on the phone\n".to_string()),
        ];
        assert_eq!(a.files(), want, "{case}");
        assert_eq!(b.files(), want, "{case}");
        assert_eq!(id(&mut a, "renamed.md").as_ref(), Some(&fid), "{case}");
        assert_eq!(id(&mut b, "renamed.md").as_ref(), Some(&fid), "{case}");
    }
}

// ------------------------------------------------------------------ uploads recorded in state.journal

/// A device with six new notes whose first sync is killed right before its
/// fourth upload: three uploads are recorded only in state.journal.
fn killed_before_the_fourth_upload() -> (Server, Device) {
    let srv = server();
    let notes: Vec<(String, String)> = (0..6).map(|i| (format!("n{i}.md"), format!("note {i}\n"))).collect();
    let notes: Vec<(&str, &str)> = notes.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let mut a = Device::new(&srv, "laptop", &notes);
    let mut puts = 0;
    let t = FaultTransport::new(
        http(&a.url),
        Box::new(move |op, _| {
            puts += (op == "put") as usize;
            if puts == 4 { Fault::PanicBefore } else { Fault::None }
        }),
    );
    a.set_transport(Box::new(t));
    quiet_simulated_crashes();
    assert!(a.sync_may_crash().is_none(), "no crash");
    assert!(a.state_dir.join("state.journal").exists());
    (srv, a)
}

/// Uploads are recorded in state.journal rather than by writing the whole
/// state each time (FINDING-126): after a crash, the restarted device knows
/// the files it uploaded, so it neither uploads them again nor downloads
/// them.
#[test]
fn crash_during_push_keeps_the_uploads_recorded_so_far() {
    let (srv, mut a) = killed_before_the_fourth_upload();
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    a.restart_with(Box::new(SharedTransport(t.clone())));
    let r = a.sync_ok();
    assert_eq!(r.pushed, 3, "uploaded again");
    assert_eq!(t.blob_bytes_in.load(Ordering::SeqCst), 0, "downloaded its own uploads");
    assert!(!a.state_dir.join("state.journal").exists(), "the journal stays after a full save");
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    assert_eq!(a.paths().len(), 6);
    assert_eq!(b.files(), a.files());
}

/// A journal whose last line was cut off by the crash: the lines before it
/// count, and the upload whose line was lost is taken back from the server
/// rather than uploaded again.
#[test]
fn journal_cut_off_in_a_line_keeps_the_lines_before() {
    let (srv, mut a) = killed_before_the_fourth_upload();
    let journal = a.state_dir.join("state.journal");
    let bytes = fs::read(&journal).unwrap();
    fs::write(&journal, &bytes[..bytes.len() - 10]).unwrap();
    a.restart();
    let mut tracked: Vec<String> = a.engine().state().files.values().map(|t| t.path.clone()).collect();
    tracked.sort();
    assert_eq!(tracked, ["n0.md", "n1.md"], "the uploads that the journal has");
    let r = a.sync_ok();
    assert_eq!(r.pushed, 3);
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    assert_eq!(b.paths(), ["n0.md", "n1.md", "n2.md", "n3.md", "n4.md", "n5.md"]);
    assert_eq!(b.files(), a.files());
}

/// A journal from before state.json was last written (the app was killed
/// after writing state.json, before removing the journal) is not read on
/// top of it: its entries are older than the state's.
#[test]
fn journal_from_before_the_last_state_save_is_ignored() {
    let (srv, mut a) = killed_before_the_fourth_upload();
    let journal = a.state_dir.join("state.journal");
    let old = fs::read(&journal).unwrap();
    a.restart();
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("n0.md", "note 0 phone\n");
    b.sync_ok();
    a.sync_ok();
    let saved = serde_json::to_value(a.engine().state()).unwrap();
    fs::write(&journal, old).unwrap();
    a.restart();
    assert_eq!(serde_json::to_value(a.engine().state()).unwrap(), saved);
    let r = a.sync_ok();
    assert_eq!((r.pulled, r.pushed, r.conflicts.len()), (0, 0, 0));
    assert_eq!(a.files(), b.files());
}

// ------------------------------------------------------------------ lost or damaged local state

fn synced_pair(files: &[(&str, &str)]) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", files);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    (srv, a, b)
}

const THREE: &[(&str, &str)] = &[("a.md", "alpha\n"), ("dir/b.md", "beta\n"), ("pic.png", "PNGDATA")];

#[test]
fn deleted_state_json_without_local_changes_adopts_everything() {
    let (_srv, mut a, mut b) = synced_pair(THREE);
    let before = a.files();
    fs::remove_file(a.state_file()).unwrap();
    a.restart();
    let r = a.sync_ok();
    assert_eq!(r.pushed, 0, "nothing should be re-uploaded: {r:?}");
    assert!(r.conflicts.is_empty(), "{r:?}");
    converge(&mut a, &mut b);
    assert_eq!(a.files(), before);
}

#[test]
fn garbage_and_truncated_state_json_behave_like_a_missing_one() {
    for damage in [&b"\x00\x01garbage"[..], &b"{\"last_seq\": 3, \"files\": {"[..], &b""[..]] {
        let (_srv, mut a, mut b) = synced_pair(THREE);
        let before = a.files();
        fs::write(a.state_file(), damage).unwrap();
        a.restart();
        let r = a.sync_ok();
        assert!(r.conflicts.is_empty(), "{r:?}");
        converge(&mut a, &mut b);
        assert_eq!(a.files(), before);
    }
}

#[test]
#[ignore = "FINDING-054 (deferred: needs an automatic delete, which sync does not make by design): after a lost state, deleted notes come back (renames are not duplicated)"]
fn lost_state_does_not_resurrect_deleted_files_or_duplicate_renames() {
    let (_srv, mut a, mut b) = synced_pair(&[("keep.md", "k\n"), ("gone_remote.md", "r\n"), ("gone_local.md", "l\n"), ("old_name.md", "o\n")]);
    // B deletes a note and syncs
    b.rm("gone_remote.md");
    b.sync_ok();
    // A deletes and renames a note, then its state file is damaged (crash
    // while saving, disk full, restore from backup without app data...)
    a.rm("gone_local.md");
    a.mv("old_name.md", "new_name.md");
    fs::write(a.state_file(), b"{\"last_seq\": 9, \"fil").unwrap();
    a.restart();
    a.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(a.paths(), vec!["keep.md", "new_name.md"], "deleted notes came back / rename duplicated");
}

#[test]
fn stale_state_json_from_a_backup_does_not_create_duplicates() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "one\n"), ("r.md", "rename me\n"), ("d.md", "delete me\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    let old_state = fs::read(a.state_file()).unwrap();
    // the vault moves on: renames with edits, deletes, new notes
    b.mv("r.md", "sub/r.md");
    b.sync_ok();
    b.write("sub/r.md", "rename me\nand edit\n");
    b.rm("d.md");
    b.write("new.md", "new\n");
    b.sync_ok();
    a.sync_ok();
    let want = a.files();
    // the app data folder is restored from an older backup
    fs::write(a.state_file(), old_state).unwrap();
    a.restart();
    converge(&mut a, &mut b);
    assert_eq!(a.files(), want, "an old state.json must not change the vault");
}

#[test]
fn lost_state_with_a_local_edit_keeps_both_versions() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n")]);
    a.write("n.md", "v2 laptop\n");
    fs::remove_file(a.state_file()).unwrap();
    a.restart();
    a.sync_ok();
    converge(&mut a, &mut b);
    let all = a.all_text();
    assert!(all.contains("v2 laptop") && all.contains("v1"), "{:?}", a.files());
}

/// A note renamed while the state was lost: A with the rename found by
/// content, B as before.
fn lost_state_with_a_local_rename() -> (Server, Device, Device) {
    let (srv, mut a, b) = synced_pair(&[("keep.md", "k\n"), ("old_name.md", "o\n")]);
    a.mv("old_name.md", "new_name.md");
    fs::remove_file(a.state_file()).unwrap();
    a.restart();
    (srv, a, b)
}

/// The rename found after a lost state is kept until it is uploaded, also
/// when the sync stops before that (FINDING-054).
#[test]
fn lost_state_rename_survives_a_failed_or_crashed_upload() {
    for fault in [Fault::ErrBefore, Fault::PanicBefore] {
        let (_srv, mut a, mut b) = lost_state_with_a_local_rename();
        let mut armed = true;
        let t = FaultTransport::new(
            http(&a.url),
            Box::new(move |op, _| {
                if armed && op == "put" {
                    armed = false;
                    fault
                } else {
                    Fault::None
                }
            }),
        );
        a.set_transport(Box::new(t));
        quiet_simulated_crashes();
        match a.sync_may_crash() {
            None => a.restart(),
            Some(r) => {
                assert!(r.is_err(), "{fault:?}: the upload did not fail");
                plain(&mut a);
            }
        }
        converge(&mut a, &mut b);
        assert_eq!(b.paths(), ["keep.md", "new_name.md"], "{fault:?}: the rename was not uploaded");
    }
}

/// The upload of another note is refused (it changed on the server
/// meanwhile), so the sync goes round again: the rename still goes up.
#[test]
fn lost_state_rename_survives_a_second_round() {
    let (_srv, mut a, b) = lost_state_with_a_local_rename();
    let b = Arc::new(parking_lot::Mutex::new(b));
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    let (root, b2) = (a.root.clone(), b.clone());
    let mut puts = 0;
    *t.before.lock() = Some(Box::new(move |op, _| match op {
        // edited here after the scan, so keep.md is uploaded first
        "changes" => fs::write(root.join("keep.md"), "k laptop\n").unwrap(),
        "put" => {
            puts += 1;
            if puts == 1 {
                let mut b = b2.lock();
                b.write("keep.md", "k phone\n");
                b.sync_ok();
            }
        }
        _ => {}
    }));
    a.set_transport(Box::new(SharedTransport(t)));
    a.sync_ok();
    plain(&mut a);
    let mut b = b.lock();
    converge(&mut a, &mut b);
    assert!(b.paths().contains(&"new_name.md".to_string()) && !b.paths().contains(&"old_name.md".to_string()), "{:?}", b.paths());
    let all = b.all_text();
    assert!(all.contains("k laptop") && all.contains("k phone"), "{:?}", b.files());
}

/// The note is edited on the other device before the rename goes up: the
/// rename is refused, and the edit is applied to the renamed note.
#[test]
fn lost_state_rename_survives_a_remote_edit_of_the_note() {
    let (_srv, mut a, b) = lost_state_with_a_local_rename();
    let b = Arc::new(parking_lot::Mutex::new(b));
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    let b2 = b.clone();
    let mut puts = 0;
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "put" {
            puts += 1;
            if puts == 1 {
                let mut b = b2.lock();
                b.write("old_name.md", "o phone\n");
                b.sync_ok();
            }
        }
    }));
    a.set_transport(Box::new(SharedTransport(t)));
    a.sync_ok();
    plain(&mut a);
    let mut b = b.lock();
    converge(&mut a, &mut b);
    assert_eq!(b.files(), [("keep.md".to_string(), "k\n".to_string()), ("new_name.md".to_string(), "o phone\n".to_string())]);
}

#[test]
fn deleted_key_file_is_reported_not_silently_reset() {
    let (_srv, a, _b) = synced_pair(THREE);
    fs::remove_file(a.state_dir.join("key")).unwrap();
    let r = cairn_sync::engine::SyncEngine::load_with(a.vault.clone(), &a.state_dir, settings(&a.url, "laptop"), http(&a.url));
    assert!(matches!(r, Err(cairn_sync::SyncError::NotConfigured)), "{:?}", r.err());
    fs::write(a.state_dir.join("key"), "!!!").unwrap();
    let r = cairn_sync::engine::SyncEngine::load_with(a.vault.clone(), &a.state_dir, settings(&a.url, "laptop"), http(&a.url));
    assert!(r.is_err());
}

// ------------------------------------------------------------------ the vault folder vanishes

/// FINDING-006: a moved SAF folder lists as empty. The sync must not take
/// that for the user deleting every note, on every device.
#[test]
fn a_vault_folder_that_lists_empty_pushes_nothing() {
    let (_srv, mut a, mut b) = synced_pair(THREE);
    let want = b.files();
    for p in a.paths() {
        a.rm(&p);
    }
    let err = a.sync().expect_err("the sync of an emptied vault folder went through").to_string();
    assert!(err.contains("looks empty or missing") && err.contains("nothing was synced"), "{err}");
    b.sync_ok();
    assert_eq!(b.files(), want, "the other device lost its notes");
    // the folder is back: the sync goes on
    for (p, c) in THREE {
        a.write(p, c);
    }
    converge(&mut a, &mut b);
    assert_eq!(b.files(), want);
}

/// The same on the desktop: a vault folder that was renamed away (or a
/// drive that is not mounted) lists as empty too.
#[test]
fn a_moved_vault_folder_pushes_nothing() {
    let (_srv, mut a, mut b) = synced_pair(THREE);
    let want = b.files();
    let moved = a.root.with_extension("moved");
    fs::rename(&a.root, &moved).unwrap();
    let err = a.sync().expect_err("the sync of a moved vault folder went through").to_string();
    assert!(err.contains("looks empty or missing"), "{err}");
    b.sync_ok();
    assert_eq!(b.files(), want, "the other device lost its notes");
    fs::rename(&moved, &a.root).unwrap();
    converge(&mut a, &mut b);
    assert_eq!(b.files(), want);
}

/// Deleting every note on purpose: the error says how to go on, and the
/// deletions go up once there is a note again.
#[test]
fn deleting_every_note_syncs_once_a_note_is_added() {
    let (_srv, mut a, mut b) = synced_pair(THREE);
    for p in a.paths() {
        a.rm(&p);
    }
    let err = a.sync().unwrap_err().to_string();
    assert!(err.contains("add a note and sync again"), "{err}");
    a.write("new.md", "fresh start\n");
    converge(&mut a, &mut b);
    assert_eq!(b.files(), [("new.md".to_string(), "fresh start\n".to_string())]);
}

/// Moving every note elsewhere is not an empty folder: the renames go up.
#[test]
fn moving_every_note_into_a_folder_still_syncs() {
    let (_srv, mut a, mut b) = synced_pair(THREE);
    for p in a.paths() {
        a.mv(&p, &format!("old/{p}"));
    }
    a.sync_ok();
    b.sync_ok();
    assert_eq!(b.paths(), ["old/a.md", "old/dir/b.md", "old/pic.png"]);
}

// ------------------------------------------------------------------ the user edits while a sync runs

/// Run `f` on A while A's sync is waiting for the changes feed (after A
/// scanned the vault, before it applies anything).
fn during_pull(d: &mut Device, f: impl FnOnce(&Device) + Send + 'static) {
    let root = d.root.clone();
    let vault = d.vault.clone();
    let name = d.name.clone();
    let t = Arc::new(FaultTransport::passthrough(http(&d.url)));
    let mut f = Some(f);
    // the closure needs a Device-like view: give it a throwaway handle
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "changes"
            && let Some(f) = f.take()
        {
            let view = DeviceView { root: root.clone(), vault: vault.clone(), name: name.clone() };
            f(&view.as_device_stub());
        }
    }));
    d.set_transport(Box::new(SharedTransport(t)));
}

struct DeviceView {
    root: std::path::PathBuf,
    vault: Arc<cairn_core::Vault>,
    name: String,
}

impl DeviceView {
    fn as_device_stub(&self) -> Device {
        // A Device whose engine is never used; only root/vault matter.
        Device::stub(&self.root, self.vault.clone(), &self.name)
    }
}

#[test]
fn note_created_by_user_during_pull_is_not_overwritten() {
    let (_srv, mut a, mut b) = synced_pair(&[("x.md", "x\n")]);
    b.write("Meeting.md", "phone agenda\n");
    b.sync_ok();
    // While A waits for the server, the user saves a new note with the same
    // name in another editor (or A's own UI writes it to disk).
    during_pull(&mut a, |d| d.write("Meeting.md", "laptop minutes - only copy\n"));
    a.sync_ok();
    plain(&mut a);
    converge(&mut a, &mut b);
    let everything = format!("{}{}", a.all_text(), b.all_text());
    assert!(everything.contains("laptop minutes - only copy"), "the user's note is gone: A={:?} trash={:?}", a.files(), a.trash());
}

#[test]
fn edit_during_pull_of_a_remote_delete_wins() {
    let (_srv, mut a, mut b) = synced_pair(&[("todo.md", "buy milk\n"), ("anchor.md", "untouched\n")]);
    b.rm("todo.md");
    b.sync_ok();
    during_pull(&mut a, |d| d.write("todo.md", "buy milk\nand call mom\n"));
    a.sync_ok();
    plain(&mut a);
    converge(&mut a, &mut b);
    assert_eq!(a.read("todo.md").as_deref(), Some("buy milk\nand call mom\n"), "A: {:?}, A trash: {:?}", a.files(), a.trash());
}

#[test]
fn edit_during_pull_of_a_remote_edit_is_merged() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "1\n2\n3\n4\n5\n")]);
    b.write("n.md", "1 phone\n2\n3\n4\n5\n");
    b.sync_ok();
    during_pull(&mut a, |d| d.write("n.md", "1\n2\n3\n4\n5 laptop\n"));
    a.sync_ok();
    plain(&mut a);
    converge(&mut a, &mut b);
    assert_eq!(a.read("n.md").as_deref(), Some("1 phone\n2\n3\n4\n5 laptop\n"));
    assert!(conflict_copies(&a.files()).is_empty());
}

#[test]
fn edit_during_pull_through_the_vault_api_gets_a_conflict_copy_not_overwritten() {
    // The app writes through Vault (the index knows about the file), so the
    // incoming note gets a conflict name instead.
    let (_srv, mut a, mut b) = synced_pair(&[("x.md", "x\n")]);
    b.write("Meeting.md", "phone agenda\n");
    b.sync_ok();
    during_pull(&mut a, |d| {
        d.vault.create_note("Meeting.md", "laptop minutes\n").unwrap();
    });
    a.sync_ok();
    plain(&mut a);
    converge(&mut a, &mut b);
    let all = a.all_text();
    assert!(all.contains("laptop minutes") && all.contains("phone agenda"), "{:?}", a.files());
}

#[test]
fn save_between_push_read_and_stat_is_still_uploaded() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n")]);
    a.write("n.md", "v2\n");
    // Right after the push's own read of n.md returns (a read made directly
    // by SyncEngine::round, not by scan), the editor autosaves v3 through the
    // vault, as the app does.
    let vault = a.vault.clone();
    let mut fired = false;
    *a.hookfs.after_read.lock() = Some(Box::new(move |p| {
        if p == "n.md" && !fired {
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            let push_read = bt.contains("SyncEngine>::round") && !bt.contains("SyncEngine>::scan") && !bt.contains("apply_remote");
            if push_read {
                fired = true;
                vault.write_file("n.md", b"v3 typed during sync\n", None).unwrap();
            }
        }
    }));
    a.sync_ok();
    *a.hookfs.after_read.lock() = None;
    for _ in 0..3 {
        a.sync_ok();
        b.sync_ok();
    }
    assert_eq!(a.read("n.md").as_deref(), Some("v3 typed during sync\n"));
    assert_eq!(b.read("n.md").as_deref(), Some("v3 typed during sync\n"), "the edit never left the laptop");
}

#[test]
fn save_between_push_read_and_stat_then_remote_edit_does_not_block_sync() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n"), ("other.md", "o\n")]);
    a.write("n.md", "v2\n");
    let vault = a.vault.clone();
    let mut fired = false;
    *a.hookfs.after_read.lock() = Some(Box::new(move |p| {
        if p == "n.md" && !fired {
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            if bt.contains("SyncEngine>::round") && !bt.contains("SyncEngine>::scan") && !bt.contains("apply_remote") {
                fired = true;
                vault.write_file("n.md", b"v3 typed during sync\n", None).unwrap();
            }
        }
    }));
    a.sync_ok();
    *a.hookfs.after_read.lock() = None;
    b.sync_ok();
    // B edits the note and another one; A must still be able to sync
    b.write("n.md", "v2\nphone line\n");
    b.write("other.md", "o\nphone\n");
    b.sync_ok();
    let r = a.sync();
    assert!(r.is_ok(), "A's sync fails: {:?}; other.md on A = {:?}", r.err().map(|e| e.to_string()), a.read("other.md"));
}

#[test]
fn mtime_preserving_same_size_edit_is_uploaded_and_does_not_block_sync() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "teh cat\n"), ("other.md", "o\n")]);
    // a tool rewrites the file and restores its mtime (cp -p, rsync -t, a
    // backup restore, a sync tool...); same length
    let p = a.root.join("n.md");
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    fs::write(&p, "the cat\n").unwrap();
    fs::File::options().write(true).open(&p).unwrap().set_modified(mtime).unwrap();
    a.sync_ok();
    b.sync_ok();
    let uploaded = b.read("n.md");
    b.write("other.md", "o\nphone\n");
    b.write("n.md", "teh cat\nphone line\n");
    b.sync_ok();
    let r = a.sync();
    assert!(
        uploaded.as_deref() == Some("the cat\n") && r.is_ok(),
        "B got {uploaded:?}; then A's sync: {:?}; other.md on A = {:?}",
        r.err().map(|e| e.to_string()),
        a.read("other.md")
    );
}

// The tests above change notes right after they were written, which the
// scan never caches. These change notes last modified a day ago, whose
// hashes the scan does cache.

fn set_mtime(p: &std::path::Path, t: std::time::SystemTime) {
    fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
}

fn day_ago() -> std::time::SystemTime {
    std::time::SystemTime::now() - std::time::Duration::from_secs(86_400)
}

/// A synced pair whose notes on A were last modified at `t`, with their
/// hashes cached on A.
fn synced_old_pair(maker: Option<FsMaker>, files: &[(&str, &str)], t: std::time::SystemTime) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new_with_fs(&srv.url, "laptop", files, maker);
    for (n, _) in files {
        set_mtime(&a.root.join(n), t);
    }
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    let cached = a.engine().state().files.values().filter(|t| t.seen.is_some()).count();
    assert_eq!(cached, files.len(), "the scan caches the hashes of old notes");
    (srv, a, b)
}

#[cfg_attr(windows, ignore = "FINDING-055: StdFs has no change stamp on Windows")]
#[test]
fn old_note_edited_by_a_tool_that_keeps_size_and_mtime_is_uploaded() {
    let t = day_ago();
    let (_srv, mut a, mut b) = synced_old_pair(None, &[("n.md", "teh cat\n"), ("other.md", "o\n")], t);
    // rsync -t, cp -p, a backup restore: same size, mtime put back
    fs::write(a.root.join("n.md"), "the cat\n").unwrap();
    set_mtime(&a.root.join("n.md"), t);
    assert_eq!(a.sync_ok().pushed, 1);
    b.sync_ok();
    assert_eq!(b.read("n.md").as_deref(), Some("the cat\n"));
}

#[cfg_attr(windows, ignore = "FINDING-055: StdFs has no change stamp on Windows")]
#[test]
fn old_note_replaced_by_one_of_the_same_size_and_mtime_is_uploaded() {
    let t = day_ago();
    let (_srv, mut a, mut b) = synced_old_pair(None, &[("a.md", "content a\n"), ("b.md", "content b\n")], t);
    a.mv("b.md", "a.md");
    a.sync_ok();
    b.sync_ok();
    assert_eq!(a.files(), vec![("a.md".to_string(), "content b\n".to_string())]);
    assert_eq!(b.files(), a.files(), "the replacement never reached the other device");
}

/// StdFs without change stamps, as Android shared storage is.
struct NoStampFs(cairn_core::StdFs);

impl cairn_core::VaultFs for NoStampFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<cairn_core::FileStat>> {
        self.0.list(dir)
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<cairn_core::FileStat>> {
        self.0.stat(path)
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.0.read(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<cairn_core::FileStat> {
        self.0.write(path, data)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.0.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        self.0.rename(from, to)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.0.remove(path)
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.0.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        self.0.describe()
    }
}

#[test]
fn without_change_stamps_a_missed_edit_does_not_block_sync() {
    // Where the file system has no change stamps, an edit that keeps the
    // size and an old mtime is not seen. A remote edit of that note then
    // finds it changed on disk: the note is hashed again and merged,
    // instead of every sync failing.
    let maker: FsMaker = Arc::new(|root: &std::path::Path| {
        Arc::new(NoStampFs(cairn_core::StdFs::new(root, cairn_core::TrashMode::Vault).unwrap())) as Arc<dyn cairn_core::VaultFs>
    });
    let (_srv, mut a, mut b) = synced_old_pair(Some(maker), &[("n.md", "teh cat\nmiddle\nend\n"), ("other.md", "o\n")], day_ago());
    let mtime = fs::metadata(a.root.join("n.md")).unwrap().modified().unwrap();
    fs::write(a.root.join("n.md"), "the cat\nmiddle\nend\n").unwrap();
    set_mtime(&a.root.join("n.md"), mtime);
    a.sync_ok();
    b.write("n.md", "teh cat\nmiddle\nend\nphone line\n");
    b.write("other.md", "o\nphone\n");
    b.sync_ok();
    let r = a.sync();
    assert!(r.is_ok(), "A's sync fails: {:?}", r.err().map(|e| e.to_string()));
    assert_eq!(a.read("other.md").as_deref(), Some("o\nphone\n"));
    assert_eq!(a.read("n.md").as_deref(), Some("the cat\nmiddle\nend\nphone line\n"));
    b.sync_ok();
    assert_eq!(b.read("n.md").as_deref(), Some("the cat\nmiddle\nend\nphone line\n"));
}

// ------------------------------------------------------------------ several changes in one pull

#[test]
fn rename_then_new_note_with_the_old_name_does_not_conflict() {
    for new_name in ["Zebra.md", "Apple.md"] {
        let (_srv, mut a, mut b) = synced_pair(&[("Untitled.md", "first note\n")]);
        // the classic: rename "Untitled", then create another "Untitled"
        a.mv("Untitled.md", new_name);
        a.write("Untitled.md", "second note\n");
        a.sync_ok();
        let r = b.sync_ok();
        converge(&mut a, &mut b);
        let mut want = vec![(new_name.to_string(), "first note\n".to_string()), ("Untitled.md".to_string(), "second note\n".to_string())];
        want.sort();
        assert_eq!(a.files(), want, "rename to {new_name}: B reported conflicts {:?}", r.conflicts);
    }
}

#[test]
fn swapping_two_note_names_does_not_conflict() {
    let (_srv, mut a, mut b) = synced_pair(&[("A.md", "content a\n"), ("B.md", "content bb\n")]);
    a.mv("A.md", "tmp.md");
    a.mv("B.md", "A.md");
    a.mv("tmp.md", "B.md");
    a.sync_ok();
    let r = b.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(
        a.files(),
        vec![("A.md".to_string(), "content bb\n".to_string()), ("B.md".to_string(), "content a\n".to_string())],
        "conflicts on B: {:?}",
        r.conflicts
    );
}

#[test]
fn lost_state_with_a_local_edit_keeps_the_note_name() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n")]);
    a.write("n.md", "v2 laptop\n");
    fs::remove_file(a.state_file()).unwrap();
    a.restart();
    a.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(a.read("n.md").as_deref(), Some("v2 laptop\n"), "n.md is gone; files: {:?}", a.files());
}

#[test]
#[ignore = "FINDING-062: won't fix (by design) for renames made outside Cairn: a rename + edit there is a delete + create; see rename_in_app_plus_edit_on_one_side_and_edit_on_the_other_gives_one_note"]
fn rename_plus_edit_on_one_side_and_edit_on_the_other_gives_one_note() {
    let (_srv, mut a, mut b) = synced_pair(&[("Draft.md", "title\nbody\nend\n")]);
    // A renames the note and keeps typing in it before the next sync
    a.mv("Draft.md", "Plan.md");
    a.write("Plan.md", "title\nbody\nend\nmore from laptop\n");
    // B fixes the title meanwhile
    b.write("Draft.md", "Title\nbody\nend\n");
    a.sync_ok();
    b.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(
        a.files(),
        vec![("Plan.md".to_string(), "Title\nbody\nend\nmore from laptop\n".to_string())],
        "PLAN.md: 'Rename on one side, edit on the other: both are applied'"
    );
}

/// FINDING-062: as above, with the rename made in the app, which records
/// it for the next sync.
#[test]
fn rename_in_app_plus_edit_on_one_side_and_edit_on_the_other_gives_one_note() {
    let (_srv, mut a, mut b) = synced_pair(&[("Draft.md", "title\nbody\nend\n")]);
    a.app_mv("Draft.md", "Plan.md");
    a.write("Plan.md", "title\nbody\nend\nmore from laptop\n");
    b.write("Draft.md", "Title\nbody\nend\n");
    a.sync_ok();
    b.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(a.files(), vec![("Plan.md".to_string(), "Title\nbody\nend\nmore from laptop\n".to_string())]);
}

/// The rename is recorded in the sync state folder: the app may be closed
/// before the next sync.
#[test]
fn rename_in_app_is_known_after_a_restart() {
    let (_srv, mut a, mut b) = synced_pair(&[("Draft.md", "title\nbody\nend\n")]);
    a.app_mv("Draft.md", "Plan.md");
    a.write("Plan.md", "title\nbody\nend\nmore from laptop\n");
    a.restart();
    b.write("Draft.md", "Title\nbody\nend\n");
    b.sync_ok();
    a.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(a.files(), vec![("Plan.md".to_string(), "Title\nbody\nend\nmore from laptop\n".to_string())]);
}

/// A rename in the app while sync is not set up records nothing.
#[test]
fn rename_in_app_without_sync_records_nothing() {
    let (_srv, a, _b) = synced_pair(&[("Draft.md", "title\n")]);
    cairn_sync::engine::SyncEngine::disconnect(&a.state_dir).unwrap();
    a.app_mv("Draft.md", "Plan.md");
    assert_eq!(a.paths(), vec!["Plan.md".to_string()]);
    assert!(!a.state_dir.join("renames").exists());
}

#[test]
fn rename_plus_edit_on_one_side_alone_moves_the_note() {
    // without a concurrent edit the result is right (the old revision chain
    // is cut, so the old note goes to the other device's trash)
    let (_srv, mut a, mut b) = synced_pair(&[("Draft.md", "title\n")]);
    a.mv("Draft.md", "Plan.md");
    a.write("Plan.md", "title\nmore\n");
    converge(&mut a, &mut b);
    assert_eq!(b.files(), vec![("Plan.md".to_string(), "title\nmore\n".to_string())]);
}

#[test]
fn same_path_created_on_two_devices_keeps_one_note_under_the_name() {
    // PLAN.md: "the file uploaded first is renamed to a conflict copy and
    // the second keeps the name"
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Today.md", "laptop notes\n")]);
    let mut b = Device::new(&srv, "phone", &[("Today.md", "phone notes\n")]);
    a.sync_ok();
    b.sync_ok();
    converge(&mut a, &mut b);
    let files = a.files();
    assert!(files.iter().any(|f| f.0 == "Today.md"), "no note is called Today.md any more: {files:?}");
}

#[test]
fn replacing_a_note_with_a_same_size_same_mtime_note_is_uploaded() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "content a\n"), ("b.md", "content b\n")]);
    // two notes with the same size and mtime (unzip, cp -p, git, scripts)
    let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000);
    for n in ["a.md", "b.md"] {
        fs::File::options().write(true).open(a.root.join(n)).unwrap().set_modified(t).unwrap();
    }
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    // the user replaces a.md with b.md (mv keeps the mtime)
    a.mv("b.md", "a.md");
    for _ in 0..2 {
        a.sync_ok();
        b.sync_ok();
    }
    assert_eq!(a.files(), vec![("a.md".to_string(), "content b\n".to_string())]);
    assert_eq!(b.files(), a.files(), "the replacement never reached the other device");
}

// ------------------------------------------------------------------ two engines, one state folder

#[test]
fn two_engines_on_one_state_dir_do_not_duplicate_notes() {
    let mut failures = Vec::new();
    for iter in 0..5 {
        let srv = server();
        let mut a = Device::new(&srv, "laptop", &[("seed.md", "seed\n")]);
        a.sync_ok();
        for i in 0..30 {
            a.write(&format!("new/{i}.md"), &format!("note {i}\n"));
        }
        // a second Vault + engine on the same folder and state dir, as
        // open_vault creates when the vault is opened again while the old
        // SyncManager's sync is still running (stop() does not wait)
        let vault2 = Arc::new(cairn_core::Vault::open(Arc::new(cairn_core::StdFs::new(&a.root, cairn_core::TrashMode::Vault).unwrap())).unwrap());
        let mut e2 = cairn_sync::engine::SyncEngine::load_with(vault2, &a.state_dir, settings(&a.url, "laptop"), http(&a.url)).unwrap();
        let mut e1 = a.engine.take().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let b2 = barrier.clone();
        let t = std::thread::spawn(move || {
            b2.wait();
            let r = e2.sync().map(|_| ()).map_err(|e| e.to_string());
            (e2, r)
        });
        barrier.wait();
        let r1 = e1.sync().map(|_| ()).map_err(|e| e.to_string());
        let (_e2, r2) = t.join().unwrap();
        let state_ok = serde_json::from_slice::<serde_json::Value>(&fs::read(a.state_file()).unwrap_or_default()).is_ok();
        drop(e1);
        a.restart();
        let mut b = Device::new(&srv, "phone", &[]);
        for _ in 0..2 {
            let _ = a.sync();
            let _ = b.sync();
        }
        let files = b.files();
        let copies = conflict_copies(&files);
        if files.len() != 31 || !copies.is_empty() || !state_ok {
            failures.push(format!(
                "iteration {iter}: sync results {r1:?} / {r2:?}; state.json valid: {state_ok}; B has {} files (want 31), conflict copies: {:?}",
                files.len(),
                &copies[..copies.len().min(3)]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_engine_loaded_before_another_synced_goes_on_from_that_sync() {
    // the other engine's sync is over before this one starts (a second app
    // instance that was idle meanwhile): this one goes on from the state
    // that sync saved, not from the one it loaded
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("seed.md", "seed\n")]);
    a.sync_ok();
    let vault2 = Arc::new(cairn_core::Vault::open(Arc::new(cairn_core::StdFs::new(&a.root, cairn_core::TrashMode::Vault).unwrap())).unwrap());
    let mut e2 = cairn_sync::engine::SyncEngine::load_with(vault2, &a.state_dir, settings(&a.url, "laptop"), http(&a.url)).unwrap();
    for i in 0..5 {
        a.write(&format!("new/{i}.md"), &format!("note {i}\n"));
    }
    assert_eq!(a.sync_ok().pushed, 5);
    let r = e2.sync().unwrap();
    assert_eq!((r.pushed, r.pulled), (0, 0), "{r:?}");
    a.write("new/0.md", "note 0 edited\n");
    assert_eq!(e2.sync().unwrap().pushed, 1);
    // and the first engine takes up what the second one uploaded
    let r = a.sync_ok();
    assert_eq!((r.pushed, r.pulled), (0, 0), "{r:?}");
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    assert_eq!(b.files(), a.files());
}

/// A second engine (another app instance) on the vault and state folder of
/// `a`, loaded now.
fn second_engine(a: &Device, t: Box<dyn cairn_sync::transport::Transport>) -> cairn_sync::engine::SyncEngine {
    let vault = Arc::new(cairn_core::Vault::open(Arc::new(cairn_core::StdFs::new(&a.root, cairn_core::TrashMode::Vault).unwrap())).unwrap());
    cairn_sync::engine::SyncEngine::load_with(vault, &a.state_dir, settings(&a.url, "laptop"), t).unwrap()
}

#[test]
fn history_finds_a_note_another_engine_uploaded_after_a_refresh() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("seed.md", "seed\n")]);
    a.sync_ok();
    let mut e2 = second_engine(&a, http(&a.url));
    a.write("new.md", "new\n");
    a.sync_ok();
    assert!(e2.history("new.md").is_err());
    e2.refresh().unwrap();
    assert_eq!(e2.history("new.md").unwrap().len(), 1);
}

#[test]
fn an_engine_whose_folder_was_connected_to_a_recreated_vault_pushes_nothing() {
    // two app instances on one vault. The server loses its data, and the
    // user turns sync off and connects again in the first one, which
    // creates the vault again with a new key. The second one must not go
    // on from that state with the old key: other devices could not read
    // its uploads, and the first one would take them for synced.
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n")]);
    a.sync_ok();
    for i in 0..4 {
        a.write("a.md", &format!("a v{i}\n"));
        a.sync_ok();
    }
    let mut e2 = second_engine(&a, http(&a.url));
    let db = rusqlite::Connection::open(&srv.db_path).unwrap();
    db.execute_batch("DELETE FROM heads; DELETE FROM revisions; DELETE FROM vaults; DELETE FROM sqlite_sequence;").unwrap();
    a.engine = None;
    cairn_sync::engine::SyncEngine::disconnect(&a.state_dir).unwrap();
    a.engine = Some(
        cairn_sync::engine::SyncEngine::connect_with(a.vault.clone(), &a.state_dir, settings(&a.url, "laptop"), PASS, http(&a.url), FAST_KDF).unwrap(),
    );
    a.sync_ok();
    a.write("b.md", "b edited\n");
    let r = e2.sync();
    assert!(matches!(r, Err(cairn_sync::SyncError::NotConfigured)), "{r:?}");
    // loaded again, as the app then does, it goes on with the new key
    let mut e2 = second_engine(&a, http(&a.url));
    assert_eq!(e2.sync().unwrap().pushed, 1);
    let r = a.sync_ok();
    assert_eq!((r.pushed, r.pulled), (0, 0), "{r:?}");
    let mut b = Device::new(&srv, "phone", &[]);
    let r = b.sync_ok();
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    assert_eq!(b.files(), a.files());
}

#[test]
fn connecting_again_waits_for_a_sync_another_engine_runs() {
    // another app instance syncs from the folder while the user turns sync
    // off and connects again here: the connect waits for that sync, so the
    // sync cannot save its state into the folder set up anew
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("seed.md", "seed\n")]);
    a.sync_ok();
    for i in 0..5 {
        a.write(&format!("new/{i}.md"), &format!("note {i}\n"));
    }
    let (in_put, in_put_rx) = std::sync::mpsc::channel();
    let (go, go_rx) = std::sync::mpsc::channel::<()>();
    let t = FaultTransport::passthrough(http(&a.url));
    let mut first = true;
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "put" && std::mem::take(&mut first) {
            in_put.send(()).unwrap();
            let _ = go_rx.recv();
        }
    }));
    let mut e2 = second_engine(&a, Box::new(t));
    let syncing = std::thread::spawn(move || e2.sync().map(|r| r.pushed));
    in_put_rx.recv().unwrap();
    a.engine = None;
    cairn_sync::engine::SyncEngine::disconnect(&a.state_dir).unwrap();
    let (vault, dir, url) = (a.vault.clone(), a.state_dir.clone(), a.url.clone());
    let (connected, connected_rx) = std::sync::mpsc::channel();
    let connecting = std::thread::spawn(move || {
        let e = cairn_sync::engine::SyncEngine::connect_with(vault, &dir, settings(&url, "laptop"), PASS, http(&url), FAST_KDF);
        let _ = connected.send(());
        e
    });
    assert!(connected_rx.recv_timeout(std::time::Duration::from_millis(500)).is_err(), "connected while another engine synced");
    go.send(()).unwrap();
    let r = syncing.join().unwrap();
    assert!(matches!(r, Err(cairn_sync::SyncError::NotConfigured)), "the sync did not notice sync was turned off: {r:?}");
    a.engine = Some(connecting.join().unwrap().unwrap());
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    converge(&mut a, &mut b);
    assert_eq!(b.files(), a.files());
    assert_eq!(b.files().len(), 6);
    assert!(conflict_copies(&b.files()).is_empty(), "{:?}", b.paths());
}

// ------------------------------------------------------------------ one bad file

#[cfg(unix)] // Made unreadable with Unix mode bits. Windows has only a read-only flag.
#[test]
fn unreadable_file_does_not_block_other_changes() {
    use std::os::unix::fs::PermissionsExt;
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n")]);
    a.write("locked.pdf", "secret");
    fs::set_permissions(a.root.join("locked.pdf"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(a.root.join("locked.pdf")).is_ok() {
        eprintln!("running as root; permission test not meaningful");
        return;
    }
    a.write("n.md", "v2 laptop\n");
    b.write("from_phone.md", "hello\n");
    b.sync_ok();
    let r = a.sync();
    fs::set_permissions(a.root.join("locked.pdf"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(r.is_ok(), "A cannot sync at all: {}", r.err().unwrap());
}

// ------------------------------------------------------------------ misbehaving server

/// A transport whose changes feed always says "more" without moving the
/// cursor (a buggy or hostile server, or a proxy cache).
struct StuckFeed {
    inner: Box<dyn cairn_sync::transport::Transport>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl cairn_sync::transport::Transport for StuckFeed {
    fn get_vault(&self, v: &str) -> Result<Option<cairn_sync::protocol::VaultInfo>, cairn_sync::SyncError> {
        self.inner.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &cairn_sync::protocol::KeyEnvelope) -> Result<(), cairn_sync::SyncError> {
        self.inner.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<cairn_sync::protocol::ChangesResponse, cairn_sync::SyncError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n > 5_000 {
            return Err(cairn_sync::SyncError::Network("test gave up: client kept paging".into()));
        }
        let mut r = self.inner.changes(v, 0, l)?;
        r.more = true;
        r.cursor = s; // never advances
        Ok(r)
    }
    fn put(&self, v: &str, f: &str, r: &cairn_sync::protocol::PutRevision) -> Result<cairn_sync::transport::PutOutcome, cairn_sync::SyncError> {
        self.inner.put(v, f, r)
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<cairn_sync::protocol::HistoryEntry>, cairn_sync::SyncError> {
        self.inner.history(v, f)
    }
    fn revision(&self, v: &str, s: u64) -> Result<cairn_sync::protocol::RevisionBlob, cairn_sync::SyncError> {
        self.inner.revision(v, s)
    }
}

#[test]
fn changes_feed_that_never_advances_does_not_hang_the_client() {
    let (_srv, mut a, _b) = synced_pair(THREE);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let url = a.url.clone();
    a.set_transport(Box::new(StuckFeed { inner: http(&url), calls: calls.clone() }));
    let started = std::time::Instant::now();
    let r = a.sync();
    let n = calls.load(Ordering::SeqCst);
    assert!(n < 50, "client requested {n} pages in {:?} before the test cut it off ({:?})", started.elapsed(), r.err().map(|e| e.to_string()));
}

#[cfg(not(windows))] // B plays a Linux device. Windows cannot hold ':' in a name.
#[test]
fn unwritable_incoming_file_does_not_block_other_changes() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n")]);
    // B (Linux) creates a note whose name A's storage rejects (':' on an
    // exFAT SD card / Windows / some SAF providers)
    b.write("Meeting 10:30.md", "agenda\n");
    b.write("n.md", "v2 phone\n");
    b.write("zz.md", "later note\n");
    b.sync_ok();
    *a.hookfs.decide.lock() = Some(Box::new(|op, path, _| if op == FsOp::Write && path.contains(':') { FsAction::Fail } else { FsAction::Pass }));
    a.write("from_laptop.md", "hello\n");
    let mut errs = Vec::new();
    for _ in 0..2 {
        if let Err(e) = a.sync() {
            errs.push(e.to_string());
        }
    }
    b.sync_ok();
    assert!(
        a.read("n.md").as_deref() == Some("v2 phone\n") && a.read("zz.md").is_some() && b.read("from_laptop.md").is_some(),
        "A is stuck: n.md={:?}, zz.md={:?}, B got from_laptop.md: {}; errors: {:?}",
        a.read("n.md"),
        a.read("zz.md"),
        b.read("from_laptop.md").is_some(),
        errs.first()
    );
}

#[test]
#[ignore = "FINDING-054 (deferred: needs an automatic delete, which sync does not make by design): turning sync off and on brings back notes deleted on another device"]
fn disconnect_and_reconnect_does_not_resurrect_deleted_notes() {
    // "turn sync off and on again" (Settings > Sync > Disconnect, Connect)
    let (_srv, mut a, mut b) = synced_pair(&[("keep.md", "k\n"), ("old.md", "obsolete\n")]);
    b.rm("old.md");
    b.sync_ok();
    // A disconnects and reconnects with the same settings before pulling
    a.engine = None;
    cairn_sync::engine::SyncEngine::disconnect(&a.state_dir).unwrap();
    a.engine = Some(
        cairn_sync::engine::SyncEngine::connect_with(a.vault.clone(), &a.state_dir, settings(&a.url, "laptop"), PASS, http(&a.url), FAST_KDF).unwrap(),
    );
    a.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(b.paths(), vec!["keep.md"], "the note deleted on the phone is back everywhere");
}

// ------------------------------------------------------------------ disconnect while syncing

#[test]
fn disconnect_while_a_sync_is_uploading_leaves_no_stale_state() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("seed.md", "s\n")]);
    a.sync_ok();
    for i in 0..20 {
        a.write(&format!("n{i}.md"), &format!("{i}\n"));
    }
    let dir = a.state_dir.clone();
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    let dir2 = dir.clone();
    *t.before.lock() = Some(Box::new(move |op, n| {
        if op == "put" && n == 5 {
            // the user presses "Disconnect" in the middle of the upload
            cairn_sync::engine::SyncEngine::disconnect(&dir2).unwrap();
        }
    }));
    a.set_transport(Box::new(SharedTransport(t)));
    let r = a.sync();
    assert!(r.is_err(), "sync should notice its state folder is gone");
    a.engine = None;
    assert!(!dir.join("config.json").exists() && !dir.join("key").exists(), "a disconnected vault got its config back");
    // connecting again works and converges with a new device
    a.engine = Some(
        cairn_sync::engine::SyncEngine::connect_with(a.vault.clone(), &a.state_dir, settings(&a.url, "laptop"), PASS, http(&a.url), FAST_KDF).unwrap(),
    );
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    converge(&mut a, &mut b);
    assert_eq!(b.files().len(), 21);
    assert!(conflict_copies(&b.files()).is_empty(), "{:?}", b.paths());
}

#[test]
fn deleted_bases_degrade_to_conflict_copies_without_loss() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "1\n2\n3\n4\n5\n")]);
    fs::remove_dir_all(a.state_dir.join("bases")).unwrap();
    fs::create_dir_all(a.state_dir.join("bases")).unwrap();
    b.write("n.md", "1 phone\n2\n3\n4\n5\n");
    b.sync_ok();
    a.write("n.md", "1\n2\n3\n4\n5 laptop\n");
    a.sync_ok();
    converge(&mut a, &mut b);
    let all = a.all_text();
    assert!(all.contains("1 phone") && all.contains("5 laptop"), "{:?}", a.files());
    eprintln!("without a base: {:?}", a.paths());
}

#[test]
fn file_on_one_device_folder_on_the_other_does_not_block_sync() {
    let srv = server();
    // a file called "Projects" (no extension) on the laptop, a folder
    // "Projects/" on the phone
    let mut a = Device::new(&srv, "laptop", &[("Projects", "a plain file\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[("Projects/plan.md", "the plan\n"), ("other.md", "o\n")]);
    let r1 = b.sync();
    let r2 = b.sync();
    let _ = a.sync();
    let all = format!("{}{}", a.all_text(), b.all_text());
    assert!(
        r1.is_ok() && r2.is_ok() && all.contains("a plain file") && all.contains("the plan") && a.read("other.md").is_some(),
        "phone: {:?} / {:?}; laptop has {:?}, phone has {:?}",
        r1.err().map(|e| e.to_string()),
        r2.err().map(|e| e.to_string()),
        a.paths(),
        b.paths()
    );
}

fn skipped_paths(r: &cairn_sync::engine::SyncReport) -> Vec<&str> {
    r.skipped.iter().map(|s| s.path.as_str()).collect()
}

/// A remote edit this device cannot write stays pending across syncs and a
/// restart. The local edit of that file waits for it (pushing it would
/// conflict with a head this device has not applied), and both are merged
/// once the file can be written.
#[test]
fn pending_remote_change_is_retried_and_merged_once_it_can_be_written() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "1\n2\n3\n"), ("other.md", "o\n")]);
    b.write("n.md", "1 phone\n2\n3\n");
    b.sync_ok();
    *a.hookfs.decide.lock() = Some(Box::new(|op, path, _| if op == FsOp::Write && path == "n.md" { FsAction::Fail } else { FsAction::Pass }));
    a.write("n.md", "1\n2\n3 laptop\n");
    a.write("other.md", "o laptop\n");
    for _ in 0..2 {
        let r = a.sync_ok();
        assert_eq!(skipped_paths(&r), ["n.md"], "{:?}", r.skipped);
    }
    b.sync_ok();
    assert_eq!(b.read("other.md").as_deref(), Some("o laptop\n"), "the rest of the vault did not sync");
    assert_eq!(b.read("n.md").as_deref(), Some("1 phone\n2\n3\n"), "the held local edit was pushed over the phone's");
    a.restart(); // the write problem is gone; the pending change was saved
    let r = a.sync_ok();
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    b.sync_ok();
    for d in [&a, &b] {
        assert_eq!(d.read("n.md").as_deref(), Some("1 phone\n2\n3 laptop\n"), "{}: {:?}", d.name, d.paths());
        assert!(conflict_copies(&d.files()).is_empty(), "{}: {:?}", d.name, d.paths());
    }
}

/// A file or folder whose name has a backslash stays out of the vault
/// (FINDING-011, by design) and is listed as not synced, with the
/// reason; once renamed it syncs and leaves the list.
#[cfg(not(windows))] // A backslash is a path separator on Windows.
#[test]
fn names_with_a_backslash_are_listed_as_not_synced() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("ok.md", "ok\n")]);
    a.sync_ok();
    fs::write(a.root.join("back\\slash.md"), "backslash\n").unwrap();
    fs::create_dir(a.root.join("x\\y")).unwrap();
    fs::write(a.root.join("x\\y/n.md"), "inside\n").unwrap();
    let r = a.sync_ok();
    let reason = cairn_core::vault::BACKSLASH_REASON;
    assert_eq!(reason, "The name contains a backslash. Rename it to sync it.", "the user's wording");
    let mut listed: Vec<(&str, &str)> = r.skipped.iter().map(|s| (s.path.as_str(), s.reason.as_str())).collect();
    listed.sort();
    assert_eq!(listed, [("back\\slash.md", reason), ("x\\y", reason)]);
    fs::rename(a.root.join("back\\slash.md"), a.root.join("back slash.md")).unwrap();
    let r = a.sync_ok();
    assert_eq!(skipped_paths(&r), ["x\\y"], "{:?}", r.skipped);
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    assert_eq!(b.paths(), ["back slash.md", "ok.md"]);
}

/// A synced file that cannot be read is not mistaken for a deletion, and a
/// remote edit of it waits until it can be read; nothing is lost.
#[cfg(unix)] // Made unreadable with Unix mode bits. Windows has only a read-only flag.
#[test]
fn unreadable_synced_file_is_not_pushed_as_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n"), ("locked.md", "secret v1\n")]);
    a.write("locked.md", "secret v2 from the laptop\n");
    fs::set_permissions(a.root.join("locked.md"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(a.root.join("locked.md")).is_ok() {
        eprintln!("running as root; permission test not meaningful");
        return;
    }
    b.write("locked.md", "secret v2 from the phone\n");
    b.sync_ok();
    a.write("n.md", "v2\n");
    let r = a.sync();
    fs::set_permissions(a.root.join("locked.md"), fs::Permissions::from_mode(0o644)).unwrap();
    let r = r.unwrap();
    assert!(skipped_paths(&r).contains(&"locked.md"), "{:?}", r.skipped);
    // The reasons the list of files not synced shows: the scan's, as the
    // core words it (without its "io error: "), and the pull's.
    let reasons: Vec<&str> = r.skipped.iter().filter(|s| s.path == "locked.md").map(|s| s.reason.as_str()).collect();
    assert_eq!(reasons, ["No permission to access \"locked.md\".", "cannot read locked.md on this device"], "{:?}", r.skipped);
    b.sync_ok();
    assert_eq!(b.read("n.md").as_deref(), Some("v2\n"));
    assert_eq!(b.read("locked.md").as_deref(), Some("secret v2 from the phone\n"), "the unreadable file was pushed as deleted");
    converge(&mut a, &mut b);
    let all = a.all_text();
    assert!(all.contains("secret v2 from the laptop") && all.contains("secret v2 from the phone"), "{:?}", a.files());
}

/// A vault folder (`dir`) that cannot be read or written while `locked`, as
/// a vault sees one that becomes unreadable (mode 000) after it was listed:
/// its files are still listed, but every look at one fails.
struct LockedFolderFs {
    inner: Arc<dyn cairn_core::VaultFs>,
    dir: &'static str,
    locked: Arc<std::sync::atomic::AtomicBool>,
}

impl LockedFolderFs {
    fn check(&self, path: &str) -> cairn_core::Result<()> {
        if self.locked.load(Ordering::SeqCst) && path.starts_with(&format!("{}/", self.dir)) {
            return Err(cairn_core::CoreError::Io(format!("{path}: Permission denied (os error 13)")));
        }
        Ok(())
    }
}

impl cairn_core::VaultFs for LockedFolderFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<cairn_core::FileStat>> {
        self.inner.list(dir)
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<cairn_core::FileStat>> {
        self.check(path)?;
        self.inner.stat(path)
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.check(path)?;
        self.inner.read(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<cairn_core::FileStat> {
        self.check(path)?;
        self.inner.write(path, data)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.check(path)?;
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        self.check(from)?;
        self.check(to)?;
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.check(path)?;
        self.inner.remove(path)
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.check(path)?;
        self.inner.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn change_stamp(&self, path: &str) -> Option<u64> {
        self.check(path).ok()?;
        self.inner.change_stamp(path)
    }
}

/// Remote changes to files in a folder that cannot be read here (an edit,
/// a new file, a deletion and a move into it) wait as pending: the sync goes
/// on both ways for the rest, and they are applied once the folder can be
/// read again.
#[test]
fn remote_changes_in_an_unreadable_folder_wait_and_the_rest_syncs() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "n\n"), ("a.md", "a\n"), ("private/x.md", "x\n"), ("private/y.md", "y\n")]);
    a.sync_ok();
    let locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let l = locked.clone();
    let maker: FsMaker = Arc::new(move |root: &std::path::Path| {
        let inner: Arc<dyn cairn_core::VaultFs> = Arc::new(cairn_core::StdFs::new(root, cairn_core::TrashMode::Vault).unwrap());
        Arc::new(LockedFolderFs { inner, dir: "private", locked: l.clone() }) as Arc<dyn cairn_core::VaultFs>
    });
    let mut b = Device::new_with_fs(&srv.url, "phone", &[], Some(maker));
    b.sync_ok();
    locked.store(true, Ordering::SeqCst);
    a.write("private/x.md", "x edited on the laptop\n");
    a.rm("private/y.md");
    a.write("private/new.md", "new on the laptop\n");
    a.mv("a.md", "private/a.md");
    a.write("n.md", "n edited on the laptop\n");
    a.sync_ok();
    b.write("phone.md", "made on the phone\n");
    for restart in [false, true] {
        if restart {
            b.restart();
        }
        let r = b.sync_ok();
        let mut skipped = skipped_paths(&r);
        skipped.sort();
        skipped.dedup();
        assert_eq!(skipped, ["private/a.md", "private/new.md", "private/x.md", "private/y.md"], "{:?}", r.skipped);
        assert_eq!(b.read("n.md").as_deref(), Some("n edited on the laptop\n"));
    }
    a.sync_ok();
    assert_eq!(a.read("phone.md").as_deref(), Some("made on the phone\n"));
    assert_eq!(a.paths(), ["n.md", "phone.md", "private/a.md", "private/new.md", "private/x.md"]);
    locked.store(false, Ordering::SeqCst);
    let r = b.sync_ok();
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    converge(&mut a, &mut b);
    assert_eq!(b.files(), a.files());
    assert_eq!(b.read("private/x.md").as_deref(), Some("x edited on the laptop\n"));
    assert!(b.trash().iter().any(|(_, c)| c == "y\n"), "{:?}", b.trash());
}

/// The same with a real folder (mode 000) that holds every note of the
/// vault: the vault does not look empty (FINDING-006 stops the sync of an
/// empty vault folder), and the sync goes on for the rest, also after a
/// restart. The scan's empty-vault check leaves out a vault with folders
/// it cannot read (Vault::unreadable_folders, FINDING-049).
#[cfg(unix)] // Made unreadable with Unix mode bits. Windows ignores the read-only flag on a folder.
#[test]
fn a_vault_whose_notes_are_all_in_an_unreadable_folder_still_syncs() {
    use std::os::unix::fs::PermissionsExt;
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("private/x.md", "x\n"), ("private/y.md", "y\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    let private = b.root.join("private");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&private).is_ok() {
        fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("running as root; permission test not meaningful");
        return;
    }
    a.write("private/x.md", "x edited on the laptop\n");
    a.write("n.md", "new on the laptop\n");
    a.sync_ok();
    // The folder is made readable again before any assertion (or a failed
    // restart) can stop the test, so that its temporary folder goes away.
    let results = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut results = Vec::new();
        for restart in [false, true] {
            if restart {
                b.restart();
            }
            results.push(b.sync().map(|r| skipped_paths(&r).contains(&"private/x.md")).map_err(|e| e.to_string()));
        }
        results
    }));
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    let results = results.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert_eq!(results, [Ok(true), Ok(true)]);
    assert_eq!(b.read("n.md").as_deref(), Some("new on the laptop\n"));
    b.sync_ok();
    converge(&mut a, &mut b);
    assert_eq!(b.read("private/x.md").as_deref(), Some("x edited on the laptop\n"));
    assert_eq!(b.paths(), ["n.md", "private/x.md", "private/y.md"]);
}

/// A file whose server head does not decrypt (bit rot on the server): the
/// rest syncs, a local edit of that file waits instead of overwriting a
/// version this device never saw, and is merged once the head is valid.
#[test]
fn local_edit_waits_while_the_server_head_does_not_decrypt() {
    let (srv, mut a, mut b) = synced_pair(&[("n.md", "1\n2\n3\n4\n5\n"), ("other.md", "o\n")]);
    b.write("n.md", "1 phone\n2\n3\n4\n5\n");
    b.sync_ok();
    {
        let conn = rusqlite::Connection::open(&srv.db_path).unwrap();
        let (seq, mut blob): (i64, Vec<u8>) = conn.query_row("SELECT seq, blob FROM revisions ORDER BY seq DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x40;
        conn.execute("UPDATE revisions SET blob = ?1 WHERE seq = ?2", rusqlite::params![blob, seq]).unwrap();
    }
    a.write("n.md", "1\n2\n3\n4\n5 laptop\n");
    a.write("other.md", "o laptop\n");
    let r = a.sync_ok();
    assert_eq!(skipped_paths(&r), ["n.md"], "{:?}", r.skipped);
    // The held record is kept here; the same broken revision is not
    // downloaded again on the next sync.
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    let ops = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let o = ops.clone();
    *t.before.lock() = Some(Box::new(move |op, _| o.lock().push(op.to_string())));
    a.set_transport(Box::new(SharedTransport(t.clone())));
    let r = a.sync_ok();
    assert_eq!(skipped_paths(&r), ["n.md"], "{:?}", r.skipped);
    // the vault info is asked for to check the server was not reset (FINDING-058)
    assert_eq!(*ops.lock(), ["get_vault", "changes"], "only the vault info and the changes feed are asked for");
    b.sync_ok();
    assert_eq!(b.read("other.md").as_deref(), Some("o laptop\n"));
    assert_eq!(b.read("n.md").as_deref(), Some("1 phone\n2\n3\n4\n5\n"), "the laptop overwrote a version it never saw");
    // The phone, which has the lost version, edits the note again.
    b.write("n.md", "1 phone\n2 phone\n3\n4\n5\n");
    b.sync_ok();
    let r = a.sync_ok();
    assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "{:?} {:?}", r.skipped, r.conflicts);
    b.sync_ok();
    assert_eq!(a.read("n.md").as_deref(), Some("1 phone\n2 phone\n3\n4\n5 laptop\n"));
    assert_eq!(b.read("n.md").as_deref(), Some("1 phone\n2 phone\n3\n4\n5 laptop\n"));
}

/// A pending change is kept on this device: retrying it does not download
/// it again (an attachment that cannot be written would otherwise cost its
/// full size on every sync), and the copy is removed once it is applied.
#[test]
fn pending_change_is_not_downloaded_again() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n")]);
    let big = "x".repeat(200_000);
    b.write("big.png", &big);
    b.sync_ok();
    *a.hookfs.decide.lock() = Some(Box::new(|op, path, _| if op == FsOp::Write && path == "big.png" { FsAction::Fail } else { FsAction::Pass }));
    a.sync_ok();
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    let fetched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let f = fetched.clone();
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "revision" {
            f.fetch_add(1, Ordering::SeqCst);
        }
    }));
    a.set_transport(Box::new(SharedTransport(t.clone())));
    for _ in 0..3 {
        let r = a.sync_ok();
        assert_eq!(skipped_paths(&r), ["big.png"], "{:?}", r.skipped);
    }
    assert_eq!(fetched.load(Ordering::SeqCst), 0, "the pending change was downloaded again");
    assert_eq!(t.blob_bytes_in.load(Ordering::SeqCst), 0);
    a.restart(); // the write problem is gone
    let r = a.sync_ok();
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    assert_eq!(a.read("big.png").as_deref(), Some(big.as_str()));
    let left: Vec<_> = fs::read_dir(a.state_dir.join("pending")).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}

/// A remote rename + edit of a note edited here: the rename is done, then
/// writing the merge fails. The moved note is not uploaded as a new file
/// meanwhile, and is merged under its new name once it can be written.
#[test]
fn half_applied_remote_rename_is_not_pushed_as_a_new_file() {
    let (_srv, mut a, mut b) = synced_pair(&[("a.md", "1\n2\n3\n4\n5\n")]);
    b.mv("a.md", "b.md");
    b.sync_ok();
    b.write("b.md", "1 phone\n2\n3\n4\n5\n");
    b.sync_ok();
    a.write("a.md", "1\n2\n3\n4\n5 laptop\n");
    *a.hookfs.decide.lock() = Some(Box::new(|op, path, _| if op == FsOp::Write && path == "b.md" { FsAction::Fail } else { FsAction::Pass }));
    for _ in 0..2 {
        let r = a.sync_ok();
        assert_eq!(skipped_paths(&r), ["b.md"], "{:?}", r.skipped);
    }
    b.sync_ok();
    assert_eq!(b.files(), [("b.md".to_string(), "1 phone\n2\n3\n4\n5\n".to_string())], "the held note reached the phone");
    a.restart();
    converge(&mut a, &mut b);
    assert_eq!(a.files(), [("b.md".to_string(), "1 phone\n2\n3\n4\n5 laptop\n".to_string())]);
}

/// Both devices renamed a note, and the phone also edited it: here the
/// rename to the phone's name fails. The phone's content is not uploaded
/// again as a new file under the laptop's name meanwhile.
#[test]
fn half_applied_rename_on_both_sides_is_not_pushed_as_a_new_file() {
    let (_srv, mut a, mut b) = synced_pair(&[("a.md", "1\n2\n3\n")]);
    b.mv("a.md", "b.md");
    b.sync_ok();
    b.write("b.md", "1 phone\n2\n3\n");
    b.sync_ok();
    a.mv("a.md", "c.md");
    *a.hookfs.decide.lock() = Some(Box::new(|op, path, _| if op == FsOp::Rename && path == "c.md -> b.md" { FsAction::Fail } else { FsAction::Pass }));
    for _ in 0..2 {
        let r = a.sync_ok();
        assert_eq!(skipped_paths(&r), ["b.md"], "{:?}", r.skipped);
    }
    b.sync_ok();
    assert_eq!(b.files(), [("b.md".to_string(), "1 phone\n2\n3\n".to_string())], "the held note reached the phone");
    a.restart();
    converge(&mut a, &mut b);
    assert_eq!(a.files(), [("b.md".to_string(), "1 phone\n2\n3\n".to_string())]);
}

/// The server loses the revisions this device holds as pending (restored
/// from an older backup): the rest of the vault keeps syncing, the local
/// copy of the held note is kept, and a file this device never had is
/// dropped from the pending list instead of being asked for forever.
#[test]
fn pending_change_missing_on_the_server_does_not_stop_sync() {
    let (srv, mut a, mut b) = synced_pair(&[("n.md", "1\n2\n3\n"), ("other.md", "o\n")]);
    let before: i64 = rusqlite::Connection::open(&srv.db_path).unwrap().query_row("SELECT MAX(seq) FROM revisions", [], |r| r.get(0)).unwrap();
    b.write("n.md", "1 phone\n2\n3\n");
    b.write("new.md", "new on the phone\n");
    b.sync_ok();
    *a.hookfs.decide.lock() =
        Some(Box::new(|op, path, _| if op == FsOp::Write && (path == "n.md" || path == "new.md") { FsAction::Fail } else { FsAction::Pass }));
    let r = a.sync_ok();
    assert_eq!(skipped_paths(&r), ["n.md", "new.md"], "{:?}", r.skipped);
    {
        let conn = rusqlite::Connection::open(&srv.db_path).unwrap();
        let lost: Vec<(i64, String, Option<i64>)> = conn
            .prepare("SELECT seq, file_id, parent_seq FROM revisions WHERE seq > ?1")
            .unwrap()
            .query_map([before], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(lost.len(), 2);
        for (seq, fid, parent) in lost {
            conn.execute("DELETE FROM revisions WHERE seq = ?1", [seq]).unwrap();
            match parent {
                Some(p) => conn.execute("UPDATE heads SET seq = ?1 WHERE file_id = ?2", rusqlite::params![p, fid]).unwrap(),
                None => conn.execute("DELETE FROM heads WHERE file_id = ?1", [fid]).unwrap(),
            };
        }
    }
    let _ = fs::remove_dir_all(a.state_dir.join("pending")); // fetched again, not from a local copy
    // The phone has seen its uploads, which the server lost: it was reset
    // as far as the phone can tell. Another device goes on.
    assert!(matches!(b.sync(), Err(cairn_sync::SyncError::ServerReset)), "the phone does not see that its uploads are gone");
    let mut c = Device::new(&srv, "tablet", &[]);
    c.sync_ok();
    c.write("other.md", "o tablet\n");
    c.sync_ok();
    a.write("from_laptop.md", "hello\n");
    let r1 = a.sync().map_err(|e| e.to_string());
    let r2 = a.sync().map_err(|e| e.to_string());
    c.sync_ok();
    let (r1, r2) = (r1.unwrap(), r2.unwrap());
    assert_eq!(a.read("other.md").as_deref(), Some("o tablet\n"));
    assert_eq!(c.read("from_laptop.md").as_deref(), Some("hello\n"));
    assert_eq!(a.read("n.md").as_deref(), Some("1\n2\n3\n"), "the held note changed");
    let mut p1 = skipped_paths(&r1);
    p1.sort();
    assert_eq!(p1, ["", "n.md"], "{:?}", r1.skipped);
    assert_eq!(skipped_paths(&r2), ["n.md"], "{:?}", r2.skipped);
}

// ------------------------------------------------------------------ case-insensitive devices

fn case_insensitive() -> Option<FsMaker> {
    Some(Arc::new(|root: &std::path::Path| Arc::new(CaseInsensitiveFs::new(root)) as Arc<dyn cairn_core::VaultFs>))
}

#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn notes_differing_only_in_case_survive_a_case_insensitive_device() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Ideas.md", "UPPER: the only copy of these ideas\n"), ("ideas.md", "lower: a different note\n")]);
    linux.sync_ok();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[], case_insensitive());
    let r = mac.sync();
    for _ in 0..2 {
        let _ = mac.sync();
        let _ = linux.sync();
    }
    let all = format!("{}{}", linux.all_text(), mac.all_text());
    assert!(
        all.contains("UPPER: the only copy") && all.contains("lower: a different note"),
        "first mac sync: {:?}\nlinux now: {:?} trash {:?}\nmac now: {:?}",
        r.map(|r| r.conflicts).map_err(|e| e.to_string()),
        linux.files(),
        linux.trash(),
        mac.files()
    );
}

/// A case-sensitive device keeps both names as they are.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn notes_differing_only_in_case_stay_apart_on_a_case_sensitive_device() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Note.md", "upper\n"), ("note.md", "lower\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[("NOTE.md", "phone\n")]);
    let r = b.sync_ok();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    a.sync_ok();
    let want = [("NOTE.md", "phone\n"), ("Note.md", "upper\n"), ("note.md", "lower\n")].map(|(p, c)| (p.to_string(), c.to_string()));
    assert_eq!(a.files(), want);
    assert_eq!(b.files(), want);
}

fn saf_like() -> Option<FsMaker> {
    Some(Arc::new(|root: &std::path::Path| Arc::new(SafLikeFs::new(root)) as Arc<dyn cairn_core::VaultFs>))
}

fn owned(files: &[(&str, &str)]) -> Vec<(String, String)> {
    files.iter().map(|(p, c)| (p.to_string(), c.to_string())).collect()
}

/// FINDING-172, by design: Android shared storage cannot hold two
/// names that differ only in case, so the phone keeps the first such note
/// from a case-sensitive device and lists the other as not synced. No device
/// renames either one: a conflict copy name there would be pushed and
/// rename the desktop's note on every device.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn case_twins_from_a_case_sensitive_device_are_listed_not_renamed_on_android_shared_storage() {
    let srv = server();
    let notes = [("Notes.md", "UPPER case note\n"), ("notes.md", "lower case note\n")];
    let mut linux = Device::new(&srv, "linux", &notes);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    for round in 0..2 {
        let r = phone.sync_ok();
        let files = phone.files();
        assert!(files.len() == 1 && owned(&notes).contains(&files[0]), "round {round}: the phone keeps one of the two, as it is: {files:?}");
        let other = notes.iter().map(|n| n.0).find(|p| *p != files[0].0).unwrap();
        assert!(r.conflicts.is_empty(), "round {round}: no conflict copy: {:?}", r.conflicts);
        assert!(
            r.skipped.iter().any(|s| s.path == other && s.reason.contains("differ only in case")),
            "round {round}: {other} is listed as not synced: {:?}",
            r.skipped
        );
        linux.sync_ok();
        assert_eq!(linux.files(), owned(&notes), "round {round}: the desktop keeps both names");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// The same for a note renamed on the case-sensitive device to a case twin of
/// another note: the phone leaves it under its old name and lists the new
/// one, and the desktop keeps the name it gave it.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn a_rename_to_a_case_twin_is_listed_not_renamed_on_android_shared_storage() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Notes.md", "upper\n"), ("draft.md", "draft\n")]);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    phone.sync_ok();
    linux.mv("draft.md", "notes.md");
    linux.sync_ok();
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.conflicts.is_empty(), "round {round}: no conflict copy: {:?}", r.conflicts);
        assert!(
            r.skipped.iter().any(|s| s.path == "notes.md" && s.reason.contains("differ only in case")),
            "round {round}: the new name is listed as not synced: {:?}",
            r.skipped
        );
        assert_eq!(phone.files(), owned(&[("Notes.md", "upper\n"), ("draft.md", "draft\n")]), "round {round}");
        linux.sync_ok();
        assert_eq!(linux.files(), owned(&[("Notes.md", "upper\n"), ("notes.md", "draft\n")]), "round {round}: the desktop keeps its rename");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// And for a note deleted on the phone while the case-sensitive device
/// renamed it to a case twin of another note: the rename wins, but the phone
/// cannot store it under that name, so it lists it instead of storing it
/// under a conflict copy name; the desktop keeps the note.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn a_rename_to_a_case_twin_of_a_note_deleted_here_is_listed_on_android_shared_storage() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Notes.md", "upper\n"), ("draft.md", "draft\n")]);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    phone.sync_ok();
    phone.rm("draft.md");
    linux.mv("draft.md", "notes.md");
    linux.sync_ok();
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.conflicts.is_empty(), "round {round}: no conflict copy: {:?}", r.conflicts);
        assert!(
            r.skipped.iter().any(|s| s.path == "notes.md" && s.reason.contains("differ only in case")),
            "round {round}: the renamed note is listed as not synced: {:?}",
            r.skipped
        );
        assert_eq!(phone.files(), owned(&[("Notes.md", "upper\n")]), "round {round}");
        linux.sync_ok();
        assert_eq!(linux.files(), owned(&[("Notes.md", "upper\n"), ("notes.md", "draft\n")]), "round {round}: the desktop keeps the renamed note");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// A case twin named like an error does not stop the sync. A reason that
/// quotes both names, one with "changed on disk" in it, must not be taken
/// for a race: every round of the sync would be retried until it failed, so
/// later files would never be pulled.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn a_case_twin_named_like_an_error_does_not_stop_the_sync_on_android_shared_storage() {
    let srv = server();
    let notes = [("Changed on disk.md", "upper\n"), ("changed on disk.md", "lower\n"), ("other.md", "other\n")];
    let mut linux = Device::new(&srv, "linux", &notes);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    let r = phone.sync_ok();
    let files = phone.files();
    assert_eq!(files.len(), 2, "the phone keeps one twin and the other note: {files:?}");
    assert!(files.contains(&("other.md".to_string(), "other\n".to_string())), "{files:?}");
    assert!(r.skipped.iter().any(|s| s.reason.contains("differ only in case")), "{:?}", r.skipped);
    linux.sync_ok();
    assert_eq!(linux.files(), owned(&notes));
}

/// A name the phone's storage cannot hold is listed as not synced (by
/// design, FINDING-172), whatever the name says: the reason SafFs gives
/// starts with the path, and one with "changed on disk" in it must not be
/// taken for a race, which would retry every round of the sync until it
/// failed.
#[cfg(not(windows))] // The desktop plays Linux. Windows cannot hold '?' in a name.
#[test]
fn a_refused_name_that_reads_like_an_error_does_not_stop_the_sync_on_android_shared_storage() {
    let srv = server();
    let notes = [("changed on disk?.md", "question\n"), ("other.md", "other\n")];
    let mut linux = Device::new(&srv, "linux", &notes);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    for round in 0..2 {
        let r = phone.sync_ok();
        assert_eq!(phone.files(), owned(&[("other.md", "other\n")]), "round {round}");
        assert!(
            r.skipped.iter().any(|s| s.path == "changed on disk?.md" && s.reason.contains("not allowed on this storage")),
            "round {round}: {:?}",
            r.skipped
        );
    }
    linux.sync_ok();
    assert_eq!(linux.files(), owned(&notes));
}

/// A synced desktop with Notes.md and draft.md, a phone on Android shared
/// storage that has both, and the desktop's rename of draft.md to notes.md,
/// which the phone cannot store next to Notes.md.
fn rename_to_a_case_twin_on_the_desktop(srv: &Server) -> (Device, Device) {
    let mut linux = Device::new(srv, "linux", &[("Notes.md", "upper\n"), ("draft.md", "line 1\nline 2\n")]);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    phone.sync_ok();
    linux.mv("draft.md", "notes.md");
    linux.sync_ok();
    (linux, phone)
}

/// Sync the phone, then the desktop, twice: the phone keeps the rename back
/// and lists the new name, with the name the note has on the phone, and the
/// desktop keeps the name it gave the note.
fn rename_stays_held(phone: &mut Device, linux: &mut Device, on_phone: &[(&str, &str)], on_desktop: &[(&str, &str)]) {
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.conflicts.is_empty(), "round {round}: no conflict copy: {:?}", r.conflicts);
        assert!(
            r.skipped.iter().any(|s| s.path == "notes.md" && s.reason.contains("differ only in case") && s.reason.contains("draft.md")),
            "round {round}: the new name is listed as not synced, with the note's name here: {:?}",
            r.skipped
        );
        assert_eq!(phone.files(), owned(on_phone), "round {round}: the phone keeps the note as it is");
        linux.sync_ok();
        assert_eq!(linux.files(), owned(on_desktop), "round {round}: the desktop keeps its rename");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// FINDING-172, case 1: a rename that the phone holds back stays held when
/// the note is edited on the phone. The phone must not push the old name
/// with the edit, which would undo the rename on every device. The edit
/// waits on the phone, which lists the note, and goes up, merged, once the
/// name is free.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn an_edit_on_the_phone_does_not_undo_a_held_rename_to_a_case_twin_on_android_shared_storage() {
    let srv = server();
    let (mut linux, mut phone) = rename_to_a_case_twin_on_the_desktop(&srv);
    let r = phone.sync_ok();
    assert!(r.skipped.iter().any(|s| s.path == "notes.md"), "the rename is held: {:?}", r.skipped);
    phone.write("draft.md", "line 1\nline 2\nphone line\n");
    let edited = [("Notes.md", "upper\n"), ("draft.md", "line 1\nline 2\nphone line\n")];
    rename_stays_held(&mut phone, &mut linux, &edited, &[("Notes.md", "upper\n"), ("notes.md", "line 1\nline 2\n")]);
    // The desktop renames the note again in Cairn, and edits it: the name is
    // free on the phone now, and both edits reach both devices. (Renamed
    // outside Cairn and edited, it would sync as a delete and a new note,
    // FINDING-062.)
    linux.app_mv("notes.md", "notes 2.md");
    linux.write("notes 2.md", "desktop line\nline 1\nline 2\n");
    linux.sync_ok();
    let r = phone.sync_ok();
    assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "{:?} {:?}", r.skipped, r.conflicts);
    linux.sync_ok();
    let want = owned(&[("Notes.md", "upper\n"), ("notes 2.md", "desktop line\nline 1\nline 2\nphone line\n")]);
    assert_eq!(phone.files(), want);
    assert_eq!(linux.files(), want);
}

/// The same for an edit made on the phone before it pulled the rename. Here
/// the desktop frees the name by renaming the other note: the phone applies
/// that rename first, then the held one, with the edit.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn an_edit_made_before_the_pull_does_not_undo_a_rename_to_a_case_twin_on_android_shared_storage() {
    let srv = server();
    let (mut linux, mut phone) = rename_to_a_case_twin_on_the_desktop(&srv);
    phone.write("draft.md", "line 1\nline 2\nphone line\n");
    let edited = [("Notes.md", "upper\n"), ("draft.md", "line 1\nline 2\nphone line\n")];
    rename_stays_held(&mut phone, &mut linux, &edited, &[("Notes.md", "upper\n"), ("notes.md", "line 1\nline 2\n")]);
    linux.mv("Notes.md", "Old notes.md");
    linux.sync_ok();
    let r = phone.sync_ok();
    assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "{:?} {:?}", r.skipped, r.conflicts);
    linux.sync_ok();
    let want = owned(&[("Old notes.md", "upper\n"), ("notes.md", "line 1\nline 2\nphone line\n")]);
    assert_eq!(phone.files(), want);
    assert_eq!(linux.files(), want);
}

/// And for a note the phone has under a conflict copy name that it has not
/// uploaded yet: the phone does not push that name, a name the sync chose,
/// over the desktop's rename.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn a_conflict_copy_name_not_uploaded_yet_does_not_undo_a_rename_to_a_case_twin_on_android_shared_storage() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Notes.md", "upper\n")]);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    phone.sync_ok();
    linux.write("x.md", "desktop x\n");
    linux.sync_ok();
    // The phone made its own x.md meanwhile: it stores the desktop's as a
    // conflict copy, and the upload of that name fails.
    phone.write("x.md", "phone x\n");
    fail_puts(&mut phone);
    assert!(phone.sync().is_err());
    plain(&mut phone);
    let copy = phone.paths().into_iter().find(|p| p.contains("(conflict ")).expect("a conflict copy on the phone");
    assert_eq!(phone.read(&copy).as_deref(), Some("desktop x\n"));
    linux.mv("x.md", "notes.md");
    linux.sync_ok();
    let on_phone = [("Notes.md", "upper\n"), (copy.as_str(), "desktop x\n"), ("x.md", "phone x\n")];
    let mut on_phone = owned(&on_phone);
    on_phone.sort();
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(
            r.skipped.iter().any(|s| s.path == "notes.md" && s.reason.contains("differ only in case") && s.reason.contains(&copy)),
            "round {round}: the new name is listed as not synced, with the note's name here: {:?}",
            r.skipped
        );
        assert_eq!(phone.files(), on_phone, "round {round}");
        linux.sync_ok();
        let want = owned(&[("Notes.md", "upper\n"), ("notes.md", "desktop x\n"), ("x.md", "phone x\n")]);
        assert_eq!(linux.files(), want, "round {round}: the desktop keeps its rename and gets the phone's x.md");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// A desktop delete of a note whose rename the phone holds back, and which
/// is edited on the phone: the edit wins over the delete, as everywhere, so
/// the note comes back on the desktop with the phone's text, under its name
/// on the phone. The delete is not held.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn a_desktop_delete_of_a_held_note_edited_on_the_phone_brings_the_note_back_on_android_shared_storage() {
    let srv = server();
    let (mut linux, mut phone) = rename_to_a_case_twin_on_the_desktop(&srv);
    phone.write("draft.md", "line 1\nline 2\nphone line\n");
    let edited = [("Notes.md", "upper\n"), ("draft.md", "line 1\nline 2\nphone line\n")];
    rename_stays_held(&mut phone, &mut linux, &edited, &[("Notes.md", "upper\n"), ("notes.md", "line 1\nline 2\n")]);
    linux.rm("notes.md");
    linux.sync_ok();
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "round {round}: {:?} {:?}", r.skipped, r.conflicts);
        linux.sync_ok();
        assert_eq!(phone.files(), owned(&edited), "round {round}");
        assert_eq!(linux.files(), owned(&edited), "round {round}: the edit beats the delete");
    }
}

/// A rename the user makes in Cairn on the phone, of a note whose rename the
/// phone holds back: as on every device when the other device's name is
/// taken there, the user's name is kept and reaches every device.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn renaming_a_held_note_on_the_phone_gives_it_that_name_everywhere_on_android_shared_storage() {
    let srv = server();
    let (mut linux, mut phone) = rename_to_a_case_twin_on_the_desktop(&srv);
    let r = phone.sync_ok();
    assert!(r.skipped.iter().any(|s| s.path == "notes.md"), "the rename is held: {:?}", r.skipped);
    phone.app_mv("draft.md", "plan.md");
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "round {round}: {:?} {:?}", r.skipped, r.conflicts);
        linux.sync_ok();
        let want = owned(&[("Notes.md", "upper\n"), ("plan.md", "line 1\nline 2\n")]);
        assert_eq!(phone.files(), want, "round {round}");
        assert_eq!(linux.files(), want, "round {round}");
    }
}

/// A synced desktop with Notes.md and notes.md, and a phone on Android shared
/// storage that stores one of them and lists the other. Both also have
/// keep.md, so that the phone's vault folder is not left empty, which would
/// stop the sync (FINDING-006). Returns the twin the phone stores and the
/// other one.
fn case_twins_on_the_phone(srv: &Server) -> (Device, Device, (String, String), (String, String)) {
    let notes = owned(&[("Notes.md", "upper\n"), ("notes.md", "lower\n")]);
    let mut linux = Device::new(srv, "linux", &[("Notes.md", "upper\n"), ("notes.md", "lower\n"), ("keep.md", "keep\n")]);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[], saf_like());
    phone.sync_ok();
    let files: Vec<(String, String)> = phone.files().into_iter().filter(|f| f.0 != "keep.md").collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let other = notes.iter().find(|n| **n != files[0]).unwrap().clone();
    (linux, phone, files[0].clone(), other)
}

/// `files` and keep.md (see `case_twins_on_the_phone`), sorted.
fn and_keep(files: &[(String, String)]) -> Vec<(String, String)> {
    let mut v = files.to_vec();
    v.push(("keep.md".into(), "keep\n".into()));
    v.sort();
    v
}

/// FINDING-172, case 2: deleting the case twin the phone stores, in Cairn,
/// deletes it on every device, and the phone then stores the other one under
/// its own name. The phone does not take the other note in its place and
/// list the deleted one, which would keep the delete from reaching the other
/// devices.
/// Also when the upload of the delete fails, after the pull stored the other
/// note: the next sync uploads it.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn deleting_the_stored_case_twin_on_the_phone_deletes_it_on_every_device_on_android_shared_storage() {
    for interrupted in [false, true] {
        let srv = server();
        let (mut linux, mut phone, kept, other) = case_twins_on_the_phone(&srv);
        phone.vault.delete(&kept.0).unwrap();
        if interrupted {
            fail_puts(&mut phone);
            assert!(phone.sync().is_err(), "the upload of the delete fails");
            plain(&mut phone);
            assert_eq!(phone.files(), and_keep(std::slice::from_ref(&other)), "the pull stored the other note");
        }
        for round in 0..2 {
            let r = phone.sync_ok();
            assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "interrupted {interrupted}, round {round}: {:?} {:?}", r.skipped, r.conflicts);
            assert_eq!(phone.files(), and_keep(std::slice::from_ref(&other)), "interrupted {interrupted}, round {round}: the phone stores the other note");
            linux.sync_ok();
            assert_eq!(linux.files(), and_keep(std::slice::from_ref(&other)), "interrupted {interrupted}, round {round}: the delete reached the desktop");
            assert_eq!(linux.trash(), vec![kept.clone()], "interrupted {interrupted}, round {round}: the deleted note is in the desktop's trash");
        }
    }
}

/// The phone deletes the stored twin while the desktop edits it: the edit
/// wins over the delete, as everywhere. The phone stores the other note and
/// lists the edited one, which it cannot store next to it, and the desktop
/// keeps both.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn deleting_the_stored_case_twin_on_the_phone_while_the_desktop_edits_it_keeps_the_edit_on_android_shared_storage() {
    let srv = server();
    let (mut linux, mut phone, kept, other) = case_twins_on_the_phone(&srv);
    phone.vault.delete(&kept.0).unwrap();
    linux.write(&kept.0, "edited on the desktop\n");
    linux.sync_ok();
    let want = and_keep(&[(kept.0.clone(), "edited on the desktop\n".to_string()), other.clone()]);
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(
            r.skipped.iter().any(|s| s.path == kept.0 && s.reason.contains("differ only in case")),
            "round {round}: the edited note is listed: {:?}",
            r.skipped
        );
        assert_eq!(phone.files(), and_keep(std::slice::from_ref(&other)), "round {round}");
        linux.sync_ok();
        assert_eq!(linux.files(), want, "round {round}: the desktop keeps both notes");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// The same for the stored twin renamed outside Cairn on the phone: the
/// rename reaches every device. The other devices do not get the note under
/// its new name as a new note and keep the old one too.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn renaming_the_stored_case_twin_outside_cairn_on_the_phone_renames_it_on_every_device_on_android_shared_storage() {
    let srv = server();
    let (mut linux, mut phone, kept, other) = case_twins_on_the_phone(&srv);
    phone.mv(&kept.0, "Ideas.md");
    let want = and_keep(&[("Ideas.md".to_string(), kept.1.clone()), other.clone()]);
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "round {round}: {:?} {:?}", r.skipped, r.conflicts);
        assert_eq!(phone.files(), want, "round {round}");
        linux.sync_ok();
        assert_eq!(linux.files(), want, "round {round}: the rename reached the desktop, with no copy left under the old name");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// Cases 1 and 2 together: the phone deletes the note whose name a held
/// rename wants, after editing the renamed note. The delete and the edit
/// reach the desktop, and the rename stays.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn deleting_the_other_case_twin_on_the_phone_lets_a_held_rename_and_its_edit_through_on_android_shared_storage() {
    let srv = server();
    let (mut linux, mut phone) = rename_to_a_case_twin_on_the_desktop(&srv);
    phone.write("draft.md", "line 1\nline 2\nphone line\n");
    let edited = [("Notes.md", "upper\n"), ("draft.md", "line 1\nline 2\nphone line\n")];
    rename_stays_held(&mut phone, &mut linux, &edited, &[("Notes.md", "upper\n"), ("notes.md", "line 1\nline 2\n")]);
    phone.vault.delete("Notes.md").unwrap();
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.skipped.is_empty() && r.conflicts.is_empty(), "round {round}: {:?} {:?}", r.skipped, r.conflicts);
        linux.sync_ok();
        let want = owned(&[("notes.md", "line 1\nline 2\nphone line\n")]);
        assert_eq!(phone.files(), want, "round {round}");
        assert_eq!(linux.files(), want, "round {round}");
        assert_eq!(linux.trash(), owned(&[("Notes.md", "upper\n")]), "round {round}");
    }
}

/// FINDING-172, case 3: a note the phone already has, with the content of a
/// note elsewhere whose name differs from it only in case, is that note, and
/// takes its name on the phone. The phone does not keep its own spelling and
/// push it, which would rename the note on every device, also when the other
/// device has a second note whose name differs only in case (then the phone
/// lists that one).
#[test]
fn a_note_the_phone_has_under_another_spelling_takes_the_name_it_has_elsewhere_on_android_shared_storage() {
    for twins in [false, true] {
        // Case twins need a case-sensitive file system.
        if twins && !cfg!(target_os = "linux") {
            continue;
        }
        let srv = server();
        let desk: &[(&str, &str)] = if twins { &[("Notes.md", "upper\n"), ("notes.md", "lower\n")] } else { &[("Notes.md", "upper\n")] };
        let mut linux = Device::new(&srv, "linux", desk);
        linux.sync_ok();
        let mut phone = Device::new_with_fs(&srv.url, "phone", &[("NOTES.md", "upper\n")], saf_like());
        for round in 0..2 {
            let r = phone.sync_ok();
            assert!(r.conflicts.is_empty(), "twins {twins}, round {round}: {:?}", r.conflicts);
            assert_eq!(phone.files(), owned(&[("Notes.md", "upper\n")]), "twins {twins}, round {round}: the phone's copy takes the desktop's name");
            let listed: Vec<&str> = r.skipped.iter().map(|s| s.path.as_str()).collect();
            assert_eq!(listed, if twins { vec!["notes.md"] } else { vec![] }, "twins {twins}, round {round}: {:?}", r.skipped);
            linux.sync_ok();
            assert_eq!(linux.files(), owned(desk), "twins {twins}, round {round}: the desktop keeps its names");
            assert!(linux.trash().is_empty(), "twins {twins}, round {round}: {:?}", linux.trash());
        }
    }
}

/// If the phone cannot rename its copy, the desktop's note waits as not
/// synced and the phone's copy goes up as a note of its own. Nothing is
/// lost, and the desktop keeps its name.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn a_note_the_phone_cannot_rename_to_the_name_it_has_elsewhere_is_kept_as_its_own_on_android_shared_storage() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Notes.md", "upper\n")]);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[("NOTES.md", "upper\n")], saf_like());
    *phone.hookfs.decide.lock() = Some(Box::new(|op, _, _| if op == FsOp::Rename { FsAction::Fail } else { FsAction::Pass }));
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.skipped.iter().any(|s| s.path == "Notes.md"), "round {round}: {:?}", r.skipped);
        assert_eq!(phone.files(), owned(&[("NOTES.md", "upper\n")]), "round {round}");
        linux.sync_ok();
        assert_eq!(linux.files(), owned(&[("NOTES.md", "upper\n"), ("Notes.md", "upper\n")]), "round {round}: the desktop keeps its note");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// A note the phone has in a folder whose name differs only in case is not
/// taken for the desktop's note: the phone would have to rename the folder,
/// and with it every note in it, on every device (FINDING-034). The desktop's
/// note is listed as not synced, as any note in that folder is, and the
/// phone's goes up as a note of its own; the desktop keeps its names.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn a_note_the_phone_has_in_a_folder_spelled_another_way_does_not_rename_the_desktop_note_on_android_shared_storage() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Ideas/a.md", "same\n")]);
    linux.sync_ok();
    let mut phone = Device::new_with_fs(&srv.url, "phone", &[("IDEAS/a.md", "same\n")], saf_like());
    for round in 0..2 {
        let r = phone.sync_ok();
        assert!(r.skipped.iter().any(|s| s.path == "Ideas/a.md"), "round {round}: {:?}", r.skipped);
        assert_eq!(phone.files(), owned(&[("IDEAS/a.md", "same\n")]), "round {round}");
        linux.sync_ok();
        assert_eq!(linux.files(), owned(&[("IDEAS/a.md", "same\n"), ("Ideas/a.md", "same\n")]), "round {round}: the desktop keeps its note");
        assert!(linux.trash().is_empty(), "round {round}: {:?}", linux.trash());
    }
}

/// A state a client without case-twin handling can leave behind on a
/// case-insensitive device when its push failed after the pull: `Note.md`
/// and `note.md` are both tracked, but they are one file on disk, which now
/// holds the lower case note. The next sync must not push a delete of
/// `note.md`, nor the lower case note as an edit of `Note.md`.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn two_tracked_files_that_are_one_file_on_disk_are_downloaded_again() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Note.md", "upper case note\n"), ("note.md", "lower case note\n")]);
    linux.sync_ok();
    let mut mac = Device::new(&srv, "mac", &[]);
    mac.sync_ok();
    mac.rm("note.md");
    mac.write("Note.md", "lower case note\n");
    mac.fs_maker = case_insensitive();
    mac.restart();
    let r = mac.sync_ok();
    converge(&mut mac, &mut linux);
    for d in [&mac, &linux] {
        let live: Vec<String> = d.files().into_iter().map(|f| f.1).collect();
        assert!(live.contains(&"upper case note\n".to_string()), "{}: {:?}", d.name, d.files());
        assert!(live.contains(&"lower case note\n".to_string()), "{}: {:?}", d.name, d.files());
    }
    assert!(linux.trash().is_empty(), "{:?}", linux.trash());
    // both come back under conflict copy names
    assert_eq!(r.conflicts.len(), 2, "{:?}", r.conflicts);
}

/// A case-only rename with an edit, made outside Cairn on a
/// case-insensitive device, is pushed as a rename, not as a delete and a
/// new file.
#[test]
fn case_only_rename_with_edit_on_a_case_insensitive_device_keeps_the_file() {
    let srv = server();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[("meeting.md", "notes\n")], case_insensitive());
    mac.sync_ok();
    let mut linux = Device::new(&srv, "linux", &[]);
    linux.sync_ok();
    mac.mv("meeting.md", "Meeting.md");
    mac.write("Meeting.md", "notes\nmore\n");
    mac.sync_ok();
    linux.sync_ok();
    assert_eq!(linux.files(), vec![("Meeting.md".to_string(), "notes\nmore\n".to_string())]);
    assert!(linux.trash().is_empty(), "pushed as a delete: {:?}", linux.trash());
}

#[test]
fn case_only_rename_on_a_case_insensitive_device_syncs() {
    let srv = server();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[("meeting.md", "notes\n")], case_insensitive());
    mac.sync_ok();
    let mut linux = Device::new(&srv, "linux", &[]);
    linux.sync_ok();
    linux.mv("meeting.md", "Meeting.md");
    linux.sync_ok();
    mac.sync_ok();
    converge(&mut mac, &mut linux);
    assert_eq!(linux.files(), vec![("Meeting.md".to_string(), "notes\n".to_string())]);
    assert_eq!(mac.files().len(), 1, "{:?}", mac.files());
}

/// A case-only rename with an edit on a case-insensitive device, against an
/// edit of the same note on another device: the edits are merged as for
/// any edited file, and the device keeps syncing.
#[test]
fn case_only_rename_with_edit_merges_a_remote_edit_on_a_case_insensitive_device() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("meeting.md", "1\n2\n3\n")]);
    linux.sync_ok();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[], case_insensitive());
    mac.sync_ok();
    mac.mv("meeting.md", "Meeting.md");
    mac.write("Meeting.md", "1 mac\n2\n3\n");
    linux.write("meeting.md", "1\n2\n3 linux\n");
    linux.sync_ok();
    mac.write("new_on_mac.md", "new\n");
    let r = mac.sync_ok();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    linux.sync_ok();
    let want = [("Meeting.md", "1 mac\n2\n3 linux\n"), ("new_on_mac.md", "new\n")].map(|(p, c)| (p.to_string(), c.to_string()));
    assert_eq!(linux.files(), want);
    assert_eq!(mac.files(), want);
    assert!(linux.trash().is_empty(), "{:?}", linux.trash());
}

/// The same, made in the app and with edits that do not merge: the remote
/// edit becomes a conflict copy.
#[test]
fn case_only_rename_with_edit_keeps_both_edits_when_they_do_not_merge() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("meeting.md", "1\n2\n3\n")]);
    linux.sync_ok();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[], case_insensitive());
    mac.sync_ok();
    mac.vault.rename("meeting.md", "Meeting.md").unwrap();
    mac.vault.write_file("Meeting.md", b"1 mac\n2\n3\n", None).unwrap();
    linux.write("meeting.md", "1 linux\n2\n3\n");
    linux.sync_ok();
    let r = mac.sync_ok();
    assert_eq!(r.conflicts.len(), 1, "{:?}", r.conflicts);
    converge(&mut mac, &mut linux);
    for d in [&mac, &linux] {
        let files = d.files();
        assert!(files.contains(&("Meeting.md".to_string(), "1 mac\n2\n3\n".to_string())), "{}: {:?}", d.name, files);
        assert!(files.iter().any(|f| f.1 == "1 linux\n2\n3\n"), "{}: {:?}", d.name, files);
        assert_eq!(files.len(), 2, "{}: {:?}", d.name, files);
    }
    assert!(linux.trash().is_empty(), "{:?}", linux.trash());
}

/// The other device renamed and edited the note too: its name wins, as
/// when both rename, and the edits are merged.
#[test]
fn case_only_rename_with_edit_against_a_remote_rename_with_edit_merges() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("meeting.md", "1\n2\n3\n")]);
    linux.sync_ok();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[], case_insensitive());
    mac.sync_ok();
    mac.mv("meeting.md", "Meeting.md");
    mac.write("Meeting.md", "1 mac\n2\n3\n");
    linux.mv("meeting.md", "notes.md");
    linux.sync_ok();
    linux.write("notes.md", "1\n2\n3 linux\n");
    linux.sync_ok();
    let r = mac.sync_ok();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    converge(&mut mac, &mut linux);
    assert_eq!(linux.files(), vec![("notes.md".to_string(), "1 mac\n2\n3 linux\n".to_string())]);
    assert!(linux.trash().is_empty(), "{:?}", linux.trash());
}

/// The user edits the renamed note while the sync pulls: the merge is
/// tried again with that edit, and the case-only rename is kept.
#[test]
fn case_only_rename_with_edit_merges_again_after_an_edit_during_the_pull() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("meeting.md", "1\n2\n3\n4\n5\n")]);
    linux.sync_ok();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[], case_insensitive());
    mac.sync_ok();
    mac.mv("meeting.md", "Meeting.md");
    mac.write("Meeting.md", "1 mac\n2\n3\n4\n5\n");
    linux.write("meeting.md", "1\n2\n3 linux\n4\n5\n");
    linux.sync_ok();
    during_pull(&mut mac, |d| d.write("Meeting.md", "1 mac\n2\n3\n4\n5 mac again\n"));
    let r = mac.sync_ok();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    plain(&mut mac);
    converge(&mut mac, &mut linux);
    assert_eq!(linux.files(), vec![("Meeting.md".to_string(), "1 mac\n2\n3 linux\n4\n5 mac again\n".to_string())]);
    assert!(linux.trash().is_empty(), "{:?}", linux.trash());
}

/// The same note created on both devices under names that differ only in
/// case is one note on a case-insensitive device: it is adopted, not
/// copied, and the device's name is kept.
#[test]
fn same_note_created_under_names_differing_in_case_is_adopted() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("todo.md", "same\n")]);
    linux.sync_ok();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[("Todo.md", "same\n")], case_insensitive());
    let r = mac.sync_ok();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    converge(&mut mac, &mut linux);
    assert_eq!(linux.files(), vec![("Todo.md".to_string(), "same\n".to_string())]);
    assert!(linux.trash().is_empty(), "{:?}", linux.trash());
}

/// Two remote notes with the same content, whose names differ only in
/// case, are not both adopted as the one local file.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn two_same_notes_differing_in_case_are_not_adopted_as_one() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("todo.md", "same\n"), ("TODO.md", "same\n")]);
    linux.sync_ok();
    let mut mac = Device::new_with_fs(&srv.url, "mac", &[("Todo.md", "same\n")], case_insensitive());
    mac.sync_ok();
    converge(&mut mac, &mut linux);
    assert_eq!(linux.files().len(), 2, "{:?}", linux.files());
    assert!(linux.trash().is_empty(), "{:?}", linux.trash());
    let mut live: Vec<String> = mac.engine().state().files.values().filter(|t| !t.deleted).map(|t| t.path.clone()).collect();
    live.sort();
    live.dedup();
    assert_eq!(live.len(), 2, "{live:?}");
}

/// A file system whose next listing of one file's folder misses it, as when
/// another program saves the file by deleting and creating it again.
struct MissOnceFs {
    inner: cairn_core::StdFs,
    path: String,
    armed: Arc<std::sync::atomic::AtomicBool>,
}

impl cairn_core::VaultFs for MissOnceFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<cairn_core::FileStat>> {
        let mut out = self.inner.list(dir)?;
        if out.iter().any(|s| s.path == self.path) && self.armed.swap(false, Ordering::SeqCst) {
            out.retain(|s| s.path != self.path);
        }
        Ok(out)
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<cairn_core::FileStat>> {
        self.inner.stat(path)
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.inner.read(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<cairn_core::FileStat> {
        self.inner.write(path, data)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.remove(path)
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
}

/// On a case-sensitive device, a note that one scan misses while a note
/// whose name differs only in case is there is not taken for the same file.
#[cfg_attr(not(target_os = "linux"), ignore = "case twins need a case-sensitive file system")]
#[test]
fn note_missed_by_a_scan_next_to_a_case_variant_is_not_a_case_twin() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Note.md", "upper\n"), ("note.md", "lower case\n")]);
    linux.sync_ok();
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a = armed.clone();
    linux.fs_maker = Some(Arc::new(move |root: &std::path::Path| {
        let inner = cairn_core::StdFs::new(root, cairn_core::TrashMode::Vault).unwrap();
        Arc::new(MissOnceFs { inner, path: "note.md".into(), armed: a.clone() }) as Arc<dyn cairn_core::VaultFs>
    }));
    linux.restart();
    armed.store(true, Ordering::SeqCst);
    let r = linux.sync_ok();
    assert!(!armed.load(Ordering::SeqCst), "the scan did not miss the note");
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    linux.sync_ok();
    assert_eq!(linux.files(), [("Note.md", "upper\n"), ("note.md", "lower case\n")].map(|(p, c)| (p.to_string(), c.to_string())));
}

#[test]
fn interrupted_sync_after_local_rename_then_remote_edit_keeps_the_rename() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Draft.md", "line 1\nline 2\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("Draft.md", "line 1\nline 2 phone\n");
    b.sync_ok();
    a.mv("Draft.md", "Projects/Final.md");
    fail_puts(&mut a);
    assert!(a.sync().is_err());
    plain(&mut a);
    a.sync_ok();
    // later the phone edits the note again
    b.write("Draft.md", "line 1\nline 2 phone\nline 3 phone\n");
    b.sync_ok();
    a.sync_ok();
    assert_eq!(a.paths(), vec!["Projects/Final.md"], "the laptop user's rename was silently undone");
}
