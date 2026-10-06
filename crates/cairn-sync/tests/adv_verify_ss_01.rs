//! Regression tests for FINDING-004 (notes whose names differ only in
//! case used to destroy each other through a case-insensitive device).
//!
//! The other tests use a simulated `CaseInsensitiveFs`. These tests
//! use the product's real `StdFs` (atomic temp-file + rename writes) on a REAL
//! case-insensitive, case-preserving directory: a Linux casefold (`chattr +F`)
//! directory on a tmpfs mounted with `-o casefold` inside a private user and
//! mount namespace (nothing outside the namespace is touched). The "linux"
//! device stays on the normal case-sensitive temp dir.
//!
//! Run (no root needed):
//!   S=$(mktemp -d); cargo test -p cairn-sync --test adv_verify_ss_01 --no-run
//!   BIN=$(ls -t target/debug/deps/adv_verify_ss_01-* | grep -v '\.d$' | head -1)
//!   unshare -Urm sh -c "mount -t tmpfs -o casefold tmpfs $S && mkdir $S/ci && chattr +F $S/ci && \
//!     CAIRN_CI_DIR=$S/ci $BIN --nocapture --test-threads=1"
//!
//! Without CAIRN_CI_DIR (or if it is not actually case-insensitive) each test
//! prints "skipped" and returns.

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};
use cairn_sync::engine::{SyncEngine, SyncSettings};
use cairn_sync::transport::HttpTransport;
use common::*;

/// A device whose vault lives on a real case-insensitive directory and uses
/// the product's own `StdFs`, exactly like the desktop app on macOS/Windows.
struct CiDevice {
    root: PathBuf,
    engine: SyncEngine,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl CiDevice {
    fn new(srv: &Server, name: &str, files: &[(&str, &str)]) -> Option<CiDevice> {
        let Some(base) = std::env::var_os("CAIRN_CI_DIR") else {
            eprintln!("skipped: set CAIRN_CI_DIR to a case-insensitive directory");
            return None;
        };
        let vd = tempfile::Builder::new().prefix("ci-vault-").tempdir_in(&base).unwrap();
        // Make sure this really is a case-insensitive directory.
        fs::write(vd.path().join("Probe"), "x").unwrap();
        let ci = vd.path().join("probe").exists();
        fs::remove_file(vd.path().join("Probe")).unwrap();
        if !ci {
            eprintln!("skipped: {:?} is case-sensitive", vd.path());
            return None;
        }
        for (p, c) in files {
            fs::write(vd.path().join(p), c).unwrap();
        }
        let sd = tempfile::Builder::new().prefix("ci-state-").tempdir().unwrap();
        let root = vd.path().canonicalize().unwrap();
        let vault = Arc::new(Vault::open(Arc::new(StdFs::new(&root, TrashMode::Vault).unwrap())).unwrap());
        let settings = SyncSettings { server: srv.url.clone(), token: TOKEN.into(), vault_id: "notes".into(), device: name.into() };
        let engine =
            SyncEngine::connect_with(vault, sd.path(), settings, PASS, Box::new(HttpTransport::new(&srv.url, TOKEN)), FAST_KDF).unwrap();
        Some(CiDevice { root, engine, _dirs: (vd, sd) })
    }

    fn files(&self) -> Vec<(String, String)> {
        listing(&self.root, false)
    }

    fn all_text(&self) -> String {
        listing(&self.root, true).into_iter().map(|f| f.1).collect::<Vec<_>>().join("\n")
    }
}

fn listing(root: &Path, hidden: bool) -> Vec<(String, String)> {
    let mut v = Vec::new();
    walk(root, root, hidden, &mut v);
    v.sort();
    v.into_iter().map(|(p, b)| (p, String::from_utf8_lossy(&b).into_owned())).collect()
}

/// Every revision on the server, decrypted (seq 1.. until the server says no).
fn server_text(e: &SyncEngine) -> String {
    let mut out = String::new();
    for seq in 1..200 {
        match e.revision_content(seq) {
            Ok(p) => {
                out.push_str(&format!("[{seq} {}] {}", p.path, String::from_utf8_lossy(&p.data)));
            }
            Err(_) => break,
        }
    }
    out
}

/// FINDING-004 main case on a real case-insensitive directory.
#[test]
fn real_ci_fs_two_notes_differing_only_in_case() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Note.md", "upper case note\n"), ("note.md", "lower case note\n")]);
    linux.sync();
    let Some(mut mac) = CiDevice::new(&srv, "mac", &[]) else { return };
    let r = mac.engine.sync();
    eprintln!("mac first sync: {:?}", r.as_ref().map(|r| (&r.conflicts, &r.changes)).map_err(|e| e.to_string()));
    eprintln!("mac files right after first sync: {:?}", mac.files());
    for _ in 0..2 {
        let _ = linux.try_sync();
        let _ = mac.engine.sync();
    }
    let srv_text = server_text(&linux.engine);
    eprintln!("linux files: {:?}\nlinux trash: {:?}\nmac files: {:?}", linux.files(), linux.trash_text(), mac.files());
    eprintln!("server revisions: {srv_text}");
    let linux_live: String = linux.files().into_iter().map(|f| f.1).collect();
    let mac_live: String = mac.files().into_iter().map(|f| f.1).collect();
    for w in ["upper case note", "lower case note"] {
        assert!(mac_live.contains(w), "{w:?} missing from the mac's vault: {:?}", mac.files());
        assert!(linux_live.contains(w), "{w:?} missing from the linux vault: {:?} (trash {:?})", linux.files(), linux.trash_text());
    }
}

