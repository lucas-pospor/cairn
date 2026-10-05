//! Impact checks for FINDING-017 (one bad pull record blocks all sync).
//! Questions checked: does it lose data, can the user recover, what can a key
//! holder already do with valid revisions, and can honest devices hit the
//! same root cause (no per-record isolation in the pull loop)?
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_sx_01_impact -- --ignored --nocapture

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::crypto::{FilePayload, VaultKey};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "verify2-token-0123456789";
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
    fn put(&self, file_id: &str, parent: Option<u64>, path: &str, data: &[u8], deleted: bool) -> PutOutcome {
        let payload = FilePayload { path: path.into(), mtime: 0, data: data.to_vec() };
        let rev = PutRevision { parent_seq: parent, device: "other".into(), deleted, blob: b64(&self.key.encrypt(file_id, &payload.encode())) };
        self.t.put("notes", file_id, &rev).unwrap()
    }
    /// (file_id, seq, path, deleted) of every head on the server.
    fn heads(&self) -> Vec<(String, u64, String, bool)> {
        let c = self.t.changes("notes", 0, 2000).unwrap();
        c.heads
            .iter()
            .filter_map(|h| {
                let plain = self.key.decrypt(&h.file_id, &unb64(&h.blob)?).ok()?;
                Some((h.file_id.clone(), h.seq, FilePayload::decode(&plain).ok()?.path, h.deleted))
            })
            .collect()
    }
    fn live_paths(&self) -> Vec<String> {
        self.heads().into_iter().filter(|h| !h.3).map(|h| h.2).collect()
    }
}

struct Device {
    root: PathBuf,
    state_dir: PathBuf,
    vault: Arc<Vault>,
    settings: SyncSettings,
    srv_url: String,
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
        let engine = SyncEngine::connect_with(
            vault.clone(),
            sd.path(),
            settings.clone(),
            PASS,
            Box::new(HttpTransport::new(&srv.url, TOKEN)),
            FAST_KDF,
        )
        .unwrap();
        Device {
            root: vd.path().to_path_buf(),
            state_dir: sd.path().to_path_buf(),
            vault,
            settings,
            srv_url: srv.url.clone(),
            engine,
            _dirs: (vd, sd),
        }
    }
    /// What a user can do in Settings > Sync: disconnect, then set up again.
    fn reconnect(&mut self) {
        SyncEngine::disconnect(&self.state_dir).unwrap();
        self.engine = SyncEngine::connect_with(
            self.vault.clone(),
            &self.state_dir,
            self.settings.clone(),
            PASS,
            Box::new(HttpTransport::new(&self.srv_url, TOKEN)),
            FAST_KDF,
        )
        .unwrap();
    }
    fn read(&self, p: &str) -> Option<Vec<u8>> {
        fs::read(self.root.join(p)).ok()
    }
}

/// Bad record AFTER a good one (higher seq): the good file is pulled, but
/// the push phase never runs, and the user's only recovery action in the UI
/// (disconnect + set up again) does not help. Local data is never touched.
#[test]
fn reconnect_does_not_recover_and_local_data_is_kept() {
    let srv = server();
    let kh = KeyHolder::create(&srv);
    assert!(matches!(kh.put("bbbbbbbbbbbbbbbb", None, "Good.md", b"hello", false), PutOutcome::Stored(_)));
    assert!(matches!(kh.put("aaaaaaaaaaaaaaaa", None, ".git/hooks/post-commit", b"#!/bin/sh\nrm -rf ~\n", false), PutOutcome::Stored(_)));

    let mut d = Device::new(&srv, "laptop", &[("Local.md", b"my local note")]);
    let r1 = d.engine.sync();
    let good_pulled = d.read("Good.md").as_deref() == Some(b"hello".as_slice());
    fs::write(d.root.join("Local.md"), b"my local note, edited while sync is broken").unwrap();
    let r2 = d.engine.sync();
    d.reconnect();
    let r3 = d.engine.sync();
    eprintln!("sync 1: {:?}\nGood.md pulled before the abort: {good_pulled}\nsync 2: {:?}\nafter reconnect: {:?}", r1.as_ref().err(), r2.as_ref().err(), r3.as_ref().err());
    eprintln!("server live paths: {:?}", kh.live_paths());

    // No data loss and no escape: the local note keeps its newest text, the
    // hostile hook was never written.
    assert_eq!(d.read("Local.md").as_deref(), Some(b"my local note, edited while sync is broken".as_slice()));
    assert!(!d.root.join(".git").exists(), "hidden path written");
    // The defect: even after the user's only recovery action, sync fails
    // and the local note never reaches the server.
    assert!(r3.is_ok(), "still broken after disconnect + reconnect: {:?}", r3.err());
    assert!(kh.live_paths().iter().any(|p| p == "Local.md"), "Local.md never pushed");
}

/// Context: a key holder can already do worse with fully
/// valid revisions. Deleting every file on the server makes the honest
/// device delete them locally (to the trash). So the DoS gives an attacker
/// who holds the key no new power over the data.
#[test]
fn key_holder_can_already_delete_everything_with_valid_revisions() {
    let srv = server();
    let kh = KeyHolder::create(&srv);
    let mut d = Device::new(&srv, "laptop", &[("A.md", b"note a"), ("sub/B.md", b"note b")]);
    d.engine.sync().unwrap();
    for (fid, seq, path, deleted) in kh.heads() {
        if !deleted {
            assert!(matches!(kh.put(&fid, Some(seq), &path, b"", true), PutOutcome::Stored(_)));
        }
    }
    let r = d.engine.sync();
    eprintln!("sync after remote mass delete: {:?}", r.as_ref().map(|r| (r.pulled, r.pushed)));
    assert!(r.is_ok());
    assert!(d.read("A.md").is_none() && d.read("sub/B.md").is_none(), "valid deletes were applied");
    let trash = d.root.join(".trash");
    eprintln!("trash exists: {}", trash.exists());
}

/// Honest devices, no attacker: a plain file named `Archive` (an attachment
/// without extension) on one device and a folder `Archive/` on another.
/// The device that syncs second can no longer sync anything: its other new
/// note is never pushed, every retry fails the same way.
#[test]
fn honest_file_folder_clash_stops_all_sync() {
    let srv = server();
    let _kh = KeyHolder::create(&srv);
    let mut a = Device::new(&srv, "laptop", &[("Archive/a.md", b"inside the folder")]);
    a.engine.sync().unwrap();
    let mut b = Device::new(&srv, "phone", &[("Archive", b"a file with no extension"), ("Phone.md", b"phone note")]);
    let errs: Vec<String> = (0..3).map(|_| format!("{:?}", b.engine.sync().err())).collect();
    let _ = a.engine.sync();
    eprintln!("phone errors: {errs:#?}");
    eprintln!("Phone.md reached laptop: {}", a.read("Phone.md").is_some());
    assert_eq!(b.read("Archive").as_deref(), Some(b"a file with no extension".as_slice()), "local file lost");
    assert!(errs.iter().all(|e| e == "None"), "phone sync fails every time: {errs:?}");
    assert!(a.read("Phone.md").is_some(), "Phone.md never pushed");
}
