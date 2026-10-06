//! Regression tests for FINDING-017 (one hostile pull record blocked
//! all sync, permanently, for every device).
//!
//! The basic test in adv_sync_security.rs only checks the first sync of a
//! fresh device. These tests check the stronger cases: every later sync
//! works too, on every device; a device that was already in sync before the
//! bad record appeared keeps pushing its changes; a hidden-path record is
//! handled the same way; and a single local write failure does not stop the
//! other files.
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_sx_01

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::crypto::{FilePayload, VaultKey};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "verify-token-0123456789";
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
    /// Decrypted paths of all heads on the server.
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

/// Every sync skips the one bad record: the good note is pulled, the local
/// note is pushed, and a second fresh device is not stuck either. (With the
/// defect, every later sync re-read the same bad head and failed again.)
#[test]
fn bad_record_does_not_block_sync_for_any_device() {
    let srv = server();
    let kh = KeyHolder::create(&srv);
    kh.put_new("aaaaaaaaaaaaaaaa", "../escape.md", b"pwn");
    kh.put_new("bbbbbbbbbbbbbbbb", "Good.md", b"hello");

    let mut a = Device::new(&srv, "laptop", &[("Local.md", b"my local note")]);
    let mut errs = Vec::new();
    for _ in 0..5 {
        errs.push(format!("{:?}", a.engine.sync().err()));
    }
    let mut b = Device::new(&srv, "phone", &[("Phone.md", b"phone note")]);
    let b_err = format!("{:?}", b.engine.sync().err());
    eprintln!("laptop errors over 5 syncs: {errs:#?}");
    eprintln!("phone error: {b_err}");
    eprintln!("laptop last_seq after 5 syncs: {}", a.last_seq());
    eprintln!("server paths: {:?}", kh.server_paths());

    // Nothing escaped the vault (the traversal itself is blocked).
    assert!(!a.root.parent().unwrap().join("escape.md").exists());
    // Desired: every sync succeeds after skipping the one bad record.
    assert!(errs.iter().all(|e| e == "None"), "sync failed on every attempt: {errs:?}");
    assert!(a.exists("Good.md"), "Good.md never pulled");
    assert!(kh.server_paths().iter().any(|p| p == "Local.md"), "Local.md never pushed");
    assert_eq!(b_err, "None", "a second device is stuck too");
}

/// A device that was fully in sync before the bad record appeared keeps
/// syncing: its new notes still reach the server.
#[test]
fn in_sync_device_keeps_pushing_after_bad_record() {
    let srv = server();
    let kh = KeyHolder::create(&srv);
    kh.put_new("bbbbbbbbbbbbbbbb", "Good.md", b"hello");
    let mut a = Device::new(&srv, "laptop", &[("Local.md", b"v1")]);
    a.engine.sync().expect("first sync is clean");
    assert!(kh.server_paths().iter().any(|p| p == "Local.md"));

    // One hidden-path record shows up (e.g. a newer/buggy client that syncs
    // `.cairn/`, or a malicious key holder planting a plugin).
    kh.put_new("cccccccccccccccc", ".cairn/plugins/evil.js", b"postMessage('pwn')");

    fs::write(a.root.join("New.md"), b"written after the bad record").unwrap();
    let res = a.engine.sync();
    eprintln!("sync after bad record: {:?}", res.as_ref().err());
    assert!(!a.exists(".cairn/plugins/evil.js"), "plugin planted");
    assert!(res.is_ok(), "in-sync device now fails every sync: {:?}", res.err());
    assert!(kh.server_paths().iter().any(|p| p == "New.md"), "New.md never pushed");
}

/// Not malice: one local write failure (a read-only folder on this device)
/// does not stop the other files, pulls or pushes. (With the defect, the
/// missing per-record isolation stopped them all until the folder was fixed.)
#[test]
fn one_local_write_failure_does_not_block_other_files() {
    use std::os::unix::fs::PermissionsExt;
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("Archive/old.md", b"old")]);
    a.engine.sync().unwrap();
    let mut b = Device::new(&srv, "phone", &[]);
    b.engine.sync().unwrap();
    assert!(b.exists("Archive/old.md"));
    // On the phone the Archive folder is read-only.
    fs::set_permissions(b.root.join("Archive"), fs::Permissions::from_mode(0o555)).unwrap();
    // The laptop adds a note inside Archive and an unrelated note.
    fs::write(a.root.join("Archive/new.md"), b"new").unwrap();
    fs::write(a.root.join("Unrelated.md"), b"unrelated").unwrap();
    a.engine.sync().unwrap();
    // The phone has its own unrelated edit to push.
    fs::write(b.root.join("PhoneNote.md"), b"from phone").unwrap();
    let res = b.engine.sync();
    let pulled_unrelated = b.exists("Unrelated.md");
    let laptop = a.engine.sync();
    let pushed = a.root.join("PhoneNote.md").exists();
    fs::set_permissions(b.root.join("Archive"), fs::Permissions::from_mode(0o755)).unwrap();
    eprintln!("phone sync: {:?}; Unrelated.md pulled: {pulled_unrelated}; PhoneNote.md reached laptop: {pushed} ({:?})", res.as_ref().err(), laptop.err());
    assert!(res.is_ok() || (pulled_unrelated && pushed), "one failing file blocked every other file: {:?}", res.err());
}

#[allow(dead_code)]
fn _unused(_: &Path) {}
