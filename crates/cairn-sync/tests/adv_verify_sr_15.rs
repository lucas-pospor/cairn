//! Regression tests for FINDING-058 along the path the app actually offers.
//!
//! In the app, a configured device has no "connect" form: the only way to
//! reconnect is Settings > Sync > "Turn off" (SyncEngine::disconnect, which
//! deletes the whole sync state folder) and then "Connect and sync". That
//! path does re-upload. The silent part was on every OTHER device: as soon
//! as one device had re-created the vault, their syncs could succeed again
//! while their changes-feed cursor still pointed into the old database, so
//! they would skip the new server's revisions until its seq numbers passed the
//! old cursor. So a device whose cursor is ahead of the server's head now stops
//! with an error that tells the user to turn sync off and connect again.
//!
//! Run: cargo test -p cairn-sync --test adv_verify_sr_15 -- --nocapture

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use cairn_sync::engine::SyncEngine;
use common::*;

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn server_bin() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/cairn-server");
    assert!(p.exists(), "build cairn-server first: {}", p.display());
    p
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn spawn_server(data: &std::path::Path, port: u16) -> Proc {
    let child = Command::new(server_bin())
        .env("CAIRN_TOKENS", TOKEN)
        .env("CAIRN_DATA", data)
        .env("CAIRN_ADDR", format!("127.0.0.1:{port}"))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let t = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(t.elapsed() < Duration::from_secs(10), "server did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    Proc(child)
}

fn wipe(data: &std::path::Path) {
    for f in fs::read_dir(data).unwrap() {
        let p = f.unwrap().path();
        if p.is_dir() {
            fs::remove_dir_all(p).unwrap();
        } else {
            fs::remove_file(p).unwrap();
        }
    }
}

/// What Settings > Sync > "Turn off" and then "Connect and sync" do.
fn turn_off_and_connect(d: &mut Device, url: &str) {
    d.engine = None;
    SyncEngine::disconnect(&d.state_dir).unwrap();
    d.engine = Some(SyncEngine::connect_with(d.vault.clone(), &d.state_dir, settings(url, &d.name), PASS, http(url), FAST_KDF).unwrap());
}

/// Two devices in normal use, so their cursors are above the number of files.
fn two_devices_in_use(url: &str) -> (Device, Device) {
    let mut a = Device::new_url(url, "laptop", &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n")]);
    a.sync_ok();
    let mut b = Device::new_url(url, "phone", &[]);
    b.sync_ok();
    for i in 0..6 {
        a.write("a.md", &format!("a v{i}\n"));
        a.sync_ok();
        b.sync_ok();
    }
    assert_eq!(a.files(), b.files());
    (a, b)
}

#[test]
fn after_a_server_reset_the_device_that_did_not_reconnect_is_told_to_reconnect() {
    let data = tempfile::tempdir().unwrap();
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let srv = spawn_server(data.path(), port);
    let (mut a, mut b) = two_devices_in_use(&url);
    let cursor_b = b.engine().state().last_seq;

    // the server comes back without its data
    drop(srv);
    wipe(data.path());
    let _srv = spawn_server(data.path(), port);
    let ea = a.sync().err().map(|e| e.to_string());
    let eb = b.sync().err().map(|e| e.to_string());
    eprintln!("after the reset: laptop {ea:?}, phone {eb:?}");
    assert!(ea.is_some() && eb.is_some(), "both devices report the missing vault");

    // the user fixes the laptop the only way the app offers
    turn_off_and_connect(&mut a, &url);
    let r = a.sync_ok();
    eprintln!("laptop after turn off + connect: pulled {} pushed {}", r.pulled, r.pushed);

    // the phone's background sync no longer fails
    let r = b.sync();
    eprintln!("phone next sync: {:?}", r.as_ref().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string()));

    a.write("after-reset.md", "written on the laptop after the reset\n");
    a.sync_ok();
    let r = b.sync();
    eprintln!(
        "phone (cursor {cursor_b}) after the laptop wrote a new note: {:?}; phone has {:?}",
        r.as_ref().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string()),
        b.paths()
    );
    // FINDING-058, by design: not silent, and nothing is reset
    // behind the user's back
    let err = r.err().map(|e| e.to_string()).unwrap_or_else(|| {
        panic!("the phone says it is in sync, but its cursor {cursor_b} is above the new server's seqs; phone has {:?}", b.paths())
    });
    assert!(err.contains("Turn sync off in Settings > Sync and connect again"), "{err}");
    // the user does what the error says
    turn_off_and_connect(&mut b, &url);
    b.sync_ok();
    assert!(b.read("after-reset.md").is_some(), "phone has {:?}", b.paths());
    converge(&mut a, &mut b);
    assert!(conflict_copies(&b.files()).is_empty(), "{:?}", b.paths());
}

#[test]
fn after_a_server_reset_edits_on_the_stale_device_wait_for_a_reconnect() {
    let data = tempfile::tempdir().unwrap();
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let srv = spawn_server(data.path(), port);
    let (mut a, mut b) = two_devices_in_use(&url);
    drop(srv);
    wipe(data.path());
    let _srv = spawn_server(data.path(), port);
    turn_off_and_connect(&mut a, &url);
    a.sync_ok();
    b.write("b.md", "edited on the phone after the reset\n");
    let errs: Vec<String> = (0..3).filter_map(|_| b.sync().err().map(|e| e.to_string())).collect();
    eprintln!("phone edit errors: {errs:?}");
    // FINDING-058, by design: every sync fails until the user
    // connects again, and the error says so (instead of blaming "the server
    // kept changing" forever)
    assert!(errs.len() == 3 && errs.iter().all(|e| e.contains("Turn sync off in Settings > Sync and connect again")), "{errs:?}");
    turn_off_and_connect(&mut b, &url);
    b.sync_ok();
    converge(&mut a, &mut b);
    assert!(a.all_text().contains("edited on the phone after the reset"), "the phone's edit is lost: {:?}", a.files());
}

/// Control: turning sync off and on on EVERY device recovers fully.
#[test]
fn after_a_server_reset_turning_sync_off_and_on_everywhere_recovers() {
    let data = tempfile::tempdir().unwrap();
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let srv = spawn_server(data.path(), port);
    let (mut a, mut b) = two_devices_in_use(&url);
    drop(srv);
    wipe(data.path());
    let _srv = spawn_server(data.path(), port);
    turn_off_and_connect(&mut a, &url);
    a.sync_ok();
    turn_off_and_connect(&mut b, &url);
    let r = b.sync_ok();
    eprintln!("phone after turn off + connect: pulled {} pushed {} conflicts {:?}", r.pulled, r.pushed, r.conflicts);
    a.write("after-reset.md", "x\n");
    b.write("phone-new.md", "y\n");
    converge(&mut a, &mut b);
    assert!(conflict_copies(&a.files()).is_empty(), "{:?}", a.paths());
    assert!(b.read("after-reset.md").is_some() && a.read("phone-new.md").is_some());
}
