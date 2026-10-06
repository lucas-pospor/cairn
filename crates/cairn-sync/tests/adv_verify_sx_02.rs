//! Regression test for FINDING-018 (one undecryptable blob in the
//! changes feed stopped all sync).
//!
//! The basic test in adv_sync_security.rs checks the first sync of a fresh
//! device, and its attacker holds the vault key (although the garbage PUT
//! does not use it). This test checks the stronger case:
//!   * the attacker has only the bearer token, not the passphrase/key;
//!   * it overwrites the head of an EXISTING, already-synced file;
//!   * every device keeps syncing: every retry succeeds, the laptop's later
//!     local edit reaches the server, and a new device can sync;
//!   * local notes are NOT lost or modified.
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_sx_02 -- --nocapture

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "verify-sx02-token-0123456789";
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

struct Device {
    root: PathBuf,
    _vault: Arc<Vault>,
    engine: SyncEngine,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Device {
    fn new(srv: &Server, name: &str, files: &[(&str, &[u8])]) -> Device {
        let vd = tempfile::tempdir().unwrap();
        let sd = tempfile::tempdir().unwrap();
        for (p, c) in files {
            fs::write(vd.path().join(p), c).unwrap();
        }
        let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
        let settings = SyncSettings { server: srv.url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: name.into() };
        let engine =
            SyncEngine::connect_with(vault.clone(), sd.path(), settings, PASS, Box::new(HttpTransport::new(&srv.url, TOKEN)), FAST_KDF).unwrap();
        Device { root: vd.path().to_path_buf(), _vault: vault, engine, _dirs: (vd, sd) }
    }
    fn read(&self, p: &str) -> Option<Vec<u8>> {
        fs::read(self.root.join(p)).ok()
    }
}

/// A token-only attacker (a leaked token, or the server operator writing to
/// its own database): it overwrites the head of one existing file with a
/// blob it cannot have encrypted. No vault key, no passphrase.
#[test]
fn token_only_garbage_head_stops_no_device() {
    let srv = server();

    // Two honest devices in sync.
    let mut laptop = Device::new(&srv, "laptop", &[("A.md", b"alpha"), ("B.md", b"bravo")]);
    laptop.engine.sync().expect("initial push");
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.engine.sync().expect("initial pull");
    assert_eq!(phone.read("B.md").as_deref(), Some(&b"bravo"[..]));

    // Attacker: token only.
    let atk = HttpTransport::new(&srv.url, TOKEN);
    let feed = atk.changes("notes", 0, 500).unwrap();
    assert_eq!(feed.heads.len(), 2);
    let victim = &feed.heads[0];
    let rev = PutRevision { parent_seq: Some(victim.seq), device: "x".into(), deleted: false, blob: b64(&[1u8; 64]) };
    let stored = atk.put("notes", &victim.file_id, &rev).unwrap();
    eprintln!("attacker PUT over head {} of {}: stored={}", victim.seq, victim.file_id, matches!(stored, PutOutcome::Stored(_)));
    let PutOutcome::Stored(atk_seq) = stored else { panic!("server refused the garbage head") };

    // The laptop edits the OTHER file and syncs, three times.
    fs::write(laptop.root.join("B.md"), b"bravo, edited on the laptop").unwrap();
    let mut laptop_results = Vec::new();
    for _ in 0..3 {
        laptop_results.push(laptop.engine.sync().map(|r| (r.pulled, r.pushed)));
    }
    eprintln!("laptop syncs: {laptop_results:?}");
    let phone_res = phone.engine.sync().map(|r| (r.pulled, r.pushed));
    eprintln!("phone sync: {phone_res:?}");
    let mut fresh = Device::new(&srv, "tablet", &[]);
    let fresh_res = fresh.engine.sync().map(|r| (r.pulled, r.pushed));
    // Heads are served in seq order, so B.md (seq 2) comes before the garbage
    // head (seq 3) and IS applied on a fresh device; only heads newer than the
    // garbage one are cut off (and no device can push, so none appear).
    eprintln!("fresh tablet sync: {fresh_res:?}, has B.md: {:?}, has A.md: {:?}", fresh.read("B.md").is_some(), fresh.read("A.md").is_some());

    // No data loss either way: both devices keep their local notes as they were.
    assert_eq!(laptop.read("A.md").as_deref(), Some(&b"alpha"[..]), "laptop lost/changed A.md");
    assert_eq!(laptop.read("B.md").as_deref(), Some(&b"bravo, edited on the laptop"[..]), "laptop lost its edit");
    assert_eq!(phone.read("A.md").as_deref(), Some(&b"alpha"[..]), "phone lost/changed A.md");

    // Did the laptop's edit of the unrelated file ever reach the server?
    let after = atk.changes("notes", 0, 500).unwrap();
    let laptop_pushed_edit = after.heads.iter().any(|h| h.device == "laptop" && h.seq > atk_seq);
    eprintln!("laptop edit of B.md reached server: {laptop_pushed_edit}");

    // Desired behaviour: skip + report the one bad head, keep syncing the rest.
    assert!(laptop_results.iter().all(|r| r.is_ok()), "laptop sync aborted on one undecryptable head: {laptop_results:?}");
    assert!(laptop_pushed_edit, "the laptop's edit of an unrelated file never reached the server");
    assert!(phone_res.is_ok(), "phone sync aborted: {phone_res:?}");
    assert!(fresh_res.is_ok(), "a new device cannot sync at all: {fresh_res:?}");
}
