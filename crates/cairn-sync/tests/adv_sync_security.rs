//! Adversarial sync tests: a malicious-or-buggy OTHER
//! client (it knows the vault key, so it can upload correctly encrypted
//! payloads with any path/content) and a malicious server (it cannot forge
//! ciphertext but can withhold, reorder, swap, replay and return garbage).
//!
//! Harness copied from `two_devices.rs` (a real axum server over HTTP, real
//! `HttpTransport`), extended so a test can act as a raw client that holds
//! the vault key and PUTs arbitrary revisions.
//!
//! Run one test with, e.g.:
//!   cargo test -p cairn-sync --test adv_sync_security -- --exact one_bad_pull_record_does_not_block_sync
//! Run the whole file with:
//!   cargo test -p cairn-sync --test adv_sync_security

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::crypto::{FilePayload, VaultKey};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "test-token-0123456789";
const PASS: &str = "correct horse battery";

struct Server {
    url: String,
    _rt: tokio::runtime::Runtime,
    db_path: PathBuf,
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
    Server { url: format!("http://{addr}"), _rt: rt, db_path, _dir: dir }
}

/// A raw client that holds the vault key: the "malicious or buggy other
/// client" from the plan. It creates the vault, so it owns the key, and the
/// honest device below unlocks the same vault with the same passphrase.
struct RawClient {
    key: VaultKey,
    t: HttpTransport,
    vault: String,
}

impl RawClient {
    fn create(srv: &Server, vault: &str) -> RawClient {
        let t = HttpTransport::new(&srv.url, TOKEN);
        let key = VaultKey::generate();
        t.create_vault(vault, &key.wrap(PASS, FAST_KDF).unwrap()).unwrap();
        RawClient { key, t, vault: vault.to_string() }
    }

    /// PUT a brand-new file id with the given (possibly hostile) path/content.
    fn put_new(&self, file_id: &str, path: &str, data: &[u8]) -> PutOutcome {
        self.put(file_id, None, path, data, false)
    }

    fn put(&self, file_id: &str, parent: Option<u64>, path: &str, data: &[u8], deleted: bool) -> PutOutcome {
        let payload = FilePayload { path: path.into(), mtime: 0, data: data.to_vec() };
        let rev = PutRevision { parent_seq: parent, device: "attacker".into(), deleted, blob: b64(&self.key.encrypt(file_id, &payload.encode())) };
        self.t.put(&self.vault, file_id, &rev).unwrap()
    }

    /// PUT a revision whose blob is arbitrary bytes (undecryptable / garbage).
    fn put_raw_blob(&self, file_id: &str, parent: Option<u64>, blob: &[u8], deleted: bool) -> PutOutcome {
        let rev = PutRevision { parent_seq: parent, device: "attacker".into(), deleted, blob: b64(blob) };
        self.t.put(&self.vault, file_id, &rev).unwrap()
    }
}

/// An honest device unlocking the same vault with the passphrase.
struct Honest {
    root: PathBuf,
    vault: Arc<Vault>,
    engine: SyncEngine,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Honest {
    fn new(srv: &Server, vault_id: &str, name: &str, files: &[(&str, &[u8])]) -> Honest {
        let vd = tempfile::tempdir().unwrap();
        let sd = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let abs = vd.path().join(p);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(abs, c).unwrap();
        }
        let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
        let settings = SyncSettings { server: srv.url.clone(), token: TOKEN.into(), vault_id: vault_id.into(), device: name.into() };
        let engine =
            SyncEngine::connect_with(vault.clone(), sd.path(), settings, PASS, Box::new(HttpTransport::new(&srv.url, TOKEN)), FAST_KDF).unwrap();
        Honest { root: vd.path().to_path_buf(), vault, engine, _dirs: (vd, sd) }
    }

