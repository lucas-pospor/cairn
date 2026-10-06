//! Adversarial sync robustness: large files near the limits, network
//! faults from a fake HTTP server, corrupt server data, and killing the real
//! cairn-server process (kill -9) in the middle of a sync.
//!
//! Run: cargo test -p cairn-sync --test adv_sync_robust_net
//! Slow tests (about a minute each): add `-- --ignored slow_`
//! One test: cargo test -p cairn-sync --test adv_sync_robust_net -- --exact <name>

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;

fn bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed | 1);
    (0..n).map(|_| r.next() as u8).collect()
}

// ------------------------------------------------------------------ size limits

#[test]
fn an_8mb_attachment_reaches_the_other_device() {
    let srv = server(); // max_body 50 MB, the default is 200 MB
    let mut a = Device::new(&srv, "laptop", &[("note.md", "see scan\n")]);
    a.write_bytes("scan.pdf", &bytes(8 << 20, 7));
    let up = a.sync_ok();
    assert_eq!(up.pushed, 2, "upload accepted by the server");
    let mut b = Device::new(&srv, "phone", &[]);
    let r = b.sync();
    // the uploader's next sync, which used to pull its own heads again
    let again = a.sync();
    assert!(
        r.is_ok() && again.is_ok(),
        "second device: {:?}; uploader's next sync: {:?}",
        r.err().map(|e| e.to_string()),
        again.err().map(|e| e.to_string())
    );
}

#[test]
fn a_vault_with_many_photos_can_be_pulled_by_a_new_device() {
    // 40 photos of 300 KB: no file is large, but one 500-head page is ~16 MB
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("index.md", "photos\n")]);
    for i in 0..40 {
        a.write_bytes(&format!("photos/img{i:02}.jpg"), &bytes(300 << 10, i));
    }
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    let r = b.sync();
    assert!(r.is_ok(), "new device cannot do its first sync: {}", r.err().unwrap());
    assert_eq!(b.files().len(), 41);
}

#[test]
fn a_6mb_attachment_syncs() {
    // just under ureq's default 10 MB response limit (base64 of 6 MB + JSON)
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[]);
    a.write_bytes("a.bin", &bytes(6 << 20, 3));
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    assert_eq!(fs::read(b.root.join("a.bin")).unwrap(), bytes(6 << 20, 3));
}

#[test]
fn a_file_over_the_upload_limit_does_not_block_other_files() {
    let srv = server_with_body(1 << 20); // CAIRN_MAX_BODY_MB=1
    let mut a = Device::new(&srv, "laptop", &[("a.md", "a\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.write_bytes("big.mov", &bytes(900 << 10, 1)); // 900 KB: base64+JSON is over 1 MB
    a.write("notes/today.md", "written after the video\n");
    a.write("zz.md", "sorted last\n");
    let mut errors = Vec::new();
    for _ in 0..3 {
        if let Err(e) = a.sync() {
            errors.push(e.to_string());
        }
    }
    b.sync_ok();
    assert!(
        b.read("notes/today.md").is_some() && b.read("zz.md").is_some(),
        "B has {:?}; A's sync errors: {:?}",
        b.paths(),
        errors.first()
    );
}

#[test]
fn a_server_that_cannot_store_uploads_fails_the_sync() {
    // Only an upload's own failure (too large, stalled, cut off) skips the
    // file. A server error would fail every upload, so it must show as a
    // sync error, not as a successful sync that pushed nothing.
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "a\n")]);
    a.sync_ok();
    a.write("b.md", "b\n");
    a.write("c.md", "c\n");
    // another writer holds the database: every upload gets HTTP 500
    let lock = rusqlite::Connection::open(&srv.db_path).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let r = a.sync();
    lock.execute_batch("ROLLBACK").unwrap();
    match r {
        Ok(r) => panic!("sync succeeded: pushed {}, skipped {:?}", r.pushed, r.skipped),
        Err(e) => assert!(e.to_string().contains("HTTP 500"), "{e}"),
    }
    assert_eq!(a.sync_ok().pushed, 2);
}

