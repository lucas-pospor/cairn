//! Adversarial tests of the SERVER's behaviour, driven
//! through the real `HttpTransport` against a real axum server, with the
//! SQLite file inspected directly via rusqlite.
//!
//! (These live in cairn-sync, not cairn-server/tests, because exercising the
//! HTTP surface needs tokio + HttpTransport + cairn-server + rusqlite, all of
//! which are cairn-sync dev-dependencies; cairn-server's only dev-dep is
//! tempfile.)
//!
//! Ignored bug reproductions:
//!   cargo test -p cairn-sync --test adv_sync_security_server -- --ignored --exact server_does_not_store_device_name
//! Everything else:
//!   cargo test -p cairn-sync --test adv_sync_security_server

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cairn_sync::crypto::{FilePayload, VaultKey};
use cairn_sync::protocol::*;
use cairn_sync::transport::{HttpTransport, PutOutcome, Transport};
use rusqlite::{Connection, OpenFlags};

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "test-token-0123456789";
const PASS: &str = "correct horse battery";

struct Server {
    url: String,
    _rt: tokio::runtime::Runtime,
    db_path: PathBuf,
    _dir: tempfile::TempDir,
}

fn server_with(tokens: Vec<String>, max_body: usize) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens, max_body });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { cairn_server::serve(listener, st).await });
    Server { url: format!("http://{addr}"), _rt: rt, db_path, _dir: dir }
}

fn server() -> Server {
    server_with(vec![TOKEN.into()], 100 << 20)
}

fn create_vault(srv: &Server, vault: &str) -> VaultKey {
    let t = HttpTransport::new(&srv.url, TOKEN);
    let key = VaultKey::generate();
    t.create_vault(vault, &key.wrap(PASS, FAST_KDF).unwrap()).unwrap();
    key
}

fn put(srv: &Server, key: &VaultKey, vault: &str, fid: &str, parent: Option<u64>, path: &str, data: &[u8]) -> PutOutcome {
    let p = FilePayload { path: path.into(), mtime: 0, data: data.to_vec() };
    let rev = PutRevision { parent_seq: parent, device: "dev".into(), deleted: false, blob: b64(&key.encrypt(fid, &p.encode())) };
    HttpTransport::new(&srv.url, TOKEN).put(vault, fid, &rev).unwrap()
}

fn open_ro(path: &PathBuf) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI).unwrap()
}

// ===================================================================
// FINDING-152: the server stores (and serves) each device's name in
// plaintext. The plan says the server "sees only IDs, sizes and timestamps";
// the user-chosen device name (often a host name such as "alices-macbook")
// is none of those and is not listed as something the server learns.
// ===================================================================
#[test]
#[ignore = "FINDING-152: device names are stored on the server in plaintext"]
fn server_does_not_store_device_name() {
    let srv = server();
    let key = create_vault(&srv, "notes");
    put(&srv, &key, "notes", "aaaaaaaaaaaaaaaa", None, "n.md", b"x");
    // A distinctive device name the user would not want disclosed.
    let p = FilePayload { path: "n.md".into(), mtime: 0, data: b"y".to_vec() };
    let rev = PutRevision { parent_seq: None, device: "alices-secret-macbook".into(), deleted: false, blob: b64(&key.encrypt("bbbbbbbbbbbbbbbb", &p.encode())) };
    HttpTransport::new(&srv.url, TOKEN).put("notes", "bbbbbbbbbbbbbbbb", &rev).unwrap();

    let raw = std::fs::read(&srv.db_path).unwrap();
    let wal = std::fs::read(srv.db_path.with_extension("sqlite-wal")).unwrap_or_default();
    let needle = b"alices-secret-macbook";
    let leaked = raw.windows(needle.len()).any(|w| w == needle) || wal.windows(needle.len()).any(|w| w == needle);
    assert!(!leaked, "the device name is stored on the server in plaintext");
}

// ===================================================================
// Coverage of attacks that the design already handles.
// ===================================================================