/// Same as above with notes of different sizes (the usual case: the other
/// reproductions' two notes happen to be exactly 16 bytes each, and on a real
/// file system the overwrite can land in the same millisecond, so the
/// size+mtime shortcut in `SyncEngine::scan` hides the overwrite; with
/// different sizes it does not).
#[test]
fn real_ci_fs_two_notes_differing_only_in_case_different_sizes() {
    let srv = server();
    let mut linux = Device::new(
        &srv,
        "linux",
        &[("Ideas.md", "UPPER: the only copy of these ideas, a longer note\n"), ("ideas.md", "lower: a different note\n")],
    );
    linux.sync();
    let Some(mut mac) = CiDevice::new(&srv, "mac", &[]) else { return };
    let r = mac.engine.sync();
    eprintln!("mac first sync: {:?}", r.as_ref().map(|r| (&r.conflicts, &r.changes)).map_err(|e| e.to_string()));
    for _ in 0..2 {
        let _ = linux.try_sync();
        let _ = mac.engine.sync();
    }
    let srv_text = server_text(&linux.engine);
    eprintln!("linux files: {:?}\nlinux trash: {:?}\nmac files: {:?}", linux.files(), linux.trash_text(), mac.files());
    eprintln!("server revisions: {srv_text}");
    let live = format!("{:?}{:?}", linux.files(), mac.files());
    let trash = format!("{}{}", linux.trash_text(), mac.all_text());
    eprintln!(
        "UPPER in a live vault: {}, in a trash: {}, in server history: {}",
        live.contains("UPPER"),
        trash.contains("UPPER"),
        srv_text.contains("UPPER")
    );
    assert!(live.contains("UPPER") && live.contains("lower: a different note"), "a note vanished from every live vault");
}

/// Variant: the case-insensitive device's own, never-synced `Note.md` is
/// not lost when a remote `note.md` arrives in the pull (pull runs before push).
#[test]
fn real_ci_fs_local_unsynced_note_survives_remote_note() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("note.md", "from linux\n")]);
    linux.sync();
    let Some(mut mac) = CiDevice::new(&srv, "mac", &[("Note.md", "only on the mac, never synced\n")]) else { return };
    let r = mac.engine.sync();
    let _ = linux.try_sync();
    let _ = mac.engine.sync();
    let everywhere = format!("{}{}{}", mac.all_text(), linux.all_text(), server_text(&linux.engine));
    eprintln!(
        "mac sync: {:?}\nmac files {:?}\nlinux files {:?}\nserver: {}",
        r.map(|r| r.conflicts).map_err(|e| e.to_string()),
        mac.files(),
        linux.files(),
        server_text(&linux.engine)
    );
    assert!(everywhere.contains("only on the mac, never synced"), "the mac's own note is gone from both vaults, both trashes and the server");
}

/// Realistic workflow: both devices are already set up and in sync. Between
/// syncs (offline laptop), the mac user creates `Todo.md` and the Linux user
/// creates `todo.md`. Linux happens to sync first.
#[test]
fn real_ci_fs_concurrent_creates_differing_in_case() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("Welcome.md", "hi\n")]);
    linux.sync();
    let Some(mut mac) = CiDevice::new(&srv, "mac", &[]) else { return };
    mac.engine.sync().unwrap();
    assert_eq!(mac.files().len(), 1);
    // offline edits
    fs::write(mac.root.join("Todo.md"), "mac: buy milk\n").unwrap();
    linux.write("todo.md", "linux: call bob\n");
    linux.sync();
    let r = mac.engine.sync();
    for _ in 0..2 {
        let _ = linux.try_sync();
        let _ = mac.engine.sync();
    }
    let everywhere = format!("{}{}{}", mac.all_text(), linux.all_text(), server_text(&linux.engine));
    eprintln!(
        "mac sync: {:?}\nmac files {:?}\nlinux files {:?}\nserver: {}",
        r.map(|r| r.conflicts).map_err(|e| e.to_string()),
        mac.files(),
        linux.files(),
        server_text(&linux.engine)
    );
    assert!(everywhere.contains("mac: buy milk"), "the mac's offline note is gone everywhere");
    assert!(everywhere.contains("linux: call bob"));
}