    fn files(&self) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }
    fn exists(&self, p: &str) -> bool {
        self.root.join(p).exists()
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

// ===================================================================
// FINDING-017: one hostile pull record must not abort ALL sync.
//
// A buggy-or-malicious other client (it knows the vault key) uploads a
// single revision whose decrypted path is unsafe (e.g. "../escape.md").
// The honest engine skips and reports that one record instead of failing
// the whole `round()` with a hard error, so:
//   * a perfectly good file uploaded alongside it is still pulled, and
//   * the honest device's own local notes are still pushed.
// With the defect, `last_seq` was not advanced past the bad head, so every
// later sync re-fetched it and failed again: sync was broken for good on
// every device.
// ===================================================================
#[test]
fn one_bad_pull_record_does_not_block_sync() {
    let srv = server();
    let atk = RawClient::create(&srv, "notes");
    // Hostile record FIRST (lowest seq), a perfectly good record after it.
    assert!(matches!(atk.put_new("aaaaaaaaaaaaaaaa", "../escape.md", b"pwn"), PutOutcome::Stored(_)));
    assert!(matches!(atk.put_new("bbbbbbbbbbbbbbbb", "Good.md", b"hello"), PutOutcome::Stored(_)));

    let mut honest = Honest::new(&srv, "notes", "laptop", &[("Local.md", b"my local note")]);
    let res = honest.engine.sync();

    // Desired behaviour: skip the one bad record, pull the good file, push ours.
    assert!(res.is_ok(), "sync aborted on a single bad record: {:?}", res.err());
    assert!(honest.exists("Good.md"), "a valid file was not pulled because of one bad record");
    // The hostile path must NOT have escaped the vault or landed inside it.
    assert!(!honest.exists("escape.md"));
    assert!(!Path::new(&honest.root).parent().unwrap().join("escape.md").exists(), "path traversal wrote outside the vault");
    // And our own local note must have reached the server.
    assert!(matches!(atk.t.changes("notes", 0, 500), Ok(c) if c.heads.iter().any(|h| h.device == "laptop")), "local note never pushed");
}

// ===================================================================
// FINDING-018: a single undecryptable / malformed blob must not abort all sync.
//
// A malicious server (or a corrupt record) returns one revision whose blob
// does not decrypt. The engine skips and reports that record; the other
// files in the feed are still applied and the device still pushes. With the
// defect, `round()` propagated the decrypt error: the same permanent
// breakage as FINDING-017, but triggerable by the server, which the plan
// treats as untrusted.
// ===================================================================
#[test]
fn one_undecryptable_blob_does_not_block_sync() {
    let srv = server();
    let atk = RawClient::create(&srv, "notes");
    // A blob of pure garbage (valid base64, wrong bytes) at the lowest seq.
    assert!(matches!(atk.put_raw_blob("aaaaaaaaaaaaaaaa", None, &[1u8; 64], false), PutOutcome::Stored(_)));
    assert!(matches!(atk.put_new("bbbbbbbbbbbbbbbb", "Good.md", b"hello"), PutOutcome::Stored(_)));

    let mut honest = Honest::new(&srv, "notes", "laptop", &[]);
    let res = honest.engine.sync();
    assert!(res.is_ok(), "sync aborted on one undecryptable blob: {:?}", res.err());
    assert!(honest.exists("Good.md"), "a valid file was not pulled because one blob would not decrypt");
}

// ===================================================================
// FINDING-019: a file larger than ~7.5 MB that one device pushed must also
// be pulled by another device.
//
// The server accepts bodies up to CAIRN_MAX_BODY_MB (default 200 MB), but
// the client's HTTP library (ureq) caps every *response* body at 10 MB by
// default. The changes feed inlines the base64 blob (~1.33x), so a single
// ~8 MB attachment made the changes response exceed 10 MB, the receiving
// device's `read_json` failed with BodyExceedsLimit, and that error aborted
// the whole sync (see FINDING-017/018). The client now reads the feed with
// its own, larger limit and asks for fewer records per page when a page
// would exceed it. Attachments this size are ordinary (photos, PDFs).
// ===================================================================
#[test]
fn large_file_can_be_pulled() {
    let srv = server();
    let big = vec![b'x'; 9 * 1024 * 1024]; // 9 MiB -> ~12 MiB base64 in the feed
    let mut a = Honest::new(&srv, "notes", "laptop", &[("big.bin", &big)]);
    let r = a.engine.sync().expect("push of a large file should succeed");
    assert_eq!(r.pushed, 1, "the large file was not pushed");

    let mut b = Honest::new(&srv, "notes", "phone", &[]);
    let res = b.engine.sync();
    assert!(res.is_ok(), "pulling a vault with one large attachment failed: {:?}", res.err());
    assert!(b.exists("big.bin"), "the large attachment never reached the second device");
}

// ===================================================================
// FINDING-065: a payload path containing NUL must not abort sync (another
// permanent-breakage vector in the FINDING-017 family). Path normalization
// now rejects interior NUL, so that record is skipped; with the defect, the
// engine carried it to `write`, where the OS rejected the file name, and
// the whole round errored.
// ===================================================================
#[test]
fn nul_in_pulled_path_does_not_block_sync() {
    let srv = server();
    let atk = RawClient::create(&srv, "notes");
    atk.put_new("aaaaaaaaaaaaaaaa", "a\u{0}b.md", b"x");
    atk.put_new("bbbbbbbbbbbbbbbb", "Good.md", b"hello");
    let mut honest = Honest::new(&srv, "notes", "laptop", &[]);
    let res = honest.engine.sync();
    assert!(res.is_ok(), "sync aborted on a NUL path: {:?}", res.err());
    assert!(honest.exists("Good.md"));
}

// ===================================================================
// Coverage of attacks that the design already handles.
// ===================================================================

/// A second client cannot overwrite the wrapped key / salt of an existing
/// vault, so it cannot lock everyone out or substitute its own key.
#[test]
fn create_vault_on_existing_is_rejected() {
    let srv = server();
    let atk = RawClient::create(&srv, "notes");
    // The envelope stored by the creator.
    let before = atk.t.get_vault("notes").unwrap().unwrap().keys;
    // A different key + passphrase tries to clobber it.
    let other = VaultKey::generate();
    let r = HttpTransport::new(&srv.url, TOKEN).create_vault("notes", &other.wrap("another pass", FAST_KDF).unwrap());
    assert!(r.is_err(), "a second create_vault on an existing vault must be rejected");
    let after = atk.t.get_vault("notes").unwrap().unwrap().keys;
    assert_eq!(before, after, "the key envelope was overwritten by a second client");
}

/// The file id is AEAD associated data: a server that swaps a file's blob for
/// another file's blob cannot make it decrypt, and the engine surfaces an
/// error rather than writing the wrong content to that path.
#[test]
fn swapped_blob_does_not_decrypt_to_wrong_file() {
    let key = VaultKey::generate();
    let p1 = FilePayload { path: "a.md".into(), mtime: 0, data: b"content of A".to_vec() };
    let p2 = FilePayload { path: "b.md".into(), mtime: 0, data: b"content of B".to_vec() };
    let blob1 = key.encrypt("f1", &p1.encode());
    let _blob2 = key.encrypt("f2", &p2.encode());
    // Pretend the server returns f1's blob under f2's id (a swap).
    assert!(key.decrypt("f2", &blob1).is_err(), "a swapped blob must not decrypt under the wrong file id");
    // And the honest id still works.
    assert_eq!(FilePayload::decode(&key.decrypt("f1", &blob1).unwrap()).unwrap(), p1);
}

/// Every API route requires the token.
#[test]
fn all_routes_require_the_token() {
    let srv = server();
    let atk = RawClient::create(&srv, "notes");
    atk.put_new("aaaaaaaaaaaaaaaa", "n.md", b"x");
    let bad = HttpTransport::new(&srv.url, "wrong-token");
    assert!(matches!(bad.get_vault("notes"), Err(cairn_sync::SyncError::Unauthorized)));
    assert!(matches!(bad.changes("notes", 0, 10), Err(cairn_sync::SyncError::Unauthorized)));
    assert!(matches!(bad.history("notes", "aaaaaaaaaaaaaaaa"), Err(cairn_sync::SyncError::Unauthorized)));
    assert!(matches!(bad.revision("notes", 1), Err(cairn_sync::SyncError::Unauthorized)));
    let rev = PutRevision { parent_seq: None, device: "x".into(), deleted: false, blob: b64(b"x") };
    assert!(matches!(bad.put("notes", "cccccccccccccccc", &rev), Err(cairn_sync::SyncError::Unauthorized)));
}

/// A revision cannot be read through a different vault's URL (seq is checked
/// against the vault).
#[test]
fn revision_is_scoped_to_its_vault() {
    let srv = server();
    let a = RawClient::create(&srv, "vaulta");
    let _b = RawClient::create(&srv, "vaultb");
    let PutOutcome::Stored(seq) = a.put_new("aaaaaaaaaaaaaaaa", "n.md", b"secret") else { panic!() };
    // Reading seq through vaultb must not return vaulta's revision.
    let t = HttpTransport::new(&srv.url, TOKEN);
    assert!(t.revision("vaultb", seq).is_err(), "a revision leaked across vaults");
    assert!(t.revision("vaulta", seq).is_ok());
}