// ------------------------------------------------------------------ a pull larger than one batch

/// The cursor in a device's saved sync state.
fn saved_cursor(d: &Device) -> u64 {
    let state = fs::read(d.state_file()).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    state.and_then(|s| s["last_seq"].as_u64()).unwrap_or(0)
}

/// Twelve 30 KB photos on the server, a new device that pulls two records
/// a page in batches of about 60 KB of records: each page (about 80 KB) is
/// a batch.
fn photos_and_a_batched_new_device() -> (Server, Device, Arc<FaultTransport>) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[]);
    for i in 0..12 {
        a.write_bytes(&format!("photos/{i:02}.jpg"), &bytes(30 << 10, 40 + i));
    }
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    let t = b.pull_in_batches(2, 60 << 10);
    (srv, b, t)
}

/// A first sync of a large vault does not hold the whole vault in memory:
/// each batch of the feed is written and recorded before the next is read.
#[test]
fn a_large_pull_is_applied_batch_by_batch() {
    let (_srv, mut b, t) = photos_and_a_batched_new_device();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (seen2, root, state) = (seen.clone(), b.root.clone(), b.state_dir.clone());
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "changes" {
            let photos = fs::read_dir(root.join("photos")).map(|d| d.count()).unwrap_or(0);
            let cursor = fs::read(state.join("state.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
            seen2.lock().unwrap().push((photos, cursor.and_then(|s| s["last_seq"].as_u64()).unwrap_or(0)));
        }
    }));
    b.sync_ok();
    // (photos on disk, saved cursor) before each page is read
    let want: Vec<(usize, u64)> = (0..6).map(|k| (2 * k, 2 * k as u64)).collect();
    assert_eq!(*seen.lock().unwrap(), want);
    for i in 0..12 {
        assert_eq!(fs::read(b.root.join(format!("photos/{i:02}.jpg"))).unwrap(), bytes(30 << 10, 40 + i));
    }
}

/// A first sync cut off midway goes on from the last batch it applied.
#[test]
fn a_large_pull_cut_off_midway_goes_on_from_the_last_batch() {
    let (_srv, mut b, t) = photos_and_a_batched_new_device();
    let mut pages = 0;
    *t.decide.lock() = Box::new(move |op, _| {
        if op == "changes" {
            pages += 1;
            if pages == 4 {
                return Fault::ErrBefore;
            }
        }
        Fault::None
    });
    assert!(b.sync().is_err());
    assert_eq!(saved_cursor(&b), 6);
    assert_eq!(b.paths().len(), 6, "{:?}", b.paths());
    let before = t.blob_bytes_in.load(Ordering::SeqCst);
    b.sync_ok();
    assert_eq!(b.paths().len(), 12);
    // only the other half was downloaded (the same size, give or take the
    // records' lengths)
    let again = t.blob_bytes_in.load(Ordering::SeqCst) - before;
    assert!(again < before + before / 10, "pulled {again} bytes after {before}");
}

/// A file that takes the place of a folder reaches the server before the
/// deletion of the folder's last note (uploads go in path order). When a
/// batch ends between the two, the file waits for the next batch, which
/// removes the folder first, and arrives in the same sync (FINDING-084).
#[test]
fn a_file_replacing_a_folder_arrives_when_a_batch_ends_between_them() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Old/x.md", "x\n"), ("keep.md", "k\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    fs::remove_dir_all(a.root.join("Old")).unwrap();
    a.write("Old", "now a file\n");
    a.sync_ok();
    b.pull_in_batches(1, 1);
    let r = b.sync_ok();
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    assert_eq!(b.files(), [("Old".to_string(), "now a file\n".to_string()), ("keep.md".to_string(), "k\n".to_string())]);
}

