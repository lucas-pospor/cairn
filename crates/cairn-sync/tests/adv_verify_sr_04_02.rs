//! Further regression tests for FINDING-015: how wide
//! was the overwrite window once the desktop file watcher is taken into
//! account?
//!
//! The desktop app runs a notify debouncer (app/src-tauri/src/watcher.rs,
//! 250 ms) that calls `Vault::rescan_paths` for externally changed paths.
//! Once the index knows the new file, `SyncEngine::exists()` returns true and
//! the incoming remote file gets a conflict name instead of overwriting it
//! (`apply_remote`'s new-file branch). So the overwrite needed the external create to land
//! after the round's scan AND less than one debounce before the apply of the
//! head with the same path.
//!
//! Model used here: the external write happens right after the scan (in the
//! transport hook for the `changes` call); a "watcher" thread calls
//! `rescan_paths` 250 ms later, like the debouncer callback; the `changes`
//! call is delayed by a simulated network round trip.
//!
//! Cases (in each, the user's text must survive):
//! * fast network (round trip shorter than the debounce, i.e. a LAN or a
//!   nearby server): the watcher never gets there first, so the WHOLE
//!   scan-to-apply interval was exposed;
//! * slow network (round trip longer than the debounce): the watcher alone
//!   would save the file;
//! * no watcher (Android, the sync_dir CLI): the whole interval was
//!   exposed regardless of latency.
//!
//!   cargo test -p cairn-sync --test adv_verify_sr_04_02 -- --nocapture --test-threads=1

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use common::*;
use parking_lot::Mutex;

const DEBOUNCE: Duration = Duration::from_millis(250);
const USER_TEXT: &str = "laptop minutes - typed in another editor, only copy\n";

fn synced_pair(files: &[(&str, &str)]) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", files);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    (srv, a, b)
}

/// (path, content) pairs.
type Files = Vec<(String, String)>;

/// B creates Meeting.md; A syncs while an external program creates its own
/// Meeting.md right after A's scan. Returns (A files, A trash, does the
/// user's text survive anywhere on A or B after convergence).
fn race(rtt: Duration, watcher: bool) -> (Files, Files, bool) {
    let (_srv, mut a, mut b) = synced_pair(&[("x.md", "x\n")]);
    b.write("Meeting.md", "phone agenda\n");
    b.sync_ok();

    let root = a.root.clone();
    let vault = a.vault.clone();
    let watcher_thread: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));
    let wt = watcher_thread.clone();
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    let mut fired = false;
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op != "changes" || fired {
            return;
        }
        fired = true;
        std::thread::sleep(Duration::from_millis(3));
        std::fs::write(root.join("Meeting.md"), USER_TEXT).unwrap();
        if watcher {
            let v = vault.clone();
            *wt.lock() = Some(std::thread::spawn(move || {
                std::thread::sleep(DEBOUNCE);
                v.rescan_paths(&["Meeting.md".to_string()]).unwrap();
            }));
        }
        // the changes request is in flight
        std::thread::sleep(rtt);
    }));
    a.set_transport(Box::new(SharedTransport(t)));
    a.sync_ok();
    if let Some(h) = watcher_thread.lock().take() {
        h.join().unwrap();
    }
    let files = a.files();
    let trash = a.trash();
    let url = a.url.clone();
    a.set_transport(http(&url));
    converge(&mut a, &mut b);
    let all = format!("{}{}", a.all_text(), b.all_text());
    (files, trash, all.contains(USER_TEXT))
}

#[test]
fn fast_network_with_watcher_keeps_the_file() {
    let (files, trash, survives) = race(Duration::from_millis(20), true);
    println!("rtt 20 ms, watcher on: A files {files:?}; trash {trash:?}; survives {survives}");
    assert!(survives, "user's note overwritten despite the watcher: A files {files:?}; trash {trash:?}");
}

#[test]
fn desktop_watcher_saves_the_file_on_a_slow_network() {
    let (files, trash, survives) = race(Duration::from_millis(700), true);
    println!("rtt 700 ms, watcher on: A files {files:?}; trash {trash:?}; survives {survives}");
    assert!(survives, "A files {files:?}; trash {trash:?}");
}

#[test]
fn no_watcher_slow_network_does_not_overwrite() {
    let (files, trash, survives) = race(Duration::from_millis(700), false);
    println!("rtt 700 ms, no watcher: A files {files:?}; trash {trash:?}; survives {survives}");
    assert!(survives, "user's note overwritten: A files {files:?}; trash {trash:?}");
}

/// Same race, but the external program writes the SAME bytes the remote
/// file has (e.g. a second sync tool delivering the same note): nothing is
/// lost, and no conflict copy is made. With the defect, this showed that
/// the loss needed two different contents under one new name.
#[test]
fn same_content_external_create_loses_nothing() {
    let (_srv, mut a, mut b) = synced_pair(&[("x.md", "x\n")]);
    b.write("Meeting.md", "phone agenda\n");
    b.sync_ok();
    let root = a.root.clone();
    let t = Arc::new(FaultTransport::passthrough(http(&a.url)));
    let mut fired = false;
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "changes" && !fired {
            fired = true;
            std::thread::sleep(Duration::from_millis(3));
            std::fs::write(root.join("Meeting.md"), "phone agenda\n").unwrap();
        }
    }));
    a.set_transport(Box::new(SharedTransport(t)));
    a.sync_ok();
    let url = a.url.clone();
    a.set_transport(http(&url));
    converge(&mut a, &mut b);
    assert_eq!(a.read("Meeting.md").as_deref(), Some("phone agenda\n"));
    assert!(conflict_copies(&a.files()).is_empty(), "{:?}", a.files());
}
