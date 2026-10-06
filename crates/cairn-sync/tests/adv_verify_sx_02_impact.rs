//! Impact checks for FINDING-018 (one undecryptable blob in
//! the changes feed stopped all sync).
//!
//! What these tests establish:
//!   1. `garbage_head_is_fail_closed_and_heals`: a
//!      token-only garbage head (which used to stop sync with a Crypto error)
//!      changes nothing on disk: nothing is lost, overwritten, trashed or
//!      conflict-copied, the client keeps no damaged state, and once the
//!      server serves a valid head again the file updates and the local edit
//!      reaches the other device.
//!   2. `malicious_server_can_already_withhold_silently`: the admitted
//!      known gap ("the server can withhold ... revisions")
//!      already lets a hostile server cut a device off completely, and do it
//!      silently (the client reports success). A garbage blob was a loud,
//!      detected version of the same denial, not a new capability.
//!   3. `one_flipped_bit_in_server_storage_does_not_stop_sync` (FINDING-018):
//!      the realistic non-malicious trigger, a single corrupted blob in the
//!      server's SQLite file (disk bit rot, bad restore), must not stop every
//!      device until someone edits the server database by hand.
//!
//! Run with:
//!   cargo test -p cairn-sync --test adv_verify_sx_02_impact -- --include-ignored --nocapture

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::crypto::{FilePayload, VaultKey};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};
use cairn_sync::SyncError;

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "verify-sx02-impact-token-0123456789";
const PASS: &str = "correct horse battery";

struct Server {
    url: String,
    db_path: PathBuf,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body: 100 << 20 });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { cairn_server::serve(listener, st).await });
    Server { url: format!("http://{addr}"), db_path, _rt: rt, _dir: dir }
}

struct Device {
    root: PathBuf,
    sync_dir: PathBuf,
    _vault: Arc<Vault>,
    engine: SyncEngine,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Device {
    fn new(srv: &Server, name: &str, files: &[(&str, &[u8])]) -> Device {
        Self::with_transport(srv, name, files, Box::new(HttpTransport::new(&srv.url, TOKEN)))
    }

    fn with_transport(srv: &Server, name: &str, files: &[(&str, &[u8])], t: Box<dyn Transport>) -> Device {
        let vd = tempfile::tempdir().unwrap();
        let sd = tempfile::tempdir().unwrap();
        for (p, c) in files {
            fs::write(vd.path().join(p), c).unwrap();
        }
        let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
        let settings = SyncSettings { server: srv.url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: name.into() };
        let engine = SyncEngine::connect_with(vault.clone(), sd.path(), settings, PASS, t, FAST_KDF).unwrap();
        Device { root: vd.path().to_path_buf(), sync_dir: sd.path().to_path_buf(), _vault: vault, engine, _dirs: (vd, sd) }
    }

    fn read(&self, p: &str) -> Option<Vec<u8>> {
        fs::read(self.root.join(p)).ok()
    }

    /// The vault key this device stored (what any honest client holds).
    fn key(&self) -> VaultKey {
        let raw = fs::read_to_string(self.sync_dir.join("key")).unwrap();
        VaultKey::from_bytes(unb64(raw.trim()).unwrap().try_into().unwrap())
    }

    /// Every file in the vault, dot-folders (trash) included.
    fn all_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

fn head_for<'a>(feed: &'a ChangesResponse, device: &str) -> Vec<&'a RemoteHead> {
    feed.heads.iter().filter(|h| h.device == device).collect()
}

#[test]
fn garbage_head_is_fail_closed_and_heals() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("A.md", b"alpha"), ("B.md", b"bravo")]);
    laptop.engine.sync().expect("initial push");
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.engine.sync().expect("initial pull");
    let laptop_files_before = laptop.all_files();

    // Token-only attacker overwrites A.md's head with bytes it cannot have encrypted.
    let t = HttpTransport::new(&srv.url, TOKEN);
    let feed = t.changes("notes", 0, 500).unwrap();
    let a_head = feed.heads.iter().find(|h| h.seq == feed.heads.iter().map(|h| h.seq).min().unwrap()).unwrap().clone();
    let a_fid = a_head.file_id.clone();
    let rev = PutRevision { parent_seq: Some(a_head.seq), device: "attacker".into(), deleted: true, blob: b64(&[7u8; 80]) };
    let PutOutcome::Stored(bad_seq) = t.put("notes", &a_fid, &rev).unwrap() else { panic!("garbage head refused") };

    // The laptop edits the OTHER file and tries to sync a few times.
    fs::write(laptop.root.join("B.md"), b"bravo, edited on the laptop").unwrap();
    let mut results = Vec::new();
    for _ in 0..3 {
        results.push(laptop.engine.sync().map(|r| (r.pulled, r.pushed)));
    }
    eprintln!("laptop syncs while the garbage head is served: {results:?}");
    let blocked = results.iter().all(|r| matches!(r, Err(SyncError::Crypto(_))));
    eprintln!("every attempt fails with a visible Crypto error: {blocked}");

    // Fail-closed: the garbage head (even marked deleted) did not delete, trash,
    // overwrite or conflict-copy anything, and the local edit is still on disk.
    assert_eq!(laptop.all_files(), laptop_files_before, "the vault's file set changed (trash/conflict/delete)");
    assert_eq!(laptop.read("A.md").as_deref(), Some(&b"alpha"[..]), "A.md was changed by a garbage head");
    assert_eq!(laptop.read("B.md").as_deref(), Some(&b"bravo, edited on the laptop"[..]), "local edit lost");
    assert_eq!(phone.read("A.md").as_deref(), Some(&b"alpha"[..]));

    // Heal: once the server serves a valid head for that file again (any holder
    // of the vault key uploads one on top of the garbage), sync resumes.
    let key = laptop.key();
    // Pushes go out sorted by path, so the lowest seq (the garbage target) is A.md.
    let a_path = "A.md";
    let payload = FilePayload { path: a_path.into(), mtime: 0, data: b"alpha restored".to_vec() };
    let fix = PutRevision { parent_seq: Some(bad_seq), device: "repair".into(), deleted: false, blob: b64(&key.encrypt(&a_fid, &payload.encode())) };
    assert!(matches!(t.put("notes", &a_fid, &fix).unwrap(), PutOutcome::Stored(_)));

    let healed = laptop.engine.sync();
    eprintln!("laptop sync after the head is valid again: {:?}", healed.as_ref().map(|r| (r.pulled, r.pushed)));
    assert!(healed.is_ok(), "client kept damaged state after the server head was fixed: {healed:?}");
    assert_eq!(laptop.read("A.md").as_deref(), Some(&b"alpha restored"[..]));
    let after = t.changes("notes", 0, 500).unwrap();
    assert!(!head_for(&after, "laptop").is_empty(), "the stuck local edit was not pushed after healing");
    phone.engine.sync().expect("phone sync after heal");
    assert_eq!(phone.read("B.md").as_deref(), Some(&b"bravo, edited on the laptop"[..]), "edit did not propagate after heal");
    assert_eq!(phone.read("A.md").as_deref(), Some(&b"alpha restored"[..]));
}