/// The laptop moves a note away, writes a new one under its name, then
/// edits the moved note. The feed lists each file at its last change, so
/// the new note comes first: after its batch, the phone, which moved the
/// note too, tracks both at that name until a later batch brings the move.
/// A pull that stops there must not take the new note for an edit of the
/// moved one when it goes on.
#[test]
fn a_pull_stopped_between_batches_goes_on_without_mixing_up_files() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "first note\n"), ("keep.md", "k\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.mv("n.md", "moved here.md");
    a.mv("n.md", "z/moved there.md");
    a.sync_ok();
    a.write("n.md", "second note\n");
    a.sync_ok();
    a.write("z/moved there.md", "first note\nedited\n");
    a.sync_ok();
    let t = b.pull_in_batches(1, 1);
    let mut pages = 0;
    *t.decide.lock() = Box::new(move |op, _| {
        if op == "changes" {
            pages += 1;
            if pages == 2 {
                return Fault::ErrBefore;
            }
        }
        Fault::None
    });
    assert!(b.sync().is_err());
    assert_eq!(b.read("n.md").as_deref(), Some("second note\n"), "{:?}", b.paths());
    b.pull = None;
    b.restart();
    converge(&mut a, &mut b);
    let want = [
        ("keep.md".to_string(), "k\n".to_string()),
        ("n.md".to_string(), "second note\n".to_string()),
        ("z/moved there.md".to_string(), "first note\nedited\n".to_string()),
    ];
    assert_eq!(b.files(), want);
}

/// The same, but the user edits the new note before the sync goes on: then
/// neither tracked file has the content at that name, and it is still the
/// new note, the one the pull wrote there last.
#[test]
fn a_pull_stopped_between_batches_goes_on_when_the_new_note_was_edited() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "first note\n"), ("keep.md", "k\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.mv("n.md", "moved here.md");
    a.mv("n.md", "z/moved there.md");
    a.sync_ok();
    a.write("n.md", "second note\n");
    a.sync_ok();
    a.write("z/moved there.md", "first note\nedited\n");
    a.sync_ok();
    let t = b.pull_in_batches(1, 1);
    let mut pages = 0;
    *t.decide.lock() = Box::new(move |op, _| {
        if op == "changes" {
            pages += 1;
            if pages == 2 {
                return Fault::ErrBefore;
            }
        }
        Fault::None
    });
    assert!(b.sync().is_err());
    assert_eq!(b.read("n.md").as_deref(), Some("second note\n"), "{:?}", b.paths());
    b.write("n.md", "second note\nmore on the phone\n");
    b.pull = None;
    b.restart();
    converge(&mut a, &mut b);
    let want = [
        ("keep.md".to_string(), "k\n".to_string()),
        ("n.md".to_string(), "second note\nmore on the phone\n".to_string()),
        ("z/moved there.md".to_string(), "first note\nedited\n".to_string()),
    ];
    assert_eq!(b.files(), want);
    assert!(a.trash().is_empty() && b.trash().is_empty(), "{:?} {:?}", a.trash(), b.trash());
}

/// Before the phone reads page `page` of the feed, the laptop runs `edit`
/// and syncs it, so that a file's head moves while the phone's pull is
/// between two batches. The laptop comes back when that has happened.
fn laptop_syncs_during_pull(t: &FaultTransport, page: usize, a: Device, edit: impl FnOnce(&Device) + Send + 'static) -> std::sync::mpsc::Receiver<Device> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (mut a, mut edit, mut pages) = (Some(a), Some(edit), 0);
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "changes" {
            pages += 1;
            if pages == page {
                let mut a = a.take().unwrap();
                (edit.take().unwrap())(&a);
                a.sync_ok();
                tx.send(a).unwrap();
            }
        }
    }));
    rx
}

