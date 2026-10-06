//! Regression tests for FINDING-065 (a pulled path containing NUL aborted
//! all sync; `vpath::normalize` did not reject interior NUL).
//!
//! The basic test in adv_sync_security.rs checks one sync of a fresh device.
//! These tests check:
//!   * the core validators (`vpath::normalize`, `Vault::write_file`) reject
//!     NUL as an invalid path, instead of letting it fail later as a local
//!     I/O error from the OS;
//!   * a device that was already in sync keeps syncing (a good file uploaded
//!     after the record is pulled, its new note is pushed);
//!   * an over-long name (ENAMETOOLONG) does not abort sync either.
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_sx_04 -- --nocapture

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use cairn_core::path as vpath;
use cairn_core::{CoreError, StdFs, TrashMode, Vault};
use cairn_sync::crypto::{FilePayload, VaultKey};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};
use cairn_sync::SyncError;

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "verify-sx04-token-0123456789";
const PASS: &str = "correct horse battery";

struct Server {
    url: String,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let conn = cairn_server::open_db(&dir.path().join("cairn.sqlite")).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body: 100 << 20 });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { cairn_server::serve(listener, st).await });
    Server { url: format!("http://{addr}"), _rt: rt, _dir: dir }
}

/// A key-holding client that can PUT arbitrary (correctly encrypted) revisions.
struct KeyHolder {
    key: VaultKey,
    t: HttpTransport,
}

impl KeyHolder {
    fn create(srv: &Server) -> KeyHolder {
        let t = HttpTransport::new(&srv.url, TOKEN);
        let key = VaultKey::generate();
        t.create_vault("notes", &key.wrap(PASS, FAST_KDF).unwrap()).unwrap();
        KeyHolder { key, t }
    }
    fn put_new(&self, file_id: &str, path: &str, data: &[u8]) {
        let payload = FilePayload { path: path.into(), mtime: 0, data: data.to_vec() };
        let rev = PutRevision {
            parent_seq: None,
            device: "other".into(),
            deleted: false,
            blob: b64(&self.key.encrypt(file_id, &payload.encode())),
        };
        assert!(matches!(self.t.put("notes", file_id, &rev).unwrap(), PutOutcome::Stored(_)));
    }
    fn server_paths(&self) -> Vec<String> {
        let c = self.t.changes("notes", 0, 2000).unwrap();
        c.heads
            .iter()
            .filter_map(|h| {
                let blob = unb64(&h.blob)?;
                let plain = self.key.decrypt(&h.file_id, &blob).ok()?;
                Some(FilePayload::decode(&plain).ok()?.path)
            })
            .collect()
    }
}

struct Device {
    root: PathBuf,
    state_dir: PathBuf,
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
        Device { root: vd.path().to_path_buf(), state_dir: sd.path().to_path_buf(), engine, _dirs: (vd, sd) }
    }
    fn exists(&self, p: &str) -> bool {
        self.root.join(p).exists()
    }
    fn last_seq(&self) -> u64 {
        let v: serde_json::Value = serde_json::from_slice(&fs::read(self.state_dir.join("state.json")).unwrap()).unwrap();
        v["last_seq"].as_u64().unwrap()
    }
}

/// The validators that decide "this path can never be a vault file" reject
/// NUL: `validate_name` (used for names the user types) rejects control
/// characters, and `normalize` (used for paths from sync, the watcher and
/// the UI) rejects NUL, so `Vault::write_file` reports `InvalidPath`, not an
/// OS I/O error.
#[test]
fn core_validators_reject_nul() {
    let n = vpath::normalize("a\u{0}b.md");
    let nl = vpath::normalize("a\nb.md");
    let long = vpath::normalize(&format!("{}.md", "x".repeat(300)));
    eprintln!("normalize(\"a\\0b.md\") = {n:?}");
    eprintln!("normalize(\"a\\nb.md\") = {nl:?}");
    eprintln!("normalize(300-byte name) is_ok = {}", long.is_ok());
    eprintln!("validate_name(\"a\\0b\") = {:?}", vpath::validate_name("a\u{0}b"));

    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(Arc::new(StdFs::new(dir.path(), TrashMode::Vault).unwrap())).unwrap();
    let w = vault.write_file("a\u{0}b.md", b"x", None);
    eprintln!("Vault::write_file(\"a\\0b.md\") = {:?}", w.as_ref().err());

    assert!(vpath::validate_name("a\u{0}b").is_err(), "sanity: user-chosen names already reject control chars");
    assert!(matches!(w, Err(CoreError::InvalidPath(_))), "write_file did not reject the NUL path as invalid: {:?}", w.err());
    assert!(n.is_err(), "normalize kept the NUL: {n:?}");
}

/// A device that was in sync before the NUL record appeared keeps syncing:
/// every later sync succeeds, a good file uploaded after the record is
/// pulled and the device's new note is pushed. (With the defect, the record
/// got past the engine's "unsafe path" check and failed as a local I/O error
/// on write, and every later sync failed the same way.)
#[test]
fn nul_record_does_not_block_in_sync_device() {
    let srv = server();
    let kh = KeyHolder::create(&srv);
    kh.put_new("bbbbbbbbbbbbbbbb", "Good.md", b"hello");
    let mut a = Device::new(&srv, "laptop", &[("Local.md", b"v1")]);
    a.engine.sync().expect("first sync is clean");
    assert!(kh.server_paths().iter().any(|p| p == "Local.md"));
    let seq_before = a.last_seq();

    kh.put_new("aaaaaaaaaaaaaaaa", "a\u{0}b.md", b"x");
    kh.put_new("cccccccccccccccc", "After.md", b"after");
    fs::write(a.root.join("New.md"), b"written after the NUL record").unwrap();

    let mut errs = Vec::new();
    for _ in 0..3 {
        errs.push(a.engine.sync().err());
    }
    eprintln!("errors over 3 syncs: {errs:#?}");
    eprintln!("last_seq before/after: {seq_before} / {}", a.last_seq());
    eprintln!("After.md pulled: {}; New.md pushed: {}", a.exists("After.md"), kh.server_paths().iter().any(|p| p == "New.md"));

    // Where it fails: as a local I/O error, i.e. past the engine's validation.
    let first_is_local_io = matches!(&errs[0], Some(SyncError::Local(m)) if m.contains("NUL"));
    eprintln!("first error is a local I/O error (not 'unsafe path'): {first_is_local_io}");

    assert!(errs.iter().all(|e| e.is_none()), "every sync fails: {errs:?}");
    assert!(a.exists("After.md"), "After.md never pulled");
    assert!(kh.server_paths().iter().any(|p| p == "New.md"), "New.md never pushed");
}

/// Same kind of record, different byte: an over-long file name (no NUL,
/// valid UTF-8) does not abort the round and the good file is still pulled.
/// (With the defect, its failure at write with ENAMETOOLONG aborted the
/// round, like the NUL path.)
#[test]
fn overlong_name_record_does_not_block_sync() {
    let srv = server();
    let kh = KeyHolder::create(&srv);
    let long = format!("{}.md", "\u{65e5}".repeat(100)); // 100 CJK chars = 300 UTF-8 bytes
    kh.put_new("aaaaaaaaaaaaaaaa", &long, b"x");
    kh.put_new("bbbbbbbbbbbbbbbb", "Good.md", b"hello");
    let mut a = Device::new(&srv, "laptop", &[]);
    let res = a.engine.sync();
    eprintln!("sync with a 300-byte name: {:?}", res.as_ref().err());
    assert!(res.is_ok(), "sync aborted on an over-long name: {:?}", res.err());
    assert!(a.exists("Good.md"));
}
