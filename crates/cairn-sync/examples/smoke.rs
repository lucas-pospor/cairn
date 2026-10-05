//! End-to-end check against a running server:
//!   cargo run -p cairn-sync --example smoke -- http://127.0.0.1:8787 <token>
//! Creates two throwaway vaults, syncs a note from one to the other through
//! the server, and prints what happened.

use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::engine::{SyncEngine, SyncSettings};

fn device(server: &str, token: &str, vault_id: &str, name: &str) -> (tempfile::TempDir, tempfile::TempDir, Arc<Vault>, SyncEngine) {
    let vd = tempfile::tempdir().unwrap();
    let sd = tempfile::tempdir().unwrap();
    let vault = Arc::new(Vault::open(Arc::new(StdFs::new(vd.path(), TrashMode::Vault).unwrap())).unwrap());
    let settings = SyncSettings { server: server.into(), token: token.into(), vault_id: vault_id.into(), device: name.into() };
    let engine = SyncEngine::connect(vault.clone(), sd.path(), settings, "smoke test passphrase").expect("connect");
    (vd, sd, vault, engine)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let server = args.next().expect("server url");
    let token = args.next().expect("token");
    let vault_id = format!("smoke-{}", std::process::id());
    let (vd_a, _s1, _va, mut a) = device(&server, &token, &vault_id, "smoke-a");
    std::fs::write(vd_a.path().join("hello.md"), "# Hello from A\n").unwrap();
    let r = a.sync().expect("sync A");
    println!("A pushed {} file(s)", r.pushed);
    let (vd_b, _s2, _vb, mut b) = device(&server, &token, &vault_id, "smoke-b");
    let r = b.sync().expect("sync B");
    println!("B pulled {} file(s)", r.pulled);
    let got = std::fs::read_to_string(vd_b.path().join("hello.md")).expect("B has the note");
    assert_eq!(got, "# Hello from A\n");
    println!("OK: note arrived on B through {server}");
}
