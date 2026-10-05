//! Reproduction for FINDING-055.
//!
//! Root cause: `SyncEngine::scan` reuses the tracked hash whenever the tracked
//! size and mtime equal the index entry, and several places record a
//! (size, mtime) that was taken separately from the bytes whose hash is
//! recorded (`self.stat(path)` after a read or a rename). When the recorded
//! (size, mtime) describes newer bytes than the recorded hash, the newer bytes
//! are never hashed again: they are never uploaded, and the next remote edit
//! of that note fails the expected-hash write in the Unchanged branch on every
//! round, so the whole sync of the device fails with "the server kept changing
//! during sync".
//!
//! These tests show two more triggers:
//!
//! * a remote rename-only revision applied while the user saves the note
//!   through the vault during the pull (`apply_remote`, Unchanged branch,
//!   `rhash == old.hash` -> `self.stat(&path)`); the window is the whole
//!   changes request, not the few microseconds between the push's read and
//!   its stat;
//! * a file system with 2 s mtime granularity (FAT32 USB sticks and SD
//!   cards), where two same-size saves (checkbox toggles) in the same 2 s
//!   bucket with a sync in between are enough.
//!
//! Run: cargo test -p cairn-sync --test adv_verify_sr_05 -- --ignored --nocapture

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cairn_core::{FileStat, StdFs, TrashMode, Vault, VaultFs};
use common::*;

fn synced_pair(files: &[(&str, &str)]) -> (Server, Device, Device) {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", files);
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    a.sync_ok();
    (srv, a, b)
}

/// Run `f` (with A's vault) right before A's next sync sends its first
/// changes request, i.e. after the round's scan and before apply_remote.
fn on_first_changes(d: &mut Device, f: impl FnOnce(&Arc<Vault>) + Send + 'static) {
    let vault = d.vault.clone();
    let t = Arc::new(FaultTransport::passthrough(http(&d.url)));
    let mut f = Some(f);
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op == "changes" {
            if let Some(f) = f.take() {
                f(&vault);
            }
        }
    }));
    d.set_transport(Box::new(SharedTransport(t)));
}

fn plain(d: &mut Device) {
    let url = d.url.clone();
    d.set_transport(http(&url));
}

#[test]
fn save_during_pull_of_a_remote_rename_is_uploaded() {
    let (_srv, mut a, mut b) = synced_pair(&[("n.md", "v1\n"), ("other.md", "o\n")]);
    // The phone renames the note (content unchanged).
    b.mv("n.md", "m.md");
    b.sync_ok();
    // While A's sync waits for the changes feed, the editor autosaves the
    // note through the vault, as the app does.
    on_first_changes(&mut a, |v| {
        v.write_file("n.md", b"v2 typed while the sync was pulling\n", None).unwrap();
    });
    let r1 = a.sync_ok();
    plain(&mut a);
    for _ in 0..3 {
        a.sync_ok();
        b.sync_ok();
    }
    let on_a = a.read("m.md");
    let on_b = b.read("m.md");
    eprintln!("first sync on A: pushed={} pulled={}; A m.md={on_a:?}; B m.md={on_b:?}", r1.pushed, r1.pulled);
    assert_eq!(on_a.as_deref(), Some("v2 typed while the sync was pulling\n"));
    // Then the phone edits the note and another one; A must still sync.
    b.write("m.md", "v1\nphone line\n");
    b.write("other.md", "o\nphone\n");
    b.sync_ok();
    let r = a.sync();
    eprintln!("A's sync after the phone's edit: {:?}; A other.md={:?}", r.as_ref().map(|r| r.pulled).map_err(|e| e.to_string()), a.read("other.md"));
    assert_eq!(on_b.as_deref(), Some("v2 typed while the sync was pulling\n"), "the save never left the laptop");
    assert!(r.is_ok(), "A's sync fails");
}

// ------------------------------------------------------------ coarse mtime

/// StdFs with FAT32's 2 s modification-time granularity.
struct CoarseFs(StdFs);

fn coarse(mut s: FileStat) -> FileStat {
    s.mtime = s.mtime / 2000 * 2000;
    s
}

impl VaultFs for CoarseFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        Ok(self.0.list(dir)?.into_iter().map(coarse).collect())
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        Ok(self.0.stat(path)?.map(coarse))
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.0.read(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        Ok(coarse(self.0.write(path, data)?))
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.0.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        self.0.rename(from, to)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.0.remove(path)
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.0.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        self.0.describe()
    }
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}

fn wait_for_next_2s_bucket() {
    let next = (now_ms() / 2000 + 1) * 2000;
    while now_ms() < next + 20 {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn coarse_mtime_checkbox_toggle_after_a_sync_is_uploaded() {
    let srv = server();
    let maker: FsMaker = Arc::new(|root: &Path| Arc::new(CoarseFs(StdFs::new(root, TrashMode::Vault).unwrap())) as Arc<dyn VaultFs>);
    let mut a = Device::new_with_fs(&srv.url, "laptop", &[("todo.md", "- [ ] milk\n- [ ] eggs\n")], Some(maker));
    a.sync_ok();
    let mut b = Device::new(&srv, "phone", &[]);
    b.sync_ok();
    for attempt in 0..5 {
        let base = if attempt % 2 == 0 { "- [ ] milk\n- [ ] eggs\n" } else { "- [x] milk\n- [x] eggs\n" };
        let toggled1 = if attempt % 2 == 0 { "- [x] milk\n- [ ] eggs\n" } else { "- [ ] milk\n- [x] eggs\n" };
        let toggled2 = if attempt % 2 == 0 { "- [x] milk\n- [x] eggs\n" } else { "- [ ] milk\n- [ ] eggs\n" };
        assert_eq!(a.read("todo.md").as_deref(), Some(base));
        wait_for_next_2s_bucket();
        // first click, saved through the vault like the editor does
        a.vault.write_note("todo.md", toggled1, None).unwrap();
        let m1 = a.vault.index().entry("todo.md").unwrap().mtime;
        let r1 = a.sync_ok(); // minute timer or "Sync now"
        // second click a moment later: same size
        a.vault.write_note("todo.md", toggled2, None).unwrap();
        let m2 = a.vault.index().entry("todo.md").unwrap().mtime;
        if m1 != m2 {
            eprintln!("attempt {attempt}: the two saves fell into different 2 s buckets; retrying");
            a.sync_ok();
            b.sync_ok();
            continue;
        }
        let r2 = a.sync_ok();
        b.sync_ok();
        eprintln!("attempt {attempt}: first sync pushed {}, second sync pushed {}; phone has {:?}", r1.pushed, r2.pushed, b.read("todo.md"));
        assert_eq!(b.read("todo.md").as_deref(), Some(toggled2), "the second checkbox toggle never reached the phone");
        return;
    }
    panic!("timing never lined up; inconclusive");
}
