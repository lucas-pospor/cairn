//! Reproduction for FINDING-152 (the server stores each device's
//! human-chosen name in plaintext). `server_does_not_store_device_name` in
//! adv_sync_security_server.rs hand-builds a `PutRevision`; this test drives
//! the real `SyncEngine` exactly as the app does, with a device name of the
//! kind the app pre-fills (the host name, see app/src-tauri/src/commands.rs
//! `default_device_name`), and then looks at what the server holds and what
//! a token holder WITHOUT the passphrase can read back.
//!
//!   cargo test -p cairn-sync --test adv_verify_sx_05 -- --ignored --nocapture

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::protocol::KdfParams;
use cairn_sync::transport::{HttpTransport, Transport};

const FAST_KDF: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };
const TOKEN: &str = "test-token-0123456789";
const PASS: &str = "correct horse battery";
// Shaped like a macOS default host name; the app pre-fills the host name.
const DEVICE: &str = "Alices-MacBook-Pro";

#[test]
#[ignore = "FINDING-152: the real sync engine sends the device name (default: host name) to the server in plaintext"]
fn engine_sync_does_not_disclose_device_name_to_server() {
    // Real server on a temp SQLite file.
    let sdir = tempfile::tempdir().unwrap();
    let db_path = sdir.path().join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body: 50 << 20 });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    rt.spawn(async move { cairn_server::serve(listener, st).await });

    // One device, one note, one sync round through the real engine.
    let vd = tempfile::tempdir().unwrap();
    let sd = tempfile::tempdir().unwrap();
    fs::write(vd.path().join("Diary.md"), "private words").unwrap();
    let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
    let settings = SyncSettings { server: url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: DEVICE.into() };
    let mut engine =
        SyncEngine::connect_with(vault, sd.path(), settings, PASS, Box::new(HttpTransport::new(&url, TOKEN)), FAST_KDF).unwrap();
    let rep = engine.sync().unwrap();
    assert_eq!(rep.pushed, 1, "the note should have been pushed");

    // Sanity: note text and path are not in the server files (the documented guarantee).
    let raw = fs::read(&db_path).unwrap();
    let wal = fs::read(db_path.with_extension("sqlite-wal")).unwrap_or_default();
    let has = |needle: &[u8]| raw.windows(needle.len()).any(|w| w == needle) || wal.windows(needle.len()).any(|w| w == needle);
    assert!(!has(b"private words") && !has(b"Diary"), "content or path leaked (would be a different, worse finding)");

    // What a token holder without the passphrase reads back from the API.
    let t = HttpTransport::new(&url, TOKEN);
    let feed = t.changes("notes", 0, 100).unwrap();
    let fid = feed.heads[0].file_id.clone();
    let hist = t.history("notes", &fid).unwrap();
    println!("changes feed device field (no passphrase needed): {:?}", feed.heads.iter().map(|h| &h.device).collect::<Vec<_>>());
    println!("history device field (no passphrase needed): {:?}", hist.iter().map(|h| &h.device).collect::<Vec<_>>());
    println!("device name present in on-disk sqlite/wal: {}", has(DEVICE.as_bytes()));

    assert!(!has(DEVICE.as_bytes()), "the device name ({DEVICE}) is stored in the server's SQLite files in plaintext");
    assert!(feed.heads.iter().all(|h| h.device != DEVICE), "the changes feed returns the device name to anyone with the token");
}
