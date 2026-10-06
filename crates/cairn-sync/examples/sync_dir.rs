//! Sync a folder once from the command line (a second "device" for tests,
//! or a headless sync for servers and scripts).
//!
//!   sync_dir <notebook dir> <state dir> <server> <token> <notebook id> <device> <passphrase>
//!
//! Prints a JSON report.

use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::engine::{SyncEngine, SyncSettings};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() != 7 {
        eprintln!("usage: sync_dir <notebook dir> <state dir> <server> <token> <notebook id> <device> <passphrase>");
        std::process::exit(2);
    }
    std::fs::create_dir_all(&a[0]).unwrap();
    let vault = Arc::new(Vault::open(Arc::new(StdFs::new(&a[0], TrashMode::Vault).unwrap())).unwrap());
    let dir = std::path::Path::new(&a[1]);
    let settings = SyncSettings { server: a[2].clone(), token: a[3].clone(), vault_id: a[4].clone(), device: a[5].clone() };
    let mut engine = match SyncEngine::load(vault.clone(), dir) {
        Ok(Some(e)) => e,
        _ => SyncEngine::connect(vault, dir, settings, &a[6]).unwrap_or_else(|e| {
            eprintln!("connect failed: {e}");
            std::process::exit(1)
        }),
    };
    match engine.sync() {
        Ok(r) => println!("{}", serde_json::to_string(&r).unwrap()),
        Err(e) => {
            eprintln!("sync failed: {e}");
            std::process::exit(1);
        }
    }
}