/// A server that withholds: it never shows other devices' revisions and
/// pretends to store uploads while dropping them. This is inside the admitted
/// gap ("The server can withhold or roll back revisions").
struct Withholding {
    inner: HttpTransport,
}

impl Transport for Withholding {
    fn get_vault(&self, vault: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.inner.get_vault(vault)
    }
    fn create_vault(&self, vault: &str, keys: &KeyEnvelope) -> Result<(), SyncError> {
        self.inner.create_vault(vault, keys)
    }
    fn changes(&self, _vault: &str, since: u64, _limit: u32) -> Result<ChangesResponse, SyncError> {
        Ok(ChangesResponse { heads: vec![], cursor: since, more: false })
    }
    fn put(&self, _vault: &str, _file_id: &str, rev: &PutRevision) -> Result<PutOutcome, SyncError> {
        Ok(PutOutcome::Stored(rev.parent_seq.unwrap_or(0) + 1_000_000))
    }
    fn history(&self, vault: &str, file_id: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        self.inner.history(vault, file_id)
    }
    fn revision(&self, vault: &str, seq: u64) -> Result<RevisionBlob, SyncError> {
        self.inner.revision(vault, seq)
    }
}

#[test]
fn malicious_server_can_already_withhold_silently() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("A.md", b"alpha")]);
    laptop.engine.sync().expect("initial push");

    // The phone talks to the same server, but the server withholds from it.
    let mut phone =
        Device::with_transport(&srv, "phone", &[("P.md", b"from phone")], Box::new(Withholding { inner: HttpTransport::new(&srv.url, TOKEN) }));
    let r = phone.engine.sync();
    eprintln!("phone sync against a withholding server: {:?}", r.as_ref().map(|r| (r.pulled, r.pushed)));
    assert!(r.is_ok(), "withholding is silent: the client reports success");
    assert!(phone.read("A.md").is_none(), "phone got A.md despite withholding");
    let feed = HttpTransport::new(&srv.url, TOKEN).changes("notes", 0, 500).unwrap();
    assert!(head_for(&feed, "phone").is_empty(), "the phone's note reached the server despite withholding");
    // So a hostile server can already deny all sync, without any error shown.
}

/// The non-malicious trigger: one blob corrupted in the server's storage.
#[test]
fn one_flipped_bit_in_server_storage_does_not_stop_sync() {
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("A.md", b"alpha"), ("B.md", b"bravo")]);
    laptop.engine.sync().expect("initial push");
    let mut phone = Device::new(&srv, "phone", &[]);
    phone.engine.sync().expect("initial pull");

    // A later edit of A.md, then bit rot in that revision's stored blob.
    fs::write(laptop.root.join("A.md"), b"alpha v2").unwrap();
    laptop.engine.sync().expect("push A v2");
    {
        let db = rusqlite::Connection::open(&srv.db_path).unwrap();
        let (seq, mut blob): (i64, Vec<u8>) =
            db.query_row("SELECT seq, blob FROM revisions ORDER BY seq DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let mid = blob.len() / 2;
        blob[mid] ^= 0x01;
        db.execute("UPDATE revisions SET blob = ?1 WHERE seq = ?2", rusqlite::params![blob, seq]).unwrap();
    }

    // Unrelated edit on the phone; it never reaches the laptop.
    fs::write(phone.root.join("B.md"), b"bravo, edited on the phone").unwrap();
    let pr = phone.engine.sync();
    eprintln!("phone sync with one corrupted blob in the feed: {:?}", pr.as_ref().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string()));
    let lr = laptop.engine.sync();
    eprintln!("laptop sync: {:?}", lr.as_ref().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string()));

    // No data loss on either side.
    assert_eq!(phone.read("A.md").as_deref(), Some(&b"alpha"[..]));
    assert_eq!(phone.read("B.md").as_deref(), Some(&b"bravo, edited on the phone"[..]));

    // Desired: skip and report the one bad record, keep syncing the rest.
    assert!(pr.is_ok(), "phone sync aborted on one corrupted blob: {pr:?}");
    assert!(lr.is_ok(), "laptop sync aborted: {lr:?}");
    assert_eq!(laptop.read("B.md").as_deref(), Some(&b"bravo, edited on the phone"[..]), "the unrelated edit never propagated");
}