/// The phone deletes a note that the laptop edits, and the laptop edits it
/// again while the phone's pull is between two batches: the phone gets two
/// heads of the note in one sync. The newer edit takes the place of the
/// older one, as it does when both come in one batch; it is not taken for a
/// conflict with it.
#[test]
fn a_note_deleted_here_and_edited_twice_during_a_batched_pull_keeps_the_newest_edit() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "v0\n"), ("keep.md", "k\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.rm("n.md");
    a.write("n.md", "v1\n");
    a.sync_ok();
    a.write("x.md", "x\n");
    a.sync_ok();
    let t = b.pull_in_batches(1, 1);
    let laptop = laptop_syncs_during_pull(&t, 2, a, |a| a.write("n.md", "v2\n"));
    let r = b.sync_ok();
    assert!(r.conflicts.is_empty(), "{r:?}");
    let mut a = laptop.recv().unwrap();
    converge(&mut a, &mut b);
    let want = [
        ("keep.md".to_string(), "k\n".to_string()),
        ("n.md".to_string(), "v2\n".to_string()),
        ("x.md".to_string(), "x\n".to_string()),
    ];
    assert_eq!(b.files(), want);
}

/// The phone renames a photo that the laptop replaces, and the laptop
/// replaces it again while the phone's pull is between two batches. A file
/// that cannot be merged keeps the newest version under the phone's name.
#[test]
fn a_file_renamed_here_and_replaced_twice_during_a_batched_pull_keeps_the_newest_version() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("photo.bin", "v0"), ("keep.md", "k\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.mv("photo.bin", "moved.bin");
    a.write("photo.bin", "v1");
    a.sync_ok();
    a.write("x.md", "x\n");
    a.sync_ok();
    let t = b.pull_in_batches(1, 1);
    let laptop = laptop_syncs_during_pull(&t, 2, a, |a| a.write("photo.bin", "v2"));
    let r = b.sync_ok();
    assert!(r.conflicts.is_empty(), "{r:?}");
    let mut a = laptop.recv().unwrap();
    converge(&mut a, &mut b);
    let want = [
        ("keep.md".to_string(), "k\n".to_string()),
        ("moved.bin".to_string(), "v2".to_string()),
        ("x.md".to_string(), "x\n".to_string()),
    ];
    assert_eq!(b.files(), want);
}

/// A lost state is rebuilt from the whole feed, also when it is pulled in
/// batches: a note renamed here whose server file is in a later batch is
/// taken for it, not uploaded again (FINDING-054).
#[test]
fn a_rename_found_after_a_lost_state_is_kept_when_the_pull_is_batched() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n"), ("z old.md", "renamed here\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.mv("z old.md", "moved.md");
    fs::remove_file(a.state_file()).unwrap();
    a.restart();
    a.pull_in_batches(1, 1);
    let r = a.sync_ok();
    assert!(r.conflicts.is_empty() && r.skipped.is_empty(), "{r:?}");
    assert_eq!(a.paths(), ["a.md", "b.md", "c.md", "moved.md"]);
    b.sync_ok();
    assert_eq!(b.paths(), ["a.md", "b.md", "c.md", "moved.md"]);
}

/// The other way round: a later batch has a server file at the path of the
/// new file here, so that file is no rename of the missing one. As with one
/// batch, the missing note comes back; it is not deleted everywhere.
#[test]
fn a_lost_state_is_matched_with_the_whole_feed_when_the_pull_is_batched() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "same\n"), ("b.md", "same\n"), ("c.md", "c\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.rm("a.md");
    fs::remove_file(a.state_file()).unwrap();
    a.restart();
    a.pull_in_batches(1, 1);
    a.sync_ok();
    b.sync_ok();
    let want = [("a.md".to_string(), "same\n".to_string()), ("b.md".to_string(), "same\n".to_string()), ("c.md".to_string(), "c\n".to_string())];
    assert_eq!(a.files(), want);
    assert_eq!(b.files(), want);
}

// ------------------------------------------------------------------ fake HTTP server

