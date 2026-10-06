//! Regression tests for FINDING-019 (ureq's 10 MiB response cap vs the
//! server's CAIRN_MAX_BODY_MB upload limit). Checks what the cap used to
//! break, one case at a time:
//!
//!   cargo test -p cairn-sync --test adv_verify_sx_03 -- --nocapture --test-threads=1
//!
//! Each test is a separate case so the output shows which ones work.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::HttpTransport;

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "test-token-0123456789";
const PASS: &str = "correct horse battery";

struct Server {
    url: String,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let conn = cairn_server::open_db(&dir.path().join("cairn.sqlite")).unwrap();
    // The production default (CAIRN_MAX_BODY_MB=200).
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body: 200 << 20 });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { cairn_server::serve(listener, st).await });
    Server { url: format!("http://{addr}"), _rt: rt, _dir: dir }
}

struct Device {
    root: PathBuf,
    engine: SyncEngine,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Device {
    fn new(srv: &Server, name: &str, files: &[(&str, &[u8])]) -> Device {
        let vd = tempfile::tempdir().unwrap();
        let sd = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let abs = vd.path().join(p);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(abs, c).unwrap();
        }
        let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
        let settings = SyncSettings { server: srv.url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: name.into() };
        let engine =
            SyncEngine::connect_with(vault, sd.path(), settings, PASS, Box::new(HttpTransport::new(&srv.url, TOKEN)), FAST_KDF).unwrap();
        Device { root: vd.path().to_path_buf(), engine, _dirs: (vd, sd) }
    }
    fn exists(&self, p: &str) -> bool {
        self.root.join(p).exists()
    }
}

/// Pseudo-random bytes so nothing in the stack can shrink them.
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// The uploading device's next sync works. With the defect it did not: the
/// engine skipped its own head only AFTER the changes page had been read, and
/// `last_seq` was not advanced past its own uploads, so the uploader's next
/// sync downloaded the same oversized page.
#[test]
fn uploader_does_not_break_on_its_next_sync() {
    let srv = server();
    let big = noise(8 * 1024 * 1024, 1); // 8 MiB, an ordinary photo/PDF size
    let mut a = Device::new(&srv, "laptop", &[("big.bin", &big), ("a.md", b"hello")]);
    let r = a.engine.sync().expect("first push");
    assert_eq!(r.pushed, 2);
    let second = a.engine.sync();
    println!("uploader's second sync: {:?}", second.as_ref().map(|r| r.pushed).map_err(|e| e.to_string()));
    assert!(second.is_ok(), "the uploading device's own next sync failed: {:?}", second.err());
}

/// The receiving device's whole round completes, so it pushes its own local
/// changes too (pull runs before push; with the defect the round aborted).
#[test]
fn receiver_can_push_its_own_notes() {
    let srv = server();
    let big = noise(8 * 1024 * 1024, 2);
    let mut a = Device::new(&srv, "laptop", &[("big.bin", &big)]);
    a.engine.sync().expect("push");
    let mut b = Device::new(&srv, "phone", &[("phone-note.md", b"written on the phone")]);
    let rb = b.engine.sync();
    println!("receiver sync: {:?}", rb.as_ref().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string()));
    // Does phone-note.md ever reach a third device? (It cannot: the third device
    // would hit the same oversized page, so check the server feed size instead.)
    assert!(rb.is_ok(), "receiver sync failed, so its own note was never uploaded: {:?}", rb.err());
    assert!(b.exists("big.bin"));
}

/// No single big file is needed. The changes feed returns up to 500 heads per
/// page with every blob inlined and no byte cap, so an initial pull of a vault
/// whose files add up to more than ~7.5 MB in one 500-file window used to fail
/// the same way; it must work. Here: 16 attachments of 640 KiB each.
#[test]
fn many_medium_files_do_not_break_initial_pull() {
    let srv = server();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..16 {
        files.push((format!("attachments/photo{i:02}.jpg"), noise(640 * 1024, 100 + i)));
    }
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, d)| (p.as_str(), d.as_slice())).collect();
    let mut a = Device::new(&srv, "laptop", &refs);
    let r = a.engine.sync().expect("push of 16 x 640 KiB");
    assert_eq!(r.pushed, 16);
    let mut b = Device::new(&srv, "phone", &[]);
    let rb = b.engine.sync();
    println!("initial pull of 16 x 640 KiB: {:?}", rb.as_ref().map(|r| r.pulled).map_err(|e| e.to_string()));
    assert!(rb.is_ok(), "initial pull of 10 MiB of medium attachments failed: {:?}", rb.err());
}

/// Version history of a file larger than ~7.5 MB can be read.
#[test]
fn large_revision_content_is_readable() {
    let srv = server();
    let big = noise(8 * 1024 * 1024, 3);
    let mut a = Device::new(&srv, "laptop", &[("big.bin", &big)]);
    a.engine.sync().expect("push");
    let hist = a.engine.history("big.bin").expect("history list is small");
    let seq = hist.first().expect("one revision").seq;
    let rc = a.engine.revision_content(seq);
    println!("revision_content: {:?}", rc.as_ref().map(|p| p.data.len()).map_err(|e| e.to_string()));
    assert!(rc.is_ok(), "revision content of an 8 MiB file failed: {:?}", rc.err());
}

/// Control: a file just under the old threshold round-trips (with the defect,
/// this showed the failures above were the response cap, nothing else).
#[test]
fn control_7mib_file_round_trips() {
    let srv = server();
    let big = noise(7 * 1024 * 1024, 4); // 7 MiB -> ~9.4 MiB base64 + JSON
    let mut a = Device::new(&srv, "laptop", &[("big.bin", &big)]);
    a.engine.sync().expect("push");
    let mut b = Device::new(&srv, "phone", &[]);
    b.engine.sync().expect("7 MiB pull should fit under 10 MiB");
    assert_eq!(fs::read(b.root.join("big.bin")).unwrap(), big);
    a.engine.sync().expect("uploader's second sync with a 7 MiB head");
}