/// Compare-and-swap under real concurrency: many PUTs with the same parent
/// race; exactly one is accepted, the rest get 409.
#[test]
fn concurrent_put_same_parent_only_one_wins() {
    let srv = server();
    let key = create_vault(&srv, "notes");
    let PutOutcome::Stored(seq) = put(&srv, &key, "notes", "ffffffffffffffff", None, "n.md", b"v0") else { panic!() };

    let stored = AtomicUsize::new(0);
    let conflict = AtomicUsize::new(0);
    let key = Arc::new(key);
    let url = srv.url.clone();
    std::thread::scope(|s| {
        for i in 0..16 {
            let stored = &stored;
            let conflict = &conflict;
            let key = key.clone();
            let url = url.clone();
            s.spawn(move || {
                let p = FilePayload { path: "n.md".into(), mtime: 0, data: format!("v{i}").into_bytes() };
                let rev = PutRevision { parent_seq: Some(seq), device: "dev".into(), deleted: false, blob: b64(&key.encrypt("ffffffffffffffff", &p.encode())) };
                match HttpTransport::new(&url, TOKEN).put("notes", "ffffffffffffffff", &rev).unwrap() {
                    PutOutcome::Stored(_) => stored.fetch_add(1, Ordering::SeqCst),
                    PutOutcome::Conflict(_) => conflict.fetch_add(1, Ordering::SeqCst),
                };
            });
        }
    });
    assert_eq!(stored.load(Ordering::SeqCst), 1, "more than one racing PUT with the same parent was accepted");
    assert_eq!(conflict.load(Ordering::SeqCst), 15);
}

/// Global seq is monotonically increasing across files and vaults.
#[test]
fn seq_is_monotonic() {
    let srv = server();
    let ka = create_vault(&srv, "va");
    let kb = create_vault(&srv, "vb");
    let mut last = 0u64;
    for (k, v, f) in [(&ka, "va", "aaaaaaaaaaaaaaaa"), (&kb, "vb", "bbbbbbbbbbbbbbbb"), (&ka, "va", "cccccccccccccccc")] {
        let PutOutcome::Stored(seq) = put(&srv, k, v, f, None, "n.md", b"x") else { panic!() };
        assert!(seq > last, "seq {seq} not greater than previous {last}");
        last = seq;
    }
}

/// The server validates file ids: a non-[A-Za-z0-9_-] id is refused.
#[test]
fn invalid_file_id_is_rejected() {
    let srv = server();
    let key = create_vault(&srv, "notes");
    let p = FilePayload { path: "n.md".into(), mtime: 0, data: b"x".to_vec() };
    let rev = PutRevision { parent_seq: None, device: "d".into(), deleted: false, blob: b64(&key.encrypt("x", &p.encode())) };
    // Space and slash are both outside the allowed set.
    let r = HttpTransport::new(&srv.url, TOKEN).put("notes", "bad%20id", &rev);
    assert!(r.is_err(), "server accepted an invalid file id");
}

/// A blob larger than the server's body limit is refused with an error (not
/// silently stored, and does not panic the server).
#[test]
fn body_limit_is_enforced() {
    let srv = server_with(vec![TOKEN.into()], 1 << 20); // 1 MiB limit
    let key = create_vault(&srv, "notes");
    let big = vec![b'x'; 4 * 1024 * 1024];
    let p = FilePayload { path: "big.bin".into(), mtime: 0, data: big };
    let rev = PutRevision { parent_seq: None, device: "d".into(), deleted: false, blob: b64(&key.encrypt("aaaaaaaaaaaaaaaa", &p.encode())) };
    let r = HttpTransport::new(&srv.url, TOKEN).put("notes", "aaaaaaaaaaaaaaaa", &rev);
    assert!(r.is_err(), "a body over the limit should be rejected");
    // The server is still alive afterwards.
    assert!(HttpTransport::new(&srv.url, TOKEN).get_vault("notes").is_ok());
}

/// The server never stores note content or paths in plaintext (re-checks the
/// core guarantee from an adversarial angle: a path with a distinctive word).
#[test]
fn server_stores_no_plaintext_path_or_content() {
    let srv = server();
    let key = create_vault(&srv, "notes");
    put(&srv, &key, "notes", "aaaaaaaaaaaaaaaa", None, "TopSecretFolder/Plans.md", b"invade at dawn");
    let raw = std::fs::read(&srv.db_path).unwrap();
    let wal = std::fs::read(srv.db_path.with_extension("sqlite-wal")).unwrap_or_default();
    for hay in [&raw, &wal] {
        assert!(!hay.windows(9).any(|w| w == b"TopSecret"), "path leaked");
        assert!(!hay.windows(6).any(|w| w == b"invade"), "content leaked");
    }
    // Sanity: the db really did get populated (so the test is meaningful).
    let c = open_ro(&srv.db_path);
    let n: i64 = c.query_row("SELECT COUNT(*) FROM revisions", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