#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Read the request and never answer.
    Hang,
    /// Send headers and then one byte of the body every second.
    Trickle,
    /// Promise 5000 bytes, send a few, close.
    Truncated,
    /// 200 with a body that is not JSON.
    Garbage,
    /// 500 with an HTML body, as a reverse proxy would.
    Http500,
}

struct Fake {
    url: String,
    stop: Arc<AtomicBool>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn read_request(s: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut b = [0u8; 4096];
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    loop {
        match s.read(&mut b) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&b[..n]),
        }
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            while buf.len() < i + 4 + len {
                match s.read(&mut b) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&b[..n]),
                }
            }
            return;
        }
    }
}

fn fake(mode: Mode) -> Fake {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.set_nonblocking(true).unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while !stop2.load(Ordering::SeqCst) {
            match l.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false).unwrap();
                    let stop3 = stop2.clone();
                    std::thread::spawn(move || {
                        read_request(&mut s);
                        match mode {
                            Mode::Hang => {
                                while !stop3.load(Ordering::SeqCst) {
                                    std::thread::sleep(Duration::from_millis(50));
                                }
                            }
                            Mode::Trickle => {
                                let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100000\r\n\r\n");
                                while !stop3.load(Ordering::SeqCst) {
                                    if s.write_all(b" ").is_err() {
                                        break;
                                    }
                                    std::thread::sleep(Duration::from_secs(1));
                                }
                            }
                            Mode::Truncated => {
                                let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 5000\r\n\r\n{\"heads\":[{\"file_id\":\"ab");
                            }
                            Mode::Garbage => {
                                let body = b"<html>hello from a captive portal</html>";
                                let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\r\n", body.len());
                                let _ = s.write_all(body);
                            }
                            Mode::Http500 => {
                                let body = b"<html><h1>502 Bad Gateway</h1></html>";
                                let _ = write!(s, "HTTP/1.1 500 Internal Server Error\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\r\n", body.len());
                                let _ = s.write_all(body);
                            }
                        }
                    });
                    held.push(());
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    });
    Fake { url, stop }
}

/// A synced pair; then A has local and remote changes pending and talks to
/// a broken server once. Afterwards A goes back to the real server.
fn broken_server_round(mode: Mode) -> (Result<(), String>, Duration) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("n.md", "1\n2\n3\n")]);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    b.write("n.md", "1 phone\n2\n3\n");
    b.sync_ok();
    a.write("n.md", "1\n2\n3 laptop\n");
    a.write("new.md", "new\n");
    let f = fake(mode);
    a.set_transport(http(&f.url));
    let t = Instant::now();
    let r = a.sync();
    let took = t.elapsed();
    drop(f);
    assert!(r.is_err(), "{mode:?}: sync against a broken server reported success");
    let msg = r.err().unwrap().to_string();
    eprintln!("{mode:?}: error after {took:?}: {msg}");
    let url = a.url.clone();
    a.set_transport(http(&url));
    converge(&mut a, &mut b);
    let ok = a.read("n.md").as_deref() == Some("1 phone\n2\n3 laptop\n") && b.read("new.md").is_some() && conflict_copies(&a.files()).is_empty();
    (if ok { Ok(()) } else { Err(format!("{mode:?}: after recovery A={:?}", a.files())) }, took)
}

#[test]
fn truncated_garbage_and_500_responses_fail_cleanly_and_recover() {
    for mode in [Mode::Truncated, Mode::Garbage, Mode::Http500] {
        let (r, took) = broken_server_round(mode);
        r.unwrap();
        assert!(took < Duration::from_secs(10), "{mode:?} took {took:?}");
    }
}

#[test]
#[ignore = "slow: about 60 s; a server that accepts and never answers"]
fn slow_hanging_server_times_out_and_recovers() {
    let (r, took) = broken_server_round(Mode::Hang);
    r.unwrap();
    eprintln!("hanging server: sync gave up after {took:?}");
    assert!(took < Duration::from_secs(75), "took {took:?}");
}

#[test]
#[ignore = "slow: about 60 s; a server that sends one byte per second"]
fn slow_trickling_server_times_out_and_recovers() {
    let (r, took) = broken_server_round(Mode::Trickle);
    r.unwrap();
    eprintln!("trickling server: sync gave up after {took:?}");
    assert!(took < Duration::from_secs(75), "took {took:?}");
}

// ------------------------------------------------------------------ corrupt data on the server

#[test]
fn one_corrupt_revision_does_not_stop_all_sync() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n")]);
    a.sync_ok();
    // a bit flips in one stored blob (disk error, bad backup restore)
    {
        let conn = rusqlite::Connection::open(&srv.db_path).unwrap();
        let (seq, mut blob): (i64, Vec<u8>) = conn.query_row("SELECT seq, blob FROM revisions ORDER BY seq LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x40;
        conn.execute("UPDATE revisions SET blob = ?1 WHERE seq = ?2", rusqlite::params![blob, seq]).unwrap();
    }
    let mut b = Device::new(&srv, "phone", &[("phone.md", "written on the phone\n")]);
    let r = b.sync();
    a.sync_ok();
    assert!(
        r.is_ok() && a.read("phone.md").is_some(),
        "B: {:?}; B has {:?}; A has phone.md: {}",
        r.err().map(|e| e.to_string()),
        b.paths(),
        a.read("phone.md").is_some()
    );
}

// ------------------------------------------------------------------ the real server process, killed

fn server_bin() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/cairn-server");
    assert!(p.exists(), "build cairn-server first: {}", p.display());
    p
}

struct Proc {
    child: Child,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
    Proc { child }
}

#[test]
fn server_killed_with_sigkill_mid_upload_keeps_acknowledged_revisions() {
    let data = tempfile::tempdir().unwrap();
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let mut srv = spawn_server(data.path(), port);
    let mut a = Device::new_url(&url, "laptop", &[]);
    // Enough for the upload to go on well past the kill.
    let n = 2000;
    fs::create_dir_all(a.root.join("n")).unwrap();
    for i in 0..n {
        fs::write(a.root.join(format!("n/{i:04}.md")), format!("note {i}\n{}\n", "x".repeat(2000))).unwrap();
    }
    // kill -9 the server while A is uploading, once it has stored `stored`
    // notes. A sends one note at a time, so by then A has recorded at least
    // all but the last of them as acknowledged. Not a fixed delay: A scans
    // and hashes every note before the first upload, which takes longer on a
    // slower machine.
    let pid = srv.child.id();
    let watch_url = url.clone();
    let stored = 20;
    let killer = std::thread::spawn(move || {
        let t = http(&watch_url);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            match t.changes(VAULT_ID, 0, stored) {
                Ok(c) if c.heads.len() >= stored as usize => break,
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        unsafe_kill(pid);
    });
    let r = a.sync();
    killer.join().unwrap();
    let _ = srv.child.wait();
    assert!(r.is_err(), "the upload finished before the kill; make the vault bigger");
    let acked: Vec<u64> = a.engine.as_ref().unwrap().state().files.values().map(|t| t.seq).collect();
    eprintln!("server killed after {} acknowledged uploads: {}", acked.len(), r.err().unwrap());
    assert!(!acked.is_empty());
    // restart on the same data
    srv = spawn_server(data.path(), port);
    // every revision the client recorded as stored is still there
    let t = http(&url);
    for seq in &acked {
        t.revision(VAULT_ID, *seq).unwrap_or_else(|e| panic!("acknowledged revision {seq} lost: {e}"));
    }
    a.sync_ok();
    let mut b = Device::new_url(&url, "phone", &[]);
    converge(&mut a, &mut b);
    assert_eq!(b.files().len(), n);
    assert!(conflict_copies(&b.files()).is_empty());
    drop(srv);
}

#[test]
fn server_restart_between_syncs_keeps_everything() {
    let data = tempfile::tempdir().unwrap();
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let srv = spawn_server(data.path(), port);
    let mut a = Device::new_url(&url, "laptop", &[("a.md", "a\n")]);
    a.sync_ok();
    drop(srv); // SIGKILL
    let r = a.sync();
    assert!(r.is_err());
    let srv = spawn_server(data.path(), port);
    a.write("b.md", "b\n");
    a.sync_ok();
    let mut b = Device::new_url(&url, "phone", &[]);
    b.sync_ok();
    assert_eq!(b.paths(), vec!["a.md", "b.md"]);
    drop(srv);
}

fn unsafe_kill(pid: u32) {
    // SIGKILL by pid (no libc dependency): use the kill command
    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
}

// ------------------------------------------------------------------ slow uplink

/// TCP proxy that forwards client-to-server bytes at `rate` bytes/second.
fn throttled_proxy(target: &str, rate: usize) -> (String, Arc<AtomicBool>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let target = target.trim_start_matches("http://").to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        for c in l.incoming() {
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let Ok(mut c) = c else { continue };
            let Ok(mut s) = TcpStream::connect(&target) else { continue };
            let (mut c2, mut s2) = (c.try_clone().unwrap(), s.try_clone().unwrap());
            std::thread::spawn(move || {
                let chunk = (rate / 10).max(1);
                let mut buf = vec![0u8; chunk];
                loop {
                    match c.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if s.write_all(&buf[..n]).is_err() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
                let _ = s.shutdown(std::net::Shutdown::Write);
            });
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut s2, &mut c2);
            });
        }
    });
    (url, stop)
}

#[test]
#[ignore = "slow: about 2 minutes; a 4 MB attachment over a 50 KB/s uplink"]
fn slow_uplink_large_attachment_eventually_uploads() {
    let srv = server();
    // 50 KB/s uplink (a weak mobile connection): 4 MB needs ~110 s
    let (purl, _stop) = throttled_proxy(&srv.url, 50 << 10);
    let mut a = Device::new(&srv, "laptop", &[]);
    a.write_bytes("Recording.m4a", &bytes(4 << 20, 9));
    a.write("z-notes.md", "meeting notes\n");
    a.set_transport(http(&purl));
    let mut errors = Vec::new();
    let mut skipped = Vec::new();
    for _ in 0..2 {
        let t = Instant::now();
        match a.sync() {
            Ok(r) => skipped.extend(r.skipped),
            Err(e) => errors.push(format!("{e} after {:?}", t.elapsed())),
        }
    }
    assert!(errors.is_empty(), "every sync fails: {errors:?}");
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    let got = fs::read(b.root.join("Recording.m4a")).ok();
    assert!(got == Some(bytes(4 << 20, 9)), "the recording did not arrive ({:?} bytes); skipped {skipped:?}", got.map(|g| g.len()));
    assert_eq!(b.read("z-notes.md").as_deref(), Some("meeting notes\n"));
}

// ------------------------------------------------------------------ the server loses its data

#[test]
fn server_data_loss_is_recovered_by_reconnecting() {
    let data = tempfile::tempdir().unwrap();
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let srv = spawn_server(data.path(), port);
    let mut a = Device::new_url(&url, "laptop", &[("a.md", "laptop note\n"), ("b.md", "second\n")]);
    a.sync_ok();
    a.write("c.md", "third\n");
    a.sync_ok();
    // the server is recreated without its data volume
    drop(srv);
    fs::remove_dir_all(data.path().join("cairn.sqlite")).ok();
    for f in fs::read_dir(data.path()).unwrap() {
        fs::remove_file(f.unwrap().path()).unwrap();
    }
    let srv = spawn_server(data.path(), port);
    let r = a.sync();
    eprintln!("A after the reset: {:?}", r.as_ref().err().map(|e| e.to_string()));
    let err = r.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("no longer has this notebook") && err.contains("turn sync off in Settings > Sync and connect again"), "{err:?}");
    // the user does what the error suggests: connect again, same settings
    a.engine = None;
    a.engine = Some(
        cairn_sync::engine::SyncEngine::connect_with(a.vault.clone(), &a.state_dir, settings(&url, "laptop"), PASS, http(&url), FAST_KDF).unwrap(),
    );
    let r = a.sync_ok();
    eprintln!("A after reconnecting: pulled {} pushed {}", r.pulled, r.pushed);
    // a new phone joins the (new) server vault
    let mut b = Device::new_url(&url, "phone", &[("phone.md", "from the phone\n")]);
    b.sync_ok();
    a.sync_ok();
    b.sync_ok();
    let (pa, pb) = (a.paths(), b.paths());
    drop(srv);
    assert!(
        pb.contains(&"a.md".to_string()) && pa.contains(&"phone.md".to_string()),
        "both devices say they are in sync, but the phone has {pb:?} and the laptop has {pa:?}"
    );
}

#[test]
fn upload_limit_boundary_is_three_quarters_of_cairn_max_body() {
    // CAIRN_MAX_BODY_MB limits the JSON request (base64 + overhead), so the
    // largest file is about 0.75 of it: with 1 MB, 740 KB passes, 790 KB not.
    // The refused file is reported by name; the sync itself goes on.
    let srv = server_with_body(1 << 20);
    let mut a = Device::new(&srv, "laptop", &[]);
    a.write_bytes("ok.bin", &bytes(740 << 10, 2));
    assert_eq!(a.sync_ok().pushed, 1);
    a.write_bytes("too-big.bin", &bytes(790 << 10, 3));
    let r = a.sync_ok();
    assert_eq!(r.pushed, 0);
    assert_eq!(r.skipped.len(), 1, "{:?}", r.skipped);
    assert_eq!(r.skipped[0].path, "too-big.bin");
    assert!(r.skipped[0].reason.contains("does not accept files this large"), "{:?}", r.skipped);
}

#[test]
fn a_file_over_the_client_limit_is_not_uploaded_and_large_pages_are_split() {
    // The client's own limit (MAX_BODY, scaled down to 1 MB here) applies
    // to uploads and to each changes page; the server would take more.
    let limited = |url: &str| -> Box<dyn cairn_sync::transport::Transport> {
        Box::new(cairn_sync::transport::HttpTransport::new(url, TOKEN).with_max_body(1 << 20))
    };
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[]);
    a.set_transport(limited(&srv.url));
    // five files that each fit, about 3.4 MB in one page
    for i in 0..5 {
        a.write_bytes(&format!("photos/{i}.jpg"), &bytes(500 << 10, 20 + i));
    }
    // not even read: larger than any upload that fits
    a.write_bytes("photos/huge.jpg", &bytes(900 << 10, 30));
    // read and encoded, then refused: the encoded upload is just over
    a.write_bytes("photos/edge.jpg", &bytes(768 << 10, 31));
    a.write("z.md", "sorted last\n");
    let r = a.sync_ok();
    assert_eq!(r.pushed, 6);
    let skipped: Vec<_> = r.skipped.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(skipped, ["photos/edge.jpg", "photos/huge.jpg"], "{:?}", r.skipped);
    assert!(r.skipped.iter().all(|s| s.reason == "too large to sync (over 0.8 MB)"), "{:?}", r.skipped);
    let mut b = Device::new(&srv, "phone", &[]);
    b.set_transport(limited(&srv.url));
    b.sync_ok();
    for i in 0..5 {
        assert_eq!(fs::read(b.root.join(format!("photos/{i}.jpg"))).unwrap(), bytes(500 << 10, 20 + i));
    }
    assert_eq!(b.read("z.md").as_deref(), Some("sorted last\n"));
    assert!(!b.root.join("photos/huge.jpg").exists() && !b.root.join("photos/edge.jpg").exists());
}
